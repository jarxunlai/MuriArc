use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex},
};

use chrono::Utc;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use sqlx::{Connection, SqliteConnection, sqlite::SqliteConnectOptions};
use thiserror::Error;
use uuid::Uuid;

const STORAGE_VERSION: u32 = 1;
const LOCATOR_FILE: &str = "storage-location.json";
const MIGRATION_FILE: &str = "storage-migration.json";
const ROOT_MARKER_FILE: &str = ".muriarc-storage-root.json";
const BACKUP_DIRECTORY: &str = "storage-backups";
const DESKTOP_UPGRADE_INTENT_FILE: &str = "desktop-upgrade-intent.json";
const DESKTOP_UPGRADE_HISTORY_DIRECTORY: &str = "desktop-upgrade-history";
const DESKTOP_UPGRADE_JOURNAL_DIRECTORY: &str = "upgrade-journals";
const MANAGED_ENTRIES: [&str; 5] = [
    "muriarc.sqlite3",
    "attachments",
    "data",
    "ai-provider.json",
    "deployment-generation.json",
];

#[derive(Debug, Error)]
pub(crate) enum StorageRootError {
    #[error("local storage configuration is invalid")]
    InvalidConfiguration,
    #[error("the selected local storage directory is not supported")]
    InvalidTarget,
    #[error("the selected local storage directory is not empty")]
    TargetNotEmpty,
    #[error("the active local storage directory is unavailable")]
    ActiveRootUnavailable,
    #[error("a local storage migration is already pending")]
    MigrationPending,
    #[error("the local storage migration could not be verified")]
    VerificationFailed,
    #[error("local storage I/O failed")]
    Io(#[source] io::Error),
    #[error("local storage database verification failed")]
    Database(#[source] sqlx::Error),
    #[error("local storage metadata is invalid")]
    Metadata(#[source] serde_json::Error),
    #[error("the selected directory is no longer available")]
    SelectionExpired,
    #[error("the local storage dialog could not be completed")]
    DialogUnavailable,
    #[error("the local storage directory could not be opened")]
    OpenFailed,
}

impl StorageRootError {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::InvalidTarget | Self::TargetNotEmpty => "invalid_storage_target",
            Self::ActiveRootUnavailable => "storage_root_unavailable",
            Self::MigrationPending => "storage_migration_pending",
            Self::SelectionExpired => "storage_selection_expired",
            Self::VerificationFailed => "storage_verification_failed",
            Self::InvalidConfiguration | Self::Metadata(_) => "storage_configuration_error",
            Self::Io(_) | Self::Database(_) | Self::DialogUnavailable | Self::OpenFailed => {
                "storage_error"
            }
        }
    }

    pub(crate) fn safe_message(&self) -> &'static str {
        match self {
            Self::InvalidTarget => "请选择本机固定磁盘上的独立空目录，且不要选择程序安装目录",
            Self::TargetNotEmpty => {
                "目标目录必须为空；默认目录中的既有 MuriArc 数据会自动保留为备份"
            }
            Self::ActiveRootUnavailable => "当前本地数据目录不可用；请重新连接磁盘后再启动 MuriArc",
            Self::MigrationPending => "已有本地数据迁移等待重启执行",
            Self::SelectionExpired => "目录选择已经失效，请重新选择",
            Self::VerificationFailed => "本地数据迁移校验失败，仍将继续使用原目录",
            Self::InvalidConfiguration | Self::Metadata(_) => {
                "本地数据位置配置无效；程序没有创建新的空数据库"
            }
            Self::Io(_) | Self::Database(_) => "本地数据迁移失败，仍将继续使用原目录",
            Self::DialogUnavailable => "无法完成本地数据目录选择，请重试",
            Self::OpenFailed => "无法打开当前本地数据目录",
        }
    }
}

impl From<io::Error> for StorageRootError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<sqlx::Error> for StorageRootError {
    fn from(error: sqlx::Error) -> Self {
        Self::Database(error)
    }
}

