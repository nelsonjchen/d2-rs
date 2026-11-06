// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// Source-derived portions: elk-go v0.2.0 internal/common/networksimplex.
// Copyright (c) 2016 Kiel University and others.
// Copyright (c) 2026 The elk-go Authors. EPL-2.0;
// see THIRD_PARTY_NOTICES.md.

//! Source-shaped network-simplex layering used by ELK's phase two.
//!
//! The pinned elk-go implementation keeps this algorithm in a small generic
//! graph package.  This module keeps the same mutable-tree steps while using
//! the layered arena's stable node and edge IDs at the boundary.

use super::{LEdge, LEdgeId, LNodeId, LayeredArena};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

const FUZZY_SMALLER_THAN_ZERO: f64 = -1e-10;

#[derive(Clone, Debug)]
struct Node {
    id: LNodeId,
    layer: i32,
    incoming: Vec<LEdgeId>,
    outgoing: Vec<LEdgeId>,
    tree_node: bool,
    unknown_cutvalues: Vec<LEdgeId>,
}

#[derive(Clone, Debug)]
struct Edge {
    id: LEdgeId,
    source: usize,
    target: usize,
    weight: f64,
    delta: i32,
    internal_id: usize,
    tree_edge: bool,
}

struct Simplex {
    nodes: Vec<Node>,
    node_order: Vec<usize>,
    edges: Vec<Edge>,
    edge_order: Vec<usize>,
    edge_index: BTreeMap<LEdgeId, usize>,
    tree_edges: Vec<usize>,
    edge_visited: Vec<bool>,
    post_order: usize,
    post_order_id: Vec<usize>,
    lowest_post_order_id: Vec<usize>,
    cutvalue: Vec<f64>,
    previous_layering_node_counts: Vec<usize>,
    balance: bool,
    subtree_stack: Vec<(usize, usize)>,
}

impl Simplex {
    fn new(
        arena: &LayeredArena,
        component: &[LNodeId],
        edge_ids: &[LEdgeId],
        previous_layering_node_counts: &[usize],
    ) -> Self {
        let node_index = component
            .iter()
            .enumerate()
            .map(|(index, node_id)| (*node_id, index))
            .collect::<BTreeMap<_, _>>();
        let mut nodes = component
            .iter()
            .copied()
            .map(|id| Node {
                id,
                layer: 0,
                incoming: Vec::new(),
                outgoing: Vec::new(),
                tree_node: false,
                unknown_cutvalues: Vec::new(),
            })
            .collect::<Vec<_>>();
        let mut edges = Vec::new();
        let mut edge_index = BTreeMap::new();
        // networksimplex.Adapter.Layer imports edges by component node, then
        // NodeOutgoingEdges' port order. Arena insertion order changes the
        // incoming lists and the first minimum-slack edge when costs tie.
        let selected_edges = edge_ids.iter().copied().collect::<BTreeSet<_>>();
        let selected_edges = &selected_edges;
        let ordered_edges = component.iter().flat_map(|&node| {
            // Direct phase callers can omit implicit ports. The wire
            // pipeline materializes them and uses their adjacency directly.
            let outgoing = if arena.nodes[node as usize].ports.is_empty() {
                edge_ids
                    .iter()
                    .copied()
                    .filter(|&id| arena.edges[id as usize].source == node)
                    .collect()
            } else {
                arena.node_outgoing_edges(node)
            };
            outgoing
                .into_iter()
                .filter(move |id| selected_edges.contains(id))
        });
        for edge_id in ordered_edges {
            let edge = &arena.edges[edge_id as usize];
            let (Some(&source), Some(&target)) =
                (node_index.get(&edge.source), node_index.get(&edge.target))
            else {
                continue;
            };
            if source == target {
                continue;
            }
            let internal_id = edges.len();
            edges.push(Edge {
                id: edge_id,
                source,
                target,
                weight: arena
                    .edge_priority_shortness
                    .get(&edge_id)
                    .copied()
                    .unwrap_or(1.0),
                delta: 1,
                internal_id,
                tree_edge: false,
            });
            edge_index.insert(edge_id, internal_id);
            nodes[source].outgoing.push(edge_id);
            nodes[target].incoming.push(edge_id);
        }
        let node_count = nodes.len();
        Self {
            node_order: (0..node_count).collect(),
            nodes,
            edge_order: (0..edges.len()).collect(),
            edges,
            edge_index,
            tree_edges: Vec::new(),
            edge_visited: vec![false; edge_ids.len().max(1)],
            post_order: 1,
            post_order_id: vec![0; node_count],
            lowest_post_order_id: vec![0; node_count],
            cutvalue: vec![0.0; edge_ids.len()],
            previous_layering_node_counts: previous_layering_node_counts.to_vec(),
            balance: true,
            subtree_stack: Vec::new(),
        }
    }

