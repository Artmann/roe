use std::collections::VecDeque;

use fixedbitset::FixedBitSet;
use lasso::{Spur, ThreadedRodeo};
use rustc_hash::FxHashMap;
use smallvec::SmallVec;

use crate::extract::{FILE_ROOT, FileFacts, RawRefKind};
use crate::model::{ProjectId, SymbolId, Workspace};
use crate::resolve::{Resolution, SymbolFlags};

pub struct SymbolGraph {
    /// Flat CSR edge storage: materialized reference + structural edges.
    edges: Vec<SymbolId>,
    edge_ranges: Vec<(u32, u32)>,
    /// Flat CSR of member names referenced by each symbol's body. These are
    /// NOT materialized as edges — the BFS type-gates them: a member becomes
    /// reachable only when its name is live AND its containing type is.
    member_names: Vec<Spur>,
    member_name_ranges: Vec<(u32, u32)>,
    /// Containment-down: per symbol, its live-with-type members.
    live_children: Vec<SmallVec<[SymbolId; 2]>>,
}

impl SymbolGraph {
    pub fn edges_of(&self, id: SymbolId) -> &[SymbolId] {
        let (start, end) = self.edge_ranges[id.index()];
        &self.edges[start as usize..end as usize]
    }

    pub fn member_names_of(&self, id: SymbolId) -> &[Spur] {
        let (start, end) = self.member_name_ranges[id.index()];
        &self.member_names[start as usize..end as usize]
    }
}

/// Per-file resolution context: namespace usings and aliases in scope.
struct FileContext {
    usings: Vec<Vec<Spur>>,
    aliases: FxHashMap<Spur, Vec<Spur>>,
    has_errors: bool,
}

