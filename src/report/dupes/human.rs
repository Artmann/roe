use colored::Colorize;

use crate::model::{DupeGroup, DupesResult, Workspace};

pub fn print(result: &DupesResult, workspace: &Workspace) {
    if result.groups.is_empty() {
        println!(
            "{} no duplicate code found · {} project(s), {} file(s) scanned in {} ms",
            "✓".green().bold(),
            result.summary.projects,
            result.summary.files_scanned,
            result.summary.elapsed_ms
        );
        return;
    }

    for (index, group) in result.groups.iter().enumerate() {
        if index > 0 {
            println!();
        }
        print_group(group, workspace);
    }

    println!();
    let s = &result.summary;
    println!(
        "{} {} · {} — {} project(s), {} file(s) scanned in {} ms",
        "found".bold(),
        pluralize(s.groups, "duplicate group").red().bold(),
        pluralize(s.duplicated_lines, "duplicated line"),
        s.projects,
        s.files_scanned,
        s.elapsed_ms
    );
}

fn print_group(group: &DupeGroup, workspace: &Workspace) {
    println!(
        "{} {}",
        pluralize(group.occurrences.len(), "occurrence")
            .yellow()
            .bold(),
        format!(
            "({} tokens, {})",
            group.token_count,
            pluralize(group.line_count as usize, "line")
        )
        .dimmed()
    );
    for occurrence in &group.occurrences {
        let display = occurrence
            .file
            .strip_prefix(&workspace.root)
            .unwrap_or(&occurrence.file);
        println!(
            "  {}:{}:{}-{}:{}",
            display.display(),
            occurrence.start_line,
            occurrence.start_column,
            occurrence.end_line,
            occurrence.end_column
        );
    }
}

fn pluralize(count: usize, noun: &str) -> String {
    format!("{count} {noun}{}", if count == 1 { "" } else { "s" })
}
