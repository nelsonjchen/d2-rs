// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Recursive hierarchy placement and copy-back.
//!
//! Child scopes are materialized independently, placed, and then reconciled
//! into their owning graph while retaining stable node and edge identities.

use super::*;

impl Pipeline {
    #[cfg(feature = "diagnostic-traces")]
    fn trace_hierarchy_after_fit_state(&self, container: NodeId, phase: &str) {
        let container_tala_id = self.graph.nodes[container.0 as usize].tala_id;
        if !crate::internal::trace_env_value("D2RS_TRACE_HIERARCHY_AFTER_FIT").is_some_and(
            |target| target == "all" || target.parse::<i64>().ok() == Some(container_tala_id),
        ) {
            return;
        }
        let mut nodes = self.graph.active_descendants(container);
        nodes.push(container);
        nodes.sort_by_key(|node| self.graph.nodes[node.0 as usize].tala_id);
        let state = nodes
            .into_iter()
            .map(|node| {
                let node = &self.graph.nodes[node.0 as usize];
                (node.tala_id, node.position, node.rect.size)
            })
            .collect::<Vec<_>>();
        eprintln!(
            "D2RS_HIERARCHY_AFTER_FIT phase={phase} container={container_tala_id} nodes={state:?}"
        );
    }

    #[cfg(not(feature = "diagnostic-traces"))]
    #[inline(always)]
    fn trace_hierarchy_after_fit_state(&self, _container: NodeId, _phase: &str) {}

    /// Source optimizeCluster commits scalars on the same Cluster pointer
    /// later consumed by SyncGeometry (cluster_optimization.go:188-218).
    pub(super) fn publish_accepted_cluster_policies(
        &mut self,
        arrangements: &BTreeMap<usize, ClusterArrangement>,
        desired: &BTreeMap<usize, ClusterArrangement>,
        placed: &ArenaGraph,
    ) -> Result<(), crate::LayoutError> {
        for index in arrangements
            .keys()
            .chain(desired.keys())
            .copied()
            .collect::<BTreeSet<_>>()
        {
            let cluster = &mut self.graph.clusters[index];
            if let Some(&value) = arrangements.get(&index) {
                cluster.arrangement = value;
            }
            if let Some(&value) = desired.get(&index) {
                cluster.desired_arrangement = value;
            }
            if let Some(layout) = placed
                .transaction_external_cluster_layouts
                .get(&cluster.vessel_tala_id)
            {
                cluster.padding = layout.padding;
            }
            let arrangement = cluster.arrangement;
            let desired_arrangement = cluster.desired_arrangement;
            let padding = cluster.padding;
            let Some(vessel) = self
                .graph
                .topology_vessel_node(TopologyVessel::Cluster(index))
            else {
                continue;
            };
            let group = self
                .graph
                .owner_graph_group_entry(
                    OwnerGraphId(0),
                    owner_groups::OwnerGroupKind::Cluster,
                    Some(vessel),
                )
                .map_err(|e| crate::LayoutError::PlacementTransaction(e.to_string()))?
                .flatten()
                .expect("bound cluster has its original graph entry");
            let mut policy = self
                .graph
                .owner_cluster_policy(group)
                .map_err(|e| crate::LayoutError::PlacementTransaction(e.to_string()))?;
            policy.arrangement = match arrangement {
                ClusterArrangement::Row => "Row",
                ClusterArrangement::Column => "Column",
            }
            .to_owned();
            policy.desired_arrangement = match desired_arrangement {
                ClusterArrangement::Row => "Row",
                ClusterArrangement::Column => "Column",
            }
            .to_owned();
            policy.padding = padding;
            self.graph
                .set_owner_cluster_policy(group, policy)
                .map_err(|e| crate::LayoutError::PlacementTransaction(e.to_string()))?;
        }
        Ok(())
    }

    fn copy_tree_edge_label_placements(&mut self, placement: &PlacementScope, placed: &ArenaGraph) {
        for (&original_edge, &placed_edge) in &placement.tree_edge_old_to_new {
            let Some(placed_label) = placed.edge_records[placed_edge.0 as usize].label.as_ref()
            else {
                continue;
            };
            let Some(original_label) = self.graph.edge_records[original_edge.0 as usize]
                .label
                .as_mut()
            else {
                continue;
            };
            original_label.position = placed_label.position;
            original_label.percentage = placed_label.percentage;
        }
    }

    pub(super) fn publish_placed_tree_orientations(&mut self, placed: &ArenaGraph) {
        // CopyEntitiesFrom shares Tree pointers between every temporary graph
        // and the owner. Rust gives a retained tree node both an ordinary
        // projection and a separate auxiliary Tree projection, so publish the
        // only field mirrorAxes mutates by the pointer's durable TALA ID.
        let placed_orientation_by_tala = placed
            .tree_routing_nodes
            .iter()
            .map(|(local, state)| (placed.nodes[local.0 as usize].tala_id, state.orientation))
            .collect::<BTreeMap<_, _>>();
        let stable_tala_ids = self
            .graph
            .nodes
            .iter()
            .map(|node| node.tala_id)
            .collect::<Vec<_>>();
        let mut assigned_orientations = Vec::new();
        for (&stable, state) in self.graph.tree_routing_nodes.iter_mut() {
            let tala_id = stable_tala_ids[stable.0 as usize];
            if let Some(&orientation) = placed_orientation_by_tala.get(&tala_id) {
                if crate::internal::trace_env_enabled("TALA_TRACE_TREE_ORIENT")
                    && matches!(tala_id, 1171112347 | 1154334728 | 1271778061 | 1187889966)
                {
                    eprintln!(
                        "TREE_ORIENT_PUBLISH_RUST tala={} from={:?} to={:?}",
                        tala_id, state.orientation, orientation
                    );
                }
                state.orientation = orientation;
                assigned_orientations.push((stable, orientation));
            }
        }
        for (node, orientation) in assigned_orientations {
            self.graph.publish_tree_orientation(node, orientation);
        }
    }

    /// Preserve live nested vessels when publishing combined root snapshots.
    pub(super) fn publish_combined_root_projection(&mut self, snapshot: &ArenaNode) {
        let Some(position) = snapshot.position else {
            return;
        };
        let Some(owner_index) = self
            .graph
            .nodes
            .iter()
            .position(|node| node.tala_id == snapshot.tala_id)
        else {
            return;
        };
        let owner_node = NodeId(owner_index as u32);
        let owner_parent_position = self.graph.nodes[owner_index]
            .container
            .and_then(|parent| self.graph.position(parent));
        #[cfg(feature = "diagnostic-traces")]
        if crate::internal::trace_env_enabled("D2RS_TRACE_ROOT_PROJECTION_BINDING") {
            eprintln!(
                "ROOT_PROJECTION_BINDING_RUST tala={} canonical={} parent={:?} binding={:?} live={:?} snapshot={:?} pending={:?}",
                snapshot.tala_id,
                owner_index,
                owner_parent_position,
                self.graph.topology_node_vessel(owner_node),
                self.graph.position(owner_node),
                position,
                self.graph.pending_cluster_vessel_positions,
            );
        }
        if owner_parent_position.is_some() {
            match self.graph.topology_node_vessel(owner_node) {
                Some(TopologyVessel::Cluster(index)) => {
                    // Parent MoveWithChildren already moved this actual
                    // vessel (node.go2136–2143). The flattened snapshot still
                    // belongs to the earlier local frame. Preserve that live
                    // pointee and make the numerical vessel cache observe it.
                    if let Some(live) = self.graph.position(owner_node) {
                        self.graph
                            .pending_cluster_vessel_positions
                            .insert(index, live);
                    }
                    return;
                }
                Some(TopologyVessel::Sequence(_)) => {}
                None => return,
            }
        }
        // A directly combined vessel, or the independently combined sequence
        // projection, still owns this publication boundary.
        Self::publish_induced_owner_point(&mut self.graph, owner_node, Some(position));
    }

    /// Publish a nested scope's cloned descendant boxes into the stable arena.
    ///
    /// Go's temporary placement graphs retain the same `*Node` pointers as
    /// their owner. Initializing/syncing a nested container therefore changes
    /// the descendant pointers before the carrier is copied back. Rust scopes
    /// intentionally clone graph nodes, so copying only the carrier and then
    /// translating its stale stable subtree applies the owner's old frame a
    /// second time. Reconstruct the local parent chain from the scope's
    /// external child snapshots and publish those boxes directly.
    pub(super) fn publish_scope_subtree_geometry(
        &mut self,
        scope: &ArenaGraph,
        local: NodeId,
        stable_child: NodeId,
        target: Point,
    ) -> bool {
        // AddCluster introduces a temporary vessel whose first retained
        // member is the stable owner in this arena.  The vessel projection
        // can have no position after the induced scope is flattened even
        // though the member does; use that member as the local frame so its
        // ordinary descendants retain their fitted in-vessel offsets.
        let stable_tala_id = self.graph.nodes[stable_child.0 as usize].tala_id;
        let local_uses_external_stable_frame = scope.nodes[local.0 as usize].tala_id
            != stable_tala_id
            && !scope
                .nodes
                .iter()
                .any(|node| node.tala_id == stable_tala_id && node.position.is_some());
        let local = if !local_uses_external_stable_frame
            && scope.nodes[local.0 as usize].tala_id != stable_tala_id
        {
            scope
                .nodes
                .iter()
                .position(|node| node.tala_id == stable_tala_id && node.position.is_some())
                .map(|index| NodeId(index as u32))
                .unwrap_or(local)
        } else {
            local
        };
        let local_tala_id = if local_uses_external_stable_frame {
            stable_tala_id
        } else {
            scope.nodes[local.0 as usize].tala_id
        };
        let trace_subtree_publish = crate::internal::trace_env_value("TALA_TRACE_SUBTREE_PUBLISH")
            .is_some_and(|target| {
                target == "all"
                    || target.split(',').any(|id| {
                        id.trim().parse::<i64>().ok() == Some(local_tala_id)
                            || id.trim().parse::<i64>().ok() == Some(stable_tala_id)
                    })
            });
        if trace_subtree_publish {
            eprintln!(
                "SUBTREE_PUBLISH_RUST begin local={} stable={} target={:?} active_descendants={:?}",
                local_tala_id,
                stable_tala_id,
                target,
                self.graph
                    .active_descendants(stable_child)
                    .iter()
                    .map(|node| self.graph.nodes[node.0 as usize].tala_id)
                    .collect::<Vec<_>>()
            );
            eprintln!(
                "SUBTREE_PUBLISH_RUST maps ordinary={:?} aggregate={:?}",
                scope
                    .transaction_external_container_children
                    .keys()
                    .collect::<Vec<_>>(),
                scope
                    .transaction_external_aggregate_children
                    .keys()
                    .collect::<Vec<_>>()
            );
            for parent in [3677471320_i64, 2305561650894865143, 3946625030, 2043751575] {
                eprintln!(
                    "SUBTREE_PUBLISH_RUST map parent={} ordinary={:?} aggregate={:?}",
                    parent,
                    scope
                        .transaction_external_container_children
                        .get(&parent)
                        .map(|nodes| nodes
                            .iter()
                            .map(|n| (n.tala_id, n.position))
                            .collect::<Vec<_>>()),
                    scope
                        .transaction_external_aggregate_children
                        .get(&parent)
                        .map(|nodes| nodes
                            .iter()
                            .map(|n| (n.tala_id, n.position))
                            .collect::<Vec<_>>())
                );
            }
        }
        let active_descendants = self
            .graph
            .active_descendants(stable_child)
            .into_iter()
            .map(|node| (self.graph.nodes[node.0 as usize].tala_id, node))
            .collect::<BTreeMap<_, _>>();
        if active_descendants.is_empty() {
            return false;
        }

        let mut queue = vec![(local_tala_id, target)];
        let mut visited = BTreeSet::new();
        let mut published = false;
        while let Some((parent_tala_id, parent_target)) = queue.pop() {
            if !visited.insert(parent_tala_id) {
                continue;
            }
            let parent_scope_position =
                if local_uses_external_stable_frame && parent_tala_id == local_tala_id {
                    Some(target)
                } else if parent_tala_id == local_tala_id {
                    scope.position(local)
                } else {
                    scope.projected_node_position_by_tala(parent_tala_id)
                };
            let Some(parent_scope_position) = parent_scope_position else {
                continue;
            };
            let mut children = Vec::new();
            if let Some(ordinary) = scope
                .transaction_external_container_children
                .get(&parent_tala_id)
            {
                children.extend(ordinary.iter());
            }
            if let Some(aggregate) = scope
                .transaction_external_aggregate_children
                .get(&parent_tala_id)
            {
                children.extend(aggregate.iter());
            }
            for child in children {
                let Some(scope_child_position) = child.position else {
                    continue;
                };
                let child_target = Point {
                    x: parent_target.x + scope_child_position.x - parent_scope_position.x,
                    y: parent_target.y + scope_child_position.y - parent_scope_position.y,
                };
                let stable_child = active_descendants.contains_key(&child.tala_id)
                    || self
                        .graph
                        .nodes
                        .iter()
                        .any(|node| node.tala_id == child.tala_id);
                if trace_subtree_publish {
                    eprintln!(
                        "SUBTREE_PUBLISH_RUST child={} parent={} scope_pos={:?} parent_scope={:?} parent_target={:?} child_target={:?}",
                        child.tala_id,
                        parent_tala_id,
                        child.position,
                        parent_scope_position,
                        parent_target,
                        child_target
                    );
                }
                if stable_child {
                    self.graph
                        .set_projected_node_position(child.tala_id, child_target);
                    // The vessel is a distinct shared node in Go. Its
                    // position must accompany its published descendants,
                    // so a later mirrored-offset replay starts from the
                    // same frame instead of translating them twice.
                    if let Some(index) = self
                        .graph
                        .clusters
                        .iter()
                        .position(|cluster| cluster.vessel_tala_id == child.tala_id)
                    {
                        self.graph
                            .pending_cluster_vessel_positions
                            .insert(index, child_target);
                    }
                    published = true;
                }
                // Go's moveNodeWithChildren walks every child in the current
                // Containers map, including cluster members hidden from the
                // active-node traversal. Such a member may have no active
                // NodeId in this scope while its descendants remain stable
                // arena nodes. Follow its child maps even when the member
                // itself was omitted from `active_descendants`.
                if stable_child
                    || scope
                        .transaction_external_container_children
                        .contains_key(&child.tala_id)
                    || scope
                        .transaction_external_aggregate_children
                        .contains_key(&child.tala_id)
                {
                    queue.push((child.tala_id, child_target));
                }
            }
        }
        if published {
            self.graph.set_projected_node_position(
                self.graph.nodes[stable_child.0 as usize].tala_id,
                target,
            );
        }
        if trace_subtree_publish {
            eprintln!(
                "SUBTREE_PUBLISH_RUST after local={} stable={} published={} state={:?}",
                local_tala_id,
                stable_tala_id,
                published,
                self.graph
                    .nodes
                    .iter()
                    .filter(|node| {
                        matches!(
                            node.tala_id,
                            4254926550 | 1369972579 | 3471367806 | 2092070983
                        )
                    })
                    .map(|node| (node.tala_id, node.position))
                    .collect::<Vec<_>>()
            );
        }
        published
    }

    pub(super) fn publish_external_shared_child_offsets(&mut self, offsets: &[(i64, i64, Point)]) {
        self.publish_external_shared_child_offsets_mode(offsets, false, None);
    }

    pub(super) fn publish_external_shared_child_offsets_mode(
        &mut self,
        offsets: &[(i64, i64, Point)],
        preserve_cluster_geometry: bool,
        cluster_member_positions: Option<&BTreeMap<i64, Point>>,
    ) {
        if crate::internal::trace_env_enabled("TALA_TRACE_PUBLISH_CALLS") {
            eprintln!(
                "PUBLISH_CALL_RUST caller={} offsets={:?}",
                std::panic::Location::caller(),
                offsets
            );
        }
        // The projected vessel is still a shared *Node in recovered Go.
        // Moving its parent calls getAllDescendantNodes(..., true), so every
        // hidden cluster member moves with the vessel even though those
        // members remain absent from Graph.Nodes until syncNested restores
        // them.  The stable arena must publish that hidden geometry without
        // making the members active layout candidates.
        self.publish_external_child_offsets(
            offsets,
            preserve_cluster_geometry,
            cluster_member_positions,
        );
    }

    pub(super) fn publish_mirrored_external_child_offsets(
        &mut self,
        offsets: &[(i64, i64, Point)],
    ) {
        for &(container_tala_id, child_tala_id, offset) in offsets {
            let Some(container_position) = self
                .graph
                .nodes
                .iter()
                .find(|node| node.tala_id == container_tala_id)
                .and_then(|node| node.position)
            else {
                continue;
            };
            let target = Point {
                x: container_position.x + offset.x,
                y: container_position.y + offset.y,
            };
            if crate::internal::trace_env_value("TALA_TRACE_EXTERNAL_OFFSETS_ALL").is_some_and(
                |target| {
                    target == "all"
                        || matches!(
                            (container_tala_id, child_tala_id),
                            (1832207848, 2280770807)
                                | (1832207848, 298920306)
                                | (3680227035, 1832207848)
                        )
                },
            ) {
                eprintln!(
                    "PUBLISH_MIRRORED_OFFSET_RUST container={} child={} owner_pos={:?} offset={:?} target={:?}",
                    container_tala_id, child_tala_id, container_position, offset, target
                );
            }
            if let Some(cluster_index) = self
                .graph
                .clusters
                .iter()
                .position(|cluster| cluster.vessel_tala_id == child_tala_id)
            {
                let Some(owner) = self.graph.clusters[cluster_index].members.first().copied()
                else {
                    continue;
                };
                if let Some(current) = self.graph.active_node_position(owner) {
                    self.graph.translate_active_node_with_children(
                        owner,
                        Point {
                            x: target.x - current.x,
                            y: target.y - current.y,
                        },
                    );
                }
                self.graph
                    .pending_cluster_vessel_positions
                    .insert(cluster_index, target);
                continue;
            }
            if let Some(owner) = self
                .graph
                .sequences
                .iter()
                .find(|sequence| sequence.vessel_tala_id == child_tala_id)
                .and_then(|sequence| sequence.members.first().copied())
            {
                self.graph.move_active_node_abs_with_children(owner, target);
                continue;
            }
            let Some(child) = self
                .graph
                .nodes
                .iter()
                .find(|node| node.tala_id == child_tala_id)
                .map(|node| node.input_id)
            else {
                continue;
            };
            self.graph.move_node_abs_with_children(child, target);
        }
    }

