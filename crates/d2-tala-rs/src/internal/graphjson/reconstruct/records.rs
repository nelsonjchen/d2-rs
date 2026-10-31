// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! codec.go's staged pointer interning, independent of Graph.Nodes membership.

use super::*;
use std::collections::BTreeSet;

#[derive(Default)]
pub(super) struct Records {
    pub nodes: Vec<SerializedNode>,
    pub node_ids: BTreeMap<i64, usize>,
    pub edges: Vec<SerializedEdge>,
    pub edge_ids: BTreeMap<i64, usize>,
    pub nil_owners: BTreeSet<i64>,
    pub nears: Vec<(i64, i64)>,
    pub trees: BTreeMap<i64, Vec<SerializedTreeState>>,
    pub cluster_abductions: BTreeMap<i64, Vec<SerializedAggregateEdgeAbduction>>,
    pub sequence_abductions: BTreeMap<i64, Vec<SerializedAggregateEdgeAbduction>>,
}

impl Records {
    fn add_node(&mut self, node: &SerializedNode) {
        self.node_ids.insert(node.id, self.nodes.len());
        self.nodes.push(node.clone());
    }

    fn add_edge(&mut self, edge: &SerializedEdge) -> Result<(), String> {
        for (end, id) in [("from", edge.from_node), ("to", edge.to_node)] {
            if !self.node_ids.contains_key(&id) {
                return Err(format!(
                    "error while deserializing: edge {} has {end} node {id} which is missing from g.Nodes",
                    edge.id
                ));
            }
        }
        self.edge_ids.insert(edge.id, self.edges.len());
        self.edges.push(edge.clone());
        Ok(())
    }

    fn vessel(
        &mut self,
        kind: &str,
        id: i64,
        vessels: Option<&BTreeMap<i64, SerializedNode>>,
    ) -> Result<(), String> {
        // Existing pointers win, including cluster vessels reused by sequences.
        // The unused map payload (even its ID) is never consulted in Go.
        if self.node_ids.contains_key(&id) {
            return Ok(());
        }
        let node = vessels.and_then(|map| map.get(&id)).ok_or_else(|| {
            format!("error while deserializing: {kind} {id} has no node or serialized vessel")
        })?;
        if node.id != id {
            return Err(format!(
                "error while deserializing: {kind} vessel key {id} does not match node ID {}",
                node.id
            ));
        }
        self.add_node(node);
        self.nil_owners.insert(id);
        Ok(())
    }

    fn group_members(
        &self,
        kind: &str,
        id: i64,
        container: i64,
        members: &[i64],
    ) -> Result<(), String> {
        if container != 0 && !self.node_ids.contains_key(&container) {
            return Err(format!(
                "error while deserializing: {kind} {id} references missing container {container}"
            ));
        }
        for member in members {
            if !self.node_ids.contains_key(member) {
                return Err(format!(
                    "error while deserializing: {kind} {id} has node {member} which is missing from g.Nodes"
                ));
            }
        }
        Ok(())
    }

    fn tree(
        &mut self,
        tree: &SerializedTree,
        sentinel: i64,
    ) -> Result<SerializedTreeState, String> {
        if !self.node_ids.contains_key(&tree.node.id) {
            // deserializeTreeNode resolves nears before publishing the new node.
            for near in tree.node.nears.as_deref().unwrap_or_default() {
                if !self.node_ids.contains_key(near) {
                    return Err(format!("could not find tree node near of ID: {near}"));
                }
                self.nears.push((tree.node.id, *near));
            }
            self.add_node(&tree.node);
        }
        if !self.edge_ids.contains_key(&tree.sentinel_edge.id) {
            self.add_edge(&tree.sentinel_edge)?;
        }
        let edge = &self.edges[self.edge_ids[&tree.sentinel_edge.id]];
        let edge_id = edge.original_id.unwrap_or(edge.id);
        let children = tree
            .children
            .as_deref()
            .unwrap_or_default()
            .iter()
            .map(|child| self.tree(child, sentinel))
            .collect::<Result<_, _>>()?;
        Ok(SerializedTreeState {
            root_sentinel_tala_id: sentinel,
            node_tala_id: tree.node.id,
            sentinel_edge_tala_id: edge_id,
            sentinel_edge_serialized_id: Some(tree.sentinel_edge.id),
            orientation: tree.orientation,
            children,
        })
    }

