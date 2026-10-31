// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Uncharged numerical publication after authoritative candidate movement.
use super::{ArenaGraph, NodeId, OwnerScoreSession};

/// The edge scorer's ordinary endpoint boxes, route probes, Nears, herd and
/// common-uncle geometry already come from its OwnerScoreFrame. Container
/// alignment positions come from that frame as well. Table row ordering still
/// reads workspace neighbors, so column edges retain the eager bridge.
/// Flow continuity reads canonical endpoint rays through the score frame.
/// Aggregate scopes retain their projected-frame bookkeeping.
pub(super) fn owner_edge_score_without_publication(
    workspace: &ArenaGraph,
    session: &OwnerScoreSession<'_>,
    node: NodeId,
) -> bool {
    owner_edge_score_with_inert_external(workspace, session, node, false)
}

/// Nil external projections have no translation to replay. Mutation-capable
/// contexts retain the original admission rule when deferring publication.
pub(super) fn owner_edge_score_with_inert_external(
    workspace: &ArenaGraph,
    session: &OwnerScoreSession<'_>,
    node: NodeId,
    allow_inert_nil: bool,
) -> bool {
    let record = &workspace.nodes[node.0 as usize];
    !record.scoring_is_aggregate_vessel
        && workspace.clusters.is_empty()
        && workspace.sequences.is_empty()
        && workspace.active_edge_ids(node).iter().all(|edge| {
            !workspace.edge_records[edge.0 as usize].is_between_table_columns()
        })
        && (0..workspace.nodes.len()).all(|index| {
            workspace.position(NodeId(index as u32)).is_some()
        })
        // Skipping intermediate publications must not change rounding of an
        // implicit descendant's accumulated translations. Only direct owner
        // assignments can be coalesced across rejected candidates.
        && complete_position_projection(workspace, session, allow_inert_nil)
}

pub(super) fn mirror(workspace: &mut ArenaGraph, session: &OwnerScoreSession<'_>) {
    // An ordinary scope is a complete numerical projection: every box that
    // movement can touch is overwritten from its canonical owner below. There
    // is no aggregate frame or external child whose relative translation must
    // be reconstructed. Go candidate_movement.go moves these owners once;
    // replaying that movement through the projected Containers hierarchy does
    // no useful work. Keep the legacy bridge for scopes with implicit geometry.
    if complete_position_projection(workspace, session, true) {
        publish_positions(workspace, session);
        publish_external_positions(workspace, session);
        return;
    }
    mirror_with_descendants(workspace, session);
}

fn complete_position_projection(workspace: &ArenaGraph, session: &OwnerScoreSession<'_>, allow_inert_nil: bool) -> bool {
    session.current_mapping().nodes().len() == workspace.nodes.len()
        && workspace.clusters.is_empty()
        && workspace.sequences.is_empty()
        && workspace.transaction_external_containers.iter()
            .chain(workspace.transaction_external_container_children.values().flatten())
            .chain(workspace.transaction_external_aggregate_children.values().flatten())
            .all(|node| node.owner_geometry_source.is_some_and(|source| {
                session.owner().position(source).is_some()
                    || (allow_inert_nil && node.position.is_none())
            }))
        // A positioned carrier with a nil owner still needs descendant
        // replay before clearing it. An already-nil external carrier is inert:
        // translate_node_with_children only translates Some external points.
        // Inline points must all remain explicit, as before.
        && session.current_mapping().nodes().iter().all(|&node| session.owner().position(node).is_some())
}

fn publish_positions(workspace: &mut ArenaGraph, session: &OwnerScoreSession<'_>) {
    for (index, &canonical) in session.current_mapping().nodes().iter().enumerate() {
        let local = NodeId(index as u32);
        match session.owner().position(canonical) {
            Some(point) => workspace.set_position(local, point),
            None => workspace.clear_position(local),
        }
    }
}

