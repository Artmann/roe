use std::process::ExitCode;

use rustc_hash::FxHashSet;

use crate::cli::{CheckArgs, DeadCodeArgs, DupesArgs, HealthArgs};
use crate::commands::{self, dead_code, dupes, health};
use crate::report;

/// Run all three analyses over the same root and report them together. This is
/// what a bare `roe` does.
///
/// The context — root resolution and config discovery — is resolved once and
/// shared, so a combined run reads `roe.json` a single time. Per-analysis
/// options are not exposed on `check`: each analysis gets its defaults, which
/// the shared config can still override the same way it does for the individual
/// commands. Where to look and how to print are the context's and the reporter's
/// business, so the `path`, `config`, and `format` fields of these argument
/// structs are never read here.
pub fn run(args: &CheckArgs) -> anyhow::Result<ExitCode> {
    let context = commands::resolve(&args.path, &args.config)?;

    let dead_code_analysis = dead_code::execute(&context, &DeadCodeArgs::default())?;
    let dupes_analysis = dupes::execute(&context, &DupesArgs::default())?;
    let health_analysis = health::execute(&context, &HealthArgs::default())?;

    // All three walked the same tree, so they produced the same discovery and
    // config warnings. Print each distinct one once rather than three times.
    let mut seen = FxHashSet::default();
    for warning in dead_code_analysis
        .workspace
        .warnings
        .iter()
        .chain(&dupes_analysis.workspace.warnings)
        .chain(&health_analysis.workspace.warnings)
    {
        if seen.insert(warning.as_str()) {
            eprintln!("warning: {warning}");
        }
    }

    Ok(report::check::emit(
        &dead_code_analysis,
        &dupes_analysis,
        &health_analysis,
        args.format,
    ))
}
