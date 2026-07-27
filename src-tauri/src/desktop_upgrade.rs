use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    process::Command,
    sync::Arc,
};

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use chrono::{DateTime, Duration, Utc};
use fs2::{FileExt, available_space};
use minisign_verify::{PublicKey, Signature};
use muriarc_core::{
    BackendKind, DeploymentGenerationManifest, MuriArcStore, PersistentRecoveryInventory,
    ReleaseIdentity, ReleaseManifest,
};
use muriarc_store_sqlite::SqliteStore;
use muriarc_upgrade::{
    ActivationVerificationEvidence, ActiveGeneration, BackendUpgradeLock, BackupEvidence,
    CandidateEvidence, DeploymentProfile, DrainEvidence, FreezeEvidence, MigrationEvidence,
    PreflightEvidence, ReadOnlyActivationEvidence, RecoveryComponent, RestoreEvidence,
    SwitchEvidence, UpgradeDriver, UpgradeEngine, UpgradeError, UpgradePhase, UpgradeSnapshot,
    UpgradeStatus, VerificationEvidence, VerificationLayer, VerificationLayerEvidence,
    VerifiedRelease, WriteLeaseEvidence,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{Connection, Row, SqlitePool};
use thiserror::Error;
use uuid::Uuid;

use crate::{
    runtime_compatibility::{verify_or_create_manifest, write_manifest_atomic},
    storage_root::{
        StorageRootError, TreeDigest, activate_root_for_upgrade, checkpoint_and_verify_database,
        copy_managed_tree, payload_tree_digest, read_json_optional, recover_atomic_file,
        remove_atomic_file, resolve_active_root_for_upgrade, tree_digest,
        verify_database_read_only, write_json_atomic,
    },
};

const INTENT_VERSION: u32 = 2;
const INTENT_FILE: &str = "desktop-upgrade-intent.json";
const BINARY_FALLBACK_FILE: &str = "desktop-binary-fallback.json";
const BINARY_RECOVERY_DIRECTORY: &str = "desktop-binary-recovery";
const FALLBACK_EXECUTABLE_FILE: &str = "MuriArc-fallback.exe";
const HISTORY_DIRECTORY: &str = "desktop-upgrade-history";
const GENERATION_MANIFEST_FILE: &str = "deployment-generation.json";
const DATABASE_FILE: &str = "muriarc.sqlite3";
const MINIMUM_MIGRATION_HEADROOM: u64 = 512 * 1024 * 1024;
const DESKTOP_TARGET_VALIDITY: Duration = Duration::hours(24);
const WRITE_LEASE_HOLDER: &str = "desktop-updater";

#[derive(Debug, Error)]
pub(crate) enum DesktopUpgradeError {
    #[error("another Desktop upgrade is already pending or running")]
    OperationBusy,
    #[error("signed updater metadata is incomplete or inconsistent")]
    TargetInvalid,
    #[error("the installed Desktop binary does not match the verified target")]
    BinaryVersionMismatch,
    #[error("the Desktop upgrade journal is malformed or was modified")]
    JournalIntegrity,
    #[error("the Desktop recovery set is incomplete or corrupted")]
    RecoveryInvalid,
    #[error("the Desktop Candidate failed compatibility or data verification")]
    CandidateInvalid,
    #[error("insufficient free space for a recovery copy and isolated Candidate")]
    InsufficientSpace,
    #[error("automatic rollback is forbidden after the Candidate's first write")]
    FirstWriteBlocksRollback,
    #[error("Desktop update requires the signed Windows installer")]
    UnsupportedPlatform,
    #[error("the previous Desktop executable recovery copy is missing or invalid")]
    BinaryRecoveryInvalid,
    #[error("Desktop upgrade storage operation failed")]
    Storage(#[from] StorageRootError),
    #[error("Desktop upgrade database operation failed")]
    Store(#[from] muriarc_core::StoreError),
    #[error("Desktop upgrade I/O failed")]
    Io(#[from] std::io::Error),
    #[error("Desktop upgrade metadata failed to decode")]
    Metadata(#[from] serde_json::Error),
    #[error("Desktop upgrade database verification failed")]
    Database(#[from] sqlx::Error),
    #[error("Desktop upgrade control plane failed")]
    ControlPlane(#[from] muriarc_upgrade::UpgradeError),
}

impl DesktopUpgradeError {
    pub(crate) const fn code(&self) -> &'static str {
        match self {
            Self::OperationBusy => "desktop_upgrade_busy",
            Self::TargetInvalid => "desktop_update_target_invalid",
            Self::BinaryVersionMismatch => "desktop_update_binary_mismatch",
            Self::JournalIntegrity => "desktop_upgrade_journal_invalid",
            Self::RecoveryInvalid => "desktop_upgrade_recovery_invalid",
            Self::CandidateInvalid => "desktop_upgrade_candidate_invalid",
            Self::InsufficientSpace => "desktop_upgrade_insufficient_space",
            Self::FirstWriteBlocksRollback => "desktop_upgrade_forward_fix_required",
            Self::UnsupportedPlatform => "desktop_update_unsupported_platform",
            Self::BinaryRecoveryInvalid => "desktop_update_binary_recovery_invalid",
            Self::Storage(_) | Self::Store(_) | Self::Io(_) | Self::Database(_) => {
                "desktop_upgrade_storage_error"
            }
            Self::Metadata(_) | Self::ControlPlane(_) => "desktop_upgrade_control_error",
        }
    }

    pub(crate) const fn safe_message(&self) -> &'static str {
        match self {
            Self::OperationBusy => "已有 Desktop 更新等待处理，请先完成或恢复该操作",
            Self::TargetInvalid => "更新元数据、版本或制品摘要不一致，已拒绝安装",
            Self::BinaryVersionMismatch => "当前程序不是已验证的目标版本，已阻止数据切换",
            Self::JournalIntegrity => "更新日志不完整或被修改，已停止升级并保留原数据",
            Self::RecoveryInvalid => "恢复副本未通过完整性验证，已停止升级并保留原数据",
            Self::CandidateInvalid => "候选数据未通过完整验证，原数据位置没有切换",
            Self::InsufficientSpace => "磁盘空间不足，无法同时保留恢复点和候选数据",
            Self::FirstWriteBlocksRollback => {
                "新版本已经写入数据，禁止自动降级；请使用只读修复或显式恢复"
            }
            Self::UnsupportedPlatform => "正式 Desktop 更新仅支持签名 Windows 安装包",
            Self::BinaryRecoveryInvalid => "旧版程序恢复副本缺失或损坏，已停止更新且不会切换数据",
            Self::Storage(_) | Self::Store(_) | Self::Io(_) | Self::Database(_) => {
                "Desktop 更新的数据保护步骤失败，原数据和恢复点均未自动删除"
            }
            Self::Metadata(_) | Self::ControlPlane(_) => "Desktop 更新控制信息无效，已停止升级",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct VerifiedDesktopUpdate {
    pub(crate) version: String,
    pub(crate) updater_target: String,
    pub(crate) artifact_name: String,
    pub(crate) artifact_sha256: String,
    pub(crate) artifact_size_bytes: u64,
    pub(crate) release_manifest: ReleaseManifest,
    pub(crate) release_manifest_signature: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DesktopUpgradeSpace {
    pub(crate) data_free_bytes: u64,
    pub(crate) data_required_bytes: u64,
    pub(crate) control_free_bytes: u64,
    pub(crate) control_required_bytes: u64,
    pub(crate) sufficient: bool,
}

pub(crate) fn upgrade_space_preflight(
    config_root: &Path,
    source_executable: &Path,
    artifact_size_bytes: u64,
) -> Result<DesktopUpgradeSpace, DesktopUpgradeError> {
    let config_root = canonical_real_directory(config_root)?;
    let source_root = resolve_active_root_for_upgrade(&config_root)?;
    let source_tree = tree_digest(&source_root)?;
    let data_required_bytes = source_tree
        .total_bytes
        .checked_mul(2)
        .and_then(|bytes| bytes.checked_add(MINIMUM_MIGRATION_HEADROOM))
        .ok_or(DesktopUpgradeError::InsufficientSpace)?;
    let data_volume = source_root
        .parent()
        .ok_or(DesktopUpgradeError::InsufficientSpace)?;
    let data_free_bytes = available_space(data_volume)?;
    let control_required_bytes = require_real_file_size(source_executable)?
        .checked_add(artifact_size_bytes)
        .ok_or(DesktopUpgradeError::InsufficientSpace)?;
    let control_free_bytes = available_space(&config_root)?;
    Ok(DesktopUpgradeSpace {
        data_free_bytes,
        data_required_bytes,
        control_free_bytes,
        control_required_bytes,
        sufficient: data_free_bytes >= data_required_bytes
            && control_free_bytes >= control_required_bytes,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DesktopUpgradeIntent {
    version: u32,
    operation_id: Uuid,
    source_data_root: PathBuf,
    source_generation_id: Uuid,
    source_application_version: String,
    candidate_generation_id: Uuid,
    updater_target: String,
    artifact_name: String,
    artifact_sha256: String,
    artifact_size_bytes: u64,
    source_executable_sha256: String,
    source_executable_size_bytes: u64,
    release_manifest: ReleaseManifest,
    release_manifest_signature: String,
    requested_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum BinaryFallbackState {
    Armed,
    Fallback,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DesktopBinaryFallback {
    version: u32,
    operation_id: Uuid,
    state: BinaryFallbackState,
    source_application_version: String,
    failed_application_version: String,
    source_generation_id: Uuid,
    intent_digest: String,
    executable_sha256: String,
    executable_size_bytes: u64,
    created_at: DateTime<Utc>,
    activated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CompletedDesktopUpgrade {
    intent: DesktopUpgradeIntent,
    final_snapshot: UpgradeSnapshot,
    recovery_root: PathBuf,
    candidate_root: PathBuf,
    completed_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
struct DesktopUpgradePaths {
    operation_root: PathBuf,
    recovery_root: PathBuf,
    candidate_root: PathBuf,
    binary_recovery_root: PathBuf,
    fallback_executable: PathBuf,
}

pub(crate) fn parse_update_metadata(
    raw: &Value,
    updater_public_key: &str,
) -> Result<(ReleaseManifest, String, String), DesktopUpgradeError> {
    let manifest = serde_json::from_value::<ReleaseManifest>(
        raw.get("muriarc_release_manifest")
            .cloned()
            .ok_or(DesktopUpgradeError::TargetInvalid)?,
    )?;
    manifest
        .validate()
        .map_err(|_| DesktopUpgradeError::TargetInvalid)?;
    let manifest_signature = raw
        .get("muriarc_release_manifest_signature")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or(DesktopUpgradeError::TargetInvalid)?;
    verify_manifest_signature(&manifest, manifest_signature, updater_public_key)?;
    let artifact_name = raw
        .get("muriarc_artifact_name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or(DesktopUpgradeError::TargetInvalid)?
        .to_owned();
    if !manifest.artifacts.contains_key(&artifact_name) {
        return Err(DesktopUpgradeError::TargetInvalid);
    }
    Ok((manifest, artifact_name, manifest_signature.to_owned()))
}

fn verify_manifest_signature(
    manifest: &ReleaseManifest,
    wrapped_signature: &str,
    wrapped_public_key: &str,
) -> Result<(), DesktopUpgradeError> {
    let canonical_manifest =
        serde_json::to_vec(manifest).map_err(|_| DesktopUpgradeError::TargetInvalid)?;
    verify_minisign_payload(&canonical_manifest, wrapped_signature, wrapped_public_key)
}

fn verify_minisign_payload(
    payload: &[u8],
    wrapped_signature: &str,
    wrapped_public_key: &str,
) -> Result<(), DesktopUpgradeError> {
    let public_key_text = STANDARD
        .decode(wrapped_public_key.as_bytes())
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .ok_or(DesktopUpgradeError::TargetInvalid)?;
    let signature_text = STANDARD
        .decode(wrapped_signature.as_bytes())
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .ok_or(DesktopUpgradeError::TargetInvalid)?;
    let public_key =
        PublicKey::decode(&public_key_text).map_err(|_| DesktopUpgradeError::TargetInvalid)?;
    let signature =
        Signature::decode(&signature_text).map_err(|_| DesktopUpgradeError::TargetInvalid)?;
    public_key
        .verify(payload, &signature, true)
        .map_err(|_| DesktopUpgradeError::TargetInvalid)
}

pub(crate) async fn schedule_verified_update(
    config_root: &Path,
    source_executable: &Path,
    update: VerifiedDesktopUpdate,
) -> Result<Uuid, DesktopUpgradeError> {
    let config_root = canonical_real_directory(config_root)?;
    let intent_path = config_root.join(INTENT_FILE);
    recover_atomic_file(&intent_path)?;
    if intent_path.exists() {
        return Err(DesktopUpgradeError::OperationBusy);
    }
    validate_verified_update(&update)?;
    let source_root = resolve_active_root_for_upgrade(&config_root)?;
    let database_path = source_root.join(DATABASE_FILE);
    require_real_file(&database_path)?;
    let store = SqliteStore::connect_path(&database_path).await?;
    let source_state = store
        .compatibility_report()
        .await?
        .require_compatible()
        .cloned()
        .map_err(|_| DesktopUpgradeError::RecoveryInvalid)?;
    verify_or_create_manifest(&source_root, &source_state, false)
        .map_err(|_| DesktopUpgradeError::RecoveryInvalid)?;

    let operation_id = Uuid::new_v4();
    let source_executable_size_bytes = require_real_file_size(source_executable)?;
    let source_executable_sha256 = file_sha256(source_executable)?;
    let intent = DesktopUpgradeIntent {
        version: INTENT_VERSION,
        operation_id,
        source_data_root: source_root,
        source_generation_id: source_state.generation_id,
        source_application_version: source_state.identity.application_version.to_string(),
        candidate_generation_id: Uuid::new_v4(),
        updater_target: update.updater_target,
        artifact_name: update.artifact_name,
        artifact_sha256: update.artifact_sha256,
        artifact_size_bytes: update.artifact_size_bytes,
        source_executable_sha256: source_executable_sha256.clone(),
        source_executable_size_bytes,
        release_manifest: update.release_manifest,
        release_manifest_signature: update.release_manifest_signature,
        requested_at: Utc::now(),
    };
    validate_intent(&intent)?;
    let paths = derive_paths(&config_root, &intent)?;
    stage_binary_recovery(source_executable, &paths, &intent)?;
    let fallback = DesktopBinaryFallback {
        version: 1,
        operation_id,
        state: BinaryFallbackState::Armed,
        source_application_version: intent.source_application_version.clone(),
        failed_application_version: intent.release_manifest.application_version.to_string(),
        source_generation_id: intent.source_generation_id,
        intent_digest: digest_json(&intent)?,
        executable_sha256: source_executable_sha256,
        executable_size_bytes: source_executable_size_bytes,
        created_at: Utc::now(),
        activated_at: None,
    };
    replace_or_arm_binary_fallback(&config_root, &fallback)?;
    if let Err(error) = write_json_atomic(&intent_path, &intent) {
        let _ = remove_atomic_file(&config_root.join(BINARY_FALLBACK_FILE));
        return Err(error.into());
    }
    Ok(intent.operation_id)
}

pub(crate) fn cancel_scheduled_update(
    config_root: &Path,
    operation_id: Uuid,
) -> Result<(), DesktopUpgradeError> {
    let config_root = canonical_real_directory(config_root)?;
    let intent_path = config_root.join(INTENT_FILE);
    recover_atomic_file(&intent_path)?;
    let Some(intent) = read_json_optional::<DesktopUpgradeIntent>(&intent_path)? else {
        return Ok(());
    };
    if intent.operation_id != operation_id {
        return Err(DesktopUpgradeError::OperationBusy);
    }
    remove_atomic_file(&intent_path)?;
    remove_binary_fallback_if_armed(&config_root, operation_id)?;
    Ok(())
}

/// Runs before any database or storage service is initialized. After a failed
/// target startup, the installed executable only launches the verified old
/// executable and exits; it never opens user data with an incompatible schema.
pub(crate) fn delegate_to_binary_fallback(config_root: &Path) -> Result<bool, DesktopUpgradeError> {
    fs::create_dir_all(config_root)?;
    let config_root = canonical_real_directory(config_root)?;
    let fallback_path = config_root.join(BINARY_FALLBACK_FILE);
    recover_atomic_file(&fallback_path)?;
    let Some(fallback) = read_json_optional::<DesktopBinaryFallback>(&fallback_path)? else {
        return Ok(false);
    };
    validate_binary_fallback(&config_root, &fallback)?;
    let current_version = env!("CARGO_PKG_VERSION");
    if current_version == fallback.source_application_version {
        return Ok(false);
    }
    if current_version != fallback.failed_application_version {
        return Err(DesktopUpgradeError::BinaryVersionMismatch);
    }
    if fallback.state != BinaryFallbackState::Fallback {
        return Ok(false);
    }
    Command::new(binary_recovery_executable(
        &config_root,
        fallback.operation_id,
    ))
    .spawn()
    .map_err(|_| DesktopUpgradeError::BinaryRecoveryInvalid)?;
    Ok(true)
}

/// Arms the old executable fallback only when the Candidate has not received
/// its first write. If the locator was already switched, restore it first.
pub(crate) async fn activate_binary_fallback_after_failure(
    config_root: &Path,
) -> Result<PathBuf, DesktopUpgradeError> {
    let config_root = canonical_real_directory(config_root)?;
    let fallback_path = config_root.join(BINARY_FALLBACK_FILE);
    recover_atomic_file(&fallback_path)?;
    let mut fallback = read_json_optional::<DesktopBinaryFallback>(&fallback_path)?
        .ok_or(DesktopUpgradeError::BinaryRecoveryInvalid)?;
    validate_binary_fallback(&config_root, &fallback)?;
    if env!("CARGO_PKG_VERSION") != fallback.failed_application_version {
        return Err(DesktopUpgradeError::BinaryVersionMismatch);
    }

    let intent_path = config_root.join(INTENT_FILE);
    recover_atomic_file(&intent_path)?;
    if let Some(intent) = read_json_optional::<DesktopUpgradeIntent>(&intent_path)? {
        validate_intent(&intent)?;
        validate_intent_binding(&config_root, &intent)?;
        if intent.operation_id != fallback.operation_id
            || intent.source_generation_id != fallback.source_generation_id
        {
            return Err(DesktopUpgradeError::JournalIntegrity);
        }
        let paths = derive_paths(&config_root, &intent)?;
        let active = resolve_active_root_for_upgrade(&config_root)?;
        let source = canonical_real_directory(&intent.source_data_root)?;
        if paths.candidate_root.exists()
            && active == canonical_real_directory(&paths.candidate_root)?
        {
            let candidate =
                SqliteStore::connect_path(paths.candidate_root.join(DATABASE_FILE)).await?;
            ensure_no_first_write(&candidate, intent.candidate_generation_id).await?;
            activate_root_for_upgrade(&config_root, &source)?;
        } else if active != source {
            return Err(DesktopUpgradeError::FirstWriteBlocksRollback);
        }
    }

    fallback.state = BinaryFallbackState::Fallback;
    fallback.activated_at = Some(Utc::now());
    write_json_atomic(&fallback_path, &fallback)?;
    let executable = binary_recovery_executable(&config_root, fallback.operation_id);
    validate_recovery_executable(&executable, &fallback)?;
    Ok(executable)
}

pub(crate) async fn resume_pending_upgrade(config_root: &Path) -> Result<(), DesktopUpgradeError> {
    resume_pending_upgrade_inner(config_root, false).await
}

async fn resume_pending_upgrade_inner(
    config_root: &Path,
    allow_same_version_for_test: bool,
) -> Result<(), DesktopUpgradeError> {
    fs::create_dir_all(config_root)?;
    let config_root = canonical_real_directory(config_root)?;
    let intent_path = config_root.join(INTENT_FILE);
    recover_atomic_file(&intent_path)?;
    let Some(intent) = read_json_optional::<DesktopUpgradeIntent>(&intent_path)? else {
        return Ok(());
    };
    validate_intent(&intent)?;
    validate_intent_binding(&config_root, &intent)?;

    let current_version = env!("CARGO_PKG_VERSION");
    let target_version = intent.release_manifest.application_version.as_str();
    if !allow_same_version_for_test && current_version == intent.source_application_version {
        // The installer was cancelled or failed before replacing the binary.
        // A persistent fallback marker means the target did start and sent us
        // back here; retain that marker so future shortcut launches keep using
        // this old executable. Otherwise the installer never replaced us and
        // the armed recovery copy can be disarmed.
        remove_atomic_file(&intent_path)?;
        remove_binary_fallback_if_armed(&config_root, intent.operation_id)?;
        return Ok(());
    }
    if current_version != target_version {
        return Err(DesktopUpgradeError::BinaryVersionMismatch);
    }
    if !allow_same_version_for_test {
        let updater_public_key = option_env!("MURIARC_DESKTOP_UPDATER_PUBLIC_KEY")
            .ok_or(DesktopUpgradeError::TargetInvalid)?;
        verify_manifest_signature(
            &intent.release_manifest,
            &intent.release_manifest_signature,
            updater_public_key,
        )?;
    }

    let paths = derive_paths(&config_root, &intent)?;
    let target = VerifiedRelease::from_verified_platform_artifact(
        intent.release_manifest.clone(),
        intent.artifact_name.clone(),
        intent.artifact_size_bytes,
        intent.artifact_sha256.clone(),
        intent.requested_at + DESKTOP_TARGET_VALIDITY,
    )?;
    let driver = Arc::new(DesktopUpgradeDriver::new(
        config_root.clone(),
        intent.clone(),
        paths.clone(),
    ));
    let engine = UpgradeEngine::new(driver.clone(), &config_root);
    let snapshot = match driver.load_operation(intent.operation_id).await {
        Ok(snapshot) => {
            validate_active_pointer(&config_root, &intent, &paths, snapshot.phase)?;
            engine.resume(intent.operation_id, target).await?
        }
        Err(UpgradeError::OperationNotFound { .. }) => {
            validate_active_pointer(&config_root, &intent, &paths, UpgradePhase::Initialized)?;
            engine
                .run_with_operation_id(intent.operation_id, target)
                .await?
        }
        Err(error) => return Err(error.into()),
    };
    if snapshot.status != UpgradeStatus::Succeeded || snapshot.phase != UpgradePhase::Completed {
        return Err(DesktopUpgradeError::JournalIntegrity);
    }
    archive_completed(&config_root, intent, snapshot, &paths)?;
    remove_binary_fallback_for_success(&config_root)?;
    remove_atomic_file(&intent_path)?;
    Ok(())
}

struct DesktopBackendLock {
    file: File,
}

impl Drop for DesktopBackendLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

#[derive(Clone)]
struct DesktopUpgradeDriver {
    config_root: PathBuf,
    intent: DesktopUpgradeIntent,
    paths: DesktopUpgradePaths,
}

impl DesktopUpgradeDriver {
    fn new(config_root: PathBuf, intent: DesktopUpgradeIntent, paths: DesktopUpgradePaths) -> Self {
        Self {
            config_root,
            intent,
            paths,
        }
    }

    fn source_database(&self) -> PathBuf {
        self.intent.source_data_root.join(DATABASE_FILE)
    }

    fn candidate_database(&self) -> PathBuf {
        self.paths.candidate_root.join(DATABASE_FILE)
    }

    async fn source_store(&self) -> Result<SqliteStore, UpgradeError> {
        SqliteStore::connect_path(self.source_database())
            .await
            .map_err(|error| driver_error(UpgradePhase::Initialized, error))
    }

    async fn candidate_store(&self) -> Result<SqliteStore, UpgradeError> {
        SqliteStore::connect_path(self.candidate_database())
            .await
            .map_err(|error| driver_error(UpgradePhase::CandidatePrepared, error))
    }

    async fn persist_snapshot(
        &self,
        pool: &SqlitePool,
        snapshot: &UpgradeSnapshot,
        allow_insert: bool,
    ) -> Result<(), UpgradeError> {
        let current: Option<String> = sqlx::query_scalar(
            "SELECT journal_json FROM muriarc_upgrade_operations WHERE operation_id = ?",
        )
        .bind(snapshot.operation_id.to_string())
        .fetch_optional(pool)
        .await
        .map_err(persistence_error)?;
        if let Some(current) = current.as_ref() {
            let current_snapshot: UpgradeSnapshot =
                serde_json::from_str(current).map_err(serialization_error)?;
            if current_snapshot.revision > snapshot.revision {
                return Err(UpgradeError::Persistence {
                    message: "Desktop operation persistence rejected a stale revision".to_owned(),
                });
            }
            if current_snapshot.revision == snapshot.revision {
                return if current_snapshot == *snapshot {
                    Ok(())
                } else {
                    Err(UpgradeError::JournalIntegrity {
                        message: "Desktop operation revision conflicts with persisted state"
                            .to_owned(),
                    })
                };
            }
        } else if !allow_insert {
            return Err(UpgradeError::OperationNotFound {
                operation_id: snapshot.operation_id,
            });
        }

        let candidate_exists: bool = match snapshot.candidate_generation_id {
            Some(generation_id) => sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM muriarc_generation_sets WHERE generation_id = ?)",
            )
            .bind(generation_id.to_string())
            .fetch_one(pool)
            .await
            .map_err(persistence_error)?,
            None => false,
        };
        let candidate_generation_id = snapshot
            .candidate_generation_id
            .filter(|_| candidate_exists)
            .map(|value| value.to_string());
        let journal = serde_json::to_string(snapshot).map_err(serialization_error)?;
        if current.is_some() {
            sqlx::query(
                "UPDATE muriarc_upgrade_operations
                    SET candidate_generation_id = ?, phase = ?, status = ?, journal_json = ?,
                        updated_at = ?, completed_at = ?
                  WHERE operation_id = ?",
            )
            .bind(candidate_generation_id)
            .bind(upgrade_phase_name(snapshot.phase))
            .bind(upgrade_status_name(snapshot.status))
            .bind(journal)
            .bind(snapshot.updated_at)
            .bind(snapshot.completed_at)
            .bind(snapshot.operation_id.to_string())
            .execute(pool)
            .await
            .map_err(persistence_error)?;
        } else {
            sqlx::query(
                "INSERT INTO muriarc_upgrade_operations (
                    operation_id, source_generation_id, candidate_generation_id,
                    target_application_version, target_data_epoch, target_backend_state_digest,
                    target_gateway_contract_revision, maintenance_class, phase, status,
                    journal_version, journal_json, started_at, updated_at, completed_at
                 ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(snapshot.operation_id.to_string())
            .bind(snapshot.source_generation_id.to_string())
            .bind(candidate_generation_id)
            .bind(&snapshot.target_application_version)
            .bind(&snapshot.target_data_epoch)
            .bind(&snapshot.target_backend_state_digest)
            .bind(&snapshot.target_gateway_contract_revision)
            .bind(migration_class_name(snapshot.maintenance_class))
            .bind(upgrade_phase_name(snapshot.phase))
            .bind(upgrade_status_name(snapshot.status))
            .bind(i64::from(snapshot.journal_version))
            .bind(journal)
            .bind(snapshot.started_at)
            .bind(snapshot.updated_at)
            .bind(snapshot.completed_at)
            .execute(pool)
            .await
            .map_err(persistence_error)?;
        }
        Ok(())
    }

    async fn load_operation_from_source(
        &self,
        operation_id: Uuid,
    ) -> Result<UpgradeSnapshot, UpgradeError> {
        let source = self.source_store().await?;
        let journal: Option<String> = sqlx::query_scalar(
            "SELECT journal_json FROM muriarc_upgrade_operations WHERE operation_id = ?",
        )
        .bind(operation_id.to_string())
        .fetch_optional(source.pool())
        .await
        .map_err(persistence_error)?;
        serde_json::from_str(
            journal
                .as_deref()
                .ok_or(UpgradeError::OperationNotFound { operation_id })?,
        )
        .map_err(serialization_error)
    }

    async fn raw_generation(&self, database: &Path) -> Result<ActiveGeneration, UpgradeError> {
        require_real_file(database)
            .map_err(|error| driver_error(UpgradePhase::Initialized, error))?;
        let store = SqliteStore::connect_path(database)
            .await
            .map_err(|error| driver_error(UpgradePhase::Initialized, error))?;
        let row = sqlx::query(
            "SELECT application_version, data_epoch, backend_state_digest,
                    gateway_contract_revision, generation_id, first_write_at
               FROM muriarc_deployment_state WHERE singleton = 1",
        )
        .fetch_optional(store.pool())
        .await
        .map_err(persistence_error)?
        .ok_or_else(|| UpgradeError::Prerequisite {
            message: "Desktop deployment state is missing".to_owned(),
        })?;
        let identity = ReleaseIdentity::parse(
            row.try_get("application_version")
                .map_err(persistence_error)?,
            row.try_get("data_epoch").map_err(persistence_error)?,
            row.try_get("backend_state_digest")
                .map_err(persistence_error)?,
            row.try_get("gateway_contract_revision")
                .map_err(persistence_error)?,
        )
        .map_err(|message| UpgradeError::Prerequisite { message })?;
        let generation_id = Uuid::parse_str(
            &row.try_get::<String, _>("generation_id")
                .map_err(persistence_error)?,
        )
        .map_err(|error| UpgradeError::Persistence {
            message: error.to_string(),
        })?;
        Ok(ActiveGeneration {
            generation_id,
            identity,
            backend: BackendKind::Sqlite,
            first_write_at: row.try_get("first_write_at").map_err(persistence_error)?,
        })
    }

    async fn restore_source_write_lease(&self) -> Result<(), UpgradeError> {
        let source = self.source_store().await?;
        let state_generation: String = sqlx::query_scalar(
            "SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1",
        )
        .fetch_one(source.pool())
        .await
        .map_err(persistence_error)?;
        if state_generation != self.intent.source_generation_id.to_string() {
            return Err(UpgradeError::Prerequisite {
                message: "source database no longer belongs to the frozen generation".to_owned(),
            });
        }
        let active: Option<String> = sqlx::query_scalar(
            "SELECT lease_id FROM muriarc_write_leases
              WHERE generation_id = ? AND status = 'active'
                AND julianday(expires_at) > julianday('now')",
        )
        .bind(self.intent.source_generation_id.to_string())
        .fetch_optional(source.pool())
        .await
        .map_err(persistence_error)?;
        if let Some(lease_id) = active {
            sqlx::query(
                "UPDATE muriarc_deployment_state SET write_lease_id = ?, updated_at = ?
                  WHERE singleton = 1 AND generation_id = ?",
            )
            .bind(lease_id)
            .bind(Utc::now())
            .bind(self.intent.source_generation_id.to_string())
            .execute(source.pool())
            .await
            .map_err(persistence_error)?;
            return Ok(());
        }
        let now = Utc::now();
        let expires_at = now + Duration::days(3650);
        let fencing_token: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(fencing_token), 0) + 1 FROM muriarc_write_leases",
        )
        .fetch_one(source.pool())
        .await
        .map_err(persistence_error)?;
        let lease_id = Uuid::new_v4();
        let mut transaction = source.pool().begin().await.map_err(persistence_error)?;
        sqlx::query(
            "INSERT INTO muriarc_write_leases (
                 lease_id, generation_id, holder, fencing_token, status, issued_at, expires_at
             ) VALUES (?, ?, ?, ?, 'active', ?, ?)",
        )
        .bind(lease_id.to_string())
        .bind(self.intent.source_generation_id.to_string())
        .bind("desktop-rollback")
        .bind(fencing_token)
        .bind(now)
        .bind(expires_at)
        .execute(&mut *transaction)
        .await
        .map_err(persistence_error)?;
        let updated = sqlx::query(
            "UPDATE muriarc_deployment_state SET write_lease_id = ?, updated_at = ?
              WHERE singleton = 1 AND generation_id = ? AND write_lease_id IS NULL",
        )
        .bind(lease_id.to_string())
        .bind(now)
        .bind(self.intent.source_generation_id.to_string())
        .execute(&mut *transaction)
        .await
        .map_err(persistence_error)?;
        if updated.rows_affected() != 1 {
            return Err(UpgradeError::Prerequisite {
                message: "source database refused the rollback Write Lease".to_owned(),
            });
        }
        transaction.commit().await.map_err(persistence_error)
    }
}

#[async_trait]
impl UpgradeDriver for DesktopUpgradeDriver {
    fn profile(&self) -> DeploymentProfile {
        DeploymentProfile::Desktop
    }

    async fn acquire_backend_lock(
        &self,
        operation_id: Uuid,
    ) -> Result<Box<dyn BackendUpgradeLock>, UpgradeError> {
        let path = self
            .intent
            .source_data_root
            .join(".muriarc-desktop-backend.lock");
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true);
        let mut file = options.open(path).map_err(persistence_io_error)?;
        file.try_lock_exclusive().map_err(|error| {
            if error.kind() == std::io::ErrorKind::WouldBlock {
                UpgradeError::LockBusy
            } else {
                persistence_io_error(error)
            }
        })?;
        file.set_len(0).map_err(persistence_io_error)?;
        file.write_all(operation_id.to_string().as_bytes())
            .map_err(persistence_io_error)?;
        file.sync_all().map_err(persistence_io_error)?;
        Ok(Box::new(DesktopBackendLock { file }))
    }

    async fn current_generation(&self) -> Result<ActiveGeneration, UpgradeError> {
        let generation = self.raw_generation(&self.source_database()).await?;
        if generation.generation_id != self.intent.source_generation_id
            || generation.identity.application_version.as_str()
                != self.intent.source_application_version
        {
            return Err(UpgradeError::Prerequisite {
                message: "Desktop source generation differs from the signed installer intent"
                    .to_owned(),
            });
        }
        Ok(generation)
    }

    async fn create_operation(&self, snapshot: &UpgradeSnapshot) -> Result<(), UpgradeError> {
        if snapshot.phase != UpgradePhase::LocksAcquired
            || snapshot.status != UpgradeStatus::Running
            || snapshot.operation_id != self.intent.operation_id
        {
            return Err(UpgradeError::InvalidTransition {
                from: snapshot.phase,
                to: UpgradePhase::LocksAcquired,
            });
        }
        let source = self.source_store().await?;
        self.persist_snapshot(source.pool(), snapshot, true).await
    }

    async fn save_operation(&self, snapshot: &UpgradeSnapshot) -> Result<(), UpgradeError> {
        let source = self.source_store().await?;
        self.persist_snapshot(source.pool(), snapshot, false)
            .await?;
        if self.candidate_database().is_file() {
            let candidate = self.candidate_store().await?;
            self.persist_snapshot(candidate.pool(), snapshot, true)
                .await?;
        }
        Ok(())
    }

    async fn load_operation(&self, operation_id: Uuid) -> Result<UpgradeSnapshot, UpgradeError> {
        self.load_operation_from_source(operation_id).await
    }

    async fn preflight(
        &self,
        snapshot: &UpgradeSnapshot,
        target: &VerifiedRelease,
    ) -> Result<PreflightEvidence, UpgradeError> {
        let fallback = binary_recovery_executable(&self.config_root, self.intent.operation_id);
        let space = upgrade_space_preflight(
            &self.config_root,
            &fallback,
            self.intent.artifact_size_bytes,
        )
        .map_err(|error| driver_error(UpgradePhase::PreflightPassed, error))?;
        Ok(PreflightEvidence {
            source_generation_id: snapshot.source_generation_id,
            target_application_version: target.manifest.application_version.to_string(),
            free_bytes: space.data_free_bytes,
            required_bytes: space.data_required_bytes,
            maintenance_class: target.manifest.migration_class,
            recovery_prerequisites_satisfied: space.sufficient,
            checked_at: Utc::now(),
        })
    }

    async fn drain(&self, snapshot: &UpgradeSnapshot) -> Result<DrainEvidence, UpgradeError> {
        let source = self.source_store().await?;
        sqlx::query(
            "UPDATE muriarc_write_leases SET status = 'draining'
              WHERE lease_id = (
                    SELECT write_lease_id FROM muriarc_deployment_state
                     WHERE singleton = 1 AND generation_id = ?
              ) AND status = 'active'",
        )
        .bind(snapshot.source_generation_id.to_string())
        .execute(source.pool())
        .await
        .map_err(persistence_error)?;
        let status: Option<String> = sqlx::query_scalar(
            "SELECT lease.status FROM muriarc_deployment_state AS state
               JOIN muriarc_write_leases AS lease ON lease.lease_id = state.write_lease_id
              WHERE state.singleton = 1 AND state.generation_id = ?",
        )
        .bind(snapshot.source_generation_id.to_string())
        .fetch_optional(source.pool())
        .await
        .map_err(persistence_error)?;
        if status.as_deref() != Some("draining") {
            return Err(UpgradeError::Prerequisite {
                message: "Desktop source Write Lease could not enter draining state".to_owned(),
            });
        }
        // This driver runs before Tauri constructs Application, AI jobs, or
        // attachment services, so zero is an enforced startup invariant rather
        // than a best-effort observation of a live process.
        Ok(DrainEvidence {
            inflight_requests: 0,
            running_jobs: 0,
            pending_attachment_writes: 0,
            provider_requests: 0,
            drained_at: Utc::now(),
        })
    }

    async fn freeze_writes(
        &self,
        snapshot: &UpgradeSnapshot,
    ) -> Result<FreezeEvidence, UpgradeError> {
        let source = self.source_store().await?;
        let existing = sqlx::query(
            "SELECT lease.lease_id, lease.fencing_token, lease.status, lease.revoked_at
               FROM muriarc_write_leases AS lease
              WHERE lease.generation_id = ?
              ORDER BY lease.fencing_token DESC LIMIT 1",
        )
        .bind(snapshot.source_generation_id.to_string())
        .fetch_optional(source.pool())
        .await
        .map_err(persistence_error)?
        .ok_or_else(|| UpgradeError::Prerequisite {
            message: "Desktop source Write Lease is missing".to_owned(),
        })?;
        let lease_id = Uuid::parse_str(
            &existing
                .try_get::<String, _>("lease_id")
                .map_err(persistence_error)?,
        )
        .map_err(|error| UpgradeError::Persistence {
            message: error.to_string(),
        })?;
        let fencing_token: i64 = existing
            .try_get("fencing_token")
            .map_err(persistence_error)?;
        let status: String = existing.try_get("status").map_err(persistence_error)?;
        let frozen_at = if status == "revoked" {
            existing
                .try_get::<Option<DateTime<Utc>>, _>("revoked_at")
                .map_err(persistence_error)?
                .ok_or_else(|| UpgradeError::Persistence {
                    message: "revoked Desktop lease has no timestamp".to_owned(),
                })?
        } else {
            if status != "draining" {
                return Err(UpgradeError::Prerequisite {
                    message: "Desktop source Write Lease was not drained".to_owned(),
                });
            }
            let now = Utc::now();
            let mut transaction = source.pool().begin().await.map_err(persistence_error)?;
            sqlx::query(
                "UPDATE muriarc_write_leases SET status = 'revoked', revoked_at = ?
                  WHERE lease_id = ? AND status = 'draining'",
            )
            .bind(now)
            .bind(lease_id.to_string())
            .execute(&mut *transaction)
            .await
            .map_err(persistence_error)?;
            sqlx::query(
                "UPDATE muriarc_deployment_state SET write_lease_id = NULL, updated_at = ?
                  WHERE singleton = 1 AND generation_id = ?",
            )
            .bind(now)
            .bind(snapshot.source_generation_id.to_string())
            .execute(&mut *transaction)
            .await
            .map_err(persistence_error)?;
            transaction.commit().await.map_err(persistence_error)?;
            now
        };
        Ok(FreezeEvidence {
            source_generation_id: snapshot.source_generation_id,
            revoked_lease_id: lease_id,
            fencing_token,
            frozen_at,
        })
    }

    async fn create_backup(
        &self,
        snapshot: &UpgradeSnapshot,
    ) -> Result<BackupEvidence, UpgradeError> {
        checkpoint_and_verify_database(&self.source_database())
            .await
            .map_err(|error| driver_error(UpgradePhase::BackupCreated, error))?;
        let source_tree = tree_digest(&self.intent.source_data_root)
            .map_err(|error| driver_error(UpgradePhase::BackupCreated, error))?;
        prepare_verified_copy(
            &self.intent.source_data_root,
            &self.paths.recovery_root,
            &self.paths.operation_root,
            &source_tree,
            true,
        )
        .await
        .map_err(|error| driver_error(UpgradePhase::BackupCreated, error))?;
        let source = self.source_store().await?;
        let inventory = source
            .persistent_recovery_inventory()
            .await
            .map_err(|error| driver_error(UpgradePhase::BackupCreated, error))?;
        Ok(BackupEvidence {
            backup_id: snapshot.operation_id,
            source_generation_id: snapshot.source_generation_id,
            artifact_digest: format!("sha256:{}", source_tree.sha256),
            recovery_set_digest: digest_json(&(source_tree, inventory))
                .map_err(|error| driver_error(UpgradePhase::BackupCreated, error))?,
            components: RecoveryComponent::required(),
            created_at: Utc::now(),
        })
    }

    async fn verify_backup_restore(
        &self,
        snapshot: &UpgradeSnapshot,
        backup: &BackupEvidence,
    ) -> Result<RestoreEvidence, UpgradeError> {
        let recovery_tree = tree_digest(&self.paths.recovery_root)
            .map_err(|error| driver_error(UpgradePhase::BackupRestored, error))?;
        if format!("sha256:{}", recovery_tree.sha256) != backup.artifact_digest {
            return Err(UpgradeError::ArtifactVerification {
                message: "Desktop recovery tree differs from the frozen source".to_owned(),
            });
        }
        verify_database_read_only(&self.paths.recovery_root.join(DATABASE_FILE))
            .await
            .map_err(|error| driver_error(UpgradePhase::BackupRestored, error))?;
        let restored = SqliteStore::connect_path(self.paths.recovery_root.join(DATABASE_FILE))
            .await
            .map_err(|error| driver_error(UpgradePhase::BackupRestored, error))?;
        let inventory = restored
            .persistent_recovery_inventory()
            .await
            .map_err(|error| driver_error(UpgradePhase::BackupRestored, error))?;
        if digest_json(&(recovery_tree, inventory))
            .map_err(|error| driver_error(UpgradePhase::BackupRestored, error))?
            != backup.recovery_set_digest
        {
            return Err(UpgradeError::ArtifactVerification {
                message: "Desktop restored recovery inventory is incomplete".to_owned(),
            });
        }
        Ok(RestoreEvidence {
            backup_id: backup.backup_id,
            backup_artifact_digest: backup.artifact_digest.clone(),
            restored_generation_id: snapshot.source_generation_id,
            isolated_restore: true,
            verified_at: Utc::now(),
        })
    }

    async fn prepare_candidate(
        &self,
        snapshot: &UpgradeSnapshot,
        _restore: &RestoreEvidence,
        _target: &VerifiedRelease,
    ) -> Result<CandidateEvidence, UpgradeError> {
        let recovery_tree = tree_digest(&self.paths.recovery_root)
            .map_err(|error| driver_error(UpgradePhase::CandidatePrepared, error))?;
        prepare_verified_copy(
            &self.paths.recovery_root,
            &self.paths.candidate_root,
            &self.paths.operation_root,
            &recovery_tree,
            true,
        )
        .await
        .map_err(|error| driver_error(UpgradePhase::CandidatePrepared, error))?;
        Ok(CandidateEvidence {
            generation_id: self.intent.candidate_generation_id,
            isolated: true,
            private_endpoint: true,
            external_providers_disabled: true,
            background_jobs_disabled: true,
            real_user_writes_disabled: true,
            prepared_at: snapshot.updated_at.max(Utc::now()),
        })
    }

    async fn migrate_candidate(
        &self,
        snapshot: &UpgradeSnapshot,
        candidate: &CandidateEvidence,
        _target: &VerifiedRelease,
    ) -> Result<MigrationEvidence, UpgradeError> {
        verify_database_read_only(&self.candidate_database())
            .await
            .map_err(|error| driver_error(UpgradePhase::CandidateMigrated, error))?;
        let store = self.candidate_store().await?;
        store
            .apply_upgrade_migrations()
            .await
            .map_err(|error| driver_error(UpgradePhase::CandidateMigrated, error))?;
        let state = store
            .prepare_upgraded_candidate(snapshot.source_generation_id, candidate.generation_id)
            .await
            .map_err(|error| driver_error(UpgradePhase::CandidateMigrated, error))?;
        ensure_target_identity(&state.identity, &self.intent.release_manifest)
            .map_err(|error| driver_error(UpgradePhase::CandidateMigrated, error))?;
        write_manifest_atomic(
            &self.paths.candidate_root.join(GENERATION_MANIFEST_FILE),
            &DeploymentGenerationManifest::from_state(&state),
        )
        .map_err(|error| driver_error(UpgradePhase::CandidateMigrated, error))?;
        Ok(MigrationEvidence {
            generation_id: candidate.generation_id,
            identity: state.identity,
            migration_path: vec![format!(
                "{}:{}->{}:{}",
                snapshot.source_identity.application_version,
                snapshot.source_identity.data_epoch,
                snapshot.target_application_version,
                snapshot.target_data_epoch
            )],
            completed_at: Utc::now(),
        })
    }

    async fn verify_candidate(
        &self,
        snapshot: &UpgradeSnapshot,
        candidate: &CandidateEvidence,
    ) -> Result<VerificationEvidence, UpgradeError> {
        let source = self.source_store().await?;
        let source_inventory = source
            .persistent_recovery_inventory()
            .await
            .map_err(|error| driver_error(UpgradePhase::CandidateVerified, error))?;
        let (candidate_inventory, candidate_payload, verification_digest) =
            verify_candidate(&self.intent, &self.paths, &source_inventory)
                .await
                .map_err(|error| driver_error(UpgradePhase::CandidateVerified, error))?;
        let shared = digest_json(&(
            snapshot.operation_id,
            candidate.generation_id,
            candidate_inventory,
            candidate_payload,
            verification_digest,
        ))
        .map_err(|error| driver_error(UpgradePhase::CandidateVerified, error))?;
        let layers = VerificationLayer::required()
            .into_iter()
            .map(|layer| {
                let evidence_digest = digest_json(&(layer, &shared))
                    .map_err(|error| driver_error(UpgradePhase::CandidateVerified, error))?;
                Ok((
                    layer,
                    VerificationLayerEvidence {
                        evidence_digest,
                        verified_at: Utc::now(),
                    },
                ))
            })
            .collect::<Result<BTreeMap<_, _>, UpgradeError>>()?;
        Ok(VerificationEvidence {
            generation_id: candidate.generation_id,
            layers,
        })
    }

    async fn switch_generation(
        &self,
        snapshot: &UpgradeSnapshot,
        candidate: &CandidateEvidence,
    ) -> Result<SwitchEvidence, UpgradeError> {
        let store = self.candidate_store().await?;
        ensure_no_first_write(&store, candidate.generation_id)
            .await
            .map_err(|error| driver_error(UpgradePhase::Switched, error))?;
        activate_root_for_upgrade(&self.config_root, &self.paths.candidate_root)
            .map_err(|error| driver_error(UpgradePhase::Switched, error))?;
        if resolve_active_root_for_upgrade(&self.config_root)
            .map_err(|error| driver_error(UpgradePhase::Switched, error))?
            != canonical_real_directory(&self.paths.candidate_root)
                .map_err(|error| driver_error(UpgradePhase::Switched, error))?
        {
            return Err(UpgradeError::Driver {
                phase: UpgradePhase::Switched,
                message: "Desktop active data locator did not switch atomically".to_owned(),
            });
        }
        Ok(SwitchEvidence {
            source_generation_id: snapshot.source_generation_id,
            candidate_generation_id: candidate.generation_id,
            atomic: true,
            switched_at: Utc::now(),
        })
    }

    async fn activate_read_only(
        &self,
        _snapshot: &UpgradeSnapshot,
        candidate: &CandidateEvidence,
    ) -> Result<ReadOnlyActivationEvidence, UpgradeError> {
        let store = self.candidate_store().await?;
        let state = store
            .compatibility_report()
            .await
            .map_err(|error| driver_error(UpgradePhase::ReadOnlyActivated, error))?
            .require_read_only_compatible()
            .cloned()
            .map_err(|message| UpgradeError::Driver {
                phase: UpgradePhase::ReadOnlyActivated,
                message,
            })?;
        if state.generation_id != candidate.generation_id
            || state.write_lease_id.is_some()
            || state.first_write_at.is_some()
        {
            return Err(UpgradeError::Driver {
                phase: UpgradePhase::ReadOnlyActivated,
                message: "Desktop Candidate is not at the read-only activation boundary".to_owned(),
            });
        }
        Ok(ReadOnlyActivationEvidence {
            generation_id: candidate.generation_id,
            write_lease_absent: true,
            external_traffic_blocked: true,
            activated_at: Utc::now(),
        })
    }

    async fn verify_activated(
        &self,
        _snapshot: &UpgradeSnapshot,
        candidate: &CandidateEvidence,
    ) -> Result<ActivationVerificationEvidence, UpgradeError> {
        let before = self.first_write_at(candidate.generation_id).await?;
        let store = self.candidate_store().await?;
        let state = store
            .compatibility_report()
            .await
            .map_err(|error| driver_error(UpgradePhase::ActivationVerified, error))?
            .require_read_only_compatible()
            .cloned()
            .map_err(|message| UpgradeError::Driver {
                phase: UpgradePhase::ActivationVerified,
                message,
            })?;
        verify_application_read_surface(store.pool())
            .await
            .map_err(|error| driver_error(UpgradePhase::ActivationVerified, error))?;
        let after = self.first_write_at(candidate.generation_id).await?;
        let compatibility_verified =
            ensure_target_identity(&state.identity, &self.intent.release_manifest).is_ok();
        Ok(ActivationVerificationEvidence {
            generation_id: candidate.generation_id,
            readiness_verified: state.generation_id == candidate.generation_id,
            compatibility_verified,
            no_write_side_effects: before.is_none() && after.is_none(),
            verified_at: Utc::now(),
        })
    }

    async fn open_write_lease(
        &self,
        _snapshot: &UpgradeSnapshot,
        candidate: &CandidateEvidence,
    ) -> Result<WriteLeaseEvidence, UpgradeError> {
        let store = self.candidate_store().await?;
        let state = store
            .open_candidate_write_lease(candidate.generation_id, WRITE_LEASE_HOLDER)
            .await
            .map_err(|error| driver_error(UpgradePhase::WriteLeaseOpened, error))?;
        let lease_id = state.write_lease_id.ok_or_else(|| UpgradeError::Driver {
            phase: UpgradePhase::WriteLeaseOpened,
            message: "Desktop Candidate did not receive a Write Lease".to_owned(),
        })?;
        let row = sqlx::query(
            "SELECT fencing_token, expires_at FROM muriarc_write_leases WHERE lease_id = ?",
        )
        .bind(lease_id.to_string())
        .fetch_one(store.pool())
        .await
        .map_err(persistence_error)?;
        Ok(WriteLeaseEvidence {
            generation_id: candidate.generation_id,
            lease_id,
            fencing_token: row.try_get("fencing_token").map_err(persistence_error)?,
            expires_at: row.try_get("expires_at").map_err(persistence_error)?,
        })
    }

    async fn first_write_at(
        &self,
        generation_id: Uuid,
    ) -> Result<Option<DateTime<Utc>>, UpgradeError> {
        let database = if generation_id == self.intent.candidate_generation_id
            && self.candidate_database().is_file()
        {
            self.candidate_database()
        } else {
            self.source_database()
        };
        let store = SqliteStore::connect_path(database)
            .await
            .map_err(|error| driver_error(UpgradePhase::Switched, error))?;
        sqlx::query_scalar(
            "SELECT first_write_at FROM muriarc_generation_sets WHERE generation_id = ?",
        )
        .bind(generation_id.to_string())
        .fetch_optional(store.pool())
        .await
        .map(Option::flatten)
        .map_err(persistence_error)
    }

    async fn recover_before_first_write(
        &self,
        snapshot: &UpgradeSnapshot,
    ) -> Result<(), UpgradeError> {
        if let Some(candidate_id) = snapshot.candidate_generation_id
            && let Some(first_write_at) = self.first_write_at(candidate_id).await?
        {
            return Err(UpgradeError::FirstWriteBlocksRollback { first_write_at });
        }
        let active = resolve_active_root_for_upgrade(&self.config_root)
            .map_err(|error| driver_error(snapshot.phase, error))?;
        let source = canonical_real_directory(&self.intent.source_data_root)
            .map_err(|error| driver_error(snapshot.phase, error))?;
        if active != source {
            let candidate = canonical_real_directory(&self.paths.candidate_root)
                .map_err(|error| driver_error(snapshot.phase, error))?;
            if active != candidate {
                return Err(UpgradeError::Prerequisite {
                    message: "Desktop active locator is neither source nor Candidate".to_owned(),
                });
            }
            if self.candidate_database().is_file() {
                let candidate_store = self.candidate_store().await?;
                sqlx::query(
                    "UPDATE muriarc_write_leases SET status = 'revoked', revoked_at = ?
                      WHERE generation_id = ? AND status IN ('active', 'draining')",
                )
                .bind(Utc::now())
                .bind(self.intent.candidate_generation_id.to_string())
                .execute(candidate_store.pool())
                .await
                .map_err(persistence_error)?;
            }
            activate_root_for_upgrade(&self.config_root, &source)
                .map_err(|error| driver_error(snapshot.phase, error))?;
        }
        self.restore_source_write_lease().await
    }
}

fn driver_error(phase: UpgradePhase, error: impl std::fmt::Display) -> UpgradeError {
    UpgradeError::Driver {
        phase,
        message: error.to_string(),
    }
}

fn persistence_error(error: sqlx::Error) -> UpgradeError {
    UpgradeError::Persistence {
        message: error.to_string(),
    }
}

fn persistence_io_error(error: std::io::Error) -> UpgradeError {
    UpgradeError::Persistence {
        message: error.to_string(),
    }
}

fn serialization_error(error: serde_json::Error) -> UpgradeError {
    UpgradeError::Persistence {
        message: error.to_string(),
    }
}

fn migration_class_name(class: muriarc_core::MigrationClass) -> &'static str {
    match class {
        muriarc_core::MigrationClass::M0 => "M0",
        muriarc_core::MigrationClass::M1 => "M1",
        muriarc_core::MigrationClass::M2 => "M2",
        muriarc_core::MigrationClass::M3 => "M3",
    }
}

fn upgrade_phase_name(phase: UpgradePhase) -> &'static str {
    match phase {
        UpgradePhase::Initialized => "initialized",
        UpgradePhase::LocksAcquired => "locks_acquired",
        UpgradePhase::PreflightPassed => "preflight_passed",
        UpgradePhase::Drained => "drained",
        UpgradePhase::WritesFrozen => "writes_frozen",
        UpgradePhase::BackupCreated => "backup_created",
        UpgradePhase::BackupRestored => "backup_restored",
        UpgradePhase::CandidatePrepared => "candidate_prepared",
        UpgradePhase::CandidateMigrated => "candidate_migrated",
        UpgradePhase::CandidateVerified => "candidate_verified",
        UpgradePhase::Switched => "switched",
        UpgradePhase::ReadOnlyActivated => "read_only_activated",
        UpgradePhase::ActivationVerified => "activation_verified",
        UpgradePhase::WriteLeaseOpened => "write_lease_opened",
        UpgradePhase::Completed => "completed",
    }
}

fn upgrade_status_name(status: UpgradeStatus) -> &'static str {
    match status {
        UpgradeStatus::Running => "running",
        UpgradeStatus::Succeeded => "succeeded",
        UpgradeStatus::Failed => "failed",
        UpgradeStatus::RecoveryRequired => "recovery_required",
    }
}

fn validate_verified_update(update: &VerifiedDesktopUpdate) -> Result<(), DesktopUpgradeError> {
    update
        .release_manifest
        .validate()
        .map_err(|_| DesktopUpgradeError::TargetInvalid)?;
    if update.version != update.release_manifest.application_version.as_str()
        || update.updater_target.trim().is_empty()
        || update.artifact_name.trim().is_empty()
        || update.release_manifest_signature.trim().is_empty()
        || !valid_sha256(&update.artifact_sha256)
        || update.artifact_size_bytes == 0
    {
        return Err(DesktopUpgradeError::TargetInvalid);
    }
    let artifact = update
        .release_manifest
        .artifacts
        .get(&update.artifact_name)
        .ok_or(DesktopUpgradeError::TargetInvalid)?;
    if artifact.digest.as_str() != update.artifact_sha256
        || artifact.size_bytes != update.artifact_size_bytes
    {
        return Err(DesktopUpgradeError::TargetInvalid);
    }
    Ok(())
}

fn validate_intent_binding(
    config_root: &Path,
    intent: &DesktopUpgradeIntent,
) -> Result<(), DesktopUpgradeError> {
    let fallback_path = config_root.join(BINARY_FALLBACK_FILE);
    recover_atomic_file(&fallback_path)?;
    let fallback = read_json_optional::<DesktopBinaryFallback>(&fallback_path)?
        .ok_or(DesktopUpgradeError::BinaryRecoveryInvalid)?;
    validate_binary_fallback(config_root, &fallback)?;
    if fallback.operation_id != intent.operation_id
        || fallback.source_generation_id != intent.source_generation_id
        || fallback.intent_digest != digest_json(intent)?
    {
        return Err(DesktopUpgradeError::JournalIntegrity);
    }
    Ok(())
}

fn validate_intent(intent: &DesktopUpgradeIntent) -> Result<(), DesktopUpgradeError> {
    if intent.version != INTENT_VERSION
        || intent.operation_id.is_nil()
        || intent.source_generation_id.is_nil()
        || intent.candidate_generation_id.is_nil()
        || intent.source_generation_id == intent.candidate_generation_id
        || intent.source_application_version.trim().is_empty()
        || !valid_sha256(&intent.source_executable_sha256)
        || intent.source_executable_size_bytes == 0
    {
        return Err(DesktopUpgradeError::TargetInvalid);
    }
    validate_verified_update(&VerifiedDesktopUpdate {
        version: intent.release_manifest.application_version.to_string(),
        updater_target: intent.updater_target.clone(),
        artifact_name: intent.artifact_name.clone(),
        artifact_sha256: intent.artifact_sha256.clone(),
        artifact_size_bytes: intent.artifact_size_bytes,
        release_manifest: intent.release_manifest.clone(),
        release_manifest_signature: intent.release_manifest_signature.clone(),
    })
}

fn ensure_target_identity(
    observed: &ReleaseIdentity,
    manifest: &ReleaseManifest,
) -> Result<(), DesktopUpgradeError> {
    let sqlite_digest = manifest
        .backend_states
        .get(&BackendKind::Sqlite)
        .ok_or(DesktopUpgradeError::TargetInvalid)?;
    if observed.application_version != manifest.application_version
        || observed.data_epoch != manifest.data_epoch
        || observed.gateway_contract_revision != manifest.gateway_contract_revision
        || observed.backend_state_digest != *sqlite_digest
    {
        return Err(DesktopUpgradeError::CandidateInvalid);
    }
    Ok(())
}

fn derive_paths(
    config_root: &Path,
    intent: &DesktopUpgradeIntent,
) -> Result<DesktopUpgradePaths, DesktopUpgradeError> {
    let source = canonical_real_directory(&intent.source_data_root)?;
    let parent = source
        .parent()
        .ok_or(DesktopUpgradeError::RecoveryInvalid)?;
    let source_name = source
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .ok_or(DesktopUpgradeError::RecoveryInvalid)?;
    let operation_root = parent.join(format!(
        ".{source_name}.muriarc-upgrade-{}",
        intent.operation_id
    ));
    let recovery_root = operation_root.join("recovery");
    let candidate_root = operation_root.join("candidate");
    if paths_overlap(&source, &operation_root)
        || paths_overlap(&source, &recovery_root)
        || paths_overlap(&source, &candidate_root)
    {
        return Err(DesktopUpgradeError::RecoveryInvalid);
    }
    let binary_recovery_root = config_root
        .join(BINARY_RECOVERY_DIRECTORY)
        .join(intent.operation_id.to_string());
    let fallback_executable = binary_recovery_root.join(FALLBACK_EXECUTABLE_FILE);
    Ok(DesktopUpgradePaths {
        operation_root,
        recovery_root,
        candidate_root,
        binary_recovery_root,
        fallback_executable,
    })
}

fn validate_active_pointer(
    config_root: &Path,
    intent: &DesktopUpgradeIntent,
    paths: &DesktopUpgradePaths,
    phase: UpgradePhase,
) -> Result<(), DesktopUpgradeError> {
    let active = resolve_active_root_for_upgrade(config_root)?;
    let source = canonical_real_directory(&intent.source_data_root)?;
    if active == source {
        return Ok(());
    }
    if phase.has_switched()
        && paths.candidate_root.exists()
        && active == canonical_real_directory(&paths.candidate_root)?
    {
        return Ok(());
    }
    Err(DesktopUpgradeError::RecoveryInvalid)
}

async fn prepare_verified_copy(
    source: &Path,
    target: &Path,
    operation_root: &Path,
    expected: &TreeDigest,
    replace_partial: bool,
) -> Result<(), DesktopUpgradeError> {
    if target.exists() {
        let reusable = canonical_real_directory(target).ok().is_some_and(|_| {
            tree_digest(target).is_ok_and(|digest| digest == *expected)
                && target.join(DATABASE_FILE).is_file()
        });
        if reusable {
            verify_database_read_only(&target.join(DATABASE_FILE)).await?;
            return Ok(());
        }
        if !replace_partial {
            return Err(DesktopUpgradeError::RecoveryInvalid);
        }
        remove_partial_directory(target, operation_root)?;
    }
    if !operation_root.exists() {
        fs::create_dir(operation_root)?;
    } else {
        require_real_directory(operation_root)?;
    }
    fs::create_dir(target)?;
    copy_managed_tree(source, target)?;
    if tree_digest(target)? != *expected {
        return Err(DesktopUpgradeError::RecoveryInvalid);
    }
    verify_database_read_only(&target.join(DATABASE_FILE)).await?;
    Ok(())
}

async fn verify_candidate(
    intent: &DesktopUpgradeIntent,
    paths: &DesktopUpgradePaths,
    source_inventory: &PersistentRecoveryInventory,
) -> Result<(PersistentRecoveryInventory, TreeDigest, String), DesktopUpgradeError> {
    let database_path = paths.candidate_root.join(DATABASE_FILE);
    verify_database_read_only(&database_path).await?;
    verify_database_semantics(&database_path).await?;
    let before_database = file_sha256(&database_path)?;
    let before_payload = payload_tree_digest(&paths.candidate_root)?;
    let source_payload = payload_tree_digest(&intent.source_data_root)?;
    if before_payload != source_payload {
        return Err(DesktopUpgradeError::CandidateInvalid);
    }

    let store = SqliteStore::connect_path(&database_path).await?;
    let state = store
        .compatibility_report()
        .await?
        .require_read_only_compatible()
        .cloned()
        .map_err(|_| DesktopUpgradeError::CandidateInvalid)?;
    if state.generation_id != intent.candidate_generation_id || state.first_write_at.is_some() {
        return Err(DesktopUpgradeError::CandidateInvalid);
    }
    ensure_target_identity(&state.identity, &intent.release_manifest)?;
    verify_or_create_manifest(&paths.candidate_root, &state, false)
        .map_err(|_| DesktopUpgradeError::CandidateInvalid)?;

    let candidate_inventory = store.persistent_recovery_inventory().await?;
    ensure_inventory_preserved(source_inventory, &candidate_inventory)?;
    verify_attachment_objects(store.pool(), &paths.candidate_root.join("attachments")).await?;
    verify_application_read_surface(store.pool()).await?;
    verify_continue_write_with_rollback(&store, intent.candidate_generation_id).await?;

    let after_database = file_sha256(&database_path)?;
    let after_payload = payload_tree_digest(&paths.candidate_root)?;
    if before_database != after_database || before_payload != after_payload {
        return Err(DesktopUpgradeError::CandidateInvalid);
    }
    let verification_digest = digest_json(&BTreeMap::from([
        ("database", before_database),
        ("payload", format!("sha256:{}", before_payload.sha256)),
        ("inventory", digest_json(&candidate_inventory)?),
        (
            "attachments",
            digest_attachment_metadata(store.pool()).await?,
        ),
        ("read_only", "verified".to_owned()),
        ("continue_write", "rollback_verified".to_owned()),
        ("application", "read_surface_verified".to_owned()),
    ]))?;
    Ok((candidate_inventory, after_payload, verification_digest))
}

fn ensure_inventory_preserved(
    source: &PersistentRecoveryInventory,
    candidate: &PersistentRecoveryInventory,
) -> Result<(), DesktopUpgradeError> {
    if candidate.attachment_records < source.attachment_records
        || candidate.encrypted_secret_records < source.encrypted_secret_records
        || candidate.secret_reference_records < source.secret_reference_records
        || candidate.ai_history_records < source.ai_history_records
        || candidate.audit_records < source.audit_records
    {
        return Err(DesktopUpgradeError::CandidateInvalid);
    }
    Ok(())
}

async fn verify_database_semantics(path: &Path) -> Result<(), DesktopUpgradeError> {
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(path)
        .read_only(true)
        .foreign_keys(true);
    let mut connection = sqlx::SqliteConnection::connect_with(&options).await?;
    let quick_check: String = sqlx::query_scalar("PRAGMA quick_check")
        .fetch_one(&mut connection)
        .await?;
    let foreign_key_violations: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM pragma_foreign_key_check")
            .fetch_one(&mut connection)
            .await?;
    connection.close().await?;
    if quick_check != "ok" || foreign_key_violations != 0 {
        return Err(DesktopUpgradeError::CandidateInvalid);
    }
    Ok(())
}

async fn verify_application_read_surface(
    pool: &sqlx::SqlitePool,
) -> Result<(), DesktopUpgradeError> {
    for table in [
        "labs",
        "users",
        "projects",
        "animals",
        "experiments",
        "observations",
        "attachments",
        "ai_conversations",
        "audit_entries",
    ] {
        let statement = format!("SELECT COUNT(*) FROM \"{table}\"");
        let count: i64 = sqlx::query_scalar(&statement).fetch_one(pool).await?;
        if count < 0 {
            return Err(DesktopUpgradeError::CandidateInvalid);
        }
    }
    let lab_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM labs")
        .fetch_one(pool)
        .await?;
    if lab_count == 0 {
        return Err(DesktopUpgradeError::CandidateInvalid);
    }
    Ok(())
}

async fn verify_continue_write_with_rollback(
    store: &SqliteStore,
    generation_id: Uuid,
) -> Result<(), DesktopUpgradeError> {
    let mut tx = store.pool().begin().await?;
    let now = Utc::now();
    let expires_at = now + Duration::minutes(10);
    let lease_id = Uuid::new_v4();
    let fencing_token: i64 =
        sqlx::query_scalar("SELECT COALESCE(MAX(fencing_token), 0) + 1 FROM muriarc_write_leases")
            .fetch_one(&mut *tx)
            .await?;
    sqlx::query(
        "INSERT INTO muriarc_write_leases (
             lease_id, generation_id, holder, fencing_token, status, issued_at, expires_at
         ) VALUES (?, ?, 'desktop-candidate-verifier', ?, 'active', ?, ?)",
    )
    .bind(lease_id.to_string())
    .bind(generation_id.to_string())
    .bind(fencing_token)
    .bind(now)
    .bind(expires_at)
    .execute(&mut *tx)
    .await?;
    let state_updated = sqlx::query(
        "UPDATE muriarc_deployment_state
            SET write_lease_id = ?, updated_at = ?
          WHERE singleton = 1 AND generation_id = ? AND write_lease_id IS NULL",
    )
    .bind(lease_id.to_string())
    .bind(now)
    .bind(generation_id.to_string())
    .execute(&mut *tx)
    .await?;
    let business_write = sqlx::query(
        "UPDATE labs SET name = name WHERE id = (SELECT id FROM labs ORDER BY id LIMIT 1)",
    )
    .execute(&mut *tx)
    .await?;
    let first_write: Option<DateTime<Utc>> = sqlx::query_scalar(
        "SELECT first_write_at FROM muriarc_deployment_state WHERE singleton = 1",
    )
    .fetch_one(&mut *tx)
    .await?;
    if state_updated.rows_affected() != 1
        || business_write.rows_affected() != 1
        || first_write.is_none()
    {
        return Err(DesktopUpgradeError::CandidateInvalid);
    }
    tx.rollback().await?;
    let state = store
        .compatibility_report()
        .await?
        .require_read_only_compatible()
        .cloned()
        .map_err(|_| DesktopUpgradeError::CandidateInvalid)?;
    if state.first_write_at.is_some() || state.write_lease_id.is_some() {
        return Err(DesktopUpgradeError::CandidateInvalid);
    }
    Ok(())
}

async fn ensure_no_first_write(
    store: &SqliteStore,
    generation_id: Uuid,
) -> Result<(), DesktopUpgradeError> {
    let first_write: Option<DateTime<Utc>> = sqlx::query_scalar(
        "SELECT first_write_at FROM muriarc_generation_sets WHERE generation_id = ?",
    )
    .bind(generation_id.to_string())
    .fetch_optional(store.pool())
    .await?
    .flatten();
    if first_write.is_some() {
        return Err(DesktopUpgradeError::FirstWriteBlocksRollback);
    }
    Ok(())
}

async fn verify_attachment_objects(
    pool: &sqlx::SqlitePool,
    attachment_root: &Path,
) -> Result<(), DesktopUpgradeError> {
    let rows = sqlx::query(
        "SELECT relative_path, size_bytes, sha256 FROM attachments
         UNION ALL
         SELECT relative_path, size_bytes, sha256 FROM attachment_derivatives
          WHERE relative_path IS NOT NULL AND size_bytes IS NOT NULL AND sha256 IS NOT NULL",
    )
    .fetch_all(pool)
    .await?;
    if rows.is_empty() {
        require_real_directory(attachment_root)?;
        return Ok(());
    }
    let root = canonical_real_directory(attachment_root)?;
    for row in rows {
        let relative: String = row.try_get("relative_path")?;
        let size: i64 = row.try_get("size_bytes")?;
        let expected: String = row.try_get("sha256")?;
        if size < 0 || !valid_bare_sha256(&expected) {
            return Err(DesktopUpgradeError::CandidateInvalid);
        }
        let path = resolve_relative_without_symlinks(&root, &relative)?;
        let metadata = fs::metadata(&path)?;
        if metadata.len()
            != u64::try_from(size).map_err(|_| DesktopUpgradeError::CandidateInvalid)?
            || file_sha256_bare(&path)? != expected.to_ascii_lowercase()
        {
            return Err(DesktopUpgradeError::CandidateInvalid);
        }
    }
    Ok(())
}

async fn digest_attachment_metadata(
    pool: &sqlx::SqlitePool,
) -> Result<String, DesktopUpgradeError> {
    let rows =
        sqlx::query("SELECT id, relative_path, size_bytes, sha256 FROM attachments ORDER BY id")
            .fetch_all(pool)
            .await?;
    let mut hasher = Sha256::new();
    for row in rows {
        for value in [
            row.try_get::<String, _>("id")?,
            row.try_get::<String, _>("relative_path")?,
            row.try_get::<i64, _>("size_bytes")?.to_string(),
            row.try_get::<String, _>("sha256")?,
        ] {
            hasher.update(value.as_bytes());
            hasher.update([0]);
        }
    }
    Ok(format!("sha256:{:x}", hasher.finalize()))
}

fn resolve_relative_without_symlinks(
    root: &Path,
    relative: &str,
) -> Result<PathBuf, DesktopUpgradeError> {
    let relative_path = Path::new(relative);
    if relative_path.is_absolute() {
        return Err(DesktopUpgradeError::CandidateInvalid);
    }
    let mut current = root.to_path_buf();
    for component in relative_path.components() {
        let Component::Normal(component) = component else {
            return Err(DesktopUpgradeError::CandidateInvalid);
        };
        current.push(component);
        let metadata = fs::symlink_metadata(&current)?;
        if metadata.file_type().is_symlink() {
            return Err(DesktopUpgradeError::CandidateInvalid);
        }
    }
    let canonical = fs::canonicalize(&current)?;
    if !canonical.starts_with(root) || !canonical.is_file() {
        return Err(DesktopUpgradeError::CandidateInvalid);
    }
    Ok(canonical)
}

fn archive_completed(
    config_root: &Path,
    intent: DesktopUpgradeIntent,
    final_snapshot: UpgradeSnapshot,
    paths: &DesktopUpgradePaths,
) -> Result<(), DesktopUpgradeError> {
    let history_root = config_root.join(HISTORY_DIRECTORY);
    require_real_directory(&history_root)?;
    let history_path = history_root.join(format!("{}.json", intent.operation_id));
    recover_atomic_file(&history_path)?;
    let completed = CompletedDesktopUpgrade {
        intent,
        final_snapshot,
        recovery_root: paths.recovery_root.clone(),
        candidate_root: paths.candidate_root.clone(),
        completed_at: Utc::now(),
    };
    match read_json_optional::<CompletedDesktopUpgrade>(&history_path)? {
        Some(existing)
            if existing.intent.operation_id == completed.intent.operation_id
                && existing.final_snapshot.phase == UpgradePhase::Completed
                && existing.final_snapshot.status == UpgradeStatus::Succeeded => {}
        Some(_) => return Err(DesktopUpgradeError::JournalIntegrity),
        None => write_json_atomic(&history_path, &completed)?,
    }
    Ok(())
}

fn digest_json<T: Serialize>(value: &T) -> Result<String, DesktopUpgradeError> {
    Ok(format!(
        "sha256:{:x}",
        Sha256::digest(serde_json::to_vec(value)?)
    ))
}

fn file_sha256(path: &Path) -> Result<String, DesktopUpgradeError> {
    Ok(format!("sha256:{}", file_sha256_bare(path)?))
}

fn file_sha256_bare(path: &Path) -> Result<String, DesktopUpgradeError> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn valid_sha256(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(valid_bare_sha256)
}

fn valid_bare_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn canonical_real_directory(path: &Path) -> Result<PathBuf, DesktopUpgradeError> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(DesktopUpgradeError::RecoveryInvalid);
    }
    Ok(fs::canonicalize(path)?)
}

fn require_real_directory(path: &Path) -> Result<(), DesktopUpgradeError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => Ok(()),
        Ok(_) => Err(DesktopUpgradeError::RecoveryInvalid),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir_all(path)?;
            let metadata = fs::symlink_metadata(path)?;
            if metadata.is_dir() && !metadata.file_type().is_symlink() {
                Ok(())
            } else {
                Err(DesktopUpgradeError::RecoveryInvalid)
            }
        }
        Err(error) => Err(error.into()),
    }
}

fn require_real_file(path: &Path) -> Result<(), DesktopUpgradeError> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.is_file() && !metadata.file_type().is_symlink() {
        Ok(())
    } else {
        Err(DesktopUpgradeError::RecoveryInvalid)
    }
}

fn remove_partial_directory(path: &Path, operation_root: &Path) -> Result<(), DesktopUpgradeError> {
    if path == operation_root
        || !path.starts_with(operation_root)
        || fs::symlink_metadata(path)?.file_type().is_symlink()
    {
        return Err(DesktopUpgradeError::RecoveryInvalid);
    }
    fs::remove_dir_all(path)?;
    Ok(())
}

fn paths_overlap(left: &Path, right: &Path) -> bool {
    left.starts_with(right) || right.starts_with(left)
}

fn stage_binary_recovery(
    source_executable: &Path,
    paths: &DesktopUpgradePaths,
    intent: &DesktopUpgradeIntent,
) -> Result<(), DesktopUpgradeError> {
    require_real_file(source_executable)?;
    let recovery_parent = paths
        .binary_recovery_root
        .parent()
        .ok_or(DesktopUpgradeError::BinaryRecoveryInvalid)?;
    require_real_directory(recovery_parent)?;
    if paths.binary_recovery_root.exists() {
        return Err(DesktopUpgradeError::OperationBusy);
    }
    fs::create_dir(&paths.binary_recovery_root)?;
    let mut source = File::open(source_executable)?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o700);
    }
    let mut target = options.open(&paths.fallback_executable)?;
    std::io::copy(&mut source, &mut target)?;
    target.sync_all()?;
    validate_recovery_executable(
        &paths.fallback_executable,
        &DesktopBinaryFallback {
            version: 1,
            operation_id: intent.operation_id,
            state: BinaryFallbackState::Armed,
            source_application_version: intent.source_application_version.clone(),
            failed_application_version: intent.release_manifest.application_version.to_string(),
            source_generation_id: intent.source_generation_id,
            intent_digest: digest_json(intent)?,
            executable_sha256: intent.source_executable_sha256.clone(),
            executable_size_bytes: intent.source_executable_size_bytes,
            created_at: intent.requested_at,
            activated_at: None,
        },
    )
}

