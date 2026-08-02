pub mod check;
pub mod dead_code;
pub mod dupes;
pub mod health;

use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::Context as _;

use crate::extract::{self, FileFacts, Interner};
use crate::model::Workspace;
use crate::{config, discover};

/// The stages `dead-code` and `health` compute identically: the discovered
/// workspace, the interner, and the extracted per-file facts.
///
/// Parsing every file is by far the most expensive stage, so a combined run
/// does it once and both analyses read the result. Each analysis still owns its
/// own `Workspace` clone — both append warnings to it and hand it back in their
/// result — and still builds its own symbol table and graph, because
/// `dead-code` mutates symbol flags (kill list, entry points) that `health`
/// must not see.
pub(crate) struct Extracted {
    pub workspace: Workspace,
    pub rodeo: Interner,
    pub facts: Vec<FileFacts>,
    /// When discovery began. Both analyses time themselves from here, so a
    /// shared run reports the same elapsed figures a standalone run would
    /// rather than silently omitting the parse they both depend on.
    pub started: Instant,
}

impl Extracted {
    pub fn build(root: &Path) -> anyhow::Result<Self> {
        let started = Instant::now();
        let workspace = discover::discover(root)?;
        let rodeo = extract::new_interner();
        let facts = extract::extract_all(&workspace.files, &rodeo);

        Ok(Self {
            workspace,
            rodeo,
            facts,
            started,
        })
    }
}

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

/// Which analysis is asking for its ignore globs — each one unions the
/// top-level list with its own config section's list.
#[derive(Clone, Copy)]
pub enum Analysis {
    DeadCode,
    Dupes,
    Health,
}

impl Context {
    /// The union of the config's top-level `ignore` globs and the given
    /// analysis's own list, paired with the directory they resolve against.
    /// `None` when the union is empty.
    ///
    /// Built in one place so every consumer of an analysis's globs — filters,
    /// summaries, footers — sees the same list and their counts can't drift.
    pub fn ignore_for(&self, analysis: Analysis) -> Option<(Vec<String>, &Path)> {
        let resolved = self.config.as_ref()?;
        let config = &resolved.config;
        let scoped = match analysis {
            Analysis::DeadCode => config.dead_code.as_ref().and_then(|c| c.ignore.as_deref()),
            Analysis::Dupes => config.dupes.as_ref().and_then(|c| c.ignore.as_deref()),
            Analysis::Health => config.health.as_ref().and_then(|c| c.ignore.as_deref()),
        };

        let mut patterns = config.ignore.clone().unwrap_or_default();
        patterns.extend(scoped.into_iter().flatten().cloned());

        if patterns.is_empty() {
            return None;
        }

        Some((patterns, resolved.dir.as_path()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{DeadCodeConfig, DupesConfig, HealthConfig, ResolvedConfig, RoeConfig};

    fn context_with(config: RoeConfig) -> Context {
        Context {
            root: PathBuf::from("/repo"),
            config: Some(ResolvedConfig {
                path: PathBuf::from("/repo/roe.json"),
                dir: PathBuf::from("/repo"),
                config,
            }),
            warnings: Vec::new(),
        }
    }

    #[test]
    fn ignore_for_unions_top_level_and_scoped_globs() {
        let context = context_with(RoeConfig {
            ignore: Some(vec!["Generated/".to_string()]),
            dupes: Some(DupesConfig {
                ignore: Some(vec!["Legacy/".to_string()]),
                ..Default::default()
            }),
            ..Default::default()
        });

        let (patterns, dir) = context.ignore_for(Analysis::Dupes).expect("a union");

        assert_eq!(
            patterns,
            vec!["Generated/".to_string(), "Legacy/".to_string()]
        );
        assert_eq!(dir, Path::new("/repo"));
    }

    #[test]
    fn ignore_for_scopes_each_list_to_its_own_analysis() {
        let context = context_with(RoeConfig {
            dead_code: Some(DeadCodeConfig {
                ignore: Some(vec!["Plugins/".to_string()]),
            }),
            dupes: Some(DupesConfig {
                ignore: Some(vec!["Legacy/".to_string()]),
                ..Default::default()
            }),
            health: Some(HealthConfig {
                ignore: Some(vec!["Models/".to_string()]),
                ..Default::default()
            }),
            ..Default::default()
        });

        let (dead_code, _) = context.ignore_for(Analysis::DeadCode).expect("patterns");
        let (dupes, _) = context.ignore_for(Analysis::Dupes).expect("patterns");
        let (health, _) = context.ignore_for(Analysis::Health).expect("patterns");

        assert_eq!(dead_code, vec!["Plugins/".to_string()]);
        assert_eq!(dupes, vec!["Legacy/".to_string()]);
        assert_eq!(health, vec!["Models/".to_string()]);
    }

    #[test]
    fn ignore_for_returns_none_when_nothing_is_configured() {
        let no_config = Context {
            root: PathBuf::from("/repo"),
            config: None,
            warnings: Vec::new(),
        };
        assert!(no_config.ignore_for(Analysis::Dupes).is_none());

        let empty = context_with(RoeConfig::default());
        assert!(empty.ignore_for(Analysis::Health).is_none());
    }

    #[test]
    fn ignore_for_treats_an_empty_scoped_list_as_a_no_op() {
        let context = context_with(RoeConfig {
            dupes: Some(DupesConfig {
                ignore: Some(Vec::new()),
                ..Default::default()
            }),
            ..Default::default()
        });

        assert!(context.ignore_for(Analysis::Dupes).is_none());
    }
}
