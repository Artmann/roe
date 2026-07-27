use std::collections::VecDeque;

use fixedbitset::FixedBitSet;
use lasso::Spur;
use rayon::prelude::*;
use rustc_hash::FxHashMap;
use smallvec::SmallVec;

use crate::extract::{FILE_ROOT, FileFacts, Interner, RawRefKind};
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
///
/// Usings are kept as pre-rendered dotted strings rather than `Spur` paths.
/// Every reference in the file probes every using, so rendering them once per
/// file — instead of re-resolving their segments through the interner for
/// every reference — is where most of this phase's time used to go. Project
/// globals are rendered once per project and borrowed, never cloned per file.
struct FileContext<'a> {
    local_usings: Vec<String>,
    global_usings: &'a [String],
    aliases: FxHashMap<Spur, Vec<Spur>>,
    has_errors: bool,
}

impl FileContext<'_> {
    fn using_prefixes(&self) -> impl Iterator<Item = &str> {
        self.local_usings
            .iter()
            .map(String::as_str)
            .chain(self.global_usings.iter().map(String::as_str))
    }
}

/// Reusable buffers for candidate-FQN construction, owned by the per-file
/// resolution loop and cleared between uses. Resolving one reference probes
/// (enclosing types + namespace ancestors + usings) candidates, and building
/// each one as a fresh `String` dominated the whole analysis.
#[derive(Default)]
struct Scratch {
    /// The candidate FQN handed to the interner lookup.
    fqn: String,
    /// The reference's own dotted path — identical across every candidate for
    /// a given reference, so rendered once.
    path: String,
    /// The origin's namespace, dotted, with `namespace_ends[k]` holding the
    /// byte length of its first `k` segments so every ancestor prefix is a
    /// subslice rather than a rebuild.
    namespace: String,
    namespace_ends: Vec<usize>,
}

/// Renders a `Spur` path as a dotted string into `buffer`, replacing its
/// contents. A segment that interns to the empty string contributes no
/// separator, matching how these names have always been joined.
fn render_path_into(path: &[Spur], rodeo: &Interner, buffer: &mut String) {
    buffer.clear();
    for segment in path {
        if !buffer.is_empty() {
            buffer.push('.');
        }
        buffer.push_str(rodeo.resolve(segment));
    }
}

fn render_path(path: &[Spur], rodeo: &Interner) -> String {
    let mut buffer = String::new();
    render_path_into(path, rodeo, &mut buffer);

    buffer
}

fn render_paths(paths: &[Vec<Spur>], rodeo: &Interner) -> Vec<String> {
    paths.iter().map(|path| render_path(path, rodeo)).collect()
}

/// One file's resolved references, indexed by that file's own declaration
/// slots rather than by `SymbolId`.
///
/// Slot-indexing is what lets files resolve concurrently: two files can share
/// a `SymbolId` (partial types) and two slots within a file can too (method
/// overloads merge), so writing straight to a shared per-symbol array would
/// need coordination. The slot → symbol mapping is applied when merging.
/// The last slot, `local_map.len()`, stands for the file root.
struct FileOutput {
    edges: Vec<Vec<SymbolId>>,
    names: Vec<Vec<Spur>>,
    scan_targets: Vec<SymbolId>,
}

/// Resolve every reference in one file. Reads the symbol table and interner
/// but mutates neither, apart from interning `…Attribute` probes — which
/// `ThreadedRodeo` handles concurrently, and which extraction already does
/// from multiple threads.
fn resolve_file(
    resolver: &Resolver,
    file_facts: &FileFacts,
    workspace: &Workspace,
    resolution: &Resolution,
    project_global_strings: &FxHashMap<ProjectId, Vec<String>>,
    all_global_strings: &[String],
    scratch: &mut Scratch,
) -> FileOutput {
    let rodeo = resolver.rodeo;
    let file = &workspace.files[file_facts.file.index()];
    // Files with a project see that project's globals; orphan files see
    // every global (over-inclusion is safe).
    let global_usings = match file.project.and_then(|p| project_global_strings.get(&p)) {
        Some(globals) => globals.as_slice(),
        None => all_global_strings,
    };
    let mut context = FileContext {
        local_usings: Vec::new(),
        global_usings,
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
            None => context.local_usings.push(render_path(&using.path, rodeo)),
        }
    }

    let local_map = &resolution.decl_map[file_facts.file.index()];
    let file_root = resolution.file_roots[file_facts.file.index()];
    let slot_count = local_map.len() + 1;
    let mut output = FileOutput {
        edges: vec![Vec::new(); slot_count],
        names: vec![Vec::new(); slot_count],
        scan_targets: Vec::new(),
    };

    for raw_ref in &file_facts.refs {
        let slot = if raw_ref.origin == FILE_ROOT {
            local_map.len()
        } else {
            raw_ref.origin as usize
        };
        // The origin symbol is still needed: resolution is scoped by where the
        // reference sits, not by which slot collects it.
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
                    scratch,
                    &mut output.edges[slot],
                    &mut output.names[slot],
                );
            }
            RawRefKind::Member => {
                if let Some(&last) = raw_ref.path.last() {
                    output.names[slot].push(last);
                }
            }
            RawRefKind::Ambient => {
                resolver.resolve_type_path(
                    &raw_ref.path,
                    origin,
                    &context,
                    scratch,
                    &mut output.edges[slot],
                    &mut output.names[slot],
                );
                if raw_ref.path.len() == 1 {
                    output.names[slot].push(raw_ref.path[0]);
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
                    scratch,
                    &mut targets,
                    &mut discard,
                );
                output.scan_targets.extend(targets);
            }
            RawRefKind::Attribute => {
                resolver.resolve_type_path(
                    &raw_ref.path,
                    origin,
                    &context,
                    scratch,
                    &mut output.edges[slot],
                    &mut output.names[slot],
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
                        scratch,
                        &mut output.edges[slot],
                        &mut output.names[slot],
                    );
                }
            }
        }
    }

    output
}

