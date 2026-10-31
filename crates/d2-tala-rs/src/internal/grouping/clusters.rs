// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Cluster recognition, carrier state, arrangement, and restoration.
//!
//! Eligible equivalent nodes move through a temporary row/column vessel so
//! outer scopes can place the cluster as one object.

use super::*;
use std::cmp::Ordering;

mod cluster_transactions;
mod gap_reduction;
#[cfg(test)]
mod gap_reduction_tests;

#[derive(Clone, Debug, Eq, PartialEq)]
struct EdgeSignature {
    bidirectional: usize,
    undirected: usize,
    directed: usize,
    from: usize,
    to: usize,
    node_side_arrowheads: BTreeSet<Option<String>>,
    other_side_arrowheads: BTreeSet<Option<String>>,
}

impl EdgeSignature {
    fn arrow_type_count(&self) -> usize {
        usize::from(self.directed > 0)
            + usize::from(self.bidirectional > 0)
            + usize::from(self.undirected > 0)
    }

    fn matches(&self, other: &Self) -> bool {
        self.from == other.from
            && self.to == other.to
            && self.bidirectional == other.bidirectional
            && self.undirected == other.undirected
            && self.arrow_type_count() <= 1
            && other.arrow_type_count() <= 1
            && self.node_side_arrowheads == other.node_side_arrowheads
            && self.other_side_arrowheads == other.other_side_arrowheads
    }
}

const CLUSTER_PRECISION: f64 = 0.0001;

/// Go's math.Round rounds half away from zero. Rust's f64::round follows the
/// same rule today, but keeping the operation explicit documents the
/// cross-language boundary and avoids relying on target libm behavior while
/// reproducing pointer-move deltas.
#[inline]
fn go_round(value: f64) -> f64 {
    if value < 0.0 {
        (value - 0.5).ceil()
    } else {
        (value + 0.5).floor()
    }
}

fn cluster_precision_compare(left: f64, right: f64) -> Ordering {
    if (left - right).abs() < CLUSTER_PRECISION {
        Ordering::Equal
    } else {
        left.total_cmp(&right)
    }
}

impl ArenaGraph {
    #[cfg(feature = "diagnostic-traces")]
    fn trace_cluster_policy(&self, phase: &str, index: usize, only_flip: bool) {
        if !crate::internal::trace_env_enabled("D2RS_TRACE_CLUSTER_POLICY") {
            return;
        }
        let cluster = &self.clusters[index];
        let policy = self
            .owner_group_binding(TopologyVessel::Cluster(index))
            .map(|group| {
                self.owner_cluster_policy(group)
                    .expect("bound cluster policy")
            });
        let arrangement = |value| match value {
            ClusterArrangement::Row => "Row",
            ClusterArrangement::Column => "Column",
        };
        eprintln!(
            "{}",
            serde_json::json!({"event":"cluster-policy", "phase":phase,
            "id":cluster.vessel_tala_id.to_string(), "onlyFlip":only_flip,
            "arrangement":policy.as_ref().map(|p| &p.arrangement),
            "desired":policy.as_ref().map(|p| &p.desired_arrangement),
            "workingArrangement":arrangement(cluster.arrangement),
            "workingDesired":arrangement(cluster.desired_arrangement)})
        );
    }

    /// Match the Go engine's `ResetClusters` at the start of a fresh layout.
    /// GraphJSON snapshots retain cluster maps so they can be round-tripped,
    /// but `Layout` first detaches those prior vessels and rediscovers them
    /// during `PreprocessClusters`. Before detaching, Go reconnects each
    /// recorded edge to its non-nil original endpoint in cluster-ID and
    /// abduction-slice order. Keep every serialized member visible through
    /// Prescale and the preceding preprocessing stages.
    pub(super) fn reset_serialized_clusters_for_layout(&mut self) {
        if self.clusters.is_empty() {
            return;
        }
        let reset_owner_records = self.reset_owner_cluster_records_for_layout();
        let reconnections = self
            .serialized_cluster_edge_abductions
            .values()
            .flatten()
            .filter_map(|abduction| {
                let edge = abduction.edge?;
                Some((
                    edge,
                    abduction
                        .originally_from
                        .and_then(AggregateNodeRef::graph_node),
                    abduction
                        .originally_to
                        .and_then(AggregateNodeRef::graph_node),
                ))
            })
            .collect::<Vec<_>>();
        // ResetClusters detaches each serialized vessel before the fresh
        // PreprocessClusters pass allocates its replacement. The replacement
        // may receive the same deterministic ID, so retaining the decoded
        // vessel box would override the new cluster's live geometry in later
        // placement scopes.
        for vessel_tala_id in self.clusters.iter().map(|cluster| cluster.vessel_tala_id) {
            self.serialized_vessel_positions_by_tala
                .remove(&vessel_tala_id);
            self.serialized_vessel_sizes_by_tala.remove(&vessel_tala_id);
            self.serialized_cluster_arrangements_by_tala
                .remove(&vessel_tala_id);
        }
        for (edge, originally_from, originally_to) in
            reconnections.into_iter().filter(|_| !reset_owner_records)
        {
            if let Some(original) = originally_from {
                self.reconnect_edge_source_for_cluster_reset(edge, original);
            }
            if let Some(original) = originally_to {
                self.reconnect_edge_target_for_cluster_reset(edge, original);
            }
        }
        // Placement scopes need the original endpoints after ResetClusters,
        // before sequence/cluster preprocessing rewrites active carriers.
        // Refresh that pointer-visible source topology alongside the live
        // arena so a later scope cannot resurrect a serialized pre-reset
        // endpoint.
        for edge_index in self.member_edge_indices() {
            let edge = &self.edge_records[edge_index];
            let input_edge_index = edge.input_id.0 as usize;
            let from = &self.nodes[edge.from.0 as usize];
            let to = &self.nodes[edge.to.0 as usize];
            let endpoints = (from.tala_id, to.tala_id);
            if let Some(source_endpoints) = self.source_edge_endpoints.get_mut(input_edge_index) {
                *source_endpoints = endpoints;
            }
            self.source_edge_endpoints_by_id
                .insert(edge.tala_id, endpoints);
        }
        for cluster in &self.clusters {
            for member in &cluster.members {
                self.nodes[member.0 as usize].cluster = None;
            }
        }
        self.clusters.clear();
        self.cluster_vessel_sizes.clear();
        self.serialized_cluster_edge_abductions.clear();
        self.pending_cluster_vessel_positions.clear();
        self.cluster_backlinks_complete = false;
        if !reset_owner_records {
            self.node_order = (0..self.nodes.len())
                .map(|index| NodeId(index as u32))
                .collect();
        }
        self.invalidate_node_order_membership();
    }

    fn reconnect_edge_source_for_cluster_reset(&mut self, edge_id: EdgeId, node: NodeId) {
        let old_source = self.edge_records[edge_id.0 as usize].from;
        if let Some(position) = self.nodes[old_source.0 as usize]
            .edges
            .read()
            .iter()
            .position(|candidate| *candidate == edge_id)
        {
            self.nodes[old_source.0 as usize].edges.remove(position);
        }
        self.nodes[node.0 as usize].edges.push(edge_id);
        self.edge_records[edge_id.0 as usize].from = node;
    }

    fn reconnect_edge_target_for_cluster_reset(&mut self, edge_id: EdgeId, node: NodeId) {
        let old_target = self.edge_records[edge_id.0 as usize].to;
        if let Some(position) = self.nodes[old_target.0 as usize]
            .edges
            .read()
            .iter()
            .position(|candidate| *candidate == edge_id)
        {
            self.nodes[old_target.0 as usize].edges.remove(position);
        }
        self.nodes[node.0 as usize].edges.push(edge_id);
        self.edge_records[edge_id.0 as usize].to = node;
    }

    /// Loads already-materialized cluster ownership from a diagnostic
    /// graphjson snapshot. Serialized vessels are pointer-only Go records, so
    /// the stable arena uses the first member as its carrier and keeps the
    /// vessel's independent origin in `pending_cluster_vessel_positions`.
    pub(super) fn load_serialized_clusters(
        &mut self,
        serialized: &[SerializedClusterState],
    ) -> bool {
        if serialized.is_empty() {
            return false;
        }
        let by_tala_id = self
            .nodes
            .iter()
            .map(|node| (node.tala_id, node.input_id))
            .collect::<BTreeMap<_, _>>();
        let arrangement = |value: &str| {
            if value.eq_ignore_ascii_case("Column") {
                ClusterArrangement::Column
            } else {
                ClusterArrangement::Row
            }
        };
        let mut loaded = Vec::new();
        for cluster in serialized {
            let Some(members) = cluster
                .member_tala_ids
                .iter()
                .map(|id| by_tala_id.get(id).copied())
                .collect::<Option<Vec<_>>>()
            else {
                continue;
            };
            self.serialized_cluster_arrangements_by_tala
                .insert(cluster.vessel_tala_id, cluster.arrangement.clone());
            let cluster_index = loaded.len();
            let current = arrangement(&cluster.arrangement);
            let desired = if cluster.desired_arrangement.is_empty() {
                current
            } else {
                arrangement(&cluster.desired_arrangement)
            };
            for member in &members {
                self.nodes[member.0 as usize].cluster = Some(cluster_index);
            }
            if let Some(position) = cluster.vessel_position {
                self.pending_cluster_vessel_positions
                    .insert(cluster_index, position);
                self.serialized_vessel_positions_by_tala
                    .insert(cluster.vessel_tala_id, position);
            }
            if cluster.vessel_size != Size::default() {
                self.serialized_vessel_sizes_by_tala
                    .insert(cluster.vessel_tala_id, cluster.vessel_size);
            }
            self.serialized_cluster_containers_by_tala.insert(
                cluster.vessel_tala_id,
                (cluster.container_tala_id != 0).then_some(cluster.container_tala_id),
            );
            loaded.push(ClusterState {
                members,
                arrangement: current,
                desired_arrangement: desired,
                padding: cluster.padding,
                vessel_tala_id: cluster.vessel_tala_id,
                fixed_size: cluster.fixed_size,
            });
        }
        if loaded.is_empty() {
            return false;
        }
        // ClusterState does not currently carry its container separately; the
        // member nodes retain the serialized parent relationship and the
        // active projection resolves it from the carrier.
        self.clusters = loaded;
        self.cluster_vessel_sizes = self
            .clusters
            .iter()
            .enumerate()
            .map(|(index, cluster)| {
                self.serialized_vessel_sizes_by_tala
                    .get(&cluster.vessel_tala_id)
                    .copied()
                    .filter(|size| *size != Size::default())
                    .unwrap_or_else(|| {
                        self.cluster_vessel_size_for_arrangement(index, cluster.arrangement)
                    })
            })
            .collect();
        self.cluster_backlinks_complete = true;
        // The arena applies codec.go's final active-member filter after both
        // aggregate maps have been loaded. Concrete vessels keep their own
        // node identity; no member is appended as a synthetic carrier.
        true
    }

    /// Resolve graphjson's signed pointer references once, after every
    /// aggregate vessel has been interned. Go's decoder does this before it
    /// publishes the graph; keeping the resolved records on their owning
    /// vessel avoids later guessing by member-set equality.
    pub(super) fn resolve_serialized_aggregate_edge_abductions(
        &mut self,
        clusters: &[SerializedClusterState],
        sequences: &[SerializedSequenceState],
        decoded_edges: &BTreeMap<i64, EdgeId>,
    ) {
        let node_by_tala_id = self
            .nodes
            .iter()
            .enumerate()
            .map(|(index, node)| (node.tala_id, NodeId(index as u32)))
            .collect::<BTreeMap<_, _>>();
        let edge_by_tala_id = self
            .edge_records
            .iter()
            .enumerate()
            .map(|(index, edge)| (edge.tala_id, EdgeId(index as u32)))
            .collect::<BTreeMap<_, _>>();
        let vessel_ids = clusters
            .iter()
            .map(|cluster| cluster.vessel_tala_id)
            .chain(sequences.iter().map(|sequence| sequence.vessel_tala_id))
            .collect::<BTreeSet<_>>();
        let resolve_endpoint = |tala_id: i64| {
            if tala_id == 0 {
                None
            } else if let Some(&node) = node_by_tala_id.get(&tala_id) {
                Some(AggregateNodeRef::GraphNode { node, tala_id })
            } else if vessel_ids.contains(&tala_id) {
                Some(AggregateNodeRef::Vessel { tala_id })
            } else {
                Some(AggregateNodeRef::Unknown { tala_id })
            }
        };
        let resolve = |abductions: &[SerializedAggregateEdgeAbduction]| {
            abductions
                .iter()
                .map(|abduction| ResolvedAggregateEdgeAbduction {
                    edge_tala_id: abduction.edge_id,
                    edge: decoded_edges
                        .get(&abduction.edge_id)
                        .copied()
                        .or_else(|| edge_by_tala_id.get(&abduction.edge_id).copied()),
                    originally_from: resolve_endpoint(abduction.originally_from_node),
                    originally_to: resolve_endpoint(abduction.originally_to_node),
                    current_from: resolve_endpoint(abduction.current_from_node),
                    current_to: resolve_endpoint(abduction.current_to_node),
                })
                .collect::<Vec<_>>()
        };
        self.serialized_cluster_edge_abductions = clusters
            .iter()
            .map(|cluster| (cluster.vessel_tala_id, resolve(&cluster.edge_abductions)))
            .collect();
        self.serialized_sequence_edge_abductions = sequences
            .iter()
            .map(|sequence| (sequence.vessel_tala_id, resolve(&sequence.edge_abductions)))
            .collect();
    }

    pub(super) fn cluster_bounds(&self, members: &[NodeId]) -> Option<Rect> {
        let mut left = f64::INFINITY;
        let mut top = f64::INFINITY;
        let mut right = f64::NEG_INFINITY;
        let mut bottom = f64::NEG_INFINITY;
        for member in members {
            let node = &self.nodes[member.0 as usize];
            let position = node.position?;
            left = left.min(position.x);
            top = top.min(position.y);
            right = right.max(position.x + node.rect.size.width);
            bottom = bottom.max(position.y + node.rect.size.height);
        }
        left.is_finite().then_some(Rect {
            origin: Point { x: left, y: top },
            size: Size {
                width: right - left,
                height: bottom - top,
            },
        })
    }

    /// Reconstructs the node occupying an edge endpoint immediately before
    /// `Cluster.AbductEdges` creates `cluster_index`.
    ///
    /// Sequences abduct first. Clusters are then created and abduct in slice
    /// order, so only an earlier cluster can already own the endpoint.
    fn endpoint_before_cluster_abduction(&self, endpoint: NodeId, cluster_index: usize) -> NodeId {
        let materialized_sequence_owner = |node: NodeId| {
            self.sequences
                .iter()
                .find(|sequence| sequence.members.contains(&node))
                .map(|sequence| {
                    let vessel_tala_id = sequence.vessel_tala_id;
                    self.nodes
                        .iter()
                        .find(|candidate| {
                            candidate.tala_id == vessel_tala_id && candidate.position.is_some()
                        })
                        .map(|candidate| candidate.input_id)
                        .unwrap_or(sequence.members[0])
                })
                .unwrap_or(node)
        };
        let mut current = materialized_sequence_owner(endpoint);
        for cluster in self.clusters.iter().take(cluster_index) {
            if cluster
                .members
                .iter()
                .copied()
                .any(|member| member == current || materialized_sequence_owner(member) == current)
            {
                current = self
                    .nodes
                    .iter()
                    .find(|candidate| {
                        candidate.tala_id == cluster.vessel_tala_id && candidate.position.is_some()
                    })
                    .map(|candidate| candidate.input_id)
                    .unwrap_or(cluster.members[0]);
                break;
            }
        }
        current
    }

    pub(crate) fn serialized_cluster_edge_abductions(
        &self,
        vessel_tala_id: i64,
    ) -> Option<&[ResolvedAggregateEdgeAbduction]> {
        self.serialized_cluster_edge_abductions
            .get(&vessel_tala_id)
            .map(Vec::as_slice)
    }

    fn cluster_external_nodes(&self, cluster_index: usize) -> Vec<NodeId> {
        let cluster = &self.clusters[cluster_index];
        if let Some(abductions) = self.serialized_cluster_edge_abductions(cluster.vessel_tala_id) {
            // graphjson restores these records as pointer-owned
            // Cluster.EdgeAbductions. Go's clusterExternalConnectedNodes
            // selects the current endpoint opposite the single nil original
            // endpoint, retaining first-seen abduction order. A vessel gets a
            // pointer identity that is stable across the decoded aggregate;
            // use the vessel key directly instead of inferring ownership from
            // an equal member set.
            let mut added = BTreeSet::new();
            let mut external = Vec::new();
            for abduction in abductions {
                let endpoint = match (
                    abduction.originally_from.is_none(),
                    abduction.originally_to.is_none(),
                ) {
                    (true, false) => abduction.current_from,
                    (false, true) => abduction.current_to,
                    _ => continue,
                };
                let Some(endpoint) = endpoint else {
                    continue;
                };
                let Some(node) = endpoint.graph_node().or_else(|| {
                    self.nodes
                        .iter()
                        .position(|node| node.tala_id == endpoint.tala_id())
                        .map(|index| NodeId(index as u32))
                }) else {
                    // Its distinct vessel identity remains in the resolved
                    // record. This NodeId-only helper can project only nodes
                    // that participate in the stable arena.
                    continue;
                };
                if self.active_aggregate_owner(node) == node
                    && self.position(node).is_some()
                    && added.insert(node)
                {
                    external.push(node);
                }
            }
            return external;
        }

        let member_set = cluster.members.iter().copied().collect::<BTreeSet<_>>();
        let vessel = self.topology_vessel_node(TopologyVessel::Cluster(cluster_index));
        let mut added = BTreeSet::new();
        let mut external = Vec::new();
        // Cluster.AbductEdges walks Graph.Edges in order. Its From and To
        // tests are independent, so an internal edge records both the member
        // seen by the first branch and this cluster's vessel seen by the
        // second. Each record retains the opposite *current* endpoint.
        for edge_index in self.member_edge_indices() {
            let edge = &self.edge_records[edge_index];
            // AddCluster rewrites current endpoints to the distinct Vessel.
            // Retain the same first-seen external inventory after reconnecting
            // those edges, as Cluster.EdgeAbductions does in Source.
            let from_is_member = member_set.contains(&edge.from) || vessel == Some(edge.from);
            let to_is_member = member_set.contains(&edge.to) || vessel == Some(edge.to);

            if from_is_member {
                let current_to = self.endpoint_before_cluster_abduction(edge.to, cluster_index);
                // getExternalConnectedNodes filters retained pointers whose
                // node was subsequently removed from the current graph by a
                // later aggregate. In the stable arena that is exactly a
                // pointer whose final active owner has changed.
                if self.active_aggregate_owner(current_to) == current_to
                    && self.position(current_to).is_some()
                    && added.insert(current_to)
                {
                    external.push(current_to);
                }
            }

            if to_is_member {
                let current_from = if from_is_member {
                    cluster.members[0]
                } else {
                    self.endpoint_before_cluster_abduction(edge.from, cluster_index)
                };
                if self.active_aggregate_owner(current_from) == current_from
                    && self.position(current_from).is_some()
                    && added.insert(current_from)
                {
                    external.push(current_from);
                }
            }
        }
        external
    }

    /// Projects sequence vessels whose pointers were retained by this
    /// cluster's edge abductions into TALA's current `Graph.Nodes` shape.
    ///
    /// Weftan keeps every sequence member in its stable arena. TALA instead
    /// keeps the temporary sequence vessel in the graph while the cluster
    /// transaction runs, even though the cluster owns only a retained pointer
    /// to it. Removing the non-owner members from `node_order` activates the
    /// existing aggregate sizing, movement, and overlap semantics for exactly
    /// the lifetime of that transaction.
    fn project_cluster_retained_sequence_vessels(
        &mut self,
        cluster_index: usize,
        external: &[NodeId],
    ) -> BTreeSet<NodeId> {
        let retained_owners = self
            .sequences
            .iter()
            .filter_map(|sequence| {
                let owner = sequence.members.first().copied()?;
                (external.contains(&owner)
                    && self.endpoint_before_cluster_abduction(owner, cluster_index) == owner)
                    .then_some(owner)
            })
            .collect::<BTreeSet<_>>();
        let mut hidden = self
            .sequences
            .iter()
            .filter(|sequence| {
                sequence
                    .members
                    .first()
                    .is_some_and(|owner| retained_owners.contains(owner))
            })
            .flat_map(|sequence| sequence.members.iter().skip(1).copied())
            .collect::<BTreeSet<_>>();
        // Cluster preprocessing has the same lifetime: every cluster vessel
        // is still present until CleanupStuff. Represent those vessels during
        // Commit validation so a moved retained sequence vessel is checked
        // against other current cluster vessels, rather than their restored
        // stable members.
        hidden.extend(
            self.clusters
                .iter()
                .flat_map(|cluster| cluster.members.iter().skip(1).copied()),
        );
        hidden.extend(
            self.sequences
                .iter()
                .flat_map(|sequence| sequence.members.iter().skip(1).copied()),
        );
        self.node_order.retain(|node| !hidden.contains(node));
        retained_owners
    }

    fn desired_arrangement_from_external_geometry(
        &self,
        cluster_index: usize,
        current: ClusterArrangement,
    ) -> ClusterArrangement {
        let members = &self.clusters[cluster_index].members;
        let Some(&vessel_owner) = members.first() else {
            return current;
        };
        // Go classifies external edges against Cluster.Vessel's actual box.
        // Member bounds omit the vessel's padding, which can change a
        // diagonal edge's orientation when an external node lies near that
        // padded boundary. The stable arena stores the vessel origin and size
        // separately from its member boxes.
        let Some(vessel_position) = self.active_node_position(vessel_owner) else {
            return current;
        };
        let vessel = Rect {
            origin: vessel_position,
            size: self.cluster_vessel_size(cluster_index),
        };
        let mut horizontal_count = 0;
        let mut vertical_count = 0;
        let trace_orientation = crate::internal::trace_env_value("WEFTAN_TRACE_CLUSTER_FLIP")
            .as_deref()
            == Some("orientation");
        if trace_orientation {
            eprintln!(
                "CLUSTER_ORIENT_RUST vessel={} vessel_rect=({},{};{}x{})",
                self.clusters[cluster_index].vessel_tala_id,
                vessel.origin.x,
                vessel.origin.y,
                vessel.size.width,
                vessel.size.height,
            );
        }
        for external in self.cluster_external_nodes(cluster_index) {
            let external_rect = Rect {
                origin: self.position(external).unwrap(),
                size: self.active_node_size(external),
            };
            let separated_horizontally =
                external_rect.right() < vessel.origin.x || vessel.right() < external_rect.origin.x;
            let separated_vertically = external_rect.bottom() < vessel.origin.y
                || vessel.bottom() < external_rect.origin.y;
            if separated_horizontally && !separated_vertically {
                horizontal_count += 1;
            } else if separated_vertically && !separated_horizontally {
                vertical_count += 1;
            } else if separated_horizontally && separated_vertically {
                let x_distance = (external_rect.center().x - vessel.center().x).abs();
                let y_distance = (external_rect.center().y - vessel.center().y).abs();
                if y_distance < x_distance {
                    horizontal_count += 1;
                } else if x_distance < y_distance {
                    vertical_count += 1;
                }
            }
            if trace_orientation {
                eprintln!(
                    "CLUSTER_ORIENT_RUST external={} rect=({},{};{}x{}) sep_h={} sep_v={} counts={}x{}",
                    self.nodes[external.0 as usize].tala_id,
                    external_rect.origin.x,
                    external_rect.origin.y,
                    external_rect.size.width,
                    external_rect.size.height,
                    separated_horizontally,
                    separated_vertically,
                    horizontal_count,
                    vertical_count,
                );
            }
        }
        if vertical_count > horizontal_count {
            ClusterArrangement::Row
        } else if vertical_count < horizontal_count {
            ClusterArrangement::Column
        } else {
            current
        }
    }

    /// optimizeCluster writes the preference on its shared Cluster before
    /// taking a flip snapshot. Rejected flips retain this mutation, even when
    /// no geometry changes and the stage returns false.
    pub(super) fn set_cluster_desired_arrangement(
        &mut self,
        index: usize,
        desired: ClusterArrangement,
    ) {
        self.clusters[index].desired_arrangement = desired;
        if let Some(group) = self.owner_group_binding(TopologyVessel::Cluster(index)) {
            let value = match desired {
                ClusterArrangement::Row => "Row",
                ClusterArrangement::Column => "Column",
            };
            self.set_owner_cluster_desired_arrangement(group, value.to_owned())
                .expect("bound cluster policy");
        }
    }

    /// Recovered desired-arrangement selection from `Cluster.optimize`.
    ///
    /// This updates the requested state only. A failed TALA flip deliberately
    /// leaves `Arrangement` and `DesiredArrangement` unequal.
    pub(super) fn update_cluster_desired_arrangements(&mut self) {
        for cluster_index in 0..self.clusters.len() {
            let desired = {
                let cluster = &self.clusters[cluster_index];
                self.desired_arrangement_from_external_geometry(
                    cluster_index,
                    cluster.desired_arrangement,
                )
            };
            if crate::internal::trace_env_value("WEFTAN_TRACE_CLUSTER_FLIP").as_deref()
                == Some("all")
            {
                eprintln!(
                    "CLUSTER_DESIRED_RUST vessel={} current={:?} desired={:?}",
                    self.clusters[cluster_index].vessel_tala_id,
                    self.clusters[cluster_index].arrangement,
                    desired
                );
            }
            self.set_cluster_desired_arrangement(cluster_index, desired);
        }
    }

    fn initial_cluster_arrangement(
        &self,
        members: &[NodeId],
        consider_positions: bool,
        is_connected_to_sequence: bool,
        rng: &mut go_rng::GoRng,
    ) -> Option<ClusterArrangement> {
        if consider_positions {
            let mut all_horizontal = true;
            let mut all_vertical = true;
            for first in members {
                for second in members {
                    if first == second {
                        continue;
                    }
                    let first_node = &self.nodes[first.0 as usize];
                    let second_node = &self.nodes[second.0 as usize];
                    let first_rect = Rect {
                        origin: first_node.position?,
                        size: first_node.rect.size,
                    };
                    let second_rect = Rect {
                        origin: second_node.position?,
                        size: second_node.rect.size,
                    };
                    let horizontal = first_rect.right() <= second_rect.origin.x
                        || second_rect.right() <= first_rect.origin.x;
                    let vertical = first_rect.bottom() <= second_rect.origin.y
                        || second_rect.bottom() <= first_rect.origin.y;
                    if horizontal == vertical {
                        // TALA returns BadStateErr when getOrientation has no
                        // unique horizontal or vertical orientation.
                        return None;
                    }
                    if vertical {
                        all_horizontal = false;
                    }
                    if horizontal {
                        all_vertical = false;
                    }
                }
            }
            if all_horizontal {
                return Some(ClusterArrangement::Row);
            }
            if all_vertical {
                return (!is_connected_to_sequence).then_some(ClusterArrangement::Column);
            }
            let bounds = self.cluster_bounds(members)?;
            if bounds.size.height < bounds.size.width {
                return Some(ClusterArrangement::Row);
            }
            return (!is_connected_to_sequence).then_some(ClusterArrangement::Column);
        }

        let count = members.len() as f64;
        let average_width = (members
            .iter()
            .map(|member| self.nodes[member.0 as usize].rect.size.width)
            .sum::<f64>()
            / count)
            .round();
        let average_height = (members
            .iter()
            .map(|member| self.nodes[member.0 as usize].rect.size.height)
            .sum::<f64>()
            / count)
            .round();
        if is_connected_to_sequence {
            return Some(ClusterArrangement::Row);
        }
        if average_height < average_width {
            Some(ClusterArrangement::Column)
        } else if average_width < average_height {
            Some(ClusterArrangement::Row)
        } else if rng.float64() > 0.5 {
            Some(ClusterArrangement::Column)
        } else {
            Some(ClusterArrangement::Row)
        }
    }

