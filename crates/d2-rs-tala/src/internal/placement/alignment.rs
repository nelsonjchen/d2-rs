// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Axis-alignment refinement after initial node placement.
//!
//! Candidate translations are trialed transactionally and kept only when the
//! recovered alignment score and precision ordering improve.

use super::model::ProjectedTransactionState;
use super::*;
use std::cmp::Ordering;

const ALIGNMENT_PRECISION: f64 = 0.0001;

fn alignment_precision_compare(left: f64, right: f64) -> Ordering {
    if (left - right).abs() < ALIGNMENT_PRECISION {
        Ordering::Equal
    } else {
        left.total_cmp(&right)
    }
}

/// State changed by an AlignAxes transaction trial.
///
/// TALA reuses one Transaction and rolls back the mutable node/projection
/// state after each candidate. Cloning ArenaGraph here also clones immutable
/// topology, ordering, scoring tables, and routing metadata; on a large
/// graph that cost dominates the actual alignment trial. Keep the rollback
/// surface explicit and leave the immutable arena shared in place.
#[derive(Clone)]
pub(super) struct AlignmentTrialSnapshot {
    geometry: super::trial_geometry::TrialGeometrySnapshot,
    // Read-only dense validation geometry retains the existing acceptance
    // boundary. Only the source-selected shared journal restores live state.
    nodes: Vec<super::owner_score_session::OwnerNodeGeometry>,
    existing_overlaps: BTreeSet<(NodeId, NodeId)>,
    existing_exact_overlaps: BTreeSet<(NodeId, NodeId)>,
    hierarchy_materialized: bool,
    projected_transaction_state: Option<ProjectedTransactionState>,
}

impl AlignmentTrialSnapshot {
    fn capture_with_overlap_state(
        graph: &ArenaGraph,
        existing_overlaps: BTreeSet<(NodeId, NodeId)>,
        existing_exact_overlaps: BTreeSet<(NodeId, NodeId)>,
    ) -> Self {
        let hierarchy_materialized = graph.transaction_containment_is_valid();
        let has_active_aggregate_vessel = graph
            .graph_node_order()
            .iter()
            .any(|node| graph.active_node_tala_id(*node) != graph.nodes[node.0 as usize].tala_id);
        let projected_transaction_state = (hierarchy_materialized && has_active_aggregate_vessel)
            .then(|| {
                let mut state = Self::projected_geometry(graph);
                // Go creates one GraphState when AlignAxes creates its reusable
                // transaction. Every speculative edge trial reuses that
                // state's existing-overlap exceptions after rollback. The
                // projected Rust carrier is rebuilt from live geometry, so
                // translate the stable NodeId pairs into its snapshot indices
                // instead of recomputing the baseline after each trial.
                // Lookup-only; collecting keeps the last index per Tala ID,
                // exactly as the ordered map did.
                let index_by_tala = state
                    .nodes
                    .iter()
                    .enumerate()
                    .map(|(index, entry)| (entry.node.tala_id, index))
                    .collect::<crate::internal::IdHashMap<_, _>>();
                state.existing_overlaps = existing_overlaps
                    .iter()
                    .filter_map(|(left, right)| {
                        let left = index_by_tala.get(&graph.active_node_tala_id(*left))?;
                        let right = index_by_tala.get(&graph.active_node_tala_id(*right))?;
                        Some(((*left).min(*right), (*left).max(*right)))
                    })
                    .collect();
                state.existing_exact_overlaps = existing_exact_overlaps
                    .iter()
                    .filter_map(|(left, right)| {
                        let left = index_by_tala.get(&graph.active_node_tala_id(*left))?;
                        let right = index_by_tala.get(&graph.active_node_tala_id(*right))?;
                        Some(((*left).min(*right), (*left).max(*right)))
                    })
                    .collect();
                state
            });

        Self {
            geometry: graph.trial_geometry_snapshot(),
            nodes: graph
                .nodes
                .iter()
                .map(super::owner_score_session::OwnerNodeGeometry::from_node)
                .collect(),
            // AlignAxes transactions use the same spacing predicate as the
            // normal transaction snapshot.  The vessel-edge variant omits
            // loop clearances and therefore fails to retain pre-existing
            // spacing overlaps (notably self-edge layouts) as exceptions.
            existing_overlaps,
            existing_exact_overlaps,
            hierarchy_materialized,
            projected_transaction_state,
        }
    }

    /// AlignAxes retains the transaction's initial overlap exceptions. A
    /// generic projected transaction would scan every pair here, only for
    /// capture_with_overlap_state to replace both overlap sets immediately.
    /// Retain the same node records and current original positions directly.
    fn projected_geometry(graph: &ArenaGraph) -> ProjectedTransactionState {
        let nodes = if graph.projected_transaction_nodes.is_empty() {
            graph
                .graph_node_order()
                .into_iter()
                .map(|node| graph.projected_transaction_node_from_current(node))
                .collect::<Vec<_>>()
        } else {
            graph.projected_transaction_nodes.clone()
        };
        // Match projected_transaction_geometry's first-match precedence:
        // active graph nodes, container children, aggregate children, then
        // external containers. Index only lookup results; traversal order
        // and duplicate Tala-ID winners remain unchanged.
        let mut positions = crate::internal::IdHashMap::default();
        for node in graph.graph_node_order() {
            positions
                .entry(graph.active_node_tala_id(node))
                .or_insert_with(|| graph.active_node_position(node));
        }
        for node in graph
            .transaction_external_container_children
            .values()
            .flatten()
            .chain(
                graph
                    .transaction_external_aggregate_children
                    .values()
                    .flatten(),
            )
            .chain(graph.transaction_external_containers.iter())
        {
            positions
                .entry(node.tala_id)
                .or_insert_with(|| graph.anchored_external_position(node));
        }
        let original_positions = nodes
            .iter()
            .map(|entry| {
                positions
                    .get(&entry.node.tala_id)
                    .copied()
                    .unwrap_or_else(|| graph.anchored_external_position(&entry.node))
            })
            .collect();
        ProjectedTransactionState {
            nodes,
            original_positions,
            existing_overlaps: BTreeSet::new(),
            existing_exact_overlaps: BTreeSet::new(),
        }
    }

    fn restore(&self, graph: &mut ArenaGraph) {
        graph.restore_trial_geometry_snapshot(self.geometry.clone());
    }
}

// TALA's Graph.intersectsOtherNode delegates to Node.passesThrough and the
// recovered segmentIntersectsBox helper. Keep that deliberately unusual
// endpoint perturbation and orientation test here instead of substituting a
// generic Liang-Barsky test: alignment trial acceptance depends on these
// boundary cases.
fn tala_orientation(p: Point, q: Point, r: Point) -> f64 {
    (q.y - p.y) * (r.x - q.x) - (q.x - p.x) * (r.y - q.y)
}

fn tala_equal_signs(a: f64, b: f64) -> bool {
    (a > 0.0 && b > 0.0) || (a == 0.0 && b == 0.0) || (a < 0.0 && b < 0.0)
}

fn tala_on_orthogonal_segment(p: Point, q: Point, r: Point) -> bool {
    r.x >= p.x.min(q.x) && r.x <= p.x.max(q.x) && r.y >= p.y.min(q.y) && r.y <= p.y.max(q.y)
}

fn tala_intersects(p1: Point, q1: Point, p2: Point, q2: Point) -> bool {
    let o1 = tala_orientation(p1, q1, p2);
    if o1 == 0.0 && tala_on_orthogonal_segment(p1, q1, p2) {
        return true;
    }
    let o2 = tala_orientation(p1, q1, q2);
    if o2 == 0.0 && tala_on_orthogonal_segment(p1, q1, q2) {
        return true;
    }
    let o3 = tala_orientation(p2, q2, p1);
    if o3 == 0.0 && tala_on_orthogonal_segment(p2, q2, p1) {
        return true;
    }
    let o4 = tala_orientation(p2, q2, q1);
    if o4 == 0.0 && tala_on_orthogonal_segment(p2, q2, q1) {
        return true;
    }
    !tala_equal_signs(o1, o2) && !tala_equal_signs(o3, o4)
}

fn tala_segment_intersects_box(first: Point, second: Point, position: Point, size: Size) -> bool {
    let left = position.x;
    let right = left + size.width;
    if first.x < second.x {
        if second.x < left || right < first.x {
            return false;
        }
    } else if first.x < left || right < second.x {
        return false;
    }

    let top = position.y;
    let bottom = top + size.height;
    if first.y < second.y {
        if second.y < top || bottom < first.y {
            return false;
        }
    } else if first.y < top || bottom < second.y {
        return false;
    }

    if left <= first.x && first.x <= right && top <= first.y && first.y <= bottom {
        return true;
    }
    if left <= second.x && second.x <= right && top <= second.y && second.y <= bottom {
        return true;
    }

    let top_left = position;
    let top_right = Point { x: right, y: top };
    let bottom_right = Point {
        x: right,
        y: bottom,
    };
    let bottom_left = Point { x: left, y: bottom };
    tala_intersects(
        first,
        Point {
            x: second.x,
            y: second.y - 1.0,
        },
        top_left,
        top_right,
    ) || tala_intersects(
        first,
        Point {
            x: second.x - 1.0,
            y: second.y,
        },
        top_left,
        bottom_left,
    ) || tala_intersects(
        first,
        Point {
            x: second.x + 1.0,
            y: second.y,
        },
        top_right,
        bottom_right,
    ) || tala_intersects(
        first,
        Point {
            x: second.x,
            y: second.y + 1.0,
        },
        bottom_left,
        bottom_right,
    )
}

impl ArenaGraph {
    /// Endpoint box used by recovered `Edge.getAlignmentDeltas`.
    ///
    /// The live edge remains connected to aggregate vessels, but this helper
    /// restores an endpoint through `Sequence.findAbductedNodeByEdge` when the
    /// current endpoint is still that sequence vessel. A later cluster
    /// abduction replaces the current endpoint again and therefore prevents
    /// the sequence restoration.
    fn alignment_member_endpoint_box(&self, endpoint: NodeId) -> (Point, Size) {
        // Cluster abduction reconnects the live edge to a temporary vessel in
        // Go. The stable arena retains the first member as the owner, but its
        // box is only one child of that vessel. Use the pending vessel frame
        // for alignment geometry so IsAxisAligned/GetAlignmentDeltas observe
        // the same aggregate endpoint as the oracle.
        if let Some(cluster_index) = self.active_cluster_index(endpoint) {
            let cluster = &self.clusters[cluster_index];
            let vessel_position = self
                .pending_cluster_vessel_positions
                .get(&cluster_index)
                .copied()
                .or_else(|| {
                    self.cluster_bounds(&cluster.members)
                        .map(|bounds| bounds.origin)
                })
                .expect("positioned cluster vessel");
            return (vessel_position, self.cluster_vessel_size(cluster_index));
        }
        if self.active_cluster_index(endpoint).is_none()
            && let Some(sequence_index) = self.active_sequence_index(endpoint)
        {
            let sequence = &self.sequences[sequence_index];
            let owner = sequence.members[0];
            let vessel_position = self
                .active_node_position(owner)
                .expect("positioned sequence vessel");
            let (offset, size) = self
                .sequence_member_geometry(sequence_index, endpoint)
                .expect("sequence endpoint belongs to its active vessel");
            return (
                Point {
                    x: vessel_position.x + offset.x,
                    y: vessel_position.y + offset.y,
                },
                size,
            );
        }

        let owner = self.active_aggregate_owner(endpoint);
        (
            self.active_node_position(owner)
                .expect("positioned alignment endpoint"),
            self.active_node_size(owner),
        )
    }

    /// Source Edge.IsAxisAligned and orientation tests use current vessel
    /// boxes; only getAlignmentDeltas restores original sequence steps.
    fn alignment_delta_endpoint_box(&self, endpoint: NodeId) -> (Point, Size) {
        if self.topology_node_vessel(endpoint).is_some() && self.node_order_contains(endpoint) {
            return (
                self.position(endpoint)
                    .expect("positioned live alignment vessel"),
                self.nodes[endpoint.0 as usize].rect.size,
            );
        }
        let carrier = match self.topology_node_vessel(endpoint) {
            Some(TopologyVessel::Sequence(index)) => self.sequences[index].members.first().copied(),
            Some(TopologyVessel::Cluster(index)) => self.clusters[index].members.first().copied(),
            None => None,
        }
        .unwrap_or_else(|| self.active_aggregate_owner(endpoint));
        (
            self.active_node_position(carrier)
                .expect("positioned alignment vessel"),
            self.active_node_size(carrier),
        )
    }

