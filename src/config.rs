use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use globset::{Glob, GlobSet, GlobSetBuilder};
use serde::Deserialize;

/// User-authored suppression/override settings, loaded from `roe.json`,
/// `roe.yaml`, or `roe.yml`.
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoeConfig {
    pub aggressive: Option<bool>,
    pub roots: Option<Vec<String>>,
    /// Glob patterns (relative to this config file's directory) whose
    /// matching files have all their findings suppressed. A pattern ending in
    /// `/` also matches everything under that directory.
    pub ignore: Option<Vec<String>>,
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
}

/// `aggressive` is OR'd because clap's plain bool flag can only ever be
/// explicitly `true` — there's no `--no-aggressive` to override a config's
/// `true` back to `false`. `roots`, when passed on the CLI, replaces the
/// config's list wholesale rather than merging with it.
pub fn merge(
    config: Option<&RoeConfig>,
    cli_aggressive: bool,
    cli_roots: &[String],
) -> EffectiveArgs {
    let config_aggressive = config.and_then(|c| c.aggressive).unwrap_or(false);
    let roots = if !cli_roots.is_empty() {
        cli_roots.to_vec()
    } else {
        config.and_then(|c| c.roots.clone()).unwrap_or_default()
    };
    EffectiveArgs {
        aggressive: cli_aggressive || config_aggressive,
        roots,
    }
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
        assert!(merge(Some(&config), true, &[]).aggressive);
    }

    #[test]
    fn merge_falls_back_to_config_aggressive() {
        let config = RoeConfig {
            aggressive: Some(true),
            ..Default::default()
        };
        assert!(merge(Some(&config), false, &[]).aggressive);
    }

    #[test]
    fn merge_defaults_aggressive_to_false() {
        assert!(!merge(None, false, &[]).aggressive);
    }

    #[test]
    fn merge_cli_roots_override_config_roots() {
        let config = RoeConfig {
            roots: Some(vec!["Config.Root".to_string()]),
            ..Default::default()
        };
        let cli_roots = vec!["Cli.Root".to_string()];
        assert_eq!(
            merge(Some(&config), false, &cli_roots).roots,
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
            merge(Some(&config), false, &[]).roots,
            vec!["Config.Root".to_string()]
        );
    }

    #[test]
    fn merge_defaults_roots_to_empty() {
        assert!(merge(None, false, &[]).roots.is_empty());
    }

    #[test]
    fn rejects_unknown_fields() {
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
