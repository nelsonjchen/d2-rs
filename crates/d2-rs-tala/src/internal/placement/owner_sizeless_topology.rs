// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Validate retained sizeless preparation without rebuilding its closure.
use super::*;

/// All mapped carriers and every discovered ancestor are retained by the
/// first score. Unchanged mappings and parent fields imply the same ancestor
/// closure, including its first-visit order. Compare live records in place;
/// geometry stays live in the scorer. A difference takes the original builder
/// so malformed topology keeps its original error and validation order.
pub(super) fn unchanged(
    saved: &OwnerSizelessTopology,
    workspace: &ArenaGraph,
    frame: &OwnerScoreFrame<'_>,
    local_node: NodeId,
) -> bool {
    matches_current(saved, workspace, frame, local_node).unwrap_or(false)
}

fn matches_current(
    saved: &OwnerSizelessTopology,
    workspace: &ArenaGraph,
    frame: &OwnerScoreFrame<'_>,
    local_node: NodeId,
) -> Result<bool, limits::Error> {
    if frame.abductions().is_some_and(|slice| slice.present)
        || saved.arena != frame.source_arena()
        || saved.mapping_view != frame.mapping_view()
        || saved.node_mapping != frame.local_to_canonical()
        || saved.edge_mapping != frame.local_to_canonical_edges()
        || workspace.nodes.len() != saved.node_mapping.len()
        || workspace.edge_records.len() != saved.edge_mapping.len()
        || saved.node
            != frame
                .canonical_node(local_node)
                .map_err(owner_score_error)?
    {
        return Ok(false);
    }
    for (carrier, &canonical) in workspace.nodes.iter().zip(&saved.node_mapping) {
        if carrier.tala_id != frame.node(canonical).map_err(owner_score_error)?.tala_id {
            return Ok(false);
        }
    }
    for saved in &saved.nodes {
        let actual = frame.node(saved.node).map_err(owner_score_error)?;
        let receiver = frame.graph_id(saved.node).map_err(owner_score_error)?;
        let direction = receiver
            .map(|receiver| {
                frame
                    .owner()
                    .owner_graph_direction_entry(receiver, actual.container)
            })
            .transpose()
            .map_err(owner_score_error)?
            .flatten();
        if saved.tala_id != actual.tala_id
            || saved.container != actual.container
            || saved.receiver != receiver
            || saved.direction_entry != direction
            || saved.is_table != (actual.shape == ShapeKind::SqlTable)
        {
            return Ok(false);
        }
        let mut index = 0;
        let mut equal = true;
        match frame
            .owner()
            .for_each_owner_node_incident(saved.node, |edge| {
                equal &= saved.edges.get(index) == Some(edge);
                index += 1;
            }) {
            Ok(()) => {
                if !equal || index != saved.edges.len() {
                    return Ok(false);
                }
            }
            Err(OwnerTopologyError::UnboundIncidentHeader(_)) => {
                if owner_score_incidents(frame.owner(), saved.node)? != saved.edges {
                    return Ok(false);
                }
            }
            Err(error) => return Err(owner_score_error(error)),
        }
    }
    for (carrier, saved) in workspace.edge_records.iter().zip(&saved.edges) {
        let actual = frame.edge(saved.edge).map_err(owner_score_error)?;
        if carrier.tala_id != actual.tala_id
            || carrier.source_arrow != actual.source_arrow
            || carrier.target_arrow != actual.target_arrow
            || carrier.is_invisible() != actual.is_invisible()
            || saved.tala_id != actual.tala_id
            || saved.source_arrow != actual.source_arrow
            || saved.target_arrow != actual.target_arrow
            || saved.invisible != actual.is_invisible()
            || saved.from
                != frame
                    .owner()
                    .owner_edge_endpoint(saved.edge, false)
                    .map_err(owner_score_error)?
            || saved.to
                != frame
                    .owner()
                    .owner_edge_endpoint(saved.edge, true)
                    .map_err(owner_score_error)?
        {
            return Ok(false);
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retained_topology_reads_geometry_live_and_rejects_unrelated_member_changes() {
        let mut input = Graph::default();
        let nodes = ["moving", "adjacent", "other"].map(|id| {
            input.add_node(crate::Node::new(
                id,
                Size {
                    width: 10.0,
                    height: 10.0,
                },
            ))
        });
        let edge = input.add_edge(crate::Edge {
            source: nodes[0],
            target: nodes[1],
        });
        let mut owner = ArenaGraph::from_input(&input);
        owner.initialize_shared_topology();
        for (index, node) in nodes.into_iter().enumerate() {
            owner.set_position(
                node,
                Point {
                    x: index as f64 * 10.0,
                    y: 0.0,
                },
            );
        }
        let mut workspace = owner.clone();
        let arena = owner.topology_arena_id().unwrap();
        let edges = [edge];
        let mut scorer = OwnerSizelessEdgeLengthScorer::new(NodeId(0), true);
        let score = |scorer: &mut OwnerSizelessEdgeLengthScorer,
                     owner: &ArenaGraph,
                     workspace: &ArenaGraph| {
            let frame = OwnerScoreFrame::new_with_edge_mapping(
                owner,
                arena,
                OwnerGraphId(0),
                &nodes,
                &edges,
            )
            .unwrap();
            scorer.score(workspace, &frame, ScoreContext::none())
        };
        // Stable sweeps still admit the entire initial owner/workspace, even
        // though they retain only incident preparation after that admission.
        for fault in 0..3 {
            let mut invalid_owner = owner.clone();
            let mut invalid_workspace = workspace.clone();
            match fault {
                0 => invalid_owner.nodes[nodes[2].0 as usize].container = Some(nodes[2]),
                1 => invalid_workspace.nodes[2].tala_id += 1000,
                _ => {
                    invalid_workspace.edge_records[0].source_arrow =
                        !invalid_workspace.edge_records[0].source_arrow
                }
            }
            let mut errors = Vec::new();
            for stable in [false, true] {
                let mut scorer = OwnerSizelessEdgeLengthScorer::new(NodeId(0), true);
                if stable {
                    scorer.assume_stable_topology();
                }
                errors.push(
                    score(&mut scorer, &invalid_owner, &invalid_workspace)
                        .unwrap_err()
                        .to_string(),
                );
                assert!(scorer.prepared.is_none());
            }
            assert_eq!(errors[0], errors[1], "initial admission fault {fault}");
        }
        let first = score(&mut scorer, &owner, &workspace).unwrap();
        owner.set_position(nodes[1], Point { x: 30.0, y: 0.0 });
        workspace.nodes[1].position = Some(Point {
            x: -900.0,
            y: 500.0,
        });
        assert_ne!(score(&mut scorer, &owner, &workspace).unwrap(), first);

        // Even a member with no incident edge to the scored node belongs to
        // the retained topology contract. Do not prune its validation away.
        owner.nodes[nodes[2].0 as usize].shape = ShapeKind::SqlTable;
        let error = score(&mut scorer, &owner, &workspace).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("topology changed during its movement sweep")
        );

        // Fall back to the original constructor's specific malformed-input
        // error, rather than replacing it with a generic mismatch.
        owner.nodes[nodes[2].0 as usize].container = Some(nodes[2]);
        let error = score(&mut scorer, &owner, &workspace).unwrap_err();
        assert!(error.to_string().contains("container ancestry is cyclic"));
    }
}
