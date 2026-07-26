pub mod analyze;
pub mod churn;
pub mod cli;
pub mod clone_extraction;
pub mod commands;
pub mod config;
pub mod coupling;
pub mod discover;
pub mod entry_points;
pub mod extract;
pub mod graph;
pub mod hotspot;
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

    let result = match &cli.command {
        // No command runs all three analyses. The flattened `check` arguments
        // carry whatever was given on the command line.
        None => commands::check::run(&cli.check),
        Some(cli::Command::Check(args)) => commands::check::run(args),
        Some(cli::Command::DeadCode(args)) => commands::dead_code::run(args),
        Some(cli::Command::Dupes(args)) => commands::dupes::run(args),
        Some(cli::Command::Health(args)) => commands::health::run(args),
    };

    match result {
        Ok(exit_code) => exit_code,
        Err(error) => {
            eprintln!("error: {error:#}");
            if let Some(hint) = mistyped_command_hint(&cli) {
                eprintln!("hint: {hint}");
            }
            ExitCode::from(2)
        }
    }
}

/// The default run takes a positional path, so a mistyped command name (`roe
/// dupez`) reaches it as a path and fails with "path not found" — accurate, but
/// mystifying if the user thought they were naming a command. Nudge them towards
/// the command list when the failing path looks like a bare word rather than a
/// path.
fn mistyped_command_hint(cli: &cli::Cli) -> Option<String> {
    if cli.command.is_some() {
        return None;
    }

    let path = cli.check.path.as_ref()?;
    let value = path.to_str()?;
    let looks_like_a_path = value.contains('/')
        || value.contains('\\')
        || value.contains('.')
        || std::path::Path::new(value).exists();

    if looks_like_a_path {
        return None;
    }

    Some(format!(
        "`{value}` is not a path — run `roe --help` for the list of commands."
    ))
}
