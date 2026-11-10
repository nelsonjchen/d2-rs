// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! The Graph.Edges field, separate from the canonical edge allocation table.
//! Views of the same Graph share the field; copied Graph values copy only its
//! slice header. Retained slice aliases never follow field replacement.
use super::{EdgeId, owner_pointer_slices::SharedEdgeSlice};
use crate::internal::go_cell::GoCell;
use d2_rs_core::stable_sort::StableSort;
use std::sync::Arc;

#[derive(Debug)]
pub(super) struct GraphEdges(Arc<GoCell<SharedEdgeSlice>>);
impl Clone for GraphEdges {
    fn clone(&self) -> Self {
        Self::from_header(self.header())
    }
}
impl Default for GraphEdges {
    fn default() -> Self {
        Self::from_header(SharedEdgeSlice::allocated_empty())
    }
}
impl GraphEdges {
    pub(super) fn from_header(header: SharedEdgeSlice) -> Self {
        Self(Arc::new(GoCell::new(header)))
    }
    pub(super) fn share_graph(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
    pub(super) fn header(&self) -> SharedEdgeSlice {
        self.0.read().expect("Graph.Edges lock").clone()
    }
    /// Borrow the field's current header; no handle is retained.
    pub(super) fn with_header<R>(&self, read: impl FnOnce(&SharedEdgeSlice) -> R) -> R {
        read(&self.0.read().expect("Graph.Edges lock"))
    }
    /// Refill this field with a fork of `source` when the field and its
    /// backing are uniquely owned. False means nothing was changed.
    pub(super) fn fork_storage_from(&mut self, source: &SharedEdgeSlice) -> bool {
        let Some(cell) = Arc::get_mut(&mut self.0) else {
            return false;
        };
        cell.get_mut()
            .expect("Graph.Edges lock")
            .fork_storage_into(source)
    }
    pub(super) fn replace_header(&self, header: SharedEdgeSlice) {
        *self.0.write().expect("Graph.Edges lock") = header;
    }
    // Callback-free reads borrow the field header under its guard rather
    // than cloning its Arc, as contains() does.
    pub(super) fn read(&self) -> Vec<EdgeId> {
        self.0.read().expect("Graph.Edges lock").read_values()
    }
    pub(super) fn assert_values_non_nil(&self) {
        self.0
            .read()
            .expect("Graph.Edges lock")
            .assert_values_non_nil();
    }
    pub(super) fn len(&self) -> usize {
        self.0.read().expect("Graph.Edges lock").len()
    }
    pub(super) fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub(super) fn contains(&self, edge: &EdgeId) -> bool {
        // This lookup has no callbacks and retains no slice alias. Borrow
        // the field header under its read guard instead of cloning its Arc
        // for every scoring membership test.
        self.0
            .read()
            .expect("Graph.Edges lock")
            .contains_value(edge)
    }
    pub(super) fn retain_members(&self, edges: &mut Vec<EdgeId>) {
        self.0
            .read()
            .expect("Graph.Edges lock")
            .retain_members(edges);
    }
    pub(super) fn push(&self, edge: EdgeId) {
        self.replace_header(
            self.header()
                .append(&[Some(edge)])
                .expect("Graph.Edges append"),
        );
    }
    pub(super) fn replace_values(&self, edges: &[EdgeId]) {
        self.replace_header(
            SharedEdgeSlice::from_values(edges).expect("explicit Graph.Edges literal"),
        );
    }
    pub(super) fn clear(&self) {
        self.replace_header(SharedEdgeSlice::allocated_empty());
    }
    /// Graph.Disconnect: make(0,len(old)) then append survivors. Capacity slots
    /// in the old backing retain their original canonical edge identities.
    pub(super) fn retain(&self, mut keep: impl FnMut(&EdgeId) -> bool) {
        let old = self.read();
        let mut next =
            SharedEdgeSlice::with_len_cap(0, old.len()).expect("Graph.Disconnect capacity");
        for edge in old {
            if keep(&edge) {
                next = next.append(&[Some(edge)]).expect("Graph.Disconnect append");
            }
        }
        self.replace_header(next);
    }
    #[cfg(any(test, feature = "diagnostic-traces"))]
    pub(super) fn resize(&self, len: usize, value: EdgeId) {
        let mut values = self.read();
        values.resize(len, value);
        self.replace_values(&values);
    }
    pub(super) fn from_appends(edges: &[EdgeId]) -> Self {
        let result = Self::default();
        for &edge in edges {
            result.push(edge);
        }
        result
    }
}
impl From<Vec<EdgeId>> for GraphEdges {
    fn from(edges: Vec<EdgeId>) -> Self {
        Self::from_header(SharedEdgeSlice::from_values(&edges).expect("Graph.Edges literal"))
    }
}
impl FromIterator<EdgeId> for GraphEdges {
    fn from_iter<T: IntoIterator<Item = EdgeId>>(values: T) -> Self {
        values.into_iter().collect::<Vec<_>>().into()
    }
}
impl PartialEq<Vec<EdgeId>> for GraphEdges {
    fn eq(&self, rhs: &Vec<EdgeId>) -> bool {
        self.read() == *rhs
    }
}
impl<const N: usize> PartialEq<[EdgeId; N]> for GraphEdges {
    fn eq(&self, rhs: &[EdgeId; N]) -> bool {
        self.read() == rhs
    }
}

impl GraphEdges {
    pub(super) fn disconnect(&self, removed: EdgeId) {
        let old = self.header();
        let mut next =
            SharedEdgeSlice::with_len_cap(0, old.len()).expect("Graph.Disconnect capacity");
        for slot in old.iter_slots() {
            if slot != Some(removed) {
                next = next.append(&[slot]).expect("Graph.Disconnect append");
            }
        }
        self.replace_header(next);
    }
}

impl super::ArenaGraph {
    /// layoutgraph.NewEdge and Graph.Connect. New allocations never reuse a
    /// disconnected or transaction-created canonical handle.
    pub(super) fn connect_owner_nodes(
        &mut self,
        owner: super::OwnerGraphId,
        from: super::NodeId,
        to: super::NodeId,
    ) -> EdgeId {
        let id = EdgeId(u32::try_from(self.edge_records.len()).expect("canonical edge count"));
        self.edge_records.push(super::ArenaEdge {
            input_id: id,
            tala_id: 0,
            invisible: false,
            label_position_fixed: false,
            source_label_positions: Default::default(),
            from,
            to,
            route: super::RoutePoints::literal(&[]),
            is_curve: false,
            source_arrow: false,
            target_arrow: false,
            source_arrowhead: None,
            target_arrowhead: None,
            label: None,
            source_arrowhead_label: None,
            target_arrowhead_label: None,
            style: Default::default(),
            source_table_column: None,
            target_table_column: None,
            source_table_column_count: None,
            target_table_column_count: None,
            min_width: 0.0,
            min_height: 0.0,
        });
        self.owner_graph_view(owner)
            .expect("Graph.Connect receiver")
            .edges
            .push(id);
        self.nodes[from.0 as usize].edges.push(id);
        if from != to {
            self.nodes[to.0 as usize].edges.push(id);
        }
        id
    }
}

#[cfg(feature = "diagnostic-traces")]
impl super::ArenaGraph {
    pub(super) fn observe_edge_program(
        &mut self,
        name: &str,
        operations: &[serde_json::Value],
    ) -> serde_json::Value {
        use super::{NodeId, OwnerGraphId};
        use serde_json::{Value, json};
        use std::collections::BTreeMap;
        fn target_node(g: &super::ArenaGraph, target: &str) -> NodeId {
            let id: i64 = target
                .strip_prefix("node:")
                .expect("node target")
                .parse()
                .unwrap();
            NodeId(
                g.nodes
                    .iter()
                    .position(|n| n.tala_id == id)
                    .expect("node record") as u32,
            )
        }
        fn get(g: &super::ArenaGraph, target: &str) -> SharedEdgeSlice {
            if target == "graph" {
                g.edge_order.header()
            } else {
                g.nodes[target_node(g, target).0 as usize]
                    .edges
                    .header()
                    .clone()
            }
        }
        fn set(g: &mut super::ArenaGraph, target: &str, header: SharedEdgeSlice) {
            if target == "graph" {
                g.edge_order.replace_header(header);
            } else {
                let node = target_node(g, target);
                g.nodes[node.0 as usize].edges.replace_header(header);
            }
        }
        fn edge(g: &super::ArenaGraph, id: i64) -> EdgeId {
            EdgeId(
                g.edge_records
                    .iter()
                    .position(|e| e.tala_id == id)
                    .expect("edge record") as u32,
            )
        }
        fn capture(g: &super::ArenaGraph, retained: &BTreeMap<String, SharedEdgeSlice>) -> Value {
            let mut headers = BTreeMap::from([("graph".to_owned(), g.edge_order.header())]);
            for node in &g.nodes {
                headers.insert(
                    format!("node:{}", node.tala_id),
                    node.edges.header().clone(),
                );
            }
            for (name, header) in retained {
                headers.insert(format!("retained:{name}"), header.clone());
            }
            let values=headers.iter().map(|(name,h)|(name.clone(),json!({"nil":h.is_nil(),"len":h.len(),"cap":h.capacity(),"slots":(0..h.capacity()).map(|i|h.read_capacity_slot(i).unwrap().map(|id|g.edge_records[id.0 as usize].tala_id)).collect::<Vec<_>>()}))).collect::<serde_json::Map<_,_>>();
            let entries = headers.iter().collect::<Vec<_>>();
            let mut aliases = Vec::new();
            for (i, (a, ha)) in entries.iter().enumerate() {
                for (b, hb) in &entries[i + 1..] {
                    aliases.push(
                        json!({"a":a,"b":b,"same_data":ha.data_address()==hb.data_address()}),
                    );
                }
            }
            let mut records=g.edge_records.iter().map(|e|json!({"id":e.tala_id,"from":g.nodes[e.from.0 as usize].tala_id,"to":g.nodes[e.to.0 as usize].tala_id,"points_nil":e.route.header().is_nil()})).collect::<Vec<_>>();
            records.stable_sort_by_key(|e| e["id"].as_i64().unwrap());
            json!({"headers":values,"aliases":aliases,"records":records})
        }
        let mut retained = BTreeMap::new();
        let mut snapshots = BTreeMap::new();
        let mut states = vec![capture(self, &retained)];
        for op in operations {
            let target = op["target"].as_str().unwrap_or("");
            let key = op["name"].as_str().unwrap_or("");
            match op["op"].as_str().unwrap() {
                "retain" => {
                    retained.insert(key.to_owned(), get(self, target));
                }
                "literal" => {
                    let header = if op["present"] == false {
                        SharedEdgeSlice::nil()
                    } else {
                        let slots = op["slots"].as_array().unwrap();
                        let h = SharedEdgeSlice::with_len_cap(
                            slots.len(),
                            op["capacity"].as_u64().unwrap() as usize,
                        )
                        .unwrap();
                        for (i, value) in slots.iter().enumerate() {
                            h.write_capacity_slot(i, value.as_i64().map(|id| edge(self, id)))
                                .unwrap();
                        }
                        h
                    };
                    set(self, target, header);
                }
                "reslice" => {
                    let h = retained[key]
                        .reslice_with_capacity(
                            op["low"].as_u64().unwrap() as usize,
                            op["high"].as_u64().unwrap() as usize,
                            op["max"].as_u64().unwrap() as usize,
                        )
                        .unwrap();
                    set(self, target, h);
                }
                "write" => {
                    retained[key]
                        .write_capacity_slot(
                            op["slot"].as_u64().unwrap() as usize,
                            op["edge"].as_i64().map(|id| edge(self, id)),
                        )
                        .unwrap();
                }
                "disconnect" => self.disconnect_edge(edge(self, op["edge"].as_i64().unwrap())),
                "add" => self
                    .add_owner_graph_edge(OwnerGraphId(0), edge(self, op["edge"].as_i64().unwrap()))
                    .unwrap(),
                "connect" => {
                    let from = target_node(self, &format!("node:{}", op["from"].as_i64().unwrap()));
                    let to = target_node(self, &format!("node:{}", op["to"].as_i64().unwrap()));
                    let e = self.connect_owner_nodes(OwnerGraphId(0), from, to);
                    self.edge_records[e.0 as usize].tala_id = op["id"].as_i64().unwrap();
                }
                "snapshot" => {
                    snapshots.insert(key.to_owned(), self.owner_topology_snapshot());
                }
                "rollback" => self.restore_owner_topology_snapshot(snapshots.remove(key).unwrap()),
                _ => panic!("unknown edge program operation"),
            }
            states.push(capture(self, &retained));
        }
        json!({"name":name,"states":states})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::internal::{ArenaGraph, NodeId, OwnerGraphId, graphjson};
    fn graph() -> ArenaGraph {
        let input=graphjson::graphjson_to_graph(br#"{"nodes":[{"id":1,"width":20,"height":20},{"id":2,"width":20,"height":20}],"edges":[{"id":10,"from":1,"to":2},{"id":20,"from":1,"to":1},{"id":30,"from":2,"to":1}]}"#).unwrap();
        ArenaGraph::from_input(&input)
    }
    #[test]
    fn disconnect_preserves_canonical_handles_and_retained_capacity_slots() {
        let mut g = graph();
        let graph_header = g.owner_graph_edge_header(OwnerGraphId(0)).unwrap();
        let incident = g.owner_node_incident_header(NodeId(0)).unwrap();
        g.disconnect_edge(EdgeId(1));
        assert_eq!(g.edge_records.len(), 3);
        assert_eq!(g.edge_records[1].tala_id, 20);
        assert_eq!(g.edge_records[2].tala_id, 30);
        assert_eq!(g.edge_order.read(), [EdgeId(0), EdgeId(2)]);
        let current = g.owner_graph_edge_header(OwnerGraphId(0)).unwrap();
        assert_eq!((current.len(), current.capacity()), (2, 3));
        assert_eq!(current.read_capacity_slot(2), Some(None));
        assert_eq!(
            graph_header.iter_slots().collect::<Vec<_>>(),
            [Some(EdgeId(0)), Some(EdgeId(1)), Some(EdgeId(2))]
        );
        assert_eq!(incident.read_capacity_slot(2), Some(Some(EdgeId(2))));
        assert_eq!(
            g.member_edge_entries()
                .map(|(index, e)| (index, e.tala_id))
                .collect::<Vec<_>>(),
            [(0, 10), (2, 30)]
        );
        g.disconnect_edge(EdgeId(1));
        assert_eq!(g.edge_order.header().capacity(), 2);
        assert_eq!(
            g.member_edges().map(|e| e.tala_id).collect::<Vec<_>>(),
            [10, 30]
        );
    }
    #[test]
    fn loop_disconnect_removes_twice_and_graph_membership_preserves_nil() {
        let mut g = graph();
        let h = g
            .allocate_owner_incident_header(
                &[
                    Some(EdgeId(1)),
                    Some(EdgeId(0)),
                    Some(EdgeId(1)),
                    Some(EdgeId(1)),
                ],
                6,
                true,
            )
            .unwrap();
        g.set_owner_node_incident_header(NodeId(0), h.clone())
            .unwrap();
        let graph_header = g
            .allocate_owner_incident_header(
                &[None, Some(EdgeId(1)), Some(EdgeId(2)), Some(EdgeId(1))],
                5,
                true,
            )
            .unwrap();
        g.set_owner_graph_edge_header(OwnerGraphId(0), graph_header.clone())
            .unwrap();
        g.disconnect_edge(EdgeId(1));
        assert_eq!(g.nodes[0].edges.read(), [EdgeId(0), EdgeId(1)]);
        assert_eq!(h.read_capacity_slot(2), Some(Some(EdgeId(1))));
        assert_eq!(
            g.edge_order.header().iter_slots().collect::<Vec<_>>(),
            [None, Some(EdgeId(2))]
        );
        assert_eq!(
            (g.edge_order.len(), g.edge_order.header().capacity()),
            (2, 4)
        );
        assert_eq!(graph_header.len(), 4);
    }
    #[test]
    fn disconnect_observes_nil_target_after_the_source_incident_mutation() {
        let mut g = graph();
        let graph_before = g.edge_order.read();
        g.set_owner_edge_endpoint(EdgeId(0), true, None).unwrap();
        let failure = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            g.disconnect_edge(EdgeId(0))
        }));
        assert!(failure.is_err());
        assert!(!g.nodes[0].edges.contains(&EdgeId(0)));
        assert!(g.nodes[1].edges.contains(&EdgeId(0)));
        assert_eq!(g.edge_order.read(), graph_before);
    }
    #[test]
    fn raw_graph_header_replacement_is_visible_in_ordinary_algorithms() {
        let mut g = graph();
        let h = g
            .allocate_owner_incident_header(&[Some(EdgeId(2)), Some(EdgeId(0))], 4, true)
            .unwrap();
        g.set_owner_graph_edge_header(OwnerGraphId(0), h.clone())
            .unwrap();
        assert_eq!(
            g.member_edges().map(|e| e.tala_id).collect::<Vec<_>>(),
            [30, 10]
        );
        g.write_owner_incident_capacity_slot(&h, 0, Some(EdgeId(1)))
            .unwrap();
        assert_eq!(
            g.member_edges().map(|e| e.tala_id).collect::<Vec<_>>(),
            [20, 10]
        );
        let mut filtered = vec![EdgeId(2), EdgeId(1), EdgeId(0), EdgeId(1)];
        g.edge_order.retain_members(&mut filtered);
        assert_eq!(filtered, [EdgeId(1), EdgeId(0), EdgeId(1)]);
        g.set_owner_view_edge_membership(OwnerGraphId(0), vec![EdgeId(2)])
            .unwrap();
        assert_eq!(g.edge_order.read(), [EdgeId(2)]);
        // Retained backing mutations cannot replace the current graph field.
        g.write_owner_incident_capacity_slot(&h, 1, None).unwrap();
        let mut filtered = vec![EdgeId(2), EdgeId(1), EdgeId(2), EdgeId(0)];
        g.edge_order.retain_members(&mut filtered);
        assert_eq!(filtered, [EdgeId(2), EdgeId(2)]);
        g.set_owner_graph_edge_header(OwnerGraphId(0), h).unwrap();
        let mut filtered = vec![EdgeId(2), EdgeId(1), EdgeId(0)];
        g.edge_order.retain_members(&mut filtered);
        assert_eq!(filtered, [EdgeId(1)]);
    }
    #[test]
    fn rollback_restores_membership_but_never_reuses_new_edge_identity() {
        let mut g = graph();
        let retained = g.owner_graph_edge_header(OwnerGraphId(0)).unwrap();
        let snapshot = g.owner_topology_snapshot();
        g.disconnect_edge(EdgeId(1));
        let created = g.connect_owner_nodes(OwnerGraphId(0), NodeId(1), NodeId(1));
        assert_eq!(created, EdgeId(3));
        assert!(!g.edge_records[created.0 as usize].route.header().is_nil());
        assert_eq!(
            g.nodes[1]
                .edges
                .read()
                .iter()
                .filter(|&&id| id == created)
                .count(),
            1
        );
        g.restore_owner_topology_snapshot(snapshot);
        assert_eq!(g.edge_order.read(), [EdgeId(0), EdgeId(1), EdgeId(2)]);
        assert_eq!(g.edge_records[created.0 as usize].from, NodeId(1));
        assert!(!g.nodes[1].edges.contains(&created));
        let next = g.connect_owner_nodes(OwnerGraphId(0), NodeId(0), NodeId(1));
        assert_eq!(next, EdgeId(4));
        assert_eq!(
            retained.iter_slots().collect::<Vec<_>>(),
            [Some(EdgeId(0)), Some(EdgeId(1)), Some(EdgeId(2))]
        );
    }
}
