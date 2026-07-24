use std::path::PathBuf;

use roe::commands::health::{Thresholds, analyze};
use roe::model::HealthFindingKind;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

/// Loose thresholds nothing in the fixtures should trip.
fn lenient() -> Thresholds {
    Thresholds {
        max_cognitive: 100,
        max_complexity: 100,
        max_method_lines: 100,
        max_parameters: 100,
        max_file_lines: 1000,
        max_type_members: 100,
    }
}

/// Sorted (kind, name) pairs for stable assertions.
fn findings(name: &str, thresholds: Thresholds) -> Vec<(HealthFindingKind, String)> {
    let analysis = analyze(&fixture(name), thresholds).expect("analysis should succeed");
    let mut pairs: Vec<(HealthFindingKind, String)> = analysis
        .result
        .findings
        .iter()
        .map(|f| (f.kind, f.name.clone()))
        .collect();
    pairs.sort_by(|a, b| a.1.cmp(&b.1));
    pairs
}

fn assert_findings(
    actual: Vec<(HealthFindingKind, String)>,
    mut expected: Vec<(HealthFindingKind, String)>,
) {
    expected.sort_by(|a, b| a.1.cmp(&b.1));
    assert_eq!(actual, expected);
}

#[test]
fn nonexistent_path_errors() {
    assert!(analyze(&fixture("does_not_exist"), lenient()).is_err());
}

#[test]
fn lenient_thresholds_find_nothing() {
    assert_findings(findings("health_metrics", lenient()), vec![]);
}

#[test]
fn high_complexity_flags_the_branchy_method_only() {
    let mut thresholds = lenient();
    thresholds.max_complexity = 3;

    assert_findings(
        findings("health_metrics", thresholds),
        vec![(
            HealthFindingKind::HighComplexity,
            "HealthMetrics.Widget.Branchy".to_string(),
        )],
    );
}

#[test]
fn high_cognitive_complexity_flags_the_branchy_method_only() {
    // Branchy scores 3: the `if`, the flat `else if`, and the `&&`.
    let mut thresholds = lenient();
    thresholds.max_cognitive = 2;

    assert_findings(
        findings("health_metrics", thresholds),
        vec![(
            HealthFindingKind::HighCognitiveComplexity,
            "HealthMetrics.Widget.Branchy".to_string(),
        )],
    );
}

#[test]
fn too_many_parameters_flags_the_six_parameter_method_only() {
    let mut thresholds = lenient();
    thresholds.max_parameters = 3;

    assert_findings(
        findings("health_metrics", thresholds),
        vec![(
            HealthFindingKind::TooManyParameters,
            "HealthMetrics.Widget.ManyParams".to_string(),
        )],
    );
}

#[test]
fn large_type_flags_widget_which_has_four_members() {
    let mut thresholds = lenient();
    thresholds.max_type_members = 3;

    assert_findings(
        findings("health_metrics", thresholds),
        vec![(
            HealthFindingKind::LargeType,
            "HealthMetrics.Widget".to_string(),
        )],
    );
}

#[test]
fn large_file_flags_files_over_the_line_limit() {
    let mut thresholds = lenient();
    thresholds.max_file_lines = 5;

    let found = findings("health_metrics", thresholds);
    assert!(
        found
            .iter()
            .any(|(kind, _)| *kind == HealthFindingKind::LargeFile),
        "expected at least one large-file finding, got {found:?}"
    );
}

#[test]
fn alpha_and_beta_form_a_circular_dependency() {
    // Cycles are reported regardless of thresholds — there is no knob that
    // turns them off, and lenient size limits must not hide them.
    let analysis = analyze(&fixture("health_metrics"), lenient()).expect("analysis should succeed");
    assert_eq!(analysis.result.cycles.len(), 1);

    let mut names: Vec<&str> = analysis.result.cycles[0]
        .members
        .iter()
        .map(|member| member.name.as_str())
        .collect();
    names.sort_unstable();
    assert_eq!(names, vec!["HealthMetrics.Alpha", "HealthMetrics.Beta"]);
}

#[test]
fn generated_file_is_never_flagged() {
    // Even with thresholds tight enough to flag GeneratedThing.Complex many
    // times over, the generated file must produce nothing.
    let thresholds = Thresholds {
        max_cognitive: 1,
        max_complexity: 1,
        max_method_lines: 1,
        max_parameters: 1,
        max_file_lines: 1,
        max_type_members: 0,
    };

    let found = findings("health_metrics", thresholds);
    assert!(
        found
            .iter()
            .all(|(_, name)| !name.contains("GeneratedThing")),
        "generated declarations must never be flagged, got {found:?}"
    );
}
