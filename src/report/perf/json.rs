use serde::Serialize;

use crate::model::{Confidence, Cost, PerfFindingKind, PerfResult, Workspace};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct JsonReport {
    version: u32,
    root: String,
    summary: JsonSummary,
    findings: Vec<JsonFinding>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsonSummary {
    projects: usize,
    files_scanned: usize,
    findings: usize,
    /// How many findings decide the exit code. Everything else is advice.
    gating: usize,
    /// Findings per check, worst-scoring check first. Checks that found
    /// nothing are left out.
    counts: Vec<JsonCount>,
    /// Omitted when every check ran, so its presence means "this report is
    /// narrower than the catalog".
    #[serde(skip_serializing_if = "Vec::is_empty")]
    disabled_checks: Vec<PerfFindingKind>,
    elapsed_ms: u128,
    /// Omitted when nothing was excluded, so an all-zero object never reads as
    /// a claim that something was.
    #[serde(skip_serializing_if = "Option::is_none")]
    excluded: Option<JsonExcluded>,
}

/// What was ruled out before any check ran. The counts above are narrowed by
/// exactly this, so a consumer can reconcile the two.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsonExcluded {
    test_projects: Vec<String>,
    ignored_files: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsonCount {
    kind: PerfFindingKind,
    count: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsonFinding {
    kind: PerfFindingKind,
    confidence: Confidence,
    cost: Cost,
    /// Whether this finding on its own makes the command exit 1.
    gating: bool,
    /// Fully-qualified name of the member the finding sits in.
    member: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    project: Option<String>,
    file: String,
    line: u32,
    column: u32,
    /// Loops enclosing the finding inside its own member.
    loop_depth: u32,
    /// Estimated executions per call of that member, as a multiple.
    heat: f64,
    /// `cost × heat` — what the report is ranked by.
    score: f64,
    detail: String,
    fix: String,
}

pub fn print(result: &PerfResult, workspace: &Workspace) {
    let report = build(result, workspace);

    match serde_json::to_string_pretty(&report) {
        Ok(json) => println!("{json}"),
        Err(error) => eprintln!("error: failed to serialize JSON report: {error}"),
    }
}

/// The report as data rather than output, so a future combined run can nest it
/// instead of printing it on its own.
pub(crate) fn build(result: &PerfResult, workspace: &Workspace) -> JsonReport {
    let summary = &result.summary;

    JsonReport {
        version: 1,
        root: crate::paths::display(&workspace.root),
        summary: JsonSummary {
            projects: summary.projects,
            files_scanned: summary.files_scanned,
            findings: result.findings.len(),
            gating: summary.gating,
            counts: summary
                .counts
                .iter()
                .map(|&(kind, count)| JsonCount { kind, count })
                .collect(),
            disabled_checks: summary.disabled_checks.clone(),
            elapsed_ms: summary.elapsed_ms,
            excluded: build_excluded(result),
        },
        findings: result
            .findings
            .iter()
            .map(|finding| JsonFinding {
                kind: finding.kind,
                confidence: finding.confidence(),
                cost: finding.cost(),
                gating: finding.is_gating(),
                member: finding.member.clone(),
                project: finding.project.clone(),
                file: crate::paths::display(
                    finding
                        .file
                        .strip_prefix(&workspace.root)
                        .unwrap_or(&finding.file),
                ),
                line: finding.line,
                column: finding.column,
                loop_depth: finding.loop_depth,
                heat: finding.heat(),
                score: finding.score(),
                detail: finding.detail.clone(),
                fix: finding.fix.clone(),
            })
            .collect(),
    }
}

fn build_excluded(result: &PerfResult) -> Option<JsonExcluded> {
    let summary = &result.summary;

    if summary.excluded_test_projects.is_empty() && summary.excluded_files == 0 {
        return None;
    }

    Some(JsonExcluded {
        test_projects: summary.excluded_test_projects.clone(),
        ignored_files: summary.excluded_files,
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::model::{PerfFinding, PerfSummary};

    use super::*;

    fn report(findings: Vec<PerfFinding>) -> serde_json::Value {
        let result = PerfResult {
            findings,
            summary: PerfSummary::default(),
        };
        let workspace = Workspace {
            root: PathBuf::from("/repo"),
            projects: Vec::new(),
            files: Vec::new(),
            warnings: Vec::new(),
            missing_obj: false,
        };

        serde_json::to_value(build(&result, &workspace)).expect("serializes")
    }

    fn finding(kind: PerfFindingKind, loop_depth: u32) -> PerfFinding {
        PerfFinding {
            kind,
            member: "App.Service.Run".to_string(),
            project: None,
            file: PathBuf::from("/repo/Service.cs"),
            line: 10,
            column: 5,
            loop_depth,
            detail: "detail".to_string(),
            fix: "fix".to_string(),
        }
    }

    #[test]
    fn a_finding_carries_everything_needed_to_rank_it_elsewhere() {
        // A consumer that wants its own ordering must not have to re-derive
        // the cost table from the check name.
        let json = report(vec![finding(PerfFindingKind::NPlusOneQuery, 1)]);
        let first = &json["findings"][0];

        assert_eq!(first["kind"], "n-plus-one-query");
        assert_eq!(first["confidence"], "high");
        assert_eq!(first["cost"], "severe");
        assert_eq!(first["gating"], true);
        assert_eq!(first["loopDepth"], 1);
        assert_eq!(first["heat"], 10.0);
        assert_eq!(first["score"], 1000.0);
        assert_eq!(first["file"], "Service.cs");
    }

    #[test]
    fn a_medium_confidence_finding_is_marked_as_not_gating() {
        let json = report(vec![finding(PerfFindingKind::UnstableClock, 0)]);

        assert_eq!(json["findings"][0]["gating"], false);
    }

    #[test]
    fn a_plain_run_omits_the_exclusion_and_disabled_blocks() {
        let json = report(Vec::new());

        assert!(json["summary"].get("excluded").is_none());
        assert!(json["summary"].get("disabledChecks").is_none());
    }
}
