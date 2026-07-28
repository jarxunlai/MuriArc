use std::{
    env,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use chrono::Utc;
use muriarc_delivery::{DeliveryConfig, PhysicalDriverClient, load_delivery_config};
use muriarc_upgrade::{
    BackupEvidence, DeploymentProfile, RestoreEvidence, TrustedMetadataVersions, TufVerifier,
    UpgradeError, VerificationEvidence, VerifiedRelease, verify_target_artifact,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{delivery_error, state_root};

pub(crate) const PHYSICAL_DRIVER_ENV: &str = "MURIARCCTL_PHYSICAL_DRIVER";
const TUF_ROOT_ENV: &str = "MURIARCCTL_TUF_ROOT";
const TUF_TIMESTAMP_ENV: &str = "MURIARCCTL_TUF_TIMESTAMP";
const TUF_SNAPSHOT_ENV: &str = "MURIARCCTL_TUF_SNAPSHOT";
const TUF_TARGETS_ENV: &str = "MURIARCCTL_TUF_TARGETS";
const TUF_TARGET_NAME_ENV: &str = "MURIARCCTL_TUF_TARGET_NAME";
const TARGET_ARTIFACT_ENV: &str = "MURIARCCTL_TARGET_ARTIFACT";
const TRUST_STATE_FORMAT: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TrustedMetadataState {
    format_version: u32,
    root_digest: String,
    versions: TrustedMetadataVersions,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct VerifiedBackupResponse {
    pub(crate) backup: BackupEvidence,
    pub(crate) restore: RestoreEvidence,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReadOnlyVerificationResponse {
    pub(crate) state_digest_before: String,
    pub(crate) state_digest_after: String,
    pub(crate) verification: VerificationEvidence,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OperationSelection {
    pub(crate) operation_id: uuid::Uuid,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RestoreOperationResponse {
    pub(crate) backup_id: uuid::Uuid,
    pub(crate) backup_artifact_digest: String,
    pub(crate) restored_generation_id: uuid::Uuid,
    pub(crate) data_loss_confirmation_recorded: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PruneResponse {
    pub(crate) backup_id: uuid::Uuid,
    pub(crate) artifact_deleted: bool,
}

pub(crate) fn control_context() -> Result<(DeliveryConfig, PhysicalDriverClient), UpgradeError> {
    let config = load_delivery_config(&state_root()).map_err(delivery_error)?;
    let executable = required_control_file(PHYSICAL_DRIVER_ENV)?;
    let driver = PhysicalDriverClient::new(executable, config.profile)?;
    Ok((config, driver))
}

pub(crate) fn load_verified_release(
    profile: DeploymentProfile,
    requested_version: Option<&str>,
) -> Result<VerifiedRelease, UpgradeError> {
    let root_path = required_control_file(TUF_ROOT_ENV)?;
    let timestamp_path = required_control_file(TUF_TIMESTAMP_ENV)?;
    let snapshot_path = required_control_file(TUF_SNAPSHOT_ENV)?;
    let targets_path = required_control_file(TUF_TARGETS_ENV)?;
    let artifact_path = required_control_file(TARGET_ARTIFACT_ENV)?;
    let target_name = required_upgrade_env(TUF_TARGET_NAME_ENV)?;

    let root = fs::read(&root_path).map_err(control_io)?;
    let timestamp = fs::read(timestamp_path).map_err(control_io)?;
    let snapshot = fs::read(snapshot_path).map_err(control_io)?;
    let targets = fs::read(targets_path).map_err(control_io)?;
    let root_digest = digest_bytes(&root);
    let now = Utc::now();
    let mut verifier = TufVerifier::from_trusted_root(&root, now)?;
    let release = verifier.verify_release(&timestamp, &snapshot, &targets, &target_name, now)?;
    if requested_version
        .is_some_and(|version| version != release.manifest.application_version.as_str())
    {
        return Err(UpgradeError::TargetInvalid {
            message: "signed target version differs from --to".to_owned(),
        });
    }
    let artifact_name = profile_artifact_name(profile)?;
    let artifact = release
        .manifest
        .artifacts
        .get(artifact_name)
        .ok_or_else(|| UpgradeError::TargetInvalid {
            message: "signed Release Manifest is missing the installed profile artifact".to_owned(),
        })?;
    if artifact.digest.as_str() != release.target_digest
        || artifact.size_bytes != release.target_length
    {
        return Err(UpgradeError::TargetInvalid {
            message: "TUF target differs from the profile artifact in Release Manifest".to_owned(),
        });
    }
    verify_target_artifact(&artifact_path, &release)?;
    persist_trusted_metadata(&state_root(), &root_digest, &release.metadata_versions)?;
    Ok(release)
}

fn profile_artifact_name(profile: DeploymentProfile) -> Result<&'static str, UpgradeError> {
    match profile {
        DeploymentProfile::NativeSystem => Ok("native-system"),
        DeploymentProfile::ManagedCompose => Ok("managed-compose"),
        DeploymentProfile::Desktop => Err(UpgradeError::Prerequisite {
            message: "Desktop updates must enter through the signed Tauri updater".to_owned(),
        }),
    }
}

fn persist_trusted_metadata(
    root: &Path,
    root_digest: &str,
    observed: &TrustedMetadataVersions,
) -> Result<(), UpgradeError> {
    let path = root.join("trusted-metadata-versions.json");
    if let Ok(metadata) = fs::symlink_metadata(&path) {
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(UpgradeError::JournalIntegrity {
                message: "trusted metadata state is not a regular file".to_owned(),
            });
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            if metadata.permissions().mode() & 0o077 != 0 {
                return Err(UpgradeError::JournalIntegrity {
                    message: "trusted metadata state permissions are too broad".to_owned(),
                });
            }
        }
        let previous: TrustedMetadataState =
            serde_json::from_slice(&fs::read(&path).map_err(control_io)?).map_err(|_| {
                UpgradeError::JournalIntegrity {
                    message: "trusted metadata state is malformed".to_owned(),
                }
            })?;
        if previous.format_version != TRUST_STATE_FORMAT {
            return Err(UpgradeError::JournalIntegrity {
                message: "trusted metadata state format is unsupported".to_owned(),
            });
        }
        if previous.root_digest != root_digest {
            return Err(UpgradeError::JournalIntegrity {
                message: "trusted root digest changed without an authenticated rotation".to_owned(),
            });
        }
        reject_version_rollback("root", observed.root, previous.versions.root)?;
        reject_version_rollback("timestamp", observed.timestamp, previous.versions.timestamp)?;
        reject_version_rollback("snapshot", observed.snapshot, previous.versions.snapshot)?;
        reject_version_rollback("targets", observed.targets, previous.versions.targets)?;
    }
    let state = TrustedMetadataState {
        format_version: TRUST_STATE_FORMAT,
        root_digest: root_digest.to_owned(),
        versions: observed.clone(),
    };
    fs::create_dir_all(root).map_err(control_io)?;
    let temporary = path.with_extension(format!("tmp-{}", uuid::Uuid::new_v4()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut output = options.open(&temporary).map_err(control_io)?;
    serde_json::to_writer_pretty(&mut output, &state).map_err(|error| {
        UpgradeError::Persistence {
            message: error.to_string(),
        }
    })?;
    output.write_all(b"\n").map_err(control_io)?;
    output.sync_all().map_err(control_io)?;
    if let Err(error) = fs::rename(&temporary, &path) {
        let _ = fs::remove_file(&temporary);
        return Err(control_io(error));
    }
    Ok(())
}

fn reject_version_rollback(
    role: &'static str,
    observed: u64,
    trusted: u64,
) -> Result<(), UpgradeError> {
    if observed < trusted {
        Err(UpgradeError::MetadataRollback {
            role: role.to_owned(),
            observed,
            trusted,
        })
    } else {
        Ok(())
    }
}

fn required_control_file(name: &'static str) -> Result<PathBuf, UpgradeError> {
    let path = PathBuf::from(required_upgrade_env(name)?);
    if !path.is_absolute() {
        return Err(UpgradeError::Prerequisite {
            message: format!("{name} must be an absolute path"),
        });
    }
    let metadata = fs::symlink_metadata(&path).map_err(|_| UpgradeError::Prerequisite {
        message: format!("required control input {name} is unavailable"),
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() == 0 {
        return Err(UpgradeError::Prerequisite {
            message: format!("required control input {name} must be a non-empty regular file"),
        });
    }
    Ok(path)
}

fn required_upgrade_env(name: &'static str) -> Result<String, UpgradeError> {
    env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| UpgradeError::Prerequisite {
            message: format!("required control environment variable {name} is missing"),
        })
}

fn digest_bytes(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

pub(crate) fn valid_digest(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn control_io(error: std::io::Error) -> UpgradeError {
    UpgradeError::Persistence {
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestRoot(PathBuf);

    impl TestRoot {
        fn new() -> Self {
            let path = env::temp_dir().join(format!(
                "muriarcctl-trust-state-test-{}",
                uuid::Uuid::new_v4()
            ));
            fs::create_dir_all(&path).expect("test state root should be created");
            Self(path)
        }
    }

    impl Drop for TestRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn versions(value: u64) -> TrustedMetadataVersions {
        TrustedMetadataVersions {
            root: value,
            timestamp: value,
            snapshot: value,
            targets: value,
        }
    }

    fn digest(fill: char) -> String {
        format!("sha256:{}", fill.to_string().repeat(64))
    }

    #[test]
    fn trusted_metadata_state_rejects_role_rollback() {
        let root = TestRoot::new();
        persist_trusted_metadata(&root.0, &digest('a'), &versions(4))
            .expect("initial trust state should persist");

        let mut rolled_back = versions(4);
        rolled_back.timestamp = 3;
        let error = persist_trusted_metadata(&root.0, &digest('a'), &rolled_back)
            .expect_err("timestamp rollback must fail closed");

        assert!(matches!(
            error,
            UpgradeError::MetadataRollback {
                role,
                observed: 3,
                trusted: 4,
            } if role == "timestamp"
        ));
    }

    #[test]
    fn trusted_metadata_state_rejects_root_digest_drift() {
        let root = TestRoot::new();
        persist_trusted_metadata(&root.0, &digest('a'), &versions(1))
            .expect("initial trust state should persist");

        let error = persist_trusted_metadata(&root.0, &digest('b'), &versions(2))
            .expect_err("unauthenticated root drift must fail closed");

        assert!(matches!(error, UpgradeError::JournalIntegrity { .. }));
    }

    #[test]
    fn trusted_metadata_state_is_owner_only() {
        let root = TestRoot::new();
        persist_trusted_metadata(&root.0, &digest('a'), &versions(1))
            .expect("trust state should persist");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = fs::metadata(root.0.join("trusted-metadata-versions.json"))
                .expect("trust state metadata should exist")
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }
}
