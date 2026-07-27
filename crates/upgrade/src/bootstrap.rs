use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    process::Command,
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{UpgradeError, VerifiedRelease};

pub const BOOTSTRAP_PROTOCOL_REVISION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BootstrapRequest {
    pub bootstrap_protocol_revision: u32,
    pub expected_controller_protocol_revision: u32,
    pub controller_path: PathBuf,
    pub controller_args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedBootstrapPlan {
    controller_path: PathBuf,
    controller_args: Vec<String>,
    release: VerifiedRelease,
}

impl VerifiedBootstrapPlan {
    pub fn controller_path(&self) -> &Path {
        &self.controller_path
    }

    pub fn controller_args(&self) -> &[String] {
        &self.controller_args
    }

    pub fn release(&self) -> &VerifiedRelease {
        &self.release
    }

    pub fn command(&self) -> Command {
        let mut command = Command::new(&self.controller_path);
        command.args(&self.controller_args);
        command
    }
}

pub fn verify_bootstrap_target(
    request: BootstrapRequest,
    release: VerifiedRelease,
) -> Result<VerifiedBootstrapPlan, UpgradeError> {
    release.validate_for_controller()?;
    if request.bootstrap_protocol_revision != BOOTSTRAP_PROTOCOL_REVISION
        || release.manifest.bootstrap_protocol_revision != BOOTSTRAP_PROTOCOL_REVISION
    {
        return Err(UpgradeError::ControllerProtocolMismatch {
            controller: BOOTSTRAP_PROTOCOL_REVISION,
            minimum: release.manifest.bootstrap_protocol_revision,
            maximum: release.manifest.bootstrap_protocol_revision,
        });
    }
    if request.expected_controller_protocol_revision < release.manifest.controller_protocol_min
        || request.expected_controller_protocol_revision > release.manifest.controller_protocol_max
    {
        return Err(UpgradeError::ControllerProtocolMismatch {
            controller: request.expected_controller_protocol_revision,
            minimum: release.manifest.controller_protocol_min,
            maximum: release.manifest.controller_protocol_max,
        });
    }
    verify_target_artifact(&request.controller_path, &release)?;
    Ok(VerifiedBootstrapPlan {
        controller_path: request.controller_path,
        controller_args: request.controller_args,
        release,
    })
}

pub fn verify_target_artifact(path: &Path, release: &VerifiedRelease) -> Result<(), UpgradeError> {
    let mut file = File::open(path).map_err(|error| UpgradeError::ArtifactVerification {
        message: format!("target artifact cannot be opened: {error}"),
    })?;
    let mut hasher = Sha256::new();
    let mut length = 0_u64;
    let mut buffer = [0_u8; 1024 * 64];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| UpgradeError::ArtifactVerification {
                message: format!("target artifact cannot be read: {error}"),
            })?;
        if read == 0 {
            break;
        }
        length = length
            .checked_add(
                u64::try_from(read).map_err(|_| UpgradeError::ArtifactVerification {
                    message: "target artifact length exceeds supported range".to_owned(),
                })?,
            )
            .ok_or_else(|| UpgradeError::ArtifactVerification {
                message: "target artifact length overflowed".to_owned(),
            })?;
        hasher.update(&buffer[..read]);
    }
    let digest = format!("sha256:{:x}", hasher.finalize());
    if length != release.target_length || digest != release.target_digest {
        return Err(UpgradeError::ArtifactVerification {
            message: "target artifact length or SHA-256 differs from signed metadata".to_owned(),
        });
    }
    Ok(())
}

/// Re-verifies the artifact immediately before process replacement. On Unix,
/// success does not return. Other platforms wait for the verified controller
/// and propagate its exit status.
pub fn reexec_verified_controller(plan: VerifiedBootstrapPlan) -> Result<(), UpgradeError> {
    verify_target_artifact(&plan.controller_path, &plan.release)?;
    let mut command = plan.command();
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        let error = command.exec();
        Err(UpgradeError::Prerequisite {
            message: format!("verified controller re-exec failed: {error}"),
        })
    }
    #[cfg(not(unix))]
    {
        let status = command
            .status()
            .map_err(|error| UpgradeError::Prerequisite {
                message: format!("verified controller could not start: {error}"),
            })?;
        if status.success() {
            Ok(())
        } else {
            Err(UpgradeError::Prerequisite {
                message: format!("verified controller exited with {status}"),
            })
        }
    }
}