    fn alignment_original_endpoint_box(&self, edge: EdgeId, endpoint: NodeId) -> (Point, Size) {
        if let Some(TopologyVessel::Sequence(_)) = self.topology_node_vessel(endpoint) {
            let kind = super::owner_groups::OwnerGroupKind::Sequence;
            if let Some(group) = self
                .owner_graph_group_entry(OwnerGraphId(0), kind, Some(endpoint))
                .expect("alignment sequence map")
            {
                let group = group
                    .expect("runtime error: invalid memory address or nil pointer dereference");
                if let Some(header) = self
                    .owner_sequence_abductions(group)
                    .expect("alignment sequence records")
                {
                    for id in header.iter_slots() {
                        let record = self
                            .owner_edge_abduction(id.expect("alignment abduction pointer"))
                            .expect("alignment abduction record");
                        if record.source_edge != Some(edge) {
                            continue;
                        }
                        // Sequence.findAbductedNodeByEdge checks CurrentFrom
                        // before CurrentTo, independent of this call's side.
                        let original = if record.current_from == Some(endpoint) {
                            record.originally_from
                        } else if record.current_to == Some(endpoint) {
                            record.originally_to
                        } else {
                            continue;
                        };
                        return self.alignment_member_endpoint_box(
                            original.expect("alignment original step"),
                        );
                    }
                }
                panic!("runtime error: invalid memory address or nil pointer dereference");
            }
        }
        self.alignment_member_endpoint_box(endpoint)
    }

    pub(super) fn edge_axis_aligned(&self, edge: EdgeId) -> bool {
        let edge_id = edge;
        let edge = &self.edge_records[edge_id.0 as usize];
        let (from, from_size) = self.alignment_delta_endpoint_box(edge.from);
        let (to, to_size) = self.alignment_delta_endpoint_box(edge.to);
        if edge.has_table_column() {
            let facing =
                self.facing_table_ports_for_boxes(edge_id, (from, from_size), (to, to_size));
            return match (facing.source, facing.target) {
                (Some(source), Some(target)) => (source.y - target.y).abs() < 1.0,
                (Some(source), None) => (source.y - (to.y + to_size.height * 0.5)).abs() < 1.0,
                (None, Some(target)) => (target.y - (from.y + from_size.height * 0.5)).abs() < 1.0,
                (None, None) => {
                    (from.x - to.x).abs() < 1.0
                        || ((from.x + from_size.width) - (to.x + to_size.width)).abs() < 1.0
                }
            };
        }
        // `Edge.isAxisAligned` uses `geo.PrecisionCompare(..., 1)` for both
        // center coordinates, including after AffectContainers refits.
        ((from.y + from_size.height * 0.5) - (to.y + to_size.height * 0.5)).abs() < 1.0
            || ((from.x + from_size.width * 0.5) - (to.x + to_size.width * 0.5)).abs() < 1.0
    }

    pub(super) fn alignment_deltas(&self, edge: EdgeId) -> Point {
        let edge_id = edge;
        let edge = &self.edge_records[edge_id.0 as usize];
        let (from, from_size) = self.alignment_delta_endpoint_box(edge.from);
        let (to, to_size) = self.alignment_delta_endpoint_box(edge.to);
        if edge.has_table_column() {
            let facing =
                self.facing_table_ports_for_boxes(edge_id, (from, from_size), (to, to_size));
            return match (facing.source, facing.target) {
                (Some(source), Some(target)) => Point {
                    x: 0.0,
                    y: target.y - source.y,
                },
                (Some(source), None) => Point {
                    x: 0.0,
                    y: (to.y + to_size.height * 0.5) - source.y,
                },
                (None, Some(target)) => Point {
                    x: 0.0,
                    y: target.y - (from.y + from_size.height * 0.5),
                },
                (None, None) => Point {
                    x: to.x - from.x,
                    y: 0.0,
                },
            };
        }
        // `getAlignmentDeltas` restores a live sequence-vessel endpoint to the
        // original step for this edge. Its orientation/gap tests below still
        // use the current aggregate vessel boxes.
        let (from_delta_position, from_delta_size) =
            self.alignment_original_endpoint_box(edge_id, edge.from);
        let (to_delta_position, to_delta_size) =
            self.alignment_original_endpoint_box(edge_id, edge.to);
        let from_center = Point {
            x: from_delta_position.x + from_delta_size.width * 0.5,
            y: from_delta_position.y + from_delta_size.height * 0.5,
        };
        let to_center = Point {
            x: to_delta_position.x + to_delta_size.width * 0.5,
            y: to_delta_position.y + to_delta_size.height * 0.5,
        };
        let mut delta = Point {
            x: (to_center.x - from_center.x).round(),
            y: (to_center.y - from_center.y).round(),
        };
        let orientation = self.sized_box_orientation((from, from_size), (to, to_size));
        if matches!(orientation, Orientation::Left | Orientation::Right) {
            delta.x = 0.0;
        } else if matches!(orientation, Orientation::Top | Orientation::Bottom) {
            delta.y = 0.0;
        } else {
            let x_gap = if from.x + from_size.width < to.x {
                to.x - (from.x + from_size.width)
            } else if to.x + to_size.width < from.x {
                from.x - (to.x + to_size.width)
            } else {
                0.0
            };
            let y_gap = if from.y + from_size.height < to.y {
                to.y - (from.y + from_size.height)
            } else if to.y + to_size.height < from.y {
                from.y - (to.y + to_size.height)
            } else {
                0.0
            };
            if x_gap < 60.0 {
                delta.y = 0.0;
            }
            if y_gap < 60.0 {
                delta.x = 0.0;
            }
        }
        delta
    }

    /// Reflexive translation of TALA's `Node.isDescendentOf`.
    pub(super) fn is_descendant_of(&self, node: NodeId, ancestor: NodeId) -> bool {
        if node == ancestor {
            return true;
        }
        let mut container = self.nodes[node.0 as usize].container;
        while let Some(current) = container {
            if current == ancestor {
                return true;
            }
            container = self.nodes[current.0 as usize].container;
        }
        false
    }

    /// Translation of `Node.getConnectedNodes` over the current graph after
    /// sequence and cluster abduction.
    ///
    /// TALA's traversal is deliberately broader than graph connectivity: after
    /// following a node's edges it walks between ordinary containers and their
    /// children. Exclusions cut off both descendant branches and unrelated
    /// ancestors of an excluded branch, preventing an alignment transaction
    /// from moving the opposite side through a shared container.
    pub(super) fn connected_nodes_excluding(
        &self,
        start: NodeId,
        excluded: &BTreeSet<NodeId>,
    ) -> Vec<NodeId> {
        let excluded_list = excluded.iter().copied().collect::<Vec<_>>();
        self.connected_nodes_excluding_list(start, &excluded_list)
    }

    /// Variant used by AlignAxes, whose recovered Go caller maintains an
    /// exclusion slice rather than a set.  The slice intentionally preserves
    /// duplicate endpoint entries: when a tree/fixed node is also the edge
    /// endpoint, replacing the trailing slot does not remove the earlier
    /// exclusion, and getConnectedNodes therefore returns an empty opposite
    /// component.  A set cannot represent that source-visible state.
    pub(super) fn connected_nodes_excluding_list(
        &self,
        start: NodeId,
        excluded: &[NodeId],
    ) -> Vec<NodeId> {
        // Container ownership order is a property of the current graph
        // state, not of the BFS cursor.  The recovered traversal consults
        // that order repeatedly, but recomputing it for every visited node
        // makes large nested graphs spend minutes rebuilding the same
        // sequence/cluster projection.  Materialize it once for this
        // read-only traversal; the observable order and exclusion checks are
        // unchanged.
        let container_orders = self
            .node_order
            .iter()
            .copied()
            .filter(|node| self.nodes[node.0 as usize].is_container)
            .map(|container| (container, self.container_node_order(Some(container))))
            .collect::<Vec<_>>();
        // Graph.getConnectedNodes sees a synthetic cluster/sequence vessel,
        // while the arena retains the vessel's first member as its stable
        // representative.  Canonicalize the traversal carrier at every
        // enqueue so one active vessel is returned once, rather than once per
        // retained member.  This is observable: Go's alignment move list
        // contains vessel IDs, not all of the hidden member IDs.
        let canonical = |node: NodeId| self.active_aggregate_owner(node);
        let start = canonical(start);
        let excluded_set = excluded
            .iter()
            .copied()
            .map(canonical)
            .collect::<BTreeSet<_>>();
        let mut visited = BTreeSet::from([start]);
        let mut queue = VecDeque::from([start]);
        let mut result = Vec::new();
        while let Some(current) = queue.pop_front() {
            let crosses_excluded_hierarchy = excluded_set.iter().copied().any(|excluded_node| {
                self.active_is_descendant_of(current, excluded_node)
                    || (self.active_is_descendant_of(excluded_node, current)
                        && !self.active_is_descendant_of(start, current))
            });
            if crosses_excluded_hierarchy {
                continue;
            }

            result.push(current);

            // The recovered Go routine follows curr.Edges only. Near
            // constraints are intentionally not graph connectivity here.
            // `ConnectedNodeSet` iterates `current.Edges`, not `Graph.Edges`.
            // Temporary placement scopes can narrow Graph.Edges while shared
            // node pointers retain their full incidence slices; alignment
            // still follows those pointer-visible edges. Aggregate vessels
            // are synthesized in the arena, so reconstruct their incidence.
            let edge_ids = if self.active_aggregate_members(current).is_some() {
                self.active_edge_ids(current)
                    .into_iter()
                    .collect::<Vec<_>>()
            } else {
                self.nodes[current.0 as usize].edges.read()
            };
            for edge in edge_ids {
                let adjacent = canonical(self.active_adjacent(current, edge));
                if !excluded_set.contains(&adjacent) && visited.insert(adjacent) {
                    queue.push_back(adjacent);
                }
            }

            // Go's getConnectedNodes has a distinct cluster-vessel branch:
            // it walks children of each container member, rather than adding
            // the retained members themselves to the result.  The stable
            // arena uses the first member as the vessel representative.
            if let Some(cluster_index) = self.active_cluster_index(current)
                && self.active_aggregate_owner(current) == current
            {
                for member in self.clusters[cluster_index].members.iter().copied() {
                    if !self.nodes[member.0 as usize].is_container {
                        continue;
                    }
                    for child in self.container_node_order(Some(member)) {
                        let child = canonical(child);
                        if !excluded_set.contains(&child) && visited.insert(child) {
                            queue.push_back(child);
                        }
                    }
                }
            }

            // The inverse cluster relationship is also explicit in Go: a
            // child of a cluster member queues the vessel, not the member.
            for cluster in &self.clusters {
                if !self.cluster_is_active(cluster) {
                    continue;
                }
                let is_cluster_child = cluster.members.iter().copied().any(|member| {
                    self.nodes[member.0 as usize].is_container
                        && self.container_node_order(Some(member)).contains(&current)
                });
                if is_cluster_child
                    && let Some(vessel) = cluster.members.first().copied()
                    && !excluded_set.contains(&vessel)
                    && visited.insert(vessel)
                {
                    queue.push_back(vessel);
                }
            }

            // Go ranges over every ordinary container, checks whether the
            // current node is one of its direct children, and avoids crossing
            // that relationship when any direct child is explicitly excluded.
            for (container, children) in &container_orders {
                if excluded_set.contains(container) {
                    continue;
                }
                let excluded_child = children.iter().any(|child| excluded_set.contains(child));
                if excluded_child {
                    continue;
                }
                if children
                    .iter()
                    .copied()
                    .map(canonical)
                    .any(|child| child == current)
                    && visited.insert(*container)
                {
                    queue.push_back(*container);
                }
                if *container == current {
                    for child in children.iter().copied().map(canonical) {
                        if visited.insert(child) {
                            queue.push_back(child);
                        }
                    }
                }
            }
        }
        result
    }

    pub(super) fn center(&self, node: NodeId) -> Point {
        // Aggregate carriers are retained member IDs in the stable arena;
        // their live TALA vessel position is kept in the pending carrier map.
        // Graph.intersectsOtherNode reads the vessel box, so use the active
        // position rather than the member's stale arranged top-left.
        let position = self.active_node_position(node).unwrap();
        let size = self.active_node_size(node);
        Point {
            x: position.x + size.width * 0.5,
            y: position.y + size.height * 0.5,
        }
    }

