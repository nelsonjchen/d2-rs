// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
use super::super::limits::{Context, ContextError};
use super::super::owner_groups::OwnerGroupId;
#[cfg(feature = "diagnostic-traces")]
use super::super::owner_groups::OwnerGroupKind;
use super::*;
use serde_json::json;
use std::cell::Cell;

struct Observe<'a> {
    name: &'a str,
    polls: Cell<usize>,
    nodes: [NodeId; 5],
    members: OwnerMemberHeader,
    group: OwnerGroupId,
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
        if poll == 2 {
            match self.name {
                "live-future-member" => {
                    owner
                        .write_owner_member_capacity_slot(&self.members, 0, Some(self.nodes[4]))
                        .unwrap();
                    let mut f = owner.owner_group_fields(self.group).unwrap();
                    f.nodes = owner.nil_owner_member_header();
                    owner.set_owner_group_fields(self.group, f).unwrap();
                }
                "live-loaded-slot" => owner
                    .write_owner_member_capacity_slot(&self.members, 1, None)
                    .unwrap(),
                "live-graph" => owner
                    .set_owner_node_graph_reference(self.nodes[0], Some(self.foreign))
                    .unwrap(),
                "move-live-root" => owner.set_position(self.nodes[0], Point { x: 100.0, y: 200.0 }),
                "move-live-child" => owner.clear_position(self.nodes[1]),
                "cancel-before-push" => return Some(ContextError::Cancelled),
                _ => (),
            }
        }
        None
    }
}

#[test]
fn owner_movement_raw_scoring_budget_matches_source_priority() {
    use super::super::sizeless::move_owner_node_to_best;
    struct Quiet(Cell<usize>);
    impl Context for Quiet {
        fn done(&self) -> Option<bool> {
            None
        }
        fn error(&self) -> Option<ContextError> {
            self.0.set(self.0.get() + 1);
            None
        }
    }
    let mut input = Graph::default();
    let nodes = ["A", "B"].map(|name| {
        input.add_node(crate::Node::new(
            name,
            Size {
                width: 10.0,
                height: 20.0,
            },
        ))
    });
    let edge = input.add_edge(crate::Edge {
        source: nodes[0],
        target: nodes[1],
    });
    let mut owner = ArenaGraph::from_input(&input);
    owner.initialize_shared_topology();
    owner.cell_size = 10.0;
    for (index, &node) in nodes.iter().enumerate() {
        owner.set_position(
            node,
            Point {
                x: index as f64 * 2.0,
                y: index as f64 * -3.0,
            },
        );
    }
    let header = owner
        .allocate_owner_incident_header(&[Some(edge), None], 2, true)
        .unwrap();
    owner
        .set_owner_node_incident_header(nodes[0], header)
        .unwrap();
    let mapping = CanonicalScoreMapping::new(
        owner.topology_arena_id().unwrap(),
        nodes.to_vec(),
        vec![edge],
    );
    let mut workspace = owner.clone();
    let mut session = OwnerScoreSession::new(&mut owner, OwnerGraphId(0), mapping, None).unwrap();
    let context = Quiet(Cell::new(0));
    let mut guard = OptimizationWorkGuard::new(Some(&context), "control", 10).unwrap();
    // The original source control invokes moveNodeToBestGuarded on a
    // literal optimizer containing only g; it does not call its constructor.
    // Exercise the same production helper without unrelated numerical setup.
    let error = move_owner_node_to_best(
        &mut workspace,
        nodes[0],
        [Some(Point { x: 10.0, y: 20.0 })].into_iter(),
        |point| point,
        &mut session,
        &context,
        &mut guard,
    )
    .unwrap_err();
    let row = json!({"case":"raw-scoring-budget","error":error.to_string(),"used":guard.used(),"polls":context.0.get(),"positions":nodes.iter().map(|&node|session.owner().position(node).map(|p|[format!("{:016x}",p.x.to_bits()),format!("{:016x}",p.y.to_bits())])).collect::<Vec<_>>()});
    assert_eq!(guard.used(), 2);
    assert_eq!(context.0.get(), 1);
    assert_eq!(
        session.owner().position(nodes[0]),
        Some(Point { x: 0.0, y: -0.0 })
    );
    println!("OWNER_MOVEMENT_BUDGET_CONTROL {row}");
}

