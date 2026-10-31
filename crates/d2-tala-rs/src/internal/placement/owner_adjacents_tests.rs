// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
use super::super::limits::{Context, ContextError};
use super::super::node_near_map::NodeNearMap;
use super::super::shared_topology::{OwnerAbductionId, OwnerEdgeAbduction, OwnerIncidentHeader};
use super::*;
use crate::{Edge, Graph, Node, Point, Size};
use serde_json::json;
use std::cell::Cell;

const NAMES: &[&str] = &[
    "empty-capacity",
    "direct",
    "abduct-used",
    "abduct-reverse",
    "nil-abduction",
    "nil-after-used",
    "nil-incident",
    "malformed",
    "nil-endpoint",
    "unpositioned",
    "direct-nears",
    "outside-container",
    "active-cluster",
    "active-sequence",
    "inactive-cluster",
    "tree-near",
    "nil-tree",
    "nil-sentinel-endpoint",
    "cluster-fallback",
    "sequence-fallback",
    "both-fallback",
    "tree-fallback",
    "nil-member",
    "flag-missing",
    "edge-live",
    "abduction-live",
    "abduction-local",
    "abduction-future-slot",
    "incident-retained",
    "members-retained",
    "foreign-graph-live",
    "container-live",
    "cancel-priority",
    "work-limit",
];

struct Observe<'a> {
    name: &'a str,
    polls: Cell<usize>,
    nodes: [NodeId; 5],
    edges: [super::super::EdgeId; 3],
    records: [OwnerAbductionId; 2],
    abductions: OwnerAbductionHeader,
    incidents: OwnerIncidentHeader,
    group: super::super::owner_groups::OwnerGroupId,
    members: super::super::shared_topology::OwnerMemberHeader,
    foreign: OwnerGraphId,
}
impl Context for Observe<'_> {
    fn done(&self) -> Option<bool> {
        None
    }
    fn error(&self) -> Option<ContextError> {
        self.polls.set(self.polls.get() + 1);
        None
    }
    #[cfg(feature = "diagnostic-traces")]
    fn error_with_owner(&self, owner: &mut ArenaGraph) -> Option<ContextError> {
        let poll = self.polls.get() + 1;
        self.polls.set(poll);
        let n = self.nodes;
        if poll == 2 {
            match self.name {
                "edge-live" => owner
                    .set_owner_edge_endpoint(self.edges[0], true, Some(n[2]))
                    .unwrap(),
                "abduction-live" => {
                    let mut record = *owner.owner_edge_abduction(self.records[0]).unwrap();
                    record.originally_to = Some(n[4]);
                    owner
                        .set_owner_edge_abduction(self.records[0], record)
                        .unwrap();
                }
                "abduction-local" => owner
                    .write_owner_abduction_capacity_slot(&self.abductions, 0, None)
                    .unwrap(),
                "abduction-future-slot" => owner
                    .write_owner_abduction_capacity_slot(&self.abductions, 1, None)
                    .unwrap(),
                "incident-retained" => {
                    owner
                        .write_owner_incident_capacity_slot(&self.incidents, 1, None)
                        .unwrap();
                    let nil = owner.allocate_owner_incident_header(&[], 0, false).unwrap();
                    owner.set_owner_node_incident_header(n[0], nil).unwrap();
                }
                "foreign-graph-live" => owner
                    .set_owner_node_graph_reference(n[0], Some(self.foreign))
                    .unwrap(),
                "container-live" => {
                    owner.nodes[n[0].0 as usize].container = Some(n[3]);
                    owner.nodes[n[1].0 as usize].container = Some(n[3]);
                }
                "cancel-priority" => return Some(ContextError::Cancelled),
                _ => (),
            }
        }
        if poll == 3 && self.name == "members-retained" {
            owner
                .write_owner_member_capacity_slot(&self.members, 1, None)
                .unwrap();
            let mut fields = owner.owner_group_fields(self.group).unwrap();
            fields.nodes = owner.nil_owner_member_header();
            owner.set_owner_group_fields(self.group, fields).unwrap();
        }
        None
    }
}

