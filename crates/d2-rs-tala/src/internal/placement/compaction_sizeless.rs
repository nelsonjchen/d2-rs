// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Compaction of the integer-cell sizeless placement.
//!
//! Occupancy and score changes are evaluated in stable axis order using the
//! same open precision boundary as the recovered implementation.

use super::*;
use d2_rs_core::stable_sort::StableSort;
#[path = "compaction_sizeless_geometry.rs"]
mod geometry;
#[path = "compaction_sizeless_scoring.rs"]
mod subgraph_scoring;

fn sizeless_point_key(point: Point) -> (u64, u64) {
    // Rust/Go floating-point equality treats both signed zeroes as equal.
    // Sizeless placement never admits NaN, so normalized IEEE bits retain the
    // exact Point equality used by TALA while permitting an ordered index.
    let bits = |value: f64| {
        if value == 0.0 {
            0.0_f64.to_bits()
        } else {
            value.to_bits()
        }
    };
    (bits(point.x), bits(point.y))
}

fn compaction_precision_compare(left: f64, right: f64) -> std::cmp::Ordering {
    const PRECISION: f64 = 0.0001;
    if (left - right).abs() <= PRECISION {
        std::cmp::Ordering::Equal
    } else {
        left.total_cmp(&right)
    }
}

impl ArenaGraph {
    pub(super) fn snap_nonfixed_to_cells(&mut self) {
        for index in 0..self.nodes.len() {
            let node = NodeId(index as u32);
            if self.nodes[index].fixed_top_left.is_some() {
                continue;
            }
            let current = self.position(node).unwrap();
            let snapped = Point {
                x: (current.x / self.cell_size).floor() * self.cell_size,
                y: (current.y / self.cell_size).floor() * self.cell_size,
            };
            if snapped != current {
                self.move_node_abs_with_children(node, snapped);
            }
        }
    }

    pub(super) fn globally_furthest_behind_in(&self, nodes: &[NodeId], horizontal: bool) -> NodeId {
        nodes
            .iter()
            .copied()
            .filter(|node| self.position(*node).is_some())
            .min_by(|a, b| {
                let a = self.position(*a).unwrap();
                let b = self.position(*b).unwrap();
                let a_axis = if horizontal { a.x } else { a.y };
                let b_axis = if horizontal { b.x } else { b.y };
                a_axis.total_cmp(&b_axis)
            })
            .unwrap_or(NodeId(0))
    }

    pub(super) fn globally_furthest_behind(&self, horizontal: bool) -> NodeId {
        let nodes = self.graph_node_order();
        self.globally_furthest_behind_in(&nodes, horizontal)
    }

    pub(super) fn possible_sizeless_compaction_moves(
        &self,
        node: NodeId,
        factor: f64,
        horizontal: bool,
        floor_decrease: i32,
        visibility: &BTreeMap<NodeId, NodeId>,
        active: &[NodeId],
    ) -> Result<Vec<Point>, limits::Error> {
        self.possible_sizeless_compaction_moves_with_context(
            node,
            factor,
            horizontal,
            floor_decrease,
            visibility,
            active,
            &compaction_candidates::CompactionContext,
        )
    }
    pub(super) fn possible_sizeless_compaction_moves_with_context(
        &self,
        node: NodeId,
        factor: f64,
        horizontal: bool,
        floor_decrease: i32,
        visibility: &BTreeMap<NodeId, NodeId>,
        active: &[NodeId],
        context: &dyn limits::Context,
    ) -> Result<Vec<Point>, limits::Error> {
        let mut guard = limits::WorkGuard::new(
            Some(context),
            "CompactionCandidates",
            limits::MAX_ENGINE_WORK_UNITS,
        )?;
        let current = self.position(node).unwrap();
        let predecessor = visibility.get(&node).copied();
        let anchor =
            predecessor.unwrap_or_else(|| self.globally_furthest_behind_in(active, horizontal));
        let anchor_position = self.position(anchor).unwrap();
        let another_axis = if horizontal {
            anchor_position.y != current.y
        } else {
            anchor_position.x != current.x
        };
        let same_axis = if horizontal {
            anchor_position.x == current.x
        } else {
            anchor_position.y == current.y
        };
        let anchor_axis = if horizontal {
            anchor_position.x
        } else {
            anchor_position.y
        };
        let mut floor = anchor_axis + factor.floor();
        if same_axis || another_axis {
            floor = anchor_axis;
        }
        floor -= f64::from(floor_decrease);
        let ceiling = if horizontal { current.x } else { current.y };
        compaction_candidates::positions(&mut guard, floor, ceiling, current, horizontal, 1.0)
    }

