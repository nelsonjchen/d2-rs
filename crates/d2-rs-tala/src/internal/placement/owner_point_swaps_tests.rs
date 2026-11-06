// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
use super::super::limits::{Context, ContextError};
#[cfg(feature = "diagnostic-traces")]
use super::super::node_near_map::NodeNearMap;
use super::super::occupied::OccupiedPointMap;
#[cfg(feature = "diagnostic-traces")]
use super::super::owner_groups::OwnerGroupKind;
#[cfg(feature = "diagnostic-traces")]
use super::super::shared_topology::OwnerGraphId;
use super::super::shared_topology::OwnerNodePoint;
#[cfg(feature = "diagnostic-traces")]
use super::super::{CanonicalScoreMapping, OwnerScoreSession};
use super::*;
#[cfg(feature = "diagnostic-traces")]
use crate::Edge;
use crate::{Graph, Node, Size};
#[cfg(feature = "diagnostic-traces")]
use serde_json::json;
use std::cell::{Cell, RefCell};
struct Quiet;
impl Context for Quiet {
    fn done(&self) -> Option<bool> {
        None
    }
    fn error(&self) -> Option<ContextError> {
        None
    }
}
struct Observe<'a> {
    name: &'a str,
    polls: Cell<usize>,
    root: NodeId,
    tl: OwnerNodePoint,
    occupied: &'a RefCell<OccupiedPointMap>,
    last: NodeId,
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
        let p = self.polls.get() + 1;
        self.polls.set(p);
        if p == 2 {
            match self.name {
                "live-in-place" => owner
                    .write_owner_point(&self.tl, Point { x: 10.0, y: 20.0 })
                    .unwrap(),
                "live-replace" => owner.set_position(self.root, Point { x: 10.0, y: 20.0 }),
                "live-nil-field" => owner.clear_position(self.root),
                "occupied-live" => {
                    let map = OccupiedPointMap::new();
                    let p = self.tl.get().unwrap();
                    map.insert(
                        Point {
                            x: p.x,
                            y: p.y - 1.0,
                        },
                        Some(self.last),
                    );
                    *self.occupied.borrow_mut() = map;
                }
                "cancel-once" => return Some(ContextError::Cancelled),
                _ => (),
            }
        }
        None
    }
}
fn bits(point: Option<Point>) -> Option<[String; 2]> {
    point.map(|p| {
        [
            format!("{:016x}", p.x.to_bits()),
            format!("{:016x}", p.y.to_bits()),
        ]
    })
}
#[cfg(feature = "diagnostic-traces")]
#[test]
fn owner_point_swap_actual_source_controls() {
    for name in [
        "compass",
        "missing-cell",
        "duplicate",
        "present-nil",
        "last-error",
        "fixed",
        "tree-excluded",
        "foreign-node-owner",
        "zero-edges",
        "near-eligible",
        "live-in-place",
        "live-replace",
        "live-nil-field",
        "occupied-live",
        "cancel-once",
        "nil-before-limit",
        "alias-restore",
        "alias-move",
        "alias-move-rollback",
    ] {
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
        let edges = [1, 2, 3, 4].map(|i| {
            input.add_edge(Edge {
                source: nodes[0],
                target: nodes[i],
            })
        });
        let mut owner = ArenaGraph::from_input(&input);
        owner.initialize_shared_topology();
        owner.cell_size = 10.0;
        for (index, &node) in nodes.iter().enumerate() {
            owner.nodes[node.0 as usize].tala_id = index as i64 + 1;
            let slots = if index == 0 {
                edges.iter().copied().map(Some).collect::<Vec<_>>()
            } else {
                vec![Some(edges[index - 1])]
            };
            let header = owner
                .allocate_owner_incident_header(&slots, slots.len(), true)
                .unwrap();
            owner.set_owner_node_incident_header(node, header).unwrap();
        }
        let center = Point { x: 0.25, y: -0.5 };
        let coords = [
            center,
            Point {
                x: center.x,
                y: center.y - 1.0,
            },
            Point {
                x: center.x,
                y: center.y + 1.0,
            },
            Point {
                x: center.x - 1.0,
                y: center.y,
            },
            Point {
                x: center.x + 1.0,
                y: center.y,
            },
        ];
        for (&node, &point) in nodes.iter().zip(&coords) {
            let header = owner.allocate_owner_node_point(Some(point));
            owner.bind_owner_node_point(node, header).unwrap();
        }
        let retained = owner.owner_node_point(nodes[0]).unwrap();
        let optimizer =
            OwnerSizelessOptimizer::new(&mut owner, Some(OwnerGraphId(0)), None, Some(&Quiet))
                .unwrap();
        if name == "missing-cell" {
            optimizer.occupied().borrow().delete(coords[1]);
            optimizer.occupied().borrow().delete(coords[2]);
        }
        if name == "duplicate" {
            for &point in &coords[1..] {
                optimizer.occupied().borrow().insert(point, Some(nodes[1]));
            }
        }
        if matches!(name, "present-nil" | "last-error") {
            optimizer.occupied().borrow().insert(coords[1], None);
        }
        if name == "last-error" {
            let empty = owner.allocate_owner_incident_header(&[], 0, false).unwrap();
            owner
                .set_owner_node_incident_header(nodes[3], empty)
                .unwrap();
            owner.nodes[nodes[3].0 as usize].nears = NodeNearMap::from_raw_keys([Some(nodes[4])]);
            owner.nodes[nodes[3].0 as usize].container = Some(nodes[0]);
            owner.nodes[nodes[4].0 as usize].container = Some(nodes[4]);
        }
        if name == "fixed" {
            owner.nodes[nodes[1].0 as usize].fixed_top_left = Some(Point::default());
        }
        if matches!(name, "tree-excluded" | "foreign-node-owner") {
            let map = owner.allocate_owner_node_to_tree_map(true);
            owner
                .write_owner_node_to_tree_entry(&map, Some(nodes[1]), None)
                .unwrap();
            owner
                .set_owner_graph_node_to_tree_map(OwnerGraphId(0), map)
                .unwrap();
        }
        if name == "foreign-node-owner" {
            let foreign = owner.create_owner_graph_view(vec![], vec![]).unwrap();
            owner
                .set_owner_node_graph_reference(nodes[1], Some(foreign))
                .unwrap();
        }
        if matches!(name, "zero-edges" | "near-eligible") {
            let h = owner.allocate_owner_incident_header(&[], 0, false).unwrap();
            owner.set_owner_node_incident_header(nodes[1], h).unwrap();
            if name == "near-eligible" {
                owner.nodes[nodes[1].0 as usize].nears =
                    NodeNearMap::from_raw_keys([Some(nodes[0])]);
            }
        }
        if name == "nil-before-limit" {
            owner.clear_position(nodes[0]);
        }
        let context = Observe {
            name,
            polls: Cell::new(0),
            root: nodes[0],
            tl: retained.clone(),
            occupied: optimizer.occupied(),
            last: nodes[4],
        };
        let mut guard = OptimizationWorkGuard::new_with_owner(
            Some(&context),
            "control",
            if name == "nil-before-limit" {
                0
            } else {
                100000
            },
            &mut owner,
        )
        .unwrap();
        if matches!(
            name,
            "live-in-place" | "live-replace" | "live-nil-field" | "occupied-live" | "cancel-once"
        ) {
            guard.add_with_owner(63, &mut owner).unwrap();
        }
        let mut row =
            json!({"case":name,"ids":null,"nil":null,"cap":null,"error":null,"replacement":null});
        if name == "alias-restore" {
            owner
                .bind_owner_node_point(nodes[1], retained.clone())
                .unwrap();
            let saved = retained.snapshot();
            owner
                .write_owner_point(&retained, Point { x: 9.0, y: 9.0 })
                .unwrap();
            let replacement = owner.allocate_owner_node_point(Some(Point { x: 7.0, y: 8.0 }));
            owner
                .bind_owner_node_point(nodes[0], replacement.clone())
                .unwrap();
            owner.restore_owner_node_point(nodes[0], &saved).unwrap();
            row["replacement"] = json!(bits(replacement.get()));
        } else if matches!(name, "alias-move" | "alias-move-rollback") {
            owner
                .bind_owner_node_point(nodes[1], retained.clone())
                .unwrap();
            let group = owner.new_owner_group(OwnerGroupKind::Cluster);
            let mut f = owner.owner_group_fields(group).unwrap();
            f.nodes = owner
                .allocate_owner_member_header(&[Some(nodes[1])], 1)
                .unwrap();
            owner.set_owner_group_fields(group, f).unwrap();
            owner.set_owner_cluster_vessel_flag(nodes[0], true).unwrap();
            let map = owner
                .owner_graph_group_map(OwnerGraphId(0), OwnerGroupKind::Cluster)
                .unwrap();
            owner
                .write_owner_group_map_entry(&map, Some(nodes[0]), Some(group))
                .unwrap();
            let mapping = CanonicalScoreMapping::new(
                owner.topology_arena_id().unwrap(),
                nodes.to_vec(),
                edges.to_vec(),
            );
            {
                let mut session =
                    OwnerScoreSession::new(&mut owner, OwnerGraphId(0), mapping, None).unwrap();
                let mut movement = session
                    .optimizer_candidate_movement(nodes[0], &mut guard)
                    .unwrap();
                movement
                    .move_abs(Point { x: 10.0, y: 20.0 }, &mut guard)
                    .unwrap();
                if name == "alias-move" {
                    movement.commit();
                }
            }
        } else {
            match optimizer.swap_candidates_guarded(&mut owner, Some(nodes[0]), &mut guard) {
                Ok(result) => {
                    row["ids"] = json!(
                        result
                            .require_nodes()
                            .map(|n| owner.nodes[n.0 as usize].tala_id)
                            .collect::<Vec<_>>()
                    );
                    row["nil"] = json!(result.is_nil());
                    row["cap"] = json!(result.capacity());
                }
                Err(error) => {
                    row["error"] = json!(error.to_string());
                    row["nil"] = json!(true);
                    row["cap"] = json!(0);
                }
            }
        }
        row["used"] = json!(guard.used());
        row["polls"] = json!(context.polls.get());
        row["points"] = json!(
            nodes
                .iter()
                .map(|&n| bits(owner.position(n)))
                .collect::<Vec<_>>()
        );
        row["retained"] = json!(bits(retained.get()));
        row["same_retained"] = json!(
            owner
                .owner_node_point(nodes[0])
                .unwrap()
                .same_allocation(&retained)
        );
        row["alias_ab"] = json!(
            owner
                .owner_node_point(nodes[0])
                .unwrap()
                .same_allocation(&owner.owner_node_point(nodes[1]).unwrap())
        );
        println!("OWNER_POINT_SWAPS_CONTROL {row}");
    }
}

#[test]
fn numerical_point_copy_preserves_alias_classes_and_rejects_parent_scope() {
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
    let p = owner.allocate_owner_node_point(Some(Point { x: 1.0, y: 2.0 }));
    owner.bind_owner_node_point(a, p.clone()).unwrap();
    owner.bind_owner_node_point(b, p.clone()).unwrap();
    let mut copy = owner.clone();
    let ca = copy.owner_node_point(a).unwrap();
    let cb = copy.owner_node_point(b).unwrap();
    assert!(ca.same_allocation(&cb));
    assert!(!ca.same_allocation(&p));
    copy.write_owner_point(&ca, Point { x: 3.0, y: 4.0 })
        .unwrap();
    assert_eq!(copy.position(b), Some(Point { x: 3.0, y: 4.0 }));
    assert_eq!(owner.position(a), Some(Point { x: 1.0, y: 2.0 }));
    assert!(matches!(
        copy.bind_owner_node_point(a, p),
        Err(OwnerTopologyError::ForeignPointScope)
    ));
}
