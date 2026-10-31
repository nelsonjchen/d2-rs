// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Dimension-independent placement on the graph's cell lattice.
//!
//! This stage chooses relative topology before rendered node sizes participate.
//! Candidate order and floating-point comparison boundaries intentionally match
//! the reference implementation because ties affect every later stage.

use super::go_rng::GoRng;
use super::limits::{self, Context, OptimizationError, OptimizationWorkGuard};
use super::occupied::OccupiedPointMap;
use super::owner_sizeless_constructor::{OwnerOptimizerRng, OwnerSizelessOptimizer};
use super::scoring::OwnerSizelessEdgeLengthScorer;
use super::scoring_context::ScoreContext;
use super::{ArenaGraph, NodeId, OwnerScoreSession, Point};
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};

// Open comparison tolerance used by the reference geometry package.
const PRECISION: f64 = 0.0001;

pub(super) fn precision_compare(a: f64, b: f64) -> std::cmp::Ordering {
    if (a - b).abs() < PRECISION {
        std::cmp::Ordering::Equal
    } else {
        if a < b {
            std::cmp::Ordering::Less
        } else {
            std::cmp::Ordering::Greater
        }
    }
}

fn optimizer_scoring_error(error: limits::Error) -> OptimizationError {
    match error {
        limits::Error::Context { location, cause } => {
            OptimizationError::Context { location, cause }
        }
        error => OptimizationError::Invalid(error.to_string()),
    }
}

pub(super) fn move_owner_node_to_best<P>(
    workspace: &mut ArenaGraph,
    node: NodeId,
    points: impl Iterator<Item = P>,
    read_point: impl Fn(P) -> Option<Point>,
    session: &mut OwnerScoreSession<'_>,
    context: &dyn Context,
    guard: &mut OptimizationWorkGuard<'_>,
) -> Result<bool, OptimizationError> {
    owner_candidate_start(workspace, node, session, guard)?;
    let result = catch_unwind(AssertUnwindSafe(|| {
        search_owner_node_to_best(workspace, node, points, read_point, session, context, guard)
    }));
    refresh_owner_candidate_positions(
        workspace,
        session.owner(),
        session.current_mapping().nodes(),
    );
    match result {
        Ok(result) => result,
        Err(payload) => resume_unwind(payload),
    }
}

fn owner_candidate_start(
    workspace: &ArenaGraph,
    node: NodeId,
    session: &OwnerScoreSession<'_>,
    guard: &OptimizationWorkGuard<'_>,
) -> Result<(NodeId, Point), OptimizationError> {
    let mapping = session.current_mapping().nodes();
    if mapping.len() != workspace.nodes.len() {
        return Err(OptimizationError::Invalid(
            "sizeless workspace node mapping length differs".into(),
        ));
    }
    for &canonical in mapping {
        if session.owner().nodes.get(canonical.0 as usize).is_none() {
            return Err(OptimizationError::Invalid(format!(
                "sizeless workspace mapping contains unknown node {canonical:?}"
            )));
        }
    }
    let canonical = session
        .canonical_node(node)
        .map_err(|error| OptimizationError::Invalid(error.to_string()))?;
    let current = session.owner().position(canonical).ok_or_else(|| {
        OptimizationError::Invalid(format!(
            "TALA {} cannot move an unpositioned node",
            guard.location()
        ))
    })?;

    Ok((canonical, current))
}

/// The production sizeless pass keeps geometry exclusively on the owner until
/// its publication boundary. Scoring and swap selection borrow this workspace
/// for identity/topology only (sizeless_optimizer.go:403–544).
pub(super) fn search_owner_node_to_best<P>(
    workspace: &ArenaGraph,
    node: NodeId,
    points: impl Iterator<Item = P>,
    read_point: impl Fn(P) -> Option<Point>,
    session: &mut OwnerScoreSession<'_>,
    context: &dyn Context,
    guard: &mut OptimizationWorkGuard<'_>,
) -> Result<bool, OptimizationError> {
    let (canonical, current) = owner_candidate_start(workspace, node, session, guard)?;

    // Candidate Drop restores the owner before either the standalone caller
    // or the enclosing optimizer publishes geometry, including during unwind.
    {
        let mut scorer = CloseOwnerSizelessScorer(OwnerSizelessEdgeLengthScorer::new(node, true));
        // Between scores this sweep only moves the candidate's owner boxes,
        // and its polls go to these two contexts.
        if context.stable_sized_owner() && guard.context().stable_sized_owner() {
            scorer.0.assume_stable_topology();
        }
        let mut best_distance = f64::INFINITY;
        let mut best = current;
        let mut movement = session.optimizer_candidate_movement(node, guard)?;
        for point in points {
            #[cfg(feature = "diagnostic-traces")]
            guard.step_with_owner(movement.session().owner_mut())?;
            #[cfg(not(feature = "diagnostic-traces"))]
            guard.step()?;
            let point = read_point(point).ok_or_else(|| {
                OptimizationError::Invalid(
                    "TALA sizelessOptimizer.optimize found a nil placement point".into(),
                )
            })?;
            movement.move_abs(point, guard)?;
            // The sizeless scorer reads geometry from the live owner; the
            // workspace supplies identity and topology only. The caller
            // publishes positions at the standalone or whole-pass boundary.

            // optimization_guard.go530–543 uses Node.Edges and the node's
            // actual Graph.Nodes, including the nil-abduction +1 term.
            let owner = movement.session().owner();
            let receiver = owner.topology_node_owner(canonical).ok_or_else(|| {
                OptimizationError::Invalid(format!(
                    "TALA {} scoring requires a node with a graph",
                    guard.location()
                ))
            })?;
            let view = owner.owner_graph_view(receiver).ok_or_else(|| {
                OptimizationError::Invalid(format!(
                    "TALA {} scoring requires a node with a graph",
                    guard.location()
                ))
            })?;
            let edges = owner.owner_node_incident_count(canonical) as u64 + 1;
            let nodes = view.nodes.len() as u64 + 1;
            #[cfg(feature = "diagnostic-traces")]
            guard.add_product_with_owner(edges, nodes, movement.session().owner_mut())?;
            #[cfg(not(feature = "diagnostic-traces"))]
            guard.add_product(edges, nodes)?;
            let distance = movement
                .session()
                .score_sizeless_owner(&mut scorer.0, workspace, ScoreContext::new(context))
                .map_err(optimizer_scoring_error)?;
            #[cfg(feature = "diagnostic-traces")]
            if crate::internal::trace_env_enabled("WEFTAN_TRACE_SIZELESS_SCORE") {
                eprintln!(
                    "{}",
                    serde_json::json!({
                        "event": "sizeless-score",
                        "node_id": crate::internal::pipeline::trace_node_external_id(movement.session().owner(), canonical),
                        "x_bits": format!("{:016x}", point.x.to_bits()),
                        "y_bits": format!("{:016x}", point.y.to_bits()),
                        "score_bits": format!("{:016x}", distance.to_bits()),
                        "best_bits": format!("{:016x}", best_distance.to_bits()),
                    })
                );
            }
            match precision_compare(distance, best_distance) {
                std::cmp::Ordering::Less => {
                    best_distance = distance;
                    best = point;
                }
                std::cmp::Ordering::Equal if point == current => {
                    best_distance = distance;
                    best = point;
                }
                _ => {}
            }
        }
        #[cfg(feature = "diagnostic-traces")]
        let cause = context.error_with_owner(movement.session().owner_mut());
        #[cfg(not(feature = "diagnostic-traces"))]
        let cause = context.error();
        if let Some(cause) = cause {
            return Err(OptimizationError::Context {
                location: "EdgeLength".into(),
                cause,
            });
        }
        if best_distance == f64::INFINITY {
            return Err(OptimizationError::Invalid(
                "sizelessOptimizer.moveNodeToBest: could not find any placement".into(),
            ));
        }
        movement.move_abs(best, guard)?;
        let changed = movement.session().owner().position(canonical) != Some(current);
        movement.commit();
        Ok(changed)
    }
}

/// The mapping is validated before candidate movement and cannot change while
/// that movement owns the session. Only positions are copied: a dense retained
/// child or box must never hydrate the actual source pointer's geometry.
fn refresh_owner_candidate_positions(
    workspace: &mut ArenaGraph,
    owner: &ArenaGraph,
    mapping: &[NodeId],
) {
    for (local, canonical) in mapping.iter().copied().enumerate() {
        let actual = &owner.nodes[canonical.0 as usize];
        let position = owner.position(canonical);
        workspace.nodes[local].position = position;
        workspace.nodes[local].rect.origin = position.unwrap_or(actual.rect.origin);
    }
    // The following sized pass consumes these projected snapshots. Go keeps
    // the same child pointers throughout both passes, so refresh them from
    // the owner after trial movement, never from a stale sibling projection.
    let refresh = |node: &mut super::ArenaNode| {
        let Some(canonical) = node.owner_geometry_source else {
            return;
        };
        let actual = &owner.nodes[canonical.0 as usize];
        node.position = owner.position(canonical);
        node.rect.origin = node.position.unwrap_or(actual.rect.origin);
        node.rect.size = actual.rect.size;
        if let Some(anchor) = node.owner_geometry_anchor {
            node.transaction_anchor_offset =
                node.position
                    .zip(owner.position(anchor))
                    .map(|(point, anchor)| Point {
                        x: point.x - anchor.x,
                        y: point.y - anchor.y,
                    });
        }
    };
    for node in &mut workspace.transaction_external_containers {
        refresh(node);
    }
    for children in workspace
        .transaction_external_container_children
        .values_mut()
        .chain(
            workspace
                .transaction_external_aggregate_children
                .values_mut(),
        )
    {
        for node in children {
            refresh(node);
        }
    }
    for projected in &mut workspace.projected_transaction_nodes {
        refresh(&mut projected.node);
    }
}

struct CloseOwnerSizelessScorer(OwnerSizelessEdgeLengthScorer);

impl Drop for CloseOwnerSizelessScorer {
    fn drop(&mut self) {
        self.0.close();
    }
}

pub(super) struct SizelessOptimizer<'a> {
    graph: &'a mut ArenaGraph,
    nodes: Vec<NodeId>,
    components: Vec<Vec<NodeId>>,
    occupied: OccupiedPointMap,
    rng: OwnerOptimizerRng,
    owner_optimizer: Option<OwnerSizelessOptimizer>,
    materialized_owner_points: Vec<NodeId>,
    materialized_owner_incidents: Vec<NodeId>,
}

fn is_nested_trace_node(tala_id: i64) -> bool {
    matches!(
        tala_id,
        1_051_177_149 | 2_631_351_342 | 3_642_540_283 | 104_801_518
    )
}

fn trace_sizeless_graph(graph: &ArenaGraph) -> bool {
    crate::internal::trace_env_enabled("WEFTAN_TRACE_SIZELESS_STEPS")
        && (graph.nodes.len() == 4
            && !crate::internal::trace_env_enabled("WEFTAN_TRACE_SIZELESS_ROOT")
            || (crate::internal::trace_env_enabled("WEFTAN_TRACE_SIZELESS_ROOT")
                && graph.nodes.len() == 10)
            || graph
                .nodes
                .iter()
                .any(|node| is_nested_trace_node(node.tala_id)))
}

fn rng_scope_node_id(graph: &ArenaGraph, node: NodeId) -> String {
    super::pipeline::trace_node_external_id(graph, node)
}

#[cfg(feature = "diagnostic-traces")]
fn trace_container_external_id(
    graph: &ArenaGraph,
    parent_tala_id: i64,
    children: &[super::ArenaNode],
) -> String {
    if let Some(parent) = graph
        .nodes
        .iter()
        .find(|node| node.tala_id == parent_tala_id)
    {
        return parent.external_id.clone();
    }
    let parent_paths = children
        .iter()
        .map(|child| child.external_id.rsplit_once('.').map(|(parent, _)| parent))
        .collect::<Option<Vec<_>>>();
    if let Some(parent_paths) = parent_paths
        && let Some(first) = parent_paths.first()
        && parent_paths.iter().all(|parent| parent == first)
    {
        return (*first).to_owned();
    }
    format!("tala:{parent_tala_id}")
}

