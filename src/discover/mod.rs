mod csproj;
mod sln;
mod sources;
mod walk;

pub use csproj::{CsprojData, parse_csproj};
pub use sln::parse_sln;
pub use sources::is_generated_path;

use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::model::{DiscoveredProject, FileId, ProjectId, ProjectKind, SourceFile, Workspace};
use sources::{Claim, SourceRules};

/// Discover the workspace under `input`, which may be a directory, a .sln
/// file, or a .csproj file.
pub fn discover(input: &Path) -> anyhow::Result<Workspace> {
    let input = crate::paths::canonicalize(input)
        .with_context(|| format!("path not found: {}", input.display()))?;

    let (root, explicit_sln, explicit_csproj) = classify_input(&input)?;
    let walked = walk::walk(&root);

    let mut warnings: Vec<String> = walked
        .skipped_large
        .iter()
        .map(|p| format!("skipped large file (>5 MB): {}", p.display()))
        .collect();

    let csproj_paths = collect_csproj_paths(
        &root,
        explicit_sln,
        explicit_csproj,
        &walked.sln_files,
        &walked.csproj_files,
        &mut warnings,
    );

    let (projects, rules) = build_projects(&root, &csproj_paths, &mut warnings);
    let (files, missing_obj) =
        collect_files(&root, &projects, &rules, &walked.cs_files, &mut warnings);

    Ok(Workspace {
        root,
        projects,
        files,
        warnings,
        missing_obj,
    })
}

fn classify_input(input: &Path) -> anyhow::Result<(PathBuf, Option<PathBuf>, Option<PathBuf>)> {
    if input.is_dir() {
        return Ok((input.to_path_buf(), None, None));
    }
    let parent = input
        .parent()
        .map(Path::to_path_buf)
        .context("input file has no parent directory")?;
    match input
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("sln") => Ok((parent, Some(input.to_path_buf()), None)),
        Some("csproj") => Ok((parent, None, Some(input.to_path_buf()))),
        _ => bail!(
            "expected a directory, .sln, or .csproj path: {}",
            input.display()
        ),
    }
}

fn collect_csproj_paths(
    root: &Path,
    explicit_sln: Option<PathBuf>,
    explicit_csproj: Option<PathBuf>,
    sln_files: &[PathBuf],
    csproj_files: &[PathBuf],
    warnings: &mut Vec<String>,
) -> Vec<PathBuf> {
    if let Some(csproj) = explicit_csproj {
        return vec![csproj];
    }

    let sln = explicit_sln.or_else(|| pick_sln(root, sln_files, warnings));
    let mut paths = Vec::new();
    let mut seen = FxHashSet::default();

    if let Some(sln_path) = sln {
        let sln_dir = sln_path.parent().unwrap_or(root).to_path_buf();
        match std::fs::read_to_string(&sln_path) {
            Ok(content) => {
                for project in parse_sln(&content) {
                    let candidate = sln_dir.join(&project.relative_path);
                    match crate::paths::canonicalize(&candidate) {
                        Ok(path) => {
                            if seen.insert(path.clone()) {
                                paths.push(path);
                            }
                        }
                        Err(_) => warnings.push(format!(
                            "project listed in {} not found: {}",
                            sln_path.display(),
                            candidate.display()
                        )),
                    }
                }
            }
            Err(error) => {
                warnings.push(format!("failed to read {}: {error}", sln_path.display()));
            }
        }
        if !paths.is_empty() {
            return paths;
        }
    }

    for csproj in csproj_files {
        if let Ok(path) = crate::paths::canonicalize(csproj)
            && seen.insert(path.clone())
        {
            paths.push(path);
        }
    }
    paths
}

/// Prefer the shallowest solution file; warn when several exist.
fn pick_sln(root: &Path, sln_files: &[PathBuf], warnings: &mut Vec<String>) -> Option<PathBuf> {
    let chosen = sln_files
        .iter()
        .min_by_key(|p| {
            (
                p.strip_prefix(root)
                    .map_or(usize::MAX, |r| r.components().count()),
                (*p).clone(),
            )
        })?
        .clone();
    if sln_files.len() > 1 {
        let others: Vec<String> = sln_files
            .iter()
            .filter(|p| **p != chosen)
            .map(|p| p.display().to_string())
            .collect();
        warnings.push(format!(
            "multiple solutions found; using {} (ignoring {})",
            chosen.display(),
            others.join(", ")
        ));
    }
    Some(chosen)
}

