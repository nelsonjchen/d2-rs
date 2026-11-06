// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Included in shared_topology. grouping/lifecycle.go:6–123 retains the map
// allocation and filters existing slice windows; retired arena allocations
// must never be republished as Graph.Nodes by a subsequent layout.
impl ArenaGraph {
    pub(super) fn reset_owner_cluster_records_for_layout(&mut self) -> bool {
        let map = self.shared_topology.views[0]
            .as_ref()
            .unwrap()
            .clusters
            .clone();
        if map.len() == 0 {
            return false;
        }
        let mut retired = BTreeSet::new();
        for (vessel, group) in map.entries() {
            let Some(group) = group else {
                continue;
            };
            let record = self.owner_group(group).expect("decoded cluster");
            let fields = record.fields();
            if let Some(abductions) = record.policy().edge_abductions {
                for i in 0..abductions.len() {
                    let Some(id) = abductions.get(i).flatten() else {
                        continue;
                    };
                    let record = *self.owner_edge_abduction(id).expect("cluster abduction");
                    let Some(edge) = record.source_edge else {
                        continue;
                    };
                    if let Some(node) = record.originally_from {
                        self.reconnect_owner_edge_endpoint(edge, node, false);
                    }
                    if let Some(node) = record.originally_to {
                        self.reconnect_owner_edge_endpoint(edge, node, true);
                    }
                }
            }
            for node in fields.nodes.iter_slots().flatten() {
                if self
                    .owner_node_group_id(node, OwnerGroupKind::Cluster)
                    .unwrap()
                    == Some(group)
                {
                    self.set_owner_node_group(node, OwnerGroupKind::Cluster, None)
                        .unwrap();
                    self.nodes[node.0 as usize].cluster = None;
                }
                self.nodes[node.0 as usize].nears.delete(vessel);
            }
            let Some(vessel) = vessel else {
                continue;
            };
            for near in self.nodes[vessel.0 as usize]
                .nears
                .raw_keys()
                .into_iter()
                .flatten()
            {
                self.nodes[near.0 as usize].nears.delete(Some(vessel));
            }
            self.nodes[vessel.0 as usize].nears = super::node_near_map::NodeNearMap::allocated();
            self.nodes[vessel.0 as usize].container = None;
            self.set_owner_node_graph_reference(vessel, None).unwrap();
            self.set_owner_cluster_vessel_flag(vessel, false).unwrap();
            retired.insert(vessel);
        }
        // Source Nodes[:0]/children[:0] overwrites retained backing slots,
        // preserving capacity and leaving the old tail visible to old headers.
        let filter = |header: SharedNodeSlice| {
            let mut filtered = header.reslice(0, 0).unwrap();
            for slot in header.iter_slots() {
                if !slot.is_some_and(|node| retired.contains(&node)) {
                    filtered = filtered.append(&[slot]).unwrap();
                }
            }
            filtered
        };
        if !retired.is_empty() {
            let view = self.shared_topology.views[0].as_mut().unwrap();
            view.nodes.replace(filter(view.nodes.header()));
            for container in self.containers.keys() {
                let header = self.containers.get_header(&container).unwrap();
                self.containers.replace_header(container, filter(header));
            }
        }
        for (key, _) in map.entries() {
            map.delete(key);
        }
        self.node_order = self.shared_topology.views[0]
            .as_ref()
            .unwrap()
            .nodes
            .require_nodes()
            .collect();
        self.invalidate_node_order_membership();
        true
    }
}

#[cfg(test)]
mod cluster_reset_owner_tests {
    use super::*;

    #[test]
    fn reset_retires_actual_vessel_without_resurrecting_arena_allocations() {
        let mut input = Graph::default();
        for name in ["first", "vessel", "second"] {
            input.add_node(crate::Node::new(
                name,
                Size {
                    width: 10.0,
                    height: 10.0,
                },
            ));
        }
        let mut graph = ArenaGraph::from_input(&input);
        let (first, vessel, second) = (NodeId(0), NodeId(1), NodeId(2));
        let kind = OwnerGroupKind::Cluster;
        let group = graph.new_owner_group(kind);
        let other = graph.new_owner_group(kind);
        let nodes = graph
            .allocate_owner_member_header(&[Some(first), Some(second)], 2)
            .unwrap();
        graph
            .set_owner_group_fields(
                group,
                OwnerGroupFieldView {
                    vessel: Some(vessel),
                    nodes,
                    graph: Some(OwnerGraphId(0)),
                    container: None,
                },
            )
            .unwrap();
        graph
            .set_owner_node_group(first, kind, Some(group))
            .unwrap();
        graph
            .set_owner_node_group(second, kind, Some(other))
            .unwrap();
        graph.set_owner_cluster_vessel_flag(vessel, true).unwrap();
        let map = graph.owner_graph_group_map(OwnerGraphId(0), kind).unwrap();
        graph
            .write_owner_group_map_entry(&map, Some(vessel), Some(group))
            .unwrap();
        graph.nodes[first.0 as usize].nears.push(vessel);
        graph.nodes[vessel.0 as usize].nears.push(first);
        let old_nears = graph.nodes[vessel.0 as usize].nears.retained();
        let old_nodes = graph.owner_graph_member_header(OwnerGraphId(0)).unwrap();
        let old_children = graph.containers.get_header(&None).unwrap();
        assert!(graph.reset_owner_cluster_records_for_layout());
        assert_eq!(graph.node_order, vec![first, second]);
        assert_eq!(
            old_nodes.iter_slots().collect::<Vec<_>>(),
            vec![Some(first), Some(second), Some(second)]
        );
        assert_eq!(
            old_children.require_nodes().collect::<Vec<_>>(),
            vec![first, second, second]
        );
        assert_eq!(map.len(), 0, "clear mutates the retained map");
        assert_eq!(graph.topology_node_owner(vessel), None);
        assert!(!graph.owner_is_cluster_vessel(vessel));
        assert_eq!(graph.owner_node_group_id(first, kind).unwrap(), None);
        assert_eq!(
            graph.owner_node_group_id(second, kind).unwrap(),
            Some(other)
        );
        assert!(!graph.nodes[first.0 as usize].nears.contains(&vessel));
        assert!(graph.nodes[vessel.0 as usize].nears.is_empty());
        assert!(old_nears.contains(&first), "vessel replaces its near map");
        assert_eq!(
            graph.owner_group_fields(group).unwrap().vessel,
            Some(vessel)
        );
    }
}