    pub(super) fn move_node_to_best_sizeless(&mut self, node: NodeId, points: &[Point]) -> bool {
        let current = self.position(node).unwrap();
        let mut least = f64::INFINITY;
        let mut best = current;
        for point in points.iter().copied() {
            if point != current && self.is_occupied(point, Some(node)) {
                continue;
            }
            self.move_node_abs_with_children(node, point);
            let distance = self.sizeless_edge_length(node, true);
            match compaction_precision_compare(distance, least) {
                std::cmp::Ordering::Less => {
                    least = distance;
                    best = point;
                }
                std::cmp::Ordering::Equal if point == current => {
                    least = distance;
                    best = point;
                }
                _ => {}
            }
        }
        self.move_node_abs_with_children(node, best);
        best != current
    }

    pub(super) fn nodes_sizeless_edge_length(&self, nodes: &[NodeId]) -> f64 {
        nodes
            .iter()
            .copied()
            .map(|node| self.sizeless_edge_length(node, true))
            .sum()
    }

    // Direct translation of shiftSubgraphs for includeSizes=false.
    pub(super) fn shift_sizeless_subgraphs(
        &mut self,
        horizontal: bool,
        factor: f64,
        visibility: &BTreeMap<NodeId, NodeId>,
        active: &[NodeId],
    ) -> Result<bool, limits::OptimizationError> {
        self.shift_sizeless_subgraphs_with_owner_context(
            horizontal,
            factor,
            visibility,
            active,
            None,
            &compaction_candidates::CompactionContext,
        )
    }
    fn shift_sizeless_subgraphs_with_owner_context(
        &mut self,
        horizontal: bool,
        factor: f64,
        visibility: &BTreeMap<NodeId, NodeId>,
        active: &[NodeId],
        mut owner_session: Option<&mut OwnerScoreSession>,
        context: &dyn limits::Context,
    ) -> Result<bool, limits::OptimizationError> {
        let stable_workspace = context.stable_compaction_workspace()
            && owner_session
                .as_deref()
                .is_some_and(|s| geometry::detached(self, s.owner()));
        let ordered = self.ordered_subset_along_axis(active, horizontal);
        let mut subgraphs = BTreeMap::<NodeId, Vec<NodeId>>::new();
        let mut roots = Vec::new();
        for node in ordered {
            if let Some(predecessor) = visibility.get(&node).copied() {
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
        roots.stable_sort_by(|a, b| {
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
            })
        });
        let global = self.globally_furthest_behind_in(active, horizontal);
        let fixed: Vec<_> = active
            .iter()
            .copied()
            .filter(|node| self.nodes[node.0 as usize].fixed_top_left.is_some())
            .collect();
        let mut changed = false;

        for root in roots {
            let subgraph = subgraphs[&root].clone();
            let root_position = self.position(root).unwrap();
            let root_axis = if horizontal {
                root_position.x
            } else {
                root_position.y
            };
            let global_position = self.position(global).unwrap();
            let global_axis = if horizontal {
                global_position.x
            } else {
                global_position.y
            };
            let floor_decrease = if root_axis == global_axis { 2 } else { 0 };
            let mut moves = self
                .possible_sizeless_compaction_moves_with_context(
                    root,
                    factor,
                    horizontal,
                    floor_decrease,
                    visibility,
                    active,
                    context,
                )
                .map_err(compaction_candidates::optimization_error)?;
            moves.push(root_position);
            let subgraph_set: BTreeSet<_> = subgraph.iter().copied().collect();
            let mut outside = fixed.clone();
            outside.extend(
                active
                    .iter()
                    .copied()
                    .filter(|node| !subgraph_set.contains(node)),
            );
            // Only the subgraph moves while its candidate translations are
            // scored. TALA tests exact Point occupancy against this stable
            // outside inventory; index it once rather than rescanning every
            // outside node for every moved node and candidate.
            let mut scorers = subgraph_scoring::Scorers::new(
                &subgraph,
                context.stable_compaction_workspace() && owner_session.is_some(),
                context,
            );
            let mut best_distance = scorers.score(self, &subgraph, &mut owner_session, context)?;
            let mut best_delta = 0.0;
            // compaction.go snapshots once per root and restores after every
            // scored trial. Floating-point inverse translation loses bits.
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
            let footprints = stable_workspace.then(|| {
                subgraph
                    .iter()
                    .map(|&n| geometry::Footprint::capture(self, n))
                    .collect::<Vec<_>>()
            });
            let journal = footprints
                .as_ref()
                .map(|f| geometry::Journal::capture(self, f));
            let positions = journal
                .is_none()
                .then(|| compaction_sized::CompactionWorkspacePositions::capture(self));
            let mut owner_positions = owner_session
                .as_deref_mut()
                .map(|s| s.compaction_positions_with_context(&subgraph, context))
                .transpose()
                .map_err(compaction_candidates::optimization_error)?;

            for target in moves {
                let target_axis = if horizontal { target.x } else { target.y };
                let delta = root_axis - target_axis;
                let overlaps = subgraph.iter().copied().any(|node| {
                    let point = self.position(node).unwrap();
                    let moved = if horizontal {
                        Point {
                            x: point.x - delta,
                            y: point.y,
                        }
                    } else {
                        Point {
                            x: point.x,
                            y: point.y - delta,
                        }
                    };
                    // PointPastFixedOrigin converts the container origin to
                    // integer cells before testing either axis in sizeless mode.
                    if !fixed.is_empty() {
                        let container = self.active_node_container(node);
                        let origin = active.iter().find_map(|&other| {
                            if self.active_node_container(other) != container {
                                return None;
                            }
                            let fixed = self.nodes[other.0 as usize].fixed_top_left?;
                            let position = self.position(other)?;
                            Some(Point {
                                x: ((position.x - fixed.x) / self.cell_size).round(),
                                y: ((position.y - fixed.y) / self.cell_size).round(),
                            })
                        });
                        if origin.is_some_and(|origin| moved.x < origin.x || moved.y < origin.y) {
                            return true;
                        }
                    }
                    outside
                        .iter()
                        .any(|&other| self.position(other) == Some(moved))
                });
                if overlaps {
                    continue;
                }
                let translation = if horizontal {
                    Point { x: -delta, y: 0.0 }
                } else {
                    Point { x: 0.0, y: -delta }
                };
                if let Some(footprints) = &footprints {
                    for footprint in footprints {
                        footprint.apply(self, translation);
                    }
                } else {
                    for &node in &subgraph {
                        self.translate_node_with_children(node, translation);
                    }
                }
                let mut trial_session = owner_positions.as_mut().map(|p| p.session());
                let distance = scorers.score(self, &subgraph, &mut trial_session, context);
                drop(trial_session);
                if let Some(snapshot) = direct_positions.as_mut() {
                    snapshot.restore(self);
                    positions.as_ref().unwrap().restore_projections(self);
                } else if let Some(journal) = &journal {
                    journal.restore(self);
                } else {
                    positions.as_ref().unwrap().restore(self);
                }
                if let Some(snapshot) = owner_positions.as_mut() {
                    snapshot.restore();
                }
                let distance = distance?;
                if distance < best_distance {
                    best_distance = distance;
                    best_delta = delta;
                }
            }
            drop(owner_positions);
            if best_delta != 0.0 {
                for node in subgraph {
                    self.translate_node_with_children(
                        node,
                        if horizontal {
                            Point {
                                x: -best_delta,
                                y: 0.0,
                            }
                        } else {
                            Point {
                                x: 0.0,
                                y: -best_delta,
                            }
                        },
                    );
                }
                changed = true;
            }
        }
        Ok(changed)
    }

