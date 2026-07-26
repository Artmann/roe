use std::path::Path;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use crate::cli::{DupeMode, DupesArgs};
use crate::commands::Context;
use crate::model::{DupesResult, DupesSummary, Workspace};
use crate::{clone_extraction, commands, config, discover, report, tokenize};

pub struct Analysis {
    pub workspace: Workspace,
    pub result: DupesResult,
}

/// The full dupes pipeline: discover → tokenize → suffix array + LCP →
/// clone-group extraction.
pub fn analyze(
    root: &Path,
    mode: DupeMode,
    min_tokens: u32,
    min_lines: u32,
    min_occurrences: u32,
) -> anyhow::Result<Analysis> {
    analyze_with_ignores(root, mode, min_tokens, min_lines, min_occurrences, None)
}

/// Same pipeline, but additionally filters `workspace.files` against a
/// config's `ignore` globs before tokenizing — a duplicate spans multiple
/// files, so ignoring has to happen at the file-list stage rather than by
/// filtering a finished result list the way `dead-code` does.
fn analyze_with_ignores(
    root: &Path,
    mode: DupeMode,
    min_tokens: u32,
    min_lines: u32,
    min_occurrences: u32,
    ignore: Option<(&[String], &Path)>,
) -> anyhow::Result<Analysis> {
    let start = Instant::now();

    let mut workspace = discover::discover(root)?;

    // Generated sources (obj/ harvest, *.g.cs, designer files) exist in the
    // workspace for dead-code reference tracking, but duplication in them is
    // the generator's business, not the user's.
    workspace.files.retain(|file| !file.is_generated);

    if let Some((patterns, config_dir)) = ignore {
        let mut warnings = Vec::new();
        if let Some(set) = config::build_ignore_globset(config_dir, patterns, &mut warnings) {
            workspace.files.retain(|file| !set.is_match(&file.path));
        }
        workspace.warnings.append(&mut warnings);
    }

    let corpus = tokenize::tokenize_all(&workspace.files, mode);
    let groups = clone_extraction::extract_groups(&corpus, min_tokens, min_lines, min_occurrences);
    let result = build_result(groups, &workspace, start.elapsed());

    Ok(Analysis { workspace, result })
}

fn build_result(
    groups: Vec<crate::model::DupeGroup>,
    workspace: &Workspace,
    elapsed: Duration,
) -> DupesResult {
    let duplicated_lines = groups
        .iter()
        .map(|group| group.line_count as usize * group.occurrences.len())
        .sum();

    let summary = DupesSummary {
        projects: workspace.projects.len(),
        files_scanned: workspace.files.len(),
        groups: groups.len(),
        duplicated_lines,
        elapsed_ms: elapsed.as_millis(),
    };

    DupesResult { groups, summary }
}

/// The analysis half of `run`. Prints nothing and decides no exit code, so
/// `check` can run it alongside the other two analyses.
pub(crate) fn execute(context: &Context, args: &DupesArgs) -> anyhow::Result<Analysis> {
    let mut analysis = analyze_with_ignores(
        &context.root,
        args.mode,
        args.min_tokens,
        args.min_lines,
        args.min_occurrences,
        context.ignore(),
    )?;
    analysis
        .workspace
        .warnings
        .extend(context.warnings.iter().cloned());

    Ok(analysis)
}

pub fn run(args: &DupesArgs) -> anyhow::Result<ExitCode> {
    let context = commands::resolve(&args.path, &args.config)?;
    let analysis = execute(&context, args)?;

    for warning in &analysis.workspace.warnings {
        eprintln!("warning: {warning}");
    }

    Ok(report::dupes::emit(
        &analysis.result,
        &analysis.workspace,
        args.format,
        args.mode,
        !args.no_code,
    ))
}
