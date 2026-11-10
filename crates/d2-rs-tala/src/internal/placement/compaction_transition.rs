// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Transition from occupied cells to ordered, size-aware geometry.
//!
//! Axis ordering and snapping preserve the topology chosen by sizeless search
//! while preparing nonuniform rectangles for the sized optimizer.

use super::*;
use d2_rs_core::stable_sort::StableSort;

impl ArenaGraph {
    pub(super) fn ordered_along_axis(&self, horizontal: bool) -> Vec<NodeId> {
        // Go's compact pass receives the current Graph.Nodes slice. The arena
        // retains hidden sequence/cluster members for stable identity, but
        // those boxes are not independent placement nodes while a vessel is
        // active.
        let nodes = self.graph_node_order();
        self.ordered_subset_along_axis(&nodes, horizontal)
    }

    pub(super) fn ordered_subset_along_axis(
        &self,
        nodes: &[NodeId],
        horizontal: bool,
    ) -> Vec<NodeId> {
        let mut nodes = nodes.to_vec();
        nodes.stable_sort_by(|a, b| {
            let a_position = self.position(*a).unwrap();
            let b_position = self.position(*b).unwrap();
            let a_axis = if horizontal {
                a_position.x
            } else {
                a_position.y
            };
            let b_axis = if horizontal {
                b_position.x
            } else {
                b_position.y
            };
            a_axis.total_cmp(&b_axis).then_with(|| {
                self.nodes[a.0 as usize]
                    .tala_id
                    .cmp(&self.nodes[b.0 as usize].tala_id)
                    .then_with(|| a.cmp(b))
            })
        });
        nodes
    }

    // includeSizes=false visibility edges reduce to consecutive points on the
    // same row or column. This is the exact predicate used by the sizeless
    // half of recovered compaction.
    pub(super) fn visibility_predecessor(&self, node: NodeId, horizontal: bool) -> Option<NodeId> {
        let position = self.position(node)?;
        self.nodes
            .iter()
            .filter_map(|candidate| {
                let candidate_position = candidate.position?;
                let aligned = if horizontal {
                    candidate_position.y == position.y && candidate_position.x < position.x
                } else {
                    candidate_position.x == position.x && candidate_position.y < position.y
                };
                aligned.then_some((candidate.input_id, candidate_position))
            })
            .max_by(|(a_id, a), (b_id, b)| {
                let a_axis = if horizontal { a.x } else { a.y };
                let b_axis = if horizontal { b.x } else { b.y };
                a_axis.total_cmp(&b_axis).then_with(|| a_id.cmp(b_id))
            })
            .map(|(id, _)| id)
    }

    pub(super) fn sizeless_visibility_edges(
        &self,
        horizontal: bool,
        nodes: &[NodeId],
    ) -> BTreeMap<NodeId, NodeId> {
        let active: BTreeSet<_> = nodes.iter().copied().collect();
        nodes
            .iter()
            .filter_map(|node| {
                let position = self.position(*node)?;
                active
                    .iter()
                    .copied()
                    .filter_map(|candidate| {
                        let candidate_position = self.position(candidate)?;
                        let aligned = if horizontal {
                            candidate_position.y == position.y && candidate_position.x < position.x
                        } else {
                            candidate_position.x == position.x && candidate_position.y < position.y
                        };
                        aligned.then_some((candidate, candidate_position))
                    })
                    .max_by(|(a_id, a), (b_id, b)| {
                        let a_axis = if horizontal { a.x } else { a.y };
                        let b_axis = if horizontal { b.x } else { b.y };
                        a_axis.total_cmp(&b_axis).then_with(|| a_id.cmp(b_id))
                    })
                    .map(|(from, _)| (*node, from))
            })
            .collect()
    }

    pub(super) fn is_visibility_candidate(
        &self,
        from: NodeId,
        to: NodeId,
        horizontal: bool,
        include_sizes: bool,
        padding: f64,
    ) -> bool {
        let from_position = self.position(from).unwrap();
        let to_position = self.position(to).unwrap();
        if horizontal {
            if from_position.x >= to_position.x {
                return false;
            }
            if include_sizes {
                let from_height = self.active_node_size(from).height;
                let to_height = self.active_node_size(to).height;
                from_position.y <= to_position.y + to_height + padding
                    && to_position.y <= from_position.y + from_height + padding
            } else {
                from_position.y == to_position.y
            }
        } else {
            if from_position.y >= to_position.y {
                return false;
            }
            if include_sizes {
                let from_width = self.active_node_size(from).width;
                let to_width = self.active_node_size(to).width;
                from_position.x <= to_position.x + to_width + padding
                    && to_position.x <= from_position.x + from_width + padding
            } else {
                from_position.x == to_position.x
            }
        }
    }

