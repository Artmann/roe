use std::path::Path;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use globset::GlobSet;
use crate::extract::Interner;
use rustc_hash::FxHashMap;

use crate::cli::HealthArgs;
use crate::commands::Context;
use crate::config::EffectiveHealth;
use crate::model::{
    CircularDependency, CycleMember, HealthFinding, HealthFindingKind, HealthResult, HealthSummary,
    Hotspot, MemberBreakdown, MemberKind, ProjectId, SymbolId, SymbolKind, Workspace,
};
use crate::resolve::{Resolution, SymbolFlags};
use crate::{
    baseline, churn, commands, config, coupling, discover, extract, graph, hotspot, report,
    resolve, suppress,
};

pub struct Analysis {
    pub workspace: Workspace,
    pub result: HealthResult,
}

/// What to check and where. The thresholds are the limits a declaration has
/// to exceed to be reported; `exclude_tests` decides whether test projects are
/// looked at in the first place.
#[derive(Debug, Clone, Copy)]
pub struct Thresholds {
    pub max_cognitive: u32,
    pub max_complexity: u32,
    pub max_method_lines: u32,
    pub max_parameters: u32,
    pub max_file_lines: u32,
    pub max_type_members: u32,
    pub exclude_tests: bool,
}

impl From<EffectiveHealth> for Thresholds {
    fn from(effective: EffectiveHealth) -> Self {
        Thresholds {
            max_cognitive: effective.max_cognitive,
            max_complexity: effective.max_complexity,
            max_method_lines: effective.max_method_lines,
            max_parameters: effective.max_parameters,
            max_file_lines: effective.max_file_lines,
            max_type_members: effective.max_type_members,
            exclude_tests: effective.exclude_tests,
        }
    }
}

/// The full health pipeline: discover → extract → symbol table → reference
/// graph → complexity/size/coupling checks. No reachability analysis runs —
/// health flags issues in code regardless of whether it's dead.
pub fn analyze(root: &Path, thresholds: Thresholds) -> anyhow::Result<Analysis> {
    analyze_inner(root, thresholds, None, None, None)
}

/// The same pipeline, filtered against a [baseline](crate::baseline) file so
/// only findings it doesn't already record survive.
pub fn analyze_with_baseline(
    root: &Path,
    thresholds: Thresholds,
    baseline: &Path,
) -> anyhow::Result<Analysis> {
    analyze_inner(root, thresholds, None, None, Some(baseline))
}

/// Same pipeline, plus the two things only `run` needs: hotspot ranking (which
/// costs a git history walk, so it is opt-in) and dropping findings under a
/// config's `ignore` globs.
///
/// Ignore filtering happens after resolution, not at the file-list stage the
/// way `dupes` does it — `resolve::build_symbols` and `graph::build_graph`
/// both index into `workspace.files` by raw `FileId`, so dropping entries
/// beforehand would desync those positions. Hotspots are the exception: they
/// are scored relative to each other, so ignored files have to be dropped
/// before ranking rather than after, or they would skew the normalization and
/// eat slots in the `--top` list.
fn analyze_inner(
    root: &Path,
    thresholds: Thresholds,
    hotspots: Option<usize>,
    ignore: Option<(&[String], &Path)>,
    baseline: Option<&Path>,
) -> anyhow::Result<Analysis> {
    let start = Instant::now();

    let mut workspace = discover::discover(root)?;
    let rodeo = crate::extract::new_interner();
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

    // Built once, up front, because both the finding filter and the hotspot
    // ranking need it and the ranking needs it *before* it runs.
    let ignore_set = match ignore {
        Some((patterns, config_dir)) => {
            let mut warnings = Vec::new();
            let set = config::build_ignore_globset(config_dir, patterns, &mut warnings);
            workspace.warnings.append(&mut warnings);

            set
        }
        None => None,
    };

    let mut ranked = Vec::new();
    let mut commits_walked = None;

    if let Some(top) = hotspots {
        let (hotspots, walked, mut warnings) =
            collect_hotspots(&workspace, &facts, top, ignore_set.as_ref(), thresholds)?;
        ranked = hotspots;
        commits_walked = Some(walked);
        workspace.warnings.append(&mut warnings);
    }

    if let Some(set) = &ignore_set {
        findings.retain(|finding| !set.is_match(&finding.file));
        cycles.retain(|cycle| {
            cycle
                .path
                .iter()
                .chain(&cycle.others)
                .all(|member| !set.is_match(&member.file))
        });
    }

    let mut result = HealthResult {
        findings,
        cycles,
        hotspots: ranked,
        summary: HealthSummary::default(),
    };

    suppress::apply_inline_suppressions_health(&mut result, &mut workspace);

    // Applied before the summary so baselined findings inflate no count but
    // `baselined` itself, and after inline suppressions so an entry covering
    // something already suppressed reads as stale — which it is.
    let applied = match baseline {
        Some(path) => {
            let recorded = baseline::load(path)?;
            let applied = baseline::apply(&recorded, &mut result);

            if applied.stale > 0 {
                workspace.warnings.push(stale_warning(path, applied.stale));
            }

            Some(applied)
        }
        None => None,
    };

    result.summary = summarize(
        &workspace,
        &resolution,
        &result.findings,
        &result.cycles,
        thresholds,
        ignore_set.as_ref(),
        start.elapsed(),
    );
    result.summary.commits_walked = commits_walked;
    result.summary.baselined = applied.map(|applied| applied.hidden);

    Ok(Analysis { workspace, result })
}

