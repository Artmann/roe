pub(crate) mod human;
pub(crate) mod json;

use std::process::ExitCode;

use crate::cli::{OutputFormat, PerfSort};
use crate::model::{PerfResult, Workspace};

/// Only high-confidence findings move the exit code.
///
/// A medium-confidence check is roe guessing without type information, and a
/// guess that turns out wrong must not be able to break someone's build — so
/// this asks [`PerfResult::has_gating_findings`] rather than
/// [`PerfResult::has_findings`], which is the one place `perf` deliberately
/// departs from the other commands.
///
/// `sort` and `limit` are presentation only, and so apply to the human report
/// alone: JSON is consumed by tooling that does its own ordering and would be
/// actively harmed by a silently truncated array.
pub fn emit(
    result: &PerfResult,
    workspace: &Workspace,
    format: OutputFormat,
    sort: PerfSort,
    limit: usize,
) -> ExitCode {
    match format {
        OutputFormat::Human => human::print(result, workspace, sort, limit),
        OutputFormat::Json => json::print(result, workspace),
    }

    if result.has_gating_findings() {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}