#[cfg(feature = "diagnostic-traces")]
#[test]
fn owner_adjacents_actual_source_controls() {
    for &name in NAMES {
        let mut input = Graph::default();
        let nodes = ["A", "B", "C", "D", "E"].map(|id| {
            input.add_node(Node::new(
                id,
                Size {
                    width: 10.0,
                    height: 20.0,
                },
            ))
        });
        let edges = [(0, 1), (0, 2), (1, 3)].map(|(from, to)| {
            input.add_edge(Edge {
                source: nodes[from],
                target: nodes[to],
            })
        });
        let mut owner = ArenaGraph::from_input(&input);
        owner.initialize_shared_topology();
        for (index, &node) in nodes.iter().enumerate() {
            owner.nodes[node.0 as usize].tala_id = index as i64 + 1;
            owner.set_position(
                node,
                Point {
                    x: index as f64 * 2.0,
                    y: index as f64 * -3.0,
                },
            );
            let empty = owner.allocate_owner_incident_header(&[], 0, true).unwrap();
            owner.set_owner_node_incident_header(node, empty).unwrap();
        }
        let record = |edge, from, to, orig_to| OwnerEdgeAbduction {
            source_edge: Some(edge),
            originally_from: Some(from),
            originally_to: Some(orig_to),
            current_from: Some(from),
            current_to: Some(to),
        };
        let registered = owner
            .register_owner_edge_abductions(
                vec![
                    record(edges[0], nodes[0], nodes[1], nodes[3]),
                    record(edges[1], nodes[0], nodes[2], nodes[4]),
                ],
                true,
            )
            .unwrap();
        let records = [registered.records[0], registered.records[1]];
        let mut incident_slots = Vec::new();
        let mut abduction_slots = Vec::new();
        let mut prefix = 0;
        match name {
            "direct" | "incident-retained" => incident_slots = vec![Some(edges[0]), Some(edges[1])],
            "abduct-used" | "nil-after-used" => {
                incident_slots = vec![Some(edges[0]), Some(edges[0])];
                abduction_slots = vec![Some(records[0])];
            }
            "abduct-reverse" | "abduction-live" | "abduction-local" | "abduction-future-slot" => {
                incident_slots = vec![Some(edges[0])];
                abduction_slots = vec![Some(records[0])];
            }
            "nil-abduction" => {
                incident_slots = vec![Some(edges[0])];
                abduction_slots = vec![None];
            }
            "nil-incident" | "cancel-priority" => incident_slots = vec![None],
            "malformed" => incident_slots = vec![Some(edges[2])],
            "nil-endpoint" | "unpositioned" | "edge-live" | "work-limit" => {
                incident_slots = vec![Some(edges[0])]
            }
            _ => (),
        }
        if name == "nil-after-used" {
            abduction_slots.push(None);
        }
        if name == "abduct-reverse" {
            let mut r = *owner.owner_edge_abduction(records[0]).unwrap();
            r.current_from = Some(nodes[1]);
            r.current_to = Some(nodes[0]);
            r.originally_from = Some(nodes[4]);
            owner.set_owner_edge_abduction(records[0], r).unwrap();
        }
        if name == "abduction-future-slot" {
            abduction_slots.push(Some(records[1]));
            let mut r = *owner.owner_edge_abduction(records[0]).unwrap();
            r.current_to = Some(nodes[2]);
            owner.set_owner_edge_abduction(records[0], r).unwrap();
        }
        if matches!(
            name,
            "edge-live" | "incident-retained" | "cancel-priority" | "members-retained"
        ) {
            prefix = 63;
        }
        if matches!(
            name,
            "abduction-live" | "abduction-local" | "abduction-future-slot"
        ) {
            prefix = 62;
        }
        let incidents = owner
            .allocate_owner_incident_header(
                &incident_slots,
                if name == "empty-capacity" {
                    2
                } else {
                    incident_slots.len()
                },
                true,
            )
            .unwrap();
        owner
            .set_owner_node_incident_header(nodes[0], incidents.clone())
            .unwrap();
        let abductions = owner
            .allocate_owner_abduction_header(
                &abduction_slots,
                abduction_slots.len(),
                !abduction_slots.is_empty(),
            )
            .unwrap();
        if name == "nil-endpoint" {
            owner.set_owner_edge_endpoint(edges[0], true, None).unwrap();
        }
        if name == "unpositioned" {
            owner.nodes[nodes[1].0 as usize].position = None;
        }
        let group = owner.new_owner_group(OwnerGroupKind::Cluster);
        let sequence = owner.new_owner_group(OwnerGroupKind::Sequence);
        let members = owner
            .allocate_owner_member_header(
                &[
                    Some(nodes[1]),
                    if name == "nil-member" {
                        None
                    } else {
                        Some(nodes[2])
                    },
                ],
                2,
            )
            .unwrap();
        for (id, vessel) in [(group, nodes[2]), (sequence, nodes[4])] {
            let mut f = owner.owner_group_fields(id).unwrap();
            f.nodes = members.clone();
            f.vessel = Some(vessel);
            owner.set_owner_group_fields(id, f).unwrap();
        }
        let nears = |owner: &mut ArenaGraph, node: NodeId, values: Vec<Option<NodeId>>| {
            owner.nodes[node.0 as usize].nears = NodeNearMap::from_raw_keys(values);
        };
        if matches!(
            name,
            "direct-nears"
                | "outside-container"
                | "active-cluster"
                | "active-sequence"
                | "inactive-cluster"
                | "tree-near"
                | "nil-tree"
                | "nil-sentinel-endpoint"
                | "container-live"
        ) {
            nears(&mut owner, nodes[0], vec![Some(nodes[1])]);
            if name == "direct-nears" {
                nears(&mut owner, nodes[0], vec![Some(nodes[1]), Some(nodes[2])]);
            }
        }
        if name == "outside-container" {
            owner.nodes[nodes[0].0 as usize].container = Some(nodes[3]);
            owner.nodes[nodes[1].0 as usize].container = Some(nodes[4]);
        }
        if matches!(name, "active-cluster" | "inactive-cluster") {
            owner
                .set_owner_node_group(nodes[1], OwnerGroupKind::Cluster, Some(group))
                .unwrap();
        }
        if name == "inactive-cluster" {
            owner
                .set_owner_node_graph_reference(nodes[2], None)
                .unwrap();
        }
        if name == "active-sequence" {
            owner
                .set_owner_node_group(nodes[1], OwnerGroupKind::Sequence, Some(sequence))
                .unwrap();
        }
        let root = owner.new_owner_tree(Some(nodes[1])).unwrap();
        let child = owner.new_owner_tree(Some(nodes[2])).unwrap();
        let mut f = owner.owner_tree(root).unwrap().fields();
        f.sentinel_edge = Some(edges[2]);
        owner.set_owner_tree_fields(root, f).unwrap();
        if matches!(name, "tree-near" | "nil-tree" | "nil-sentinel-endpoint") {
            let map = owner.allocate_owner_node_to_tree_map(true);
            owner
                .write_owner_node_to_tree_entry(
                    &map,
                    Some(nodes[1]),
                    if name == "nil-tree" { None } else { Some(root) },
                )
                .unwrap();
            owner
                .set_owner_graph_node_to_tree_map(OwnerGraphId(0), map)
                .unwrap();
            if name == "nil-sentinel-endpoint" {
                owner.set_owner_edge_endpoint(edges[2], true, None).unwrap();
            }
        }
        if matches!(
            name,
            "cluster-fallback"
                | "sequence-fallback"
                | "both-fallback"
                | "tree-fallback"
                | "nil-member"
                | "members-retained"
                | "foreign-graph-live"
        ) {
            nears(&mut owner, nodes[1], vec![Some(nodes[3])]);
            nears(&mut owner, nodes[2], vec![Some(nodes[4])]);
        }
        if matches!(
            name,
            "cluster-fallback"
                | "both-fallback"
                | "nil-member"
                | "members-retained"
                | "flag-missing"
        ) {
            owner.set_owner_cluster_vessel_flag(nodes[0], true).unwrap();
            if name != "flag-missing" {
                let map = owner
                    .owner_graph_group_map(OwnerGraphId(0), OwnerGroupKind::Cluster)
                    .unwrap();
                owner
                    .write_owner_group_map_entry(&map, Some(nodes[0]), Some(group))
                    .unwrap();
            }
        }
        if matches!(name, "sequence-fallback" | "both-fallback") {
            let map = owner
                .owner_graph_group_map(OwnerGraphId(0), OwnerGroupKind::Sequence)
                .unwrap();
            owner
                .write_owner_group_map_entry(&map, Some(nodes[0]), Some(sequence))
                .unwrap();
        }
        if name == "tree-fallback" {
            let mut f = owner.owner_tree(root).unwrap().fields();
            f.children =
                super::super::owner_tree_map::OwnerTreeSlice::from_slots(&[Some(child)], 1)
                    .unwrap();
            owner.set_owner_tree_fields(root, f).unwrap();
            let roots = owner
                .allocate_owner_tree_roots_header(&[None, Some(root)], 2, true)
                .unwrap();
            let map = owner.owner_graph_tree_roots_map(OwnerGraphId(0)).unwrap();
            owner
                .write_owner_tree_roots_entry(&map, Some(nodes[0]), &roots)
                .unwrap();
        }
        let foreign = owner.create_owner_graph_view(vec![], vec![]).unwrap();
        if name == "foreign-graph-live" {
            let mut f = owner.owner_group_fields(sequence).unwrap();
            f.nodes = members.reslice_with_capacity(1, 2, 2).unwrap();
            owner.set_owner_group_fields(sequence, f).unwrap();
            let map = owner
                .owner_graph_group_map(foreign, OwnerGroupKind::Sequence)
                .unwrap();
            owner
                .write_owner_group_map_entry(&map, Some(nodes[0]), Some(sequence))
                .unwrap();
        }
        let context = Observe {
            name,
            polls: Cell::new(0),
            nodes,
            edges,
            records,
            abductions: abductions.clone(),
            incidents,
            group,
            members,
            foreign,
        };
        let mut guard = OptimizationWorkGuard::new_with_owner(
            Some(&context),
            "control",
            if name == "work-limit" { 0 } else { 100000 },
            &mut owner,
        )
        .unwrap();
        if prefix != 0 {
            guard.add_with_owner(prefix, &mut owner).unwrap();
        }
        let result = optimizer_owner_adjacents(&mut owner, Some(nodes[0]), &abductions, &mut guard);
        let mut row = json!({"case":name,"nil":true,"cap":0,"ids":null,"error":null,"used":guard.used(),"polls":context.polls.get()});
        match result {
            Ok(result) => {
                row["nil"] = json!(result.is_nil());
                row["cap"] = json!(result.capacity());
                row["ids"] = json!(
                    result
                        .require_nodes()
                        .map(|node| owner.nodes[node.0 as usize].tala_id)
                        .collect::<Vec<_>>()
                );
            }
            Err(error) => row["error"] = json!(error.to_string()),
        }
        row["abduction_slots"] = json!(
            (0..abductions.capacity())
                .map(|i| abductions
                    .read_capacity_slot(i)
                    .unwrap()
                    .map(|id| records.iter().position(|&known| known == id).unwrap()))
                .collect::<Vec<_>>()
        );
        let to_index = |node: Option<NodeId>| {
            node.map(|node| nodes.iter().position(|&known| known == node).unwrap())
        };
        row["record_fields"] = json!(
            records
                .iter()
                .map(|&id| {
                    let f = owner.owner_edge_abduction(id).unwrap();
                    [
                        to_index(f.current_from),
                        to_index(f.current_to),
                        to_index(f.originally_from),
                        to_index(f.originally_to),
                    ]
                })
                .collect::<Vec<_>>()
        );
        row["node_graphs"] = json!(
            nodes
                .iter()
                .map(|&node| owner.topology_node_owner(node).map(|id| id.0))
                .collect::<Vec<_>>()
        );
        println!("OWNER_ADJACENTS_CONTROL {row}");
    }
}

