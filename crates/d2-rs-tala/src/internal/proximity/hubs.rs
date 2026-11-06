// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Hub discovery from the upstream `internal/proximity` package.

use super::super::{ArenaGraph, BTreeMap, NodeId, SharedNodeSlice, SharedSliceMap};

impl ArenaGraph {
    pub(in crate::internal) fn compute_common_uncle_map(&self) -> SharedSliceMap {
        fn rdfs(graph: &ArenaGraph, root: Option<NodeId>, order: &mut Vec<NodeId>) {
            for child in graph
                .containers
                .get(&root)
                .unwrap_or_default()
                .iter()
                .rev()
                .copied()
            {
                if graph.nodes[child.0 as usize].is_container {
                    rdfs(graph, Some(child), order);
                    order.push(child);
                } else if let Some(index) = graph.active_cluster_index(child) {
                    if graph.clusters[index].members.first() == Some(&child) {
                        for member in graph.clusters[index].members.iter().rev().copied() {
                            if graph.nodes[member.0 as usize].is_container {
                                rdfs(graph, Some(member), order);
                                order.push(member);
                            }
                        }
                    }
                }
            }
        }
        let mut order = Vec::new();
        rdfs(self, None, &mut order);
        let map = SharedSliceMap::new();
        // Source retains orderedUncles across containers, including repeated
        // uncle keys. Their earlier position can decide equal-size ties.
        let mut uncle_order = Vec::new();
        for container in order {
            let mut by_uncle = BTreeMap::<NodeId, Vec<NodeId>>::new();
            let mut group_headers = BTreeMap::<NodeId, SharedNodeSlice>::new();
            for child in self
                .containers
                .get(&Some(container))
                .unwrap_or_default()
                .iter()
                .copied()
            {
                // An active cluster vessel is a child here; its Node.Edges are
                // the abducted member edges, exposed through the first member.
                let edge_owner = (0..self.clusters.len())
                    .find(|&index| {
                        self.topology_vessel_node(super::super::TopologyVessel::Cluster(index))
                            == Some(child)
                    })
                    .and_then(|index| self.clusters[index].members.first().copied())
                    .unwrap_or(child);
                for edge in self.active_edge_ids(edge_owner) {
                    let adjacent = self.active_adjacent(edge_owner, edge);
                    if self.active_node_container(adjacent) != self.active_node_container(container)
                    {
                        continue;
                    }
                    if !by_uncle.contains_key(&adjacent) {
                        uncle_order.push(adjacent);
                        group_headers.insert(adjacent, SharedNodeSlice::allocated_empty());
                    }
                    let siblings = by_uncle.entry(adjacent).or_default();
                    if !siblings.contains(&child) {
                        siblings.push(child);
                        let header = group_headers[&adjacent]
                            .append_node(child)
                            .expect("source common-uncle append");
                        group_headers.insert(adjacent, header);
                    }
                }
            }
            for uncle in &uncle_order {
                let Some(siblings) = by_uncle.get(uncle) else {
                    continue;
                };
                if siblings.len() < 2 {
                    continue;
                }
                let header = &group_headers[uncle];
                for &sibling in siblings {
                    if map
                        .get(&Some(sibling))
                        .is_none_or(|old| old.len() < siblings.len())
                    {
                        map.replace(Some(sibling), header.clone())
                            .expect("allocated common-uncle map");
                    }
                }
            }
        }
        map
    }

