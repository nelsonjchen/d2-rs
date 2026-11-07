// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Geometry and adapter projection journal shared by speculative trials.
//! Original GraphState.updateContext and geometry Transaction.Rollback restore
//! selected objects and fields; unrelated storage and node metadata stay live.

use super::*;

// Source node geometry, plus the adapter fields that encode its phase frame.
// Topology, constraints, styles and map headers remain live through rollback.
#[derive(Clone, Copy)]
pub(super) struct TrialNodeGeometry {
    position: Option<Point>,
    rect: Rect,
    unpositioned_scope_translation: Option<Point>,
    scope_translation_materialized: bool,
    transaction_anchor_tala_id: Option<i64>,
    transaction_anchor_offset: Option<Point>,
    transaction_position_was_nil: bool,
}
impl TrialNodeGeometry {
    fn capture(node: &ArenaNode) -> Self {
        Self {
            position: node.position,
            rect: node.rect,
            unpositioned_scope_translation: node.unpositioned_scope_translation,
            scope_translation_materialized: node.scope_translation_materialized,
            transaction_anchor_tala_id: node.transaction_anchor_tala_id,
            transaction_anchor_offset: node.transaction_anchor_offset,
            transaction_position_was_nil: node.transaction_position_was_nil,
        }
    }
    fn restore(self, node: &mut ArenaNode) {
        node.position = self.position;
        node.rect = self.rect;
        node.unpositioned_scope_translation = self.unpositioned_scope_translation;
        node.scope_translation_materialized = self.scope_translation_materialized;
        node.transaction_anchor_tala_id = self.transaction_anchor_tala_id;
        node.transaction_anchor_offset = self.transaction_anchor_offset;
        node.transaction_position_was_nil = self.transaction_position_was_nil;
    }
}
pub(super) fn trial_external_geometry(nodes: &[ArenaNode]) -> Vec<TrialNodeGeometry> {
    nodes.iter().map(TrialNodeGeometry::capture).collect()
}
pub(super) fn restore_trial_external_geometry(
    nodes: &mut [ArenaNode],
    saved: &[TrialNodeGeometry],
) {
    for (node, geometry) in nodes.iter_mut().zip(saved) {
        geometry.restore(node);
    }
}
pub(super) fn trial_external_map_geometry(
    nodes: &BTreeMap<i64, Vec<ArenaNode>>,
) -> BTreeMap<i64, Vec<TrialNodeGeometry>> {
    nodes
        .iter()
        .map(|(&key, nodes)| (key, trial_external_geometry(nodes)))
        .collect()
}
pub(super) fn restore_trial_external_map_geometry(
    nodes: &mut BTreeMap<i64, Vec<ArenaNode>>,
    saved: &BTreeMap<i64, Vec<TrialNodeGeometry>>,
) {
    // Geometry rollback does not restore graph map membership/header fields.
    for (key, geometry) in saved {
        if let Some(nodes) = nodes.get_mut(key) {
            restore_trial_external_geometry(nodes, geometry);
        }
    }
}

