// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Derived from Dagro v0.2.1 / Dagre ordering, distributed under the MIT License.
// Copyright (c) 2012-2014 Chris Pettitt
// Copyright (c) 2026 Dagro contributors
// The applicable MIT permission notice is in the repository root's THIRD_PARTY_NOTICES.md.

//! Compound ordering translated from Dagro v0.2.1's order*.go at D2 v0.9.0.
//!
//! Layer graphs retain their child insertion order across sweeps. A compound
//! is sorted recursively and expanded only after sibling conflicts have been
//! resolved, so its border pair encloses every descendant on the layer.

use super::BkNeighbors;
#[cfg(feature = "diagnostic-traces")]
use super::diagnostics;
use d2_rs_core::{Graph, NodeId};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::num::NonZeroUsize;

#[derive(Clone, Debug)]
struct Entry {
    nodes: Vec<NodeId>,
    index: usize,
    barycenter: Option<f64>,
    weight: f64,
}

#[derive(Clone, Debug, Default)]
struct OrderResult {
    nodes: Vec<NodeId>,
    barycenter: Option<f64>,
    weight: f64,
    used_bias: bool,
}

#[derive(Default)]
struct LayerGraph {
    children: BTreeMap<Option<NodeId>, Vec<NodeId>>,
    parents: LayerParents,
    incoming: BTreeMap<NodeId, Vec<(NodeId, f64)>>,
    borders: BTreeMap<NodeId, (NodeId, NodeId)>,
}

// Layer topology is fixed during Dagro's ordering sweeps. Resolve Graph.Parent
// links to compact slots once so an ancestor walk does not look up every link
// again for every leaf and sweep. Synthetic NodeIds can be sparse.
#[derive(Default)]
struct LayerParents {
    indices: HashMap<NodeId, usize>,
    links: Vec<(NodeId, Option<NonZeroUsize>)>,
}

impl LayerParents {
    fn slot(&mut self, node: NodeId) -> usize {
        *self.indices.entry(node).or_insert_with(|| {
            let index = self.links.len();
            self.links.push((node, None));
            index
        })
    }

    fn insert(&mut self, node: NodeId, parent: Option<NodeId>) {
        let index = self.slot(node);
        // One-based links keep None in one word alongside the node ID.
        let parent = parent.map(|id| {
            NonZeroUsize::new(self.slot(id) + 1).expect("parent slot fits an allocated vector")
        });
        self.links[index].1 = parent;
    }

    fn parent(&self, node: NodeId) -> Option<usize> {
        self.indices
            .get(&node)
            .and_then(|&index| self.links[index].1)
            .map(|index| index.get() - 1)
    }
}

