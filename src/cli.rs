use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Debug, Parser)]
#[command(
    name = "roe",
    version,
    about = "Codebase intelligence for C#",
    long_about = "Codebase intelligence for C#.\n\nWith no command, roe runs dead-code, dupes, \
                  and health together — the same thing as `roe check`.",
    // The flattened `check` args include a positional PATH, so without this a
    // subcommand could be mixed with top-level args and the parse would be
    // ambiguous. Subcommand names still win over the positional, so
    // `roe dupes` is the dupes command and not a path called "dupes".
    args_conflicts_with_subcommands = true
)]
pub struct Cli {
    /// Arguments for the default combined run, used when no command is given.
    #[command(flatten)]
    pub check: CheckArgs,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Run dead-code, dupes, and health together (the default when no command
    /// is given)
    #[command(name = "check")]
    Check(CheckArgs),

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

/// The combined run deliberately takes only the arguments that mean the same
/// thing to all three analyses. Per-analysis tuning stays on the individual
/// subcommands; for a combined run it comes from `roe.json` instead.
#[derive(Debug, Args, PartialEq, Eq)]
pub struct CheckArgs {
    /// Path to the codebase root (defaults to the current directory)
    pub path: Option<PathBuf>,

    /// Output format
    #[arg(long, short = 'f', value_enum, default_value_t = OutputFormat::Human)]
    pub format: OutputFormat,

    /// Path to an explicit roe.json/roe.yaml/roe.yml config (skips
    /// auto-discovery)
    #[arg(long, value_name = "PATH")]
    pub config: Option<PathBuf>,
}

#[derive(Debug, Args, PartialEq, Eq)]
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

// The `Default` impls below mirror the `default_value_t` attributes above them
// so `check` can build each command's arguments without restating the defaults.
// `defaults_match_clap` in the tests at the bottom of this file fails if the two
// ever drift apart.
impl Default for DeadCodeArgs {
    fn default() -> Self {
        DeadCodeArgs {
            path: None,
            format: OutputFormat::Human,
            aggressive: false,
            roots: Vec::new(),
            library_projects: Vec::new(),
            config: None,
        }
    }
}

#[derive(Debug, Args, PartialEq, Eq)]
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

impl Default for HealthArgs {
    fn default() -> Self {
        HealthArgs {
            path: None,
            format: OutputFormat::Human,
            max_complexity: None,
            max_cognitive: None,
            max_method_lines: None,
            max_parameters: None,
            max_file_lines: None,
            max_type_members: None,
            exclude_tests: false,
            sort: HealthSort::Severity,
            limit: 0,
            hotspots: false,
            top: 10,
            config: None,
        }
    }
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

#[derive(Debug, Args, PartialEq, Eq)]
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

impl Default for DupesArgs {
    fn default() -> Self {
        DupesArgs {
            path: None,
            format: OutputFormat::Human,
            mode: DupeMode::Exact,
            no_code: false,
            min_tokens: 50,
            min_lines: 5,
            min_occurrences: 2,
            config: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_command_means_the_combined_run() {
        let cli = Cli::parse_from(["roe"]);

        assert!(cli.command.is_none());
        assert_eq!(cli.check.path, None);
        assert_eq!(cli.check.format, OutputFormat::Human);
        assert_eq!(cli.check.config, None);
    }

    #[test]
    fn no_command_still_takes_a_path() {
        let cli = Cli::parse_from(["roe", "./src", "--format", "json"]);

        assert!(cli.command.is_none());
        assert_eq!(cli.check.path, Some(PathBuf::from("./src")));
        assert_eq!(cli.check.format, OutputFormat::Json);
    }

    /// The top-level positional path must not shadow the subcommand names.
    #[test]
    fn subcommands_win_over_the_default_path() {
        let cli = Cli::parse_from(["roe", "dupes", "."]);

        assert!(matches!(cli.command, Some(Command::Dupes(_))));

        let cli = Cli::parse_from(["roe", "check", "."]);

        assert!(matches!(cli.command, Some(Command::Check(_))));
    }

    /// `#[derive(Args)]` gives no `CommandFactory`, so these structs cannot be
    /// parsed on their own the way `Cli` can — build a throwaway command around
    /// the struct's own arguments instead.
    fn parse_bare<T: Args + clap::FromArgMatches>() -> T {
        let command = T::augment_args(clap::Command::new("test"));
        let matches = command
            .try_get_matches_from(["test"])
            .expect("no arguments are required");

        T::from_arg_matches(&matches).expect("matches build the struct")
    }

    /// `check` builds each command's arguments from `Default`, so the
    /// hand-written impls have to agree with what clap produces for a bare
    /// invocation.
    #[test]
    fn defaults_match_clap() {
        assert_eq!(parse_bare::<DeadCodeArgs>(), DeadCodeArgs::default());
        assert_eq!(parse_bare::<DupesArgs>(), DupesArgs::default());
        assert_eq!(parse_bare::<HealthArgs>(), HealthArgs::default());
    }
}
