// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! The error/rollback boundary of packing/binpack.go::packAtomic.
//!
//! The compact arena still differs from Go's pointer inventory and inner
//! transaction work accounting. Charge actual visited operations here; never
//! replace the missing inner guard propagation with an estimated total.
use super::*;
use binpack_traversal::PackingNodeRef;
#[path = "binpack_commit.rs"]
mod commit;
pub(super) use commit::CommitState as GeometryCommitState;
#[path = "binpack_operations.rs"]
mod operations;
pub(super) use operations::{PackingOperation, PackingOperations};
#[path = "binpack_request.rs"]
mod request;
pub(super) use request::{GeometryTransactionOptions, PackingGeometryTransaction};

pub(super) const MAX_BIN_PACK_WORK_UNITS: i64 = 1_000_000_000;

pub(super) struct PackingContext;
impl limits::Context for PackingContext {
    fn stable_packing_topology(&self) -> bool { true }
    fn done(&self) -> Option<bool> {
        cancellation_done()
    }
    fn error(&self) -> Option<limits::ContextError> {
        cancellation_error()
    }
}

/// Go returns its computed values together with a failure from the final
/// Finish check. Earlier validation failures return false and no incidents.
#[derive(Debug)]
pub(super) struct BinPackRouteFailure {
    pub(super) error: limits::Error,
    pub(super) routed: bool,
    pub(super) incident: Vec<EdgeId>,
}
impl From<limits::Error> for BinPackRouteFailure {
    fn from(error: limits::Error) -> Self {
        Self {
            error,
            routed: false,
            incident: Vec::new(),
        }
    }
}

#[derive(Clone, Copy)]
struct Geometry {
    position: Option<Point>,
    rect: Rect,
    anchor_offset: Option<Point>,
    was_nil: bool,
}
impl Geometry {
    fn capture(node: &ArenaNode) -> Self {
        Self {
            position: node.position,
            rect: node.rect,
            anchor_offset: node.transaction_anchor_offset,
            was_nil: node.transaction_position_was_nil,
        }
    }
    fn restore(&self, node: &mut ArenaNode) {
        node.position = self.position;
        node.rect = self.rect;
        node.transaction_anchor_offset = self.anchor_offset;
        node.transaction_position_was_nil = self.was_nil;
    }
}