impl From<serde_json::Error> for StorageRootError {
    fn from(error: serde_json::Error) -> Self {
        Self::Metadata(error)
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LocalStorageStatus {
    pub active_data_root: String,
    pub default_data_root: String,
    pub uses_custom_root: bool,
    pub migration_pending: bool,
    pub pending_target_root: Option<String>,
    pub requires_restart: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LocalStorageSelection {
    pub selection_token: String,
    pub target_data_root: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RequestStorageMigrationInput {
    pub selection_token: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StorageMigrationRequestResult {
    pub scheduled: bool,
    pub requires_restart: bool,
    pub target_data_root: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StorageLocator {
    version: u32,
    active_data_root: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StorageMigrationIntent {
    version: u32,
    migration_id: Uuid,
    source_data_root: PathBuf,
    target_data_root: PathBuf,
    requested_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct TreeDigest {
    pub(crate) file_count: u64,
    pub(crate) directory_count: u64,
    pub(crate) total_bytes: u64,
    pub(crate) sha256: String,
}

pub(crate) fn resolve_active_root_for_upgrade(
    config_root: &Path,
) -> Result<PathBuf, StorageRootError> {
    let config_root = canonical_directory(config_root)?;
    recover_atomic_file(&config_root.join(LOCATOR_FILE))?;
    recover_atomic_file(&config_root.join(MIGRATION_FILE))?;
    if read_json_optional::<StorageMigrationIntent>(&config_root.join(MIGRATION_FILE))?.is_some() {
        return Err(StorageRootError::MigrationPending);
    }
    match read_json_optional::<StorageLocator>(&config_root.join(LOCATOR_FILE))? {
        Some(locator) if locator.version == STORAGE_VERSION => {
            canonical_directory(&locator.active_data_root)
                .map_err(|_| StorageRootError::ActiveRootUnavailable)
        }
        Some(_) => Err(StorageRootError::InvalidConfiguration),
        None => Ok(config_root),
    }
}

pub(crate) fn activate_root_for_upgrade(
    config_root: &Path,
    candidate_root: &Path,
) -> Result<(), StorageRootError> {
    let config_root = canonical_directory(config_root)?;
    let candidate_root = canonical_directory(candidate_root)?;
    if paths_overlap(&config_root, &candidate_root) {
        return Err(StorageRootError::InvalidTarget);
    }
    write_json_atomic(
        &config_root.join(LOCATOR_FILE),
        &StorageLocator {
            version: STORAGE_VERSION,
            active_data_root: candidate_root,
        },
    )
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StorageRootMarker {
    version: u32,
    migration_id: Uuid,
    manifest: TreeDigest,
    created_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum DefaultFinalizePhase {
    BackingUp,
    Installing,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DefaultFinalizeState {
    version: u32,
    migration_id: Uuid,
    phase: DefaultFinalizePhase,
    old_entries: Vec<String>,
}

#[derive(Debug, Clone)]
struct SelectedTarget {
    token: String,
    path: PathBuf,
}

#[derive(Debug, Clone)]
pub(crate) struct StorageRootState {
    config_root: PathBuf,
    default_root: PathBuf,
    active_root: PathBuf,
    install_root: PathBuf,
    selection: Arc<Mutex<Option<SelectedTarget>>>,
}

impl StorageRootState {
    pub(crate) async fn initialize(
        config_root: impl AsRef<Path>,
        install_root: impl AsRef<Path>,
    ) -> Result<Self, StorageRootError> {
        fs::create_dir_all(config_root.as_ref())?;
        let config_root = canonical_directory(config_root.as_ref())?;
        let install_root = canonical_directory(install_root.as_ref())?;
        recover_atomic_file(&config_root.join(LOCATOR_FILE))?;
        recover_atomic_file(&config_root.join(MIGRATION_FILE))?;

        let locator_path = config_root.join(LOCATOR_FILE);
        let locator = read_json_optional::<StorageLocator>(&locator_path)?;
        if locator
            .as_ref()
            .is_some_and(|value| value.version != STORAGE_VERSION)
        {
            return Err(StorageRootError::InvalidConfiguration);
        }
        let mut active_root = match locator {
            Some(locator) => canonical_directory(&locator.active_data_root)
                .map_err(|_| StorageRootError::ActiveRootUnavailable)?,
            None => config_root.clone(),
        };

        let migration_path = config_root.join(MIGRATION_FILE);
        if let Some(intent) = read_json_optional::<StorageMigrationIntent>(&migration_path)? {
            if intent.version != STORAGE_VERSION {
                return Err(StorageRootError::InvalidConfiguration);
            }
            let source = canonical_directory(&intent.source_data_root)
                .map_err(|_| StorageRootError::ActiveRootUnavailable)?;
            let target = canonical_pending_target(&intent.target_data_root, intent.migration_id)?;
            if active_root == target {
                remove_atomic_file(&migration_path)?;
            } else {
                if active_root != source {
                    return Err(StorageRootError::InvalidConfiguration);
                }
                validate_pending_target(
                    &source,
                    &target,
                    &config_root,
                    &install_root,
                    intent.migration_id,
                )?;
                perform_migration(&config_root, &intent, &source, &target).await?;
                write_json_atomic(
                    &locator_path,
                    &StorageLocator {
                        version: STORAGE_VERSION,
                        active_data_root: target.clone(),
                    },
                )?;
                remove_atomic_file(&migration_path)?;
                active_root = target;
            }
        }

        if !active_root.is_dir() {
            return Err(StorageRootError::ActiveRootUnavailable);
        }

        Ok(Self {
            default_root: config_root.clone(),
            config_root,
            active_root,
            install_root,
            selection: Arc::new(Mutex::new(None)),
        })
    }

    pub(crate) fn active_root(&self) -> &Path {
        &self.active_root
    }

    pub(crate) fn status(&self) -> Result<LocalStorageStatus, StorageRootError> {
        let pending =
            read_json_optional::<StorageMigrationIntent>(&self.config_root.join(MIGRATION_FILE))?;
        Ok(LocalStorageStatus {
            active_data_root: display_path(&self.active_root),
            default_data_root: display_path(&self.default_root),
            uses_custom_root: self.active_root != self.default_root,
            migration_pending: pending.is_some(),
            pending_target_root: pending
                .as_ref()
                .map(|intent| display_path(&intent.target_data_root)),
            requires_restart: pending.is_some(),
        })
    }

    pub(crate) fn select_target(
        &self,
        target: impl AsRef<Path>,
    ) -> Result<LocalStorageSelection, StorageRootError> {
        let target = canonical_directory(target.as_ref())?;
        self.validate_target(&target, false)?;
        let token = Uuid::new_v4().to_string();
        *self
            .selection
            .lock()
            .map_err(|_| StorageRootError::InvalidConfiguration)? = Some(SelectedTarget {
            token: token.clone(),
            path: target.clone(),
        });
        Ok(LocalStorageSelection {
            selection_token: token,
            target_data_root: display_path(&target),
        })
    }

    pub(crate) fn selected_target(&self, token: &str) -> Result<PathBuf, StorageRootError> {
        let selection = self
            .selection
            .lock()
            .map_err(|_| StorageRootError::InvalidConfiguration)?;
        selection
            .as_ref()
            .filter(|selected| selected.token == token)
            .map(|selected| selected.path.clone())
            .ok_or(StorageRootError::SelectionExpired)
    }

    pub(crate) fn schedule_selected(
        &self,
        token: &str,
    ) -> Result<StorageMigrationRequestResult, StorageRootError> {
        let selected = {
            let mut selection = self
                .selection
                .lock()
                .map_err(|_| StorageRootError::InvalidConfiguration)?;
            let selected = selection
                .take()
                .filter(|selected| selected.token == token)
                .ok_or(StorageRootError::SelectionExpired)?;
            selected.path
        };
        self.schedule_target(&selected, false)
    }

    pub(crate) fn schedule_default(
        &self,
    ) -> Result<StorageMigrationRequestResult, StorageRootError> {
        if self.active_root == self.default_root {
            return Ok(StorageMigrationRequestResult {
                scheduled: false,
                requires_restart: false,
                target_data_root: display_path(&self.default_root),
            });
        }
        self.schedule_target(&self.default_root, true)
    }

    fn schedule_target(
        &self,
        target: &Path,
        restoring_default: bool,
    ) -> Result<StorageMigrationRequestResult, StorageRootError> {
        if self.config_root.join(MIGRATION_FILE).exists() {
            return Err(StorageRootError::MigrationPending);
        }
        self.validate_target(target, restoring_default)?;
        let intent = StorageMigrationIntent {
            version: STORAGE_VERSION,
            migration_id: Uuid::new_v4(),
            source_data_root: self.active_root.clone(),
            target_data_root: target.to_path_buf(),
            requested_at: Utc::now().to_rfc3339(),
        };
        write_json_atomic(&self.config_root.join(MIGRATION_FILE), &intent)?;
        Ok(StorageMigrationRequestResult {
            scheduled: true,
            requires_restart: true,
            target_data_root: display_path(target),
        })
    }

    fn validate_target(
        &self,
        target: &Path,
        restoring_default: bool,
    ) -> Result<(), StorageRootError> {
        if target == self.active_root
            || paths_overlap(target, &self.active_root)
            || target.starts_with(&self.install_root)
            || is_unsupported_windows_path(target)
        {
            return Err(StorageRootError::InvalidTarget);
        }
        if restoring_default {
            if target != self.default_root {
                return Err(StorageRootError::InvalidTarget);
            }
            validate_default_target_entries(target)?;
        } else if fs::read_dir(target)?.next().transpose()?.is_some() {
            return Err(StorageRootError::TargetNotEmpty);
        }
        probe_writable(target)?;
        Ok(())
    }

    pub(crate) fn open_active_root(&self) -> Result<(), StorageRootError> {
        open_directory(&self.active_root)
    }
}

async fn perform_migration(
    config_root: &Path,
    intent: &StorageMigrationIntent,
    source: &Path,
    target: &Path,
) -> Result<(), StorageRootError> {
    checkpoint_and_verify_database(&source.join("muriarc.sqlite3")).await?;
    let source_manifest = tree_digest(source)?;

    if let Some(marker) = read_json_optional::<StorageRootMarker>(&target.join(ROOT_MARKER_FILE))? {
        if marker.version == STORAGE_VERSION
            && marker.migration_id == intent.migration_id
            && marker.manifest == source_manifest
            && tree_digest(target)? == marker.manifest
        {
            verify_database_read_only(&target.join("muriarc.sqlite3")).await?;
            cleanup_completed_migration(config_root, target, intent.migration_id)?;
            return Ok(());
        }
        return Err(StorageRootError::VerificationFailed);
    }

    let staging = staging_path(target, intent.migration_id)?;
    if staging.exists() {
        validate_managed_target_entries(&staging, true)?;
    }
    let reusable_staging = read_json_optional::<StorageRootMarker>(
        &staging.join(ROOT_MARKER_FILE),
    )?
    .is_some_and(|marker| {
        marker.version == STORAGE_VERSION
            && marker.migration_id == intent.migration_id
            && marker.manifest == source_manifest
            && tree_digest(&staging).is_ok_and(|manifest| manifest == source_manifest)
    });
    if staging.exists() && !reusable_staging {
        fs::remove_dir_all(&staging)?;
    }
    if !reusable_staging {
        fs::create_dir(&staging)?;
        copy_managed_tree(source, &staging)?;
        let staged_manifest = tree_digest(&staging)?;
        if staged_manifest != source_manifest {
            return Err(StorageRootError::VerificationFailed);
        }
        verify_database_read_only(&staging.join("muriarc.sqlite3")).await?;
        write_json_atomic(
            &staging.join(ROOT_MARKER_FILE),
            &StorageRootMarker {
                version: STORAGE_VERSION,
                migration_id: intent.migration_id,
                manifest: staged_manifest,
                created_at: Utc::now().to_rfc3339(),
            },
        )?;
    }

    if target == config_root {
        finalize_default_target(config_root, &staging, intent.migration_id)?;
    } else {
        if fs::read_dir(target)?.next().transpose()?.is_some() {
            return Err(StorageRootError::TargetNotEmpty);
        }
        fs::remove_dir(target)?;
        fs::rename(&staging, target)?;
    }

    let marker: StorageRootMarker = read_json_required(&target.join(ROOT_MARKER_FILE))?;
    if marker.migration_id != intent.migration_id
        || marker.manifest != source_manifest
        || tree_digest(target)? != source_manifest
    {
        return Err(StorageRootError::VerificationFailed);
    }
    verify_database_read_only(&target.join("muriarc.sqlite3")).await?;
    Ok(())
}

fn finalize_default_target(
    default_root: &Path,
    staging: &Path,
    migration_id: Uuid,
) -> Result<(), StorageRootError> {
    let backup_container = default_root.join(BACKUP_DIRECTORY);
    if backup_container.exists() {
        require_real_directory(&backup_container)?;
    } else {
        fs::create_dir(&backup_container)?;
    }
    let backup_root = backup_container.join(migration_id.to_string());
    if backup_root.exists() {
        require_real_directory(&backup_root)?;
    } else {
        fs::create_dir(&backup_root)?;
    }
    let journal_path = backup_root.join("finalize.json");
    recover_atomic_file(&journal_path)?;
    let mut state = match read_json_optional::<DefaultFinalizeState>(&journal_path)? {
        Some(state) if state.version == STORAGE_VERSION && state.migration_id == migration_id => {
            state
        }
        Some(_) => return Err(StorageRootError::InvalidConfiguration),
        None => {
            let state = DefaultFinalizeState {
                version: STORAGE_VERSION,
                migration_id,
                phase: DefaultFinalizePhase::BackingUp,
                old_entries: default_finalize_names()
                    .into_iter()
                    .filter(|name| default_root.join(name).exists())
                    .map(str::to_owned)
                    .collect(),
            };
            write_json_atomic(&journal_path, &state)?;
            state
        }
    };

    if state.phase == DefaultFinalizePhase::BackingUp {
        for name in &state.old_entries {
            let backup = backup_root.join(name);
            if backup.exists() {
                continue;
            }
            let existing = default_root.join(name);
            if !existing.exists() {
                return Err(StorageRootError::VerificationFailed);
            }
            fs::rename(existing, backup)?;
        }
        sync_directory(&backup_root)?;
        state.phase = DefaultFinalizePhase::Installing;
        write_json_atomic(&journal_path, &state)?;
    }

    for name in default_finalize_names() {
        let staged = staging.join(name);
        if staged.exists() {
            let installed = default_root.join(name);
            if installed.exists() {
                return Err(StorageRootError::VerificationFailed);
            }
            fs::rename(staged, installed)?;
        }
    }
    if staging.exists() {
        if fs::read_dir(staging)?.next().transpose()?.is_some() {
            return Err(StorageRootError::VerificationFailed);
        }
        fs::remove_dir(staging)?;
    }
    remove_atomic_file(&journal_path)?;
    if fs::read_dir(&backup_root)?.next().transpose()?.is_none() {
        fs::remove_dir(&backup_root)?;
    }
    sync_directory(default_root)?;
    Ok(())
}

fn default_finalize_names() -> [&'static str; 6] {
    [
        "muriarc.sqlite3",
        "attachments",
        "data",
        "ai-provider.json",
        "deployment-generation.json",
        ROOT_MARKER_FILE,
    ]
}

fn cleanup_completed_migration(
    config_root: &Path,
    target: &Path,
    migration_id: Uuid,
) -> Result<(), StorageRootError> {
    let staging = staging_path(target, migration_id)?;
    if staging.exists() {
        validate_managed_target_entries(&staging, true)?;
        fs::remove_dir_all(staging)?;
    }
    if target == config_root {
        let backup_root = config_root
            .join(BACKUP_DIRECTORY)
            .join(migration_id.to_string());
        if backup_root.exists() {
            require_real_directory(&backup_root)?;
            remove_atomic_file(&backup_root.join("finalize.json"))?;
        }
    }
    Ok(())
}

pub(crate) async fn checkpoint_and_verify_database(path: &Path) -> Result<(), StorageRootError> {
    if !path.is_file() {
        return Err(StorageRootError::VerificationFailed);
    }
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(false)
        .foreign_keys(true);
    let mut connection = SqliteConnection::connect_with(&options).await?;
    let result: String = sqlx::query_scalar("PRAGMA integrity_check")
        .fetch_one(&mut connection)
        .await?;
    if result != "ok" {
        return Err(StorageRootError::VerificationFailed);
    }
    let _ = sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
        .fetch_all(&mut connection)
        .await?;
    connection.close().await?;
    Ok(())
}

pub(crate) async fn verify_database_read_only(path: &Path) -> Result<(), StorageRootError> {
    if !path.is_file() {
        return Err(StorageRootError::VerificationFailed);
    }
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(false)
        .read_only(true)
        .foreign_keys(true);
    let mut connection = SqliteConnection::connect_with(&options).await?;
    let result: String = sqlx::query_scalar("PRAGMA integrity_check")
        .fetch_one(&mut connection)
        .await?;
    connection.close().await?;
    if result == "ok" {
        Ok(())
    } else {
        Err(StorageRootError::VerificationFailed)
    }
}

pub(crate) fn copy_managed_tree(source: &Path, target: &Path) -> Result<(), StorageRootError> {
    for name in MANAGED_ENTRIES {
        let source_entry = source.join(name);
        if source_entry.exists() {
            copy_entry(&source_entry, &target.join(name))?;
        }
    }
    Ok(())
}

fn copy_entry(source: &Path, target: &Path) -> Result<(), StorageRootError> {
    let metadata = fs::symlink_metadata(source)?;
    if metadata.file_type().is_symlink() {
        return Err(StorageRootError::InvalidTarget);
    }
    if metadata.is_dir() {
        fs::create_dir(target)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            copy_entry(&entry.path(), &target.join(entry.file_name()))?;
        }
        sync_directory(target)?;
    } else if metadata.is_file() {
        let mut input = File::open(source)?;
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(target)?;
        io::copy(&mut input, &mut output)?;
        output.sync_all()?;
    } else {
        return Err(StorageRootError::InvalidTarget);
    }
    Ok(())
}

pub(crate) fn tree_digest(root: &Path) -> Result<TreeDigest, StorageRootError> {
    tree_digest_for_names(root, &MANAGED_ENTRIES)
}

pub(crate) fn payload_tree_digest(root: &Path) -> Result<TreeDigest, StorageRootError> {
    tree_digest_for_names(root, &["attachments", "data", "ai-provider.json"])
}

fn tree_digest_for_names(root: &Path, names: &[&str]) -> Result<TreeDigest, StorageRootError> {
    let mut entries = Vec::new();
    for name in names {
        let entry = root.join(*name);
        if entry.exists() {
            collect_entries(root, &entry, &mut entries)?;
        }
    }
    entries.sort_by(|left, right| left.0.cmp(&right.0));

    let mut aggregate = Sha256::new();
    let mut file_count = 0_u64;
    let mut directory_count = 0_u64;
    let mut total_bytes = 0_u64;
    for (relative, path, is_dir) in entries {
        let relative = relative
            .to_str()
            .ok_or(StorageRootError::InvalidTarget)?
            .replace('\\', "/");
        if is_dir {
            directory_count += 1;
            aggregate.update(b"D\0");
            aggregate.update(relative.as_bytes());
            aggregate.update(b"\0");
        } else {
            file_count += 1;
            let (size, digest) = file_digest(&path)?;
            total_bytes = total_bytes
                .checked_add(size)
                .ok_or(StorageRootError::VerificationFailed)?;
            aggregate.update(b"F\0");
            aggregate.update(relative.as_bytes());
            aggregate.update(b"\0");
            aggregate.update(size.to_le_bytes());
            aggregate.update(digest);
        }
    }
    Ok(TreeDigest {
        file_count,
        directory_count,
        total_bytes,
        sha256: encode_hex(&aggregate.finalize()),
    })
}

fn collect_entries(
    root: &Path,
    path: &Path,
    entries: &mut Vec<(PathBuf, PathBuf, bool)>,
) -> Result<(), StorageRootError> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        return Err(StorageRootError::InvalidTarget);
    }
    let relative = path
        .strip_prefix(root)
        .map_err(|_| StorageRootError::InvalidTarget)?
        .to_path_buf();
    if metadata.is_dir() {
        entries.push((relative, path.to_path_buf(), true));
        for entry in fs::read_dir(path)? {
            collect_entries(root, &entry?.path(), entries)?;
        }
    } else if metadata.is_file() {
        entries.push((relative, path.to_path_buf(), false));
    } else {
        return Err(StorageRootError::InvalidTarget);
    }
    Ok(())
}

fn file_digest(path: &Path) -> Result<(u64, Vec<u8>), StorageRootError> {
    let mut file = File::open(path)?;
    let mut digest = Sha256::new();
    let mut size = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        size = size
            .checked_add(read as u64)
            .ok_or(StorageRootError::VerificationFailed)?;
        digest.update(&buffer[..read]);
    }
    Ok((size, digest.finalize().to_vec()))
}

fn encode_hex(bytes: &[u8]) -> String {
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(&mut encoded, "{byte:02x}");
    }
    encoded
}

fn staging_path(target: &Path, migration_id: Uuid) -> Result<PathBuf, StorageRootError> {
    let parent = target.parent().ok_or(StorageRootError::InvalidTarget)?;
    let name = target
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or(StorageRootError::InvalidTarget)?;
    Ok(parent.join(format!(".{name}.muriarc-migrating-{migration_id}")))
}

fn validate_default_target_entries(target: &Path) -> Result<(), StorageRootError> {
    for entry in fs::read_dir(target)? {
        let entry = entry?;
        let metadata = fs::symlink_metadata(entry.path())?;
        if metadata.file_type().is_symlink() {
            return Err(StorageRootError::InvalidTarget);
        }
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            return Err(StorageRootError::TargetNotEmpty);
        };
        let allowed = MANAGED_ENTRIES.contains(&name)
            || matches!(
                name,
                LOCATOR_FILE
                    | MIGRATION_FILE
                    | ROOT_MARKER_FILE
                    | BACKUP_DIRECTORY
                    | DESKTOP_UPGRADE_INTENT_FILE
                    | DESKTOP_UPGRADE_HISTORY_DIRECTORY
                    | DESKTOP_UPGRADE_JOURNAL_DIRECTORY
                    | "storage-location.json.bak"
                    | "storage-migration.json.bak"
            )
            || name.starts_with(".storage-location.json.tmp-")
            || name.starts_with(".storage-migration.json.tmp-")
            || name.starts_with(".desktop-upgrade-intent.json.tmp-");
        if !allowed {
            return Err(StorageRootError::TargetNotEmpty);
        }
        if name == BACKUP_DIRECTORY && !metadata.is_dir() {
            return Err(StorageRootError::InvalidTarget);
        }
    }
    Ok(())
}

fn probe_writable(target: &Path) -> Result<(), StorageRootError> {
    let probe = target.join(format!(".muriarc-write-probe-{}", Uuid::new_v4()));
    let result = (|| -> io::Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&probe)?;
        file.write_all(b"MuriArc")?;
        file.sync_all()?;
        fs::remove_file(&probe)?;
        Ok(())
    })();
    if probe.exists() {
        let _ = fs::remove_file(&probe);
    }
    result.map_err(StorageRootError::Io)
}

fn paths_overlap(left: &Path, right: &Path) -> bool {
    left.starts_with(right) || right.starts_with(left)
}

#[cfg(target_os = "windows")]
fn is_unsupported_windows_path(path: &Path) -> bool {
    use std::path::{Component, Prefix};
    let Some(Component::Prefix(prefix)) = path.components().next() else {
        return true;
    };
    let drive = match prefix.kind() {
        Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => letter,
        Prefix::UNC(_, _)
        | Prefix::VerbatimUNC(_, _)
        | Prefix::DeviceNS(_)
        | Prefix::Verbatim(_) => return true,
    };
    let root = [drive as u16, b':' as u16, b'\\' as u16, 0];
    // SAFETY: `root` is a valid, NUL-terminated `X:\` UTF-16 buffer for this call.
    unsafe {
        windows_sys::Win32::Storage::FileSystem::GetDriveTypeW(root.as_ptr())
            != windows_sys::Win32::System::WindowsProgramming::DRIVE_FIXED
    }
}

#[cfg(not(target_os = "windows"))]
fn is_unsupported_windows_path(_path: &Path) -> bool {
    false
}

fn canonical_directory(path: &Path) -> Result<PathBuf, StorageRootError> {
    if !path.is_absolute() {
        return Err(StorageRootError::InvalidTarget);
    }
    let metadata = fs::symlink_metadata(path).map_err(StorageRootError::Io)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(StorageRootError::InvalidTarget);
    }
    fs::canonicalize(path).map_err(StorageRootError::Io)
}

fn canonical_pending_target(path: &Path, migration_id: Uuid) -> Result<PathBuf, StorageRootError> {
    if path.is_dir() {
        return canonical_directory(path).map_err(|_| StorageRootError::InvalidTarget);
    }
    if !path.is_absolute() {
        return Err(StorageRootError::InvalidTarget);
    }
    let parent = path.parent().ok_or(StorageRootError::InvalidTarget)?;
    let parent = canonical_directory(parent).map_err(|_| StorageRootError::InvalidTarget)?;
    let name = path.file_name().ok_or(StorageRootError::InvalidTarget)?;
    let target = parent.join(name);
    let staging = staging_path(&target, migration_id)?;
    if !staging.is_dir() {
        return Err(StorageRootError::InvalidTarget);
    }
    fs::create_dir(&target)?;
    canonical_directory(&target).map_err(|_| StorageRootError::InvalidTarget)
}

fn validate_pending_target(
    source: &Path,
    target: &Path,
    config_root: &Path,
    install_root: &Path,
    migration_id: Uuid,
) -> Result<(), StorageRootError> {
    if target == source
        || paths_overlap(target, source)
        || target.starts_with(install_root)
        || is_unsupported_windows_path(target)
    {
        return Err(StorageRootError::InvalidTarget);
    }
    if target == config_root {
        validate_default_target_entries(target)?;
    } else {
        let marker_matches =
            read_json_optional::<StorageRootMarker>(&target.join(ROOT_MARKER_FILE))?.is_some_and(
                |marker| marker.version == STORAGE_VERSION && marker.migration_id == migration_id,
            );
        if marker_matches {
            validate_managed_target_entries(target, true)?;
        } else if fs::read_dir(target)?.next().transpose()?.is_some() {
            return Err(StorageRootError::TargetNotEmpty);
        }
    }
    Ok(())
}

fn validate_managed_target_entries(
    target: &Path,
    allow_marker: bool,
) -> Result<(), StorageRootError> {
    let metadata = fs::symlink_metadata(target)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(StorageRootError::InvalidTarget);
    }
    for entry in fs::read_dir(target)? {
        let entry = entry?;
        if fs::symlink_metadata(entry.path())?.file_type().is_symlink() {
            return Err(StorageRootError::InvalidTarget);
        }
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            return Err(StorageRootError::InvalidTarget);
        };
        if !(MANAGED_ENTRIES.contains(&name) || allow_marker && name == ROOT_MARKER_FILE) {
            return Err(StorageRootError::InvalidTarget);
        }
    }
    Ok(())
}

