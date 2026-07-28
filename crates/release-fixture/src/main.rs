use std::{env, process::ExitCode};

#[tokio::main]
async fn main() -> ExitCode {
    match muriarc_release_fixture::run_cli(env::args_os().skip(1)).await {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => {
            eprintln!(
                "usage: muriarc-release-fixture <prepare-sqlite|prepare-postgres|finalize|verify> [options]"
            );
            ExitCode::from(2)
        }
        Err(error) => {
            eprintln!("muriarc-release-fixture: {error}");
            ExitCode::FAILURE
        }
    }
}
