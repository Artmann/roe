pub(crate) mod human;
pub(crate) mod json;

use std::process::ExitCode;

use crate::cli::{HealthSort, OutputFormat};
use crate::model::{HealthResult, Workspace};

/// `sort` and `limit` are presentation only, and so apply to the human report
/// alone: JSON is consumed by tooling that does its own ordering and would be
/// actively harmed by a silently truncated array.
pub fn emit(
    result: &HealthResult,
    workspace: &Workspace,
    format: OutputFormat,
    sort: HealthSort,
    limit: usize,
) -> ExitCode {
    match format {
        OutputFormat::Human => human::print(result, workspace, sort, limit),
        OutputFormat::Json => json::print(result, workspace),
    }
    if result.has_findings() {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}
