// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Node.Edges owns a nullable shared slice header. Numerical readers take
//! temporary value images; no stored dense copy can hide raw slot mutations.
use super::{EdgeId, graph_edges::GraphEdges, owner_pointer_slices::SharedEdgeSlice};
use d2_rs_core::stable_sort::StableSort;

#[derive(Debug)]
pub(super) struct IncidentEdges(GraphEdges);
impl Clone for IncidentEdges {
    fn clone(&self) -> Self {
        Self::from_header(
            self.with_header(SharedEdgeSlice::fork_storage)
                .expect("numerical node incidence"),
        )
    }

    // A uniquely owned field and backing are refilled in place: no retained
    // field or slice header can tell that from a fresh fork.
    fn clone_from(&mut self, source: &Self) {
        if !source.with_header(|header| self.0.fork_storage_from(header)) {
            *self = source.clone();
        }
    }
}
impl Default for IncidentEdges {
    fn default() -> Self {
        Self::from_header(SharedEdgeSlice::allocated_empty())
    }
}
impl IncidentEdges {
    pub(super) fn from_header(header: SharedEdgeSlice) -> Self {
        Self(GraphEdges::from_header(header))
    }
    pub(super) fn from_field(field: GraphEdges) -> Self {
        Self(field)
    }
    pub(super) fn header(&self) -> SharedEdgeSlice {
        self.0.header()
    }
    pub(super) fn with_header<R>(&self, read: impl FnOnce(&SharedEdgeSlice) -> R) -> R {
        self.0.with_header(read)
    }
    // A retained Node.Edges field follows assignment on that same Node; a
    // retained slice header keeps its original backing and length instead.
    pub(super) fn retain_field(&self) -> GraphEdges {
        self.0.share_graph()
    }
    pub(super) fn replace_header(&self, header: SharedEdgeSlice) {
        self.0.replace_header(header);
    }
    pub(super) fn read(&self) -> Vec<EdgeId> {
        self.0.read()
    }
    pub(super) fn len(&self) -> usize {
        self.0.len()
    }
    pub(super) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub(super) fn capacity(&self) -> usize {
        self.header().capacity()
    }
    pub(super) fn contains(&self, edge: &EdgeId) -> bool {
        self.0.contains(edge)
    }
    pub(super) fn push(&mut self, edge: EdgeId) {
        self.0.push(edge);
    }
    pub(super) fn remove(&mut self, index: usize) -> EdgeId {
        let header = self.header();
        let removed = header
            .get(index)
            .expect("incident index")
            .expect("nil incident");
        for next in index + 1..header.len() {
            header
                .write_capacity_slot(next - 1, header.get(next).unwrap())
                .expect("incident shift");
        }
        self.replace_header(
            header
                .reslice_with_capacity(0, header.len() - 1, header.capacity())
                .expect("incident removal"),
        );
        removed
    }
    pub(super) fn remove_first(&mut self, edge: EdgeId) {
        self.replace_header(self.header().remove_first_slot(Some(edge)));
    }
    pub(super) fn clear(&mut self) {
        self.replace_header(SharedEdgeSlice::allocated_empty());
    }
    pub(super) fn sort_by_key<K: Ord>(&mut self, mut key: impl FnMut(&EdgeId) -> K) {
        let header = self.header();
        let mut values = header.read_values();
        values.stable_sort_by_key(|edge| key(edge));
        for (index, edge) in values.into_iter().enumerate() {
            header
                .write_capacity_slot(index, Some(edge))
                .expect("incident sort slot");
        }
    }
    pub(super) fn literal(values: &[EdgeId]) -> Self {
        Self::from_header(SharedEdgeSlice::from_values(values).expect("incident literal"))
    }
}
impl FromIterator<EdgeId> for IncidentEdges {
    fn from_iter<T: IntoIterator<Item = EdgeId>>(values: T) -> Self {
        Self::literal(&values.into_iter().collect::<Vec<_>>())
    }
}
impl PartialEq<Vec<EdgeId>> for IncidentEdges {
    fn eq(&self, values: &Vec<EdgeId>) -> bool {
        self.read() == *values
    }
}
impl PartialEq for IncidentEdges {
    fn eq(&self, other: &Self) -> bool {
        self.header().iter_slots().eq(other.header().iter_slots())
    }
}
impl From<Vec<EdgeId>> for IncidentEdges {
    fn from(values: Vec<EdgeId>) -> Self {
        Self::literal(&values)
    }
}
impl<const N: usize> PartialEq<[EdgeId; N]> for IncidentEdges {
    fn eq(&self, values: &[EdgeId; N]) -> bool {
        self.read() == values
    }
}
impl PartialEq<IncidentEdges> for Vec<EdgeId> {
    fn eq(&self, edges: &IncidentEdges) -> bool {
        *self == edges.read()
    }
}
impl IntoIterator for IncidentEdges {
    type Item = EdgeId;
    type IntoIter = std::vec::IntoIter<EdgeId>;
    fn into_iter(self) -> Self::IntoIter {
        self.read().into_iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::internal::{ArenaGraph, NodeId, OwnerGraphId, graphjson};
    fn graph() -> ArenaGraph {
        let input = graphjson::graphjson_to_graph(br#"{"nodes":[{"id":1,"width":20,"height":20},{"id":2,"width":20,"height":20}],"edges":[{"id":11,"from":1,"to":2},{"id":12,"from":1,"to":2}]}"#).unwrap();
        ArenaGraph::from_input(&input)
    }
    #[test]
    fn raw_incidence_changes_are_visible_to_ordinary_node_readers() {
        let mut graph = graph();
        let header = graph.owner_node_incident_header(NodeId(0)).unwrap();
        let field = graph.nodes[0].edges.retain_field();
        assert!(!header.is_nil());
        assert_eq!((header.len(), header.capacity()), (2, 2));
        assert!(
            !graph
                .materialize_owner_node_incident_header(NodeId(0))
                .unwrap()
        );
        graph
            .write_owner_incident_capacity_slot(&header, 0, Some(EdgeId(1)))
            .unwrap();
        assert_eq!(graph.nodes[0].edges.read(), [EdgeId(1), EdgeId(1)]);
        let nil = graph.allocate_owner_incident_header(&[], 0, false).unwrap();
        graph
            .set_owner_node_incident_header(NodeId(0), nil)
            .unwrap();
        assert!(graph.nodes[0].edges.is_empty() && graph.nodes[0].edges.header().is_nil());
        assert!(field.header().is_nil());
        assert_eq!(header.len(), 2);
        graph.nodes[0].edges.push(EdgeId(0));
        assert_eq!(
            graph.owner_node_incident_header(NodeId(0)).unwrap().get(0),
            Some(Some(EdgeId(0)))
        );
        assert_eq!(field.read(), [EdgeId(0)]);
        graph.nodes[0].edges.clear();
        assert!(field.header().is_empty() && !field.header().is_nil());
        assert_eq!(header.len(), 2);
    }
    #[test]
    fn reconnect_removes_first_duplicate_and_retains_capacity_aliases() {
        let mut graph = graph();
        let header = graph
            .allocate_owner_incident_header(
                &[Some(EdgeId(0)), Some(EdgeId(1)), Some(EdgeId(0))],
                5,
                true,
            )
            .unwrap();
        graph
            .set_owner_node_incident_header(NodeId(0), header.clone())
            .unwrap();
        graph.reconnect_owner_edge_endpoint(EdgeId(0), NodeId(0), false);
        assert_eq!(
            graph.nodes[0].edges.read(),
            [EdgeId(1), EdgeId(0), EdgeId(0)]
        );
        assert_eq!(
            header.iter_slots().collect::<Vec<_>>(),
            [Some(EdgeId(1)), Some(EdgeId(0)), Some(EdgeId(0))]
        );
        graph.nodes[0].edges.remove_first(EdgeId(0));
        assert_eq!(graph.nodes[0].edges.read(), [EdgeId(1), EdgeId(0)]);
        assert_eq!(header.read_capacity_slot(2), Some(Some(EdgeId(0))));
        assert_eq!(graph.nodes[0].edges.capacity(), 5);
    }
    #[test]
    fn numerical_clone_preserves_node_graph_aliases_without_sharing_parent() {
        let mut original = graph();
        let header = original
            .allocate_owner_incident_header(
                &[Some(EdgeId(0)), Some(EdgeId(1)), Some(EdgeId(0))],
                5,
                true,
            )
            .unwrap();
        original
            .set_owner_node_incident_header(NodeId(0), header.clone())
            .unwrap();
        original
            .set_owner_node_incident_header(
                NodeId(1),
                header.reslice_with_capacity(1, 2, 4).unwrap(),
            )
            .unwrap();
        original
            .set_owner_graph_edge_header(OwnerGraphId(0), header.clone())
            .unwrap();
        let mut copied = original.clone();
        let copied_header = copied.owner_node_incident_header(NodeId(0)).unwrap();
        assert_ne!(copied_header.slot_address(0), header.slot_address(0));
        copied
            .write_owner_incident_capacity_slot(&copied_header, 1, Some(EdgeId(0)))
            .unwrap();
        assert_eq!(copied.nodes[1].edges.read(), [EdgeId(0)]);
        assert_eq!(
            copied
                .owner_graph_edge_header(OwnerGraphId(0))
                .unwrap()
                .get(1),
            Some(Some(EdgeId(0)))
        );
        assert_eq!(original.nodes[1].edges.read(), [EdgeId(1)]);
        assert!(
            copied
                .write_owner_incident_capacity_slot(&header, 0, None)
                .is_err()
        );
    }
    #[test]
    fn topology_rollback_restores_real_node_headers_and_spare_slots() {
        let mut graph = graph();
        let header = graph
            .allocate_owner_incident_header(&[Some(EdgeId(0)), Some(EdgeId(1))], 4, true)
            .unwrap();
        graph
            .set_owner_node_incident_header(NodeId(0), header.clone())
            .unwrap();
        let snapshot = graph.owner_topology_snapshot();
        graph.nodes[0].edges.remove_first(EdgeId(0));
        graph.nodes[0].edges.push(EdgeId(0));
        graph
            .write_owner_incident_capacity_slot(&header, 3, Some(EdgeId(1)))
            .unwrap();
        let replacement = graph.allocate_owner_incident_header(&[], 0, false).unwrap();
        graph
            .set_owner_node_incident_header(NodeId(0), replacement)
            .unwrap();
        graph.restore_owner_topology_snapshot(snapshot);
        let restored = graph.owner_node_incident_header(NodeId(0)).unwrap();
        assert_eq!(restored.slot_address(0), header.slot_address(0));
        assert_eq!(graph.nodes[0].edges.read(), [EdgeId(0), EdgeId(1)]);
        assert_eq!(header.read_capacity_slot(3), Some(None));
    }
}
