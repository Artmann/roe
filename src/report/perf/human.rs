use std::path::Path;

use colored::{ColoredString, Colorize};

use crate::cli::PerfSort;
use crate::model::{Cost, PerfFinding, PerfResult, PerfSummary, Workspace};
use crate::report::{name_projects, pluralize};

/// Width of the check-name column. Wide enough for the longest label, so the
/// detail column starts in the same place on every row.
const LABEL_WIDTH: usize = 16;

/// Width of the `line:column` column, matching `roe health`'s.
const LOCATION_WIDTH: usize = 7;

pub fn print(result: &PerfResult, workspace: &Workspace, sort: PerfSort, limit: usize) {
    if result.findings.is_empty() {
        println!(
            "{} no quick wins found · {} project(s), {} file(s) scanned in {} ms",
            "✓".green().bold(),
            result.summary.projects,
            result.summary.files_scanned,
            result.summary.elapsed_ms
        );
        print_exclusions(&result.summary);
        print_disabled(&result.summary);

        return;
    }

    let mut findings: Vec<&PerfFinding> = result.findings.iter().collect();

    // The result arrives ranked worst-first. `--sort path` trades that for an
    // order that groups a file's findings together and never moves between
    // runs, which is what a reader working through one file at a time wants.
    if sort == PerfSort::Path {
        findings.sort_by(|a, b| {
            a.file
                .cmp(&b.file)
                .then((a.line, a.column).cmp(&(b.line, b.column)))
        });
    }

    let cap = if limit == 0 {
        findings.len()
    } else {
        limit.min(findings.len())
    };

    let mut previous_file: Option<&Path> = None;

    for (index, finding) in findings.iter().take(cap).enumerate() {
        if index > 0 {
            println!();
        }

        if previous_file != Some(finding.file.as_path()) {
            print_file_header(finding, workspace);
            previous_file = Some(finding.file.as_path());
        }

        print_finding(finding);
    }

    if cap < findings.len() {
        println!(
            "  {}",
            format!(
                "… and {} more — re-run with --limit 0 to see all",
                findings.len() - cap
            )
            .dimmed()
        );
    }

    println!();
    print_footer(result);
}

fn print_file_header(finding: &PerfFinding, workspace: &Workspace) {
    let display = crate::paths::display(
        finding
            .file
            .strip_prefix(&workspace.root)
            .unwrap_or(&finding.file),
    );

    match &finding.project {
        Some(project) => println!("{} {}", display.bold(), format!("({project})").dimmed()),
        None => println!("{}", display.bold()),
    }
}

/// Three lines per finding: what and where, how often it runs and how sure roe
/// is, and what to do about it.
///
/// The fix gets its own line rather than being folded into the detail because
/// it is the reason the command exists — a ranked list nobody can act on is
/// just a slower way to read the source.
fn print_finding(finding: &PerfFinding) {
    // Padded before colouring: ANSI escapes count towards a format width, so
    // aligning an already-coloured string silently under-pads it.
    let location = format!(
        "{:>LOCATION_WIDTH$}",
        format!("{}:{}", finding.line, finding.column)
    );
    let indent = " ".repeat(LOCATION_WIDTH);

    println!(
        "  {}  {}  {}",
        location.dimmed(),
        label(finding),
        finding.detail
    );
    println!(
        "  {}  {}",
        indent,
        format!(
            "~{}× per call · {}",
            finding.heat() as u64,
            finding.confidence().label()
        )
        .dimmed()
    );
    println!("  {}  {}", indent, format!("→ {}", finding.fix).dimmed());
}

/// Coloured by what the pattern costs, not by how sure roe is: confidence
/// decides the exit code and deliberately stays out of the ranking, so letting
/// it drive the colour would tell the eye to read the report in an order the
/// sort does not use.
fn label(finding: &PerfFinding) -> ColoredString {
    let text = format!("{:<LABEL_WIDTH$}", finding.kind.label());

    match finding.cost() {
        Cost::Severe => text.red().bold(),
        Cost::High => text.yellow(),
        Cost::Moderate => text.normal(),
    }
}

