use std::path::Path;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use anyhow::Context;

use crate::cli::{DupeMode, DupesArgs};
use crate::model::{DupesResult, DupesSummary, Workspace};
use crate::{clone_extraction, config, discover, report, tokenize};

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

pub fn run(args: &DupesArgs) -> anyhow::Result<ExitCode> {
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

    let ignore = resolved_config.as_ref().and_then(|resolved| {
        resolved
            .config
            .ignore
            .as_ref()
            .map(|patterns| (patterns.as_slice(), resolved.dir.as_path()))
    });

    let mut analysis = analyze_with_ignores(
        &root,
        args.mode,
        args.min_tokens,
        args.min_lines,
        args.min_occurrences,
        ignore,
    )?;
    analysis.workspace.warnings.append(&mut config_warnings);

    for warning in &analysis.workspace.warnings {
        eprintln!("warning: {warning}");
    }

    Ok(report::dupes::emit(
        &analysis.result,
        &analysis.workspace,
        args.format,
        args.mode,
    ))
}
