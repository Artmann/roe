use std::path::{Path, PathBuf};

use assert_cmd::Command;

use roe::commands::perf::{Settings, analyze};
use roe::model::{Confidence, PerfFinding, PerfFindingKind};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn roe() -> Command {
    let mut command = Command::cargo_bin("roe").expect("binary builds");
    command.env("NO_COLOR", "1");

    command
}

/// Timings and absolute paths vary by run and by machine — pin them for
/// snapshots, the way `tests/cli.rs` does.
fn normalize(output: &[u8]) -> String {
    let text = String::from_utf8_lossy(output);
    let milliseconds = regex::Regex::new(r"\d+ ms").expect("valid regex");
    let elapsed = regex::Regex::new(r#""elapsedMs": \d+"#).expect("valid regex");
    let text = milliseconds.replace_all(&text, "<n> ms");
    let text = elapsed.replace_all(&text, r#""elapsedMs": 0"#);
    let repo = env!("CARGO_MANIFEST_DIR").replace('\\', "/");

    text.replace(&repo, "<repo>")
}

fn findings(name: &str, settings: &Settings) -> Vec<PerfFinding> {
    analyze(&fixture(name), settings)
        .expect("analysis should succeed")
        .result
        .findings
}

fn kinds(findings: &[PerfFinding]) -> Vec<PerfFindingKind> {
    let mut found: Vec<PerfFindingKind> = findings.iter().map(|finding| finding.kind).collect();
    found.sort();
    found.dedup();

    found
}

fn file_name(finding: &PerfFinding) -> &str {
    finding
        .file
        .file_name()
        .and_then(|name| name.to_str())
        .expect("every finding names a file")
}

#[test]
fn a_nonexistent_path_errors() {
    assert!(analyze(&fixture("does_not_exist"), &Settings::default()).is_err());
}

#[test]
fn every_check_in_the_catalog_fires_on_the_fixture() {
    // A check nobody exercises is a check nobody can tell has stopped working,
    // so the fixture owes the catalog one of each.
    assert_eq!(
        kinds(&findings("perf_catalog", &Settings::default())),
        PerfFindingKind::ALL.to_vec()
    );
}

#[test]
fn the_loop_variable_test_splits_the_two_database_findings() {
    // The two queries sit on consecutive lines and differ only in whether they
    // mention `order`. Getting this test the wrong way round would hand the
    // reader the wrong fix for both.
    let found = findings("perf_catalog", &Settings::default());
    let line_of = |kind: PerfFindingKind| {
        found
            .iter()
            .find(|finding| finding.kind == kind)
            .unwrap_or_else(|| panic!("{kind:?} should be reported"))
            .line
    };

    let n_plus_one = line_of(PerfFindingKind::NPlusOneQuery);
    let invariant = line_of(PerfFindingKind::LoopInvariantQuery);

    assert_eq!(
        invariant,
        n_plus_one + 1,
        "the query that mentions the loop variable comes first"
    );
    assert_eq!(
        found
            .iter()
            .filter(|finding| finding.kind == PerfFindingKind::NPlusOneQuery)
            .count(),
        1,
        "the invariant query must not also be reported as an n+1"
    );
}

#[test]
fn a_hashed_collection_is_never_flagged() {
    // `_blockedSkus.Contains` and `_allowedSkus.Contains` are the same call on
    // the same line shape; only the declaration tells them apart. This is the
    // whole premise of the declared-type map.
    let found = findings("perf_catalog", &Settings::default());
    let lookups: Vec<&str> = found
        .iter()
        .filter(|finding| finding.kind == PerfFindingKind::ListLookupInLoop)
        .map(|finding| finding.detail.as_str())
        .collect();

    assert_eq!(lookups.len(), 1, "got {lookups:?}");
    assert!(lookups[0].contains("_blockedSkus"), "got {lookups:?}");
}

#[test]
fn a_generated_file_is_never_scanned() {
    // Legacy.designer.cs holds a `.Result` — the most severe pattern in the
    // catalog — and must still produce nothing.
    let found = findings("perf_catalog", &Settings::default());

    assert!(
        found
            .iter()
            .all(|finding| file_name(finding) != "Legacy.designer.cs"),
        "generated code must never be reported, got {:?}",
        found.iter().map(file_name).collect::<Vec<_>>()
    );
}

#[test]
fn an_await_that_already_has_a_database_finding_is_not_reported_twice() {
    // Three of the four awaits in the loop are a database call. Only
    // `Task.Delay` is left for `await-in-loop`.
    let found = findings("perf_catalog", &Settings::default());
    let awaits: Vec<u32> = found
        .iter()
        .filter(|finding| finding.kind == PerfFindingKind::AwaitInLoop)
        .map(|finding| finding.line)
        .collect();

    assert_eq!(awaits.len(), 1, "got lines {awaits:?}");
}

#[test]
fn a_finding_is_named_after_the_member_it_sits_in() {
    // The member name is the join between the catalog pass and the walker's
    // declaration list; a wrong join shows up here and nowhere else.
    let found = findings("perf_catalog", &Settings::default());
    let member_of = |kind: PerfFindingKind| {
        found
            .iter()
            .find(|finding| finding.kind == kind)
            .unwrap_or_else(|| panic!("{kind:?} should be reported"))
            .member
            .clone()
    };

    assert_eq!(
        member_of(PerfFindingKind::NPlusOneQuery),
        "App.OrderReportService.BuildAsync"
    );
    assert_eq!(
        member_of(PerfFindingKind::SyncOverAsync),
        "App.HttpHelpers.Fetch"
    );
}

#[test]
fn the_hottest_finding_leads_the_report() {
    // `list-lookup-in-loop` and `n-plus-one-query` both cost `Severe` and both
    // sit one loop deep, so they tie at 1000 and fall back to position — which
    // is what the two `.Result`-grade findings at depth 0 have to lose to.
    let found = findings("perf_catalog", &Settings::default());
    let scores: Vec<f64> = found.iter().map(|finding| finding.score()).collect();

    assert!(
        scores.windows(2).all(|pair| pair[0] >= pair[1]),
        "the report must be ranked worst-first, got {scores:?}"
    );
    assert_eq!(found[0].score(), 1000.0);
    assert_eq!(found[0].kind, PerfFindingKind::NPlusOneQuery);
}

#[test]
fn filtering_to_high_confidence_keeps_only_what_can_fail_a_build() {
    let settings = Settings {
        min_confidence: Confidence::High,
        ..Settings::default()
    };
    let found = findings("perf_catalog", &settings);

    assert!(!found.is_empty());
    assert!(
        found.iter().all(|finding| finding.is_gating()),
        "got {:?}",
        kinds(&found)
    );
    assert!(
        !kinds(&found).contains(&PerfFindingKind::UnstableClock),
        "a medium-confidence check must be dropped"
    );
}

#[test]
fn a_disabled_check_produces_nothing_and_is_named_in_the_summary() {
    // "Clean" and "not looked for" must never read the same, so the summary
    // has to carry what was turned off.
    let settings = Settings {
        disabled_checks: vec![PerfFindingKind::SyncOverAsync],
        ..Settings::default()
    };
    let analysis = analyze(&fixture("perf_catalog"), &settings).expect("analysis should succeed");

    assert!(
        !kinds(&analysis.result.findings).contains(&PerfFindingKind::SyncOverAsync),
        "the disabled check must not fire"
    );
    assert_eq!(
        analysis.result.summary.disabled_checks,
        vec![PerfFindingKind::SyncOverAsync]
    );
}

#[test]
fn the_summary_counts_only_the_files_a_check_could_reach() {
    // Four files in the fixture, one of them generated. A footer that claimed
    // four would be claiming to have looked at code it skipped.
    let analysis =
        analyze(&fixture("perf_catalog"), &Settings::default()).expect("analysis should succeed");
    let summary = &analysis.result.summary;

    assert_eq!((summary.projects, summary.files_scanned), (1, 3));
    assert_eq!(summary.gating, 8, "the eight high-confidence findings");

    // The breakdown line is printed under the total, so the two have to agree.
    let counted: usize = summary.counts.iter().map(|(_, count)| count).sum();
    assert_eq!(counted, analysis.result.findings.len());
}

#[test]
fn human_output_is_stable() {
    let output = roe()
        .args(["perf", &fixture("perf_catalog").display().to_string()])
        .output()
        .expect("command runs");

    assert_eq!(output.status.code(), Some(1));
    insta::assert_snapshot!("perf_human_catalog", normalize(&output.stdout));
}

#[test]
fn sorting_by_path_groups_each_file_once() {
    // The default ranking interleaves files, so `--sort path` is the only way
    // to read a report one file at a time.
    let output = roe()
        .args([
            "perf",
            &fixture("perf_catalog").display().to_string(),
            "--sort",
            "path",
        ])
        .output()
        .expect("command runs");

    assert_eq!(output.status.code(), Some(1));
    insta::assert_snapshot!("perf_human_catalog_by_path", normalize(&output.stdout));
}

#[test]
fn json_output_is_stable() {
    let output = roe()
        .args([
            "perf",
            &fixture("perf_catalog").display().to_string(),
            "--format",
            "json",
        ])
        .output()
        .expect("command runs");

    assert_eq!(output.status.code(), Some(1));

    let stdout = normalize(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert_eq!(parsed["version"], 1);

    insta::assert_snapshot!("perf_json_catalog", stdout);
}

#[test]
fn a_limit_truncates_the_report_and_says_so() {
    let output = roe()
        .args([
            "perf",
            &fixture("perf_catalog").display().to_string(),
            "--limit",
            "2",
        ])
        .output()
        .expect("command runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        stdout.contains("--limit 0 to see all"),
        "a truncated report has to say it was truncated, got:\n{stdout}"
    );
    assert!(
        stdout.contains("found 14 quick wins"),
        "the footer counts what was found, not what was printed, got:\n{stdout}"
    );
}

#[test]
fn only_medium_confidence_findings_exit_0() {
    // The promise that makes the command safe to adopt: advice prints, but it
    // does not fail anyone's build.
    let output = roe()
        .args(["perf", &fixture("perf_medium_only").display().to_string()])
        .output()
        .expect("command runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert_eq!(output.status.code(), Some(0));
    assert!(
        stdout.contains("none gating"),
        "the footer has to explain the passing exit code, got:\n{stdout}"
    );
    assert!(
        stdout.contains("exit 0 — only high-confidence findings fail a build"),
        "got:\n{stdout}"
    );
}

#[test]
fn one_sync_over_async_turns_the_same_project_red() {
    // Same project, one line of new code — the difference between advice and a
    // failed build has to be exactly that line.
    let root = copy_fixture("perf_medium_only", "gating");
    std::fs::write(
        root.join("Advice/Blocking.cs"),
        "namespace Advice;\n\
         \n\
         public class Blocking\n\
         {\n\
             public string Fetch(System.Threading.Tasks.Task<string> pending)\n\
             {\n\
                 return pending.Result;\n\
             }\n\
         }\n",
    )
    .expect("the scratch fixture is writable");

    let output = roe()
        .args(["perf", &root.display().to_string()])
        .output()
        .expect("command runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert_eq!(output.status.code(), Some(1));
    assert!(stdout.contains("1 gating"), "got:\n{stdout}");
}

/// A writable copy of a fixture, for the tests that need to add a file to one.
/// `CARGO_TARGET_TMPDIR` is per-crate and cleaned by `cargo clean`, so nothing
/// leaks back into `tests/fixtures`.
fn copy_fixture(name: &str, into: &str) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("perf")
        .join(into);

    if root.exists() {
        std::fs::remove_dir_all(&root).expect("the scratch directory is writable");
    }

    copy_tree(&fixture(name), &root);

    root
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("the scratch directory is writable");

    for entry in std::fs::read_dir(from).expect("the fixture is readable") {
        let entry = entry.expect("the fixture is readable");
        let target = to.join(entry.file_name());

        if entry.path().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).expect("the scratch directory is writable");
        }
    }
}
