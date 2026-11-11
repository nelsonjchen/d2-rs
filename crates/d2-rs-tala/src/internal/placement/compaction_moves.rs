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
    // moves.go changes only geometry between scores. Ordinary scopes can use
    // edge_length_scorer.go's fixed-topology preparation here too: Score
    // replays every preparation poll and reads current owner geometry. Keep
    // fresh preparation for projected aggregate scopes and custom contexts.
    let score_context = ScoreContext::new(context);
    let mut scorer = (sized
        && score_context.stable_owner()
        && guard.context().stable_sized_owner()
        && super::sized::movement_adapter::owner_edge_score_with_inert_external(
            workspace, session, node, true,
        ))
    .then(|| {
        let mut scorer = scoring::SizedEdgeLengthScorer::new(node, true);
        scorer.use_stable_owner_obstructions();
        scorer.use_owner_carrier_positions();
        scorer
    });
    // NodeEdgeLength's sizeless preparation also depends only on topology.
    // moves.go changes geometry between candidates; retain preparation only
    // when both contexts leave the owner unchanged at their polls.
    let mut sizeless_scorer = (!sized
        && score_context.stable_owner()
        && guard.context().stable_sized_owner())
    .then(|| {
        let mut scorer = scoring::OwnerSizelessEdgeLengthScorer::new(node, true);
        scorer.assume_stable_topology();
        scorer
    });
    let mut frame_validated = false;
    // This index is never rebuilt, so occupancy always follows the source's
    // full scan. Reuse only overlap scratch; each call captures the current
    // member header and starts a fresh spacing generation.
    let mut index = OptimizerSpatialIndex::default();
    // moves.go scores the owner directly. A stable scorer needs no numerical
    // publication until a sized candidate reaches crossing/symmetry. Refresh
    // on every exit, including cancellation, so the transport still exposes
    // the last completed owner move at the original boundary.
    let defer_publication = scorer.is_some() || sizeless_scorer.is_some();
    let mut pending_publication = false;
    let result = (|| {
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
            pending_publication = defer_publication;
            if !defer_publication {
                refresh(workspace, session)?;
                pending_publication = false;
            }
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
            let mut distance = if sized {
                let reuse_scorer = scorer.is_some();
                let score = |frame: &OwnerScoreFrame<'_>| {
                    if let Some(scorer) = scorer.as_mut() {
                        scorer.score_owner_routes(
                            workspace,
                            None,
                            frame.owner(),
                            canonical,
                            Some(frame),
                            score_context,
                        )
                    } else {
                        workspace.sized_edge_length_with_owner_routes_checked(
                            node,
                            true,
                            None,
                            frame.owner(),
                            canonical,
                            Some(frame),
                            score_context,
                        )
                    }
                };
                (if reuse_scorer {
                    session.with_frame_reusing_mapping_validation(&mut frame_validated, score)
                } else {
                    session.with_frame(score)
                })
                .map_err(invalid)?
                .map_err(compaction_candidates::optimization_error)?
            } else {
                if let Some(scorer) = sizeless_scorer.as_mut() {
                    session.score_sizeless_owner(scorer, workspace, score_context)
                } else {
                    let mut scorer = scoring::OwnerSizelessEdgeLengthScorer::new(node, true);
                    let result =
                        session.score_sizeless_owner(&mut scorer, workspace, score_context);
                    scorer.close();
                    result
                }
                .map_err(compaction_candidates::optimization_error)?
            };
            if sized {
                if least != f64::INFINITY
                    && compare(distance - symmetry_cost, least) == std::cmp::Ordering::Greater
                {
                    continue;
                }
                if pending_publication {
                    pending_publication = false;
                    refresh(workspace, session)?;
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
        pending_publication = false;
        refresh(workspace, session)?;
        finish(session.owner_mut(), guard)?;
        Ok(session.owner().position(canonical) != Some(current))
    })();
    if pending_publication {
        refresh(workspace, session)?;
    }
    result
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::internal::limits::ContextError;
    use std::cell::Cell;

    #[test]
    fn stable_compaction_scoring_preserves_moves_work_and_cancellation() {
        struct Count {
            stable: bool,
            polls: Cell<usize>,
            cancel: Option<usize>,
            panic: bool,
        }
        impl Context for Count {
            fn stable_sized_owner(&self) -> bool {
                self.stable
            }
            fn done(&self) -> Option<bool> {
                None
            }
            fn error(&self) -> Option<ContextError> {
                let count = self.polls.get() + 1;
                self.polls.set(count);
                if self.panic && self.cancel.is_some_and(|cancel| count >= cancel) {
                    panic!("compaction poll");
                }
                self.cancel
                    .filter(|&cancel| count >= cancel)
                    .map(|_| ContextError::Cancelled)
            }
        }
        let mut input = Graph::default();
        for index in 0..5 {
            input.add_node(crate::Node::new(
                format!("n{index}"),
                Size {
                    width: 30.,
                    height: 20.,
                },
            ));
        }
        for (from, to) in [(0, 1), (0, 2), (0, 3), (1, 4)] {
            input.add_edge(crate::Edge {
                source: NodeId(from),
                target: NodeId(to),
            });
        }
        // Compare the retained sweep to fresh NodeEdgeLength calls, including
        // cancellation and unwinding at each poll, plus scoring-work failure.
        for (sized, scoring_mode, bound) in [false, true].into_iter().flat_map(|sized| {
            (0..3).flat_map(move |mode| [false, true].map(|bound| (sized, mode, bound)))
        }) {
            let mut total_polls = 0;
            for cancel in std::iter::once(None).chain((3..).map(Some)) {
                if cancel.is_some_and(|cancel| cancel > total_polls) {
                    break;
                }
                for (limit, panic) in [(1_000_000, false), (10, false), (1_000_000, true)] {
                    let mut outcomes = Vec::new();
                    for stable in [false, true] {
                        let mut owner = ArenaGraph::from_input(&input);
                        owner.cell_size = 99.;
                        owner
                            .set_owner_graph_cell_size(OwnerGraphId(0), 99.)
                            .unwrap();
                        for (index, point) in [
                            Point { x: 0., y: 0. },
                            Point { x: 300., y: 0. },
                            Point { x: 0., y: 300. },
                            Point { x: -300., y: 0. },
                            Point { x: 300., y: 300. },
                        ]
                        .into_iter()
                        .enumerate()
                        {
                            owner.set_position(NodeId(index as u32), point);
                        }
                        if bound {
                            // Go's TopLeft pointers can alias an unrelated
                            // node. Such a leaf still needs full publication.
                            let point = owner.allocate_owner_node_point(owner.position(NodeId(0)));
                            owner
                                .bind_owner_node_point(NodeId(0), point.clone())
                                .unwrap();
                            owner.bind_owner_node_point(NodeId(4), point).unwrap();
                        }
                        let context = Count {
                            stable,
                            polls: Cell::new(0),
                            cancel,
                            panic,
                        };
                        let mut guard =
                            OptimizationWorkGuard::new(Some(&context), "compaction-sweep", limit)
                                .unwrap();
                        let points = [
                            Point { x: 0., y: 0. },
                            Point { x: 99., y: 99. },
                            Point { x: -99., y: 99. },
                            Point { x: 0., y: -99. },
                            Point { x: 0., y: 0. },
                        ]
                        .map(|point| Some(SharedOwnerPoint::new(point)));
                        let mapping = CanonicalScoreMapping::new(
                            owner.topology_arena_id().unwrap(),
                            (0..owner.nodes.len()).map(|i| NodeId(i as u32)).collect(),
                            (0..owner.edge_records.len())
                                .map(|i| EdgeId(i as u32))
                                .collect(),
                        );
                        let mut workspace = owner.clone();
                        if scoring_mode == 0 {
                            // The first completed movement must reconcile an
                            // unrelated stale carrier before symmetry reads it.
                            workspace.set_position(NodeId(3), Point { x: -600., y: 99. });
                        }
                        let mut session =
                            OwnerScoreSession::new(&mut owner, OwnerGraphId(0), mapping, None)
                                .unwrap();
                        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            if scoring_mode == 0 {
                                move_to_best(
                                    &mut workspace,
                                    &mut session,
                                    NodeId(0),
                                    points.iter().cloned(),
                                    sized,
                                    &context,
                                    Some(&mut guard),
                                )
                                .map(u64::from)
                            } else {
                                // Fresh NodesEdgeLength with an exclusive owner
                                // and with its directly borrowed frame.
                                compaction_scoring::nodes_edge_length(
                                    &mut workspace,
                                    &[NodeId(0), NodeId(1)],
                                    sized,
                                    &mut (scoring_mode == 1).then_some(&mut session),
                                    &context,
                                )
                                .map(f64::to_bits)
                            }
                            .map_err(|error| error.to_string())
                        }))
                        .unwrap_or_else(|_| Err("compaction poll panic".into()));
                        drop(session);
                        // An unwind leaves the workspace unspecified: the seed
                        // recovery discards it without reading it.
                        let unwound = result
                            .as_ref()
                            .is_err_and(|error| error == "compaction poll panic");
                        let workspace_positions = (0..5)
                            .filter(|_| !unwound)
                            .map(|index| {
                                workspace
                                    .position(NodeId(index))
                                    .map(|point| (point.x.to_bits(), point.y.to_bits()))
                            })
                            .collect::<Vec<_>>();
                        let positions = (0..5)
                            .map(|index| {
                                owner
                                    .position(NodeId(index))
                                    .map(|point| (point.x.to_bits(), point.y.to_bits()))
                            })
                            .collect::<Vec<_>>();
                        if cancel.is_none() && limit == 1_000_000 {
                            assert!(result.is_ok());
                            total_polls = context.polls.get();
                        }
                        outcomes.push((
                            result,
                            positions,
                            workspace_positions,
                            guard.used(),
                            context.polls.get(),
                        ));
                    }
                    assert_eq!(
                        outcomes[0], outcomes[1],
                        "sized={sized}, mode={scoring_mode}, bound={bound}, cancel={cancel:?}, limit={limit}, panic={panic}"
                    );
                }
            }
            assert!(total_polls > 3);
        }
    }
}