/// Build reference edges by resolving every raw reference against the symbol
/// table, then flatten into CSR form. Also roots implementations of
/// scan-target types (reflection-based registration).
pub fn build_graph(
    resolution: &mut Resolution,
    workspace: &Workspace,
    facts: &[FileFacts],
    rodeo: &ThreadedRodeo,
) -> SymbolGraph {
    let symbol_count = resolution.symbols.len();
    let mut edge_lists: Vec<Vec<SymbolId>> = vec![Vec::new(); symbol_count];
    let mut name_lists: Vec<Vec<Spur>> = vec![Vec::new(); symbol_count];
    let mut scan_targets: rustc_hash::FxHashSet<SymbolId> = rustc_hash::FxHashSet::default();

    // Project-wide global usings: `global using` directives from any file of
    // the project, plus csproj <Using Include> items.
    let mut project_globals: FxHashMap<ProjectId, Vec<Vec<Spur>>> = FxHashMap::default();
    for project in &workspace.projects {
        let mut globals: Vec<Vec<Spur>> = Vec::new();
        for using in &project.extra_usings {
            globals.push(using.split('.').map(|s| rodeo.get_or_intern(s)).collect());
        }
        project_globals.insert(project.id, globals);
    }
    let mut all_globals: Vec<Vec<Spur>> = Vec::new();
    for file_facts in facts {
        for using in &file_facts.usings {
            if using.is_global && using.alias.is_none() && !using.is_static {
                let project = workspace.files[file_facts.file.index()].project;
                all_globals.push(using.path.clone());
                if let Some(project) = project
                    && let Some(globals) = project_globals.get_mut(&project)
                {
                    globals.push(using.path.clone());
                }
            }
        }
    }

    let resolver = Resolver {
        resolution: &*resolution,
        rodeo,
    };

    for file_facts in facts {
        let file = &workspace.files[file_facts.file.index()];
        let mut context = FileContext {
            usings: Vec::new(),
            aliases: FxHashMap::default(),
            has_errors: file_facts.has_errors,
        };
        for using in &file_facts.usings {
            if using.is_static {
                continue; // target already emitted as a Type reference
            }
            match using.alias {
                Some(alias) => {
                    context.aliases.insert(alias, using.path.clone());
                }
                None => context.usings.push(using.path.clone()),
            }
        }
        // Files with a project see that project's globals; orphan files see
        // every global (over-inclusion is safe).
        match file.project.and_then(|p| project_globals.get(&p)) {
            Some(globals) => context.usings.extend(globals.iter().cloned()),
            None => context.usings.extend(all_globals.iter().cloned()),
        }

        let local_map = &resolution.decl_map[file_facts.file.index()];
        let file_root = resolution.file_roots[file_facts.file.index()];

        for raw_ref in &file_facts.refs {
            let origin = if raw_ref.origin == FILE_ROOT {
                file_root
            } else {
                local_map[raw_ref.origin as usize]
            };

            match raw_ref.kind {
                RawRefKind::Type => {
                    resolver.resolve_type_path(
                        &raw_ref.path,
                        origin,
                        &context,
                        &mut edge_lists[origin.index()],
                        &mut name_lists[origin.index()],
                    );
                }
                RawRefKind::Member => {
                    if let Some(&last) = raw_ref.path.last() {
                        name_lists[origin.index()].push(last);
                    }
                }
                RawRefKind::Ambient => {
                    resolver.resolve_type_path(
                        &raw_ref.path,
                        origin,
                        &context,
                        &mut edge_lists[origin.index()],
                        &mut name_lists[origin.index()],
                    );
                    if raw_ref.path.len() == 1 {
                        name_lists[origin.index()].push(raw_ref.path[0]);
                    }
                }
                RawRefKind::ScanTarget => {
                    // The generic argument itself is already a Type reference
                    // (emitted by the generic-name walk); here we only record
                    // which in-source types are reflection-scan contracts.
                    let mut targets = Vec::new();
                    let mut discard = Vec::new();
                    resolver.resolve_type_path(
                        &raw_ref.path,
                        origin,
                        &context,
                        &mut targets,
                        &mut discard,
                    );
                    scan_targets.extend(targets);
                }
                RawRefKind::Attribute => {
                    resolver.resolve_type_path(
                        &raw_ref.path,
                        origin,
                        &context,
                        &mut edge_lists[origin.index()],
                        &mut name_lists[origin.index()],
                    );
                    // [Authorize] → class AuthorizeAttribute.
                    if let Some(&last) = raw_ref.path.last() {
                        let with_suffix =
                            rodeo.get_or_intern(format!("{}Attribute", rodeo.resolve(&last)));
                        let mut suffixed: SmallVec<[Spur; 2]> = SmallVec::from_slice(&raw_ref.path);
                        *suffixed.last_mut().expect("non-empty path") = with_suffix;
                        resolver.resolve_type_path(
                            &suffixed,
                            origin,
                            &context,
                            &mut edge_lists[origin.index()],
                            &mut name_lists[origin.index()],
                        );
                    }
                }
            }
        }
    }

    // Root every concrete type whose base closure reaches a scan target:
    // reflection registration (`GetExports<IImageProvider>()`) instantiates
    // implementations that are never named in source.
    if !scan_targets.is_empty() {
        let mut scan_roots: Vec<SymbolId> = Vec::new();
        for symbol in &resolution.symbols {
            if !symbol.kind.is_type()
                || matches!(
                    symbol.kind,
                    crate::model::SymbolKind::Type(crate::model::TypeKind::Interface)
                )
                || symbol.modifiers.contains(crate::model::Modifiers::ABSTRACT)
                || symbol.base_names.is_empty()
            {
                continue;
            }
            if base_closure_hits(resolution, symbol, &scan_targets) {
                scan_roots.push(symbol.id);
            }
        }
        for id in scan_roots {
            resolution.symbols[id.index()].flags |= SymbolFlags::ROOT;
        }
    }

    // Containment-down lists from LIVE_WITH_TYPE flags.
    let mut live_children: Vec<SmallVec<[SymbolId; 2]>> = vec![SmallVec::new(); symbol_count];
    for symbol in &resolution.symbols {
        if symbol.flags.contains(SymbolFlags::LIVE_WITH_TYPE)
            && let Some(parent) = symbol.parent
        {
            live_children[parent.index()].push(symbol.id);
        }
    }

    // Flatten to CSR, deduping per symbol.
    let mut edges = Vec::new();
    let mut edge_ranges = Vec::with_capacity(symbol_count);
    let mut member_names = Vec::new();
    let mut member_name_ranges = Vec::with_capacity(symbol_count);
    for index in 0..symbol_count {
        let list = &mut edge_lists[index];
        list.sort_unstable();
        list.dedup();
        let start = edges.len() as u32;
        edges.extend_from_slice(list);
        edge_ranges.push((start, edges.len() as u32));

        let names = &mut name_lists[index];
        names.sort_unstable();
        names.dedup();
        let start = member_names.len() as u32;
        member_names.extend_from_slice(names);
        member_name_ranges.push((start, member_names.len() as u32));
    }

    SymbolGraph {
        edges,
        edge_ranges,
        member_names,
        member_name_ranges,
        live_children,
    }
}