fn replace_or_arm_binary_fallback(
    config_root: &Path,
    replacement: &DesktopBinaryFallback,
) -> Result<(), DesktopUpgradeError> {
    let path = config_root.join(BINARY_FALLBACK_FILE);
    recover_atomic_file(&path)?;
    if let Some(existing) = read_json_optional::<DesktopBinaryFallback>(&path)? {
        validate_binary_fallback(config_root, &existing)?;
        if existing.state == BinaryFallbackState::Armed
            || replacement.source_application_version != existing.source_application_version
            || replacement.failed_application_version == existing.failed_application_version
        {
            return Err(DesktopUpgradeError::OperationBusy);
        }
    }
    write_json_atomic(&path, replacement)?;
    Ok(())
}

fn remove_binary_fallback_if_armed(
    config_root: &Path,
    operation_id: Uuid,
) -> Result<(), DesktopUpgradeError> {
    let path = config_root.join(BINARY_FALLBACK_FILE);
    recover_atomic_file(&path)?;
    let Some(fallback) = read_json_optional::<DesktopBinaryFallback>(&path)? else {
        return Ok(());
    };
    validate_binary_fallback(config_root, &fallback)?;
    if fallback.operation_id != operation_id {
        return Err(DesktopUpgradeError::OperationBusy);
    }
    if fallback.state == BinaryFallbackState::Armed {
        remove_atomic_file(&path)?;
    }
    Ok(())
}

