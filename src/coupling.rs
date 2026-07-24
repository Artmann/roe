//! Type-level coupling analysis: circular dependencies (cycles in the type
//! reference graph), rolled up from the symbol-level reference graph built
//! for dead-code analysis.
//!
//! The fan-out map is the intermediate step — the type-to-type edge set that
//! cycle detection walks. It is deliberately not reported on its own: a high
//! outgoing dependency count is normal in constructor-injected C#, so the
//! count says little about whether a type is actually hard to work with.

use std::collections::VecDeque;
use std::collections::hash_map::Entry;

use rustc_hash::{FxHashMap, FxHashSet};

use crate::graph::SymbolGraph;
use crate::model::SymbolId;
use crate::resolve::Resolution;

/// The type "owning" a symbol: itself if it is a type, its containing type
/// if it is a member. `None` for anything else (file roots, or a member
/// somehow parented outside a type).
pub fn owning_type(resolution: &Resolution, id: SymbolId) -> Option<SymbolId> {
    let symbol = &resolution.symbols[id.index()];
    if symbol.kind.is_type() {
        Some(id)
    } else if symbol.kind.is_member() {
        symbol
            .parent
            .filter(|&parent| resolution.symbols[parent.index()].kind.is_type())
    } else {
        None
    }
}

/// Type-level fan-out: for every type, the distinct set of other types it
/// (or any of its members) references. Self-references are excluded, since
/// a type using its own members is not coupling.
pub fn fan_out(
    resolution: &Resolution,
    graph: &SymbolGraph,
) -> FxHashMap<SymbolId, FxHashSet<SymbolId>> {
    let mut result: FxHashMap<SymbolId, FxHashSet<SymbolId>> = FxHashMap::default();

    for symbol in &resolution.symbols {
        let Some(owner) = owning_type(resolution, symbol.id) else {
            continue;
        };
        for &target in graph.edges_of(symbol.id) {
            let Some(target_owner) = owning_type(resolution, target) else {
                continue;
            };
            if target_owner == owner {
                continue;
            }
            result.entry(owner).or_default().insert(target_owner);
        }
    }

    result
}

struct Frame {
    node: SymbolId,
    neighbors: Vec<SymbolId>,
    next: usize,
}

/// One circular dependency: a real cycle, plus whatever else is tangled with
/// it.
#[derive(Debug, PartialEq, Eq)]
pub struct Cycle {
    /// Consecutive entries are joined by an actual edge, and the last entry
    /// has an edge back to the first. Never empty, never shorter than 2.
    pub path: Vec<SymbolId>,
    /// Members of the same strongly-connected component that `path` does not
    /// visit, ascending. Sorted for determinism.
    pub others: Vec<SymbolId>,
}

/// Circular dependencies in the type-level fan-out graph.
///
/// Finding the components is Tarjan's algorithm, with an explicit work stack
/// instead of recursion so it can't blow the stack on a codebase with a long
/// dependency chain. Reporting them is the harder half: a strongly-connected
/// component is *not* generally a cycle you can walk through every member
/// exactly once, so listing an SCC's members joined by arrows would claim
/// edges that need not exist. Each component is therefore reduced to the
/// shortest genuine cycle through its lowest-id member, with the members that
/// cycle misses carried alongside in [`Cycle::others`].
pub fn find_cycles(fan_out: &FxHashMap<SymbolId, FxHashSet<SymbolId>>) -> Vec<Cycle> {
    components(fan_out)
        .into_iter()
        .map(|component| shortest_cycle(fan_out, &component))
        .collect()
}

/// The shortest cycle through `component`'s lowest-id member, found by BFS
/// restricted to the component. Such a cycle always exists: every member of a
/// strongly-connected component of size > 1 lies on one.
fn shortest_cycle(
    fan_out: &FxHashMap<SymbolId, FxHashSet<SymbolId>>,
    component: &[SymbolId],
) -> Cycle {
    let members: FxHashSet<SymbolId> = component.iter().copied().collect();
    let anchor = *component
        .iter()
        .min()
        .expect("a strongly-connected component is never empty");

    let empty: FxHashSet<SymbolId> = FxHashSet::default();
    let mut previous: FxHashMap<SymbolId, SymbolId> = FxHashMap::default();
    let mut queue: VecDeque<SymbolId> = VecDeque::from([anchor]);
    let mut path = Vec::new();

    'search: while let Some(node) = queue.pop_front() {
        // Ties inside a BFS layer are broken by id so the reported cycle is
        // the same on every run and every platform, regardless of how the
        // neighbour set happens to iterate.
        let mut neighbors: Vec<SymbolId> = fan_out
            .get(&node)
            .unwrap_or(&empty)
            .iter()
            .copied()
            .filter(|neighbor| members.contains(neighbor))
            .collect();
        neighbors.sort();

        for neighbor in neighbors {
            if neighbor == anchor {
                path = walk_back(&previous, anchor, node);
                break 'search;
            }
            if previous.contains_key(&neighbor) {
                continue;
            }
            previous.insert(neighbor, node);
            queue.push_back(neighbor);
        }
    }

    debug_assert!(
        path.len() >= 2,
        "every member of a multi-node SCC lies on a cycle"
    );

    let on_path: FxHashSet<SymbolId> = path.iter().copied().collect();
    let mut others: Vec<SymbolId> = component
        .iter()
        .copied()
        .filter(|id| !on_path.contains(id))
        .collect();
    others.sort();

    Cycle { path, others }
}

