// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The pinned v0.9.0 source-container orientation pass.
//!
//! `orientSourceInterior` is a small, optional transform after component
//! combination. It turns a horizontally dominant interior when its container
//! feeds a vertically directed parent flow, then re-compacts and validates
//! the candidate through the same graph transaction predicates used by the
//! placement stages.

use super::*;

#[cfg(feature = "diagnostic-traces")]
fn trace_orientation_boxes(phase: &str, graph: &ArenaGraph, root: &str, nodes: &[NodeId]) {
    if !crate::internal::trace_env_enabled("D2RS_TRACE_ORIENTATION_BOXES") {
        return;
    }
    let boxes = nodes
        .iter()
        .map(|id| {
            let node = &graph.nodes[id.0 as usize];
            let mut value = serde_json::json!({"stable_id":node.tala_id.to_string(),
            "w_bits":format!("0x{:016x}",node.rect.size.width.to_bits()),
            "h_bits":format!("0x{:016x}",node.rect.size.height.to_bits())});
            if let Some(p) = graph.position(*id) {
                value["x_bits"] = serde_json::json!(format!("0x{:016x}", p.x.to_bits()));
                value["y_bits"] = serde_json::json!(format!("0x{:016x}", p.y.to_bits()));
            }
            value
        })
        .collect::<Vec<_>>();
    trace_source_orientation(phase, root, serde_json::json!({"nodes":boxes}));
}

#[cfg(feature = "diagnostic-traces")]
fn trace_source_orientation(phase: &str, root: &str, details: serde_json::Value) {
    if !crate::internal::trace_env_enabled("TALA_TRACE_PLACEMENT")
        || crate::internal::trace_env_value("TALA_TRACE_PLACEMENT_ROOT")
            .is_some_and(|target| target != root)
    {
        return;
    }
    let mut event = serde_json::Map::new();
    event.insert("event".into(), serde_json::json!("source-orientation"));
    event.insert("phase".into(), serde_json::json!(phase));
    event.insert("root".into(), serde_json::json!(root));
    if let Some(details) = details.as_object() {
        event.extend(
            details
                .iter()
                .map(|(key, value)| (key.clone(), value.clone())),
        );
    }
    eprintln!("{}", serde_json::Value::Object(event));
}

#[cfg(feature = "diagnostic-traces")]
fn trace_source_orientation_rollback(root: &str, reason: &str) {
    trace_source_orientation("rollback", root, serde_json::json!({"reason": reason}));
}

#[cfg(feature = "diagnostic-traces")]
fn trace_orientation_direction(direction: Option<Direction>) -> i32 {
    match direction {
        Some(Direction::Up) => 4,
        Some(Direction::Right) => 5,
        Some(Direction::Down) => 6,
        Some(Direction::Left) => 7,
        None => 8,
    }
}

struct ActiveLayoutContext;

impl limits::Context for ActiveLayoutContext {
    fn done(&self) -> Option<bool> {
        crate::internal::cancellation_done()
    }

    fn error(&self) -> Option<limits::ContextError> {
        crate::internal::cancellation_error()
    }
}

