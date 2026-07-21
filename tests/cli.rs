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
fn malformed_config_exits_2() {
    let output = roe()
        .args(["dead-code", &fixture("config_malformed")])
        .output()
        .expect("command runs");
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("error:"));
}
