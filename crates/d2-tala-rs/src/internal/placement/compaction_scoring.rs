// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! compaction.go uses the checked NodesEdgeLength and NodesSymmetry readers.
use super::limits::OptimizationError;
use super::scoring_context::ScoreContext;
use super::*;
fn invalid(e: impl std::fmt::Display) -> OptimizationError {
    OptimizationError::Invalid(e.to_string())
}
pub(super) fn nodes_edge_length(
    graph: &mut ArenaGraph,
    nodes: &[NodeId],
    sized: bool,
    session: &mut Option<&mut OwnerScoreSession<'_>>,
    context: &dyn limits::Context,
) -> Result<f64, OptimizationError> {
    if session.is_none() {
        graph.initialize_shared_topology();
        graph.bind_owner_placement_directions();
    }
    if let Some(session) = session.as_deref_mut() {
        session
            .update_current_member_positions(graph)
            .map_err(invalid)?;
    }
    let score_context = ScoreContext::new(context);
    score_context
        .check()
        .map_err(compaction_candidates::optimization_error)?;
    let mut total = 0.;
    for &node in nodes {
        let value = if let Some(session) = session.as_deref_mut() {
            if sized {
                let canonical = session.canonical_node(node).map_err(invalid)?;
                session
                    .with_frame(|frame| {
                        graph.sized_edge_length_with_owner_routes_checked(
                            node,
                            true,
                            None,
                            frame.owner(),
                            canonical,
                            Some(frame),
                            score_context,
                        )
                    })
                    .map_err(invalid)?
                    .map_err(compaction_candidates::optimization_error)?
            } else {
                let mut scorer = scoring::OwnerSizelessEdgeLengthScorer::new(node, true);
                let value = session.score_sizeless_owner(&mut scorer, graph, score_context);
                scorer.close();
                value.map_err(compaction_candidates::optimization_error)?
            }
        } else {
            let mapping = (0..graph.nodes.len())
                .map(|i| NodeId(i as u32))
                .collect::<Vec<_>>();
            let edges = (0..graph.edge_records.len())
                .map(|i| EdgeId(i as u32))
                .collect::<Vec<_>>();
            let abductions = if sized && graph.has_sized_edge_abductions() {
                let rows = graph.sized_edge_abductions.clone();
                Some(
                    graph
                        .from_sized_owner_abductions(&rows, true)
                        .map_err(invalid)?,
                )
            } else {
                None
            };
            let mut frame = OwnerScoreFrame::new_with_edge_mapping(
                graph,
                graph.topology_arena_id().expect("bound compaction owner"),
                OwnerGraphId(0),
                &mapping,
                &edges,
            )
            .map_err(invalid)?;
            if let Some(rows) = &abductions {
                frame = frame.with_abductions(rows).map_err(invalid)?;
            }
            if sized {
                graph
                    .sized_edge_length_with_owner_routes_checked(
                        node,
                        true,
                        None,
                        graph,
                        node,
                        Some(&frame),
                        score_context,
                    )
                    .map_err(compaction_candidates::optimization_error)?
            } else {
                let mut scorer = scoring::OwnerSizelessEdgeLengthScorer::new(node, true);
                let value = scorer.score(graph, &frame, score_context);
                scorer.close();
                value.map_err(compaction_candidates::optimization_error)?
            }
        };
        total += value;
    }
    score_context
        .check()
        .map_err(compaction_candidates::optimization_error)?;
    Ok(total)
}
pub(super) fn nodes_symmetry(
    graph: &ArenaGraph,
    nodes: &[NodeId],
    context: &dyn limits::Context,
) -> Result<f64, OptimizationError> {
    let score_context = ScoreContext::new(context);
    score_context
        .check()
        .map_err(compaction_candidates::optimization_error)?;
    let mut total = 0.;
    for (i, &node) in nodes.iter().enumerate() {
        if i % 64 == 0 {
            score_context
                .check()
                .map_err(compaction_candidates::optimization_error)?;
        }
        total += graph
            .sized_symmetry_checked(node, true, score_context)
            .map_err(compaction_candidates::optimization_error)?;
    }
    score_context
        .check()
        .map_err(compaction_candidates::optimization_error)?;
    Ok(total)
}
