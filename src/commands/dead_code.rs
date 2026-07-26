use std::path::Path;
use std::process::ExitCode;
use std::time::Instant;

use lasso::ThreadedRodeo;

use crate::cli::DeadCodeArgs;
use crate::commands::Context;
use crate::model::{AnalysisResult, SymbolId, Workspace};
use crate::resolve::SymbolFlags;
use crate::{
    analyze, commands, config, discover, entry_points, extract, graph, report, resolve, rules,
    suppress,
};

pub struct Analysis {
    pub workspace: Workspace,
    pub result: AnalysisResult,
}

/// The full dead-code pipeline: discover → extract → symbol table → kill
/// list → entry points → graph → reachability → detectors → inline
/// suppressions.
pub fn analyze(
    root: &Path,
    aggressive: bool,
    manual_roots: &[String],
    library_projects: &[String],
) -> anyhow::Result<Analysis> {
    let start = Instant::now();

    let mut workspace = discover::discover(root)?;
    let rodeo = ThreadedRodeo::default();
    let facts = extract::extract_all(&workspace.files, &rodeo);

    let mut resolution = resolve::build_symbols(&workspace.files, &facts, &rodeo);

    rules::apply_kill_list(&mut resolution, &workspace, &rodeo, aggressive);

    let notes = entry_points::mark_roots(
        &mut resolution,
        &workspace,
        &facts,
        manual_roots,
        library_projects,
        &rodeo,
    );

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

/// The analysis half of `run`: merge config with the command line, analyze, and
/// apply the config's `ignore` globs. Prints nothing and decides no exit code,
/// so `check` can run it alongside the other two analyses.
///
/// The returned workspace collects the context's config warnings too, in the
/// order they were produced: discovery, then config, then ignore-glob problems.
pub(crate) fn execute(context: &Context, args: &DeadCodeArgs) -> anyhow::Result<Analysis> {
    let effective = config::merge(
        context.config.as_ref().map(|resolved| &resolved.config),
        args.aggressive,
        &args.roots,
        &args.library_projects,
    );

    let mut analysis = analyze(
        &context.root,
        effective.aggressive,
        &effective.roots,
        &effective.library_projects,
    )?;
    analysis
        .workspace
        .warnings
        .extend(context.warnings.iter().cloned());

    if let Some(resolved) = &context.config
        && let Some(ignore) = &resolved.config.ignore
    {
        suppress::apply_config_ignores(
            &mut analysis.result,
            ignore,
            &resolved.dir,
            &mut analysis.workspace.warnings,
        );
    }

    Ok(analysis)
}

pub fn run(args: &DeadCodeArgs) -> anyhow::Result<ExitCode> {
    let context = commands::resolve(&args.path, &args.config)?;
    let analysis = execute(&context, args)?;

    for warning in &analysis.workspace.warnings {
        eprintln!("warning: {warning}");
    }

    Ok(report::emit(
        &analysis.result,
        &analysis.workspace,
        args.format,
    ))
}