    pub(super) fn segment_intersects_rect(&self, start: Point, end: Point, node: NodeId) -> bool {
        // Cluster vessels are the one case where TALA's live synthetic box
        // and its endpoint perturbation matter. Ordinary nodes retain the
        // historical rectangle test; applying the vessel predicate to them
        // changes boundary-touch routing decisions in large flat graphs.
        if let Some(cluster_index) = self.active_cluster_index(node)
            && self
                .pending_cluster_vessel_positions
                .contains_key(&cluster_index)
        {
            let owner = self.active_aggregate_owner(node);
            let position = self.active_node_position(owner).unwrap();
            let size = self.active_node_size(owner);
            return tala_segment_intersects_box(start, end, position, size);
        }
        let position = self.position(node).unwrap();
        let size = self.active_node_size(node);
        let mut t_min: f64 = 0.0;
        let mut t_max: f64 = 1.0;
        for (origin, direction, low, high) in [
            (
                start.x,
                end.x - start.x,
                position.x,
                position.x + size.width,
            ),
            (
                start.y,
                end.y - start.y,
                position.y,
                position.y + size.height,
            ),
        ] {
            if direction.abs() <= 1e-9 {
                if origin < low || origin > high {
                    return false;
                }
                continue;
            }
            let first = (low - origin) / direction;
            let second = (high - origin) / direction;
            t_min = t_min.max(first.min(second));
            t_max = t_max.min(first.max(second));
            if t_min > t_max {
                return false;
            }
        }
        true
    }

    pub(super) fn aligning_edge_intersects_other_node(&self, edge: EdgeId) -> bool {
        let edge_ref = &self.edge_records[edge.0 as usize];
        let from = self.active_aggregate_owner(edge_ref.from);
        let to = self.active_aggregate_owner(edge_ref.to);
        let start = self.center(from);
        let end = self.center(to);
        let trace = crate::internal::trace_env_value("TALA_TRACE_ALIGN_INTERSECTIONS")
            .map(|target| {
                let mut ids = target.split('>');
                let left = ids.next().and_then(|id| id.parse::<i64>().ok());
                let right = ids.next().and_then(|id| id.parse::<i64>().ok());
                [
                    self.nodes[edge_ref.from.0 as usize].tala_id,
                    self.nodes[edge_ref.to.0 as usize].tala_id,
                ] == [left.unwrap_or(0), right.unwrap_or(0)]
                    || [
                        self.nodes[edge_ref.from.0 as usize].tala_id,
                        self.nodes[edge_ref.to.0 as usize].tala_id,
                    ] == [right.unwrap_or(0), left.unwrap_or(0)]
            })
            .unwrap_or(false);
        if trace {
            eprintln!(
                "ALIGN_INTERSECTIONS_RUST edge={}>{} active={}>{} start={},{} end={},{}",
                self.nodes[edge_ref.from.0 as usize].tala_id,
                self.nodes[edge_ref.to.0 as usize].tala_id,
                self.nodes[from.0 as usize].tala_id,
                self.nodes[to.0 as usize].tala_id,
                start.x,
                start.y,
                end.x,
                end.y
            );
        }
        let use_active_aggregates = !self.pending_cluster_vessel_positions.is_empty();
        let mut seen = BTreeSet::new();
        self.node_order.iter().copied().any(|raw_node| {
            let node = if use_active_aggregates {
                self.active_aggregate_owner(raw_node)
            } else {
                raw_node
            };
            if use_active_aggregates && !seen.insert(node) {
                return false;
            }
            let eligible = node != from
                && node != to
                && if use_active_aggregates {
                !self.active_is_descendant_of_vessel(from, node)
                        && !self.active_is_descendant_of_vessel(to, node)
                        && !self.active_is_descendant_of_vessel(node, from)
                        && !self.active_is_descendant_of_vessel(node, to)
                        && self.active_node_position(node).is_some()
                } else {
                    !self.is_descendant_of(from, node)
                        && !self.is_descendant_of(to, node)
                        && !self.is_descendant_of(node, from)
                        && !self.is_descendant_of(node, to)
                        && self.position(node).is_some()
                };
            let hit = eligible && self.segment_intersects_rect(start, end, node);
            if trace {
                eprintln!(
                    "ALIGN_INTERSECTION_NODE_RUST raw={} owner={} eligible={} hit={} pos={:?} size={:?} container={:?}",
                    self.nodes[node.0 as usize].tala_id,
                    self.nodes[node.0 as usize].tala_id,
                    eligible,
                    hit,
                    self.position(node),
                    self.active_node_size(node),
                    self.nodes[raw_node.0 as usize]
                        .container
                        .map(|parent| self.nodes[parent.0 as usize].tala_id),
                );
            }
            hit
        })
    }

    pub(super) fn flat_aligning_edge_intersects_other_node(&self, edge: EdgeId) -> bool {
        let edge_ref = &self.edge_records[edge.0 as usize];
        let from = self.active_aggregate_owner(edge_ref.from);
        let to = self.active_aggregate_owner(edge_ref.to);
        let start = self.center(from);
        let end = self.center(to);
        let mut seen = BTreeSet::new();
        self.node_order.iter().copied().any(|raw_node| {
            let node = self.active_aggregate_owner(raw_node);
            if !seen.insert(node) {
                return false;
            }
            node != from
                && node != to
                && self.active_node_position(node).is_some()
                && self.segment_intersects_rect(start, end, node)
        })
    }

