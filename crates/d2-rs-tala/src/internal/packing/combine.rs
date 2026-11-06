// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Component packing and actual per-call work accounting.
use super::*;

struct CombineContext;
impl limits::Context for CombineContext {
    fn done(&self) -> Option<bool> {
        crate::internal::cancellation_done()
    }
    fn error(&self) -> Option<limits::ContextError> {
        crate::internal::cancellation_error()
    }
}

/// Positions are restored in place so node, route and retained-view storage
/// keeps its original allocation and capacity across a failed packing call.
struct CombineGeometry {
    nodes: Vec<(Option<Point>, Point)>,
    routes: Vec<route_points::RouteSnapshot>,
    pending_vessels: Vec<Point>,
}

impl CombineGeometry {
    fn capture(graph: &ArenaGraph) -> Self {
        Self {
            nodes: graph
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
            routes: graph
                .edge_records
                .iter()
                .map(|edge| edge.route.snapshot())
                .collect(),
            pending_vessels: graph
                .pending_cluster_vessel_positions
                .values()
                .copied()
                .collect(),
        }
    }

    fn restore(self, graph: &mut ArenaGraph) {
        for (node, (position, origin)) in graph
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
            )
            .zip(self.nodes)
        {
            node.position = position;
            node.rect.origin = origin;
        }
        for (edge, points) in graph.edge_records.iter_mut().zip(self.routes) {
            edge.route.replace(points.restore());
        }
        for (point, original) in graph
            .pending_cluster_vessel_positions
            .values_mut()
            .zip(self.pending_vessels)
        {
            *point = original;
        }
    }
}

impl ArenaGraph {
    /// Recovered `CombineSubgraphs` placement for the flat graph represented by
    /// this arena. Each independently optimized component has its own local
    /// coordinate system; TALA packs those component bounds into a shared one.
    pub(super) fn combine_placement_components(
        &mut self,
        sort_by_area: bool,
        root_only: bool,
        recovered_node_bounds: bool,
    ) -> Result<(), limits::Error> {
        let mut components = if root_only {
            Vec::new()
        } else {
            self.split_placement_subgraphs()
        };
        for node in self.nodes.iter().map(|node| node.input_id) {
            if self.position(node).is_some()
                && (!root_only || self.nodes[node.0 as usize].container.is_none())
                && !components.iter().any(|component| component.contains(&node))
            {
                components.push(vec![node]);
            }
        }
        self.combine_known_placement_components(
            components,
            sort_by_area,
            root_only,
            recovered_node_bounds,
        )?;
        Ok(())
    }

    /// Recovered `CombineSubgraphs` over the concrete graph partition returned
    /// by the caller's earlier `SplitSubgraphs` invocation.
    ///
    /// `Graph.placeNodes` retains those graph objects while hierarchy and tree
    /// placement mutate their contents, then passes the retained slice to
    /// `CombineSubgraphs`. Re-running reachability at this boundary can merge
    /// originally independent graphs and suppress their component packing.
    pub(super) fn combine_known_placement_components(
        &mut self,
        components: Vec<Vec<NodeId>>,
        sort_by_area: bool,
        root_only: bool,
        recovered_node_bounds: bool,
    ) -> Result<Vec<NodeId>, limits::Error> {
        self.combine_known_placement_components_with_obstacles(
            components,
            sort_by_area,
            root_only,
            recovered_node_bounds,
            &[],
        )
    }

    /// `placeNodes` passes the same ancestor boxes to the optimizer and to
    /// packing. Avoiding those boxes takes priority over the area heuristic.
    pub(super) fn combine_known_placement_components_with_obstacles(
        &mut self,
        components: Vec<Vec<NodeId>>,
        sort_by_area: bool,
        root_only: bool,
        recovered_node_bounds: bool,
        ancestor_obstacles: &[Rect],
    ) -> Result<Vec<NodeId>, limits::Error> {
        let context = CombineContext;
        let mut guard = limits::WorkGuard::new(
            Some(&context),
            "CombineSubgraphs",
            limits::MAX_ENGINE_WORK_UNITS,
        )?;
        self.combine_known_placement_components_with_guard(
            components,
            sort_by_area,
            root_only,
            recovered_node_bounds,
            ancestor_obstacles,
            &context,
            &mut guard,
        )
    }