pub(super) struct PackingState {
    nodes: BTreeMap<NodeId, (Geometry, Option<shared_topology::OwnerNodePointSnapshot>)>,
    virtual_nodes: BTreeMap<PackingNodeRef, (Option<Point>, Option<Size>, Option<Point>)>,
    commit_geometry: crate::internal::IdHashMap<PackingNodeRef, (Option<Point>, Size)>,
    captured_has_fixed: bool,
    edge_geometry: CapturedEdgeGeometry,
    clusters: Vec<(usize, ClusterArrangement, ClusterArrangement, f64)>,
    orientations: BTreeMap<NodeId, Orientation>,
    order: Vec<NodeId>,
    /// Whether the Graph.Nodes membership index was current at capture.
    order_membership_valid: bool,
    members: Option<shared_topology::OwnerGeometryMembersSnapshot>,
    owner_clusters: Vec<(owner_groups::OwnerGroupId, String, String, f64)>,
    owner_trees: Vec<(owner_tree_map::OwnerTreeId, i64)>,
    // These are additional coordinate views of the same compact-arena nodes,
    // not copies of unrelated topology, caches or arbitrary graph fields.
    external: Vec<Geometry>,
    children: BTreeMap<i64, Vec<Geometry>>,
    aggregates: BTreeMap<i64, Vec<Geometry>>,
    vessels: BTreeMap<usize, Point>,
    serialized_vessels: BTreeMap<i64, Point>,
    serialized_sizes: BTreeMap<i64, Size>,
}
impl PackingState {
    fn record_value(&mut self, graph: &ArenaGraph, node: PackingNodeRef) {
        // transaction.go::recordNode freezes validation geometry at this
        // node's charged visit, before later Context.Err callbacks can mutate it.
        let (position, size, metadata) = graph.packing_bounds_geometry(node);
        self.commit_geometry.insert(node, (position, size));
        self.captured_has_fixed |= metadata.is_some_and(|node| node.fixed_top_left.is_some());
        let PackingNodeRef::Node(node) = node else {
            let id = graph.packing_ref_id(node);
            let pending = match node {
                PackingNodeRef::Cluster(index) => {
                    graph.pending_cluster_vessel_positions.get(&index).copied()
                }
                _ => None,
            };
            self.virtual_nodes.insert(
                node,
                (
                    graph.serialized_vessel_positions_by_tala.get(&id).copied(),
                    graph.serialized_vessel_sizes_by_tala.get(&id).copied(),
                    pending,
                ),
            );
            return;
        };
        self.nodes.insert(
            node,
            (
                Geometry::capture(&graph.nodes[node.0 as usize]),
                graph
                    .owner_node_point(node)
                    .ok()
                    .map(|point| point.snapshot()),
            ),
        );
    }
    pub(super) fn capture(
        graph: &ArenaGraph,
        guard: &mut limits::WorkGuard<'_>,
        capture_edge_routes: bool,
    ) -> Result<Self, limits::Error> {
        use gap_transaction_work::{GeometryCaptureEvent, GeometrySnapshotObject};
        let mut state: Option<Self> = None;
        let mut edge_geometry = None;
        let packing_node = |object: GeometrySnapshotObject| match object {
            GeometrySnapshotObject::Node(node) => PackingNodeRef::Node(node),
            GeometrySnapshotObject::Cluster(index) => PackingNodeRef::Cluster(index),
            GeometrySnapshotObject::Sequence(index) => PackingNodeRef::Sequence(index),
        };
        graph.geometry_transaction_capture_walk_with_routes(
            guard,
            &mut |graph, event| {
                if let GeometryCaptureEvent::Begin = event {
                    state = Some(Self {
                        nodes: BTreeMap::new(),
                        virtual_nodes: BTreeMap::new(),
                        commit_geometry: Default::default(),
                        captured_has_fixed: false,
                        edge_geometry: CapturedEdgeGeometry {
                            edges: BTreeMap::new(),
                            routes: BTreeMap::new(),
                        },
                        clusters: graph
                            .clusters
                            .iter()
                            .enumerate()
                            .map(|(i, c)| (i, c.arrangement, c.desired_arrangement, c.padding))
                            .collect(),
                        orientations: graph
                            .tree_routing_nodes
                            .iter()
                            .map(|(&node, tree)| (node, tree.orientation))
                            .collect(),
                        order: graph.node_order.clone(),
                        order_membership_valid: graph.node_order_membership_valid,
                        members: graph.geometry_members_snapshot(OwnerGraphId(0)).ok(),
                        owner_clusters: Vec::new(),
                        owner_trees: Vec::new(),
                        external: Vec::new(),
                        children: BTreeMap::new(),
                        aggregates: BTreeMap::new(),
                        vessels: graph.pending_cluster_vessel_positions.clone(),
                        serialized_vessels: graph.serialized_vessel_positions_by_tala.clone(),
                        serialized_sizes: graph.serialized_vessel_sizes_by_tala.clone(),
                    });
                    return;
                }
                let state = state.as_mut().expect("packing capture preflight completed");
                match event {
                    GeometryCaptureEvent::Node(object) => {
                        state.record_value(graph, packing_node(object))
                    }
                    GeometryCaptureEvent::Cluster(group) => {
                        let policy = graph
                            .owner_cluster_policy(group)
                            .expect("captured packing cluster");
                        state.owner_clusters.push((
                            group,
                            policy.arrangement,
                            policy.desired_arrangement,
                            policy.padding,
                        ));
                    }
                    GeometryCaptureEvent::Tree(tree) => state.owner_trees.push((
                        tree,
                        graph
                            .owner_tree(tree)
                            .expect("captured packing Tree")
                            .fields()
                            .orientation,
                    )),
                    GeometryCaptureEvent::End => {
                        state.external = graph
                            .transaction_external_containers
                            .iter()
                            .map(Geometry::capture)
                            .collect();
                        state.children = graph
                            .transaction_external_container_children
                            .iter()
                            .map(|(&key, nodes)| {
                                (key, nodes.iter().map(Geometry::capture).collect())
                            })
                            .collect();
                        state.aggregates = graph
                            .transaction_external_aggregate_children
                            .iter()
                            .map(|(&key, nodes)| {
                                (key, nodes.iter().map(Geometry::capture).collect())
                            })
                            .collect();
                    }
                    GeometryCaptureEvent::Begin => unreachable!(),
                }
            },
            &mut |graph, nodes, guard| {
                let nodes = nodes.iter().map(packing_node).collect::<Vec<_>>();
                edge_geometry = Some(capture_edge_geometry(
                    graph,
                    &nodes,
                    guard,
                    capture_edge_routes,
                )?);
                Ok(())
            },
            false,
        )?;
        let mut state = state.expect("successful packing capture begins");
        state.edge_geometry = edge_geometry.expect("packing capture visits route phase");
        Ok(state)
    }
    pub(super) fn restore(&self, graph: &mut ArenaGraph) {
        for (node, (geometry, point)) in &self.nodes {
            // An originally inline point must not be hidden by a binding
            // introduced during the candidate being rolled back.
            if point.is_none() {
                graph.release_materialized_owner_node_point(*node);
            }
            geometry.restore(&mut graph.nodes[node.0 as usize]);
            if let Some(point) = point {
                graph
                    .restore_owner_node_point(*node, point)
                    .expect("packing retains node point ownership");
            }
        }
        for (&node, &(position, size, pending)) in &self.virtual_nodes {
            let id = graph.packing_ref_id(node);
            if let Some(position) = position {
                graph
                    .serialized_vessel_positions_by_tala
                    .insert(id, position);
            } else {
                graph.serialized_vessel_positions_by_tala.remove(&id);
            }
            if let Some(size) = size {
                graph.serialized_vessel_sizes_by_tala.insert(id, size);
            } else {
                graph.serialized_vessel_sizes_by_tala.remove(&id);
            }
            if let PackingNodeRef::Cluster(index) = node {
                if let Some(pending) = pending {
                    graph
                        .pending_cluster_vessel_positions
                        .insert(index, pending);
                } else {
                    graph.pending_cluster_vessel_positions.remove(&index);
                }
            }
        }
        self.edge_geometry.restore(graph);
        for (index, arrangement, desired, padding) in &self.clusters {
            let cluster = &mut graph.clusters[*index];
            cluster.arrangement = *arrangement;
            cluster.desired_arrangement = *desired;
            cluster.padding = *padding;
        }
        for (node, orientation) in &self.orientations {
            if let Some(tree) = graph.tree_routing_nodes.get_mut(&node) {
                tree.orientation = *orientation;
            }
        }
        for (group, arrangement, desired, padding) in &self.owner_clusters {
            graph
                .set_owner_cluster_orientation_policy(
                    *group,
                    arrangement.clone(),
                    desired.clone(),
                    *padding,
                )
                .expect("packing cluster rollback");
        }
        for &(tree, orientation) in &self.owner_trees {
            graph
                .set_owner_tree_orientation(tree, orientation)
                .expect("packing Tree rollback");
        }
        if let Some(members) = &self.members {
            graph
                .restore_geometry_members(members.clone())
                .expect("packing member rollback");
        }
        graph.node_order.clone_from(&self.order);
        graph.node_order_membership_valid = false;
        for (node, geometry) in graph
            .transaction_external_containers
            .iter_mut()
            .zip(&self.external)
        {
            geometry.restore(node);
        }
        for (key, values) in &self.children {
            if let Some(nodes) = graph.transaction_external_container_children.get_mut(&key) {
                for (node, geometry) in nodes.iter_mut().zip(values) {
                    geometry.restore(node);
                }
            }
        }
        for (key, values) in &self.aggregates {
            if let Some(nodes) = graph.transaction_external_aggregate_children.get_mut(&key) {
                for (node, geometry) in nodes.iter_mut().zip(values) {
                    geometry.restore(node);
                }
            }
        }
        graph
            .pending_cluster_vessel_positions
            .clone_from(&self.vessels);
        graph
            .serialized_vessel_positions_by_tala
            .clone_from(&self.serialized_vessels);
        graph
            .serialized_vessel_sizes_by_tala
            .clone_from(&self.serialized_sizes);
        // Every input of the membership index (Graph.Nodes, members and
        // group backlinks) is restored above. Rebuild it so later trials keep
        // the indexed lookups that the captured state had.
        if self.order_membership_valid {
            graph.refresh_node_order_membership();
        }
    }
}
/// Geometry snapshot for repeated source transactions. Candidate mutations
/// borrow the actual arena; the captured state restores rejected trials.
pub(super) struct PackingTrialSnapshot {
    state: PackingState,
    commit: commit::CommitState,
}
impl PackingTrialSnapshot {
    pub(super) fn capture(
        graph: &ArenaGraph,
        capture_edge_routes: bool,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<Self, limits::Error> {
        guard.finish()?;
        guard.finish()?;
        Self::capture_refresh(graph, capture_edge_routes, guard)
    }
    fn capture_refresh(
        graph: &ArenaGraph,
        capture_edge_routes: bool,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<Self, limits::Error> {
        let state = PackingState::capture(graph, guard, capture_edge_routes)?;
        let commit = commit::CommitState::capture(graph, &state);
        Ok(Self { state, commit })
    }
    pub(super) fn trial<'a>(&'a self, graph: &'a mut ArenaGraph) -> PackingTrial<'a> {
        PackingTrial {
            graph,
            state: &self.state,
            commit: &self.commit,
            accepted: false,
        }
    }
}

pub(super) struct PackingTrial<'a> {
    graph: &'a mut ArenaGraph,
    state: &'a PackingState,
    commit: &'a commit::CommitState,
    accepted: bool,
}
impl std::ops::Deref for PackingTrial<'_> {
    type Target = ArenaGraph;
    fn deref(&self) -> &ArenaGraph {
        self.graph
    }
}
impl std::ops::DerefMut for PackingTrial<'_> {
    fn deref_mut(&mut self) -> &mut ArenaGraph {
        self.graph
    }
}
impl Drop for PackingTrial<'_> {
    fn drop(&mut self) {
        if !self.accepted {
            self.state.restore(self.graph);
        }
    }
}
impl PackingTrial<'_> {
    /// Transaction.Commit's entry and one operation, through its post-op ctx.Err.
    /// Group synchronization and remaining Commit validation are later boundaries.
    /// Failure/panic restores the real receiver before the caller's next poll.
    pub(super) fn apply_operation_checked(
        &mut self,
        guard: &mut limits::WorkGuard<'_>,
        operation: impl FnOnce(&mut ArenaGraph, &mut limits::WorkGuard<'_>) -> Result<(), limits::Error>,
    ) -> Result<(), limits::Error> {
        use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
        let result = catch_unwind(AssertUnwindSafe(|| {
            guard.finish()?;
            guard.check_context()?;
            guard.step()?;
            guard.check_context()?;
            operation(self.graph, guard)?;
            guard.check_context()
        }));
        match result {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => {
                self.state.restore(self.graph);
                Err(error)
            }
            Err(payload) => {
                self.state.restore(self.graph);
                resume_unwind(payload)
            }
        }
    }
    /// The BinPack transaction uses IgnoreContainerEscape=true and leaves
    /// AffectContainers=false. Validate the actual post-sync receiver; a rejected
    /// candidate is restored before the caller can poll its own work guard.
    pub(super) fn apply_packing_commit_checked(
        &mut self,
        overlaps: &BTreeSet<(NodeId, NodeId)>,
        exact_overlaps: &BTreeSet<(NodeId, NodeId)>,
        guard: &mut limits::WorkGuard<'_>,
        operation: impl FnOnce(&mut ArenaGraph, &mut limits::WorkGuard<'_>) -> Result<(), limits::Error>,
    ) -> Result<bool, limits::Error> {
        self.apply_packing_operations_checked(
            overlaps,
            exact_overlaps,
            guard,
            GeometryTransactionOptions::packing(false),
            || std::iter::once(operation),
        )
    }
    pub(super) fn apply_queued_packing_commit_checked(
        &mut self,
        overlaps: &BTreeSet<(NodeId, NodeId)>,
        exact_overlaps: &BTreeSet<(NodeId, NodeId)>,
        guard: &mut limits::WorkGuard<'_>,
        operations: &PackingOperations<'_>,
    ) -> Result<bool, limits::Error> {
        self.apply_queued_transaction_commit_checked(
            overlaps,
            exact_overlaps,
            guard,
            operations,
            GeometryTransactionOptions::packing(false),
        )
    }
    pub(super) fn apply_queued_transaction_commit_checked(
        &mut self,
        overlaps: &BTreeSet<(NodeId, NodeId)>,
        exact_overlaps: &BTreeSet<(NodeId, NodeId)>,
        guard: &mut limits::WorkGuard<'_>,
        operations: &PackingOperations<'_>,
        options: GeometryTransactionOptions,
    ) -> Result<bool, limits::Error> {
        self.apply_packing_operations_checked(overlaps, exact_overlaps, guard, options, || {
            operations.range().map(|operation| {
                move |graph: &mut ArenaGraph, guard: &mut limits::WorkGuard<'_>| {
                    operation
                        .expect("runtime error: invalid memory address or nil pointer dereference")(
                        graph, guard,
                    )
                }
            })
        })
    }
    fn apply_packing_operations_checked<I, F>(
        &mut self,
        overlaps: &BTreeSet<(NodeId, NodeId)>,
        exact_overlaps: &BTreeSet<(NodeId, NodeId)>,
        guard: &mut limits::WorkGuard<'_>,
        options: GeometryTransactionOptions,
        operations: impl FnOnce() -> I,
    ) -> Result<bool, limits::Error>
    where
        I: IntoIterator<Item = F>,
        F: FnOnce(&mut ArenaGraph, &mut limits::WorkGuard<'_>) -> Result<(), limits::Error>,
    {
        use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
        let result = catch_unwind(AssertUnwindSafe(|| {
            guard.finish()?;
            guard.check_context()?;
            for operation in operations() {
                guard.step()?;
                guard.check_context()?;
                operation(self.graph, guard)?;
                guard.check_context()?;
                if options.affect_containers
                    && !self
                        .graph
                        .transaction_refit_containers_with_options_checked(
                            |node| self.commit.original_point(node),
                            overlaps,
                            options.ignore_container_escape,
                            None,
                            guard,
                        )?
                {
                    return Ok(false);
                }
                self.graph.sync_clusters();
                self.graph.sync_sequences();
                guard.check_context()?;
            }
            self.commit.validate_with_options(
                self.graph,
                overlaps,
                exact_overlaps,
                options.ignore_container_escape,
                guard,
            )
        }));
        match result {
            Ok(Ok(true)) => Ok(true),
            Ok(Ok(false)) => {
                self.state.restore(self.graph);
                Ok(false)
            }
            Ok(Err(error)) => {
                self.state.restore(self.graph);
                Err(error)
            }
            Err(payload) => {
                self.state.restore(self.graph);
                resume_unwind(payload)
            }
        }
    }
    pub(super) fn accept(mut self) {
        self.accepted = true;
    }
}

struct PackingTransaction<'a> {
    graph: &'a mut ArenaGraph,
    state: Option<PackingState>,
}
impl Drop for PackingTransaction<'_> {
    fn drop(&mut self) {
        if let Some(state) = self.state.take() {
            state.restore(self.graph);
        }
    }
}

