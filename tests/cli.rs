use assert_cmd::Command;

fn roe() -> Command {
    let mut cmd = Command::cargo_bin("roe").expect("binary builds");
    cmd.env("NO_COLOR", "1");
    cmd
}

fn fixture(name: &str) -> String {
    format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))
}

/// Timings and absolute paths vary by run/machine — pin them for snapshots.
fn normalize(output: &[u8]) -> String {
    let text = String::from_utf8_lossy(output);
    let ms = regex::Regex::new(r"\d+ ms").expect("valid regex");
    let elapsed = regex::Regex::new(r#""elapsedMs": \d+"#).expect("valid regex");
    let text = ms.replace_all(&text, "<n> ms");
    let text = elapsed.replace_all(&text, r#""elapsedMs": 0"#);
    // Report paths always use forward slashes, so normalize the repo path the
    // same way before replacing it — on Windows CARGO_MANIFEST_DIR has
    // backslashes.
    let repo = env!("CARGO_MANIFEST_DIR").replace('\\', "/");

    text.replace(&repo, "<repo>")
}

#[test]
fn findings_exit_code_1_and_human_output() {
    let output = roe()
        .args(["dead-code", &fixture("console_app")])
        .output()
        .expect("command runs");
    assert_eq!(output.status.code(), Some(1));
    insta::assert_snapshot!("human_console_app", normalize(&output.stdout));
}

#[test]
fn json_output_is_stable() {
    let output = roe()
        .args(["dead-code", &fixture("console_app"), "--format", "json"])
        .output()
        .expect("command runs");
    assert_eq!(output.status.code(), Some(1));

    let stdout = normalize(&output.stdout);
    // Must be valid JSON with the v1 schema markers.
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert_eq!(parsed["version"], 1);
    insta::assert_snapshot!("json_console_app", stdout);
}

#[test]
fn clean_codebase_exits_0() {
    let output = roe()
        .args(["dead-code", &fixture("generated")])
        .output()
        .expect("command runs");
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("no dead code found"));
}

#[test]
fn invalid_path_exits_2() {
    let output = roe()
        .args(["dead-code", "/definitely/not/a/real/path"])
        .output()
        .expect("command runs");
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("error:"));
}

#[test]
fn sln_path_argument_is_accepted() {
    let output = roe()
        .args([
            "dead-code",
            &format!("{}/ConsoleApp.sln", fixture("console_app")),
        ])
        .output()
        .expect("command runs");
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn config_ignore_json_filters_out_ignored_folder() {
    let output = roe()
        .args(["dead-code", &fixture("config_ignore_json")])
        .output()
        .expect("command runs");
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("StillDead.cs"));
    assert!(!stdout.contains("Ignored"));
}

#[test]
fn config_ignore_yaml_filters_out_ignored_folder() {
    let output = roe()
        .args(["dead-code", &fixture("config_ignore_yaml")])
        .output()
        .expect("command runs");
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("StillDead.cs"));
    assert!(!stdout.contains("Ignored"));
}

#[test]
fn config_resolution_walks_up_to_parent_directory() {
    // roe.json lives at the fixture root; --path points at a nested
    // subdirectory, so the ignore glob (relative to the config file's own
    // directory) must still resolve correctly against files under it.
    let output = roe()
        .args(["dead-code", &format!("{}/Sub", fixture("config_nested"))])
        .output()
        .expect("command runs");
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("StillDead.cs"));
    assert!(!stdout.contains("Ignored"));
}

#[test]
fn config_aggressive_true_takes_effect_without_cli_flag() {
    let output = roe()
        .args(["dead-code", &fixture("config_aggressive")])
        .output()
        .expect("command runs");
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Legacy"));
}

#[test]
fn library_flag_protects_a_named_project_despite_a_sibling_executable() {
    let output = roe()
        .args(["dead-code", &fixture("library_and_exe"), "--library", "Lib"])
        .output()
        .expect("command runs");
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("no dead code found"));
}

#[test]
fn explicit_config_flag_overrides_auto_discovery() {
    let fixture_root = fixture("config_explicit");
    let output = roe()
        .args([
            "dead-code",
            &format!("{fixture_root}/code"),
            "--config",
            &format!("{fixture_root}/config/roe.json"),
        ])
        .output()
        .expect("command runs");
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Legacy"));
}

