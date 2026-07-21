use std::{path::PathBuf, process::ExitCode};

use muriarc_legacy_migrator::{audit_legacy, migrate_legacy, write_json_report};
use serde::Serialize;

enum Command {
    Audit {
        source: PathBuf,
        report: Option<PathBuf>,
    },
    Migrate {
        source: PathBuf,
        target: PathBuf,
        report: Option<PathBuf>,
    },
    Help,
}

#[tokio::main]
async fn main() -> ExitCode {
    match parse_args(std::env::args().skip(1)).and_then(check_report_destination) {
        Ok(Command::Help) => {
            print_help();
            ExitCode::SUCCESS
        }
        Ok(command) => match run(command).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("error: {error}");
                ExitCode::FAILURE
            }
        },
        Err(error) => {
            eprintln!("error: {error}\n");
            print_help();
            ExitCode::from(2)
        }
    }
}

async fn run(command: Command) -> Result<(), Box<dyn std::error::Error>> {
    match command {
        Command::Audit { source, report } => {
            let result = audit_legacy(&source).await?;
            emit(&result, report.as_deref())?;
        }
        Command::Migrate {
            source,
            target,
            report,
        } => {
            let result = migrate_legacy(&source, &target).await?;
            emit(&result, report.as_deref())?;
        }
        Command::Help => print_help(),
    }
    Ok(())
}

fn emit<T: Serialize>(
    value: &T,
    report: Option<&std::path::Path>,
) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(path) = report {
        write_json_report(path, value)?;
    }
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn parse_args(args: impl Iterator<Item = String>) -> Result<Command, String> {
    let mut args = args.peekable();
    let Some(subcommand) = args.next() else {
        return Ok(Command::Help);
    };
    if matches!(subcommand.as_str(), "help" | "--help" | "-h") {
        return Ok(Command::Help);
    }
    if !matches!(subcommand.as_str(), "audit" | "migrate") {
        return Err(format!("unknown subcommand: {subcommand}"));
    }

    let mut source = None;
    let mut target = None;
    let mut report = None;
    while let Some(flag) = args.next() {
        if matches!(flag.as_str(), "--help" | "-h") {
            return Ok(Command::Help);
        }
        let value = args
            .next()
            .ok_or_else(|| format!("missing value for {flag}"))?;
        match flag.as_str() {
            "--source" => set_once(&mut source, PathBuf::from(value), "--source")?,
            "--target" => set_once(&mut target, PathBuf::from(value), "--target")?,
            "--report" => set_once(&mut report, PathBuf::from(value), "--report")?,
            _ => return Err(format!("unknown option: {flag}")),
        }
    }

    let source = source.ok_or_else(|| "--source is required".to_owned())?;
    match subcommand.as_str() {
        "audit" => {
            if target.is_some() {
                return Err("--target is only valid for migrate".to_owned());
            }
            Ok(Command::Audit { source, report })
        }
        "migrate" => Ok(Command::Migrate {
            source,
            target: target.ok_or_else(|| "--target is required for migrate".to_owned())?,
            report,
        }),
        _ => unreachable!(),
    }
}

fn set_once<T>(slot: &mut Option<T>, value: T, flag: &str) -> Result<(), String> {
    if slot.replace(value).is_some() {
        Err(format!("{flag} may only be supplied once"))
    } else {
        Ok(())
    }
}

fn check_report_destination(command: Command) -> Result<Command, String> {
    let report = match &command {
        Command::Audit { report, .. } | Command::Migrate { report, .. } => report.as_ref(),
        Command::Help => None,
    };
    if let Some(path) = report
        && std::fs::symlink_metadata(path).is_ok()
    {
        return Err(format!(
            "report already exists; refusing to overwrite it: {}",
            path.display()
        ));
    }
    Ok(command)
}

fn print_help() {
    println!(
        "MuriArc legacy migrator\n\n\
Usage:\n  \
  muriarc-legacy-migrator audit --source <mice.db> [--report <audit.json>]\n  \
  muriarc-legacy-migrator migrate --source <mice.db> --target <new.db> [--report <migration.json>]\n\n\
Safety:\n  \
  The source is opened read-only. Existing target databases and JSON reports are never overwritten."
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_migrate_command() {
        let command = parse_args(
            [
                "migrate",
                "--source",
                "old.db",
                "--target",
                "new.db",
                "--report",
                "report.json",
            ]
            .into_iter()
            .map(str::to_owned),
        )
        .unwrap();
        assert!(matches!(command, Command::Migrate { .. }));
    }

    #[test]
    fn target_is_required_for_migrate() {
        let error = parse_args(
            ["migrate", "--source", "old.db"]
                .into_iter()
                .map(str::to_owned),
        )
        .err()
        .unwrap();
        assert!(error.contains("--target"));
    }
}
