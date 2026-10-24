//! `parcel` — the Parcel command-line client.
//!
//! Publishes HTML pages (and their supporting files) to a Parcel server,
//! reads and manages existing parcels, and installs the bundled agent skill.
//! See `parcel --help`, and `skills/parcel/` for how agents are taught to use it.

mod cli;
mod client;
mod commands;
mod config;
mod error;
mod files;
mod git;
mod output;
mod skill;

use std::process::ExitCode;

use clap::Parser;

fn main() -> ExitCode {
    // clap prints its own usage errors and exits 2.
    let cli = cli::Cli::parse();
    match commands::run(cli) {
        Ok(()) => ExitCode::from(error::EXIT_OK),
        Err(err) if error::is_broken_pipe(&err) => ExitCode::from(error::EXIT_OK),
        Err(err) => {
            eprintln!("error: {}", error::format_error(&err));
            ExitCode::from(error::exit_code(&err))
        }
    }
}
