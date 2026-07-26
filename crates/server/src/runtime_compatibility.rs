use std::{
    env,
    error::Error,
    fs::{self, OpenOptions},
    io::{ErrorKind, Write},
    path::Path,
};

use muriarc_core::{
    DeploymentGenerationManifest, DeploymentState, MuriArcStore, PersistentRecoveryInventory,
};
use muriarc_server::RuntimeAccessMode;
use muriarc_store_postgres::PostgresStore;
use uuid::Uuid;

const GENERATION_MANIFEST_FILE: &str = "deployment-generation.json";

pub(crate) struct ServerRuntimeCompatibility {
    #[allow(dead_code)]
    pub deployment_state: DeploymentState,
    pub recovery_inventory: PersistentRecoveryInventory,
    pub access_mode: RuntimeAccessMode,
}

pub(crate) async fn prepare_server_runtime(
    store: &PostgresStore,
    data_root: &Path,
    attachment_root: &Path,
) -> Result<ServerRuntimeCompatibility, Box<dyn Error>> {
    let preview_bootstrap = preview_bootstrap_enabled()?;
    let access_mode = runtime_access_mode()?;
    if preview_bootstrap && access_mode == RuntimeAccessMode::ReadOnlyActivation {
        return Err("preview bootstrap cannot run in read-only activation mode".into());
    }
    if preview_bootstrap {
        tracing::warn!(
            "explicit preview_epoch_0 bootstrap enabled; this escape hatch is not a stable-version upgrade path"
        );
        store.migrate().await?;
    }

    let recovery_inventory = store.persistent_recovery_inventory().await?;
    let has_persisted_database_data = recovery_inventory.attachment_records != 0
        || recovery_inventory.encrypted_secret_records != 0
        || recovery_inventory.ai_history_records != 0
        || recovery_inventory.audit_records != 0;
    ensure_directory(
        data_root,
        preview_bootstrap && !has_persisted_database_data,
        has_persisted_database_data,
        "data root",
    )?;
    ensure_directory(
        attachment_root,
        preview_bootstrap && recovery_inventory.attachment_records == 0,
        recovery_inventory.attachment_records != 0,
        "attachment root",
    )?;
    if recovery_inventory.attachment_records != 0 && directory_is_empty(attachment_root)? {
        return Err("attachment metadata exists but the attachment root is empty".into());
    }

    let report = store.compatibility_report().await?;
    let deployment_state = if access_mode == RuntimeAccessMode::ReadWrite && report.is_compatible()
    {
        report
            .observed
            .clone()
            .ok_or("compatible report did not contain deployment state")?
    } else if access_mode == RuntimeAccessMode::ReadOnlyActivation {
        report
            .require_read_only_compatible()
            .map_err(|error| -> Box<dyn Error> { error.into() })?
            .clone()
    } else if preview_bootstrap
        && report.issues.len() == 1
        && report.issues[0].code == "deployment_state_missing"
    {
        store.adopt_current_release(Uuid::new_v4()).await?
    } else {
        return Err(format!(
            "runtime compatibility verification failed: {}",
            report
                .issues
                .iter()
                .map(|issue| format!("{} ({})", issue.code, issue.detail))
                .collect::<Vec<_>>()
                .join("; ")
        )
        .into());
    };

    verify_or_create_generation_manifest(data_root, &deployment_state, preview_bootstrap)?;
    Ok(ServerRuntimeCompatibility {
        deployment_state,
        recovery_inventory,
        access_mode,
    })
}

fn runtime_access_mode() -> Result<RuntimeAccessMode, Box<dyn Error>> {
    match env::var("MURIARC_ACTIVATION_MODE") {
        Err(env::VarError::NotPresent) => Ok(RuntimeAccessMode::ReadWrite),
        Err(error) => Err(error.into()),
        Ok(value) => parse_runtime_access_mode(&value).map_err(Into::into),
    }
}

fn parse_runtime_access_mode(value: &str) -> Result<RuntimeAccessMode, &'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "" | "read-write" => Ok(RuntimeAccessMode::ReadWrite),
        "read-only" => Ok(RuntimeAccessMode::ReadOnlyActivation),
        _ => Err("MURIARC_ACTIVATION_MODE must be read-write or read-only"),
    }
}