    /// Recovered `Cluster.getPaddingBetween(false)`.
    fn initial_cluster_padding(&self, members: &[NodeId], arrangement: ClusterArrangement) -> f64 {
        let nodes = members
            .iter()
            .map(|member| &self.nodes[member.0 as usize])
            .collect::<Vec<_>>();
        Self::cluster_padding_for_nodes(&nodes, arrangement)
    }

    fn cluster_padding_for_nodes(members: &[&ArenaNode], arrangement: ClusterArrangement) -> f64 {
        let count = members.len() as f64;
        let average_extent = members
            .iter()
            .map(|member| {
                let size = member.rect.size;
                match arrangement {
                    ClusterArrangement::Row => size.width,
                    ClusterArrangement::Column => size.height,
                }
            })
            .sum::<f64>()
            / count;
        let mut padding = 20.0_f64.max(average_extent.round().ceil() * 0.1);
        if members.iter().any(|member| member.has_icon) {
            let maximum_label_extent = members
                .iter()
                .filter_map(|member| member.label_size)
                .map(|size| match arrangement {
                    ClusterArrangement::Row => size.width,
                    ClusterArrangement::Column => size.height,
                })
                .fold(0.0_f64, f64::max);
            padding = padding.max(maximum_label_extent + 10.0);
        }
        padding.round()
    }

    /// container_orientation.go115–121 also flips clusters carried only by
    /// the dense graph's retained external group map. They are not entries
    /// in this workspace's `clusters` vector.
    pub(super) fn rotate_projected_cluster_for_orientation(&mut self, node: NodeId) {
        let id = self.nodes[node.0 as usize].tala_id;
        let Some(layout) = self.transaction_external_cluster_layouts.get(&id).copied() else {
            return;
        };
        let Some(members) = self.transaction_external_aggregate_children.get(&id) else {
            return;
        };
        if members.is_empty() {
            return;
        }
        let arrangement = match layout.arrangement {
            ClusterArrangement::Row => ClusterArrangement::Column,
            ClusterArrangement::Column => ClusterArrangement::Row,
        };
        let refs = members.iter().collect::<Vec<_>>();
        let given = Self::cluster_padding_for_nodes(&refs, arrangement);
        let distances = members
            .windows(2)
            .map(|pair| {
                Self::cluster_rect_distance(
                    Rect {
                        origin: pair[0].position.expect("positioned orientation member"),
                        size: pair[0].rect.size,
                    },
                    Rect {
                        origin: pair[1].position.expect("positioned orientation member"),
                        size: pair[1].rect.size,
                    },
                )
            })
            .sum::<f64>()
            / (members.len() - 1) as f64;
        let padding = if distances > 0.0 {
            distances.min(given).round()
        } else {
            given
        };
        let maximum = members.iter().fold(Size::default(), |size, member| Size {
            width: size.width.max(member.rect.size.width),
            height: size.height.max(member.rect.size.height),
        });
        let count = members.len() as f64;
        let size = match arrangement {
            ClusterArrangement::Row => Size {
                width: maximum.width * count + padding * (count - 1.0),
                height: maximum.height,
            },
            ClusterArrangement::Column => Size {
                width: maximum.width,
                height: maximum.height * count + padding * (count - 1.0),
            },
        };
        if !layout.fixed_size {
            let member_ids = members
                .iter()
                .map(|member| member.tala_id)
                .collect::<Vec<_>>();
            for member_id in member_ids {
                self.set_projected_node_size(member_id, maximum);
            }
        }
        let updated = self
            .transaction_external_cluster_layouts
            .get_mut(&id)
            .unwrap();
        updated.arrangement = arrangement;
        updated.padding = padding;
        self.nodes[node.0 as usize].scoring_cluster_arrangement = Some(arrangement);
        self.set_projected_node_size(id, size);
    }

    /// Recovered `Node.distanceTo(other, true)` for positioned node boxes.
    fn cluster_rect_distance(first: Rect, second: Rect) -> f64 {
        let horizontal = if first.right() < second.origin.x {
            second.origin.x - first.right()
        } else if second.right() < first.origin.x {
            first.origin.x - second.right()
        } else {
            0.0
        };
        let vertical = if first.bottom() < second.origin.y {
            second.origin.y - first.bottom()
        } else if second.bottom() < first.origin.y {
            first.origin.y - second.bottom()
        } else {
            0.0
        };
        if horizontal > 0.0 && vertical > 0.0 {
            crate::internal::routing::channel_points::go_hypot(horizontal, vertical)
        } else {
            horizontal.max(vertical)
        }
    }

    /// Recovered `Cluster.getPaddingBetween(true)`.
    ///
    /// The ordinary arrangement-derived padding remains the upper bound. TALA
    /// preserves a smaller average gap already present between consecutive
    /// members when flipping a positioned cluster.
    pub(super) fn prearranged_cluster_padding(
        &self,
        cluster_index: usize,
        arrangement: ClusterArrangement,
    ) -> f64 {
        let cluster = &self.clusters[cluster_index];
        let given = self.initial_cluster_padding(&cluster.members, arrangement);
        let distances = cluster
            .members
            .windows(2)
            .filter_map(|pair| {
                let first = &self.nodes[pair[0].0 as usize];
                let second = &self.nodes[pair[1].0 as usize];
                Some(Self::cluster_rect_distance(
                    Rect {
                        origin: first.position?,
                        size: first.rect.size,
                    },
                    Rect {
                        origin: second.position?,
                        size: second.rect.size,
                    },
                ))
            })
            .collect::<Vec<_>>();
        if distances.is_empty() {
            return given.round();
        }
        let average = distances.iter().sum::<f64>() / distances.len() as f64;
        if average > 0.0 {
            average.min(given).round()
        } else {
            given.round()
        }
    }

    fn cluster_flip_trial(&self, cluster_index: usize, try_center: bool) -> Option<Self> {
        let prior_overlaps = self.existing_overlap_pairs();
        let prior_exact_overlaps = self.exact_overlap_pairs();
        let prior_bounds = self.cluster_bounds(&self.clusters[cluster_index].members)?;
        // `Cluster.SyncGeometry` resizes and arranges the temporary vessel at
        // its existing vessel top-left. The stable arena stores that carrier
        // position separately from the retained member boxes; using the
        // member bounds here shifts a Column→Row flip by the old column
        // height and changes the transaction's fit/overlap result.
        let prior_vessel_top_left = self
            .active_node_position(self.clusters[cluster_index].members[0])
            .unwrap_or(prior_bounds.origin);
        if crate::internal::trace_env_value("WEFTAN_TRACE_CLUSTER_FLIP").as_deref() == Some("all")
            && self.clusters[cluster_index].vessel_tala_id == 6019456696934794384
        {
            eprintln!(
                "CLUSTER_PRIOR_RUST vessel={} prior_bounds={:?} active={:?} pending={:?}",
                self.clusters[cluster_index].vessel_tala_id,
                prior_bounds,
                self.active_node_position(self.clusters[cluster_index].members[0]),
                self.pending_cluster_vessel_positions
            );
        }
        let prior_size = self.cluster_vessel_size(cluster_index);

        let mut trial = self.clone();
        let desired = trial.clusters[cluster_index].desired_arrangement;
        trial.clusters[cluster_index].arrangement = desired;
        trial.clusters[cluster_index].padding =
            trial.prearranged_cluster_padding(cluster_index, desired);
        trial.resize_cluster_members(cluster_index);
        // The trial changes the active arrangement and padding.  Refresh the
        // temporary vessel geometry for this transaction only; ordinary
        // Resize calls must not overwrite the vessel captured when the
        // cluster was created while nested members are subsequently fitted.
        if trial.cluster_vessel_sizes.len() <= cluster_index {
            trial
                .cluster_vessel_sizes
                .resize(cluster_index + 1, Size::default());
        }
        trial.cluster_vessel_sizes[cluster_index] =
            trial.cluster_vessel_size_for_arrangement(cluster_index, desired);
        // Source optimizeCluster mutates the shared Cluster and SyncGeometry
        // resizes its distinct vessel inside the request transaction. Keep the
        // canonical owner policy and every vessel view in this detached trial;
        // a rejected trial must leave the original graph untouched.
        if let Some(group) = trial.owner_group_binding(TopologyVessel::Cluster(cluster_index)) {
            let arrangement = match desired {
                ClusterArrangement::Row => "Row",
                ClusterArrangement::Column => "Column",
            };
            trial
                .set_owner_cluster_orientation_policy(
                    group,
                    arrangement.to_owned(),
                    arrangement.to_owned(),
                    trial.clusters[cluster_index].padding,
                )
                .expect("bound cluster policy");
        }
        let vessel_id = trial.clusters[cluster_index].vessel_tala_id;
        let vessel_size = trial.cluster_vessel_sizes[cluster_index];
        trial.set_projected_node_size(vessel_id, vessel_size);
        trial.arrange_cluster_members(cluster_index, prior_vessel_top_left);

        if try_center {
            let new_size = trial.cluster_vessel_size(cluster_index);
            let delta = match desired {
                ClusterArrangement::Row => Point {
                    x: ((prior_size.width - new_size.width) / 2.0).round(),
                    // Cluster.SyncGeometry keeps the vessel's existing y
                    // coordinate for a Column→Row flip; only the width
                    // centering move is applied by the surrounding trial.
                    y: 0.0,
                },
                ClusterArrangement::Column => Point {
                    x: 0.0,
                    y: ((prior_size.height - new_size.height) / 2.0).round(),
                },
            };
            // The source operation is `c.Vessel.moveNodeWithChildren`, not a
            // loop over the retained member pointers. The temporary vessel is
            // a distinct box in TALA, so move its pending projection together
            // with every member and descendant exactly once.
            let vessel = trial.clusters[cluster_index].members[0];
            trial.translate_active_node_with_children(vessel, delta);
        }

        if crate::internal::trace_env_value("WEFTAN_TRACE_CLUSTER_FLIP").as_deref() == Some("all")
            && self.clusters[cluster_index].vessel_tala_id == 6019456696934794384
        {
            let first = trial.clusters[cluster_index].members[0];
            eprintln!(
                "CLUSTER_AFTER_ARRANGE_RUST centered={} vessel={:?} size={:?} member2={:?}",
                try_center,
                trial.active_node_position(first),
                trial.active_node_size(first),
                trial.active_node_position(trial.clusters[cluster_index].members[1])
            );
        }

        // `Cluster.optimize` refits the cluster's immediate container inside
        // the transaction operation, then repositions that container's
        // children while preserving its existing top-left. This precedes the
        // transaction-wide smallest-to-largest `repositionContainers` pass.
        //
        // The distinction is observable: widening a cluster can keep its
        // immediate container anchored, move the siblings into the new inner
        // placement, and consequently widen an ancestor enough for Commit to
        // reject a new overlap.
        trial.refit_cluster_container_preserving_origin(cluster_index);

        // Cluster.optimize uses NewTransactionWithOptions(AffectContainers:
        // true). Commit refits containers, syncs every cluster, then rejects
        // new bad states against the graph state captured above.
        trial.reposition_ordinary_containers();
        let new_overlap = trial.transaction_has_new_overlap(&prior_overlaps);
        let regular_bad_state_overlap = trial.transaction_has_bad_state_overlap(&prior_overlaps);
        let bad_state_overlap = regular_bad_state_overlap;
        let spacing_exact =
            trial.existing_spacing_overlap_became_exact(&prior_overlaps, &prior_exact_overlaps);
        let containment_invalid = !trial.transaction_containment_is_valid();
        let origin_valid = trial.transaction_fixed_origin_is_valid();
        // Go validates the current Graph.Nodes view after the refit. Retained
        // cluster members are exempt; their distinct vessel has no authored
        // fixed position. Ordinary siblings can move in the anchored refit.
        let fixed_moved = trial.graph_node_order().into_iter().any(|node_id| {
            let node = &trial.nodes[node_id.0 as usize];
            node.cluster.is_none()
                && node.fixed_top_left.is_some()
                && trial.active_node_position(node_id) != self.active_node_position(node_id)
        });
        if crate::internal::trace_env_value("WEFTAN_TRACE_CLUSTER_FLIP").as_deref() == Some("all") {
            eprintln!(
                "CLUSTER_FLIP_CHECK_RUST vessel={} centered={} new_overlap={} bad_state={} spacing_exact={} containment_invalid={} origin={} fixed_moved={}",
                self.clusters[cluster_index].vessel_tala_id,
                try_center,
                new_overlap,
                regular_bad_state_overlap,
                spacing_exact,
                containment_invalid,
                origin_valid,
                fixed_moved
            );
            for target in [2600858170_i64, 753265730] {
                if let Some(index) = trial.nodes.iter().position(|node| node.tala_id == target) {
                    let container = NodeId(index as u32);
                    let outer = trial
                        .active_node_position(container)
                        .map(|position| (position, trial.active_node_size(container)));
                    let children = trial.container_node_order(Some(container));
                    eprintln!(
                        "CLUSTER_CONTAINER_CHECK_RUST vessel={} centered={} container={} outer={:?} children={:?}",
                        self.clusters[cluster_index].vessel_tala_id,
                        try_center,
                        target,
                        outer,
                        children
                            .iter()
                            .map(|child| (
                                trial.nodes[child.0 as usize].tala_id,
                                trial.active_node_position(*child),
                                trial.active_node_size(*child)
                            ))
                            .collect::<Vec<_>>()
                    );
                }
            }
            if self.clusters[cluster_index].vessel_tala_id == 6_129_484_611_666_145_821 {
                eprintln!(
                    "CLUSTER_EXTERNAL_CHECK_RUST vessel={} containers={:?} children={:?}",
                    self.clusters[cluster_index].vessel_tala_id,
                    trial
                        .transaction_external_containers
                        .iter()
                        .filter(|node| matches!(node.tala_id, 2600858170 | 753265730 | 2202785448))
                        .map(|node| (node.tala_id, node.position, node.rect.size))
                        .collect::<Vec<_>>(),
                    trial
                        .transaction_external_container_children
                        .iter()
                        .filter(|(parent, _)| matches!(**parent, 2600858170 | 753265730))
                        .map(|(parent, children)| {
                            (
                                *parent,
                                children
                                    .iter()
                                    .map(|node| (node.tala_id, node.position, node.rect.size))
                                    .collect::<Vec<_>>(),
                            )
                        })
                        .collect::<Vec<_>>()
                );
                eprintln!(
                    "CLUSTER_GRAPH_ORDER_CHECK_RUST vessel={} order={:?} nodes={:?}",
                    self.clusters[cluster_index].vessel_tala_id,
                    trial
                        .node_order
                        .iter()
                        .map(|node| trial.nodes[node.0 as usize].tala_id)
                        .filter(|id| matches!(
                            *id,
                            2600858170
                                | 753265730
                                | 2202785448
                                | 3946625030
                                | 3984593615
                                | 2328842519
                        ))
                        .collect::<Vec<_>>(),
                    trial
                        .nodes
                        .iter()
                        .filter(|node| matches!(
                            node.tala_id,
                            2600858170
                                | 753265730
                                | 2202785448
                                | 3946625030
                                | 3984593615
                                | 2328842519
                        ))
                        .map(|node| (
                            node.tala_id,
                            node.position,
                            node.rect.size,
                            node.cluster,
                            node.container
                        ))
                        .collect::<Vec<_>>()
                );
            }
        }
        if new_overlap
            || bad_state_overlap
            || spacing_exact
            || containment_invalid
            || !origin_valid
            || fixed_moved
        {
            return None;
        }
        Some(trial)
    }

    /// Apply the `onlyFlip` form of TALA's `optimizeCluster` to one cluster.
    ///
    /// Transpose runs this from inside each candidate transaction whenever a
    /// moved node is a cluster vessel.  The full `optimize_clusters` pass is
    /// intentionally too broad here: it also tries vessel/node alignment and
    /// walks every cluster, while upstream only flips the specific vessel that
    /// was moved by the current transpose operation.
    pub(super) fn optimize_cluster_only_flip(&mut self, cluster_index: usize) -> bool {
        #[cfg(feature = "diagnostic-traces")]
        self.trace_cluster_policy("before", cluster_index, true);
        let desired = {
            let cluster = &self.clusters[cluster_index];
            self.desired_arrangement_from_external_geometry(
                cluster_index,
                cluster.desired_arrangement,
            )
        };
        self.set_cluster_desired_arrangement(cluster_index, desired);
        #[cfg(feature = "diagnostic-traces")]
        self.trace_cluster_policy("selected", cluster_index, true);
        if self.clusters[cluster_index].arrangement == desired {
            return false;
        }

        let centered = self.cluster_flip_trial(cluster_index, true);
        let top_left = centered
            .is_none()
            .then(|| self.cluster_flip_trial(cluster_index, false))
            .flatten();
        if let Some(committed) = centered.or(top_left) {
            *self = committed;
            true
        } else {
            false
        }
    }

    fn refit_cluster_container_preserving_origin(&mut self, cluster_index: usize) {
        let trace_refit = crate::internal::trace_env_value("WEFTAN_TRACE_CLUSTER_REFIT")
            .is_some_and(|target| {
                target == "all" || target == self.clusters[cluster_index].vessel_tala_id.to_string()
            });
        let Some(first_member) = self.clusters[cluster_index].members.first().copied() else {
            return;
        };
        let Some(container) = self.active_node_container(first_member) else {
            return;
        };
        let Some(container_position) = self.position(container) else {
            return;
        };
        let children = self.container_node_order(Some(container));
        let Some((top_left, bottom_right)) = self.transaction_container_bounds(&children) else {
            return;
        };
        let content = Size {
            width: bottom_right.x - top_left.x,
            height: bottom_right.y - top_left.y,
        };
        let padding = self.shape_fit_padding(container);
        let fitted = self.shape_dimensions_to_fit(container, content, padding);
        self.nodes[container.0 as usize].rect.size = fitted;
        let inside = self.bin_pack_shape_inside_placement(container, content, padding);
        let delta = Point {
            x: container_position.x + inside.x - top_left.x,
            y: container_position.y + inside.y - top_left.y,
        };
        if trace_refit {
            eprintln!(
                "CLUSTER_REFIT_RUST vessel={} container={} container_position={:?} children={:?} bounds={:?},{:?} content={:?} padding={:?} fitted={:?} inside={:?} delta={:?}",
                self.clusters[cluster_index].vessel_tala_id,
                self.nodes[container.0 as usize].tala_id,
                container_position,
                children
                    .iter()
                    .map(|child| {
                        (
                            self.nodes[child.0 as usize].tala_id,
                            self.active_node_position(*child),
                            self.active_node_size(*child),
                            self.active_node_is_aggregate(*child),
                        )
                    })
                    .collect::<Vec<_>>(),
                top_left,
                bottom_right,
                content,
                padding,
                fitted,
                inside,
                delta,
            );
        }
        for child in children {
            self.translate_active_node_with_children(child, delta);
        }
        if trace_refit {
            eprintln!(
                "CLUSTER_REFIT_RUST_AFTER vessel={} container={} container_pos={:?} container_size={:?} active_first={:?}",
                self.clusters[cluster_index].vessel_tala_id,
                self.nodes[container.0 as usize].tala_id,
                self.position(container),
                self.nodes[container.0 as usize].rect.size,
                self.active_node_position(first_member),
            );
        }
    }

    fn cluster_reverse_dfs_order(&self) -> Vec<usize> {
        let mut order = Vec::with_capacity(self.clusters.len());
        for container in self
            .container_reverse_dfs_order()
            .into_iter()
            .map(Some)
            .chain([None])
        {
            order.extend(
                self.clusters
                    .iter()
                    .enumerate()
                    .filter(|(_, cluster)| {
                        cluster.members.first().is_some_and(|member| {
                            self.nodes[member.0 as usize].container == container
                        })
                    })
                    .map(|(index, _)| index),
            );
        }
        order
    }

    /// One recovered AffectContainers trial for `alignVessel` or
    /// `alignConnectedNodes`. The caller compares the two rolled-back scores
    /// and commits the same winner as `Cluster.optimize`.
    fn cluster_alignment_trial(&self, cluster_index: usize, move_vessel: bool) -> Option<Self> {
        let stable_node_order = self.node_order.clone();
        let external = self.cluster_external_nodes(cluster_index);
        let mut trial = self.clone();
        let retained_vessels =
            trial.project_cluster_retained_sequence_vessels(cluster_index, &external);
        let prior_retained_overlaps = trial.retained_vessel_overlap_pairs(&retained_vessels);
        let prior_overlaps = trial.existing_overlap_pairs();
        let prior_exact_overlaps = trial.exact_overlap_pairs();
        let cluster = trial.clusters[cluster_index].clone();
        if external.is_empty() {
            trial.node_order = stable_node_order;
            return Some(trial);
        }
        // TALA aligns against the temporary `c.Vessel` box, whose dimensions
        // include the full cluster spacing.  The stable arena keeps that
        // frame in `pending_cluster_vessel_positions`; deriving a box from
        // member extrema drops the trailing gap and changes the rounded
        // center by a few pixels (visible in cluster edge routing).
        let vessel_origin = trial
            .pending_cluster_vessel_positions
            .get(&cluster_index)
            .copied()
            .or_else(|| {
                trial
                    .cluster_bounds(&cluster.members)
                    .map(|bounds| bounds.origin)
            })?;
        let vessel = Rect {
            origin: vessel_origin,
            size: trial.cluster_vessel_size(cluster_index),
        };
        let horizontally = cluster.arrangement == ClusterArrangement::Row;

        #[cfg(feature = "diagnostic-traces")]
        if crate::internal::trace_env_value("WEFTAN_TRACE_CLUSTER_FLIP").as_deref() == Some("all") {
            eprintln!(
                "CLUSTER_ALIGN_RUST vessel={} trial={} external={:?} vessel_rect={:?} baseline_raw_turn={}",
                cluster.vessel_tala_id,
                if move_vessel { "vessel" } else { "nodes" },
                external
                    .iter()
                    .map(|node| {
                        (
                            self.active_node_tala_id(*node),
                            self.active_node_position(*node),
                            self.active_node_size(*node),
                        )
                    })
                    .collect::<Vec<_>>(),
                vessel,
                self.raw_turn_cost(),
            );
        }

        if move_vessel {
            let average_center = external.iter().fold(Point::default(), |sum, node| {
                let center = trial.active_node_center(*node);
                Point {
                    x: sum.x + center.x,
                    y: sum.y + center.y,
                }
            });
            let count = external.len() as f64;
            let delta = if horizontally {
                Point {
                    x: (average_center.x / count - vessel.center().x).round(),
                    y: 0.0,
                }
            } else {
                Point {
                    x: 0.0,
                    y: (average_center.y / count - vessel.center().y).round(),
                }
            };
            #[cfg(feature = "diagnostic-traces")]
            if crate::internal::trace_env_value("WEFTAN_TRACE_CLUSTER_FLIP").as_deref()
                == Some("all")
            {
                eprintln!(
                    "CLUSTER_DELTA_RUST vessel={} trial=vessel delta={:?} pending_before={:?}",
                    cluster.vessel_tala_id, delta, trial.pending_cluster_vessel_positions,
                );
            }
            trial.translate_active_node_box(cluster.members[0], delta);
        } else {
            for node_id in external {
                if !trial.active_node_is_aggregate(node_id)
                    && trial.nodes[node_id.0 as usize].fixed_top_left.is_some()
                {
                    continue;
                }
                let node_rect = Rect {
                    origin: trial.position(node_id).unwrap(),
                    size: trial.active_node_size(node_id),
                };
                let overlaps_cross_axis = if horizontally {
                    node_rect.origin.x <= vessel.right() && vessel.origin.x <= node_rect.right()
                } else {
                    node_rect.origin.y <= vessel.bottom() && vessel.origin.y <= node_rect.bottom()
                };
                if !overlaps_cross_axis {
                    continue;
                }
                let delta = if horizontally {
                    Point {
                        x: (vessel.center().x - node_rect.center().x).round(),
                        y: 0.0,
                    }
                } else {
                    Point {
                        x: 0.0,
                        y: (vessel.center().y - node_rect.center().y).round(),
                    }
                };
                trial.translate_active_node_with_children(node_id, delta);
            }
        }

        trial.reposition_ordinary_containers();
        let origin_valid = trial.transaction_fixed_origin_is_valid();
        // Commit checks saved positions in the current Go Graph.Nodes view,
        // including ordinary descendants moved with a connected container.
        // Distinct aggregate vessels have no authored FixedTopLeft; their
        // retained arena members keep that metadata only for restoration.
        let fixed_moved = trial.graph_node_order().into_iter().any(|node_id| {
            let node = &trial.nodes[node_id.0 as usize];
            !trial.active_node_is_aggregate(node_id)
                && node.cluster.is_none()
                && node.fixed_top_left.is_some()
                && trial.active_node_position(node_id) != self.active_node_position(node_id)
        });
        #[cfg(feature = "diagnostic-traces")]
        if crate::internal::trace_env_value("WEFTAN_TRACE_CLUSTER_FLIP").as_deref() == Some("all") {
            eprintln!(
                "CLUSTER_ALIGN_RUST vessel={} trial={} raw_turn={} reject={} origin_valid={} fixed_moved={}",
                cluster.vessel_tala_id,
                if move_vessel { "vessel" } else { "nodes" },
                trial.raw_turn_cost(),
                trial.transaction_has_new_overlap(&prior_overlaps)
                    || trial.transaction_has_bad_state_overlap(&prior_overlaps)
                    || !trial
                        .retained_vessel_overlap_pairs(&retained_vessels)
                        .is_subset(&prior_retained_overlaps)
                    || trial.existing_spacing_overlap_became_exact(
                        &prior_overlaps,
                        &prior_exact_overlaps
                    )
                    || !trial.transaction_containment_is_valid()
                    || !origin_valid
                    || fixed_moved,
                origin_valid,
                fixed_moved,
            );
            for target in [1408816216_i64, 3860869894, 5920220759044228662] {
                if let Some(node) = trial.nodes.iter().find(|node| node.tala_id == target) {
                    eprintln!(
                        "CLUSTER_ALIGN_RUST_NODE vessel={} trial={} node={} pos={:?} size={},{}",
                        cluster.vessel_tala_id,
                        if move_vessel { "vessel" } else { "nodes" },
                        target,
                        node.position,
                        node.rect.size.width,
                        node.rect.size.height,
                    );
                }
            }
        }
        // Original Transaction.Commit admits these alignment candidates
        // without a size cap. The completed OptimizeClusters stage checks
        // presentation bounds after the winning candidate has been applied.
        if trial.transaction_has_new_overlap(&prior_overlaps)
            || trial.transaction_has_bad_state_overlap(&prior_overlaps)
            || !trial
                .retained_vessel_overlap_pairs(&retained_vessels)
                .is_subset(&prior_retained_overlaps)
            || trial.existing_spacing_overlap_became_exact(&prior_overlaps, &prior_exact_overlaps)
            || !trial.transaction_containment_is_valid()
            || !origin_valid
            || fixed_moved
        {
            return None;
        }
        trial.node_order = stable_node_order;
        Some(trial)
    }

