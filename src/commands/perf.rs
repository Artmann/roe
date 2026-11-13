use std::path::Path;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use globset::GlobSet;
use rustc_hash::FxHashMap;

use crate::cli::{ConfidenceFilter, PerfArgs};
use crate::commands::{Context, in_test_project, member_names, project_name};
use crate::config::EffectivePerf;
use crate::model::{Confidence, PerfFinding, PerfFindingKind, PerfResult, PerfSummary, Workspace};
use crate::resolve::Resolution;
use crate::{commands, config, discover, extract, report, resolve, suppress};

pub struct Analysis {
    pub workspace: Workspace,
    pub result: PerfResult,
}

/// What to look for and where.
///
/// `min_confidence` filters the report rather than the exit code: only
/// high-confidence findings ever gate, so `--confidence high` narrows what is
/// printed without changing whether the build fails.
#[derive(Debug, Clone)]
pub struct Settings {
    pub disabled_checks: Vec<PerfFindingKind>,
    pub exclude_tests: bool,
    pub min_confidence: Confidence,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            disabled_checks: Vec::new(),
            exclude_tests: false,
            min_confidence: Confidence::Medium,
        }
    }
}

impl Settings {
    fn from_effective(effective: EffectivePerf, confidence: ConfidenceFilter) -> Self {
        Settings {
            disabled_checks: effective.disabled_checks,
            exclude_tests: effective.exclude_tests,
            min_confidence: match confidence {
                ConfidenceFilter::All => Confidence::Medium,
                ConfidenceFilter::High => Confidence::High,
            },
        }
    }

    fn wants(&self, kind: PerfFindingKind) -> bool {
        kind.confidence() >= self.min_confidence && !self.disabled_checks.contains(&kind)
    }
}

/// The full perf pipeline: discover → extract (with the catalog pass) → symbol
/// table → attribute findings to their members → rank.
///
/// No reachability analysis and no reference graph: a costly pattern is costly
/// whether or not anything else points at the method around it, and the only
/// thing the symbol table is here for is to give each finding a name a reader
/// can search for.
pub fn analyze(root: &Path, settings: &Settings) -> anyhow::Result<Analysis> {
    analyze_inner(root, settings, None)
}

/// Same pipeline, plus dropping findings under a config's `ignore` globs.
///
/// Ignore filtering happens after resolution rather than at the file-list
/// stage, for the reason `health` documents: `resolve::build_symbols` indexes
/// into `workspace.files` by raw `FileId`, so dropping entries beforehand
/// would desync those positions.
fn analyze_inner(
    root: &Path,
    settings: &Settings,
    ignore: Option<(&[String], &Path)>,
) -> anyhow::Result<Analysis> {
    let start = Instant::now();

    let mut workspace = discover::discover(root)?;
    let rodeo = extract::new_interner();
    let facts = extract::extract_all(&workspace.files, &rodeo, extract::PerfPass::Run);
    let resolution = resolve::build_symbols(&workspace.files, &facts, &rodeo);

    let mut findings = collect_findings(&resolution, &workspace, &facts, &rodeo, settings);

    let ignore_set = match ignore {
        Some((patterns, config_dir)) => {
            let mut warnings = Vec::new();
            let set = config::build_ignore_globset(config_dir, patterns, &mut warnings);
            workspace.warnings.append(&mut warnings);

            set
        }
        None => None,
    };

    if let Some(set) = &ignore_set {
        findings.retain(|finding| !set.is_match(&finding.file));
    }

    rank(&mut findings);

    let mut result = PerfResult {
        findings,
        summary: PerfSummary::default(),
    };

    suppress::apply_inline_suppressions_perf(&mut result, &mut workspace);

    result.summary = summarize(
        &workspace,
        &result.findings,
        settings,
        ignore_set.as_ref(),
        start.elapsed(),
    );

    Ok(Analysis { workspace, result })
}