    pub(super) fn decode(graph: &SerializedGraph) -> Result<Self, String> {
        let mut out = Self::default();
        for node in graph.nodes.as_deref().unwrap_or_default() {
            out.add_node(node); // Duplicate original IDs are rejected by preflight.
        }
        for edge in graph.edges.as_deref().unwrap_or_default() {
            out.add_edge(edge)?;
        }
        for (parent, children) in graph.containers.as_ref().into_iter().flatten() {
            if *parent != 0 && !out.node_ids.contains_key(parent) {
                return Err(format!(
                    "error while deserializing: missing container node {parent}"
                ));
            }
            for child in children.as_deref().unwrap_or_default() {
                if !out.node_ids.contains_key(child) {
                    return Err(format!(
                        "error while deserializing: container {parent} has node {child} which is missing from g.Nodes"
                    ));
                }
            }
        }
        // These BTreeMaps follow codec.go's explicitly sorted group IDs.
        for (&id, group) in graph.clusters.as_ref().into_iter().flatten() {
            out.vessel("cluster", id, graph.cluster_vessels.as_ref())?;
            out.group_members(
                "cluster",
                id,
                group.container,
                group.nodes.as_deref().unwrap_or_default(),
            )?;
        }
        for (&id, group) in graph.sequences.as_ref().into_iter().flatten() {
            out.vessel("sequence", id, graph.sequence_vessels.as_ref())?;
            let members = group.nodes.as_deref().unwrap_or_default();
            if members.len() < 2 {
                return Err(format!(
                    "error while deserializing: sequence {id} has {} steps; want at least 2",
                    members.len()
                ));
            }
            out.group_members("sequence", id, group.container, members)?;
        }
        for (id, spokes) in graph.hubs.as_ref().into_iter().flatten() {
            if !out.node_ids.contains_key(id) {
                return Err(format!(
                    "error while deserializing: hub {id} is missing from g.Nodes"
                ));
            }
            for spoke in spokes.as_deref().unwrap_or_default() {
                if !out.node_ids.contains_key(spoke) {
                    return Err(format!(
                        "error while deserializing: hub {id} has node {spoke} which is missing from g.Nodes"
                    ));
                }
            }
        }
        for (&id, group) in graph.clusters.as_ref().into_iter().flatten() {
            out.cluster_abductions.insert(
                id,
                checked_edge_abductions(
                    group.edge_abductions.as_deref(),
                    "cluster",
                    id,
                    &out.node_ids,
                    &out.edge_ids,
                )?,
            );
        }
        for (&id, group) in graph.sequences.as_ref().into_iter().flatten() {
            out.sequence_abductions.insert(
                id,
                checked_edge_abductions(
                    group.edge_abductions.as_deref(),
                    "sequence",
                    id,
                    &out.node_ids,
                    &out.edge_ids,
                )?,
            );
        }
        // Go ranges this map without sorting. Cross-root forward references
        // remain order-sensitive; canonical tree publication is separate work.
        for (&sentinel, roots) in graph.trees.as_ref().into_iter().flatten() {
            if sentinel != 0 && !out.node_ids.contains_key(&sentinel) {
                return Err(format!(
                    "error while deserializing: tree references missing root sentinel {sentinel}"
                ));
            }
            for tree in roots.as_deref().unwrap_or_default() {
                let state = out.tree(tree, sentinel)?;
                out.trees.entry(sentinel).or_default().push(state);
            }
        }
        for node in graph.nodes.as_deref().unwrap_or_default() {
            for near in node.nears.as_deref().unwrap_or_default() {
                if !out.node_ids.contains_key(near) {
                    return Err(format!("could not find node {} near node {near}", node.id));
                }
                out.nears.push((node.id, *near));
            }
        }
        Ok(out)
    }
}