/// A baseline entry matching nothing means the debt it recorded is gone —
/// good news, and never a failure. It is still worth saying, because until
/// the file is regenerated that entry would hide the same debt if it came
/// back.
fn stale_warning(path: &Path, stale: usize) -> String {
    let display = crate::paths::display(path);
    let (noun, verb) = if stale == 1 {
        ("entry", "matches")
    } else {
        ("entries", "match")
    };

    format!(
        "{stale} stale {noun} in {display} no longer {verb} any finding — regenerate it with `roe health --write-baseline {display}`"
    )
}

/// Rank the analyzed files by the product of their complexity density and
/// their recency-weighted change count.
///
/// Generated files are excluded — they change whenever their generator runs,
/// which would put them at the top of every list without telling anyone
/// anything. Ignored and (optionally) test files are excluded here rather
/// than from the ranked output, because `hotspot::rank` normalizes every
/// score against the highest one it is given: filtering afterwards would both
/// leave the surviving scores measured against a file the user excluded and
/// return fewer than `top` rows.
fn collect_hotspots(
    workspace: &Workspace,
    facts: &[extract::FileFacts],
    top: usize,
    ignore: Option<&GlobSet>,
    thresholds: Thresholds,
) -> anyhow::Result<(Vec<Hotspot>, usize, Vec<String>)> {
    let churn = churn::analyze(&workspace.root)?;

    let candidates: Vec<hotspot::Candidate> = facts
        .iter()
        .filter(|file_facts| !file_facts.is_generated)
        .filter_map(|file_facts| {
            let file = &workspace.files[file_facts.file.index()];

            if ignore.is_some_and(|set| set.is_match(&file.path)) {
                return None;
            }
            if thresholds.exclude_tests && in_test_project(workspace, file.project) {
                return None;
            }

            Some(hotspot::Candidate {
                file: file.path.clone(),
                project: project_name(workspace, file.project),
                cyclomatic: file_facts.decls.iter().map(|decl| decl.cyclomatic).sum(),
                lines: file_facts.line_count,
                weighted_commits: churn.weight(&file.path),
            })
        })
        .collect();

    Ok((
        hotspot::rank(&candidates, top),
        churn.commits_walked,
        churn.warnings,
    ))
}