/// Build reference edges by resolving every raw reference against the symbol
/// table, then flatten into CSR form. Also roots implementations of
/// scan-target types (reflection-based registration).
pub fn build_graph(
    resolution: &mut Resolution,
    workspace: &Workspace,
    facts: &[FileFacts],
    rodeo: &Interner,
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

    // Render the globals once here rather than per file: every file in a
    // project sees the same list, and orphan files all see `all_globals`.
    let project_global_strings: FxHashMap<ProjectId, Vec<String>> = project_globals
        .iter()
        .map(|(&project, globals)| (project, render_paths(globals, rodeo)))
        .collect();
    let all_global_strings = render_paths(&all_globals, rodeo);

    let resolver = Resolver {
        resolution: &*resolution,
        rodeo,
    };

    // Resolve each file independently, then merge. Files write only into their
    // own slot-indexed buffers, so nothing is shared but the read-only symbol
    // table and the interner (which is built for concurrent access).
    let per_file: Vec<FileOutput> = facts
        .par_iter()
        .map_init(Scratch::default, |scratch, file_facts| {
            resolve_file(
                &resolver,
                file_facts,
                workspace,
                resolution,
                &project_global_strings,
                &all_global_strings,
                scratch,
            )
        })
        .collect();

    // Merge slot-indexed output back onto symbols. Both lists are sorted and
    // deduped in the CSR flatten below, so merge order cannot affect the graph.
    for (file_facts, output) in facts.iter().zip(per_file) {
        let local_map = &resolution.decl_map[file_facts.file.index()];
        let file_root = resolution.file_roots[file_facts.file.index()];
        let origin_of = |slot: usize| {
            if slot == local_map.len() {
                file_root
            } else {
                local_map[slot]
            }
        };

        for (slot, list) in output.edges.into_iter().enumerate() {
            if !list.is_empty() {
                edge_lists[origin_of(slot).index()].extend(list);
            }
        }
        for (slot, list) in output.names.into_iter().enumerate() {
            if !list.is_empty() {
                name_lists[origin_of(slot).index()].extend(list);
            }
        }
        scan_targets.extend(output.scan_targets);
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
    rodeo: &'a Interner,
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
        scratch: &mut Scratch,
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

        // Destructured so the prefix source and the candidate buffer can be
        // borrowed at the same time.
        let Scratch {
            fqn,
            path: path_text,
            namespace: namespace_text,
            namespace_ends,
        } = scratch;

        // The path half of every candidate below is the same string, so it is
        // rendered once here instead of once per candidate.
        render_path_into(path, self.rodeo, path_text);

        // 1. Enclosing type scope (nested types).
        let mut enclosing = self.enclosing_type_of(origin);
        while let Some(type_id) = enclosing {
            let symbol = &self.resolution.symbols[type_id.index()];
            if let Some(prefix) = symbol.fqn {
                found |= self.lookup_joined(self.rodeo.resolve(&prefix), path_text, fqn, edges);
            }
            enclosing = symbol.parent;
        }

        // 2. Namespace ancestors, including the bare path. Rendered once with
        // per-segment end offsets so each ancestor prefix is a subslice.
        let namespace = self.namespace_of(origin);
        namespace_text.clear();
        namespace_ends.clear();
        namespace_ends.push(0);
        for segment in namespace {
            if !namespace_text.is_empty() {
                namespace_text.push('.');
            }
            namespace_text.push_str(self.rodeo.resolve(segment));
            namespace_ends.push(namespace_text.len());
        }
        for prefix_len in (0..=namespace.len()).rev() {
            let prefix = &namespace_text[..namespace_ends[prefix_len]];
            found |= self.lookup_joined(prefix, path_text, fqn, edges);
        }

        // 3. Usings.
        for using in context.using_prefixes() {
            found |= self.lookup_joined(using, path_text, fqn, edges);
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

    /// Joins an already-rendered prefix and path into `buffer` and looks the
    /// result up. `buffer` is reused across every candidate — building it
    /// fresh each time was the single hottest allocation in the analysis.
    fn lookup_joined(
        &self,
        prefix: &str,
        path: &str,
        buffer: &mut String,
        edges: &mut Vec<SymbolId>,
    ) -> bool {
        buffer.clear();
        if !prefix.is_empty() {
            buffer.push_str(prefix);
            if !path.is_empty() {
                buffer.push('.');
            }
        }
        buffer.push_str(path);

        self.lookup_fqn(buffer, edges)
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

    fn namespace_of(&self, origin: SymbolId) -> &[Spur] {
        let mut current = Some(origin);
        while let Some(id) = current {
            let symbol = &self.resolution.symbols[id.index()];
            if symbol.kind.is_type() {
                return &symbol.namespace;
            }
            current = symbol.parent;
        }
        &[]
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