#[cfg(feature = "diagnostic-traces")]
fn trace_retained_container_context(graph: &ArenaGraph) -> Vec<(String, Vec<String>)> {
    graph
        .trace_container_children
        .iter()
        .map(|(parent_tala_id, children)| {
            let parent = trace_container_external_id(graph, *parent_tala_id, children);
            (
                parent,
                children
                    .iter()
                    .map(|child| child.external_id.clone())
                    .collect(),
            )
        })
        .collect()
}

impl<'a> SizelessOptimizer<'a> {
    pub(super) fn graph(&self) -> &ArenaGraph {
        self.graph
    }

    pub(super) fn new(graph: &'a mut ArenaGraph, seed: i64) -> Self {
        Self::with_trace_root(graph, seed, None)
    }

    pub(super) fn with_trace_root(
        graph: &'a mut ArenaGraph,
        seed: i64,
        trace_root: Option<&str>,
    ) -> Self {
        #[cfg(feature = "diagnostic-traces")]
        if crate::internal::trace_env_enabled("WEFTAN_TRACE_RNG_SCOPE_CONTEXT") {
            let nodes = graph
                .node_order
                .iter()
                .map(|node| {
                    let state = &graph.nodes[node.0 as usize];
                    (
                        state.external_id.clone(),
                        state.hierarchy.is_some(),
                        state.is_container,
                        graph.active_edge_ids(*node).len(),
                        state.nears.len(),
                        graph.preprocessed_tree_children.contains_key(&Some(*node)),
                        graph
                            .preprocessed_tree_children
                            .values()
                            .any(|children| children.contains(node)),
                        graph.tree_sentinels.contains(node),
                        graph.tree_routing_nodes.contains_key(node),
                    )
                })
                .collect::<Vec<_>>();
            eprintln!(
                "RNG_SCOPE_CONTEXT root={trace_root:?} nodes={nodes:?} retained={:?}",
                trace_retained_container_context(graph)
            );
        }
        // TALA only opens a node-placement RNG for subgraphs not owned by an
        // assigned hierarchy. Tree sentinels alone are insufficient evidence:
        // some remain ordinary placement roots while others are handled only
        // by PlaceTrees, so preserve the hierarchy guard without suppressing
        // scopes based on tree membership.
        let opens_node_placement_scope = graph
            .node_order
            .first()
            .is_some_and(|node| graph.nodes[node.0 as usize].hierarchy.is_none());
        let rng = if opens_node_placement_scope {
            GoRng::with_stream(seed, "node-placement")
        } else {
            GoRng::new_untraced(seed)
        };
        Self::trace_rng_scope(graph, trace_root);
        Self::with_rng(graph, rng)
    }

    /// Mirrors `placeNodes` opening its RNG before `initializeNodes`. Nested
    /// scopes create the stream at that boundary and hand it to the optimizer
    /// only after initialization has finished.
    pub(super) fn trace_rng_scope(graph: &ArenaGraph, trace_root: Option<&str>) {
        let opens_node_placement_scope = graph
            .node_order
            .first()
            .is_some_and(|node| graph.nodes[node.0 as usize].hierarchy.is_none());
        #[cfg(feature = "diagnostic-traces")]
        if opens_node_placement_scope
            && crate::internal::trace_env_enabled("WEFTAN_TRACE_RNG_JSONL")
        {
            let mut event = serde_json::json!({
                "event": "rng-scope",
                "stream": "node-placement",
                "root": trace_root,
                "nodes": graph.node_order.iter().map(|node| rng_scope_node_id(graph, *node)).collect::<Vec<_>>(),
            });
            if crate::internal::trace_env_enabled("WEFTAN_TRACE_SCOPE_CONTEXT") {
                let root_geometry = trace_root
                    .and_then(|root_id| graph.nodes.iter().find(|node| node.external_id == root_id))
                    .and_then(|node| {
                        node.position.map(|position| {
                            serde_json::json!({
                                "x": position.x,
                                "y": position.y,
                                "width": node.rect.size.width,
                                "height": node.rect.size.height,
                            })
                        })
                    })
                    .unwrap_or(serde_json::Value::Null);
                event["root_geometry"] = root_geometry;
                let containers = graph
                    .trace_container_children
                    .iter()
                    .filter_map(|(parent_tala_id, children)| {
                        // Go omits its nil-key source-root entry. The retained
                        // diagnostic map mirrors the shared non-nil
                        // Graph.Containers entries visible to SplitSubgraphs,
                        // whereas `graph.containers` is only this Rust
                        // projection's induced topology.
                        if *parent_tala_id == 0 {
                            return None;
                        }
                        let id = trace_container_external_id(graph, *parent_tala_id, children);
                        let node = graph
                            .nodes
                            .iter()
                            .chain(&graph.transaction_external_containers)
                            .chain(&graph.transaction_validation_external_containers)
                            .find(|node| node.tala_id == *parent_tala_id);
                        let mut record = serde_json::json!({
                            "id": id,
                            "positioned": node.is_some_and(|node| node.position.is_some()),
                            "children": children.len(),
                        });
                        if let Some(node) = node
                            && let Some(position) = node.position
                        {
                            record["x"] = serde_json::json!(position.x);
                            record["y"] = serde_json::json!(position.y);
                            record["width"] = serde_json::json!(node.rect.size.width);
                            record["height"] = serde_json::json!(node.rect.size.height);
                        }
                        Some(record)
                    })
                    .collect::<Vec<_>>();
                event["containers"] = serde_json::json!(containers);
            }
            eprintln!("{event}");
        }
        #[cfg(not(feature = "diagnostic-traces"))]
        let _ = (opens_node_placement_scope, trace_root);
    }

    /// Consumes the stream opened at the source placement boundary. It has
    /// already been traced there, so construction here must not emit a second
    /// RNG initialization/scope event.
    pub(super) fn with_trace_rng(
        graph: &'a mut ArenaGraph,
        rng: GoRng,
        trace_root: Option<&str>,
    ) -> Self {
        #[cfg(not(feature = "diagnostic-traces"))]
        let _ = trace_root;
        #[cfg(feature = "diagnostic-traces")]
        if crate::internal::trace_env_enabled("WEFTAN_TRACE_RNG_SCOPE_CONTEXT") {
            let nodes = graph
                .node_order
                .iter()
                .map(|node| {
                    let state = &graph.nodes[node.0 as usize];
                    (
                        state.external_id.clone(),
                        state.hierarchy.is_some(),
                        state.is_container,
                        graph.active_edge_ids(*node).len(),
                        state.nears.len(),
                        graph.preprocessed_tree_children.contains_key(&Some(*node)),
                        graph
                            .preprocessed_tree_children
                            .values()
                            .any(|children| children.contains(node)),
                        graph.tree_sentinels.contains(node),
                        graph.tree_routing_nodes.contains_key(node),
                    )
                })
                .collect::<Vec<_>>();
            eprintln!(
                "RNG_SCOPE_CONTEXT root={trace_root:?} nodes={nodes:?} retained={:?}",
                trace_retained_container_context(graph)
            );
        }
        Self::with_rng(graph, rng)
    }

    /// Builds the read-only optimizer view used by candidate-cost helpers.
    /// Those helpers inspect occupancy and never draw randomness, so they
    /// must not appear as independent node-placement RNG streams in traces.
    pub(super) fn for_scoring(graph: &'a mut ArenaGraph, seed: i64) -> Self {
        Self::with_rng(graph, GoRng::new_untraced(seed))
    }

    pub(super) fn with_rng(graph: &'a mut ArenaGraph, rng: GoRng) -> Self {
        // The optimizer is created after sequence/cluster membership and the
        // current Graph.Nodes order are fixed for this scope. Publish the
        // dense aggregate lookup tables once here so its many owner/container
        // queries do not repeatedly scan the invalidated order vector.
        graph.refresh_node_order_membership();
        let components = graph.optimizable_components();
        // TALA constructs an optimizer for each SplitSubgraphs result, whose
        // Nodes slice is FIFO connectivity order rather than declaration
        // order. ArenaGraph recovers that order where its graph scope matches.
        let nodes = graph.optimizer_nodes();
        let occupied = Self::occupied_for_graph(graph);
        let optimizer = Self {
            graph,
            nodes,
            components,
            occupied,
            rng: OwnerOptimizerRng::new(rng),
            owner_optimizer: None,
            materialized_owner_points: Vec::new(),
            materialized_owner_incidents: Vec::new(),
        };
        if trace_sizeless_graph(optimizer.graph) {
            eprintln!(
                "SIZELESS_RUST_NEW nodes={:?}",
                optimizer
                    .nodes
                    .iter()
                    .map(|node| {
                        let n = &optimizer.graph.nodes[node.0 as usize];
                        (n.tala_id, n.position)
                    })
                    .collect::<Vec<_>>()
            );
        }
        optimizer
    }

    pub(super) fn into_rng(self) -> GoRng {
        drop(self.owner_optimizer);
        self.rng.into_rng()
    }

    pub(super) fn optimize_with_owner_session(
        &mut self,
        temperature: f64,
        session: &mut OwnerScoreSession<'_>,
        context: &dyn Context,
    ) -> Result<(), OptimizationError> {
        if self.owner_optimizer.is_none() {
            for index in 0..session.owner().nodes.len() {
                let node = NodeId(index as u32);
                if session
                    .owner_mut()
                    .materialize_owner_node_point(node)
                    .map_err(|e| OptimizationError::Invalid(e.to_string()))?
                {
                    self.materialized_owner_points.push(node);
                }
                if session
                    .owner_mut()
                    .materialize_owner_node_incident_header(node)
                    .map_err(|e| OptimizationError::Invalid(e.to_string()))?
                {
                    self.materialized_owner_incidents.push(node);
                }
            }
            let receiver = session.current_view();
            self.owner_optimizer = Some(
                OwnerSizelessOptimizer::new(
                    session.owner_mut(),
                    Some(receiver),
                    Some(self.rng.clone()),
                    Some(context),
                )
                .map_err(|e| OptimizationError::Invalid(e.to_string()))?,
            );
        }
        let result = catch_unwind(AssertUnwindSafe(|| {
            self.owner_optimizer
                .as_ref()
                .unwrap()
                .optimize_with_limit(
                    self.graph,
                    session,
                    context,
                    temperature,
                    limits::MAX_OPTIMIZATION_WORK_UNITS,
                )
                .map_err(|e| OptimizationError::Invalid(e.to_string()))
        }));
        refresh_owner_candidate_positions(
            self.graph,
            session.owner(),
            session.current_mapping().nodes(),
        );
        match result {
            Ok(result) => result,
            Err(payload) => resume_unwind(payload),
        }
    }

    pub(super) fn release_owner_geometry(&mut self, session: &mut OwnerScoreSession<'_>) {
        for node in self.materialized_owner_points.drain(..) {
            session
                .owner_mut()
                .release_materialized_owner_node_point(node);
        }
        for node in self.materialized_owner_incidents.drain(..) {
            session
                .owner_mut()
                .release_materialized_owner_node_incident_header(node);
        }
    }

    pub(super) fn compact_with_owner_session(
        &mut self,
        horizontal: bool,
        factor: f64,
        session: &mut OwnerScoreSession<'_>,
    ) -> Result<(), OptimizationError> {
        // The dense transport can retain master-arena indices in its topology.
        // Validate and snapshot the actual selected SplitSubgraphs owner, while
        // keeping numerical work in its explicit local member mapping.
        let active = self.graph.graph_node_order();
        self.graph.compact_sizeless_with_owner_context(
            horizontal,
            factor,
            &active,
            Some(session),
            &super::compaction_candidates::CompactionContext,
        )?;
        self.occupied = Self::occupied_for_graph(self.graph);
        refresh_owner_candidate_positions(
            self.graph,
            session.owner(),
            session.current_mapping().nodes(),
        );
        if let Some(optimizer) = self.owner_optimizer.as_mut() {
            optimizer
                .reset_occupied(session.owner())
                .map_err(|e| OptimizationError::Invalid(e.to_string()))?;
        }
        Ok(())
    }