    pub(super) fn try_alignment_move(
        &mut self,
        edge: EdgeId,
        nodes: &[ConnectedNode],
        delta: Point,
        original_state: &AlignmentTrialSnapshot,
    ) -> Option<(f64, Point)> {
        #[cfg(feature = "diagnostic-traces")]
        crate::internal::call_counts::count!(AlignmentAttempt);
        let attempts = [
            (alignment_precision_compare(delta.y, 0.0) != Ordering::Equal)
                .then_some(Point { x: 0.0, y: delta.y }),
            (alignment_precision_compare(delta.x, 0.0) != Ordering::Equal)
                .then_some(Point { x: delta.x, y: 0.0 }),
        ];
        // Go's tryMove commits each candidate through Transaction and then
        // rolls the whole transaction back. Restoring only the visible node
        // boxes is insufficient here: container wrapping and aggregate sync
        // also mutate projected children, vessel layouts, abduction caches,
        // and derived transaction carriers. Keep one focused mutable-state
        // snapshot so the second orthogonal attempt starts from precisely the
        // same state as the first (and as Go's Rollback), without cloning the
        // immutable arena topology for every trial.
        // GraphState snapshots `Node.doesOverlap`, whose getDeltaTo includes
        // connected-edge minimum spacing.  Alignment trials must reuse that
        // same connected-vessel baseline; the generic non-transaction helper
        // omits the 60px connected floor and makes a candidate appear newly
        // invalid after an aggregate endpoint moves.
        if crate::internal::trace_env_enabled("TALA_TRACE_EXISTING_OVERLAP_PAIR") {
            let pair = self
                .nodes
                .iter()
                .find(|node| node.tala_id == 370_564_473)
                .and_then(|left| {
                    self.nodes
                        .iter()
                        .find(|node| node.tala_id == 739_672_091)
                        .map(|right| {
                            let pair = if left.input_id < right.input_id {
                                (left.input_id, right.input_id)
                            } else {
                                (right.input_id, left.input_id)
                            };
                            (
                                left,
                                right,
                                original_state.existing_overlaps.contains(&pair),
                                original_state.existing_exact_overlaps.contains(&pair),
                            )
                        })
                });
            if let Some((left, right, spacing, exact)) = pair {
                eprintln!(
                    "EXISTING_OVERLAP_RUST left={} box={:?}/{:?} right={} box={:?}/{:?} spacing={} exact={}",
                    left.tala_id,
                    left.position,
                    left.rect.size,
                    right.tala_id,
                    right.position,
                    right.rect.size,
                    spacing,
                    exact
                );
            }
        }
        // TALA updates one transaction state and reuses it for both endpoint
        // directions. AlignmentTrialSnapshot owns that same immutable
        // validation baseline; rebuilding it here doubled the pair scans.
        let hierarchy_materialized = original_state.hierarchy_materialized;
        let mut best: Option<(f64, Point)> = None;
        for attempt in attempts.into_iter().flatten() {
            if crate::internal::trace_env_enabled("TALA_TRACE_ALIGN_TARGET_NODES") {
                let edge_ref = &self.edge_records[edge.0 as usize];
                let ids = [
                    self.nodes[edge_ref.from.0 as usize].tala_id,
                    self.nodes[edge_ref.to.0 as usize].tala_id,
                ];
                if ids == [727019916, 1433953299] || ids == [1433953299, 727019916] {
                    eprint!(
                        "ALIGN_TARGET_RUST edge={}>{} attempt={},{} nodes=",
                        self.active_node_tala_id(edge_ref.from),
                        self.active_node_tala_id(edge_ref.to),
                        attempt.x,
                        attempt.y
                    );
                    for &node in nodes {
                        eprint!(
                            "{}(view={:?} box={:?} container={:?}),",
                            self.connected_node_id(node),
                            node,
                            self.connected_node_box(node),
                            self.connected_owning_container(node)
                                .map(|parent| self.active_node_tala_id(parent)),
                        );
                    }
                    eprintln!();
                }
            }
            self.translate_connected_node_boxes(nodes, attempt);
            if crate::internal::trace_env_enabled("TALA_TRACE_ALIGN_CLUSTER_STATE") {
                eprintln!(
                    "ALIGN_CLUSTER_STATE_RUST phase=translated edge={} attempt={},{} members={:?}",
                    edge.0,
                    attempt.x,
                    attempt.y,
                    self.clusters
                        .iter()
                        .filter(|cluster| self.cluster_is_active(cluster))
                        .flat_map(|cluster| cluster.members.iter().map(|member| (
                            member.0,
                            self.nodes[member.0 as usize].tala_id,
                            self.position(*member)
                        )))
                        .collect::<Vec<_>>()
                );
            }
            // TALA's attemptShift performs the edge-intersection rejection
            // before Transaction.Commit refits any containers.  Keep that
            // ordering: a refit can move a vessel away from the candidate
            // segment, but it must not turn an already-illegal shift into a
            // legal one.
            let intersects_before_reposition = self.aligning_edge_intersects_other_node(edge);
            let within_max_size_before_reposition = self.is_within_max_size();
            if crate::internal::trace_env_enabled("TALA_TRACE_ALIGN_CHILD_STATE")
                && attempt.x == 0.0
                && attempt.y == 560.0
            {
                let edge_ref = &self.edge_records[edge.0 as usize];
                let ids = [
                    self.nodes[edge_ref.from.0 as usize].tala_id,
                    self.nodes[edge_ref.to.0 as usize].tala_id,
                ];
                if ids == [727019916, 1433953299] || ids == [1433953299, 727019916] {
                    let container = self
                        .nodes
                        .iter()
                        .find(|node| node.tala_id == 739672091)
                        .map(|node| node.input_id);
                    if let Some(container) = container {
                        let children = self.container_node_order(Some(container));
                        eprintln!(
                            "ALIGN_CHILD_STATE_RUST phase=translated container=739672091 pos={:?} size={:?} children={:?}",
                            self.position(container),
                            self.nodes[container.0 as usize].rect.size,
                            children
                                .iter()
                                .map(|child| {
                                    let node = &self.nodes[child.0 as usize];
                                    (node.tala_id, node.position, node.rect.size)
                                })
                                .collect::<Vec<_>>()
                        );
                    }
                }
            }
            // Source attemptShift returns ErrInvalidCandidate before Commit
            // refits or validates containers. Once either check fails, the
            // materialized-hierarchy branch below cannot accept this attempt.
            // Keep detailed diagnostics on their original full-validation path.
            if !cfg!(feature = "diagnostic-traces")
                && hierarchy_materialized
                && (intersects_before_reposition || !within_max_size_before_reposition)
            {
                original_state.restore(self);
                continue;
            }
            let valid = if hierarchy_materialized {
                // Transaction.Commit validates refitted ordinary containers
                // before synchronizing cluster/sequence aggregates.  Keep
                // that phase boundary here; syncing first can restore stale
                // vessel geometry and make an illegal candidate appear valid.
                self.reposition_ordinary_containers_without_sync();
                if crate::internal::trace_env_enabled("TALA_TRACE_ALIGN_CLUSTER_STATE") {
                    eprintln!(
                        "ALIGN_CLUSTER_STATE_RUST phase=repositioned edge={} attempt={},{} members={:?}",
                        edge.0,
                        attempt.x,
                        attempt.y,
                        self.clusters
                            .iter()
                            .filter(|cluster| self.cluster_is_active(cluster))
                            .flat_map(|cluster| cluster.members.iter().map(|member| (
                                member.0,
                                self.nodes[member.0 as usize].tala_id,
                                self.position(*member)
                            )))
                            .collect::<Vec<_>>()
                    );
                }
                if crate::internal::trace_env_enabled("TALA_TRACE_ALIGN_GEOMETRY") {
                    eprint!(
                        "ALIGN_GEOMETRY_RUST attempt={},{} containers=",
                        attempt.x, attempt.y
                    );
                    for node in self
                        .nodes
                        .iter()
                        .filter(|node| node.is_container && node.position.is_some())
                    {
                        let position = node.position.expect("positioned container");
                        eprint!(
                            "{}:{},{} / {},{};",
                            node.tala_id,
                            position.x,
                            position.y,
                            node.rect.size.width,
                            node.rect.size.height
                        );
                    }
                    eprintln!();
                }
                if crate::internal::trace_env_enabled("TALA_TRACE_ALIGN_CHILD_STATE")
                    && attempt.x == 0.0
                    && attempt.y == 560.0
                {
                    let edge_ref = &self.edge_records[edge.0 as usize];
                    let ids = [
                        self.nodes[edge_ref.from.0 as usize].tala_id,
                        self.nodes[edge_ref.to.0 as usize].tala_id,
                    ];
                    if ids == [727019916, 1433953299] || ids == [1433953299, 727019916] {
                        let container = self
                            .nodes
                            .iter()
                            .find(|node| node.tala_id == 739672091)
                            .map(|node| node.input_id);
                        if let Some(container) = container {
                            let children = self.container_node_order(Some(container));
                            eprintln!(
                                "ALIGN_CHILD_STATE_RUST phase=repositioned container=739672091 pos={:?} size={:?} children={:?}",
                                self.position(container),
                                self.nodes[container.0 as usize].rect.size,
                                children
                                    .iter()
                                    .map(|child| {
                                        let node = &self.nodes[child.0 as usize];
                                        (node.tala_id, node.position, node.rect.size)
                                    })
                                    .collect::<Vec<_>>()
                            );
                        }
                    }
                }
                if crate::internal::trace_env_enabled("TALA_TRACE_REPOSITION") {
                    for traced in self
                        .nodes
                        .iter()
                        .filter(|node| matches!(node.tala_id, 370564473 | 739672091))
                    {
                        eprintln!(
                            "REPOSITION_RUST node={} pos={:?} size={:?}",
                            traced.tala_id, traced.position, traced.rect.size
                        );
                    }
                }
                let intersects = intersects_before_reposition;
                let existing_became_exact = original_state
                    .projected_transaction_state
                    .as_ref()
                    .map(|state| self.projected_transaction_spacing_overlap_became_exact(state))
                    .unwrap_or_else(|| {
                        self.existing_spacing_overlap_became_exact(
                            &original_state.existing_overlaps,
                            &original_state.existing_exact_overlaps,
                        )
                    });
                let fixed_moved = self.nodes.iter().enumerate().any(|(index, node)| {
                    node.fixed_top_left.is_some()
                        && node.position != original_state.nodes[index].position
                });
                let fixed_origin_valid = self.transaction_fixed_origin_is_valid();
                let external_containers_valid = self.transaction_external_containers_are_valid();
                let containment_valid = self.transaction_containment_is_valid();
                let pre_sync_container_valid = external_containers_valid && containment_valid;
                if pre_sync_container_valid {
                    self.sync_clusters();
                    self.sync_sequences();
                    if crate::internal::trace_env_enabled("TALA_TRACE_ALIGN_CLUSTER_STATE") {
                        eprintln!(
                            "ALIGN_CLUSTER_STATE_RUST phase=synced edge={} attempt={},{} members={:?}",
                            edge.0,
                            attempt.x,
                            attempt.y,
                            self.clusters
                                .iter()
                                .filter(|cluster| self.cluster_is_active(cluster))
                                .flat_map(|cluster| cluster.members.iter().map(|member| (
                                    member.0,
                                    self.nodes[member.0 as usize].tala_id,
                                    self.position(*member)
                                )))
                                .collect::<Vec<_>>()
                        );
                    }
                }
                let within_max_size = within_max_size_before_reposition;
                let bad_state_overlap = original_state
                    .projected_transaction_state
                    .as_ref()
                    .map(|state| {
                        let mut dirty_nodes = self.projected_transaction_dirty_node_ids(state);
                        // Go's Commit marks a candidate dirty when container
                        // refitting changes its box, even if the translated
                        // operation touched only one of its children.  The
                        // projected carrier historically tracked positions
                        // only, so a one-pixel height expansion could evade
                        // the sibling-overlap pass and be accepted. Include
                        // ordinary position and size changes from the full
                        // rollback snapshot in the dirty set.
                        for (index, node) in self.nodes.iter().enumerate() {
                            let original = &original_state.nodes[index];
                            if node.position != original.position
                                || node.rect.size != original.rect.size
                            {
                                dirty_nodes.insert(node.tala_id);
                            }
                        }
                        !self.projected_transaction_nodes_are_valid_with_dirty(
                            state,
                            Some(&dirty_nodes),
                        )
                    })
                    .unwrap_or_else(|| {
                        self.transaction_has_bad_state_overlap(&original_state.existing_overlaps)
                    });
                if let Some(target) =
                    crate::internal::trace_env_value("TALA_TRACE_ALIGN_VALIDATION")
                {
                    let mut parts = target.split('>');
                    let from = parts.next().and_then(|id| id.parse::<i64>().ok());
                    let to = parts.next().and_then(|id| id.parse::<i64>().ok());
                    let edge_ref = &self.edge_records[edge.0 as usize];
                    let edge_matches = [
                        self.nodes[edge_ref.from.0 as usize].tala_id,
                        self.nodes[edge_ref.to.0 as usize].tala_id,
                    ] == [from.unwrap_or(0), to.unwrap_or(0)]
                        || [
                            self.nodes[edge_ref.from.0 as usize].tala_id,
                            self.nodes[edge_ref.to.0 as usize].tala_id,
                        ] == [to.unwrap_or(0), from.unwrap_or(0)];
                    if edge_matches {
                        eprintln!(
                            "ALIGN_VALIDATION_RUST edge={}>{} attempt={},{} within={} intersects={} new_overlap=omitted bad_state={} spacing_exact={} fixed_moved={} fixed_origin={} containment={} external={} hierarchy={}",
                            self.nodes[edge_ref.from.0 as usize].tala_id,
                            self.nodes[edge_ref.to.0 as usize].tala_id,
                            attempt.x,
                            attempt.y,
                            within_max_size,
                            intersects,
                            bad_state_overlap,
                            existing_became_exact,
                            fixed_moved,
                            fixed_origin_valid,
                            containment_valid,
                            external_containers_valid,
                            hierarchy_materialized,
                        );
                        for node in self
                            .nodes
                            .iter()
                            .filter(|node| matches!(node.tala_id, 370564473 | 739672091))
                        {
                            eprintln!(
                                "ALIGN_VALIDATION_RUST_NODE id={} pos={:?} size={:?} container={:?}",
                                node.tala_id,
                                node.position,
                                node.rect.size,
                                node.container
                                    .map(|parent| self.nodes[parent.0 as usize].tala_id),
                            );
                        }
                        for container in &self.transaction_external_containers {
                            if matches!(container.tala_id, 370564473 | 739672091) {
                                eprintln!(
                                    "ALIGN_VALIDATION_RUST_EXT id={} pos={:?} size={:?} anchor={:?} ancestors={:?} vessel={:?}",
                                    container.tala_id,
                                    container.position,
                                    container.rect.size,
                                    container.transaction_anchor_tala_id,
                                    container.scoring_container_ancestors,
                                    container.scoring_cluster_vessel,
                                );
                            }
                        }
                    }
                }
                within_max_size
                    && !intersects
                    && !bad_state_overlap
                    && !existing_became_exact
                    && !fixed_moved
                    && fixed_origin_valid
                    && containment_valid
                    && external_containers_valid
            } else {
                // Transaction.Commit uses AffectContainers=true for AlignAxes
                // even on aggregate-only graphs. With no ordinary hierarchy
                // to refit, the commit still publishes the active cluster
                // and sequence vessels before validation. Omitting this sync
                // leaves half-pixel row/column members in the opposite phase
                // from the Go transaction and changes later placement costs.
                self.sync_clusters();
                self.sync_sequences();
                !self.has_node_overlaps()
                    && !self.flat_aligning_edge_intersects_other_node(edge)
                    && self.transaction_fixed_origin_is_valid()
                    && nodes.iter().all(|node| {
                        let node = node.arena_node();
                        let point = self.position(node).unwrap();
                        !self.sized_point_overlaps(node, point)
                    })
            };
            if valid {
                let global_edge_length = self.global_sized_edge_length_for_alignment_candidate();
                let container_alignment = self.container_alignment_cost();
                let score = global_edge_length + container_alignment;
                if crate::internal::trace_env_enabled("TALA_TRACE_ALIGN_COST") {
                    eprintln!(
                        "ALIGN_TOTAL_COST_RUST attempt={},{} global={} container={} total={}",
                        attempt.x, attempt.y, global_edge_length, container_alignment, score,
                    );
                }
                if best.is_none_or(|(best_score, _)| {
                    alignment_precision_compare(score, best_score) != Ordering::Greater
                }) {
                    best = Some((score, attempt));
                }
            }
            original_state.restore(self);
        }
        best
    }

    /// Source addSequence publishes a vessel into Nodes/Containers and removes
    /// its steps; the root numerical adapter must cross this boundary before
    /// direct root scoring consumes the already reconnected real endpoints.
    pub(super) fn activate_root_sequence_vessels(&mut self) {
        #[cfg(feature = "diagnostic-traces")]
        if std::env::var_os("D2_RS_TRACE_SEQUENCE_ACTIVATION").is_some() {
            for (index, sequence) in self.sequences.iter().enumerate() {
                let first = sequence.members[0];
                let vessel = self.topology_vessel_node(TopologyVessel::Sequence(index));
                eprintln!(
                    "SEQUENCE_PUBLICATION_INPUT first={} firstLive={} vessel={:?} vesselLive={} cluster={:?} raw={:?} point={:?} vesselPoint={:?}",
                    self.nodes[first.0 as usize].tala_id,
                    self.node_order_contains(first),
                    vessel,
                    vessel.is_some_and(|node| self.node_order_contains(node)),
                    self.active_cluster_index(first),
                    self.nodes[first.0 as usize].position,
                    self.position(first),
                    vessel.and_then(|node| self.position(node))
                );
            }
        }
        let groups = self
            .sequences
            .iter()
            .enumerate()
            .filter_map(|(index, sequence)| {
                let first = *sequence.members.first()?;
                let vessel = self.topology_vessel_node(TopologyVessel::Sequence(index))?;
                (vessel != first
                    && (self.node_order_contains(first) || self.node_order_contains(vessel))
                    && self.active_cluster_index(first).is_none())
                .then_some((
                    vessel,
                    first,
                    sequence.members.clone(),
                    sequence.container,
                    index,
                ))
            })
            .collect::<Vec<_>>();
        for (vessel, carrier, members, parent, index) in groups {
            // Copyback retains the original first step's fitted point. This
            // boundary must publish it even if canonical Nodes already lists
            // the vessel; active_node_position would read that stale vessel.
            let position = self.position(carrier);
            let size = self.sequence_vessel_size(index);
            if let Some(position) = position {
                Pipeline::publish_induced_owner_point(self, vessel, Some(position));
            }
            self.nodes[vessel.0 as usize].rect.size = size;
            // Go's Graph.IsSequenceVessel still recognizes this pointer after
            // placement publishes the allocated vessel into Graph.Nodes.
            // Numerical aggregate lookup only recognizes the retired carrier
            // at this boundary; retain the vessel identity for scoring too.
            self.nodes[vessel.0 as usize].scoring_is_aggregate_vessel = true;
            if !self.node_order_contains(vessel) {
                // Place preserves the original graph's existing vessel slot.
                // This workspace represents that slot by the first sequence
                // member; replacing it must retain the order established by
                // PreprocessHubs rather than append a new optimizer candidate.
                if let Some(slot) = self.node_order.iter().position(|node| *node == carrier) {
                    self.node_order[slot] = vessel;
                } else {
                    self.node_order.push(vessel);
                }
            }
            self.node_order.retain(|node| !members.contains(node));
            if !self
                .containers
                .get(&parent)
                .unwrap_or_default()
                .contains(&vessel)
            {
                self.containers.append_node(parent, vessel);
            }
            let current = self.containers.get_header(&parent).unwrap_or_default();
            let mut updated = SharedNodeSlice::nil();
            for child in current.iter_slots() {
                if !child.is_some_and(|child| members.contains(&child)) {
                    updated = updated
                        .append(&[child])
                        .expect("sequence activation container append");
                }
            }
            self.containers.replace_header(parent, updated);
            for member in members {
                self.nodes[member.0 as usize].container = None;
            }
        }
        self.invalidate_node_order_membership();
        self.synchronize_original_owner_graph_view();
        self.sync_sequences();
        self.cost_cache.reset_placement();
    }