fn preview_bootstrap_enabled() -> Result<bool, Box<dyn Error>> {
    match env::var("MURIARC_PREVIEW_BOOTSTRAP") {
        Err(env::VarError::NotPresent) => Ok(false),
        Err(error) => Err(error.into()),
        Ok(value) => match value.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" => Ok(true),
            "0" | "false" | "no" | "" => Ok(false),
            _ => Err("MURIARC_PREVIEW_BOOTSTRAP must be true or false".into()),
        },
    }
}

fn ensure_directory(
    path: &Path,
    allow_create: bool,
    missing_is_data_loss: bool,
    label: &str,
) -> Result<(), Box<dyn Error>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() => {
            Ok(())
        }
        Ok(_) => Err(format!("{label} is not a real directory: {}", path.display()).into()),
        Err(error) if error.kind() == ErrorKind::NotFound && allow_create => {
            fs::create_dir_all(path)?;
            Ok(())
        }
        Err(error) if error.kind() == ErrorKind::NotFound && missing_is_data_loss => Err(format!(
            "{label} is missing while the database contains persistent data: {}",
            path.display()
        )
        .into()),
        Err(error) if error.kind() == ErrorKind::NotFound => Err(format!(
            "{label} is missing; install or the upgrade controller must create it: {}",
            path.display()
        )
        .into()),
        Err(error) => Err(error.into()),
    }
}

fn directory_is_empty(path: &Path) -> Result<bool, Box<dyn Error>> {
    Ok(fs::read_dir(path)?.next().transpose()?.is_none())
}

fn verify_or_create_generation_manifest(
    data_root: &Path,
    state: &DeploymentState,
    allow_create: bool,
) -> Result<(), Box<dyn Error>> {
    let path = data_root.join(GENERATION_MANIFEST_FILE);
    match fs::read(&path) {
        Ok(bytes) => {
            let manifest: DeploymentGenerationManifest = serde_json::from_slice(&bytes)?;
            manifest
                .validate(state)
                .map_err(|issue| format!("{}: {}", issue.code, issue.detail).into())
        }
        Err(error) if error.kind() == ErrorKind::NotFound && allow_create => {
            write_manifest_atomic(&path, &DeploymentGenerationManifest::from_state(state))
        }
        Err(error) if error.kind() == ErrorKind::NotFound => Err(format!(
            "generation manifest is missing: {}; refusing to guess the data generation",
            path.display()
        )
        .into()),
        Err(error) => Err(error.into()),
    }
}

fn write_manifest_atomic(
    path: &Path,
    manifest: &DeploymentGenerationManifest,
) -> Result<(), Box<dyn Error>> {
    let temporary = path.with_extension(format!("tmp-{}", Uuid::new_v4()));
    let bytes = serde_json::to_vec_pretty(manifest)?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary)?;
    file.write_all(&bytes)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    match fs::rename(&temporary, path) {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = fs::remove_file(&temporary);
            Err(error.into())
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use muriarc_core::{BackendKind, ReleaseIdentity};

    use super::*;

    fn state() -> DeploymentState {
        DeploymentState {
            identity: ReleaseIdentity::parse(
                "0.1.0".to_owned(),
                "preview_epoch_0".to_owned(),
                format!("sha256:{}", "0".repeat(64)),
                "gateway-v1".to_owned(),
            )
            .unwrap(),
            generation_id: Uuid::new_v4(),
            write_lease_id: Some(Uuid::new_v4()),
            first_write_at: None,
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn generation_manifest_never_self_heals_without_bootstrap() {
        let root = tempfile::tempdir().unwrap();
        let error = verify_or_create_generation_manifest(root.path(), &state(), false)
            .unwrap_err()
            .to_string();
        assert!(error.contains("refusing to guess"));
    }

    #[test]
    fn generation_manifest_detects_mismatch() {
        let root = tempfile::tempdir().unwrap();
        let first = state();
        verify_or_create_generation_manifest(root.path(), &first, true).unwrap();
        let second = state();
        let error = verify_or_create_generation_manifest(root.path(), &second, false)
            .unwrap_err()
            .to_string();
        assert!(error.contains("generation_manifest_mismatch"));
        let _ = BackendKind::Postgres;
    }

    #[test]
    fn activation_mode_is_explicit_and_fail_closed() {
        assert_eq!(
            parse_runtime_access_mode("read-only").unwrap(),
            RuntimeAccessMode::ReadOnlyActivation
        );
        assert!(parse_runtime_access_mode("maintenance-ish").is_err());
    }
}