    /// sizeless_optimizer.go82–90 keeps the complete Point value from each
    /// current Graph.Nodes member. Fractional coordinates are not a lattice
    /// conversion; signed zeros, infinities and NaNs retain map semantics.
    fn occupied_for_graph(graph: &ArenaGraph) -> OccupiedPointMap {
        let occupied = OccupiedPointMap::new();
        for &node in &graph.node_order {
            if let Some(point) = graph.position(node) {
                occupied.insert(point, Some(node));
            }
        }
        occupied
    }

    fn is_occupied(&self, point: Point) -> bool {
        self.occupied.lookup(point).is_some()
    }

    pub(super) fn compact(
        &mut self,
        horizontal: bool,
        factor: f64,
    ) -> Result<(), OptimizationError> {
        if self.graph.nodes.len() <= 1 {
            return Ok(());
        }
        // Graph.compaction operates on the complete SplitSubgraphs owner.
        // Fixed and otherwise skipped optimizer nodes remain visible anchors;
        // only the compaction move loop decides which nodes are movable.
        let active = self
            .graph
            .nodes
            .iter()
            .map(|node| node.input_id)
            .collect::<Vec<_>>();
        self.graph.compact_sizeless(horizontal, factor, &active)?;
        self.occupied = Self::occupied_for_graph(self.graph);
        Ok(())
    }

    pub(super) fn largest_component_size(&self) -> usize {
        self.components.iter().map(Vec::len).max().unwrap_or(0)
    }

    pub(super) fn next_float_probe(&self) -> f64 {
        self.rng.stream().borrow().clone_without_trace().float64()
    }

    // Direct translation of FindClosestUnoccupiedDistance's diamond walk.
    pub(super) fn closest_unoccupied_distance(&self, median: Point) -> Option<f64> {
        self.closest_unoccupied_distance_for(None, median)
    }

    fn fixed_origin_in_cells(&self, node: Option<NodeId>) -> Option<Point> {
        let node = node?;
        let container = self.graph.nodes[node.0 as usize].container;
        self.graph
            .container_fixed_origin(container)
            .map(|origin| Point {
                x: (origin.x / self.graph.cell_size).round(),
                y: (origin.y / self.graph.cell_size).round(),
            })
    }

    pub(super) fn closest_unoccupied_distance_for(
        &self,
        node: Option<NodeId>,
        median: Point,
    ) -> Option<f64> {
        let fixed_origin = self.fixed_origin_in_cells(node);
        let usable = |point: Point| {
            fixed_origin.is_none_or(|origin| point.x >= origin.x && point.y >= origin.y)
                && !self.is_occupied(point)
        };
        for current in 0..=100_i64 {
            let mut y = 0_i64;
            for x in (-current..=current).rev() {
                if y != 0 {
                    let point = Point {
                        x: median.x + x as f64,
                        y: median.y - y as f64,
                    };
                    if usable(point) {
                        return Some(current as f64);
                    }
                }
                let point = Point {
                    x: median.x + x as f64,
                    y: median.y + y as f64,
                };
                if usable(point) {
                    return Some(current as f64);
                }
                if x > 0 {
                    y += 1;
                } else {
                    y -= 1;
                }
            }
        }
        None
    }

    // Direct translation of getPlacementPoints. TALA evaluates every point
    // within the next Manhattan diamond, not only the perimeter.
    pub(super) fn placement_points(&self, median: Point, minimum: f64) -> Vec<Point> {
        self.placement_points_for(None, median, minimum)
    }

    fn placement_points_for(
        &self,
        node: Option<NodeId>,
        median: Point,
        minimum: f64,
    ) -> Vec<Point> {
        let distance = minimum.round() as i64 + 1;
        let fixed_origin = self.fixed_origin_in_cells(node);
        let usable = |point: Point| {
            fixed_origin.is_none_or(|origin| point.x >= origin.x && point.y >= origin.y)
                && !self.is_occupied(point)
        };
        let mut points = Vec::new();
        for x in (-distance..=distance).rev() {
            for y in 0..=distance - x.abs() {
                if y != 0 {
                    let point = Point {
                        x: median.x + x as f64,
                        y: median.y - y as f64,
                    };
                    if usable(point) {
                        points.push(point);
                    }
                }
                let point = Point {
                    x: median.x + x as f64,
                    y: median.y + y as f64,
                };
                if usable(point) {
                    points.push(point);
                }
            }
        }
        points
    }

    fn median_point(&mut self, node: NodeId, temperature: f64) -> Point {
        let mut median = self.graph.median_to_neighbors(node);
        let fixed_origin = self.fixed_origin_in_cells(Some(node));
        if let Some(origin) = fixed_origin {
            if median.x < origin.x {
                median.x = origin.x + temperature;
            }
            if median.y < origin.y {
                median.y = origin.y + temperature;
            }
        }
        let random_x = self.rng.stream().borrow_mut().float64();
        let random_y = self.rng.stream().borrow_mut().float64();
        let base = median;
        median.x += -temperature + random_x * (2.0 * temperature);
        median.y += -temperature + random_y * (2.0 * temperature);
        if let Some(origin) = fixed_origin {
            median.x = median.x.max(origin.x);
            median.y = median.y.max(origin.y);
        }
        let point = Point {
            x: median.x.round(),
            y: median.y.round(),
        };
        if trace_sizeless_graph(self.graph)
            && (crate::internal::trace_env_enabled("WEFTAN_TRACE_SIZELESS_ROOT")
                || is_nested_trace_node(self.graph.nodes[node.0 as usize].tala_id))
        {
            eprintln!(
                "SIZELESS_RUST_MEDIAN node={} base={:?} temperature={} result={:?} draws={}",
                self.graph.nodes[node.0 as usize].tala_id,
                self.graph.median_to_neighbors(node),
                temperature,
                point,
                self.rng.stream().borrow().draw_count()
            );
            if self.graph.nodes.len() == 10 {
                eprintln!(
                    "SIZELESS_RUST_MEDIAN_DEBUG node={} base={:?} random={:.17},{:.17} temp={:.17}",
                    self.graph.nodes[node.0 as usize].tala_id,
                    base,
                    random_x,
                    random_y,
                    temperature,
                );
            }
        }
        point
    }

    fn move_node_to_best(&mut self, node: NodeId, points: &[Point]) -> bool {
        let Some(current) = self.graph.position(node) else {
            return false;
        };
        let mut best_distance = f64::INFINITY;
        let mut best = current;
        for point in points.iter().copied() {
            self.graph.move_node_abs_with_children(node, point);
            let distance = self.graph.sizeless_edge_length(node, true);
            #[cfg(feature = "diagnostic-traces")]
            if crate::internal::trace_env_enabled("WEFTAN_TRACE_SIZELESS_SCORE") {
                eprintln!(
                    "{}",
                    serde_json::json!({
                        "event": "sizeless-score",
                        "node_id": crate::internal::pipeline::trace_node_external_id(self.graph, node),
                        "x_bits": format!("{:016x}", point.x.to_bits()),
                        "y_bits": format!("{:016x}", point.y.to_bits()),
                        "score_bits": format!("{:016x}", distance.to_bits()),
                        "best_bits": format!("{:016x}", best_distance.to_bits()),
                    })
                );
            }
            if trace_sizeless_graph(self.graph)
                && (crate::internal::trace_env_enabled("WEFTAN_TRACE_SIZELESS_ROOT")
                    || is_nested_trace_node(self.graph.nodes[node.0 as usize].tala_id))
            {
                eprintln!(
                    "SIZELESS_RUST_SCORE node={} candidate={:?} score={:.17}",
                    self.graph.nodes[node.0 as usize].tala_id, point, distance
                );
            }
            match precision_compare(distance, best_distance) {
                std::cmp::Ordering::Less => {
                    best_distance = distance;
                    best = point;
                }
                std::cmp::Ordering::Equal if point == current => {
                    best_distance = distance;
                    best = point;
                }
                _ => {}
            }
        }
        self.graph.move_node_abs_with_children(node, best);
        best != current
    }

    /// sizeless_optimizer.go403–476: an explicit checked candidate sweep.
    /// The actual owner's root.Graph supplies movement and scoring operands;
    /// successful trials remain live until the selected point is committed.
    /// This does not yet replace the enclosing optimizer pass or its journal.
    pub(super) fn move_node_to_best_with_owner_session(
        &mut self,
        node: NodeId,
        points: &[Point],
        session: &mut OwnerScoreSession<'_>,
        context: &dyn Context,
        guard: &mut OptimizationWorkGuard<'_>,
    ) -> Result<bool, OptimizationError> {
        self.move_node_to_best_with_owner_candidates(
            node,
            points.iter().copied().map(Some),
            session,
            context,
            guard,
        )
    }

    /// The source []*geo.Point input can contain nil. Each entry is charged
    /// before the nil check, and shares the nonnil sweep's cleanup lifetime.
    pub(super) fn move_node_to_best_with_owner_nullable_session(
        &mut self,
        node: NodeId,
        points: &[Option<Point>],
        session: &mut OwnerScoreSession<'_>,
        context: &dyn Context,
        guard: &mut OptimizationWorkGuard<'_>,
    ) -> Result<bool, OptimizationError> {
        self.move_node_to_best_with_owner_candidates(
            node,
            points.iter().copied(),
            session,
            context,
            guard,
        )
    }

    fn move_node_to_best_with_owner_candidates(
        &mut self,
        node: NodeId,
        points: impl Iterator<Item = Option<Point>>,
        session: &mut OwnerScoreSession<'_>,
        context: &dyn Context,
        guard: &mut OptimizationWorkGuard<'_>,
    ) -> Result<bool, OptimizationError> {
        move_owner_node_to_best(self.graph, node, points, |p| p, session, context, guard)
    }

    fn swap_candidates(&self, node: NodeId) -> Vec<NodeId> {
        let Some(position) = self.graph.position(node) else {
            return Vec::new();
        };
        [(0.0, -1.0), (0.0, 1.0), (-1.0, 0.0), (1.0, 0.0)]
            .into_iter()
            .filter_map(|(dx, dy)| {
                let candidate = self
                    .occupied
                    .lookup(Point {
                        x: position.x + dx,
                        y: position.y + dy,
                    })
                    .flatten()?;
                self.graph.can_optimize_node(candidate).then_some(candidate)
            })
            .collect()
    }

    // Source550–582 attempts every compass direction even after an error;
    // a later rejected Step overwrites an earlier nil-node diagnostic.
    fn swap_candidates_guarded(
        &self,
        node: NodeId,
        guard: &mut OptimizationWorkGuard<'_>,
    ) -> Result<Vec<NodeId>, OptimizationError> {
        let position = self.graph.position(node);
        let mut adjacent = Vec::with_capacity(4);
        let mut adjacent_error = None;
        for (dx, dy) in [(0.0, -1.0), (0.0, 1.0), (-1.0, 0.0), (1.0, 0.0)] {
            if let Err(error) = guard.step() {
                adjacent_error = Some(error);
                continue;
            }
            // Source dereferences its retained TopLeft only after this Step.
            let position = position.expect("sizeless swap adjacency requires node.TopLeft");
            match self.occupied.lookup(Point {
                x: position.x + dx,
                y: position.y + dy,
            }) {
                Some(None) => {
                    adjacent_error = Some(OptimizationError::Invalid(format!(
                        "TALA {} found a nil optimizer node",
                        guard.location()
                    )));
                }
                Some(Some(candidate)) if self.graph.can_optimize_node(candidate) => {
                    adjacent.push(candidate);
                }
                _ => {}
            }
        }
        match adjacent_error {
            Some(error) => Err(error),
            None => Ok(adjacent),
        }
    }

    // Direct translation of getBestSwapCandidate.
    fn best_swap_candidate(&mut self, node: NodeId) -> Option<NodeId> {
        let mut candidates = self.swap_candidates(node);
        self.rng.stream().borrow_mut().shuffle(&mut candidates);
        self.evaluate_best_swap_candidate(node, candidates)
    }

    fn best_swap_candidate_guarded(
        &mut self,
        node: NodeId,
        guard: &mut OptimizationWorkGuard<'_>,
    ) -> Result<Option<NodeId>, OptimizationError> {
        let mut candidates = self.swap_candidates_guarded(node, guard)?;
        self.rng
            .stream()
            .borrow_mut()
            .shuffle_guarded(&mut candidates, guard)?;
        guard.add(candidates.len() as u64)?;
        Ok(self.evaluate_best_swap_candidate(node, candidates))
    }

