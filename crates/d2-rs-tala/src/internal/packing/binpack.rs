// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Packing of disconnected components and completed container scopes.

use super::*;
use d2_rs_core::stable_sort::StableSort;

#[cfg(feature = "diagnostic-traces")]
fn trace_binpack_event(
    graph: &ArenaGraph,
    phase: &str,
    root: Option<NodeId>,
    groups: &[Vec<NodeId>],
    cross_container_count: usize,
    point: Option<Point>,
    score: f64,
) {
    if !crate::internal::trace_env_enabled("TALA_TRACE_PACKING") {
        return;
    }
    let group_ids = groups
        .iter()
        .enumerate()
        .map(|(index, group)| graph.trace_binpack_group_ids(group, index >= cross_container_count))
        .collect::<Vec<_>>();
    let mut event = serde_json::json!({
        "event": "packing",
        "phase": phase,
        "root": root
            .map(|node| graph.nodes[node.0 as usize].external_id.clone())
            .unwrap_or_default(),
        "groups": group_ids,
        "score_bits": format!("0x{:016x}", score.to_bits()),
    });
    if let Some(point) = point {
        event["x_bits"] = serde_json::Value::String(format!("0x{:016x}", point.x.to_bits()));
        event["y_bits"] = serde_json::Value::String(format!("0x{:016x}", point.y.to_bits()));
    }
    eprintln!("{event}");
}

#[cfg(not(feature = "diagnostic-traces"))]
#[inline(always)]
fn trace_binpack_event(
    _graph: &ArenaGraph,
    _phase: &str,
    _root: Option<NodeId>,
    _groups: &[Vec<NodeId>],
    _cross_container_count: usize,
    _point: Option<Point>,
    _score: f64,
) {
}

impl ArenaGraph {
    /// Replays Go's aggregate pointer lifecycle for diagnostic packing traces.
    ///
    /// Go gives an active cluster/sequence both a temporary vessel pointer and
    /// the original member pointers. The stable arena stores the vessel as
    /// the first member's carrier, so its ordinary BFS cannot represent the
    /// duplicate carrier occurrence or the queue position of hidden members.
    /// Keep that distinction in the trace-only projection; release packing
    /// continues to operate on stable NodeIds.
    #[cfg(feature = "diagnostic-traces")]
    fn trace_binpack_virtual_group_ids(
        &self,
        group: &[NodeId],
        include_nears: bool,
    ) -> Option<Vec<String>> {
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        enum TraceNode {
            Member(NodeId),
            Vessel(NodeId),
        }

        let group_set = group.iter().copied().collect::<BTreeSet<_>>();
        let mut member_owner = BTreeMap::<NodeId, NodeId>::new();
        let mut members_by_owner = BTreeMap::<NodeId, Vec<NodeId>>::new();
        for cluster in &self.clusters {
            let Some(&owner) = cluster.members.first() else {
                continue;
            };
            if !self.active_aggregate_members(owner).is_some() {
                continue;
            }
            members_by_owner.insert(owner, cluster.members.clone());
            for member in cluster.members.iter().copied() {
                member_owner.insert(member, owner);
            }
        }
        for sequence in &self.sequences {
            let Some(&owner) = sequence.members.first() else {
                continue;
            };
            if !self.active_aggregate_members(owner).is_some() {
                continue;
            }
            members_by_owner.insert(owner, sequence.members.clone());
            for member in sequence.members.iter().copied() {
                member_owner.insert(member, owner);
            }
        }
        if !group_set.iter().any(|node| member_owner.contains_key(node)) {
            return None;
        }

        let key_for_node = |node: NodeId| {
            member_owner
                .get(&node)
                .copied()
                .map_or(TraceNode::Member(node), TraceNode::Vessel)
        };
        let key_for_graph_node = |node: NodeId| {
            if member_owner.get(&node).copied() == Some(node) {
                TraceNode::Vessel(node)
            } else {
                TraceNode::Member(node)
            }
        };
        let allowed = |key: TraceNode| match key {
            TraceNode::Member(node) => group_set.contains(&node),
            TraceNode::Vessel(owner) => {
                group_set.contains(&owner)
                    || members_by_owner.get(&owner).is_some_and(|members| {
                        members.iter().any(|member| group_set.contains(member))
                    })
            }
        };
        let container_of = |key: TraceNode| match key {
            TraceNode::Vessel(owner) => self.active_node_container(owner),
            // AddCluster/AddSequence clear the original member's Container;
            // ancestry still reaches the temporary vessel through Cluster or
            // Sequence, handled separately by parent_of below.
            TraceNode::Member(node) if member_owner.contains_key(&node) => None,
            TraceNode::Member(node) => self.active_node_container(node),
        };
        let parent_of = |key: TraceNode| match key {
            TraceNode::Vessel(owner) => container_of(TraceNode::Vessel(owner)).map(key_for_node),
            TraceNode::Member(node) => member_owner
                .get(&node)
                .copied()
                .map(TraceNode::Vessel)
                .or_else(|| self.nodes[node.0 as usize].container.map(key_for_node)),
        };
        let is_descendant = |descendant: TraceNode, ancestor: TraceNode| {
            let mut current = descendant;
            let mut seen = BTreeSet::new();
            loop {
                if current == ancestor {
                    return true;
                }
                if !seen.insert(current) {
                    return false;
                }
                let Some(parent) = parent_of(current) else {
                    return false;
                };
                current = parent;
            }
        };

        let first = group.first().copied()?;
        let initial = if member_owner.get(&first).copied() == Some(first) {
            TraceNode::Vessel(first)
        } else {
            TraceNode::Member(first)
        };
        let mut visited = BTreeSet::from([initial]);
        let mut queue = VecDeque::from([initial]);
        let mut ordered = Vec::new();
        let enqueue =
            |key: TraceNode, queue: &mut VecDeque<TraceNode>, visited: &mut BTreeSet<TraceNode>| {
                if allowed(key) && visited.insert(key) {
                    queue.push_back(key);
                }
            };
        while let Some(current) = queue.pop_front() {
            ordered.push(current);
            let current_node = match current {
                TraceNode::Member(node) | TraceNode::Vessel(node) => node,
            };

            // Cluster members have empty Node.Edges after Go's edge abduction.
            // A vessel retains the aggregate edge slice; ordinary nodes keep
            // their endpoint-local order after tree reconnection and ordered
            // hierarchy endpoint restoration. Do not sort by serialized IDs.
            let edges = match current {
                TraceNode::Vessel(node) => {
                    self.active_edge_ids(node).into_iter().collect::<Vec<_>>()
                }
                TraceNode::Member(node) if member_owner.contains_key(&node) => Vec::new(),
                TraceNode::Member(node) => {
                    self.active_edge_ids(node).into_iter().collect::<Vec<_>>()
                }
            };
            if crate::internal::trace_env_value("TALA_TRACE_PACKING_BFS_NODE").is_some_and(
                |targets| {
                    targets
                        .split(',')
                        .any(|target| target == self.nodes[current_node.0 as usize].external_id)
                },
            ) {
                eprintln!(
                    "PACKING_BFS_NODE_RUST node={} first={} include_nears={} edges={:?} published={:?}",
                    self.nodes[current_node.0 as usize].external_id,
                    self.nodes[first.0 as usize].external_id,
                    include_nears,
                    edges
                        .iter()
                        .map(|edge| (
                            edge.0,
                            &self.nodes[self.active_adjacent(current_node, *edge).0 as usize]
                                .external_id
                        ))
                        .collect::<Vec<_>>(),
                    self.published_tree_node_edge_order
                        .iter()
                        .map(|edge| edge.0)
                        .collect::<Vec<_>>()
                );
            }
            for edge in edges {
                enqueue(
                    key_for_node(self.active_adjacent(current_node, edge)),
                    &mut queue,
                    &mut visited,
                );
            }

            if include_nears {
                let mut nears = match current {
                    TraceNode::Vessel(owner) => members_by_owner
                        .get(&owner)
                        .into_iter()
                        .flatten()
                        .flat_map(|member| self.nodes[member.0 as usize].nears.keys().into_iter())
                        .collect::<Vec<_>>(),
                    TraceNode::Member(node) => self.nodes[node.0 as usize].nears.keys(),
                };
                nears.stable_sort_by_key(|node| self.nodes[node.0 as usize].tala_id);
                nears.dedup();
                let current_container = container_of(current);
                for near in nears {
                    let near_key = key_for_node(near);
                    if container_of(near_key) == current_container {
                        enqueue(near_key, &mut queue, &mut visited);
                    }
                }
            }

            match current {
                TraceNode::Vessel(owner) => {
                    if let Some(members) = members_by_owner.get(&owner) {
                        for member in members.iter().copied() {
                            enqueue(TraceNode::Member(member), &mut queue, &mut visited);
                        }
                    }
                }
                TraceNode::Member(node) => {
                    if let Some(sequence_index) = self.nodes[node.0 as usize].sequence {
                        for member in self.sequences[sequence_index].members.iter().copied() {
                            enqueue(TraceNode::Member(member), &mut queue, &mut visited);
                        }
                    }
                }
            }

            for other in self.graph_node_order() {
                let other_key = key_for_graph_node(other);
                if visited.contains(&other_key) {
                    continue;
                }
                if is_descendant(current, other_key) || is_descendant(other_key, current) {
                    enqueue(other_key, &mut queue, &mut visited);
                }
            }
        }

        let output = ordered
            .into_iter()
            .map(|key| match key {
                TraceNode::Vessel(owner) => {
                    if let Some(cluster_index) = self.active_cluster_index(owner) {
                        let cluster = &self.clusters[cluster_index];
                        let arrangement = match cluster.arrangement {
                            ClusterArrangement::Row => "Row",
                            ClusterArrangement::Column => "Column",
                        };
                        let members = cluster
                            .members
                            .iter()
                            .map(|member| self.nodes[member.0 as usize].external_id.as_str())
                            .collect::<Vec<_>>()
                            .join(", ");
                        format!("Cluster vessel of: [{members}]; Arrangement: {arrangement}")
                    } else if let Some(sequence_index) = self.active_sequence_index(owner) {
                        let sequence = &self.sequences[sequence_index];
                        let members = sequence
                            .members
                            .iter()
                            .map(|member| self.nodes[member.0 as usize].external_id.as_str())
                            .collect::<Vec<_>>()
                            .join(", ");
                        format!("Sequence vessel of: [{members}]")
                    } else {
                        self.nodes[owner.0 as usize].external_id.clone()
                    }
                }
                TraceNode::Member(node) => self.nodes[node.0 as usize].external_id.clone(),
            })
            .collect::<Vec<_>>();
        Some(output)
    }

    #[cfg(not(feature = "diagnostic-traces"))]
    #[inline(always)]
    fn trace_binpack_virtual_group_ids(
        &self,
        _group: &[NodeId],
        _include_nears: bool,
    ) -> Option<Vec<String>> {
        None
    }

