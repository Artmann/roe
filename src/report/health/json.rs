use serde::Serialize;

use crate::model::{HealthFindingKind, HealthResult, Workspace};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsonReport {
    version: u32,
    root: String,
    summary: JsonSummary,
    findings: Vec<JsonFinding>,
    cycles: Vec<JsonCycle>,
    hotspots: Vec<JsonHotspot>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsonSummary {
    projects: usize,
    files_scanned: usize,
    symbols: usize,
    high_complexity: usize,
    high_cognitive_complexity: usize,
    long_methods: usize,
    too_many_parameters: usize,
    large_files: usize,
    large_types: usize,
    circular_dependencies: usize,
    /// Omitted unless hotspots were requested.
    #[serde(skip_serializing_if = "Option::is_none")]
    commits_walked: Option<usize>,
    elapsed_ms: u128,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsonFinding {
    kind: HealthFindingKind,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    project: Option<String>,
    file: String,
    line: u32,
    column: u32,
    metric: u32,
    threshold: u32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsonCycleMember {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    project: Option<String>,
    file: String,
    line: u32,
    column: u32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsonCycle {
    members: Vec<JsonCycleMember>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsonHotspot {
    file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    project: Option<String>,
    score: f64,
    weighted_commits: f64,
    cyclomatic: u32,
    lines: u32,
    complexity_density: f64,
}

pub fn print(result: &HealthResult, workspace: &Workspace) {
    let report = JsonReport {
        version: 1,
        root: crate::paths::display(&workspace.root),
        summary: JsonSummary {
            projects: result.summary.projects,
            files_scanned: result.summary.files_scanned,
            symbols: result.summary.symbols,
            high_complexity: result.summary.high_complexity,
            high_cognitive_complexity: result.summary.high_cognitive_complexity,
            long_methods: result.summary.long_methods,
            too_many_parameters: result.summary.too_many_parameters,
            large_files: result.summary.large_files,
            large_types: result.summary.large_types,
            circular_dependencies: result.summary.circular_dependencies,
            commits_walked: result.summary.commits_walked,
            elapsed_ms: result.summary.elapsed_ms,
        },
        findings: result
            .findings
            .iter()
            .map(|finding| JsonFinding {
                kind: finding.kind,
                name: finding.name.clone(),
                project: finding.project.clone(),
                file: crate::paths::display(
                    finding
                        .file
                        .strip_prefix(&workspace.root)
                        .unwrap_or(&finding.file),
                ),
                line: finding.line,
                column: finding.column,
                metric: finding.metric,
                threshold: finding.threshold,
            })
            .collect(),
        cycles: result
            .cycles
            .iter()
            .map(|cycle| JsonCycle {
                members: cycle
                    .members
                    .iter()
                    .map(|member| JsonCycleMember {
                        name: member.name.clone(),
                        project: member.project.clone(),
                        file: crate::paths::display(
                            member
                                .file
                                .strip_prefix(&workspace.root)
                                .unwrap_or(&member.file),
                        ),
                        line: member.line,
                        column: member.column,
                    })
                    .collect(),
            })
            .collect(),
        hotspots: result
            .hotspots
            .iter()
            .map(|hotspot| JsonHotspot {
                file: crate::paths::display(
                    hotspot
                        .file
                        .strip_prefix(&workspace.root)
                        .unwrap_or(&hotspot.file),
                ),
                project: hotspot.project.clone(),
                score: round(hotspot.score, 1),
                weighted_commits: round(hotspot.weighted_commits, 2),
                cyclomatic: hotspot.cyclomatic,
                lines: hotspot.lines,
                complexity_density: round(hotspot.complexity_density, 4),
            })
            .collect(),
    };

    match serde_json::to_string_pretty(&report) {
        Ok(json) => println!("{json}"),
        Err(error) => eprintln!("error: failed to serialize JSON report: {error}"),
    }
}

/// Trim a score to a fixed number of decimals. The underlying values carry
/// far more precision than they mean — a hotspot score is a relative ranking,
/// not a measurement — and full float output makes the report unreadable and
/// impossible to diff.
fn round(value: f64, places: u32) -> f64 {
    let factor = 10_f64.powi(places as i32);

    (value * factor).round() / factor
}
