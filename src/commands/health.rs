use std::path::Path;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use anyhow::Context;
use lasso::ThreadedRodeo;
use rustc_hash::FxHashMap;

use crate::cli::HealthArgs;
use crate::model::{
    CircularDependency, CycleMember, HealthFinding, HealthFindingKind, HealthResult, HealthSummary,
    Hotspot, ProjectId, SymbolId, Workspace,
};
use crate::resolve::{Resolution, SymbolFlags};
use crate::{churn, config, coupling, discover, extract, graph, hotspot, report, resolve};

pub struct Analysis {
    pub workspace: Workspace,
    pub result: HealthResult,
}

#[derive(Debug, Clone, Copy)]
pub struct Thresholds {
    pub max_cognitive: u32,
    pub max_complexity: u32,
    pub max_method_lines: u32,
    pub max_parameters: u32,
    pub max_file_lines: u32,
    pub max_type_members: u32,
}

/// The full health pipeline: discover → extract → symbol table → reference
/// graph → complexity/size/coupling checks. No reachability analysis runs —
/// health flags issues in code regardless of whether it's dead.
pub fn analyze(root: &Path, thresholds: Thresholds) -> anyhow::Result<Analysis> {
    analyze_inner(root, thresholds, None, None)
}

/// Same pipeline, plus the two things only `run` needs: hotspot ranking (which
/// costs a git history walk, so it is opt-in) and dropping findings under a
/// config's `ignore` globs.
///
/// Ignore filtering happens after resolution, not at the file-list stage the
/// way `dupes` does it — `resolve::build_symbols` and `graph::build_graph`
/// both index into `workspace.files` by raw `FileId`, so dropping entries
/// beforehand would desync those positions.
fn analyze_inner(
    root: &Path,
    thresholds: Thresholds,
    hotspots: Option<usize>,
    ignore: Option<(&[String], &Path)>,
) -> anyhow::Result<Analysis> {
    let start = Instant::now();

    let mut workspace = discover::discover(root)?;
    let rodeo = ThreadedRodeo::default();
    let facts = extract::extract_all(&workspace.files, &rodeo);
    let mut resolution = resolve::build_symbols(&workspace.files, &facts, &rodeo);
    let symbol_graph = graph::build_graph(&mut resolution, &workspace, &facts, &rodeo);

    let (mut findings, mut cycles) = collect_findings(
        &resolution,
        &symbol_graph,
        &workspace,
        &facts,
        &rodeo,
        thresholds,
    );

    let mut ranked = Vec::new();
    let mut commits_walked = None;

    if let Some(top) = hotspots {
        let (hotspots, walked, mut warnings) = collect_hotspots(&workspace, &facts, top)?;
        ranked = hotspots;
        commits_walked = Some(walked);
        workspace.warnings.append(&mut warnings);
    }

    if let Some((patterns, config_dir)) = ignore {
        let mut warnings = Vec::new();
        if let Some(set) = config::build_ignore_globset(config_dir, patterns, &mut warnings) {
            findings.retain(|finding| !set.is_match(&finding.file));
            cycles.retain(|cycle| {
                cycle
                    .members
                    .iter()
                    .all(|member| !set.is_match(&member.file))
            });
            ranked.retain(|hotspot| !set.is_match(&hotspot.file));
        }
        workspace.warnings.append(&mut warnings);
    }

    let mut summary = summarize(&workspace, &resolution, &findings, &cycles, start.elapsed());
    summary.commits_walked = commits_walked;

    let result = HealthResult {
        findings,
        cycles,
        hotspots: ranked,
        summary,
    };

    Ok(Analysis { workspace, result })
}