fn require_real_directory(path: &Path) -> Result<(), StorageRootError> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(StorageRootError::InvalidTarget);
    }
    Ok(())
}

fn display_path(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

pub(crate) fn write_json_atomic<T: Serialize>(
    path: &Path,
    value: &T,
) -> Result<(), StorageRootError> {
    let parent = path
        .parent()
        .ok_or(StorageRootError::InvalidConfiguration)?;
    fs::create_dir_all(parent)?;
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or(StorageRootError::InvalidConfiguration)?;
    let temporary = parent.join(format!(".{file_name}.tmp-{}", Uuid::new_v4()));
    let backup = path.with_extension(format!(
        "{}bak",
        path.extension()
            .and_then(|value| value.to_str())
            .map(|value| format!("{value}."))
            .unwrap_or_default()
    ));
    let bytes = serde_json::to_vec_pretty(value)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    file.write_all(&bytes)?;
    file.write_all(b"\n")?;
    file.sync_all()?;

    if backup.exists() {
        fs::remove_file(&backup)?;
    }
    let had_existing = path.exists();
    if had_existing {
        fs::rename(path, &backup)?;
    }
    if let Err(error) = fs::rename(&temporary, path) {
        if had_existing && backup.exists() {
            let _ = fs::rename(&backup, path);
        }
        let _ = fs::remove_file(&temporary);
        return Err(StorageRootError::Io(error));
    }
    sync_directory(parent)?;
    if backup.exists() {
        fs::remove_file(backup)?;
    }
    Ok(())
}

pub(crate) fn recover_atomic_file(path: &Path) -> Result<(), StorageRootError> {
    let backup = path.with_extension(format!(
        "{}bak",
        path.extension()
            .and_then(|value| value.to_str())
            .map(|value| format!("{value}."))
            .unwrap_or_default()
    ));
    if !path.exists() && backup.exists() {
        fs::rename(backup, path)?;
    }
    Ok(())
}

pub(crate) fn remove_atomic_file(path: &Path) -> Result<(), StorageRootError> {
    if path.exists() {
        fs::remove_file(path)?;
    }
    let backup = path.with_extension(format!(
        "{}bak",
        path.extension()
            .and_then(|value| value.to_str())
            .map(|value| format!("{value}."))
            .unwrap_or_default()
    ));
    if backup.exists() {
        fs::remove_file(backup)?;
    }
    Ok(())
}

pub(crate) fn read_json_optional<T: DeserializeOwned>(
    path: &Path,
) -> Result<Option<T>, StorageRootError> {
    if !path.exists() {
        return Ok(None);
    }
    Ok(Some(read_json_required(path)?))
}

fn read_json_required<T: DeserializeOwned>(path: &Path) -> Result<T, StorageRootError> {
    let bytes = fs::read(path)?;
    serde_json::from_slice(&bytes).map_err(StorageRootError::Metadata)
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), StorageRootError> {
    File::open(path)?.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<(), StorageRootError> {
    Ok(())
}

fn open_directory(path: &Path) -> Result<(), StorageRootError> {
    #[cfg(target_os = "windows")]
    let result = Command::new("explorer.exe").arg(path).spawn();
    #[cfg(target_os = "macos")]
    let result = Command::new("open").arg(path).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let result = Command::new("xdg-open").arg(path).spawn();
    result.map(|_| ()).map_err(|_| StorageRootError::OpenFailed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::Executor;
    use tempfile::TempDir;

    async fn create_source(root: &Path) {
        let database = root.join("muriarc.sqlite3");
        let options = SqliteConnectOptions::new()
            .filename(&database)
            .create_if_missing(true);
        let mut connection = SqliteConnection::connect_with(&options).await.unwrap();
        connection
            .execute("CREATE TABLE records (id INTEGER PRIMARY KEY, value TEXT NOT NULL)")
            .await
            .unwrap();
        connection
            .execute("INSERT INTO records (value) VALUES ('preserved')")
            .await
            .unwrap();
        connection.close().await.unwrap();
        fs::create_dir(root.join("attachments")).unwrap();
        fs::write(root.join("attachments").join("object.bin"), b"attachment").unwrap();
        fs::create_dir(root.join("data")).unwrap();
        fs::write(root.join("data").join("artifact.bin"), b"artifact").unwrap();
        fs::write(root.join("ai-provider.json"), br#"{"enabled":true}"#).unwrap();
        fs::write(
            root.join("deployment-generation.json"),
            br#"{"generationId":"fixture"}"#,
        )
        .unwrap();
    }

    fn roots() -> (TempDir, PathBuf, PathBuf, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let config = temp.path().join("config");
        let install = temp.path().join("install");
        let target = temp.path().join("target");
        fs::create_dir_all(&config).unwrap();
        fs::create_dir_all(&install).unwrap();
        fs::create_dir_all(&target).unwrap();
        (temp, config, install, target)
    }

    #[tokio::test]
    async fn defaults_to_the_config_root_without_creating_a_locator() {
        let (_temp, config, install, _target) = roots();
        let state = StorageRootState::initialize(&config, &install)
            .await
            .unwrap();
        assert_eq!(state.active_root(), fs::canonicalize(&config).unwrap());
        assert!(!config.join(LOCATOR_FILE).exists());
        assert!(!state.status().unwrap().uses_custom_root);
    }

    #[tokio::test]
    async fn migrates_the_complete_managed_tree_before_switching_the_locator() {
        let (_temp, config, install, target) = roots();
        create_source(&config).await;
        let state = StorageRootState::initialize(&config, &install)
            .await
            .unwrap();
        let selection = state.select_target(&target).unwrap();
        let scheduled = state.schedule_selected(&selection.selection_token).unwrap();
        assert!(scheduled.requires_restart);
        assert!(!config.join(LOCATOR_FILE).exists());

        let restarted = StorageRootState::initialize(&config, &install)
            .await
            .unwrap();
        assert_eq!(restarted.active_root(), fs::canonicalize(&target).unwrap());
        assert!(target.join("muriarc.sqlite3").is_file());
        assert_eq!(
            fs::read(target.join("attachments").join("object.bin")).unwrap(),
            b"attachment"
        );
        assert_eq!(
            fs::read(target.join("data").join("artifact.bin")).unwrap(),
            b"artifact"
        );
        assert_eq!(
            fs::read(target.join("deployment-generation.json")).unwrap(),
            br#"{"generationId":"fixture"}"#
        );
        assert!(config.join("muriarc.sqlite3").is_file());
        assert!(!config.join(MIGRATION_FILE).exists());
    }

    #[tokio::test]
    async fn refuses_non_empty_unknown_targets_without_writing_an_intent() {
        let (_temp, config, install, target) = roots();
        create_source(&config).await;
        fs::write(target.join("unrelated.txt"), b"owner data").unwrap();
        let state = StorageRootState::initialize(&config, &install)
            .await
            .unwrap();
        let error = state.select_target(&target).unwrap_err();
        assert!(matches!(error, StorageRootError::TargetNotEmpty));
        assert!(!config.join(MIGRATION_FILE).exists());
    }

    #[tokio::test]
    async fn refuses_the_install_tree_and_active_root_descendants() {
        let (_temp, config, install, _target) = roots();
        create_source(&config).await;
        let install_target = install.join("data");
        let active_child = config.join("nested");
        fs::create_dir(&install_target).unwrap();
        fs::create_dir(&active_child).unwrap();
        let state = StorageRootState::initialize(&config, &install)
            .await
            .unwrap();

        assert!(matches!(
            state.select_target(&install_target).unwrap_err(),
            StorageRootError::InvalidTarget
        ));
        assert!(matches!(
            state.select_target(&active_child).unwrap_err(),
            StorageRootError::InvalidTarget
        ));
        assert!(!config.join(MIGRATION_FILE).exists());
    }

    #[tokio::test]
    async fn missing_custom_root_fails_closed_instead_of_creating_an_empty_database() {
        let (temp, config, install, _target) = roots();
        let missing = temp.path().join("missing");
        write_json_atomic(
            &config.join(LOCATOR_FILE),
            &StorageLocator {
                version: STORAGE_VERSION,
                active_data_root: missing.clone(),
            },
        )
        .unwrap();
        let error = StorageRootState::initialize(&config, &install)
            .await
            .unwrap_err();
        assert!(matches!(error, StorageRootError::ActiveRootUnavailable));
        assert!(!missing.exists());
    }

    #[tokio::test]
    async fn restores_to_default_while_preserving_the_inactive_default_copy() {
        let (_temp, config, install, target) = roots();
        create_source(&config).await;
        let state = StorageRootState::initialize(&config, &install)
            .await
            .unwrap();
        let selection = state.select_target(&target).unwrap();
        state.schedule_selected(&selection.selection_token).unwrap();
        let custom = StorageRootState::initialize(&config, &install)
            .await
            .unwrap();
        fs::write(target.join("attachments").join("new.bin"), b"new data").unwrap();
        custom.schedule_default().unwrap();

        let restored = StorageRootState::initialize(&config, &install)
            .await
            .unwrap();
        assert_eq!(restored.active_root(), fs::canonicalize(&config).unwrap());
        assert_eq!(
            fs::read(config.join("attachments").join("new.bin")).unwrap(),
            b"new data"
        );
        let backup_count = fs::read_dir(config.join(BACKUP_DIRECTORY)).unwrap().count();
        assert_eq!(backup_count, 1);
        assert!(target.join("muriarc.sqlite3").is_file());
    }

    #[tokio::test]
    async fn resumes_a_verified_staging_copy_when_the_custom_target_disappeared() {
        let (_temp, config, install, target) = roots();
        create_source(&config).await;
        let state = StorageRootState::initialize(&config, &install)
            .await
            .unwrap();
        let selection = state.select_target(&target).unwrap();
        state.schedule_selected(&selection.selection_token).unwrap();
        let intent: StorageMigrationIntent =
            read_json_required(&config.join(MIGRATION_FILE)).unwrap();

        checkpoint_and_verify_database(&config.join("muriarc.sqlite3"))
            .await
            .unwrap();
        let manifest = tree_digest(&config).unwrap();
        let staging = staging_path(&target, intent.migration_id).unwrap();
        fs::create_dir(&staging).unwrap();
        copy_managed_tree(&config, &staging).unwrap();
        write_json_atomic(
            &staging.join(ROOT_MARKER_FILE),
            &StorageRootMarker {
                version: STORAGE_VERSION,
                migration_id: intent.migration_id,
                manifest,
                created_at: Utc::now().to_rfc3339(),
            },
        )
        .unwrap();
        fs::remove_dir(&target).unwrap();

        let restarted = StorageRootState::initialize(&config, &install)
            .await
            .unwrap();
        assert_eq!(restarted.active_root(), fs::canonicalize(&target).unwrap());
        assert!(target.join("muriarc.sqlite3").is_file());
        assert!(!staging.exists());
    }

    #[test]
    fn resumes_default_finalization_after_a_partial_backup() {
        let temp = tempfile::tempdir().unwrap();
        let default = temp.path().join("default");
        let staging = temp.path().join("staging");
        fs::create_dir(&default).unwrap();
        fs::create_dir(&staging).unwrap();
        fs::write(default.join("muriarc.sqlite3"), b"old database").unwrap();
        fs::create_dir(default.join("attachments")).unwrap();
        fs::write(default.join("attachments").join("old.bin"), b"old").unwrap();
        fs::write(staging.join("muriarc.sqlite3"), b"new database").unwrap();
        fs::create_dir(staging.join("attachments")).unwrap();
        fs::write(staging.join("attachments").join("new.bin"), b"new").unwrap();
        fs::write(staging.join(ROOT_MARKER_FILE), b"marker").unwrap();

        let migration_id = Uuid::new_v4();
        let backup = default
            .join(BACKUP_DIRECTORY)
            .join(migration_id.to_string());
        fs::create_dir_all(&backup).unwrap();
        let journal = backup.join("finalize.json");
        write_json_atomic(
            &journal,
            &DefaultFinalizeState {
                version: STORAGE_VERSION,
                migration_id,
                phase: DefaultFinalizePhase::BackingUp,
                old_entries: vec!["muriarc.sqlite3".to_owned(), "attachments".to_owned()],
            },
        )
        .unwrap();
        fs::rename(&journal, journal.with_extension("json.bak")).unwrap();
        fs::rename(
            default.join("muriarc.sqlite3"),
            backup.join("muriarc.sqlite3"),
        )
        .unwrap();

        finalize_default_target(&default, &staging, migration_id).unwrap();

        assert_eq!(
            fs::read(default.join("muriarc.sqlite3")).unwrap(),
            b"new database"
        );
        assert!(default.join("attachments").join("new.bin").is_file());
        assert_eq!(
            fs::read(backup.join("muriarc.sqlite3")).unwrap(),
            b"old database"
        );
        assert!(backup.join("attachments").join("old.bin").is_file());
    }

    #[test]
    fn resumes_default_finalization_after_a_partial_install() {
        let temp = tempfile::tempdir().unwrap();
        let default = temp.path().join("default");
        let staging = temp.path().join("staging");
        fs::create_dir(&default).unwrap();
        fs::create_dir(&staging).unwrap();
        fs::write(staging.join("muriarc.sqlite3"), b"new database").unwrap();
        fs::create_dir(staging.join("data")).unwrap();
        fs::write(staging.join("data").join("new.bin"), b"new").unwrap();
        fs::write(staging.join(ROOT_MARKER_FILE), b"marker").unwrap();

        let migration_id = Uuid::new_v4();
        let backup = default
            .join(BACKUP_DIRECTORY)
            .join(migration_id.to_string());
        fs::create_dir_all(&backup).unwrap();
        fs::write(backup.join("muriarc.sqlite3"), b"old database").unwrap();
        write_json_atomic(
            &backup.join("finalize.json"),
            &DefaultFinalizeState {
                version: STORAGE_VERSION,
                migration_id,
                phase: DefaultFinalizePhase::Installing,
                old_entries: vec!["muriarc.sqlite3".to_owned()],
            },
        )
        .unwrap();
        fs::rename(
            staging.join("muriarc.sqlite3"),
            default.join("muriarc.sqlite3"),
        )
        .unwrap();

        finalize_default_target(&default, &staging, migration_id).unwrap();

        assert_eq!(
            fs::read(default.join("muriarc.sqlite3")).unwrap(),
            b"new database"
        );
        assert!(default.join("data").join("new.bin").is_file());
        assert!(default.join(ROOT_MARKER_FILE).is_file());
        assert!(!staging.exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn refuses_symbolic_links_in_the_managed_tree() {
        use std::os::unix::fs::symlink;

        let (_temp, config, install, target) = roots();
        create_source(&config).await;
        fs::remove_file(config.join("attachments").join("object.bin")).unwrap();
        symlink(
            config.join("muriarc.sqlite3"),
            config.join("attachments").join("object.bin"),
        )
        .unwrap();
        let state = StorageRootState::initialize(&config, &install)
            .await
            .unwrap();
        let selection = state.select_target(&target).unwrap();
        state.schedule_selected(&selection.selection_token).unwrap();
        let error = StorageRootState::initialize(&config, &install)
            .await
            .unwrap_err();
        assert!(matches!(error, StorageRootError::InvalidTarget));
        assert!(!config.join(LOCATOR_FILE).exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn refuses_a_symbolic_link_as_the_selected_root() {
        use std::os::unix::fs::symlink;

        let (temp, config, install, target) = roots();
        create_source(&config).await;
        let linked = temp.path().join("linked-target");
        symlink(&target, &linked).unwrap();
        let state = StorageRootState::initialize(&config, &install)
            .await
            .unwrap();

        assert!(matches!(
            state.select_target(&linked).unwrap_err(),
            StorageRootError::InvalidTarget
        ));
        assert!(!config.join(MIGRATION_FILE).exists());
    }
}
