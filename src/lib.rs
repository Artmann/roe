pub mod analyze;
pub mod cli;
pub mod clone_extraction;
pub mod commands;
pub mod config;
pub mod discover;
pub mod entry_points;
pub mod extract;
pub mod graph;
pub mod model;
pub mod paths;
pub mod report;
pub mod resolve;
pub mod rules;
pub mod suffix_array;
pub mod suppress;
pub mod tokenize;

use std::process::ExitCode;

use clap::Parser;

pub fn run() -> ExitCode {
    let cli = cli::Cli::parse();

    let result = match cli.command {
        cli::Command::DeadCode(args) => commands::dead_code::run(&args),
        cli::Command::Dupes(args) => commands::dupes::run(&args),
    };

    match result {
        Ok(exit_code) => exit_code,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::from(2)
        }
    }
}