    // Direct translation of Node.IsBlocked for the non-proxy arena nodes.
    pub(super) fn blocks_visibility(
        &self,
        blocker: NodeId,
        from: NodeId,
        to: NodeId,
        horizontal: bool,
        include_sizes: bool,
    ) -> bool {
        let blocker_position = self.position(blocker).unwrap();
        let from_position = self.position(from).unwrap();
        let to_position = self.position(to).unwrap();
        let blocker_size = self.active_node_size(blocker);
        let from_size = self.active_node_size(from);
        let to_size = self.active_node_size(to);
        if horizontal {
            let between = if include_sizes {
                blocker_position.x >= from_position.x + from_size.width
                    && blocker_position.x + blocker_size.width <= to_position.x
            } else {
                blocker_position.x >= from_position.x && blocker_position.x <= to_position.x
            };
            if !between {
                return false;
            }
            if include_sizes {
                blocker_position.y <= from_position.y.max(to_position.y)
                    && blocker_position.y + blocker_size.height
                        >= (from_position.y + from_size.height).min(to_position.y + to_size.height)
            } else {
                blocker_position.y <= from_position.y.max(to_position.y)
                    && blocker_position.y >= from_position.y.min(to_position.y)
            }
        } else {
            let between = if include_sizes {
                blocker_position.y >= from_position.y + from_size.height
                    && blocker_position.y + blocker_size.height <= to_position.y
            } else {
                blocker_position.y >= from_position.y && blocker_position.y <= to_position.y
            };
            if !between {
                return false;
            }
            if include_sizes {
                blocker_position.x <= from_position.x.max(to_position.x)
                    && blocker_position.x + blocker_size.width
                        >= (from_position.x + from_size.width).min(to_position.x + to_size.width)
            } else {
                blocker_position.x <= from_position.x.max(to_position.x)
                    && blocker_position.x >= from_position.x.min(to_position.x)
            }
        }
    }

    pub(super) fn visibility_edges(
        &self,
        horizontal: bool,
        include_sizes: bool,
    ) -> Vec<(NodeId, NodeId)> {
        self.visibility_edges_with_guard(horizontal, include_sizes, None)
            .expect("unguarded visibility scan cannot fail")
    }

    pub(super) fn visibility_edges_with_guard(
        &self,
        horizontal: bool,
        include_sizes: bool,
        mut guard: Option<&mut limits::WorkGuard<'_>>,
    ) -> Result<Vec<(NodeId, NodeId)>, limits::Error> {
        let mut edges = Vec::new();
        let nodes = self.graph_node_order();
        for &from in &nodes {
            if let Some(guard) = guard.as_deref_mut() {
                guard.step()?;
            }
            for &to in &nodes {
                if let Some(guard) = guard.as_deref_mut() {
                    guard.step()?;
                }
                if from == to {
                    continue;
                }
                // Recovered Graph.getVisibilityEdges always passes
                // node.getDeltaTo(otherNode, node.Box.TopLeft), regardless of
                // container ownership or includeSizes.
                let padding = self.spacing_delta_with_loops(from, to, self.position(from).unwrap());
                if !self.is_visibility_candidate(from, to, horizontal, include_sizes, padding) {
                    continue;
                }
                let mut blocked = false;
                for &blocker in &nodes {
                    if let Some(guard) = guard.as_deref_mut() {
                        guard.step()?;
                    }
                    if blocker != from
                        && blocker != to
                        && self.blocks_visibility(blocker, from, to, horizontal, include_sizes)
                    {
                        blocked = true;
                        break;
                    }
                }
                if !blocked {
                    edges.push((from, to));
                }
            }
        }
        if let Some(guard) = guard {
            guard.finish()?;
        }
        if crate::internal::trace_env_enabled("TALA_TRACE_VISIBILITY") {
            eprint!(
                "VISIBILITY_RUST horizontal={} includeSizes={} nodes={}",
                horizontal,
                include_sizes,
                nodes.len()
            );
            for (from, to) in &edges {
                eprint!(
                    " {}>{}",
                    self.nodes[from.0 as usize].tala_id, self.nodes[to.0 as usize].tala_id
                );
            }
            eprintln!();
        }
        Ok(edges)
    }