#[test]
fn dupes_exact_clone_exit_code_1_and_human_output() {
    let output = roe()
        .args(["dupes", &fixture("dupes_exact_clone")])
        .output()
        .expect("command runs");
    assert_eq!(output.status.code(), Some(1));
    insta::assert_snapshot!("dupes_human_exact_clone", normalize(&output.stdout));
}

#[test]
fn dupes_no_code_hides_snippet() {
    let output = roe()
        .args(["dupes", &fixture("dupes_exact_clone"), "--no-code"])
        .output()
        .expect("command runs");
    assert_eq!(output.status.code(), Some(1));
    insta::assert_snapshot!("dupes_human_exact_clone_no_code", normalize(&output.stdout));
}

#[test]
fn dupes_semantic_mode_notes_representative_snippet() {
    let output = roe()
        .args([
            "dupes",
            &fixture("dupes_semantic_clone"),
            "--mode",
            "semantic",
        ])
        .output()
        .expect("command runs");
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("showing first occurrence"));
}

#[test]
fn dupes_json_output_is_stable() {
    let output = roe()
        .args(["dupes", &fixture("dupes_exact_clone"), "--format", "json"])
        .output()
        .expect("command runs");
    assert_eq!(output.status.code(), Some(1));

    let stdout = normalize(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert_eq!(parsed["version"], 1);
    assert_eq!(parsed["mode"], "exact");
    insta::assert_snapshot!("dupes_json_exact_clone", stdout);
}

#[test]
fn dupes_renamed_clone_is_invisible_in_exact_mode() {
    let output = roe()
        .args(["dupes", &fixture("dupes_semantic_clone")])
        .output()
        .expect("command runs");
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("no duplicate code found"));
}

#[test]
fn dupes_renamed_clone_is_found_with_semantic_mode() {
    let output = roe()
        .args([
            "dupes",
            &fixture("dupes_semantic_clone"),
            "--mode",
            "semantic",
        ])
        .output()
        .expect("command runs");
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("found 1 duplicate group"));
}

#[test]
fn dupes_clean_codebase_exits_0() {
    let output = roe()
        .args(["dupes", &fixture("dupes_no_duplicates")])
        .output()
        .expect("command runs");
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("no duplicate code found"));
}

#[test]
fn dupes_below_threshold_is_hidden_by_default_but_found_when_relaxed() {
    let clean = roe()
        .args(["dupes", &fixture("dupes_below_threshold")])
        .output()
        .expect("command runs");
    assert_eq!(clean.status.code(), Some(0));

    let relaxed = roe()
        .args([
            "dupes",
            &fixture("dupes_below_threshold"),
            "--min-tokens",
            "5",
            "--min-lines",
            "1",
        ])
        .output()
        .expect("command runs");
    assert_eq!(relaxed.status.code(), Some(1));
}

#[test]
fn dupes_config_ignore_json_drops_the_ignored_occurrence() {
    let output = roe()
        .args(["dupes", &fixture("dupes_config_ignore_json")])
        .output()
        .expect("command runs");
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("no duplicate code found"));
}

#[test]
fn dupes_config_ignore_yaml_drops_the_ignored_occurrence() {
    let output = roe()
        .args(["dupes", &fixture("dupes_config_ignore_yaml")])
        .output()
        .expect("command runs");
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("no duplicate code found"));
}

#[test]
fn dupes_invalid_path_exits_2() {
    let output = roe()
        .args(["dupes", "/definitely/not/a/real/path"])
        .output()
        .expect("command runs");
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("error:"));
}

#[test]
fn malformed_config_exits_2() {
    let output = roe()
        .args(["dead-code", &fixture("config_malformed")])
        .output()
        .expect("command runs");
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("error:"));
}

#[test]
fn health_clean_codebase_exits_0_with_default_thresholds() {
    let output = roe()
        .args(["health", &fixture("console_app")])
        .output()
        .expect("command runs");
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("no health issues found"));
}

#[test]
fn health_findings_exit_code_1_and_human_output() {
    let output = roe()
        .args([
            "health",
            &fixture("health_metrics"),
            "--max-cognitive",
            "2",
            "--max-complexity",
            "3",
            "--max-parameters",
            "3",
            "--max-type-members",
            "3",
        ])
        .output()
        .expect("command runs");
    assert_eq!(output.status.code(), Some(1));
    insta::assert_snapshot!("human_health_metrics", normalize(&output.stdout));
}

