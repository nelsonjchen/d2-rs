// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Compaction rules for real, size-aware node rectangles.
//!
//! The routines compute movement floors and trial axis translations while
//! respecting dimensions, containers, aggregates, and fixed geometry.

use super::limits::OptimizationError;
use super::*;

// Dense placement workspaces also retain projected child carriers. Save only
// the positions that movement can touch, without replacing node identities,
// sizes, routes, or scoring caches. These are numerical workspace copies; the
// separate owner snapshot below preserves the actual point allocations.
pub(super) struct CompactionWorkspacePositions {
    bound_points: Vec<(NodeId, shared_topology::OwnerNodePointSnapshot)>,
    positions: Vec<(Option<Point>, Point)>,
    pending: BTreeMap<usize, Point>,
}
impl CompactionWorkspacePositions {
    pub(super) fn capture(graph: &ArenaGraph) -> Self {
        Self {
            bound_points: (0..graph.nodes.len())
                .filter_map(|i| {
                    let n = NodeId(i as u32);
                    graph.owner_node_point(n).ok().map(|p| (n, p.snapshot()))
                })
                .collect(),
            positions: graph
                .nodes
                .iter()
                .chain(graph.transaction_external_containers.iter())
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
                .map(|node| (node.position, node.rect.origin))
                .collect(),
            pending: graph.pending_cluster_vessel_positions.clone(),
        }
    }
    pub(super) fn restore_projections(&self, graph: &mut ArenaGraph) {
        let count = graph.nodes.len();
        let nodes = graph
            .transaction_external_containers
            .iter_mut()
            .chain(
                graph
                    .transaction_external_container_children
                    .values_mut()
                    .flatten(),
            )
            .chain(
                graph
                    .transaction_external_aggregate_children
                    .values_mut()
                    .flatten(),
            );
        for (node, &(position, origin)) in nodes.zip(self.positions.iter().skip(count)) {
            node.position = position;
            node.rect.origin = origin;
        }
        graph
            .pending_cluster_vessel_positions
            .clone_from(&self.pending);
    }
    pub(super) fn restore(&self, graph: &mut ArenaGraph) {
        for (node, point) in &self.bound_points {
            graph
                .restore_owner_node_point(*node, point)
                .expect("captured workspace point");
        }
        let nodes = graph
            .nodes
            .iter_mut()
            .chain(graph.transaction_external_containers.iter_mut())
            .chain(
                graph
                    .transaction_external_container_children
                    .values_mut()
                    .flatten(),
            )
            .chain(
                graph
                    .transaction_external_aggregate_children
                    .values_mut()
                    .flatten(),
            );
        for (node, &(position, origin)) in nodes.zip(&self.positions) {
            node.position = position;
            node.rect.origin = origin;
        }
        graph
            .pending_cluster_vessel_positions
            .clone_from(&self.pending);
    }
}

impl ArenaGraph {
    fn compaction_edge_length_with_owner_nears(
        &self,
        node: NodeId,
        cache: Option<&[Vec<NodeId>]>,
        session: &mut Option<&mut OwnerScoreSession>,
    ) -> Result<f64, OptimizationError> {
        if let Some(session) = session.as_deref_mut() {
            session
                .update_current_member_positions(self)
                .map_err(|error| OptimizationError::Invalid(error.to_string()))?;
            let canonical = session
                .canonical_node(node)
                .map_err(|error| OptimizationError::Invalid(error.to_string()))?;
            // Go compaction calls the same NodeEdgeLength reader as local
            // optimization (compaction.go233/294). Lend the actual owner
            // routes and Cluster abductions here as well as in candidate scoring.
            session
                .with_frame(|frame| {
                    self.sized_edge_length_with_owner_routes(
                        node,
                        true,
                        cache,
                        frame.owner(),
                        canonical,
                        Some(frame),
                    )
                })
                .map_err(|error| OptimizationError::Invalid(error.to_string()))?
                .map_err(|error| OptimizationError::Invalid(error.to_string()))
        } else {
            Ok(self.sized_edge_length_with_cache(node, true, cache))
        }
    }

    fn compaction_nodes_edge_length_with_owner_nears(
        &self,
        nodes: &[NodeId],
        session: &mut Option<&mut OwnerScoreSession>,
    ) -> Result<f64, OptimizationError> {
        if session.is_none() {
            return Ok(self.nodes_sized_edge_length(nodes));
        }
        let mut total = 0.0;
        for node in nodes.iter().copied() {
            total += self.compaction_edge_length_with_owner_nears(node, None, session)?;
        }
        Ok(total)
    }

    pub(super) fn has_current_fixed_node(&self) -> bool {
        self.graph_node_order()
            .iter()
            .any(|node| self.nodes[node.0 as usize].fixed_top_left.is_some())
    }

    pub(super) fn sized_compaction_floor_decrease(
        &self,
        root: NodeId,
        globally_furthest: NodeId,
        horizontal: bool,
    ) -> i32 {
        let root = self.position(root).unwrap();
        let globally_furthest = self.position(globally_furthest).unwrap();
        let root_axis = if horizontal { root.x } else { root.y };
        let global_axis = if horizontal {
            globally_furthest.x
        } else {
            globally_furthest.y
        };
        if root_axis == global_axis { 2 } else { 0 }
    }

