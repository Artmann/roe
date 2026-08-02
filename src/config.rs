use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use globset::{Glob, GlobSet, GlobSetBuilder};
use serde::Deserialize;

use crate::cli::DupeMode;

/// User-authored suppression/override settings, loaded from `roe.json`,
/// `roe.yaml`, or `roe.yml`.
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoeConfig {
    pub aggressive: Option<bool>,
    pub roots: Option<Vec<String>>,
    /// Project names to always treat in library mode (public API is used),
    /// regardless of executables elsewhere in the workspace.
    #[serde(rename = "libraryProjects")]
    pub library_projects: Option<Vec<String>>,
    /// Glob patterns (relative to this config file's directory) whose
    /// matching files have all their findings suppressed. A pattern ending in
    /// `/` also matches everything under that directory.
    pub ignore: Option<Vec<String>>,
    /// Defaults for `roe dupes`' matching mode and thresholds, so a combined
    /// `roe check` — which takes no dupes flags — can be calibrated.
    pub dupes: Option<DupesConfig>,
    /// Defaults for `roe health`'s thresholds, so a CI invocation doesn't have
    /// to repeat six flags.
    pub health: Option<HealthConfig>,
}

/// The `health` block of a config file. Every field is optional; an absent
/// one falls back to the built-in default.
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct HealthConfig {
    pub max_complexity: Option<u32>,
    pub max_cognitive: Option<u32>,
    pub max_method_lines: Option<u32>,
    pub max_parameters: Option<u32>,
    pub max_file_lines: Option<u32>,
    pub max_type_members: Option<u32>,
    pub exclude_tests: Option<bool>,
    /// Path to a baseline file, relative to this config file's own directory
    /// the way `ignore` globs are.
    pub baseline: Option<String>,
}

/// The `dupes` block of a config file. Every field is optional; an absent one
/// falls back to the built-in default.
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct DupesConfig {
    pub min_lines: Option<u32>,
    pub min_occurrences: Option<u32>,
    pub min_tokens: Option<u32>,
    pub mode: Option<DupeMode>,
}

pub struct ResolvedConfig {
    pub path: PathBuf,
    /// The config file's own directory — the base `ignore` globs resolve
    /// against.
    pub dir: PathBuf,
    pub config: RoeConfig,
}

const CANDIDATES: [&str; 3] = ["roe.json", "roe.yaml", "roe.yml"];

/// Nearest-config resolution: starting at `start` (a directory), check for
/// `roe.json`, then `roe.yaml`, then `roe.yml`; if none is present, walk up
/// to the parent directory and repeat until one is found or the filesystem
/// root is reached. `Ok(None)` means nothing was found anywhere up the tree;
/// a config that IS found but fails to parse is a hard error.
pub fn discover(
    start: &Path,
    warnings: &mut Vec<String>,
) -> anyhow::Result<Option<ResolvedConfig>> {
    let mut dir = start.to_path_buf();
    loop {
        if let Some(resolved) = find_in_dir(&dir, warnings)? {
            return Ok(Some(resolved));
        }
        match dir.parent() {
            Some(parent) => dir = parent.to_path_buf(),
            None => return Ok(None),
        }
    }
}

fn find_in_dir(dir: &Path, warnings: &mut Vec<String>) -> anyhow::Result<Option<ResolvedConfig>> {
    let existing: Vec<PathBuf> = CANDIDATES
        .iter()
        .map(|name| dir.join(name))
        .filter(|path| path.is_file())
        .collect();
    let Some(chosen) = existing.first() else {
        return Ok(None);
    };
    if existing.len() > 1 {
        let others: Vec<String> = existing[1..]
            .iter()
            .map(|p| p.display().to_string())
            .collect();
        warnings.push(format!(
            "multiple config files found in {}; using {} (ignoring {})",
            dir.display(),
            chosen.display(),
            others.join(", ")
        ));
    }
    load_file(chosen).map(Some)
}

/// Load an explicit `--config` path. Both a missing file and a parse failure
/// are hard errors — the user pointed at this file deliberately, so silently
/// ignoring a typo would hide the fact suppression isn't actually applied.
pub fn load_explicit(path: &Path) -> anyhow::Result<ResolvedConfig> {
    let path = crate::paths::canonicalize(path)
        .with_context(|| format!("config file not found: {}", path.display()))?;
    load_file(&path)
}

