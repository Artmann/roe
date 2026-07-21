use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Debug, Parser)]
#[command(name = "roe", version, about = "Codebase intelligence for C#")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Find unused types, members, and files
    #[command(name = "dead-code")]
    DeadCode(DeadCodeArgs),
}

#[derive(Debug, Args)]
pub struct DeadCodeArgs {
    /// Path to the codebase root (defaults to the current directory)
    pub path: Option<PathBuf>,

    /// Output format
    #[arg(long, short = 'f', value_enum, default_value_t = OutputFormat::Human)]
    pub format: OutputFormat,

    /// Also flag enum members and public settable auto-properties
    #[arg(long)]
    pub aggressive: bool,

    /// Additional entry-point roots (fully-qualified symbol names)
    #[arg(long = "root", value_name = "FQN")]
    pub roots: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum OutputFormat {
    Human,
    Json,
}
