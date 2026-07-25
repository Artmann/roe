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
        exclude_tests: false,
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
fn a_metric_that_only_reaches_its_threshold_is_not_flagged() {
    // Every limit set to exactly what the fixture's worst declaration
    // measures: Branchy is cyclomatic 4, cognitive 3 and 10 lines long,
    // ManyParams takes 6 arguments, Widget has 4 members, and Widget.cs is 25
    // lines. A threshold is the most a declaration may be, not the least it
    // may not — so all of this is fine, and one-past-any of it is not.
    let at_the_limit = Thresholds {
        max_cognitive: 3,
        max_complexity: 4,
        max_method_lines: 10,
        max_parameters: 6,
        max_file_lines: 25,
        max_type_members: 4,
        exclude_tests: false,
    };

    assert_findings(findings("health_metrics", at_the_limit), vec![]);

    let one_under = |mutate: fn(&mut Thresholds)| {
        let mut thresholds = at_the_limit;
        mutate(&mut thresholds);

        findings("health_metrics", thresholds).len()
    };

    assert_eq!(one_under(|t| t.max_cognitive -= 1), 1, "cognitive");
    assert_eq!(one_under(|t| t.max_complexity -= 1), 1, "complexity");
    assert_eq!(one_under(|t| t.max_method_lines -= 1), 1, "method lines");
    assert_eq!(one_under(|t| t.max_parameters -= 1), 1, "parameters");
    assert_eq!(one_under(|t| t.max_file_lines -= 1), 1, "file lines");
    assert_eq!(one_under(|t| t.max_type_members -= 1), 1, "type members");
}

#[test]
fn overloads_are_told_apart_by_arity_and_nothing_else_is() {
    // Every member trips the complexity check, so the whole file is named.
    let mut thresholds = lenient();
    thresholds.max_complexity = 0;

    assert_findings(
        findings("health_overloads", thresholds),
        vec![
            (
                HealthFindingKind::HighComplexity,
                "Overloads.Mailer.Close".to_string(),
            ),
            // The static and the instance constructor share a name without
            // being overloads of each other, so neither is suffixed.
            (
                HealthFindingKind::HighComplexity,
                "Overloads.Mailer.Mailer".to_string(),
            ),
            (
                HealthFindingKind::HighComplexity,
                "Overloads.Mailer.Mailer".to_string(),
            ),
            (
                HealthFindingKind::HighComplexity,
                "Overloads.Mailer.Send/1".to_string(),
            ),
            (
                HealthFindingKind::HighComplexity,
                "Overloads.Mailer.Send/2".to_string(),
            ),
        ],
    );
}

#[test]
fn alpha_and_beta_form_a_circular_dependency() {
    // Cycles are reported regardless of thresholds — there is no knob that
    // turns them off, and lenient size limits must not hide them.
    let analysis = analyze(&fixture("health_metrics"), lenient()).expect("analysis should succeed");
    assert_eq!(analysis.result.cycles.len(), 1);

    let cycle = &analysis.result.cycles[0];
    assert!(
        cycle.others.is_empty(),
        "a two-type cycle has nothing off the path, got {:?}",
        cycle.others
    );

    let mut names: Vec<&str> = cycle
        .path
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
        exclude_tests: false,
    };

    let found = findings("health_metrics", thresholds);
    assert!(
        found
            .iter()
            .all(|(_, name)| !name.contains("GeneratedThing")),
        "generated declarations must never be flagged, got {found:?}"
    );
}

#[test]
fn exclude_tests_drops_test_project_declarations_only() {
    // Tight enough to catch the test project's one method as well as its one
    // class, so both the member-level and the type-level checks are covered.
    let mut thresholds = lenient();
    thresholds.max_method_lines = 3;
    thresholds.max_type_members = 0;

    let included = findings("with_tests", thresholds);
    assert!(
        included
            .iter()
            .any(|(_, name)| name == "Lib.Tests.CalculatorTests"),
        "the test class is flagged by default, got {included:?}"
    );
    assert!(
        included
            .iter()
            .any(|(_, name)| name == "Lib.Tests.CalculatorTests.Adds"),
        "and so is the test method, got {included:?}"
    );

    thresholds.exclude_tests = true;
    let excluded = findings("with_tests", thresholds);
    assert_findings(
        excluded,
        vec![
            (HealthFindingKind::LargeType, "Lib.Calculator".to_string()),
            (
                HealthFindingKind::LargeType,
                "Lib.UnusedInternal".to_string(),
            ),
        ],
    );
}

#[test]
fn exclude_tests_hides_a_cycle_inside_a_test_project() {
    // Cycles ignore every threshold, so `--exclude-tests` is the only thing
    // that can hide one — and it must hide only the test project's.
    let cycle_names = |thresholds: Thresholds| {
        let analysis =
            analyze(&fixture("health_test_cycle"), thresholds).expect("analysis should succeed");
        let mut names: Vec<String> = analysis
            .result
            .cycles
            .iter()
            .flat_map(|cycle| cycle.path.iter().chain(&cycle.others))
            .map(|member| member.name.clone())
            .collect();
        names.sort();

        names
    };

    assert_eq!(
        cycle_names(lenient()),
        vec![
            "Lib.Alpha",
            "Lib.Beta",
            "Lib.Tests.FakeAlpha",
            "Lib.Tests.FakeBeta"
        ]
    );

    let mut thresholds = lenient();
    thresholds.exclude_tests = true;

    assert_eq!(cycle_names(thresholds), vec!["Lib.Alpha", "Lib.Beta"]);
}

#[test]
fn a_scoped_marker_suppresses_only_the_rule_it_names() {
    let mut thresholds = lenient();
    thresholds.max_complexity = 3;
    thresholds.max_cognitive = 2;

    let found = findings("health_suppress", thresholds);

    // Scoped keeps its cognitive finding; Bare loses both; Unmarked keeps both.
    assert_findings(
        found,
        vec![
            (
                HealthFindingKind::HighCognitiveComplexity,
                "HealthSuppress.Widget.Scoped".to_string(),
            ),
            (
                HealthFindingKind::HighComplexity,
                "HealthSuppress.Widget.Unmarked".to_string(),
            ),
            (
                HealthFindingKind::HighCognitiveComplexity,
                "HealthSuppress.Widget.Unmarked".to_string(),
            ),
        ],
    );
}

#[test]
fn a_large_file_marker_is_honored_anywhere_in_the_file() {
    let mut thresholds = lenient();
    thresholds.max_file_lines = 5;

    let flagged: Vec<String> = findings("health_suppress", thresholds)
        .into_iter()
        .filter(|(kind, _)| *kind == HealthFindingKind::LargeFile)
        .map(|(_, name)| name)
        .collect();

    assert!(
        flagged.iter().any(|name| name.contains("Loud.cs")),
        "the unmarked file must still be reported, got {flagged:?}"
    );
    assert!(
        !flagged.iter().any(|name| name.contains("Padded.cs")),
        "the marked file must be suppressed, got {flagged:?}"
    );
}
