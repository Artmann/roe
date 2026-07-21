mod human;
mod json;

use std::process::ExitCode;

use crate::cli::OutputFormat;
use crate::model::{AnalysisResult, Workspace};

pub fn emit(result: &AnalysisResult, workspace: &Workspace, format: OutputFormat) -> ExitCode {
    match format {
        OutputFormat::Human => human::print(result, workspace),
        OutputFormat::Json => json::print(result, workspace),
    }
    if result.findings.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}