    fn evaluate_best_swap_candidate(
        &mut self,
        node: NodeId,
        candidates: Vec<NodeId>,
    ) -> Option<NodeId> {
        let mut minimum_l1 = self.graph.sizeless_edge_length(node, true);
        let trace_swap = trace_sizeless_graph(self.graph)
            && (crate::internal::trace_env_enabled("WEFTAN_TRACE_SIZELESS_ROOT")
                || is_nested_trace_node(self.graph.nodes[node.0 as usize].tala_id));
        if trace_swap {
            eprint!(
                "SIZELESS_RUST_SWAP_START node={} min={:.17} candidates=",
                self.graph.nodes[node.0 as usize].tala_id, minimum_l1
            );
            for candidate in &candidates {
                eprint!(
                    "{}@{:?} ",
                    self.graph.nodes[candidate.0 as usize].tala_id,
                    self.graph.position(*candidate)
                );
            }
            eprintln!();
        }
        let mut best = None;
        for candidate in candidates {
            let current_l2 = self.graph.sizeless_edge_length(candidate, true);
            self.graph.swap_positions(node, candidate);
            let swapped_l1 = self.graph.sizeless_edge_length(node, true);
            let swapped_l2 = if precision_compare(swapped_l1, minimum_l1).is_lt() {
                self.graph.sizeless_edge_length(candidate, true)
            } else {
                0.0
            };
            self.graph.swap_positions(node, candidate);

            if trace_swap {
                eprintln!(
                    "SIZELESS_RUST_SWAP_CAND node={} candidate={} current={:.17} swapped1={:.17} swapped2={:.17}",
                    self.graph.nodes[node.0 as usize].tala_id,
                    self.graph.nodes[candidate.0 as usize].tala_id,
                    current_l2,
                    swapped_l1,
                    swapped_l2
                );
            }

            if precision_compare(swapped_l1, minimum_l1).is_lt()
                && !precision_compare(swapped_l2, current_l2).is_gt()
            {
                if trace_swap {
                    eprintln!(
                        "SIZELESS_RUST_SWAP_ACCEPT node={} candidate={} l1={:.17} l2={:.17}",
                        self.graph.nodes[node.0 as usize].tala_id,
                        self.graph.nodes[candidate.0 as usize].tala_id,
                        swapped_l1,
                        swapped_l2
                    );
                }
                minimum_l1 = swapped_l1;
                best = Some(candidate);
            }
        }
        best
    }

    pub(super) fn optimize(&mut self, temperature: f64) {
        let mut indices: Vec<_> = (0..self.nodes.len()).collect();
        self.rng.stream().borrow_mut().shuffle(&mut indices);
        if trace_sizeless_graph(self.graph) {
            eprintln!(
                "SIZELESS_RUST_SHUFFLE indices={:?} draws={}",
                indices,
                self.rng.stream().borrow().draw_count()
            );
        }
        for index in indices {
            let node = self.nodes[index];
            let Some(position) = self.graph.position(node) else {
                continue;
            };
            self.occupied.delete(position);
            let median = self.median_point(node, temperature);
            let Some(distance) = self.closest_unoccupied_distance_for(Some(node), median) else {
                self.occupied.insert(position, Some(node));
                continue;
            };
            let mut points = self.placement_points_for(Some(node), median, distance);
            if crate::internal::trace_env_enabled("WEFTAN_TRACE_SIZELESS_POINTS") {
                #[derive(serde::Serialize)]
                struct PointBits {
                    x: String,
                    y: String,
                }
                let point_bits = points
                    .iter()
                    .map(|point| PointBits {
                        x: format!("{:016x}", point.x.to_bits()),
                        y: format!("{:016x}", point.y.to_bits()),
                    })
                    .collect::<Vec<_>>();
                let node_state = &self.graph.nodes[node.0 as usize];
                let event = serde_json::json!({
                    "event": "sizeless-points",
                    "node_id": node_state.tala_id.to_string(),
                    "start_x_bits": format!("{:016x}", position.x.to_bits()),
                    "start_y_bits": format!("{:016x}", position.y.to_bits()),
                    "median_x_bits": format!("{:016x}", median.x.to_bits()),
                    "median_y_bits": format!("{:016x}", median.y.to_bits()),
                    "distance_bits": format!("{:016x}", distance.to_bits()),
                    "points": point_bits,
                });
                eprintln!("{event}");
            }
            self.rng.stream().borrow_mut().shuffle(&mut points);
            let moved = self.move_node_to_best(node, &points);
            if !moved && let Some(candidate) = self.best_swap_candidate(node) {
                self.graph.swap_positions(node, candidate);
                if let Some(position) = self.graph.position(candidate) {
                    self.occupied.insert(position, Some(candidate));
                }
            }
            if trace_sizeless_graph(self.graph)
                && (crate::internal::trace_env_enabled("WEFTAN_TRACE_SIZELESS_ROOT")
                    || is_nested_trace_node(self.graph.nodes[node.0 as usize].tala_id))
            {
                eprintln!(
                    "SIZELESS_RUST_POINTS node={} original={:?} median={:?} points={:?}",
                    self.graph.nodes[node.0 as usize].tala_id, position, median, points
                );
                eprintln!(
                    "SIZELESS_RUST_NODE node={} start={:?} median={:?} distance={} points={} moved={} end={:?} draws={}",
                    self.graph.nodes[node.0 as usize].tala_id,
                    position,
                    median,
                    distance,
                    points.len(),
                    moved,
                    self.graph.position(node),
                    self.rng.stream().borrow().draw_count()
                );
            }
            if let Some(position) = self.graph.position(node) {
                self.occupied.insert(position, Some(node));
            }
        }
    }