/// Walk a type's transitive base closure (simple-name over-resolution, same
/// safe direction as the interface-satisfaction rules) looking for a scan
/// target.
fn base_closure_hits(
    resolution: &Resolution,
    symbol: &crate::resolve::Symbol,
    scan_targets: &rustc_hash::FxHashSet<SymbolId>,
) -> bool {
    let mut visited: rustc_hash::FxHashSet<SymbolId> = rustc_hash::FxHashSet::default();
    let mut stack: Vec<SymbolId> = Vec::new();

    let expand = |base_names: &[crate::extract::NamePath], stack: &mut Vec<SymbolId>| {
        for base in base_names {
            if let Some(&last) = base.last()
                && let Some(ids) = resolution.types_by_simple.get(&last)
            {
                stack.extend(ids.iter().copied());
            }
        }
    };

    expand(&symbol.base_names, &mut stack);
    while let Some(base_id) = stack.pop() {
        if !visited.insert(base_id) {
            continue;
        }
        if scan_targets.contains(&base_id) {
            return true;
        }
        expand(&resolution.symbols[base_id.index()].base_names, &mut stack);
    }
    false
}

struct Resolver<'a> {
    resolution: &'a Resolution,
    rodeo: &'a ThreadedRodeo,
}

impl Resolver<'_> {
    /// Scoped type resolution with the safety valves:
    /// - candidates from enclosing types → namespace ancestors → usings →
    ///   aliases, marking ALL hits (union, not first-match);
    /// - valve 1: zero candidates → every type with the same simple name;
    /// - valve 3: parse-error files additionally get simple-name matches,
    ///   and unresolved multi-segment paths degrade per-segment (types AND
    ///   members).
    fn resolve_type_path(
        &self,
        path: &[Spur],
        origin: SymbolId,
        context: &FileContext,
        edges: &mut Vec<SymbolId>,
        member_names: &mut Vec<Spur>,
    ) {
        if path.is_empty() {
            return;
        }
        let mut found = false;

        // Alias expansion: `using F = App.Widgets.Factory;` then `F.Create()`.
        let expanded: Option<Vec<Spur>> = context.aliases.get(&path[0]).map(|target| {
            let mut full = target.clone();
            full.extend_from_slice(&path[1..]);
            full
        });
        let path: &[Spur] = expanded.as_deref().unwrap_or(path);

        // 1. Enclosing type scope (nested types).
        let mut enclosing = self.enclosing_type_of(origin);
        while let Some(type_id) = enclosing {
            let symbol = &self.resolution.symbols[type_id.index()];
            if let Some(fqn) = symbol.fqn {
                found |= self.try_candidate(Some(fqn), path, edges);
            }
            enclosing = symbol.parent;
        }

        // 2. Namespace ancestors, including the bare path.
        let namespace = self.namespace_of(origin);
        for prefix_len in (0..=namespace.len()).rev() {
            found |= self.try_prefixed(&namespace[..prefix_len], path, edges);
        }

        // 3. Usings.
        for using in &context.usings {
            found |= self.try_prefixed(using, path, edges);
        }

        if !found || context.has_errors {
            // Valve: cannot distinguish "external type" from "incomplete
            // context", so fall back to simple-name matching. Multi-segment
            // paths degrade per segment, as both types and members
            // (covers Enum.Member, Constants.Value, Outer.Nested).
            for &segment in path {
                if let Some(ids) = self.resolution.types_by_simple.get(&segment) {
                    edges.extend_from_slice(ids);
                }
                if path.len() > 1 {
                    member_names.push(segment);
                }
            }
        }
    }

    fn try_prefixed(&self, prefix: &[Spur], path: &[Spur], edges: &mut Vec<SymbolId>) -> bool {
        let mut fqn = String::new();
        for segment in prefix.iter().chain(path.iter()) {
            if !fqn.is_empty() {
                fqn.push('.');
            }
            fqn.push_str(self.rodeo.resolve(segment));
        }
        self.lookup_fqn(&fqn, edges)
    }

    fn try_candidate(
        &self,
        prefix_fqn: Option<Spur>,
        path: &[Spur],
        edges: &mut Vec<SymbolId>,
    ) -> bool {
        let mut fqn = prefix_fqn
            .map(|f| self.rodeo.resolve(&f).to_string())
            .unwrap_or_default();
        for segment in path {
            if !fqn.is_empty() {
                fqn.push('.');
            }
            fqn.push_str(self.rodeo.resolve(segment));
        }
        self.lookup_fqn(&fqn, edges)
    }

    fn lookup_fqn(&self, fqn: &str, edges: &mut Vec<SymbolId>) -> bool {
        if let Some(spur) = self.rodeo.get(fqn)
            && let Some(ids) = self.resolution.types_by_fqn.get(&spur)
        {
            edges.extend_from_slice(ids);
            return true;
        }
        false
    }

    fn enclosing_type_of(&self, origin: SymbolId) -> Option<SymbolId> {
        let symbol = &self.resolution.symbols[origin.index()];
        if symbol.kind.is_type() {
            Some(origin)
        } else {
            symbol
                .parent
                .filter(|p| self.resolution.symbols[p.index()].kind.is_type())
        }
    }

    fn namespace_of(&self, origin: SymbolId) -> Vec<Spur> {
        let mut current = Some(origin);
        while let Some(id) = current {
            let symbol = &self.resolution.symbols[id.index()];
            if symbol.kind.is_type() {
                return symbol.namespace.to_vec();
            }
            current = symbol.parent;
        }
        Vec::new()
    }
}