fn build_projects(
    root: &Path,
    csproj_paths: &[PathBuf],
    warnings: &mut Vec<String>,
) -> (Vec<DiscoveredProject>, Vec<SourceRules>) {
    let mut projects = Vec::new();
    let mut rules = Vec::new();

    if csproj_paths.is_empty() {
        let id = ProjectId(0);
        let name = root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "workspace".to_string());
        projects.push(DiscoveredProject {
            id,
            name,
            csproj_path: None,
            root_dir: root.to_path_buf(),
            kind: ProjectKind::Library,
            project_refs: Vec::new(),
            package_refs: Vec::new(),
            test_framework: None,
            implicit_usings: true,
            extra_usings: Vec::new(),
            is_packable: false,
        });
        rules.push(SourceRules::new(
            id,
            root.to_path_buf(),
            true,
            &[],
            &[],
            warnings,
        ));
        return (projects, rules);
    }

    let mut id_by_path: FxHashMap<PathBuf, ProjectId> = FxHashMap::default();
    let mut raw_refs: Vec<Vec<String>> = Vec::new();

    for (index, csproj_path) in csproj_paths.iter().enumerate() {
        let id = ProjectId(index as u32);
        let root_dir = csproj_path.parent().unwrap_or(root).to_path_buf();
        let name = csproj_path
            .file_stem()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| format!("project-{index}"));

        let data = match std::fs::read_to_string(csproj_path) {
            Ok(content) => match parse_csproj(&content) {
                Ok(data) => data,
                Err(error) => {
                    warnings.push(format!(
                        "failed to parse {}: {error}",
                        csproj_path.display()
                    ));
                    CsprojData {
                        is_sdk_style: true,
                        enable_default_compile_items: true,
                        ..CsprojData::default()
                    }
                }
            },
            Err(error) => {
                warnings.push(format!("failed to read {}: {error}", csproj_path.display()));
                CsprojData {
                    is_sdk_style: true,
                    enable_default_compile_items: true,
                    ..CsprojData::default()
                }
            }
        };

        id_by_path.insert(csproj_path.clone(), id);
        rules.push(SourceRules::new(
            id,
            root_dir.clone(),
            data.enable_default_compile_items,
            &data.compile_includes,
            &data.compile_removes,
            warnings,
        ));
        projects.push(DiscoveredProject {
            id,
            name,
            csproj_path: Some(csproj_path.clone()),
            root_dir,
            kind: data.kind(),
            project_refs: Vec::new(),
            package_refs: data.package_refs.clone(),
            test_framework: data.test_framework(),
            implicit_usings: data.implicit_usings,
            extra_usings: data.usings.clone(),
            is_packable: data.is_packable(),
        });
        raw_refs.push(data.project_refs);
    }

    for (index, refs) in raw_refs.iter().enumerate() {
        let root_dir = projects[index].root_dir.clone();
        for raw in refs {
            if let Ok(target) = crate::paths::canonicalize(&root_dir.join(raw))
                && let Some(&target_id) = id_by_path.get(&target)
            {
                projects[index].project_refs.push(target_id);
            }
        }
    }

    (projects, rules)
}

fn collect_files(
    root: &Path,
    projects: &[DiscoveredProject],
    rules: &[SourceRules],
    walked_cs: &[PathBuf],
    warnings: &mut Vec<String>,
) -> (Vec<SourceFile>, bool) {
    // Longest project root wins when projects nest.
    let mut order: Vec<usize> = (0..rules.len()).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(rules[i].root_dir.components().count()));

    let mut seen: FxHashSet<PathBuf> = FxHashSet::default();
    let mut entries: Vec<(PathBuf, Option<ProjectId>, bool)> = Vec::new();

    for path in walked_cs {
        let mut project = None;
        let mut removed = false;
        for &i in &order {
            if path.starts_with(&rules[i].root_dir) {
                match rules[i].claims(path) {
                    Claim::Included => project = Some(rules[i].project),
                    Claim::Removed => removed = true,
                    // Present on disk but not compiled by its old-style
                    // project: keep it as an unassigned source (its
                    // references still count; its declarations are real
                    // orphans worth reporting).
                    Claim::NotListed => {}
                }
                break;
            }
        }
        if removed {
            continue;
        }
        if seen.insert(path.clone()) {
            entries.push((path.clone(), project, is_generated_path(path)));
        }
    }

    // Compile Include can pull in files from outside the walked tree
    // (e.g. ..\Shared\Version.cs).
    for rule in rules {
        for path in &rule.literal_includes {
            if seen.insert(path.clone()) {
                entries.push((path.clone(), Some(rule.project), is_generated_path(path)));
            }
        }
    }

    // Harvest obj/ for generated sources (reference-only).
    let mut any_obj = false;
    for project in projects {
        let obj_files = walk::walk_obj(&project.root_dir);
        if !obj_files.is_empty() {
            any_obj = true;
        }
        for path in obj_files {
            if seen.insert(path.clone()) {
                entries.push((path, Some(project.id), true));
            }
        }
    }
    let missing_obj = !any_obj;
    if missing_obj {
        warnings.push(format!(
            "no obj/ directories found under {} — build the solution for better results \
             (generated sources like GlobalUsings and source-generator output are invisible)",
            root.display()
        ));
    }

    entries.sort_by(|a, b| a.0.cmp(&b.0));
    let files = entries
        .into_iter()
        .enumerate()
        .map(|(index, (path, project, is_generated))| SourceFile {
            id: FileId(index as u32),
            path,
            project,
            is_generated,
        })
        .collect();

    (files, missing_obj)
}
