// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Original placement/moves.go. One operation guard spans all accepted work;
//! each actual move collects its own live descendants, as the source does.
use super::limits::{Context, OptimizationError, OptimizationWorkGuard};
use super::optimizer_spatial_index::OptimizerSpatialIndex;
use super::owner_points::SharedOwnerPoint;
use super::scoring_context::ScoreContext;
use super::*;

fn invalid(error: impl std::fmt::Display) -> OptimizationError {
    OptimizationError::Invalid(error.to_string())
}
fn step(
    owner: &mut ArenaGraph,
    guard: &mut OptimizationWorkGuard<'_>,
) -> Result<(), OptimizationError> {
    #[cfg(feature = "diagnostic-traces")]
    {
        guard.step_with_owner(owner)
    }
    #[cfg(not(feature = "diagnostic-traces"))]
    {
        let _ = owner;
        guard.step()
    }
}
fn finish(
    owner: &mut ArenaGraph,
    guard: &OptimizationWorkGuard<'_>,
) -> Result<(), OptimizationError> {
    #[cfg(feature = "diagnostic-traces")]
    {
        guard.finish_with_owner(owner)
    }
    #[cfg(not(feature = "diagnostic-traces"))]
    {
        let _ = owner;
        guard.finish()
    }
}
fn compare(a: f64, b: f64) -> std::cmp::Ordering {
    if (a - b).abs() < 0.0001 {
        std::cmp::Ordering::Equal
    } else if a < b {
        std::cmp::Ordering::Less
    } else {
        std::cmp::Ordering::Greater
    }
}
/// Uncharged numerical publication. Preserve any already-bound transport point
/// allocations; these carriers never substitute for canonical movement.
fn refresh(
    workspace: &mut ArenaGraph,
    session: &OwnerScoreSession<'_>,
) -> Result<(), OptimizationError> {
    for (index, &canonical) in session.current_mapping().nodes().iter().enumerate() {
        let node = NodeId(index as u32);
        let value = session.owner().position(canonical);
        if let (Some(value), Ok(point)) = (value, workspace.owner_node_point(node)) {
            if let Some(point) = point.shared_point() {
                point.set(value);
            } else {
                workspace.set_position(node, value);
            }
        } else if let Some(value) = value {
            workspace.set_position(node, value);
        } else {
            workspace.clear_position(node);
        }
        workspace.nodes[index].position = value;
        workspace.nodes[index].rect.origin =
            value.unwrap_or(session.owner().nodes[canonical.0 as usize].rect.origin);
    }
    session
        .refresh_explicit_owner_projections(workspace)
        .map_err(invalid)
}

