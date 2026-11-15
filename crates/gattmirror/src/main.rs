//! The `gattmirror` binary: argument handling only; see the library for the mirror.

use std::process::ExitCode;

use gattmirror::cli::{self, Command, RunError, USAGE};

fn main() -> ExitCode {
    match cli::parse(std::env::args().skip(1)) {
        Ok(Command::Help) => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        Ok(Command::Run(args)) => match cli::run(&args, &mut std::io::stdout()) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e @ (RunError::ServingNotWired | RunError::NoAdapterBackend)) => {
                eprintln!("gattmirror: {e}");
                ExitCode::from(3)
            }
            Err(e) => {
                eprintln!("gattmirror: {e}");
                ExitCode::FAILURE
            }
        },
        Err(e) => {
            eprintln!("gattmirror: {e}\n\n{USAGE}");
            ExitCode::from(2)
        }
    }
}