/// Rank the analyzed files by the product of their complexity density and
/// their recency-weighted change count.
///
/// Generated files are excluded — they change whenever their generator runs,
/// which would put them at the top of every list without telling anyone
/// anything.
fn collect_hotspots(
    workspace: &Workspace,
    facts: &[extract::FileFacts],
    top: usize,
) -> anyhow::Result<(Vec<Hotspot>, usize, Vec<String>)> {
    let churn = churn::analyze(&workspace.root)?;

    let candidates: Vec<hotspot::Candidate> = facts
        .iter()
        .filter(|file_facts| !file_facts.is_generated)
        .map(|file_facts| {
            let file = &workspace.files[file_facts.file.index()];

            hotspot::Candidate {
                file: file.path.clone(),
                project: project_name(workspace, file.project),
                cyclomatic: file_facts.decls.iter().map(|decl| decl.cyclomatic).sum(),
                lines: file_facts.line_count,
                weighted_commits: churn.weight(&file.path),
            }
        })
        .collect();

    Ok((
        hotspot::rank(&candidates, top),
        churn.commits_walked,
        churn.warnings,
    ))
}

/// Member-level (complexity, method length, parameter count), file-level
/// (line count), and type-level (member count, cycles) findings.
/// Generated declarations are skipped throughout — they're the generator's
/// business, not the user's.
fn collect_findings(
    resolution: &Resolution,
    symbol_graph: &graph::SymbolGraph,
    workspace: &Workspace,
    facts: &[extract::FileFacts],
    rodeo: &ThreadedRodeo,
    thresholds: Thresholds,
) -> (Vec<HealthFinding>, Vec<CircularDependency>) {
    let mut findings = Vec::new();

    for file_facts in facts {
        if file_facts.is_generated {
            continue;
        }

        let file = &workspace.files[file_facts.file.index()];
        let project = project_name(workspace, file.project);
        let local_map = &resolution.decl_map[file_facts.file.index()];

        for (local_index, decl) in file_facts.decls.iter().enumerate() {
            if !decl.kind.is_member() || !decl.has_body {
                continue;
            }

            let name = resolution.display_name(local_map[local_index], rodeo);

            if decl.cyclomatic > thresholds.max_complexity {
                findings.push(HealthFinding {
                    kind: HealthFindingKind::HighComplexity,
                    name: name.clone(),
                    project: project.clone(),
                    file: file.path.clone(),
                    line: decl.line,
                    column: decl.column,
                    metric: decl.cyclomatic,
                    threshold: thresholds.max_complexity,
                });
            }

            if decl.cognitive > thresholds.max_cognitive {
                findings.push(HealthFinding {
                    kind: HealthFindingKind::HighCognitiveComplexity,
                    name: name.clone(),
                    project: project.clone(),
                    file: file.path.clone(),
                    line: decl.line,
                    column: decl.column,
                    metric: decl.cognitive,
                    threshold: thresholds.max_cognitive,
                });
            }

            if decl.body_lines > thresholds.max_method_lines {
                findings.push(HealthFinding {
                    kind: HealthFindingKind::LongMethod,
                    name: name.clone(),
                    project: project.clone(),
                    file: file.path.clone(),
                    line: decl.line,
                    column: decl.column,
                    metric: decl.body_lines,
                    threshold: thresholds.max_method_lines,
                });
            }

            if decl.parameter_count > thresholds.max_parameters {
                findings.push(HealthFinding {
                    kind: HealthFindingKind::TooManyParameters,
                    name,
                    project: project.clone(),
                    file: file.path.clone(),
                    line: decl.line,
                    column: decl.column,
                    metric: decl.parameter_count,
                    threshold: thresholds.max_parameters,
                });
            }
        }

        if file_facts.line_count > thresholds.max_file_lines {
            findings.push(HealthFinding {
                kind: HealthFindingKind::LargeFile,
                name: crate::paths::display(
                    file.path
                        .strip_prefix(&workspace.root)
                        .unwrap_or(&file.path),
                ),
                project,
                file: file.path.clone(),
                line: 1,
                column: 1,
                metric: file_facts.line_count,
                threshold: thresholds.max_file_lines,
            });
        }
    }

    let mut member_counts: FxHashMap<SymbolId, u32> = FxHashMap::default();
    for symbol in &resolution.symbols {
        if symbol.kind.is_member()
            && !symbol.flags.contains(SymbolFlags::GENERATED)
            && let Some(parent) = symbol.parent
        {
            *member_counts.entry(parent).or_default() += 1;
        }
    }

    // Type-level dependency edges. Not reported as a finding of their own —
    // a high dependency count is normal in constructor-injected code — but
    // they're the edge set cycle detection runs over.
    let fan_out = coupling::fan_out(resolution, symbol_graph);

    for symbol in &resolution.symbols {
        if !symbol.kind.is_type() || symbol.flags.contains(SymbolFlags::GENERATED) {
            continue;
        }

        let member_count = member_counts.get(&symbol.id).copied().unwrap_or(0);
        if member_count <= thresholds.max_type_members {
            continue;
        }

        let file = &workspace.files[symbol.file.index()];

        findings.push(HealthFinding {
            kind: HealthFindingKind::LargeType,
            name: resolution.display_name(symbol.id, rodeo),
            project: project_name(workspace, file.project),
            file: file.path.clone(),
            line: symbol.line,
            column: symbol.column,
            metric: member_count,
            threshold: thresholds.max_type_members,
        });
    }

    findings.sort_by(|a, b| {
        a.file
            .cmp(&b.file)
            .then(a.line.cmp(&b.line))
            .then(a.column.cmp(&b.column))
    });

    let cycles = coupling::find_cycles(&fan_out)
        .into_iter()
        .filter(|members| {
            members.iter().all(|&id| {
                !resolution.symbols[id.index()]
                    .flags
                    .contains(SymbolFlags::GENERATED)
            })
        })
        .map(|members| CircularDependency {
            members: members
                .into_iter()
                .map(|id| cycle_member(resolution, workspace, rodeo, id))
                .collect(),
        })
        .collect();

    (findings, cycles)
}

