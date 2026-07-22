mod human;
mod json;

use std::process::ExitCode;

use crate::cli::{DupeMode, OutputFormat};
use crate::model::{DupesResult, Workspace};

pub fn emit(
    result: &DupesResult,
    workspace: &Workspace,
    format: OutputFormat,
    mode: DupeMode,
) -> ExitCode {
    match format {
        OutputFormat::Human => human::print(result, workspace),
        OutputFormat::Json => json::print(result, workspace, mode),
    }
    if result.groups.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}