    fn publish_external_child_offsets(
        &mut self,
        offsets: &[(i64, i64, Point)],
        preserve_cluster_geometry: bool,
        cluster_member_positions: Option<&BTreeMap<i64, Point>>,
    ) {
        // At the root boundary a temporary cluster vessel and its retained
        // members are separate Go pointers.  The vessel offset first moves
        // each member into the vessel frame (including its nested subtree),
        // while the same root snapshot also contains stale local offsets for
        // every ordinary descendant below those members.  Replaying those
        // descendant records would pull the subtree back into the old root
        // frame.  Keep their original frame only when the corresponding
        // vessel is not being published here; the vessel arrangement is the
        // authoritative copy-back for the whole retained subtree.
        let preserved_cluster_descendants = if preserve_cluster_geometry {
            let vessel_ids = offsets
                .iter()
                .filter_map(|(_, child_tala_id, _)| {
                    self.graph
                        .clusters
                        .iter()
                        .any(|cluster| cluster.vessel_tala_id == *child_tala_id)
                        .then_some(*child_tala_id)
                })
                .collect::<BTreeSet<_>>();
            self.graph
                .clusters
                .iter()
                .filter(|cluster| vessel_ids.contains(&cluster.vessel_tala_id))
                .flat_map(|cluster| {
                    cluster.members.iter().flat_map(|member| {
                        std::iter::once(self.graph.nodes[member.0 as usize].tala_id).chain(
                            self.graph
                                .active_descendants(*member)
                                .into_iter()
                                .map(|descendant| self.graph.nodes[descendant.0 as usize].tala_id),
                        )
                    })
                })
                .collect::<BTreeSet<_>>()
        } else {
            BTreeSet::new()
        };
        let mut ordered_offsets = offsets.to_vec();
        if preserve_cluster_geometry {
            // These are relative child offsets. Publish each ancestor frame
            // before its descendants so a later parent translation cannot
            // move a child that was already placed in the root frame.
            let container_depth = |container_tala_id: i64| {
                let Some(mut node) = self
                    .graph
                    .nodes
                    .iter()
                    .find(|node| node.tala_id == container_tala_id)
                else {
                    return usize::MAX;
                };
                let mut depth = 0;
                while let Some(parent) = node.container {
                    depth += 1;
                    node = &self.graph.nodes[parent.0 as usize];
                }
                depth
            };
            ordered_offsets
                .sort_by_key(|(container_tala_id, _, _)| container_depth(*container_tala_id));
        }
        for &(container_tala_id, child_tala_id, offset) in &ordered_offsets {
            let Some(container_position) = self
                .graph
                .nodes
                .iter()
                .find(|node| node.tala_id == container_tala_id)
                .and_then(|node| node.position)
            else {
                continue;
            };
            let position = Point {
                x: container_position.x + offset.x,
                y: container_position.y + offset.y,
            };
            if crate::internal::trace_env_enabled("TALA_TRACE_EXTERNAL_OFFSETS_ALL")
                && matches!(
                    (container_tala_id, child_tala_id),
                    (1832207848, 2280770807) | (1832207848, 298920306) | (3680227035, 1832207848)
                )
            {
                eprintln!(
                    "PUBLISH_EXTERNAL_OFFSET_RUST container={} child={} owner_pos={:?} offset={:?} target={:?}",
                    container_tala_id, child_tala_id, container_position, offset, position
                );
            }
            if child_tala_id == 1149337423
                && crate::internal::trace_env_enabled("TALA_TRACE_MOVE_PROJECTION")
            {
                eprintln!(
                    "PUBLISH_EXTERNAL_OFFSET_RUST container={} child={} container_pos={:?} offset={:?} target={:?}",
                    container_tala_id, child_tala_id, container_position, offset, position
                );
            }
            if let Some(cluster_index) = self
                .graph
                .clusters
                .iter()
                .position(|cluster| cluster.vessel_tala_id == child_tala_id)
            {
                // A nested temporary graph can retain offset records for
                // aggregate vessels that live in a sibling/ancestor scope.
                // Go's SyncNestedGeometry visits only vessel pointers that
                // are actually present in that graph's Nodes slice. Arrange
                // those members when their vessel is local; defer external
                // vessels until the owning scope reaches them.
                if preserve_cluster_geometry {
                    // Root CombineSubgraphs has already chosen each member's
                    // position. SyncClusters runs later at GapNormalization;
                    // arranging from declaration order here erases the
                    // optimized member order visible at NodePlacement.
                    let members = self.graph.clusters[cluster_index].members.clone();
                    let projected_positions = cluster_member_positions.filter(|positions| {
                        members.iter().all(|member| {
                            positions.contains_key(&self.graph.nodes[member.0 as usize].tala_id)
                        })
                    });
                    if let Some(projected_positions) = projected_positions {
                        for member in members {
                            let tala_id = self.graph.nodes[member.0 as usize].tala_id;
                            let target = projected_positions[&tala_id];
                            if let Some(current) = self.graph.position(member) {
                                self.graph.translate_node_with_children(
                                    member,
                                    Point {
                                        x: target.x - current.x,
                                        y: target.y - current.y,
                                    },
                                );
                            } else {
                                self.graph.set_position(member, target);
                                self.graph.position_container_children(member, false);
                            }
                        }
                    } else {
                        self.graph
                            .arrange_cluster_members_with_descendants(cluster_index, position);
                    }
                } else {
                    self.graph.arrange_cluster_members(cluster_index, position);
                }
                // Root CombineSubgraphs already moved the actual vessel
                // with its parent. Offset snapshots can retain an earlier
                // local frame even when the member boxes above are final.
                // Source has one live TopLeft; make this numerical cache
                // observe that same bound Point instead of replaying the
                // stale offset into only the cache.
                let position = if preserve_cluster_geometry {
                    self.graph
                        .topology_vessel_node(TopologyVessel::Cluster(cluster_index))
                        .and_then(|vessel| self.graph.position(vessel))
                        .unwrap_or(position)
                } else {
                    position
                };
                self.graph
                    .pending_cluster_vessel_positions
                    .insert(cluster_index, position);
                continue;
            }
            if let Some(owner) = self
                .graph
                .sequences
                .iter()
                .find(|sequence| sequence.vessel_tala_id == child_tala_id)
                .and_then(|sequence| sequence.members.first().copied())
            {
                self.graph
                    .move_active_node_abs_with_children(owner, position);
                continue;
            }
            if preserved_cluster_descendants.contains(&container_tala_id) {
                continue;
            }
            let Some(child) = self
                .graph
                .nodes
                .iter()
                .find(|node| node.tala_id == child_tala_id)
                .map(|node| node.input_id)
            else {
                continue;
            };
            self.graph.move_node_abs_with_children(child, position);
        }
    }

    pub(super) fn hierarchy_depth(&self, mut node: NodeId) -> usize {
        let mut depth = 0;
        while let Some(parent) = self.graph.nodes[node.0 as usize].container {
            depth += 1;
            node = parent;
        }
        depth
    }

    fn push_container_postorder(
        &self,
        container: NodeId,
        seen: &mut BTreeSet<NodeId>,
        ordered: &mut Vec<NodeId>,
    ) {
        if !seen.insert(container) {
            return;
        }
        self.container_placement_postorder(Some(container), seen, ordered);
        ordered.push(container);
    }

    /// `PlaceHierarchies` has already positioned every container owned by a
    /// hierarchy.  The ordinary `placeNodes` walk must still account for the
    /// whole nested container tree, but must not re-enter it as an ordinary
    /// placement scope.
    fn mark_hierarchy_containers(&self, container: NodeId, seen: &mut BTreeSet<NodeId>) {
        if !seen.insert(container) {
            return;
        }
        for child in self
            .graph
            .containers
            .get(&Some(container))
            .into_iter()
            .flatten()
        {
            if self.graph.nodes[child.0 as usize].is_container {
                self.mark_hierarchy_containers(child, seen);
            }
        }
    }

    /// Recovered recursive loop in `Graph.placeNodes`.
    fn container_placement_postorder(
        &self,
        scope: Option<NodeId>,
        seen: &mut BTreeSet<NodeId>,
        ordered: &mut Vec<NodeId>,
    ) {
        // Only the scope's child order is read here; skip the owner-source
        // graph snapshot that placement_scope_graph attaches for placement.
        let Some(mut placement) = self.build_placement_scope_graph_after(scope, &BTreeSet::new())
        else {
            return;
        };
        #[cfg(feature = "diagnostic-traces")]
        let trace_parent_owner_order = placement
            .trace_owner_graph
            .node_order
            .iter()
            .map(|node| placement.trace_owner_graph.nodes[node.0 as usize].tala_id)
            .collect::<Vec<_>>();
        #[cfg(feature = "diagnostic-traces")]
        crate::internal::pipeline::trace_record_children_graph_owner(
            &placement.trace_owner_graph,
            &placement.trace_children_graph_node_order,
        );
        let child_order = std::mem::take(&mut placement.child_order);
        // Go's recursive child graphs share their entity maps. This preview
        // only supplies ordering; retaining its copied projection at every
        // ancestor keeps quadratic snapshot storage live during this walk.
        // Diagnostic callbacks below still observe the preview's owner view.
        #[cfg(not(feature = "diagnostic-traces"))]
        drop(placement);
        for child in child_order {
            if self.graph.nodes[child.0 as usize].hierarchy.is_some() {
                if self.graph.nodes[child.0 as usize].is_container {
                    self.mark_hierarchy_containers(child, seen);
                }
                continue;
            }
            if self.graph.nodes[child.0 as usize].is_container {
                self.push_container_postorder(child, seen, ordered);
                #[cfg(feature = "diagnostic-traces")]
                crate::internal::pipeline::trace_restore_graph_reference(
                    &placement.trace_owner_graph,
                    &trace_parent_owner_order,
                );
            }

            // AddClusters replaces its members with a vessel. The Rust arena
            // retains the first member as that vessel's stable carrier.
            for cluster in self
                .graph
                .clusters
                .iter()
                .filter(|cluster| cluster.members.first() == Some(&child))
            {
                for member in cluster.members.iter().copied() {
                    if self.graph.nodes[member.0 as usize].is_container {
                        self.push_container_postorder(member, seen, ordered);
                        #[cfg(feature = "diagnostic-traces")]
                        crate::internal::pipeline::trace_restore_graph_reference(
                            &placement.trace_owner_graph,
                            &trace_parent_owner_order,
                        );
                    }
                }
            }
        }
        #[cfg(feature = "diagnostic-traces")]
        if let Some(root) = scope {
            crate::internal::pipeline::trace_record_root_graph_owner(
                &placement.trace_owner_graph,
                self.graph.nodes[root.0 as usize].tala_id,
            );
        }
    }

    /// D2 v0.9.0 placement.NormalizeGaps. Geometry-only candidate rejection
    /// keeps memoized costs; a failed enclosing stage restores its entry state.
    pub(super) fn gap_normalization_stage(&mut self) -> Result<bool, LayoutError> {
        let context = gap_guard::ActiveContext::new();
        let mut guard =
            limits::WorkGuard::transaction(Some(&context), "GapNormalizationTransactions")
                .map_err(|error| LayoutError::PlacementTransaction(error.to_string()))?;
        self.gap_normalization_stage_with_guard(&mut guard)
            .map_err(|error| LayoutError::PlacementTransaction(error.to_string()))
    }