impl ArenaGraph {
    #[cfg(feature = "diagnostic-traces")]
    pub(super) fn probe_packing_snapshot(
        &mut self,
        guard: &mut limits::WorkGuard<'_>,
        routed: bool,
        request_prefix: bool,
        repeats: usize,
    ) -> Result<(), limits::Error> {
        if request_prefix {
            guard.finish()?;
            guard.finish()?;
        }
        let state = PackingState::capture(self, guard, routed)?;
        for _ in 0..repeats {
            for node in self.node_order.clone() {
                if let Some(p) = self.position(node) {
                    let replacement = self.allocate_owner_node_point(Some(Point {
                        x: p.x + 37.0,
                        y: p.y - 19.0,
                    }));
                    self.bind_owner_node_point(node, replacement).unwrap();
                }
                self.nodes[node.0 as usize].rect.size.width += 11.0;
                self.nodes[node.0 as usize].rect.size.height += 7.0;
            }
            for edge in &self.edge_order.read() {
                for mut point in self.edge_records[edge.0 as usize].route.iter_mut() {
                    point.x += 13.0;
                    point.y -= 17.0;
                }
            }
            state.restore(self);
        }
        Ok(())
    }
    pub(super) fn bin_pack_recovered(&mut self) -> Result<(), limits::Error> {
        let context = PackingContext;
        let mut guard =
            limits::WorkGuard::new(Some(&context), "BinPack", limits::MAX_ENGINE_WORK_UNITS)?;
        guard.set_limit(MAX_BIN_PACK_WORK_UNITS);
        self.bin_pack_atomic_with_guard(&mut guard, false)
    }
    pub(super) fn bin_pack_atomic_with_guard(
        &mut self,
        guard: &mut limits::WorkGuard<'_>,
        extra_root_grid: bool,
    ) -> Result<(), limits::Error> {
        let state = PackingState::capture(self, guard, true)?;
        let mut transaction = PackingTransaction {
            graph: self,
            state: Some(state),
        };
        transaction.graph.bin_pack_guarded(guard)?;
        if extra_root_grid {
            transaction
                .graph
                .bin_pack_scope_guarded(None, false, guard)?;
        }
        guard.finish()?;
        transaction.state = None;
        Ok(())
    }
    // Current numerical carriers map to concrete vessels when an actual owner
    // binding exists. Dense entity IDs are never compared to source EntityIDs.
    fn bin_pack_inventory_node(&self, node: NodeId) -> NodeId {
        if self.nodes.get(node.0 as usize).is_none() || !self.bin_pack_node_is_vessel_carrier(node)
        {
            return node;
        }
        self.active_cluster_index(node)
            .and_then(|index| self.topology_vessel_node(TopologyVessel::Cluster(index)))
            .or_else(|| {
                self.active_sequence_index(node)
                    .and_then(|index| self.topology_vessel_node(TopologyVessel::Sequence(index)))
            })
            .unwrap_or(node)
    }
    fn bin_pack_inventory_endpoint(&self, edge: EdgeId, is_to: bool) -> Option<NodeId> {
        self.owner_edge_endpoint(edge, is_to)
            .ok()
            .flatten()
            .map(|node| {
                if self.nodes.get(node.0 as usize).is_none() {
                    return node;
                }
                let owner = self.active_aggregate_owner(node);
                if self.bin_pack_node_is_vessel_carrier(owner) {
                    self.bin_pack_inventory_node(owner)
                } else {
                    node
                }
            })
    }
    fn bin_pack_inventory_owned(&self, node: NodeId) -> bool {
        self.nodes.get(node.0 as usize).is_some()
            && (self.topology_arena_id().is_none()
                || self.topology_node_owner(node) == Some(OwnerGraphId(0)))
    }
    fn bin_pack_inventory_incidents(&self, node: NodeId) -> Vec<Option<EdgeId>> {
        if let Ok(slots) = self.owner_node_incident_slots(node) {
            return slots;
        }
        // Unbound numerical incidence omits retired tree edges. Explicit raw
        // bindings above retain every slot, including nil/nonincident edges.
        if self.bin_pack_node_is_vessel_carrier(node) {
            self.active_edge_ids(node).into_iter().map(Some).collect()
        } else {
            self.nodes[node.0 as usize]
                .edges
                .read()
                .iter()
                .copied()
                .filter(|edge| self.edge_order.contains(edge))
                .map(Some)
                .collect()
        }
    }
    fn bin_pack_descendants_guarded(
        &self,
        root: NodeId,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<Vec<NodeId>, limits::Error> {
        let mut stack = Vec::new();
        let mut result = Vec::new();
        let mut seen = BTreeSet::from([root]);
        let push_children = |parent: NodeId,
                             stack: &mut Vec<NodeId>,
                             guard: &mut limits::WorkGuard<'_>|
         -> Result<(), limits::Error> {
            for (index, sequence) in self.sequences.iter().enumerate() {
                if self.topology_vessel_node(TopologyVessel::Sequence(index)) == Some(parent)
                    || self.nodes[parent.0 as usize].tala_id == sequence.vessel_tala_id
                {
                    for &node in sequence.members.iter().rev() {
                        guard.step()?;
                        stack.push(node);
                    }
                }
            }
            for (index, cluster) in self.clusters.iter().enumerate() {
                if self.topology_vessel_node(TopologyVessel::Cluster(index)) == Some(parent)
                    || self.nodes[parent.0 as usize].tala_id == cluster.vessel_tala_id
                {
                    for &node in cluster.members.iter().rev() {
                        guard.step()?;
                        stack.push(node);
                    }
                }
            }
            if self.nodes[parent.0 as usize].is_container {
                for node in self
                    .containers
                    .get(&Some(parent))
                    .unwrap_or_default()
                    .into_iter()
                    .rev()
                {
                    guard.step()?;
                    stack.push(self.bin_pack_inventory_node(node));
                }
            }
            Ok(())
        };
        push_children(root, &mut stack, guard)?;
        while let Some(node) = stack.pop() {
            guard.step()?;
            if !seen.insert(node) {
                continue;
            }
            result.push(node);
            push_children(node, &mut stack, guard)?;
        }
        guard.finish()?;
        Ok(result)
    }
    /// packing/guard.go::allEdgesHaveCompleteRoutesGuarded. Validation order
    /// matters: ownership and multiplicity errors precede route completeness,
    /// and partial routing precedes the per-node incidence scan.
    pub(super) fn bin_pack_complete_routes(
        &self,
        root: Option<NodeId>,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<(bool, Vec<EdgeId>), BinPackRouteFailure> {
        let invalid = |message: String| {
            BinPackRouteFailure::from(limits::Error::Invalid(format!(
                "layout invariant violated: {message}"
            )))
        };
        let affected = if let Some(root) = root {
            let mut nodes = vec![root];
            nodes.extend(self.bin_pack_descendants_guarded(root, guard)?);
            nodes
        } else {
            self.node_order
                .iter()
                .copied()
                .map(|n| self.bin_pack_inventory_node(n))
                .collect()
        };
        let mut expected = BTreeMap::<NodeId, BTreeMap<EdgeId, usize>>::new();
        for &node in &affected {
            guard.step()?;
            if !self.bin_pack_inventory_owned(node) {
                return Err(invalid(
                    "BinPack affected-node inventory is malformed".into(),
                ));
            }
            if expected.insert(node, BTreeMap::new()).is_some() {
                return Err(invalid(format!(
                    "BinPack affected-node inventory repeats node {}",
                    self.nodes[node.0 as usize].tala_id
                )));
            }
        }
        let edges: Vec<Option<EdgeId>> = match self.owner_graph_edge_header(OwnerGraphId(0)) {
            Ok(header) => header.iter_slots().collect(),
            Err(_) => self.edge_order.read().iter().copied().map(Some).collect(),
        };
        let mut affected_edges = BTreeSet::new();
        let mut routed = 0;
        let mut incident = Vec::new();
        for (index, edge_id) in edges.iter().copied().enumerate() {
            guard.step()?;
            let Some(edge_id) = edge_id else {
                return Err(invalid(format!(
                    "BinPack found a nil edge at index {index}"
                )));
            };
            let edge = &self.edge_records[edge_id.0 as usize];
            let (Some(from), Some(to)) = (
                self.bin_pack_inventory_endpoint(edge_id, false),
                self.bin_pack_inventory_endpoint(edge_id, true),
            ) else {
                return Err(invalid(format!(
                    "BinPack edge {} has a nil endpoint",
                    edge.tala_id
                )));
            };
            if !self.bin_pack_inventory_owned(from) || !self.bin_pack_inventory_owned(to) {
                return Err(invalid(format!(
                    "BinPack edge {} has an endpoint owned by another graph",
                    edge.tala_id
                )));
            }
            let mut affects = false;
            if let Some(counts) = expected.get_mut(&from) {
                *counts.entry(edge_id).or_default() += 1;
                affects = true;
            }
            if to != from {
                if let Some(counts) = expected.get_mut(&to) {
                    *counts.entry(edge_id).or_default() += 1;
                    affects = true;
                }
            }
            if affects && !affected_edges.insert(edge_id) {
                return Err(invalid(format!(
                    "BinPack graph edge inventory repeats affected edge {}",
                    edge.tala_id
                )));
            }
            if edge.route.is_empty() {
                continue;
            }
            if edge.route.len() < 2 {
                return Err(invalid(format!(
                    "BinPack edge {} has an incomplete route with {} point(s)",
                    edge.tala_id,
                    edge.route.len()
                )));
            }
            for _ in &edge.route.read() {
                guard.step()?;
            }
            routed += 1;
            if root.is_some_and(|root| root == from || root == to) {
                incident.push(edge_id);
            }
        }
        if routed != 0 && routed != edges.len() {
            return Err(invalid(format!(
                "BinPack graph is partially routed: {routed} of {} edges have routes",
                edges.len()
            )));
        }
        for node in affected {
            let mut counts = BTreeMap::<EdgeId, usize>::new();
            let actual = self.bin_pack_inventory_incidents(node);
            for (index, edge) in actual.iter().copied().enumerate() {
                guard.step()?;
                let Some(edge_id) = edge else {
                    return Err(invalid(format!(
                        "BinPack node {} edge inventory contains a nil edge at index {index}",
                        self.nodes[node.0 as usize].tala_id
                    )));
                };
                let edge = &self.edge_records[edge_id.0 as usize];
                if self.bin_pack_inventory_endpoint(edge_id, false) != Some(node)
                    && self.bin_pack_inventory_endpoint(edge_id, true) != Some(node)
                {
                    return Err(invalid(format!(
                        "BinPack node {} edge inventory contains non-incident edge {}",
                        self.nodes[node.0 as usize].tala_id, edge.tala_id
                    )));
                }
                let actual = counts.entry(edge_id).or_default();
                *actual += 1;
                let needed = expected[&node].get(&edge_id).copied().unwrap_or(0);
                if *actual > needed {
                    return Err(invalid(format!(
                        "BinPack node {} edge inventory has {actual} occurrence(s) of edge {}; graph requires {needed}",
                        self.nodes[node.0 as usize].tala_id, edge.tala_id
                    )));
                }
            }
            let needed: usize = expected[&node].values().sum();
            if actual.len() != needed {
                return Err(invalid(format!(
                    "BinPack node {} edge inventory has {} occurrence(s); graph requires {needed}",
                    self.nodes[node.0 as usize].tala_id,
                    actual.len()
                )));
            }
        }
        if let Err(error) = guard.finish() {
            return Err(BinPackRouteFailure {
                error,
                routed: routed != 0,
                incident,
            });
        }
        Ok((routed != 0, incident))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct PollContext {
        polls: Cell<usize>,
        cancel_at: usize,
        deadline: bool,
    }
    impl limits::Context for PollContext {
        fn done(&self) -> Option<bool> {
            None
        }
        fn error(&self) -> Option<limits::ContextError> {
            let poll = self.polls.get() + 1;
            self.polls.set(poll);
            (self.cancel_at > 0 && poll >= self.cancel_at).then_some(if self.deadline {
                limits::ContextError::DeadlineExceeded
            } else {
                limits::ContextError::Cancelled
            })
        }
    }

    fn prepared() -> ArenaGraph {
        let mut input = Graph::default();
        for index in 0..4 {
            input.add_node(crate::Node::new(
                index.to_string(),
                Size {
                    width: 100.0,
                    height: 80.0,
                },
            ));
        }
        let mut graph = ArenaGraph::from_input(&input);
        for index in 0..4 {
            graph.set_position(
                NodeId(index),
                Point {
                    x: f64::from(index) * 200.0,
                    y: f64::from(index) * 150.0,
                },
            );
        }
        graph
    }
    fn geometry(graph: &ArenaGraph) -> Vec<(Option<Point>, Rect)> {
        graph
            .nodes
            .iter()
            .map(|n| (graph.position(n.input_id), n.rect))
            .collect()
    }
    #[test]
    fn packing_rolls_back_every_observed_budget_and_cancellation_failure() {
        let context = PollContext {
            polls: Cell::new(0),
            cancel_at: 0,
            deadline: false,
        };
        let mut graph = prepared();
        let before = geometry(&graph);
        let mut guard =
            limits::WorkGuard::new(Some(&context), "BinPack", MAX_BIN_PACK_WORK_UNITS).unwrap();
        graph.bin_pack_atomic_with_guard(&mut guard, false).unwrap();
        assert_ne!(
            before,
            geometry(&graph),
            "fixture must actually move geometry"
        );
        let work = guard.used();
        let polls = context.polls.get();
        for limit in 0..work {
            let mut graph = prepared();
            let before = geometry(&graph);
            let mut guard = limits::WorkGuard::new(Some(&context), "BinPack", limit).unwrap();
            assert!(
                matches!(
                    graph.bin_pack_atomic_with_guard(&mut guard, false),
                    Err(limits::Error::Limit { .. })
                ),
                "limit={limit}"
            );
            assert_eq!(geometry(&graph), before, "limit={limit}");
        }
        for deadline in [false, true] {
            for cancel_at in 1..=polls {
                let context = PollContext {
                    polls: Cell::new(0),
                    cancel_at,
                    deadline,
                };
                let mut graph = prepared();
                let before = geometry(&graph);
                let result = (|| {
                    let mut guard =
                        limits::WorkGuard::new(Some(&context), "BinPack", MAX_BIN_PACK_WORK_UNITS)?;
                    graph.bin_pack_atomic_with_guard(&mut guard, false)
                })();
                // Source Commit polls both its WorkGuard (BinPack prefix)
                // and ctx.Err directly (raw cause). This exhaustive native
                // interruption test verifies rollback/cause; the source
                // operation fixture asserts each exact error and poll site.
                let error = result.unwrap_err();
                let limits::Error::Context { location, cause } = error else {
                    panic!("unexpected interruption error {error:?}");
                };
                assert!(location.is_empty() || location == "BinPack");
                assert_eq!(
                    cause,
                    if deadline {
                        limits::ContextError::DeadlineExceeded
                    } else {
                        limits::ContextError::Cancelled
                    }
                );
                assert_eq!(geometry(&graph), before, "poll={cancel_at}");
            }
        }
    }
    #[test]
    fn reusable_trial_snapshot_restores_replaced_points_between_candidates() {
        let mut graph = prepared();
        graph.initialize_shared_topology();
        graph.materialize_owner_node_point(NodeId(0)).unwrap();
        let original = geometry(&graph);
        let context = PollContext {
            polls: Cell::new(0),
            cancel_at: 0,
            deadline: false,
        };
        let mut guard =
            limits::WorkGuard::new(Some(&context), "BinPack", MAX_BIN_PACK_WORK_UNITS).unwrap();
        let saved = PackingTrialSnapshot::capture(&graph, false, &mut guard).unwrap();
        let node = NodeId(0);
        let point = graph.owner_node_point(node).unwrap();
        {
            let mut trial = saved.trial(&mut graph);
            let replacement = trial.allocate_owner_node_point(Some(Point { x: 99.0, y: -41.0 }));
            trial.bind_owner_node_point(node, replacement).unwrap();
            trial.nodes[0].rect.size.width = 3.0;
            assert_ne!(geometry(&trial), original);
        }
        assert_eq!(geometry(&graph), original);
        assert!(
            graph
                .owner_node_point(node)
                .unwrap()
                .same_allocation(&point)
        );
        {
            let mut trial = saved.trial(&mut graph);
            trial.translate_owner_point(node, 55.0, 67.0);
        }
        assert_eq!(geometry(&graph), original);
    }

    #[test]
    fn reusable_snapshot_removes_a_binding_created_for_an_inline_point() {
        let mut graph = prepared();
        graph.initialize_shared_topology();
        let before = geometry(&graph);
        let context = PollContext {
            polls: Cell::new(0),
            cancel_at: 0,
            deadline: false,
        };
        let mut guard =
            limits::WorkGuard::new(Some(&context), "BinPack", MAX_BIN_PACK_WORK_UNITS).unwrap();
        let saved = PackingState::capture(&graph, &mut guard, false).unwrap();
        let replacement = graph.allocate_owner_node_point(Some(Point {
            x: -700.0,
            y: 333.0,
        }));
        graph.bind_owner_node_point(NodeId(0), replacement).unwrap();
        saved.restore(&mut graph);
        assert_eq!(geometry(&graph), before);
        assert!(graph.owner_node_point(NodeId(0)).is_err());
        graph.translate_owner_point(NodeId(0), 1.0, 2.0);
        saved.restore(&mut graph);
        assert_eq!(geometry(&graph), before);
    }

    #[test]
    fn packing_snapshot_visits_and_restores_real_route_tail_allocations() {
        let mut input = Graph::default();
        let a = input.add_node(crate::Node::new(
            "a",
            Size {
                width: 10.0,
                height: 10.0,
            },
        ));
        let b = input.add_node(crate::Node::new(
            "b",
            Size {
                width: 10.0,
                height: 10.0,
            },
        ));
        input.add_edge(crate::Edge {
            source: a,
            target: b,
        });
        let mut graph = ArenaGraph::from_input(&input);
        graph.initialize_shared_topology();
        let p = graph.allocate_owner_node_point(Some(Point { x: 1.0, y: 2.0 }));
        let nil = graph.allocate_owner_node_point(None);
        let hidden = graph.allocate_owner_node_point(Some(Point { x: 30.0, y: 40.0 }));
        let route = graph
            .allocate_owner_route_header(&[p, nil], 5, true)
            .unwrap();
        graph.write_owner_route_slot(&route, 4, &hidden).unwrap();
        graph
            .bind_owner_edge_route(EdgeId(0), route.clone())
            .unwrap();
        let context = PollContext {
            polls: Cell::new(0),
            cancel_at: 0,
            deadline: false,
        };
        let mut guard =
            limits::WorkGuard::new(Some(&context), "BinPack", MAX_BIN_PACK_WORK_UNITS).unwrap();
        let saved = PackingState::capture(&graph, &mut guard, true).unwrap();
        let with_tail_work = guard.used();
        graph
            .write_owner_point(&hidden, Point { x: 90.0, y: 91.0 })
            .unwrap();
        let empty = graph.allocate_owner_route_header(&[], 0, false).unwrap();
        graph.bind_owner_edge_route(EdgeId(0), empty).unwrap();
        saved.restore(&mut graph);
        let restored = graph.owner_edge_route(EdgeId(0)).unwrap();
        assert!(restored.same_backing(&route));
        assert_eq!(hidden.get(), Some(Point { x: 30.0, y: 40.0 }));
        assert!(restored.capacity_point(4).unwrap().same_allocation(&hidden));
        graph
            .bind_owner_edge_route(EdgeId(0), route.reslice(0, 2, 2).unwrap())
            .unwrap();
        let mut guard =
            limits::WorkGuard::new(Some(&context), "BinPack", MAX_BIN_PACK_WORK_UNITS).unwrap();
        PackingState::capture(&graph, &mut guard, true).unwrap();
        assert_eq!(with_tail_work - guard.used(), 3);
    }

    #[test]
    fn geometry_snapshot_omits_routes_when_transaction_does_not_affect_them() {
        let mut input = Graph::default();
        let a = input.add_node(crate::Node::new(
            "a",
            Size {
                width: 10.0,
                height: 10.0,
            },
        ));
        let b = input.add_node(crate::Node::new(
            "b",
            Size {
                width: 10.0,
                height: 10.0,
            },
        ));
        input.add_edge(crate::Edge {
            source: a,
            target: b,
        });
        let mut graph = ArenaGraph::from_input(&input);
        graph.edge_records[0]
            .route
            .replace(RoutePoints::literal(&(vec![Point { x: 3.0, y: 5.0 }])));
        let context = PollContext {
            polls: Cell::new(0),
            cancel_at: 0,
            deadline: false,
        };
        for routed in [false, true] {
            let mut guard =
                limits::WorkGuard::new(Some(&context), "BinPack", MAX_BIN_PACK_WORK_UNITS).unwrap();
            let before = graph.edge_records[0].route.read().clone();
            let saved = PackingState::capture(&graph, &mut guard, routed).unwrap();
            graph.edge_records[0].route.point_mut(0).x += 17.0;
            saved.restore(&mut graph);
            assert_eq!(graph.edge_records[0].route.read() == before, routed);
        }
    }

    #[test]
    fn packing_unwind_restores_point_alias_and_routes_without_resetting_caches() {
        let mut graph = prepared();
        graph.initialize_shared_topology();
        let node = NodeId(0);
        let origin = graph.position(node).unwrap();
        let point = graph.allocate_owner_node_point(Some(origin));
        graph.bind_owner_node_point(node, point.clone()).unwrap();
        let context = PollContext {
            polls: Cell::new(0),
            cancel_at: 0,
            deadline: false,
        };
        let mut guard =
            limits::WorkGuard::new(Some(&context), "BinPack", MAX_BIN_PACK_WORK_UNITS).unwrap();
        let before = geometry(&graph);
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let state = PackingState::capture(&graph, &mut guard, true).unwrap();
            let transaction = PackingTransaction {
                graph: &mut graph,
                state: Some(state),
            };
            transaction
                .graph
                .write_owner_point(
                    &point,
                    Point {
                        x: 10000.0,
                        y: 20000.0,
                    },
                )
                .unwrap();
            let replacement = transaction.graph.allocate_owner_node_point(None);
            transaction
                .graph
                .bind_owner_node_point(node, replacement)
                .unwrap();
            transaction.graph.nodes[0].rect.size.width = 1.0;
            transaction.graph.cell_size = 731.0;
            panic!("packing test panic");
        }));
        assert!(panic.is_err());
        assert_eq!(geometry(&graph), before);
        assert_eq!(point.get(), Some(origin));
        assert!(point.same_allocation(&graph.owner_node_point(node).unwrap()));
        assert_eq!(
            graph.cell_size, 731.0,
            "source rollback leaves graph caches alone"
        );
    }
    #[test]
    fn packing_rejects_incomplete_and_mixed_routes_before_geometry_changes() {
        let mut input = Graph::default();
        for i in 0..4 {
            input.add_node(crate::Node::new(
                i.to_string(),
                Size {
                    width: 100.0,
                    height: 80.0,
                },
            ));
        }
        input.add_edge(crate::Edge {
            source: NodeId(0),
            target: NodeId(1),
        });
        input.add_edge(crate::Edge {
            source: NodeId(2),
            target: NodeId(3),
        });
        for count in [1, 2] {
            let mut graph = ArenaGraph::from_input(&input);
            for i in 0..4 {
                graph.set_position(
                    NodeId(i),
                    Point {
                        x: f64::from(i) * 200.0,
                        y: 0.0,
                    },
                );
            }
            graph.edge_records[0].tala_id = 10;
            graph.edge_records[0].route.replace(RoutePoints::literal(
                &(vec![Point { x: 50.0, y: 40.0 }; count]),
            ));
            let before = geometry(&graph);
            let points = graph.edge_records[0].route.read().clone();
            let error = graph.bin_pack_recovered().unwrap_err().to_string();
            assert_eq!(
                error,
                if count == 1 {
                    "layout invariant violated: BinPack edge 10 has an incomplete route with 1 point(s)"
                } else {
                    "layout invariant violated: BinPack graph is partially routed: 1 of 2 edges have routes"
                }
            );
            assert_eq!(geometry(&graph), before);
            assert_eq!(graph.edge_records[0].route.read(), points);
        }
    }
    #[test]
    fn packing_keeps_the_production_deadline_cause() {
        let mut graph = prepared();
        let before = geometry(&graph);
        let token = CancellationToken::new();
        token.expire();
        let error = with_cancellation(Some(&token), || graph.bin_pack_recovered()).unwrap_err();
        assert_eq!(
            error,
            limits::Error::Context {
                location: "BinPack".into(),
                cause: limits::ContextError::DeadlineExceeded
            }
        );
        assert_eq!(geometry(&graph), before);
    }
    #[test]
    fn packing_restores_routes_on_late_work_exhaustion() {
        let mut input = Graph::default();
        for i in 0..4 {
            input.add_node(crate::Node::new(
                i.to_string(),
                Size {
                    width: 100.0,
                    height: 80.0,
                },
            ));
        }
        for i in [0, 2] {
            input.add_edge(crate::Edge {
                source: NodeId(i),
                target: NodeId(i + 1),
            });
        }
        let mut original = ArenaGraph::from_input(&input);
        for i in 0..4 {
            original.set_position(
                NodeId(i),
                Point {
                    x: f64::from(i) * 200.0,
                    y: f64::from(i) * 150.0,
                },
            );
        }
        for i in 0..2 {
            original.edge_records[i].route.replace(RoutePoints::literal(
                &(vec![
                    Point {
                        x: (i * 400 + 50) as f64,
                        y: (i * 300 + 40) as f64,
                    },
                    Point {
                        x: (i * 400 + 250) as f64,
                        y: (i * 300 + 190) as f64,
                    },
                ]),
            ));
        }
        let original_geometry = geometry(&original);
        let original_routes = original
            .member_edges()
            .map(|e| e.route.read().clone())
            .collect::<Vec<_>>();
        let context = PollContext {
            polls: Cell::new(0),
            cancel_at: 0,
            deadline: false,
        };
        let mut graph = original.clone();
        let mut guard =
            limits::WorkGuard::new(Some(&context), "BinPack", MAX_BIN_PACK_WORK_UNITS).unwrap();
        graph.bin_pack_atomic_with_guard(&mut guard, false).unwrap();
        assert_ne!(
            graph
                .member_edges()
                .map(|e| e.route.read().clone())
                .collect::<Vec<_>>(),
            original_routes,
            "successful fixture must move routes"
        );
        for limit in 0..guard.used() {
            let mut graph = original.clone();
            let mut guard = limits::WorkGuard::new(Some(&context), "BinPack", limit).unwrap();
            assert!(graph.bin_pack_atomic_with_guard(&mut guard, false).is_err());
            assert_eq!(geometry(&graph), original_geometry, "limit={limit}");
            assert_eq!(
                graph
                    .member_edges()
                    .map(|e| e.route.read().clone())
                    .collect::<Vec<_>>(),
                original_routes,
                "limit={limit}"
            );
        }
    }