fn remove_binary_fallback_for_success(config_root: &Path) -> Result<(), DesktopUpgradeError> {
    let path = config_root.join(BINARY_FALLBACK_FILE);
    recover_atomic_file(&path)?;
    if let Some(fallback) = read_json_optional::<DesktopBinaryFallback>(&path)? {
        validate_binary_fallback(config_root, &fallback)?;
        if fallback.state != BinaryFallbackState::Armed {
            return Err(DesktopUpgradeError::FirstWriteBlocksRollback);
        }
        remove_atomic_file(&path)?;
    }
    Ok(())
}

fn validate_binary_fallback(
    config_root: &Path,
    fallback: &DesktopBinaryFallback,
) -> Result<(), DesktopUpgradeError> {
    if fallback.version != 1
        || fallback.operation_id.is_nil()
        || fallback.source_generation_id.is_nil()
        || fallback.source_application_version.trim().is_empty()
        || fallback.failed_application_version.trim().is_empty()
        || (!cfg!(test)
            && fallback.source_application_version == fallback.failed_application_version)
        || !valid_sha256(&fallback.intent_digest)
        || !valid_sha256(&fallback.executable_sha256)
        || fallback.executable_size_bytes == 0
        || (fallback.state == BinaryFallbackState::Armed && fallback.activated_at.is_some())
        || (fallback.state == BinaryFallbackState::Fallback && fallback.activated_at.is_none())
    {
        return Err(DesktopUpgradeError::BinaryRecoveryInvalid);
    }
    validate_recovery_executable(
        &binary_recovery_executable(config_root, fallback.operation_id),
        fallback,
    )
}