#[derive(Clone)]
pub(super) struct TrialGeometrySnapshot {
    commit_state: super::binpack_guard::GeometryCommitState,
    members: Option<super::shared_topology::OwnerGeometryMembersSnapshot>,
    legacy_node_order: Vec<NodeId>,
    tree_orientations: Vec<(super::owner_tree_map::OwnerTreeId, i64)>,
    nodes: Vec<(NodeId, TrialNodeGeometry)>,
    owner_points: Vec<(NodeId, super::shared_topology::OwnerNodePointSnapshot)>,
    cluster_policies: Vec<(ClusterArrangement, ClusterArrangement, f64)>,
    owner_cluster_policies: Vec<(super::owner_groups::OwnerGroupId, String, String, f64)>,
    cluster_vessel_sizes: Vec<Size>,
    transaction_external_containers: Vec<TrialNodeGeometry>,
    transaction_external_container_children: BTreeMap<i64, Vec<TrialNodeGeometry>>,
    transaction_external_aggregate_children: BTreeMap<i64, Vec<TrialNodeGeometry>>,
    // Tracked clones keep their generations, so a restored snapshot also
    // restores the stamps its contents had.
    sized_adjacent_overrides: Tracked<BTreeMap<(NodeId, EdgeId), ProjectedAdjacent>>,
    sized_cluster_distance_boxes: Tracked<BTreeMap<(NodeId, i64), ProjectedClusterDistance>>,
    sized_edge_abductions: Tracked<Vec<SizedEdgeAbduction>>,
    sized_edge_abductions_present: bool,
    sized_projected_obstructions: Tracked<BTreeMap<(NodeId, EdgeId), SharedProjections>>,
    sized_projected_obstruction_sets:
        Tracked<BTreeMap<(NodeId, EdgeId), OrderedObstructionInventory>>,
    sized_collapsed_symmetry_neighbors: Tracked<BTreeMap<i64, Vec<ProjectedAdjacent>>>,
    pending_cluster_vessel_positions: BTreeMap<usize, Point>,
}

