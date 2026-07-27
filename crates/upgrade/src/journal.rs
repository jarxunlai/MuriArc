use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, ErrorKind, Write},
    path::{Path, PathBuf},
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{BackupEvidence, RestoreEvidence, UpgradeError, UpgradeSnapshot};

const GENESIS_DIGEST: &str = "genesis";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostLockRecord {
    pub operation_id: Uuid,
    pub process_id: u32,
    pub acquired_at: DateTime<Utc>,
    pub nonce: Uuid,
}

pub struct HostUpgradeLock {
    path: PathBuf,
    record: HostLockRecord,
    #[allow(dead_code)]
    file: File,
}

impl HostUpgradeLock {
    pub fn acquire(state_root: &Path, operation_id: Uuid) -> Result<Self, UpgradeError> {
        fs::create_dir_all(state_root).map_err(persistence)?;
        let path = state_root.join("upgrade.lock");
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = match options.open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                return Err(UpgradeError::LockBusy);
            }
            Err(error) => return Err(persistence(error)),
        };
        let record = HostLockRecord {
            operation_id,
            process_id: std::process::id(),
            acquired_at: Utc::now(),
            nonce: Uuid::new_v4(),
        };
        serde_json::to_writer(&mut file, &record).map_err(|error| UpgradeError::Persistence {
            message: error.to_string(),
        })?;
        file.write_all(b"\n").map_err(persistence)?;
        file.sync_all().map_err(persistence)?;
        Ok(Self { path, record, file })
    }

    pub fn acquire_for_resume(state_root: &Path, operation_id: Uuid) -> Result<Self, UpgradeError> {
        match Self::acquire(state_root, operation_id) {
            Ok(lock) => return Ok(lock),
            Err(UpgradeError::LockBusy) => {}
            Err(error) => return Err(error),
        }
        let existing = Self::inspect(state_root)?.ok_or(UpgradeError::LockBusy)?;
        if existing.operation_id != operation_id || process_is_alive(existing.process_id)? {
            return Err(UpgradeError::LockBusy);
        }
        let stale_directory = state_root.join("stale-locks");
        fs::create_dir_all(&stale_directory).map_err(persistence)?;
        let stale_path =
            stale_directory.join(format!("{}-{}.json", existing.operation_id, existing.nonce));
        match fs::rename(state_root.join("upgrade.lock"), &stale_path) {
            Ok(()) => Self::acquire(state_root, operation_id),
            Err(error) if error.kind() == ErrorKind::NotFound => {
                Self::acquire(state_root, operation_id)
            }
            Err(error) => Err(persistence(error)),
        }
    }

    pub fn record(&self) -> &HostLockRecord {
        &self.record
    }

    pub fn inspect(state_root: &Path) -> Result<Option<HostLockRecord>, UpgradeError> {
        let path = state_root.join("upgrade.lock");
        match fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map(Some).map_err(|error| {
                UpgradeError::JournalIntegrity {
                    message: format!("host lock is malformed: {error}"),
                }
            }),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
            Err(error) => Err(persistence(error)),
        }
    }
}

