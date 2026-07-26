mod cli;

use std::{
    env,
    path::{Path, PathBuf},
    process::ExitCode,
};

use cli::{CommandResponse, CtlCommand, HELP, OutputFormat, ParsedCommand, parse_args};
use muriarc_upgrade::{HostUpgradeLock, UpgradeError};
use serde::Serialize;
use serde_json::{Value, json};

#[tokio::main]
async fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err((format, command, error)) => {
            emit(
                format,
                &CommandResponse {
                    ok: false,
                    command,
                    code: error.code(),
                    message: error.safe_detail(),
                    data: Value::Null,
                },
            );
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<ExitCode, (OutputFormat, &'static str, UpgradeError)> {
    let parsed = parse_args(env::args_os().skip(1))
        .map_err(|error| (OutputFormat::Human, "parse", error))?;
    dispatch(parsed)
}

fn dispatch(parsed: ParsedCommand) -> Result<ExitCode, (OutputFormat, &'static str, UpgradeError)> {
    let command_name = parsed.command.name();
    match parsed.command {
        CtlCommand::Help => {
            if parsed.output == OutputFormat::Json {
                emit(
                    parsed.output,
                    &CommandResponse {
                        ok: true,
                        command: command_name,
                        code: "ok",
                        message: "command help".to_owned(),
                        data: json!({ "usage": HELP }),
                    },
                );
            } else {
                print!("{HELP}");
            }
            Ok(ExitCode::SUCCESS)
        }
        CtlCommand::Status => {
            let root = state_root();
            let lock = HostUpgradeLock::inspect(&root).map_err(|error| {
                (parsed.output, command_name, error)
            })?;
            emit(
                parsed.output,
                &CommandResponse {
                    ok: true,
                    command: command_name,
                    code: "ok",
                    message: "local control-plane status inspected".to_owned(),
                    data: json!({
                        "configured": deployment_driver_configured(),
                        "stateRootExists": root.is_dir(),
                        "upgradeLock": lock,
                    }),
                },
            );
            Ok(ExitCode::SUCCESS)
        }
        CtlCommand::Doctor => {
            let root = state_root();
            let required = [
                ("database", env::var_os("MURIARC_DATABASE_URL").is_some()),
                ("dataRoot", env::var_os("MURIARC_DATA_ROOT").is_some()),
                (
                    "attachmentRoot",
                    env::var_os("MURIARC_ATTACHMENT_ROOT").is_some(),
                ),
                ("deploymentDriver", deployment_driver_configured()),
            ];
            let ready = required.iter().all(|(_, available)| *available);
            emit(
                parsed.output,
                &CommandResponse {
                    ok: ready,
                    command: command_name,
                    code: if ready { "ok" } else { "prerequisite_missing" },
                    message: if ready {
                        "control-plane prerequisites are present".to_owned()
                    } else {
                        "one or more control-plane prerequisites are missing".to_owned()
                    },
                    data: json!({
                        "stateRoot": path_class(&root),
                        "checks": required
                            .into_iter()
                            .map(|(name, available)| json!({"name": name, "available": available}))
                            .collect::<Vec<_>>(),
                    }),
                },
            );
            Ok(if ready {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(2)
            })
        }
        command => Err((
            parsed.output,
            command.name(),
            UpgradeError::Prerequisite {
                message: "the selected deployment Driver is not installed; refusing to report a false upgrade success".to_owned(),
            },
        )),
    }
}

fn state_root() -> PathBuf {
    env::var_os("MURIARCCTL_STATE_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            #[cfg(windows)]
            {
                PathBuf::from(r"C:\ProgramData\MuriArc\control")
            }
            #[cfg(not(windows))]
            {
                PathBuf::from("/var/lib/muriarc/control")
            }
        })
}

fn deployment_driver_configured() -> bool {
    matches!(
        env::var("MURIARCCTL_PROFILE").ok().as_deref(),
        Some("native-system" | "managed-compose" | "desktop")
    )
}

fn path_class(path: &Path) -> &'static str {
    if path.is_dir() {
        "directory"
    } else if path.exists() {
        "invalid"
    } else {
        "missing"
    }
}

fn emit<T: Serialize>(format: OutputFormat, response: &T) {
    match format {
        OutputFormat::Json => println!(
            "{}",
            serde_json::to_string(response).expect("command response must be serializable")
        ),
        OutputFormat::Human => {
            let value =
                serde_json::to_value(response).expect("command response must be serializable");
            let ok = value.get("ok").and_then(Value::as_bool).unwrap_or(false);
            let code = value
                .get("code")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let message = value
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("no message");
            println!("{} [{code}] {message}", if ok { "OK" } else { "ERROR" });
        }
    }
}