    fn connected_edges(&self, node: usize) -> impl Iterator<Item = usize> + '_ {
        self.nodes[node]
            .incoming
            .iter()
            .chain(self.nodes[node].outgoing.iter())
            .filter_map(|edge_id| self.edge_index.get(edge_id).copied())
    }

    fn other(&self, edge: usize, node: usize) -> usize {
        let edge = &self.edges[edge];
        if edge.source == node {
            edge.target
        } else {
            edge.source
        }
    }

    fn execute(&mut self, iteration_limit: usize) {
        if self.nodes.is_empty() {
            return;
        }
        // common/networksimplex Execute removes degree-one subtrees at 40
        // nodes, then appends them in reverse removal order before balancing.
        // Keep arena indices stable while its mutable Nodes list changes.
        let prune = self.nodes.len() >= 40;
        if prune {
            self.remove_subtrees();
        }
        self.initialize();
        self.feasible_tree();
        let mut leave = self.leave_edge();
        for _ in 0..iteration_limit {
            let Some(leave_edge) = leave else { break };
            let Some(enter_edge) = self.enter_edge(leave_edge) else {
                break;
            };
            self.exchange(leave_edge, enter_edge);
            leave = self.leave_edge();
        }
        if prune {
            self.reattach_subtrees();
        }
        let filling = self.normalize();
        if self.balance {
            self.balance_layers(filling);
        }
    }

    fn initialize(&mut self) {
        for node in &mut self.nodes {
            node.layer = 0;
            node.tree_node = false;
            node.unknown_cutvalues.clear();
        }
        self.edge_order = self
            .node_order
            .iter()
            .flat_map(|&node| {
                self.nodes[node]
                    .outgoing
                    .iter()
                    .map(|id| self.edge_index[id])
            })
            .collect();
        for (index, &edge) in self.edge_order.iter().enumerate() {
            self.edges[edge].internal_id = index;
            self.edges[edge].tree_edge = false;
        }
        self.tree_edges.clear();
        self.post_order = 1;
    }

    fn remove_subtrees(&mut self) {
        self.subtree_stack.clear();
        let mut queue = self
            .node_order
            .iter()
            .copied()
            .filter(|&node| self.nodes[node].incoming.len() + self.nodes[node].outgoing.len() == 1)
            .collect::<VecDeque<_>>();
        while let Some(node) = queue.pop_front() {
            let Some(edge) = self.connected_edges(node).next() else {
                continue;
            };
            let outgoing = !self.nodes[node].outgoing.is_empty();
            let other = self.other(edge, node);
            let edge_id = self.edges[edge].id;
            let adjacent = if outgoing {
                &mut self.nodes[other].incoming
            } else {
                &mut self.nodes[other].outgoing
            };
            if let Some(index) = adjacent.iter().position(|&id| id == edge_id) {
                adjacent.remove(index);
            }
            if self.nodes[other].incoming.len() + self.nodes[other].outgoing.len() == 1 {
                queue.push_back(other);
            }
            self.subtree_stack.push((node, edge));
            if let Some(index) = self.node_order.iter().position(|&id| id == node) {
                self.node_order.remove(index);
            }
        }
    }

    fn reattach_subtrees(&mut self) {
        while let Some((node, edge)) = self.subtree_stack.pop() {
            let placed = self.other(edge, node);
            let edge = &self.edges[edge];
            if edge.target == node {
                self.nodes[placed].outgoing.push(edge.id);
                self.nodes[node].layer = self.nodes[placed].layer + edge.delta;
            } else {
                self.nodes[placed].incoming.push(edge.id);
                self.nodes[node].layer = self.nodes[placed].layer - edge.delta;
            }
            self.node_order.push(node);
        }
    }

    fn topological_layering(&mut self) {
        let mut incident = self
            .nodes
            .iter()
            .map(|node| node.incoming.len())
            .collect::<Vec<_>>();
        let mut queue = self
            .node_order
            .iter()
            .copied()
            .filter(|&node| self.nodes[node].incoming.is_empty())
            .collect::<VecDeque<_>>();
        while let Some(node) = queue.pop_front() {
            let layer = self.nodes[node].layer;
            let outgoing = self.nodes[node].outgoing.clone();
            for edge_id in outgoing {
                let edge = self.edge_index[&edge_id];
                let target = self.edges[edge].target;
                let new_layer = layer + self.edges[edge].delta;
                self.nodes[target].layer = self.nodes[target].layer.max(new_layer);
                incident[target] = incident[target].saturating_sub(1);
                if incident[target] == 0 {
                    queue.push_back(target);
                }
            }
        }
    }

    fn feasible_tree(&mut self) {
        self.topological_layering();
        if self.edge_order.is_empty() {
            return;
        }
        loop {
            self.edge_visited.fill(false);
            if self.tight_tree_dfs(self.node_order[0]) == self.node_order.len() {
                break;
            }
            let Some(edge) = self.minimal_slack() else {
                break;
            };
            let mut slack = self.nodes[self.edges[edge].target].layer
                - self.nodes[self.edges[edge].source].layer
                - self.edges[edge].delta;
            if self.nodes[self.edges[edge].target].tree_node {
                slack = -slack;
            }
            for &index in &self.node_order {
                let node = &mut self.nodes[index];
                if node.tree_node {
                    node.layer += slack;
                }
            }
        }
        self.edge_visited.fill(false);
        self.postorder_traversal(self.node_order[0]);
        self.calculate_cutvalues();
    }

    fn tight_tree_dfs(&mut self, node: usize) -> usize {
        let mut node_count = 1;
        self.nodes[node].tree_node = true;
        let connected = self.connected_edges(node).collect::<Vec<_>>();
        for edge in connected {
            let internal = self.edges[edge].internal_id;
            if self.edge_visited.get(internal).copied().unwrap_or(false) {
                continue;
            }
            if let Some(visited) = self.edge_visited.get_mut(internal) {
                *visited = true;
            }
            let opposite = self.other(edge, node);
            if self.edges[edge].tree_edge {
                node_count += self.tight_tree_dfs(opposite);
                continue;
            }
            let edge_ref = &self.edges[edge];
            let tight = edge_ref.delta
                == self.nodes[edge_ref.target].layer - self.nodes[edge_ref.source].layer;
            if !self.nodes[opposite].tree_node && tight {
                self.edges[edge].tree_edge = true;
                self.tree_edges.push(edge);
                node_count += self.tight_tree_dfs(opposite);
            }
        }
        node_count
    }

    fn minimal_slack(&self) -> Option<usize> {
        self.edge_order
            .iter()
            .copied()
            .filter(|&index| {
                let edge = &self.edges[index];
                self.nodes[edge.source].tree_node != self.nodes[edge.target].tree_node
            })
            .min_by_key(|&index| {
                let edge = &self.edges[index];
                self.nodes[edge.target].layer - self.nodes[edge.source].layer - edge.delta
            })
    }

    fn postorder_traversal(&mut self, node: usize) -> usize {
        let mut lowest = usize::MAX;
        let connected = self.connected_edges(node).collect::<Vec<_>>();
        for edge in connected {
            let internal = self.edges[edge].internal_id;
            if self.edges[edge].tree_edge && !self.edge_visited[internal] {
                self.edge_visited[internal] = true;
                lowest = lowest.min(self.postorder_traversal(self.other(edge, node)));
            }
        }
        self.post_order_id[node] = self.post_order;
        self.lowest_post_order_id[node] = lowest.min(self.post_order);
        self.post_order += 1;
        self.lowest_post_order_id[node]
    }

    fn is_in_head(&self, node: usize, edge: usize) -> bool {
        let edge = &self.edges[edge];
        let source = edge.source;
        let target = edge.target;
        let order = self.post_order_id[node];
        let inside_source =
            self.lowest_post_order_id[source] <= order && order <= self.post_order_id[source];
        let inside_target =
            self.lowest_post_order_id[target] <= order && order <= self.post_order_id[target];
        if inside_source && inside_target {
            self.post_order_id[source] >= self.post_order_id[target]
        } else {
            self.post_order_id[source] < self.post_order_id[target]
        }
    }

    fn calculate_cutvalues(&mut self) {
        let mut leaves = Vec::new();
        for node in self.node_order.clone() {
            self.nodes[node].unknown_cutvalues = self
                .connected_edges(node)
                .filter(|edge| self.edges[*edge].tree_edge)
                .map(|edge| self.edges[edge].id)
                .collect();
            if self.nodes[node].unknown_cutvalues.len() == 1 {
                leaves.push(node);
            }
        }
        for leaf in leaves {
            let mut node = leaf;
            while self.nodes[node].unknown_cutvalues.len() == 1 {
                let edge_id = self.nodes[node].unknown_cutvalues[0];
                let edge = self.edge_index[&edge_id];
                let source = self.edges[edge].source;
                let target = self.edges[edge].target;
                let mut value = self.edges[edge].weight;
                let connected = self.connected_edges(node).collect::<Vec<_>>();
                for other in connected {
                    if other == edge {
                        continue;
                    }
                    let other_edge = &self.edges[other];
                    if other_edge.tree_edge {
                        if source == other_edge.source || target == other_edge.target {
                            value -= self.cutvalue[other_edge.internal_id] - other_edge.weight;
                        } else {
                            value += self.cutvalue[other_edge.internal_id] - other_edge.weight;
                        }
                    } else if node == source {
                        if other_edge.source == node {
                            value += other_edge.weight;
                        } else {
                            value -= other_edge.weight;
                        }
                    } else if other_edge.source == node {
                        value -= other_edge.weight;
                    } else {
                        value += other_edge.weight;
                    }
                }
                self.cutvalue[self.edges[edge].internal_id] = value;
                self.nodes[source]
                    .unknown_cutvalues
                    .retain(|id| *id != edge_id);
                self.nodes[target]
                    .unknown_cutvalues
                    .retain(|id| *id != edge_id);
                node = if source == node { target } else { source };
            }
        }
    }

    fn leave_edge(&self) -> Option<usize> {
        self.tree_edges.iter().copied().find(|edge| {
            self.edges[*edge].tree_edge
                && self.cutvalue[self.edges[*edge].internal_id] < FUZZY_SMALLER_THAN_ZERO
        })
    }

    fn enter_edge(&self, leave: usize) -> Option<usize> {
        self.edge_order
            .iter()
            .copied()
            .filter(|&index| {
                let candidate = &self.edges[index];
                self.is_in_head(candidate.source, leave)
                    && !self.is_in_head(candidate.target, leave)
            })
            .min_by_key(|&index| {
                let candidate = &self.edges[index];
                self.nodes[candidate.target].layer
                    - self.nodes[candidate.source].layer
                    - candidate.delta
            })
    }

    fn exchange(&mut self, leave: usize, enter: usize) {
        self.edges[leave].tree_edge = false;
        self.tree_edges.retain(|edge| *edge != leave);
        self.edges[enter].tree_edge = true;
        self.tree_edges.push(enter);
        let edge = &self.edges[enter];
        let mut delta = self.nodes[edge.target].layer - self.nodes[edge.source].layer - edge.delta;
        if !self.is_in_head(edge.target, leave) {
            delta = -delta;
        }
        for node in self.node_order.clone() {
            if !self.is_in_head(node, leave) {
                self.nodes[node].layer += delta;
            }
        }
        self.post_order = 1;
        self.edge_visited.fill(false);
        self.postorder_traversal(self.node_order[0]);
        self.calculate_cutvalues();
    }

    fn normalize(&mut self) -> Vec<usize> {
        let lowest = self
            .node_order
            .iter()
            .map(|&node| self.nodes[node].layer)
            .min()
            .unwrap_or(0);
        let highest = self
            .node_order
            .iter()
            .map(|&node| self.nodes[node].layer)
            .max()
            .unwrap_or(0);
        let mut filling = vec![0; (highest - lowest + 1).max(1) as usize];
        for &index in &self.node_order {
            let node = &mut self.nodes[index];
            node.layer -= lowest;
            filling[node.layer as usize] += 1;
        }
        for (layer, count) in self
            .previous_layering_node_counts
            .iter()
            .copied()
            .enumerate()
        {
            if layer == filling.len() {
                break;
            }
            filling[layer] += count;
        }
        filling
    }

    fn balance_layers(&mut self, mut filling: Vec<usize>) {
        for index in self.node_order.clone() {
            if self.nodes[index].incoming.len() != self.nodes[index].outgoing.len() {
                continue;
            }
            let mut new_layer = self.nodes[index].layer;
            let (minimum_incoming, minimum_outgoing) = self.minimal_span(index);
            let start = self.nodes[index].layer - minimum_incoming + 1;
            let end = self.nodes[index].layer + minimum_outgoing;
            for layer in start..end {
                if layer >= 0
                    && (layer as usize) < filling.len()
                    && filling[layer as usize] < filling[new_layer as usize]
                {
                    new_layer = layer;
                }
            }
            if filling[new_layer as usize] < filling[self.nodes[index].layer as usize] {
                filling[self.nodes[index].layer as usize] -= 1;
                filling[new_layer as usize] += 1;
                self.nodes[index].layer = new_layer;
            }
        }
    }

    fn minimal_span(&self, node: usize) -> (i32, i32) {
        let mut incoming = i32::MAX;
        let mut outgoing = i32::MAX;
        for edge in self.connected_edges(node) {
            let edge_ref = &self.edges[edge];
            let span = self.nodes[edge_ref.target].layer - self.nodes[edge_ref.source].layer;
            if edge_ref.target == node && span < incoming {
                incoming = span;
            } else if span < outgoing {
                outgoing = span;
            }
        }
        if incoming == i32::MAX {
            incoming = -1;
        }
        if outgoing == i32::MAX {
            outgoing = -1;
        }
        (incoming, outgoing)
    }
}

