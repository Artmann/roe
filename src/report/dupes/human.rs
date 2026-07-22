use std::path::Path;

use colored::Colorize;

use crate::cli::DupeMode;
use crate::model::{DupeGroup, DupesResult, Occurrence, Workspace};

pub fn print(result: &DupesResult, workspace: &Workspace, mode: DupeMode, show_code: bool) {
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
        print_group(group, workspace, mode, show_code);
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

fn print_group(group: &DupeGroup, workspace: &Workspace, mode: DupeMode, show_code: bool) {
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
        println!(
            "  {}:{}:{}-{}:{}",
            display_path(occurrence, workspace).display(),
            occurrence.start_line,
            occurrence.start_column,
            occurrence.end_line,
            occurrence.end_column
        );
    }

    if !show_code {
        return;
    }

    let Some(first) = group.occurrences.first() else {
        return;
    };

    println!();

    if mode == DupeMode::Semantic {
        println!(
            "  {}",
            "(showing first occurrence — identifiers and literals may differ in the other occurrences)"
                .dimmed()
        );
    }

    print_snippet(first, workspace);
}

fn print_snippet(occurrence: &Occurrence, workspace: &Workspace) {
    let display = display_path(occurrence, workspace);

    let source = match std::fs::read_to_string(&occurrence.file) {
        Ok(source) => source,
        Err(error) => {
            println!(
                "  {}",
                format!(
                    "(code not shown: could not read {}: {error} — the file may have changed since the scan)",
                    display.display()
                )
                .dimmed()
            );

            return;
        }
    };

    let lines = extract_lines(&source, occurrence.start_line, occurrence.end_line);

    if lines.is_empty() {
        println!(
            "  {}",
            format!(
                "(code not shown: {} now has fewer lines than when it was scanned — the file may have changed since the scan)",
                display.display()
            )
            .dimmed()
        );

        return;
    }

    let width = occurrence.end_line.to_string().len();

    for (offset, text) in lines.iter().enumerate() {
        let line_number = occurrence.start_line as usize + offset;

        println!("  {} {}", format!("{line_number:>width$} │").dimmed(), text);
    }

    let requested =
        (occurrence.end_line.max(occurrence.start_line) - occurrence.start_line) as usize + 1;

    if lines.len() < requested {
        println!(
            "  {}",
            format!(
                "({} now has fewer lines than when it was scanned — the file may have changed since the scan)",
                display.display()
            )
            .dimmed()
        );
    }
}

/// Slice `source` into full lines `start_line..=end_line` (1-based,
/// inclusive). Returns fewer lines than requested — or none — when the file
/// has shrunk since it was scanned.
fn extract_lines(source: &str, start_line: u32, end_line: u32) -> Vec<&str> {
    let start_index = (start_line.max(1) as usize) - 1;
    let count = (end_line.max(start_line) - start_line) as usize + 1;

    source.lines().skip(start_index).take(count).collect()
}

fn display_path<'a>(occurrence: &'a Occurrence, workspace: &Workspace) -> &'a Path {
    occurrence
        .file
        .strip_prefix(&workspace.root)
        .unwrap_or(&occurrence.file)
}

fn pluralize(count: usize, noun: &str) -> String {
    format!("{count} {noun}{}", if count == 1 { "" } else { "s" })
}

#[cfg(test)]
mod tests {
    use super::extract_lines;

    const SOURCE: &str = "one\ntwo\nthree\nfour\nfive\n";

    #[test]
    fn extracts_the_requested_range() {
        assert_eq!(extract_lines(SOURCE, 2, 4), vec!["two", "three", "four"]);
    }

    #[test]
    fn extracts_from_the_first_line() {
        assert_eq!(extract_lines(SOURCE, 1, 2), vec!["one", "two"]);
    }

    #[test]
    fn clamps_a_range_that_ends_past_the_end_of_the_file() {
        assert_eq!(extract_lines(SOURCE, 4, 10), vec!["four", "five"]);
    }

    #[test]
    fn returns_empty_when_the_range_starts_past_the_end_of_the_file() {
        assert!(extract_lines(SOURCE, 6, 10).is_empty());
    }
}