/// The threshold flags that make `health_metrics` produce findings, shared by
/// the ordering tests below so they differ only in what they are testing.
fn health_metrics_args() -> Vec<String> {
    [
        "health",
        &fixture("health_metrics"),
        "--max-cognitive",
        "2",
        "--max-complexity",
        "3",
        "--max-parameters",
        "3",
        "--max-type-members",
        "3",
    ]
    .iter()
    .map(|argument| argument.to_string())
    .collect()
}

#[test]
fn health_sort_path_orders_by_position_not_severity() {
    let mut args = health_metrics_args();
    args.extend(["--sort".to_string(), "path".to_string()]);

    let output = roe().args(&args).output().expect("command runs");
    assert_eq!(output.status.code(), Some(1));

    let stdout = normalize(&output.stdout);
    let widget = stdout.find("3:14").expect("Widget is reported");
    let branchy = stdout.find("5:17").expect("Branchy is reported");
    let many_params = stdout.find("17:17").expect("ManyParams is reported");
    assert!(
        widget < branchy && branchy < many_params,
        "--sort path must follow line order, got:\n{stdout}"
    );
}

#[test]
fn health_sort_severity_leads_with_the_worst_offender() {
    let output = roe()
        .args(health_metrics_args())
        .output()
        .expect("command runs");
    assert_eq!(output.status.code(), Some(1));

    // ManyParams is 2× its limit; Branchy is 1.5× and Widget 1.33×.
    let stdout = normalize(&output.stdout);
    let many_params = stdout.find("17:17").expect("ManyParams is reported");
    let branchy = stdout.find("5:17").expect("Branchy is reported");
    let widget = stdout.find("3:14").expect("Widget is reported");
    assert!(
        many_params < branchy && branchy < widget,
        "severity sort must put the worst first, got:\n{stdout}"
    );
}

#[test]
fn health_limit_caps_the_list_and_says_what_was_hidden() {
    let mut args = health_metrics_args();
    args.extend(["--limit".to_string(), "1".to_string()]);

    let output = roe().args(&args).output().expect("command runs");
    let stdout = normalize(&output.stdout);

    assert!(stdout.contains("17:17"), "the worst finding still prints");
    assert!(!stdout.contains("5:17"), "the rest are held back");
    assert!(
        stdout.contains("… and 2 more — re-run with --limit 0 to see all"),
        "the footer must name what was hidden, got:\n{stdout}"
    );
    // Truncation is presentation only — the summary still counts everything.
    assert!(stdout.contains("found 5 issues"));
}

#[test]
fn health_grouping_reports_one_entry_per_declaration() {
    let output = roe()
        .args(health_metrics_args())
        .output()
        .expect("command runs");
    let stdout = normalize(&output.stdout);

    // Branchy trips both complexity checks, but is listed once.
    assert_eq!(
        stdout.matches("HealthMetrics.Widget.Branchy").count(),
        1,
        "one declaration is one entry, got:\n{stdout}"
    );
    assert!(stdout.contains("cyclomatic 4/3 · cognitive 3/2"));
}

