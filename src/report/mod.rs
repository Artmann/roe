pub mod check;
pub mod dupes;
pub mod health;
pub(crate) mod human;
pub(crate) mod json;

use std::process::ExitCode;

use crate::cli::OutputFormat;
use crate::model::{AnalysisResult, Workspace};

pub fn emit(result: &AnalysisResult, workspace: &Workspace, format: OutputFormat) -> ExitCode {
    match format {
        OutputFormat::Human => human::print(result, workspace),
        OutputFormat::Json => json::print(result, workspace),
    }
    if result.has_findings() {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}