    /// Reconstructs the pointer inventory that Go's BinPack trace observes
    /// when a temporary cluster/sequence vessel replaces stable members. The
    /// release arena keeps the first member as the aggregate carrier. Go has
    /// a distinct vessel pointer, so its trace emits a vessel at the carrier's
    /// dequeue point and emits that carrier with the other members when the
    /// queued member pointers are reached. This projection is diagnostic-only;
    /// release packing continues to use the stable arena representation.
    fn trace_binpack_group_ids(&self, group: &[NodeId], include_nears: bool) -> Vec<String> {
        if let Some(virtual_ids) = self.trace_binpack_virtual_group_ids(group, include_nears) {
            return virtual_ids;
        }
        if crate::internal::trace_env_enabled("TALA_TRACE_BINPACK_GROUPS") {
            eprintln!(
                "BINPACK_RAW_RUST {:?}",
                group
                    .iter()
                    .map(|node| self.nodes[node.0 as usize].external_id.as_str())
                    .collect::<Vec<_>>()
            );
        }
        let mut ids = Vec::new();
        let mut deferred_carriers = BTreeMap::<usize, NodeId>::new();
        let mut emitted_carriers = BTreeSet::new();
        let mut emitted_aggregates = BTreeSet::new();
        for &node in group {
            let Some(cluster_index) = self.active_cluster_index(node) else {
                if let Some(sequence_index) = self.active_sequence_index(node) {
                    let sequence = &self.sequences[sequence_index];
                    let Some(&carrier) = sequence.members.first() else {
                        continue;
                    };
                    if node == carrier {
                        if emitted_aggregates.insert((1usize << 31) | sequence_index) {
                            let members = sequence
                                .members
                                .iter()
                                .map(|member| self.nodes[member.0 as usize].external_id.as_str())
                                .collect::<Vec<_>>()
                                .join(", ");
                            ids.push(format!("Sequence vessel of: [{members}]"));
                            deferred_carriers.insert((1usize << 31) | sequence_index, carrier);
                        }
                        continue;
                    }
                    if deferred_carriers
                        .remove(&((1usize << 31) | sequence_index))
                        .is_some()
                        && emitted_carriers.insert((1usize << 31) | sequence_index)
                    {
                        ids.push(self.nodes[carrier.0 as usize].external_id.clone());
                    }
                    ids.push(self.nodes[node.0 as usize].external_id.clone());
                    continue;
                }
                ids.push(self.nodes[node.0 as usize].external_id.clone());
                continue;
            };
            let cluster = &self.clusters[cluster_index];
            let Some(&carrier) = cluster.members.first() else {
                continue;
            };
            if node == carrier {
                if !emitted_aggregates.insert(cluster_index) {
                    continue;
                }
                let arrangement = match cluster.arrangement {
                    ClusterArrangement::Row => "Row",
                    ClusterArrangement::Column => "Column",
                };
                let members = cluster
                    .members
                    .iter()
                    .map(|member| self.nodes[member.0 as usize].external_id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                ids.push(format!(
                    "Cluster vessel of: [{members}]; Arrangement: {arrangement}"
                ));
                deferred_carriers.insert(cluster_index, carrier);
                continue;
            }
            if deferred_carriers.remove(&cluster_index).is_some()
                && emitted_carriers.insert(cluster_index)
            {
                ids.push(self.nodes[carrier.0 as usize].external_id.clone());
            }
            ids.push(self.nodes[node.0 as usize].external_id.clone());
        }
        // A one-member aggregate has no later member pointer at which to
        // flush the carrier. Preserve Go's queued pointer at group end.
        for (aggregate, carrier) in deferred_carriers {
            if emitted_carriers.insert(aggregate) {
                ids.push(self.nodes[carrier.0 as usize].external_id.clone());
            }
        }
        ids
    }

    /// Returns direct children with active sequence and cluster members replaced
    /// by their temporary vessels.
    pub(super) fn bin_pack_direct_children(&self, scope: Option<NodeId>) -> Vec<NodeId> {
        let mut direct = Vec::new();
        let mut seen = BTreeSet::new();
        // Graph.BinPack iterates the current Graph.Containers slice. During
        // aggregate preprocessing AddSequences/AddClusters remove member
        // pointers and append their vessel, so the raw stable-arena map is
        // stale while an aggregate is active. Without an active aggregate,
        // the child vector itself is authoritative: it preserves the input
        // insertion order used by Go's BinPack, whereas node_order follows
        // edge/discovery order and can permute independent siblings.
        let raw = self.containers.get(&scope).into_iter().flatten();
        let has_active_aggregate = raw.clone().any(|node| {
            self.active_aggregate_owner(node) != node || self.active_node_is_aggregate(node)
        });
        let ordered = if has_active_aggregate {
            self.container_node_order(scope)
        } else {
            self.containers.get(&scope).into_iter().flatten().collect()
        };
        for node in ordered {
            if crate::internal::trace_env_enabled("TALA_TRACE_BINPACK_GROUPS") {
                eprintln!(
                    "BINPACK_ACTIVE_RUST scope={:?} node={} owner={} container={:?} active_container={:?} cluster={:?}",
                    scope.map(|id| self.nodes[id.0 as usize].tala_id),
                    self.nodes[node.0 as usize].tala_id,
                    self.nodes[self.active_aggregate_owner(node).0 as usize].tala_id,
                    self.nodes[node.0 as usize]
                        .container
                        .map(|id| self.nodes[id.0 as usize].tala_id),
                    self.active_node_container(node)
                        .map(|id| self.nodes[id.0 as usize].tala_id),
                    self.active_cluster_index(node),
                );
            }
            if self.active_node_container(node) != scope {
                continue;
            }
            let owner = self.active_aggregate_owner(node);
            if seen.insert(owner) {
                direct.push(owner);
            }
        }
        direct
    }

    /// A compact arena carrier stands for a temporary vessel only while that
    /// vessel has no separate current node. Retained members and explicitly
    /// decoded vessel nodes keep their own body, labels and loop extents.
    pub(super) fn bin_pack_node_is_vessel_carrier(&self, node: NodeId) -> bool {
        if !self.active_node_is_aggregate(node) || self.active_aggregate_owner(node) != node {
            return false;
        }
        let vessel_id = self.active_node_tala_id(node);
        if self.node_order_membership_valid
            && let Some(carriers) = &self.node_order_tala_carriers
        {
            return carriers
                .get(&vessel_id)
                .is_none_or(|&(first, distinct)| !distinct && first == node);
        }
        !self
            .node_order
            .iter()
            .any(|&current| current != node && self.nodes[current.0 as usize].tala_id == vessel_id)
    }

    /// Bounds of the active graph nodes represented by a BinPack group.
    /// Temporary sequence/cluster vessels have no member labels or shape
    /// modifiers of their own; their box is simply the vessel dimensions.
    /// Ordinary nodes retain the recovered fixed-bounds behavior.
    pub(super) fn bin_pack_active_bounds(&self, nodes: &[NodeId]) -> Option<(Point, Point)> {
        let ordinary = nodes
            .iter()
            .copied()
            .filter(|node| !self.bin_pack_node_is_vessel_carrier(*node))
            .collect::<Vec<_>>();
        let mut bounds = if ordinary.is_empty() {
            None
        } else {
            self.fixed_node_bounds(&ordinary)
        };
        for node in nodes.iter().copied() {
            if crate::internal::trace_env_enabled("TALA_TRACE_BINPACK_NODE_CANDIDATES") {
                eprintln!(
                    "BINPACK_NODE_CAND_RUST node={} active_pos={:?} active_size={:?} active_container={:?} fixed={:?} active_owner={} aggregate={} cluster={:?}",
                    self.nodes[node.0 as usize].tala_id,
                    self.active_node_position(node),
                    self.active_node_size(node),
                    self.active_node_container(node)
                        .map(|id| self.nodes[id.0 as usize].tala_id),
                    self.nodes[node.0 as usize].fixed_top_left,
                    self.nodes[self.active_aggregate_owner(node).0 as usize].tala_id,
                    self.bin_pack_node_is_vessel_carrier(node),
                    self.active_cluster_index(node),
                );
            }
            if !self.bin_pack_node_is_vessel_carrier(node) {
                continue;
            }
            let (Some(position), size) =
                (self.active_node_position(node), self.active_node_size(node))
            else {
                continue;
            };
            let top_left = position;
            let bottom_right = Point {
                x: position.x + size.width,
                y: position.y + size.height,
            };
            bounds = Some(match bounds {
                None => (top_left, bottom_right),
                Some((mut low, mut high)) => {
                    low.x = low.x.min(top_left.x);
                    low.y = low.y.min(top_left.y);
                    high.x = high.x.max(bottom_right.x);
                    high.y = high.y.max(bottom_right.y);
                    (low, high)
                }
            });
            // Go keeps a distinct temporary vessel pointer alongside every
            // retained cluster/sequence member. The stable arena uses the
            // first member as the vessel carrier, so add the raw member boxes
            // explicitly to preserve the same packing bounds.
            if self.active_aggregate_owner(node) == node
                && let Some(members) = self.active_aggregate_members(node)
            {
                for member in members.iter().copied() {
                    let Some(member_position) = self.position(member) else {
                        continue;
                    };
                    let member_size = self.nodes[member.0 as usize].rect.size;
                    let member_top_left = member_position;
                    let member_bottom_right = Point {
                        x: member_position.x + member_size.width,
                        y: member_position.y + member_size.height,
                    };
                    bounds = Some(match bounds {
                        None => (member_top_left, member_bottom_right),
                        Some((mut low, mut high)) => {
                            low.x = low.x.min(member_top_left.x);
                            low.y = low.y.min(member_top_left.y);
                            high.x = high.x.max(member_bottom_right.x);
                            high.y = high.y.max(member_bottom_right.y);
                            (low, high)
                        }
                    });
                }
            }
        }
        bounds
    }

    /// Bounding-box counterpart used by Go's candidate inventory. Unlike
    /// `bin_pack_active_bounds`, this intentionally leaves fixed-origin nodes
    /// at their visible coordinates.
    fn bin_pack_unanchored_bounds(&self, nodes: &[NodeId]) -> Option<(Point, Point)> {
        let ordinary = nodes
            .iter()
            .copied()
            .filter(|node| !self.bin_pack_node_is_vessel_carrier(*node))
            .collect::<Vec<_>>();
        let mut bounds = if ordinary.is_empty() {
            None
        } else {
            // graphbounds.BoundingBox uses the same Node.boundingBoxValues
            // order as ordinary placement, including modifiers before labels.
            self.ordinary_node_bounds(&ordinary)
        };
        for node in nodes.iter().copied() {
            if !self.bin_pack_node_is_vessel_carrier(node) {
                continue;
            }
            let (Some(position), size) =
                (self.active_node_position(node), self.active_node_size(node))
            else {
                continue;
            };
            let top_left = position;
            let bottom_right = Point {
                x: position.x + size.width,
                y: position.y + size.height,
            };
            bounds = Some(match bounds {
                None => (top_left, bottom_right),
                Some((mut low, mut high)) => {
                    low.x = low.x.min(top_left.x);
                    low.y = low.y.min(top_left.y);
                    high.x = high.x.max(bottom_right.x);
                    high.y = high.y.max(bottom_right.y);
                    (low, high)
                }
            });
            if self.active_aggregate_owner(node) == node
                && let Some(members) = self.active_aggregate_members(node)
            {
                for member in members.iter().copied() {
                    let Some(member_position) = self.position(member) else {
                        continue;
                    };
                    let member_size = self.nodes[member.0 as usize].rect.size;
                    let member_top_left = member_position;
                    let member_bottom_right = Point {
                        x: member_position.x + member_size.width,
                        y: member_position.y + member_size.height,
                    };
                    bounds = Some(match bounds {
                        None => (member_top_left, member_bottom_right),
                        Some((mut low, mut high)) => {
                            low.x = low.x.min(member_top_left.x);
                            low.y = low.y.min(member_top_left.y);
                            high.x = high.x.max(member_bottom_right.x);
                            high.y = high.y.max(member_bottom_right.y);
                            (low, high)
                        }
                    });
                }
            }
        }
        bounds
    }

    pub(super) fn bin_pack_bounding_box(&self, nodes: &[NodeId]) -> Option<(Point, Point)> {
        // With distinct real nodes, graphbounds.FixedBoundingBox is the
        // ordinary fixed-bounds contract. Applying the compact-carrier origin
        // override again would read stale member containers and replace the
        // already-correct origin selected through the concrete vessel.
        if nodes
            .iter()
            .all(|&node| !self.bin_pack_node_is_vessel_carrier(node))
        {
            return self.fixed_node_bounds(nodes);
        }
        let (mut top_left, bottom_right) = self.bin_pack_active_bounds(nodes)?;

        // Recovered `Nodes.getFixedBoundingBox` first computes the ordinary
        // bounds, then replaces only their top-left with `Nodes.getFixedOrigin`.
        // The origin is the current-minus-requested displacement of the first
        // fixed node directly owned by the subgraph's shallowest container.
        // At root, an exactly anchored fixed node therefore contributes
        // `(0, 0)` even when its visible box is elsewhere on the canvas.
        let subgraph_container = if nodes
            .iter()
            .any(|node| self.nodes[node.0 as usize].container.is_none())
        {
            None
        } else {
            nodes
                .iter()
                .filter_map(|node| self.nodes[node.0 as usize].container)
                .min_by_key(|container| {
                    let mut level = 0;
                    let mut current = Some(*container);
                    while let Some(node) = current {
                        level += 1;
                        current = self.nodes[node.0 as usize].container;
                    }
                    level
                })
        };
        if let Some(fixed_origin) = nodes.iter().find_map(|node| {
            let node_ref = &self.nodes[node.0 as usize];
            if self.active_node_container(*node) != subgraph_container {
                return None;
            }
            Some(Point {
                x: self.active_node_position(*node)?.x - node_ref.fixed_top_left?.x,
                y: self.active_node_position(*node)?.y - node_ref.fixed_top_left?.y,
            })
        }) {
            top_left = fixed_origin;
        }
        Some((top_left, bottom_right))
    }

    pub(super) fn bin_pack_score(&self, nodes: &[NodeId], root: Option<NodeId>) -> f64 {
        let Some((top_left, bottom_right)) = self.bin_pack_bounding_box(nodes) else {
            return 0.0;
        };
        let width = bottom_right.x - top_left.x;
        let height = bottom_right.y - top_left.y;
        let desired_axis_penalty = root.map_or(0.0, |root| {
            let root = &self.nodes[root.0 as usize];
            match (root.desired_width, root.desired_height) {
                (Some(desired), None) if desired > width => desired - width,
                (None, Some(desired)) if desired > height => desired - height,
                _ => 0.0,
            }
        });
        width * height + 0.5 * (width - height).powi(2) + 0.5 * desired_axis_penalty.powi(2)
    }

    /// Recovered `Node.getReachableNodes` traversal used by `Graph.BinPack`.
    ///
    /// Candidate ties are resolved by first occurrence, so the node order
    /// inside a connected component is behavioral state. TALA performs a
    /// breadth-first traversal, visiting each node's edges in their current
    /// lifecycle order, then ID-ordered near relations, then sequence and
    /// container relations in graph-node order.
    fn bin_pack_reachable_nodes_with(
        &self,
        start: NodeId,
        ignore: Option<NodeId>,
        descendant_root: Option<NodeId>,
        include_nears: bool,
    ) -> Vec<NodeId> {
        let start = self.active_aggregate_owner(start);
        let allowed = |node| {
            Some(node) != ignore
                && descendant_root
                    .is_none_or(|root| node != root && self.is_descendant_of(node, root))
        };
        let mut reachable = Vec::new();
        let mut visited = BTreeSet::from([start]);
        let mut queue = VecDeque::from([start]);
        while let Some(current) = queue.pop_front() {
            reachable.push(current);
            let mut enqueue_node = |node: NodeId, collapse_aggregate: bool| {
                let node = if collapse_aggregate {
                    self.active_aggregate_owner(node)
                } else {
                    node
                };
                if allowed(node) && visited.insert(node) {
                    queue.push_back(node);
                }
            };

            // Go's pointer-visible Node.Edges slice retains Graph.Edges
            // insertion order through the aggregate abduction lifecycle.
            // The stable arena's reconnect pass can move an ordinary edge to
            // the end of that slice; BinPack's BFS observes the original
            // edge order, so recover it locally without perturbing routing or
            // placement scans that use the general active-edge view.
            let traversal_edges = self
                .active_edge_ids(current)
                .into_iter()
                .collect::<Vec<_>>();
            // Keep the pointer-visible endpoint order. Go's `Node.Edges`
            // slice is rewritten by tree reconnection and aggregate cleanup;
            // its order can therefore differ from serialized edge IDs on the
            // second BinPack pass. Sorting here made the first pass stable but
            // hid that lifecycle difference and changed BFS tie ordering.
            for edge in traversal_edges {
                enqueue_node(self.active_adjacent(current, edge), true);
            }

            if include_nears {
                let mut nears = self.nodes[current.0 as usize].nears.keys();
                if let Some(members) = self.active_aggregate_leaf_members(current) {
                    for member in members {
                        nears.extend(self.nodes[member.0 as usize].nears.keys());
                    }
                }
                nears.stable_sort_by_key(|node| self.nodes[node.0 as usize].tala_id);
                nears.dedup();
                let start_container = self.active_node_container(start);
                for near in nears {
                    let near = self.active_aggregate_owner(near);
                    if self.active_node_container(near) == start_container {
                        enqueue_node(near, true);
                    }
                }
            }

            // Every BinPack traversal enables includeContainers.
            // getReachableNodes traverses the receiver's Sequence.Nodes even
            // after CleanupStuff deactivates and removes the vessel. Those
            // retained memberships preserve the defining-edge connectivity
            // that Graph.Disconnect removed from Graph.Edges and Node.Edges.
            if let Some(sequence) = self.nodes[current.0 as usize].sequence {
                for step in self.sequences[sequence].members.iter().copied() {
                    enqueue_node(step, true);
                }
            }

            // A live Go cluster vessel exposes its member pointers through
            // Graph.Clusters while reachability is walking with
            // includeContainers. The stable arena uses the first member as
            // the carrier, so enqueue the remaining member pointers here to
            // keep BinPack's rigid component inventory identical.
            if let Some(members) = self.active_aggregate_leaf_members(current) {
                for member in members {
                    enqueue_node(member, false);
                }
            }

            // Temporary cluster vessels expose their members here too.
            // active_aggregate_owner already expands that relation while the
            // vessel is active; ordinary hierarchy relations are traversed
            // over the current Graph.Nodes order.
            for other in self.graph_node_order() {
                if self.is_descendant_of(current, other) || self.is_descendant_of(other, current) {
                    enqueue_node(other, true);
                }
            }
        }
        reachable
    }

    fn bin_pack_reachable_nodes(&self, start: NodeId, scope: Option<NodeId>) -> Vec<NodeId> {
        self.bin_pack_reachable_nodes_with(start, scope, scope, true)
    }

    /// Builds the ordered reachability groups consumed by recovered
    /// `Graph.BinPack`.
    pub(super) fn bin_pack_groups(&self, scope: Option<NodeId>) -> Vec<Vec<NodeId>> {
        let direct = self.bin_pack_direct_children(scope);
        let mut assigned = BTreeSet::new();
        let mut groups = Vec::new();
        for start in direct {
            if assigned.contains(&start) {
                continue;
            }
            let group = self.bin_pack_reachable_nodes(start, scope);
            assigned.extend(group.iter().copied());
            groups.push(group);
        }
        groups
    }

    pub(super) fn bin_pack_placement_candidates(
        &self,
        nodes: &[NodeId],
        root: Option<NodeId>,
        edges_placed: bool,
    ) -> Vec<Point> {
        let mut candidates = Vec::new();
        let mut in_container = Vec::new();
        let mut deferred_member_candidates = BTreeMap::<NodeId, Vec<Point>>::new();
        // Go's placementCandidatesGuarded only emits points around individual
        // nodes when the first node is outside a hierarchy. Hierarchy groups
        // still contribute their six enclosing bounding-box points below, but
        // their interior points are intentionally absent. Generating then
        // filtering those points changes equal-score tie ordering.
        let consider_within = nodes
            .first()
            .is_none_or(|node| self.nodes[node.0 as usize].hierarchy.is_none());
        for node in nodes.iter().copied() {
            if self.active_node_container(node) != root {
                continue;
            }
            // The active carrier is visited as Go's temporary vessel.  Its
            // retained member pointer appears later in Node.Edges/Graph.Nodes;
            // hold the raw member candidates until the first non-carrier
            // member is reached so candidate ordering remains identical.
            if self.active_node_is_aggregate(node)
                && self.active_aggregate_owner(node) != node
                && let Some(points) =
                    deferred_member_candidates.remove(&self.active_aggregate_owner(node))
            {
                candidates.extend(points);
            }
            in_container.push(node);
            if !consider_within {
                continue;
            }
            let Some(position) = self.active_node_position(node) else {
                continue;
            };
            let size = self.active_node_size(node);
            let side_delta = if self.nodes[node.0 as usize].shape == ShapeKind::SqlTable {
                120.0
            } else {
                20.0
            };
            let mut has_right = false;
            let mut has_bottom = false;
            let mut has_bottom_right = false;
            if !edges_placed {
                for edge in self.active_edge_ids(node) {
                    let adjacent = self.adjacent(node, edge);
                    match self.sized_orientation(node, adjacent) {
                        Orientation::Left => has_right = true,
                        Orientation::Top => has_bottom = true,
                        Orientation::TopLeft => has_bottom_right = true,
                        _ => {}
                    }
                }
            }
            if edges_placed || !has_right {
                candidates.push(Point {
                    x: position.x + size.width + side_delta,
                    y: position.y,
                });
            }
            if edges_placed || !has_bottom {
                candidates.push(Point {
                    x: position.x,
                    y: position.y + size.height + 20.0,
                });
            }
            if edges_placed || !has_bottom_right {
                candidates.push(Point {
                    x: position.x + size.width + side_delta,
                    y: position.y + size.height,
                });
            }

            // A Go cluster/sequence exposes both its temporary vessel and
            // each retained member in the placement-candidate walk.  The
            // carrier's active geometry is the vessel box, while its stable
            // member box still contributes a second set of candidate points.
            // Emit those raw member points here without changing the active
            // movement/scoring representation.
            if self.active_node_is_aggregate(node) && self.active_aggregate_owner(node) == node {
                let members = self
                    .active_aggregate_members(node)
                    .map(|members| members.to_vec())
                    .unwrap_or_default();
                let mut member_points = Vec::new();
                for (member_index, member) in members.into_iter().enumerate() {
                    if member_index > 0 {
                        continue;
                    }
                    let Some(member_position) = self.position(member) else {
                        continue;
                    };
                    // The carrier's active position/size is the vessel. Its
                    // raw stable box is the retained member pointer that Go
                    // visits after the vessel, so include it as an additional
                    // candidate even when it aliases the carrier ID.
                    let member_size = self.nodes[member.0 as usize].rect.size;
                    let member_side_delta =
                        if self.nodes[member.0 as usize].shape == ShapeKind::SqlTable {
                            120.0
                        } else {
                            20.0
                        };
                    member_points.push(Point {
                        x: member_position.x + member_size.width + member_side_delta,
                        y: member_position.y,
                    });
                    member_points.push(Point {
                        x: member_position.x,
                        y: member_position.y + member_size.height + 20.0,
                    });
                    member_points.push(Point {
                        x: member_position.x + member_size.width + member_side_delta,
                        y: member_position.y + member_size.height,
                    });
                }
                deferred_member_candidates.insert(node, member_points);
            }
        }
        if let Some((top_left, bottom_right)) = self.bin_pack_bounding_box(&in_container) {
            candidates.extend([
                Point {
                    x: bottom_right.x,
                    y: top_left.y,
                },
                bottom_right,
                Point {
                    x: top_left.x,
                    y: bottom_right.y,
                },
                Point {
                    x: bottom_right.x + 20.0,
                    y: top_left.y,
                },
                Point {
                    x: bottom_right.x + 20.0,
                    y: bottom_right.y,
                },
                Point {
                    x: top_left.x,
                    y: bottom_right.y + 20.0,
                },
            ]);
        }
        candidates
    }

    /// Source packing/binpack.go506 captures these ordinary bounds once per
    /// greedy placement iteration, before candidate generation or trial moves.
    /// Point values and the eligible hierarchy inventory belong to that capture.
    fn bin_pack_hierarchy_boxes(&self, packed: &[Vec<NodeId>]) -> Vec<(Point, Point)> {
        packed
            .iter()
            .filter(|group| {
                group
                    .first()
                    .is_some_and(|node| self.nodes[node.0 as usize].hierarchy.is_some())
            })
            .filter_map(|group| self.bin_pack_unanchored_bounds(group))
            .collect()
    }

    /// The source query reads captured boundaries, never the trial graph.
    /// Contact with either corner is inclusive.
    fn bin_pack_point_in_hierarchy(boxes: &[(Point, Point)], point: Point) -> bool {
        boxes.iter().any(|&(top_left, bottom_right)| {
            top_left.x <= point.x
                && point.x <= bottom_right.x
                && top_left.y <= point.y
                && point.y <= bottom_right.y
        })
    }

    fn bin_pack_hierarchy_intrusion_bounds(&self, node: NodeId) -> Option<(Point, Point)> {
        if self.bin_pack_node_is_vessel_carrier(node) {
            // The temporary source vessel has its own undecorated body.
            let position = self.active_node_position(node)?;
            let size = self.active_node_size(node);
            return Some((
                position,
                Point {
                    x: (position.x + size.width).round(),
                    y: (position.y + size.height).round(),
                },
            ));
        }
        Some(Self::positioned_node_body_bounds(
            &self.nodes[node.0 as usize],
            self.position(node)?,
            true,
        ))
    }

    #[cfg(feature = "diagnostic-traces")]
    pub(super) fn diagnostic_bin_pack_bounds(
        &mut self,
        nodes: &[NodeId],
        points: &[Point],
    ) -> serde_json::Value {
        use serde_json::json;
        let point = |p: Point| {
            json!([
                format!("{:016x}", p.x.to_bits()),
                format!("{:016x}", p.y.to_bits())
            ])
        };
        let bounds = |value: Option<(Point, Point)>| match value {
            Some((low, high)) => json!({"low":point(low),"high":point(high)}),
            None => json!({"low":null,"high":null}),
        };
        let group = vec![nodes.to_vec()];
        let singles = nodes.iter().map(|&node| vec![node]).collect::<Vec<_>>();
        let deltas = self.bin_pack_smallest_deltas(&group);
        let single_deltas = self.bin_pack_smallest_deltas(&singles);
        let hierarchy = self.nodes[nodes[0].0 as usize].hierarchy.clone();
        self.assign_hierarchy_member(nodes[0], 0, None, 0, 1);
        let boxes = self.bin_pack_hierarchy_boxes(&group);
        let contains = points
            .iter()
            .map(|&point| Self::bin_pack_point_in_hierarchy(&boxes, point))
            .collect::<Vec<_>>();
        self.nodes[nodes[0].0 as usize].hierarchy = hierarchy;
        json!({
            "ordinary":bounds(self.bin_pack_unanchored_bounds(nodes)),
            "fixed":bounds(self.bin_pack_bounding_box(nodes)),
            "smallest_selection":point(Point { x:deltas.0, y:deltas.1 }),
            "smallest_nodes":point(Point { x:single_deltas.0, y:single_deltas.1 }),
            "intrusion":nodes.iter().map(|&node| bounds(self.bin_pack_hierarchy_intrusion_bounds(node))).collect::<Vec<_>>(),
            "hierarchy_contains":contains,
        })
    }

    #[cfg(feature = "diagnostic-traces")]
    pub(super) fn diagnostic_bin_pack_hierarchy_snapshot(
        &mut self,
        nodes: &[NodeId],
        points: &[Point],
        shift: Point,
    ) -> serde_json::Value {
        let hierarchy = self.nodes[nodes[0].0 as usize].hierarchy.clone();
        self.assign_hierarchy_member(nodes[0], 0, None, 0, 1);
        let group = vec![nodes.to_vec()];
        let boxes = self.bin_pack_hierarchy_boxes(&group);
        let before = points
            .iter()
            .map(|&point| Self::bin_pack_point_in_hierarchy(&boxes, point))
            .collect::<Vec<_>>();
        let saved = nodes
            .iter()
            .map(|&node| {
                let n = &self.nodes[node.0 as usize];
                (
                    node,
                    n.position.expect("snapshot requires positioned nodes"),
                    n.rect.origin,
                )
            })
            .collect::<Vec<_>>();
        for &node in nodes {
            let n = &mut self.nodes[node.0 as usize];
            let position = n
                .position
                .as_mut()
                .expect("snapshot requires positioned nodes");
            position.x += shift.x;
            position.y += shift.y;
            n.rect.origin = *position;
        }
        let cached = points
            .iter()
            .map(|&point| Self::bin_pack_point_in_hierarchy(&boxes, point))
            .collect::<Vec<_>>();
        let rebuilt = self.bin_pack_hierarchy_boxes(&group);
        let fresh = points
            .iter()
            .map(|&point| Self::bin_pack_point_in_hierarchy(&rebuilt, point))
            .collect::<Vec<_>>();
        for (node, position, origin) in saved {
            let n = &mut self.nodes[node.0 as usize];
            n.position = Some(position);
            n.rect.origin = origin;
        }
        self.nodes[nodes[0].0 as usize].hierarchy = hierarchy;
        serde_json::json!({"before":before,"after_cached":cached,"after_rebuilt":fresh})
    }

    /// Candidate-side approximation of Transaction.Commit's ordinary node
    /// spacing validation.
    pub(super) fn bin_pack_groups_overlap(&self, moving: &[NodeId], packed: &[NodeId]) -> bool {
        moving.iter().copied().any(|left| {
            let Some(left_position) = self.active_node_position(left) else {
                return false;
            };
            let left_size = self.active_node_size(left);
            packed.iter().copied().any(|right| {
                // Graph.IsBadState exempts ancestor/descendant pairs because
                // a packed child necessarily intersects its containing
                // hierarchy.  Candidate group checks must apply the same
                // exception or an ancestor container can block a valid child
                // translation before the transaction validator sees it.
                if self.active_is_descendant_of(left, right)
                    || self.active_is_descendant_of(right, left)
                {
                    return false;
                }
                let Some(right_position) = self.active_node_position(right) else {
                    return false;
                };
                let right_size = self.active_node_size(right);
                // Transaction.Commit validates every moved node through
                // Graph.IsBadState -> doesOverlap -> Node.getDeltaTo. BinPack
                // groups are disconnected, so this is the same recovered
                // 20-unit base plus table clearance and the two facing
                // outside-label/icon margins used by CombineSubgraphs.
                let delta = self.combine_subgraph_spacing_delta(left, right, left_position);
                let overlaps = left_position.x < right_position.x + right_size.width + delta
                    && right_position.x < left_position.x + left_size.width + delta
                    && left_position.y < right_position.y + right_size.height + delta
                    && right_position.y < left_position.y + left_size.height + delta;
                if overlaps && crate::internal::trace_env_enabled("TALA_TRACE_BINPACK_CANDIDATES") {
                    eprintln!(
                        "BINPACK_GROUP_OVERLAP_RUST left={} right={} delta={} left_pos={:?} left_size={:?} right_pos={:?} right_size={:?}",
                        self.nodes[left.0 as usize].tala_id,
                        self.nodes[right.0 as usize].tala_id,
                        delta,
                        left_position,
                        left_size,
                        right_position,
                        right_size
                    );
                }
                overlaps
            })
        })
    }

    /// `Graph.IsBadState(node, nil, false)` as used after BinPack wraps a
    /// non-cluster container. With no prior GraphState, TALA validates the
    /// node's containment and fixed-origin constraints, then checks it against
    /// every node except itself, descendants, and ancestors.
    pub(super) fn bin_pack_wrapped_container_is_bad_state(&self, node: NodeId) -> bool {
        let node_ref = &self.nodes[node.0 as usize];
        let trace_bad_state = crate::internal::trace_env_enabled("TALA_TRACE_BINPACK_DETAIL");
        let trace_reason = |reason: &str| {
            if trace_bad_state {
                eprintln!(
                    "BINPACK_WRAP_BAD_STATE_RUST node={} reason={}",
                    node_ref.tala_id, reason
                );
            }
        };
        if node_ref.cluster.is_some() {
            return false;
        }
        let Some(position) = self.position(node) else {
            return false;
        };
        let size = node_ref.rect.size;
        let surrounds = |outer: Point, outer_size: Size, inner: Point, inner_size: Size| {
            // Graph.IsBadState calls Node.Surrounds(child, 0): all four
            // comparisons are strict, including exact edge contact.
            inner.x > outer.x
                && inner.y > outer.y
                && inner.x + inner_size.width < outer.x + outer_size.width
                && inner.y + inner_size.height < outer.y + outer_size.height
        };
        if let Some(container) = node_ref.container
            && let Some(container_position) = self.position(container)
            && !surrounds(
                container_position,
                self.nodes[container.0 as usize].rect.size,
                position,
                size,
            )
        {
            trace_reason("container-escape");
            return true;
        }
        if node_ref.is_container
            && self
                .containers
                .get(&Some(node))
                .into_iter()
                .flatten()
                .any(|child| {
                    self.position(child).is_some_and(|child_position| {
                        !surrounds(
                            position,
                            size,
                            child_position,
                            self.nodes[child.0 as usize].rect.size,
                        )
                    })
                })
        {
            trace_reason("child-outside");
            return true;
        }
        // TALA checks the shaped content box separately from the outer node
        // box after wrapping. A child can remain strictly inside the node
        // rectangle while still protruding into the shape's padding or cutout;
        // that failed `childrenFit` check rolls back the complete packing
        // transaction.
        if node_ref.is_container
            && let Some(inner) = self.shape_inner_box(node)
            && self
                .containers
                .get(&Some(node))
                .into_iter()
                .flatten()
                .any(|child| {
                    self.position(child).is_some_and(|child_position| {
                        let child_size = self.nodes[child.0 as usize].rect.size;
                        child_position.x < inner.origin.x
                            || child_position.y < inner.origin.y
                            || child_position.x + child_size.width
                                > inner.origin.x + inner.size.width
                            || child_position.y + child_size.height
                                > inner.origin.y + inner.size.height
                    })
                })
        {
            trace_reason("inner-box");
            return true;
        }
        if self.bin_pack_group_past_fixed_origin(&[node]) {
            trace_reason("fixed-origin");
            return true;
        }
        // Graph.doesOverlapWithDimensionsContext scans Graph.Nodes. Retired
        // vessels and other retained entities are not current obstacles;
        // active carriers must resolve to their actual allocated vessels.
        let members = self.original_owner_member_nodes();
        let overlap = members.iter().copied().any(|other_id| {
            let other = &self.nodes[other_id.0 as usize];
            if self.is_descendant_of(other_id, node) || self.is_descendant_of(node, other_id) {
                return false;
            }
            let Some(other_position) = self.position(other_id) else {
                return false;
            };
            let delta = self.spacing_delta_with_loops(node, other_id, position);

            let overlaps = position.x < other_position.x + other.rect.size.width + delta
                && position.x + size.width + delta > other_position.x
                && position.y < other_position.y + other.rect.size.height + delta
                && position.y + size.height + delta > other_position.y;
            #[cfg(feature = "diagnostic-traces")]
            if overlaps && std::env::var_os("D2RS_TRACE_PACKING_OVERLAP").is_some() {
                eprintln!("{}", serde_json::json!({
                    "event": "packing-wrap-overlap",
                    "node": node_ref.tala_id.to_string(), "other": other.tala_id.to_string(),
                    "node_box": [position.x, position.y, size.width, size.height],
                    "other_box": [other_position.x, other_position.y, other.rect.size.width, other.rect.size.height],
                    "delta_bits": format!("{:016x}", delta.to_bits()),
                    "other_is_member": members.contains(&other_id),
                    "members": members.iter().map(|node| self.nodes[node.0 as usize].tala_id.to_string()).collect::<Vec<_>>()
                }));
            }
            overlaps
        });
        if overlap {
            trace_reason("overlap");
        }
        overlap
    }

    /// Recovered `Graph.IsBadState` fixed-origin validation for BinPack's
    /// affected nodes. `getContainerFixedOrigin` scans fixed nodes directly
    /// owned by the same container and returns current-minus-requested
    /// coordinates; a transaction may not move another node above or left of
    /// that origin.
    pub(super) fn bin_pack_group_past_fixed_origin(&self, moving: &[NodeId]) -> bool {
        moving.iter().copied().any(|node| {
            let container = self.active_node_container(node);
            let fixed_origin = self.nodes.iter().find_map(|candidate| {
                if self.active_node_container(candidate.input_id) != container {
                    return None;
                }
                // graph.go:containerFixedOrigin reads the current TopLeft.
                // Routing and packing may already have bound this node to a
                // shared point; its original inline coordinates are stale.
                let position = self.position(candidate.input_id)?;
                Some(Point {
                    x: position.x - candidate.fixed_top_left?.x,
                    y: position.y - candidate.fixed_top_left?.y,
                })
            });
            fixed_origin.is_some_and(|origin| {
                self.active_node_position(node)
                    .is_some_and(|position| position.x < origin.x || position.y < origin.y)
            })
        })
    }

    /// Transaction.Commit preserves every positioned fixed node against the
    /// transaction snapshot (graph.go:isStructurallyBadStateWithFixedOriginContext).
    /// BinPack creates that snapshot after staging movable groups off-canvas.
    /// A fixed descendant therefore still constrains its movable ancestor.
    fn bin_pack_fixed_positions_unchanged(&self, before: &Self) -> bool {
        self.graph_node_order_cow().iter().copied().all(|id| {
            let node = &self.nodes[id.0 as usize];
            // Current vessels are distinct source nodes with no FixedTopLeft;
            // retained cluster members bypass structural validation entirely.
            if node.fixed_top_left.is_none()
                || self.active_node_is_aggregate(id)
                || node.cluster.is_some()
            {
                return true;
            }
            let Some(position) = self.active_node_position(id) else {
                return true;
            };
            let original = before.active_node_position(id).unwrap_or_default();
            position.x == original.x && position.y == original.y
        })
    }

    #[cfg(test)]
    pub(super) fn translate_bin_pack_group(
        &mut self,
        group: &[NodeId],
        scope: Option<NodeId>,
        delta: Point,
    ) {
        let context = binpack_guard::PackingContext;
        let mut guard = limits::WorkGuard::new(
            Some(&context),
            "BinPack",
            binpack_guard::MAX_BIN_PACK_WORK_UNITS,
        )
        .unwrap();
        self.translate_bin_pack_group_guarded(group, scope, delta, &mut guard)
            .unwrap();
    }

    pub(super) fn translate_bin_pack_group_guarded(
        &mut self,
        group: &[NodeId],
        _scope: Option<NodeId>,
        delta: Point,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<(), limits::Error> {
        // `Node.translate` in recovered Graph.BinPack changes only the
        // receiver's box.  It does not recurse through containers.  The
        // recovered graph also contains one temporary cluster vessel while
        // the Rust arena retains the stable member boxes, so publish the
        // vessel movement separately and move every retained box in the
        // group exactly once.  Filtering to direct children and calling the
        // ordinary recursive translation here leaves nested members behind;
        // their stale coordinates then change the next group's bounding box.
        let mut moved_clusters = BTreeSet::new();
        for node in group.iter().copied() {
            guard.step()?;
            if let Some(cluster_index) = self.active_cluster_index(node)
                && self.cluster_is_active(&self.clusters[cluster_index])
                && self.clusters[cluster_index].members.first() == Some(&node)
            {
                // A nested BinPack scope translates the retained member
                // boxes, but its enclosing temporary vessel is not one of
                // that scope's current nodes. Only the group containing the
                // vessel owner publishes a pending-vessel translation.
                moved_clusters.insert(cluster_index);
            }
            if let Some(position) = self.position(node) {
                // Graph.Nodes and Graph.Containers share the same Go pointer.
                // A BinPack translation touches the member box, and every
                // retained projection of that identity observes the same
                // translation even when the descendant is absent from this
                // temporary Nodes slice.
                self.set_projected_node_position(
                    self.nodes[node.0 as usize].tala_id,
                    Point {
                        x: position.x + delta.x,
                        y: position.y + delta.y,
                    },
                );
            }
        }
        for cluster_index in moved_clusters {
            if let Some(position) = self
                .pending_cluster_vessel_positions
                .get_mut(&cluster_index)
            {
                position.x += delta.x;
                position.y += delta.y;
            }
        }
        Ok(())
    }

    /// Recovered BinPack route ownership after an accepted node placement.
    ///
    /// Staging and candidate transactions move only node boxes. Once a
    /// placement is accepted, TALA walks the group's nodes in order and moves
    /// each incident route exactly once by the first owning node's displacement
    /// from its pre-BinPack position.
    #[cfg(test)]
    pub(super) fn translate_bin_pack_routes_for_group(
        &mut self,
        group: &[NodeId],
        original_positions: &BTreeMap<NodeId, Point>,
    ) {
        let context = binpack_guard::PackingContext;
        let mut guard = limits::WorkGuard::new(
            Some(&context),
            "BinPack",
            binpack_guard::MAX_BIN_PACK_WORK_UNITS,
        )
        .unwrap();
        self.translate_bin_pack_routes_for_group_guarded(group, original_positions, &mut guard)
            .unwrap();
    }

    pub(super) fn translate_bin_pack_routes_for_group_guarded(
        &mut self,
        group: &[NodeId],
        original_positions: &BTreeMap<NodeId, Point>,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<(), limits::Error> {
        let mut seen_edges = BTreeSet::new();
        for node in group.iter().copied() {
            guard.step()?;
            let Some(original) = original_positions.get(&node).copied() else {
                continue;
            };
            let Some(current) = self.active_node_position(node) else {
                continue;
            };
            let delta = Point {
                x: current.x - original.x,
                y: current.y - original.y,
            };
            let incident: Vec<_> = self.active_edge_ids(node).into_iter().collect();
            if crate::internal::trace_env_enabled("TALA_TRACE_BINPACK_ROUTE") {
                eprintln!(
                    "BINPACK_ROUTE_GROUP_RUST node={} original={:?} current={:?} delta={:?} incident={:?}",
                    self.nodes[node.0 as usize].tala_id,
                    original,
                    current,
                    delta,
                    incident
                        .iter()
                        .map(|id| self.edge_records[id.0 as usize].tala_id)
                        .collect::<Vec<_>>()
                );
            }
            for edge_id in incident {
                guard.step()?;
                if !seen_edges.insert(edge_id) {
                    continue;
                }
                for mut point in self.edge_records[edge_id.0 as usize].route.iter_mut() {
                    guard.step()?;
                    point.x += delta.x;
                    point.y += delta.y;
                }
            }
        }
        Ok(())
    }

    /// Recovered `getSmallestDeltas`: measure each remaining subgraph
    /// independently, then retain the smallest width and height. The values
    /// are temporal carriers: BinPack freezes them into the segment inventory
    /// when a group becomes packed.
    pub(super) fn bin_pack_smallest_deltas(&self, groups: &[Vec<NodeId>]) -> (f64, f64) {
        groups
            .iter()
            .filter_map(|group| self.bin_pack_unanchored_bounds(group))
            .fold(
                (f64::INFINITY, f64::INFINITY),
                |(smallest_x, smallest_y), (top_left, bottom_right)| {
                    (
                        smallest_x.min(bottom_right.x - top_left.x),
                        smallest_y.min(bottom_right.y - top_left.y),
                    )
                },
            )
    }

    pub(super) fn bin_pack_segment_nodes(
        &self,
        nodes: &[NodeId],
        scope: Option<NodeId>,
    ) -> Vec<NodeId> {
        nodes
            .iter()
            .copied()
            .filter(|node| Some(*node) != scope && self.is_descendant_of_scope(*node, scope))
            .collect()
    }

    /// Recovered `getSegments`/`estimateEdgeSegments` for ordinary endpoints.
    /// Axis-aligned edges shorter than the smallest remaining subgraph on that
    /// axis do not reserve a route corridor. Diagonal edges reserve both L
    /// alternatives and both rounded midpoint alternatives.
    pub(super) fn bin_pack_estimated_edge_segments(
        &self,
        nodes: &[NodeId],
        smallest_x_gap: f64,
        smallest_y_gap: f64,
    ) -> Vec<(Point, Point)> {
        let mut edge_ids = BTreeSet::new();
        for node in nodes.iter().copied() {
            edge_ids.extend(self.active_edge_ids(node));
        }
        let mut segments = Vec::new();
        for edge_id in edge_ids {
            let edge = &self.edge_records[edge_id.0 as usize];
            let from_node = self.active_aggregate_owner(edge.from);
            let to_node = self.active_aggregate_owner(edge.to);
            if from_node == to_node {
                continue;
            };
            let (Some(from_position), Some(to_position)) = (
                self.active_node_position(from_node),
                self.active_node_position(to_node),
            ) else {
                continue;
            };
            let from_size = self.active_node_size(from_node);
            let to_size = self.active_node_size(to_node);
            let from = Point {
                x: from_position.x + from_size.width * 0.5,
                y: from_position.y + from_size.height * 0.5,
            };
            let to = Point {
                x: to_position.x + to_size.width * 0.5,
                y: to_position.y + to_size.height * 0.5,
            };
            let orientation =
                self.sized_box_orientation((from_position, from_size), (to_position, to_size));
            let from_cluster = self.active_cluster_index(edge.from);
            let to_cluster = self.active_cluster_index(edge.to);
            if !orientation.is_diagonal() && from_cluster.is_none() && to_cluster.is_none() {
                match orientation {
                    Orientation::Left | Orientation::Right
                        if (from.x - to.x).abs() > smallest_x_gap =>
                    {
                        segments.push((from, to));
                    }
                    Orientation::Top | Orientation::Bottom
                        if (from.y - to.y).abs() > smallest_y_gap =>
                    {
                        segments.push((from, to));
                    }
                    _ => {}
                }
            } else {
                // estimateEdgeSegments' local getClusterEndpoints closure
                // exposes the two ends of an arranged cluster vessel.
                // Sequence vessels are ordinary endpoints here: only
                // Cluster marks its vessel with isClusterVessel.
                let endpoint_points =
                    |node: NodeId, cluster_index: Option<usize>, center: Point| {
                        let Some(cluster_index) = cluster_index else {
                            return vec![center];
                        };
                        let position = self
                            .position(node)
                            .expect("positioned active cluster vessel");
                        let size = self.active_node_size(node);
                        match self.clusters[cluster_index].arrangement {
                            ClusterArrangement::Row => vec![
                                Point {
                                    x: position.x,
                                    y: center.y,
                                },
                                Point {
                                    x: position.x + size.width,
                                    y: center.y,
                                },
                            ],
                            ClusterArrangement::Column => vec![
                                Point {
                                    x: center.x,
                                    y: position.y,
                                },
                                Point {
                                    x: center.x,
                                    y: position.y + size.height,
                                },
                            ],
                        }
                    };
                let from_points = endpoint_points(from_node, from_cluster, from);
                let to_points = endpoint_points(to_node, to_cluster, to);
                for from in from_points {
                    for to in to_points.iter().copied() {
                        let from_corner = Point { x: from.x, y: to.y };
                        let to_corner = Point { x: to.x, y: from.y };
                        let middle_x = ((from.x + to.x) * 0.5).round();
                        let middle_y = ((from.y + to.y) * 0.5).round();
                        segments.extend([
                            (from, from_corner),
                            (from_corner, to),
                            (from, to_corner),
                            (to_corner, to),
                            (
                                Point {
                                    x: middle_x,
                                    y: from.y,
                                },
                                Point {
                                    x: middle_x,
                                    y: to.y,
                                },
                            ),
                            (
                                Point {
                                    x: from.x,
                                    y: middle_y,
                                },
                                Point {
                                    x: to.x,
                                    y: middle_y,
                                },
                            ),
                        ]);
                    }
                }
            }
        }
        segments
    }

    pub(super) fn bin_pack_routed_edge_segments(&self, nodes: &[NodeId]) -> Vec<(Point, Point)> {
        let mut edge_ids = BTreeSet::new();
        for node in nodes.iter().copied() {
            edge_ids.extend(self.nodes[node.0 as usize].edges.read().iter().copied());
        }
        edge_ids
            .into_iter()
            .flat_map(|edge_id| {
                self.edge_records[edge_id.0 as usize]
                    .route
                    .read()
                    .windows(2)
                    // Recovered BinPack appends Edges.getSegments(true)
                    // followed by Edges.getSegments(false). Diagonal route
                    // legs belong to neither inventory and therefore cannot
                    // obstruct a later packing candidate.
                    .filter(|pair| pair[0].y == pair[1].y || pair[0].x == pair[1].x)
                    .map(|pair| (pair[0], pair[1]))
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    pub(super) fn bin_pack_frozen_segments(
        &self,
        nodes: &[NodeId],
        scope: Option<NodeId>,
        remaining: &[Vec<NodeId>],
        edges_placed: bool,
    ) -> Vec<(Point, Point)> {
        let filtered = self.bin_pack_segment_nodes(nodes, scope);
        if edges_placed {
            self.bin_pack_routed_edge_segments(&filtered)
        } else {
            let (smallest_x, smallest_y) = self.bin_pack_smallest_deltas(remaining);
            self.bin_pack_estimated_edge_segments(&filtered, smallest_x, smallest_y)
        }
    }

    pub(super) fn segment_overlaps_padded_node(
        &self,
        start: Point,
        end: Point,
        node: NodeId,
        padding: f64,
    ) -> bool {
        let Some(position) = self.active_node_position(node) else {
            return false;
        };
        let size = self.active_node_size(node);
        let left = position.x - padding;
        let right = position.x + size.width + padding;
        let top = position.y - padding;
        let bottom = position.y + size.height + padding;
        let dx = end.x - start.x;
        let dy = end.y - start.y;
        let mut lower: f64 = 0.0;
        let mut upper: f64 = 1.0;
        for (p, q) in [
            (-dx, start.x - left),
            (dx, right - start.x),
            (-dy, start.y - top),
            (dy, bottom - start.y),
        ] {
            if p == 0.0 {
                if q < 0.0 {
                    return false;
                }
                continue;
            }
            let ratio = q / p;
            if p < 0.0 {
                lower = lower.max(ratio);
            } else {
                upper = upper.min(ratio);
            }
            if lower > upper {
                return false;
            }
        }
        true
    }

    pub(super) fn bin_pack_blocks_routes(
        &self,
        packing: &[NodeId],
        packed: &[Vec<NodeId>],
        packed_segments: &[(Point, Point)],
        scope: Option<NodeId>,
    ) -> bool {
        self.bin_pack_blocks_routes_with_deltas(
            packing,
            packed,
            packed_segments,
            scope,
            self.bin_pack_smallest_deltas(packed),
        )
    }

    fn bin_pack_blocks_routes_with_deltas(
        &self,
        packing: &[NodeId],
        packed: &[Vec<NodeId>],
        packed_segments: &[(Point, Point)],
        scope: Option<NodeId>,
        (smallest_x, smallest_y): (f64, f64),
    ) -> bool {
        let filtered_packing = self.bin_pack_segment_nodes(packing, scope);
        let packing_segments =
            self.bin_pack_estimated_edge_segments(&filtered_packing, smallest_x, smallest_y);
        packed.iter().flatten().copied().any(|node| {
            packing_segments
                .iter()
                .copied()
                .any(|(start, end)| self.segment_overlaps_padded_node(start, end, node, 20.0))
        }) || packing.iter().copied().any(|node| {
            packed_segments
                .iter()
                .copied()
                .any(|(start, end)| self.segment_overlaps_padded_node(start, end, node, 20.0))
        })
    }

    pub(super) fn bin_pack_group_has_external_relation(
        &self,
        group: &[NodeId],
        scope: Option<NodeId>,
        edges_placed: bool,
    ) -> bool {
        let Some(scope) = scope else {
            return false;
        };
        let members = group.iter().copied().collect::<BTreeSet<_>>();
        self.member_edges().any(|edge| {
            let from = members.contains(&edge.from);
            let to = members.contains(&edge.to);
            // Once routes exist, moving a descendant connected directly to
            // this container translates the container endpoint off its fixed
            // border. Go's BinPack treats that attachment as external even
            // though both endpoints are descendants of the scope.
            if edges_placed && ((from && edge.to == scope) || (to && edge.from == scope)) {
                return true;
            }
            from != to
                && (!self.is_descendant_of_scope(edge.from, Some(scope))
                    || !self.is_descendant_of_scope(edge.to, Some(scope)))
        }) || group.iter().copied().any(|node| {
            self.nodes[node.0 as usize]
                .nears
                .keys()
                .into_iter()
                .any(|near| {
                    !members.contains(&near) && !self.is_descendant_of_scope(near, Some(scope))
                })
        })
    }

    pub(super) fn bin_pack_movement_permissions(
        &self,
        scope: Option<NodeId>,
        edges_placed: bool,
    ) -> (bool, bool, bool, bool) {
        let Some(root) = scope.filter(|_| edges_placed) else {
            return (true, true, true, true);
        };
        let mut can_move_left = true;
        let mut can_move_right = true;
        let mut can_move_top = true;
        let mut can_move_bottom = true;
        for edge_id in self.nodes[root.0 as usize].edges.read().iter().copied() {
            let edge = &self.edge_records[edge_id.0 as usize];
            if edge.route.len() < 2 {
                continue;
            }
            let (start, end) = if edge.from == root {
                (edge.route.point(0), edge.route.point(1))
            } else {
                let last = edge.route.len() - 1;
                (edge.route.point(last), edge.route.point(last - 1))
            };
            if start.y == end.y {
                if start.x < end.x {
                    can_move_right = false;
                } else if start.x > end.x {
                    can_move_left = false;
                }
                continue;
            }
            if start.x == end.x {
                if start.y < end.y {
                    can_move_bottom = false;
                } else if start.y > end.y {
                    can_move_top = false;
                }
                continue;
            }
            return (false, false, false, false);
        }
        (can_move_left, can_move_right, can_move_top, can_move_bottom)
    }

    pub(super) fn bin_pack_partition_groups(
        &self,
        groups: Vec<Vec<NodeId>>,
        scope: Option<NodeId>,
    ) -> (Vec<Vec<NodeId>>, Vec<Vec<NodeId>>) {
        let mut packed = Vec::<Vec<NodeId>>::new();
        let mut to_pack = Vec::<Vec<NodeId>>::new();
        let mut fixed_nodes_group = Vec::<NodeId>::new();
        for group in groups {
            let fixed = group
                .iter()
                .any(|node| self.nodes[node.0 as usize].fixed_top_left.is_some());
            let canvas = group
                .iter()
                .any(|node| self.nodes[node.0 as usize].canvas_position.is_some());
            if fixed {
                fixed_nodes_group.extend(group);
            } else if canvas || self.bin_pack_group_has_external_relation(&group, scope, false) {
                packed.push(group);
            } else {
                to_pack.push(group);
            }
        }
        // Recovered BinPack appends one fixedNodesSubgraph after all
        // cross-container groups. Disconnected fixed components therefore
        // remain one atomic packed obstacle and contribute candidates only
        // after every cross-container group.
        if !fixed_nodes_group.is_empty() {
            fixed_nodes_group.stable_sort();
            fixed_nodes_group.dedup();
            packed.push(fixed_nodes_group);
        }
        (packed, to_pack)
    }

    /// Recovered `Node.wrapChildren` terminal step of a successful nested
    /// `Graph.BinPack`. Children stay at their accepted absolute positions;
    /// the container is resized and moved around their label-aware bounds.
    #[cfg(test)]
    pub(super) fn wrap_bin_packed_container(&mut self, container: NodeId) {
        let mut guard =
            limits::WorkGuard::new(Some(&binpack_guard::PackingContext), "BinPack", i64::MAX)
                .expect("unlimited test guard");
        self.wrap_bin_packed_container_checked(container, &mut guard)
            .expect("unlimited container wrap");
    }

    /// packing/guard.go::binPackWrapChildren. The checked fixed bounds and
    /// each label visit are the operations used to produce the fitted box.
    fn wrap_bin_packed_container_checked(
        &mut self,
        container: NodeId,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<(), limits::Error> {
        if !self.nodes[container.0 as usize].is_container {
            return Ok(());
        }
        let children = self.packing_direct_refs(Some(container));
        let padding = self.shape_fit_padding(container);
        let refs = children.iter().copied().map(Some).collect::<Vec<_>>();
        let (mut top_left, mut bottom_right) = self
            .packing_fixed_bounds_checked(&refs, guard)
            .map_err(|failure| failure.error)?;
        let trace_wrap_detail = crate::internal::trace_env_enabled("TALA_TRACE_BINPACK_DETAIL");
        for child in children {
            guard.step()?;
            let (position, size, physical) = self.packing_bounds_geometry(child);
            let position = position.expect("positioned packed child");
            if let Some(label) = physical.and_then(|node| node.label_size) {
                if (position.x == top_left.x || position.x + size.width == bottom_right.x)
                    && label.width > size.width
                {
                    let half_overhang = size.width / 2.0 - label.width / 2.0;
                    top_left.x = routing::channel_solver::minimum(
                        top_left.x,
                        (position.x + half_overhang).floor(),
                    );
                    bottom_right.x = routing::channel_solver::maximum(
                        bottom_right.x,
                        (position.x + size.width - half_overhang).ceil(),
                    );
                }
            }
        }
        let content = Size {
            width: bottom_right.x - top_left.x,
            height: bottom_right.y - top_left.y,
        };
        let mut fitted = self.bin_pack_shape_dimensions_to_fit(container, content, padding);
        if let Some(minimum) = self.nodes[container.0 as usize].folded_label_min_size {
            fitted.width = fitted.width.max(minimum.width);
            fitted.height = fitted.height.max(minimum.height);
        }
        self.nodes[container.0 as usize].rect.size = fitted;
        // binPackWrapChildren rounds Node.InsidePlacement in the absolute
        // frame, then Node.Translate adds the resulting delta to TopLeft.
        // Keep both operations and their grouping: cancellation of the old
        // origin changes fractional positions and later packing decisions.
        let absolute = self
            .shape_inside_placement_absolute(container, content, padding)
            .expect("positioned packed container");
        self.packing_translate_ref(
            binpack_traversal::PackingNodeRef::Node(container),
            Point {
                x: top_left.x - absolute.x,
                y: top_left.y - absolute.y,
            },
        );
        if trace_wrap_detail {
            eprintln!(
                "BINPACK_WRAP_RUST node={} bounds=({},{:?})->({},{:?}) padding={:?} content={:?} fitted={:?}",
                self.nodes[container.0 as usize].tala_id,
                top_left.x,
                top_left.y,
                bottom_right.x,
                bottom_right.y,
                padding,
                content,
                fitted
            );
        }
        guard.finish()
    }

    /// Idiomatic stable-ID translation of recovered `Graph.BinPack` for one
    /// hierarchy scope. Candidate evaluation is transactional by cloning the
    /// arena: a rejected placement cannot leak either node or descendant
    /// movement into the next candidate.
    #[cfg(test)]
    pub(super) fn bin_pack_scope(&mut self, scope: Option<NodeId>, edges_placed: bool) -> bool {
        let context = binpack_guard::PackingContext;
        let mut guard = limits::WorkGuard::new(
            Some(&context),
            "BinPack",
            binpack_guard::MAX_BIN_PACK_WORK_UNITS,
        )
        .unwrap();
        self.bin_pack_scope_guarded(scope, edges_placed, &mut guard)
            .unwrap()
    }

    pub(super) fn bin_pack_scope_guarded(
        &mut self,
        scope: Option<NodeId>,
        edges_placed: bool,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<bool, limits::Error> {
        // Manual scope/test callers do not have packGuarded's retained edge
        // inventory. The recursive production path passes the original scan.
        let incident = self
            .edge_order
            .read()
            .iter()
            .copied()
            .filter(|&edge| {
                let e = &self.edge_records[edge.0 as usize];
                scope.is_some_and(|root| e.from == root || e.to == root)
            })
            .collect::<Vec<_>>();
        self.bin_pack_scope_guarded_with_incidents(scope, edges_placed, &incident, guard)
    }

    fn bin_pack_scope_guarded_with_incidents(
        &mut self,
        scope: Option<NodeId>,
        edges_placed: bool,
        graph_incident: &[EdgeId],
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<bool, limits::Error> {
        #[cfg(feature = "diagnostic-traces")]
        crate::internal::call_counts::count!(Binpack);
        if scope.is_none()
            && self
                .nodes
                .iter()
                .any(|node| node.container.is_none() && node.canvas_position.is_some())
        {
            return Ok(false);
        }
        let trace_binpack_scope = crate::internal::trace_env_enabled("TALA_TRACE_BINPACK_GROUPS");
        let trace_binpack_candidates =
            crate::internal::trace_env_enabled("TALA_TRACE_BINPACK_CANDIDATES");
        if trace_binpack_scope {
            eprint!(
                "BINPACK_SCOPE_RUST root={:?} direct=",
                scope.map(|node| self.nodes[node.0 as usize].tala_id)
            );
            for node in self.containers.get(&scope).into_iter().flatten() {
                let child = &self.nodes[node.0 as usize];
                eprint!(
                    " {}@{:?}:{}x{}",
                    child.tala_id, child.position, child.rect.size.width, child.rect.size.height
                );
            }
            eprintln!();
        }
        let (packed, to_pack, cross_container_count) =
            self.packing_subgraphs_checked(scope, edges_placed, guard)?;
        let initial_bounds_nodes = to_pack
            .first()
            .map(|group| group.iter().copied().map(Some).collect::<Vec<_>>());
        let mut packed_refs = packed;
        let mut packed = self.packing_numerical_groups(packed_refs.clone());
        let mut to_pack_refs = to_pack;
        let mut to_pack = self.packing_numerical_groups(to_pack_refs.clone());
        if crate::internal::trace_env_enabled("TALA_TRACE_BINPACK_ROUTE") {
            eprintln!(
                "BINPACK_SUBGRAPHS_RUST root={:?} packed={:?} to_pack={:?}",
                scope.map(|node| self.nodes[node.0 as usize].tala_id),
                packed
                    .iter()
                    .map(|group| group
                        .iter()
                        .map(|node| self.nodes[node.0 as usize].tala_id)
                        .collect::<Vec<_>>())
                    .collect::<Vec<_>>(),
                to_pack
                    .iter()
                    .map(|group| group
                        .iter()
                        .map(|node| self.nodes[node.0 as usize].tala_id)
                        .collect::<Vec<_>>())
                    .collect::<Vec<_>>(),
            );
        }
        if to_pack.is_empty() {
            // packGuarded checks once when no contained subgraph can move.
            guard.step()?;
            return Ok(false);
        }
        trace_binpack_event(
            self,
            "packed",
            scope,
            &packed,
            cross_container_count,
            None,
            0.0,
        );
        trace_binpack_event(self, "to-pack", scope, &to_pack, 0, None, 0.0);
        if crate::internal::trace_env_enabled("TALA_TRACE_BINPACK_GROUPS") {
            eprint!("BINPACK_POS_RUST");
            for node in &self.nodes {
                if node.container.is_none() {
                    eprint!(
                        " {}@{:?}:{}x{}",
                        node.tala_id, node.position, node.rect.size.width, node.rect.size.height
                    );
                }
            }
            eprintln!();
            eprint!(
                "BINPACK_GROUPS_RUST root={:?} packed=",
                scope.map(|node| self.nodes[node.0 as usize].tala_id)
            );
            for group in &packed {
                eprint!("[");
                for node in group {
                    eprint!("{},", self.nodes[node.0 as usize].tala_id);
                }
                eprint!("]");
            }
            eprint!(" to_pack=");
            for group in &to_pack {
                eprint!("[");
                for node in group {
                    eprint!("{},", self.nodes[node.0 as usize].tala_id);
                }
                eprint!("]");
            }
            eprintln!();
        }
        // The Go transaction snapshot contains a distinct temporary vessel
        // node. In the stable arena that node is represented by a pending
        // aggregate position rather than a NodeId, so the ordinary node
        // position restore below cannot roll it back by itself.
        // Source checks the first contained group's origin before the score.
        // Retain vessel/member identities until both checked scans finish.
        let root_top_left = self
            .packing_container_origin_checked(initial_bounds_nodes.as_ref().unwrap(), scope, guard)
            .map_err(|failure| failure.error)?;
        let score_nodes = self
            .packing_direct_refs(scope)
            .into_iter()
            .map(Some)
            .collect::<Vec<_>>();
        let original_score = self
            .packing_score_checked(&score_nodes, scope, guard)
            .map_err(|failure| failure.error)?;

        if crate::internal::trace_env_enabled("TALA_TRACE_BINPACK_GROUPS") {
            let bounds = self.bin_pack_bounding_box(&to_pack[0]);
            eprintln!(
                "BINPACK_ROOT_RUST root={:?} root_tl={:?} root_size={:?} root_label={:?} original_score={} bounds={:?}",
                scope.map(|node| self.nodes[node.0 as usize].tala_id),
                root_top_left,
                scope.map(|node| self.nodes[node.0 as usize].rect.size),
                scope.map(|node| self.nodes[node.0 as usize].label_position),
                original_score,
                bounds
            );
        }
        // binpack.go:245-334 snapshots immediately before each move and
        // copies each incident route on its first visit. Keep vessel identities
        // through movement even while the numerical candidate layer uses carriers.
        let mut staging = binpack_staging::PackingStaging::default();
        self.packing_stage_checked(
            &to_pack_refs,
            packed.is_empty(),
            root_top_left,
            edges_placed,
            &mut staging,
            guard,
        )?;
        if packed.is_empty() {
            packed_refs.push(to_pack_refs.remove(0));
            packed.push(to_pack.remove(0));
        }

        let reference_groups = |groups: &[Vec<binpack_traversal::PackingNodeRef>]| {
            groups
                .iter()
                .map(|g| g.iter().copied().map(Some).collect::<Vec<_>>())
                .collect::<Vec<_>>()
        };
        let mut segment_state = binpack_segments::PackingSegments::default();
        self.packing_initial_segments_checked(
            &reference_groups(&packed_refs),
            &reference_groups(&to_pack_refs),
            scope.map(binpack_traversal::PackingNodeRef::Node),
            edges_placed,
            &mut segment_state,
            guard,
        )?;
        let mut permissions = [true; 4];
        self.packing_permissions_checked(
            scope.map(binpack_traversal::PackingNodeRef::Node),
            edges_placed,
            &mut permissions,
            guard,
        )?;
        let [can_move_left, can_move_right, can_move_top, can_move_bottom] = permissions;
        if trace_binpack_scope {
            eprintln!(
                "BINPACK_PERM_RUST root={:?} LRTB={}{}{}{} edges={}",
                scope.map(|node| self.nodes[node.0 as usize].tala_id),
                can_move_left,
                can_move_right,
                can_move_top,
                can_move_bottom,
                scope.map_or(0, |node| self.nodes[node.0 as usize].edges.len())
            );
        }

        let mut accepted_candidates = Vec::<Point>::new();
        let mut failed = false;
        while !to_pack.is_empty() {
            guard.step()?;
            let (hierarchy_boxes, mut candidates, packed_top_left) = self
                .packing_prepare_candidates_checked(
                    &reference_groups(&packed_refs),
                    scope.map(binpack_traversal::PackingNodeRef::Node),
                    Some(root_top_left),
                    edges_placed,
                    &accepted_candidates,
                    guard,
                )?;
            let mut candidate_seen = candidates.clone();
            // Source removes the current group only after the packed-group
            // hierarchy, bounds and candidate scans have all succeeded.
            let current = to_pack.remove(0);
            let current_refs = to_pack_refs.remove(0);
            let current_selection = current_refs.iter().copied().map(Some).collect::<Vec<_>>();
            let (current_low, current_high) = self
                .packing_fixed_bounds_checked(&current_selection, guard)
                .map_err(|failure| failure.error)?;
            let current_size = Size {
                width: current_high.x - current_low.x,
                height: current_high.y - current_low.y,
            };
            if can_move_top {
                candidates.push(Point {
                    x: packed_top_left.x,
                    y: packed_top_left.y - current_size.height - 20.0,
                });
            }
            if can_move_left {
                candidates.push(Point {
                    x: packed_top_left.x - current_size.width - 20.0,
                    y: packed_top_left.y,
                });
            }
            if can_move_top && can_move_left {
                candidates.push(Point {
                    x: packed_top_left.x - current_size.width - 20.0,
                    y: packed_top_left.y - current_size.height - 20.0,
                });
            }
            let transaction =
                binpack_guard::PackingGeometryTransaction::capture(self, edges_placed, guard)?;
            let trial_snapshot = transaction.snapshot();
            // BinPack candidates commit through a ValidateBadState
            // transaction. Its overlap inventory covers the complete graph,
            // not only groups already packed in this hierarchy scope.
            let (existing_overlaps, existing_exact_overlaps) = transaction.overlaps();
            // Every trial below calls Node.translate only for currPacking.
            // GraphState still snapshots the whole graph once, but unchanged
            // pairs cannot become newly overlapping during this candidate.
            let (current_top_left, _) = self
                .packing_bounds_checked(&current_selection, guard)
                .map_err(|failure| failure.error)?;
            let prepared_trial = self.packing_prepare_trial_checked(
                &segment_state
                    .children
                    .iter()
                    .copied()
                    .map(Some)
                    .collect::<Vec<_>>(),
                &current_selection,
                &reference_groups(&packed_refs),
                &staging,
                &mut candidates,
                &mut candidate_seen,
                guard,
            )?;
            if trace_binpack_candidates
                && current.first().is_some_and(|node| {
                    self.nodes[node.0 as usize].tala_id == 11
                        || crate::internal::trace_env_value("TALA_TRACE_BINPACK_CURRENT")
                            .is_some_and(|target| target == self.nodes[node.0 as usize].external_id)
                })
            {
                eprintln!("BINPACK_CANDIDATE_LIST_RUST current=11 points={candidates:?}");
            }
            let mut best: Option<(f64, Point)> = None;
            for candidate in candidates.iter().copied() {
                #[cfg(feature = "diagnostic-traces")]
                crate::internal::call_counts::count!(BinpackTrial);
                guard.step()?;
                let mut trial = trial_snapshot.trial(self);
                let commit_result = trial.apply_packing_commit_checked(
                    existing_overlaps,
                    existing_exact_overlaps,
                    guard,
                    |graph, guard| {
                        graph.packing_translate_refs_checked(
                            &current_refs,
                            Point {
                                x: candidate.x - current_top_left.x,
                                y: candidate.y - current_top_left.y,
                            },
                            guard,
                        )
                    },
                );
                if matches!(&commit_result, Ok(false)) {
                    continue;
                }
                if let Err(error) = commit_result {
                    // packGuarded checks its own guard after Commit fails.
                    guard.step()?;
                    return Err(error);
                }
                if let Some(root) = scope {
                    let root_node = &trial.nodes[root.0 as usize];
                    let inner = match root_node.shape {
                        ShapeKind::Circle => {
                            let radius = root_node.rect.size.width / 2.0;
                            let offset = (radius - radius * std::f64::consts::SQRT_2 / 2.0).ceil();
                            Size {
                                width: root_node.rect.size.width - 2.0 * offset,
                                height: root_node.rect.size.height - 2.0 * offset,
                            }
                        }
                        ShapeKind::Oval => {
                            let offset = trial.shape_inside_placement(
                                root,
                                root_node.rect.size,
                                Insets::uniform(0.0),
                            );
                            Size {
                                width: root_node.rect.size.width - 2.0 * offset.x,
                                height: root_node.rect.size.height - 2.0 * offset.y,
                            }
                        }
                        ShapeKind::Diamond => Size {
                            width: root_node.rect.size.width / 2.0,
                            height: root_node.rect.size.height / 2.0,
                        },
                        ShapeKind::Cloud => {
                            let children = trial.containers.get(&Some(root)).unwrap_or_default();
                            let aspect_ratio = trial
                                .external_label_node_bounds(&children)
                                .map(|(top_left, bottom_right)| {
                                    (bottom_right.x - top_left.x) / (bottom_right.y - top_left.y)
                                })
                                .unwrap_or(1.0);
                            const WIDE_WIDTH: f64 = 0.819;
                            const WIDE_HEIGHT: f64 = 0.548;
                            const TALL_WIDTH: f64 = 0.549;
                            const TALL_HEIGHT: f64 = 0.820;
                            const SQUARE_SIDE: f64 = 0.663;
                            const WIDE_BOUNDARY: f64 = (1.0 + WIDE_WIDTH / WIDE_HEIGHT) / 2.0;
                            const TALL_BOUNDARY: f64 = (1.0 + TALL_WIDTH / TALL_HEIGHT) / 2.0;
                            let (width_factor, height_factor) = if aspect_ratio > WIDE_BOUNDARY {
                                (WIDE_WIDTH, WIDE_HEIGHT)
                            } else if aspect_ratio < TALL_BOUNDARY {
                                (TALL_WIDTH, TALL_HEIGHT)
                            } else {
                                (SQUARE_SIDE, SQUARE_SIDE)
                            };
                            Size {
                                width: root_node.rect.size.width * width_factor,
                                height: root_node.rect.size.height * height_factor,
                            }
                        }
                        _ => root_node.rect.size,
                    };
                    // Pristine BinPack moved every not-yet-packed group far
                    // off-canvas before entering the candidate loop, then
                    // built packedInContainer from packedWithCurr only. Using
                    // all direct children here lets an unseen group's stale
                    // position reject an otherwise valid candidate.
                    let mut packed_in_container = Vec::new();
                    for &n in &prepared_trial.scoring_nodes {
                        guard.step()?;
                        if trial.packing_raw_container(n.expect("non-nil packing node"))
                            == Some(binpack_traversal::PackingNodeRef::Node(root))
                        {
                            packed_in_container.push(n);
                        }
                    }
                    let (top_left, bottom_right) = trial
                        .packing_bounds_checked(&packed_in_container, guard)
                        .map_err(|f| f.error)?;
                    if bottom_right.x - top_left.x > inner.width
                        || bottom_right.y - top_left.y > inner.height
                    {
                        if trace_binpack_candidates {
                            eprintln!(
                                "BINPACK_CANDIDATE_RUST root={:?} current={} candidate={:?} reject=inner_bounds",
                                scope.map(|node| trial.nodes[node.0 as usize].tala_id),
                                trial.nodes[current[0].0 as usize].tala_id,
                                candidate
                            );
                        }
                        continue;
                    }
                }
                let mut hierarchy_intrusion = false;
                if !edges_placed {
                    let boxes = hierarchy_boxes
                        .iter()
                        .map(|&(a, b)| (Some(a), Some(b)))
                        .collect::<Vec<_>>();
                    for &n in &current_refs {
                        let (low, high) = trial
                            .packing_node_bounds_checked(Some(n), None, guard)
                            .map_err(|f| f.error)?;
                        let left =
                            Self::packing_point_in_hierarchy_checked(&boxes, Some(low), guard)?;
                        let right =
                            Self::packing_point_in_hierarchy_checked(&boxes, Some(high), guard)?;
                        if left || right {
                            hierarchy_intrusion = true;
                            break;
                        }
                    }
                }
                if hierarchy_intrusion {
                    if trace_binpack_candidates {
                        eprintln!(
                            "BINPACK_CANDIDATE_RUST root={:?} current={} candidate={:?} reject=hierarchy_boundary",
                            scope.map(|node| trial.nodes[node.0 as usize].tala_id),
                            trial.nodes[current[0].0 as usize].tala_id,
                            candidate
                        );
                    }
                    continue;
                }
                if trial.packing_blocks_routes_checked(
                    &current_selection,
                    &reference_groups(&packed_refs),
                    segment_state.segments.as_deref().unwrap_or_default(),
                    prepared_trial.packed_deltas,
                    guard,
                )? {
                    if trace_binpack_candidates {
                        eprintln!(
                            "BINPACK_CANDIDATE_RUST root={:?} current={} candidate={:?} reject=routes",
                            scope.map(|node| trial.nodes[node.0 as usize].tala_id),
                            trial.nodes[current[0].0 as usize].tala_id,
                            candidate
                        );
                    }
                    continue;
                }
                let score = trial
                    .packing_score_checked(&prepared_trial.scoring_nodes, scope, guard)
                    .map_err(|f| f.error)?;
                if crate::internal::trace_env_enabled("TALA_TRACE_BINPACK_CANDIDATE_SCORES")
                    && scope.is_none()
                    && current.first().is_some_and(|node| {
                        matches!(trial.nodes[node.0 as usize].tala_id, 7 | 26 | 27 | 30 | 31)
                    })
                {
                    let current_id = trial.nodes[current[0].0 as usize].tala_id;
                    eprintln!(
                        "BINPACK_SCORE_CANDIDATE_RUST current={} candidate=({:.17},{:.17}) score=0x{:016x}",
                        current_id,
                        candidate.x,
                        candidate.y,
                        score.to_bits()
                    );
                }
                if trace_binpack_candidates
                    && current.first().is_some_and(|node| {
                        trial.nodes[node.0 as usize].tala_id == 11
                            || crate::internal::trace_env_value("TALA_TRACE_BINPACK_CURRENT")
                                .is_some_and(|target| {
                                    target == trial.nodes[node.0 as usize].external_id
                                })
                    })
                {
                    eprintln!(
                        "BINPACK_CANDIDATE_RUST current=11 candidate=({:.17},{:.17}) score=0x{:016x} vessel_id={} vessel_cluster={:?} vessel_pending={:?} vessel_pos={:?} vessel_size={:?} current_tl={:?}",
                        candidate.x,
                        candidate.y,
                        score.to_bits(),
                        trial.nodes[27].tala_id,
                        trial.active_cluster_index(NodeId(27)),
                        trial.pending_cluster_vessel_positions,
                        trial.position(NodeId(27)),
                        trial.active_node_size(NodeId(27)),
                        current_top_left,
                    );
                }
                if score < best.map_or(f64::INFINITY, |(best_score, _)| best_score) {
                    best = Some((score, candidate));
                }
            }
            // returnIfCanceled delegates to checkCanceled, whose source
            // operation is Step (not an unconditional Finish poll).
            guard.step()?;
            let Some((best_score, candidate)) = best else {
                // Go records the failed choice and lets the common score and
                // rollback path run. Returning early skips those observable
                // trace events and leaves the enclosing scope at a different
                // lifecycle boundary.
                trace_binpack_event(
                    self,
                    "chosen",
                    scope,
                    std::slice::from_ref(&current),
                    0,
                    None,
                    f64::INFINITY,
                );
                if trace_binpack_candidates {
                    eprintln!(
                        "BINPACK_NO_CANDIDATE_RUST root={:?} current={}",
                        scope.map(|node| self.nodes[node.0 as usize].tala_id),
                        self.nodes[current[0].0 as usize].tala_id
                    );
                }
                failed = true;
                break;
            };
            trace_binpack_event(
                self,
                "chosen",
                scope,
                std::slice::from_ref(&current),
                0,
                Some(candidate),
                best_score,
            );
            if trace_binpack_candidates {
                eprintln!(
                    "BINPACK_CHOOSE_RUST root={:?} current={} candidate={:?}",
                    scope.map(|node| self.nodes[node.0 as usize].tala_id),
                    self.nodes[current[0].0 as usize].tala_id,
                    candidate
                );
            }
            {
                let mut committed = trial_snapshot.trial(self);
                let commit_result = committed.apply_packing_commit_checked(
                    existing_overlaps,
                    existing_exact_overlaps,
                    guard,
                    |graph, guard| {
                        graph.packing_translate_refs_checked(
                            &current_refs,
                            Point {
                                x: candidate.x - current_top_left.x,
                                y: candidate.y - current_top_left.y,
                            },
                            guard,
                        )?;
                        if edges_placed {
                            graph.packing_move_routes_checked(&current_refs, &staging, guard)?;
                        }
                        Ok(())
                    },
                );
                if !matches!(&commit_result, Ok(true)) {
                    guard.step()?;
                    return Err(commit_result.err().unwrap_or_else(|| {
                        limits::Error::Invalid("invalid layout candidate".into())
                    }));
                }
                committed.accept();
            }
            // Each speculative and selected operation now crosses the actual
            // per-trial synchronization and validation boundary above.
            accepted_candidates.push(candidate);
            self.packing_extend_segments_checked(
                &current_refs.iter().copied().map(Some).collect::<Vec<_>>(),
                &reference_groups(&to_pack_refs),
                scope.map(binpack_traversal::PackingNodeRef::Node),
                edges_placed,
                &mut segment_state,
                guard,
            )?;
            packed_refs.push(current_refs);
            packed.push(current);
        }

        // binpack.go:862-874 charges the post-loop visit before scoring the
        // current direct children. A late score failure must reach packAtomic's
        // rollback boundary, including when there were no candidate iterations.
        guard.step()?;
        let final_score = if failed {
            f64::INFINITY
        } else {
            let score_nodes = self
                .packing_direct_refs(scope)
                .into_iter()
                .map(Some)
                .collect::<Vec<_>>();
            self.packing_score_checked(&score_nodes, scope, guard)
                .map_err(|failure| failure.error)?
        };
        trace_binpack_event(self, "original-score", scope, &[], 0, None, original_score);
        trace_binpack_event(self, "final-score", scope, &[], 0, None, final_score);
        if trace_binpack_candidates {
            eprintln!(
                "BINPACK_SCORE_RUST root={:?} original={} final={}",
                scope.map(|node| self.nodes[node.0 as usize].tala_id),
                original_score,
                final_score
            );
        }
        if final_score > original_score {
            staging.restore_positions_checked(self, guard)?;
            // The Go implementation marks this scope rejected after
            // restoring its original geometry.  The recursive caller uses
            // that bit to preserve the same failed transaction boundary.
            failed = true;
            // Source always synchronizes both group kinds after restoring
            // rejected placements, including routed requests.
            self.packing_sync_groups_checked(owner_groups::OwnerGroupKind::Cluster, guard)?;
            self.packing_sync_groups_checked(owner_groups::OwnerGroupKind::Sequence, guard)?;
        } else if let Some(container) =
            scope.filter(|container| self.nodes[container.0 as usize].cluster.is_none())
        {
            // binpack.go:891-904 retains descendants absent from toPack as
            // well. The actual saved inventory is consumed by rollback.
            let descendants = self.packing_descendants_checked(
                binpack_traversal::PackingNodeRef::Node(container),
                guard,
            )?;
            for child in descendants {
                guard.step()?;
                staging.record(self, child);
            }
            let old_position = self.position(container).unwrap_or_default();
            let old_size = self.nodes[container.0 as usize].rect.size;
            self.wrap_bin_packed_container_checked(container, guard)?;
            use binpack_routed_container::Decision;
            let decision = if edges_placed {
                binpack_routed_container::decision(
                    Some(self),
                    Some(container),
                    Some(Rect {
                        origin: old_position,
                        size: old_size,
                    }),
                    &graph_incident.iter().copied().map(Some).collect::<Vec<_>>(),
                    guard,
                )
                .map_err(|failure| failure.error)?
            } else {
                Decision::Defer
            };
            let routed_container_needs_original = decision == Decision::Keep;
            if crate::internal::trace_env_enabled("TALA_TRACE_BINPACK_DETAIL") {
                eprintln!(
                    "BINPACK_WRAP_ROUTE_DECISION_RUST node={} edges_placed={} edge_count={} needs_original={} old_pos={:?} old_size={:?} new_pos={:?} new_size={:?}",
                    self.nodes[container.0 as usize].tala_id,
                    edges_placed,
                    self.nodes[container.0 as usize].edges.len(),
                    routed_container_needs_original,
                    old_position,
                    old_size,
                    self.position(container),
                    self.nodes[container.0 as usize].rect.size
                );
            }
            if routed_container_needs_original {
                self.packing_write_root_position(container, old_position);
                self.nodes[container.0 as usize].rect.size = old_size;
            } else if decision == Decision::Defer {
                let mut position = self.position(container).unwrap_or(old_position);
                if !can_move_left {
                    position.x = old_position.x;
                }
                if !can_move_top {
                    position.y = old_position.y;
                }
                self.packing_write_root_position(container, position);
                if !can_move_right {
                    self.nodes[container.0 as usize].rect.size.width = old_size.width;
                }
                if !can_move_bottom {
                    self.nodes[container.0 as usize].rect.size.height = old_size.height;
                }
                if matches!(
                    self.nodes[container.0 as usize].shape,
                    ShapeKind::Circle | ShapeKind::RealSquare
                ) {
                    let size = self.nodes[container.0 as usize].rect.size;
                    let side = routing::channel_solver::maximum(size.width, size.height);
                    self.nodes[container.0 as usize].rect.size = Size {
                        width: side,
                        height: side,
                    };
                }
            }
            // Source computes child coverage first but still validates the
            // root even when a child does not fit. binpack.go dereferences
            // root.TopLeft here (InnerBox), so a nil root position panics.
            let inner = self
                .shape_inner_box(container)
                .expect("runtime error: invalid memory address or nil pointer dereference");
            let mut children_fit = true;
            for child in self.packing_direct_refs(Some(container)) {
                guard.step()?;
                let (p, size, _) = self.packing_bounds_geometry(child);
                let p = p.expect("positioned packed child");
                if !(p.x >= inner.origin.x
                    && p.y >= inner.origin.y
                    && p.x + size.width <= inner.origin.x + inner.size.width
                    && p.y + size.height <= inner.origin.y + inner.size.height)
                {
                    children_fit = false;
                    break;
                }
            }
            let wrapped_bad_state = self.packing_standalone_bad_state_checked(
                binpack_traversal::PackingNodeRef::Node(container),
                guard,
            )?;
            if !children_fit || wrapped_bad_state {
                staging.restore_positions_checked(self, guard)?;
                self.packing_write_root_position(container, old_position);
                self.nodes[container.0 as usize].rect.size = old_size;
                failed = true;
            }
        }
        if trace_binpack_scope {
            eprint!(
                "BINPACK_SCOPE_RUST_DONE root={:?}",
                scope.map(|node| self.nodes[node.0 as usize].tala_id)
            );
            for node in self.containers.get(&scope).into_iter().flatten() {
                let child = &self.nodes[node.0 as usize];
                eprint!(
                    " {}@{:?}:{}x{}",
                    child.tala_id, child.position, child.rect.size.width, child.rect.size.height
                );
            }
            eprintln!();
        }
        if crate::internal::trace_env_enabled("TALA_TRACE_BINPACK_SCOPE_STATE") {
            for cluster in &self.clusters {
                eprintln!(
                    "BINPACK_SCOPE_STATE_RUST root={:?} failed={} vessel={} member6={:?} active={}",
                    scope.map(|node| self.nodes[node.0 as usize].tala_id),
                    failed,
                    cluster.vessel_tala_id,
                    cluster
                        .members
                        .iter()
                        .find(|member| self.nodes[member.0 as usize].tala_id == 6)
                        .and_then(|member| self.position(*member)),
                    self.cluster_is_active(cluster),
                );
            }
        }
        if failed {
            staging.restore_routes_checked(self, guard)?;
        }
        guard.finish()?;
        Ok(failed)
    }

    pub(super) fn bin_pack_recursive_scope(
        &mut self,
        scope: Option<NodeId>,
        skip_collapsed_aggregate_children: bool,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<bool, limits::Error> {
        // Recovered Graph.BinPack is a post-order DFS over g.Containers[root],
        // not a global depth sort. The distinction is observable when routed
        // edges cross sibling branches: an earlier sibling may translate its
        // routes before the later sibling constructs packedSegments.
        guard.step()?;
        let (edges_placed, incident_edges) = self
            .bin_pack_complete_routes(scope, guard)
            .map_err(|failure| failure.error)?;
        let children = self.containers.get(&scope).unwrap_or_default();
        let mut rejected = false;
        for child in children {
            guard.step()?;
            if skip_collapsed_aggregate_children
                && (self.nodes[child.0 as usize].cluster.is_some()
                    || self.nodes[child.0 as usize].sequence.is_some())
            {
                continue;
            }
            if self.nodes[child.0 as usize].is_container {
                rejected |= self.bin_pack_recursive_scope(
                    Some(child),
                    skip_collapsed_aggregate_children,
                    guard,
                )?;
            }
        }
        rejected |= self.bin_pack_scope_guarded_with_incidents(
            scope,
            edges_placed,
            &incident_edges,
            guard,
        )?;
        Ok(rejected)
    }

    pub(super) fn bin_pack_guarded(
        &mut self,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<(), limits::Error> {
        if crate::internal::trace_env_enabled("TALA_TRACE_BINPACK_NODE_ORDER") {
            eprintln!(
                "BINPACK_NODE_ORDER_RUST {:?}",
                self.node_order
                    .iter()
                    .map(|node| self.nodes[node.0 as usize].tala_id)
                    .collect::<Vec<_>>()
            );
            for cluster in &self.clusters {
                eprintln!(
                    "BINPACK_CLUSTER_ORDER_RUST vessel={} active={} members={:?}",
                    cluster.vessel_tala_id,
                    self.cluster_is_active(cluster),
                    cluster
                        .members
                        .iter()
                        .map(|member| (
                            self.nodes[member.0 as usize].tala_id,
                            self.node_order_contains(*member)
                        ))
                        .collect::<Vec<_>>()
                );
            }
        }
        // Pristine Graph.BinPack derives this from the current graph on every
        // invocation (binpack.go:48-54); it is not synonymous with the first
        // or second pipeline pass. In particular, both passes are false for
        // an edgeless graph.
        // AddClusters/AddSequences replace every member pointer with one
        // temporary vessel for the first (unrouted) pass.  The owner arena
        // still retains the original member containers, so a raw recursive
        // walk would pack hidden scopes before their vessel.  Cleanup restores
        // those members before the routed pass; detect that lifecycle from the
        // active node order rather than from edge presence (edgeless diagrams
        // run both passes with `edges_placed == false`).
        let aggregates_collapsed = self.clusters.iter().any(|cluster| {
            cluster.members.len() > 1
                && self
                    .node_order
                    .iter()
                    .filter(|node| cluster.members.contains(node))
                    .count()
                    < cluster.members.len()
        }) || self.sequences.iter().any(|sequence| {
            sequence.members.len() > 1
                && self
                    .node_order
                    .iter()
                    .filter(|node| sequence.members.contains(node))
                    .count()
                    < sequence.members.len()
        });
        let _rejected = self.bin_pack_recursive_scope(None, aggregates_collapsed, guard)?;
        #[cfg(feature = "diagnostic-traces")]
        if crate::internal::trace_env_enabled("TALA_TRACE_BINPACK_AGG") {
            let edges_placed = self.member_edges().any(|edge| !edge.route.is_empty());
            eprintln!(
                "BINPACK_AGG_RUST edges_placed={} clusters={} sized_abductions={} ext_aggs={} ext_containers={} cluster_distance={} adjacent_overrides={}",
                edges_placed,
                self.clusters.len(),
                self.sized_edge_abductions.len(),
                self.transaction_external_aggregate_children.len(),
                self.transaction_external_containers.len(),
                self.sized_cluster_distance_boxes.len(),
                self.sized_adjacent_overrides.len(),
            );
        }
        self.refresh_placement_components();
        Ok(())
    }
}

#[cfg(test)]
mod bounds_contract_tests {
    use super::*;

    #[test]
    fn candidate_bounds_apply_modifiers_before_outside_label_checks() {
        let mut input = Graph::default();
        input.add_node(crate::Node::new(
            "hex",
            Size {
                width: 100.0,
                height: 80.0,
            },
        ));
        let mut graph = ArenaGraph::from_input(&input);
        graph.set_position(NodeId(0), Point::default());
        let node = &mut graph.nodes[0];
        node.shape = ShapeKind::Hexagon;
        node.is_3d = true;
        node.label_size = Some(Size {
            width: 39.4,
            height: 13.3,
        });
        node.label_position = LabelPosition::OutsideRightMiddle;
        // The label starts at 105, within the modifier-expanded right edge115.
        // Source bounds therefore do not expand through the label's far edge.
        assert_eq!(
            graph.bin_pack_unanchored_bounds(&[NodeId(0)]),
            Some((Point { x: 0.0, y: -7.5 }, Point { x: 115.0, y: 80.0 }))
        );
    }

    #[test]
    fn hierarchy_capture_keeps_geometry_and_membership_across_trial_changes() {
        let mut input = Graph::default();
        let node = input.add_node(crate::Node::new(
            "hierarchy",
            Size {
                width: 100.0,
                height: 80.0,
            },
        ));
        let mut graph = ArenaGraph::from_input(&input);
        graph.set_position(node, Point { x: 10.5, y: 20.25 });
        graph.assign_hierarchy_member(node, 1, None, 0, 1);
        let hierarchy = graph.nodes[0].hierarchy.clone();
        let groups = vec![vec![node]];
        let captured = graph.bin_pack_hierarchy_boxes(&groups);
        assert_eq!(
            captured,
            vec![(Point { x: 10.5, y: 20.25 }, Point { x: 111.0, y: 100.0 })]
        );
        graph.set_position(
            node,
            Point {
                x: 1010.5,
                y: 2020.25,
            },
        );
        graph.nodes[0].hierarchy = None;
        assert!(ArenaGraph::bin_pack_point_in_hierarchy(
            &captured,
            Point { x: 10.5, y: 20.25 }
        ));
        assert!(!ArenaGraph::bin_pack_point_in_hierarchy(
            &captured,
            Point {
                x: 1010.5,
                y: 2020.25
            }
        ));
        assert!(graph.bin_pack_hierarchy_boxes(&groups).is_empty());
        graph.nodes[0].hierarchy = hierarchy;
        let rebuilt = graph.bin_pack_hierarchy_boxes(&groups);
        assert!(ArenaGraph::bin_pack_point_in_hierarchy(
            &rebuilt,
            Point {
                x: 1010.5,
                y: 2020.25
            }
        ));
        assert!(!ArenaGraph::bin_pack_point_in_hierarchy(
            &rebuilt,
            Point { x: 10.5, y: 20.25 }
        ));
    }

    #[test]
    fn hierarchy_capture_refreshes_for_the_next_packed_group() {
        let mut input = Graph::default();
        for name in ["first", "second"] {
            input.add_node(crate::Node::new(
                name,
                Size {
                    width: 100.0,
                    height: 80.0,
                },
            ));
        }
        let mut graph = ArenaGraph::from_input(&input);
        for (index, x) in [(0, 0.0), (1, 500.0)] {
            graph.set_position(NodeId(index), Point { x, y: 0.0 });
            graph.assign_hierarchy_member(
                NodeId((index as usize) as u32),
                index as usize,
                None,
                0,
                1,
            );
        }
        let mut packed = vec![vec![NodeId(0)]];
        let captured = graph.bin_pack_hierarchy_boxes(&packed);
        packed.push(vec![NodeId(1)]);
        assert!(!ArenaGraph::bin_pack_point_in_hierarchy(
            &captured,
            Point { x: 500.0, y: 0.0 }
        ));
        let next_iteration = graph.bin_pack_hierarchy_boxes(&packed);
        assert!(ArenaGraph::bin_pack_point_in_hierarchy(
            &next_iteration,
            Point { x: 500.0, y: 0.0 }
        ));
    }

    #[test]
    fn packing_bounds_distinguish_compact_vessels_from_retained_members() {
        let mut input = Graph::default();
        for name in ["first", "second", "vessel"] {
            input.add_node(crate::Node::new(
                name,
                Size {
                    width: 100.0,
                    height: 80.0,
                },
            ));
        }
        input.tala_id_overrides = vec![Some(1), Some(2), Some(99)];
        let mut graph = ArenaGraph::from_input(&input);
        let first = NodeId(0);
        let second = NodeId(1);
        let vessel = NodeId(2);
        graph.set_position(first, Point { x: 10.0, y: 20.0 });
        graph.set_position(second, Point { x: 200.0, y: 300.0 });
        graph.set_position(
            vessel,
            Point {
                x: 1000.0,
                y: 2000.0,
            },
        );
        graph.clusters.push(ClusterState {
            members: vec![first, second],
            arrangement: ClusterArrangement::Row,
            desired_arrangement: ClusterArrangement::Row,
            padding: 10.0,
            vessel_tala_id: 99,
            fixed_size: false,
        });
        graph.cluster_vessel_sizes.push(Size {
            width: 500.0,
            height: 500.0,
        });
        graph.pending_cluster_vessel_positions.insert(
            0,
            Point {
                x: 1000.0,
                y: 2000.0,
            },
        );
        graph.nodes[0].cluster = Some(0);
        graph.nodes[1].cluster = Some(0);
        graph.node_order = vec![first];
        graph.invalidate_node_order_membership();
        assert_eq!(
            graph.bin_pack_hierarchy_intrusion_bounds(first),
            Some((
                Point {
                    x: 1000.0,
                    y: 2000.0
                },
                Point {
                    x: 1500.0,
                    y: 2500.0
                }
            ))
        );
        assert_eq!(
            graph.bin_pack_hierarchy_intrusion_bounds(second),
            Some((Point { x: 200.0, y: 300.0 }, Point { x: 300.0, y: 380.0 }))
        );
        // With a concrete current vessel pointer, the first member no longer
        // doubles as the vessel's geometry in a selected member slice.
        graph.node_order.push(vessel);
        graph.invalidate_node_order_membership();
        let member_bounds = Some((Point { x: 10.0, y: 20.0 }, Point { x: 110.0, y: 100.0 }));
        assert_eq!(
            graph.bin_pack_hierarchy_intrusion_bounds(first),
            member_bounds
        );
        assert_eq!(graph.bin_pack_unanchored_bounds(&[first]), member_bounds);
    }

    #[test]
    fn packing_spacing_and_hierarchy_boxes_keep_fixed_nodes_at_visible_coordinates() {
        let mut input = Graph::default();
        let node = input.add_node(crate::Node::new(
            "fixed",
            Size {
                width: 100.0,
                height: 80.0,
            },
        ));
        let mut graph = ArenaGraph::from_input(&input);
        graph.set_position(node, Point { x: 200.0, y: 100.0 });
        graph.nodes[node.0 as usize].fixed_top_left = Some(Point { x: 200.0, y: 100.0 });
        graph.assign_hierarchy_member(node, 0, None, 0, 1);
        let groups = vec![vec![node]];
        assert_eq!(graph.bin_pack_smallest_deltas(&groups), (100.0, 80.0));
        let boxes = graph.bin_pack_hierarchy_boxes(&groups);
        assert!(!ArenaGraph::bin_pack_point_in_hierarchy(
            &boxes,
            Point::default()
        ));
        assert!(ArenaGraph::bin_pack_point_in_hierarchy(
            &boxes,
            Point { x: 200.0, y: 100.0 }
        ));
        assert!(ArenaGraph::bin_pack_point_in_hierarchy(
            &boxes,
            Point { x: 300.0, y: 180.0 }
        ));
        assert!(!ArenaGraph::bin_pack_point_in_hierarchy(
            &boxes,
            Point { x: 301.0, y: 180.0 }
        ));
        // Score/escape bounds deliberately retain the fixed-origin override.
        assert_eq!(
            graph.bin_pack_bounding_box(&[node]).unwrap().0,
            Point::default()
        );
    }

    #[test]
    fn packing_intrusion_corners_include_modifiers_and_loops_without_label_or_icon_space() {
        let mut input = Graph::default();
        let id = input.add_node(crate::Node::new(
            "decorated",
            Size {
                width: 100.0,
                height: 80.0,
            },
        ));
        let mut graph = ArenaGraph::from_input(&input);
        graph.set_position(id, Point { x: 10.5, y: 20.5 });
        let node = &mut graph.nodes[id.0 as usize];
        node.shape = ShapeKind::Hexagon;
        node.is_3d = true;
        node.label_size = Some(Size {
            width: 100.0,
            height: 80.0,
        });
        node.label_position = LabelPosition::OutsideLeftMiddle;
        node.icon_position = Some(LabelPosition::OutsideTopCenter);
        // Source geo.Top/Right/Bottom/Left cardinal offsets.
        node.loop_offsets = Some(
            [(4, 2.0), (5, 4.0), (6, 2.0), (7, 3.5)]
                .into_iter()
                .collect(),
        );
        assert_eq!(
            graph.bin_pack_hierarchy_intrusion_bounds(id),
            Some((Point { x: 7.0, y: 11.0 }, Point { x: 130.0, y: 103.0 }))
        );
        assert_ne!(
            graph.bin_pack_hierarchy_intrusion_bounds(id),
            graph.bin_pack_unanchored_bounds(&[id])
        );
    }

    #[test]
    fn fixed_descendant_is_checked_against_staged_transaction_position() {
        let mut input = Graph::default();
        let parent = input.add_node(crate::Node::new(
            "parent",
            Size {
                width: 300.0,
                height: 300.0,
            },
        ));
        let child = input.add_node(crate::Node::new(
            "child",
            Size {
                width: 50.0,
                height: 50.0,
            },
        ));
        input.nodes[child.0 as usize].parent = Some(parent);
        let mut graph = ArenaGraph::from_input(&input);
        graph.nodes[child.0 as usize].fixed_top_left = Some(Point { x: 20.0, y: 30.0 });
        graph.set_position(
            parent,
            Point {
                x: 100_000.0,
                y: 100_000.0,
            },
        );
        graph.set_position(
            child,
            Point {
                x: 100_080.0,
                y: 100_090.0,
            },
        );
        let before = graph.clone();
        assert!(graph.bin_pack_fixed_positions_unchanged(&before));
        graph.translate_bin_pack_group(
            &[parent, child],
            None,
            Point {
                x: -100_000.0,
                y: -100_000.0,
            },
        );
        assert!(!graph.bin_pack_fixed_positions_unchanged(&before));
        // Fixed-origin clearance alone does not reject this whole-group move.
        assert!(!graph.bin_pack_group_past_fixed_origin(&[parent, child]));
        graph.set_position(child, before.position(child).unwrap());
        assert!(graph.bin_pack_fixed_positions_unchanged(&before));
    }

    #[test]
    fn fixed_position_check_preserves_nullable_snapshot_and_cluster_rules() {
        let mut input = Graph::default();
        input.add_node(crate::Node::new(
            "fixed",
            Size {
                width: 50.0,
                height: 50.0,
            },
        ));
        let mut graph = ArenaGraph::from_input(&input);
        graph.nodes[0].fixed_top_left = Some(Point { x: 20.0, y: 30.0 });
        let before = graph.clone();
        assert!(graph.bin_pack_fixed_positions_unchanged(&before));
        graph.set_position(NodeId(0), Point::default());
        assert!(graph.bin_pack_fixed_positions_unchanged(&before));
        graph.set_position(NodeId(0), Point { x: 1.0, y: 0.0 });
        assert!(!graph.bin_pack_fixed_positions_unchanged(&before));
        graph.clusters.push(ClusterState {
            members: vec![NodeId(0)],
            arrangement: ClusterArrangement::Row,
            desired_arrangement: ClusterArrangement::Row,
            padding: 0.0,
            vessel_tala_id: 2,
            fixed_size: false,
        });
        graph.nodes[0].cluster = Some(0);
        assert!(graph.bin_pack_fixed_positions_unchanged(&before));
    }

    #[test]
    fn wrapping_preserves_source_absolute_rounding_and_translation_order() {
        let mut input = Graph::default();
        let parent = input.add_node(crate::Node::new(
            "parent",
            Size {
                width: 520.0,
                height: 521.0,
            },
        ));
        let mut leaf = crate::Node::new(
            "fixed",
            Size {
                width: 100.0,
                height: 80.0,
            },
        );
        leaf.parent = Some(parent);
        let child = input.add_node(leaf);
        let mut graph = ArenaGraph::from_input(&input);
        graph.nodes[parent.0 as usize].shape = ShapeKind::Square;
        graph.nodes[child.0 as usize].fixed_top_left = Some(Point { x: 300.4, y: 200.6 });
        graph.set_position(
            parent,
            Point {
                x: 0.0,
                y: f64::from_bits(0x3d6c000000000000),
            },
        );
        graph.set_position(child, Point { x: 360.4, y: 260.6 });
        graph.wrap_bin_packed_container(parent);
        // Source guard.go:WrapChildren retains the prior origin, adds the
        // 2^-45 delta from its fixed bound, and never translates the child.
        assert_eq!(
            graph.position(parent).unwrap().y.to_bits(),
            0x3d6d000000000000
        );
        assert_eq!(graph.position(parent).unwrap().x, 0.0);
        assert_eq!(
            graph.nodes[parent.0 as usize].rect.size,
            Size {
                width: 520.0,
                height: 401.0
            }
        );
        assert_eq!(graph.position(child), Some(Point { x: 360.4, y: 260.6 }));
    }
}

#[cfg(test)]
mod live_fixed_origin_tests {
    use super::*;

    #[test]
    fn fixed_origin_reads_current_shared_point_after_routing() {
        let mut input = Graph::default();
        let mut fixed = crate::Node::new(
            "fixed",
            Size {
                width: 10.0,
                height: 10.0,
            },
        );
        fixed.locked_position = Some(Point { x: 10.0, y: 20.0 });
        let fixed = input.add_node(fixed);
        let moving = input.add_node(crate::Node::new(
            "moving",
            Size {
                width: 10.0,
                height: 10.0,
            },
        ));
        let mut graph = ArenaGraph::from_input(&input);
        graph.set_position(fixed, Point { x: 100.0, y: 100.0 });
        graph.materialize_owner_node_point(fixed).unwrap();
        graph.set_position(fixed, Point { x: 300.0, y: 400.0 });
        graph.set_position(moving, Point { x: 250.0, y: 400.0 });
        assert!(graph.bin_pack_group_past_fixed_origin(&[moving]));
        let point = graph.owner_node_point(fixed).unwrap();
        graph
            .write_owner_point(&point, Point { x: 200.0, y: 300.0 })
            .unwrap();
        assert!(!graph.bin_pack_group_past_fixed_origin(&[moving]));
    }
}
