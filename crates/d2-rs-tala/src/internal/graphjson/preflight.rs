// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! D2 v0.9.0 graphjson/preflight.go. This validates serialized records only.
//! Map walks currently use signed-ID order; upstream multi-error map traversal
//! order is nondeterministic and has not been qualified.
use super::wire::*;
use crate::internal::limits::{Context, Error, WorkGuard};
use std::collections::{BTreeMap, BTreeSet};

const MAX_NODES: usize = 10_000;
const MAX_EDGES: usize = 50_000;
const MAX_REFERENCES: usize = 1_000_000;
const MAX_ROUTE_POINTS: usize = 1_000_000;
const MAX_DEPTH: usize = 256;
const MAX_WORK: i64 = 8_000_000;
fn len<T>(value: &Option<Vec<T>>) -> usize {
    value.as_ref().map_or(0, Vec::len)
}
fn map_len<K, V>(value: &Option<BTreeMap<K, V>>) -> usize {
    value.as_ref().map_or(0, BTreeMap::len)
}
fn invalid(message: String) -> Error {
    Error::Invalid(format!("TALA DeserializeGraph {message}"))
}
fn bounded(
    total: &mut usize,
    count: usize,
    limit: usize,
    kind: &str,
    guard: &WorkGuard<'_>,
) -> Result<(), Error> {
    if count > limit - *total {
        return Err(invalid(format!("{kind} exceed limit {limit}")));
    }
    *total += count;
    guard.check()
}
struct Validation<'a, 'c> {
    guard: WorkGuard<'c>,
    node_records: usize,
    edge_records: usize,
    references: usize,
    route_points: usize,
    nodes: BTreeMap<i64, &'a SerializedNode>,
    edges: BTreeMap<i64, &'a SerializedEdge>,
    effective_edges: BTreeMap<i64, i64>,
}
impl<'a> Validation<'a, '_> {
    fn references(&mut self, count: usize, kind: &str) -> Result<(), Error> {
        bounded(
            &mut self.references,
            count,
            MAX_REFERENCES,
            &format!("{kind} references"),
            &self.guard,
        )
    }
    fn node(&mut self, node: &'a SerializedNode, location: &str) -> Result<(), Error> {
        self.guard.step()?;
        bounded(
            &mut self.node_records,
            1,
            MAX_NODES,
            "node records",
            &self.guard,
        )?;
        if node.id == 0 {
            return Err(invalid(format!("{location} uses reserved node ID 0")));
        }
        if let Some(previous) = self.nodes.get(&node.id) {
            // Option preserves Go nil/empty distinctions; float equality matches
            // reflect.DeepEqual for these independently decoded value records.
            if **previous != *node {
                return Err(invalid(format!(
                    "node ID {} has inconsistent repeated records",
                    node.id
                )));
            }
        } else {
            self.nodes.insert(node.id, node);
        }
        self.references(len(&node.nears), location)
    }
    fn edge(&mut self, edge: &'a SerializedEdge, location: &str) -> Result<(), Error> {
        self.guard.step()?;
        bounded(
            &mut self.edge_records,
            1,
            MAX_EDGES,
            "edge records",
            &self.guard,
        )?;
        if edge.id == 0 {
            return Err(invalid(format!(
                "{location} uses reserved serialized edge ID 0"
            )));
        }
        let mut effective = edge.id;
        if let Some(original) = edge.original_id {
            if edge.id >= 0 || original != 0 {
                return Err(invalid(format!(
                    "{location} has invalid original edge ID {original} for serialized ID {}",
                    edge.id
                )));
            }
            effective = original;
        }
        if effective != 0 {
            if let Some(previous) = self.effective_edges.get(&effective)
                && *previous != edge.id
            {
                return Err(invalid(format!(
                    "{location} effective edge ID {effective} is also represented by serialized ID {previous}"
                )));
            }
            self.effective_edges.insert(effective, edge.id);
        }
        if let Some(previous) = self.edges.get(&edge.id) {
            if **previous != *edge {
                return Err(invalid(format!(
                    "edge ID {} has inconsistent repeated records",
                    edge.id
                )));
            }
        } else {
            self.edges.insert(edge.id, edge);
        }
        bounded(
            &mut self.route_points,
            len(&edge.points),
            MAX_ROUTE_POINTS,
            "route points",
            &self.guard,
        )
    }
}

