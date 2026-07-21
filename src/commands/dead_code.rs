use std::path::Path;
use std::process::ExitCode;
use std::time::Instant;

use anyhow::Context;
use lasso::ThreadedRodeo;

use crate::cli::DeadCodeArgs;
use crate::model::{AnalysisResult, SymbolId, Workspace};
use crate::resolve::SymbolFlags;
use crate::{analyze, config, discover, entry_points, extract, graph, report, resolve, rules, suppress};

pub struct Analysis {
    pub workspace: Workspace,
    pub result: AnalysisResult,
}

/// The full dead-code pipeline: discover → extract → symbol table → kill
/// list → entry points → graph → reachability → detectors → inline
/// suppressions.
pub fn analyze(root: &Path, aggressive: bool, manual_roots: &[String]) -> anyhow::Result<Analysis> {
    let start = Instant::now();

    let mut workspace = discover::discover(root)?;
    let rodeo = ThreadedRodeo::default();
    let facts = extract::extract_all(&workspace.files, &rodeo);

    let mut resolution = resolve::build_symbols(&workspace.files, &facts, &rodeo);
    rules::apply_kill_list(&mut resolution, &workspace, &rodeo, aggressive);
    let notes = entry_points::mark_roots(&mut resolution, &workspace, &facts, manual_roots, &rodeo);

    let symbol_graph = graph::build_graph(&mut resolution, &workspace, &facts, &rodeo);
    let roots: Vec<SymbolId> = resolution
        .symbols
        .iter()
        .filter(|s| s.flags.contains(SymbolFlags::ROOT))
        .map(|s| s.id)
        .collect();
    let reachable = graph::mark_reachable(&resolution, &symbol_graph, roots.into_iter());

    let mut result = analyze::find_dead(
        &resolution,
        &reachable,
        &workspace,
        &rodeo,
        start.elapsed(),
        notes,
    );

    suppress::apply_inline_suppressions(&mut result, &mut workspace);

    Ok(Analysis { workspace, result })
}

pub fn run(args: &DeadCodeArgs) -> anyhow::Result<ExitCode> {
    let root = match &args.path {
        Some(path) => path.clone(),
        None => std::env::current_dir()?,
    };

    let mut config_warnings = Vec::new();
    let resolved_config = match &args.config {
        Some(path) => Some(config::load_explicit(path)?),
        None => {
            let canonical_root = std::fs::canonicalize(&root)
                .with_context(|| format!("path not found: {}", root.display()))?;
            let config_start = if canonical_root.is_dir() {
                canonical_root
            } else {
                canonical_root
                    .parent()
                    .map(Path::to_path_buf)
                    .unwrap_or(canonical_root)
            };
            config::discover(&config_start, &mut config_warnings)?
        }
    };

    let effective = config::merge(
        resolved_config.as_ref().map(|r| &r.config),
        args.aggressive,
        &args.roots,
    );

    let mut analysis = analyze(&root, effective.aggressive, &effective.roots)?;
    analysis.workspace.warnings.append(&mut config_warnings);

    if let Some(resolved) = &resolved_config
        && let Some(ignore) = &resolved.config.ignore
    {
        suppress::apply_config_ignores(
            &mut analysis.result,
            ignore,
            &resolved.dir,
            &mut analysis.workspace.warnings,
        );
    }

    for warning in &analysis.workspace.warnings {
        eprintln!("warning: {warning}");
    }

    Ok(report::emit(
        &analysis.result,
        &analysis.workspace,
        args.format,
    ))
}