    /// Original OptimizeClusters request lifetime. Guard and operation
    /// contexts remain distinct across transactions, scores and rollback.
    pub(super) fn optimize_clusters_with_context(
        &mut self,
        operation_context: &dyn limits::Context,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<bool, limits::Error> {
        self.optimize_clusters_transactional(operation_context, guard)
    }

    pub(super) fn optimize_clusters(&mut self) -> bool {
        let mut changed = false;
        if crate::internal::trace_env_value("WEFTAN_TRACE_CLUSTER_FLIP").as_deref() == Some("all") {
            eprintln!(
                "CLUSTER_META_RUST root_container={:?} node_order={:?}",
                self.containers
                    .get(&None)
                    .into_iter()
                    .flatten()
                    .map(|node| self.nodes[node.0 as usize].tala_id)
                    .collect::<Vec<_>>(),
                self.node_order
                    .iter()
                    .map(|node| self.nodes[node.0 as usize].tala_id)
                    .collect::<Vec<_>>()
            );
            for sequence in &self.sequences {
                eprintln!(
                    "CLUSTER_META_RUST sequence vessel={} members={:?} container={:?}",
                    sequence.vessel_tala_id,
                    sequence
                        .members
                        .iter()
                        .map(|node| self.nodes[node.0 as usize].tala_id)
                        .collect::<Vec<_>>(),
                    sequence
                        .container
                        .map(|node| self.nodes[node.0 as usize].tala_id)
                );
                for node in &sequence.members {
                    eprintln!(
                        "CLUSTER_META_RUST sequence_member={} parent={:?}",
                        self.nodes[node.0 as usize].tala_id,
                        self.nodes[node.0 as usize]
                            .container
                            .map(|parent| self.nodes[parent.0 as usize].tala_id)
                    );
                }
            }
            for cluster in &self.clusters {
                eprintln!(
                    "CLUSTER_META_RUST cluster vessel={} members={:?}",
                    cluster.vessel_tala_id,
                    cluster
                        .members
                        .iter()
                        .map(|node| self.nodes[node.0 as usize].tala_id)
                        .collect::<Vec<_>>()
                );
            }
        }
        for cluster_index in self.cluster_reverse_dfs_order() {
            #[cfg(feature = "diagnostic-traces")]
            self.trace_cluster_policy("before", cluster_index, false);
            if crate::internal::trace_env_value("WEFTAN_TRACE_CLUSTER_FLIP").as_deref()
                == Some("all")
            {
                let cluster = &self.clusters[cluster_index];
                let external = self.cluster_external_nodes(cluster_index);
                eprintln!(
                    "CLUSTER_STATE_RUST vessel={} arrangement={:?} desired={:?} external={:?}",
                    cluster.vessel_tala_id,
                    cluster.arrangement,
                    cluster.desired_arrangement,
                    external
                        .iter()
                        .map(|node| self.nodes[node.0 as usize].tala_id)
                        .collect::<Vec<_>>(),
                );
            }
            let desired = {
                let cluster = &self.clusters[cluster_index];
                self.desired_arrangement_from_external_geometry(
                    cluster_index,
                    cluster.desired_arrangement,
                )
            };
            if crate::internal::trace_env_value("WEFTAN_TRACE_CLUSTER_FLIP").as_deref()
                == Some("all")
            {
                eprintln!(
                    "CLUSTER_DESIRED_OPT_RUST vessel={} current={:?} desired={:?}",
                    self.clusters[cluster_index].vessel_tala_id,
                    self.clusters[cluster_index].arrangement,
                    desired
                );
            }
            self.set_cluster_desired_arrangement(cluster_index, desired);
            #[cfg(feature = "diagnostic-traces")]
            self.trace_cluster_policy("selected", cluster_index, false);
            if self.clusters[cluster_index].arrangement != desired {
                let centered = self.cluster_flip_trial(cluster_index, true);
                let top_left = centered
                    .is_none()
                    .then(|| self.cluster_flip_trial(cluster_index, false))
                    .flatten();
                if crate::internal::trace_env_value("WEFTAN_TRACE_CLUSTER_FLIP").as_deref()
                    == Some("all")
                {
                    eprintln!(
                        "CLUSTER_FLIP_RESULT_RUST vessel={} desired={:?} centered={} top_left={}",
                        self.clusters[cluster_index].vessel_tala_id,
                        desired,
                        centered.is_some(),
                        top_left.is_some()
                    );
                }
                if let Some(committed) = centered.or(top_left) {
                    self.publish_cluster_optimization_candidate(committed);
                    changed = true;
                }
            }
            if self.clusters[cluster_index].arrangement
                != self.clusters[cluster_index].desired_arrangement
            {
                continue;
            }

            let baseline = self.global_sized_edge_length_after_tree_restoration(true);
            let vessel_trial = self.cluster_alignment_trial(cluster_index, true);
            let mut best_length = baseline;
            let mut best_trial = None;
            if let Some(trial) = vessel_trial {
                let length = trial.global_sized_edge_length_after_tree_restoration(true);
                if cluster_precision_compare(length, best_length) == Ordering::Less {
                    best_length = length;
                    best_trial = Some(trial);
                }
            }
            if let Some(trial) = self.cluster_alignment_trial(cluster_index, false) {
                let length = trial.global_sized_edge_length_after_tree_restoration(true);
                if crate::internal::trace_env_value("WEFTAN_TRACE_CLUSTER_FLIP").as_deref()
                    == Some("all")
                {
                    eprintln!(
                        "CLUSTER_SCORE_RUST vessel={} trial=nodes baseline_bits=0x{:016x} length_bits=0x{:016x} diff={:.17e} cmp={:?}",
                        self.clusters[cluster_index].vessel_tala_id,
                        baseline.to_bits(),
                        length.to_bits(),
                        length - baseline,
                        cluster_precision_compare(length, best_length)
                    );
                }
                if cluster_precision_compare(length, best_length) == Ordering::Less {
                    best_trial = Some(trial);
                }
            }
            if let Some(committed) = best_trial {
                if crate::internal::trace_env_value("WEFTAN_TRACE_CLUSTER_FLIP").as_deref()
                    == Some("all")
                {
                    eprintln!(
                        "CLUSTER_BEST_RUST vessel={} baseline={:.17e} best={:.17e} cmp={:?}",
                        self.clusters[cluster_index].vessel_tala_id,
                        baseline,
                        best_length,
                        cluster_precision_compare(best_length, baseline)
                    );
                }
                self.publish_cluster_optimization_candidate(committed);
                changed = true;
                self.reduce_cluster_gap_to_neighbors(
                    cluster_index,
                    self.clusters[cluster_index].arrangement == ClusterArrangement::Column,
                    true,
                    true,
                );
                // Graph.optimizeClusters wraps the gap reduction in an
                // AffectContainers transaction. Even when the reduction is
                // a no-op (for example, no node lies ahead), Transaction
                // still refits ordinary containers and publishes clusters
                // and sequences before committing the operation. Keep that
                // outer lifecycle explicit in the stable arena.
                self.reposition_ordinary_containers();
            }
            if crate::internal::trace_env_value("WEFTAN_TRACE_CLUSTER_FLIP").as_deref()
                == Some("all")
            {
                eprint!(
                    "CLUSTER_AFTER_RUST vessel={} changed={} ",
                    self.clusters[cluster_index].vessel_tala_id, changed
                );
                for target in [1408816216_i64, 3860869894, 5920220759044228662] {
                    if let Some(node) = self.nodes.iter().find(|node| node.tala_id == target) {
                        eprint!(
                            "{}@{:?}:{},{} ",
                            target, node.position, node.rect.size.width, node.rect.size.height
                        );
                    }
                }
                eprintln!();
            }
        }
        changed
    }

    fn unique_neighbors_in_edge_order(&self, node: NodeId) -> Vec<NodeId> {
        let mut seen = BTreeSet::new();
        self.nodes[node.0 as usize]
            .edges
            .read()
            .iter()
            .filter_map(|edge_id| {
                let edge = &self.edge_records[edge_id.0 as usize];
                let adjacent = if edge.from == node && edge.to != node {
                    Some(edge.to)
                } else if edge.to == node && edge.from != node {
                    Some(edge.from)
                } else {
                    None
                }?;
                // cluster_discovery.go refreshNeighbors/sequenceOriginal:
                // discovery compares original steps even though the live
                // edge is now connected to the allocated sequence vessel.
                let adjacent = if let Some(Some(group)) = self
                    .owner_graph_group_entry(
                        OwnerGraphId(0),
                        super::owner_groups::OwnerGroupKind::Sequence,
                        Some(adjacent),
                    )
                    .expect("cluster sequence lookup")
                {
                    let header = self
                        .owner_sequence_abductions(group)
                        .expect("sequence abductions");
                    header
                        .into_iter()
                        .flat_map(|header| header.iter_slots())
                        .find_map(|id| {
                            let record = self
                                .owner_edge_abduction(id.expect("sequence abduction pointer"))
                                .unwrap();
                            if record.source_edge != Some(*edge_id) {
                                return None;
                            }
                            if record.current_from == Some(adjacent) {
                                Some(record.originally_from)
                            } else if record.current_to == Some(adjacent) {
                                Some(record.originally_to)
                            } else {
                                None
                            }
                        })
                        .flatten()
                        .expect("cluster sequence neighbor has an original step")
                } else {
                    adjacent
                };
                seen.insert(adjacent).then_some(adjacent)
            })
            .collect()
    }

    fn unique_neighbors(&self, node: NodeId) -> BTreeSet<NodeId> {
        self.unique_neighbors_in_edge_order(node)
            .into_iter()
            .collect()
    }

    fn edge_signature(&self, node: NodeId) -> EdgeSignature {
        let mut signature = EdgeSignature {
            bidirectional: 0,
            undirected: 0,
            directed: 0,
            from: 0,
            to: 0,
            node_side_arrowheads: BTreeSet::new(),
            other_side_arrowheads: BTreeSet::new(),
        };
        for edge_index in self.member_edge_indices() {
            let edge = &self.edge_records[edge_index];
            if edge.from != node && edge.to != node {
                continue;
            }
            match (edge.source_arrow, edge.target_arrow) {
                (true, true) => signature.bidirectional += 1,
                (false, false) => signature.undirected += 1,
                _ => signature.directed += 1,
            }
            if edge.from == node {
                signature.from += 1;
                signature
                    .node_side_arrowheads
                    .insert(edge.source_arrowhead.clone());
                signature
                    .other_side_arrowheads
                    .insert(edge.target_arrowhead.clone());
            }
            if edge.to == node {
                signature.to += 1;
                signature
                    .node_side_arrowheads
                    .insert(edge.target_arrowhead.clone());
                signature
                    .other_side_arrowheads
                    .insert(edge.source_arrowhead.clone());
            }
        }
        signature
    }

    fn cluster_eligible(&self, node: NodeId, tree_sentinels: &BTreeSet<NodeId>) -> bool {
        let node_ref = &self.nodes[node.0 as usize];
        !tree_sentinels.contains(&node)
            && self.owner_graph_group_entry(
                OwnerGraphId(0), super::owner_groups::OwnerGroupKind::Sequence, Some(node),
            ).expect("cluster sequence vessel lookup").is_none()
            // AddClusters skips a node whose incident edge now terminates at
            // an earlier cluster vessel. Stable Rust retains the member
            // endpoint instead of replacing it with a vessel pointer, so a
            // clustered adjacent member is the equivalent ownership marker.
            && !node_ref.edges.read().iter().any(|edge_id| {
                let edge = &self.edge_records[edge_id.0 as usize];
                let adjacent = if edge.from == node && edge.to != node {
                    edge.to
                } else if edge.to == node && edge.from != node {
                    edge.from
                } else {
                    return false;
                };
                self.nodes[adjacent.0 as usize].cluster.is_some()
            })
            && node_ref.shape != ShapeKind::SqlTable
            && !node_ref
                .edges
                .read().iter()
                .any(|edge_id| self.edge_records[edge_id.0 as usize].has_table_column())
            && node_ref.hierarchy.is_none()
            && node_ref.sequence.is_none()
            && node_ref.fixed_top_left.is_none()
            && !self.has_leaky_edge(node)
            && !node_ref.edges.read().iter().any(|edge_id| {
                let edge = &self.edge_records[edge_id.0 as usize];
                edge.from == edge.to
            })
    }

    fn cluster_equivalent(
        &self,
        first: NodeId,
        second: NodeId,
        neighbors: &[BTreeSet<NodeId>],
        signatures: &[EdgeSignature],
    ) -> bool {
        let first_node = &self.nodes[first.0 as usize];
        let second_node = &self.nodes[second.0 as usize];
        if first_node.shape != second_node.shape {
            return false;
        }
        let first_size = first_node.rect.size;
        let second_size = second_node.rect.size;
        const MAX_SIZE_DIFF: f64 = 4.0;
        if second_size.width * MAX_SIZE_DIFF < first_size.width
            || first_size.width * MAX_SIZE_DIFF < second_size.width
            || second_size.height * MAX_SIZE_DIFF < first_size.height
            || first_size.height * MAX_SIZE_DIFF < second_size.height
        {
            return false;
        }
        !neighbors[first.0 as usize].is_empty()
            && neighbors[first.0 as usize] == neighbors[second.0 as usize]
            && signatures[first.0 as usize].matches(&signatures[second.0 as usize])
    }

    /// Membership portion of recovered `Graph.AddClusters`.
    ///
    /// The pristine stage groups eligible same-scope nodes when their shape,
    /// estimated dimensions, unique-neighbor set, and edge-direction/
    /// arrowhead inventories agree. Arrangement and vessel geometry are
    /// separate consumers; routing needs the resulting membership carrier.
    pub(super) fn assign_clusters(&mut self, seed: i64, fixed_sizes: bool) {
        let mut misc_rng = go_rng::GoRng::new(seed);
        self.assign_clusters_with_rng(seed, false, fixed_sizes, &mut misc_rng);
    }

    pub(super) fn assign_clusters_with_rng(
        &mut self,
        seed: i64,
        consider_positions: bool,
        fixed_sizes: bool,
        misc_rng: &mut go_rng::GoRng,
    ) {
        if crate::internal::trace_env_enabled("WEFTAN_DEBUG_CLUSTERS") {
            eprintln!("WEFTAN_CLUSTER_ENTER nodes={}", self.nodes.len());
        }
        self.begin_owner_group_discovery(super::owner_groups::OwnerGroupKind::Cluster);
        self.clusters.clear();
        self.cluster_vessel_sizes.clear();
        for node in &mut self.nodes {
            node.cluster = None;
        }

        let tree_nodes = routing::routing_tree_nodes(self);
        // Recovered AddClusters checks membership in Graph.Trees, whose keys
        // are the terminal sentinels. Graph.NodeToTree is a different carrier:
        // its member nodes remain eligible to cluster. Collapsing both maps
        // into one exclusion incorrectly suppresses clusters made from sibling
        // tree roots.
        let tree_sentinels = self
            .tree_routing_nodes
            .values()
            .filter_map(|tree| (!tree_nodes.contains(&tree.parent)).then_some(tree.parent))
            .collect::<BTreeSet<_>>();
        let neighbors = self
            .nodes
            .iter()
            .map(|node| self.unique_neighbors(node.input_id))
            .collect::<Vec<_>>();
        let ordered_neighbors = self
            .nodes
            .iter()
            .map(|node| self.unique_neighbors_in_edge_order(node.input_id))
            .collect::<Vec<_>>();
        let signatures = self
            .nodes
            .iter()
            .map(|node| self.edge_signature(node.input_id))
            .collect::<Vec<_>>();
        let mut scopes = self
            .container_reverse_dfs_order()
            .into_iter()
            .map(Some)
            .collect::<Vec<_>>();
        scopes.push(None);

        for scope in scopes {
            // Graph.AddClusters constructs a fresh math/rand.Rand with the
            // layout seed for each container in reverse DFS order.
            let mut rng = go_rng::GoRng::with_stream(seed, "cluster-scope");
            let children = self
                .preprocessed_tree_children
                .get(&scope)
                .cloned()
                .or_else(|| self.containers.get(&scope))
                .unwrap_or_default();
            for (seed_index, seed) in children.iter().copied().enumerate() {
                if self.nodes[seed.0 as usize].cluster.is_some()
                    || !self.cluster_eligible(seed, &tree_sentinels)
                {
                    continue;
                }
                let mut members = vec![seed];
                let mut is_connected_to_sequence = false;
                for candidate in children.iter().copied().skip(seed_index + 1) {
                    if self.nodes[candidate.0 as usize].cluster.is_some()
                        || !self.cluster_eligible(candidate, &tree_sentinels)
                    {
                        continue;
                    }

                    // Recovered AddClusters updates this accumulator while it
                    // compares the seed's ordered unique-neighbor slice. It is
                    // deliberately not recomputed from the final members:
                    // an earlier equal-length candidate can set it before
                    // failing a later neighbor comparison.
                    if !ordered_neighbors[seed.0 as usize].is_empty()
                        && ordered_neighbors[seed.0 as usize].len()
                            == ordered_neighbors[candidate.0 as usize].len()
                    {
                        for adjacent in &ordered_neighbors[seed.0 as usize] {
                            is_connected_to_sequence |=
                                self.nodes[adjacent.0 as usize].sequence.is_some();
                            if !neighbors[candidate.0 as usize].contains(adjacent) {
                                break;
                            }
                        }
                    }

                    if self.cluster_equivalent(seed, candidate, &neighbors, &signatures) {
                        members.push(candidate);
                    }
                }
                if members.len() < 2 {
                    continue;
                }
                let Some(arrangement) = self.initial_cluster_arrangement(
                    &members,
                    consider_positions,
                    is_connected_to_sequence,
                    &mut rng,
                ) else {
                    continue;
                };
                // CreateVessel orders an already-positioned cluster before it
                // is collapsed: rows follow x and columns follow y. Keep the
                // same order when a direct graph fixture supplies serialized
                // positions, while ordinary source graphs retain discovery
                // order because they have no preprocessing hint.
                if members
                    .iter()
                    .all(|member| self.nodes[member.0 as usize].initial_position.is_some())
                {
                    members.sort_by(|left, right| {
                        let left = self.nodes[left.0 as usize].initial_position.unwrap();
                        let right = self.nodes[right.0 as usize].initial_position.unwrap();
                        let ordering = match arrangement {
                            ClusterArrangement::Row => left.x.total_cmp(&right.x),
                            ClusterArrangement::Column => left.y.total_cmp(&right.y),
                        };
                        ordering.then_with(|| left.x.total_cmp(&right.x))
                    });
                }
                let padding = self.initial_cluster_padding(&members, arrangement);
                // The successful AddClusters closure consumes rand.Int for the
                // vessel ID after AssignArrangement, even though ArenaGraph
                // retains the original flat nodes instead of materializing
                // that temporary vessel.
                let vessel_tala_id = misc_rng.int63();
                let cluster = self.clusters.len();
                if crate::internal::trace_env_enabled("WEFTAN_TRACE_CLUSTER_CREATE") {
                    eprintln!(
                        "CLUSTER_CREATE_RUST before vessel={} members={:?}",
                        vessel_tala_id,
                        members
                            .iter()
                            .map(|member| {
                                let node = &self.nodes[member.0 as usize];
                                (node.tala_id, node.position, node.rect.size)
                            })
                            .collect::<Vec<_>>()
                    );
                }
                let vessel_position = members
                    .iter()
                    .filter_map(|member| self.nodes[member.0 as usize].position)
                    .fold(None, |minimum: Option<Point>, position| {
                        Some(match minimum {
                            Some(minimum) => Point {
                                x: minimum.x.min(position.x),
                                y: minimum.y.min(position.y),
                            },
                            None => position,
                        })
                    });
                for member in &members {
                    self.nodes[member.0 as usize].cluster = Some(cluster);
                }
                self.clusters.push(ClusterState {
                    fixed_size: fixed_sizes
                        || members.iter().any(|member| {
                            self.nodes[member.0 as usize].shape.has_unit_aspect_ratio()
                        }),
                    members,
                    arrangement,
                    desired_arrangement: arrangement,
                    padding,
                    vessel_tala_id,
                });
                // TALA's successful AddClusters paths call Cluster.Resize
                // before the vessel is installed.  CreateVessel only sorts
                // already-positioned members and leaves their coordinates in
                // place; ArrangeClusterNodes is deferred until Cleanup after
                // the vessel has been laid out.  Keeping that lifecycle is
                // essential for prearranged GraphJSON witnesses: moving the
                // members here changes the geometry used by later cluster
                // discovery and placement.
                self.resize_cluster_members(cluster);
                let vessel_size = self.cluster_vessel_size_for_arrangement(
                    cluster,
                    self.clusters[cluster].arrangement,
                );
                #[cfg(feature = "diagnostic-traces")]
                crate::internal::pipeline::trace_record_cluster_initial_size(
                    vessel_tala_id,
                    vessel_size,
                );
                if self.cluster_vessel_sizes.len() <= cluster {
                    self.cluster_vessel_sizes
                        .resize(cluster + 1, Size::default());
                }
                self.cluster_vessel_sizes[cluster] = vessel_size;
                if let Some(position) = vessel_position {
                    self.pending_cluster_vessel_positions
                        .insert(cluster, position);
                }
                if crate::internal::trace_env_enabled("WEFTAN_TRACE_CLUSTER_CREATE") {
                    eprintln!(
                        "CLUSTER_CREATE_RUST after vessel={} pending={:?} members={:?}",
                        vessel_tala_id,
                        self.pending_cluster_vessel_positions.get(&cluster),
                        self.clusters[cluster]
                            .members
                            .iter()
                            .map(|member| {
                                let node = &self.nodes[member.0 as usize];
                                (node.tala_id, node.position, node.rect.size)
                            })
                            .collect::<Vec<_>>()
                    );
                }
            }
        }
        self.cluster_backlinks_complete = true;
        self.rebuild_active_aggregate_node_order();
        for index in 0..self.clusters.len() {
            self.publish_cluster_creation(index)
                .expect("discovered cluster owner publication");
        }
        self.synchronize_original_owner_graph_view();
    }

    /// Apply recovered `Cluster.Resize` member sizing without materializing
    /// TALA's temporary vessel node. Non-fixed clusters give every member the
    /// maximum member width and height; the vessel-only aggregate dimension
    /// is irrelevant to the stable-ID arena.
    pub(super) fn resize_cluster_members(&mut self, cluster_index: usize) {
        let cluster = &self.clusters[cluster_index];
        if cluster.members.is_empty() {
            return;
        }
        if !cluster.fixed_size {
            let members = cluster.members.clone();
            let maximum = members.iter().fold(Size::default(), |maximum, member| {
                let size = self.nodes[member.0 as usize].rect.size;
                Size {
                    width: maximum.width.max(size.width),
                    height: maximum.height.max(size.height),
                }
            });
            for member in members {
                self.nodes[member.0 as usize].rect.size = maximum;
            }
        }
    }

    pub(super) fn cluster_vessel_size_for_arrangement(
        &self,
        cluster_index: usize,
        arrangement: ClusterArrangement,
    ) -> Size {
        self.cluster_vessel_size_for_arrangement_with_padding(
            cluster_index,
            arrangement,
            self.clusters[cluster_index].padding,
        )
    }

    pub(super) fn cluster_vessel_size_for_arrangement_with_padding(
        &self,
        cluster_index: usize,
        arrangement: ClusterArrangement,
        padding: f64,
    ) -> Size {
        let cluster = &self.clusters[cluster_index];
        let count = cluster.members.len() as f64;
        let maximum = cluster
            .members
            .iter()
            .fold(Size::default(), |maximum, member| {
                let size = self.nodes[member.0 as usize].rect.size;
                Size {
                    width: maximum.width.max(size.width),
                    height: maximum.height.max(size.height),
                }
            });
        let gaps = padding * (count - 1.0).max(0.0);
        match arrangement {
            ClusterArrangement::Row => Size {
                width: maximum.width * count + gaps,
                height: maximum.height,
            },
            ClusterArrangement::Column => Size {
                width: maximum.width,
                height: maximum.height * count + gaps,
            },
        }
    }

    pub(super) fn cluster_vessel_size(&self, cluster_index: usize) -> Size {
        if let Some(size) = self.cluster_vessel_sizes.get(cluster_index)
            && *size != Size::default()
        {
            if crate::internal::trace_env_enabled("WEFTAN_TRACE_CLUSTER_SIZE_CACHE") {
                eprintln!(
                    "CLUSTER_SIZE_CACHE_RUST vessel={} cached={}x{} current={}x{}",
                    self.clusters[cluster_index].vessel_tala_id,
                    size.width,
                    size.height,
                    self.cluster_vessel_size_for_arrangement(
                        cluster_index,
                        self.clusters[cluster_index].arrangement,
                    )
                    .width,
                    self.cluster_vessel_size_for_arrangement(
                        cluster_index,
                        self.clusters[cluster_index].arrangement,
                    )
                    .height,
                );
            }
            return *size;
        }
        self.cluster_vessel_size_for_arrangement(
            cluster_index,
            self.clusters[cluster_index].arrangement,
        )
    }

    /// Publish the vessel dimensions after recursive placement has expanded
    /// nested members. During scope fitting the cached dimensions deliberately
    /// describe the pre-expansion temporary vessel so the owner can choose its
    /// bounds without counting hidden descendants twice. Once the complete
    /// placement scope is materialized, later optimization stages must see the
    /// expanded vessel dimensions that TALA's shared pointers retain.
    pub(super) fn refresh_cluster_vessel_sizes(&mut self) {
        self.cluster_vessel_sizes = self
            .clusters
            .iter()
            .enumerate()
            .map(|(index, cluster)| {
                self.cluster_vessel_size_for_arrangement(index, cluster.arrangement)
            })
            .collect();
    }

    /// Member geometry inside TALA's temporary cluster vessel immediately
    /// after `Cluster.Resize` and `Cluster.ArrangeClusterNodes`.
    pub(super) fn cluster_member_geometry_for_arrangement(
        &self,
        cluster_index: usize,
        member: NodeId,
        arrangement: ClusterArrangement,
    ) -> Option<(Point, Size)> {
        self.cluster_member_geometry_for_arrangement_with_padding(
            cluster_index,
            member,
            arrangement,
            self.clusters[cluster_index].padding,
        )
    }

    pub(super) fn cluster_member_geometry_for_arrangement_with_padding(
        &self,
        cluster_index: usize,
        member: NodeId,
        arrangement: ClusterArrangement,
        padding: f64,
    ) -> Option<(Point, Size)> {
        let cluster = &self.clusters[cluster_index];
        let vessel = if arrangement == cluster.arrangement && padding == cluster.padding {
            self.cluster_vessel_size(cluster_index)
        } else {
            self.cluster_vessel_size_for_arrangement_with_padding(
                cluster_index,
                arrangement,
                padding,
            )
        };
        let resized_member = (!cluster.fixed_size).then(|| {
            cluster
                .members
                .iter()
                .fold(Size::default(), |maximum, member| {
                    let size = self.nodes[member.0 as usize].rect.size;
                    Size {
                        width: maximum.width.max(size.width),
                        height: maximum.height.max(size.height),
                    }
                })
        });
        if crate::internal::trace_env_enabled("WEFTAN_TRACE_CLUSTER_GEOMETRY")
            && cluster.vessel_tala_id == 6129484611666145821
        {
            eprintln!(
                "CLUSTER_GEOMETRY_RUST vessel={} cluster_arr={:?} requested={:?} padding={} cluster_padding={} vessel={:?} resized={:?} member_sizes={:?}",
                cluster.vessel_tala_id,
                cluster.arrangement,
                arrangement,
                padding,
                cluster.padding,
                vessel,
                resized_member,
                cluster
                    .members
                    .iter()
                    .map(|member| (
                        self.nodes[member.0 as usize].tala_id,
                        self.nodes[member.0 as usize].rect.size
                    ))
                    .collect::<Vec<_>>()
            );
        }
        let mut offset = Point::default();
        for candidate in cluster.members.iter().copied() {
            // `Cluster.Resize` gives every member of a non-fixed cluster the
            // maximum member dimensions before `ArrangeClusterNodes` lays the
            // members out inside the temporary vessel.
            let size = resized_member.unwrap_or(self.nodes[candidate.0 as usize].rect.size);
            match arrangement {
                ClusterArrangement::Row => {
                    offset.y = (vessel.height / 2.0 - size.height / 2.0).round();
                    if candidate == member {
                        return Some((offset, size));
                    }
                    offset.x += size.width + padding;
                }
                ClusterArrangement::Column => {
                    offset.x = (vessel.width / 2.0 - size.width / 2.0).round();
                    if candidate == member {
                        return Some((offset, size));
                    }
                    offset.y += size.height + padding;
                }
            }
        }
        None
    }

    pub(super) fn cluster_member_geometry(
        &self,
        cluster_index: usize,
        member: NodeId,
    ) -> Option<(Point, Size)> {
        self.cluster_member_geometry_for_arrangement(
            cluster_index,
            member,
            self.clusters[cluster_index].arrangement,
        )
    }

    pub(super) fn projected_cluster_desired_arrangement(
        &self,
        node: NodeId,
        current: ClusterArrangement,
        requested: ClusterArrangement,
    ) -> ClusterArrangement {
        let Some(position) = self.position(node) else {
            return current;
        };
        let vessel = Rect {
            origin: position,
            size: self.nodes[node.0 as usize].rect.size,
        };
        // Go's Cluster.getExternalConnectedNodes only returns endpoints whose
        // `Graph` pointer is this temporary placement graph.  The stable
        // arena keeps shared aliases in transaction metadata, so an alias
        // from the owning graph can otherwise look like a local endpoint and
        // cause a projected child cluster to flip before its parent scope.
        // Restrict every metadata fallback to IDs actually present in this
        // graph's Nodes slice; those are the only endpoints Go would expose
        // to Cluster.optimize for this scope.
        let local_tala_ids = self
            .nodes
            .iter()
            .map(|candidate| candidate.tala_id)
            .collect::<BTreeSet<_>>();
        let mut seen = BTreeSet::new();
        let mut seen_external_tala_ids = BTreeSet::new();
        let mut horizontal_count = 0;
        let mut vertical_count = 0;
        let mut count_external = |adjacent_rect: Rect| {
            let separated_horizontally =
                adjacent_rect.right() < vessel.origin.x || vessel.right() < adjacent_rect.origin.x;
            let separated_vertically = adjacent_rect.bottom() < vessel.origin.y
                || vessel.bottom() < adjacent_rect.origin.y;
            if separated_horizontally && !separated_vertically {
                horizontal_count += 1;
            } else if separated_vertically && !separated_horizontally {
                vertical_count += 1;
            } else if separated_horizontally && separated_vertically {
                let x_distance = (adjacent_rect.center().x - vessel.center().x).abs();
                let y_distance = (adjacent_rect.center().y - vessel.center().y).abs();
                if y_distance < x_distance {
                    horizontal_count += 1;
                } else if x_distance < y_distance {
                    vertical_count += 1;
                }
            }
        };
        for edge_id in self.nodes[node.0 as usize].edges.read().iter().copied() {
            let adjacent = self.adjacent(node, edge_id);
            if adjacent == node || !seen.insert(adjacent) {
                continue;
            }
            let adjacent_node = &self.nodes[adjacent.0 as usize];
            seen_external_tala_ids.insert(adjacent_node.tala_id);
            let Some(adjacent_position) = adjacent_node.position else {
                continue;
            };
            let adjacent_rect = Rect {
                origin: adjacent_position,
                size: adjacent_node.rect.size,
            };
            count_external(adjacent_rect);
        }
        // A temporary SplitSubgraphs graph keeps an aggregate vessel in its
        // Nodes slice while its member edges remain on the shared original
        // pointers. The projected vessel therefore has no local edges even
        // though Go's Cluster.getExternalConnectedNodes still sees those
        // endpoints when Cluster.optimize(true) runs. Reuse the retained
        // external inventory captured while building this placement scope.
        for distance in self.sized_cluster_distance_boxes.values() {
            if distance.vessel_tala_id != self.nodes[node.0 as usize].tala_id {
                continue;
            }
            if crate::internal::trace_env_value("WEFTAN_TRACE_PROJECTED_CLUSTER_EXTERNAL")
                .as_deref()
                == Some("all")
            {
                eprintln!(
                    "PROJECTED_CLUSTER_EXTERNAL_RUST vessel={} node={} distance_owner={:?} arrangement={:?} external={:?}",
                    distance.vessel_tala_id,
                    self.nodes[node.0 as usize].tala_id,
                    distance.offset,
                    distance.arrangement,
                    distance
                        .external_connected
                        .iter()
                        .map(|adjacent| (
                            adjacent.tala_id,
                            adjacent.owner,
                            adjacent.offset,
                            adjacent.size
                        ))
                        .collect::<Vec<_>>()
                );
            }
            for adjacent in &distance.external_connected {
                if !local_tala_ids.contains(&adjacent.tala_id) {
                    continue;
                }
                if !seen_external_tala_ids.insert(adjacent.tala_id) {
                    continue;
                }
                let Some(owner_position) = self.position(adjacent.owner) else {
                    continue;
                };
                count_external(Rect {
                    origin: Point {
                        x: owner_position.x + adjacent.offset.x,
                        y: owner_position.y + adjacent.offset.y,
                    },
                    size: adjacent.size,
                });
            }
        }
        // Some direct aggregate edges disappear from the temporary graph
        // entirely when both endpoints were descendants of the same scope.
        // The original Go pointers still retain those edges on each cluster
        // member, so Cluster.optimize can classify the external node. Keep
        // the complete adapter-time endpoint table on every induced graph and
        // replay that pointer-visible inventory here as a final fallback.
        if let Some(cluster_members) = self
            .transaction_external_aggregate_children
            .get(&self.nodes[node.0 as usize].tala_id)
        {
            let member_tala_ids = cluster_members
                .iter()
                .map(|member| member.tala_id)
                .collect::<BTreeSet<_>>();
            for (from_tala_id, to_tala_id) in self.source_edge_endpoints_by_id.values() {
                let from_in = member_tala_ids.contains(from_tala_id);
                let to_in = member_tala_ids.contains(to_tala_id);
                if from_in == to_in {
                    continue;
                }
                let external_tala_id = if from_in { *to_tala_id } else { *from_tala_id };
                if !local_tala_ids.contains(&external_tala_id) {
                    continue;
                }
                if !seen_external_tala_ids.insert(external_tala_id) {
                    continue;
                }
                let local = self
                    .nodes
                    .iter()
                    .enumerate()
                    .find(|(_, candidate)| candidate.tala_id == external_tala_id)
                    .map(|(index, _)| NodeId(index as u32));
                if let Some(local) = local {
                    let Some(external_position) = self.position(local) else {
                        continue;
                    };
                    count_external(Rect {
                        origin: external_position,
                        size: self.nodes[local.0 as usize].rect.size,
                    });
                    continue;
                }
                let Some(alias) = self
                    .nodes
                    .iter()
                    .find(|candidate| candidate.tala_id == external_tala_id)
                else {
                    continue;
                };
                let Some(external_position) = alias.position else {
                    continue;
                };
                if crate::internal::trace_env_value("WEFTAN_TRACE_PROJECTED_CLUSTER_EXTERNAL")
                    .as_deref()
                    == Some("all")
                {
                    eprintln!(
                        "PROJECTED_CLUSTER_ALIAS_RUST vessel={} external={} pos={:?} size={:?}",
                        self.nodes[node.0 as usize].tala_id,
                        external_tala_id,
                        external_position,
                        alias.rect.size
                    );
                }
                count_external(Rect {
                    origin: external_position,
                    size: alias.rect.size,
                });
            }
        }
        if crate::internal::trace_env_value("WEFTAN_TRACE_PROJECTED_CLUSTER_EXTERNAL").as_deref()
            == Some("all")
            && self.nodes[node.0 as usize].tala_id == 4_037_200_794_235_010_051
        {
            eprintln!(
                "PROJECTED_CLUSTER_EXTERNAL_STATE_RUST node={} aggregate_keys={:?} aliases={:?} source_edges={:?}",
                self.nodes[node.0 as usize].tala_id,
                self.transaction_external_aggregate_children
                    .keys()
                    .collect::<Vec<_>>(),
                self.transaction_external_containers
                    .iter()
                    .chain(
                        self.transaction_external_container_children
                            .values()
                            .flatten()
                    )
                    .chain(
                        self.transaction_external_aggregate_children
                            .values()
                            .flatten()
                    )
                    .chain(
                        self.projected_transaction_nodes
                            .iter()
                            .map(|entry| &entry.node)
                    )
                    .map(|node| node.tala_id)
                    .collect::<Vec<_>>(),
                self.source_edge_endpoints_by_id
                    .values()
                    .collect::<Vec<_>>()
            );
        }
        if vertical_count > horizontal_count {
            ClusterArrangement::Row
        } else if vertical_count < horizontal_count {
            ClusterArrangement::Column
        } else {
            // Cluster.optimize starts from DesiredArrangement, rather than
            // the currently applied Arrangement. This matters for a
            // temporary vessel whose shared endpoint inventory has no
            // positioned external nodes yet.
            requested
        }
    }

    pub(super) fn apply_projected_cluster_layout(
        &mut self,
        node: NodeId,
        arrangement: ClusterArrangement,
        layout: &ProjectedClusterLayout,
    ) {
        self.nodes[node.0 as usize].rect.size = layout.size;
        self.nodes[node.0 as usize].scoring_cluster_arrangement = Some(arrangement);
        let vessel_tala_id = self.nodes[node.0 as usize].tala_id;
        // The projected vessel and the retained external layout describe the
        // same `*Cluster`. Recovered Cluster.optimize mutates c.Arrangement
        // before c.sync and Transaction.Commit later syncs that same pointer.
        // Publish the mutation here so a commit-phase sync cannot restore the
        // pre-trial arrangement and dimensions.
        if let Some(external_layout) = self
            .transaction_external_cluster_layouts
            .get_mut(&vessel_tala_id)
        {
            external_layout.arrangement = arrangement;
            external_layout.padding = layout.padding;
        }
        for projected in self.sized_adjacent_overrides.values_mut() {
            if projected.owner == node
                && projected.cluster_member
                && let Some(&(offset, size)) = layout.members.get(&projected.tala_id)
            {
                projected.offset = offset;
                projected.size = size;
            }
        }
        for obstructions in self.sized_projected_obstructions.values_mut() {
            for obstruction in obstructions {
                if obstruction.owner == node && obstruction.tala_id == vessel_tala_id {
                    obstruction.size = layout.size;
                }
            }
        }
        for inventory in self.sized_projected_obstruction_sets.values_mut() {
            for obstruction in inventory.projections_mut() {
                if obstruction.owner == node && obstruction.tala_id == vessel_tala_id {
                    obstruction.size = layout.size;
                }
            }
        }
    }

    fn projected_cluster_flip_trial(
        &self,
        node: NodeId,
        arrangement: ClusterArrangement,
        layout: &ProjectedClusterLayout,
        try_center: bool,
    ) -> Option<Self> {
        let prior_transaction_state = self.projected_transaction_state();
        let graph_node_tala_ids = prior_transaction_state
            .nodes
            .iter()
            .map(|entry| entry.node.tala_id)
            .collect::<BTreeSet<_>>();
        let prior_external_overlaps = self
            .existing_external_overlap_pairs()
            .into_iter()
            // GraphState creates pairwise exceptions only from Graph.Nodes.
            // A Containers key that is absent from that slice receives no
            // exception merely because its projected box already overlapped.
            .filter(|(container, node)| {
                graph_node_tala_ids.contains(container) && graph_node_tala_ids.contains(node)
            })
            .collect();
        let prior_size = self.nodes[node.0 as usize].rect.size;
        let prior_position = self.position(node)?;
        let mut trial = self.clone();
        trial.apply_projected_cluster_layout(node, arrangement, layout);
        // Cluster.optimize's transaction op calls c.sync immediately after
        // changing the arrangement. This moves retained member/container
        // pointers before centering and before Commit refits containers.
        trial.sync_external_cluster(self.nodes[node.0 as usize].tala_id);
        if try_center {
            let delta = match arrangement {
                ClusterArrangement::Row => Point {
                    x: ((prior_size.width - layout.size.width) / 2.0).round(),
                    y: 0.0,
                },
                ClusterArrangement::Column => Point {
                    x: 0.0,
                    y: ((prior_size.height - layout.size.height) / 2.0).round(),
                },
            };
            trial.move_node_abs_with_children(
                node,
                Point {
                    x: prior_position.x + delta.x,
                    y: prior_position.y + delta.y,
                },
            );
        }
        // Recovered Transaction.Commit validates every positioned container
        // immediately after repositionContainers, before Graph.syncClusters
        // and Graph.syncSequences. Keep that phase boundary explicit: a
        // projected cluster's trial dimensions must be visible to container
        // validation, not replaced by a later aggregate synchronization.
        trial.reposition_ordinary_containers_without_sync();
        let containment_invalid = !trial.transaction_containment_is_valid();
        let external_invalid = !trial
            .transaction_external_containers_are_valid_with_exceptions(&prior_external_overlaps);
        if containment_invalid || external_invalid {
            if crate::internal::trace_env_enabled("WEFTAN_TRACE_CLUSTER_FLIP")
                && crate::internal::trace_env_value("WEFTAN_TRACE_CLUSTER_FLIP").as_deref()
                    == Some("all")
            {
                eprintln!(
                    "CLUSTER_TRIAL_RUST vessel={} arrangement={:?} center={} prior_size={:?} next_size={:?} rejects=bad:false spacing:false containment:{} external:{}",
                    self.nodes[node.0 as usize].tala_id,
                    arrangement,
                    try_center,
                    prior_size,
                    layout.size,
                    containment_invalid,
                    external_invalid,
                );
            }
            return None;
        }
        trial.sync_clusters();
        trial.sync_sequences();
        // Transaction.Commit now walks the owning Graph.Nodes carrier in its
        // recovered order, then applies the separate padded-to-exact guard.
        // Container-map projections above are not silently promoted into
        // GraphState membership.
        let bad_state = !trial.projected_transaction_nodes_are_valid(&prior_transaction_state);
        let spacing_exact =
            trial.projected_transaction_spacing_overlap_became_exact(&prior_transaction_state);
        if crate::internal::trace_env_enabled("WEFTAN_TRACE_CLUSTER_FLIP")
            && crate::internal::trace_env_value("WEFTAN_TRACE_CLUSTER_FLIP").as_deref()
                == Some("all")
        {
            eprintln!(
                "CLUSTER_TRIAL_RUST vessel={} arrangement={:?} center={} prior_size={:?} next_size={:?} rejects=bad:{} spacing:{} containment:{} external:{}",
                self.nodes[node.0 as usize].tala_id,
                arrangement,
                try_center,
                prior_size,
                layout.size,
                bad_state,
                spacing_exact,
                containment_invalid,
                external_invalid,
            );
        }
        if bad_state || spacing_exact || containment_invalid || external_invalid {
            return None;
        }
        Some(trial)
    }

    /// Recovered `Cluster.optimize(ctx, true)` for a cluster represented by
    /// its temporary vessel in one `SplitSubgraphs` arena.
    pub(super) fn optimize_projected_cluster_flip(
        &mut self,
        node: NodeId,
        projection: &ProjectedCluster,
    ) -> Option<ClusterArrangement> {
        let desired = self.projected_cluster_desired_arrangement(
            node,
            projection.arrangement,
            projection.desired_arrangement,
        );
        if crate::internal::trace_env_enabled("WEFTAN_TRACE_CLUSTER_FLIP")
            && crate::internal::trace_env_value("WEFTAN_TRACE_CLUSTER_FLIP").as_deref()
                == Some("all")
        {
            eprintln!(
                "CLUSTER_FLIP_RUST vessel={} current={:?} desired={:?} size={:?} position={:?} edges={:?}",
                self.nodes[node.0 as usize].tala_id,
                projection.arrangement,
                desired,
                self.nodes[node.0 as usize].rect.size,
                self.position(node),
                self.nodes[node.0 as usize]
                    .edges
                    .read()
                    .iter()
                    .map(|edge_id| {
                        let edge = &self.edge_records[edge_id.0 as usize];
                        (
                            self.nodes[edge.from.0 as usize].tala_id,
                            self.nodes[edge.to.0 as usize].tala_id,
                            self.position(edge.from),
                            self.position(edge.to),
                        )
                    })
                    .collect::<Vec<_>>()
            );
        }
        if desired == projection.arrangement {
            return None;
        }
        let layout = match desired {
            ClusterArrangement::Row => &projection.row,
            ClusterArrangement::Column => &projection.column,
        };
        let committed = self
            .projected_cluster_flip_trial(node, desired, layout, true)
            .or_else(|| self.projected_cluster_flip_trial(node, desired, layout, false))?;
        *self = committed;
        Some(desired)
    }

    /// Recovered `Cluster.ArrangeClusterNodes`.
    ///
    /// Weftan retains stable input node IDs instead of inserting TALA's
    /// temporary vessel into the arena. The pre-sync cluster origin carries
    /// the vessel's top-left; `Cluster.Resize` still determines its dimensions.
    #[track_caller]
    pub(super) fn arrange_cluster_members(&mut self, cluster_index: usize, vessel_top_left: Point) {
        if crate::internal::trace_env_enabled("WEFTAN_TRACE_ARRANGE_CALLS") {
            let cluster = &self.clusters[cluster_index];
            eprintln!(
                "ARRANGE_CALL_RUST caller={} vessel={} arrangement={:?} vessel_pos={:?} members={:?}",
                std::panic::Location::caller(),
                cluster.vessel_tala_id,
                cluster.arrangement,
                vessel_top_left,
                cluster
                    .members
                    .iter()
                    .map(|member| self.nodes[member.0 as usize].tala_id)
                    .collect::<Vec<_>>()
            );
        }
        self.arrange_cluster_members_impl(cluster_index, vessel_top_left, false);
    }

    /// Arrange a live cluster through the owning Graph boundary.  Direct
    /// transactions operate on the same source graph as TALA's
    /// `Graph.SyncClusters`, so existing container members must translate
    /// their source descendants along with the member pointer.  Induced
    /// placement scopes continue to use the flattened-safe path above.
    pub(super) fn arrange_cluster_members_with_descendants(
        &mut self,
        cluster_index: usize,
        vessel_top_left: Point,
    ) {
        self.arrange_cluster_members_impl(cluster_index, vessel_top_left, true);
    }

    /// Arrange members while retiring a temporary cluster vessel.
    pub(super) fn arrange_cluster_members_cleanup(
        &mut self,
        cluster_index: usize,
        vessel_top_left: Point,
    ) {
        self.arrange_cluster_members_impl(cluster_index, vessel_top_left, false);
    }

    fn arrange_cluster_members_impl(
        &mut self,
        cluster_index: usize,
        vessel_top_left: Point,
        move_descendants: bool,
    ) {
        let cluster = self.clusters[cluster_index].clone();
        let vessel = self.cluster_vessel_size(cluster_index);
        let trace_arrange = crate::internal::trace_env_enabled("WEFTAN_TRACE_CLUSTER_ARRANGE")
            && crate::internal::trace_env_value("WEFTAN_TRACE_CLUSTER_ARRANGE_VESSEL").is_none_or(
                |target| {
                    target == "all"
                        || target
                            .parse::<i64>()
                            .is_ok_and(|vessel_tala_id| vessel_tala_id == cluster.vessel_tala_id)
                },
            );
        if trace_arrange {
            eprintln!(
                "CLUSTER_ARRANGE_RUST vessel={} arrangement={:?} pos={:?} size={},{} members={:?}",
                cluster.vessel_tala_id,
                cluster.arrangement,
                vessel_top_left,
                vessel.width,
                vessel.height,
                cluster
                    .members
                    .iter()
                    .map(|member| {
                        let node = &self.nodes[member.0 as usize];
                        (node.tala_id, self.position(*member), node.rect.size)
                    })
                    .collect::<Vec<_>>()
            );
        }
        match cluster.arrangement {
            ClusterArrangement::Row => {
                let center = vessel_top_left.y + vessel.height / 2.0;
                let mut position = vessel_top_left.x;
                for member in cluster.members.iter().copied() {
                    let size = self.nodes[member.0 as usize].rect.size;
                    if crate::internal::trace_env_enabled("WEFTAN_TRACE_GAP_PAIR")
                        && cluster.vessel_tala_id == 6129484611666145821
                    {
                        eprintln!(
                            "GAP_CLUSTER_MEMBER_RUST member={} current={:?} size={:?}",
                            self.nodes[member.0 as usize].tala_id,
                            self.position(member),
                            size,
                        );
                    }
                    if let Some(current) = self.position(member) {
                        if trace_arrange {
                            eprintln!(
                                "CLUSTER_ARRANGE_DETAIL_RUST member={} descendants={:?} external_children={:?} external_containers={:?}",
                                self.nodes[member.0 as usize].tala_id,
                                self.active_descendants(member)
                                    .iter()
                                    .map(|node| self.nodes[node.0 as usize].tala_id)
                                    .collect::<Vec<_>>(),
                                self.transaction_external_container_children
                                    .iter()
                                    .filter(|(_, children)| children
                                        .iter()
                                        .any(|child| child.tala_id
                                            == self.nodes[member.0 as usize].tala_id))
                                    .map(|(parent, children)| (
                                        *parent,
                                        children
                                            .iter()
                                            .map(|child| child.tala_id)
                                            .collect::<Vec<_>>()
                                    ))
                                    .collect::<Vec<_>>(),
                                self.transaction_external_containers
                                    .iter()
                                    .filter(|container| container.tala_id
                                        == self.nodes[member.0 as usize].tala_id)
                                    .map(|container| container.tala_id)
                                    .collect::<Vec<_>>()
                            );
                            if crate::internal::trace_env_enabled(
                                "WEFTAN_TRACE_CLUSTER_ARRANGE_DETAIL",
                            ) {
                                eprintln!(
                                    "CLUSTER_ARRANGE_DETAIL2_RUST member={} is_container={} scoring={} active_cluster={:?} parent={:?} child_meta={:?}",
                                    self.nodes[member.0 as usize].tala_id,
                                    self.nodes[member.0 as usize].is_container,
                                    self.nodes[member.0 as usize].scoring_is_container,
                                    self.active_cluster_index(member),
                                    self.source_parent_tala_by_tala
                                        .get(&self.nodes[member.0 as usize].tala_id),
                                    self.active_descendants(member)
                                        .iter()
                                        .map(|child| {
                                            let n = &self.nodes[child.0 as usize];
                                            (
                                                n.tala_id,
                                                n.is_container,
                                                n.scoring_is_container,
                                                self.active_cluster_index(*child),
                                                self.source_parent_tala_by_tala
                                                    .get(&n.tala_id)
                                                    .copied(),
                                            )
                                        })
                                        .collect::<Vec<_>>()
                                );
                            }
                        }
                        // Go's ArrangeClusterNodes moves an existing member by
                        // a rounded delta. Match Go's `math.Round(vesselCenter -
                        // (node.TopLeft.Y + node.Height/2))` before moving an
                        // existing member. Keeping this subtraction grouped
                        // in the same order matters at negative half-pixel
                        // ties (`-0.5` rounds to `-1` in Go).
                        let delta_y = (center - (current.y + size.height / 2.0)).round();
                        if crate::internal::trace_env_enabled("WEFTAN_TRACE_GAP_PAIR")
                            && cluster.vessel_tala_id == 6129484611666145821
                        {
                            eprintln!(
                                "GAP_CLUSTER_MOVE_RUST member={} current={:?} delta={:?} is_container={} scoring={} cluster={:?} active_desc={:?}",
                                self.nodes[member.0 as usize].tala_id,
                                current,
                                Point {
                                    x: position - current.x,
                                    y: delta_y
                                },
                                self.nodes[member.0 as usize].is_container,
                                self.nodes[member.0 as usize].scoring_is_container,
                                self.nodes[member.0 as usize].cluster,
                                self.active_descendants(member)
                                    .iter()
                                    .map(|node| self.nodes[node.0 as usize].tala_id)
                                    .collect::<Vec<_>>(),
                            );
                        }
                        let delta = Point {
                            x: position - current.x,
                            y: delta_y,
                        };
                        if crate::internal::trace_env_enabled("WEFTAN_TRACE_CLUSTER_STRATEGY") {
                            eprintln!(
                                "CLUSTER_STRATEGY_RUST member={} desc_container={} cluster={:?} scoring={}",
                                self.nodes[member.0 as usize].tala_id,
                                self.active_descendants(member)
                                    .iter()
                                    .any(|child| self.nodes[child.0 as usize].is_container),
                                self.nodes[member.0 as usize].cluster,
                                self.nodes[member.0 as usize].scoring_is_container
                            );
                        }
                        if (move_descendants && self.nodes[member.0 as usize].is_container)
                            || (self.nodes[member.0 as usize].cluster.is_some()
                                && self.nodes[member.0 as usize].scoring_is_container
                                && self
                                    .active_descendants(member)
                                    .iter()
                                    .all(|child| !self.nodes[child.0 as usize].is_container))
                        {
                            self.translate_node_with_children(member, delta);
                        } else {
                            self.translate_node_only(member, delta);
                        }
                    } else {
                        // Cluster.ArrangeClusterNodes assigns a previously
                        // nil member directly, then positions that member's
                        // retained container children in its new frame.
                        self.set_position(
                            member,
                            Point {
                                x: position,
                                y: (center - size.height / 2.0).round(),
                            },
                        );
                        self.position_container_children(member, false);
                    }
                    if trace_arrange {
                        eprintln!(
                            "CLUSTER_ARRANGE_RUST_AFTER_MEMBER vessel={} member={} pos={:?}",
                            cluster.vessel_tala_id,
                            self.nodes[member.0 as usize].tala_id,
                            self.position(member)
                        );
                    }
                    position += size.width + cluster.padding;
                }
            }
            ClusterArrangement::Column => {
                let center = vessel_top_left.x + vessel.width / 2.0;
                let mut position = vessel_top_left.y;
                for member in cluster.members.iter().copied() {
                    let size = self.nodes[member.0 as usize].rect.size;
                    if crate::internal::trace_env_enabled("WEFTAN_TRACE_GAP_PAIR")
                        && cluster.vessel_tala_id == 6129484611666145821
                    {
                        eprintln!(
                            "GAP_CLUSTER_MEMBER_RUST member={} current={:?} size={:?}",
                            self.nodes[member.0 as usize].tala_id,
                            self.position(member),
                            size,
                        );
                    }
                    if let Some(current) = self.position(member) {
                        if trace_arrange {
                            eprintln!(
                                "CLUSTER_ARRANGE_DETAIL_RUST member={} descendants={:?} external_children={:?} external_containers={:?}",
                                self.nodes[member.0 as usize].tala_id,
                                self.active_descendants(member)
                                    .iter()
                                    .map(|node| self.nodes[node.0 as usize].tala_id)
                                    .collect::<Vec<_>>(),
                                self.transaction_external_container_children
                                    .iter()
                                    .filter(|(_, children)| children
                                        .iter()
                                        .any(|child| child.tala_id
                                            == self.nodes[member.0 as usize].tala_id))
                                    .map(|(parent, children)| (
                                        *parent,
                                        children
                                            .iter()
                                            .map(|child| child.tala_id)
                                            .collect::<Vec<_>>()
                                    ))
                                    .collect::<Vec<_>>(),
                                self.transaction_external_containers
                                    .iter()
                                    .filter(|container| container.tala_id
                                        == self.nodes[member.0 as usize].tala_id)
                                    .map(|container| container.tala_id)
                                    .collect::<Vec<_>>()
                            );
                            if crate::internal::trace_env_enabled(
                                "WEFTAN_TRACE_CLUSTER_ARRANGE_DETAIL",
                            ) {
                                eprintln!(
                                    "CLUSTER_ARRANGE_DETAIL2_RUST member={} is_container={} scoring={} active_cluster={:?} parent={:?} child_meta={:?}",
                                    self.nodes[member.0 as usize].tala_id,
                                    self.nodes[member.0 as usize].is_container,
                                    self.nodes[member.0 as usize].scoring_is_container,
                                    self.active_cluster_index(member),
                                    self.source_parent_tala_by_tala
                                        .get(&self.nodes[member.0 as usize].tala_id),
                                    self.active_descendants(member)
                                        .iter()
                                        .map(|child| {
                                            let n = &self.nodes[child.0 as usize];
                                            (
                                                n.tala_id,
                                                n.is_container,
                                                n.scoring_is_container,
                                                self.active_cluster_index(*child),
                                                self.source_parent_tala_by_tala
                                                    .get(&n.tala_id)
                                                    .copied(),
                                            )
                                        })
                                        .collect::<Vec<_>>()
                                );
                            }
                        }
                        let delta_x = (center - (current.x + size.width / 2.0)).round();
                        if crate::internal::trace_env_enabled("WEFTAN_TRACE_CLEANUP_CLUSTER") {
                            eprintln!(
                                "CLEANUP_CLUSTER_RUST member={} current={} ideal={} raw={} delta={}",
                                self.nodes[member.0 as usize].tala_id,
                                current.x,
                                center - size.width / 2.0,
                                center - (current.x + size.width / 2.0),
                                delta_x,
                            );
                        }
                        if crate::internal::trace_env_enabled("WEFTAN_TRACE_GAP_PAIR")
                            && cluster.vessel_tala_id == 6129484611666145821
                        {
                            eprintln!(
                                "GAP_CLUSTER_MOVE_RUST member={} current={:?} delta={:?} is_container={} scoring={} cluster={:?} active_desc={:?}",
                                self.nodes[member.0 as usize].tala_id,
                                current,
                                Point {
                                    x: delta_x,
                                    y: position - current.y
                                },
                                self.nodes[member.0 as usize].is_container,
                                self.nodes[member.0 as usize].scoring_is_container,
                                self.nodes[member.0 as usize].cluster,
                                self.active_descendants(member)
                                    .iter()
                                    .map(|node| self.nodes[node.0 as usize].tala_id)
                                    .collect::<Vec<_>>(),
                            );
                        }
                        let delta = Point {
                            x: delta_x,
                            y: position - current.y,
                        };
                        if crate::internal::trace_env_enabled("WEFTAN_TRACE_CLUSTER_STRATEGY") {
                            eprintln!(
                                "CLUSTER_STRATEGY_RUST member={} desc_container={} cluster={:?} scoring={}",
                                self.nodes[member.0 as usize].tala_id,
                                self.active_descendants(member)
                                    .iter()
                                    .any(|child| self.nodes[child.0 as usize].is_container),
                                self.nodes[member.0 as usize].cluster,
                                self.nodes[member.0 as usize].scoring_is_container
                            );
                        }
                        if (move_descendants && self.nodes[member.0 as usize].is_container)
                            || (self.nodes[member.0 as usize].cluster.is_some()
                                && self.nodes[member.0 as usize].scoring_is_container
                                && self
                                    .active_descendants(member)
                                    .iter()
                                    .all(|child| !self.nodes[child.0 as usize].is_container))
                        {
                            self.translate_node_with_children(member, delta);
                        } else {
                            self.translate_node_only(member, delta);
                        }
                    } else {
                        self.set_position(
                            member,
                            Point {
                                x: (center - size.width / 2.0).round(),
                                y: position,
                            },
                        );
                        self.position_container_children(member, false);
                    }
                    if trace_arrange {
                        eprintln!(
                            "CLUSTER_ARRANGE_RUST_AFTER_MEMBER vessel={} member={} pos={:?}",
                            cluster.vessel_tala_id,
                            self.nodes[member.0 as usize].tala_id,
                            self.position(member)
                        );
                    }
                    position += size.height + cluster.padding;
                }
            }
        }
        if trace_arrange {
            eprintln!(
                "CLUSTER_ARRANGE_RUST_AFTER vessel={} arrangement={:?} pos={:?} members={:?}",
                cluster.vessel_tala_id,
                cluster.arrangement,
                vessel_top_left,
                cluster
                    .members
                    .iter()
                    .map(|member| (
                        self.nodes[member.0 as usize].tala_id,
                        self.position(*member)
                    ))
                    .collect::<Vec<_>>()
            );
        }
    }

    /// `Graph.placeNodes` resizes a root's cluster immediately after fitting
    /// that root. A later sibling fit can enlarge the same cluster again.
    pub(super) fn resize_clusters_containing(&mut self, node: NodeId) {
        if let Some(cluster_index) = self.nodes[node.0 as usize].cluster {
            let trace_resize = crate::internal::trace_env_enabled("WEFTAN_TRACE_NESTED_FIT_SIZE")
                && matches!(
                    self.nodes[node.0 as usize].tala_id,
                    3946625030 | 2328842519 | 3984593615
                );
            if trace_resize {
                eprintln!(
                    "NESTED_RESIZE_BEFORE_RUST node={} node_size={:?} cluster={} members={:?}",
                    self.nodes[node.0 as usize].tala_id,
                    self.nodes[node.0 as usize].rect.size,
                    self.clusters[cluster_index].vessel_tala_id,
                    self.clusters[cluster_index]
                        .members
                        .iter()
                        .map(|member| (
                            self.nodes[member.0 as usize].tala_id,
                            self.nodes[member.0 as usize].rect.size
                        ))
                        .collect::<Vec<_>>()
                );
            }
            #[cfg(feature = "diagnostic-traces")]
            if let Some(size) = self.cluster_vessel_sizes.get(cluster_index).copied() {
                crate::internal::pipeline::trace_record_cluster_pre_resize_size(
                    self.clusters[cluster_index].vessel_tala_id,
                    size,
                );
            }
            self.resize_cluster_members(cluster_index);
            // Graph.placeNodes calls Cluster.Resize on the owning vessel after
            // a nested member has been fitted. Keep the vessel dimensions in
            // step with that one cluster; refreshing every aggregate here can
            // alter arrangement scoring for unrelated clusters.
            let vessel_size = self.cluster_vessel_size_for_arrangement(
                cluster_index,
                self.clusters[cluster_index].arrangement,
            );
            if self.cluster_vessel_sizes.len() <= cluster_index {
                self.cluster_vessel_sizes
                    .resize(cluster_index + 1, Size::default());
            }
            self.cluster_vessel_sizes[cluster_index] = vessel_size;
            if trace_resize {
                eprintln!(
                    "NESTED_RESIZE_AFTER_RUST node={} cluster_size={:?} members={:?}",
                    self.nodes[node.0 as usize].tala_id,
                    vessel_size,
                    self.clusters[cluster_index]
                        .members
                        .iter()
                        .map(|member| (
                            self.nodes[member.0 as usize].tala_id,
                            self.nodes[member.0 as usize].rect.size
                        ))
                        .collect::<Vec<_>>()
                );
            }
            #[cfg(feature = "diagnostic-traces")]
            crate::internal::pipeline::trace_record_cluster_transition_size(
                self.clusters[cluster_index].vessel_tala_id,
                vessel_size,
            );
        }
    }

    /// Recovered `Graph.syncClusters` lifecycle.
    ///
    /// Each cluster sync performs `Resize` followed by
    /// `ArrangeClusterNodes`. TALA invokes it after every transaction
    /// operation, including after `repositionContainers` has independently
    /// wrapped each hierarchy node.
    #[track_caller]
    pub(super) fn sync_clusters(&mut self) {
        if crate::internal::trace_env_enabled("WEFTAN_TRACE_SYNC_CLUSTER_CALLER") {
            eprintln!(
                "SYNC_CLUSTER_CALLER_RUST {}",
                std::panic::Location::caller()
            );
        }
        let vessel_positions = self.cluster_vessel_positions();
        self.sync_clusters_from_positions(&vessel_positions);
    }

    /// Synchronize clusters from a direct transaction in the owning graph.
    /// The Go engine's live pointers make `MoveNodeWithChildren` walk each
    /// member's source descendants during this boundary; induced placement
    /// scopes use `sync_clusters` and its flattened-safe translation.
    pub(super) fn sync_clusters_with_descendants(&mut self) {
        let vessel_positions = self.cluster_vessel_positions();
        self.sync_clusters_from_positions_with_descendants(&vessel_positions);
    }

    /// Publish the final first-pass aggregate boundary while preserving the
    /// row-cluster member frame that Go's rejected packing transaction leaves
    /// for CleanupStuff.  Column clusters still need the post-recursion sync
    /// because their final vessel movement is not a half-pixel tie.
    pub(super) fn sync_non_row_clusters(&mut self) {
        let vessel_positions = self.cluster_vessel_positions();
        self.sync_clusters_from_positions_filtered(&vessel_positions, true);
    }

    /// Snapshot the independent temporary vessel boxes before another
    /// aggregate layer is synchronized.
    ///
    /// A cluster can contain a sequence vessel. TALA stores both temporary
    /// nodes independently, so `Sequence.sync` may move its retained steps
    /// without changing the enclosing cluster vessel. The stable-ID arena
    /// aliases both carriers onto retained members and must preserve the
    /// cluster top-left explicitly across that sequence publication.
    pub(super) fn cluster_vessel_positions(&self) -> Vec<Option<Point>> {
        self.clusters
            .iter()
            .enumerate()
            .map(|(cluster_index, cluster)| {
                self.pending_cluster_vessel_positions
                    .get(&cluster_index)
                    .copied()
                    .or_else(|| {
                        self.cluster_bounds(&cluster.members)
                            .map(|bounds| bounds.origin)
                    })
            })
            .collect()
    }

    /// Reconcile the stable-arena vessel carrier at the final `syncNested`
    /// boundary.
    ///
    /// TALA's cluster vessel is a separate `NewNode`; its position is the
    /// first member's position at the point the cluster is installed.  Rust
    /// keeps that member as the stable carrier, so the pending vessel frame
    /// must follow the carrier—not the minimum of all retained member boxes
    /// (which may still contain a hidden member from the pre-cluster graph).
    /// A hidden member's stale coordinate otherwise shifts the entire parent
    /// container by the exact amount of the parent origin.
    pub(super) fn reconcile_root_cluster_vessel_positions(&mut self) -> Vec<usize> {
        let carriers = self
            .pending_cluster_vessel_positions
            .keys()
            .copied()
            .filter_map(|cluster_index| {
                let carrier = *self.clusters[cluster_index].members.first()?;
                let position = self.position(carrier)?;
                Some((cluster_index, position))
            })
            .collect::<Vec<_>>();
        for (cluster_index, position) in carriers {
            self.pending_cluster_vessel_positions
                .insert(cluster_index, position);
        }
        self.pending_cluster_vessel_positions
            .keys()
            .copied()
            .collect()
    }

    #[track_caller]
    pub(super) fn sync_clusters_from_positions(&mut self, vessel_positions: &[Option<Point>]) {
        self.sync_clusters_from_positions_filtered(vessel_positions, false);
    }

    fn sync_clusters_from_positions_with_descendants(
        &mut self,
        vessel_positions: &[Option<Point>],
    ) {
        self.sync_clusters_from_positions_filtered_impl(vessel_positions, false, true, true);
    }

    /// Publish aggregate member geometry at a stage boundary even when the
    /// stable arena's carrier is temporarily absent from `node_order`.
    /// Go's `Graph.SyncClusters` still visits that live temporary vessel;
    /// routing cleanup uses the ordinary guarded form below to avoid replaying
    /// retired vessels.
    #[track_caller]
    pub(super) fn sync_clusters_from_positions_force(
        &mut self,
        vessel_positions: &[Option<Point>],
    ) {
        self.sync_clusters_from_positions_filtered_impl(vessel_positions, false, false, false);
    }

    /// Correct the one-pixel tie left by a stable carrier after a rejected
    /// transaction.  Go's temporary vessel is synchronized from the member
    /// pointer state that preceded the transaction; the arena can retain the
    /// opposite side of a half-pixel center after rollback.  The only
    /// observable difference is a unit translation, so move members to the
    /// rounded geometry offset when they are already within one pixel of that
    /// target. Larger deltas indicate a real candidate move and are left to
    /// the normal transaction lifecycle.
    pub(super) fn reconcile_cluster_member_rounding_ties(&mut self) {
        let positions = self.cluster_vessel_positions();
        for (cluster_index, vessel_top_left) in positions.into_iter().enumerate() {
            let Some(vessel_top_left) = vessel_top_left else {
                continue;
            };
            let members = self.clusters[cluster_index].members.clone();
            for member in members {
                let Some(current) = self.position(member) else {
                    continue;
                };
                let Some((offset, _size)) = self.cluster_member_geometry(cluster_index, member)
                else {
                    continue;
                };
                let target = Point {
                    x: vessel_top_left.x + offset.x,
                    y: vessel_top_left.y + offset.y,
                };
                let delta = Point {
                    x: target.x - current.x,
                    y: target.y - current.y,
                };
                if delta != Point::default() && delta.x.abs() <= 1.0 && delta.y.abs() <= 1.0 {
                    if crate::internal::trace_env_enabled("WEFTAN_TRACE_CLUSTER_TIES") {
                        eprintln!(
                            "CLUSTER_TIE_RECONCILE_RUST vessel={} member={} current={:?} target={:?} delta={:?}",
                            self.clusters[cluster_index].vessel_tala_id,
                            self.nodes[member.0 as usize].tala_id,
                            current,
                            target,
                            delta,
                        );
                    }
                    self.translate_node_with_children(member, delta);
                }
            }
        }
    }

    #[track_caller]
    fn sync_clusters_from_positions_filtered(
        &mut self,
        vessel_positions: &[Option<Point>],
        skip_row_clusters: bool,
    ) {
        self.sync_clusters_from_positions_filtered_impl(
            vessel_positions,
            skip_row_clusters,
            true,
            false,
        );
    }

    #[track_caller]
    fn sync_clusters_from_positions_filtered_impl(
        &mut self,
        vessel_positions: &[Option<Point>],
        skip_row_clusters: bool,
        skip_inactive_with_pending: bool,
        move_descendants: bool,
    ) {
        if crate::internal::trace_env_enabled("WEFTAN_TRACE_SYNC_CLUSTER_CALLER") {
            eprintln!(
                "SYNC_CLUSTER_POS_CALLER_RUST {}",
                std::panic::Location::caller()
            );
        }
        for cluster_index in 0..self.clusters.len() {
            if skip_row_clusters
                && self.clusters[cluster_index].arrangement == ClusterArrangement::Row
            {
                continue;
            }
            let cluster_active = self.cluster_is_active(&self.clusters[cluster_index]);
            // Once Cleanup retires a temporary Go cluster vessel, the cluster
            // remains in metadata only so edge abductions and final snapshots
            // can still refer to it. Go's SyncNested walks the live node list,
            // so it cannot visit that retired vessel again. The stable arena
            // keeps the metadata and member nodes, therefore an unconditional
            // sync here would replay the stale vessel frame during the routed
            // BinPack pass. Restrict publication to clusters whose carrier is
            // still active in node_order, matching the upstream lifecycle.
            if skip_inactive_with_pending
                && !cluster_active
                && self
                    .pending_cluster_vessel_positions
                    .contains_key(&cluster_index)
            {
                if crate::internal::trace_env_enabled("WEFTAN_TRACE_SYNC_CLUSTER_SKIP") {
                    eprintln!(
                        "SYNC_CLUSTER_SKIP_RUST vessel={} caller={} active={} first_in_order={} pending={} ext_aggs={} ext_containers={} node_order_len={}",
                        self.clusters[cluster_index].vessel_tala_id,
                        std::panic::Location::caller(),
                        cluster_active,
                        self.clusters[cluster_index]
                            .members
                            .first()
                            .is_some_and(|member| self.node_order_contains(*member)),
                        self.pending_cluster_vessel_positions
                            .contains_key(&cluster_index),
                        self.transaction_external_aggregate_children.len(),
                        self.transaction_external_containers.len(),
                        self.node_order.len(),
                    );
                }
                continue;
            }
            let Some(vessel_top_left) = vessel_positions.get(cluster_index).copied().flatten()
            else {
                if crate::internal::trace_env_enabled("WEFTAN_TRACE_SYNC_CLUSTER_DETAIL") {
                    eprintln!(
                        "SYNC_CLUSTER_NO_POSITION_RUST vessel={} caller={} active={} first_in_order={} pending={} node_order_len={}",
                        self.clusters[cluster_index].vessel_tala_id,
                        std::panic::Location::caller(),
                        cluster_active,
                        self.clusters[cluster_index]
                            .members
                            .first()
                            .is_some_and(|member| self.node_order_contains(*member)),
                        self.pending_cluster_vessel_positions
                            .contains_key(&cluster_index),
                        self.node_order.len(),
                    );
                }
                continue;
            };
            let trace =
                crate::internal::trace_env_value("WEFTAN_TRACE_SYNC_CLUSTER").unwrap_or_default();
            let traced = trace == "all"
                || trace
                    .parse::<i64>()
                    .ok()
                    .is_some_and(|id| id == self.clusters[cluster_index].vessel_tala_id);
            if traced {
                eprintln!(
                    "SYNC_CLUSTER_ACTIVE_RUST vessel={} active={} first_in_order={}",
                    self.clusters[cluster_index].vessel_tala_id,
                    cluster_active,
                    self.clusters[cluster_index]
                        .members
                        .first()
                        .is_some_and(|member| self.node_order_contains(*member))
                );
                eprint!(
                    "SYNC_CLUSTER_RUST before vessel={}@{},{}:{},{}",
                    self.clusters[cluster_index].vessel_tala_id,
                    vessel_top_left.x,
                    vessel_top_left.y,
                    self.cluster_vessel_size(cluster_index).width,
                    self.cluster_vessel_size(cluster_index).height
                );
                for member in self.clusters[cluster_index].members.iter().copied() {
                    let node = &self.nodes[member.0 as usize];
                    eprint!(
                        " member={}@{},{}:{},{}",
                        node.tala_id,
                        node.position.unwrap().x,
                        node.position.unwrap().y,
                        node.rect.size.width,
                        node.rect.size.height
                    );
                }
                eprintln!();
            }
            self.resize_cluster_members(cluster_index);
            if move_descendants {
                self.arrange_cluster_members_with_descendants(cluster_index, vessel_top_left);
            } else {
                self.arrange_cluster_members(cluster_index, vessel_top_left);
            }
            if self
                .pending_cluster_vessel_positions
                .contains_key(&cluster_index)
            {
                self.pending_cluster_vessel_positions
                    .insert(cluster_index, vessel_top_left);
            }
            if traced {
                eprint!(
                    "SYNC_CLUSTER_RUST after vessel={}@{},{}:{},{}",
                    self.clusters[cluster_index].vessel_tala_id,
                    vessel_top_left.x,
                    vessel_top_left.y,
                    self.cluster_vessel_size(cluster_index).width,
                    self.cluster_vessel_size(cluster_index).height
                );
                for member in self.clusters[cluster_index].members.iter().copied() {
                    let node = &self.nodes[member.0 as usize];
                    eprint!(
                        " member={}@{},{}:{},{}",
                        node.tala_id,
                        node.position.unwrap().x,
                        node.position.unwrap().y,
                        node.rect.size.width,
                        node.rect.size.height
                    );
                }
                eprintln!();
            }
        }
        if !crate::internal::trace_env_enabled("WEFTAN_DISABLE_EXTERNAL_SYNC") {
            self.sync_external_clusters();
        }
    }

    fn sync_external_clusters(&mut self) {
        // A projected cluster vessel has aggregate children.
        if self.transaction_external_aggregate_children.is_empty()
            && !crate::internal::trace_env_enabled("WEFTAN_TRACE_EXTERNAL_ORDER")
        {
            return;
        }
        let order = self.projected_reverse_dfs_order();
        if crate::internal::trace_env_enabled("WEFTAN_TRACE_EXTERNAL_ORDER") {
            eprintln!("EXTERNAL_ORDER_RUST {:?}", order);
        }
        for tala_id in order {
            if self.projected_node_is_cluster_vessel(tala_id) {
                self.sync_external_cluster(tala_id);
            }
        }
    }

    /// Synchronize one projected cluster vessel retained by an induced graph.
    ///
    /// `Graph.syncNested` performs this operation at the vessel's position in
    /// `Graph.Nodes`, before applying container padding to that cluster's
    /// retained member children. Keeping the single-vessel boundary explicit
    /// lets the flattened projection preserve the same node-loop order.
    pub(super) fn sync_external_cluster(&mut self, vessel_tala_id: i64) {
        let trace_object2 = crate::internal::trace_env_enabled("WEFTAN_TRACE_OBJECT2");
        if trace_object2 {
            let positions = self
                .transaction_external_aggregate_children
                .get(&6334824724549167320)
                .map(|children| {
                    children
                        .iter()
                        .map(|child| (child.tala_id, child.position))
                        .collect::<Vec<_>>()
                });
            eprintln!(
                "OBJECT2_SYNC_CLUSTER_BEGIN caller={} vessel={} {:?}",
                std::panic::Location::caller(),
                vessel_tala_id,
                positions
            );
        }
        let Some(layout) = self
            .transaction_external_cluster_layouts
            .get(&vessel_tala_id)
            .copied()
        else {
            return;
        };
        if crate::internal::trace_env_enabled("WEFTAN_TRACE_CLUSTER_STATE")
            && vessel_tala_id == 6_129_484_611_666_145_821
        {
            eprintln!(
                "CLUSTER_STATE_SYNC_RUST vessel={} layout={:?} clusters={:?}",
                vessel_tala_id,
                layout.arrangement,
                self.clusters
                    .iter()
                    .map(|cluster| (
                        cluster.vessel_tala_id,
                        cluster.arrangement,
                        cluster.desired_arrangement
                    ))
                    .collect::<Vec<_>>()
            );
        }
        let Some(mut members) = self
            .transaction_external_aggregate_children
            .get(&vessel_tala_id)
            .cloned()
        else {
            return;
        };
        if members.is_empty() {
            return;
        }
        // Cluster.sync begins with Cluster.Resize. Non-fixed clusters publish
        // the maximum width and height to every retained member pointer before
        // computing the temporary vessel dimensions.
        let maximum = members
            .iter()
            .fold(Size::default(), |maximum, member| Size {
                width: maximum.width.max(member.rect.size.width),
                height: maximum.height.max(member.rect.size.height),
            });
        if !layout.fixed_size {
            for member in &mut members {
                member.rect.size = maximum;
                self.set_projected_node_size(member.tala_id, maximum);
            }
        }
        let count = members.len() as f64;
        let gaps = layout.padding * (count - 1.0).max(0.0);
        let vessel = match layout.arrangement {
            ClusterArrangement::Row => Size {
                width: maximum.width * count + gaps,
                height: maximum.height,
            },
            ClusterArrangement::Column => Size {
                width: maximum.width,
                height: maximum.height * count + gaps,
            },
        };
        // Cluster.Resize always publishes the aggregate dimensions to the
        // temporary vessel. FixedSize gates only the member equalization
        // above; it does not preserve the vessel's previous dimensions.
        self.set_projected_node_size(vessel_tala_id, vessel);

        // Cluster.sync always runs Resize first. ArrangeClusterNodes alone
        // returns for a nil TopLeft. Resolve the distinct vessel pointer from
        // the current Graph.Nodes slice or any retained container/aggregate
        // alias only after publishing all Resize mutations.
        let Some(vessel_top_left) = self.projected_node_position_by_tala(vessel_tala_id) else {
            return;
        };
        if crate::internal::trace_env_enabled("WEFTAN_TRACE_SYNC_EXTERNAL_DETAIL") {
            eprintln!(
                "SYNC_EXTERNAL_DETAIL_RUST before vessel={} pos={:?} members={:?}",
                vessel_tala_id,
                vessel_top_left,
                members
                    .iter()
                    .map(|member| (member.tala_id, member.position))
                    .collect::<Vec<_>>()
            );
        }
        let mut targets = Vec::with_capacity(members.len());
        match layout.arrangement {
            ClusterArrangement::Row => {
                let center = vessel_top_left.y + vessel.height / 2.0;
                let mut position = vessel_top_left.x;
                for member in &members {
                    let point = if let Some(old_position) = member.position {
                        Point {
                            x: position,
                            // ArrangeClusterNodes moves existing pointers by
                            // a rounded delta. This differs from rounding the
                            // absolute target at half-pixel centers because
                            // Go's math.Round is applied to the displacement.
                            y: old_position.y
                                + go_round(
                                    center - (old_position.y + member.rect.size.height / 2.0),
                                ),
                        }
                    } else {
                        Point {
                            x: position,
                            y: (center - member.rect.size.height / 2.0).round(),
                        }
                    };
                    targets.push((
                        member.tala_id,
                        member.position,
                        point,
                        member.is_container,
                        member.transaction_position_was_nil,
                    ));
                    position += member.rect.size.width + layout.padding;
                }
            }
            ClusterArrangement::Column => {
                let center = vessel_top_left.x + vessel.width / 2.0;
                let mut position = vessel_top_left.y;
                for member in &members {
                    let point = if let Some(old_position) = member.position {
                        Point {
                            x: old_position.x
                                + go_round(
                                    center - (old_position.x + member.rect.size.width / 2.0),
                                ),
                            y: position,
                        }
                    } else {
                        Point {
                            x: (center - member.rect.size.width / 2.0).round(),
                            y: position,
                        }
                    };
                    targets.push((
                        member.tala_id,
                        member.position,
                        point,
                        member.is_container,
                        member.transaction_position_was_nil,
                    ));
                    position += member.rect.size.height + layout.padding;
                }
            }
        }
        for (member_tala_id, old_position, target, is_container, position_was_nil) in targets {
            if position_was_nil {
                self.set_projected_node_position(member_tala_id, target);
                if is_container {
                    self.position_projected_container_children(member_tala_id, false);
                }
                self.consume_projected_logical_nil_position(member_tala_id);
            } else if let Some(old_position) = old_position {
                self.translate_projected_subtrees(
                    &[member_tala_id],
                    Point {
                        x: target.x - old_position.x,
                        y: target.y - old_position.y,
                    },
                );
            } else {
                self.set_projected_node_position(member_tala_id, target);
                if is_container {
                    self.position_projected_container_children(member_tala_id, false);
                }
            }
        }
        self.reconcile_projected_offsets_from_external_cluster(vessel_tala_id);
        if crate::internal::trace_env_enabled("WEFTAN_TRACE_SYNC_EXTERNAL_DETAIL") {
            eprintln!(
                "SYNC_EXTERNAL_DETAIL_RUST after vessel={} members={:?}",
                vessel_tala_id,
                self.transaction_external_aggregate_children
                    .get(&vessel_tala_id)
                    .map(|children| children
                        .iter()
                        .map(|member| (member.tala_id, member.position))
                        .collect::<Vec<_>>())
            );
        }
        if trace_object2 {
            let positions = self
                .transaction_external_aggregate_children
                .get(&6334824724549167320)
                .map(|children| {
                    children
                        .iter()
                        .map(|child| (child.tala_id, child.position))
                        .collect::<Vec<_>>()
                });
            eprintln!(
                "OBJECT2_SYNC_CLUSTER_AFTER vessel={} {:?}",
                vessel_tala_id, positions
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ContentAlignment, Edge, Insets, LabelPosition, Node};

    #[test]
    fn orientation_resizes_external_cluster_before_moving_its_members() {
        for fixed_size in [true, false] {
            let mut input = Graph::default();
            let vessel = input.add_node(Node::new(
                "vessel",
                Size {
                    width: 40.0,
                    height: 67.0,
                },
            ));
            let a = input.add_node(Node::new(
                "a",
                Size {
                    width: 20.0,
                    height: 30.0,
                },
            ));
            let b = input.add_node(Node::new(
                "b",
                Size {
                    width: 40.0,
                    height: 10.0,
                },
            ));
            let mut graph = ArenaGraph::from_input(&input);
            let id = graph.nodes[vessel.0 as usize].tala_id;
            let mut first = graph.nodes[a.0 as usize].clone();
            let mut second = graph.nodes[b.0 as usize].clone();
            first.position = Some(Point { x: 0.0, y: 0.0 });
            second.position = Some(Point { x: 0.0, y: 37.0 });
            graph
                .transaction_external_aggregate_children
                .insert(id, vec![first, second]);
            graph.transaction_external_cluster_layouts.insert(
                id,
                ExternalClusterLayout {
                    arrangement: ClusterArrangement::Column,
                    padding: 7.0,
                    fixed_size,
                },
            );
            assert!(graph.clusters.is_empty());
            graph.rotate_projected_cluster_for_orientation(vessel);
            assert_eq!(
                graph.nodes[vessel.0 as usize].rect.size,
                Size {
                    width: 87.0,
                    height: 30.0
                }
            );
            assert_eq!(
                graph.transaction_external_cluster_layouts[&id].arrangement,
                ClusterArrangement::Row
            );
            assert_eq!(graph.transaction_external_cluster_layouts[&id].padding, 7.0);
            // Source calls Resize before moving the vessel, and SyncGeometry only
            // after all vessel origins have turned. The retained positions remain
            // untouched by this resize boundary.
            assert_eq!(
                graph.transaction_external_aggregate_children[&id][1].position,
                Some(Point { x: 0.0, y: 37.0 })
            );
            assert_eq!(
                graph.transaction_external_aggregate_children[&id][0]
                    .rect
                    .size,
                Size {
                    width: if fixed_size { 20.0 } else { 40.0 },
                    height: 30.0,
                }
            );
            assert_eq!(
                graph.transaction_external_aggregate_children[&id][1]
                    .rect
                    .size,
                Size {
                    width: 40.0,
                    height: if fixed_size { 10.0 } else { 30.0 },
                }
            );
            assert_eq!(
                graph.transaction_external_aggregate_children[&id][0].position,
                Some(Point { x: 0.0, y: 0.0 })
            );
        }
    }

    #[test]
    fn cluster_optimization_retains_reconnected_external_and_half_pixel_shift() {
        let mut input = Graph::default();
        let first = input.add_node(Node::new(
            "first",
            Size {
                width: 139.0,
                height: 86.0,
            },
        ));
        let second = input.add_node(Node::new(
            "second",
            Size {
                width: 139.0,
                height: 86.0,
            },
        ));
        let peer = input.add_node(Node::new(
            "peer",
            Size {
                width: 197.0,
                height: 86.0,
            },
        ));
        for member in [first, second] {
            input.add_edge(Edge {
                source: member,
                target: peer,
            });
        }
        let mut graph = ArenaGraph::from_input(&input);
        graph.clusters.push(ClusterState {
            members: vec![first, second],
            arrangement: ClusterArrangement::Row,
            desired_arrangement: ClusterArrangement::Row,
            padding: 20.0,
            vessel_tala_id: 10001,
            fixed_size: false,
        });
        for member in [first, second] {
            graph.nodes[member.0 as usize].cluster = Some(0);
        }
        graph.node_order = vec![first, peer];
        let origin = Point {
            x: 100.25,
            y: 300.75,
        };
        let mut entity = graph.nodes[first.0 as usize].clone();
        entity.tala_id = 10001;
        entity.position = Some(origin);
        entity.rect.size = Size {
            width: 298.0,
            height: 86.0,
        };
        let vessel = graph.retain_topology_node(entity);
        graph
            .bind_topology_vessel(TopologyVessel::Cluster(0), vessel)
            .unwrap();
        graph.pending_cluster_vessel_positions.insert(0, origin);
        graph.set_position(first, origin);
        graph.set_position(
            second,
            Point {
                x: 259.25,
                y: 300.75,
            },
        );
        graph.set_position(
            peer,
            Point {
                x: 151.25,
                y: 100.75,
            },
        );
        graph.refresh_node_order_membership();
        assert_eq!(graph.cluster_external_nodes(0), vec![peer]);
        for edge in &mut graph.edge_records {
            edge.from = vessel;
        }
        assert_eq!(graph.cluster_external_nodes(0), vec![peer]);
        // Go math.Round(-0.5) moves the connected node one pixel left.
        let trial = graph.cluster_alignment_trial(0, false).unwrap();
        assert_eq!(
            trial.position(peer),
            Some(Point {
                x: 150.25,
                y: 100.75
            })
        );
        assert_eq!(
            graph.position(peer),
            Some(Point {
                x: 151.25,
                y: 100.75
            })
        );
        graph.clear_position(peer);
        assert!(graph.cluster_external_nodes(0).is_empty());
    }

    fn node(name: &str, shape: ShapeKind) -> Node {
        Node {
            d2_near_key: None,
            external_id: name.to_owned(),
            size: Size {
                width: 100.0,
                height: 80.0,
            },
            declared_size: None,
            d2_authored_geometry: None,
            label_size: None,
            font_size: None,
            label_position: LabelPosition::Unset,
            parent: None,
            locked_position: None,
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
            content_insets: Insets::uniform(60.0),
            layout_margins: Insets::uniform(0.0),
            external_label: None,
            icon_position: None,
            has_icon: false,
            label_aware_grid: false,
            packed_grid: false,
            content_alignment: ContentAlignment::Padding,
            port_spread: 0.0,
            person: shape == ShapeKind::Person,
            is_3d: false,
            is_multiple: false,
            shape,
        }
    }

    #[test]
    fn add_clusters_groups_same_shape_neighbor_and_direction_signature() {
        let mut input = Graph::default();
        let first = input.add_node(node("first", ShapeKind::Person));
        let second = input.add_node(node("second", ShapeKind::Person));
        let pivot = input.add_node(node("pivot", ShapeKind::Person));
        for (from, to) in [
            (first, pivot),
            (first, pivot),
            (pivot, first),
            (second, pivot),
            (second, pivot),
            (pivot, second),
        ] {
            input.add_edge(Edge {
                source: from,
                target: to,
            });
        }

        let mut graph = ArenaGraph::from_input(&input);
        graph.assign_clusters(1, false);

        assert_eq!(
            graph.nodes[first.0 as usize].cluster,
            graph.nodes[second.0 as usize].cluster
        );
        assert!(graph.nodes[first.0 as usize].cluster.is_some());
        assert!(graph.nodes[pivot.0 as usize].cluster.is_none());
        assert_eq!(graph.clusters.len(), 1);
        assert_eq!(graph.clusters[0].members, vec![first, second]);
        assert_eq!(graph.clusters[0].arrangement, ClusterArrangement::Column);
        assert_eq!(
            graph.clusters[0].desired_arrangement,
            ClusterArrangement::Column
        );
    }

    #[test]
    fn add_clusters_orders_positioned_column_members_before_vessel_creation() {
        let mut input = Graph::default();
        let first = input.add_node(node("first", ShapeKind::Person));
        let second = input.add_node(node("second", ShapeKind::Person));
        let pivot = input.add_node(node("pivot", ShapeKind::Person));
        for (from, to) in [(first, pivot), (second, pivot)] {
            input.add_edge(Edge {
                source: from,
                target: to,
            });
        }
        input.set_initial_position(first, Some(Point { x: 0.0, y: 100.0 }));
        input.set_initial_position(second, Some(Point { x: 0.0, y: 0.0 }));

        let mut graph = ArenaGraph::from_input(&input);
        graph.assign_clusters(1, false);

        assert_eq!(graph.clusters.len(), 1);
        assert_eq!(graph.clusters[0].arrangement, ClusterArrangement::Column);
        assert_eq!(graph.clusters[0].members, vec![second, first]);
    }

    #[test]
    fn cluster_member_geometry_uses_vessel_arrangement_and_padding() {
        let mut input = Graph::default();
        let mut first_node = node("first", ShapeKind::Rectangle);
        first_node.size = Size {
            width: 80.0,
            height: 40.0,
        };
        let first = input.add_node(first_node);
        let mut second_node = node("second", ShapeKind::Rectangle);
        second_node.size = Size {
            width: 100.0,
            height: 80.0,
        };
        let second = input.add_node(second_node);
        let graph = {
            let mut graph = ArenaGraph::from_input(&input);
            graph.clusters.push(ClusterState {
                members: vec![first, second],
                arrangement: ClusterArrangement::Row,
                desired_arrangement: ClusterArrangement::Row,
                padding: 20.0,
                vessel_tala_id: 17,
                fixed_size: false,
            });
            graph
        };

        assert_eq!(
            graph.cluster_member_geometry(0, first),
            Some((
                Point { x: 0.0, y: 0.0 },
                Size {
                    width: 100.0,
                    height: 80.0,
                },
            ))
        );
        assert_eq!(
            graph.cluster_member_geometry(0, second),
            Some((
                Point { x: 120.0, y: 0.0 },
                Size {
                    width: 100.0,
                    height: 80.0,
                },
            ))
        );
    }

    #[test]
    fn projected_flip_padding_uses_target_arrangement_and_positioned_gap() {
        let mut input = Graph::default();
        let mut first_node = node("first", ShapeKind::Rectangle);
        first_node.size = Size {
            width: 400.0,
            height: 50.0,
        };
        let first = input.add_node(first_node);
        let mut second_node = node("second", ShapeKind::Rectangle);
        second_node.size = Size {
            width: 400.0,
            height: 50.0,
        };
        let second = input.add_node(second_node);
        let mut graph = ArenaGraph::from_input(&input);
        graph.set_position(first, Point::default());
        graph.set_position(second, Point { x: 440.0, y: 0.0 });
        graph.clusters.push(ClusterState {
            members: vec![first, second],
            arrangement: ClusterArrangement::Row,
            desired_arrangement: ClusterArrangement::Row,
            padding: 40.0,
            vessel_tala_id: 17,
            fixed_size: false,
        });

        let padding = graph.prearranged_cluster_padding(0, ClusterArrangement::Column);
        assert_eq!(padding, 20.0);
        assert_eq!(
            graph.cluster_vessel_size_for_arrangement_with_padding(
                0,
                ClusterArrangement::Column,
                padding,
            ),
            Size {
                width: 400.0,
                height: 120.0,
            }
        );
        assert_eq!(
            graph.cluster_member_geometry_for_arrangement_with_padding(
                0,
                second,
                ClusterArrangement::Column,
                padding,
            ),
            Some((
                Point { x: 0.0, y: 70.0 },
                input.nodes[second.0 as usize].size
            ))
        );
    }

    #[test]
    fn projected_cluster_flip_retains_row_when_both_column_trials_are_invalid() {
        use crate::internal::model::ProjectedTransactionNode;

        const VESSEL_TALA_ID: i64 = 10_000;
        const FIRST_MEMBER_TALA_ID: i64 = 10_001;
        const SECOND_MEMBER_TALA_ID: i64 = 10_002;
        const PRIOR_SPACING_TALA_ID: i64 = 20_000;
        const EXTERNAL_CONTAINER_TALA_ID: i64 = 30_000;

        let mut input = Graph::default();
        let mut vessel_node = node("projected vessel", ShapeKind::Rectangle);
        vessel_node.size = Size {
            width: 128.0,
            height: 66.0,
        };
        let vessel = input.add_node(vessel_node);
        let mut adjacent_node = node("right adjacent", ShapeKind::Rectangle);
        adjacent_node.size = Size {
            width: 53.0,
            height: 66.0,
        };
        let adjacent = input.add_node(adjacent_node);
        input.add_edge(Edge {
            source: vessel,
            target: adjacent,
        });

        let mut graph = ArenaGraph::from_input(&input);
        graph.nodes[vessel.0 as usize].tala_id = VESSEL_TALA_ID;
        graph.nodes[vessel.0 as usize].scoring_is_aggregate_vessel = true;
        graph.nodes[vessel.0 as usize].scoring_cluster_arrangement = Some(ClusterArrangement::Row);
        graph.set_position(vessel, Point { x: 0.0, y: 128.0 });
        graph.set_position(adjacent, Point { x: 256.0, y: 128.0 });
        graph.initialize_projected_transaction_nodes_from_current_graph();

        let member_size = Size {
            width: 54.0,
            height: 66.0,
        };
        let row = ProjectedClusterLayout {
            size: Size {
                width: 128.0,
                height: 66.0,
            },
            padding: 20.0,
            members: BTreeMap::from([
                (FIRST_MEMBER_TALA_ID, (Point::default(), member_size)),
                (
                    SECOND_MEMBER_TALA_ID,
                    (Point { x: 74.0, y: 0.0 }, member_size),
                ),
            ]),
        };
        let column = ProjectedClusterLayout {
            size: Size {
                width: 54.0,
                height: 152.0,
            },
            padding: 20.0,
            members: BTreeMap::from([
                (FIRST_MEMBER_TALA_ID, (Point::default(), member_size)),
                (
                    SECOND_MEMBER_TALA_ID,
                    (Point { x: 0.0, y: 86.0 }, member_size),
                ),
            ]),
        };
        let projection = ProjectedCluster {
            node: vessel,
            cluster_index: 0,
            arrangement: ClusterArrangement::Row,
            desired_arrangement: ClusterArrangement::Row,
            row,
            column,
        };

        let mut first_member = graph.nodes[vessel.0 as usize].clone();
        first_member.input_id = NodeId(u32::MAX - 2);
        first_member.tala_id = FIRST_MEMBER_TALA_ID;
        first_member.rect.size = member_size;
        first_member.edges.clear();
        first_member.nears.clear();
        first_member.is_container = false;
        first_member.scoring_is_container = false;
        first_member.position = Some(Point { x: 0.0, y: 128.0 });
        first_member.rect.origin = first_member.position.unwrap();
        let mut second_member = first_member.clone();
        second_member.input_id = NodeId(u32::MAX - 1);
        second_member.tala_id = SECOND_MEMBER_TALA_ID;
        second_member.position = Some(Point { x: 74.0, y: 128.0 });
        second_member.rect.origin = second_member.position.unwrap();
        graph
            .transaction_external_aggregate_children
            .insert(VESSEL_TALA_ID, vec![first_member, second_member]);
        graph.transaction_external_cluster_layouts.insert(
            VESSEL_TALA_ID,
            ExternalClusterLayout {
                arrangement: ClusterArrangement::Row,
                padding: 20.0,
                fixed_size: false,
            },
        );

        // The centered Column trial moves the vessel from y=128 to y=85.
        // This 52x66 retained node is separated from the Row by 12 pixels,
        // but the taller centered Column overlaps its exact box.
        let mut prior_spacing_node = graph.nodes[vessel.0 as usize].clone();
        prior_spacing_node.input_id = NodeId(u32::MAX);
        prior_spacing_node.tala_id = PRIOR_SPACING_TALA_ID;
        prior_spacing_node.rect.size = Size {
            width: 52.0,
            height: 66.0,
        };
        prior_spacing_node.edges.clear();
        prior_spacing_node.nears.clear();
        prior_spacing_node.is_container = false;
        prior_spacing_node.scoring_is_container = false;
        prior_spacing_node.position = Some(Point { x: 0.0, y: 50.0 });
        prior_spacing_node.rect.origin = prior_spacing_node.position.unwrap();
        graph
            .projected_transaction_nodes
            .push(ProjectedTransactionNode {
                structural_container_tala_id: None,
                structural_ancestor_tala_ids: Vec::new(),
                node: prior_spacing_node,
                container_tala_id: None,
                ancestor_tala_ids: Vec::new(),
                child_tala_ids: Vec::new(),
                edges: Vec::new(),
            });

        // The uncentered Column occupies y=128..280. The external container
        // begins at y=284, so TALA's 20-pixel IsBadState spacing rejects it;
        // the prior Row ends at y=194 and is valid.
        let mut external_container = graph.nodes[vessel.0 as usize].clone();
        external_container.input_id = NodeId(u32::MAX);
        external_container.tala_id = EXTERNAL_CONTAINER_TALA_ID;
        external_container.rect.size = Size {
            width: 189.0,
            height: 318.0,
        };
        external_container.edges.clear();
        external_container.nears.clear();
        external_container.is_container = true;
        external_container.scoring_is_container = true;
        external_container.position = Some(Point { x: 0.0, y: 284.0 });
        external_container.rect.origin = external_container.position.unwrap();
        graph
            .transaction_external_containers
            .push(external_container);

        assert_eq!(
            graph.projected_cluster_desired_arrangement(
                vessel,
                ClusterArrangement::Row,
                ClusterArrangement::Row,
            ),
            ClusterArrangement::Column
        );
        assert!(graph.transaction_external_containers_are_valid());

        let mut spacing_only = graph.clone();
        spacing_only.transaction_external_containers.clear();
        assert!(
            spacing_only
                .projected_cluster_flip_trial(
                    vessel,
                    ClusterArrangement::Column,
                    &projection.column,
                    true,
                )
                .is_none(),
            "the centered Column must be rejected when padded spacing becomes exact overlap"
        );
        assert!(
            spacing_only
                .projected_cluster_flip_trial(
                    vessel,
                    ClusterArrangement::Column,
                    &projection.column,
                    false,
                )
                .is_some(),
            "the prior-spacing node must not reject the uncentered Column"
        );

        let mut fallback = graph.clone();
        fallback.apply_projected_cluster_layout(
            vessel,
            ClusterArrangement::Column,
            &projection.column,
        );
        fallback.reposition_ordinary_containers();
        assert_eq!(
            fallback.nodes[vessel.0 as usize].rect.size, projection.column.size,
            "Graph.syncClusters must retain the trial's Column vessel size"
        );
        assert_eq!(
            fallback.transaction_external_cluster_layouts[&VESSEL_TALA_ID].arrangement,
            ClusterArrangement::Column,
            "the projected arrangement is shared transaction state, like recovered Cluster.Arrangement"
        );
        assert!(
            !fallback.transaction_external_containers_are_valid(),
            "the 54x152 Column must violate the external container's 20-pixel spacing"
        );
        assert!(
            graph
                .projected_cluster_flip_trial(
                    vessel,
                    ClusterArrangement::Column,
                    &projection.column,
                    false,
                )
                .is_none(),
            "the uncentered Column must be rejected by the unrelated external container"
        );

        assert_eq!(
            graph.optimize_projected_cluster_flip(vessel, &projection),
            None
        );
        assert_eq!(
            graph.nodes[vessel.0 as usize].rect.size,
            projection.row.size
        );
        assert_eq!(
            graph.nodes[vessel.0 as usize].scoring_cluster_arrangement,
            Some(ClusterArrangement::Row)
        );
        assert_eq!(
            graph.transaction_external_cluster_layouts[&VESSEL_TALA_ID].arrangement,
            ClusterArrangement::Row
        );
    }

    #[test]
    fn projected_transaction_state_uses_global_connected_edge_delta() {
        use crate::internal::model::{ProjectedTransactionEdge, ProjectedTransactionNode};

        const VESSEL_TALA_ID: i64 = 40_000;
        const GLOBAL_TALA_ID: i64 = 40_001;
        let mut input = Graph::default();
        let mut vessel_node = node("vessel", ShapeKind::Rectangle);
        vessel_node.size = Size {
            width: 100.0,
            height: 66.0,
        };
        let vessel = input.add_node(vessel_node);
        let mut graph = ArenaGraph::from_input(&input);
        graph.nodes[vessel.0 as usize].tala_id = VESSEL_TALA_ID;
        graph.set_position(vessel, Point { x: 0.0, y: 128.0 });
        graph.initialize_projected_transaction_nodes_from_current_graph();

        let mut global = graph.nodes[vessel.0 as usize].clone();
        global.input_id = NodeId(u32::MAX);
        global.tala_id = GLOBAL_TALA_ID;
        global.position = Some(Point { x: 0.0, y: 0.0 });
        global.rect.origin = global.position.unwrap();
        global.rect.size = Size {
            width: 100.0,
            height: 100.0,
        };
        global.edges.clear();
        graph.projected_transaction_nodes[0]
            .edges
            .push(ProjectedTransactionEdge {
                adjacent_tala_id: GLOBAL_TALA_ID,
                min_width: 0.0,
                min_height: 80.0,
            });
        graph
            .projected_transaction_nodes
            .push(ProjectedTransactionNode {
                structural_container_tala_id: None,
                structural_ancestor_tala_ids: Vec::new(),
                node: global,
                container_tala_id: None,
                ancestor_tala_ids: Vec::new(),
                child_tala_ids: Vec::new(),
                edges: Vec::new(),
            });

        // The boxes have a 28-unit vertical gap. They overlap only because
        // the owning-graph edge raises getDeltaTo above the ordinary 20.
        let prior = graph.projected_transaction_state();
        assert_eq!(prior.existing_overlaps, BTreeSet::from([(0, 1)]));
        assert!(prior.existing_exact_overlaps.is_empty());

        graph.nodes[vessel.0 as usize].rect.size.height = 152.0;
        graph.set_position(vessel, Point { x: 0.0, y: 85.0 });
        assert!(
            graph.projected_transaction_nodes_are_valid(&prior),
            "the prior padded pair is an IsBadState overlap exception"
        );
        assert!(
            graph.projected_transaction_spacing_overlap_became_exact(&prior),
            "Commit must separately reject the padded-to-exact transition"
        );
    }

    #[test]
    fn projected_transaction_carrier_synthesizes_aggregate_vessels_in_graph_order() {
        let mut input = Graph::default();
        let ordinary = input.add_node(node("ordinary", ShapeKind::Diamond));
        let sequence_first = input.add_node(node("sequence first", ShapeKind::Step));
        let sequence_hidden = input.add_node(node("sequence hidden", ShapeKind::Step));
        let cluster_first = input.add_node(node("cluster first", ShapeKind::SqlTable));
        let cluster_hidden = input.add_node(node("cluster hidden", ShapeKind::Oval));
        let mut graph = ArenaGraph::from_input(&input);
        let ordinary_tala_id = graph.nodes[ordinary.0 as usize].tala_id;
        let sequence_hidden_tala_id = graph.nodes[sequence_hidden.0 as usize].tala_id;
        let cluster_hidden_tala_id = graph.nodes[cluster_hidden.0 as usize].tala_id;
        graph.sequences.push(SequenceState {
            members: vec![sequence_first, sequence_hidden],
            vessel_tala_id: 51_000,
            container: None,
            has_edge_abductions: false,
        });
        graph.clusters.push(ClusterState {
            members: vec![cluster_first, cluster_hidden],
            arrangement: ClusterArrangement::Row,
            desired_arrangement: ClusterArrangement::Row,
            padding: 20.0,
            vessel_tala_id: 52_000,
            fixed_size: false,
        });
        graph.node_order = vec![ordinary, sequence_first, cluster_first];
        for (node, position) in [
            (ordinary, Point { x: 0.0, y: 0.0 }),
            (sequence_first, Point { x: 120.0, y: 0.0 }),
            (cluster_first, Point { x: 240.0, y: 0.0 }),
        ] {
            graph.set_position(node, position);
        }

        graph.initialize_projected_transaction_nodes_from_current_graph();
        let ids = graph
            .projected_transaction_nodes
            .iter()
            .map(|entry| entry.node.tala_id)
            .collect::<Vec<_>>();
        assert_eq!(ids, vec![ordinary_tala_id, 51_000, 52_000]);
        assert!(!ids.contains(&sequence_hidden_tala_id));
        assert!(!ids.contains(&cluster_hidden_tala_id));
        for vessel in &graph.projected_transaction_nodes[1..] {
            assert_eq!(vessel.node.shape, ShapeKind::Rectangle);
            assert_eq!(vessel.node.declared_size, None);
            assert_eq!(vessel.node.layout_margins, Insets::uniform(0.0));
            assert_eq!(vessel.node.loop_offsets, None);
            assert_eq!(vessel.node.fixed_top_left, None);
            assert_eq!(vessel.node.hierarchy.clone(), None);
            assert_eq!(vessel.node.desired_width, None);
            assert_eq!(vessel.node.desired_height, None);
            assert_eq!(vessel.node.label_size, None);
            assert_eq!(vessel.node.content_insets, Insets::uniform(0.0));
            assert_eq!(vessel.node.node_padding, Insets::uniform(0.0));
            assert_eq!(vessel.node.grid_rows, None);
            assert_eq!(vessel.node.grid_columns, None);
            assert_eq!(vessel.node.sequence, None);
            assert_eq!(vessel.node.cluster, None);
            assert!(vessel.node.scoring_is_aggregate_vessel);
        }
    }

    #[test]
    fn projected_transaction_carrier_survives_induced_graph_and_appends_restored_nodes() {
        let mut input = Graph::default();
        let first = input.add_node(node("first", ShapeKind::Rectangle));
        let second = input.add_node(node("second", ShapeKind::Rectangle));
        let restored = input.add_node(node("restored", ShapeKind::Rectangle));
        let mut graph = ArenaGraph::from_input(&input);
        let ids = graph
            .nodes
            .iter()
            .map(|node| node.tala_id)
            .collect::<Vec<_>>();
        graph.node_order = vec![first, second];
        graph.initialize_projected_transaction_nodes_from_current_graph();

        let (induced, _) = graph.induced_placement_subgraph(&[first]);
        assert_eq!(
            induced
                .projected_transaction_nodes
                .iter()
                .map(|entry| entry.node.tala_id)
                .collect::<Vec<_>>(),
            vec![ids[0], ids[1]],
            "SplitSubgraphs must not filter the owning Graph.Nodes carrier"
        );

        graph.node_order.push(restored);
        graph.append_restored_projected_transaction_nodes();
        graph.append_restored_projected_transaction_nodes();
        assert_eq!(
            graph
                .projected_transaction_nodes
                .iter()
                .map(|entry| entry.node.tala_id)
                .collect::<Vec<_>>(),
            ids,
            "restored tree nodes append once in the owning Graph.Nodes order"
        );
    }

    #[test]
    fn projected_transaction_state_resolves_live_shared_alias_before_fallback() {
        use crate::internal::model::ProjectedTransactionNode;

        let mut input = Graph::default();
        let local = input.add_node(node("local", ShapeKind::Rectangle));
        let mut graph = ArenaGraph::from_input(&input);
        graph.set_position(local, Point::default());
        graph.initialize_projected_transaction_nodes_from_current_graph();

        let mut fallback = graph.nodes[local.0 as usize].clone();
        fallback.input_id = NodeId(u32::MAX);
        fallback.tala_id = 61_000;
        fallback.position = Some(Point { x: 100.0, y: 0.0 });
        fallback.rect.origin = fallback.position.unwrap();
        graph
            .projected_transaction_nodes
            .push(ProjectedTransactionNode {
                structural_container_tala_id: None,
                structural_ancestor_tala_ids: Vec::new(),
                node: fallback.clone(),
                container_tala_id: None,
                ancestor_tala_ids: Vec::new(),
                child_tala_ids: Vec::new(),
                edges: Vec::new(),
            });
        let mut live_alias = fallback;
        live_alias.position = Some(Point { x: 250.0, y: 0.0 });
        live_alias.rect.origin = live_alias.position.unwrap();
        graph
            .transaction_external_container_children
            .insert(99, vec![live_alias]);

        let state = graph.projected_transaction_state();
        assert_eq!(
            state.original_positions[1],
            Some(Point { x: 250.0, y: 0.0 }),
            "GraphState must snapshot the live shared pointer, not its carrier fallback"
        );
        let live_alias = &mut graph
            .transaction_external_container_children
            .get_mut(&99)
            .unwrap()[0];
        live_alias.position = Some(Point { x: 300.0, y: 0.0 });
        live_alias.rect.origin = Point { x: 300.0, y: 0.0 };
        graph.refresh_projected_transaction_node_geometry();
        assert_eq!(
            graph.projected_transaction_nodes[1].node.position,
            Some(Point { x: 300.0, y: 0.0 })
        );
    }

    #[test]
    fn projected_transaction_preserves_parent_across_arena_reindexing() {
        let mut input = Graph::default();
        let parent = input.add_node(node("parent", ShapeKind::Rectangle));
        let child = input.add_node(node("child", ShapeKind::Rectangle));
        let mut graph = ArenaGraph::from_input(&input);
        graph.nodes[child.0 as usize].container = Some(parent);
        graph.set_position(child, Point { x: 128.0, y: 0.0 });
        graph.initialize_projected_transaction_nodes_from_current_graph();
        let parent_identity = graph.nodes[parent.0 as usize].tala_id;

        // An induced graph reuses index zero for the child. The snapshot's
        // source-arena parent index must not become a self-container check.
        let (mut induced, _) = graph.induced_placement_subgraph(&[child]);
        assert_eq!(
            induced.nodes[0].tala_id,
            graph.nodes[child.0 as usize].tala_id
        );
        assert_eq!(
            induced.projected_transaction_nodes[1].structural_container_tala_id,
            Some(parent_identity)
        );
        let prior = induced.projected_transaction_state();
        assert!(induced.projected_transaction_nodes_are_valid(&prior));
        induced.set_position(NodeId(0), Point { x: 128.0, y: -43.0 });
        assert!(induced.projected_transaction_nodes_are_valid(&prior));
    }

    #[test]
    fn projected_transaction_rejects_new_padded_overlap_with_global_node() {
        use crate::internal::model::ProjectedTransactionNode;

        let mut input = Graph::default();
        let local = input.add_node(node("local", ShapeKind::Rectangle));
        let mut graph = ArenaGraph::from_input(&input);
        graph.nodes[local.0 as usize].rect.size = Size {
            width: 50.0,
            height: 50.0,
        };
        graph.set_position(local, Point::default());
        graph.initialize_projected_transaction_nodes_from_current_graph();

        let mut global = graph.nodes[local.0 as usize].clone();
        global.input_id = NodeId(u32::MAX);
        global.tala_id = 62_000;
        global.position = Some(Point { x: 200.0, y: 0.0 });
        global.rect.origin = global.position.unwrap();
        graph
            .projected_transaction_nodes
            .push(ProjectedTransactionNode {
                structural_container_tala_id: None,
                structural_ancestor_tala_ids: Vec::new(),
                node: global,
                container_tala_id: None,
                ancestor_tala_ids: Vec::new(),
                child_tala_ids: Vec::new(),
                edges: Vec::new(),
            });
        let prior = graph.projected_transaction_state();
        assert!(prior.existing_overlaps.is_empty());

        graph.set_position(local, Point { x: 131.0, y: 0.0 });
        assert!(
            !graph.projected_transaction_nodes_are_valid(&prior),
            "Graph.IsBadState must reject a new padded overlap even while exact boxes remain disjoint"
        );
        assert!(
            !graph.projected_transaction_spacing_overlap_became_exact(&prior),
            "the rejection comes from IsBadState, not the separate exact-transition guard"
        );
    }

    #[test]
    fn recursive_scope_copy_preserves_cluster_member_offsets() {
        let mut input = Graph::default();
        let mut first_node = node("first", ShapeKind::Rectangle);
        first_node.size = Size {
            width: 80.0,
            height: 40.0,
        };
        let first = input.add_node(first_node);
        let mut second_node = node("second", ShapeKind::Rectangle);
        second_node.size = Size {
            width: 100.0,
            height: 80.0,
        };
        let second = input.add_node(second_node);
        let mut pipeline = Pipeline::new(&input, 1, false, false);
        pipeline.graph.clusters.push(ClusterState {
            members: vec![first, second],
            arrangement: ClusterArrangement::Column,
            desired_arrangement: ClusterArrangement::Column,
            padding: 20.0,
            vessel_tala_id: 17,
            fixed_size: false,
        });

        assert_eq!(
            pipeline.scope_aggregate_member_geometry(None, second),
            Some((
                first,
                Point { x: 0.0, y: 100.0 },
                Size {
                    width: 100.0,
                    height: 80.0,
                },
            ))
        );
    }

    #[test]
    fn sequence_connection_forces_row_and_icon_uses_maximum_label_width() {
        let mut input = Graph::default();
        let mut first_node = node("first", ShapeKind::Rectangle);
        first_node.size = Size {
            width: 145.0,
            height: 92.0,
        };
        first_node.has_icon = true;
        first_node.label_size = Some(Size {
            width: 74.0,
            height: 21.0,
        });
        let mut second_node = node("second", ShapeKind::Rectangle);
        second_node.size = Size {
            width: 122.0,
            height: 66.0,
        };
        second_node.label_size = Some(Size {
            width: 77.0,
            height: 21.0,
        });
        let mut third_node = node("third", ShapeKind::Rectangle);
        third_node.size = Size {
            width: 82.0,
            height: 66.0,
        };
        third_node.label_size = Some(Size {
            width: 37.0,
            height: 21.0,
        });
        let first = input.add_node(first_node);
        let second = input.add_node(second_node);
        let third = input.add_node(third_node);
        let step_one = input.add_node(node("step-one", ShapeKind::Step));
        let step_two = input.add_node(node("step-two", ShapeKind::Step));
        // BuildSequence permanently disconnects step-one -> step-two. Give
        // the three prospective members a second common neighbor so
        // PreprocessTrees does not correctly peel them as a branching tree
        // before this test reaches AddClusters.
        let anchor = input.add_node(node("anchor", ShapeKind::Oval));
        for (source, target) in [
            (step_one, step_two),
            (first, step_one),
            (second, step_one),
            (third, step_one),
            (first, anchor),
            (second, anchor),
            (third, anchor),
        ] {
            input.add_edge(Edge { source, target });
        }

        let mut graph = ArenaGraph::from_input(&input);
        let mut misc_rng = go_rng::GoRng::new(1);
        graph.assign_sequences(&mut misc_rng);
        graph.preprocess_trees();
        let neighbors = graph
            .nodes
            .iter()
            .map(|node| graph.unique_neighbors(node.input_id))
            .collect::<Vec<_>>();
        let signatures = graph
            .nodes
            .iter()
            .map(|node| graph.edge_signature(node.input_id))
            .collect::<Vec<_>>();
        assert!(graph.cluster_equivalent(first, second, &neighbors, &signatures));
        assert!(graph.nodes[step_one.0 as usize].sequence.is_some());
        graph.assign_clusters_with_rng(1, false, false, &mut misc_rng);

        assert_eq!(graph.clusters.len(), 1);
        assert_eq!(graph.clusters[0].members, vec![first, second, third]);
        assert_eq!(graph.clusters[0].arrangement, ClusterArrangement::Row);
        assert_eq!(graph.clusters[0].padding, 87.0);
        graph.resize_cluster_members(0);
        assert_eq!(
            graph.cluster_vessel_size(0),
            Size {
                width: 609.0,
                height: 92.0,
            }
        );
    }

    #[test]
    fn add_clusters_rejects_a_different_edge_direction_inventory() {
        let mut input = Graph::default();
        let first = input.add_node(node("first", ShapeKind::Rectangle));
        let second = input.add_node(node("second", ShapeKind::Rectangle));
        let pivot = input.add_node(node("pivot", ShapeKind::Rectangle));
        input.add_edge(Edge {
            source: first,
            target: pivot,
        });
        input.add_edge(Edge {
            source: pivot,
            target: second,
        });

        let mut graph = ArenaGraph::from_input(&input);
        graph.assign_clusters(1, false);

        assert!(graph.nodes[first.0 as usize].cluster.is_none());
        assert!(graph.nodes[second.0 as usize].cluster.is_none());
    }

    #[test]
    fn assign_arrangement_uses_recovered_average_dimension_rule() {
        let mut input = Graph::default();
        let mut tall = node("first", ShapeKind::Rectangle);
        tall.size = Size {
            width: 60.0,
            height: 120.0,
        };
        let first = input.add_node(tall.clone());
        tall.external_id = "second".to_owned();
        let second = input.add_node(tall);
        let pivot = input.add_node(node("pivot", ShapeKind::Rectangle));
        for (source, target) in [
            (first, pivot),
            (pivot, first),
            (second, pivot),
            (pivot, second),
        ] {
            input.add_edge(Edge { source, target });
        }

        let mut graph = ArenaGraph::from_input(&input);
        graph.assign_clusters(1, false);

        assert_eq!(graph.clusters.len(), 1);
        assert_eq!(graph.clusters[0].arrangement, ClusterArrangement::Row);
        assert_eq!(
            graph.clusters[0].desired_arrangement,
            ClusterArrangement::Row
        );
    }

    #[test]
    fn optimize_selects_row_for_external_nodes_above_and_below_a_cluster() {
        let mut input = Graph::default();
        let first = input.add_node(node("first", ShapeKind::Rectangle));
        let second = input.add_node(node("second", ShapeKind::Rectangle));
        let above = input.add_node(node("above", ShapeKind::Diamond));
        let below = input.add_node(node("below", ShapeKind::Hexagon));
        for (source, target) in [
            (above, first),
            (below, first),
            (above, second),
            (below, second),
        ] {
            input.add_edge(Edge { source, target });
        }
        let mut graph = ArenaGraph::from_input(&input);
        graph.assign_clusters(1, false);
        graph.nodes[first.0 as usize].position = Some(Point { x: 100.0, y: 200.0 });
        graph.nodes[second.0 as usize].position = Some(Point { x: 220.0, y: 200.0 });
        graph.nodes[above.0 as usize].position = Some(Point { x: 170.0, y: 0.0 });
        graph.nodes[below.0 as usize].position = Some(Point { x: 170.0, y: 400.0 });

        graph.update_cluster_desired_arrangements();

        assert_eq!(graph.clusters.len(), 1);
        assert_eq!(graph.clusters[0].arrangement, ClusterArrangement::Column);
        assert_eq!(
            graph.clusters[0].desired_arrangement,
            ClusterArrangement::Row
        );
    }

    #[test]
    fn optimize_commits_the_recovered_centered_cluster_flip() {
        let mut input = Graph::default();
        let first = input.add_node(node("first", ShapeKind::Rectangle));
        let second = input.add_node(node("second", ShapeKind::Rectangle));
        let above = input.add_node(node("above", ShapeKind::Diamond));
        let below = input.add_node(node("below", ShapeKind::Hexagon));
        for (source, target) in [
            (above, first),
            (below, first),
            (above, second),
            (below, second),
        ] {
            input.add_edge(Edge { source, target });
        }
        let mut graph = ArenaGraph::from_input(&input);
        for member in [first, second] {
            graph.nodes[member.0 as usize].cluster = Some(0);
        }
        graph.clusters.push(ClusterState {
            members: vec![first, second],
            arrangement: ClusterArrangement::Column,
            desired_arrangement: ClusterArrangement::Column,
            padding: 20.0,
            vessel_tala_id: 1,
            fixed_size: false,
        });
        graph.nodes[first.0 as usize].position = Some(Point { x: 100.0, y: 200.0 });
        graph.nodes[second.0 as usize].position = Some(Point { x: 100.0, y: 300.0 });
        graph.nodes[above.0 as usize].position = Some(Point { x: 100.0, y: 0.0 });
        graph.nodes[below.0 as usize].position = Some(Point { x: 100.0, y: 500.0 });

        assert!(graph.optimize_clusters());

        assert_eq!(graph.clusters[0].arrangement, ClusterArrangement::Row);
        assert_eq!(
            graph.clusters[0].desired_arrangement,
            ClusterArrangement::Row
        );
        assert_eq!(graph.position(first), Some(Point { x: 40.0, y: 200.0 }));
        assert_eq!(graph.position(second), Some(Point { x: 160.0, y: 200.0 }));
    }

    #[test]
    fn cluster_flip_fixed_snapshot_rejects_circle_container_sibling_move() {
        let mut input = Graph::default();
        let mut container_node = node("container", ShapeKind::Circle);
        container_node.size = Size {
            width: 948.0,
            height: 948.0,
        };
        let container = input.add_node(container_node);
        let mut member_node = node("first", ShapeKind::Rectangle);
        member_node.size.width = 60.0;
        member_node.parent = Some(container);
        let first = input.add_node(member_node.clone());
        member_node.external_id = "second".to_owned();
        let second = input.add_node(member_node);
        let mut peer_node = node("fixed-peer", ShapeKind::Rectangle);
        peer_node.parent = Some(container);
        // The already positioned container supplies the 60-unit inner frame.
        // FixedTopLeft remains the authored offset from that frame.
        peer_node.locked_position = Some(Point { x: 540.0, y: 0.0 });
        let peer = input.add_node(peer_node);
        let mut graph = ArenaGraph::from_input(&input);
        graph.cell_size = 20.0;
        assert!(graph.tree_routing_nodes.is_empty());
        graph.clusters.push(ClusterState {
            members: vec![first, second],
            arrangement: ClusterArrangement::Column,
            desired_arrangement: ClusterArrangement::Row,
            padding: 60.0,
            vessel_tala_id: 5,
            fixed_size: false,
        });
        for member in [first, second] {
            graph.nodes[member.0 as usize].cluster = Some(0);
        }
        graph.node_order.retain(|id| *id != second);
        graph.rebuild_active_aggregate_node_order();
        for (id, position) in [
            (
                container,
                Point {
                    x: -94.0,
                    y: -304.0,
                },
            ),
            (first, Point { x: 60.0, y: 60.0 }),
            (second, Point { x: 60.0, y: 200.0 }),
            (peer, Point { x: 600.0, y: 60.0 }),
        ] {
            graph.set_position(id, position);
        }
        graph
            .pending_cluster_vessel_positions
            .insert(0, Point { x: 60.0, y: 60.0 });
        let baseline = graph
            .nodes
            .iter()
            .map(|node| (node.position, node.rect.size))
            .collect::<Vec<_>>();
        graph.sync_clusters();
        graph.reposition_ordinary_containers();
        assert_eq!(
            graph
                .nodes
                .iter()
                .map(|node| (node.position, node.rect.size))
                .collect::<Vec<_>>(),
            baseline,
            "the independent Go baseline must be refit-stable"
        );
        assert!(graph.transaction_containment_is_valid());
        assert!(graph.transaction_fixed_origin_is_valid());
        // The snapshot records container/descendant box intersections too;
        // production overlap validation exempts those hierarchy relations.
        let baseline_overlaps = graph.existing_overlap_pairs();
        assert!(!graph.transaction_has_new_overlap(&baseline_overlaps));
        assert!(!graph.transaction_has_bad_state_overlap(&baseline_overlaps));

        let mut unfixed = graph.clone();
        unfixed.nodes[peer.0 as usize].fixed_top_left = None;
        let top_left = unfixed
            .cluster_flip_trial(0, false)
            .expect("Go accepts the unfixed top-left control");
        assert_eq!(top_left.position(container), graph.position(container));
        assert_eq!(
            top_left.nodes[container.0 as usize].rect.size,
            input.nodes[container.0 as usize].size
        );
        assert_eq!(top_left.position(first), Some(Point { x: 60.0, y: 130.0 }));
        assert_eq!(
            top_left.position(second),
            Some(Point { x: 140.0, y: 130.0 })
        );
        assert_eq!(top_left.position(peer), Some(Point { x: 600.0, y: 130.0 }));
        assert_eq!(top_left.clusters[0].arrangement, ClusterArrangement::Row);
        assert_eq!(top_left.clusters[0].padding, 20.0);
        let mut moved_fixed = top_left.clone();
        moved_fixed.nodes[peer.0 as usize].fixed_top_left =
            graph.nodes[peer.0 as usize].fixed_top_left;
        assert!(moved_fixed.transaction_fixed_origin_is_valid());
        assert_ne!(moved_fixed.position(peer), graph.position(peer));

        let mut centered = unfixed.clone();
        assert!(centered.optimize_cluster_only_flip(0));
        assert_eq!(centered.position(container), graph.position(container));
        assert_eq!(
            centered.nodes[container.0 as usize].rect.size,
            Size {
                width: 1005.0,
                height: 1005.0
            }
        );
        assert_eq!(centered.position(first), Some(Point { x: 69.0, y: 159.0 }));
        assert_eq!(
            centered.position(second),
            Some(Point { x: 149.0, y: 159.0 })
        );
        assert_eq!(centered.position(peer), Some(Point { x: 649.0, y: 159.0 }));

        assert!(
            graph.cluster_flip_trial(0, false).is_none(),
            "Go rejects the origin-valid top-left trial because the fixed sibling moved"
        );
        assert!(!graph.optimize_cluster_only_flip(0));
        assert_eq!(
            graph
                .nodes
                .iter()
                .map(|node| (node.position, node.rect.size))
                .collect::<Vec<_>>(),
            baseline
        );
        assert_eq!(graph.clusters[0].arrangement, ClusterArrangement::Column);
    }

    fn assert_cluster_flip_defers_pipeline_size_gate(member_width: f64, expected_width: f64) {
        // The pristine Go witness discovers this Column with AddClusters,
        // then optimizeCluster(ctx, true) commits every valid Row flip. The
        // pipeline checks its final stage bounds separately: 30,000 passes,
        // while 30,002 produces an OptimizeClusters dimension invariant.
        let mut input = Graph::default();
        let mut first_node = node("first", ShapeKind::Rectangle);
        first_node.size = Size {
            width: member_width,
            height: 60.0,
        };
        let first = input.add_node(first_node.clone());
        first_node.external_id = "second".to_owned();
        let second = input.add_node(first_node);
        let mut peer_node = node("peer", ShapeKind::Rectangle);
        peer_node.size = Size {
            width: 100.0,
            height: 100.0,
        };
        let peer = input.add_node(peer_node);
        for source in [first, second] {
            for _ in 0..2 {
                input.add_edge(Edge {
                    source,
                    target: peer,
                });
            }
        }
        let mut graph = ArenaGraph::from_input(&input);
        graph.set_position(
            first,
            Point {
                x: 10_000.0,
                y: 10_000.0,
            },
        );
        graph.set_position(
            second,
            Point {
                x: 10_000.0,
                y: 10_080.0,
            },
        );
        graph.set_position(
            peer,
            Point {
                x: 10_000.0 + member_width / 2.0 - 50.0,
                y: 25_000.0,
            },
        );
        graph.assign_clusters(1, false);
        assert_eq!(graph.clusters.len(), 1);
        assert_eq!(graph.clusters[0].members, [first, second]);
        assert_eq!(graph.clusters[0].arrangement, ClusterArrangement::Column);
        assert_eq!(
            graph.cluster_vessel_size(0),
            Size {
                width: member_width,
                height: 140.0
            }
        );
        assert_eq!(graph.clusters[0].padding, 20.0);
        assert!(graph.is_within_max_size());

        assert!(
            graph.optimize_cluster_only_flip(0),
            "Go commits this valid cluster flip; dimension overflow belongs to the subsequent pipeline stage gate"
        );
        assert_eq!(graph.clusters[0].arrangement, ClusterArrangement::Row);
        assert_eq!(
            graph.cluster_vessel_size(0),
            Size {
                width: expected_width,
                height: 60.0
            }
        );
        assert_eq!(graph.clusters[0].padding, 20.0);
        assert_eq!(
            graph.active_node_position(first),
            Some(Point {
                x: 10_000.0 - ((member_width + 20.0) / 2.0).round(),
                y: 10_000.0,
            })
        );
        assert_eq!(graph.is_within_max_size(), expected_width <= 30_000.0);
    }

    #[test]
    fn cluster_flip_defers_pipeline_size_gate_below_limit() {
        assert_cluster_flip_defers_pipeline_size_gate(14_989.0, 29_998.0);
    }

    #[test]
    fn cluster_flip_defers_pipeline_size_gate_at_limit() {
        assert_cluster_flip_defers_pipeline_size_gate(14_990.0, 30_000.0);
    }

    #[test]
    fn cluster_flip_defers_pipeline_size_gate_above_limit() {
        assert_cluster_flip_defers_pipeline_size_gate(14_991.0, 30_002.0);
    }

    #[test]
    fn cluster_alignment_fixed_snapshot_rejects_connected_container_child_move() {
        let mut input = Graph::default();
        let first = input.add_node(node("first", ShapeKind::Rectangle));
        let second = input.add_node(node("second", ShapeKind::Rectangle));
        let mut peer_node = node("connected-container", ShapeKind::Rectangle);
        peer_node.size = Size {
            width: 220.0,
            height: 200.0,
        };
        let peer = input.add_node(peer_node);
        let mut child_node = node("fixed-child", ShapeKind::Rectangle);
        child_node.parent = Some(peer);
        child_node.locked_position = Some(Point::default());
        let child = input.add_node(child_node);
        input.tala_id_overrides = vec![Some(1), Some(2), Some(3), Some(4)];
        // Equivalent parallel edges avoid eager fringe-tree extraction while
        // preserving the one external endpoint used by Go's alignment trial.
        for member in [first, second] {
            for _ in 0..2 {
                input.add_edge(Edge {
                    source: member,
                    target: peer,
                });
            }
        }
        let mut graph = ArenaGraph::from_input(&input);
        graph.cell_size = 20.0;
        assert!(graph.tree_routing_nodes.is_empty());
        graph.clusters.push(ClusterState {
            members: vec![first, second],
            arrangement: ClusterArrangement::Row,
            desired_arrangement: ClusterArrangement::Row,
            padding: 60.0,
            vessel_tala_id: 5,
            fixed_size: false,
        });
        for member in [first, second] {
            graph.nodes[member.0 as usize].cluster = Some(0);
        }
        // Go AddCluster removes both members and appends its distinct vessel.
        // The fixed ordinary child remains in Graph.Nodes after its parent.
        graph.node_order = vec![peer, child, first];
        graph.rebuild_active_aggregate_node_order();
        for (id, position) in [
            (first, Point::default()),
            (second, Point { x: 160.0, y: 0.0 }),
            (peer, Point { x: 110.0, y: 360.0 }),
            (child, Point { x: 170.0, y: 420.0 }),
        ] {
            graph.set_position(id, position);
        }
        graph
            .pending_cluster_vessel_positions
            .insert(0, Point::default());
        let baseline = assert_cluster_alignment_stable_baseline(&mut graph, &[peer, child, first]);
        assert_eq!(graph.cluster_external_nodes(0), vec![peer]);
        assert!(graph.graph_node_order().contains(&child));

        let mut unfixed = graph.clone();
        unfixed.nodes[child.0 as usize].fixed_top_left = None;
        let aligned = unfixed
            .cluster_alignment_trial(0, false)
            .expect("Go accepts the connected-container alignment without a fixed child");
        assert_eq!(aligned.position(peer), Some(Point { x: 20.0, y: 360.0 }));
        assert_eq!(aligned.position(child), Some(Point { x: 80.0, y: 420.0 }));
        assert_eq!(aligned.active_node_position(first), Some(Point::default()));
        assert_eq!(
            aligned.nodes[peer.0 as usize].rect.size,
            input.nodes[peer.0 as usize].size
        );
        let mut moved_fixed = aligned.clone();
        moved_fixed.nodes[child.0 as usize].fixed_top_left = Some(Point::default());
        assert!(moved_fixed.transaction_fixed_origin_is_valid());
        assert_ne!(moved_fixed.position(child), graph.position(child));

        assert!(
            graph.cluster_alignment_trial(0, false).is_none(),
            "Go rejects this origin-valid trial because the fixed descendant moved"
        );
        assert_eq!(
            graph
                .nodes
                .iter()
                .map(|node| (node.position, node.rect.size))
                .collect::<Vec<_>>(),
            baseline
        );
    }

    #[test]
    fn cluster_alignment_fixed_origin_rejects_active_vessel_move() {
        let mut input = Graph::default();
        let first = input.add_node(node("first", ShapeKind::Rectangle));
        let second = input.add_node(node("second", ShapeKind::Rectangle));
        let peer = input.add_node(node("connected-peer", ShapeKind::Rectangle));
        let mut anchor_node = node("fixed-anchor", ShapeKind::Rectangle);
        anchor_node.locked_position = Some(Point { x: 0.0, y: 1000.0 });
        let anchor = input.add_node(anchor_node);
        input.tala_id_overrides = vec![Some(1), Some(2), Some(3), Some(4)];
        for member in [first, second] {
            for _ in 0..2 {
                input.add_edge(Edge {
                    source: member,
                    target: peer,
                });
            }
        }
        let mut graph = ArenaGraph::from_input(&input);
        graph.cell_size = 20.0;
        assert!(graph.tree_routing_nodes.is_empty());
        graph.clusters.push(ClusterState {
            members: vec![first, second],
            arrangement: ClusterArrangement::Row,
            desired_arrangement: ClusterArrangement::Row,
            padding: 60.0,
            vessel_tala_id: 5,
            fixed_size: false,
        });
        for member in [first, second] {
            graph.nodes[member.0 as usize].cluster = Some(0);
        }
        graph.node_order = vec![peer, anchor, first];
        graph.rebuild_active_aggregate_node_order();
        for (id, position) in [
            (first, Point { x: 200.0, y: 0.0 }),
            (second, Point { x: 360.0, y: 0.0 }),
            (peer, Point { x: 100.0, y: 400.0 }),
            (anchor, Point { x: 60.0, y: 1000.0 }),
        ] {
            graph.set_position(id, position);
        }
        graph
            .pending_cluster_vessel_positions
            .insert(0, Point { x: 200.0, y: 0.0 });
        let baseline = assert_cluster_alignment_stable_baseline(&mut graph, &[peer, anchor, first]);
        assert_eq!(graph.cluster_external_nodes(0), vec![peer]);

        let mut unfixed = graph.clone();
        unfixed.nodes[anchor.0 as usize].fixed_top_left = None;
        let aligned = unfixed
            .cluster_alignment_trial(0, true)
            .expect("Go accepts the vessel alignment without a fixed origin");
        assert_eq!(
            aligned.active_node_position(first),
            Some(Point { x: 20.0, y: 0.0 })
        );
        assert_eq!(aligned.position(first), Some(Point { x: 20.0, y: 0.0 }));
        assert_eq!(aligned.position(second), Some(Point { x: 180.0, y: 0.0 }));
        assert_eq!(aligned.position(peer), graph.position(peer));
        assert_eq!(aligned.position(anchor), graph.position(anchor));
        let mut crossed_origin = aligned.clone();
        crossed_origin.nodes[anchor.0 as usize].fixed_top_left =
            graph.nodes[anchor.0 as usize].fixed_top_left;
        assert!(!crossed_origin.transaction_fixed_origin_is_valid());

        assert!(
            graph.cluster_alignment_trial(0, true).is_none(),
            "Go rejects the active vessel before the stationary anchor's fixed origin"
        );
        assert_eq!(
            graph
                .nodes
                .iter()
                .map(|node| (node.position, node.rect.size))
                .collect::<Vec<_>>(),
            baseline
        );
    }

    fn assert_cluster_alignment_stable_baseline(
        graph: &mut ArenaGraph,
        expected_node_order: &[NodeId],
    ) -> Vec<(Option<Point>, Size)> {
        let baseline = graph
            .nodes
            .iter()
            .map(|node| (node.position, node.rect.size))
            .collect::<Vec<_>>();
        for _ in 0..2 {
            graph.sync_clusters();
            graph.reposition_ordinary_containers();
            assert_eq!(
                graph
                    .nodes
                    .iter()
                    .map(|node| (node.position, node.rect.size))
                    .collect::<Vec<_>>(),
                baseline,
                "the independently accepted Go baseline must remain refit-stable"
            );
        }
        assert_eq!(graph.graph_node_order(), expected_node_order);
        assert!(graph.transaction_containment_is_valid());
        assert!(graph.transaction_fixed_origin_is_valid());
        let overlaps = graph.existing_overlap_pairs();
        assert!(!graph.transaction_has_new_overlap(&overlaps));
        assert!(!graph.transaction_has_bad_state_overlap(&overlaps));
        assert!(graph.is_within_max_size());
        baseline
    }

    #[test]
    fn centered_cluster_flip_moves_the_distinct_vessel_before_refitting_its_container() {
        let mut input = Graph::default();
        let mut container_node = node("container", ShapeKind::Rectangle);
        container_node.size = Size {
            width: 460.0,
            height: 300.0,
        };
        let container = input.add_node(container_node);

        let mut first_node = node("first", ShapeKind::Rectangle);
        first_node.parent = Some(container);
        let first = input.add_node(first_node);
        let mut second_node = node("second", ShapeKind::Rectangle);
        second_node.parent = Some(container);
        let second = input.add_node(second_node);
        let mut sibling_node = node("sibling", ShapeKind::Rectangle);
        sibling_node.parent = Some(container);
        let sibling = input.add_node(sibling_node);
        let mut descendant_node = node("descendant", ShapeKind::Rectangle);
        descendant_node.size = Size {
            width: 20.0,
            height: 20.0,
        };
        let descendant = input.add_node(descendant_node);

        let mut graph = ArenaGraph::from_input(&input);
        graph.clusters.push(ClusterState {
            members: vec![first, second],
            arrangement: ClusterArrangement::Row,
            desired_arrangement: ClusterArrangement::Column,
            padding: 20.0,
            vessel_tala_id: 10_001,
            fixed_size: false,
        });
        graph.nodes[first.0 as usize].cluster = Some(0);
        graph.nodes[second.0 as usize].cluster = Some(0);
        graph.set_position(container, Point::default());
        graph.set_position(first, Point { x: 60.0, y: 60.0 });
        graph.set_position(second, Point { x: 180.0, y: 60.0 });
        graph.set_position(sibling, Point { x: 300.0, y: 60.0 });
        graph.set_position(
            descendant,
            Point {
                x: 1000.0,
                y: 1000.0,
            },
        );
        graph
            .pending_cluster_vessel_positions
            .insert(0, Point { x: 60.0, y: 60.0 });
        graph.rebuild_active_aggregate_node_order();

        let mut centered = graph.clone();
        centered.containers.insert(Some(first), vec![descendant]);
        centered.translate_active_node_with_children(first, Point { x: 0.0, y: -25.0 });
        assert_eq!(
            centered.pending_cluster_vessel_positions.get(&0),
            Some(&Point { x: 60.0, y: 35.0 })
        );
        assert_eq!(centered.position(first), Some(Point { x: 60.0, y: 35.0 }));
        assert_eq!(centered.position(second), Some(Point { x: 180.0, y: 35.0 }));
        assert_eq!(
            centered.position(descendant),
            Some(Point {
                x: 1000.0,
                y: 975.0,
            }),
            "the vessel move must visit each retained descendant exactly once"
        );

        let flipped = graph
            .cluster_flip_trial(0, true)
            .expect("the centered flip should remain a valid transaction");

        assert_eq!(
            flipped.pending_cluster_vessel_positions.get(&0),
            Some(&Point { x: 60.0, y: 60.0 }),
            "centering moves the distinct vessel up before the anchored container refit moves it back"
        );
        assert_eq!(flipped.position(first), Some(Point { x: 60.0, y: 60.0 }));
        assert_eq!(flipped.position(second), Some(Point { x: 60.0, y: 160.0 }));
        assert_eq!(
            flipped.position(descendant),
            Some(Point {
                x: 1000.0,
                y: 1000.0,
            }),
            "an unrelated root must not follow the vessel"
        );
        assert_eq!(
            flipped.position(sibling),
            Some(Point { x: 300.0, y: 110.0 }),
            "the immediate-container refit must carry an unrelated sibling"
        );
        assert_eq!(flipped.position(container), Some(Point::default()));
    }

    #[test]
    fn optimize_preserves_the_requested_state_when_both_flip_trials_fail() {
        let mut input = Graph::default();
        let first = input.add_node(node("first", ShapeKind::Rectangle));
        let second = input.add_node(node("second", ShapeKind::Rectangle));
        let above = input.add_node(node("above", ShapeKind::Diamond));
        let below = input.add_node(node("below", ShapeKind::Hexagon));
        let obstacle = input.add_node(node("obstacle", ShapeKind::Cloud));
        for (source, target) in [
            (above, first),
            (below, first),
            (above, second),
            (below, second),
        ] {
            input.add_edge(Edge { source, target });
        }
        let mut graph = ArenaGraph::from_input(&input);
        for member in [first, second] {
            graph.nodes[member.0 as usize].cluster = Some(0);
        }
        graph.clusters.push(ClusterState {
            members: vec![first, second],
            arrangement: ClusterArrangement::Column,
            desired_arrangement: ClusterArrangement::Column,
            padding: 20.0,
            vessel_tala_id: 1,
            fixed_size: false,
        });
        graph.nodes[first.0 as usize].position = Some(Point { x: 100.0, y: 200.0 });
        graph.nodes[second.0 as usize].position = Some(Point { x: 100.0, y: 300.0 });
        graph.nodes[above.0 as usize].position = Some(Point { x: 100.0, y: 0.0 });
        graph.nodes[below.0 as usize].position = Some(Point { x: 100.0, y: 500.0 });
        graph.nodes[obstacle.0 as usize].position = Some(Point { x: 250.0, y: 200.0 });
        let original_positions = [graph.position(first), graph.position(second)];

        assert!(!graph.optimize_clusters());

        assert_eq!(graph.clusters[0].arrangement, ClusterArrangement::Column);
        assert_eq!(
            graph.clusters[0].desired_arrangement,
            ClusterArrangement::Row
        );
        assert_eq!(
            [graph.position(first), graph.position(second)],
            original_positions
        );
    }

    #[test]
    fn optimize_runs_alignment_when_the_arrangement_already_matches() {
        let mut input = Graph::default();
        let first = input.add_node(node("first", ShapeKind::Rectangle));
        let second = input.add_node(node("second", ShapeKind::Rectangle));
        let above = input.add_node(node("above", ShapeKind::Diamond));
        let below = input.add_node(node("below", ShapeKind::Hexagon));
        for (source, target) in [
            (above, first),
            (below, first),
            (above, second),
            (below, second),
        ] {
            input.add_edge(Edge { source, target });
        }
        let mut graph = ArenaGraph::from_input(&input);
        for member in [first, second] {
            graph.nodes[member.0 as usize].cluster = Some(0);
        }
        graph.clusters.push(ClusterState {
            members: vec![first, second],
            arrangement: ClusterArrangement::Row,
            desired_arrangement: ClusterArrangement::Row,
            padding: 20.0,
            vessel_tala_id: 1,
            fixed_size: false,
        });
        graph.nodes[first.0 as usize].position = Some(Point { x: 100.0, y: 200.0 });
        graph.nodes[second.0 as usize].position = Some(Point { x: 220.0, y: 200.0 });
        graph.nodes[above.0 as usize].position = Some(Point { x: 250.0, y: 0.0 });
        graph.nodes[below.0 as usize].position = Some(Point { x: 250.0, y: 400.0 });

        assert!(graph.optimize_clusters());

        // alignVessel is evaluated first. The equal alignConnectedNodes score
        // does not replace it because TALA requires a strict improvement.
        assert_eq!(graph.position(first), Some(Point { x: 190.0, y: 200.0 }));
        assert_eq!(graph.position(second), Some(Point { x: 310.0, y: 200.0 }));
        assert_eq!(graph.position(above), Some(Point { x: 250.0, y: 0.0 }));
        assert_eq!(graph.position(below), Some(Point { x: 250.0, y: 400.0 }));
    }

    #[test]
    fn cluster_abduction_retains_the_current_sequence_vessel_endpoint() {
        let mut input = Graph::default();
        let cluster_first = input.add_node(node("cluster first", ShapeKind::Rectangle));
        let cluster_second = input.add_node(node("cluster second", ShapeKind::Rectangle));
        let first_step = input.add_node(node("first step", ShapeKind::Step));
        let second_step = input.add_node(node("second step", ShapeKind::Step));
        input.add_edge(Edge {
            source: cluster_first,
            target: second_step,
        });

        let mut graph = ArenaGraph::from_input(&input);
        graph.sequences.push(SequenceState {
            members: vec![first_step, second_step],
            vessel_tala_id: 10_001,
            container: None,
            has_edge_abductions: true,
        });
        graph.clusters.push(ClusterState {
            members: vec![cluster_first, cluster_second],
            arrangement: ClusterArrangement::Row,
            desired_arrangement: ClusterArrangement::Row,
            padding: 20.0,
            vessel_tala_id: 10_002,
            fixed_size: false,
        });
        // The stable arena has already restored both sequence steps. The
        // retained endpoint still denotes TALA's temporary sequence vessel
        // during the cluster transaction.
        graph.node_order = vec![cluster_first, cluster_second, first_step, second_step];
        graph.set_position(first_step, Point { x: 90.0, y: 300.0 });
        graph.set_position(second_step, Point { x: 155.0, y: 300.0 });
        graph.set_position(cluster_first, Point { x: 100.0, y: 0.0 });
        graph.set_position(cluster_second, Point { x: 220.0, y: 0.0 });

        assert_eq!(graph.cluster_external_nodes(0), vec![first_step]);

        let aligned = graph
            .cluster_alignment_trial(0, false)
            .expect("moving the current sequence vessel is legal");
        assert_eq!(
            aligned.position(first_step),
            Some(Point { x: 128.0, y: 300.0 })
        );
        assert_eq!(
            aligned.position(second_step),
            Some(Point { x: 193.0, y: 300.0 })
        );
    }

    #[test]
    fn retained_sequence_vessel_rejects_cluster_alignment_overlap() {
        let mut input = Graph::default();
        let cluster_first = input.add_node(node("cluster first", ShapeKind::Rectangle));
        let cluster_second = input.add_node(node("cluster second", ShapeKind::Rectangle));
        let first_step = input.add_node(node("first step", ShapeKind::Step));
        let second_step = input.add_node(node("second step", ShapeKind::Step));
        let blocker = input.add_node(node("blocker", ShapeKind::Rectangle));
        input.add_edge(Edge {
            source: cluster_first,
            target: second_step,
        });

        let mut graph = ArenaGraph::from_input(&input);
        graph.sequences.push(SequenceState {
            members: vec![first_step, second_step],
            vessel_tala_id: 10_001,
            container: None,
            has_edge_abductions: true,
        });
        graph.clusters.push(ClusterState {
            members: vec![cluster_first, cluster_second],
            arrangement: ClusterArrangement::Row,
            desired_arrangement: ClusterArrangement::Row,
            padding: 20.0,
            vessel_tala_id: 10_002,
            fixed_size: false,
        });
        graph.node_order = vec![
            cluster_first,
            cluster_second,
            first_step,
            second_step,
            blocker,
        ];
        graph.set_position(first_step, Point { x: 90.0, y: 300.0 });
        graph.set_position(second_step, Point { x: 155.0, y: 300.0 });
        graph.set_position(cluster_first, Point { x: 100.0, y: 0.0 });
        graph.set_position(cluster_second, Point { x: 220.0, y: 0.0 });
        graph.set_position(blocker, Point { x: 280.0, y: 300.0 });

        assert!(
            graph.cluster_alignment_trial(0, false).is_none(),
            "the moved sequence vessel must collide even though its first stable member does not"
        );
    }

    #[test]
    fn cluster_vessel_gap_reduction_moves_the_connected_side_to_release_gap() {
        let mut input = Graph::default();
        let first = input.add_node(node("first", ShapeKind::Rectangle));
        let second = input.add_node(node("second", ShapeKind::Rectangle));
        let nearest = input.add_node(node("nearest", ShapeKind::Diamond));
        let tail = input.add_node(node("tail", ShapeKind::Hexagon));
        input.tala_id_overrides = vec![Some(1), Some(2), Some(3), Some(4)];
        input.add_edge(Edge {
            source: first,
            target: nearest,
        });
        input.add_edge(Edge {
            source: second,
            target: nearest,
        });
        input.add_edge(Edge {
            source: nearest,
            target: tail,
        });
        let mut graph = ArenaGraph::from_input(&input);
        graph.cell_size = 20.0;
        for member in [first, second] {
            graph.nodes[member.0 as usize].cluster = Some(0);
        }
        graph.clusters.push(ClusterState {
            members: vec![first, second],
            arrangement: ClusterArrangement::Column,
            desired_arrangement: ClusterArrangement::Column,
            padding: 20.0,
            vessel_tala_id: 5,
            fixed_size: false,
        });
        graph.node_order.retain(|node| *node != second);
        graph.rebuild_active_aggregate_node_order();
        graph.nodes[first.0 as usize].position = Some(Point { x: 0.0, y: 0.0 });
        graph.nodes[second.0 as usize].position = Some(Point { x: 0.0, y: 100.0 });
        graph.nodes[nearest.0 as usize].position = Some(Point { x: 400.0, y: 0.0 });
        graph.nodes[tail.0 as usize].position = Some(Point { x: 600.0, y: 0.0 });

        assert_eq!(
            graph.connected_node_views(
                ConnectedNode::Active(nearest),
                &[ConnectedNode::Active(first)],
            ),
            vec![ConnectedNode::Active(nearest), ConnectedNode::Active(tail)],
            "the active cluster vessel is excluded while live connected nodes retain their order"
        );
        assert_eq!(
            graph
                .global_sized_edge_length_with_direction(false)
                .to_bits(),
            0x4099_0027_6276_2762,
            "initial edge score matches the D2 v0.9.0 cluster-gap oracle"
        );
        assert!(graph.reduce_cluster_gap_to_neighbors(0, true, true, true));

        assert_eq!(graph.position(nearest), Some(Point { x: 250.0, y: 0.0 }));
        assert_eq!(graph.position(tail), Some(Point { x: 450.0, y: 0.0 }));
        assert_eq!(
            graph
                .global_sized_edge_length_with_direction(false)
                .to_bits(),
            0x408f_404e_c4ec_4ec4,
            "accepted edge score matches the D2 v0.9.0 cluster-gap oracle"
        );
    }

    #[test]
    fn resize_equalizes_non_fixed_cluster_member_dimensions() {
        let mut input = Graph::default();
        let first = input.add_node(node("first", ShapeKind::Rectangle));
        let second = input.add_node(node("second", ShapeKind::Rectangle));
        let mut graph = ArenaGraph::from_input(&input);
        graph.nodes[first.0 as usize].rect.size = Size {
            width: 369.0,
            height: 410.0,
        };
        graph.nodes[second.0 as usize].rect.size = Size {
            width: 398.0,
            height: 436.0,
        };
        for member in [first, second] {
            graph.nodes[member.0 as usize].cluster = Some(0);
        }
        graph.clusters.push(ClusterState {
            members: vec![first, second],
            arrangement: ClusterArrangement::Row,
            desired_arrangement: ClusterArrangement::Row,
            padding: 20.0,
            vessel_tala_id: 1,
            fixed_size: false,
        });

        graph.resize_clusters_containing(first);

        for member in [first, second] {
            assert_eq!(
                graph.nodes[member.0 as usize].rect.size,
                Size {
                    width: 398.0,
                    height: 436.0,
                }
            );
        }
    }

    #[test]
    fn resize_preserves_fixed_cluster_member_dimensions() {
        let mut input = Graph::default();
        let first = input.add_node(node("first", ShapeKind::Circle));
        let second = input.add_node(node("second", ShapeKind::Circle));
        let mut graph = ArenaGraph::from_input(&input);
        graph.nodes[first.0 as usize].rect.size = Size {
            width: 80.0,
            height: 80.0,
        };
        graph.nodes[second.0 as usize].rect.size = Size {
            width: 120.0,
            height: 120.0,
        };
        graph.clusters.push(ClusterState {
            members: vec![first, second],
            arrangement: ClusterArrangement::Column,
            desired_arrangement: ClusterArrangement::Column,
            padding: 20.0,
            vessel_tala_id: 1,
            fixed_size: true,
        });

        graph.resize_cluster_members(0);

        assert_eq!(
            graph.nodes[first.0 as usize].rect.size,
            Size {
                width: 80.0,
                height: 80.0,
            }
        );
        assert_eq!(
            graph.nodes[second.0 as usize].rect.size,
            Size {
                width: 120.0,
                height: 120.0,
            }
        );
    }

    // Original Go v0.9.0 actual AddClusters and OptimizeClusters prepared
    // controls. Full authored NodePlacement/CLI reachability is separate.
    fn prepared_cluster_alignment_max_fixture(x: f64) -> (Pipeline, [NodeId; 5]) {
        let mut input = Graph::default();
        let first = input.add_node(Node::new(
            "first",
            Size {
                width: 100.0,
                height: 101.0,
            },
        ));
        let second = input.add_node(Node::new(
            "second",
            Size {
                width: 100.0,
                height: 101.0,
            },
        ));
        let peer = input.add_node(Node::new(
            "wide-peer",
            Size {
                width: 29_900.0,
                height: 100.0,
            },
        ));
        let blocker = input.add_node(Node::new(
            "blocker",
            Size {
                width: 1.0,
                height: 101.0,
            },
        ));
        let mut anchor_node = Node::new(
            "fixed-anchor",
            Size {
                width: 100.0,
                height: 100.0,
            },
        );
        anchor_node.locked_position = Some(Point { x: 0.0, y: 2000.0 });
        let anchor = input.add_node(anchor_node);
        input.tala_id_overrides = vec![Some(1), Some(2), Some(3), Some(4), Some(5)];
        for member in [first, second] {
            for _ in 0..2 {
                input.add_edge(Edge {
                    source: member,
                    target: peer,
                });
            }
        }
        let mut pipeline = Pipeline::new_untraced(&input, 1, true, true);
        for (id, position) in [
            (first, Point { x, y: 1000.0 }),
            (
                second,
                Point {
                    x: x + 120.0,
                    y: 1000.0,
                },
            ),
            (
                peer,
                Point {
                    x: 10_000.0,
                    y: 0.0,
                },
            ),
            (
                blocker,
                Point {
                    x: 24_840.0,
                    y: 1000.0,
                },
            ),
            (
                anchor,
                Point {
                    x: 10_000.0,
                    y: 2000.0,
                },
            ),
        ] {
            pipeline.graph.set_position(id, position);
        }
        pipeline.graph.cell_size = 20.0;
        assert!(pipeline.graph.tree_nodes_in_extraction_order().is_empty());
        pipeline.graph.assign_clusters(1, false);
        assert_eq!(pipeline.graph.clusters.len(), 1);
        assert_eq!(pipeline.graph.clusters[0].members, [first, second]);
        assert_eq!(
            pipeline.graph.clusters[0].arrangement,
            ClusterArrangement::Row
        );
        assert_eq!(
            pipeline.graph.clusters[0].desired_arrangement,
            ClusterArrangement::Row
        );
        assert_eq!(pipeline.graph.clusters[0].padding, 20.0);
        assert_eq!(
            pipeline.graph.clusters[0].vessel_tala_id,
            5_577_006_791_947_779_410
        );
        assert_eq!(
            pipeline.graph.cluster_vessel_size(0),
            Size {
                width: 220.0,
                height: 101.0
            }
        );
        assert_eq!(pipeline.graph.cluster_external_nodes(0), [peer]);
        assert_cluster_alignment_stable_baseline(
            &mut pipeline.graph,
            &[peer, blocker, anchor, first],
        );
        // Original main-graph EdgeLength lazily materializes full costs.
        // The prepared native graph bypasses NodePlacement; preserve that
        // full surface before exact scores (half placement turn cost differs).
        pipeline.graph.initialize_combined_scoring_costs();
        (pipeline, [first, second, peer, blocker, anchor])
    }

    fn prepared_cluster_alignment_max_contract(x: f64, baseline_bits: u64) {
        let (mut pipeline, ids @ [first, second, peer, blocker, anchor]) =
            prepared_cluster_alignment_max_fixture(x);
        let geometry_bits = |graph: &ArenaGraph| {
            ids.map(|id| {
                let position = graph.position(id).unwrap();
                let size = graph.nodes[id.0 as usize].rect.size;
                [
                    position.x.to_bits(),
                    position.y.to_bits(),
                    size.width.to_bits(),
                    size.height.to_bits(),
                ]
            })
        };
        let before = geometry_bits(&pipeline.graph);
        let vessel_before = pipeline.graph.pending_cluster_vessel_positions.clone();
        let (tl, br) = pipeline.stage_graph_bounds().unwrap();
        assert_eq!((br.x - tl.x, br.y - tl.y), (29_900.0, 2100.0));
        assert!(
            pipeline
                .check_stage_dimensions("PreparedClusterAlignment")
                .is_ok()
        );
        assert_eq!(
            pipeline
                .graph
                .global_sized_edge_length_with_direction(true)
                .to_bits(),
            baseline_bits
        );

        // Go first tries moving the vessel onto x24840. The independent
        // ordinary blocker rejects that candidate, with exact rollback.
        assert!(pipeline.graph.cluster_alignment_trial(0, true).is_none());
        assert_eq!(geometry_bits(&pipeline.graph), before);
        assert_eq!(
            pipeline.graph.pending_cluster_vessel_positions,
            vessel_before
        );

        // The source then admits a peer move by99/100/101 even when the
        // completed graph width reaches30001. No fixed point is moved and
        // all current nodes remain beyond the anchor's fixed origin.
        let connected = pipeline
            .graph
            .cluster_alignment_trial(0, false)
            .expect("source admits alignment before the completed-stage size invariant");
        assert_eq!(
            connected.position(peer),
            Some(Point {
                x: x - 14_840.0,
                y: 0.0
            })
        );
        assert_eq!(connected.position(first), pipeline.graph.position(first));
        assert_eq!(connected.position(second), pipeline.graph.position(second));
        assert_eq!(
            connected.position(blocker),
            pipeline.graph.position(blocker)
        );
        assert_eq!(connected.position(anchor), pipeline.graph.position(anchor));
        assert!(connected.transaction_fixed_origin_is_valid());
        assert!(connected.transaction_containment_is_valid());
        assert_eq!(
            connected
                .global_sized_edge_length_with_direction(true)
                .to_bits(),
            0x40bc_2000_0000_0000
        );
        assert_eq!(
            geometry_bits(&pipeline.graph),
            before,
            "trial clone leaves baseline geometry exact"
        );
        assert_eq!(
            pipeline.graph.pending_cluster_vessel_positions,
            vessel_before
        );

        // Actual OptimizeClusters selects the connected-node candidate;
        // the subsequent forward vertical gap reduction is a real no-op
        // because the only connected peer is above this Row vessel.
        assert!(pipeline.graph.optimize_clusters());
        assert_eq!(
            pipeline.graph.position(peer),
            Some(Point {
                x: x - 14_840.0,
                y: 0.0
            })
        );
        assert_eq!(pipeline.graph.position(first), Some(Point { x, y: 1000.0 }));
        assert_eq!(
            pipeline.graph.position(second),
            Some(Point {
                x: x + 120.0,
                y: 1000.0
            })
        );
        assert_eq!(
            pipeline.graph.position(blocker),
            Some(Point {
                x: 24_840.0,
                y: 1000.0
            })
        );
        assert_eq!(
            pipeline.graph.position(anchor),
            Some(Point {
                x: 10_000.0,
                y: 2000.0
            })
        );
        let (tl, br) = pipeline.stage_graph_bounds().unwrap();
        assert_eq!((br.x - tl.x, br.y - tl.y), (x + 5060.0, 2100.0));
        if x <= 24_940.0 {
            assert!(pipeline.check_stage_dimensions("OptimizeClusters").is_ok());
        } else {
            let error = pipeline
                .check_stage_dimensions("OptimizeClusters")
                .unwrap_err();
            assert!(matches!(
                error,
                LayoutError::StageDimensions {
                    stage: "OptimizeClusters",
                    width: 30_001.0,
                    height: 2100.0
                }
            ));
            assert_eq!(
                error.to_string(),
                "layout invariant violated: Dimensions w:30001, h:2100 reached after stage OptimizeClusters"
            );
        }
    }

    #[test]
    fn cluster_alignment_trial_below_max_size_matches_source_boundary() {
        prepared_cluster_alignment_max_contract(24_939.0, 0x40bf_3800_5629_b1c0);
    }
    #[test]
    fn cluster_alignment_trial_equal_max_size_matches_source_boundary() {
        prepared_cluster_alignment_max_contract(24_940.0, 0x40bf_4000_5708_7fd4);
    }
    #[test]
    fn cluster_alignment_trial_above_max_size_matches_source_boundary() {
        prepared_cluster_alignment_max_contract(24_941.0, 0x40bf_4800_57e7_4de9);
    }
}
