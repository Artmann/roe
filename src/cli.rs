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

    /// Flag complexity, size, and coupling issues
    #[command(name = "health")]
    Health(HealthArgs),
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

#[derive(Debug, Args)]
pub struct HealthArgs {
    /// Path to the codebase root (defaults to the current directory)
    pub path: Option<PathBuf>,

    /// Output format
    #[arg(long, short = 'f', value_enum, default_value_t = OutputFormat::Human)]
    pub format: OutputFormat,

    // The threshold options are Option<u32> rather than clap defaults on
    // purpose: with `default_value_t` there is no way to tell "the user asked
    // for 10" from "nobody said anything", so a `health` block in roe.json
    // could never take effect. The defaults live in
    // `impl Default for config::EffectiveHealth` and are spelled out in the
    // help text below instead.
    /// Flag methods/properties above this cyclomatic complexity [default: 10]
    #[arg(long)]
    pub max_complexity: Option<u32>,

    /// Flag methods/properties above this cognitive complexity — like
    /// cyclomatic, but weighted by how deeply nested the code is [default: 15]
    #[arg(long)]
    pub max_cognitive: Option<u32>,

    /// Flag methods/properties whose body spans more than this many lines
    /// [default: 40]
    #[arg(long)]
    pub max_method_lines: Option<u32>,

    /// Flag methods/operators/indexers declared with more than this many
    /// parameters [default: 5]
    #[arg(long)]
    pub max_parameters: Option<u32>,

    /// Flag files longer than this many lines [default: 750]
    #[arg(long)]
    pub max_file_lines: Option<u32>,

    /// Flag types with more than this many members [default: 20]
    #[arg(long)]
    pub max_type_members: Option<u32>,

    /// Skip declarations in test projects — long arrange/act/assert methods
    /// and multi-case fixtures are normal there
    #[arg(long)]
    pub exclude_tests: bool,

    /// Order findings by how far past their threshold they sit, or by file
    /// path
    #[arg(long, value_enum, default_value_t = HealthSort::Severity)]
    pub sort: HealthSort,

    /// Print at most this many findings; 0 prints all of them
    #[arg(long, default_value_t = 0, value_name = "N")]
    pub limit: usize,

    /// Also rank the files that are both complex and frequently changed, read
    /// from git history. Informational — never affects the exit code
    #[arg(long)]
    pub hotspots: bool,

    /// How many hotspots to list
    #[arg(long, default_value_t = 10, requires = "hotspots")]
    pub top: usize,

    /// Path to an explicit roe.json/roe.yaml/roe.yml config (skips
    /// auto-discovery)
    #[arg(long, value_name = "PATH")]
    pub config: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum HealthSort {
    /// Worst first, by how many times over its threshold each finding sits.
    Severity,
    /// Grouped by file path, ascending — stable regardless of the metrics.
    Path,
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
