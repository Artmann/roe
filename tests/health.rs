use std::path::PathBuf;

use roe::baseline;
use roe::commands::health::{Thresholds, analyze, analyze_with_baseline};
use roe::model::HealthFindingKind;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

/// A writable path for a baseline the test owns. `CARGO_TARGET_TMPDIR` is
/// per-crate and cleaned by `cargo clean`, so nothing leaks into the fixtures.
fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("baselines");
    std::fs::create_dir_all(&dir).expect("the scratch directory is writable");

    dir.join(name)
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
fn only_required_parameters_count_towards_the_limit() {
    // At a limit of 5, the sixteen-parameter method (two required) and the
    // six-parameter method (two of them `out`) both stay silent, while the
    // two methods with six genuinely required parameters fire.
    let mut thresholds = lenient();
    thresholds.max_parameters = 5;

    assert_findings(
        findings("health_optional_params", thresholds),
        vec![
            (
                HealthFindingKind::TooManyParameters,
                "OptionalParams.Extensions.Blend".to_string(),
            ),
            (
                HealthFindingKind::TooManyParameters,
                "OptionalParams.Signatures.Configure".to_string(),
            ),
        ],
    );
}

#[test]
fn a_parameter_finding_reports_the_metric_and_the_declared_signature() {
    // The metric is what tripped the limit; the breakdown is what the reader
    // needs in order to see why the two numbers differ.
    let mut thresholds = lenient();
    thresholds.max_parameters = 1;

    let analysis =
        analyze(&fixture("health_optional_params"), thresholds).expect("analysis should succeed");
    let by_name = |name: &str| {
        analysis
            .result
            .findings
            .iter()
            .find(|finding| finding.name == name)
            .unwrap_or_else(|| panic!("{name} should be flagged"))
    };

    let sum = by_name("OptionalParams.Signatures.Sum");
    let parameters = sum.parameters.expect("a parameter finding carries a split");
    assert_eq!(sum.metric, 2, "two required, not sixteen declared");
    assert_eq!(parameters.required, 2);
    assert_eq!(parameters.optional, 14);
    assert_eq!(parameters.out, 0);
    assert_eq!(parameters.total(), 16);

    let try_get = by_name("OptionalParams.Signatures.TryGet");
    let parameters = try_get
        .parameters
        .expect("a parameter finding carries a split");
    assert_eq!(try_get.metric, 4);
    assert_eq!(parameters.out, 2);

    // `Log(string message, params object[] arguments)` has two parameters in
    // the source and one at the call site, so even a limit of one leaves it
    // alone.
    assert!(
        !analysis
            .result
            .findings
            .iter()
            .any(|finding| finding.name == "OptionalParams.Signatures.Log"),
        "a params array is omittable, so Log asks for one parameter"
    );
}