#[test]
fn health_json_output_is_stable() {
    let output = roe()
        .args([
            "health",
            &fixture("health_metrics"),
            "--format",
            "json",
            "--max-cognitive",
            "2",
            "--max-complexity",
            "3",
            "--max-parameters",
            "3",
            "--max-type-members",
            "3",
        ])
        .output()
        .expect("command runs");
    assert_eq!(output.status.code(), Some(1));

    let stdout = normalize(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert_eq!(parsed["version"], 1);
    insta::assert_snapshot!("json_health_metrics", stdout);
}

#[test]
fn health_prints_the_declared_signature_next_to_the_required_count() {
    // `Sum` declares sixteen parameters but asks the caller for two, so the
    // number the report leads with has to be explained or it reads as a bug.
    let output = roe()
        .args([
            "health",
            &fixture("health_optional_params"),
            "--max-parameters",
            "1",
        ])
        .output()
        .expect("command runs");

    assert_eq!(output.status.code(), Some(1));

    let stdout = normalize(&output.stdout);
    assert!(
        stdout.contains("2/1 params (16 declared: 2 required, 14 optional)"),
        "a defaulted-heavy signature must print its split, got:\n{stdout}"
    );
    assert!(
        stdout.contains("4/1 params (6 declared: 4 required, 2 out)"),
        "an out-heavy signature must print its split, got:\n{stdout}"
    );
    assert!(
        stdout.contains("6/1 params\n") || stdout.contains("6/1 params "),
        "an all-required signature has nothing to add, got:\n{stdout}"
    );
    assert!(
        !stdout.contains("6 declared: 6 required"),
        "saying it twice is noise, got:\n{stdout}"
    );
}

#[test]
fn health_json_carries_the_parameter_split() {
    let output = roe()
        .args([
            "health",
            &fixture("health_optional_params"),
            "--format",
            "json",
            "--max-parameters",
            "1",
        ])
        .output()
        .expect("command runs");

    assert_eq!(output.status.code(), Some(1));

    let stdout = normalize(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    let findings = parsed["findings"].as_array().expect("findings array");

    let try_get = findings
        .iter()
        .find(|finding| finding["name"] == "OptionalParams.Signatures.TryGet")
        .expect("TryGet is reported");

    assert_eq!(try_get["kind"], "too-many-parameters");
    assert_eq!(try_get["metric"], 4);
    assert_eq!(try_get["parameters"]["required"], 4);
    assert_eq!(try_get["parameters"]["optional"], 0);
    assert_eq!(try_get["parameters"]["out"], 2);
}

#[test]
fn health_reports_a_cycle_on_its_own() {
    // Nothing here trips a threshold, so the circular dependency is the only
    // thing left — and it alone has to fail the build and lead the report.
    let output = roe()
        .args([
            "health",
            &fixture("health_metrics"),
            // ManyParams sits one over the default parameter limit, and this
            // test is about what happens when a cycle is *all* there is.
            "--max-parameters",
            "6",
        ])
        .output()
        .expect("command runs");

    assert_eq!(output.status.code(), Some(1));

    let stdout = normalize(&output.stdout);
    assert!(
        !stdout.contains("no health issues found"),
        "a cycle is an issue, got:\n{stdout}"
    );
    assert!(
        stdout.starts_with("circular dependencies"),
        "with nothing printed above it, the section leads the report, got:\n{stdout}"
    );
    assert!(stdout.contains("found 1 issue across 0 locations in 0 files"));
}

#[test]
fn health_separates_files_with_a_blank_line_and_counts_every_kind() {
    // Two files, each with findings, and both a long method and large files in
    // the totals — the summed headline is otherwise indistinguishable from one
    // that drops the kinds that happen to be zero.
    let output = roe()
        .args([
            "health",
            &fixture("health_metrics"),
            "--max-complexity",
            "3",
            "--max-file-lines",
            "10",
            "--max-method-lines",
            "5",
            "--max-parameters",
            "6",
        ])
        .output()
        .expect("command runs");

    assert_eq!(output.status.code(), Some(1));

    let stdout = normalize(&output.stdout);
    assert!(
        stdout.starts_with("HealthMetrics/Widget.cs"),
        "the worst file leads, with no blank line above it, got:\n{stdout}"
    );
    assert!(
        stdout.contains("\n\nHealthMetrics/Cycle.cs"),
        "a blank line separates the next file, got:\n{stdout}"
    );
    // 1 complex method + 1 long method + 2 large files + 1 cycle.
    assert!(
        stdout.contains("found 5 issues across 3 locations in 2 files"),
        "every kind counts towards the headline, got:\n{stdout}"
    );
}

#[test]
fn health_footer_names_the_test_project_it_excluded() {
    // The issue: two runs over the same solution printed byte-identical
    // scanned counts, so the footer couldn't confirm the flag did anything.
    let included = roe()
        .args(["health", &fixture("with_tests"), "--max-type-members", "0"])
        .output()
        .expect("command runs");
    let included = normalize(&included.stdout);

    assert!(
        !included.contains("excluded:"),
        "nothing was excluded, so nothing is claimed, got:\n{included}"
    );

    let excluded = roe()
        .args([
            "health",
            &fixture("with_tests"),
            "--max-type-members",
            "0",
            "--exclude-tests",
        ])
        .output()
        .expect("command runs");
    let excluded = normalize(&excluded.stdout);

    assert!(
        excluded.contains("excluded: 1 test project (Lib.Tests)"),
        "the footer must name what it dropped, got:\n{excluded}"
    );

    let scanned = |report: &str| {
        report
            .lines()
            .find(|line| line.contains("scanned in"))
            .expect("a footer is printed")
            .to_string()
    };

    assert_ne!(
        scanned(&included),
        scanned(&excluded),
        "the scanned counts have to move when the flag does"
    );
}

#[test]
fn health_json_summary_agrees_with_the_human_footer() {
    let output = roe()
        .args([
            "health",
            &fixture("with_tests"),
            "--format",
            "json",
            "--max-type-members",
            "0",
            "--exclude-tests",
        ])
        .output()
        .expect("command runs");

    let stdout = normalize(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    let summary = &parsed["summary"];

    assert_eq!(summary["projects"], 1, "got {stdout}");
    assert_eq!(summary["excluded"]["testProjects"][0], "Lib.Tests");
    assert_eq!(summary["excluded"]["ignoredFiles"], 0);
}

#[test]
fn health_ignored_files_are_subtracted_from_the_scanned_count() {
    // The config-file case, which is the one with no command line to eyeball.
    let output = roe()
        .args([
            "health",
            &fixture("health_config_ignore"),
            "--format",
            "json",
            "--max-complexity",
            "2",
        ])
        .output()
        .expect("command runs");

    let stdout = normalize(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    let summary = &parsed["summary"];

    assert_eq!(summary["filesScanned"], 1, "got {stdout}");
    assert_eq!(summary["excluded"]["ignoredFiles"], 1);
    assert!(
        summary["excluded"]["testProjects"]
            .as_array()
            .expect("test projects array")
            .is_empty(),
        "no test project was excluded here, got {stdout}"
    );
}

#[test]
fn health_footer_counts_the_files_an_ignore_glob_dropped() {
    // An ignored file is the other half of the footer, and the half a reader
    // is least able to verify: no flag was typed, so the line is the only
    // evidence the glob matched anything at all.
    let output = roe()
        .args([
            "health",
            &fixture("health_config_ignore"),
            "--max-complexity",
            "2",
        ])
        .output()
        .expect("command runs");

    let stdout = normalize(&output.stdout);

    assert!(
        stdout.contains("excluded: 1 ignored file"),
        "the footer must count what the glob dropped, got:\n{stdout}"
    );
    assert!(
        !stdout.contains("test project"),
        "no test project was excluded here, got:\n{stdout}"
    );
}

#[test]
fn health_ignore_globs_drop_findings_and_the_cycles_they_touch() {
    let output = roe()
        .args([
            "health",
            &fixture("health_config_ignore"),
            "--format",
            "json",
            "--max-complexity",
            "2",
        ])
        .output()
        .expect("command runs");

    let stdout = normalize(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");

    let names: Vec<&str> = parsed["findings"]
        .as_array()
        .expect("findings array")
        .iter()
        .filter_map(|finding| finding["name"].as_str())
        .collect();
    assert_eq!(names, vec!["Lib.KeptWidget.Branchy"], "got {stdout}");

    // A cycle is dropped only when the ignore glob covers it — the one in
    // Kept.cs has to survive the one in Ignored.cs being removed.
    let cycles = parsed["cycles"].as_array().expect("cycles array");
    let members: Vec<&str> = cycles
        .iter()
        .flat_map(|cycle| cycle["path"].as_array().expect("path array"))
        .filter_map(|member| member["name"].as_str())
        .collect();
    assert_eq!(cycles.len(), 1, "got {stdout}");
    assert_eq!(
        members,
        vec!["Lib.KeptAlpha", "Lib.KeptBeta"],
        "got {stdout}"
    );
}

/// A writable path for a baseline the test owns. `CARGO_TARGET_TMPDIR` is
/// per-crate and cleaned by `cargo clean`, so nothing leaks into the fixtures.
fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("cli-baselines");
    std::fs::create_dir_all(&dir).expect("the scratch directory is writable");

    dir.join(name)
}

#[test]
fn health_write_baseline_records_the_run_and_exits_0() {
    let path = scratch("written.json");
    let mut args = health_metrics_args();
    args.extend(["--write-baseline".to_string(), path.display().to_string()]);

    let output = roe().args(&args).output().expect("command runs");
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert_eq!(
        output.status.code(),
        Some(0),
        "recording a codebase is not judging it, got {stderr}"
    );
    assert!(
        stderr.contains("wrote 4 finding(s) and 1 cycle(s) to "),
        "the write says what it recorded, got {stderr}"
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).is_empty(),
        "no report is printed alongside the write"
    );

    let written: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("the baseline exists"))
            .expect("valid JSON");
    assert_eq!(written["version"], 1);
    assert_eq!(
        written["findings"][0]["kind"], "high-complexity",
        "entries are sorted by kind, got {written}"
    );
}

#[test]
fn health_baseline_reports_only_what_it_does_not_cover() {
    let path = scratch("gate.json");
    let mut write = health_metrics_args();
    write.extend(["--write-baseline".to_string(), path.display().to_string()]);
    roe().args(&write).output().expect("command runs");

    let mut gate = health_metrics_args();
    gate.extend(["--baseline".to_string(), path.display().to_string()]);

    let clean = roe().args(&gate).output().expect("command runs");
    let stdout = String::from_utf8_lossy(&clean.stdout);
    assert_eq!(
        clean.status.code(),
        Some(0),
        "known debt does not fail the build, got:\n{stdout}"
    );
    assert!(
        stdout.contains("5 baselined finding(s) hidden"),
        "hidden findings are never silently gone, got:\n{stdout}"
    );

    // A tighter limit stands in for new code.
    gate.extend(["--max-method-lines".to_string(), "5".to_string()]);

    let regressed = roe().args(&gate).output().expect("command runs");
    let stdout = String::from_utf8_lossy(&regressed.stdout);
    assert_eq!(regressed.status.code(), Some(1));
    assert!(
        stdout.contains("found 1 issue across 1 location in 1 file"),
        "only the new finding is reported, got:\n{stdout}"
    );
    assert!(
        stdout.contains("5 baselined finding(s) hidden"),
        "and the rest are still accounted for, got:\n{stdout}"
    );
}

#[test]
fn health_json_summary_reports_what_the_baseline_hid() {
    let path = scratch("json-summary.json");
    let mut write = health_metrics_args();
    write.extend(["--write-baseline".to_string(), path.display().to_string()]);
    roe().args(&write).output().expect("command runs");

    let mut gate = health_metrics_args();
    gate.extend([
        "--baseline".to_string(),
        path.display().to_string(),
        "--format".to_string(),
        "json".to_string(),
    ]);

    let output = roe().args(&gate).output().expect("command runs");
    let stdout = normalize(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");

    assert_eq!(parsed["summary"]["baselined"], 5, "got {stdout}");
    assert!(
        parsed["findings"]
            .as_array()
            .expect("findings array")
            .is_empty(),
        "got {stdout}"
    );
}

#[test]
fn health_without_a_baseline_says_nothing_about_one() {
    let output = roe()
        .args(health_metrics_args())
        .output()
        .expect("command runs");
    let stdout = normalize(&output.stdout);

    assert!(
        !stdout.contains("baselined"),
        "a run with no baseline stays quiet about them, got:\n{stdout}"
    );
}

#[test]
fn health_reads_a_baseline_named_by_the_config() {
    // The config path is what makes a bare `roe` honour a baseline too, and
    // it resolves against the config file's own directory rather than the cwd.
    let output = roe()
        .args(["health", &fixture("health_config_baseline")])
        .output()
        .expect("command runs");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(output.status.code(), Some(1), "got:\n{stdout}");
    assert!(
        stdout.contains("Lib.Widget.New"),
        "the unrecorded finding is reported, got:\n{stdout}"
    );
    assert!(
        !stdout.contains("Lib.Widget.Known"),
        "the recorded one is not, got:\n{stdout}"
    );
    assert!(
        stdout.contains("1 baselined finding(s) hidden"),
        "got:\n{stdout}"
    );
}

#[test]
fn health_missing_baseline_exits_2_and_says_how_to_create_one() {
    let mut args = health_metrics_args();
    args.extend([
        "--baseline".to_string(),
        "does/not/exist/roe-baseline.json".to_string(),
    ]);

    let output = roe().args(&args).output().expect("command runs");
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr.contains(
            "error: baseline file not found: does/not/exist/roe-baseline.json — create one with `roe health --write-baseline does/not/exist/roe-baseline.json`"
        ),
        "got {stderr}"
    );
}

#[test]
fn health_refuses_to_write_a_baseline_through_a_baseline() {
    let mut args = health_metrics_args();
    args.extend([
        "--baseline".to_string(),
        "a.json".to_string(),
        "--write-baseline".to_string(),
        "b.json".to_string(),
    ]);

    let output = roe().args(&args).output().expect("command runs");

    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("cannot be used with"),
        "writing a filtered picture of the codebase is never what was meant"
    );
}

#[test]
fn health_invalid_path_exits_2() {
    let output = roe()
        .args(["health", "/definitely/not/a/real/path"])
        .output()
        .expect("command runs");
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("error:"));
}