fn binary_recovery_executable(config_root: &Path, operation_id: Uuid) -> PathBuf {
    config_root
        .join(BINARY_RECOVERY_DIRECTORY)
        .join(operation_id.to_string())
        .join(FALLBACK_EXECUTABLE_FILE)
}

fn validate_recovery_executable(
    executable: &Path,
    fallback: &DesktopBinaryFallback,
) -> Result<(), DesktopUpgradeError> {
    let size = require_real_file_size(executable)?;
    if size != fallback.executable_size_bytes
        || file_sha256(executable)? != fallback.executable_sha256
    {
        return Err(DesktopUpgradeError::BinaryRecoveryInvalid);
    }
    Ok(())
}

fn require_real_file_size(path: &Path) -> Result<u64, DesktopUpgradeError> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() == 0 {
        return Err(DesktopUpgradeError::BinaryRecoveryInvalid);
    }
    Ok(metadata.len())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use muriarc_core::{BackendStateDigest, DeploymentState, MigrationClass, ReleaseArtifact};
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn wrapped_minisign_metadata_is_verified_and_tampering_is_rejected() {
        let public_key = "untrusted comment: minisign public key\n\
RWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3";
        let signature = "untrusted comment: signature from minisign secret key\n\
RWQf6LRCGA9i59SLOFxz6NxvASXDJeRtuZykwQepbDEGt87ig1BNpWaVWuNrm73YiIiJbq71Wi+dP9eKL8OC351vwIasSSbXxwA=\n\
trusted comment: timestamp:1555779966\tfile:test\n\
QtKMXWyYcwdpZAlPF7tE2ENJkRd1ujvKjlj1m9RtHTBnZPa5WKU5uWRs5GoP5M/VqE81QFuMKI5k/SfNQUaOAA==";
        let wrapped_public_key = STANDARD.encode(public_key);
        let wrapped_signature = STANDARD.encode(signature);
        verify_minisign_payload(b"test", &wrapped_signature, &wrapped_public_key).unwrap();
        assert!(matches!(
            verify_minisign_payload(b"tampered", &wrapped_signature, &wrapped_public_key),
            Err(DesktopUpgradeError::TargetInvalid)
        ));
    }

    async fn source_fixture() -> (TempDir, PathBuf, DeploymentState) {
        let temp = tempfile::tempdir().unwrap();
        let config = temp.path().join("config");
        fs::create_dir(&config).unwrap();
        fs::create_dir(config.join("attachments")).unwrap();
        fs::create_dir(config.join("data")).unwrap();
        let store = SqliteStore::connect_path(config.join(DATABASE_FILE))
            .await
            .unwrap();
        store.migrate().await.unwrap();
        let now = Utc::now();
        sqlx::query(
            "INSERT INTO labs (id, name, created_at, updated_at, deleted_at, revision)
             VALUES (?, 'Upgrade Test Lab', ?, ?, NULL, 1)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(now)
        .bind(now)
        .execute(store.pool())
        .await
        .unwrap();
        let state = store
            .compatibility_report()
            .await
            .unwrap()
            .require_compatible()
            .unwrap()
            .clone();
        write_manifest_atomic(
            &config.join(GENERATION_MANIFEST_FILE),
            &DeploymentGenerationManifest::from_state(&state),
        )
        .unwrap();
        (temp, config, state)
    }

    fn release_for(state: &DeploymentState, artifact_digest: &str) -> ReleaseManifest {
        let postgres: BackendStateDigest =
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                .parse()
                .unwrap();
        ReleaseManifest {
            format_version: 1,
            application_version: state.identity.application_version.clone(),
            data_epoch: state.identity.data_epoch.clone(),
            gateway_contract_revision: state.identity.gateway_contract_revision.clone(),
            backend_states: BTreeMap::from([
                (
                    BackendKind::Sqlite,
                    state.identity.backend_state_digest.clone(),
                ),
                (BackendKind::Postgres, postgres),
            ]),
            postgres_major: 17,
            bootstrap_protocol_revision: 1,
            controller_protocol_min: 1,
            controller_protocol_max: 1,
            migration_class: MigrationClass::M3,
            artifacts: BTreeMap::from([(
                "desktop-test".to_owned(),
                ReleaseArtifact {
                    media_type: "application/vnd.muriarc.desktop-updater.v1".to_owned(),
                    digest: artifact_digest.parse().unwrap(),
                    size_bytes: 16,
                },
            )]),
        }
    }

    async fn schedule_test(config: &Path, state: &DeploymentState) -> Uuid {
        let artifact = format!("sha256:{}", "b".repeat(64));
        let executable = config
            .parent()
            .unwrap()
            .join(format!("MuriArc-test-{}.exe", Uuid::new_v4()));
        fs::write(&executable, b"synthetic signed desktop executable").unwrap();
        schedule_verified_update(
            config,
            &executable,
            VerifiedDesktopUpdate {
                version: state.identity.application_version.to_string(),
                updater_target: "windows-x86_64-nsis".to_owned(),
                artifact_name: "desktop-test".to_owned(),
                artifact_sha256: artifact.clone(),
                artifact_size_bytes: 16,
                release_manifest: release_for(state, &artifact),
                release_manifest_signature: "test-only-manifest-signature".to_owned(),
            },
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn upgrade_uses_verified_recovery_and_switches_only_after_candidate_validation() {
        let (_temp, config, source_state) = source_fixture().await;
        let operation_id = schedule_test(&config, &source_state).await;
        resume_pending_upgrade_inner(&config, true).await.unwrap();

        assert!(!config.join(INTENT_FILE).exists());
        assert!(!config.join(BINARY_FALLBACK_FILE).exists());
        let history: CompletedDesktopUpgrade = read_json_optional(
            &config
                .join(HISTORY_DIRECTORY)
                .join(format!("{operation_id}.json")),
        )
        .unwrap()
        .unwrap();
        assert_eq!(history.final_snapshot.phase, UpgradePhase::Completed);
        assert_eq!(history.final_snapshot.status, UpgradeStatus::Succeeded);
        assert!(history.final_snapshot.evidence.switch.is_some());
        assert!(
            history
                .final_snapshot
                .evidence
                .activation_verification
                .is_some()
        );
        assert!(history.final_snapshot.evidence.write_lease.is_some());
        assert!(history.recovery_root.join(DATABASE_FILE).is_file());
        assert!(history.candidate_root.join(DATABASE_FILE).is_file());
        let active = resolve_active_root_for_upgrade(&config).unwrap();
        assert_eq!(active, fs::canonicalize(history.candidate_root).unwrap());
        let source_store = SqliteStore::connect_path(config.join(DATABASE_FILE))
            .await
            .unwrap();
        let preserved = source_store
            .compatibility_report()
            .await
            .unwrap()
            .observed
            .unwrap();
        assert_eq!(preserved.generation_id, source_state.generation_id);
        let source_operation: String = sqlx::query_scalar(
            "SELECT status FROM muriarc_upgrade_operations WHERE operation_id = ?",
        )
        .bind(operation_id.to_string())
        .fetch_one(source_store.pool())
        .await
        .unwrap();
        assert_eq!(source_operation, "succeeded");
        let candidate_store = SqliteStore::connect_path(active.join(DATABASE_FILE))
            .await
            .unwrap();
        let candidate_operation: String = sqlx::query_scalar(
            "SELECT status FROM muriarc_upgrade_operations WHERE operation_id = ?",
        )
        .bind(operation_id.to_string())
        .fetch_one(candidate_store.pool())
        .await
        .unwrap();
        assert_eq!(candidate_operation, "succeeded");
    }

    #[tokio::test]
    async fn tampered_intent_path_is_rejected_before_copying() {
        let (temp, config, source_state) = source_fixture().await;
        schedule_test(&config, &source_state).await;
        let intent_path = config.join(INTENT_FILE);
        let mut intent: DesktopUpgradeIntent = read_json_optional(&intent_path).unwrap().unwrap();
        let attacker = temp.path().join("attacker");
        fs::create_dir(&attacker).unwrap();
        intent.source_data_root = attacker;
        write_json_atomic(&intent_path, &intent).unwrap();
        let error = resume_pending_upgrade_inner(&config, true)
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            DesktopUpgradeError::JournalIntegrity | DesktopUpgradeError::RecoveryInvalid
        ));
    }

    #[tokio::test]
    async fn incomplete_candidate_copy_is_rebuilt_from_the_verified_recovery() {
        let (_temp, config, source_state) = source_fixture().await;
        schedule_test(&config, &source_state).await;
        let intent: DesktopUpgradeIntent = read_json_optional(&config.join(INTENT_FILE))
            .unwrap()
            .unwrap();
        let paths = derive_paths(&config, &intent).unwrap();
        fs::create_dir(&paths.operation_root).unwrap();
        fs::create_dir(&paths.candidate_root).unwrap();
        fs::write(paths.candidate_root.join("partial"), b"partial").unwrap();

        resume_pending_upgrade_inner(&config, true).await.unwrap();
        assert!(!paths.candidate_root.join("partial").exists());
        assert!(paths.candidate_root.join(DATABASE_FILE).is_file());
    }

    #[tokio::test]
    async fn candidate_verification_failure_never_switches_the_source_locator() {
        let (_temp, config, source_state) = source_fixture().await;
        schedule_test(&config, &source_state).await;
        let intent: DesktopUpgradeIntent = read_json_optional(&config.join(INTENT_FILE))
            .unwrap()
            .unwrap();
        let paths = derive_paths(&config, &intent).unwrap();
        checkpoint_and_verify_database(&config.join(DATABASE_FILE))
            .await
            .unwrap();
        let digest = tree_digest(&config).unwrap();
        prepare_verified_copy(
            &config,
            &paths.recovery_root,
            &paths.operation_root,
            &digest,
            true,
        )
        .await
        .unwrap();
        prepare_verified_copy(
            &paths.recovery_root,
            &paths.candidate_root,
            &paths.operation_root,
            &digest,
            true,
        )
        .await
        .unwrap();
        let source_store = SqliteStore::connect_path(config.join(DATABASE_FILE))
            .await
            .unwrap();
        let candidate = SqliteStore::connect_path(paths.candidate_root.join(DATABASE_FILE))
            .await
            .unwrap();
        sqlx::query("DELETE FROM labs")
            .execute(candidate.pool())
            .await
            .unwrap();
        candidate.apply_upgrade_migrations().await.unwrap();
        candidate
            .prepare_upgraded_candidate(intent.source_generation_id, intent.candidate_generation_id)
            .await
            .unwrap();
        let source_inventory = source_store.persistent_recovery_inventory().await.unwrap();

        let error = verify_candidate(&intent, &paths, &source_inventory)
            .await
            .unwrap_err();
        assert!(matches!(error, DesktopUpgradeError::CandidateInvalid));
        assert_eq!(
            resolve_active_root_for_upgrade(&config).unwrap(),
            fs::canonicalize(&config).unwrap()
        );
        assert!(paths.recovery_root.join(DATABASE_FILE).is_file());
    }

    #[tokio::test]
    async fn first_write_boundary_rejects_automatic_rollback() {
        let (_temp, config, source_state) = source_fixture().await;
        schedule_test(&config, &source_state).await;
        resume_pending_upgrade_inner(&config, true).await.unwrap();
        let active = resolve_active_root_for_upgrade(&config).unwrap();
        let store = SqliteStore::connect_path(active.join(DATABASE_FILE))
            .await
            .unwrap();
        sqlx::query("UPDATE labs SET name = name")
            .execute(store.pool())
            .await
            .unwrap();
        let generation = store
            .compatibility_report()
            .await
            .unwrap()
            .observed
            .unwrap()
            .generation_id;
        let error = ensure_no_first_write(&store, generation).await.unwrap_err();
        assert!(matches!(
            error,
            DesktopUpgradeError::FirstWriteBlocksRollback
        ));
    }

    #[tokio::test]
    async fn candidate_failure_arms_a_verified_old_binary_without_switching_data() {
        let (_temp, config, source_state) = source_fixture().await;
        let operation_id = schedule_test(&config, &source_state).await;

        let executable = activate_binary_fallback_after_failure(&config)
            .await
            .unwrap();
        assert!(executable.is_file());
        let fallback: DesktopBinaryFallback =
            read_json_optional(&config.join(BINARY_FALLBACK_FILE))
                .unwrap()
                .unwrap();
        assert_eq!(fallback.operation_id, operation_id);
        assert_eq!(fallback.state, BinaryFallbackState::Fallback);
        assert_eq!(
            resolve_active_root_for_upgrade(&config).unwrap(),
            fs::canonicalize(&config).unwrap()
        );
    }

    #[tokio::test]
    async fn tampered_old_binary_is_never_used_for_fallback() {
        let (_temp, config, source_state) = source_fixture().await;
        schedule_test(&config, &source_state).await;
        let fallback: DesktopBinaryFallback =
            read_json_optional(&config.join(BINARY_FALLBACK_FILE))
                .unwrap()
                .unwrap();
        fs::write(
            binary_recovery_executable(&config, fallback.operation_id),
            b"tampered",
        )
        .unwrap();

        let error = activate_binary_fallback_after_failure(&config)
            .await
            .unwrap_err();
        assert!(matches!(error, DesktopUpgradeError::BinaryRecoveryInvalid));
        assert_eq!(
            resolve_active_root_for_upgrade(&config).unwrap(),
            fs::canonicalize(&config).unwrap()
        );
    }
}