/// Solve one connected component and return its normalized layer by stable
/// arena node ID.  The caller owns layer allocation and graph hierarchy rules.
pub(crate) fn layer_component(
    arena: &LayeredArena,
    component: &[LNodeId],
    edge_ids: &[LEdgeId],
    previous_layering_node_counts: &[usize],
    thoroughness: usize,
) -> BTreeMap<LNodeId, usize> {
    let mut simplex = Simplex::new(arena, component, edge_ids, previous_layering_node_counts);
    // elk-go uses iterationLimitFactor=4 and then scales by the square root
    // of the component size. Keep the integer conversion explicit so the
    // diagnostic trace and normal builds use the same budget.
    let root = (component.len() as f64).sqrt() as usize;
    let iteration_limit = thoroughness.saturating_mul(4).saturating_mul(root);
    simplex.execute(iteration_limit);
    simplex
        .nodes
        .into_iter()
        .map(|node| (node.id, node.layer.max(0) as usize))
        .collect()
}

/// Return connected components in elk-go's largest-first, stable order.
pub(crate) fn connected_components(
    arena: &LayeredArena,
    nodes: &[LNodeId],
    edge_ids: &[LEdgeId],
) -> Vec<Vec<LNodeId>> {
    let node_set = nodes.iter().copied().collect::<BTreeSet<_>>();
    let mut neighbors = BTreeMap::<LNodeId, Vec<LNodeId>>::new();
    for &node in nodes {
        neighbors.entry(node).or_default();
    }
    for &edge_id in edge_ids {
        let edge: &LEdge = &arena.edges[edge_id as usize];
        if node_set.contains(&edge.source) && node_set.contains(&edge.target) {
            neighbors.entry(edge.source).or_default().push(edge.target);
            neighbors.entry(edge.target).or_default().push(edge.source);
        }
    }
    let mut visited = BTreeSet::new();
    let mut components = Vec::new();
    for &start in nodes {
        if !visited.insert(start) {
            continue;
        }
        let mut queue = VecDeque::from([start]);
        let mut component = Vec::new();
        while let Some(node) = queue.pop_front() {
            component.push(node);
            for &neighbor in neighbors.get(&node).into_iter().flatten() {
                if visited.insert(neighbor) {
                    queue.push_back(neighbor);
                }
            }
        }
        components.push(component);
    }
    components.sort_by_key(|left| std::cmp::Reverse(left.len()));
    components
}
