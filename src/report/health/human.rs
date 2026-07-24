use std::path::Path;

use colored::{ColoredString, Colorize};

use crate::cli::HealthSort;
use crate::model::{CircularDependency, HealthFinding, HealthFindingKind, HealthResult, Workspace};

/// Every check one declaration tripped, printed as a single entry.
///
/// A method that is too long, too branchy, and too deeply nested is one thing
/// to fix, not three. Listing it three times buries everything else in the
/// report and makes the codebase read as worse than it is — so the findings
/// stay one-per-check in [`HealthResult`] (JSON and the exit code are
/// unchanged) and are folded together only here, on the way to the terminal.
struct Group<'a> {
    line: u32,
    column: u32,
    name: &'a str,
    findings: Vec<&'a HealthFinding>,
}

impl Group<'_> {
    /// A group is as bad as its worst check.
    fn severity(&self) -> f64 {
        self.findings
            .iter()
            .map(|finding| finding.severity())
            .fold(0.0, f64::max)
    }
}

struct FileGroup<'a> {
    file: &'a Path,
    project: Option<&'a str>,
    groups: Vec<Group<'a>>,
}

impl FileGroup<'_> {
    fn severity(&self) -> f64 {
        self.groups.iter().map(Group::severity).fold(0.0, f64::max)
    }
}