#[test]
fn canonical_group_pointer_nil_overrides_legacy_and_ancestry_uses_live_vessel() {
    let mut input = Graph::default();
    let a = input.add_node(Node::new(
        "A",
        Size {
            width: 1.0,
            height: 1.0,
        },
    ));
    let b = input.add_node(Node::new(
        "B",
        Size {
            width: 1.0,
            height: 1.0,
        },
    ));
    let mut owner = ArenaGraph::from_input(&input);
    owner.initialize_shared_topology();
    owner.nodes[a.0 as usize].cluster = Some(999);
    owner
        .set_owner_node_group(a, OwnerGroupKind::Cluster, None)
        .unwrap();
    assert_eq!(
        owner
            .owner_node_group_vessel(a, OwnerGroupKind::Cluster)
            .unwrap(),
        None
    );
    let group = owner.new_owner_group(OwnerGroupKind::Cluster);
    owner
        .set_owner_node_group(a, OwnerGroupKind::Cluster, Some(group))
        .unwrap();
    assert_eq!(
        owner
            .owner_node_group_vessel(a, OwnerGroupKind::Cluster)
            .unwrap(),
        Some(None)
    );
    let mut f = owner.owner_group_fields(group).unwrap();
    f.vessel = Some(b);
    owner.set_owner_group_fields(group, f).unwrap();
    struct Quiet;
    impl Context for Quiet {
        fn done(&self) -> Option<bool> {
            None
        }
        fn error(&self) -> Option<ContextError> {
            None
        }
    }
    let mut guard = OptimizationWorkGuard::new(Some(&Quiet), "control", 100).unwrap();
    assert!(optimizer_owner_is_descendant_of(&mut owner, Some(a), Some(b), &mut guard).unwrap());
    assert_eq!(guard.used(), 2);
    let snapshot = owner.owner_topology_snapshot();
    owner
        .set_owner_node_group(a, OwnerGroupKind::Cluster, None)
        .unwrap();
    owner.set_owner_cluster_vessel_flag(a, true).unwrap();
    owner.restore_owner_topology_snapshot(snapshot);
    assert_eq!(
        owner
            .owner_node_group_vessel(a, OwnerGroupKind::Cluster)
            .unwrap(),
        Some(Some(b))
    );
    assert!(!owner.owner_is_cluster_vessel(a));
}

