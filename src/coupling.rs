//! Type-level coupling analysis: circular dependencies (cycles in the type
//! reference graph), rolled up from the symbol-level reference graph built
//! for dead-code analysis.
//!
//! The fan-out map is the intermediate step — the type-to-type edge set that
//! cycle detection walks. It is deliberately not reported on its own: a high
//! outgoing dependency count is normal in constructor-injected C#, so the
//! count says little about whether a type is actually hard to work with.

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

/// Strongly-connected components (size > 1) of the type-level fan-out
/// graph — each is a set of types whose dependencies form a cycle. Uses
/// Tarjan's algorithm with an explicit work stack instead of recursion, so
/// it can't blow the stack on a codebase with a long dependency chain.
pub fn find_cycles(fan_out: &FxHashMap<SymbolId, FxHashSet<SymbolId>>) -> Vec<Vec<SymbolId>> {
    let empty: FxHashSet<SymbolId> = FxHashSet::default();
    let neighbors_of = |id: SymbolId| -> Vec<SymbolId> {
        fan_out.get(&id).unwrap_or(&empty).iter().copied().collect()
    };

    let mut nodes: FxHashSet<SymbolId> = FxHashSet::default();
    for (&from, targets) in fan_out {
        nodes.insert(from);
        nodes.extend(targets.iter().copied());
    }

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
                            components.push(component);
                        }
                    }
                }
            }
        }
    }

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

    #[test]
    fn two_cycle_is_detected() {
        // A -> B -> A
        let fan_out = graph(&[(1, &[2]), (2, &[1])]);
        let cycles = find_cycles(&fan_out);
        assert_eq!(cycles.len(), 1);
        let mut members = cycles[0].clone();
        members.sort();
        assert_eq!(members, vec![id(1), id(2)]);
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
        let mut members = cycles[0].clone();
        members.sort();
        assert_eq!(members, vec![id(1), id(2), id(3)]);
    }

    #[test]
    fn disjoint_cycles_are_both_reported() {
        // A <-> B, and separately C <-> D.
        let fan_out = graph(&[(1, &[2]), (2, &[1]), (3, &[4]), (4, &[3])]);
        let mut cycles = find_cycles(&fan_out);
        cycles.sort_by_key(|c| *c.iter().min().unwrap());
        assert_eq!(cycles.len(), 2);
        let mut first = cycles[0].clone();
        first.sort();
        assert_eq!(first, vec![id(1), id(2)]);
        let mut second = cycles[1].clone();
        second.sort();
        assert_eq!(second, vec![id(3), id(4)]);
    }
}
