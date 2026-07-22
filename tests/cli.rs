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
    text.replace(env!("CARGO_MANIFEST_DIR"), "<repo>")
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
