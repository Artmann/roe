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

    /// Find duplicated code blocks
    #[command(name = "dupes")]
    Dupes(DupesArgs),
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

    /// Project names to always treat in library mode (public API is used),
    /// regardless of executables elsewhere in the workspace
    #[arg(long = "library", value_name = "PROJECT")]
    pub library_projects: Vec<String>,

    /// Path to an explicit roe.json/roe.yaml/roe.yml config (skips
    /// auto-discovery)
    #[arg(long, value_name = "PATH")]
    pub config: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum OutputFormat {
    Human,
    Json,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum DupeMode {
    /// Exact token match — only verbatim copy-pasted blocks match.
    Exact,
    /// Identifiers and numeric literals are normalized to a shared
    /// placeholder, so renamed-but-structurally-identical blocks match too.
    Semantic,
}

#[derive(Debug, Args)]
pub struct DupesArgs {
    /// Path to the codebase root (defaults to the current directory)
    pub path: Option<PathBuf>,

    /// Output format
    #[arg(long, short = 'f', value_enum, default_value_t = OutputFormat::Human)]
    pub format: OutputFormat,

    /// Matching mode
    #[arg(long, value_enum, default_value_t = DupeMode::Exact)]
    pub mode: DupeMode,

    /// Hide the duplicated source code printed under each group (human format
    /// only)
    #[arg(long)]
    pub no_code: bool,

    /// Minimum token-run length for a match to be reported
    #[arg(long, default_value_t = 50)]
    pub min_tokens: u32,

    /// Minimum line span (of the shortest occurrence) for a match to be
    /// reported
    #[arg(long, default_value_t = 5)]
    pub min_lines: u32,

    /// Minimum number of occurrences for a match to be reported
    #[arg(long, default_value_t = 2)]
    pub min_occurrences: u32,

    /// Path to an explicit roe.json/roe.yaml/roe.yml config (skips
    /// auto-discovery)
    #[arg(long, value_name = "PATH")]
    pub config: Option<PathBuf>,
}