    pub(super) fn sized_compaction_floor(
        &self,
        anchor: NodeId,
        factor: f64,
        horizontal: bool,
        padding: f64,
    ) -> f64 {
        let position = self.position(anchor).unwrap();
        let size = self.nodes[anchor.0 as usize].rect.size;
        let anchor_axis = if horizontal { position.x } else { position.y };
        let anchor_length = if horizontal { size.width } else { size.height };
        // Go 1.27.0 lowers `anchor + length*factor` in getFloor to ARM64
        // FMADDD. Keeping the multiply and add fused matters at exact cell
        // boundaries (609 * 1.137931... - 792 is the parity regression).
        let fused_extent = anchor_length.mul_add(factor, anchor_axis);
        let mut floor = (fused_extent / self.cell_size).ceil();
        while self.cell_size * floor - (anchor_axis + anchor_length) <= padding {
            floor += 1.0;
        }
        floor
    }

    pub(super) fn possible_sized_compaction_moves(
        &self,
        node: NodeId,
        factor: f64,
        horizontal: bool,
        floor_decrease: i32,
        has_fixed_node: bool,
        visibility: &[(NodeId, NodeId)],
    ) -> Result<Vec<Point>, limits::Error> {
        self.possible_sized_compaction_moves_with_context(
            node,
            factor,
            horizontal,
            floor_decrease,
            has_fixed_node,
            visibility,
            &compaction_candidates::CompactionContext,
        )
    }
    pub(super) fn possible_sized_compaction_moves_with_context(
        &self,
        node: NodeId,
        factor: f64,
        horizontal: bool,
        floor_decrease: i32,
        has_fixed_node: bool,
        visibility: &[(NodeId, NodeId)],
        context: &dyn limits::Context,
    ) -> Result<Vec<Point>, limits::Error> {
        let mut guard = limits::WorkGuard::new(
            Some(context),
            "CompactionCandidates",
            limits::MAX_ENGINE_WORK_UNITS,
        )?;
        let current = self.position(node).unwrap();
        let anchor = self
            .nearest_visibility_predecessor(node, horizontal, true, visibility)
            .unwrap_or_else(|| self.globally_furthest_behind(horizontal));
        let anchor_position = self.position(anchor).unwrap();
        let node_size = self.nodes[node.0 as usize].rect.size;
        let anchor_size = self.nodes[anchor.0 as usize].rect.size;
        // Node.DeltaTo includes loop extents and directional margins.  The
        // sized compaction floor uses that exact clearance to keep a node's
        // loop routes from being pulled through its visibility predecessor.
        let padding = self.spacing_delta_with_loops(node, anchor, current);
        let mut floor = self.sized_compaction_floor(anchor, factor, horizontal, padding);
        let on_another_axis = if horizontal {
            anchor_position.y > current.y + node_size.height
                || current.y > anchor_position.y + anchor_size.height
        } else {
            anchor_position.x > current.x + node_size.width
                || current.x > anchor_position.x + anchor_size.width
        };
        let same_axis = if horizontal {
            anchor_position.x == current.x
        } else {
            anchor_position.y == current.y
        };
        if same_axis || on_another_axis {
            let aligned_floor = if horizontal {
                anchor_position.x / self.cell_size
            } else {
                anchor_position.y / self.cell_size
            };
            // Go rounds this override when the current Graph.Nodes slice has
            // a fixed node. Without that rounding, an off-grid fixed origin
            // generates fractional candidate cells (for example 240/126),
            // shifting unrelated nodes by the origin's sub-cell remainder.
            floor = if has_fixed_node {
                aligned_floor.round()
            } else {
                aligned_floor
            };
        }
        let ceiling = if horizontal {
            current.x / self.cell_size
        } else {
            current.y / self.cell_size
        };
        compaction_candidates::positions(
            &mut guard,
            floor - f64::from(floor_decrease),
            ceiling,
            current,
            horizontal,
            self.cell_size,
        )
    }

    pub(super) fn sized_point_overlaps(&self, node: NodeId, point: Point) -> bool {
        if self.container_direction_is_unset(node) {
            return self.sized_point_overlaps_with_loops(node, point);
        }
        let size = self.nodes[node.0 as usize].rect.size;
        self.nodes.iter().any(|other| {
            if other.input_id == node {
                return false;
            }
            let Some(other_position) = other.position else {
                return false;
            };
            // Match optimizerDoesOverlap's fixed 500px broad phase. The
            // graph-wide spacing bound excludes loop-aware deltas (which can
            // exceed the ordinary 20/60px gap), changing whether a candidate
            // is considered occupied.
            const MAX_SAFE_DELTA: f64 = 500.0;
            if point.x > other_position.x + other.rect.size.width + MAX_SAFE_DELTA
                || point.x + size.width + MAX_SAFE_DELTA < other_position.x
                || point.y > other_position.y + other.rect.size.height + MAX_SAFE_DELTA
                || point.y + size.height + MAX_SAFE_DELTA < other_position.y
            {
                return false;
            }
            let delta = self.spacing_delta(node, other.input_id, point);
            point.x < other_position.x + other.rect.size.width + delta
                && point.x + size.width + delta > other_position.x
                && point.y < other_position.y + other.rect.size.height + delta
                && point.y + size.height + delta > other_position.y
        })
    }

