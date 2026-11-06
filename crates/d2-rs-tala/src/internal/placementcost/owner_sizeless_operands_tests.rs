// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
use super::*;
use crate::internal::limits::{Context, ContextError};
use crate::internal::node_near_map::NodeNearMap;
use crate::internal::owner_groups::{OwnerClusterPolicy, OwnerGroupKind};
use serde_json::{Value, json};
use std::cell::Cell;
use std::panic::{AssertUnwindSafe, catch_unwind};
struct Observe {
    polls: Cell<u64>,
    cancel: u64,
}
impl Context for Observe {
    fn done(&self) -> Option<bool> {
        None
    }
    fn error(&self) -> Option<ContextError> {
        let poll = self.polls.get() + 1;
        self.polls.set(poll);
        (poll == self.cancel).then_some(ContextError::Cancelled)
    }
}
fn fixture(name: &str) -> (ArenaGraph, ArenaGraph, Vec<NodeId>, Vec<EdgeId>) {
    let mut input = Graph::default();
    let nodes = ["A", "B", "C", "V", "E"]
        .map(|id| {
            input.add_node(crate::Node::new(
                id,
                Size {
                    width: 10.,
                    height: 20.,
                },
            ))
        })
        .to_vec();
    let edge = input.add_edge(crate::Edge {
        source: nodes[0],
        target: nodes[1],
    });
    let other = input.add_edge(crate::Edge {
        source: nodes[4],
        target: nodes[2],
    });
    input.set_edge_arrows(
        edge,
        crate::EdgeArrows {
            source: false,
            target: true,
        },
    );
    let edges = vec![edge, other];
    let mut owner = ArenaGraph::from_input(&input);
    owner.initialize_shared_topology();
    owner.directions.clear();
    for (i, p) in [
        Point { x: 0., y: 0. },
        Point { x: 3., y: 4. },
        Point { x: -2., y: 0. },
        Point { x: 20., y: 10. },
        Point { x: 70., y: 90. },
    ]
    .into_iter()
    .enumerate()
    {
        owner.nodes[i].tala_id = i as i64 + 1;
        let header = owner.allocate_owner_node_point(Some(p));
        owner.bind_owner_node_point(nodes[i], header).unwrap();
    }
    let mut workspace = owner.clone(); // Value carrier geometry never supplies operands.
    let receiver = if name.starts_with("foreign-") {
        let graph = owner
            .create_owner_graph_view(vec![nodes[0], nodes[1]], edges.clone())
            .unwrap();
        owner
            .set_owner_node_graph_reference(nodes[0], Some(graph))
            .unwrap();
        graph
    } else {
        OwnerGraphId(0)
    };
    let degree = if name == "duplicate-65" {
        65
    } else if name == "duplicate-129" {
        129
    } else {
        1
    };
    let mut slots = vec![Some(edge); degree];
    if name == "nil-edge" {
        slots = vec![None]
    }
    if name == "unrelated-edge" {
        slots = vec![Some(other)]
    }
    if name == "near-nil-root" {
        slots.clear();
        owner.clear_position(nodes[0]);
    }
    let header = owner
        .allocate_owner_incident_header(&slots, slots.len() + 2, true)
        .unwrap();
    owner
        .set_owner_node_incident_header(nodes[0], header)
        .unwrap();
    if name == "nil-from" {
        owner.set_owner_edge_endpoint(edge, false, None).unwrap()
    }
    if name == "nil-to" {
        owner.set_owner_edge_endpoint(edge, true, None).unwrap()
    }
    if name == "nil-graph" {
        owner
            .set_owner_node_graph_reference(nodes[0], None)
            .unwrap()
    }
    let grouped = name.starts_with("cluster-")
        || name == "both-clusters"
        || name == "foreign-cluster"
        || name == "nil-root-cluster-no-direction"
        || name.starts_with("cancel-");
    if grouped {
        let group = owner.new_owner_group(OwnerGroupKind::Cluster);
        let mut fields = owner.owner_group_fields(group).unwrap();
        fields.vessel = if name == "cluster-nil-vessel" {
            None
        } else {
            Some(nodes[3])
        };
        fields.graph = Some(receiver);
        owner.set_owner_group_fields(group, fields).unwrap();
        owner
            .set_owner_node_group(
                if name == "cluster-b" {
                    nodes[1]
                } else {
                    nodes[0]
                },
                OwnerGroupKind::Cluster,
                Some(group),
            )
            .unwrap();
        let map = owner
            .owner_graph_group_map(receiver, OwnerGroupKind::Cluster)
            .unwrap();
        owner
            .write_owner_group_map_entry(&map, Some(nodes[3]), Some(group))
            .unwrap();
        let length = if name == "cluster-abductions-129" || name.starts_with("cancel-") {
            129
        } else {
            0
        };
        let abductions = owner
            .allocate_owner_abduction_header(&vec![None; length], length, true)
            .unwrap();
        owner
            .set_owner_cluster_policy(
                group,
                OwnerClusterPolicy {
                    edge_abductions: Some(abductions),
                    ..Default::default()
                },
            )
            .unwrap();
        if name == "both-clusters" {
            let second = owner.new_owner_group(OwnerGroupKind::Cluster);
            let mut fields = owner.owner_group_fields(second).unwrap();
            fields.vessel = Some(nodes[4]);
            owner.set_owner_group_fields(second, fields).unwrap();
            owner
                .set_owner_node_group(nodes[1], OwnerGroupKind::Cluster, Some(second))
                .unwrap();
            owner
                .write_owner_group_map_entry(&map, Some(nodes[4]), Some(second))
                .unwrap();
        }
        if name == "nil-root-cluster-no-direction" {
            owner.clear_position(nodes[0]);
        }
    }
    if name.starts_with("vessel-") || name == "specified-vessel" {
        owner.set_owner_cluster_vessel_flag(nodes[0], true).unwrap();
        if name == "specified-vessel" {
            owner
                .set_owner_graph_direction_entry(OwnerGraphId(0), None, Some(Orientation::Bottom))
                .unwrap();
        }
        if name != "vessel-missing" {
            let group = owner.new_owner_group(OwnerGroupKind::Cluster);
            let mut fields = owner.owner_group_fields(group).unwrap();
            fields.vessel = Some(nodes[0]);
            owner.set_owner_group_fields(group, fields).unwrap();
            owner
                .set_owner_cluster_policy(
                    group,
                    OwnerClusterPolicy {
                        arrangement: match name {
                            "vessel-row" => "Row",
                            "vessel-column" => "Column",
                            _ => "Other",
                        }
                        .into(),
                        ..Default::default()
                    },
                )
                .unwrap();
            let map = owner
                .owner_graph_group_map(receiver, OwnerGroupKind::Cluster)
                .unwrap();
            owner
                .write_owner_group_map_entry(&map, Some(nodes[0]), Some(group))
                .unwrap();
        }
    }
    if name.starts_with("nil-map-key") || name == "nil-group-value" {
        let map = owner
            .owner_graph_group_map(receiver, OwnerGroupKind::Cluster)
            .unwrap();
        let group = owner.new_owner_group(OwnerGroupKind::Cluster);
        owner
            .write_owner_group_map_entry(
                &map,
                if name == "nil-group-value" {
                    Some(nodes[3])
                } else {
                    None
                },
                if name == "nil-group-value" {
                    None
                } else {
                    Some(group)
                },
            )
            .unwrap();
        if name == "nil-map-key-sort" {
            owner
                .write_owner_group_map_entry(&map, Some(nodes[3]), Some(group))
                .unwrap();
        }
    }
    let near = match name {
        "near-one" | "near-nil-root" => Some(vec![Some(nodes[1])]),
        "near-many" => Some(vec![
            Some(nodes[1]),
            Some(nodes[2]),
            Some(nodes[3]),
            Some(nodes[4]),
        ]),
        "near-nil-key" => Some(vec![None]),
        "near-nil-position" => {
            owner.clear_position(nodes[2]);
            Some(vec![Some(nodes[2])])
        }
        _ => None,
    };
    if let Some(keys) = near {
        owner
            .set_owner_node_near_map(nodes[0], NodeNearMap::from_raw_keys(keys))
            .unwrap();
    }
    if name.starts_with("uncle-") || name == "foreign-uncle" {
        let keys = match name {
            "uncle-empty" => vec![],
            "uncle-two" => vec![Some(nodes[0]), Some(nodes[1])],
            "uncle-duplicates" => vec![Some(nodes[0]), Some(nodes[0]), Some(nodes[1])],
            "uncle-nil-slot" => vec![None, Some(nodes[0]), Some(nodes[1])],
            "uncle-len1-nil" => vec![None],
            _ => vec![Some(nodes[0]), Some(nodes[1]), Some(nodes[3])],
        };
        let header = owner
            .allocate_owner_member_header(&keys, keys.len() + 2)
            .unwrap();
        owner
            .replace_owner_common_uncle_map(receiver, Some(SharedSliceMap::new()))
            .unwrap();
        owner
            .set_owner_common_uncle_members(receiver, Some(nodes[0]), header)
            .unwrap();
        if name == "uncle-nil-position" {
            owner.clear_position(nodes[3]);
        }
        if name == "uncle-nan-width" {
            for i in [0, 1, 3] {
                owner.nodes[i].rect.size.width = f64::from_bits(0x7ff8000000000001);
            }
        }
        if name == "foreign-uncle" {
            owner
                .common_uncle_siblings
                .replace_header(SharedSliceMap::new());
            owner
                .common_uncle_siblings
                .insert(nodes[0], vec![nodes[0], nodes[1]]);
        }
    }
    if name == "undirected" {
        owner.edge_records[0].target_arrow = false;
        workspace.edge_records[0].target_arrow = false;
    }
    (owner, workspace, nodes, edges)
}
#[test]
fn complete_sizeless_operand_source_controls() {
    let mut cases = [
        "ordinary",
        "duplicate-65",
        "duplicate-129",
        "nil-edge",
        "unrelated-edge",
        "nil-from",
        "nil-to",
        "nil-graph",
        "cluster-a",
        "cluster-b",
        "both-clusters",
        "cluster-nil-vessel",
        "cluster-abductions-129",
        "vessel-row",
        "vessel-column",
        "vessel-unknown",
        "vessel-missing",
        "specified-vessel",
        "nil-group-value",
        "nil-map-key",
        "nil-map-key-sort",
        "near-one",
        "near-many",
        "near-nil-key",
        "near-nil-position",
        "near-nil-root",
        "uncle-empty",
        "uncle-two",
        "uncle-three",
        "uncle-duplicates",
        "uncle-nil-slot",
        "uncle-len1-nil",
        "uncle-nil-position",
        "uncle-nan-width",
        "foreign-cluster",
        "foreign-uncle",
        "undirected",
        "nil-root-cluster-no-direction",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<Vec<_>>();
    cases.extend((1..=16).map(|n| format!("cancel-{n}")));
    for name in cases {
        for cached in [false, true] {
            let (mut owner, workspace, nodes, edges) = fixture(&name);
            let mut scorer = OwnerSizelessEdgeLengthScorer::new(
                nodes[0],
                name != "nil-root-cluster-no-direction",
            );
            for epoch in 0..if cached { 2 } else { 1 } {
                if epoch == 1 {
                    owner.set_position(nodes[0], Point { x: 1., y: 2. });
                    owner.set_position(nodes[1], Point { x: -4., y: 8. });
                    owner.set_position(nodes[3], Point { x: -10., y: 5. });
                }
                let ctx = Observe {
                    polls: Cell::new(0),
                    cancel: name
                        .strip_prefix("cancel-")
                        .and_then(|n| n.parse().ok())
                        .unwrap_or(0),
                };
                let frame = OwnerScoreFrame::new(
                    &owner,
                    owner.topology_arena_id().unwrap(),
                    OwnerGraphId(0),
                    &nodes,
                )
                .unwrap()
                .with_edge_mapping(&edges)
                .unwrap();
                let result = catch_unwind(AssertUnwindSafe(|| {
                    scorer.score(&workspace, &frame, ScoreContext::new(&ctx))
                }));
                let (mut bits, mut error, mut panic) = (Value::Null, Value::Null, Value::Null);
                match result {
                    Ok(Ok(v)) => bits = json!(format!("{:016x}", v.to_bits())),
                    Ok(Err(e)) => error = json!(e.to_string()),
                    Err(p) => {
                        panic = json!(
                            p.downcast_ref::<&str>()
                                .copied()
                                .or_else(|| p.downcast_ref::<String>().map(String::as_str))
                                .unwrap_or("unknown")
                        )
                    }
                };
                println!(
                    "OWNER_SIZELESS_OPERANDS_CONTROL {}",
                    json!({"case":format!("{name}-{}-{epoch}",if cached{"cache"}else{"fresh"}),"bits":bits,"error":error,"panic":panic,"polls":ctx.polls.get()})
                );
                if !panic.is_null() {
                    break;
                }
            }
            scorer.close();
        }
    }
}
