use std::{collections::BTreeSet, path::PathBuf};

use muriarc_upgrade::DeploymentProfile;
use serde::{Deserialize, Serialize};

use crate::DeliveryError;

pub const DELIVERY_CONFIG_FORMAT: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeliveryPaths {
    pub release_root: PathBuf,
    pub current_release: PathBuf,
    pub config_root: PathBuf,
    pub data_root: PathBuf,
    pub control_root: PathBuf,
}

impl DeliveryPaths {
    pub fn native_system() -> Self {
        Self {
            release_root: "/opt/muriarc/releases".into(),
            current_release: "/opt/muriarc/current".into(),
            config_root: "/etc/muriarc".into(),
            data_root: "/var/lib/muriarc".into(),
            control_root: "/var/lib/muriarc/control".into(),
        }
    }

    pub fn managed_compose(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        Self {
            release_root: root.join("releases"),
            current_release: root.join("current"),
            config_root: root.join("config"),
            data_root: root.join("data"),
            control_root: root.join("control"),
        }
    }

    pub fn validate(&self, profile: DeploymentProfile) -> Result<(), DeliveryError> {
        let values = [
            &self.release_root,
            &self.current_release,
            &self.config_root,
            &self.data_root,
            &self.control_root,
        ];
        if values.iter().any(|path| !path.is_absolute())
            || values
                .iter()
                .enumerate()
                .any(|(index, path)| values.iter().skip(index + 1).any(|other| path == other))
        {
            return Err(DeliveryError::InvalidPolicy(
                "delivery paths must be distinct absolute paths".to_owned(),
            ));
        }
        if profile == DeploymentProfile::NativeSystem && self != &Self::native_system() {
            return Err(DeliveryError::InvalidPolicy(
                "native-system uses the fixed /opt, /etc, and /var/lib layout".to_owned(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeliveryConfig {
    pub format_version: u32,
    pub profile: DeploymentProfile,
    pub paths: DeliveryPaths,
    pub service_user: String,
    pub loopback_origin: String,
    pub environment_file: PathBuf,
    pub activation_file: PathBuf,
    pub compose_project: Option<String>,
    pub compose_file: Option<PathBuf>,
}

impl DeliveryConfig {
    pub fn validate(&self) -> Result<(), DeliveryError> {
        if self.format_version != DELIVERY_CONFIG_FORMAT
            || self.service_user != "muriarc"
            || self.loopback_origin != "http://127.0.0.1:8787"
        {
            return Err(DeliveryError::InvalidPolicy(
                "delivery identity, service user, or loopback origin is invalid".to_owned(),
            ));
        }
        self.paths.validate(self.profile)?;
        if !self.environment_file.is_absolute()
            || !self.environment_file.starts_with(&self.paths.config_root)
            || !self.activation_file.is_absolute()
            || !self.activation_file.starts_with(&self.paths.control_root)
        {
            return Err(DeliveryError::InvalidPolicy(
                "delivery environment file must be absolute and inside config root".to_owned(),
            ));
        }
        match self.profile {
            DeploymentProfile::NativeSystem => {
                if self.compose_project.is_some()
                    || self.compose_file.is_some()
                    || self.environment_file != PathBuf::from("/etc/muriarc/server.env")
                    || self.activation_file != PathBuf::from("/var/lib/muriarc/control/active.env")
                {
                    return Err(DeliveryError::InvalidPolicy(
                        "native-system cannot carry Compose settings".to_owned(),
                    ));
                }
            }
            DeploymentProfile::ManagedCompose => {
                let project = self.compose_project.as_deref().unwrap_or_default();
                if project.is_empty()
                    || !project.bytes().all(|byte| {
                        byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'
                    })
                    || self
                        .compose_file
                        .as_ref()
                        .is_none_or(|path| !path.is_absolute())
                {
                    return Err(DeliveryError::InvalidPolicy(
                        "managed-compose needs a safe project and absolute Compose file".to_owned(),
                    ));
                }
            }
            DeploymentProfile::Desktop => {
                return Err(DeliveryError::InvalidPolicy(
                    "Desktop uses its dedicated updater Driver".to_owned(),
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeliveryCapabilities {
    pub service_control: bool,
    pub postgres_major: Option<u16>,
    pub backup_restore: bool,
    pub isolated_candidate_database: bool,
    pub isolated_candidate_storage: bool,
    pub ddl_executor: bool,
    pub verifier: bool,
    pub bundle_signature_verified: bool,
    pub unavailable_reasons: BTreeSet<String>,
}

impl DeliveryCapabilities {
    pub fn require_upgrade_ready(&self) -> Result<(), DeliveryError> {
        if self.service_control
            && self.postgres_major.is_some_and(|major| major >= 17)
            && self.backup_restore
            && self.isolated_candidate_database
            && self.isolated_candidate_storage
            && self.ddl_executor
            && self.verifier
            && self.bundle_signature_verified
            && self.unavailable_reasons.is_empty()
        {
            Ok(())
        } else {
            Err(DeliveryError::Prerequisite(
                "backup restore, isolated Candidate, PostgreSQL 17, signed bundle, DDL executor, verifier, and service control are all mandatory".to_owned(),
            ))
        }
    }
}

pub fn validate_digest_pinned_image(reference: &str) -> Result<(), DeliveryError> {
    let Some((repository, digest)) = reference.rsplit_once("@sha256:") else {
        return Err(DeliveryError::InvalidPolicy(
            "image must be pinned with @sha256".to_owned(),
        ));
    };
    if !repository.starts_with("ghcr.io/")
        || repository.contains(":latest")
        || digest.len() != 64
        || !digest.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(DeliveryError::InvalidPolicy(
            "image must be a digest-pinned GHCR reference".to_owned(),
        ));
    }
    Ok(())
}

pub fn validate_compose_policy(rendered: &str) -> Result<(), DeliveryError> {
    let lowercase = rendered.to_ascii_lowercase();
    for forbidden in [
        "build:",
        ":latest",
        "watchtower",
        "/var/run/docker.sock",
        "0.0.0.0:8787:8787",
        "5432:5432",
    ] {
        if lowercase.contains(forbidden) {
            return Err(DeliveryError::InvalidPolicy(format!(
                "managed Compose contains forbidden token {forbidden}"
            )));
        }
    }
    if !rendered.contains("127.0.0.1:8787:8787")
        || !rendered.contains("no-new-privileges:true")
        || !rendered.contains("cap_drop:")
        || !rendered.contains("read_only: true")
    {
        return Err(DeliveryError::InvalidPolicy(
            "managed Compose misses loopback or low-privilege controls".to_owned(),
        ));
    }
    Ok(())
}
