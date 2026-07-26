use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use chrono::{DateTime, Utc};
use muriarc_core::ApplicationVersion;
use muriarc_upgrade::DeploymentProfile;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    DeliveryConfig, DeliveryError, VerifiedServerBundle, activate_release_link,
    stage_verified_release, verify_server_bundle,
};

pub const INSTALL_RECEIPT_FORMAT: u32 = 1;
pub const INSTALL_RECEIPT_FILE: &str = "install-receipt.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallReceipt {
    pub format_version: u32,
    pub profile: DeploymentProfile,
    pub application_version: ApplicationVersion,
    pub manifest_digest: String,
    pub content_digest: String,
    pub release_path: PathBuf,
    pub installed_at: DateTime<Utc>,
}

impl InstallReceipt {
    pub fn validate(&self, config: &DeliveryConfig) -> Result<(), DeliveryError> {
        config.validate()?;
        let expected_release = config
            .paths
            .release_root
            .join(self.application_version.as_str());
        if self.format_version != INSTALL_RECEIPT_FORMAT
            || self.profile != config.profile
            || self.release_path != expected_release
            || !self.release_path.is_absolute()
            || !self.manifest_digest.starts_with("sha256:")
            || !self.content_digest.starts_with("sha256:")
        {
            return Err(DeliveryError::InvalidPolicy(
                "install receipt differs from the configured immutable release".to_owned(),
            ));
        }
        Ok(())
    }
}

/// Verifies a signed bundle digest, stages an immutable release, atomically
/// activates it and writes a root-owned receipt. Platform service registration
/// is deliberately separate so a failed systemd/Docker prerequisite cannot be
/// hidden inside a successful bundle copy.
#[cfg(unix)]
pub fn install_server_bundle(
    bundle_root: &Path,
    trusted_manifest_digest: &str,
    config: &DeliveryConfig,
) -> Result<(InstallReceipt, VerifiedServerBundle), DeliveryError> {
    if trusted_manifest_digest.trim().is_empty() {
        return Err(DeliveryError::Prerequisite(
            "a bundle manifest digest from verified signed metadata is mandatory".to_owned(),
        ));
    }
    config.validate()?;
    let (manifest, verified) = verify_server_bundle(bundle_root, Some(trusted_manifest_digest))?;
    if manifest.profile != config.profile {
        return Err(DeliveryError::InvalidPolicy(
            "bundle profile differs from install profile".to_owned(),
        ));
    }
    fs::create_dir_all(&config.paths.control_root).map_err(io)?;

    let release = match stage_verified_release(bundle_root, &manifest, &config.paths.release_root) {
        Ok(release) => release,
        Err(DeliveryError::AlreadyInstalled(release)) => {
            let (_, installed) = verify_server_bundle(&release, Some(trusted_manifest_digest))?;
            if installed != verified {
                return Err(DeliveryError::InvalidBundle(
                    "existing immutable release differs from the requested bundle".to_owned(),
                ));
            }
            release
        }
        Err(error) => return Err(error),
    };

    let previous = fs::read_link(&config.paths.current_release).ok();
    activate_release_link(&release, &config.paths.current_release)?;
    let receipt = InstallReceipt {
        format_version: INSTALL_RECEIPT_FORMAT,
        profile: config.profile,
        application_version: manifest.application_version,
        manifest_digest: verified.manifest_digest.clone(),
        content_digest: verified.content_digest.clone(),
        release_path: release,
        installed_at: Utc::now(),
    };
    if let Err(error) = write_install_state(config, &receipt) {
        let _ = rollback_activation(&config.paths.current_release, previous.as_deref());
        return Err(error);
    }
    Ok((receipt, verified))
}

#[cfg(not(unix))]
pub fn install_server_bundle(
    _bundle_root: &Path,
    _trusted_manifest_digest: &str,
    _config: &DeliveryConfig,
) -> Result<(InstallReceipt, VerifiedServerBundle), DeliveryError> {
    Err(DeliveryError::InvalidPolicy(
        "Server delivery profiles require Unix; Desktop uses its own updater".to_owned(),
    ))
}

pub fn load_install_state(
    config: &DeliveryConfig,
) -> Result<Option<InstallReceipt>, DeliveryError> {
    config.validate()?;
    let path = config.paths.control_root.join(INSTALL_RECEIPT_FILE);
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(io(error)),
    };
    let metadata = fs::symlink_metadata(&path).map_err(io)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(DeliveryError::InvalidPolicy(
            "install receipt must be a regular non-symlink file".to_owned(),
        ));
    }
    let receipt: InstallReceipt = serde_json::from_slice(&bytes)
        .map_err(|error| DeliveryError::Serialization(error.to_string()))?;
    receipt.validate(config)?;
    Ok(Some(receipt))
}

pub fn load_delivery_config(control_root: &Path) -> Result<DeliveryConfig, DeliveryError> {
    if !control_root.is_absolute() {
        return Err(DeliveryError::InvalidPolicy(
            "control root must be absolute".to_owned(),
        ));
    }
    let path = control_root.join("delivery.json");
    let metadata = fs::symlink_metadata(&path).map_err(io)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(DeliveryError::InvalidPolicy(
            "delivery config must be a regular non-symlink file".to_owned(),
        ));
    }
    let config: DeliveryConfig = serde_json::from_slice(&fs::read(path).map_err(io)?)
        .map_err(|error| DeliveryError::Serialization(error.to_string()))?;
    config.validate()?;
    if config.paths.control_root != control_root {
        return Err(DeliveryError::InvalidPolicy(
            "delivery config belongs to another control root".to_owned(),
        ));
    }
    Ok(config)
}

fn write_install_state(
    config: &DeliveryConfig,
    receipt: &InstallReceipt,
) -> Result<(), DeliveryError> {
    receipt.validate(config)?;
    let config_path = config.paths.control_root.join("delivery.json");
    write_json_atomic(&config_path, config, 0o600)?;
    let receipt_path = config.paths.control_root.join(INSTALL_RECEIPT_FILE);
    write_json_atomic(&receipt_path, receipt, 0o600)
}

fn write_json_atomic(
    path: &Path,
    value: &impl Serialize,
    #[allow(unused_variables)] mode: u32,
) -> Result<(), DeliveryError> {
    let parent = path.parent().ok_or_else(|| {
        DeliveryError::InvalidPolicy("state file has no parent directory".to_owned())
    })?;
    fs::create_dir_all(parent).map_err(io)?;
    let temporary = parent.join(format!(".{}.tmp-{}", file_name(path)?, Uuid::new_v4()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(mode);
    }
    let mut output = options.open(&temporary).map_err(io)?;
    let mut bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| DeliveryError::Serialization(error.to_string()))?;
    bytes.push(b'\n');
    output.write_all(&bytes).map_err(io)?;
    output.sync_all().map_err(io)?;
    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(io(error));
    }
    Ok(())
}

fn file_name(path: &Path) -> Result<&str, DeliveryError> {
    path.file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| DeliveryError::InvalidPolicy("state file name must be UTF-8".to_owned()))
}

#[cfg(unix)]
fn rollback_activation(current: &Path, previous: Option<&Path>) -> Result<(), DeliveryError> {
    match previous {
        Some(previous) => activate_release_link(previous, current),
        None => match fs::remove_file(current) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(io(error)),
        },
    }
}

fn io(error: std::io::Error) -> DeliveryError {
    DeliveryError::Io(error.to_string())
}