    pub(super) fn compact_sizeless_axis(
        &mut self,
        horizontal: bool,
        factor: f64,
        visibility: &BTreeMap<NodeId, NodeId>,
        active: &[NodeId],
        mut owner_session: Option<&mut OwnerScoreSession>,
        context: &dyn limits::Context,
        move_guard: &mut limits::OptimizationWorkGuard<'_>,
    ) -> Result<bool, limits::OptimizationError> {
        let ordered = self.ordered_subset_along_axis(active, horizontal);
        let mut changed = false;
        for node in ordered {
            if self.nodes[node.0 as usize].fixed_top_left.is_some() {
                continue;
            }
            let mut moves = self
                .possible_sizeless_compaction_moves_with_context(
                    node, factor, horizontal, 0, visibility, active, context,
                )
                .map_err(compaction_candidates::optimization_error)?;
            moves.push(self.position(node).unwrap());
            changed |= compaction_moves::current(
                self,
                owner_session.as_deref_mut(),
                node,
                &moves,
                false,
                context,
                move_guard,
            )?;
        }
        Ok(changed)
    }

    pub(super) fn inflate_sizeless_axis(
        &mut self,
        horizontal: bool,
        factor: f64,
        visibility: &BTreeMap<NodeId, NodeId>,
        active: &[NodeId],
    ) {
        let ordered = self.ordered_subset_along_axis(active, horizontal);
        for node in ordered {
            let Some(predecessor) = visibility.get(&node).copied() else {
                continue;
            };
            if self.nodes[node.0 as usize].fixed_top_left.is_some() {
                continue;
            }
            let current = self.position(node).unwrap();
            let predecessor = self.position(predecessor).unwrap();
            let floor = if horizontal {
                predecessor.x + factor.floor()
            } else {
                predecessor.y + factor.floor()
            };
            let axis = if horizontal { current.x } else { current.y };
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

    pub(super) fn compact_sizeless(
        &mut self,
        horizontal: bool,
        factor: f64,
        active: &[NodeId],
    ) -> Result<(), limits::OptimizationError> {
        self.compact_sizeless_with_context(
            horizontal,
            factor,
            active,
            &compaction_candidates::CompactionContext,
        )
    }
    pub(super) fn compact_sizeless_with_context(
        &mut self,
        horizontal: bool,
        factor: f64,
        active: &[NodeId],
        context: &dyn limits::Context,
    ) -> Result<(), limits::OptimizationError> {
        self.compact_sizeless_with_owner_context(horizontal, factor, active, None, context)
    }
    pub(super) fn compact_sizeless_with_owner_context(
        &mut self,
        horizontal: bool,
        factor: f64,
        active: &[NodeId],
        owner_session: Option<&mut OwnerScoreSession>,
        context: &dyn limits::Context,
    ) -> Result<(), limits::OptimizationError> {
        self.compact_sizeless_with_owner_move_limit_context(
            horizontal,
            factor,
            active,
            owner_session,
            0,
            context,
        )
    }

    pub(super) fn compact_sizeless_with_owner_move_limit_context(
        &mut self,
        horizontal: bool,
        factor: f64,
        active: &[NodeId],
        owner_session: Option<&mut OwnerScoreSession>,
        move_work_limit: u64,
        context: &dyn limits::Context,
    ) -> Result<(), limits::OptimizationError> {
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
        let positions = compaction_sized::CompactionWorkspacePositions::capture(self);
        let result = self.compact_sizeless_kernel(
            horizontal,
            factor,
            active,
            owner_positions.as_mut().map(|snapshot| snapshot.session()),
            context,
            move_work_limit,
        );
        if result.is_ok() {
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
        result
    }

    fn compact_sizeless_kernel(
        &mut self,
        horizontal: bool,
        factor: f64,
        active: &[NodeId],
        mut owner_session: Option<&mut OwnerScoreSession>,
        context: &dyn limits::Context,
        move_work_limit: u64,
    ) -> Result<(), limits::OptimizationError> {
        let mut visibility_guard = limits::WorkGuard::new(
            Some(context),
            "CompactionVisibility",
            limits::MAX_ENGINE_WORK_UNITS,
        )
        .map_err(compaction_candidates::optimization_error)?;
        let edges = self
            .visibility_edges_with_guard(horizontal, false, Some(&mut visibility_guard))
            .map_err(compaction_candidates::optimization_error)?;
        let visibility = active
            .iter()
            .filter_map(|&node| {
                self.nearest_visibility_predecessor(node, horizontal, false, &edges)
                    .map(|from| (node, from))
            })
            .collect::<BTreeMap<_, _>>();
        self.inflate_sizeless_axis(horizontal, factor, &visibility, active);
        if let Some(session) = owner_session.as_deref_mut() {
            session
                .update_current_member_positions(self)
                .map_err(|e| limits::OptimizationError::Invalid(e.to_string()))?;
        }
        compaction_candidates::check_context(context)?;
        for _ in 0..20 {
            let changed = self.shift_sizeless_subgraphs_with_owner_context(
                horizontal,
                factor,
                &visibility,
                active,
                owner_session.as_deref_mut(),
                context,
            )?;
            if !changed {
                break;
            }
        }
        let mut move_guard = limits::OptimizationWorkGuard::new(
            Some(context),
            "CompactionMoves",
            compaction_candidates::normalized_move_work_limit(move_work_limit),
        )?;
        for _ in 0..20 {
            if !self.compact_sizeless_axis(
                horizontal,
                factor,
                &visibility,
                active,
                owner_session.as_deref_mut(),
                context,
                &mut move_guard,
            )? {
                break;
            }
        }
        if let Some(session) = owner_session {
            session
                .update_current_member_positions(self)
                .map_err(|e| limits::OptimizationError::Invalid(e.to_string()))?;
        }
        compaction_candidates::check_context(context)?;
        Ok(())
    }

    // Direct translation of Node.scaleBasedOnEdges.
    pub(super) fn scale_node_based_on_edges(&mut self, node_id: NodeId) {
        let index = node_id.0 as usize;
        let node = &self.nodes[index];
        if node.fixed_top_left.is_some()
            || node.desired_width.is_some()
            || node.desired_height.is_some()
            || matches!(node.shape, ShapeKind::SqlTable | ShapeKind::Class)
            || node.edges.is_empty()
        {
            return;
        }
        let mut edge_counts = BTreeMap::<NodeId, usize>::new();
        for edge in node.edges.read().iter().copied() {
            // Prescale runs before AddSequences/AddClusters in v0.9.0. A
            // graphjson snapshot may already carry those decoded ownership
            // maps, but Go still counts the raw edge endpoints at this stage;
            // projecting both members onto a retained aggregate would merge
            // distinct neighbors and enlarge the node spuriously.
            let edge_ref = &self.edge_records[edge.0 as usize];
            let adjacent = if edge_ref.from == node_id {
                edge_ref.to
            } else {
                edge_ref.from
            };
            if adjacent != node_id {
                *edge_counts.entry(adjacent).or_default() += 1;
            }
        }
        let total_edges: usize = edge_counts.values().sum();
        let max_edges_to_adjacent = edge_counts.values().copied().max().unwrap_or(0);
        if crate::internal::trace_env_enabled("TALA_TRACE_PRESCALE") {
            eprintln!(
                "PRESCALE_RUST node={} edges={:?} counts={:?} total={} max={}",
                node.tala_id, node.edges, edge_counts, total_edges, max_edges_to_adjacent
            );
        }
        // Go's float `0 / 0` becomes NaN and converting that to int on the
        // supported architectures yields zero. Preserve that observable
        // behavior for nodes that only have self-edges: the resulting
        // minimum length is still one side spacing (40px).
        let mut edges_per_side = if edge_counts.is_empty() {
            0
        } else {
            total_edges.div_ceil(edge_counts.len().min(4))
        };
        edges_per_side = edges_per_side.max(max_edges_to_adjacent);
        if edges_per_side == 1 {
            return;
        }
        let mut min_length = (edges_per_side + 1) as f64 * 40.0;
        let node = &mut self.nodes[index];
        if node.rect.size.width.min(node.rect.size.height) > min_length {
            return;
        }
        let (x_ratio, y_ratio);
        if node.shape.has_unit_aspect_ratio() {
            if node.rect.size.width < min_length {
                x_ratio = min_length / node.rect.size.width;
                y_ratio = min_length / node.rect.size.height;
                node.rect.size.width = min_length;
                node.rect.size.height = min_length;
            } else {
                x_ratio = 1.0;
                y_ratio = 1.0;
            }
        } else {
            if node.rect.size.width < min_length {
                x_ratio = min_length / node.rect.size.width;
                node.rect.size.width = min_length;
            } else {
                x_ratio = 1.0;
            }
            if node.rect.size.height < min_length {
                y_ratio = min_length / node.rect.size.height;
                node.rect.size.height = min_length;
            } else {
                y_ratio = 1.0;
            }
        }

        if let Some(current_font_size) = node.font_size {
            const FONT_SIZES: [u32; 7] = [13, 14, 16, 20, 24, 28, 32];
            let min_ratio = x_ratio.min(y_ratio);
            let mut closest_distance = f64::INFINITY;
            let mut best_ratio = 1.0;
            let mut font_size = current_font_size;
            for candidate in FONT_SIZES {
                let font_ratio = f64::from(candidate) / f64::from(current_font_size);
                let distance = (font_ratio - min_ratio).abs();
                if distance < closest_distance {
                    closest_distance = distance;
                    best_ratio = font_ratio;
                    font_size = candidate;
                }
            }
            if best_ratio > min_ratio {
                min_length = (min_length * best_ratio / min_ratio).ceil();
                if node.rect.size.width < min_length {
                    node.rect.size.width = min_length;
                }
                if node.rect.size.height < min_length {
                    node.rect.size.height = min_length;
                }
            }
            node.font_size = Some(font_size);
            if let Some(label) = node.label_size.as_mut() {
                label.width = (label.width * best_ratio).ceil();
                label.height = (label.height * best_ratio).ceil();
            }
        }
    }

    pub(super) fn prescale(&mut self, fixed_sizes: bool) {
        for index in 0..self.nodes.len() {
            let node = &mut self.nodes[index];
            if node.shape.has_unit_aspect_ratio() {
                let size = node.rect.size.width.max(node.rect.size.height);
                node.rect.size.width = size;
                node.rect.size.height = size;
            }
            if !fixed_sizes {
                self.scale_node_based_on_edges(NodeId(index as u32));
            }
        }
    }
}

#[cfg(test)]
mod precision_tests {
    use super::compaction_precision_compare;
    use std::cmp::Ordering;

    #[test]
    fn compaction_score_comparison_uses_tala_geo_precision() {
        assert_eq!(
            compaction_precision_compare(10.0 - 0.000_05, 10.0),
            Ordering::Equal
        );
        assert_eq!(
            compaction_precision_compare(10.0 - 0.000_2, 10.0),
            Ordering::Less
        );
    }
}