#[test]
fn no_command_runs_all_three() {
    let output = roe()
        .args([&fixture("console_app")])
        .output()
        .expect("command runs");
    assert_eq!(output.status.code(), Some(1));
    insta::assert_snapshot!("check_human_console_app", normalize(&output.stdout));
}

#[test]
fn check_command_matches_bare_invocation() {
    let bare = roe()
        .args([&fixture("console_app")])
        .output()
        .expect("command runs");
    let explicit = roe()
        .args(["check", &fixture("console_app")])
        .output()
        .expect("command runs");

    assert_eq!(bare.status.code(), explicit.status.code());
    // Only the elapsed timings differ between two runs of the same analysis.
    assert_eq!(normalize(&bare.stdout), normalize(&explicit.stdout));
}

#[test]
fn check_defaults_to_the_current_directory() {
    let output = roe()
        .current_dir(fixture("console_app"))
        .output()
        .expect("command runs");
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("DeadClass.cs"), "got {stdout}");
}

#[test]
fn check_clean_codebase_exits_0() {
    let output = roe()
        .args([&fixture("generated")])
        .output()
        .expect("command runs");
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("no dead code found"), "got {stdout}");
    assert!(stdout.contains("no duplicate code found"), "got {stdout}");
    assert!(stdout.contains("no health issues found"), "got {stdout}");
}