    pub(super) fn sized_point_overlaps_with_loops(&self, node: NodeId, point: Point) -> bool {
        let size = self.nodes[node.0 as usize].rect.size;
        self.nodes.iter().any(|other| {
            if other.input_id == node {
                return false;
            }
            let Some(other_position) = other.position else {
                return false;
            };
            const MAX_SAFE_DELTA: f64 = 500.0;
            if point.x > other_position.x + other.rect.size.width + MAX_SAFE_DELTA
                || point.x + size.width + MAX_SAFE_DELTA < other_position.x
                || point.y > other_position.y + other.rect.size.height + MAX_SAFE_DELTA
                || point.y + size.height + MAX_SAFE_DELTA < other_position.y
            {
                return false;
            }
            let delta = self.spacing_delta_with_loops(node, other.input_id, point);
            point.x < other_position.x + other.rect.size.width + delta
                && point.x + size.width + delta > other_position.x
                && point.y < other_position.y + other.rect.size.height + delta
                && point.y + size.height + delta > other_position.y
        })
    }

    pub(super) fn sorted_overlap_candidates(&self, node: NodeId) -> (Vec<NodeId>, f64) {
        let mut candidates: Vec<_> = self
            .nodes
            .iter()
            .filter(|other| other.input_id != node && other.position.is_some())
            .map(|other| other.input_id)
            .collect();
        candidates.sort_by(|left, right| {
            self.position(*left)
                .unwrap()
                .x
                .total_cmp(&self.position(*right).unwrap().x)
        });
        let max_width = candidates
            .iter()
            .map(|other| self.nodes[other.0 as usize].rect.size.width)
            .fold(0.0_f64, f64::max);
        (candidates, max_width)
    }

    pub(super) fn sized_point_overlaps_among(
        &self,
        node: NodeId,
        point: Point,
        candidates: &[NodeId],
        max_width: f64,
    ) -> bool {
        let size = self.nodes[node.0 as usize].rect.size;
        let lower_x = point.x - self.max_spacing_delta - max_width;
        let upper_x = point.x + size.width + self.max_spacing_delta;
        let start = candidates.partition_point(|other| self.position(*other).unwrap().x <= lower_x);
        let end = candidates.partition_point(|other| self.position(*other).unwrap().x < upper_x);
        candidates[start..end].iter().copied().any(|other| {
            if !self.could_overlap_with_max_delta(node, other, point) {
                return false;
            }
            let other_position = self.position(other).unwrap();
            let other_size = self.nodes[other.0 as usize].rect.size;
            let delta = if self.container_direction_is_unset(node) {
                self.spacing_delta_with_loops(node, other, point)
            } else {
                self.spacing_delta(node, other, point)
            };
            point.x < other_position.x + other_size.width + delta
                && point.x + size.width + delta > other_position.x
                && point.y < other_position.y + other_size.height + delta
                && point.y + size.height + delta > other_position.y
        })
    }

    // Cheap fail-closed prefilter for the recovered >10-node positions-cache
    // role. max_spacing_delta is a graph-wide upper bound, so candidates
    // rejected here cannot overlap under the more precise pairwise delta.
    pub(super) fn could_overlap_with_max_delta(
        &self,
        node: NodeId,
        other: NodeId,
        point: Point,
    ) -> bool {
        let size = self.nodes[node.0 as usize].rect.size;
        let other_position = self.position(other).unwrap();
        let other_size = self.nodes[other.0 as usize].rect.size;
        let delta = self.max_spacing_delta;
        point.x < other_position.x + other_size.width + delta
            && point.x + size.width + delta > other_position.x
            && point.y < other_position.y + other_size.height + delta
            && point.y + size.height + delta > other_position.y
    }

    pub(super) fn move_node_to_best_sized(&mut self, node: NodeId, points: &[Point]) -> bool {
        self.move_node_to_best_sized_with_owner_nears(node, points, None)
            .expect("unbound compaction scoring cannot fail")
    }