/// Apply Dagro's recursive compound sweeps to the initialized leaf layers.
/// `dummy_meta` contains ordinary `dummy="edge"` nodes only; Dagro's
/// `sortSubgraph` excludes `edge-label` nodes from reversed-edge pairing.
#[allow(clippy::too_many_arguments)]
pub(super) fn order_compounds(
    graph: &Graph,
    layers: &mut [Vec<NodeId>],
    ranks: &BTreeMap<NodeId, usize>,
    edges: &[(NodeId, NodeId)],
    all_nodes: &[NodeId],
    owners: &BTreeMap<NodeId, NodeId>,
    side: &BTreeMap<NodeId, bool>,
    dummy_meta: &BTreeMap<NodeId, (NodeId, NodeId, bool)>,
) {
    if layers.is_empty() {
        return;
    }
    #[cfg(feature = "diagnostic-traces")]
    let capture_noop_sweeps = std::env::var_os("DAGRE_ORDER_TRACE").is_some();
    #[cfg(not(feature = "diagnostic-traces"))]
    let capture_noop_sweeps = false;
    if layers.len() < 2 && !capture_noop_sweeps {
        return;
    }
    #[cfg(feature = "diagnostic-traces")]
    {
        diagnostics::compound_order_nodes(all_nodes, graph);
        diagnostics::compound_order_layers("init", -1, None, layers, all_nodes);
    }
    let compounds = owners.values().copied().collect::<BTreeSet<_>>();
    let parent_of = |id: NodeId| {
        if let Some(owner) = owners.get(&id) {
            return Some(*owner);
        }
        let mut parent = graph.node(id).and_then(|node| node.parent);
        while let Some(id) = parent {
            if compounds.contains(&id) {
                return Some(id);
            }
            parent = graph.node(id).and_then(|node| node.parent);
        }
        None
    };
    // Graph.Parent is immutable during ordering. Resolve it once for every
    // registered or ranked node, including compounds that span many layers.
    let mut parents = HashMap::with_capacity(all_nodes.len() + compounds.len());
    for &id in all_nodes.iter().chain(ranks.keys()).chain(compounds.iter()) {
        parents.entry(id).or_insert_with(|| parent_of(id));
    }
    let parent_of = |id: NodeId| parents.get(&id).copied().flatten();
    let mut intervals = BTreeMap::<NodeId, (usize, usize)>::new();
    for (&id, &rank) in ranks {
        if let Some(id) = parent_of(id) {
            intervals
                .entry(id)
                .and_modify(|range| {
                    range.0 = range.0.min(rank);
                    range.1 = range.1.max(rank);
                })
                .or_insert((rank, rank));
        }
    }
    // Dagro buildLayerNodeBuckets reads each compound's inclusive minRank /
    // maxRank. Fold descendant ranges into their parents from the leaves up,
    // rather than walking every ranked node's entire ancestor chain. Min/max
    // are associative, so each compound link only needs to be visited once.
    let mut pending = compounds
        .iter()
        .map(|&id| (id, 0usize))
        .collect::<HashMap<_, _>>();
    for &id in &compounds {
        if let Some(parent) = parent_of(id) {
            *pending.get_mut(&parent).expect("parent is a compound") += 1;
        }
    }
    let mut ready = compounds
        .iter()
        .copied()
        .filter(|id| pending[id] == 0)
        .collect::<Vec<_>>();
    while let Some(id) = ready.pop() {
        if let Some(parent) = parent_of(id) {
            if let Some((min, max)) = intervals.get(&id).copied() {
                intervals
                    .entry(parent)
                    .and_modify(|range| {
                        range.0 = range.0.min(min);
                        range.1 = range.1.max(max);
                    })
                    .or_insert((min, max));
            }
            let remaining = pending.get_mut(&parent).expect("parent is a compound");
            *remaining -= 1;
            if *remaining == 0 {
                ready.push(parent);
            }
        }
    }
    drop(pending);
    // D2's ObjectMapper assigns original objects decimal IDs in registration
    // order. Thus `all_nodes` (original IDs, then generated nodes in creation
    // order) already matches Graphlib's JavaScript property-key enumeration.
    // Authored external_id strings do not participate in this ordering.
    // Dagro order.go buildLayerNodeBuckets visits nodes once in registration
    // order and appends each to its rank and inclusive compound interval.
    let mut layer_nodes = vec![Vec::new(); layers.len()];
    for &id in all_nodes {
        let node_rank = ranks.get(&id).copied();
        if let Some(rank) = node_rank.and_then(|rank| layer_nodes.get_mut(rank)) {
            rank.push(id);
        }
        if let Some(&(min, max)) = intervals.get(&id) {
            for rank in min..=max.min(layers.len() - 1) {
                if Some(rank) != node_rank {
                    layer_nodes[rank].push(id);
                }
            }
        }
    }
    // buildLayerGraphNodes reads InEdges/OutEdges for each node. These
    // lookup-only indexes retain the full edge list's encounter order.
    let predecessors = BkNeighbors::predecessors(edges);
    let successors = BkNeighbors::successors(edges);
    let build = |rank: usize, down: bool| {
        let mut layer = LayerGraph::default();
        let incident = if down { &predecessors } else { &successors };
        for &id in &layer_nodes[rank] {
            let parent = parent_of(id);
            layer.parents.insert(id, parent);
            layer.children.entry(parent).or_default().push(id);
            for &neighbor in incident.get(id) {
                // buildLayerGraph collapses parallel edges in encounter
                // order, accumulating their weights on the first edge.
                let incoming = layer.incoming.entry(id).or_default();
                if let Some((_, weight)) = incoming.iter_mut().find(|(node, _)| *node == neighbor) {
                    *weight += 1.0;
                } else {
                    incoming.push((neighbor, 1.0));
                }
            }
        }
        for &id in &layer_nodes[rank] {
            if !compounds.contains(&id) {
                continue;
            }
            let children = layer
                .children
                .get(&Some(id))
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let left = children
                .iter()
                .copied()
                .find(|id| side.get(id) == Some(&true));
            let right = children
                .iter()
                .copied()
                .find(|id| side.get(id) == Some(&false));
            if let (Some(left), Some(right)) = (left, right) {
                layer.borders.insert(id, (left, right));
            }
        }
        layer
    };
    let down = (0..layers.len())
        .map(|rank| build(rank, true))
        .collect::<Vec<_>>();
    let up = (0..layers.len())
        .map(|rank| build(rank, false))
        .collect::<Vec<_>>();
    drop(parents);
    let mut positions = layers
        .iter()
        .flat_map(|layer| layer.iter().enumerate().map(|(index, id)| (*id, index)))
        .collect::<BTreeMap<_, _>>();
    let mut best = layers.to_vec();
    let mut best_crossings = usize::MAX;
    let mut last_best = 0;
    let mut iteration = 0;
    while last_best < 4 {
        let downward = iteration % 2 == 1;
        let switch_bias = iteration % 4 >= 2;
        let mut bias_right = true;
        let mut constraints = Vec::new();
        let ranks_to_visit = if downward {
            (1..layers.len()).collect::<Vec<_>>()
        } else {
            (0..layers.len() - 1).rev().collect::<Vec<_>>()
        };
        for rank in ranks_to_visit {
            let layer = if downward { &down[rank] } else { &up[rank] };
            let sorted = sort_subgraph(
                layer,
                None,
                &constraints,
                &positions,
                bias_right,
                dummy_meta,
            );
            if switch_bias && sorted.used_bias {
                bias_right = !bias_right;
            }
            for (index, id) in sorted.nodes.iter().enumerate() {
                positions.insert(*id, index);
            }
            add_subgraph_constraints(layer, &mut constraints, &sorted.nodes);
        }
        for layer in layers.iter_mut() {
            layer.sort_by_key(|id| positions.get(id).copied().unwrap_or(0));
        }
        let crossings = cross_count(layers, &successors);
        #[cfg(feature = "diagnostic-traces")]
        diagnostics::compound_order_layers(
            "sweep",
            iteration as isize,
            Some(crossings),
            layers,
            all_nodes,
        );
        if crossings < best_crossings {
            best_crossings = crossings;
            last_best = 0;
            best = layers.to_vec();
        } else if crossings == best_crossings {
            best = layers.to_vec();
        }
        iteration += 1;
        last_best += 1;
    }
    layers.clone_from_slice(&best);
    #[cfg(feature = "diagnostic-traces")]
    diagnostics::compound_order_layers("best", -1, Some(best_crossings), &best, all_nodes);
}