fn mirror_with_descendants(workspace: &mut ArenaGraph, session: &OwnerScoreSession<'_>) {
    let owner = session.owner();
    let mapping = session.current_mapping().nodes();
    // Preserve the adapter's aggregate frame bookkeeping, then overwrite each
    // explicit point from its actual owner. No adapter walk writes the owner.
    for local in workspace.graph_node_order() {
        let canonical = mapping[local.0 as usize];
        if let Some(point) = owner.position(canonical) {
            workspace.move_active_node_abs_with_children(local, point);
        }
    }
    publish_positions(workspace, session);
    for local in workspace.graph_node_order() {
        let canonical = mapping[local.0 as usize];
        if let Some(point) = owner.position(canonical) {
            if let Some(index) = workspace.active_cluster_index(local) {
                if workspace.clusters[index].members.first() == Some(&local) {
                    workspace
                        .pending_cluster_vessel_positions
                        .insert(index, point);
                }
            }
            if let Some(index) = workspace.nodes[local.0 as usize].sequence {
                if workspace.sequences[index].members.first() == Some(&local) {
                    workspace
                        .serialized_vessel_positions_by_tala
                        .insert(workspace.sequences[index].vessel_tala_id, point);
                }
            }
        }
    }
    publish_external_positions(workspace, session);
}