    fn move_node_to_best_sized_with_owner_nears(
        &mut self,
        node: NodeId,
        points: &[Point],
        mut owner_session: Option<&mut OwnerScoreSession>,
    ) -> Result<bool, OptimizationError> {
        let current = self.position(node).unwrap();
        let trace_member = crate::internal::trace_env_value("TALA_TRACE_COMPACTION_MEMBER")
            .and_then(|value| value.parse::<i64>().ok());
        let trace_move =
            trace_member.is_some_and(|member| self.nodes[node.0 as usize].tala_id == member);
        if trace_move {
            eprintln!(
                "COMPACTION_MOVE_RUST node={} begin current=({:.17},{:.17}) candidates={}",
                self.nodes[node.0 as usize].tala_id,
                current.x,
                current.y,
                points.len()
            );
        }
        let (overlap_candidates, max_candidate_width) = self.sorted_overlap_candidates(node);
        let symmetry_cost = self.cell_size * self.nodes[node.0 as usize].edges.len() as f64;
        // TALA walks stable Graph.Containers slices while scoring candidates.
        // Materialize the identical per-edge order once for this node instead
        // of rebuilding container chains and temporary vectors at every point.
        let mut obstruction_cache = vec![Vec::new(); self.edge_records.len()];
        for edge in self.nodes[node.0 as usize].edges.read().iter().copied() {
            let adjacent = self.adjacent(node, edge);
            obstruction_cache[edge.0 as usize] = self
                .edge_obstruction_nodes(node, adjacent)
                .into_iter()
                .filter(|obstruction| {
                    *obstruction != node
                        && *obstruction != adjacent
                        && !self.is_descendant_of(node, *obstruction)
                        && !self.is_descendant_of(adjacent, *obstruction)
                })
                .collect();
        }
        let mut least = f64::INFINITY;
        let mut best = current;
        for point in points.iter().copied() {
            let overlaps = self.sized_point_overlaps_among(
                node,
                point,
                &overlap_candidates,
                max_candidate_width,
            );
            if point != current && overlaps {
                if trace_move {
                    eprintln!(
                        "COMPACTION_MOVE_RUST node={} candidate=({:.17},{:.17}) overlaps=true",
                        self.nodes[node.0 as usize].tala_id, point.x, point.y
                    );
                }
                continue;
            }
            self.move_node_abs_with_children(node, point);
            // Node.edgeLength scans Graph.Containers in graph order. Alternate
            // routes are only checked after the direct route first becomes
            // blocked, so sorting obstructions is not behavior-preserving.
            let edge_length = self.compaction_edge_length_with_owner_nears(
                node,
                Some(&obstruction_cache),
                &mut owner_session,
            )?;
            // The v0.9.0 Go compaction helper applies this strongest-possible
            // symmetry-credit prune to every sized candidate. It does not
            // gate the shortcut on the container direction, so keeping that
            // extra Rust-only condition changes which later candidate wins.
            if least.is_finite() && edge_length - symmetry_cost > least + 0.0001 {
                if trace_move {
                    eprintln!(
                        "COMPACTION_MOVE_RUST node={} candidate=({:.17},{:.17}) pruned edge={:.17} least={:.17}",
                        self.nodes[node.0 as usize].tala_id, point.x, point.y, edge_length, least
                    );
                }
                continue;
            }
            let score = edge_length + self.column_to_column_crossing_cost(node, false)
                - self.sized_symmetry(node, true) * symmetry_cost;
            if trace_move {
                eprintln!(
                    "COMPACTION_MOVE_RUST node={} candidate=({:.17},{:.17}) edge={:.17} score={:.17} least={:.17}",
                    self.nodes[node.0 as usize].tala_id,
                    point.x,
                    point.y,
                    edge_length,
                    score,
                    least
                );
            }
            if score + 0.0001 < least || ((score - least).abs() <= 0.0001 && point == current) {
                least = score;
                best = point;
            }
        }
        self.move_node_abs_with_children(node, best);
        if trace_move {
            eprintln!(
                "COMPACTION_MOVE_RUST node={} selected=({:.17},{:.17}) score={:.17} moved={}",
                self.nodes[node.0 as usize].tala_id,
                best.x,
                best.y,
                least,
                best != current
            );
        }
        Ok(best != current)
    }

    pub(super) fn nodes_sized_edge_length(&self, nodes: &[NodeId]) -> f64 {
        let trace = crate::internal::trace_env_enabled("TALA_TRACE_EDGE_NODES_RUST");
        let mut total = 0.0;
        for node in nodes.iter().copied() {
            let length = self.sized_edge_length(node, true);
            total += length;
            if trace {
                eprintln!(
                    "EDGE_NODE_RUST count={} node={} pos={:?} length={:.17} total={:.17}",
                    nodes.len(),
                    self.nodes[node.0 as usize].tala_id,
                    self.position(node),
                    length,
                    total,
                );
            }
        }
        total
    }

    pub(super) fn nodes_sized_symmetry(&self, nodes: &[NodeId]) -> f64 {
        nodes
            .iter()
            .copied()
            .map(|node| self.sized_symmetry(node, true))
            .sum()
    }

    pub(super) fn sized_node_overlaps_other_at(
        &self,
        node: NodeId,
        other: NodeId,
        point: Point,
    ) -> bool {
        let other_position = self.position(other).unwrap();
        let size = self.nodes[node.0 as usize].rect.size;
        let other_size = self.nodes[other.0 as usize].rect.size;
        let delta = if self.container_direction_is_unset(node)
            && self.container_direction_is_unset(other)
        {
            self.spacing_delta_with_loops(node, other, point)
        } else {
            self.spacing_delta(node, other, point)
        };
        point.x < other_position.x + other_size.width + delta
            && point.x + size.width + delta > other_position.x
            && point.y < other_position.y + other_size.height + delta
            && point.y + size.height + delta > other_position.y
    }