impl Drop for HostUpgradeLock {
    fn drop(&mut self) {
        let owned = fs::read(&self.path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<HostLockRecord>(&bytes).ok())
            .is_some_and(|record| record.nonce == self.record.nonce);
        if owned {
            let _ = fs::remove_file(&self.path);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalRecord {
    pub sequence: u64,
    pub previous_digest: String,
    pub snapshot_digest: String,
    pub record_digest: String,
    pub written_at: DateTime<Utc>,
    pub snapshot: UpgradeSnapshot,
}

#[derive(Debug, Clone)]
pub struct UpgradeJournal {
    path: PathBuf,
}

impl UpgradeJournal {
    pub fn new(state_root: &Path, operation_id: Uuid) -> Result<Self, UpgradeError> {
        let directory = state_root.join("operations");
        fs::create_dir_all(&directory).map_err(persistence)?;
        Ok(Self {
            path: directory.join(format!("{operation_id}.jsonl")),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn append(&self, snapshot: &UpgradeSnapshot) -> Result<JournalRecord, UpgradeError> {
        let records = self.read_verified()?;
        if let Some(last) = records.last()
            && (last.snapshot.operation_id != snapshot.operation_id
                || last.snapshot.revision >= snapshot.revision)
        {
            return Err(UpgradeError::JournalIntegrity {
                message: "journal revisions must increase monotonically".to_owned(),
            });
        }
        let sequence = records.last().map_or(1, |record| record.sequence + 1);
        let previous_digest = records.last().map_or_else(
            || GENESIS_DIGEST.to_owned(),
            |record| record.record_digest.clone(),
        );
        let snapshot_digest = digest_json(snapshot)?;
        let record_digest = chain_digest(sequence, &previous_digest, &snapshot_digest);
        let record = JournalRecord {
            sequence,
            previous_digest,
            snapshot_digest,
            record_digest,
            written_at: Utc::now(),
            snapshot: snapshot.clone(),
        };
        let mut options = OpenOptions::new();
        options.append(true).create(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&self.path).map_err(persistence)?;
        serde_json::to_writer(&mut file, &record).map_err(|error| UpgradeError::Persistence {
            message: error.to_string(),
        })?;
        file.write_all(b"\n").map_err(persistence)?;
        file.sync_data().map_err(persistence)?;
        Ok(record)
    }

    pub fn latest(&self) -> Result<Option<UpgradeSnapshot>, UpgradeError> {
        Ok(self
            .read_verified()?
            .last()
            .map(|record| record.snapshot.clone()))
    }

    pub fn read_verified(&self) -> Result<Vec<JournalRecord>, UpgradeError> {
        let file = match File::open(&self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(persistence(error)),
        };
        let mut records = Vec::new();
        let mut expected_previous = GENESIS_DIGEST.to_owned();
        for (index, line) in BufReader::new(file).lines().enumerate() {
            let line = line.map_err(persistence)?;
            if line.trim().is_empty() {
                return Err(UpgradeError::JournalIntegrity {
                    message: format!("journal line {} is empty", index + 1),
                });
            }
            let record: JournalRecord =
                serde_json::from_str(&line).map_err(|error| UpgradeError::JournalIntegrity {
                    message: format!("journal line {} is malformed: {error}", index + 1),
                })?;
            let expected_sequence = u64::try_from(index).unwrap_or(u64::MAX) + 1;
            let snapshot_digest = digest_json(&record.snapshot)?;
            let record_digest = chain_digest(
                record.sequence,
                &record.previous_digest,
                &record.snapshot_digest,
            );
            if record.sequence != expected_sequence
                || record.previous_digest != expected_previous
                || record.snapshot_digest != snapshot_digest
                || record.record_digest != record_digest
            {
                return Err(UpgradeError::JournalIntegrity {
                    message: format!("journal hash chain failed at line {}", index + 1),
                });
            }
            expected_previous = record.record_digest.clone();
            records.push(record);
        }
        Ok(records)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifiedRecoveryPoint {
    pub backup: BackupEvidence,
    pub restore: RestoreEvidence,
    pub registered_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryPointCatalog {
    pub format_version: u32,
    pub last_verified_backup_id: Option<Uuid>,
    pub points: BTreeMap<Uuid, VerifiedRecoveryPoint>,
}

impl Default for RecoveryPointCatalog {
    fn default() -> Self {
        Self {
            format_version: 1,
            last_verified_backup_id: None,
            points: BTreeMap::new(),
        }
    }
}

impl RecoveryPointCatalog {
    pub fn load(state_root: &Path) -> Result<Self, UpgradeError> {
        let path = recovery_catalog_path(state_root);
        match fs::read(&path) {
            Ok(bytes) => {
                let catalog: Self = serde_json::from_slice(&bytes).map_err(|error| {
                    UpgradeError::JournalIntegrity {
                        message: format!("recovery catalog is malformed: {error}"),
                    }
                })?;
                if catalog.format_version != 1 {
                    return Err(UpgradeError::JournalIntegrity {
                        message: "recovery catalog format is unsupported".to_owned(),
                    });
                }
                Ok(catalog)
            }
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(persistence(error)),
        }
    }

    pub fn register_verified(
        &mut self,
        backup: BackupEvidence,
        restore: RestoreEvidence,
    ) -> Result<(), UpgradeError> {
        backup.validate(backup.source_generation_id)?;
        restore.validate(&backup)?;
        let backup_id = backup.backup_id;
        self.points.insert(
            backup_id,
            VerifiedRecoveryPoint {
                backup,
                restore,
                registered_at: Utc::now(),
            },
        );
        self.last_verified_backup_id = Some(backup_id);
        Ok(())
    }

    pub fn require_prunable(
        &self,
        backup_id: Uuid,
    ) -> Result<&VerifiedRecoveryPoint, UpgradeError> {
        let point = self
            .points
            .get(&backup_id)
            .ok_or_else(|| UpgradeError::Prerequisite {
                message: "recovery point does not exist".to_owned(),
            })?;
        if self.last_verified_backup_id == Some(backup_id) {
            return Err(UpgradeError::Prerequisite {
                message: "the last verified recovery point cannot be pruned".to_owned(),
            });
        }
        Ok(point)
    }

    /// Call only after the deployment Driver has deleted the exact artifact
    /// authorized by require_prunable.
    pub fn commit_pruned(&mut self, backup_id: Uuid) -> Result<(), UpgradeError> {
        self.require_prunable(backup_id)?;
        self.points.remove(&backup_id);
        Ok(())
    }

    pub fn save_atomic(&self, state_root: &Path) -> Result<(), UpgradeError> {
        fs::create_dir_all(state_root).map_err(persistence)?;
        let path = recovery_catalog_path(state_root);
        let temporary = path.with_extension(format!("tmp-{}", Uuid::new_v4()));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary).map_err(persistence)?;
        serde_json::to_writer_pretty(&mut file, self).map_err(|error| {
            UpgradeError::Persistence {
                message: error.to_string(),
            }
        })?;
        file.write_all(b"\n").map_err(persistence)?;
        file.sync_all().map_err(persistence)?;
        match fs::rename(&temporary, &path) {
            Ok(()) => Ok(()),
            Err(error) => {
                let _ = fs::remove_file(&temporary);
                Err(persistence(error))
            }
        }
    }
}

fn recovery_catalog_path(state_root: &Path) -> PathBuf {
    state_root.join("recovery-points.json")
}

fn digest_json(value: &impl Serialize) -> Result<String, UpgradeError> {
    let bytes = serde_json::to_vec(value).map_err(|error| UpgradeError::Persistence {
        message: error.to_string(),
    })?;
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}

fn chain_digest(sequence: u64, previous: &str, snapshot: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"MuriArc/upgrade-journal/v1\0");
    hasher.update(sequence.to_be_bytes());
    hasher.update(previous.as_bytes());
    hasher.update(b"\0");
    hasher.update(snapshot.as_bytes());
    format!("sha256:{:x}", hasher.finalize())
}

fn persistence(error: std::io::Error) -> UpgradeError {
    UpgradeError::Persistence {
        message: error.to_string(),
    }
}

#[cfg(unix)]
fn process_is_alive(process_id: u32) -> Result<bool, UpgradeError> {
    let process_id = i32::try_from(process_id).map_err(|_| UpgradeError::JournalIntegrity {
        message: "host lock process identifier is outside the platform range".to_owned(),
    })?;
    // SAFETY: kill with signal 0 performs only an existence/permission check
    // for the numeric PID and does not send a signal.
    let result = unsafe { libc::kill(process_id, 0) };
    if result == 0 {
        return Ok(true);
    }
    match std::io::Error::last_os_error().raw_os_error() {
        Some(libc::ESRCH) => Ok(false),
        Some(libc::EPERM) => Ok(true),
        _ => Err(UpgradeError::JournalIntegrity {
            message: "host lock process liveness could not be determined".to_owned(),
        }),
    }
}

#[cfg(windows)]
fn process_is_alive(process_id: u32) -> Result<bool, UpgradeError> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, ERROR_ACCESS_DENIED, ERROR_INVALID_PARAMETER, GetLastError},
        System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
    };
    // SAFETY: OpenProcess receives a numeric PID and no inheritable handles.
    // A successful handle is closed on every path below.
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process_id) };
    if !handle.is_null() {
        // SAFETY: handle was returned by OpenProcess and is closed exactly once.
        unsafe {
            CloseHandle(handle);
        }
        return Ok(true);
    }
    // SAFETY: GetLastError has no preconditions and reads thread-local state.
    match unsafe { GetLastError() } {
        ERROR_INVALID_PARAMETER => Ok(false),
        ERROR_ACCESS_DENIED => Ok(true),
        _ => Err(UpgradeError::JournalIntegrity {
            message: "host lock process liveness could not be determined".to_owned(),
        }),
    }
}

#[cfg(not(any(unix, windows)))]
fn process_is_alive(_process_id: u32) -> Result<bool, UpgradeError> {
    Ok(true)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use muriarc_core::{
        BackendKind, BackendStateDigest, GatewayContractRevision, ReleaseArtifact, ReleaseManifest,
    };

    use super::*;
    use crate::{
        ActiveGeneration, DeploymentProfile, RecoveryComponent, TrustedMetadataVersions,
        UpgradePhase, VerifiedRelease,
    };

    fn snapshot() -> UpgradeSnapshot {
        let digest: BackendStateDigest = format!("sha256:{}", "a".repeat(64)).parse().unwrap();
        let manifest = ReleaseManifest {
            format_version: 1,
            application_version: "1.0.0".parse().unwrap(),
            data_epoch: "E0001".parse().unwrap(),
            gateway_contract_revision: "gateway-v1".parse::<GatewayContractRevision>().unwrap(),
            backend_states: BTreeMap::from([
                (BackendKind::Sqlite, digest.clone()),
                (BackendKind::Postgres, digest.clone()),
            ]),
            postgres_major: 17,
            bootstrap_protocol_revision: 1,
            controller_protocol_min: 1,
            controller_protocol_max: 1,
            migration_class: muriarc_core::MigrationClass::M3,
            artifacts: BTreeMap::from([(
                "test".to_owned(),
                ReleaseArtifact {
                    media_type: "application/octet-stream".to_owned(),
                    digest,
                    size_bytes: 1,
                },
            )]),
        };
        let target = VerifiedRelease {
            manifest,
            target_name: "test".to_owned(),
            target_length: 1,
            target_digest: format!("sha256:{}", "b".repeat(64)),
            metadata_versions: TrustedMetadataVersions {
                root: 1,
                timestamp: 1,
                snapshot: 1,
                targets: 1,
            },
            metadata_expires_at: Utc::now() + chrono::Duration::hours(1),
        };
        let source = ActiveGeneration {
            generation_id: Uuid::new_v4(),
            identity: muriarc_core::ReleaseIdentity::parse(
                "0.1.0".to_owned(),
                "preview_epoch_0".to_owned(),
                format!("sha256:{}", "c".repeat(64)),
                "gateway-v1".to_owned(),
            )
            .unwrap(),
            backend: BackendKind::Postgres,
            first_write_at: None,
        };
        UpgradeSnapshot::new(
            Uuid::new_v4(),
            DeploymentProfile::NativeSystem,
            &source,
            &target,
        )
        .unwrap()
    }

    #[test]
    fn host_lock_is_exclusive_and_owner_scoped() {
        let root = tempfile::tempdir().unwrap();
        let first = HostUpgradeLock::acquire(root.path(), Uuid::new_v4()).unwrap();
        assert!(matches!(
            HostUpgradeLock::acquire(root.path(), Uuid::new_v4()),
            Err(UpgradeError::LockBusy)
        ));
        drop(first);
        HostUpgradeLock::acquire(root.path(), Uuid::new_v4()).unwrap();
    }

    #[test]
    #[cfg(any(unix, windows))]
    fn resume_reclaims_only_a_proven_stale_lock_for_the_same_operation() {
        let root = tempfile::tempdir().unwrap();
        let operation_id = Uuid::new_v4();
        let stale = HostLockRecord {
            operation_id,
            process_id: i32::MAX as u32,
            acquired_at: Utc::now(),
            nonce: Uuid::new_v4(),
        };
        fs::write(
            root.path().join("upgrade.lock"),
            serde_json::to_vec(&stale).unwrap(),
        )
        .unwrap();
        let lock = HostUpgradeLock::acquire_for_resume(root.path(), operation_id).unwrap();
        assert_eq!(lock.record().operation_id, operation_id);
        assert_eq!(
            fs::read_dir(root.path().join("stale-locks"))
                .unwrap()
                .count(),
            1
        );
        drop(lock);
        assert!(HostUpgradeLock::inspect(root.path()).unwrap().is_none());
    }

    #[test]
    fn journal_detects_tampering() {
        let root = tempfile::tempdir().unwrap();
        let mut snapshot = snapshot();
        snapshot.advance(UpgradePhase::LocksAcquired).unwrap();
        let journal = UpgradeJournal::new(root.path(), snapshot.operation_id).unwrap();
        journal.append(&snapshot).unwrap();
        let mut bytes = fs::read(journal.path()).unwrap();
        let position = bytes.iter().position(|byte| *byte == b'a').unwrap();
        bytes[position] = b'z';
        fs::write(journal.path(), bytes).unwrap();
        assert!(matches!(
            journal.read_verified(),
            Err(UpgradeError::JournalIntegrity { .. })
        ));
    }

    #[test]
    fn recovery_catalog_never_prunes_last_verified_point() {
        fn point(source: Uuid) -> (BackupEvidence, RestoreEvidence) {
            let backup = BackupEvidence {
                backup_id: Uuid::new_v4(),
                source_generation_id: source,
                artifact_digest: format!("sha256:{}", "a".repeat(64)),
                recovery_set_digest: format!("sha256:{}", "b".repeat(64)),
                components: RecoveryComponent::required(),
                created_at: Utc::now(),
            };
            let restore = RestoreEvidence {
                backup_id: backup.backup_id,
                backup_artifact_digest: backup.artifact_digest.clone(),
                restored_generation_id: Uuid::new_v4(),
                isolated_restore: true,
                verified_at: Utc::now(),
            };
            (backup, restore)
        }

        let source = Uuid::new_v4();
        let (first, first_restore) = point(source);
        let first_id = first.backup_id;
        let (second, second_restore) = point(source);
        let second_id = second.backup_id;
        let mut catalog = RecoveryPointCatalog::default();
        catalog.register_verified(first, first_restore).unwrap();
        catalog.register_verified(second, second_restore).unwrap();
        assert!(catalog.require_prunable(first_id).is_ok());
        assert!(catalog.require_prunable(second_id).is_err());
        catalog.commit_pruned(first_id).unwrap();
        assert!(!catalog.points.contains_key(&first_id));
        assert!(catalog.points.contains_key(&second_id));
    }
}