fn publish_external_positions(workspace: &mut ArenaGraph, session: &OwnerScoreSession<'_>) {
    let owner = session.owner();
    let refresh = |node: &mut crate::internal::ArenaNode| {
        if let Some(source) = node.owner_geometry_source {
            match owner.position(source) {
                Some(point) => {
                    node.position = Some(point);
                    node.rect.origin = point;
                }
                None => node.position = None,
            }
        }
    };
    for node in &mut workspace.transaction_external_containers {
        refresh(node);
    }
    for children in workspace
        .transaction_external_container_children
        .values_mut()
    {
        for node in children {
            refresh(node);
        }
    }
    for children in workspace
        .transaction_external_aggregate_children
        .values_mut()
    {
        for node in children {
            refresh(node);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::internal::{CanonicalScoreMapping, OwnerGraphId};
    use crate::{Graph, Node, Point, Size};

    #[test]
    fn ordinary_edge_score_uses_owner_with_stale_carrier_boxes() {
        use crate::internal::scoring::SizedEdgeLengthScorer;
        use crate::internal::scoring_context::ScoreContext;
        let mut input = Graph::default();
        for index in 0..5 {
            input.add_node(Node::new(
                format!("n{index}"),
                Size {
                    width: 30.,
                    height: 20.,
                },
            ));
        }
        for (from, to) in [(0, 1), (0, 2), (1, 3), (2, 4)] {
            input.add_edge(crate::Edge {
                source: NodeId(from),
                target: NodeId(to),
            });
        }
        let mut owner = ArenaGraph::from_input(&input);
        let mut workspace = ArenaGraph::from_input(&input);
        owner.cell_size = 99.;
        workspace.cell_size = 99.;
        owner.set_owner_graph_cell_size(OwnerGraphId(0), 99.).unwrap();
        for (index, point) in [
            Point { x: 0., y: 0. },
            Point { x: 0., y: 0. },
            Point { x: 0., y: 150. },
            Point { x: 240., y: 0. },
            Point { x: 0., y: 300. },
        ]
        .into_iter()
        .enumerate()
        {
            owner.set_position(NodeId(index as u32), point);
            workspace.set_position(NodeId(index as u32), point);
        }
        // Ordinary containers exercise the carrier alignment term as well as
        // endpoint distance and routing. Their size and topology stay fixed.
        for index in 0..2 {
            owner.nodes[index].is_container = true;
            workspace.nodes[index].is_container = true;
            workspace.nodes[index].scoring_is_container = true;
        }
        let mapping = CanonicalScoreMapping::new(
            owner.topology_arena_id().unwrap(),
            (0..5).map(NodeId).collect(),
            (0..4).map(crate::EdgeId).collect(),
        );
        let mut session = OwnerScoreSession::new(&mut owner, OwnerGraphId(0), mapping, None).unwrap();
        assert!(owner_edge_score_without_publication(
            &workspace,
            &session,
            NodeId(0)
        ));
        let mut scorer = SizedEdgeLengthScorer::new(NodeId(0), true);
        scorer.use_owner_carrier_positions();
        let mut scores = Vec::new();
        // Reuse the same lazy scorer across owner moves, including a return
        // to the starting point. No publication is needed to refresh it.
        for point in [
            Point { x: 60., y: 90. },
            Point {
                x: -99.25,
                y: 300.5,
            },
            Point { x: 0., y: 0. },
        ] {
            session.owner_mut().set_position(NodeId(0), point);
            let mut eager = workspace.clone();
            mirror(&mut eager, &session);
            let actual = session
                .with_frame(|frame| {
                    scorer.score_owner_routes(
                        &workspace,
                        None,
                        frame.owner(),
                        NodeId(0),
                        Some(frame),
                        ScoreContext::none(),
                    )
                })
                .unwrap()
                .unwrap();
            let expected = session
                .with_frame(|frame| {
                    SizedEdgeLengthScorer::new(NodeId(0), true).score_owner_routes(
                        &eager,
                        None,
                        frame.owner(),
                        NodeId(0),
                        Some(frame),
                        ScoreContext::none(),
                    )
                })
                .unwrap()
                .unwrap();
            assert_eq!(actual.to_bits(), expected.to_bits());
            scores.push(actual.to_bits());
            assert_eq!(workspace.position(NodeId(0)), Some(Point { x: 0., y: 0. }));
        }
        assert_ne!(scores[0], scores[1], "owner movement must affect the score");
        workspace.nodes[0].is_container = false;
        workspace.nodes[0].scoring_is_container = false;
        assert!(owner_edge_score_without_publication(&workspace, &session, NodeId(0)));
        workspace.nodes[0].is_container = true;
        workspace.nodes[0].scoring_is_container = true;
        workspace.nodes[0].scoring_is_aggregate_vessel = true;
        assert!(!owner_edge_score_without_publication(
            &workspace,
            &session,
            NodeId(0)
        ));
        workspace.nodes[0].scoring_is_aggregate_vessel = false;
        workspace.edge_records[0].source_table_column = Some(0);
        workspace.edge_records[0].target_table_column = Some(1);
        assert!(!owner_edge_score_without_publication(
            &workspace,
            &session,
            NodeId(0)
        ));
        workspace.edge_records[0].source_table_column = None;
        workspace.edge_records[0].target_table_column = None;
        session.owner_mut().clear_position(NodeId(4));
        assert!(!owner_edge_score_without_publication(
            &workspace,
            &session,
            NodeId(0)
        ));
    }

    #[test]
    fn active_flow_uses_live_remapped_owners_across_reused_scores() {
        use crate::internal::scoring::SizedEdgeLengthScorer;
        use crate::internal::scoring_context::ScoreContext;
        let nodes = || {
            (0..4)
                .map(|index| {
                    Node::new(
                        format!("n{index}"),
                        Size {
                            width: 30.,
                            height: 20.,
                        },
                    )
                })
                .collect::<Vec<_>>()
        };
        let mut input = Graph::default();
        for node in nodes() {
            input.add_node(node);
        }
        let pairs = [(0, 1), (1, 2), (1, 3), (2, 1)];
        for (from, to) in pairs {
            input.add_edge(crate::Edge {
                source: NodeId(from),
                target: NodeId(to),
            });
        }
        let mut owner = ArenaGraph::from_input(&input);
        let mapping_nodes = vec![NodeId(2), NodeId(0), NodeId(3), NodeId(1)];
        let inverse = [1, 3, 0, 2];
        let mut local = Graph::default();
        for &canonical in &mapping_nodes {
            local.add_node(nodes()[canonical.0 as usize].clone());
        }
        for (from, to) in pairs {
            local.add_edge(crate::Edge {
                source: NodeId(inverse[from as usize]),
                target: NodeId(inverse[to as usize]),
            });
        }
        let mut workspace = ArenaGraph::from_input(&local);
        for edge in &mut owner.edge_records {
            edge.target_arrow = true;
        }
        for edge in &mut workspace.edge_records {
            edge.target_arrow = true;
        }
        owner.initialize_shared_topology();
        owner.cell_size = 99.;
        workspace.cell_size = 99.;
        owner.restore_routing_costs((1., 99., 2.));
        workspace.restore_routing_costs((1., 99., 2.));
        owner
            .restore_owner_graph_routing_costs(OwnerGraphId(0), (1., 99., 2.))
            .unwrap();
        owner
            .set_owner_graph_cell_size(OwnerGraphId(0), 99.)
            .unwrap();
        owner
            .set_owner_view_node_mapping(OwnerGraphId(0), mapping_nodes.clone())
            .unwrap();
        for (index, point) in [
            Point { x: 0., y: 0. },
            Point { x: 100., y: 0. },
            Point { x: 200., y: 0. },
            Point { x: 100., y: 200. },
        ]
        .into_iter()
        .enumerate()
        {
            owner.set_position(NodeId(index as u32), point);
            let mapped = inverse[index];
            workspace.set_position(NodeId(mapped), point);
        }
        let mapping = CanonicalScoreMapping::new(
            owner.topology_arena_id().unwrap(),
            mapping_nodes,
            (0..4).map(crate::EdgeId).collect(),
        );
        let mut session =
            OwnerScoreSession::new(&mut owner, OwnerGraphId(0), mapping, None).unwrap();
        let receiver = NodeId(3);
        assert!(owner_edge_score_without_publication(
            &workspace, &session, receiver
        ));
        let mut scorer = SizedEdgeLengthScorer::new(receiver, true);
        scorer.use_owner_carrier_positions();
        let mut observed_stale_flow = false;
        for point in [
            Point { x: 100., y: 40. },
            Point { x: 60., y: 120. },
            Point { x: 100., y: 0. },
        ] {
            session.owner_mut().set_position(NodeId(1), point);
            let mut eager = workspace.clone();
            mirror(&mut eager, &session);
            let evaluate = |graph: &ArenaGraph, scorer: &mut SizedEdgeLengthScorer| {
                session
                    .with_frame(|frame| {
                        scorer.score_owner_routes(
                            graph,
                            None,
                            frame.owner(),
                            NodeId(1),
                            Some(frame),
                            ScoreContext::none(),
                        )
                    })
                    .unwrap()
                    .unwrap()
            };
            let actual = evaluate(&workspace, &mut scorer);
            let expected = evaluate(&eager, &mut SizedEdgeLengthScorer::new(receiver, true));
            let stale = evaluate(&workspace, &mut SizedEdgeLengthScorer::new(receiver, true));
            assert_eq!(actual.to_bits(), expected.to_bits());
            observed_stale_flow |= stale.to_bits() != expected.to_bits();
            assert_eq!(workspace.position(receiver), Some(Point { x: 100., y: 0. }));
        }
        assert!(
            observed_stale_flow,
            "the witness must distinguish live and stale flow rays"
        );
    }

    fn input() -> Graph {
        let mut input = Graph::default();
        for index in 0..4 {
            let mut node = Node::new(
                format!("n{index}"),
                Size {
                    width: 30.,
                    height: 20.,
                },
            );
            node.parent = (index > 0).then(|| NodeId(index - 1));
            input.add_node(node);
        }
        input
    }

    // Publication follows explicit canonical correspondence, including a nil
    // point and an owner mutation outside the candidate's own movement set.
    // Nested children must not be translated again through the projection.
    #[test]
    fn ordinary_publication_reads_live_remapped_owners_without_replaying_descendants() {
        let input = input();
        let mut owner = ArenaGraph::from_input(&input);
        let mapping_nodes = vec![NodeId(3), NodeId(1), NodeId(0), NodeId(2)];
        owner
            .set_owner_view_node_mapping(OwnerGraphId(0), mapping_nodes.clone())
            .unwrap();
        let mapping =
            CanonicalScoreMapping::new(owner.topology_arena_id().unwrap(), mapping_nodes, vec![]);
        let mut session =
            OwnerScoreSession::new(&mut owner, OwnerGraphId(0), mapping, None).unwrap();
        let mut workspace = ArenaGraph::from_input(&input);
        for index in 0..4 {
            workspace.set_position(
                NodeId(index),
                Point {
                    x: index as f64 * 11.,
                    y: -40.,
                },
            );
            session.owner_mut().set_position(
                NodeId(index),
                Point {
                    x: 70. + index as f64 * 11.,
                    y: -0.0,
                },
            );
        }
        mirror(&mut workspace, &session);
        for (local, &canonical) in session.current_mapping().nodes().iter().enumerate() {
            let expected = session.owner().position(canonical);
            assert_eq!(workspace.position(NodeId(local as u32)), expected);
            if let Some(expected) = expected {
                assert_eq!(
                    workspace.nodes[local].rect.origin.x.to_bits(),
                    expected.x.to_bits()
                );
                assert_eq!(
                    workspace.nodes[local].rect.origin.y.to_bits(),
                    expected.y.to_bits()
                );
            }
        }
        session
            .owner_mut()
            .set_position(NodeId(0), Point { x: -39.25, y: 1.5 });
        mirror(&mut workspace, &session);
        assert_eq!(
            workspace.position(NodeId(2)),
            Some(Point { x: -39.25, y: 1.5 })
        );
        // Nil is not a complete box projection: clearing a carrier preserves
        // the rect origin produced by the ancestor replay. Keep that behavior.
        session.owner_mut().clear_position(NodeId(1));
        session
            .owner_mut()
            .set_position(NodeId(3), Point { x: 400., y: 500. });
        let mut expected = workspace.clone();
        mirror_with_descendants(&mut expected, &session);
        mirror(&mut workspace, &session);
        for (actual, expected) in workspace.nodes.iter().zip(&expected.nodes) {
            assert_eq!(actual.position, expected.position);
            assert_eq!(actual.rect, expected.rect);
        }
    }

    #[test]
    fn explicit_external_child_reads_its_owner_outside_the_dense_mapping() {
        let input = input();
        let mut owner = ArenaGraph::from_input(&input);
        owner.set_position(NodeId(0), Point { x: 100., y: 200. });
        owner.set_position(NodeId(3), Point { x: -70.5, y: 82.25 });
        owner
            .set_owner_view_node_mapping(OwnerGraphId(0), vec![NodeId(0)])
            .unwrap();
        let mapping =
            CanonicalScoreMapping::new(owner.topology_arena_id().unwrap(), vec![NodeId(0)], vec![]);
        let session = OwnerScoreSession::new(&mut owner, OwnerGraphId(0), mapping, None).unwrap();
        let mut local = Graph::default();
        local.add_node(Node::new(
            "container",
            Size {
                width: 100.,
                height: 80.,
            },
        ));
        let mut workspace = ArenaGraph::from_input(&local);
        workspace.set_position(NodeId(0), Point { x: 10., y: 20. });
        let mut child = workspace.nodes[0].clone();
        child.tala_id += 1;
        child.owner_geometry_source = Some(NodeId(3));
        child.position = Some(Point { x: 30., y: 40. });
        child.rect.origin = child.position.unwrap();
        workspace
            .transaction_external_container_children
            .insert(workspace.nodes[0].tala_id, vec![child]);
        mirror(&mut workspace, &session);
        let child = &workspace
            .transaction_external_container_children
            .values()
            .next()
            .unwrap()[0];
        assert_eq!(child.position, session.owner().position(NodeId(3)));
        assert_eq!(child.rect.origin, child.position.unwrap());
    }

    #[test]
    fn nil_external_projection_preserves_its_origin_while_owner_moves() {
        let mut input = Graph::default();
        input.add_node(Node::new("container", Size { width: 100., height: 80. }));
        input.add_node(Node::new("child", Size { width: 10., height: 8. }));
        let mut owner = ArenaGraph::from_input(&input);
        owner.set_position(NodeId(0), Point { x: 100., y: 200. });
        owner.clear_position(NodeId(1));
        owner.set_owner_view_node_mapping(OwnerGraphId(0), vec![NodeId(0)]).unwrap();
        let mapping = CanonicalScoreMapping::new(owner.topology_arena_id().unwrap(), vec![NodeId(0)], vec![]);
        let mut session = OwnerScoreSession::new(&mut owner, OwnerGraphId(0), mapping, None).unwrap();
        let mut local = Graph::default();
        local.add_node(Node::new("container", Size { width: 100., height: 80. }));
        let mut workspace = ArenaGraph::from_input(&local);
        workspace.set_position(NodeId(0), Point { x: 10., y: 20. });
        let mut child = workspace.nodes[0].clone();
        child.tala_id += 1;
        child.owner_geometry_source = Some(NodeId(1));
        child.position = None;
        child.rect.origin = Point { x: -0., y: 40.25 };
        workspace.transaction_external_container_children.insert(workspace.nodes[0].tala_id, vec![child]);
        assert!(!complete_position_projection(&workspace, &session, false));
        assert!(complete_position_projection(&workspace, &session, true));
        for point in [Point { x: 100., y: 200. }, Point { x: -15.5, y: 3.25 }] {
            session.owner_mut().set_position(NodeId(0), point);
            let mut expected = workspace.clone();
            mirror_with_descendants(&mut expected, &session);
            mirror(&mut workspace, &session);
            let actual = workspace.transaction_external_container_children.values().next().unwrap().first().unwrap();
            let reference = expected.transaction_external_container_children.values().next().unwrap().first().unwrap();
            assert_eq!(actual.position, None);
            assert_eq!(actual.rect.origin.x.to_bits(), (-0.0f64).to_bits());
            assert_eq!(actual.rect.origin.y.to_bits(), reference.rect.origin.y.to_bits());
            assert_eq!(workspace.position(NodeId(0)), expected.position(NodeId(0)));
        }
        // A still-positioned external carrier must replay translation before
        // clearing its point, so it cannot use the new admission rule.
        workspace.transaction_external_container_children.values_mut().next().unwrap()[0].position = Some(Point { x: 1., y: 2. });
        assert!(!complete_position_projection(&workspace, &session, true));
        let mut expected = workspace.clone();
        session.owner_mut().set_position(NodeId(0), Point { x: 10., y: 20. });
        mirror_with_descendants(&mut expected, &session);
        mirror(&mut workspace, &session);
        let actual = &workspace.transaction_external_container_children.values().next().unwrap()[0];
        let reference = &expected.transaction_external_container_children.values().next().unwrap()[0];
        assert_eq!(actual.position, None);
        assert_eq!(actual.rect.origin, reference.rect.origin);
    }

    // An external child without a canonical source still needs the enclosing
    // frame's relative translation. It must keep the original bridge.
    #[test]
    fn incomplete_projection_keeps_external_child_translation() {
        let mut input = Graph::default();
        input.add_node(Node::new(
            "container",
            Size {
                width: 100.,
                height: 80.,
            },
        ));
        let mut owner = ArenaGraph::from_input(&input);
        owner.set_position(NodeId(0), Point { x: 100., y: 200. });
        let mapping =
            CanonicalScoreMapping::new(owner.topology_arena_id().unwrap(), vec![NodeId(0)], vec![]);
        let session = OwnerScoreSession::new(&mut owner, OwnerGraphId(0), mapping, None).unwrap();
        let mut workspace = ArenaGraph::from_input(&input);
        workspace.set_position(NodeId(0), Point { x: 10., y: 20. });
        let mut child = workspace.nodes[0].clone();
        child.tala_id += 1;
        child.position = Some(Point { x: 30., y: 40. });
        child.rect.origin = child.position.unwrap();
        child.owner_geometry_source = None;
        workspace
            .transaction_external_container_children
            .insert(workspace.nodes[0].tala_id, vec![child]);
        mirror(&mut workspace, &session);
        let child = &workspace
            .transaction_external_container_children
            .values()
            .next()
            .unwrap()[0];
        assert_eq!(child.position, Some(Point { x: 120., y: 220. }));
        assert_eq!(child.rect.origin, child.position.unwrap());
    }
}
