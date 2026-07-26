pub(crate) mod json;

use std::process::ExitCode;

use colored::Colorize;

use crate::cli::{DupeMode, HealthSort, OutputFormat};
use crate::commands::{dead_code, dupes, health};
use crate::report;

/// The presentation defaults the combined run uses. `check` takes no
/// per-analysis flags, so these mirror what each command does when invoked with
/// nothing but a path.
const DUPE_MODE: DupeMode = DupeMode::Exact;
const HEALTH_SORT: HealthSort = HealthSort::Severity;

/// Fixed rather than terminal width, so the output diffs cleanly between runs
/// and machines.
const SECTION_WIDTH: usize = 60;

/// Report all three analyses together.
///
/// Human output is the three existing reports verbatim, each under a section
/// header, so the combined run prints exactly what running the three commands in
/// sequence would. JSON output is a single document instead of three
/// concatenated ones, which would not be parseable.
pub fn emit(
    dead_code: &dead_code::Analysis,
    dupes: &dupes::Analysis,
    health: &health::Analysis,
    format: OutputFormat,
) -> ExitCode {
    match format {
        OutputFormat::Human => {
            section("dead-code");
            report::emit(&dead_code.result, &dead_code.workspace, format);

            println!();
            section("dupes");
            report::dupes::emit(
                &dupes.result,
                &dupes.workspace,
                format,
                DUPE_MODE,
                // `--no-code` is a dupes-only flag; the combined run shows the
                // duplicated source the same way a bare `roe dupes` does.
                true,
            );

            println!();
            section("health");
            report::health::emit(
                &health.result,
                &health.workspace,
                format,
                HEALTH_SORT,
                // 0 means "print everything" — truncating a combined run would
                // hide findings the user never asked to hide.
                0,
            );
        }
        OutputFormat::Json => json::print(dead_code, dupes, health, DUPE_MODE),
    }

    let found = dead_code.result.has_findings()
        || dupes.result.has_findings()
        || health.result.has_findings();

    if found {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

/// A header separating one analysis' report from the next.
fn section(name: &str) {
    let label = format!("── {name} ");
    let rule = "─".repeat(SECTION_WIDTH.saturating_sub(label.chars().count()));

    println!("{}", format!("{label}{rule}").dimmed());
    println!();
}
