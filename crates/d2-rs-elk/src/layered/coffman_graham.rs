// Copyright (c) 2016, 2020 Kiel University and others.
// Copyright (c) 2026 The elk-go Authors.
// Rust translation and modifications: Copyright 2026 Nelson Chen.
// This program and the accompanying materials are made available under the
// terms of the Eclipse Public License 2.0 which is available at
// https://www.eclipse.org/legal/epl-2.0.
// SPDX-License-Identifier: EPL-2.0

//! Source CoffmanGrahamLayerer transitive reduction, priorities and packing.
use super::{
    LGraphId, LNodeId, Layer, LayerId, LayeredArena, LayeredNodeType, LayeringError,
    gwt_priority_queue as queue, source_layering_connected_components,
};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn assign(arena: &mut LayeredArena, graph: LGraphId) -> Result<(), LayeringError> {
    let ordered = arena.graphs[graph as usize].layerless_nodes.clone();
    if ordered.is_empty() {
        return Ok(());
    }
    let node_set = ordered.iter().copied().collect::<BTreeSet<_>>();
    let edge_ids = arena
        .edges
        .iter()
        .enumerate()
        .filter_map(|(id, e)| {
            (e.active && node_set.contains(&e.source) && node_set.contains(&e.target))
                .then_some(id as u32)
        })
        .collect::<Vec<_>>();
    let scope = &arena.graphs[graph as usize];
    // Source runComponents splits once, before LabelDummyInserter. Repeating
    // DFS here reorders appended label proxies and changes priority ties.
    let split = !scope.separated_component
        && !scope.includes_children
        && scope.parent_node.is_none()
        && scope.separate_components
        && !ordered
            .iter()
            .any(|n| arena.nodes[*n as usize].node_type == LayeredNodeType::ExternalPort);
    let groups = if split {
        let mut groups = source_layering_connected_components(arena, &ordered, &edge_ids);
        groups.sort_by_key(|group| {
            group
                .iter()
                .filter_map(|n| ordered.iter().position(|p| p == n))
                .min()
        });
        groups
    } else {
        vec![ordered]
    };
    let bound = scope.coffman_graham_layer_bound.unwrap_or(i32::MAX);
    let mut all_layers = Vec::<Vec<LNodeId>>::new();
    for nodes in groups {
        let rows = layer_component(arena, &nodes, bound)?;
        while all_layers.len() < rows.len() {
            all_layers.push(Vec::new());
        }
        for (rank, row) in rows.into_iter().enumerate() {
            all_layers[rank].extend(row);
        }
    }
    for row in all_layers {
        let id = arena.layers.len() as LayerId;
        for node in &row {
            arena.nodes[*node as usize].layer = Some(id);
        }
        arena.layers.push(Layer {
            graph,
            nodes: row,
            width: 0.0,
            height: 0.0,
        });
        arena.graphs[graph as usize].layers.push(id);
    }
    Ok(())
}

