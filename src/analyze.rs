use std::time::Duration;

use fixedbitset::FixedBitSet;
use lasso::ThreadedRodeo;
use rustc_hash::FxHashSet;

use crate::model::{
    AnalysisResult, Finding, FindingKind, Summary, SymbolId, SymbolKind, Workspace,
};
use crate::resolve::{Resolution, SymbolFlags};

/// Run the detectors over the marked graph and assemble findings.
pub fn find_dead(
    resolution: &Resolution,
    reachable: &FixedBitSet,
    workspace: &Workspace,
    rodeo: &ThreadedRodeo,
    elapsed: Duration,
    notes: Vec<String>,
) -> AnalysisResult {
    let mut flagged: FxHashSet<SymbolId> = FxHashSet::default();

    for symbol in &resolution.symbols {
        if symbol.kind == SymbolKind::FileRoot
            || symbol.flags.contains(SymbolFlags::GENERATED)
            || symbol.flags.contains(SymbolFlags::ROOT)
            || reachable.contains(symbol.id.index())
        {
            continue;
        }
        match symbol.kind {
            SymbolKind::Type(_) => {
                flagged.insert(symbol.id);
            }
            SymbolKind::Member(_) => {
                // Members of dead types are subsumed by the type finding.
                let parent_reachable = symbol.parent.is_some_and(|p| reachable.contains(p.index()));
                if parent_reachable && !symbol.flags.contains(SymbolFlags::LIVE_WITH_TYPE) {
                    flagged.insert(symbol.id);
                }
            }
            SymbolKind::FileRoot => unreachable!(),
        }
    }

    // Wholly-dead files: every symbol declared in the file is dead (partial
    // types keep their file alive from any live part, because the merged
    // symbol appears in this file's decl list).
    let mut dead_files: FxHashSet<usize> = FxHashSet::default();
    for (file_index, locals) in resolution.decl_map.iter().enumerate() {
        let file = &workspace.files[file_index];
        if file.is_generated || locals.is_empty() {
            continue;
        }
        let all_dead = locals.iter().all(|id| {
            let symbol = &resolution.symbols[id.index()];
            flagged.contains(id)
                || (symbol.kind.is_member() && symbol.parent.is_some_and(|p| flagged.contains(&p)))
        });
        if all_dead {
            dead_files.insert(file_index);
        }
    }

    let mut findings: Vec<Finding> = Vec::new();
    let mut summary = Summary {
        projects: workspace.projects.len(),
        files_scanned: workspace.files.len(),
        symbols: resolution.symbols.len(),
        elapsed_ms: elapsed.as_millis(),
        ..Summary::default()
    };

    for &file_index in &dead_files {
        let file = &workspace.files[file_index];
        summary.unused_files += 1;
        findings.push(Finding {
            kind: FindingKind::UnusedFile,
            symbol_kind: None,
            name: display_path(workspace, file_index),
            project: project_name(workspace, file_index),
            file: file.path.clone(),
            line: 1,
            column: 1,
            visibility: None,
        });
    }

    for symbol in &resolution.symbols {
        if !flagged.contains(&symbol.id) || dead_files.contains(&symbol.file.index()) {
            continue;
        }
        let kind = if symbol.kind.is_type() {
            summary.unused_types += 1;
            FindingKind::UnusedType
        } else {
            summary.unused_members += 1;
            FindingKind::UnusedMember
        };
        findings.push(Finding {
            kind,
            symbol_kind: Some(symbol.kind),
            name: resolution.display_name(symbol.id, rodeo),
            project: project_name(workspace, symbol.file.index()),
            file: workspace.files[symbol.file.index()].path.clone(),
            line: symbol.line,
            column: symbol.column,
            visibility: Some(symbol.visibility()),
        });
    }

    findings.sort_by(|a, b| {
        (a.kind != FindingKind::UnusedFile)
            .cmp(&(b.kind != FindingKind::UnusedFile))
            .then_with(|| a.file.cmp(&b.file))
            .then_with(|| a.line.cmp(&b.line))
    });

    AnalysisResult {
        findings,
        summary,
        notes,
    }
}

fn project_name(workspace: &Workspace, file_index: usize) -> Option<String> {
    workspace.files[file_index]
        .project
        .map(|p| workspace.projects[p.index()].name.clone())
}

fn display_path(workspace: &Workspace, file_index: usize) -> String {
    let path = &workspace.files[file_index].path;
    path.strip_prefix(&workspace.root)
        .unwrap_or(path)
        .display()
        .to_string()
}