    // Recovered Graph.shiftSubgraphs includeSizes=true path. Visibility roots
    // move as a unit, and candidate ranking includes the Nodes symmetry reward.
    pub(super) fn shift_sized_subgraphs(
        &mut self,
        horizontal: bool,
        factor: f64,
        visibility: &[(NodeId, NodeId)],
    ) -> bool {
        self.shift_sized_subgraphs_with_owner_nears(
            horizontal,
            factor,
            visibility,
            None,
            &compaction_candidates::CompactionContext,
        )
        .expect("unbound compaction scoring cannot fail")
    }

    fn shift_sized_subgraphs_with_owner_nears(
        &mut self,
        horizontal: bool,
        factor: f64,
        visibility: &[(NodeId, NodeId)],
        mut owner_session: Option<&mut OwnerScoreSession>,
        context: &dyn limits::Context,
    ) -> Result<bool, OptimizationError> {
        let trace_member = crate::internal::trace_env_value("TALA_TRACE_COMPACTION_MEMBER")
            .and_then(|value| {
                (value != "all")
                    .then(|| value.parse::<i64>().ok())
                    .flatten()
            });
        let trace_compaction = crate::internal::trace_env_enabled("TALA_TRACE_COMPACTION_MEMBER")
            && trace_member
                .is_some_and(|member| self.nodes.iter().any(|node| node.tala_id == member));
        let ordered = self.ordered_along_axis(horizontal);
        let mut subgraphs = BTreeMap::<NodeId, Vec<NodeId>>::new();
        let mut roots = Vec::new();
        for node in ordered {
            if let Some(predecessor) =
                self.nearest_visibility_predecessor(node, horizontal, true, visibility)
            {
                subgraphs.entry(predecessor).or_default().push(node);
            } else {
                subgraphs.insert(node, vec![node]);
                roots.push(node);
            }
        }
        roots.retain(|root| {
            !subgraphs[root]
                .iter()
                .any(|node| self.nodes[node.0 as usize].fixed_top_left.is_some())
        });
        roots.sort_by(|a, b| {
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

        if trace_compaction {
            eprint!(
                "COMPACTION_SHIFT_RUST axis={} factor={} roots=",
                if horizontal { 1 } else { 2 },
                factor
            );
            for root in &roots {
                eprint!(
                    "{}@{:?}(edges={},active={},cluster={:?},aggregate={},container={},scoring_cluster={:?})[",
                    self.nodes[root.0 as usize].tala_id,
                    self.position(*root),
                    self.nodes[root.0 as usize].edges.len(),
                    self.active_edge_count(*root),
                    self.active_cluster_index(*root),
                    self.nodes[root.0 as usize].scoring_is_aggregate_vessel,
                    self.nodes[root.0 as usize].scoring_is_container,
                    self.nodes[root.0 as usize].scoring_cluster_arrangement
                );
                for member in &subgraphs[root] {
                    eprint!("{},", self.nodes[member.0 as usize].tala_id);
                }
                eprint!("]; ");
            }
            eprintln!();
        }

        let global = self.globally_furthest_behind(horizontal);
        let has_fixed_node = self.has_current_fixed_node();
        let all_nodes = self.graph_node_order();
        let fixed: Vec<_> = all_nodes
            .iter()
            .copied()
            .filter(|node| self.nodes[node.0 as usize].fixed_top_left.is_some())
            .collect();
        let mut changed = false;
        for root in roots {
            let subgraph = subgraphs[&root].clone();
            let subgraph_set: BTreeSet<_> = subgraph.iter().copied().collect();
            let root_position = self.position(root).unwrap();
            let root_axis = if horizontal {
                root_position.x
            } else {
                root_position.y
            };
            // Recovered shiftSubgraphs retains the globally-furthest Node
            // pointer, not a snapshot of its coordinate. An earlier root can
            // move that node during this pass, and later floorDecrease checks
            // observe its current TopLeft.
            let floor_decrease = self.sized_compaction_floor_decrease(root, global, horizontal);
            let mut moves = self
                .possible_sized_compaction_moves_with_context(
                    root,
                    factor,
                    horizontal,
                    floor_decrease,
                    has_fixed_node,
                    visibility,
                    context,
                )
                .map_err(compaction_candidates::optimization_error)?;
            moves.push(root_position);
            if trace_compaction
                && trace_member.is_some_and(|member| self.nodes[root.0 as usize].tala_id == member)
            {
                eprint!(
                    "COMPACTION_SHIFT_RUST root={} moves=",
                    self.nodes[root.0 as usize].tala_id
                );
                for target in &moves {
                    eprint!("{:?};", target);
                }
                eprintln!();
            }
            let mut outside = fixed.clone();
            outside.extend(
                all_nodes
                    .iter()
                    .copied()
                    .filter(|node| !subgraph_set.contains(node)),
            );

            let symmetry_cost = self.cell_size
                * subgraph
                    .iter()
                    .map(|node| self.nodes[node.0 as usize].edges.len())
                    .sum::<usize>() as f64;
            let mut best_distance = compaction_scoring::nodes_edge_length(
                self,
                &subgraph,
                true,
                &mut owner_session,
                context,
            )?;
            let mut best_delta = 0.0;
            let mut direct_positions = if owner_session.is_none() {
                let slots = subgraph.iter().copied().map(Some).collect::<Vec<_>>();
                let roots = self
                    .allocate_owner_member_header(&slots, slots.len())
                    .map_err(|e| limits::OptimizationError::Invalid(e.to_string()))?;
                Some(
                    compaction_checkpoint::CapturedCompactionPositions::capture(
                        self, roots, context,
                    )
                    .map_err(compaction_candidates::optimization_error)?,
                )
            } else {
                None
            };
            let positions = CompactionWorkspacePositions::capture(self);
            let mut owner_positions = owner_session
                .as_deref_mut()
                .map(|session| session.compaction_positions_with_context(&subgraph, context))
                .transpose()
                .map_err(compaction_candidates::optimization_error)?;
            for target in moves {
                let target_axis = if horizontal { target.x } else { target.y };
                let delta = root_axis - target_axis;
                let overlaps = subgraph.iter().copied().any(|node| {
                    let current = self.position(node).unwrap();
                    let moved = if horizontal {
                        Point {
                            x: current.x - delta,
                            y: current.y,
                        }
                    } else {
                        Point {
                            x: current.x,
                            y: current.y - delta,
                        }
                    };
                    // Go's shiftSubgraphs checks PointPastFixedOrigin before
                    // testing sibling overlaps.  A non-fixed container can
                    // still have a fixed origin inherited from its owning
                    // scope; allowing this move would place the whole
                    // subgraph outside that origin and changes later
                    // candidate rings.
                    if let Some(origin) =
                        self.container_fixed_origin(self.nodes[node.0 as usize].container)
                        && (moved.x < origin.x || moved.y < origin.y)
                    {
                        return true;
                    }
                    outside.iter().copied().any(|other| {
                        !subgraph_set.contains(&other)
                            && self.sized_node_overlaps_other_at(node, other, moved)
                    })
                });
                if overlaps {
                    continue;
                }
                let translation = if horizontal {
                    Point { x: -delta, y: 0.0 }
                } else {
                    Point { x: 0.0, y: -delta }
                };
                for node in subgraph.iter().copied() {
                    self.translate_node_with_children(node, translation);
                }
                let mut trial_session = owner_positions.as_mut().map(|snapshot| snapshot.session());
                let distance = compaction_scoring::nodes_edge_length(
                    self,
                    &subgraph,
                    true,
                    &mut trial_session,
                    context,
                )
                .and_then(|distance| {
                    compaction_scoring::nodes_symmetry(self, &subgraph, context)
                        .map(|symmetry| distance - symmetry_cost * symmetry)
                });
                // compaction.go239–308 restores snapshots after EVERY trial,
                // including scoring errors. Inverse translation is not exact.
                drop(trial_session);
                if let Some(snapshot) = direct_positions.as_mut() {
                    snapshot.restore(self);
                    positions.restore_projections(self);
                } else {
                    positions.restore(self);
                }
                if let Some(snapshot) = owner_positions.as_mut() {
                    snapshot.restore();
                }
                let distance = distance?;
                if distance < best_distance {
                    best_distance = distance;
                    best_delta = delta;
                }
                if trace_compaction
                    && trace_member
                        .is_some_and(|member| self.nodes[root.0 as usize].tala_id == member)
                {
                    eprintln!(
                        "COMPACTION_SHIFT_RUST candidate={:?} score={} best={} delta={} overlap={}",
                        target, distance, best_distance, best_delta, overlaps
                    );
                }
            }
            drop(owner_positions);
            if best_delta != 0.0 {
                let translation = if horizontal {
                    Point {
                        x: -best_delta,
                        y: 0.0,
                    }
                } else {
                    Point {
                        x: 0.0,
                        y: -best_delta,
                    }
                };
                for node in subgraph.iter().copied() {
                    self.translate_node_with_children(node, translation);
                }
                changed = true;
            }
            if trace_compaction
                && trace_member.is_some_and(|member| {
                    self.nodes[root.0 as usize].tala_id == member
                        || subgraph
                            .iter()
                            .any(|node| self.nodes[node.0 as usize].tala_id == member)
                })
            {
                eprintln!(
                    "COMPACTION_SHIFT_RUST result={} delta={} changed={} pos={:?} members={:?}",
                    self.nodes[root.0 as usize].tala_id,
                    best_delta,
                    best_delta != 0.0,
                    self.position(root),
                    subgraph
                        .iter()
                        .map(|node| { (self.nodes[node.0 as usize].tala_id, self.position(*node)) })
                        .collect::<Vec<_>>()
                );
            }
        }
        if trace_compaction {
            eprint!("COMPACTION_SHIFT_RUST after changed={} state=", changed);
            for node in &self.nodes {
                eprint!("{}@{:?};", node.tala_id, node.position);
            }
            eprintln!();
        }
        Ok(changed)
    }

    pub(super) fn inflate_sized_axis(
        &mut self,
        horizontal: bool,
        factor: f64,
        visibility: &[(NodeId, NodeId)],
    ) {
        for node in self.ordered_along_axis(horizontal) {
            let Some(anchor) =
                self.nearest_visibility_predecessor(node, horizontal, true, visibility)
            else {
                continue;
            };
            if self.nodes[node.0 as usize].fixed_top_left.is_some() {
                continue;
            }
            let current = self.position(node).unwrap();
            let padding = self.spacing_delta_with_loops(node, anchor, current);
            let floor =
                self.sized_compaction_floor(anchor, factor, horizontal, padding) * self.cell_size;
            let axis = if horizontal { current.x } else { current.y };
            if crate::internal::trace_env_enabled("TALA_TRACE_INFLATE_DETAIL")
                && self.nodes.len() == 10
                && horizontal
                && (factor - 2.549295774647887).abs() < 1e-9
            {
                eprintln!(
                    "INFLATE_DETAIL_RUST node={} anchor={} current={} padding={} floor={} anchorPos={}",
                    self.nodes[node.0 as usize].tala_id,
                    self.nodes[anchor.0 as usize].tala_id,
                    axis,
                    padding,
                    floor,
                    if horizontal {
                        self.position(anchor).unwrap().x
                    } else {
                        self.position(anchor).unwrap().y
                    },
                    // sizes and loop extents are printed below by the parallel diagnostic parser
                );
                eprintln!(
                    "INFLATE_DETAIL_RUST_META nodeSize={:?} anchorSize={:?} loops={:?} anchorLoops={:?}",
                    self.nodes[node.0 as usize].rect.size,
                    self.nodes[anchor.0 as usize].rect.size,
                    self.loop_spacing_extents(node),
                    self.loop_spacing_extents(anchor),
                );
            }
            if axis < floor {
                self.move_node_abs_with_children(
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
                );
            }
        }
    }

    pub(super) fn compact_sized_axis(
        &mut self,
        horizontal: bool,
        factor: f64,
    ) -> Result<(), OptimizationError> {
        self.compact_sized_axis_with_owner_nears(horizontal, factor, None)
    }

    pub(super) fn compact_sized_axis_with_owner_nears(
        &mut self,
        horizontal: bool,
        factor: f64,
        owner_session: Option<&mut OwnerScoreSession>,
    ) -> Result<(), OptimizationError> {
        self.compact_sized_axis_with_context(
            horizontal,
            factor,
            owner_session,
            &compaction_candidates::CompactionContext,
        )
    }

    pub(super) fn compact_sized_axis_with_context(
        &mut self,
        horizontal: bool,
        factor: f64,
        owner_session: Option<&mut OwnerScoreSession>,
        context: &dyn limits::Context,
    ) -> Result<(), OptimizationError> {
        self.compact_sized_axis_with_move_limit_context(
            horizontal,
            factor,
            owner_session,
            0,
            context,
        )
    }

    pub(super) fn compact_sized_axis_with_move_limit_context(
        &mut self,
        horizontal: bool,
        factor: f64,
        owner_session: Option<&mut OwnerScoreSession>,
        move_work_limit: u64,
        context: &dyn limits::Context,
    ) -> Result<(), OptimizationError> {
        compaction_candidates::validate_factor(factor)?;
        if let Some(session) = owner_session.as_deref() {
            session
                .owner()
                .validate_owner_graph_topology(
                    Some(session.current_view()),
                    Some(context),
                    "Compaction",
                )
                .map_err(compaction_candidates::optimization_error)?;
        } else {
            self.initialize_shared_topology();
            self.bind_owner_placement_directions();
            self.validate_owner_graph_topology(Some(OwnerGraphId(0)), Some(context), "Compaction")
                .map_err(compaction_candidates::optimization_error)?;
        }
        let routing = self.cost_cache.raw();
        let state = if owner_session.is_none() {
            Some(
                compaction_checkpoint::CapturedCompactionState::capture(
                    self,
                    OwnerGraphId(0),
                    context,
                )
                .map_err(compaction_candidates::optimization_error)?,
            )
        } else {
            None
        };
        let mut owner_positions = owner_session
            .map(|session| session.compaction_entry(context))
            .transpose()
            .map_err(compaction_candidates::optimization_error)?;
        // Dense carrier state is derived; it does not add a second source guard.
        let positions = CompactionWorkspacePositions::capture(self);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.compact_sized_axis_kernel(
                horizontal,
                factor,
                owner_positions.as_mut().map(|snapshot| snapshot.session()),
                context,
                move_work_limit,
            )
        }));
        if matches!(&result, Ok(Ok(()))) {
            if let Some(snapshot) = owner_positions.as_mut() {
                snapshot.commit();
            }
        } else {
            if let Some(state) = &state {
                state.restore(self);
                positions.restore_projections(self);
            } else {
                positions.restore(self);
            }
            self.cost_cache.restore_routing(routing);
        }
        match result {
            Ok(result) => result,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    fn compact_sized_axis_kernel(
        &mut self,
        horizontal: bool,
        factor: f64,
        mut owner_session: Option<&mut OwnerScoreSession>,
        context: &dyn limits::Context,
        move_work_limit: u64,
    ) -> Result<(), OptimizationError> {
        let trace_member = crate::internal::trace_env_value("TALA_TRACE_COMPACTION_MEMBER")
            .and_then(|value| {
                (value != "all")
                    .then(|| value.parse::<i64>().ok())
                    .flatten()
            });
        let trace_compaction = crate::internal::trace_env_enabled("TALA_TRACE_COMPACTION_MEMBER")
            && trace_member
                .is_some_and(|member| self.nodes.iter().any(|node| node.tala_id == member));
        if trace_compaction {
            eprintln!(
                "COMPACTION_ENTER_RUST nodes={} axis={} factor={}",
                self.nodes.len(),
                if horizontal { 1 } else { 2 },
                factor
            );
            if let Some(member) = trace_member
                && let Some(node) = self.nodes.iter().find(|node| node.tala_id == member)
            {
                eprintln!(
                    "COMPACTION_BEFORE_RUST {}={:?} fixed={}",
                    member,
                    node.position,
                    node.fixed_top_left.is_some()
                );
            }
        }
        let has_fixed_node = self.has_current_fixed_node();
        let mut visibility_guard = limits::WorkGuard::new(
            Some(context),
            "CompactionVisibility",
            limits::MAX_ENGINE_WORK_UNITS,
        )
        .map_err(compaction_candidates::optimization_error)?;
        let visibility = self
            .visibility_edges_with_guard(horizontal, true, Some(&mut visibility_guard))
            .map_err(compaction_candidates::optimization_error)?;
        self.inflate_sized_axis(horizontal, factor, &visibility);
        if let Some(session) = owner_session.as_deref_mut() {
            session
                .update_current_member_positions(self)
                .map_err(|e| OptimizationError::Invalid(e.to_string()))?;
        }
        compaction_candidates::check_context(context)?;
        if trace_compaction
            && let Some(member) = trace_member
            && let Some(node) = self.nodes.iter().find(|node| node.tala_id == member)
        {
            eprintln!(
                "COMPACTION_AFTER_INFLATE_RUST {}={:?}",
                member, node.position
            );
        }
        // The recovered Go compaction always runs shiftSubgraphs on the
        // materialized scope being optimized. The Rust arena may retain
        // parent links from the complete hierarchy, so placement_scope_owned
        // cannot be used as a proxy for whether this phase is required.
        for _ in 0..20 {
            if !self.shift_sized_subgraphs_with_owner_nears(
                horizontal,
                factor,
                &visibility,
                owner_session.as_deref_mut(),
                context,
            )? {
                break;
            }
        }
        let mut move_guard = limits::OptimizationWorkGuard::new(
            Some(context),
            "CompactionMoves",
            compaction_candidates::normalized_move_work_limit(move_work_limit),
        )?;
        for _ in 0..20 {
            let mut changed = false;
            for node in self.ordered_along_axis(horizontal) {
                if self.nodes[node.0 as usize].fixed_top_left.is_some() {
                    continue;
                }
                let mut moves = self
                    .possible_sized_compaction_moves_with_context(
                        node,
                        factor,
                        horizontal,
                        0,
                        has_fixed_node,
                        &visibility,
                        context,
                    )
                    .map_err(compaction_candidates::optimization_error)?;
                moves.push(self.position(node).unwrap());
                changed |= compaction_moves::current(
                    self,
                    owner_session.as_deref_mut(),
                    node,
                    &moves,
                    true,
                    context,
                    &mut move_guard,
                )?;
            }
            if !changed {
                break;
            }
        }
        if let Some(session) = owner_session {
            session
                .update_current_member_positions(self)
                .map_err(|error| OptimizationError::Invalid(error.to_string()))?;
        }
        compaction_candidates::check_context(context)?;
        Ok(())
    }

    // Recovered Nodes.getClusters. An edge or near pair farther apart than
    // three cells seeds the partition; nodes within that threshold are then
    // merged recursively into the seed's cluster.
}

#[cfg(test)]
mod snapshot_tests {
    use super::*;

    #[test]
    fn rejected_compaction_subgraph_moves_preserve_fractional_descendants() {
        let mut input = Graph::default();
        let parent = input.add_node(crate::Node::new(
            "parent",
            Size {
                width: 1040.0,
                height: 1042.0,
            },
        ));
        let mut child_node = crate::Node::new(
            "child",
            Size {
                width: 100.0,
                height: 80.0,
            },
        );
        child_node.parent = Some(parent);
        let child = input.add_node(child_node);
        let mut graph = ArenaGraph::from_input(&input);
        graph.node_order = vec![parent];
        graph.refresh_node_order_membership();
        graph.set_position(parent, Point::default());
        let initial = Point { x: 620.4, y: 521.6 };
        graph.set_position(child, initial);
        for horizontal in [false, true] {
            assert!(!graph.shift_sized_subgraphs(horizontal, 1.0, &[]));
            let restored = graph.position(child).unwrap();
            assert_eq!(
                (restored.x.to_bits(), restored.y.to_bits()),
                (initial.x.to_bits(), initial.y.to_bits())
            );
        }
    }
}