    /// Run one sizeless pass with the v0.9.0 optimization work accounting.
    ///
    /// The ordinary layout path intentionally keeps its established infallible
    /// API.  Diagnostic callers use this entry point to exercise the same
    /// operation boundaries as Go (including guarded shuffles) and to verify
    /// cancellation/resource-limit behavior without putting a guard or its
    /// error construction in the hot release loop.
    pub(super) fn optimize_with_limit(
        &mut self,
        temperature: f64,
        context: &dyn super::limits::Context,
        limit: u64,
    ) -> Result<bool, OptimizationError> {
        #[cfg(feature = "diagnostic-traces")]
        crate::internal::call_counts::count!(SizelessOptimize);
        let mut guard =
            OptimizationWorkGuard::new(Some(context), "sizelessOptimizer.optimize", limit)?;
        guard.add(self.nodes.len() as u64)?;
        let mut indices: Vec<_> = (0..self.nodes.len()).collect();
        for _ in &indices {
            guard.step()?;
        }
        self.rng
            .stream()
            .borrow_mut()
            .shuffle_guarded(&mut indices, &mut guard)?;

        let mut changed = false;
        for index in indices {
            guard.step()?;
            let node = self.nodes[index];
            let Some(position) = self.graph.position(node) else {
                return Err(OptimizationError::Invalid(
                    "TALA sizelessOptimizer.optimize found an unpositioned node".into(),
                ));
            };
            self.occupied.delete(position);

            // medianPoint consumes exactly two stream draws after its
            // neighbor/fixed-origin scans. Charge those draws before entering
            // the existing implementation so a rejected pass cannot advance
            // farther than the Go guard permits.
            guard.add(2)?;
            let median = self.median_point(node, temperature);
            guard.add(1)?;
            let Some(distance) = self.closest_unoccupied_distance_for(Some(node), median) else {
                self.occupied.insert(position, Some(node));
                continue;
            };
            let mut points = self.placement_points_for(Some(node), median, distance);
            guard.add(points.len() as u64)?;
            self.rng
                .stream()
                .borrow_mut()
                .shuffle_guarded(&mut points, &mut guard)?;

            // Score each candidate once. The unguarded helper is reused to
            // preserve the exact floating-point and tie-breaking sequence.
            let before = self.graph.position(node);
            let moved = self.move_node_to_best(node, &points);
            guard.add(points.len() as u64)?;
            if moved {
                changed |= before != self.graph.position(node);
            } else {
                if let Some(candidate) = self.best_swap_candidate_guarded(node, &mut guard)? {
                    guard.add(2)?;
                    self.graph.swap_positions(node, candidate);
                    if let Some(candidate_position) = self.graph.position(candidate) {
                        self.occupied.insert(candidate_position, Some(candidate));
                    }
                    changed = true;
                }
            }
            if let Some(node_position) = self.graph.position(node) {
                self.occupied.insert(node_position, Some(node));
            }
        }
        guard.finish()?;
        Ok(changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::internal::{CanonicalScoreMapping, OwnerGraphId};
    use crate::{ContentAlignment, Edge, Graph, Insets, Node, ShapeKind, Size};
    use std::cell::Cell;

    struct OwnerSearchContext;

    impl Context for OwnerSearchContext {
        fn done(&self) -> Option<bool> {
            None
        }
        fn error(&self) -> Option<limits::ContextError> {
            None
        }
    }

    fn production_eligibility_fixture() -> (ArenaGraph, NodeId, NodeId) {
        let mut input = Graph::default();
        let first = input.add_node(Node::new(
            "C",
            Size {
                width: 10.0,
                height: 10.0,
            },
        ));
        let second = input.add_node(Node::new(
            "D",
            Size {
                width: 10.0,
                height: 10.0,
            },
        ));
        input.add_edge(Edge {
            source: first,
            target: second,
        });
        let mut arena = ArenaGraph::from_input(&input);
        arena.set_position(first, Point::default());
        arena.set_position(second, Point { x: 1.0, y: 0.0 });
        arena.initialize_shared_topology();
        (arena, first, second)
    }

    #[test]
    fn owner_geometry_handoff_uses_canonical_references_for_duplicate_entity_ids() {
        let (mut owner, first, second) = production_eligibility_fixture();
        let mut workspace = owner.clone();
        owner.nodes[second.0 as usize].tala_id = owner.nodes[first.0 as usize].tala_id;
        let point = owner.allocate_owner_node_point(Some(Point { x: 17.0, y: 29.0 }));
        owner.bind_owner_node_point(second, point).unwrap();
        let mut child = workspace.nodes[second.0 as usize].clone();
        child.tala_id = owner.nodes[first.0 as usize].tala_id;
        child.owner_geometry_source = Some(second);
        child.owner_geometry_anchor = Some(first);
        child.position = Some(Point {
            x: 9000.0,
            y: 9000.0,
        });
        workspace
            .transaction_external_container_children
            .insert(42, vec![child]);
        refresh_owner_candidate_positions(&mut workspace, &owner, &[first, second]);
        let published = &workspace.transaction_external_container_children[&42][0];
        assert_eq!(published.position, Some(Point { x: 17.0, y: 29.0 }));
        assert_eq!(published.rect.origin, Point { x: 17.0, y: 29.0 });
        assert_eq!(
            published.transaction_anchor_offset,
            Some(Point { x: 17.0, y: 29.0 })
        );
        assert_eq!(workspace.position(second), owner.position(second));
    }

    #[test]
    fn production_owner_iteration_reads_shared_positions_and_retains_explicit_bindings() {
        let (mut owner, first, second) = production_eligibility_fixture();
        let mut workspace = owner.clone();
        let point = owner.allocate_owner_node_point(Some(Point::default()));
        owner.bind_owner_node_point(first, point.clone()).unwrap();
        let arena = owner.topology_arena_id().unwrap();
        let mapping =
            CanonicalScoreMapping::new(arena, vec![first, second], vec![super::super::EdgeId(0)]);
        let mut session =
            OwnerScoreSession::new(&mut owner, OwnerGraphId(0), mapping, None).unwrap();
        let mut optimizer = SizelessOptimizer::new(&mut workspace, 1);
        optimizer
            .optimize_with_owner_session(0.0, &mut session, &OwnerSearchContext)
            .unwrap();
        for node in [first, second] {
            assert!(optimizer.graph().position(node).is_some());
            assert_eq!(
                optimizer.graph().position(node),
                session.owner().position(node)
            );
        }
        assert!(
            session
                .owner()
                .owner_node_point(first)
                .unwrap()
                .same_allocation(&point)
        );
        optimizer.release_owner_geometry(&mut session);
        assert!(
            session
                .owner()
                .owner_node_point(first)
                .unwrap()
                .same_allocation(&point)
        );
        assert!(session.owner().position(second).is_some());
        let rng = optimizer.into_rng();
        assert!(rng.draw_count() > 0);
    }

    #[test]
    fn production_owner_phase_publishes_before_error_or_resumed_panic() {
        struct Stop(bool);
        impl Context for Stop {
            fn done(&self) -> Option<bool> {
                None
            }
            fn error(&self) -> Option<limits::ContextError> {
                if self.0 {
                    std::panic::panic_any(0x1577_u64);
                }
                Some(limits::ContextError::Cancelled)
            }
        }
        for panic in [false, true] {
            let (mut owner, first, second) = production_eligibility_fixture();
            let mut workspace = owner.clone();
            let mapping = CanonicalScoreMapping::new(
                owner.topology_arena_id().unwrap(),
                vec![first, second],
                vec![super::super::EdgeId(0)],
            );
            let mut session =
                OwnerScoreSession::new(&mut owner, OwnerGraphId(0), mapping, None).unwrap();
            let mut optimizer = SizelessOptimizer::new(&mut workspace, 1);
            optimizer
                .optimize_with_owner_session(0.0, &mut session, &OwnerSearchContext)
                .unwrap();
            for node in &mut optimizer.graph.nodes {
                node.position = Some(Point {
                    x: -999.0,
                    y: -999.0,
                });
            }
            let result = catch_unwind(AssertUnwindSafe(|| {
                optimizer.optimize_with_owner_session(0.0, &mut session, &Stop(panic))
            }));
            if panic {
                assert_eq!(*result.unwrap_err().downcast::<u64>().unwrap(), 0x1577);
            } else {
                assert!(
                    result
                        .unwrap()
                        .unwrap_err()
                        .to_string()
                        .contains("context canceled")
                );
            }
            for node in [first, second] {
                assert_eq!(
                    optimizer.graph.position(node),
                    session.owner().position(node)
                );
            }
            optimizer.release_owner_geometry(&mut session);
        }
    }

    #[test]
    fn production_worklist_keeps_raw_incident_nodes_when_graph_edges_are_narrowed() {
        let (mut arena, first, second) = production_eligibility_fixture();
        arena.edge_order.clear();
        assert_eq!(arena.active_edge_count(first), 0);
        assert_eq!(arena.nodes[first.0 as usize].edges.len(), 1);
        let owner_optimizer =
            crate::internal::owner_sizeless_constructor::OwnerSizelessOptimizer::new(
                &mut arena,
                Some(OwnerGraphId(0)),
                None,
                Some(&OwnerSearchContext),
            )
            .unwrap();
        let optimizer = SizelessOptimizer::with_rng(&mut arena, GoRng::new_untraced(1));
        assert_eq!(optimizer.nodes, vec![first, second]);
        assert_eq!(
            owner_optimizer.nodes().visible_slots(),
            vec![Some(first), Some(second)]
        );
    }

    #[test]
    fn production_worklist_reads_selected_tree_map_and_retains_constructor_selection() {
        let (mut arena, first, second) = production_eligibility_fixture();
        let foreign = arena
            .create_owner_graph_view(vec![first], Vec::new())
            .unwrap();
        arena
            .set_owner_node_graph_reference(first, Some(foreign))
            .unwrap();
        let excluded = arena.allocate_owner_node_to_tree_map(true);
        arena
            .write_owner_node_to_tree_entry(&excluded, Some(first), None)
            .unwrap();
        arena
            .set_owner_graph_node_to_tree_map(foreign, excluded.clone())
            .unwrap();
        assert_eq!(
            SizelessOptimizer::with_rng(&mut arena, GoRng::new_untraced(1)).nodes,
            vec![first, second]
        );
        arena
            .set_owner_graph_node_to_tree_map(OwnerGraphId(0), excluded.clone())
            .unwrap();
        let optimizer = SizelessOptimizer::with_rng(&mut arena, GoRng::new_untraced(1));
        assert_eq!(optimizer.nodes, vec![second]);
        assert!(!optimizer.graph.can_optimize_node(first));
        let replacement = optimizer.graph.allocate_owner_node_to_tree_map(false);
        optimizer
            .graph
            .set_owner_graph_node_to_tree_map(OwnerGraphId(0), replacement)
            .unwrap();
        assert!(optimizer.graph.can_optimize_node(first));
        assert_eq!(optimizer.nodes, vec![second]);
        // Replacing the selected field does not edit its previous allocation.
        assert_eq!(
            optimizer
                .graph
                .owner_graph_node_to_tree_entry(foreign, Some(first))
                .unwrap(),
            Some(None)
        );
    }

    #[test]
    fn production_eligibility_accepts_raw_nil_near_only_at_root_and_honors_fixed_position() {
        let (mut arena, first, second) = production_eligibility_fixture();
        arena.nodes[first.0 as usize].edges.clear();
        arena.nodes[first.0 as usize].nears = crate::internal::NodeNearMap::allocated();
        arena.nodes[first.0 as usize].nears.insert(None).unwrap();
        assert!(arena.can_optimize_node(first));
        arena.nodes[first.0 as usize].container = Some(second);
        assert!(!arena.can_optimize_node(first));
        arena.nodes[first.0 as usize].container = None;
        arena.nodes[first.0 as usize].fixed_top_left = Some(Point::default());
        assert!(!arena.can_optimize_node(first));
    }

    fn owner_search_fixture() -> (
        ArenaGraph,
        ArenaGraph,
        OwnerGraphId,
        CanonicalScoreMapping,
        [NodeId; 5],
    ) {
        let mut input = Graph::default();
        let c = input.add_node(Node::new(
            "C",
            Size {
                width: 60.0,
                height: 60.0,
            },
        ));
        let mut child = Node::new(
            "c",
            Size {
                width: 10.0,
                height: 10.0,
            },
        );
        child.parent = Some(c);
        let child = input.add_node(child);
        let leaf = input.add_node(Node::new(
            "L",
            Size {
                width: 10.0,
                height: 10.0,
            },
        ));
        let d = input.add_node(Node::new(
            "D",
            Size {
                width: 60.0,
                height: 60.0,
            },
        ));
        let mut foreign_child = Node::new(
            "d",
            Size {
                width: 10.0,
                height: 10.0,
            },
        );
        foreign_child.parent = Some(d);
        let foreign_child = input.add_node(foreign_child);
        input.add_edge(Edge {
            source: c,
            target: leaf,
        });
        let mut owner = ArenaGraph::from_input(&input);
        for (node, point) in [
            (c, Point::default()),
            (child, Point { x: 50.0, y: 50.0 }),
            (leaf, Point { x: 10.0, y: 0.0 }),
            (d, Point { x: 300.0, y: 80.0 }),
            (foreign_child, Point { x: 320.0, y: 100.0 }),
        ] {
            owner.set_position(node, point);
            owner.nodes[node.0 as usize].fixed_top_left = None;
        }
        owner.initialize_shared_topology();
        let (mut workspace, nodes, edges) =
            owner.induced_placement_subgraph_with_edge_mapping(&[c, leaf]);
        let view = owner
            .create_owner_graph_view(vec![c, leaf], edges.clone())
            .unwrap();
        owner
            .set_owner_view_node_mapping(view, nodes.clone())
            .unwrap();
        owner
            .set_owner_graph_reference(
                view,
                &[c, leaf],
                true,
                &mut crate::internal::OwnerGraphOwnershipJournal::default(),
            )
            .unwrap();
        owner.set_owner_graph_cell_size(view, 10.0).unwrap();
        let mapping = CanonicalScoreMapping::new(owner.topology_arena_id().unwrap(), nodes, edges);
        workspace.set_position(NodeId(0), Point { x: 123.0, y: 456.0 });
        workspace.set_position(
            NodeId(1),
            Point {
                x: -1000.0,
                y: 500.0,
            },
        );
        // Projected retained children are stale dense data too. The candidate
        // path must use C.Graph's actual c pointer and preserve its offset.
        for children in workspace.trace_container_children.values_mut() {
            for projected in children {
                projected.position = Some(Point {
                    x: 9000.0,
                    y: 9000.0,
                });
                projected.rect.origin = projected.position.unwrap();
            }
        }
        (
            owner,
            workspace,
            view,
            mapping,
            [c, child, leaf, d, foreign_child],
        )
    }

    fn mapped_owner_positions(session: &OwnerScoreSession<'_>) -> Vec<Option<Point>> {
        session
            .current_mapping()
            .nodes()
            .iter()
            .map(|&node| session.owner().position(node))
            .collect()
    }

    struct OwnerSearchCountingContext(Cell<usize>);

    impl Context for OwnerSearchCountingContext {
        fn done(&self) -> Option<bool> {
            None
        }
        fn error(&self) -> Option<limits::ContextError> {
            self.0.set(self.0.get() + 1);
            None
        }
    }

    #[test]
    fn owner_candidate_search_nil_first_charges_before_nil_and_restores_carriers() {
        for limit in [4, 5] {
            let (mut owner, mut workspace, view, mapping, _) = owner_search_fixture();
            let before: Vec<_> = owner.nodes.iter().map(|node| node.position).collect();
            let mut session = OwnerScoreSession::new(&mut owner, view, mapping, None).unwrap();
            let context = OwnerSearchCountingContext(Cell::new(0));
            let mut guard =
                OptimizationWorkGuard::new(Some(&context), "sizelessOptimizer.optimize", limit)
                    .unwrap();
            let mut optimizer = SizelessOptimizer::new(&mut workspace, 1);
            let error = optimizer
                .move_node_to_best_with_owner_nullable_session(
                    NodeId(0),
                    &[None],
                    &mut session,
                    &context,
                    &mut guard,
                )
                .unwrap_err();
            if limit == 4 {
                assert!(matches!(error, OptimizationError::ResourceLimit { .. }));
                assert_eq!(
                    error.to_string(),
                    "TALA optimization resource limit exceeded: TALA sizelessOptimizer.optimize work exceeds limit 4"
                );
                assert_eq!(guard.used(), 4);
            } else {
                assert_eq!(
                    error.to_string(),
                    "TALA sizelessOptimizer.optimize found a nil placement point"
                );
                assert_eq!(guard.used(), 5); // capture4 + nil point Step1
            }
            assert_eq!(context.0.get(), 1); // guard construction; no scorer/final check
            assert_eq!(
                session
                    .owner()
                    .nodes
                    .iter()
                    .map(|node| node.position)
                    .collect::<Vec<_>>(),
                before
            );
            assert_eq!(
                optimizer
                    .graph
                    .nodes
                    .iter()
                    .map(|node| node.position)
                    .collect::<Vec<_>>(),
                mapped_owner_positions(&session)
            );
        }
    }

    #[test]
    fn owner_candidate_search_nil_after_live_trial_restores_actual_child_and_carriers() {
        for limit in [15, 16] {
            let (mut owner, mut workspace, view, mapping, _) = owner_search_fixture();
            let before: Vec<_> = owner.nodes.iter().map(|node| node.position).collect();
            let mut session = OwnerScoreSession::new(&mut owner, view, mapping, None).unwrap();
            let context = OwnerSearchCountingContext(Cell::new(0));
            let mut guard =
                OptimizationWorkGuard::new(Some(&context), "sizelessOptimizer.optimize", limit)
                    .unwrap();
            let mut optimizer = SizelessOptimizer::new(&mut workspace, 1);
            let error = optimizer
                .move_node_to_best_with_owner_nullable_session(
                    NodeId(0),
                    &[Some(Point { x: 8.0, y: 0.0 }), None],
                    &mut session,
                    &context,
                    &mut guard,
                )
                .unwrap_err();
            if limit == 15 {
                assert!(matches!(error, OptimizationError::ResourceLimit { .. }));
                assert_eq!(
                    error.to_string(),
                    "TALA optimization resource limit exceeded: TALA sizelessOptimizer.optimize work exceeds limit 15"
                );
                assert_eq!(guard.used(), 15);
            } else {
                assert_eq!(
                    error.to_string(),
                    "TALA sizelessOptimizer.optimize found a nil placement point"
                );
                assert_eq!(guard.used(), 16); // live scored trial15 + nil point Step1
            }
            assert_eq!(context.0.get(), 8); // guard1 + live scorer7; no final check
            assert_eq!(
                session
                    .owner()
                    .nodes
                    .iter()
                    .map(|node| node.position)
                    .collect::<Vec<_>>(),
                before
            );
            assert_eq!(
                optimizer
                    .graph
                    .nodes
                    .iter()
                    .map(|node| node.position)
                    .collect::<Vec<_>>(),
                mapped_owner_positions(&session)
            );
        }
    }

    #[test]
    fn owner_candidate_search_explicit_nil_scoring_preserves_prepared_nonnil_session() {
        let (mut owner, mut workspace, view, mapping, [c, child, leaf, _, _]) =
            owner_search_fixture();
        let mut slice = owner
            .register_owner_edge_abductions(
                vec![crate::internal::OwnerEdgeAbduction {
                    source_edge: Some(crate::internal::EdgeId(0)),
                    originally_from: Some(child),
                    originally_to: None,
                    current_from: Some(c),
                    current_to: Some(leaf),
                }],
                true,
            )
            .unwrap();
        slice.records.push(slice.records[0]); // retain source ordered pointer duplicates
        let before = slice.clone();
        let records_backing = slice.records.as_ptr();
        let record_values: Vec<_> = slice
            .records
            .iter()
            .map(|&id| *owner.owner_edge_abduction(id).unwrap())
            .collect();
        let mut session = OwnerScoreSession::new(&mut owner, view, mapping, Some(&slice)).unwrap();
        let context = OwnerSearchCountingContext(Cell::new(0));
        let mut guard =
            OptimizationWorkGuard::new(Some(&context), "sizelessOptimizer.optimize", 1000).unwrap();
        let mut optimizer = SizelessOptimizer::new(&mut workspace, 1);
        assert_eq!(
            optimizer.move_node_to_best_with_owner_nullable_session(
                NodeId(0),
                &[Some(Point { x: 8.0, y: 0.0 })],
                &mut session,
                &context,
                &mut guard,
            ),
            Ok(true)
        );
        assert_eq!(guard.used(), 16);
        assert_eq!(context.0.get(), 9);
        assert_eq!(session.owner().position(c), Some(Point { x: 8.0, y: 0.0 }));
        assert_eq!(
            session.owner().position(child),
            Some(Point { x: 58.0, y: 50.0 })
        );
        assert!(std::ptr::eq(session.abductions().unwrap(), &slice));
        assert_eq!(session.abductions().unwrap(), &before);
        assert_eq!(
            session.abductions().unwrap().records.as_ptr(),
            records_backing
        );
        assert_eq!(
            slice
                .records
                .iter()
                .map(|&id| *session.owner().owner_edge_abduction(id).unwrap())
                .collect::<Vec<_>>(),
            record_values
        );
        session
            .with_frame(|frame| {
                assert!(std::ptr::eq(frame.abductions().unwrap(), &slice));
                assert_eq!(frame.abductions().unwrap(), &before);
            })
            .unwrap();
    }

    #[test]
    fn owner_phase_search_keeps_workspace_read_only_until_publication() {
        for fail in [false, true] {
            let (mut owner, mut workspace, view, mapping, [root, child, _, _, _]) =
                owner_search_fixture();
            let workspace_before = format!("{workspace:?}");
            let before = [owner.position(root), owner.position(child)];
            let mut session = OwnerScoreSession::new(&mut owner, view, mapping, None).unwrap();
            let context = OwnerSearchContext;
            let mut guard =
                OptimizationWorkGuard::new(Some(&context), "sizelessOptimizer.optimize", 1000)
                    .unwrap();
            let result = search_owner_node_to_best(
                &workspace,
                NodeId(0),
                [
                    Some(Point { x: 8.0, y: 0.0 }),
                    (!fail).then_some(Point { x: 10.0, y: 0.0 }),
                ]
                .into_iter(),
                |point| point,
                &mut session,
                &context,
                &mut guard,
            );
            assert_eq!(format!("{workspace:?}"), workspace_before);
            if fail {
                assert!(
                    result
                        .unwrap_err()
                        .to_string()
                        .contains("nil placement point")
                );
                assert_eq!(
                    [
                        session.owner().position(root),
                        session.owner().position(child)
                    ],
                    before
                );
            } else {
                assert_eq!(result, Ok(true));
                assert_eq!(
                    session.owner().position(root),
                    Some(Point { x: 10.0, y: 0.0 })
                );
                assert_eq!(
                    session.owner().position(child),
                    Some(Point { x: 60.0, y: 50.0 })
                );
            }
            refresh_owner_candidate_positions(
                &mut workspace,
                session.owner(),
                session.current_mapping().nodes(),
            );
            assert_eq!(
                workspace
                    .nodes
                    .iter()
                    .map(|node| node.position)
                    .collect::<Vec<_>>(),
                mapped_owner_positions(&session)
            );
        }
    }

    #[test]
    fn owner_candidate_search_commits_live_child_and_ignores_stale_workspace_geometry() {
        let (mut owner, mut workspace, view, mapping, [c, child, leaf, d, foreign_child]) =
            owner_search_fixture();
        let foreign_before = [owner.position(d), owner.position(foreign_child)];
        let child_address = &owner.nodes[child.0 as usize] as *const super::super::ArenaNode;
        let mut session = OwnerScoreSession::new(&mut owner, view, mapping, None).unwrap();
        let context = OwnerSearchContext;
        let mut guard =
            OptimizationWorkGuard::new(Some(&context), "sizelessOptimizer.optimize", 1000).unwrap();
        let mut optimizer = SizelessOptimizer::new(&mut workspace, 17);
        let rng_before = optimizer.rng.stream().borrow().draw_count();
        assert!(
            optimizer
                .move_node_to_best_with_owner_session(
                    NodeId(0),
                    &[
                        Point::default(),
                        Point { x: 8.0, y: 0.0 },
                        Point { x: 10.0, y: 0.0 }
                    ],
                    &mut session,
                    &context,
                    &mut guard,
                )
                .unwrap()
        );
        assert_eq!(session.owner().position(c), Some(Point { x: 10.0, y: 0.0 }));
        assert_eq!(
            session.owner().position(child),
            Some(Point { x: 60.0, y: 50.0 })
        );
        assert_eq!(
            session.owner().position(leaf),
            Some(Point { x: 10.0, y: 0.0 })
        );
        assert_eq!(
            [
                session.owner().position(d),
                session.owner().position(foreign_child)
            ],
            foreign_before
        );
        assert_eq!(
            &session.owner().nodes[child.0 as usize] as *const super::super::ArenaNode,
            child_address
        );
        assert_eq!(
            optimizer
                .graph
                .nodes
                .iter()
                .map(|node| node.position)
                .collect::<Vec<_>>(),
            mapped_owner_positions(&session)
        );
        assert_eq!(optimizer.rng.stream().borrow().draw_count(), rng_before);
    }

    #[test]
    fn owner_candidate_search_equal_score_favors_actual_original_coordinate() {
        let (mut owner, mut workspace, view, mapping, [c, child, _, _, _]) = owner_search_fixture();
        for edge in owner
            .edge_records
            .iter_mut()
            .chain(workspace.edge_records.iter_mut())
        {
            edge.source_arrow = false;
            edge.target_arrow = false;
        }
        let mut session = OwnerScoreSession::new(&mut owner, view, mapping, None).unwrap();
        let context = OwnerSearchContext;
        let mut guard =
            OptimizationWorkGuard::new(Some(&context), "sizelessOptimizer.optimize", 1000).unwrap();
        let mut optimizer = SizelessOptimizer::new(&mut workspace, 1);
        assert_eq!(
            optimizer.move_node_to_best_with_owner_session(
                NodeId(0),
                &[
                    Point { x: 20.0, y: 0.0 },
                    Point::default(),
                    Point { x: 20.0, y: 0.0 }
                ],
                &mut session,
                &context,
                &mut guard,
            ),
            Ok(false)
        );
        assert_eq!(session.owner().position(c), Some(Point::default()));
        assert_eq!(
            session.owner().position(child),
            Some(Point { x: 50.0, y: 50.0 })
        );
        assert_eq!(optimizer.graph.position(NodeId(0)), Some(Point::default()));
    }

    #[cfg(feature = "diagnostic-traces")]
    #[test]
    fn owner_candidate_search_scorer_frames_observe_each_live_retained_child() {
        struct ObserveTrial {
            root: NodeId,
            child: NodeId,
            trials: std::cell::RefCell<Vec<[Option<Point>; 2]>>,
        }
        impl Context for ObserveTrial {
            fn done(&self) -> Option<bool> {
                None
            }
            fn error(&self) -> Option<limits::ContextError> {
                None
            }
            fn scoring_state(
                &self,
                graph: &ArenaGraph,
                site: &'static str,
                node: Option<NodeId>,
                _iteration: Option<usize>,
            ) {
                if site == "node-edge-length-entry" {
                    assert_eq!(node, Some(self.root));
                    self.trials
                        .borrow_mut()
                        .push([graph.position(self.root), graph.position(self.child)]);
                }
            }
        }
        let (mut owner, mut workspace, view, mapping, [c, child, _, _, _]) = owner_search_fixture();
        let mut session = OwnerScoreSession::new(&mut owner, view, mapping, None).unwrap();
        let context = ObserveTrial {
            root: c,
            child,
            trials: std::cell::RefCell::new(Vec::new()),
        };
        let mut guard =
            OptimizationWorkGuard::new(Some(&context), "sizelessOptimizer.optimize", 1000).unwrap();
        let mut optimizer = SizelessOptimizer::new(&mut workspace, 1);
        assert_eq!(
            optimizer.move_node_to_best_with_owner_session(
                NodeId(0),
                &[Point { x: 8.0, y: 0.0 }, Point::default()],
                &mut session,
                &context,
                &mut guard,
            ),
            Ok(true)
        );
        assert_eq!(
            *context.trials.borrow(),
            vec![
                [
                    Some(Point { x: 8.0, y: 0.0 }),
                    Some(Point { x: 58.0, y: 50.0 })
                ],
                [Some(Point::default()), Some(Point { x: 50.0, y: 50.0 })],
            ]
        );
        assert_eq!(session.owner().position(c), Some(Point { x: 8.0, y: 0.0 }));
        assert_eq!(
            session.owner().position(child),
            Some(Point { x: 58.0, y: 50.0 })
        );
    }

    #[test]
    fn owner_candidate_search_charges_actual_node_graph_and_restores_at_scoring_limit() {
        for limit in [9, 16] {
            let (mut owner, mut workspace, view, mapping, [c, child, _, _, _]) =
                owner_search_fixture();
            let before = [owner.position(c), owner.position(child)];
            let mut session = OwnerScoreSession::new(&mut owner, view, mapping, None).unwrap();
            let context = OwnerSearchContext;
            let mut guard =
                OptimizationWorkGuard::new(Some(&context), "sizelessOptimizer.optimize", limit)
                    .unwrap();
            let mut optimizer = SizelessOptimizer::new(&mut workspace, 1);
            let result = optimizer.move_node_to_best_with_owner_session(
                NodeId(0),
                &[Point { x: 8.0, y: 0.0 }],
                &mut session,
                &context,
                &mut guard,
            );
            if limit == 9 {
                assert!(matches!(
                    result,
                    Err(OptimizationError::ResourceLimit { .. })
                ));
                assert_eq!(guard.used(), 9);
                assert_eq!(
                    [session.owner().position(c), session.owner().position(child)],
                    before
                );
            } else {
                assert_eq!(result, Ok(true));
                // Capture4 + candidate1 + movement4 + (Edges1+1)*(Graph.Nodes2+1) + selected no-op1.
                assert_eq!(guard.used(), 16);
                assert_eq!(session.owner().position(c), Some(Point { x: 8.0, y: 0.0 }));
                assert_eq!(
                    session.owner().position(child),
                    Some(Point { x: 58.0, y: 50.0 })
                );
            }
            assert_eq!(
                optimizer
                    .graph
                    .nodes
                    .iter()
                    .map(|node| node.position)
                    .collect::<Vec<_>>(),
                mapped_owner_positions(&session)
            );
        }
    }

    #[test]
    fn owner_candidate_search_empty_points_preserves_actual_geometry_and_refreshes_carriers() {
        let (mut owner, mut workspace, view, mapping, [c, child, _, _, _]) = owner_search_fixture();
        let before = [owner.position(c), owner.position(child)];
        let mut session = OwnerScoreSession::new(&mut owner, view, mapping, None).unwrap();
        let context = OwnerSearchContext;
        let mut guard =
            OptimizationWorkGuard::new(Some(&context), "sizelessOptimizer.optimize", 1000).unwrap();
        let mut optimizer = SizelessOptimizer::new(&mut workspace, 1);
        assert_eq!(
            optimizer
                .move_node_to_best_with_owner_session(
                    NodeId(0),
                    &[],
                    &mut session,
                    &context,
                    &mut guard
                )
                .unwrap_err()
                .to_string(),
            "sizelessOptimizer.moveNodeToBest: could not find any placement"
        );
        assert_eq!(
            [session.owner().position(c), session.owner().position(child)],
            before
        );
        assert_eq!(
            optimizer
                .graph
                .nodes
                .iter()
                .map(|node| node.position)
                .collect::<Vec<_>>(),
            mapped_owner_positions(&session)
        );
    }

    #[test]
    fn owner_candidate_search_context_failure_restores_actual_child_and_dense_carriers() {
        struct CancelAtScore(Cell<usize>);
        impl Context for CancelAtScore {
            fn done(&self) -> Option<bool> {
                None
            }
            fn error(&self) -> Option<limits::ContextError> {
                self.0.set(self.0.get() + 1);
                (self.0.get() == 2).then_some(limits::ContextError::Cancelled)
            }
        }
        let (mut owner, mut workspace, view, mapping, [c, child, _, _, _]) = owner_search_fixture();
        let before = [owner.position(c), owner.position(child)];
        let mut session = OwnerScoreSession::new(&mut owner, view, mapping, None).unwrap();
        let context = CancelAtScore(Cell::new(0));
        let mut guard =
            OptimizationWorkGuard::new(Some(&context), "sizelessOptimizer.optimize", 1000).unwrap();
        let mut optimizer = SizelessOptimizer::new(&mut workspace, 1);
        assert_eq!(
            optimizer.move_node_to_best_with_owner_session(
                NodeId(0),
                &[Point { x: 8.0, y: 0.0 }],
                &mut session,
                &context,
                &mut guard
            ),
            Err(OptimizationError::Context {
                location: "EdgeLength".into(),
                cause: limits::ContextError::Cancelled
            })
        );
        assert_eq!(context.0.get(), 2);
        assert_eq!(
            [session.owner().position(c), session.owner().position(child)],
            before
        );
        assert_eq!(
            optimizer
                .graph
                .nodes
                .iter()
                .map(|node| node.position)
                .collect::<Vec<_>>(),
            mapped_owner_positions(&session)
        );
    }

    #[test]
    fn owner_candidate_search_final_context_failure_reverts_successful_trial() {
        struct CancelAfterScore(Cell<usize>);
        impl Context for CancelAfterScore {
            fn done(&self) -> Option<bool> {
                None
            }
            fn error(&self) -> Option<limits::ContextError> {
                self.0.set(self.0.get() + 1);
                (self.0.get() == 9).then_some(limits::ContextError::Cancelled)
            }
        }
        let (mut owner, mut workspace, view, mapping, [c, child, _, _, _]) = owner_search_fixture();
        let before = [owner.position(c), owner.position(child)];
        let mut session = OwnerScoreSession::new(&mut owner, view, mapping, None).unwrap();
        let context = CancelAfterScore(Cell::new(0));
        let mut guard =
            OptimizationWorkGuard::new(Some(&context), "sizelessOptimizer.optimize", 1000).unwrap();
        let mut optimizer = SizelessOptimizer::new(&mut workspace, 1);
        assert_eq!(
            optimizer.move_node_to_best_with_owner_session(
                NodeId(0),
                &[Point { x: 8.0, y: 0.0 }],
                &mut session,
                &context,
                &mut guard,
            ),
            Err(OptimizationError::Context {
                location: "EdgeLength".into(),
                cause: limits::ContextError::Cancelled
            })
        );
        assert_eq!(context.0.get(), 9);
        assert_eq!(guard.used(), 15); // selected final move has not run
        assert_eq!(
            [session.owner().position(c), session.owner().position(child)],
            before
        );
        assert_eq!(
            optimizer
                .graph
                .nodes
                .iter()
                .map(|node| node.position)
                .collect::<Vec<_>>(),
            mapped_owner_positions(&session)
        );
    }

    #[test]
    fn owner_candidate_search_preserves_panic_payload_after_actual_and_dense_cleanup() {
        struct PanicAtScore(Cell<usize>);
        impl Context for PanicAtScore {
            fn done(&self) -> Option<bool> {
                None
            }
            fn error(&self) -> Option<limits::ContextError> {
                self.0.set(self.0.get() + 1);
                if self.0.get() == 2 {
                    panic!("candidate scorer panic");
                }
                None
            }
        }
        let (mut owner, mut workspace, view, mapping, [c, child, _, _, _]) = owner_search_fixture();
        let before = [owner.position(c), owner.position(child)];
        let mut session = OwnerScoreSession::new(&mut owner, view, mapping, None).unwrap();
        let context = PanicAtScore(Cell::new(0));
        let mut guard =
            OptimizationWorkGuard::new(Some(&context), "sizelessOptimizer.optimize", 1000).unwrap();
        let mut optimizer = SizelessOptimizer::new(&mut workspace, 1);
        let payload = catch_unwind(AssertUnwindSafe(|| {
            optimizer.move_node_to_best_with_owner_session(
                NodeId(0),
                &[Point { x: 8.0, y: 0.0 }],
                &mut session,
                &context,
                &mut guard,
            )
        }))
        .unwrap_err();
        assert_eq!(
            payload.downcast_ref::<&str>(),
            Some(&"candidate scorer panic")
        );
        assert_eq!(
            [session.owner().position(c), session.owner().position(child)],
            before
        );
        assert_eq!(
            optimizer
                .graph
                .nodes
                .iter()
                .map(|node| node.position)
                .collect::<Vec<_>>(),
            mapped_owner_positions(&session)
        );
    }

    #[test]
    fn owner_candidate_search_rejects_workspace_mapping_length_before_any_mutation() {
        let (mut owner, mut workspace, view, mapping, [c, child, _, _, _]) = owner_search_fixture();
        workspace.nodes.pop();
        let before = [owner.position(c), owner.position(child)];
        let dense_before = workspace.nodes[0].position;
        let mut session = OwnerScoreSession::new(&mut owner, view, mapping, None).unwrap();
        let context = OwnerSearchContext;
        let mut guard =
            OptimizationWorkGuard::new(Some(&context), "sizelessOptimizer.optimize", 1000).unwrap();
        // Construct from the valid fixture, then corrupt only this carrier's
        // mapping length; no optimizer initialization reads the missing node.
        let mut optimizer = SizelessOptimizer {
            graph: &mut workspace,
            nodes: vec![NodeId(0)],
            components: vec![vec![NodeId(0)]],
            occupied: OccupiedPointMap::new(),
            rng: OwnerOptimizerRng::new(GoRng::new(1)),
            owner_optimizer: None,
            materialized_owner_points: Vec::new(),
            materialized_owner_incidents: Vec::new(),
        };
        assert_eq!(
            optimizer
                .move_node_to_best_with_owner_session(
                    NodeId(0),
                    &[Point { x: 8.0, y: 0.0 }],
                    &mut session,
                    &context,
                    &mut guard
                )
                .unwrap_err()
                .to_string(),
            "sizeless workspace node mapping length differs"
        );
        assert_eq!(guard.used(), 0);
        assert_eq!(
            [session.owner().position(c), session.owner().position(child)],
            before
        );
        assert_eq!(optimizer.graph.nodes[0].position, dense_before);
    }

    fn node(name: &str, position: Point) -> Node {
        Node {
            d2_near_key: None,
            external_id: name.into(),
            size: Size {
                width: 40.0,
                height: 40.0,
            },
            declared_size: None,
            d2_authored_geometry: None,
            label_size: None,
            font_size: None,
            label_position: crate::LabelPosition::Unset,
            parent: None,
            locked_position: Some(position),
            constrained_x: None,
            constrained_y: None,
            near: None,
            fixed_width: false,
            fixed_height: false,
            direction: None,
            force_hierarchy: false,
            grid_rows: None,
            grid_columns: None,
            canvas_position: None,
            content_insets: Insets::uniform(0.0),
            layout_margins: Insets::uniform(0.0),
            external_label: None,
            icon_position: None,
            has_icon: false,
            label_aware_grid: false,
            packed_grid: false,
            content_alignment: ContentAlignment::Padding,
            port_spread: 0.0,
            person: false,
            is_3d: false,
            is_multiple: false,
            shape: ShapeKind::Rectangle,
        }
    }

    #[test]
    fn optimizer_occupied_keys_preserve_fractional_coordinates_and_member_order() {
        let mut input = Graph::default();
        let first = input.add_node(node("first", Point::default()));
        let second = input.add_node(node("second", Point::default()));
        let duplicate = input.add_node(node("duplicate", Point::default()));
        let unpositioned = input.add_node(node("unpositioned", Point::default()));
        let mut arena = ArenaGraph::from_input(&input);
        let first_point = Point { x: 0.2, y: 0.4 };
        let second_point = Point { x: 0.4, y: 0.2 };
        arena.set_position(first, first_point);
        arena.set_position(second, second_point);
        arena.set_position(duplicate, first_point);
        arena.clear_position(unpositioned);
        let optimizer = SizelessOptimizer::new(&mut arena, 1);

        assert_eq!(optimizer.occupied.len(), 2);
        assert_eq!(
            optimizer.occupied.lookup(first_point),
            Some(Some(duplicate))
        );
        assert_eq!(optimizer.occupied.lookup(second_point), Some(Some(second)));
        assert!(!optimizer.is_occupied(Point::default()));
        assert!(
            optimizer
                .placement_points(Point::default(), 0.0)
                .contains(&Point::default())
        );
        optimizer.occupied.delete(first_point);
        assert!(!optimizer.is_occupied(first_point));
        assert!(optimizer.is_occupied(second_point));
        optimizer.occupied.insert(first_point, Some(first));
        assert_eq!(optimizer.occupied.lookup(first_point), Some(Some(first)));
    }

    #[test]
    fn optimizer_occupied_presence_handles_nil_values_infinities_and_nan_keys() {
        let mut input = Graph::default();
        let positive = input.add_node(node("positive", Point::default()));
        let negative = input.add_node(node("negative", Point::default()));
        let nan_node = input.add_node(node("nan", Point::default()));
        let mut arena = ArenaGraph::from_input(&input);
        let positive_point = Point {
            x: f64::INFINITY,
            y: 1.0,
        };
        let negative_point = Point {
            x: f64::NEG_INFINITY,
            y: 1.0,
        };
        let nan_point = Point {
            x: f64::from_bits(0x7ff8_0000_0000_0007),
            y: 1.0,
        };
        arena.set_position(positive, positive_point);
        arena.set_position(negative, negative_point);
        arena.set_position(nan_node, nan_point);
        let optimizer = SizelessOptimizer::new(&mut arena, 1);

        assert_eq!(optimizer.occupied.len(), 3);
        assert!(optimizer.is_occupied(positive_point));
        assert!(optimizer.is_occupied(negative_point));
        assert!(!optimizer.is_occupied(Point {
            x: i64::MAX as f64,
            y: 1.0
        }));
        assert!(!optimizer.is_occupied(nan_point));
        optimizer.occupied.delete(nan_point);
        assert_eq!(optimizer.occupied.len(), 3);
        optimizer.occupied.insert(Point::default(), None);
        assert!(optimizer.is_occupied(Point::default()));
        assert_eq!(optimizer.occupied.lookup(Point::default()), Some(None));
    }

    #[test]
    fn optimizer_swap_candidates_add_offsets_to_exact_fractional_position() {
        let mut input = Graph::default();
        let point = Point { x: -0.25, y: 0.2 };
        let center = input.add_node(node("center", point));
        let adjacent_points = [
            Point {
                x: point.x,
                y: point.y - 1.0,
            },
            Point {
                x: point.x,
                y: point.y + 1.0,
            },
            Point {
                x: point.x - 1.0,
                y: point.y,
            },
            Point {
                x: point.x + 1.0,
                y: point.y,
            },
        ];
        let mut adjacent = Vec::new();
        for (index, position) in adjacent_points.into_iter().enumerate() {
            let candidate = input.add_node(node(&format!("adjacent-{index}"), position));
            input.add_edge(Edge {
                source: center,
                target: candidate,
            });
            adjacent.push(candidate);
        }
        let rounded_decoy = input.add_node(node("rounded-decoy", Point { x: 0.0, y: -1.0 }));
        input.add_edge(Edge {
            source: center,
            target: rounded_decoy,
        });
        let mut arena = ArenaGraph::from_input(&input);
        for node in &mut arena.nodes {
            node.fixed_top_left = None;
        }
        let optimizer = SizelessOptimizer::new(&mut arena, 1);

        assert_eq!(optimizer.swap_candidates(center), adjacent);
    }

    #[test]
    fn diamond_scan_matches_recovered_order() {
        let mut input = Graph::default();
        input.add_node(node("center", Point { x: 0.0, y: 0.0 }));
        input.add_node(node("right", Point { x: 1.0, y: 0.0 }));
        let mut arena = ArenaGraph::from_input(&input);
        let optimizer = SizelessOptimizer::new(&mut arena, 1);

        assert_eq!(
            optimizer.closest_unoccupied_distance(Point::default()),
            Some(1.0)
        );
        assert_eq!(
            optimizer.placement_points(Point::default(), 0.0),
            vec![
                Point { x: 0.0, y: -1.0 },
                Point { x: 0.0, y: 1.0 },
                Point { x: -1.0, y: 0.0 },
            ]
        );
    }

    #[test]
    fn optimizer_is_deterministic_with_go_seeded_shuffling() {
        fn arena() -> ArenaGraph {
            let mut input = Graph::default();
            let a = input.add_node(node("a", Point { x: 0.0, y: 0.0 }));
            let b = input.add_node(node("b", Point { x: 4.0, y: 0.0 }));
            let c = input.add_node(node("c", Point { x: 4.0, y: 4.0 }));
            input.add_edge(Edge {
                source: a,
                target: b,
            });
            input.add_edge(Edge {
                source: b,
                target: c,
            });
            let mut arena = ArenaGraph::from_input(&input);
            for node in &mut arena.nodes {
                node.fixed_top_left = None;
            }
            arena
        }

        let mut first = arena();
        let mut second = arena();
        SizelessOptimizer::new(&mut first, 1).optimize(0.0);
        SizelessOptimizer::new(&mut second, 1).optimize(0.0);

        assert_eq!(
            first
                .nodes
                .iter()
                .map(|node| node.position)
                .collect::<Vec<_>>(),
            second
                .nodes
                .iter()
                .map(|node| node.position)
                .collect::<Vec<_>>()
        );
        let unique: std::collections::BTreeSet<_> = first
            .nodes
            .iter()
            .map(|node| {
                let point = node.position.unwrap();
                (point.x as i64, point.y as i64)
            })
            .collect();
        assert_eq!(unique.len(), 3);
    }

    #[test]
    fn guarded_optimizer_preserves_layout_and_rng_stream() {
        struct Background;
        impl crate::internal::limits::Context for Background {
            fn done(&self) -> Option<bool> {
                None
            }
            fn error(&self) -> Option<crate::internal::limits::ContextError> {
                None
            }
        }
        fn arena() -> ArenaGraph {
            let mut input = Graph::default();
            let a = input.add_node(node("a", Point { x: 0.0, y: 0.0 }));
            let b = input.add_node(node("b", Point { x: 4.0, y: 0.0 }));
            let c = input.add_node(node("c", Point { x: 4.0, y: 4.0 }));
            input.add_edge(Edge {
                source: a,
                target: b,
            });
            input.add_edge(Edge {
                source: b,
                target: c,
            });
            let mut arena = ArenaGraph::from_input(&input);
            for node in &mut arena.nodes {
                node.fixed_top_left = None;
            }
            arena
        }

        let mut ordinary = arena();
        let mut guarded = arena();
        let mut ordinary_optimizer = SizelessOptimizer::new(&mut ordinary, 17);
        ordinary_optimizer.optimize(0.0);
        let ordinary_rng = ordinary_optimizer.into_rng();
        let mut guarded_optimizer = SizelessOptimizer::new(&mut guarded, 17);
        let context = Background;
        guarded_optimizer
            .optimize_with_limit(
                0.0,
                &context,
                super::super::limits::MAX_OPTIMIZATION_WORK_UNITS,
            )
            .unwrap();
        let guarded_rng = guarded_optimizer.into_rng();
        assert_eq!(
            ordinary
                .nodes
                .iter()
                .map(|node| node.position)
                .collect::<Vec<_>>(),
            guarded
                .nodes
                .iter()
                .map(|node| node.position)
                .collect::<Vec<_>>()
        );
        assert_eq!(ordinary_rng.draw_count(), guarded_rng.draw_count());
        assert_eq!(
            ordinary_rng.clone_without_trace().float64().to_bits(),
            guarded_rng.clone_without_trace().float64().to_bits()
        );
    }

    #[test]
    fn swap_candidates_follow_recovered_compass_order() {
        let mut input = Graph::default();
        let center = input.add_node(node("center", Point { x: 0.0, y: 0.0 }));
        let above = input.add_node(node("above", Point { x: 0.0, y: -1.0 }));
        let below = input.add_node(node("below", Point { x: 0.0, y: 1.0 }));
        let left = input.add_node(node("left", Point { x: -1.0, y: 0.0 }));
        let right = input.add_node(node("right", Point { x: 1.0, y: 0.0 }));
        for adjacent in [above, below, left, right] {
            input.add_edge(Edge {
                source: center,
                target: adjacent,
            });
        }
        let mut arena = ArenaGraph::from_input(&input);
        for node in &mut arena.nodes {
            node.fixed_top_left = None;
        }
        let optimizer = SizelessOptimizer::new(&mut arena, 1);

        assert_eq!(
            optimizer.swap_candidates(center),
            vec![above, below, left, right]
        );
    }

    #[test]
    fn guarded_swap_preserves_present_nil_error_and_attempts_every_direction() {
        let mut input = Graph::default();
        let center = input.add_node(node("center", Point { x: 0.25, y: 0.5 }));
        let mut arena = ArenaGraph::from_input(&input);
        let mut optimizer = SizelessOptimizer::new(&mut arena, 1);
        let context = OwnerSearchContext;
        let nil_key = Point { x: 0.25, y: -0.5 };
        optimizer.occupied.insert(nil_key, None);
        let mut guard = OptimizationWorkGuard::new(Some(&context), "adjacent", 4).unwrap();
        let error = optimizer
            .best_swap_candidate_guarded(center, &mut guard)
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "TALA adjacent found a nil optimizer node"
        );
        assert_eq!(guard.used(), 4);
        assert_eq!(optimizer.rng.stream().borrow().draw_count(), 0);
        optimizer.occupied.delete(nil_key);
        let mut guard = OptimizationWorkGuard::new(Some(&context), "adjacent", 4).unwrap();
        assert!(
            optimizer
                .swap_candidates_guarded(center, &mut guard)
                .unwrap()
                .is_empty()
        );
        assert_eq!(guard.used(), 4);
    }

    #[test]
    fn guarded_swap_later_work_rejection_overwrites_earlier_nil_error() {
        let mut input = Graph::default();
        let center = input.add_node(node("center", Point { x: 0.0, y: 0.0 }));
        let mut arena = ArenaGraph::from_input(&input);
        let optimizer = SizelessOptimizer::new(&mut arena, 1);
        optimizer.occupied.insert(Point { x: 0.0, y: -1.0 }, None);
        let context = OwnerSearchContext;
        let mut guard = OptimizationWorkGuard::new(Some(&context), "adjacent", 2).unwrap();
        let error = optimizer
            .swap_candidates_guarded(center, &mut guard)
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "TALA optimization resource limit exceeded: TALA adjacent work exceeds limit 2"
        );
        assert_eq!(guard.used(), 2);
    }