impl TrialGeometrySnapshot {
    pub(super) fn validate_commit(
        &self,
        graph: &ArenaGraph,
        overlaps: &BTreeSet<(NodeId, NodeId)>,
        exact_overlaps: &BTreeSet<(NodeId, NodeId)>,
        context: &dyn limits::Context,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<bool, limits::Error> {
        self.commit_state.validate_with_context_options(
            graph,
            overlaps,
            exact_overlaps,
            false,
            Some(context),
            guard,
        )
    }
    pub(super) fn original_size(&self, node: NodeId) -> Option<Size> {
        self.nodes
            .iter()
            .find(|(id, _)| *id == node)
            .map(|(_, geometry)| geometry.rect.size)
    }
    pub(super) fn original_position(&self, node: NodeId) -> Option<Point> {
        if let Some((_, point)) = self.owner_points.iter().find(|(id, _)| *id == node) {
            return point.captured_value();
        }
        self.nodes
            .iter()
            .find(|(id, _)| *id == node)
            .and_then(|(_, geometry)| geometry.position)
    }
}

impl ArenaGraph {
    /// transaction.go recordDescendants: actual canonical identities, every
    /// Containers key/value, and retained group Vessel/Container/Nodes. Never
    /// follow Node.Graph/Container links or deduplicate by the numeric Tala ID.
    fn trial_geometry_node_ids(&self) -> Vec<NodeId> {
        use super::owner_groups::OwnerGroupKind;
        use super::shared_topology::OwnerGraphId;
        let mut pending = self
            .owner_graph_member_header(OwnerGraphId(0))
            .map(|header| {
                header
                    .visible_slots()
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_else(|_| self.graph_node_order());
        for (container, children) in self.containers.iter() {
            pending.extend(container);
            pending.extend(children);
        }
        for (index, cluster) in self.clusters.iter().enumerate() {
            pending.extend(cluster.members.iter().copied());
            pending.extend(self.topology_vessel_node(TopologyVessel::Cluster(index)));
            pending.extend(
                cluster
                    .members
                    .first()
                    .and_then(|node| self.nodes[node.0 as usize].container),
            );
        }
        for (index, sequence) in self.sequences.iter().enumerate() {
            pending.extend(sequence.members.iter().copied());
            pending.extend(sequence.container);
            pending.extend(self.topology_vessel_node(TopologyVessel::Sequence(index)));
        }
        for kind in [OwnerGroupKind::Cluster, OwnerGroupKind::Sequence] {
            if let Ok(map) = self.owner_graph_group_map(OwnerGraphId(0), kind) {
                for group in self
                    .owner_group_map_values(&map)
                    .expect("geometry group map")
                    .flatten()
                {
                    let fields = self
                        .owner_group_fields(group)
                        .expect("geometry group fields");
                    pending.extend(fields.vessel);
                    pending.extend(fields.container);
                    pending.extend(fields.nodes.visible_slots().into_iter().flatten());
                }
            }
        }
        let mut selected = vec![false; self.nodes.len()];
        while let Some(node) = pending.pop() {
            assert!(
                (node.0 as usize) < self.nodes.len(),
                "geometry snapshot canonical node"
            );
            if std::mem::replace(&mut selected[node.0 as usize], true) {
                continue;
            }
            if let Some(children) = self.containers.get_header(&Some(node)) {
                pending.extend(children.visible_slots().into_iter().flatten());
            }
        }
        // Ascending canonical ids, as the ordered set produced.
        selected
            .iter()
            .enumerate()
            .filter(|&(_, &seen)| seen)
            .map(|(index, _)| NodeId(index as u32))
            .collect()
    }

    pub(super) fn trial_geometry_snapshot(&self) -> TrialGeometrySnapshot {
        #[cfg(feature = "diagnostic-traces")]
        crate::internal::call_counts::count!(GeometrySnapshotUnchecked);
        let selected = self.trial_geometry_node_ids();
        let mut owner_cluster_policies = Vec::new();
        if let Ok(map) = self.owner_graph_group_map(
            super::shared_topology::OwnerGraphId(0),
            super::owner_groups::OwnerGroupKind::Cluster,
        ) {
            let mut seen = BTreeSet::new();
            for group in self
                .owner_group_map_values(&map)
                .expect("geometry cluster map")
                .flatten()
            {
                if !seen.insert(group) {
                    continue;
                }
                let policy = self
                    .owner_cluster_policy(group)
                    .expect("geometry cluster policy");
                owner_cluster_policies.push((
                    group,
                    policy.arrangement,
                    policy.desired_arrangement,
                    policy.padding,
                ));
            }
        }
        // Source captures only nonnil NodeToTree values, retaining the actual
        // Tree allocation across later map replacement. Graph.Trees is unrelated.
        let receiver = super::shared_topology::OwnerGraphId(0);
        let mut tree_orientations = BTreeMap::new();
        if let Ok(map) = self.owner_graph_node_to_tree_map(receiver) {
            for key in map.live_keys() {
                if let Some(Some(tree)) = self
                    .owner_graph_node_to_tree_entry(receiver, key)
                    .expect("geometry NodeToTree entry")
                {
                    tree_orientations.insert(
                        tree,
                        self.owner_tree(tree)
                            .expect("geometry Tree allocation")
                            .fields()
                            .orientation,
                    );
                }
            }
        }
        let mut snapshot = self.trial_geometry_adapter_snapshot();
        snapshot.nodes = selected
            .iter()
            .map(|&node| {
                (
                    node,
                    TrialNodeGeometry::capture(&self.nodes[node.0 as usize]),
                )
            })
            .collect();
        for &node in &selected {
            snapshot
                .commit_state
                .record_geometry(self, super::binpack_traversal::PackingNodeRef::Node(node));
        }
        snapshot.owner_points = selected
            .iter()
            .copied()
            .filter_map(|node| {
                self.owner_node_point(node)
                    .ok()
                    .map(|point| (node, point.snapshot()))
            })
            .collect();
        snapshot.owner_cluster_policies = owner_cluster_policies;
        snapshot.tree_orientations = tree_orientations.into_iter().collect();
        snapshot
    }

    fn trial_geometry_adapter_snapshot(&self) -> TrialGeometrySnapshot {
        TrialGeometrySnapshot {
            commit_state: super::binpack_guard::GeometryCommitState::begin_capture(self),
            members: self
                .geometry_members_snapshot(super::shared_topology::OwnerGraphId(0))
                .ok(),
            legacy_node_order: self.node_order.clone(),
            tree_orientations: Vec::new(),
            nodes: Vec::new(),
            // transaction.go Rollback restores TopLeft's allocation as well
            // as its value. Inline ArenaNode copies cannot restore aliases.
            owner_points: Vec::new(),
            // Geometry rollback saves these three policy fields, not the
            // complete Cluster object (in particular, not FixedSize).
            cluster_policies: self
                .clusters
                .iter()
                .map(|cluster| {
                    (
                        cluster.arrangement,
                        cluster.desired_arrangement,
                        cluster.padding,
                    )
                })
                .collect(),
            owner_cluster_policies: Vec::new(),
            cluster_vessel_sizes: self.cluster_vessel_sizes.clone(),
            transaction_external_containers: trial_external_geometry(
                &self.transaction_external_containers,
            ),
            transaction_external_container_children: trial_external_map_geometry(
                &self.transaction_external_container_children,
            ),
            transaction_external_aggregate_children: trial_external_map_geometry(
                &self.transaction_external_aggregate_children,
            ),
            sized_adjacent_overrides: self.sized_adjacent_overrides.clone(),
            sized_cluster_distance_boxes: self.sized_cluster_distance_boxes.clone(),
            sized_edge_abductions: self.sized_edge_abductions.clone(),
            sized_edge_abductions_present: self.sized_edge_abductions_present,
            sized_projected_obstructions: self.sized_projected_obstructions.clone(),
            sized_projected_obstruction_sets: self.sized_projected_obstruction_sets.clone(),
            sized_collapsed_symmetry_neighbors: self.sized_collapsed_symmetry_neighbors.clone(),
            pending_cluster_vessel_positions: self.pending_cluster_vessel_positions.clone(),
        }
    }

    /// Build a new immutable checkpoint while the source traversal charges it.
    /// Errors discard the partial snapshot; the caller owns prior-state rollback.
    pub(super) fn trial_geometry_snapshot_with_guard(
        &self,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<TrialGeometrySnapshot, limits::Error> {
        use super::gap_transaction_work::{GeometryCaptureEvent, GeometrySnapshotObject};
        let mut captured: Option<TrialGeometrySnapshot> = None;
        self.geometry_transaction_capture_walk(guard, &mut |graph, event| {
            if let GeometryCaptureEvent::Begin = event {
                // originalNodesRef and its visible copy precede the first poll.
                captured = Some(graph.trial_geometry_adapter_snapshot());
                return;
            }
            let snapshot = captured.as_mut().expect("capture begins after preflight");
            if let GeometryCaptureEvent::Node(object) = event {
                use super::binpack_traversal::PackingNodeRef;
                let node = match object {
                    GeometrySnapshotObject::Node(node) => PackingNodeRef::Node(node),
                    GeometrySnapshotObject::Cluster(index) => PackingNodeRef::Cluster(index),
                    GeometrySnapshotObject::Sequence(index) => PackingNodeRef::Sequence(index),
                };
                snapshot.commit_state.record_geometry(graph, node);
            }
            match event {
                GeometryCaptureEvent::Node(GeometrySnapshotObject::Node(node)) => {
                    snapshot.nodes.push((
                        node,
                        TrialNodeGeometry::capture(&graph.nodes[node.0 as usize]),
                    ));
                    if let Ok(point) = graph.owner_node_point(node) {
                        snapshot.owner_points.push((node, point.snapshot()));
                    }
                }
                GeometryCaptureEvent::Cluster(group) => {
                    let policy = graph
                        .owner_cluster_policy(group)
                        .expect("captured cluster policy");
                    snapshot.owner_cluster_policies.push((
                        group,
                        policy.arrangement,
                        policy.desired_arrangement,
                        policy.padding,
                    ));
                }
                GeometryCaptureEvent::Tree(tree) => {
                    snapshot.tree_orientations.push((
                        tree,
                        graph
                            .owner_tree(tree)
                            .expect("captured Tree")
                            .fields()
                            .orientation,
                    ));
                }
                GeometryCaptureEvent::Node(_) => {} // compact-only projection fields are retained by the adapter base
                GeometryCaptureEvent::Begin => unreachable!(),
                GeometryCaptureEvent::End => {}
            }
        })?;
        Ok(captured.expect("successful geometry capture begins"))
    }

    /// A cluster optimization transaction changes geometry on existing source
    /// pointers. A numerical candidate must not replace the owning arena and
    /// detach geo.Point handles retained by the caller.
    pub(super) fn publish_cluster_optimization_candidate(&mut self, candidate: Self) {
        assert_eq!(
            self.nodes.len(),
            candidate.nodes.len(),
            "cluster geometry cannot allocate source nodes"
        );
        let mut geometry = candidate.trial_geometry_snapshot();
        for (node, _) in &geometry.nodes {
            Pipeline::publish_induced_owner_point(self, *node, candidate.position(*node));
        }
        // Keep the actual owner point bindings and Graph.Nodes/Containers
        // headers. The clone's snapshots refer to detached allocations.
        geometry.owner_points.clear();
        geometry.members = None;
        self.restore_trial_geometry_snapshot(geometry);
        self.cell_size = candidate.cell_size;
    }

    pub(super) fn restore_trial_geometry_snapshot(&mut self, snapshot: TrialGeometrySnapshot) {
        for (node, geometry) in snapshot.nodes {
            geometry.restore(&mut self.nodes[node.0 as usize]);
        }
        for (node, point) in &snapshot.owner_points {
            self.restore_owner_node_point(*node, point)
                .expect("gap point rollback");
        }
        for (cluster, (arrangement, desired_arrangement, padding)) in
            self.clusters.iter_mut().zip(snapshot.cluster_policies)
        {
            cluster.arrangement = arrangement;
            cluster.desired_arrangement = desired_arrangement;
            cluster.padding = padding;
        }
        for (group, arrangement, desired, padding) in snapshot.owner_cluster_policies {
            self.set_owner_cluster_orientation_policy(group, arrangement, desired, padding)
                .expect("geometry cluster policy rollback");
        }
        for (tree, orientation) in snapshot.tree_orientations {
            self.set_owner_tree_orientation(tree, orientation)
                .expect("geometry Tree rollback");
        }
        if let Some(members) = snapshot.members {
            self.restore_geometry_members(members)
                .expect("geometry member rollback");
        }
        // Ordinary geometry trials leave member order unchanged. Preserve the
        // valid derived cache in that common path; invalidating it unconditionally
        // sends every scoring read through repeated aggregate scans.
        if self.node_order != snapshot.legacy_node_order {
            self.node_order = snapshot.legacy_node_order;
            self.invalidate_node_order_membership();
            self.refresh_node_order_membership();
        }
        self.cluster_vessel_sizes = snapshot.cluster_vessel_sizes;
        restore_trial_external_geometry(
            &mut self.transaction_external_containers,
            &snapshot.transaction_external_containers,
        );
        restore_trial_external_map_geometry(
            &mut self.transaction_external_container_children,
            &snapshot.transaction_external_container_children,
        );
        restore_trial_external_map_geometry(
            &mut self.transaction_external_aggregate_children,
            &snapshot.transaction_external_aggregate_children,
        );
        self.sized_adjacent_overrides = snapshot.sized_adjacent_overrides;
        self.sized_cluster_distance_boxes = snapshot.sized_cluster_distance_boxes;
        self.sized_edge_abductions = snapshot.sized_edge_abductions;
        self.sized_edge_abductions_present = snapshot.sized_edge_abductions_present;
        self.sized_projected_obstructions = snapshot.sized_projected_obstructions;
        self.sized_projected_obstruction_sets = snapshot.sized_projected_obstruction_sets;
        self.sized_collapsed_symmetry_neighbors = snapshot.sized_collapsed_symmetry_neighbors;
        self.pending_cluster_vessel_positions = snapshot.pending_cluster_vessel_positions;
    }
}