fn cycle_member(
    resolution: &Resolution,
    workspace: &Workspace,
    rodeo: &ThreadedRodeo,
    id: SymbolId,
) -> CycleMember {
    let symbol = &resolution.symbols[id.index()];
    let file = &workspace.files[symbol.file.index()];

    CycleMember {
        name: resolution.display_name(id, rodeo),
        project: project_name(workspace, file.project),
        file: file.path.clone(),
        line: symbol.line,
        column: symbol.column,
    }
}

fn project_name(workspace: &Workspace, project: Option<ProjectId>) -> Option<String> {
    project.map(|id| workspace.projects[id.index()].name.clone())
}

fn summarize(
    workspace: &Workspace,
    resolution: &Resolution,
    findings: &[HealthFinding],
    cycles: &[CircularDependency],
    elapsed: Duration,
) -> HealthSummary {
    let count = |kind: HealthFindingKind| findings.iter().filter(|f| f.kind == kind).count();

    HealthSummary {
        projects: workspace.projects.len(),
        files_scanned: workspace
            .files
            .iter()
            .filter(|file| !file.is_generated)
            .count(),
        symbols: resolution.symbols.len(),
        high_complexity: count(HealthFindingKind::HighComplexity),
        high_cognitive_complexity: count(HealthFindingKind::HighCognitiveComplexity),
        long_methods: count(HealthFindingKind::LongMethod),
        too_many_parameters: count(HealthFindingKind::TooManyParameters),
        large_files: count(HealthFindingKind::LargeFile),
        large_types: count(HealthFindingKind::LargeType),
        circular_dependencies: cycles.len(),
        commits_walked: None,
        elapsed_ms: elapsed.as_millis(),
    }
}

pub fn run(args: &HealthArgs) -> anyhow::Result<ExitCode> {
    let root = match &args.path {
        Some(path) => path.clone(),
        None => std::env::current_dir()?,
    };

    let thresholds = Thresholds {
        max_cognitive: args.max_cognitive,
        max_complexity: args.max_complexity,
        max_method_lines: args.max_method_lines,
        max_parameters: args.max_parameters,
        max_file_lines: args.max_file_lines,
        max_type_members: args.max_type_members,
    };

    let mut config_warnings = Vec::new();
    let resolved_config = match &args.config {
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

    let hotspots = args.hotspots.then_some(args.top);
    let mut analysis = analyze_inner(&root, thresholds, hotspots, ignore)?;
    analysis.workspace.warnings.append(&mut config_warnings);

    for warning in &analysis.workspace.warnings {
        eprintln!("warning: {warning}");
    }

    Ok(report::health::emit(
        &analysis.result,
        &analysis.workspace,
        args.format,
    ))
}