fn load_file(path: &Path) -> anyhow::Result<ResolvedConfig> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read config file: {}", path.display()))?;
    let config = parse(path, &content)
        .with_context(|| format!("failed to parse config file: {}", path.display()))?;
    let dir = path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    Ok(ResolvedConfig {
        path: path.to_path_buf(),
        dir,
        config,
    })
}

fn parse(path: &Path, content: &str) -> anyhow::Result<RoeConfig> {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("json") => Ok(serde_json::from_str(content)?),
        Some("yaml") | Some("yml") => Ok(serde_yaml_ng::from_str(content)?),
        other => bail!("unsupported config extension: {other:?} (expected .json, .yaml, or .yml)"),
    }
}

/// Resolved settings after applying default → config file → CLI flag
/// precedence.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EffectiveArgs {
    pub aggressive: bool,
    pub roots: Vec<String>,
    pub library_projects: Vec<String>,
}

/// `aggressive` is OR'd because clap's plain bool flag can only ever be
/// explicitly `true` — there's no `--no-aggressive` to override a config's
/// `true` back to `false`. `roots` and `library_projects`, when passed on the
/// CLI, replace the config's list wholesale rather than merging with it.
pub fn merge(
    config: Option<&RoeConfig>,
    cli_aggressive: bool,
    cli_roots: &[String],
    cli_library_projects: &[String],
) -> EffectiveArgs {
    let config_aggressive = config.and_then(|c| c.aggressive).unwrap_or(false);
    let roots = if !cli_roots.is_empty() {
        cli_roots.to_vec()
    } else {
        config.and_then(|c| c.roots.clone()).unwrap_or_default()
    };
    let library_projects = if !cli_library_projects.is_empty() {
        cli_library_projects.to_vec()
    } else {
        config
            .and_then(|c| c.library_projects.clone())
            .unwrap_or_default()
    };
    EffectiveArgs {
        aggressive: cli_aggressive || config_aggressive,
        roots,
        library_projects,
    }
}

/// `roe health` settings after applying default → config file → CLI flag
/// precedence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EffectiveHealth {
    pub max_complexity: u32,
    pub max_cognitive: u32,
    pub max_method_lines: u32,
    pub max_parameters: u32,
    pub max_file_lines: u32,
    pub max_type_members: u32,
    pub exclude_tests: bool,
}

impl Default for EffectiveHealth {
    /// `max_complexity` of 10 traces back to McCabe's 1976 paper; the rest sit
    /// near the values common C# linters use.
    fn default() -> Self {
        EffectiveHealth {
            max_complexity: 10,
            max_cognitive: 15,
            max_method_lines: 40,
            max_parameters: 5,
            max_file_lines: 750,
            max_type_members: 20,
            exclude_tests: false,
        }
    }
}

/// `roe dupes` settings after applying default → config file → CLI flag
/// precedence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EffectiveDupes {
    pub min_lines: u32,
    pub min_occurrences: u32,
    pub min_tokens: u32,
    pub mode: DupeMode,
}

impl Default for EffectiveDupes {
    /// The same values the `roe dupes` help text spells out as `[default: …]`.
    fn default() -> Self {
        EffectiveDupes {
            min_lines: 5,
            min_occurrences: 2,
            min_tokens: 50,
            mode: DupeMode::Exact,
        }
    }
}

/// The same three-way fallback [`merge_health`] applies: an explicit CLI flag
/// wins, then the config file's value, then the built-in default. Nothing
/// here is OR'd the way `aggressive` and `exclude_tests` are — `--mode`
/// carries a value, so an explicit `--mode exact` overrides a config's
/// `"semantic"` cleanly.
pub fn merge_dupes(config: Option<&DupesConfig>, cli: DupesOverrides) -> EffectiveDupes {
    let defaults = EffectiveDupes::default();
    let pick = |from_cli: Option<u32>, from_config: Option<u32>, fallback: u32| {
        from_cli.or(from_config).unwrap_or(fallback)
    };

    EffectiveDupes {
        min_lines: pick(
            cli.min_lines,
            config.and_then(|c| c.min_lines),
            defaults.min_lines,
        ),
        min_occurrences: pick(
            cli.min_occurrences,
            config.and_then(|c| c.min_occurrences),
            defaults.min_occurrences,
        ),
        min_tokens: pick(
            cli.min_tokens,
            config.and_then(|c| c.min_tokens),
            defaults.min_tokens,
        ),
        mode: cli
            .mode
            .or(config.and_then(|c| c.mode))
            .unwrap_or(defaults.mode),
    }
}

