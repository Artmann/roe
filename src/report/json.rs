use serde::Serialize;

use crate::model::{AnalysisResult, FindingKind, Workspace};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsonReport<'a> {
    version: u32,
    root: String,
    summary: JsonSummary,
    notes: &'a [String],
    findings: Vec<JsonFinding>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsonSummary {
    projects: usize,
    files_scanned: usize,
    symbols: usize,
    unused_types: usize,
    unused_members: usize,
    unused_files: usize,
    elapsed_ms: u128,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsonFinding {
    kind: FindingKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    symbol_kind: Option<&'static str>,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    project: Option<String>,
    file: String,
    line: u32,
    column: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    visibility: Option<&'static str>,
}

pub fn print(result: &AnalysisResult, workspace: &Workspace) {
    let report = JsonReport {
        version: 1,
        root: workspace.root.display().to_string(),
        summary: JsonSummary {
            projects: result.summary.projects,
            files_scanned: result.summary.files_scanned,
            symbols: result.summary.symbols,
            unused_types: result.summary.unused_types,
            unused_members: result.summary.unused_members,
            unused_files: result.summary.unused_files,
            elapsed_ms: result.summary.elapsed_ms,
        },
        notes: &result.notes,
        findings: result
            .findings
            .iter()
            .map(|finding| JsonFinding {
                kind: finding.kind,
                symbol_kind: finding.symbol_kind.map(|k| k.label()),
                name: finding.name.clone(),
                project: finding.project.clone(),
                file: finding
                    .file
                    .strip_prefix(&workspace.root)
                    .unwrap_or(&finding.file)
                    .display()
                    .to_string(),
                line: finding.line,
                column: finding.column,
                visibility: finding.visibility.map(|v| v.label()),
            })
            .collect(),
    };

    match serde_json::to_string_pretty(&report) {
        Ok(json) => println!("{json}"),
        Err(error) => eprintln!("error: failed to serialize JSON report: {error}"),
    }
}