fn layer_component(
    arena: &LayeredArena,
    nodes: &[LNodeId],
    bound: i32,
) -> Result<Vec<Vec<LNodeId>>, LayeringError> {
    let index = nodes
        .iter()
        .copied()
        .enumerate()
        .map(|(i, n)| (n, i))
        .collect::<BTreeMap<_, _>>();
    let outgoing = nodes
        .iter()
        .map(|n| {
            arena
                .node_outgoing_edges(*n)
                .into_iter()
                .filter(|e| index.contains_key(&arena.edges[*e as usize].target))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let incoming = nodes
        .iter()
        .map(|n| {
            arena
                .node_incoming_edges(*n)
                .into_iter()
                .filter(|e| index.contains_key(&arena.edges[*e as usize].source))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let mut removed = BTreeSet::new();
    fn dfs(
        arena: &LayeredArena,
        index: &BTreeMap<LNodeId, usize>,
        outgoing: &[Vec<u32>],
        incoming: &[Vec<u32>],
        start: LNodeId,
        node: LNodeId,
        marks: &mut [bool],
        removed: &mut BTreeSet<u32>,
    ) {
        let i = index[&node];
        if marks[i] {
            return;
        }
        for edge in &outgoing[i] {
            let target = arena.edges[*edge as usize].target;
            for candidate in &incoming[index[&target]] {
                if arena.edges[*candidate as usize].source == start {
                    removed.insert(*candidate);
                }
            }
            dfs(
                arena, index, outgoing, incoming, start, target, marks, removed,
            );
        }
        marks[i] = true;
    }
    for start in nodes {
        let mut marks = vec![false; nodes.len()];
        for edge in &outgoing[index[start]] {
            dfs(
                arena,
                &index,
                &outgoing,
                &incoming,
                *start,
                arena.edges[*edge as usize].target,
                &mut marks,
                &mut removed,
            );
        }
    }
    let mut indegree = incoming
        .iter()
        .map(|es| es.iter().filter(|e| !removed.contains(e)).count())
        .collect::<Vec<_>>();
    let mut history = vec![Vec::<usize>::new(); nodes.len()];
    let mut sources = Vec::new();
    for i in 0..nodes.len() {
        if indegree[i] == 0 {
            queue::offer(&mut sources, i, |a, b| compare_history(a, b, &history));
        }
    }
    let mut order = vec![0; nodes.len()];
    let mut position = 0;
    while let Some(i) = queue::poll(&mut sources, |a, b| compare_history(a, b, &history)) {
        order[i] = position;
        position += 1;
        for edge in &outgoing[i] {
            if removed.contains(edge) {
                continue;
            }
            let target = index[&arena.edges[*edge as usize].target];
            indegree[target] -= 1;
            history[target].push(order[i]);
            if indegree[target] == 0 {
                queue::offer(&mut sources, target, |a, b| compare_history(a, b, &history));
            }
        }
    }
    if position != nodes.len() {
        return Err(LayeringError::Cycle(
            nodes[indegree.iter().position(|d| *d > 0).unwrap_or(0)],
        ));
    }
    let mut outdegree = outgoing
        .iter()
        .map(|es| es.iter().filter(|e| !removed.contains(e)).count())
        .collect::<Vec<_>>();
    let mut sinks = Vec::new();
    for i in 0..nodes.len() {
        if outdegree[i] == 0 {
            queue::offer(&mut sinks, i, |a, b| order[b].cmp(&order[a]));
        }
    }
    let mut rows = vec![Vec::new()];
    let mut assigned = vec![None; nodes.len()];
    while let Some(i) = queue::poll(&mut sinks, |a, b| order[b].cmp(&order[a])) {
        let mut rank = rows.len() - 1;
        let can_add = outgoing[i]
            .iter()
            .all(|e| assigned[index[&arena.edges[*e as usize].target]] != Some(rank));
        if rows[rank].len() as i64 >= i64::from(bound) || !can_add {
            rows.push(Vec::new());
            rank += 1;
        }
        rows[rank].push(nodes[i]);
        assigned[i] = Some(rank);
        for edge in &incoming[i] {
            if removed.contains(edge) {
                continue;
            }
            let source = index[&arena.edges[*edge as usize].source];
            outdegree[source] -= 1;
            if outdegree[source] == 0 {
                queue::offer(&mut sinks, source, |a, b| order[b].cmp(&order[a]));
            }
        }
    }
    rows.reverse();
    Ok(rows)
}

fn compare_history(first: usize, second: usize, history: &[Vec<usize>]) -> Ordering {
    let a = &history[first];
    let b = &history[second];
    let mut ai = a.len();
    let mut bi = b.len();
    while ai > 0 && bi > 0 {
        ai -= 1;
        bi -= 1;
        let cmp = a[ai].cmp(&b[bi]);
        if cmp != Ordering::Equal {
            return cmp;
        }
    }
    let ah = ai < a.len();
    let bh = bi < b.len();
    if !ah && !bh {
        first.cmp(&second)
    } else if ah {
        Ordering::Greater
    } else {
        Ordering::Less
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_comparator_preserves_source_list_iterator_has_next() {
        // ELK checks hasNext after traversing backward. Equal nonempty
        // histories therefore return Greater in either direction; replacing
        // this with lexicographic ordering changes the source heap behavior.
        let history = vec![vec![], vec![], vec![2], vec![2], vec![3]];
        assert_eq!(compare_history(0, 1, &history), Ordering::Less);
        assert_eq!(compare_history(2, 3, &history), Ordering::Greater);
        assert_eq!(compare_history(3, 2, &history), Ordering::Greater);
        assert_eq!(compare_history(2, 4, &history), Ordering::Less);
        assert_eq!(compare_history(0, 2, &history), Ordering::Less);
    }
}