    /// AlignAxes explicitly supplies Sequence.EdgeAbductions. Retain its
    /// current pair and original pointers; geometry is read afresh per trial.
    fn prepare_root_sequence_alignment_records(&mut self) {
        if !self.sequences.iter().enumerate().any(|(index, _)| {
            self.topology_vessel_node(TopologyVessel::Sequence(index))
                .is_some()
        }) {
            return;
        }
        let kind = super::owner_groups::OwnerGroupKind::Sequence;
        let map = self
            .owner_graph_group_map(OwnerGraphId(0), kind)
            .expect("root sequence map");
        let mut records = Vec::new();
        for group in self
            .owner_group_map_values(&map)
            .expect("root sequence values")
        {
            let group =
                group.expect("runtime error: invalid memory address or nil pointer dereference");
            if let Some(header) = self
                .owner_sequence_abductions(group)
                .expect("root sequence records")
            {
                for id in header.iter_slots() {
                    records.push(
                        *self
                            .owner_edge_abduction(id.expect("root sequence record pointer"))
                            .expect("root sequence record"),
                    );
                }
            }
        }
        let original = |node: Option<NodeId>, owner: Option<NodeId>| {
            node.map(|node| AbductedNodeRef {
                source_node: Some(node),
                source_owner: owner,
                owner_tala_id: owner.map(|owner| self.nodes[owner.0 as usize].tala_id),
                owner,
                tala_id: self.nodes[node.0 as usize].tala_id,
                container_tala_id: self
                    .active_node_container(node)
                    .map(|parent| self.nodes[parent.0 as usize].tala_id),
                offset: None,
                size: self.nodes[node.0 as usize].rect.size,
                cluster_member: self.nodes[node.0 as usize].cluster.is_some(),
            })
        };
        let current = |node| {
            let mut reference = AbductionCurrentNodeRef::local_with_source(node, node);
            if self.original_owner_node(node) != node {
                // AddCluster reconnects the edge, but the earlier sequence
                // abduction retains its old CurrentFrom/CurrentTo pointer.
                // That absorbed member is outside the current numerical node
                // set even when its arena slot now carries the cluster vessel.
                // Do not let pointer matching restore a sequence step onto an
                // edge whose current endpoint is the new cluster vessel.
                reference.location = AbductionNodeLocation::Foreign;
                reference.tala_id = Some(self.nodes[node.0 as usize].tala_id);
                reference.position = self.position(node);
                reference.size = Some(self.nodes[node.0 as usize].rect.size);
            }
            reference
        };
        self.sized_edge_abductions = records
            .into_iter()
            .map(|record| SizedEdgeAbduction {
                source_edge: record
                    .source_edge
                    .expect("numerical abduction requires an edge"),
                edge_tala_id: self.edge_records[record
                    .source_edge
                    .expect("numerical abduction requires an edge")
                    .0 as usize]
                    .tala_id,
                edge: record.source_edge,
                current_from: record.current_from.map(current),
                current_to: record.current_to.map(current),
                originally_from: original(record.originally_from, record.current_from),
                originally_to: original(record.originally_to, record.current_to),
                originally_from_container: record
                    .originally_from
                    .and_then(|node| self.active_node_container(node)),
                originally_to_container: record
                    .originally_to
                    .and_then(|node| self.active_node_container(node)),
                originally_from_table_neighbors: Vec::new(),
                originally_to_table_neighbors: Vec::new(),
                sequence_abduction: true,
                obstructions_from_to: Vec::new(),
                obstructions_to_from: Vec::new(),
                obstruction_sets_from_to: OrderedObstructionInventory::default(),
                obstruction_sets_to_from: OrderedObstructionInventory::default(),
            })
            .collect();
        self.sized_edge_abductions_present = !self.sized_edge_abductions.is_empty();
        // placement/alignment.go49-58 only assembles scoring options here.
        // Retain the graph's memoized costs and shared edge-length map across
        // alignment passes, even after sequence geometry has changed.
    }