pub(super) fn layout_error_for_work(
    error: limits::Error,
    seed: i64,
    location: &'static str,
    limit: i64,
) -> crate::LayoutError {
    match error {
        limits::Error::Context { cause, .. } => match cause {
            limits::ContextError::Cancelled => crate::LayoutError::Cancelled,
            limits::ContextError::DeadlineExceeded => crate::LayoutError::DeadlineExceeded,
        },
        limits::Error::Limit { .. } => crate::LayoutError::WorkLimit {
            seed,
            location,
            limit: limit as u64,
        },
        limits::Error::Invalid(message) => crate::LayoutError::BadRouteState(message),
        limits::Error::SeedInvariant(_) => crate::LayoutError::SeedInvariant,
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct InteriorFlow {
    x: f64,
    y: f64,
    across: f64,
    along: f64,
}

impl ArenaGraph {
    /// Project a node onto its direct child of a placement scope, matching
    /// `HierarchyScope::projected_child_in_scope` over the stable hierarchy.
    fn source_orientation_child_in_scope(
        &self,
        mut node: NodeId,
        scope: Option<NodeId>,
    ) -> Option<NodeId> {
        match scope {
            None => {
                while let Some(parent) = self.nodes.get(node.0 as usize)?.container {
                    node = parent;
                }
                Some(node)
            }
            Some(scope) => loop {
                let parent = self.nodes.get(node.0 as usize)?.container?;
                if parent == scope {
                    return Some(node);
                }
                node = parent;
            },
        }
    }

    /// Reconstruct both `root.Edges`' length and its projected boundary edges
    /// at the `orientSourceInterior` call boundary. Go shares node pointers
    /// across recursive graph values, so the parent scope's `abductEdges` has
    /// already reconnected edges from root descendants to the root container.
    /// The stable arena keeps authored endpoints unchanged; project those same
    /// crossing edges here, preserving root adjacency followed by parent edge
    /// traversal order.
    pub(crate) fn source_orientation_boundary_edges(
        &self,
        root_id: NodeId,
    ) -> Vec<Option<(i64, i64)>> {
        self.source_orientation_boundary_edge_context(root_id).1
    }

    pub(crate) fn source_orientation_boundary_edge_context(
        &self,
        root_id: NodeId,
    ) -> (usize, Vec<Option<(i64, i64)>>) {
        let Some(root) = self.nodes.get(root_id.0 as usize) else {
            return (0, Vec::new());
        };
        if self
            .active_cluster_index(root_id)
            .and_then(|index| self.clusters.get(index))
            .is_some_and(|cluster| self.cluster_is_active(cluster))
        {
            // AddCluster reconnects every live member edge onto its synthetic
            // vessel and removes the member from Graph.Nodes. Go still visits
            // the member container recursively, but its shared Node.Edges
            // slice is empty at this source-orientation callback.
            return (0, Vec::new());
        }
        let parent_scope = self.active_node_container(root_id);
        let mut edge_order = Vec::with_capacity(root.edges.len());
        let mut seen = BTreeSet::new();
        for &edge_id in &root.edges.read() {
            let Some(edge) = self.edge_records.get(edge_id.0 as usize) else {
                continue;
            };
            let from_child = self.source_orientation_child_in_scope(edge.from, parent_scope);
            let to_child = self.source_orientation_child_in_scope(edge.to, parent_scope);
            // If an endpoint is outside this parent scope, an earlier ancestor
            // abducted the edge onto the parent carrier; Go's shared pointer no
            // longer leaves it in this root's adjacency.
            if parent_scope.is_some() && (from_child.is_none() || to_child.is_none()) {
                continue;
            }
            // Go's parent abductEdges removes an edge from the root's shared
            // Node.Edges slice when it connects the root directly to one of
            // its own descendants. It remains internal to the child graph.
            if from_child == Some(root_id) && to_child == Some(root_id) {
                continue;
            }
            if seen.insert(edge_id) {
                edge_order.push(edge_id);
            }
        }
        for edge_id in self.edge_order.read().iter().copied() {
            if seen.contains(&edge_id) {
                continue;
            }
            let Some(edge) = self.edge_records.get(edge_id.0 as usize) else {
                continue;
            };
            let from_child = self.source_orientation_child_in_scope(edge.from, parent_scope);
            let to_child = self.source_orientation_child_in_scope(edge.to, parent_scope);
            let crosses_root_boundary = matches!((from_child, to_child),
                (Some(from), Some(to)) if (from == root_id && to != root_id)
                    || (to == root_id && from != root_id));
            if crosses_root_boundary && seen.insert(edge_id) {
                edge_order.push(edge_id);
            }
        }
        let local_edge_ids = root.edges.read().iter().copied().collect::<BTreeSet<_>>();
        let projected_external_edges = edge_order
            .iter()
            .filter(|edge_id| !local_edge_ids.contains(edge_id))
            .count();
        let root_edge_count = root.edges.len() + projected_external_edges;
        let projected_edges = edge_order
            .into_iter()
            .filter_map(|edge_id| self.edge_records.get(edge_id.0 as usize))
            .map(|edge| {
                let projected_from =
                    self.source_orientation_child_in_scope(edge.from, parent_scope);
                let projected_to = self.source_orientation_child_in_scope(edge.to, parent_scope);
                let (from, to) = match (projected_from, projected_to) {
                    (Some(from), Some(to)) if from == root_id || to == root_id => (
                        self.active_aggregate_owner(from),
                        self.active_aggregate_owner(to),
                    ),
                    _ => (
                        self.active_aggregate_owner(edge.from),
                        self.active_aggregate_owner(edge.to),
                    ),
                };
                if edge.source_arrow == edge.target_arrow {
                    return None;
                }
                let (from, to) = if edge.source_arrow {
                    (to, from)
                } else {
                    (from, to)
                };
                Some((self.active_node_tala_id(from), self.active_node_tala_id(to)))
            })
            .collect();
        (root_edge_count, projected_edges)
    }

    /// Match `orientationContext`'s once-per-Place loop preflight and work
    /// guard before any recursive placement scope is materialized.
    pub(super) fn source_orientation_disabled_for_loop(
        &self,
        seed: i64,
    ) -> Result<bool, crate::LayoutError> {
        let active_context = ActiveLayoutContext;
        self.source_orientation_disabled_for_loop_with_context_and_limit(
            seed,
            &active_context,
            limits::MAX_ENGINE_WORK_UNITS,
        )
    }

    fn source_orientation_disabled_for_loop_with_context_and_limit(
        &self,
        seed: i64,
        work_context: &dyn limits::Context,
        work_limit: i64,
    ) -> Result<bool, crate::LayoutError> {
        let mut work = limits::WorkGuard::new(
            Some(work_context),
            "ContainerOrientationPreflight",
            work_limit,
        )
        .map_err(|error| {
            layout_error_for_work(error, seed, "ContainerOrientationPreflight", work_limit)
        })?;
        for edge_id in &self.edge_order.read() {
            work.step().map_err(|error| {
                layout_error_for_work(error, seed, "ContainerOrientationPreflight", work_limit)
            })?;
            let Some(edge) = self.edge_records.get(edge_id.0 as usize) else {
                return Err(crate::LayoutError::BadRouteState(format!(
                    "ContainerOrientationPreflight references missing edge {}",
                    edge_id.0
                )));
            };
            if edge.from == edge.to {
                return Ok(true);
            }
        }
        work.finish().map_err(|error| {
            layout_error_for_work(error, seed, "ContainerOrientationPreflight", work_limit)
        })?;
        Ok(false)
    }

    /// Apply the source release's optional source-container orientation.
    ///
    /// `source_orientation` was captured from the owning graph before the
    /// temporary direct-child graph was projected. That matters because the
    /// root's boundary edges and its parent direction are intentionally not
    /// members of the combined interior graph.
    pub(super) fn orient_source_interior(
        &mut self,
        context: &SourceOrientationContext,
        seed: i64,
    ) -> Result<bool, crate::LayoutError> {
        let active_context = ActiveLayoutContext;
        self.orient_source_interior_with_context_and_limit(
            context,
            seed,
            &active_context,
            limits::MAX_ENGINE_WORK_UNITS,
        )
    }

    pub(super) fn orient_source_interior_with_owner(
        &mut self,
        context: &SourceOrientationContext,
        seed: i64,
        owner: Option<&mut OwnerScoreSession<'_>>,
    ) -> Result<bool, crate::LayoutError> {
        self.orient_source_interior_impl(
            context,
            seed,
            &ActiveLayoutContext,
            limits::MAX_ENGINE_WORK_UNITS,
            owner,
        )
    }

    fn orient_source_interior_with_context_and_limit(
        &mut self,
        context: &SourceOrientationContext,
        seed: i64,
        work_context: &dyn limits::Context,
        work_limit: i64,
    ) -> Result<bool, crate::LayoutError> {
        self.orient_source_interior_impl(context, seed, work_context, work_limit, None)
    }

    fn orient_source_interior_impl(
        &mut self,
        context: &SourceOrientationContext,
        seed: i64,
        work_context: &dyn limits::Context,
        work_limit: i64,
        owner: Option<&mut OwnerScoreSession<'_>>,
    ) -> Result<bool, crate::LayoutError> {
        if let Some(cause) = work_context.error() {
            return Err(match cause {
                limits::ContextError::Cancelled => crate::LayoutError::Cancelled,
                limits::ContextError::DeadlineExceeded => crate::LayoutError::DeadlineExceeded,
            });
        }
        let Some(root) = context.root.as_ref() else {
            #[cfg(feature = "diagnostic-traces")]
            trace_source_orientation(
                "enter",
                "top",
                serde_json::json!({
                    "root_present": false,
                    "nodes": context.graph_node_count,
                    "obstacles": context.has_ancestor_obstacles,
                    "disabled_for_loop": context.disabled_for_loop,
                }),
            );
            #[cfg(feature = "diagnostic-traces")]
            trace_source_orientation("reject-precondition", "top", serde_json::json!({}));
            return Ok(false);
        };
        #[cfg(feature = "diagnostic-traces")]
        trace_source_orientation(
            "enter",
            &root.external_id,
            serde_json::json!({
                    "root_present": true,
                "nodes": context.graph_node_count,
                "obstacles": context.has_ancestor_obstacles,
                "disabled_for_loop": context.disabled_for_loop,
                "fixed": root.fixed_top_left.is_some(),
                "cluster": root.cluster.is_some(),
                "sequence": root.sequence.is_some(),
                "force_hierarchy": root.force_hierarchy,
                "root_direction": trace_orientation_direction(context.root_direction),
                "parent_direction": trace_orientation_direction(context.parent_direction),
                "root_edges": context.root_adjacency_edge_count,
            }),
        );
        if context.disabled_for_loop
            || !(3..=16).contains(&context.graph_node_count)
            || context.has_ancestor_obstacles
            || root.fixed_top_left.is_some()
            || root.cluster.is_some()
            || root.sequence.is_some()
            || root.force_hierarchy
            || context.root_direction.is_some()
        {
            #[cfg(feature = "diagnostic-traces")]
            trace_source_orientation(
                "reject-precondition",
                &root.external_id,
                serde_json::json!({}),
            );
            return Ok(false);
        }

        let nodes = self.graph_node_order();
        if !matches!(
            context.parent_direction,
            Some(Direction::Down | Direction::Up)
        ) || context.root_adjacency_edge_count < 3
        {
            #[cfg(feature = "diagnostic-traces")]
            trace_source_orientation(
                "reject-boundary",
                &root.external_id,
                serde_json::json!({
                    "parent_direction": trace_orientation_direction(context.parent_direction),
                    "root_edges": context.root_adjacency_edge_count,
                }),
            );
            return Ok(false);
        }

        let mut work =
            limits::WorkGuard::new(Some(work_context), "ContainerOrientation", work_limit)
                .map_err(|error| {
                    layout_error_for_work(error, seed, "ContainerOrientation", work_limit)
                })?;

        let mut destinations = BTreeSet::new();
        for edge in &context.root_edges {
            work.step().map_err(|error| {
                layout_error_for_work(error, seed, "ContainerOrientation", work_limit)
            })?;
            let Some((from, to)) = edge else {
                #[cfg(feature = "diagnostic-traces")]
                trace_source_orientation(
                    "reject-boundary",
                    &root.external_id,
                    serde_json::json!({"reason": "edge-is-not-outbound"}),
                );
                return Ok(false);
            };
            if *from != root.tala_id || *to == root.tala_id {
                #[cfg(feature = "diagnostic-traces")]
                trace_source_orientation(
                    "reject-boundary",
                    &root.external_id,
                    serde_json::json!({"reason": "edge-is-not-outbound"}),
                );
                return Ok(false);
            }
            destinations.insert(*to);
        }
        if destinations.len() < 2 {
            #[cfg(feature = "diagnostic-traces")]
            trace_source_orientation(
                "reject-boundary",
                &root.external_id,
                serde_json::json!({"destinations": destinations.len()}),
            );
            return Ok(false);
        }
        #[cfg(feature = "diagnostic-traces")]
        trace_source_orientation(
            "boundary",
            &root.external_id,
            serde_json::json!({
                "parent_direction": trace_orientation_direction(context.parent_direction),
                "root_edges": context.root_adjacency_edge_count,
                "destinations": destinations.len(),
            }),
        );

        let local = nodes.iter().copied().collect::<BTreeSet<_>>();
        for &node_id in &nodes {
            let node = &self.nodes[node_id.0 as usize];
            if node.is_container
                || node.hierarchy.is_some()
                || node.fixed_top_left.is_some()
                || node.sequence.is_some()
                || self.active_sequence_index(node_id).is_some()
                || self.is_tree_sentinel(node_id)
                || node.force_hierarchy
                || self.has_herd_assignment(node_id)
                || node
                    .loop_offsets
                    .as_ref()
                    .is_some_and(|offsets| offsets.values().any(|offset| *offset > 0.0))
            {
                #[cfg(feature = "diagnostic-traces")]
                trace_source_orientation(
                    "reject-content",
                    &root.external_id,
                    serde_json::json!({
                        "node": node.external_id,
                        "reason": "unsupported-node",
                        "container": node.is_container,
                        "hierarchy": node.hierarchy.is_some(),
                        "fixed": node.fixed_top_left.is_some(),
                        "sequence": node.sequence.is_some(),
                        "sequence_vessel": self.active_sequence_index(node_id).is_some(),
                        "tree_sentinel": self.is_tree_sentinel(node_id),
                        "force_hierarchy": node.force_hierarchy,
                        "herd": node.herd_assignment.is_some(),
                    }),
                );
                return Ok(false);
            }
            if node.nears.keys().iter().any(|near| !local.contains(near)) {
                #[cfg(feature = "diagnostic-traces")]
                trace_source_orientation(
                    "reject-content",
                    &root.external_id,
                    serde_json::json!({"node": node.external_id, "reason": "external-near"}),
                );
                return Ok(false);
            }
        }

        let Some(flow) = Self::source_interior_flow(self, &local, &mut work, seed, work_limit)?
        else {
            #[cfg(feature = "diagnostic-traces")]
            trace_source_orientation("reject-flow", &root.external_id, serde_json::json!({}));
            return Ok(false);
        };
        #[cfg(feature = "diagnostic-traces")]
        trace_source_orientation(
            "flow",
            &root.external_id,
            serde_json::json!({
                "x_bits": format!("0x{:016x}", flow.x.to_bits()),
                "y_bits": format!("0x{:016x}", flow.y.to_bits()),
                "across_bits": format!("0x{:016x}", flow.across.to_bits()),
                "along_bits": format!("0x{:016x}", flow.along.to_bits()),
            }),
        );
        if flow.across <= flow.along + 1e-9 || flow.x.abs() <= 1e-9 {
            #[cfg(feature = "diagnostic-traces")]
            trace_source_orientation("reject-flow", &root.external_id, serde_json::json!({}));
            return Ok(false);
        }

        let direction = context
            .parent_direction
            .expect("vertical direction checked");
        let turn = if (flow.x < 0.0) != (direction == Direction::Up) {
            -1.0
        } else {
            1.0
        };
        #[cfg(feature = "diagnostic-traces")]
        trace_source_orientation(
            "candidate",
            &root.external_id,
            serde_json::json!({
                "turn": if turn < 0.0 { -1 } else { 1 },
                "parent_direction": trace_orientation_direction(Some(direction)),
            }),
        );
        let Some((before_width, before_height)) =
            Self::source_orientation_root_size(self, root, context.root_padding)
        else {
            #[cfg(feature = "diagnostic-traces")]
            trace_source_orientation_rollback(&root.external_id, "invalid layout candidate");
            return Ok(false);
        };

        let mut candidate = self.clone();
        let centers = nodes
            .iter()
            .copied()
            .filter_map(|node| {
                let position = candidate.position(node)?;
                let size = candidate.active_node_size(node);
                Some((
                    node,
                    Point {
                        x: position.x + size.width / 2.0,
                        y: position.y + size.height / 2.0,
                    },
                ))
            })
            .collect::<Vec<_>>();
        if centers.len() != nodes.len() {
            #[cfg(feature = "diagnostic-traces")]
            trace_source_orientation_rollback(&root.external_id, "invalid layout candidate");
            return Ok(false);
        }

        // Cluster vessels are ordinary placement nodes in Go. Preserve their
        // member ordering while flipping the axis, then let SyncNestedGeometry
        // restore the hidden member boxes after the vessel moves.
        let cluster_indices = nodes
            .iter()
            .filter_map(|&node| candidate.active_cluster_index(node))
            .collect::<BTreeSet<_>>();
        for cluster_index in cluster_indices {
            let arrangement = match candidate.clusters[cluster_index].arrangement {
                ClusterArrangement::Row => ClusterArrangement::Column,
                ClusterArrangement::Column => ClusterArrangement::Row,
            };
            let padding = candidate.prearranged_cluster_padding(cluster_index, arrangement);
            candidate.clusters[cluster_index].arrangement = arrangement;
            candidate.clusters[cluster_index].desired_arrangement = arrangement;
            candidate.clusters[cluster_index].padding = padding;
            candidate.resize_cluster_members(cluster_index);
        }
        candidate.refresh_cluster_vessel_sizes();
        for &node in &nodes {
            if candidate.active_cluster_index(node).is_none()
                && candidate.nodes[node.0 as usize].scoring_is_aggregate_vessel
                && candidate.nodes[node.0 as usize]
                    .scoring_cluster_arrangement
                    .is_some()
            {
                candidate.rotate_projected_cluster_for_orientation(node);
            }
        }

        for (node_id, center) in centers {
            work.step().map_err(|error| {
                layout_error_for_work(error, seed, "ContainerOrientation", work_limit)
            })?;
            let size = candidate.active_node_size(node_id);
            let position = Point {
                x: (-turn * center.y - size.width / 2.0).round(),
                y: (turn * center.x - size.height / 2.0).round(),
            };
            candidate.set_position(node_id, position);
            if let Some(cluster_index) = candidate.active_cluster_index(node_id) {
                // The Go graph moves a distinct cluster vessel node and then
                // syncs member boxes from that position. Rust aliases the
                // vessel to the first stable member, so publish the moved
                // vessel frame before SyncNested reads its cached position.
                candidate
                    .pending_cluster_vessel_positions
                    .insert(cluster_index, position);
            }
            candidate.nodes[node_id.0 as usize].rect.origin = position;
        }
        #[cfg(feature = "diagnostic-traces")]
        trace_orientation_boxes("raw-before-sync", &candidate, &root.external_id, &nodes);
        candidate.sync_clusters_with_descendants();
        candidate.sync_nested_projected_container_children();
        #[cfg(feature = "diagnostic-traces")]
        trace_orientation_boxes("raw-after-sync", &candidate, &root.external_id, &nodes);

        // Go compacts the combined graph, including restored group members.
        // Keep this candidate isolated until every acceptance predicate passes.
        let canonical_candidate = if let Some(session) = owner.as_deref() {
            let (mut full, receiver) = session
                .combined_transition_workspace(&candidate)
                .map_err(|error| crate::LayoutError::PlacementTransaction(error.to_string()))?;
            #[cfg(feature = "diagnostic-traces")]
            if crate::internal::trace_env_enabled("D2RS_TRACE_CANONICAL_COMPACTION") {
                trace_orientation_boxes(
                    "canonical-after-sync",
                    &full,
                    &root.external_id,
                    &full.graph_node_order(),
                );
            }
            let original_cell_size = full.cell_size;
            full.cell_size = 1.0;
            for horizontal in [true, false] {
                full.transition_compact_canonical_axis_checked(
                    receiver,
                    horizontal,
                    1.0,
                    work_context,
                    seed,
                )?;
            }
            full.cell_size = original_cell_size;
            let mapping = CanonicalScoreMapping::new(
                full.topology_arena_id().unwrap(),
                (0..full.nodes.len()).map(|i| NodeId(i as u32)).collect(),
                full.owner_graph_view(receiver).unwrap().edges.read(),
            );
            OwnerScoreSession::new(&mut full, receiver, mapping, None)
                .map_err(|error| crate::LayoutError::PlacementTransaction(error.to_string()))?
                .sync_current_nested_geometry()
                .map_err(|error| crate::LayoutError::PlacementTransaction(error.to_string()))?;
            #[cfg(feature = "diagnostic-traces")]
            if crate::internal::trace_env_enabled("D2RS_TRACE_CANONICAL_COMPACTION") {
                trace_orientation_boxes(
                    "canonical-after-compaction",
                    &full,
                    &root.external_id,
                    &full.graph_node_order(),
                );
            }
            session
                .project_combined_geometry(&full, &mut candidate)
                .map_err(|error| crate::LayoutError::PlacementTransaction(error.to_string()))?;
            Some((full, receiver))
        } else {
            let original_cell_size = candidate.cell_size;
            candidate.cell_size = 1.0;
            candidate
                .transition_compact_axis_checked(true, 1.0, work_context)
                .map_err(|error| {
                    layout_error_for_work(
                        error,
                        seed,
                        "CompactionVisibility",
                        limits::MAX_ENGINE_WORK_UNITS,
                    )
                })?;
            candidate.sync_clusters_with_descendants();
            candidate
                .transition_compact_axis_checked(false, 1.0, work_context)
                .map_err(|error| {
                    layout_error_for_work(
                        error,
                        seed,
                        "CompactionVisibility",
                        limits::MAX_ENGINE_WORK_UNITS,
                    )
                })?;
            candidate.cell_size = original_cell_size;
            candidate.sync_clusters_with_descendants();
            candidate.sync_nested_projected_container_children();
            None
        };
        #[cfg(feature = "diagnostic-traces")]
        trace_orientation_boxes(
            "raw-after-compaction",
            &candidate,
            &root.external_id,
            &nodes,
        );

        let geometry_candidate = canonical_candidate
            .as_ref()
            .map_or(&candidate, |(full, _)| full);
        if !geometry_candidate.is_within_max_size() {
            #[cfg(feature = "diagnostic-traces")]
            trace_source_orientation_rollback(&root.external_id, "invalid layout candidate");
            return Ok(false);
        }
        let Some((candidate_width, candidate_height)) =
            Self::source_orientation_root_size(geometry_candidate, root, context.root_padding)
        else {
            #[cfg(feature = "diagnostic-traces")]
            trace_source_orientation_rollback(&root.external_id, "invalid layout candidate");
            return Ok(false);
        };
        if candidate_width * candidate_height > 2.0 * before_width * before_height {
            #[cfg(feature = "diagnostic-traces")]
            trace_source_orientation_rollback(
                &root.external_id,
                "layout candidate did not improve",
            );
            return Ok(false);
        }
        let Some(after) =
            Self::source_interior_flow(&candidate, &local, &mut work, seed, work_limit)?
        else {
            #[cfg(feature = "diagnostic-traces")]
            trace_source_orientation_rollback(&root.external_id, "invalid layout candidate");
            return Ok(false);
        };
        let sign = if direction == Direction::Up {
            -1.0
        } else {
            1.0
        };
        if sign * after.y <= sign * flow.y + 1e-9 {
            #[cfg(feature = "diagnostic-traces")]
            trace_source_orientation_rollback(
                &root.external_id,
                "layout candidate did not improve",
            );
            return Ok(false);
        }

        work.finish().map_err(|error| {
            layout_error_for_work(error, seed, "ContainerOrientation", work_limit)
        })?;
        crate::internal::check_cancellation()?;

        // The source transaction leaves AffectContainers unset: SyncNested
        // moves shared descendants, but ordinary containers are not refit
        // inside this candidate transaction.
        if canonical_candidate.is_none() {
            candidate.sync_clusters_with_descendants();
            candidate.sync_nested_projected_container_children();
        }
        crate::internal::check_cancellation()?;
        if !candidate.ordinary_gap_trial_is_valid_ignoring_container_escape(self) {
            #[cfg(feature = "diagnostic-traces")]
            trace_source_orientation_rollback(&root.external_id, "invalid layout candidate");
            return Ok(false);
        }
        crate::internal::check_cancellation()?;
        #[cfg(feature = "diagnostic-traces")]
        trace_source_orientation("commit", &root.external_id, serde_json::json!({}));
        if let Some((full, receiver)) = canonical_candidate {
            owner
                .expect("canonical candidate has an owner")
                .publish_combined_orientation(&full, receiver)
                .map_err(|error| crate::LayoutError::PlacementTransaction(error.to_string()))?;
        }
        *self = candidate;
        Ok(true)
    }

    fn source_interior_flow(
        graph: &ArenaGraph,
        local: &BTreeSet<NodeId>,
        work: &mut limits::WorkGuard<'_>,
        seed: i64,
        work_limit: i64,
    ) -> Result<Option<InteriorFlow>, crate::LayoutError> {
        let mut flow = InteriorFlow::default();
        let mut seen = BTreeSet::new();
        for edge_id in &graph.edge_order.read() {
            work.step().map_err(|error| {
                layout_error_for_work(error, seed, "ContainerOrientation", work_limit)
            })?;
            let Some(edge) = graph.edge_records.get(edge_id.0 as usize) else {
                return Ok(None);
            };
            if edge.source_arrow == edge.target_arrow {
                continue;
            }
            let (from, to) = if edge.source_arrow {
                (edge.to, edge.from)
            } else {
                (edge.from, edge.to)
            };
            if from == to || !local.contains(&from) || !local.contains(&to) {
                continue;
            }
            if !seen.insert((from, to)) {
                continue;
            }
            let Some(from_position) = graph.position(from) else {
                return Ok(None);
            };
            let Some(to_position) = graph.position(to) else {
                return Ok(None);
            };
            let from_size = graph.active_node_size(from);
            let to_size = graph.active_node_size(to);
            let dx =
                (to_position.x + to_size.width / 2.0) - (from_position.x + from_size.width / 2.0);
            let dy =
                (to_position.y + to_size.height / 2.0) - (from_position.y + from_size.height / 2.0);
            let length = dx.abs() + dy.abs();
            if length == 0.0 {
                continue;
            }
            flow.x += dx / length;
            flow.across += dx.abs() / length;
            flow.y += dy / length;
            flow.along += dy.abs() / length;
        }
        work.finish().map_err(|error| {
            layout_error_for_work(error, seed, "ContainerOrientation", work_limit)
        })?;
        Ok(Some(flow))
    }

    fn source_orientation_root_size(
        graph: &ArenaGraph,
        root: &ArenaNode,
        padding: Insets,
    ) -> Option<(f64, f64)> {
        let active_nodes = graph
            .graph_node_order()
            .into_iter()
            .map(|node| {
                let mut value = graph.nodes[node.0 as usize].clone();
                value.position = graph.position(node);
                value
            })
            .collect::<Vec<_>>();
        let identities = graph.graph_node_order();
        let projected = identities
            .into_iter()
            .zip(active_nodes.iter())
            .collect::<Vec<_>>();
        let (top_left, bottom_right) = graph.fixed_projected_node_corners(&projected);
        let (top_left, bottom_right) = (top_left?, bottom_right?);
        let fitted = Self::shape_dimensions_to_fit_node(
            root,
            Size {
                width: bottom_right.x - top_left.x,
                height: bottom_right.y - top_left.y,
            },
            padding,
        );
        let width = fitted.width.max(root.desired_width.unwrap_or(0.0));
        let height = fitted.height.max(root.desired_height.unwrap_or(0.0));
        (width.is_finite() && height.is_finite()).then_some((width, height))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Edge, EdgeArrows, Node};
    use std::cell::Cell;

    struct CancelAtWorkPoll {
        done_polls: Cell<usize>,
        cancelled: Cell<bool>,
    }

    impl limits::Context for CancelAtWorkPoll {
        fn done(&self) -> Option<bool> {
            let polls = self.done_polls.get();
            self.done_polls.set(polls + 1);
            if polls >= 2 {
                self.cancelled.set(true);
            }
            Some(self.cancelled.get())
        }

        fn error(&self) -> Option<limits::ContextError> {
            self.cancelled
                .get()
                .then_some(limits::ContextError::Cancelled)
        }
    }

    fn horizontal_cluster_source_graph(
        direction: Direction,
    ) -> (ArenaGraph, SourceOrientationContext, usize, [NodeId; 2]) {
        let mut input = Graph::default();
        let first = input.add_node(Node::new(
            "first",
            Size {
                width: 100.0,
                height: 60.0,
            },
        ));
        let second = input.add_node(Node::new(
            "second",
            Size {
                width: 100.0,
                height: 60.0,
            },
        ));
        let member_a = input.add_node(Node::new(
            "member-a",
            Size {
                width: 100.0,
                height: 40.0,
            },
        ));
        let member_b = input.add_node(Node::new(
            "member-b",
            Size {
                width: 100.0,
                height: 40.0,
            },
        ));
        // Cluster abduction reconnects both member edges to the active vessel.
        let first_edge = input.add_edge(Edge {
            source: first,
            target: member_a,
        });
        input.set_edge_arrows(
            first_edge,
            EdgeArrows {
                source: false,
                target: true,
            },
        );
        let second_edge = input.add_edge(Edge {
            source: second,
            target: member_a,
        });
        input.set_edge_arrows(
            second_edge,
            EdgeArrows {
                source: false,
                target: true,
            },
        );
        let mut graph = ArenaGraph::from_input(&input);
        graph.set_position(first, Point { x: 0.0, y: 0.0 });
        graph.set_position(second, Point { x: 0.0, y: 500.0 });
        graph.set_position(member_a, Point { x: 1000.0, y: 0.0 });
        graph.set_position(
            member_b,
            Point {
                x: 1000.0,
                y: 100.0,
            },
        );
        graph.clusters.push(ClusterState {
            members: vec![member_a, member_b],
            arrangement: ClusterArrangement::Column,
            desired_arrangement: ClusterArrangement::Column,
            padding: 20.0,
            vessel_tala_id: 50,
            fixed_size: false,
        });
        graph.nodes[member_a.0 as usize].cluster = Some(0);
        graph.nodes[member_b.0 as usize].cluster = Some(0);
        graph
            .pending_cluster_vessel_positions
            .insert(0, Point { x: 1000.0, y: 0.0 });
        // This unit fixture mirrors the direct upstream orientation test,
        // whose graph has passed AddClusters but not tree preprocessing.
        graph.tree_sentinels.clear();
        graph.tree_routing_nodes.clear();
        graph.preprocessed_tree_children.clear();
        graph.restored_tree_edges.clear();
        graph.published_tree_node_edge_order.clear();
        graph.restored_hierarchy_endpoint_order.clear();
        graph.rebuild_active_aggregate_node_order();
        graph.refresh_cluster_vessel_sizes();

        let mut root_input = Graph::default();
        let root_id = root_input.add_node(Node::new(
            "source",
            Size {
                width: 30_000.0,
                height: 30_000.0,
            },
        ));
        let mut root = ArenaGraph::from_input(&root_input).nodes[root_id.0 as usize].clone();
        root.is_container = true;
        let root_edges: Vec<Option<(i64, i64)>> = [2_i64, 3_i64, 4_i64]
            .into_iter()
            .map(|destination| Some((root.tala_id, destination)))
            .collect();
        let graph_node_count = graph.graph_node_order().len();
        (
            graph,
            SourceOrientationContext {
                root: Some(root),
                graph_node_count,
                parent_direction: Some(direction),
                root_direction: None,
                root_adjacency_edge_count: root_edges.len(),
                root_edges,
                disabled_for_loop: false,
                has_ancestor_obstacles: false,
                root_padding: Insets::uniform(0.0),
            },
            0,
            [member_a, member_b],
        )
    }

    fn horizontal_source_graph(
        direction: Direction,
        reverse_edges: bool,
    ) -> (ArenaGraph, SourceOrientationContext) {
        let mut input = Graph::default();
        let mut interior = Vec::new();
        for name in ["first", "middle", "last"] {
            interior.push(input.add_node(Node::new(
                name,
                Size {
                    width: 100.0,
                    height: 60.0,
                },
            )));
        }
        for pair in interior.windows(2) {
            let edge = input.add_edge(Edge {
                source: pair[0],
                target: pair[1],
            });
            input.set_edge_arrows(
                edge,
                EdgeArrows {
                    source: reverse_edges,
                    target: !reverse_edges,
                },
            );
        }
        let mut graph = ArenaGraph::from_input(&input);
        graph.cell_size = 100.0;
        for (node, x) in interior.iter().copied().zip([0.0, 400.0, 800.0]) {
            let position = Point { x, y: 0.0 };
            graph.nodes[node.0 as usize].position = Some(position);
            graph.nodes[node.0 as usize].rect.origin = position;
        }

        let mut root_input = Graph::default();
        let root_id = root_input.add_node(Node::new(
            "source",
            Size {
                width: 30_000.0,
                height: 30_000.0,
            },
        ));
        let mut root = ArenaGraph::from_input(&root_input).nodes[root_id.0 as usize].clone();
        root.is_container = true;
        let root_edges: Vec<Option<(i64, i64)>> = [2_i64, 3_i64, 2_i64]
            .into_iter()
            .map(|destination| Some((root.tala_id, destination)))
            .collect();
        let graph_node_count = graph.graph_node_order().len();
        (
            graph,
            SourceOrientationContext {
                root: Some(root),
                graph_node_count,
                parent_direction: Some(direction),
                root_direction: None,
                root_adjacency_edge_count: root_edges.len(),
                root_edges,
                disabled_for_loop: false,
                has_ancestor_obstacles: false,
                root_padding: Insets::uniform(0.0),
            },
        )
    }

    #[test]
    fn turns_a_small_horizontal_interior_to_match_surrounding_source_flow() {
        for (direction, reverse_edges, expected) in [
            (
                Direction::Down,
                false,
                [(-80.0, 20.0), (-80.0, 420.0), (-80.0, 820.0)],
            ),
            (
                Direction::Down,
                true,
                [(-20.0, -80.0), (-20.0, -480.0), (-20.0, -880.0)],
            ),
            (
                Direction::Up,
                false,
                [(-20.0, -80.0), (-20.0, -480.0), (-20.0, -880.0)],
            ),
            (
                Direction::Up,
                true,
                [(-80.0, 20.0), (-80.0, 420.0), (-80.0, 820.0)],
            ),
        ] {
            let (mut graph, context) = horizontal_source_graph(direction, reverse_edges);
            assert!(graph.orient_source_interior(&context, 1).unwrap());
            for (node, (x, y)) in graph.nodes.iter().zip(expected) {
                assert_eq!(node.position, Some(Point { x, y }));
            }
            assert_eq!(graph.cell_size, 100.0);
        }
    }

    #[test]
    fn preserves_the_original_geometry_when_the_source_pass_is_disabled() {
        let (mut graph, mut context) = horizontal_source_graph(Direction::Down, false);
        context.disabled_for_loop = true;
        let before = graph
            .nodes
            .iter()
            .map(|node| node.position)
            .collect::<Vec<_>>();
        assert!(!graph.orient_source_interior(&context, 1).unwrap());
        assert_eq!(
            graph
                .nodes
                .iter()
                .map(|node| node.position)
                .collect::<Vec<_>>(),
            before
        );
    }

    #[test]
    fn source_pass_admission_uses_the_combined_graph_node_count() {
        let (mut graph, mut context) = horizontal_source_graph(Direction::Down, false);
        context.graph_node_count = 17;
        let before = graph
            .nodes
            .iter()
            .map(|node| node.position)
            .collect::<Vec<_>>();

        assert!(!graph.orient_source_interior(&context, 1).unwrap());
        assert_eq!(
            graph
                .nodes
                .iter()
                .map(|node| node.position)
                .collect::<Vec<_>>(),
            before
        );
    }

    #[test]
    fn flips_active_cluster_members_with_the_surrounding_source_flow() {
        let (mut graph, context, cluster_index, members) =
            horizontal_cluster_source_graph(Direction::Down);
        assert_eq!(graph.graph_node_order().len(), 3);
        assert_eq!(graph.position(NodeId(0)), Some(Point { x: 0.0, y: 0.0 }));
        assert_eq!(graph.position(NodeId(1)), Some(Point { x: 0.0, y: 500.0 }));
        assert_eq!(graph.position(NodeId(2)), Some(Point { x: 1000.0, y: 0.0 }));
        assert!(graph.orient_source_interior(&context, 1).unwrap());
        assert_eq!(graph.position(NodeId(0)), Some(Point { x: -80.0, y: 20.0 }));
        assert_eq!(
            graph.position(NodeId(1)),
            Some(Point { x: -580.0, y: 20.0 })
        );
        assert_eq!(
            graph.position(members[0]),
            Some(Point {
                x: -160.0,
                y: 1030.0
            })
        );
        assert_eq!(
            graph.position(members[1]),
            Some(Point {
                x: -40.0,
                y: 1030.0
            })
        );
        let cluster = &graph.clusters[cluster_index];
        assert_eq!(cluster.arrangement, ClusterArrangement::Row);
        assert_eq!(cluster.desired_arrangement, ClusterArrangement::Row);
        assert_eq!(cluster.padding, 20.0);
        assert_eq!(
            graph.pending_cluster_vessel_positions.get(&cluster_index),
            Some(&Point {
                x: -160.0,
                y: 1030.0,
            })
        );
        assert_eq!(
            graph.nodes[members[0].0 as usize].rect.size,
            Size {
                width: 100.0,
                height: 40.0,
            }
        );
        assert_eq!(
            graph.nodes[members[1].0 as usize].rect.size,
            Size {
                width: 100.0,
                height: 40.0,
            }
        );
        assert_eq!(graph.cell_size, 100.0);
    }

    #[test]
    fn orientation_work_limit_is_reported_without_publishing_candidate() {
        let (mut graph, context) = horizontal_source_graph(Direction::Down, false);
        let before = graph
            .nodes
            .iter()
            .map(|node| (node.position, node.rect, node.cluster))
            .collect::<Vec<_>>();
        let work_context = CancelAtWorkPoll {
            done_polls: Cell::new(0),
            cancelled: Cell::new(false),
        };

        let result =
            graph.orient_source_interior_with_context_and_limit(&context, 73, &work_context, 1);

        assert!(matches!(
            result,
            Err(crate::LayoutError::WorkLimit {
                seed: 73,
                location: "ContainerOrientation",
                limit: 1,
            })
        ));
        assert_eq!(
            graph
                .nodes
                .iter()
                .map(|node| (node.position, node.rect, node.cluster))
                .collect::<Vec<_>>(),
            before
        );
    }

    #[test]
    fn orientation_work_guard_propagates_cancellation_from_interior_flow() {
        let (mut graph, context) = horizontal_source_graph(Direction::Down, false);
        graph.edge_order.resize(1_021, EdgeId(0));
        let before = graph
            .nodes
            .iter()
            .map(|node| (node.position, node.rect, node.cluster))
            .collect::<Vec<_>>();
        let work_context = CancelAtWorkPoll {
            done_polls: Cell::new(0),
            cancelled: Cell::new(false),
        };

        let result = graph.orient_source_interior_with_context_and_limit(
            &context,
            73,
            &work_context,
            limits::MAX_ENGINE_WORK_UNITS,
        );

        assert!(matches!(result, Err(crate::LayoutError::Cancelled)));
        assert_eq!(
            graph
                .nodes
                .iter()
                .map(|node| (node.position, node.rect, node.cluster))
                .collect::<Vec<_>>(),
            before
        );
    }

    #[test]
    fn orientation_preflight_disables_the_transform_on_a_loop() {
        let (mut graph, _) = horizontal_source_graph(Direction::Down, false);
        let loop_edge = graph.edge_records[0].clone();
        graph.edge_records.push(ArenaEdge {
            input_id: EdgeId(graph.edge_records.len() as u32),
            tala_id: 99,
            from: loop_edge.from,
            to: loop_edge.from,
            ..loop_edge
        });
        graph
            .edge_order
            .push(EdgeId((graph.edge_records.len() - 1) as u32));

        assert!(graph.source_orientation_disabled_for_loop(1).unwrap());
    }

    #[test]
    fn orientation_preflight_reports_its_work_limit() {
        let (graph, _) = horizontal_source_graph(Direction::Down, false);
        let work_context = CancelAtWorkPoll {
            done_polls: Cell::new(0),
            cancelled: Cell::new(false),
        };

        let result =
            graph.source_orientation_disabled_for_loop_with_context_and_limit(73, &work_context, 1);

        assert!(matches!(
            result,
            Err(crate::LayoutError::WorkLimit {
                seed: 73,
                location: "ContainerOrientationPreflight",
                limit: 1,
            })
        ));
    }

    #[test]
    fn orientation_preflight_propagates_cancellation_while_scanning_edges() {
        let (graph, _) = horizontal_source_graph(Direction::Down, false);
        graph.edge_order.resize(1_021, EdgeId(0));
        let work_context = CancelAtWorkPoll {
            done_polls: Cell::new(0),
            cancelled: Cell::new(false),
        };

        let result = graph.source_orientation_disabled_for_loop_with_context_and_limit(
            73,
            &work_context,
            limits::MAX_ENGINE_WORK_UNITS,
        );

        assert!(matches!(result, Err(crate::LayoutError::Cancelled)));
    }
}
