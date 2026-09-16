use std::path::Path;
use std::process::ExitCode;
use std::time::Instant;

use crate::cli::DeadCodeArgs;
use crate::commands::Context;
use crate::model::{AnalysisResult, SymbolId, Workspace};
use crate::resolve::SymbolFlags;
use crate::{analyze, commands, config, entry_points, graph, report, resolve, rules, suppress};

pub struct Analysis {
    pub workspace: Workspace,
    pub result: AnalysisResult,
}

/// The full dead-code pipeline: discover → extract → symbol table → kill
/// list → entry points → graph → reachability → detectors → inline
/// suppressions. `entry_points` globs resolve against the workspace root;
/// the config-aware path through [`execute`] resolves them against the
/// config file's own directory instead.
pub fn analyze(
    root: &Path,
    aggressive: bool,
    manual_roots: &[String],
    library_projects: &[String],
    entry_points: &[String],
) -> anyhow::Result<Analysis> {
    let extracted = commands::Extracted::build(root)?;
    let entry_point_dir = extracted.workspace.root.clone();

    analyze_extracted(
        &extracted,
        extracted.started,
        aggressive,
        manual_roots,
        library_projects,
        entry_points,
        &entry_point_dir,
    )
}

/// The pipeline from the symbol table onward, over an extraction a caller may
/// already have built. `check` shares one with `health`, which is where the
/// parse cost is paid.
pub(crate) fn analyze_extracted(
    extracted: &commands::Extracted,
    start: Instant,
    aggressive: bool,
    manual_roots: &[String],
    library_projects: &[String],
    entry_points: &[String],
    entry_point_dir: &Path,
) -> anyhow::Result<Analysis> {
    let mut workspace = extracted.workspace.clone();
    let rodeo = &extracted.rodeo;
    let facts = &extracted.facts;

    let mut resolution = resolve::build_symbols(&workspace.files, facts, rodeo);

    rules::apply_kill_list(&mut resolution, &workspace, rodeo, aggressive);

    let mut notes = entry_points::mark_roots(
        &mut resolution,
        &workspace,
        facts,
        manual_roots,
        library_projects,
        rodeo,
    );

    let entry_point_globs =
        config::build_entry_point_globs(entry_point_dir, entry_points, &mut workspace.warnings);
    notes.extend(entry_points::mark_entry_point_files(
        &mut resolution,
        &workspace,
        &entry_point_globs,
    ));

    let symbol_graph = graph::build_graph(&mut resolution, &workspace, facts, rodeo);
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
        rodeo,
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
    let extracted = commands::Extracted::build(&context.root)?;

    execute_extracted(context, args, &extracted)
}

/// `execute` over an extraction the caller already built, so `check` can share
/// one with `health`.
pub(crate) fn execute_extracted(
    context: &Context,
    args: &DeadCodeArgs,
    extracted: &commands::Extracted,
) -> anyhow::Result<Analysis> {
    let effective = config::merge(
        context.config.as_ref().map(|resolved| &resolved.config),
        args.aggressive,
        &args.roots,
        &args.library_projects,
    );

    // `entryPoints` has no CLI flag (like `ignore`), so it comes straight
    // from the config, and its globs resolve against the config file's own
    // directory the way `ignore` globs do.
    let (entry_points, entry_point_dir) = match context.config.as_ref() {
        Some(resolved) => (
            resolved.config.entry_points.clone().unwrap_or_default(),
            resolved.dir.clone(),
        ),
        None => (Vec::new(), context.root.clone()),
    };

    let mut analysis = analyze_extracted(
        extracted,
        extracted.started,
        effective.aggressive,
        &effective.roots,
        &effective.library_projects,
        &entry_points,
        &entry_point_dir,
    )?;
    analysis
        .workspace
        .warnings
        .extend(context.warnings.iter().cloned());

    if let Some((patterns, dir)) = context.ignore_for(commands::Analysis::DeadCode) {
        suppress::apply_config_ignores(
            &mut analysis.result,
            &patterns,
            dir,
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
