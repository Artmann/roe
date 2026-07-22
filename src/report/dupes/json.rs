use serde::Serialize;

use crate::cli::DupeMode;
use crate::model::{DupesResult, Workspace};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsonReport {
    version: u32,
    root: String,
    mode: &'static str,
    summary: JsonSummary,
    groups: Vec<JsonGroup>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsonSummary {
    projects: usize,
    files_scanned: usize,
    groups: usize,
    duplicated_lines: usize,
    elapsed_ms: u128,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsonGroup {
    token_count: u32,
    line_count: u32,
    occurrences: Vec<JsonOccurrence>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsonOccurrence {
    file: String,
    start_line: u32,
    start_column: u32,
    end_line: u32,
    end_column: u32,
}

pub fn print(result: &DupesResult, workspace: &Workspace, mode: DupeMode) {
    let report = JsonReport {
        version: 1,
        root: crate::paths::display(&workspace.root),
        mode: match mode {
            DupeMode::Exact => "exact",
            DupeMode::Semantic => "semantic",
        },
        summary: JsonSummary {
            projects: result.summary.projects,
            files_scanned: result.summary.files_scanned,
            groups: result.summary.groups,
            duplicated_lines: result.summary.duplicated_lines,
            elapsed_ms: result.summary.elapsed_ms,
        },
        groups: result
            .groups
            .iter()
            .map(|group| JsonGroup {
                token_count: group.token_count,
                line_count: group.line_count,
                occurrences: group
                    .occurrences
                    .iter()
                    .map(|occurrence| JsonOccurrence {
                        file: crate::paths::display(
                            occurrence
                                .file
                                .strip_prefix(&workspace.root)
                                .unwrap_or(&occurrence.file),
                        ),
                        start_line: occurrence.start_line,
                        start_column: occurrence.start_column,
                        end_line: occurrence.end_line,
                        end_column: occurrence.end_column,
                    })
                    .collect(),
            })
            .collect(),
    };

    match serde_json::to_string_pretty(&report) {
        Ok(json) => println!("{json}"),
        Err(error) => eprintln!("error: failed to serialize JSON report: {error}"),
    }
}