fn print_footer(result: &PerfResult) {
    let summary = &result.summary;
    let gating = if summary.gating == 0 {
        "none gating".normal()
    } else {
        format!("{} gating", summary.gating).red().bold()
    };

    println!(
        "{} {}, {} — {} project(s), {} file(s) scanned in {} ms",
        "found".bold(),
        pluralize(result.findings.len(), "quick win").bold(),
        gating,
        summary.projects,
        summary.files_scanned,
        summary.elapsed_ms
    );

    let breakdown: Vec<String> = summary
        .counts
        .iter()
        .map(|(kind, count)| format!("{count} {}", kind.label()))
        .collect();

    if !breakdown.is_empty() {
        println!("  {}", breakdown.join(" · "));
    }

    // `perf` is the one command whose exit code is not simply "did it find
    // anything", so a run that found plenty and still passes has to say why.
    if summary.gating == 0 {
        println!(
            "  {}",
            "exit 0 — only high-confidence findings fail a build".dimmed()
        );
    }

    print_exclusions(summary);
    print_disabled(summary);
}

/// What the run ruled out before any check could reach it.
///
/// Printed only when something actually was: a run with nothing excluded says
/// nothing, so the line's presence is itself the confirmation that a setting
/// took effect.
fn print_exclusions(summary: &PerfSummary) {
    let mut parts = Vec::new();

    if !summary.excluded_test_projects.is_empty() {
        parts.push(format!(
            "{} ({})",
            pluralize(summary.excluded_test_projects.len(), "test project"),
            name_projects(&summary.excluded_test_projects)
        ));
    }

    if summary.excluded_files > 0 {
        parts.push(pluralize(summary.excluded_files, "ignored file"));
    }

    if parts.is_empty() {
        return;
    }

    println!("  {}", format!("excluded: {}", parts.join(", ")).dimmed());
}

/// Which checks never ran, so "clean" cannot be confused with "not looked
/// for".
fn print_disabled(summary: &PerfSummary) {
    if summary.disabled_checks.is_empty() {
        return;
    }

    let names: Vec<&str> = summary
        .disabled_checks
        .iter()
        .map(|kind| kind.slug())
        .collect();

    println!("  {}", format!("disabled: {}", names.join(", ")).dimmed());
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::model::PerfFindingKind;
    use crate::report::forcing_color;

    use super::*;

    fn finding(kind: PerfFindingKind, loop_depth: u32) -> PerfFinding {
        PerfFinding {
            kind,
            member: "App.Service.Run".to_string(),
            project: None,
            file: PathBuf::from("Service.cs"),
            line: 10,
            column: 5,
            loop_depth,
            detail: String::new(),
            fix: String::new(),
        }
    }

    #[test]
    fn the_label_column_is_padded_to_a_fixed_width() {
        // Padded inside the colouring, so the detail column stays aligned once
        // the escapes are in the string.
        let _color = forcing_color();

        assert_eq!(
            label(&finding(PerfFindingKind::UnstableClock, 0)).to_string(),
            format!("{:<LABEL_WIDTH$}", "unstable clock")
                .normal()
                .to_string()
        );
    }

    #[test]
    fn a_severe_pattern_is_reddened_and_a_moderate_one_is_not() {
        let _color = forcing_color();

        let severe = label(&finding(PerfFindingKind::SyncOverAsync, 0)).to_string();
        let moderate = label(&finding(PerfFindingKind::UnstableClock, 0)).to_string();

        assert_eq!(
            severe,
            format!("{:<LABEL_WIDTH$}", "sync-over-async")
                .red()
                .bold()
                .to_string()
        );
        assert!(!moderate.contains("31m"), "moderate should not be red");
    }

    #[test]
    fn every_label_fits_the_column() {
        // A label wider than the column would push one row's detail out of
        // line with every other row's.
        for kind in PerfFindingKind::ALL {
            assert!(
                kind.label().len() <= LABEL_WIDTH,
                "'{}' is too wide for the label column",
                kind.label()
            );
        }
    }
}
