use std::{
    fs,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::Mutex,
};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use muriarc_upgrade::{
    ActivationVerificationEvidence, ActiveGeneration, BackendUpgradeLock, BackupEvidence,
    CandidateEvidence, DeploymentProfile, DrainEvidence, FreezeEvidence, MigrationEvidence,
    PreflightEvidence, ReadOnlyActivationEvidence, RestoreEvidence, SwitchEvidence, UpgradeDriver,
    UpgradeError, UpgradeSnapshot, VerificationEvidence, VerifiedRelease, WriteLeaseEvidence,
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use uuid::Uuid;

const DRIVER_PROTOCOL_FORMAT: u32 = 1;

#[derive(Debug, Clone)]
pub struct PhysicalDriverClient {
    executable: PathBuf,
    profile: DeploymentProfile,
}

impl PhysicalDriverClient {
    pub fn new(
        executable: impl Into<PathBuf>,
        profile: DeploymentProfile,
    ) -> Result<Self, UpgradeError> {
        let executable = executable.into();
        require_executable(&executable)?;
        Ok(Self {
            executable,
            profile,
        })
    }

    pub fn executable(&self) -> &Path {
        &self.executable
    }

    pub fn profile(&self) -> DeploymentProfile {
        self.profile
    }

    pub fn invoke<T: DeserializeOwned + Serialize>(
        &self,
        action: &'static str,
        payload: Value,
    ) -> Result<T, UpgradeError> {
        let request = driver_request(action, self.profile, payload);
        let request_bytes = serde_json::to_vec(&request).map_err(serialization)?;
        let mut child = Command::new(&self.executable)
            .args(["invoke", action])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| driver_unavailable(action))?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| driver_unavailable(action))?;
        stdin
            .write_all(&request_bytes)
            .and_then(|()| stdin.write_all(b"\n"))
            .map_err(|_| driver_unavailable(action))?;
        drop(stdin);
        let output = child
            .wait_with_output()
            .map_err(|_| driver_unavailable(action))?;
        if !output.status.success() {
            return Err(driver_failed(action));
        }
        parse_driver_response(action, &output.stdout)
    }

    fn acquire_lock(
        &self,
        operation_id: Uuid,
    ) -> Result<Box<dyn BackendUpgradeLock>, UpgradeError> {
        let action = "acquire_backend_lock";
        let request = driver_request(
            action,
            self.profile,
            json!({ "operation_id": operation_id }),
        );
        let request_bytes = serde_json::to_vec(&request).map_err(serialization)?;
        let mut child = Command::new(&self.executable)
            .args(["hold-lock", action])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| driver_unavailable(action))?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| driver_unavailable(action))?;
        stdin
            .write_all(&request_bytes)
            .and_then(|()| stdin.write_all(b"\n"))
            .map_err(|_| driver_unavailable(action))?;
        drop(stdin);
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| driver_unavailable(action))?;
        let mut reader = BufReader::new(stdout);
        let mut line = Vec::new();
        let count = reader
            .read_until(b'\n', &mut line)
            .map_err(|_| driver_unavailable(action))?;
        if count == 0 || line.len() > 64 * 1024 {
            let _ = child.kill();
            let _ = child.wait();
            return Err(driver_failed(action));
        }
        let response: LockResponse = parse_driver_response(action, &line)?;
        if child
            .try_wait()
            .map_err(|_| driver_unavailable(action))?
            .is_some()
        {
            return Err(driver_failed(action));
        }
        if response.operation_id != operation_id || !response.lock_held {
            let _ = child.kill();
            let _ = child.wait();
            return Err(driver_failed(action));
        }
        Ok(Box::new(PhysicalBackendLock {
            child: Mutex::new(Some(child)),
        }))
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct DriverResponse {
    format_version: u32,
    action: String,
    status: String,
    data: Value,
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct LockResponse {
    operation_id: Uuid,
    lock_held: bool,
}

fn driver_request(action: &str, profile: DeploymentProfile, payload: Value) -> Value {
    json!({
        "format_version": DRIVER_PROTOCOL_FORMAT,
        "action": action,
        "profile": profile,
        "payload": payload,
    })
}

fn parse_driver_response<T: DeserializeOwned + Serialize>(
    action: &'static str,
    bytes: &[u8],
) -> Result<T, UpgradeError> {
    let response: DriverResponse =
        serde_json::from_slice(bytes).map_err(|_| driver_failed(action))?;
    if response.format_version != DRIVER_PROTOCOL_FORMAT
        || response.action != action
        || response.status != "pass"
    {
        return Err(driver_failed(action));
    }
    let parsed: T =
        serde_json::from_value(response.data.clone()).map_err(|_| driver_failed(action))?;
    if serde_json::to_value(&parsed).map_err(serialization)? != response.data {
        return Err(driver_failed(action));
    }
    Ok(parsed)
}

fn require_executable(path: &Path) -> Result<(), UpgradeError> {
    if !path.is_absolute() {
        return Err(UpgradeError::Prerequisite {
            message: "physical Driver path must be absolute".to_owned(),
        });
    }
    let metadata = fs::symlink_metadata(path).map_err(|_| UpgradeError::Prerequisite {
        message: "physical Driver executable is unavailable".to_owned(),
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(UpgradeError::Prerequisite {
            message: "physical Driver must be a regular non-symlink executable".to_owned(),
        });
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err(UpgradeError::Prerequisite {
                message: "physical Driver file is not executable".to_owned(),
            });
        }
    }
    Ok(())
}

fn driver_unavailable(action: &'static str) -> UpgradeError {
    UpgradeError::Prerequisite {
        message: format!("physical Driver is unavailable for {action}"),
    }
}

fn driver_failed(action: &'static str) -> UpgradeError {
    UpgradeError::EvidenceInvalid {
        phase: action_phase(action),
        message: format!("physical Driver failed closed for {action}"),
    }
}

fn action_phase(action: &str) -> muriarc_upgrade::UpgradePhase {
    use muriarc_upgrade::UpgradePhase;
    match action {
        "preflight" => UpgradePhase::PreflightPassed,
        "drain" => UpgradePhase::Drained,
        "freeze_writes" => UpgradePhase::WritesFrozen,
        "create_backup" => UpgradePhase::BackupCreated,
        "verify_backup_restore" => UpgradePhase::BackupRestored,
        "prepare_candidate" => UpgradePhase::CandidatePrepared,
        "migrate_candidate" => UpgradePhase::CandidateMigrated,
        "verify_candidate" => UpgradePhase::CandidateVerified,
        "switch_generation" => UpgradePhase::Switched,
        "activate_read_only" => UpgradePhase::ReadOnlyActivated,
        "verify_activated" => UpgradePhase::ActivationVerified,
        "open_write_lease" => UpgradePhase::WriteLeaseOpened,
        _ => UpgradePhase::Initialized,
    }
}

fn serialization(error: serde_json::Error) -> UpgradeError {
    UpgradeError::Persistence {
        message: format!("physical Driver request cannot be serialized: {error}"),
    }
}

struct PhysicalBackendLock {
    child: Mutex<Option<Child>>,
}

impl Drop for PhysicalBackendLock {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.child.lock()
            && let Some(mut child) = guard.take()
        {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[async_trait]
impl UpgradeDriver for PhysicalDriverClient {
    fn profile(&self) -> DeploymentProfile {
        self.profile
    }

    async fn acquire_backend_lock(
        &self,
        operation_id: Uuid,
    ) -> Result<Box<dyn BackendUpgradeLock>, UpgradeError> {
        self.acquire_lock(operation_id)
    }

    async fn current_generation(&self) -> Result<ActiveGeneration, UpgradeError> {
        self.invoke("current_generation", json!({}))
    }

    async fn create_operation(&self, snapshot: &UpgradeSnapshot) -> Result<(), UpgradeError> {
        self.invoke("create_operation", json!({ "snapshot": snapshot }))
    }

    async fn save_operation(&self, snapshot: &UpgradeSnapshot) -> Result<(), UpgradeError> {
        self.invoke("save_operation", json!({ "snapshot": snapshot }))
    }

    async fn load_operation(&self, operation_id: Uuid) -> Result<UpgradeSnapshot, UpgradeError> {
        self.invoke("load_operation", json!({ "operation_id": operation_id }))
    }

    async fn preflight(
        &self,
        snapshot: &UpgradeSnapshot,
        target: &VerifiedRelease,
    ) -> Result<PreflightEvidence, UpgradeError> {
        self.invoke(
            "preflight",
            json!({ "snapshot": snapshot, "target": target }),
        )
    }

    async fn drain(&self, snapshot: &UpgradeSnapshot) -> Result<DrainEvidence, UpgradeError> {
        self.invoke("drain", json!({ "snapshot": snapshot }))
    }

    async fn freeze_writes(
        &self,
        snapshot: &UpgradeSnapshot,
    ) -> Result<FreezeEvidence, UpgradeError> {
        self.invoke("freeze_writes", json!({ "snapshot": snapshot }))
    }

    async fn create_backup(
        &self,
        snapshot: &UpgradeSnapshot,
    ) -> Result<BackupEvidence, UpgradeError> {
        self.invoke("create_backup", json!({ "snapshot": snapshot }))
    }

    async fn verify_backup_restore(
        &self,
        snapshot: &UpgradeSnapshot,
        backup: &BackupEvidence,
    ) -> Result<RestoreEvidence, UpgradeError> {
        self.invoke(
            "verify_backup_restore",
            json!({ "snapshot": snapshot, "backup": backup }),
        )
    }

    async fn prepare_candidate(
        &self,
        snapshot: &UpgradeSnapshot,
        restore: &RestoreEvidence,
        target: &VerifiedRelease,
    ) -> Result<CandidateEvidence, UpgradeError> {
        self.invoke(
            "prepare_candidate",
            json!({ "snapshot": snapshot, "restore": restore, "target": target }),
        )
    }

    async fn migrate_candidate(
        &self,
        snapshot: &UpgradeSnapshot,
        candidate: &CandidateEvidence,
        target: &VerifiedRelease,
    ) -> Result<MigrationEvidence, UpgradeError> {
        self.invoke(
            "migrate_candidate",
            json!({ "snapshot": snapshot, "candidate": candidate, "target": target }),
        )
    }

    async fn verify_candidate(
        &self,
        snapshot: &UpgradeSnapshot,
        candidate: &CandidateEvidence,
    ) -> Result<VerificationEvidence, UpgradeError> {
        self.invoke(
            "verify_candidate",
            json!({ "snapshot": snapshot, "candidate": candidate }),
        )
    }

    async fn switch_generation(
        &self,
        snapshot: &UpgradeSnapshot,
        candidate: &CandidateEvidence,
    ) -> Result<SwitchEvidence, UpgradeError> {
        self.invoke(
            "switch_generation",
            json!({ "snapshot": snapshot, "candidate": candidate }),
        )
    }

    async fn activate_read_only(
        &self,
        snapshot: &UpgradeSnapshot,
        candidate: &CandidateEvidence,
    ) -> Result<ReadOnlyActivationEvidence, UpgradeError> {
        self.invoke(
            "activate_read_only",
            json!({ "snapshot": snapshot, "candidate": candidate }),
        )
    }

    async fn verify_activated(
        &self,
        snapshot: &UpgradeSnapshot,
        candidate: &CandidateEvidence,
    ) -> Result<ActivationVerificationEvidence, UpgradeError> {
        self.invoke(
            "verify_activated",
            json!({ "snapshot": snapshot, "candidate": candidate }),
        )
    }

    async fn open_write_lease(
        &self,
        snapshot: &UpgradeSnapshot,
        candidate: &CandidateEvidence,
    ) -> Result<WriteLeaseEvidence, UpgradeError> {
        self.invoke(
            "open_write_lease",
            json!({ "snapshot": snapshot, "candidate": candidate }),
        )
    }

    async fn first_write_at(
        &self,
        generation_id: Uuid,
    ) -> Result<Option<DateTime<Utc>>, UpgradeError> {
        self.invoke("first_write_at", json!({ "generation_id": generation_id }))
    }

    async fn recover_before_first_write(
        &self,
        snapshot: &UpgradeSnapshot,
    ) -> Result<(), UpgradeError> {
        self.invoke(
            "recover_before_first_write",
            json!({ "snapshot": snapshot }),
        )
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::fs::PermissionsExt as _;

    use tempfile::tempdir;

    use super::*;

    static DRIVER_PROCESS_TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn protocol_accepts_only_exact_typed_success_response() {
        let _driver_process_guard = DRIVER_PROCESS_TEST_LOCK.lock().unwrap();
        let temporary = tempdir().unwrap();
        let driver = temporary.path().join("driver.sh");
        fs::write(
            &driver,
            format!(
                "#!/usr/bin/env bash\ncat >/dev/null\nprintf '%s\\n' '{}'\n",
                json!({
                    "format_version": 1,
                    "action": "current_generation",
                    "status": "pass",
                    "data": {
                        "generation_id": "11111111-1111-4111-8111-111111111111",
                        "identity": {
                            "application_version": "1.0.0",
                            "data_epoch": "E0001",
                            "backend_state_digest": format!("sha256:{}", "a".repeat(64)),
                            "gateway_contract_revision": "gateway-v1",
                        },
                        "backend": "postgres",
                        "first_write_at": null,
                    },
                })
            ),
        )
        .unwrap();
        fs::set_permissions(&driver, fs::Permissions::from_mode(0o700)).unwrap();
        let client = PhysicalDriverClient::new(&driver, DeploymentProfile::ManagedCompose).unwrap();
        let generation: ActiveGeneration = client.invoke("current_generation", json!({})).unwrap();
        assert_eq!(
            generation.generation_id,
            Uuid::parse_str("11111111-1111-4111-8111-111111111111").unwrap()
        );
    }

    #[test]
    fn protocol_rejects_symlinked_or_schema_drifting_driver() {
        let _driver_process_guard = DRIVER_PROCESS_TEST_LOCK.lock().unwrap();
        let temporary = tempdir().unwrap();
        let target = temporary.path().join("target.sh");
        fs::write(
            &target,
            "#!/usr/bin/env bash\ncat >/dev/null\nprintf '%s\\n' '{\"format_version\":1,\"action\":\"current_generation\",\"status\":\"pass\",\"data\":{},\"extra\":true}'\n",
        )
        .unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o700)).unwrap();
        let link = temporary.path().join("driver-link");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(PhysicalDriverClient::new(&link, DeploymentProfile::NativeSystem).is_err());

        let client = PhysicalDriverClient::new(&target, DeploymentProfile::NativeSystem).unwrap();
        assert!(
            client
                .invoke::<ActiveGeneration>("current_generation", json!({}))
                .is_err()
        );
    }
}
