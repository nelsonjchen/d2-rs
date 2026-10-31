// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Actual Graph.Hubs/CommonUncleSiblings map fields. Copying the field retains
//! the Go map allocation; views of the same graph share field replacement too.
use super::{NodeId, SharedNodeSlice, SharedSliceMap};
use std::{
    collections::BTreeMap,
    sync::Arc,
};
use crate::internal::go_cell::{GoCell};

#[derive(Debug)]
pub(super) struct GraphNodeMap(Arc<GoCell<SharedSliceMap>>);
impl Clone for GraphNodeMap {
    fn clone(&self) -> Self {
        Self::from_header(self.header())
    }
}
impl Default for GraphNodeMap {
    fn default() -> Self {
        Self::from_header(SharedSliceMap::new())
    }
}
impl GraphNodeMap {
    pub(super) fn new() -> Self {
        Self::default()
    }
    pub(super) fn nil() -> Self {
        Self::from_header(SharedSliceMap::nil())
    }
    pub(super) fn from_header(map: SharedSliceMap) -> Self {
        Self(Arc::new(GoCell::new(map)))
    }
    pub(super) fn same_field(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
    pub(super) fn share_graph(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
    /// Current map field; release before callbacks or field replacement.
    pub(super) fn read_header(&self) -> crate::internal::go_cell::GoCellRef<'_, SharedSliceMap> {
        self.0.read().expect("graph map field")
    }
    pub(super) fn header(&self) -> SharedSliceMap {
        self.read_header().clone()
    }
    pub(super) fn replace_header(&self, map: SharedSliceMap) -> SharedSliceMap {
        std::mem::replace(&mut *self.0.write().expect("graph map field"), map)
    }
    pub(super) fn len(&self) -> usize {
        self.read_header().len()
    }
    pub(super) fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub(super) fn is_nil(&self) -> bool {
        self.read_header().is_nil()
    }
    pub(super) fn clear(&self) {
        self.header().clear();
    }
    pub(super) fn get(&self, node: &NodeId) -> Option<Vec<NodeId>> {
        self.read_header().with_entry(&Some(*node), |entry| {
            entry.map(|s| s.borrowed_slots().map(|node| node.expect("nil container child")).collect())
        })
    }
    pub(super) fn contains_key(&self, node: &NodeId) -> bool {
        self.read_header().with_entry(&Some(*node), |entry| entry.is_some())
    }
    /// Explicit make(len) for projected numerical data and test literals.
    /// Source constructors and discovery publish their actual append headers.
    pub(super) fn insert(&self, node: NodeId, values: Vec<NodeId>) {
        self.header()
            .replace(
                Some(node),
                SharedNodeSlice::from_nodes(&values).expect("map literal"),
            )
            .expect("graph map assignment");
    }
    pub(super) fn iter(&self) -> std::vec::IntoIter<(NodeId, Vec<NodeId>)> {
        self.read_header().map_entries(|key, value| {
                (
                    key.expect("numerical graph map requires nonnil key"),
                    value.borrowed_slots().map(|node| node.expect("nil container child")).collect(),
                )
            })
            .into_iter()
    }
    pub(super) fn values(&self) -> std::vec::IntoIter<Vec<NodeId>> {
        self.iter().map(|(_, v)| v).collect::<Vec<_>>().into_iter()
    }
    pub(super) fn read(&self) -> BTreeMap<NodeId, Vec<NodeId>> {
        self.iter().collect()
    }
    pub(super) fn replace_values(&self, values: &BTreeMap<NodeId, Vec<NodeId>>) {
        self.replace_header(Self::from(values.clone()).header());
    }
}
impl From<BTreeMap<NodeId, Vec<NodeId>>> for GraphNodeMap {
    fn from(values: BTreeMap<NodeId, Vec<NodeId>>) -> Self {
        let map = Self::new();
        for (key, value) in values {
            map.insert(key, value);
        }
        map
    }
}
impl FromIterator<(NodeId, Vec<NodeId>)> for GraphNodeMap {
    fn from_iter<T: IntoIterator<Item = (NodeId, Vec<NodeId>)>>(values: T) -> Self {
        Self::from(values.into_iter().collect::<BTreeMap<_, _>>())
    }
}
impl IntoIterator for &GraphNodeMap {
    type Item = (NodeId, Vec<NodeId>);
    type IntoIter = std::vec::IntoIter<Self::Item>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}
impl PartialEq<BTreeMap<NodeId, Vec<NodeId>>> for GraphNodeMap {
    fn eq(&self, rhs: &BTreeMap<NodeId, Vec<NodeId>>) -> bool {
        &self.read() == rhs
    }
}

#[cfg(any(test, feature = "diagnostic-traces"))]
impl super::ArenaGraph {
    pub(super) fn observe_map_program(
        &mut self,
        name: &str,
        operations: &[serde_json::Value],
    ) -> serde_json::Value {
        use super::OwnerGraphId;
        use serde_json::{Value, json};
        fn node(g: &super::ArenaGraph, v: &Value) -> Option<NodeId> {
            v.as_i64().map(|id| {
                NodeId(
                    g.nodes
                        .iter()
                        .position(|n| n.tala_id == id)
                        .expect("probe node") as u32,
                )
            })
        }
        fn capture(
            g: &super::ArenaGraph,
            fields: &BTreeMap<String, GraphNodeMap>,
            retained: &BTreeMap<String, SharedNodeSlice>,
        ) -> Value {
            let node_id = |id: Option<NodeId>| id.map(|id| g.nodes[id.0 as usize].tala_id);
            let header = |h: &SharedNodeSlice| json!({"nil":h.is_nil(),"len":h.len(),"cap":h.capacity(),"slots":h.capacity_slots().into_iter().map(node_id).collect::<Vec<_>>()});
            let maps: BTreeMap<_, _> = fields
                .iter()
                .map(|(k, v)| (k.clone(), v.header()))
                .collect();
            let mut slices = BTreeMap::from([(
                "graph-nodes".to_owned(),
                g.owner_graph_view(OwnerGraphId(0)).unwrap().nodes.header(),
            )]);
            for (key, value) in retained {
                slices.insert(format!("retained:{key}"), value.clone());
            }
            let mut observed = serde_json::Map::new();
            for (name, map) in &maps {
                let mut entries = map.entries();
                entries.sort_by_key(|(key, _)| node_id(*key));
                let entries = entries
                    .into_iter()
                    .map(|(key, h)| {
                        let label =
                            node_id(key).map_or_else(|| "nil".to_owned(), |id| id.to_string());
                        slices.insert(format!("{name}:{label}"), h.clone());
                        json!({"key":node_id(key),"header":header(&h)})
                    })
                    .collect::<Vec<_>>();
                observed.insert(name.clone(), json!({"nil":map.is_nil(),"entries":entries}));
            }
            let pairs = maps.iter().collect::<Vec<_>>();
            let mut map_aliases = Vec::new();
            for (i, (a, am)) in pairs.iter().enumerate() {
                for (b, bm) in &pairs[i + 1..] {
                    map_aliases.push(json!({"a":a,"b":b,"same":am.same_map_allocation(bm)||(am.is_nil()&&bm.is_nil())}));
                }
            }
            let pairs = slices.iter().collect::<Vec<_>>();
            let mut slice_aliases = Vec::new();
            for (i, (a, am)) in pairs.iter().enumerate() {
                for (b, bm) in &pairs[i + 1..] {
                    slice_aliases
                        .push(json!({"a":a,"b":b,"same":am.data_address()==bm.data_address()}));
                }
            }
            let slices = slices
                .iter()
                .map(|(k, v)| (k.clone(), header(v)))
                .collect::<serde_json::Map<_, _>>();
            json!({"maps":observed,"map_aliases":map_aliases,"slices":slices,"slice_aliases":slice_aliases})
        }
        let other = self
            .create_owner_graph_view(Vec::new(), Vec::new())
            .unwrap();
        let mut fields = BTreeMap::from([
            ("hubs".to_owned(), self.hubs.share_graph()),
            (
                "siblings".to_owned(),
                self.common_uncle_siblings.share_graph(),
            ),
            (
                "other-hubs".to_owned(),
                self.owner_graph_view(other).unwrap().hubs.share_graph(),
            ),
            (
                "other-siblings".to_owned(),
                self.owner_graph_view(other)
                    .unwrap()
                    .common_uncle_siblings
                    .share_graph(),
            ),
        ]);
        let mut retained = BTreeMap::new();
        let mut snapshots = BTreeMap::new();
        let mut errors = Vec::new();
        let mut states = vec![capture(self, &fields, &retained)];
        for op in operations {
            let target = op["target"].as_str().unwrap_or("");
            let key = op["name"].as_str().unwrap_or("");
            let number = |k: &str| op[k].as_u64().unwrap() as usize;
            let result = (|| -> Result<(), super::container_slices::ContainerSliceError> {
                match op["op"].as_str().unwrap() {
                    "retain-map" => {
                        fields.insert(key.to_owned(), fields[target].clone());
                    }
                    "copy" => {
                        self.share_owner_proximity_maps(OwnerGraphId(0), other)
                            .unwrap();
                        fields["other-hubs"].replace_header(self.hubs.header());
                        fields["other-siblings"]
                            .replace_header(self.common_uncle_siblings.header());
                    }
                    "new-map" => {
                        fields[target].replace_header(if op["present"] == false {
                            SharedSliceMap::nil()
                        } else {
                            SharedSliceMap::new()
                        });
                    }
                    "put" => {
                        let h = if op["present"] == false {
                            SharedNodeSlice::nil()
                        } else {
                            let slots = op["slots"].as_array().unwrap();
                            let h = SharedNodeSlice::with_len_cap(slots.len(), number("capacity"))?;
                            for (i, v) in slots.iter().enumerate() {
                                h.write_capacity_slot(i, node(self, v))?;
                            }
                            h
                        };
                        fields[target].header().replace(node(self, &op["key"]), h)?;
                    }
                    "retain-slice" => {
                        retained.insert(
                            key.to_owned(),
                            fields[target]
                                .header()
                                .get(&node(self, &op["key"]))
                                .unwrap_or_default(),
                        );
                    }
                    "write" => {
                        retained[key]
                            .write_capacity_slot(number("slot"), node(self, &op["node"]))?;
                    }
                    "append" => {
                        let map = fields[target].header();
                        let node_key = node(self, &op["key"]);
                        let prior = map.get(&node_key).unwrap_or_default();
                        let h = prior.append(&[node(self, &op["node"])])?;
                        map.replace(node_key, h)?;
                    }
                    "alias" => {
                        fields[target].header().replace(
                            node(self, &op["key"]),
                            retained[key].reslice_with_capacity(
                                number("low"),
                                number("high"),
                                number("max"),
                            )?,
                        )?;
                    }
                    "nodes-alias" => {
                        let h = retained[key].reslice_with_capacity(
                            number("low"),
                            number("high"),
                            number("max"),
                        )?;
                        self.probe_replace_graph_nodes(h);
                    }
                    "delete" => {
                        fields[target].header().remove(&node(self, &op["key"]));
                    }
                    "clear" => fields[target].clear(),
                    "hubs" => self.compute_hubs(),
                    "siblings" => {
                        self.common_uncle_siblings
                            .replace_header(self.compute_common_uncle_map());
                    }
                    "snapshot" => {
                        snapshots.insert(key.to_owned(), self.owner_topology_snapshot());
                    }
                    "rollback" => {
                        self.restore_owner_topology_snapshot(snapshots.remove(key).unwrap());
                        fields.insert("hubs".to_owned(), self.hubs.share_graph());
                        fields.insert(
                            "siblings".to_owned(),
                            self.common_uncle_siblings.share_graph(),
                        );
                    }
                    _ => panic!("map probe operation {op}"),
                };
                Ok(())
            })();
            errors.push(result.err().map_or_else(String::new, |err| match err {
                super::container_slices::ContainerSliceError::NilMapAssignment => {
                    "assignment to entry in nil map".to_owned()
                }
                e => e.to_string(),
            }));
            states.push(capture(self, &fields, &retained));
        }
        json!({"name":name,"states":states,"errors":errors})
    }
}

#[cfg(test)]
mod tests {
    use super::super::{ArenaGraph, ContainerMap, OwnerGraphId};
    use super::*;
    use crate::{Graph, Node, Size};
    fn fixture() -> ArenaGraph {
        let mut input = Graph::default();
        for i in 0..3 {
            input.add_node(Node::new(
                i.to_string(),
                Size {
                    width: 20.0,
                    height: 20.0,
                },
            ));
        }
        ArenaGraph::from_input(&input)
    }
    #[test]
    fn graph_map_fields_follow_new_graph_nil_and_allocated_contract() {
        let graph = fixture();
        assert!(!graph.hubs.is_nil());
        assert!(graph.common_uncle_siblings.is_nil());
        let view = graph.owner_graph_view(OwnerGraphId(0)).unwrap();
        assert!(view.hubs.header().same_map_allocation(&graph.hubs.header()));
        assert!(
            graph
                .common_uncle_siblings
                .header()
                .replace(None, SharedNodeSlice::nil())
                .is_err()
        );
        let map = SharedSliceMap::new();
        map.replace(None, SharedNodeSlice::nil()).unwrap();
        view.common_uncle_siblings.replace_header(map.clone());
        assert!(
            graph
                .common_uncle_siblings
                .header()
                .same_map_allocation(&map)
        );
        assert!(
            graph
                .common_uncle_siblings
                .header()
                .get(&None)
                .unwrap()
                .is_nil()
        );
    }
    #[test]
    fn copy_entities_retains_map_and_slices_but_not_later_field_replacement() {
        let mut graph = fixture();
        let other = graph
            .create_owner_graph_view(Vec::new(), Vec::new())
            .unwrap();
        graph.hubs.insert(NodeId(0), vec![NodeId(1)]);
        graph
            .share_owner_proximity_maps(OwnerGraphId(0), other)
            .unwrap();
        let retained = graph.owner_graph_view(other).unwrap().hubs.header();
        retained.append_node(Some(NodeId(0)), NodeId(2)).unwrap();
        assert_eq!(graph.hubs.get(&NodeId(0)), Some(vec![NodeId(1), NodeId(2)]));
        let old = graph.hubs.replace_header(SharedSliceMap::new());
        assert!(retained.same_map_allocation(&old));
        assert!(graph.hubs.is_empty());
        old.clear();
        assert_eq!(retained.len(), 0);
    }
    #[test]
    fn numerical_clone_preserves_aliases_across_maps_members_and_containers() {
        let mut graph = fixture();
        let map = SharedSliceMap::new();
        let header = SharedNodeSlice::with_len_cap(2, 4).unwrap();
        header.write_capacity_slot(0, Some(NodeId(0))).unwrap();
        header.write_capacity_slot(1, Some(NodeId(1))).unwrap();
        header.write_capacity_slot(3, Some(NodeId(2))).unwrap();
        map.replace(Some(NodeId(0)), header.clone()).unwrap();
        graph.hubs.replace_header(map.clone());
        graph.common_uncle_siblings.replace_header(map.clone());
        graph.containers = ContainerMap::from_shared(map.clone());
        graph.probe_replace_graph_nodes(header.clone());
        let copy = graph.clone();
        let copied = copy.hubs.header();
        assert!(!copied.same_map_allocation(&map));
        assert!(copied.same_map_allocation(&copy.common_uncle_siblings.header()));
        assert!(copied.same_map_allocation(&copy.containers.share_map()));
        let h = copied.get(&Some(NodeId(0))).unwrap();
        assert!(
            h.same_backing_allocation(
                &copy
                    .owner_graph_view(OwnerGraphId(0))
                    .unwrap()
                    .nodes
                    .header()
            )
        );
        h.write_capacity_slot(3, None).unwrap();
        assert_eq!(header.read_capacity_slot(3), Some(Some(NodeId(2))));
    }
    #[test]
    fn map_rollback_repairs_original_field_and_retained_capacity_window() {
        let mut graph = fixture();
        let field = graph.hubs.share_graph();
        let original = field.header();
        let h = SharedNodeSlice::with_len_cap(1, 3).unwrap();
        h.write_capacity_slot(0, Some(NodeId(0))).unwrap();
        h.write_capacity_slot(2, Some(NodeId(2))).unwrap();
        original.replace(Some(NodeId(0)), h.clone()).unwrap();
        let snapshot = graph.owner_topology_snapshot();
        original.clear();
        h.write_capacity_slot(2, None).unwrap();
        let replacement = SharedSliceMap::new();
        replacement.replace(None, SharedNodeSlice::nil()).unwrap();
        field.replace_header(replacement.clone());
        graph
            .common_uncle_siblings
            .replace_header(SharedSliceMap::new());
        graph.restore_owner_topology_snapshot(snapshot);
        assert!(field.header().same_map_allocation(&original));
        assert_eq!(graph.hubs.get(&NodeId(0)), Some(vec![NodeId(0)]));
        assert_eq!(h.read_capacity_slot(2), Some(Some(NodeId(2))));
        assert!(graph.common_uncle_siblings.is_nil());
        assert_eq!(replacement.len(), 1);
    }
    #[test]
    fn ordinary_sibling_reader_observes_mutation_through_owner_header() {
        let mut graph = fixture();
        let map = SharedSliceMap::new();
        let h = SharedNodeSlice::from_nodes(&[NodeId(0), NodeId(1)]).unwrap();
        map.replace(Some(NodeId(0)), h.clone()).unwrap();
        graph
            .replace_owner_common_uncle_map(OwnerGraphId(0), Some(map))
            .unwrap();
        h.write_capacity_slot(1, Some(NodeId(2))).unwrap();
        assert_eq!(
            graph.common_uncle_siblings.get(&NodeId(0)),
            Some(vec![NodeId(0), NodeId(2)])
        );
        let alias = graph
            .owner_common_uncle_header(OwnerGraphId(0), NodeId(0))
            .unwrap()
            .unwrap();
        assert_eq!(alias.read_capacity_slot(1), Some(Some(NodeId(2))));
    }
    #[test]
    fn nil_sibling_map_is_authoritative_during_scope_projection() {
        use super::super::Pipeline;
        use crate::Edge;
        let mut input = Graph::default();
        let outside = input.add_node(Node::new(
            "outside",
            Size {
                width: 20.0,
                height: 20.0,
            },
        ));
        let container = input.add_node(Node::new(
            "container",
            Size {
                width: 80.0,
                height: 80.0,
            },
        ));
        let mut children = Vec::new();
        for i in 0..3 {
            let mut node = Node::new(
                format!("child{i}"),
                Size {
                    width: 20.0,
                    height: 20.0,
                },
            );
            node.parent = Some(container);
            let child = input.add_node(node);
            children.push(child);
            input.add_edge(Edge {
                source: child,
                target: outside,
            });
        }
        let pipeline = Pipeline::new_untraced(&input, 1, false, true);
        assert!(pipeline.graph.common_uncle_siblings.is_nil());
        assert!(
            pipeline
                .placement_scope_graph(Some(container))
                .unwrap()
                .common_uncle_groups
                .is_empty()
        );
        let map = pipeline.graph.compute_common_uncle_map();
        assert_eq!(map.len(), 3);
        pipeline.graph.common_uncle_siblings.replace_header(map);
        assert!(
            !pipeline
                .placement_scope_graph(Some(container))
                .unwrap()
                .common_uncle_groups
                .is_empty()
        );
        pipeline
            .graph
            .common_uncle_siblings
            .replace_header(SharedSliceMap::nil());
        assert!(
            pipeline
                .placement_scope_graph(Some(container))
                .unwrap()
                .common_uncle_groups
                .is_empty()
        );
    }
}
