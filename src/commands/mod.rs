pub mod check;
pub mod dead_code;
pub mod dupes;
pub mod health;
pub mod perf;

use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::Context as _;
use rustc_hash::FxHashMap;

use crate::extract::{self, FileFacts, Interner};
use crate::model::{MemberKind, ProjectId, SymbolId, SymbolKind, Workspace};
use crate::resolve::Resolution;
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
        let facts = extract::extract_all(&workspace.files, &rodeo, extract::PerfPass::Skip);

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
    Perf,
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
            Analysis::Perf => config.perf.as_ref().and_then(|c| c.ignore.as_deref()),
        };

        let mut patterns = config.ignore.clone().unwrap_or_default();
        patterns.extend(scoped.into_iter().flatten().cloned());

        if patterns.is_empty() {
            return None;
        }

        Some((patterns, resolved.dir.as_path()))
    }
}

/// Display names for every declaration in one file, indexed by local decl
/// index (non-members get an empty placeholder that nothing reads).
///
/// Overloads merge into a single symbol — `build_symbols` keys members on
/// (type, name, kind) — so `display_name` returns the same string for
/// `TryBeginActivation(Unit)` and `TryBeginActivation(Unit, Order)`, and a
/// report listing both is two identical rows. Where that collision actually
/// happens, the name gets a Roslyn-style `/arity` suffix. Where it doesn't,
/// nothing is appended: an arity on a name with no overloads is noise.
///
/// Shared by `health` and `perf` so the same declaration is named identically
/// in both reports.
pub(crate) fn member_names(
    resolution: &Resolution,
    file_facts: &extract::FileFacts,
    local_map: &[SymbolId],
    rodeo: &Interner,
) -> Vec<String> {
    let mut names: Vec<String> = Vec::with_capacity(file_facts.decls.len());

    for (local_index, decl) in file_facts.decls.iter().enumerate() {
        if decl.kind.is_member() {
            names.push(resolution.display_name(local_map[local_index], rodeo));
        } else {
            names.push(String::new());
        }
    }

    let mut occurrences: FxHashMap<&str, u32> = FxHashMap::default();
    for (name, decl) in names.iter().zip(&file_facts.decls) {
        if takes_parameters(decl.kind) {
            *occurrences.entry(name.as_str()).or_default() += 1;
        }
    }

    let ambiguous: Vec<bool> = names
        .iter()
        .zip(&file_facts.decls)
        .map(|(name, decl)| {
            takes_parameters(decl.kind) && occurrences.get(name.as_str()).is_some_and(|&n| n > 1)
        })
        .collect();

    for (local_index, is_ambiguous) in ambiguous.into_iter().enumerate() {
        if is_ambiguous {
            // The declared total, not the required count — the suffix exists
            // to tell overloads apart, so it has to match the signature the
            // reader will find in the source.
            names[local_index].push_str(&format!(
                "/{}",
                file_facts.decls[local_index].parameters.total()
            ));
        }
    }

    names
}

/// Whether an arity suffix means anything for this kind. Fields and
/// properties have no parameter list, so `Foo/0` would be a lie dressed as
/// precision.
fn takes_parameters(kind: SymbolKind) -> bool {
    matches!(
        kind,
        SymbolKind::Member(
            MemberKind::Method
                | MemberKind::Constructor
                | MemberKind::Indexer
                | MemberKind::Operator
                | MemberKind::ConversionOperator
        )
    )
}

pub(crate) fn in_test_project(workspace: &Workspace, project: Option<ProjectId>) -> bool {
    project.is_some_and(|id| workspace.projects[id.index()].is_test())
}

pub(crate) fn project_name(workspace: &Workspace, project: Option<ProjectId>) -> Option<String> {
    project.map(|id| workspace.projects[id.index()].name.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        DeadCodeConfig, DupesConfig, HealthConfig, PerfConfig, ResolvedConfig, RoeConfig,
    };

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
            perf: Some(PerfConfig {
                ignore: Some(vec!["Scripts/".to_string()]),
                ..Default::default()
            }),
            ..Default::default()
        });

        let (dead_code, _) = context.ignore_for(Analysis::DeadCode).expect("patterns");
        let (dupes, _) = context.ignore_for(Analysis::Dupes).expect("patterns");
        let (health, _) = context.ignore_for(Analysis::Health).expect("patterns");
        let (perf, _) = context.ignore_for(Analysis::Perf).expect("patterns");

        assert_eq!(dead_code, vec!["Plugins/".to_string()]);
        assert_eq!(dupes, vec!["Legacy/".to_string()]);
        assert_eq!(health, vec!["Models/".to_string()]);
        assert_eq!(perf, vec!["Scripts/".to_string()]);
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