pub(super) fn move_to_best(
    workspace: &mut ArenaGraph,
    session: &mut OwnerScoreSession<'_>,
    node: NodeId,
    points: impl Iterator<Item = Option<SharedOwnerPoint>>,
    sized: bool,
    context: &dyn Context,
    guard: Option<&mut OptimizationWorkGuard<'_>>,
) -> Result<bool, OptimizationError> {
    let guard = guard.ok_or_else(|| invalid("TALA moveNodeToBest requires a work guard"))?;
    finish(session.owner_mut(), guard)?;
    let canonical = session.canonical_node(node).map_err(invalid)?;
    let view = session.current_view();
    let cell = session
        .owner()
        .owner_graph_cell_size(view)
        .map_err(invalid)?;
    let symmetry_cost = cell * session.owner().owner_node_incident_count(canonical) as f64;
    let current = session
        .owner()
        .position(canonical)
        .expect("runtime error: invalid memory address or nil pointer dereference");
    let mut least = f64::INFINITY;
    let mut best = SharedOwnerPoint::new(current);
    let container = session
        .with_frame(|frame| frame.effective_container(Some(canonical)))
        .map_err(invalid)?
        .map_err(invalid)?;
    let mut origin = owner_placement::optimizer_owner_fixed_origin(
        session.owner_mut(),
        Some(view),
        container,
        guard,
    )
    .map_err(OptimizationError::from)?;
    if !sized {
        if let Some(origin) = origin.as_mut() {
            origin.x = (origin.x / cell).round();
            origin.y = (origin.y / cell).round();
        }
    }
    for handle in points {
        step(session.owner_mut(), guard)?;
        let handle = handle.ok_or_else(|| {
            invalid(format!(
                "TALA {} found a nil placement point",
                guard.location()
            ))
        })?;
        let point = handle.get();
        if origin.is_some_and(|origin| point.x < origin.x || point.y < origin.y) {
            continue;
        }
        if point != current && session.owner().position(canonical) != Some(point) {
            let graph = session
                .owner()
                .topology_node_owner(canonical)
                .ok_or_else(|| {
                    invalid(format!(
                        "TALA {} movement check requires a node, graph, and point",
                        guard.location()
                    ))
                })?;
            let mut index = OptimizerSpatialIndex::default();
            let (_, occupied, result) =
                index.is_occupied(session.owner_mut(), graph, &handle, guard);
            result?;
            if occupied {
                continue;
            }
            if sized
                && index.does_overlap(
                    session.owner_mut(),
                    Some(canonical),
                    Some(&handle),
                    &container_slices::SharedNodeSlice::nil(),
                    guard,
                )?
            {
                continue;
            }
        }
        session
            .owner_mut()
            .optimizer_move_node_abs(Some(canonical), point, guard)?;
        refresh(workspace, session)?;
        let abductions = if sized {
            session.abductions().map_or(0, |slice| slice.records.len())
        } else {
            0
        };
        optimizer_scoring_work::charge(
            session.owner_mut(),
            Some(canonical),
            abductions,
            sized,
            guard,
        )?;
        let score_context = ScoreContext::new(context);
        let mut distance = if sized {
            session
                .with_frame(|frame| {
                    workspace.sized_edge_length_with_owner_routes_checked(
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
            // Direct NodeEdgeLength prepares anew on each call, rather than retaining
            // NodeEdgeLengthScorer's sweep-wide preparation across candidates.
            let mut scorer = scoring::OwnerSizelessEdgeLengthScorer::new(node, true);
            let result = session.score_sizeless_owner(&mut scorer, workspace, score_context);
            scorer.close();
            result.map_err(compaction_candidates::optimization_error)?
        };
        if sized {
            if least != f64::INFINITY
                && compare(distance - symmetry_cost, least) == std::cmp::Ordering::Greater
            {
                continue;
            }
            let crossing = workspace
                .column_to_column_crossing_cost_checked(node, false, score_context)
                .map_err(compaction_candidates::optimization_error)?;
            let symmetry = workspace
                .sized_symmetry_checked(node, true, score_context)
                .map_err(compaction_candidates::optimization_error)?;
            distance += crossing;
            distance = (-symmetry).mul_add(symmetry_cost, distance);
        }
        match compare(distance, least) {
            std::cmp::Ordering::Less => {
                least = distance;
                best = handle;
            }
            std::cmp::Ordering::Equal if handle.get() == current => {
                least = distance;
                best = handle;
            }
            _ => {}
        }
    }
    #[cfg(feature = "diagnostic-traces")]
    let cause = context.error_with_owner(session.owner_mut());
    #[cfg(not(feature = "diagnostic-traces"))]
    let cause = context.error();
    if let Some(cause) = cause {
        return Err(OptimizationError::Context {
            location: "EdgeLength".into(),
            cause,
        });
    }
    if least == f64::INFINITY {
        return Err(invalid(
            "sizedOptimizer.moveNodeToBest: could not find any placement",
        ));
    }
    session
        .owner_mut()
        .optimizer_move_node_abs(Some(canonical), best.get(), guard)?;
    refresh(workspace, session)?;
    finish(session.owner_mut(), guard)?;
    Ok(session.owner().position(canonical) != Some(current))
}

impl ArenaGraph {
    /// Canonical owning entry for moves.go. Numerical scope transports need an
    /// explicit canonical mapping and must call move_to_best with that session.
    pub(super) fn move_node_to_best_compaction_with_context(
        &mut self,
        receiver: OwnerGraphId,
        node: NodeId,
        points: &[Option<SharedOwnerPoint>],
        sized: bool,
        context: &dyn Context,
        guard: Option<&mut OptimizationWorkGuard<'_>>,
    ) -> Result<bool, OptimizationError> {
        if guard.is_none() {
            return Err(invalid("TALA moveNodeToBest requires a work guard"));
        }
        self.initialize_shared_topology();
        let mapping = CanonicalScoreMapping::new(
            self.topology_arena_id().unwrap(),
            (0..self.nodes.len()).map(|i| NodeId(i as u32)).collect(),
            (0..self.edge_records.len())
                .map(|i| EdgeId(i as u32))
                .collect(),
        );
        let mut workspace = self.clone();
        let mut session = OwnerScoreSession::new(self, receiver, mapping, None).map_err(invalid)?;
        move_to_best(
            &mut workspace,
            &mut session,
            node,
            points.iter().cloned(),
            sized,
            context,
            guard,
        )
    }
}

/// Keep the numerical transport separate from the actual graph/point owner.
/// The caller owns the one shared search guard for every pass and node.
pub(super) fn current(
    graph: &mut ArenaGraph,
    session: Option<&mut OwnerScoreSession<'_>>,
    node: NodeId,
    points: &[Point],
    sized: bool,
    context: &dyn Context,
    guard: &mut OptimizationWorkGuard<'_>,
) -> Result<bool, OptimizationError> {
    let points = points
        .iter()
        .copied()
        .map(|point| Some(SharedOwnerPoint::new(point)));
    if let Some(session) = session {
        session
            .update_current_member_positions(graph)
            .map_err(invalid)?;
        move_to_best(graph, session, node, points, sized, context, Some(guard))
    } else {
        graph.initialize_shared_topology();
        // CopyEntitiesFrom shares requested Source Directions. A numerical
        // default direction is not a present entry in that semantic map.
        graph.bind_owner_placement_directions();
        let mapping = CanonicalScoreMapping::new(
            graph.topology_arena_id().unwrap(),
            (0..graph.nodes.len()).map(|i| NodeId(i as u32)).collect(),
            (0..graph.edge_records.len())
                .map(|i| EdgeId(i as u32))
                .collect(),
        );
        let mut workspace = graph.clone();
        // node_placement.go passes AbductEdges' allocated slice even with zero rows.
        let abductions = if sized && graph.has_sized_edge_abductions() {
            let records = graph.sized_edge_abductions.clone();
            Some(
                graph
                    .from_sized_owner_abductions(&records, true)
                    .map_err(invalid)?,
            )
        } else {
            None
        };
        let mut session =
            OwnerScoreSession::new(graph, OwnerGraphId(0), mapping, abductions.as_ref())
                .map_err(invalid)?;
        move_to_best(
            &mut workspace,
            &mut session,
            node,
            points,
            sized,
            context,
            Some(guard),
        )
    }
}
