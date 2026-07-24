//! End-to-end hotspot tests over a throwaway git repository.
//!
//! Hotspots are the one part of `roe health` that reads git history, so they
//! can't be exercised from the checked-in fixtures — a fixture directory has
//! no history of its own. Each test builds a small repository, commits into
//! it in a controlled order, and runs the real binary against it.

use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;

use assert_cmd::Command;

/// A git repository under the system temp directory, removed on drop.
struct Repository {
    path: PathBuf,
}

impl Repository {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("roe-hotspots-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("temp directory is writable");

        let repository = Repository { path };

        repository.git(&["init", "--initial-branch=main"]);
        repository.git(&["config", "user.email", "test@example.com"]);
        repository.git(&["config", "user.name", "Test"]);
        repository.git(&["config", "commit.gpgsign", "false"]);

        repository.write(
            "App/App.csproj",
            "<Project Sdk=\"Microsoft.NET.Sdk\"><PropertyGroup>\
             <TargetFramework>net8.0</TargetFramework>\
             </PropertyGroup></Project>\n",
        );

        repository
    }

    fn git(&self, args: &[&str]) {
        let output = StdCommand::new("git")
            .args(args)
            .current_dir(&self.path)
            .output()
            .expect("git is installed and on PATH");

        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn write(&self, relative: &str, contents: &str) {
        let path = self.path.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("parent directory is writable");
        }
        std::fs::write(path, contents).expect("file is writable");
    }

    fn commit(&self, message: &str) {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-m", message]);
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Repository {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// A method whose body is one decision point per line — dense complexity in
/// few lines, which is exactly what the density term rewards.
fn dense_class(name: &str, branches: usize) -> String {
    let mut source =
        format!("namespace App;\n\npublic class {name}\n{{\n    public int Run(int n)\n    {{\n");
    for index in 0..branches {
        source.push_str(&format!(
            "        if (n == {index}) {{ return {index}; }}\n"
        ));
    }
    source.push_str("        return -1;\n    }\n}\n");

    source
}

fn health_json(root: &Path, extra: &[&str]) -> serde_json::Value {
    let mut command = Command::cargo_bin("roe").expect("binary builds");
    command.env("NO_COLOR", "1");
    command.args(["health", &root.display().to_string(), "--format", "json"]);
    command.args(extra);

    let output = command.output().expect("command runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    serde_json::from_str(&stdout).unwrap_or_else(|error| {
        panic!(
            "expected JSON on stdout, got {error}\nstdout: {stdout}\nstderr: {}",
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

#[test]
fn the_complex_and_frequently_changed_file_ranks_first() {
    let repository = Repository::new("ranking");

    repository.write("App/Calm.cs", &dense_class("Calm", 1));
    repository.write("App/Hot.cs", &dense_class("Hot", 12));
    repository.commit("initial");

    // Only Hot.cs keeps changing, so only Hot.cs accumulates churn. Each
    // revision must differ from the last or git refuses an empty commit.
    for revision in 1..5 {
        repository.write("App/Hot.cs", &dense_class("Hot", 12 + revision));
        repository.commit("touch Hot");
    }

    let report = health_json(repository.path(), &["--hotspots"]);
    let hotspots = report["hotspots"].as_array().expect("hotspots array");

    assert!(!hotspots.is_empty(), "expected a ranking, got {report}");
    assert_eq!(hotspots[0]["file"], "App/Hot.cs");
    assert_eq!(
        hotspots[0]["score"], 100.0,
        "the riskiest file should normalize to 100"
    );
    assert_eq!(report["summary"]["commitsWalked"], 5);
}

#[test]
fn hotspots_never_change_the_exit_code() {
    // Every codebase has a riskiest file. Failing a build over the existence
    // of a ranking would make the check useless as a CI gate.
    let repository = Repository::new("exit-code");

    repository.write("App/Hot.cs", &dense_class("Hot", 12));
    repository.commit("initial");
    repository.write("App/Hot.cs", &dense_class("Hot", 14));
    repository.commit("touch Hot");

    let mut command = Command::cargo_bin("roe").expect("binary builds");
    let output = command
        .env("NO_COLOR", "1")
        .args([
            "health",
            &repository.path().display().to_string(),
            // Thresholds nothing here can trip, so a ranking is the only thing
            // the exit code could possibly be reacting to.
            "--max-complexity",
            "1000",
            "--hotspots",
        ])
        .output()
        .expect("command runs");

    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("hotspots"),
        "the ranking should still be printed"
    );
}

#[test]
fn without_the_flag_no_history_is_read() {
    let repository = Repository::new("opt-in");

    repository.write("App/Hot.cs", &dense_class("Hot", 12));
    repository.commit("initial");

    let report = health_json(repository.path(), &[]);

    assert_eq!(report["hotspots"].as_array().expect("array").len(), 0);
    assert!(
        report["summary"].get("commitsWalked").is_none(),
        "commitsWalked should be absent when hotspots were not requested"
    );
}

#[test]
fn hotspots_outside_a_git_repository_fail_with_an_actionable_error() {
    let path = std::env::temp_dir().join(format!("roe-hotspots-nogit-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(path.join("App")).expect("temp directory is writable");
    std::fs::write(
        path.join("App/Program.cs"),
        "namespace App;\n\npublic class Program { }\n",
    )
    .expect("file is writable");

    let mut command = Command::cargo_bin("roe").expect("binary builds");
    let output = command
        .env("NO_COLOR", "1")
        .args(["health", &path.display().to_string(), "--hotspots"])
        .output()
        .expect("command runs");

    let stderr = String::from_utf8_lossy(&output.stderr);

    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr.contains("not a git repository"),
        "stderr should say what went wrong, got: {stderr}"
    );
    assert!(
        stderr.contains("--hotspots"),
        "stderr should say what to do about it, got: {stderr}"
    );

    let _ = std::fs::remove_dir_all(&path);
}
