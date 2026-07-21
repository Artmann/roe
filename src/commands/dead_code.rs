use std::path::Path;
use std::process::ExitCode;
use std::time::Instant;

use lasso::ThreadedRodeo;

use crate::cli::DeadCodeArgs;
use crate::model::{AnalysisResult, SymbolId, Workspace};
use crate::resolve::SymbolFlags;
use crate::{analyze, discover, entry_points, extract, graph, report, resolve, rules};

pub struct Analysis {
    pub workspace: Workspace,
    pub result: AnalysisResult,
}

/// The full dead-code pipeline: discover → extract → symbol table → kill
/// list → entry points → graph → reachability → detectors.
pub fn analyze(root: &Path, aggressive: bool, manual_roots: &[String]) -> anyhow::Result<Analysis> {
    let start = Instant::now();

    let workspace = discover::discover(root)?;
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

    let result = analyze::find_dead(
        &resolution,
        &reachable,
        &workspace,
        &rodeo,
        start.elapsed(),
        notes,
    );

    Ok(Analysis { workspace, result })
}

pub fn run(args: &DeadCodeArgs) -> anyhow::Result<ExitCode> {
    let root = match &args.path {
        Some(path) => path.clone(),
        None => std::env::current_dir()?,
    };

    let analysis = analyze(&root, args.aggressive, &args.roots)?;
    for warning in &analysis.workspace.warnings {
        eprintln!("warning: {warning}");
    }

    Ok(report::emit(
        &analysis.result,
        &analysis.workspace,
        args.format,
    ))
}