    pub(super) fn nearest_visibility_predecessor(
        &self,
        node: NodeId,
        horizontal: bool,
        include_sizes: bool,
        edges: &[(NodeId, NodeId)],
    ) -> Option<NodeId> {
        // Recovered Edges.getNearestFrom updates on a strict `>` comparison.
        // Equal trailing edges therefore retain the first visibility edge in
        // Graph.getVisibilityEdges order.
        let mut nearest = None;
        let mut largest_trailing_edge = f64::NEG_INFINITY;
        for (from, to) in edges.iter().copied() {
            if to != node {
                continue;
            }
            let position = self.position(from).unwrap();
            let trailing_edge = if horizontal {
                position.x
                    + if include_sizes {
                        self.active_node_size(from).width
                    } else {
                        0.0
                    }
            } else {
                position.y
                    + if include_sizes {
                        self.active_node_size(from).height
                    } else {
                        0.0
                    }
            };
            if trailing_edge > largest_trailing_edge {
                largest_trailing_edge = trailing_edge;
                nearest = Some(from);
            }
        }
        nearest
    }

    pub(super) fn transition_compact_axis(&mut self, horizontal: bool, factor: f64) {
        self.trace_transition_state(horizontal, true);
        let visibility = self.visibility_edges(horizontal, true);
        self.inflate_transition_axis(horizontal, factor, &visibility);
        self.trace_transition_state(horizontal, false);
    }

    pub(super) fn transition_compact_axis_checked(
        &mut self,
        horizontal: bool,
        factor: f64,
        context: &dyn limits::Context,
    ) -> Result<(), limits::Error> {
        self.trace_transition_state(horizontal, true);
        let mut guard = limits::WorkGuard::new(
            Some(context),
            "CompactionVisibility",
            limits::MAX_ENGINE_WORK_UNITS,
        )?;
        let visibility = self.visibility_edges_with_guard(horizontal, true, Some(&mut guard))?;
        self.inflate_transition_axis(horizontal, factor, &visibility);
        guard.check()?;
        self.trace_transition_state(horizontal, false);
        Ok(())
    }

    /// The full canonical Graph.Nodes workspace has real vessel/member
    /// pointers. Its movement expands Node.Graph's live maps rather than the
    /// induced projection's cached descendant slices.
    pub(super) fn transition_compact_canonical_axis_checked(
        &mut self,
        receiver: OwnerGraphId,
        horizontal: bool,
        factor: f64,
        context: &dyn limits::Context,
        seed: i64,
    ) -> Result<(), crate::LayoutError> {
        let work_error = |error| {
            super::container_orientation::layout_error_for_work(
                error,
                seed,
                "CompactionVisibility",
                limits::MAX_ENGINE_WORK_UNITS,
            )
        };
        let entry_error = |error| {
            super::container_orientation::layout_error_for_work(
                error,
                seed,
                "Compaction",
                limits::MAX_ENGINE_WORK_UNITS,
            )
        };
        if factor <= 0.0 || !factor.is_finite() {
            return Err(entry_error(limits::Error::Invalid(
                "TALA Compaction requires a finite positive factor".into(),
            )));
        }
        self.validate_owner_graph_topology(Some(receiver), Some(context), "Compaction")
            .map_err(entry_error)?;
        let mut checkpoint =
            compaction_checkpoint::CompactionCheckpoint::capture(self, receiver, context)
                .map_err(entry_error)?;
        let graph = checkpoint.graph();
        let mut guard = limits::WorkGuard::new(
            Some(context),
            "CompactionVisibility",
            limits::MAX_ENGINE_WORK_UNITS,
        )
        .map_err(work_error)?;
        let visibility = graph
            .visibility_edges_with_guard(horizontal, true, Some(&mut guard))
            .map_err(work_error)?;
        graph.inflate_transition_axis_with_move(
            horizontal,
            factor,
            &visibility,
            |graph, node, target| {
                let mapping = CanonicalScoreMapping::new(
                    graph
                        .topology_arena_id()
                        .expect("canonical workspace arena"),
                    (0..graph.nodes.len()).map(|i| NodeId(i as u32)).collect(),
                    graph
                        .owner_graph_view(receiver)
                        .expect("canonical receiver")
                        .edges
                        .read(),
                );
                let mut session = OwnerScoreSession::new(graph, receiver, mapping, None)
                    .map_err(|error| crate::LayoutError::PlacementTransaction(error.to_string()))?;
                session
                    .move_canonical_abs_with_children(node, target)
                    .map_err(|error| crate::LayoutError::PlacementTransaction(error.to_string()))
            },
        )?;
        // compaction.go checks ctx.Err directly after inflation; the
        // visibility WorkGuard has already finished before any move.
        if let Some(cause) = context.error() {
            return Err(entry_error(limits::Error::Context {
                location: "Compaction".into(),
                cause,
            }));
        }
        checkpoint.commit();
        Ok(())
    }

