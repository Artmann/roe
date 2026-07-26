pub mod check;
pub mod dead_code;
pub mod dupes;
pub mod health;

use std::path::{Path, PathBuf};

use anyhow::Context as _;

use crate::config;

/// Everything the three analyses need before any of them starts: where to look,
/// and which config file (if any) governs the run.
///
/// `check` resolves this once and hands the same context to all three analyses,
/// so a combined run discovers its config once and reports config warnings
/// once.
pub struct Context {
    pub root: PathBuf,
    pub config: Option<config::ResolvedConfig>,
    /// Non-fatal problems from config discovery. Kept here rather than printed
    /// immediately so the caller can merge them with the workspace's own
    /// warnings and emit each one exactly once.
    pub warnings: Vec<String>,
}

/// Resolve the codebase root and the governing config. An explicit `--config`
/// path skips discovery entirely; otherwise discovery starts at the root (or
/// its parent, when the root names a `.sln`/`.csproj` rather than a directory)
/// and walks up.
pub fn resolve(
    path: &Option<PathBuf>,
    explicit_config: &Option<PathBuf>,
) -> anyhow::Result<Context> {
    let root = match path {
        Some(path) => path.clone(),
        None => std::env::current_dir()?,
    };

    let mut warnings = Vec::new();
    let config = match explicit_config {
        Some(path) => Some(config::load_explicit(path)?),
        None => {
            let canonical_root = crate::paths::canonicalize(&root)
                .with_context(|| format!("path not found: {}", root.display()))?;
            let config_start = if canonical_root.is_dir() {
                canonical_root
            } else {
                canonical_root
                    .parent()
                    .map(Path::to_path_buf)
                    .unwrap_or(canonical_root)
            };
            config::discover(&config_start, &mut warnings)?
        }
    };

    Ok(Context {
        root,
        config,
        warnings,
    })
}

impl Context {
    /// The config's `ignore` globs paired with the directory they resolve
    /// against, in the shape `dupes` and `health` filter with.
    pub fn ignore(&self) -> Option<(&[String], &Path)> {
        self.config.as_ref().and_then(|resolved| {
            resolved
                .config
                .ignore
                .as_ref()
                .map(|patterns| (patterns.as_slice(), resolved.dir.as_path()))
        })
    }
}
