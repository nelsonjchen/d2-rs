// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Construction, solution, and copy-back of recursive placement scopes.
//!
//! Child graphs project stable arena entities through containers and aggregate
//! carriers, then publish positions and lifecycle order back to their owner.

use super::model::{ProjectedTransactionEdge, ProjectedTransactionNode};
use super::*;

#[derive(Clone, Copy, Debug)]
enum PlacementAggregate {
    Sequence(usize),
    Cluster(usize),
}

/// Canonical owner operations paired with a materialized placement scope.
/// The compact scope keeps local indices; these handles stay in its owner.
#[derive(Clone)]
pub(super) struct PreparedOwnerScope {
    pub(super) abductions: OwnerAbductionSlice,
    pub(super) view: OwnerGraphId,
    pub(super) ownership_journal: OwnerGraphOwnershipJournal,
    // Read-only input to the dense projection builder. Actual endpoints and
    // incidents remain abducted on the owner throughout child recursion.
    pub(super) construction_endpoints: BTreeMap<EdgeId, (NodeId, NodeId)>,
    pub(super) construction_incidents: BTreeMap<NodeId, Vec<EdgeId>>,
}

#[cfg(test)]
mod owner_initialization_publication_tests {
    use super::*;

    #[test]
    fn combined_root_preserves_nested_cluster_live_point_and_publishes_direct_vessel() {
        for nested in [false, true] {
            let mut input = Graph::default();
            let container = input.add_node(crate::Node::new(
                "container",
                Size {
                    width: 300.0,
                    height: 300.0,
                },
            ));
            let mut member = crate::Node::new(
                "first",
                Size {
                    width: 30.0,
                    height: 10.0,
                },
            );
            member.parent = Some(container);
            let first = input.add_node(member.clone());
            member.external_id = "second".to_owned();
            let second = input.add_node(member);
            let mut pipeline = Pipeline::new(&input, 1, false, false);
            pipeline.graph.clusters.push(ClusterState {
                members: vec![first, second],
                arrangement: ClusterArrangement::Row,
                desired_arrangement: ClusterArrangement::Row,
                padding: 20.0,
                vessel_tala_id: 10001,
                fixed_size: false,
            });
            let mut entity = pipeline.graph.nodes[first.0 as usize].clone();
            entity.tala_id = 10001;
            entity.position = Some(Point {
                x: 120.25,
                y: 230.75,
            });
            let vessel = pipeline.graph.retain_topology_node(entity);
            pipeline
                .graph
                .bind_topology_vessel(TopologyVessel::Cluster(0), vessel)
                .unwrap();
            pipeline.graph.materialize_owner_node_point(vessel).unwrap();
            let retained = pipeline.graph.owner_node_point(vessel).unwrap();
            let live = retained.get().unwrap();
            let local = Point { x: 10.0, y: 20.0 };
            pipeline
                .graph
                .pending_cluster_vessel_positions
                .insert(0, local);
            if nested {
                pipeline
                    .graph
                    .set_position(container, Point { x: 100.0, y: 200.0 });
            }
            let mut stale = pipeline.graph.nodes[vessel.0 as usize].clone();
            stale.position = Some(local);
            pipeline.publish_combined_root_projection(&stale);
            assert!(retained.same_allocation(&pipeline.graph.owner_node_point(vessel).unwrap()));
            assert_eq!(
                pipeline.graph.position(vessel),
                Some(if nested { live } else { local })
            );
            assert_eq!(retained.get(), Some(if nested { live } else { local }));
            if nested {
                assert_eq!(pipeline.graph.pending_cluster_vessel_positions[&0], live);
                let second_point = Point {
                    x: live.x + 50.0,
                    y: live.y,
                };
                pipeline.graph.set_position(first, live);
                pipeline.graph.set_position(second, second_point);
                let members = BTreeMap::from([
                    (pipeline.graph.nodes[first.0 as usize].tala_id, live),
                    (
                        pipeline.graph.nodes[second.0 as usize].tala_id,
                        second_point,
                    ),
                ]);
                let parent_tala_id = pipeline.graph.nodes[container.0 as usize].tala_id;
                // Replay a stale local offset after final parent movement.
                // Both member boxes and the retained actual vessel stay live.
                pipeline.publish_external_shared_child_offsets_mode(
                    &[(parent_tala_id, 10001, local)],
                    true,
                    Some(&members),
                );
                assert_eq!(pipeline.graph.pending_cluster_vessel_positions[&0], live);
                assert_eq!(pipeline.graph.position(vessel), Some(live));
                assert_eq!(retained.get(), Some(live));
                assert_eq!(pipeline.graph.position(first), Some(live));
                assert_eq!(pipeline.graph.position(second), Some(second_point));
            }
        }
    }

    #[test]
    fn induced_copyback_updates_retained_owner_point_for_each_external_inventory() {
        for inventory in 0..3 {
            let mut input = Graph::default();
            let node = input.add_node(crate::Node::new(
                "child",
                Size {
                    width: 70.0,
                    height: 30.0,
                },
            ));
            let mut owner = ArenaGraph::from_input(&input);
            owner.initialize_shared_topology();
            owner.set_position(node, Point { x: -99.0, y: 198.0 });
            owner.materialize_owner_node_point(node).unwrap();
            let retained = owner.owner_node_point(node).unwrap();
            let mut projected = owner.nodes[node.0 as usize].clone();
            projected.position = Some(Point { x: 60.0, y: 60.0 });
            projected.rect.origin = projected.position.unwrap();
            let mut placed = ArenaGraph::from_input(&Graph::default());
            match inventory {
                0 => placed.transaction_external_containers.push(projected),
                1 => {
                    owner
                        .transaction_external_container_children
                        .insert(999, vec![owner.nodes[node.0 as usize].clone()]);
                    placed
                        .transaction_external_container_children
                        .insert(999, vec![projected]);
                }
                _ => {
                    placed
                        .transaction_external_aggregate_children
                        .insert(999, vec![projected]);
                }
            }
            Pipeline::publish_induced_external_geometry(&mut owner, &placed);
            let live = owner.owner_node_point(node).unwrap();
            assert!(retained.same_allocation(&live));
            assert_eq!(retained.get(), Some(Point { x: 60.0, y: 60.0 }));
            assert_eq!(owner.position(node), retained.get());
            assert_eq!(owner.nodes[node.0 as usize].position, retained.get());
        }
    }

    #[test]
    fn nested_sequence_is_allocated_before_parent_endpoint_abduction() {
        let mut input = Graph::default();
        let [container, a, b, peer] = ["box", "box.a", "box.b", "peer"].map(|name| {
            input.add_node(crate::Node::new(
                name,
                Size {
                    width: 70.0,
                    height: 10.0,
                },
            ))
        });
        for member in [a, b] {
            input.node_mut(member).unwrap().parent = Some(container);
        }
        let outgoing = input.add_edge(crate::Edge {
            source: a,
            target: peer,
        });
        let incoming = input.add_edge(crate::Edge {
            source: peer,
            target: b,
        });
        let mut pipeline = Pipeline::new_untraced(&input, 1, false, true);
        pipeline.graph.sequences.push(SequenceState {
            members: vec![a, b],
            vessel_tala_id: 10001,
            container: Some(container),
            has_edge_abductions: true,
        });
        for member in [a, b] {
            pipeline.graph.nodes[member.0 as usize].sequence = Some(0);
        }
        pipeline.graph.rebuild_active_aggregate_node_order();
        let (_, parent) = pipeline
            .prepare_placement_scope_after(None, &BTreeSet::new())
            .unwrap()
            .unwrap();
        assert!(
            parent.is_some(),
            "nested groups must not disable parent owner abduction"
        );
        let parent = parent.unwrap();
        let originals = parent
            .abductions
            .records
            .iter()
            .map(|id| {
                let record = pipeline.graph.owner_edge_abduction(*id).unwrap();
                (record.originally_from, record.originally_to)
            })
            .collect::<Vec<_>>();
        assert_eq!(originals, vec![(Some(a), None), (None, Some(b))]);
        let vessel = pipeline
            .graph
            .topology_vessel_node(TopologyVessel::Sequence(0))
            .unwrap();
        assert_eq!(
            pipeline.graph.edge_records[outgoing.0 as usize].from,
            container
        );
        assert_eq!(
            pipeline.graph.edge_records[incoming.0 as usize].to,
            container
        );
        pipeline
            .graph
            .materialize_owner_node_incident_header(vessel)
            .unwrap();
        assert_eq!(
            pipeline
                .graph
                .owner_node_incident_header(vessel)
                .unwrap()
                .len(),
            0
        );
        let (_, child) = pipeline
            .prepare_placement_scope_after(Some(container), &BTreeSet::new())
            .unwrap()
            .unwrap();
        assert!(child.is_some());
        assert_eq!(
            pipeline.graph.edge_records[outgoing.0 as usize].from,
            container
        );
        assert_eq!(
            pipeline.graph.edge_records[incoming.0 as usize].to,
            container
        );
        assert_eq!(
            pipeline
                .graph
                .owner_node_incident_header(vessel)
                .unwrap()
                .len(),
            0
        );
        pipeline
            .graph
            .set_position(vessel, Point { x: -99.0, y: 198.0 });
        pipeline.graph.materialize_owner_node_point(vessel).unwrap();
        let retained = pipeline.graph.owner_node_point(vessel).unwrap();
        let mut projected_input = Graph::default();
        let local = projected_input.add_node(crate::Node::new(
            "vessel",
            Size {
                width: 105.0,
                height: 10.0,
            },
        ));
        projected_input.tala_id_overrides[local.0 as usize] = Some(10001);
        let mut placed = ArenaGraph::from_input(&projected_input);
        placed.set_position(local, Point { x: 60.0, y: 60.0 });
        Pipeline::publish_induced_external_geometry(&mut pipeline.graph, &placed);
        assert!(retained.same_allocation(&pipeline.graph.owner_node_point(vessel).unwrap()));
        assert_eq!(retained.get(), placed.position(local));
        assert_eq!(pipeline.graph.position(vessel), placed.position(local));
        let mut hidden = ArenaGraph::from_input(&Graph::default());
        let mut first = pipeline.graph.nodes[a.0 as usize].clone();
        first.position = Some(Point { x: 80.0, y: 90.0 });
        let mut second = pipeline.graph.nodes[b.0 as usize].clone();
        second.position = Some(Point { x: 115.0, y: 90.0 });
        hidden
            .transaction_external_aggregate_children
            .insert(10001, vec![first, second]);
        let mut stale_vessel = pipeline.graph.nodes[vessel.0 as usize].clone();
        stale_vessel.position = Some(Point { x: 0.0, y: 0.0 });
        hidden
            .transaction_external_container_children
            .insert(999, vec![stale_vessel]);
        Pipeline::reconcile_projected_sequence_vessels(&pipeline.graph, &mut hidden);
        assert_eq!(
            hidden.transaction_external_container_children[&999][0].position,
            Some(Point { x: 80.0, y: 90.0 })
        );
        Pipeline::publish_induced_external_geometry(&mut pipeline.graph, &hidden);
        assert_eq!(retained.get(), Some(Point { x: 80.0, y: 90.0 }));
        assert!(retained.same_allocation(&pipeline.graph.owner_node_point(vessel).unwrap()));
    }

    #[test]
    fn sequence_scope_retains_external_abductions_and_skips_internal_edges() {
        let mut input = Graph::default();
        let [a, b, peer] = ["a", "b", "peer"].map(|name| {
            input.add_node(crate::Node::new(
                name,
                Size {
                    width: 70.0,
                    height: 10.0,
                },
            ))
        });
        let outgoing = input.add_edge(crate::Edge {
            source: a,
            target: peer,
        });
        let incoming = input.add_edge(crate::Edge {
            source: peer,
            target: b,
        });
        let internal = input.add_edge(crate::Edge {
            source: b,
            target: a,
        });
        let mut pipeline = Pipeline::new_untraced(&input, 1, false, true);
        pipeline.graph.sequences.push(SequenceState {
            members: vec![a, b],
            vessel_tala_id: 10001,
            container: None,
            has_edge_abductions: true,
        });
        for member in [a, b] {
            pipeline.graph.nodes[member.0 as usize].sequence = Some(0);
        }
        pipeline.graph.rebuild_active_aggregate_node_order();
        pipeline
            .prepare_placement_scope_after(None, &BTreeSet::new())
            .unwrap()
            .unwrap();
        let vessel = pipeline
            .graph
            .topology_vessel_node(TopologyVessel::Sequence(0))
            .unwrap();
        let group = pipeline
            .graph
            .owner_graph_group_entry(
                OwnerGraphId(0),
                super::super::owner_groups::OwnerGroupKind::Sequence,
                Some(vessel),
            )
            .unwrap()
            .flatten()
            .unwrap();
        let header = pipeline
            .graph
            .owner_sequence_abductions(group)
            .unwrap()
            .unwrap();
        assert_eq!(header.len(), 2);
        let records = header
            .iter_slots()
            .map(|id| *pipeline.graph.owner_edge_abduction(id.unwrap()).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(records[0].source_edge, Some(outgoing));
        assert_eq!(records[0].originally_from, Some(a));
        assert_eq!(records[1].source_edge, Some(incoming));
        assert_eq!(records[1].originally_to, Some(b));
        assert_eq!(
            pipeline.graph.edge_records[outgoing.0 as usize].from,
            vessel
        );
        assert_eq!(pipeline.graph.edge_records[incoming.0 as usize].to, vessel);
        assert_eq!(pipeline.graph.edge_records[internal.0 as usize].from, b);
        assert_eq!(pipeline.graph.edge_records[internal.0 as usize].to, a);
        pipeline.graph.restore_aggregate_members_to_node_order();
        assert_eq!(pipeline.graph.edge_records[outgoing.0 as usize].from, a);
        assert_eq!(pipeline.graph.edge_records[incoming.0 as usize].to, b);
        assert_eq!(pipeline.graph.edge_records[internal.0 as usize].from, b);
        assert_eq!(pipeline.graph.edge_records[internal.0 as usize].to, a);
        assert_eq!(header.len(), 2);
    }

    #[test]
    fn accepted_cluster_policy_copyback_controls_later_shared_member_geometry() {
        let mut input = Graph::default();
        let [a, b, peer] = ["a", "b", "peer"].map(|name| {
            input.add_node(crate::Node::new(
                name,
                Size {
                    width: 10.0,
                    height: 10.0,
                },
            ))
        });
        for source in [a, b] {
            input.add_edge(crate::Edge {
                source,
                target: peer,
            });
        }
        let mut pipeline = Pipeline::new_untraced(&input, 1, false, true);
        pipeline.graph.clusters.push(ClusterState {
            members: vec![a, b],
            arrangement: ClusterArrangement::Row,
            desired_arrangement: ClusterArrangement::Row,
            padding: 12.0,
            vessel_tala_id: 10001,
            fixed_size: true,
        });
        for member in [a, b] {
            pipeline.graph.nodes[member.0 as usize].cluster = Some(0);
        }
        pipeline.graph.rebuild_active_aggregate_node_order();
        let (placement, prepared) = pipeline
            .prepare_placement_scope_after(None, &BTreeSet::new())
            .unwrap()
            .unwrap();
        let prepared = prepared.unwrap();
        let vessel = pipeline
            .graph
            .topology_vessel_node(TopologyVessel::Cluster(0))
            .unwrap();
        let group = pipeline
            .graph
            .owner_graph_group_entry(
                OwnerGraphId(0),
                owner_groups::OwnerGroupKind::Cluster,
                Some(vessel),
            )
            .unwrap()
            .flatten()
            .unwrap();
        let before = pipeline.graph.owner_cluster_policy(group).unwrap();
        let retained_record = before.edge_abductions.as_ref().unwrap().get(0);
        let mut placed = ArenaGraph::from_input(&Graph::default());
        placed.transaction_external_cluster_layouts.insert(
            10001,
            ExternalClusterLayout {
                arrangement: ClusterArrangement::Column,
                padding: 14.0,
                fixed_size: true,
            },
        );
        pipeline
            .publish_accepted_cluster_policies(
                &BTreeMap::from([(0, ClusterArrangement::Column)]),
                &BTreeMap::from([(0, ClusterArrangement::Column)]),
                &placed,
            )
            .unwrap();
        // Source can update DesiredArrangement without accepting a flip.
        pipeline
            .publish_accepted_cluster_policies(
                &BTreeMap::new(),
                &BTreeMap::from([(0, ClusterArrangement::Row)]),
                &placed,
            )
            .unwrap();
        let policy = pipeline.graph.owner_cluster_policy(group).unwrap();
        assert_eq!(policy.arrangement, "Column");
        assert_eq!(policy.desired_arrangement, "Row");
        assert_eq!(policy.padding, 14.0);
        assert_eq!(policy.fixed_size, before.fixed_size);
        assert_eq!(
            policy.edge_abductions.as_ref().unwrap().get(0),
            retained_record
        );
        for (node, point) in [
            (vessel, Point { x: 100.0, y: 200.0 }),
            (a, Point { x: 100.0, y: 200.0 }),
            (b, Point { x: 132.0, y: 200.0 }),
            (peer, Point { x: 500.0, y: 500.0 }),
        ] {
            pipeline.graph.set_position(node, point);
        }
        let mapping = pipeline
            .owner_score_mapping_for_placement(&placement)
            .unwrap();
        let mut session = OwnerScoreSession::new(
            &mut pipeline.graph,
            prepared.view,
            mapping,
            Some(&prepared.abductions),
        )
        .unwrap();
        session.sync_current_nested_geometry().unwrap();
        assert_eq!(
            session.owner().position(a),
            Some(Point { x: 100.0, y: 200.0 })
        );
        assert_eq!(
            session.owner().position(b),
            Some(Point { x: 100.0, y: 224.0 })
        );
        assert_eq!(
            session.owner().position(peer),
            Some(Point { x: 500.0, y: 500.0 })
        );
    }

    #[test]
    fn nested_cluster_copyback_keeps_member_spacing_in_the_accepted_arrangement() {
        let mut input = Graph::default();
        let [box_node, a, b, peer, outside] =
            ["box", "box.a", "box.b", "box.peer", "outside"].map(|name| {
                input.add_node(crate::Node::new(
                    name,
                    Size {
                        width: 10.0,
                        height: 10.0,
                    },
                ))
            });
        for member in [a, b, peer] {
            input.node_mut(member).unwrap().parent = Some(box_node);
        }
        for (source, target) in [(a, peer), (b, peer), (a, outside), (b, outside)] {
            input.add_edge(crate::Edge { source, target });
        }
        let mut pipeline = Pipeline::new_untraced(&input, 1, false, true);
        pipeline.graph.clusters.push(ClusterState {
            members: vec![a, b],
            arrangement: ClusterArrangement::Column,
            desired_arrangement: ClusterArrangement::Column,
            padding: 12.0,
            vessel_tala_id: 10001,
            fixed_size: true,
        });
        for member in [a, b] {
            pipeline.graph.nodes[member.0 as usize].cluster = Some(0);
        }
        pipeline.graph.rebuild_active_aggregate_node_order();
        for (member, point) in [
            (a, Point { x: 0.0, y: 0.0 }),
            (b, Point { x: 22.0, y: 0.0 }),
        ] {
            pipeline.graph.set_position(member, point);
            pipeline.graph.materialize_owner_node_point(member).unwrap();
        }
        assert!(pipeline.place_nodes_recursively().unwrap());
        let vessel = pipeline
            .graph
            .topology_vessel_node(TopologyVessel::Cluster(0))
            .unwrap();
        assert!(
            pipeline
                .graph
                .topology_container_slice(Some(box_node))
                .unwrap()
                .unwrap()
                .contains(&vessel)
        );
        let group = pipeline
            .graph
            .owner_graph_group_entry(
                OwnerGraphId(0),
                owner_groups::OwnerGroupKind::Cluster,
                Some(vessel),
            )
            .unwrap()
            .flatten()
            .unwrap();
        let policy = pipeline.graph.owner_cluster_policy(group).unwrap();
        let a_position = pipeline.graph.position(a).unwrap();
        let b_position = pipeline.graph.position(b).unwrap();
        // Source direct may reflect the contents. Both member pointers must
        // still have the accepted axis and the exact inter-member padding.
        match policy.arrangement.as_str() {
            "Row" => {
                assert_eq!(a_position.y, b_position.y);
                assert_eq!((b_position.x - a_position.x).abs(), 10.0 + policy.padding);
            }
            "Column" => {
                assert_eq!(a_position.x, b_position.x);
                assert_eq!((b_position.y - a_position.y).abs(), 10.0 + policy.padding);
            }
            arrangement => panic!("invalid accepted arrangement {arrangement}"),
        }
        // The pointer kernel may release its temporary materialized field
        // after restoring ownership. Verify the live field in either storage
        // form rather than requiring that transient adapter to remain bound.
        for (member, position) in [(a, a_position), (b, b_position)] {
            assert_eq!(
                pipeline
                    .graph
                    .owner_bound_point_value(member)
                    .unwrap_or(pipeline.graph.nodes[member.0 as usize].position),
                Some(position),
            );
        }
    }

    #[test]
    fn recursive_cluster_copyback_publishes_the_retained_vessel_point() {
        let mut input = Graph::default();
        let [a, b, peer] = ["a", "b", "peer"].map(|name| {
            input.add_node(crate::Node::new(
                name,
                Size {
                    width: 10.0,
                    height: 10.0,
                },
            ))
        });
        for (source, target) in [(a, peer), (peer, b), (b, peer), (peer, a)] {
            input.add_edge(crate::Edge { source, target });
        }
        let mut pipeline = Pipeline::new_untraced(&input, 1, false, true);
        pipeline.graph.clusters.push(ClusterState {
            members: vec![a, b],
            arrangement: ClusterArrangement::Row,
            desired_arrangement: ClusterArrangement::Row,
            padding: 10.0,
            vessel_tala_id: 10001,
            fixed_size: true,
        });
        for member in [a, b] {
            pipeline.graph.nodes[member.0 as usize].cluster = Some(0);
        }
        pipeline.graph.rebuild_active_aggregate_node_order();
        let placement = pipeline.placement_scope_graph(None).unwrap();
        pipeline
            .bind_placement_aggregate_vessels(&placement)
            .unwrap();
        let vessel = pipeline
            .graph
            .topology_vessel_node(TopologyVessel::Cluster(0))
            .unwrap();
        let initial = Point {
            x: 512.0,
            y: -504.0,
        };
        pipeline.graph.set_position(vessel, initial);
        pipeline.graph.materialize_owner_node_point(vessel).unwrap();
        assert!(pipeline.place_nodes_recursively().unwrap());
        let expected = pipeline.graph.pending_cluster_vessel_positions[&0];
        assert_ne!(expected, initial);
        assert_eq!(pipeline.graph.position(vessel), Some(expected));
        // Source initialize.go clears/replaces TopLeft during initialization;
        // the final field must expose the combined position to every graph
        // view retaining this Node, rather than preserve an old Point pointer.
        assert_eq!(
            pipeline.graph.owner_node_point(vessel).unwrap().get(),
            Some(expected)
        );
        let root_children = pipeline
            .graph
            .topology_container_slice(None)
            .unwrap()
            .unwrap();
        assert!(root_children.iter().any(|child| *child == vessel));
    }

    #[test]
    fn projected_scope_keeps_nears_on_distinct_cluster_vessels() {
        let mut input = Graph::default();
        let [a, b, c, d] = ["a", "b", "c", "d"].map(|name| {
            input.add_node(crate::Node::new(
                name,
                Size {
                    width: 30.0,
                    height: 20.0,
                },
            ))
        });
        let mut pipeline = Pipeline::new_untraced(&input, 1, false, true);
        for (index, members) in [vec![a, b], vec![c, d]].into_iter().enumerate() {
            for member in &members {
                pipeline.graph.nodes[member.0 as usize].cluster = Some(index);
            }
            pipeline.graph.clusters.push(ClusterState {
                members,
                arrangement: ClusterArrangement::Column,
                desired_arrangement: ClusterArrangement::Column,
                padding: 10.0,
                vessel_tala_id: 10001 + index as i64,
                fixed_size: true,
            });
        }
        pipeline.graph.rebuild_active_aggregate_node_order();
        pipeline
            .prepare_placement_scope_after(None, &BTreeSet::new())
            .unwrap()
            .unwrap();
        let first = pipeline
            .graph
            .topology_vessel_node(TopologyVessel::Cluster(0))
            .unwrap();
        let second = pipeline
            .graph
            .topology_vessel_node(TopologyVessel::Cluster(1))
            .unwrap();
        pipeline
            .graph
            .add_owner_near(Some(first), Some(second))
            .unwrap();
        assert!(pipeline.graph.nodes[a.0 as usize].nears.is_empty());
        assert!(pipeline.graph.nodes[c.0 as usize].nears.is_empty());
        let placement = pipeline
            .placement_scope_graph_after(None, &BTreeSet::new())
            .unwrap();
        let first_local = placement.old_to_new[&a];
        let second_local = placement.old_to_new[&c];
        assert_eq!(
            placement.graph.node(first_local).unwrap().near,
            Some(second_local)
        );
        assert_eq!(
            placement.graph.node(second_local).unwrap().near,
            Some(first_local)
        );
        assert!(
            pipeline.graph.nodes[first.0 as usize]
                .nears
                .contains(&second)
        );
        assert!(pipeline.graph.nodes[a.0 as usize].nears.is_empty());
    }

    #[test]
    fn grouped_scope_binds_a_distinct_vessel_and_retains_original_near_pointers() {
        let mut input = Graph::default();
        let [a, b, peer] = ["a", "b", "peer"].map(|name| {
            input.add_node(crate::Node::new(
                name,
                Size {
                    width: 10.0,
                    height: 10.0,
                },
            ))
        });
        input.node_mut(peer).unwrap().near = Some(a);
        let first = input.add_edge(crate::Edge {
            source: a,
            target: peer,
        });
        let second = input.add_edge(crate::Edge {
            source: b,
            target: peer,
        });
        let mut pipeline = Pipeline::new_untraced(&input, 1, false, true);
        pipeline.graph.clusters.push(ClusterState {
            members: vec![a, b],
            arrangement: ClusterArrangement::Row,
            desired_arrangement: ClusterArrangement::Row,
            padding: 10.0,
            vessel_tala_id: 10001,
            fixed_size: true,
        });
        for member in [a, b] {
            pipeline.graph.nodes[member.0 as usize].cluster = Some(0);
        }
        pipeline.graph.rebuild_active_aggregate_node_order();
        let original_nears = pipeline.graph.nodes[a.0 as usize].nears.retained();
        let (placement, prepared) = pipeline
            .prepare_placement_scope_after(None, &BTreeSet::new())
            .unwrap()
            .unwrap();
        let prepared = prepared.expect("grouped scope must use its canonical owner");
        let vessel = pipeline
            .graph
            .topology_vessel_node(TopologyVessel::Cluster(0))
            .unwrap();
        assert!(![a, b].contains(&vessel));
        assert!(pipeline.graph.nodes[vessel.0 as usize].nears.is_empty());
        assert!(pipeline.graph.nodes[a.0 as usize].nears.contains(&peer));
        assert!(original_nears.same_allocation(&pipeline.graph.nodes[a.0 as usize].nears));
        assert!(pipeline.graph.nodes[peer.0 as usize].nears.contains(&a));
        assert!(
            !pipeline.graph.nodes[peer.0 as usize]
                .nears
                .contains(&vessel)
        );
        let mapping = pipeline
            .owner_score_mapping_for_placement(&placement)
            .unwrap();
        assert_eq!(
            mapping.canonical_node(placement.old_to_new[&a]).unwrap(),
            vessel
        );
        assert_eq!(
            mapping.canonical_node(placement.old_to_new[&b]).unwrap(),
            vessel
        );
        assert_eq!(pipeline.graph.edge_records[first.0 as usize].from, vessel);
        assert_eq!(pipeline.graph.edge_records[second.0 as usize].from, vessel);
        let group = pipeline
            .graph
            .owner_graph_group_entry(
                prepared.view,
                super::super::owner_groups::OwnerGroupKind::Cluster,
                Some(vessel),
            )
            .unwrap()
            .flatten()
            .unwrap();
        let policy = pipeline.graph.owner_cluster_policy(group).unwrap();
        assert_eq!(policy.arrangement, "Row");
        let abductions = policy.edge_abductions.unwrap();
        assert_eq!(abductions.len(), 2);
        for (index, member) in [a, b].into_iter().enumerate() {
            let record = pipeline
                .graph
                .owner_edge_abduction(abductions.get(index).unwrap().unwrap())
                .unwrap();
            assert_eq!(record.originally_from, Some(member));
            assert_eq!(record.originally_to, None);
            assert_eq!(record.current_from, Some(vessel));
            assert_eq!(record.current_to, Some(peer));
        }
        assert_eq!(placement.sized_edge_abductions.len(), 2);
        for (index, member) in [a, b].into_iter().enumerate() {
            assert_eq!(
                placement.sized_edge_abductions[index]
                    .originally_from
                    .unwrap()
                    .source_node,
                Some(member)
            );
            assert_eq!(
                pipeline
                    .graph
                    .owner_edge_abduction(prepared.abductions.records[index])
                    .unwrap()
                    .originally_from,
                Some(member)
            );
        }
        pipeline.graph.restore_aggregate_members_to_node_order();
        assert_eq!(pipeline.graph.edge_records[first.0 as usize].from, a);
        assert_eq!(pipeline.graph.edge_records[second.0 as usize].from, b);
        assert_eq!(pipeline.graph.edge_records[first.0 as usize].to, peer);
        assert_eq!(pipeline.graph.edge_records[second.0 as usize].to, peer);
        assert!(original_nears.same_allocation(&pipeline.graph.nodes[a.0 as usize].nears));
        assert!(pipeline.graph.nodes[peer.0 as usize].nears.contains(&a));
    }

    #[test]
    fn common_uncle_groups_survive_parent_abduction_and_share_actual_owner_headers() {
        let mut input = Graph::default();
        let outside = input.add_node(crate::Node::new(
            "outside",
            Size {
                width: 10.0,
                height: 10.0,
            },
        ));
        let container = input.add_node(crate::Node::new(
            "container",
            Size {
                width: 60.0,
                height: 60.0,
            },
        ));
        let children = ["first", "second", "third"].map(|name| {
            let mut node = crate::Node::new(
                name,
                Size {
                    width: 10.0,
                    height: 10.0,
                },
            );
            node.parent = Some(container);
            let child = input.add_node(node);
            input.tala_id_overrides[child.0 as usize] = Some(7);
            input.add_edge(crate::Edge {
                source: child,
                target: outside,
            });
            child
        });
        let mut pipeline = Pipeline::new_untraced(&input, 1, false, true);
        for child in children {
            pipeline.graph.set_position(child, Point::default());
        }
        let map = pipeline.placement_common_uncles();
        assert_eq!(
            map.get(&Some(children[0]))
                .unwrap()
                .require_nodes()
                .collect::<Vec<_>>(),
            children
        );
        assert_eq!(
            map.get(&Some(children[1]))
                .unwrap()
                .require_nodes()
                .collect::<Vec<_>>(),
            children
        );
        pipeline
            .graph
            .replace_owner_common_uncle_map(OwnerGraphId(0), Some(map))
            .unwrap();
        let original = pipeline
            .graph
            .owner_common_uncle_header(OwnerGraphId(0), children[0])
            .unwrap()
            .unwrap();
        assert_eq!(
            original.capacity(),
            4,
            "source append growth retains spare capacity"
        );
        let (_, parent) = pipeline
            .prepare_placement_scope_after(None, &BTreeSet::new())
            .unwrap()
            .unwrap();
        let parent = parent.unwrap();
        assert!(
            children
                .iter()
                .all(|child| pipeline.graph.nodes[child.0 as usize].edges.is_empty())
        );
        let (child, prepared) = pipeline
            .prepare_placement_scope_after(Some(container), &BTreeSet::new())
            .unwrap()
            .unwrap();
        let prepared = prepared.unwrap();
        assert_eq!(
            child.common_uncle_groups,
            vec![children.map(|node| child.old_to_new[&node]).to_vec()]
        );
        for canonical in children {
            let actual = pipeline
                .graph
                .owner_common_uncle_header(prepared.view, canonical)
                .unwrap()
                .unwrap();
            assert_eq!(actual.visible_slots(), children.map(Some));
            assert_eq!(actual.backing_address(), original.backing_address());
        }
        let mut numerical = pipeline.graph.clone();
        let detached = numerical
            .owner_common_uncle_header(prepared.view, children[0])
            .unwrap()
            .unwrap();
        assert_ne!(detached.backing_address(), original.backing_address());
        let empty = numerical.allocate_owner_member_header(&[], 0).unwrap();
        numerical
            .set_owner_common_uncle_members(prepared.view, Some(children[0]), empty)
            .unwrap();
        assert_eq!(
            pipeline
                .graph
                .owner_common_uncle_header(prepared.view, children[0])
                .unwrap()
                .unwrap()
                .visible_slots(),
            children.map(Some)
        );
        assert!(
            numerical
                .owner_common_uncle_header(OwnerGraphId(0), children[0])
                .unwrap()
                .unwrap()
                .is_empty()
        );
        pipeline.restore_prepared_owner_scope(&prepared).unwrap();
        pipeline.restore_prepared_owner_scope(&parent).unwrap();
    }

    #[test]
    fn ancestor_abductions_remain_live_while_child_projection_and_layout_run() {
        let mut input = Graph::default();
        let a = input.add_node(crate::Node::new(
            "a",
            Size {
                width: 53.0,
                height: 66.0,
            },
        ));
        let b = input.add_node(crate::Node::new(
            "b",
            Size {
                width: 58.0,
                height: 81.0,
            },
        ));
        let mut inner = crate::Node::new(
            "inner",
            Size {
                width: 72.0,
                height: 76.0,
            },
        );
        inner.parent = Some(b);
        let inner = input.add_node(inner);
        let mut leaf = crate::Node::new(
            "leaf",
            Size {
                width: 64.0,
                height: 66.0,
            },
        );
        leaf.parent = Some(inner);
        let leaf = input.add_node(leaf);
        let edge = input.add_edge(crate::Edge {
            source: a,
            target: inner,
        });
        let mut pipeline = Pipeline::new_untraced(&input, 1, false, true);
        pipeline.graph.set_position(leaf, Point::default());
        let (_, parent) = pipeline
            .prepare_placement_scope_after(None, &BTreeSet::new())
            .unwrap()
            .unwrap();
        let parent = parent.unwrap();
        assert_eq!(pipeline.graph.edge_records[edge.0 as usize].to, b);
        assert!(!pipeline.graph.nodes[inner.0 as usize].edges.contains(&edge));
        let (child, prepared) = pipeline
            .prepare_placement_scope_after(Some(b), &BTreeSet::new())
            .unwrap()
            .unwrap();
        assert!(child.graph.edges.is_empty());
        let rebuilt = pipeline
            .refresh_prepared_placement_scope(None, &BTreeSet::new(), &parent)
            .unwrap();
        assert_eq!(rebuilt.sized_edge_abductions.len(), 1);
        assert_eq!(pipeline.graph.edge_records[edge.0 as usize].to, b);
        assert!(!pipeline.graph.nodes[inner.0 as usize].edges.contains(&edge));
        let _placed = pipeline
            .solve_prepared_owner_scope(&child, prepared.as_ref())
            .unwrap();
        if let Some(prepared) = prepared {
            pipeline.restore_prepared_owner_scope(&prepared).unwrap();
        }
        assert_eq!(pipeline.graph.edge_records[edge.0 as usize].to, b);
        pipeline.restore_prepared_owner_scope(&parent).unwrap();
        assert_eq!(pipeline.graph.edge_records[edge.0 as usize].to, inner);
        assert!(pipeline.graph.nodes[inner.0 as usize].edges.contains(&edge));
    }

    #[test]
    fn ordinary_production_scope_initialization_reaches_the_canonical_owner() {
        let mut input = Graph::default();
        let container = input.add_node(crate::Node::new(
            "C",
            Size {
                width: 180.0,
                height: 180.0,
            },
        ));
        let mut child = crate::Node::new(
            "c",
            Size {
                width: 10.0,
                height: 10.0,
            },
        );
        child.parent = Some(container);
        let child = input.add_node(child);
        let leaf = input.add_node(crate::Node::new(
            "L",
            Size {
                width: 10.0,
                height: 10.0,
            },
        ));
        input.add_edge(crate::Edge {
            source: container,
            target: leaf,
        });
        let mut pipeline = Pipeline::new_untraced(&input, 1, false, true);
        for (node, point) in [
            (container, Point { x: 800.0, y: 800.0 }),
            (child, Point { x: 820.0, y: 820.0 }),
            (
                leaf,
                Point {
                    x: 1200.0,
                    y: 800.0,
                },
            ),
        ] {
            pipeline.graph.set_position(node, point);
        }
        let child_address = &pipeline.graph.nodes[child.0 as usize] as *const ArenaNode;
        let before = pipeline.graph.position(child);
        let (placement, prepared) = pipeline
            .prepare_placement_scope_after(None, &BTreeSet::new())
            .unwrap()
            .unwrap();
        assert!(
            prepared.is_some(),
            "ordinary scope must carry the real owner session"
        );
        let _placed = pipeline
            .solve_prepared_owner_scope(&placement, prepared.as_ref())
            .unwrap();
        // solve has not performed successful dense copyback. These changes
        // therefore prove the production initialization boundary writes the
        // actual retained node, rather than only a diagnostic reconstruction.
        assert_ne!(pipeline.graph.position(child), before);
        assert_eq!(
            &pipeline.graph.nodes[child.0 as usize] as *const ArenaNode,
            child_address
        );
        assert!(pipeline.graph.position(container).is_some());
        pipeline
            .restore_prepared_owner_scope(prepared.as_ref().unwrap())
            .unwrap();
    }
}

#[cfg(test)]
mod nullable_edge_abduction_tests {
    use super::*;

    fn direct_fixture(reverse: bool, mixed: bool) -> (Pipeline, EdgeId, NodeId) {
        let mut graph = Graph::default();
        let order = if reverse {
            [2_i64, 1_i64]
        } else {
            [1_i64, 2_i64]
        };
        let ids = order
            .iter()
            .enumerate()
            .map(|(index, &tala)| (tala, NodeId(index as u32)))
            .collect::<BTreeMap<_, _>>();
        for tala in order {
            let dimension = if tala == 1 { 60.0 } else { 10.0 };
            let mut node = crate::Node::new(
                format!("nullable-{tala}"),
                Size {
                    width: dimension,
                    height: dimension,
                },
            );
            node.parent = (tala == 2).then_some(ids[&1]);
            let id = graph.add_node(node);
            graph.tala_id_overrides[id.0 as usize] = Some(tala);
            graph.set_initial_position(
                id,
                Some(if tala == 1 {
                    Point { x: 80.0, y: 80.0 }
                } else {
                    Point { x: 20.0, y: 20.0 }
                }),
            );
            graph.set_container_flag(id, tala == 1);
        }
        let (source, target) = if reverse {
            (ids[&2], ids[&1])
        } else {
            (ids[&1], ids[&2])
        };
        let detached = graph.add_edge(crate::Edge { source, target });
        graph.tala_edge_ids[detached.0 as usize] = Some(11);
        if mixed {
            let outside = graph.add_node(crate::Node::new(
                "outside",
                Size {
                    width: 10.0,
                    height: 10.0,
                },
            ));
            graph.tala_id_overrides[outside.0 as usize] = Some(3);
            graph.set_initial_position(outside, Some(Point { x: 200.0, y: 100.0 }));
            let external = graph.add_edge(crate::Edge {
                source: ids[&2],
                target: outside,
            });
            graph.tala_edge_ids[external.0 as usize] = Some(12);
        }
        let mut pipeline = Pipeline::new_untraced(&graph, 0, true, true);
        // This is the direct NewGraph/AddNode/Connect/AbductEdges boundary,
        // whose source has not extracted trees or run movable-node placement.
        pipeline.graph.preprocessed_tree_children.clear();
        pipeline.graph.tree_routing_nodes.clear();
        pipeline.graph.tree_sentinels.clear();
        pipeline.graph.restored_tree_edges.clear();
        pipeline.graph.published_tree_node_edge_order.clear();
        for (id, _) in graph.nodes() {
            pipeline
                .graph
                .set_position(id, graph.initial_positions[id.0 as usize].unwrap());
        }
        (pipeline, detached, ids[&1])
    }

    #[test]
    fn construction_inputs_restore_incident_field_aliases_after_unwind() {
        let (mut pipeline, _, _) = direct_fixture(false, true);
        let (_, prepared) = pipeline
            .prepare_placement_scope_after(None, &BTreeSet::new())
            .unwrap()
            .unwrap();
        let prepared = prepared.unwrap();
        pipeline
            .graph
            .assign_hierarchy_member(NodeId(0), 0, None, 1, 3);
        pipeline
            .graph
            .assign_hierarchy_member(NodeId(1), 0, None, 2, 3);
        let hierarchy = pipeline.graph.nodes[0]
            .hierarchy
            .as_ref()
            .unwrap()
            .record
            .clone();
        assert!(!prepared.construction_endpoints.is_empty());
        assert!(!prepared.construction_incidents.is_empty());
        let retained = prepared
            .construction_incidents
            .keys()
            .map(|&node| {
                let edges = &pipeline.graph.nodes[node.0 as usize].edges;
                (node, edges.retain_field(), edges.header(), edges.read())
            })
            .collect::<Vec<_>>();
        let original = format!("{:?}", pipeline.graph);
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            pipeline.with_prepared_construction_inputs(&prepared, |builder| {
                let projected = builder.graph.nodes[0].hierarchy.as_ref().unwrap();
                assert!(!projected.record.same_allocation(&hierarchy));
                assert_eq!(projected.level(), 1);
                for (&edge, &(from, to)) in &prepared.construction_endpoints {
                    let value = &builder.graph.edge_records[edge.0 as usize];
                    assert_eq!((value.from, value.to), (from, to));
                }
                for (node, field, header, values) in &retained {
                    assert_eq!(field.read(), *values);
                    assert_eq!(header.read_values(), *values);
                    assert_eq!(
                        builder.graph.nodes[node.0 as usize].edges.read(),
                        prepared.construction_incidents[node],
                    );
                }
                panic!("construction interruption");
            });
        }));
        assert!(outcome.is_err());
        assert_eq!(format!("{:?}", pipeline.graph), original);
        let projected = pipeline.with_prepared_construction_inputs(&prepared, |builder| {
            [0, 1].map(|i| builder.graph.nodes[i].hierarchy.clone().unwrap())
        });
        assert!(projected[0].record.same_allocation(&projected[1].record));
        assert!(!projected[0].record.same_allocation(&hierarchy));
        assert!(
            pipeline.graph.nodes[0]
                .hierarchy
                .as_ref()
                .unwrap()
                .record
                .same_allocation(&hierarchy)
        );
        assert_eq!([projected[0].level(), projected[1].level()], [1, 2]);
        assert_eq!(projected[0].level_count(), 3);
        assert_eq!(format!("{:?}", pipeline.graph), original);
        for (node, field, header, values) in retained {
            let restored = pipeline.graph.nodes[node.0 as usize].edges.header();
            assert_eq!(restored.slot_address(0), header.slot_address(0));
            assert_eq!(restored.capacity(), header.capacity());
            assert_eq!(field.read(), values);
            assert_eq!(header.read_values(), values);
        }
    }

    #[test]
    fn actual_preparation_mutates_owner_and_foreign_restore_keeps_record_handles() {
        for reverse in [false, true] {
            let (mut pipeline, detached, owner) = direct_fixture(reverse, true);
            let external = EdgeId(1);
            let original_external = pipeline.graph.edge_records[external.0 as usize].from;
            let original_target = pipeline.graph.edge_records[external.0 as usize].to;
            let (_, prepared) = pipeline
                .prepare_placement_scope_after(None, &BTreeSet::new())
                .unwrap()
                .unwrap();
            let prepared = prepared.expect("ordinary nodes have canonical owner lineage");
            assert_eq!(pipeline.graph.edge_records[external.0 as usize].from, owner);
            assert_eq!(
                pipeline.graph.edge_records[external.0 as usize].to,
                original_target
            );
            assert!(
                !pipeline.graph.nodes[original_external.0 as usize]
                    .edges
                    .contains(&external)
            );
            assert!(
                pipeline.graph.nodes[owner.0 as usize]
                    .edges
                    .contains(&external)
            );
            assert!(
                !pipeline.graph.nodes[owner.0 as usize]
                    .edges
                    .contains(&detached)
            );
            let remaining = pipeline
                .graph
                .restore_owner_edge_abductions(&prepared.abductions, &[owner])
                .unwrap();
            assert_eq!(remaining.records.len(), 1);
            assert!(remaining.present);
            assert!(prepared.abductions.records.contains(&remaining.records[0]));
            assert_eq!(
                pipeline
                    .graph
                    .owner_edge_abduction(remaining.records[0])
                    .unwrap()
                    .source_edge,
                Some(external)
            );
            assert_eq!(pipeline.graph.edge_records[external.0 as usize].from, owner);
            pipeline.restore_prepared_owner_scope(&prepared).unwrap();
            assert_eq!(
                pipeline.graph.edge_records[external.0 as usize].from,
                original_external
            );
            assert_eq!(
                pipeline.graph.edge_records[external.0 as usize].to,
                original_target
            );
            assert!(
                pipeline.graph.nodes[original_external.0 as usize]
                    .edges
                    .contains(&external)
            );
            assert!(
                !pipeline.graph.nodes[owner.0 as usize]
                    .edges
                    .contains(&external)
            );
        }
    }

    #[test]
    fn projector_uses_live_owner_endpoints_after_an_active_abduction() {
        let (mut pipeline, detached, owner) = direct_fixture(false, true);
        let (_, prepared) = pipeline
            .prepare_placement_scope_after(None, &BTreeSet::new())
            .unwrap()
            .unwrap();
        let prepared = prepared.unwrap();
        let active_projection = pipeline.placement_scope_graph(None).unwrap();
        assert_eq!(pipeline.graph.edge_records[1].from, owner);
        assert!(
            !active_projection
                .sized_edge_abductions
                .iter()
                .any(|record| record.source_edge == EdgeId(1))
        );
        assert!(
            active_projection
                .sized_edge_abductions
                .iter()
                .any(|record| record.source_edge == detached)
        );
        pipeline.restore_prepared_owner_scope(&prepared).unwrap();
        assert!(
            pipeline
                .placement_scope_graph(None)
                .unwrap()
                .sized_edge_abductions
                .iter()
                .any(|record| record.source_edge == EdgeId(1))
        );
    }

    #[test]
    fn direct_container_descendant_edge_retains_nullable_record_in_both_directions() {
        for reverse in [false, true] {
            let (pipeline, source_edge, center) = direct_fixture(reverse, false);
            let placement = pipeline.placement_scope_graph(None).unwrap();
            assert_eq!(placement.graph.edges().count(), 0);
            assert_eq!(placement.sized_edge_abductions.len(), 1);
            let record = &placement.sized_edge_abductions[0];
            assert_eq!(record.source_edge, source_edge);
            assert_eq!(record.edge_tala_id, 11);
            assert_eq!(
                (record.edge, record.current_from, record.current_to),
                (None, None, None)
            );
            let (from, to) = if reverse { (2, 1) } else { (1, 2) };
            assert_eq!(record.originally_from.unwrap().tala_id, from);
            assert_eq!(record.originally_to.unwrap().tala_id, to);
            let child = if reverse {
                record.originally_from.unwrap()
            } else {
                record.originally_to.unwrap()
            };
            assert_eq!(child.offset, Some(Point { x: 20.0, y: 20.0 }));
            assert_eq!(child.owner, Some(placement.old_to_new[&center]));
            assert!(placement.sized_collapsed_symmetry_neighbors.is_empty());
            assert_eq!(placement.collapsed_restored_mirror_edges.len(), 1);
            assert!(
                pipeline.graph.nodes[center.0 as usize]
                    .fixed_top_left
                    .is_none()
            );
        }
    }

    #[test]
    fn detached_unpositioned_originals_keep_identity_and_owning_restore_order() {
        for reverse in [false, true] {
            for owner_positioned in [false, true] {
                let (mut pipeline, source_edge, owner) = direct_fixture(reverse, false);
                let edge = &pipeline.graph.edge_records[source_edge.0 as usize];
                let original_from = edge.from;
                let original_to = edge.to;
                let child = if original_from == owner {
                    original_to
                } else {
                    original_from
                };
                pipeline.graph.clear_position(child);
                if !owner_positioned {
                    pipeline.graph.clear_position(owner);
                }
                let placement = pipeline.placement_scope_graph(None).unwrap();
                let record = &placement.sized_edge_abductions[0];
                let from = record
                    .originally_from
                    .expect("real original From node survives nil TopLeft");
                let to = record
                    .originally_to
                    .expect("real original To node survives nil TopLeft");
                assert_eq!(from.source_node, Some(original_from));
                assert_eq!(to.source_node, Some(original_to));
                let (owner_ref, child_ref) = if original_from == owner {
                    (from, to)
                } else {
                    (to, from)
                };
                assert_eq!(child_ref.source_node, Some(child));
                assert_eq!(child_ref.offset, None);
                assert!(child_ref.projection().is_none());
                assert_eq!(
                    owner_ref.offset,
                    owner_positioned.then_some(Point::default())
                );
                assert_eq!(
                    placement.restored_endpoint_order,
                    [(original_from, source_edge), (original_to, source_edge)]
                );
                assert_eq!(
                    (record.edge, record.current_from, record.current_to),
                    (None, None, None)
                );
                assert!(placement.sized_collapsed_symmetry_neighbors.is_empty());
                assert_eq!(pipeline.graph.position(child), None);
                assert_eq!(pipeline.graph.position(owner).is_some(), owner_positioned);
                assert!(
                    pipeline.graph.nodes[owner.0 as usize]
                        .fixed_top_left
                        .is_none()
                );
            }
        }
    }

    #[test]
    fn hydrated_induced_unpositioned_originals_preserve_owning_lineage() {
        let (mut pipeline, source_edge, owner) = direct_fixture(true, false);
        let source = &pipeline.graph.edge_records[source_edge.0 as usize];
        let child = source.from;
        assert_ne!(child, owner);
        pipeline.graph.clear_position(child);
        pipeline.graph.clear_position(owner);
        let placement = pipeline.placement_scope_graph(None).unwrap();
        let mut temporary = Pipeline::new_untraced(&placement.graph, 0, false, true);
        temporary.hydrate_placement_scope(&placement);
        let (induced, new_to_old) = temporary
            .graph
            .induced_placement_subgraph(&temporary.graph.node_order);
        let before = &temporary.graph.sized_edge_abductions[0];
        let after = &induced.sized_edge_abductions[0];
        assert!(induced.sized_edge_abductions_present);
        for (before, after) in [
            (
                before.originally_from.unwrap(),
                after.originally_from.unwrap(),
            ),
            (before.originally_to.unwrap(), after.originally_to.unwrap()),
        ] {
            assert_eq!(after.source_node, before.source_node);
            assert_eq!(
                Some(new_to_old[after.owner.unwrap().0 as usize]),
                before.owner
            );
            assert_eq!(after.tala_id, before.tala_id);
            assert_eq!(after.offset, None);
            assert!(after.projection().is_none());
        }
        // Reversed declarations put A at owning slot0, while the temporary
        // carrier C is local slot0. Lineage must not be densified into C.
        assert_eq!(after.originally_from.unwrap().source_node, Some(child));
        assert_eq!(after.originally_to.unwrap().source_node, Some(owner));
        assert_ne!(
            after.originally_to.unwrap().source_node.unwrap(),
            after.originally_to.unwrap().owner.unwrap()
        );
        assert_eq!(
            (after.edge, after.current_from, after.current_to),
            (None, None, None)
        );
        assert_eq!(after.source_edge, source_edge);
    }

    #[test]
    fn mixed_detached_and_reconnected_records_keep_owning_edge_order() {
        let (pipeline, detached, _) = direct_fixture(false, true);
        let placement = pipeline.placement_scope_graph(None).unwrap();
        assert_eq!(placement.graph.edges().count(), 1);
        let records = &placement.sized_edge_abductions;
        assert_eq!(
            records
                .iter()
                .map(|record| record.edge_tala_id)
                .collect::<Vec<_>>(),
            [11, 12]
        );
        assert_eq!(records[0].source_edge, detached);
        assert!(records[0].edge.is_none());
        assert!(records[1].edge.is_some());
        assert!(records[1].current_from.is_some() && records[1].current_to.is_some());
    }

    #[test]
    fn nested_detached_carrier_keeps_zero_self_offset_and_local_child_offset() {
        let mut graph = Graph::default();
        let outer = graph.add_node(crate::Node::new(
            "outer",
            Size {
                width: 500.0,
                height: 500.0,
            },
        ));
        graph.tala_id_overrides[outer.0 as usize] = Some(1);
        graph.set_container_flag(outer, true);
        graph.set_initial_position(outer, Some(Point { x: 300.0, y: 400.0 }));
        let mut carrier_node = crate::Node::new(
            "carrier",
            Size {
                width: 60.0,
                height: 60.0,
            },
        );
        carrier_node.parent = Some(outer);
        let carrier = graph.add_node(carrier_node);
        graph.tala_id_overrides[carrier.0 as usize] = Some(2);
        graph.set_container_flag(carrier, true);
        graph.set_initial_position(carrier, Some(Point { x: 80.0, y: 80.0 }));
        let mut child_node = crate::Node::new(
            "child",
            Size {
                width: 10.0,
                height: 10.0,
            },
        );
        child_node.parent = Some(carrier);
        let child = graph.add_node(child_node);
        graph.tala_id_overrides[child.0 as usize] = Some(3);
        graph.set_initial_position(child, Some(Point { x: 20.0, y: 20.0 }));
        let edge = graph.add_edge(crate::Edge {
            source: carrier,
            target: child,
        });
        graph.tala_edge_ids[edge.0 as usize] = Some(11);
        let mut pipeline = Pipeline::new_untraced(&graph, 0, true, true);
        pipeline.graph.preprocessed_tree_children.clear();
        pipeline.graph.tree_routing_nodes.clear();
        pipeline.graph.tree_sentinels.clear();
        pipeline.graph.restored_tree_edges.clear();
        pipeline.graph.published_tree_node_edge_order.clear();
        for (id, _) in graph.nodes() {
            pipeline
                .graph
                .set_position(id, graph.initial_positions[id.0 as usize].unwrap());
        }
        let placement = pipeline.placement_scope_graph(Some(outer)).unwrap();
        assert_eq!(placement.graph.edges().count(), 0);
        assert_eq!(placement.sized_edge_abductions.len(), 1);
        let record = &placement.sized_edge_abductions[0];
        assert_eq!(
            record.originally_from.unwrap().offset,
            Some(Point::default())
        );
        assert_eq!(
            record.originally_to.unwrap().offset,
            Some(Point { x: 20.0, y: 20.0 })
        );
        assert_eq!(
            record.originally_from.unwrap().owner,
            Some(placement.old_to_new[&carrier])
        );
        assert_eq!(
            (record.edge, record.current_from, record.current_to),
            (None, None, None)
        );
    }

    #[test]
    fn hydrated_and_induced_detached_records_keep_nil_fields_and_slice_presence() {
        let (pipeline, source_edge, _) = direct_fixture(false, false);
        let placement = pipeline.placement_scope_graph(None).unwrap();
        let mut temporary = Pipeline::new_untraced(&placement.graph, 0, false, true);
        assert!(!temporary.graph.has_sized_edge_abductions());
        temporary.hydrate_placement_scope(&placement);
        assert!(temporary.graph.sized_edge_abductions_present);
        let (induced, _) = temporary
            .graph
            .induced_placement_subgraph(&temporary.graph.node_order);
        assert!(induced.sized_edge_abductions_present);
        assert_eq!(induced.sized_edge_abductions.len(), 1);
        let record = &induced.sized_edge_abductions[0];
        assert_eq!(record.source_edge, source_edge);
        assert_eq!(
            (record.edge, record.current_from, record.current_to),
            (None, None, None)
        );
        assert_eq!(record.originally_from.unwrap().tala_id, 1);
        assert_eq!(record.originally_to.unwrap().tala_id, 2);
    }
}

#[cfg(test)]
mod foreign_edge_abduction_slice_tests {
    use super::*;
    use crate::internal::limits::{Context, ContextError};
    use crate::internal::scoring_context::ScoreContext;
    use std::cell::Cell;

    struct PollContext {
        polls: Cell<usize>,
        cancel_at: Option<usize>,
    }

    impl Context for PollContext {
        fn done(&self) -> Option<bool> {
            None
        }
        fn error(&self) -> Option<ContextError> {
            let poll = self.polls.get() + 1;
            self.polls.set(poll);
            self.cancel_at
                .filter(|&at| poll >= at)
                .map(|_| ContextError::Cancelled)
        }
    }

    // The production AbductEdges projector, hydration and induced-component
    // constructor all run here. No SizedEdgeAbduction records are authored.
    fn fixture(
        count: usize,
        live_foreign: bool,
        local_detached: bool,
        reverse: bool,
    ) -> (ArenaGraph, NodeId, Vec<(EdgeId, i64)>) {
        let mut input = Graph::default();
        let mut source_edge_order = None;
        for index in 0..count {
            let owner = input.add_node(crate::Node::new(
                format!("D{index}"),
                Size {
                    width: 60.0,
                    height: 60.0,
                },
            ));
            input.set_container_flag(owner, true);
            let mut child = crate::Node::new(
                format!("d{index}"),
                Size {
                    width: 10.0,
                    height: 10.0,
                },
            );
            child.parent = Some(owner);
            let child = input.add_node(child);
            let (source, target) = if reverse {
                (child, owner)
            } else {
                (owner, child)
            };
            input.add_edge(crate::Edge { source, target });
            if live_foreign {
                let outside = input.add_node(crate::Node::new(
                    format!("X{index}"),
                    Size {
                        width: 10.0,
                        height: 10.0,
                    },
                ));
                input.add_edge(crate::Edge {
                    source: child,
                    target: outside,
                });
            }
        }
        let center = input.add_node(crate::Node::new(
            "C",
            Size {
                width: 60.0,
                height: 60.0,
            },
        ));
        input.set_container_flag(center, true);
        let mut child = crate::Node::new(
            "c",
            Size {
                width: 10.0,
                height: 10.0,
            },
        );
        child.parent = Some(center);
        let child = input.add_node(child);
        if local_detached {
            let local_edge = input.add_edge(crate::Edge {
                source: center,
                target: child,
            });
            // Interleave an authentic local record between foreign records.
            let mut order = input.edges().map(|(id, _)| id).collect::<Vec<_>>();
            order.retain(|&edge| edge != local_edge);
            order.insert(usize::from(!reverse).min(order.len()), local_edge);
            source_edge_order = Some(order);
        }
        let mut pipeline = Pipeline::new_untraced(&input, 0, true, true);
        pipeline.graph.preprocessed_tree_children.clear();
        pipeline.graph.tree_routing_nodes.clear();
        pipeline.graph.tree_sentinels.clear();
        pipeline.graph.restored_tree_edges.clear();
        pipeline.graph.published_tree_node_edge_order.clear();
        if let Some(order) = source_edge_order {
            pipeline.graph.edge_order.replace_values(&order);
        }
        let placement = pipeline.placement_scope_graph(None).unwrap();
        let expected = placement
            .sized_edge_abductions
            .iter()
            .map(|record| (record.source_edge, record.edge_tala_id))
            .collect();
        let mut temporary = Pipeline::new_untraced(&placement.graph, 0, false, true);
        temporary.hydrate_placement_scope(&placement);
        (temporary.graph, placement.old_to_new[&center], expected)
    }

    #[test]
    fn induced_foreign_detached_slice_keeps_one_and_sixty_five_records_and_real_polls() {
        for count in [1, 65] {
            for reverse in [false, true] {
                let (temporary, center, expected) = fixture(count, false, false, reverse);
                let (component, _) = temporary.induced_placement_subgraph(&[center]);
                assert_eq!(component.nodes.len(), 1);
                assert!(component.edge_order.is_empty());
                assert!(component.sized_edge_abductions_present);
                assert_eq!(component.sized_edge_abductions.len(), count);
                assert_eq!(
                    component
                        .sized_edge_abductions
                        .iter()
                        .map(|r| (r.source_edge, r.edge_tala_id))
                        .collect::<Vec<_>>(),
                    expected
                );
                for record in &component.sized_edge_abductions {
                    assert!(record.current_from.is_none() && record.current_to.is_none());
                    for original in [
                        record.originally_from.unwrap(),
                        record.originally_to.unwrap(),
                    ] {
                        assert!(original.source_node.is_some());
                        assert!(original.source_owner.is_some());
                        assert!(original.owner_tala_id.is_some());
                        assert!(original.owner.is_none());
                        assert!(original.projection().is_none());
                    }
                }
                // Source symmetry.go:95,136 (0/64),290 (0/64),398.
                let expected_polls = 2 + 2 * count.div_ceil(64);
                let context = PollContext {
                    polls: Cell::new(0),
                    cancel_at: None,
                };
                assert_eq!(
                    component
                        .sized_symmetry_checked(NodeId(0), true, ScoreContext::new(&context))
                        .unwrap()
                        .to_bits(),
                    0.0_f64.to_bits()
                );
                assert_eq!(context.polls.get(), expected_polls);
                assert!(component.position(NodeId(0)).is_none());
                for cancel_at in 2..expected_polls {
                    let context = PollContext {
                        polls: Cell::new(0),
                        cancel_at: Some(cancel_at),
                    };
                    assert_eq!(
                        component
                            .sized_symmetry_checked(NodeId(0), true, ScoreContext::new(&context))
                            .unwrap_err()
                            .to_string(),
                        "EdgeLength: context canceled"
                    );
                    assert_eq!(context.polls.get(), cancel_at);
                }
            }
        }
    }

    #[test]
    fn production_projector_keeps_full_foreign_obstruction_sets_through_hydration_and_induction() {
        for count in [1, 65] {
            for reverse in [false, true] {
                let mut input = Graph::default();
                let mut foreign = Vec::new();
                for index in (0..count).map(|index| if reverse { count - 1 - index } else { index })
                {
                    let owner = input.add_node(crate::Node::new(
                        format!("D{index}"),
                        Size {
                            width: 60.0,
                            height: 60.0,
                        },
                    ));
                    input.set_container_flag(owner, true);
                    let mut child = crate::Node::new(
                        format!("d{index}"),
                        Size {
                            width: 10.0,
                            height: 10.0,
                        },
                    );
                    child.parent = Some(owner);
                    input.add_node(child);
                    foreign.push(owner);
                }
                let center = input.add_node(crate::Node::new(
                    "C",
                    Size {
                        width: 60.0,
                        height: 60.0,
                    },
                ));
                input.set_container_flag(center, true);
                let mut child = crate::Node::new(
                    "c",
                    Size {
                        width: 10.0,
                        height: 10.0,
                    },
                );
                child.parent = Some(center);
                let child = input.add_node(child);
                let outside = input.add_node(crate::Node::new(
                    "L",
                    Size {
                        width: 10.0,
                        height: 10.0,
                    },
                ));
                input.add_edge(crate::Edge {
                    source: if reverse { outside } else { child },
                    target: if reverse { child } else { outside },
                });
                let mut source = Pipeline::new_untraced(&input, 0, true, true);
                source.graph.preprocessed_tree_children.clear();
                source.graph.tree_routing_nodes.clear();
                source.graph.tree_sentinels.clear();
                source.graph.restored_tree_edges.clear();
                source.graph.published_tree_node_edge_order.clear();
                source
                    .graph
                    .set_position(center, Point { x: 80.0, y: 80.0 });
                source
                    .graph
                    .set_position(child, Point { x: 100.0, y: 100.0 });
                source
                    .graph
                    .set_position(outside, Point { x: -80.0, y: 100.0 });
                // The first real foreign root remains nil; its pointer must
                // still occupy its actual root-slice position.
                for (index, &owner) in foreign.iter().enumerate().skip(1) {
                    source.graph.set_position(
                        owner,
                        Point {
                            x: 300.0 + index as f64 * 100.0,
                            y: 80.0,
                        },
                    );
                }
                source.graph.initialize_shared_topology();
                let placement = source.placement_scope_graph(None).unwrap();
                assert_eq!(placement.sized_edge_abductions.len(), 1);
                let mut temporary = Pipeline::new_untraced(&placement.graph, 0, false, true);
                temporary.hydrate_placement_scope(&placement);
                let members = [
                    placement.old_to_new[&center],
                    placement.old_to_new[&outside],
                ];
                let (mut component, _) = temporary.graph.induced_placement_subgraph(&members);
                for _ in 0..2 {
                    let record = &component.sized_edge_abductions[0];
                    let inventory = if reverse {
                        &record.obstruction_sets_to_from
                    } else {
                        &record.obstruction_sets_from_to
                    };
                    assert!(inventory.sets_present);
                    assert_eq!(inventory.sets.len(), 2);
                    assert_eq!(inventory.sets[0].source_container, Some(center));
                    let child_item = &inventory.sets[0].items.as_ref().unwrap()[0];
                    assert_eq!(inventory.sets[0].items.as_ref().unwrap().len(), 1);
                    assert_eq!(child_item.source_node, Some(child));
                    assert_eq!(child_item.source_owner, Some(center));
                    assert_eq!(child_item.location, AbductionNodeLocation::Local(NodeId(0)));
                    assert!(child_item.same_graph_as_replacement);
                    assert!(child_item.is_endpoint);
                    assert_eq!(inventory.sets[1].source_container, None);
                    let roots = inventory.sets[1].items.as_ref().unwrap();
                    assert_eq!(roots.len(), count + 2);
                    assert_eq!(
                        roots
                            .iter()
                            .map(|item| item.source_node.unwrap())
                            .collect::<Vec<_>>(),
                        foreign
                            .iter()
                            .copied()
                            .chain([center, outside])
                            .collect::<Vec<_>>()
                    );
                    for item in &roots[..count] {
                        assert_eq!(item.location, AbductionNodeLocation::Foreign);
                        assert!(!item.same_graph_as_replacement);
                    }
                    assert!(roots[0].position.is_none());
                    (component, _) = component.induced_placement_subgraph(&[NodeId(0), NodeId(1)]);
                }
            }
        }
    }

    #[test]
    fn induced_foreign_live_pointers_never_alias_a_local_dense_node_or_become_nil() {
        let (temporary, center, expected) = fixture(1, true, false, false);
        let (component, _) = temporary.induced_placement_subgraph(&[center]);
        assert_eq!(component.sized_edge_abductions.len(), 2);
        let live = &component.sized_edge_abductions[1];
        assert!(live.edge.is_none());
        for current in [live.current_from.unwrap(), live.current_to.unwrap()] {
            assert_eq!(current.location, AbductionNodeLocation::Foreign);
            assert!(current.local_node().is_none());
            assert!(current.source_node.is_some());
            assert!(current.tala_id.is_some());
            assert!(current.size.is_some());
            assert!(current.position.is_none());
        }
        let (repeated, _) = component.induced_placement_subgraph(&[NodeId(0)]);
        assert_eq!(
            repeated
                .sized_edge_abductions
                .iter()
                .map(|r| (r.source_edge, r.edge_tala_id))
                .collect::<Vec<_>>(),
            expected
        );
        for (first, second) in component
            .sized_edge_abductions
            .iter()
            .zip(&repeated.sized_edge_abductions)
        {
            assert_eq!(first.current_from, second.current_from);
            assert_eq!(first.current_to, second.current_to);
            for (first, second) in [
                (first.originally_from, second.originally_from),
                (first.originally_to, second.originally_to),
            ] {
                assert_eq!(
                    first.map(|r| (
                        r.source_node,
                        r.source_owner,
                        r.owner_tala_id,
                        r.owner,
                        r.tala_id
                    )),
                    second.map(|r| (
                        r.source_node,
                        r.source_owner,
                        r.owner_tala_id,
                        r.owner,
                        r.tala_id
                    ))
                );
            }
        }
    }

    #[test]
    fn induced_local_and_foreign_detached_records_preserve_mixed_source_order() {
        for reverse in [false, true] {
            let (temporary, center, expected) = fixture(2, false, true, reverse);
            let (component, _) = temporary.induced_placement_subgraph(&[center]);
            assert_eq!(component.sized_edge_abductions.len(), 3);
            assert_eq!(
                component
                    .sized_edge_abductions
                    .iter()
                    .map(|r| (r.source_edge, r.edge_tala_id))
                    .collect::<Vec<_>>(),
                expected
            );
            let owners = component
                .sized_edge_abductions
                .iter()
                .map(|r| r.originally_from.unwrap().owner)
                .collect::<Vec<_>>();
            assert_eq!(
                owners
                    .iter()
                    .filter(|owner| **owner == Some(NodeId(0)))
                    .count(),
                1
            );
            assert_eq!(owners.iter().filter(|owner| owner.is_none()).count(), 2);
            assert!(
                component
                    .sized_edge_abductions
                    .iter()
                    .all(|r| r.current_from.is_none() && r.current_to.is_none())
            );
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum MirrorReachNode {
    Current(NodeId),
    Original(i64),
}

/// Returns the endpoint of each serialized cluster abduction that TALA
/// considers external to the cluster. The zero node ID is graphjson's nil
/// pointer representation. Preserve the edge-abduction slice's first-seen
/// order because `clusterExternalConnectedNodes` feeds an ordered comparison.
fn serialized_cluster_external_endpoint_ids(
    abductions: &[ResolvedAggregateEdgeAbduction],
) -> Vec<AggregateNodeRef> {
    let mut seen = BTreeSet::new();
    let mut endpoints = Vec::new();
    for abduction in abductions {
        let external = match (
            abduction.originally_from.is_none(),
            abduction.originally_to.is_none(),
        ) {
            (true, false) => abduction.current_from,
            (false, true) => abduction.current_to,
            _ => continue,
        };
        if let Some(external) = external
            && seen.insert(external)
        {
            endpoints.push(external);
        }
    }
    endpoints
}

pub(super) fn restored_mirror_reachable_containers(
    placement: &PlacementScope,
    graph: &ArenaGraph,
    members: &[NodeId],
) -> BTreeSet<Option<i64>> {
    let scope_parent = placement
        .scoring_node_metadata
        .first()
        .and_then(|metadata| metadata.parent_tala_id);
    let current_containers = placement
        .scoring_node_metadata
        .iter()
        .map(|metadata| (metadata.node, metadata.parent_tala_id))
        .collect::<BTreeMap<_, _>>();
    let mut original_containers = BTreeMap::<i64, Option<i64>>::new();
    let mut adjacency = BTreeMap::<MirrorReachNode, Vec<MirrorReachNode>>::new();
    let mut connect = |first: MirrorReachNode, second: MirrorReachNode| {
        adjacency.entry(first).or_default().push(second);
        if first != second {
            adjacency.entry(second).or_default().push(first);
        }
    };
    let current_by_tala = graph
        .nodes
        .iter()
        .enumerate()
        .map(|(index, node)| (node.tala_id, NodeId(index as u32)))
        .collect::<BTreeMap<_, _>>();
    let mut projected_aliases = graph
        .transaction_external_containers
        .iter()
        .chain(
            graph
                .transaction_external_container_children
                .values()
                .flatten(),
        )
        .chain(
            graph
                .transaction_external_aggregate_children
                .values()
                .flatten(),
        )
        .collect::<Vec<&ArenaNode>>();
    // Projected child snapshots retain the parent key in the transaction
    // inventory even when their flattened ArenaNode.container points at a
    // temporary carrier.  Mirror reachability follows the original shared
    // Node pointers, so use that durable parent identity here rather than the
    // carrier-local dense ID.
    let mut projected_parent_by_tala = BTreeMap::<i64, Option<i64>>::new();
    for (&parent_tala_id, children) in &graph.transaction_external_container_children {
        for child in children {
            projected_parent_by_tala
                .entry(child.tala_id)
                .or_insert(Some(parent_tala_id));
        }
    }
    for (&parent_tala_id, children) in &graph.transaction_external_aggregate_children {
        // An active aggregate member's effective container is the synthetic
        // vessel's owning container.  Go's Node.container() substitutes
        // Vessel.Container while a cluster/sequence is active. Preserve an
        // explicit nil parent for a top-level vessel instead of substituting
        // the vessel ID, which would create a spurious reachable-container
        // key and mirror its members as if they were nested under that key.
        let effective_parent = graph
            .transaction_external_container_children
            .iter()
            .find_map(|(owner, siblings)| {
                siblings
                    .iter()
                    .any(|candidate| candidate.tala_id == parent_tala_id)
                    .then_some(Some(*owner))
            })
            .or_else(|| {
                graph
                    .sequences
                    .iter()
                    .find(|sequence| sequence.vessel_tala_id == parent_tala_id)
                    .map(|sequence| {
                        sequence
                            .container
                            .map(|container| graph.nodes[container.0 as usize].tala_id)
                    })
            })
            .or_else(|| {
                graph
                    .clusters
                    .iter()
                    .find(|cluster| cluster.vessel_tala_id == parent_tala_id)
                    .and_then(|cluster| cluster.members.first())
                    .map(|member| {
                        graph
                            .active_node_container(*member)
                            .map(|container| graph.nodes[container.0 as usize].tala_id)
                    })
            })
            .or_else(|| {
                graph
                    .serialized_cluster_containers_by_tala
                    .get(&parent_tala_id)
                    .copied()
            })
            .unwrap_or(Some(parent_tala_id));
        for child in children {
            let member_effective_parent = graph
                .source_parent_tala_by_tala
                .get(&child.tala_id)
                .copied()
                .unwrap_or(effective_parent);
            projected_parent_by_tala
                .entry(child.tala_id)
                .or_insert(member_effective_parent);
        }
    }
    projected_aliases.sort_by_key(|node| node.tala_id);
    projected_aliases.dedup_by_key(|node| node.tala_id);
    let projected_tala_ids = projected_aliases
        .iter()
        .map(|node| node.tala_id)
        .collect::<BTreeSet<_>>();
    for node in projected_aliases {
        let parent = projected_parent_by_tala
            .get(&node.tala_id)
            .copied()
            .or_else(|| graph.source_parent_tala_by_tala.get(&node.tala_id).copied())
            .unwrap_or(node.scoring_container_parent);
        original_containers.entry(node.tala_id).or_insert(parent);
    }
    // Use the original endpoint identities captured at the adapter boundary;
    // the live edge slice may now point at hierarchy/aggregate carriers.
    for &(from_tala_id, to_tala_id) in &graph.source_edge_endpoints {
        let endpoint = |tala_id: i64| {
            current_by_tala
                .get(&tala_id)
                .copied()
                .map(MirrorReachNode::Current)
                .or_else(|| {
                    projected_tala_ids
                        .contains(&tala_id)
                        .then_some(MirrorReachNode::Original(tala_id))
                })
        };
        if let (Some(from), Some(to)) = (endpoint(from_tala_id), endpoint(to_tala_id)) {
            connect(from, to);
        }
    }
    // Restoring edge abductions reconnects the parent graph's visible edge
    // slice, but a collapsed component can still retain the original
    // endpoint edge only through the shared Node.Edges pointers.  Go's
    // AllReachableNodes follows those incident pointers during mirrorAxes;
    // the flattened arena records the same endpoint pair in this placement
    // scope's collapsed mirror inventory.  Include those links so a direct
    // mirror reaches nested containers (and reflects their children) even
    // when the edge itself is absent from Graph.Edges.
    for &(_, from_tala_id, _, to_tala_id, _) in &placement.collapsed_restored_mirror_edges {
        let endpoint = |tala_id: i64| {
            current_by_tala
                .get(&tala_id)
                .copied()
                .map(MirrorReachNode::Current)
                .or_else(|| {
                    projected_tala_ids
                        .contains(&tala_id)
                        .then_some(MirrorReachNode::Original(tala_id))
                })
        };
        if let (Some(from), Some(to)) = (endpoint(from_tala_id), endpoint(to_tala_id)) {
            connect(from, to);
        }
    }

    let mut queue = members
        .iter()
        .copied()
        .map(MirrorReachNode::Current)
        .collect::<VecDeque<_>>();
    let mut visited = BTreeSet::new();
    let mut containers = BTreeSet::new();
    while let Some(node) = queue.pop_front() {
        if !visited.insert(node) {
            continue;
        }
        match node {
            MirrorReachNode::Current(current) => {
                // A present metadata entry with `None` is an explicit
                // top-level container, matching Go's nil `EffectiveContainer`.
                // Do not replace that nil with the enclosing placement scope's
                // parent: the scope parent is only a fallback when this node
                // has no metadata entry (for example, an external carrier).
                let container = if let Some(cluster_index) = graph.active_cluster_index(current) {
                    let vessel_tala_id = graph.clusters[cluster_index].vessel_tala_id;
                    graph
                        .serialized_cluster_containers_by_tala
                        .get(&vessel_tala_id)
                        .copied()
                        .or_else(|| {
                            graph
                                .active_node_container(current)
                                .map(|id| Some(graph.nodes[id.0 as usize].tala_id))
                        })
                        .or_else(|| current_containers.get(&current).copied())
                        .unwrap_or(scope_parent)
                } else if let Some(sequence_index) = graph.active_sequence_index(current) {
                    graph.sequences[sequence_index]
                        .container
                        .map(|id| graph.nodes[id.0 as usize].tala_id)
                } else {
                    current_containers
                        .get(&current)
                        .copied()
                        .unwrap_or_else(|| {
                            graph
                                .active_node_container(current)
                                .map(|id| graph.nodes[id.0 as usize].tala_id)
                                .or_else(|| {
                                    graph
                                        .nodes
                                        .get(current.0 as usize)
                                        .and_then(|node| node.container)
                                        .map(|id| graph.nodes[id.0 as usize].tala_id)
                                })
                                .or(scope_parent)
                        })
                };
                containers.insert(container);
            }
            MirrorReachNode::Original(tala_id) => {
                if let Some(&container) = original_containers.get(&tala_id) {
                    containers.insert(container);
                }
            }
        }
        // `AllReachableNodes(includeContainers=true)` also queues visible
        // Graph.Nodes that are ancestors/descendants of the current node.
        // The induced placement slice contains only `members`, while the
        // edge walk above can reach retained nested endpoints; preserve the
        // same ancestry closure without treating every hidden arena node as
        // an independent root.
        if let MirrorReachNode::Current(current) = node {
            for &candidate in members {
                if visited.contains(&MirrorReachNode::Current(candidate)) {
                    continue;
                }
                if graph.active_is_descendant_of(current, candidate)
                    || graph.active_is_descendant_of(candidate, current)
                {
                    queue.push_back(MirrorReachNode::Current(candidate));
                }
            }
            // reachableNodesGuarded (layoutgraph/node.go:1767–1782) also
            // queues a cluster vessel's members and a sequence's steps, so
            // edges leaving those members reach further containers.
            let current_tala_id = graph.nodes.get(current.0 as usize).map(|node| node.tala_id);
            let aggregate_members = graph
                .clusters
                .iter()
                .filter(|cluster| Some(cluster.vessel_tala_id) == current_tala_id)
                .flat_map(|cluster| cluster.members.iter().copied())
                .chain(
                    graph
                        .sequences
                        .iter()
                        .filter(|sequence| Some(sequence.vessel_tala_id) == current_tala_id)
                        .flat_map(|sequence| sequence.members.iter().copied()),
                )
                .collect::<Vec<_>>();
            // A compact placement graph may carry only the vessel; its members
            // then survive as projected aggregate children.
            let projected_members = current_tala_id
                .and_then(|tala_id| graph.transaction_external_aggregate_children.get(&tala_id))
                .into_iter()
                .flatten()
                .map(|child| child.tala_id);
            let member_tala_ids = aggregate_members
                .into_iter()
                .filter_map(|member| graph.nodes.get(member.0 as usize).map(|node| node.tala_id))
                .chain(projected_members)
                .collect::<Vec<_>>();
            for tala_id in member_tala_ids {
                if let Some(&local) = current_by_tala.get(&tala_id) {
                    queue.push_back(MirrorReachNode::Current(local));
                } else if projected_tala_ids.contains(&tala_id) {
                    queue.push_back(MirrorReachNode::Original(tala_id));
                }
            }
        }
        queue.extend(adjacency.get(&node).into_iter().flatten().copied());
    }

    containers
}

impl Pipeline {
    /// Geometry of one original child inside the temporary aggregate vessel
    /// that owns it in `scope`.
    ///
    /// Recovered preprocessing installs sequences first and clusters second.
    /// Each later aggregate therefore lays out the preceding vessel as one
    /// member; the original child's offset is the sum of those nested member
    /// offsets.
    pub(super) fn scope_aggregate_member_geometry(
        &self,
        scope: Option<NodeId>,
        member: NodeId,
    ) -> Option<(NodeId, Point, Size)> {
        let mut owner = member;
        let mut offset = Point::default();
        let mut size = self.graph.nodes[member.0 as usize].rect.size;
        let mut aggregated = false;

        if let Some((sequence_index, sequence)) = self
            .graph
            .sequences
            .iter()
            .enumerate()
            .find(|(_, sequence)| sequence.container == scope && sequence.members.contains(&owner))
        {
            let (member_offset, member_size) = self
                .graph
                .sequence_member_geometry(sequence_index, member)?;
            offset = member_offset;
            size = member_size;
            owner = *sequence.members.first()?;
            aggregated = true;
        }

        for (cluster_index, cluster) in self.graph.clusters.iter().enumerate() {
            if !cluster.members.contains(&owner)
                || !cluster.members.iter().all(|cluster_member| {
                    self.graph.nodes[cluster_member.0 as usize].container == scope
                })
            {
                continue;
            }
            let (vessel_offset, member_size) =
                self.graph.cluster_member_geometry(cluster_index, owner)?;
            offset = Point {
                x: vessel_offset.x + offset.x,
                y: vessel_offset.y + offset.y,
            };
            if !aggregated {
                size = member_size;
            }
            owner = *cluster.members.first()?;
            aggregated = true;
        }

        aggregated.then_some((owner, offset, size))
    }

    /// Box of the cluster vessel retained by an original descendant endpoint,
    /// expressed relative to the direct carrier used by the parent placement
    /// graph.
    fn retained_cluster_vessel_geometry(
        &self,
        member: NodeId,
        carrier: NodeId,
        scope: Option<NodeId>,
        scope_owner: &BTreeMap<NodeId, NodeId>,
        old_to_new: &BTreeMap<NodeId, NodeId>,
    ) -> Option<ProjectedClusterDistance> {
        let cluster_index = self.graph.nodes[member.0 as usize].cluster?;
        let carrier_position = self.graph.position(carrier).unwrap_or_default();
        let cluster = &self.graph.clusters[cluster_index];
        // Cluster.CreateVessel anchors the temporary vessel at the member
        // minimum, but later Cluster.sync/repositioning can move that vessel
        // independently while the retained member boxes remain one pixel
        // away. The recovered edgeLength measures the vessel box, so preserve
        // the independent pending vessel position whenever one exists.
        let vessel_top_left = self
            .graph
            .pending_cluster_vessel_positions
            .get(&cluster_index)
            .copied()
            .or_else(|| {
                cluster
                    .members
                    .iter()
                    .filter_map(|member| self.graph.position(*member))
                    .fold(None, |minimum: Option<Point>, position| {
                        Some(match minimum {
                            Some(minimum) => Point {
                                x: minimum.x.min(position.x),
                                y: minimum.y.min(position.y),
                            },
                            None => position,
                        })
                    })
            })
            .unwrap_or(Point {
                x: f64::INFINITY,
                y: f64::INFINITY,
            });
        if !vessel_top_left.x.is_finite() || !vessel_top_left.y.is_finite() {
            return None;
        }
        if crate::internal::trace_env_enabled("WEFTAN_TRACE_CLUSTER_GEOMETRY")
            && cluster.vessel_tala_id == 6640668014774057861
        {
            eprint!(
                "CLUSTER_GEOMETRY_RUST vessel={} carrier={} min={:?} members=",
                cluster.vessel_tala_id,
                self.graph.nodes[carrier.0 as usize].tala_id,
                vessel_top_left,
            );
            for member in &cluster.members {
                eprint!(
                    "{}@{:?},",
                    self.graph.nodes[member.0 as usize].tala_id,
                    self.graph.position(*member),
                );
            }
            eprintln!();
        }
        let members = cluster.members.iter().copied().collect::<BTreeSet<_>>();
        // A decoded cluster retains its original vessel identity through the
        // placement boundary, so read its pointer-owned abductions directly
        // from that vessel's state.
        let serialized_abductions = self
            .graph
            .serialized_cluster_edge_abductions(cluster.vessel_tala_id);
        let external_tala_ids = if let Some(abductions) = serialized_abductions {
            // Go's clusterExternalConnectedNodes walks the live
            // Cluster.EdgeAbductions slice and selects CurrentFrom/CurrentTo
            // according to which original endpoint is nil. Using the graph's
            // edge inventory here loses that pointer-owned provenance (and
            // changes the result when the serialized abduction records differ
            // from the endpoint shape inferred from Graph.Edges).
            serialized_cluster_external_endpoint_ids(abductions)
        } else {
            // Clusters created during this layout have no serialized
            // EdgeAbductions slice. Reconstruct their external inventory from
            // the live graph edges, matching the native AddClusters path.
            let mut seen = BTreeSet::new();
            self.graph
                .member_edges()
                .filter_map(|edge| {
                    let from_in = members.contains(&edge.from);
                    let to_in = members.contains(&edge.to);
                    (from_in != to_in).then(|| {
                        let external = if from_in { edge.to } else { edge.from };
                        AggregateNodeRef::GraphNode {
                            node: external,
                            tala_id: self.graph.nodes[external.0 as usize].tala_id,
                        }
                    })
                })
                .filter(|endpoint| seen.insert(*endpoint))
                .collect::<Vec<_>>()
        };
        let mut seen = BTreeSet::new();
        let mut external_connected = Vec::new();
        for external_endpoint in external_tala_ids {
            let external_tala_id = external_endpoint.tala_id();
            if !seen.insert(external_tala_id) {
                continue;
            }
            let Some(external) = external_endpoint.graph_node().or_else(|| {
                self.graph
                    .nodes
                    .iter()
                    .position(|node| node.tala_id == external_tala_id)
                    .map(|index| NodeId(index as u32))
            }) else {
                // The abduction retains a separate aggregate-vessel
                // reference even when the stable arena has no NodeId for it.
                continue;
            };
            if serialized_abductions.is_some() && self.graph.position(external).is_none() {
                // clusterExternalConnectedNodes excludes unpositioned or
                // inactive current endpoints before projecting them.
                continue;
            }
            let Some(projected_owner) = self.projected_child_in_scope(external, scope) else {
                continue;
            };
            let projected_owner = scope_owner
                .get(&projected_owner)
                .copied()
                .unwrap_or(projected_owner);
            let Some(&owner) = old_to_new.get(&projected_owner) else {
                continue;
            };
            let offset = if external == projected_owner {
                Point::default()
            } else {
                let Some(external_position) = self.graph.position(external) else {
                    continue;
                };
                let owner_position = self.graph.position(projected_owner).unwrap_or_default();
                Point {
                    x: external_position.x - owner_position.x,
                    y: external_position.y - owner_position.y,
                }
            };
            external_connected.push(ProjectedAdjacent {
                owner,
                tala_id: self.graph.nodes[external.0 as usize].tala_id,
                container_tala_id: self.graph.nodes[external.0 as usize]
                    .container
                    .map(|container| self.graph.nodes[container.0 as usize].tala_id),
                offset,
                size: self.graph.nodes[external.0 as usize].rect.size,
                cluster_member: false,
            });
        }
        Some(ProjectedClusterDistance {
            offset: Point {
                x: vessel_top_left.x - carrier_position.x,
                y: vessel_top_left.y - carrier_position.y,
            },
            size: self.graph.cluster_vessel_size(cluster_index),
            arrangement: cluster.arrangement,
            vessel_tala_id: cluster.vessel_tala_id,
            external_connected,
        })
    }

    fn aggregate_member_geometry(
        &self,
        aggregate: PlacementAggregate,
        member: NodeId,
    ) -> Option<(Point, Size, bool)> {
        match aggregate {
            PlacementAggregate::Sequence(sequence) => self
                .graph
                .sequence_member_geometry(sequence, member)
                .map(|(offset, size)| (offset, size, false)),
            PlacementAggregate::Cluster(cluster) => self
                .graph
                .cluster_member_geometry(cluster, member)
                .map(|(offset, size)| (offset, size, true)),
        }
    }

    /// Direct translation of recovered `placeChildrenOrder`.
    ///
    /// `edge_abductions` preserves TALA's slice order. Neighbor sets are used
    /// only for degree; the breadth-first walk deliberately scans the original
    /// abduction slice for every visited node.
    pub(super) fn place_children_order(
        nodes: &[NodeId],
        edge_abductions: &[(NodeId, NodeId)],
    ) -> Vec<NodeId> {
        let mut neighbors = nodes
            .iter()
            .copied()
            .map(|node| (node, BTreeSet::new()))
            .collect::<BTreeMap<_, _>>();
        for &(from, to) in edge_abductions {
            if let Some(adjacent) = neighbors.get_mut(&from) {
                adjacent.insert(to);
            }
            if let Some(adjacent) = neighbors.get_mut(&to) {
                adjacent.insert(from);
            }
        }

        let mut ordered = Vec::with_capacity(nodes.len());
        for &node in nodes {
            if neighbors.get(&node).is_some_and(BTreeSet::is_empty) {
                ordered.push(node);
                neighbors.remove(&node);
            }
        }

        while ordered.len() != nodes.len() {
            let Some(start) = nodes
                .iter()
                .copied()
                .filter(|node| neighbors.contains_key(node))
                .min_by_key(|node| {
                    (
                        neighbors.get(node).map_or(usize::MAX, BTreeSet::len),
                        nodes
                            .iter()
                            .position(|candidate| candidate == node)
                            .unwrap_or(usize::MAX),
                    )
                })
            else {
                break;
            };

            let mut seen = BTreeSet::new();
            let mut queue = VecDeque::from([start]);
            while let Some(current) = queue.pop_front() {
                if !seen.insert(current) {
                    continue;
                }
                ordered.push(current);
                neighbors.remove(&current);
                for &(from, to) in edge_abductions {
                    if from == current {
                        queue.push_back(to);
                    } else if to == current {
                        queue.push_back(from);
                    }
                }
            }
        }
        ordered
    }

    pub(super) fn map_scope_nears(
        old_to_new: &BTreeMap<NodeId, NodeId>,
        assigned_nears: &[(NodeId, NodeId)],
    ) -> Vec<(NodeId, NodeId)> {
        let new_to_old = old_to_new
            .iter()
            .map(|(&old, &new)| (new, old))
            .collect::<BTreeMap<_, _>>();

        assigned_nears
            .iter()
            .filter_map(|&(first, second)| {
                Some((*new_to_old.get(&first)?, *new_to_old.get(&second)?))
            })
            .collect()
    }

    pub(super) fn retain_scope_nears(&mut self, assigned_nears: &[(NodeId, NodeId)]) {
        for &(first, second) in assigned_nears {
            if !self.graph.nodes[first.0 as usize].nears.contains(&second) {
                self.graph.nodes[first.0 as usize].nears.push(second);
            }
            if !self.graph.nodes[second.0 as usize].nears.contains(&first) {
                self.graph.nodes[second.0 as usize].nears.push(first);
            }
        }
    }

    pub(super) fn projected_child_in_scope(
        &self,
        mut node: NodeId,
        scope: Option<NodeId>,
    ) -> Option<NodeId> {
        match scope {
            None => {
                while let Some(parent) = self.graph.nodes[node.0 as usize].container {
                    node = parent;
                }
                Some(node)
            }
            Some(scope) => loop {
                let parent = self.graph.nodes[node.0 as usize].container?;
                if parent == scope {
                    return Some(node);
                }
                node = parent;
            },
        }
    }

    pub(super) fn scope_direction(&self, scope: Option<NodeId>) -> Direction {
        let mut current = scope;
        while let Some(node) = current {
            if let Some(direction) = self.graph.directions.get(&Some(node)) {
                return *direction;
            }
            current = self.graph.nodes[node.0 as usize].container;
        }
        self.graph.directions[&None]
    }

    /// stages.go Place installs these hints before any recursive AbductEdges.
    /// Keep the group's actual slice allocation shared by all selected keys.
    pub(super) fn placement_common_uncles(&self) -> SharedSliceMap {
        self.graph.compute_common_uncle_map()
    }

    /// Materializes the temporary `childrenGraph` used by recovered
    /// `Graph.placeNodes`. Direct children become roots and every edge whose
    /// endpoints cross child ownership is abducted onto those roots.
    pub(super) fn placement_scope_graph(&self, scope: Option<NodeId>) -> Option<PlacementScope> {
        self.placement_scope_graph_after(scope, &BTreeSet::new())
    }

    /// Source CreateVessel allocates a fresh node, never a clone of a member.
    /// Retain original members and their Near maps independently of the vessel.
    fn bind_placement_aggregate_vessels(
        &mut self,
        placement: &PlacementScope,
    ) -> Result<(), OwnerTopologyError> {
        for metadata in placement
            .scoring_node_metadata
            .iter()
            .filter(|m| m.is_aggregate_vessel)
        {
            let member = placement
                .old_to_new
                .iter()
                .find_map(|(&old, &local)| (local == metadata.node).then_some(old))
                .expect("aggregate member lineage");
            let kind = if let Some(index) = self.graph.nodes[member.0 as usize].cluster {
                TopologyVessel::Cluster(index)
            } else {
                TopologyVessel::Sequence(
                    self.graph.nodes[member.0 as usize]
                        .sequence
                        .expect("aggregate sequence lineage"),
                )
            };
            if let Some(vessel) = self.graph.topology_vessel_node(kind) {
                // Discovery allocates the pointer before placement. Refresh
                // the induced frame metadata just as for a newly bound vessel.
                let entity = &mut self.graph.nodes[vessel.0 as usize];
                entity.scoring_container_parent = metadata.parent_tala_id;
                entity.scoring_container_ancestors = metadata.container_ancestors.clone();
                entity.scoring_cluster_arrangement = metadata.cluster_arrangement;
                // The owning child scope has already positioned its numerical
                // carrier. Source shares that vessel pointer with the parent;
                // publish the completed frame before parent abduction captures
                // Containers aliases and relative offsets.
                if let TopologyVessel::Sequence(index) = kind
                    && let Some(position) = self.graph.position(member)
                {
                    Self::publish_induced_owner_point(&mut self.graph, vessel, Some(position));
                    let size = self.graph.sequence_vessel_size(index);
                    self.graph.nodes[vessel.0 as usize].rect.size = size;
                }
                continue;
            }
            let (members, group_kind) = match kind {
                TopologyVessel::Cluster(index) => (
                    self.graph.clusters[index].members.clone(),
                    super::owner_groups::OwnerGroupKind::Cluster,
                ),
                TopologyVessel::Sequence(index) => (
                    self.graph.sequences[index].members.clone(),
                    super::owner_groups::OwnerGroupKind::Sequence,
                ),
            };
            let projected = placement
                .graph
                .node(metadata.node)
                .expect("aggregate projection");
            let mut fresh = Graph::default();
            let local = fresh.add_node(crate::Node::new(
                projected.external_id.clone(),
                projected.size,
            ));
            fresh.tala_id_overrides[local.0 as usize] = metadata.tala_id;
            let mut entity = ArenaGraph::from_input(&fresh).nodes.remove(0);
            entity.container = metadata.parent_container;
            entity.position = match kind {
                TopologyVessel::Cluster(index) => self
                    .graph
                    .pending_cluster_vessel_positions
                    .get(&index)
                    .copied(),
                TopologyVessel::Sequence(_) => self.graph.nodes[member.0 as usize].position,
            };
            entity.scoring_container_parent = metadata.parent_tala_id;
            entity.scoring_container_ancestors = metadata.container_ancestors.clone();
            entity.scoring_cluster_arrangement = metadata.cluster_arrangement;
            let vessel = self.graph.retain_topology_node(entity);
            let group = self.graph.new_owner_group(group_kind);
            let nodes = self.graph.allocate_owner_member_header(
                &members.iter().copied().map(Some).collect::<Vec<_>>(),
                members.len(),
            )?;
            self.graph.set_owner_group_fields(
                group,
                super::shared_topology::OwnerGroupFieldView {
                    vessel: Some(vessel),
                    nodes,
                    graph: Some(OwnerGraphId(0)),
                    container: metadata.parent_container,
                },
            )?;
            for &member in &members {
                self.graph
                    .set_owner_node_group(member, group_kind, Some(group))?;
            }
            self.graph.bind_owner_group_reference(kind, group)?;
            self.graph.bind_topology_vessel(kind, vessel)?;
            if matches!(kind, TopologyVessel::Cluster(_)) {
                self.graph.set_owner_cluster_vessel_flag(vessel, true)?;
            }
            {
                let mut records = Vec::new();
                for &edge_id in &self.graph.edge_order.read() {
                    let edge = &self.graph.edge_records[edge_id.0 as usize];
                    let mut from = edge.from;
                    let mut to = edge.to;
                    // Sequence defining edges have already been disconnected.
                    // Other edges between two steps are retained unchanged;
                    // abductSequenceEdges skips both-member endpoints.
                    if matches!(kind, TopologyVessel::Sequence(_))
                        && members.contains(&from)
                        && members.contains(&to)
                    {
                        continue;
                    }
                    if members.contains(&from) {
                        records.push(OwnerEdgeAbduction {
                            source_edge: Some(edge_id),
                            originally_from: Some(from),
                            originally_to: None,
                            current_from: Some(vessel),
                            current_to: Some(to),
                        });
                        from = vessel;
                    }
                    if members.contains(&to) {
                        records.push(OwnerEdgeAbduction {
                            source_edge: Some(edge_id),
                            originally_from: None,
                            originally_to: Some(to),
                            current_from: Some(from),
                            current_to: Some(vessel),
                        });
                        to = vessel;
                    }
                    let _ = to;
                }
                let records = self.graph.register_owner_edge_abductions(records, true)?;
                self.graph.abduct_owner_edge_abductions(&records);
                let header = self.graph.allocate_owner_abduction_header(
                    &records
                        .records
                        .iter()
                        .copied()
                        .map(Some)
                        .collect::<Vec<_>>(),
                    records.records.len(),
                    !records.records.is_empty() || matches!(kind, TopologyVessel::Cluster(_)),
                )?;
                if let TopologyVessel::Cluster(index) = kind {
                    let cluster = &self.graph.clusters[index];
                    self.graph.set_owner_cluster_policy(
                        group,
                        super::owner_groups::OwnerClusterPolicy {
                            arrangement: format!("{:?}", cluster.arrangement),
                            desired_arrangement: format!("{:?}", cluster.desired_arrangement),
                            padding: cluster.padding,
                            fixed_size: cluster.fixed_size,
                            edge_abductions: Some(header),
                        },
                    )?;
                } else {
                    self.graph
                        .set_owner_sequence_abductions(group, Some(header))?;
                }
            }
            let map = self
                .graph
                .owner_graph_group_map(OwnerGraphId(0), group_kind)?;
            self.graph
                .write_owner_group_map_entry(&map, Some(vessel), Some(group))?;
            if matches!(kind, TopologyVessel::Cluster(_)) {
                self.graph.publish_owner_cluster_children(group)?;
            }
        }
        Ok(())
    }

    fn canonical_placement_node(
        &self,
        placement: &PlacementScope,
        old: NodeId,
        local: NodeId,
    ) -> Result<NodeId, OwnerTopologyError> {
        if self.graph.topology_node_vessel(old).is_some() {
            return Ok(old);
        }
        if placement
            .scoring_node_metadata
            .iter()
            .any(|m| m.node == local && m.is_aggregate_vessel)
        {
            let node = &self.graph.nodes[old.0 as usize];
            let kind = node
                .cluster
                .map(TopologyVessel::Cluster)
                .or_else(|| node.sequence.map(TopologyVessel::Sequence))
                .expect("aggregate member lineage");
            self.graph
                .topology_vessel_node(kind)
                .ok_or(OwnerTopologyError::UnboundVessel(kind))
        } else {
            Ok(old)
        }
    }

    /// Execute the owner-side half of CopyEntitiesFrom/AddNode/AbductEdges.
    /// Discovery uses the pure builder above; only actual placement calls
    /// prepare shared entities. Aggregate allocation is still represented by
    /// the older projected path until real distinct vessels are installed.
    pub(super) fn prepare_placement_scope_after(
        &mut self,
        root: Option<NodeId>,
        materialized: &BTreeSet<Option<NodeId>>,
    ) -> Result<Option<(PlacementScope, Option<PreparedOwnerScope>)>, crate::LayoutError> {
        self.graph.synchronize_original_owner_graph_view();
        // Source grouping precedes every parent AbductEdges call. Allocate
        // retained nested groups before resolving original endpoint pointers;
        // otherwise an unbound nested member forces the parent onto the
        // detached path and the child later sees live outside incidences.
        let group_scopes = self
            .graph
            .sequences
            .iter()
            .enumerate()
            .filter(|(index, group)| {
                self.graph.sequence_is_active(group)
                    && self
                        .graph
                        .topology_vessel_node(TopologyVessel::Sequence(*index))
                        .is_none()
            })
            .map(|(_, group)| group.container)
            .chain(
                self.graph
                    .clusters
                    .iter()
                    .enumerate()
                    .filter_map(|(index, group)| {
                        if !self.graph.cluster_is_active(group)
                            || self
                                .graph
                                .topology_vessel_node(TopologyVessel::Cluster(index))
                                .is_some()
                        {
                            return None;
                        }
                        group
                            .members
                            .first()
                            .map(|member| self.graph.nodes[member.0 as usize].container)
                    }),
            )
            .collect::<BTreeSet<_>>();
        for group_scope in group_scopes {
            if let Some(group_placement) =
                self.build_placement_scope_graph_after(group_scope, materialized)
            {
                self.bind_placement_aggregate_vessels(&group_placement)
                    .map_err(|error| crate::LayoutError::PlacementTransaction(error.to_string()))?;
            }
        }
        let Some(mut placement) = self.build_placement_scope_graph_after(root, materialized) else {
            return Ok(None);
        };
        let aggregate_input_endpoints = self
            .graph
            .edge_order
            .read()
            .iter()
            .map(|&edge| {
                let value = &self.graph.edge_records[edge.0 as usize];
                (edge, (value.from, value.to))
            })
            .collect::<BTreeMap<_, _>>();
        let aggregate_input_incidents = self
            .graph
            .nodes
            .iter()
            .map(|node| (node.input_id, node.edges.read()))
            .collect::<BTreeMap<_, _>>();
        self.bind_placement_aggregate_vessels(&placement)
            .map_err(|error| crate::LayoutError::PlacementTransaction(error.to_string()))?;
        self.graph.synchronize_original_owner_graph_view();
        // A restore may replace an original member pointer by its vessel.
        // Preflight both original and current sides before changing edges;
        // accepting a first-member carrier here would corrupt that identity.
        let unbound_original_vessel = placement.sized_edge_abductions.iter().any(|record| {
            [record.originally_from, record.originally_to]
                .into_iter()
                .flatten()
                .filter_map(|reference| reference.source_node)
                .any(|node| {
                    let node = &self.graph.nodes[node.0 as usize];
                    node.cluster.is_some_and(|index| {
                        self.graph
                            .topology_vessel_node(TopologyVessel::Cluster(index))
                            .is_none()
                    }) || node.sequence.is_some_and(|index| {
                        self.graph
                            .topology_vessel_node(TopologyVessel::Sequence(index))
                            .is_none()
                    })
                })
        });
        if unbound_original_vessel {
            return Ok(Some((placement, None)));
        }
        let abductions = match self
            .graph
            .from_sized_owner_abductions(&placement.sized_edge_abductions, true)
        {
            Ok(records) => records,
            Err(OwnerTopologyError::UnboundVessel(_)) => return Ok(Some((placement, None))),
            Err(error) => return Err(crate::LayoutError::PlacementTransaction(error.to_string())),
        };
        let mut roots = placement
            .old_to_new
            .iter()
            .filter(|(_, local)| !placement.tree_auxiliary_nodes.contains(local))
            .map(|(&old, &local)| {
                self.canonical_placement_node(&placement, old, local)
                    .map(|node| (local, node))
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| crate::LayoutError::PlacementTransaction(error.to_string()))?;
        roots.sort_by_key(|&(local, _)| local);
        roots.dedup();
        let roots = roots.into_iter().map(|(_, old)| old).collect::<Vec<_>>();
        let edges = self
            .graph
            .edge_order
            .read()
            .iter()
            .copied()
            .filter(|edge| {
                placement.edge_old_to_new.contains_key(edge)
                    || placement.tree_edge_old_to_new.contains_key(edge)
            })
            .collect();
        let view = self
            .graph
            .create_owner_graph_view(roots.clone(), edges)
            .map_err(|error| crate::LayoutError::PlacementTransaction(error.to_string()))?;
        self.graph
            .share_owner_proximity_maps(OwnerGraphId(0), view)
            .map_err(|error| crate::LayoutError::PlacementTransaction(error.to_string()))?;
        for kind in [
            super::owner_groups::OwnerGroupKind::Cluster,
            super::owner_groups::OwnerGroupKind::Sequence,
        ] {
            self.graph
                .copy_owner_graph_group_map_from(view, OwnerGraphId(0), kind)
                .map_err(|error| crate::LayoutError::PlacementTransaction(error.to_string()))?;
        }
        self.graph
            .copy_owner_graph_tree_roots_from(view, OwnerGraphId(0))
            .map_err(|error| crate::LayoutError::PlacementTransaction(error.to_string()))?;
        self.graph
            .copy_owner_graph_node_to_tree_from(view, OwnerGraphId(0))
            .map_err(|error| crate::LayoutError::PlacementTransaction(error.to_string()))?;
        let mapping = self.owner_score_mapping_for_placement(&placement)?;
        self.graph
            .set_owner_view_node_mapping(view, mapping.nodes().to_vec())
            .map_err(|error| crate::LayoutError::PlacementTransaction(error.to_string()))?;
        let mut ownership_journal = OwnerGraphOwnershipJournal::default();
        self.graph
            .set_owner_graph_reference(view, &roots, false, &mut ownership_journal)
            .map_err(|error| crate::LayoutError::PlacementTransaction(error.to_string()))?;
        // The compact builder still represents groups by member carriers.
        // Retain its explicit pre-allocation input; never rewrite actual
        // canonical endpoints merely to rebuild that numerical workspace.
        let mut construction_endpoints = aggregate_input_endpoints
            .into_iter()
            .filter(|(edge, endpoints)| {
                let value = &self.graph.edge_records[edge.0 as usize];
                *endpoints != (value.from, value.to)
            })
            .collect::<BTreeMap<_, _>>();
        let mut construction_incidents = aggregate_input_incidents
            .into_iter()
            .filter(|(node, edges)| *edges != self.graph.nodes[node.0 as usize].edges)
            .collect::<BTreeMap<_, _>>();
        for &id in &abductions.records {
            let record = *self
                .graph
                .owner_edge_abduction(id)
                .expect("prepared record");
            let source_edge = record
                .source_edge
                .expect("constructed abduction has an edge");
            let edge = &self.graph.edge_records[source_edge.0 as usize];
            construction_endpoints
                .entry(source_edge)
                .or_insert((edge.from, edge.to));
            for node in [
                record.originally_from,
                record.originally_to,
                record.current_from,
                record.current_to,
            ]
            .into_iter()
            .flatten()
            {
                construction_incidents.entry(node).or_insert_with(|| {
                    self.graph
                        .owner_node_incident_header(node)
                        .map(|header| {
                            (0..header.len())
                                .map(|i| {
                                    header
                                        .get(i)
                                        .expect("logical incident")
                                        .expect("production incident")
                                })
                                .collect()
                        })
                        .unwrap_or_else(|_| self.graph.nodes[node.0 as usize].edges.read())
                });
            }
        }
        self.graph.abduct_owner_edge_abductions(&abductions);
        #[cfg(feature = "diagnostic-traces")]
        {
            // Observation must read the post-abduction owning arena.
            placement.trace_owner_graph = self.graph.clone();
        }
        Ok(Some((
            placement,
            Some(PreparedOwnerScope {
                abductions,
                view,
                ownership_journal,
                construction_endpoints,
                construction_incidents,
            }),
        )))
    }

    pub(super) fn restore_prepared_owner_scope(
        &mut self,
        prepared: &PreparedOwnerScope,
    ) -> Result<(), crate::LayoutError> {
        self.graph.synchronize_original_owner_graph_view();
        let roots = self
            .graph
            .owner_graph_view(OwnerGraphId(0))
            .expect("published original owner graph")
            .nodes
            .require_nodes()
            .collect::<Vec<_>>();
        #[cfg(feature = "diagnostic-traces")]
        let observe = |phase: &str, graph: &ArenaGraph| {
            if std::env::var_os("D2RS_TRACE_OWNER_RESTORE").is_some() {
                let ids = |nodes: &[NodeId]| {
                    nodes
                        .iter()
                        .map(|node| graph.nodes[node.0 as usize].tala_id.to_string())
                        .collect::<Vec<_>>()
                };
                let closure = graph
                    .topology_retained_closure(&roots)
                    .expect("diagnostic restore closure");
                eprintln!(
                    "{}",
                    serde_json::json!({
                        "event": "owner-restore", "phase": phase,
                        "roots": ids(&roots), "closure": ids(&closure),
                        "edges": graph.edge_order.read().iter().map(|id| {
                            let e = &graph.edge_records[id.0 as usize];
                            serde_json::json!({"id": e.tala_id.to_string(), "from": graph.nodes[e.from.0 as usize].tala_id.to_string(), "to": graph.nodes[e.to.0 as usize].tala_id.to_string()})
                        }).collect::<Vec<_>>()
                    })
                );
            }
        };
        #[cfg(feature = "diagnostic-traces")]
        observe("before", &self.graph);
        // Ordinary placeNodes ignores the returned remaining slice here.
        // Hierarchy placement has a different lifetime and consumes it.
        let _remaining = self
            .graph
            .restore_owner_edge_abductions(&prepared.abductions, &roots)
            .map_err(|error| crate::LayoutError::PlacementTransaction(error.to_string()))?;
        let mut journal = OwnerGraphOwnershipJournal::default();
        self.graph
            .set_owner_graph_reference(OwnerGraphId(0), &roots, true, &mut journal)
            .map_err(|error| crate::LayoutError::PlacementTransaction(error.to_string()))?;
        #[cfg(feature = "diagnostic-traces")]
        observe("after", &self.graph);
        Ok(())
    }

    /// The ordinary Go defer restores owner abductions even when placement
    /// returns an error. Keep that lifetime around the native scope solver.
    pub(super) fn solve_prepared_owner_scope(
        &mut self,
        placement: &PlacementScope,
        owner: Option<&PreparedOwnerScope>,
    ) -> Result<FlatScopePlacement, crate::LayoutError> {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.solve_prepared_owner_scope_inner(placement, owner)
        }));
        match result {
            Ok(Ok(placed)) => Ok(placed),
            Ok(Err(error)) => {
                if let Some(owner) = owner {
                    self.restore_prepared_owner_scope(owner)?;
                }
                Err(error)
            }
            Err(payload) => {
                if let Some(owner) = owner {
                    self.restore_prepared_owner_scope(owner)
                        .expect("canonical owning abduction unwind must restore");
                }
                std::panic::resume_unwind(payload)
            }
        }
    }

    pub(super) fn solve_prepared_owner_scope_inner(
        &mut self,
        placement: &PlacementScope,
        owner: Option<&PreparedOwnerScope>,
    ) -> Result<FlatScopePlacement, crate::LayoutError> {
        if let Some(prepared) = owner {
            let mapping = self.owner_score_mapping_for_placement(placement)?;
            let mut session = OwnerScoreSession::new(
                &mut self.graph,
                prepared.view,
                mapping,
                Some(&prepared.abductions),
            )
            .map_err(|error| crate::LayoutError::PlacementTransaction(error.to_string()))?;
            let placed = Self::place_flat_scope_with_owner_session(
                placement,
                self.seed,
                true,
                Some(&mut session),
            )?;
            session
                .commit_scope_ownership()
                .map_err(|error| crate::LayoutError::PlacementTransaction(error.to_string()))?;
            Ok(placed)
        } else {
            Self::place_flat_scope(placement, self.seed, true)
        }
    }

    /// proximity/nears.go: parent abductions retain the original descendants
    /// after their incident edges have moved to the outer carrier. Publish
    /// replacement Nears maps on the actual children before SplitSubgraphs.
    pub(super) fn assign_parent_abduction_nears(
        &mut self,
        root: NodeId,
        receiver: OwnerGraphId,
        parent: &PreparedOwnerScope,
    ) -> Result<(), crate::LayoutError> {
        let context = gap_guard::ActiveContext::new();
        let error = |e: limits::Error| crate::LayoutError::PlacementTransaction(e.to_string());
        let mut guard =
            limits::WorkGuard::new(Some(&context), "AssignNears", limits::MAX_ENGINE_WORK_UNITS)
                .map_err(error)?;
        let children = self
            .graph
            .owner_container_map(receiver)
            .ok_or_else(|| crate::LayoutError::PlacementTransaction("unknown near graph".into()))?
            .get(Some(root))
            .map(|header| {
                (0..header.len())
                    .map(|i| header.get(i).expect("logical child slot"))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let vessel = |node: Option<NodeId>| -> Result<Option<NodeId>, crate::LayoutError> {
            let Some(node) = node else { return Ok(None) };
            let live = &self.graph.nodes[node.0 as usize];
            let kind = live
                .cluster
                .map(TopologyVessel::Cluster)
                .or_else(|| live.sequence.map(TopologyVessel::Sequence));
            kind.map(|kind| {
                self.graph.topology_vessel_node(kind).ok_or_else(|| {
                    crate::LayoutError::PlacementTransaction(
                        "AssignNears requires a bound group vessel".into(),
                    )
                })
            })
            .transpose()
            .map(|bound| bound.or(Some(node)))
        };
        let descendant = |mut node: Option<NodeId>,
                          child: Option<NodeId>,
                          guard: &mut limits::WorkGuard<'_>|
         -> Result<bool, crate::LayoutError> {
            loop {
                guard.step().map_err(error)?;
                if node == child {
                    return Ok(true);
                }
                let Some(current) = node else {
                    return Ok(false);
                };
                let live = &self.graph.nodes[current.0 as usize];
                node = if live.container.is_some() {
                    live.container
                } else if live.cluster.is_some() || live.sequence.is_some() {
                    vessel(Some(current))?
                } else {
                    None
                };
            }
        };
        let mut uncles = BTreeMap::<NodeId, BTreeSet<NodeId>>::new();
        for &id in &parent.abductions.records {
            guard.step().map_err(error)?;
            let record = *self
                .graph
                .owner_edge_abduction(id)
                .expect("live parent record");
            let from = vessel(record.originally_from)?;
            let to = vessel(record.originally_to)?;
            for &child in &children {
                guard.step().map_err(error)?;
                // Source evaluates both walks before selecting the matched
                // endpoint; the second walk still charges/polls on a from match.
                let from_descendant = descendant(from, child, &mut guard)?;
                let to_descendant = descendant(to, child, &mut guard)?;
                let uncle = if from_descendant {
                    Some(record.current_to)
                } else if to_descendant {
                    Some(record.current_from)
                } else {
                    None
                };
                if let Some(uncle) = uncle {
                    if let (Some(uncle), Some(child)) = (uncle, child) {
                        uncles.entry(uncle).or_default().insert(child);
                    }
                    break;
                }
            }
        }
        let mut ordered = Vec::new();
        for &uncle in uncles.keys() {
            guard.step().map_err(error)?;
            ordered.push(uncle);
        }
        ordered.sort_by_key(|node| self.graph.nodes[node.0 as usize].tala_id);
        let mut original = BTreeMap::new();
        let mut replacements = BTreeMap::<NodeId, super::node_near_map::NodeNearMap>::new();
        let mut references = 0usize;
        for uncle in ordered {
            let mut nodes = Vec::new();
            for &node in &uncles[&uncle] {
                guard.step().map_err(error)?;
                nodes.push(node);
            }
            nodes.sort_by_key(|node| self.graph.nodes[node.0 as usize].tala_id);
            for (index, &first) in nodes.iter().enumerate() {
                for &second in &nodes[index + 1..] {
                    guard.step().map_err(error)?;
                    if self.graph.nodes[first.0 as usize].hierarchy.is_some()
                        || self.graph.nodes[second.0 as usize].hierarchy.is_some()
                    {
                        continue;
                    }
                    let incident = self.graph.owner_node_incident_header(first).ok();
                    let edges = if let Some(header) = incident {
                        (0..header.len())
                            .map(|i| header.get(i).expect("live incident slot"))
                            .collect::<Vec<_>>()
                    } else {
                        self.graph.nodes[first.0 as usize]
                            .edges
                            .read()
                            .iter()
                            .copied()
                            .map(Some)
                            .collect()
                    };
                    let mut connected = false;
                    for edge in edges {
                        guard.step().map_err(error)?;
                        if let Some(edge) = edge {
                            let edge = &self.graph.edge_records[edge.0 as usize];
                            let adjacent = if edge.from == first {
                                edge.to
                            } else {
                                edge.from
                            };
                            if adjacent == second {
                                connected = true;
                                break;
                            }
                        }
                    }
                    if connected {
                        continue;
                    }
                    for node in [first, second] {
                        if replacements.contains_key(&node) {
                            continue;
                        }
                        let map = self
                            .graph
                            .owner_node_near_map(node)
                            .expect("actual child Nears");
                        if map.len() > 1_000_000usize.saturating_sub(references) {
                            return Err(crate::LayoutError::PlacementTransaction(
                                "TALA AssignNears topology references exceed limit 1000000".into(),
                            ));
                        }
                        references += map.len();
                        original.insert(node, map.retained());
                        let replacement = super::node_near_map::NodeNearMap::allocated();
                        for key in map.raw_keys() {
                            guard.step().map_err(error)?;
                            replacement.insert(key).expect("allocated map");
                        }
                        replacements.insert(node, replacement);
                    }
                    for (node, near) in [(first, second), (second, first)] {
                        if !replacements[&node].contains(&near) {
                            if references >= 1_000_000 {
                                return Err(crate::LayoutError::PlacementTransaction(
                                    "TALA AssignNears topology references exceed limit 1000000"
                                        .into(),
                                ));
                            }
                            replacements.get_mut(&node).unwrap().push(near);
                            references += 1;
                        }
                    }
                }
            }
        }
        guard.finish().map_err(error)?;
        let mut commit = replacements.keys().copied().collect::<Vec<_>>();
        commit.sort_by_key(|node| self.graph.nodes[node.0 as usize].tala_id);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            for node in commit {
                self.graph
                    .set_owner_node_near_map(node, replacements.remove(&node).unwrap())
                    .map_err(|e| crate::LayoutError::PlacementTransaction(e.to_string()))?;
                guard.finish().map_err(error)?;
            }
            Ok(())
        }));
        if !matches!(&result, Ok(Ok(()))) {
            for (node, map) in original {
                self.graph
                    .set_owner_node_near_map(node, map)
                    .expect("restore original Nears header");
            }
        }
        match result {
            Ok(result) => result,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    /// Rebuild value projections after child layout using this frame's saved
    /// construction inputs. The builder is synchronous and does not call into
    /// layout or user contexts; canonical abductions resume before it returns.
    pub(super) fn refresh_prepared_placement_scope(
        &mut self,
        root: Option<NodeId>,
        materialized: &BTreeSet<Option<NodeId>>,
        prepared: &PreparedOwnerScope,
    ) -> Option<PlacementScope> {
        self.with_prepared_construction_inputs(prepared, |builder| {
            builder.build_placement_scope_graph_after(root, materialized)
        })
    }

    fn with_prepared_construction_inputs<R>(
        &mut self,
        prepared: &PreparedOwnerScope,
        build: impl FnOnce(&Self) -> R,
    ) -> R {
        // Allocate replacements and validate indices before changing anything.
        // Move the original field, rather than assigning its shared header:
        // retained field/header aliases must keep the canonical incidence.
        let endpoints = prepared
            .construction_endpoints
            .iter()
            .map(|(&id, &(from, to))| {
                let edge = &self.graph.edge_records[id.0 as usize];
                (id, (edge.from, edge.to), (from, to))
            })
            .collect::<Vec<_>>();
        let incidents = prepared
            .construction_incidents
            .iter()
            .map(|(&id, edges)| {
                let _node = &self.graph.nodes[id.0 as usize];
                (id, IncidentEdges::literal(edges), edges.clone())
            })
            .collect::<Vec<_>>();
        // ArenaNode::clone retains HierarchyMembership records. Preserve the
        // numerical graph copy's detached hierarchy records, including aliases
        // between nodes and external projections, without copying other state.
        let original_hierarchies = self
            .graph
            .nodes
            .iter()
            .chain(&self.graph.transaction_external_containers)
            .chain(&self.graph.transaction_validation_external_containers)
            .chain(
                self.graph
                    .projected_transaction_nodes
                    .iter()
                    .map(|p| &p.node),
            )
            .chain(
                self.graph
                    .transaction_external_container_children
                    .values()
                    .chain(self.graph.transaction_external_aggregate_children.values())
                    .chain(self.graph.trace_container_children.values())
                    .flatten(),
            )
            .map(|node| node.hierarchy.clone())
            .collect::<Vec<_>>();
        let mut original_incidents = Vec::with_capacity(incidents.len());
        for (id, replacement, order) in incidents {
            let original =
                std::mem::replace(&mut self.graph.nodes[id.0 as usize].edges, replacement);
            let original_order = self.graph.incident_edge_order.insert(id, order);
            original_incidents.push((id, original, original_order));
        }
        for &(id, _, (from, to)) in &endpoints {
            let edge = &mut self.graph.edge_records[id.0 as usize];
            edge.from = from;
            edge.to = to;
        }
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.graph.detach_hierarchy_storage();
            build(self)
        }));
        for (node, hierarchy) in self
            .graph
            .nodes
            .iter_mut()
            .chain(&mut self.graph.transaction_external_containers)
            .chain(&mut self.graph.transaction_validation_external_containers)
            .chain(
                self.graph
                    .projected_transaction_nodes
                    .iter_mut()
                    .map(|p| &mut p.node),
            )
            .chain(
                self.graph
                    .transaction_external_container_children
                    .values_mut()
                    .chain(
                        self.graph
                            .transaction_external_aggregate_children
                            .values_mut(),
                    )
                    .chain(self.graph.trace_container_children.values_mut())
                    .flatten(),
            )
            .zip(original_hierarchies)
        {
            node.hierarchy = hierarchy;
        }
        for (id, original, original_order) in original_incidents {
            self.graph.nodes[id.0 as usize].edges = original;
            if let Some(order) = original_order {
                self.graph.incident_edge_order.insert(id, order);
            } else {
                self.graph.incident_edge_order.remove(&id);
            }
        }
        for (id, (from, to), _) in endpoints {
            let edge = &mut self.graph.edge_records[id.0 as usize];
            edge.from = from;
            edge.to = to;
        }
        match result {
            Ok(value) => value,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    /// Invert each explicit source-to-workspace map independently. A node
    /// mapping cannot establish edge identity, and compact indices/Tala IDs
    /// are not canonical pointers. Retained tree slots remain storage entries
    /// independently of the prepared view's member slice.
    fn owner_score_mapping_for_placement(
        &self,
        placement: &PlacementScope,
    ) -> Result<CanonicalScoreMapping, crate::LayoutError> {
        let binding_error =
            |message: &str| crate::LayoutError::PlacementTransaction(message.to_owned());
        let mut nodes = vec![None; placement.graph.nodes.len()];
        for (&old, &local) in &placement.old_to_new {
            let canonical = self
                .canonical_placement_node(placement, old, local)
                .map_err(|error| crate::LayoutError::PlacementTransaction(error.to_string()))?;
            let slot = nodes
                .get_mut(local.0 as usize)
                .ok_or_else(|| binding_error("owner scope node mapping is out of range"))?;
            if slot.replace(canonical).is_some_and(|old| old != canonical) {
                return Err(binding_error("owner scope node mapping is ambiguous"));
            }
        }
        let mut edges = vec![None; placement.graph.edges.len()];
        for (&canonical, &local) in placement
            .edge_old_to_new
            .iter()
            .chain(&placement.tree_edge_old_to_new)
        {
            let slot = edges
                .get_mut(local.0 as usize)
                .ok_or_else(|| binding_error("owner scope edge mapping is out of range"))?;
            if slot.replace(canonical).is_some_and(|old| old != canonical) {
                return Err(binding_error("owner scope edge mapping is ambiguous"));
            }
        }
        let nodes = nodes
            .into_iter()
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| binding_error("owner scope has an unbound node slot"))?;
        let edges = edges
            .into_iter()
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| binding_error("owner scope has an unbound edge slot"))?;
        let arena = self
            .graph
            .topology_arena_id()
            .ok_or_else(|| binding_error("owner scope has no canonical arena"))?;
        Ok(CanonicalScoreMapping::new(arena, nodes, edges))
    }

    /// Builds `childrenGraph` with the container positions that exist at this
    /// exact point in the recovered recursive walk.
    ///
    /// `Graph.placeNodes` creates a parent's `childrenGraph` before recursing.
    /// Fitting a child scope gives that carrier its size but not its TopLeft;
    /// only optimizing the parent scope positions the carrier. Containers
    /// below a scope that has already completed are therefore materialized,
    /// while ordinary direct children and unvisited sibling branches remain
    /// nil even if an adapter-side preliminary snapshot positioned them.
    pub(super) fn placement_scope_graph_after(
        &self,
        scope: Option<NodeId>,
        materialized_parent_scopes: &BTreeSet<Option<NodeId>>,
    ) -> Option<PlacementScope> {
        let mut placement =
            self.build_placement_scope_graph_after(scope, materialized_parent_scopes)?;
        placement.owner_source = Some(std::sync::Arc::new(PlacementOwnerSource {
            input: self.input.clone(),
            graph: self.graph.clone(),
            root: scope,
            materialized: materialized_parent_scopes.clone(),
            source_orientation_disabled_for_loop: self.source_orientation_disabled_for_loop,
        }));
        Some(placement)
    }

    pub(super) fn build_placement_scope_graph_after(
        &self,
        scope: Option<NodeId>,
        materialized_parent_scopes: &BTreeSet<Option<NodeId>>,
    ) -> Option<PlacementScope> {
        // Graph.Containers retains real vessel pointers. The numerical
        // workspace needs the group's input-member lineage to construct its
        // existing dense carrier; expand only here, never in the shared map
        // or the Source obstruction scratch headers.
        let children = self
            .graph
            .containers
            .get(&scope)?
            .into_iter()
            .flat_map(|child| match self.graph.topology_node_vessel(child) {
                Some(TopologyVessel::Cluster(index)) => self.graph.clusters[index].members.clone(),
                Some(TopologyVessel::Sequence(index)) => {
                    self.graph.sequences[index].members.clone()
                }
                _ => vec![child],
            })
            .collect::<Vec<_>>();
        if children.is_empty() {
            return None;
        }

        if crate::internal::trace_env_enabled("WEFTAN_TRACE_PLACEMENT_CONTAINER_ORDER") {
            eprintln!(
                "PLACEMENT_CONTAINER_RUST root={:?} nodes={:?}",
                scope.map(|node| self.graph.nodes[node.0 as usize].tala_id),
                children
                    .iter()
                    .map(|node| self.graph.nodes[node.0 as usize].tala_id)
                    .collect::<Vec<_>>()
            );
        }

        // AddSequences and AddClusters replace same-scope members with
        // temporary vessels before Graph.placeNodes builds its children graph.
        // Keep stable IDs in the main arena, but project each aggregate through
        // its first member as the adapter-owned vessel carrier.
        let mut scope_owner = children
            .iter()
            .copied()
            .map(|child| (child, child))
            .collect::<BTreeMap<_, _>>();
        let mut aggregate_by_owner = BTreeMap::<NodeId, PlacementAggregate>::new();
        for (sequence_index, sequence) in self.graph.sequences.iter().enumerate() {
            if sequence.members.len() < 2
                || sequence.container != scope
                || !sequence
                    .members
                    .iter()
                    .all(|member| children.contains(member))
            {
                continue;
            }
            let owner = sequence.members[0];
            aggregate_by_owner.insert(owner, PlacementAggregate::Sequence(sequence_index));
            for member in &sequence.members {
                scope_owner.insert(*member, owner);
            }
            if let Some(vessel) = self
                .graph
                .topology_vessel_node(TopologyVessel::Sequence(sequence_index))
            {
                scope_owner.insert(vessel, owner);
            }
        }
        // A recursively fitted container can remain in the owning
        // Graph.Containers map after Go's SplitSubgraphs has removed it from
        // the current Graph.Nodes slice.  The optimizer must still see its
        // live box as an external transaction carrier, while the container
        // itself must not become a second active obstacle.  Go makes this
        // distinction for materialized containers that have no edge crossing
        // their boundary; containers with an outside edge stay active so
        // their routing obstruction is scored normally.
        let contains_node = |candidate: NodeId, ancestor: NodeId| {
            let mut current = Some(candidate);
            while let Some(node) = current {
                if node == ancestor {
                    return true;
                }
                current = self.graph.nodes[node.0 as usize].container;
            }
            false
        };
        let has_edge_outside = |container: NodeId| {
            self.graph.member_edges().any(|edge| {
                let from_inside = contains_node(edge.from, container);
                let to_inside = contains_node(edge.to, container);
                from_inside != to_inside
            })
        };
        let has_hierarchy_descendant = |container: NodeId| {
            self.graph
                .nodes
                .iter()
                .enumerate()
                .any(|(index, candidate)| {
                    candidate.hierarchy.is_some()
                        && NodeId(index as u32) != container
                        && contains_node(NodeId(index as u32), container)
                })
        };
        let has_internal_edge = |container: NodeId| {
            self.graph.member_edges().any(|edge| {
                contains_node(edge.from, container)
                    && contains_node(edge.to, container)
                    && edge.from != container
                    && edge.to != container
            })
        };
        let externalized_materialized_container = |node_id: NodeId| {
            let node = &self.graph.nodes[node_id.0 as usize];
            let result = node.is_container
                && node.position.is_some()
                // A positioned materialized container is an external carrier
                // only while validating the root projection. Nested scopes
                // already carry their parent vessel through the owning scope;
                // adding the same alias there applies its translation twice
                // during copy-back (notably in pre-positioned clusters).
                && scope.is_none()
                && node.container.is_none()
                && !crate::internal::trace_env_enabled("WEFTAN_DISABLE_MATERIALIZED_EXTERNAL")
                && materialized_parent_scopes.contains(&Some(node_id))
                && !has_edge_outside(node_id);
            if result && crate::internal::trace_env_enabled("WEFTAN_TRACE_EXTERNAL_DETAILS") {
                eprintln!(
                    "EXTERNALIZED_RUST scope={:?} node={} position={:?}",
                    scope.map(|id| self.graph.nodes[id.0 as usize].tala_id),
                    node.tala_id,
                    node.position
                );
            }
            result
        };
        for (cluster_index, cluster) in self.graph.clusters.iter().enumerate() {
            if cluster.members.len() < 2
                || !cluster
                    .members
                    .iter()
                    .all(|member| children.contains(member))
            {
                continue;
            }
            let owner = cluster.members[0];
            aggregate_by_owner.insert(owner, PlacementAggregate::Cluster(cluster_index));
            for member in &cluster.members {
                scope_owner.insert(*member, owner);
            }
            if let Some(vessel) = self
                .graph
                .topology_vessel_node(TopologyVessel::Cluster(cluster_index))
            {
                scope_owner.insert(vessel, owner);
            }
        }
        // AddSequence/AddCluster remove every absorbed member and append the
        // vessel. Sequences are installed before clusters, so the concrete
        // graph order is ordinary children, sequence vessels, cluster vessels.
        let aggregate_owners = aggregate_by_owner.keys().copied().collect::<BTreeSet<_>>();
        if crate::internal::trace_env_enabled("WEFTAN_TRACE_SCOPE_AGGREGATES") {
            eprintln!(
                "SCOPE_AGGREGATES_RUST scope=370 owners={:?} sequences={:?} clusters={:?}",
                aggregate_by_owner
                    .iter()
                    .map(|(owner, aggregate)| (
                        self.graph.nodes[owner.0 as usize].tala_id,
                        aggregate
                    ))
                    .collect::<Vec<_>>(),
                self.graph
                    .sequences
                    .iter()
                    .map(|sequence| (
                        sequence.vessel_tala_id,
                        sequence
                            .container
                            .map(|node| self.graph.nodes[node.0 as usize].tala_id),
                        sequence
                            .members
                            .iter()
                            .map(|node| self.graph.nodes[node.0 as usize].tala_id)
                            .collect::<Vec<_>>()
                    ))
                    .collect::<Vec<_>>(),
                self.graph
                    .clusters
                    .iter()
                    .map(|cluster| (
                        cluster.vessel_tala_id,
                        cluster
                            .members
                            .iter()
                            .map(|node| self.graph.nodes[node.0 as usize].tala_id)
                            .collect::<Vec<_>>()
                    ))
                    .collect::<Vec<_>>()
            );
        }
        let mut scope_children = children
            .iter()
            .copied()
            .filter(|child| scope_owner[child] == *child && !aggregate_owners.contains(child))
            .collect::<Vec<_>>();
        scope_children.extend(
            self.graph
                .sequences
                .iter()
                .filter(|sequence| {
                    sequence.container == scope
                        && sequence
                            .members
                            .iter()
                            .all(|member| children.contains(member))
                })
                .filter_map(|sequence| sequence.members.first().copied()),
        );
        scope_children.extend(
            self.graph
                .clusters
                .iter()
                .filter(|cluster| {
                    cluster
                        .members
                        .iter()
                        .all(|member| children.contains(member))
                })
                .filter_map(|cluster| cluster.members.first().copied()),
        );
        if crate::internal::trace_env_enabled("WEFTAN_TRACE_SCOPE_AGGREGATES") {
            eprintln!(
                "SCOPE_CHILDREN_RUST scope={:?} children={:?} aggregates={:?}",
                scope.map(|node| self.graph.nodes[node.0 as usize].tala_id),
                scope_children
                    .iter()
                    .map(|node| self.graph.nodes[node.0 as usize].tala_id)
                    .collect::<Vec<_>>(),
                aggregate_by_owner
                    .iter()
                    .map(|(owner, aggregate)| (
                        self.graph.nodes[owner.0 as usize].tala_id,
                        aggregate
                    ))
                    .collect::<Vec<_>>()
            );
        }
        for child in &children {
            debug_assert!(scope_children.contains(&scope_owner[child]));
        }
        let direction = self.scope_direction(scope);
        let mut graph = Graph {
            direction,
            explicit_direction: scope
                .and_then(|container| self.input.node(container)?.direction)
                .or(self.input.explicit_direction.filter(|_| scope.is_none())),
            source_positions_by_tala: self.graph.source_positions_by_tala.clone(),
            source_parent_tala_by_tala: self.graph.source_parent_tala_by_tala.clone(),
            ..Graph::default()
        };
        let mut projected_edges = Vec::new();
        let mut detached_container_edges = Vec::<(EdgeId, NodeId)>::new();
        let mut collapsed_symmetry_edges = Vec::<(NodeId, NodeId, NodeId)>::new();
        let mut collapsed_table_column_edges = BTreeMap::<EdgeId, NodeId>::new();
        let mut collapsed_restored_mirror_edges =
            Vec::<(NodeId, i64, Option<NodeId>, i64, Option<NodeId>)>::new();
        let mut edge_abductions = Vec::new();
        let mut children_by_uncle = BTreeMap::<NodeId, Vec<NodeId>>::new();
        let mut children_by_exact_uncle = BTreeMap::<NodeId, Vec<NodeId>>::new();
        let mut abducted_children = BTreeSet::new();
        // `CopyEntitiesFrom` aliases each original Go Node pointer. A
        // temporary children graph omits an edge whose other endpoint belongs
        // outside this scope, but the direct child still sees that edge in
        // `Node.Edges` while ExtractTrees tests fringe degree. Preserve that
        // non-routable incident count on the projected node.
        let mut projected_external_edge_counts = BTreeMap::<NodeId, usize>::new();
        // Graph.abductEdges mutates the shared Node.Edges slices through
        // Edge.reconnect while it walks Graph.Edges. Keep a local copy of
        // those pointer-visible endpoint slices so the temporary scope sees
        // the same remove/append order without mutating the owning arena.
        let mut projected_endpoint_edge_order = self
            .graph
            .nodes
            .iter()
            .map(|node| (node.input_id, node.edges.read()))
            .collect::<BTreeMap<_, _>>();
        let extracted_tree_edges = self
            .graph
            .tree_routing_nodes
            .values()
            .map(|tree| tree.sentinel_edge)
            .collect::<BTreeSet<_>>();
        let parent_scope =
            scope.and_then(|container| self.graph.nodes[container.0 as usize].container);
        if crate::internal::trace_env_enabled("WEFTAN_TRACE_PLACEMENT_EDGES") {
            let root_tala_id = scope
                .map(|node| self.graph.nodes[node.0 as usize].tala_id)
                .unwrap_or(0);
            eprint!("PLACEMENT_INPUT_EDGES_RUST root={root_tala_id}");
            for edge_id in self.graph.edge_order.read().iter().copied() {
                let edge = &self.graph.edge_records[edge_id.0 as usize];
                eprint!(
                    " [{}]{}>{}",
                    edge_id.0,
                    self.graph.nodes[edge.from.0 as usize].tala_id,
                    self.graph.nodes[edge.to.0 as usize].tala_id
                );
            }
            eprintln!();
        }
        for edge_id in self.graph.edge_order.read().iter().copied() {
            // Go CopyEntitiesFrom keeps the same Edge pointer. A parent's
            // active abduction can therefore have changed its endpoints
            // before this scope is prepared; authored input is not topology.
            let owner_edge = &self.graph.edge_records[edge_id.0 as usize];
            let edge = crate::Edge {
                source: owner_edge.from,
                target: owner_edge.to,
            };
            // ExtractTrees removes every peeled node from Graph.Nodes and
            // disconnects its sentinel edge. Only reconnectTree restores
            // non-branching trees before placeNodes builds childrenGraph;
            // branching trees remain represented by Graph.Trees until
            // subgraph.PlaceTrees reconnects them.
            let source = self
                .projected_child_in_scope(edge.source, scope)
                .map(|child| scope_owner.get(&child).copied().unwrap_or(child));
            let target = self
                .projected_child_in_scope(edge.target, scope)
                .map(|child| scope_owner.get(&child).copied().unwrap_or(child));
            if extracted_tree_edges.contains(&edge_id) {
                // ExtractTrees removes the sentinel from the temporary
                // graph's routable edge inventory, but the original child
                // pointers remain visible to Node.getSymmetry. Preserve an
                // internal sentinel that collapses onto one scope owner for
                // projected child-recursion scoring only.
                if let (Some(source), Some(target)) = (source, target)
                    && source == target
                {
                    collapsed_symmetry_edges.push((edge.source, edge.target, source));
                }
                continue;
            }
            if crate::internal::trace_env_enabled("WEFTAN_TRACE_SCOPE_EDGE_PROJECTION")
                && self.graph.nodes[edge.source.0 as usize].tala_id == 4_219_055_323
                && self.graph.nodes[edge.target.0 as usize].tala_id == 3_468_733_332
            {
                eprintln!(
                    "SCOPE_EDGE_PROJECTION_RUST scope={:?} source={} container={:?} -> {:?} target={} container={:?} -> {:?}",
                    scope.map(|node| self.graph.nodes[node.0 as usize].tala_id),
                    self.graph.nodes[edge.source.0 as usize].tala_id,
                    self.graph.nodes[edge.source.0 as usize]
                        .container
                        .map(|node| self.graph.nodes[node.0 as usize].tala_id),
                    source.map(|node| self.graph.nodes[node.0 as usize].tala_id),
                    self.graph.nodes[edge.target.0 as usize].tala_id,
                    self.graph.nodes[edge.target.0 as usize]
                        .container
                        .map(|node| self.graph.nodes[node.0 as usize].tala_id),
                    target.map(|node| self.graph.nodes[node.0 as usize].tala_id),
                );
            }
            match (source, target) {
                (Some(child), None) => {
                    if edge.source == child
                        && let Some(edges) = projected_endpoint_edge_order.get_mut(&child)
                    {
                        edges.retain(|candidate| *candidate != edge_id);
                        edges.push(edge_id);
                    }
                    *projected_external_edge_counts.entry(child).or_default() += 1;
                    let exact_cousins = children_by_exact_uncle.entry(edge.target).or_default();
                    if !exact_cousins.contains(&child) {
                        exact_cousins.push(child);
                    }
                    // assignNears receives only the immediate parent's
                    // abductEdges result. If the opposite endpoint lies
                    // outside that parent, an ancestor has already collapsed
                    // this edge onto the parent carrier and the immediate
                    // call produces no abduction for this child scope.
                    if let Some(uncle) = self.projected_child_in_scope(edge.target, parent_scope) {
                        let cousins = children_by_uncle.entry(uncle).or_default();
                        if !cousins.contains(&child) {
                            cousins.push(child);
                        }
                    }
                    continue;
                }
                (None, Some(child)) => {
                    if edge.target == child
                        && let Some(edges) = projected_endpoint_edge_order.get_mut(&child)
                    {
                        edges.retain(|candidate| *candidate != edge_id);
                        edges.push(edge_id);
                    }
                    *projected_external_edge_counts.entry(child).or_default() += 1;
                    let exact_cousins = children_by_exact_uncle.entry(edge.source).or_default();
                    if !exact_cousins.contains(&child) {
                        exact_cousins.push(child);
                    }
                    if let Some(uncle) = self.projected_child_in_scope(edge.source, parent_scope) {
                        let cousins = children_by_uncle.entry(uncle).or_default();
                        if !cousins.contains(&child) {
                            cousins.push(child);
                        }
                    }
                    continue;
                }
                (None, None) => continue,
                (Some(source), Some(target)) => {
                    if source == target {
                        // A genuine self-loop remains an edge of TALA's
                        // temporary childrenGraph and contributes loop
                        // offsets, spacing, and optimizer cost. Only a
                        // non-loop whose distinct descendants project onto
                        // the same direct child disappears at this scope.
                        // A loop belongs to this placement graph only when
                        // its endpoint is the direct child represented by the
                        // projected owner.  A descendant loop (for example a
                        // loop on `x` while placing container `f`) stays on
                        // the descendant graph; projecting it onto `f` would
                        // manufacture extra self-neighbors and change
                        // median/annealing behavior.
                        if edge.source == source && edge.target == target {
                            projected_edges.push((edge_id, source, target));
                        } else if !aggregate_by_owner.contains_key(&source)
                            && ((edge.source == source
                                && self.graph.is_descendant_of(edge.target, source))
                                || (edge.target == source
                                    && self.graph.is_descendant_of(edge.source, source)))
                        {
                            // Graph.abductEdges retains an intra-container
                            // record whose original endpoints remain real,
                            // while both current endpoints are nil and the
                            // edge is absent from childrenGraph. This differs
                            // from two strict descendants of one child: that
                            // edge keeps its original endpoint incidence.
                            detached_container_edges.push((edge_id, source));
                            // This carrier is consumed only by the restored
                            // direct/mirror reachability boundary. It must not
                            // become a pre-score symmetry neighbor: the real
                            // source edge is detached until restore reconnects
                            // its original endpoints.
                            collapsed_restored_mirror_edges.push((
                                source,
                                self.graph.nodes[edge.source.0 as usize].tala_id,
                                self.graph.nodes[edge.source.0 as usize].container,
                                self.graph.nodes[edge.target.0 as usize].tala_id,
                                self.graph.nodes[edge.target.0 as usize].container,
                            ));
                        } else {
                            // The temporary graph drops this carrier self-edge,
                            // but both original endpoint Node pointers retain
                            // it. Graph.mirrorAxes' rdfsWalk can therefore
                            // leave the carrier through either endpoint even
                            // though the projected Edge is absent.
                            collapsed_symmetry_edges.push((edge.source, edge.target, source));
                            let arena_edge = &self.graph.edge_records[edge_id.0 as usize];
                            let source_is_strict_descendant =
                                aggregate_by_owner.contains_key(&source) || edge.source != source;
                            let target_is_strict_descendant =
                                aggregate_by_owner.contains_key(&source) || edge.target != source;
                            if arena_edge.is_between_table_columns()
                                && source_is_strict_descendant
                                && target_is_strict_descendant
                            {
                                // These are exactly the same-direct-child
                                // edges for which Graph.abductEdges continues
                                // without reconnecting either endpoint. They
                                // therefore remain in each restored original
                                // table's Node.Edges slice.
                                collapsed_table_column_edges.insert(edge_id, source);
                            }
                            collapsed_restored_mirror_edges.push((
                                source,
                                self.graph.nodes[edge.source.0 as usize].tala_id,
                                self.graph.nodes[edge.source.0 as usize].container,
                                self.graph.nodes[edge.target.0 as usize].tala_id,
                                self.graph.nodes[edge.target.0 as usize].container,
                            ));
                        }
                        continue;
                    }
                    // A projected internal edge is represented by its current
                    // sibling endpoints, but either projection means both
                    // endpoints participate in the recovered abduction
                    // boundary used by the sized optimizer.
                    // The first aggregate member is the stable Rust carrier
                    // for TALA's freshly allocated vessel. Equal NodeIds do
                    // not mean equal TALA nodes in that case: an edge on the
                    // carrier member is abducted onto the vessel just like an
                    // edge on every other member.
                    let source_is_abducted =
                        source != edge.source || aggregate_by_owner.contains_key(&source);
                    let target_is_abducted =
                        target != edge.target || aggregate_by_owner.contains_key(&target);
                    if source != edge.source || aggregate_by_owner.contains_key(&source) {
                        if let Some(edges) = projected_endpoint_edge_order.get_mut(&edge.source) {
                            edges.retain(|candidate| *candidate != edge_id);
                        }
                        projected_endpoint_edge_order
                            .entry(source)
                            .or_default()
                            .retain(|candidate| *candidate != edge_id);
                        projected_endpoint_edge_order
                            .entry(source)
                            .or_default()
                            .push(edge_id);
                    }
                    if target != edge.target || aggregate_by_owner.contains_key(&target) {
                        if let Some(edges) = projected_endpoint_edge_order.get_mut(&edge.target) {
                            edges.retain(|candidate| *candidate != edge_id);
                        }
                        projected_endpoint_edge_order
                            .entry(target)
                            .or_default()
                            .retain(|candidate| *candidate != edge_id);
                        projected_endpoint_edge_order
                            .entry(target)
                            .or_default()
                            .push(edge_id);
                    }
                    if source_is_abducted || target_is_abducted {
                        abducted_children.insert(source);
                        abducted_children.insert(target);
                        edge_abductions.push((source, target));
                    }
                    projected_edges.push((edge_id, source, target));
                }
            }
        }
        if crate::internal::trace_env_enabled("WEFTAN_TEST_INPUT_EDGE_ORDER") {
            projected_edges.sort_by_key(|(edge_id, _, _)| edge_id.0);
        }
        if crate::internal::trace_env_enabled("WEFTAN_TRACE_SCOPE_PROJECTED_EDGES") {
            let root_tala_id = scope
                .map(|node| self.graph.nodes[node.0 as usize].tala_id)
                .unwrap_or(0);
            eprint!("SCOPE_PROJECTED_EDGES_RUST root={root_tala_id}");
            for (edge_id, source, target) in &projected_edges {
                eprint!(
                    " [{}]{}>{}",
                    edge_id.0,
                    self.graph.nodes[source.0 as usize].tala_id,
                    self.graph.nodes[target.0 as usize].tala_id
                );
            }
            eprintln!();
        }
        // Tree preprocessing has already recorded the concrete Containers
        // slice produced by ExtractTrees and putBackNonBranchingTrees. Project
        // that authoritative order through temporary sequence/cluster
        // carriers rather than running a second, scope-local peeling guess.
        let mut preprocessed_children = Vec::with_capacity(scope_children.len());
        for child in self
            .graph
            .preprocessed_tree_children
            .get(&scope)
            .into_iter()
            .flatten()
            .copied()
            .chain(scope_children.iter().copied())
        {
            let owner = scope_owner.get(&child).copied().unwrap_or(child);
            if scope_children.contains(&owner) && !preprocessed_children.contains(&owner) {
                preprocessed_children.push(owner);
            }
        }
        // PreprocessTrees leaves the first member of each cluster in its
        // former position. AddClusters then removes every cluster member and
        // appends one freshly allocated vessel per cluster, while sequence
        // vessels remain at the position produced by AddSequences. Replay
        // that mutation on the reconstructed child slice instead of merely
        // mapping each old member to its owner in place.
        let cluster_members = self
            .graph
            .clusters
            .iter()
            .filter(|cluster| {
                cluster
                    .members
                    .iter()
                    .all(|member| children.contains(member))
            })
            .flat_map(|cluster| cluster.members.iter().copied())
            .collect::<BTreeSet<_>>();
        let mut active_children = Vec::with_capacity(preprocessed_children.len());
        for child in preprocessed_children.iter().copied() {
            if cluster_members.contains(&child) {
                continue;
            }
            let owner = scope_owner.get(&child).copied().unwrap_or(child);
            if scope_children.contains(&owner) && !active_children.contains(&owner) {
                active_children.push(owner);
            }
        }
        for cluster in self.graph.clusters.iter().filter(|cluster| {
            cluster
                .members
                .iter()
                .all(|member| children.contains(member))
        }) {
            if let Some(owner) = cluster.members.first().copied()
                && !active_children.contains(&owner)
            {
                active_children.push(owner);
            }
        }
        preprocessed_children = active_children;
        scope_children = preprocessed_children.clone();
        if crate::internal::trace_env_enabled("WEFTAN_TRACE_SCOPE_AGGREGATES") {
            eprintln!(
                "PREPROCESSED_CHILDREN_RUST scope={:?} children={:?}",
                scope.map(|node| self.graph.nodes[node.0 as usize].tala_id),
                preprocessed_children
                    .iter()
                    .map(|node| self.graph.nodes[node.0 as usize].tala_id)
                    .collect::<Vec<_>>()
            );
        }

        // SplitSubgraphs materializes each component in FIFO connectivity
        // order. This is the graph's actual Nodes slice order, not merely an
        // optimizer worklist: InitializeNodes and several global scans observe
        // it before the optimizer is constructed.
        // `abductEdges` reconnects only descendant endpoints. A direct child
        // keeps its pre-existing Node.Edges entries, while every edge moved
        // from a descendant is appended as reconnect visits Graph.Edges.
        // SplitSubgraphs then traverses each node's resulting edge slice, not
        // the owning Graph.Edges slice. Reconstruct that endpoint-local order
        // before deriving the FIFO component order.
        let projected_by_edge = projected_edges
            .iter()
            .map(|&(edge, source, target)| (edge, (source, target)))
            .collect::<BTreeMap<_, _>>();
        let mut incident_edge_order = BTreeMap::<NodeId, Vec<EdgeId>>::new();
        for child in preprocessed_children.iter().copied() {
            let incident = incident_edge_order.entry(child).or_default();
            if !aggregate_by_owner.contains_key(&child) {
                for edge in projected_endpoint_edge_order
                    .get(&child)
                    .into_iter()
                    .flatten()
                    .copied()
                {
                    let Some(&(source, target)) = projected_by_edge.get(&edge) else {
                        continue;
                    };
                    if self.graph.restored_tree_edges.contains(&edge) {
                        continue;
                    }
                    let original = &self.graph.edge_records[edge.0 as usize];
                    if (source == child && original.from == child)
                        || (target == child && original.to == child)
                    {
                        incident.push(edge);
                    }
                }
            }
            // Disconnect removes a tree edge from both endpoint slices;
            // reconnectTree appends it. Recovered Graph.Edges has the same
            // append order, so replay restored edges after retained entries.
            for edge in self.graph.edge_order.read().iter().copied() {
                if !self.graph.restored_tree_edges.contains(&edge) {
                    continue;
                }
                let Some(&(source, target)) = projected_by_edge.get(&edge) else {
                    continue;
                };
                if (source == child || target == child) && !incident.contains(&edge) {
                    incident.push(edge);
                }
            }
            for &(edge, source, target) in &projected_edges {
                if (source == child || target == child) && !incident.contains(&edge) {
                    incident.push(edge);
                }
            }
        }
        // Graphjson can carry more than one proximity relation for a node.
        // The public Graph keeps the historical scalar `near` field for
        // compatibility and stores additional targets separately. Project
        // both forms into every temporary childrenGraph.
        let input_near_targets = |node_id: NodeId| {
            // ArenaGraph is the authoritative post-hierarchy topology. The
            // serialized Graph retains its original `near_targets` metadata,
            // but Go's hierarchy cleanup has already removed links touching
            // hierarchy members by this point in placement.
            // node.go orderedNears sorts the map before reachability queues
            // its entries. Map insertion order is not this traversal order.
            // The dense aggregate carrier uses the first member's ID, but
            // Source walks the actual vessel's Nears before its members'
            // ordered maps (node.go reachableNodesGuarded). AssignNears can
            // put a relationship directly on two vessels; reading only the
            // first member loses that relationship before initialization.
            let (source_node, cluster_members) = match aggregate_by_owner.get(&node_id) {
                Some(PlacementAggregate::Cluster(index)) => (
                    self.graph
                        .topology_vessel_node(TopologyVessel::Cluster(*index))
                        .unwrap_or(node_id),
                    self.graph.clusters[*index].members.as_slice(),
                ),
                Some(PlacementAggregate::Sequence(index)) => (
                    self.graph
                        .topology_vessel_node(TopologyVessel::Sequence(*index))
                        .unwrap_or(node_id),
                    &[][..],
                ),
                None => (node_id, &[][..]),
            };
            let node = &self.graph.nodes[source_node.0 as usize];
            let mut nears = node.nears.keys();
            nears.sort_by_key(|near| self.graph.nodes[near.0 as usize].tala_id);
            for member in cluster_members {
                let mut member_nears = self.graph.nodes[member.0 as usize].nears.keys();
                member_nears.sort_by_key(|near| self.graph.nodes[near.0 as usize].tala_id);
                nears.extend(member_nears);
            }
            // node.go reachableNodesGuarded retains cross-container Nears
            // on the shared node, but only same-container targets connect
            // placement components. Projecting a descendant to its parent
            // here fabricates a sibling relationship and changes the split.
            nears.retain(|near| self.graph.nodes[near.0 as usize].container == node.container);
            nears
        };
        let mut adjacency = BTreeMap::<NodeId, Vec<NodeId>>::new();
        for child in preprocessed_children.iter().copied() {
            for edge in incident_edge_order.get(&child).into_iter().flatten() {
                let (source, target) = projected_by_edge[edge];
                let adjacent = if source == child { target } else { source };
                if !adjacency.entry(child).or_default().contains(&adjacent) {
                    adjacency.entry(child).or_default().push(adjacent);
                }
            }
        }
        for child in preprocessed_children.iter().copied() {
            for near in input_near_targets(child) {
                let Some(projected_near) = self
                    .projected_child_in_scope(near, scope)
                    .map(|near| scope_owner.get(&near).copied().unwrap_or(near))
                else {
                    continue;
                };
                if projected_near != child {
                    if !adjacency
                        .entry(child)
                        .or_default()
                        .contains(&projected_near)
                    {
                        adjacency.entry(child).or_default().push(projected_near);
                    }
                    if !adjacency
                        .entry(projected_near)
                        .or_default()
                        .contains(&child)
                    {
                        adjacency.entry(projected_near).or_default().push(child);
                    }
                }
            }
        }
        // assignNears runs before SplitSubgraphs. Children abducted to the
        // same outside uncle are therefore mutually reachable when the
        // subgraph's FIFO node slice is built. The recovered traversal visits
        // those near neighbors in numeric node-ID order.
        for cousins in children_by_uncle.values().filter(|nodes| nodes.len() > 1) {
            let mut cousins = cousins.clone();
            cousins.sort_by_key(|node| self.graph.nodes[node.0 as usize].tala_id);
            for (index, &first) in cousins.iter().enumerate() {
                for &second in &cousins[index + 1..] {
                    if !adjacency.entry(first).or_default().contains(&second) {
                        adjacency.entry(first).or_default().push(second);
                    }
                    if !adjacency.entry(second).or_default().contains(&first) {
                        adjacency.entry(second).or_default().push(first);
                    }
                }
            }
        }
        let mut old_to_new = BTreeMap::new();
        // Graph.placeNodes copies g.Containers[root] into childrenGraph in
        // its existing slice order. The reconstructed preprocessed slice is
        // that order; SplitSubgraphs later derives component traversal from
        // each node's edge slices without reordering Graph.Nodes itself.
        for child in preprocessed_children.iter().copied() {
            let mut node = self.input.node(child)?.clone();
            // Source translates NearKey once in d2talalayout/adapter.go,
            // before constructing temporary TALA children graphs. This
            // projection carries materialized Near pointers below; its flat
            // children slice is not a new D2 object tree.
            node.d2_near_key = None;
            node.parent = None;
            if let Some(&aggregate) = aggregate_by_owner.get(&child) {
                let (suffix, size) = match aggregate {
                    PlacementAggregate::Sequence(sequence_index) => {
                        ("sequence", self.graph.sequence_vessel_size(sequence_index))
                    }
                    PlacementAggregate::Cluster(cluster_index) => {
                        ("cluster", self.graph.cluster_vessel_size(cluster_index))
                    }
                };
                node.external_id = format!("{}#{suffix}-vessel", node.external_id);
                node.size = size;
                node.declared_size = None;
                node.label_size = None;
                node.label_position = LabelPosition::Unset;
                node.locked_position = None;
                node.constrained_x = None;
                node.constrained_y = None;
                node.near = None;
                node.fixed_width = false;
                node.fixed_height = false;
                node.force_hierarchy = false;
                node.grid_rows = None;
                node.grid_columns = None;
                node.canvas_position = None;
                node.external_label = None;
                node.icon_position = None;
                node.label_aware_grid = false;
                node.packed_grid = false;
                node.person = false;
                node.is_3d = false;
                node.is_multiple = false;
                node.shape = ShapeKind::Rectangle;
            } else {
                node.size = self.graph.nodes[child.0 as usize].rect.size;
                node.near = None;
            }
            let projected = graph.add_node(node);
            // Go's temporary childrenGraph aliases the original Node pointer,
            // including its prepared LoopOffsets. Carry the exact map and
            // prior position independently; the temporary scope still starts
            // with a nil TopLeft rather than recomputing its loop envelope.
            graph.set_initial_position(projected, self.graph.nodes[child.0 as usize].position);
            if !aggregate_by_owner.contains_key(&child) {
                graph.tala_node_loop_offsets[projected.0 as usize] =
                    self.graph.nodes[child.0 as usize].loop_offsets.clone();
            }
            // CopyEntitiesFrom aliases the original Go node, including its
            // runtime TALA ID. Temporary Rust graphs allocate dense nodes,
            // so preserve that identity explicitly; optimizer ordering and
            // every ID-stable sort otherwise fall back to hashing the public
            // external name and consume a different RNG/order stream.
            graph.tala_id_overrides[projected.0 as usize] =
                Some(self.graph.nodes[child.0 as usize].tala_id);
            if !aggregate_by_owner.contains_key(&child) {
                graph.set_table_column_count(
                    projected,
                    self.graph.nodes[child.0 as usize].table_column_count,
                );
            }
            if let Some(aggregate) = aggregate_by_owner.get(&child) {
                let vessel_tala_id = match aggregate {
                    PlacementAggregate::Sequence(sequence) => {
                        self.graph.sequences[*sequence].vessel_tala_id
                    }
                    PlacementAggregate::Cluster(cluster) => {
                        self.graph.clusters[*cluster].vessel_tala_id
                    }
                };
                // The recovered temporary Go node carries the runtime vessel
                // ID allocated by AddSequence/AddCluster. Preserve that ID
                // through ArenaGraph::fromInput; hashing the synthetic
                // `#sequence-vessel`/`#cluster-vessel` name changes the
                // optimizer's deterministic ordering and RNG-visible ties.
                graph.tala_id_overrides[projected.0 as usize] = Some(vessel_tala_id);
            }
            // Graph.placeNodes copies the original Go Node pointers into its
            // childrenGraph. Detaching the parent pointer does not clear
            // Node.isContainer, so preserve that identity before Pipeline is
            // constructed; cell size and label placement observe it during
            // ArenaGraph::fromInput, before scoring metadata is applied.
            graph.set_container_flag(
                projected,
                !aggregate_by_owner.contains_key(&child)
                    && self.graph.nodes[child.0 as usize].is_container,
            );
            old_to_new.insert(child, projected);
        }
        for (&member, &owner) in &scope_owner {
            if member != owner {
                old_to_new.insert(member, old_to_new[&owner]);
            }
        }
        // Retained branching trees are absent from childrenGraph.Nodes, but
        // their Node and Edge objects remain reachable through Graph.Trees.
        // Allocate those objects in the stable arena now and publish their
        // carrier only after Pipeline construction below.
        let mut tree_auxiliary_nodes = BTreeSet::new();
        let mut tree_old_to_new = BTreeMap::new();
        for &old_tree_node in self.graph.tree_routing_nodes.keys() {
            let mut sentinel = old_tree_node;
            while let Some(state) = self.graph.tree_routing_nodes.get(&sentinel) {
                sentinel = state.parent;
            }
            let Some(&projected_sentinel) = old_to_new.get(&sentinel) else {
                continue;
            };
            if !scope_children.contains(&sentinel) {
                continue;
            }
            let mut node = self.input.node(old_tree_node)?.clone();
            // Source translates NearKey once in d2talalayout/adapter.go,
            // before constructing temporary TALA children graphs. This
            // projection carries materialized Near pointers below; its flat
            // children slice is not a new D2 object tree.
            node.d2_near_key = None;
            node.parent = None;
            node.size = self.graph.nodes[old_tree_node.0 as usize].rect.size;
            node.near = None;
            // ExtractTrees keeps the peeled node in the shared owning graph
            // while omitting it from the temporary children slice.  The
            // auxiliary projection therefore has no Rust parent pointer, but
            // fitNodeToGraph still sees the original Go pointer and its
            // container ancestry. Preserve that scoring ancestry explicitly;
            // otherwise the fitted owner only considers the first surviving
            // (non-tree) child and drops the peeled subtree from its bounds.
            let projected = graph.add_node(node);
            graph.set_initial_position(
                projected,
                self.graph.nodes[old_tree_node.0 as usize].position,
            );
            graph.tala_node_loop_offsets[projected.0 as usize] = self.graph.nodes
                [old_tree_node.0 as usize]
                .loop_offsets
                .clone();
            graph.tala_id_overrides[projected.0 as usize] =
                Some(self.graph.nodes[old_tree_node.0 as usize].tala_id);
            graph.set_table_column_count(
                projected,
                self.graph.nodes[old_tree_node.0 as usize].table_column_count,
            );
            graph.set_container_flag(
                projected,
                self.graph.nodes[old_tree_node.0 as usize].is_container,
            );
            tree_auxiliary_nodes.insert(projected);
            tree_old_to_new.insert(old_tree_node, projected);
            // A retained tree node can also be the stable carrier of a
            // temporary sequence/cluster vessel. In the owner graph that
            // node must continue to map to the vessel; the separately
            // allocated projection above is only the Tree object used by
            // PlaceTrees. Overwriting `old_to_new` here reintroduced the
            // aggregate member as an independent box during fitNodeToGraph.
            old_to_new.entry(old_tree_node).or_insert(projected);
            // Keep the sentinel lookup alive even when an aggregate carrier
            // projects several original nodes onto one temporary node.
            let _ = projected_sentinel;
        }
        let mut scoring_node_metadata = scope_children
            .iter()
            .copied()
            .map(|child| {
                let original = &self.graph.nodes[child.0 as usize];
                let aggregate = aggregate_by_owner.get(&child).copied();
                // CopyEntitiesFrom/AddNode retain the vessel Node's fields.
                // Its herd is independent of every hidden original member.
                let herd_node = aggregate
                    .and_then(|aggregate| {
                        let kind = match aggregate {
                            PlacementAggregate::Sequence(index) => TopologyVessel::Sequence(index),
                            PlacementAggregate::Cluster(index) => TopologyVessel::Cluster(index),
                        };
                        self.graph.topology_vessel_node(kind)
                    })
                    .unwrap_or(child);
                ScopeNodeMetadata {
                    node: old_to_new[&child],
                    is_container: aggregate.is_none() && original.is_container,
                    parent_container: original.container,
                    parent_tala_id: original
                        .container
                        .map(|parent| self.graph.nodes[parent.0 as usize].tala_id),
                    container_ancestors: original.scoring_container_ancestors.clone(),
                    herd_assignment: self.graph.herd_assignment_snapshot(herd_node),
                    tala_id: aggregate.map(|aggregate| match aggregate {
                        PlacementAggregate::Sequence(sequence_index) => {
                            self.graph.sequences[sequence_index].vessel_tala_id
                        }
                        PlacementAggregate::Cluster(cluster_index) => {
                            self.graph.clusters[cluster_index].vessel_tala_id
                        }
                    }),
                    hierarchy: original.hierarchy.clone(),
                    position: (aggregate.is_none()
                        && (original.fixed_top_left.is_some() || original.hierarchy.is_some()))
                    .then_some(original.position)
                    .flatten(),
                    cluster_arrangement: aggregate.and_then(|aggregate| match aggregate {
                        PlacementAggregate::Cluster(cluster) => {
                            Some(self.graph.clusters[cluster].arrangement)
                        }
                        PlacementAggregate::Sequence(_) => None,
                    }),
                    is_aggregate_vessel: aggregate.is_some(),
                    node_padding: original.node_padding,
                    content_insets: original.content_insets,
                    label_size: original.label_size,
                    label_position: original.label_position,
                    external_label: original.external_label,
                    icon_position: original.icon_position,
                }
            })
            .collect::<Vec<_>>();
        // ExtractTrees keeps peeled nodes out of `scope_children`, but their
        // auxiliary projections still participate in the temporary graph's
        // fixed-bounds calculation after PlaceTrees restores them. Carry the
        // same ancestry metadata as the owning arena for those projections;
        // otherwise the Rust copy has a detached parent and is filtered out
        // before the enclosing container is fitted.
        for (&old_tree_node, &projected) in &tree_old_to_new {
            let original = &self.graph.nodes[old_tree_node.0 as usize];
            scoring_node_metadata.push(ScopeNodeMetadata {
                node: projected,
                is_container: original.is_container,
                parent_container: original.container,
                parent_tala_id: original
                    .container
                    .map(|parent| self.graph.nodes[parent.0 as usize].tala_id),
                container_ancestors: original.scoring_container_ancestors.clone(),
                herd_assignment: self.graph.herd_assignment_snapshot(old_tree_node),
                tala_id: None,
                hierarchy: original.hierarchy.clone(),
                position: original.position,
                cluster_arrangement: original.scoring_cluster_arrangement,
                is_aggregate_vessel: false,
                node_padding: original.node_padding,
                content_insets: original.content_insets,
                label_size: original.label_size,
                label_position: original.label_position,
                external_label: original.external_label,
                icon_position: original.icon_position,
            });
        }
        if crate::internal::trace_env_enabled("WEFTAN_TRACE_SCOPE_METADATA") {
            eprintln!(
                "SCOPE_METADATA_RUST scope={:?} {:?}",
                scope.map(|node| self.graph.nodes[node.0 as usize].tala_id),
                scoring_node_metadata
                    .iter()
                    .map(|metadata| {
                        (
                            metadata.node.0,
                            metadata.tala_id,
                            metadata.is_container,
                            metadata.is_aggregate_vessel,
                            metadata.parent_tala_id,
                        )
                    })
                    .collect::<Vec<_>>()
            );
        }
        for child in scope_children.iter().copied() {
            let projected_child = old_to_new[&child];
            for near in input_near_targets(child) {
                let Some(projected_near) = self
                    .projected_child_in_scope(near, scope)
                    .map(|near| scope_owner.get(&near).copied().unwrap_or(near))
                else {
                    continue;
                };
                if projected_near == child {
                    continue;
                }
                let Some(new_near) = old_to_new.get(&projected_near).copied() else {
                    continue;
                };
                if graph
                    .node(projected_child)
                    .and_then(|node| node.near)
                    .is_none()
                {
                    graph.node_mut(projected_child).expect("scope child").near = Some(new_near);
                } else if !graph.near_targets[projected_child.0 as usize].contains(&new_near) {
                    graph.near_targets[projected_child.0 as usize].push(new_near);
                }
            }
        }

        graph.external_edge_counts.resize(graph.nodes.len(), 0);
        for (child, count) in projected_external_edge_counts {
            if let Some(&projected) = old_to_new.get(&child) {
                graph.external_edge_counts[projected.0 as usize] += count;
            }
        }
        let mut sized_adjacent_overrides = Vec::new();
        let mut sized_cluster_distance_boxes = BTreeMap::new();
        let mut sized_edge_abductions = Vec::new();
        let mut projected_edge_minimums = BTreeMap::<EdgeId, (f64, f64)>::new();
        let mut sized_projected_obstructions = BTreeMap::new();
        let mut sized_projected_obstruction_sets = BTreeMap::new();
        let mut tree_edge_old_to_new = BTreeMap::new();
        let mut projected_edge_old_to_new = BTreeMap::new();
        let mut edge_table_column_counts = BTreeMap::new();
        // layout.go abductEdges consumes retained group records by endpoint
        // pair, once each. Direct-child matches happen for every edge before
        // the second pass expands vessel pointers on ancestor abductions.
        let mut group_records = Vec::new();
        for kind in [
            super::owner_groups::OwnerGroupKind::Cluster,
            super::owner_groups::OwnerGroupKind::Sequence,
        ] {
            if let Ok(map) = self.graph.owner_graph_group_map(OwnerGraphId(0), kind) {
                let mut cursor = self
                    .graph
                    .owner_group_map_values(&map)
                    .expect("group inventory");
                while let Some(group) = cursor.next() {
                    let Some(group) = group else { continue };
                    let header = match kind {
                        super::owner_groups::OwnerGroupKind::Cluster => {
                            self.graph
                                .owner_cluster_policy(group)
                                .expect("cluster records")
                                .edge_abductions
                        }
                        super::owner_groups::OwnerGroupKind::Sequence => self
                            .graph
                            .owner_sequence_abductions(group)
                            .expect("sequence records"),
                    };
                    if let Some(header) = header {
                        for id in header.iter_slots() {
                            group_records.push(
                                *self
                                    .graph
                                    .owner_edge_abduction(id.expect("group abduction pointer"))
                                    .expect("group abduction record"),
                            );
                        }
                    }
                }
            }
        }
        let canonical_child = |child: NodeId| {
            aggregate_by_owner
                .get(&child)
                .and_then(|aggregate| {
                    let kind = match *aggregate {
                        PlacementAggregate::Cluster(index) => TopologyVessel::Cluster(index),
                        PlacementAggregate::Sequence(index) => TopologyVessel::Sequence(index),
                    };
                    self.graph.topology_vessel_node(kind)
                })
                .unwrap_or(child)
        };
        let mut inherited_origins = BTreeMap::new();
        if !group_records.is_empty() {
            let mut used = vec![false; group_records.len()];
            for &(edge, source, target) in &projected_edges {
                let live = &self.graph.edge_records[edge.0 as usize];
                let current_from = canonical_child(source);
                let current_to = canonical_child(target);
                let mut from = (live.from != current_from).then_some(live.from);
                let mut to = (live.to != current_to).then_some(live.to);
                if from.is_none() && to.is_none() {
                    for (index, record) in group_records.iter().enumerate() {
                        if !used[index]
                            && record.current_from == Some(live.from)
                            && record.current_to == Some(live.to)
                        {
                            used[index] = true;
                            if record.originally_from.is_some() {
                                from = record.originally_from;
                            } else {
                                to = record.originally_to;
                            }
                            break;
                        }
                    }
                }
                inherited_origins.insert(edge, (from, to));
            }
            for &(edge, source, target) in &projected_edges {
                let (from, to) = inherited_origins
                    .get_mut(&edge)
                    .expect("projected originals");
                if from.is_none() && to.is_none() {
                    continue;
                }
                for (index, record) in group_records.iter().enumerate() {
                    if used[index] {
                        continue;
                    }
                    let matched = if from.is_none() {
                        record.current_from == Some(canonical_child(source))
                            && record.current_to == *to
                    } else if to.is_none() {
                        record.current_to == Some(canonical_child(target))
                            && record.current_from == *from
                    } else {
                        record.current_from == *from && record.current_to == *to
                    };
                    if matched {
                        used[index] = true;
                        if record.originally_from.is_some() {
                            *from = record.originally_from;
                        } else {
                            *to = record.originally_to;
                        }
                        break;
                    }
                }
            }
        }
        // Endpoint projections retain the original node's box while the
        // temporary graph moves an enclosing carrier. Node.Box.TopLeft is
        // local to its immediate container, so recover the same absolute
        // offset that Go's shared Graph.Containers walk exposes by summing
        // the complete container chain through the projected owner.
        let project_endpoint_offset = |original: NodeId, owner: NodeId| {
            let original_position = self.graph.position(original)?;
            // A not-yet-positioned container has no translation of its local
            // child frame. This transform does not assign the owner's TopLeft.
            let owner_position = self.graph.position(owner).unwrap_or_default();
            let mut accumulated = original_position;
            let mut current = self.graph.nodes[original.0 as usize].container;
            let mut reached_owner = original == owner;
            while let Some(parent) = current {
                if let Some(parent_position) = self.graph.position(parent) {
                    accumulated.x += parent_position.x;
                    accumulated.y += parent_position.y;
                }
                if parent == owner {
                    reached_owner = true;
                    break;
                }
                current = self.graph.nodes[parent.0 as usize].container;
            }
            if reached_owner {
                Some(Point {
                    x: accumulated.x - owner_position.x,
                    y: accumulated.y - owner_position.y,
                })
            } else {
                // External carriers can be projected without an ancestor
                // chain in the stable graph. Preserve the prior direct
                // displacement for those cases.
                Some(Point {
                    x: original_position.x - owner_position.x,
                    y: original_position.y - owner_position.y,
                })
            }
        };
        for (edge_id, source, target) in projected_edges.iter().copied() {
            let input_edge_id = self.graph.edge_records[edge_id.0 as usize].input_id;
            let source_edge = &self.graph.edge_records[edge_id.0 as usize];
            let original = crate::Edge {
                source: inherited_origins
                    .get(&edge_id)
                    .and_then(|origins| origins.0)
                    .unwrap_or(source_edge.from),
                target: inherited_origins
                    .get(&edge_id)
                    .and_then(|origins| origins.1)
                    .unwrap_or(source_edge.to),
            };
            let projected_edge = graph.add_edge(crate::Edge {
                source: old_to_new[&source],
                target: old_to_new[&target],
            });
            // Graph.abductEdges reconnects the original edge onto a scope
            // vessel, but Node.deltaTo still reads the edge's minimum label
            // dimensions from that carrier edge. Preserve those dimensions
            // when materializing the temporary projected graph.
            projected_edge_minimums.insert(
                projected_edge,
                (source_edge.min_width, source_edge.min_height),
            );
            graph.set_edge_min_dimensions(
                projected_edge,
                Some(source_edge.min_width),
                Some(source_edge.min_height),
            );
            graph.tala_edge_ids[projected_edge.0 as usize] =
                Some(self.graph.edge_records[edge_id.0 as usize].tala_id);
            graph.tala_edge_invisible[projected_edge.0 as usize] =
                Some(self.graph.edge_records[edge_id.0 as usize].invisible);
            graph.tala_edge_label_fixed[projected_edge.0 as usize] =
                self.graph.edge_records[edge_id.0 as usize].label_position_fixed;
            projected_edge_old_to_new.insert(edge_id, projected_edge);
            graph.set_edge_arrows(projected_edge, self.input.edge_arrows(input_edge_id));
            graph.set_edge_arrowheads(projected_edge, self.input.edge_arrowheads(input_edge_id));
            graph.set_edge_label(
                projected_edge,
                self.input.edge_label(input_edge_id).cloned(),
            );
            graph.set_edge_arrowhead_labels(
                projected_edge,
                self.input.edge_arrowhead_labels(input_edge_id),
            );
            graph.set_edge_table_columns(
                projected_edge,
                self.input.edge_table_columns(input_edge_id),
            );
            edge_table_column_counts.insert(
                projected_edge,
                (
                    self.input.table_column_count(original.source),
                    self.input.table_column_count(original.target),
                ),
            );
            // Retain the two-pass original member pointers independently of
            // current projected vessel endpoints. Unbound numerical adapters
            // retain their existing member-carrier construction until their
            // canonical group records are allocated.
            let source_has_reconnection = inherited_origins
                .get(&edge_id)
                .map(|origins| origins.0.is_some())
                .unwrap_or(source != original.source || aggregate_by_owner.contains_key(&source));
            let target_has_reconnection = inherited_origins
                .get(&edge_id)
                .map(|origins| origins.1.is_some())
                .unwrap_or(target != original.target || aggregate_by_owner.contains_key(&target));
            // The recovered Go abduction retains the original member pointer
            // even when the current endpoint is a sequence/cluster vessel.
            // SizedOptimizer.getAdjacents then observes that shared pointer's
            // live box while the vessel moves.  Do not erase that endpoint
            // merely because the current carrier is an aggregate owner.
            let source_has_original = source_has_reconnection;
            let target_has_original = target_has_reconnection;
            let mut originally_from = None;
            let mut originally_to = None;
            if source_has_reconnection {
                let retained_cluster = self.retained_cluster_vessel_geometry(
                    original.source,
                    source,
                    scope,
                    &scope_owner,
                    &old_to_new,
                );
                // Go retains the original node pointer before looking at
                // its box. A nil TopLeft is missing geometry, not a nil node.
                let endpoint = &self.graph.nodes[original.source.0 as usize];
                let mut retained = AbductedNodeRef {
                    source_node: Some(original.source),
                    source_owner: Some(source),
                    owner_tala_id: None,
                    owner: Some(old_to_new[&source]),
                    tala_id: endpoint.tala_id,
                    container_tala_id: endpoint
                        .container
                        .map(|container| self.graph.nodes[container.0 as usize].tala_id),
                    offset: None,
                    size: endpoint.rect.size,
                    cluster_member: matches!(
                        aggregate_by_owner.get(&source),
                        Some(PlacementAggregate::Cluster(_))
                    ) || retained_cluster.is_some(),
                };
                if source_has_original {
                    originally_from = Some(retained);
                }
                let geometry = aggregate_by_owner
                    .get(&source)
                    .and_then(|aggregate| {
                        self.aggregate_member_geometry(*aggregate, original.source)
                    })
                    .or_else(|| {
                        let offset = project_endpoint_offset(original.source, source)?;
                        Some((
                            offset,
                            self.graph.nodes[original.source.0 as usize].rect.size,
                            false,
                        ))
                    });
                if let Some((offset, size, cluster_member)) = geometry {
                    if crate::internal::trace_env_enabled("WEFTAN_TRACE_PROJECT_BUILD")
                        && crate::internal::trace_env_value("WEFTAN_TRACE_PROJECT_BUILD")
                            .is_some_and(|target| {
                                target == "all"
                                    || target.parse::<i64>().ok()
                                        == Some(
                                            self.graph.nodes[original.source.0 as usize].tala_id,
                                        )
                            })
                    {
                        let mut source_chain = Vec::new();
                        let mut source_cursor = Some(original.source);
                        while let Some(cursor) = source_cursor {
                            source_chain.push((
                                self.graph.nodes[cursor.0 as usize].tala_id,
                                self.graph.position(cursor),
                            ));
                            source_cursor = self.graph.nodes[cursor.0 as usize].container;
                        }
                        eprintln!(
                            "PROJECT_BUILD_RUST scope={:?} source child={} owner={} aggregate={:?} source_container={:?} chain={:?} offset={},{} source_pos={:?} owner_pos={:?}",
                            scope.map(|node| self.graph.nodes[node.0 as usize].tala_id),
                            self.graph.nodes[original.source.0 as usize].tala_id,
                            self.graph.nodes[source.0 as usize].tala_id,
                            aggregate_by_owner.get(&source),
                            self.graph.nodes[original.source.0 as usize]
                                .container
                                .map(|node| self.graph.nodes[node.0 as usize].tala_id),
                            source_chain,
                            offset.x,
                            offset.y,
                            self.graph.position(original.source),
                            self.graph.position(source)
                        );
                    }
                    let projected = ProjectedAdjacent {
                        owner: old_to_new[&source],
                        tala_id: self.graph.nodes[original.source.0 as usize].tala_id,
                        container_tala_id: self.graph.nodes[original.source.0 as usize]
                            .container
                            .map(|container| self.graph.nodes[container.0 as usize].tala_id),
                        offset,
                        size,
                        cluster_member: cluster_member || retained_cluster.is_some(),
                    };
                    retained.offset = Some(offset);
                    retained.size = size;
                    retained.cluster_member = projected.cluster_member;
                    if source_has_original {
                        originally_from = Some(retained);
                    }
                    sized_adjacent_overrides
                        .push(((old_to_new[&target], projected_edge), projected));
                    if let Some(vessel) = retained_cluster {
                        sized_cluster_distance_boxes
                            .insert((old_to_new[&source], projected.tala_id), vessel);
                    }
                }
            }
            if target_has_reconnection {
                let retained_cluster = self.retained_cluster_vessel_geometry(
                    original.target,
                    target,
                    scope,
                    &scope_owner,
                    &old_to_new,
                );
                // Go retains the original node pointer before looking at
                // its box. A nil TopLeft is missing geometry, not a nil node.
                let endpoint = &self.graph.nodes[original.target.0 as usize];
                let mut retained = AbductedNodeRef {
                    source_node: Some(original.target),
                    source_owner: Some(target),
                    owner_tala_id: None,
                    owner: Some(old_to_new[&target]),
                    tala_id: endpoint.tala_id,
                    container_tala_id: endpoint
                        .container
                        .map(|container| self.graph.nodes[container.0 as usize].tala_id),
                    offset: None,
                    size: endpoint.rect.size,
                    cluster_member: matches!(
                        aggregate_by_owner.get(&target),
                        Some(PlacementAggregate::Cluster(_))
                    ) || retained_cluster.is_some(),
                };
                if target_has_original {
                    originally_to = Some(retained);
                }
                let geometry = aggregate_by_owner
                    .get(&target)
                    .and_then(|aggregate| {
                        self.aggregate_member_geometry(*aggregate, original.target)
                    })
                    .or_else(|| {
                        let offset = project_endpoint_offset(original.target, target)?;
                        Some((
                            offset,
                            self.graph.nodes[original.target.0 as usize].rect.size,
                            false,
                        ))
                    });
                if let Some((offset, size, cluster_member)) = geometry {
                    if crate::internal::trace_env_enabled("WEFTAN_TRACE_PROJECT_BUILD")
                        && crate::internal::trace_env_value("WEFTAN_TRACE_PROJECT_BUILD")
                            .is_some_and(|target| {
                                target == "all"
                                    || target.parse::<i64>().ok()
                                        == Some(
                                            self.graph.nodes[original.target.0 as usize].tala_id,
                                        )
                            })
                    {
                        let mut target_chain = Vec::new();
                        let mut target_cursor = Some(original.target);
                        while let Some(cursor) = target_cursor {
                            target_chain.push((
                                self.graph.nodes[cursor.0 as usize].tala_id,
                                self.graph.position(cursor),
                            ));
                            target_cursor = self.graph.nodes[cursor.0 as usize].container;
                        }
                        eprintln!(
                            "PROJECT_BUILD_RUST scope={:?} target child={} owner={} aggregate={:?} target_container={:?} chain={:?} offset={},{} target_pos={:?} owner_pos={:?}",
                            scope.map(|node| self.graph.nodes[node.0 as usize].tala_id),
                            self.graph.nodes[original.target.0 as usize].tala_id,
                            self.graph.nodes[target.0 as usize].tala_id,
                            aggregate_by_owner.get(&target),
                            self.graph.nodes[original.target.0 as usize]
                                .container
                                .map(|node| self.graph.nodes[node.0 as usize].tala_id),
                            target_chain,
                            offset.x,
                            offset.y,
                            self.graph.position(original.target),
                            self.graph.position(target)
                        );
                    }
                    let projected = ProjectedAdjacent {
                        owner: old_to_new[&target],
                        tala_id: self.graph.nodes[original.target.0 as usize].tala_id,
                        container_tala_id: self.graph.nodes[original.target.0 as usize]
                            .container
                            .map(|container| self.graph.nodes[container.0 as usize].tala_id),
                        offset,
                        size,
                        cluster_member: cluster_member || retained_cluster.is_some(),
                    };
                    retained.offset = Some(offset);
                    retained.size = size;
                    retained.cluster_member = projected.cluster_member;
                    if target_has_original {
                        originally_to = Some(retained);
                    }
                    sized_adjacent_overrides
                        .push(((old_to_new[&source], projected_edge), projected));
                    if let Some(vessel) = retained_cluster {
                        sized_cluster_distance_boxes
                            .insert((old_to_new[&target], projected.tala_id), vessel);
                    }
                }
            }
            if source_has_reconnection || target_has_reconnection {
                sized_edge_abductions.push(SizedEdgeAbduction {
                    edge: Some(projected_edge),
                    source_edge: edge_id,
                    edge_tala_id: source_edge.tala_id,
                    current_from: Some(AbductionCurrentNodeRef::local_with_source(
                        old_to_new[&source],
                        source,
                    )),
                    current_to: Some(AbductionCurrentNodeRef::local_with_source(
                        old_to_new[&target],
                        target,
                    )),
                    originally_from,
                    originally_to,
                    originally_from_table_neighbors: Vec::new(),
                    originally_to_table_neighbors: Vec::new(),
                    originally_from_container: source_has_original
                        .then(|| self.graph.nodes[original.source.0 as usize].container)
                        .flatten(),
                    originally_to_container: target_has_original
                        .then(|| self.graph.nodes[original.target.0 as usize].container)
                        .flatten(),
                    sequence_abduction: matches!(
                        aggregate_by_owner.get(&source),
                        Some(PlacementAggregate::Sequence(_))
                    ) || matches!(
                        aggregate_by_owner.get(&target),
                        Some(PlacementAggregate::Sequence(_))
                    ),
                    obstructions_from_to: Vec::new(),
                    obstructions_to_from: Vec::new(),
                    obstruction_sets_from_to: OrderedObstructionInventory::default(),
                    obstruction_sets_to_from: OrderedObstructionInventory::default(),
                });
            }
            // The temporary SplitSubgraphs graph has its own Graph.Nodes
            // slice, but Node.edgeLength still observes the parent graph's
            // shared Graph.Containers map. Preserve that obstruction carrier
            // for every projected edge, not only edges whose endpoint was
            // abducted. The receiver endpoint is walked first, so the
            // observable inventory is directional.
            let project_obstructions = |first_endpoint: NodeId, second_endpoint: NodeId| {
                let mut obstructions = Vec::new();
                let candidate_sets = self.graph.edge_obstruction_sets_for_materialized(
                    first_endpoint,
                    second_endpoint,
                    materialized_parent_scopes,
                );
                let mut projected_retained_clusters = BTreeSet::new();
                let endpoint_cluster_vessels = [first_endpoint, second_endpoint]
                    .into_iter()
                    .filter_map(|endpoint| {
                        let cluster = self.graph.nodes[endpoint.0 as usize].cluster?;
                        Some(self.graph.clusters[cluster].vessel_tala_id)
                    })
                    .collect::<BTreeSet<_>>();
                let candidates = if scope.is_none() {
                    self.graph.edge_obstruction_nodes_for_materialized(
                        first_endpoint,
                        second_endpoint,
                        materialized_parent_scopes,
                    )
                } else {
                    self.graph
                        .edge_obstruction_nodes(first_endpoint, second_endpoint)
                };
                for candidate_id in candidates {
                    let candidate = &self.graph.nodes[candidate_id.0 as usize];
                    if candidate_id == first_endpoint
                        || candidate_id == second_endpoint
                        || self.graph.is_descendant_of(first_endpoint, candidate_id)
                        || self.graph.is_descendant_of(second_endpoint, candidate_id)
                    {
                        continue;
                    }
                    let Some(projected_child) = self.projected_child_in_scope(candidate_id, scope)
                    else {
                        continue;
                    };
                    let owner = scope_owner
                        .get(&projected_child)
                        .copied()
                        .unwrap_or(projected_child);
                    let Some(&projected_owner) = old_to_new.get(&owner) else {
                        continue;
                    };
                    // A recursively placed descendant cluster remains one
                    // vessel in the shared Containers map. Its restored
                    // members keep their Cluster pointers for endpoint
                    // scoring, but they do not become separate route
                    // obstructions in the parent placement graph.
                    if !aggregate_by_owner.contains_key(&owner)
                        && let Some(cluster) = self.retained_cluster_vessel_geometry(
                            candidate_id,
                            owner,
                            scope,
                            &scope_owner,
                            &old_to_new,
                        )
                    {
                        if endpoint_cluster_vessels.contains(&cluster.vessel_tala_id)
                            || !projected_retained_clusters.insert(cluster.vessel_tala_id)
                        {
                            continue;
                        }
                        obstructions.push(ProjectedAdjacent {
                            owner: projected_owner,
                            tala_id: cluster.vessel_tala_id,
                            container_tala_id: candidate
                                .container
                                .map(|container| self.graph.nodes[container.0 as usize].tala_id),
                            offset: cluster.offset,
                            size: cluster.size,
                            cluster_member: false,
                        });
                        continue;
                    }
                    // AddSequence/AddCluster remove their members from the
                    // temporary graph's Containers slices and append one
                    // vessel. Preserve that vessel once, through its owner
                    // member; recovered Node.edgeLength skips it later only
                    // when it owns one of this edge's replacement endpoints.
                    if let Some(aggregate) = aggregate_by_owner.get(&owner) {
                        if candidate_id != owner {
                            continue;
                        }
                        if [first_endpoint, second_endpoint]
                            .iter()
                            .any(|endpoint| scope_owner.get(endpoint) == Some(&owner))
                        {
                            continue;
                        }
                        let vessel = graph.node(projected_owner).expect("aggregate vessel");
                        let tala_id = match aggregate {
                            PlacementAggregate::Sequence(index) => {
                                self.graph.sequences[*index].vessel_tala_id
                            }
                            PlacementAggregate::Cluster(index) => {
                                self.graph.clusters[*index].vessel_tala_id
                            }
                        };
                        obstructions.push(ProjectedAdjacent {
                            owner: projected_owner,
                            tala_id,
                            container_tala_id: candidate
                                .container
                                .map(|container| self.graph.nodes[container.0 as usize].tala_id),
                            offset: Point::default(),
                            size: vessel.size,
                            cluster_member: false,
                        });
                        continue;
                    }
                    // TALA leaves the carrier's TopLeft nil while its
                    // descendants retain coordinates local to the carrier.
                    // Project those descendants from the same logical origin
                    // used for abducted edge endpoints above.
                    let geometry = if candidate_id == owner {
                        (Point::default(), candidate.rect.size)
                    } else {
                        let Some(mut position) = candidate.position else {
                            continue;
                        };
                        // Node.Box.TopLeft is local to its immediate
                        // container.  The parent route sees the shared node
                        // after each enclosing container translation, so
                        // accumulate that chain before projecting it onto the
                        // temporary scope owner.
                        let mut current = candidate.container;
                        let mut reached_owner = candidate_id == owner;
                        while let Some(parent) = current {
                            if let Some(parent_position) = self.graph.position(parent) {
                                position.x += parent_position.x;
                                position.y += parent_position.y;
                            }
                            if parent == owner {
                                reached_owner = true;
                                break;
                            }
                            current = self.graph.nodes[parent.0 as usize].container;
                        }
                        if !reached_owner {
                            continue;
                        }
                        let owner_position = self.graph.position(owner).unwrap_or_default();
                        (
                            Point {
                                x: position.x - owner_position.x,
                                y: position.y - owner_position.y,
                            },
                            candidate.rect.size,
                        )
                    };
                    obstructions.push(ProjectedAdjacent {
                        owner: projected_owner,
                        tala_id: candidate.tala_id,
                        container_tala_id: candidate
                            .container
                            .map(|container| self.graph.nodes[container.0 as usize].tala_id),
                        offset: geometry.0,
                        size: geometry.1,
                        cluster_member: false,
                    });
                }
                let sets = candidate_sets
                    .into_iter()
                    .map(|(source_container, candidates)| {
                        let container_tala_id = source_container.and_then(|container| {
                            self.graph
                                .topology_node(container)
                                .or_else(|| self.graph.nodes.get(container.0 as usize))
                                .map(|container| container.tala_id)
                        });
                        let items = candidates.map(|candidates| {
                            candidates
                                .into_iter()
                                .map(|candidate_id| {
                                    let candidate = self.graph.topology_node(candidate_id)
                            .or_else(|| self.graph.nodes.get(candidate_id.0 as usize))
                            .expect("retained obstruction pointer has canonical node metadata");
                                    let input_candidate = self
                                        .graph
                                        .topology_source_input_node(candidate_id)
                                        .unwrap_or(candidate_id);
                                    let owner = self
                                        .graph
                                        .nodes
                                        .get(input_candidate.0 as usize)
                                        .and_then(|_| {
                                            self.projected_child_in_scope(input_candidate, scope)
                                        })
                                        .map(|child| {
                                            scope_owner.get(&child).copied().unwrap_or(child)
                                        });
                                    let local_owner =
                                        owner.and_then(|owner| old_to_new.get(&owner).copied());
                                    let projection = local_owner.and_then(|local_owner| {
                                        obstructions
                                            .iter()
                                            .copied()
                                            .find(|projection| {
                                                projection.owner == local_owner
                                                    && projection.tala_id == candidate.tala_id
                                            })
                                            .or_else(|| {
                                                // Endpoints and ancestors remain in the actual
                                                // scratch slice even when their predicates skip
                                                // scoring. Preserve their live carrier geometry.
                                                let owner = owner?;
                                                let (offset, size, cluster_member) =
                                                    aggregate_by_owner
                                                        .get(&owner)
                                                        .and_then(|aggregate| {
                                                            self.aggregate_member_geometry(
                                                                *aggregate,
                                                                candidate_id,
                                                            )
                                                        })
                                                        .or_else(|| {
                                                            Some((
                                                                project_endpoint_offset(
                                                                    candidate_id,
                                                                    owner,
                                                                )?,
                                                                candidate.rect.size,
                                                                false,
                                                            ))
                                                        })?;
                                                Some(ProjectedAdjacent {
                                                    owner: local_owner,
                                                    tala_id: candidate.tala_id,
                                                    container_tala_id: candidate
                                                        .container
                                                        .and_then(|container| {
                                                            self.graph
                                                                .topology_node(container)
                                                                .map(|container| container.tala_id)
                                                        }),
                                                    offset,
                                                    size,
                                                    cluster_member,
                                                })
                                            })
                                    });
                                    let is_replacement_ancestor =
                                        self.graph.nodes.get(candidate_id.0 as usize).is_some_and(
                                            |_| {
                                                self.graph
                                                    .nodes
                                                    .get(first_endpoint.0 as usize)
                                                    .is_some()
                                                    && self
                                                        .graph
                                                        .nodes
                                                        .get(second_endpoint.0 as usize)
                                                        .is_some()
                                                    && (self.graph.is_descendant_of(
                                                        first_endpoint,
                                                        candidate_id,
                                                    ) || self.graph.is_descendant_of(
                                                        second_endpoint,
                                                        candidate_id,
                                                    ))
                                            },
                                        );
                                    ProjectedObstructionRef {
                                        source_arena: self.graph.topology_arena_id(),
                                        source_node: Some(candidate_id),
                                        source_owner: owner,
                                        location: local_owner
                                            .map(AbductionNodeLocation::Local)
                                            .unwrap_or(AbductionNodeLocation::Foreign),
                                        tala_id: candidate.tala_id,
                                        source_container: candidate.container,
                                        container_tala_id: candidate.container.and_then(
                                            |container| {
                                                self.graph
                                                    .topology_node(container)
                                                    .or_else(|| {
                                                        self.graph.nodes.get(container.0 as usize)
                                                    })
                                                    .map(|container| container.tala_id)
                                            },
                                        ),
                                        position: candidate.position,
                                        size: candidate.rect.size,
                                        projection,
                                        graph_owner: self.graph.topology_node_owner(candidate_id),
                                        same_graph_as_replacement: owner.is_some()
                                            && local_owner.is_some(),
                                        is_endpoint: candidate_id == first_endpoint
                                            || candidate_id == second_endpoint,
                                        is_replacement_ancestor,
                                        is_cluster_vessel: self
                                            .graph
                                            .topology_node_vessel(candidate_id)
                                            .is_some_and(|vessel| {
                                                matches!(vessel, TopologyVessel::Cluster(_))
                                            })
                                            || matches!(
                                                owner.and_then(
                                                    |owner| aggregate_by_owner.get(&owner)
                                                ),
                                                Some(PlacementAggregate::Cluster(_))
                                            ),
                                        is_endpoint_cluster_vessel: endpoint_cluster_vessels
                                            .contains(&candidate.tala_id),
                                    }
                                })
                                .collect()
                        });
                        ProjectedObstructionSet {
                            source_container,
                            container_tala_id,
                            items,
                        }
                    })
                    .collect();
                (
                    obstructions,
                    OrderedObstructionInventory {
                        source_arena: self.graph.topology_arena_id(),
                        sets_present: true,
                        node_replacement: Some(first_endpoint),
                        adjacent_replacement: Some(second_endpoint),
                        sets,
                    },
                )
            };
            // `Node.edgeLength` first consumes abductions by current endpoint
            // pair, not by edge identity. A matched edge routes through the
            // restored endpoints recorded by that abduction. An unmatched
            // parallel edge routes through its current carrier endpoints,
            // even when the stable input edge behind that slot originally
            // joined descendants. Keep those two inventories distinct.
            let (current_obstructions_from_to, current_sets_from_to) =
                project_obstructions(source, target);
            let (current_obstructions_to_from, current_sets_to_from) =
                project_obstructions(target, source);
            sized_projected_obstructions.insert(
                (old_to_new[&source], projected_edge),
                current_obstructions_from_to,
            );
            sized_projected_obstructions.insert(
                (old_to_new[&target], projected_edge),
                current_obstructions_to_from,
            );
            sized_projected_obstruction_sets
                .insert((old_to_new[&source], projected_edge), current_sets_from_to);
            sized_projected_obstruction_sets
                .insert((old_to_new[&target], projected_edge), current_sets_to_from);
            if source_has_reconnection || target_has_reconnection {
                let abduction = sized_edge_abductions
                    .last_mut()
                    .expect("abduction inserted for projected endpoints");
                (
                    abduction.obstructions_from_to,
                    abduction.obstruction_sets_from_to,
                ) = project_obstructions(original.source, original.target);
                (
                    abduction.obstructions_to_from,
                    abduction.obstruction_sets_to_from,
                ) = project_obstructions(original.target, original.source);
            }
        }
        for (source_edge, owner) in detached_container_edges {
            let edge = &self.graph.edge_records[source_edge.0 as usize];
            let original_endpoint = |node: NodeId| {
                let endpoint = &self.graph.nodes[node.0 as usize];
                Some(AbductedNodeRef {
                    source_node: Some(node),
                    source_owner: Some(owner),
                    owner_tala_id: None,
                    owner: Some(*old_to_new.get(&owner)?),
                    tala_id: endpoint.tala_id,
                    container_tala_id: endpoint
                        .container
                        .map(|container| self.graph.nodes[container.0 as usize].tala_id),
                    offset: if node == owner {
                        self.graph.position(node).map(|_| Point::default())
                    } else {
                        project_endpoint_offset(node, owner)
                    },
                    size: endpoint.rect.size,
                    cluster_member: false,
                })
            };
            sized_edge_abductions.push(SizedEdgeAbduction {
                edge: None,
                source_edge,
                edge_tala_id: edge.tala_id,
                current_from: None,
                current_to: None,
                originally_from: original_endpoint(edge.from),
                originally_to: original_endpoint(edge.to),
                originally_from_table_neighbors: Vec::new(),
                originally_to_table_neighbors: Vec::new(),
                originally_from_container: self.graph.nodes[edge.from.0 as usize].container,
                originally_to_container: self.graph.nodes[edge.to.0 as usize].container,
                sequence_abduction: false,
                obstructions_from_to: Vec::new(),
                obstructions_to_from: Vec::new(),
                obstruction_sets_from_to: OrderedObstructionInventory::default(),
                obstruction_sets_to_from: OrderedObstructionInventory::default(),
            });
        }
        // Both detached and reconnected records are emitted during Go's one
        // Graph.Edges walk. Restore that real order after materializing the
        // two native edge inventories separately; stable IDs can collide,
        // so the originating owning-arena EdgeId is the ordering identity.
        let source_edge_order = self
            .graph
            .edge_order
            .read()
            .iter()
            .copied()
            .enumerate()
            .map(|(index, edge)| (edge, index))
            .collect::<BTreeMap<_, _>>();
        sized_edge_abductions.sort_by_key(|abduction| {
            source_edge_order
                .get(&abduction.source_edge)
                .copied()
                .unwrap_or(usize::MAX)
        });
        let project_collapsed_symmetry_node =
            |original: NodeId, owner: NodeId| -> Option<ProjectedAdjacent> {
                let projected_owner = old_to_new.get(&owner).copied()?;
                let (offset, size, cluster_member) =
                    if let Some(aggregate) = aggregate_by_owner.get(&owner) {
                        self.aggregate_member_geometry(*aggregate, original)?
                    } else if original == owner {
                        (
                            Point::default(),
                            self.graph.nodes[original.0 as usize].rect.size,
                            false,
                        )
                    } else {
                        let original_position = self.graph.position(original)?;
                        let owner_position = self.graph.position(owner).unwrap_or_default();
                        (
                            Point {
                                x: original_position.x - owner_position.x,
                                y: original_position.y - owner_position.y,
                            },
                            self.graph.nodes[original.0 as usize].rect.size,
                            false,
                        )
                    };
                Some(ProjectedAdjacent {
                    owner: projected_owner,
                    tala_id: self.graph.nodes[original.0 as usize].tala_id,
                    container_tala_id: self.graph.nodes[original.0 as usize]
                        .container
                        .map(|container| self.graph.nodes[container.0 as usize].tala_id),
                    offset,
                    size,
                    cluster_member,
                })
            };
        let mut sized_collapsed_symmetry_neighbors = BTreeMap::<i64, Vec<ProjectedAdjacent>>::new();
        for (source, target, owner) in collapsed_symmetry_edges {
            if let Some(projected_target) = project_collapsed_symmetry_node(target, owner) {
                sized_collapsed_symmetry_neighbors
                    .entry(self.graph.nodes[source.0 as usize].tala_id)
                    .or_default()
                    .push(projected_target);
            }
            if let Some(projected_source) = project_collapsed_symmetry_node(source, owner) {
                sized_collapsed_symmetry_neighbors
                    .entry(self.graph.nodes[target.0 as usize].tala_id)
                    .or_default()
                    .push(projected_source);
            }
        }
        let mut collapsed_table_neighbors =
            BTreeMap::<i64, Vec<ProjectedTableColumnNeighbor>>::new();
        // Preserve each original Node.Edges slice order. Graph.Edges order is
        // not interchangeable here: reconnect removes cross-child entries,
        // while the surviving same-child entries retain their endpoint-local
        // order.
        for (node_index, node) in self.graph.nodes.iter().enumerate() {
            let original = NodeId(node_index as u32);
            for edge_id in node.edges.read().iter().copied() {
                let Some(&owner) = collapsed_table_column_edges.get(&edge_id) else {
                    continue;
                };
                let edge = &self.graph.edge_records[edge_id.0 as usize];
                let (other, column_index) = if edge.from == original {
                    (edge.to, edge.source_table_column)
                } else if edge.to == original {
                    (edge.from, edge.target_table_column)
                } else {
                    continue;
                };
                let Some(column_index) = column_index else {
                    continue;
                };
                let Some(other) = project_collapsed_symmetry_node(other, owner) else {
                    continue;
                };
                collapsed_table_neighbors
                    .entry(node.tala_id)
                    .or_default()
                    .push(ProjectedTableColumnNeighbor {
                        other,
                        column_index,
                    });
            }
        }
        for abduction in &mut sized_edge_abductions {
            if let Some(original) = abduction.originally_from {
                abduction.originally_from_table_neighbors = collapsed_table_neighbors
                    .get(&original.tala_id)
                    .cloned()
                    .unwrap_or_default();
            }
            if let Some(original) = abduction.originally_to {
                abduction.originally_to_table_neighbors = collapsed_table_neighbors
                    .get(&original.tala_id)
                    .cloned()
                    .unwrap_or_default();
            }
        }
        let collapsed_restored_mirror_edges = collapsed_restored_mirror_edges
            .into_iter()
            .map(
                |(carrier, from_tala_id, from_container, to_tala_id, to_container)| {
                    (
                        old_to_new[&carrier],
                        from_tala_id,
                        from_container,
                        to_tala_id,
                        to_container,
                    )
                },
            )
            .collect();
        let incident_edge_order = incident_edge_order
            .iter()
            .filter_map(|(old_child, old_edges)| {
                let projected_child = old_to_new.get(old_child).copied()?;
                Some((
                    projected_child,
                    old_edges
                        .iter()
                        .filter_map(|edge| projected_edge_old_to_new.get(edge).copied())
                        .collect(),
                ))
            })
            .collect();
        for (&old_tree_node, &projected_tree_node) in &tree_old_to_new {
            let state = self.graph.tree_routing_nodes[&old_tree_node];
            let projected_parent = tree_old_to_new
                .get(&state.parent)
                .copied()
                .or_else(|| old_to_new.get(&state.parent).copied())
                .expect("tree sentinel belongs to placement scope");
            let original = self
                .input
                .edges()
                .nth(state.sentinel_edge.0 as usize)
                .expect("tree sentinel edge")
                .1;
            let projected_edge = graph.add_edge(crate::Edge {
                source: if original.source == old_tree_node {
                    projected_tree_node
                } else {
                    projected_parent
                },
                target: if original.target == old_tree_node {
                    projected_tree_node
                } else {
                    projected_parent
                },
            });
            graph.tala_edge_ids[projected_edge.0 as usize] =
                Some(self.graph.edge_records[state.sentinel_edge.0 as usize].tala_id);
            graph.tala_edge_invisible[projected_edge.0 as usize] =
                Some(self.graph.edge_records[state.sentinel_edge.0 as usize].invisible);
            graph.tala_edge_label_fixed[projected_edge.0 as usize] =
                self.graph.edge_records[state.sentinel_edge.0 as usize].label_position_fixed;
            tree_edge_old_to_new.insert(state.sentinel_edge, projected_edge);
            let input_edge_id = self.graph.edge_records[state.sentinel_edge.0 as usize].input_id;
            graph.set_edge_arrows(projected_edge, self.input.edge_arrows(input_edge_id));
            graph.set_edge_arrowheads(projected_edge, self.input.edge_arrowheads(input_edge_id));
            graph.set_edge_label(
                projected_edge,
                self.input.edge_label(input_edge_id).cloned(),
            );
            graph.set_edge_arrowhead_labels(
                projected_edge,
                self.input.edge_arrowhead_labels(input_edge_id),
            );
            graph.set_edge_table_columns(
                projected_edge,
                self.input.edge_table_columns(input_edge_id),
            );
        }
        // Go's temporary childrenGraph appends restored tree edges to the
        // owning Graph.Edges slice, while retaining the declaration order of
        // the other edges. The stable arena materializes projected edges and
        // tree edges in separate passes, so its dense vector can otherwise
        // expose the tree subset before an earlier declaration edge. Replay
        // the owning edge order through both maps before any placement or
        // direct/routing pass observes this temporary graph.
        let temporary_edge_order = self
            .graph
            .edge_order
            .read()
            .iter()
            .copied()
            .filter_map(|old_edge| {
                projected_edge_old_to_new
                    .get(&old_edge)
                    .copied()
                    .or_else(|| tree_edge_old_to_new.get(&old_edge).copied())
            })
            .collect::<Vec<_>>();
        let tree_routing_nodes: BTreeMap<NodeId, TreeRoutingNode> = tree_old_to_new
            .iter()
            .map(|(&old, &new)| {
                let state = self.graph.tree_routing_nodes[&old];
                (
                    new,
                    TreeRoutingNode {
                        parent: tree_old_to_new
                            .get(&state.parent)
                            .copied()
                            .or_else(|| old_to_new.get(&state.parent).copied())
                            .expect("projected tree parent"),
                        sentinel_edge: tree_edge_old_to_new[&state.sentinel_edge],
                        orientation: state.orientation,
                        extraction_order: state.extraction_order,
                    },
                )
            })
            .collect();
        // Recovered Graph.assignNears groups edge abductions by the shared
        // outside endpoint ("uncle"). Distinct direct children connected to
        // that uncle become mutual near-neighbors unless they already have a
        // direct edge. Preserve every pair separately because TALA's Nears is
        // a set. Serialized graphjson input may also carry several explicit
        // peers; those are projected alongside the assigned pairs above.
        let directly_connected = projected_edges
            .iter()
            .map(|&(_, source, target)| {
                if source < target {
                    (source, target)
                } else {
                    (target, source)
                }
            })
            .collect::<BTreeSet<_>>();
        let mut assigned_nears = BTreeSet::new();
        for cousins in children_by_uncle.values() {
            if cousins.len() < 2 {
                continue;
            }
            for (index, &first) in cousins.iter().enumerate() {
                for &second in &cousins[index + 1..] {
                    let ordered = if first < second {
                        (first, second)
                    } else {
                        (second, first)
                    };
                    if !directly_connected.contains(&ordered) {
                        assigned_nears.insert((old_to_new[&ordered.0], old_to_new[&ordered.1]));
                    }
                }
            }
        }
        // CopyEntitiesFrom carries the actual map, including nil. Placement
        // projection must not invent groups when that source field is nil.
        let common_uncle_by_node = self.graph.common_uncle_siblings.read();
        // Each sibling points at the same group in CommonUncleSiblings. Keep
        // one stable copy of that group rather than replaying it once per
        // member; Go's map-of-slices representation has the same uniqueness.
        let mut common_uncle_groups = Vec::new();
        let mut seen_common_uncle_groups = BTreeSet::new();
        for siblings in common_uncle_by_node.values().map(|siblings| {
            siblings
                .iter()
                .filter_map(|child| old_to_new.get(child).copied())
                .collect::<Vec<_>>()
        }) {
            if siblings.len() > 1 && seen_common_uncle_groups.insert(siblings.clone()) {
                common_uncle_groups.push(siblings);
            }
        }
        if crate::internal::trace_env_enabled("WEFTAN_TRACE_COMMON_UNCLE") {
            eprintln!(
                "COMMON_UNCLE_RUST scope={:?} parent={:?} siblings={:?} groups={:?}",
                scope.map(|node| self.graph.nodes[node.0 as usize].tala_id),
                parent_scope.map(|node| self.graph.nodes[node.0 as usize].tala_id),
                common_uncle_by_node
                    .iter()
                    .map(|(uncle, cousins)| (
                        self.graph.nodes[uncle.0 as usize].tala_id,
                        cousins
                            .iter()
                            .map(|node| self.graph.nodes[node.0 as usize].tala_id)
                            .collect::<Vec<_>>(),
                    ))
                    .collect::<Vec<_>>(),
                common_uncle_groups
                    .iter()
                    .map(|group| {
                        group
                            .iter()
                            .map(|node| self.graph.nodes[node.0 as usize].tala_id)
                            .collect::<Vec<_>>()
                    })
                    .collect::<Vec<_>>(),
            );
        }
        let edge_abduction_nodes = abducted_children
            .into_iter()
            .map(|child| old_to_new[&child])
            .collect();
        // `Graph.placeNodes` passes the post-tree-preprocessing
        // `g.Containers[root]` slice to placeChildrenOrder.  The stable arena
        // keeps the original member nodes, so `scope_children` is only the
        // aggregate projection set; use the reconstructed container slice
        // that preserves the recovered AddSequence/AddCluster/tree order.
        let child_order = Self::place_children_order(&preprocessed_children, &edge_abductions);
        if crate::internal::trace_env_enabled("WEFTAN_TRACE_CHILD_ORDER") {
            let root_tala_id = parent_scope
                .map(|scope| self.graph.nodes[scope.0 as usize].tala_id)
                .unwrap_or(0);
            eprint!("CHILD_ORDER_RUST root={root_tala_id}:");
            for child in &child_order {
                eprint!(" {}", self.graph.nodes[child.0 as usize].tala_id);
            }
            eprintln!();
        }
        // AddHubs runs once on the owning graph and SplitSubgraphs carries
        // that pointer map into every temporary graph. Recomputing hubs from
        // a projected scope changes degrees after edge abduction and causes
        // Rust to consume the hub-retry RNG path for nodes that Go never
        // retries. Remap the already recovered map through this scope's
        // dense IDs instead.
        let hubs: BTreeMap<NodeId, Vec<NodeId>> = self
            .graph
            .hubs
            .iter()
            .filter_map(|(old_hub, old_spokes)| {
                let hub = old_to_new.get(&old_hub).copied()?;
                let spokes = old_spokes
                    .iter()
                    .filter_map(|spoke| old_to_new.get(spoke).copied())
                    .collect::<Vec<_>>();
                (!spokes.is_empty()).then_some((hub, spokes))
            })
            .collect();
        if crate::internal::trace_env_enabled("WEFTAN_TRACE_HUB_SCOPE") {
            eprint!(
                "HUB_SCOPE_RUST scope={:?} source=",
                scope.map(|id| self.graph.nodes[id.0 as usize].tala_id)
            );
            for (hub, spokes) in &self.graph.hubs {
                eprint!(" {}=[", self.graph.active_node_tala_id(hub));
                for spoke in spokes {
                    eprint!(" {}", self.graph.active_node_tala_id(spoke));
                }
                eprint!("]");
            }
            eprint!(" remapped=");
            for (hub, spokes) in &hubs {
                eprint!(" {}=[", hub.0);
                for spoke in spokes {
                    eprint!(" {}", spoke.0);
                }
                eprint!("]");
            }
            eprintln!();
        }
        let scope_contains_ancestor = |candidate: NodeId| {
            let mut current_scope = scope;
            while let Some(scope_node) = current_scope {
                if scope_node == candidate {
                    return true;
                }
                current_scope = self.graph.nodes[scope_node.0 as usize].container;
            }
            false
        };
        let project_external_node = |node: &ArenaNode| {
            let mut external = node.clone();
            external.owner_geometry_source = Some(node.input_id);
            external.owner_geometry_anchor = None;
            let trace_scope_external =
                crate::internal::trace_env_value("WEFTAN_TRACE_SCOPE_EXTERNAL").is_some_and(
                    |target| target == "all" || target.parse::<i64>().ok() == Some(node.tala_id),
                );
            let mut current = node.container;
            let mut correction = Point::default();
            while let Some(container) = current {
                let ancestor = &self.graph.nodes[container.0 as usize];
                if trace_scope_external {
                    eprintln!(
                        "SCOPE_CORRECTION_RUST scope={:?} node={} node_pos={:?} ancestor={} ancestor_container={:?} ancestor_pos={:?} translation={:?} scope_ancestor={} direct_child={}",
                        scope.map(|scope_node| self.graph.nodes[scope_node.0 as usize].tala_id),
                        node.tala_id,
                        node.position,
                        ancestor.tala_id,
                        ancestor
                            .container
                            .map(|parent| self.graph.nodes[parent.0 as usize].tala_id),
                        ancestor.position,
                        ancestor.unpositioned_scope_translation,
                        scope_contains_ancestor(container),
                        scope_children.contains(&container)
                    );
                }
                // A recursively fitted direct child of this temporary scope
                // is still represented in the same local frame as its shared
                // Go Node pointer.  Its `unpositioned_scope_translation` is
                // consumed by the scope's subsequent `syncNested`/copyback;
                // subtracting it here moves the child into a frame that Go
                // never uses.  An unrelated external ancestor, by contrast,
                // is already in the owning graph's absolute frame and does
                // need the translation removed (see the focused transaction
                // geometry test).
                if !scope_contains_ancestor(container)
                    && !scope_children.contains(&container)
                    && ancestor.position.is_none()
                    && !ancestor.scope_translation_materialized
                    && let Some(translation) = ancestor.unpositioned_scope_translation
                {
                    correction.x += translation.x;
                    correction.y += translation.y;
                }
                current = ancestor.container;
            }
            if correction != Point::default()
                && let Some(position) = external.position.as_mut()
            {
                position.x -= correction.x;
                position.y -= correction.y;
                external.rect.origin = *position;
            }
            if trace_scope_external {
                eprintln!(
                    "SCOPE_EXTERNAL_PROJECTED_RUST scope={:?} node={} raw={:?} correction={:?} projected={:?}",
                    scope.map(|scope_node| self.graph.nodes[scope_node.0 as usize].tala_id),
                    node.tala_id,
                    node.position,
                    correction,
                    external.position
                );
            }
            external.scoring_cluster_vessel = node
                .cluster
                .map(|cluster| self.graph.clusters[cluster].vessel_tala_id);
            if let Some(external_position) = external.position {
                for ancestor_tala_id in &external.scoring_container_ancestors {
                    let Some(ancestor) = scope_children.iter().copied().find(|ancestor| {
                        self.graph.nodes[ancestor.0 as usize].tala_id == *ancestor_tala_id
                    }) else {
                        continue;
                    };
                    let Some(ancestor_position) = self.graph.position(ancestor) else {
                        // InitializeNodes assigns a formerly nil TopLeft
                        // directly. TALA's shared children stay in their
                        // existing absolute frame until syncNested positions
                        // them; there is no parent translation to replay.
                        continue;
                    };
                    external.owner_geometry_anchor = Some(ancestor);
                    external.transaction_anchor_tala_id = Some(*ancestor_tala_id);
                    external.transaction_anchor_offset = Some(Point {
                        x: external_position.x - ancestor_position.x,
                        y: external_position.y - ancestor_position.y,
                    });
                    break;
                }
            }
            external
        };
        let project_external_active_node = |node_id: NodeId| {
            let active_position = self.graph.active_node_position(node_id);
            let parent = self.graph.active_node_container(node_id);
            let mut node = self.graph.nodes[node_id.0 as usize].clone();
            let aggregate_owner = self.graph.active_node_is_aggregate(node_id)
                && self.graph.active_aggregate_owner(node_id) == node_id;
            // An active aggregate's vessel position is local to the placement
            // scope that owns the aggregate. When another scope projects that
            // already-laid-out node through a shared container, the stable
            // member box carries the shared absolute pointer position instead.
            // Prefer that box outside the owning scope so a local vessel origin
            // cannot pull an unrelated external container back to (0, 0).
            let projected_position = if aggregate_owner && parent != scope {
                node.position.or(active_position)
            } else {
                active_position
            };
            let trace_active_projection =
                crate::internal::trace_env_value("WEFTAN_TRACE_ACTIVE_PROJECTION").is_some_and(
                    |target| target == "all" || target.parse::<i64>().ok() == Some(node.tala_id),
                );
            if trace_active_projection {
                eprintln!(
                    "ACTIVE_PROJECTION_RUST before scope={:?} node={} external_id={:?} raw_pos={:?} active_pos={:?} parent={:?} parent_id={:?} parent_pos={:?} translation={:?}",
                    scope.map(|scope_node| self.graph.nodes[scope_node.0 as usize]
                        .external_id
                        .as_str()),
                    node.tala_id,
                    node.external_id,
                    node.position,
                    active_position,
                    parent.map(|parent| self.graph.nodes[parent.0 as usize].tala_id),
                    parent.map(|parent| self.graph.nodes[parent.0 as usize].external_id.as_str()),
                    parent.and_then(|parent| self.graph.position(parent)),
                    node.unpositioned_scope_translation
                );
            }
            if aggregate_owner {
                // AddSequence/AddCluster allocate a fresh plain NewNode for
                // the vessel. Only its identity, box, container, and current
                // edge slice survive into Graph.Nodes; member decorations do
                // not become vessel transaction metadata.
                node.tala_id = self.graph.active_node_tala_id(node_id);
                node.position = projected_position;
                node.rect.origin = node.position.unwrap_or_default();
                node.rect.size = self.graph.active_node_size(node_id);
                node.declared_size = None;
                node.layout_margins = Insets::uniform(0.0);
                node.unpositioned_scope_translation = None;
                node.scope_translation_materialized = false;
                node.loop_offsets = None;
                node.herd_assignment = None;
                node.fixed_top_left = None;
                node.force_hierarchy = false;
                node.hierarchy = None;
                node.desired_width = None;
                node.desired_height = None;
                node.folded_label_min_size = None;
                node.shape = ShapeKind::Rectangle;
                node.label_size = None;
                node.font_size = None;
                node.label_position = LabelPosition::Unset;
                node.label_position_fixed = false;
                node.external_label = None;
                node.icon_position = None;
                node.has_icon = false;
                node.is_invisible = false;
                node.is_container = false;
                node.scoring_is_container = false;
                node.scoring_is_aggregate_vessel = true;
                node.scoring_cluster_vessel = None;
                node.sequence = None;
                node.cluster = None;
                node.transaction_anchor_tala_id = None;
                node.transaction_anchor_offset = None;
                node.transaction_position_was_nil = node.position.is_none();
                node.content_insets = Insets::uniform(0.0);
                node.node_padding = Insets::uniform(0.0);
                node.grid_rows = None;
                node.grid_columns = None;
                node.canvas_position = None;
                node.label_aware_grid = false;
                node.packed_grid = false;
                node.is_3d = false;
                node.is_multiple = false;
                node.table_column_count = None;
                node.edges.clear();
                node.nears.clear();
            }
            node.container = parent;
            node.scoring_container_parent =
                parent.map(|parent| self.graph.active_node_tala_id(parent));
            node.scoring_container_ancestors.clear();
            let mut ancestor = parent;
            while let Some(current) = ancestor {
                node.scoring_container_ancestors
                    .push(self.graph.active_node_tala_id(current));
                ancestor = self.graph.active_node_container(current);
            }
            let mut projected = project_external_node(&node);
            if aggregate_owner {
                // The member carrier and the allocated Source vessel are
                // distinct pointers. Retain the explicit topology binding.
                projected.owner_geometry_source = self
                    .graph
                    .active_cluster_index(node_id)
                    .and_then(|index| {
                        self.graph
                            .topology_vessel_node(TopologyVessel::Cluster(index))
                    })
                    .or_else(|| {
                        self.graph.active_sequence_index(node_id).and_then(|index| {
                            self.graph
                                .topology_vessel_node(TopologyVessel::Sequence(index))
                        })
                    });
            }
            if let Some(parent) = parent
                && let Some(active_position) = projected_position
                && let Some(parent_position) = self.graph.position(parent)
            {
                let parent_tala_id = self.graph.nodes[parent.0 as usize].tala_id;
                projected.owner_geometry_anchor = Some(parent);
                projected.transaction_anchor_tala_id = Some(parent_tala_id);
                projected.transaction_anchor_offset = Some(Point {
                    x: active_position.x - parent_position.x,
                    y: active_position.y - parent_position.y,
                });
            }
            if trace_active_projection {
                eprintln!(
                    "ACTIVE_PROJECTION_RUST after node={} projected_pos={:?} anchor={:?} offset={:?}",
                    projected.tala_id,
                    projected.position,
                    projected.transaction_anchor_tala_id,
                    projected.transaction_anchor_offset
                );
            }
            projected
        };
        let projected_transaction_nodes = self
            .graph
            .graph_node_order()
            .into_iter()
            .map(|node_id| {
                let node = project_external_active_node(node_id);
                let container_tala_id = self
                    .graph
                    .active_node_container(node_id)
                    .map(|container| self.graph.active_node_tala_id(container));
                let ancestor_tala_ids = node.scoring_container_ancestors.clone();
                let child_tala_ids = if node.is_container {
                    let mut seen = BTreeSet::new();
                    self.graph
                        .container_node_order(Some(node_id))
                        .into_iter()
                        .map(|child| self.graph.active_aggregate_owner(child))
                        .filter(|child| seen.insert(*child))
                        .map(|child| self.graph.active_node_tala_id(child))
                        .collect()
                } else {
                    Vec::new()
                };
                let edges = self
                    .graph
                    .active_edge_ids(node_id)
                    .into_iter()
                    .map(|edge_id| {
                        let edge = &self.graph.edge_records[edge_id.0 as usize];
                        ProjectedTransactionEdge {
                            adjacent_tala_id: self
                                .graph
                                .active_node_tala_id(self.graph.active_adjacent(node_id, edge_id)),
                            min_width: edge.min_width,
                            min_height: edge.min_height,
                        }
                    })
                    .collect();
                let structural_container_tala_id = node
                    .container
                    .map(|id| self.graph.nodes[id.0 as usize].tala_id);
                let mut structural_ancestor_tala_ids = Vec::new();
                let mut structural_parent = node.container;
                while let Some(parent) = structural_parent {
                    structural_ancestor_tala_ids.push(self.graph.nodes[parent.0 as usize].tala_id);
                    structural_parent = self.graph.active_node_container(parent);
                }
                ProjectedTransactionNode {
                    structural_container_tala_id,
                    structural_ancestor_tala_ids,
                    node,
                    container_tala_id,
                    ancestor_tala_ids,
                    child_tala_ids,
                    edges,
                }
            })
            .collect::<Vec<_>>();
        // AddCluster/AddSequence leave the aggregate members in the owning
        // Graph.Containers slice even though the temporary children graph
        // exposes only the vessel.  The recovered moveNodeWithChildren walk
        // therefore translates those hidden member boxes whenever it moves a
        // vessel.  Keep a separate projected slice keyed by each vessel so
        // transaction movement can follow the same pointer-sharing walk
        // without treating the hidden members as ordinary fitting children.
        let project_external_aggregate_member =
            |node_id: NodeId, position: Option<Point>, size: Option<Size>| {
                let mut node = self.graph.nodes[node_id.0 as usize].clone();
                if let Some(position) = position {
                    node.position = Some(position);
                    node.rect.origin = position;
                }
                if let Some(size) = size {
                    node.rect.size = size;
                }
                // AddCluster removes the absorbed members from the owning
                // Graph.Containers slice and clears their direct Container. Their
                // boxes remain absolute shared pointers; only a missing position
                // is anchored to the synthetic vessel below.
                project_external_node(&node)
            };
        // `Graph.CopyEntitiesFrom` shares the complete Containers map with
        // every recursive children graph. Consequently, each Go transaction
        // refits every positioned container key, including sibling branches
        // that are absent from the temporary graph's Nodes slice. Project
        // those non-active keys too so committed Rust transactions preserve
        // the same shared-container side effects.
        let transaction_external_containers = self
            .graph
            .nodes
            .iter()
            // Graph.CopyEntitiesFrom retains every source container pointer in
            // the temporary graph's Containers map, including the scope
            // carrier while its TopLeft is still nil.  The Go transaction
            // skips a nil container only when it has no materialized child
            // geometry; keep the alias here so a child rotation can still be
            // checked against its source container bounds.  Fixed and
            // hierarchy-owned nodes remain positioned independently.
            .filter(|node| {
                node.is_container
                    && (node.position.is_some()
                        || self
                            .graph
                            .containers
                            .get(&Some(node.input_id))
                            .is_some_and(|children| children.len() >= 2))
                    && (crate::internal::trace_env_enabled("WEFTAN_TRACE_GRID_BROAD_EXTERNAL")
                        || node.fixed_top_left.is_some()
                        || node.hierarchy.is_some()
                        || externalized_materialized_container(node.input_id)
                        // Go's root Graph.Containers slice retains some
                        // positioned aliases outside an induced scope. The
                        // active-arena validation below handles those
                        // aliases without adding them to scoring obstacles.
                        || materialized_parent_scopes.contains(&node.container)
                        || node.grid_rows.is_some()
                        || node.grid_columns.is_some())
            })
            .map(|node| {
                let mut projected = project_external_node(node);
                if crate::internal::trace_env_enabled("WEFTAN_TRACE_EXTERNAL_MARKERS") {
                    eprintln!("EXTERNAL_MARKER_NODE id={} force={} hierarchy={:?} scoring={} edges={} children={:?} child_meta={:?}", node.tala_id, node.force_hierarchy, node.hierarchy.clone(), node.scoring_is_container, node.edges.len(), self.graph.containers.get(&Some(node.input_id)).map(|c| c.len()), self.graph.containers.get(&Some(node.input_id)).map(|children| children.iter().map(|id| { let child=&self.graph.nodes[id.0 as usize]; (child.tala_id, child.hierarchy.clone(), child.force_hierarchy, child.scoring_container_parent, child.scoring_is_container) }).collect::<Vec<_>>()));
                }
                projected.transaction_validation_only = externalized_materialized_container(node.input_id)
                    && !has_hierarchy_descendant(node.input_id)
                    && !has_internal_edge(node.input_id);
                projected
            })
            .collect::<Vec<_>>();
        // Materialized containers are visible to Transaction.IsBadState as
        // external obstacles, but they are not part of the movable graph.
        // Keep this inventory separate so induced placement graphs can use
        // the validation boxes without allowing repositionContainers to
        // refit or translate them during every speculative pack.
        let transaction_validation_external_containers = Vec::new();
        if crate::internal::trace_env_enabled("WEFTAN_TRACE_EXTERNAL_BUILD") {
            eprintln!(
                "EXTERNAL_BUILD_RUST scope={:?} {:?}",
                scope.map(|scope_node| self.graph.nodes[scope_node.0 as usize].tala_id),
                transaction_external_containers
                    .iter()
                    .map(|node| (
                        node.tala_id,
                        node.position,
                        node.rect.origin,
                        node.rect.size
                    ))
                    .collect::<Vec<_>>()
            );
            eprintln!(
                "EXTERNAL_CANDIDATES_RUST {:?}",
                self.graph
                    .nodes
                    .iter()
                    .filter(|node| node.is_container)
                    .map(|node| {
                        (
                            node.tala_id,
                            node.container,
                            node.position,
                            node.fixed_top_left,
                            node.hierarchy.clone(),
                            node.grid_rows,
                            node.grid_columns,
                        )
                    })
                    .collect::<Vec<_>>()
            );
            if crate::internal::trace_env_enabled("WEFTAN_TRACE_EXTERNAL_DETAILS") {
                eprintln!(
                    "EXTERNAL_DETAILS_RUST scope={:?} {:?}",
                    scope.map(|scope_node| self.graph.nodes[scope_node.0 as usize].tala_id),
                    self.graph
                        .nodes
                        .iter()
                        .filter(|node| node.is_container)
                        .map(|node| (
                            node.tala_id,
                            node.container
                                .map(|id| self.graph.nodes[id.0 as usize].tala_id),
                            node.position,
                            node.initial_position,
                            node.cluster,
                            node.sequence,
                            node.hierarchy.clone(),
                            node.force_hierarchy,
                            node.edges.len(),
                            node.scoring_is_container,
                        ))
                        .collect::<Vec<_>>()
                );
            }
            eprintln!(
                "EXTERNAL_SOURCE_CHILDREN_RUST {:?}",
                self.graph
                    .containers
                    .iter()
                    .filter_map(|(parent, children)| {
                        let parent = parent.map(|id| self.graph.nodes[id.0 as usize].tala_id)?;
                        Some((
                            parent,
                            children
                                .iter()
                                .map(|child| {
                                    let node = &self.graph.nodes[child.0 as usize];
                                    (node.tala_id, node.position)
                                })
                                .collect::<Vec<_>>(),
                        ))
                    })
                    .collect::<Vec<_>>()
            );
        }
        let transaction_external_container_children: BTreeMap<i64, Vec<ArenaNode>> = self
            .graph
            .nodes
            .iter()
            // SplitSubgraphs shares the complete Containers map, including a
            // container whose TopLeft is still nil. InitializeNodes may place
            // that container in the temporary graph, after which syncNested
            // immediately calls positionContainerChildren on its retained
            // children. Transaction.repositionContainers separately skips
            // unpositioned container keys when it builds its sorted worklist.
            .filter(|container| container.is_container)
            .filter_map(|container| {
                let mut children = self
                    .graph
                    .container_node_order(Some(container.input_id))
                    .into_iter()
                    .collect::<Vec<_>>();
                if children.is_empty() {
                    children = self
                        .graph
                        .containers
                        .get(&Some(container.input_id))
                        .unwrap_or_default();
                }
                if children.is_empty()
                    && let Some(source_order) = self
                        .graph
                        .source_container_child_order
                        .get(&container.tala_id)
                {
                    // The temporary graph may have removed the nested child
                    // nodes from its live Containers slice, while the owning
                    // graph still exposes those shared pointers to Go's
                    // optimizer. Reconstruct the projected child inventory
                    // from the stable serialized order before creating the
                    // transaction view.
                    children = source_order
                        .iter()
                        .filter_map(|child_tala_id| {
                            self.graph
                                .nodes
                                .iter()
                                .position(|candidate| candidate.tala_id == *child_tala_id)
                                .map(|index| NodeId(index as u32))
                        })
                        .collect();
                }
                // ExtractTrees removes branching-tree descendants from the
                // active Graph.Nodes/Containers slice, but CopyEntitiesFrom
                // keeps the complete Containers map available to the nested
                // sync pass.  The stable arena keeps those descendants in
                // `tree_routing_nodes`; project them back into the external
                // child slice so a parent scope follows the same shared-node
                // movement before the next optimizer is constructed.
                for tree_child in self
                    .graph
                    .nodes
                    .iter()
                    .filter(|node| {
                        node.container == Some(container.input_id)
                            && self.graph.tree_routing_nodes.contains_key(&node.input_id)
                    })
                    .map(|node| node.input_id)
                {
                    if !children.contains(&tree_child) {
                        children.push(tree_child);
                    }
                }
                // CopyEntitiesFrom shares the live Containers slice, including
                // tree-restoration order. An explicit direction does not reset
                // that slice to the immutable source declaration order.
                // Graph.Containers stores child boxes in the owning
                // container's local frame. Go's shared *Node pointers retain
                // that frame across induced scopes, including an external
                // container that sends an edge into the current scope. Do
                // not subtract the parent's absolute position here: doing so
                // converts an already-local child into a second relative
                // frame and makes wrapChildren fit the carrier at a bogus
                // offset (for example, -46,-728 instead of 0,0).
                let children = children
                    .into_iter()
                    .map(project_external_active_node)
                    .collect::<Vec<_>>();
                if crate::internal::trace_env_enabled("WEFTAN_TRACE_EXTERNAL_CHILDREN") {
                    eprintln!(
                        "EXTERNAL_CHILDREN_RUST scope={:?} parent={} children={:?}",
                        scope.map(|scope_node| self.graph.nodes[scope_node.0 as usize].tala_id),
                        container.tala_id,
                        children
                            .iter()
                            .map(|child| (
                                child.tala_id,
                                child.position,
                                child.scoring_is_aggregate_vessel
                            ))
                            .collect::<Vec<_>>()
                    );
                }
                (!children.is_empty()).then_some((container.tala_id, children))
            })
            .collect();
        let mut transaction_external_aggregate_children: BTreeMap<i64, Vec<ArenaNode>> =
            BTreeMap::new();
        let mut transaction_external_cluster_layouts =
            BTreeMap::<i64, ExternalClusterLayout>::new();
        // A cluster or sequence can already have been materialized in a
        // nested placement scope while its member pointers remain visible in
        // an ancestor's Containers map.  The recovered Go walk follows those
        // live pointers when moving an ancestor, even though the temporary
        // vessel is not one of the ancestor's direct children.  Use the
        // original container chain as the fallback relevance test so those
        // aggregate members remain available to the ancestor transaction.
        let raw_is_descendant = |node: NodeId, ancestor: NodeId| {
            let mut current = Some(node);
            while let Some(candidate) = current {
                if candidate == ancestor {
                    return true;
                }
                current = self.graph.nodes[candidate.0 as usize].container;
            }
            false
        };
        let mut transaction_aggregates = aggregate_by_owner.values().copied().collect::<Vec<_>>();
        for (sequence_index, sequence) in self.graph.sequences.iter().enumerate() {
            let Some(&owner) = sequence.members.first() else {
                continue;
            };
            let relevant = scope.is_none()
                || scope.is_some_and(|scope| {
                    self.graph.active_is_descendant_of(owner, scope)
                        || raw_is_descendant(owner, scope)
                });
            if relevant {
                let aggregate = PlacementAggregate::Sequence(sequence_index);
                if !transaction_aggregates.iter().any(|candidate| {
                    matches!(candidate, PlacementAggregate::Sequence(index) if *index == sequence_index)
                }) {
                    transaction_aggregates.push(aggregate);
                }
            }
        }
        for (cluster_index, cluster) in self.graph.clusters.iter().enumerate() {
            let Some(&owner) = cluster.members.first() else {
                continue;
            };
            let relevant = scope.is_none()
                || scope.is_some_and(|scope| {
                    self.graph.active_is_descendant_of(owner, scope)
                        || raw_is_descendant(owner, scope)
                });
            if relevant {
                let aggregate = PlacementAggregate::Cluster(cluster_index);
                if !transaction_aggregates.iter().any(|candidate| {
                    matches!(candidate, PlacementAggregate::Cluster(index) if *index == cluster_index)
                }) {
                    transaction_aggregates.push(aggregate);
                }
            }
        }
        for aggregate in transaction_aggregates {
            let (vessel_tala_id, members) = match aggregate {
                PlacementAggregate::Sequence(sequence_index) => {
                    let sequence = &self.graph.sequences[sequence_index];
                    (sequence.vessel_tala_id, &sequence.members)
                }
                PlacementAggregate::Cluster(cluster_index) => {
                    let cluster = &self.graph.clusters[cluster_index];
                    transaction_external_cluster_layouts.insert(
                        cluster.vessel_tala_id,
                        ExternalClusterLayout {
                            arrangement: cluster.arrangement,
                            padding: cluster.padding,
                            fixed_size: cluster.fixed_size,
                        },
                    );
                    (cluster.vessel_tala_id, &cluster.members)
                }
            };
            if let Some(target) = crate::internal::trace_env_value("WEFTAN_AGGREGATE_TARGET")
                && target != "all"
                && target.parse::<i64>().ok() != Some(vessel_tala_id)
            {
                continue;
            }
            let projected_members = members
                .iter()
                .copied()
                .map(|member| {
                    let geometry = match aggregate {
                        PlacementAggregate::Sequence(sequence_index) => self
                            .graph
                            .sequence_member_geometry(sequence_index, member),
                        PlacementAggregate::Cluster(cluster_index) => self
                            .graph
                            .cluster_member_geometry(cluster_index, member),
                    };
                    let (member_offset, member_size) = geometry
                        .map(|(mut offset, size)| {
                        // A projected cluster member is a live Go pointer. Its
                        // pre-sync position is the rounded target computed
                        // from the vessel's *current* frame, then translated
                        // with that vessel into the temporary graph. Using a
                        // fresh origin here loses that phase at half-cell
                        // centers (67.5 becomes 68 instead of the live 67).
                        if matches!(aggregate, PlacementAggregate::Cluster(_)) {
                            let cluster_index = match aggregate {
                                PlacementAggregate::Cluster(index) => index,
                                PlacementAggregate::Sequence(_) => unreachable!(),
                            };
                            let arrangement = self.graph.clusters[cluster_index].arrangement;
                            let prior_vessel = self
                                .graph
                                .pending_cluster_vessel_positions
                                .get(&cluster_index)
                                .copied()
                                .unwrap_or_default();
                            let vessel_size = self.graph.cluster_vessel_size_for_arrangement_with_padding(
                                cluster_index,
                                arrangement,
                                self.graph.clusters[cluster_index].padding,
                            );
                            if crate::internal::trace_env_enabled("WEFTAN_TRACE_AGG_OFFSET") {
                                eprintln!(
                                    "AGG_OFFSET_INPUT_RUST cluster={} member={:?} member_pos={:?} member_size={:?} base_offset={:?} prior_vessel={:?} vessel_size={:?}",
                                    cluster_index, member, self.graph.position(member), size, offset, prior_vessel, vessel_size
                                );
                            }
                            let live_target = match arrangement {
                                ClusterArrangement::Row => {
                                    let center = prior_vessel.y + vessel_size.height / 2.0;
                                    Point {
                                        x: prior_vessel.x + offset.x,
                                        y: (center - size.height / 2.0).round(),
                                    }
                                }
                                ClusterArrangement::Column => {
                                    let center = prior_vessel.x + vessel_size.width / 2.0;
                                    Point {
                                        x: (center - size.width / 2.0).round(),
                                        y: prior_vessel.y + offset.y,
                                    }
                                }
                            };
                            offset = Point {
                                x: live_target.x - prior_vessel.x,
                                y: live_target.y - prior_vessel.y,
                            };
                        }
                            (offset, size)
                        })
                        .map_or((None, None), |(offset, size)| (Some(offset), Some(size)));
                    // A recursively placed aggregate member is a shared Go
                    // pointer.  At an ancestor scope its stable arena box
                    // still carries the inner-scope frame, while the live
                    // vessel copy in `Containers` has already been translated
                    // by the child placement.  Use that vessel copy when it
                    // exists; falling back to the stable member preserves the
                    // initial inner scope where no ancestor projection exists.
                    let vessel_tala_id = match aggregate {
                        PlacementAggregate::Sequence(sequence_index) => {
                            self.graph.sequences[sequence_index].vessel_tala_id
                        }
                        PlacementAggregate::Cluster(cluster_index) => {
                            self.graph.clusters[cluster_index].vessel_tala_id
                        }
                    };
                    // When this aggregate is one of the current scope's
                    // direct children, the temporary Go graph has already
                    // arranged the member pointer inside its synthetic
                    // vessel.  The stable arena may still hold the member's
                    // prior absolute frame from the enclosing scope; using
                    // that value here makes FitToGraph see a wildly enlarged
                    // or shifted subtree.  Project the local aggregate
                    // geometry first, while ancestor scopes continue to use
                    // the shared member position published by their vessel.
                    let local_scope_vessel = scope_children.iter().any(|node| {
                        self.graph.active_node_is_aggregate(*node)
                            && self.graph.active_node_tala_id(*node) == vessel_tala_id
                    });
                    let local_scope_aggregate = local_scope_vessel
                        || aggregate_by_owner.values().any(|candidate| match (candidate, aggregate) {
                            (PlacementAggregate::Sequence(left), PlacementAggregate::Sequence(right)) => *left == right,
                            (PlacementAggregate::Cluster(left), PlacementAggregate::Cluster(right)) => *left == right,
                            _ => false,
                        });
                    if crate::internal::trace_env_enabled("WEFTAN_TRACE_LOCAL_AGG") {
                        eprintln!(
                            "LOCAL_AGG_RUST scope={:?} vessel={} local_vessel={} local_aggregate={} scope_children={:?} aggregate={:?} map={:?}",
                            scope.map(|node| self.graph.nodes[node.0 as usize].tala_id),
                            vessel_tala_id,
                            local_scope_vessel,
                            local_scope_aggregate,
                            scope_children
                                .iter()
                                .map(|node| self.graph.nodes[node.0 as usize].tala_id)
                                .collect::<Vec<_>>(),
                            aggregate,
                            aggregate_by_owner
                        );
                    }
                    let projected_vessel_position = self
                        .graph
                        .transaction_external_container_children
                        .values()
                        .flatten()
                        .find(|candidate| candidate.tala_id == vessel_tala_id)
                        .and_then(|candidate| candidate.position);
                    // The current temporary graph starts non-fixed vessels at
                    // the origin before its orthogonal placement pass. Use
                    // that local frame only when this scope owns the
                    // aggregate. For an aggregate nested below this scope,
                    // the projected vessel may still be in its immediate
                    // container's local frame (for example, (0, 0) inside a
                    // nested container), while the stable member pointer has
                    // already been translated into this scope's frame. Recover
                    // the vessel origin from that live member and its layout
                    // offset before considering the local alias.
                    let live_vessel_position = member_offset
                        .zip(self.graph.position(members[0]))
                        .map(|(offset, member_position)| Point {
                            x: member_position.x - offset.x,
                            y: member_position.y - offset.y,
                        });
                    let vessel = if local_scope_aggregate {
                        Some(Point::default())
                    } else {
                        live_vessel_position.or(projected_vessel_position)
                    };
                    // Go's induced graph shares the original member pointers.
                    // A cluster vessel sync then moves those live boxes from
                    // their pre-arrange absolute frame to the vessel's local
                    // target. Preserve that raw frame when it exists; eagerly
                    // materializing the canonical local offset skips the
                    // first rounded-delta move (67 instead of 68 at a
                    // half-pixel center).
                    let local_position = local_scope_aggregate
                        .then(|| self.graph.position(member))
                        .flatten()
                        .or_else(|| {
                            member_offset.zip(vessel).map(|(offset, vessel)| Point {
                                x: vessel.x + offset.x,
                                y: vessel.y + offset.y,
                            })
                        });
                    let position = local_position.or_else(|| self.graph.position(member)).or_else(|| {
                        member_offset.and_then(|offset| {
                            vessel.map(|vessel| Point {
                                x: vessel.x + offset.x,
                                y: vessel.y + offset.y,
                            })
                        })
                    });
                    let size = member_size;
                    if let Some(trace_target) = crate::internal::trace_env_value("WEFTAN_TRACE_MOVE_PROJECTION")
                        && (trace_target == "all"
                            || trace_target.parse::<i64>().ok()
                                == Some(self.graph.nodes[member.0 as usize].tala_id))
                    {
                        eprintln!(
                            "MOVE_PROJECTION_AGGREGATE_RUST vessel={} member={} geometry={:?} vessel_pos={:?} raw_pos={:?}",
                            vessel_tala_id,
                            self.graph.nodes[member.0 as usize].tala_id,
                            geometry,
                            vessel,
                            self.graph.position(member)
                        );
                    }
                    let mut projected = project_external_aggregate_member(member, position, size);
                    // Synthetic arrangement coordinates must not consume the
                    // member pointer's nil state. ArrangeClusterNodes uses that
                    // state to position a container's already-laid-out children.
                    if local_scope_aggregate && self.graph.position(member).is_none() {
                        projected.transaction_position_was_nil = true;
                    }

                    if position.is_none() {
                        projected.transaction_anchor_tala_id = Some(vessel_tala_id);
                        projected.transaction_anchor_offset =
                            Some(member_offset.unwrap_or_default());
                    }
                    projected
                })
                .collect::<Vec<_>>();
            if !projected_members.is_empty() {
                transaction_external_aggregate_children
                    .entry(vessel_tala_id)
                    .or_default()
                    .extend(projected_members);
            }
        }
        if crate::internal::trace_env_enabled("WEFTAN_DISABLE_AGGREGATE_PROJECTION") {
            transaction_external_aggregate_children.clear();
        }
        if crate::internal::trace_env_enabled("WEFTAN_TRACE_MOVE_PROJECTION") {
            for node in &self.graph.nodes {
                let trace_target = crate::internal::trace_env_value("WEFTAN_TRACE_MOVE_PROJECTION");
                if trace_target.as_deref() == Some("all")
                    || trace_target
                        .as_deref()
                        .and_then(|target| target.parse::<i64>().ok())
                        .is_some_and(|target| target == node.tala_id)
                {
                    eprintln!(
                        "MOVE_PROJECTION_SCOPE_RUST node={} input={} container={:?} is_container={} sequence={:?} cluster={:?} active_seq={:?} active_cluster={:?}",
                        node.tala_id,
                        node.input_id.0,
                        node.container
                            .map(|container| self.graph.nodes[container.0 as usize].tala_id),
                        node.is_container,
                        node.sequence,
                        node.cluster,
                        self.graph.active_sequence_index(node.input_id),
                        self.graph.active_cluster_index(node.input_id)
                    );
                }
            }
            for (index, cluster) in self.graph.clusters.iter().enumerate() {
                let trace_target = crate::internal::trace_env_value("WEFTAN_TRACE_MOVE_PROJECTION");
                if trace_target.as_deref() == Some("all")
                    || trace_target
                        .as_deref()
                        .and_then(|target| target.parse::<i64>().ok())
                        .is_some_and(|target| {
                            cluster
                                .members
                                .iter()
                                .any(|member| self.graph.nodes[member.0 as usize].tala_id == target)
                        })
                {
                    eprintln!(
                        "MOVE_PROJECTION_SCOPE_RUST cluster={} members={:?} vessel={} arrangement={:?}",
                        index,
                        cluster
                            .members
                            .iter()
                            .map(|member| self.graph.nodes[member.0 as usize].tala_id)
                            .collect::<Vec<_>>(),
                        cluster.vessel_tala_id,
                        cluster.arrangement
                    );
                }
            }
            eprintln!(
                "MOVE_PROJECTION_SCOPE_RUST containers={:?}",
                transaction_external_container_children
                    .iter()
                    .map(|(parent, children)| (
                        *parent,
                        children
                            .iter()
                            .map(|child| {
                                (
                                    child.tala_id,
                                    child.is_container,
                                    child.container.map(|container| {
                                        self.graph.nodes[container.0 as usize].tala_id
                                    }),
                                )
                            })
                            .collect::<Vec<_>>()
                    ))
                    .collect::<Vec<_>>()
            );
            eprintln!(
                "MOVE_PROJECTION_SCOPE_RUST aggregate_children={:?}",
                transaction_external_aggregate_children
                    .iter()
                    .map(|(parent, children)| (
                        *parent,
                        children
                            .iter()
                            .map(|child| (child.tala_id, child.position))
                            .collect::<Vec<_>>()
                    ))
                    .collect::<Vec<_>>()
            );
        }
        let projected_clusters: Vec<ProjectedCluster> = aggregate_by_owner
            .iter()
            .filter_map(|(&owner, aggregate)| {
                let PlacementAggregate::Cluster(cluster_index) = *aggregate else {
                    return None;
                };
                let cluster = &self.graph.clusters[cluster_index];
                let layout = |arrangement| {
                    // A fresh TALA layout keeps the padding selected by
                    // AddClusters.  The temporary cluster vessel shares that
                    // value with every induced placement graph, even after
                    // its member containers have been fitted and their live
                    // boxes have grown.  Recomputing from those post-fit
                    // dimensions incorrectly turns a 20px gap into (for
                    // example) 67px on nested clusters.  Prearranged
                    // graphjson snapshots are the one case where Go derives
                    // the flip candidate's gap from the supplied spacing.
                    let padding = if cluster.members.iter().all(|member| {
                        self.graph.nodes[member.0 as usize]
                            .initial_position
                            .is_some()
                    }) {
                        self.graph
                            .prearranged_cluster_padding(cluster_index, arrangement)
                    } else {
                        cluster.padding
                    };
                    if crate::internal::trace_env_enabled("WEFTAN_TRACE_PROJECTED_DETAIL")
                        && cluster.vessel_tala_id == 6129484611666145821
                    {
                        eprintln!(
                            "PROJECTED_DETAIL_RUST scope={:?} vessel={} cluster_arr={:?} requested={:?} cache={:?} computed={:?} members={:?}",
                            scope.map(|node| self.graph.nodes[node.0 as usize].tala_id),
                            cluster.vessel_tala_id,
                            cluster.arrangement,
                            arrangement,
                            self.graph.cluster_vessel_size(cluster_index),
                            self.graph.cluster_vessel_size_for_arrangement_with_padding(cluster_index, arrangement, padding),
                            cluster.members.iter().map(|member| (self.graph.nodes[member.0 as usize].tala_id, self.graph.nodes[member.0 as usize].rect.size)).collect::<Vec<_>>()
                        );
                    }
                    let members = cluster
                        .members
                        .iter()
                        .filter_map(|member| {
                            let tala_id = self.graph.nodes[member.0 as usize].tala_id;
                            self.graph
                                .cluster_member_geometry_for_arrangement_with_padding(
                                    cluster_index,
                                    *member,
                                    arrangement,
                                    padding,
                                )
                                .map(|geometry| (tala_id, geometry))
                        })
                        .collect();
                    ProjectedClusterLayout {
                        size: self.graph.cluster_vessel_size_for_arrangement_with_padding(
                            cluster_index,
                            arrangement,
                            padding,
                        ),
                        padding,
                        members,
                    }
                };
                Some(ProjectedCluster {
                    node: old_to_new[&owner],
                    cluster_index,
                    arrangement: cluster.arrangement,
                    desired_arrangement: cluster.desired_arrangement,
                    row: layout(ClusterArrangement::Row),
                    column: layout(ClusterArrangement::Column),
                })
            })
            .collect();
        if crate::internal::trace_env_enabled("WEFTAN_TRACE_PROJECTED_CLUSTERS") {
            eprintln!(
                "PROJECTED_CLUSTERS_RUST scope={:?} scope_tala={} {:?}",
                scope,
                scope
                    .map(|node| self.graph.nodes[node.0 as usize].tala_id)
                    .unwrap_or_default(),
                projected_clusters
                    .iter()
                    .map(|projection| (
                        self.graph.nodes[projection.node.0 as usize].tala_id,
                        projection.cluster_index,
                        projection.arrangement,
                        projection.row.size,
                        projection.column.size,
                        projection.row.members.clone(),
                        projection.column.members.clone(),
                    ))
                    .collect::<Vec<_>>()
            );
        }
        // The temporary children graph carries a projected edge slice, but
        // mirrorAxes still walks the original shared edge endpoints. Keep the
        // adapter metadata attached to this new Graph so the next
        // ArenaGraph::from_input does not fall back to the projected carrier
        // endpoints.
        graph.source_edge_endpoints = self.graph.source_edge_endpoints.clone();
        // Preserve the owning arena's reconstructed stable map. The adapter
        // input can have no graphjson map (measured graphs only carry the
        // endpoint vector), while the arena has already keyed that vector by
        // each edge's durable TALA identity before preprocessing reordered it.
        graph.source_edge_endpoints_by_id = self.graph.source_edge_endpoints_by_id.clone();
        // CombineSubgraphs starts with each active direct child and also adds
        // every descendant of that child to its returned graph. Keep the
        // complete source inventory here so the later source-orientation pass
        // sees the same node count as Go's combined graph, including tree and
        // aggregate members that are carried outside this projected arena.
        let mut source_tala_ids = BTreeSet::new();
        source_tala_ids.extend(self.graph.nodes.iter().map(|node| node.tala_id));
        source_tala_ids.extend(
            self.graph
                .transaction_external_containers
                .iter()
                .map(|node| node.tala_id),
        );
        source_tala_ids.extend(
            self.graph
                .transaction_external_container_children
                .values()
                .flatten()
                .map(|node| node.tala_id),
        );
        source_tala_ids.extend(
            self.graph
                .transaction_external_aggregate_children
                .values()
                .flatten()
                .map(|node| node.tala_id),
        );
        // CombineSubgraphs includes each active cluster vessel in its returned
        // Graph.Nodes inventory even though the stable source arena keeps the
        // member nodes as the cluster's identity carrier. Include that runtime
        // node when reconstructing the source graph's node count.
        source_tala_ids.extend(
            self.graph
                .clusters
                .iter()
                .filter(|cluster| self.graph.cluster_is_active(cluster))
                .map(|cluster| cluster.vessel_tala_id),
        );
        let mut owning_graph_node_count = scope
            .and_then(|owner| {
                let owner_tala_id = self.graph.nodes[owner.0 as usize].tala_id;
                transaction_external_container_children
                    .get(&owner_tala_id)
                    .map(Vec::len)
            })
            .filter(|count| *count > 0)
            .unwrap_or(self.graph.nodes.len());
        // Go's recursive placeNodes receives the owning graph, whose Nodes
        // slice still contains every descendant of a container. The Rust
        // placement projection may expose only one direct carrier and keep
        // descendants in transaction maps, so counting only that carrier
        // incorrectly takes the singleton fast path. Reconstruct the source
        // subtree cardinality from durable TALA identities before deciding
        // whether ordinary placement passes run.
        if let Some(owner) = scope {
            let owner_tala_id = self.graph.nodes[owner.0 as usize].tala_id;
            let subtree_count = source_tala_ids
                .iter()
                .filter(|tala_id| {
                    **tala_id == owner_tala_id
                        || self
                            .graph
                            .source_is_descendant_of_tala(**tala_id, owner_tala_id)
                })
                .count();
            if crate::internal::trace_env_enabled("WEFTAN_TRACE_SCOPE_STEPS") {
                eprintln!(
                    "SCOPE_COUNT_RUST owner={} map_count={} subtree_count={} nodes={} children={:?}",
                    owner_tala_id,
                    owning_graph_node_count,
                    subtree_count,
                    self.graph.nodes.len(),
                    transaction_external_container_children
                        .get(&owner_tala_id)
                        .map(|children| children
                            .iter()
                            .map(|node| node.tala_id)
                            .collect::<Vec<_>>())
                );
            }
            owning_graph_node_count = owning_graph_node_count.max(subtree_count);
        }
        // Tree extraction removes retained descendants from the temporary
        // `Graph.Nodes` slice, but the owning Go graph still counts those
        // direct children when deciding whether to run its placement passes.
        // Include the auxiliary tree nodes in that lifecycle count.
        owning_graph_node_count = owning_graph_node_count.max(
            self.graph
                .nodes
                .len()
                .saturating_add(tree_auxiliary_nodes.len()),
        );
        let mut restored_endpoint_order = Vec::new();
        for abduction in &sized_edge_abductions {
            let original_edge = abduction.source_edge;
            for (current, original) in [
                (
                    abduction.current_from.and_then(|node| node.local_node()),
                    abduction.originally_from,
                ),
                (
                    abduction.current_to.and_then(|node| node.local_node()),
                    abduction.originally_to,
                ),
            ] {
                let Some(original) = original else {
                    continue;
                };
                let current_owner = current.and_then(|current| {
                    old_to_new
                        .iter()
                        .find(|(_, local)| **local == current)
                        .map(|(&owner, _)| owner)
                });
                // Production refs carry the owning NodeId even if their
                // TopLeft is nil. Stable IDs are only a compatibility fallback
                // for explicit synthetic projections without owning lineage.
                let source_node = original.source_node.or_else(|| {
                    self.graph
                        .nodes
                        .iter()
                        .find(|node| node.tala_id == original.tala_id)
                        .map(|node| node.input_id)
                });
                let Some(source_node) = source_node else {
                    continue;
                };
                let restored = self.graph.active_aggregate_owner(source_node);
                if Some(restored) != current_owner {
                    restored_endpoint_order.push((restored, original_edge));
                }
            }
        }
        let mut sized_obstacles = scope
            .and_then(|root_id| {
                let root = &self.graph.nodes[root_id.0 as usize];
                self.graph.containers.get(&root.container)
            })
            .into_iter()
            .flatten()
            .filter_map(|sibling| {
                let sibling = &self.graph.nodes[sibling.0 as usize];
                Some(Rect {
                    origin: sibling.fixed_top_left?,
                    size: sibling.rect.size,
                })
            })
            .collect::<Vec<_>>();
        if let Some(root_id) = scope
            && let Some(root_fixed_top_left) = self.graph.nodes[root_id.0 as usize].fixed_top_left
            && sized_obstacles.len() > 1
        {
            // placeNodes translates inherited obstacles into a fixed root's
            // local coordinates only when there are multiple boxes.
            let dx = -root_fixed_top_left.x - CONTAINER_PADDING;
            let dy = -root_fixed_top_left.y - CONTAINER_PADDING;
            if dx != 0.0 || dy != 0.0 {
                for obstacle in &mut sized_obstacles {
                    obstacle.origin.x += dx;
                    obstacle.origin.y += dy;
                }
            }
        }
        let source_orientation = if let Some(root_id) = scope {
            let root = &self.graph.nodes[root_id.0 as usize];
            let (root_adjacency_edge_count, root_edges) =
                self.graph.source_orientation_boundary_edge_context(root_id);
            let source_graph_node_tala_ids = source_tala_ids
                .iter()
                .copied()
                .filter(|tala_id| {
                    if *tala_id == root.tala_id
                        || self
                            .graph
                            .source_is_descendant_of_tala(*tala_id, root.tala_id)
                    {
                        return *tala_id != root.tala_id;
                    }
                    // A live cluster vessel is a Go Graph.Nodes entry at the
                    // cluster members' owning scope. The stable source-parent
                    // map belongs to member nodes, so inherit that scope for
                    // the vessel when reconstructing CombineSubgraphs' count.
                    self.graph
                        .clusters
                        .iter()
                        .filter(|cluster| {
                            self.graph.cluster_is_active(cluster)
                                && cluster.vessel_tala_id == *tala_id
                        })
                        .any(|cluster| {
                            cluster.members.iter().any(|member| {
                                let member_tala_id = self.graph.nodes[member.0 as usize].tala_id;
                                self.graph
                                    .source_parent_tala_by_tala
                                    .get(&member_tala_id)
                                    .copied()
                                    .flatten()
                                    .is_some_and(|parent| {
                                        self.graph
                                            .source_is_descendant_of_tala(parent, root.tala_id)
                                    })
                            })
                        })
                })
                .collect::<Vec<_>>();
            #[cfg(feature = "diagnostic-traces")]
            if crate::internal::trace_env_enabled("D2RS_TRACE_SOURCE_ORIENTATION_CONTEXT") {
                let incident_edges = self
                    .graph
                    .edge_order
                    .read().iter()
                    .filter_map(|edge_id| {
                        let edge = self.graph.edge_records.get(edge_id.0 as usize)?;
                        let from_owner = self.graph.active_aggregate_owner(edge.from);
                        let to_owner = self.graph.active_aggregate_owner(edge.to);
                        (from_owner == root_id || to_owner == root_id).then(|| {
                            serde_json::json!({
                                "edge_tala_id": edge.tala_id,
                                "edge_id": edge_id.0,
                                "from": self.graph.nodes[edge.from.0 as usize].tala_id,
                                "to": self.graph.nodes[edge.to.0 as usize].tala_id,
                                "from_owner": self.graph.nodes[from_owner.0 as usize].tala_id,
                                "to_owner": self.graph.nodes[to_owner.0 as usize].tala_id,
                                "source_endpoints": self.graph.source_edge_endpoints_by_id.get(&edge.tala_id),
                            })
                        })
                    })
                    .collect::<Vec<_>>();
                eprintln!(
                    "D2_RS_SOURCE_ORIENTATION_CONTEXT {}",
                    serde_json::json!({
                        "root": root.external_id,
                        "root_tala_id": root.tala_id,
                        "local_edge_ids": root.edges.read().iter().map(|edge| edge.0).collect::<Vec<_>>(),
                        "local_edge_tala_ids": root.edges.read().iter().filter_map(|edge| self.graph.edge_records.get(edge.0 as usize).map(|edge| edge.tala_id)).collect::<Vec<_>>(),
                        "external_edge_count": self.graph.external_edge_counts.get(root_id.0 as usize),
                        "context_edges": root_edges,
                        "active_owner_incident_edges": incident_edges,
                        "source_graph_node_tala_ids": source_graph_node_tala_ids,
                        "source_edges_with_root_tala_id": self.graph.source_edge_endpoints_by_id.iter().filter(|(_, endpoints)| endpoints.0 == root.tala_id || endpoints.1 == root.tala_id).map(|(edge, endpoints)| (*edge, *endpoints)).collect::<Vec<_>>(),
                    })
                );
            }
            let has_ancestor_obstacles = !sized_obstacles.is_empty();
            let graph_node_count = source_graph_node_tala_ids.len();
            // Go's `Graph.Direction` is an explicit per-graph map lookup. The
            // Rust arena also stores an effective default direction at the
            // top-level key, so reading `directions[root.container]` here can
            // invent a parent direction that the source graph does not have.
            let parent_direction = match root.container {
                Some(parent) => self.input.node(parent).and_then(|node| node.direction),
                None => self.input.explicit_direction,
            };
            SourceOrientationContext {
                root: Some(root.clone()),
                graph_node_count,
                parent_direction,
                root_direction: self.graph.directions.get(&Some(root_id)).copied(),
                root_adjacency_edge_count,
                root_edges,
                disabled_for_loop: self.source_orientation_disabled_for_loop,
                has_ancestor_obstacles,
                root_padding: self.graph.shape_fit_padding(root_id),
            }
        } else {
            SourceOrientationContext {
                graph_node_count: source_tala_ids.len(),
                disabled_for_loop: self.source_orientation_disabled_for_loop,
                ..SourceOrientationContext::default()
            }
        };
        Some(PlacementScope {
            owner_source: None,
            edge_old_to_new: projected_edge_old_to_new,
            graph,
            sized_obstacles,
            source_orientation,
            #[cfg(feature = "diagnostic-traces")]
            trace_owner_graph: self.graph.clone(),
            #[cfg(feature = "diagnostic-traces")]
            trace_children_graph_node_order: scope_children
                .iter()
                .map(|node| self.graph.nodes[node.0 as usize].tala_id)
                .collect(),
            // Keep both counts: the temporary children graph can contain one
            // direct child while the owning graph still follows the normal
            // optimizer path.
            owner_node_count: scope_children.len(),
            // Go's singleton fast path uses the owning Graph.Nodes slice,
            // which still contains tree-restored direct children even when
            // the temporary childrenGraph has one visible carrier. The
            // transaction child projection is the stable equivalent for a
            // nested scope; using only the induced arena length would wrongly
            // suppress that scope's placement lifecycle and trace events.
            owning_graph_node_count,
            root_external_id: scope
                .map(|node| self.graph.nodes[node.0 as usize].external_id.clone())
                .unwrap_or_default(),
            scoring_directions_by_tala: self.graph.scoring_directions_by_tala.clone(),
            edge_table_column_counts,
            projected_edge_minimums,
            temporary_edge_order,
            old_to_new,
            tree_auxiliary_nodes,
            tree_routing_nodes,
            tree_edge_old_to_new,
            restored_endpoint_order,
            incident_edge_order,
            hubs,
            child_order,
            assigned_nears: assigned_nears.into_iter().collect(),
            common_uncle_groups,
            edge_abduction_nodes,
            sized_adjacent_overrides,
            sized_cluster_distance_boxes,
            sized_edge_abductions,
            sized_projected_obstructions,
            sized_projected_obstruction_sets,
            sized_collapsed_symmetry_neighbors,
            collapsed_restored_mirror_edges,
            scoring_node_metadata,
            projected_clusters,
            trace_container_children: self.graph.trace_container_children.clone(),
            transaction_external_containers,
            transaction_validation_external_containers,
            transaction_external_container_children,
            transaction_external_aggregate_children,
            transaction_external_cluster_layouts,
            projected_transaction_nodes,
        })
    }

    pub(super) fn mirrored_external_container_children(
        graph: &ArenaGraph,
        container: NodeId,
        children: &[ArenaNode],
        mirrors: (bool, bool),
    ) -> Vec<ArenaNode> {
        let mut mirrored = children.to_vec();
        for child in &mut mirrored {
            let Some(mut position) = child.position else {
                continue;
            };
            let [top, right, bottom, left] = loop_cardinal_extents(child.loop_offsets.as_ref());
            if mirrors.0 {
                position.x = -position.x - child.rect.size.width + left - right;
            }
            if mirrors.1 {
                position.y = -position.y - child.rect.size.height + top - bottom;
            }
            child.position = Some(position);
            child.rect.origin = position;
        }
        let Some((top_left, bottom_right)) = ArenaGraph::fixed_external_node_bounds(&mirrored)
        else {
            return mirrored;
        };
        let content = Size {
            width: bottom_right.x - top_left.x,
            height: bottom_right.y - top_left.y,
        };
        let padding = graph.shape_fit_padding(container);
        let Some(inside) = graph.shape_inside_placement_absolute(container, content, padding)
        else {
            return mirrored;
        };
        let delta = Point {
            x: inside.x - top_left.x,
            y: inside.y - top_left.y,
        };
        for child in &mut mirrored {
            let Some(mut position) = child.position else {
                continue;
            };
            position.x += delta.x;
            position.y += delta.y;
            child.position = Some(position);
            child.rect.origin = position;
        }
        mirrored
    }

    pub(super) fn external_children_have_hidden_aggregate(
        children: &[ArenaNode],
        directed_tala_ids: &BTreeSet<i64>,
        projected_vessel_tala_ids: &BTreeSet<i64>,
    ) -> bool {
        children.iter().any(|child| {
            !directed_tala_ids.contains(&child.tala_id)
                && projected_vessel_tala_ids.contains(&child.tala_id)
        })
    }

    pub(super) fn projected_mirror_refit_offsets(
        graph: &ArenaGraph,
        refitted_container_tala_ids: &BTreeSet<i64>,
    ) -> Vec<(i64, i64, Point)> {
        refitted_container_tala_ids
            .iter()
            .filter_map(|&container_tala_id| {
                let container_position =
                    graph.projected_node_position_by_tala(container_tala_id)?;
                let children = graph
                    .transaction_external_container_children
                    .get(&container_tala_id)?;
                Some(children.iter().filter_map(move |child| {
                    let child_position = child.position?;
                    Some((
                        container_tala_id,
                        child.tala_id,
                        Point {
                            x: child_position.x - container_position.x,
                            y: child_position.y - container_position.y,
                        },
                    ))
                }))
            })
            .flatten()
            .collect()
    }

    // Source Sequence.SyncGeometry/PlaceVessel keeps Containers' vessel and
    // its retained original steps in one shared frame. Numerical scope maps
    // store separate values, so reconcile those aliases before deriving offsets.
    pub(super) fn reconcile_projected_sequence_vessels(
        owner: &ArenaGraph,
        placed: &mut ArenaGraph,
    ) {
        let mut positions = Vec::new();
        for (index, sequence) in owner.sequences.iter().enumerate() {
            if owner
                .topology_vessel_node(TopologyVessel::Sequence(index))
                .is_none()
            {
                continue;
            }
            let Some(members) = placed
                .transaction_external_aggregate_children
                .get(&sequence.vessel_tala_id)
            else {
                continue;
            };
            if members.len() != sequence.members.len()
                || members
                    .iter()
                    .zip(&sequence.members)
                    .any(|(member, original)| {
                        member.tala_id != owner.nodes[original.0 as usize].tala_id
                    })
            {
                continue;
            }
            let Some(points) = members
                .iter()
                .map(|member| member.position)
                .collect::<Option<Vec<_>>>()
            else {
                continue;
            };
            if points.is_empty() {
                continue;
            }
            positions.push((
                sequence.vessel_tala_id,
                Point {
                    x: points
                        .iter()
                        .map(|point| point.x)
                        .fold(f64::INFINITY, f64::min),
                    y: points
                        .iter()
                        .map(|point| point.y)
                        .fold(f64::INFINITY, f64::min),
                },
            ));
        }
        for (tala_id, point) in positions {
            for node in placed
                .transaction_external_container_children
                .values_mut()
                .flatten()
                .chain(placed.transaction_external_containers.iter_mut())
            {
                if node.tala_id == tala_id {
                    node.position = Some(point);
                    node.rect.origin = point;
                }
            }
        }
    }

    // Source induced graphs share node pointers. Publishing a numerical copy
    // must update the retained TopLeft pointee, not just ArenaNode's inline
    // compatibility field (node.go:2136–2148).
    #[cfg_attr(feature = "diagnostic-traces", track_caller)]
    pub(super) fn publish_induced_owner_point(
        owner: &mut ArenaGraph,
        node: NodeId,
        value: Option<Point>,
    ) {
        #[cfg(feature = "diagnostic-traces")]
        if crate::internal::trace_env_value("D2RS_TRACE_PUBLISHED_OWNER_POINT").is_some_and(
            |target| {
                target == "all"
                    || target.split(',').any(|id| {
                        id.parse::<i64>().ok() == Some(owner.nodes[node.0 as usize].tala_id)
                    })
            },
        ) {
            eprintln!(
                "PUBLISHED_OWNER_POINT_RUST caller={} nodes={} node={} previous={:?} next={:?}",
                std::panic::Location::caller(),
                owner.nodes.len(),
                owner.nodes[node.0 as usize].tala_id,
                owner.position(node),
                value
            );
        }
        if let Some(value) = value {
            if let Ok(point) = owner.owner_node_point(node)
                && point.get().is_some()
            {
                owner
                    .write_owner_point(&point, value)
                    .expect("induced owner point publication");
            } else {
                owner.set_position(node, value);
            }
            owner.nodes[node.0 as usize].position = Some(value);
            owner.nodes[node.0 as usize].rect.origin = value;
        } else {
            owner.clear_position(node);
        }
    }

    #[cfg_attr(feature = "diagnostic-traces", track_caller)]
    pub(super) fn publish_induced_external_geometry(owner: &mut ArenaGraph, placed: &ArenaGraph) {
        let trace_aggregate_copyback =
            crate::internal::trace_env_value("WEFTAN_TRACE_AGG_COPYBACK_TARGET")
                .and_then(|target| target.parse::<i64>().ok());
        if let Some(target) = trace_aggregate_copyback
            && (placed
                .transaction_external_aggregate_children
                .values()
                .flatten()
                .any(|node| node.tala_id == target)
                || owner
                    .transaction_external_aggregate_children
                    .values()
                    .flatten()
                    .any(|node| node.tala_id == target))
        {
            let compact = |graph: &ArenaGraph| {
                let mut rows = Vec::new();
                for (vessel, children) in &graph.transaction_external_aggregate_children {
                    for child in children {
                        if child.tala_id == target {
                            rows.push((*vessel, child.position, child.rect.size));
                        }
                    }
                }
                rows
            };
            let nested_frame = |graph: &ArenaGraph| {
                (
                    graph
                        .nodes
                        .iter()
                        .find(|node| node.tala_id == 3677471320)
                        .map(|node| node.position),
                    graph
                        .transaction_external_containers
                        .iter()
                        .find(|node| node.tala_id == 3677471320)
                        .map(|node| node.position),
                    graph
                        .transaction_external_container_children
                        .get(&3677471320)
                        .map(|children| {
                            children
                                .iter()
                                .map(|node| (node.tala_id, node.position))
                                .collect::<Vec<_>>()
                        }),
                )
            };
            let member_state = |graph: &ArenaGraph| {
                graph
                    .nodes
                    .iter()
                    .find(|node| node.tala_id == target)
                    .map(|node| {
                        (
                            node.rect.size,
                            node.cluster,
                            graph.active_cluster_index(node.input_id),
                            graph.active_aggregate_owner(node.input_id),
                            graph.active_node_is_aggregate(node.input_id),
                        )
                    })
            };
            eprintln!(
                "AGG_COPYBACK_RUST before caller={} target={} owner_nodes={:?} owner_member_state={:?} owner_agg={:?} owner_agg_keys={:?} owner_children={:?} owner_nested_frame={:?} placed_nodes={:?} placed_member_state={:?} placed_agg={:?} placed_agg_keys={:?} placed_children={:?} placed_nested_frame={:?}",
                std::panic::Location::caller(),
                target,
                owner
                    .nodes
                    .iter()
                    .find(|node| node.tala_id == target)
                    .map(|node| node.position),
                member_state(owner),
                compact(owner),
                owner
                    .transaction_external_aggregate_children
                    .iter()
                    .map(|(v, c)| (*v, c.iter().map(|n| n.tala_id).collect::<Vec<_>>()))
                    .collect::<Vec<_>>(),
                owner
                    .transaction_external_container_children
                    .iter()
                    .filter_map(|(parent, children)| children
                        .iter()
                        .find(|node| node.tala_id == target)
                        .map(|child| (*parent, child.position)))
                    .collect::<Vec<_>>(),
                nested_frame(owner),
                placed
                    .nodes
                    .iter()
                    .find(|node| node.tala_id == target)
                    .map(|node| node.position),
                member_state(placed),
                compact(placed),
                placed
                    .transaction_external_aggregate_children
                    .iter()
                    .map(|(v, c)| (*v, c.iter().map(|n| n.tala_id).collect::<Vec<_>>()))
                    .collect::<Vec<_>>(),
                placed
                    .transaction_external_container_children
                    .iter()
                    .filter_map(|(parent, children)| children
                        .iter()
                        .find(|node| node.tala_id == target)
                        .map(|child| (*parent, child.position)))
                    .collect::<Vec<_>>(),
                nested_frame(placed),
            );
        }
        let trace_target_copyback = crate::internal::trace_env_value(
            "WEFTAN_TRACE_COPYBACK_TARGET",
        )
        .is_some_and(|target| target == "all" || target.parse::<i64>().ok() == Some(3471367806));
        if trace_target_copyback {
            let compact = |graph: &ArenaGraph| {
                graph
                    .transaction_external_container_children
                    .get(&4254926550)
                    .map(|children| {
                        children
                            .iter()
                            .map(|child| (child.tala_id, child.position))
                            .collect::<Vec<_>>()
                    })
            };
            eprintln!(
                "COPYBACK_TARGET_RUST before owner_nodes={:?} owner_map={:?} placed_nodes={:?} placed_map={:?}",
                owner
                    .nodes
                    .iter()
                    .map(|node| node.tala_id)
                    .collect::<Vec<_>>(),
                compact(owner),
                placed
                    .nodes
                    .iter()
                    .map(|node| node.tala_id)
                    .collect::<Vec<_>>(),
                compact(placed),
            );
        }
        owner.trace_target_shared_geometry("copyback-before");
        placed.trace_target_shared_geometry("copyback-placed");
        let trace_copyback = crate::internal::trace_env_enabled("WEFTAN_TRACE_COPYBACK")
            && (owner.nodes.iter().any(|node| node.tala_id == 3248328362)
                || placed.nodes.iter().any(|node| node.tala_id == 3248328362)
                || owner
                    .transaction_external_container_children
                    .contains_key(&3248328362)
                || placed
                    .transaction_external_container_children
                    .contains_key(&3248328362));
        if trace_copyback {
            let compact = |graph: &ArenaGraph| {
                graph
                    .transaction_external_container_children
                    .iter()
                    .filter(|(parent, children)| {
                        **parent == 3248328362
                            || children.iter().any(|child| {
                                matches!(child.tala_id, 3248328362 | 3680227035 | 1832207848)
                            })
                    })
                    .map(|(parent, children)| {
                        (
                            *parent,
                            children
                                .iter()
                                .filter(|child| {
                                    matches!(child.tala_id, 3248328362 | 3680227035 | 1832207848)
                                })
                                .map(|child| (child.tala_id, child.position, child.rect.size))
                                .collect::<Vec<_>>(),
                        )
                    })
                    .collect::<Vec<_>>()
            };
            eprintln!(
                "COPYBACK_RUST before owner_nodes={:?} owner_children={:?} placed_nodes={:?} placed_children={:?}",
                owner
                    .nodes
                    .iter()
                    .filter(|node| matches!(node.tala_id, 3248328362 | 3680227035 | 1832207848))
                    .map(|node| (node.tala_id, node.position, node.rect.size))
                    .collect::<Vec<_>>(),
                compact(owner),
                placed
                    .nodes
                    .iter()
                    .filter(|node| matches!(node.tala_id, 3248328362 | 3680227035 | 1832207848))
                    .map(|node| (node.tala_id, node.position, node.rect.size))
                    .collect::<Vec<_>>(),
                compact(placed),
            );
        }
        if crate::internal::trace_env_enabled("WEFTAN_TRACE_HIDDEN_EXTERNAL") {
            let tracked = [
                1832207848_i64,
                2280770807,
                298920306,
                3777840222,
                4222097421,
                1379919604,
            ];
            let placed_values = placed
                .nodes
                .iter()
                .filter(|node| tracked.contains(&node.tala_id))
                .map(|node| (node.tala_id, node.position))
                .collect::<Vec<_>>();
            if !placed_values.is_empty() {
                let placed_aliases = placed
                    .transaction_external_container_children
                    .values()
                    .flatten()
                    .filter(|node| tracked.contains(&node.tala_id))
                    .map(|node| (node.tala_id, node.position))
                    .collect::<Vec<_>>();
                eprintln!(
                    "PUBLISH_INDUCED_RUST placed={:?} aliases={:?}",
                    placed_values, placed_aliases
                );
            }
        }
        if crate::internal::trace_env_enabled("WEFTAN_TRACE_INDUCED_GEOMETRY") {
            eprintln!(
                "INDUCED_GEOMETRY_RUST placed_nodes={:?} placed_external={:?} placed_children={:?} placed_aggregate={:?}",
                placed
                    .nodes
                    .iter()
                    .filter(|node| matches!(
                        node.tala_id,
                        3447521236 | 5577006791947779410 | 1445424226 | 2895098185
                    ))
                    .map(|node| (node.tala_id, node.position, node.rect.size))
                    .collect::<Vec<_>>(),
                placed
                    .transaction_external_containers
                    .iter()
                    .filter(|node| matches!(
                        node.tala_id,
                        3447521236 | 5577006791947779410 | 1445424226 | 2895098185
                    ))
                    .map(|node| (node.tala_id, node.position, node.rect.size))
                    .collect::<Vec<_>>(),
                placed
                    .transaction_external_container_children
                    .iter()
                    .flat_map(
                        |(parent, children)| children.iter().filter_map(move |child| {
                            matches!(
                                child.tala_id,
                                3447521236 | 5577006791947779410 | 1445424226 | 2895098185
                            )
                            .then_some((
                                *parent,
                                child.tala_id,
                                child.position,
                                child.rect.size,
                            ))
                        })
                    )
                    .collect::<Vec<_>>(),
                placed
                    .transaction_external_aggregate_children
                    .iter()
                    .flat_map(
                        |(parent, children)| children.iter().filter_map(move |child| {
                            matches!(
                                child.tala_id,
                                3447521236 | 5577006791947779410 | 1445424226 | 2895098185
                            )
                            .then_some((
                                *parent,
                                child.tala_id,
                                child.position,
                                child.rect.size,
                            ))
                        })
                    )
                    .collect::<Vec<_>>()
            );
        }
        // A hidden sequence is represented by its ordered original step
        // pointers in the numerical scope. Source PlaceVessel reads their
        // positioned bounds; publish that same real vessel before the parent
        // takes a Containers snapshot (sequence.go:105–115).
        for (&tala_id, members) in &placed.transaction_external_aggregate_children {
            let Some((index, sequence)) = owner
                .sequences
                .iter()
                .enumerate()
                .find(|(_, sequence)| sequence.vessel_tala_id == tala_id)
            else {
                continue;
            };
            if members.len() != sequence.members.len()
                || members
                    .iter()
                    .zip(&sequence.members)
                    .any(|(projected, original)| {
                        projected.tala_id != owner.nodes[original.0 as usize].tala_id
                    })
            {
                continue;
            }
            let Some(vessel) = owner.topology_vessel_node(TopologyVessel::Sequence(index)) else {
                continue;
            };
            let positions = members
                .iter()
                .map(|node| node.position)
                .collect::<Option<Vec<_>>>();
            if let Some(positions) = positions
                && !positions.is_empty()
            {
                let position = Point {
                    x: positions
                        .iter()
                        .map(|point| point.x)
                        .fold(f64::INFINITY, f64::min),
                    y: positions
                        .iter()
                        .map(|point| point.y)
                        .fold(f64::INFINITY, f64::min),
                };
                Self::publish_induced_owner_point(owner, vessel, Some(position));
            }
        }

        // Visible induced vessels are shared pointers too. Their compact
        // carrier has the vessel identity, while the original owner retains
        // that actual allocation outside the member's numerical carrier.
        let vessels = owner
            .clusters
            .iter()
            .enumerate()
            .filter_map(|(index, _)| owner.topology_vessel_node(TopologyVessel::Cluster(index)))
            .chain(owner.sequences.iter().enumerate().filter_map(|(index, _)| {
                owner.topology_vessel_node(TopologyVessel::Sequence(index))
            }))
            .collect::<Vec<_>>();
        for vessel in vessels {
            let tala_id = owner.nodes[vessel.0 as usize].tala_id;
            if let Some(local) = placed
                .nodes
                .iter()
                .find(|node| node.tala_id == tala_id && placed.node_order_contains(node.input_id))
            {
                Self::publish_induced_owner_point(owner, vessel, placed.position(local.input_id));
                owner.nodes[vessel.0 as usize].rect.size = local.rect.size;
            }
        }

        // TALA's induced graph shares the owning graph's node pointers.
        // Publish container boxes refitted through its hidden Containers map
        // before copying the component's visible Graph.Nodes back.
        for external in &placed.transaction_external_containers {
            let stable_carrier_member_size = owner
                .nodes
                .iter()
                .find(|node| node.tala_id == external.tala_id)
                .and_then(|node| {
                    let id = node.input_id;
                    (owner.active_node_is_aggregate(id) && owner.active_aggregate_owner(id) == id)
                        .then_some(node.rect.size)
                });
            let published_size = stable_carrier_member_size.unwrap_or(external.rect.size);
            let Some(owner_index) = owner
                .nodes
                .iter()
                .position(|node| node.tala_id == external.tala_id)
            else {
                if let Some(owner_external) = owner
                    .transaction_external_containers
                    .iter_mut()
                    .find(|node| node.tala_id == external.tala_id)
                {
                    // Graph.Containers and Graph.Nodes share the same Go
                    // *Node. A hidden container can be present only in this
                    // external carrier for the current induced scope; publish
                    // its refitted position as well as its size so the next
                    // parent-scope copyback uses the same frame as the shared
                    // pointer.
                    owner_external.position = external.position;
                    owner_external.rect.origin = external.rect.origin;
                    owner_external.rect.size = external.rect.size;
                }
                continue;
            };
            Self::publish_induced_owner_point(owner, NodeId(owner_index as u32), external.position);
            owner.nodes[owner_index].rect.origin = external.rect.origin;
            owner.nodes[owner_index].rect.size = published_size;
        }
        for (&container_tala_id, placed_children) in &placed.transaction_external_container_children
        {
            if !owner
                .transaction_external_container_children
                .contains_key(&container_tala_id)
            {
                continue;
            }
            for placed_child in placed_children {
                let stable_carrier_member_size = owner
                    .nodes
                    .iter()
                    .find(|node| node.tala_id == placed_child.tala_id)
                    .and_then(|node| {
                        let id = node.input_id;
                        (owner.active_node_is_aggregate(id)
                            && owner.active_aggregate_owner(id) == id)
                            .then_some(node.rect.size)
                    });
                let published_size = stable_carrier_member_size.unwrap_or(placed_child.rect.size);
                let Some(owner_child_index) = owner
                    .transaction_external_container_children
                    .get(&container_tala_id)
                    .and_then(|children| {
                        children
                            .iter()
                            .position(|child| child.tala_id == placed_child.tala_id)
                    })
                else {
                    continue;
                };
                let owner_child = &mut owner
                    .transaction_external_container_children
                    .get_mut(&container_tala_id)
                    .expect("owner child map disappeared")[owner_child_index];
                owner_child.position = placed_child.position;
                owner_child.rect.origin = placed_child.rect.origin;
                owner_child.rect.size = published_size;
                // The recovered graph stores the same *Node pointer in
                // Graph.Nodes and Graph.Containers. Our induced scope carries
                // container children as value snapshots, so publish positioned
                // snapshots back to any stable node with the same durable TALA
                // identity as well. Do not erase an existing owner position
                // when the projected child is still unpositioned.
                if let Some(position) = placed_child.position
                    && let Some(owner_index) = owner
                        .nodes
                        .iter()
                        .position(|node| node.tala_id == placed_child.tala_id)
                {
                    Self::publish_induced_owner_point(
                        owner,
                        NodeId(owner_index as u32),
                        Some(position),
                    );
                    let owner_node = &mut owner.nodes[owner_index];
                    owner_node.rect.origin = placed_child.rect.origin;
                    owner_node.rect.size = published_size;
                }
                if placed_child.is_container
                    && let Some(owner_external) = owner
                        .transaction_external_containers
                        .iter_mut()
                        .find(|candidate| candidate.tala_id == placed_child.tala_id)
                {
                    // This child alias can be stale relative to a refit that
                    // survived a rejected transpose. Preserve the authoritative
                    // external carrier's position while publishing its size.
                    owner_external.rect.size = published_size;
                }
            }
        }
        // Cluster and sequence members are kept in a separate projection in
        // the Rust arena.  They are still the same shared Go pointers, so a
        // direct mirror must publish their positions to every owner view
        // before component packing translates the enclosing container.
        for placed_child in placed
            .transaction_external_aggregate_children
            .values()
            .flatten()
        {
            let stable_carrier_member_size = owner
                .nodes
                .iter()
                .find(|node| node.tala_id == placed_child.tala_id)
                .and_then(|node| {
                    let id = node.input_id;
                    (owner.active_node_is_aggregate(id) && owner.active_aggregate_owner(id) == id)
                        .then_some(node.rect.size)
                });
            let published_size = stable_carrier_member_size.unwrap_or(placed_child.rect.size);
            let owner_ids = owner
                .nodes
                .iter()
                .enumerate()
                .filter_map(|(index, node)| {
                    (node.tala_id == placed_child.tala_id).then_some(NodeId(index as u32))
                })
                .collect::<Vec<_>>();
            for node in owner_ids {
                Self::publish_induced_owner_point(owner, node, placed_child.position);
            }
            let update = |node: &mut ArenaNode| {
                if node.tala_id == placed_child.tala_id {
                    node.position = placed_child.position;
                    node.rect.origin = placed_child.rect.origin;
                    node.rect.size = published_size;
                }
            };
            for node in &mut owner.nodes {
                update(node);
            }
            for node in &mut owner.transaction_external_containers {
                update(node);
            }
            for children in owner.transaction_external_container_children.values_mut() {
                for node in children {
                    update(node);
                }
            }
            for children in owner.transaction_external_aggregate_children.values_mut() {
                for node in children {
                    update(node);
                }
            }
        }
        if trace_copyback {
            eprintln!(
                "COPYBACK_RUST after owner_nodes={:?} owner_children={:?}",
                owner
                    .nodes
                    .iter()
                    .filter(|node| matches!(node.tala_id, 3248328362 | 3680227035 | 1832207848))
                    .map(|node| (node.tala_id, node.position, node.rect.size))
                    .collect::<Vec<_>>(),
                owner
                    .transaction_external_container_children
                    .iter()
                    .filter(|(parent, children)| {
                        **parent == 3248328362
                            || children.iter().any(|child| {
                                matches!(child.tala_id, 3248328362 | 3680227035 | 1832207848)
                            })
                    })
                    .map(|(parent, children)| {
                        (
                            *parent,
                            children
                                .iter()
                                .filter(|child| {
                                    matches!(child.tala_id, 3248328362 | 3680227035 | 1832207848)
                                })
                                .map(|child| (child.tala_id, child.position, child.rect.size))
                                .collect::<Vec<_>>(),
                        )
                    })
                    .collect::<Vec<_>>(),
            );
        }
        owner.trace_target_shared_geometry("copyback-after");
        if let Some(target) = trace_aggregate_copyback
            && (owner
                .transaction_external_aggregate_children
                .values()
                .flatten()
                .any(|node| node.tala_id == target)
                || owner.nodes.iter().any(|node| node.tala_id == target))
        {
            let mut aggregate = Vec::new();
            for (vessel, children) in &owner.transaction_external_aggregate_children {
                for child in children {
                    if child.tala_id == target {
                        aggregate.push((*vessel, child.position));
                    }
                }
            }
            eprintln!(
                "AGG_COPYBACK_RUST after caller={} target={} owner_node={:?} owner_agg={:?} owner_children={:?}",
                std::panic::Location::caller(),
                target,
                owner
                    .nodes
                    .iter()
                    .find(|node| node.tala_id == target)
                    .map(|node| node.position),
                aggregate,
                owner
                    .transaction_external_container_children
                    .iter()
                    .filter_map(|(parent, children)| children
                        .iter()
                        .find(|node| node.tala_id == target)
                        .map(|child| (*parent, child.position)))
                    .collect::<Vec<_>>(),
            );
        }
        if trace_target_copyback {
            let compact = |graph: &ArenaGraph| {
                graph
                    .transaction_external_container_children
                    .get(&4254926550)
                    .map(|children| {
                        children
                            .iter()
                            .map(|child| (child.tala_id, child.position))
                            .collect::<Vec<_>>()
                    })
            };
            eprintln!(
                "COPYBACK_TARGET_RUST after owner_nodes={:?} owner_map={:?}",
                owner
                    .nodes
                    .iter()
                    .map(|node| node.tala_id)
                    .collect::<Vec<_>>(),
                compact(owner),
            );
        }
    }

    /// Publish the current shared-pointer geometry into an owner clone used by
    /// placement trace snapshots. The source hierarchy clone does not always
    /// retain `transaction_external_container_children` entries for every
    /// descendant, so the runtime copyback helper alone can leave owner nodes
    /// stale even when the induced scope has the correct projected geometry.
    #[cfg(feature = "diagnostic-traces")]
    pub(super) fn publish_diagnostic_shared_geometry(owner: &mut ArenaGraph, placed: &ArenaGraph) {
        let mut previous_owner_positions = BTreeMap::<i64, (Point, Point)>::new();
        for node in owner
            .nodes
            .iter()
            .chain(owner.transaction_external_containers.iter())
            .chain(
                owner
                    .transaction_external_container_children
                    .values()
                    .flatten(),
            )
            .chain(
                owner
                    .transaction_external_aggregate_children
                    .values()
                    .flatten(),
            )
            .chain(owner.trace_container_children.values().flatten())
        {
            if let Some(position) = node.position {
                previous_owner_positions
                    .entry(node.tala_id)
                    .or_insert((position, node.rect.origin));
            }
        }
        Self::publish_induced_external_geometry(owner, placed);

        let external_cluster_layouts = placed.transaction_external_cluster_layouts.clone();
        owner.transaction_external_cluster_layouts.extend(
            external_cluster_layouts
                .iter()
                .map(|(id, layout)| (*id, *layout)),
        );
        for (owner_index, owner_cluster) in owner.clusters.iter_mut().enumerate() {
            let vessel_tala_id = owner_cluster.vessel_tala_id;
            if let Some(layout) = external_cluster_layouts.get(&vessel_tala_id) {
                owner_cluster.arrangement = layout.arrangement;
                owner_cluster.padding = layout.padding;
                owner_cluster.fixed_size = layout.fixed_size;
            }
            let placed_index = placed
                .clusters
                .iter()
                .position(|cluster| cluster.vessel_tala_id == vessel_tala_id);
            if let Some(index) = placed_index {
                let placed_cluster = &placed.clusters[index];
                owner_cluster.arrangement = placed_cluster.arrangement;
                owner_cluster.desired_arrangement = placed_cluster.desired_arrangement;
                owner_cluster.padding = placed_cluster.padding;
                owner_cluster.fixed_size = placed_cluster.fixed_size;
            }
            // The aggregate alias follows the live shared Go vessel pointer
            // through component packing. Its position can advance before the
            // Rust-side pending vessel cache is refreshed, so prefer the
            // currently projected node and use the cache as a fallback.
            // A recursive placement scope represents the owning cluster as
            // one ordinary active Graph.Nodes entry, not as a ClusterState.
            // Go retains that temporary vessel pointer in the owning graph;
            // recover its live position by stable ID before consulting the
            // scope's external aggregate aliases.
            let scope_vessel = placed.projected_node_by_tala(vessel_tala_id);
            let projected_position = scope_vessel
                .as_ref()
                .and_then(|node| node.position)
                .or_else(|| placed.projected_node_position_by_tala(vessel_tala_id));
            let pending_position = placed_index
                .and_then(|index| placed.pending_cluster_vessel_positions.get(&index).copied());
            let position = projected_position.or(pending_position).or_else(|| {
                placed_index
                    .and_then(|index| placed.clusters[index].members.first().copied())
                    .and_then(|carrier| placed.active_node_position(carrier))
            });
            if let Some(position) = position {
                owner
                    .pending_cluster_vessel_positions
                    .insert(owner_index, position);
            } else if scope_vessel.is_some() {
                // The source graph retains a shared vessel pointer while it
                // places an unrelated sibling component. Absence from this
                // induced scope means no geometry update occurred; only clear
                // the cached origin when the vessel is present here but
                // explicitly unpositioned.
                owner.pending_cluster_vessel_positions.remove(&owner_index);
            }
        }

        let mut geometry_by_tala_id = BTreeMap::<i64, ArenaNode>::new();
        for node in &placed.nodes {
            geometry_by_tala_id.insert(node.tala_id, node.clone());
        }
        for node in placed
            .transaction_external_containers
            .iter()
            .chain(
                placed
                    .transaction_external_container_children
                    .values()
                    .flatten(),
            )
            .chain(
                placed
                    .transaction_external_aggregate_children
                    .values()
                    .flatten(),
            )
        {
            match geometry_by_tala_id.get_mut(&node.tala_id) {
                Some(current) if current.position.is_none() && node.position.is_some() => {
                    // Prefer the positioned shared-pointer alias when an
                    // active arena node has not yet received coordinates.
                    // Keep the active node's size, which belongs to this
                    // optimizer scope.
                    current.position = node.position;
                    current.rect.origin = node.rect.origin;
                }
                Some(_) => {}
                None => {
                    geometry_by_tala_id.insert(node.tala_id, node.clone());
                }
            }
        }

        let publish = |node: &mut ArenaNode| {
            if let Some(geometry) = geometry_by_tala_id.get(&node.tala_id) {
                if geometry.position.is_none()
                    && let Some((position, origin)) =
                        previous_owner_positions.get(&node.tala_id).copied()
                {
                    // An unrelated component can carry an unpositioned
                    // transaction alias for a sibling whose shared owner
                    // pointer already has coordinates. Preserve that owner
                    // view rather than erasing it with the stale alias.
                    node.position = Some(position);
                    node.rect.origin = origin;
                } else {
                    node.position = geometry.position;
                    node.rect.origin = geometry.rect.origin;
                }
                node.rect.size = geometry.rect.size;
            }
        };
        for node in &mut owner.nodes {
            publish(node);
        }
        for node in &mut owner.transaction_external_containers {
            publish(node);
        }
        for children in owner.transaction_external_container_children.values_mut() {
            for node in children {
                publish(node);
            }
        }
        for children in owner.transaction_external_aggregate_children.values_mut() {
            for node in children {
                publish(node);
            }
        }
        for children in owner.trace_container_children.values_mut() {
            for node in children {
                publish(node);
            }
        }
    }

    /// Publish only geometry that changed during one upstream callback. The
    /// Go engine mutates shared `*Node` values in place, while these Rust
    /// induced graphs carry value snapshots for external owners. An unchanged
    /// external snapshot may therefore be older than the owner state already
    /// published by an earlier component; copying it back would rewind the
    /// diagnostic owner even though the layout operation did not touch it.
    #[cfg(feature = "diagnostic-traces")]
    #[track_caller]
    pub(super) fn publish_diagnostic_shared_geometry_since(
        owner: &mut ArenaGraph,
        placed: &ArenaGraph,
        before: &ArenaGraph,
    ) -> BTreeSet<i64> {
        Self::publish_diagnostic_geometry_since(owner, placed, before, &BTreeSet::new())
    }

    /// InitializeNodes publishes every active component's shared Node pointer,
    /// including fixed nodes eagerly positioned when the arena was induced.
    /// External aliases still publish only their actual callback deltas.
    #[cfg(feature = "diagnostic-traces")]
    #[track_caller]
    pub(super) fn publish_diagnostic_initialized_geometry(
        owner: &mut ArenaGraph,
        placed: &ArenaGraph,
        before: &ArenaGraph,
    ) -> BTreeSet<i64> {
        let initialized_members = placed
            .graph_node_order()
            .into_iter()
            .map(|node| placed.nodes[node.0 as usize].tala_id)
            .collect();
        Self::publish_diagnostic_geometry_since(owner, placed, before, &initialized_members)
    }

    #[cfg(feature = "diagnostic-traces")]
    #[track_caller]
    fn publish_diagnostic_geometry_since(
        owner: &mut ArenaGraph,
        placed: &ArenaGraph,
        before: &ArenaGraph,
        initialized_members: &BTreeSet<i64>,
    ) -> BTreeSet<i64> {
        type GeometryBits = (Option<(u64, u64)>, (u64, u64), (u64, u64));

        fn geometries(graph: &ArenaGraph) -> BTreeMap<i64, Vec<GeometryBits>> {
            fn push(node: &ArenaNode, by_tala: &mut BTreeMap<i64, Vec<GeometryBits>>) {
                let point_bits = |point: Point| (point.x.to_bits(), point.y.to_bits());
                by_tala.entry(node.tala_id).or_default().push((
                    node.position.map(point_bits),
                    point_bits(node.rect.origin),
                    (
                        node.rect.size.width.to_bits(),
                        node.rect.size.height.to_bits(),
                    ),
                ));
            }

            let mut by_tala = BTreeMap::new();
            for node in &graph.nodes {
                push(node, &mut by_tala);
            }
            for node in &graph.transaction_external_containers {
                push(node, &mut by_tala);
            }
            for children in graph.transaction_external_container_children.values() {
                for node in children {
                    push(node, &mut by_tala);
                }
            }
            for children in graph.transaction_external_aggregate_children.values() {
                for node in children {
                    push(node, &mut by_tala);
                }
            }
            by_tala
        }

        fn restore_unchanged(
            current: &mut [ArenaNode],
            previous: &[ArenaNode],
            changed: &BTreeSet<i64>,
        ) {
            for (node, old) in current.iter_mut().zip(previous) {
                if node.tala_id == old.tala_id && !changed.contains(&node.tala_id) {
                    node.position = old.position;
                    node.rect.origin = old.rect.origin;
                    node.rect.size = old.rect.size;
                }
            }
        }

        fn restore_unchanged_map(
            current: &mut BTreeMap<i64, Vec<ArenaNode>>,
            previous: &BTreeMap<i64, Vec<ArenaNode>>,
            changed: &BTreeSet<i64>,
        ) {
            for (key, nodes) in current {
                if let Some(old) = previous.get(key) {
                    restore_unchanged(nodes, old, changed);
                }
            }
        }

        let before_geometry = geometries(before);
        let placed_geometry = geometries(placed);
        let changed = before_geometry
            .keys()
            .chain(placed_geometry.keys())
            .copied()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .filter(|tala_id| {
                initialized_members.contains(tala_id)
                    || before_geometry.get(tala_id) != placed_geometry.get(tala_id)
            })
            .collect::<BTreeSet<_>>();
        #[cfg(feature = "diagnostic-traces")]
        let trace_target = crate::internal::trace_env_value("D2RS_TRACE_OWNER_FRAME_TARGET")
            .and_then(|value| value.split_once(':').map(|(_, id)| id.to_owned()))
            .and_then(|id| id.parse::<i64>().ok());
        #[cfg(feature = "diagnostic-traces")]
        let trace_position = |graph: &ArenaGraph| {
            graph
                .nodes
                .iter()
                .find(|node| Some(node.tala_id) == trace_target)
                .and_then(|node| {
                    node.position
                        .map(|point| (point.x.to_bits(), point.y.to_bits()))
                })
        };
        #[cfg(feature = "diagnostic-traces")]
        let trace_aliases = |graph: &ArenaGraph| {
            let mut aliases = Vec::new();
            let mut record = |source: String, node: &ArenaNode| {
                if Some(node.tala_id) == trace_target {
                    aliases.push(format!(
                        "{source}={:?}",
                        (node.position, node.rect.origin, node.rect.size)
                    ));
                }
            };
            for (index, node) in graph.nodes.iter().enumerate() {
                record(format!("nodes[{index}]"), node);
            }
            for (index, node) in graph.transaction_external_containers.iter().enumerate() {
                record(format!("transaction_external_containers[{index}]"), node);
            }
            for (parent, nodes) in &graph.transaction_external_container_children {
                for (index, node) in nodes.iter().enumerate() {
                    record(
                        format!("transaction_external_container_children[{parent}][{index}]"),
                        node,
                    );
                }
            }
            for (parent, nodes) in &graph.transaction_external_aggregate_children {
                for (index, node) in nodes.iter().enumerate() {
                    record(
                        format!("transaction_external_aggregate_children[{parent}][{index}]"),
                        node,
                    );
                }
            }
            for (parent, nodes) in &graph.trace_container_children {
                for (index, node) in nodes.iter().enumerate() {
                    record(format!("trace_container_children[{parent}][{index}]"), node);
                }
            }
            aliases
        };
        #[cfg(feature = "diagnostic-traces")]
        let trace_owner_before = trace_position(owner);
        #[cfg(feature = "diagnostic-traces")]
        let trace_placed_position = trace_position(placed);
        #[cfg(feature = "diagnostic-traces")]
        let trace_before_position = trace_position(before);
        let previous_owner = owner.clone();
        Self::publish_diagnostic_shared_geometry(owner, placed);

        restore_unchanged(&mut owner.nodes, &previous_owner.nodes, &changed);
        restore_unchanged(
            &mut owner.transaction_external_containers,
            &previous_owner.transaction_external_containers,
            &changed,
        );
        restore_unchanged_map(
            &mut owner.transaction_external_container_children,
            &previous_owner.transaction_external_container_children,
            &changed,
        );
        restore_unchanged_map(
            &mut owner.transaction_external_aggregate_children,
            &previous_owner.transaction_external_aggregate_children,
            &changed,
        );
        restore_unchanged_map(
            &mut owner.trace_container_children,
            &previous_owner.trace_container_children,
            &changed,
        );
        #[cfg(feature = "diagnostic-traces")]
        if let Some(trace_target) = trace_target {
            let trace_owner_after = trace_position(owner);
            if trace_owner_before != trace_owner_after || changed.contains(&trace_target) {
                let caller = std::panic::Location::caller();
                let owner_geometry_before = geometries(&previous_owner).get(&trace_target).cloned();
                let owner_geometry_after = geometries(owner).get(&trace_target).cloned();
                eprintln!(
                    "D2RS_PUBLISH_OWNER caller={}:{} target={} before={trace_before_position:?} placed={trace_placed_position:?} owner_before={trace_owner_before:?} owner_after={trace_owner_after:?} before_geometry={:?} placed_geometry={:?} owner_before_geometry={owner_geometry_before:?} owner_after_geometry={owner_geometry_after:?} before_aliases={:?} placed_aliases={:?} owner_before_aliases={:?} owner_after_aliases={:?} changed={}",
                    caller.file(),
                    caller.line(),
                    trace_target,
                    before_geometry.get(&trace_target),
                    placed_geometry.get(&trace_target),
                    trace_aliases(before),
                    trace_aliases(placed),
                    trace_aliases(&previous_owner),
                    trace_aliases(owner),
                    changed.contains(&trace_target),
                );
            }
        }
        changed
    }

    #[cfg(feature = "diagnostic-traces")]
    fn trace_scope_raw_handoff(
        root: &str,
        phase: &str,
        component: Option<usize>,
        workspace: &ArenaGraph,
        owner: Option<&ArenaGraph>,
    ) {
        if !crate::internal::trace_env_enabled("D2RS_TRACE_RAW_HANDOFF")
            || crate::internal::trace_env_value("D2RS_TRACE_RAW_HANDOFF_ROOT")
                .is_some_and(|target| target != root)
        {
            return;
        }
        let geometry = |graph: &ArenaGraph| {
            serde_json::json!({
                "node_order": graph.node_order.iter().map(|n| graph.nodes[n.0 as usize].tala_id).collect::<Vec<_>>(),
                "nodes": graph.nodes.iter().enumerate().map(|(i,n)| serde_json::json!({
                    "id":n.tala_id,"position":graph.position(NodeId(i as u32)),
                    "raw_position":n.position,"width":n.rect.size.width,"height":n.rect.size.height,
                })).collect::<Vec<_>>(),
                "external_children":graph.transaction_external_container_children.iter().map(|(id,children)| serde_json::json!({
                    "parent":id,"nodes":children.iter().map(|n| serde_json::json!({"id":n.tala_id,"position":n.position,"width":n.rect.size.width,"height":n.rect.size.height})).collect::<Vec<_>>()
                })).collect::<Vec<_>>(),
            })
        };
        eprintln!(
            "{}",
            serde_json::json!({"diagnostic":"raw-scope-handoff","root":root,"phase":phase,"component":component,"workspace":geometry(workspace),"owner":owner.map(geometry)})
        );
    }

    #[cfg(feature = "diagnostic-traces")]
    fn trace_scope_nesting_witness(
        placement: &PlacementScope,
        phase: &str,
        graph: &ArenaGraph,
        component: Option<usize>,
    ) {
        if !crate::internal::trace_env_enabled("WEFTAN_TRACE_NESTING_POWER_WITNESS")
            || placement.root_external_id != "center"
        {
            return;
        }
        let stable = graph
            .nodes
            .iter()
            .find(|node| node.external_id == "directions.l")
            .map(|node| (node.position, node.rect.origin, node.container));
        let external = graph
            .transaction_external_containers
            .iter()
            .find(|node| node.external_id == "directions.l")
            .map(|node| {
                (
                    node.position,
                    node.rect.origin,
                    node.transaction_validation_only,
                )
            });
        let child = graph
            .transaction_external_container_children
            .values()
            .flatten()
            .find(|node| node.external_id == "directions.l")
            .map(|node| (node.position, node.rect.origin));
        eprintln!(
            "NESTING_WITNESS_SCOPE_RUST phase={} component={:?} stable={:?} external={:?} child={:?}",
            phase, component, stable, external, child
        );
    }

    /// Copies the production children-graph carriers before any component
    /// splitting, placement, or direct scorer invocation.
    pub(super) fn hydrate_placement_scope(&mut self, placement: &PlacementScope) {
        self.graph.transaction_validation_external_containers =
            placement.transaction_validation_external_containers.clone();
        // Keep the complete owner hierarchy available to diagnostic snapshots.
        // The executable placement graph remains direct-child-only, matching
        // the Rust transaction model; this field is read only by the
        // diagnostic projection and cannot affect layout decisions.
        self.graph.trace_container_children = placement.trace_container_children.clone();
        if placement.temporary_edge_order.len() == self.graph.edge_order.len() {
            self.graph
                .edge_order
                .replace_values(&placement.temporary_edge_order.clone());
        }
        for (&edge, &(min_width, min_height)) in &placement.projected_edge_minimums {
            if let Some(projected) = self.graph.edge_records.get_mut(edge.0 as usize) {
                projected.min_width = min_width;
                projected.min_height = min_height;
            }
        }
        // Recovered `CopyEntitiesFrom` shares `Graph.Directions`; constructing
        // a standalone Rust `Graph` above would otherwise retain only the
        // current scope's explicit direction.
        self.graph
            .scoring_directions_by_tala
            .clone_from(&placement.scoring_directions_by_tala);
        for (&edge, &(source_count, target_count)) in &placement.edge_table_column_counts {
            let projected = &mut self.graph.edge_records[edge.0 as usize];
            projected.source_table_column_count = source_count;
            projected.target_table_column_count = target_count;
        }
        // TALA keeps extracted branching-tree descendants in the shared
        // Graph so PlaceTrees can reconnect and position them later, but they
        // are absent from the temporary childrenGraph optimized by this
        // self. The stable arena uses one dense node slice, so keep those
        // auxiliary IDs addressable while excluding them from every ordinary
        // placement component and its initializer.
        self.graph
            .node_order
            .retain(|node| !placement.tree_auxiliary_nodes.contains(node));
        // Auxiliary tree objects are addressable in the numerical arena,
        // but absent from both childrenGraph.Nodes and Containers until
        // reconnectTree appends them in placement preorder. The standalone
        // input constructor included them as ordinary root children.
        if !placement.tree_auxiliary_nodes.is_empty() {
            let children = self.graph.containers.get(&None).unwrap_or_default();
            self.graph.containers.insert(
                None,
                children
                    .into_iter()
                    .filter(|node| !placement.tree_auxiliary_nodes.contains(node))
                    .collect(),
            );
        }
        // Keep Graph.Nodes ownership separate from the root's child list.
        self.graph.active_graph_node_order = self.graph.node_order.clone();
        if crate::internal::trace_env_enabled("WEFTAN_TRACE_PLACEMENT_EDGES") {
            let root_tala_id = placement
                .scoring_node_metadata
                .first()
                .and_then(|metadata| metadata.parent_tala_id)
                .unwrap_or(0);
            eprint!(
                "PLACEMENT_GRAPH_RUST root={root_tala_id} cell={} nodes",
                self.graph.cell_size
            );
            for node in &self.graph.nodes {
                eprint!(" {}(container={} pos=", node.tala_id, node.is_container);
                if let Some(position) = node.position {
                    eprint!("{},{}", position.x, position.y);
                } else {
                    eprint!("nil");
                }
                eprint!(" edges=");
                for edge in &node.edges.read() {
                    eprint!(
                        "{}#{},",
                        edge.0, self.graph.edge_records[edge.0 as usize].input_id.0
                    );
                }
                eprint!(")");
            }
            for (index, edge) in self.graph.member_edge_entries() {
                eprint!(
                    " edge={} from={} to={} min={},{} label=",
                    index,
                    self.graph.nodes[edge.from.0 as usize].tala_id,
                    self.graph.nodes[edge.to.0 as usize].tala_id,
                    edge.min_width,
                    edge.min_height
                );
                if let Some(label) = &edge.label {
                    eprint!("{:?}", label.text);
                } else {
                    eprint!("nil");
                }
            }
            eprintln!();
        }
        // `SplitSubgraphs(..., traverseTrees=true)` walks each node's
        // reachable tree pointers as well as ordinary child edges. The
        // endpoint-local replay below replaces only the ordinary projected
        // slice; retained tree sentinel edges were already installed while
        // materializing `placement.graph` and must stay attached to their
        // shared parent/child nodes.
        let tree_edge_ids = placement
            .tree_edge_old_to_new
            .values()
            .copied()
            .collect::<BTreeSet<_>>();
        for (&node, edges) in &placement.incident_edge_order {
            let retained_tree_edges = self.graph.nodes[node.0 as usize]
                .edges
                .read()
                .iter()
                .copied()
                .filter(|edge| tree_edge_ids.contains(edge))
                .collect::<Vec<_>>();
            let node_edges = &mut self.graph.nodes[node.0 as usize].edges;
            *node_edges = IncidentEdges::literal(edges);
            for edge in retained_tree_edges {
                if !node_edges.contains(&edge) {
                    node_edges.push(edge);
                }
            }
        }
        self.graph
            .incident_edge_order
            .clone_from(&placement.incident_edge_order);
        if crate::internal::trace_env_enabled("WEFTAN_TRACE_PLACEMENT_SUBGRAPHS") {
            eprintln!("PLACEMENT_EDGES_AFTER_COPY_RUST");
            for node in &self.graph.nodes {
                eprintln!(
                    " node={} input={} edges={:?} order={}",
                    node.tala_id,
                    node.input_id.0,
                    node.edges
                        .read()
                        .iter()
                        .map(|edge| edge.0)
                        .collect::<Vec<_>>(),
                    self.graph
                        .node_order
                        .iter()
                        .position(|candidate| *candidate == node.input_id)
                        .unwrap_or(usize::MAX)
                );
            }
        }
        self.graph
            .tree_routing_nodes
            .clone_from(&placement.tree_routing_nodes);
        self.graph.tree_sentinels = placement
            .tree_routing_nodes
            .values()
            .map(|tree| tree.parent)
            .collect();
        // Keep retained tree nodes in the temporary graph's traversal order.
        // Go's SplitSubgraphs(..., traverseTrees=true) includes those nodes
        // in the optimizer subgraph, even though they were absent from the
        // owning Graph.Nodes slice before the traversal.
        // TALA's temporary children graphs share the Hubs map installed by
        // AddHubs. SplitSubgraphs and edge abduction do not recompute it.
        self.graph.hubs.replace_values(&placement.hubs);
        self.graph
            .sized_adjacent_overrides
            .extend(placement.sized_adjacent_overrides.iter().copied());
        self.graph.sized_cluster_distance_boxes.extend(
            placement
                .sized_cluster_distance_boxes
                .iter()
                .map(|(&key, geometry)| (key, geometry.clone())),
        );
        self.graph
            .sized_edge_abductions
            .clone_from(&placement.sized_edge_abductions);
        // Graph.abductEdges returns a real allocated slice even when it has
        // no records. Node.getSymmetry distinguishes that from a nil argument.
        self.graph.sized_edge_abductions_present = true;
        self.graph.sized_projected_obstructions.extend(
            placement
                .sized_projected_obstructions
                .iter()
                .map(|(&edge, obstructions)| (edge, obstructions.clone())),
        );
        self.graph.sized_projected_obstruction_sets.extend(
            placement
                .sized_projected_obstruction_sets
                .iter()
                .map(|(&edge, inventory)| (edge, inventory.clone())),
        );
        self.graph
            .sized_collapsed_symmetry_neighbors
            .clone_from(&placement.sized_collapsed_symmetry_neighbors);
        self.graph
            .transaction_external_containers
            .clone_from(&placement.transaction_external_containers);
        #[cfg(feature = "diagnostic-traces")]
        Self::trace_scope_nesting_witness(placement, "scope-external-copy", &self.graph, None);
        if crate::internal::trace_env_enabled("WEFTAN_TRACE_EXTERNAL_MARKERS") {
            eprintln!(
                "EXTERNAL_MARKERS_SCOPE root={:?} placement={:?} graph={:?}",
                placement.root_external_id,
                placement
                    .transaction_external_containers
                    .iter()
                    .map(|node| (node.tala_id, node.transaction_validation_only))
                    .collect::<Vec<_>>(),
                self.graph
                    .transaction_external_containers
                    .iter()
                    .map(|node| (node.tala_id, node.transaction_validation_only))
                    .collect::<Vec<_>>()
            );
        }
        self.graph
            .transaction_validation_external_containers
            .clone_from(&placement.transaction_validation_external_containers);
        self.graph
            .transaction_external_container_children
            .clone_from(&placement.transaction_external_container_children);
        self.graph
            .transaction_external_aggregate_children
            .clone_from(&placement.transaction_external_aggregate_children);
        self.graph
            .transaction_external_cluster_layouts
            .clone_from(&placement.transaction_external_cluster_layouts);
        self.graph
            .projected_transaction_nodes
            .clone_from(&placement.projected_transaction_nodes);
        if crate::internal::trace_env_enabled("WEFTAN_TRACE_MOVE_PROJECTION") {
            eprintln!(
                "MOVE_PROJECTION_SCOPE_GRAPH_RUST initial={:?}",
                self.graph
                    .transaction_external_container_children
                    .keys()
                    .collect::<Vec<_>>()
            );
        }
        for metadata in &placement.scoring_node_metadata {
            let node = &mut self.graph.nodes[metadata.node.0 as usize];
            node.is_container = metadata.is_container;
            node.scoring_is_container = metadata.is_container;
            node.scoring_container_parent = metadata.parent_tala_id;
            node.scoring_cluster_arrangement = metadata.cluster_arrangement;
            node.scoring_is_aggregate_vessel = metadata.is_aggregate_vessel;
            node.scoring_container_ancestors
                .clone_from(&metadata.container_ancestors);
            node.herd_assignment.clone_from(&metadata.herd_assignment);
            node.hierarchy = metadata.hierarchy.clone();
            node.position = metadata.position;
            node.node_padding = metadata.node_padding;
            node.content_insets = metadata.content_insets;
            // AddCluster/AddSequence replace the member with a label-less
            // vessel in the temporary Graph. The source metadata still comes
            // from the first member, so do not reattach that member's outside
            // label/icon to the synthetic vessel before CombineSubgraphs
            // computes its bounds.
            if metadata.is_aggregate_vessel {
                node.label_size = None;
                node.label_position = LabelPosition::Unset;
                node.external_label = None;
                node.icon_position = None;
            } else {
                node.label_size = metadata.label_size;
                node.label_position = metadata.label_position;
                node.external_label = metadata.external_label;
                node.icon_position = metadata.icon_position;
            }
            if let Some(tala_id) = metadata.tala_id {
                node.tala_id = tala_id;
            }
        }
        if crate::internal::trace_env_enabled("WEFTAN_TRACE_PLACEMENT_ABDUCTIONS") {
            let root_tala_id = placement
                .scoring_node_metadata
                .first()
                .and_then(|metadata| metadata.parent_tala_id)
                .unwrap_or(0);
            eprint!(
                "PLACEMENT_ABDUCTIONS_RUST root={root_tala_id} count={}",
                placement.sized_edge_abductions.len()
            );
            for abduction in &placement.sized_edge_abductions {
                eprint!(
                    " edge={} from={:?} to={:?} origFrom={:?}@{:?},{:?}:{},{} origTo={:?}@{:?},{:?}:{},{}",
                    abduction.edge_tala_id,
                    abduction
                        .current_from
                        .and_then(|endpoint| self.graph.abduction_current_tala_id(endpoint)),
                    abduction
                        .current_to
                        .and_then(|endpoint| self.graph.abduction_current_tala_id(endpoint)),
                    abduction
                        .originally_from
                        .as_ref()
                        .map(|original| original.tala_id),
                    abduction
                        .originally_from
                        .as_ref()
                        .and_then(|original| original.offset.map(|offset| offset.x)),
                    abduction
                        .originally_from
                        .as_ref()
                        .and_then(|original| original.offset.map(|offset| offset.y)),
                    abduction
                        .originally_from
                        .as_ref()
                        .map_or(0.0, |original| original.size.width),
                    abduction
                        .originally_from
                        .as_ref()
                        .map_or(0.0, |original| original.size.height),
                    abduction
                        .originally_to
                        .as_ref()
                        .map(|original| original.tala_id),
                    abduction
                        .originally_to
                        .as_ref()
                        .and_then(|original| original.offset.map(|offset| offset.x)),
                    abduction
                        .originally_to
                        .as_ref()
                        .and_then(|original| original.offset.map(|offset| offset.y)),
                    abduction
                        .originally_to
                        .as_ref()
                        .map_or(0.0, |original| original.size.width),
                    abduction
                        .originally_to
                        .as_ref()
                        .map_or(0.0, |original| original.size.height)
                );
            }
            eprintln!();
        }
        for &(first, second) in &placement.assigned_nears {
            if !self.graph.nodes[first.0 as usize].nears.contains(&second) {
                self.graph.nodes[first.0 as usize].nears.push(second);
            }
            if !self.graph.nodes[second.0 as usize].nears.contains(&first) {
                self.graph.nodes[second.0 as usize].nears.push(first);
            }
        }
        for siblings in &placement.common_uncle_groups {
            for sibling in siblings.iter().copied() {
                if self
                    .graph
                    .common_uncle_siblings
                    .get(&sibling)
                    .is_none_or(|old| old.len() < siblings.len())
                {
                    if self.graph.common_uncle_siblings.is_nil() {
                        self.graph
                            .common_uncle_siblings
                            .replace_header(SharedSliceMap::new());
                    }
                    self.graph
                        .common_uncle_siblings
                        .insert(sibling, siblings.clone());
                }
            }
        }
    }

    pub(super) fn place_flat_scope(
        placement: &PlacementScope,
        seed: i64,
        recovered_component_bounds: bool,
    ) -> Result<FlatScopePlacement, crate::LayoutError> {
        if let Some(source) = &placement.owner_source {
            let mut owner = Self::new_untraced(&source.input, seed, false, true);
            owner.graph = source.graph.clone();
            owner.source_orientation_disabled_for_loop =
                source.source_orientation_disabled_for_loop;
            owner.graph.bind_owner_placement_directions();
            let (prepared_placement, prepared) = owner
                .prepare_placement_scope_after(source.root, &source.materialized)?
                .ok_or_else(|| {
                    crate::LayoutError::PlacementTransaction(
                        "owning placement scope disappeared".into(),
                    )
                })?;
            let prepared = prepared.ok_or_else(|| {
                crate::LayoutError::PlacementTransaction(
                    "placement scope requires complete canonical owner bindings".into(),
                )
            })?;
            return owner.solve_prepared_owner_scope(&prepared_placement, Some(&prepared));
        }
        Self::place_flat_scope_with_owner_session(placement, seed, recovered_component_bounds, None)
    }

    fn place_flat_scope_with_owner_session(
        placement: &PlacementScope,
        seed: i64,
        recovered_component_bounds: bool,
        mut owner_session: Option<&mut OwnerScoreSession<'_>>,
    ) -> Result<FlatScopePlacement, crate::LayoutError> {
        let mut scope = Pipeline::new_untraced(&placement.graph, seed, false, true);
        #[cfg(feature = "diagnostic-traces")]
        {
            // This pipeline owns an independently materialized ArenaGraph.
            // Share the diagnostic owner table with its source owner so
            // SplitSubgraphs/SetGraphReference transitions are visible to
            // the root.Graph callbacks below.
            scope.graph.trace_owner_node_orders =
                placement.trace_owner_graph.trace_owner_node_orders.clone();
            scope.graph.trace_root_owner_node_orders = placement
                .trace_owner_graph
                .trace_root_owner_node_orders
                .clone();
            scope.graph.trace_root_owner_container_children = placement
                .trace_owner_graph
                .trace_root_owner_container_children
                .clone();
        }
        #[cfg(feature = "diagnostic-traces")]
        let trace_nesting_witness = |phase: &str, graph: &ArenaGraph, component: Option<usize>| {
            Self::trace_scope_nesting_witness(placement, phase, graph, component);
        };
        #[cfg(feature = "diagnostic-traces")]
        trace_nesting_witness("scope-input", &scope.graph, None);
        #[cfg(feature = "diagnostic-traces")]
        let mut trace_owner_graph = placement.trace_owner_graph.clone();
        #[cfg(feature = "diagnostic-traces")]
        let trace_owner_nodes_by_tala = trace_owner_graph
            .nodes
            .iter()
            .enumerate()
            .map(|(index, node)| (node.tala_id, index))
            .collect::<BTreeMap<_, _>>();
        scope.trace_root = placement.root_external_id.clone();
        scope.hydrate_placement_scope(placement);
        let mut next_rng_float = None;
        let mut descendant_mirrors = BTreeMap::new();
        let mut cluster_arrangements = BTreeMap::new();
        let mut cluster_desired_arrangements = BTreeMap::new();
        // Recovered Graph.placeNodes partitions children before computing cell
        // size or constructing either optimizer. Each SplitSubgraphs result
        // receives a fresh RNG seeded with the same randomSeed, is directed
        // independently, and is only then packed by CombineSubgraphs.
        let subgraphs = scope.graph.split_placement_subgraphs();
        // SplitSubgraphs redirects roots of ALL components before placement
        // starts (graph.go730–745), while source node_placement.go163–167
        // later computes each cell size and redirects its descendants. Keep
        // those two ownership phases separate; a later component's root is
        // already foreign to the first component's Graph during its scores.
        let mut prepared_owner_components = Vec::with_capacity(subgraphs.len());
        for members in &subgraphs {
            let prepared = if let Some(session) = owner_session.as_deref_mut() {
                let (component, parent_nodes, parent_edges) = scope
                    .graph
                    .induced_placement_subgraph_with_edge_mapping(members);
                Some(
                    session
                        .prepare_component(
                            &parent_nodes,
                            &parent_edges,
                            &component.graph_node_order(),
                        )
                        .map_err(|error| {
                            crate::LayoutError::PlacementTransaction(error.to_string())
                        })?,
                )
            } else {
                None
            };
            prepared_owner_components.push(prepared);
        }
        let mut prepared_owner_components = prepared_owner_components.into_iter();
        #[cfg(feature = "diagnostic-traces")]
        crate::internal::pipeline::trace_placement_state(
            "after-split",
            &placement.root_external_id,
            &trace_owner_graph,
        );
        let mut mirrored_external_child_offsets = Vec::new();
        // Visible members in each induced component are Go shared pointers.
        // Keep their final component-local geometry so later external-carrier
        // copybacks cannot replace it with an ancestor frame before packing.
        let mut local_member_positions = Vec::<(NodeId, Point, Size)>::new();
        if crate::internal::trace_env_enabled("WEFTAN_TRACE_PLACEMENT_SUBGRAPHS") {
            eprintln!(
                "PLACEMENT_SUBGRAPHS_RUST count={} {:?}",
                subgraphs.len(),
                subgraphs
                    .iter()
                    .map(|members| {
                        members
                            .iter()
                            .map(|node| scope.graph.nodes[node.0 as usize].tala_id)
                            .collect::<Vec<_>>()
                    })
                    .collect::<Vec<_>>()
            );
        }
        let mut combined_subgraphs = Vec::with_capacity(subgraphs.len());
        for (subgraph_index, members) in subgraphs.into_iter().enumerate() {
            #[cfg(not(feature = "diagnostic-traces"))]
            let _ = subgraph_index;
            #[cfg(feature = "diagnostic-traces")]
            crate::internal::pipeline::trace_placement_state(
                &format!("before-subgraph-{subgraph_index}"),
                &placement.root_external_id,
                &trace_owner_graph,
            );
            if crate::internal::trace_env_enabled("WEFTAN_TRACE_PLACEMENT_SUBGRAPHS") {
                eprintln!(
                    "SUBGRAPH_ROOT_RUST members={:?} hierarchy={:?}",
                    members
                        .iter()
                        .map(|node| scope.graph.nodes[node.0 as usize].tala_id)
                        .collect::<Vec<_>>(),
                    members
                        .first()
                        .and_then(|node| scope.graph.nodes[node.0 as usize].hierarchy.as_ref())
                        .map(|h| h.id),
                );
            }
            let (mut subgraph, old_by_new, parent_edges) = scope
                .graph
                .induced_placement_subgraph_with_edge_mapping(&members);
            let owner_component = match prepared_owner_components
                .next()
                .expect("one owner preparation per split component")
            {
                Some(prepared) => Some(
                    owner_session
                        .as_deref_mut()
                        .expect("prepared components retain the owning session")
                        .enter_prepared_component_with_dimensions(
                            prepared,
                            &old_by_new,
                            &parent_edges,
                            &subgraph.graph_node_order(),
                            &subgraph,
                        )
                        .map_err(|error| {
                            crate::LayoutError::PlacementTransaction(error.to_string())
                        })?,
                ),
                None => None,
            };
            if let Some(session) = owner_session.as_deref() {
                session
                    .bind_current_workspace_costs(&mut subgraph)
                    .map_err(|error| crate::LayoutError::PlacementTransaction(error.to_string()))?;
            }
            // `PlaceHierarchies` owns this component's internal coordinates.
            // TALA's temporary graph shares the positioned node pointers and
            // skips ordinary placement when the first component node carries
            // a hierarchy.
            if members
                .first()
                .is_some_and(|node| scope.graph.nodes[node.0 as usize].hierarchy.is_some())
            {
                let hierarchy_node = members[0];
                // Graph.placeNodes still runs syncNested for a hierarchy-owned
                // child graph before it skips recursive ordinary placement.
                // Keep the shared container pointers aligned at this boundary
                // even though the hierarchy's own coordinates are already
                // arranged by PlaceHierarchies.
                #[cfg(feature = "diagnostic-traces")]
                let trace_before_hierarchy_sync = scope.graph.clone();
                if crate::internal::trace_env_value("WEFTAN_TRACE_HIERARCHY_SYNC").is_some_and(
                    |target| {
                        target == "all"
                            || target
                                == scope.graph.nodes[hierarchy_node.0 as usize]
                                    .tala_id
                                    .to_string()
                    },
                ) {
                    eprintln!(
                        "HIERARCHY_SYNC_RUST before node={} children={:?}",
                        scope.graph.nodes[hierarchy_node.0 as usize].tala_id,
                        scope
                            .graph
                            .transaction_external_container_children
                            .get(&scope.graph.nodes[hierarchy_node.0 as usize].tala_id)
                            .map(|children| children
                                .iter()
                                .map(|child| (child.tala_id, child.position))
                                .collect::<Vec<_>>())
                    );
                }
                scope.graph.sync_nested_projected_container_children();
                if crate::internal::trace_env_value("WEFTAN_TRACE_HIERARCHY_SYNC").is_some_and(
                    |target| {
                        target == "all"
                            || target
                                == scope.graph.nodes[hierarchy_node.0 as usize]
                                    .tala_id
                                    .to_string()
                    },
                ) {
                    eprintln!(
                        "HIERARCHY_SYNC_RUST after node={} children={:?}",
                        scope.graph.nodes[hierarchy_node.0 as usize].tala_id,
                        scope
                            .graph
                            .transaction_external_container_children
                            .get(&scope.graph.nodes[hierarchy_node.0 as usize].tala_id)
                            .map(|children| children
                                .iter()
                                .map(|child| (child.tala_id, child.position))
                                .collect::<Vec<_>>())
                    );
                }
                // Go skips orthogonal placement and cluster optimization when
                // the first component node has a Hierarchy, but still emits
                // the post-optimization snapshot and runs PlaceTrees before
                // restoring edges and calling direct. Keep those stage
                // boundaries, including tree reconnection, in this path too.
                #[cfg(feature = "diagnostic-traces")]
                {
                    Self::publish_diagnostic_shared_geometry_since(
                        &mut trace_owner_graph,
                        &scope.graph,
                        &trace_before_hierarchy_sync,
                    );
                    crate::internal::pipeline::trace_placement_state(
                        &format!("after-cluster-optimize-{subgraph_index}"),
                        &placement.root_external_id,
                        &trace_owner_graph,
                    );
                }
                #[cfg(feature = "diagnostic-traces")]
                let trace_before_hierarchy_trees = scope.graph.clone();
                scope.graph.place_trees(&members);
                #[cfg(feature = "diagnostic-traces")]
                {
                    Self::publish_diagnostic_shared_geometry_since(
                        &mut trace_owner_graph,
                        &scope.graph,
                        &trace_before_hierarchy_trees,
                    );
                    // Go's component and owner share the root's current child
                    // slice. Rust stores it under None in the induced graph,
                    // so project that slice for this owner snapshot.
                    if let Some(scope_children) = scope.graph.containers.get(&None)
                        && let Some(owner_parent_dense_index) = trace_owner_graph
                            .nodes
                            .iter()
                            .position(|node| node.external_id == placement.root_external_id)
                        && let Some(&owner_parent_index) = trace_owner_nodes_by_tala
                            .get(&trace_owner_graph.nodes[owner_parent_dense_index].tala_id)
                    {
                        let owner_children = scope_children
                            .iter()
                            .filter_map(|child| {
                                let tala_id = scope.graph.nodes[child.0 as usize].tala_id;
                                trace_owner_nodes_by_tala
                                    .get(&tala_id)
                                    .map(|index| NodeId(*index as u32))
                            })
                            .collect();
                        trace_owner_graph
                            .containers
                            .insert(Some(NodeId(owner_parent_index as u32)), owner_children);
                    }
                    crate::internal::pipeline::trace_placement_state(
                        &format!("after-trees-{subgraph_index}"),
                        &placement.root_external_id,
                        &trace_owner_graph,
                    );
                }
                #[cfg(feature = "diagnostic-traces")]
                let trace_before_hierarchy_restore = scope.graph.clone();
                scope.graph.append_restored_projected_transaction_nodes();
                #[cfg(feature = "diagnostic-traces")]
                {
                    // Go adds PlaceTrees' retained nodes to the owner after
                    // after-trees and before edge restoration.
                    let copyback_owner_nodes = scope
                        .graph
                        .placement_tree_owner_copyback_order(&members)
                        .into_iter()
                        .filter_map(|tree_node| {
                            let tala_id = scope.graph.nodes[tree_node.0 as usize].tala_id;
                            trace_owner_nodes_by_tala
                                .get(&tala_id)
                                .copied()
                                .map(|owner_node| NodeId(owner_node as u32))
                        })
                        .collect::<Vec<_>>();
                    trace_owner_graph
                        .node_order
                        .retain(|node| !copyback_owner_nodes.contains(node));
                    trace_owner_graph.node_order.extend(copyback_owner_nodes);
                    Self::publish_diagnostic_shared_geometry_since(
                        &mut trace_owner_graph,
                        &scope.graph,
                        &trace_before_hierarchy_restore,
                    );
                    crate::internal::pipeline::trace_placement_state(
                        &format!("after-restore-edges-{subgraph_index}"),
                        &placement.root_external_id,
                        &trace_owner_graph,
                    );
                }
                let mut direct_members = members.clone();
                for tree_node in scope.graph.placement_tree_reconnect_order(&members) {
                    if direct_members.contains(&tree_node)
                        || !scope.graph.tree_routing_nodes.contains_key(&tree_node)
                    {
                        continue;
                    }
                    let mut sentinel = scope.graph.tree_routing_nodes[&tree_node].parent;
                    while let Some(parent) = scope.graph.tree_routing_nodes.get(&sentinel) {
                        sentinel = parent.parent;
                    }
                    if members.contains(&sentinel) {
                        direct_members.push(tree_node);
                    }
                }
                let (mut hierarchy_trace_graph, _) =
                    scope.graph.induced_placement_subgraph(&direct_members);
                #[cfg(feature = "diagnostic-traces")]
                crate::internal::pipeline::trace_placement_state(
                    "before-direct",
                    &placement.root_external_id,
                    &hierarchy_trace_graph,
                );
                // Go's direct returns immediately when the first node carries
                // a Hierarchy. Running that same early-return predicate here
                // preserves the stage without changing this component.
                #[cfg(feature = "diagnostic-traces")]
                let trace_before_hierarchy_direct = hierarchy_trace_graph.clone();
                hierarchy_trace_graph.direct_with_reachable(false, None);
                #[cfg(feature = "diagnostic-traces")]
                {
                    crate::internal::pipeline::trace_placement_state(
                        "after-direct",
                        &placement.root_external_id,
                        &hierarchy_trace_graph,
                    );
                    let changed = Self::publish_diagnostic_shared_geometry_since(
                        &mut trace_owner_graph,
                        &hierarchy_trace_graph,
                        &trace_before_hierarchy_direct,
                    );
                    crate::internal::pipeline::trace_remember_owner_geometry_changes(
                        &trace_owner_graph,
                        &changed,
                    );
                    crate::internal::pipeline::trace_placement_state(
                        &format!("after-direct-owner-{subgraph_index}"),
                        &placement.root_external_id,
                        &trace_owner_graph,
                    );
                    crate::internal::pipeline::trace_placement_state(
                        &format!("after-subgraph-{subgraph_index}"),
                        &placement.root_external_id,
                        &trace_owner_graph,
                    );
                }
                if let Some(component) = owner_component {
                    owner_session
                        .as_deref_mut()
                        .expect("hierarchy component retains the owning session")
                        .leave_component(component, true)
                        .map_err(|error| {
                            crate::LayoutError::PlacementTransaction(error.to_string())
                        })?;
                }
                combined_subgraphs.push(direct_members);
                continue;
            }
            #[cfg(feature = "diagnostic-traces")]
            trace_nesting_witness("component-input", &subgraph, Some(subgraph_index));
            if crate::internal::trace_env_enabled("WEFTAN_TRACE_PLACEMENT_CELL") {
                eprintln!(
                    "PLACEMENT_CELL_RUST root={:?} cell={} nodes={}",
                    placement
                        .scoring_node_metadata
                        .first()
                        .and_then(|metadata| metadata.parent_tala_id),
                    subgraph.cell_size,
                    subgraph.graph_node_order().len()
                );
            }
            let edge_abduction_nodes = old_by_new
                .iter()
                .enumerate()
                .filter_map(|(new_index, old)| {
                    placement
                        .edge_abduction_nodes
                        .contains(old)
                        .then_some(NodeId(new_index as u32))
                })
                .collect::<BTreeSet<_>>();
            // `placeNodes` checks the owning graph's node count for its
            // singleton fast path. A recursive subgraph may contain one
            // direct child while the owning graph still has descendants; in
            // that case Go runs placeNodesOrthogonally for the child and
            // emits the complete placement lifecycle.
            // A true one-node owner takes Go's singleton path. Tree-only
            // components are also singleton at the optimizer boundary: Go's
            // tree placement handles their retained descendants directly and
            // does not run the orthogonal optimizer for the one-child slice.
            let root_scope = placement
                .scoring_node_metadata
                .first()
                .and_then(|metadata| metadata.parent_tala_id)
                .is_none();
            // At the top level, tree preprocessing can leave exactly one
            // visible sentinel in Go's Graph.Nodes while its retained tree
            // descendants remain reachable only through Graph.Trees. Go
            // takes placeNodes' singleton return before SplitSubgraphs in
            // that case, so Rust must bypass the otherwise empty optimizer
            // scope as well. Nested scopes still count their tree descendants
            // in the owning graph and must keep the ordinary placement path.
            let root_tree_singleton = root_scope
                && placement.owner_node_count == 1
                && !placement.tree_auxiliary_nodes.is_empty();
            let owning_graph_singleton = placement.owning_graph_node_count == 1
                && !(placement.owner_node_count == 1 && !placement.tree_auxiliary_nodes.is_empty());
            let singleton_scope = placement.transaction_external_container_children.is_empty()
                && (owning_graph_singleton || root_tree_singleton);
            if crate::internal::trace_env_enabled("WEFTAN_TRACE_SINGLETON_PATH") {
                eprintln!(
                    "SINGLETON_SCOPE_RUST root={:?} owner_nodes={} graph_nodes={} external_children={} tree_auxiliary={} fast_path={}",
                    placement
                        .scoring_node_metadata
                        .first()
                        .and_then(|metadata| metadata.parent_tala_id),
                    placement.owning_graph_node_count,
                    subgraph.nodes.len(),
                    placement.transaction_external_container_children.len(),
                    placement.tree_auxiliary_nodes.len(),
                    singleton_scope,
                );
            }
            let mut sub_scope = Pipeline {
                input: placement.graph.clone(),
                graph: subgraph,
                sized_obstacles: placement.sized_obstacles.clone(),
                seed,
                prearranged: false,
                fixed_sizes: true,
                completed_stages: Vec::new(),
                misc_rng: go_rng::GoRng::new_untraced(seed),
                node_placement_rng: None,
                hierarchy_rng: go_rng::GoRng::new_untraced(seed),
                hierarchy_assignments: Vec::new(),
                hierarchy_placements: Vec::new(),
                next_rng_float: None,
                run_align_axis: true,
                run_edge_route: false,
                singleton_fast_path: singleton_scope,
                suppress_route_trace: false,
                trace_root: placement.root_external_id.clone(),
                route_stage_work: routing::RouteStageWork::new(
                    routing::MAX_EDGE_ROUTING_STAGE_WORK_UNITS,
                ),
                source_orientation_disabled_for_loop: placement
                    .source_orientation
                    .disabled_for_loop,
            };
            let opens_node_placement_scope =
                !singleton_scope
                    && sub_scope.graph.node_order.first().is_some_and(|node| {
                        sub_scope.graph.nodes[node.0 as usize].hierarchy.is_none()
                    });
            if opens_node_placement_scope {
                // D2 v0.9.0 creates the placement RNG and records the scope
                // before InitializeNodes, then carries that same stream into
                // the later optimizer. Keep the Rust lifetime at that boundary
                // even though optimization itself starts after synchronization.
                sub_scope.node_placement_rng =
                    Some(go_rng::GoRng::with_stream(seed, "node-placement"));
                sizeless::SizelessOptimizer::trace_rng_scope(
                    &sub_scope.graph,
                    Some(&sub_scope.trace_root),
                );
            }
            if crate::internal::trace_env_enabled("WEFTAN_TRACE_SCOPE_STEPS") {
                eprintln!(
                    "SCOPE_STEP_RUST before_initialize root={:?} members={:?}",
                    placement
                        .scoring_node_metadata
                        .first()
                        .and_then(|metadata| metadata.parent_tala_id),
                    members
                        .iter()
                        .map(|node| scope.graph.nodes[node.0 as usize].tala_id)
                        .collect::<Vec<_>>()
                );
            }
            #[cfg(feature = "diagnostic-traces")]
            if !singleton_scope && !placement.root_external_id.is_empty() {
                crate::internal::pipeline::trace_placement_state_with_cell_size(
                    "orthogonal-start",
                    &placement.root_external_id,
                    &trace_owner_graph,
                    sub_scope.graph.cell_size,
                );
            }
            #[cfg(feature = "diagnostic-traces")]
            let trace_before_initialize = sub_scope.graph.clone();
            if seed % 2 != 0 || !sub_scope.graph.initialize_by_graph_distance() {
                sub_scope.run_initialize_nodes()?;
            }
            #[cfg(feature = "diagnostic-traces")]
            trace_nesting_witness(
                "component-initialize",
                &sub_scope.graph,
                Some(subgraph_index),
            );
            let trace_scope_pos = crate::internal::trace_env_value("WEFTAN_TRACE_SCOPE_POS")
                .is_some_and(|value| {
                    value == "all"
                        || placement
                            .scoring_node_metadata
                            .first()
                            .and_then(|metadata| metadata.parent_tala_id)
                            .is_some_and(|root| value.parse::<i64>().ok() == Some(root))
                });
            if trace_scope_pos {
                eprintln!(
                    "SCOPE_POS_RUST phase=after-initialize {:?}",
                    sub_scope
                        .graph
                        .nodes
                        .iter()
                        .map(|node| (node.tala_id, node.position, node.rect.size))
                        .collect::<Vec<_>>()
                );
                eprintln!(
                    "SCOPE_CLUSTER_POS_RUST phase=after-initialize pending={:?} vessel={:?} aggregate={:?} containers={:?}",
                    sub_scope
                        .graph
                        .pending_cluster_vessel_positions
                        .iter()
                        .map(|(index, position)| (
                            sub_scope.graph.clusters[*index].vessel_tala_id,
                            *position
                        ))
                        .collect::<Vec<_>>(),
                    sub_scope
                        .graph
                        .projected_node_position_by_tala(2305561650894865143),
                    sub_scope
                        .graph
                        .transaction_external_aggregate_children
                        .iter()
                        .map(|(vessel, children)| (
                            *vessel,
                            children
                                .iter()
                                .filter(|node| matches!(
                                    node.tala_id,
                                    3946625030 | 2328842519 | 3984593615
                                ))
                                .map(|node| (node.tala_id, node.position))
                                .collect::<Vec<_>>()
                        ))
                        .collect::<Vec<_>>(),
                    sub_scope
                        .graph
                        .transaction_external_container_children
                        .iter()
                        .filter_map(|(parent, children)| {
                            let found = children
                                .iter()
                                .filter(|node| {
                                    matches!(node.tala_id, 3946625030 | 2328842519 | 3984593615)
                                })
                                .map(|node| (node.tala_id, node.position))
                                .collect::<Vec<_>>();
                            (!found.is_empty()).then_some((*parent, found))
                        })
                        .collect::<Vec<_>>()
                );
            }
            if crate::internal::trace_env_enabled("WEFTAN_TRACE_GRID_ROOT_PHASES")
                && sub_scope
                    .graph
                    .transaction_external_container_children
                    .contains_key(&233_611_931)
            {
                eprintln!(
                    "GRID_ROOT_PHASE_RUST phase=after-init {:?}",
                    sub_scope
                        .graph
                        .transaction_external_container_children
                        .get(&233_611_931)
                        .map(|children| children
                            .iter()
                            .filter_map(|child| {
                                matches!(
                                    child.tala_id,
                                    498_183_754
                                        | 1_149_337_423
                                        | 3_584_521_799
                                        | 1_782_109_120
                                        | 2_919_616_609
                                )
                                .then_some((
                                    child.tala_id,
                                    child.position,
                                    child.rect.size,
                                ))
                            })
                            .collect::<Vec<_>>())
                );
            }
            if crate::internal::trace_env_enabled("WEFTAN_TRACE_GRID28_STEPS")
                && members.iter().any(|node| {
                    matches!(
                        scope.graph.nodes[node.0 as usize].tala_id,
                        498_183_754 | 1_149_337_423 | 3_584_521_799 | 1_782_109_120 | 2_919_616_609
                    )
                })
            {
                eprintln!(
                    "GRID28_STEP_RUST after_init {:?}",
                    sub_scope
                        .graph
                        .transaction_external_container_children
                        .values()
                        .flatten()
                        .filter(|child| {
                            matches!(
                                child.tala_id,
                                498_183_754
                                    | 1_149_337_423
                                    | 3_584_521_799
                                    | 1_782_109_120
                                    | 2_919_616_609
                            )
                        })
                        .map(|child| (child.tala_id, child.position, child.rect.size))
                        .collect::<Vec<_>>()
                );
            }
            if crate::internal::trace_env_enabled("WEFTAN_TRACE_SCOPE_STEPS") {
                eprintln!(
                    "SCOPE_STEP_RUST after_initialize root={:?}",
                    placement
                        .scoring_node_metadata
                        .first()
                        .and_then(|metadata| metadata.parent_tala_id)
                );
            }
            sub_scope.graph.initialize_transaction_projected_positions();
            if trace_scope_pos {
                eprintln!(
                    "SCOPE_POS_RUST phase=after-projected-init {:?}",
                    sub_scope
                        .graph
                        .nodes
                        .iter()
                        .map(|node| (node.tala_id, node.position, node.rect.size))
                        .collect::<Vec<_>>()
                );
                eprintln!(
                    "SCOPE_CLUSTER_POS_RUST phase=after-projected-init pending={:?} vessel={:?} aggregate={:?} containers={:?}",
                    sub_scope
                        .graph
                        .pending_cluster_vessel_positions
                        .iter()
                        .map(|(index, position)| (
                            sub_scope.graph.clusters[*index].vessel_tala_id,
                            *position
                        ))
                        .collect::<Vec<_>>(),
                    sub_scope
                        .graph
                        .projected_node_position_by_tala(2305561650894865143),
                    sub_scope
                        .graph
                        .transaction_external_aggregate_children
                        .iter()
                        .map(|(vessel, children)| (
                            *vessel,
                            children
                                .iter()
                                .filter(|node| matches!(
                                    node.tala_id,
                                    3946625030 | 2328842519 | 3984593615
                                ))
                                .map(|node| (node.tala_id, node.position))
                                .collect::<Vec<_>>()
                        ))
                        .collect::<Vec<_>>(),
                    sub_scope
                        .graph
                        .transaction_external_container_children
                        .iter()
                        .filter_map(|(parent, children)| {
                            let found = children
                                .iter()
                                .filter(|node| {
                                    matches!(node.tala_id, 3946625030 | 2328842519 | 3984593615)
                                })
                                .map(|node| (node.tala_id, node.position))
                                .collect::<Vec<_>>();
                            (!found.is_empty()).then_some((*parent, found))
                        })
                        .collect::<Vec<_>>()
                );
            }
            if crate::internal::trace_env_enabled("WEFTAN_TRACE_GRID28_STEPS")
                && members.iter().any(|node| {
                    matches!(
                        scope.graph.nodes[node.0 as usize].tala_id,
                        498_183_754 | 1_149_337_423 | 3_584_521_799 | 1_782_109_120 | 2_919_616_609
                    )
                })
            {
                eprintln!(
                    "GRID28_STEP_RUST after_projected_init {:?}",
                    sub_scope
                        .graph
                        .transaction_external_container_children
                        .values()
                        .flatten()
                        .filter(|child| {
                            matches!(
                                child.tala_id,
                                498_183_754
                                    | 1_149_337_423
                                    | 3_584_521_799
                                    | 1_782_109_120
                                    | 2_919_616_609
                            )
                        })
                        .map(|child| (child.tala_id, child.position, child.rect.size))
                        .collect::<Vec<_>>()
                );
            }
            if crate::internal::trace_env_enabled("WEFTAN_TRACE_SCOPE_STEPS") {
                eprintln!(
                    "SCOPE_STEP_RUST after_transaction_init root={:?}",
                    placement
                        .scoring_node_metadata
                        .first()
                        .and_then(|metadata| metadata.parent_tala_id)
                );
            }
            if crate::internal::trace_env_value("WEFTAN_TRACE_POSITION_CHILDREN").as_deref()
                == Some("1472025070")
            {
                eprintln!(
                    "POSITION_CHILDREN_RUST_BEFORE_SYNC nodes={:?} maps={:?}",
                    members
                        .iter()
                        .map(|node| (
                            scope.graph.nodes[node.0 as usize].tala_id,
                            sub_scope.graph.position(NodeId(
                                old_by_new.iter().position(|old| old == node).unwrap() as u32
                            ))
                        ))
                        .collect::<Vec<_>>(),
                    sub_scope
                        .graph
                        .transaction_external_container_children
                        .iter()
                        .filter(|(parent, _)| **parent == 1472025070)
                        .map(|(parent, children)| (
                            *parent,
                            children
                                .iter()
                                .map(|child| (child.tala_id, child.position))
                                .collect::<Vec<_>>()
                        ))
                        .collect::<Vec<_>>()
                );
            }
            sub_scope.graph.sync_nested_projected_container_children();
            if trace_scope_pos {
                let parent_ids = [3677471320_i64, 2305561650894865143, 3946625030];
                eprintln!(
                    "SCOPE_FOCUS_RUST phase=after-sync root={:?} containers={:?} aggregates={:?}",
                    placement
                        .scoring_node_metadata
                        .first()
                        .and_then(|metadata| metadata.parent_tala_id),
                    parent_ids.map(|parent| (
                        parent,
                        sub_scope
                            .graph
                            .transaction_external_container_children
                            .get(&parent)
                            .map(|children| children
                                .iter()
                                .map(|child| (child.tala_id, child.position))
                                .collect::<Vec<_>>())
                    )),
                    parent_ids.map(|parent| (
                        parent,
                        sub_scope
                            .graph
                            .transaction_external_aggregate_children
                            .get(&parent)
                            .map(|children| children
                                .iter()
                                .map(|child| (child.tala_id, child.position))
                                .collect::<Vec<_>>())
                    )),
                );
            }
            // Ordinary source initialization is the first real shared-node
            // write in this component. Publish it to the genuine owner before
            // constructing either optimizer; diagnostic clones are separate.
            // The source singleton fast path and subsequent trial writers
            // retain their distinct, still-unqualified integration boundaries.
            if !singleton_scope && let Some(session) = owner_session.as_deref_mut() {
                session
                    .initialize_current_member_positions(&sub_scope.graph)
                    .map_err(|error| crate::LayoutError::PlacementTransaction(error.to_string()))?;
            }
            // InitializeNodes/SyncNestedGeometry mutate shared Go pointers at
            // every scope, including top where root.Graph trace hooks are
            // absent. Publish active members even if induction already gave
            // them the same fixed position; retain delta-only external alias
            // publication so an unrelated sibling keeps its current frame.
            #[cfg(feature = "diagnostic-traces")]
            if !singleton_scope {
                let changed = Self::publish_diagnostic_initialized_geometry(
                    &mut trace_owner_graph,
                    &sub_scope.graph,
                    &trace_before_initialize,
                );
                crate::internal::pipeline::trace_remember_owner_geometry_changes(
                    &trace_owner_graph,
                    &changed,
                );
                if !placement.root_external_id.is_empty() {
                    crate::internal::pipeline::trace_placement_state_with_cell_size(
                        "orthogonal-after-initialize",
                        &placement.root_external_id,
                        &trace_owner_graph,
                        sub_scope.graph.cell_size,
                    );
                }
            }
            if crate::internal::trace_env_value("WEFTAN_TRACE_EXTERNAL_SCOPE_MAP").is_some_and(
                |target| {
                    target == "all"
                        || placement
                            .scoring_node_metadata
                            .first()
                            .and_then(|metadata| metadata.parent_tala_id)
                            .is_some_and(|root| root == target.parse::<i64>().unwrap_or_default())
                },
            ) {
                eprintln!(
                    "EXTERNAL_SCOPE_MAP_RUST root={:?} map={:?} nodes={:?}",
                    placement
                        .scoring_node_metadata
                        .first()
                        .and_then(|metadata| metadata.parent_tala_id),
                    sub_scope
                        .graph
                        .transaction_external_container_children
                        .get(&4254926550)
                        .map(|children| {
                            children
                                .iter()
                                .map(|child| (child.tala_id, child.position, child.rect.size))
                                .collect::<Vec<_>>()
                        }),
                    sub_scope
                        .graph
                        .nodes
                        .iter()
                        .filter(|node| matches!(node.tala_id, 4254926550 | 1369972579 | 3471367806))
                        .map(|node| (node.tala_id, node.position, node.rect.size))
                        .collect::<Vec<_>>()
                );
            }
            if trace_scope_pos {
                eprintln!(
                    "SCOPE_POS_RUST phase=after-sync {:?}",
                    sub_scope
                        .graph
                        .nodes
                        .iter()
                        .map(|node| (node.tala_id, node.position, node.rect.size))
                        .collect::<Vec<_>>()
                );
                eprintln!(
                    "SCOPE_CLUSTER_POS_RUST phase=after-sync pending={:?} vessel={:?} aggregate={:?} containers={:?}",
                    sub_scope
                        .graph
                        .pending_cluster_vessel_positions
                        .iter()
                        .map(|(index, position)| (
                            sub_scope.graph.clusters[*index].vessel_tala_id,
                            *position
                        ))
                        .collect::<Vec<_>>(),
                    sub_scope
                        .graph
                        .projected_node_position_by_tala(2305561650894865143),
                    sub_scope
                        .graph
                        .transaction_external_aggregate_children
                        .iter()
                        .map(|(vessel, children)| (
                            *vessel,
                            children
                                .iter()
                                .filter(|node| matches!(
                                    node.tala_id,
                                    3946625030 | 2328842519 | 3984593615
                                ))
                                .map(|node| (node.tala_id, node.position))
                                .collect::<Vec<_>>()
                        ))
                        .collect::<Vec<_>>(),
                    sub_scope
                        .graph
                        .transaction_external_container_children
                        .iter()
                        .filter_map(|(parent, children)| {
                            let found = children
                                .iter()
                                .filter(|node| {
                                    matches!(node.tala_id, 3946625030 | 2328842519 | 3984593615)
                                })
                                .map(|node| (node.tala_id, node.position))
                                .collect::<Vec<_>>();
                            (!found.is_empty()).then_some((*parent, found))
                        })
                        .collect::<Vec<_>>()
                );
            }
            if crate::internal::trace_env_enabled("WEFTAN_TRACE_GRID_ROOT_PHASES")
                && sub_scope
                    .graph
                    .transaction_external_container_children
                    .contains_key(&233_611_931)
            {
                eprintln!(
                    "GRID_ROOT_PHASE_RUST phase=after-sync {:?}",
                    sub_scope
                        .graph
                        .transaction_external_container_children
                        .get(&233_611_931)
                        .map(|children| children
                            .iter()
                            .filter_map(|child| {
                                matches!(
                                    child.tala_id,
                                    498_183_754
                                        | 1_149_337_423
                                        | 3_584_521_799
                                        | 1_782_109_120
                                        | 2_919_616_609
                                )
                                .then_some((
                                    child.tala_id,
                                    child.position,
                                    child.rect.size,
                                ))
                            })
                            .collect::<Vec<_>>())
                );
            }
            if crate::internal::trace_env_enabled("WEFTAN_TRACE_GRID28_STEPS")
                && members.iter().any(|node| {
                    matches!(
                        scope.graph.nodes[node.0 as usize].tala_id,
                        498_183_754 | 1_149_337_423 | 3_584_521_799 | 1_782_109_120 | 2_919_616_609
                    )
                })
            {
                eprintln!(
                    "GRID28_STEP_RUST after_sync {:?}",
                    sub_scope
                        .graph
                        .transaction_external_container_children
                        .values()
                        .flatten()
                        .filter(|child| {
                            matches!(
                                child.tala_id,
                                498_183_754
                                    | 1_149_337_423
                                    | 3584521799
                                    | 1_782_109_120
                                    | 2_919_616_609
                            )
                        })
                        .map(|child| (child.tala_id, child.position, child.rect.size))
                        .collect::<Vec<_>>()
                );
            }
            if crate::internal::trace_env_enabled("WEFTAN_TRACE_SCOPE_STEPS") {
                eprintln!(
                    "SCOPE_STEP_RUST after_sync_nested root={:?}",
                    placement
                        .scoring_node_metadata
                        .first()
                        .and_then(|metadata| metadata.parent_tala_id)
                );
            }
            // InitializeNodes/SyncNested moves the shared child boxes into
            // their fitted container frame.  Go's EdgeAbduction retains the
            // same *Node pointers, so sizedOptimizer.getAdjacents observes
            // those post-fit coordinates.  Refresh the flattened projections
            // at the same lifecycle boundary before constructing the sized
            // optimizer state.
            sub_scope
                .graph
                .reconcile_projected_offsets_from_external_children();
            if trace_scope_pos {
                let parent_ids = [3677471320_i64, 2305561650894865143, 3946625030];
                eprintln!(
                    "SCOPE_FOCUS_RUST phase=after-reconcile root={:?} containers={:?} aggregates={:?}",
                    placement
                        .scoring_node_metadata
                        .first()
                        .and_then(|metadata| metadata.parent_tala_id),
                    parent_ids.map(|parent| (
                        parent,
                        sub_scope
                            .graph
                            .transaction_external_container_children
                            .get(&parent)
                            .map(|children| children
                                .iter()
                                .map(|child| (child.tala_id, child.position))
                                .collect::<Vec<_>>())
                    )),
                    parent_ids.map(|parent| (
                        parent,
                        sub_scope
                            .graph
                            .transaction_external_aggregate_children
                            .get(&parent)
                            .map(|children| children
                                .iter()
                                .map(|child| (child.tala_id, child.position))
                                .collect::<Vec<_>>())
                    )),
                );
            }
            if !singleton_scope {
                crate::internal::pipeline::trace_placement_state(
                    "initialized",
                    &sub_scope.trace_root,
                    &sub_scope.graph,
                );
            }
            if crate::internal::trace_env_enabled("WEFTAN_TRACE_GRID_ROOT_PHASES")
                && sub_scope
                    .graph
                    .transaction_external_container_children
                    .contains_key(&233_611_931)
            {
                eprintln!(
                    "GRID_ROOT_PHASE_RUST phase=after-reconcile {:?}",
                    sub_scope
                        .graph
                        .transaction_external_container_children
                        .get(&233_611_931)
                        .map(|children| children
                            .iter()
                            .filter_map(|child| {
                                matches!(
                                    child.tala_id,
                                    498_183_754
                                        | 1_149_337_423
                                        | 3_584_521_799
                                        | 1_782_109_120
                                        | 2_919_616_609
                                )
                                .then_some((
                                    child.tala_id,
                                    child.position,
                                    child.rect.size,
                                ))
                            })
                            .collect::<Vec<_>>())
                );
            }
            if crate::internal::trace_env_enabled("WEFTAN_TRACE_GRID28_STEPS")
                && members.iter().any(|node| {
                    matches!(
                        scope.graph.nodes[node.0 as usize].tala_id,
                        498_183_754 | 1_149_337_423 | 3_584_521_799 | 1_782_109_120 | 2_919_616_609
                    )
                })
            {
                eprintln!(
                    "GRID28_STEP_RUST after_reconcile {:?}",
                    sub_scope
                        .graph
                        .transaction_external_container_children
                        .values()
                        .flatten()
                        .filter(|child| {
                            matches!(
                                child.tala_id,
                                498_183_754
                                    | 1_149_337_423
                                    | 3584521799
                                    | 1_782_109_120
                                    | 2_919_616_609
                            )
                        })
                        .map(|child| (child.tala_id, child.position, child.rect.size))
                        .collect::<Vec<_>>()
                );
            }
            if crate::internal::trace_env_enabled("WEFTAN_TRACE_ROOT_INITIAL") {
                eprintln!(
                    "ROOT_INITIAL_RUST members={:?}",
                    members
                        .iter()
                        .map(|node| (
                            scope.graph.nodes[node.0 as usize].tala_id,
                            sub_scope.graph.position(NodeId(
                                old_by_new.iter().position(|old| old == node).unwrap() as u32
                            ))
                        ))
                        .collect::<Vec<_>>()
                );
                eprintln!(
                    "ROOT_INITIAL_RUST external={:?}",
                    sub_scope
                        .graph
                        .transaction_external_container_children
                        .iter()
                        .map(|(parent, children)| (
                            *parent,
                            children
                                .iter()
                                .map(|child| (child.tala_id, child.position))
                                .collect::<Vec<_>>()
                        ))
                        .collect::<Vec<_>>()
                );
                eprintln!(
                    "ROOT_INITIAL_RUST nodes={:?}",
                    sub_scope
                        .graph
                        .nodes
                        .iter()
                        .map(|node| (
                            node.tala_id,
                            node.is_container,
                            node.position,
                            node.rect.size
                        ))
                        .collect::<Vec<_>>()
                );
            }
            let node_count = sub_scope.graph.nodes.len();
            let iterations = (90.0 * (node_count as f64).sqrt()) as usize;
            let pass_count = iterations.saturating_sub(iterations / 2 + 1);
            #[cfg(feature = "diagnostic-traces")]
            let mut trace_last_published_graph = sub_scope.graph.clone();
            #[cfg(feature = "diagnostic-traces")]
            let mut trace_orthogonal_phase = |phase: &str, placed: &ArenaGraph| {
                if placement.root_external_id.is_empty() {
                    return;
                }
                let changed = Self::publish_diagnostic_shared_geometry_since(
                    &mut trace_owner_graph,
                    placed,
                    &trace_last_published_graph,
                );
                crate::internal::pipeline::trace_remember_owner_geometry_changes(
                    &trace_owner_graph,
                    &changed,
                );
                trace_last_published_graph.clone_from(placed);
                crate::internal::pipeline::trace_placement_state_with_cell_size(
                    phase,
                    &placement.root_external_id,
                    &trace_owner_graph,
                    placed.cell_size,
                );
            };
            #[cfg(feature = "diagnostic-traces")]
            let trace_orthogonal_observer =
                crate::internal::trace_env_enabled("WEFTAN_TRACE_PLACEMENT")
                    .then_some(&mut trace_orthogonal_phase as &mut dyn FnMut(&str, &ArenaGraph));
            sub_scope.run_split_subgraph_sized_pass_with_owner(
                pass_count,
                Some(edge_abduction_nodes),
                #[cfg(feature = "diagnostic-traces")]
                trace_orthogonal_observer,
                owner_session.as_deref_mut(),
            )?;
            #[cfg(feature = "diagnostic-traces")]
            trace_nesting_witness(
                "component-optimized",
                &sub_scope.graph,
                Some(subgraph_index),
            );
            #[cfg(feature = "diagnostic-traces")]
            drop(trace_orthogonal_phase);
            if crate::internal::trace_env_enabled("WEFTAN_TRACE_SINGLETON_PATH") {
                eprintln!(
                    "SINGLETON_SCOPE_RUST phase=after-optimizer nodes={:?}",
                    sub_scope
                        .graph
                        .nodes
                        .iter()
                        .map(|node| (node.tala_id, node.position, node.fixed_top_left))
                        .collect::<Vec<_>>(),
                );
            }
            if crate::internal::trace_env_enabled("WEFTAN_TRACE_ROOT5")
                && placement
                    .scoring_node_metadata
                    .first()
                    .and_then(|metadata| metadata.parent_tala_id)
                    == Some(5)
            {
                eprintln!(
                    "ROOT5_RUST after-opt active={:?} external={:?} children={:?}",
                    sub_scope
                        .graph
                        .nodes
                        .iter()
                        .find(|node| node.tala_id == 5)
                        .map(|node| (node.position, node.rect.size)),
                    sub_scope
                        .graph
                        .transaction_external_containers
                        .iter()
                        .find(|node| node.tala_id == 5)
                        .map(|node| (node.position, node.rect.size)),
                    sub_scope
                        .graph
                        .transaction_external_container_children
                        .get(&5)
                        .map(|children| children
                            .iter()
                            .map(|node| (node.tala_id, node.position))
                            .collect::<Vec<_>>())
                );
            }
            if crate::internal::trace_env_value("WEFTAN_TRACE_FLAT_SCOPE_DIRECT").is_some_and(
                |target| {
                    target == "all"
                        || placement
                            .scoring_node_metadata
                            .first()
                            .and_then(|metadata| metadata.parent_tala_id)
                            .is_some_and(|root| root == target.parse::<i64>().unwrap_or_default())
                },
            ) {
                eprintln!(
                    "FLAT_SCOPE_DIRECT_RUST root={:?} external={:?} nodes={:?}",
                    placement
                        .scoring_node_metadata
                        .first()
                        .and_then(|metadata| metadata.parent_tala_id),
                    sub_scope
                        .graph
                        .transaction_external_container_children
                        .get(&4254926550)
                        .map(|children| {
                            children
                                .iter()
                                .map(|child| (child.tala_id, child.position, child.rect.size))
                                .collect::<Vec<_>>()
                        }),
                    sub_scope
                        .graph
                        .nodes
                        .iter()
                        .filter(|node| matches!(node.tala_id, 4254926550 | 1369972579 | 3471367806))
                        .map(|node| (node.tala_id, node.position, node.rect.size))
                        .collect::<Vec<_>>()
                );
            }
            if crate::internal::trace_env_enabled("WEFTAN_TRACE_ROOT_OPT_EXTERNAL")
                && placement
                    .scoring_node_metadata
                    .first()
                    .and_then(|metadata| metadata.parent_tala_id)
                    .is_none()
            {
                eprintln!(
                    "ROOT_OPT_EXTERNAL_RUST members={:?} children={:?}",
                    members
                        .iter()
                        .map(|node| scope.graph.nodes[node.0 as usize].tala_id)
                        .collect::<Vec<_>>(),
                    sub_scope
                        .graph
                        .transaction_external_container_children
                        .iter()
                        .filter(|(parent, _)| **parent == 2043751575)
                        .map(|(parent, children)| (
                            *parent,
                            children
                                .iter()
                                .map(|child| (child.tala_id, child.position))
                                .collect::<Vec<_>>(),
                        ))
                        .collect::<Vec<_>>()
                );
            }
            if crate::internal::trace_env_enabled("WEFTAN_TRACE_SCOPE_POS")
                && placement
                    .scoring_node_metadata
                    .first()
                    .and_then(|metadata| metadata.parent_tala_id)
                    == Some(2_367_942_975)
            {
                eprintln!(
                    "SCOPE_POS_RUST phase=after-opt {:?}",
                    sub_scope
                        .graph
                        .nodes
                        .iter()
                        .map(|node| (node.tala_id, node.position, node.rect.size))
                        .collect::<Vec<_>>()
                );
            }
            if crate::internal::trace_env_enabled("WEFTAN_TRACE_GRID_ROOT_PHASES")
                && sub_scope
                    .graph
                    .transaction_external_container_children
                    .contains_key(&233_611_931)
            {
                eprintln!(
                    "GRID_ROOT_PHASE_RUST phase=after-opt {:?}",
                    sub_scope
                        .graph
                        .transaction_external_container_children
                        .get(&233_611_931)
                        .map(|children| children
                            .iter()
                            .filter_map(|child| {
                                matches!(
                                    child.tala_id,
                                    498_183_754
                                        | 1_149_337_423
                                        | 3_584_521_799
                                        | 1_782_109_120
                                        | 2_919_616_609
                                )
                                .then_some((
                                    child.tala_id,
                                    child.position,
                                    child.rect.size,
                                ))
                            })
                            .collect::<Vec<_>>())
                );
            }
            if crate::internal::trace_env_enabled("WEFTAN_TRACE_GRID28_STEPS")
                && members.iter().any(|node| {
                    matches!(
                        scope.graph.nodes[node.0 as usize].tala_id,
                        498_183_754 | 1_149_337_423 | 3_584_521_799 | 1_782_109_120 | 2_919_616_609
                    )
                })
            {
                eprintln!(
                    "GRID28_STEP_RUST after_opt {:?}",
                    sub_scope
                        .graph
                        .transaction_external_container_children
                        .values()
                        .flatten()
                        .filter(|child| {
                            matches!(
                                child.tala_id,
                                498_183_754
                                    | 1_149_337_423
                                    | 3584521799
                                    | 1_782_109_120
                                    | 2_919_616_609
                            )
                        })
                        .map(|child| (child.tala_id, child.position, child.rect.size))
                        .collect::<Vec<_>>()
                );
            }
            if crate::internal::trace_env_enabled("WEFTAN_TRACE_FLAT_SCOPE")
                && old_by_new.iter().any(|old| {
                    matches!(
                        scope.graph.nodes[old.0 as usize].tala_id,
                        727019916 | 2317112547 | 1098606932
                    )
                })
            {
                eprintln!(
                    "FLAT_SCOPE_OPT_RUST members={:?}",
                    old_by_new
                        .iter()
                        .map(|old| (
                            scope.graph.nodes[old.0 as usize].tala_id,
                            sub_scope.graph.nodes[old_by_new
                                .iter()
                                .position(|candidate| candidate == old)
                                .unwrap()]
                            .position,
                            sub_scope.graph.nodes[old_by_new
                                .iter()
                                .position(|candidate| candidate == old)
                                .unwrap()]
                            .rect
                            .size,
                            sub_scope.graph.nodes[old_by_new
                                .iter()
                                .position(|candidate| candidate == old)
                                .unwrap()]
                            .scoring_is_aggregate_vessel,
                            sub_scope.graph.nodes[old_by_new
                                .iter()
                                .position(|candidate| candidate == old)
                                .unwrap()]
                            .grid_rows,
                            sub_scope.graph.nodes[old_by_new
                                .iter()
                                .position(|candidate| candidate == old)
                                .unwrap()]
                            .grid_columns,
                        ))
                        .collect::<Vec<_>>()
                );
            }
            next_rng_float = sub_scope.next_rng_float;
            #[cfg(feature = "diagnostic-traces")]
            {
                let changed = Self::publish_diagnostic_shared_geometry_since(
                    &mut trace_owner_graph,
                    &sub_scope.graph,
                    &trace_last_published_graph,
                );
                trace_last_published_graph.clone_from(&sub_scope.graph);
                crate::internal::pipeline::trace_placement_state_with_owner_changes(
                    &format!("after-orthogonal-{subgraph_index}"),
                    &placement.root_external_id,
                    &trace_owner_graph,
                    &changed,
                );
            }
            if crate::internal::trace_env_enabled("WEFTAN_TRACE_PLACEMENT_SCOPES") {
                eprint!(
                    "PLACEMENT_SCOPE_RUST optimizer root={:?}",
                    placement
                        .scoring_node_metadata
                        .first()
                        .and_then(|m| m.parent_tala_id)
                );
                for node in &sub_scope.graph.nodes {
                    if let Some(position) = node.position {
                        eprint!(
                            " {}={},{}:{}x{}",
                            node.tala_id,
                            position.x,
                            position.y,
                            node.rect.size.width,
                            node.rect.size.height
                        );
                    }
                }
                eprintln!();
            }
            // placeNodes optimizes each component's cluster vessels before
            // combining its shared Node pointers into the parent. Publish
            // retained member/container aliases only after that final flip;
            // copying them first leaves the old arrangement in the owner.
            for projection in &placement.projected_clusters {
                let Some(new_index) = old_by_new.iter().position(|old| *old == projection.node)
                else {
                    continue;
                };
                let projected_node = NodeId(new_index as u32);
                let desired = sub_scope.graph.projected_cluster_desired_arrangement(
                    projected_node,
                    projection.arrangement,
                    projection.desired_arrangement,
                );
                cluster_desired_arrangements.insert(projection.cluster_index, desired);
                let optimized = sub_scope
                    .graph
                    .optimize_projected_cluster_flip(projected_node, projection);
                if crate::internal::trace_env_enabled("WEFTAN_TRACE_CLUSTER_STATE") {
                    eprintln!(
                        "CLUSTER_STATE_RUST scope={:?} vessel={} index={} projection={:?} desired={:?} optimized={:?} external={:?}",
                        placement
                            .scoring_node_metadata
                            .first()
                            .and_then(|m| m.parent_tala_id),
                        scope.graph.nodes[projection.node.0 as usize].tala_id,
                        projection.cluster_index,
                        projection.arrangement,
                        desired,
                        optimized,
                        sub_scope
                            .graph
                            .transaction_external_cluster_layouts
                            .get(&scope.graph.nodes[projection.node.0 as usize].tala_id)
                            .map(|layout| layout.arrangement),
                    );
                }
                if let Some(arrangement) = optimized {
                    cluster_arrangements.insert(projection.cluster_index, arrangement);
                }
            }
            #[cfg(feature = "diagnostic-traces")]
            {
                let changed = Self::publish_diagnostic_shared_geometry_since(
                    &mut trace_owner_graph,
                    &sub_scope.graph,
                    &trace_last_published_graph,
                );
                trace_last_published_graph.clone_from(&sub_scope.graph);
                crate::internal::pipeline::trace_placement_state_with_owner_changes(
                    &format!("after-cluster-optimize-{subgraph_index}"),
                    &placement.root_external_id,
                    &trace_owner_graph,
                    &changed,
                );
            }
            Self::publish_induced_external_geometry(&mut scope.graph, &sub_scope.graph);
            #[cfg(feature = "diagnostic-traces")]
            trace_nesting_witness("component-published", &scope.graph, Some(subgraph_index));
            // Go's SplitSubgraphs retains the same *Node pointers in the
            // temporary component and the owning graph.  The Rust induced
            // graph stores visible members as value snapshots, so publish
            // their local placement back after external carriers have been
            // synchronized.  This ordering matters for a nested container:
            // its child projection is already in the component-local frame
            // while the owner arena's external carrier may still hold the
            // parent's outer coordinate.
            for (new_index, old_node) in old_by_new.iter().copied().enumerate() {
                let Some(position) = sub_scope.graph.nodes[new_index].position else {
                    continue;
                };
                let is_aggregate_member = scope.graph.active_node_is_aggregate(old_node);
                #[cfg(feature = "diagnostic-traces")]
                if crate::internal::trace_env_value("WEFTAN_TRACE_COPYBACK_MEMBER").is_some_and(
                    |target| {
                        target == "all"
                            || target.parse::<i64>().ok().is_some_and(|target_id| {
                                target_id == scope.graph.nodes[old_node.0 as usize].tala_id
                                    || target_id == scope.graph.active_node_tala_id(old_node)
                            })
                    },
                ) {
                    eprintln!(
                        "COPYBACK_MEMBER_RUST node={} active_id={} aggregate_member={} before={:?} placed={:?}",
                        scope.graph.nodes[old_node.0 as usize].tala_id,
                        scope.graph.active_node_tala_id(old_node),
                        is_aggregate_member,
                        scope.graph.nodes[old_node.0 as usize].rect.size,
                        sub_scope.graph.nodes[new_index].rect.size,
                    );
                }
                let owner_node = &mut scope.graph.nodes[old_node.0 as usize];
                if old_by_new.len() == 1
                    && sub_scope
                        .graph
                        .transaction_external_container_children
                        .contains_key(&owner_node.tala_id)
                {
                    local_member_positions.push((
                        old_node,
                        position,
                        sub_scope.graph.nodes[new_index].rect.size,
                    ));
                }
                owner_node.position = Some(position);
                owner_node.rect.origin = position;
                // The induced graph projects every aggregate member ID
                // through the temporary vessel. Its dimensions therefore
                // describe the vessel at each member index, while the owner
                // graph retains the individual member boxes.
                if !is_aggregate_member {
                    owner_node.rect.size = sub_scope.graph.nodes[new_index].rect.size;
                }
            }
            if crate::internal::trace_env_enabled("WEFTAN_TRACE_ROOT5")
                && placement
                    .scoring_node_metadata
                    .first()
                    .and_then(|metadata| metadata.parent_tala_id)
                    == Some(5)
            {
                eprintln!(
                    "ROOT5_RUST after-publish active={:?} external={:?} children={:?}",
                    scope
                        .graph
                        .nodes
                        .iter()
                        .find(|node| node.tala_id == 5)
                        .map(|node| (node.position, node.rect.size)),
                    scope
                        .graph
                        .transaction_external_containers
                        .iter()
                        .find(|node| node.tala_id == 5)
                        .map(|node| (node.position, node.rect.size)),
                    scope
                        .graph
                        .transaction_external_container_children
                        .get(&5)
                        .map(|children| children
                            .iter()
                            .map(|node| (node.tala_id, node.position))
                            .collect::<Vec<_>>())
                );
            }
            if crate::internal::trace_env_value("WEFTAN_TRACE_FLAT_SCOPE_PHASES").is_some_and(
                |target| {
                    target == "all"
                        || placement
                            .scoring_node_metadata
                            .first()
                            .and_then(|metadata| metadata.parent_tala_id)
                            .is_some_and(|root| root == target.parse::<i64>().unwrap_or_default())
                },
            ) {
                eprintln!(
                    "FLAT_SCOPE_PHASE_RUST phase=after-optimizer-publish root={:?} map={:?} nodes={:?}",
                    placement
                        .scoring_node_metadata
                        .first()
                        .and_then(|metadata| metadata.parent_tala_id),
                    scope
                        .graph
                        .transaction_external_container_children
                        .get(&4254926550)
                        .map(|children| {
                            children
                                .iter()
                                .map(|child| (child.tala_id, child.position))
                                .collect::<Vec<_>>()
                        }),
                    scope
                        .graph
                        .nodes
                        .iter()
                        .filter(|node| matches!(node.tala_id, 4254926550 | 1369972579 | 3471367806))
                        .map(|node| (node.tala_id, node.position))
                        .collect::<Vec<_>>()
                );
            }
            if crate::internal::trace_env_enabled("WEFTAN_TRACE_COPYBACK")
                && placement
                    .scoring_node_metadata
                    .first()
                    .and_then(|metadata| metadata.parent_tala_id)
                    .is_none()
            {
                eprintln!(
                    "COPYBACK_PHASE_RUST after_optimizer owner={:?} stable={:?}",
                    scope
                        .graph
                        .transaction_external_container_children
                        .iter()
                        .filter(|(parent, _)| **parent == 3248328362 || **parent == 3680227035)
                        .map(|(parent, children)| (
                            *parent,
                            children
                                .iter()
                                .map(|child| (child.tala_id, child.position))
                                .collect::<Vec<_>>()
                        ))
                        .collect::<Vec<_>>(),
                    scope
                        .graph
                        .nodes
                        .iter()
                        .filter(|node| matches!(node.tala_id, 3248328362 | 3680227035 | 1832207848))
                        .map(|node| (node.tala_id, node.position))
                        .collect::<Vec<_>>()
                );
            }
            if crate::internal::trace_env_enabled("WEFTAN_TRACE_GRID_CARRIERS") {
                let trace_target = |graph: &ArenaGraph| {
                    graph
                        .transaction_external_containers
                        .iter()
                        .filter(|node| {
                            node.tala_id == 1_149_337_423 || node.tala_id == 1_782_109_120
                        })
                        .map(|node| (node.tala_id, node.position, node.rect.size))
                        .collect::<Vec<_>>()
                };
                eprintln!(
                    "GRID_SCOPE_COPYBACK_RUST phase=after-external parent={:?} target={:?} sub={:?}",
                    trace_target(&scope.graph),
                    trace_target(&sub_scope.graph),
                    scope
                        .graph
                        .transaction_external_container_children
                        .iter()
                        .flat_map(
                            |(parent, children)| children.iter().filter_map(move |child| {
                                (child.tala_id == 1_149_337_423 || child.tala_id == 1_782_109_120)
                                    .then_some((
                                        *parent,
                                        child.tala_id,
                                        child.position,
                                        child.rect.size,
                                    ))
                            })
                        )
                        .collect::<Vec<_>>()
                );
                let trace_children = |graph: &ArenaGraph, parent: i64| {
                    graph
                        .transaction_external_container_children
                        .get(&parent)
                        .map(|children| {
                            children
                                .iter()
                                .filter(|child| child.position.is_some())
                                .map(|child| (child.tala_id, child.position))
                                .collect::<Vec<_>>()
                        })
                };
                eprintln!(
                    "GRID_SCOPE_COPYBACK_RUST children=after-external parent_b={:?} parent_d={:?} sub_b={:?} sub_d={:?} sub_nodes={:?} owner_nodes={:?}",
                    trace_children(&scope.graph, 1_149_337_423),
                    trace_children(&scope.graph, 1_782_109_120),
                    trace_children(&sub_scope.graph, 1_149_337_423),
                    trace_children(&sub_scope.graph, 1_782_109_120),
                    sub_scope
                        .graph
                        .nodes
                        .iter()
                        .filter(|node| matches!(
                            node.tala_id,
                            1_402_186_134
                                | 1_385_408_515
                                | 1_368_630_896
                                | 1_486_074_229
                                | 1_469_296_610
                                | 1_452_519_991
                                | 1_435_741_372
                                | 1_553_184_705
                                | 1_536_407_086
                                | 1_519_629_467
                        ))
                        .map(|node| (node.tala_id, node.position))
                        .collect::<Vec<_>>(),
                    scope
                        .graph
                        .nodes
                        .iter()
                        .filter(|node| matches!(
                            node.tala_id,
                            1_402_186_134
                                | 1_385_408_515
                                | 1_368_630_896
                                | 1_486_074_229
                                | 1_469_296_610
                                | 1_452_519_991
                                | 1_435_741_372
                                | 1_553_184_705
                                | 1_536_407_086
                                | 1_519_629_467
                        ))
                        .map(|node| (node.tala_id, node.position))
                        .collect::<Vec<_>>(),
                );
            }
            for (&vessel_tala_id, placed_children) in
                &sub_scope.graph.transaction_external_aggregate_children
            {
                let Some(owner_children) = scope
                    .graph
                    .transaction_external_aggregate_children
                    .get_mut(&vessel_tala_id)
                else {
                    continue;
                };
                for placed_child in placed_children {
                    let Some(owner_index) = owner_children
                        .iter()
                        .position(|child| child.tala_id == placed_child.tala_id)
                    else {
                        continue;
                    };
                    let owner_child = &mut owner_children[owner_index];
                    owner_child.position = placed_child.position;
                    owner_child.rect.origin = placed_child.rect.origin;
                    owner_child.rect.size = placed_child.rect.size;
                    // Aggregate members are shared Go pointers: the same
                    // member can be reached through the cluster map and its
                    // owning container's child slice. Keep both Rust
                    // projections synchronized when the induced scope
                    // mirrors or translates that member.
                    if let Some(owner_node) = scope
                        .graph
                        .nodes
                        .iter_mut()
                        .find(|node| node.tala_id == placed_child.tala_id)
                    {
                        owner_node.position = placed_child.position;
                        owner_node.rect.origin = placed_child.rect.origin;
                        owner_node.rect.size = placed_child.rect.size;
                    }
                    for children in scope
                        .graph
                        .transaction_external_container_children
                        .values_mut()
                    {
                        if let Some(owner_child) = children
                            .iter_mut()
                            .find(|child| child.tala_id == placed_child.tala_id)
                        {
                            owner_child.position = placed_child.position;
                            owner_child.rect.origin = placed_child.rect.origin;
                            owner_child.rect.size = placed_child.rect.size;
                        }
                    }
                    if crate::internal::trace_env_enabled("WEFTAN_TRACE_MOVE_PROJECTION") {
                        eprintln!(
                            "MOVE_AGG_COPYBACK_RUST vessel={} member={} pos={:?} stable={:?}",
                            vessel_tala_id,
                            placed_child.tala_id,
                            placed_child.position,
                            scope
                                .graph
                                .nodes
                                .iter()
                                .find(|node| node.tala_id == placed_child.tala_id)
                                .and_then(|node| node.position)
                        );
                        if placed_child.tala_id == 3946625030 {
                            eprintln!(
                                "MOVE_AGG_COPYBACK_MATCHES_RUST {:?}",
                                scope
                                    .graph
                                    .nodes
                                    .iter()
                                    .filter(|node| node.tala_id == placed_child.tala_id)
                                    .map(|node| (node.input_id, node.position, node.rect.size))
                                    .collect::<Vec<_>>()
                            );
                        }
                    }
                }
            }
            // SplitSubgraphs retains the same *Node pointers for ordinary
            // container children as it does for aggregate members. The Rust
            // induced graph stores those pointers as cloned snapshots, so
            // copy the post-SyncNested child boxes back into the enclosing
            // projection before the next scope is built. Without this merge,
            // a child nested below a hidden cluster member is reset to the
            // pre-sync (0,0) frame while the Go pointer remains translated.
            for (&parent_tala_id, placed_children) in
                &sub_scope.graph.transaction_external_container_children
            {
                for placed_child in placed_children {
                    let Some(owner_index) = scope
                        .graph
                        .transaction_external_container_children
                        .get(&parent_tala_id)
                        .and_then(|owner_children| {
                            owner_children
                                .iter()
                                .position(|child| child.tala_id == placed_child.tala_id)
                        })
                    else {
                        continue;
                    };
                    {
                        let owner_child = &mut scope
                            .graph
                            .transaction_external_container_children
                            .get_mut(&parent_tala_id)
                            .expect("owner child map disappeared")[owner_index];
                        owner_child.position = placed_child.position;
                        owner_child.rect.origin = placed_child.rect.origin;
                        owner_child.rect.size = placed_child.rect.size;
                    }
                    if let Some(owner_node) = scope
                        .graph
                        .nodes
                        .iter_mut()
                        .find(|node| node.tala_id == placed_child.tala_id)
                    {
                        owner_node.position = placed_child.position;
                        owner_node.rect.origin = placed_child.rect.origin;
                        owner_node.rect.size = placed_child.rect.size;
                    }
                    for children in scope
                        .graph
                        .transaction_external_container_children
                        .values_mut()
                    {
                        if let Some(alias) = children
                            .iter_mut()
                            .find(|child| child.tala_id == placed_child.tala_id)
                        {
                            alias.position = placed_child.position;
                            alias.rect.origin = placed_child.rect.origin;
                            alias.rect.size = placed_child.rect.size;
                        }
                    }
                }
            }
            if crate::internal::trace_env_enabled("WEFTAN_TRACE_GRID28_STEPS")
                && scope
                    .graph
                    .transaction_external_container_children
                    .contains_key(&233_611_931)
            {
                eprintln!(
                    "GRID28_STEP_RUST owner_after_copyback {:?}",
                    scope
                        .graph
                        .transaction_external_container_children
                        .get(&233_611_931)
                        .map(|children| {
                            children
                                .iter()
                                .filter(|child| {
                                    matches!(
                                        child.tala_id,
                                        498_183_754
                                            | 1_149_337_423
                                            | 3_584_521_799
                                            | 1_782_109_120
                                            | 2_919_616_609
                                    )
                                })
                                .map(|child| (child.tala_id, child.position, child.rect.size))
                                .collect::<Vec<_>>()
                        })
                );
            }
            if crate::internal::trace_env_enabled("WEFTAN_TRACE_MOVE_PROJECTION") {
                eprintln!(
                    "MOVE_PROJECTION_SCOPE_GRAPH_RUST after_subscope={:?}",
                    scope
                        .graph
                        .transaction_external_container_children
                        .keys()
                        .collect::<Vec<_>>()
                );
            }
            for (new_index, old) in old_by_new.iter().copied().enumerate() {
                let placed = &sub_scope.graph.nodes[new_index];
                let is_aggregate_member = scope.graph.active_node_is_aggregate(old);
                scope.graph.nodes[old.0 as usize].position = placed.position;
                // The active projection carries the synthetic vessel size
                // through every member mapping. Preserve member geometry;
                // cluster_vessel_sizes carries the separate vessel box.
                if !is_aggregate_member {
                    scope.graph.nodes[old.0 as usize].rect.size = placed.rect.size;
                }
                scope.graph.nodes[old.0 as usize].herd_assignment = placed.herd_assignment.clone();
                scope.graph.nodes[old.0 as usize].scoring_cluster_arrangement =
                    placed.scoring_cluster_arrangement;
                scope.graph.nodes[old.0 as usize].scoring_is_aggregate_vessel =
                    placed.scoring_is_aggregate_vessel;
            }
            // Go's layout graph stores one *Node pointer in Graph.Nodes and
            // every Containers child slice.  The Rust arena keeps those
            // carriers as value snapshots, so a component copyback must
            // refresh every ordinary alias from the authoritative stable
            // node before the next enclosing scope opens its mirror pass.
            // This preserves the mixed-frame state that mirrorAxes observes
            // after nested components have already been placed.
            // Nested scopes need their aliases refreshed from the stable arena
            // before the next enclosing scope opens.  The root scope is
            // different: its temporary graph has just selected the packed
            // offsets for direct children (including recursively fitted
            // containers), and replacing those aliases with the pre-root
            // stable positions would discard the root placement result.
            let is_root_scope = placement
                .scoring_node_metadata
                .first()
                .and_then(|metadata| metadata.parent_tala_id)
                .is_none();
            if !is_root_scope {
                let stable_geometry = scope
                    .graph
                    .nodes
                    .iter()
                    .map(|node| {
                        (
                            node.tala_id,
                            (node.position, node.rect.origin, node.rect.size),
                        )
                    })
                    .collect::<BTreeMap<_, _>>();
                for children in scope
                    .graph
                    .transaction_external_container_children
                    .values_mut()
                {
                    for child in children {
                        let Some((position, origin, size)) = stable_geometry.get(&child.tala_id)
                        else {
                            continue;
                        };
                        child.position = *position;
                        child.rect.origin = *origin;
                        child.rect.size = *size;
                    }
                }
                for child in &mut scope.graph.transaction_external_containers {
                    if let Some((position, origin, size)) = stable_geometry.get(&child.tala_id) {
                        child.position = *position;
                        child.rect.origin = *origin;
                        child.rect.size = *size;
                    }
                }
            }
            if crate::internal::trace_env_enabled("WEFTAN_TRACE_DIRECT_204")
                && placement
                    .scoring_node_metadata
                    .first()
                    .and_then(|metadata| metadata.parent_tala_id)
                    .is_none()
            {
                let tracked = [
                    2043751575_i64,
                    3677471320,
                    2884276447,
                    1010209959,
                    1231386872,
                    2204119069,
                    4137376658,
                    1323527919,
                    2801930458,
                    696109821,
                    3554470064,
                    2202785448,
                    6129484611666145821,
                ];
                let dump = |graph: &ArenaGraph| {
                    graph
                        .transaction_external_container_children
                        .iter()
                        .filter(|(parent, _)| tracked.contains(parent))
                        .map(|(parent, children)| {
                            (
                                *parent,
                                children
                                    .iter()
                                    .filter(|child| tracked.contains(&child.tala_id))
                                    .map(|child| (child.tala_id, child.position))
                                    .collect::<Vec<_>>(),
                            )
                        })
                        .collect::<Vec<_>>()
                };
                eprintln!("DIRECT_204_RUST_AFTER_STABLE {:?}", dump(&scope.graph));
            }
            if crate::internal::trace_env_enabled("WEFTAN_TRACE_GRID_CARRIERS") {
                let target = scope
                    .graph
                    .nodes
                    .iter()
                    .filter(|node| node.tala_id == 1_149_337_423 || node.tala_id == 1_782_109_120)
                    .map(|node| (node.tala_id, node.position, node.rect.size))
                    .collect::<Vec<_>>();
                eprintln!(
                    "GRID_SCOPE_COPYBACK_RUST phase=after-active parent_nodes={:?}",
                    target
                );
                let trace_children = |graph: &ArenaGraph, parent: i64| {
                    graph
                        .transaction_external_container_children
                        .get(&parent)
                        .map(|children| {
                            children
                                .iter()
                                .filter(|child| child.position.is_some())
                                .map(|child| (child.tala_id, child.position))
                                .collect::<Vec<_>>()
                        })
                };
                eprintln!(
                    "GRID_SCOPE_COPYBACK_RUST children=after-active parent_b={:?} parent_d={:?}",
                    trace_children(&scope.graph, 1_149_337_423),
                    trace_children(&scope.graph, 1_782_109_120),
                );
            }
            for projection in &placement.projected_clusters {
                if !old_by_new.contains(&projection.node) {
                    continue;
                }
                let Some(&arrangement) = cluster_arrangements.get(&projection.cluster_index) else {
                    continue;
                };
                let layout = match arrangement {
                    ClusterArrangement::Row => &projection.row,
                    ClusterArrangement::Column => &projection.column,
                };
                scope
                    .graph
                    .apply_projected_cluster_layout(projection.node, arrangement, layout);
                if crate::internal::trace_env_enabled("WEFTAN_TRACE_PROJECTED_CLUSTERS") {
                    eprintln!(
                        "APPLY_PROJECTED_CLUSTER_RUST vessel={} arrangement={:?} size={:?}",
                        scope.graph.nodes[projection.node.0 as usize].tala_id,
                        arrangement,
                        scope.graph.nodes[projection.node.0 as usize].rect.size,
                    );
                }
            }
            // The next SplitSubgraphs component opens its transaction on the
            // same owning Graph. Publish this component's shared-pointer
            // geometry into the carrier fallback before constructing it.
            if crate::internal::trace_env_value("WEFTAN_TRACE_FLAT_SCOPE_PHASES").is_some_and(
                |target| {
                    target == "all"
                        || placement
                            .scoring_node_metadata
                            .first()
                            .and_then(|metadata| metadata.parent_tala_id)
                            .is_some_and(|root| root == target.parse::<i64>().unwrap_or_default())
                },
            ) {
                eprintln!(
                    "FLAT_SCOPE_PHASE_RUST phase=before-refresh root={:?} map={:?} nodes={:?}",
                    placement
                        .scoring_node_metadata
                        .first()
                        .and_then(|metadata| metadata.parent_tala_id),
                    scope
                        .graph
                        .transaction_external_container_children
                        .get(&4254926550)
                        .map(|children| {
                            children
                                .iter()
                                .map(|child| (child.tala_id, child.position))
                                .collect::<Vec<_>>()
                        }),
                    scope
                        .graph
                        .nodes
                        .iter()
                        .filter(|node| matches!(node.tala_id, 4254926550 | 1369972579 | 3471367806))
                        .map(|node| (node.tala_id, node.position))
                        .collect::<Vec<_>>()
                );
            }
            scope.graph.refresh_projected_transaction_node_geometry();
            if crate::internal::trace_env_enabled("WEFTAN_TRACE_CELL_SIZE") {
                eprintln!(
                    "CELL_RUST phase=after-refresh root={:?} cell={} nodes={} sizes={:?}",
                    placement
                        .scoring_node_metadata
                        .first()
                        .and_then(|metadata| metadata.parent_tala_id),
                    scope.graph.cell_size,
                    scope.graph.nodes.len(),
                    scope
                        .graph
                        .nodes
                        .iter()
                        .map(|node| (node.tala_id, node.rect.size))
                        .collect::<Vec<_>>()
                );
            }
            if crate::internal::trace_env_value("WEFTAN_TRACE_FLAT_SCOPE_PHASES").is_some_and(
                |target| {
                    target == "all"
                        || placement
                            .scoring_node_metadata
                            .first()
                            .and_then(|metadata| metadata.parent_tala_id)
                            .is_some_and(|root| root == target.parse::<i64>().unwrap_or_default())
                },
            ) {
                eprintln!(
                    "FLAT_SCOPE_PHASE_RUST phase=after-refresh root={:?} map={:?} nodes={:?}",
                    placement
                        .scoring_node_metadata
                        .first()
                        .and_then(|metadata| metadata.parent_tala_id),
                    scope
                        .graph
                        .transaction_external_container_children
                        .get(&4254926550)
                        .map(|children| {
                            children
                                .iter()
                                .map(|child| (child.tala_id, child.position))
                                .collect::<Vec<_>>()
                        }),
                    scope
                        .graph
                        .nodes
                        .iter()
                        .filter(|node| matches!(node.tala_id, 4254926550 | 1369972579 | 3471367806))
                        .map(|node| (node.tala_id, node.position))
                        .collect::<Vec<_>>()
                );
            }
            // TALA reconnects and places retained branching trees after the
            // ordinary optimizer and before Graph.direct sees the restored
            // tree edges.
            if crate::internal::trace_env_enabled("WEFTAN_TRACE_SCOPE_STEPS") {
                eprintln!(
                    "SCOPE_STEP_RUST before_place_trees root={:?}",
                    placement
                        .scoring_node_metadata
                        .first()
                        .and_then(|metadata| metadata.parent_tala_id)
                );
            }
            #[cfg(feature = "diagnostic-traces")]
            let trace_before_place_trees = scope.graph.clone();
            scope.graph.place_trees(&members);
            #[cfg(feature = "diagnostic-traces")]
            {
                Self::publish_diagnostic_shared_geometry_since(
                    &mut trace_owner_graph,
                    &scope.graph,
                    &trace_before_place_trees,
                );
                // Go's component graph shares the current root's live child
                // slice with the owner. Rust stores that slice under None in
                // the temporary component, so project only these restored
                // children back onto the owner clone for this diagnostic
                // boundary. Other sibling components remain at their current
                // placement stage.
                if let Some(scope_children) = scope.graph.containers.get(&None)
                    && let Some(owner_parent_dense_index) = trace_owner_graph
                        .nodes
                        .iter()
                        .position(|node| node.external_id == placement.root_external_id)
                    && let Some(&owner_parent_index) = trace_owner_nodes_by_tala
                        .get(&trace_owner_graph.nodes[owner_parent_dense_index].tala_id)
                {
                    let owner_children = scope_children
                        .iter()
                        .filter_map(|child| {
                            let tala_id = scope.graph.nodes[child.0 as usize].tala_id;
                            trace_owner_nodes_by_tala
                                .get(&tala_id)
                                .map(|index| NodeId(*index as u32))
                        })
                        .collect();
                    trace_owner_graph
                        .containers
                        .insert(Some(NodeId(owner_parent_index as u32)), owner_children);
                }
                crate::internal::pipeline::trace_placement_state(
                    &format!("after-trees-{subgraph_index}"),
                    &placement.root_external_id,
                    &trace_owner_graph,
                );
            }
            #[cfg(feature = "diagnostic-traces")]
            let trace_before_restore_edges = scope.graph.clone();
            scope.graph.append_restored_projected_transaction_nodes();
            if crate::internal::trace_env_enabled("WEFTAN_TRACE_CELL_SIZE") {
                eprintln!(
                    "CELL_RUST phase=after-trees root={:?} cell={} nodes={}",
                    placement
                        .scoring_node_metadata
                        .first()
                        .and_then(|metadata| metadata.parent_tala_id),
                    scope.graph.cell_size,
                    scope.graph.nodes.len()
                );
            }
            if crate::internal::trace_env_value("WEFTAN_TRACE_FLAT_SCOPE_PHASES").is_some_and(
                |target| {
                    target == "all"
                        || placement
                            .scoring_node_metadata
                            .first()
                            .and_then(|metadata| metadata.parent_tala_id)
                            .is_some_and(|root| root == target.parse::<i64>().unwrap_or_default())
                },
            ) {
                eprintln!(
                    "FLAT_SCOPE_PHASE_RUST phase=after-trees root={:?} map={:?} nodes={:?}",
                    placement
                        .scoring_node_metadata
                        .first()
                        .and_then(|metadata| metadata.parent_tala_id),
                    scope
                        .graph
                        .transaction_external_container_children
                        .get(&4254926550)
                        .map(|children| {
                            children
                                .iter()
                                .map(|child| (child.tala_id, child.position))
                                .collect::<Vec<_>>()
                        }),
                    scope
                        .graph
                        .nodes
                        .iter()
                        .filter(|node| matches!(node.tala_id, 4254926550 | 1369972579 | 3471367806))
                        .map(|node| (node.tala_id, node.position))
                        .collect::<Vec<_>>()
                );
            }
            if crate::internal::trace_env_enabled("WEFTAN_TRACE_SCOPE_STEPS") {
                eprintln!(
                    "SCOPE_STEP_RUST after_place_trees root={:?}",
                    placement
                        .scoring_node_metadata
                        .first()
                        .and_then(|metadata| metadata.parent_tala_id)
                );
            }
            // Graph.direct receives the concrete Graph.Nodes slice produced
            // by SplitSubgraphs.  The arena's container/node_order view can
            // retain the pre-split container order (notably around root
            // containers), but Go's shared pointer order is the placement
            // `members` order.  Preserve that order for ordinary members and
            // append only retained tree auxiliaries that are reached from a
            // member.
            let root_scope = placement
                .scoring_node_metadata
                .first()
                .and_then(|metadata| metadata.parent_tala_id)
                .is_none();
            let member_tala_ids = members
                .iter()
                .map(|node| scope.graph.nodes[node.0 as usize].tala_id)
                .collect::<BTreeSet<_>>();
            let aggregate_vessel_tala_ids = scope
                .graph
                .clusters
                .iter()
                .map(|cluster| cluster.vessel_tala_id)
                .chain(
                    scope
                        .graph
                        .sequences
                        .iter()
                        .map(|sequence| sequence.vessel_tala_id),
                )
                .collect::<BTreeSet<_>>();
            let has_pre_direct_hidden_external_nodes = scope
                .graph
                .transaction_external_container_children
                .values()
                .flatten()
                .any(|child| {
                    !member_tala_ids.contains(&child.tala_id)
                        && aggregate_vessel_tala_ids.contains(&child.tala_id)
                });
            if crate::internal::trace_env_enabled("WEFTAN_TRACE_HIDDEN_EXTERNAL") && root_scope {
                eprintln!(
                    "HIDDEN_EXTERNAL_RUST members={} nodes={} aggregate={:?} hidden={} ext={:?}",
                    members.len(),
                    scope.graph.nodes.len(),
                    aggregate_vessel_tala_ids,
                    has_pre_direct_hidden_external_nodes,
                    scope
                        .graph
                        .transaction_external_container_children
                        .iter()
                        .map(|(p, c)| (
                            *p,
                            c.iter()
                                .map(|n| (n.tala_id, n.position))
                                .collect::<Vec<_>>()
                        ))
                        .collect::<Vec<_>>()
                );
                eprintln!(
                    "HIDDEN_EXTERNAL_STABLE_RUST {:?}",
                    scope
                        .graph
                        .nodes
                        .iter()
                        .filter(|node| matches!(
                            node.tala_id,
                            1832207848
                                | 2280770807
                                | 298920306
                                | 3777840222
                                | 4222097421
                                | 1379919604
                        ))
                        .map(|node| (node.tala_id, node.position))
                        .collect::<Vec<_>>()
                );
                eprintln!(
                    "HIDDEN_EXTERNAL_ROOT_NODES_RUST {:?}",
                    scope
                        .graph
                        .nodes
                        .iter()
                        .map(|node| (node.tala_id, node.position, node.rect.size))
                        .collect::<Vec<_>>()
                );
            }
            let mut directed_members = if root_scope
                && (4..50).contains(&members.len())
                && scope.graph.nodes.len() > members.len()
                && has_pre_direct_hidden_external_nodes
            {
                scope
                    .graph
                    .nodes
                    .iter()
                    .map(|node| node.input_id)
                    .filter(|node| scope.graph.position(*node).is_some())
                    .collect::<Vec<_>>()
            } else {
                members.clone()
            };
            // Go's reconnectTree appends every retained tree node in sorted
            // placement-tree preorder. Iterating the stable arena order here
            // changes the temporary Graph.Nodes slice and therefore every
            // order-sensitive direct/mirroring decision.
            for node in scope.graph.placement_tree_reconnect_order(&members) {
                if directed_members.contains(&node)
                    || !scope.graph.tree_routing_nodes.contains_key(&node)
                {
                    continue;
                }
                let mut sentinel = scope.graph.tree_routing_nodes[&node].parent;
                while let Some(parent) = scope.graph.tree_routing_nodes.get(&sentinel) {
                    sentinel = parent.parent;
                }
                if members.contains(&sentinel) {
                    directed_members.push(node);
                }
            }
            // restoreEdgeAbductions reconnects concrete edge identities before
            // Graph.direct. Reconnection removes a fully restored edge from
            // both carriers, while a one-sided restoration remains reachable
            // through its current carrier. Traverse that restored graph from
            // the temporary component nodes just as mirrorAxes does.
            let reachable_tala_keys =
                restored_mirror_reachable_containers(placement, &scope.graph, &directed_members);
            let reachable_containers = reachable_tala_keys
                .iter()
                .filter_map(|key| {
                    let Some(tala_id) = key else {
                        return Some(None);
                    };
                    let local = scope
                        .graph
                        .nodes
                        .iter()
                        .find(|node| node.tala_id == *tala_id)
                        .and_then(|node| {
                            placement
                                .old_to_new
                                .iter()
                                .find_map(|(old, new)| (*new == node.input_id).then_some(*old))
                        });
                    local
                        .or_else(|| {
                            scope
                                .graph
                                .transaction_external_containers
                                .iter()
                                .chain(
                                    scope
                                        .graph
                                        .transaction_external_container_children
                                        .values()
                                        .flatten(),
                                )
                                .find(|node| node.tala_id == *tala_id)
                                .map(|node| node.input_id)
                        })
                        .map(Some)
                })
                .collect::<BTreeSet<_>>();
            if crate::internal::trace_env_enabled("WEFTAN_TRACE_SCOPE_STEPS") {
                eprintln!(
                    "SCOPE_STEP_RUST before_induced_direct root={:?} members={:?}",
                    placement
                        .scoring_node_metadata
                        .first()
                        .and_then(|metadata| metadata.parent_tala_id),
                    directed_members
                        .iter()
                        .map(|node| scope.graph.nodes[node.0 as usize].tala_id)
                        .collect::<Vec<_>>()
                );
            }
            // PlaceTrees has reconnected these retained nodes to the current
            // graph by this point. When this graph owns the restored tree
            // nodes, include them in its live Graph.Nodes ownership view
            // before constructing the induced component: that constructor
            // snapshots Node.Graph ownership into `directed`. Appending them
            // afterward leaves direct(false) seeing only the root owner and
            // can select the wrong mirror axis.
            let direct_owns_restored_trees = placement.owner_node_count == 1
                && placement.owning_graph_node_count
                    == scope
                        .graph
                        .nodes
                        .len()
                        .saturating_add(placement.tree_auxiliary_nodes.len());
            if direct_owns_restored_trees {
                // Preserve the order already produced by PlaceTrees above.
                // Iterating tree_routing_nodes (a stable arena map) silently
                // replaces reconnectTree's sorted tree preorder with dense
                // arena order in the temporary Graph.Nodes slice.
                for &tree_node in &directed_members {
                    if scope.graph.tree_routing_nodes.contains_key(&tree_node)
                        && !scope.graph.active_graph_node_order.contains(&tree_node)
                    {
                        scope.graph.active_graph_node_order.push(tree_node);
                    }
                }
            }
            let (mut directed, directed_old_by_new) =
                scope.graph.induced_placement_subgraph(&directed_members);
            if crate::internal::trace_env_enabled("WEFTAN_TRACE_DIRECT_ROOT") {
                eprintln!(
                    "DIRECT_SCOPE_META_RUST root={:?} owner_count={} owning_graph_count={} tree_aux={} source_nodes={} member_nodes={} same_graph={} source_active={:?}",
                    placement
                        .scoring_node_metadata
                        .first()
                        .and_then(|metadata| metadata.parent_tala_id),
                    placement.owner_node_count,
                    placement.owning_graph_node_count,
                    placement.tree_auxiliary_nodes.len(),
                    scope.graph.nodes.len(),
                    directed_members.len(),
                    direct_owns_restored_trees,
                    scope
                        .graph
                        .active_graph_node_order
                        .iter()
                        .map(|node| scope.graph.nodes[node.0 as usize].tala_id)
                        .collect::<Vec<_>>(),
                );
            }
            // Go keeps the split graph's pre-placement CellSize through
            // cluster flips and tree restoration. Recomputing from the new
            // dimensions here changes the state observed by direct.
            directed.cell_size = sub_scope.graph.cell_size;
            if crate::internal::trace_env_enabled("WEFTAN_TRACE_CELL_SIZE") {
                eprintln!(
                    "CELL_RUST phase=after-induced root={:?} source={} directed={} source_nodes={} directed_nodes={}",
                    placement
                        .scoring_node_metadata
                        .first()
                        .and_then(|metadata| metadata.parent_tala_id),
                    scope.graph.cell_size,
                    directed.cell_size,
                    scope.graph.nodes.len(),
                    directed.nodes.len()
                );
            }
            // `mirrorAxes` computes reachability on the owning temporary
            // graph before the induced `Graph.Nodes` slice is reduced.  Map
            // those old container identities into the induced arena so
            // projected aliases can apply the same reachable/unreachable
            // descendant rule during direct placement.
            // Container reachability crosses original and compact arenas.
            // Carry pointer-equivalent TALA identities until this final map;
            // matching numeric NodeIds across the two arenas aliases parents.
            let directed_reachable_containers = reachable_tala_keys
                .iter()
                .filter_map(|container| match container {
                    None => Some(None),
                    Some(tala_id) => directed
                        .nodes
                        .iter()
                        .position(|node| node.tala_id == *tala_id)
                        .map(|index| Some(NodeId(index as u32))),
                })
                .collect::<BTreeSet<_>>();
            let reachable_container_tala_ids = reachable_tala_keys
                .iter()
                .flatten()
                .copied()
                .collect::<BTreeSet<_>>();
            let mut directed_reachable_containers = directed_reachable_containers;
            // The Go induced graph retains each component root's Container
            // pointer even when that parent is outside Graph.Nodes.  The
            // compact Rust arena has no slot for that external pointer and
            // consequently represents these roots with `container = None`.
            // Preserve the same mirrorAxes reachability for that carrier: a
            // nil container is a real map key in TALA and is reachable for
            // every induced component whose roots have lost their parent
            // slot.
            if directed.nodes.iter().all(|node| node.container.is_none()) {
                directed_reachable_containers.insert(None);
            }
            for (index, node) in directed.nodes.iter().enumerate() {
                if reachable_container_tala_ids.contains(&node.tala_id) {
                    directed_reachable_containers.insert(Some(NodeId(index as u32)));
                }
            }
            for node in directed
                .transaction_external_containers
                .iter()
                .chain(
                    directed
                        .transaction_external_container_children
                        .values()
                        .flatten(),
                )
                .chain(
                    directed
                        .transaction_external_aggregate_children
                        .values()
                        .flatten(),
                )
            {
                if reachable_container_tala_ids.contains(&node.tala_id) {
                    // Projected children keep the owner arena's input ID in
                    // their `container` field, so retain that ID alongside
                    // the compact Graph.Nodes indices above.
                    directed_reachable_containers.insert(Some(node.input_id));
                }
            }
            directed.reachable_container_tala_ids = Some(reachable_container_tala_ids.clone());
            if crate::internal::trace_env_enabled("WEFTAN_TRACE_SCOPE_STEPS") {
                eprintln!(
                    "SCOPE_STEP_RUST after_induced_direct root={:?} nodes={}",
                    placement
                        .scoring_node_metadata
                        .first()
                        .and_then(|metadata| metadata.parent_tala_id),
                    directed.nodes.len()
                );
            }
            for (new_index, old) in directed_old_by_new.iter().copied().enumerate() {
                directed.nodes[new_index].position = scope.graph.position(old);
                directed.nodes[new_index].rect.origin =
                    directed.nodes[new_index].position.unwrap_or_default();
                // The induced graph is a fresh arena, but Graph.direct still
                // mutates the retained Tree orientation carrier. Carry just
                // that source-visible state into the induced graph; the
                // sentinel edge IDs are not consulted by direct and are
                // intentionally left local to this short-lived mirror pass.
                if let Some(tree) = scope.graph.tree_routing_nodes.get(&old) {
                    directed.tree_routing_nodes.insert(
                        NodeId(new_index as u32),
                        TreeRoutingNode {
                            parent: NodeId(0),
                            sentinel_edge: EdgeId(0),
                            orientation: tree.orientation,
                            extraction_order: tree.extraction_order,
                        },
                    );
                }
            }
            if let Some(session) = owner_session.as_deref() {
                session
                    .refresh_explicit_owner_projections(&mut directed)
                    .map_err(|error| crate::LayoutError::PlacementTransaction(error.to_string()))?;
            }
            let pre_direct_external_children =
                directed.transaction_external_container_children.clone();
            let pre_direct_external_aggregate_children =
                directed.transaction_external_aggregate_children.clone();
            let pre_direct_external_containers = directed.transaction_external_containers.clone();

            // `restoreEdgeAbductions` runs before `Graph.direct` in the Go
            // implementation.  The arena keeps the edge endpoints projected
            // onto their carriers for sized scoring, so the operation is not
            // represented by changing `ArenaEdge.from/to` here.  It is still
            // observable through the shared `Node.Edges` slices: a restored
            // edge is removed from its current carrier and appended to the
            // original vessel in abduction-slice order.  Replay that narrow
            // pointer-visible part before direct/mirroring.  The final global
            // adjacency publication remains in `sync_edge_adjacency_order`.
            //
            // Use stable TALA identities for the compact induced graph. Dense
            // NodeId/EdgeId values differ at every split, while the source
            // graph and each placement scope retain the same node/edge IDs.
            let mut replay_restored_edge =
                |current_tala_id: Option<i64>, original_tala_id: i64, edge_tala_id: i64| {
                    let current_index = current_tala_id.and_then(|current_tala_id| {
                        directed
                            .nodes
                            .iter()
                            .position(|node| node.tala_id == current_tala_id)
                    });
                    let original_index = directed
                        .nodes
                        .iter()
                        .position(|node| node.tala_id == original_tala_id);
                    let Some(edge_index) = directed
                        .edge_records
                        .iter()
                        .position(|edge| edge.tala_id == edge_tala_id)
                    else {
                        return;
                    };
                    let edge_id = EdgeId(edge_index as u32);
                    if let Some(current_index) = current_index {
                        directed.nodes[current_index].edges.remove_first(edge_id);
                    }
                    if let Some(original_index) = original_index {
                        directed.nodes[original_index].edges.remove_first(edge_id);
                        directed.nodes[original_index].edges.push(edge_id);
                    }
                };
            for abduction in &placement.sized_edge_abductions {
                let edge_tala_id = abduction.edge_tala_id;
                for (current_local, original) in [
                    (
                        abduction.current_from.and_then(|node| node.local_node()),
                        abduction.originally_from,
                    ),
                    (
                        abduction.current_to.and_then(|node| node.local_node()),
                        abduction.originally_to,
                    ),
                ] {
                    let Some(original) = original else {
                        continue;
                    };
                    let current_tala_id = current_local.and_then(|current| {
                        scope
                            .graph
                            .nodes
                            .get(current.0 as usize)
                            .map(|node| node.tala_id)
                    });
                    // Ordinary hierarchy descendants are retained in Go's
                    // `Containers` map but are absent from the compact
                    // arena's `nodes` slice at this boundary.  Their stable
                    // ID is already the restore target. Aggregate members
                    // are the one exception: recover their vessel key from
                    // the external aggregate projection when the member is
                    // not materialized locally.
                    let original_tala_id = scope
                        .graph
                        .nodes
                        .iter()
                        .find(|node| node.tala_id == original.tala_id)
                        .map(|node| scope.graph.active_aggregate_owner(node.input_id))
                        .map(|owner| scope.graph.nodes[owner.0 as usize].tala_id)
                        .or_else(|| {
                            scope
                                .graph
                                .transaction_external_aggregate_children
                                .iter()
                                .find(|(_, children)| {
                                    children
                                        .iter()
                                        .any(|child| child.tala_id == original.tala_id)
                                })
                                .map(|(owner, _)| *owner)
                        })
                        .unwrap_or(original.tala_id);
                    if current_tala_id == Some(original_tala_id) {
                        continue;
                    }
                    if crate::internal::trace_env_enabled("WEFTAN_TRACE_RESTORED_DIRECT") {
                        eprintln!(
                            "RESTORED_DIRECT_RUST edge={} current={:?} original={}",
                            edge_tala_id, current_tala_id, original_tala_id
                        );
                    }
                    replay_restored_edge(current_tala_id, original_tala_id, edge_tala_id);
                }
            }
            #[cfg(feature = "diagnostic-traces")]
            {
                let copyback_owner_nodes = scope
                    .graph
                    .placement_tree_owner_copyback_order(&members)
                    .into_iter()
                    .filter_map(|tree_node| {
                        let tala_id = scope.graph.nodes[tree_node.0 as usize].tala_id;
                        trace_owner_nodes_by_tala
                            .get(&tala_id)
                            .copied()
                            .map(|owner_node| NodeId(owner_node as u32))
                    })
                    .collect::<Vec<_>>();
                trace_owner_graph
                    .node_order
                    .retain(|node| !copyback_owner_nodes.contains(node));
                trace_owner_graph.node_order.extend(copyback_owner_nodes);
                Self::publish_diagnostic_shared_geometry_since(
                    &mut trace_owner_graph,
                    &scope.graph,
                    &trace_before_restore_edges,
                );
                crate::internal::pipeline::trace_placement_state(
                    &format!("after-restore-edges-{subgraph_index}"),
                    &placement.root_external_id,
                    &trace_owner_graph,
                );
            }
            if crate::internal::trace_env_enabled("WEFTAN_TRACE_DIRECT_POS") {
                eprint!(
                    "DIRECT_BEFORE_RUST root={:?}",
                    placement
                        .scoring_node_metadata
                        .first()
                        .and_then(|metadata| metadata.parent_tala_id)
                );
                for node in &directed.nodes {
                    if let Some(position) = node.position {
                        eprint!(" {}={},{}", node.tala_id, position.x, position.y);
                    }
                }
                eprintln!();
            }
            if crate::internal::trace_env_enabled("WEFTAN_TRACE_DIRECT_204") && root_scope {
                eprintln!(
                    "DIRECT_204_RUST_BEFORE_NESTED {:?}",
                    directed
                        .transaction_external_container_children
                        .iter()
                        .filter(|(parent, _)| matches!(
                            **parent,
                            2884276447 | 4137376658 | 1323527919 | 2801930458 | 3677471320
                        ))
                        .map(|(parent, children)| (
                            *parent,
                            children
                                .iter()
                                .map(|child| (child.tala_id, child.position))
                                .collect::<Vec<_>>()
                        ))
                        .collect::<Vec<_>>()
                );
            }
            if crate::internal::trace_env_enabled("WEFTAN_TRACE_DEBUG_PARENT")
                && placement
                    .scoring_node_metadata
                    .first()
                    .and_then(|metadata| metadata.parent_tala_id)
                    .is_none()
            {
                eprintln!(
                    "DEBUG_PARENT_RUST phase=before_direct {:?}",
                    directed
                        .transaction_external_container_children
                        .iter()
                        .filter(|(parent, _)| matches!(**parent, 3536874587 | 370564473))
                        .map(|(parent, children)| (
                            *parent,
                            children
                                .iter()
                                .map(|child| (child.tala_id, child.position))
                                .collect::<Vec<_>>()
                        ))
                        .collect::<Vec<_>>()
                );
            }
            // `false` is material: TALA applies the requested-axis mirror
            // directly instead of retaining it only on a global score win.
            if crate::internal::trace_env_enabled("WEFTAN_TRACE_SCOPE_STEPS") {
                eprintln!(
                    "SCOPE_STEP_RUST before_direct root={:?}",
                    placement
                        .scoring_node_metadata
                        .first()
                        .and_then(|metadata| metadata.parent_tala_id)
                );
            }
            if !singleton_scope {
                crate::internal::pipeline::trace_placement_state(
                    "before-direct",
                    &placement.root_external_id,
                    &directed,
                );
            }
            #[cfg(feature = "diagnostic-traces")]
            let trace_before_direct = scope.graph.clone();
            let (mirrors, projected_mirror_refits) =
                directed.direct_with_reachable(false, Some(&directed_reachable_containers));
            if crate::internal::trace_env_enabled("WEFTAN_TRACE_DIRECT_204") && root_scope {
                eprintln!(
                    "DIRECT_204_RUST mirrors={:?} children_before_copy={:?}",
                    mirrors,
                    directed
                        .transaction_external_container_children
                        .iter()
                        .filter(|(parent, _)| **parent == 2043751575)
                        .map(|(parent, children)| (
                            *parent,
                            children
                                .iter()
                                .map(|child| (child.tala_id, child.position))
                                .collect::<Vec<_>>(),
                        ))
                        .collect::<Vec<_>>()
                );
                eprintln!(
                    "DIRECT_204_RUST_ALL {:?}",
                    directed
                        .transaction_external_container_children
                        .iter()
                        .filter(|(parent, _)| matches!(**parent, 2043751575 | 3248328362))
                        .map(|(parent, children)| (
                            *parent,
                            children
                                .iter()
                                .map(|child| (child.tala_id, child.position))
                                .collect::<Vec<_>>()
                        ))
                        .collect::<Vec<_>>()
                );
                eprintln!(
                    "DIRECT_204_RUST_NESTED {:?}",
                    directed
                        .transaction_external_container_children
                        .iter()
                        .filter(|(parent, _)| matches!(
                            **parent,
                            2884276447 | 4137376658 | 1323527919 | 2801930458 | 3677471320
                        ))
                        .map(|(parent, children)| (
                            *parent,
                            children
                                .iter()
                                .map(|child| (child.tala_id, child.position))
                                .collect::<Vec<_>>()
                        ))
                        .collect::<Vec<_>>()
                );
                eprintln!(
                    "DIRECT_204_RUST_AGG {:?}",
                    directed
                        .transaction_external_aggregate_children
                        .iter()
                        .filter(|(parent, _)| matches!(
                            **parent,
                            6129484611666145821 | 8674665223082153551
                        ))
                        .map(|(parent, children)| (
                            *parent,
                            children
                                .iter()
                                .map(|child| (child.tala_id, child.position))
                                .collect::<Vec<_>>()
                        ))
                        .collect::<Vec<_>>()
                );
            }
            if crate::internal::trace_env_enabled("WEFTAN_TRACE_HIDDEN_EXTERNAL") && root_scope {
                let mut changed = Vec::new();
                for (parent, before_children) in &pre_direct_external_children {
                    if let Some(after_children) =
                        directed.transaction_external_container_children.get(parent)
                    {
                        for before in before_children {
                            if let Some(after) = after_children
                                .iter()
                                .find(|candidate| candidate.tala_id == before.tala_id)
                                && (before.position != after.position
                                    || before.rect.size != after.rect.size)
                            {
                                changed.push((
                                    *parent,
                                    before.tala_id,
                                    before.position,
                                    after.position,
                                    before.rect.size,
                                    after.rect.size,
                                ));
                            }
                        }
                    }
                }
                eprintln!("HIDDEN_EXTERNAL_CHANGED_RUST {:?}", changed);
                eprintln!(
                    "HIDDEN_EXTERNAL_AFTER_ROOT_RUST {:?}",
                    directed
                        .nodes
                        .iter()
                        .map(|node| (node.tala_id, node.position, node.rect.size))
                        .collect::<Vec<_>>()
                );
            }
            if !singleton_scope {
                crate::internal::pipeline::trace_placement_state(
                    "after-direct",
                    &placement.root_external_id,
                    &directed,
                );
            }
            if crate::internal::trace_env_enabled("WEFTAN_TRACE_MOVE_PROJECTION")
                && placement
                    .scoring_node_metadata
                    .first()
                    .and_then(|metadata| metadata.parent_tala_id)
                    .is_none()
            {
                eprintln!(
                    "DIRECT_PROJECTIONS_RUST ordinary={:?} aggregate={:?}",
                    directed
                        .transaction_external_container_children
                        .values()
                        .flatten()
                        .filter(|node| matches!(node.tala_id, 3818877272 | 3869210129))
                        .map(|node| (node.tala_id, node.position))
                        .collect::<Vec<_>>(),
                    directed
                        .transaction_external_aggregate_children
                        .values()
                        .flatten()
                        .filter(|node| matches!(node.tala_id, 3818877272 | 3869210129))
                        .map(|node| (node.tala_id, node.position))
                        .collect::<Vec<_>>()
                );
            }
            if crate::internal::trace_env_enabled("WEFTAN_TRACE_DEBUG_PARENT")
                && placement
                    .scoring_node_metadata
                    .first()
                    .and_then(|metadata| metadata.parent_tala_id)
                    .is_none()
            {
                eprintln!(
                    "DEBUG_PARENT_RUST phase=after_direct mirrors={:?} {:?}",
                    mirrors,
                    directed
                        .transaction_external_container_children
                        .iter()
                        .filter(|(parent, _)| matches!(**parent, 3536874587 | 370564473))
                        .map(|(parent, children)| (
                            *parent,
                            children
                                .iter()
                                .map(|child| (child.tala_id, child.position))
                                .collect::<Vec<_>>()
                        ))
                        .collect::<Vec<_>>()
                );
            }
            if crate::internal::trace_env_enabled("WEFTAN_TRACE_ALL_MIRRORS") {
                eprintln!(
                    "ALL_MIRRORS_RUST root={:?} mirrors={:?} directed_nodes={} external_containers={}",
                    placement
                        .scoring_node_metadata
                        .first()
                        .and_then(|metadata| metadata.parent_tala_id),
                    mirrors,
                    directed.nodes.len(),
                    directed.transaction_external_containers.len()
                );
            }
            if crate::internal::trace_env_enabled("WEFTAN_TRACE_SCOPE_STEPS") {
                eprintln!(
                    "SCOPE_STEP_RUST after_direct root={:?}",
                    placement
                        .scoring_node_metadata
                        .first()
                        .and_then(|metadata| metadata.parent_tala_id)
                );
            }
            if crate::internal::trace_env_enabled("WEFTAN_TRACE_FLAT_SCOPE")
                && directed_old_by_new.iter().any(|old| {
                    matches!(
                        scope.graph.nodes[old.0 as usize].tala_id,
                        727019916 | 2317112547 | 1098606932
                    )
                })
            {
                eprintln!(
                    "FLAT_SCOPE_DIRECT_RUST mirrors={:?} members={:?}",
                    mirrors,
                    directed_old_by_new
                        .iter()
                        .map(|old| (
                            scope.graph.nodes[old.0 as usize].tala_id,
                            directed.nodes[directed_old_by_new
                                .iter()
                                .position(|candidate| candidate == old)
                                .unwrap()]
                            .position,
                            directed.nodes[directed_old_by_new
                                .iter()
                                .position(|candidate| candidate == old)
                                .unwrap()]
                            .rect
                            .size,
                        ))
                        .collect::<Vec<_>>()
                );
            }
            let directed_tala_ids = directed
                .nodes
                .iter()
                .map(|node| node.tala_id)
                .collect::<BTreeSet<_>>();
            let projected_vessel_tala_ids = directed
                .transaction_external_cluster_layouts
                .keys()
                .copied()
                .chain(
                    directed
                        .transaction_external_aggregate_children
                        .keys()
                        .copied(),
                )
                .collect::<BTreeSet<_>>();
            let has_external_hidden_aggregate =
                pre_direct_external_children.values().any(|children| {
                    Self::external_children_have_hidden_aggregate(
                        children,
                        &directed_tala_ids,
                        &projected_vessel_tala_ids,
                    )
                });
            let publish_external_mirror_offsets = mirrors != (false, false)
                && (4..50).contains(&members.len())
                && has_external_hidden_aggregate;
            if publish_external_mirror_offsets {
                // A container that is itself present in this directed graph
                // is refitted in that graph's absolute coordinate frame.
                // Publish those offsets at every recursive scope: replaying
                // the fit after translating the stable arena can change the
                // release's absolute half-value rounding by one pixel.
                for (&container_tala_id, children) in &pre_direct_external_children {
                    // Shared-pointer refitting belongs to the container that
                    // directly owns the hidden aggregate carrier. A carrier
                    // elsewhere in the temporary graph does not make an
                    // unrelated directed container part of that publication.
                    if !Self::external_children_have_hidden_aggregate(
                        children,
                        &directed_tala_ids,
                        &projected_vessel_tala_ids,
                    ) {
                        continue;
                    }
                    let Some(container) = directed.nodes.iter().find_map(|node| {
                        (node.tala_id == container_tala_id
                            && directed.node_order.contains(&node.input_id))
                        .then_some(node.input_id)
                    }) else {
                        continue;
                    };
                    let scope_container = directed_old_by_new[container.0 as usize];
                    let Some(owner_container) = placement
                        .old_to_new
                        .iter()
                        .find_map(|(&owner, &local)| (local == scope_container).then_some(owner))
                    else {
                        continue;
                    };
                    if !reachable_containers.contains(&Some(owner_container)) {
                        continue;
                    }
                    let Some(container_position) = directed.position(container) else {
                        continue;
                    };
                    let mirrored_children = Self::mirrored_external_container_children(
                        &directed, container, children, mirrors,
                    );
                    for child in mirrored_children {
                        let Some(child_position) = child.position else {
                            continue;
                        };
                        mirrored_external_child_offsets.push((
                            container_tala_id,
                            child.tala_id,
                            Point {
                                x: child_position.x - container_position.x,
                                y: child_position.y - container_position.y,
                            },
                        ));
                    }
                }
            }
            // Projected-only ordinary containers participate in mirrorAxes'
            // postorder positionContainerChildren callback without having their
            // own boxes reflected. Preserve only offsets whose callback made a
            // real move, directly from the post-direct pointer projection. The
            // owner's later mirror replay runs in a translated frame where
            // half-value rounding can differ by one pixel.
            mirrored_external_child_offsets.extend(Self::projected_mirror_refit_offsets(
                &directed,
                &projected_mirror_refits,
            ));
            // Source direct mutates the component's shared points. Publish
            // the concrete mapped nodes before the next component can read
            // them through the live owner; the dense copyback alone is stale.
            if let Some(session) = owner_session.as_deref_mut() {
                let updates = directed_old_by_new
                    .iter()
                    .enumerate()
                    .filter_map(|(index, parent)| {
                        directed.position(NodeId(index as u32)).map(|_| {
                            (
                                *parent,
                                OwnerNodeGeometry::from_graph(&directed, NodeId(index as u32)),
                            )
                        })
                    })
                    .collect::<Vec<_>>();
                session
                    .publish_component_direct_geometry(&updates)
                    .map_err(|error| crate::LayoutError::PlacementTransaction(error.to_string()))?;
                session
                    .publish_projected_direct_geometry(
                        pre_direct_external_children.values().flatten(),
                        directed
                            .transaction_external_container_children
                            .values()
                            .flatten(),
                    )
                    .map_err(|error| crate::LayoutError::PlacementTransaction(error.to_string()))?;
                session
                    .publish_projected_direct_geometry(
                        pre_direct_external_aggregate_children.values().flatten(),
                        directed
                            .transaction_external_aggregate_children
                            .values()
                            .flatten(),
                    )
                    .map_err(|error| crate::LayoutError::PlacementTransaction(error.to_string()))?;
                session
                    .publish_projected_direct_geometry(
                        pre_direct_external_containers.iter(),
                        directed.transaction_external_containers.iter(),
                    )
                    .map_err(|error| crate::LayoutError::PlacementTransaction(error.to_string()))?;
            }
            for (new_index, old) in directed_old_by_new.into_iter().enumerate() {
                scope.graph.nodes[old.0 as usize].position =
                    directed.position(NodeId(new_index as u32));
                if let Some(projected) = directed.tree_routing_nodes.get(&NodeId(new_index as u32))
                    && let Some(tree) = scope.graph.tree_routing_nodes.get_mut(&old)
                {
                    if crate::internal::trace_env_enabled("WEFTAN_TRACE_TREE_ORIENT")
                        && matches!(
                            scope.graph.nodes[old.0 as usize].tala_id,
                            1171112347 | 1154334728 | 1271778061 | 1187889966
                        )
                    {
                        eprintln!(
                            "TREE_ORIENT_DIRECT_RUST tala={} from={:?} to={:?}",
                            scope.graph.nodes[old.0 as usize].tala_id,
                            tree.orientation,
                            projected.orientation
                        );
                    }
                    tree.orientation = projected.orientation;
                    scope
                        .graph
                        .publish_tree_orientation(old, projected.orientation);
                }
            }
            // `Graph.direct` walks shared pointers reachable through the
            // temporary container/cluster maps. Those aggregate members are
            // intentionally absent from `directed_old_by_new`, so copying
            // only the concrete `Nodes` slice drops a reflected member back
            // into the owner's pre-direct frame. Publish every aggregate
            // pointer projection into the stable arena and its aliases before
            // the enclosing scope combines components.
            for placed_child in directed
                .transaction_external_aggregate_children
                .values()
                .flatten()
                .cloned()
                .collect::<Vec<_>>()
            {
                let changed = pre_direct_external_aggregate_children
                    .values()
                    .flatten()
                    .find(|before| before.tala_id == placed_child.tala_id)
                    .is_none_or(|before| {
                        before.position != placed_child.position
                            || before.rect.origin != placed_child.rect.origin
                            || before.rect.size != placed_child.rect.size
                    });
                if !changed {
                    continue;
                }
                let update = |node: &mut ArenaNode| {
                    if node.tala_id == placed_child.tala_id {
                        node.position = placed_child.position;
                        node.rect.origin = placed_child.rect.origin;
                        node.rect.size = placed_child.rect.size;
                    }
                };
                for node in &mut scope.graph.nodes {
                    update(node);
                }
                for children in scope
                    .graph
                    .transaction_external_container_children
                    .values_mut()
                {
                    for node in children {
                        update(node);
                    }
                }
                for children in scope
                    .graph
                    .transaction_external_aggregate_children
                    .values_mut()
                {
                    for node in children {
                        update(node);
                    }
                }
                for node in &mut scope.graph.transaction_external_containers {
                    update(node);
                }
                if crate::internal::trace_env_enabled("WEFTAN_TRACE_MOVE_PROJECTION") {
                    eprintln!(
                        "DIRECT_AGG_COPYBACK_RUST member={} pos={:?}",
                        placed_child.tala_id, placed_child.position
                    );
                }
            }
            // The Go implementation's induced graphs retain shared *Node
            // pointers for ordinary container children as well as aggregate
            // members.  `directed` owns value snapshots, so publish the
            // ordinary projections before the enclosing scope resumes its
            // component combination.  Without this copyback, a root-level
            // mirrorAxes pass can move an aliased descendant in Go while its
            // Rust owner remains in the pre-direct frame.
            // `Graph.direct` mutates the live Go *Node pointers reachable
            // through Containers, including ordinary projected children.
            // The Rust induced graph stores value snapshots, so always copy
            // the resulting projections back before the enclosing scope
            // combines components.  Restricting this to the historical
            // hidden-aggregate case leaves ordinary aliases in their
            // pre-direct frame and produces invalid shared offsets.
            // Copy only pointers whose direct pass actually changed a box.
            // Each induced graph starts as a value clone, while Go mutates
            // shared pointers; publishing unchanged snapshots from an
            // unrelated component would overwrite the owner with that
            // component's local coordinate frame.
            let selected_containers = directed
                .transaction_external_containers
                .iter()
                .filter(|external| {
                    pre_direct_external_containers
                        .iter()
                        .find(|before| before.tala_id == external.tala_id)
                        .is_none_or(|before| {
                            before.position != external.position
                                || before.rect.origin != external.rect.origin
                                || before.rect.size != external.rect.size
                        })
                })
                .cloned()
                .collect();
            let selected_children = directed
                .transaction_external_container_children
                .iter()
                .filter(|(parent, children)| {
                    let before_children = pre_direct_external_children.get(parent);
                    children.iter().any(|child| {
                        before_children
                            .into_iter()
                            .flatten()
                            .find(|before| before.tala_id == child.tala_id)
                            .is_none_or(|before| {
                                before.position != child.position
                                    || before.rect.origin != child.rect.origin
                                    || before.rect.size != child.rect.size
                            })
                    })
                })
                .map(|(&parent, children)| (parent, children.clone()))
                .collect();
            let selected_aggregates = directed
                .transaction_external_aggregate_children
                .iter()
                .filter(|(parent, children)| {
                    let before_children = pre_direct_external_aggregate_children.get(parent);
                    children.iter().any(|child| {
                        before_children
                            .into_iter()
                            .flatten()
                            .find(|before| before.tala_id == child.tala_id)
                            .is_none_or(|before| {
                                before.position != child.position
                                    || before.rect.origin != child.rect.origin
                                    || before.rect.size != child.rect.size
                            })
                    })
                })
                .map(|(&parent, children)| (parent, children.clone()))
                .collect();
            // Publication only reads the selected numerical geometry. Keep the
            // directed graph's other state in place instead of detaching its
            // topology, routes and cost owners for this one read-only call.
            let original_containers = std::mem::replace(
                &mut directed.transaction_external_containers,
                selected_containers,
            );
            let original_children = std::mem::replace(
                &mut directed.transaction_external_container_children,
                selected_children,
            );
            let original_aggregates = std::mem::replace(
                &mut directed.transaction_external_aggregate_children,
                selected_aggregates,
            );
            Self::publish_induced_external_geometry(&mut scope.graph, &directed);
            directed.transaction_external_containers = original_containers;
            directed.transaction_external_container_children = original_children;
            directed.transaction_external_aggregate_children = original_aggregates;
            if crate::internal::trace_env_value("WEFTAN_TRACE_FLAT_SCOPE_PHASES").is_some_and(
                |target| {
                    target == "all"
                        || placement
                            .scoring_node_metadata
                            .first()
                            .and_then(|metadata| metadata.parent_tala_id)
                            .is_some_and(|root| root == target.parse::<i64>().unwrap_or_default())
                },
            ) {
                eprintln!(
                    "FLAT_SCOPE_PHASE_RUST phase=after-publish root={:?} map={:?} nodes={:?}",
                    placement
                        .scoring_node_metadata
                        .first()
                        .and_then(|metadata| metadata.parent_tala_id),
                    scope
                        .graph
                        .transaction_external_container_children
                        .get(&4254926550)
                        .map(|children| {
                            children
                                .iter()
                                .map(|child| (child.tala_id, child.position))
                                .collect::<Vec<_>>()
                        }),
                    scope
                        .graph
                        .nodes
                        .iter()
                        .filter(|node| matches!(node.tala_id, 4254926550 | 1369972579 | 3471367806))
                        .map(|node| (node.tala_id, node.position))
                        .collect::<Vec<_>>()
                );
            }
            if crate::internal::trace_env_enabled("WEFTAN_TRACE_COPYBACK")
                && placement
                    .scoring_node_metadata
                    .first()
                    .and_then(|metadata| metadata.parent_tala_id)
                    .is_none()
            {
                eprintln!(
                    "COPYBACK_PHASE_RUST after_direct owner={:?} stable={:?}",
                    scope
                        .graph
                        .transaction_external_container_children
                        .iter()
                        .filter(|(parent, _)| **parent == 3248328362 || **parent == 3680227035)
                        .map(|(parent, children)| (
                            *parent,
                            children
                                .iter()
                                .map(|child| (child.tala_id, child.position))
                                .collect::<Vec<_>>()
                        ))
                        .collect::<Vec<_>>(),
                    scope
                        .graph
                        .nodes
                        .iter()
                        .filter(|node| matches!(node.tala_id, 3248328362 | 3680227035 | 1832207848))
                        .map(|node| (node.tala_id, node.position))
                        .collect::<Vec<_>>()
                );
            }
            if crate::internal::trace_env_enabled("WEFTAN_TRACE_DIRECT_204") && root_scope {
                let tracked = [
                    2043751575_i64,
                    3677471320,
                    2884276447,
                    1010209959,
                    1231386872,
                    2204119069,
                    4137376658,
                    1323527919,
                    2801930458,
                    696109821,
                    3554470064,
                    2202785448,
                    6129484611666145821,
                ];
                let dump = |graph: &ArenaGraph| {
                    graph
                        .transaction_external_container_children
                        .iter()
                        .filter(|(parent, _)| tracked.contains(parent))
                        .map(|(parent, children)| {
                            (
                                *parent,
                                children
                                    .iter()
                                    .filter(|child| tracked.contains(&child.tala_id))
                                    .map(|child| (child.tala_id, child.position))
                                    .collect::<Vec<_>>(),
                            )
                        })
                        .collect::<Vec<_>>()
                };
                eprintln!("DIRECT_204_RUST_AFTER_PUBLISH {:?}", dump(&scope.graph));
            }
            if root_scope && scope.graph.nodes.len() == members.len() {
                // Root direct can refit containers through projected aliases;
                // their dimensions are shared pointer state, while positions
                // remain in the temporary combine frame. Carry sizes only so
                // the enclosing fit sees the same boxes without importing
                // local absolute coordinates.
                for (&parent_tala_id, placed_children) in
                    &directed.transaction_external_container_children
                {
                    let Some(owner_children) = scope
                        .graph
                        .transaction_external_container_children
                        .get_mut(&parent_tala_id)
                    else {
                        continue;
                    };
                    for placed_child in placed_children {
                        if let Some(owner_child) = owner_children
                            .iter_mut()
                            .find(|child| child.tala_id == placed_child.tala_id)
                        {
                            owner_child.rect.size = placed_child.rect.size;
                        }
                    }
                }
            }
            if mirrors != (false, false) {
                for old in old_by_new {
                    descendant_mirrors.insert(
                        old,
                        SubtreeMirror {
                            mirror_x: mirrors.0,
                            mirror_y: mirrors.1,
                            reachable_containers: reachable_containers.clone(),
                        },
                    );
                }
            }
            if crate::internal::trace_env_enabled("WEFTAN_TRACE_GRID_ROOT_PHASES")
                && scope
                    .graph
                    .transaction_external_container_children
                    .contains_key(&233_611_931)
            {
                eprintln!(
                    "GRID_ROOT_PHASE_RUST phase=after-direct {:?}",
                    scope
                        .graph
                        .transaction_external_container_children
                        .get(&233_611_931)
                        .map(|children| children
                            .iter()
                            .filter_map(|child| {
                                matches!(
                                    child.tala_id,
                                    498_183_754
                                        | 1_149_337_423
                                        | 3_584_521_799
                                        | 1_782_109_120
                                        | 2_919_616_609
                                )
                                .then_some((
                                    child.tala_id,
                                    child.position,
                                    child.rect.size,
                                ))
                            })
                            .collect::<Vec<_>>())
                );
                for parent_tala_id in [1_149_337_423_i64, 1_782_109_120] {
                    if let Some(children) = scope
                        .graph
                        .transaction_external_container_children
                        .get(&parent_tala_id)
                    {
                        eprintln!(
                            "GRID_ROOT_PHASE_RUST_CHILDREN phase=after-init parent={} {:?}",
                            parent_tala_id,
                            children
                                .iter()
                                .map(|node| (node.tala_id, node.position, node.rect.size))
                                .collect::<Vec<_>>()
                        );
                    }
                }
            }
            // Match the upstream `tracePlacement` boundaries around each
            // component's direct pass and edge-abduction restoration. Go
            // traces the full owner graph because the induced graph aliases
            // its node pointers; publish this component's pointer-visible
            // geometry into the diagnostic owner clone before tracing.
            #[cfg(feature = "diagnostic-traces")]
            {
                for member in &members {
                    let placed = &scope.graph.nodes[member.0 as usize];
                    if let Some(&owner_index) = trace_owner_nodes_by_tala.get(&placed.tala_id) {
                        let owner = &mut trace_owner_graph.nodes[owner_index];
                        owner.position = placed.position;
                        owner.rect.origin = placed.rect.origin;
                        owner.rect.size = placed.rect.size;
                    }
                }
                // Go re-adds PlaceTrees' retained tree nodes to the owning
                // Graph.Nodes slice before its first owner callback after
                // direct placement. The Rust owner snapshot has those same
                // nodes in its stable arena but keeps them outside node_order
                // until the enclosing placement scope finishes.
                for &tree_node in &directed_members {
                    if !scope.graph.tree_routing_nodes.contains_key(&tree_node) {
                        continue;
                    }
                    let tala_id = scope.graph.nodes[tree_node.0 as usize].tala_id;
                    if let Some(&owner_node) = trace_owner_nodes_by_tala.get(&tala_id)
                        && !trace_owner_graph
                            .node_order
                            .contains(&NodeId(owner_node as u32))
                    {
                        trace_owner_graph.node_order.push(NodeId(owner_node as u32));
                    }
                }
                // The owner trace already contains the optimizer callback's
                // geometry. Publish only changes made by this direct pass so
                // external value snapshots from the induced graph cannot
                // rewind siblings that an earlier component already moved.
                let changed = Self::publish_diagnostic_shared_geometry_since(
                    &mut trace_owner_graph,
                    &scope.graph,
                    &trace_before_direct,
                );
                crate::internal::pipeline::trace_remember_owner_geometry_changes(
                    &trace_owner_graph,
                    &changed,
                );
                crate::internal::pipeline::trace_placement_state(
                    &format!("after-direct-owner-{subgraph_index}"),
                    &placement.root_external_id,
                    &trace_owner_graph,
                );
                crate::internal::pipeline::trace_placement_state(
                    &format!("after-subgraph-{subgraph_index}"),
                    &placement.root_external_id,
                    &trace_owner_graph,
                );
            }
            #[cfg(feature = "diagnostic-traces")]
            Self::trace_scope_raw_handoff(
                &placement.root_external_id,
                "before-leave-component",
                Some(subgraph_index),
                &scope.graph,
                owner_session.as_deref().map(|session| session.owner()),
            );
            if let Some(component) = owner_component {
                owner_session
                    .as_deref_mut()
                    .expect("component retains the owning session")
                    .leave_component(component, true)
                    .map_err(|error| crate::LayoutError::PlacementTransaction(error.to_string()))?;
            }
            #[cfg(feature = "diagnostic-traces")]
            Self::trace_scope_raw_handoff(
                &placement.root_external_id,
                "after-leave-component",
                Some(subgraph_index),
                &scope.graph,
                owner_session.as_deref().map(|session| session.owner()),
            );
            combined_subgraphs.push(directed_members);
        }
        for (node, position, size) in local_member_positions {
            let owner = &mut scope.graph.nodes[node.0 as usize];
            owner.position = Some(position);
            owner.rect.origin = position;
            owner.rect.size = size;
        }
        if crate::internal::trace_env_enabled("WEFTAN_TRACE_COMBINE_ALL") {
            eprint!("PLACEMENT_BEFORE_COMBINE_RUST ");
            for node in &scope.graph.nodes {
                if let Some(position) = node.position {
                    eprint!("{}={},{} ", node.tala_id, position.x, position.y);
                }
            }
            eprintln!();
        }
        // Go's `placeNodes` returns immediately for a one-node root graph
        // after the singleton fast path. In particular it does not run the
        // root `CombineSubgraphs` pass, whose recovered bounding-box padding
        // would move a `style.multiple` node by (10, 10). Keep that lifecycle
        // detail observable in the arena as well: nested scopes still combine
        // their singleton components normally.
        // Go's placeNodes singleton return is based on the complete graph
        // passed to it, not on the number of direct children in the
        // temporary root childrenGraph. A root with one container can still
        // have many descendants and therefore must run CombineSubgraphs;
        // otherwise its node-level label bounds never translate the root
        // component into the same frame as the oracle.
        // Tree preprocessing removes retained tree descendants from Go's
        // owning Graph.Nodes slice before placeNodes dispatches its singleton
        // fast path.  The Rust arena keeps those descendants addressable as
        // tree auxiliaries, so owning_graph_node_count deliberately includes
        // them for ordinary nested scopes.  At the root, however, a single
        // visible sentinel plus retained tree auxiliaries is exactly Go's
        // len(Graph.Nodes)==1 case: trees are reconnected and mirrored, but
        // CombineSubgraphs is skipped.  Preserve that lifecycle boundary so
        // NodePlacement retains the tree's local (possibly negative) frame;
        // Normalize performs the later global translation just as upstream.
        let singleton_root = (placement.owning_graph_node_count == 1
            || (placement.owner_node_count == 1 && !placement.tree_auxiliary_nodes.is_empty()))
            && placement
                .scoring_node_metadata
                .first()
                .and_then(|metadata| metadata.parent_tala_id)
                .is_none()
            && combined_subgraphs.len() == 1;
        #[cfg(feature = "diagnostic-traces")]
        crate::internal::pipeline::trace_placement_state(
            "before-combine",
            &placement.root_external_id,
            &trace_owner_graph,
        );
        #[cfg(feature = "diagnostic-traces")]
        let trace_before_combine = scope.graph.clone();
        #[cfg(feature = "diagnostic-traces")]
        Self::trace_scope_raw_handoff(
            &placement.root_external_id,
            "before-combine",
            None,
            &scope.graph,
            owner_session.as_deref().map(|session| session.owner()),
        );
        let combined_root_order = if !singleton_root {
            scope
                .graph
                .combine_known_placement_components_with_obstacles(
                    combined_subgraphs,
                    true,
                    false,
                    recovered_component_bounds,
                    &placement.sized_obstacles,
                )
                .map_err(|error| {
                    container_orientation::layout_error_for_work(
                        error,
                        seed,
                        "CombineSubgraphs",
                        limits::MAX_ENGINE_WORK_UNITS,
                    )
                })?
        } else {
            scope.graph.graph_node_order()
        };
        if !singleton_root && let Some(session) = owner_session.as_deref_mut() {
            // Packing moved shared Source pointers. Direct copyback now
            // includes restored tree nodes through the explicit parent map,
            // so the retained movement closure has its real prepacking points.
            session
                .publish_packed_scope_geometry(&scope.graph)
                .map_err(|error| crate::LayoutError::PlacementTransaction(error.to_string()))?;
        }
        #[cfg(feature = "diagnostic-traces")]
        {
            let changed = Self::publish_diagnostic_shared_geometry_since(
                &mut trace_owner_graph,
                &scope.graph,
                &trace_before_combine,
            );
            crate::internal::pipeline::trace_remember_owner_geometry_changes(
                &trace_owner_graph,
                &changed,
            );
            crate::internal::pipeline::trace_placement_state(
                "after-combine",
                &placement.root_external_id,
                &trace_owner_graph,
            );
        }
        // v0.9.0 reaches orientSourceInterior only after placeNodes passes its
        // empty/singleton fast paths and has children to combine. That call
        // can have a nil root for a multi-node top-level graph; preserve it
        // there while skipping the no-op invocation for singleton sources.
        #[cfg(feature = "diagnostic-traces")]
        let trace_before_orient = scope.graph.clone();
        // orientSourceInterior sees the temporary packed graph, while later
        // master-graph stages retain the original Graph.Nodes header.
        let master_node_order = scope.graph.node_order.clone();
        scope.graph.node_order = combined_root_order;
        scope.graph.refresh_node_order_membership();
        #[cfg(feature = "diagnostic-traces")]
        if crate::internal::trace_env_enabled("D2RS_TRACE_COMBINED_MEMBER_WORKSPACE")
            && crate::internal::trace_env_value("WEFTAN_TRACE_PLACEMENT_ROOT")
                .is_none_or(|target| target == placement.root_external_id)
        {
            if let Some(session) = owner_session.as_deref() {
                let geometry = session
                    .packed_group_member_geometry(&scope.graph)
                    .map_err(|error| crate::LayoutError::BadRouteState(error.to_string()))?;
                let (combined, receiver) = session
                    .combined_member_workspace(&scope.graph, &geometry)
                    .map_err(|error| crate::LayoutError::BadRouteState(error.to_string()))?;
                let owner_nodes = combined.graph_node_order().iter().map(|&id| {
                    let node = &session.owner().nodes[id.0 as usize];
                    let p = session.owner().position(id);
                    serde_json::json!({"canonical_node":id.0,"stable_id":node.tala_id.to_string(),
                        "x_bits":p.map(|p|format!("0x{:016x}",p.x.to_bits())),
                        "y_bits":p.map(|p|format!("0x{:016x}",p.y.to_bits())),
                        "inline_x_bits":node.position.map(|p|format!("0x{:016x}",p.x.to_bits())),
                        "inline_y_bits":node.position.map(|p|format!("0x{:016x}",p.y.to_bits())),
                        "w_bits":format!("0x{:016x}",node.rect.size.width.to_bits()),
                        "h_bits":format!("0x{:016x}",node.rect.size.height.to_bits())})
                }).collect::<Vec<_>>();
                let nodes = combined.graph_node_order().iter().map(|&id| {
                    let node = &combined.nodes[id.0 as usize];
                    let p = combined.position(id);
                    serde_json::json!({"canonical_node":id.0,"stable_id":node.tala_id.to_string(),
                        "owner_is_combined":combined.topology_node_owner(id)==Some(receiver),
                        "x_bits":p.map(|p|format!("0x{:016x}",p.x.to_bits())),
                        "y_bits":p.map(|p|format!("0x{:016x}",p.y.to_bits())),
                        "w_bits":format!("0x{:016x}",node.rect.size.width.to_bits()),
                        "h_bits":format!("0x{:016x}",node.rect.size.height.to_bits())})
                }).collect::<Vec<_>>();
                let prepack_nodes = trace_before_combine
                    .graph_node_order()
                    .iter()
                    .map(|&id| {
                        let node = &trace_before_combine.nodes[id.0 as usize];
                        let p = trace_before_combine.position(id);
                        serde_json::json!({"local_node":id.0,"stable_id":node.tala_id.to_string(),
                        "x_bits":p.map(|p|format!("0x{:016x}",p.x.to_bits())),
                        "y_bits":p.map(|p|format!("0x{:016x}",p.y.to_bits())),
                        "w_bits":format!("0x{:016x}",node.rect.size.width.to_bits()),
                        "h_bits":format!("0x{:016x}",node.rect.size.height.to_bits())})
                    })
                    .collect::<Vec<_>>();
                eprintln!(
                    "{}",
                    serde_json::json!({"event":"combined-member-workspace",
                    "root":placement.root_external_id,"nodes":nodes,"owner_nodes":owner_nodes,"prepack_nodes":prepack_nodes,
                    "explicit_member_geometry":geometry.iter().map(|(node,_)|node.0).collect::<Vec<_>>(),
                    "geometry_scope":"packed roots and explicit canonical group-member geometry; retained fallback remains visible"})
                );
            }
        }
        let source_orientation_applied = if !singleton_root
            && placement.owning_graph_node_count > 1
            && placement.owner_node_count > 0
        {
            scope.graph.orient_source_interior_with_owner(
                &placement.source_orientation,
                seed,
                owner_session.as_deref_mut(),
            )?
        } else {
            false
        };
        scope.graph.node_order = master_node_order;
        scope.graph.refresh_node_order_membership();
        if source_orientation_applied {
            // The source turn sets DesiredArrangement together with
            // Arrangement. Carry both mutations back from projected groups,
            // rather than retaining the pre-turn optimizer's requested axis.
            for projection in &placement.projected_clusters {
                let node = &scope.graph.nodes[projection.node.0 as usize];
                if let Some(layout) = scope
                    .graph
                    .transaction_external_cluster_layouts
                    .get(&node.tala_id)
                {
                    cluster_arrangements.insert(projection.cluster_index, layout.arrangement);
                    cluster_desired_arrangements
                        .insert(projection.cluster_index, layout.arrangement);
                }
            }
        }
        #[cfg(feature = "diagnostic-traces")]
        {
            let changed = Self::publish_diagnostic_shared_geometry_since(
                &mut trace_owner_graph,
                &scope.graph,
                &trace_before_orient,
            );
            crate::internal::pipeline::trace_remember_owner_geometry_changes(
                &trace_owner_graph,
                &changed,
            );
            crate::internal::pipeline::trace_placement_state(
                "after-orient",
                &placement.root_external_id,
                &trace_owner_graph,
            );
        }
        if crate::internal::trace_env_enabled("WEFTAN_TRACE_COPYBACK")
            && placement
                .scoring_node_metadata
                .first()
                .and_then(|metadata| metadata.parent_tala_id)
                .is_none()
        {
            eprintln!(
                "COPYBACK_PHASE_RUST after_combine owner={:?} stable={:?}",
                scope
                    .graph
                    .transaction_external_container_children
                    .iter()
                    .filter(|(parent, _)| **parent == 3248328362 || **parent == 3680227035)
                    .map(|(parent, children)| (
                        *parent,
                        children
                            .iter()
                            .map(|child| (child.tala_id, child.position))
                            .collect::<Vec<_>>()
                    ))
                    .collect::<Vec<_>>(),
                scope
                    .graph
                    .nodes
                    .iter()
                    .filter(|node| matches!(node.tala_id, 3248328362 | 3680227035 | 1832207848))
                    .map(|node| (node.tala_id, node.position))
                    .collect::<Vec<_>>()
            );
        }
        if crate::internal::trace_env_enabled("WEFTAN_TRACE_COMBINE_ALL")
            && placement
                .scoring_node_metadata
                .first()
                .and_then(|metadata| metadata.parent_tala_id)
                .is_none()
        {
            eprintln!(
                "PLACEMENT_AFTER_COMBINE_EXTERNAL_RUST {:?}",
                scope
                    .graph
                    .transaction_external_container_children
                    .get(&457364737)
                    .map(|children| children
                        .iter()
                        .map(|child| (child.tala_id, child.position))
                        .collect::<Vec<_>>())
            );
        }
        if crate::internal::trace_env_enabled("WEFTAN_TRACE_DIRECT_204")
            && placement
                .scoring_node_metadata
                .first()
                .and_then(|metadata| metadata.parent_tala_id)
                .is_some_and(|parent| parent == 2043751575)
        {
            eprintln!(
                "AFTER_COMBINE_204_RUST {:?}",
                scope
                    .graph
                    .transaction_external_container_children
                    .iter()
                    .filter(|(parent, _)| matches!(
                        **parent,
                        2043751575 | 2884276447 | 4137376658 | 1323527919 | 2801930458 | 3677471320
                    ))
                    .map(|(parent, children)| (
                        *parent,
                        children
                            .iter()
                            .map(|child| (child.tala_id, child.position))
                            .collect::<Vec<_>>()
                    ))
                    .collect::<Vec<_>>()
            );
        }
        if crate::internal::trace_env_enabled("WEFTAN_TRACE_COMBINE_ALL") {
            let root_tala_id = placement
                .scoring_node_metadata
                .first()
                .and_then(|metadata| metadata.parent_tala_id)
                .unwrap_or(0);
            eprint!("PLACEMENT_AFTER_COMBINE_RUST root={root_tala_id}");
            for node in &scope.graph.nodes {
                if let Some(position) = node.position {
                    eprint!(
                        " {}={},{}:{},{}",
                        node.tala_id,
                        position.x,
                        position.y,
                        node.rect.size.width,
                        node.rect.size.height
                    );
                }
            }
            eprintln!();
        }
        if crate::internal::trace_env_enabled("WEFTAN_TRACE_COMBINE_SCOPE")
            && scope
                .graph
                .nodes
                .iter()
                .any(|node| matches!(node.tala_id, 370564473 | 1472025070 | 739672091))
        {
            eprintln!(
                "COMBINE_SCOPE_RUST {:?}",
                scope
                    .graph
                    .nodes
                    .iter()
                    .filter(|node| {
                        matches!(
                            node.tala_id,
                            370564473
                                | 727019916
                                | 777352773
                                | 8475284246537043955
                                | 760575154
                                | 676687059
                                | 739672091
                                | 1472025070
                                | 2317112547
                                | 2333890166
                                | 2350667785
                                | 2367445404
                                | 1220360785
                                | 1148939789
                                | 1048274075
                                | 1098606932
                                | 1132162170
                        )
                    })
                    .map(|node| (node.tala_id, node.position, node.rect.size))
                    .collect::<Vec<_>>()
            );
        }
        // Pipeline.NodePlacementStage installs CommonUncleSiblings only while
        // placeNodes is scoring placement candidates, then clears the carrier
        // before the later alignment and normalization stages.
        scope
            .graph
            .common_uncle_siblings
            .replace_header(SharedSliceMap::nil());
        let cluster_vessel_positions = placement
            .projected_clusters
            .iter()
            .filter_map(|projection| {
                scope
                    .graph
                    .position(projection.node)
                    .map(|position| (projection.cluster_index, position))
            })
            .collect();
        let cluster_vessel_tala_ids = placement
            .projected_clusters
            .iter()
            .map(|projection| scope.graph.nodes[projection.node.0 as usize].tala_id)
            .collect::<BTreeSet<_>>();
        let mirrored_container_tala_ids = descendant_mirrors
            .keys()
            .map(|node| scope.graph.nodes[node.0 as usize].tala_id)
            .collect::<BTreeSet<_>>();
        let scope_node_tala_ids = scope
            .graph
            .node_order
            .iter()
            .map(|node| scope.graph.nodes[node.0 as usize].tala_id)
            .collect::<BTreeSet<_>>();
        // Graph.syncNested publishes a cluster member's child movement only
        // when the temporary cluster vessel is a current Graph.Nodes entry.
        // Retained aggregate metadata alone is not enough: it can outlive the
        // vessel and otherwise republishes stale hidden-container offsets.
        let live_cluster_vessel_tala_ids = scope
            .graph
            .node_order
            .iter()
            .filter_map(|node| {
                let node = &scope.graph.nodes[node.0 as usize];
                (node.scoring_is_aggregate_vessel
                    && node.scoring_cluster_arrangement.is_some()
                    && scope
                        .graph
                        .transaction_external_cluster_layouts
                        .contains_key(&node.tala_id))
                .then_some(node.tala_id)
            })
            .collect::<BTreeSet<_>>();
        let live_cluster_member_positions = live_cluster_vessel_tala_ids
            .iter()
            .filter_map(|vessel_tala_id| {
                scope
                    .graph
                    .transaction_external_aggregate_children
                    .get(vessel_tala_id)
            })
            .flatten()
            .filter_map(|member| Some((member.tala_id, member.position?)))
            .collect::<BTreeMap<_, _>>();
        if let Some(session) = owner_session.as_deref() {
            Self::reconcile_projected_sequence_vessels(session.owner(), &mut scope.graph);
        }
        let external_shared_child_offsets: Vec<(i64, i64, Point)> = scope
            .graph
            .transaction_external_container_children
            .iter()
            .filter_map(|(&container_tala_id, children)| {
                let cluster_vessel_tala_ids = &cluster_vessel_tala_ids;
                let mirrored_container_tala_ids = &mirrored_container_tala_ids;
                let scope_node_tala_ids = &scope_node_tala_ids;
                let container_position = scope
                    .graph
                    .nodes
                    .iter()
                    .find(|node| node.tala_id == container_tala_id)
                    .and_then(|node| node.position)
                    // A positioned external container is still a shared Go
                    // pointer even when it is not a member of this temporary
                    // Graph.Nodes slice.  Its final transaction carrier is
                    // the authoritative frame for child offsets.
                    .or_else(|| {
                        scope
                            .graph
                            .transaction_external_containers
                            .iter()
                            .find(|node| node.tala_id == container_tala_id)
                            .and_then(|node| node.position)
                    })
                    .or_else(|| {
                        scope
                            .graph
                            .transaction_external_container_children
                            .values()
                            .flatten()
                            .find(|node| node.tala_id == container_tala_id)
                            .and_then(|node| node.position)
                    })
                    // A container cluster member is absent from Graph.Nodes,
                    // but the live vessel's syncNested branch has just moved
                    // the same shared child pointers. Publish only that
                    // source-backed hidden-member case.
                    .or_else(|| {
                        live_cluster_member_positions
                            .get(&container_tala_id)
                            .copied()
                    })?;
                Some(children.iter().filter_map(move |child| {
                    if mirrored_container_tala_ids.contains(&container_tala_id)
                        && !cluster_vessel_tala_ids.contains(&child.tala_id)
                        && scope_node_tala_ids.contains(&child.tala_id)
                    {
                        return None;
                    }
                    let child_position = child.position?;
                    if matches!(
                        (container_tala_id, child.tala_id),
                        (233611931, 1149337423)
                            | (1149337423, 1402186134)
                            | (1149337423, 1385408515)
                            | (1149337423, 1368630896)
                    )
                        && crate::internal::trace_env_enabled("WEFTAN_TRACE_MOVE_PROJECTION")
                    {
                        eprintln!(
                            "EXTERNAL_OFFSET_BUILD_RUST container={} child={} container_pos={:?} child_pos={:?}",
                            container_tala_id, child.tala_id, container_position, child_position
                        );
                    }
                    Some((
                        container_tala_id,
                        child.tala_id,
                        Point {
                            x: child_position.x - container_position.x,
                            y: child_position.y - container_position.y,
                        },
                    ))
                }))
            })
            .flatten()
            .collect();
        if crate::internal::trace_env_enabled("WEFTAN_TRACE_ALIAS_FINAL")
            && placement
                .scoring_node_metadata
                .first()
                .and_then(|metadata| metadata.parent_tala_id)
                .is_none()
        {
            eprintln!(
                "ALIAS_FINAL_RUST nodes={:?} ext={:?}",
                scope
                    .graph
                    .nodes
                    .iter()
                    .filter(|node| matches!(
                        node.tala_id,
                        3248328362
                            | 3680227035
                            | 1832207848
                            | 2280770807
                            | 298920306
                            | 3297548089
                            | 3777840222
                            | 2869257226
                    ))
                    .map(|node| (node.tala_id, node.position, node.rect.size))
                    .collect::<Vec<_>>(),
                scope
                    .graph
                    .transaction_external_container_children
                    .iter()
                    .filter(|(parent, _)| matches!(
                        **parent,
                        3248328362 | 3680227035 | 1832207848 | 3297548089 | 3777840222
                    ))
                    .map(|(parent, children)| (
                        *parent,
                        children
                            .iter()
                            .filter(|child| matches!(
                                child.tala_id,
                                3248328362
                                    | 3680227035
                                    | 1832207848
                                    | 2280770807
                                    | 298920306
                                    | 3297548089
                                    | 3777840222
                                    | 2869257226
                            ))
                            .map(|child| (child.tala_id, child.position, child.rect.size))
                            .collect::<Vec<_>>()
                    ))
                    .collect::<Vec<_>>()
            );
        }
        if crate::internal::trace_env_enabled("WEFTAN_TRACE_EXTERNAL_OFFSETS_ALL") {
            eprintln!(
                "EXTERNAL_OFFSETS_ALL_RUST root={:?} offsets={:?}",
                placement
                    .scoring_node_metadata
                    .first()
                    .and_then(|metadata| metadata.parent_tala_id),
                external_shared_child_offsets
                    .iter()
                    .filter(|(container, child, _)| {
                        matches!(
                            (*container, *child),
                            (1832207848, 2280770807)
                                | (1832207848, 298920306)
                                | (3680227035, 1832207848)
                                | (3248328362, 3680227035)
                        )
                    })
                    .collect::<Vec<_>>()
            );
        }
        if crate::internal::trace_env_enabled("WEFTAN_TRACE_CLUSTER_POSITION") {
            eprintln!(
                "CLUSTER_POSITION_RUST root={:?} positions={:?} external={:?}",
                placement
                    .scoring_node_metadata
                    .first()
                    .and_then(|metadata| metadata.parent_tala_id),
                cluster_vessel_positions,
                external_shared_child_offsets
                    .iter()
                    .filter(|(_, child, _)| {
                        placement.projected_clusters.iter().any(|projection| {
                            scope.graph.nodes[projection.node.0 as usize].tala_id == *child
                        })
                    })
                    .collect::<Vec<_>>()
            );
        }
        if crate::internal::trace_env_enabled("WEFTAN_TRACE_GRID_OFFSETS") {
            eprintln!(
                "GRID_SCOPE_OFFSETS_RUST root={:?} parent={:?} parent_pos={:?} children={:?}",
                placement
                    .scoring_node_metadata
                    .first()
                    .and_then(|metadata| metadata.parent_tala_id),
                scope
                    .graph
                    .nodes
                    .iter()
                    .find(|node| node.tala_id == 233611931)
                    .map(|node| node.tala_id),
                scope
                    .graph
                    .nodes
                    .iter()
                    .find(|node| node.tala_id == 233611931)
                    .and_then(|node| node.position),
                scope
                    .graph
                    .transaction_external_container_children
                    .get(&233611931)
                    .map(|children| {
                        children
                            .iter()
                            .filter(|child| {
                                matches!(
                                    child.tala_id,
                                    498183754 | 1149337423 | 3584521799 | 1782109120 | 2919616609
                                )
                            })
                            .map(|child| (child.tala_id, child.position))
                            .collect::<Vec<_>>()
                    })
            );
        }
        if crate::internal::trace_env_value("WEFTAN_TRACE_FLAT_SCOPE_RETURN").is_some_and(
            |target| {
                target == "all"
                    || placement
                        .scoring_node_metadata
                        .first()
                        .and_then(|metadata| metadata.parent_tala_id)
                        .is_some_and(|root| root == target.parse::<i64>().unwrap_or_default())
            },
        ) {
            eprintln!(
                "FLAT_SCOPE_RETURN_RUST root={:?} external={:?} nodes={:?}",
                placement
                    .scoring_node_metadata
                    .first()
                    .and_then(|metadata| metadata.parent_tala_id),
                scope
                    .graph
                    .transaction_external_container_children
                    .get(&4254926550)
                    .map(|children| {
                        children
                            .iter()
                            .map(|child| (child.tala_id, child.position, child.rect.size))
                            .collect::<Vec<_>>()
                    }),
                scope
                    .graph
                    .nodes
                    .iter()
                    .filter(|node| matches!(node.tala_id, 4254926550 | 1369972579 | 3471367806))
                    .map(|node| (node.tala_id, node.position, node.rect.size))
                    .collect::<Vec<_>>()
            );
        }
        Ok(FlatScopePlacement {
            graph: scope.graph,
            source_orientation_applied,
            next_rng_float,
            descendant_mirrors,
            cluster_arrangements,
            cluster_desired_arrangements,
            cluster_vessel_positions,
            external_shared_child_offsets,
            mirrored_external_child_offsets,
        })
    }
}