/// Threshold precedence is a plain three-way fallback, which the `Option`
/// typing on both sides makes explicit: an explicit CLI flag wins, then the
/// config file's value, then the built-in default.
///
/// `exclude_tests` is OR'd rather than overridden, for the same reason
/// `aggressive` is: clap's plain bool flag can only ever be `true`, so there
/// is no `--no-exclude-tests` to override a config's `true` back to `false`.
pub fn merge_health(config: Option<&HealthConfig>, cli: HealthOverrides) -> EffectiveHealth {
    let defaults = EffectiveHealth::default();
    let pick = |from_cli: Option<u32>, from_config: Option<u32>, fallback: u32| {
        from_cli.or(from_config).unwrap_or(fallback)
    };

    EffectiveHealth {
        max_complexity: pick(
            cli.max_complexity,
            config.and_then(|c| c.max_complexity),
            defaults.max_complexity,
        ),
        max_cognitive: pick(
            cli.max_cognitive,
            config.and_then(|c| c.max_cognitive),
            defaults.max_cognitive,
        ),
        max_method_lines: pick(
            cli.max_method_lines,
            config.and_then(|c| c.max_method_lines),
            defaults.max_method_lines,
        ),
        max_parameters: pick(
            cli.max_parameters,
            config.and_then(|c| c.max_parameters),
            defaults.max_parameters,
        ),
        max_file_lines: pick(
            cli.max_file_lines,
            config.and_then(|c| c.max_file_lines),
            defaults.max_file_lines,
        ),
        max_type_members: pick(
            cli.max_type_members,
            config.and_then(|c| c.max_type_members),
            defaults.max_type_members,
        ),
        exclude_tests: cli.exclude_tests || config.and_then(|c| c.exclude_tests).unwrap_or(false),
    }
}

/// Which baseline file, if any, this run filters against.
///
/// Kept out of [`EffectiveHealth`] because a `PathBuf` is not `Copy` and the
/// thresholds are passed around by value everywhere; the precedence is the
/// same three-way fallback [`merge_health`] applies.
///
/// A config-file path resolves against the config's own directory, so
/// `"roe-baseline.json"` means the one sitting next to `roe.json` no matter
/// which subdirectory the run started in. A `--baseline` path is left exactly
/// as typed, since the shell that typed it resolved it against the working
/// directory already.
pub fn resolve_baseline(
    config: Option<&HealthConfig>,
    config_dir: Option<&Path>,
    cli: Option<&Path>,
) -> Option<PathBuf> {
    if let Some(path) = cli {
        return Some(path.to_path_buf());
    }

    let named = config.and_then(|health| health.baseline.as_deref())?;

    Some(config_dir.unwrap_or(Path::new("")).join(named))
}

/// The subset of `DupesArgs` that participates in config merging — passed as
/// a struct so the three same-typed thresholds can't be transposed at the
/// call site.
#[derive(Debug, Clone, Copy, Default)]
pub struct DupesOverrides {
    pub min_lines: Option<u32>,
    pub min_occurrences: Option<u32>,
    pub min_tokens: Option<u32>,
    pub mode: Option<DupeMode>,
}

/// The subset of `HealthArgs` that participates in config merging — passed as
/// a struct so the six same-typed thresholds can't be transposed at the call
/// site.
#[derive(Debug, Clone, Copy, Default)]
pub struct HealthOverrides {
    pub max_complexity: Option<u32>,
    pub max_cognitive: Option<u32>,
    pub max_method_lines: Option<u32>,
    pub max_parameters: Option<u32>,
    pub max_file_lines: Option<u32>,
    pub max_type_members: Option<u32>,
    pub exclude_tests: bool,
}