pub(super) fn validate(
    context: Option<&dyn Context>,
    graph: &SerializedGraph,
) -> Result<(), Error> {
    let mut guard = WorkGuard::new(context, "DeserializeGraph preflight", 60_000 * 1024)?;
    guard.set_limit(MAX_WORK);
    if len(&graph.nodes) > MAX_NODES {
        return Err(invalid(format!("node records exceed limit {MAX_NODES}")));
    }
    if len(&graph.edges) > MAX_EDGES {
        return Err(invalid(format!("edge records exceed limit {MAX_EDGES}")));
    }
    let mut v = Validation {
        guard,
        node_records: 0,
        edge_records: 0,
        references: 0,
        route_points: 0,
        nodes: BTreeMap::new(),
        edges: BTreeMap::new(),
        effective_edges: BTreeMap::new(),
    };
    for (count, kind) in [
        (map_len(&graph.containers), "container map"),
        (map_len(&graph.hubs), "hub map"),
        (map_len(&graph.clusters), "cluster map"),
        (map_len(&graph.sequences), "sequence map"),
        (map_len(&graph.cluster_vessels), "cluster vessel map"),
        (map_len(&graph.sequence_vessels), "sequence vessel map"),
        (map_len(&graph.trees), "tree map"),
        (len(&graph.hierarchies), "hierarchy list"),
        (map_len(&graph.directions), "direction map"),
    ] {
        v.references(count, kind)?;
    }
    let mut top_nodes = BTreeSet::new();
    for (i, node) in graph.nodes.iter().flatten().enumerate() {
        if !top_nodes.insert(node.id) {
            return Err(invalid(format!(
                "has duplicate node ID {} in top-level records",
                node.id
            )));
        }
        v.node(node, &format!("node {i}"))?;
    }
    let mut top_edges = BTreeSet::new();
    for (i, edge) in graph.edges.iter().flatten().enumerate() {
        if !top_edges.insert(edge.id) {
            return Err(invalid(format!(
                "has duplicate edge ID {} in top-level records",
                edge.id
            )));
        }
        v.edge(edge, &format!("edge {i}"))?;
    }
    for (map, kind) in [(&graph.containers, "container"), (&graph.hubs, "hub")] {
        for (id, members) in map.iter().flatten() {
            v.guard.step()?;
            v.references(len(members), &format!("{kind} {id}"))?;
        }
    }
    for (map, kind) in [
        (&graph.cluster_vessels, "cluster vessel"),
        (&graph.sequence_vessels, "sequence vessel"),
    ] {
        for (id, node) in map.iter().flatten() {
            v.node(node, &format!("{kind} {id}"))?;
        }
    }
    for (id, cluster) in graph.clusters.iter().flatten() {
        v.guard.step()?;
        if cluster.container == *id {
            return Err(invalid(format!("cluster {id} cannot contain itself")));
        }
        v.references(
            len(&cluster.nodes) + len(&cluster.edge_abductions),
            &format!("cluster {id}"),
        )?;
    }
    for (id, sequence) in graph.sequences.iter().flatten() {
        v.guard.step()?;
        if sequence.container == *id {
            return Err(invalid(format!("sequence {id} cannot contain itself")));
        }
        v.references(
            len(&sequence.nodes) + len(&sequence.edge_abductions),
            &format!("sequence {id}"),
        )?;
    }
    let mut tree_owners = BTreeMap::new();
    for (root_id, roots) in graph.trees.iter().flatten() {
        v.references(len(roots), &format!("tree root {root_id}"))?;
        let mut stack: Vec<_> = roots
            .as_deref()
            .unwrap_or_default()
            .iter()
            .enumerate()
            .rev()
            .map(|(i, tree)| (tree, 1, format!("tree {root_id}[{i}]")))
            .collect();
        while let Some((tree, depth, location)) = stack.pop() {
            v.guard.step()?;
            if depth > MAX_DEPTH {
                return Err(invalid(format!("tree depth exceeds limit {MAX_DEPTH}")));
            }
            if let Some(previous) = tree_owners.get(&tree.node.id) {
                return Err(invalid(format!(
                    "tree node {} has more than one tree owner ({previous} and {location})",
                    tree.node.id
                )));
            }
            if v.nodes.contains_key(&tree.node.id) && !top_nodes.contains(&tree.node.id) {
                return Err(invalid(format!(
                    "tree node {} repeats a non-top-level node record",
                    tree.node.id
                )));
            }
            tree_owners.insert(tree.node.id, location.clone());
            v.node(&tree.node, &format!("{location} node"))?;
            v.edge(&tree.sentinel_edge, &format!("{location} sentinel edge"))?;
            v.references(len(&tree.children), &format!("{location} children"))?;
            for (i, child) in tree
                .children
                .as_deref()
                .unwrap_or_default()
                .iter()
                .enumerate()
                .rev()
            {
                stack.push((child, depth + 1, format!("{location} child {i}")));
            }
        }
    }
    let mut hierarchy_owners = BTreeMap::new();
    for (i, hierarchy) in graph.hierarchies.iter().flatten().enumerate() {
        v.guard.step()?;
        v.references(map_len(&hierarchy.levels), &format!("hierarchy {i}"))?;
        for node_id in hierarchy.levels.iter().flatten().map(|(id, _)| id) {
            if let Some(previous) = hierarchy_owners.insert(node_id, i) {
                return Err(invalid(format!(
                    "hierarchy node {node_id} appears in both hierarchy {previous} and hierarchy {i}"
                )));
            }
        }
    }
    v.guard.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::internal::limits::ContextError;
    struct Background;
    impl Context for Background {
        fn done(&self) -> Option<bool> {
            None
        }
        fn error(&self) -> Option<ContextError> {
            None
        }
    }
    #[test]
    fn cancellation_precedes_record_validation() {
        struct Cancelled;
        impl Context for Cancelled {
            fn done(&self) -> Option<bool> {
                Some(true)
            }
            fn error(&self) -> Option<ContextError> {
                Some(ContextError::Cancelled)
            }
        }
        let graph = SerializedGraph {
            nodes: Some(vec![SerializedNode::default()]),
            ..Default::default()
        };
        assert_eq!(
            validate(Some(&Cancelled), &graph).unwrap_err().to_string(),
            "DeserializeGraph preflight: context canceled"
        );
        assert_eq!(
            validate(None, &graph).unwrap_err().to_string(),
            "TALA DeserializeGraph preflight requires a context"
        );
    }
    #[test]
    fn record_limits_precede_duplicate_checks_and_include_vessels() {
        let mut graph = SerializedGraph {
            nodes: Some(vec![SerializedNode::default(); MAX_NODES + 1]),
            ..Default::default()
        };
        assert_eq!(
            validate(Some(&Background), &graph).unwrap_err().to_string(),
            "TALA DeserializeGraph node records exceed limit 10000"
        );
        graph.nodes = Some(
            (1..=MAX_NODES)
                .map(|id| SerializedNode {
                    id: id as i64,
                    ..Default::default()
                })
                .collect(),
        );
        graph.cluster_vessels = Some(
            [(
                1,
                SerializedNode {
                    id: 1,
                    ..Default::default()
                },
            )]
            .into(),
        );
        assert_eq!(
            validate(Some(&Background), &graph).unwrap_err().to_string(),
            "TALA DeserializeGraph node records exceed limit 10000"
        );
    }
    #[test]
    fn repeated_records_preserve_nil_empty_distinctions() {
        let node = SerializedNode {
            id: 1,
            ..Default::default()
        };
        let mut graph = SerializedGraph {
            nodes: Some(vec![node.clone()]),
            cluster_vessels: Some([(1, node)].into()),
            ..Default::default()
        };
        validate(Some(&Background), &graph).unwrap();
        graph
            .cluster_vessels
            .as_mut()
            .unwrap()
            .get_mut(&1)
            .unwrap()
            .nears = Some(vec![]);
        assert_eq!(
            validate(Some(&Background), &graph).unwrap_err().to_string(),
            "TALA DeserializeGraph node ID 1 has inconsistent repeated records"
        );
    }
    #[test]
    fn iterative_tree_walk_checks_depth_before_repeated_records() {
        let mut tree = SerializedTree {
            node: SerializedNode {
                id: 257,
                ..Default::default()
            },
            sentinel_edge: SerializedEdge {
                id: 257,
                ..Default::default()
            },
            ..Default::default()
        };
        for id in (1..257).rev() {
            tree = SerializedTree {
                node: SerializedNode {
                    id,
                    ..Default::default()
                },
                sentinel_edge: SerializedEdge {
                    id,
                    ..Default::default()
                },
                children: Some(vec![tree]),
                ..Default::default()
            };
        }
        let graph = SerializedGraph {
            trees: Some([(0, Some(vec![tree]))].into()),
            ..Default::default()
        };
        assert_eq!(
            validate(Some(&Background), &graph).unwrap_err().to_string(),
            "TALA DeserializeGraph tree depth exceeds limit 256"
        );
    }
}