/// Any one analysis reporting is enough to fail the whole run.
#[test]
fn check_exits_1_when_only_dupes_reports() {
    let output = roe()
        .args([&fixture("dupes_exact_clone")])
        .output()
        .expect("command runs");
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("no dead code found"), "got {stdout}");
    assert!(stdout.contains("2 occurrences"), "got {stdout}");
}

#[test]
fn check_json_is_a_single_document() {
    let output = roe()
        .args([&fixture("console_app"), "--format", "json"])
        .output()
        .expect("command runs");
    assert_eq!(output.status.code(), Some(1));

    let stdout = normalize(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert_eq!(parsed["version"], 1);
    // Each section is a standalone v1 report, so tooling written against the
    // individual commands can consume it unchanged.
    for section in ["deadCode", "dupes", "health"] {
        assert_eq!(parsed[section]["version"], 1, "{section} in {stdout}");
        assert!(parsed[section]["summary"].is_object(), "{section}");
    }
    insta::assert_snapshot!("check_json_console_app", stdout);
}

#[test]
fn check_invalid_path_exits_2() {
    let output = roe()
        .args(["/definitely/not/a/real/path"])
        .output()
        .expect("command runs");
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("error:"), "got {stderr}");
    // Looks like a path, so no command-list hint.
    assert!(!stderr.contains("hint:"), "got {stderr}");
}

/// A mistyped command name lands on the default run's positional path, so the
/// bare "path not found" error needs to point back at the command list.
#[test]
fn mistyped_command_hints_at_the_command_list() {
    let output = roe().args(["dupez"]).output().expect("command runs");
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("path not found: dupez"), "got {stderr}");
    assert!(stderr.contains("run `roe --help`"), "got {stderr}");
}

/// All three analyses walk the same tree and hit the same problems, so their
/// shared warnings must not be printed three times over.
#[test]
fn check_reports_each_warning_once() {
    let output = roe()
        .args([&fixture("console_app")])
        .output()
        .expect("command runs");
    let stderr = String::from_utf8_lossy(&output.stderr);
    let occurrences = stderr.matches("no obj/ directories found").count();
    assert_eq!(occurrences, 1, "got {stderr}");
}