/// Type-gated mark-and-sweep BFS.
///
/// A symbol is reached via: materialized edges, containment-up (member keeps
/// its type, nested keeps outer), containment-down (live-with-type members),
/// or member-name matching — gated so a member only lights up when its name
/// is referenced from reachable code AND its containing type is reachable.
pub fn mark_reachable(
    resolution: &Resolution,
    graph: &SymbolGraph,
    roots: impl Iterator<Item = SymbolId>,
) -> FixedBitSet {
    let symbol_count = resolution.symbols.len();
    let mut visited = FixedBitSet::with_capacity(symbol_count);
    let mut queue: VecDeque<SymbolId> = VecDeque::new();
    let mut live_names: FxHashMap<Spur, ()> = FxHashMap::default();
    // Members whose name is live but whose containing type isn't yet.
    let mut pending: FxHashMap<SymbolId, Vec<SymbolId>> = FxHashMap::default();

    let push = |id: SymbolId, visited: &mut FixedBitSet, queue: &mut VecDeque<SymbolId>| {
        if !visited.contains(id.index()) {
            visited.insert(id.index());
            queue.push_back(id);
        }
    };

    for root in roots {
        push(root, &mut visited, &mut queue);
    }

    while let Some(current) = queue.pop_front() {
        for &target in graph.edges_of(current) {
            push(target, &mut visited, &mut queue);
        }
        if let Some(parent) = resolution.symbols[current.index()].parent {
            push(parent, &mut visited, &mut queue);
        }
        for &child in &graph.live_children[current.index()] {
            push(child, &mut visited, &mut queue);
        }

        for &name in graph.member_names_of(current) {
            if live_names.insert(name, ()).is_none()
                && let Some(candidates) = resolution.members_by_name.get(&name)
            {
                for &member in candidates {
                    let symbol = &resolution.symbols[member.index()];
                    if symbol.flags.contains(SymbolFlags::NO_TYPE_GATE) {
                        push(member, &mut visited, &mut queue);
                        continue;
                    }
                    match symbol.parent {
                        Some(container) if !visited.contains(container.index()) => {
                            pending.entry(container).or_default().push(member);
                        }
                        _ => push(member, &mut visited, &mut queue),
                    }
                }
            }
        }

        // A type becoming reachable unlocks its parked members.
        if resolution.symbols[current.index()].kind.is_type()
            && let Some(parked) = pending.remove(&current)
        {
            for member in parked {
                push(member, &mut visited, &mut queue);
            }
        }
    }

    visited
}