/// Reconstruct `anchor → … → last` by following the BFS parent chain back.
fn walk_back(
    previous: &FxHashMap<SymbolId, SymbolId>,
    anchor: SymbolId,
    last: SymbolId,
) -> Vec<SymbolId> {
    let mut path = vec![last];
    let mut current = last;

    while current != anchor {
        current = previous[&current];
        path.push(current);
    }

    path.reverse();

    path
}

/// Strongly-connected components of size > 1 — each a set of types whose
/// dependencies form a cycle.
fn components(fan_out: &FxHashMap<SymbolId, FxHashSet<SymbolId>>) -> Vec<Vec<SymbolId>> {
    let empty: FxHashSet<SymbolId> = FxHashSet::default();
    let neighbors_of = |id: SymbolId| -> Vec<SymbolId> {
        fan_out.get(&id).unwrap_or(&empty).iter().copied().collect()
    };

    let mut node_set: FxHashSet<SymbolId> = FxHashSet::default();
    for (&from, targets) in fan_out {
        node_set.insert(from);
        node_set.extend(targets.iter().copied());
    }

    // Tarjan finds the same components whatever order it starts in, but the
    // order it *emits* them follows the traversal. Seeding from a sorted list
    // keeps the report stable across runs and platforms.
    let mut nodes: Vec<SymbolId> = node_set.into_iter().collect();
    nodes.sort();

    let mut next_index = 0u32;
    let mut indices: FxHashMap<SymbolId, u32> = FxHashMap::default();
    let mut lowlink: FxHashMap<SymbolId, u32> = FxHashMap::default();
    let mut on_stack: FxHashSet<SymbolId> = FxHashSet::default();
    let mut tarjan_stack: Vec<SymbolId> = Vec::new();
    let mut components: Vec<Vec<SymbolId>> = Vec::new();

    for start in nodes {
        if indices.contains_key(&start) {
            continue;
        }

        let mut work: Vec<Frame> = vec![Frame {
            node: start,
            neighbors: neighbors_of(start),
            next: 0,
        }];
        indices.insert(start, next_index);
        lowlink.insert(start, next_index);
        next_index += 1;
        tarjan_stack.push(start);
        on_stack.insert(start);

        while let Some(top) = work.len().checked_sub(1) {
            let next_neighbor = {
                let frame = &mut work[top];
                if frame.next < frame.neighbors.len() {
                    let neighbor = frame.neighbors[frame.next];
                    frame.next += 1;
                    Some(neighbor)
                } else {
                    None
                }
            };

            match next_neighbor {
                Some(neighbor) => {
                    let node = work[top].node;
                    match indices.entry(neighbor) {
                        Entry::Vacant(entry) => {
                            entry.insert(next_index);
                            lowlink.insert(neighbor, next_index);
                            next_index += 1;
                            tarjan_stack.push(neighbor);
                            on_stack.insert(neighbor);
                            work.push(Frame {
                                node: neighbor,
                                neighbors: neighbors_of(neighbor),
                                next: 0,
                            });
                        }
                        Entry::Occupied(entry) => {
                            if on_stack.contains(&neighbor) {
                                let neighbor_index = *entry.get();
                                let node_lowlink = lowlink[&node];
                                lowlink.insert(node, node_lowlink.min(neighbor_index));
                            }
                        }
                    }
                }
                None => {
                    let node = work[top].node;
                    let node_lowlink = lowlink[&node];
                    work.pop();

                    if let Some(parent_frame) = work.last() {
                        let parent = parent_frame.node;
                        let parent_lowlink = lowlink[&parent];
                        lowlink.insert(parent, parent_lowlink.min(node_lowlink));
                    }

                    if node_lowlink == indices[&node] {
                        let mut component = Vec::new();
                        loop {
                            let member = tarjan_stack.pop().expect("node is on the Tarjan stack");
                            on_stack.remove(&member);
                            component.push(member);
                            if member == node {
                                break;
                            }
                        }
                        if component.len() > 1 {
                            component.sort();
                            components.push(component);
                        }
                    }
                }
            }
        }
    }

    components.sort_by(|a, b| a[0].cmp(&b[0]));

    components
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u32) -> SymbolId {
        SymbolId(n)
    }

    fn graph(edges: &[(u32, &[u32])]) -> FxHashMap<SymbolId, FxHashSet<SymbolId>> {
        edges
            .iter()
            .map(|&(from, targets)| (id(from), targets.iter().map(|&t| id(t)).collect()))
            .collect()
    }

    /// Every consecutive pair in the reported path — including the wrap from
    /// the last member back to the first — must be a real edge. This is the
    /// property the old SCC-in-pop-order output silently violated.
    fn assert_path_is_a_real_cycle(
        cycle: &Cycle,
        fan_out: &FxHashMap<SymbolId, FxHashSet<SymbolId>>,
    ) {
        assert!(cycle.path.len() >= 2, "a cycle needs at least two members");

        for (index, &from) in cycle.path.iter().enumerate() {
            let to = cycle.path[(index + 1) % cycle.path.len()];
            assert!(
                fan_out
                    .get(&from)
                    .is_some_and(|targets| targets.contains(&to)),
                "{from:?} -> {to:?} is not a real edge"
            );
        }
    }

    #[test]
    fn two_cycle_is_detected() {
        // A -> B -> A
        let fan_out = graph(&[(1, &[2]), (2, &[1])]);
        let cycles = find_cycles(&fan_out);
        assert_eq!(cycles.len(), 1);
        assert_eq!(cycles[0].path, vec![id(1), id(2)]);
        assert!(cycles[0].others.is_empty());
        assert_path_is_a_real_cycle(&cycles[0], &fan_out);
    }

    #[test]
    fn linear_chain_has_no_cycle() {
        // A -> B -> C
        let fan_out = graph(&[(1, &[2]), (2, &[3])]);
        assert!(find_cycles(&fan_out).is_empty());
    }

    #[test]
    fn self_loop_alone_is_not_reported() {
        // A -> A only — not coupling between distinct types.
        let fan_out = graph(&[(1, &[1])]);
        assert!(find_cycles(&fan_out).is_empty());
    }

    #[test]
    fn three_cycle_is_detected() {
        // A -> B -> C -> A
        let fan_out = graph(&[(1, &[2]), (2, &[3]), (3, &[1])]);
        let cycles = find_cycles(&fan_out);
        assert_eq!(cycles.len(), 1);
        assert_eq!(cycles[0].path, vec![id(1), id(2), id(3)]);
        assert!(cycles[0].others.is_empty());
        assert_path_is_a_real_cycle(&cycles[0], &fan_out);
    }

    #[test]
    fn disjoint_cycles_are_both_reported() {
        // A <-> B, and separately C <-> D.
        let fan_out = graph(&[(1, &[2]), (2, &[1]), (3, &[4]), (4, &[3])]);
        let cycles = find_cycles(&fan_out);
        assert_eq!(cycles.len(), 2);
        // Ordered by lowest member, with no sorting by the caller.
        assert_eq!(cycles[0].path, vec![id(1), id(2)]);
        assert_eq!(cycles[1].path, vec![id(3), id(4)]);
    }

    #[test]
    fn a_component_with_no_hamiltonian_cycle_reports_a_real_path() {
        // 1 <-> 2 and 2 <-> 3: one strongly-connected component of three
        // types, but no cycle visits all three — 1 and 3 never touch. The old
        // output printed "1 → 2 → 3 → 1", claiming two edges that don't exist.
        let fan_out = graph(&[(1, &[2]), (2, &[1, 3]), (3, &[2])]);
        let cycles = find_cycles(&fan_out);

        assert_eq!(cycles.len(), 1);
        assert_eq!(cycles[0].path, vec![id(1), id(2)]);
        assert_eq!(cycles[0].others, vec![id(3)]);
        assert_path_is_a_real_cycle(&cycles[0], &fan_out);
    }

    #[test]
    fn the_shortest_cycle_through_the_anchor_wins() {
        // 1 -> 2 -> 1 is available alongside the longer 1 -> 3 -> 4 -> 1.
        let fan_out = graph(&[(1, &[2, 3]), (2, &[1]), (3, &[4]), (4, &[1])]);
        let cycles = find_cycles(&fan_out);

        assert_eq!(cycles.len(), 1);
        assert_eq!(cycles[0].path, vec![id(1), id(2)]);
        assert_eq!(cycles[0].others, vec![id(3), id(4)]);
        assert_path_is_a_real_cycle(&cycles[0], &fan_out);
    }

    #[test]
    fn a_four_cycle_keeps_every_member_on_the_path() {
        // A -> B -> C -> D -> A, with nothing shorter available.
        let fan_out = graph(&[(1, &[2]), (2, &[3]), (3, &[4]), (4, &[1])]);
        let cycles = find_cycles(&fan_out);

        assert_eq!(cycles.len(), 1);
        assert_eq!(cycles[0].path, vec![id(1), id(2), id(3), id(4)]);
        assert!(cycles[0].others.is_empty());
        assert_path_is_a_real_cycle(&cycles[0], &fan_out);
    }
}
