#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let mut args = std::env::args_os();
    let _program = args.next();
    if args
        .next()
        .is_some_and(|value| value == "--muriarc-standard-fixture")
    {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap_or_else(|error| {
                eprintln!("MuriArc fixture runtime could not start: {error}");
                std::process::exit(1);
            });
        match runtime.block_on(muriarc_standard_fixture::run_cli(args)) {
            Ok(true) => return,
            Ok(false) => {
                eprintln!(
                    "usage: MuriArc --muriarc-standard-fixture <seed|verify> --fixture <standard-v1-dir> --output <new-data-root> --source-commit <40-hex>"
                );
                std::process::exit(2);
            }
            Err(error) => {
                eprintln!("MuriArc standard fixture failed: {error}");
                std::process::exit(1);
            }
        }
    }
    muriarc_desktop_lib::run();
}