    #[test]
    fn guarded_swap_work_rejection_precedes_nil_position_dereference() {
        let mut input = Graph::default();
        let center = input.add_node(node("center", Point { x: 0.0, y: 0.0 }));
        let mut arena = ArenaGraph::from_input(&input);
        arena.nodes[center.0 as usize].position = None;
        let optimizer = SizelessOptimizer::new(&mut arena, 1);
        let context = OwnerSearchContext;
        let mut guard = OptimizationWorkGuard::new(Some(&context), "adjacent", 1).unwrap();
        guard.step().unwrap();
        let error = optimizer
            .swap_candidates_guarded(center, &mut guard)
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "TALA optimization resource limit exceeded: TALA adjacent work exceeds limit 1"
        );
        assert_eq!(guard.used(), 1);
        let mut guard = OptimizationWorkGuard::new(Some(&context), "adjacent", 4).unwrap();
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _ = optimizer.swap_candidates_guarded(center, &mut guard);
            }))
            .is_err()
        );
        assert_eq!(guard.used(), 1);
    }

    #[test]
    fn best_swap_requires_both_nodes_not_to_regress() {
        let mut input = Graph::default();
        let moving = input.add_node(node("moving", Point { x: 0.0, y: 0.0 }));
        let candidate = input.add_node(node("candidate", Point { x: 1.0, y: 0.0 }));
        let moving_anchor = input.add_node(node("moving anchor", Point { x: 2.0, y: 0.0 }));
        let candidate_anchor = input.add_node(node("candidate anchor", Point { x: -1.0, y: 0.0 }));
        input.add_edge(Edge {
            source: moving,
            target: moving_anchor,
        });
        input.add_edge(Edge {
            source: candidate,
            target: candidate_anchor,
        });
        let mut arena = ArenaGraph::from_input(&input);
        for node in &mut arena.nodes {
            node.fixed_top_left = None;
        }
        let mut optimizer = SizelessOptimizer::new(&mut arena, 1);

        assert_eq!(optimizer.best_swap_candidate(moving), Some(candidate));
        assert_eq!(
            optimizer.graph.position(moving),
            Some(Point { x: 0.0, y: 0.0 })
        );
        assert_eq!(
            optimizer.graph.position(candidate),
            Some(Point { x: 1.0, y: 0.0 })
        );
    }

    #[test]
    fn fixed_sibling_origin_clamps_optimizer_candidates() {
        let mut input = Graph::default();
        let parent = input.add_node(node("parent", Point { x: 120.0, y: 200.0 }));
        let mut child_node = node("child", Point { x: 3.0, y: 5.0 });
        child_node.parent = Some(parent);
        let child = input.add_node(child_node);
        let mut anchor_node = node("anchor", Point { x: 120.0, y: 200.0 });
        anchor_node.parent = Some(parent);
        let anchor = input.add_node(anchor_node);
        let mut arena = ArenaGraph::from_input(&input);
        arena.nodes[child.0 as usize].fixed_top_left = None;
        arena.cell_size = 40.0;
        // Recovered getFixedOrigin is current minus requested. The fixed
        // sibling therefore contributes (120, 200), or (3, 5) in cells.
        arena.nodes[anchor.0 as usize].position = Some(Point { x: 240.0, y: 400.0 });
        let optimizer = SizelessOptimizer::new(&mut arena, 1);

        let points = optimizer.placement_points_for(Some(child), Point { x: 3.0, y: 5.0 }, 0.0);
        assert!(!points.is_empty());
        assert!(points.iter().all(|point| point.x >= 3.0 && point.y >= 5.0));
    }
}
