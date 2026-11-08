// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Transactional geometry updates and rollback validation.
//!
//! Placement trials may move projected carriers and real descendants together.
//! This module snapshots the affected geometry, validates global invariants,
//! and either commits the complete move or restores the prior state.

use super::model::{ProjectedTransactionEdge, ProjectedTransactionNode, ProjectedTransactionState};
use super::scoring_context::{ScoreContext, score_interval, score_poll};
use super::*;

use crate::internal::{IdHashMap, IdHashSet};
use std::borrow::Cow;

/// The exact bit pattern of a point, distinguishing -0.0 and NaN payloads.
#[inline]
fn point_bits(point: Point) -> (u64, u64) {
    (point.x.to_bits(), point.y.to_bits())
}

#[inline]
fn size_bits(size: Size) -> (u64, u64) {
    (size.width.to_bits(), size.height.to_bits())
}

/// How many recent fixpoints each projection memo keeps.
const PROJECTION_MEMO_ENTRIES: usize = 32;

/// The size the last of `updates` assigns to `tala_id`, if any.
#[inline]
fn last_size_update(updates: &[(i64, Size)], tala_id: i64) -> Option<Size> {
    updates
        .iter()
        .rev()
        .find(|(id, _)| *id == tala_id)
        .map(|&(_, size)| size)
}

/// Preserve Go 1.27's ARM64 fused crossing-penalty accumulation in
/// `placementcost.EdgeLength`. The pinned oracle emits FMADD for
/// `total += crossingCost * crossings` on ARM64; other Go targets round the
/// product before adding it.
#[inline]
fn go_127_crossing_penalty_total(edge_length: f64, crossings: usize, crossing_cost: f64) -> f64 {
    #[cfg(target_arch = "aarch64")]
    {
        crossing_cost.mul_add(crossings as f64, edge_length)
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        edge_length + crossings as f64 * crossing_cost
    }
}

/// Match D2 v0.9.0's fixed candidate-distance filter in
/// `Graph.doesOverlapWithDimensionsContext`. The oracle applies this 500-unit
/// bound before asking for the edge-dependent spacing delta, even when that
/// delta is larger than 500; preserve that behavior for transaction parity.
#[inline]
fn transaction_overlap_prefilter_excludes_pair(
    node_position: Point,
    node_size: Size,
    other_position: Point,
    other_size: Size,
) -> bool {
    const MAX_SAFE_DELTA: f64 = 500.0;
    node_position.x > other_position.x + other_size.width + MAX_SAFE_DELTA
        || node_position.x + node_size.width + MAX_SAFE_DELTA < other_position.x
        || node_position.y > other_position.y + other_size.height + MAX_SAFE_DELTA
        || node_position.y + node_size.height + MAX_SAFE_DELTA < other_position.y
}

/// Membership view of a projected state's existing overlap index pairs.
/// Small states use a dense `left * len + right` bitset instead of hashing
/// every probed pair; larger ones keep the hash set.
struct ProjectedOverlapPairs {
    len: usize,
    dense: Vec<u64>,
    sparse: IdHashSet<(usize, usize)>,
}
impl ProjectedOverlapPairs {
    const DENSE_LIMIT: usize = 1 << 20;
    fn new(len: usize, pairs: &BTreeSet<(usize, usize)>) -> Self {
        let mut lookup = Self {
            len,
            dense: Vec::new(),
            sparse: IdHashSet::default(),
        };
        if len
            .checked_mul(len)
            .is_some_and(|bits| bits <= Self::DENSE_LIMIT)
        {
            lookup.dense = vec![0; (len * len).div_ceil(64)];
            for &(left, right) in pairs {
                // Probes only name in-range indices; others can never match.
                if left < len && right < len {
                    let bit = left * len + right;
                    lookup.dense[bit / 64] |= 1u64 << (bit % 64);
                }
            }
        } else {
            lookup.sparse = pairs.iter().copied().collect();
        }
        lookup
    }
    #[inline]
    fn contains(&self, &(left, right): &(usize, usize)) -> bool {
        if self.dense.is_empty() {
            return self.sparse.contains(&(left, right));
        }
        let bit = left * self.len + right;
        self.dense[bit / 64] & (1u64 << (bit % 64)) != 0
    }
}

pub(super) enum ActiveEdgeIds<'a> {
    Borrowed(&'a [EdgeId]),
    Owned(Vec<EdgeId>),
}

impl std::ops::Deref for ActiveEdgeIds<'_> {
    type Target = [EdgeId];

    fn deref(&self) -> &Self::Target {
        match self {
            Self::Borrowed(edges) => edges,
            Self::Owned(edges) => edges,
        }
    }
}

pub(super) enum ActiveEdgeIdsIter<'a> {
    Borrowed(std::iter::Copied<std::slice::Iter<'a, EdgeId>>),
    Owned(std::vec::IntoIter<EdgeId>),
}

impl Iterator for ActiveEdgeIdsIter<'_> {
    type Item = EdgeId;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Borrowed(iter) => iter.next(),
            Self::Owned(iter) => iter.next(),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match self {
            Self::Borrowed(iter) => iter.size_hint(),
            Self::Owned(iter) => iter.size_hint(),
        }
    }
}

impl ExactSizeIterator for ActiveEdgeIdsIter<'_> {}

impl<'a> IntoIterator for ActiveEdgeIds<'a> {
    type Item = EdgeId;
    type IntoIter = ActiveEdgeIdsIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        match self {
            Self::Borrowed(edges) => ActiveEdgeIdsIter::Borrowed(edges.iter().copied()),
            Self::Owned(edges) => ActiveEdgeIdsIter::Owned(edges.into_iter()),
        }
    }
}

/// Go's Graph.EdgeLengthState uses FNV-1a over the scoring flags, costs, node
/// boxes, and current Node.Edges identities. Keep the same compact state key
/// for the recovered edgeLengthCache rather than hashing Rust addresses.
#[derive(Clone, Copy)]
struct EdgeLengthStateHasher(u64);

impl EdgeLengthStateHasher {
    fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }

    fn write_bytes(&mut self, bytes: &[u8]) {
        for byte in bytes {
            // FNV-1a's 64-bit prime is 0x100000001b3 (2^40 + 0x1b3).
            // Keep the grouping explicit so an extra zero cannot silently
            // turn this into a different cache hash.
            self.0 = (self.0 ^ u64::from(*byte)).wrapping_mul(0x100_0000_01b3);
        }
    }

    fn write_u64(&mut self, value: u64) {
        self.write_bytes(&value.to_le_bytes());
    }

    fn write_entity_id(&mut self, value: i64) {
        self.write_bytes(&value.to_le_bytes());
    }

    fn write_f64(&mut self, value: f64) {
        self.write_u64(value.to_bits());
    }

    fn write_node(&mut self, node: NodeId) {
        self.write_u64(node.0 as u64);
    }

    fn write_optional_node(&mut self, node: Option<NodeId>) {
        match node {
            Some(node) => {
                self.write_u64(1);
                self.write_node(node);
            }
            None => self.write_u64(0),
        }
    }

    fn write_optional_entity_id(&mut self, value: Option<i64>) {
        match value {
            Some(value) => {
                self.write_u64(1);
                self.write_bytes(&value.to_le_bytes());
            }
            None => self.write_u64(0),
        }
    }

    fn write_point(&mut self, point: Point) {
        self.write_f64(point.x);
        self.write_f64(point.y);
    }

    fn write_size(&mut self, size: Size) {
        self.write_f64(size.width);
        self.write_f64(size.height);
    }

    fn write_projected_adjacent(&mut self, adjacent: &ProjectedAdjacent) {
        self.write_node(adjacent.owner);
        self.write_entity_id(adjacent.tala_id);
        self.write_optional_entity_id(adjacent.container_tala_id);
        self.write_point(adjacent.offset);
        self.write_size(adjacent.size);
        self.write_u64(u64::from(adjacent.cluster_member));
    }

    fn write_optional_abduction_current(&mut self, endpoint: Option<AbductionCurrentNodeRef>) {
        let Some(endpoint) = endpoint else {
            self.write_u64(0);
            return;
        };
        self.write_u64(1);
        match endpoint.location {
            AbductionNodeLocation::Local(node) => {
                self.write_u64(1);
                self.write_node(node);
            }
            AbductionNodeLocation::Foreign => self.write_u64(2),
        }
        self.write_optional_node(endpoint.source_node);
        self.write_optional_entity_id(endpoint.tala_id);
        self.write_optional_entity_id(endpoint.container_tala_id);
        match endpoint.position {
            Some(position) => {
                self.write_u64(1);
                self.write_point(position);
            }
            None => self.write_u64(0),
        }
        match endpoint.size {
            Some(size) => {
                self.write_u64(1);
                self.write_size(size);
            }
            None => self.write_u64(0),
        }
        self.write_u64(u64::from(endpoint.is_aggregate_vessel));
    }

    fn write_optional_projected_adjacent(&mut self, adjacent: Option<&ProjectedAdjacent>) {
        match adjacent {
            Some(adjacent) => {
                self.write_u64(1);
                self.write_projected_adjacent(adjacent);
            }
            None => self.write_u64(0),
        }
    }

    fn write_optional_abducted_node(&mut self, node: Option<&AbductedNodeRef>) {
        match node {
            None => self.write_u64(0),
            Some(node) => {
                self.write_u64(1);
                self.write_optional_node(node.source_node);
                self.write_optional_node(node.source_owner);
                self.write_optional_entity_id(node.owner_tala_id);
                self.write_optional_node(node.owner);
                self.write_entity_id(node.tala_id);
                self.write_optional_entity_id(node.container_tala_id);
                match node.offset {
                    Some(offset) => {
                        self.write_u64(1);
                        self.write_point(offset);
                    }
                    None => self.write_u64(0),
                }
                self.write_size(node.size);
                self.write_u64(u64::from(node.cluster_member));
            }
        }
    }

    fn write_adjacent_list(&mut self, adjacent: &[ProjectedAdjacent]) {
        self.write_u64(adjacent.len() as u64);
        for adjacent in adjacent {
            self.write_projected_adjacent(adjacent);
        }
    }

    fn write_obstruction_inventory(&mut self, inventory: &OrderedObstructionInventory) {
        self.write_u64(u64::from(inventory.sets_present));
        self.write_optional_node(inventory.node_replacement);
        self.write_optional_node(inventory.adjacent_replacement);
        self.write_u64(inventory.sets.len() as u64);
        for set in inventory.sets.iter() {
            self.write_optional_node(set.source_container);
            self.write_optional_entity_id(set.container_tala_id);
            self.write_u64(u64::from(set.items.is_some()));
            let Some(items) = &set.items else {
                continue;
            };
            self.write_u64(items.len() as u64);
            for item in items {
                self.write_optional_node(item.source_node);
                self.write_optional_node(item.source_owner);
                match item.location {
                    AbductionNodeLocation::Local(node) => {
                        self.write_u64(1);
                        self.write_node(node);
                    }
                    AbductionNodeLocation::Foreign => self.write_u64(0),
                }
                self.write_entity_id(item.tala_id);
                self.write_optional_node(item.source_container);
                self.write_optional_entity_id(item.container_tala_id);
                self.write_u64(u64::from(item.position.is_some()));
                if let Some(position) = item.position {
                    self.write_point(position);
                }
                self.write_size(item.size);
                self.write_optional_projected_adjacent(item.projection.as_ref());
                self.write_u64(u64::from(item.same_graph_as_replacement));
                self.write_u64(u64::from(item.is_endpoint));
                self.write_u64(u64::from(item.is_replacement_ancestor));
                self.write_u64(u64::from(item.is_cluster_vessel));
                self.write_u64(u64::from(item.is_endpoint_cluster_vessel));
            }
        }
    }

    fn write_arrangement(&mut self, arrangement: ClusterArrangement) {
        self.write_u64(match arrangement {
            ClusterArrangement::Row => 0,
            ClusterArrangement::Column => 1,
        });
    }

    fn finish(self) -> u64 {
        self.0
    }
}

impl ArenaGraph {
    /// Diagnostic snapshot for a shared ordinary-container witness. This is
    /// deliberately opt-in so release builds never pay for formatting or
    /// walking the projection maps.
    pub(super) fn trace_target_shared_geometry(&self, phase: &str) {
        let Some(trace_mode) = crate::internal::trace_env_value("TALA_TRACE_TARGET_GEOMETRY")
        else {
            return;
        };
        if trace_mode == "move-focus" && !phase.starts_with("move-focus") {
            return;
        }
        if trace_mode == "direct-focus"
            && !matches!(
                phase,
                "mirror-after-publish" | "mirror-after-reconcile" | "direct-after-mirror"
            )
        {
            return;
        }
        if trace_mode == "mirror-all"
            && !matches!(
                phase,
                "mirror-before" | "mirror-after-publish" | "mirror-after-reconcile"
            )
        {
            return;
        }
        if trace_mode == "copyback-focus" && !phase.starts_with("copyback-") {
            return;
        }
        let ids = [4254926550_i64, 1369972579, 3471367806];
        let active = self
            .nodes
            .iter()
            .filter(|node| ids.contains(&node.tala_id))
            .map(|node| (node.tala_id, node.position, node.rect.size))
            .collect::<Vec<_>>();
        let external_containers = self
            .transaction_external_containers
            .iter()
            .filter(|node| ids.contains(&node.tala_id))
            .map(|node| (node.tala_id, node.position, node.rect.size))
            .collect::<Vec<_>>();
        let ordinary = self
            .transaction_external_container_children
            .iter()
            .filter_map(|(parent, children)| {
                let selected = children
                    .iter()
                    .filter(|child| ids.contains(&child.tala_id))
                    .map(|child| (child.tala_id, child.position, child.rect.size))
                    .collect::<Vec<_>>();
                (!selected.is_empty() || ids.contains(parent)).then_some((*parent, selected))
            })
            .collect::<Vec<_>>();
        if trace_mode == "copyback-negative" {
            let relevant = ordinary.iter().any(|(parent, children)| {
                *parent == 4254926550
                    && children.iter().any(|(tala_id, position, _)| {
                        *tala_id == 3471367806
                            && position.is_some_and(|position| position.y < -1000.0)
                    })
            });
            if !relevant {
                return;
            }
        }
        if trace_mode == "focus" {
            let focus_state = self
                .nodes
                .iter()
                .find(|node| node.tala_id == 1051177149)
                .and_then(|node| node.position)
                .is_some_and(|position| {
                    (position.x - 300.0).abs() < f64::EPSILON
                        && (position.y + 1425.0).abs() < f64::EPSILON
                });
            if !focus_state {
                return;
            }
        }
        eprintln!(
            "TARGET_GEOMETRY_RUST phase={} active={:?} external_containers={:?} ordinary={:?}",
            phase, active, external_containers, ordinary
        );
    }

    /// Rebuild projected edge offsets from the external child boxes after a
    /// transaction rollback. TALA's induced graphs retain shared `*Node`
    /// pointers; restoring the owner with `moveNodeAbsWithChildren` can move
    /// an external child by one cell even though the owner returns to its
    /// saved box. The flattened Rust carrier must observe that resulting
    /// relative position before the next scoring pass.
    pub(super) fn reconcile_projected_offsets_from_external_children(&mut self) {
        self.reconcile_projected_offsets_from_external_children_with_layout(true);
    }

    /// Rebuild projections after `Transaction.Rollback` without running the
    /// cluster arrange step.  The recovered Go rollback first restores the
    /// concrete cluster-member pointers independently, leaving the temporary
    /// vessel's move-with-children effect visible until the next
    /// `Graph.syncClusters` boundary.  Re-arranging aggregate members here
    /// would publish that later boundary too early and changes the next
    /// optimizer median by one pixel.
    pub(super) fn reconcile_projected_offsets_after_rollback(&mut self) {
        self.reconcile_projected_offsets_from_external_children_with_layout(false);
    }

    fn reconcile_projected_offsets_from_external_children_with_layout(
        &mut self,
        arrange_aggregate_members: bool,
    ) {
        // A nested cluster's member slice is retained separately from the
        // direct vessel child.  Recursive placement updates the vessel in the
        // ordinary container projection, while the aggregate member copies
        // can still hold their inner-scope frame.  Reapply the recovered
        // arrange-cluster geometry before deriving endpoint offsets so the
        // member pointers follow that vessel exactly.
        if arrange_aggregate_members {
            let mut aggregate_positions = Vec::new();
            for (&vessel_tala_id, members) in &self.transaction_external_aggregate_children {
                let Some(layout) = self
                    .transaction_external_cluster_layouts
                    .get(&vessel_tala_id)
                else {
                    continue;
                };
                let Some(vessel) = self
                    .transaction_external_container_children
                    .values()
                    .flatten()
                    .find(|candidate| candidate.tala_id == vessel_tala_id)
                else {
                    continue;
                };
                let Some(vessel_position) = vessel.position else {
                    continue;
                };
                let mut cursor = match layout.arrangement {
                    ClusterArrangement::Row => vessel_position.x,
                    ClusterArrangement::Column => vessel_position.y,
                };
                for member in members {
                    let Some(member_position) = member.position else {
                        continue;
                    };
                    let position = match layout.arrangement {
                        ClusterArrangement::Row => Point {
                            x: cursor,
                            y: (vessel_position.y + vessel.rect.size.height * 0.5
                                - member.rect.size.height * 0.5)
                                .round(),
                        },
                        ClusterArrangement::Column => Point {
                            x: (vessel_position.x + vessel.rect.size.width * 0.5
                                - member.rect.size.width * 0.5)
                                .round(),
                            y: cursor,
                        },
                    };
                    if position != member_position {
                        aggregate_positions.push((vessel_tala_id, member.tala_id, position));
                    }
                    match layout.arrangement {
                        ClusterArrangement::Row => {
                            cursor += member.rect.size.width + layout.padding;
                        }
                        ClusterArrangement::Column => {
                            cursor += member.rect.size.height + layout.padding;
                        }
                    }
                }
            }
            for (vessel_tala_id, member_tala_id, position) in aggregate_positions {
                if crate::internal::trace_env_enabled("TALA_TRACE_OBJECT2")
                    && vessel_tala_id == 6334824724549167320
                {
                    eprintln!(
                        "OBJECT2_RECONCILE_SET_RUST member={} target={},{}",
                        member_tala_id, position.x, position.y
                    );
                }
                if let Some(member) = self
                    .transaction_external_aggregate_children
                    .get_mut(&vessel_tala_id)
                    .and_then(|members| {
                        members
                            .iter_mut()
                            .find(|member| member.tala_id == member_tala_id)
                    })
                {
                    member.position = Some(position);
                    member.rect.origin = position;
                }
            }
        }
        // The recovered cluster vessel is a live `*Node` whose box follows
        // the fitted member slice.  `ProjectedClusterDistance` retains that
        // vessel box separately from each member endpoint, so publish the
        // same min-member origin before scoring the next candidate.
        let vessel_offsets =
            self.sized_cluster_distance_boxes
                .iter()
                .filter_map(|(&(owner, member_tala_id), geometry)| {
                    let members = self
                        .transaction_external_aggregate_children
                        .get(&geometry.vessel_tala_id)?;
                    let vessel_position = members
                        .iter()
                        .filter_map(|member| member.position)
                        .reduce(|left, right| Point {
                            x: left.x.min(right.x),
                            y: left.y.min(right.y),
                        })?;
                    let owner_position = self.position(owner)?;
                    Some((
                        (owner, member_tala_id),
                        Point {
                            x: vessel_position.x - owner_position.x,
                            y: vessel_position.y - owner_position.y,
                        },
                    ))
                })
                .collect::<Vec<_>>();
        for (key, offset) in vessel_offsets {
            // Write only a changed offset, so an unchanged map keeps its
            // generation for the memoized passes below.
            if self
                .sized_cluster_distance_boxes
                .get(&key)
                .is_some_and(|geometry| point_bits(geometry.offset) != point_bits(offset))
                && let Some(geometry) = self.sized_cluster_distance_boxes.get_mut(&key)
            {
                geometry.offset = offset;
            }
        }
        // The positions feed only the sized projections.
        if !self.has_sized_projections()
            && !crate::internal::trace_env_enabled("TALA_TRACE_PROJECTED_RECONCILE")
        {
            return;
        }
        let mut positions = IdHashMap::<i64, Option<Point>>::default();
        positions.reserve(
            self.transaction_external_container_children
                .values()
                .map(Vec::len)
                .sum::<usize>()
                + self
                    .transaction_external_aggregate_children
                    .values()
                    .map(Vec::len)
                    .sum::<usize>(),
        );
        for children in self.transaction_external_container_children.values() {
            for child in children {
                positions.insert(child.tala_id, child.position);
            }
        }
        // Cluster members are published through the aggregate carrier rather
        // than the direct container-child map.  They still participate in
        // sized-optimizer endpoint projections, so include their live boxes
        // before rebuilding owner-relative offsets.
        for members in self.transaction_external_aggregate_children.values() {
            for member in members {
                positions.insert(member.tala_id, member.position);
            }
        }
        if crate::internal::trace_env_enabled("TALA_TRACE_PROJECTED_RECONCILE")
            && let Some(owner) = self.nodes.iter().find(|node| node.tala_id == 3_447_521_236)
        {
            let direct = self
                .transaction_external_container_children
                .values()
                .flatten()
                .find(|child| child.tala_id == 5_577_006_791_947_779_410)
                .and_then(|child| child.position);
            let aggregate = self
                .transaction_external_aggregate_children
                .get(&5_577_006_791_947_779_410)
                .map(|children| {
                    children
                        .iter()
                        .map(|child| (child.tala_id, child.position))
                        .collect::<Vec<_>>()
                });
            eprintln!(
                "PROJECTED_RECONCILE_STATE_RUST owner={:?} direct={:?} aggregate={:?}",
                owner.position, direct, aggregate
            );
        }
        self.reconcile_projected_offsets_from_positions(&positions);
    }

    /// `Graph.syncClusters` rearranges the original cluster-member pointers
    /// retained by an induced graph. Publish those new absolute boxes into
    /// the flattened endpoint projections at the same lifecycle boundary.
    pub(super) fn reconcile_projected_offsets_from_external_cluster(
        &mut self,
        vessel_tala_id: i64,
    ) {
        let Some(members) = self
            .transaction_external_aggregate_children
            .get(&vessel_tala_id)
        else {
            return;
        };
        let positions = members
            .iter()
            .map(|member| (member.tala_id, member.position))
            .collect::<IdHashMap<_, _>>();
        self.reconcile_projected_offsets_from_positions(&positions);
    }

    fn has_sized_projections(&self) -> bool {
        !(self.sized_adjacent_overrides.is_empty()
            && self.sized_edge_abductions.is_empty()
            && self.sized_projected_obstructions.is_empty()
            && self.sized_projected_obstruction_sets.is_empty()
            && self.sized_collapsed_symmetry_neighbors.is_empty()
            && self.sized_cluster_distance_boxes.is_empty())
    }

    fn reconcile_projected_offsets_from_positions(
        &mut self,
        positions: &IdHashMap<i64, Option<Point>>,
    ) {
        if positions.is_empty() || !self.has_sized_projections() {
            return;
        }
        // Projected owners are stable arena IDs. Keep their indexed values
        // directly instead of rebuilding a TALA-ID tree map on every trial.
        let owner_positions = self
            .nodes
            .iter()
            .map(|node| node.position)
            .collect::<Vec<_>>();
        let trace_reconcile = crate::internal::trace_env_enabled("TALA_TRACE_PROJECTED_RECONCILE");
        // The pass writes each matched offset from the child and owner
        // positions alone, so once it has run, rerunning it with bit-equal
        // positions over caches of unchanged generation is a no-op.
        let generations = self.projection_generations();
        let owner_bits = owner_positions
            .iter()
            .map(|position| position.map(point_bits))
            .collect::<Vec<_>>();
        let same_positions = |recorded: &[(i64, Option<(u64, u64)>)]| {
            recorded.len() == positions.len()
                && recorded.iter().all(|(tala_id, bits)| {
                    positions
                        .get(tala_id)
                        .is_some_and(|position| position.map(point_bits) == *bits)
                })
        };
        if !trace_reconcile
            && self.projection_memo.reconciled.iter().any(
                |(recorded_generations, recorded_owners, recorded_positions)| {
                    *recorded_generations == generations
                        && *recorded_owners == owner_bits
                        && same_positions(recorded_positions)
                },
            )
        {
            return;
        }
        // Only the trace below names owners.
        let owner_tala_ids = if trace_reconcile {
            self.nodes
                .iter()
                .map(|node| node.tala_id)
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let changed = std::cell::Cell::new(false);
        let reconcile = |projected: &mut ProjectedAdjacent| {
            let Some(child_position) = positions.get(&projected.tala_id).copied().flatten() else {
                return;
            };
            let Some(owner_position) = owner_positions
                .get(projected.owner.0 as usize)
                .copied()
                .flatten()
            else {
                return;
            };
            let offset = Point {
                x: child_position.x - owner_position.x,
                y: child_position.y - owner_position.y,
            };
            if point_bits(offset) != point_bits(projected.offset) {
                changed.set(true);
            }
            projected.offset = offset;
            if trace_reconcile
                && (crate::internal::trace_env_value("TALA_TRACE_PROJECTED_RECONCILE").as_deref()
                    == Some("all")
                    || crate::internal::trace_env_value("TALA_TRACE_PROJECTED_RECONCILE")
                        .and_then(|target| target.parse::<i64>().ok())
                        == Some(projected.tala_id))
            {
                eprintln!(
                    "PROJECTED_RECONCILE_RUST child={} owner={} child_pos={},{} owner_pos={},{} offset={},{}",
                    projected.tala_id,
                    owner_tala_ids[projected.owner.0 as usize],
                    child_position.x,
                    child_position.y,
                    owner_position.x,
                    owner_position.y,
                    projected.offset.x,
                    projected.offset.y
                );
            }
        };
        for projected in self.sized_adjacent_overrides.values_mut() {
            reconcile(projected);
        }
        // A known nil TopLeft differs from an absent lookup entry. Retained
        // originals observe later child placement and rollback on the same
        // identity; only actual child/owner positions create an offset.
        let reconcile_original = |original: &mut AbductedNodeRef| {
            let Some(position) = positions.get(&original.tala_id) else {
                return;
            };
            let offset = position.and_then(|child| {
                let owner = owner_positions
                    .get(original.owner?.0 as usize)
                    .copied()
                    .flatten()?;
                Some(Point {
                    x: child.x - owner.x,
                    y: child.y - owner.y,
                })
            });
            if offset.map(point_bits) != original.offset.map(point_bits) {
                changed.set(true);
            }
            original.offset = offset;
        };
        for abduction in &mut self.sized_edge_abductions {
            if let Some(original) = &mut abduction.originally_from {
                reconcile_original(original);
            }
            if let Some(original) = &mut abduction.originally_to {
                reconcile_original(original);
            }
            for neighbor in &mut abduction.originally_from_table_neighbors {
                reconcile(&mut neighbor.other);
            }
            for neighbor in &mut abduction.originally_to_table_neighbors {
                reconcile(&mut neighbor.other);
            }
            for projected in &mut abduction.obstructions_from_to {
                reconcile(projected);
            }
            for projected in &mut abduction.obstructions_to_from {
                reconcile(projected);
            }
            abduction
                .obstruction_sets_from_to
                .update_projections(reconcile);
            abduction
                .obstruction_sets_to_from
                .update_projections(reconcile);
        }
        for projections in self.sized_projected_obstructions.values_mut() {
            projections.update_projections(reconcile);
        }
        for inventory in self.sized_projected_obstruction_sets.values_mut() {
            inventory.update_projections(reconcile);
        }
        for projected in self
            .sized_collapsed_symmetry_neighbors
            .values_mut()
            .flatten()
        {
            reconcile(projected);
        }
        for geometry in self.sized_cluster_distance_boxes.values_mut() {
            for projected in &mut geometry.external_connected {
                reconcile(projected);
            }
        }
        if !changed.get() {
            self.restore_unchanged_projection_generations(generations);
        }
        let recorded_positions = positions
            .iter()
            .map(|(&tala_id, position)| (tala_id, position.map(point_bits)))
            .collect();
        let generations = self.projection_generations();
        let memo = &mut self.projection_memo.reconciled;
        if memo.len() == PROJECTION_MEMO_ENTRIES {
            memo.remove(0);
        }
        memo.push((generations, owner_bits, recorded_positions));
    }

    /// Generation stamps of the six sized projection caches.
    pub(super) fn projection_generations(&self) -> ProjectionGenerations {
        [
            self.sized_adjacent_overrides.generation(),
            self.sized_cluster_distance_boxes.generation(),
            self.sized_edge_abductions.generation(),
            self.sized_projected_obstructions.generation(),
            self.sized_projected_obstruction_sets.generation(),
            self.sized_collapsed_symmetry_neighbors.generation(),
        ]
    }

    /// After a mutable pass that changed no cache bit, keep the stamps the
    /// caches had before it.
    pub(super) fn restore_unchanged_projection_generations(
        &mut self,
        generations: ProjectionGenerations,
    ) {
        self.sized_adjacent_overrides
            .restore_unchanged_generation(generations[0]);
        self.sized_cluster_distance_boxes
            .restore_unchanged_generation(generations[1]);
        self.sized_edge_abductions
            .restore_unchanged_generation(generations[2]);
        self.sized_projected_obstructions
            .restore_unchanged_generation(generations[3]);
        self.sized_projected_obstruction_sets
            .restore_unchanged_generation(generations[4]);
        self.sized_collapsed_symmetry_neighbors
            .restore_unchanged_generation(generations[5]);
    }

    /// Keep flattened projections equivalent to TALA's shared original-node
    /// pointers when `wrapChildren` moves a carrier around unchanged hidden
    /// descendants.
    pub(super) fn adjust_projected_offsets_for_refit(&mut self, owner: NodeId, delta: Point) {
        if delta.x == 0.0 && delta.y == 0.0 {
            return;
        }
        let trace_adjust = crate::internal::trace_env_enabled("TALA_TRACE_PROJECTED_REFIT");
        let owner_tala_id = self.nodes[owner.0 as usize].tala_id;
        let adjust = |projected: &mut ProjectedAdjacent| {
            if projected.owner == owner {
                if trace_adjust && projected.tala_id == 3_302_651_804 {
                    eprintln!(
                        "PROJECTED_REFIT_RUST owner={} child={} before={},{} delta={},{}",
                        owner_tala_id,
                        projected.tala_id,
                        projected.offset.x,
                        projected.offset.y,
                        delta.x,
                        delta.y
                    );
                }
                projected.offset.x -= delta.x;
                projected.offset.y -= delta.y;
                if trace_adjust && projected.tala_id == 3_302_651_804 {
                    eprintln!(
                        "PROJECTED_REFIT_RUST after={},{}",
                        projected.offset.x, projected.offset.y
                    );
                }
            }
        };
        for projected in self.sized_adjacent_overrides.values_mut() {
            adjust(projected);
        }
        let adjust_original = |original: &mut AbductedNodeRef| {
            if original.owner == Some(owner)
                && let Some(offset) = &mut original.offset
            {
                offset.x -= delta.x;
                offset.y -= delta.y;
            }
        };
        for abduction in &mut self.sized_edge_abductions {
            if let Some(original) = &mut abduction.originally_from {
                adjust_original(original);
            }
            if let Some(original) = &mut abduction.originally_to {
                adjust_original(original);
            }
            for neighbor in &mut abduction.originally_from_table_neighbors {
                adjust(&mut neighbor.other);
            }
            for neighbor in &mut abduction.originally_to_table_neighbors {
                adjust(&mut neighbor.other);
            }
            for projected in &mut abduction.obstructions_from_to {
                adjust(projected);
            }
            for projected in &mut abduction.obstructions_to_from {
                adjust(projected);
            }
            abduction
                .obstruction_sets_from_to
                .update_projections(adjust);
            abduction
                .obstruction_sets_to_from
                .update_projections(adjust);
        }
        for projections in self.sized_projected_obstructions.values_mut() {
            projections.update_projections(adjust);
        }
        for inventory in self.sized_projected_obstruction_sets.values_mut() {
            inventory.update_projections(adjust);
        }
        for projected in self
            .sized_collapsed_symmetry_neighbors
            .values_mut()
            .flatten()
        {
            adjust(projected);
        }
        for (&(carrier, _), geometry) in &mut self.sized_cluster_distance_boxes {
            if carrier == owner {
                geometry.offset.x -= delta.x;
                geometry.offset.y -= delta.y;
            }
            for projected in &mut geometry.external_connected {
                adjust(projected);
            }
        }
    }

    pub(super) fn initialize_transaction_projected_positions(&mut self) {
        let anchor_positions = self
            .nodes
            .iter()
            .filter_map(|node| Some((node.tala_id, node.position?)))
            .collect::<BTreeMap<_, _>>();
        let live_container_tala_ids = self
            .node_order
            .iter()
            .filter_map(|node| {
                let node = &self.nodes[node.0 as usize];
                node.is_container.then_some(node.tala_id)
            })
            .collect::<BTreeSet<_>>();
        let initialize = |node: &mut ArenaNode| {
            let position_was_nil = node.position.is_none();
            if position_was_nil
                && let Some((anchor, offset)) = node
                    .transaction_anchor_tala_id
                    .zip(node.transaction_anchor_offset)
                && let Some(anchor_position) = anchor_positions.get(&anchor)
            {
                let position = Point {
                    x: anchor_position.x + offset.x,
                    y: anchor_position.y + offset.y,
                };
                node.position = Some(position);
                node.rect.origin = position;
                node.transaction_position_was_nil |= position_was_nil;
            }
            // This projection bridge represents TALA's initial shared-pointer
            // state. Subsequent node and subtree translations update the cloned
            // boxes directly; wrapChildren intentionally moves only its
            // container and must not re-anchor the children.
            node.transaction_anchor_tala_id = None;
            node.transaction_anchor_offset = None;
        };
        for (&container_tala_id, children) in &mut self.transaction_external_container_children {
            let live_container = live_container_tala_ids.contains(&container_tala_id);
            for child in children {
                // Aggregate-member projections may be keyed by their vessel,
                // which is not an ordinary container key in the temporary
                // graph. They still carry an anchor to that vessel and must
                // be initialized once the vessel itself has a position.
                if live_container || child.transaction_anchor_tala_id.is_some() {
                    initialize(child);
                }
            }
        }
        for children in self.transaction_external_aggregate_children.values_mut() {
            for child in children {
                if child.transaction_anchor_tala_id.is_some() {
                    initialize(child);
                }
            }
        }
    }

    /// Project the container and cluster-vessel portions of recovered
    /// `Graph.syncNested` through an induced graph's shared pointer maps.
    ///
    /// The release performs both operations in `Graph.Nodes` order. An
    /// ordinary container first positions its children with shape padding. A
    /// cluster vessel then synchronizes its member boxes and moves each
    /// container member's direct child subtrees by that member's full
    /// `getContainerPadding(member, true)`.
    #[cfg_attr(feature = "diagnostic-traces", track_caller)]
    pub(super) fn sync_nested_projected_container_children(&mut self) {
        // AddCluster/AddSequence replace a member slice with a synthetic
        // vessel in the temporary graph.  The Go vessel is still the same
        // pointer that was fitted in its inner scope, so its box already
        // lives in the enclosing container's frame when the outer
        // syncNested walk starts.  A Rust projection has no pointer alias to
        // carry that translation and can otherwise leave the vessel at the
        // inner origin (0,0).  Rebase only those logical-nil synthetic
        // vessels; ordinary child projections retain their established
        // absolute positions.
        self.rebase_projected_aggregate_vessels();
        if crate::internal::trace_env_enabled("TALA_TRACE_SYNC_SCOPE_DETAIL") {
            eprintln!(
                "SYNC_SCOPE_DETAIL_RUST before_nodes={:?} agg={:?} containers={:?}",
                self.nodes
                    .iter()
                    .map(|node| (
                        node.tala_id,
                        node.position,
                        node.scoring_is_aggregate_vessel
                    ))
                    .collect::<Vec<_>>(),
                self.transaction_external_aggregate_children
                    .iter()
                    .map(|(parent, children)| (
                        *parent,
                        children
                            .iter()
                            .map(|child| (child.tala_id, child.position))
                            .collect::<Vec<_>>()
                    ))
                    .collect::<Vec<_>>(),
                self.transaction_external_container_children
                    .iter()
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
        let nodes = self.graph_node_order();
        let trace_sync_scope = crate::internal::trace_env_enabled("TALA_TRACE_SYNC_SCOPE");
        if trace_sync_scope {
            eprintln!(
                "SYNC_SCOPE_RUST caller={} nodes={:?} current_nodes={:?} aggregate_keys={:?} cluster_keys={:?}",
                std::panic::Location::caller(),
                nodes
                    .iter()
                    .map(|node| {
                        let live = &self.nodes[node.0 as usize];
                        (
                            live.tala_id,
                            live.is_container,
                            live.scoring_is_aggregate_vessel,
                        )
                    })
                    .collect::<Vec<_>>(),
                self.graph_node_order()
                    .iter()
                    .map(|node| self.nodes[node.0 as usize].tala_id)
                    .collect::<Vec<_>>(),
                self.transaction_external_aggregate_children
                    .keys()
                    .collect::<Vec<_>>(),
                self.transaction_external_cluster_layouts
                    .keys()
                    .collect::<Vec<_>>()
            );
        }
        for node in nodes.iter().copied() {
            if self.nodes[node.0 as usize].is_container {
                self.sync_nested_projected_ordinary_container(node);
            }
            // Go's syncNested visits only Graph.Nodes. A synthetic vessel
            // found in this node's Containers slice has already moved with
            // the ordinary container subtree above; processing that hidden
            // vessel here would apply its member-child padding a second time.
            let (vessel_tala_id, is_aggregate_vessel, cluster_arrangement) = {
                let live_node = &self.nodes[node.0 as usize];
                (
                    live_node.tala_id,
                    live_node.scoring_is_aggregate_vessel,
                    live_node.scoring_cluster_arrangement,
                )
            };
            // Recovered Graph.syncNested gates this branch on the current
            // Graph.Nodes entry's `isClusterVessel` bit.  The projected
            // layout map can outlive that temporary vessel, so its presence
            // alone is not a lifecycle predicate.  Sequence vessels are also
            // aggregates, but have no cluster arrangement.
            if is_aggregate_vessel
                && cluster_arrangement.is_some()
                && self
                    .transaction_external_cluster_layouts
                    .contains_key(&vessel_tala_id)
            {
                self.sync_external_cluster(vessel_tala_id);
                self.sync_nested_projected_cluster_member_children(vessel_tala_id);
            }
            // Graph.syncNested performs Sequence.sync after the container and
            // cluster branches for this same Graph.Nodes entry. Projected
            // sequence vessels have aggregate members but no cluster layout.
            if is_aggregate_vessel
                && cluster_arrangement.is_none()
                && !self
                    .transaction_external_cluster_layouts
                    .contains_key(&vessel_tala_id)
                && self
                    .transaction_external_aggregate_children
                    .contains_key(&vessel_tala_id)
            {
                self.sync_external_sequence(vessel_tala_id);
            }
        }
    }

    /// Replay the source graph's `SyncNestedGeometry` cluster-member walk for
    /// diagnostic snapshots.  An induced placement graph owns only the
    /// current scope's dense nodes, while Go's `CopyEntitiesFrom` aliases the
    /// complete source `*Node` graph.  At an after-fit trace boundary the
    /// latter has already run `positionContainerChildren` on each container
    /// member of a visible cluster.  The release graph deliberately keeps
    /// that projection out of the runtime path; this helper is called only on
    /// the clone used to materialize a diagnostic snapshot.
    #[cfg(feature = "diagnostic-traces")]
    pub(super) fn sync_trace_source_cluster_children(&mut self, root_tala_id: i64) {
        if crate::internal::trace_env_enabled("TALA_TRACE_SOURCE_CLUSTER_SYNC") {
            eprintln!(
                "SOURCE_CLUSTER_CALL_RUST root={} trace_children={} clusters={}",
                root_tala_id,
                self.trace_container_children.len(),
                self.clusters.len()
            );
        }
        if self.trace_container_children.is_empty() {
            return;
        }

        // Refresh the immutable source hierarchy aliases from the latest
        // dense node boxes.  `trace_container_children` is captured when the
        // arena is built, whereas an induced scope updates its live nodes as
        // each nested placement returns.
        let latest = self
            .nodes
            .iter()
            .cloned()
            .map(|node| (node.tala_id, node))
            .collect::<BTreeMap<_, _>>();
        for children in self.trace_container_children.values_mut() {
            for child in children {
                if let Some(current) = latest.get(&child.tala_id) {
                    *child = current.clone();
                }
            }
        }

        let mut actions = Vec::<(Vec<i64>, Insets)>::new();
        let mut seen_members = BTreeSet::new();
        for cluster in &self.clusters {
            // In the upstream callback the extra cluster-member padding is
            // applied by the graph that owns the cluster vessel.  A nested
            // member's own after-fit callback has already positioned its
            // descendants and must not receive this translation a second
            // time.  Source parent identity is the stable equivalent of the
            // owning graph's `Containers[vessel.Container]` relation.
            let relevant = cluster.members.iter().any(|member| {
                let tala_id = self.nodes[member.0 as usize].tala_id;
                self.source_parent_tala_by_tala
                    .get(&tala_id)
                    .copied()
                    .flatten()
                    == Some(root_tala_id)
            });
            if !relevant {
                continue;
            }
            for member in &cluster.members {
                let member_tala_id = self.nodes[member.0 as usize].tala_id;
                if !seen_members.insert(member_tala_id) {
                    continue;
                }
                let Some(children) = self.trace_container_children.get(&member_tala_id) else {
                    continue;
                };
                let Some(member_node) = latest.get(&member_tala_id) else {
                    continue;
                };
                if !member_node.is_container || children.is_empty() {
                    continue;
                }
                // The cloned native sync pass above already translates child
                // IDs present in its current Graph.Nodes slice. This repair
                // is only for source pointers outside that dense slice;
                // translating a shared child twice applies the member inset
                // twice.
                let roots = children
                    .iter()
                    .filter(|child| !latest.contains_key(&child.tala_id))
                    .map(|child| child.tala_id)
                    .collect::<Vec<_>>();
                if roots.is_empty() {
                    continue;
                }
                let padding = Self::projected_container_padding(member_node, children, true);
                actions.push((roots, padding));
            }
        }
        if actions.is_empty() {
            if crate::internal::trace_env_enabled("TALA_TRACE_SOURCE_CLUSTER_SYNC") {
                eprintln!(
                    "SOURCE_CLUSTER_ACTIONS_RUST root={} actions=0",
                    root_tala_id
                );
            }
            return;
        }
        if crate::internal::trace_env_enabled("TALA_TRACE_SOURCE_CLUSTER_SYNC") {
            eprintln!(
                "SOURCE_CLUSTER_ACTIONS_RUST root={} actions={:?}",
                root_tala_id,
                actions
                    .iter()
                    .map(|(roots, p)| (roots.clone(), p.left, p.top))
                    .collect::<Vec<_>>()
            );
        }

        // Reuse the same subtree translation implementation as the normal
        // projection bridge.  It updates dense nodes and all aliases in the
        // temporary map, giving `TRACE_SYNCED_NODES` the shared-pointer state
        // that Go exposes to `tracePlacement`.
        self.transaction_external_container_children = self.trace_container_children.clone();
        for (roots, padding) in actions {
            self.translate_projected_subtrees(
                &roots,
                Point {
                    x: padding.left,
                    y: padding.top,
                },
            );
        }
    }

    /// Materialize the outer-frame position of aggregate vessels that were
    /// synthesized from an unpositioned member carrier.  This mirrors the
    /// shared pointer state produced by an inner `positionContainerChildren`
    /// call before the enclosing graph translates the parent subtree.
    fn rebase_projected_aggregate_vessels(&mut self) {
        let trace_rebase = crate::internal::trace_env_enabled("TALA_TRACE_AGGREGATE_REBASE");
        let mut positions = BTreeMap::<i64, Point>::new();
        let live_ids = self
            .node_order
            .iter()
            .map(|node| self.nodes[node.0 as usize].tala_id)
            .collect::<BTreeSet<_>>();
        for node in self
            .nodes
            .iter()
            .chain(self.transaction_external_containers.iter())
            .chain(
                self.transaction_external_container_children
                    .values()
                    .flatten(),
            )
        {
            if let Some(position) = node.position {
                // Current Graph.Nodes is authoritative for live entries;
                // hidden parents are represented by the retained child
                // aliases below and must be allowed to replace their stale
                // stable-arena frame.
                if live_ids.contains(&node.tala_id) {
                    positions.insert(node.tala_id, position);
                } else {
                    positions.entry(node.tala_id).or_insert(position);
                }
            }
        }
        for children in self.transaction_external_container_children.values() {
            for node in children {
                if !live_ids.contains(&node.tala_id)
                    && let Some(position) = node.position
                {
                    positions.insert(node.tala_id, position);
                }
            }
        }
        let mut plans = Vec::<(i64, i64, Point)>::new();
        for (&parent_tala_id, children) in &self.transaction_external_container_children {
            let Some(parent_position) = positions.get(&parent_tala_id).copied() else {
                continue;
            };
            let Some(parent_node) = self
                .nodes
                .iter()
                .find(|node| node.tala_id == parent_tala_id)
                .or_else(|| {
                    self.transaction_external_containers
                        .iter()
                        .find(|node| node.tala_id == parent_tala_id)
                })
                .or_else(|| {
                    self.transaction_external_container_children
                        .values()
                        .flatten()
                        .find(|node| node.tala_id == parent_tala_id)
                })
            else {
                continue;
            };
            let padding = Self::projected_shape_fit_padding(parent_node);
            let parent_is_live = self
                .node_order
                .iter()
                .any(|node| self.nodes[node.0 as usize].tala_id == parent_tala_id);
            for child in children {
                let is_aggregate_vessel = child.scoring_is_aggregate_vessel
                    || self
                        .transaction_external_aggregate_children
                        .contains_key(&child.tala_id);
                if trace_rebase && is_aggregate_vessel {
                    let cluster_members = self
                        .clusters
                        .iter()
                        .enumerate()
                        .find(|(_, cluster)| cluster.vessel_tala_id == child.tala_id)
                        .map(|(cluster_index, cluster)| {
                            cluster
                                .members
                                .iter()
                                .map(|member| {
                                    let tala_id = self.nodes[member.0 as usize].tala_id;
                                    (
                                        tala_id,
                                        self.projected_node_position_by_tala(tala_id)
                                            .or_else(|| self.position(*member)),
                                        self.cluster_member_geometry(cluster_index, *member),
                                    )
                                })
                                .collect::<Vec<_>>()
                        });
                    eprintln!(
                        "AGGREGATE_REBASE_INPUT_RUST parent={} parent_pos={:?} parent_live={} vessel={} vessel_pos={:?} position_was_nil={} anchor={:?} members={:?} cluster_members={:?} recovered_frame={:?}",
                        parent_tala_id,
                        parent_position,
                        parent_is_live,
                        child.tala_id,
                        child.position,
                        child.transaction_position_was_nil,
                        child.transaction_anchor_tala_id,
                        self.transaction_external_aggregate_children
                            .get(&child.tala_id)
                            .into_iter()
                            .flatten()
                            .map(|member| (
                                member.tala_id,
                                member.position,
                                member.transaction_position_was_nil,
                                member.transaction_anchor_tala_id,
                                member.transaction_anchor_offset,
                            ))
                            .collect::<Vec<_>>(),
                        cluster_members,
                        self.projected_cluster_vessel_frame(child.tala_id)
                    );
                }
                // A live parent is positioned by the normal Graph.Nodes walk.
                // Only vessels below hidden containers need their enclosing
                // parent frame reconstructed here.
                if !is_aggregate_vessel || parent_is_live {
                    continue;
                }
                if !child.transaction_position_was_nil {
                    // A non-nil pointer is usually already absolute, including
                    // when an oversized aggregate extends beyond its old
                    // parent bounds. A nested cluster can also retain a local
                    // origin; accept that interpretation only when its first
                    // member independently confirms the parent-relative frame.
                    let Some(member_frame_position) =
                        self.projected_cluster_vessel_frame(child.tala_id)
                    else {
                        continue;
                    };
                    let local_offset = child.position.unwrap_or_default();
                    let parent_frame_position = Point {
                        x: parent_position.x + padding.left + local_offset.x,
                        y: parent_position.y + padding.top + local_offset.y,
                    };
                    if member_frame_position != parent_frame_position {
                        continue;
                    }
                    if trace_rebase {
                        eprintln!(
                            "AGGREGATE_REBASE_MEMBER_FRAME_RUST parent={} vessel={} old={:?} member_frame={:?}",
                            parent_tala_id, child.tala_id, child.position, member_frame_position
                        );
                    }
                    plans.push((parent_tala_id, child.tala_id, member_frame_position));
                    continue;
                }
                // Reconstructing the frame is a one-time bridge for a vessel
                // that still carries its inner-scope coordinates. Once the
                // vessel is already inside this parent's padded interior,
                // Graph.syncNested's shared-pointer equivalent has placed it.
                // Treating its absolute position as a new local offset on a
                // later sync adds the parent's padding repeatedly (and makes
                // the enclosing container grow on every pass).
                let already_in_parent_frame = child.position.is_some_and(|position| {
                    position.x >= parent_position.x + padding.left
                        && position.y >= parent_position.y + padding.top
                        && position.x + child.rect.size.width
                            <= parent_position.x + parent_node.rect.size.width - padding.right
                        && position.y + child.rect.size.height
                            <= parent_position.y + parent_node.rect.size.height - padding.bottom
                });
                if already_in_parent_frame {
                    continue;
                }
                let child_offset = child.position.unwrap_or_default();
                let target = Point {
                    x: parent_position.x + padding.left + child_offset.x,
                    y: parent_position.y + padding.top + child_offset.y,
                };
                if trace_rebase {
                    eprintln!(
                        "AGGREGATE_REBASE_TARGET_RUST parent={} vessel={} child_offset={:?} padding=({}, {}) target={:?}",
                        parent_tala_id,
                        child.tala_id,
                        child_offset,
                        padding.left,
                        padding.top,
                        target
                    );
                }
                plans.push((parent_tala_id, child.tala_id, target));
            }
        }
        for (parent_tala_id, vessel_tala_id, target) in plans {
            if let Some(child) = self
                .transaction_external_container_children
                .get_mut(&parent_tala_id)
                .and_then(|children| {
                    children
                        .iter_mut()
                        .find(|child| child.tala_id == vessel_tala_id)
                })
            {
                child.position = Some(target);
                child.rect.origin = target;
                child.transaction_position_was_nil = false;
                child.transaction_anchor_tala_id = None;
                child.transaction_anchor_offset = None;
            }

            // The stable arena reads a cluster's active vessel origin from
            // this side table. Keep that logical pointer in the same frame as
            // the external vessel projection; otherwise the later
            // OptimizeClusters pass compares external endpoints against the
            // pre-rebase origin even though Go's shared vessel pointer moved.
            for (cluster_index, cluster) in self.clusters.iter().enumerate() {
                if cluster.vessel_tala_id == vessel_tala_id {
                    self.pending_cluster_vessel_positions
                        .insert(cluster_index, target);
                }
            }

            // The vessel's retained member pointers move with it. Preserve
            // each member's inner-scope offset while rebasing the carrier.
            if let Some(members) = self
                .transaction_external_aggregate_children
                .get_mut(&vessel_tala_id)
            {
                let base = members
                    .iter()
                    .find_map(|member| member.position)
                    .unwrap_or_default();
                for member in members {
                    if let Some(position) = member.position {
                        let position = Point {
                            x: target.x + position.x - base.x,
                            y: target.y + position.y - base.y,
                        };
                        member.position = Some(position);
                        member.rect.origin = position;
                    }
                    member.transaction_position_was_nil = false;
                    member.transaction_anchor_tala_id = None;
                    member.transaction_anchor_offset = None;
                }
            }
        }
    }

    /// Recover a cluster vessel's absolute origin from its shared members.
    /// ArrangeClusterNodes places the first ordered member at a deterministic
    /// offset from the vessel, making that pointer sufficient even when later
    /// members have not yet been synchronized in this induced graph.
    fn projected_cluster_vessel_frame(&self, vessel_tala_id: i64) -> Option<Point> {
        let layout = self
            .transaction_external_cluster_layouts
            .get(&vessel_tala_id)?;
        let members = self
            .transaction_external_aggregate_children
            .get(&vessel_tala_id)?;
        let first = members.first()?;
        let maximum = members
            .iter()
            .fold(Size::default(), |maximum, member| Size {
                width: maximum.width.max(member.rect.size.width),
                height: maximum.height.max(member.rect.size.height),
            });
        let first_size = if layout.fixed_size {
            first.rect.size
        } else {
            maximum
        };
        let count = members.len() as f64;
        let gaps = layout.padding * (count - 1.0).max(0.0);
        let vessel_size = match layout.arrangement {
            ClusterArrangement::Row => Size {
                width: maximum.width * count + gaps,
                height: maximum.height,
            },
            ClusterArrangement::Column => Size {
                width: maximum.width,
                height: maximum.height * count + gaps,
            },
        };
        let first_offset = match layout.arrangement {
            ClusterArrangement::Row => Point {
                x: 0.0,
                y: (vessel_size.height / 2.0 - first_size.height / 2.0).round(),
            },
            ClusterArrangement::Column => Point {
                x: (vessel_size.width / 2.0 - first_size.width / 2.0).round(),
                y: 0.0,
            },
        };
        let first_position = self
            .projected_node_position_by_tala(first.tala_id)
            .or(first.position)?;
        Some(Point {
            x: first_position.x - first_offset.x,
            y: first_position.y - first_offset.y,
        })
    }

    /// Padding equivalent for a container alias that is not present in the
    /// dense temporary `nodes` arena. Hidden parents still carry the shape,
    /// label, icon, and explicit padding fields needed by getContainerPadding.
    fn projected_shape_fit_padding(node: &ArenaNode) -> Insets {
        let mut spacing = Insets::uniform(60.0);
        let explicit = node.node_padding;
        let padding_floor = if node.shape == ShapeKind::Circle {
            15.0
        } else if node.has_icon && node.shape != ShapeKind::Image {
            74.0
        } else {
            60.0
        };
        let label_is_outside = Self::label_position_is_outside(node.label_position);
        if let Some(label) = node.label_size.filter(|_| !label_is_outside) {
            let label_width = label.width + 10.0;
            let label_height = label.height + 10.0;
            match node.label_position {
                LabelPosition::InsideTopLeft
                | LabelPosition::InsideTopCenter
                | LabelPosition::InsideTopRight => spacing.top = spacing.top.max(label_height),
                LabelPosition::InsideBottomLeft
                | LabelPosition::InsideBottomCenter
                | LabelPosition::InsideBottomRight => {
                    spacing.bottom = spacing.bottom.max(label_height)
                }
                LabelPosition::InsideMiddleLeft => spacing.left = spacing.left.max(label_width),
                LabelPosition::InsideMiddleRight => spacing.right = spacing.right.max(label_width),
                _ => {}
            }
        }
        if node.shape == ShapeKind::Circle {
            spacing.top *= 0.25;
            spacing.right *= 0.25;
            spacing.bottom *= 0.25;
            spacing.left *= 0.25;
        }
        Insets {
            top: spacing.top.max(explicit.top).max(padding_floor),
            right: spacing.right.max(explicit.right).max(padding_floor),
            bottom: spacing.bottom.max(explicit.bottom).max(padding_floor),
            left: spacing.left.max(explicit.left).max(padding_floor),
        }
    }

    fn sync_nested_projected_ordinary_container(&mut self, container: NodeId) {
        let container_tala_id = self.nodes[container.0 as usize].tala_id;
        let Some(children) = self
            .transaction_external_container_children
            .get(&container_tala_id)
            .cloned()
        else {
            return;
        };
        let Some((mut top_left, mut bottom_right)) = Self::fixed_external_node_bounds(&children)
        else {
            return;
        };
        // Node.expandForLabels expands only a boundary child whose label
        // is wider than its shape box.
        for child in &children {
            let Some(label_size) = child.label_size else {
                continue;
            };
            let Some(position) = child.position else {
                continue;
            };
            if position.x != top_left.x && position.x + child.rect.size.width != bottom_right.x {
                continue;
            }
            if label_size.width > child.rect.size.width {
                let overhang = (label_size.width - child.rect.size.width) * 0.5;
                top_left.x = top_left.x.min((position.x - overhang).floor());
                bottom_right.x = bottom_right
                    .x
                    .max((position.x + child.rect.size.width + overhang).ceil());
            }
        }
        let content = Size {
            width: bottom_right.x - top_left.x,
            height: bottom_right.y - top_left.y,
        };
        let padding = self.shape_fit_padding(container);
        let Some(inside) = self.shape_inside_placement_absolute(container, content, padding) else {
            return;
        };
        let delta = Point {
            x: inside.x - top_left.x,
            y: inside.y - top_left.y,
        };
        let trace_position_children =
            crate::internal::trace_env_value("TALA_TRACE_POSITION_CHILDREN").is_some_and(
                |target| target == "all" || target.parse::<i64>().ok() == Some(container_tala_id),
            );
        if trace_position_children {
            eprintln!(
                "POSITION_CHILDREN_RUST node={} shape={:?} position={:?} tl={},{} br={},{} padding={},{},{},{} inside={},{} delta={},{} children={:?}",
                container_tala_id,
                self.nodes[container.0 as usize].shape,
                self.nodes[container.0 as usize].position,
                top_left.x,
                top_left.y,
                bottom_right.x,
                bottom_right.y,
                padding.top,
                padding.right,
                padding.bottom,
                padding.left,
                inside.x,
                inside.y,
                delta.x,
                delta.y,
                children
                    .iter()
                    .map(|child| (child.tala_id, child.position, child.fixed_top_left))
                    .collect::<Vec<_>>()
            );
            eprintln!(
                "POSITION_CHILDREN_FIXED_ORIGIN_RUST node={} fixed_origin={:?}",
                container_tala_id,
                self.container_fixed_origin(Some(container))
            );
        }
        let child_tala_ids = children
            .iter()
            .map(|child| child.tala_id)
            .collect::<Vec<_>>();
        // positionContainerChildren calls moveNodeWithChildren for each direct
        // child. That walk follows ordinary, cluster, and sequence descendants
        // and mutates the same *Node through every Graph alias.
        self.translate_projected_subtrees(&child_tala_ids, delta);
        if trace_position_children {
            eprintln!(
                "POSITION_CHILDREN_RUST_AFTER node={} maps={:?}",
                container_tala_id,
                self.transaction_external_container_children
                    .iter()
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
    }

    /// The `n.isClusterVessel` branch of recovered `Graph.syncNested`.
    /// `sync_external_cluster` above has already arranged the retained member
    /// pointers. This pass deliberately moves each direct child, not the
    /// member or vessel itself.
    pub(super) fn sync_nested_projected_cluster_member_children(&mut self, vessel_tala_id: i64) {
        let members = self
            .transaction_external_aggregate_children
            .get(&vessel_tala_id)
            .cloned()
            .unwrap_or_default();
        for member in members.into_iter().filter(|member| member.is_container) {
            let children = self
                .transaction_external_container_children
                .get(&member.tala_id)
                .cloned()
                .unwrap_or_default();
            if children.is_empty() {
                continue;
            }
            let padding = Self::projected_container_padding(&member, &children, true);
            let child_tala_ids = children
                .iter()
                .map(|child| child.tala_id)
                .collect::<Vec<_>>();
            self.translate_projected_subtrees(
                &child_tala_ids,
                Point {
                    x: padding.left,
                    y: padding.top,
                },
            );
        }
    }

    /// Flattened equivalent of calling `moveNodeWithChildren` on each root.
    /// The closure follows both ordinary container and aggregate-vessel maps,
    /// matching `getAllDescendantNodes(root, true)`.
    #[cfg_attr(feature = "diagnostic-traces", track_caller)]
    pub(super) fn translate_projected_subtrees(&mut self, roots: &[i64], delta: Point) {
        if delta == Point::default() {
            return;
        }
        let trace_projection_translation = crate::internal::trace_env_value(
            "TALA_TRACE_PROJECTED_TRANSLATION",
        )
        .is_some_and(|target| {
            target == "all"
                || roots.iter().any(|root| root.to_string() == target)
                || self
                    .nodes
                    .iter()
                    .any(|node| node.tala_id.to_string() == target)
        });
        if trace_projection_translation {
            eprintln!(
                "PROJECTED_TRANSLATION_RUST caller={} roots={:?} delta={},{} target_positions={:?}",
                std::panic::Location::caller(),
                roots,
                delta.x,
                delta.y,
                self.nodes
                    .iter()
                    .filter(|node| {
                        crate::internal::trace_env_value("TALA_TRACE_PROJECTED_TRANSLATION")
                            .is_some_and(|target| {
                                target == "all" || node.tala_id.to_string() == target
                            })
                    })
                    .map(|node| (node.tala_id, node.position))
                    .collect::<Vec<_>>()
            );
        }
        let mut moved_tala_ids = roots.iter().copied().collect::<BTreeSet<_>>();
        loop {
            let mut changed = false;
            for children_by_parent in [
                &self.transaction_external_container_children,
                &self.transaction_external_aggregate_children,
            ] {
                for (&parent_tala_id, children) in children_by_parent {
                    if !moved_tala_ids.contains(&parent_tala_id) {
                        continue;
                    }
                    for child in children {
                        changed |= moved_tala_ids.insert(child.tala_id);
                    }
                }
            }
            if !changed {
                break;
            }
        }

        let trace_object2 = crate::internal::trace_env_enabled("TALA_TRACE_OBJECT2")
            && moved_tala_ids.iter().any(|id| {
                matches!(
                    id,
                    6334824724549167320 | 185443821 | 3356053350 | 4053080803 | 1033547076
                )
            });
        if trace_object2 {
            eprintln!(
                "OBJECT2_TRANSLATE_RUST caller={} roots={:?} delta={},{} moved={:?}",
                std::panic::Location::caller(),
                roots,
                delta.x,
                delta.y,
                moved_tala_ids
            );
        }
        let trace_target_translation =
            crate::internal::trace_env_value("TALA_TRACE_PROJECTED_TRANSLATION_TARGET");
        let trace_target_hit = trace_target_translation.as_deref().is_some_and(|target| {
            target == "all" || moved_tala_ids.iter().any(|id| id.to_string() == target)
        });
        if trace_target_hit {
            let positions = |graph: &ArenaGraph| {
                graph
                    .nodes
                    .iter()
                    .filter(|node| moved_tala_ids.contains(&node.tala_id))
                    .map(|node| (node.tala_id, node.position))
                    .chain(
                        graph
                            .transaction_external_container_children
                            .values()
                            .flatten()
                            .filter(|node| moved_tala_ids.contains(&node.tala_id))
                            .map(|node| (node.tala_id, node.position)),
                    )
                    .collect::<Vec<_>>()
            };
            eprintln!(
                "PROJECTED_TRANSLATION_TARGET_RUST caller={} roots={:?} delta={},{} moved={:?} before={:?}",
                std::panic::Location::caller(),
                roots,
                delta.x,
                delta.y,
                moved_tala_ids,
                positions(self)
            );
        }

        let translate_projected = |node: &mut ArenaNode| {
            if !moved_tala_ids.contains(&node.tala_id) {
                return;
            }
            if let Some(position) = node.position.as_mut() {
                position.x += delta.x;
                position.y += delta.y;
                node.rect.origin = *position;
            }
            // Anchor offsets are relative. Preserve them when the anchor is
            // translated in the same subtree; otherwise publish this move so
            // a later transaction initialization cannot restore stale
            // pre-syncNested geometry.
            if !node
                .transaction_anchor_tala_id
                .is_some_and(|anchor| moved_tala_ids.contains(&anchor))
                && let Some(offset) = node.transaction_anchor_offset.as_mut()
            {
                offset.x += delta.x;
                offset.y += delta.y;
            }
        };
        for children in self.transaction_external_container_children.values_mut() {
            for child in children {
                translate_projected(child);
            }
        }
        for children in self.transaction_external_aggregate_children.values_mut() {
            for child in children {
                translate_projected(child);
            }
        }
        for node in &mut self.transaction_external_containers {
            translate_projected(node);
        }

        for node in &mut self.nodes {
            if moved_tala_ids.contains(&node.tala_id)
                && let Some(position) = node.position.as_mut()
            {
                position.x += delta.x;
                position.y += delta.y;
                node.rect.origin = *position;
            }
        }
        if trace_projection_translation {
            eprintln!(
                "PROJECTED_TRANSLATION_RUST_AFTER target_positions={:?}",
                self.nodes
                    .iter()
                    .filter(|node| {
                        crate::internal::trace_env_value("TALA_TRACE_PROJECTED_TRANSLATION")
                            .is_some_and(|target| {
                                target == "all" || node.tala_id.to_string() == target
                            })
                    })
                    .map(|node| (node.tala_id, node.position))
                    .collect::<Vec<_>>()
            );
        }
        if trace_target_hit {
            let positions = self
                .nodes
                .iter()
                .filter(|node| moved_tala_ids.contains(&node.tala_id))
                .map(|node| (node.tala_id, node.position))
                .chain(
                    self.transaction_external_container_children
                        .values()
                        .flatten()
                        .filter(|node| moved_tala_ids.contains(&node.tala_id))
                        .map(|node| (node.tala_id, node.position)),
                )
                .collect::<Vec<_>>();
            eprintln!(
                "PROJECTED_TRANSLATION_TARGET_RUST_AFTER positions={:?}",
                positions
            );
        }
        for (cluster_index, cluster) in self.clusters.iter().enumerate() {
            if moved_tala_ids.contains(&cluster.vessel_tala_id)
                && let Some(position) = self
                    .pending_cluster_vessel_positions
                    .get_mut(&cluster_index)
            {
                position.x += delta.x;
                position.y += delta.y;
            }
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
            eprintln!("OBJECT2_TRANSLATE_RUST_AFTER {:?}", positions);
        }
    }

    fn projected_node_ref_by_tala(&self, tala_id: i64) -> Option<&ArenaNode> {
        // Graph.Nodes owns the authoritative pointer when the node is current.
        // Stable arena entries outside node_order are not current Graph.Nodes;
        // find their retained pointer through Containers/Clusters/Sequences.
        self.node_order
            .iter()
            .map(|node| &self.nodes[node.0 as usize])
            .find(|node| node.tala_id == tala_id)
            .or_else(|| {
                self.transaction_external_container_children
                    .values()
                    .flatten()
                    .find(|node| node.tala_id == tala_id)
            })
            .or_else(|| {
                self.transaction_external_aggregate_children
                    .values()
                    .flatten()
                    .find(|node| node.tala_id == tala_id)
            })
            .or_else(|| {
                self.transaction_external_containers
                    .iter()
                    .find(|node| node.tala_id == tala_id)
            })
    }

    pub(super) fn projected_node_by_tala(&self, tala_id: i64) -> Option<ArenaNode> {
        self.projected_node_ref_by_tala(tala_id).cloned()
    }

    pub(super) fn projected_node_position_by_tala(&self, tala_id: i64) -> Option<Point> {
        self.projected_node_ref_by_tala(tala_id)
            .and_then(|node| node.position)
            .or_else(|| {
                self.serialized_vessel_positions_by_tala
                    .get(&tala_id)
                    .copied()
            })
    }

    pub(super) fn projected_node_is_cluster_vessel(&self, tala_id: i64) -> bool {
        // At commit, Graph.Clusters/Sequences map membership is authoritative.
        // External pointer projections can carry a synthetic vessel ID/Box
        // without the scoring-only flags installed on current Graph.Nodes.
        self.transaction_external_cluster_layouts
            .contains_key(&tala_id)
            && self
                .transaction_external_aggregate_children
                .contains_key(&tala_id)
    }

    pub(super) fn projected_node_is_sequence_vessel(&self, tala_id: i64) -> bool {
        self.transaction_external_aggregate_children
            .contains_key(&tala_id)
            && !self.projected_node_is_cluster_vessel(tala_id)
    }

    /// Projected equivalent of recovered `Node.rdfsWalk`.
    ///
    /// The walk deliberately has no visited set. Go revisits a shared pointer
    /// whenever it occurs through another current root/path, and invokes the
    /// aggregate sync callback after each descendant traversal.
    pub(super) fn projected_reverse_dfs_order(&self) -> Vec<i64> {
        let containers = self
            .nodes
            .iter()
            .chain(self.transaction_external_containers.iter())
            .chain(
                self.transaction_external_container_children
                    .values()
                    .flatten(),
            )
            .chain(
                self.transaction_external_aggregate_children
                    .values()
                    .flatten(),
            )
            .filter_map(|node| node.is_container.then_some(node.tala_id))
            .collect::<IdHashSet<_>>();

        // Children are read from the projections in place; nothing moves
        // during the walk. A cluster or sequence vessel is exactly a node with
        // aggregate children (see projected_node_is_{cluster,sequence}_vessel).
        fn walk(graph: &ArenaGraph, node: i64, containers: &IdHashSet<i64>, order: &mut Vec<i64>) {
            if containers.contains(&node) {
                for child in graph
                    .transaction_external_container_children
                    .get(&node)
                    .into_iter()
                    .flatten()
                {
                    walk(graph, child.tala_id, containers, order);
                }
            }
            for member in graph
                .transaction_external_aggregate_children
                .get(&node)
                .into_iter()
                .flatten()
            {
                walk(graph, member.tala_id, containers, order);
            }
            order.push(node);
        }

        let mut order = Vec::new();
        for node in &self.node_order {
            walk(
                self,
                self.nodes[node.0 as usize].tala_id,
                &containers,
                &mut order,
            );
        }
        order
    }

    #[track_caller]
    pub(super) fn set_projected_node_position(&mut self, tala_id: i64, target: Point) {
        let trace_target_set = crate::internal::trace_env_value("TALA_TRACE_TARGET_SET")
            .is_some_and(|target_id| {
                target_id == "all"
                    || target_id
                        .split(',')
                        .any(|id| id.trim().parse::<i64>().ok() == Some(tala_id))
            });
        if trace_target_set {
            eprintln!(
                "TARGET_SET_RUST caller={} tala={} target={},{} before_active={:?} before_aliases={:?}",
                std::panic::Location::caller(),
                tala_id,
                target.x,
                target.y,
                self.nodes
                    .iter()
                    .find(|node| node.tala_id == tala_id)
                    .and_then(|node| node.position),
                self.transaction_external_container_children
                    .values()
                    .flatten()
                    .filter(|node| node.tala_id == tala_id)
                    .map(|node| node.position)
                    .collect::<Vec<_>>()
            );
        }
        if crate::internal::trace_env_enabled("TALA_TRACE_OBJECT2")
            && matches!(
                tala_id,
                6334824724549167320 | 185443821 | 3356053350 | 4053080803 | 1033547076
            )
        {
            eprintln!(
                "OBJECT2_SET_RUST caller={} tala={} target={},{}",
                std::panic::Location::caller(),
                tala_id,
                target.x,
                target.y
            );
        }
        // Resolve only the anchors the updated copies name, against the
        // positions before any update; the last positioned copy wins.
        let mut anchor_positions: Vec<(i64, Option<Point>)> = Vec::new();
        for node in self
            .nodes
            .iter()
            .chain(
                self.transaction_external_container_children
                    .values()
                    .flatten(),
            )
            .chain(
                self.transaction_external_aggregate_children
                    .values()
                    .flatten(),
            )
            .chain(&self.transaction_external_containers)
        {
            if node.tala_id == tala_id
                && let Some(anchor) = node.transaction_anchor_tala_id
                && !anchor_positions.iter().any(|&(id, _)| id == anchor)
            {
                anchor_positions.push((anchor, None));
            }
        }
        if !anchor_positions.is_empty() {
            for node in self.nodes.iter().chain(
                self.transaction_external_container_children
                    .values()
                    .flatten(),
            ) {
                if let Some(position) = node.position
                    && let Some(entry) = anchor_positions
                        .iter_mut()
                        .find(|(id, _)| *id == node.tala_id)
                {
                    entry.1 = Some(position);
                }
            }
        }
        let set = |node: &mut ArenaNode| {
            if node.tala_id != tala_id {
                return;
            }
            node.position = Some(target);
            node.rect.origin = target;
            if let Some(anchor) = node.transaction_anchor_tala_id
                && let Some(anchor_position) = anchor_positions
                    .iter()
                    .find(|(id, _)| *id == anchor)
                    .and_then(|(_, position)| position.as_ref())
            {
                node.transaction_anchor_offset = Some(Point {
                    x: target.x - anchor_position.x,
                    y: target.y - anchor_position.y,
                });
            }
        };
        for node in &mut self.nodes {
            set(node);
        }
        for children in self.transaction_external_container_children.values_mut() {
            for node in children {
                set(node);
            }
        }
        for children in self.transaction_external_aggregate_children.values_mut() {
            for node in children {
                set(node);
            }
        }
        for node in &mut self.transaction_external_containers {
            set(node);
        }
    }

    pub(super) fn consume_projected_logical_nil_position(&mut self, tala_id: i64) {
        let consume = |node: &mut ArenaNode| {
            if node.tala_id == tala_id {
                node.transaction_position_was_nil = false;
            }
        };
        for node in &mut self.nodes {
            consume(node);
        }
        for children in self.transaction_external_container_children.values_mut() {
            for node in children {
                consume(node);
            }
        }
        for children in self.transaction_external_aggregate_children.values_mut() {
            for node in children {
                consume(node);
            }
        }
        for node in &mut self.transaction_external_containers {
            consume(node);
        }
    }

    pub(super) fn set_projected_node_size(&mut self, tala_id: i64, size: Size) {
        self.set_projected_node_sizes(&[(tala_id, size)]);
    }

    /// `set_projected_node_size` for each update in order, in one pass over
    /// the node lists and projection caches. Each value receives the size of
    /// the last update naming its TALA ID, as sequential calls would leave it.
    pub(super) fn set_projected_node_sizes(&mut self, updates: &[(i64, Size)]) {
        let set = |node: &mut ArenaNode| {
            if let Some(size) = last_size_update(updates, node.tala_id) {
                node.rect.size = size;
            }
        };
        for node in &mut self.nodes {
            set(node);
        }
        for children in self.transaction_external_container_children.values_mut() {
            for node in children {
                set(node);
            }
        }
        for children in self.transaction_external_aggregate_children.values_mut() {
            for node in children {
                set(node);
            }
        }
        for node in &mut self.transaction_external_containers {
            set(node);
        }

        self.set_projected_cache_sizes(updates);
    }

    fn set_projected_cache_size(&mut self, tala_id: i64, size: Size) {
        self.set_projected_cache_sizes(&[(tala_id, size)]);
    }

    fn set_projected_cache_sizes(&mut self, updates: &[(i64, Size)]) {
        // The sized optimizer flattens shared original *Node pointers into
        // several immutable-looking projection caches. A recovered Resize
        // mutates the pointed-to Box, so every cached view of that TALA node
        // must observe the new dimensions at the same lifecycle boundary.
        // A pass that already ran with bit-equal updates over caches of
        // unchanged generation would assign the same sizes again.
        let generations = self.projection_generations();
        let update_bits = updates
            .iter()
            .map(|&(tala_id, size)| (tala_id, size_bits(size)))
            .collect::<Vec<_>>();
        if self
            .projection_memo
            .sized
            .iter()
            .any(|(recorded_generations, recorded_updates)| {
                *recorded_generations == generations && *recorded_updates == update_bits
            })
        {
            return;
        }
        let changed = std::cell::Cell::new(false);
        let set_projected = |projected: &mut ProjectedAdjacent| {
            if let Some(size) = last_size_update(updates, projected.tala_id) {
                if size_bits(size) != size_bits(projected.size) {
                    changed.set(true);
                }
                projected.size = size;
            }
        };
        for projected in self.sized_adjacent_overrides.values_mut() {
            set_projected(projected);
        }
        for abduction in &mut self.sized_edge_abductions {
            for original in [&mut abduction.originally_from, &mut abduction.originally_to]
                .into_iter()
                .flatten()
            {
                if let Some(size) = last_size_update(updates, original.tala_id) {
                    if size_bits(size) != size_bits(original.size) {
                        changed.set(true);
                    }
                    original.size = size;
                }
            }
            for neighbor in &mut abduction.originally_from_table_neighbors {
                set_projected(&mut neighbor.other);
            }
            for neighbor in &mut abduction.originally_to_table_neighbors {
                set_projected(&mut neighbor.other);
            }
            for projected in &mut abduction.obstructions_from_to {
                set_projected(projected);
            }
            for projected in &mut abduction.obstructions_to_from {
                set_projected(projected);
            }
            abduction
                .obstruction_sets_from_to
                .update_projections(set_projected);
            abduction
                .obstruction_sets_to_from
                .update_projections(set_projected);
        }
        for projections in self.sized_projected_obstructions.values_mut() {
            projections.update_projections(set_projected);
        }
        for inventory in self.sized_projected_obstruction_sets.values_mut() {
            inventory.update_projections(set_projected);
        }
        for projected in self
            .sized_collapsed_symmetry_neighbors
            .values_mut()
            .flatten()
        {
            set_projected(projected);
        }
        for geometry in self.sized_cluster_distance_boxes.values_mut() {
            if let Some(size) = last_size_update(updates, geometry.vessel_tala_id) {
                if size_bits(size) != size_bits(geometry.size) {
                    changed.set(true);
                }
                geometry.size = size;
            }
            for projected in &mut geometry.external_connected {
                set_projected(projected);
            }
        }
        if !changed.get() {
            self.restore_unchanged_projection_generations(generations);
        }
        let generations = self.projection_generations();
        let memo = &mut self.projection_memo.sized;
        if memo.len() == PROJECTION_MEMO_ENTRIES {
            memo.remove(0);
        }
        memo.push((generations, update_bits));
    }

    /// Projected `Node.positionContainerChildren`; used by the nil-member arm
    /// of `Cluster.ArrangeClusterNodes` after assigning the member top-left.
    pub(super) fn position_projected_container_children(
        &mut self,
        container_tala_id: i64,
        with_padding: bool,
    ) -> bool {
        let trace_position_children = crate::internal::trace_env_value(
            "TALA_TRACE_POSITION_CHILDREN",
        )
        .is_some_and(|target| {
            target == "all"
                || target
                    .split(',')
                    .any(|id| id.trim().parse::<i64>().ok() == Some(container_tala_id))
        });
        let Some(container) = self.projected_node_by_tala(container_tala_id) else {
            return false;
        };
        if !container.is_container {
            return false;
        }
        let Some(children) = self
            .transaction_external_container_children
            .get(&container_tala_id)
            .cloned()
        else {
            return false;
        };
        let Some((mut top_left, mut bottom_right)) = Self::fixed_external_node_bounds(&children)
        else {
            return false;
        };
        for child in &children {
            let Some(label_size) = child.label_size else {
                continue;
            };
            let Some(position) = child.position else {
                continue;
            };
            if position.x != top_left.x && position.x + child.rect.size.width != bottom_right.x {
                continue;
            }
            if label_size.width > child.rect.size.width {
                let overhang = (label_size.width - child.rect.size.width) * 0.5;
                top_left.x = top_left.x.min((position.x - overhang).floor());
                bottom_right.x = bottom_right
                    .x
                    .max((position.x + child.rect.size.width + overhang).ceil());
            }
        }
        let content = Size {
            width: bottom_right.x - top_left.x,
            height: bottom_right.y - top_left.y,
        };
        let padding = if with_padding {
            Self::projected_container_padding(&container, &children, false)
        } else {
            Insets::uniform(0.0)
        };
        let Some(origin) = container.position else {
            return false;
        };
        let inside = Self::shape_inside_placement_at_node(&container, content, padding, origin);
        let child_tala_ids = children
            .iter()
            .map(|child| child.tala_id)
            .collect::<Vec<_>>();
        let delta = Point {
            x: inside.x - top_left.x,
            y: inside.y - top_left.y,
        };
        if trace_position_children {
            eprintln!(
                "POSITION_PROJECTED_CHILDREN_RUST node={} container={:?} bounds=({},{})->({},{}), content={:?} padding={:?} inside={:?} delta={:?} children={:?}",
                container_tala_id,
                container.position,
                top_left.x,
                top_left.y,
                bottom_right.x,
                bottom_right.y,
                content,
                padding,
                inside,
                delta,
                children
                    .iter()
                    .map(|child| (child.tala_id, child.position, child.rect.size))
                    .collect::<Vec<_>>()
            );
        }
        if crate::internal::trace_env_enabled("TALA_TRACE_MIRROR") && container_tala_id == 457364737
        {
            eprintln!(
                "MIRROR_PROJECTED_FIT_RUST parent={} container={:?} bounds=({},{})->({},{}), content={:?} padding={:?} inside={:?} delta={:?} children={:?}",
                container_tala_id,
                container.position,
                top_left.x,
                top_left.y,
                bottom_right.x,
                bottom_right.y,
                content,
                padding,
                inside,
                delta,
                children
                    .iter()
                    .map(|child| (child.tala_id, child.position, child.rect.size))
                    .collect::<Vec<_>>()
            );
        }
        self.translate_projected_subtrees(&child_tala_ids, delta);
        if trace_position_children {
            let after = self
                .transaction_external_container_children
                .get(&container_tala_id)
                .into_iter()
                .flatten()
                .map(|child| (child.tala_id, child.position))
                .collect::<Vec<_>>();
            eprintln!(
                "POSITION_PROJECTED_CHILDREN_RUST_AFTER node={} children={:?}",
                container_tala_id, after
            );
        }
        delta != Point::default()
    }

    pub(super) fn anchored_external_position(&self, node: &ArenaNode) -> Option<Point> {
        node.transaction_anchor_tala_id
            .zip(node.transaction_anchor_offset)
            .and_then(|(anchor_tala_id, offset)| {
                self.nodes
                    .iter()
                    .find(|candidate| candidate.tala_id == anchor_tala_id)
                    .and_then(|anchor| anchor.position)
                    .map(|anchor| Point {
                        x: anchor.x + offset.x,
                        y: anchor.y + offset.y,
                    })
            })
            .or(node.position)
    }

    fn external_container_spacing_delta(&self, container: &ArenaNode, node_ref: &ArenaNode) -> f64 {
        let container_position = container.position.expect("positioned external container");
        let node_position = node_ref.position.expect("positioned transaction node");
        let orientation = self.sized_box_orientation(
            (container_position, container.rect.size),
            (node_position, node_ref.rect.size),
        );
        // Node.deltaTo considers the edge inventory and loop clearance even
        // when one endpoint is an external container projection. The old
        // external path only used shape margins, so a large self-loop on an
        // external container could be ignored and Rust would accept a
        // transpose that Go rejects.
        let edge_metrics = |first: &ArenaNode, second: &ArenaNode| {
            let mut connected = false;
            let mut max_edge_width = f64::NEG_INFINITY;
            let mut max_edge_height = f64::NEG_INFINITY;
            for edge_id in &first.edges.read() {
                let Some(edge) = self.edge_records.get(edge_id.0 as usize) else {
                    continue;
                };
                let adjacent = if edge.from == first.input_id {
                    edge.to
                } else if edge.to == first.input_id {
                    edge.from
                } else {
                    continue;
                };
                if adjacent != second.input_id {
                    continue;
                }
                connected = true;
                max_edge_width = max_edge_width.max(edge.min_width);
                max_edge_height = max_edge_height.max(edge.min_height);
            }
            (connected, max_edge_width, max_edge_height)
        };
        let (connected, max_edge_width, max_edge_height) = edge_metrics(container, node_ref);
        let mut horizontal: f64 = if connected { 60.0 } else { 20.0 };
        let mut vertical: f64 = if connected { 60.0 } else { 20.0 };
        if container.shape == ShapeKind::SqlTable || node_ref.shape == ShapeKind::SqlTable {
            horizontal = 120.0;
        }
        horizontal = horizontal.max(max_edge_width);
        vertical = vertical.max(max_edge_height);
        let directional_margin = |margins: Insets, side: Orientation| match side {
            Orientation::TopLeft => (margins.left.trunc(), margins.top.trunc()),
            Orientation::Top => (0.0, margins.top.trunc()),
            Orientation::TopRight => (margins.right.trunc(), margins.top.trunc()),
            Orientation::Right => (margins.right.trunc(), 0.0),
            Orientation::BottomRight => (margins.right.trunc(), margins.bottom.trunc()),
            Orientation::Bottom => (0.0, margins.bottom.trunc()),
            Orientation::BottomLeft => (margins.left.trunc(), margins.bottom.trunc()),
            Orientation::Left => (margins.left.trunc(), 0.0),
            Orientation::None => (0.0, 0.0),
        };
        let (container_margin_width, container_margin_height) =
            directional_margin(container.layout_margins, orientation.opposite());
        let (node_margin_width, node_margin_height) =
            directional_margin(node_ref.layout_margins, orientation);
        horizontal = horizontal.max(container_margin_width + node_margin_width);
        vertical = vertical.max(container_margin_height + node_margin_height);
        let mut delta = match orientation {
            Orientation::Top | Orientation::Bottom => vertical,
            Orientation::Right | Orientation::Left => horizontal,
            _ => horizontal.min(vertical),
        };
        delta = delta.max(loop_spacing_gap(
            container.loop_offsets.as_ref(),
            node_ref.loop_offsets.as_ref(),
            orientation,
        ));
        delta
    }

    /// External half of recovered `Transaction.Commit` container validation.
    ///
    /// External ordinary containers are strict because the transaction
    /// snapshots overlap exceptions only among `Graph.Nodes`. External
    /// members of an active cluster are synchronized from their vessel before
    /// `IsBadState`; the flattened Rust scope therefore validates their
    /// current vessel node instead of stale member snapshots.
    pub(super) fn transaction_external_containers_are_valid(&self) -> bool {
        self.transaction_external_containers_are_valid_with_exceptions(&BTreeSet::new())
    }

    pub(super) fn transaction_external_containers_are_valid_with_exceptions(
        &self,
        existing_external_overlaps: &BTreeSet<(i64, i64)>,
    ) -> bool {
        let trace_external_container =
            crate::internal::trace_env_value("TALA_TRACE_EXTERNAL_CONTAINER");
        if let Some(target) = trace_external_container.as_deref()
            && crate::internal::trace_env_enabled("TALA_TRACE_SIZED_PASS_ACTIVE")
        {
            eprintln!(
                "EXTERNAL_CONTAINER_LIST_RUST target={} live={:?} validation={:?}",
                target,
                self.transaction_external_containers
                    .iter()
                    .map(|container| container.tala_id)
                    .collect::<Vec<_>>(),
                self.transaction_validation_external_containers
                    .iter()
                    .map(|container| container.tala_id)
                    .collect::<Vec<_>>()
            );
        }
        let validate_external = |container: &ArenaNode| {
            // Graph.Transaction.Commit iterates concrete Graph.Containers but
            // skips an unpositioned source container before IsBadState. Its
            // children may still carry absolute coordinates from the owning
            // graph, so interpreting them as local coordinates here creates
            // a false rejection for otherwise valid transpose candidates.
            if container.position.is_none() && container.transaction_anchor_tala_id.is_none() {
                return true;
            }
            if container
                .scoring_cluster_vessel
                .is_some_and(|vessel| self.nodes.iter().any(|node| node.tala_id == vessel))
            {
                return true;
            }
            // The source Graph.Containers map stores children in their
            // parent's local frame.  A temporary placement graph can retain
            // the parent pointer with a nil TopLeft while its direct child
            // boxes are already materialized (the ordinary group scope is
            // the common case).  Go's Transaction.Commit still validates
            // that container through Node.IsBadState; do the equivalent
            // strict containment check before deciding that a nil parent is
            // irrelevant.  Without this, transpose can rotate a leaf branch
            // outside its source group and Rust accepts a candidate that Go
            // rejects.
            // A projected carrier with children has already received the
            // shared-pointer move/refit for this trial, so its live position
            // is authoritative. Standalone external containers must resolve
            // their anchor again because the anchor may have moved since the
            // previous transaction.
            let container_position = if let Some(children) = self
                .transaction_external_container_children
                .get(&container.tala_id)
            {
                // Split-scope child projections are often already in the
                // local frame while the validation-only parent retained its
                // owning-graph coordinates. Go's shared pointer has the
                // local container origin after wrapChildren; use that origin
                // for the validation projection when the frames differ.
                let child_min =
                    children
                        .iter()
                        .filter_map(|child| child.position)
                        .reduce(|left, right| Point {
                            x: left.x.min(right.x),
                            y: left.y.min(right.y),
                        });
                match (container.position, child_min) {
                    (Some(parent), Some(child_min))
                        if container.transaction_validation_only
                            && ((parent.x - child_min.x).abs() > 500.0
                                || (parent.y - child_min.y).abs() > 500.0) =>
                    {
                        Some(Point::default())
                    }
                    (position, _) => {
                        position.or_else(|| self.anchored_external_position(container))
                    }
                }
            } else {
                self.anchored_external_position(container)
                    .or(container.position)
            };
            if crate::internal::trace_env_value("TALA_TRACE_TRANSPOSE_BOXES")
                .is_some_and(|target| target == "742559388")
                && container.tala_id == 2609282659
            {
                eprintln!(
                    "EXTERNAL_CONTAINER_CHECK_RUST container={} position={:?} size={:?} nodes={:?}",
                    container.tala_id,
                    container_position,
                    container.rect.size,
                    self.nodes
                        .iter()
                        .filter_map(|node| Some((node.tala_id, node.position?, node.rect.size)))
                        .collect::<Vec<_>>()
                );
            }
            let Some(container_position) = container_position else {
                return true;
            };
            let trace_this_container = trace_external_container
                .as_deref()
                .is_some_and(|target| target == "all" || target == container.tala_id.to_string())
                && crate::internal::trace_env_enabled("TALA_TRACE_SIZED_PASS_ACTIVE");
            if trace_this_container {
                eprintln!(
                    "EXTERNAL_CONTAINER_CANDIDATE_RUST container={} position={:?} size={:?} validation_only={} anchor={:?} anchor_offset={:?} scope_translation={:?} materialized={} children={:?}",
                    container.tala_id,
                    container_position,
                    container.rect.size,
                    container.transaction_validation_only,
                    container.transaction_anchor_tala_id,
                    container.transaction_anchor_offset,
                    container.unpositioned_scope_translation,
                    container.scope_translation_materialized,
                    self.transaction_external_container_children
                        .get(&container.tala_id)
                        .map(|children| children
                            .iter()
                            .map(|child| (
                                child.tala_id,
                                child.position,
                                child.rect.size,
                                child.transaction_anchor_tala_id,
                                child.transaction_anchor_offset,
                                child.unpositioned_scope_translation,
                                child.scope_translation_materialized,
                            ))
                            .collect::<Vec<_>>())
                );
            }
            let valid = self.external_container_position_is_valid(
                container,
                container_position,
                existing_external_overlaps,
            );
            #[cfg(feature = "diagnostic-traces")]
            if !valid && crate::internal::trace_env_enabled("TALA_TRACE_TRANSACTION_INVALID") {
                eprintln!(
                    "EXTERNAL_TRANSACTION_INVALID_RUST container={} position={:?} size={:?} anchor={:?} primary_aliases={:?} validation_aliases={:?} children={:?}",
                    container.tala_id,
                    container_position,
                    container.rect.size,
                    container.transaction_anchor_tala_id,
                    self.transaction_external_containers
                        .iter()
                        .filter(|alias| alias.tala_id == container.tala_id)
                        .map(|alias| (
                            alias.position,
                            alias.rect.size,
                            alias.transaction_validation_only
                        ))
                        .collect::<Vec<_>>(),
                    self.transaction_validation_external_containers
                        .iter()
                        .filter(|alias| alias.tala_id == container.tala_id)
                        .map(|alias| (
                            alias.position,
                            alias.rect.size,
                            alias.transaction_validation_only
                        ))
                        .collect::<Vec<_>>(),
                    self.transaction_external_container_children
                        .get(&container.tala_id)
                        .map(|children| children
                            .iter()
                            .map(|child| (child.tala_id, child.position, child.rect.size))
                            .collect::<Vec<_>>()),
                );
            }
            if crate::internal::trace_env_value("TALA_TRACE_TRANSPOSE_BOXES")
                .is_some_and(|target| target == "742559388")
                && container.tala_id == 2609282659
            {
                eprintln!(
                    "EXTERNAL_CONTAINER_RESULT_RUST container={} position={:?} valid={}",
                    container.tala_id, container_position, valid
                );
            }
            valid
        };
        let external_valid = self
            .transaction_external_containers
            .iter()
            .all(validate_external)
            && self
                .transaction_validation_external_containers
                .iter()
                // Go's Graph.Containers and Graph.Nodes share pointers. A
                // validation-only projection can therefore duplicate a
                // container that was also refit as a movable external
                // carrier during this transaction. Validate the live carrier
                // once; checking its stale clone can reject a candidate that
                // Go accepts after repositionContainers.
                .filter(|validation| {
                    !self
                        .transaction_external_containers
                        .iter()
                        .any(|external| external.tala_id == validation.tala_id)
                })
                .all(validate_external);
        if !external_valid {
            return false;
        }

        // The Go transaction validates only its concrete Graph.Containers
        // entries. Projected parents are shared aliases whose descendants can
        // be absent from the induced Graph.Nodes slice; validating their
        // wrapped box against that slice would treat descendants as external
        // overlaps and reject valid transpose candidates.
        true
    }

    fn external_container_position_is_valid(
        &self,
        container: &ArenaNode,
        container_position: Point,
        existing_external_overlaps: &BTreeSet<(i64, i64)>,
    ) -> bool {
        let trace_this_container =
            crate::internal::trace_env_value("TALA_TRACE_EXTERNAL_CONTAINER")
                .is_some_and(|target| target == "all" || target == container.tala_id.to_string())
                && crate::internal::trace_env_enabled("TALA_TRACE_SIZED_PASS_ACTIVE");
        self.nodes.iter().all(|node| {
            let Some(node_position) = node.position else {
                return true;
            };
            if crate::internal::trace_env_enabled("TALA_TRACE_EXTERNAL_PAIR")
                && container.tala_id == 3809224601
                && node.tala_id == 2314375987
            {
                eprintln!(
                    "EXTERNAL_PAIR_CHECK_RUST container={} cpos={:?} csize={:?} node={} npos={:?} nsize={:?} c_anc={:?} n_anc={:?} c_edges={:?} n_edges={:?} existing={}",
                    container.tala_id,
                    container_position,
                    container.rect.size,
                    node.tala_id,
                    node_position,
                    node.rect.size,
                    container.scoring_container_ancestors,
                    node.scoring_container_ancestors,
                    container.edges,
                    node.edges,
                    existing_external_overlaps.contains(&(container.tala_id, node.tala_id)),
                );
            }
            if node.tala_id == container.tala_id {
                return true;
            }
            let node_inside_container = node
                .scoring_container_ancestors
                .contains(&container.tala_id);
            let container_inside_node = container
                .scoring_container_ancestors
                .contains(&node.tala_id);
            let trace_external_node =
                crate::internal::trace_env_value("TALA_TRACE_EXTERNAL_NODE");
            let trace_pair = trace_this_container
                && trace_external_node.as_deref().is_some_and(|targets| {
                    targets == "all"
                        || targets
                            .split(',')
                            .any(|target| target.trim() == node.tala_id.to_string())
                });
            if trace_pair {
                eprintln!(
                    "EXTERNAL_CONTAINER_PAIR_RUST container={} cpos={:?}/{:?} node={} npos={:?}/{:?} inside={} reverse={} existing={}",
                    container.tala_id,
                    container_position,
                    container.rect.size,
                    node.tala_id,
                    node_position,
                    node.rect.size,
                    node_inside_container,
                    container_inside_node,
                    existing_external_overlaps.contains(&(container.tala_id, node.tala_id))
                );
            }
            if node_inside_container {
                // Go's IsBadState exempts ancestor/descendant pairs from the
                // spacing-overlap pass, but its structural pass still
                // requires each child to fit strictly inside its parent.
                // Split placement scopes keep positioned parents outside the
                // local `containers` map, so preserve that invariant here
                // before treating the pair as an overlap exemption.
                return node_position.x > container_position.x
                    && node_position.y > container_position.y
                    && node_position.x + node.rect.size.width
                        < container_position.x + container.rect.size.width
                    && node_position.y + node.rect.size.height
                        < container_position.y + container.rect.size.height;
            }
            if container_inside_node {
                return true;
            }
            if existing_external_overlaps.contains(&(container.tala_id, node.tala_id)) {
                return true;
            }
            let delta = self.external_container_spacing_delta(container, node);
            let overlaps = container_position.x < node_position.x + node.rect.size.width + delta
                && container_position.x + container.rect.size.width + delta > node_position.x
                && container_position.y < node_position.y + node.rect.size.height + delta
                && container_position.y + container.rect.size.height + delta > node_position.y;
            if trace_pair {
                eprintln!(
                    "EXTERNAL_CONTAINER_SPACING_RUST container={} node={} delta={} overlaps={}",
                    container.tala_id, node.tala_id, delta, overlaps
                );
            }
            if crate::internal::trace_env_enabled("TALA_TRACE_TRANSPOSE_BOXES")
                && crate::internal::trace_env_value("TALA_TRACE_TRANSPOSE_BOXES")
                    .is_some_and(|target| target == "1364103258")
                && container.tala_id == 354304103
            {
                eprintln!(
                    "EXTERNAL_PAIR_RUST container={} node={} cpos={:?}/{:?} npos={:?}/{:?} delta={} overlap={}",
                    container.tala_id,
                    node.tala_id,
                    container_position,
                    container.rect.size,
                    node_position,
                    node.rect.size,
                    delta,
                    container_position.x < node_position.x + node.rect.size.width + delta
                        && container_position.x + container.rect.size.width + delta > node_position.x
                        && container_position.y < node_position.y + node.rect.size.height + delta
                        && container_position.y + container.rect.size.height + delta > node_position.y
                );
            }

            !overlaps
        })
    }

    pub(super) fn existing_external_overlap_pairs(&self) -> BTreeSet<(i64, i64)> {
        let mut pairs = BTreeSet::new();
        for container in &self.transaction_external_containers {
            let Some(container_position) = self.anchored_external_position(container) else {
                continue;
            };
            for node in &self.nodes {
                let Some(node_position) = node.position else {
                    continue;
                };
                if container.tala_id == node.tala_id {
                    continue;
                }
                // Cluster flip trials retain the recovered aggregate's
                // pre-existing external spacing while evaluating a new
                // vessel arrangement. This is separate from GraphState's
                // ordinary transaction snapshot, which only contains local
                // Graph.Nodes pairs.
                let delta = self.external_container_spacing_delta(container, node);
                let overlaps = container_position.x
                    < node_position.x + node.rect.size.width + delta
                    && node_position.x < container_position.x + container.rect.size.width + delta
                    && container_position.y < node_position.y + node.rect.size.height + delta
                    && node_position.y < container_position.y + container.rect.size.height + delta;
                if overlaps {
                    pairs.insert((container.tala_id, node.tala_id));
                }
            }
        }
        pairs
    }

    #[track_caller]
    pub(super) fn move_node_abs_with_children(&mut self, node: NodeId, target: Point) {
        let trace_focus_move = self.nodes[node.0 as usize].tala_id == 1051177149
            && target.x == 300.0
            && target.y == -1425.0;
        if trace_focus_move {
            self.trace_target_shared_geometry("move-focus-before");
        }
        if crate::internal::trace_env_enabled("TALA_TRACE_CALLER_3447521236")
            && self.nodes[node.0 as usize].tala_id == 3447521236
            && target.y == self.position(node).unwrap_or_default().y - 1.0
        {
            eprintln!(
                "CALLER_MOVE_NODE_ABS_RUST caller={} target={:?} current={:?}",
                std::panic::Location::caller(),
                target,
                self.active_node_position(node)
            );
        }
        // Go's InitializeNodes gives every node a concrete `(0, 0)` box
        // before `moveNodeAbsWithChildren` is called.  The Rust arena may
        // still carry `None` for a cloned scope, but treating that as a direct
        // assignment skips the descendant translation (and pending cluster
        // vessel translation) that the Go call performs.  Materialize the
        // same zero origin, then take the ordinary delta path.
        let current = self.position(node).unwrap_or_default();
        if self.position(node).is_none() {
            self.set_position(node, current);
        }
        if current == target {
            return;
        }
        let delta = Point {
            x: target.x - current.x,
            y: target.y - current.y,
        };
        self.translate_node_with_children(node, delta);
        if trace_focus_move {
            self.trace_target_shared_geometry("move-focus-after");
        }
    }

    #[track_caller]
    pub(super) fn translate_node_with_children(&mut self, node: NodeId, delta: Point) {
        // Exact zero: adding it leaves every box, projection and owner point
        // unchanged, so skip the projected-subtree walk.
        if delta.x == 0.0 && delta.y == 0.0 {
            return;
        }
        self.translate_node_with_children_walk(node, delta);
    }

    #[track_caller]
    fn translate_node_with_children_walk(&mut self, node: NodeId, delta: Point) {
        if crate::internal::trace_env_enabled("TALA_TRACE_MOVE_DEBUG")
            && self.nodes[node.0 as usize].tala_id == 3677471320
            && (delta.x.abs() > 100.0 || delta.y.abs() > 100.0)
        {
            eprintln!(
                "MOVE_DEBUG_RUST receiver=3677471320 delta={},{} nodes={} order={} clusters={} sequences={} cluster_backlinks={} sequence_backlinks={} active_cluster={:?} active_sequence={:?} ordinary_keys={:?} aggregate_keys={:?} target_aggregate={:?}",
                delta.x,
                delta.y,
                self.nodes.len(),
                self.node_order.len(),
                self.clusters.len(),
                self.sequences.len(),
                self.cluster_backlinks_complete,
                self.sequence_backlinks_complete,
                self.active_cluster_index(node),
                self.active_sequence_index(node),
                self.transaction_external_container_children
                    .keys()
                    .collect::<Vec<_>>(),
                self.transaction_external_aggregate_children
                    .keys()
                    .collect::<Vec<_>>(),
                self.transaction_external_aggregate_children
                    .values()
                    .flatten()
                    .find(|child| child.tala_id == 3946625030)
                    .map(|child| child.position),
            );
        }
        if crate::internal::trace_env_enabled("TALA_TRACE_CALLER_3447521236")
            && self.nodes[node.0 as usize].tala_id == 3447521236
            && delta.y == -1.0
        {
            eprintln!(
                "CALLER_TRANSLATE_RUST caller={} delta={:?} current={:?}",
                std::panic::Location::caller(),
                delta,
                self.position(node)
            );
        }
        let trace_projection = crate::internal::trace_env_value("TALA_TRACE_MOVE_PROJECTION")
            .is_some_and(|target| {
                target == "all"
                    || target
                        .parse::<i64>()
                        .ok()
                        .is_some_and(|id| id == self.nodes[node.0 as usize].tala_id)
            });
        if trace_projection {
            eprintln!(
                "MOVE_PROJECTION_RUST begin caller={} node={} delta={},{} pos={:?} focus6={:?} descendants={:?}",
                std::panic::Location::caller(),
                self.nodes[node.0 as usize].tala_id,
                delta.x,
                delta.y,
                self.nodes[node.0 as usize].position,
                self.nodes
                    .iter()
                    .filter(|candidate| candidate.tala_id == 856466825)
                    .map(|candidate| candidate.position)
                    .collect::<Vec<_>>(),
                self.active_descendants(node)
                    .iter()
                    .map(|child| self.nodes[child.0 as usize].tala_id)
                    .collect::<Vec<_>>()
            );
            for (&parent, children) in &self.transaction_external_container_children {
                if true {
                    eprintln!(
                        "MOVE_PROJECTION_RUST before parent={} children={:?}",
                        parent,
                        children
                            .iter()
                            .map(|child| (child.tala_id, child.position))
                            .collect::<Vec<_>>()
                    );
                }
            }
            eprintln!(
                "MOVE_PROJECTION_RUST aggregate_before={:?}",
                self.transaction_external_aggregate_children
                    .iter()
                    .map(|(parent, children)| (
                        *parent,
                        children
                            .iter()
                            .map(|child| (child.tala_id, child.position))
                            .collect::<Vec<_>>(),
                    ))
                    .collect::<Vec<_>>()
            );
        }
        // TALA walks the live Graph.Containers hierarchy for every move.
        // The cached descendant slice belongs to the stable input graph and
        // can be stale or differently indexed in an induced placement graph;
        // use the current hierarchy so shared child pointers translate with
        // their container exactly as the recovered Go implementation does.
        // A placement scope can retain a source container as a scoring-only
        // projection while its nested children belong to another owning Go
        // graph.  Graph.moveNodeWithChildren on that projection walks the
        // owning graph's Containers map and therefore does not translate the
        // flattened child a second time.  The child is still translated when
        // it appears directly in the current connected-node set.
        let active_descendants = if self.nodes[node.0 as usize].scoring_is_container
            && !self.nodes[node.0 as usize].is_container
        {
            Vec::new()
        } else {
            self.active_descendants(node)
        };
        // These are stable owner nodes standing in for aggregate vessels in
        // an ordinary container's recovered Containers traversal. Keep the
        // distinction from `node` itself: optimizer operations may move one
        // real cluster member directly, and that must not translate every
        // sibling merely because the member is also the Rust vessel proxy.
        // Membership sets are hashed. `moved` is applied below in ascending
        // NodeId order, exactly as the BTreeSet iterated it: nodes sharing a
        // point refresh their rect origins in that order.
        let aggregate_descendant_proxies =
            active_descendants.iter().copied().collect::<IdHashSet<_>>();
        // The closure below grows both sets; size them once for that growth
        // instead of collecting and then rehashing into a larger table. They
        // are only probed or sorted, so table capacity is not observable.
        let mut moved = IdHashSet::with_capacity_and_hasher(
            active_descendants.len() + 1 + 32,
            Default::default(),
        );
        moved.extend(active_descendants.into_iter().chain(std::iter::once(node)));
        // TALA's induced graphs retain Node pointers shared with their owning
        // Containers map. Rust clones those projections, so translate every
        // projected copy of the moved subtree too, including hidden aggregate
        // vessels that have no stable NodeId.
        let mut moved_tala_ids =
            IdHashSet::with_capacity_and_hasher(moved.len() + 32, Default::default());
        moved_tala_ids.extend(
            moved
                .iter()
                .map(|moved| self.nodes[moved.0 as usize].tala_id),
        );
        // A cluster vessel is a distinct Go Node pointer. The first retained
        // member is only its stable-arena carrier; moving that member directly
        // must not move the vessel. Record vessel traversal at the point where
        // getAllDescendantNodes actually crosses the synthetic vessel instead
        // of inferring it later from the carrier appearing in `moved`.
        let mut moved_cluster_vessels = IdHashSet::default();
        if trace_projection {
            eprintln!(
                "MOVE_PROJECTION_RUST_CLUSTERS {:?}",
                self.clusters
                    .iter()
                    .map(|cluster| (
                        cluster.vessel_tala_id,
                        cluster
                            .members
                            .iter()
                            .map(|member| self.nodes[member.0 as usize].tala_id)
                            .collect::<Vec<_>>()
                    ))
                    .collect::<Vec<_>>()
            );
        }
        loop {
            let mut changed = false;
            for (cluster_index, cluster) in self.clusters.iter().enumerate() {
                // Recovered Node.moveNodeWithChildren calls
                // getAllDescendantNodes(node, true).  The `true` branch
                // appends every node owned by a cluster vessel.  Rust keeps
                // the vessel as its first stable member, so recognize that
                // proxy in addition to projections that carry the synthetic
                // vessel TALA ID.
                let owner_is_descendant = cluster
                    .members
                    .first()
                    .is_some_and(|owner| aggregate_descendant_proxies.contains(owner));
                // A nested scope has already materialized the Go vessel as a
                // pending position even when this stable arena still lists
                // every original member in node_order. In that state
                // cluster_is_active() is false, but the shared Go
                // Containers walk still reaches the distinct vessel pointer
                // and translates it with its containing node.
                let vessel_is_pending = self
                    .pending_cluster_vessel_positions
                    .contains_key(&cluster_index);
                if !(moved_tala_ids.contains(&cluster.vessel_tala_id)
                    || owner_is_descendant
                        && (self.cluster_is_active(cluster) || vessel_is_pending))
                {
                    continue;
                }
                moved_cluster_vessels.insert(cluster_index);
                for member in &cluster.members {
                    changed |= moved.insert(*member);
                    changed |= moved_tala_ids.insert(self.nodes[member.0 as usize].tala_id);
                    // AddClusters can absorb an already-materialized sequence
                    // vessel as one cluster member. Go's independent cluster
                    // and sequence passes then recurse through that vessel as
                    // well, so carry its synthetic ID into the next closure
                    // iteration and expose every hidden sequence step.
                    if let Some(sequence_index) = self.active_sequence_index(*member)
                        && self.sequences[sequence_index].members.first() == Some(member)
                    {
                        changed |=
                            moved_tala_ids.insert(self.sequences[sequence_index].vessel_tala_id);
                    }
                }
            }
            for sequence in &self.sequences {
                // Sequences are an independent pass in
                // getAllDescendantNodes and likewise expose every step when
                // includeClusterNodes is true.
                if !(moved_tala_ids.contains(&sequence.vessel_tala_id)
                    || self.sequence_is_active(sequence)
                        && sequence
                            .members
                            .first()
                            .is_some_and(|owner| aggregate_descendant_proxies.contains(owner)))
                {
                    continue;
                }
                for member in &sequence.members {
                    changed |= moved.insert(*member);
                    changed |= moved_tala_ids.insert(self.nodes[member.0 as usize].tala_id);
                }
            }
            for (&container_tala_id, children) in &self.transaction_external_container_children {
                if !moved_tala_ids.contains(&container_tala_id) {
                    continue;
                }
                for child in children {
                    changed |= moved_tala_ids.insert(child.tala_id);
                    if let Some(stable) = self
                        .nodes
                        .iter()
                        .position(|candidate| candidate.tala_id == child.tala_id)
                    {
                        changed |= moved.insert(NodeId(stable as u32));
                    }
                }
            }
            for (&container_tala_id, children) in &self.transaction_external_aggregate_children {
                if !moved_tala_ids.contains(&container_tala_id) {
                    continue;
                }
                for child in children {
                    changed |= moved_tala_ids.insert(child.tala_id);
                    if let Some(stable) = self
                        .nodes
                        .iter()
                        .position(|candidate| candidate.tala_id == child.tala_id)
                    {
                        changed |= moved.insert(NodeId(stable as u32));
                    }
                }
            }
            if !changed {
                break;
            }
            if trace_projection {
                eprintln!(
                    "MOVE_PROJECTION_RUST_SET ids={:?} moved={:?}",
                    moved_tala_ids.iter().collect::<BTreeSet<_>>(),
                    moved
                        .iter()
                        .collect::<BTreeSet<_>>()
                        .into_iter()
                        .map(|id| self.nodes[id.0 as usize].tala_id)
                        .collect::<Vec<_>>()
                );
            }
        }
        // Diagnostic-only witness for parity investigations.  The Go
        // implementation moves the receiver and the exact descendant walk
        // returned by allDescendantNodes; record the closure before mutating
        // any boxes so a translated sibling can be attributed to its caller.
        if let Some(targets) = crate::internal::trace_env_value("TALA_TRACE_MOVE_CONTAINS") {
            let targets = targets
                .split(',')
                .filter_map(|target| target.trim().parse::<i64>().ok())
                .collect::<BTreeSet<_>>();
            let contains_target = moved_tala_ids.iter().any(|id| targets.contains(id));
            if contains_target {
                eprintln!(
                    "MOVE_CONTAINS_RUST caller={} node={} delta={},{} moved={:?} targets={:?}",
                    std::panic::Location::caller(),
                    self.nodes[node.0 as usize].tala_id,
                    delta.x,
                    delta.y,
                    moved_tala_ids.iter().collect::<BTreeSet<_>>(),
                    targets,
                );
            }
        }
        let trace_move_child = crate::internal::trace_env_value("TALA_TRACE_MOVE_CHILD")
            .and_then(|target| target.parse::<i64>().ok())
            .filter(|target| moved_tala_ids.contains(target));
        if let Some(target) = trace_move_child {
            eprintln!(
                "MOVE_CHILD_RUST before caller={} receiver={} delta={},{} target={} maps={:?} stable={:?}",
                std::panic::Location::caller(),
                self.nodes[node.0 as usize].tala_id,
                delta.x,
                delta.y,
                target,
                self.transaction_external_container_children
                    .iter()
                    .filter_map(|(parent, children)| {
                        children
                            .iter()
                            .find(|child| child.tala_id == target)
                            .map(|child| (*parent, child.position))
                    })
                    .collect::<Vec<_>>(),
                self.nodes
                    .iter()
                    .find(|node| node.tala_id == target)
                    .and_then(|node| node.position)
            );
        }
        // The closure is final here. Every external projection is probed
        // against it below; a small closure is a linear scan, not a hash
        // probe per projection. Membership is unchanged.
        let moved_tala_list =
            (moved_tala_ids.len() <= 8).then(|| moved_tala_ids.iter().copied().collect::<Vec<_>>());
        let is_moved = |tala_id: i64| match &moved_tala_list {
            Some(list) => list.contains(&tala_id),
            None => moved_tala_ids.contains(&tala_id),
        };
        for children in self.transaction_external_container_children.values_mut() {
            for child in children {
                if is_moved(child.tala_id)
                    && let Some(position) = child.position.as_mut()
                {
                    position.x += delta.x;
                    position.y += delta.y;
                    child.rect.origin = *position;
                }
            }
        }
        for children in self.transaction_external_aggregate_children.values_mut() {
            for child in children {
                if is_moved(child.tala_id)
                    && let Some(position) = child.position.as_mut()
                {
                    position.x += delta.x;
                    position.y += delta.y;
                    child.rect.origin = *position;
                }
            }
        }
        // Graph.Containers and Graph.Nodes hold the same Go *Node pointers.
        // Keep the flattened external container carrier in that same move
        // set; otherwise a later refit reads an old container origin while
        // its projected child aliases have already translated.
        for container in &mut self.transaction_external_containers {
            if is_moved(container.tala_id)
                && let Some(position) = container.position.as_mut()
            {
                position.x += delta.x;
                position.y += delta.y;
                container.rect.origin = *position;
            }
        }
        for (&cluster_index, position) in &mut self.pending_cluster_vessel_positions {
            if moved_cluster_vessels.contains(&cluster_index) {
                position.x += delta.x;
                position.y += delta.y;
            }
        }
        let mut moved = moved.into_iter().collect::<Vec<_>>();
        moved.sort_unstable();
        for moved in moved {
            if trace_projection {
                eprintln!(
                    "MOVE_PROJECTION_RUST_MOVED node={} pos={:?}",
                    self.nodes[moved.0 as usize].tala_id,
                    self.position(moved)
                );
            }
            // Node.translate mutates the actual TopLeft allocation. A bound
            // vessel can retain an older inline point after placement; moving
            // only that copy leaves transaction validation at its old origin.
            if self.position(moved).is_some() {
                self.translate_owner_point(moved, delta.x, delta.y);
            }
        }
        if let Some(target) = trace_move_child {
            eprintln!(
                "MOVE_CHILD_RUST after caller={} receiver={} target={} maps={:?} stable={:?}",
                std::panic::Location::caller(),
                self.nodes[node.0 as usize].tala_id,
                target,
                self.transaction_external_container_children
                    .iter()
                    .filter_map(|(parent, children)| {
                        children
                            .iter()
                            .find(|child| child.tala_id == target)
                            .map(|child| (*parent, child.position))
                    })
                    .collect::<Vec<_>>(),
                self.nodes
                    .iter()
                    .find(|node| node.tala_id == target)
                    .and_then(|node| node.position)
            );
        }
        if trace_projection {
            eprintln!(
                "MOVE_PROJECTION_RUST after node={} pos={:?}",
                self.nodes[node.0 as usize].tala_id, self.nodes[node.0 as usize].position
            );
            for (&parent, children) in &self.transaction_external_container_children {
                if true {
                    eprintln!(
                        "MOVE_PROJECTION_RUST after parent={} children={:?}",
                        parent,
                        children
                            .iter()
                            .map(|child| (child.tala_id, child.position))
                            .collect::<Vec<_>>()
                    );
                }
            }
            eprintln!(
                "MOVE_PROJECTION_RUST aggregate_after={:?}",
                self.transaction_external_aggregate_children
                    .iter()
                    .map(|(parent, children)| (
                        *parent,
                        children
                            .iter()
                            .map(|child| (child.tala_id, child.position))
                            .collect::<Vec<_>>(),
                    ))
                    .collect::<Vec<_>>()
            );
        }
    }

    /// Translate one cluster member box without walking flattened descendants.
    ///
    /// A cluster is synchronized in the owning Go graph.  When a member is a
    /// container whose children belong to a nested graph, the member pointer
    /// is in the current Graph while those children are not; Go's
    /// `moveNodeWithChildren` therefore moves only the member from this
    /// graph's perspective.  The Rust arena flattens both scopes, so using
    /// the ordinary descendant walk would translate those children twice.
    pub(super) fn translate_node_only(&mut self, node: NodeId, delta: Point) {
        if delta == Point::default() {
            return;
        }
        if let Some(position) = self.position(node) {
            Pipeline::publish_induced_owner_point(
                self,
                node,
                Some(Point {
                    x: position.x + delta.x,
                    y: position.y + delta.y,
                }),
            );
        }
        let tala_id = self.nodes[node.0 as usize].tala_id;
        for children in self.transaction_external_container_children.values_mut() {
            for child in children {
                if child.tala_id == tala_id
                    && let Some(position) = child.position.as_mut()
                {
                    position.x += delta.x;
                    position.y += delta.y;
                    child.rect.origin = *position;
                }
            }
        }
        for children in self.transaction_external_aggregate_children.values_mut() {
            for child in children {
                if child.tala_id == tala_id
                    && let Some(position) = child.position.as_mut()
                {
                    position.x += delta.x;
                    position.y += delta.y;
                    child.rect.origin = *position;
                }
            }
        }
        for container in &mut self.transaction_external_containers {
            if container.tala_id == tala_id
                && let Some(position) = container.position.as_mut()
            {
                position.x += delta.x;
                position.y += delta.y;
                container.rect.origin = *position;
            }
        }
    }

    pub(super) fn sequence_is_active(&self, sequence: &SequenceState) -> bool {
        sequence.members.len() > 1
            && sequence
                .members
                .iter()
                .skip(1)
                .any(|member| !self.node_order_contains(*member))
    }

    pub(super) fn cluster_is_active(&self, cluster: &ClusterState) -> bool {
        let active = cluster.members.len() > 1
            && cluster.members.first().is_some_and(|owner| {
                self.node_order_contains(*owner)
                    && cluster
                        .members
                        .iter()
                        .skip(1)
                        .any(|member| !self.node_order_contains(*member))
            });
        if active && crate::internal::trace_env_enabled("TALA_TRACE_CLUSTER_ACTIVE_DETAIL") {
            eprintln!(
                "CLUSTER_ACTIVE_DETAIL_RUST vessel={} members={:?} node_order={:?}",
                cluster.vessel_tala_id,
                cluster
                    .members
                    .iter()
                    .map(|member| self.nodes[member.0 as usize].tala_id)
                    .collect::<Vec<_>>(),
                self.node_order
                    .iter()
                    .map(|node| self.nodes[node.0 as usize].tala_id)
                    .collect::<Vec<_>>()
            );
        }
        active
    }

    #[inline]
    pub(super) fn node_order_contains(&self, node: NodeId) -> bool {
        if self.node_order_membership_valid {
            self.node_order_membership
                .get(node.0 as usize)
                .copied()
                .unwrap_or(false)
        } else {
            self.node_order.contains(&node)
        }
    }

    #[inline(always)]
    pub(super) fn active_sequence_index(&self, node: NodeId) -> Option<usize> {
        if self.node_order_membership_valid {
            return self
                .active_sequence_indices
                .get(node.0 as usize)
                .copied()
                .flatten();
        }
        self.active_sequence_index_slow(node)
    }

    #[cold]
    #[inline(never)]
    fn active_sequence_index_slow(&self, node: NodeId) -> Option<usize> {
        // AddSequences reconnects outside edges to the allocated vessel.
        // Numerical placement still represents that vessel by its first
        // member, so resolve the bound pointer before following backlinks.
        if let Some(index) = self
            .sequences
            .iter()
            .enumerate()
            .find_map(|(index, sequence)| {
                // Identity is independent of membership and geometry. Reject
                // unrelated vessels before the uncached activity scan.
                (!self.node_order_contains(node)
                    && self.topology_vessel_node(TopologyVessel::Sequence(index)) == Some(node)
                    && self.sequence_is_active(sequence))
                .then_some(index)
            })
        {
            return Some(index);
        }
        if let Some(index) = self.nodes[node.0 as usize].sequence {
            let active = self.sequence_is_active(&self.sequences[index]);
            return active.then_some(index);
        }
        // Hand-materialized placement graphs and recovered partial scopes can
        // carry the original Sequence slice before node backlinks are copied.
        (!self.sequence_backlinks_complete)
            .then(|| {
                self.sequences.iter().position(|sequence| {
                    self.sequence_is_active(sequence) && sequence.members.contains(&node)
                })
            })
            .flatten()
    }

    #[inline(always)]
    pub(super) fn active_cluster_index(&self, node: NodeId) -> Option<usize> {
        if self.node_order_membership_valid {
            return self
                .active_cluster_indices
                .get(node.0 as usize)
                .copied()
                .flatten();
        }
        self.active_cluster_index_slow(node)
    }

    #[cold]
    #[inline(never)]
    fn active_cluster_index_slow(&self, node: NodeId) -> Option<usize> {
        let node = self.active_sequence_owner(node);
        // Abducted edges can already point at the canonical allocated vessel
        // while the numerical graph still uses its first member as carrier.
        // Resolve that exact bound pointer before treating it as an ordinary
        // node with historical geometry and no cluster abductions.
        if let Some(index) = self
            .clusters
            .iter()
            .enumerate()
            .find_map(|(index, cluster)| {
                // With an invalid membership index, cluster_is_active can
                // scan Graph.Nodes for each member. Only the exact matching
                // vessel needs that test; retain first-active-match order.
                (self.topology_vessel_node(TopologyVessel::Cluster(index)) == Some(node)
                    && self.cluster_is_active(cluster))
                .then_some(index)
            })
        {
            return Some(index);
        }
        if let Some(index) = self.nodes[node.0 as usize].cluster {
            let active = self.cluster_is_active(&self.clusters[index]);
            return active.then_some(index);
        }
        (!self.cluster_backlinks_complete)
            .then(|| {
                self.clusters.iter().position(|cluster| {
                    self.cluster_is_active(cluster) && cluster.members.contains(&node)
                })
            })
            .flatten()
    }

    #[inline(always)]
    pub(super) fn active_sequence_owner(&self, node: NodeId) -> NodeId {
        if self.node_order_membership_valid {
            return self.active_sequence_owners[node.0 as usize];
        }
        self.active_sequence_index(node)
            .and_then(|index| self.sequences[index].members.first().copied())
            .unwrap_or(node)
    }

    #[inline(always)]
    pub(super) fn active_cluster_owner(&self, node: NodeId) -> NodeId {
        if self.node_order_membership_valid {
            return self.active_cluster_owners[node.0 as usize];
        }
        self.active_cluster_index(node)
            .and_then(|index| self.clusters[index].members.first().copied())
            .unwrap_or(node)
    }

    pub(super) fn active_aggregate_members(&self, node: NodeId) -> Option<&[NodeId]> {
        self.clusters
            .iter()
            .find(|cluster| {
                self.cluster_is_active(cluster) && cluster.members.first() == Some(&node)
            })
            .map(|cluster| cluster.members.as_slice())
            .or_else(|| {
                self.sequences
                    .iter()
                    .find(|sequence| {
                        self.sequence_is_active(sequence)
                            && sequence.members.first() == Some(&node)
                            && self.active_cluster_owner(node) == node
                    })
                    .map(|sequence| sequence.members.as_slice())
            })
    }

    pub(super) fn active_aggregate_leaf_members(&self, node: NodeId) -> Option<Vec<NodeId>> {
        if let Some(cluster_index) = self.active_cluster_index(node)
            && self.clusters[cluster_index].members.first() == Some(&node)
        {
            let cluster = &self.clusters[cluster_index];
            let mut leaves = Vec::new();
            for member in cluster.members.iter().copied() {
                if let Some(sequence_index) = self.active_sequence_index(member) {
                    leaves.extend(self.sequences[sequence_index].members.iter().copied());
                } else {
                    leaves.push(member);
                }
            }
            return Some(leaves);
        }
        self.sequences
            .iter()
            .find(|sequence| {
                self.sequence_is_active(sequence)
                    && sequence.members.first() == Some(&node)
                    && self.active_cluster_owner(node) == node
            })
            .map(|sequence| sequence.members.clone())
    }

    #[inline(always)]
    pub(super) fn active_aggregate_owner(&self, node: NodeId) -> NodeId {
        if self.node_order_membership_valid {
            return self.active_cluster_owners[node.0 as usize];
        }
        self.active_cluster_owner(self.active_sequence_owner(node))
    }

    /// Reconstructs `Node.getContainer` while sequence and cluster vessels are
    /// present. TALA removes aggregate members from their ordinary container
    /// slices and assigns that container to the temporary vessel.
    #[inline(always)]
    pub(super) fn active_node_container(&self, node: NodeId) -> Option<NodeId> {
        if self.node_order_membership_valid {
            return self.active_node_containers[node.0 as usize];
        }
        if let Some(cluster_index) = self.active_cluster_index(node) {
            let first = *self.clusters[cluster_index].members.first()?;
            if let Some(sequence_index) = self.active_sequence_index(first) {
                return self.sequences[sequence_index].container;
            }
            if let Some(container_tala_id) = self
                .serialized_cluster_containers_by_tala
                .get(&self.clusters[cluster_index].vessel_tala_id)
            {
                return container_tala_id.and_then(|container_tala_id| {
                    self.nodes
                        .iter()
                        .find(|candidate| candidate.tala_id == container_tala_id)
                        .map(|candidate| candidate.input_id)
                });
            }
            return self.nodes[first.0 as usize].container;
        }
        if let Some(sequence_index) = self.active_sequence_index(node) {
            return self.sequences[sequence_index].container;
        }
        self.nodes[node.0 as usize].container
    }

    /// `Node.isDescendentOf` over the current graph while aggregate members
    /// are represented by their temporary sequence or cluster vessel.
    pub(super) fn active_is_descendant_of(&self, node: NodeId, ancestor: NodeId) -> bool {
        let node = self.active_aggregate_owner(node);
        let ancestor = self.active_aggregate_owner(ancestor);
        if node == ancestor {
            return true;
        }
        // A retained cluster member still appears as the raw container of its
        // ordinary descendants (for example, a child below the second
        // member). Go's Node.IsDescendentOf crosses that member's Cluster
        // pointer to the temporary vessel before walking Containers. Apply
        // the same aggregate hop at every container boundary.
        let mut container = self
            .active_node_container(node)
            .map(|container| self.active_aggregate_owner(container));
        while let Some(current) = container {
            if current == ancestor {
                return true;
            }
            container = self
                .active_node_container(current)
                .map(|container| self.active_aggregate_owner(container));
        }
        false
    }

    /// Descendant relation used by TALA's intersection checks while a
    /// cluster vessel is live. Go keeps a distinct vessel node and every
    /// cluster member points at it through `Node.Cluster`; the stable arena
    /// uses the first member as the carrier, so an ordinary descendant of a
    /// non-carrier member needs one extra cluster hop to reach that carrier.
    /// Keep the general active relation unchanged for placement/scoring and
    /// add this narrow vessel-aware predicate for alignment candidate tests.
    pub(super) fn active_is_descendant_of_vessel(&self, node: NodeId, ancestor: NodeId) -> bool {
        if self.active_is_descendant_of(node, ancestor) {
            return true;
        }
        let Some(cluster_index) = self.active_cluster_index(ancestor) else {
            return false;
        };
        if !self.cluster_is_active(&self.clusters[cluster_index]) {
            return false;
        }
        let mut current = node;
        let mut seen = BTreeSet::new();
        loop {
            if !seen.insert(current) {
                return false;
            }
            if self.nodes[current.0 as usize].cluster == Some(cluster_index) {
                return true;
            }
            let Some(next) = self.nodes[current.0 as usize].container else {
                return false;
            };
            current = next;
        }
    }

    /// Returns the ID of the current graph node, including a temporary
    /// aggregate vessel rather than its stable-arena owner.
    #[inline(always)]
    pub(super) fn active_node_tala_id(&self, node: NodeId) -> i64 {
        if self.node_order_membership_valid {
            if let Some(cluster_index) = self.active_cluster_indices[node.0 as usize] {
                return self.clusters[cluster_index].vessel_tala_id;
            }
            if let Some(sequence_index) = self.active_sequence_indices[node.0 as usize] {
                return self.sequences[sequence_index].vessel_tala_id;
            }
            return self.nodes[node.0 as usize].tala_id;
        }
        self.active_node_tala_id_slow(node)
    }

    #[cold]
    #[inline(never)]
    fn active_node_tala_id_slow(&self, node: NodeId) -> i64 {
        if let Some(cluster_index) = self.active_cluster_index(node) {
            return self.clusters[cluster_index].vessel_tala_id;
        }
        if let Some(sequence_index) = self.active_sequence_index(node) {
            return self.sequences[sequence_index].vessel_tala_id;
        }
        self.nodes[node.0 as usize].tala_id
    }

    #[inline(always)]
    pub(super) fn active_node_is_aggregate(&self, node: NodeId) -> bool {
        if self.node_order_membership_valid {
            let index = node.0 as usize;
            return self.active_cluster_indices[index].is_some()
                || self.active_sequence_indices[index].is_some();
        }
        self.active_cluster_index(node).is_some() || self.active_sequence_index(node).is_some()
    }

    #[inline(always)]
    pub(super) fn active_adjacent(&self, node: NodeId, edge: EdgeId) -> NodeId {
        let edge = &self.edge_records[edge.0 as usize];
        self.active_adjacent_for_edge(node, edge)
    }

    #[inline(always)]
    pub(super) fn active_adjacent_for_edge(&self, node: NodeId, edge: &ArenaEdge) -> NodeId {
        let from = self.active_aggregate_owner(edge.from);
        // node.go adjacent reads To only when From is the receiver. Resolve
        // the same branch after mapping the source to its current vessel.
        if from == node {
            self.active_aggregate_owner(edge.to)
        } else {
            from
        }
    }

    /// Returns the stable endpoints represented by the receiver and adjacent
    /// sides of an edge whose current endpoints may be aggregate vessels.
    pub(super) fn active_edge_original_endpoints(
        &self,
        node: NodeId,
        edge: EdgeId,
    ) -> (NodeId, NodeId) {
        let edge = &self.edge_records[edge.0 as usize];
        if self.active_aggregate_owner(edge.from) == node {
            (edge.from, edge.to)
        } else {
            (edge.to, edge.from)
        }
    }

    /// Reconstructs the current `Node.Edges` slice after Sequence.abductEdges
    /// and Cluster.AbductEdges have reconnected stable input edges to their
    /// temporary vessels.
    pub(super) fn active_edge_ids(&self, node: NodeId) -> ActiveEdgeIds<'_> {
        if let Some(cluster_index) = self.active_cluster_index(node)
            && self.clusters[cluster_index].members.first() == Some(&node)
        {
            let cluster = &self.clusters[cluster_index];
            let bound_vessel = self.topology_vessel_node(TopologyVessel::Cluster(cluster_index));
            let mut edges = Vec::new();
            for (edge_index, edge) in self.member_edge_entries() {
                let from_sequence = self.active_sequence_index(edge.from);
                if from_sequence.is_some() && from_sequence == self.active_sequence_index(edge.to) {
                    continue;
                }
                let from = self.active_sequence_owner(edge.from);
                let to = self.active_sequence_owner(edge.to);
                if cluster.members.contains(&from) || bound_vessel == Some(from) {
                    edges.push(EdgeId(edge_index as u32));
                }
                if cluster.members.contains(&to) || bound_vessel == Some(to) {
                    // Cluster.AbductEdges has two independent endpoint tests;
                    // an internal cluster edge is therefore appended twice to
                    // the vessel's Node.Edges slice.
                    edges.push(EdgeId(edge_index as u32));
                }
            }
            return ActiveEdgeIds::Owned(edges);
        }
        if let Some(sequence_index) = self.active_sequence_index(node)
            && self.sequences[sequence_index].members.first() == Some(&node)
            && self.active_cluster_owner(node) == node
        {
            let sequence = &self.sequences[sequence_index];
            if let Some(vessel) =
                self.topology_vessel_node(TopologyVessel::Sequence(sequence_index))
            {
                return ActiveEdgeIds::Owned(
                    self.nodes[vessel.0 as usize]
                        .edges
                        .read()
                        .into_iter()
                        .filter(|edge| self.edge_order.contains(edge))
                        .collect(),
                );
            }
            return ActiveEdgeIds::Owned(
                self.member_edge_entries()
                    .filter_map(|(edge_index, edge)| {
                        let from_is_step = sequence.members.contains(&edge.from);
                        let to_is_step = sequence.members.contains(&edge.to);
                        (from_is_step ^ to_is_step).then_some(EdgeId(edge_index as u32))
                    })
                    .collect(),
            );
        }
        // Ordinary nodes keep the pointer-visible incidence order from the
        // graph, but ExtractTrees removes peeled sentinel edges from the
        // active Graph.Edges/edge order while the stable arena retains the
        // original Node.Edges slice for restoration. Filter only those
        // inactive edges here; preserving the remaining slice order matches
        // Go's in-place removal without perturbing parallel-edge ordering.
        let edges = &self.nodes[node.0 as usize].edges;
        // Read the live incidence slice once and filter its transient image
        // in place. No context callbacks run here; a second allocation/read
        // cannot contribute another source-visible range operation.
        let mut active = edges.read();
        self.edge_order.retain_members(&mut active);
        ActiveEdgeIds::Owned(active)
    }

    /// Returns the current `Node.Edges` incidence view without filtering it
    /// through `Graph.Edges`. Go's `Graph.maxEdgeLength` walks each live node's
    /// edge slice directly, so edges omitted from the graph-wide edge slice
    /// still affect this cached placement cost.
    pub(super) fn max_edge_length_incident_edges(&self, node: NodeId) -> ActiveEdgeIds<'_> {
        if let Some(cluster_index) = self.active_cluster_index(node)
            && self.clusters[cluster_index].members.first() == Some(&node)
        {
            let cluster = &self.clusters[cluster_index];
            let bound_vessel = self.topology_vessel_node(TopologyVessel::Cluster(cluster_index));
            let mut edges = Vec::new();
            for (edge_index, edge) in self.member_edge_entries() {
                let from_sequence = self.active_sequence_index(edge.from);
                if from_sequence.is_some() && from_sequence == self.active_sequence_index(edge.to) {
                    continue;
                }
                let from = self.active_sequence_owner(edge.from);
                let to = self.active_sequence_owner(edge.to);
                if cluster.members.contains(&from) || bound_vessel == Some(from) {
                    edges.push(EdgeId(edge_index as u32));
                }
                if cluster.members.contains(&to) || bound_vessel == Some(to) {
                    // Cluster.AbductEdges performs two independent endpoint
                    // checks, retaining an internal edge twice on the vessel.
                    edges.push(EdgeId(edge_index as u32));
                }
            }
            return ActiveEdgeIds::Owned(edges);
        }
        if let Some(sequence_index) = self.active_sequence_index(node)
            && self.sequences[sequence_index].members.first() == Some(&node)
            && self.active_cluster_owner(node) == node
        {
            let sequence = &self.sequences[sequence_index];
            if let Some(vessel) =
                self.topology_vessel_node(TopologyVessel::Sequence(sequence_index))
            {
                return ActiveEdgeIds::Owned(self.nodes[vessel.0 as usize].edges.read());
            }
            return ActiveEdgeIds::Owned(
                self.member_edge_entries()
                    .filter_map(|(edge_index, edge)| {
                        let from_is_step = sequence.members.contains(&edge.from);
                        let to_is_step = sequence.members.contains(&edge.to);
                        (from_is_step ^ to_is_step).then_some(EdgeId(edge_index as u32))
                    })
                    .collect(),
            );
        }
        ActiveEdgeIds::Owned(self.nodes[node.0 as usize].edges.read())
    }

    /// Length-only view of active_edge_ids. Aggregate reconstruction can scan
    /// the graph without allocating the observable ordered edge slice.
    pub(super) fn active_edge_count(&self, node: NodeId) -> usize {
        if let Some(cluster_index) = self.active_cluster_index(node)
            && self.clusters[cluster_index].members.first() == Some(&node)
        {
            let cluster = &self.clusters[cluster_index];
            let bound_vessel = self.topology_vessel_node(TopologyVessel::Cluster(cluster_index));
            return self
                .member_edge_entries()
                .filter(|(_, edge)| {
                    let from_sequence = self.active_sequence_index(edge.from);
                    from_sequence.is_none() || from_sequence != self.active_sequence_index(edge.to)
                })
                .map(|(_, edge)| {
                    let from = self.active_sequence_owner(edge.from);
                    let to = self.active_sequence_owner(edge.to);
                    usize::from(cluster.members.contains(&from) || bound_vessel == Some(from))
                        + usize::from(cluster.members.contains(&to) || bound_vessel == Some(to))
                })
                .sum();
        }
        if let Some(sequence_index) = self.active_sequence_index(node)
            && self.sequences[sequence_index].members.first() == Some(&node)
            && self.active_cluster_owner(node) == node
        {
            let sequence = &self.sequences[sequence_index];
            if let Some(vessel) =
                self.topology_vessel_node(TopologyVessel::Sequence(sequence_index))
            {
                return self.nodes[vessel.0 as usize]
                    .edges
                    .read()
                    .iter()
                    .filter(|edge| self.edge_order.contains(edge))
                    .count();
            }
            return self
                .member_edge_entries()
                .filter(|(_, edge)| {
                    sequence.members.contains(&edge.from) ^ sequence.members.contains(&edge.to)
                })
                .count();
        }
        let edges = &self.nodes[node.0 as usize].edges;
        if edges
            .read()
            .iter()
            .all(|edge| self.edge_order.contains(edge))
        {
            edges.len()
        } else {
            edges
                .read()
                .iter()
                .filter(|edge| self.edge_order.contains(edge))
                .count()
        }
    }

    /// EdgeLength subtracts symmetry using the live len(Node.Edges), after
    /// nodeSymmetry has returned. Ordinary nodes need no graph-membership scan.
    fn scoring_edge_count(&self, node: NodeId) -> usize {
        if self.active_cluster_index(node).is_none() && self.active_sequence_index(node).is_none() {
            self.nodes[node.0 as usize].edges.len()
        } else {
            self.max_edge_length_incident_edges(node).len()
        }
    }

    /// Returns the edge order used by TALA's pointer-visible Node.Edges slice
    /// for score accumulation. The ordinary active-edge view is intentionally
    /// kept for routing and graph scans; changing that view globally perturbs
    /// equal-cost route construction. Race scoring, however, folds the edge
    /// terms in Node.Edges order, and that fold is observable at strict ties.
    pub(super) fn scoring_edge_ids(&self, node: NodeId) -> ActiveEdgeIds<'_> {
        // NodeEdgeLength consumes Node.Edges even when Graph.Edges omits an
        // incident pointer. Routing eligibility is a separate graph view.
        let edges = self.max_edge_length_incident_edges(node);
        let order = match self.incident_edge_order.get(&node) {
            Some(order) => Cow::Borrowed(order.as_slice()),
            // max_edge_length_incident_edges already copied Node.Edges for
            // an ordinary node. Without an auxiliary order there is nothing
            // to reorder, and no reason to copy that same slice again.
            None if self.active_cluster_index(node).is_none()
                && self.active_sequence_index(node).is_none() =>
            {
                return edges;
            }
            // The placement-scope order map is auxiliary and is not retained
            // by every ArenaGraph clone. Node.Edges itself is the authoritative
            // pointer-visible order after reconnecting projected endpoints;
            // falling back to Graph.Edges reorders parallel abductions and
            // changes their arrow-direction costs.
            None => Cow::Owned(self.nodes[node.0 as usize].edges.read()),
        };
        // edge_length.go walks Node.Edges directly. Ordinary incidence and
        // the retained order usually agree; avoid searching every edge's rank
        // in the same slice before returning that unchanged source order.
        if order.as_ref() == &*edges {
            return edges;
        }
        // Node degree is normally tiny relative to Graph.Edges. Recover each
        // edge's first pointer-slice rank directly instead of allocating and
        // filling a map over the complete graph for every node score.
        let rank = |edge_id: &EdgeId| {
            order
                .iter()
                .position(|candidate| candidate == edge_id)
                .unwrap_or(usize::MAX)
        };
        let mut previous_rank = None;
        let already_ordered = edges.iter().all(|edge| {
            let current_rank = rank(edge);
            let ordered = previous_rank.is_none_or(|previous| previous <= current_rank);
            previous_rank = Some(current_rank);
            ordered
        });
        if already_ordered {
            return edges;
        }
        let mut edges = match edges {
            ActiveEdgeIds::Borrowed(edges) => edges.to_vec(),
            ActiveEdgeIds::Owned(edges) => edges,
        };
        edges.sort_by_key(rank);
        ActiveEdgeIds::Owned(edges)
    }

    #[inline(always)]
    pub(super) fn active_node_size(&self, node: NodeId) -> Size {
        if self.node_order_membership_valid {
            let index = node.0 as usize;
            if let Some(cluster_index) = self.active_cluster_indices[index]
                && self.clusters[cluster_index].members.first() == Some(&node)
            {
                return self.cluster_vessel_size(cluster_index);
            }
            if let Some(sequence_index) = self.active_sequence_indices[index]
                && self.sequences[sequence_index].members.first() == Some(&node)
                && self.active_cluster_owners[index] == node
            {
                return self.sequence_vessel_size(sequence_index);
            }
            return self.nodes[index].rect.size;
        }
        let node_ref = &self.nodes[node.0 as usize];
        if self.cluster_backlinks_complete
            && self.sequence_backlinks_complete
            && node_ref.cluster.is_none()
            && node_ref.sequence.is_none()
        {
            return node_ref.rect.size;
        }
        self.active_node_size_slow(node)
    }

    #[cold]
    #[inline(never)]
    fn active_node_size_slow(&self, node: NodeId) -> Size {
        if let Some(cluster_index) = self.nodes[node.0 as usize].cluster {
            let cluster = &self.clusters[cluster_index];
            let active = if self.node_order_membership_valid {
                self.active_cluster_flags
                    .get(cluster_index)
                    .copied()
                    .unwrap_or(false)
            } else {
                self.cluster_is_active(cluster)
            };
            if cluster.members.first() == Some(&node) && active {
                return self.cluster_vessel_size(cluster_index);
            }
        } else if !self.cluster_backlinks_complete
            && let Some(cluster_index) = self.clusters.iter().position(|cluster| {
                self.cluster_is_active(cluster) && cluster.members.first() == Some(&node)
            })
        {
            return self.cluster_vessel_size(cluster_index);
        }
        if let Some(sequence_index) = self.nodes[node.0 as usize].sequence {
            let sequence = &self.sequences[sequence_index];
            let active = if self.node_order_membership_valid {
                self.active_sequence_flags
                    .get(sequence_index)
                    .copied()
                    .unwrap_or(false)
            } else {
                self.sequence_is_active(sequence)
            };
            if sequence.members.first() == Some(&node)
                && active
                && self.active_cluster_owner(node) == node
            {
                return self.sequence_vessel_size(sequence_index);
            }
        } else if !self.sequence_backlinks_complete
            && let Some(sequence_index) = self.sequences.iter().position(|sequence| {
                self.sequence_is_active(sequence)
                    && sequence.members.first() == Some(&node)
                    && self.active_cluster_owner(node) == node
            })
        {
            return self.sequence_vessel_size(sequence_index);
        }
        self.nodes[node.0 as usize].rect.size
    }

    #[inline(always)]
    pub(super) fn active_node_position(&self, node: NodeId) -> Option<Point> {
        if let Some(index) = self.nodes[node.0 as usize].sequence {
            if self.sequences[index].members.first() == Some(&node) {
                if let Some(vessel) = self.topology_vessel_node(TopologyVessel::Sequence(index)) {
                    if vessel != node && self.node_order_contains(vessel) {
                        return self.position(vessel);
                    }
                }
            }
        }
        if self.node_order_membership_valid {
            let index = node.0 as usize;
            if let Some(cluster_index) = self.active_cluster_indices[index]
                && self.clusters[cluster_index].members.first() == Some(&node)
            {
                return self
                    .pending_cluster_vessel_positions
                    .get(&cluster_index)
                    .copied()
                    .or(self.nodes[index].position);
            }
            if let Some(sequence_index) = self.active_sequence_indices[index]
                && self.sequences[sequence_index].members.first() == Some(&node)
                && self.active_cluster_owners[index] == node
            {
                return self
                    .serialized_vessel_positions_by_tala
                    .get(&self.sequences[sequence_index].vessel_tala_id)
                    .copied()
                    .or(self.nodes[index].position);
            }
            return self.position(node);
        }
        let node_ref = &self.nodes[node.0 as usize];
        if self.cluster_backlinks_complete
            && self.sequence_backlinks_complete
            && node_ref.cluster.is_none()
            && node_ref.sequence.is_none()
        {
            return self.position(node);
        }
        self.active_node_position_slow(node)
    }

    #[cold]
    #[inline(never)]
    fn active_node_position_slow(&self, node: NodeId) -> Option<Point> {
        if let Some(cluster_index) = self.nodes[node.0 as usize].cluster {
            let cluster = &self.clusters[cluster_index];
            let active = if self.node_order_membership_valid {
                self.active_cluster_flags
                    .get(cluster_index)
                    .copied()
                    .unwrap_or(false)
            } else {
                self.cluster_is_active(cluster)
            };
            if cluster.members.first() == Some(&node) && active {
                return self
                    .pending_cluster_vessel_positions
                    .get(&cluster_index)
                    .copied()
                    .or_else(|| self.position(node));
            }
        }
        if let Some(sequence_index) = self.nodes[node.0 as usize].sequence {
            let sequence = &self.sequences[sequence_index];
            let active = if self.node_order_membership_valid {
                self.active_sequence_flags
                    .get(sequence_index)
                    .copied()
                    .unwrap_or(false)
            } else {
                self.sequence_is_active(sequence)
            };
            if sequence.members.first() == Some(&node)
                && active
                && self.active_cluster_owner(node) == node
            {
                return self
                    .serialized_vessel_positions_by_tala
                    .get(&sequence.vessel_tala_id)
                    .copied()
                    .or_else(|| self.position(node));
            }
        }
        self.position(node)
    }

    pub(super) fn active_node_center(&self, node: NodeId) -> Point {
        let position = self
            .active_node_position(node)
            .expect("positioned active node");
        let size = self.active_node_size(node);
        Point {
            x: position.x + size.width * 0.5,
            y: position.y + size.height * 0.5,
        }
    }

    #[track_caller]
    pub(super) fn move_active_node_abs_with_children(&mut self, node: NodeId, target: Point) {
        let trace_focus6 = crate::internal::trace_env_enabled("TALA_TRACE_FOCUS6_MOVE");
        let focus6_before = trace_focus6
            .then(|| {
                self.nodes
                    .iter()
                    .find(|candidate| candidate.tala_id == 856466825)
                    .and_then(|candidate| candidate.position)
            })
            .flatten();
        let trace_focus_move = self.nodes[node.0 as usize].tala_id == 1051177149
            && target.x == 300.0
            && target.y == -1425.0;
        if trace_focus_move {
            self.trace_target_shared_geometry("move-focus-before");
        }
        if crate::internal::trace_env_value("TALA_TRACE_ACTIVE_MOVE_NODE").is_some_and(
            |target_id| {
                target_id == "all"
                    || target_id.parse::<i64>().ok() == Some(self.nodes[node.0 as usize].tala_id)
            },
        ) {
            eprintln!(
                "ACTIVE_MOVE_RUST caller={} node={} aggregate={} current={:?} target={},{} leaves={:?}",
                std::panic::Location::caller(),
                self.nodes[node.0 as usize].tala_id,
                self.active_node_is_aggregate(node),
                self.active_node_position(node),
                target.x,
                target.y,
                self.active_aggregate_leaf_members(node)
                    .map(|members| members
                        .iter()
                        .map(|member| self.nodes[member.0 as usize].tala_id)
                        .collect::<Vec<_>>()),
            );
        }
        if crate::internal::trace_env_enabled("TALA_TRACE_CALLER_3447521236")
            && self.nodes[node.0 as usize].tala_id == 3447521236
            && target.y == self.active_node_position(node).unwrap_or_default().y - 1.0
        {
            eprintln!(
                "CALLER_MOVE_ACTIVE_RUST caller={} target={:?} current={:?}",
                std::panic::Location::caller(),
                target,
                self.active_node_position(node)
            );
        }
        let Some(current) = self.active_node_position(node) else {
            self.set_position(node, target);
            return;
        };
        let delta = Point {
            x: target.x - current.x,
            y: target.y - current.y,
        };
        if self.active_node_is_aggregate(node) {
            // A temporary Go vessel has no member children in its graph
            // container. Move the vessel frame here; SyncClusters will later
            // arrange the retained member boxes and move their descendants.
            self.translate_active_node_with_children(node, delta);
        } else {
            self.translate_node_with_children(node, delta);
        }
        if trace_focus6 {
            let focus6_after = self
                .nodes
                .iter()
                .find(|candidate| candidate.tala_id == 856466825)
                .and_then(|candidate| candidate.position);
            if focus6_after != focus6_before {
                eprintln!(
                    "FOCUS6_CHANGED_RUST caller={} receiver={} delta={},{} before={:?} after={:?}",
                    std::panic::Location::caller(),
                    self.nodes[node.0 as usize].tala_id,
                    delta.x,
                    delta.y,
                    focus6_before,
                    focus6_after,
                );
            }
        }
        if trace_focus_move {
            self.trace_target_shared_geometry("move-focus-after");
        }
    }

    /// Stable-arena representation of TALA `Node.translate` while aggregate
    /// vessels are active. An ordinary current node owns one box. A temporary
    /// bound cluster vessel owns its actual Point as well as a cached box;
    /// unbound aggregate projections retain their member-box representation.
    pub(super) fn translate_active_node_box(&mut self, node: NodeId, delta: Point) {
        self.translate_active_node_boxes(&[node], delta);
    }

    /// Publishes one `Transaction.MoveNodes` operation to retained arena
    /// boxes. Distinct current TALA nodes can map to overlapping stable Rust
    /// boxes (a cluster vessel, its nested sequence vessel, and a member may
    /// all occur in the same connected-node slice), but one batch translation
    /// changes each absolute box exactly once.
    #[track_caller]
    pub(super) fn translate_active_node_boxes(&mut self, nodes: &[NodeId], delta: Point) {
        let trace_target = crate::internal::trace_env_value("TALA_TRACE_MOVE_TARGET");
        if let Some(target) = trace_target.as_deref()
            && (target == "all"
                || nodes.iter().any(|node| {
                    self.nodes[node.0 as usize].tala_id == target.parse::<i64>().unwrap_or(-1)
                }))
        {
            eprintln!(
                "ACTIVE_BOXES_RUST caller={} nodes={:?} delta={},{}",
                std::panic::Location::caller(),
                nodes
                    .iter()
                    .map(|node| self.nodes[node.0 as usize].tala_id)
                    .collect::<Vec<_>>(),
                delta.x,
                delta.y
            );
        }
        // Sorted and deduplicated before use: the ascending node set.
        let mut moved = Vec::new();
        let mut moved_clusters = BTreeSet::new();
        let mut moved_vessels = BTreeSet::new();
        for node in nodes.iter().copied() {
            if let Some(cluster_index) = self.active_cluster_index(node)
                && (self.clusters[cluster_index].members.first()
                    == Some(&self.active_sequence_owner(node))
                    || self.topology_vessel_node(TopologyVessel::Cluster(cluster_index))
                        == Some(node))
                && self.cluster_is_active(&self.clusters[cluster_index])
            {
                if self
                    .pending_cluster_vessel_positions
                    .contains_key(&cluster_index)
                {
                    moved_clusters.insert(cluster_index);
                    // Source Node.Translate mutates Cluster.Vessel.TopLeft.
                    // The pending box is a numerical view of that same node;
                    // updating only it leaves actual field readers stale.
                    if let Some(vessel) =
                        self.topology_vessel_node(TopologyVessel::Cluster(cluster_index))
                        && self.topology_node(vessel).is_some()
                    {
                        moved.push(vessel);
                        moved_vessels.insert(vessel);
                    }
                } else if let Some(members) = self.active_aggregate_leaf_members(node) {
                    for member in members {
                        moved.push(member);
                        moved.extend(self.descendants(member));
                    }
                }
                continue;
            } else if let Some(sequence_index) = self.active_sequence_index(node)
                && self.sequences[sequence_index].members.first() == Some(&node)
                && self.sequence_is_active(&self.sequences[sequence_index])
            {
                if let Some(members) = self.active_aggregate_leaf_members(node) {
                    for member in members {
                        moved.push(member);
                        moved.extend(self.descendants(member));
                    }
                }
                continue;
            }
            moved.push(node);
        }
        moved.sort_unstable();
        moved.dedup();
        if let Some(target) = trace_target
            .as_deref()
            .and_then(|value| value.parse::<i64>().ok())
            && (target == -1
                || moved
                    .iter()
                    .any(|node| self.nodes[node.0 as usize].tala_id == target))
        {
            let target_nodes = moved
                .iter()
                .filter(|node| self.nodes[node.0 as usize].tala_id == target)
                .map(|node| self.nodes[node.0 as usize].tala_id)
                .collect::<Vec<_>>();
            eprintln!(
                "ACTIVE_BOXES_MOVED_RUST caller={} target={} delta={},{} moved_target={:?} moved_clusters={:?}",
                std::panic::Location::caller(),
                target,
                delta.x,
                delta.y,
                target_nodes,
                moved_clusters
            );
        }
        for node in moved.iter().copied() {
            if self.position(node).is_some() {
                if moved_vessels.contains(&node) {
                    self.translate_owner_point(node, delta.x, delta.y);
                    continue;
                }
                // Node.translate updates the existing TopLeft allocation.
                self.translate_owner_point(node, delta.x, delta.y);
                self.nodes[node.0 as usize].unpositioned_scope_translation = None;
            }
        }
        for (&cluster_index, position) in &mut self.pending_cluster_vessel_positions {
            if moved_clusters.contains(&cluster_index) {
                position.x += delta.x;
                position.y += delta.y;
            }
        }
    }

    /// Stable-arena representation of `Node.moveNodeWithChildren` for a
    /// current graph node. Recovered `getAllDescendantNodes(node, true)` walks
    /// every member below a cluster/sequence vessel as well as ordinary
    /// container descendants. A pending cluster vessel is a separate Go box,
    /// so move that projection and its retained member boxes exactly once.
    #[track_caller]
    pub(super) fn translate_active_node_with_children(&mut self, node: NodeId, delta: Point) {
        let trace_target = crate::internal::trace_env_value("TALA_TRACE_MOVE_TARGET");
        if let Some(target) = trace_target.as_deref()
            && (target == "all"
                || self.nodes[node.0 as usize].tala_id == target.parse::<i64>().unwrap_or(-1))
        {
            eprintln!(
                "ACTIVE_WITH_CHILDREN_RUST caller={} node={} delta={},{} aggregate={} pending={:?}",
                std::panic::Location::caller(),
                self.nodes[node.0 as usize].tala_id,
                delta.x,
                delta.y,
                self.active_node_is_aggregate(node),
                self.pending_cluster_vessel_positions
            );
        }
        if let Some(cluster_index) = self.active_cluster_index(node)
            && self.clusters[cluster_index].members.first()
                == Some(&self.active_sequence_owner(node))
            && self.cluster_is_active(&self.clusters[cluster_index])
            && self
                .pending_cluster_vessel_positions
                .contains_key(&cluster_index)
        {
            // A Go cluster vessel is a separate temporary node. Its members
            // have been removed from Graph.Containers, but
            // getAllDescendantNodes(vessel, true) still walks those retained
            // member pointers and their ordinary container descendants.
            // Translate the stable carrier, members, and descendants together;
            // Cluster.SyncGeometry then aligns each member frame to the vessel.
            self.translate_active_node_box(node, delta);
            // The vessel's descendant walk still includes each retained
            // member (and that member's ordinary container descendants). The
            // stable carrier therefore has to translate the leaf boxes along
            // with the pending vessel; SyncGeometry will subsequently align
            // their frames to the vessel origin.
            let mut moved = BTreeSet::new();
            if let Some(members) = self.active_aggregate_leaf_members(node) {
                for member in members {
                    moved.insert(member);
                    moved.extend(self.descendants(member));
                }
            }
            for member in moved {
                if self.position(member).is_some() {
                    self.translate_owner_point(member, delta.x, delta.y);
                    self.nodes[member.0 as usize].unpositioned_scope_translation = None;
                }
            }
            return;
        }
        if self.active_node_is_aggregate(node) {
            self.translate_active_node_box(node, delta);
            return;
        }
        self.translate_node_with_children(node, delta);
    }

    pub(super) fn swap_positions(&mut self, a: NodeId, b: NodeId) {
        let (Some(a_position), Some(b_position)) =
            (self.active_node_position(a), self.active_node_position(b))
        else {
            return;
        };
        self.move_active_node_abs_with_children(a, b_position);
        self.move_active_node_abs_with_children(b, a_position);
    }

    pub(super) fn smart_swap_positions(&mut self, a: NodeId, b: NodeId) {
        let (Some(a_position), Some(b_position)) =
            (self.active_node_position(a), self.active_node_position(b))
        else {
            return;
        };
        let a_size = self.active_node_size(a);
        let b_size = self.active_node_size(b);
        let orientation = self.sized_box_orientation((a_position, a_size), (b_position, b_size));
        if crate::internal::trace_env_value("TALA_TRACE_SWAP_ORIENTATION").is_some_and(|target| {
            target == "all"
                || target == self.nodes[a.0 as usize].tala_id.to_string()
                || target == self.nodes[b.0 as usize].tala_id.to_string()
        }) {
            eprintln!(
                "SWAP_ORIENTATION_RUST a={} b={} orientation={:?} a_pos={:?} a_size={:?} b_pos={:?} b_size={:?}",
                self.nodes[a.0 as usize].tala_id,
                self.nodes[b.0 as usize].tala_id,
                orientation,
                a_position,
                a_size,
                b_position,
                b_size,
            );
        }
        let (new_a, new_b) = match orientation {
            Orientation::Left => (
                Point {
                    x: b_position.x + b_size.width - a_size.width,
                    y: a_position.y,
                },
                Point {
                    x: a_position.x,
                    y: b_position.y,
                },
            ),
            Orientation::Right => (
                Point {
                    x: b_position.x,
                    y: a_position.y,
                },
                Point {
                    x: a_position.x + a_size.width - b_size.width,
                    y: b_position.y,
                },
            ),
            Orientation::Top => (
                Point {
                    x: a_position.x,
                    y: b_position.y + b_size.height - a_size.height,
                },
                Point {
                    x: b_position.x,
                    y: a_position.y,
                },
            ),
            Orientation::Bottom => (
                Point {
                    x: a_position.x,
                    y: b_position.y,
                },
                Point {
                    x: b_position.x,
                    y: a_position.y + a_size.height - b_size.height,
                },
            ),
            Orientation::TopLeft => (
                Point {
                    x: b_position.x + b_size.width - a_size.width,
                    y: b_position.y + b_size.height - a_size.height,
                },
                a_position,
            ),
            Orientation::TopRight => (
                Point {
                    x: b_position.x,
                    y: b_position.y + b_size.height - a_size.height,
                },
                Point {
                    x: a_position.x + a_size.width - b_size.width,
                    y: a_position.y,
                },
            ),
            Orientation::BottomLeft => (
                Point {
                    x: b_position.x + b_size.width - a_size.width,
                    y: b_position.y,
                },
                Point {
                    x: a_position.x,
                    y: a_position.y + a_size.height - b_size.height,
                },
            ),
            Orientation::BottomRight => (
                b_position,
                Point {
                    x: a_position.x + a_size.width - b_size.width,
                    y: a_position.y + a_size.height - b_size.height,
                },
            ),
            Orientation::None => return,
        };
        self.move_active_node_abs_with_children(a, new_a);
        self.move_active_node_abs_with_children(b, new_b);
    }

    pub(super) fn has_node_overlaps(&self) -> bool {
        self.has_node_overlaps_except(&BTreeSet::new())
    }

    /// `Node.getDeltaTo` while the transaction graph contains temporary
    /// aggregate vessels. Stable members retain the physical input edges, so
    /// recover the vessel's current edge slice before deciding whether the
    /// connected-node 60-unit floor applies.
    fn transaction_spacing_delta(&self, node: NodeId, other: NodeId, proposed: Point) -> f64 {
        // spacing_delta_with_loops already resolves the active aggregate
        // endpoint and scans the active vessel edge inventory.  A connected
        // pair initializes both axis clearances to 60, so every orientation
        // returns at least that floor.  Repeating active_edge_ids here was an
        // identical second connectivity scan for every transaction pair.
        self.spacing_delta_with_loops(node, other, proposed)
    }

    pub(super) fn projected_transaction_node_from_current(
        &self,
        node_id: NodeId,
    ) -> ProjectedTransactionNode {
        let mut node = self.nodes[node_id.0 as usize].clone();
        let active_position = self.active_node_position(node_id);
        let active_size = self.active_node_size(node_id);
        let container = self.active_node_container(node_id);
        if self.active_node_is_aggregate(node_id) && self.active_aggregate_owner(node_id) == node_id
        {
            node.tala_id = self.active_node_tala_id(node_id);
            node.declared_size = None;
            node.layout_margins = Insets::uniform(0.0);
            node.unpositioned_scope_translation = None;
            node.scope_translation_materialized = false;
            node.loop_offsets = None;
            node.herd_assignment = None;
            node.fixed_top_left = None;
            node.force_hierarchy = false;
            node.hierarchy = None;
            node.desired_width = None;
            node.desired_height = None;
            node.folded_label_min_size = None;
            node.shape = ShapeKind::Rectangle;
            node.label_size = None;
            node.font_size = None;
            node.label_position = LabelPosition::Unset;
            node.label_position_fixed = false;
            node.external_label = None;
            node.icon_position = None;
            node.has_icon = false;
            node.is_invisible = false;
            node.is_container = false;
            node.scoring_is_container = false;
            node.scoring_is_aggregate_vessel = true;
            node.scoring_cluster_vessel = None;
            node.sequence = None;
            node.cluster = None;
            node.transaction_anchor_tala_id = None;
            node.transaction_anchor_offset = None;
            node.transaction_position_was_nil = active_position.is_none();
            node.content_insets = Insets::uniform(0.0);
            node.node_padding = Insets::uniform(0.0);
            node.grid_rows = None;
            node.grid_columns = None;
            node.canvas_position = None;
            node.label_aware_grid = false;
            node.packed_grid = false;
            node.is_3d = false;
            node.is_multiple = false;
            node.table_column_count = None;
            node.edges.clear();
            node.nears.clear();
        }
        node.position = active_position;
        node.rect.origin = active_position.unwrap_or_default();
        node.rect.size = active_size;
        node.container = container;
        node.scoring_container_parent =
            container.map(|container| self.active_node_tala_id(container));
        node.scoring_container_ancestors.clear();
        let mut current = container;
        while let Some(ancestor) = current {
            node.scoring_container_ancestors
                .push(self.active_node_tala_id(ancestor));
            current = self.active_node_container(ancestor);
        }
        let child_tala_ids = if node.is_container {
            let mut seen = BTreeSet::new();
            self.container_node_order(Some(node_id))
                .into_iter()
                .map(|child| self.active_aggregate_owner(child))
                .filter(|child| seen.insert(*child))
                .map(|child| self.active_node_tala_id(child))
                .collect()
        } else {
            Vec::new()
        };
        let edges = self
            .active_edge_ids(node_id)
            .into_iter()
            .map(|edge_id| {
                let edge = &self.edge_records[edge_id.0 as usize];
                ProjectedTransactionEdge {
                    adjacent_tala_id: self
                        .active_node_tala_id(self.active_adjacent(node_id, edge_id)),
                    min_width: edge.min_width,
                    min_height: edge.min_height,
                }
            })
            .collect();
        let structural_container_tala_id =
            node.container.map(|id| self.nodes[id.0 as usize].tala_id);
        let mut structural_ancestor_tala_ids = Vec::new();
        // isDescendantOf follows the retained Container pointer and then
        // its Cluster.Vessel backlink. Both are ancestors even when they
        // share an arena slot; omitting the vessel rejects its own children
        // as new overlaps after SyncClusters changes the arrangement.
        let mut structural_parent = self.connected_parent(ConnectedNode::Active(node_id));
        while let Some(parent) = structural_parent {
            structural_ancestor_tala_ids.push(self.connected_node_id(parent));
            structural_parent = self.connected_parent(parent);
        }
        ProjectedTransactionNode {
            structural_container_tala_id,
            structural_ancestor_tala_ids,
            container_tala_id: node.scoring_container_parent,
            ancestor_tala_ids: node.scoring_container_ancestors.clone(),
            child_tala_ids,
            edges,
            node,
        }
    }

    pub(super) fn initialize_projected_transaction_nodes_from_current_graph(&mut self) {
        self.projected_transaction_nodes = self
            .graph_node_order()
            .into_iter()
            .map(|node| self.projected_transaction_node_from_current(node))
            .collect();
    }

    /// PlaceTrees appends retained nodes to the owning Graph.Nodes slice after
    /// each split component. Preserve that membership/order growth before a
    /// later component opens its next transaction.
    pub(super) fn append_restored_projected_transaction_nodes(&mut self) {
        let current = self
            .graph_node_order()
            .into_iter()
            .map(|node| self.projected_transaction_node_from_current(node))
            .collect::<Vec<_>>();
        for current in current {
            if let Some(existing) = self
                .projected_transaction_nodes
                .iter_mut()
                .find(|entry| entry.node.tala_id == current.node.tala_id)
            {
                // ReconnectTree changes existing endpoint edge slices, while
                // PlaceTree can move the surviving root as well as newly
                // restored descendants. Refresh every observable field but
                // retain the owning Graph.Nodes vector position.
                *existing = current;
            } else {
                self.projected_transaction_nodes.push(current);
            }
        }
    }

    fn projected_transaction_geometry(
        &self,
        entry: &ProjectedTransactionNode,
    ) -> (Option<Point>, Size) {
        let tala_id = entry.node.tala_id;
        if let Some(node) = self
            .graph_node_order()
            .into_iter()
            .find(|node| self.active_node_tala_id(*node) == tala_id)
        {
            return (self.active_node_position(node), self.active_node_size(node));
        }
        if let Some(node) = self
            .transaction_external_container_children
            .values()
            .flatten()
            .find(|node| node.tala_id == tala_id)
        {
            return (self.anchored_external_position(node), node.rect.size);
        }
        if let Some(node) = self
            .transaction_external_aggregate_children
            .values()
            .flatten()
            .find(|node| node.tala_id == tala_id)
        {
            return (self.anchored_external_position(node), node.rect.size);
        }
        if let Some(node) = self
            .transaction_external_containers
            .iter()
            .find(|node| node.tala_id == tala_id)
        {
            return (self.anchored_external_position(node), node.rect.size);
        }
        (
            self.anchored_external_position(&entry.node),
            entry.node.rect.size,
        )
    }

    /// Resolve all live projected boxes once for a transaction validation
    /// pass.  The transaction carrier is immutable while `Commit` validates
    /// it, so repeatedly rebuilding `graph_node_order` and rescanning every
    /// external alias for each node/pair is semantically redundant (and was
    /// enough to push large nesting cases past the release timeout).
    pub(super) fn projected_transaction_live_geometry(
        &self,
    ) -> IdHashMap<i64, (Option<Point>, Size)> {
        // Lookup-only (entry/get), so a hash map preserves every result.
        let mut geometry = IdHashMap::default();
        for node in self.graph_node_order() {
            geometry
                .entry(self.active_node_tala_id(node))
                .or_insert((self.active_node_position(node), self.active_node_size(node)));
        }
        for node in self
            .transaction_external_container_children
            .values()
            .flatten()
        {
            geometry
                .entry(node.tala_id)
                .or_insert((self.anchored_external_position(node), node.rect.size));
        }
        for node in self
            .transaction_external_aggregate_children
            .values()
            .flatten()
        {
            geometry
                .entry(node.tala_id)
                .or_insert((self.anchored_external_position(node), node.rect.size));
        }
        for node in &self.transaction_external_containers {
            geometry
                .entry(node.tala_id)
                .or_insert((self.anchored_external_position(node), node.rect.size));
        }
        geometry
    }

    fn projected_transaction_validation_geometry(
        &self,
        state: &ProjectedTransactionState,
    ) -> (
        Vec<(Option<Point>, Size)>,
        IdHashMap<i64, (Option<Point>, Size)>,
    ) {
        let mut geometry = self.projected_transaction_live_geometry();
        let mut state_geometry = Vec::with_capacity(state.nodes.len());
        for entry in &state.nodes {
            let value = geometry.entry(entry.node.tala_id).or_insert_with(|| {
                (
                    self.anchored_external_position(&entry.node),
                    entry.node.rect.size,
                )
            });
            state_geometry.push(*value);
        }
        (state_geometry, geometry)
    }

    fn projected_transaction_box_by_tala(
        &self,
        state: &ProjectedTransactionState,
        tala_id: i64,
    ) -> Option<(Point, Size)> {
        if let Some(entry) = state
            .nodes
            .iter()
            .find(|entry| entry.node.tala_id == tala_id)
        {
            let (position, size) = self.projected_transaction_geometry(entry);
            return Some((position?, size));
        }
        if let Some(node) = self
            .graph_node_order()
            .into_iter()
            .find(|node| self.active_node_tala_id(*node) == tala_id)
        {
            return Some((
                self.active_node_position(node)?,
                self.active_node_size(node),
            ));
        }
        self.transaction_external_container_children
            .values()
            .flatten()
            .chain(
                self.transaction_external_aggregate_children
                    .values()
                    .flatten(),
            )
            .chain(self.transaction_external_containers.iter())
            .find(|node| node.tala_id == tala_id)
            .and_then(|node| Some((self.anchored_external_position(node)?, node.rect.size)))
    }

    fn projected_transaction_orientation(
        node_position: Point,
        node_size: Size,
        other_position: Point,
        other_size: Size,
    ) -> Orientation {
        if node_position.y + node_size.height < other_position.y {
            if node_position.x + node_size.width < other_position.x {
                Orientation::TopLeft
            } else if other_position.x + other_size.width < node_position.x {
                Orientation::TopRight
            } else {
                Orientation::Top
            }
        } else if other_position.y + other_size.height < node_position.y {
            if node_position.x + node_size.width < other_position.x {
                Orientation::BottomLeft
            } else if other_position.x + other_size.width < node_position.x {
                Orientation::BottomRight
            } else {
                Orientation::Bottom
            }
        } else if other_position.x + other_size.width < node_position.x {
            Orientation::Right
        } else if node_position.x + node_size.width < other_position.x {
            Orientation::Left
        } else {
            Orientation::None
        }
    }

    fn projected_transaction_loop_extent(
        node: &ProjectedTransactionNode,
        orientation: Orientation,
    ) -> f64 {
        loop_offset_toward(node.node.loop_offsets.as_ref(), orientation)
    }

    fn projected_transaction_spacing_delta(
        &self,
        left: &ProjectedTransactionNode,
        left_position: Point,
        left_size: Size,
        right: &ProjectedTransactionNode,
        right_position: Point,
        right_size: Size,
    ) -> f64 {
        let mut connected = false;
        let mut max_edge_width = f64::NEG_INFINITY;
        let mut max_edge_height = f64::NEG_INFINITY;
        for edge in &left.edges {
            if edge.adjacent_tala_id != right.node.tala_id {
                continue;
            }
            connected = true;
            max_edge_width = max_edge_width.max(edge.min_width);
            max_edge_height = max_edge_height.max(edge.min_height);
        }
        let mut horizontal: f64 = if connected { 60.0 } else { 20.0 };
        let mut vertical: f64 = if connected { 60.0 } else { 20.0 };
        if left.node.shape == ShapeKind::SqlTable || right.node.shape == ShapeKind::SqlTable {
            horizontal = 120.0;
        }
        horizontal = horizontal.max(max_edge_width);
        vertical = vertical.max(max_edge_height);
        let orientation = Self::projected_transaction_orientation(
            left_position,
            left_size,
            right_position,
            right_size,
        );
        let directional_margin = |margins: Insets, side: Orientation| match side {
            Orientation::TopLeft => (margins.left.trunc(), margins.top.trunc()),
            Orientation::Top => (0.0, margins.top.trunc()),
            Orientation::TopRight => (margins.right.trunc(), margins.top.trunc()),
            Orientation::Right => (margins.right.trunc(), 0.0),
            Orientation::BottomRight => (margins.right.trunc(), margins.bottom.trunc()),
            Orientation::Bottom => (0.0, margins.bottom.trunc()),
            Orientation::BottomLeft => (margins.left.trunc(), margins.bottom.trunc()),
            Orientation::Left => (margins.left.trunc(), 0.0),
            Orientation::None => (0.0, 0.0),
        };
        let (left_margin_width, left_margin_height) =
            directional_margin(left.node.layout_margins, orientation.opposite());
        let (right_margin_width, right_margin_height) =
            directional_margin(right.node.layout_margins, orientation);
        horizontal = horizontal.max(left_margin_width + right_margin_width);
        vertical = vertical.max(left_margin_height + right_margin_height);
        let mut delta = match orientation {
            Orientation::Top | Orientation::Bottom => vertical,
            Orientation::Right | Orientation::Left => horizontal,
            _ => horizontal.min(vertical),
        };
        delta = delta.max(loop_spacing_gap(
            left.node.loop_offsets.as_ref(),
            right.node.loop_offsets.as_ref(),
            orientation,
        ));
        delta
    }

    fn projected_transaction_boxes_overlap(
        left_position: Point,
        left_size: Size,
        right_position: Point,
        right_size: Size,
        delta: f64,
    ) -> bool {
        left_position.x < right_position.x + right_size.width + delta
            && right_position.x < left_position.x + left_size.width + delta
            && left_position.y < right_position.y + right_size.height + delta
            && right_position.y < left_position.y + left_size.height + delta
    }

    pub(super) fn projected_transaction_state(&self) -> ProjectedTransactionState {
        let nodes = if self.projected_transaction_nodes.is_empty() {
            self.graph_node_order()
                .into_iter()
                .map(|node| self.projected_transaction_node_from_current(node))
                .collect::<Vec<_>>()
        } else {
            self.projected_transaction_nodes.clone()
        };
        let geometry = nodes
            .iter()
            .map(|entry| self.projected_transaction_geometry(entry))
            .collect::<Vec<_>>();
        let mut existing_overlaps = BTreeSet::new();
        let mut existing_exact_overlaps = BTreeSet::new();
        for left_index in 0..nodes.len() {
            let (Some(left_position), left_size) = geometry[left_index] else {
                continue;
            };
            for right_index in left_index + 1..nodes.len() {
                let (Some(right_position), right_size) = geometry[right_index] else {
                    continue;
                };
                let delta = self.projected_transaction_spacing_delta(
                    &nodes[left_index],
                    left_position,
                    left_size,
                    &nodes[right_index],
                    right_position,
                    right_size,
                );
                if Self::projected_transaction_boxes_overlap(
                    left_position,
                    left_size,
                    right_position,
                    right_size,
                    delta,
                ) {
                    existing_overlaps.insert((left_index, right_index));
                }
                if Self::projected_transaction_boxes_overlap(
                    left_position,
                    left_size,
                    right_position,
                    right_size,
                    0.0,
                ) {
                    existing_exact_overlaps.insert((left_index, right_index));
                }
            }
        }
        ProjectedTransactionState {
            nodes,
            original_geometry: geometry,
            existing_overlaps,
            existing_exact_overlaps,
        }
    }

    fn projected_transaction_node_is_bad(
        &self,
        state: &ProjectedTransactionState,
        node_index: usize,
        state_geometry: &[(Option<Point>, Size)],
        geometry: &IdHashMap<i64, (Option<Point>, Size)>,
        dirty_flags: Option<&[bool]>,
        fixed_origins: &BTreeMap<Option<i64>, Point>,
        dirty_indices: Option<&[usize]>,
        existing_overlaps: &ProjectedOverlapPairs,
    ) -> bool {
        let node = &state.nodes[node_index];
        // Keep structural validation keyed by the real container pointers.
        // `container_tala_id` and `ancestor_tala_ids` intentionally use the
        // active aggregate vessel IDs for scoring, but Go's Node.container()
        // used by IsBadState follows the member pointer for ordinary
        // descendants.  Canonicalizing that parent to a cluster vessel makes
        // a child appear outside the wrong box during speculative moves.
        let structural_container_tala_id = node.structural_container_tala_id;
        let structural_ancestor_tala_ids = &node.structural_ancestor_tala_ids;
        // Graph.IsBadState returns immediately for real cluster members.
        // Active members are absent from the carrier and a fresh synthetic
        // vessel has its inherited stable-member backlink cleared above.
        if node.node.cluster.is_some() {
            return false;
        }
        let (Some(position), size) = state_geometry[node_index] else {
            return false;
        };
        if let Some(container_tala_id) = structural_container_tala_id
            && let Some((Some(container_position), container_size)) =
                geometry.get(&container_tala_id).copied()
            && !(position.x > container_position.x
                && position.y > container_position.y
                && position.x + size.width < container_position.x + container_size.width
                && position.y + size.height < container_position.y + container_size.height)
        {
            return true;
        }
        if node.node.is_container {
            for child_tala_id in &node.child_tala_ids {
                let Some((Some(child_position), child_size)) = geometry.get(child_tala_id).copied()
                else {
                    continue;
                };
                if !(child_position.x > position.x
                    && child_position.y > position.y
                    && child_position.x + child_size.width < position.x + size.width
                    && child_position.y + child_size.height < position.y + size.height)
                {
                    return true;
                }
            }
        }
        let fixed_origin = fixed_origins.get(&structural_container_tala_id).copied();
        if fixed_origin.is_some_and(|origin| origin.x > position.x || origin.y > position.y) {
            return true;
        }
        if node.node.fixed_top_left.is_some()
            && state.original_geometry[node_index].0 != Some(position)
        {
            return true;
        }
        let node_is_dirty = dirty_flags.is_none_or(|dirty| dirty[node_index]);
        let (dirty_partners, all_partners) = match dirty_indices {
            Some(dirty) if !node_is_dirty => (dirty, 0..0),
            _ => (&[][..], 0..state.nodes.len()),
        };
        for other_index in dirty_partners.iter().copied().chain(all_partners) {
            // Every skip below is a pure `continue`, so test the cheap box
            // prefilter first; the ancestry scans and the overlap lookup only
            // run for partners within the safe delta.
            let (Some(other_position), other_size) = state_geometry[other_index] else {
                continue;
            };
            if transaction_overlap_prefilter_excludes_pair(
                position,
                size,
                other_position,
                other_size,
            ) {
                continue;
            }
            let other = &state.nodes[other_index];
            let other_structural_ancestor_tala_ids = &other.structural_ancestor_tala_ids;
            if other_index == node_index
                || structural_ancestor_tala_ids.contains(&other.node.tala_id)
                || other_structural_ancestor_tala_ids.contains(&node.node.tala_id)
            {
                continue;
            }
            // Go's transaction overlap pass is dirty-node based. Unchanged
            // pairs are deliberately left alone, even when their spacing
            // boundary is visible in the projected aggregate geometry. The
            // stable arena keeps temporary vessel boxes in the validation
            // snapshot, so checking every pair here would reject a candidate
            // that upstream accepts because an unrelated vessel is exactly
            // at its existing minimum gap.
            if dirty_flags.is_some_and(|dirty| !dirty[node_index] && !dirty[other_index]) {
                continue;
            }
            let pair = if node_index < other_index {
                (node_index, other_index)
            } else {
                (other_index, node_index)
            };
            if existing_overlaps.contains(&pair) {
                continue;
            }
            let delta = self.projected_transaction_spacing_delta(
                node,
                position,
                size,
                other,
                other_position,
                other_size,
            );
            if Self::projected_transaction_boxes_overlap(
                position,
                size,
                other_position,
                other_size,
                delta,
            ) {
                return true;
            }
        }
        false
    }

    pub(super) fn projected_transaction_nodes_are_valid(
        &self,
        state: &ProjectedTransactionState,
    ) -> bool {
        self.projected_transaction_nodes_are_valid_with_dirty(state, None)
    }

    pub(super) fn projected_transaction_nodes_are_valid_with_dirty(
        &self,
        state: &ProjectedTransactionState,
        dirty_nodes: Option<&BTreeSet<i64>>,
    ) -> bool {
        #[cfg(feature = "diagnostic-traces")]
        crate::internal::call_counts::count!(TransactionValidate);
        let (state_geometry, geometry) = self.projected_transaction_validation_geometry(state);
        // Each node's fixed-origin bound depends only on its structural
        // container: resolve it once per container, with the same first-match
        // order as a per-node scan of `state.nodes`.
        let mut fixed_origins = BTreeMap::<Option<i64>, Point>::new();
        for candidate in &state.nodes {
            let container = candidate.structural_container_tala_id;
            if fixed_origins.contains_key(&container) {
                continue;
            }
            let Some(fixed) = candidate.node.fixed_top_left else {
                continue;
            };
            let Some((Some(current), _)) = geometry.get(&candidate.node.tala_id).copied() else {
                continue;
            };
            fixed_origins.insert(
                container,
                Point {
                    x: current.x - fixed.x,
                    y: current.y - fixed.y,
                },
            );
        }
        // With a dirty set, a pair is checked only when either node is dirty,
        // so a clean node needs only the dirty partners. All checks are pure;
        // visiting fewer pairs cannot change the result.
        let dirty_flags = dirty_nodes.map(|dirty| {
            state
                .nodes
                .iter()
                .map(|entry| dirty.contains(&entry.node.tala_id))
                .collect::<Vec<_>>()
        });
        let dirty_indices = dirty_flags.as_ref().map(|flags| {
            (0..flags.len())
                .filter(|&index| flags[index])
                .collect::<Vec<_>>()
        });
        // Per-pair membership view of the existing overlaps.
        let existing_overlaps =
            ProjectedOverlapPairs::new(state.nodes.len(), &state.existing_overlaps);
        state.nodes.iter().enumerate().all(|(index, _)| {
            let bad = self.projected_transaction_node_is_bad(
                state,
                index,
                &state_geometry,
                &geometry,
                dirty_flags.as_deref(),
                &fixed_origins,
                dirty_indices.as_deref(),
                &existing_overlaps,
            );
            !bad
        })
    }

    /// Return the current Graph.Nodes identities whose boxes changed from a
    /// transaction snapshot. This is the same dirty boundary used by Go's
    /// post-state overlap sweep; structural checks remain graph-wide.
    pub(super) fn projected_transaction_dirty_node_ids(
        &self,
        state: &ProjectedTransactionState,
    ) -> BTreeSet<i64> {
        let (_, geometry) = self.projected_transaction_validation_geometry(state);
        state
            .nodes
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| {
                let current = geometry.get(&entry.node.tala_id)?;
                (*current != state.original_geometry[index]).then_some(entry.node.tala_id)
            })
            .collect()
    }

    pub(super) fn projected_transaction_spacing_overlap_became_exact(
        &self,
        state: &ProjectedTransactionState,
    ) -> bool {
        let (state_geometry, _) = self.projected_transaction_validation_geometry(state);
        state
            .existing_overlaps
            .difference(&state.existing_exact_overlaps)
            .any(|&(left, right)| {
                let (Some(left_position), left_size) = state_geometry[left] else {
                    return false;
                };
                let (Some(right_position), right_size) = state_geometry[right] else {
                    return false;
                };
                Self::projected_transaction_boxes_overlap(
                    left_position,
                    left_size,
                    right_position,
                    right_size,
                    0.0,
                )
            })
    }

    /// Refresh fallback boxes after one split component publishes its shared
    /// node mutations to the owner. The carrier's identity/order and static
    /// metadata remain unchanged.
    pub(super) fn refresh_projected_transaction_node_geometry(&mut self) {
        let geometry = self
            .projected_transaction_nodes
            .iter()
            .map(|entry| self.projected_transaction_geometry(entry))
            .collect::<Vec<_>>();
        for (entry, (position, size)) in self.projected_transaction_nodes.iter_mut().zip(geometry) {
            entry.node.position = position;
            entry.node.rect.origin = position.unwrap_or_default();
            entry.node.rect.size = size;
        }
    }

    pub(super) fn overlap_pairs(&self, current_vessel_edges: bool) -> BTreeSet<(NodeId, NodeId)> {
        let mut pairs = BTreeSet::new();
        let nodes = self.graph_node_order();
        let geometry = nodes
            .iter()
            .copied()
            .map(|node| {
                self.active_node_position(node)
                    .map(|position| (position, self.active_node_size(node)))
            })
            .collect::<Vec<_>>();
        if crate::internal::trace_env_enabled("TALA_TRACE_OVERLAP_PAIR")
            && nodes
                .iter()
                .any(|node| matches!(self.nodes[node.0 as usize].tala_id, 560439961 | 510107104))
        {
            eprintln!(
                "OVERLAP_ORDER_RUST nodes={:?}",
                nodes
                    .iter()
                    .map(|node| self.nodes[node.0 as usize].tala_id)
                    .collect::<Vec<_>>()
            );
        }
        for (index, left) in nodes.iter().copied().enumerate() {
            let Some((a, left_size)) = geometry[index] else {
                continue;
            };
            for (right_index, right) in nodes.iter().copied().enumerate().skip(index + 1) {
                let Some((b, right_size)) = geometry[right_index] else {
                    continue;
                };
                let delta = if current_vessel_edges {
                    self.transaction_spacing_delta(left, right, a)
                } else {
                    self.spacing_delta_with_loops(left, right, a)
                };
                if crate::internal::trace_env_enabled("TALA_TRACE_OVERLAP_PAIR")
                    && ((self.nodes[left.0 as usize].tala_id == 1794367627
                        && self.nodes[right.0 as usize].tala_id == 3648382702)
                        || (self.nodes[left.0 as usize].tala_id == 3648382702
                            && self.nodes[right.0 as usize].tala_id == 1794367627))
                {
                    eprintln!(
                        "OVERLAP_PAIR_RUST vessel_edges={} left={} right={} delta={} left_edges={:?}",
                        current_vessel_edges,
                        self.nodes[left.0 as usize].tala_id,
                        self.nodes[right.0 as usize].tala_id,
                        delta,
                        self.active_edge_ids(left)
                            .iter()
                            .map(
                                |edge| self.nodes[self.active_adjacent(left, *edge).0 as usize]
                                    .tala_id
                            )
                            .collect::<Vec<_>>()
                    );
                }
                if a.x < b.x + right_size.width + delta
                    && b.x < a.x + left_size.width + delta
                    && a.y < b.y + right_size.height + delta
                    && b.y < a.y + left_size.height + delta
                {
                    pairs.insert(if left < right {
                        (left, right)
                    } else {
                        (right, left)
                    });
                }
            }
        }
        pairs
    }

    pub(super) fn existing_overlap_pairs(&self) -> BTreeSet<(NodeId, NodeId)> {
        self.overlap_pairs(false)
    }

    /// transaction.go buildTransactionOverlaps: sweep positioned boxes with
    /// a conservative clearance bound before evaluating the spacing predicate.
    /// Equidistance and transpose need both sets at the same rollback point.
    pub(super) fn refinement_overlap_pairs(
        &self,
    ) -> (BTreeSet<(NodeId, NodeId)>, BTreeSet<(NodeId, NodeId)>) {
        let fallback = || (self.existing_overlap_pairs(), self.exact_overlap_pairs());
        let nodes = self.graph_node_order();
        let mut padding = 120.0_f64; // TableNodeGap, also covers NodeGap.
        let mut max_margin = 0.0_f64;
        let mut max_loop_offset = 0.0_f64;
        let mut placed = Vec::with_capacity(nodes.len());
        for (index, &id) in nodes.iter().enumerate() {
            let node = &self.nodes[id.0 as usize];
            let margin = node.layout_margins;
            for value in [margin.top, margin.right, margin.bottom, margin.left] {
                if !value.is_finite() {
                    return fallback();
                }
                max_margin = max_margin.max(value);
            }
            if let Some(offsets) = &node.loop_offsets {
                for &value in offsets.values() {
                    // Preserve wrapping int conversion for malformed direct
                    // callers by retaining the old scan outside this range.
                    if !(0.0..=1_000_000_000_000_000.0).contains(&value) {
                        return fallback();
                    }
                    max_loop_offset = max_loop_offset.max(value);
                }
            }
            let Some(position) = self.active_node_position(id) else {
                continue;
            };
            let size = self.active_node_size(id);
            if !position.x.is_finite()
                || !position.y.is_finite()
                || !size.width.is_finite()
                || !size.height.is_finite()
                || size.width < 0.0
                || size.height < 0.0
                || !(position.x + size.width).is_finite()
                || !(position.y + size.height).is_finite()
            {
                return fallback();
            }
            placed.push((index, id, position, size));
        }
        // The predicates can inspect incident edges absent from the current
        // graph edge slice. edge_records includes those retained references.
        for edge in &self.edge_records {
            if !edge.min_width.is_finite() || !edge.min_height.is_finite() {
                return fallback();
            }
            padding = padding.max(edge.min_width).max(edge.min_height);
        }
        padding = padding
            .max(2.0 * max_margin)
            .max(20.0 + 2.0 * max_loop_offset);
        if !padding.is_finite() {
            return fallback();
        }
        placed.sort_by(|left, right| {
            left.2
                .x
                .partial_cmp(&right.2.x)
                .unwrap()
                .then(left.0.cmp(&right.0))
        });
        let mut overlaps = BTreeSet::new();
        let mut exact = BTreeSet::new();
        let mut active = Vec::<usize>::new();
        for (current_index, &(index, id, position, size)) in placed.iter().enumerate() {
            active.retain(|&previous| {
                let (_, _, other_position, other_size) = placed[previous];
                other_position.x + other_size.width + padding > position.x
            });
            for &previous in &active {
                let (other_index, other_id, other_position, other_size) = placed[previous];
                if other_position.y >= position.y + size.height + padding
                    || position.y >= other_position.y + other_size.height + padding
                {
                    continue;
                }
                // Keep the original Graph.Nodes operand order. A malformed
                // asymmetric incident-edge slice can change spacing by side.
                let (left, right, a, b, left_size, right_size) = if index < other_index {
                    (id, other_id, position, other_position, size, other_size)
                } else {
                    (other_id, id, other_position, position, other_size, size)
                };
                let pair = if left < right {
                    (left, right)
                } else {
                    (right, left)
                };
                if a.x < b.x + right_size.width
                    && b.x < a.x + left_size.width
                    && a.y < b.y + right_size.height
                    && b.y < a.y + left_size.height
                {
                    exact.insert(pair);
                }
                let delta = self.spacing_delta_with_loops(left, right, a);
                if a.x < b.x + right_size.width + delta
                    && b.x < a.x + left_size.width + delta
                    && a.y < b.y + right_size.height + delta
                    && b.y < a.y + left_size.height + delta
                {
                    overlaps.insert(pair);
                }
            }
            active.push(current_index);
        }
        (overlaps, exact)
    }

    /// Refinement trials on ordinary graphs change only box geometry. As in
    /// transaction.go geometryChanged, unchanged pairs retain the constructor's
    /// overlap exceptions. Aggregate flips also carry projected edge state, so
    /// keep the full pair validation whenever those adapters are present.
    pub(super) fn refinement_validation_geometry(
        &self,
    ) -> Option<Vec<(NodeId, Option<Point>, Size)>> {
        if !self.clusters.is_empty() || !self.sequences.is_empty() {
            return None;
        }
        Some(
            self.graph_node_order()
                .into_iter()
                .map(|id| (id, self.active_node_position(id), self.active_node_size(id)))
                .collect(),
        )
    }

    /// Transaction.Commit builds nearby-box candidates before the symmetric
    /// and directional checks. Share that broad phase for the two refinement
    /// paths, keeping each exact predicate's operand order and exceptions.
    pub(super) fn refinement_validation_overlaps(
        &self,
        existing_overlaps: &BTreeSet<(NodeId, NodeId)>,
        include_symmetric: bool,
        baseline: Option<&[(NodeId, Option<Point>, Size)]>,
    ) -> (bool, bool) {
        let fallback = || {
            (
                include_symmetric && self.transaction_has_new_overlap(existing_overlaps),
                self.transaction_has_bad_state_overlap(existing_overlaps),
            )
        };
        // Diagnostic observers report the first pair in Graph.Nodes order.
        if crate::internal::trace_env_enabled("TALA_TRACE_OVERLAP_PAIR")
            || crate::internal::trace_env_enabled("TALA_TRACE_BAD_STATE_PAIR")
        {
            return fallback();
        }
        let nodes = self.graph_node_order();
        let baseline = baseline.filter(|original| {
            original.len() == nodes.len()
                && original
                    .iter()
                    .zip(&nodes)
                    .all(|(entry, id)| entry.0 == *id)
        });
        let mut placed = Vec::with_capacity(nodes.len());
        for (index, id) in nodes.into_iter().enumerate() {
            let Some(position) = self.active_node_position(id) else {
                continue;
            };
            let size = self.active_node_size(id);
            if !position.x.is_finite()
                || !position.y.is_finite()
                || !size.width.is_finite()
                || !size.height.is_finite()
                || size.width < 0.0
                || size.height < 0.0
                || !(position.x + size.width).is_finite()
                || !(position.y + size.height).is_finite()
            {
                return fallback();
            }
            let unchanged = baseline.is_some_and(|original| {
                let (_, old_position, old_size) = original[index];
                old_position.is_some_and(|old| {
                    old.x.to_bits() == position.x.to_bits()
                        && old.y.to_bits() == position.y.to_bits()
                }) && old_size.width.to_bits() == size.width.to_bits()
                    && old_size.height.to_bits() == size.height.to_bits()
            });
            placed.push((index, id, position, size, !unchanged));
        }
        placed.sort_by(|left, right| {
            left.2
                .x
                .partial_cmp(&right.2.x)
                .unwrap()
                .then(left.0.cmp(&right.0))
        });
        let mut symmetric = false;
        let mut directional = false;
        let mut active = Vec::<usize>::new();
        for (current, &(index, id, position, size, changed)) in placed.iter().enumerate() {
            // Keep equality, as the recovered 500-unit filter uses strict gaps.
            active.retain(|&previous| {
                let (_, _, other, other_size, _) = placed[previous];
                !(other.x + other_size.width + 500.0 < position.x)
            });
            for &previous in &active {
                let (other_index, other_id, other, other_size, other_changed) = placed[previous];
                if !changed && !other_changed {
                    continue;
                }
                let (left, right, a, b, left_size, right_size) = if index < other_index {
                    (id, other_id, position, other, size, other_size)
                } else {
                    (other_id, id, other, position, other_size, size)
                };
                let pair = if left < right {
                    (left, right)
                } else {
                    (right, left)
                };
                if existing_overlaps.contains(&pair) {
                    continue;
                }
                let forward =
                    !transaction_overlap_prefilter_excludes_pair(a, left_size, b, right_size);
                let reverse =
                    !transaction_overlap_prefilter_excludes_pair(b, right_size, a, left_size);
                let need_forward = (include_symmetric && !symmetric)
                    || (!directional
                        && left != right
                        && self.nodes[left.0 as usize].cluster.is_none());
                let forward_overlap = forward && need_forward && {
                    let delta = self.spacing_delta_with_loops(left, right, a);
                    a.x < b.x + right_size.width + delta
                        && b.x < a.x + left_size.width + delta
                        && a.y < b.y + right_size.height + delta
                        && b.y < a.y + left_size.height + delta
                };
                let reverse_overlap = reverse
                    && !directional
                    && left != right
                    && self.nodes[right.0 as usize].cluster.is_none()
                    && {
                        let delta = self.spacing_delta_with_loops(right, left, b);
                        b.x < a.x + left_size.width + delta
                            && a.x < b.x + right_size.width + delta
                            && b.y < a.y + left_size.height + delta
                            && a.y < b.y + right_size.height + delta
                    };
                // Ancestry is a pure read. Resolve it only for an actual
                // geometric hit, rather than for every pair in the graph.
                if !(forward_overlap || reverse_overlap)
                    || self.active_is_descendant_of(left, right)
                    || self.active_is_descendant_of(right, left)
                {
                    continue;
                }
                symmetric |= include_symmetric && forward_overlap;
                directional |= reverse_overlap
                    || (forward_overlap
                        && left != right
                        && self.nodes[left.0 as usize].cluster.is_none());
                if (!include_symmetric || symmetric) && directional {
                    return (symmetric, directional);
                }
            }
            active.push(current);
        }
        (symmetric, directional)
    }

    pub(super) fn retained_vessel_overlap_pairs(
        &self,
        retained_vessels: &BTreeSet<NodeId>,
    ) -> BTreeSet<(NodeId, NodeId)> {
        let mut pairs = BTreeSet::new();
        let nodes = self.graph_node_order();
        let geometry = nodes
            .iter()
            .copied()
            .map(|node| {
                self.active_node_position(node)
                    .map(|position| (position, self.active_node_size(node)))
            })
            .collect::<Vec<_>>();
        for (index, left) in nodes.iter().copied().enumerate() {
            let Some((a, left_size)) = geometry[index] else {
                continue;
            };
            for (right_index, right) in nodes.iter().copied().enumerate().skip(index + 1) {
                if !retained_vessels.contains(&left) && !retained_vessels.contains(&right) {
                    continue;
                }
                let Some((b, right_size)) = geometry[right_index] else {
                    continue;
                };
                let delta = self.transaction_spacing_delta(left, right, a);
                if a.x < b.x + right_size.width + delta
                    && b.x < a.x + left_size.width + delta
                    && a.y < b.y + right_size.height + delta
                    && b.y < a.y + left_size.height + delta
                {
                    pairs.insert(if left < right {
                        (left, right)
                    } else {
                        (right, left)
                    });
                }
            }
        }
        pairs
    }

    pub(super) fn transaction_has_new_overlap(
        &self,
        existing_overlaps: &BTreeSet<(NodeId, NodeId)>,
    ) -> bool {
        self.transaction_has_new_overlap_mode(existing_overlaps, false)
    }

    /// Candidate transactions normally move only a small set of nodes. Once
    /// the pre-transaction overlap set is known, an unchanged pair cannot
    /// become a new overlap, so retain the recovered predicate while avoiding
    /// a full quadratic scan for every gap trial.
    pub(super) fn transaction_has_new_overlap_for_nodes(
        &self,
        existing_overlaps: &BTreeSet<(NodeId, NodeId)>,
        moved: &BTreeSet<NodeId>,
    ) -> bool {
        let nodes = self.graph_node_order();
        let geometry = nodes
            .iter()
            .copied()
            .map(|node| {
                self.active_node_position(node)
                    .map(|position| (position, self.active_node_size(node)))
            })
            .collect::<Vec<_>>();
        let mut checked = BTreeSet::new();
        for (moved_index, moved_node) in nodes
            .iter()
            .copied()
            .enumerate()
            .filter(|(_, node)| moved.contains(node))
        {
            for (other_index, other) in nodes.iter().copied().enumerate() {
                if moved_index == other_index {
                    continue;
                }
                // Preserve the operand order of the original complete
                // Graph.Nodes nested loop. Spacing margins can depend on the
                // facing side of the earlier node.
                let (left, right) = if moved_index < other_index {
                    (moved_node, other)
                } else {
                    (other, moved_node)
                };
                let pair = if left < right {
                    (left, right)
                } else {
                    (right, left)
                };
                if !checked.insert(pair)
                    || existing_overlaps.contains(&pair)
                    || self.active_is_descendant_of(left, right)
                    || self.active_is_descendant_of(right, left)
                {
                    continue;
                }
                let Some((a, left_size)) = geometry[if moved_index < other_index {
                    moved_index
                } else {
                    other_index
                }] else {
                    continue;
                };
                let Some((b, right_size)) = geometry[if moved_index < other_index {
                    other_index
                } else {
                    moved_index
                }] else {
                    continue;
                };
                if transaction_overlap_prefilter_excludes_pair(a, left_size, b, right_size) {
                    continue;
                }
                let delta = self.spacing_delta_with_loops(left, right, a);
                if a.x < b.x + right_size.width + delta
                    && b.x < a.x + left_size.width + delta
                    && a.y < b.y + right_size.height + delta
                    && b.y < a.y + left_size.height + delta
                {
                    #[cfg(feature = "diagnostic-traces")]
                    if crate::internal::trace_env_enabled("TALA_TRACE_GAP_VALIDATION") {
                        eprintln!(
                            "TRANSACTION_NEW_OVERLAP_RUST left={} right={} delta_bits={:#018x} left_box={:?}/{:?} right_box={:?}/{:?}",
                            self.active_node_tala_id(left),
                            self.active_node_tala_id(right),
                            delta.to_bits(),
                            a,
                            left_size,
                            b,
                            right_size
                        );
                    }
                    if crate::internal::trace_env_enabled("TALA_TRACE_BINPACK_OVERLAP_260")
                        && moved.iter().any(|node| {
                            matches!(
                                self.nodes[node.0 as usize].tala_id,
                                2600858170
                                    | 1308591071
                                    | 4248473833
                                    | 3802298614
                                    | 365607126
                                    | 2531950148
                            )
                        })
                    {
                        eprintln!(
                            "BINPACK_NEW_PAIR_260_RUST left={} right={} delta={} left_desc_right={} right_desc_left={} left_container={:?} right_container={:?} left_box={:?}/{:?} right_box={:?}/{:?}",
                            self.nodes[left.0 as usize].tala_id,
                            self.nodes[right.0 as usize].tala_id,
                            delta,
                            self.active_is_descendant_of(left, right),
                            self.active_is_descendant_of(right, left),
                            self.active_node_container(left)
                                .map(|node| self.nodes[node.0 as usize].tala_id),
                            self.active_node_container(right)
                                .map(|node| self.nodes[node.0 as usize].tala_id),
                            a,
                            left_size,
                            b,
                            right_size
                        );
                    }
                    if crate::internal::trace_env_enabled("TALA_TRACE_BINPACK_PAIR")
                        && moved
                            .iter()
                            .any(|node| self.nodes[node.0 as usize].tala_id == 11)
                    {
                        eprintln!(
                            "BINPACK_NEW_PAIR_RUST left={} right={} delta={} left_box={:?}/{:?} right_box={:?}/{:?}",
                            self.nodes[left.0 as usize].tala_id,
                            self.nodes[right.0 as usize].tala_id,
                            delta,
                            a,
                            left_size,
                            b,
                            right_size
                        );
                    }
                    return true;
                }
            }
        }
        false
    }

    fn transaction_has_new_overlap_mode(
        &self,
        existing_overlaps: &BTreeSet<(NodeId, NodeId)>,
        current_vessel_edges: bool,
    ) -> bool {
        let nodes = self.graph_node_order();
        let geometry = nodes
            .iter()
            .copied()
            .map(|node| {
                self.active_node_position(node)
                    .map(|position| (position, self.active_node_size(node)))
            })
            .collect::<Vec<_>>();
        if crate::internal::trace_env_enabled("TALA_TRACE_OVERLAP_PAIR")
            && nodes
                .iter()
                .any(|node| matches!(self.nodes[node.0 as usize].tala_id, 560439961 | 510107104))
        {
            eprintln!(
                "OVERLAP_MODE_ORDER_RUST nodes={:?}",
                nodes
                    .iter()
                    .map(|node| self.nodes[node.0 as usize].tala_id)
                    .collect::<Vec<_>>()
            );
        }
        for (index, left) in nodes.iter().copied().enumerate() {
            let Some((a, left_size)) = geometry[index] else {
                continue;
            };
            for (right_index, right) in nodes.iter().copied().enumerate().skip(index + 1) {
                let pair = if left < right {
                    (left, right)
                } else {
                    (right, left)
                };
                if existing_overlaps.contains(&pair)
                    || self.active_is_descendant_of(left, right)
                    || self.active_is_descendant_of(right, left)
                {
                    if crate::internal::trace_env_enabled("TALA_TRACE_OVERLAP_PAIR") {
                        let left_id = self.nodes[left.0 as usize].tala_id;
                        let right_id = self.nodes[right.0 as usize].tala_id;
                        if (left_id == 560439961 && right_id == 510107104)
                            || (left_id == 510107104 && right_id == 560439961)
                        {
                            eprintln!(
                                "OVERLAP_PAIR_RUST_SKIP left={} right={} existing={} left_desc_right={} right_desc_left={} order_len={}",
                                left_id,
                                right_id,
                                existing_overlaps.contains(&pair),
                                self.is_descendant_of(left, right),
                                self.is_descendant_of(right, left),
                                nodes.len()
                            );
                        }
                    }
                    continue;
                }
                let Some((b, right_size)) = geometry[right_index] else {
                    continue;
                };
                if transaction_overlap_prefilter_excludes_pair(a, left_size, b, right_size) {
                    continue;
                }
                let delta = if current_vessel_edges {
                    self.transaction_spacing_delta(left, right, a)
                } else {
                    self.spacing_delta_with_loops(left, right, a)
                };
                if a.x < b.x + right_size.width + delta
                    && b.x < a.x + left_size.width + delta
                    && a.y < b.y + right_size.height + delta
                    && b.y < a.y + left_size.height + delta
                {
                    if crate::internal::trace_env_enabled("TALA_TRACE_OVERLAP_PAIR") {
                        let left_id = self.nodes[left.0 as usize].tala_id;
                        let right_id = self.nodes[right.0 as usize].tala_id;
                        if (left_id == 560439961 && right_id == 510107104)
                            || (left_id == 510107104 && right_id == 560439961)
                        {
                            eprintln!(
                                "OVERLAP_PAIR_RUST_HIT left={} right={} delta={} left_box={:?}/{:?} right_box={:?}/{:?}",
                                left_id, right_id, delta, a, left_size, b, right_size
                            );
                        }
                    }
                    return true;
                }
            }
        }
        false
    }

    /// Mirrors Graph.IsBadState's per-node overlap pass used by
    /// Transaction.Commit. The ordinary transaction predicates above are
    /// useful for broad candidate filtering, but Commit also asks whether
    /// each moved node overlaps anything outside its ancestor/descendant
    /// exceptions under the prior overlap map.
    pub(super) fn transaction_has_bad_state_overlap(
        &self,
        existing_overlaps: &BTreeSet<(NodeId, NodeId)>,
    ) -> bool {
        self.transaction_has_bad_state_overlap_mode(existing_overlaps, false)
    }

    pub(super) fn transaction_has_bad_state_overlap_for_nodes(
        &self,
        existing_overlaps: &BTreeSet<(NodeId, NodeId)>,
        moved: &BTreeSet<NodeId>,
    ) -> bool {
        self.transaction_bad_state_overlap_mode(existing_overlaps, moved, false)
    }

    pub(super) fn transaction_containers_have_bad_state_overlap(
        &self,
        existing_overlaps: &BTreeSet<(NodeId, NodeId)>,
    ) -> bool {
        self.transaction_bad_state_overlap_mode(existing_overlaps, &BTreeSet::new(), true)
    }

    fn transaction_bad_state_overlap_mode(
        &self,
        existing_overlaps: &BTreeSet<(NodeId, NodeId)>,
        moved: &BTreeSet<NodeId>,
        containers_only: bool,
    ) -> bool {
        let graph_node_set = self.graph_node_order().into_iter().collect::<BTreeSet<_>>();
        let mut node_set = graph_node_set.clone();
        if containers_only {
            // Go's Commit validates every Graph.Containers key, including
            // containers temporarily absent from Graph.Nodes while an
            // aggregate vessel is active. Keep the stable arena identity and
            // add those keys to the pre-SyncClusters candidate set.
            node_set.extend(self.containers.keys().flatten());
        }
        let nodes = node_set.into_iter().collect::<Vec<_>>();
        // A validation call observes one immutable candidate geometry.  The
        // recovered pair scan is quadratic, so resolving aggregate positions,
        // aggregate sizes, and moved membership inside both loops repeats the
        // same work O(nodes²) times.  Materialize those values once while
        // retaining the original Graph.Nodes pair order and directional
        // spacing predicate below.
        let geometry = nodes
            .iter()
            .copied()
            .map(|node| {
                (
                    self.active_node_position(node)
                        .map(|position| (position, self.active_node_size(node))),
                    moved.contains(&node),
                )
            })
            .collect::<Vec<_>>();
        // Go transaction.go:900 enumerates only pairs incident to dirty
        // nodes. Keep receivers in their original order and both directional
        // checks, but a clean receiver needs only the ordered moved partners.
        let moved_indices = geometry
            .iter()
            .enumerate()
            .filter_map(|(index, &(_, moved))| moved.then_some(index))
            .collect::<Vec<_>>();
        for (node_index, node) in nodes.iter().copied().enumerate() {
            // Commit checks every retained Containers key before SyncClusters.
            // This pass uses only the container as the directional receiver.
            if containers_only && !self.containers.contains_key(&Some(node)) {
                continue;
            }

            // Graph.IsBadState returns immediately for cluster members; the
            // cluster vessel owns their overlap validity during Commit.
            if self.nodes[node.0 as usize].cluster.is_some() {
                continue;
            }
            // The spacing delta is directional: Graph.IsBadState checks the
            // current node as the receiver, so a moved node can change the
            // result even when it appears as `other` in an unchanged node's
            // pass. Skip only pairs where neither endpoint moved, retaining
            // both orientations for every affected pair.
            let Some((node_position, node_size)) = geometry[node_index].0 else {
                continue;
            };
            let node_moved = geometry[node_index].1;
            let (moved_partners, all_partners) = if !containers_only && !node_moved {
                (moved_indices.as_slice(), 0..0)
            } else {
                (&[][..], 0..nodes.len())
            };
            for other_index in moved_partners.iter().copied().chain(all_partners) {
                let other = nodes[other_index];
                if other == node {
                    continue;
                }
                // Graph.isBadState checks a container receiver against
                // Graph.Nodes. Containers that are temporarily retained in
                // the stable arena are valid receivers, but they are not
                // additional comparison candidates. Including hidden
                // containers here makes Rust reject a trial against a box
                // that the Go graph never visits.
                if containers_only && !graph_node_set.contains(&other) {
                    continue;
                }
                let Some((other_position, other_size)) = geometry[other_index].0 else {
                    continue;
                };
                if transaction_overlap_prefilter_excludes_pair(
                    node_position,
                    node_size,
                    other_position,
                    other_size,
                ) {
                    continue;
                }
                let reverse_descendant = self.active_is_descendant_of(other, node);
                let forward_descendant = self.active_is_descendant_of(node, other);
                let pair = if node < other {
                    (node, other)
                } else {
                    (other, node)
                };
                let prior_overlap = existing_overlaps.contains(&pair);
                if crate::internal::trace_env_enabled("TALA_TRACE_SWAP_PAIRS") {
                    let ids = (
                        self.nodes[node.0 as usize].tala_id,
                        self.nodes[other.0 as usize].tala_id,
                    );
                    if ids.0 == 3159886487
                        || ids.1 == 3159886487
                        || ids.0 == 2414116196
                        || ids.1 == 2414116196
                        || ids.0 == 3193441725
                        || ids.1 == 3193441725
                    {
                        eprintln!(
                            "SWAP_PAIR_RUST node={} other={} moved={} prior={} desc={} node_pos={:?} other_pos={:?}",
                            ids.0,
                            ids.1,
                            node_moved || geometry[other_index].1,
                            prior_overlap,
                            reverse_descendant || forward_descendant,
                            geometry[node_index].0,
                            geometry[other_index].0
                        );
                    }
                }
                if reverse_descendant || forward_descendant {
                    continue;
                }
                // Graph.isBadState passes PriorGraphState.existingOverlaps
                // through to doesOverlapWithDimensionsContext even during
                // the pre-SyncClusters container pass. Retain an overlap
                // that was already present before this trial in both paths.
                if prior_overlap {
                    continue;
                }
                let delta = self.spacing_delta_with_loops(node, other, node_position);
                if node_position.x < other_position.x + other_size.width + delta
                    && other_position.x < node_position.x + node_size.width + delta
                    && node_position.y < other_position.y + other_size.height + delta
                    && other_position.y < node_position.y + node_size.height + delta
                {
                    if crate::internal::trace_env_enabled("TALA_TRACE_BAD_STATE") {
                        eprintln!(
                            "BAD_STATE_PAIR_RUST containers_only={} node_is_container={} other_in_graph={} node={} other={} prior_overlap={} desc={} delta={} node_box={:?}/{:?} other_box={:?}/{:?}",
                            containers_only,
                            self.containers.contains_key(&Some(node)),
                            graph_node_set.contains(&other),
                            self.nodes[node.0 as usize].tala_id,
                            self.nodes[other.0 as usize].tala_id,
                            prior_overlap,
                            reverse_descendant || forward_descendant,
                            delta,
                            node_position,
                            node_size,
                            other_position,
                            other_size,
                        );
                    }
                    return true;
                }
            }
        }
        false
    }

    fn transaction_has_bad_state_overlap_mode(
        &self,
        existing_overlaps: &BTreeSet<(NodeId, NodeId)>,
        current_vessel_edges: bool,
    ) -> bool {
        let nodes = self.graph_node_order();
        let geometry = nodes
            .iter()
            .copied()
            .map(|node| {
                self.active_node_position(node)
                    .map(|position| (position, self.active_node_size(node)))
            })
            .collect::<Vec<_>>();
        for (node_index, node) in nodes.iter().copied().enumerate() {
            // Keep the recovered IsBadState cluster-member short circuit in
            // the full validation path as well.
            if self.nodes[node.0 as usize].cluster.is_some() {
                continue;
            }
            let Some((node_position, node_size)) = geometry[node_index] else {
                continue;
            };
            for (other_index, other) in nodes.iter().copied().enumerate() {
                if other == node {
                    continue;
                }
                let Some((other_position, other_size)) = geometry[other_index] else {
                    continue;
                };
                if transaction_overlap_prefilter_excludes_pair(
                    node_position,
                    node_size,
                    other_position,
                    other_size,
                ) {
                    continue;
                }
                if self.active_is_descendant_of(other, node)
                    || self.active_is_descendant_of(node, other)
                {
                    continue;
                }
                let pair = if node < other {
                    (node, other)
                } else {
                    (other, node)
                };
                if existing_overlaps.contains(&pair) {
                    continue;
                }
                let delta = if current_vessel_edges {
                    self.transaction_spacing_delta(node, other, node_position)
                } else {
                    self.spacing_delta_with_loops(node, other, node_position)
                };
                let overlap = node_position.x < other_position.x + other_size.width + delta
                    && node_position.x + node_size.width + delta > other_position.x
                    && node_position.y < other_position.y + other_size.height + delta
                    && node_position.y + node_size.height + delta > other_position.y;
                if overlap {
                    if crate::internal::trace_env_enabled("TALA_TRACE_BAD_STATE_PAIR") {
                        eprintln!(
                            "BAD_STATE_PAIR_RUST node={} other={} delta={} existing={} nodeBox={:?},{:?} otherBox={:?},{:?} node_container={:?} other_container={:?} descendants={} reverse={}",
                            self.nodes[node.0 as usize].tala_id,
                            self.nodes[other.0 as usize].tala_id,
                            delta,
                            existing_overlaps.contains(&pair),
                            node_position,
                            node_size,
                            other_position,
                            other_size,
                            self.nodes[node.0 as usize]
                                .container
                                .map(|parent| self.nodes[parent.0 as usize].tala_id),
                            self.nodes[other.0 as usize]
                                .container
                                .map(|parent| self.nodes[parent.0 as usize].tala_id),
                            self.is_descendant_of(node, other),
                            self.is_descendant_of(other, node),
                        );
                    }
                    return true;
                }
            }
        }
        false
    }

    pub(super) fn transaction_containment_is_valid(&self) -> bool {
        let local_valid = self.containers.iter().all(|(container, _children)| {
            let Some(raw_container) = container else {
                return true;
            };
            // Graph.IsBadState returns immediately for a cluster member.
            // Such a member can itself remain a Containers key even though
            // the active graph exposes the cluster's distinct vessel.  Its
            // retained stable box is therefore not a container-escape
            // boundary during transaction validation.
            if self.nodes[raw_container.0 as usize].cluster.is_some() {
                return true;
            }
            let container = self.active_aggregate_owner(raw_container);
            let Some(outer) = self.active_node_position(container) else {
                return true;
            };
            let outer_size = self.active_node_size(container);
            let mut seen = BTreeSet::new();
            self.container_node_order(Some(container))
                .into_iter()
                .all(|child| {
                    let child = self.active_aggregate_owner(child);
                    if !seen.insert(child) {
                        return true;
                    }
                    let Some(inner) = self.active_node_position(child) else {
                        return true;
                    };
                    let inner_size = self.active_node_size(child);
                    // Graph.IsBadState validates ordinary container children
                    // through Node.Surrounds(child, 0), whose four release
                    // comparisons are strict. Exact contact with any side is
                    // therefore an escape, not valid containment.
                    let valid = inner.x > outer.x
                        && inner.y > outer.y
                        && inner.x + inner_size.width < outer.x + outer_size.width
                        && inner.y + inner_size.height < outer.y + outer_size.height;
                    if !valid && crate::internal::trace_env_enabled("TALA_TRACE_DEJITTER_NODE") {
                        eprintln!(
                            "CONTAINMENT_FAIL_RUST container={} child={} outer={:?}/{:?} inner={:?}/{:?}",
                            self.nodes[container.0 as usize].tala_id,
                            self.nodes[child.0 as usize].tala_id,
                            outer,
                            outer_size,
                            inner,
                            inner_size,
                        );
                    }
                    valid
                })
        });
        if !local_valid {
            return false;
        }

        true
    }

    /// Match Go's transaction `collectFixedOrigins` plus `IsBadState` fixed
    /// origin predicate for the current `Graph.Nodes` view. A fixed node does
    /// not pin every movable node directly; it establishes the minimum x/y
    /// origin for the other nodes in its placement container.
    pub(super) fn transaction_fixed_origin_is_valid(&self) -> bool {
        let nodes = self.graph_node_order_cow();
        let mut fixed_origins = BTreeMap::new();
        for node_id in nodes.iter() {
            let node = &self.nodes[node_id.0 as usize];
            // Graph.Nodes contains a distinct, newly created aggregate vessel,
            // whose FixedTopLeft is nil. The arena's first member carries that
            // identity but retains its own authored metadata for restoration.
            if self.active_node_is_aggregate(*node_id)
                && self.active_aggregate_owner(*node_id) == *node_id
            {
                continue;
            }
            let (Some(position), Some(fixed)) =
                (self.active_node_position(*node_id), node.fixed_top_left)
            else {
                continue;
            };
            fixed_origins
                .entry(self.active_node_container(*node_id))
                .or_insert(Point {
                    x: position.x - fixed.x,
                    y: position.y - fixed.y,
                });
        }

        // With no current fixed origin there is no origin constraint. Keep
        // the ordered collection above, but avoid a second node walk in the
        // common unfixed case, as collectFixedOrigins does in Go.
        if fixed_origins.is_empty() {
            return true;
        }

        nodes.iter().copied().all(|node_id| {
            let node = &self.nodes[node_id.0 as usize];
            if node.cluster.is_some() && !self.active_node_is_aggregate(node_id) {
                // Go skips retained cluster members, but the distinct current
                // vessel has Cluster=nil and must satisfy the origin bound.
                return true;
            }
            let Some(position) = self.active_node_position(node_id) else {
                return true;
            };
            fixed_origins
                .get(&self.active_node_container(node_id))
                .is_none_or(|origin| !(position.x < origin.x || position.y < origin.y))
        })
    }

    /// Whether ordinary container transactions can operate on the arena's
    /// current geometry. This is a state precondition, not a graph category:
    /// every represented hierarchy uses the same transaction semantics once
    /// its child boxes are materially contained.
    pub(super) fn has_materialized_hierarchy(&self) -> bool {
        self.nodes.iter().any(|node| node.is_container) && self.transaction_containment_is_valid()
    }

    /// Gap-normalization always uses an AffectContainers transaction for a
    /// materialized hierarchy. Descendant/container intersections are
    /// expected containment, not node overlaps.
    pub(super) fn gap_transaction_affects_containers(&self) -> bool {
        self.nodes.iter().any(|node| node.is_container) && self.transaction_containment_is_valid()
    }

    pub(super) fn is_within_max_size(&self) -> bool {
        let nodes = self.graph_node_order();
        let mut minimum = Point {
            x: f64::INFINITY,
            y: f64::INFINITY,
        };
        let mut maximum = Point {
            x: f64::NEG_INFINITY,
            y: f64::NEG_INFINITY,
        };
        for node in nodes {
            let Some(position) = self.position(node) else {
                continue;
            };
            let size = self.active_node_size(node);
            minimum.x = minimum.x.min(position.x);
            minimum.y = minimum.y.min(position.y);
            maximum.x = maximum.x.max(position.x + size.width);
            maximum.y = maximum.y.max(position.y + size.height);
        }
        !minimum.x.is_finite()
            || (maximum.x - minimum.x <= 30_000.0 && maximum.y - minimum.y <= 30_000.0)
    }

    /// Recovered `Transaction.repositionContainers`: transactions with
    /// `AffectContainers` wrap positioned containers from smallest to largest
    /// after applying their operation. Wrapping changes the container, not its
    /// children, so trial rotations remain legal while every hierarchy level
    /// is refit around the resulting absolute child geometry.
    pub(super) fn reposition_ordinary_containers(&mut self) {
        self.reposition_ordinary_containers_with_sync(true);
    }

    /// Refit ordinary containers without publishing aggregate vessel/member
    /// synchronization. TALA's Transaction.Commit validates the refitted
    /// ordinary boxes first and calls Graph.syncClusters/syncSequences only
    /// after those checks succeed.
    pub(super) fn reposition_ordinary_containers_without_sync(&mut self) {
        self.reposition_ordinary_containers_with_sync(false);
    }

    /// Refit a transpose trial in Go's container order. Transaction.Commit
    /// rejects as soon as a wrapped container is invalid, before a later
    /// parent can move that container and make the final graph look valid.
    pub(super) fn reposition_ordinary_containers_without_sync_sequential(&mut self) -> bool {
        self.reposition_ordinary_containers_with_sync_and_validation(false, true)
    }

    fn reposition_ordinary_containers_with_sync(&mut self, sync_aggregates: bool) {
        let _ =
            self.reposition_ordinary_containers_with_sync_and_validation(sync_aggregates, false);
    }

    fn reposition_ordinary_containers_with_sync_and_validation(
        &mut self,
        sync_aggregates: bool,
        validate_external_each_step: bool,
    ) -> bool {
        #[cfg(feature = "diagnostic-traces")]
        crate::internal::call_counts::count!(RefitContainers);
        if crate::internal::trace_env_enabled("TALA_TRACE_GRID_CARRIERS") {
            let target_ids = [1_149_337_423_i64, 1_782_109_120_i64];
            let active = self
                .nodes
                .iter()
                .filter(|node| target_ids.contains(&node.tala_id))
                .map(|node| {
                    (
                        "active",
                        node.tala_id,
                        node.position,
                        node.rect.size,
                        node.is_container,
                    )
                })
                .collect::<Vec<_>>();
            let external = self
                .transaction_external_containers
                .iter()
                .filter(|node| target_ids.contains(&node.tala_id))
                .map(|node| {
                    (
                        "external",
                        node.tala_id,
                        node.position,
                        node.rect.size,
                        node.is_container,
                    )
                })
                .collect::<Vec<_>>();
            let children = self
                .transaction_external_container_children
                .iter()
                .flat_map(|(parent, children)| {
                    children
                        .iter()
                        .filter(|node| target_ids.contains(&node.tala_id))
                        .map(move |node| {
                            (
                                "child",
                                *parent,
                                node.tala_id,
                                node.position,
                                node.rect.size,
                                node.is_container,
                            )
                        })
                })
                .collect::<Vec<_>>();
            let keys = self
                .transaction_external_container_children
                .keys()
                .filter(|key| target_ids.contains(key) || **key == 233_611_931)
                .copied()
                .collect::<Vec<_>>();
            let target_children = self
                .transaction_external_container_children
                .iter()
                .filter(|(parent, _)| target_ids.contains(parent))
                .map(|(parent, children)| {
                    (
                        *parent,
                        children
                            .iter()
                            .map(|node| {
                                (
                                    node.tala_id,
                                    node.position,
                                    node.rect.size,
                                    node.is_container,
                                )
                            })
                            .collect::<Vec<_>>(),
                    )
                })
                .collect::<Vec<_>>();
            eprintln!(
                "GRID_CARRIERS_RUST sync={} active={:?} external={:?} keys={:?} children={:?} target_children={:?}",
                sync_aggregates, active, external, keys, children, target_children
            );
        }
        let mut containers = self
            .nodes
            .iter()
            .filter(|node| node.is_container && node.position.is_some())
            .map(|node| node.input_id)
            .collect::<Vec<_>>();
        containers.sort_by(|left, right| {
            let left_node = &self.nodes[left.0 as usize];
            let right_node = &self.nodes[right.0 as usize];
            let left_area = left_node.rect.size.width * left_node.rect.size.height;
            let right_area = right_node.rect.size.width * right_node.rect.size.height;
            left_area
                .total_cmp(&right_area)
                .then_with(|| left_node.tala_id.cmp(&right_node.tala_id))
        });
        let has_active_containers = !containers.is_empty();

        let trace_wrap_target = crate::internal::trace_env_value("TALA_TRACE_WRAP_CHILDREN");
        let trace_wrap_pass_active = !crate::internal::trace_env_enabled("TALA_TRACE_SIZED_PASS")
            || crate::internal::trace_env_enabled("TALA_TRACE_SIZED_PASS_ACTIVE");
        let trace_wrap_all = trace_wrap_target.as_deref() == Some("all");
        let trace_wrap_tala_id = trace_wrap_target
            .as_deref()
            .and_then(|target| target.parse::<i64>().ok());
        // Go keeps active containers and projected descendants in one
        // Graph.Containers map, then refits that shared set smallest to
        // largest. Refit projected descendants before active ancestors so
        // an enclosing active container sees their final wrapped bounds.
        let active_geometry_before_refit = has_active_containers.then(|| {
            self.nodes
                .iter()
                .map(|node| (node.position, node.rect.size))
                .collect::<Vec<_>>()
        });
        if !self.reposition_external_containers(validate_external_each_step) {
            return false;
        }
        for container in containers {
            let container_tala_id = self.nodes[container.0 as usize].tala_id;
            let trace_wrap = trace_wrap_pass_active
                && (trace_wrap_all || trace_wrap_tala_id == Some(container_tala_id));
            if trace_wrap {
                let node = &self.nodes[container.0 as usize];
                eprintln!(
                    "WRAP_CHILDREN_RUST before container={} pos={:?} size={},{} external={}",
                    container_tala_id,
                    node.position,
                    node.rect.size.width,
                    node.rect.size.height,
                    self.transaction_external_container_children
                        .contains_key(&container_tala_id)
                );
            }
            if let Some((content, top_left, bottom_right)) = self
                .transaction_external_container_children
                .get(&container_tala_id)
                .and_then(|children| {
                    if trace_wrap {
                        eprintln!(
                            "WRAP_CHILDREN_RUST children container={} {:?}",
                            container_tala_id,
                            children
                                .iter()
                                .map(|child| {
                                    (
                                        child.tala_id,
                                        child.position,
                                        child.rect.size,
                                        child.is_container,
                                        child.container,
                                    )
                                })
                                .collect::<Vec<_>>()
                        );
                    }
                    let (mut top_left, mut bottom_right) =
                        Self::fixed_external_node_bounds(children)?;
                    for child in children {
                        let Some(label_size) = child.label_size else {
                            continue;
                        };
                        let Some(position) = child.position else {
                            continue;
                        };
                        if position.x != top_left.x
                            && position.x + child.rect.size.width != bottom_right.x
                        {
                            continue;
                        }
                        if label_size.width > child.rect.size.width {
                            let overhang = (label_size.width - child.rect.size.width) * 0.5;
                            top_left.x = top_left.x.min((position.x - overhang).floor());
                            bottom_right.x = bottom_right
                                .x
                                .max((position.x + child.rect.size.width + overhang).ceil());
                        }
                    }
                    Some((
                        Size {
                            width: bottom_right.x - top_left.x,
                            height: bottom_right.y - top_left.y,
                        },
                        top_left,
                        bottom_right,
                    ))
                })
            {
                let padding = self.shape_fit_padding(container);
                let fitted = self.shape_dimensions_to_fit(container, content, padding);
                self.nodes[container.0 as usize].rect.size = fitted;
                let placement = self
                    .shape_inside_placement_absolute(container, content, padding)
                    .expect("positioned transaction container");
                let current = self
                    .position(container)
                    .expect("positioned transaction container");
                if trace_wrap {
                    eprintln!(
                        "WRAP_DETAIL_RUST node={} current={:?} content={:?} padding={:?} fitted={:?} placement={:?}",
                        container_tala_id, current, content, padding, fitted, placement
                    );
                }
                let position = Point {
                    x: current.x + (top_left.x - placement.x),
                    y: current.y + (top_left.y - placement.y),
                };
                self.adjust_projected_offsets_for_refit(
                    container,
                    Point {
                        x: position.x - current.x,
                        y: position.y - current.y,
                    },
                );
                self.nodes[container.0 as usize].position = Some(position);
                self.nodes[container.0 as usize].rect.origin = position;
                let updated = &self.nodes[container.0 as usize];
                for siblings in self.transaction_external_container_children.values_mut() {
                    for child in siblings {
                        if child.tala_id == container_tala_id {
                            child.clone_from(updated);
                        }
                    }
                }
                if trace_wrap {
                    eprintln!(
                        "WRAP_CHILDREN_RUST after container={} pos={:?} size={},{} tl={:?} br={:?}",
                        container_tala_id,
                        updated.position,
                        updated.rect.size.width,
                        updated.rect.size.height,
                        top_left,
                        bottom_right
                    );
                }
                continue;
            }
            let children = self.container_node_order(Some(container));
            if trace_wrap {
                eprintln!(
                    "WRAP_CHILDREN_RUST children container={} {:?}",
                    container_tala_id,
                    children
                        .iter()
                        .map(|child| {
                            let node = &self.nodes[child.0 as usize];
                            (
                                node.tala_id,
                                node.position,
                                node.rect.size,
                                node.is_container,
                                self.active_node_is_aggregate(*child),
                                self.nodes[self.active_aggregate_owner(*child).0 as usize].tala_id,
                                node.cluster,
                                node.sequence,
                                self.active_cluster_index(*child),
                                self.active_sequence_index(*child),
                                self.active_node_size(*child),
                                self.active_node_position(*child),
                            )
                        })
                        .collect::<Vec<_>>()
                );
            }
            let Some((mut top_left, mut bottom_right)) =
                self.transaction_container_bounds(&children)
            else {
                continue;
            };
            // Node.wrapChildren calls expandForLabels after
            // getFixedBoundingBox. A boundary child whose label is wider than
            // its box expands both horizontal bounds around the child's
            // center, even when the label's placement did not extend the
            // ordinary node bounding-box branch.
            for child in children.iter().copied() {
                if self.active_node_is_aggregate(child) {
                    continue;
                }
                let node = &self.nodes[child.0 as usize];
                let Some(label_size) = node.label_size else {
                    continue;
                };
                let Some(position) = self.position(child) else {
                    continue;
                };
                if position.x != top_left.x && position.x + node.rect.size.width != bottom_right.x {
                    continue;
                }
                if label_size.width > node.rect.size.width {
                    let overhang = (label_size.width - node.rect.size.width) * 0.5;
                    top_left.x = top_left.x.min((position.x - overhang).floor());
                    bottom_right.x = bottom_right
                        .x
                        .max((position.x + node.rect.size.width + overhang).ceil());
                }
            }
            // Transaction.repositionContainers calls Node.wrapChildren from
            // smallest to largest. wrapChildren derives label-aware padding
            // from the pre-fit box, fits through the shape implementation,
            // then moves only the container around its unchanged children.
            let padding = self.shape_fit_padding(container);
            let content = Size {
                width: bottom_right.x - top_left.x,
                height: bottom_right.y - top_left.y,
            };
            let fitted = self.shape_dimensions_to_fit(container, content, padding);
            self.nodes[container.0 as usize].rect.size = fitted;
            let placement = self
                .shape_inside_placement_absolute(container, content, padding)
                .expect("positioned transaction container");
            let current = self
                .position(container)
                .expect("positioned transaction container");
            if trace_wrap {
                eprintln!(
                    "WRAP_DETAIL_RUST node={} current={:?} content={:?} padding={:?} fitted={:?} placement={:?}",
                    container_tala_id, current, content, padding, fitted, placement
                );
            }
            let position = Point {
                x: current.x + (top_left.x - placement.x),
                y: current.y + (top_left.y - placement.y),
            };
            self.adjust_projected_offsets_for_refit(
                container,
                Point {
                    x: position.x - current.x,
                    y: position.y - current.y,
                },
            );
            self.nodes[container.0 as usize].position = Some(position);
            self.nodes[container.0 as usize].rect.origin = position;
            if trace_wrap {
                let node = &self.nodes[container.0 as usize];
                eprintln!(
                    "WRAP_CHILDREN_RUST after container={} pos={:?} size={},{} tl={:?} br={:?}",
                    container_tala_id,
                    node.position,
                    node.rect.size.width,
                    node.rect.size.height,
                    top_left,
                    bottom_right
                );
            }
        }
        // With no in-scope containers, Go's unified Graph.Containers pass
        // wraps each projected external container exactly once. A second
        // projected-only pass is not harmless: shape placement can be
        // translation-sensitive, so wrapping the same children twice may
        // shift an external owner by a cell even though no child changed.
        // Refit projected parents only when an active node changed, so an
        // unrelated active container cannot trigger a second wrap of fixed
        // external children.
        let changed_active_tala_ids = active_geometry_before_refit
            .as_ref()
            .map(|geometry_before_refit| {
                self.nodes
                    .iter()
                    .enumerate()
                    .filter_map(|(index, node)| {
                        geometry_before_refit
                            .get(index)
                            .is_none_or(|(position, size)| {
                                node.position != *position || node.rect.size != *size
                            })
                            .then_some(node.tala_id)
                    })
                    .collect::<BTreeSet<_>>()
            })
            .unwrap_or_default();
        if has_active_containers
            && !changed_active_tala_ids.is_empty()
            && !self.reposition_external_containers_for_changed_geometry(
                validate_external_each_step,
                &changed_active_tala_ids,
            )
        {
            return false;
        }
        // Transaction.Commit immediately follows repositionContainers with
        // Graph.syncClusters. Without that synchronization, independently
        // wrapped members of a non-fixed cluster silently lose the dimensions
        // established by Cluster.Resize.
        if sync_aggregates {
            self.sync_clusters();
            // Recovered TALA commits publish cluster state first and sequence
            // state second. Keep the same order after repositioning so a
            // retained sequence member cannot be mistaken for the active
            // cluster vessel during the enclosing-container refit.
            self.sync_sequences();
        }
        true
    }

    fn external_container_padding(node: &ArenaNode) -> Insets {
        Self::projected_container_padding(node, &[], false)
    }

    /// Return the source-compatible inset used when a container's direct
    /// children are positioned in a diagnostic projection.
    #[cfg(feature = "diagnostic-traces")]
    pub(crate) fn trace_source_container_padding(&self, parent_tala_id: i64) -> Insets {
        let Some(parent) = self
            .nodes
            .iter()
            .find(|node| node.tala_id == parent_tala_id)
        else {
            return Insets::uniform(0.0);
        };
        let children = self
            .trace_container_children
            .get(&parent_tala_id)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        Self::projected_container_padding(parent, children, true)
    }

    #[cfg(not(feature = "diagnostic-traces"))]
    #[inline(always)]
    pub(crate) fn trace_source_container_padding(&self, _parent_tala_id: i64) -> Insets {
        Insets::uniform(0.0)
    }

    /// Recovered `Graph.getContainerPadding` for a node retained in the
    /// induced graph's external pointer projection.
    fn projected_container_padding(
        node: &ArenaNode,
        children: &[ArenaNode],
        consider_children: bool,
    ) -> Insets {
        let geometry_is_in_insets = match node.shape {
            ShapeKind::Cylinder => {
                node.content_insets.top >= 108.0 && node.content_insets.bottom >= 84.0
            }
            ShapeKind::Package => node.content_insets.top > node.content_insets.bottom,
            _ => false,
        };
        let explicit =
            if node.grid_rows.is_some() || node.grid_columns.is_some() || geometry_is_in_insets {
                Insets {
                    top: if node.content_insets.top == 60.0 {
                        0.0
                    } else {
                        node.content_insets.top
                    },
                    right: if node.content_insets.right == 60.0 {
                        0.0
                    } else {
                        node.content_insets.right
                    },
                    bottom: if node.content_insets.bottom == 60.0 {
                        0.0
                    } else {
                        node.content_insets.bottom
                    },
                    left: if node.content_insets.left == 60.0 {
                        0.0
                    } else {
                        node.content_insets.left
                    },
                }
            } else {
                node.node_padding
            };
        let mut padding_floor = if node.shape == ShapeKind::Circle {
            15.0
        } else {
            60.0
        };
        if node.has_icon && node.shape != ShapeKind::Image {
            padding_floor = 74.0;
        }
        let mut spacing = Insets::uniform(60.0);
        if let Some(label) = node
            .label_size
            .filter(|_| !Self::label_position_is_outside(node.label_position))
        {
            let label_width = label.width + 10.0;
            let label_height = label.height + 10.0;
            match node.label_position {
                LabelPosition::InsideTopLeft
                | LabelPosition::InsideTopCenter
                | LabelPosition::InsideTopRight => spacing.top = spacing.top.max(label_height),
                LabelPosition::InsideBottomLeft
                | LabelPosition::InsideBottomCenter
                | LabelPosition::InsideBottomRight => {
                    spacing.bottom = spacing.bottom.max(label_height)
                }
                LabelPosition::InsideMiddleLeft => spacing.left = spacing.left.max(label_width),
                LabelPosition::InsideMiddleRight => spacing.right = spacing.right.max(label_width),
                _ => {}
            }
            let min_width = spacing.left + label_width + spacing.right;
            if min_width > node.rect.size.width {
                let extra = ((min_width - node.rect.size.width) * 0.5).ceil();
                spacing.left = spacing.left.max(extra);
                spacing.right = spacing.right.max(extra);
            }
            let min_height = spacing.top + label_height + spacing.bottom;
            if min_height > node.rect.size.height {
                let extra = ((min_height - node.rect.size.height) * 0.5).ceil();
                spacing.top = spacing.top.max(extra);
                spacing.bottom = spacing.bottom.max(extra);
            }
        }
        if consider_children {
            let mut child_margin = Insets::uniform(0.0);
            let mut child_has_icon = false;
            for child in children {
                child_margin.top = child_margin.top.max(child.layout_margins.top);
                child_margin.right = child_margin.right.max(child.layout_margins.right);
                child_margin.bottom = child_margin.bottom.max(child.layout_margins.bottom);
                child_margin.left = child_margin.left.max(child.layout_margins.left);
                child_has_icon |= child.has_icon
                    && child.icon_position.is_none()
                    && child.shape != ShapeKind::Image;
            }
            spacing.top = spacing.top.max(explicit.top + child_margin.top);
            spacing.right = spacing.right.max(explicit.right + child_margin.right);
            spacing.bottom = spacing.bottom.max(explicit.bottom + child_margin.bottom);
            spacing.left = spacing.left.max(explicit.left + child_margin.left);
            if child_has_icon {
                padding_floor = 74.0;
            }
        }
        if node.shape == ShapeKind::Circle {
            spacing.top *= 0.25;
            spacing.right *= 0.25;
            spacing.bottom *= 0.25;
            spacing.left *= 0.25;
        }
        spacing.top = spacing.top.max(explicit.top).max(padding_floor);
        spacing.right = spacing.right.max(explicit.right).max(padding_floor);
        spacing.bottom = spacing.bottom.max(explicit.bottom).max(padding_floor);
        spacing.left = spacing.left.max(explicit.left).max(padding_floor);
        spacing
    }

    fn reposition_external_containers(&mut self, validate_each_step: bool) -> bool {
        self.reposition_external_containers_filtered(validate_each_step, None)
    }

    fn reposition_external_containers_for_changed_geometry(
        &mut self,
        validate_each_step: bool,
        changed_tala_ids: &BTreeSet<i64>,
    ) -> bool {
        self.reposition_external_containers_filtered(validate_each_step, Some(changed_tala_ids))
    }

    fn reposition_external_containers_filtered(
        &mut self,
        validate_each_step: bool,
        changed_tala_ids: Option<&BTreeSet<i64>>,
    ) -> bool {
        if crate::internal::trace_env_enabled("TALA_TRACE_GRID_CARRIERS") {
            let target_ids = [1_149_337_423_i64, 1_782_109_120_i64];
            let external = self
                .transaction_external_containers
                .iter()
                .filter(|node| target_ids.contains(&node.tala_id))
                .map(|node| {
                    (
                        node.tala_id,
                        node.position,
                        node.rect.size,
                        node.is_container,
                    )
                })
                .collect::<Vec<_>>();
            let children = self
                .transaction_external_container_children
                .iter()
                .flat_map(|(parent, children)| {
                    children
                        .iter()
                        .filter(|node| target_ids.contains(&node.tala_id))
                        .map(move |node| {
                            (
                                *parent,
                                node.tala_id,
                                node.position,
                                node.rect.size,
                                node.is_container,
                            )
                        })
                })
                .collect::<Vec<_>>();
            eprintln!(
                "GRID_CARRIERS_RUST external-reposition external={:?} children={:?}",
                external, children
            );
        }
        if self.transaction_external_container_children.is_empty() {
            return true;
        }
        let geometry_before_refit = changed_tala_ids.map(|_| {
            self.transaction_external_containers
                .iter()
                .map(|container| (container.position, container.rect.size))
                .collect::<Vec<_>>()
        });
        let mut changed_geometry = changed_tala_ids.cloned();
        // Both position maps are only probed. Size them for every node up
        // front instead of rehashing while the filtered inserts grow them;
        // inserts keep their order, so a repeated TALA ID keeps its last box.
        let mut anchor_positions =
            IdHashMap::with_capacity_and_hasher(self.nodes.len(), Default::default());
        anchor_positions.extend(
            self.nodes
                .iter()
                .filter_map(|node| Some((node.tala_id, node.position?))),
        );
        let anchored_position = |node: &ArenaNode| {
            node.transaction_anchor_tala_id
                .zip(node.transaction_anchor_offset)
                .and_then(|(anchor, offset)| {
                    anchor_positions.get(&anchor).map(|position| Point {
                        x: position.x + offset.x,
                        y: position.y + offset.y,
                    })
                })
                .or(node.position)
        };
        // Graph.Containers and Graph.Nodes contain the same *Node pointers in
        // the Go implementation.  Our induced arena stores cloned
        // projections, so refresh projections for nodes that are live in this
        // transaction's node slice before applying anchor fallbacks.  Without
        // this, a local optimizer move updates `self.nodes` while an
        // external carrier still sees the stale absolute position captured
        // when the scope was constructed (and wraps around that stale frame).
        let mut active_positions =
            IdHashMap::with_capacity_and_hasher(self.node_order.len(), Default::default());
        active_positions.extend(self.node_order.iter().filter_map(|node_id| {
            self.nodes[node_id.0 as usize]
                .position
                .map(|position| (self.nodes[node_id.0 as usize].tala_id, position))
        }));
        for container in &mut self.transaction_external_containers {
            if let Some(position) = active_positions.get(&container.tala_id).copied() {
                container.position = Some(position);
                container.rect.origin = position;
            }
        }
        for children in self.transaction_external_container_children.values_mut() {
            for child in children {
                if let Some(position) = active_positions.get(&child.tala_id).copied() {
                    child.position = Some(position);
                    child.rect.origin = position;
                }
            }
        }
        let trace_external_reposition_target =
            crate::internal::trace_env_value("TALA_TRACE_EXTERNAL_REPOSITION");
        let trace_external_reposition_all =
            trace_external_reposition_target.as_deref() == Some("all");
        let trace_external_reposition_tala_id = trace_external_reposition_target
            .as_deref()
            .and_then(|target| target.parse::<i64>().ok());
        if trace_external_reposition_tala_id == Some(5) {
            eprintln!(
                "EXTERNAL_REPOSITION_RUST active_nodes={:?}",
                self.nodes
                    .iter()
                    .filter(|node| (6..=10).contains(&node.tala_id))
                    .map(|node| (
                        node.tala_id,
                        node.position,
                        node.container.map(|id| self.nodes[id.0 as usize].tala_id)
                    ))
                    .collect::<Vec<_>>()
            );
        }
        for container in &mut self.transaction_external_containers {
            if trace_external_reposition_all
                || trace_external_reposition_tala_id == Some(container.tala_id)
            {
                eprintln!(
                    "EXTERNAL_REPOSITION_RUST before container={} position={:?} hasChildren={} children={:?}",
                    container.tala_id,
                    container.position,
                    self.transaction_external_container_children
                        .contains_key(&container.tala_id),
                    self.transaction_external_container_children
                        .get(&container.tala_id)
                        .map(|children| children
                            .iter()
                            .map(|child| (child.tala_id, child.position, child.rect.size))
                            .collect::<Vec<_>>())
                );
                eprintln!(
                    "EXTERNAL_REPOSITION_RUST anchor container={} anchor={:?} offset={:?}",
                    container.tala_id,
                    container.transaction_anchor_tala_id,
                    container.transaction_anchor_offset
                );
            }
            // An external projection may still have an anchor to its owning
            // container. A live node in this induced arena takes precedence;
            // otherwise resolve the anchor so repeated transactions follow
            // a parent that moved after the projection was created.
            container.position = active_positions
                .get(&container.tala_id)
                .copied()
                .or_else(|| anchored_position(container))
                .or(container.position);
            // Go skips nil containers during refitting and validation.
            // Positioned descendants do not materialize their parent's
            // TopLeft, regardless of the number of projected graph nodes.
            container.rect.origin = container.position.unwrap_or_default();
        }
        for children in self.transaction_external_container_children.values_mut() {
            for child in children {
                child.position = active_positions
                    .get(&child.tala_id)
                    .copied()
                    .or_else(|| anchored_position(child))
                    .or(child.position);
                child.rect.origin = child.position.unwrap_or_default();
            }
        }

        let mut order = (0..self.transaction_external_containers.len()).collect::<Vec<_>>();
        order.sort_by(|&left, &right| {
            let left = &self.transaction_external_containers[left];
            let right = &self.transaction_external_containers[right];
            (left.rect.size.width * left.rect.size.height)
                .total_cmp(&(right.rect.size.width * right.rect.size.height))
                .then_with(|| left.tala_id.cmp(&right.tala_id))
        });
        for index in order {
            // Transaction.repositionContainers skips every nil TopLeft,
            // regardless of scope size or positioned descendants.
            let container = &self.transaction_external_containers[index];
            let Some(current_position) = container.position else {
                continue;
            };
            if self
                .transaction_external_container_children
                .contains_key(&container.tala_id)
            {
                let has_positioned_child = self
                    .transaction_external_container_children
                    .get(&container.tala_id)
                    .is_some_and(|children| children.iter().any(|child| child.position.is_some()));
                if container.transaction_validation_only
                    || (self.projected_transaction_nodes.len() > 64 && !has_positioned_child)
                {
                    continue;
                }
            }
            let tala_id = self.transaction_external_containers[index].tala_id;
            if let Some(changed_geometry) = changed_geometry.as_ref() {
                let child_geometry_changed = self
                    .transaction_external_container_children
                    .get(&tala_id)
                    .is_some_and(|children| {
                        children
                            .iter()
                            .any(|child| changed_geometry.contains(&child.tala_id))
                    });
                let anchor_geometry_changed = self.transaction_external_containers[index]
                    .transaction_anchor_tala_id
                    .is_some_and(|anchor| changed_geometry.contains(&anchor));
                if !child_geometry_changed && !anchor_geometry_changed {
                    continue;
                }
            }
            let trace_external_reposition =
                trace_external_reposition_all || trace_external_reposition_tala_id == Some(tala_id);
            let Some((content, top_left, bottom_right)) = self
                .transaction_external_container_children
                .get(&tala_id)
                .and_then(|children| {
                    let (mut top_left, mut bottom_right) =
                        Self::fixed_external_node_bounds(children)?;
                    for child in children {
                        let Some(label_size) = child.label_size else {
                            continue;
                        };
                        let Some(position) = child.position else {
                            continue;
                        };
                        if position.x != top_left.x
                            && position.x + child.rect.size.width != bottom_right.x
                        {
                            continue;
                        }
                        if label_size.width > child.rect.size.width {
                            let overhang = (label_size.width - child.rect.size.width) * 0.5;
                            top_left.x = top_left.x.min((position.x - overhang).floor());
                            bottom_right.x = bottom_right
                                .x
                                .max((position.x + child.rect.size.width + overhang).ceil());
                        }
                    }
                    Some((
                        Size {
                            width: bottom_right.x - top_left.x,
                            height: bottom_right.y - top_left.y,
                        },
                        top_left,
                        bottom_right,
                    ))
                })
            else {
                continue;
            };
            // A shared Go container may be captured after its nested scope
            // has materialized. The aliases can still carry absolute
            // coordinates, but Go's shared *Node pointer is refit by the
            // enclosing transaction before the next lifecycle snapshot.
            // The fit operation is translation-aware, so apply it to every
            // external container rather than suppressing the update based on
            // the coordinate frame marker.
            let container = &mut self.transaction_external_containers[index];
            let padding = Self::external_container_padding(container);
            container.rect.size = Self::shape_dimensions_to_fit_node(container, content, padding);
            let current = current_position;
            let placement =
                Self::shape_inside_placement_at_node(container, content, padding, current);
            let position = Point {
                x: current.x + (top_left.x - placement.x),
                y: current.y + (top_left.y - placement.y),
            };
            container.position = Some(position);
            container.rect.origin = position;
            if let Some(anchor) = container.transaction_anchor_tala_id
                && let Some(anchor_position) = anchor_positions.get(&anchor)
            {
                container.transaction_anchor_offset = Some(Point {
                    x: position.x - anchor_position.x,
                    y: position.y - anchor_position.y,
                });
            }
            // Nothing below writes this container again, so its aliases and
            // the validation read the live entry instead of a full copy.
            let updated_size = container.rect.size;
            if trace_external_reposition {
                eprintln!(
                    "EXTERNAL_REPOSITION_RUST after container={} position={:?} size={:?} padding={:?} topLeft={:?} bottomRight={:?}",
                    container.tala_id,
                    container.position,
                    container.rect.size,
                    padding,
                    top_left,
                    bottom_right
                );
            }
            // Go's external container projections alias the live node pointer:
            // wrapping moves the container but leaves its children in place.
            // Keep our flattened child offsets relative to that moved owner so
            // endpoint restoration still resolves to each child's unchanged
            // absolute position during the transaction score.
            if let Some(owner) = self
                .nodes
                .iter()
                .find(|node| node.tala_id == tala_id)
                .map(|node| node.input_id)
            {
                self.adjust_projected_offsets_for_refit(
                    owner,
                    Point {
                        x: position.x - current.x,
                        y: position.y - current.y,
                    },
                );
            }
            // The recovered transaction stores the same *Node pointer in both
            // Graph.Nodes and Graph.Containers.  External projections are
            // arena clones, so publish the refit back to the active owner as
            // well; otherwise relative projected offsets are measured against
            // the stale active origin and lose a pixel whenever wrapChildren
            // moves the retained container by one cell.
            if let Some(active) = self.nodes.iter_mut().find(|node| node.tala_id == tala_id) {
                active.position = Some(position);
                active.rect.origin = position;
                active.rect.size = updated_size;
            }
            let updated = &self.transaction_external_containers[index];
            for siblings in self.transaction_external_container_children.values_mut() {
                for child in siblings {
                    if child.tala_id == tala_id {
                        child.clone_from(updated);
                    }
                }
            }
            for members in self.transaction_external_aggregate_children.values_mut() {
                for member in members {
                    if member.tala_id == tala_id {
                        member.clone_from(updated);
                    }
                }
            }
            // Every flattened endpoint/obstruction cache is another view of
            // the same recovered *Node.Box. Refit dimensions must be visible
            // before the immediately following cluster/sequence sync.
            self.set_projected_cache_size(tala_id, updated_size);
            if let (Some(changed_geometry), Some(geometry_before_refit)) =
                (changed_geometry.as_mut(), geometry_before_refit.as_ref())
            {
                let geometry_changed = geometry_before_refit.get(index).is_none_or(
                    |(before_position, before_size)| {
                        Some(position) != *before_position || updated_size != *before_size
                    },
                );
                if geometry_changed {
                    changed_geometry.insert(tala_id);
                }
            }
            if validate_each_step
                && !self.external_container_position_is_valid(
                    &self.transaction_external_containers[index],
                    position,
                    &BTreeSet::new(),
                )
            {
                return false;
            }
        }
        true
    }

    pub(super) fn transaction_container_bounds(&self, nodes: &[NodeId]) -> Option<(Point, Point)> {
        let mut top_left = Point {
            x: f64::INFINITY,
            y: f64::INFINITY,
        };
        let mut bottom_right = Point {
            x: f64::NEG_INFINITY,
            y: f64::NEG_INFINITY,
        };
        let ordinary = nodes
            .iter()
            .copied()
            // Every member of an active sequence/cluster is an aggregate
            // endpoint, not only the carrier whose `active_aggregate_members`
            // lookup succeeds.  The stable arena retains hidden members that
            // TALA removes from `Graph.Containers`; letting a non-carrier
            // member into the ordinary bounds reintroduces its stale box and
            // changes the enclosing container refit.
            .filter(|node| !self.active_node_is_aggregate(*node))
            .collect::<Vec<_>>();
        if !ordinary.is_empty() {
            // Nodes.getFixedBoundingBox passes the complete sibling slice to
            // every Node.getBoundingBox call. Outside-label boundary padding
            // therefore depends on siblings whose ordinary box reaches
            // farther on that side; evaluating one child at a time falsely
            // makes every labeled child a graph-boundary node.
            let bounds = self.fixed_node_bounds(&ordinary)?;
            top_left = bounds.0;
            bottom_right = bounds.1;
        }
        for node in nodes.iter().copied().filter(|node| {
            self.active_node_is_aggregate(*node) && self.active_aggregate_owner(*node) == *node
        }) {
            let position = self.active_node_position(node)?;
            // Temporary sequence/cluster vessels are plain recovered
            // `NewNode` boxes. Their stable carrier member's rendering
            // modifiers must not leak into the vessel envelope.
            let size = self.active_node_size(node);
            let bounds = (
                position,
                Point {
                    x: position.x + size.width,
                    y: position.y + size.height,
                },
            );
            top_left.x = top_left.x.min(bounds.0.x);
            top_left.y = top_left.y.min(bounds.0.y);
            bottom_right.x = bottom_right.x.max(bounds.1.x);
            bottom_right.y = bottom_right.y.max(bounds.1.y);
        }
        top_left.x.is_finite().then_some((top_left, bottom_right))
    }

    pub(super) fn exact_overlap_pairs(&self) -> BTreeSet<(NodeId, NodeId)> {
        let mut pairs = BTreeSet::new();
        let nodes = self.graph_node_order();
        for (index, left) in nodes.iter().copied().enumerate() {
            let Some(a) = self.active_node_position(left) else {
                continue;
            };
            let left_size = self.active_node_size(left);
            for right in nodes.iter().copied().skip(index + 1) {
                let Some(b) = self.active_node_position(right) else {
                    continue;
                };
                let right_size = self.active_node_size(right);
                if a.x < b.x + right_size.width
                    && b.x < a.x + left_size.width
                    && a.y < b.y + right_size.height
                    && b.y < a.y + left_size.height
                {
                    pairs.insert(if left < right {
                        (left, right)
                    } else {
                        (right, left)
                    });
                }
            }
        }
        pairs
    }

    /// Exact overlaps are a subset of spacing overlaps: every recovered
    /// spacing delta is non-negative, so a pair whose boxes overlap without
    /// clearance is necessarily present in `existing_overlaps`. Gap trials
    /// already paid for that spacing scan; filter it instead of repeating the
    /// complete quadratic node-pair walk.
    pub(super) fn exact_overlap_pairs_from(
        &self,
        existing_overlaps: &BTreeSet<(NodeId, NodeId)>,
    ) -> BTreeSet<(NodeId, NodeId)> {
        existing_overlaps
            .iter()
            .copied()
            .filter(|&(left, right)| {
                let Some(left_position) = self.active_node_position(left) else {
                    return false;
                };
                let Some(right_position) = self.active_node_position(right) else {
                    return false;
                };
                let left_size = self.active_node_size(left);
                let right_size = self.active_node_size(right);
                left_position.x < right_position.x + right_size.width
                    && right_position.x < left_position.x + left_size.width
                    && left_position.y < right_position.y + right_size.height
                    && right_position.y < left_position.y + left_size.height
            })
            .collect()
    }

    pub(super) fn existing_spacing_overlap_became_exact(
        &self,
        existing_overlaps: &BTreeSet<(NodeId, NodeId)>,
        existing_exact_overlaps: &BTreeSet<(NodeId, NodeId)>,
    ) -> bool {
        existing_overlaps
            .difference(existing_exact_overlaps)
            .any(|&(left, right)| self.spacing_overlap_pair_is_exact(left, right))
    }

    /// The same recovered spacing-to-exact predicate, restricted to pairs
    /// touched by a candidate transaction. An unchanged pair cannot cross
    /// from padded overlap into exact box overlap.
    pub(super) fn existing_spacing_overlap_became_exact_for_moved_nodes(
        &self,
        existing_overlaps: &BTreeSet<(NodeId, NodeId)>,
        existing_exact_overlaps: &BTreeSet<(NodeId, NodeId)>,
        moved: &BTreeSet<NodeId>,
    ) -> bool {
        let nodes = self.graph_node_order();
        let mut checked = BTreeSet::new();
        for (moved_index, moved_node) in nodes
            .iter()
            .copied()
            .enumerate()
            .filter(|(_, node)| moved.contains(node))
        {
            for (other_index, other) in nodes.iter().copied().enumerate() {
                if moved_index == other_index {
                    continue;
                }
                let (left, right) = if moved_index < other_index {
                    (moved_node, other)
                } else {
                    (other, moved_node)
                };
                let pair = if left < right {
                    (left, right)
                } else {
                    (right, left)
                };
                if !checked.insert(pair)
                    || !existing_overlaps.contains(&pair)
                    || existing_exact_overlaps.contains(&pair)
                {
                    continue;
                }
                if self.spacing_overlap_pair_is_exact(left, right) {
                    if crate::internal::trace_env_enabled("TALA_TRACE_BINPACK_OVERLAP_260")
                        && moved.iter().any(|node| {
                            matches!(
                                self.nodes[node.0 as usize].tala_id,
                                2600858170
                                    | 1308591071
                                    | 4248473833
                                    | 3802298614
                                    | 365607126
                                    | 2531950148
                            )
                        })
                    {
                        eprintln!(
                            "BINPACK_EXACT_PAIR_260_RUST left={} right={} left_desc_right={} right_desc_left={} left_container={:?} right_container={:?} pos_left={:?} size_left={:?} pos_right={:?} size_right={:?}",
                            self.nodes[left.0 as usize].tala_id,
                            self.nodes[right.0 as usize].tala_id,
                            self.active_is_descendant_of(left, right),
                            self.active_is_descendant_of(right, left),
                            self.active_node_container(left)
                                .map(|node| self.nodes[node.0 as usize].tala_id),
                            self.active_node_container(right)
                                .map(|node| self.nodes[node.0 as usize].tala_id),
                            self.active_node_position(left),
                            self.active_node_size(left),
                            self.active_node_position(right),
                            self.active_node_size(right),
                        );
                    }
                    if crate::internal::trace_env_enabled("TALA_TRACE_BINPACK_PAIR")
                        && moved
                            .iter()
                            .any(|node| self.nodes[node.0 as usize].tala_id == 11)
                    {
                        eprintln!(
                            "BINPACK_EXACT_PAIR_RUST left={} right={} moved={:?} existing={} exact={} pos_left={:?} size_left={:?} pos_right={:?} size_right={:?}",
                            self.nodes[left.0 as usize].tala_id,
                            self.nodes[right.0 as usize].tala_id,
                            moved
                                .iter()
                                .map(|node| self.nodes[node.0 as usize].tala_id)
                                .collect::<Vec<_>>(),
                            existing_overlaps.contains(&pair),
                            existing_exact_overlaps.contains(&pair),
                            self.active_node_position(left),
                            self.active_node_size(left),
                            self.active_node_position(right),
                            self.active_node_size(right),
                        );
                    }
                    return true;
                }
            }
        }
        false
    }

    fn spacing_overlap_pair_is_exact(&self, left: NodeId, right: NodeId) -> bool {
        // transaction.go checks DoesOverlapExact on every saved spacing
        // overlap, including ancestor pairs. A nested trial can begin with
        // a child outside its container; refitting must not turn that saved
        // clearance overlap into an exact overlap.
        let Some(a) = self.active_node_position(left) else {
            return false;
        };
        let Some(b) = self.active_node_position(right) else {
            return false;
        };
        let left_size = self.active_node_size(left);
        let right_size = self.active_node_size(right);
        let became_exact = a.x < b.x + right_size.width
            && b.x < a.x + left_size.width
            && a.y < b.y + right_size.height
            && b.y < a.y + left_size.height;
        if became_exact && crate::internal::trace_env_enabled("TALA_TRACE_GAP_EXACT") {
            eprintln!(
                "GAP_EXACT_RUST left={} right={} left_pos={:?} left_size={:?} right_pos={:?} right_size={:?}",
                self.nodes[left.0 as usize].tala_id,
                self.nodes[right.0 as usize].tala_id,
                a,
                left_size,
                b,
                right_size,
            );
        }
        became_exact
    }

    pub(super) fn has_node_overlaps_except(&self, exceptions: &BTreeSet<(NodeId, NodeId)>) -> bool {
        for (index, left) in self.nodes.iter().enumerate() {
            let Some(a) = left.position else { continue };
            for right in self.nodes.iter().skip(index + 1) {
                if !self.same_placement_component(left.input_id, right.input_id) {
                    continue;
                }
                if exceptions.contains(&(left.input_id, right.input_id)) {
                    continue;
                }
                let Some(b) = right.position else { continue };
                // Recovered Graph.doesOverlapWithDimensions always expands
                // the candidate by Node.getDeltaTo. Scope ownership is
                // represented by placement_component above; it must not
                // silently select a second raw-box overlap model.
                let delta = self.spacing_delta_with_loops(left.input_id, right.input_id, a);
                if a.x < b.x + right.rect.size.width + delta
                    && b.x < a.x + left.rect.size.width + delta
                    && a.y < b.y + right.rect.size.height + delta
                    && b.y < a.y + left.rect.size.height + delta
                {
                    return true;
                }
            }
        }
        false
    }

    pub(super) fn global_edge_crossings(&self) -> usize {
        self.global_edge_crossings_checked(ScoreContext::none())
            .expect("an unchecked crossing score cannot be cancelled")
    }

    fn global_edge_crossings_checked(
        &self,
        context: ScoreContext<'_>,
    ) -> Result<usize, limits::Error> {
        score_poll!(context, self, "graph-crossings.entry", None, None)?;
        if self.edge_order.is_empty() {
            score_poll!(context, self, "graph-crossings.empty-final", None, None)?;
            return Ok(0);
        }
        let center = |node: NodeId| self.active_node_center(node);
        // Direct translation of recovered `doesCross`. Unlike the usual
        // proper-intersection predicate, TALA includes endpoint contact
        // between distinct nodes.
        let does_cross = |u0: Point, u1: Point, v0: Point, v1: Point| {
            let denom = (u1.y - u0.y) * (v1.x - v0.x) - (u1.x - u0.x) * (v1.y - v0.y);
            if denom == 0.0 {
                return false;
            }
            let s = ((v0.y - u0.y) * (v1.x - v0.x) - (v0.x - u0.x) * (v1.y - v0.y)) / denom;
            if !(0.0..=1.0).contains(&s) {
                return false;
            }
            let t = ((u1.x - u0.x) * (v0.y - u0.y) - (u1.y - u0.y) * (v0.x - u0.x)) / denom;
            (0.0..=1.0).contains(&t)
        };

        // Recovered Graph.getGlobalEdgeCrossings groups edges by the
        // getContainerLevel of their endpoints. The scoring ancestry is the
        // original Go Node.Container chain retained across temporary scopes.
        let container_level = |node: NodeId| {
            self.nodes[node.0 as usize]
                .scoring_container_ancestors
                .len()
                + 1
        };
        let mut edges_by_level = BTreeMap::<usize, Vec<(&ArenaEdge, NodeId, NodeId)>>::new();
        for (edge_index, edge) in self.member_edge_entries() {
            score_interval!(context, self, "graph-crossings.edges", None, edge_index)?;
            if self.sequences.iter().any(|sequence| {
                self.sequence_is_active(sequence)
                    && sequence.members.contains(&edge.from)
                    && sequence.members.contains(&edge.to)
            }) {
                continue;
            }
            let from = self.active_aggregate_owner(edge.from);
            let to = self.active_aggregate_owner(edge.to);
            if self.position(from).is_none() || self.position(to).is_none() {
                continue;
            }
            let from_level = container_level(from);
            let to_level = container_level(to);
            if from_level == to_level {
                edges_by_level
                    .entry(from_level)
                    .or_default()
                    .push((edge, from, to));
            }
        }

        let mut crossings = 0;
        for edges in edges_by_level.values() {
            for index in 0..edges.len().saturating_sub(1) {
                score_interval!(context, self, "edge-crossings.outer", None, index)?;
                let left = &edges[index];
                for (inner_index, right) in edges.iter().skip(index + 1).enumerate() {
                    score_interval!(context, self, "edge-crossings.inner", None, inner_index)?;
                    if left.1 == right.1
                        || left.1 == right.2
                        || left.2 == right.1
                        || left.2 == right.2
                    {
                        continue;
                    }
                    let (a, b) = (center(left.1), center(left.2));
                    let (c, d) = (center(right.1), center(right.2));
                    if does_cross(a, b, c, d) {
                        #[cfg(feature = "diagnostic-traces")]
                        #[cfg(feature = "diagnostic-traces")]
                        if crate::internal::trace_env_enabled("TALA_TRACE_EDGE_CROSSINGS") {
                            eprintln!(
                                "EDGE_CROSSING_RUST edge={}>{} edge2={}>{} level={}",
                                self.nodes[left.1.0 as usize].tala_id,
                                self.nodes[left.2.0 as usize].tala_id,
                                self.nodes[right.1.0 as usize].tala_id,
                                self.nodes[right.2.0 as usize].tala_id,
                                edges_by_level
                                    .iter()
                                    .find_map(|(level, candidate)| (std::ptr::eq(candidate, edges))
                                        .then_some(level))
                                    .copied()
                                    .unwrap_or_default(),
                            );
                        }
                        crossings += 1;
                    }
                }
            }
            score_poll!(context, self, "edge-crossings.final", None, None)?;
        }
        score_poll!(context, self, "graph-crossings.final", None, None)?;
        #[cfg(feature = "diagnostic-traces")]
        if crate::internal::trace_env_enabled("TALA_TRACE_EDGE_CROSSINGS") {
            eprintln!("EDGE_CROSSING_TOTAL_RUST count={crossings}");
        }
        Ok(crossings)
    }

    pub(super) fn global_sized_edge_length(&self) -> f64 {
        self.global_sized_edge_length_with_direction(true)
    }

    pub(super) fn global_sized_edge_length_for_alignment(&self) -> f64 {
        // The cache and projected aggregate views are now keyed explicitly;
        // score the baseline through the same live projection as candidates.
        // Keeping a separate retained-pointer baseline would diverge once a
        // prior AlignAxes pass has committed aggregate translations.
        self.global_sized_edge_length_for_alignment_mode(false, false)
    }

    /// Score an AlignAxes candidate after its transaction has been applied.
    ///
    /// The pre-trial score in the pinned Go implementation can observe a
    /// retained aggregate replacement pointer.  A committed candidate is
    /// scored through the live graph projection instead.  Keep this separate
    /// from the baseline helper so that the retained-pointer behavior cannot
    /// leak into candidate acceptance.
    pub(super) fn global_sized_edge_length_for_alignment_candidate(&self) -> f64 {
        self.global_sized_edge_length_for_alignment_mode(false, false)
    }

    #[cfg(test)]
    pub(super) fn global_sized_edge_length_for_alignment_serial_for_test(&self) -> f64 {
        self.global_sized_edge_length_for_alignment_mode(true, true)
    }

    fn global_sized_edge_length_for_alignment_mode(
        &self,
        force_serial: bool,
        alignment_baseline_flow: bool,
    ) -> f64 {
        #[cfg(feature = "diagnostic-traces")]
        crate::internal::call_counts::count!(GlobalScore);
        if crate::internal::trace_env_enabled("TALA_TRACE_ALIGN_GLOBAL_STATE") {
            let local = self
                .clusters
                .iter()
                .flat_map(|cluster| {
                    cluster.members.iter().map(|member| {
                        (
                            self.nodes[member.0 as usize].tala_id,
                            self.position(*member),
                        )
                    })
                })
                .collect::<Vec<_>>();
            let external = self
                .transaction_external_aggregate_children
                .iter()
                .map(|(vessel, members)| {
                    (
                        *vessel,
                        members
                            .iter()
                            .map(|member| (member.tala_id, member.position))
                            .collect::<Vec<_>>(),
                    )
                })
                .collect::<Vec<_>>();
            eprintln!(
                "ALIGN_GLOBAL_STATE_RUST force_serial={} baseline_flow={} local={:?} external={:?}",
                force_serial, alignment_baseline_flow, local, external
            );
        }
        // AlignAxes builds its explicit abduction slice solely by appending
        // Sequence.EdgeAbductions. If no sequence contributes a record, the
        // Go slice remains nil and Node.edgeLength gathers cluster abductions
        // itself. Preserve that nil-versus-present distinction.
        let has_sequence_abductions = self
            .sequences
            .iter()
            .any(|sequence| sequence.has_edge_abductions);
        // TALA sums the current Graph.Nodes slice. The stable arena keeps
        // absorbed sequence/cluster members allocated for cleanup, but those
        // members are absent from Graph.Nodes while their temporary vessel is
        // active and must not contribute a second edge-length term.
        let nodes = self.graph_node_order_cow();
        // The Rust arena keeps absorbed members allocated and projects them
        // through a stable carrier, while Go's temporary graph rewires the
        // vessel's Node.Edges slice.  The v0.9.0 cache key sees that rewired
        // slice; our release-shaped key cannot encode the projection without
        // changing the public graph.  Do not reuse a cached alignment score
        // while an aggregate vessel is active, or a score from a previous
        // projection can be returned for the same visible boxes.
        let materialized_containers = self.materialized_ordinary_container_scoring();
        let edge_length_state = self.edge_length_state_key(
            &nodes,
            true,
            false,
            has_sequence_abductions,
            materialized_containers,
            alignment_baseline_flow,
        );
        if !crate::internal::trace_env_enabled("TALA_TRACE_DISABLE_EDGE_CACHE")
            && let Some(state) = edge_length_state
            && let Some(cached) = self.cost_cache.lookup_edge_length(state)
        {
            return cached;
        }
        #[cfg(feature = "diagnostic-traces")]
        let alignment_trace_target =
            crate::internal::trace_env_value("D2RS_TRACE_ALIGNMENT_TOTAL_BITS")
                .and_then(|target| u64::from_str_radix(target.trim_start_matches("0x"), 16).ok());
        #[cfg(feature = "diagnostic-traces")]
        let alignment_trace_terms = alignment_trace_target
            .map(|_| std::sync::Mutex::new(BTreeMap::<NodeId, (u64, u64, u64, u64, usize)>::new()));
        let edge_length = self.ordered_global_scoring_sum(&nodes, force_serial, |node| {
            let sized = if alignment_baseline_flow {
                self.sized_edge_length_with_cache_mode_materialized_alignment_baseline(
                    node,
                    true,
                    None,
                    has_sequence_abductions,
                    false,
                    true,
                    materialized_containers,
                )
            } else {
                self.sized_edge_length_with_cache_mode_materialized(
                    node,
                    true,
                    None,
                    has_sequence_abductions,
                    false,
                    true,
                    materialized_containers,
                )
            };
            let column = self.column_to_column_crossing_cost(node, true);
            let raw_symmetry = self.sized_symmetry(node, true);
            let symmetry = raw_symmetry
                * self.cell_size
                * self.scoring_edge_count(node) as f64;
            if crate::internal::trace_env_value("TALA_TRACE_ALIGN_GLOBAL_TERM_NODE")
                .is_some_and(|target| target == "all" || target == self.nodes[node.0 as usize].tala_id.to_string())
            {
                eprintln!(
                    "ALIGN_GLOBAL_TERM_RUST node={} pos={:?} sized={} column={} symmetry={} activeCluster={:?} members={:?}",
                    self.nodes[node.0 as usize].tala_id,
                    self.position(node),
                    sized,
                    column,
                    symmetry,
                    self.active_cluster_index(node),
                    self.clusters
                        .iter()
                        .filter(|cluster| self.cluster_is_active(cluster) && cluster.members.contains(&node))
                        .map(|cluster| cluster.vessel_tala_id)
                        .collect::<Vec<_>>()
                );
            }
            let cost = sized + column
                - self.sized_symmetry(node, true)
                    * self.cell_size
                    * self.scoring_edge_count(node) as f64;
            #[cfg(feature = "diagnostic-traces")]
            if let Some(terms) = &alignment_trace_terms {
                terms.lock().expect("alignment trace mutex poisoned").insert(
                    node, (cost.to_bits(), sized.to_bits(), column.to_bits(), raw_symmetry.to_bits(), self.scoring_edge_count(node)),
                );
            }
            cost
        });
        let crossings = self.global_edge_crossings();
        let crossing_cost = self.crossing_cost_value();
        let total = go_127_crossing_penalty_total(edge_length, crossings, crossing_cost);
        #[cfg(feature = "diagnostic-traces")]
        if alignment_trace_target == Some(total.to_bits()) {
            eprintln!(
                "ALIGN_TARGET_RUST bits={:016x} nodes={} crossings={} crossing_cost_bits={:016x} edge_length_bits={:016x} baseline={}",
                total.to_bits(),
                nodes.len(),
                crossings,
                crossing_cost.to_bits(),
                edge_length.to_bits(),
                alignment_baseline_flow
            );
            if let Some(terms) = &alignment_trace_terms {
                let terms = terms.lock().expect("alignment trace mutex poisoned");
                for &node in nodes.iter() {
                    let (cost, sized, column, symmetry, edges) = terms[&node];
                    let vessel = self.active_cluster_index(node).and_then(|index| {
                        self.topology_vessel_node(TopologyVessel::Cluster(index))
                    });
                    eprintln!(
                        "ALIGN_TARGET_NODE_RUST node={} canonical={} bits={:016x} node_edge_bits={:016x} column_bits={:016x} symmetry_bits={:016x} edges={} active_position={:?} actual_position={:?} vessel={:?}",
                        self.active_node_tala_id(node),
                        node.0,
                        cost,
                        sized,
                        column,
                        symmetry,
                        edges,
                        self.active_node_position(node),
                        self.position(node),
                        vessel.map(|v| (v.0, self.position(v)))
                    );
                }
            }
        }
        if crate::internal::trace_env_enabled("TALA_TRACE_ALIGN_GLOBAL_TERMS") {
            eprintln!(
                "ALIGN_TOTAL_RUST edge_length={} crossings={} crossing_cost={} total={} baseline={}",
                edge_length,
                crossings,
                self.raw_crossing_cost(),
                total,
                alignment_baseline_flow,
            );
        }
        if !crate::internal::trace_env_enabled("TALA_TRACE_DISABLE_EDGE_CACHE")
            && let Some(state) = edge_length_state
        {
            self.cost_cache.store_edge_length(state, total);
            if crate::internal::trace_env_enabled("TALA_TRACE_EDGE_LENGTH_CACHE") {
                eprintln!(
                    "EDGE_LENGTH_CACHE_RUST store state={} value={}",
                    state, total
                );
            }
        }
        total
    }

    pub(super) fn materialized_ordinary_container_scoring(&self) -> bool {
        // Graph.Containers is shared in Go. Inspect its current root header
        // without copying every child before testing the first one. This
        // predicate has no callbacks or polls that can replace the header.
        self.containers.with_header_entry(&None, |roots| {
            roots.is_some_and(|roots| {
                let Some(first) = roots
                    .get(0)
                    .map(|root| &self.nodes[root.expect("nil container child").0 as usize])
                else {
                    return false;
                };
                first.is_container
                    && roots.borrowed_slots().all(|root| {
                        let node = &self.nodes[root.expect("nil container child").0 as usize];
                        node.is_container
                            && node.shape == first.shape
                            && node.declared_size == first.declared_size
                    })
            })
        })
    }

    /// Direct translation of `Graph.getContainerAlignment`: equal-sized
    /// sibling containers pay one non-center-port cost unless either axis is
    /// aligned. `AlignAxes` includes this carrier in every candidate score.
    pub(super) fn container_alignment_cost(&mut self) -> f64 {
        if !self.materialized_ordinary_container_scoring()
            && !self.nodes.iter().any(|node| node.scoring_is_container)
        {
            // Mixed temporary roots still stand in for a Go Hierarchy. Their
            // abduction/scoring surface is not materialized in this arena.
            return 0.0;
        }
        // Go's ContainerAlignmentCost scans the current Graph.Nodes slice.
        // The stable arena also retains nodes removed from temporary
        // aggregate graphs; scoring those retained pointers would introduce
        // pairs that do not exist in the Go graph and can initialize its lazy
        // non-center-port cost from the wrong candidate geometry.
        let graph_nodes = self.graph_node_order_cow();
        let containers = graph_nodes
            .iter()
            .copied()
            .filter(|node| {
                self.nodes[node.0 as usize].is_container
                    // CreateVessel uses a new, non-container Node in Go.
                    // An active vessel can occupy its first member's arena
                    // slot here; that member's container flag is not the
                    // vessel's flag (placementcost/graph.go).
                    && !self.active_node_is_aggregate(*node)
                    && self.active_node_position(*node).is_some()
            })
            .collect::<Vec<_>>();
        let mut cost = 0.0;
        for (index, left_id) in containers.iter().copied().enumerate() {
            let left = &self.nodes[left_id.0 as usize];
            let left_container = self.active_node_container(left_id);
            let Some(left_position) = self.active_node_position(left_id) else {
                continue;
            };
            let left_size = self.active_node_size(left_id);
            for right_id in containers.iter().copied().skip(index + 1) {
                let right = &self.nodes[right_id.0 as usize];
                let right_container = self.active_node_container(right_id);
                let Some(right_position) = self.active_node_position(right_id) else {
                    continue;
                };
                let right_size = self.active_node_size(right_id);
                if left_container != right_container
                    || left_size != right_size
                    || left_position.x == right_position.x
                    || left_position.y == right_position.y
                {
                    continue;
                }
                #[cfg(feature = "diagnostic-traces")]
                let trace_cold_unit =
                    crate::internal::trace_env_enabled("TALA_TRACE_CONTAINER_ALIGNMENT")
                        && self.raw_non_center_port_cost() == 0.0;
                let unit_cost = self.non_center_port_cost_value();
                #[cfg(feature = "diagnostic-traces")]
                if trace_cold_unit {
                    eprintln!(
                        "ALIGN_CONTAINER_UNIT_RUST root_hierarchy={} unit={:.17} max_length={:.17} edges={} active_edges={} nodes={} graph_nodes={} graph_ids={:?} max_pair={:?}",
                        self.root_hierarchy,
                        unit_cost,
                        self.positioned_max_length(),
                        self.edge_records.len(),
                        self.edge_order.len(),
                        self.nodes.len(),
                        self.graph_node_order().len(),
                        self.graph_node_order()
                            .iter()
                            .map(|node| self.active_node_tala_id(*node))
                            .collect::<Vec<_>>(),
                        self.positioned_max_pair(),
                    );
                }
                if crate::internal::trace_env_enabled("TALA_TRACE_CONTAINER_ALIGNMENT") {
                    eprintln!(
                        "CONTAINER_PAIR_RUST {}>{} parent={:?},{:?} pos={:?}>{:?} size={:?} unit={}",
                        left.tala_id,
                        right.tala_id,
                        left_container.map(|parent| self.nodes[parent.0 as usize].tala_id),
                        right_container.map(|parent| self.nodes[parent.0 as usize].tala_id),
                        Some(left_position),
                        Some(right_position),
                        left_size,
                        unit_cost,
                    );
                }
                cost += unit_cost;
            }
        }
        cost
    }

    pub(super) fn global_sized_edge_length_with_direction(&self, direction_penalty: bool) -> f64 {
        self.global_sized_edge_length_with_direction_mode(direction_penalty, false, false)
    }

    pub(super) fn global_sized_edge_length_after_tree_restoration(
        &self,
        direction_penalty: bool,
    ) -> f64 {
        self.global_sized_edge_length_with_direction_mode(direction_penalty, true, false)
    }

    #[cfg(test)]
    pub(super) fn global_sized_edge_length_with_direction_serial_for_test(
        &self,
        direction_penalty: bool,
    ) -> f64 {
        self.global_sized_edge_length_with_direction_mode(direction_penalty, false, true)
    }

    pub(super) fn global_sized_edge_length_after_tree_restoration_checked(
        &self,
        direction_penalty: bool,
        context: &dyn limits::Context,
    ) -> Result<f64, limits::Error> {
        self.global_sized_edge_length_with_direction_mode_checked(
            direction_penalty,
            true,
            false,
            Some(context),
        )
    }

    fn global_sized_edge_length_with_direction_mode(
        &self,
        direction_penalty: bool,
        tree_children_restored: bool,
        force_serial: bool,
    ) -> f64 {
        self.global_sized_edge_length_with_direction_mode_checked(
            direction_penalty,
            tree_children_restored,
            force_serial,
            None,
        )
        .expect("an unchecked edge-length score cannot be cancelled")
    }

    fn global_sized_edge_length_with_direction_mode_checked(
        &self,
        direction_penalty: bool,
        tree_children_restored: bool,
        force_serial: bool,
        context: Option<&dyn limits::Context>,
    ) -> Result<f64, limits::Error> {
        #[cfg(feature = "diagnostic-traces")]
        crate::internal::call_counts::count!(GlobalScore);
        let score_context = context.map_or_else(ScoreContext::none, ScoreContext::new);
        score_poll!(score_context, self, "edge-length-entry", None, None)?;
        #[cfg(feature = "diagnostic-traces")]
        if crate::internal::trace_env_enabled("TALA_TRACE_EQ_SCORE") {
            eprintln!(
                "EQ_GLOBAL_RUST abductions={} sequence_abductions={} sequence_flags={:?} overrides={} clusters={} direction={}",
                self.sized_edge_abductions.len(),
                self.sized_edge_abductions
                    .iter()
                    .filter(|abduction| abduction.sequence_abduction)
                    .count(),
                self.sequences
                    .iter()
                    .map(|sequence| sequence.has_edge_abductions)
                    .collect::<Vec<_>>(),
                self.sized_adjacent_overrides.len(),
                self.sized_cluster_distance_boxes.len(),
                direction_penalty,
            );
        }
        let nodes = self.graph_node_order_cow();
        #[cfg(feature = "diagnostic-traces")]
        let edge_total_trace_target =
            crate::internal::trace_env_value("TALA_TRACE_EDGE_LENGTH_TOTAL_BITS")
                .and_then(|target| u64::from_str_radix(target.trim_start_matches("0x"), 16).ok());
        #[cfg(feature = "diagnostic-traces")]
        let edge_total_trace_terms = edge_total_trace_target.map(|_| {
            std::sync::Mutex::new(
                Vec::<(i64, String, u64, u64, u64, usize, u64)>::with_capacity(nodes.len()),
            )
        });
        // Graph.EdgeLength uses the same graph-owned cache for ordinary and
        // aggregate views. The visible node/edge surface supplies its key.
        let materialized_containers = self.materialized_ordinary_container_scoring();
        let edge_length_state = self.edge_length_state_key(
            &nodes,
            direction_penalty,
            tree_children_restored,
            false,
            materialized_containers,
            false,
        );
        // Unit and production execution share the source cache contract.
        // Independent restored/collapsed projection comparisons must prepare
        // distinct cache state, just as their original Go graph views differ.
        if !crate::internal::trace_env_enabled("TALA_TRACE_DISABLE_EDGE_CACHE")
            && let Some(state) = edge_length_state
        {
            if let Some(cached) = self.cost_cache.lookup_edge_length(state) {
                #[cfg(feature = "diagnostic-traces")]
                #[cfg(feature = "diagnostic-traces")]
                if crate::internal::trace_env_enabled("TALA_TRACE_EDGE_LENGTH_CACHE") {
                    eprintln!("EDGE_LENGTH_CACHE_RUST hit state={state}");
                }
                return Ok(cached);
            }
            #[cfg(feature = "diagnostic-traces")]
            if crate::internal::trace_env_enabled("TALA_TRACE_EDGE_LENGTH_CACHE") {
                eprintln!("EDGE_LENGTH_CACHE_RUST miss state={state}");
            }
        }
        let compute = |node, node_context: ScoreContext<'_>| {
            let sized = self.sized_edge_length_with_cache_mode_materialized_checked(
                node,
                direction_penalty,
                None,
                false,
                false,
                tree_children_restored,
                materialized_containers,
                node_context,
            )?;
            let crossing =
                self.column_to_column_crossing_cost_checked(node, false, node_context)?;
            let symmetry = self.sized_symmetry_checked(node, true, node_context)?;
            let active_edges = self.scoring_edge_count(node);
            // Go 1.27 ARM64 graph.go rounds symmetry*CellSize, then
            // FMSUB fuses the edge-count product with the subtraction.
            // Separately rounding that product changes SQL-table gap scores.
            let value =
                (-(symmetry * self.cell_size)).mul_add(active_edges as f64, sized + crossing);
            #[cfg(feature = "diagnostic-traces")]
            if let Some(terms) = &edge_total_trace_terms {
                terms
                    .lock()
                    .expect("edge-length trace mutex poisoned")
                    .push((
                        self.active_node_tala_id(node),
                        self.nodes[node.0 as usize].external_id.clone(),
                        sized.to_bits(),
                        crossing.to_bits(),
                        symmetry.to_bits(),
                        active_edges,
                        value.to_bits(),
                    ));
            }
            #[cfg(feature = "diagnostic-traces")]
            let trace_terms = crate::internal::trace_env_enabled("TALA_TRACE_EDGE_LENGTH_TERMS")
                && crate::internal::trace_env_value("TALA_TRACE_EDGE_LENGTH_NODE").is_none_or(
                    |target| {
                        target == "all" || target == self.nodes[node.0 as usize].tala_id.to_string()
                    },
                );
            #[cfg(feature = "diagnostic-traces")]
            if trace_terms {
                eprintln!(
                    "EDGE_TERM_RUST node={} pos={:?} activePos={:?} activeSize={:?} sized={} crossing={} symmetry={} count={} value={} cluster={:?} owner={:?} edges={:?}",
                    self.nodes[node.0 as usize].tala_id,
                    self.position(node),
                    self.active_node_position(node),
                    self.active_node_size(node),
                    sized,
                    crossing,
                    symmetry,
                    active_edges,
                    value,
                    self.active_cluster_index(node),
                    self.nodes[self.active_aggregate_owner(node).0 as usize].tala_id,
                    self.scoring_edge_ids(node)
                        .iter()
                        .map(|edge| self.edge_records[edge.0 as usize].tala_id)
                        .collect::<Vec<_>>(),
                );
            }
            score_poll!(
                node_context,
                self,
                "edge-length-node-finished",
                Some(node),
                None
            )?;
            Ok(value)
        };
        let edge_length = match context {
            None => self.ordered_global_scoring_result_sum(&nodes, force_serial, |node| {
                compute(node, ScoreContext::none())
            })?,
            Some(context) => match context.shared_scoring_context() {
                Some(shared) => {
                    self.ordered_global_scoring_result_sum(&nodes, force_serial, |node| {
                        compute(node, ScoreContext::new(shared))
                    })?
                }
                None => {
                    // Cell/RefCell diagnostic contexts explicitly model the
                    // source's one-worker path and never cross a thread boundary.
                    let mut total = 0.0;
                    for node in nodes.iter().copied() {
                        total += compute(node, ScoreContext::new(context))?;
                    }
                    total
                }
            },
        };
        let crossings = self.global_edge_crossings_checked(score_context)?;
        let crossing_cost = self.crossing_cost_value();
        let total = go_127_crossing_penalty_total(edge_length, crossings, crossing_cost);
        #[cfg(feature = "diagnostic-traces")]
        if edge_total_trace_target == Some(total.to_bits()) {
            eprintln!(
                "EDGE_TARGET_RUST bits={:016x} nodes={} crossings={} crossing_cost_bits=0x{:016x} edge_length_bits=0x{:016x}",
                total.to_bits(),
                nodes.len(),
                crossings,
                self.raw_crossing_cost().to_bits(),
                edge_length.to_bits(),
            );
            #[cfg(feature = "diagnostic-traces")]
            if let Some(terms) = &edge_total_trace_terms {
                for (tala_id, external_id, sized_bits, column_bits, symmetry_bits, edges, bits) in
                    terms
                        .lock()
                        .expect("edge-length trace mutex poisoned")
                        .iter()
                {
                    eprintln!(
                        "EDGE_TARGET_NODE_RUST node={} debug_id={:?} bits={:016x} node_edge_bits={:016x} column_bits={:016x} symmetry_bits={:016x} edges={}",
                        tala_id, external_id, bits, sized_bits, column_bits, symmetry_bits, edges
                    );
                }
            }
        }
        #[cfg(feature = "diagnostic-traces")]
        if crate::internal::trace_env_enabled("TALA_TRACE_EDGE_LENGTH_TERMS") {
            eprintln!(
                "EDGE_TOTAL_RUST edge_length={} crossings={} crossing_cost={} total={} restored={}",
                edge_length,
                crossings,
                self.raw_crossing_cost(),
                total,
                tree_children_restored
            );
        }
        score_poll!(score_context, self, "edge-length-before-store", None, None)?;
        if let Some(state) = edge_length_state {
            self.cost_cache.store_edge_length(state, total);
            #[cfg(feature = "diagnostic-traces")]
            if crate::internal::trace_env_enabled("TALA_TRACE_EDGE_LENGTH_CACHE") {
                eprintln!(
                    "EDGE_LENGTH_CACHE_RUST store-direction state={} value={}",
                    state, total
                );
            }
        }
        Ok(total)
    }

    /// Borrow the current node order when the membership cache is valid. The
    /// global scoring loop runs once per optimizer candidate, so cloning this
    /// unchanged vector on every score is measurable on ordinary graphs. The
    /// filtered fallback remains owned for manually materialized transitional
    /// graphs whose aggregate membership has not been indexed yet.
    pub(super) fn graph_node_order_cow(&self) -> Cow<'_, [NodeId]> {
        if self.node_order_membership_valid {
            Cow::Borrowed(&self.active_graph_node_order)
        } else {
            Cow::Owned(self.graph_node_order())
        }
    }

    /// Hash the current active Graph.Nodes scoring surface. The pinned Go
    /// release keys this cache from the visible node IDs, boxes, and edge
    /// slices; Rust keeps some projections in side tables, but those are not
    /// part of the release cache identity and must not invalidate a hit.
    fn edge_length_state_key(
        &self,
        nodes: &[NodeId],
        direction_penalty: bool,
        tree_children_restored: bool,
        use_sequence_abductions: bool,
        materialized_containers: bool,
        alignment_baseline_flow: bool,
    ) -> Option<u64> {
        let mut hasher = EdgeLengthStateHasher::new();
        let costs = self.raw_routing_costs();
        if crate::internal::trace_env_enabled("TALA_TRACE_EDGE_LENGTH_CACHE_ORDER") {
            eprintln!(
                "EDGE_LENGTH_CACHE_ORDER_RUST valid={} nodes={:?}",
                self.node_order_membership_valid,
                nodes
                    .iter()
                    .map(|node| self.active_node_tala_id(*node))
                    .collect::<Vec<_>>()
            );
            eprintln!(
                "EDGE_LENGTH_CACHE_MODE_RUST direction={} restored={} sequence_abductions={} materialized={} alignment_baseline={} costs={},{},{},{}",
                direction_penalty,
                tree_children_restored,
                use_sequence_abductions,
                materialized_containers,
                alignment_baseline_flow,
                self.cell_size,
                costs.1,
                costs.2,
                costs.0,
            );
            if crate::internal::trace_env_enabled("TALA_TRACE_EDGE_LENGTH_CACHE_DETAIL") {
                for node in nodes {
                    eprintln!(
                        "EDGE_CACHE_RUST_NODE id={} pos={:?} size={:?} edges={:?}",
                        self.active_node_tala_id(*node),
                        self.active_node_position(*node),
                        self.active_node_size(*node),
                        self.active_edge_ids(*node)
                            .iter()
                            .map(|edge| self.edge_records[edge.0 as usize].tala_id)
                            .collect::<Vec<_>>()
                    );
                }
            }
        }
        // includeSizes is always true for this sized scorer. The other
        // bits mirror Graph.EdgeLengthState's mode flags.
        // Preserve the v0.9.0 cache boundary for projection metadata. The
        // restored-child surface is a separate namespace because Go rewires
        // Node.Edges at that boundary while the stable arena cannot mutate
        // the carrier slice without disturbing cleanup.
        let go_cache_key = true;
        let mut flags = 1_u8;
        if tree_children_restored && !go_cache_key {
            flags |= 2;
        }
        if direction_penalty {
            flags |= 4;
        }
        // The Go cache key has no mode bit. Keep the namespace identical; the
        // restoration path invalidates the Rust cache when its projected edge
        // surface changes rather than perturbing unrelated alignment caches.
        if materialized_containers && !go_cache_key {
            flags |= 8;
        }
        // The Go key includes every sequence EdgeAbduction when AlignAxes
        // supplies that slice.  Keep that release encoding; there is no
        // separate alignment-baseline bit in the Go key.
        // Go has no abduction-presence flag. Supplied sequence records are
        // encoded after the nodes below, using the same three option bits.
        if alignment_baseline_flow && !go_cache_key {
            flags |= 32;
        }
        hasher.write_bytes(&[flags]);
        hasher.write_f64(self.cell_size);
        // RoutingCosts is a raw coherent triple, independent of map occupancy.
        // Reading the key must never materialize any lazy scalar.
        hasher.write_f64(costs.1);
        hasher.write_f64(costs.2);
        hasher.write_f64(costs.0);
        for node in nodes {
            hasher.write_entity_id(self.active_node_tala_id(*node));
            let position = self.active_node_position(*node)?;
            hasher.write_f64(position.x);
            hasher.write_f64(position.y);
            let size = self.active_node_size(*node);
            hasher.write_f64(size.width);
            hasher.write_f64(size.height);
            // Go writes each incident edge ID directly, without a length
            // delimiter. The node order and edge slices already determine
            // the sequence, so preserve that byte stream exactly and avoid
            // introducing a Rust-only cache identity component.
            // Go hashes the pointer-visible Node.Edges slice.  For an active
            // aggregate the stable arena's carrier slice is only the backing
            // storage; reconstruct the temporary vessel's incident edges so
            // cache identity follows the same projected graph surface.
            for edge in self.max_edge_length_incident_edges(*node).iter() {
                hasher.write_u64(self.edge_records[edge.0 as usize].tala_id as u64);
            }
        }
        if go_cache_key {
            // Diagnostic probe for the pinned Go cache identity: its key
            // continues with the option's edge-abduction endpoint IDs, then
            // stops. Go writes optional pointers only when present (without a
            // presence marker), so mirror that byte stream instead of using
            // Rust's richer projection encoding below.
            if use_sequence_abductions {
                for abduction in self
                    .sized_edge_abductions
                    .iter()
                    .filter(|abduction| abduction.sequence_abduction)
                {
                    hasher.write_u64(abduction.edge_tala_id as u64);
                    if let Some(originally_from) = abduction.originally_from {
                        hasher.write_entity_id(originally_from.tala_id);
                    }
                    if let Some(originally_to) = abduction.originally_to {
                        hasher.write_entity_id(originally_to.tala_id);
                    }
                    if let Some(current) = abduction.current_to {
                        if let Some(tala_id) = self.abduction_current_tala_id(current) {
                            hasher.write_entity_id(tala_id);
                        }
                    }
                    if let Some(current) = abduction.current_from {
                        if let Some(tala_id) = self.abduction_current_tala_id(current) {
                            hasher.write_entity_id(tala_id);
                        }
                    }
                }
            }
            return Some(hasher.finish());
        }
        hasher.write_u64(self.node_order.len() as u64);
        for node in &self.node_order {
            hasher.write_node(*node);
        }
        // The Go key includes edge-abduction endpoint IDs. These Rust
        // projection tables are the equivalent state for materialized
        // temporary scopes and must participate in the cache identity.
        hasher.write_u64(self.sized_adjacent_overrides.len() as u64);
        for ((node, edge), adjacent) in &self.sized_adjacent_overrides {
            hasher.write_node(*node);
            hasher.write_u64(edge.0 as u64);
            hasher.write_projected_adjacent(adjacent);
        }
        hasher.write_u64(self.sized_cluster_distance_boxes.len() as u64);
        for ((node, identity), distance) in &self.sized_cluster_distance_boxes {
            hasher.write_node(*node);
            hasher.write_entity_id(*identity);
            hasher.write_point(distance.offset);
            hasher.write_size(distance.size);
            hasher.write_arrangement(distance.arrangement);
            hasher.write_entity_id(distance.vessel_tala_id);
            hasher.write_u64(distance.external_connected.len() as u64);
            for adjacent in &distance.external_connected {
                hasher.write_projected_adjacent(adjacent);
            }
        }
        hasher.write_u64(self.sized_edge_abductions.len() as u64);
        for abduction in &self.sized_edge_abductions {
            hasher.write_u64(abduction.source_edge.0 as u64);
            hasher.write_entity_id(abduction.edge_tala_id);
            if let Some(edge) = abduction.edge {
                hasher.write_u64(edge.0 as u64);
            }
            hasher.write_optional_abduction_current(abduction.current_from);
            hasher.write_optional_abduction_current(abduction.current_to);
            hasher.write_optional_abducted_node(abduction.originally_from.as_ref());
            hasher.write_optional_abducted_node(abduction.originally_to.as_ref());
            hasher.write_optional_node(abduction.originally_from_container);
            hasher.write_optional_node(abduction.originally_to_container);
            hasher.write_u64(u64::from(abduction.sequence_abduction));
            hasher.write_adjacent_list(&abduction.obstructions_from_to);
            hasher.write_adjacent_list(&abduction.obstructions_to_from);
            hasher.write_obstruction_inventory(&abduction.obstruction_sets_from_to);
            hasher.write_obstruction_inventory(&abduction.obstruction_sets_to_from);
            hasher.write_u64(abduction.originally_from_table_neighbors.len() as u64);
            for neighbor in &abduction.originally_from_table_neighbors {
                hasher.write_projected_adjacent(&neighbor.other);
                hasher.write_u64(neighbor.column_index as u64);
            }
            hasher.write_u64(abduction.originally_to_table_neighbors.len() as u64);
            for neighbor in &abduction.originally_to_table_neighbors {
                hasher.write_projected_adjacent(&neighbor.other);
                hasher.write_u64(neighbor.column_index as u64);
            }
        }
        hasher.write_u64(self.sized_projected_obstructions.len() as u64);
        for ((node, edge), obstructions) in &self.sized_projected_obstructions {
            hasher.write_node(*node);
            hasher.write_u64(edge.0 as u64);
            hasher.write_adjacent_list(obstructions);
        }
        hasher.write_u64(self.sized_projected_obstruction_sets.len() as u64);
        for ((node, edge), inventory) in &self.sized_projected_obstruction_sets {
            hasher.write_node(*node);
            hasher.write_u64(edge.0 as u64);
            hasher.write_obstruction_inventory(inventory);
        }
        hasher.write_u64(self.sized_collapsed_symmetry_neighbors.len() as u64);
        for (identity, neighbors) in &self.sized_collapsed_symmetry_neighbors {
            hasher.write_entity_id(*identity);
            hasher.write_adjacent_list(neighbors);
        }
        hasher.write_u64(self.sequences.len() as u64);
        for sequence in &self.sequences {
            hasher.write_u64(sequence.members.len() as u64);
            for member in &sequence.members {
                hasher.write_node(*member);
            }
            hasher.write_entity_id(sequence.vessel_tala_id);
            hasher.write_optional_node(sequence.container);
            hasher.write_u64(u64::from(sequence.has_edge_abductions));
        }
        hasher.write_u64(self.clusters.len() as u64);
        for cluster in &self.clusters {
            hasher.write_u64(cluster.members.len() as u64);
            for member in &cluster.members {
                hasher.write_node(*member);
            }
            hasher.write_arrangement(cluster.arrangement);
            hasher.write_arrangement(cluster.desired_arrangement);
            hasher.write_f64(cluster.padding);
            hasher.write_entity_id(cluster.vessel_tala_id);
            hasher.write_u64(u64::from(cluster.fixed_size));
        }
        hasher.write_u64(self.cluster_vessel_sizes.len() as u64);
        for size in &self.cluster_vessel_sizes {
            hasher.write_size(*size);
        }
        hasher.write_u64(self.active_cluster_flags.len() as u64);
        for active in &self.active_cluster_flags {
            hasher.write_u64(u64::from(*active));
        }
        hasher.write_u64(self.active_sequence_flags.len() as u64);
        for active in &self.active_sequence_flags {
            hasher.write_u64(u64::from(*active));
        }
        hasher.write_u64(self.pending_cluster_vessel_positions.len() as u64);
        for (cluster, position) in &self.pending_cluster_vessel_positions {
            hasher.write_u64(*cluster as u64);
            hasher.write_point(*position);
        }
        // These are uncommon, but they can affect symmetry through the
        // shared-pointer transaction surface. Preserve collision safety
        // with a deterministic debug fallback only when present.
        if !self.transaction_external_containers.is_empty()
            || !self.transaction_external_container_children.is_empty()
            || !self.transaction_external_aggregate_children.is_empty()
            || !self.projected_transaction_nodes.is_empty()
        {
            hasher.write_bytes(
                format!(
                    "{:?}{:?}{:?}{:?}",
                    self.transaction_external_containers,
                    self.transaction_external_container_children,
                    self.transaction_external_aggregate_children,
                    self.projected_transaction_nodes,
                )
                .as_bytes(),
            );
        }
        hasher.write_bytes(&[u8::from(self.suppress_aggregate_projection)]);
        Some(hasher.finish())
    }

    fn ordered_global_scoring_result_sum<F>(
        &self,
        nodes: &[NodeId],
        force_serial: bool,
        compute: F,
    ) -> Result<f64, limits::Error>
    where
        F: Fn(NodeId) -> Result<f64, limits::Error> + Send + Sync,
    {
        #[cfg(feature = "diagnostic-traces")]
        let tracing_enabled = crate::internal::trace_env_enabled("TALA_TRACE_EQ_SCORE")
            || crate::internal::trace_env_enabled("TALA_TRACE_EDGE_LENGTH_TERMS")
            || crate::internal::trace_env_value("TALA_TRACE_EDGE_LENGTH_TOTAL_BITS").is_some()
            || crate::internal::trace_env_enabled("TALA_TRACE_SIZED_DETAIL")
            || crate::internal::trace_env_enabled("TALA_TRACE_SIZED_DETAIL_POSITION");
        #[cfg(not(feature = "diagnostic-traces"))]
        let tracing_enabled = false;
        #[cfg(not(target_arch = "wasm32"))]
        {
            let pool = scoring_pool();
            if !force_serial
                && nodes.len() > 10
                && pool.current_num_threads() > 1
                && !tracing_enabled
            {
                return scoring_result_sum_in_pool(pool, nodes, compute);
            }
        }
        let _ = (force_serial, tracing_enabled);
        let mut total = 0.0;
        for node in nodes.iter().copied() {
            total += compute(node)?;
        }
        Ok(total)
    }

    /// Evaluate independent node-scoring terms in parallel while preserving
    /// the original graph order for the floating-point reduction. TALA uses
    /// this same shape for Graph.getGlobalEdgeLength: node work may complete
    /// in any order, but each result lands in a fixed slot and the final sum
    /// is ordered. Keep small graphs serial, matching TALA's threshold.
    fn ordered_global_scoring_sum<F>(&self, nodes: &[NodeId], force_serial: bool, compute: F) -> f64
    where
        F: Fn(NodeId) -> f64 + Send + Sync,
    {
        let tracing_enabled = crate::internal::trace_env_enabled("TALA_TRACE_EQ_SCORE")
            || crate::internal::trace_env_enabled("TALA_TRACE_EDGE_LENGTH_TERMS")
            || crate::internal::trace_env_value("TALA_TRACE_EDGE_LENGTH_TOTAL_BITS").is_some()
            || crate::internal::trace_env_enabled("TALA_TRACE_SIZED_DETAIL")
            || crate::internal::trace_env_enabled("TALA_TRACE_SIZED_DETAIL_POSITION");
        #[cfg(not(target_arch = "wasm32"))]
        {
            let pool = scoring_pool();
            if !force_serial
                && nodes.len() > 10
                && pool.current_num_threads() > 1
                && !tracing_enabled
            {
                return scoring_result_sum_in_pool(pool, nodes, |node| {
                    Ok::<_, limits::Error>(compute(node))
                })
                .expect("infallible scoring terms");
            }
        }

        let _ = (force_serial, tracing_enabled);
        nodes
            .iter()
            .copied()
            .map(compute)
            .fold(0.0, |sum, term| sum + term)
    }

    // Direct translation of directionCounts.getTransformsTo. TALA's stable
    // ordering matters when two axes have equal directed-edge counts: the
    // requested direction wins its tie without disturbing the remaining
    // Right, Bottom, Left, Top order.
}

/// Match EdgeLength's worker-strided traversal, join and ordered reduction.
/// One worker's error stops that worker; other workers still join normally.
/// A captured, shared context reaches this helper through `compute`, rather
/// than reading a request token from the Rayon worker's thread-local storage.
#[cfg(not(target_arch = "wasm32"))]
fn scoring_result_sum_in_pool<F>(
    pool: &rayon::ThreadPool,
    nodes: &[NodeId],
    compute: F,
) -> Result<f64, limits::Error>
where
    F: Fn(NodeId) -> Result<f64, limits::Error> + Send + Sync,
{
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    let worker_count = nodes.len().min(pool.current_num_threads());
    let costs = nodes.iter().map(|_| AtomicU64::new(0)).collect::<Vec<_>>();
    let first_error = std::sync::Mutex::new(None);
    let worker_panicked = AtomicBool::new(false);
    let run_stripe = |worker: usize| {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            for index in (worker..nodes.len()).step_by(worker_count) {
                match compute(nodes[index]) {
                    Ok(value) => costs[index].store(value.to_bits(), Ordering::Relaxed),
                    Err(error) => {
                        first_error
                            .lock()
                            .expect("edge-length error mutex poisoned")
                            .get_or_insert(error);
                        return;
                    }
                }
            }
        }));
        if result.is_err() {
            worker_panicked.store(true, Ordering::Relaxed);
        }
    };
    // The caller takes the first stripe, as one of EdgeLength's workers,
    // instead of blocking idle while the pool runs every stripe.
    pool.in_place_scope(|scope| {
        let run_stripe = &run_stripe;
        for worker in 1..worker_count {
            scope.spawn(move |_| run_stripe(worker));
        }
        run_stripe(0);
    });
    if worker_panicked.load(Ordering::Relaxed) {
        panic!("TALA EdgeLength worker invariant failure");
    }
    if let Some(error) = first_error
        .into_inner()
        .expect("edge-length error mutex poisoned")
    {
        return Err(error);
    }
    Ok(costs.iter().fold(0.0, |total, cost| {
        total + f64::from_bits(cost.load(Ordering::Relaxed))
    }))
}

/// The global score's node terms are independent but relatively fine-grained:
/// a full machine-sized Rayon pool spends more time scheduling and waiting than
/// computing on the recovered transaction workload. Keep a small dedicated
/// pool for this one fan-out, while retaining `RAYON_NUM_THREADS` as an
/// explicit tuning escape hatch for benchmark and deployment experiments.
#[cfg(not(target_arch = "wasm32"))]
fn scoring_pool() -> &'static rayon::ThreadPool {
    static POOL: std::sync::OnceLock<rayon::ThreadPool> = std::sync::OnceLock::new();
    POOL.get_or_init(|| {
        let threads = std::env::var("RAYON_NUM_THREADS")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .filter(|&value| value > 0)
            .unwrap_or_else(|| {
                std::thread::available_parallelism()
                    .map(|parallelism| parallelism.get().min(6))
                    .unwrap_or(2)
            });
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .expect("global scoring Rayon pool should build")
    })
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod checked_scoring_worker_tests {
    use super::*;

    #[test]
    fn worker_error_joins_other_stripes_before_returning() {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(2)
            .build()
            .unwrap();
        let nodes = (0..12).map(NodeId).collect::<Vec<_>>();
        let visited = std::sync::Mutex::new(Vec::new());
        let result = scoring_result_sum_in_pool(&pool, &nodes, |node| {
            visited.lock().unwrap().push(node.0);
            if node.0 == 0 {
                return Err(limits::Error::Context {
                    location: "EdgeLength".into(),
                    cause: limits::ContextError::Cancelled,
                });
            }
            Ok(f64::from(node.0))
        });
        assert_eq!(
            result.unwrap_err().to_string(),
            "EdgeLength: context canceled"
        );
        let visited = visited.into_inner().unwrap();
        assert_eq!(
            visited
                .iter()
                .filter(|node| **node % 2 == 0)
                .copied()
                .collect::<Vec<_>>(),
            vec![0]
        );
        assert_eq!(
            visited
                .iter()
                .filter(|node| **node % 2 == 1)
                .copied()
                .collect::<Vec<_>>(),
            vec![1, 3, 5, 7, 9, 11]
        );
    }

    #[test]
    fn worker_completion_order_does_not_change_float_reduction() {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(2)
            .build()
            .unwrap();
        let nodes = (0..12).map(NodeId).collect::<Vec<_>>();
        let mut terms = [1.0; 12];
        terms[0] = 1e16;
        terms[2] = -1e16;
        let result =
            scoring_result_sum_in_pool(&pool, &nodes, |node| Ok(terms[node.0 as usize])).unwrap();
        let ordered = terms.iter().fold(0.0, |sum, term| sum + term);
        let stripe_sum =
            terms.iter().step_by(2).sum::<f64>() + terms.iter().skip(1).step_by(2).sum::<f64>();
        assert_ne!(ordered.to_bits(), stripe_sum.to_bits());
        assert_eq!(result.to_bits(), ordered.to_bits());
    }
}

#[cfg(test)]
mod active_cluster_origin_tests {
    use super::*;
    use crate::Node;

    /// Match the accepted centered Circle flip used by the pristine Go
    /// stationary transaction witness. Retained members stay in the arena,
    /// while current Graph.Nodes exposes the first member as vessel id 5.
    fn centered_circle_cluster() -> (ArenaGraph, NodeId, NodeId, NodeId) {
        let mut input = Graph::default();
        let mut container_node = Node::new(
            "container",
            Size {
                width: 948.0,
                height: 948.0,
            },
        );
        container_node.shape = ShapeKind::Circle;
        let container = input.add_node(container_node);
        let mut member = Node::new(
            "first",
            Size {
                width: 60.0,
                height: 80.0,
            },
        );
        member.parent = Some(container);
        let first = input.add_node(member.clone());
        member.external_id = "second".to_owned();
        let second = input.add_node(member);
        let mut peer = Node::new(
            "peer",
            Size {
                width: 100.0,
                height: 80.0,
            },
        );
        peer.parent = Some(container);
        let peer = input.add_node(peer);
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
        let before = geometry(&graph);
        graph.sync_clusters();
        graph.reposition_ordinary_containers();
        assert_eq!(geometry(&graph), before, "the Go baseline is refit-stable");
        assert!(graph.optimize_cluster_only_flip(0));
        assert_eq!(graph.clusters[0].arrangement, ClusterArrangement::Row);
        assert_eq!(graph.clusters[0].padding, 20.0);
        assert_eq!(
            graph.position(container),
            Some(Point {
                x: -94.0,
                y: -304.0
            })
        );
        assert_eq!(
            graph.nodes[container.0 as usize].rect.size,
            Size {
                width: 1005.0,
                height: 1005.0,
            }
        );
        assert_eq!(graph.position(first), Some(Point { x: 69.0, y: 159.0 }));
        assert_eq!(graph.position(second), Some(Point { x: 149.0, y: 159.0 }));
        assert_eq!(graph.position(peer), Some(Point { x: 649.0, y: 159.0 }));
        assert_eq!(graph.active_node_position(first), graph.position(first));
        assert_eq!(
            graph.active_node_size(first),
            Size {
                width: 140.0,
                height: 80.0
            }
        );
        assert_eq!(graph.active_node_tala_id(first), 5);
        assert_eq!(graph.active_node_container(first), Some(container));
        assert!(graph.graph_node_order().contains(&first));
        assert!(!graph.graph_node_order().contains(&second));
        assert_eq!(graph.nodes[first.0 as usize].cluster, Some(0));
        assert_eq!(graph.nodes[second.0 as usize].cluster, Some(0));
        (graph, container, first, peer)
    }

    fn geometry(graph: &ArenaGraph) -> Vec<(Option<Point>, Size)> {
        graph
            .nodes
            .iter()
            .map(|node| (node.position, node.rect.size))
            .collect()
    }

    fn stationary_origin_case(peer_fixed: Option<Point>, member_fixed: Option<Point>, valid: bool) {
        let (mut graph, _container, first, peer) = centered_circle_cluster();
        graph.nodes[peer.0 as usize].fixed_top_left = peer_fixed;
        graph.nodes[first.0 as usize].fixed_top_left = member_fixed;
        let before = geometry(&graph);
        let active_before = graph
            .graph_node_order()
            .into_iter()
            .map(|id| {
                (
                    graph.active_node_tala_id(id),
                    graph.active_node_position(id),
                    graph.active_node_size(id),
                )
            })
            .collect::<Vec<_>>();
        // No trial movement or container fitting: the sole rejection in Go's
        // AffectContainers=false Commit is the distinct vessel past origin.
        assert!(graph.transaction_containment_is_valid());
        let overlap_snapshot = graph.existing_overlap_pairs();
        assert!(!graph.transaction_has_new_overlap(&overlap_snapshot));
        assert!(!graph.transaction_has_bad_state_overlap(&overlap_snapshot));
        let projected = graph.projected_transaction_state();
        let vessel = projected
            .nodes
            .iter()
            .find(|entry| entry.node.tala_id == 5)
            .unwrap();
        assert_eq!(vessel.node.cluster, None);
        assert_eq!(vessel.node.fixed_top_left, None);
        assert_eq!(
            graph.projected_transaction_nodes_are_valid(&projected),
            valid,
            "the current-node projection matches the pristine Go stationary Commit"
        );
        let actual = graph.transaction_fixed_origin_is_valid();
        assert_eq!(
            geometry(&graph),
            before,
            "stationary validation preserves all retained and active geometry"
        );
        assert_eq!(
            graph
                .graph_node_order()
                .into_iter()
                .map(|id| (
                    graph.active_node_tala_id(id),
                    graph.active_node_position(id),
                    graph.active_node_size(id)
                ))
                .collect::<Vec<_>>(),
            active_before
        );
        assert_eq!(
            actual, valid,
            "fixed-origin guard must use active vessel identity and metadata"
        );
    }

    #[test]
    fn active_cluster_origin_no_fixed_stationary_control() {
        stationary_origin_case(None, None, true);
    }

    #[test]
    fn active_cluster_origin_at_origin_stationary_control() {
        stationary_origin_case(Some(Point { x: 580.0, y: 0.0 }), None, true);
    }

    #[test]
    fn active_cluster_origin_rejects_stationary_vessel_before_origin() {
        stationary_origin_case(Some(Point { x: 540.0, y: 0.0 }), None, false);
    }

    #[test]
    fn active_cluster_origin_ignores_retained_fixed_metadata() {
        stationary_origin_case(None, Some(Point { x: -1000.0, y: 0.0 }), true);
    }
}

#[cfg(test)]
mod go_crossing_rounding_tests {
    use super::go_127_crossing_penalty_total;

    #[test]
    fn matches_go_127_arm64_fused_crossing_accumulation() {
        let edge_length = 41038.89019330686;
        let crossings = 3_usize;
        let crossing_cost = 2_400.033_186_186_46;
        let total = go_127_crossing_penalty_total(edge_length, crossings, crossing_cost);

        #[cfg(target_arch = "aarch64")]
        assert_eq!(total.to_bits(), 0x40e7_8ddf_ac0c_1b15);
        #[cfg(not(target_arch = "aarch64"))]
        assert_eq!(total.to_bits(), 0x40e7_8ddf_ac0c_1b16);
    }
}
#[cfg(test)]
mod turn_cache_state_key_tests {
    use super::*;

    #[test]
    fn lazy_turn_state_keys_match_original_go_with_full_and_empty_maps() {
        // Source golden inputs are recorded in tala-turn-key-witness-20261002:
        // actual private placementEdgeLengthState, IDs1/2, edge0, cell100,
        // two100x100 boxes at(1000,1000)/(2000,1000), sized flag1, no abductions.
        let mut input = Graph::default();
        let ids = ["a", "b"].map(|name| {
            input.add_node(crate::Node::new(
                name,
                Size {
                    width: 100.0,
                    height: 100.0,
                },
            ))
        });
        input.add_edge(crate::Edge {
            source: ids[0],
            target: ids[1],
        });
        let mut graph = ArenaGraph::from_input(&input);
        for (index, id) in ids.into_iter().enumerate() {
            graph.nodes[id.0 as usize].tala_id = index as i64 + 1;
            graph.set_position(
                id,
                Point {
                    x: 1000.0 + 1000.0 * index as f64,
                    y: 1000.0,
                },
            );
        }
        graph.edge_records[0].tala_id = 0;
        graph.compute_cell_size();
        let key = |graph: &ArenaGraph| {
            graph
                .edge_length_state_key(&graph.graph_node_order(), false, false, false, false, false)
                .unwrap()
        };
        assert_eq!(key(&graph), 0x3d10_d8e5_38de_630e);
        assert_eq!(
            graph.raw_turn_cost().to_bits(),
            0,
            "raw key is not a semantic reader"
        );
        assert_eq!(graph.turn_cost_value(), 112.5);
        assert_eq!(graph.cost_cache.edge_length_entries(), 0);
        assert_eq!(
            key(&graph),
            0xa82b_96a7_0ccd_1842,
            "a populated turn cache is visible even with an empty map"
        );

        // Actual source-shaped scores materialize Crossing even when there
        // are no crossings; the two raw-key namespaces each get one entry.
        graph.reset_turn_cost();
        graph.global_sized_edge_length_with_direction(false);
        graph.global_sized_edge_length_with_direction(false);
        assert_eq!(graph.cost_cache.edge_length_entries(), 2);
        assert_eq!(graph.raw_crossing_cost().to_bits(), 0x4058_e219_652b_d3c3);
        graph.reset_turn_cost();
        assert_eq!(key(&graph), 0x4b32_5f7b_9076_c29b);
        assert_eq!(graph.raw_turn_cost().to_bits(), 0);
        assert_eq!(graph.cost_cache.edge_length_entries(), 2);
        assert_eq!(graph.turn_cost_value(), 112.5);
        assert_eq!(key(&graph), 0xf7da_5955_5443_cec7);
        assert_eq!(graph.cost_cache.edge_length_entries(), 2);
    }
}

#[cfg(test)]
mod routing_cost_owner_tests {
    use super::*;

    fn prepared_cost_graph(containers: bool, connected: bool) -> ArenaGraph {
        let mut input = Graph::default();
        let ids = ["a", "b"].map(|name| {
            input.add_node(crate::Node::new(
                name,
                Size {
                    width: 100.0,
                    height: 100.0,
                },
            ))
        });
        if connected {
            input.add_edge(crate::Edge {
                source: ids[0],
                target: ids[1],
            });
        }
        let mut graph = ArenaGraph::from_input(&input);
        for (index, id) in ids.into_iter().enumerate() {
            graph.nodes[id.0 as usize].tala_id = index as i64 + 1;
            graph.nodes[id.0 as usize].is_container = containers;
            graph.set_position(
                id,
                Point {
                    x: 1000.0 + 1000.0 * index as f64,
                    y: 1000.0
                        + if containers {
                            1000.0 * index as f64
                        } else {
                            0.0
                        },
                },
            );
        }
        if connected {
            graph.edge_records[0].tala_id = 0;
        }
        graph.compute_cell_size();
        graph
    }

    fn key(graph: &ArenaGraph) -> u64 {
        graph
            .edge_length_state_key(&graph.graph_node_order(), false, false, false, false, false)
            .unwrap()
    }

    #[test]
    fn cold_zero_crossing_score_materializes_graph_crossing_cost() {
        let graph = prepared_cost_graph(false, true);
        let before = key(&graph);
        let first = graph.global_sized_edge_length_with_direction(false);
        let first_key = key(&graph);
        assert_eq!(graph.cost_cache.edge_length_entries(), 1);
        let second = graph.global_sized_edge_length_with_direction(false);
        eprintln!(
            "COST_OWNER_COLD first_bits={:016x} second_bits={:016x} crossing_bits={:016x} turn_bits={:016x} noncenter_bits={:016x} keys={:016x},{:016x},{:016x} entries={}",
            first.to_bits(),
            second.to_bits(),
            graph.raw_crossing_cost().to_bits(),
            graph.raw_turn_cost().to_bits(),
            graph.raw_non_center_port_cost().to_bits(),
            before,
            first_key,
            key(&graph),
            graph.cost_cache.edge_length_entries()
        );
        assert_eq!(graph.raw_crossing_cost().to_bits(), 0x4058_e219_652b_d3c3);
        assert_eq!(first.to_bits(), 0x409c_2000_0000_0000);
        assert_eq!(first.to_bits(), second.to_bits());
        assert_eq!(before, 0x3d10_d8e5_38de_630e);
        assert_eq!(first_key, 0xf7da_5955_5443_cec7);
        assert_eq!(key(&graph), first_key);
        assert_eq!(graph.cost_cache.edge_length_entries(), 2);
        assert_eq!(graph.raw_non_center_port_cost().to_bits(), 0);
    }

    #[test]
    fn warm_raw_crossing_key_does_not_depend_on_map_occupancy() {
        let graph = prepared_cost_graph(false, true);
        // This retains the pre-migration raw RestoreRoutingCosts/key control;
        // semantic first-reader controls below separately exercise getters.
        graph.set_crossing_cost(f64::from_bits(0x4058_e219_652b_d3c3));
        eprintln!(
            "COST_OWNER_WARM_CROSS key={:016x} crossing_bits={:016x} entries={}",
            key(&graph),
            graph.raw_crossing_cost().to_bits(),
            graph.cost_cache.edge_length_entries()
        );
        assert_eq!(key(&graph), 0x4b32_5f7b_9076_c29b);
        assert_eq!(graph.cost_cache.edge_length_entries(), 0);
    }

    #[test]
    fn alignment_first_reader_publishes_the_graph_noncenter_cost() {
        let mut graph = prepared_cost_graph(true, true);
        let alignment = graph.container_alignment_cost();
        eprintln!(
            "COST_OWNER_ALIGNMENT cost_bits={:016x} raw_bits={:016x} entries={}",
            alignment.to_bits(),
            graph.raw_non_center_port_cost().to_bits(),
            graph.cost_cache.edge_length_entries()
        );
        assert_eq!(alignment.to_bits(), 0x404b_4915_6898_d367);
        assert_eq!(key(&graph), 0x4c0c_e3c1_2b3a_446d);
        assert_eq!(
            graph.raw_non_center_port_cost().to_bits(),
            alignment.to_bits()
        );
        assert_eq!(graph.cost_cache.edge_length_entries(), 0);
    }

    #[test]
    fn alignment_reuses_a_warm_graph_noncenter_cost() {
        let mut graph = prepared_cost_graph(true, true);
        graph.set_non_center_port_cost(37.0);
        let alignment = graph.container_alignment_cost();
        eprintln!(
            "COST_OWNER_ALIGNMENT_WARM cost_bits={:016x} raw_bits={:016x}",
            alignment.to_bits(),
            graph.raw_non_center_port_cost().to_bits()
        );
        assert_eq!(alignment, 37.0);
    }

    #[test]
    fn alignment_without_edges_keeps_noncenter_zero() {
        let mut graph = prepared_cost_graph(true, false);
        let alignment = graph.container_alignment_cost();
        eprintln!(
            "COST_OWNER_ALIGNMENT_EMPTY cost_bits={:016x} raw_bits={:016x}",
            alignment.to_bits(),
            graph.raw_non_center_port_cost().to_bits()
        );
        assert_eq!(alignment.to_bits(), 0);
        assert_eq!(graph.raw_non_center_port_cost().to_bits(), 0);
    }

    #[test]
    fn actual_first_readers_warm_costs_without_populating_the_edge_map() {
        let graph = prepared_cost_graph(false, true);
        assert_eq!(graph.crossing_cost_value().to_bits(), 0x4058_e219_652b_d3c3);
        assert_eq!(key(&graph), 0x4b32_5f7b_9076_c29b);
        assert_eq!(graph.cost_cache.edge_length_entries(), 0);
        let graph = prepared_cost_graph(false, true);
        assert_eq!(
            graph.non_center_port_cost_value().to_bits(),
            0x4043_4b33_3333_3332
        );
        assert_eq!(key(&graph), 0x50a4_ebc8_5bdb_0f86);
        assert_eq!(graph.cost_cache.edge_length_entries(), 0);
        assert_eq!(graph.raw_turn_cost().to_bits(), 0);
        assert_eq!(graph.raw_crossing_cost().to_bits(), 0);
    }

    #[test]
    fn zero_edge_noncenter_reader_does_not_replace_negative_zero() {
        let graph = prepared_cost_graph(false, false);
        graph.restore_routing_costs((-0.0, -0.0, -0.0));
        assert_eq!(graph.non_center_port_cost_value().to_bits(), 0);
        assert_eq!(
            graph.raw_non_center_port_cost().to_bits(),
            (-0.0_f64).to_bits()
        );
        assert_eq!(graph.crossing_cost_value().to_bits(), 0);
        assert_eq!(graph.raw_crossing_cost().to_bits(), 0);
        assert_eq!(graph.turn_cost_value().to_bits(), 0);
        assert_eq!(graph.raw_turn_cost().to_bits(), 0);
    }

    #[test]
    fn routing_cost_owner_shares_trials_and_detaches_independent_graphs() {
        let graph = prepared_cost_graph(false, true);
        let mut trial = graph.clone();
        trial.set_position(
            NodeId(1),
            Point {
                x: 3000.0,
                y: 1000.0,
            },
        );
        let crossing = trial.crossing_cost_value();
        let noncenter = trial.non_center_port_cost_value();
        assert_eq!(graph.raw_crossing_cost().to_bits(), crossing.to_bits());
        assert_eq!(
            graph.raw_non_center_port_cost().to_bits(),
            noncenter.to_bits()
        );
        let mut independent = graph.clone();
        independent.detach_cost_owner();
        independent.restore_routing_costs((7.0, 9.0, 11.0));
        assert_eq!(graph.raw_crossing_cost().to_bits(), crossing.to_bits());
        assert_eq!(
            graph.raw_non_center_port_cost().to_bits(),
            noncenter.to_bits()
        );
        independent.cost_cache.store_edge_length(1, 2.0);
        independent.cost_cache.reset_placement();
        assert_eq!(independent.raw_routing_costs(), (0.0, 0.0, 0.0));
        assert_eq!(independent.cost_cache.edge_length_entries(), 0);
        assert_ne!(graph.raw_crossing_cost(), 0.0);
    }

    #[test]
    fn raw_zero_and_nonfinite_cost_keys_match_original_go() {
        // Exact frozen original Go getter/raw-key observations, including the
        // no-store negative-zero NonCenterPort branch, not normalized floats.
        // See tala-routing-cost-negative-zero-original-go-witness-20261002.json.
        let cases = [
            (
                "attempt-4/negative-zero-empty-edges/Crossing",
                [0x8000000000000000, 0x8000000000000000, 0x8000000000000000],
                0xd943751a4decfd4e,
                ArenaGraph::crossing_cost_value as fn(&ArenaGraph) -> f64,
                0x0000000000000000,
                [0x0000000000000000, 0x8000000000000000, 0x8000000000000000],
                0xbdd9023b744383ce,
            ),
            (
                "attempt-4/negative-zero-empty-edges/Turn",
                [0x8000000000000000, 0x8000000000000000, 0x8000000000000000],
                0xd943751a4decfd4e,
                ArenaGraph::turn_cost_value as fn(&ArenaGraph) -> f64,
                0x0000000000000000,
                [0x8000000000000000, 0x0000000000000000, 0x8000000000000000],
                0xb3cbe93ca39d83ce,
            ),
            (
                "attempt-4/negative-zero-empty-edges/NonCenterPort",
                [0x8000000000000000, 0x8000000000000000, 0x8000000000000000],
                0xd943751a4decfd4e,
                ArenaGraph::non_center_port_cost_value as fn(&ArenaGraph) -> f64,
                0x0000000000000000,
                [0x8000000000000000, 0x8000000000000000, 0x8000000000000000],
                0xd943751a4decfd4e,
            ),
            (
                "attempt-4/cached-quiet-nan/Crossing",
                [0x7ff8123456789abc, 0x0000000000000000, 0x0000000000000000],
                0xb4ab2496f542d551,
                ArenaGraph::crossing_cost_value as fn(&ArenaGraph) -> f64,
                0x7ff8123456789abc,
                [0x7ff8123456789abc, 0x0000000000000000, 0x0000000000000000],
                0xb4ab2496f542d551,
            ),
            (
                "attempt-4/cached-quiet-nan/Turn",
                [0x0000000000000000, 0x7ff8123456789abc, 0x0000000000000000],
                0xba18ade94c521691,
                ArenaGraph::turn_cost_value as fn(&ArenaGraph) -> f64,
                0x7ff8123456789abc,
                [0x0000000000000000, 0x7ff8123456789abc, 0x0000000000000000],
                0xba18ade94c521691,
            ),
            (
                "attempt-4/cached-quiet-nan/NonCenterPort",
                [0x0000000000000000, 0x0000000000000000, 0x7ff8123456789abc],
                0x6f4418b8d99ee0b1,
                ArenaGraph::non_center_port_cost_value as fn(&ArenaGraph) -> f64,
                0x7ff8123456789abc,
                [0x0000000000000000, 0x0000000000000000, 0x7ff8123456789abc],
                0x6f4418b8d99ee0b1,
            ),
            (
                "attempt-4/cached-positive-inf/Crossing",
                [0x7ff0000000000000, 0x0000000000000000, 0x0000000000000000],
                0x4784759ed8afa353,
                ArenaGraph::crossing_cost_value as fn(&ArenaGraph) -> f64,
                0x7ff0000000000000,
                [0x7ff0000000000000, 0x0000000000000000, 0x0000000000000000],
                0x4784759ed8afa353,
            ),
            (
                "attempt-4/cached-positive-inf/Turn",
                [0x0000000000000000, 0x7ff0000000000000, 0x0000000000000000],
                0x2d078e8c58b41b13,
                ArenaGraph::turn_cost_value as fn(&ArenaGraph) -> f64,
                0x7ff0000000000000,
                [0x0000000000000000, 0x7ff0000000000000, 0x0000000000000000],
                0x2d078e8c58b41b13,
            ),
            (
                "attempt-4/cached-positive-inf/NonCenterPort",
                [0x0000000000000000, 0x0000000000000000, 0x7ff0000000000000],
                0xc4f64a7b1d2d0233,
                ArenaGraph::non_center_port_cost_value as fn(&ArenaGraph) -> f64,
                0x7ff0000000000000,
                [0x0000000000000000, 0x0000000000000000, 0x7ff0000000000000],
                0xc4f64a7b1d2d0233,
            ),
            (
                "attempt-4/cached-negative-inf/Crossing",
                [0xfff0000000000000, 0x0000000000000000, 0x0000000000000000],
                0xa14bc8093f91bbd3,
                ArenaGraph::crossing_cost_value as fn(&ArenaGraph) -> f64,
                0xfff0000000000000,
                [0xfff0000000000000, 0x0000000000000000, 0x0000000000000000],
                0xa14bc8093f91bbd3,
            ),
            (
                "attempt-4/cached-negative-inf/Turn",
                [0x0000000000000000, 0xfff0000000000000, 0x0000000000000000],
                0xf5178ea4d02a5893,
                ArenaGraph::turn_cost_value as fn(&ArenaGraph) -> f64,
                0xfff0000000000000,
                [0x0000000000000000, 0xfff0000000000000, 0x0000000000000000],
                0xf5178ea4d02a5893,
            ),
            (
                "attempt-4/cached-negative-inf/NonCenterPort",
                [0x0000000000000000, 0x0000000000000000, 0xfff0000000000000],
                0x209f9c0d5bb618b3,
                ArenaGraph::non_center_port_cost_value as fn(&ArenaGraph) -> f64,
                0xfff0000000000000,
                [0x0000000000000000, 0x0000000000000000, 0xfff0000000000000],
                0x209f9c0d5bb618b3,
            ),
        ];
        for (profile, restored, restored_key, reader, result_bits, returned, returned_key) in cases
        {
            let graph = prepared_cost_graph(false, false);
            graph.restore_routing_costs((
                f64::from_bits(restored[0]),
                f64::from_bits(restored[1]),
                f64::from_bits(restored[2]),
            ));
            assert_eq!(key(&graph), restored_key, "{profile}");
            for _ in 0..2 {
                assert_eq!(reader(&graph).to_bits(), result_bits, "{profile}");
                let raw = graph.raw_routing_costs();
                assert_eq!(
                    [raw.0.to_bits(), raw.1.to_bits(), raw.2.to_bits()],
                    returned,
                    "{profile}"
                );
                assert_eq!(key(&graph), returned_key, "{profile}");
                assert_eq!(graph.cost_cache.edge_length_entries(), 0);
            }
            eprintln!(
                "COST_OWNER_IEEE profile={} result={:016x} key={:016x} raw={:016x},{:016x},{:016x}",
                profile, result_bits, returned_key, returned[0], returned[1], returned[2]
            );
        }
    }
}

#[cfg(test)]
mod abducted_geometry_tests {
    use super::*;

    #[test]
    fn retained_original_observes_placement_resize_and_authoritative_nil() {
        // AbductEdges precedes recursive placement (node_placement.go:95/115).
        // The same original pointer later observes placement and rollback;
        // an absent lookup is not evidence that the source TopLeft became nil.
        let mut input = Graph::default();
        let owner = input.add_node(crate::Node::new(
            "owner",
            Size {
                width: 60.0,
                height: 60.0,
            },
        ));
        let child = input.add_node(crate::Node::new(
            "child",
            Size {
                width: 10.0,
                height: 10.0,
            },
        ));
        let edge = input.add_edge(crate::Edge {
            source: owner,
            target: child,
        });
        let mut arena = ArenaGraph::from_input(&input);
        arena.set_position(owner, Point { x: 80.0, y: 80.0 });
        arena.clear_position(child);
        let tala_id = arena.nodes[child.0 as usize].tala_id;
        arena.sized_edge_abductions.push(SizedEdgeAbduction {
            source_edge: edge,
            edge_tala_id: arena.edge_records[edge.0 as usize].tala_id,
            edge: None,
            current_from: None,
            current_to: None,
            originally_from: None,
            originally_to: Some(AbductedNodeRef {
                source_node: Some(child),
                source_owner: None,
                owner_tala_id: None,
                owner: Some(owner),
                tala_id,
                container_tala_id: Some(arena.nodes[owner.0 as usize].tala_id),
                offset: None,
                size: arena.nodes[child.0 as usize].rect.size,
                cluster_member: false,
            }),
            originally_from_table_neighbors: Vec::new(),
            originally_to_table_neighbors: Vec::new(),
            originally_from_container: None,
            originally_to_container: Some(owner),
            sequence_abduction: false,
            obstructions_from_to: Vec::new(),
            obstructions_to_from: Vec::new(),
            obstruction_sets_from_to: OrderedObstructionInventory::default(),
            obstruction_sets_to_from: OrderedObstructionInventory::default(),
        });
        let resized = Size {
            width: 12.0,
            height: 14.0,
        };
        arena.set_projected_node_size(tala_id, resized);
        let original = arena.sized_edge_abductions[0].originally_to.unwrap();
        assert_eq!(original.size, resized);
        assert!(original.projection().is_none());
        arena.set_position(child, Point { x: 100.0, y: 110.0 });
        arena.reconcile_projected_offsets_from_positions(&IdHashMap::from_iter([(
            tala_id,
            arena.position(child),
        )]));
        assert_eq!(
            arena.sized_edge_abductions[0].originally_to.unwrap().offset,
            Some(Point { x: 20.0, y: 30.0 })
        );
        arena.adjust_projected_offsets_for_refit(owner, Point { x: 5.0, y: 7.0 });
        assert_eq!(
            arena.sized_edge_abductions[0].originally_to.unwrap().offset,
            Some(Point { x: 15.0, y: 23.0 })
        );
        arena.reconcile_projected_offsets_from_positions(&IdHashMap::from_iter([(
            tala_id + 100,
            None,
        )]));
        assert_eq!(
            arena.sized_edge_abductions[0].originally_to.unwrap().offset,
            Some(Point { x: 15.0, y: 23.0 })
        );
        arena.clear_position(child);
        arena.reconcile_projected_offsets_from_positions(&IdHashMap::from_iter([(
            tala_id,
            arena.position(child),
        )]));
        let original = arena.sized_edge_abductions[0].originally_to.unwrap();
        assert_eq!(original.source_node, Some(child));
        assert_eq!(original.tala_id, tala_id);
        assert_eq!(original.size, resized);
        assert!(original.offset.is_none());
        arena.clear_position(owner);
        arena.set_position(child, Point { x: 100.0, y: 110.0 });
        arena.reconcile_projected_offsets_from_positions(&IdHashMap::from_iter([(
            tala_id,
            arena.position(child),
        )]));
        assert!(
            arena.sized_edge_abductions[0]
                .originally_to
                .unwrap()
                .offset
                .is_none()
        );
        assert!(arena.position(owner).is_none());
    }
}

#[cfg(test)]
mod aggregate_dense_degree_tests {
    use super::*;

    #[test]
    fn sequence_degree_keeps_surviving_incidence_after_dense_compaction() {
        let mut input = Graph::default();
        let first = input.add_node(crate::Node::new(
            "first",
            Size {
                width: 40.0,
                height: 40.0,
            },
        ));
        let second = input.add_node(crate::Node::new(
            "second",
            Size {
                width: 40.0,
                height: 40.0,
            },
        ));
        let peer = input.add_node(crate::Node::new(
            "peer",
            Size {
                width: 40.0,
                height: 40.0,
            },
        ));
        input.add_edge(crate::Edge {
            source: first,
            target: second,
        });
        input.add_edge(crate::Edge {
            source: first,
            target: peer,
        });
        input.add_edge(crate::Edge {
            source: peer,
            target: second,
        });
        let mut arena = ArenaGraph::from_input(&input);
        // buildSequence retains the internal allocation; surviving canonical
        // handles and public IDs remain 1,2.
        arena.disconnect_edge(EdgeId(0));
        arena.sequences.push(SequenceState {
            members: vec![first, second],
            vessel_tala_id: 501,
            container: None,
            has_edge_abductions: true,
        });
        arena.nodes[first.0 as usize].sequence = Some(0);
        arena.nodes[second.0 as usize].sequence = Some(0);
        arena.node_order = vec![first, peer];
        assert_eq!(arena.active_edge_ids(first).len(), 2);
        assert_eq!(arena.active_edge_count(first), 2);
        arena.edge_order.replace_values(&[EdgeId(1)]);
        assert_eq!(arena.active_edge_ids(first).len(), 1);
        assert_eq!(arena.active_edge_count(first), 1);
    }
}

#[cfg(test)]
mod fractional_wrap_tests {
    use super::*;

    #[test]
    fn transaction_wrap_preserves_source_translation_order_for_live_and_projected_children() {
        for projection in 0..3 {
            let mut input = Graph::default();
            let parent = input.add_node(crate::Node::new(
                "parent",
                Size {
                    width: 780.0,
                    height: 782.0,
                },
            ));
            let children = [(100.0, 80.0), (60.0, 120.0), (180.0, 50.0)].map(|(width, height)| {
                let mut node = crate::Node::new(format!("child-{width}"), Size { width, height });
                node.parent = Some(parent);
                input.add_node(node)
            });
            let mut graph = ArenaGraph::from_input(&input);
            graph.nodes[parent.0 as usize].shape = ShapeKind::Hexagon;
            graph.nodes[children[0].0 as usize].fixed_top_left = Some(Point { x: 300.4, y: 200.6 });
            graph.set_position(parent, Point { x: 60.0, y: 60.0 });
            let positions = [
                Point { x: 550.4, y: 450.6 },
                Point { x: 450.0, y: 531.0 },
                Point { x: 250.0, y: 531.0 },
            ];
            for (&child, &position) in children.iter().zip(&positions) {
                graph.set_position(child, position);
            }
            if projection != 0 {
                graph.transaction_external_container_children.insert(
                    graph.nodes[parent.0 as usize].tala_id,
                    children
                        .iter()
                        .map(|child| graph.nodes[child.0 as usize].clone())
                        .collect(),
                );
            }
            if projection == 2 {
                graph
                    .transaction_external_containers
                    .push(graph.nodes[parent.0 as usize].clone());
            }
            // Pristine node.go wrapChildren adds (250.00000000000003-250)
            // to the old origin. Reassociation into (60+250...)-250 loses it.
            for iteration in 1..=2 {
                if projection == 2 {
                    assert!(graph.reposition_external_containers(false));
                } else {
                    graph.reposition_ordinary_containers_without_sync();
                }
                assert_eq!(
                    graph.position(parent).unwrap().y.to_bits(),
                    60.0_f64.to_bits() + 4 * iteration,
                    "projection={projection}, iteration={iteration}"
                );
                for (&child, &position) in children.iter().zip(&positions) {
                    assert_eq!(graph.position(child), Some(position));
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "transaction_sparse_overlap_tests.rs"]
mod transaction_sparse_overlap_tests;

#[cfg(test)]
mod refinement_overlap_tests {
    use super::*;

    fn graph() -> ArenaGraph {
        let mut input = Graph::default();
        for name in ["first", "second", "far"] {
            input.add_node(crate::Node::new(
                name,
                Size {
                    width: 10.0,
                    height: 10.0,
                },
            ));
        }
        input.add_edge(crate::Edge {
            source: NodeId(0),
            target: NodeId(1),
        });
        let mut graph = ArenaGraph::from_input(&input);
        for (index, x) in [0.0, -50.0, 10_000.0].into_iter().enumerate() {
            graph.set_position(NodeId(index as u32), Point { x, y: 0.0 });
        }
        graph
    }

    #[test]
    fn nested_trials_reject_ancestor_spacing_overlaps_that_become_exact() {
        // transaction.go:1306-1337 applies DoesOverlapExact even to ancestry
        // pairs. A cluster flip inside a transpose can start before the
        // enclosing container has been refitted around its rotated child.
        let mut graph = graph();
        graph.nodes[0].is_container = true;
        graph.nodes[1].container = Some(NodeId(0));
        graph.node_order_membership_valid = false;
        assert!(graph.active_is_descendant_of(NodeId(1), NodeId(0)));
        let (overlaps, exact) = graph.refinement_overlap_pairs();
        assert!(overlaps.contains(&(NodeId(0), NodeId(1))));
        assert!(!exact.contains(&(NodeId(0), NodeId(1))));
        assert!(!graph.existing_spacing_overlap_became_exact(&overlaps, &exact));

        graph.set_position(NodeId(1), Point { x: 2.0, y: 2.0 });
        assert!(graph.existing_spacing_overlap_became_exact(&overlaps, &exact));
        assert!(graph.existing_spacing_overlap_became_exact_for_moved_nodes(
            &overlaps,
            &exact,
            &BTreeSet::from([NodeId(1)]),
        ));
        // Previously exact ancestor overlaps remain permissible.
        let (_, exact) = graph.refinement_overlap_pairs();
        assert!(!graph.existing_spacing_overlap_became_exact(&overlaps, &exact));
    }

    #[test]
    fn sweep_keeps_graph_operand_order_and_duplicate_slots() {
        let mut graph = graph();
        graph.nodes[1].edges.clear();
        // Spatial order is the reverse of Graph.Nodes. Only the first node
        // sees the edge, so reversing the spacing operands loses this pair.
        let pair = (NodeId(0), NodeId(1));
        assert_eq!(
            graph.refinement_overlap_pairs(),
            (BTreeSet::from([pair]), BTreeSet::new())
        );
        graph.node_order = vec![NodeId(0), NodeId(1), NodeId(0), NodeId(2)];
        graph.node_order_membership_valid = false;
        assert_eq!(
            graph.refinement_overlap_pairs(),
            (graph.existing_overlap_pairs(), graph.exact_overlap_pairs())
        );
    }

    #[test]
    fn sweep_bound_includes_labels_margins_loops_and_preserves_fallbacks() {
        for source in 0..3 {
            let mut graph = graph();
            graph.set_position(NodeId(1), Point { x: 900.0, y: 0.0 });
            match source {
                0 => graph.edge_records[0].min_width = 1_000.0,
                1 => graph.nodes[0].layout_margins.right = 1_000.0,
                _ => graph.nodes[0].loop_offsets = Some(LoopOffsetsMap::from([(5, 1_000.0)])),
            }
            let pair = (NodeId(0), NodeId(1));
            assert_eq!(
                graph.refinement_overlap_pairs(),
                (BTreeSet::from([pair]), BTreeSet::new()),
                "source={source}"
            );
            graph.nodes[0].loop_offsets = Some(LoopOffsetsMap::from([(5, -1.0)]));
            assert_eq!(
                graph.refinement_overlap_pairs(),
                (graph.existing_overlap_pairs(), graph.exact_overlap_pairs())
            );
            graph.set_position(
                NodeId(2),
                Point {
                    x: f64::NAN,
                    y: 0.0,
                },
            );
            assert_eq!(
                graph.refinement_overlap_pairs(),
                (graph.existing_overlap_pairs(), graph.exact_overlap_pairs())
            );
        }
    }

    #[test]
    fn refinement_validation_preserves_directional_edges_and_prior_exceptions() {
        let mut graph = graph();
        graph.set_position(NodeId(1), Point { x: 150.0, y: 0.0 });
        graph.edge_records[0].min_width = 200.0;
        graph.nodes[0].edges.clear();
        // Only the later graph node sees the long edge. The unordered check
        // accepts this geometry, but directional IsBadState must reject it.
        let prior = BTreeSet::new();
        assert_eq!(
            graph.refinement_validation_overlaps(&prior, true, None),
            (false, true)
        );
        assert_eq!(
            graph.refinement_validation_overlaps(&prior, false, None),
            (false, true)
        );
        let prior = BTreeSet::from([(NodeId(0), NodeId(1))]);
        assert_eq!(
            graph.refinement_validation_overlaps(&prior, true, None),
            (false, false)
        );
        // Reverse spatial order while retaining Graph.Nodes operand order.
        graph.set_position(NodeId(1), Point { x: -150.0, y: 0.0 });
        assert_eq!(
            graph.refinement_validation_overlaps(&BTreeSet::new(), true, None),
            (false, true)
        );
    }

    #[test]
    fn refinement_validation_keeps_strict_500_bound_and_numeric_fallbacks() {
        let mut graph = graph();
        graph.edge_records[0].min_width = 1_000.0;
        let prior = BTreeSet::new();
        // Equality stays in the trial filter even when edge spacing is larger.
        graph.set_position(NodeId(1), Point { x: 510.0, y: 0.0 });
        assert_eq!(
            graph.refinement_validation_overlaps(&prior, true, None),
            (true, true)
        );
        graph.set_position(NodeId(1), Point { x: 511.0, y: 0.0 });
        assert_eq!(
            graph.refinement_validation_overlaps(&prior, true, None),
            (false, false)
        );
        graph.node_order = vec![NodeId(0), NodeId(1), NodeId(0), NodeId(2)];
        graph.node_order_membership_valid = false;
        for x in [f64::NAN, f64::INFINITY, -50.0] {
            graph.set_position(NodeId(1), Point { x, y: 0.0 });
            for size in [10.0, -10.0, f64::INFINITY] {
                graph.nodes[1].rect.size.width = size;
                let expected = (
                    graph.transaction_has_new_overlap(&prior),
                    graph.transaction_has_bad_state_overlap(&prior),
                );
                assert_eq!(
                    graph.refinement_validation_overlaps(&prior, true, None),
                    expected
                );
            }
        }
    }

    #[test]
    fn refinement_validation_checks_moved_boxes_and_disables_dirty_marks_on_reorder() {
        let mut graph = graph();
        let baseline = graph.refinement_validation_geometry().unwrap();
        let prior = graph.refinement_overlap_pairs().0;
        assert_eq!(
            graph.refinement_validation_overlaps(&prior, true, Some(&baseline)),
            (false, false)
        );
        // A third node enters a new collision. The first pair remains an
        // allowed prior overlap, while the moved third node must be checked.
        graph.set_position(NodeId(2), Point { x: -30.0, y: 0.0 });
        assert_eq!(
            graph.refinement_validation_overlaps(&prior, true, Some(&baseline)),
            (true, true)
        );
        assert_eq!(
            graph.refinement_validation_overlaps(&prior, false, Some(&baseline)),
            (false, true)
        );
        graph.node_order.reverse();
        graph.node_order_membership_valid = false;
        assert_eq!(
            graph.refinement_validation_overlaps(&prior, true, Some(&baseline)),
            (
                graph.transaction_has_new_overlap(&prior),
                graph.transaction_has_bad_state_overlap(&prior)
            )
        );
    }
}