fn barycenter(layer: &LayerGraph, node: NodeId, positions: &BTreeMap<NodeId, usize>) -> Entry {
    let mut entry = Entry {
        nodes: vec![node],
        index: 0,
        barycenter: None,
        weight: 0.0,
    };
    if let Some(incoming) = layer.incoming.get(&node).filter(|edges| !edges.is_empty()) {
        let mut sum = 0.0;
        for &(source, weight) in incoming {
            sum += weight * positions.get(&source).copied().unwrap_or(0) as f64;
            entry.weight += weight;
        }
        entry.barycenter = Some(sum / entry.weight);
    }
    entry
}

fn sort_subgraph(
    layer: &LayerGraph,
    parent: Option<NodeId>,
    constraints: &[(NodeId, NodeId)],
    positions: &BTreeMap<NodeId, usize>,
    bias_right: bool,
    dummy_meta: &BTreeMap<NodeId, (NodeId, NodeId, bool)>,
) -> OrderResult {
    let borders = parent.and_then(|id| layer.borders.get(&id)).copied();
    let movable = layer
        .children
        .get(&parent)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let mut subgraphs = BTreeMap::new();
    let mut entries = Vec::new();
    for &node in movable {
        if borders.is_some_and(|(left, right)| node == left || node == right) {
            continue;
        }
        let mut entry = barycenter(layer, node, positions);
        entry.index = entries.len();
        if layer
            .children
            .get(&Some(node))
            .is_some_and(|children| !children.is_empty())
        {
            let result = sort_subgraph(
                layer,
                Some(node),
                constraints,
                positions,
                bias_right,
                dummy_meta,
            );
            if let Some(other) = result.barycenter {
                if let Some(current) = entry.barycenter {
                    entry.barycenter = Some(
                        (current * entry.weight + other * result.weight)
                            / (entry.weight + result.weight),
                    );
                    entry.weight += result.weight;
                } else {
                    entry.barycenter = Some(other);
                    entry.weight = result.weight;
                }
            }
            subgraphs.insert(node, result);
        }
        entries.push(entry);
    }
    let mut entries = resolve_conflicts(entries, constraints);
    for entry in &mut entries {
        entry.nodes = entry
            .nodes
            .iter()
            .flat_map(|node| {
                subgraphs
                    .get(node)
                    .map(|result| result.nodes.clone())
                    .unwrap_or_else(|| vec![*node])
            })
            .collect();
    }
    let mut pairs = Vec::new();
    let mut used_bias = false;
    let mut i = 0;
    while i < entries.len() {
        let mut j = i + 1;
        let mut removed_i = false;
        while j < entries.len() {
            let (Some(left), Some(right)) = (entries[i].barycenter, entries[j].barycenter) else {
                j += 1;
                continue;
            };
            if !truthy_number(left) || !truthy_number(right) || left != right {
                j += 1;
                continue;
            }
            let left_node = entries[i].nodes.first().copied();
            let right_node = entries[j].nodes.first().copied();
            let left_meta = left_node.and_then(|id| dummy_meta.get(&id));
            let right_meta = right_node.and_then(|id| dummy_meta.get(&id));
            if let (Some(&(ls, lt, lr)), Some(&(rs, rt, rr))) = (left_meta, right_meta)
                && ls == rs
                && lt == rt
                && lr != rr
            {
                if lr {
                    pairs.push((right_node.expect("dummy has an ID"), entries.remove(i)));
                    removed_i = true;
                    break;
                }
                pairs.push((left_node.expect("dummy has an ID"), entries.remove(j)));
            } else {
                used_bias = true;
                j += 1;
            }
        }
        if !removed_i {
            i += 1;
        }
    }
    let mut result = sort_entries(entries, pairs, bias_right);
    result.used_bias = used_bias;
    if let Some((left, right)) = borders {
        result.nodes.insert(0, left);
        result.nodes.push(right);
        if let Some(&(left_pred, _)) = layer.incoming.get(&left).and_then(|edges| edges.first()) {
            let right_pred = layer
                .incoming
                .get(&right)
                .and_then(|edges| edges.first())
                .map(|&(id, _)| id);
            let sum = result.barycenter.unwrap_or(0.0) * result.weight
                + positions.get(&left_pred).copied().unwrap_or(0) as f64
                + right_pred
                    .and_then(|id| positions.get(&id))
                    .copied()
                    .unwrap_or(0) as f64;
            result.barycenter = Some(sum / (result.weight + 2.0));
            result.weight += 2.0;
        }
    }
    result
}

