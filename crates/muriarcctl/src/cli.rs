use std::ffi::OsString;

use muriarc_upgrade::{DeploymentProfile, UpgradeError};
use serde::Serialize;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Human,
    Json,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CtlCommand {
    Help,
    Install {
        profile: DeploymentProfile,
    },
    Doctor,
    Status,
    UpdateCheck,
    Upgrade {
        to: Option<String>,
    },
    BackupCreate,
    BackupVerify,
    VerifyReadOnly,
    RecoveryResume {
        operation_id: Option<Uuid>,
    },
    RecoveryRestore {
        backup_id: Option<Uuid>,
        confirm_data_loss: bool,
    },
    RecoveryPrune {
        backup_id: Uuid,
    },
}

impl CtlCommand {
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Help => "help",
            Self::Install { .. } => "install",
            Self::Doctor => "doctor",
            Self::Status => "status",
            Self::UpdateCheck => "update_check",
            Self::Upgrade { .. } => "upgrade",
            Self::BackupCreate => "backup_create",
            Self::BackupVerify => "backup_verify",
            Self::VerifyReadOnly => "verify_read_only",
            Self::RecoveryResume { .. } => "recovery_resume",
            Self::RecoveryRestore { .. } => "recovery_restore",
            Self::RecoveryPrune { .. } => "recovery_prune",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedCommand {
    pub output: OutputFormat,
    pub command: CtlCommand,
}

#[derive(Debug, Serialize)]
pub struct CommandResponse<T: Serialize> {
    pub ok: bool,
    pub command: &'static str,
    pub code: &'static str,
    pub message: String,
    pub data: T,
}

pub fn parse_args(args: impl IntoIterator<Item = OsString>) -> Result<ParsedCommand, UpgradeError> {
    let mut args = args
        .into_iter()
        .map(|value| {
            value
                .into_string()
                .map_err(|_| UpgradeError::InvalidCommand {
                    message: "arguments must be valid UTF-8".to_owned(),
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if args.first().is_some_and(|value| value == "muriarcctl") {
        args.remove(0);
    }
    if args
        .iter()
        .any(|value| value == "--force" || value == "--skip-verify")
    {
        return Err(UpgradeError::InvalidCommand {
            message: "verification and recovery safety gates cannot be bypassed".to_owned(),
        });
    }
    let mut output = OutputFormat::Human;
    let mut index = 0;
    while index < args.len() {
        if args[index] == "--output" {
            let value = args
                .get(index + 1)
                .ok_or_else(|| UpgradeError::InvalidCommand {
                    message: "--output requires human or json".to_owned(),
                })?;
            output = match value.as_str() {
                "human" => OutputFormat::Human,
                "json" => OutputFormat::Json,
                _ => {
                    return Err(UpgradeError::InvalidCommand {
                        message: "--output requires human or json".to_owned(),
                    });
                }
            };
            args.drain(index..=index + 1);
        } else {
            index += 1;
        }
    }
    let command = parse_command(&args)?;
    Ok(ParsedCommand { output, command })
}

fn parse_command(args: &[String]) -> Result<CtlCommand, UpgradeError> {
    let Some(command) = args.first().map(String::as_str) else {
        return Ok(CtlCommand::Help);
    };
    match command {
        "help" | "-h" | "--help" => exact(args, 1, CtlCommand::Help),
        "doctor" => exact(args, 1, CtlCommand::Doctor),
        "status" => exact(args, 1, CtlCommand::Status),
        "install" => {
            if args.len() != 3 || args[1] != "--profile" {
                return invalid(
                    "usage: muriarcctl install --profile native-system|managed-compose",
                );
            }
            let profile = match args[2].as_str() {
                "native-system" => DeploymentProfile::NativeSystem,
                "managed-compose" => DeploymentProfile::ManagedCompose,
                _ => {
                    return invalid(
                        "stable Server install profile must be native-system or managed-compose",
                    );
                }
            };
            Ok(CtlCommand::Install { profile })
        }
        "update" if args.get(1).map(String::as_str) == Some("check") => {
            exact(args, 2, CtlCommand::UpdateCheck)
        }
        "upgrade" => {
            if args.len() == 1 {
                Ok(CtlCommand::Upgrade { to: None })
            } else if args.len() == 3 && args[1] == "--to" && !args[2].trim().is_empty() {
                Ok(CtlCommand::Upgrade {
                    to: Some(args[2].clone()),
                })
            } else {
                invalid("usage: muriarcctl upgrade [--to <version>]")
            }
        }
        "backup" if args.get(1).map(String::as_str) == Some("create") => {
            exact(args, 2, CtlCommand::BackupCreate)
        }
        "backup" if args.get(1).map(String::as_str) == Some("verify") => {
            exact(args, 2, CtlCommand::BackupVerify)
        }
        "verify" if args.get(1).map(String::as_str) == Some("--read-only") => {
            exact(args, 2, CtlCommand::VerifyReadOnly)
        }
        "recovery" if args.get(1).map(String::as_str) == Some("resume") => {
            if args.len() == 2 {
                Ok(CtlCommand::RecoveryResume { operation_id: None })
            } else if args.len() == 4 && args[2] == "--operation" {
                Ok(CtlCommand::RecoveryResume {
                    operation_id: Some(parse_uuid(&args[3], "operation")?),
                })
            } else {
                invalid("usage: muriarcctl recovery resume [--operation <uuid>]")
            }
        }
        "recovery" if args.get(1).map(String::as_str) == Some("restore") => {
            let mut backup_id = None;
            let mut confirm_data_loss = false;
            let mut index = 2;
            while index < args.len() {
                match args[index].as_str() {
                    "--backup" => {
                        let value =
                            args.get(index + 1)
                                .ok_or_else(|| UpgradeError::InvalidCommand {
                                    message: "--backup requires a UUID".to_owned(),
                                })?;
                        backup_id = Some(parse_uuid(value, "backup")?);
                        index += 2;
                    }
                    "--confirm-data-loss" => {
                        confirm_data_loss = true;
                        index += 1;
                    }
                    _ => return invalid("unsupported recovery restore argument"),
                }
            }
            Ok(CtlCommand::RecoveryRestore {
                backup_id,
                confirm_data_loss,
            })
        }
        "recovery" if args.get(1).map(String::as_str) == Some("prune") => {
            if args.len() != 4 || args[2] != "--backup" {
                return invalid("usage: muriarcctl recovery prune --backup <uuid>");
            }
            Ok(CtlCommand::RecoveryPrune {
                backup_id: parse_uuid(&args[3], "backup")?,
            })
        }
        "migration" | "migrate" | "raw-migration" => {
            invalid("raw migration commands are intentionally unavailable")
        }
        _ => invalid("unknown command; run muriarcctl help"),
    }
}

fn exact(args: &[String], length: usize, command: CtlCommand) -> Result<CtlCommand, UpgradeError> {
    if args.len() == length {
        Ok(command)
    } else {
        invalid("unexpected command arguments")
    }
}

fn parse_uuid(value: &str, label: &str) -> Result<Uuid, UpgradeError> {
    Uuid::parse_str(value).map_err(|_| UpgradeError::InvalidCommand {
        message: format!("{label} identifier must be a UUID"),
    })
}

fn invalid<T>(message: &str) -> Result<T, UpgradeError> {
    Err(UpgradeError::InvalidCommand {
        message: message.to_owned(),
    })
}

pub const HELP: &str = "\
MuriArc safe upgrade control plane

Usage:
  muriarcctl install --profile native-system|managed-compose
  muriarcctl doctor|status [--output json]
  muriarcctl update check
  muriarcctl upgrade [--to <version>]
  muriarcctl backup create|verify
  muriarcctl verify --read-only
  muriarcctl recovery resume [--operation <uuid>]
  muriarcctl recovery restore [--backup <uuid>] [--confirm-data-loss]
  muriarcctl recovery prune --backup <uuid>

There is no raw migration, --force, or skip-verification command.
";

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(values: &[&str]) -> Result<ParsedCommand, UpgradeError> {
        parse_args(values.iter().map(OsString::from))
    }

    #[test]
    fn public_command_contract_parses() {
        assert!(matches!(
            parse(&["upgrade", "--to", "1.2.3", "--output", "json"])
                .unwrap()
                .command,
            CtlCommand::Upgrade { to: Some(_) }
        ));
        assert!(matches!(
            parse(&["install", "--profile", "managed-compose"])
                .unwrap()
                .command,
            CtlCommand::Install {
                profile: DeploymentProfile::ManagedCompose
            }
        ));
        assert!(matches!(
            parse(&["verify", "--read-only"]).unwrap().command,
            CtlCommand::VerifyReadOnly
        ));
    }

    #[test]
    fn bypass_and_raw_migration_are_rejected() {
        assert!(parse(&["upgrade", "--force"]).is_err());
        assert!(parse(&["upgrade", "--skip-verify"]).is_err());
        assert!(parse(&["migrate"]).is_err());
    }
}