    fn decoded(input: &[u8]) -> ArenaGraph {
        let input = crate::internal::graphjson::graphjson_to_graph(input).unwrap();
        let mut graph = ArenaGraph::from_input(&input);
        graph.initialize_shared_topology();
        graph
    }
    #[test]
    fn packing_final_score_checks_restore_the_single_node_scope() {
        // Pinned Go Pack on this input uses 33 units and 21 context polls.
        // Units 30-33 and polls 16-19 belong to the post-loop visit/score;
        // polls 20-21 finish the enclosing scope and atomic operation.
        // This catches an unchecked final scorer even when packing has no
        // candidate iterations and the geometry itself would still match.
        let input = br#"{"nodes":[{"id":1,"width":100,"height":80,"topLeft":{"x":17,"y":31},"shapeType":"Square"}],"edges":[],"containers":{"0":[1]},"clusters":{}}"#;
        for limit in 29..=33 {
            let mut graph = decoded(input);
            let before = geometry(&graph);
            let context = PollContext {
                polls: Cell::new(0),
                cancel_at: 0,
                deadline: false,
            };
            let mut guard = limits::WorkGuard::new(Some(&context), "BinPack", limit).unwrap();
            let result = graph.bin_pack_atomic_with_guard(&mut guard, false);
            if limit < 33 {
                assert_eq!(
                    result.unwrap_err().to_string(),
                    format!("TALA BinPack work exceeds limit {limit}")
                );
                assert_eq!(guard.used(), limit + 1);
                assert_eq!(geometry(&graph), before);
            } else {
                result.unwrap();
                assert_eq!(guard.used(), 33);
                assert_eq!(context.polls.get(), 21);
            }
        }
        for deadline in [false, true] {
            for cancel_at in 16..=21 {
                let mut graph = decoded(input);
                let before = geometry(&graph);
                let context = PollContext {
                    polls: Cell::new(0),
                    cancel_at,
                    deadline,
                };
                let mut guard = limits::WorkGuard::new(Some(&context), "BinPack", 100).unwrap();
                let error = graph
                    .bin_pack_atomic_with_guard(&mut guard, false)
                    .unwrap_err();
                assert_eq!(
                    error,
                    limits::Error::Context {
                        location: "BinPack".into(),
                        cause: if deadline {
                            limits::ContextError::DeadlineExceeded
                        } else {
                            limits::ContextError::Cancelled
                        },
                    }
                );
                assert_eq!(context.polls.get(), cancel_at);
                assert_eq!(geometry(&graph), before);
            }
        }
    }

