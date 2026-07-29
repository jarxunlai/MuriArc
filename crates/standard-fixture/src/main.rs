use std::{env, process::ExitCode};

use muriarc_standard_fixture::run_cli;

#[tokio::main]
async fn main() -> ExitCode {
    match run_cli(env::args_os().skip(1)).await {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => {
            eprintln!(
                "usage: muriarc-standard-fixture <seed|verify|seed-postgres|verify-postgres> --fixture <standard-v1-dir> --output <new-data-root> --source-commit <40-hex>"
            );
            ExitCode::from(2)
        }
        Err(error) => {
            eprintln!("muriarc-standard-fixture: {error}");
            ExitCode::FAILURE
        }
    }
}