fn truthy_number(value: f64) -> bool {
    value != 0.0 && !value.is_nan()
}

fn resolve_conflicts(entries: Vec<Entry>, constraints: &[(NodeId, NodeId)]) -> Vec<Entry> {
    #[derive(Clone)]
    struct Conflict {
        entry: Entry,
        indegree: usize,
        incoming: Vec<usize>,
        outgoing: Vec<usize>,
        merged: bool,
    }
    let indices = entries
        .iter()
        .enumerate()
        .map(|(index, entry)| (entry.nodes[0], index))
        .collect::<BTreeMap<_, _>>();
    let mut entries = entries
        .into_iter()
        .map(|entry| Conflict {
            entry,
            indegree: 0,
            incoming: Vec::new(),
            outgoing: Vec::new(),
            merged: false,
        })
        .collect::<Vec<_>>();
    for &(source, target) in constraints {
        if let (Some(&source), Some(&target)) = (indices.get(&source), indices.get(&target)) {
            entries[target].indegree += 1;
            entries[source].outgoing.push(target);
        }
    }
    let mut sources = entries
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| (entry.indegree == 0).then_some(index))
        .collect::<Vec<_>>();
    let mut visited = Vec::new();
    while let Some(index) = sources.pop() {
        visited.push(index);
        for source in entries[index].incoming.clone().into_iter().rev() {
            if entries[source].merged {
                continue;
            }
            let source_bary = entries[source].entry.barycenter;
            let target_bary = entries[index].entry.barycenter;
            if source_bary.is_none() || target_bary.is_none() || source_bary >= target_bary {
                let source_entry = entries[source].entry.clone();
                let target = &mut entries[index].entry;
                let mut sum = 0.0;
                let mut weight = 0.0;
                if truthy_number(target.weight) {
                    sum += target.barycenter.unwrap_or(f64::NAN) * target.weight;
                    weight += target.weight;
                }
                if truthy_number(source_entry.weight) {
                    sum += source_entry.barycenter.unwrap_or(f64::NAN) * source_entry.weight;
                    weight += source_entry.weight;
                }
                let mut nodes = source_entry.nodes;
                nodes.append(&mut target.nodes);
                target.nodes = nodes;
                target.barycenter = Some(sum / weight);
                target.weight = weight;
                target.index = target.index.min(source_entry.index);
                entries[source].merged = true;
            }
        }
        for target in entries[index].outgoing.clone() {
            entries[target].incoming.push(index);
            entries[target].indegree -= 1;
            if entries[target].indegree == 0 {
                sources.push(target);
            }
        }
    }
    visited
        .into_iter()
        .filter(|&index| !entries[index].merged)
        .map(|index| entries[index].entry.clone())
        .collect()
}