    pub(super) fn align_axes_pass(
        &mut self,
        transaction_existing_overlaps: &BTreeSet<(NodeId, NodeId)>,
        transaction_existing_exact_overlaps: &BTreeSet<(NodeId, NodeId)>,
        _sync_after_commit: bool,
    ) -> bool {
        // Go alignment.go:48-51 builds call-local sequence records even if
        // the sequence list is empty. Restore the caller's scoring scope on
        // normal return and unwind without installing a catch boundary.
        struct Restore<'a> {
            graph: &'a mut ArenaGraph,
            records: Tracked<Vec<SizedEdgeAbduction>>,
            present: bool,
        }
        impl Drop for Restore<'_> {
            fn drop(&mut self) {
                self.graph.sized_edge_abductions = std::mem::take(&mut self.records);
                self.graph.sized_edge_abductions_present = self.present;
            }
        }
        let records = std::mem::take(&mut self.sized_edge_abductions);
        let present = std::mem::replace(&mut self.sized_edge_abductions_present, false);
        let scope = Restore {
            graph: self,
            records,
            present,
        };
        scope.graph.align_axes_pass_with_sequence_records(
            transaction_existing_overlaps,
            transaction_existing_exact_overlaps,
        )
    }

    fn align_axes_pass_with_sequence_records(
        &mut self,
        transaction_existing_overlaps: &BTreeSet<(NodeId, NodeId)>,
        transaction_existing_exact_overlaps: &BTreeSet<(NodeId, NodeId)>,
    ) -> bool {
        self.prepare_root_sequence_alignment_records();
        if crate::internal::trace_env_enabled("TALA_TRACE_ALIGN_AGGREGATES") {
            for sequence in &self.sequences {
                eprint!(
                    "ALIGN_SEQUENCE_RUST vessel={} members=",
                    sequence.vessel_tala_id
                );
                for member in &sequence.members {
                    eprint!("{},", self.nodes[member.0 as usize].tala_id);
                }
                eprintln!();
            }
            for cluster in &self.clusters {
                eprint!(
                    "ALIGN_CLUSTER_RUST vessel={} members=",
                    cluster.vessel_tala_id
                );
                for member in &cluster.members {
                    eprint!("{},", self.nodes[member.0 as usize].tala_id);
                }
                eprint!(" member_nears=");
                for member in &cluster.members {
                    eprint!("{}:[", self.nodes[member.0 as usize].tala_id);
                    for near in self.nodes[member.0 as usize].nears.keys() {
                        eprint!("{},", self.nodes[near.0 as usize].tala_id);
                    }
                    eprint!("],");
                }
                eprintln!();
            }
            for abduction in &self.sized_edge_abductions {
                eprintln!(
                    "ALIGN_ABDUCTION_RUST current={:?}>{:?} original={:?}>{:?} sequence={}",
                    abduction
                        .current_from
                        .and_then(|endpoint| self.abduction_current_tala_id(endpoint)),
                    abduction
                        .current_to
                        .and_then(|endpoint| self.abduction_current_tala_id(endpoint)),
                    abduction.originally_from.map(|node| node.tala_id),
                    abduction.originally_to.map(|node| node.tala_id),
                    abduction.sequence_abduction,
                );
            }
        }
        let tree_nodes = routing::routing_tree_nodes(self);
        // Keep a separate slice for the AlignAxes exclusion carrier.  Go
        // appends Graph.NodeToTree nodes and fixed nodes independently, so an
        // endpoint present in both remains duplicated when the final slot is
        // replaced for the opposite-side trial.
        let alignment_excluded_base = self
            .node_order
            .iter()
            .copied()
            .filter(|node| tree_nodes.contains(node))
            .chain(
                self.node_order
                    .iter()
                    .copied()
                    .filter(|node| self.nodes[node.0 as usize].fixed_top_left.is_some()),
            )
            .collect::<Vec<_>>();
        let edges = self.edge_order.read();
        // AlignAxes snapshots the raw global length before table accounting;
        // the later fallback intentionally compares a global+container score
        // against this raw baseline, matching the recovered Go control flow.
        let initial_length = self.global_sized_edge_length_for_alignment();
        let has_table_columns = edges
            .iter()
            .any(|edge| self.edge_records[edge.0 as usize].has_table_column());
        let initial_aligned_table_columns = edges
            .iter()
            .filter(|edge| {
                self.edge_records[edge.0 as usize].has_table_column()
                    && self.edge_axis_aligned(**edge)
            })
            .count();
        let mut changed = false;
        for edge in edges.iter().copied() {
            let edge_ref = &self.edge_records[edge.0 as usize];
            let edge_from = edge_ref.from;
            let edge_to = edge_ref.to;
            let edge_has_table_column = edge_ref.has_table_column();
            let is_tree = self.is_tree_edge(edge);
            let is_sequence_internal = self.sequences.iter().any(|sequence| {
                self.sequence_is_active(sequence)
                    && sequence.members.contains(&edge_from)
                    && sequence.members.contains(&edge_to)
            });
            if crate::internal::trace_env_enabled("TALA_TRACE_ALIGN_EDGES") {
                let active_from = self.active_aggregate_owner(edge_from);
                let active_to = self.active_aggregate_owner(edge_to);
                let (from_position, from_size) = self.alignment_delta_endpoint_box(edge_from);
                let (to_position, to_size) = self.alignment_delta_endpoint_box(edge_to);
                eprintln!(
                    "ALIGN_EDGE_RUST edge={}>{} active={}>{} tree={} sequence_internal={} aligned={} hierarchy={} from=({:016x},{:016x},{:016x},{:016x}) to=({:016x},{:016x},{:016x},{:016x})",
                    self.nodes[edge_from.0 as usize].tala_id,
                    self.nodes[edge_to.0 as usize].tala_id,
                    self.nodes[active_from.0 as usize].tala_id,
                    self.nodes[active_to.0 as usize].tala_id,
                    is_tree,
                    is_sequence_internal,
                    self.edge_axis_aligned(edge),
                    self.nodes[active_from.0 as usize].hierarchy.is_some()
                        || self.nodes[active_to.0 as usize].hierarchy.is_some(),
                    from_position.x.to_bits(),
                    from_position.y.to_bits(),
                    from_size.width.to_bits(),
                    from_size.height.to_bits(),
                    to_position.x.to_bits(),
                    to_position.y.to_bits(),
                    to_size.width.to_bits(),
                    to_size.height.to_bits(),
                );
            }
            if is_tree || is_sequence_internal {
                continue;
            }
            if self.edge_axis_aligned(edge) {
                continue;
            }
            let (from, to) = (
                self.active_aggregate_owner(edge_from),
                self.active_aggregate_owner(edge_to),
            );
            if from == to {
                continue;
            }
            // Recovered `Graph.AlignAxes` skips an edge when either endpoint
            // belongs to a hierarchy. `PlaceHierarchies` owns the ranked
            // geometry; the later global alignment stage must not straighten
            // those edges by moving individual hierarchy members.
            if self.nodes[from.0 as usize].hierarchy.is_some()
                || self.nodes[to.0 as usize].hierarchy.is_some()
            {
                continue;
            }
            // Go refreshes the reusable transaction's rollback point before
            // it evaluates the baseline edge/container cost.  The scoring
            // path can publish aggregate geometry (and, for half-pixel
            // cluster centers, change which side of the rounding tie is
            // visible), so capturing after the baseline would make the
            // accepted move commit from a different state than Go's
            // Transaction.Rollback followed by its final op.
            let original_state = AlignmentTrialSnapshot::capture_with_overlap_state(
                self,
                transaction_existing_overlaps.clone(),
                transaction_existing_exact_overlaps.clone(),
            );
            let delta = self.alignment_deltas(edge);
            // The capture above evaluated this pure predicate on the same,
            // unmodified graph. Diagnostic builds re-run it so its failure
            // trace is emitted exactly as before.
            #[cfg(feature = "diagnostic-traces")]
            let hierarchy_materialized = self.transaction_containment_is_valid();
            #[cfg(not(feature = "diagnostic-traces"))]
            let hierarchy_materialized = original_state.hierarchy_materialized;
            if crate::internal::trace_env_enabled("TALA_TRACE_ALIGN_SNAPSHOTS") {
                let cluster_member = self
                    .clusters
                    .iter()
                    .find(|cluster| cluster.vessel_tala_id == 1_543_039_099_823_358_511)
                    .and_then(|cluster| cluster.members.get(1).copied())
                    .and_then(|member| original_state.nodes.get(member.0 as usize))
                    .and_then(|node| node.position);
                eprintln!(
                    "ALIGN_SNAPSHOT_RUST edge={}>{} member6={:?}",
                    self.nodes[from.0 as usize].tala_id,
                    self.nodes[to.0 as usize].tala_id,
                    cluster_member,
                );
            }
            if crate::internal::trace_env_enabled("TALA_TRACE_ALIGN_PRE_GLOBAL") {
                eprintln!(
                    "ALIGN_PRE_GLOBAL_RUST edge={}>{} cluster={:?}",
                    self.nodes[from.0 as usize].tala_id,
                    self.nodes[to.0 as usize].tala_id,
                    self.clusters
                        .iter()
                        .filter(|cluster| cluster.vessel_tala_id == 5_577_006_791_947_779_410)
                        .map(|cluster| cluster
                            .members
                            .iter()
                            .map(|member| (
                                self.nodes[member.0 as usize].tala_id,
                                self.position(*member)
                            ))
                            .collect::<Vec<_>>())
                        .collect::<Vec<_>>()
                );
            }
            let global_edge_length = self.global_sized_edge_length_for_alignment();
            let container_alignment = self.container_alignment_cost();
            let mut best_score = global_edge_length + container_alignment;
            if crate::internal::trace_env_enabled("TALA_TRACE_ALIGN_GLOBAL_TERMS") {
                eprintln!(
                    "ALIGN_GLOBAL_TERMS_RUST edge={}>{} global={} container={} total={} raw_turn={}",
                    self.nodes[from.0 as usize].tala_id,
                    self.nodes[to.0 as usize].tala_id,
                    global_edge_length,
                    container_alignment,
                    best_score,
                    self.raw_turn_cost(),
                );
            }
            if crate::internal::trace_env_enabled("TALA_TRACE_ALIGN_COMPONENTS") {
                eprintln!(
                    "ALIGN_COMPONENTS_RUST edge={}>{} global={} container={} total={}",
                    self.nodes[from.0 as usize].tala_id,
                    self.nodes[to.0 as usize].tala_id,
                    global_edge_length,
                    container_alignment,
                    best_score,
                );
                if let Some(cluster) = self
                    .clusters
                    .iter()
                    .find(|cluster| cluster.vessel_tala_id == 5_577_006_791_947_779_410)
                {
                    eprintln!(
                        "ALIGN_COMPONENT_CLUSTER_RUST vessel={} members={:?}",
                        cluster.vessel_tala_id,
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
                for trace_id in [1535972209_i64, 3360005514, 3272550982, 1539737869] {
                    for node in &self.nodes {
                        if node.tala_id == trace_id {
                            eprintln!(
                                "ALIGN_COST_NODE_RUST id={} pos={:?} size={:?} container={:?}",
                                node.tala_id,
                                node.position,
                                node.rect.size,
                                node.container
                                    .map(|parent| self.nodes[parent.0 as usize].tala_id),
                            );
                        }
                    }
                }
            }
            if crate::internal::trace_env_enabled("TALA_TRACE_ALIGN_BREAKDOWN") {
                eprintln!(
                    "ALIGN_BREAKDOWN_RUST global={} container={} total={} raw_turn={}",
                    global_edge_length,
                    container_alignment,
                    best_score,
                    self.raw_turn_cost(),
                );
            }
            let mut best: Option<(Vec<ConnectedNode>, Point)> = None;
            let mut excluded = alignment_excluded_base
                .iter()
                .copied()
                .map(|node| self.retained_container_ref(node))
                .collect::<Vec<_>>();
            excluded.push(ConnectedNode::Active(to));
            let from_nodes = self.connected_node_views(ConnectedNode::Active(from), &excluded);
            let from_attempt = self.try_alignment_move(edge, &from_nodes, delta, &original_state);
            if crate::internal::trace_env_enabled("TALA_TRACE_ALIGN_TRIALS") {
                eprint!(
                    "ALIGN_TRIAL_RUST edge={}>{} side=from base={} delta={},{} result={:?} nodes=",
                    self.active_node_tala_id(from),
                    self.active_node_tala_id(to),
                    best_score,
                    delta.x,
                    delta.y,
                    from_attempt,
                );
                for node in &from_nodes {
                    eprint!("{},", self.connected_node_id(*node));
                }
                eprintln!();
            }
            if let Some((score, attempt)) = from_attempt {
                let comparison = alignment_precision_compare(score, best_score);
                if comparison == Ordering::Less
                    || (edge_has_table_column && comparison == Ordering::Equal)
                {
                    best_score = score;
                    best = Some((from_nodes, attempt));
                }
            }

            *excluded
                .last_mut()
                .expect("alignment exclusion endpoint slot") = ConnectedNode::Active(from);
            let to_nodes = self.connected_node_views(ConnectedNode::Active(to), &excluded);
            let to_attempt = self.try_alignment_move(
                edge,
                &to_nodes,
                Point {
                    x: -delta.x,
                    y: -delta.y,
                },
                &original_state,
            );
            if crate::internal::trace_env_enabled("TALA_TRACE_ALIGN_TRIALS") {
                eprint!(
                    "ALIGN_TRIAL_RUST edge={}>{} side=to base={} delta={},{} result={:?} nodes=",
                    self.active_node_tala_id(from),
                    self.active_node_tala_id(to),
                    best_score,
                    -delta.x,
                    -delta.y,
                    to_attempt,
                );
                for node in &to_nodes {
                    eprint!("{},", self.connected_node_id(*node));
                }
                eprintln!();
            }
            if let Some((score, attempt)) = to_attempt {
                let comparison = alignment_precision_compare(score, best_score);
                if comparison == Ordering::Less
                    || (edge_has_table_column && comparison == Ordering::Equal)
                {
                    best = Some((to_nodes, attempt));
                }
            }
            if let Some((nodes, attempt)) = best {
                if crate::internal::trace_env_enabled("TALA_TRACE_ALIGN_MOVES") {
                    eprint!(
                        "ALIGN_MOVE_RUST edge={}>{} active={}>{} delta={},{} nodes=",
                        self.active_node_tala_id(edge_from),
                        self.active_node_tala_id(edge_to),
                        self.active_node_tala_id(from),
                        self.active_node_tala_id(to),
                        attempt.x,
                        attempt.y,
                    );
                    for node in &nodes {
                        eprint!("{},", self.connected_node_id(*node));
                    }
                    eprintln!();
                }
                self.translate_connected_node_boxes(&nodes, attempt);
                if hierarchy_materialized {
                    // Keep the transaction phases separate. The Go commit
                    // refits ordinary containers first and then publishes
                    // clusters/sequences exactly once; the public helper
                    // would perform that publication internally and make the
                    // final carrier sync below run twice on half-pixel
                    // cluster centers.
                    self.reposition_ordinary_containers_without_sync();
                }
                // Go's Transaction.Commit always follows the refit with
                // SyncClusters then SyncSequences. Aggregate-only graphs do
                // not materialize ordinary containers, so keep this
                // publication unconditional. `sync_after_commit` remains a
                // stage-level compatibility parameter for callers that need
                // the historical lazy frame; the transaction boundary itself
                // must always be complete.
                self.sync_clusters();
                self.sync_sequences();
                // Go's Transaction.UpdateState snapshots live boxes before
                // each later edge. Refresh the projected carrier geometry at
                // this commit boundary so dirty-node overlap checks do not
                // classify unchanged aggregate vessels as modified.
                self.refresh_projected_transaction_node_geometry();
                if crate::internal::trace_env_enabled("TALA_TRACE_ALIGN_STATE_TARGETS") {
                    eprint!(
                        "ALIGN_STATE_TARGETS_RUST edge={}>{} delta={},{}",
                        self.nodes[edge_from.0 as usize].tala_id,
                        self.nodes[edge_to.0 as usize].tala_id,
                        attempt.x,
                        attempt.y
                    );
                    for target in [1_149_337_423_i64, 1_782_109_120_i64, 233_611_931_i64] {
                        if let Some(node) = self.nodes.iter().find(|node| node.tala_id == target) {
                            eprint!(" target{}={:?}", target, node.position);
                        }
                    }
                    eprintln!();
                }
                changed = true;
            }
        }
        if has_table_columns && changed {
            let aligned_table_columns = edges
                .iter()
                .filter(|edge| {
                    self.edge_records[edge.0 as usize].has_table_column()
                        && self.edge_axis_aligned(**edge)
                })
                .count();
            if aligned_table_columns <= initial_aligned_table_columns {
                let final_length =
                    self.global_sized_edge_length_for_alignment() + self.container_alignment_cost();
                changed =
                    alignment_precision_compare(final_length, initial_length) == Ordering::Less;
            }
        }
        changed
    }

    pub(super) fn align_axes(&mut self) {
        self.align_axes_with_commit_sync(false);
    }

    /// Run AlignAxes and publish aggregate carrier frames at the final
    /// pre-packing boundary. Earlier passes retain the lazy vessel frame so
    /// the stable arena follows the same visible lifecycle as the Go graph.
    pub(super) fn align_axes_with_commit_sync(&mut self, sync_after_commit: bool) {
        // The Go Align entry point creates one transaction before its 100-pass
        // loop.  Its existing-overlap exceptions remain anchored to that
        // transaction even as each edge refreshes the rollback geometry. Keep
        // the same baseline for every Rust pass and edge.
        let transaction_existing_overlaps = self.overlap_pairs(false);
        let transaction_existing_exact_overlaps = self.exact_overlap_pairs();
        for iteration in 0..100 {
            if crate::internal::trace_env_enabled("TALA_TRACE_ALIGN_STATES") {
                eprintln!(
                    "ALIGN_PRE_RUST {iteration} containment={}",
                    self.transaction_containment_is_valid()
                );
            }
            let changed = self.align_axes_pass(
                &transaction_existing_overlaps,
                &transaction_existing_exact_overlaps,
                sync_after_commit,
            );
            if crate::internal::trace_env_enabled("TALA_TRACE_ALIGN_ITER") {
                eprintln!(
                    "ALIGN_ITER_RUST iteration={} changed={}",
                    iteration, changed
                );
                if let Some(cluster) = self
                    .clusters
                    .iter()
                    .find(|cluster| cluster.vessel_tala_id == 1_543_039_099_823_358_511)
                {
                    let member6 = cluster
                        .members
                        .get(1)
                        .and_then(|member| self.position(*member));
                    eprintln!(
                        "ALIGN_ITER_CLUSTER_RUST iteration={} member6={:?}",
                        iteration, member6
                    );
                }
            }
            if crate::internal::trace_env_enabled("TALA_TRACE_ALIGN_STATES") {
                eprint!("ALIGN_STATE_RUST {iteration} changed={changed}");
                for node in &self.nodes {
                    eprint!(" {}=", node.tala_id);
                    if let Some(position) = node.position {
                        eprint!(
                            "{},{}:{},{}",
                            position.x, position.y, node.rect.size.width, node.rect.size.height
                        );
                    } else {
                        eprint!("nil");
                    }
                }
                eprintln!();
            }
            if !changed {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Edge, Graph, Node, Size};

    #[test]
    fn alignment_projected_snapshot_preserves_first_match_and_nil_positions() {
        let mut input = Graph::default();
        let active = input.add_node(Node::new(
            "active",
            Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        let hidden = input.add_node(Node::new(
            "hidden",
            Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        let mut graph = ArenaGraph::from_input(&input);
        graph.set_position(active, Point { x: 1.0, y: -0.0 });
        graph.set_position(hidden, Point { x: 9.0, y: 9.0 });
        let template = graph.projected_transaction_node_from_current(hidden);
        graph.projected_transaction_nodes =
            vec![graph.projected_transaction_node_from_current(active)];
        for (offset, position) in [
            Some(Point { x: 20.0, y: 2.0 }),
            None,
            Some(Point { x: 40.0, y: 4.0 }),
            Some(Point { x: 50.0, y: 5.0 }),
        ]
        .into_iter()
        .enumerate()
        {
            let mut entry = template.clone();
            entry.node.tala_id = 100 + offset as i64;
            entry.node.position = position;
            graph.projected_transaction_nodes.push(entry);
        }
        let mut child = template.node.clone();
        child.tala_id = 100;
        child.position = Some(Point { x: 21.0, y: -0.0 });
        graph
            .transaction_external_container_children
            .insert(1, vec![child.clone()]);
        child.position = Some(Point { x: 22.0, y: 2.0 });
        graph
            .transaction_external_container_children
            .insert(2, vec![child.clone()]);
        graph
            .transaction_external_aggregate_children
            .insert(1, vec![child.clone()]);
        graph.transaction_external_containers.push(child);
        let mut nil_child = template.node.clone();
        nil_child.tala_id = 101;
        nil_child.position = None;
        graph
            .transaction_external_aggregate_children
            .insert(2, vec![nil_child.clone()]);
        nil_child.position = Some(Point { x: 99.0, y: 99.0 });
        graph.transaction_external_containers.push(nil_child);
        let expected = graph.projected_transaction_state();
        let actual = AlignmentTrialSnapshot::projected_geometry(&graph);
        let bits = |positions: &[Option<Point>]| {
            positions
                .iter()
                .map(|point| point.map(|p| (p.x.to_bits(), p.y.to_bits())))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            bits(&actual.original_positions),
            bits(&expected.original_positions)
        );
        assert_eq!(
            format!("{:?}", actual.nodes),
            format!("{:?}", expected.nodes)
        );
        assert_eq!(
            actual.original_positions[1],
            Some(Point { x: 21.0, y: -0.0 })
        );
        assert_eq!(actual.original_positions[2], None);
        assert!(actual.existing_overlaps.is_empty());
        assert!(actual.existing_exact_overlaps.is_empty());
    }

    #[test]
    fn cluster_box_translation_moves_actual_point_once_and_rolls_back() {
        let mut input = Graph::default();
        let first = input.add_node(Node::new(
            "first",
            Size {
                width: 30.0,
                height: 10.0,
            },
        ));
        let second = input.add_node(Node::new(
            "second",
            Size {
                width: 30.0,
                height: 10.0,
            },
        ));
        let mut graph = ArenaGraph::from_input(&input);
        graph.clusters.push(ClusterState {
            members: vec![first, second],
            arrangement: ClusterArrangement::Column,
            desired_arrangement: ClusterArrangement::Column,
            padding: 20.0,
            vessel_tala_id: 10001,
            fixed_size: false,
        });
        for member in [first, second] {
            graph.nodes[member.0 as usize].cluster = Some(0);
        }
        graph.node_order = vec![first];
        let start = Point {
            x: 120.25,
            y: 230.75,
        };
        let mut entity = graph.nodes[first.0 as usize].clone();
        entity.tala_id = 10001;
        entity.position = Some(start);
        entity.rect.origin = start;
        let vessel = graph.retain_topology_node(entity);
        graph
            .bind_topology_vessel(TopologyVessel::Cluster(0), vessel)
            .unwrap();
        graph.materialize_owner_node_point(vessel).unwrap();
        let retained = graph.owner_node_point(vessel).unwrap();
        graph.pending_cluster_vessel_positions.insert(0, start);
        graph.set_position(
            first,
            Point {
                x: -400.0,
                y: -500.0,
            },
        );
        graph.refresh_node_order_membership();
        let prior = AlignmentTrialSnapshot::capture_with_overlap_state(
            &graph,
            BTreeSet::new(),
            BTreeSet::new(),
        );
        // Source Node.Translate changes the vessel's own TopLeft, without
        // implicitly translating its hidden members. Duplicate native
        // carrier handles must still resolve to one actual node movement.
        graph.translate_active_node_boxes(&[first, first], Point { x: 0.5, y: -60.25 });
        let shifted = Point {
            x: 120.75,
            y: 170.5,
        };
        assert_eq!(retained.get(), Some(shifted));
        assert_eq!(graph.position(vessel), Some(shifted));
        assert_eq!(graph.pending_cluster_vessel_positions[&0], shifted);
        assert_eq!(
            graph.position(first),
            Some(Point {
                x: -400.0,
                y: -500.0
            })
        );
        prior.restore(&mut graph);
        assert_eq!(retained.get(), Some(start));
        assert_eq!(graph.pending_cluster_vessel_positions[&0], start);
        assert!(retained.same_allocation(&graph.owner_node_point(vessel).unwrap()));
        // An abducted edge can supply the actual vessel handle directly,
        // rather than the first-member carrier used by the numerical order.
        graph.translate_active_node_boxes(&[vessel, vessel], Point { x: 0.5, y: -60.25 });
        assert_eq!(graph.position(vessel), Some(shifted));
        assert_eq!(graph.pending_cluster_vessel_positions[&0], shifted);
        assert_eq!(retained.get(), Some(shifted));
    }

    #[test]
    fn alignment_rollback_restores_bound_point_values_and_allocation() {
        let mut input = Graph::default();
        let node = input.add_node(Node::new(
            "node",
            Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        let mut arena = ArenaGraph::from_input(&input);
        arena.initialize_shared_topology();
        arena.set_position(node, Point { x: 10.0, y: 20.0 });
        arena.materialize_owner_node_point(node).unwrap();
        let retained = arena.owner_node_point(node).unwrap();
        let prior = AlignmentTrialSnapshot::capture_with_overlap_state(
            &arena,
            BTreeSet::new(),
            BTreeSet::new(),
        );
        arena.translate_owner_point(node, 5.0, 7.0);
        assert_eq!(retained.get(), Some(Point { x: 15.0, y: 27.0 }));
        // A rejected trial must also undo pointer replacement, restoring the
        // allocation that the caller retained before the transaction.
        arena.set_position(node, Point { x: 100.0, y: 200.0 });
        prior.restore(&mut arena);
        assert_eq!(retained.get(), Some(Point { x: 10.0, y: 20.0 }));
        assert_eq!(arena.position(node), retained.get());
        assert!(retained.same_allocation(&arena.owner_node_point(node).unwrap()));
        arena.clear_position(node);
        let prior_nil = AlignmentTrialSnapshot::capture_with_overlap_state(
            &arena,
            BTreeSet::new(),
            BTreeSet::new(),
        );
        arena.set_position(node, Point { x: 30.0, y: 40.0 });
        prior_nil.restore(&mut arena);
        assert!(arena.position(node).is_none());
        assert!(arena.owner_node_point(node).unwrap().get().is_none());
    }

    #[test]
    fn bound_cluster_endpoint_uses_live_carrier_in_cached_and_uncached_views() {
        let mut input = Graph::default();
        let first = input.add_node(Node::new(
            "first",
            Size {
                width: 54.0,
                height: 66.0,
            },
        ));
        let second = input.add_node(Node::new(
            "second",
            Size {
                width: 54.0,
                height: 66.0,
            },
        ));
        let peer = input.add_node(Node::new(
            "peer",
            Size {
                width: 52.0,
                height: 66.0,
            },
        ));
        let edge = input.add_edge(Edge {
            source: first,
            target: peer,
        });
        input.add_edge(Edge {
            source: second,
            target: peer,
        });
        let mut pipeline = Pipeline::new(&input, 1, false, true);
        pipeline.graph.clusters.push(ClusterState {
            members: vec![first, second],
            arrangement: ClusterArrangement::Row,
            desired_arrangement: ClusterArrangement::Row,
            padding: 20.0,
            vessel_tala_id: 501,
            fixed_size: false,
        });
        for member in [first, second] {
            pipeline.graph.nodes[member.0 as usize].cluster = Some(0);
        }
        pipeline.graph.node_order = vec![first, peer];
        pipeline
            .prepare_placement_scope_after(None, &BTreeSet::new())
            .unwrap()
            .unwrap();
        let vessel = pipeline
            .graph
            .topology_vessel_node(TopologyVessel::Cluster(0))
            .unwrap();
        assert_eq!(pipeline.graph.edge_records[edge.0 as usize].from, vessel);
        // Canonical binding, rather than a coincidental Tala ID or stale box,
        // establishes the vessel's numerical owner in both lookup paths.
        for cached in [false, true] {
            pipeline.graph.invalidate_node_order_membership();
            if cached {
                pipeline.graph.refresh_node_order_membership();
            }
            assert_eq!(pipeline.graph.active_cluster_index(vessel), Some(0));
            assert_eq!(pipeline.graph.active_aggregate_owner(vessel), first);
            assert_eq!(pipeline.graph.active_adjacent(peer, edge), first);
            assert_eq!(
                pipeline
                    .graph
                    .active_edge_ids(first)
                    .into_iter()
                    .collect::<Vec<_>>(),
                vec![edge, EdgeId(1)]
            );
            assert_eq!(pipeline.graph.active_edge_count(first), 2);
            assert_eq!(
                pipeline
                    .graph
                    .max_edge_length_incident_edges(first)
                    .into_iter()
                    .collect::<Vec<_>>(),
                vec![edge, EdgeId(1)]
            );
        }
        pipeline.graph.node_order = vec![first, second, peer];
        pipeline.graph.invalidate_node_order_membership();
        assert_eq!(pipeline.graph.active_cluster_index(vessel), None);
        pipeline.graph.refresh_node_order_membership();
        assert_eq!(pipeline.graph.active_cluster_index(vessel), None);
    }

    #[test]
    fn published_nested_sequence_projection_reads_live_member_points() {
        let mut input = Graph::default();
        let container = input.add_node(Node::new(
            "box",
            Size {
                width: 271.0,
                height: 221.0,
            },
        ));
        let first = input.add_node(Node::new(
            "first",
            Size {
                width: 93.0,
                height: 101.0,
            },
        ));
        let second = input.add_node(Node::new(
            "second",
            Size {
                width: 93.0,
                height: 101.0,
            },
        ));
        let mut pipeline = Pipeline::new(&input, 1, false, true);
        pipeline.graph.nodes[container.0 as usize].is_container = true;
        for member in [first, second] {
            pipeline.graph.nodes[member.0 as usize].container = Some(container);
            pipeline.graph.nodes[member.0 as usize].sequence = Some(0);
        }
        pipeline.graph.sequences.push(SequenceState {
            members: vec![first, second],
            vessel_tala_id: 501,
            container: Some(container),
            has_edge_abductions: true,
        });
        pipeline.graph.node_order = vec![container, first];
        pipeline
            .graph
            .containers
            .insert(Some(container), vec![first, second]);
        pipeline
            .prepare_placement_scope_after(Some(container), &BTreeSet::new())
            .unwrap()
            .unwrap();
        let vessel = pipeline
            .graph
            .topology_vessel_node(TopologyVessel::Sequence(0))
            .unwrap();
        pipeline
            .graph
            .set_position(container, Point { x: 0.0, y: 0.0 });
        pipeline
            .graph
            .set_position(first, Point { x: 60.0, y: 60.0 });
        pipeline.graph.activate_root_sequence_vessels();
        assert_eq!(
            pipeline.graph.container_node_order(Some(container)),
            vec![vessel]
        );
        let projected = ProjectedAdjacent {
            owner: vessel,
            tala_id: pipeline.graph.nodes[second.0 as usize].tala_id,
            container_tala_id: Some(pipeline.graph.nodes[container.0 as usize].tala_id),
            offset: Point { x: 58.0, y: 0.0 },
            size: pipeline.graph.nodes[second.0 as usize].rect.size,
            cluster_member: false,
        };
        assert_eq!(
            pipeline.graph.projected_absolute_position(projected),
            Some(Point { x: 118.0, y: 60.0 })
        );
        pipeline.graph.translate_connected_node_boxes(
            &[ConnectedNode::Active(vessel)],
            Point { x: 31.0, y: 0.0 },
        );
        pipeline.graph.sync_sequences();
        assert_eq!(
            pipeline.graph.projected_absolute_position(projected),
            Some(Point { x: 149.0, y: 60.0 })
        );
        pipeline.graph.clear_position(second);
        assert_eq!(pipeline.graph.projected_absolute_position(projected), None);
    }

    #[test]
    fn sequence_alignment_preparation_preserves_graph_cost_storage() {
        let mut input = Graph::default();
        let first = input.add_node(Node::new(
            "first",
            Size {
                width: 80.0,
                height: 50.0,
            },
        ));
        let second = input.add_node(Node::new(
            "second",
            Size {
                width: 80.0,
                height: 50.0,
            },
        ));
        let peer = input.add_node(Node::new(
            "peer",
            Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        input.add_edge(Edge {
            source: first,
            target: peer,
        });
        let mut pipeline = Pipeline::new(&input, 1, false, true);
        pipeline.graph.sequences.push(SequenceState {
            members: vec![first, second],
            vessel_tala_id: 501,
            container: None,
            has_edge_abductions: true,
        });
        for member in [first, second] {
            pipeline.graph.nodes[member.0 as usize].sequence = Some(0);
        }
        pipeline.graph.node_order = vec![first, peer];
        pipeline
            .graph
            .containers
            .insert(None, vec![first, second, peer]);
        pipeline
            .prepare_placement_scope_after(None, &BTreeSet::new())
            .unwrap()
            .unwrap();
        pipeline
            .graph
            .set_position(first, Point { x: 0.0, y: 100.0 });
        pipeline.graph.set_position(peer, Point { x: 0.0, y: 0.0 });
        pipeline.graph.activate_root_sequence_vessels();
        let cache = pipeline.graph.cost_cache.clone();
        cache.restore_routing((3.0, 5.0, 7.0));
        cache.store_edge_length(11, 13.0);
        let retained = cache.edge_length_cache_alias().unwrap();
        let before = cache.diagnostic_observe();
        for _ in 0..2 {
            pipeline.graph.prepare_root_sequence_alignment_records();
            assert_eq!(pipeline.graph.sized_edge_abductions.len(), 1);
            assert!(pipeline.graph.sized_edge_abductions_present);
            let after = pipeline.graph.cost_cache.diagnostic_observe();
            assert_eq!(after.cost_owner_address, before.cost_owner_address);
            assert_eq!(after.routing_addresses, before.routing_addresses);
            assert_eq!(after.routing_bits, before.routing_bits);
            let map = after.cache.unwrap();
            let original = before.cache.as_ref().unwrap();
            assert_eq!(map.reference_address, original.reference_address);
            assert_eq!(map.entries, original.entries);
            assert!(retained.diagnostic_observe().is_current);
        }
        cache.disable_edge_length_cache();
        pipeline.graph.prepare_root_sequence_alignment_records();
        assert!(!cache.edge_length_cache_enabled());
        assert_eq!(cache.raw(), (3.0, 5.0, 7.0));
        assert_eq!(
            retained.diagnostic_observe().cache.entries,
            vec![(11, 13.0_f64.to_bits())]
        );
    }

    #[test]
    fn alignment_restores_bound_sequence_steps_by_retained_edge_pointer() {
        let mut input = Graph::default();
        let first = input.add_node(Node::new(
            "first",
            Size {
                width: 80.0,
                height: 50.0,
            },
        ));
        let second = input.add_node(Node::new(
            "second",
            Size {
                width: 80.0,
                height: 50.0,
            },
        ));
        let peer = input.add_node(Node::new(
            "peer",
            Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        let outgoing = input.add_edge(Edge {
            source: first,
            target: peer,
        });
        let incoming = input.add_edge(Edge {
            source: peer,
            target: second,
        });
        let mut pipeline = Pipeline::new(&input, 1, false, true);
        pipeline.graph.sequences.push(SequenceState {
            members: vec![first, second],
            vessel_tala_id: 501,
            container: None,
            has_edge_abductions: true,
        });
        for member in [first, second] {
            pipeline.graph.nodes[member.0 as usize].sequence = Some(0);
        }
        pipeline.graph.node_order = vec![first, peer];
        pipeline
            .graph
            .containers
            .insert(None, vec![first, second, peer]);
        pipeline
            .prepare_placement_scope_after(None, &BTreeSet::new())
            .unwrap()
            .unwrap();
        let vessel = pipeline
            .graph
            .topology_vessel_node(TopologyVessel::Sequence(0))
            .unwrap();
        pipeline
            .graph
            .set_position(first, Point { x: 0.0, y: 100.0 });
        pipeline.graph.set_position(peer, Point { x: 0.0, y: 0.0 });
        let arena = &pipeline.graph;
        assert_eq!(arena.edge_records[outgoing.0 as usize].from, vessel);
        assert_eq!(arena.edge_records[incoming.0 as usize].to, vessel);
        assert_eq!(
            arena.alignment_delta_endpoint_box(vessel),
            (
                Point { x: 0.0, y: 100.0 },
                Size {
                    width: 125.0,
                    height: 50.0
                }
            )
        );
        assert_eq!(
            arena.alignment_original_endpoint_box(outgoing, vessel),
            (
                Point { x: 0.0, y: 100.0 },
                Size {
                    width: 80.0,
                    height: 50.0
                }
            )
        );
        assert_eq!(
            arena.alignment_original_endpoint_box(incoming, vessel),
            (
                Point { x: 45.0, y: 100.0 },
                Size {
                    width: 80.0,
                    height: 50.0
                }
            )
        );
        assert_eq!(arena.alignment_deltas(outgoing), Point { x: -20.0, y: 0.0 });
        assert_eq!(arena.alignment_deltas(incoming), Point { x: 65.0, y: 0.0 });
        // A flat root is placement_scope_owned too. That geometry flag must
        // not suppress publication of its bound canonical sequence pointers.
        assert!(pipeline.graph.placement_scope_owned);
        pipeline.graph.activate_root_sequence_vessels();
        // Source Place retains the vessel's existing Graph.Nodes slot;
        // Containers has its own independently published child order.
        assert_eq!(pipeline.graph.node_order, vec![vessel, peer]);
        assert_eq!(
            pipeline.graph.containers.get(&None).unwrap(),
            vec![peer, vessel]
        );
        for member in [first, second] {
            assert!(pipeline.graph.nodes[member.0 as usize].container.is_none());
        }
        assert_eq!(pipeline.graph.active_edge_ids(vessel).len(), 2);
        pipeline.graph.activate_root_sequence_vessels();
        // Source Place retains the vessel's existing Graph.Nodes slot;
        // Containers has its own independently published child order.
        assert_eq!(pipeline.graph.node_order, vec![vessel, peer]);
        assert_eq!(
            pipeline.graph.containers.get(&None).unwrap(),
            vec![peer, vessel]
        );
        assert_eq!(
            pipeline.graph.container_node_order(None),
            vec![peer, vessel]
        );
        assert_eq!(
            pipeline.graph.connected_node_views(
                ConnectedNode::Active(vessel),
                &[ConnectedNode::Active(peer)]
            ),
            vec![ConnectedNode::Active(vessel)]
        );
        assert_eq!(
            pipeline.graph.nodes[vessel.0 as usize].rect.size,
            Size {
                width: 125.0,
                height: 50.0
            }
        );
        pipeline.graph.prepare_root_sequence_alignment_records();
        assert_eq!(pipeline.graph.sized_edge_abductions.len(), 2);
        let baseline = pipeline.graph.global_sized_edge_length_for_alignment();
        assert!(
            baseline.is_finite() && baseline > 0.0,
            "root score: {baseline}"
        );
        pipeline.graph.translate_owner_point(vessel, -20.0, 0.0);
        pipeline.graph.sync_sequences();
        assert_eq!(
            pipeline.graph.position(first),
            Some(Point { x: -20.0, y: 100.0 })
        );
        assert_eq!(
            pipeline.graph.position(second),
            Some(Point { x: 25.0, y: 100.0 })
        );
        let kind = super::super::owner_groups::OwnerGroupKind::Sequence;
        let map = pipeline
            .graph
            .owner_graph_group_map(OwnerGraphId(0), kind)
            .unwrap();
        pipeline
            .graph
            .write_owner_group_map_entry(&map, Some(vessel), None)
            .unwrap();
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                pipeline
                    .graph
                    .alignment_original_endpoint_box(outgoing, vessel)
            }))
            .is_err()
        );
    }

    #[test]
    fn alignment_connected_node_views_keep_pointer_visible_scope_edges() {
        let mut input = Graph::default();
        let first = input.add_node(Node::new(
            "first",
            Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        let second = input.add_node(Node::new(
            "second",
            Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        let edge = input.add_edge(Edge {
            source: first,
            target: second,
        });
        let graph = ArenaGraph::from_input(&input);

        // Model an induced placement scope: Graph.Edges no longer contains
        // this cross-scope edge, but the source Node.Edges slice still does.
        graph.edge_order.retain(|&active| active != edge);
        assert!(!graph.active_edge_ids(first).contains(&edge));
        assert!(graph.nodes[first.0 as usize].edges.contains(&edge));

        assert_eq!(
            graph.connected_node_views(ConnectedNode::Active(first), &[]),
            vec![ConnectedNode::Active(first), ConnectedNode::Active(second)]
        );
    }

    #[test]
    fn alignment_score_comparison_matches_recovered_geo_precision() {
        assert_eq!(
            alignment_precision_compare(10.0 - 0.000_099, 10.0),
            Ordering::Equal
        );
        assert_eq!(
            alignment_precision_compare(0.0, ALIGNMENT_PRECISION),
            Ordering::Less
        );
        assert_eq!(
            alignment_precision_compare(ALIGNMENT_PRECISION, 0.0),
            Ordering::Greater
        );
    }

    #[test]
    fn rejected_alignment_attempts_restore_shared_points_before_next_axis() {
        let mut input = Graph::default();
        let size = Size {
            width: 40.0,
            height: 30.0,
        };
        let from = input.add_node(Node::new("from", size));
        let to = input.add_node(Node::new("to", size));
        let blocker = input.add_node(Node::new("blocker", size));
        let edge = input.add_edge(Edge {
            source: from,
            target: to,
        });
        let mut graph = ArenaGraph::from_input(&input);
        graph.initialize_shared_topology();
        graph.set_position(from, Point { x: 0.0, y: 0.0 });
        graph.set_position(to, Point { x: 400.0, y: 200.0 });
        graph.set_position(blocker, Point { x: 200.0, y: 200.0 });
        graph.materialize_owner_node_point(from).unwrap();
        let retained = graph.owner_node_point(from).unwrap();
        let snapshot = AlignmentTrialSnapshot::capture_with_overlap_state(
            &graph,
            graph.overlap_pairs(false),
            graph.exact_overlap_pairs(),
        );
        assert!(snapshot.hierarchy_materialized);
        let geometry = |graph: &ArenaGraph| {
            [from, to, blocker].map(|node| (graph.position(node), graph.active_node_size(node)))
        };
        let original = geometry(&graph);
        // The vertical slide is obstructed; the large slide exceeds max size.
        // Both must roll back the existing point allocation before returning.
        for y in [200.0, 31_000.0] {
            assert!(
                graph
                    .try_alignment_move(
                        edge,
                        &[ConnectedNode::Active(from)],
                        Point { x: 0.0, y },
                        &snapshot
                    )
                    .is_none()
            );
            assert_eq!(retained.get(), Some(Point { x: 0.0, y: 0.0 }));
            assert!(retained.same_allocation(&graph.owner_node_point(from).unwrap()));
            assert_eq!(geometry(&graph), original);
            // A rejected first axis must not suppress the legal second axis.
            let (score, delta) = graph
                .try_alignment_move(
                    edge,
                    &[ConnectedNode::Active(from)],
                    Point { x: 400.0, y },
                    &snapshot,
                )
                .expect("legal horizontal attempt");
            assert!(score.is_finite());
            assert_eq!(delta, Point { x: 400.0, y: 0.0 });
            assert_eq!(retained.get(), Some(Point { x: 0.0, y: 0.0 }));
            assert_eq!(geometry(&graph), original);
        }
    }
}
