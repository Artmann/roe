use std::path::PathBuf;

use roe::cli::DupeMode;
use roe::commands::dupes::analyze;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

#[test]
fn exact_clone_is_found_with_default_thresholds() {
    let analysis = analyze(&fixture("dupes_exact_clone"), DupeMode::Exact, 50, 5, 2)
        .expect("analysis should succeed");

    assert_eq!(analysis.result.groups.len(), 1);
    assert_eq!(analysis.result.groups[0].occurrences.len(), 2);
}

#[test]
fn renamed_clone_is_invisible_in_exact_mode() {
    let analysis = analyze(&fixture("dupes_semantic_clone"), DupeMode::Exact, 50, 5, 2)
        .expect("analysis should succeed");

    assert!(analysis.result.groups.is_empty());
}

#[test]
fn renamed_clone_is_found_in_semantic_mode() {
    let analysis = analyze(
        &fixture("dupes_semantic_clone"),
        DupeMode::Semantic,
        50,
        5,
        2,
    )
    .expect("analysis should succeed");

    assert_eq!(analysis.result.groups.len(), 1);
    assert_eq!(analysis.result.groups[0].occurrences.len(), 2);
}

#[test]
fn short_snippet_is_hidden_by_default_thresholds() {
    let analysis = analyze(&fixture("dupes_below_threshold"), DupeMode::Exact, 50, 5, 2)
        .expect("analysis should succeed");

    assert!(analysis.result.groups.is_empty());
}

#[test]
fn short_snippet_appears_once_thresholds_are_relaxed() {
    let analysis = analyze(&fixture("dupes_below_threshold"), DupeMode::Exact, 5, 1, 2)
        .expect("analysis should succeed");

    assert!(!analysis.result.groups.is_empty());
}

#[test]
fn distinct_files_report_no_duplicates() {
    let analysis = analyze(&fixture("dupes_no_duplicates"), DupeMode::Exact, 50, 5, 2)
        .expect("analysis should succeed");

    assert!(analysis.result.groups.is_empty());
}

#[test]
fn summary_counts_files_and_projects_scanned() {
    let analysis = analyze(&fixture("dupes_exact_clone"), DupeMode::Exact, 50, 5, 2)
        .expect("analysis should succeed");

    assert_eq!(analysis.result.summary.files_scanned, 2);
    assert_eq!(analysis.result.summary.projects, 1);
    assert_eq!(analysis.result.summary.groups, 1);
}