pub fn print(result: &HealthResult, workspace: &Workspace, sort: HealthSort, limit: usize) {
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

    let mut files = group(&result.findings);
    if sort == HealthSort::Severity {
        sort_by_severity(&mut files);
    }

    let total_groups: usize = files.iter().map(|file| file.groups.len()).sum();
    let cap = if limit == 0 {
        total_groups
    } else {
        limit.min(total_groups)
    };

    let mut printed = 0;
    for file in &files {
        if printed >= cap {
            break;
        }
        if printed > 0 {
            println!();
        }

        print_file_header(file.file, file.project, workspace);

        for group in &file.groups {
            if printed >= cap {
                break;
            }

            print_group(group);
            printed += 1;
        }
    }

    if printed < total_groups {
        println!(
            "  {}",
            format!(
                "… and {} more — re-run with --limit 0 to see all",
                total_groups - printed
            )
            .dimmed()
        );
    }

    if !result.cycles.is_empty() {
        if printed > 0 {
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
        "{} {} across {} in {} — {} project(s), {} file(s), {} symbol(s) scanned in {} ms",
        "found".bold(),
        pluralize(total_findings, "issue").red().bold(),
        pluralize(total_groups, "location"),
        pluralize(files.len(), "file"),
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

/// Fold the flat finding list into files and, within them, declarations.
///
/// `findings` arrives sorted by `(file, line, column)`, and a line/column pair
/// identifies exactly one declaration, so both levels can be built by
/// comparing against the entry just pushed — no map, and the input order is
/// preserved for `--sort path`.
fn group(findings: &[HealthFinding]) -> Vec<FileGroup<'_>> {
    let mut files: Vec<FileGroup<'_>> = Vec::new();

    for finding in findings {
        let same_file = files
            .last()
            .is_some_and(|file| file.file == finding.file.as_path());

        if !same_file {
            files.push(FileGroup {
                file: &finding.file,
                project: finding.project.as_deref(),
                groups: Vec::new(),
            });
        }

        let Some(file) = files.last_mut() else {
            continue;
        };

        let same_declaration = file.groups.last().is_some_and(|group| {
            group.line == finding.line
                && group.column == finding.column
                && group.name == finding.name
        });

        if same_declaration {
            if let Some(group) = file.groups.last_mut() {
                group.findings.push(finding);
            }
        } else {
            file.groups.push(Group {
                line: finding.line,
                column: finding.column,
                name: &finding.name,
                findings: vec![finding],
            });
        }
    }

    files
}

/// Worst first, at both levels, so the thing most worth fixing leads the
/// report instead of whatever happens to sort first alphabetically. Ties fall
/// back to position, which keeps the output stable across runs.
fn sort_by_severity(files: &mut [FileGroup<'_>]) {
    for file in files.iter_mut() {
        file.groups.sort_by(|a, b| {
            b.severity()
                .total_cmp(&a.severity())
                .then((a.line, a.column).cmp(&(b.line, b.column)))
        });
    }

    files.sort_by(|a, b| {
        b.severity()
            .total_cmp(&a.severity())
            .then(a.file.cmp(b.file))
    });
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

fn print_group(group: &Group<'_>) {
    // Padded before colouring: ANSI escapes count towards a format width, so
    // aligning a already-coloured string silently under-pads it.
    let location = format!("{:>7}", format!("{}:{}", group.line, group.column));
    let checks: Vec<String> = group.findings.iter().map(|f| describe(f)).collect();
    let detail = checks.join(" · ");

    // A large file has no symbol to name, and the header directly above it
    // already says which file it is — so there is nothing to put on a first
    // line.
    let is_file_level = group
        .findings
        .iter()
        .all(|finding| finding.kind == HealthFindingKind::LargeFile);

    if is_file_level {
        println!("  {}  {}", location.dimmed(), detail);
        return;
    }

    println!("  {}  {}", location.dimmed(), group.name);
    println!("  {}  {}", " ".repeat(7), detail);
}

/// `metric/threshold noun`, uniform across kinds so several checks read as one
/// line. Coloured by how far past the limit the value sits rather than by
/// which check it is — 46 branches against a limit of 10 is a different
/// problem from 11 against the same limit, and the old per-kind colouring
/// could not say so.
fn describe(finding: &HealthFinding) -> String {
    let headline = match finding.kind {
        HealthFindingKind::HighComplexity => {
            format!("cyclomatic {}/{}", finding.metric, finding.threshold)
        }
        HealthFindingKind::HighCognitiveComplexity => {
            format!("cognitive {}/{}", finding.metric, finding.threshold)
        }
        HealthFindingKind::LongMethod => {
            format!("{}/{} lines", finding.metric, finding.threshold)
        }
        HealthFindingKind::TooManyParameters => {
            format!("{}/{} params", finding.metric, finding.threshold)
        }
        HealthFindingKind::LargeFile => {
            format!("large file {}/{} lines", finding.metric, finding.threshold)
        }
        HealthFindingKind::LargeType => {
            format!("{}/{} members", finding.metric, finding.threshold)
        }
    };

    let colored = if finding.severity() >= 2.0 {
        headline.red().bold()
    } else {
        headline.yellow()
    };

    // The breakdown is context, not a verdict: thirty auto-properties and
    // thirty methods trip the same threshold and mean entirely different
    // things, so it is printed plainly and left to the reader.
    match finding.breakdown {
        Some(breakdown) => format!(
            "{colored} {}",
            format!("({})", breakdown.describe()).dimmed()
        ),
        None => colored.to_string(),
    }
}

fn print_cycle(cycle: &CircularDependency, workspace: &Workspace) {
    let mut names: Vec<&str> = cycle
        .path
        .iter()
        .map(|member| member.name.as_str())
        .collect();
    if let Some(&first) = names.first() {
        names.push(first);
    }
    println!("  {}", names.join(" → ").red().bold());

    for member in &cycle.path {
        let display = crate::paths::display(
            member
                .file
                .strip_prefix(&workspace.root)
                .unwrap_or(&member.file),
        );
        println!("    {} {}:{}", display.dimmed(), member.line, member.column);
    }

    // Same tangle, but not on the arrow chain above — printing them inline
    // would claim edges between them that may not exist.
    if !cycle.others.is_empty() {
        let names: Vec<&str> = cycle
            .others
            .iter()
            .map(|member| member.name.as_str())
            .collect();

        let count = cycle.others.len();

        println!(
            "    {}",
            format!(
                "+ {count} more {} in this cycle: {}",
                if count == 1 { "type" } else { "types" },
                names.join(", ")
            )
            .dimmed()
        );
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
