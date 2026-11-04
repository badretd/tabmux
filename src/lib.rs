//! `tabmux` — safe, reproducible table and workbook conversion.
//!
//! The crate is split into a testable library and a thin binary so that every
//! command can be exercised without spawning a process.

pub mod cli;
pub mod error;
pub mod formats;
pub mod io;
pub mod model;

use std::process::ExitCode;

use clap::Parser as _;

/// Parse arguments, run the requested command, and return a process exit code.
///
/// CLI parsing errors are handled by `clap`, which prints usage information to
/// stderr and exits with code 2. Runtime errors are mapped through
/// [`error::ExitCode`].
pub fn run() -> ExitCode {
    let cli = cli::Cli::parse();

    match cli::dispatch(cli) {
        Ok(()) => error::ExitCode::Success.process(),
        Err(error) => {
            eprintln!("tabmux: {error}");
            error.exit_code().process()
        }
    }
}
