// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use super::super::limits::{Context, ContextError, OptimizationWorkGuard};
use super::super::shared_topology::{OwnerGraphId, OwnerIncidentHeader, OwnerTopologyError};
use super::super::{ArenaGraph, NodeId};
use super::*;
use crate::{Edge, Graph, Node, Size};
use serde_json::{Value, json};
struct Quiet;
impl Context for Quiet {
    fn done(&self) -> Option<bool> {
        None
    }
    fn error(&self) -> Option<ContextError> {
        None
    }
}
fn fixture() -> (ArenaGraph, [NodeId; 3], [EdgeId; 3]) {
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
    let edges = [(0, 1), (0, 2), (1, 2)].map(|(from, to)| {
        input.add_edge(Edge {
            source: nodes[from],
            target: nodes[to],
        })
    });
    let mut owner = ArenaGraph::from_input(&input);
    owner.initialize_shared_topology();
    for &node in &nodes {
        let empty = owner.allocate_owner_incident_header(&[], 0, true).unwrap();
        owner.set_owner_node_incident_header(node, empty).unwrap()
    }
    let empty = owner.allocate_owner_incident_header(&[], 0, true).unwrap();
    owner
        .set_owner_graph_edge_header(OwnerGraphId(0), empty)
        .unwrap();
    (owner, nodes, edges)
}
fn header(
    owner: &mut ArenaGraph,
    slots: &[Option<EdgeId>],
    capacity: usize,
) -> OwnerIncidentHeader {
    owner
        .allocate_owner_incident_header(slots, capacity, true)
        .unwrap()
}
fn observe(h: Option<&OwnerIncidentHeader>, edges: &[EdgeId; 3]) -> Value {
    let Some(h) = h else {
        return json!({"present":false,"nil":true,"len":0,"cap":0,"slots":[]});
    };
    let slots: Vec<_> = (0..h.capacity())
        .map(|i| {
            h.read_capacity_slot(i)
                .unwrap()
                .map(|edge| edges.iter().position(|&known| known == edge).unwrap())
        })
        .collect();
    json!({"present":true,"nil":h.is_nil(),"len":h.len(),"cap":h.capacity(),"slots":slots})
}
#[test]
fn owner_incidents_actual_source_controls() {
    for name in [
        "nil",
        "empty",
        "nullable",
        "append-spare",
        "append-grow",
        "remove-first",
        "remove-nil",
        "same-endpoint",
        "reconnect-to",
        "iterator-live",
        "restore-overlap",
        "nil-endpoint",
    ] {
        let (mut owner, nodes, edges) = fixture();
        let mut retained = None;
        let mut replacement = None;
        let mut yielded = None;
        if name == "nil" {
            let nil = owner.allocate_owner_incident_header(&[], 0, false).unwrap();
            owner.set_owner_node_incident_header(nodes[0], nil).unwrap()
        }
        if name == "nullable" {
            let h = header(&mut owner, &[None, Some(edges[1])], 4);
            owner
                .write_owner_incident_capacity_slot(&h, 3, Some(edges[2]))
                .unwrap();
            owner.set_owner_node_incident_header(nodes[0], h).unwrap()
        }
        if name == "append-spare"
            || name == "remove-first"
            || name == "remove-nil"
            || name == "same-endpoint"
            || name == "reconnect-to"
        {
            let slots = if name == "remove-nil" {
                vec![None, Some(edges[0]), None]
            } else {
                vec![Some(edges[0]), Some(edges[1]), Some(edges[0])]
            };
            let h = header(&mut owner, &slots, 5);
            owner
                .write_owner_incident_capacity_slot(&h, 4, Some(edges[2]))
                .unwrap();
            owner
                .set_owner_node_incident_header(nodes[0], h.clone())
                .unwrap();
            retained = Some(h);
            if name == "append-spare" {
                owner.append_owner_incident_slot(nodes[0], None).unwrap()
            }
            if name == "remove-first" {
                owner
                    .remove_owner_incident_slot(nodes[0], Some(edges[0]))
                    .unwrap()
            }
            if name == "remove-nil" {
                owner.remove_owner_incident_slot(nodes[0], None).unwrap()
            }
            if name == "same-endpoint" {
                owner.reconnect_owner_edge_endpoint(edges[0], nodes[0], false)
            }
            if name == "reconnect-to" {
                let b = header(&mut owner, &[Some(edges[0]), Some(edges[2])], 3);
                owner.set_owner_node_incident_header(nodes[1], b).unwrap();
                owner.reconnect_owner_edge_endpoint(edges[0], nodes[2], true)
            }
        }
        if name == "append-grow" {
            let h = header(&mut owner, &vec![Some(edges[0]); 64], 64);
            owner
                .set_owner_node_incident_header(nodes[0], h.clone())
                .unwrap();
            retained = Some(h);
            owner
                .append_owner_incident_slot(nodes[0], Some(edges[1]))
                .unwrap()
        }
        if name == "iterator-live" {
            let h = header(&mut owner, &[Some(edges[0]), Some(edges[1])], 3);
            owner
                .set_owner_node_incident_header(nodes[0], h.clone())
                .unwrap();
            retained = Some(h.clone());
            let mut range = h.iter_slots();
            let first = range.next().unwrap();
            owner
                .write_owner_incident_capacity_slot(&h, 1, None)
                .unwrap();
            let empty = header(&mut owner, &[], 0);
            owner
                .set_owner_node_incident_header(nodes[0], empty)
                .unwrap();
            let mut values = vec![first];
            values.extend(range);
            yielded = Some(
                values
                    .into_iter()
                    .map(|edge| {
                        edge.map(|edge| edges.iter().position(|&known| known == edge).unwrap())
                    })
                    .collect::<Vec<_>>(),
            )
        }
        if name == "restore-overlap" {
            let all = header(
                &mut owner,
                &[Some(edges[0]), Some(edges[1]), Some(edges[2])],
                4,
            );
            let a = all.reslice_with_capacity(0, 2, 4).unwrap();
            let b = all.reslice_with_capacity(1, 2, 3).unwrap();
            owner
                .set_owner_node_incident_header(nodes[0], a.clone())
                .unwrap();
            owner.set_owner_node_incident_header(nodes[1], b).unwrap();
            owner
                .set_owner_graph_edge_header(OwnerGraphId(0), all.clone())
                .unwrap();
            retained = Some(a);
            let snapshot = owner.owner_topology_snapshot();
            for i in 0..4 {
                owner
                    .write_owner_incident_capacity_slot(&all, i, None)
                    .unwrap()
            }
            let changed = header(&mut owner, &[Some(edges[2])], 1);
            owner
                .set_owner_node_incident_header(nodes[0], changed)
                .unwrap();
            let changed = header(&mut owner, &[Some(edges[1])], 1);
            owner
                .set_owner_graph_edge_header(OwnerGraphId(0), changed.clone())
                .unwrap();
            replacement = Some(changed);
            owner
                .set_owner_edge_endpoint(edges[0], false, None)
                .unwrap();
            owner
                .set_owner_edge_endpoint(edges[1], true, Some(nodes[0]))
                .unwrap();
            owner.restore_owner_topology_snapshot(snapshot);
        }
        if name == "nil-endpoint" {
            owner
                .set_owner_edge_endpoint(edges[0], false, None)
                .unwrap();
            owner.set_owner_edge_endpoint(edges[1], true, None).unwrap()
        }
        let headers = [
            Some(owner.owner_node_incident_header(nodes[0]).unwrap()),
            Some(owner.owner_node_incident_header(nodes[1]).unwrap()),
            Some(owner.owner_node_incident_header(nodes[2]).unwrap()),
            Some(owner.owner_graph_edge_header(OwnerGraphId(0)).unwrap()),
            retained,
            replacement,
        ];
        let mut overlaps = Vec::new();
        for left in 0..6 {
            if let Some(a) = &headers[left] {
                for ai in 0..a.capacity() {
                    for right in left + 1..6 {
                        if let Some(b) = &headers[right] {
                            for bi in 0..b.capacity() {
                                if a.slot_address(ai) == b.slot_address(bi) {
                                    overlaps.push([left, ai, right, bi])
                                }
                            }
                        }
                    }
                }
            }
        }
        let endpoints: Vec<_> = edges
            .iter()
            .map(|&edge| {
                [false, true].map(|to| {
                    owner
                        .owner_edge_endpoint(edge, to)
                        .unwrap()
                        .map(|node| nodes.iter().position(|&known| known == node).unwrap())
                })
            })
            .collect();
        let mut guarded_eligible = Vec::new();
        let mut eligibility_used = Vec::new();
        for node in nodes {
            let mut guard = OptimizationWorkGuard::new(Some(&Quiet), "control", 100000).unwrap();
            guarded_eligible.push(
                super::super::owner_near_eligibility::can_optimize_owner_node_guarded(
                    &mut owner,
                    Some(node),
                    Some(OwnerGraphId(0)),
                    &mut guard,
                )
                .unwrap(),
            );
            eligibility_used.push(guard.used());
        }
        println!(
            "OWNER_INCIDENTS_CONTROL {}",
            json!({"case":name,"headers":headers.iter().map(|h|observe(h.as_ref(),&edges)).collect::<Vec<_>>(),"overlaps":overlaps,"endpoints":endpoints,"yielded":yielded,"incident_counts":nodes.map(|node|owner.owner_node_incident_count(node)),"eligible":guarded_eligible,"eligible_dense":nodes.map(|node|owner.can_optimize_node(node)),"eligibility_used":eligibility_used})
        );
    }
}
#[test]
fn numerical_edge_fork_preserves_graph_incident_aliases_and_rejects_parent_scope() {
    let (mut owner, nodes, edges) = fixture();
    let h = header(&mut owner, &[Some(edges[0]), None], 4);
    owner
        .set_owner_node_incident_header(nodes[0], h.clone())
        .unwrap();
    owner
        .set_owner_graph_edge_header(OwnerGraphId(0), h.clone())
        .unwrap();
    let mut fork = owner.clone();
    let local = fork.owner_node_incident_header(nodes[0]).unwrap();
    let graph = fork.owner_graph_edge_header(OwnerGraphId(0)).unwrap();
    assert_eq!(local.slot_address(0), graph.slot_address(0));
    assert_ne!(local.slot_address(0), h.slot_address(0));
    fork.write_owner_incident_capacity_slot(&local, 0, None)
        .unwrap();
    assert_eq!(graph.get(0), Some(None));
    assert_eq!(h.get(0), Some(Some(edges[0])));
    assert_eq!(
        fork.set_owner_node_incident_header(nodes[0], h),
        Err(OwnerTopologyError::ForeignEdgeScope)
    );
}
#[test]
fn nullable_reconnect_panics_before_incidence_mutation_and_nil_endpoint_restores() {
    let (mut owner, nodes, edges) = fixture();
    let h = header(&mut owner, &[Some(edges[0]), None], 3);
    owner
        .set_owner_node_incident_header(nodes[0], h.clone())
        .unwrap();
    owner
        .set_owner_edge_endpoint(edges[0], false, None)
        .unwrap();
    let snapshot = owner.owner_topology_snapshot();
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        owner.reconnect_owner_edge_endpoint(edges[0], nodes[2], false)
    }));
    assert!(panic.is_err());
    assert_eq!(h.get(0), Some(Some(edges[0])));
    assert_eq!(owner.owner_edge_endpoint(edges[0], false), Ok(None));
    owner
        .set_owner_edge_endpoint(edges[0], false, Some(nodes[2]))
        .unwrap();
    owner.restore_owner_topology_snapshot(snapshot);
    assert_eq!(owner.owner_edge_endpoint(edges[0], false), Ok(None));
}

#[test]
fn non_nil_assertion_rechecks_a_window_after_any_slot_write() {
    let slice = SharedEdgeSlice::from_values(&[EdgeId(0), EdgeId(1), EdgeId(2)]).unwrap();
    // A verified window is remembered; repeating the check stays quiet.
    slice.assert_values_non_nil();
    slice.assert_values_non_nil();
    // Writing a nil through an alias of the same backing invalidates it, and
    // the next check still reaches the raw nil-member panic.
    let alias = slice.clone();
    alias.write_capacity_slot(1, None).unwrap();
    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        slice.assert_values_non_nil();
    }));
    assert!(panicked.is_err());
    // A different window over the same backing is checked on its own.
    alias.write_capacity_slot(1, Some(EdgeId(1))).unwrap();
    slice.assert_values_non_nil();
}
