use std::path::Path;

use colored::Colorize;

use crate::model::{AnalysisResult, Finding, FindingKind, Workspace};

pub fn print(result: &AnalysisResult, workspace: &Workspace) {
    for note in &result.notes {
        println!("{} {note}", "note:".cyan().bold());
    }
    if !result.notes.is_empty() {
        println!();
    }

    if result.findings.is_empty() {
        println!(
            "{} no dead code found · {} project(s), {} file(s) scanned in {} ms",
            "✓".green().bold(),
            result.summary.projects,
            result.summary.files_scanned,
            result.summary.elapsed_ms
        );
        return;
    }

    let mut current_file: Option<&Path> = None;
    for finding in &result.findings {
        if current_file != Some(finding.file.as_path()) {
            if current_file.is_some() {
                println!();
            }
            current_file = Some(finding.file.as_path());
            let display = finding
                .file
                .strip_prefix(&workspace.root)
                .unwrap_or(&finding.file);
            match &finding.project {
                Some(project) => {
                    println!(
                        "{} {}",
                        display.display().to_string().bold(),
                        format!("({project})").dimmed()
                    );
                }
                None => println!("{}", display.display().to_string().bold()),
            }
        }
        print_finding(finding);
    }

    println!();
    let s = &result.summary;
    println!(
        "{} {} · {} · {} — {} project(s), {} file(s), {} symbol(s) scanned in {} ms",
        "found".bold(),
        pluralize(s.unused_files, "dead file").red().bold(),
        pluralize(s.unused_types, "unused type").red().bold(),
        pluralize(s.unused_members, "unused member").red().bold(),
        s.projects,
        s.files_scanned,
        s.symbols,
        s.elapsed_ms
    );
}

fn print_finding(finding: &Finding) {
    let location = format!("{}:{}", finding.line, finding.column);
    match finding.kind {
        FindingKind::UnusedFile => {
            println!(
                "  {:>7}  {}  {}",
                location.dimmed(),
                "dead file    ".red().bold(),
                "every declaration in this file is unused".dimmed()
            );
        }
        FindingKind::UnusedType | FindingKind::UnusedMember => {
            let label = if finding.kind == FindingKind::UnusedType {
                "unused type  ".yellow().bold()
            } else {
                "unused member".yellow()
            };
            let detail = match (finding.visibility, finding.symbol_kind) {
                (Some(v), Some(k)) => format!("({} {})", v.label(), k.label()),
                _ => String::new(),
            };
            println!(
                "  {:>7}  {}  {} {}",
                location.dimmed(),
                label,
                finding.name,
                detail.dimmed()
            );
        }
    }
}

fn pluralize(count: usize, noun: &str) -> String {
    format!("{count} {noun}{}", if count == 1 { "" } else { "s" })
}
