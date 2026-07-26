use std::{
    env,
    error::Error,
    fs::{self, OpenOptions},
    io::{ErrorKind, Write},
    path::Path,
};

use muriarc_core::{DeploymentGenerationManifest, DeploymentState, MuriArcStore};
use muriarc_store_sqlite::SqliteStore;
use uuid::Uuid;

const GENERATION_MANIFEST_FILE: &str = "deployment-generation.json";

pub(crate) async fn prepare_desktop_runtime(
    database_path: &Path,
    active_data_root: &Path,
) -> Result<DeploymentState, Box<dyn Error>> {
    let database_existed = fs::symlink_metadata(database_path).is_ok();
    let preview_bootstrap = preview_bootstrap_enabled()?;
    let fresh_install = !database_existed && fresh_data_root(active_data_root)?;
    let store = SqliteStore::connect_path(database_path).await?;
    if fresh_install || preview_bootstrap {
        if preview_bootstrap && !fresh_install {
            eprintln!(
                "MuriArc: explicit preview_epoch_0 desktop bootstrap enabled; stable upgrades must use the signed updater"
            );
        }
        store.migrate().await?;
    }

    let inventory = store.persistent_recovery_inventory().await?;
    let attachment_root = active_data_root.join("attachments");
    match fs::symlink_metadata(&attachment_root) {
        Ok(metadata) if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() => {}
        Ok(_) => return Err("desktop attachment root is not a real directory".into()),
        Err(error) if error.kind() == ErrorKind::NotFound && inventory.attachment_records == 0 => {
            fs::create_dir_all(&attachment_root)?;
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Err(
                "desktop attachment metadata exists but the attachment root is missing".into(),
            );
        }
        Err(error) => return Err(error.into()),
    }
    if inventory.attachment_records != 0
        && fs::read_dir(&attachment_root)?
            .next()
            .transpose()?
            .is_none()
    {
        return Err("desktop attachment metadata exists but the attachment root is empty".into());
    }

    let report = store.compatibility_report().await?;
    let state = if report.is_compatible() {
        report
            .observed
            .ok_or("compatible report did not contain deployment state")?
    } else if (fresh_install || preview_bootstrap)
        && report.issues.len() == 1
        && report.issues[0].code == "deployment_state_missing"
    {
        store.adopt_current_release(Uuid::new_v4()).await?
    } else {
        return Err(format!(
            "desktop runtime compatibility verification failed: {}",
            report
                .issues
                .iter()
                .map(|issue| format!("{} ({})", issue.code, issue.detail))
                .collect::<Vec<_>>()
                .join("; ")
        )
        .into());
    };
    verify_or_create_manifest(active_data_root, &state, fresh_install || preview_bootstrap)?;
    Ok(state)
}

pub(crate) fn preview_bootstrap_enabled() -> Result<bool, Box<dyn Error>> {
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

fn fresh_data_root(root: &Path) -> Result<bool, Box<dyn Error>> {
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let name = entry.file_name();
        if !matches!(
            name.to_str(),
            Some("storage-location.json" | "storage-migration.json")
        ) {
            return Ok(false);
        }
    }
    Ok(true)
}

fn verify_or_create_manifest(
    root: &Path,
    state: &DeploymentState,
    allow_create: bool,
) -> Result<(), Box<dyn Error>> {
    let path = root.join(GENERATION_MANIFEST_FILE);
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
            "desktop generation manifest is missing: {}; refusing to create an empty replacement",
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
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    let mut file = options.open(&temporary)?;
    file.write_all(&serde_json::to_vec_pretty(manifest)?)?;
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