/// Turn each file's raw catalog hits into named findings.
///
/// The descent recorded the source position of the member declaration it sat
/// in; the first pass recorded the same position on every `RawDecl`. Joining
/// on that pair is what keeps `src/perf` from having to know anything about
/// the walker's declaration numbering.
fn collect_findings(
    resolution: &Resolution,
    workspace: &Workspace,
    facts: &[extract::FileFacts],
    rodeo: &extract::Interner,
    settings: &Settings,
) -> Vec<PerfFinding> {
    let mut findings = Vec::new();

    for file_facts in facts {
        // Extraction never scans a generated file, so its list is empty too.
        if file_facts.perf.is_empty() {
            continue;
        }

        let file = &workspace.files[file_facts.file.index()];
        if settings.exclude_tests && in_test_project(workspace, file.project) {
            continue;
        }

        let project = project_name(workspace, file.project);
        let local_map = &resolution.decl_map[file_facts.file.index()];
        let names = member_names(resolution, file_facts, local_map, rodeo);

        let mut by_position: FxHashMap<(u32, u32), usize> = FxHashMap::default();
        for (local_index, decl) in file_facts.decls.iter().enumerate() {
            if decl.kind.is_member() {
                by_position.insert((decl.line, decl.column), local_index);
            }
        }

        for raw in &file_facts.perf {
            if !settings.wants(raw.kind) {
                continue;
            }

            let member = raw
                .member
                .and_then(|position| by_position.get(&position))
                .map(|&local_index| names[local_index].clone())
                .unwrap_or_else(|| TOP_LEVEL.to_string());

            findings.push(PerfFinding {
                kind: raw.kind,
                member,
                project: project.clone(),
                file: file.path.clone(),
                line: raw.line,
                column: raw.column,
                loop_depth: raw.loop_depth,
                detail: raw.detail.clone(),
                fix: raw.fix.clone(),
            });
        }
    }

    findings
}

/// What a finding outside any declaration is called. Top-level statements
/// belong to a compiler-generated entry point with no name in the source, so
/// there is nothing truthful to put here but a placeholder.
const TOP_LEVEL: &str = "<top-level statements>";

/// Worst first, by estimated wasted runtime. Ties fall back to position, which
/// keeps the output stable across runs and platforms — the same rule
/// `hotspot::rank` follows.
fn rank(findings: &mut [PerfFinding]) {
    findings.sort_by(|a, b| {
        b.score()
            .total_cmp(&a.score())
            .then_with(|| a.file.cmp(&b.file))
            .then((a.line, a.column).cmp(&(b.line, b.column)))
    });
}

/// The scan totals count what was *eligible to be reported*, not what was
/// parsed, so a run that skipped a whole test project does not claim to have
/// looked at it. Whatever was ruled out is counted separately so the report
/// can name it — the same contract `roe health`'s footer follows.
fn summarize(
    workspace: &Workspace,
    findings: &[PerfFinding],
    settings: &Settings,
    ignore: Option<&GlobSet>,
    elapsed: Duration,
) -> PerfSummary {
    let mut excluded_files = 0;
    let mut files_scanned = 0;

    for file in &workspace.files {
        let in_excluded_test = settings.exclude_tests && in_test_project(workspace, file.project);
        let is_ignored = ignore.is_some_and(|set| set.is_match(&file.path));

        // Generated files were never eligible under any setting, so counting
        // them here would report an exclusion the user did not choose. Files
        // already dropped with their test project are reported as that
        // project, not twice.
        if is_ignored && !in_excluded_test && !file.is_generated {
            excluded_files += 1;
        }

        if !in_excluded_test && !is_ignored && !file.is_generated {
            files_scanned += 1;
        }
    }

    let mut excluded_test_projects: Vec<String> = if settings.exclude_tests {
        workspace
            .projects
            .iter()
            .filter(|project| project.is_test())
            .map(|project| project.name.clone())
            .collect()
    } else {
        Vec::new()
    };
    excluded_test_projects.sort();

    PerfSummary {
        projects: workspace.projects.len() - excluded_test_projects.len(),
        files_scanned,
        excluded_test_projects,
        excluded_files,
        counts: count_kinds(findings),
        gating: findings.iter().filter(|f| f.is_gating()).count(),
        disabled_checks: settings.disabled_checks.clone(),
        elapsed_ms: elapsed.as_millis(),
    }
}

/// Findings per kind, in the order the kinds first appear in the report.
///
/// `findings` arrives ranked, so first appearance *is* the ranking — which is
/// the point: a footer that ordered the same scores differently from the list
/// above it would read as a second, contradictory ranking. (`--sort path`
/// re-orders only what is printed; the breakdown stays in ranked order, which
/// is what the reader asked to look past rather than to replace.)
///
/// Kinds that found nothing are left out; a row of zeroes says only that roe
/// has a lot of checks.
fn count_kinds(findings: &[PerfFinding]) -> Vec<(PerfFindingKind, usize)> {
    let mut counts: Vec<(PerfFindingKind, usize)> = Vec::new();

    for finding in findings {
        match counts.iter_mut().find(|(kind, _)| *kind == finding.kind) {
            Some(entry) => entry.1 += 1,
            None => counts.push((finding.kind, 1)),
        }
    }

    counts
}