    /// The guard belongs to one actual packing call. Geometry is only
    /// published when the complete operation succeeds; an interrupted move
    /// restores the pre-call arena, including retained external geometry.
    pub(super) fn combine_known_placement_components_with_guard(
        &mut self,
        components: Vec<Vec<NodeId>>,
        sort_by_area: bool,
        root_only: bool,
        recovered_node_bounds: bool,
        ancestor_obstacles: &[Rect],
        snapshot_context: &dyn limits::Context,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<Vec<NodeId>, limits::Error> {
        let original = CombineGeometry::capture(self);
        let result = self.combine_placement_components_inner(
            components,
            sort_by_area,
            root_only,
            recovered_node_bounds,
            ancestor_obstacles,
            snapshot_context,
            guard,
        );
        if result.is_err() {
            original.restore(self);
        }
        result
    }

    fn combine_component_edges(&self, component: &[NodeId]) -> Vec<EdgeId> {
        let members = component.iter().copied().collect::<BTreeSet<_>>();
        self.edge_order
            .read()
            .iter()
            .copied()
            .filter(|id| {
                let edge = &self.edge_records[id.0 as usize];
                members.contains(&edge.from) && members.contains(&edge.to)
            })
            .collect()
    }

    fn combine_placement_components_inner(
        &mut self,
        mut components: Vec<Vec<NodeId>>,
        sort_by_area: bool,
        root_only: bool,
        recovered_node_bounds: bool,
        ancestor_obstacles: &[Rect],
        snapshot_context: &dyn limits::Context,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<Vec<NodeId>, limits::Error> {
        // combine.go153–202: charge each graph's actual node/edge slice,
        // and capture each distinct edge's route once across shared views.
        let mut captured_edges = BTreeSet::new();
        for (index, component) in std::iter::once(self.graph_node_order())
            .chain(components.iter().cloned())
            .enumerate()
        {
            for _ in &component {
                guard.step()?;
            }
            let edges = if index == 0 {
                self.edge_order.read()
            } else {
                self.combine_component_edges(&component)
            };
            for edge in edges {
                guard.step()?;
                if captured_edges.insert(edge) {
                    for _ in &self.edge_records[edge.0 as usize].route.read() {
                        guard.step()?;
                    }
                }
            }
        }
        // snapshotCombineNodePositions uses its own fresh work envelope.
        let mut snapshot_guard = limits::WorkGuard::new(
            Some(snapshot_context),
            "CombineSubgraphs",
            limits::MAX_ENGINE_WORK_UNITS,
        )?;
        let mut captured_nodes = BTreeSet::new();
        for node in self
            .graph_node_order()
            .into_iter()
            .chain(components.iter().flatten().copied())
        {
            for descendant in std::iter::once(node).chain(self.active_descendants(node)) {
                if captured_nodes.insert(descendant) {
                    snapshot_guard.step()?;
                }
            }
        }
        snapshot_guard.finish()?;
        if components.is_empty() {
            return Ok(Vec::new());
        }

        let bounds = |graph: &Self, component: &[NodeId]| -> (Point, Point) {
            // Go's CombineSubgraphs calls Graph.getBoundingBox for every
            // component. That begins with Nodes.getFixedBoundingBox (the
            // node-level label/icon/loop-aware bounds), even for ordinary
            // edge-bearing graphs; it is not conditional on the caller's
            // rendering-mode flag. Keep the flag for the separate modifier
            // accounting below, but use the recovered node bounds as the
            // placement envelope in every mode.
            if let Some((mut top_left, mut bottom_right)) = graph.fixed_node_bounds(component) {
                if crate::internal::trace_env_enabled("TALA_TRACE_COMBINE_BOUNDS")
                    && !component.is_empty()
                {
                    eprintln!(
                        "COMBINE_BOUNDS_RUST component_len={} fixed={:?}->{:?}",
                        component.len(),
                        top_left,
                        bottom_right
                    );
                    for node in component {
                        eprintln!(
                            "COMBINE_BOUND_NODE_RUST tala={} pos={:?} size={:?} loops={:?} label={:?}",
                            graph.nodes[node.0 as usize].tala_id,
                            graph.position(*node),
                            graph.nodes[node.0 as usize].rect.size,
                            graph.loop_spacing_extents(*node),
                            graph.nodes[node.0 as usize]
                                .external_label
                                .as_ref()
                                .map(|label| (label.side, label.reserve_space, label.automatic)),
                        );
                        eprintln!(
                            "COMBINE_BOUND_SINGLE_RUST tala={} single={:?}",
                            graph.nodes[node.0 as usize].tala_id,
                            graph.fixed_node_bounds(&[*node]),
                        );
                    }
                }
                // Graph.getBoundingBox expands the fixed node envelope with
                // every routed edge in the temporary graph. Most placement
                // scopes have no points yet, but retaining this merge is
                // important for routed disconnected components and costs
                // nothing when an edge has an empty point list.
                let members = component.iter().copied().collect::<BTreeSet<_>>();
                for edge_id in &graph.edge_order.read() {
                    let edge = &graph.edge_records[edge_id.0 as usize];
                    if !members.contains(&edge.from) || !members.contains(&edge.to) {
                        continue;
                    }
                    if let Some((edge_top_left, edge_bottom_right)) =
                        bounds::routed_edge_bounds(edge)
                    {
                        top_left.x = top_left.x.min(edge_top_left.x);
                        top_left.y = top_left.y.min(edge_top_left.y);
                        bottom_right.x = bottom_right.x.max(edge_bottom_right.x);
                        bottom_right.y = bottom_right.y.max(edge_bottom_right.y);
                    }
                }
                // Graph.getBoundingBox rounds the aggregate extrema after it
                // includes routed edges. Node.getBoundingBox itself keeps
                // fractional label geometry, so round only at this outer
                // graph boundary.
                return (
                    Point {
                        x: top_left.x.round(),
                        y: top_left.y.round(),
                    },
                    Point {
                        x: bottom_right.x.round(),
                        y: bottom_right.y.round(),
                    },
                );
            }
            component.iter().fold(
                (
                    Point {
                        x: f64::INFINITY,
                        y: f64::INFINITY,
                    },
                    Point {
                        x: f64::NEG_INFINITY,
                        y: f64::NEG_INFINITY,
                    },
                ),
                |(mut top_left, mut bottom_right), node| {
                    let position = graph.position(*node).unwrap();
                    let node_ref = &graph.nodes[node.0 as usize];
                    let size = node_ref.rect.size;
                    let (modifier_x, modifier_y) = if recovered_node_bounds && node_ref.is_3d {
                        (
                            15.0,
                            if node_ref.shape == ShapeKind::Hexagon {
                                7.0
                            } else {
                                15.0
                            },
                        )
                    } else if recovered_node_bounds && node_ref.is_multiple {
                        (10.0, 10.0)
                    } else {
                        (0.0, 0.0)
                    };
                    // Recovered `Node.getBoundingBox(allNodes)`. TALA does not
                    // simply union the label rectangle with the node: it only
                    // expands an axis when the label's top-left crosses that
                    // node boundary, and gives graph-boundary labels five
                    // rather than ten units of additional padding.
                    let mut left = position.x;
                    let mut top = position.y;
                    let mut right = (position.x + size.width).round();
                    let mut bottom = (position.y + size.height).round();
                    if let Some(label) = node_ref
                        .external_label
                        .filter(|label| label.reserve_space && graph.edge_records.is_empty())
                    {
                        let label_top_left = label.top_left(
                            Rect {
                                origin: position,
                                size,
                            },
                            5.0,
                        );
                        let leftmost = component.iter().copied().all(|other| {
                            other == *node
                                || graph
                                    .position(other)
                                    .is_none_or(|other_position| other_position.x >= position.x)
                        });
                        let topmost = component.iter().copied().all(|other| {
                            other == *node
                                || graph
                                    .position(other)
                                    .is_none_or(|other_position| other_position.y >= position.y)
                        });
                        let rightmost = component.iter().copied().all(|other| {
                            other == *node
                                || graph.position(other).is_none_or(|other_position| {
                                    let other_size = graph.nodes[other.0 as usize].rect.size;
                                    other_position.x + other_size.width <= position.x + size.width
                                })
                        });
                        let bottommost = component.iter().copied().all(|other| {
                            other == *node
                                || graph.position(other).is_none_or(|other_position| {
                                    let other_size = graph.nodes[other.0 as usize].rect.size;
                                    other_position.y + other_size.height <= position.y + size.height
                                })
                        });
                        if label_top_left.x < left {
                            left = (label_top_left.x - if leftmost { 5.0 } else { 10.0 }).floor();
                        }
                        if label_top_left.y < top {
                            top = (label_top_left.y - if topmost { 5.0 } else { 10.0 }).floor();
                        }
                        if label_top_left.x > right {
                            right = (label_top_left.x
                                + label.size.width
                                + if rightmost { 5.0 } else { 10.0 })
                            .ceil();
                        }
                        if label_top_left.y > bottom {
                            bottom = (label_top_left.y
                                + label.size.height
                                + if bottommost { 5.0 } else { 10.0 })
                            .ceil();
                        }
                    }
                    top_left.x = top_left.x.min(left);
                    top_left.y = top_left.y.min(top - modifier_y);
                    bottom_right.x = bottom_right.x.max(right + modifier_x);
                    bottom_right.y = bottom_right.y.max(bottom);
                    (top_left, bottom_right)
                },
            )
        };
        let area =
            |graph: &Self, component: &[NodeId]| -> f64 { graph.component_graph_area(component) };

        let trace_combine = crate::internal::trace_env_enabled("TALA_TRACE_COMBINE");
        if trace_combine {
            eprint!(
                "COMBINE_RUST root_only={} sort_by_area={} components=",
                root_only, sort_by_area
            );
            for component in &components {
                let (top_left, bottom_right) = bounds(self, component);
                eprint!(
                    "[{:?} {:?}->{:?} area={}] ",
                    component
                        .iter()
                        .map(|n| self.nodes[n.0 as usize].tala_id)
                        .collect::<Vec<_>>(),
                    top_left,
                    bottom_right,
                    area(self, component)
                );
            }
            eprintln!();
        }

        // SplitSubgraphs coalesces every fixed node and everything reachable
        // from those nodes into the leading graph. Consume that concrete
        // partition directly; rebuilding it from `optimizable_components`
        // loses the fixed nodes by definition.
        let fixed_component = if !root_only {
            components
                .first()
                .is_some_and(|component| {
                    component
                        .iter()
                        .any(|node| self.nodes[node.0 as usize].fixed_top_left.is_some())
                })
                .then(|| components.remove(0))
        } else {
            let component_nodes = components
                .iter()
                .flatten()
                .copied()
                .collect::<BTreeSet<_>>();
            let mut fixed_members = component_nodes
                .iter()
                .copied()
                .filter(|node| self.nodes[node.0 as usize].fixed_top_left.is_some())
                .collect::<BTreeSet<_>>();
            let mut queue = fixed_members.iter().copied().collect::<VecDeque<_>>();
            while let Some(node) = queue.pop_front() {
                let node_ref = &self.nodes[node.0 as usize];
                let incident_edges = node_ref.edges.read();
                let adjacent = incident_edges
                    .iter()
                    .copied()
                    .map(|edge| self.adjacent(node, edge))
                    .chain(node_ref.nears.keys())
                    .chain(self.nodes.iter().filter_map(|candidate| {
                        candidate
                            .nears
                            .contains(&node)
                            .then_some(candidate.input_id)
                    }));
                for candidate in adjacent {
                    if component_nodes.contains(&candidate) && fixed_members.insert(candidate) {
                        queue.push_back(candidate);
                    }
                }
            }
            (!fixed_members.is_empty()).then(|| {
                let fixed_component = self
                    .nodes
                    .iter()
                    .map(|node| node.input_id)
                    .filter(|node| fixed_members.contains(node))
                    .collect::<Vec<_>>();
                for component in &mut components {
                    component.retain(|node| !fixed_members.contains(node));
                }
                components.retain(|component| !component.is_empty());
                fixed_component
            })
        };

        let mut combined = Vec::<NodeId>::new();
        let mut combined_roots = Vec::<NodeId>::new();
        let mut combined_bottom_right = Point {
            x: f64::NEG_INFINITY,
            y: f64::NEG_INFINITY,
        };
        let mut candidate_points = Vec::<Point>::new();

        // Recovered `CombineSubgraphs`: `SplitSubgraphs` places the coalesced
        // fixed graph first. It is copied into the combined graph without any
        // translation, then its bottom-right seeds the placement candidates
        // for the movable graphs. Only the remaining graphs are area-sorted.
        if let Some(component) = fixed_component {
            let (_, bottom_right) = bounds(self, &component);
            let component_edges = self.combine_component_edges(&component);
            for node in component {
                guard.step()?;
                combined_roots.push(node);
                combined.push(node);
                let descendants = self.active_descendants(node);
                if crate::internal::trace_env_enabled("TALA_TRACE_COMBINE_DESC")
                    && self.nodes[node.0 as usize].tala_id == 6440086189707786506
                {
                    eprintln!(
                        "COMBINE_RUST_DESC parent={} container={} scoring_container={} cluster={:?} active_cluster={:?} sequence={:?} external_children={} aggregate_children={} members={:?} member_indices={:?} descendants={:?}",
                        self.nodes[node.0 as usize].tala_id,
                        self.nodes[node.0 as usize].is_container,
                        self.nodes[node.0 as usize].scoring_is_container,
                        self.nodes[node.0 as usize].cluster,
                        self.active_cluster_index(node),
                        self.active_sequence_index(node),
                        self.transaction_external_container_children
                            .contains_key(&self.nodes[node.0 as usize].tala_id),
                        self.transaction_external_aggregate_children
                            .contains_key(&self.nodes[node.0 as usize].tala_id),
                        self.transaction_external_aggregate_children
                            .get(&self.nodes[node.0 as usize].tala_id)
                            .map(|members| members
                                .iter()
                                .map(|member| member.tala_id)
                                .collect::<Vec<_>>()),
                        self.transaction_external_aggregate_children
                            .get(&self.nodes[node.0 as usize].tala_id)
                            .map(|members| members
                                .iter()
                                .map(|member| self
                                    .nodes
                                    .iter()
                                    .position(|candidate| candidate.tala_id == member.tala_id))
                                .collect::<Vec<_>>()),
                        descendants
                            .iter()
                            .map(|descendant| self.nodes[descendant.0 as usize].tala_id)
                            .collect::<Vec<_>>()
                    );
                }
                for descendant in descendants {
                    guard.step()?;
                    if !combined.contains(&descendant) {
                        combined.push(descendant);
                    }
                }
            }
            for _ in component_edges {
                guard.step()?;
            }
            combined_bottom_right.x = combined_bottom_right.x.max(bottom_right.x);
            combined_bottom_right.y = combined_bottom_right.y.max(bottom_right.y);
            candidate_points.extend([
                Point {
                    x: bottom_right.x,
                    y: 0.0,
                },
                bottom_right,
                Point {
                    x: 0.0,
                    y: bottom_right.y,
                },
                Point {
                    x: bottom_right.x + 20.0,
                    y: 0.0,
                },
                Point {
                    x: bottom_right.x + 20.0,
                    y: bottom_right.y,
                },
                Point {
                    x: 0.0,
                    y: bottom_right.y + 20.0,
                },
            ]);
        }

        if sort_by_area {
            // Recovered CombineSubgraphs uses sort.Slice with descending graph
            // area. Graph.BinPack consumes its contained subgraphs in their
            // existing order instead, so its caller disables this ordering.
            go_sort::sort_by(&mut components, |left, right| {
                area(self, left) > area(self, right)
            });
            if trace_combine {
                eprint!("COMBINE_RUST_SORTED");
                for component in &components {
                    eprint!(
                        " [{}]",
                        component
                            .first()
                            .map_or(0, |n| self.nodes[n.0 as usize].tala_id)
                    );
                }
                eprintln!();
            }
        }
        guard.finish()?;
        for component in components {
            guard.step()?;
            let (top_left, bottom_right) = bounds(self, &component);
            let width = bottom_right.x - top_left.x;
            let height = bottom_right.y - top_left.y;
            let mut best_cost = f64::INFINITY;
            let mut best_overlaps = i64::MAX;
            let mut best_point = Point::default();
            let mut best_index = 0;

            for (index, point) in candidate_points.iter().copied().enumerate() {
                guard.step()?;
                let delta = Point {
                    x: point.x - top_left.x,
                    y: point.y - top_left.y,
                };
                #[cfg(feature = "diagnostic-traces")]
                let mut collision_pair = None;
                let mut collides = false;
                let mut ancestor_overlaps = 0_i64;
                for node in &component {
                    guard.step()?;
                    // Go performs the candidate translation and its inverse;
                    // floating-point rounding from those moves is observable.
                    self.translate_node_with_children(*node, delta);
                    let size = self.nodes[node.0 as usize].rect.size;
                    let moved = self.position(*node).unwrap();
                    for other in &combined {
                        guard.step()?;
                        let other_position = self.position(*other).unwrap();
                        let other_size = self.nodes[other.0 as usize].rect.size;
                        // Recovered `CombineSubgraphs` always delegates the
                        // collision test to `Node.doesOverlap`, whose
                        // `getDeltaTo` spacing applies to disconnected
                        // components regardless of whether the graph has
                        // edges. The old edge/no-edge split was a candidate
                        // shortcut that silently removed margins and table
                        // spacing on ordinary graphs.
                        // `Node.DoesOverlap` delegates to `deltaTo`, which
                        // includes cached self-loop extents in addition to
                        // ordinary edge/label spacing.  Keep the combine
                        // collision predicate on that same path so a loop's
                        // reserved lane is honored while packing components.
                        let delta = self.spacing_delta_with_loops(*node, *other, moved);
                        let overlaps = moved.x < other_position.x + other_size.width + delta
                            && other_position.x < moved.x + size.width + delta
                            && moved.y < other_position.y + other_size.height + delta
                            && other_position.y < moved.y + size.height + delta;
                        if overlaps {
                            #[cfg(feature = "diagnostic-traces")]
                            {
                                collision_pair = Some((
                                    self.nodes[node.0 as usize].tala_id,
                                    self.nodes[other.0 as usize].tala_id,
                                    delta,
                                    moved,
                                    other_position,
                                ));
                            }
                            collides = true;
                            break;
                        }
                        // Synthetic cluster/sequence vessels retain their
                        // hidden Go child pointers outside the stable arena.
                        // CombineSubgraphs adds those descendants to the
                        // temporary combined graph, so inspect their boxes
                        // here even though they have no stable NodeId.
                        for external in self
                            .transaction_external_aggregate_children
                            .get(&self.nodes[other.0 as usize].tala_id)
                            .into_iter()
                            .flatten()
                            .chain(
                                self.transaction_external_container_children
                                    .get(&self.nodes[other.0 as usize].tala_id)
                                    .into_iter()
                                    .flatten(),
                            )
                        {
                            guard.step()?;
                            let Some(external_position) = external.position else {
                                continue;
                            };
                            let external_delta = self.spacing_delta_for_geometry(
                                moved,
                                self.nodes[node.0 as usize].rect.size,
                                external_position,
                                external.rect.size,
                                self.nodes[node.0 as usize].shape == ShapeKind::SqlTable
                                    || external.shape == ShapeKind::SqlTable,
                                self.nodes[node.0 as usize].layout_margins,
                                external.layout_margins,
                                (false, f64::NEG_INFINITY, f64::NEG_INFINITY),
                            );
                            let external_delta = Self::spacing_delta_with_loop_geometry(
                                external_delta,
                                moved,
                                self.nodes[node.0 as usize].rect.size,
                                external_position,
                                external.rect.size,
                                self.nodes[node.0 as usize].loop_offsets.as_ref(),
                                external.loop_offsets.as_ref(),
                            );
                            let overlaps = moved.x
                                < external_position.x + external.rect.size.width + external_delta
                                && external_position.x
                                    < moved.x
                                        + self.nodes[node.0 as usize].rect.size.width
                                        + external_delta
                                && moved.y
                                    < external_position.y
                                        + external.rect.size.height
                                        + external_delta
                                && external_position.y
                                    < moved.y
                                        + self.nodes[node.0 as usize].rect.size.height
                                        + external_delta;
                            if overlaps {
                                #[cfg(feature = "diagnostic-traces")]
                                {
                                    collision_pair = Some((
                                        self.nodes[node.0 as usize].tala_id,
                                        external.tala_id,
                                        external_delta,
                                        moved,
                                        external_position,
                                    ));
                                }
                                collides = true;
                                break;
                            }
                        }
                        if collides {
                            break;
                        }
                    }
                    if !collides {
                        for obstacle in ancestor_obstacles {
                            // Source charges even the translated root's own
                            // box, then skips its overlap predicate.
                            guard.step()?;
                            if obstacle.origin.x == -CONTAINER_PADDING
                                && obstacle.origin.y == -CONTAINER_PADDING
                            {
                                continue;
                            }
                            // geo.Box.Overlaps uses strict edges and the raw
                            // dimensions, without spacing or normalization.
                            if moved.x < obstacle.origin.x + obstacle.size.width
                                && moved.x + size.width > obstacle.origin.x
                                && moved.y < obstacle.origin.y + obstacle.size.height
                                && moved.y + size.height > obstacle.origin.y
                            {
                                ancestor_overlaps += 1;
                            }
                        }
                    }
                    self.translate_node_with_children(
                        *node,
                        Point {
                            x: -delta.x,
                            y: -delta.y,
                        },
                    );
                    if collides {
                        break;
                    }
                }
                #[cfg(feature = "diagnostic-traces")]
                if crate::internal::trace_env_value("TALA_TRACE_COMBINE_CANDIDATE")
                    .as_deref()
                    .is_some_and(|target| {
                        target == "all"
                            || component.first().is_some_and(|node| {
                                target == self.nodes[node.0 as usize].tala_id.to_string()
                            })
                    })
                {
                    eprintln!(
                        "COMBINE_RUST_CANDIDATE point={:?} collides={} pair={:?}",
                        point, collides, collision_pair
                    );
                }
                if collides || ancestor_overlaps > best_overlaps {
                    continue;
                }
                let new_width = combined_bottom_right.x.max(point.x + width);
                let new_height = combined_bottom_right.y.max(point.y + height);
                let cost = new_width * new_height + 0.5 * (new_width - new_height).powi(2);
                if ancestor_overlaps < best_overlaps
                    || (ancestor_overlaps == best_overlaps && cost < best_cost)
                {
                    best_overlaps = ancestor_overlaps;
                    best_cost = cost;
                    best_point = point;
                    best_index = index;
                }
            }

            if trace_combine {
                eprintln!(
                    "COMBINE_CHOICE_RUST component={:?} bounds={:?}->{:?} candidates={} best={:?} cost={}",
                    component
                        .iter()
                        .map(|n| self.nodes[n.0 as usize].tala_id)
                        .collect::<Vec<_>>(),
                    top_left,
                    bottom_right,
                    candidate_points.len(),
                    best_point,
                    best_cost
                );
            }

            if !candidate_points.is_empty() {
                candidate_points.swap_remove(best_index);
            }
            let delta = Point {
                x: best_point.x - top_left.x,
                y: best_point.y - top_left.y,
            };
            for node in component.iter().copied() {
                guard.step()?;
                self.translate_node_with_children(node, delta);
                combined_roots.push(node);
                combined.push(node);
                let descendants = self.active_descendants(node);
                if crate::internal::trace_env_enabled("TALA_TRACE_COMBINE_DESC")
                    && self.nodes[node.0 as usize].tala_id == 6440086189707786506
                {
                    eprintln!(
                        "COMBINE_RUST_DESC parent={} container={} scoring_container={} cluster={:?} active_cluster={:?} sequence={:?} external_children={} aggregate_children={} members={:?} member_indices={:?} descendants={:?}",
                        self.nodes[node.0 as usize].tala_id,
                        self.nodes[node.0 as usize].is_container,
                        self.nodes[node.0 as usize].scoring_is_container,
                        self.nodes[node.0 as usize].cluster,
                        self.active_cluster_index(node),
                        self.active_sequence_index(node),
                        self.transaction_external_container_children
                            .contains_key(&self.nodes[node.0 as usize].tala_id),
                        self.transaction_external_aggregate_children
                            .contains_key(&self.nodes[node.0 as usize].tala_id),
                        self.transaction_external_aggregate_children
                            .get(&self.nodes[node.0 as usize].tala_id)
                            .map(|members| members
                                .iter()
                                .map(|member| member.tala_id)
                                .collect::<Vec<_>>()),
                        self.transaction_external_aggregate_children
                            .get(&self.nodes[node.0 as usize].tala_id)
                            .map(|members| members
                                .iter()
                                .map(|member| self
                                    .nodes
                                    .iter()
                                    .position(|candidate| candidate.tala_id == member.tala_id))
                                .collect::<Vec<_>>()),
                        descendants
                            .iter()
                            .map(|descendant| self.nodes[descendant.0 as usize].tala_id)
                            .collect::<Vec<_>>()
                    );
                }
                for descendant in descendants {
                    guard.step()?;
                    if !combined.contains(&descendant) {
                        combined.push(descendant);
                    }
                }
            }
            for edge in self.combine_component_edges(&component) {
                guard.step()?;
                for mut point in self.edge_records[edge.0 as usize].route.iter_mut() {
                    guard.step()?;
                    point.x += delta.x;
                    point.y += delta.y;
                }
            }
            let placed_top_left = Point {
                x: top_left.x + delta.x,
                y: top_left.y + delta.y,
            };
            let placed_bottom_right = Point {
                x: bottom_right.x + delta.x,
                y: bottom_right.y + delta.y,
            };
            combined_bottom_right.x = combined_bottom_right.x.max(placed_bottom_right.x);
            combined_bottom_right.y = combined_bottom_right.y.max(placed_bottom_right.y);
            candidate_points.extend([
                Point {
                    x: placed_bottom_right.x,
                    y: placed_top_left.y,
                },
                placed_bottom_right,
                Point {
                    x: placed_top_left.x,
                    y: placed_bottom_right.y,
                },
                Point {
                    x: placed_bottom_right.x + 20.0,
                    y: placed_top_left.y,
                },
                Point {
                    x: placed_bottom_right.x + 20.0,
                    y: placed_bottom_right.y,
                },
                Point {
                    x: placed_top_left.x,
                    y: placed_bottom_right.y + 20.0,
                },
            ]);
        }
        // CombineSubgraphs returns a temporary Graph. TALA copies its
        // placements back into the master graph, but does not replace the
        // master's Graph.Nodes slice with the temporary packed order. Keep
        // that ownership order intact: SwapStuff and later stages iterate the
        // master slice, while `combined` exists only to choose translations.
        guard.finish()?;
        self.refresh_placement_components();
        Ok(combined_roots)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prepared(count: u32) -> (ArenaGraph, Vec<Vec<NodeId>>) {
        let mut input = Graph::default();
        for index in 0..count {
            input.add_node(crate::Node::new(
                index.to_string(),
                Size {
                    width: 40.0,
                    height: 30.0,
                },
            ));
        }
        let mut arena = ArenaGraph::from_input(&input);
        for index in 0..count {
            arena.set_position(NodeId(index), Point::default());
        }
        (arena, (0..count).map(|i| vec![NodeId(i)]).collect())
    }

    #[test]
    fn combine_charges_the_operations_in_the_source_loop() {
        // Counts observed from pristine v0.9.0 with an inert Used observer.
        for (count, work) in [
            (0, 0),
            (1, 4),
            (2, 26),
            (3, 71),
            (10, 1553),
            (25, 18697),
            (80, 506914),
        ] {
            let (mut arena, components) = prepared(count);
            let context = CombineContext;
            let mut guard = limits::WorkGuard::new(
                Some(&context),
                "CombineSubgraphs",
                limits::MAX_ENGINE_WORK_UNITS,
            )
            .unwrap();
            arena
                .combine_known_placement_components_with_guard(
                    components,
                    true,
                    false,
                    true,
                    &[],
                    &context,
                    &mut guard,
                )
                .unwrap();
            assert_eq!(guard.used(), work, "nodes={count}");
        }
    }

    #[test]
    fn combine_restores_geometry_and_route_storage_at_every_rejected_unit() {
        for limit in 0..33 {
            let (mut arena, components) = prepared(2);
            arena.nodes[0].rect.size = Size {
                width: 100.0,
                height: 100.0,
            };
            arena.nodes[1].rect.size = Size {
                width: 10.0,
                height: 10.0,
            };
            arena.set_position(NodeId(0), Point { x: 100.0, y: 100.0 });
            let mut input = Graph::default();
            input.add_node(crate::Node::new("first", arena.nodes[0].rect.size));
            input.add_node(crate::Node::new("second", arena.nodes[1].rect.size));
            input.add_edge(crate::Edge {
                source: NodeId(0),
                target: NodeId(0),
            });
            let mut routed = ArenaGraph::from_input(&input);
            routed.set_position(NodeId(0), Point { x: 100.0, y: 100.0 });
            routed.set_position(NodeId(1), Point::default());
            routed.edge_records[0].route.replace(RoutePoints::literal(&[
                Point { x: 100.0, y: 100.0 },
                Point { x: 150.0, y: 175.0 },
            ]));
            let slots = routed.edge_records[0]
                .route
                .header()
                .iter_slots()
                .collect::<Vec<_>>();
            let backing =
                owner_pointer_slices::SharedPointSlice::with_len_cap(slots.len(), slots.len() + 7)
                    .unwrap();
            for (i, point) in slots.into_iter().enumerate() {
                backing.write_capacity_slot(i, point).unwrap();
            }
            routed.edge_records[0]
                .route
                .replace(RoutePoints::from_header(backing));
            let nodes = routed.nodes.as_ptr();
            let points = routed.edge_records[0].route.backing_identity();
            let capacity = routed.edge_records[0].route.capacity();
            let context = CombineContext;
            let mut guard =
                limits::WorkGuard::new(Some(&context), "CombineSubgraphs", limit).unwrap();
            assert!(matches!(
                routed.combine_known_placement_components_with_guard(
                    components,
                    true,
                    false,
                    true,
                    &[],
                    &context,
                    &mut guard
                ),
                Err(limits::Error::Limit { .. })
            ));
            assert_eq!(guard.used(), limit + 1);
            assert_eq!(routed.nodes.as_ptr(), nodes);
            assert_eq!(routed.edge_records[0].route.backing_identity(), points);
            assert_eq!(routed.edge_records[0].route.capacity(), capacity);
            assert_eq!(
                routed.position(NodeId(0)),
                Some(Point { x: 100.0, y: 100.0 })
            );
            assert_eq!(routed.position(NodeId(1)), Some(Point::default()));
            assert_eq!(
                routed.edge_records[0].route.read(),
                [Point { x: 100.0, y: 100.0 }, Point { x: 150.0, y: 175.0 }]
            );
        }
    }

    #[test]
    fn combine_observes_the_active_context_before_packing() {
        let (mut graph, groups) = prepared(2);
        let token = CancellationToken::new();
        token.expire();
        let result = crate::internal::with_cancellation(Some(&token), || {
            graph.combine_known_placement_components(groups, true, false, true)
        });
        assert!(matches!(
            result,
            Err(limits::Error::Context {
                cause: limits::ContextError::DeadlineExceeded,
                ..
            })
        ));
        assert_eq!(graph.position(NodeId(0)), Some(Point::default()));
    }

    #[test]
    fn combine_prefers_fewer_ancestor_overlaps_before_area() {
        // Pinned Go places the second rectangle to the right, although the
        // unobstructed area heuristic prefers the smaller below placement.
        let (mut graph, groups) = prepared(2);
        graph.nodes[0].rect.size = Size {
            width: 100.0,
            height: 200.0,
        };
        graph.nodes[1].rect.size = Size {
            width: 40.0,
            height: 10.0,
        };
        let obstacle = Rect {
            origin: Point { x: -10.0, y: 210.0 },
            size: Size {
                width: 300.0,
                height: 100.0,
            },
        };
        graph
            .combine_known_placement_components_with_obstacles(
                groups,
                true,
                false,
                true,
                &[obstacle],
            )
            .unwrap();
        assert_eq!(graph.position(NodeId(1)), Some(Point { x: 120.0, y: 0.0 }));
    }

    #[test]
    fn combine_charges_ignored_root_boxes_and_restores_interrupted_candidates() {
        let obstacle = Rect {
            origin: Point { x: -60.0, y: -60.0 },
            size: Size {
                width: 1000.0,
                height: 1000.0,
            },
        };
        // Source uses 29 units with this ignored box, versus 26 without it.
        for limit in [26, 27, 28, 29] {
            let (mut graph, groups) = prepared(2);
            graph.nodes[0].rect.size = Size {
                width: 100.0,
                height: 100.0,
            };
            graph.nodes[1].rect.size = Size {
                width: 10.0,
                height: 10.0,
            };
            let context = CombineContext;
            let mut guard =
                limits::WorkGuard::new(Some(&context), "CombineSubgraphs", limit).unwrap();
            let result = graph.combine_known_placement_components_with_guard(
                groups,
                true,
                false,
                true,
                &[obstacle],
                &context,
                &mut guard,
            );
            if limit == 29 {
                assert!(result.is_ok());
                assert_eq!(guard.used(), 29);
                assert_eq!(graph.position(NodeId(1)), Some(Point { x: 120.0, y: 0.0 }));
            } else {
                assert!(matches!(result, Err(limits::Error::Limit { .. })));
                assert_eq!(guard.used(), limit + 1);
                assert!(
                    graph
                        .nodes
                        .iter()
                        .all(|node| node.position == Some(Point::default()))
                );
            }
        }
    }
}
