use std::path::Path;

use colored::{ColoredString, Colorize};

use crate::model::{CircularDependency, HealthFinding, HealthFindingKind, HealthResult, Workspace};

pub fn print(result: &HealthResult, workspace: &Workspace) {
    if result.findings.is_empty() && result.cycles.is_empty() {
        println!(
            "{} no health issues found · {} project(s), {} file(s) scanned in {} ms",
            "✓".green().bold(),
            result.summary.projects,
            result.summary.files_scanned,
            result.summary.elapsed_ms
        );
        print_hotspots(result, workspace);
        return;
    }

    let mut current_file: Option<&Path> = None;
    for finding in &result.findings {
        if current_file != Some(finding.file.as_path()) {
            if current_file.is_some() {
                println!();
            }
            current_file = Some(finding.file.as_path());
            print_file_header(&finding.file, finding.project.as_deref(), workspace);
        }
        print_finding(finding);
    }

    if !result.cycles.is_empty() {
        if current_file.is_some() {
            println!();
        }
        println!("{}", "circular dependencies".bold());
        for cycle in &result.cycles {
            print_cycle(cycle, workspace);
        }
    }

    print_hotspots(result, workspace);

    println!();
    let s = &result.summary;
    let total_findings = s.high_complexity
        + s.high_cognitive_complexity
        + s.long_methods
        + s.too_many_parameters
        + s.large_files
        + s.large_types
        + s.circular_dependencies;
    println!(
        "{} {} — {} project(s), {} file(s), {} symbol(s) scanned in {} ms",
        "found".bold(),
        pluralize(total_findings, "issue").red().bold(),
        s.projects,
        s.files_scanned,
        s.symbols,
        s.elapsed_ms
    );
    println!(
        "  {} · {} · {} · {} · {} · {} · {}",
        pluralize(s.high_complexity, "complex method"),
        pluralize(s.high_cognitive_complexity, "hard-to-follow method"),
        pluralize(s.long_methods, "long method"),
        pluralize(s.too_many_parameters, "over-parameterized method"),
        pluralize(s.large_files, "large file"),
        pluralize(s.large_types, "large type"),
        pluralize_dependencies(s.circular_dependencies),
    );
}

/// The hotspot ranking, printed only when `--hotspots` asked for it.
///
/// Deliberately separated from the findings above by wording as well as
/// position: hotspots are not violations, they carry no threshold, and they
/// never move the exit code. They answer "where should I look first", not
/// "what is broken".
fn print_hotspots(result: &HealthResult, workspace: &Workspace) {
    let Some(commits_walked) = result.summary.commits_walked else {
        return;
    };

    println!();
    println!(
        "{} {}",
        "hotspots".bold(),
        format!("(complexity × churn over {commits_walked} commit(s))").dimmed()
    );

    if result.hotspots.is_empty() {
        println!(
            "  {}",
            "no file is both complex and actively changed".dimmed()
        );
        return;
    }

    for hotspot in &result.hotspots {
        let display = crate::paths::display(
            hotspot
                .file
                .strip_prefix(&workspace.root)
                .unwrap_or(&hotspot.file),
        );
        let detail = format!(
            "complexity {} over {} line(s), {:.1} weighted commit(s)",
            hotspot.cyclomatic, hotspot.lines, hotspot.weighted_commits
        );

        println!(
            "  {:>5}  {}  {}",
            score(hotspot.score),
            display,
            detail.dimmed()
        );
    }
}

/// Red at the top of the ranking, yellow through the middle, dim once the
/// score stops meaning much.
fn score(value: f64) -> ColoredString {
    let text = format!("{value:.0}");

    if value >= 60.0 {
        text.red().bold()
    } else if value >= 25.0 {
        text.yellow()
    } else {
        text.dimmed()
    }
}

fn print_file_header(file: &Path, project: Option<&str>, workspace: &Workspace) {
    let display = crate::paths::display(file.strip_prefix(&workspace.root).unwrap_or(file));

    match project {
        Some(project) => println!("{} {}", display.bold(), format!("({project})").dimmed()),
        None => println!("{}", display.bold()),
    }
}

fn print_finding(finding: &HealthFinding) {
    let location = format!("{}:{}", finding.line, finding.column);
    let (label, detail) = describe(finding);

    if finding.kind == HealthFindingKind::LargeFile {
        println!("  {:>7}  {}  {}", location.dimmed(), label, detail.dimmed());
    } else {
        println!(
            "  {:>7}  {}  {} {}",
            location.dimmed(),
            label,
            finding.name,
            detail.dimmed()
        );
    }
}

fn describe(finding: &HealthFinding) -> (ColoredString, String) {
    match finding.kind {
        HealthFindingKind::HighComplexity => (
            "high complexity".red().bold(),
            format!(
                "cyclomatic complexity {} (max {})",
                finding.metric, finding.threshold
            ),
        ),
        HealthFindingKind::HighCognitiveComplexity => (
            "hard to follow ".red().bold(),
            format!(
                "cognitive complexity {} (max {})",
                finding.metric, finding.threshold
            ),
        ),
        HealthFindingKind::LongMethod => (
            "long method    ".yellow(),
            format!("{} line(s) (max {})", finding.metric, finding.threshold),
        ),
        HealthFindingKind::TooManyParameters => (
            "too many params".yellow(),
            format!(
                "{} parameter(s) (max {})",
                finding.metric, finding.threshold
            ),
        ),
        HealthFindingKind::LargeFile => (
            "large file     ".yellow(),
            format!("{} line(s) (max {})", finding.metric, finding.threshold),
        ),
        HealthFindingKind::LargeType => (
            "large type     ".yellow(),
            format!("{} member(s) (max {})", finding.metric, finding.threshold),
        ),
    }
}

fn print_cycle(cycle: &CircularDependency, workspace: &Workspace) {
    let mut names: Vec<&str> = cycle
        .members
        .iter()
        .map(|member| member.name.as_str())
        .collect();
    if let Some(&first) = names.first() {
        names.push(first);
    }
    println!("  {}", names.join(" → ").red().bold());

    for member in &cycle.members {
        let display = crate::paths::display(
            member
                .file
                .strip_prefix(&workspace.root)
                .unwrap_or(&member.file),
        );
        println!("    {} {}:{}", display.dimmed(), member.line, member.column);
    }
}

fn pluralize(count: usize, noun: &str) -> String {
    format!("{count} {noun}{}", if count == 1 { "" } else { "s" })
}

fn pluralize_dependencies(count: usize) -> String {
    format!(
        "{count} circular {}",
        if count == 1 {
            "dependency"
        } else {
            "dependencies"
        }
    )
}