    /// AddHubs reads Node.Edges, including the remove/append lifecycle of
    /// extracted tree edges. An actual bound header is authoritative even
    /// while the numerical workspace retains its original inline adjacency.
    fn hub_incident_edges(
        &self,
        node: crate::NodeId,
    ) -> super::super::transactions::ActiveEdgeIds<'_> {
        use super::super::shared_topology::TopologyVessel;
        use super::super::transactions::ActiveEdgeIds;
        let aggregate = self
            .active_cluster_index(node)
            .map(TopologyVessel::Cluster)
            .or_else(|| {
                self.active_sequence_index(node)
                    .map(TopologyVessel::Sequence)
            });
        let actual = match aggregate {
            Some(kind) => match self.topology_vessel_node(kind) {
                Some(vessel) => vessel,
                // Before placement allocates the retained vessel, its current
                // incidence is represented by the aggregate's projected edges.
                // A carrier's own header only describes its original member.
                None => return self.active_edge_ids(node),
            },
            None => node,
        };
        match self.owner_node_incident_slots(actual) {
            Ok(slots) => ActiveEdgeIds::Owned(
                slots
                    .into_iter()
                    .map(|slot| slot.expect("AddHubs requires nonnil incident edges"))
                    .collect(),
            ),
            Err(super::super::shared_topology::OwnerTopologyError::UnboundIncidentHeader(_)) => {
                ActiveEdgeIds::Owned(self.nodes[actual.0 as usize].edges.read())
            }
            Err(error) => panic!("AddHubs invalid owner incidence: {error}"),
        }
    }

    /// Direct translation of recovered `Graph.AddHubs`.
    pub(in crate::internal) fn compute_hubs(&mut self) {
        let hubs = super::super::SharedSliceMap::new();
        // AddHubs runs after sequence/cluster preprocessing. TALA iterates the
        // current Graph.Nodes slice, whose aggregate members have been
        // replaced by vessels, and reads those vessels' current edge slices.
        // Stable Rust IDs retain the members, so reconstruct both surfaces.
        let current_nodes = self.graph_node_order();
        #[cfg(feature = "diagnostic-traces")]
        self.trace_hub_incidence("input");
        #[cfg(feature = "diagnostic-traces")]
        if crate::internal::trace_env_enabled("TALA_TRACE_HUB_MAP") {
            eprint!("HUB_EDGE_ORDER_RUST");
            for edge in &self.edge_order.read() {
                eprint!(" {}", edge.0);
            }
            eprintln!();
            eprint!("HUB_NODES_RUST");
            for node in &current_nodes {
                eprint!(
                    " {}(owner={})",
                    self.nodes[node.0 as usize].tala_id,
                    self.nodes[self.active_aggregate_owner(*node).0 as usize].tala_id
                );
            }
            eprintln!();
        }
        for node in current_nodes {
            let container = self.active_node_container(node);
            let mut has_connected = false;
            let mut spokes = Vec::new();
            for edge in self.hub_incident_edges(node) {
                let adjacent = self.active_adjacent(node, edge);
                #[cfg(feature = "diagnostic-traces")]
                if crate::internal::trace_env_enabled("TALA_TRACE_HUB_DETAIL")
                    && (crate::internal::trace_env_value("TALA_TRACE_HUB_DETAIL_NODE").is_none_or(
                        |target| {
                            target == "all"
                                || target == self.nodes[node.0 as usize].external_id
                                || target == self.nodes[adjacent.0 as usize].external_id
                        },
                    ))
                {
                    eprintln!(
                        "HUB_DETAIL_RUST node={} edge={} adjacent={} adjacent_raw_edges={} adjacent_active_edges={}",
                        self.nodes[node.0 as usize].external_id,
                        edge.0,
                        self.nodes[adjacent.0 as usize].external_id,
                        self.nodes[adjacent.0 as usize].edges.len(),
                        self.hub_incident_edges(adjacent).len()
                    );
                }
                if self.active_node_container(adjacent) != container {
                    continue;
                }
                if self.hub_incident_edges(adjacent).len() == 1 {
                    spokes.push(adjacent);
                } else {
                    has_connected = true;
                }
            }
            if has_connected && !spokes.is_empty() {
                // Pinned AddHubs uses a four-pointer stack buffer and publishes via
                // moveSliceNoCap; beyond it, ordinary growslice retains capacity.
                let header = SharedNodeSlice::from_go127_buffered_appends(&spokes)
                    .expect("AddHubs local slice publication");
                hubs.replace(Some(node), header)
                    .expect("AddHubs map assignment");
            }
        }
        self.hubs.replace_header(hubs);
        #[cfg(feature = "diagnostic-traces")]
        self.trace_hub_incidence("output");
        #[cfg(feature = "diagnostic-traces")]
        if crate::internal::trace_env_enabled("TALA_TRACE_HUB_MAP") {
            eprint!("HUB_MAP_RUST");
            for (hub, spokes) in &self.hubs {
                eprint!(" {}=[", self.active_node_tala_id(hub));
                for spoke in spokes {
                    eprint!(" {}", self.active_node_tala_id(spoke));
                }
                eprint!(" ]");
            }
            eprintln!();
        }
    }

    #[cfg(feature = "diagnostic-traces")]
    fn trace_hub_incidence(&self, phase: &str) {
        if !crate::internal::trace_env_enabled("D2RS_TRACE_HUB_INCIDENCE") {
            return;
        }
        let node_id = |id: Option<crate::NodeId>| {
            id.map_or_else(
                || "0".to_owned(),
                |id| self.active_node_tala_id(id).to_string(),
            )
        };
        let nodes = self.graph_node_order().into_iter().map(|node| {
            let edges = self.hub_incident_edges(node).into_iter().map(|edge| {
                let adjacent = self.active_adjacent(node, edge);
                serde_json::json!({
                    "id": self.edge_records[edge.0 as usize].tala_id.to_string(),
                    "adjacent": node_id(Some(adjacent)),
                    "adjacent_container": node_id(self.active_node_container(adjacent)),
                    "adjacent_degree": self.hub_incident_edges(adjacent).len(),
                })
            }).collect::<Vec<_>>();
            serde_json::json!({"id": node_id(Some(node)), "container": node_id(self.active_node_container(node)), "edges": edges})
        }).collect::<Vec<_>>();
        let hubs = self
            .hubs
            .iter()
            .map(|(node, spokes)| {
                (
                    node_id(Some(node)),
                    spokes
                        .iter()
                        .map(|&spoke| node_id(Some(spoke)))
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        eprintln!(
            "{}",
            serde_json::json!({"event": "hub-incidence", "phase": phase, "nodes": nodes, "hubs": hubs})
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Edge, Graph, Node, Size};

    #[test]
    fn aggregate_hub_incidence_switches_from_carrier_to_bound_vessel() {
        use crate::internal::{ClusterArrangement, ClusterState, TopologyVessel};
        let mut input = Graph::default();
        let ids = (0..3)
            .map(|i| {
                input.add_node(Node::new(
                    i.to_string(),
                    Size {
                        width: 40.0,
                        height: 40.0,
                    },
                ))
            })
            .collect::<Vec<_>>();
        let [first, second, peer]: [_; 3] = ids.try_into().unwrap();
        let a = input.add_edge(Edge {
            source: first,
            target: peer,
        });
        let b = input.add_edge(Edge {
            source: second,
            target: peer,
        });
        let mut graph = ArenaGraph::from_input(&input);
        graph.clusters.push(ClusterState {
            members: vec![first, second],
            arrangement: ClusterArrangement::Row,
            desired_arrangement: ClusterArrangement::Row,
            padding: 20.0,
            vessel_tala_id: 10001,
            fixed_size: false,
        });
        for member in [first, second] {
            graph.nodes[member.0 as usize].cluster = Some(0);
        }
        graph.node_order = vec![first, peer];
        graph.materialize_owner_node_incident_header(first).unwrap();
        assert_eq!(
            graph
                .hub_incident_edges(first)
                .into_iter()
                .collect::<Vec<_>>(),
            vec![a, b]
        );
        let mut entity = graph.nodes[peer.0 as usize].clone();
        entity.tala_id = 10001;
        entity.edges.clear();
        let vessel = graph.retain_topology_node(entity);
        graph
            .bind_topology_vessel(TopologyVessel::Cluster(0), vessel)
            .unwrap();
        let header = graph
            .allocate_owner_incident_header(&[Some(b), Some(a), Some(b)], 3, true)
            .unwrap();
        graph
            .set_owner_node_incident_header(vessel, header)
            .unwrap();
        assert_eq!(
            graph
                .hub_incident_edges(first)
                .into_iter()
                .collect::<Vec<_>>(),
            vec![b, a, b]
        );
    }

    #[test]
    fn hub_spokes_follow_current_incident_header_with_duplicates_and_external_edges() {
        let mut input = Graph::default();
        let ids = (0..5)
            .map(|i| {
                input.add_node(Node::new(
                    i.to_string(),
                    Size {
                        width: 40.0,
                        height: 40.0,
                    },
                ))
            })
            .collect::<Vec<_>>();
        let [hub, a, b, connected, tail]: [_; 5] = ids.try_into().unwrap();
        let ea = input.add_edge(Edge {
            source: hub,
            target: a,
        });
        let eb = input.add_edge(Edge {
            source: hub,
            target: b,
        });
        let ec = input.add_edge(Edge {
            source: hub,
            target: connected,
        });
        input.add_edge(Edge {
            source: connected,
            target: tail,
        });
        let mut graph = ArenaGraph::from_input(&input);
        graph.initialize_shared_topology();
        let header = graph
            .allocate_owner_incident_header(&[Some(eb), Some(ec), Some(ea), Some(eb)], 4, true)
            .unwrap();
        graph.set_owner_node_incident_header(hub, header).unwrap();
        // The graph slice can omit edges retained by shared endpoint pointers.
        // AddHubs only asks the current Node.Edges fields, not Graph.Edges.
        graph.edge_order.clear();
        graph.compute_hubs();
        assert_eq!(graph.hubs.get(&hub).unwrap(), vec![b, a, b]);
        assert_eq!(graph.nodes[hub.0 as usize].edges, vec![eb, ec, ea, eb]);
        graph.remove_owner_incident_slot(hub, Some(eb)).unwrap();
        graph.compute_hubs();
        assert_eq!(graph.hubs.get(&hub).unwrap(), vec![a, b]);
    }
}