/// The analysis half of `run`: merge the config's `perf` block with the
/// command line, then analyze. Prints nothing and decides no exit code.
pub(crate) fn execute(context: &Context, args: &PerfArgs) -> anyhow::Result<Analysis> {
    let mut warnings = Vec::new();
    let effective = config::merge_perf(
        context
            .config
            .as_ref()
            .and_then(|resolved| resolved.config.perf.as_ref()),
        config::PerfOverrides {
            exclude_tests: args.exclude_tests,
        },
        &mut warnings,
    );

    let settings = Settings::from_effective(effective, args.confidence);
    let ignore = context.ignore_for(commands::Analysis::Perf);
    let mut analysis = analyze_inner(
        &context.root,
        &settings,
        ignore
            .as_ref()
            .map(|(patterns, dir)| (patterns.as_slice(), *dir)),
    )?;

    analysis
        .workspace
        .warnings
        .extend(context.warnings.iter().cloned());
    analysis.workspace.warnings.append(&mut warnings);

    Ok(analysis)
}

pub fn run(args: &PerfArgs) -> anyhow::Result<ExitCode> {
    let context = commands::resolve(&args.path, &args.config)?;
    let analysis = execute(&context, args)?;

    for warning in &analysis.workspace.warnings {
        eprintln!("warning: {warning}");
    }

    Ok(report::perf::emit(
        &analysis.result,
        &analysis.workspace,
        args.format,
        args.sort,
        args.limit,
    ))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn finding(kind: PerfFindingKind, loop_depth: u32, line: u32) -> PerfFinding {
        PerfFinding {
            kind,
            member: "App.Service.Run".to_string(),
            project: None,
            file: PathBuf::from("Service.cs"),
            line,
            column: 1,
            loop_depth,
            detail: String::new(),
            fix: String::new(),
        }
    }

    #[test]
    fn the_hottest_finding_leads_the_report() {
        let mut findings = vec![
            finding(PerfFindingKind::SyncOverAsync, 0, 10),
            finding(PerfFindingKind::UnstableClock, 2, 20),
        ];

        rank(&mut findings);

        assert_eq!(findings[0].kind, PerfFindingKind::UnstableClock);
    }

    #[test]
    fn equally_costly_findings_are_ordered_by_position() {
        // Stability matters more than any tiebreak rule: a report that
        // reshuffles between runs cannot be diffed.
        let mut findings = vec![
            finding(PerfFindingKind::SyncOverAsync, 1, 40),
            finding(PerfFindingKind::SyncOverAsync, 1, 12),
        ];

        rank(&mut findings);

        assert_eq!(findings[0].line, 12);
    }

    #[test]
    fn the_breakdown_reads_in_the_same_order_as_the_report() {
        // Counted off the ranked list, so the footer cannot contradict the
        // findings printed above it.
        let mut findings = vec![
            finding(PerfFindingKind::UnstableClock, 0, 1),
            finding(PerfFindingKind::UnstableClock, 0, 2),
            finding(PerfFindingKind::SyncOverAsync, 0, 3),
        ];

        rank(&mut findings);

        assert_eq!(
            count_kinds(&findings),
            vec![
                (PerfFindingKind::SyncOverAsync, 1),
                (PerfFindingKind::UnstableClock, 2),
            ]
        );
    }

    #[test]
    fn a_kind_that_found_nothing_is_left_out_of_the_breakdown() {
        assert!(count_kinds(&[]).is_empty());
    }

    #[test]
    fn filtering_to_high_confidence_drops_the_guesses() {
        let settings = Settings {
            min_confidence: Confidence::High,
            ..Settings::default()
        };

        assert!(settings.wants(PerfFindingKind::SyncOverAsync));
        assert!(!settings.wants(PerfFindingKind::ListLookupInLoop));
    }

    #[test]
    fn a_disabled_check_is_dropped_whatever_its_confidence() {
        let settings = Settings {
            disabled_checks: vec![PerfFindingKind::SyncOverAsync],
            ..Settings::default()
        };

        assert!(!settings.wants(PerfFindingKind::SyncOverAsync));
        assert!(settings.wants(PerfFindingKind::ListLookupInLoop));
    }
}
