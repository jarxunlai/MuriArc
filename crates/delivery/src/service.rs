use std::{ffi::OsString, path::PathBuf, process::Command};

use muriarc_upgrade::DeploymentProfile;
use serde::{Deserialize, Serialize};

use crate::{DeliveryConfig, DeliveryError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandSpec {
    pub program: PathBuf,
    pub args: Vec<OsString>,
}

impl CommandSpec {
    pub fn new(program: impl Into<PathBuf>, args: impl IntoIterator<Item = OsString>) -> Self {
        Self {
            program: program.into(),
            args: args.into_iter().collect(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandOutcome {
    pub success: bool,
    pub exit_code: Option<i32>,
}

pub trait CommandRunner: Send + Sync {
    fn run(&self, command: &CommandSpec) -> Result<CommandOutcome, DeliveryError>;
}

#[derive(Debug, Default)]
pub struct ProcessCommandRunner;

impl CommandRunner for ProcessCommandRunner {
    fn run(&self, command: &CommandSpec) -> Result<CommandOutcome, DeliveryError> {
        if !command.program.is_absolute() {
            return Err(DeliveryError::InvalidPolicy(
                "service-control executables must use absolute paths".to_owned(),
            ));
        }
        let status = Command::new(&command.program)
            .args(&command.args)
            .status()
            .map_err(|error| DeliveryError::Command(error.to_string()))?;
        Ok(CommandOutcome {
            success: status.success(),
            exit_code: status.code(),
        })
    }
}

pub struct ServerServiceController<R> {
    config: DeliveryConfig,
    runner: R,
}

impl<R: CommandRunner> ServerServiceController<R> {
    pub fn new(config: DeliveryConfig, runner: R) -> Result<Self, DeliveryError> {
        config.validate()?;
        Ok(Self { config, runner })
    }

    pub fn stop_for_drain(&self) -> Result<(), DeliveryError> {
        self.require_success(&self.command("stop")?, "graceful service drain")
    }

    pub fn start_read_only(&self) -> Result<(), DeliveryError> {
        self.require_success(&self.command("start")?, "read-only service activation")
    }

    pub fn restart(&self) -> Result<(), DeliveryError> {
        self.require_success(&self.command("restart")?, "service restart")
    }

    pub fn is_active(&self) -> Result<bool, DeliveryError> {
        Ok(self.runner.run(&self.command("status")?)?.success)
    }

    fn require_success(&self, command: &CommandSpec, action: &str) -> Result<(), DeliveryError> {
        let outcome = self.runner.run(command)?;
        if outcome.success {
            Ok(())
        } else {
            Err(DeliveryError::Command(format!(
                "{action} failed with exit code {:?}",
                outcome.exit_code
            )))
        }
    }

    fn command(&self, action: &str) -> Result<CommandSpec, DeliveryError> {
        match self.config.profile {
            DeploymentProfile::NativeSystem => {
                let verb = match action {
                    "stop" => "stop",
                    "start" => "start",
                    "restart" => "restart",
                    "status" => "is-active",
                    _ => {
                        return Err(DeliveryError::InvalidPolicy(
                            "unknown service action".into(),
                        ));
                    }
                };
                Ok(CommandSpec::new(
                    "/usr/bin/systemctl",
                    [OsString::from(verb), OsString::from("muriarc.service")],
                ))
            }
            DeploymentProfile::ManagedCompose => {
                let compose = self.config.compose_file.as_ref().ok_or_else(|| {
                    DeliveryError::InvalidPolicy("Compose file is missing".into())
                })?;
                let project = self.config.compose_project.as_deref().ok_or_else(|| {
                    DeliveryError::InvalidPolicy("Compose project is missing".into())
                })?;
                let mut args = vec![
                    OsString::from("compose"),
                    OsString::from("--project-name"),
                    OsString::from(project),
                    OsString::from("--env-file"),
                    self.config.environment_file.as_os_str().to_owned(),
                    OsString::from("--env-file"),
                    self.config.activation_file.as_os_str().to_owned(),
                    OsString::from("--file"),
                    compose.as_os_str().to_owned(),
                ];
                match action {
                    "stop" => args.extend([
                        OsString::from("stop"),
                        OsString::from("--timeout"),
                        OsString::from("60"),
                        OsString::from("server"),
                    ]),
                    "start" => args.extend([
                        OsString::from("up"),
                        OsString::from("--detach"),
                        OsString::from("--no-build"),
                        OsString::from("server"),
                    ]),
                    "restart" => args.extend([
                        OsString::from("restart"),
                        OsString::from("--timeout"),
                        OsString::from("60"),
                        OsString::from("server"),
                    ]),
                    "status" => args.extend([
                        OsString::from("ps"),
                        OsString::from("--status"),
                        OsString::from("running"),
                        OsString::from("--quiet"),
                        OsString::from("server"),
                    ]),
                    _ => {
                        return Err(DeliveryError::InvalidPolicy(
                            "unknown service action".into(),
                        ));
                    }
                }
                Ok(CommandSpec::new("/usr/bin/docker", args))
            }
            DeploymentProfile::Desktop => Err(DeliveryError::InvalidPolicy(
                "Desktop service control is not a Server Driver".to_owned(),
            )),
        }
    }
}