    #[test]
    fn decoded_empty_container_entries_survive_and_mark_the_parent() {
        let empty = decoded(br#"{"nodes":[],"containers":{"0":[]}}"#);
        assert!(empty.containers.get_header(&None).is_some());
        assert!(empty.containers.get(&None).unwrap().is_empty());
        let graph = decoded(br#"{"nodes":[{"id":7}],"containers":{"0":[7],"7":[]}}"#);
        assert!(graph.containers.get_header(&None).is_some());
        assert!(graph.containers.get_header(&Some(NodeId(0))).is_some());
        assert!(graph.nodes[0].is_container);
        let mut guard = limits::WorkGuard::new(Some(&PackingContext), "BinPack", 100).unwrap();
        assert_eq!(
            graph
                .bin_pack_complete_routes(Some(NodeId(0)), &mut guard)
                .unwrap(),
            (false, vec![])
        );
        assert_eq!(guard.used(), 1);
    }
    #[test]
    fn decoding_a_cluster_filters_members_without_replacing_their_pointers() {
        let graph = decoded(br#"{"nodes":[{"id":9},{"id":7},{"id":2}],"edges":[{"id":-1,"from":7,"to":2}],"containers":{"0":[9,7,2]},"clusters":{"9":{"Nodes":[7,2],"Arrangement":"Row","DesiredArrangement":"Row"}}}"#);
        assert_eq!(
            graph
                .node_order
                .iter()
                .map(|id| graph.nodes[id.0 as usize].tala_id)
                .collect::<Vec<_>>(),
            vec![9]
        );
        assert!(!graph.cluster_is_active(&graph.clusters[0]));
        assert_eq!(
            graph.owner_edge_endpoint(EdgeId(0), false).unwrap(),
            Some(NodeId(1))
        );
        assert_eq!(
            graph.owner_edge_endpoint(EdgeId(0), true).unwrap(),
            Some(NodeId(2))
        );
        for root in [None, Some(NodeId(0))] {
            let mut guard = limits::WorkGuard::new(Some(&PackingContext), "BinPack", 100).unwrap();
            assert_eq!(
                graph.bin_pack_complete_routes(root, &mut guard).unwrap(),
                (false, vec![])
            );
            assert_eq!(guard.used(), if root.is_none() { 2 } else { 10 });
        }
    }
    #[test]
    fn inventory_self_loop_is_once_and_duplicate_graph_edge_is_rejected() {
        let mut graph = decoded(
            br#"{"nodes":[{"id":7}],"edges":[{"id":-1,"from":7,"to":7}],"containers":{"0":[7]}}"#,
        );
        let mut guard = limits::WorkGuard::new(Some(&PackingContext), "BinPack", 100).unwrap();
        assert_eq!(
            graph.bin_pack_complete_routes(None, &mut guard).unwrap(),
            (false, vec![])
        );
        assert_eq!(guard.used(), 3);
        let header =
            graph.allocate_owner_incident_header(&[Some(EdgeId(0)), Some(EdgeId(0))], 2, true);
        graph
            .set_owner_graph_edge_header(OwnerGraphId(0), header.unwrap())
            .unwrap();
        let mut guard = limits::WorkGuard::new(Some(&PackingContext), "BinPack", 100).unwrap();
        let error = graph
            .bin_pack_complete_routes(None, &mut guard)
            .unwrap_err();
        assert_eq!(
            error.error.to_string(),
            "layout invariant violated: BinPack graph edge inventory repeats affected edge -1"
        );
        assert_eq!(guard.used(), 3);
    }
    #[test]
    fn inventory_final_poll_preserves_completed_route_result_on_cancellation() {
        let graph = decoded(br#"{"nodes":[{"id":7},{"id":2}],"edges":[{"id":-1,"from":7,"to":2,"points":[{"x":0,"y":0},{"x":1,"y":1}]}],"containers":{"0":[7,2]}}"#);
        let context = PollContext {
            polls: Cell::new(0),
            cancel_at: 0,
            deadline: false,
        };
        let mut guard = limits::WorkGuard::new(Some(&context), "BinPack", 100).unwrap();
        assert_eq!(
            graph
                .bin_pack_complete_routes(Some(NodeId(0)), &mut guard)
                .unwrap(),
            (true, vec![EdgeId(0)])
        );
        let polls = context.polls.get();
        for deadline in [false, true] {
            let context = PollContext {
                polls: Cell::new(0),
                cancel_at: polls,
                deadline,
            };
            let mut guard = limits::WorkGuard::new(Some(&context), "BinPack", 100).unwrap();
            let failure = graph
                .bin_pack_complete_routes(Some(NodeId(0)), &mut guard)
                .unwrap_err();
            assert!(failure.routed);
            assert_eq!(failure.incident, vec![EdgeId(0)]);
            assert_eq!(
                failure.error,
                limits::Error::Context {
                    location: "BinPack".into(),
                    cause: if deadline {
                        limits::ContextError::DeadlineExceeded
                    } else {
                        limits::ContextError::Cancelled
                    }
                }
            );
        }
    }

    #[test]
    fn filtered_member_fixed_origin_uses_the_concrete_vessels_container() {
        let input = crate::internal::graphjson::graphjson_to_graph(br#"{"nodes":[{"id":10,"topLeft":{"x":0,"y":0}},{"id":9,"topLeft":{"x":0,"y":0}},{"id":1,"topLeft":{"x":30,"y":40},"fixedTopLeft":{"x":5,"y":5}},{"id":2,"topLeft":{"x":80,"y":90}},{"id":4,"topLeft":{"x":200,"y":300},"fixedTopLeft":{"x":10,"y":20}}],"containers":{"0":[10,9,4],"10":[1,2]},"clusters":{"9":{"Nodes":[1,2],"Arrangement":"Row","DesiredArrangement":"Row"}}}"#).unwrap();
        let mut graph = ArenaGraph::from_input(&input);
        // Prepared geometry precedes layout's fixed-position initialization.
        for (index, position) in input.initial_positions.iter().copied().enumerate() {
            graph.nodes[index].position = position;
        }
        let selected = [NodeId(2), NodeId(4)];
        assert_eq!(
            graph.bin_pack_bounding_box(&selected).unwrap().0,
            Point { x: 25.0, y: 35.0 }
        );
        assert_eq!(
            graph.fixed_node_corners(&selected, true).0,
            Some(Point { x: 25.0, y: 35.0 })
        );
        graph.initialize_shared_topology();
        assert_eq!(
            graph.fixed_node_corners(&selected, true).0,
            Some(Point { x: 25.0, y: 35.0 })
        );
    }
}



impl ArenaGraph {
    pub(super) fn packing_standalone_bad_state_checked(
        &self,
        node: PackingNodeRef,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<bool, limits::Error> {
        commit::standalone_bad_state(self, node, guard)
    }
}


impl ArenaGraph {
    pub(super) fn transaction_container_bad_state_checked(
        &self,
        node: NodeId,
        original_point: Option<Point>,
        overlaps: &BTreeSet<(NodeId, NodeId)>,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<bool, limits::Error> {
        self.transaction_container_bad_state_with_options_checked(
            node,
            original_point,
            overlaps,
            false,
            guard,
        )
    }
    pub(super) fn transaction_container_bad_state_with_options_checked(
        &self,
        node: NodeId,
        original_point: Option<Point>,
        overlaps: &BTreeSet<(NodeId, NodeId)>,
        ignore_container_escape: bool,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<bool, limits::Error> {
        commit::transaction_container_bad_state(
            self,
            PackingNodeRef::Node(node),
            original_point,
            overlaps,
            ignore_container_escape,
            guard,
        )
    }
}

/// transaction_snapshot.go278-316: the routed geometry option. The caller
/// retains the node geometry inventory and owns the enclosing Finish polls.
pub(super) struct CapturedEdgeGeometry {
    edges: BTreeMap<EdgeId, ArenaEdge>,
    routes: BTreeMap<EdgeId, route_points::RouteSnapshot>,
}
impl CapturedEdgeGeometry {
    pub(super) fn restore(&self, graph: &mut ArenaGraph) {
        // Capture records every edge in both maps, so the route loop below
        // replaces each header that restore_value would have set first.
        for (&edge, saved) in &self.edges {
            graph.edge_records[edge.0 as usize].restore_value_except_route(saved);
        }
        for (&edge, saved) in &self.routes {
            graph.edge_records[edge.0 as usize]
                .route
                .replace(saved.restore());
        }
    }
}
pub(super) fn capture_edge_geometry(
    graph: &ArenaGraph,
    nodes: &[PackingNodeRef],
    guard: &mut limits::WorkGuard<'_>,
    routed: bool,
) -> Result<CapturedEdgeGeometry, limits::Error> {
    let mut captured = CapturedEdgeGeometry {
        edges: BTreeMap::new(),
        routes: BTreeMap::new(),
    };
    if routed {
        let mut edges = BTreeSet::new();
        let primary = graph
            .owner_graph_edge_header(OwnerGraphId(0))
            .map(|header| Box::new(header.iter_slots()) as Box<dyn Iterator<Item = Option<EdgeId>>>)
            .unwrap_or_else(|_| Box::new(graph.edge_order.header().iter_slots()));
        for edge in primary {
            guard.step()?;
            let edge = edge.ok_or_else(|| {
                limits::Error::Invalid("TALA geometry snapshot contains a nil edge".into())
            })?;
            edges.insert(edge);
        }
        for node in nodes.iter().copied() {
            guard.step()?;
            let incidents: Box<dyn Iterator<Item = Option<EdgeId>>> = match node {
                PackingNodeRef::Node(node) => {
                    Box::new(graph.nodes[node.0 as usize].edges.header().iter_slots())
                }
                _ => Box::new(graph.packing_segment_incidents(node).into_iter()),
            };
            for edge in incidents {
                guard.step()?;
                if let Some(edge) = edge {
                    edges.insert(edge);
                }
            }
        }
        for edge in edges {
            guard.step()?;
            let live = &graph.edge_records[edge.0 as usize];
            // Source ranges over the earlier full-capacity header solely
            // to charge work. It does not index a later replacement route.
            // captureEdge then reads the live header after every poll.
            for _ in 0..live.route.capacity() {
                guard.step()?;
            }
            captured.routes.insert(edge, live.route.snapshot());
            captured.edges.insert(edge, live.clone());
        }
    }
    Ok(captured)
}