fn sort_entries(entries: Vec<Entry>, pairs: Vec<(NodeId, Entry)>, bias_right: bool) -> OrderResult {
    let (mut sortable, mut unsortable): (Vec<_>, Vec<_>) = entries
        .into_iter()
        .partition(|entry| entry.barycenter.is_some());
    unsortable.sort_by_key(|entry| std::cmp::Reverse(entry.index));
    sortable.sort_by(|left, right| {
        let left_bary = left.barycenter.unwrap_or(0.0);
        let right_bary = right.barycenter.unwrap_or(0.0);
        if left_bary < right_bary {
            std::cmp::Ordering::Less
        } else if left_bary > right_bary {
            std::cmp::Ordering::Greater
        } else if bias_right {
            right.index.cmp(&left.index)
        } else {
            left.index.cmp(&right.index)
        }
    });
    let mut inserted_after = BTreeMap::<NodeId, usize>::new();
    for (key, entry) in pairs {
        let after = sortable
            .iter()
            .position(|entry| entry.nodes.first() == Some(&key))
            .map_or(0, |index| index + 1);
        let count = inserted_after.entry(key).or_default();
        sortable.insert((after + *count).min(sortable.len()), entry);
        *count += 1;
    }
    let mut result = OrderResult::default();
    let mut expanded_index = 0;
    let consume = |nodes: &mut Vec<NodeId>, unsortable: &mut Vec<Entry>, index: &mut usize| {
        while unsortable.last().is_some_and(|entry| entry.index <= *index) {
            nodes.extend(unsortable.pop().expect("last exists").nodes);
            // Dagro advances once for an unsortable entry, irrespective of
            // the number of expanded descendants in that entry.
            *index += 1;
        }
    };
    consume(&mut result.nodes, &mut unsortable, &mut expanded_index);
    let mut sum = 0.0;
    for entry in sortable {
        expanded_index += entry.nodes.len();
        result.nodes.extend(entry.nodes);
        sum += entry.barycenter.unwrap_or(0.0) * entry.weight;
        result.weight += entry.weight;
        consume(&mut result.nodes, &mut unsortable, &mut expanded_index);
    }
    if truthy_number(result.weight) {
        result.barycenter = Some(sum / result.weight);
    }
    result
}

fn add_subgraph_constraints(
    layer: &LayerGraph,
    constraints: &mut Vec<(NodeId, NodeId)>,
    nodes: &[NodeId],
) {
    // `None` denotes the artificial root of buildLayerGraph. Source rootPrev
    // only ever observes that same artificial root, so it cannot produce a
    // constraint. previous[None] represents source prev[artificial_root],
    // retaining the meaningful constraints between top-level compounds.
    // One extra slot represents the artificial root's None parent.
    let root = layer.parents.links.len();
    let mut previous = vec![None; root + 1];
    for &node in nodes {
        let mut child = layer.parents.parent(node);
        while let Some(index) = child {
            let (id, parent) = layer.parents.links[index];
            let parent = parent.map(|index| index.get() - 1);
            if let Some(previous_child) = previous[parent.unwrap_or(root)].replace(id)
                && previous_child != id
            {
                if !constraints.contains(&(previous_child, id)) {
                    constraints.push((previous_child, id));
                }
                break;
            }
            child = parent;
        }
    }
}