fn in_test_project(workspace: &Workspace, project: Option<ProjectId>) -> bool {
    project.is_some_and(|id| workspace.projects[id.index()].is_test())
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
    rodeo: &Interner,
    thresholds: Thresholds,
) -> (Vec<HealthFinding>, Vec<CircularDependency>) {
    let mut findings = Vec::new();

    for file_facts in facts {
        if file_facts.is_generated {
            continue;
        }

        let file = &workspace.files[file_facts.file.index()];
        if thresholds.exclude_tests && in_test_project(workspace, file.project) {
            continue;
        }

        let project = project_name(workspace, file.project);
        let local_map = &resolution.decl_map[file_facts.file.index()];
        let names = member_names(resolution, file_facts, local_map, rodeo);

        for (local_index, decl) in file_facts.decls.iter().enumerate() {
            if !decl.kind.is_member() || !decl.has_body {
                continue;
            }

            let name = names[local_index].clone();

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
                    breakdown: None,
                    parameters: None,
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
                    breakdown: None,
                    parameters: None,
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
                    breakdown: None,
                    parameters: None,
                });
            }

            // Only the required parameters are measured — the threshold is
            // about call-site burden, and a defaulted or `out` parameter puts
            // none on the caller. The full declaration rides along so the
            // report can still show it.
            if decl.parameters.required > thresholds.max_parameters {
                findings.push(HealthFinding {
                    kind: HealthFindingKind::TooManyParameters,
                    name,
                    project: project.clone(),
                    file: file.path.clone(),
                    line: decl.line,
                    column: decl.column,
                    metric: decl.parameters.required,
                    threshold: thresholds.max_parameters,
                    breakdown: None,
                    parameters: Some(decl.parameters),
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
                breakdown: None,
                parameters: None,
            });
        }
    }

    // Kept as a breakdown rather than a bare count so the report can say what
    // the members *are*: thirty auto-properties is a data holder, thirty
    // methods is a god class, and only one of those is worth acting on.
    let mut member_counts: FxHashMap<SymbolId, MemberBreakdown> = FxHashMap::default();
    for symbol in &resolution.symbols {
        if symbol.flags.contains(SymbolFlags::GENERATED) {
            continue;
        }

        let (SymbolKind::Member(kind), Some(parent)) = (symbol.kind, symbol.parent) else {
            continue;
        };

        member_counts
            .entry(parent)
            .or_default()
            .record(kind, symbol.modifiers);
    }

    // Type-level dependency edges. Not reported as a finding of their own —
    // a high dependency count is normal in constructor-injected code — but
    // they're the edge set cycle detection runs over.
    let fan_out = coupling::fan_out(resolution, symbol_graph);

    for symbol in &resolution.symbols {
        if !symbol.kind.is_type() || symbol.flags.contains(SymbolFlags::GENERATED) {
            continue;
        }

        let breakdown = member_counts.get(&symbol.id).copied().unwrap_or_default();
        if breakdown.total() <= thresholds.max_type_members {
            continue;
        }

        let file = &workspace.files[symbol.file.index()];
        if thresholds.exclude_tests && in_test_project(workspace, file.project) {
            continue;
        }

        findings.push(HealthFinding {
            kind: HealthFindingKind::LargeType,
            name: resolution.display_name(symbol.id, rodeo),
            project: project_name(workspace, file.project),
            file: file.path.clone(),
            line: symbol.line,
            column: symbol.column,
            metric: breakdown.total(),
            threshold: thresholds.max_type_members,
            breakdown: Some(breakdown),
            parameters: None,
        });
    }

    findings.sort_by(|a, b| {
        a.file
            .cmp(&b.file)
            .then(a.line.cmp(&b.line))
            .then(a.column.cmp(&b.column))
    });

    let is_hidden = |&id: &SymbolId| {
        let symbol = &resolution.symbols[id.index()];

        if symbol.flags.contains(SymbolFlags::GENERATED) {
            return true;
        }

        let file = &workspace.files[symbol.file.index()];

        thresholds.exclude_tests && in_test_project(workspace, file.project)
    };

    // A cycle that touches hidden code anywhere — on the printed path or
    // merely tangled alongside it — is dropped whole rather than partially
    // reported, since a path with holes in it names edges that aren't there.
    // Generated tangles are the generator's problem, and `--exclude-tests`
    // means the user asked not to hear about test projects at all.
    let cycles = coupling::find_cycles(&fan_out)
        .into_iter()
        .filter(|cycle| !cycle.path.iter().chain(&cycle.others).any(is_hidden))
        .map(|cycle| CircularDependency {
            path: cycle
                .path
                .into_iter()
                .map(|id| cycle_member(resolution, workspace, rodeo, id))
                .collect(),
            others: cycle
                .others
                .into_iter()
                .map(|id| cycle_member(resolution, workspace, rodeo, id))
                .collect(),
        })
        .collect();

    (findings, cycles)
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
fn member_names(
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

fn cycle_member(
    resolution: &Resolution,
    workspace: &Workspace,
    rodeo: &Interner,
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

/// The scan totals count what was *eligible to be reported*, not what was
/// parsed: a run that skipped a whole test project and still claimed to have
/// scanned it left the reader with no way to confirm their setting applied.
/// Whatever was ruled out is counted separately so the report can name it.
fn summarize(
    workspace: &Workspace,
    resolution: &Resolution,
    findings: &[HealthFinding],
    cycles: &[CircularDependency],
    thresholds: Thresholds,
    ignore: Option<&GlobSet>,
    elapsed: Duration,
) -> HealthSummary {
    let count = |kind: HealthFindingKind| findings.iter().filter(|f| f.kind == kind).count();

    // Indexed by raw `FileId`, so the symbol pass can reuse it rather than
    // re-matching every glob.
    let mut eligible = Vec::with_capacity(workspace.files.len());
    let mut excluded_files = 0;

    for file in &workspace.files {
        let in_excluded_test = thresholds.exclude_tests && in_test_project(workspace, file.project);
        let is_ignored = ignore.is_some_and(|set| set.is_match(&file.path));

        // Generated files were never eligible under any setting, so counting
        // them here would report an exclusion the user did not choose. Files
        // already dropped with their test project are reported as that
        // project, not twice.
        if is_ignored && !in_excluded_test && !file.is_generated {
            excluded_files += 1;
        }

        eligible.push(!in_excluded_test && !is_ignored);
    }

    let mut excluded_test_projects: Vec<String> = if thresholds.exclude_tests {
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

    HealthSummary {
        projects: workspace.projects.len() - excluded_test_projects.len(),
        files_scanned: workspace
            .files
            .iter()
            .zip(&eligible)
            .filter(|(file, is_eligible)| **is_eligible && !file.is_generated)
            .count(),
        symbols: resolution
            .symbols
            .iter()
            .filter(|symbol| eligible[symbol.file.index()])
            .count(),
        excluded_test_projects,
        excluded_files,
        high_complexity: count(HealthFindingKind::HighComplexity),
        high_cognitive_complexity: count(HealthFindingKind::HighCognitiveComplexity),
        long_methods: count(HealthFindingKind::LongMethod),
        too_many_parameters: count(HealthFindingKind::TooManyParameters),
        large_files: count(HealthFindingKind::LargeFile),
        large_types: count(HealthFindingKind::LargeType),
        circular_dependencies: cycles.len(),
        commits_walked: None,
        baselined: None,
        elapsed_ms: elapsed.as_millis(),
    }
}

/// The analysis half of `run`: merge the config's `health` block with the
/// command line, then analyze. Prints nothing and decides no exit code, so
/// `check` can run it alongside the other two analyses.
///
/// `--write-baseline` suppresses baseline filtering even when a config names
/// one, so the recorded file describes the codebase rather than whatever the
/// previous baseline left over.
pub(crate) fn execute(context: &Context, args: &HealthArgs) -> anyhow::Result<Analysis> {
    let health = context
        .config
        .as_ref()
        .and_then(|resolved| resolved.config.health.as_ref());
    let baseline = match args.write_baseline {
        Some(_) => None,
        None => config::resolve_baseline(
            health,
            context
                .config
                .as_ref()
                .map(|resolved| resolved.dir.as_path()),
            args.baseline.as_deref(),
        ),
    };

    let effective = config::merge_health(
        health,
        config::HealthOverrides {
            max_complexity: args.max_complexity,
            max_cognitive: args.max_cognitive,
            max_method_lines: args.max_method_lines,
            max_parameters: args.max_parameters,
            max_file_lines: args.max_file_lines,
            max_type_members: args.max_type_members,
            exclude_tests: args.exclude_tests,
        },
    );

    let hotspots = args.hotspots.then_some(args.top);
    let mut analysis = analyze_inner(
        &context.root,
        effective.into(),
        hotspots,
        context.ignore(),
        baseline.as_deref(),
    )?;
    analysis
        .workspace
        .warnings
        .extend(context.warnings.iter().cloned());

    Ok(analysis)
}

pub fn run(args: &HealthArgs) -> anyhow::Result<ExitCode> {
    let context = commands::resolve(&args.path, &args.config)?;
    let analysis = execute(&context, args)?;

    for warning in &analysis.workspace.warnings {
        eprintln!("warning: {warning}");
    }

    // Recording the codebase rather than judging it: no report, and always a
    // clean exit, or the very first CI run would fail on the debt it was
    // being told to accept.
    if let Some(path) = &args.write_baseline {
        let (findings, cycles) = baseline::write(path, &analysis.result, &analysis.workspace.root)?;
        eprintln!(
            "wrote {findings} finding(s) and {cycles} cycle(s) to {}",
            crate::paths::display(path)
        );

        return Ok(ExitCode::SUCCESS);
    }

    Ok(report::health::emit(
        &analysis.result,
        &analysis.workspace,
        args.format,
        args.sort,
        args.limit,
    ))
}