#[test]
fn owner_neighbor_median_actual_source_controls() {
    use super::super::go_rng::GoRng;
    use super::super::owner_sizeless_constructor::{OwnerOptimizerRng, OwnerSizelessOptimizer};
    struct Counter(Cell<usize>);
    impl Context for Counter {
        fn done(&self) -> Option<bool> {
            None
        }
        fn error(&self) -> Option<ContextError> {
            self.0.set(self.0.get() + 1);
            None
        }
    }
    for name in [
        "median-direct",
        "median-nears",
        "median-group",
        "median-empty",
        "median-nil-incident",
    ] {
        let mut input = Graph::default();
        let nodes = ["A", "B", "C"].map(|name| {
            input.add_node(Node::new(
                name,
                Size {
                    width: 10.0,
                    height: 20.0,
                },
            ))
        });
        let edges = [(0, 1), (0, 2)].map(|(from, to)| {
            input.add_edge(Edge {
                source: nodes[from],
                target: nodes[to],
            })
        });
        let mut owner = ArenaGraph::from_input(&input);
        owner.initialize_shared_topology();
        owner.cell_size = 10.0;
        for (index, &node) in nodes.iter().enumerate() {
            owner.nodes[node.0 as usize].tala_id = index as i64 + 1;
            owner.set_position(
                node,
                Point {
                    x: index as f64 * 2.0,
                    y: index as f64 * -3.0,
                },
            );
            let slots = if index == 0 && name == "median-direct" {
                vec![Some(edges[0]), Some(edges[1])]
            } else if index == 0 && name == "median-nil-incident" {
                vec![None]
            } else {
                vec![]
            };
            let header = owner
                .allocate_owner_incident_header(&slots, slots.len(), true)
                .unwrap();
            owner.set_owner_node_incident_header(node, header).unwrap();
        }
        if name == "median-nears" {
            owner.nodes[nodes[0].0 as usize].nears =
                NodeNearMap::from_raw_keys([Some(nodes[1]), Some(nodes[2])]);
        }
        if name == "median-group" {
            let group = owner.new_owner_group(OwnerGroupKind::Cluster);
            let mut fields = owner.owner_group_fields(group).unwrap();
            fields.nodes = owner
                .allocate_owner_member_header(&[Some(nodes[1])], 1)
                .unwrap();
            fields.vessel = Some(nodes[0]);
            owner.set_owner_group_fields(group, fields).unwrap();
            owner.set_owner_cluster_vessel_flag(nodes[0], true).unwrap();
            let map = owner
                .owner_graph_group_map(OwnerGraphId(0), OwnerGroupKind::Cluster)
                .unwrap();
            owner
                .write_owner_group_map_entry(&map, Some(nodes[0]), Some(group))
                .unwrap();
            owner.nodes[nodes[1].0 as usize].nears = NodeNearMap::from_raw_keys([Some(nodes[2])]);
        }
        let rng = OwnerOptimizerRng::new(GoRng::new_untraced(1729));
        let optimizer = OwnerSizelessOptimizer::new(
            &mut owner,
            Some(OwnerGraphId(0)),
            Some(rng.clone()),
            Some(&Counter(Cell::new(0))),
        )
        .unwrap();
        let context = Counter(Cell::new(0));
        let mut guard = OptimizationWorkGuard::new(Some(&context), "control", 100000).unwrap();
        let result = optimizer.median_point_guarded(&mut owner, nodes[0], 2.0, &mut guard);
        let mut row = json!({"case":name,"point_bits":null,"error":null,"used":guard.used(),"polls":context.0.get(),"next_rng":rng.stream().borrow_mut().int63()});
        match result {
            Ok(point) => {
                let point = point.get();
                row["point_bits"] = json!([
                    format!("{:016x}", point.x.to_bits()),
                    format!("{:016x}", point.y.to_bits())
                ]);
            }
            Err(error) => row["error"] = json!(error.to_string()),
        }
        println!("OWNER_NEIGHBOR_MEDIAN_CONTROL {row}");
    }
}