pub(super) fn cross_count(layers: &[Vec<NodeId>], successors: &BkNeighbors) -> usize {
    let mut total = 0;
    for pair in layers.windows(2) {
        let south = pair[1]
            .iter()
            .enumerate()
            .map(|(index, &id)| (id, index))
            .collect::<BTreeMap<_, _>>();
        // Dagro twoLayerCrossCount walks north nodes, then their OutEdges,
        // sorting each node's south positions. This is the same tuple order
        // as sorting all (north, south) pairs, without scanning every edge
        // for every pair of layers. Parallel segments remain separate.
        let mut ranked = Vec::new();
        for &source in &pair[0] {
            let first = ranked.len();
            ranked.extend(
                successors
                    .get(source)
                    .iter()
                    .filter_map(|id| south.get(id).copied()),
            );
            ranked[first..].sort_unstable();
        }
        let mut tree = vec![0usize; south.len() + 1];
        for (seen, target) in ranked.into_iter().enumerate() {
            let mut index = target + 1;
            let mut before = 0;
            while index > 0 {
                before += tree[index];
                index &= index - 1;
            }
            total += seen - before;
            let mut index = target + 1;
            while index < tree.len() {
                tree[index] += 1;
                index += index.isolate_lowest_one();
            }
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: u32, index: usize, barycenter: Option<f64>) -> Entry {
        Entry {
            nodes: vec![NodeId(id)],
            index,
            barycenter,
            weight: if barycenter.is_some() { 1.0 } else { 0.0 },
        }
    }

    #[test]
    fn recursively_wraps_nested_compounds_with_borders() {
        let outer = NodeId(0);
        let inner = NodeId(1);
        let mut layer = LayerGraph::default();
        layer.children.insert(None, vec![outer]);
        layer
            .children
            .insert(Some(outer), vec![NodeId(2), inner, NodeId(3)]);
        layer
            .children
            .insert(Some(inner), vec![NodeId(4), NodeId(6), NodeId(5)]);
        layer.borders.insert(outer, (NodeId(2), NodeId(3)));
        layer.borders.insert(inner, (NodeId(4), NodeId(5)));
        let result = sort_subgraph(&layer, None, &[], &BTreeMap::new(), true, &BTreeMap::new());
        assert_eq!(result.nodes, [2, 4, 6, 5, 3].map(NodeId));
    }

    #[test]
    fn singleton_layer_keeps_the_upstream_no_op_sweep_result() {
        use d2_rs_core::{Node, Size};

        let mut graph = Graph::default();
        let node = graph.add_node(Node::new(
            "c",
            Size {
                width: 100.0,
                height: 100.0,
            },
        ));
        let mut layers = vec![vec![node]];
        let ranks = [(node, 0)].into();
        order_compounds(
            &graph,
            &mut layers,
            &ranks,
            &[],
            &[node],
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
        );
        assert_eq!(layers, [vec![node]]);
    }

    #[test]
    fn sibling_barycenters_override_insertion_order() {
        let entries = vec![
            entry(7, 0, Some(3.0)),
            entry(8, 1, None),
            entry(9, 2, Some(2.0)),
        ];
        assert_eq!(
            sort_entries(entries, vec![], false).nodes,
            [9, 8, 7].map(NodeId)
        );
    }

    #[test]
    fn constraints_merge_conflicting_barycenters() {
        let entries = vec![entry(7, 0, Some(3.0)), entry(8, 1, Some(2.0))];
        let merged = resolve_conflicts(entries, &[(NodeId(7), NodeId(8))]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].nodes, [7, 8].map(NodeId));
        assert_eq!(merged[0].barycenter, Some(2.5));
    }

    #[test]
    fn parallel_edges_contribute_multiplicative_crossings() {
        let layers = vec![vec![NodeId(0), NodeId(1)], vec![NodeId(2), NodeId(3)]];
        let edges = [
            (NodeId(0), NodeId(3)),
            (NodeId(0), NodeId(3)),
            (NodeId(1), NodeId(2)),
            (NodeId(1), NodeId(2)),
        ];
        assert_eq!(cross_count(&layers, &BkNeighbors::successors(&edges)), 4);
    }

    // From Dagro order_sort_subgraph_test.go: "aggregates subgraph statistics".
    #[test]
    fn aggregates_weighted_subgraph_statistics() {
        let mut layer = LayerGraph::default();
        layer.children.insert(None, vec![NodeId(5), NodeId(6)]);
        layer.incoming.insert(NodeId(5), vec![(NodeId(3), 1.0)]);
        layer
            .incoming
            .insert(NodeId(6), vec![(NodeId(1), 2.0), (NodeId(4), 1.0)]);
        let positions = (0..5).map(|id| (NodeId(id), id as usize)).collect();
        let result = sort_subgraph(&layer, None, &[], &positions, false, &BTreeMap::new());
        assert_eq!(result.nodes, [6, 5].map(NodeId));
        assert_eq!(result.barycenter, Some(2.25));
        assert_eq!(result.weight, 4.0);
    }

    // From Dagro order_sort_subgraph_test.go: "border predecessors determine
    // barycenter". Border predecessors each contribute one, independently of
    // the ordinary weighted child barycenter calculation.
    #[test]
    fn border_predecessors_determine_barycenter() {
        let mut layer = LayerGraph::default();
        layer
            .children
            .insert(Some(NodeId(4)), vec![NodeId(2), NodeId(3)]);
        layer.borders.insert(NodeId(4), (NodeId(2), NodeId(3)));
        layer.incoming.insert(NodeId(2), vec![(NodeId(0), 1.0)]);
        layer.incoming.insert(NodeId(3), vec![(NodeId(1), 1.0)]);
        let positions = [(NodeId(0), 0), (NodeId(1), 1)].into();
        let result = sort_subgraph(
            &layer,
            Some(NodeId(4)),
            &[],
            &positions,
            false,
            &BTreeMap::new(),
        );
        assert_eq!(result.nodes, [2, 3].map(NodeId));
        assert_eq!(result.barycenter, Some(0.5));
        assert_eq!(result.weight, 2.0);
    }

    // From Dagro order_resolve_conflicts_test.go: "missing barycenter always
    // conflicts", in both orientations of the constraint.
    #[test]
    fn missing_barycenter_merges_in_either_constraint_direction() {
        for (source, target) in [(NodeId(0), NodeId(1)), (NodeId(1), NodeId(0))] {
            let mut weighted = entry(1, 1, Some(1.0));
            weighted.weight = 2.0;
            let result = resolve_conflicts(vec![entry(0, 0, None), weighted], &[(source, target)]);
            assert_eq!(result.len(), 1);
            assert_eq!(result[0].nodes, [source, target]);
            assert_eq!(result[0].barycenter, Some(1.0));
            assert_eq!(result[0].weight, 2.0);
            assert_eq!(result[0].index, 0);
        }
    }

    // mergeEntries sets HasBarycenter even for 0/0. Keeping NaN distinct from a
    // missing barycenter matters to its next constraint comparison.
    #[test]
    fn zero_weight_merge_keeps_nan_barycenter_present() {
        let result = resolve_conflicts(
            vec![entry(0, 0, None), entry(1, 1, None), entry(2, 2, Some(1.0))],
            &[(NodeId(0), NodeId(1)), (NodeId(1), NodeId(2))],
        );
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].nodes, [0, 1].map(NodeId));
        assert!(result[0].barycenter.is_some_and(f64::is_nan));
        assert_eq!(result[0].weight, 0.0);
        assert_eq!(result[1].nodes, [NodeId(2)]);
    }

    // From Dagro order_add_subgraph_constraints_test.go: "multiple levels".
    #[test]
    fn accumulates_sibling_constraints_at_each_ancestor_level() {
        let mut layer = LayerGraph::default();
        for (child, parent) in [
            (1, 9),
            (9, 8),
            (2, 8),
            (3, 10),
            (10, 8),
            (5, 11),
            (6, 12),
            (12, 11),
        ] {
            layer.parents.insert(NodeId(child), Some(NodeId(parent)));
        }
        let mut constraints = Vec::new();
        add_subgraph_constraints(
            &layer,
            &mut constraints,
            &[0, 1, 2, 3, 4, 5, 6, 7].map(NodeId),
        );
        assert_eq!(
            constraints,
            [(NodeId(9), NodeId(10)), (NodeId(8), NodeId(11))]
        );
        // SetEdge updates existing constraints without changing their order.
        add_subgraph_constraints(
            &layer,
            &mut constraints,
            &[0, 1, 2, 3, 4, 5, 6, 7].map(NodeId),
        );
        assert_eq!(constraints.len(), 2);
    }

    // From Dagro order_sort_subgraph_test.go: "preserves multiple reversed
    // partners for one forward dummy" (encounter order comes from conflicts).
    #[test]
    fn restores_every_reversed_parallel_partner() {
        let mut layer = LayerGraph::default();
        layer
            .children
            .insert(None, vec![NodeId(2), NodeId(3), NodeId(4)]);
        for id in [2, 3, 4] {
            layer.incoming.insert(NodeId(id), vec![(NodeId(1), 1.0)]);
        }
        let dummy_meta = [
            (NodeId(2), (NodeId(6), NodeId(7), false)),
            (NodeId(3), (NodeId(6), NodeId(7), true)),
            (NodeId(4), (NodeId(6), NodeId(7), true)),
        ]
        .into();
        let result = sort_subgraph(
            &layer,
            None,
            &[],
            &[(NodeId(1), 1)].into(),
            false,
            &dummy_meta,
        );
        assert_eq!(result.nodes, [2, 4, 3].map(NodeId));
        assert!(result.used_bias);
    }

    #[test]
    fn edge_label_dummy_uses_tie_order_instead_of_reversed_pairing() {
        let mut layer = LayerGraph::default();
        let labeled_reverse = NodeId(2);
        let ordinary_forward = NodeId(3);
        layer
            .children
            .insert(None, vec![labeled_reverse, ordinary_forward]);
        for id in [labeled_reverse, ordinary_forward] {
            layer.incoming.insert(id, vec![(NodeId(1), 1.0)]);
        }
        let positions = [(NodeId(1), 1)].into();
        // normalizeEdge's edge-label kind is deliberately absent from this
        // map even though it has matching edgeObj endpoints and reversed=true.
        let mut dummy_meta = [(ordinary_forward, (NodeId(6), NodeId(7), false))].into();
        let labeled = sort_subgraph(&layer, None, &[], &positions, false, &dummy_meta);
        assert_eq!(labeled.nodes, [labeled_reverse, ordinary_forward]);
        assert!(labeled.used_bias);

        // With two ordinary edge dummies, upstream instead keeps the reverse
        // after its forward partner and does not count the pair as tie bias.
        dummy_meta.insert(labeled_reverse, (NodeId(6), NodeId(7), true));
        let ordinary = sort_subgraph(&layer, None, &[], &positions, false, &dummy_meta);
        assert_eq!(ordinary.nodes, [ordinary_forward, labeled_reverse]);
        assert!(!ordinary.used_bias);
    }

    // sortSubgraph's UsedBias checks JavaScript truthiness, while sortOrder
    // still orders present zero barycenters using its normal tie comparator.
    #[test]
    fn zero_barycenter_tie_does_not_switch_sweep_bias() {
        let mut layer = LayerGraph::default();
        layer.children.insert(None, vec![NodeId(1), NodeId(2)]);
        for id in [1, 2] {
            layer.incoming.insert(NodeId(id), vec![(NodeId(0), 1.0)]);
        }
        let result = sort_subgraph(
            &layer,
            None,
            &[],
            &[(NodeId(0), 0)].into(),
            true,
            &BTreeMap::new(),
        );
        assert_eq!(result.nodes, [2, 1].map(NodeId));
        assert_eq!(result.barycenter, Some(0.0));
        assert!(!result.used_bias);
    }

    #[test]
    fn expanded_unsortable_advances_cursor_once() {
        let mut expanded = entry(0, 0, None);
        expanded.nodes = [0, 1, 2].map(NodeId).to_vec();
        let result = sort_entries(
            vec![expanded, entry(4, 1, Some(2.0)), entry(5, 2, None)],
            vec![],
            false,
        );
        assert_eq!(result.nodes, [0, 1, 2, 4, 5].map(NodeId));
    }
}