#[cfg(feature = "diagnostic-traces")]
#[test]
fn owner_movement_actual_source_controls() {
    for name in [
        "leaf",
        "cluster",
        "sequence",
        "both",
        "container-all",
        "cluster-no-flag",
        "flag-missing",
        "nil-member",
        "duplicates-cycle",
        "live-future-member",
        "live-loaded-slot",
        "live-graph",
        "move-live-root",
        "move-live-child",
        "cancel-before-push",
        "rollback",
    ] {
        let mut input = Graph::default();
        let nodes = ["A", "B", "C", "D", "E"].map(|name| {
            input.add_node(crate::Node::new(
                name,
                Size {
                    width: 10.0,
                    height: 20.0,
                },
            ))
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
        }
        let mapping =
            CanonicalScoreMapping::new(owner.topology_arena_id().unwrap(), nodes.to_vec(), vec![]);
        let group = owner.new_owner_group(OwnerGroupKind::Cluster);
        let sequence = owner.new_owner_group(OwnerGroupKind::Sequence);
        let slots = if matches!(name, "nil-member" | "cancel-before-push") {
            vec![Some(nodes[1]), None]
        } else if name == "duplicates-cycle" {
            vec![Some(nodes[1]), Some(nodes[2]), Some(nodes[1])]
        } else {
            vec![Some(nodes[1]), Some(nodes[2])]
        };
        let members = owner
            .allocate_owner_member_header(&slots, slots.len())
            .unwrap();
        let mut f = owner.owner_group_fields(group).unwrap();
        f.nodes = members.clone();
        owner.set_owner_group_fields(group, f).unwrap();
        let mut f = owner.owner_group_fields(sequence).unwrap();
        f.nodes = members.clone();
        owner.set_owner_group_fields(sequence, f).unwrap();
        let cluster_map = owner
            .owner_graph_group_map(OwnerGraphId(0), OwnerGroupKind::Cluster)
            .unwrap();
        let sequence_map = owner
            .owner_graph_group_map(OwnerGraphId(0), OwnerGroupKind::Sequence)
            .unwrap();
        if !matches!(name, "leaf" | "sequence" | "flag-missing") {
            owner
                .write_owner_group_map_entry(&cluster_map, Some(nodes[0]), Some(group))
                .unwrap();
        }
        if !matches!(name, "leaf" | "sequence" | "cluster-no-flag") {
            owner.set_owner_cluster_vessel_flag(nodes[0], true).unwrap();
        }
        if matches!(name, "sequence" | "both" | "container-all") {
            owner
                .write_owner_group_map_entry(&sequence_map, Some(nodes[0]), Some(sequence))
                .unwrap();
            if name != "sequence" {
                let mut f = owner.owner_group_fields(group).unwrap();
                f.nodes = owner
                    .allocate_owner_member_header(&[Some(nodes[1]), Some(nodes[3])], 2)
                    .unwrap();
                owner.set_owner_group_fields(group, f).unwrap();
            }
        }
        if name == "container-all" {
            owner.nodes[nodes[0].0 as usize].is_container = true;
            owner
                .containers
                .insert(Some(nodes[0]), vec![nodes[2], nodes[4]]);
        }
        if name == "duplicates-cycle" {
            let nested = owner.new_owner_group(OwnerGroupKind::Sequence);
            let mut f = owner.owner_group_fields(nested).unwrap();
            f.nodes = owner
                .allocate_owner_member_header(&[Some(nodes[0]), Some(nodes[1])], 2)
                .unwrap();
            owner.set_owner_group_fields(nested, f).unwrap();
            owner
                .write_owner_group_map_entry(&sequence_map, Some(nodes[2]), Some(nested))
                .unwrap();
        }
        if name == "live-graph" {
            let nested = owner.new_owner_group(OwnerGroupKind::Cluster);
            let mut f = owner.owner_group_fields(nested).unwrap();
            f.nodes = owner
                .allocate_owner_member_header(&[Some(nodes[3])], 1)
                .unwrap();
            owner.set_owner_group_fields(nested, f).unwrap();
            owner.set_owner_cluster_vessel_flag(nodes[2], true).unwrap();
            owner
                .write_owner_group_map_entry(&cluster_map, Some(nodes[2]), Some(nested))
                .unwrap();
        }
        let foreign = owner.create_owner_graph_view(vec![], vec![]).unwrap();
        let context = Observe {
            name,
            polls: Cell::new(0),
            nodes,
            members,
            group,
            foreign,
        };
        let mut guard =
            OptimizationWorkGuard::new_with_owner(Some(&context), "control", 100000, &mut owner)
                .unwrap();
        if matches!(
            name,
            "live-future-member" | "live-loaded-slot" | "live-graph" | "cancel-before-push"
        ) {
            guard.add_with_owner(63, &mut owner).unwrap();
        }
        let mut session =
            OwnerScoreSession::new(&mut owner, OwnerGraphId(0), mapping, None).unwrap();
        let mut row =
            json!({"case":name,"ids":null,"descendant_work":null,"capture_used":null,"error":null});
        {
            let capture = session.optimizer_candidate_movement(nodes[0], &mut guard);
            match capture {
                Err(error) => {
                    row["error"] = json!(error.to_string());
                    row["capture_used"] = json!(guard.used());
                }
                Ok(mut movement) => {
                    row["ids"] = json!(
                        movement
                            .captured_nodes()
                            .map(|node| node.0 as usize + 1)
                            .collect::<Vec<_>>()
                    );
                    row["descendant_work"] = json!(movement.descendant_work());
                    row["capture_used"] = json!(guard.used());
                    if matches!(name, "move-live-root" | "move-live-child") {
                        guard
                            .add_with_owner(56, movement.session().owner_mut())
                            .unwrap();
                    }
                    let result = movement.move_abs(Point { x: 10.0, y: 20.0 }, &mut guard);
                    if let Err(error) = result {
                        row["error"] = json!(error.to_string());
                    } else if name != "rollback" {
                        movement.commit();
                    }
                }
            }
        }
        row["used"] = json!(guard.used());
        row["polls"] = json!(context.polls.get());
        row["positions"] = json!(
            nodes
                .iter()
                .map(|&node| session.owner().position(node).map(|p| [
                    format!("{:016x}", p.x.to_bits()),
                    format!("{:016x}", p.y.to_bits())
                ]))
                .collect::<Vec<_>>()
        );
        row["graphs"] = json!(
            nodes
                .iter()
                .map(|&node| session.owner().topology_node_owner(node).map(|g| g.0))
                .collect::<Vec<_>>()
        );
        println!("OWNER_MOVEMENT_CONTROL {row}");
    }
}