    /// Callers with a request transaction share its guard across every nested
    /// trial, preserving consumed work and the source diagnostic location.
    pub(super) fn gap_normalization_stage_with_guard(
        &mut self,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<bool, limits::Error> {
        let context = guard.context();
        self.gap_normalization_stage_with_context(context, guard)
    }

    pub(super) fn gap_normalization_stage_with_context(
        &mut self,
        operation_context: &dyn limits::Context,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<bool, limits::Error> {
        #[cfg(feature = "diagnostic-traces")]
        guard.observe_transaction_graph(&self.graph, limits::TransactionPollSite::General);
        guard.finish()?;
        let mut checkpoint = GeometryCheckpoint::capture_request(&self.graph, guard)?;
        let prior = checkpoint.preserve();
        let (canonical_overlaps, canonical_exact_overlaps) =
            self.graph.transaction_overlap_snapshot_checked(guard)?;
        guard.finish()?;
        let cell_size = self.graph.cell_size;
        let routing = self.graph.raw_routing_costs();
        let overlaps = self
            .graph
            .numeric_transaction_overlap_pairs(&canonical_overlaps);
        let exact_overlaps = self
            .graph
            .numeric_transaction_overlap_pairs(&canonical_exact_overlaps);
        let mut work = gap_guard::GapWork::new(guard, operation_context, true);
        work.anchor_canonical_overlaps(canonical_overlaps, canonical_exact_overlaps);
        // Go's deferred stage rollback also runs during a panic. Catch only to
        // restore the same state, then resume the original unwind unchanged.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.gap_normalization_stage_body(
                &mut work,
                &overlaps,
                &exact_overlaps,
                &mut checkpoint,
            )
        }));
        match result {
            Ok(Ok(changed)) => Ok(changed),
            failed => {
                prior.rollback(&mut self.graph);
                self.graph.cell_size = cell_size;
                work.restore_costs(&self.graph, routing);
                match failed {
                    Ok(Err(error)) => Err(error),
                    Err(payload) => std::panic::resume_unwind(payload),
                    Ok(Ok(_)) => unreachable!(),
                }
            }
        }
    }

    fn gap_normalization_stage_body(
        &mut self,
        work: &mut gap_guard::GapWork<'_, '_>,
        prior_overlaps: &BTreeSet<(NodeId, NodeId)>,
        prior_exact_overlaps: &BTreeSet<(NodeId, NodeId)>,
        checkpoint: &mut GeometryCheckpoint,
    ) -> Result<bool, limits::Error> {
        let containers = self.graph.gap_container_reverse_dfs_order(work.guard)?;
        if crate::internal::trace_env_enabled("TALA_TRACE_GAP_CALL") {
            eprintln!(
                "GAP_ROOT_CHILDREN_RUST {:?}",
                self.graph
                    .containers
                    .get(&None)
                    .into_iter()
                    .flatten()
                    .map(|id| self.graph.nodes[id.0 as usize].tala_id)
                    .collect::<Vec<_>>()
            );
            eprintln!(
                "GAP_CONTAINERS_RUST {:?}",
                containers
                    .iter()
                    .map(|id| self.graph.nodes[id.0 as usize].tala_id)
                    .collect::<Vec<_>>()
            );
        }
        let mut changed = false;
        for container in containers {
            let descendants = self.graph.active_descendants(container);
            if crate::internal::trace_env_enabled("TALA_TRACE_GAP_CALL") {
                eprintln!(
                    "GAP_CONTAINER_RUST id={} descendants={} ids={:?}",
                    self.graph.nodes[container.0 as usize].tala_id,
                    descendants.len(),
                    descendants
                        .iter()
                        .map(|id| self.graph.nodes[id.0 as usize].tala_id)
                        .collect::<Vec<_>>()
                );
            }
            for horizontal in [true, false] {
                if crate::internal::trace_env_enabled("TALA_TRACE_GAP_CALL") {
                    eprintln!(
                        "GAP_CALL_RUST_BEFORE container={} horizontal={} tracked={:?}",
                        self.graph.nodes[container.0 as usize].tala_id,
                        horizontal,
                        self.graph
                            .nodes
                            .iter()
                            .filter(|node| matches!(node.tala_id, 1149337423 | 1782109120))
                            .map(|node| (node.tala_id, node.position))
                            .collect::<Vec<_>>()
                    );
                }
                changed |= self
                    .graph
                    .gap_normalization_pass_for_with_checkpoint_checked(
                        &descendants,
                        horizontal,
                        true,
                        prior_overlaps,
                        prior_exact_overlaps,
                        work,
                        checkpoint,
                    )?;
                if crate::internal::trace_env_enabled("TALA_TRACE_GAP_CALL") {
                    let root = self.graph.nodes.first().unwrap();
                    eprintln!(
                        "GAP_CALL_RUST container={} horizontal={} forwards=true root={:?}:{},{} tracked={:?}",
                        self.graph.nodes[container.0 as usize].tala_id,
                        horizontal,
                        root.position,
                        root.rect.size.width,
                        root.rect.size.height,
                        self.graph
                            .nodes
                            .iter()
                            .filter(|node| matches!(node.tala_id, 1149337423 | 1782109120))
                            .map(|node| (node.tala_id, node.position))
                            .collect::<Vec<_>>()
                    );
                }
                changed |= self
                    .graph
                    .gap_normalization_pass_for_with_checkpoint_checked(
                        &descendants,
                        horizontal,
                        false,
                        prior_overlaps,
                        prior_exact_overlaps,
                        work,
                        checkpoint,
                    )?;
                if crate::internal::trace_env_enabled("TALA_TRACE_GAP_CALL") {
                    let root = self.graph.nodes.first().unwrap();
                    eprintln!(
                        "GAP_CALL_RUST container={} horizontal={} forwards=false root={:?}:{},{} tracked={:?}",
                        self.graph.nodes[container.0 as usize].tala_id,
                        horizontal,
                        root.position,
                        root.rect.size.width,
                        root.rect.size.height,
                        self.graph
                            .nodes
                            .iter()
                            .filter(|node| matches!(node.tala_id, 1149337423 | 1782109120))
                            .map(|node| (node.tala_id, node.position))
                            .collect::<Vec<_>>()
                    );
                }
            }
        }
        // The recovered stage has already run the container RDFS passes
        // above. Its final loop is over the flat Graph.Nodes slice only;
        // calling the graph-level helper here would repeat every container
        // pass and can accept a different sequence of gap trials.
        // The adapter's root Graph.Nodes slice is the source insertion order
        // for ordinary hierarchical graphs. Recursive Rust placement keeps a
        // separate traversal order for containers, which must not reorder the
        // final gap-normalization initiators when no aggregate vessel exists.
        let nodes = if self.graph.source_hierarchy_ordered
            && self.graph.clusters.is_empty()
            && self.graph.sequences.is_empty()
        {
            self.graph
                .nodes
                .iter()
                .enumerate()
                .map(|(index, _)| NodeId(index as u32))
                .collect::<Vec<_>>()
        } else {
            self.graph.graph_node_order()
        };
        for horizontal in [true, false] {
            if crate::internal::trace_env_enabled("TALA_TRACE_GAP_CALL") {
                eprintln!(
                    "GAP_CALL_RUST_BEFORE container=all horizontal={} tracked={:?}",
                    horizontal,
                    self.graph
                        .nodes
                        .iter()
                        .filter(|node| matches!(node.tala_id, 1149337423 | 1782109120))
                        .map(|node| (node.tala_id, node.position))
                        .collect::<Vec<_>>()
                );
            }
            changed |= self
                .graph
                .gap_normalization_pass_for_with_checkpoint_checked(
                    &nodes,
                    horizontal,
                    true,
                    prior_overlaps,
                    prior_exact_overlaps,
                    work,
                    checkpoint,
                )?;
            changed |= self
                .graph
                .gap_normalization_pass_for_with_checkpoint_checked(
                    &nodes,
                    horizontal,
                    false,
                    prior_overlaps,
                    prior_exact_overlaps,
                    work,
                    checkpoint,
                )?;
        }
        if crate::internal::trace_env_enabled("TALA_TRACE_GAP_CALL") {
            let root = self.graph.nodes.first().unwrap();
            eprintln!(
                "GAP_CALL_RUST container=all root={:?}:{},{}",
                root.position, root.rect.size.width, root.rect.size.height
            );
        }
        if crate::internal::trace_env_enabled("TALA_TRACE_GAP_SYNC") {
            eprintln!(
                "GAP_BEFORE_SYNC_RUST {:?}",
                self.graph
                    .nodes
                    .iter()
                    .filter_map(|n| n.position.map(|p| (n.tala_id, p, n.rect.size)))
                    .collect::<Vec<_>>()
            );
        }
        // NormalizeGaps always publishes the live aggregate vessels after its
        // directional passes, including a flat root graph. The Go graph still
        // owns each temporary vessel at this boundary; CleanupStuff has not
        // yet restored the hidden members. Keep the same final SyncSequences →
        // SyncClusters lifecycle for every scope so half-pixel arrangements
        // are resolved at the same stage in Rust. SyncClusters must derive
        // vessel positions after SyncSequences; sequence publication can move
        // a retained carrier across a half-pixel rounding tie.
        self.graph.reset_turn_cost();
        self.graph.sync_sequences();
        self.graph.sync_clusters();
        if crate::internal::trace_env_enabled("TALA_TRACE_GAP_SYNC") {
            eprintln!(
                "GAP_AFTER_SYNC_RUST {:?}",
                self.graph
                    .nodes
                    .iter()
                    .filter_map(|n| n.position.map(|p| (n.tala_id, p, n.rect.size)))
                    .collect::<Vec<_>>()
            );
        }
        if crate::internal::trace_env_enabled("TALA_TRACE_GAP_CALL") {
            eprintln!("GAP_RESULT_RUST changed={changed}");
        }
        #[cfg(feature = "diagnostic-traces")]
        work.observe(&self.graph, limits::TransactionPollSite::General);
        work.guard.finish()?;
        Ok(changed)
    }

    /// Recovered bottom-up `Graph.placeNodes` hierarchy transaction. Scopes
    /// are materialized deepest-first so a parent optimizer sees each fitted
    /// child container as one atomic node. Explicit D2 grids use their
    /// deterministic cell placement at that same boundary; ordinary scopes
    /// run the recovered placement pipeline over their direct children.
    pub(super) fn place_nodes_recursively(&mut self) -> Result<bool, crate::LayoutError> {
        // CopyEntitiesFrom shares the actual requested-direction map. Local
        // layout defaults are hints, not entries in that Source map.
        self.graph.bind_owner_placement_directions();
        let common_map = self.placement_common_uncles();
        let prior_map = self
            .graph
            .replace_owner_common_uncle_map(OwnerGraphId(0), Some(common_map))
            .map_err(|error| crate::LayoutError::PlacementTransaction(error.to_string()))?;
        // placement.Place computes the owning graph's cell lattice only
        // after recursive placement succeeds. Temporary component graphs
        // compute their own lattices before entering each optimizer. Retained
        // Near neighbors can still point to this original, uninitialized graph.
        let prior_cell_size = self.graph.cell_size;
        self.graph.cell_size = self.input.serialized_cell_size.unwrap_or(0.0);
        let mut frames = Vec::new();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.place_nodes_recursively_with_owner_frames(&mut frames)
        }));
        // Source node_placement.go99–110: nested defers restore inner frames
        // before their parents on error or panic. Successful scopes remove
        // their frame only after normal copyback and restoration.
        while let Some((_, owner)) = frames.pop() {
            self.restore_prepared_owner_scope(&owner)?;
        }
        // Ok(false) delegates to the existing flat placement path; this
        // recursive path has not published a final owner lattice in that case.
        if matches!(&result, Ok(Ok(false))) {
            self.graph.cell_size = prior_cell_size;
        }
        let successful = matches!(&result, Ok(Ok(_)));
        if successful {
            self.graph
                .replace_owner_common_uncle_map(OwnerGraphId(0), Some(SharedSliceMap::nil()))
                .map_err(|error| crate::LayoutError::PlacementTransaction(error.to_string()))?;
        } else {
            self.graph.cell_size = prior_cell_size;
            self.graph
                .replace_owner_common_uncle_map(OwnerGraphId(0), prior_map)
                .map_err(|error| crate::LayoutError::PlacementTransaction(error.to_string()))?;
        }
        match result {
            Ok(result) => result,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    fn enter_recursive_owner_frame(
        &mut self,
        root: Option<NodeId>,
        materialized: &BTreeSet<Option<NodeId>>,
        frames: &mut Vec<(Option<NodeId>, super::hierarchy_scope::PreparedOwnerScope)>,
    ) -> Result<(), crate::LayoutError> {
        if self.graph.node_order.len() <= 1 || frames.iter().any(|(key, _)| *key == root) {
            return Ok(());
        }
        if let Some((_, Some(owner))) = self.prepare_placement_scope_after(root, materialized)? {
            frames.push((root, owner));
        }
        Ok(())
    }

    fn place_nodes_recursively_with_owner_frames(
        &mut self,
        frames: &mut Vec<(Option<NodeId>, super::hierarchy_scope::PreparedOwnerScope)>,
    ) -> Result<bool, crate::LayoutError> {
        self.source_orientation_disabled_for_loop =
            self.graph.source_orientation_disabled_for_loop(self.seed)?;
        #[cfg(feature = "diagnostic-traces")]
        macro_rules! trace_root_geometry {
            ($phase:expr) => {
                if crate::internal::trace_env_enabled("TALA_TRACE_ROOT_STEPS_GEOM") {
                    eprintln!(
                        "ROOT_STEP_GEOM_RUST phase={} pending={:?} {:?}",
                        $phase,
                        self.graph.pending_cluster_vessel_positions,
                        self.graph
                            .nodes
                            .iter()
                            .filter(|node| {
                                crate::internal::trace_env_value("TALA_TRACE_ROOT_GEOMETRY")
                                    .as_deref()
                                    == Some("all")
                                    || matches!(
                                        node.tala_id,
                                        2043751575
                                            | 3677471320
                                            | 2884276447
                                            | 1010209959
                                            | 1231386872
                                            | 3248328362
                                            | 3680227035
                                            | 1832207848
                                            | 2280770807
                                            | 298920306
                                            | 3297548089
                                            | 3777840222
                                    )
                            })
                            .map(|node| (node.tala_id, node.position))
                            .collect::<Vec<_>>()
                    );
                }
            };
        }
        #[cfg(not(feature = "diagnostic-traces"))]
        macro_rules! trace_root_geometry {
            ($phase:expr) => {};
        }
        trace_root_geometry!("begin");
        let all_containers = self
            .graph
            .nodes
            .iter()
            .filter(|node| node.is_container)
            .map(|node| node.input_id)
            .collect::<Vec<_>>();
        #[cfg(feature = "diagnostic-traces")]
        crate::internal::pipeline::trace_reset_owner_graph_views(&self.graph);
        let mut containers = Vec::with_capacity(all_containers.len());
        let mut seen = BTreeSet::new();
        self.container_placement_postorder(None, &mut seen, &mut containers);
        debug_assert!(
            all_containers
                .iter()
                .all(|container| seen.contains(container))
        );
        #[cfg(feature = "diagnostic-traces")]
        crate::internal::pipeline::trace_record_container_placement_order(&self.graph, &containers);
        #[cfg(feature = "diagnostic-traces")]
        let mut traced_place_nodes_entries = BTreeSet::new();
        #[cfg(feature = "diagnostic-traces")]
        if self.graph.node_order.len() > 1 {
            // Go enters placeNodes on the owning graph before recursing into
            // child containers. Rust materializes child scopes in postorder,
            // so emit each recursive entry when that scope is reached: this
            // preserves Go's preorder event sequence while retaining the
            // graph state that exists at that point in the Rust traversal.
            crate::internal::pipeline::trace_placement_state(
                "placeNodes-entry",
                "top",
                &self.graph,
            );
            traced_place_nodes_entries.insert(None);
        }
        // This method mirrors Graph.placeNodes: materialize nested placement
        // scopes deepest-first, fit their carrier nodes, then optimize the
        // root children graph. The root transaction is required for flat
        // graphs too: AddSequences/AddClusters may have replaced physical
        // members with temporary vessels before Graph.placeNodes constructs
        // its root children graph.
        let mut retained_nears = Vec::new();
        let mut materialized_parent_scopes = BTreeSet::new();
        let mut obstacle_frames = ancestor_obstacles::AncestorObstacleFrames::default();

        obstacle_frames.enter(&self.graph, None);
        self.enter_recursive_owner_frame(None, &materialized_parent_scopes, frames)?;
        for container in containers {
            // Enter the real outer-to-inner abduction lifetime before the
            // postorder geometry solver reaches this container. Source
            // redirects an outer edge away from the child before recursion.
            let mut entry_path = Vec::new();
            let mut ancestor = Some(container);
            while let Some(node) = ancestor {
                if self.graph.nodes[node.0 as usize].hierarchy.is_none() {
                    entry_path.push(node);
                }
                ancestor = self.graph.nodes[node.0 as usize].container;
            }
            for node in entry_path.into_iter().rev() {
                obstacle_frames.enter(&self.graph, Some(node));
                self.enter_recursive_owner_frame(Some(node), &materialized_parent_scopes, frames)?;
            }
            if crate::internal::trace_env_enabled("TALA_TRACE_NESTING_POWER_WITNESS") {
                let witness = self
                    .graph
                    .nodes
                    .iter()
                    .find(|node| node.external_id == "directions.l");
                eprintln!(
                    "NESTING_WITNESS_RUST phase=enter container={} container_pos={:?} witness_pos={:?} witness_translation={:?} witness_materialized={}",
                    self.graph.nodes[container.0 as usize].external_id,
                    self.graph.position(container),
                    witness.and_then(|node| node.position),
                    witness.and_then(|node| node.unpositioned_scope_translation),
                    witness.is_some_and(|node| node.scope_translation_materialized),
                );
            }
            #[cfg(feature = "diagnostic-traces")]
            {
                let mut entry_path = Vec::new();
                let mut current = Some(container);
                while let Some(node_id) = current {
                    let node = &self.graph.nodes[node_id.0 as usize];
                    if node.is_container && node.hierarchy.is_none() {
                        entry_path.push(node_id);
                    }
                    current = node.container;
                }
                entry_path.reverse();
                for node_id in entry_path {
                    if !traced_place_nodes_entries.insert(Some(node_id)) {
                        continue;
                    }
                    let root = self.graph.nodes[node_id.0 as usize].external_id.clone();
                    crate::internal::pipeline::trace_placement_state(
                        "placeNodes-entry",
                        &root,
                        &self.graph,
                    );
                }
            }
            let children = self
                .graph
                .containers
                .get(&Some(container))
                .unwrap_or_default();
            #[cfg(feature = "diagnostic-traces")]
            let trace_root = self.graph.nodes[container.0 as usize].external_id.clone();
            #[cfg(feature = "diagnostic-traces")]
            crate::internal::pipeline::trace_placement_state(
                "after-child-recursion",
                &trace_root,
                &self.graph,
            );
            if crate::internal::trace_env_value("TALA_TRACE_RECURSIVE_CHILDREN").as_deref()
                == Some("1472025070")
            {
                eprintln!(
                    "RECURSIVE_CHILDREN_RUST container={} containers={:?} node_order={:?} parents={:?}",
                    self.graph.nodes[container.0 as usize].tala_id,
                    children
                        .iter()
                        .map(|child| self.graph.nodes[child.0 as usize].tala_id)
                        .collect::<Vec<_>>(),
                    self.graph
                        .node_order
                        .iter()
                        .map(|node| self.graph.nodes[node.0 as usize].tala_id)
                        .collect::<Vec<_>>(),
                    self.graph
                        .nodes
                        .iter()
                        .filter(|node| node.container == Some(container))
                        .map(|node| node.tala_id)
                        .collect::<Vec<_>>()
                );
            }
            let parent_root = self.graph.nodes[container.0 as usize].container;
            if let Some((_, parent)) = frames.iter().find(|(key, _)| *key == parent_root) {
                let receiver = frames
                    .iter()
                    .find(|(key, _)| *key == Some(container))
                    .map_or(OwnerGraphId(0), |(_, frame)| frame.view);
                self.assign_parent_abduction_nears(container, receiver, parent)?;
                self.assign_herds_for_scope(container, receiver, &parent.abductions)?;
            }
            // Near and herd assignment only change relationships recorded on
            // the graph, not the boxes serialized by tracePlacement. Emit
            // both source boundaries in order while retaining the same owner
            // snapshot payload.
            #[cfg(feature = "diagnostic-traces")]
            crate::internal::pipeline::trace_placement_state(
                "after-nears",
                &trace_root,
                &self.graph,
            );
            // TALA's recovered Node and Graph shapes have no grid-row,
            // grid-column, packed-grid, or label-aware-grid carrier. Every
            // ordinary container therefore traverses the same temporary child
            // graph and `placeNodesOrthogonally` path. D2 may still provide
            // grid metadata to the plugin adapter, but it cannot replace this
            // TALA-owned recursive placement transaction.
            let existing_owner = frames
                .iter()
                .find(|(key, _)| *key == Some(container))
                .map(|(_, owner)| owner.clone());
            let prepared = if let Some(owner) = existing_owner {
                self.refresh_prepared_placement_scope(
                    Some(container),
                    &materialized_parent_scopes,
                    &owner,
                )
                .map(|placement| (placement, Some(owner)))
            } else {
                self.prepare_placement_scope_after(Some(container), &materialized_parent_scopes)?
            };
            let Some((mut placement, prepared_owner)) = prepared else {
                obstacle_frames.exit(Some(container));
                continue;
            };
            if let Some(obstacles) = obstacle_frames.incoming(Some(container)) {
                placement.sized_obstacles = obstacles.to_vec();
                placement.source_orientation.has_ancestor_obstacles = !obstacles.is_empty();
            }
            #[cfg(feature = "diagnostic-traces")]
            crate::internal::pipeline::trace_placement_state(
                "after-herds",
                &trace_root,
                &self.graph,
            );
            let placed_scope =
                self.solve_prepared_owner_scope_inner(&placement, prepared_owner.as_ref())?;
            self.next_rng_float = placed_scope.next_rng_float;
            self.copy_tree_edge_label_placements(&placement, &placed_scope.graph);
            if crate::internal::trace_env_enabled("TALA_TRACE_CLUSTER_STATE") {
                eprintln!(
                    "CLUSTER_STATE_OWNER_RUST phase=nested scope={} maps_desired={:?} maps_arrangements={:?} owner={:?}",
                    self.graph.nodes[container.0 as usize].tala_id,
                    placed_scope.cluster_desired_arrangements,
                    placed_scope.cluster_arrangements,
                    self.graph
                        .clusters
                        .iter()
                        .map(|cluster| (
                            cluster.vessel_tala_id,
                            cluster.arrangement,
                            cluster.desired_arrangement
                        ))
                        .collect::<Vec<_>>()
                );
            }
            self.publish_accepted_cluster_policies(
                &placed_scope.cluster_arrangements,
                &placed_scope.cluster_desired_arrangements,
                &placed_scope.graph,
            )?;
            #[cfg(feature = "diagnostic-traces")]
            if crate::internal::trace_env_enabled("TALA_TRACE_CLUSTER_STATE") {
                for &index in placed_scope.cluster_arrangements.keys() {
                    let vessel_id = self.graph.clusters[index].vessel_tala_id;
                    eprintln!(
                        "NESTED_CLUSTER_GEOMETRY_RUST scope={} vessel={} pending={:?} local={:?} original={:?}",
                        self.graph.nodes[container.0 as usize].tala_id,
                        vessel_id,
                        placed_scope.cluster_vessel_positions.get(&index),
                        placed_scope
                            .graph
                            .nodes
                            .iter()
                            .filter(|node| node.tala_id == vessel_id)
                            .map(|node| (
                                placed_scope.graph.position(node.input_id),
                                node.rect.size
                            ))
                            .collect::<Vec<_>>(),
                        self.graph
                            .topology_vessel_node(TopologyVessel::Cluster(index))
                            .map(|node| self.graph.position(node)),
                    );
                }
            }
            // Source placeNodes retains the same vessel/member pointers in
            // the combined graph and g (node_placement.go:48-50, 216-237).
            // In particular, the live Containers header now contains the
            // distinct vessel, which has no old_to_new member key. Publish
            // the completed induced geometry before the enclosing fit; the
            // ordinary child-copyback loop cannot recover these hidden members.
            Self::publish_induced_external_geometry(&mut self.graph, &placed_scope.graph);
            // The fitted descendants have now published their expanded boxes
            // into the owner arena. Refresh the independent temporary vessel
            // dimensions before the next enclosing scope computes aggregate
            // member offsets; otherwise a member wider than the pre-fit
            // vessel receives a negative centering offset.
            self.graph.refresh_cluster_vessel_sizes();
            self.graph.pending_cluster_vessel_positions.extend(
                placed_scope
                    .cluster_vessel_positions
                    .iter()
                    .map(|(&cluster_index, &position)| (cluster_index, position)),
            );
            let external_shared_child_offsets = placed_scope.external_shared_child_offsets.clone();
            let mirrored_external_child_offsets =
                placed_scope.mirrored_external_child_offsets.clone();
            let source_orientation_applied = placed_scope.source_orientation_applied;
            self.publish_placed_tree_orientations(&placed_scope.graph);
            let scope = placed_scope.graph;
            if source_orientation_applied {
                // Go's temporary placement graphs share Node pointers with
                // their owner. Source orientation therefore updates the
                // owning graph before the enclosing container's before-fit
                // callback. Publish the matching stable nodes now; ordinary
                // copyback below will then see the already-rotated frame.
                for (&stable, &local) in &placement.old_to_new {
                    let placed = &scope.nodes[local.0 as usize];
                    let owner = &mut self.graph.nodes[stable.0 as usize];
                    if owner.tala_id != placed.tala_id {
                        // Aggregate vessels have a distinct runtime identity
                        // and use the dedicated vessel/member copyback path.
                        continue;
                    }
                    owner.position = placed.position;
                    owner.rect.origin = placed.rect.origin;
                    owner.rect.size = placed.rect.size;
                }
            }
            #[cfg(feature = "diagnostic-traces")]
            let trace_nested_external_publish =
                crate::internal::trace_env_value("D2RS_TRACE_NESTED_EXTERNAL_PUBLISH");
            #[cfg(feature = "diagnostic-traces")]
            if trace_nested_external_publish.as_deref() == Some("all")
                || trace_nested_external_publish
                    .as_deref()
                    .and_then(|value| value.parse::<i64>().ok())
                    == Some(self.graph.nodes[container.0 as usize].tala_id)
            {
                eprintln!(
                    "D2RS_NESTED_EXTERNAL_PUBLISH scope={} nodes={:?} external={:?} owner={:?}",
                    self.graph.nodes[container.0 as usize].tala_id,
                    scope
                        .nodes
                        .iter()
                        .filter(|node| matches!(node.tala_id, 1 | 5 | 7 | 9 | 25 | 27 | 30))
                        .map(|node| (node.tala_id, node.position, node.rect.size))
                        .collect::<Vec<_>>(),
                    scope
                        .transaction_external_containers
                        .iter()
                        .map(|node| (node.tala_id, node.position, node.rect.size))
                        .collect::<Vec<_>>(),
                    self.graph
                        .nodes
                        .iter()
                        .filter(|node| matches!(node.tala_id, 1 | 5 | 7 | 9 | 25 | 27 | 30))
                        .map(|node| (node.tala_id, node.position, node.rect.size))
                        .collect::<Vec<_>>()
                );
            }
            // `Graph.CopyEntitiesFrom` shares the full Containers map. Every
            // transaction in this nested scope can therefore refit a
            // positioned container outside the active Nodes slice. Publish
            // those shared boxes back before the parent continues, matching
            // Go's shared node pointers; copying only this scope's root loses
            // committed sibling and ancestor refits.
            if crate::internal::trace_env_enabled("TALA_TRACE_NESTING_POWER_WITNESS")
                && let Some(published) = scope
                    .transaction_external_containers
                    .iter()
                    .find(|node| node.external_id == "directions.l")
            {
                let owner = self
                    .graph
                    .nodes
                    .iter()
                    .find(|node| node.external_id == "directions.l");
                eprintln!(
                    "NESTING_WITNESS_PUBLISH_RUST scope={} published_pos={:?} owner_pos={:?} translation={:?} materialized={}",
                    self.graph.nodes[container.0 as usize].external_id,
                    published.position,
                    owner.and_then(|node| node.position),
                    owner.and_then(|node| node.unpositioned_scope_translation),
                    owner.is_some_and(|node| node.scope_translation_materialized),
                );
            }
            for published in &scope.transaction_external_containers {
                let trace_nesting_witness =
                    crate::internal::trace_env_enabled("TALA_TRACE_NESTING_POWER_WITNESS")
                        && published.external_id == "directions.l";
                let witness_before = trace_nesting_witness.then(|| {
                    self.graph
                        .nodes
                        .iter()
                        .find(|node| node.external_id == "directions.l")
                        .and_then(|node| node.position)
                });
                let copy_box = |node: &mut ArenaNode| {
                    if node.tala_id == published.tala_id {
                        node.position = published.position;
                        node.rect.origin = published.rect.origin;
                        node.rect.size = published.rect.size;
                    }
                };
                for node in &mut self.graph.nodes {
                    copy_box(node);
                }
                for node in &mut self.graph.transaction_external_containers {
                    copy_box(node);
                }
                for node in &mut self.graph.transaction_validation_external_containers {
                    copy_box(node);
                }
                for node in self
                    .graph
                    .transaction_external_container_children
                    .values_mut()
                    .flatten()
                {
                    copy_box(node);
                }
                for node in self
                    .graph
                    .transaction_external_aggregate_children
                    .values_mut()
                    .flatten()
                {
                    copy_box(node);
                }
                if trace_nesting_witness {
                    let witness_after = self
                        .graph
                        .nodes
                        .iter()
                        .find(|node| node.external_id == "directions.l")
                        .and_then(|node| node.position);
                    eprintln!(
                        "NESTING_WITNESS_PUBLISH_RUST_APPLIED scope={} published_pos={:?} owner_before={:?} owner_after={:?}",
                        self.graph.nodes[container.0 as usize].external_id,
                        published.position,
                        witness_before.flatten(),
                        witness_after,
                    );
                }
            }
            if crate::internal::trace_env_enabled("TALA_TRACE_NESTED_GEOM") {
                let tracked = [
                    1832207848_i64,
                    2280770807,
                    298920306,
                    3777840222,
                    4222097421,
                    1379919604,
                    3680227035,
                    3297548089,
                    3248328362,
                ];
                if crate::internal::trace_env_enabled("TALA_TRACE_NESTED_ALL") {
                    eprintln!(
                        "NESTED_GEOM_RUST_ALL phase=scope root={} scope_nodes={:?} owner_nodes={:?}",
                        self.graph.nodes[container.0 as usize].tala_id,
                        scope
                            .nodes
                            .iter()
                            .map(|node| (node.tala_id, node.position, node.rect.size))
                            .collect::<Vec<_>>(),
                        self.graph
                            .nodes
                            .iter()
                            .map(|node| (node.tala_id, node.position, node.rect.size))
                            .collect::<Vec<_>>()
                    );
                }
                eprintln!(
                    "NESTED_GEOM_RUST phase=scope root={} scope_nodes={:?} owner_nodes={:?}",
                    self.graph.nodes[container.0 as usize].tala_id,
                    scope
                        .nodes
                        .iter()
                        .filter(|node| tracked.contains(&node.tala_id))
                        .map(|node| (node.tala_id, node.position, node.rect.size))
                        .collect::<Vec<_>>(),
                    self.graph
                        .nodes
                        .iter()
                        .filter(|node| tracked.contains(&node.tala_id))
                        .map(|node| (node.tala_id, node.position, node.rect.size))
                        .collect::<Vec<_>>()
                );
            }
            if crate::internal::trace_env_enabled("TALA_TRACE_GRID_NESTED_SCOPE")
                && self.graph.nodes[container.0 as usize].tala_id == 1_149_337_423
            {
                eprintln!(
                    "GRID_NESTED_SCOPE_RUST container={} scope_children={:?} stable_children={:?}",
                    self.graph.nodes[container.0 as usize].tala_id,
                    scope
                        .transaction_external_container_children
                        .values()
                        .flatten()
                        .filter(|child| matches!(
                            child.tala_id,
                            1_402_186_134 | 1_385_408_515 | 1_368_630_896
                        ))
                        .map(|child| (
                            child.tala_id,
                            child.position,
                            child.rect.origin,
                            child.rect.size
                        ))
                        .collect::<Vec<_>>(),
                    self.graph
                        .nodes
                        .iter()
                        .filter(|node| matches!(
                            node.tala_id,
                            1_402_186_134 | 1_385_408_515 | 1_368_630_896
                        ))
                        .map(|node| (
                            node.tala_id,
                            node.position,
                            node.rect.origin,
                            node.rect.size
                        ))
                        .collect::<Vec<_>>()
                );
            }
            retained_nears.extend(Self::map_scope_nears(
                &placement.old_to_new,
                &placement.assigned_nears,
            ));
            let container_origin = self.graph.position(container).unwrap_or_default();
            // Go fits the container against the direct children of the
            // temporary childrenGraph, not against the owner's original
            // Containers slice. Extracted branching-tree descendants and
            // cluster members are still reachable through shared pointers,
            // but the graph passed to fitNodeToGraph contains their direct
            // carrier (the tree root or aggregate vessel) as one box. Using
            // `children` here reintroduced those descendants as independent
            // boxes and changed both the fixed bounds and the fitted size.
            let mut local_children = placement
                .child_order
                .iter()
                .map(|child| placement.old_to_new[child])
                .collect::<Vec<_>>();
            // Branching-tree descendants remain direct entries in Go's
            // owner Containers map even though the temporary optimizer uses
            // the tree root as its carrier. FitNodeToGraph therefore sees the
            // retained tree nodes as well; include their projected boxes once
            // without reintroducing aggregate members as independent nodes.
            for child in &placement.tree_auxiliary_nodes {
                if !local_children.contains(child) {
                    local_children.push(*child);
                }
            }
            // Recovered Go path:
            //   fitNodeToGraph -> Nodes.getFixedBoundingBox
            //     -> Nodes.getBoundingBox -> Node.getBoundingBox
            // The node-level bounds include outside labels. In particular, an
            // outside-bottom child label can determine the fitted container
            // height even when another child's bare box reaches slightly
            // farther down.
            // Go's `FitToGraph` receives the combined graph, whose `Nodes`
            // slice contains every descendant pointer added by
            // `CombineSubgraphs`, not only the current container's direct
            // children.  Rust induced scopes keep those descendants in
            // transaction projections; use their local snapshots for the
            // same bounds calculation.  This is observable for nested
            // containers and aggregate members whose stable arena entries
            // still carry an enclosing-scope frame.
            let fit_root_tala_id = self.graph.nodes[container.0 as usize].tala_id;
            let mut fit_candidates = BTreeMap::<i64, ArenaNode>::new();
            let mut add_fit_candidate = |node: &ArenaNode| {
                if node.tala_id == fit_root_tala_id
                    || node.position.is_none()
                    || !node.scoring_container_ancestors.contains(&fit_root_tala_id)
                {
                    return;
                }
                fit_candidates
                    .entry(node.tala_id)
                    .or_insert_with(|| node.clone());
            };
            for node in &scope.nodes {
                add_fit_candidate(node);
            }
            for node in &scope.transaction_external_containers {
                add_fit_candidate(node);
            }
            for node in scope
                .transaction_external_container_children
                .values()
                .flatten()
            {
                add_fit_candidate(node);
            }
            for node in scope
                .transaction_external_aggregate_children
                .values()
                .flatten()
            {
                add_fit_candidate(node);
            }
            // A current scope's synthetic vessel has already arranged its
            // member pointers in the temporary local frame. The stable
            // member snapshots in transaction projections can still retain
            // their enclosing absolute positions; replace those boxes with
            // the arrangement geometry carried by the placement projection.
            let preserve_positioned_external_members = self.graph.nodes[container.0 as usize]
                .initial_position
                .is_some();
            for projection in &placement.projected_clusters {
                // A serialized/post-layout scope already carries the
                // upstream vessel's arranged geometry (including any
                // non-default gaps).  Go's combined graph preserves those
                // member boxes for FitToGraph; replacing them with a fresh
                // synthetic layout would erase that observable state.
                if preserve_positioned_external_members {
                    continue;
                }
                let Some(vessel_tala_id) = placement
                    .scoring_node_metadata
                    .iter()
                    .find(|metadata| metadata.node == projection.node)
                    .and_then(|metadata| metadata.tala_id)
                else {
                    continue;
                };
                let vessel_position = scope
                    .nodes
                    .iter()
                    .find(|node| node.tala_id == vessel_tala_id)
                    .and_then(|node| node.position)
                    .unwrap_or_default();
                // `place_flat_scope` may accept a Cluster.optimize flip while
                // laying out this induced scope.  Go's shared vessel pointer
                // is already in the committed arrangement when the enclosing
                // FitToGraph runs; use that published arrangement instead of
                // the projection snapshot captured before optimization.
                let arrangement = placed_scope
                    .cluster_arrangements
                    .get(&projection.cluster_index)
                    .copied()
                    .unwrap_or(projection.arrangement);
                let layout = match arrangement {
                    ClusterArrangement::Row => &projection.row,
                    ClusterArrangement::Column => &projection.column,
                };
                if crate::internal::trace_env_value("TALA_TRACE_FIT_PROJECTED").is_some_and(
                    |target| {
                        target == "all" || target.parse::<i64>().ok() == Some(fit_root_tala_id)
                    },
                ) {
                    eprintln!(
                        "FIT_PROJECTED_RUST root={} vessel={} arrangement={:?} vessel_pos={:?} layout_size={:?} members={:?}",
                        fit_root_tala_id,
                        vessel_tala_id,
                        arrangement,
                        vessel_position,
                        layout.size,
                        layout.members
                    );
                }
                // In the Go graph, CreateVessel replaces the whole member
                // subtree in the parent Graph.Nodes slice with one temporary
                // vessel.  The stable arena retains those member IDs for
                // routing and final publication, so the broad candidate set
                // above otherwise lets an expanded hidden container enlarge
                // its parent during fitNodeToGraph.  Remove the member
                // subtrees and retain one carrier box with the vessel's
                // independent position and pre-fit size.
                let member_tala_ids = layout.members.keys().copied().collect::<BTreeSet<_>>();
                let owner_tala_id = scope.nodes[projection.node.0 as usize].tala_id;
                let vessel_template = fit_candidates.get(&vessel_tala_id).cloned().or_else(|| {
                    fit_candidates.get(&owner_tala_id).cloned().or_else(|| {
                        scope
                            .nodes
                            .iter()
                            .find(|node| node.tala_id == owner_tala_id)
                            .cloned()
                    })
                });
                fit_candidates.retain(|tala_id, candidate| {
                    *tala_id != vessel_tala_id
                        && !member_tala_ids.contains(tala_id)
                        && !candidate
                            .scoring_container_ancestors
                            .iter()
                            .any(|ancestor| {
                                member_tala_ids.contains(ancestor) || *ancestor == vessel_tala_id
                            })
                });
                if let Some(mut vessel_candidate) = vessel_template {
                    vessel_candidate.position = Some(vessel_position);
                    vessel_candidate.rect.origin = vessel_position;
                    vessel_candidate.rect.size = layout.size;
                    vessel_candidate.tala_id = vessel_tala_id;
                    fit_candidates.insert(vessel_tala_id, vessel_candidate);
                }
                for (&member_tala_id, &(offset, size)) in &layout.members {
                    if let Some(candidate) = fit_candidates.get_mut(&member_tala_id) {
                        let position = Point {
                            x: vessel_position.x + offset.x,
                            y: vessel_position.y + offset.y,
                        };
                        candidate.position = Some(position);
                        candidate.rect.origin = position;
                        candidate.rect.size = size;
                    }
                }
            }
            // External projections from an already-fitted nested scope carry
            // the same hidden aggregate subtree that Go replaces with its
            // temporary vessel.  Normalize only those synthetic candidates;
            // ordinary container candidates remain available to the parent
            // fit and optimizer.
            let active_cluster_vessels = self
                .graph
                .clusters
                .iter()
                .enumerate()
                .map(|(index, cluster)| (cluster.vessel_tala_id, index))
                .collect::<BTreeMap<_, _>>();
            let active_cluster_vessel_ids = active_cluster_vessels
                .keys()
                .copied()
                .collect::<BTreeSet<_>>();
            let local_projected_cluster_indices = placement
                .projected_clusters
                .iter()
                .map(|projection| projection.cluster_index)
                .collect::<BTreeSet<_>>();
            let external_cluster_member_ids = self
                .graph
                .clusters
                .iter()
                .enumerate()
                .filter(|(index, _)| !local_projected_cluster_indices.contains(index))
                .flat_map(|(_, cluster)| {
                    cluster
                        .members
                        .iter()
                        .map(|member| self.graph.nodes[member.0 as usize].tala_id)
                })
                .collect::<BTreeSet<_>>();
            if preserve_positioned_external_members {
                // External aggregate projections retain the owner's absolute
                // member boxes.  CombineSubgraphs in Go places those direct
                // members back into the vessel's local frame before calling
                // FitToGraph, while deeper descendants keep their existing
                // translated coordinates.  Rebase only the direct member
                // boxes so a serialized/post-layout fixture sees the same
                // mixed frame as the reference graph.
                for cluster in &self.graph.clusters {
                    let member_ids = cluster
                        .members
                        .iter()
                        .map(|member| self.graph.nodes[member.0 as usize].tala_id)
                        .collect::<BTreeSet<_>>();
                    let Some((base_x, base_y)) = member_ids
                        .iter()
                        .filter_map(|id| fit_candidates.get(id).and_then(|node| node.position))
                        .map(|position| (position.x, position.y))
                        .reduce(|(x, y), (next_x, next_y)| (x.min(next_x), y.min(next_y)))
                    else {
                        continue;
                    };
                    for member_id in member_ids {
                        if let Some(candidate) = fit_candidates.get_mut(&member_id)
                            && let Some(position) = candidate.position
                        {
                            let rebased = Point {
                                x: position.x - base_x,
                                y: position.y - base_y,
                            };
                            candidate.position = Some(rebased);
                            candidate.rect.origin = rebased;
                        }
                    }
                }
            }
            if crate::internal::trace_env_value("TALA_TRACE_FIT_AGG_FILTER").is_some_and(|target| {
                target == "all" || target.parse::<i64>().ok() == Some(fit_root_tala_id)
            }) {
                eprintln!(
                    "FIT_AGG_FILTER_RUST root={} vessels={:?} local={:?} candidates={:?}",
                    fit_root_tala_id,
                    active_cluster_vessels,
                    local_projected_cluster_indices,
                    fit_candidates
                        .iter()
                        .filter(|(_, node)| node.scoring_is_aggregate_vessel)
                        .map(|(id, node)| (*id, node.rect.size, node.position))
                        .collect::<Vec<_>>()
                );
            }
            // The upstream combined graph keeps extracted descendants in its
            // Nodes slice.  When a scope is laid out from a nil origin, those
            // aliases are in the local frame and the active vessel is the
            // only box that should contribute to the parent fit.  A graph
            // supplied with an existing TopLeft, however, preserves the
            // absolute descendant aliases in Go; those boxes can extend
            // outside the vessel and deliberately enlarge the fit.  Retain
            // that distinction for serialized/post-layout fixtures.
            fit_candidates.retain(|tala_id, candidate| {
                if preserve_positioned_external_members {
                    return true;
                }
                if let Some(&cluster_index) = active_cluster_vessels.get(tala_id)
                    && !local_projected_cluster_indices.contains(&cluster_index)
                {
                    // CombineSubgraphs has already translated this shared
                    // vessel into the enclosing scope. The saved inner-scope
                    // position is only a fallback; restoring it here changes
                    // boundary-label padding in the parent's fixed bounds.
                    let vessel_position = candidate.position.or_else(|| {
                        self.graph
                            .pending_cluster_vessel_positions
                            .get(&cluster_index)
                            .copied()
                    });
                    candidate.position = vessel_position;
                    if let Some(position) = vessel_position {
                        candidate.rect.origin = position;
                    }
                    candidate.rect.size = self.graph.cluster_vessel_size(cluster_index);
                    return true;
                }
                // A direct retained cluster member is hidden by the vessel;
                // its descendants are hidden when their ancestor chain names
                // that vessel.  This leaves the parent fit with one box.
                if candidate.cluster.is_some()
                    && candidate.scoring_cluster_vessel.is_some_and(|vessel| {
                        active_cluster_vessels
                            .get(&vessel)
                            .is_none_or(|index| !local_projected_cluster_indices.contains(index))
                    })
                {
                    return false;
                }
                !candidate
                    .scoring_container_ancestors
                    .iter()
                    .any(|ancestor| {
                        external_cluster_member_ids.contains(ancestor)
                            || (active_cluster_vessel_ids.contains(ancestor)
                                && active_cluster_vessels.get(ancestor).is_none_or(|index| {
                                    !local_projected_cluster_indices.contains(index)
                                }))
                    })
            });
            if crate::internal::trace_env_value("TALA_TRACE_FIT_AGG_FILTER").is_some_and(|target| {
                target == "all" || target.parse::<i64>().ok() == Some(fit_root_tala_id)
            }) {
                eprintln!(
                    "FIT_AGG_FILTERED_RUST root={} candidates={:?}",
                    fit_root_tala_id,
                    fit_candidates
                        .iter()
                        .filter(|(_, node)| node.scoring_is_aggregate_vessel)
                        .map(|(id, node)| (*id, node.rect.size, node.position))
                        .collect::<Vec<_>>()
                );
            }
            let mut fit_nodes = fit_candidates.into_values().collect::<Vec<_>>();
            // `CombineSubgraphs` keeps a cluster's temporary vessel *and*
            // its member pointers in the graph passed to Go's FitToGraph.
            // The Rust projection hides those members from ordinary packing,
            // so restore their positioned boxes for bounding purposes.  This
            // matters for member-only geometry such as `multiple`/3D
            // protrusions and outside labels: the vessel remains a 298px box,
            // while a multiple member can extend its effective bound by 10px.
            // Do not fold these extents into the vessel size itself; the
            // temporary carrier's dimensions are observable independently.
            if !preserve_positioned_external_members {
                let mut fit_ids = fit_nodes
                    .iter()
                    .map(|node| node.tala_id)
                    .collect::<BTreeSet<_>>();
                for projection in &placement.projected_clusters {
                    let arrangement = placed_scope
                        .cluster_arrangements
                        .get(&projection.cluster_index)
                        .copied()
                        .unwrap_or(projection.arrangement);
                    let layout = match arrangement {
                        ClusterArrangement::Row => &projection.row,
                        ClusterArrangement::Column => &projection.column,
                    };
                    let Some(vessel_tala_id) = placement
                        .scoring_node_metadata
                        .iter()
                        .find(|metadata| metadata.node == projection.node)
                        .and_then(|metadata| metadata.tala_id)
                    else {
                        continue;
                    };
                    let placed_vessel = scope
                        .nodes
                        .iter()
                        .find(|node| node.tala_id == vessel_tala_id);
                    let vessel_position = placed_vessel
                        .and_then(|node| node.position)
                        .unwrap_or_default();
                    // Cluster.SyncGeometry resizes the vessel for its current
                    // arrangement before ArrangeClusterNodes centers members
                    // on the vessel's cross axis (layoutgraph/cluster.go
                    // 74-82, 121-141). A herd can flip the arrangement after
                    // the projection was recorded with the previous vessel
                    // size, so center on the placed vessel instead.
                    let placed_vessel_size = placed_vessel.map(|node| node.rect.size);
                    for (&member_tala_id, &(mut offset, size)) in &layout.members {
                        if let Some(vessel_size) = placed_vessel_size {
                            match arrangement {
                                ClusterArrangement::Row => {
                                    offset.y =
                                        (vessel_size.height / 2.0 - size.height / 2.0).round();
                                }
                                ClusterArrangement::Column => {
                                    offset.x = (vessel_size.width / 2.0 - size.width / 2.0).round();
                                }
                            }
                        }
                        if !fit_ids.insert(member_tala_id) {
                            continue;
                        }
                        let Some(template) = scope
                            .nodes
                            .iter()
                            .find(|node| node.tala_id == member_tala_id)
                            .or_else(|| {
                                self.graph
                                    .nodes
                                    .iter()
                                    .find(|node| node.tala_id == member_tala_id)
                            })
                        else {
                            continue;
                        };
                        let position = Point {
                            x: vessel_position.x + offset.x,
                            y: vessel_position.y + offset.y,
                        };
                        let mut member = template.clone();
                        member.position = Some(position);
                        member.rect.origin = position;
                        member.rect.size = size;
                        fit_nodes.push(member);
                    }
                }
            }
            let fit_bounds = if fit_nodes.is_empty() {
                scope.fixed_node_bounds(&local_children)
            } else {
                ArenaGraph::fixed_external_node_bounds(&fit_nodes)
            };
            let Some((mut top_left, mut bottom_right)) = fit_bounds else {
                obstacle_frames.exit(Some(container));
                continue;
            };
            // `Node.fitNodeToGraph` calls `expandForLabels` between the
            // fixed-bounds pass and the shape fit.  That helper deliberately
            // considers every direct child label (including labels whose
            // final position is inside): when a boundary child has a label
            // wider than its box, reserve the centered overhang before
            // calling `GetDimensionsToFit`.  Image nodes with long labels
            // make this observable even when their labels are later placed
            // outside by the labeling stage.
            let direct_child_tala_ids = local_children
                .iter()
                .filter_map(|child| scope.nodes.get(child.0 as usize))
                .map(|child| child.tala_id)
                .chain(
                    placement
                        .tree_auxiliary_nodes
                        .iter()
                        .filter_map(|child| scope.nodes.get(child.0 as usize))
                        .map(|child| child.tala_id),
                )
                .collect::<BTreeSet<_>>();
            // A scope without external projections took its bounds directly
            // from local_children above. Those same child pointers still
            // participate in expandForLabels.
            let fit_children = fit_nodes.iter().chain(
                local_children
                    .iter()
                    .filter(|_| fit_nodes.is_empty())
                    .map(|child| &scope.nodes[child.0 as usize]),
            );
            ArenaGraph::expand_for_child_labels(
                &mut top_left,
                &mut bottom_right,
                fit_children.filter(|child| direct_child_tala_ids.contains(&child.tala_id)),
            );
            if crate::internal::trace_env_value("TALA_TRACE_FIT_CANDIDATES").is_some_and(|target| {
                target == "all"
                    || target
                        .parse::<i64>()
                        .ok()
                        .is_some_and(|id| id == self.graph.nodes[container.0 as usize].tala_id)
            }) {
                let root_tala_id = self.graph.nodes[container.0 as usize].tala_id;
                let mut candidates = BTreeMap::<
                    i64,
                    Vec<(
                        String,
                        Option<Point>,
                        Size,
                        Vec<i64>,
                        Option<usize>,
                        bool,
                        Option<i64>,
                    )>,
                >::new();
                for (source, nodes) in [
                    ("nodes", scope.nodes.clone()),
                    (
                        "ext-container",
                        scope.transaction_external_containers.clone(),
                    ),
                ] {
                    for node in nodes {
                        if node.tala_id == root_tala_id
                            || !node.scoring_container_ancestors.contains(&root_tala_id)
                        {
                            continue;
                        }
                        candidates.entry(node.tala_id).or_default().push((
                            source.to_string(),
                            node.position,
                            node.rect.size,
                            node.scoring_container_ancestors.clone(),
                            node.cluster,
                            node.scoring_is_aggregate_vessel,
                            node.scoring_cluster_vessel,
                        ));
                    }
                }
                for (source, maps) in [
                    (
                        "ext-child",
                        scope.transaction_external_container_children.clone(),
                    ),
                    (
                        "ext-aggregate",
                        scope.transaction_external_aggregate_children.clone(),
                    ),
                ] {
                    for (_parent, nodes) in maps {
                        for node in nodes {
                            if node.tala_id == root_tala_id
                                || !node.scoring_container_ancestors.contains(&root_tala_id)
                            {
                                continue;
                            }
                            candidates.entry(node.tala_id).or_default().push((
                                source.to_string(),
                                node.position,
                                node.rect.size,
                                node.scoring_container_ancestors.clone(),
                                node.cluster,
                                node.scoring_is_aggregate_vessel,
                                node.scoring_cluster_vessel,
                            ));
                        }
                    }
                }
                eprintln!("FIT_CANDIDATES_RUST root={} {:?}", root_tala_id, candidates);
                eprintln!(
                    "FIT_AGG_MAP_RUST root={} {:?}",
                    root_tala_id,
                    scope
                        .transaction_external_aggregate_children
                        .iter()
                        .map(|(parent, children)| (
                            *parent,
                            children
                                .iter()
                                .map(|node| (node.tala_id, node.position, node.rect.size))
                                .collect::<Vec<_>>()
                        ))
                        .collect::<Vec<_>>()
                );
            }
            if crate::internal::trace_env_value("TALA_TRACE_FIT_RAW").is_some_and(|target| {
                target == "all" || target.parse::<i64>().ok() == Some(fit_root_tala_id)
            }) {
                let dump = |source: &str, nodes: &[ArenaNode]| {
                    nodes
                        .iter()
                        .map(|node| {
                            (
                                source.to_string(),
                                node.tala_id,
                                node.position,
                                node.rect.origin,
                                node.rect.size,
                                node.container.map(|container| container.0),
                                node.scoring_container_ancestors.clone(),
                                node.is_container,
                                node.scoring_is_container,
                            )
                        })
                        .collect::<Vec<_>>()
                };
                eprintln!(
                    "FIT_RAW_RUST root={} {:?}",
                    fit_root_tala_id,
                    [
                        dump("nodes", &scope.nodes),
                        dump("ext-container", &scope.transaction_external_containers),
                        dump(
                            "ext-child",
                            &scope
                                .transaction_external_container_children
                                .values()
                                .flatten()
                                .cloned()
                                .collect::<Vec<_>>(),
                        ),
                        dump(
                            "ext-aggregate",
                            &scope
                                .transaction_external_aggregate_children
                                .values()
                                .flatten()
                                .cloned()
                                .collect::<Vec<_>>(),
                        ),
                    ]
                    .concat()
                );
            }
            if crate::internal::trace_env_enabled("TALA_TRACE_HIERARCHY_FIT")
                && self.graph.nodes[container.0 as usize].tala_id == 233611931
            {
                eprintln!(
                    "HIERARCHY_FIT_RUST container={} position={:?} size={:?} bounds={:?}->{:?} local_children={:?}",
                    self.graph.nodes[container.0 as usize].tala_id,
                    self.graph.position(container),
                    self.graph.nodes[container.0 as usize].rect.size,
                    top_left,
                    bottom_right,
                    local_children
                        .iter()
                        .map(|child| {
                            (
                                scope.nodes[child.0 as usize].tala_id,
                                scope.position(*child),
                                scope.nodes[child.0 as usize].rect.size,
                            )
                        })
                        .collect::<Vec<_>>()
                );
            }
            let local_content = Size {
                width: bottom_right.x - top_left.x,
                height: bottom_right.y - top_left.y,
            };
            // Recovered Graph.placeNodes calls getContainerPadding(root,
            // false), fitNodeToGraph, and GetInsidePlacement for every
            // hierarchy scope. Fixed bounds carry labels and render
            // modifiers; no graph-topology category is involved.
            let padding = self.graph.shape_fit_padding(container);
            if crate::internal::trace_env_enabled("TALA_TRACE_FIT_ALL") {
                eprintln!(
                    "FIT_ALL_RUST container={} pos={:?} old_size={:?} bounds={:?}->{:?} content={:?} padding={:?} children={:?}",
                    self.graph.nodes[container.0 as usize].tala_id,
                    self.graph.position(container),
                    self.graph.nodes[container.0 as usize].rect.size,
                    top_left,
                    bottom_right,
                    local_content,
                    padding,
                    fit_nodes
                        .iter()
                        .map(|node| (node.tala_id, node.position, node.rect.size))
                        .collect::<Vec<_>>(),
                );
            }
            let fit_trace_root = self.graph.nodes[container.0 as usize].external_id.clone();
            crate::internal::pipeline::trace_placement_state(
                "before-fit",
                &fit_trace_root,
                &self.graph,
            );
            self.graph.nodes[container.0 as usize].rect.size = self
                .graph
                .bin_pack_shape_dimensions_to_fit(container, local_content, padding);
            crate::internal::pipeline::trace_placement_state(
                "after-fit-size",
                &fit_trace_root,
                &self.graph,
            );
            // Upstream emits `after-fit` immediately after FitToGraph too;
            // cluster resizing and owner copyback happen after that callback.
            crate::internal::pipeline::trace_placement_state(
                "after-fit",
                &fit_trace_root,
                &self.graph,
            );
            if crate::internal::trace_env_enabled("TALA_TRACE_NESTED_FIT_SIZE")
                && matches!(
                    self.graph.nodes[container.0 as usize].tala_id,
                    3946625030 | 2328842519 | 3984593615
                )
            {
                eprintln!(
                    "NESTED_FIT_SIZE_RUST container={} size={:?} content={:?}",
                    self.graph.nodes[container.0 as usize].tala_id,
                    self.graph.nodes[container.0 as usize].rect.size,
                    local_content
                );
            }
            // `fitNodeToGraph` updates the shape box before
            // `GetInsidePlacement`. Diamond, circle, cloud, package, and
            // other shape implementations derive their inner origin from
            // those fitted dimensions.
            let inside =
                self.graph
                    .bin_pack_shape_inside_placement(container, local_content, padding);
            self.graph.nodes[container.0 as usize].unpositioned_scope_translation =
                self.graph.position(container).is_none().then_some(Point {
                    x: container_origin.x + inside.x - top_left.x,
                    y: container_origin.y + inside.y - top_left.y,
                });
            self.graph.nodes[container.0 as usize].scope_translation_materialized = false;
            let mut shared_mirror_replays = Vec::new();
            for child in children {
                let Some(&local) = placement.old_to_new.get(&child) else {
                    // A materialized, edge-isolated child container is kept
                    // only as an external transaction carrier, mirroring
                    // SplitSubgraphs' Graph.Containers pointer. It has no
                    // active projection to copy back in this scope.
                    continue;
                };
                let mut position = scope.position(local).expect("placed scope child");
                if let Some((owner, offset, _)) =
                    self.scope_aggregate_member_geometry(Some(container), child)
                    && placement.old_to_new.get(&owner) == Some(&local)
                {
                    position = Point {
                        x: position.x + offset.x,
                        y: position.y + offset.y,
                    };
                }
                self.graph
                    .publish_herd_projection(child, scope.herd_assignment_snapshot(local));
                // ReconnectTree publishes retained branching-tree nodes in
                // the same Containers slice as ordinary children, but their
                // coordinates are copied by the tree-specific publication
                // below.  Do not run the ordinary container move first: Go's
                // shared pointer would otherwise receive the inner placement
                // translation twice.
                if placement.tree_auxiliary_nodes.contains(&local) {
                    continue;
                }
                // The Go temporary graph retains the main graph's Node
                // pointers. InitializeNodes therefore publishes an initial
                // `(0, 0)` before later moves, and moveNodeWithChildren
                // translates an already-fitted descendant subtree. Rust
                // scopes clone nodes, so materialize that shared-pointer
                // state before copying the final scope position back.
                if self.graph.position(child).is_none() {
                    self.graph.set_position(child, Point::default());
                }
                // Recovered Graph.placeNodes leaves a recursively fitted
                // container's children in the temporary graph's local frame,
                // even when the owner already has a serialized TopLeft. The
                // shared Go pointer is published directly here; the fitted
                // container's inside offset is applied only when its parent
                // later moves that carrier. Converting to the owner's
                // absolute frame at this boundary shifts pre-positioned
                // snapshots twice (and differs from graph.input, where the
                // owner happened to start nil).
                let target = position;
                // Go's temporary child graph and its owner share the same
                // node pointer, so a nested placement's fitted dimensions
                // are already visible to the parent's after-child-recursion
                // and fit callbacks. Rust scopes own cloned nodes; publish
                // the fitted box size alongside the position copyback before
                // the enclosing scope computes its bounds.
                let aggregate_member = self.graph.active_node_is_aggregate(child);
                // AddCluster/AddSequence project every member through the
                // same vessel node. The placement graph therefore exposes the
                // vessel dimensions for every member ID, while the stable
                // arena stores each member's own box. Preserve those member
                // dimensions at recursive copyback.
                if !aggregate_member {
                    self.graph.nodes[child.0 as usize].rect.size =
                        scope.nodes[local.0 as usize].rect.size;
                }
                let trace_copyback = crate::internal::trace_env_value(
                    "TALA_TRACE_HIERARCHY_COPYBACK",
                )
                .is_some_and(|target| {
                    target == "all"
                        || target == self.graph.nodes[container.0 as usize].tala_id.to_string()
                        || target == self.graph.nodes[child.0 as usize].tala_id.to_string()
                });
                if trace_copyback {
                    eprintln!(
                        "HIERARCHY_COPYBACK_RUST container={} child={} container_pos={:?} top_left={:?} inside={:?} scope_pos={:?} target={:?} stable_before={:?}",
                        self.graph.nodes[container.0 as usize].tala_id,
                        self.graph.nodes[child.0 as usize].tala_id,
                        self.graph.position(container),
                        top_left,
                        inside,
                        position,
                        target,
                        self.graph.position(child)
                    );
                }
                let published_scope_geometry = !self.graph.nodes[child.0 as usize]
                    .scoring_is_aggregate_vessel
                    && self.publish_scope_subtree_geometry(&scope, local, child, target);
                if !published_scope_geometry {
                    self.graph.move_node_abs_with_children(child, target);
                }
                if trace_copyback {
                    eprintln!(
                        "HIERARCHY_COPYBACK_RUST_AFTER container={} child={} stable_after={:?} descendants={:?}",
                        self.graph.nodes[container.0 as usize].tala_id,
                        self.graph.nodes[child.0 as usize].tala_id,
                        self.graph.position(child),
                        self.graph
                            .active_descendants(child)
                            .into_iter()
                            .filter_map(|descendant| {
                                self.graph.position(descendant).map(|position| {
                                    (self.graph.nodes[descendant.0 as usize].tala_id, position)
                                })
                            })
                            .collect::<Vec<_>>()
                    );
                }
                // A published subtree already contains the directed mirror's
                // descendant positions. Replaying that mirror on the stable
                // aliases would apply the reflection twice; retain a replay
                // only when publication had to fall back to a plain move.
                // The active cluster vessel is not a container in Go's
                // mirrorAxes walk. Its members have already been published
                // from the directed scope; replaying through the Rust
                // aggregate projection would translate them a second time.
                if !published_scope_geometry
                    && !self.graph.clusters.iter().any(|cluster| {
                        cluster.vessel_tala_id == self.graph.nodes[child.0 as usize].tala_id
                    })
                    && let Some(mirror) = placed_scope.descendant_mirrors.get(&local)
                {
                    shared_mirror_replays.push((child, target, mirror.clone()));
                }
                if let Some(cluster_index) = self.graph.nodes[child.0 as usize].cluster
                    && self.graph.clusters[cluster_index].members.first() == Some(&child)
                {
                    // The stable arena has no separate vessel node. The
                    // aggregate geometry correction above moves the retained
                    // carrier to the vessel's actual shared-pointer frame;
                    // publish that corrected target as the vessel position as
                    // well. Keeping the pre-correction projected coordinate
                    // here leaves a stale pending vessel (even though the
                    // visible member is at the recovered position), and the
                    // next AffectContainers transaction then re-arranges the
                    // cluster from the wrong origin.
                    // `target` is the retained member's local geometry.  A
                    // nested member can be wider than the temporary vessel,
                    // so its centered offset may be negative even though Go's
                    // distinct vessel remains at the projected vessel origin.
                    // Preserve that independent origin for the parent fit.
                    let vessel_position = placed_scope
                        .cluster_vessel_positions
                        .get(&cluster_index)
                        .copied()
                        .unwrap_or(target);
                    self.graph
                        .pending_cluster_vessel_positions
                        .insert(cluster_index, vessel_position);
                }
            }
            if crate::internal::trace_env_enabled("TALA_TRACE_PUBLISH_CALLS") {
                eprintln!(
                    "PUBLISH_SCOPE_RUST container={} offsets_len={}",
                    self.graph.nodes[container.0 as usize].tala_id,
                    external_shared_child_offsets.len()
                );
            }
            // Ordinary container children are copied back through the shared
            // child loop above. Replaying every flattened parent/child offset
            // here moves those same pointers a second time (the Go graph's
            // aliases already reflect the direct move). The descendant replay
            // below is separate: it mirrors the source graph's postorder walk.
            let _ = external_shared_child_offsets;
            // Graph.direct mirrors the temporary graph's active carrier and,
            // through shared pointers, walks its hidden descendants in
            // postorder. The cloned scope already supplied the carrier's
            // directed position. Replay the descendant walk only after
            // publishing projected child offsets, then restore the carrier to
            // that directed target so it is not mirrored twice.
            for (child, target, mirror) in shared_mirror_replays {
                let trace_mirror_replay =
                    crate::internal::trace_env_value("TALA_TRACE_MIRROR_REPLAY_TARGET")
                        .is_some_and(|value| {
                            value == "all"
                                || value.split(',').any(|id| {
                                    id.trim().parse::<i64>().ok()
                                        == Some(self.graph.nodes[child.0 as usize].tala_id)
                                })
                        });
                let trace_mirror_state = |phase: &str, graph: &ArenaGraph| {
                    if !trace_mirror_replay {
                        return;
                    }
                    let stable = graph
                        .nodes
                        .iter()
                        .filter(|node| matches!(node.tala_id, 4254926550 | 1369972579 | 3471367806))
                        .map(|node| {
                            (
                                node.tala_id,
                                node.position,
                                node.rect.origin,
                                node.rect.size,
                            )
                        })
                        .collect::<Vec<_>>();
                    let external = graph
                        .transaction_external_container_children
                        .get(&4254926550)
                        .map(|children| {
                            children
                                .iter()
                                .map(|node| {
                                    (
                                        node.tala_id,
                                        node.position,
                                        node.rect.origin,
                                        node.rect.size,
                                    )
                                })
                                .collect::<Vec<_>>()
                        });
                    eprintln!(
                        "MIRROR_REPLAY_TARGET_RUST phase={} child={} target={:?} mirror=({}, {}) stable={:?} external={:?}",
                        phase,
                        graph.nodes[child.0 as usize].tala_id,
                        target,
                        mirror.mirror_x,
                        mirror.mirror_y,
                        stable,
                        external
                    );
                };
                trace_mirror_state("before", &self.graph);
                // The temporary cluster vessel and its first stable member
                // are distinct Go pointers. Rust aliases both to `child`, so
                // restoring the member after the descendant replay must not
                // apply the same delta to the already-directed vessel box.
                let direct_cluster_vessel = self.graph.nodes[child.0 as usize]
                    .cluster
                    .filter(|&cluster_index| {
                        self.graph.clusters[cluster_index].members.first() == Some(&child)
                    })
                    .and_then(|cluster_index| {
                        self.graph
                            .pending_cluster_vessel_positions
                            .get(&cluster_index)
                            .copied()
                            .map(|position| (cluster_index, position))
                    });
                self.graph.mirror_subtree_axes(
                    child,
                    mirror.mirror_x,
                    mirror.mirror_y,
                    &mirror.reachable_containers,
                );
                trace_mirror_state("after-mirror", &self.graph);
                self.graph.move_node_abs_with_children(child, target);
                trace_mirror_state("after-move", &self.graph);
                if let Some((cluster_index, vessel_position)) = direct_cluster_vessel {
                    self.graph
                        .pending_cluster_vessel_positions
                        .insert(cluster_index, vessel_position);
                }
            }
            self.trace_hierarchy_after_fit_state(container, "after-mirror-replay");
            self.publish_mirrored_external_child_offsets(&mirrored_external_child_offsets);
            self.trace_hierarchy_after_fit_state(container, "after-mirrored-external-offsets");
            #[cfg(feature = "diagnostic-traces")]
            if crate::internal::trace_env_value("TALA_TRACE_SCOPE_CHILD_COPYBACK").as_deref()
                == Some(self.graph.nodes[container.0 as usize].external_id.as_str())
            {
                eprintln!(
                    "SCOPE_CHILD_COPYBACK_RUST root={} owner={:?} scope={:?} scope_order={:?} mapping={:?}",
                    self.graph.nodes[container.0 as usize].external_id,
                    self.graph
                        .containers
                        .get(&Some(container))
                        .map(|children| children
                            .iter()
                            .map(|id| self.graph.nodes[id.0 as usize].external_id.as_str())
                            .collect::<Vec<_>>()),
                    scope.containers.get(&None).map(|children| children
                        .iter()
                        .map(|id| scope.nodes[id.0 as usize].external_id.as_str())
                        .collect::<Vec<_>>()),
                    scope
                        .node_order
                        .iter()
                        .map(|id| scope.nodes[id.0 as usize].external_id.as_str())
                        .collect::<Vec<_>>(),
                    placement
                        .old_to_new
                        .iter()
                        .map(|(old, local)| (
                            &self.graph.nodes[old.0 as usize].external_id,
                            &scope.nodes[local.0 as usize].external_id
                        ))
                        .collect::<Vec<_>>(),
                );
            }
            // Go restores retained tree nodes by walking subgraph.Nodes, so
            // the owning Graph.Nodes and Containers slices inherit that
            // component order. `old_to_new` is map-backed here; iterating it
            // directly moves the same nodes but can permute their observable
            // traversal order. Preserve the temporary graph's live order and
            // use the mapping order only for any retained node it omits.
            let old_by_local = placement
                .old_to_new
                .iter()
                .map(|(&old, &local)| (local, old))
                .collect::<BTreeMap<_, _>>();
            let mut tree_copyback = Vec::new();
            let mut copied_tree_nodes = BTreeSet::new();
            for &local in &scope.node_order {
                let Some(&old) = old_by_local.get(&local) else {
                    continue;
                };
                if self.graph.tree_routing_nodes.contains_key(&old)
                    && placement.tree_auxiliary_nodes.contains(&local)
                    && copied_tree_nodes.insert(old)
                {
                    tree_copyback.push((old, local));
                }
            }
            for (&old, &local) in &placement.old_to_new {
                if self.graph.tree_routing_nodes.contains_key(&old)
                    && placement.tree_auxiliary_nodes.contains(&local)
                    && copied_tree_nodes.insert(old)
                {
                    tree_copyback.push((old, local));
                }
            }
            for (old, local) in tree_copyback {
                let Some(position) = scope.position(local) else {
                    continue;
                };
                if self.graph.position(old).is_none() {
                    self.graph.set_position(old, Point::default());
                }
                if !self.graph.node_order.contains(&old) {
                    self.graph.node_order.push(old);
                }
                let target = Point {
                    x: position.x - top_left.x,
                    y: position.y - top_left.y,
                };
                self.graph.move_node_abs_with_children(old, target);
                if let Some(projected) = scope.tree_routing_nodes.get(&local)
                    && let Some(state) = self.graph.tree_routing_nodes.get_mut(&old)
                {
                    if crate::internal::trace_env_enabled("TALA_TRACE_TREE_ORIENT")
                        && matches!(
                            self.graph.nodes[old.0 as usize].tala_id,
                            1171112347 | 1154334728 | 1271778061 | 1187889966
                        )
                    {
                        eprintln!(
                            "TREE_ORIENT_SCOPE_COPY_RUST tala={} from={:?} to={:?}",
                            self.graph.nodes[old.0 as usize].tala_id,
                            state.orientation,
                            projected.orientation
                        );
                    }
                    state.orientation = projected.orientation;
                    self.graph
                        .publish_tree_orientation(old, projected.orientation);
                }
            }
            self.graph.publish_placed_tree_edges(
                &self
                    .graph
                    .containers
                    .get(&Some(container))
                    .unwrap_or_default(),
                false,
            );
            // Go assigns the restored subgraph header to the owner. These
            // compact arenas use independent node namespaces, so this is
            // still a fresh make(len) projection of visible children. Source
            // capacity, offset, stale slots, and shared-header copyback remain
            // unqualified until retained canonical allocations are available.
            if let Some(local_children) = scope.containers.get(&None) {
                let owner_children = local_children
                    .iter()
                    .map(|local| {
                        *old_by_local
                            .get(local)
                            .expect("tree copyback child requires owner lineage")
                    })
                    .collect::<Vec<_>>();
                self.graph
                    .containers
                    .insert(Some(container), owner_children);
            }
            #[cfg(feature = "diagnostic-traces")]
            self.graph.trace_tree_copyback("published", Some(container));
            self.trace_hierarchy_after_fit_state(container, "after-tree-copyback");
            if crate::internal::trace_env_enabled("TALA_TRACE_TREE_COPYBACK_SET")
                && self.graph.nodes[container.0 as usize].tala_id == 1472025070
            {
                eprintln!(
                    "TREE_COPYBACK_SET_RUST container={} aux={:?} mapped={:?} children={:?}",
                    self.graph.nodes[container.0 as usize].tala_id,
                    placement
                        .tree_auxiliary_nodes
                        .iter()
                        .map(|local| scope.nodes[local.0 as usize].tala_id)
                        .collect::<Vec<_>>(),
                    placement
                        .old_to_new
                        .iter()
                        .filter(|(_, local)| placement.tree_auxiliary_nodes.contains(local))
                        .map(|(old, _)| self.graph.nodes[old.0 as usize].tala_id)
                        .collect::<Vec<_>>(),
                    self.graph
                        .containers
                        .get(&Some(container))
                        .into_iter()
                        .flatten()
                        .map(|node| self.graph.nodes[node.0 as usize].tala_id)
                        .collect::<Vec<_>>()
                );
            }
            // The upstream recursive placeNodes path only fits the container
            // here. Its descendants remain in the local frame until the
            // parent scope places the fitted carrier; calling
            // positionContainerChildren at this boundary shifts witnesses
            // such as routing_through_center before the parent optimizer.
            self.graph.resize_clusters_containing(container);
            self.trace_hierarchy_after_fit_state(container, "after-cluster-resize");
            if let Some(owner) = &prepared_owner {
                self.restore_prepared_owner_scope(owner)?;
                frames.retain(|(key, _)| *key != Some(container));
            } else {
                self.graph
                    .restored_hierarchy_endpoint_order
                    .extend(placement.restored_endpoint_order.iter().copied());
            }
            #[cfg(feature = "diagnostic-traces")]
            {
                // PlaceChildrenOrder returns a separate recursion worklist;
                // it does not mutate Go's Containers[root]. Publish the live
                // owner slice after tree restoration, not that worklist.
                let parent_tala_id = self.graph.nodes[container.0 as usize].tala_id;
                if let Some(children) = self.graph.containers.get(&Some(container)) {
                    let ordered_children = children
                        .iter()
                        .map(|child| self.graph.nodes[child.0 as usize].clone())
                        .collect::<Vec<_>>();
                    self.graph
                        .trace_container_children
                        .insert(parent_tala_id, ordered_children);
                    #[cfg(feature = "diagnostic-traces")]
                    if crate::internal::trace_env_value("TALA_TRACE_SCOPE_CHILD_COPYBACK")
                        .as_deref()
                        == Some(self.graph.nodes[container.0 as usize].external_id.as_str())
                    {
                        eprintln!(
                            "SCOPE_CHILD_COPYBACK_AFTER_RUST root={} owner={:?} trace={:?}",
                            self.graph.nodes[container.0 as usize].external_id,
                            self.graph
                                .containers
                                .get(&Some(container))
                                .map(|children| children
                                    .iter()
                                    .map(|child| self.graph.nodes[child.0 as usize]
                                        .external_id
                                        .as_str())
                                    .collect::<Vec<_>>()),
                            self.graph
                                .trace_container_children
                                .get(&parent_tala_id)
                                .map(|children| children
                                    .iter()
                                    .map(|child| child.external_id.as_str())
                                    .collect::<Vec<_>>())
                        );
                    }
                }
            }
            // Go's recursive placement refreshes a child's self-loop spacing
            // immediately after that child has been fitted, before the
            // parent's temporary graph is built.  The initial preprocessing
            // pass computes offsets from the declared box, so a container
            // whose dimensions grew while its descendants were placed would
            // otherwise carry stale loop margins into the parent optimizer.
            self.graph.update_loop_offsets(container);
            if crate::internal::trace_env_enabled("TALA_TRACE_LOOPS")
                && self.graph.nodes[container.0 as usize]
                    .edges
                    .read()
                    .iter()
                    .any(|edge| {
                        let edge = &self.graph.edge_records[edge.0 as usize];
                        edge.from == container && edge.to == container
                    })
            {
                eprintln!(
                    "LOOP_OFFSETS_RUST_UPDATE id={} shape={:?} size={:?} offsets={:?}",
                    self.graph.nodes[container.0 as usize].tala_id,
                    self.graph.nodes[container.0 as usize].shape,
                    self.graph.nodes[container.0 as usize].rect.size,
                    self.graph.nodes[container.0 as usize].loop_offsets
                );
            }
            self.graph.nodes[container.0 as usize].scope_translation_materialized = true;
            if crate::internal::trace_env_enabled("TALA_TRACE_RECURSIVE_GLOBAL") {
                let root_tala_id = self.graph.nodes[container.0 as usize].tala_id;
                eprint!("RECURSIVE_GLOBAL_RUST root={root_tala_id}");
                for node in &self.graph.nodes {
                    if let Some(position) = node.position {
                        eprint!(" {}={},{}", node.tala_id, position.x, position.y);
                    }
                }
                eprintln!();
            }
            // This scope's direct child containers now have real TopLeft
            // values. The carrier `container` itself remains nil until its
            // parent scope is optimized.
            materialized_parent_scopes.insert(Some(container));
            obstacle_frames.exit(Some(container));
            if crate::internal::trace_env_enabled("TALA_TRACE_NESTING_POWER_WITNESS") {
                let witness = self
                    .graph
                    .nodes
                    .iter()
                    .find(|node| node.external_id == "directions.l");
                eprintln!(
                    "NESTING_WITNESS_RUST phase=exit container={} container_pos={:?} witness_pos={:?} witness_translation={:?} witness_materialized={}",
                    self.graph.nodes[container.0 as usize].external_id,
                    self.graph.position(container),
                    witness.and_then(|node| node.position),
                    witness.and_then(|node| node.unpositioned_scope_translation),
                    witness.is_some_and(|node| node.scope_translation_materialized),
                );
            }
        }

        if crate::internal::trace_env_enabled("TALA_TRACE_ROOT_INPUT") {
            let tracked = [
                2043751575_i64,
                3677471320,
                2884276447,
                1010209959,
                1231386872,
                2204119069,
                4137376658,
                1323527919,
                2801930458,
                696109821,
                3554470064,
                2202785448,
                6129484611666145821,
            ];
            eprintln!(
                "ROOT_INPUT_RUST nodes={:?} containers={:?} ordinary={:?} aggregate={:?}",
                self.graph
                    .nodes
                    .iter()
                    .filter(|node| tracked.contains(&node.tala_id))
                    .map(|node| (
                        node.tala_id,
                        node.position,
                        node.rect.size,
                        node.container
                            .map(|parent| self.graph.nodes[parent.0 as usize].tala_id)
                    ))
                    .collect::<Vec<_>>(),
                self.graph
                    .transaction_external_containers
                    .iter()
                    .filter(|node| tracked.contains(&node.tala_id))
                    .map(|node| (node.tala_id, node.position, node.rect.size))
                    .collect::<Vec<_>>(),
                self.graph
                    .transaction_external_container_children
                    .iter()
                    .filter(|(parent, _)| tracked.contains(parent))
                    .map(|(parent, children)| (
                        *parent,
                        children
                            .iter()
                            .filter(|child| tracked.contains(&child.tala_id))
                            .map(|child| (child.tala_id, child.position, child.rect.size))
                            .collect::<Vec<_>>()
                    ))
                    .collect::<Vec<_>>(),
                self.graph
                    .transaction_external_aggregate_children
                    .iter()
                    .filter(|(parent, _)| tracked.contains(parent))
                    .map(|(parent, children)| (
                        *parent,
                        children
                            .iter()
                            .filter(|child| tracked.contains(&child.tala_id))
                            .map(|child| (child.tala_id, child.position, child.rect.size))
                            .collect::<Vec<_>>()
                    ))
                    .collect::<Vec<_>>()
            );
        }
        #[cfg(feature = "diagnostic-traces")]
        {
            crate::internal::pipeline::trace_placement_state(
                "after-child-recursion",
                "top",
                &self.graph,
            );
            crate::internal::pipeline::trace_placement_state("after-nears", "top", &self.graph);
        }
        let existing_owner = frames
            .iter()
            .find(|(key, _)| key.is_none())
            .map(|(_, owner)| owner.clone());
        let prepared = if let Some(owner) = existing_owner {
            self.refresh_prepared_placement_scope(None, &materialized_parent_scopes, &owner)
                .map(|placement| (placement, Some(owner)))
        } else {
            self.prepare_placement_scope_after(None, &materialized_parent_scopes)?
        };
        let Some((placement, prepared_owner)) = prepared else {
            return Ok(false);
        };
        #[cfg(feature = "diagnostic-traces")]
        crate::internal::pipeline::trace_placement_state("after-herds", "top", &self.graph);
        let placed_root =
            self.solve_prepared_owner_scope_inner(&placement, prepared_owner.as_ref())?;
        if crate::internal::trace_env_enabled("TALA_TRACE_ROOT_STEPS") {
            eprintln!("ROOT_STEP_RUST after_place_flat");
        }
        self.next_rng_float = placed_root.next_rng_float;
        self.copy_tree_edge_label_placements(&placement, &placed_root.graph);
        if crate::internal::trace_env_enabled("TALA_TRACE_CLUSTER_STATE") {
            eprintln!(
                "CLUSTER_STATE_OWNER_RUST phase=root maps_desired={:?} maps_arrangements={:?} owner={:?}",
                placed_root.cluster_desired_arrangements,
                placed_root.cluster_arrangements,
                self.graph
                    .clusters
                    .iter()
                    .map(|cluster| (
                        cluster.vessel_tala_id,
                        cluster.arrangement,
                        cluster.desired_arrangement
                    ))
                    .collect::<Vec<_>>()
            );
        }
        self.publish_accepted_cluster_policies(
            &placed_root.cluster_arrangements,
            &placed_root.cluster_desired_arrangements,
            &placed_root.graph,
        )?;
        // Nested scopes have already published absolute vessel positions while
        // their owning containers were materialized above. The root temporary
        // graph stores those same nested projections in local scope
        // coordinates; overwriting them here would subtract the parent
        // container origin. A root-owned cluster has no such materialized
        // parent, however: Graph.placeNodes publishes its root projection as
        // the vessel position that BinPack consumes. Preserve nested entries,
        // but overwrite root-owned entries with the root scope result.
        for (&cluster_index, &position) in &placed_root.cluster_vessel_positions {
            let root_owned = self.graph.clusters[cluster_index]
                .members
                .first()
                .and_then(|member| self.graph.nodes[member.0 as usize].container)
                .is_none();
            if root_owned {
                self.graph
                    .pending_cluster_vessel_positions
                    .insert(cluster_index, position);
            } else {
                self.graph
                    .pending_cluster_vessel_positions
                    .entry(cluster_index)
                    .or_insert(position);
            }
            // Source CombineSubgraphs moves the shared vessel pointer itself
            // (node_placement.go:217), not only a numerical carrier cache.
            // Preserve the already materialized nested frame selected above.
            if let Some(vessel) = self
                .graph
                .topology_vessel_node(TopologyVessel::Cluster(cluster_index))
            {
                let published = self.graph.pending_cluster_vessel_positions[&cluster_index];
                Self::publish_induced_owner_point(&mut self.graph, vessel, Some(published));
            }
        }
        let external_shared_child_offsets = placed_root.external_shared_child_offsets.clone();
        let mirrored_external_child_offsets = placed_root.mirrored_external_child_offsets.clone();
        let root_cluster_member_positions = placed_root
            .graph
            .transaction_external_aggregate_children
            .values()
            .flatten()
            .filter_map(|node| node.position.map(|position| (node.tala_id, position)))
            .collect::<BTreeMap<_, _>>();
        if crate::internal::trace_env_enabled("TALA_TRACE_BINPACK_GROUPS") {
            eprintln!(
                "ROOT_CLUSTER_VESSEL_POS_RUST {:?}",
                placed_root
                    .cluster_vessel_positions
                    .iter()
                    .map(|(cluster, position)| (
                        self.graph.clusters[*cluster].vessel_tala_id,
                        *position
                    ))
                    .collect::<Vec<_>>()
            );
        }
        if crate::internal::trace_env_enabled("TALA_TRACE_EXTERNAL_OFFSETS_ALL") {
            eprintln!(
                "ROOT_EXTERNAL_OFFSETS_RUST {:?}",
                external_shared_child_offsets
            );
            eprintln!(
                "ROOT_AGGREGATE_CHILDREN_RUST {:?}",
                placed_root
                    .graph
                    .transaction_external_aggregate_children
                    .iter()
                    .map(|(vessel, children)| (
                        *vessel,
                        children
                            .iter()
                            .map(|child| (child.tala_id, child.position, child.rect.size))
                            .collect::<Vec<_>>()
                    ))
                    .collect::<Vec<_>>()
            );
            eprintln!(
                "ROOT_CLUSTER_MEMBERS_RUST {:?}",
                self.graph
                    .clusters
                    .iter()
                    .map(|cluster| (
                        cluster.vessel_tala_id,
                        cluster
                            .members
                            .iter()
                            .map(|member| {
                                let node = &self.graph.nodes[member.0 as usize];
                                (node.tala_id, node.position, node.rect.size)
                            })
                            .collect::<Vec<_>>()
                    ))
                    .collect::<Vec<_>>()
            );
            eprintln!(
                "ROOT_MIRRORED_OFFSETS_RUST {:?}",
                mirrored_external_child_offsets
            );
        }
        if crate::internal::trace_env_enabled("TALA_TRACE_GRID_OFFSETS") {
            eprintln!(
                "GRID_ROOT_OFFSETS_RUST external={:?} mirrored={:?}",
                external_shared_child_offsets
                    .iter()
                    .filter(|(container, child, _)| {
                        matches!(
                            (*container, *child),
                            (233611931, 1149337423)
                                | (233611931, 1782109120)
                                | (233611931, 498183754)
                                | (233611931, 2919616609)
                                | (233611931, 3584521799)
                        )
                    })
                    .collect::<Vec<_>>(),
                mirrored_external_child_offsets
                    .iter()
                    .filter(|(container, child, _)| {
                        matches!(
                            (*container, *child),
                            (233611931, 1149337423)
                                | (233611931, 1782109120)
                                | (233611931, 498183754)
                                | (233611931, 2919616609)
                                | (233611931, 3584521799)
                        )
                    })
                    .collect::<Vec<_>>()
            );
        }
        let root_had_mirrors = !placed_root.descendant_mirrors.is_empty();
        if crate::internal::trace_env_enabled("TALA_TRACE_ROOT_STEPS") {
            eprintln!(
                "ROOT_MIRROR_RUST had_mirrors={} count={}",
                root_had_mirrors,
                placed_root.descendant_mirrors.len()
            );
        }
        self.publish_placed_tree_orientations(&placed_root.graph);
        let root = placed_root.graph;
        if crate::internal::trace_env_enabled("TALA_TRACE_ROOT_CLUSTER6") {
            eprintln!(
                "ROOT_CLUSTER6_RUST phase=after-place-root root_nodes={:?} root_aggregate={:?} stable={:?} pending={:?}",
                root.nodes
                    .iter()
                    .filter(|node| node.tala_id == 6)
                    .map(|node| (
                        node.position,
                        node.rect.origin,
                        node.rect.size,
                        node.container
                    ))
                    .collect::<Vec<_>>(),
                root.transaction_external_aggregate_children
                    .iter()
                    .map(|(parent, children)| (
                        *parent,
                        children
                            .iter()
                            .map(|child| (
                                child.tala_id,
                                child.position,
                                child.rect.origin,
                                child.rect.size
                            ))
                            .collect::<Vec<_>>()
                    ))
                    .collect::<Vec<_>>(),
                self.graph
                    .nodes
                    .iter()
                    .filter(|node| node.tala_id == 6)
                    .map(|node| (
                        node.position,
                        node.rect.origin,
                        node.rect.size,
                        node.container
                    ))
                    .collect::<Vec<_>>(),
                self.graph.pending_cluster_vessel_positions,
            );
        }
        if crate::internal::trace_env_enabled("TALA_TRACE_HIDDEN_EXTERNAL") {
            eprintln!(
                "ROOT_TEMP_TARGETS_RUST {:?}",
                root.nodes
                    .iter()
                    .filter(|node| matches!(
                        node.tala_id,
                        2043751575 | 3248328362 | 457364737 | 3680227035 | 1832207848 | 3777840222
                    ))
                    .map(|node| (node.tala_id, node.position, node.rect.size))
                    .collect::<Vec<_>>()
            );
        }
        if crate::internal::trace_env_enabled("TALA_TRACE_ROOT_TEMP_GEOMETRY") {
            eprintln!(
                "ROOT_TEMP_GEOMETRY_RUST nodes={:?} external={:?} children={:?}",
                root.nodes
                    .iter()
                    .filter(|node| matches!(
                        node.tala_id,
                        233611931
                            | 1149337423
                            | 1782109120
                            | 3536874587
                            | 370564473
                            | 727019916
                            | 777352773
                            | 760575154
                            | 676687059
                    ))
                    .map(|node| (node.tala_id, node.position, node.rect.size, node.container))
                    .collect::<Vec<_>>(),
                root.transaction_external_containers
                    .iter()
                    .filter(|node| matches!(
                        node.tala_id,
                        1149337423
                            | 1782109120
                            | 3536874587
                            | 370564473
                            | 727019916
                            | 777352773
                            | 760575154
                            | 676687059
                    ))
                    .map(|node| (node.tala_id, node.position, node.rect.size))
                    .collect::<Vec<_>>(),
                root.transaction_external_container_children
                    .iter()
                    .filter(|(parent, _)| matches!(
                        **parent,
                        1149337423 | 1782109120 | 3536874587 | 370564473
                    ))
                    .map(|(parent, children)| (
                        *parent,
                        children
                            .iter()
                            .map(|child| (child.tala_id, child.position))
                            .collect::<Vec<_>>()
                    ))
                    .collect::<Vec<_>>()
            );
        }
        if crate::internal::trace_env_enabled("TALA_TRACE_ROOT_EXTERNAL_204") {
            eprintln!(
                "ROOT_EXTERNAL_204_RUST containers={:?} children={:?} aggregates={:?}",
                root.transaction_external_containers
                    .iter()
                    .filter(|node| matches!(
                        node.tala_id,
                        2043751575 | 3677471320 | 2884276447 | 1010209959 | 1231386872
                    ))
                    .map(|node| (node.tala_id, node.position, node.rect.size))
                    .collect::<Vec<_>>(),
                root.transaction_external_container_children
                    .iter()
                    .filter(|(parent, _)| matches!(
                        **parent,
                        2043751575 | 3677471320 | 2884276447 | 1010209959 | 1231386872
                    ))
                    .map(|(parent, children)| (
                        *parent,
                        children
                            .iter()
                            .map(|node| (node.tala_id, node.position, node.rect.size))
                            .collect::<Vec<_>>()
                    ))
                    .collect::<Vec<_>>(),
                root.transaction_external_aggregate_children
                    .iter()
                    .filter(|(parent, _)| matches!(
                        **parent,
                        2043751575 | 3677471320 | 2884276447 | 1010209959 | 1231386872
                    ))
                    .map(|(parent, children)| (
                        *parent,
                        children
                            .iter()
                            .map(|node| (node.tala_id, node.position, node.rect.size))
                            .collect::<Vec<_>>()
                    ))
                    .collect::<Vec<_>>()
            );
        }
        if crate::internal::trace_env_enabled("TALA_TRACE_ROOT_NODE_CHILDREN") {
            eprintln!(
                "ROOT_NODE_CHILDREN_RUST {:?}",
                root.nodes
                    .iter()
                    .filter(|node| matches!(
                        node.tala_id,
                        1_402_186_134
                            | 1_385_408_515
                            | 1_368_630_896
                            | 1_486_074_229
                            | 1_469_296_610
                            | 1_452_519_991
                            | 1_435_741_372
                            | 1_553_184_705
                            | 1_536_407_086
                            | 1_519_629_467
                            | 3_274_345_617
                            | 3_224_012_760
                            | 3_240_790_379
                            | 3_324_678_474
                            | 3_341_456_093
                            | 3_291_123_236
                            | 3_307_900_855
                            | 3_123_347_046
                            | 3_140_124_665
                            | 3_089_791_808
                    ))
                    .map(|node| (node.tala_id, node.position, node.rect.size, node.container))
                    .collect::<Vec<_>>()
            );
        }
        // External container refits carry shared Box dimensions back to the
        // owner at the root boundary. Keep positions in the temporary graph's
        // local frame; hierarchy copyback publishes those separately.
        for external in &root.transaction_external_containers {
            if let Some(owner) = self
                .graph
                .nodes
                .iter_mut()
                .find(|node| node.tala_id == external.tala_id)
            {
                owner.rect.size = external.rect.size;
            }
            if let Some(owner) = self
                .graph
                .transaction_external_containers
                .iter_mut()
                .find(|node| node.tala_id == external.tala_id)
            {
                owner.rect.size = external.rect.size;
            }
        }
        for (&parent_tala_id, placed_children) in &root.transaction_external_container_children {
            // A temporary child snapshot under an unpositioned recursively
            // fitted container is still in that scope's local frame. The
            // shared parent copyback below translates the owner's children;
            // copying this snapshot first would replace the owner's local
            // positions with stale packed coordinates.
            let parent_is_positioned = self
                .graph
                .nodes
                .iter()
                .find(|node| node.tala_id == parent_tala_id)
                .and_then(|node| node.position)
                .is_some();
            for placed_child in placed_children {
                if let Some(position) = placed_child.position
                    && parent_is_positioned
                    && let Some(owner_node) = self
                        .graph
                        .nodes
                        .iter()
                        .position(|node| node.tala_id == placed_child.tala_id)
                {
                    // A nested container's direct children already live in
                    // the owning arena's shared-pointer frame.  The root
                    // temporary graph holds cloned external snapshots for
                    // those children; its direct/packing moves are local to
                    // that snapshot and must not overwrite the recursively
                    // fitted positions before the carrier translation below.
                    if self.graph.nodes[owner_node].container.is_some() {
                        continue;
                    }
                    // Graph.Containers and Graph.Nodes share the same Go
                    // *Node pointers.  Root placement stores hidden child
                    // boxes in a Rust snapshot carrier, so publish the
                    // positioned snapshot into the owner's stable node before
                    // the subsequent alignment/refit stages read it.
                    let owner_node = &mut self.graph.nodes[owner_node];
                    owner_node.position = Some(position);
                    owner_node.rect.origin = placed_child.rect.origin;
                    owner_node.rect.size = placed_child.rect.size;
                }
                if !placed_child.is_container {
                    continue;
                }
                if let Some(owner_child) = self
                    .graph
                    .transaction_external_container_children
                    .values_mut()
                    .flatten()
                    .find(|child| child.tala_id == placed_child.tala_id)
                {
                    owner_child.rect.size = placed_child.rect.size;
                }
            }
        }
        if crate::internal::trace_env_enabled("TALA_TRACE_GRID_CARRIERS") {
            eprintln!(
                "GRID_ROOT_CARRIERS_RUST {:?}",
                root.transaction_external_containers
                    .iter()
                    .filter(|node| {
                        node.tala_id == 1_149_337_423 || node.tala_id == 1_782_109_120
                    })
                    .map(|node| (node.tala_id, node.position, node.rect.size))
                    .collect::<Vec<_>>()
            );
        }
        // Graph.placeNodes uses a temporary combined graph only for choosing
        // translations. Its returned node slice is not assigned back to the
        // master Graph.Nodes slice; preserve the master's lifecycle order for
        // SwapStuff and subsequent tie-sensitive stages.
        // The recovered Go placement graph shares Tree pointers with the
        // owning graph, so Graph.mirrorAxes updates every orientation carrier.
        // A retained tree node can have both an ordinary old_to_new projection
        // and a distinct auxiliary Tree projection; the ordinary entry wins
        // that map, while the auxiliary projection carries the mirrored
        // orientation. Publish orientation by durable TALA identity for every
        // owner carrier, regardless of its ordinary mapping.
        retained_nears.extend(Self::map_scope_nears(
            &placement.old_to_new,
            &placement.assigned_nears,
        ));
        // Graph.direct mirrors every eligible temporary carrier regardless of
        // whether the graph also contains cluster vessels. The cloned scope
        // already carries the carrier's directed position; replay the shared
        // descendant walk after projected offsets are published, then restore
        // the carrier to that directed target.
        let mut shared_mirror_replays = Vec::new();
        for old in self.graph.containers.get(&None).unwrap_or_default() {
            let Some(&local) = placement.old_to_new.get(&old) else {
                // See the nested copyback above: this root child is a
                // materialized edge-isolated container represented by the
                // external transaction view, not by Graph.Nodes.
                continue;
            };
            let trace_root_copyback = crate::internal::trace_env_value("TALA_TRACE_ROOT_COPYBACK")
                .is_some_and(|target| {
                    target == "all"
                        || target.parse::<i64>().ok()
                            == Some(self.graph.nodes[old.0 as usize].tala_id)
                });
            // The owner graph is back in its ordinary post-placement form at
            // this boundary. A container can have passed through a temporary
            // aggregate projection whose scoring marker was cleared; restore
            // the ordinary marker before the shared descendant walk so moving
            // the root carrier also moves its retained children.
            if self.graph.nodes[old.0 as usize].is_container {
                self.graph.nodes[old.0 as usize].scoring_is_container = true;
            }
            if trace_root_copyback {
                eprintln!(
                    "ROOT_COPYBACK_RUST before old={} stable_pos={:?} translation={:?} local_pos={:?}",
                    self.graph.nodes[old.0 as usize].tala_id,
                    self.graph.position(old),
                    self.graph.nodes[old.0 as usize].unpositioned_scope_translation,
                    root.position(local)
                );
                eprintln!(
                    "ROOT_COPYBACK_RUST_META old={} is_container={} scoring_is_container={} children={:?} active_desc={:?}",
                    self.graph.nodes[old.0 as usize].tala_id,
                    self.graph.nodes[old.0 as usize].is_container,
                    self.graph.nodes[old.0 as usize].scoring_is_container,
                    self.graph
                        .container_node_order(Some(old))
                        .iter()
                        .map(|child| self.graph.nodes[child.0 as usize].tala_id)
                        .collect::<Vec<_>>(),
                    self.graph
                        .active_descendants(old)
                        .iter()
                        .map(|child| self.graph.nodes[child.0 as usize].tala_id)
                        .collect::<Vec<_>>()
                );
            }
            if let Some(position) = root.position(local) {
                // Cluster members are retained as external aggregate children
                // in TALA's temporary root graph. Their positions are not the
                // vessel's projected position: the first member is centered
                // inside the vessel and later members follow the cluster
                // spacing. The Go graph shares those member pointers, so the
                // positions survive root copyback independently of the vessel
                // carrier. Preserve the projected aggregate coordinates when
                // publishing the stable arena instead of collapsing every
                // member onto the vessel top-left.
                let position = root
                    .transaction_external_aggregate_children
                    .values()
                    .flatten()
                    .find(|child| child.tala_id == self.graph.nodes[old.0 as usize].tala_id)
                    .and_then(|child| child.position)
                    .unwrap_or(position);
                // See the nested-scope copyback above: TALA's shared node was
                // initialized before CombineSubgraphs moved it, so descendants
                // participate in the same final translation.
                let was_unpositioned = self.graph.position(old).is_none();
                if was_unpositioned {
                    // A recursively fitted scope with no carrier position
                    // keeps its children in the scope-local frame. Go's
                    // positionContainerChildren applies the recorded inner
                    // placement when the shared carrier is first materialized
                    // in the root graph. Apply that translation to descendants
                    // before moving the carrier; moving the carrier itself
                    // would also move the enclosing aggregate vessel, which
                    // is a separate current Graph node in TALA.
                    let unpositioned_scope_translation = self.graph.nodes[old.0 as usize]
                        .unpositioned_scope_translation
                        .take();
                    if let Some(translation) = unpositioned_scope_translation {
                        if trace_root_copyback {
                            eprintln!(
                                "ROOT_COPYBACK_RUST_TRANSLATE old={} delta={:?} before={:?}",
                                self.graph.nodes[old.0 as usize].tala_id,
                                translation,
                                self.graph
                                    .container_node_order(Some(old))
                                    .iter()
                                    .map(|child| {
                                        (
                                            self.graph.nodes[child.0 as usize].tala_id,
                                            self.graph.nodes[child.0 as usize].is_container,
                                            self.graph.nodes[child.0 as usize].scoring_is_container,
                                            self.graph.position(*child),
                                        )
                                    })
                                    .collect::<Vec<_>>()
                            );
                            eprintln!(
                                "ROOT_COPYBACK_RUST_TRANSLATE_MAP old={} maps={:?} stable_desc={:?}",
                                self.graph.nodes[old.0 as usize].tala_id,
                                self.graph
                                    .transaction_external_container_children
                                    .iter()
                                    .filter(|(parent, _)| {
                                        self.graph.nodes[old.0 as usize].tala_id == **parent
                                            || self
                                                .graph
                                                .container_node_order(Some(old))
                                                .iter()
                                                .any(|child| {
                                                    self.graph.nodes[child.0 as usize].tala_id
                                                        == **parent
                                                })
                                    })
                                    .map(|(parent, children)| {
                                        (
                                            *parent,
                                            children
                                                .iter()
                                                .map(|child| (child.tala_id, child.position))
                                                .collect::<Vec<_>>(),
                                        )
                                    })
                                    .collect::<Vec<_>>(),
                                self.graph
                                    .active_descendants(old)
                                    .iter()
                                    .map(|child| {
                                        (
                                            self.graph.nodes[child.0 as usize].tala_id,
                                            self.graph.position(*child),
                                        )
                                    })
                                    .collect::<Vec<_>>()
                            );
                        }
                        // positionContainerChildren walks the current direct
                        // child pointers. Each temporary aggregate vessel then
                        // carries all hidden members through its recovered
                        // moveNodeWithChildren traversal.
                        for child in self.graph.container_node_order(Some(old)) {
                            // A recursively fitted ordinary container can
                            // re-enter the root graph with its dynamic
                            // scoring marker cleared by the induced scope.
                            // The shared Go pointer still retains
                            // Node.isContainer, so restore that traversal bit
                            // before moving the carrier and its descendants.
                            if self.graph.nodes[child.0 as usize].is_container {
                                self.graph.nodes[child.0 as usize].scoring_is_container = true;
                            }
                            self.graph
                                .translate_active_node_with_children(child, translation);
                        }
                        if trace_root_copyback {
                            eprintln!(
                                "ROOT_COPYBACK_RUST_TRANSLATE old={} after={:?}",
                                self.graph.nodes[old.0 as usize].tala_id,
                                self.graph
                                    .container_node_order(Some(old))
                                    .iter()
                                    .map(|child| {
                                        (
                                            self.graph.nodes[child.0 as usize].tala_id,
                                            self.graph.position(*child),
                                        )
                                    })
                                    .collect::<Vec<_>>()
                            );
                            eprintln!(
                                "ROOT_COPYBACK_RUST_TRANSLATE_MAP old={} maps_after={:?} stable_desc_after={:?}",
                                self.graph.nodes[old.0 as usize].tala_id,
                                self.graph
                                    .transaction_external_container_children
                                    .iter()
                                    .filter(|(parent, _)| {
                                        self.graph.nodes[old.0 as usize].tala_id == **parent
                                            || self
                                                .graph
                                                .container_node_order(Some(old))
                                                .iter()
                                                .any(|child| {
                                                    self.graph.nodes[child.0 as usize].tala_id
                                                        == **parent
                                                })
                                    })
                                    .map(|(parent, children)| {
                                        (
                                            *parent,
                                            children
                                                .iter()
                                                .map(|child| (child.tala_id, child.position))
                                                .collect::<Vec<_>>(),
                                        )
                                    })
                                    .collect::<Vec<_>>(),
                                self.graph
                                    .active_descendants(old)
                                    .iter()
                                    .map(|child| {
                                        (
                                            self.graph.nodes[child.0 as usize].tala_id,
                                            self.graph.position(*child),
                                        )
                                    })
                                    .collect::<Vec<_>>()
                            );
                        }
                    }
                    self.graph.set_position(old, Point::default());
                }
                // Graph.direct has already mirrored the temporary carrier.
                // In recovered Go, positionContainerChildren then translates
                // its shared descendant subtree into the carrier's new frame;
                // it does not mirror those descendants recursively. Moving
                // the stable carrier with children is the equivalent copyback.
                // The root temporary graph has two distinct Go pointers at
                // this boundary: the stable member container being copied
                // back, and the temporary cluster vessel that remains in
                // Graph.Nodes.  The flattened arena aliases both onto the
                // first member, so the ordinary shared-pointer translation
                // would incorrectly move the pending vessel by the member's
                // local-frame delta. Preserve the vessel publication while
                // still translating the member and its retained descendants.
                // The root temporary graph has two distinct Go pointers at
                // this boundary: the stable member container being copied
                // back, and the temporary cluster vessel that remains in
                // Graph.Nodes. The flattened arena aliases both onto the
                // first member, so the ordinary shared-pointer translation
                // would incorrectly move the pending vessel by the member's
                // local-frame delta. Preserve the vessel publication while
                // still translating the member and its retained descendants.
                let root_cluster_vessel = self.graph.nodes[old.0 as usize]
                    .cluster
                    .filter(|&cluster_index| {
                        self.graph.clusters[cluster_index].members.first() == Some(&old)
                    })
                    .and_then(|cluster_index| {
                        self.graph
                            .pending_cluster_vessel_positions
                            .get(&cluster_index)
                            .copied()
                            .map(|position| (cluster_index, position))
                    });
                self.graph.move_node_abs_with_children(old, position);
                if let Some((cluster_index, vessel_position)) = root_cluster_vessel {
                    self.graph
                        .pending_cluster_vessel_positions
                        .insert(cluster_index, vessel_position);
                }
                // A recursively fitted container starts without a master
                // position. Its children were copied through the root
                // scope's already-directed frame above; replaying the
                // recorded mirror would reflect those shared descendants a
                // second time (the cluster-vessel alias then drifts by the
                // member-centering delta). Root-positioned containers still
                // need the explicit replay because their child pointers are
                // copied in the owner's frame.
                if !was_unpositioned
                    && crate::internal::trace_env_enabled("TALA_TRACE_REPLAY_ROOT_MIRRORS")
                    && let Some(mirror) = placed_root.descendant_mirrors.get(&local)
                {
                    shared_mirror_replays.push((old, position, mirror.clone()));
                }
                if trace_root_copyback {
                    eprintln!(
                        "ROOT_COPYBACK_RUST after old={} stable_pos={:?} translation={:?}",
                        self.graph.nodes[old.0 as usize].tala_id,
                        self.graph.position(old),
                        self.graph.nodes[old.0 as usize].unpositioned_scope_translation
                    );
                }
            }
        }
        if crate::internal::trace_env_enabled("TALA_TRACE_ROOT_CLUSTER6") {
            eprintln!(
                "ROOT_CLUSTER6_RUST phase=after-copyback stable={:?} aggregate={:?} pending={:?}",
                self.graph
                    .nodes
                    .iter()
                    .filter(|node| node.tala_id == 6)
                    .map(|node| (
                        node.position,
                        node.rect.origin,
                        node.rect.size,
                        node.container
                    ))
                    .collect::<Vec<_>>(),
                self.graph
                    .transaction_external_aggregate_children
                    .iter()
                    .filter(|(_, children)| children.iter().any(|child| child.tala_id == 6))
                    .map(|(parent, children)| (
                        *parent,
                        children
                            .iter()
                            .filter(|child| child.tala_id == 6)
                            .map(|child| (child.position, child.rect.origin, child.rect.size))
                            .collect::<Vec<_>>()
                    ))
                    .collect::<Vec<_>>(),
                self.graph.pending_cluster_vessel_positions,
            );
        }
        if crate::internal::trace_env_enabled("TALA_TRACE_ROOT_OWNER_AFTER_COPYBACK") {
            eprintln!(
                "ROOT_OWNER_AFTER_COPYBACK_RUST {:?}",
                self.graph
                    .nodes
                    .iter()
                    .filter(|node| {
                        crate::internal::trace_env_value("TALA_TRACE_ROOT_OWNER_AFTER_COPYBACK")
                            .as_deref()
                            == Some("all")
                            || matches!(node.tala_id, 233611931 | 1149337423 | 1782109120)
                    })
                    .map(|node| (node.tala_id, node.position, node.rect.size, node.container))
                    .collect::<Vec<_>>()
            );
        }
        if crate::internal::trace_env_enabled("TALA_TRACE_ROOT_STEPS") {
            eprintln!("ROOT_STEP_RUST after_copy_children");
        }
        trace_root_geometry!("after_copy_children");
        // The root placement's temporary graphs share every positioned
        // container pointer with the owner in TALA.  Preserve fitted child
        // boxes and positions when publishing the cloned Rust scope.
        for placed in root
            .nodes
            .iter()
            .filter(|node| node.is_container && node.container.is_some())
        {
            let Some(owner) = self
                .graph
                .nodes
                .iter()
                .position(|node| node.tala_id == placed.tala_id)
            else {
                continue;
            };
            self.graph.nodes[owner].position = placed.position;
            self.graph.nodes[owner].rect.origin = placed.rect.origin;
            self.graph.nodes[owner].rect.size = placed.rect.size;
        }
        // CombineSubgraphs in the Go implementation copies the combined
        // temporary graph's projected positions back to the owner graph before
        // publishing external offsets.  Cluster vessel members are already in
        // their mirrored order at this point; arranging the vessel again would
        // restore declaration order and change the NodePlacement snapshot.
        for children in root.transaction_external_aggregate_children.values() {
            for child in children {
                self.publish_combined_root_projection(child);
            }
        }
        for children in root.transaction_external_container_children.values() {
            for child in children {
                self.publish_combined_root_projection(child);
            }
        }
        trace_root_geometry!("after_root_projection");
        // Replay the external shared-child offsets at the root boundary too:
        // nested temporary cluster vessels are shared pointers in Go, so their
        // root-relative placement must be published along with ordinary nodes.
        self.publish_external_shared_child_offsets_mode(
            &external_shared_child_offsets,
            true,
            Some(&root_cluster_member_positions),
        );
        trace_root_geometry!("after_publish_external");
        if crate::internal::trace_env_enabled("TALA_TRACE_ROOT_STEPS") {
            eprintln!("ROOT_STEP_RUST after_publish_external");
        }
        for (old, target, mirror) in shared_mirror_replays {
            let direct_cluster_vessel = self.graph.nodes[old.0 as usize]
                .cluster
                .filter(|&cluster_index| {
                    self.graph.clusters[cluster_index].members.first() == Some(&old)
                })
                .and_then(|cluster_index| {
                    self.graph
                        .pending_cluster_vessel_positions
                        .get(&cluster_index)
                        .copied()
                        .map(|position| (cluster_index, position))
                });
            self.graph.mirror_subtree_axes(
                old,
                mirror.mirror_x,
                mirror.mirror_y,
                &mirror.reachable_containers,
            );
            self.graph.move_node_abs_with_children(old, target);
            if let Some((cluster_index, vessel_position)) = direct_cluster_vessel {
                self.graph
                    .pending_cluster_vessel_positions
                    .insert(cluster_index, vessel_position);
            }
        }
        // The root component's combined snapshot retains the shared child
        // order produced by the nested scope. A subsequent root mirror walk
        // visits the container carrier, but Go's shared pointers leave a
        // branching child scope in that already-combined order. Restore only
        // those multi-child snapshots; single-child scopes are refitted by
        // the replayed container translation itself.
        if self
            .graph
            .nodes
            .iter()
            .any(|node| node.fixed_top_left.is_some())
        {
            for placed_children in root.transaction_external_container_children.values() {
                if placed_children.len() <= 1 {
                    continue;
                }
                let child_ids = placed_children
                    .iter()
                    .filter_map(|placed_child| {
                        self.graph
                            .nodes
                            .iter()
                            .position(|node| node.tala_id == placed_child.tala_id)
                            .map(|index| NodeId(index as u32))
                    })
                    .collect::<BTreeSet<_>>();
                let mut sibling_edge_counts = BTreeMap::<(NodeId, NodeId), usize>::new();
                for edge in self.graph.member_edges() {
                    if !child_ids.contains(&edge.from) || !child_ids.contains(&edge.to) {
                        continue;
                    }
                    *sibling_edge_counts.entry((edge.from, edge.to)).or_default() += 1;
                }
                let child_y_span = placed_children
                    .iter()
                    .filter_map(|child| child.position.map(|position| position.y))
                    .fold(None, |span: Option<(f64, f64)>, y| {
                        Some(span.map_or((y, y), |(minimum, maximum)| {
                            (minimum.min(y), maximum.max(y))
                        }))
                    })
                    .map_or(0.0, |(minimum, maximum)| maximum - minimum);
                if !sibling_edge_counts.values().any(|count| *count > 1) || child_y_span <= 10.0 {
                    continue;
                }
                for placed_child in placed_children {
                    let Some(position) = placed_child.position else {
                        continue;
                    };
                    let Some(owner_index) = self
                        .graph
                        .nodes
                        .iter()
                        .position(|node| node.tala_id == placed_child.tala_id)
                    else {
                        continue;
                    };
                    self.graph.nodes[owner_index].position = Some(position);
                    self.graph.nodes[owner_index].rect.origin = placed_child.rect.origin;
                    self.graph.nodes[owner_index].rect.size = placed_child.rect.size;
                }
            }
        }
        // Root-owned cluster vessels publish through `cluster_vessel_positions`
        // above.  Any cluster vessel in this external-child map is nested
        // under a materialized container and has already moved with that
        // parent during root copy-back.  Replaying a projected mirrored
        // offset here can replace its shared position with the vessel's
        // pre-fit origin.
        let root_mirrored_external_child_offsets = mirrored_external_child_offsets
            .iter()
            .copied()
            .filter(|(_, child_tala_id, _)| {
                self.graph
                    .clusters
                    .iter()
                    .all(|cluster| cluster.vessel_tala_id != *child_tala_id)
            })
            .collect::<Vec<_>>();
        self.publish_mirrored_external_child_offsets(&root_mirrored_external_child_offsets);
        trace_root_geometry!("after_publish_mirrored");
        if crate::internal::trace_env_enabled("TALA_TRACE_ROOT_STEPS") {
            eprintln!("ROOT_STEP_RUST after_publish_mirrored");
        }
        // Go's root `PlaceNodes` bypasses the owner-graph copyback when
        // preprocessing leaves exactly one visible node. In that case
        // `PlaceTrees` reconnects directly into the owner graph, retaining
        // sorted placement-tree preorder.
        let reconnects_in_owner = self.graph.node_order.len() == 1;
        for (&old, &local) in &placement.old_to_new {
            if !self.graph.tree_routing_nodes.contains_key(&old)
                || !placement.tree_auxiliary_nodes.contains(&local)
            {
                continue;
            }
            let Some(position) = root.position(local) else {
                continue;
            };
            if self.graph.position(old).is_none() {
                self.graph.set_position(old, Point::default());
            }
            if !self.graph.node_order.contains(&old) {
                self.graph.node_order.push(old);
            }
            self.graph.move_node_abs_with_children(old, position);
            if let Some(projected) = root.tree_routing_nodes.get(&local)
                && let Some(state) = self.graph.tree_routing_nodes.get_mut(&old)
            {
                if crate::internal::trace_env_enabled("TALA_TRACE_TREE_ORIENT")
                    && matches!(
                        self.graph.nodes[old.0 as usize].tala_id,
                        1171112347 | 1154334728 | 1271778061 | 1187889966
                    )
                {
                    eprintln!(
                        "TREE_ORIENT_ROOT_COPY_RUST tala={} from={:?} to={:?}",
                        self.graph.nodes[old.0 as usize].tala_id,
                        state.orientation,
                        projected.orientation
                    );
                }
                state.orientation = projected.orientation;
                self.graph
                    .publish_tree_orientation(old, projected.orientation);
            }
        }
        if crate::internal::trace_env_enabled("TALA_TRACE_ROOT_STEPS") {
            eprintln!("ROOT_STEP_RUST after_tree_copyback");
        }
        // The projected root graph already carries Graph.direct's orientation
        // mutation. Copying its state above is the single mirror publication;
        // applying a second descendant flip here would undo it for reflected
        // root trees.
        #[cfg(feature = "diagnostic-traces")]
        root.trace_tree_copyback("placed-projection", None);
        self.graph.publish_placed_tree_edges(
            &self.graph.containers.get(&None).unwrap_or_default(),
            reconnects_in_owner,
        );
        #[cfg(feature = "diagnostic-traces")]
        self.graph.trace_tree_copyback("published", None);
        if crate::internal::trace_env_enabled("TALA_TRACE_ROOT_STEPS") {
            eprintln!("ROOT_STEP_RUST after_publish_tree_edges");
        }
        // Graph.placeNodes initializes and optimizes the temporary vessel,
        // then `syncNested` expands it back into its ordered cluster members.
        // TALA keeps cluster vessels and their hidden members in the graph
        // through Graph.placeNodes.  Graph.syncClusters (and syncSequences)
        // run later in GapNormalizationStage; expanding them here changes
        // which carrier participates in the following pipeline stages.
        self.graph.refresh_placement_components();
        trace_root_geometry!("after_refresh_components");
        if crate::internal::trace_env_enabled("TALA_TRACE_ROOT_STEPS") {
            eprintln!("ROOT_STEP_RUST after_refresh_components");
        }
        // TALA's temporary children graphs share Node pointers with the main
        // graph, but Near mutations are consumed by the main graph only after
        // recursive placement has completed. CommonUncleSiblings is a
        // placement-only score carrier and was cleared before this boundary.
        self.retain_scope_nears(&retained_nears);
        if crate::internal::trace_env_enabled("TALA_TRACE_ROOT_STEPS") {
            eprintln!("ROOT_STEP_RUST after_retain_nears");
        }
        if let Some(owner) = &prepared_owner {
            self.restore_prepared_owner_scope(owner)?;
            frames.retain(|(key, _)| key.is_some());
        } else {
            self.graph
                .restored_hierarchy_endpoint_order
                .extend(placement.restored_endpoint_order.iter().copied());
        }
        self.graph.sync_edge_adjacency_order();
        if crate::internal::trace_env_enabled("TALA_TRACE_ROOT_STEPS") {
            eprintln!("ROOT_STEP_RUST after_sync_edges");
        }
        self.graph.compute_cell_size();
        if crate::internal::trace_env_enabled("TALA_TRACE_ROOT_STEPS") {
            eprintln!("ROOT_STEP_RUST after_cell");
        }
        self.graph.refresh_placement_components();
        trace_root_geometry!("after_final_refresh");
        if crate::internal::trace_env_enabled("TALA_TRACE_ROOT_STEPS") {
            eprintln!("ROOT_STEP_RUST after_final_refresh");
        }
        // The fit pass above uses the initial temporary vessel dimensions so
        // an owner does not count its hidden members twice. After the complete
        // recursive scope is materialized, publish the expanded dimensions for
        // SwapStuff and the remaining pipeline stages.
        self.graph.refresh_cluster_vessel_sizes();
        if crate::internal::trace_env_enabled("TALA_TRACE_ROOT_AFTER_PLACE") {
            eprint!("RECURSIVE_GLOBAL_RUST root=0");
            for node in &self.graph.nodes {
                if let Some(position) = node.position {
                    eprint!(" {}={},{}", node.tala_id, position.x, position.y);
                }
            }
            eprintln!();
        }
        Ok(true)
    }
}