#[test]
fn only_parameter_findings_carry_a_parameter_split() {
    // The split is additive to the v1 finding shape, so it must not leak on
    // to the kinds that never had it.
    let mut thresholds = lenient();
    thresholds.max_complexity = 3;
    thresholds.max_type_members = 3;

    let analysis =
        analyze(&fixture("health_metrics"), thresholds).expect("analysis should succeed");
    for finding in &analysis.result.findings {
        assert!(
            finding.parameters.is_none(),
            "{:?} should carry no parameter split, got {:?}",
            finding.kind,
            finding.parameters
        );
    }
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
fn const_fields_do_not_count_towards_a_types_size() {
    // Tuning is twenty-one consts and nothing else, so it stays silent even
    // at a limit of three. Mixed keeps its four non-const members and Statics
    // keeps its four `static readonly` fields, so both still trip.
    let mut thresholds = lenient();
    thresholds.max_type_members = 3;

    assert_findings(
        findings("health_const_registry", thresholds),
        vec![
            (
                HealthFindingKind::LargeType,
                "ConstRegistry.Mixed".to_string(),
            ),
            (
                HealthFindingKind::LargeType,
                "ConstRegistry.Statics".to_string(),
            ),
        ],
    );
}

#[test]
fn a_const_registry_is_silent_even_at_a_limit_of_zero() {
    // Not merely under the limit — excluded outright, the way enum cases are.
    let mut thresholds = lenient();
    thresholds.max_type_members = 0;

    let flagged = findings("health_const_registry", thresholds);
    assert!(
        !flagged
            .iter()
            .any(|(_, name)| name == "ConstRegistry.Tuning"),
        "a pure const registry has no countable members, got {flagged:?}"
    );
}

#[test]
fn a_large_types_metric_counts_only_what_the_breakdown_prints() {
    // The printed breakdown has to sum to the printed metric, or the report
    // is telling the reader two different numbers for the same type.
    let mut thresholds = lenient();
    thresholds.max_type_members = 3;

    let analysis =
        analyze(&fixture("health_const_registry"), thresholds).expect("analysis should succeed");
    let mixed = analysis
        .result
        .findings
        .iter()
        .find(|finding| finding.name == "ConstRegistry.Mixed")
        .expect("Mixed should be flagged");
    let breakdown = mixed.breakdown.expect("a large type carries a breakdown");

    assert_eq!(mixed.metric, 4, "one property, one field, two methods");
    assert_eq!(breakdown.total(), mixed.metric);
    assert_eq!(breakdown.fields, 1, "only the non-const field");
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
fn a_declaration_without_a_body_is_never_measured() {
    // An interface method has a parameter list but no implementation, so
    // every metric on it would be measuring nothing. Only the class that
    // implements it is reported — even though the two share a signature, and
    // so a parameter count.
    let mut thresholds = lenient();
    thresholds.max_parameters = 0;

    let names: Vec<String> = findings("console_app", thresholds)
        .into_iter()
        .map(|(_, name)| name)
        .collect();

    assert!(
        names.contains(&"ConsoleApp.ConsoleGreeter.Greet".to_string()),
        "the implementation is measured, got {names:?}"
    );
    assert!(
        !names
            .iter()
            .any(|name| name.starts_with("ConsoleApp.IGreeter")),
        "the interface declaration is not, got {names:?}"
    );
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
fn exclude_tests_narrows_the_scanned_counts_and_names_what_it_dropped() {
    // The footer is the only place a reader can confirm the flag took effect,
    // so it has to move when the flag does.
    let scan = |thresholds| {
        let analysis =
            analyze(&fixture("with_tests"), thresholds).expect("analysis should succeed");

        analysis.result.summary
    };

    // Absolute counts on both sides rather than "one is smaller than the
    // other": a bug that under-counts *every* run would satisfy the
    // comparison while getting both numbers wrong.
    let included = scan(lenient());
    assert!(included.excluded_test_projects.is_empty());
    assert_eq!(included.excluded_files, 0);
    assert_eq!(
        (included.projects, included.files_scanned),
        (2, 3),
        "with nothing excluded the fixture's own two projects and three files \
         are all in scope"
    );

    let mut thresholds = lenient();
    thresholds.exclude_tests = true;
    let excluded = scan(thresholds);

    assert_eq!(
        excluded.excluded_test_projects,
        vec!["Lib.Tests".to_string()],
        "naming the project answers 'was it even detected as a test project?'"
    );
    assert_eq!(
        (excluded.projects, excluded.files_scanned),
        (1, 2),
        "the flag drops Lib.Tests and its one file, and nothing else"
    );
    assert!(
        excluded.symbols < included.symbols,
        "{} symbols vs {}",
        excluded.symbols,
        included.symbols
    );
    assert!(
        excluded.symbols > 0,
        "Lib's own declarations are still in scope"
    );
}

#[test]
fn a_partial_type_with_a_generated_half_is_never_flagged() {
    // Partial declarations merge into one symbol and their flags merge with
    // them, so a designer file marks the whole type as generated. That is the
    // point: the member count is a property of the type, and roe cannot tell
    // which half of it a maintainer is free to change.
    let mut thresholds = lenient();
    thresholds.max_type_members = 2;

    assert_findings(
        findings("health_partial_generated", thresholds),
        vec![(HealthFindingKind::LargeType, "Lib.Plain".to_string())],
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
fn a_baseline_hides_what_it_recorded_and_reports_what_it_did_not() {
    // The whole point of the feature: adopt roe on a codebase that already
    // has debt, and only what lands after that day fails the build.
    let mut thresholds = lenient();
    thresholds.max_complexity = 3;

    let path = scratch("hides-what-it-recorded.json");
    let today = analyze(&fixture("health_metrics"), thresholds).expect("analysis should succeed");
    assert!(today.result.has_findings(), "there is debt to record");

    let (findings, cycles) = baseline::write(&path, &today.result, &today.workspace.root)
        .expect("the baseline is written");
    assert_eq!((findings, cycles), (1, 1));

    let unchanged = analyze_with_baseline(&fixture("health_metrics"), thresholds, &path)
        .expect("analysis should succeed");
    assert!(
        !unchanged.result.has_findings(),
        "recorded debt is not reported again, got {:?}",
        unchanged.result.findings
    );
    assert_eq!(unchanged.result.summary.baselined, Some(2));
    assert!(
        !unchanged
            .workspace
            .warnings
            .iter()
            .any(|warning| warning.contains("stale")),
        "every entry matched, so there is nothing to warn about, got {:?}",
        unchanged.workspace.warnings
    );

    // A tighter limit stands in for new code: the finding it produces is not
    // in the baseline, so it — and only it — comes back.
    let mut tighter = thresholds;
    tighter.max_type_members = 3;

    let after = analyze_with_baseline(&fixture("health_metrics"), tighter, &path)
        .expect("analysis should succeed");
    let names: Vec<&str> = after
        .result
        .findings
        .iter()
        .map(|finding| finding.name.as_str())
        .collect();

    assert_eq!(names, vec!["HealthMetrics.Widget"]);
    assert!(
        after.result.cycles.is_empty(),
        "the recorded cycle stays hidden"
    );
    assert_eq!(after.result.summary.baselined, Some(2));
}

#[test]
fn a_baseline_entry_that_matches_nothing_warns_without_failing() {
    let mut thresholds = lenient();
    thresholds.max_complexity = 3;

    let path = scratch("stale-entries.json");
    let today = analyze(&fixture("health_metrics"), thresholds).expect("analysis should succeed");
    baseline::write(&path, &today.result, &today.workspace.root).expect("the baseline is written");

    // Relaxing the limit stands in for the debt being paid down: the
    // complexity entry now covers nothing, while the cycle still matches.
    let fixed = analyze_with_baseline(&fixture("health_metrics"), lenient(), &path)
        .expect("analysis should succeed");

    assert!(!fixed.result.has_findings());
    assert!(
        fixed
            .workspace
            .warnings
            .iter()
            .any(|warning| warning.starts_with("1 stale entry in")
                && warning.contains("--write-baseline")),
        "a stale entry is named and made actionable, got {:?}",
        fixed.workspace.warnings
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
