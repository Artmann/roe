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

    // `dupes` runs first and alone. It is the one analysis that shares nothing
    // with the others — it tokenizes rather than extracting facts, and filters
    // generated files out of the workspace first — so running it before the
    // shared extraction exists keeps its corpus out of the peak.
    let dupes_analysis = dupes::execute(&context, &DupesArgs::default())?;

    // Discovery and parsing are identical work for the other two and dominate
    // the run, so they happen once and both read the result. Scoped so the
    // facts are freed before reporting.
    let (dead_code_analysis, health_analysis) = {
        let extracted = commands::Extracted::build(&context.root)?;

        (
            dead_code::execute_extracted(&context, &DeadCodeArgs::default(), &extracted)?,
            health::execute_extracted(&context, &HealthArgs::default(), &extracted)?,
        )
    };

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