    fn trace_transition_state(&self, horizontal: bool, before: bool) {
        if crate::internal::trace_env_enabled("TALA_TRACE_TRANSITION_STATE")
            && self.nodes.len() == 4
        {
            eprintln!(
                "TRANSITION_STATE_RUST axis={} {}={:?}",
                if horizontal { "h" } else { "v" },
                if before { "before" } else { "after" },
                self.nodes
                    .iter()
                    .map(|node| (
                        self.nodes[node.input_id.0 as usize].tala_id,
                        self.position(node.input_id)
                    ))
                    .collect::<Vec<_>>()
            );
        }
    }

    fn inflate_transition_axis(
        &mut self,
        horizontal: bool,
        factor: f64,
        visibility: &[(NodeId, NodeId)],
    ) {
        self.inflate_transition_axis_with_move(
            horizontal,
            factor,
            visibility,
            |graph, node, target| {
                graph.move_node_abs_with_children(node, target);
                Ok::<(), std::convert::Infallible>(())
            },
        )
        .unwrap();
    }

    fn inflate_transition_axis_with_move<E>(
        &mut self,
        horizontal: bool,
        factor: f64,
        visibility: &[(NodeId, NodeId)],
        mut move_node: impl FnMut(&mut Self, NodeId, Point) -> Result<(), E>,
    ) -> Result<(), E> {
        let active_nodes = self.graph_node_order();
        let has_fixed_node = active_nodes
            .iter()
            .any(|node| self.nodes[node.0 as usize].fixed_top_left.is_some());
        for node in self.ordered_along_axis(horizontal) {
            let Some(anchor) =
                self.nearest_visibility_predecessor(node, horizontal, true, visibility)
            else {
                // inflateAlongAxis snaps otherwise-unconstrained nodes to the
                // preceding cell only when a fixed node is present. The
                // transition pass deliberately leaves fixed nodes themselves
                // untouched, but still normalizes their movable peers.
                if has_fixed_node && self.nodes[node.0 as usize].fixed_top_left.is_none() {
                    let current = self.position(node).unwrap();
                    let snapped = if horizontal {
                        Point {
                            x: (current.x / self.cell_size).floor() * self.cell_size,
                            y: current.y,
                        }
                    } else {
                        Point {
                            x: current.x,
                            y: (current.y / self.cell_size).floor() * self.cell_size,
                        }
                    };
                    move_node(self, node, snapped)?;
                }
                continue;
            };
            let current = self.position(node).unwrap();
            let floor =
                self.sized_compaction_floor(anchor, factor, horizontal, 60.0) * self.cell_size;
            let current_axis = if horizontal { current.x } else { current.y };
            if current_axis < floor {
                if crate::internal::trace_env_enabled("TALA_TRACE_TRANSITION_MOVES")
                    && (self.nodes.len() == 10 || self.nodes.len() == 4)
                {
                    eprintln!(
                        "TRANSITION_RUST axis={} node={} anchor={} from={:?} to={:?} floor={}",
                        if horizontal { "h" } else { "v" },
                        self.nodes[node.0 as usize].tala_id,
                        self.nodes[anchor.0 as usize].tala_id,
                        current,
                        if horizontal {
                            Point {
                                x: floor,
                                y: current.y,
                            }
                        } else {
                            Point {
                                x: current.x,
                                y: floor,
                            }
                        },
                        floor,
                    );
                }
                move_node(
                    self,
                    node,
                    if horizontal {
                        Point {
                            x: floor,
                            y: current.y,
                        }
                    } else {
                        Point {
                            x: current.x,
                            y: floor,
                        }
                    },
                )?;
            }
        }
        Ok(())
    }

    pub(super) fn transition_compact(&mut self) {
        self.transition_compact_axis(true, 3.0);
        self.transition_compact_axis(false, 3.0);
    }
}