/// Builds a `GlobSet` from config-relative `ignore` glob patterns, resolved
/// against `config_dir` (the config file's own directory). A trailing slash
/// on a pattern reads as "this whole directory" without the user having to
/// spell out `**` themselves. Returns `None` if no pattern successfully
/// builds a glob (nothing to match against).
pub(crate) fn build_ignore_globset(
    config_dir: &Path,
    patterns: &[String],
    warnings: &mut Vec<String>,
) -> Option<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    let mut any = false;
    for pattern in patterns {
        if pattern.contains("..") {
            warnings.push(format!("unsupported ignore glob with '..': {pattern}"));
            continue;
        }
        let expanded = match pattern.strip_suffix('/') {
            Some(dir) => format!("{dir}/**"),
            None => pattern.clone(),
        };
        let absolute = format!("{}/{}", config_dir.display(), expanded);
        match Glob::new(&absolute) {
            Ok(glob) => {
                builder.add(glob);
                any = true;
            }
            Err(error) => warnings.push(format!("invalid ignore glob {pattern}: {error}")),
        }
    }
    if !any {
        return None;
    }
    builder.build().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_prefers_cli_aggressive_true_over_config() {
        let config = RoeConfig {
            aggressive: Some(false),
            ..Default::default()
        };
        assert!(merge(Some(&config), true, &[], &[]).aggressive);
    }

    #[test]
    fn merge_falls_back_to_config_aggressive() {
        let config = RoeConfig {
            aggressive: Some(true),
            ..Default::default()
        };
        assert!(merge(Some(&config), false, &[], &[]).aggressive);
    }

    #[test]
    fn merge_defaults_aggressive_to_false() {
        assert!(!merge(None, false, &[], &[]).aggressive);
    }

    #[test]
    fn merge_cli_roots_override_config_roots() {
        let config = RoeConfig {
            roots: Some(vec!["Config.Root".to_string()]),
            ..Default::default()
        };
        let cli_roots = vec!["Cli.Root".to_string()];
        assert_eq!(
            merge(Some(&config), false, &cli_roots, &[]).roots,
            vec!["Cli.Root".to_string()]
        );
    }

    #[test]
    fn merge_falls_back_to_config_roots() {
        let config = RoeConfig {
            roots: Some(vec!["Config.Root".to_string()]),
            ..Default::default()
        };
        assert_eq!(
            merge(Some(&config), false, &[], &[]).roots,
            vec!["Config.Root".to_string()]
        );
    }

    #[test]
    fn merge_defaults_roots_to_empty() {
        assert!(merge(None, false, &[], &[]).roots.is_empty());
    }

    #[test]
    fn merge_cli_library_projects_override_config_library_projects() {
        let config = RoeConfig {
            library_projects: Some(vec!["Config.Lib".to_string()]),
            ..Default::default()
        };
        let cli_library_projects = vec!["Cli.Lib".to_string()];
        assert_eq!(
            merge(Some(&config), false, &[], &cli_library_projects).library_projects,
            vec!["Cli.Lib".to_string()]
        );
    }

    #[test]
    fn merge_falls_back_to_config_library_projects() {
        let config = RoeConfig {
            library_projects: Some(vec!["Config.Lib".to_string()]),
            ..Default::default()
        };
        assert_eq!(
            merge(Some(&config), false, &[], &[]).library_projects,
            vec!["Config.Lib".to_string()]
        );
    }

    #[test]
    fn merge_defaults_library_projects_to_empty() {
        assert!(merge(None, false, &[], &[]).library_projects.is_empty());
    }

    #[test]
    fn merge_health_prefers_the_cli_threshold_over_the_config() {
        let config = HealthConfig {
            max_complexity: Some(25),
            ..Default::default()
        };
        let cli = HealthOverrides {
            max_complexity: Some(7),
            ..Default::default()
        };
        assert_eq!(merge_health(Some(&config), cli).max_complexity, 7);
    }

    #[test]
    fn merge_health_falls_back_to_the_config_threshold() {
        let config = HealthConfig {
            max_complexity: Some(25),
            ..Default::default()
        };
        assert_eq!(
            merge_health(Some(&config), HealthOverrides::default()).max_complexity,
            25
        );
    }

    #[test]
    fn merge_health_falls_back_to_the_built_in_defaults() {
        assert_eq!(
            merge_health(None, HealthOverrides::default()),
            EffectiveHealth::default()
        );
    }

    #[test]
    fn merge_health_resolves_each_threshold_independently() {
        // A config that sets only one threshold must not reset the others.
        let config = HealthConfig {
            max_file_lines: Some(2000),
            ..Default::default()
        };
        let cli = HealthOverrides {
            max_parameters: Some(3),
            ..Default::default()
        };
        let effective = merge_health(Some(&config), cli);

        assert_eq!(effective.max_file_lines, 2000);
        assert_eq!(effective.max_parameters, 3);
        assert_eq!(
            effective.max_cognitive,
            EffectiveHealth::default().max_cognitive
        );
    }

    #[test]
    fn merge_health_ors_exclude_tests() {
        let config = HealthConfig {
            exclude_tests: Some(false),
            ..Default::default()
        };
        let cli = HealthOverrides {
            exclude_tests: true,
            ..Default::default()
        };
        assert!(merge_health(Some(&config), cli).exclude_tests);

        let config = HealthConfig {
            exclude_tests: Some(true),
            ..Default::default()
        };
        assert!(merge_health(Some(&config), HealthOverrides::default()).exclude_tests);
    }

    #[test]
    fn merge_dupes_prefers_the_cli_threshold_over_the_config() {
        let config = DupesConfig {
            min_tokens: Some(100),
            ..Default::default()
        };
        let cli = DupesOverrides {
            min_tokens: Some(30),
            ..Default::default()
        };
        assert_eq!(merge_dupes(Some(&config), cli).min_tokens, 30);
    }

    #[test]
    fn merge_dupes_falls_back_to_the_config_threshold() {
        let config = DupesConfig {
            min_tokens: Some(100),
            ..Default::default()
        };
        assert_eq!(
            merge_dupes(Some(&config), DupesOverrides::default()).min_tokens,
            100
        );
    }

    #[test]
    fn merge_dupes_falls_back_to_the_built_in_defaults() {
        assert_eq!(
            merge_dupes(None, DupesOverrides::default()),
            EffectiveDupes::default()
        );
    }

    #[test]
    fn merge_dupes_resolves_each_setting_independently() {
        // A config that sets only one threshold must not reset the others.
        let config = DupesConfig {
            min_tokens: Some(100),
            ..Default::default()
        };
        let cli = DupesOverrides {
            min_lines: Some(10),
            ..Default::default()
        };
        let effective = merge_dupes(Some(&config), cli);

        assert_eq!(effective.min_tokens, 100);
        assert_eq!(effective.min_lines, 10);
        assert_eq!(
            effective.min_occurrences,
            EffectiveDupes::default().min_occurrences
        );
        assert_eq!(effective.mode, DupeMode::Exact);
    }

    #[test]
    fn merge_dupes_prefers_the_cli_mode_over_the_config() {
        let config = DupesConfig {
            mode: Some(DupeMode::Semantic),
            ..Default::default()
        };
        let cli = DupesOverrides {
            mode: Some(DupeMode::Exact),
            ..Default::default()
        };
        assert_eq!(merge_dupes(Some(&config), cli).mode, DupeMode::Exact);

        assert_eq!(
            merge_dupes(Some(&config), DupesOverrides::default()).mode,
            DupeMode::Semantic
        );
    }

    #[test]
    fn a_config_baseline_resolves_against_the_config_directory() {
        // Not against the working directory: `roe health src/App` from the
        // repository root has to find the same file as a run from inside
        // `src/App`.
        let config = HealthConfig {
            baseline: Some("roe-baseline.json".to_string()),
            ..Default::default()
        };

        assert_eq!(
            resolve_baseline(Some(&config), Some(Path::new("/repo")), None),
            Some(Path::new("/repo").join("roe-baseline.json"))
        );
    }

    #[test]
    fn an_explicit_baseline_flag_wins_and_is_taken_as_typed() {
        let config = HealthConfig {
            baseline: Some("roe-baseline.json".to_string()),
            ..Default::default()
        };

        assert_eq!(
            resolve_baseline(
                Some(&config),
                Some(Path::new("/repo")),
                Some(Path::new("other.json"))
            ),
            Some(PathBuf::from("other.json"))
        );
    }

    #[test]
    fn no_baseline_in_either_place_filters_nothing() {
        assert_eq!(
            resolve_baseline(
                Some(&HealthConfig::default()),
                Some(Path::new("/repo")),
                None
            ),
            None
        );
        assert_eq!(resolve_baseline(None, None, None), None);
    }

    #[test]
    fn parses_a_health_block() {
        let config: RoeConfig =
            serde_json::from_str(r#"{"health": {"maxComplexity": 15, "excludeTests": true}}"#)
                .expect("valid json");
        let health = config.health.expect("a health block");

        assert_eq!(health.max_complexity, Some(15));
        assert_eq!(health.exclude_tests, Some(true));
        assert_eq!(health.max_cognitive, None);
    }

    #[test]
    fn rejects_unknown_health_fields() {
        let error = serde_json::from_str::<RoeConfig>(r#"{"health": {"maxComplexty": 15}}"#)
            .expect_err("typo should not be silently ignored");
        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn parses_a_dupes_block() {
        let config: RoeConfig =
            serde_json::from_str(r#"{"dupes": {"minTokens": 100, "mode": "semantic"}}"#)
                .expect("valid json");
        let dupes = config.dupes.expect("a dupes block");

        assert_eq!(dupes.min_tokens, Some(100));
        assert_eq!(dupes.mode, Some(DupeMode::Semantic));
        assert_eq!(dupes.min_lines, None);
    }

    #[test]
    fn parses_a_dupes_mode_from_yaml() {
        let config: RoeConfig =
            serde_yaml_ng::from_str("dupes:\n  mode: semantic\n  minTokens: 100\n")
                .expect("valid yaml");
        let dupes = config.dupes.expect("a dupes block");

        assert_eq!(dupes.mode, Some(DupeMode::Semantic));
        assert_eq!(dupes.min_tokens, Some(100));
    }

    #[test]
    fn rejects_unknown_dupes_fields() {
        // `minToken` is a near-miss of the real `minTokens` field — it must
        // fail loudly rather than leave the user thinking a threshold is in
        // force when it isn't.
        let error = serde_json::from_str::<RoeConfig>(r#"{"dupes": {"minToken": 100}}"#)
            .expect_err("typo should not be silently ignored");
        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn rejects_an_invalid_dupe_mode() {
        let error = serde_json::from_str::<RoeConfig>(r#"{"dupes": {"mode": "fast"}}"#)
            .expect_err("an unknown mode should fail loudly");
        let message = error.to_string();

        assert!(message.contains("unknown variant `fast`"), "got {message}");
        assert!(
            message.contains("expected `exact` or `semantic`"),
            "got {message}"
        );
    }

    #[test]
    fn rejects_unknown_fields() {
        // The misspelling is the point — it proves a near-miss of the real
        // `aggressive` field is rejected rather than silently ignored. It is
        // declared in `_typos.toml` so the spell checker leaves it alone.
        let err = serde_json::from_str::<RoeConfig>(r#"{"agressive": true}"#).unwrap_err();
        assert!(err.to_string().contains("unknown field"));
    }

    #[test]
    fn parses_yaml() {
        let config: RoeConfig =
            serde_yaml_ng::from_str("aggressive: true\nignore:\n  - Generated/\n")
                .expect("valid yaml");
        assert_eq!(config.aggressive, Some(true));
        assert_eq!(config.ignore, Some(vec!["Generated/".to_string()]));
    }

    #[test]
    fn discover_walks_up_to_find_nearest_config() {
        let start = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/config_walk_up/nested/deeper");
        let mut warnings = Vec::new();
        let resolved = discover(&start, &mut warnings)
            .expect("discover should succeed")
            .expect("a config should be found");
        assert_eq!(resolved.path.file_name().unwrap(), "roe.json");
        assert!(warnings.is_empty());
    }

    #[test]
    fn discover_returns_none_when_nothing_found_up_to_fs_root() {
        // A directory whose ancestry (up to the filesystem root) has no
        // roe.json/yaml/yml anywhere — use a fresh temp dir rather than a
        // repo fixture, since anything under this repo could pick up
        // roe.json placed at the repo root by other tests/fixtures.
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "roe-config-discover-none-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let mut warnings = Vec::new();
        let result = discover(&dir, &mut warnings);
        std::fs::remove_dir_all(&dir).ok();
        assert!(result.expect("discover should succeed").is_none());
    }

    #[test]
    fn ignore_glob_trailing_slash_matches_whole_directory() {
        let mut warnings = Vec::new();
        let set = build_ignore_globset(
            Path::new("/repo"),
            &["Generated/".to_string()],
            &mut warnings,
        )
        .expect("globset builds");
        assert!(set.is_match(Path::new("/repo/Generated/Nested/File.cs")));
        assert!(!set.is_match(Path::new("/repo/Other/File.cs")));
        assert!(warnings.is_empty());
    }
}
