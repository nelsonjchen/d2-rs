// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Exclusive publication into the real owning arena between borrowed scores.
//!
//! Source: graph.go710–831 assigns all split roots before the first component
//! enters; node_placement.go151–211 keeps successful component Graph pointers
//! until the enclosing restore, but unwinds exact pre-split pointers on error.
//! node.go271–277 includes retained descendants outside Graph.Nodes.
//! transaction.go230–345 records node geometry from member roots, every live
//! Containers key/value, and retained group entities;1352–1415 restores those
//! objects in place. This journal covers node position/box and edge route
//! values, not the source's complete group policy/topology transaction.
//! Positions are the current source phase coordinates; publication adds no
//! ancestor origin. Inline Option<Point> is not Go heap Point pointer identity.

use super::limits::{OptimizationError, OptimizationWorkGuard};
use super::owner_groups::OwnerGroupKind;
use super::*;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(test)]
#[path = "owner_movement_tests.rs"]
mod movement_controls;

static NEXT_OWNER_SCORE_SESSION: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum OwnerScoreSessionError {
    Frame(OwnerScoreError),
    Topology(OwnerTopologyError),
    UnknownLocalNode(NodeId),
    UnknownLocalEdge(EdgeId),
    InvalidComponentToken,
    InvalidPreparedComponentToken,
    PreparedMappingMismatch,
    InvalidTrialToken,
    PendingComponent,
    PendingPreparedComponent,
    PendingTrial,
    UncapturedNode(NodeId),
    UncapturedEdge(EdgeId),
    MemberMappingMismatch,
    UnpositionedNode(NodeId),
    UnsupportedNestedVessel(NodeId),
}

impl From<OwnerScoreError> for OwnerScoreSessionError {
    fn from(error: OwnerScoreError) -> Self {
        Self::Frame(error)
    }
}
impl From<OwnerTopologyError> for OwnerScoreSessionError {
    fn from(error: OwnerTopologyError) -> Self {
        Self::Topology(error)
    }
}
impl fmt::Display for OwnerScoreSessionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "owning score session: {self:?}")
    }
}
impl std::error::Error for OwnerScoreSessionError {}

/// Explicit dense-local to owning-arena correspondence. Repeated canonical
/// nodes are allowed: sequential publication then has source last-write order.
/// No numeric equality, input ID, or Tala ID supplies a missing correspondence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct CanonicalScoreMapping {
    arena: OwnerArenaId,
    nodes: Vec<NodeId>,
    edges: Vec<EdgeId>,
}

impl CanonicalScoreMapping {
    pub(super) fn new(arena: OwnerArenaId, nodes: Vec<NodeId>, edges: Vec<EdgeId>) -> Self {
        Self {
            arena,
            nodes,
            edges,
        }
    }
    pub(super) fn arena(&self) -> OwnerArenaId {
        self.arena
    }
    pub(super) fn nodes(&self) -> &[NodeId] {
        &self.nodes
    }
    pub(super) fn edges(&self) -> &[EdgeId] {
        &self.edges
    }
    pub(super) fn canonical_node(&self, local: NodeId) -> Result<NodeId, OwnerScoreSessionError> {
        self.nodes
            .get(local.0 as usize)
            .copied()
            .ok_or(OwnerScoreSessionError::UnknownLocalNode(local))
    }
    pub(super) fn canonical_edge(&self, local: EdgeId) -> Result<EdgeId, OwnerScoreSessionError> {
        self.edges
            .get(local.0 as usize)
            .copied()
            .ok_or(OwnerScoreSessionError::UnknownLocalEdge(local))
    }
    /// child dense IDs point into the immediate parent's mapping, for both
    /// entities. This is required at every induced-component boundary.
    pub(super) fn compose(
        &self,
        parent_nodes: &[NodeId],
        parent_edges: &[EdgeId],
    ) -> Result<Self, OwnerScoreSessionError> {
        let nodes = parent_nodes
            .iter()
            .map(|&node| self.canonical_node(node))
            .collect::<Result<Vec<_>, _>>()?;
        let edges = parent_edges
            .iter()
            .map(|&edge| self.canonical_edge(edge))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self::new(self.arena, nodes, edges))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct OwnerNodeGeometry {
    pub(super) position: Option<Point>,
    pub(super) rect: Rect,
}

/// Membership-only NodeId set: dense flags for this owner's node ids, and an
/// ordered fallback for any other id. It is never iterated.
struct DenseNodeSet {
    flags: Vec<bool>,
    other: BTreeSet<NodeId>,
}
impl DenseNodeSet {
    fn new(nodes: usize) -> Self {
        Self {
            flags: vec![false; nodes],
            other: BTreeSet::new(),
        }
    }
    fn insert(&mut self, node: NodeId) -> bool {
        match self.flags.get_mut(node.0 as usize) {
            Some(flag) => !std::mem::replace(flag, true),
            None => self.other.insert(node),
        }
    }
    fn contains(&self, node: &NodeId) -> bool {
        match self.flags.get(node.0 as usize) {
            Some(&flag) => flag,
            None => self.other.contains(node),
        }
    }
}

impl OwnerNodeGeometry {
    /// TopLeft is the live point backing, when one is bound. The inline
    /// numerical field can still contain an earlier value or be unpositioned.
    pub(super) fn from_graph(graph: &ArenaGraph, node: NodeId) -> Self {
        let mut geometry = Self::from_node(&graph.nodes[node.0 as usize]);
        geometry.position = graph.position(node);
        geometry
    }

    pub(super) fn from_node(node: &ArenaNode) -> Self {
        Self {
            position: node.position,
            rect: node.rect,
        }
    }
}

/// optimization_guard.go762–960. Capture is inert: the enclosing optimizer
/// arms rollback only after its separately charged occupied-map copy succeeds.
/// Discovery follows the selected receiver's live container/group maps.
/// Cluster policy restores only the source's three scalar fields, on the
/// original group allocation. Herd values retain their original map headers.
#[derive(Debug, Default)]
pub(super) struct OwnerOptimizerMutationSnapshot {
    arena: Option<OwnerArenaId>,
    receiver: Option<OwnerGraphId>,
    nodes: Vec<(NodeId, OwnerNodeGeometry)>,
    point_snapshots: Vec<Option<super::shared_topology::OwnerNodePointSnapshot>>,
    seen_nodes: BTreeSet<NodeId>,
    clusters: Vec<(super::owner_groups::OwnerGroupId, String, String, f64)>,
    seen_clusters: BTreeSet<super::owner_groups::OwnerGroupId>,
    herds: Vec<(
        super::shared_topology::OwnerHerd,
        super::shared_topology::OwnerHerdFields,
    )>,
    seen_herds: BTreeSet<usize>,
    cost_snapshot: Option<PlacementCostSnapshot>,
    complete: bool,
}

impl OwnerOptimizerMutationSnapshot {
    /// optimization_guard.go814–939 takes the selected Graph directly.
    /// Capturing duplicate members or cyclic child references requires no
    /// dense scorer mapping; every enqueue still charges before deduplication.
    pub(super) fn capture_into(
        &mut self,
        owner: &mut ArenaGraph,
        receiver: OwnerGraphId,
        guard: &mut OptimizationWorkGuard<'_>,
    ) -> Result<(), OptimizationError> {
        self.release();
        let member_count = owner
            .owner_graph_view(receiver)
            .ok_or_else(|| {
                OptimizationError::Invalid(OwnerTopologyError::UnknownGraph(receiver).to_string())
            })?
            .nodes
            .len();
        if member_count > 10_000 {
            return Err(OptimizationError::Invalid(format!(
                "TALA {} snapshot node count exceeds limit 10000",
                guard.location()
            )));
        }
        let map = owner.owner_container_map(receiver).ok_or_else(|| {
            OptimizationError::Invalid(OwnerTopologyError::UnknownGraph(receiver).to_string())
        })?;
        use super::owner_groups::OwnerGroupKind;
        let clusters = owner
            .owner_graph_group_map(receiver, OwnerGroupKind::Cluster)
            .map_err(|e| OptimizationError::Invalid(e.to_string()))?;
        let sequences = owner
            .owner_graph_group_map(receiver, OwnerGroupKind::Sequence)
            .map_err(|e| OptimizationError::Invalid(e.to_string()))?;
        if map.len() > 10_001 || clusters.len() > 10_000 || sequences.len() > 10_000 {
            return Err(OptimizationError::Invalid(format!(
                "TALA {} optimizer topology maps exceed node limit 10000",
                guard.location()
            )));
        }
        let cache_entries = owner
            .owner_graph_edge_length_cache_entries(receiver)
            .map_err(|error| OptimizationError::Invalid(error.to_string()))?;
        if cache_entries > 1_000_000 {
            return Err(OptimizationError::Invalid(format!(
                "TALA {} edge-length cache entries exceed limit 1000000",
                guard.location()
            )));
        }
        for _ in 0..cache_entries {
            #[cfg(feature = "diagnostic-traces")]
            guard.step_with_owner(owner)?;
            #[cfg(not(feature = "diagnostic-traces"))]
            guard.step()?;
        }
        let costs = owner
            .owner_graph_snapshot_placement_costs(receiver)
            .map_err(|error| OptimizationError::Invalid(error.to_string()))?;
        #[cfg(feature = "diagnostic-traces")]
        guard.finish_with_owner(owner)?;
        #[cfg(not(feature = "diagnostic-traces"))]
        guard.finish()?;
        self.cost_snapshot = Some(costs);
        self.arena = owner.topology_arena_id();
        self.receiver = Some(receiver);

        // Go's range retains this header once, after cost Finish. Every
        // nullable pointer slot is loaded from its live backing before its
        // enqueue Step; a callback can edit future slots or replace the live
        // Graph.Nodes field without retargeting this range.
        let members = owner
            .owner_graph_member_header(receiver)
            .map_err(|error| OptimizationError::Invalid(error.to_string()))?;
        for member_index in 0..members.len() {
            let node = members
                .get(member_index)
                .expect("retained member header bound");
            self.enqueue(owner, node, guard)?;
        }
        let mut index = 0;
        while index < self.nodes.len() {
            let node = self.nodes[index].0;
            // No IsContainer condition: source captures even a non-container
            // parent's keyed child header. Each slot is loaded before its
            // enqueue Step, with all backing locks released before callbacks.
            if let Some(children) = owner
                .owner_container_map(receiver)
                .expect("validated receiver")
                .get(Some(node))
            {
                if children.len() > 10_000 {
                    return Err(OptimizationError::Invalid(format!(
                        "TALA {} container child references exceed node limit 10000",
                        guard.location()
                    )));
                }
                for child_index in 0..children.len() {
                    let child = children.get(child_index).expect("live logical header slot");
                    self.enqueue(owner, child, guard)?;
                }
            }
            // These are keyed Graph maps, independent of the node's group
            // membership pointer or private vessel flag. Each range retains
            // its own current member header before any enqueue callbacks.
            for (kind, name) in [
                (OwnerGroupKind::Cluster, "cluster"),
                (OwnerGroupKind::Sequence, "sequence"),
            ] {
                let group = owner
                    .owner_graph_group_entry(receiver, kind, Some(node))
                    .map_err(|e| OptimizationError::Invalid(e.to_string()))?
                    .flatten();
                if let Some(group) = group {
                    let children = owner
                        .owner_group_fields(group)
                        .map_err(|e| OptimizationError::Invalid(e.to_string()))?
                        .nodes;
                    if children.len() > 10_000 {
                        return Err(OptimizationError::Invalid(format!(
                            "TALA {} {} child references exceed node limit 10000",
                            guard.location(),
                            name
                        )));
                    }
                    for child_index in 0..children.len() {
                        let child = children.get(child_index).expect("live group header slot");
                        self.enqueue(owner, child, guard)?;
                    }
                }
            }
            index += 1;
        }
        // Source ranges the map selected now; its values are loaded before
        // Step. A callback may replace the Graph map without retargeting this
        // range, and may mutate a selected group's policy before it is saved.
        let clusters = owner
            .owner_graph_group_map(receiver, OwnerGroupKind::Cluster)
            .map_err(|e| OptimizationError::Invalid(e.to_string()))?;
        let values = owner
            .owner_group_map_values(&clusters)
            .map_err(|e| OptimizationError::Invalid(e.to_string()))?;
        for cluster in values {
            #[cfg(feature = "diagnostic-traces")]
            guard.step_with_owner(owner)?;
            #[cfg(not(feature = "diagnostic-traces"))]
            guard.step()?;
            let Some(cluster) = cluster else { continue };
            if self.seen_clusters.insert(cluster) {
                let policy = owner
                    .owner_cluster_policy(cluster)
                    .map_err(|e| OptimizationError::Invalid(e.to_string()))?;
                self.clusters.push((
                    cluster,
                    policy.arrangement,
                    policy.desired_arrangement,
                    policy.padding,
                ));
            }
        }
        // Every captured node still incurs its HerdAssignment Step.
        for &(node, _) in &self.nodes {
            #[cfg(feature = "diagnostic-traces")]
            guard.step_with_owner(owner)?;
            #[cfg(not(feature = "diagnostic-traces"))]
            guard.step()?;
            let herd = match owner.owner_node_herd(node) {
                Ok(herd) => herd,
                Err(OwnerTopologyError::UnboundHerdAssignment(_)) => {
                    return Err(OptimizationError::Invalid(format!(
                        "TALA {} owner optimizer snapshot has unsupported herd bindings",
                        guard.location()
                    )));
                }
                Err(error) => return Err(OptimizationError::Invalid(error.to_string())),
            };
            if let Some(herd) = herd {
                if self.seen_herds.insert(herd.allocation_key()) {
                    let fields = owner
                        .owner_herd_fields(&herd)
                        .map_err(|e| OptimizationError::Invalid(e.to_string()))?;
                    self.herds.push((herd, fields));
                }
            }
        }
        self.complete = true;
        Ok(())
    }

    /// Release saved references and identities, retaining request-local vector
    /// storage. Dropping or releasing this value never restores mutations.
    pub(super) fn release(&mut self) {
        self.nodes.clear();
        self.point_snapshots.clear();
        self.seen_nodes.clear();
        self.clusters.clear();
        self.seen_clusters.clear();
        self.herds.clear();
        self.seen_herds.clear();
        self.cost_snapshot = None;
        self.arena = None;
        self.receiver = None;
        self.complete = false;
    }

    pub(super) fn is_complete(&self) -> bool {
        self.complete
    }

    pub(super) fn receiver(&self) -> Option<OwnerGraphId> {
        self.receiver
    }

    /// Borrow the actual saved costs, including an interrupted scratch capture.
    /// Diagnostic access never creates a snapshot or arms its restoration.
    #[cfg(any(test, feature = "diagnostic-traces"))]
    pub(super) fn cost_snapshot(&self) -> Option<&PlacementCostSnapshot> {
        self.cost_snapshot.as_ref()
    }

    /// An interrupted capture may expose its scratch prefix for diagnostics;
    /// it still has no successful returned snapshot and cannot restore it.
    pub(super) fn captured_nodes(&self) -> impl ExactSizeIterator<Item = NodeId> + '_ {
        self.nodes.iter().map(|&(node, _)| node)
    }

    pub(super) fn captured_geometry(
        &self,
    ) -> impl ExactSizeIterator<Item = (NodeId, OwnerNodeGeometry)> + '_ {
        self.nodes.iter().copied()
    }

    fn enqueue(
        &mut self,
        owner: &mut ArenaGraph,
        node: Option<NodeId>,
        guard: &mut OptimizationWorkGuard<'_>,
    ) -> Result<(), OptimizationError> {
        #[cfg(feature = "diagnostic-traces")]
        guard.step_with_owner(owner)?;
        #[cfg(not(feature = "diagnostic-traces"))]
        guard.step()?;
        let node = node.ok_or_else(|| {
            OptimizationError::Invalid(format!("TALA {} found a nil graph node", guard.location()))
        })?;
        if self.seen_nodes.contains(&node) {
            return Ok(());
        }
        if self.seen_nodes.len() >= 10_000 {
            return Err(OptimizationError::Invalid(format!(
                "TALA {} snapshot node count exceeds limit 10000",
                guard.location()
            )));
        }
        let live = owner.topology_node(node).ok_or_else(|| {
            OptimizationError::Invalid(OwnerTopologyError::UnknownNode(node).to_string())
        })?;
        self.seen_nodes.insert(node);
        let mut geometry = OwnerNodeGeometry::from_node(live);
        geometry.position = owner.position(node);
        let point = match owner.owner_node_point(node) {
            Ok(p) => Some(p.snapshot()),
            Err(OwnerTopologyError::UnboundNodePoint(_)) => None,
            Err(e) => return Err(OptimizationError::Invalid(e.to_string())),
        };
        self.nodes.push((node, geometry));
        self.point_snapshots.push(point);
        Ok(())
    }

    /// Restore only a completed capture, into its actual original owner arena.
    /// The saved cost owner/optional map remains the one captured, even if a
    /// caller has since selected another view. Restore itself is uncharged.
    pub(super) fn restore(&self, owner: &mut ArenaGraph) -> Result<(), OptimizationError> {
        if !self.complete {
            return Ok(());
        }
        if owner.topology_arena_id() != self.arena {
            return Err(OptimizationError::Invalid(
                OwnerTopologyError::ForeignArena {
                    expected: self.arena,
                    actual: owner.topology_arena_id(),
                }
                .to_string(),
            ));
        }
        // Frozen binding validation precedes every write, so an internal stale
        // arena cannot publish a partial restore or mask the original failure.
        for &(node, _) in &self.nodes {
            if owner.topology_node(node).is_none() {
                return Err(OptimizationError::Invalid(
                    OwnerTopologyError::UnknownNode(node).to_string(),
                ));
            }
        }
        for (cluster, _, _, _) in &self.clusters {
            owner
                .owner_cluster_policy(*cluster)
                .map_err(|e| OptimizationError::Invalid(e.to_string()))?;
        }
        for (herd, _) in &self.herds {
            owner
                .owner_herd_fields(herd)
                .map_err(|e| OptimizationError::Invalid(e.to_string()))?;
        }
        let costs = self.cost_snapshot.as_ref().ok_or_else(|| {
            OptimizationError::Invalid("completed owner optimizer snapshot has no costs".into())
        })?;
        for (index, &(node, geometry)) in self.nodes.iter().enumerate() {
            if let Some(point) = &self.point_snapshots[index] {
                owner
                    .restore_owner_node_point(node, point)
                    .map_err(|e| OptimizationError::Invalid(e.to_string()))?;
            } else {
                owner.nodes[node.0 as usize].position = geometry.position;
            }
            owner.nodes[node.0 as usize].rect = geometry.rect;
        }
        for (cluster, arrangement, desired, padding) in &self.clusters {
            let mut policy = owner
                .owner_cluster_policy(*cluster)
                .map_err(|e| OptimizationError::Invalid(e.to_string()))?;
            policy.arrangement.clone_from(arrangement);
            policy.desired_arrangement.clone_from(desired);
            policy.padding = *padding;
            owner
                .set_owner_cluster_policy(*cluster, policy)
                .map_err(|e| OptimizationError::Invalid(e.to_string()))?;
        }
        for (herd, fields) in &self.herds {
            owner
                .set_owner_herd_fields(herd, fields.clone())
                .map_err(|e| OptimizationError::Invalid(e.to_string()))?;
        }
        costs.restore();
        Ok(())
    }
}

/// These serials enforce LIFO/receiver control only. They are never presented
/// as an allocation address, node pointer, or evidence of physical aliasing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct OwnerComponentToken {
    session: u64,
    serial: u64,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct OwnerGeometryTrialToken {
    session: u64,
    serial: u64,
}

/// candidate_movement.go:20–73. The stable movement set and its positional
/// rollback belong to one candidate search, rather than the enclosing scope.
/// This borrows the actual owner exclusively; fresh scoring frames can be
/// borrowed through `session()` between moves. Explicit point bindings preserve
/// actual allocation identity; unbound legacy fields retain value restoration.
pub(super) struct OwnerCandidateMovement<'session, 'owner> {
    session: &'session mut OwnerScoreSession<'owner>,
    positions: Vec<(NodeId, Option<Point>, Point)>,
    point_snapshots: Vec<Option<super::shared_topology::OwnerNodePointSnapshot>>,
    descendant_work: u64,
    complete: bool,
}

impl<'session, 'owner> OwnerCandidateMovement<'session, 'owner> {
    pub(super) fn captured_nodes(&self) -> impl ExactSizeIterator<Item = NodeId> + '_ {
        self.positions.iter().map(|position| position.0)
    }

    pub(super) fn descendant_work(&self) -> u64 {
        self.descendant_work
    }

    pub(super) fn session(&mut self) -> &mut OwnerScoreSession<'owner> {
        self.session
    }

    pub(super) fn move_abs(
        &mut self,
        point: Point,
        guard: &mut OptimizationWorkGuard<'_>,
    ) -> Result<(), OptimizationError> {
        let root = self.positions[0].0;
        let current = self.session.owner.position(root).ok_or_else(|| {
            OptimizationError::Invalid(format!(
                "TALA {} cannot move an unpositioned node",
                guard.location()
            ))
        })?;
        if current == point {
            return step_candidate_owner(self.session.owner, guard);
        }
        for _ in 0..self.descendant_work {
            step_candidate_owner(self.session.owner, guard)?;
        }
        for &(node, _, _) in &self.positions[1..] {
            step_candidate_owner(self.session.owner, guard)?;
            if self.session.owner.position(node).is_none() {
                return Err(OptimizationError::Invalid(format!(
                    "TALA {} cannot move an unpositioned descendant",
                    guard.location()
                )));
            }
        }
        // candidate_movement.go: the root position is reread after replay and
        // descendant-validation polls. A callback can change it during either.
        let current = self
            .session
            .owner
            .position(root)
            .expect("runtime error: invalid memory address or nil pointer dereference");
        let delta = Point {
            x: point.x - current.x,
            y: point.y - current.y,
        };
        // The root translates before the first descendant mutation poll. A
        // failure here is a real partial move, restored by this search's Drop.
        self.translate(root, delta);
        for index in 1..self.positions.len() {
            step_candidate_owner(self.session.owner, guard)?;
            self.translate(self.positions[index].0, delta);
        }
        Ok(())
    }

    fn translate(&mut self, node: NodeId, delta: Point) {
        // Translate uses addition even for the root: assigning the requested
        // target instead would alter floating-point rounding.
        self.session
            .owner
            .translate_owner_point(node, delta.x, delta.y);
    }

    pub(super) fn commit(mut self) {
        self.complete = true;
    }
}

fn step_candidate_owner(
    owner: &mut ArenaGraph,
    guard: &mut OptimizationWorkGuard<'_>,
) -> Result<(), OptimizationError> {
    #[cfg(feature = "diagnostic-traces")]
    return guard.step_with_owner(owner);
    #[cfg(not(feature = "diagnostic-traces"))]
    {
        let _ = owner;
        guard.step()
    }
}

impl Drop for OwnerCandidateMovement<'_, '_> {
    fn drop(&mut self) {
        if !self.complete {
            for (index, &(node, position, origin)) in self.positions.iter().enumerate() {
                if let Some(snapshot) = &self.point_snapshots[index] {
                    self.session
                        .owner
                        .restore_owner_node_point(node, snapshot)
                        .expect("captured owner point scope");
                } else {
                    self.session.owner.nodes[node.0 as usize].position = position;
                }
                self.session.owner.nodes[node.0 as usize].rect.origin = origin;
            }
        }
    }
}

/// placement/types.go snapshotNodePositionsContext: retain the selected
/// roots' point headers and values, including descendants outside Graph.Nodes.
/// Each rejected subgraph translation restores these exact values; applying
/// the opposite translation would accumulate floating-point error.
pub(super) struct OwnerCompactionPositions<'session, 'owner> {
    session: &'session mut OwnerScoreSession<'owner>,
    positions: Vec<(
        NodeId,
        Option<Point>,
        Point,
        Option<super::shared_topology::OwnerNodePointSnapshot>,
    )>,
    entry: Option<super::compaction_checkpoint::CapturedCompactionState>,
    selected: Option<super::compaction_checkpoint::CapturedCompactionPositions>,
    complete: bool,
}
impl<'owner> OwnerCompactionPositions<'_, 'owner> {
    pub(super) fn session(&mut self) -> &mut OwnerScoreSession<'owner> {
        self.session
    }
    pub(super) fn commit(&mut self) {
        self.complete = true;
    }
    pub(super) fn restore(&mut self) {
        if let Some(state) = &self.entry {
            state.restore(self.session.owner);
            return;
        }

        if let Some(selected) = &self.selected {
            selected.restore(self.session.owner);
            return;
        }
        for (node, position, origin, point) in &self.positions {
            if let Some(point) = point {
                self.session
                    .owner
                    .restore_owner_node_point(*node, point)
                    .expect("captured compaction point scope");
            } else {
                self.session.owner.nodes[node.0 as usize].position = *position;
            }
            self.session.owner.nodes[node.0 as usize].rect.origin = *origin;
        }
    }
}
impl Drop for OwnerCompactionPositions<'_, '_> {
    fn drop(&mut self) {
        if self.complete {
            return;
        }
        self.restore();
    }
}

/// Source split creates every graph and assigns its reached root pointers
/// before the first component is placed. This token carries explicit dense
/// correspondence, independently of later descendant SetGraphReference.
#[derive(Clone, Debug)]
pub(super) struct PreparedOwnerComponent {
    session: u64,
    serial: u64,
    parent_view: OwnerGraphId,
    mapping: CanonicalScoreMapping,
    roots: Vec<NodeId>,
    view: OwnerGraphId,
    root_journal: usize,
}

impl PreparedOwnerComponent {
    pub(super) fn view(&self) -> OwnerGraphId {
        self.view
    }
}

#[derive(Debug)]
struct ComponentEntry {
    token: OwnerComponentToken,
    previous_view: OwnerGraphId,
    previous_mapping: CanonicalScoreMapping,
    journal: usize,
    root_journal: usize,
}
#[derive(Debug)]
struct GeometryTrial {
    token: OwnerGeometryTrialToken,
    nodes: Vec<(NodeId, OwnerNodeGeometry)>,
    edges: Vec<(EdgeId, Vec<Point>)>,
}

/// The arena is borrowed exclusively, never cloned or replaced. Each frame is
/// a short immutable reborrow; Rust prevents mutation until its joined closure
/// has returned. Independent dense candidates remain independent objects.
#[derive(Debug)]
pub(super) struct OwnerScoreSession<'a> {
    owner: &'a mut ArenaGraph,
    current_view: OwnerGraphId,
    mapping: CanonicalScoreMapping,
    abductions: Option<&'a OwnerAbductionSlice>,
    session: u64,
    next_serial: u64,
    components: Vec<ComponentEntry>,
    pending_prepared: BTreeSet<u64>,
    // Creation order, including already-left successful nested components.
    // Outer failure restores these in reverse creation order, not pop order.
    ownership: Vec<Option<OwnerGraphOwnershipJournal>>,
    // Successful shorter sessions transfer journals back to the enclosing
    // exclusive borrow. Its later failure must still unwind those pointers.
    parent_ownership: Option<&'a mut Vec<Option<OwnerGraphOwnershipJournal>>>,
    ownership_committed: bool,
    trials: Vec<GeometryTrial>,
}

impl<'a> OwnerScoreSession<'a> {
    pub(super) fn new(
        owner: &'a mut ArenaGraph,
        view: OwnerGraphId,
        mapping: CanonicalScoreMapping,
        abductions: Option<&'a OwnerAbductionSlice>,
    ) -> Result<Self, OwnerScoreSessionError> {
        let frame = OwnerScoreFrame::new_with_edge_mapping(
            owner,
            mapping.arena,
            view,
            &mapping.nodes,
            &mapping.edges,
        )?;
        if let Some(slice) = abductions {
            frame.with_abductions(slice)?;
        }
        for &edge in &mapping.edges {
            if owner.edge_records.get(edge.0 as usize).is_none() {
                return Err(OwnerTopologyError::UnknownEdge(edge).into());
            }
        }
        Ok(Self {
            owner,
            current_view: view,
            mapping,
            abductions,
            session: NEXT_OWNER_SCORE_SESSION.fetch_add(1, Ordering::Relaxed),
            next_serial: 0,
            components: Vec::new(),
            pending_prepared: BTreeSet::new(),
            ownership: Vec::new(),
            parent_ownership: None,
            ownership_committed: false,
            trials: Vec::new(),
        })
    }

    pub(super) fn owner(&self) -> &ArenaGraph {
        self.owner
    }
    pub(super) fn owner_mut(&mut self) -> &mut ArenaGraph {
        self.owner
    }
    pub(super) fn current_view(&self) -> OwnerGraphId {
        self.current_view
    }
    pub(super) fn current_mapping(&self) -> &CanonicalScoreMapping {
        &self.mapping
    }

    /// packing/combine.go248–253/388–393: roots followed by each root's
    /// masterGraph descendants, with AddNodeUnchecked changing Node.Graph.
    /// Keep canonical allocation indices in the numerical candidate so every
    /// retained Container, Near and incident edge pointer keeps its meaning.
    /// Geometry beyond the roots must be supplied explicitly by the caller;
    /// this boundary does not select among differently framed projection aliases.
    pub(super) fn combined_member_workspace(
        &self,
        workspace: &ArenaGraph,
        member_geometry: &[(NodeId, OwnerNodeGeometry)],
    ) -> Result<(ArenaGraph, OwnerGraphId), OwnerScoreSessionError> {
        let mut nodes = Vec::new();
        let mut roots = Vec::new();
        for local in workspace.graph_node_order() {
            let canonical = self.canonical_node(local)?;
            roots.push((local, canonical));
            nodes.push(canonical);
            nodes.extend(self.live_descendants(self.current_view, Some(canonical), true)?);
        }
        let edges = workspace
            .edge_order
            .read()
            .iter()
            .map(|&edge| self.canonical_edge(edge))
            .collect::<Result<Vec<_>, _>>()?;
        let mut candidate = self.owner.clone();
        let combined = candidate.create_owner_graph_view(nodes.clone(), edges.clone())?;
        candidate.set_owner_view_node_mapping(
            combined,
            (0..candidate.nodes.len())
                .map(|index| NodeId(index as u32))
                .collect(),
        )?;
        for kind in [OwnerGroupKind::Cluster, OwnerGroupKind::Sequence] {
            candidate.copy_owner_graph_group_map_from(combined, self.current_view, kind)?;
        }
        candidate.copy_owner_graph_directions_from(combined, self.current_view)?;
        candidate.copy_owner_graph_tree_roots_from(combined, self.current_view)?;
        candidate.copy_owner_graph_node_to_tree_from(combined, self.current_view)?;
        candidate.share_owner_proximity_maps(self.current_view, combined)?;
        for &node in &nodes {
            candidate.set_owner_node_graph_reference(node, Some(combined))?;
        }
        // Roots are in the already packed numerical frame. Retain all their
        // canonical reference fields and copy only their actual box geometry.
        for (local, canonical) in roots {
            let mut geometry = OwnerNodeGeometry::from_node(&workspace.nodes[local.0 as usize]);
            geometry.position = workspace.position(local);
            if let Some(position) = geometry.position {
                candidate.set_position(canonical, position);
            } else {
                candidate.clear_position(canonical);
            }
            candidate.nodes[canonical.0 as usize].rect = geometry.rect;
        }
        let selected = nodes.iter().copied().collect::<BTreeSet<_>>();
        for &(canonical, geometry) in member_geometry {
            if !selected.contains(&canonical) {
                return Err(OwnerScoreSessionError::UncapturedNode(canonical));
            }
            if let Some(position) = geometry.position {
                candidate.set_position(canonical, position);
            } else {
                candidate.clear_position(canonical);
            }
            candidate.nodes[canonical.0 as usize].rect = geometry.rect;
        }
        candidate.node_order = nodes;
        candidate.edge_order.replace_values(&edges);
        // CopyEntitiesFrom does not copy the parent's cell/cost caches.
        candidate.cell_size = candidate.owner_graph_cell_size(combined)?;
        candidate.cost_cache = candidate.owner_graph_cost_cache(combined)?.clone();
        candidate.refresh_node_order_membership();
        Ok((candidate, combined))
    }

    /// Current aggregate projections carry an explicit canonical source handle.
    /// Their boxes are already in the packed scope frame; retained owner boxes
    /// can still be in the earlier sized workspace frame. Never infer a source
    /// handle from a display name, entity number or matching coordinates.
    pub(super) fn packed_group_member_geometry(
        &self,
        workspace: &ArenaGraph,
    ) -> Result<Vec<(NodeId, OwnerNodeGeometry)>, OwnerScoreSessionError> {
        let mut roots = DenseNodeSet::new(self.owner.nodes.len());
        let mut selected = DenseNodeSet::new(self.owner.nodes.len());
        let mut stack = Vec::new();
        for local in workspace.graph_node_order() {
            let canonical = self.canonical_node(local)?;
            roots.insert(canonical);
            self.extend_live_descendant_set(self.current_view, canonical, &mut selected, &mut stack)?;
        }
        let mut geometry = BTreeMap::new();
        #[cfg(feature = "diagnostic-traces")]
        if let Some(ids) = crate::internal::trace_env_value("D2RS_TRACE_PACKED_OWNER_PROJECTION") {
            for (kind, children) in [
                (
                    "aggregate",
                    &workspace.transaction_external_aggregate_children,
                ),
                (
                    "container",
                    &workspace.transaction_external_container_children,
                ),
            ] {
                for (parent, children) in children {
                    for member in children {
                        if ids
                            .split(',')
                            .any(|id| id.parse::<i64>().ok() == Some(member.tala_id))
                        {
                            eprintln!(
                                "PACKED_OWNER_PROJECTION kind={} parent={} tala={} source={:?} anchor={:?} position={:?} selected={} root={}",
                                kind,
                                parent,
                                member.tala_id,
                                member.owner_geometry_source,
                                member.owner_geometry_anchor,
                                member.position,
                                member
                                    .owner_geometry_source
                                    .is_some_and(|node| selected.contains(&node)),
                                member
                                    .owner_geometry_source
                                    .is_some_and(|node| roots.contains(&node))
                            );
                        }
                    }
                }
            }
        }
        for member in workspace
            .transaction_external_aggregate_children
            .values()
            .flatten()
            .chain(
                workspace
                    .transaction_external_container_children
                    .values()
                    .flatten(),
            )
        {
            let Some(canonical) = member.owner_geometry_source else {
                continue;
            };
            if !selected.contains(&canonical) || roots.contains(&canonical) {
                continue;
            }
            let value = OwnerNodeGeometry::from_node(member);
            if let Some(previous) = geometry.insert(canonical, value)
                && previous != value
            {
                return Err(OwnerScoreSessionError::MemberMappingMismatch);
            }
        }
        Ok(geometry.into_iter().collect())
    }

    /// Prepare the full combined workspace from the current rotated/synced
    /// numerical projection, including its actual cluster policy mutations.
    pub(super) fn combined_transition_workspace(
        &self,
        workspace: &ArenaGraph,
    ) -> Result<(ArenaGraph, OwnerGraphId), OwnerScoreSessionError> {
        let geometry = self.packed_group_member_geometry(workspace)?;
        let (mut combined, receiver) = self.combined_member_workspace(workspace, &geometry)?;
        for local in workspace.graph_node_order() {
            let canonical = self.canonical_node(local)?;
            let Some(group) = combined
                .owner_graph_group_entry(receiver, OwnerGroupKind::Cluster, Some(canonical))?
                .flatten()
            else {
                continue;
            };
            let mut policy = combined.owner_cluster_policy(group)?;
            if let Some(index) = workspace.active_cluster_index(local) {
                let cluster = &workspace.clusters[index];
                policy.arrangement = match cluster.arrangement {
                    ClusterArrangement::Row => "Row",
                    ClusterArrangement::Column => "Column",
                }
                .into();
                policy.desired_arrangement = match cluster.desired_arrangement {
                    ClusterArrangement::Row => "Row",
                    ClusterArrangement::Column => "Column",
                }
                .into();
                policy.padding = cluster.padding;
            } else if let Some(layout) = workspace
                .transaction_external_cluster_layouts
                .get(&workspace.nodes[local.0 as usize].tala_id)
            {
                policy.arrangement = match layout.arrangement {
                    ClusterArrangement::Row => "Row",
                    ClusterArrangement::Column => "Column",
                }
                .into();
                policy.desired_arrangement = policy.arrangement.clone();
                policy.padding = layout.padding;
            }
            combined.set_owner_cluster_orientation_policy(
                group,
                policy.arrangement,
                policy.desired_arrangement,
                policy.padding,
            )?;
        }
        Ok((combined, receiver))
    }

    /// SplitSubgraphs/direct retain Source pointers rather than copying stale
    /// projected boxes. Resolve only explicit owner handles, leave the
    /// numerical transport unmodified unless all of them are valid, and
    /// retain nil geometry.
    pub(super) fn refresh_explicit_owner_projections(
        &self,
        workspace: &mut ArenaGraph,
    ) -> Result<(), OwnerScoreSessionError> {
        // The owner is immutably borrowed for the entire refresh. Each
        // carrier's explicit handles are validated, in the original order,
        // before that carrier is written, then its current geometry is read
        // directly. An unknown handle restores the plain fields of every
        // carrier already written, so an error still leaves the numerical
        // transport unmodified, exactly as a separate validation pass did.
        let mut written = Vec::new();
        let mut unknown = None;
        for node in workspace
            .transaction_external_containers
            .iter_mut()
            .chain(
                workspace
                    .transaction_external_container_children
                    .values_mut()
                    .flatten(),
            )
            .chain(
                workspace
                    .transaction_external_aggregate_children
                    .values_mut()
                    .flatten(),
            )
        {
            unknown = [node.owner_geometry_source, node.owner_geometry_anchor]
                .into_iter()
                .flatten()
                .find(|&canonical| self.owner.topology_node(canonical).is_none());
            if unknown.is_some() {
                break;
            }
            let Some(canonical) = node.owner_geometry_source else {
                written.push(None);
                continue;
            };
            written.push(Some((
                node.position,
                node.rect,
                node.transaction_anchor_offset,
            )));
            let geometry = OwnerNodeGeometry::from_graph(self.owner, canonical);
            node.position = geometry.position;
            node.rect = geometry.rect;
            // Derived offsets cannot recreate a now-nil Source TopLeft.
            node.transaction_anchor_offset = geometry.position.and_then(|point| {
                node.owner_geometry_anchor
                    .and_then(|anchor| self.owner.position(anchor))
                    .map(|origin| Point {
                        x: point.x - origin.x,
                        y: point.y - origin.y,
                    })
            });
            if let Some(point) = geometry.position {
                node.rect.origin = point;
            }
        }
        if let Some(canonical) = unknown {
            for (node, prior) in workspace
                .transaction_external_containers
                .iter_mut()
                .chain(
                    workspace
                        .transaction_external_container_children
                        .values_mut()
                        .flatten(),
                )
                .chain(
                    workspace
                        .transaction_external_aggregate_children
                        .values_mut()
                        .flatten(),
                )
                .zip(written)
            {
                if let Some((position, rect, offset)) = prior {
                    node.position = position;
                    node.rect = rect;
                    node.transaction_anchor_offset = offset;
                }
            }
            return Err(OwnerTopologyError::UnknownNode(canonical).into());
        }
        Ok(())
    }

    pub(super) fn project_combined_geometry(
        &self,
        combined: &ArenaGraph,
        workspace: &mut ArenaGraph,
    ) -> Result<(), OwnerScoreSessionError> {
        let selected = combined
            .graph_node_order()
            .into_iter()
            .collect::<BTreeSet<_>>();
        for local in workspace.graph_node_order() {
            let canonical = self.canonical_node(local)?;
            let geometry = OwnerNodeGeometry::from_graph(combined, canonical);
            if let Some(point) = geometry.position {
                workspace.set_position(local, point);
            } else {
                workspace.clear_position(local);
            }
            workspace.nodes[local.0 as usize].position = geometry.position;
            workspace.nodes[local.0 as usize].rect = geometry.rect;
        }
        for member in workspace
            .transaction_external_aggregate_children
            .values_mut()
            .flatten()
        {
            if let Some(canonical) = member.owner_geometry_source
                && selected.contains(&canonical)
            {
                let geometry = OwnerNodeGeometry::from_graph(combined, canonical);
                member.position = geometry.position;
                member.rect = geometry.rect;
            }
        }
        Ok(())
    }

    pub(super) fn publish_combined_orientation(
        &mut self,
        combined: &ArenaGraph,
        receiver: OwnerGraphId,
    ) -> Result<(), OwnerScoreSessionError> {
        let values = combined
            .graph_node_order()
            .into_iter()
            .map(|node| (node, OwnerNodeGeometry::from_graph(combined, node)))
            .collect::<Vec<_>>();
        self.publish_direct_canonical_geometry(&values)?;
        for node in combined.graph_node_order() {
            if let Some(group) = combined
                .owner_graph_group_entry(receiver, OwnerGroupKind::Cluster, Some(node))?
                .flatten()
            {
                let policy = combined.owner_cluster_policy(group)?;
                self.owner.set_owner_cluster_orientation_policy(
                    group,
                    policy.arrangement,
                    policy.desired_arrangement,
                    policy.padding,
                )?;
            }
        }
        Ok(())
    }

    /// Publish the numerical packed boxes to the original pointer objects.
    /// Translation alone preserves stale member offsets when an earlier sized
    /// projection has not yet reached the owner. Explicit member lineage supplies
    /// those current boxes without replaying cluster/container synchronization.
    pub(super) fn publish_packed_scope_geometry(
        &mut self,
        workspace: &ArenaGraph,
    ) -> Result<(), OwnerScoreSessionError> {
        let members = self.packed_group_member_geometry(workspace)?;
        self.update_current_member_positions(workspace)?;
        let mut geometry = workspace
            .graph_node_order()
            .into_iter()
            .map(|local| {
                let canonical = self.canonical_node(local)?;
                let mut value = OwnerNodeGeometry::from_node(&workspace.nodes[local.0 as usize]);
                value.position = workspace.position(local);
                Ok((canonical, value))
            })
            .collect::<Result<Vec<_>, OwnerScoreSessionError>>()?;
        geometry.extend(members);
        for (canonical, value) in geometry {
            self.publish_canonical(canonical, value)?;
            if let Some(point) = value.position {
                match self.owner.owner_node_point(canonical) {
                    Ok(retained) if retained.get().is_some() => {
                        self.owner.write_owner_point(&retained, point)?;
                    }
                    _ => self.owner.set_position(canonical, point),
                }
            } else {
                self.owner.clear_position(canonical);
            }
            self.owner.nodes[canonical.0 as usize].rect = value.rect;
        }
        Ok(())
    }

    /// optimization_guard.go814–939. The current Graph receiver supplies the
    /// member roots, shared maps and placement cost state. It is not the
    /// candidate root.Node.Graph and does not enumerate unrelated map keys.
    /// Failed capture retains an inert scratch prefix until release; Context
    /// mutations are not rolled back before the caller's occupied copy arms.
    pub(super) fn capture_optimizer_mutation_state_into(
        &mut self,
        guard: &mut OptimizationWorkGuard<'_>,
        snapshot: &mut OwnerOptimizerMutationSnapshot,
    ) -> Result<(), OptimizationError> {
        snapshot.capture_into(self.owner, self.current_view, guard)
    }
    pub(super) fn canonical_node(&self, local: NodeId) -> Result<NodeId, OwnerScoreSessionError> {
        self.mapping.canonical_node(local)
    }
    pub(super) fn canonical_edge(&self, local: EdgeId) -> Result<EdgeId, OwnerScoreSessionError> {
        self.mapping.canonical_edge(local)
    }
    pub(super) fn abductions(&self) -> Option<&OwnerAbductionSlice> {
        self.abductions
    }
    pub(super) fn compute_current_cell_size(&mut self) -> Result<f64, OwnerScoreSessionError> {
        Ok(self
            .owner
            .compute_owner_graph_cell_size(self.current_view)?)
    }
    pub(super) fn initialize_current_turn_cost(&self) -> Result<f64, OwnerScoreSessionError> {
        Ok(self.owner.owner_graph_turn_cost(self.current_view)?)
    }

    /// The numerical workspace and canonical view represent one Go Graph,
    /// including its lazy cost fields (graph.go532–549). Keep the first reader
    /// and transaction restores on that same cache allocation.
    pub(super) fn bind_current_workspace_costs(
        &self,
        workspace: &mut ArenaGraph,
    ) -> Result<(), OwnerScoreSessionError> {
        workspace.cost_cache = self
            .owner
            .owner_graph_cost_cache(self.current_view)?
            .clone();
        Ok(())
    }

    /// node_placement.go246–262 initializes only Graph.Nodes, then positions
    /// nested children through those same pointers. Dense storage is a work
    /// buffer: its explicit member mapping supplies the new TopLeft values,
    /// while retained descendants and their boxes come from the live owner.
    /// No parent origin or projected external child supplies an owner value.
    pub(super) fn initialize_current_member_positions(
        &mut self,
        workspace: &ArenaGraph,
    ) -> Result<(), OwnerScoreSessionError> {
        self.assign_current_member_positions(workspace)?;
        self.sync_current_nested_geometry()
    }

    pub(super) fn assign_current_member_positions(
        &mut self,
        workspace: &ArenaGraph,
    ) -> Result<(), OwnerScoreSessionError> {
        self.publish_current_member_positions(workspace, true)
    }

    pub(super) fn update_current_member_positions(
        &mut self,
        workspace: &ArenaGraph,
    ) -> Result<(), OwnerScoreSessionError> {
        // compaction.go moves each root with its descendants; it does not
        // call positionContainerChildren/SyncNestedGeometry again. Preserve
        // the current offsets, including a child moved by an optimizer trial.
        for local in workspace.graph_node_order() {
            let canonical = self.canonical_node(local)?;
            let target = workspace
                .position(local)
                .ok_or(OwnerScoreSessionError::UnpositionedNode(canonical))?;
            let current = self
                .owner
                .position(canonical)
                .ok_or(OwnerScoreSessionError::UnpositionedNode(canonical))?;
            if current == target {
                continue;
            }
            let delta = Point {
                x: target.x - current.x,
                y: target.y - current.y,
            };
            for node in self.live_movement_nodes(canonical)? {
                self.owner
                    .position(node)
                    .ok_or(OwnerScoreSessionError::UnpositionedNode(node))?;
                self.owner.translate_owner_point(node, delta.x, delta.y);
            }
        }
        Ok(())
    }

    fn publish_current_member_positions(
        &mut self,
        workspace: &ArenaGraph,
        replace: bool,
    ) -> Result<(), OwnerScoreSessionError> {
        let members = workspace.graph_node_order();
        let mapped = members
            .iter()
            .map(|&local| self.canonical_node(local))
            .collect::<Result<Vec<_>, _>>()?;
        let view = self
            .owner
            .owner_graph_view(self.current_view)
            .ok_or(OwnerTopologyError::UnknownGraph(self.current_view))?;
        if !mapped.iter().copied().map(Some).eq(view.nodes.iter_slots()) {
            return Err(OwnerScoreSessionError::MemberMappingMismatch);
        }
        let positions = members
            .iter()
            .zip(&mapped)
            .map(|(&local, &canonical)| {
                workspace
                    .position(local)
                    .map(|point| (canonical, point))
                    .ok_or(OwnerScoreSessionError::UnpositionedNode(canonical))
            })
            .collect::<Result<Vec<_>, _>>()?;
        // Initialization itself has finished in the workspace. Publish only
        // its member assignments. The Go enclosing ownership defer does not
        // undo these coordinates after a later placement error or panic.
        for (canonical, point) in positions {
            if !replace {
                if let Ok(header) = self.owner.owner_node_point(canonical) {
                    self.owner.write_owner_point(&header, point)?;
                }
            } else {
                self.owner.set_position(canonical, point);
            }
            let node = &mut self.owner.nodes[canonical.0 as usize];
            node.position = Some(point);
            node.rect.origin = point;
        }
        Ok(())
    }

    /// node_placement.go206 calls direct on shared node pointers after the
    /// component optimizer. Mirror/refit writes must reach those same points
    /// before a later component refreshes its retained owner projections.
    pub(super) fn publish_direct_geometry(
        &mut self,
        values: &[(NodeId, OwnerNodeGeometry)],
    ) -> Result<(), OwnerScoreSessionError> {
        let canonical = values
            .iter()
            .map(|&(local, geometry)| Ok((self.canonical_node(local)?, geometry)))
            .collect::<Result<Vec<_>, OwnerScoreSessionError>>()?;
        self.publish_direct_canonical_geometry(&canonical)
    }

    /// trees.Place appends restored nodes after the component was induced.
    /// Their explicit parent-scope handles remain valid even when absent from
    /// the original dense component mapping. Do not drop those shared writes.
    pub(super) fn publish_component_direct_geometry(
        &mut self,
        parent_values: &[(NodeId, OwnerNodeGeometry)],
    ) -> Result<(), OwnerScoreSessionError> {
        let parent = &self
            .components
            .last()
            .ok_or(OwnerScoreSessionError::InvalidComponentToken)?
            .previous_mapping;
        let canonical = parent_values
            .iter()
            .map(|&(local, geometry)| Ok((parent.canonical_node(local)?, geometry)))
            .collect::<Result<Vec<_>, OwnerScoreSessionError>>()?;
        self.publish_direct_canonical_geometry(&canonical)
    }

    /// direct/mirrorAxes mutates shared container and aggregate descendants
    /// omitted from the component Nodes slice. Publish their actual callback
    /// deltas before the next component refreshes retained owner projections.
    pub(super) fn publish_projected_direct_geometry<'b>(
        &mut self,
        before: impl IntoIterator<Item = &'b ArenaNode>,
        after: impl IntoIterator<Item = &'b ArenaNode>,
    ) -> Result<(), OwnerScoreSessionError> {
        let previous = before
            .into_iter()
            .filter_map(|node| {
                node.owner_geometry_source
                    .map(|canonical| (canonical, OwnerNodeGeometry::from_node(node)))
            })
            .collect::<BTreeMap<_, _>>();
        let mut updates = BTreeMap::new();
        for node in after {
            let Some(canonical) = node.owner_geometry_source else {
                continue;
            };
            let geometry = OwnerNodeGeometry::from_node(node);
            if previous.get(&canonical) == Some(&geometry) {
                continue;
            }
            if let Some(prior) = updates.insert(canonical, geometry)
                && prior != geometry
            {
                return Err(OwnerScoreSessionError::MemberMappingMismatch);
            }
        }
        self.publish_direct_canonical_geometry(&updates.into_iter().collect::<Vec<_>>())
    }

    fn publish_direct_canonical_geometry(
        &mut self,
        values: &[(NodeId, OwnerNodeGeometry)],
    ) -> Result<(), OwnerScoreSessionError> {
        let mut updates = Vec::with_capacity(values.len());
        for &(canonical, geometry) in values {
            let point = geometry
                .position
                .ok_or(OwnerScoreSessionError::UnpositionedNode(canonical))?;
            updates.push((canonical, point, geometry));
        }
        for (canonical, point, geometry) in updates {
            if self
                .owner
                .owner_bound_point_value(canonical)
                .is_some_and(|value| value.is_some())
            {
                let header = self.owner.owner_node_point(canonical)?;
                self.owner.write_owner_point(&header, point)?;
            } else {
                // Singleton/tree placement may be the first assignment at
                // this boundary. Preserve an existing point; allocate only
                // when the source TopLeft has not yet been positioned.
                self.owner.set_position(canonical, point);
            }
            self.publish_canonical(canonical, geometry)?;
        }
        Ok(())
    }

    pub(super) fn sync_current_nested_geometry(&mut self) -> Result<(), OwnerScoreSessionError> {
        let members = self
            .owner
            .owner_graph_view(self.current_view)
            .ok_or(OwnerTopologyError::UnknownGraph(self.current_view))?
            .nodes
            .clone();
        for canonical in members.require_nodes() {
            if self.owner.nodes[canonical.0 as usize].is_container {
                self.position_live_container_children(canonical, true)?;
            }
            if self.owner.owner_is_cluster_vessel(canonical) {
                self.sync_live_cluster(canonical)?;
            }
            if let Some(Some(group)) = self.owner.owner_graph_group_entry(
                self.current_view,
                super::owner_groups::OwnerGroupKind::Sequence,
                Some(canonical),
            )? {
                self.sync_live_sequence(group)?;
            }
        }
        Ok(())
    }

    /// graph.go918–945 and cluster.go: resizing and arrangement read the
    /// actual group header and policy, and move the retained original members.
    fn sync_live_cluster(&mut self, vessel: NodeId) -> Result<(), OwnerScoreSessionError> {
        let group = self
            .owner
            .owner_graph_group_entry(
                self.current_view,
                super::owner_groups::OwnerGroupKind::Cluster,
                Some(vessel),
            )?
            .flatten()
            .ok_or(OwnerScoreSessionError::UnsupportedNestedVessel(vessel))?;
        let fields = self.owner.owner_group_fields(group)?;
        let nodes = (0..fields.nodes.len())
            .map(|i| {
                fields
                    .nodes
                    .get(i)
                    .expect("logical group member")
                    .expect("nil group member")
            })
            .collect::<Vec<_>>();
        let policy = self.owner.owner_cluster_policy(group)?;
        let maximum = nodes.iter().fold(Size::default(), |maximum, &node| {
            let size = self.owner.nodes[node.0 as usize].rect.size;
            Size {
                width: maximum.width.max(size.width),
                height: maximum.height.max(size.height),
            }
        });
        if !policy.fixed_size {
            for &node in &nodes {
                self.owner.nodes[node.0 as usize].rect.size = maximum;
            }
        }
        let count = nodes.len() as f64;
        let vessel_size = match policy.arrangement.as_str() {
            "Row" => Size {
                width: maximum.width * count + policy.padding * (count - 1.0),
                height: maximum.height,
            },
            "Column" => Size {
                width: maximum.width,
                height: maximum.height * count + policy.padding * (count - 1.0),
            },
            _ => self.owner.nodes[vessel.0 as usize].rect.size,
        };
        self.owner.nodes[vessel.0 as usize].rect.size = vessel_size;
        let Some(point) = self.owner.position(vessel) else {
            return Ok(());
        };
        let mut cursor = if policy.arrangement == "Row" {
            point.x
        } else {
            point.y
        };
        for &node in &nodes {
            let size = self.owner.nodes[node.0 as usize].rect.size;
            let prior = self.owner.position(node);
            let target = match policy.arrangement.as_str() {
                "Row" => Point {
                    x: cursor,
                    y: (point.y + vessel_size.height / 2.0 - size.height / 2.0).round(),
                },
                "Column" => Point {
                    x: (point.x + vessel_size.width / 2.0 - size.width / 2.0).round(),
                    y: cursor,
                },
                _ => continue,
            };
            if let Some(prior) = prior {
                // Source rounds the delta when an existing member moves.
                let mut delta = Point {
                    x: target.x - prior.x,
                    y: target.y - prior.y,
                };
                if policy.arrangement == "Row" {
                    delta.y = (point.y + vessel_size.height / 2.0 - (prior.y + size.height / 2.0))
                        .round();
                } else {
                    delta.x =
                        (point.x + vessel_size.width / 2.0 - (prior.x + size.width / 2.0)).round();
                }
                for moved in self.live_movement_nodes(node)? {
                    self.owner.translate_owner_point(moved, delta.x, delta.y);
                }
            } else {
                self.owner.set_position(node, target);
                if self.owner.nodes[node.0 as usize].is_container {
                    // group_geometry.go positions a newly initialized member
                    // without padding; syncNested adds its inset below.
                    self.position_live_container_children(node, false)?;
                }
            }
            cursor += if policy.arrangement == "Row" {
                size.width
            } else {
                size.height
            } + policy.padding;
        }
        // syncNested adds container padding after arranging cluster members.
        for node in nodes {
            if self.owner.nodes[node.0 as usize].is_container {
                let padding = self.owner.shape_fit_padding_with_children(node, true);
                for child in self
                    .live_container_children(self.current_view, node)?
                    .into_iter()
                    .flatten()
                {
                    for moved in self.live_movement_nodes(child)? {
                        self.owner
                            .translate_owner_point(moved, padding.left, padding.top);
                    }
                }
            }
        }
        Ok(())
    }

    fn source_sequence_advance(width: f64) -> f64 {
        if width <= 0.0 {
            0.0
        } else if width <= 35.0 {
            width - width / 2.0
        } else {
            width - 35.0
        }
    }

    /// sequence.go SyncGeometry: resize first, then replace each step TopLeft.
    fn sync_live_sequence(
        &mut self,
        group: super::owner_groups::OwnerGroupId,
    ) -> Result<(), OwnerScoreSessionError> {
        let fields = self.owner.owner_group_fields(group)?;
        let vessel = fields.vessel.expect("sequence vessel");
        let nodes = (0..fields.nodes.len())
            .map(|i| {
                fields
                    .nodes
                    .get(i)
                    .expect("logical group member")
                    .expect("nil group member")
            })
            .collect::<Vec<_>>();
        let mut size = Size::default();
        let mut offset = 0.0_f64;
        for &node in &nodes {
            let child = self.owner.nodes[node.0 as usize].rect.size;
            size.width = size.width.max(offset + child.width.max(0.0));
            offset += Self::source_sequence_advance(child.width);
            size.height = size.height.max(child.height.max(0.0));
        }
        self.owner.nodes[vessel.0 as usize].rect.size = size;
        if let Some(mut point) = self.owner.position(vessel) {
            for node in nodes {
                self.owner.set_position(node, point);
                point.x += Self::source_sequence_advance(
                    self.owner.nodes[node.0 as usize].rect.size.width,
                );
            }
        }
        Ok(())
    }

    fn live_container_children(
        &self,
        receiver: OwnerGraphId,
        parent: NodeId,
    ) -> Result<Vec<Option<NodeId>>, OwnerScoreSessionError> {
        let map = self
            .owner
            .owner_container_map(receiver)
            .ok_or(OwnerTopologyError::UnknownGraph(receiver))?;
        let Some(header) = map.get(Some(parent)) else {
            return Ok(Vec::new());
        };
        Ok((0..header.len())
            .map(|index| header.get(index).expect("live logical header slot"))
            .collect())
    }

    /// optimization_guard.go602–679/962–1008. The optimizer's walk has a
    /// different contract from the general graph walk: nil slots reject and
    /// nodes are deduplicated when pushed, so an already-pending sibling is
    /// not emitted early through a cross-link. Root.Graph supplies every map.
    pub(super) fn optimizer_candidate_movement<'session>(
        &'session mut self,
        local: NodeId,
        guard: &mut OptimizationWorkGuard<'_>,
    ) -> Result<OwnerCandidateMovement<'session, 'a>, OptimizationError> {
        let root = self
            .canonical_node(local)
            .map_err(|error| OptimizationError::Invalid(error.to_string()))?;
        let receiver = self.owner.topology_node_owner(root).ok_or_else(|| {
            OptimizationError::Invalid(format!(
                "TALA {} movement requires a node with a graph",
                guard.location()
            ))
        })?;
        let before = guard.used();
        let mut nodes = vec![root];
        let mut seen = BTreeSet::from([root]);
        let mut stack = Vec::new();
        // captureOptimizerNodePositions' single-root leaf shortcut uses the
        // private flag and selected Graph.Sequences entry, not a global vessel
        // registry or Node.Sequence membership pointer.
        let live = self.owner.topology_node(root).expect("validated root");
        let leaf = !live.is_container
            && !self.owner.owner_is_cluster_vessel(root)
            && self
                .owner
                .owner_graph_group_entry(
                    receiver,
                    super::owner_groups::OwnerGroupKind::Sequence,
                    Some(root),
                )
                .map_err(|e| OptimizationError::Invalid(e.to_string()))?
                .flatten()
                .is_none();
        if !leaf {
            self.owner
                .owner_optimizer_push_children(root, receiver, guard, &mut seen, &mut stack)?;
        }
        while let Some(node) = stack.pop() {
            step_candidate_owner(self.owner, guard)?;
            nodes.push(node);
            self.owner
                .owner_optimizer_push_children(node, receiver, guard, &mut seen, &mut stack)?;
        }
        let descendant_work = guard.used() - before;
        let mut positions = Vec::with_capacity(nodes.len());
        let mut point_snapshots = Vec::with_capacity(nodes.len());
        for node in nodes {
            step_candidate_owner(self.owner, guard)?;
            let live = self.owner.topology_node(node).ok_or_else(|| {
                OptimizationError::Invalid(OwnerTopologyError::UnknownNode(node).to_string())
            })?;
            positions.push((node, self.owner.position(node), live.rect.origin));
            point_snapshots.push(match self.owner.owner_node_point(node) {
                Ok(point) => Some(point.snapshot()),
                Err(OwnerTopologyError::UnboundNodePoint(_)) => None,
                Err(error) => return Err(OptimizationError::Invalid(error.to_string())),
            });
        }
        Ok(OwnerCandidateMovement {
            session: self,
            positions,
            point_snapshots,
            descendant_work,
            complete: false,
        })
    }

    /// Source positional snapshot breadth-first traversal. A root contributes
    /// its own Graph receiver; all descendants retain that receiver, including
    /// when their Node.Graph has since changed. Nil slots are skipped and real
    /// node identity is deduplicated across roots. Compaction's context/work
    /// guard remains a separate outstanding port of the enclosing caller.
    /// compaction.go entry snapshot: retain the selected Graph.Nodes header
    /// before the snapshot guard polls, then arm point/routing rollback only
    /// after the complete guarded traversal returns.
    pub(super) fn compaction_entry<'session>(
        &'session mut self,
        context: &dyn super::limits::Context,
    ) -> Result<OwnerCompactionPositions<'session, 'a>, super::limits::Error> {
        let state = super::compaction_checkpoint::CapturedCompactionState::capture(
            self.owner,
            self.current_view,
            context,
        )?;
        Ok(OwnerCompactionPositions {
            session: self,
            positions: Vec::new(),
            entry: Some(state),
            selected: None,
            complete: false,
        })
    }

    pub(super) fn compaction_positions_with_context<'session>(
        &'session mut self,
        roots: &[NodeId],
        context: &dyn super::limits::Context,
    ) -> Result<OwnerCompactionPositions<'session, 'a>, super::limits::Error> {
        let roots = roots
            .iter()
            .map(|&local| self.canonical_node(local))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| super::limits::Error::Invalid(e.to_string()))?;
        let slots = roots.iter().copied().map(Some).collect::<Vec<_>>();
        let roots = self
            .owner
            .allocate_owner_member_header(&slots, slots.len())
            .map_err(|e| super::limits::Error::Invalid(e.to_string()))?;
        let selected = super::compaction_checkpoint::CapturedCompactionPositions::capture(
            self.owner, roots, context,
        )?;
        Ok(OwnerCompactionPositions {
            session: self,
            positions: Vec::new(),
            entry: None,
            selected: Some(selected),
            complete: false,
        })
    }

    pub(super) fn compaction_positions<'session>(
        &'session mut self,
        roots: &[NodeId],
    ) -> Result<OwnerCompactionPositions<'session, 'a>, OwnerScoreSessionError> {
        let mut queue = roots
            .iter()
            .map(|&local| {
                let node = self.canonical_node(local)?;
                Ok((Some(node), self.owner.topology_node_owner(node)))
            })
            .collect::<Result<Vec<_>, OwnerScoreSessionError>>()?;
        let mut seen = BTreeSet::new();
        let mut positions = Vec::new();
        let mut index = 0;
        while index < queue.len() {
            let (node, receiver) = queue[index];
            index += 1;
            let Some(node) = node else { continue };
            if !seen.insert(node) {
                continue;
            }
            let live = self
                .owner
                .topology_node(node)
                .ok_or(OwnerTopologyError::UnknownNode(node))?;
            let point = match self.owner.owner_node_point(node) {
                Ok(point) => Some(point.snapshot()),
                Err(OwnerTopologyError::UnboundNodePoint(_)) => None,
                Err(error) => return Err(error.into()),
            };
            positions.push((node, self.owner.position(node), live.rect.origin, point));
            let Some(receiver) = receiver else { continue };
            if live.is_container {
                let map = self
                    .owner
                    .owner_container_map(receiver)
                    .ok_or(OwnerTopologyError::UnknownGraph(receiver))?;
                if let Some(children) = map.get(Some(node)) {
                    queue.extend((0..children.len()).map(|index| {
                        (
                            children.get(index).expect("retained container slot"),
                            Some(receiver),
                        )
                    }));
                }
            }
            for kind in [OwnerGroupKind::Cluster, OwnerGroupKind::Sequence] {
                if kind == OwnerGroupKind::Cluster && !self.owner.owner_is_cluster_vessel(node) {
                    continue;
                }
                if let Some(group) = self
                    .owner
                    .owner_graph_group_entry(receiver, kind, Some(node))?
                    .flatten()
                {
                    let children = self.owner.owner_group_fields(group)?.nodes;
                    queue.extend(children.iter_slots().map(|child| (child, Some(receiver))));
                }
            }
        }
        Ok(OwnerCompactionPositions {
            session: self,
            positions,
            entry: None,
            selected: None,
            complete: false,
        })
    }

    /// graph.go955–1031: one Graph receiver supplies the full descendant
    /// preorder. Deduplicate actual nodes, skip nil descendant slots, and
    /// preserve container-before-cluster-before-sequence traversal.
    fn live_movement_nodes(&self, root: NodeId) -> Result<Vec<NodeId>, OwnerScoreSessionError> {
        let receiver = self
            .owner
            .topology_node_owner(root)
            .ok_or(OwnerScoreError::MissingNodeOwner(root))?;
        let mut ordered = vec![root];
        ordered.extend(self.live_descendants(receiver, Some(root), true)?);
        Ok(ordered)
    }

    /// node.go moveNodeAbsWithChildren/moveNodeWithChildren: translate the
    /// original point first, then expand the root's actual Graph receiver.
    /// A zero move must not inspect Graph or descendant positions.
    pub(super) fn move_canonical_abs_with_children(
        &mut self,
        root: NodeId,
        target: Point,
    ) -> Result<(), OwnerScoreSessionError> {
        let current = self
            .owner
            .position(root)
            .ok_or(OwnerScoreSessionError::UnpositionedNode(root))?;
        if current == target {
            return Ok(());
        }
        let delta = Point {
            x: target.x - current.x,
            y: target.y - current.y,
        };
        if delta.x == 0.0 && delta.y == 0.0 {
            return Ok(());
        }
        self.owner.translate_owner_point(root, delta.x, delta.y);
        let receiver = self
            .owner
            .topology_node_owner(root)
            .ok_or(OwnerScoreError::MissingNodeOwner(root))?;
        for child in self.live_descendants(receiver, Some(root), true)? {
            self.owner.translate_owner_point(child, delta.x, delta.y);
        }
        Ok(())
    }

    /// graph.go955–1031, via its unbounded AllDescendantNodes wrapper. Unlike
    /// movement, CombineSubgraphs supplies masterGraph explicitly: descendants'
    /// Node.Graph pointers and legacy group member vectors are not the receiver.
    pub(super) fn live_descendants(
        &self,
        receiver: OwnerGraphId,
        root: Option<NodeId>,
        include_group_members: bool,
    ) -> Result<Vec<NodeId>, OwnerScoreSessionError> {
        let mut seen = BTreeSet::new();
        if let Some(root) = root {
            self.owner
                .topology_node(root)
                .ok_or(OwnerTopologyError::UnknownNode(root))?;
            seen.insert(root);
        }
        let mut stack = Vec::new();
        self.push_live_descendant_children(receiver, root, include_group_members, &mut stack)?;
        let mut ordered = Vec::new();
        while let Some((node, emit)) = stack.pop() {
            let Some(node) = node else { continue };
            if !seen.insert(node) {
                continue;
            }
            self.owner
                .topology_node(node)
                .ok_or(OwnerTopologyError::UnknownNode(node))?;
            if emit {
                ordered.push(node);
            }
            self.push_live_descendant_children(
                receiver,
                Some(node),
                include_group_members,
                &mut stack,
            )?;
        }
        Ok(ordered)
    }

    /// Insert `root` and all of its live descendants, group members
    /// included, into `seen`. Every node in `seen` was inserted by this walk
    /// together with its whole closure, so a node reached again stops there;
    /// the set equals inserting each root and extending with its
    /// `live_descendants`, and every check that can fail is reached first.
    fn extend_live_descendant_set(
        &self,
        receiver: OwnerGraphId,
        root: NodeId,
        seen: &mut DenseNodeSet,
        stack: &mut Vec<(Option<NodeId>, bool)>,
    ) -> Result<(), OwnerScoreSessionError> {
        self.owner
            .topology_node(root)
            .ok_or(OwnerTopologyError::UnknownNode(root))?;
        if !seen.insert(root) {
            return Ok(());
        }
        stack.clear();
        self.push_live_descendant_children(receiver, Some(root), true, stack)?;
        while let Some((node, _)) = stack.pop() {
            let Some(node) = node else { continue };
            if !seen.insert(node) {
                continue;
            }
            self.owner
                .topology_node(node)
                .ok_or(OwnerTopologyError::UnknownNode(node))?;
            self.push_live_descendant_children(receiver, Some(node), true, stack)?;
        }
        Ok(())
    }

    fn push_live_descendant_children(
        &self,
        receiver: OwnerGraphId,
        parent: Option<NodeId>,
        include_group_members: bool,
        stack: &mut Vec<(Option<NodeId>, bool)>,
    ) -> Result<(), OwnerScoreSessionError> {
        for kind in [OwnerGroupKind::Sequence, OwnerGroupKind::Cluster] {
            if kind == OwnerGroupKind::Cluster
                && !parent.is_some_and(|node| self.owner.owner_is_cluster_vessel(node))
            {
                continue;
            }
            if let Some(group) = self
                .owner
                .owner_graph_group_entry(receiver, kind, parent)?
                .flatten()
            {
                let header = self.owner.owner_group_fields(group)?.nodes;
                for index in (0..header.len()).rev() {
                    stack.push((
                        header.get(index).expect("retained backward group slot"),
                        include_group_members,
                    ));
                }
            }
        }
        if parent.is_none_or(|node| self.owner.nodes[node.0 as usize].is_container) {
            let map = self
                .owner
                .owner_container_map(receiver)
                .ok_or(OwnerTopologyError::UnknownGraph(receiver))?;
            if let Some(header) = map.get(parent) {
                for index in (0..header.len()).rev() {
                    stack.push((
                        header.get(index).expect("retained backward container slot"),
                        true,
                    ));
                }
            }
        }
        Ok(())
    }

    /// node.go2151–2168/2182–2192 reads the actual child header and current boxes, then
    /// translates each child's own retained subtree. Bounds helpers consume
    /// immediate owner reads; no projected child or ID lookup is used.
    fn position_live_container_children(
        &mut self,
        container: NodeId,
        with_padding: bool,
    ) -> Result<(), OwnerScoreSessionError> {
        let receiver = self
            .owner
            .topology_node_owner(container)
            .ok_or(OwnerScoreError::MissingNodeOwner(container))?;
        let children = self
            .live_container_children(receiver, container)?
            .into_iter()
            .map(|child| {
                child.ok_or_else(|| {
                    OwnerTopologyError::ContainerSlice(
                        "nil direct child in nested placement".into(),
                    )
                    .into()
                })
            })
            .collect::<Result<Vec<_>, OwnerScoreSessionError>>()?;
        if children.is_empty() {
            return Ok(());
        }
        let boxes = children
            .iter()
            .map(|&child| {
                let node = self
                    .owner
                    .topology_node(child)
                    .ok_or(OwnerTopologyError::UnknownNode(child))?;
                let mut value = node.clone();
                value.position = Some(
                    self.owner
                        .position(child)
                        .ok_or(OwnerScoreSessionError::UnpositionedNode(child))?,
                );
                Ok(value)
            })
            .collect::<Result<Vec<_>, OwnerScoreSessionError>>()?;
        let projected = children
            .iter()
            .copied()
            .zip(boxes.iter())
            .collect::<Vec<_>>();
        let (top_left, bottom_right) = self.owner.fixed_projected_node_corners(&projected);
        let (mut top_left, mut bottom_right) = top_left
            .zip(bottom_right)
            .expect("nonempty positioned live child header");
        for child in &boxes {
            if let Some(label) = child.label_size {
                let point = child.position.expect("validated child position");
                if (point.x == top_left.x || point.x + child.rect.size.width == bottom_right.x)
                    && label.width > child.rect.size.width
                {
                    let overhang = child.rect.size.width / 2.0 - label.width / 2.0;
                    top_left.x = top_left.x.min((point.x + overhang).floor());
                    bottom_right.x = bottom_right
                        .x
                        .max((point.x + child.rect.size.width - overhang).ceil());
                }
            }
        }
        let padding = if with_padding {
            self.owner.shape_fit_padding(container)
        } else {
            Insets::uniform(0.0)
        };
        let inside = self
            .owner
            .shape_inside_placement_absolute(
                container,
                Size {
                    width: bottom_right.x - top_left.x,
                    height: bottom_right.y - top_left.y,
                },
                padding,
            )
            .ok_or(OwnerScoreSessionError::UnpositionedNode(container))?;
        let delta = Point {
            x: inside.x - top_left.x,
            y: inside.y - top_left.y,
        };
        if delta.x == 0.0 && delta.y == 0.0 {
            return Ok(());
        }
        for child in children {
            for node in self.live_movement_nodes(child)? {
                self.owner
                    .position(node)
                    .ok_or(OwnerScoreSessionError::UnpositionedNode(node))?;
                self.owner.translate_owner_point(node, delta.x, delta.y);
            }
        }
        Ok(())
    }

    /// An optimizer may own this shorter session by value. Its independent
    /// journals restore its pending work before the parent resumes. Only map
    /// correspondence is cloned; owner nodes, edges, and headers are borrowed.
    pub(super) fn reborrow(&mut self) -> OwnerScoreSession<'_> {
        OwnerScoreSession {
            owner: &mut *self.owner,
            current_view: self.current_view,
            mapping: self.mapping.clone(),
            abductions: self.abductions,
            session: NEXT_OWNER_SCORE_SESSION.fetch_add(1, Ordering::Relaxed),
            next_serial: 0,
            components: Vec::new(),
            pending_prepared: BTreeSet::new(),
            ownership: Vec::new(),
            parent_ownership: Some(&mut self.ownership),
            ownership_committed: false,
            trials: Vec::new(),
        }
    }

    pub(super) fn with_frame<R>(
        &self,
        score: impl for<'frame> FnOnce(&OwnerScoreFrame<'frame>) -> R,
    ) -> Result<R, OwnerScoreSessionError> {
        self.with_frame_abductions(self.abductions, score)
    }

    /// sizeless_optimizer.go404–407 passes EdgeAbductions:nil. The enclosing
    /// placement session can still retain its prepared slice for sized scoring.
    pub(super) fn with_frame_without_abductions<R>(
        &self,
        score: impl for<'frame> FnOnce(&OwnerScoreFrame<'frame>) -> R,
    ) -> Result<R, OwnerScoreSessionError> {
        self.with_frame_abductions(None, score)
    }

    pub(super) fn score_sizeless_owner(
        &mut self,
        scorer: &mut super::scoring::OwnerSizelessEdgeLengthScorer,
        workspace: &ArenaGraph,
        context: super::scoring_context::ScoreContext<'_>,
    ) -> Result<f64, limits::Error> {
        scorer.score_with_owner(
            workspace,
            self.owner,
            &self.mapping,
            self.current_view,
            context,
        )
    }

    fn with_frame_abductions<R>(
        &self,
        abductions: Option<&OwnerAbductionSlice>,
        score: impl for<'frame> FnOnce(&OwnerScoreFrame<'frame>) -> R,
    ) -> Result<R, OwnerScoreSessionError> {
        let mut frame = OwnerScoreFrame::new_with_edge_mapping(
            self.owner,
            self.mapping.arena,
            self.current_view,
            &self.mapping.nodes,
            &self.mapping.edges,
        )?;
        if let Some(slice) = abductions {
            frame = frame.with_abductions(slice)?;
        }
        Ok(score(&frame))
    }

    pub(super) fn enter_component(
        &mut self,
        parent_nodes: &[NodeId],
        parent_edges: &[EdgeId],
        member_local_roots: &[NodeId],
    ) -> Result<OwnerComponentToken, OwnerScoreSessionError> {
        let prepared = self.prepare_component(parent_nodes, parent_edges, member_local_roots)?;
        self.enter_prepared_component(prepared, parent_nodes, parent_edges, member_local_roots)
    }

    pub(super) fn prepare_component(
        &mut self,
        parent_nodes: &[NodeId],
        parent_edges: &[EdgeId],
        member_local_roots: &[NodeId],
    ) -> Result<PreparedOwnerComponent, OwnerScoreSessionError> {
        if !self.trials.is_empty() {
            return Err(OwnerScoreSessionError::PendingTrial);
        }
        let mapping = self.mapping.compose(parent_nodes, parent_edges)?;
        let roots = member_local_roots
            .iter()
            .map(|&node| mapping.canonical_node(node))
            .collect::<Result<Vec<_>, _>>()?;
        let view = self
            .owner
            .create_owner_graph_view(roots.clone(), mapping.edges.clone())?;
        self.owner
            .share_owner_proximity_maps(self.current_view, view)?;
        for kind in [
            super::owner_groups::OwnerGroupKind::Cluster,
            super::owner_groups::OwnerGroupKind::Sequence,
        ] {
            self.owner
                .copy_owner_graph_group_map_from(view, self.current_view, kind)?;
        }
        self.owner
            .copy_owner_graph_directions_from(view, self.current_view)?;
        self.owner
            .copy_owner_graph_tree_roots_from(view, self.current_view)?;
        self.owner
            .copy_owner_graph_node_to_tree_from(view, self.current_view)?;
        self.owner
            .set_owner_view_node_mapping(view, mapping.nodes.clone())?;
        let mut journal = OwnerGraphOwnershipJournal::default();
        self.owner
            .set_owner_graph_reference(view, &roots, false, &mut journal)?;
        self.next_serial += 1;
        let token = PreparedOwnerComponent {
            session: self.session,
            serial: self.next_serial,
            parent_view: self.current_view,
            mapping,
            roots,
            view,
            root_journal: self.ownership.len(),
        };
        self.ownership.push(Some(journal));
        self.pending_prepared.insert(token.serial);
        Ok(token)
    }

    pub(super) fn enter_prepared_component(
        &mut self,
        prepared: PreparedOwnerComponent,
        parent_nodes: &[NodeId],
        parent_edges: &[EdgeId],
        member_local_roots: &[NodeId],
    ) -> Result<OwnerComponentToken, OwnerScoreSessionError> {
        self.enter_prepared_component_inner(
            prepared,
            parent_nodes,
            parent_edges,
            member_local_roots,
            None,
        )
    }
    pub(super) fn enter_prepared_component_with_dimensions(
        &mut self,
        prepared: PreparedOwnerComponent,
        parent_nodes: &[NodeId],
        parent_edges: &[EdgeId],
        member_local_roots: &[NodeId],
        workspace: &ArenaGraph,
    ) -> Result<OwnerComponentToken, OwnerScoreSessionError> {
        self.enter_prepared_component_inner(
            prepared,
            parent_nodes,
            parent_edges,
            member_local_roots,
            Some(workspace),
        )
    }
    fn enter_prepared_component_inner(
        &mut self,
        prepared: PreparedOwnerComponent,
        parent_nodes: &[NodeId],
        parent_edges: &[EdgeId],
        member_local_roots: &[NodeId],
        workspace: Option<&ArenaGraph>,
    ) -> Result<OwnerComponentToken, OwnerScoreSessionError> {
        if prepared.session != self.session
            || prepared.parent_view != self.current_view
            || !self.pending_prepared.contains(&prepared.serial)
        {
            return Err(OwnerScoreSessionError::InvalidPreparedComponentToken);
        }
        if !self.trials.is_empty() {
            return Err(OwnerScoreSessionError::PendingTrial);
        }
        let mapping = self.mapping.compose(parent_nodes, parent_edges)?;
        let roots = member_local_roots
            .iter()
            .map(|&node| mapping.canonical_node(node))
            .collect::<Result<Vec<_>, _>>()?;
        if mapping != prepared.mapping || roots != prepared.roots {
            return Err(OwnerScoreSessionError::PreparedMappingMismatch);
        }
        // All-map lineage checks precede mutation. Source ComputeCellSize is
        // then first, before selected descendant Graph references are reset.
        self.owner.topology_retained_closure(&roots)?;
        // Source node_placement.go162 computes on the same freshly fitted
        // vessel pointers as the component. Publish adapter dimensions before
        // deriving CellSize, rather than retaining an older owner vessel box.
        let dimensions = workspace
            .map(|workspace| {
                workspace
                    .graph_node_order()
                    .into_iter()
                    .map(|local| {
                        Ok((
                            mapping.canonical_node(local)?,
                            workspace.active_node_size(local),
                        ))
                    })
                    .collect::<Result<Vec<_>, OwnerScoreSessionError>>()
            })
            .transpose()?;
        if let Some(dimensions) = dimensions {
            for (node, size) in dimensions {
                self.owner.nodes[node.0 as usize].rect.size = size;
            }
        }
        self.owner.compute_owner_graph_cell_size(prepared.view)?;
        let mut journal = OwnerGraphOwnershipJournal::default();
        self.owner
            .set_owner_graph_reference(prepared.view, &roots, true, &mut journal)?;
        let token = OwnerComponentToken {
            session: self.session,
            serial: prepared.serial,
        };
        self.pending_prepared.remove(&prepared.serial);
        let journal_index = self.ownership.len();
        self.ownership.push(Some(journal));
        let previous_mapping = std::mem::replace(&mut self.mapping, mapping);
        self.components.push(ComponentEntry {
            token,
            previous_view: self.current_view,
            previous_mapping,
            journal: journal_index,
            root_journal: prepared.root_journal,
        });
        self.current_view = prepared.view;
        Ok(token)
    }

    pub(super) fn leave_component(
        &mut self,
        token: OwnerComponentToken,
        commit_ownership: bool,
    ) -> Result<(), OwnerScoreSessionError> {
        if self.components.last().map(|entry| entry.token) != Some(token) {
            return Err(OwnerScoreSessionError::InvalidComponentToken);
        }
        if !self.trials.is_empty() {
            return Err(OwnerScoreSessionError::PendingTrial);
        }
        let entry = self.components.pop().expect("checked component stack");
        if !commit_ownership {
            // Include any successful descendants of this component. They
            // were created later and must unwind before its original owner.
            for journal in self.ownership[entry.journal..].iter_mut().rev() {
                if let Some(journal) = journal.take() {
                    self.owner.restore_owner_graph_ownership(&journal);
                }
            }
            if let Some(journal) = self.ownership[entry.root_journal].take() {
                self.owner.restore_owner_graph_ownership(&journal);
            }
        }
        self.current_view = entry.previous_view;
        self.mapping = entry.previous_mapping;
        Ok(())
    }

    /// Source splitCommitted=true. Successful component Graph pointers remain
    /// until the enclosing production restore; failure/drop before this call
    /// restores every pending component's exact prior owners.
    pub(super) fn commit_scope_ownership(&mut self) -> Result<(), OwnerScoreSessionError> {
        if !self.components.is_empty() {
            return Err(OwnerScoreSessionError::PendingComponent);
        }
        if !self.pending_prepared.is_empty() {
            return Err(OwnerScoreSessionError::PendingPreparedComponent);
        }
        if !self.trials.is_empty() {
            return Err(OwnerScoreSessionError::PendingTrial);
        }
        self.ownership_committed = true;
        Ok(())
    }

    fn geometry_snapshot(&self) -> Result<GeometryTrial, OwnerScoreSessionError> {
        let view = self
            .owner
            .owner_graph_view(self.current_view)
            .ok_or(OwnerTopologyError::UnknownGraph(self.current_view))?;
        let map = self
            .owner
            .owner_container_map(self.current_view)
            .ok_or(OwnerTopologyError::UnknownGraph(self.current_view))?;
        // Current CopyEntitiesFrom views share the actual owner map. Keys
        // below may come from owner.containers only under this real allocation
        // precondition. A future independently owned receiver map must supply
        // its own key iterator, not silently use the original graph's keys.
        let original_map = self
            .owner
            .owner_container_map(OwnerGraphId(0))
            .ok_or(OwnerTopologyError::UnknownGraph(OwnerGraphId(0)))?;
        if !map.same_map_allocation(&original_map) {
            return Err(OwnerTopologyError::ForeignContainerMap.into());
        }
        let mut seen = BTreeSet::new();
        let mut nodes = Vec::new();
        let mut record = |root: Option<NodeId>| -> Result<(), OwnerScoreSessionError> {
            let mut stack = vec![root];
            while let Some(node) = stack.pop() {
                let Some(node) = node else {
                    continue;
                };
                if !seen.insert(node) {
                    continue;
                }
                let live = self
                    .owner
                    .topology_node(node)
                    .ok_or(OwnerTopologyError::UnknownNode(node))?;
                nodes.push((node, OwnerNodeGeometry::from_node(live)));
                // Capture raw logical slots, including nil, without using the
                // numerical traversal API that intentionally rejects nil.
                if let Some(header) = map.get(Some(node)) {
                    for index in 0..header.len() {
                        stack.push(header.get(index).unwrap());
                    }
                }
            }
            Ok(())
        };
        for node in view.nodes.iter_slots() {
            record(node)?;
        }
        for parent in self.owner.containers.keys() {
            record(parent)?;
            if let Some(header) = map.get(parent) {
                for index in 0..header.len() {
                    record(header.get(index).unwrap())?;
                }
            }
        }
        for (index, cluster) in self.owner.clusters.iter().enumerate() {
            let kind = TopologyVessel::Cluster(index);
            let vessel = self
                .owner
                .topology_vessel_node(kind)
                .ok_or(OwnerTopologyError::UnboundVessel(kind))?;
            record(Some(vessel))?;
            record(
                self.owner
                    .topology_node(vessel)
                    .ok_or(OwnerTopologyError::UnknownNode(vessel))?
                    .container,
            )?;
            for &node in &cluster.members {
                record(Some(node))?;
            }
        }
        for (index, sequence) in self.owner.sequences.iter().enumerate() {
            let kind = TopologyVessel::Sequence(index);
            let vessel = self
                .owner
                .topology_vessel_node(kind)
                .ok_or(OwnerTopologyError::UnboundVessel(kind))?;
            record(Some(vessel))?;
            record(sequence.container)?;
            for &node in &sequence.members {
                record(Some(node))?;
            }
        }
        let edges = view
            .edges
            .read()
            .iter()
            .map(|&edge| {
                let live = self
                    .owner
                    .edge_records
                    .get(edge.0 as usize)
                    .ok_or(OwnerTopologyError::UnknownEdge(edge))?;
                Ok((edge, live.route.read().clone()))
            })
            .collect::<Result<Vec<_>, OwnerScoreSessionError>>()?;
        Ok(GeometryTrial {
            token: OwnerGeometryTrialToken {
                session: self.session,
                serial: self.next_serial + 1,
            },
            nodes,
            edges,
        })
    }

    pub(super) fn begin_trial(
        &mut self,
    ) -> Result<OwnerGeometryTrialToken, OwnerScoreSessionError> {
        let trial = self.geometry_snapshot()?;
        self.next_serial += 1;
        let token = trial.token;
        self.trials.push(trial);
        Ok(token)
    }

    pub(super) fn publish_canonical(
        &mut self,
        canonical: NodeId,
        geometry: OwnerNodeGeometry,
    ) -> Result<(), OwnerScoreSessionError> {
        self.owner
            .topology_node(canonical)
            .ok_or(OwnerTopologyError::UnknownNode(canonical))?;
        if self
            .trials
            .iter()
            .any(|trial| !trial.nodes.iter().any(|&(node, _)| node == canonical))
        {
            return Err(OwnerScoreSessionError::UncapturedNode(canonical));
        }
        let node = &mut self.owner.nodes[canonical.0 as usize];
        node.position = geometry.position;
        node.rect = geometry.rect;
        Ok(())
    }
    pub(super) fn publish_local(
        &mut self,
        local: NodeId,
        geometry: OwnerNodeGeometry,
    ) -> Result<(), OwnerScoreSessionError> {
        self.publish_canonical(self.mapping.canonical_node(local)?, geometry)
    }
    pub(super) fn publish_local_batch(
        &mut self,
        values: &[(NodeId, OwnerNodeGeometry)],
    ) -> Result<(), OwnerScoreSessionError> {
        let canonical = values
            .iter()
            .map(|&(local, geometry)| {
                let node = self.mapping.canonical_node(local)?;
                self.owner
                    .topology_node(node)
                    .ok_or(OwnerTopologyError::UnknownNode(node))?;
                if self
                    .trials
                    .iter()
                    .any(|trial| !trial.nodes.iter().any(|&(captured, _)| captured == node))
                {
                    return Err(OwnerScoreSessionError::UncapturedNode(node));
                }
                Ok((node, geometry))
            })
            .collect::<Result<Vec<_>, OwnerScoreSessionError>>()?;
        for (node, geometry) in canonical {
            self.publish_canonical(node, geometry)?;
        }
        Ok(())
    }
    pub(super) fn publish_local_edge_points(
        &mut self,
        local: EdgeId,
        points: Vec<Point>,
    ) -> Result<(), OwnerScoreSessionError> {
        let canonical = self.mapping.canonical_edge(local)?;
        if self
            .trials
            .iter()
            .any(|trial| !trial.edges.iter().any(|(edge, _)| *edge == canonical))
        {
            return Err(OwnerScoreSessionError::UncapturedEdge(canonical));
        }
        let edge = self
            .owner
            .edge_records
            .get_mut(canonical.0 as usize)
            .ok_or(OwnerTopologyError::UnknownEdge(canonical))?;
        edge.route.replace(RoutePoints::literal(&(points)));
        Ok(())
    }

    fn restore_geometry(&mut self, trial: GeometryTrial) {
        for (node, geometry) in trial.nodes {
            let node = &mut self.owner.nodes[node.0 as usize];
            node.position = geometry.position;
            node.rect = geometry.rect;
        }
        for (edge, points) in trial.edges {
            self.owner.edge_records[edge.0 as usize]
                .route
                .replace(RoutePoints::literal(&(points)));
        }
    }
    pub(super) fn rollback_trial(
        &mut self,
        token: OwnerGeometryTrialToken,
    ) -> Result<(), OwnerScoreSessionError> {
        if self.trials.last().map(|trial| trial.token) != Some(token) {
            return Err(OwnerScoreSessionError::InvalidTrialToken);
        }
        let trial = self.trials.pop().expect("checked geometry stack");
        self.restore_geometry(trial);
        Ok(())
    }
    pub(super) fn commit_trial(
        &mut self,
        token: OwnerGeometryTrialToken,
    ) -> Result<(), OwnerScoreSessionError> {
        if self.trials.last().map(|trial| trial.token) != Some(token) {
            return Err(OwnerScoreSessionError::InvalidTrialToken);
        }
        self.trials.pop();
        Ok(())
    }

    /// node_placement.go205–211: restore eligible records for this component,
    /// call direct, then reapply the WHOLE ordered slice before examining its
    /// returned error. R may itself be Result. A panic follows source deferred
    /// enclosing cleanup rather than pretending the skipped Apply ran.
    pub(super) fn with_restored_edges<R>(
        &mut self,
        direct: impl FnOnce(&mut Self) -> R,
    ) -> Result<R, OwnerScoreSessionError> {
        let Some(slice) = self.abductions else {
            return Ok(direct(self));
        };
        let roots = self
            .owner
            .owner_graph_view(self.current_view)
            .ok_or(OwnerTopologyError::UnknownGraph(self.current_view))?
            .nodes
            .clone();
        let roots = roots.require_nodes().collect::<Vec<_>>();
        self.owner.restore_owner_edge_abductions(slice, &roots)?;
        let result = direct(self);
        self.owner.apply_owner_edge_abductions(slice);
        Ok(result)
    }
}

impl Drop for OwnerScoreSession<'_> {
    fn drop(&mut self) {
        while let Some(trial) = self.trials.pop() {
            self.restore_geometry(trial);
        }
        if !self.ownership_committed {
            for journal in self.ownership.iter_mut().rev() {
                if let Some(journal) = journal.take() {
                    self.owner.restore_owner_graph_ownership(&journal);
                }
            }
        } else if let Some(parent) = self.parent_ownership.as_mut() {
            parent.append(&mut self.ownership);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (ArenaGraph, [NodeId; 5]) {
        fixture_with_foreign_edge(true)
    }

    fn fixture_with_foreign_edge(include_foreign_edge: bool) -> (ArenaGraph, [NodeId; 5]) {
        let mut input = Graph::default();
        let c = input.add_node(crate::Node::new(
            "C",
            Size {
                width: 60.0,
                height: 60.0,
            },
        ));
        let mut child = crate::Node::new(
            "c",
            Size {
                width: 10.0,
                height: 10.0,
            },
        );
        child.parent = Some(c);
        let child_c = input.add_node(child);
        let d = input.add_node(crate::Node::new(
            "D",
            Size {
                width: 60.0,
                height: 60.0,
            },
        ));
        let mut child = crate::Node::new(
            "d",
            Size {
                width: 10.0,
                height: 10.0,
            },
        );
        child.parent = Some(d);
        let child_d = input.add_node(child);
        let leaf = input.add_node(crate::Node::new(
            "L",
            Size {
                width: 10.0,
                height: 10.0,
            },
        ));
        input.add_edge(crate::Edge {
            source: child_c,
            target: leaf,
        });
        if include_foreign_edge {
            input.add_edge(crate::Edge {
                source: child_d,
                target: leaf,
            });
        }
        let mut owner = ArenaGraph::from_input(&input);
        owner.initialize_shared_topology();
        for (node, x, y) in [
            (c, 80.0, 80.0),
            (child_c, 100.0, 100.0),
            (d, 300.0, 80.0),
            (child_d, 320.0, 100.0),
            (leaf, 500.0, 100.0),
        ] {
            owner.set_position(node, Point { x, y });
        }
        (owner, [c, child_c, d, child_d, leaf])
    }
    fn mapping(owner: &ArenaGraph) -> CanonicalScoreMapping {
        CanonicalScoreMapping::new(
            owner.topology_arena_id().unwrap(),
            (0..owner.nodes.len())
                .map(|index| NodeId(index as u32))
                .collect(),
            (0..owner.edge_records.len())
                .map(|index| EdgeId(index as u32))
                .collect(),
        )
    }
    fn moved(owner: &ArenaGraph, node: NodeId, x: f64) -> OwnerNodeGeometry {
        let mut geometry = OwnerNodeGeometry::from_node(&owner.nodes[node.0 as usize]);
        geometry.position = Some(Point { x, y: 17.0 });
        geometry.rect.origin = geometry.position.unwrap();
        geometry
    }

    #[test]
    fn compaction_snapshot_restores_fractional_descendants_and_original_point_headers() {
        let (mut owner, [c, child, d, _, _]) = fixture();
        let initial = Point { x: 620.4, y: 521.6 };
        let point = owner.allocate_owner_node_point(Some(initial));
        owner.bind_owner_node_point(child, point.clone()).unwrap();
        let canonical = mapping(&owner);
        let mut session =
            OwnerScoreSession::new(&mut owner, OwnerGraphId(0), canonical, None).unwrap();
        {
            let mut snapshot = session.compaction_positions(&[c, c]).unwrap();
            assert_eq!(
                snapshot
                    .positions
                    .iter()
                    .map(|row| row.0)
                    .collect::<Vec<_>>(),
                [c, child]
            );
            for delta in [-2084.0, -1042.0, 2084.0] {
                snapshot
                    .session()
                    .owner_mut()
                    .translate_owner_point(child, delta, delta);
                snapshot.restore();
                let restored = snapshot.session().owner().position(child).unwrap();
                assert_eq!(
                    (restored.x.to_bits(), restored.y.to_bits()),
                    (initial.x.to_bits(), initial.y.to_bits())
                );
            }
            let replacement = snapshot
                .session()
                .owner_mut()
                .allocate_owner_node_point(None);
            snapshot
                .session()
                .owner_mut()
                .bind_owner_node_point(child, replacement)
                .unwrap();
            // A positional rollback must neither replace sizes nor undo an
            // unrelated node's update outside the selected descendant closure.
            snapshot.session().owner_mut().nodes[child.0 as usize]
                .rect
                .size
                .width = 99.0;
            snapshot
                .session()
                .owner_mut()
                .set_position(d, Point { x: 7.0, y: 9.0 });
        }
        assert!(
            session
                .owner()
                .owner_node_point(child)
                .unwrap()
                .same_allocation(&point)
        );
        assert_eq!(session.owner().position(child), Some(initial));
        assert_eq!(
            session.owner().nodes[child.0 as usize].rect.size.width,
            99.0
        );
        assert_eq!(session.owner().position(d), Some(Point { x: 7.0, y: 9.0 }));
    }

    #[test]
    fn explicit_node_and_edge_mapping_composes_each_component_without_numeric_inference() {
        let (owner, [c, _, d, _, leaf]) = fixture();
        let original = mapping(&owner);
        let selected = original
            .compose(&[leaf, c, d], &[EdgeId(1), EdgeId(0)])
            .unwrap();
        let nested = selected
            .compose(&[NodeId(1), NodeId(0)], &[EdgeId(1)])
            .unwrap();
        assert_eq!(nested.nodes(), [c, leaf]);
        assert_eq!(nested.edges(), [EdgeId(0)]);
        assert!(matches!(
            nested.canonical_node(NodeId(2)),
            Err(OwnerScoreSessionError::UnknownLocalNode(NodeId(2)))
        ));
        assert!(matches!(
            nested.compose(&[NodeId(0)], &[EdgeId(1)]),
            Err(OwnerScoreSessionError::UnknownLocalEdge(EdgeId(1)))
        ));
        let (mut other, _) = fixture();
        assert!(matches!(
            OwnerScoreSession::new(&mut other, OwnerGraphId(0), original, None),
            Err(OwnerScoreSessionError::Frame(
                OwnerScoreError::ForeignArena { .. }
            ))
        ));
    }

    #[test]
    fn component_descendants_share_actual_owner_but_remain_outside_members_and_failure_restores() {
        let (mut owner, [c, child_c, d, child_d, leaf]) = fixture();
        let canonical = mapping(&owner);
        {
            let mut session =
                OwnerScoreSession::new(&mut owner, OwnerGraphId(0), canonical, None).unwrap();
            let token = session
                .enter_component(&[c, leaf], &[EdgeId(0)], &[NodeId(0), NodeId(1)])
                .unwrap();
            let selected = session.current_view();
            assert_eq!(
                session
                    .owner()
                    .owner_graph_view(selected)
                    .unwrap()
                    .nodes
                    .require_nodes()
                    .collect::<Vec<_>>(),
                [c, leaf]
            );
            assert_eq!(session.owner().topology_node_owner(child_c), Some(selected));
            assert_eq!(
                session.owner().topology_node_owner(d),
                Some(OwnerGraphId(0))
            );
            assert_eq!(
                session.owner().topology_node_owner(child_d),
                Some(OwnerGraphId(0))
            );
            session.leave_component(token, false).unwrap();
            assert_eq!(
                session.owner().topology_node_owner(child_c),
                Some(OwnerGraphId(0))
            );
        }
        assert_eq!(owner.topology_node_owner(c), Some(OwnerGraphId(0)));
    }

    #[test]
    fn successful_component_leave_keeps_graph_until_enclosing_source_restore() {
        let (mut owner, [c, child_c, _, _, leaf]) = fixture();
        let canonical = mapping(&owner);
        let selected;
        {
            let mut session =
                OwnerScoreSession::new(&mut owner, OwnerGraphId(0), canonical, None).unwrap();
            let token = session
                .enter_component(&[c, leaf], &[EdgeId(0)], &[NodeId(0), NodeId(1)])
                .unwrap();
            selected = session.current_view();
            session.leave_component(token, true).unwrap();
            assert_eq!(session.current_view(), OwnerGraphId(0));
            assert_eq!(session.owner().topology_node_owner(child_c), Some(selected));
            session.commit_scope_ownership().unwrap();
        }
        assert_eq!(owner.topology_node_owner(c), Some(selected));
        owner
            .set_owner_graph_reference(
                OwnerGraphId(0),
                &[c, leaf],
                true,
                &mut OwnerGraphOwnershipJournal::default(),
            )
            .unwrap();
        assert_eq!(owner.topology_node_owner(child_c), Some(OwnerGraphId(0)));
    }

    #[test]
    fn outer_error_unwinds_successfully_left_nested_components_in_creation_order() {
        let (mut owner, [c, child_c, _, _, leaf]) = fixture();
        let canonical = mapping(&owner);
        {
            let mut session =
                OwnerScoreSession::new(&mut owner, OwnerGraphId(0), canonical, None).unwrap();
            let outer = session
                .enter_component(&[c, leaf], &[EdgeId(0)], &[NodeId(0), NodeId(1)])
                .unwrap();
            let outer_view = session.current_view();
            let inner = session
                .enter_component(&[NodeId(0)], &[], &[NodeId(0)])
                .unwrap();
            let inner_view = session.current_view();
            assert_ne!(outer_view, inner_view);
            assert!(matches!(
                session.leave_component(outer, true),
                Err(OwnerScoreSessionError::InvalidComponentToken)
            ));
            session.leave_component(inner, true).unwrap();
            assert_eq!(
                session.owner().topology_node_owner(child_c),
                Some(inner_view)
            );
            session.leave_component(outer, true).unwrap();
            // No splitCommitted on an outer Err: Drop restores both journals.
        }
        assert_eq!(owner.topology_node_owner(c), Some(OwnerGraphId(0)));
        assert_eq!(owner.topology_node_owner(child_c), Some(OwnerGraphId(0)));
        assert_eq!(owner.topology_node_owner(leaf), Some(OwnerGraphId(0)));
    }

    #[test]
    fn failed_outer_leave_rewinds_successfully_left_child_before_parent() {
        let (mut owner, [c, child_c, _, _, leaf]) = fixture();
        let canonical = mapping(&owner);
        let mut session =
            OwnerScoreSession::new(&mut owner, OwnerGraphId(0), canonical, None).unwrap();
        let outer = session
            .enter_component(&[c, leaf], &[EdgeId(0)], &[NodeId(0), NodeId(1)])
            .unwrap();
        let inner = session
            .enter_component(&[NodeId(0)], &[], &[NodeId(0)])
            .unwrap();
        session.leave_component(inner, true).unwrap();
        session.leave_component(outer, false).unwrap();
        assert_eq!(
            session.owner().topology_node_owner(child_c),
            Some(OwnerGraphId(0))
        );
        session.commit_scope_ownership().unwrap();
    }

    #[test]
    fn trial_captures_foreign_map_descendants_and_nil_slots_without_hydrating_coordinates() {
        let (mut owner, [c, child_c, d, child_d, leaf]) = fixture();
        owner.nodes[child_d.0 as usize].position = None;
        owner.nodes[child_d.0 as usize].rect.origin.x = -0.0;
        owner.containers.replace_header(
            None,
            SharedNodeSlice::from_slots(&[Some(c), None, Some(d), Some(leaf)]).unwrap(),
        );
        let canonical = mapping(&owner);
        let selected = owner
            .create_owner_graph_view(vec![c, leaf], vec![EdgeId(0)])
            .unwrap();
        owner
            .set_owner_view_node_mapping(selected, canonical.nodes().to_vec())
            .unwrap();
        let before = owner.nodes[child_d.0 as usize].rect.origin.x.to_bits();
        let geometry = moved(&owner, child_d, 7.0);
        let mut session = OwnerScoreSession::new(&mut owner, selected, canonical, None).unwrap();
        let token = session.begin_trial().unwrap();
        session.publish_canonical(child_d, geometry).unwrap();
        session.publish_canonical(child_c, geometry).unwrap();
        session.rollback_trial(token).unwrap();
        assert_eq!(session.owner().position(child_d), None);
        assert_eq!(
            session.owner().nodes[child_d.0 as usize]
                .rect
                .origin
                .x
                .to_bits(),
            before
        );
        assert_eq!(
            session.owner().position(child_c),
            Some(Point { x: 100.0, y: 100.0 })
        );
        assert_eq!(
            session.owner().containers.get_header(&None).unwrap().get(1),
            Some(None)
        );
    }

    #[test]
    fn ordered_publication_is_atomic_on_missing_mapping_and_last_write_wins_for_overlap() {
        let (mut owner, [c, _, _, _, _]) = fixture();
        let first = moved(&owner, c, 8.0);
        let last = moved(&owner, c, 9.0);
        let arena = owner.topology_arena_id().unwrap();
        let selected = owner.create_owner_graph_view(vec![c], Vec::new()).unwrap();
        owner
            .set_owner_view_node_mapping(selected, vec![c, c])
            .unwrap();
        let canonical = CanonicalScoreMapping::new(arena, vec![c, c], Vec::new());
        let mut session = OwnerScoreSession::new(&mut owner, selected, canonical, None).unwrap();
        assert!(
            session
                .publish_local_batch(&[(NodeId(0), first), (NodeId(2), last)])
                .is_err()
        );
        assert_eq!(session.owner().position(c).unwrap().x, 80.0);
        let trial = session.begin_trial().unwrap();
        session
            .publish_local_batch(&[(NodeId(0), first), (NodeId(1), last)])
            .unwrap();
        assert_eq!(session.owner().position(c).unwrap().x, 9.0);
        session.rollback_trial(trial).unwrap();
        assert_eq!(session.owner().position(c).unwrap().x, 80.0);
    }

    #[test]
    fn pending_trial_and_component_cleanup_runs_on_panic_without_replacing_owner_objects() {
        let (mut owner, [c, child_c, _, _, leaf]) = fixture();
        let canonical = mapping(&owner);
        let geometry = moved(&owner, c, 900.0);
        let node_address = &owner.nodes[c.0 as usize] as *const ArenaNode as usize;
        let failure = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut session =
                OwnerScoreSession::new(&mut owner, OwnerGraphId(0), canonical, None).unwrap();
            session
                .enter_component(&[c, leaf], &[EdgeId(0)], &[NodeId(0), NodeId(1)])
                .unwrap();
            session.begin_trial().unwrap();
            session.publish_local(NodeId(0), geometry).unwrap();
            panic!("source transaction failure");
        }));
        assert!(failure.is_err());
        assert_eq!(owner.position(c).unwrap().x, 80.0);
        assert_eq!(owner.topology_node_owner(child_c), Some(OwnerGraphId(0)));
        assert_eq!(
            &owner.nodes[c.0 as usize] as *const ArenaNode as usize,
            node_address
        );
    }

    #[test]
    fn reborrowed_optimizer_and_joined_frame_observe_live_owner_and_restore_short_trial() {
        let (mut owner, [c, _, _, _, _]) = fixture();
        let canonical = mapping(&owner);
        let geometry = moved(&owner, c, 23.0);
        let mut parent =
            OwnerScoreSession::new(&mut owner, OwnerGraphId(0), canonical, None).unwrap();
        {
            let mut optimizer = parent.reborrow();
            let trial = optimizer.begin_trial().unwrap();
            optimizer.publish_local(c, geometry).unwrap();
            let values = optimizer
                .with_frame(|frame| {
                    std::thread::scope(|threads| {
                        let first = threads.spawn(|| frame.position(c).unwrap().unwrap().x);
                        let second = threads.spawn(|| frame.position(c).unwrap().unwrap().x);
                        (first.join().unwrap(), second.join().unwrap())
                    })
                })
                .unwrap();
            assert_eq!(values, (23.0, 23.0));
            optimizer.rollback_trial(trial).unwrap();
        }
        assert_eq!(parent.owner().position(c).unwrap().x, 80.0);
    }

    #[test]
    fn successful_reborrow_transfers_owner_journals_to_enclosing_error_cleanup() {
        let (mut owner, [c, child_c, _, _, leaf]) = fixture();
        let canonical = mapping(&owner);
        {
            let mut parent =
                OwnerScoreSession::new(&mut owner, OwnerGraphId(0), canonical, None).unwrap();
            {
                let mut child = parent.reborrow();
                let token = child
                    .enter_component(&[c, leaf], &[EdgeId(0)], &[NodeId(0), NodeId(1)])
                    .unwrap();
                child.leave_component(token, true).unwrap();
                child.commit_scope_ownership().unwrap();
            }
            assert_ne!(parent.owner().topology_node_owner(c), Some(OwnerGraphId(0)));
            // Parent returns Err before splitCommitted; transferred journals
            // must survive the shorter successful session and unwind here.
        }
        assert_eq!(owner.topology_node_owner(child_c), Some(OwnerGraphId(0)));
    }

    #[test]
    fn whole_abduction_slice_reapplies_after_direct_returned_error_and_keeps_foreign_records() {
        let (mut owner, [c, child_c, d, child_d, leaf]) = fixture();
        let slice = owner
            .register_owner_edge_abductions(
                vec![
                    OwnerEdgeAbduction {
                        source_edge: Some(EdgeId(0)),
                        originally_from: Some(child_c),
                        originally_to: None,
                        current_from: Some(c),
                        current_to: Some(leaf),
                    },
                    OwnerEdgeAbduction {
                        source_edge: Some(EdgeId(1)),
                        originally_from: Some(child_d),
                        originally_to: None,
                        current_from: Some(d),
                        current_to: Some(leaf),
                    },
                ],
                true,
            )
            .unwrap();
        owner.abduct_owner_edge_abductions(&slice);
        let canonical = mapping(&owner);
        let mut session =
            OwnerScoreSession::new(&mut owner, OwnerGraphId(0), canonical, Some(&slice)).unwrap();
        let component = session
            .enter_component(&[c, leaf], &[EdgeId(0)], &[NodeId(0), NodeId(1)])
            .unwrap();
        let result = session
            .with_restored_edges(|session| {
                assert_eq!(session.owner().edge_records[0].from, child_c);
                assert_eq!(session.owner().edge_records[1].from, d);
                assert_eq!(session.abductions().unwrap().records.len(), 2);
                Err::<(), _>("direct failed")
            })
            .unwrap();
        assert_eq!(result, Err("direct failed"));
        assert_eq!(session.owner().edge_records[0].from, c);
        assert_eq!(session.owner().edge_records[1].from, d);
        assert!(
            !session.owner().nodes[child_c.0 as usize]
                .edges
                .contains(&EdgeId(0))
        );
        assert_eq!(session.owner().nodes[c.0 as usize].edges, [EdgeId(0)]);
        assert_eq!(session.owner().nodes[d.0 as usize].edges, [EdgeId(1)]);
        session.leave_component(component, false).unwrap();
    }

    #[test]
    fn geometry_trial_tokens_are_receiver_bound_and_nested_commit_does_not_defeat_outer_rollback() {
        let (mut owner, [c, _, _, _, _]) = fixture();
        let canonical = mapping(&owner);
        let geometry = moved(&owner, c, 70.0);
        let mut session =
            OwnerScoreSession::new(&mut owner, OwnerGraphId(0), canonical, None).unwrap();
        let outer = session.begin_trial().unwrap();
        let inner = session.begin_trial().unwrap();
        assert!(matches!(
            session.rollback_trial(outer),
            Err(OwnerScoreSessionError::InvalidTrialToken)
        ));
        session.publish_canonical(c, geometry).unwrap();
        session.commit_trial(inner).unwrap();
        {
            let mut other = session.reborrow();
            let other_token = other.begin_trial().unwrap();
            assert!(matches!(
                other.rollback_trial(outer),
                Err(OwnerScoreSessionError::InvalidTrialToken)
            ));
            other.rollback_trial(other_token).unwrap();
        }
        session.rollback_trial(outer).unwrap();
        assert_eq!(session.owner().position(c).unwrap().x, 80.0);
    }

    #[test]
    fn component_cell_size_is_computed_from_members_not_retained_foreign_nodes() {
        let (mut owner, [c, _, _, _, leaf]) = fixture();
        let canonical = mapping(&owner);
        let mut session =
            OwnerScoreSession::new(&mut owner, OwnerGraphId(0), canonical, None).unwrap();
        let prepared = session
            .prepare_component(&[c, leaf], &[EdgeId(0)], &[NodeId(0), NodeId(1)])
            .unwrap();
        assert_eq!(
            session
                .owner()
                .owner_graph_cell_size(prepared.view())
                .unwrap(),
            0.0
        );
        let token = session
            .enter_prepared_component(prepared, &[c, leaf], &[EdgeId(0)], &[NodeId(0), NodeId(1)])
            .unwrap();
        assert_eq!(
            session
                .owner()
                .owner_graph_cell_size(session.current_view())
                .unwrap(),
            15.0
        );
        assert_eq!(session.compute_current_cell_size().unwrap(), 15.0);
        session.leave_component(token, false).unwrap();
    }

    #[test]
    fn all_split_roots_have_distinct_live_graphs_before_first_component_descendants_enter() {
        let (mut owner, [c, child_c, d, child_d, leaf]) = fixture_with_foreign_edge(false);
        let canonical = mapping(&owner);
        let selected;
        let foreign;
        {
            let mut session =
                OwnerScoreSession::new(&mut owner, OwnerGraphId(0), canonical, None).unwrap();
            let first = session
                .prepare_component(&[c, leaf], &[EdgeId(0)], &[NodeId(0), NodeId(1)])
                .unwrap();
            let next = session.prepare_component(&[d], &[], &[NodeId(0)]).unwrap();
            selected = first.view();
            foreign = next.view();
            assert_ne!(selected, foreign);
            assert_eq!(session.current_view(), OwnerGraphId(0));
            assert_eq!(session.canonical_node(c).unwrap(), c);
            assert_eq!(session.owner().topology_node_owner(c), Some(selected));
            assert_eq!(session.owner().topology_node_owner(d), Some(foreign));
            assert_eq!(
                session.owner().topology_node_owner(child_c),
                Some(OwnerGraphId(0))
            );
            assert_eq!(
                session.owner().topology_node_owner(child_d),
                Some(OwnerGraphId(0))
            );
            let first_active = session
                .enter_prepared_component(first, &[c, leaf], &[EdgeId(0)], &[NodeId(0), NodeId(1)])
                .unwrap();
            assert_eq!(session.owner().topology_node_owner(child_c), Some(selected));
            assert_eq!(
                session.owner().topology_node_owner(child_d),
                Some(OwnerGraphId(0))
            );
            assert_eq!(
                session
                    .owner()
                    .owner_graph_view(selected)
                    .unwrap()
                    .nodes
                    .require_nodes()
                    .collect::<Vec<_>>(),
                [c, leaf]
            );
            assert_eq!(session.owner().topology_node_owner(d), Some(foreign));
            session.leave_component(first_active, true).unwrap();
            let next_active = session
                .enter_prepared_component(next, &[d], &[], &[NodeId(0)])
                .unwrap();
            assert_eq!(session.owner().topology_node_owner(child_d), Some(foreign));
            assert_eq!(
                session
                    .owner()
                    .owner_graph_view(foreign)
                    .unwrap()
                    .nodes
                    .require_nodes()
                    .collect::<Vec<_>>(),
                [d]
            );
            session.leave_component(next_active, true).unwrap();
            session.commit_scope_ownership().unwrap();
        }
        assert_eq!(owner.topology_node_owner(child_c), Some(selected));
        assert_eq!(owner.topology_node_owner(child_d), Some(foreign));
        owner
            .set_owner_graph_reference(
                OwnerGraphId(0),
                &[c, d, leaf],
                true,
                &mut OwnerGraphOwnershipJournal::default(),
            )
            .unwrap();
        assert_eq!(owner.topology_node_owner(child_c), Some(OwnerGraphId(0)));
        assert_eq!(owner.topology_node_owner(child_d), Some(OwnerGraphId(0)));
    }

    #[test]
    fn unentered_prepared_components_block_success_and_drop_restores_every_split_root() {
        let (mut owner, [c, child_c, d, child_d, leaf]) = fixture_with_foreign_edge(false);
        let canonical = mapping(&owner);
        {
            let mut session =
                OwnerScoreSession::new(&mut owner, OwnerGraphId(0), canonical, None).unwrap();
            let first = session
                .prepare_component(&[c, leaf], &[EdgeId(0)], &[NodeId(0), NodeId(1)])
                .unwrap();
            let next = session.prepare_component(&[d], &[], &[NodeId(0)]).unwrap();
            let token = session
                .enter_prepared_component(first, &[c, leaf], &[EdgeId(0)], &[NodeId(0), NodeId(1)])
                .unwrap();
            session.leave_component(token, true).unwrap();
            assert_eq!(session.owner().topology_node_owner(d), Some(next.view()));
            assert!(matches!(
                session.commit_scope_ownership(),
                Err(OwnerScoreSessionError::PendingPreparedComponent)
            ));
            // Failure before the next component: successful prior descendant
            // journals and unentered foreign root journals both unwind.
        }
        for node in [c, child_c, d, child_d, leaf] {
            assert_eq!(owner.topology_node_owner(node), Some(OwnerGraphId(0)));
        }
    }

    #[test]
    fn freshly_induced_node_edge_and_root_correspondence_is_validated_before_component_mutation() {
        let (mut owner, [c, child_c, d, _, leaf]) = fixture();
        let canonical = mapping(&owner);
        let mut session =
            OwnerScoreSession::new(&mut owner, OwnerGraphId(0), canonical, None).unwrap();
        let prepared = session
            .prepare_component(&[c, leaf], &[EdgeId(0)], &[NodeId(0), NodeId(1)])
            .unwrap();
        let view = prepared.view();
        for (nodes, edges, roots) in [
            (vec![d, leaf], vec![EdgeId(0)], vec![NodeId(0), NodeId(1)]),
            (vec![c, leaf], vec![EdgeId(1)], vec![NodeId(0), NodeId(1)]),
            (vec![c, leaf], vec![EdgeId(0)], vec![NodeId(0)]),
        ] {
            assert!(matches!(
                session.enter_prepared_component(prepared.clone(), &nodes, &edges, &roots),
                Err(OwnerScoreSessionError::PreparedMappingMismatch)
            ));
            assert_eq!(
                session.owner().topology_node_owner(child_c),
                Some(OwnerGraphId(0))
            );
            assert_eq!(session.owner().owner_graph_cell_size(view).unwrap(), 0.0);
            assert_eq!(session.current_view(), OwnerGraphId(0));
        }
        let active = session
            .enter_prepared_component(prepared, &[c, leaf], &[EdgeId(0)], &[NodeId(0), NodeId(1)])
            .unwrap();
        assert_eq!(session.owner().topology_node_owner(child_c), Some(view));
        session.leave_component(active, false).unwrap();
    }

    #[test]
    fn prepared_component_tokens_are_session_bound_and_cannot_enter_twice() {
        let (mut owner, [c, _, _, _, leaf]) = fixture();
        let canonical = mapping(&owner);
        let mut session =
            OwnerScoreSession::new(&mut owner, OwnerGraphId(0), canonical, None).unwrap();
        let prepared = session
            .prepare_component(&[c, leaf], &[EdgeId(0)], &[NodeId(0), NodeId(1)])
            .unwrap();
        let duplicate = prepared.clone();
        let (mut other, _) = fixture();
        let other_mapping = mapping(&other);
        let mut other_session =
            OwnerScoreSession::new(&mut other, OwnerGraphId(0), other_mapping, None).unwrap();
        assert!(matches!(
            other_session.enter_prepared_component(
                duplicate.clone(),
                &[c, leaf],
                &[EdgeId(0)],
                &[NodeId(0), NodeId(1)]
            ),
            Err(OwnerScoreSessionError::InvalidPreparedComponentToken)
        ));
        let active = session
            .enter_prepared_component(prepared, &[c, leaf], &[EdgeId(0)], &[NodeId(0), NodeId(1)])
            .unwrap();
        session.leave_component(active, true).unwrap();
        assert!(matches!(
            session.enter_prepared_component(
                duplicate,
                &[c, leaf],
                &[EdgeId(0)],
                &[NodeId(0), NodeId(1)]
            ),
            Err(OwnerScoreSessionError::InvalidPreparedComponentToken)
        ));
        session.commit_scope_ownership().unwrap();
    }

    fn initialization_fixture() -> (
        ArenaGraph,
        ArenaGraph,
        OwnerGraphId,
        CanonicalScoreMapping,
        [NodeId; 5],
    ) {
        let (mut owner, nodes @ [c, _, _, _, leaf]) = fixture();
        let mut workspace = owner.clone();
        workspace.node_order = vec![c, leaf];
        workspace.refresh_node_order_membership();
        workspace.set_position(c, Point { x: -35.0, y: -45.0 });
        workspace.set_position(leaf, Point { x: 15.0, y: 25.0 });
        let mapping = mapping(&owner);
        let view = owner
            .create_owner_graph_view(vec![c, leaf], owner.edge_order.read())
            .unwrap();
        owner
            .set_owner_view_node_mapping(view, mapping.nodes().to_vec())
            .unwrap();
        (owner, workspace, view, mapping, nodes)
    }

    #[test]
    fn initialization_publishes_actual_members_and_refits_retained_child_without_dense_geometry() {
        let (mut owner, mut workspace, view, mapping, [c, child_c, d, child_d, leaf]) =
            initialization_fixture();
        // Deliberately stale dense child/size data cannot hydrate the live
        // retained pointer; initialization publishes member TopLeft only.
        workspace.set_position(
            child_c,
            Point {
                x: 9000.0,
                y: 9000.0,
            },
        );
        workspace.nodes[c.0 as usize].rect.size.width = 999.0;
        let foreign_before = [owner.position(d), owner.position(child_d)];
        let child_before = owner.position(child_c);
        let node_address = &owner.nodes[child_c.0 as usize] as *const ArenaNode;
        let retained = owner
            .owner_container_map(OwnerGraphId(0))
            .unwrap()
            .get(Some(c))
            .unwrap();
        let mut session = OwnerScoreSession::new(&mut owner, view, mapping, None).unwrap();
        session
            .initialize_current_member_positions(&workspace)
            .unwrap();
        assert_eq!(session.owner().position(c), workspace.position(c));
        assert_eq!(session.owner().position(leaf), workspace.position(leaf));
        assert_eq!(session.owner().nodes[c.0 as usize].rect.size.width, 60.0);
        assert_ne!(session.owner().position(child_c), child_before);
        assert_ne!(
            session.owner().position(child_c),
            workspace.position(child_c)
        );
        assert_eq!(
            [
                session.owner().position(d),
                session.owner().position(child_d)
            ],
            foreign_before
        );
        assert_eq!(
            &session.owner().nodes[child_c.0 as usize] as *const ArenaNode,
            node_address
        );
        assert_eq!(retained.get(0), Some(Some(child_c)));
    }

    #[test]
    fn completed_direction_pass_updates_retained_points_before_later_component() {
        let (mut owner, [c, child_c, d, child_d, leaf]) = fixture();
        let before = owner.position(child_c).unwrap();
        let retained = owner.allocate_owner_node_point(Some(before));
        owner
            .bind_owner_node_point(child_c, retained.clone())
            .unwrap();
        let address = owner.owner_bound_point_address(child_c);
        let foreign = [owner.position(d), owner.position(child_d)];
        let canonical_mapping = mapping(&owner);
        let mut session =
            OwnerScoreSession::new(&mut owner, OwnerGraphId(0), canonical_mapping, None).unwrap();
        let first = session
            .enter_component(&[c, child_c], &[], &[NodeId(0)])
            .unwrap();
        let mut reflected =
            OwnerNodeGeometry::from_node(&session.owner().nodes[child_c.0 as usize]);
        reflected.position = Some(Point {
            x: -before.x - reflected.rect.size.width,
            y: before.y,
        });
        reflected.rect.origin = reflected.position.unwrap();
        session
            .publish_direct_geometry(&[(NodeId(1), reflected)])
            .unwrap();
        session.leave_component(first, true).unwrap();
        let second = session.enter_component(&[leaf], &[], &[NodeId(0)]).unwrap();
        assert_eq!(session.owner().position(child_c), reflected.position);
        assert_eq!(retained.get(), reflected.position);
        assert_eq!(session.owner().owner_bound_point_address(child_c), address);
        assert_eq!(
            [
                session.owner().position(d),
                session.owner().position(child_d)
            ],
            foreign
        );
        session.leave_component(second, true).unwrap();
    }

    #[test]
    fn compaction_publication_translates_shared_descendants_without_refitting() {
        let (mut owner, mut workspace, view, mapping, [c, child_c, d, child_d, leaf]) =
            initialization_fixture();
        let origin = Point { x: 100.0, y: 200.0 };
        let child_origin = Point { x: 137.0, y: 259.0 };
        owner.set_position(c, origin);
        owner.set_position(child_c, child_origin);
        let point = owner.allocate_owner_node_point(Some(child_origin));
        owner.bind_owner_node_point(child_c, point.clone()).unwrap();
        let foreign_before = [owner.position(d), owner.position(child_d)];
        workspace.set_position(c, Point { x: 111.0, y: 183.0 });
        workspace.set_position(leaf, owner.position(leaf).unwrap());
        let mut session = OwnerScoreSession::new(&mut owner, view, mapping, None).unwrap();
        session.update_current_member_positions(&workspace).unwrap();
        assert_eq!(
            session.owner().position(c),
            Some(Point { x: 111.0, y: 183.0 })
        );
        assert_eq!(
            session.owner().position(child_c),
            Some(Point { x: 148.0, y: 242.0 })
        );
        assert!(
            session
                .owner()
                .owner_node_point(child_c)
                .unwrap()
                .same_allocation(&point)
        );
        assert_eq!(
            [
                session.owner().position(d),
                session.owner().position(child_d)
            ],
            foreign_before
        );
    }

    #[test]
    fn direct_copyback_reads_live_point_when_inline_position_is_stale_or_nil() {
        for inline in [Some(Point { x: 900.0, y: 901.0 }), None] {
            let (mut owner, mut workspace, receiver, mapping, [c, _, _, _, _]) =
                initialization_fixture();
            let point = Point { x: -13.0, y: 42.0 };
            let source_point = workspace.allocate_owner_node_point(Some(point));
            workspace.bind_owner_node_point(c, source_point).unwrap();
            workspace.nodes[c.0 as usize].position = inline;
            workspace.nodes[c.0 as usize].rect.origin = point;
            let original = owner.allocate_owner_node_point(owner.position(c));
            owner.bind_owner_node_point(c, original.clone()).unwrap();
            let geometry = OwnerNodeGeometry::from_graph(&workspace, c);
            assert_eq!(geometry.position, Some(point));
            assert_eq!(workspace.nodes[c.0 as usize].position, inline);
            let mut session = OwnerScoreSession::new(&mut owner, receiver, mapping, None).unwrap();
            session.publish_direct_geometry(&[(c, geometry)]).unwrap();
            assert_eq!(session.owner().position(c), Some(point));
            assert_eq!(original.get(), Some(point));
            assert!(original.same_allocation(&session.owner().owner_node_point(c).unwrap()));
        }
    }

    #[test]
    fn explicit_projection_refresh_reads_live_points_preserves_nil_and_rejects_unknown_handles() {
        let (mut owner, [anchor, child, _, _, other]) = fixture();
        owner.set_position(anchor, Point { x: 5.0, y: 7.0 });
        owner.set_position(child, Point { x: 10.0, y: 20.0 });
        // An entity number is deliberately shared; the pointer handle chooses
        // the original, independently of local allocation and display identity.
        owner.nodes[other.0 as usize].tala_id = owner.nodes[child.0 as usize].tala_id;
        owner.set_position(other, Point { x: 99.0, y: 88.0 });
        let retained = owner.allocate_owner_node_point(Some(Point { x: 12.0, y: 23.0 }));
        owner
            .bind_owner_node_point(child, retained.clone())
            .unwrap();
        let mut workspace = owner.clone();
        let mut projection = owner.nodes[child.0 as usize].clone();
        projection.owner_geometry_source = Some(child);
        projection.owner_geometry_anchor = Some(anchor);
        projection.position = Some(Point { x: 100.0, y: 200.0 });
        workspace
            .transaction_external_container_children
            .insert(999, vec![projection]);
        let canonical = mapping(&owner);
        let mut session =
            OwnerScoreSession::new(&mut owner, OwnerGraphId(0), canonical, None).unwrap();
        session
            .refresh_explicit_owner_projections(&mut workspace)
            .unwrap();
        let projected = &workspace.transaction_external_container_children[&999][0];
        assert_eq!(projected.position, retained.get());
        assert_eq!(
            projected.transaction_anchor_offset,
            Some(Point { x: 7.0, y: 16.0 })
        );
        session.owner.clear_position(child);
        session
            .refresh_explicit_owner_projections(&mut workspace)
            .unwrap();
        assert!(
            workspace.transaction_external_container_children[&999][0]
                .transaction_anchor_offset
                .is_none()
        );
        assert!(
            workspace
                .anchored_external_position(
                    &workspace.transaction_external_container_children[&999][0]
                )
                .is_none()
        );
        assert!(
            workspace.transaction_external_container_children[&999][0]
                .position
                .is_none()
        );
        workspace
            .transaction_external_container_children
            .get_mut(&999)
            .unwrap()[0]
            .position = Some(Point { x: 77.0, y: 66.0 });
        let mut invalid = workspace.transaction_external_container_children[&999][0].clone();
        invalid.owner_geometry_source = Some(NodeId(u32::MAX));
        workspace.transaction_external_containers.push(invalid);
        assert!(
            session
                .refresh_explicit_owner_projections(&mut workspace)
                .is_err()
        );
        assert_eq!(
            workspace.transaction_external_container_children[&999][0].position,
            Some(Point { x: 77.0, y: 66.0 })
        );
    }

    #[test]
    fn canonical_transition_moves_real_group_members_and_retains_point_allocations() {
        struct Context;
        impl limits::Context for Context {
            fn done(&self) -> Option<bool> {
                Some(false)
            }
            fn error(&self) -> Option<limits::ContextError> {
                None
            }
        }
        for horizontal in [true, false] {
            let mut input = crate::Graph::default();
            for (i, width) in [20.0, 40.0, 10.0, 10.0].into_iter().enumerate() {
                input.add_node(crate::Node::new(
                    format!("n{i}"),
                    Size {
                        width,
                        height: 10.0,
                    },
                ));
            }
            let mut graph = ArenaGraph::from_input(&input);
            let mut points = Vec::new();
            for (i, x) in [0.0, 10.0, 10.0, 30.0].into_iter().enumerate() {
                let node = NodeId(i as u32);
                graph.nodes[i].tala_id = i as i64;
                graph.set_position(node, Point { x, y: 0.0 });
            }
            graph.initialize_shared_topology();
            for i in 0..4 {
                let node = NodeId(i);
                let retained = graph.allocate_owner_node_point(graph.position(node));
                graph.bind_owner_node_point(node, retained.clone()).unwrap();
                points.push(retained);
            }
            graph
                .set_owner_cluster_vessel_flag(NodeId(1), true)
                .unwrap();
            let group = graph.new_owner_group(OwnerGroupKind::Cluster);
            let mut fields = graph.owner_group_fields(group).unwrap();
            fields.vessel = Some(NodeId(1));
            fields.nodes = graph
                .allocate_owner_member_header(&[Some(NodeId(2)), Some(NodeId(3))], 2)
                .unwrap();
            graph.set_owner_group_fields(group, fields).unwrap();
            let map = graph.allocate_owner_group_map(OwnerGroupKind::Cluster, true);
            graph
                .write_owner_group_map_entry(&map, Some(NodeId(1)), Some(group))
                .unwrap();
            graph
                .set_owner_graph_group_map(OwnerGraphId(0), map)
                .unwrap();
            graph.cell_size = 1.0;
            graph
                .transition_compact_canonical_axis_checked(
                    OwnerGraphId(0),
                    horizontal,
                    1.0,
                    &Context,
                    1,
                )
                .unwrap();
            let expected = if horizontal {
                [0.0, 81.0, 81.0, 182.0]
            } else {
                [0.0, 10.0, 10.0, 30.0]
            };
            for (i, x) in expected.into_iter().enumerate() {
                assert_eq!(graph.position(NodeId(i as u32)), Some(Point { x, y: 0.0 }));
                assert_eq!(points[i].get(), graph.position(NodeId(i as u32)));
                assert!(
                    points[i].same_allocation(&graph.owner_node_point(NodeId(i as u32)).unwrap())
                );
            }
        }
    }

    #[test]
    fn projected_direct_writes_keep_hidden_points_and_do_not_rewind_untouched_aliases() {
        let (mut owner, [c, child_c, d, child_d, _leaf]) = fixture();
        let mut before = vec![
            owner.nodes[child_c.0 as usize].clone(),
            owner.nodes[child_d.0 as usize].clone(),
        ];
        before[0].owner_geometry_source = Some(child_c);
        before[1].owner_geometry_source = Some(child_d);
        let mut after = before.clone();
        let moved = Point {
            x: -9122.0,
            y: -5100.0,
        };
        after[0].position = Some(moved);
        after[0].rect.origin = moved;
        // The unrelated snapshot predates a real owner write. Direct did not
        // touch it, so its unchanged projection cannot overwrite that write.
        let foreign = Point { x: 812.0, y: 613.0 };
        owner.set_position(child_d, foreign);
        let retained = owner.allocate_owner_node_point(owner.position(child_c));
        owner
            .bind_owner_node_point(child_c, retained.clone())
            .unwrap();
        let address = owner.owner_bound_point_address(child_c);
        let canonical_mapping = mapping(&owner);
        let mut session =
            OwnerScoreSession::new(&mut owner, OwnerGraphId(0), canonical_mapping, None).unwrap();
        let first = session.enter_component(&[c], &[], &[NodeId(0)]).unwrap();
        session
            .publish_projected_direct_geometry(&before, &after)
            .unwrap();
        assert_eq!(session.owner().position(child_c), Some(moved));
        assert_eq!(session.owner().position(child_d), Some(foreign));
        assert_eq!(retained.get(), Some(moved));
        assert_eq!(session.owner().owner_bound_point_address(child_c), address);
        session.leave_component(first, true).unwrap();
        let second = session.enter_component(&[d], &[], &[NodeId(0)]).unwrap();
        let mut workspace = session.owner().clone();
        workspace
            .transaction_external_container_children
            .insert(1, before);
        session
            .refresh_explicit_owner_projections(&mut workspace)
            .unwrap();
        assert_eq!(
            workspace.transaction_external_container_children[&1][0].position,
            Some(moved)
        );
        assert_eq!(
            workspace.transaction_external_container_children[&1][1].position,
            Some(foreign)
        );
        session.leave_component(second, true).unwrap();
    }

    #[test]
    fn direct_copyback_includes_restored_nodes_through_explicit_parent_mapping() {
        let (mut owner, [c, child, d, restored, leaf]) = fixture();
        let arena = owner.topology_arena_id().unwrap();
        let parent_mapping =
            CanonicalScoreMapping::new(arena, vec![leaf, d, c, child, restored], vec![]);
        let parent_view = owner.create_owner_graph_view(vec![leaf], vec![]).unwrap();
        owner
            .set_owner_view_node_mapping(parent_view, parent_mapping.nodes().to_vec())
            .unwrap();
        owner.clear_position(restored);
        let untouched = owner.position(child);
        let mut session =
            OwnerScoreSession::new(&mut owner, parent_view, parent_mapping, None).unwrap();
        let component = session
            .enter_component(&[NodeId(0)], &[], &[NodeId(0)])
            .unwrap();
        assert_eq!(session.canonical_node(NodeId(0)).unwrap(), leaf);
        assert_eq!(
            session.canonical_node(NodeId(4)),
            Err(OwnerScoreSessionError::UnknownLocalNode(NodeId(4)))
        );
        let mut geometry =
            OwnerNodeGeometry::from_node(&session.owner().nodes[restored.0 as usize]);
        geometry.position = Some(Point { x: 7.0, y: -21.0 });
        geometry.rect.origin = geometry.position.unwrap();
        // A bad parent handle rejects the entire batch before publication.
        assert_eq!(
            session.publish_component_direct_geometry(&[
                (NodeId(4), geometry),
                (NodeId(99), geometry)
            ]),
            Err(OwnerScoreSessionError::UnknownLocalNode(NodeId(99)))
        );
        assert_eq!(session.owner().position(restored), None);
        session
            .publish_component_direct_geometry(&[(NodeId(4), geometry)])
            .unwrap();
        assert_eq!(session.owner().position(restored), geometry.position);
        assert_eq!(session.owner().position(child), untouched);
        session.leave_component(component, true).unwrap();
        assert_eq!(session.owner().position(restored), geometry.position);
        assert_eq!(
            session.publish_component_direct_geometry(&[]),
            Err(OwnerScoreSessionError::InvalidComponentToken)
        );
    }

    #[test]
    fn packed_scope_publishes_explicit_member_boxes_to_live_point_backings() {
        let (mut owner, mut workspace, receiver, mapping, [c, child, d, group_child, _]) =
            initialization_fixture();
        owner.set_owner_cluster_vessel_flag(c, true).unwrap();
        let group = owner.new_owner_group(OwnerGroupKind::Cluster);
        let mut fields = owner.owner_group_fields(group).unwrap();
        fields.nodes = owner
            .allocate_owner_member_header(&[Some(d), Some(group_child)], 2)
            .unwrap();
        owner.set_owner_group_fields(group, fields).unwrap();
        let map = owner.allocate_owner_group_map(OwnerGroupKind::Cluster, true);
        owner
            .write_owner_group_map_entry(&map, Some(c), Some(group))
            .unwrap();
        owner.set_owner_graph_group_map(receiver, map).unwrap();
        let mut first = owner.nodes[d.0 as usize].clone();
        first.owner_geometry_source = Some(d);
        first.position = Some(Point { x: 5.0, y: 6.0 });
        first.rect.origin = first.position.unwrap();
        first.rect.size.width = 37.0;
        let mut second = owner.nodes[group_child.0 as usize].clone();
        second.owner_geometry_source = Some(group_child);
        second.position = Some(Point { x: 8.0, y: 9.0 });
        second.rect.origin = second.position.unwrap();
        let retained = owner.allocate_owner_node_point(owner.position(d));
        owner.bind_owner_node_point(d, retained.clone()).unwrap();
        workspace
            .transaction_external_aggregate_children
            .insert(123, vec![first.clone(), second]);
        let mut session = OwnerScoreSession::new(&mut owner, receiver, mapping, None).unwrap();
        assert_eq!(
            session
                .packed_group_member_geometry(&workspace)
                .unwrap()
                .iter()
                .map(|(id, _)| *id)
                .collect::<Vec<_>>(),
            [d, group_child]
        );
        session.publish_packed_scope_geometry(&workspace).unwrap();
        assert_eq!(session.owner().position(c), workspace.position(c));
        assert_eq!(session.owner().position(d), Some(Point { x: 5.0, y: 6.0 }));
        assert_eq!(
            session.owner().position(group_child),
            Some(Point { x: 8.0, y: 9.0 })
        );
        assert_eq!(session.owner().nodes[d.0 as usize].rect.size.width, 37.0);
        assert_eq!(retained.get(), Some(Point { x: 5.0, y: 6.0 }));
        assert!(retained.same_allocation(&session.owner().owner_node_point(d).unwrap()));
        let untouched = session.owner().position(child);
        first.position = None;
        workspace
            .transaction_external_aggregate_children
            .get_mut(&123)
            .unwrap()[0] = first.clone();
        session.publish_packed_scope_geometry(&workspace).unwrap();
        assert_eq!(session.owner().position(d), None);
        assert_eq!(session.owner().position(child), untouched);
        // Conflicting aliases must reject before changing any real box.
        let mut conflict = first;
        conflict.position = Some(Point { x: 100.0, y: 200.0 });
        workspace
            .transaction_external_aggregate_children
            .insert(456, vec![conflict]);
        let before = session.owner().position(c);
        assert_eq!(
            session.publish_packed_scope_geometry(&workspace),
            Err(OwnerScoreSessionError::MemberMappingMismatch)
        );
        assert_eq!(session.owner().position(c), before);
        assert_eq!(session.owner().position(d), None);
    }

    #[test]
    fn packed_container_projection_overrides_translated_live_vessel_and_keeps_foreign_points() {
        let (mut owner, mut workspace, receiver, mapping, [c, child, foreign, _, _]) =
            initialization_fixture();
        owner.set_owner_cluster_vessel_flag(child, true).unwrap();
        let retained = owner.allocate_owner_node_point(owner.position(child));
        owner
            .bind_owner_node_point(child, retained.clone())
            .unwrap();
        let foreign_before = owner.position(foreign);
        let mut projected = owner.nodes[child.0 as usize].clone();
        projected.owner_geometry_source = Some(child);
        projected.position = Some(Point { x: 7.0, y: 9.0 });
        projected.rect.origin = projected.position.unwrap();
        projected.rect.size.width = 77.0;
        let mut unrelated = owner.nodes[foreign.0 as usize].clone();
        unrelated.owner_geometry_source = Some(foreign);
        unrelated.position = Some(Point { x: 999.0, y: 999.0 });
        workspace
            .transaction_external_container_children
            .insert(123, vec![projected.clone(), unrelated]);
        let mut session = OwnerScoreSession::new(&mut owner, receiver, mapping, None).unwrap();
        session.publish_packed_scope_geometry(&workspace).unwrap();
        assert_eq!(session.owner().position(c), workspace.position(c));
        assert_eq!(session.owner().position(child), projected.position);
        assert_eq!(
            session.owner().nodes[child.0 as usize].rect.size.width,
            77.0
        );
        assert_eq!(retained.get(), projected.position);
        assert!(retained.same_allocation(&session.owner().owner_node_point(child).unwrap()));
        assert_eq!(session.owner().position(foreign), foreign_before);
        let mut conflict = projected;
        conflict.position = Some(Point { x: -7.0, y: -9.0 });
        workspace
            .transaction_external_aggregate_children
            .insert(456, vec![conflict]);
        let before = session.owner().position(c);
        assert_eq!(
            session.publish_packed_scope_geometry(&workspace),
            Err(OwnerScoreSessionError::MemberMappingMismatch)
        );
        assert_eq!(session.owner().position(c), before);
        assert_eq!(retained.get(), Some(Point { x: 7.0, y: 9.0 }));
    }

    #[test]
    fn combined_workspace_keeps_canonical_references_and_actual_member_preorder() {
        let (mut owner, [c, child, d, group_child, leaf]) = fixture();
        let receiver = owner
            .create_owner_graph_view(vec![c, leaf], owner.edge_order.read())
            .unwrap();
        owner.set_owner_cluster_vessel_flag(c, true).unwrap();
        let group = owner.new_owner_group(OwnerGroupKind::Cluster);
        let mut fields = owner.owner_group_fields(group).unwrap();
        fields.nodes = owner
            .allocate_owner_member_header(&[Some(d), None], 2)
            .unwrap();
        owner.set_owner_group_fields(group, fields).unwrap();
        let map = owner.allocate_owner_group_map(OwnerGroupKind::Cluster, true);
        owner
            .write_owner_group_map_entry(&map, Some(c), Some(group))
            .unwrap();
        owner.set_owner_graph_group_map(receiver, map).unwrap();
        let _ = owner.nodes[c.0 as usize].nears.insert(Some(leaf));
        let prior_positions = owner
            .nodes
            .iter()
            .map(|n| owner.position(n.input_id))
            .collect::<Vec<_>>();
        let prior_owners = owner
            .nodes
            .iter()
            .map(|n| owner.topology_node_owner(n.input_id))
            .collect::<Vec<_>>();
        let mapping = CanonicalScoreMapping::new(
            owner.topology_arena_id().unwrap(),
            vec![leaf, c],
            vec![EdgeId(1), EdgeId(0)],
        );
        let mut workspace = owner.clone();
        workspace.nodes = vec![
            owner.nodes[leaf.0 as usize].clone(),
            owner.nodes[c.0 as usize].clone(),
        ];
        for (index, node) in workspace.nodes.iter_mut().enumerate() {
            node.input_id = NodeId(index as u32);
        }
        workspace.node_order = vec![NodeId(1), NodeId(0)];
        workspace
            .edge_order
            .replace_values(&vec![EdgeId(1), EdgeId(0)]);
        workspace.cell_size = 777.0;
        // The dense numerical input has no canonical point/backing bindings.
        workspace.shared_topology = Default::default();
        workspace.set_position(NodeId(1), Point { x: -75.0, y: 88.0 });
        workspace.nodes[1].rect.size.width = 91.0;
        workspace.refresh_node_order_membership();
        let mut child_geometry = OwnerNodeGeometry::from_node(&owner.nodes[child.0 as usize]);
        child_geometry.position = None;
        owner
            .set_owner_view_node_mapping(receiver, mapping.nodes().to_vec())
            .unwrap();
        let session = OwnerScoreSession::new(&mut owner, receiver, mapping, None).unwrap();
        let (candidate, combined) = session
            .combined_member_workspace(&workspace, &[(child, child_geometry)])
            .unwrap();
        assert_eq!(
            candidate.graph_node_order(),
            [c, child, d, group_child, leaf]
        );
        assert_eq!(
            candidate
                .owner_graph_view(combined)
                .unwrap()
                .nodes
                .require_nodes()
                .collect::<Vec<_>>(),
            [c, child, d, group_child, leaf]
        );
        assert_eq!(candidate.edge_order, [EdgeId(0), EdgeId(1)]);
        assert_eq!(
            candidate
                .owner_graph_view(combined)
                .unwrap()
                .local_to_canonical,
            (0..candidate.nodes.len())
                .map(|index| NodeId(index as u32))
                .collect::<Vec<_>>()
        );
        assert_eq!(candidate.cell_size, 0.0);
        assert_eq!(candidate.owner_graph_cell_size(combined).unwrap(), 0.0);
        assert!(std::sync::Arc::ptr_eq(
            &candidate.cost_cache,
            candidate.owner_graph_cost_cache(combined).unwrap()
        ));
        assert_eq!(candidate.nodes[child.0 as usize].container, Some(c));
        assert_eq!(candidate.nodes[c.0 as usize].nears.keys(), &[leaf]);
        assert_eq!(
            candidate.nodes[c.0 as usize].edges,
            session.owner().nodes[c.0 as usize].edges
        );
        assert_eq!(
            candidate.edge_records[0].from,
            session.owner().edge_records[0].from
        );
        assert_eq!(
            candidate.edge_records[0].to,
            session.owner().edge_records[0].to
        );
        assert_eq!(candidate.position(c), Some(Point { x: -75.0, y: 88.0 }));
        assert_eq!(candidate.nodes[c.0 as usize].rect.size.width, 91.0);
        assert_eq!(candidate.position(child), None);
        assert_eq!(candidate.nodes[child.0 as usize].rect, child_geometry.rect);
        for node in [c, child, d, group_child, leaf] {
            assert_eq!(candidate.topology_node_owner(node), Some(combined));
        }
        let a = candidate
            .owner_graph_group_map(receiver, OwnerGroupKind::Cluster)
            .unwrap();
        let b = candidate
            .owner_graph_group_map(combined, OwnerGroupKind::Cluster)
            .unwrap();
        assert!(a.same_map_allocation(&b));
        assert_eq!(
            session
                .owner()
                .nodes
                .iter()
                .map(|n| session.owner().position(n.input_id))
                .collect::<Vec<_>>(),
            prior_positions
        );
        assert_eq!(
            session
                .owner()
                .nodes
                .iter()
                .map(|n| session.owner().topology_node_owner(n.input_id))
                .collect::<Vec<_>>(),
            prior_owners
        );
        // AddNodeUnchecked does not silently deduplicate across root walks.
        workspace.node_order = vec![NodeId(1), NodeId(1)];
        workspace.refresh_node_order_membership();
        let (duplicates, _) = session.combined_member_workspace(&workspace, &[]).unwrap();
        assert_eq!(
            duplicates.graph_node_order(),
            [c, child, d, group_child, c, child, d, group_child]
        );
        assert_eq!(
            session
                .combined_member_workspace(&workspace, &[(leaf, child_geometry)])
                .unwrap_err(),
            OwnerScoreSessionError::UncapturedNode(leaf)
        );
    }

    #[test]
    fn general_descendants_use_receiver_maps_and_live_group_headers_in_source_order() {
        let (mut owner, _, receiver, mapping, [c, child, d, group_child, leaf]) =
            initialization_fixture();
        owner.nodes[child.0 as usize].is_container = true;
        owner.containers.insert(Some(child), vec![c, leaf, leaf]);
        owner.set_owner_cluster_vessel_flag(c, true).unwrap();
        let mut cluster_header = None;
        for (kind, members) in [
            (
                OwnerGroupKind::Cluster,
                vec![Some(group_child), None, Some(child)],
            ),
            (OwnerGroupKind::Sequence, vec![Some(d), Some(group_child)]),
        ] {
            let group = owner.new_owner_group(kind);
            let nodes = owner
                .allocate_owner_member_header(&members, members.len())
                .unwrap();
            if kind == OwnerGroupKind::Cluster {
                cluster_header = Some(nodes.clone());
            }
            let mut fields = owner.owner_group_fields(group).unwrap();
            fields.nodes = nodes;
            owner.set_owner_group_fields(group, fields).unwrap();
            let map = owner.allocate_owner_group_map(kind, true);
            owner
                .write_owner_group_map_entry(&map, Some(c), Some(group))
                .unwrap();
            owner.set_owner_graph_group_map(receiver, map).unwrap();
        }
        // Node.Graph remains the original graph, whose group maps are empty.
        assert_ne!(owner.topology_node_owner(c), Some(receiver));
        let mut session = OwnerScoreSession::new(&mut owner, receiver, mapping, None).unwrap();
        assert_eq!(
            session.live_descendants(receiver, Some(c), true).unwrap(),
            [child, leaf, group_child, d]
        );
        assert_eq!(
            session.live_descendants(receiver, Some(c), false).unwrap(),
            [child, leaf]
        );
        assert_eq!(session.live_movement_nodes(c).unwrap(), [c, child, leaf]);
        // The actual retained group header changes without editing legacy groups.
        session
            .owner_mut()
            .write_owner_member_capacity_slot(&cluster_header.unwrap(), 0, Some(leaf))
            .unwrap();
        assert_eq!(
            session.live_descendants(receiver, Some(c), true).unwrap(),
            [child, leaf, d, group_child]
        );
        session
            .owner_mut()
            .set_owner_cluster_vessel_flag(c, false)
            .unwrap();
        // Sequence members are visited even without a sequence vessel flag.
        assert_eq!(
            session.live_descendants(receiver, Some(c), true).unwrap(),
            [child, leaf, d, group_child]
        );
        session
            .owner_mut()
            .set_owner_node_graph_reference(c, Some(receiver))
            .unwrap();
        assert_eq!(
            session.live_movement_nodes(c).unwrap(),
            [c, child, leaf, d, group_child]
        );
        let sequence = session
            .owner()
            .owner_graph_group_entry(receiver, OwnerGroupKind::Sequence, Some(c))
            .unwrap()
            .flatten()
            .unwrap();
        let sequence_header = session.owner().owner_group_fields(sequence).unwrap().nodes;
        session
            .owner_mut()
            .write_owner_member_capacity_slot(&sequence_header, 0, Some(group_child))
            .unwrap();
        assert_eq!(
            session.live_movement_nodes(c).unwrap(),
            [c, child, leaf, group_child]
        );
    }

    #[test]
    fn general_descendants_nil_root_and_noncontainer_cluster_map_follow_source_gates() {
        let (mut owner, _, receiver, mapping, [c, child, d, group_child, leaf]) =
            initialization_fixture();
        owner.containers.insert(None, vec![c, d]);
        let group = owner.new_owner_group(OwnerGroupKind::Cluster);
        let mut fields = owner.owner_group_fields(group).unwrap();
        fields.nodes = owner
            .allocate_owner_member_header(&[Some(leaf), None], 2)
            .unwrap();
        owner.set_owner_group_fields(group, fields).unwrap();
        let map = owner.allocate_owner_group_map(OwnerGroupKind::Cluster, true);
        owner
            .write_owner_group_map_entry(&map, Some(child), Some(group))
            .unwrap();
        owner.set_owner_graph_group_map(receiver, map).unwrap();
        // The keyed cluster exists, but a non-vessel must not expand it.
        let mut session = OwnerScoreSession::new(&mut owner, receiver, mapping, None).unwrap();
        assert_eq!(
            session.live_descendants(receiver, None, true).unwrap(),
            [c, child, d, group_child]
        );
        session
            .owner_mut()
            .set_owner_cluster_vessel_flag(child, true)
            .unwrap();
        assert_eq!(
            session.live_descendants(receiver, None, true).unwrap(),
            [c, child, leaf, d, group_child]
        );
        assert_eq!(
            session
                .live_descendants(receiver, Some(child), false)
                .unwrap(),
            Vec::<NodeId>::new()
        );
    }

    #[test]
    fn initialization_reads_mutated_live_header_and_deduplicates_descendant_cycles() {
        let (mut owner, workspace, view, mapping, [c, child_c, _, child_d, _]) =
            initialization_fixture();
        let header = owner
            .owner_container_map(view)
            .unwrap()
            .get(Some(c))
            .unwrap();
        owner
            .containers
            .get_header(&Some(c))
            .unwrap()
            .write_capacity_slot(0, Some(child_d))
            .unwrap();
        let untouched = owner.position(child_c);
        let moved_before = owner.position(child_d);
        // The movement walk sees a repeated root through a retained cycle.
        owner.nodes[child_d.0 as usize].is_container = true;
        owner
            .containers
            .insert(Some(child_d), vec![child_d, child_c, child_c]);
        let mut session = OwnerScoreSession::new(&mut owner, view, mapping, None).unwrap();
        assert_eq!(
            session.live_movement_nodes(child_d).unwrap(),
            [child_d, child_c]
        );
        session
            .initialize_current_member_positions(&workspace)
            .unwrap();
        assert_ne!(session.owner().position(child_d), moved_before);
        assert_ne!(session.owner().position(child_c), untouched);
        assert_eq!(header.get(0), Some(Some(child_d)));
    }

    #[test]
    fn nested_binding_error_does_not_undo_completed_member_initialization() {
        let (mut owner, workspace, view, mapping, [c, child_c, d, child_d, leaf]) =
            initialization_fixture();
        owner.nodes[child_c.0 as usize].position = None;
        let foreign_before = [owner.position(d), owner.position(child_d)];
        let mut session = OwnerScoreSession::new(&mut owner, view, mapping, None).unwrap();
        assert_eq!(
            session.initialize_current_member_positions(&workspace),
            Err(OwnerScoreSessionError::UnpositionedNode(child_c))
        );
        assert_eq!(session.owner().position(c), workspace.position(c));
        assert_eq!(session.owner().position(leaf), workspace.position(leaf));
        assert_eq!(session.owner().position(child_c), None);
        assert_eq!(
            [
                session.owner().position(d),
                session.owner().position(child_d)
            ],
            foreign_before
        );
    }

    #[test]
    fn initialization_obeys_outer_geometry_trial_rollback_and_exact_member_order() {
        let (mut owner, mut workspace, view, mapping, _) = initialization_fixture();
        let before = owner
            .nodes
            .iter()
            .map(OwnerNodeGeometry::from_node)
            .collect::<Vec<_>>();
        let mut session = OwnerScoreSession::new(&mut owner, view, mapping, None).unwrap();
        let trial = session.begin_trial().unwrap();
        session
            .initialize_current_member_positions(&workspace)
            .unwrap();
        session.rollback_trial(trial).unwrap();
        assert_eq!(
            session
                .owner()
                .nodes
                .iter()
                .map(OwnerNodeGeometry::from_node)
                .collect::<Vec<_>>(),
            before
        );
        workspace.node_order.reverse();
        workspace.refresh_node_order_membership();
        assert_eq!(
            session.initialize_current_member_positions(&workspace),
            Err(OwnerScoreSessionError::MemberMappingMismatch)
        );
        assert_eq!(
            session
                .owner()
                .nodes
                .iter()
                .map(OwnerNodeGeometry::from_node)
                .collect::<Vec<_>>(),
            before
        );
    }

    #[test]
    fn successful_reborrowed_initialization_survives_later_scope_error_or_panic() {
        for panic_after_child in [false, true] {
            let (mut owner, workspace, view, mapping, _) = initialization_fixture();
            let before = owner
                .nodes
                .iter()
                .map(OwnerNodeGeometry::from_node)
                .collect::<Vec<_>>();
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut parent = OwnerScoreSession::new(&mut owner, view, mapping, None).unwrap();
                {
                    let mut child = parent.reborrow();
                    child
                        .initialize_current_member_positions(&workspace)
                        .unwrap();
                    child.commit_scope_ownership().unwrap();
                }
                assert_ne!(
                    parent
                        .owner()
                        .nodes
                        .iter()
                        .map(OwnerNodeGeometry::from_node)
                        .collect::<Vec<_>>(),
                    before
                );
                if panic_after_child {
                    panic!("later enclosing placement failure");
                }
                // Leaving the parent uncommitted represents a returned error.
            }));
            assert_eq!(result.is_err(), panic_after_child);
            assert_ne!(
                owner
                    .nodes
                    .iter()
                    .map(OwnerNodeGeometry::from_node)
                    .collect::<Vec<_>>(),
                before
            );
        }
    }

    struct MovementContext;
    impl limits::Context for MovementContext {
        fn done(&self) -> Option<bool> {
            None
        }
        fn error(&self) -> Option<limits::ContextError> {
            None
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum CaptureSlotMutation {
        FutureSlot,
        LoadedSlot,
        ReplaceHeader,
    }

    struct CaptureLiveSlotContext {
        polls: std::cell::Cell<usize>,
        mutation: CaptureSlotMutation,
        root: NodeId,
        foreign: NodeId,
        retained: SharedNodeSlice,
        map: SharedSliceMap,
        replacement: Option<SharedNodeSlice>,
    }

    impl limits::Context for CaptureLiveSlotContext {
        fn done(&self) -> Option<bool> {
            None
        }

        fn error(&self) -> Option<limits::ContextError> {
            self.polls.set(self.polls.get() + 1);
            if self.polls.get() == 2 {
                match self.mutation {
                    CaptureSlotMutation::FutureSlot => self
                        .retained
                        .write_capacity_slot(0, Some(self.foreign))
                        .unwrap(),
                    CaptureSlotMutation::LoadedSlot => {
                        self.retained.write_capacity_slot(1, None).unwrap();
                    }
                    CaptureSlotMutation::ReplaceHeader => {
                        self.map
                            .replace(Some(self.root), self.replacement.as_ref().unwrap().clone())
                            .unwrap();
                    }
                }
            }
            None
        }
    }

    fn assert_optimizer_capture_live_slot_contract(mutation: CaptureSlotMutation) {
        let (mut owner, [c, child, d, foreign_child, leaf]) = fixture();
        let retained = SharedNodeSlice::from_nodes(&[child; 65]).unwrap();
        owner.containers.replace_header(Some(c), retained.clone());
        let map = owner.containers.share_map();
        let map_address = map.map_address().unwrap();
        let retained_address = retained.data_address().unwrap();
        let replacement = (mutation == CaptureSlotMutation::ReplaceHeader)
            .then(|| SharedNodeSlice::from_nodes(&[d; 65]).unwrap());
        if let Some(replacement) = &replacement {
            assert_ne!(replacement.data_address(), retained.data_address());
        }
        let before = owner
            .nodes
            .iter()
            .map(OwnerNodeGeometry::from_node)
            .collect::<Vec<_>>();
        let context = CaptureLiveSlotContext {
            polls: std::cell::Cell::new(0),
            mutation,
            root: c,
            foreign: d,
            retained: retained.clone(),
            map: map.clone(),
            replacement,
        };
        let canonical = mapping(&owner);
        let mut guard =
            OptimizationWorkGuard::new(Some(&context), "sizelessOptimizer.optimize", 1000).unwrap();
        let mut session =
            OwnerScoreSession::new(&mut owner, OwnerGraphId(0), canonical, None).unwrap();
        let mut movement = session.optimizer_candidate_movement(c, &mut guard).unwrap();
        // Source live-slot controls: root header65 refs; at accepted Step64,
        // poll2 mutates index0, already-loaded index1, or the map header.
        let (captured, descendant_work, capture_work, final_work) = match mutation {
            CaptureSlotMutation::FutureSlot => (vec![c, d, foreign_child, child], 69, 73, 148),
            CaptureSlotMutation::LoadedSlot | CaptureSlotMutation::ReplaceHeader => {
                (vec![c, child], 66, 68, 136)
            }
        };
        assert_eq!(movement.captured_nodes().collect::<Vec<_>>(), captured);
        assert_eq!(movement.descendant_work(), descendant_work);
        assert_eq!(guard.used(), capture_work);
        assert_eq!(context.polls.get(), 2);
        assert_eq!(retained.len(), 65);
        assert_eq!(retained.capacity(), 65);
        assert_eq!(retained.data_address(), Some(retained_address));
        assert_eq!(map.map_address(), Some(map_address));
        assert!(map.same_map_allocation(&movement.session().owner().containers.share_map()));

        movement
            .move_abs(Point { x: 90.0, y: 95.0 }, &mut guard)
            .unwrap();
        assert_eq!(guard.used(), final_work);
        assert_eq!(context.polls.get(), 3);
        assert_eq!(
            movement.session().owner().position(c),
            Some(Point { x: 90.0, y: 95.0 })
        );
        assert_eq!(
            movement.session().owner().position(child),
            Some(Point { x: 110.0, y: 115.0 })
        );
        let expected_foreign = if mutation == CaptureSlotMutation::FutureSlot {
            [
                Some(Point { x: 310.0, y: 95.0 }),
                Some(Point { x: 330.0, y: 115.0 }),
            ]
        } else {
            [
                before[d.0 as usize].position,
                before[foreign_child.0 as usize].position,
            ]
        };
        assert_eq!(
            [
                movement.session().owner().position(d),
                movement.session().owner().position(foreign_child)
            ],
            expected_foreign
        );
        assert_eq!(
            movement.session().owner().position(leaf),
            before[leaf.0 as usize].position
        );
        // Positional cleanup must not undo the Context's actual header edit.
        drop(movement);
        assert_eq!(
            session
                .owner()
                .nodes
                .iter()
                .map(OwnerNodeGeometry::from_node)
                .collect::<Vec<_>>(),
            before
        );
        let current = map.get(&Some(c)).unwrap();
        assert_eq!(current.len(), 65);
        assert_eq!(current.capacity(), 65);
        assert_eq!(map.map_address(), Some(map_address));
        assert!(map.same_map_allocation(&session.owner().containers.share_map()));
        assert_eq!(retained.data_address(), Some(retained_address));
        match mutation {
            CaptureSlotMutation::FutureSlot => {
                assert!(current.same_backing_allocation(&retained));
                assert_eq!(current.data_address(), Some(retained_address));
                assert_eq!(retained.get(0), Some(Some(d)));
                assert_eq!(retained.get(1), Some(Some(child)));
                assert!(current.iter_slots().skip(1).all(|node| node == Some(child)));
            }
            CaptureSlotMutation::LoadedSlot => {
                assert!(current.same_backing_allocation(&retained));
                assert_eq!(current.data_address(), Some(retained_address));
                assert_eq!(current.get(0), Some(Some(child)));
                assert_eq!(current.get(1), Some(None));
                assert!(current.iter_slots().skip(2).all(|node| node == Some(child)));
            }
            CaptureSlotMutation::ReplaceHeader => {
                let replacement = context.replacement.as_ref().unwrap();
                assert!(!current.same_backing_allocation(&retained));
                assert!(current.same_backing_allocation(replacement));
                assert_ne!(current.data_address(), Some(retained_address));
                assert_eq!(current.data_address(), replacement.data_address());
                assert!(current.iter_slots().all(|node| node == Some(d)));
                assert!(retained.iter_slots().all(|node| node == Some(child)));
            }
        }
        assert_eq!(guard.used(), final_work); // Drop adds neither work nor polls.
        assert_eq!(context.polls.get(), 3);
    }

    fn optimizer_state_fixture() -> (ArenaGraph, [NodeId; 5], OwnerGraphId, CanonicalScoreMapping) {
        let (mut owner, nodes @ [c, _, _, _, leaf]) = fixture();
        let canonical = mapping(&owner);
        let receiver = owner
            .create_owner_graph_view(vec![c, leaf], owner.edge_order.read())
            .unwrap();
        owner
            .set_owner_view_node_mapping(receiver, canonical.nodes().to_vec())
            .unwrap();
        (owner, nodes, receiver, canonical)
    }

    #[test]
    fn optimizer_state_snapshot_uses_selected_receiver_and_restores_original_cache_alias() {
        let (mut owner, [c, child, d, foreign_child, leaf], receiver, canonical) =
            optimizer_state_fixture();
        let selected = owner.owner_graph_cost_cache(receiver).unwrap().clone();
        let foreign = owner.cost_cache.clone();
        for key in 1..=3 {
            selected.store_edge_length(key, key as f64 * 10.0);
        }
        selected.restore_routing((30.0, 40.0, 50.0));
        foreign.store_edge_length(55, 550.0);
        foreign.restore_routing((300.0, 400.0, 500.0));
        let original_alias = selected.edge_length_cache_alias().unwrap();
        let before = owner
            .nodes
            .iter()
            .map(OwnerNodeGeometry::from_node)
            .collect::<Vec<_>>();
        let mut snapshot = OwnerOptimizerMutationSnapshot::default();
        let context = MovementContext;
        let mut guard =
            OptimizationWorkGuard::new(Some(&context), "sizelessOptimizer.optimize", 1000).unwrap();
        {
            let mut session =
                OwnerScoreSession::new(&mut owner, receiver, canonical, None).unwrap();
            // All Node.Graph pointers still identify the foreign original Graph.
            assert_eq!(
                session.owner().topology_node_owner(c),
                Some(OwnerGraphId(0))
            );
            session
                .capture_optimizer_mutation_state_into(&mut guard, &mut snapshot)
                .unwrap();
        }
        assert!(snapshot.is_complete());
        assert_eq!(snapshot.receiver(), Some(receiver));
        assert_eq!(
            snapshot.captured_nodes().collect::<Vec<_>>(),
            [c, leaf, child]
        );
        assert_eq!(guard.used(), 9); // selected cache3 + roots2 + child1 + nil-herds3
        owner.set_position(c, Point { x: 9.0, y: 10.0 });
        owner.set_position(child, Point { x: 11.0, y: 12.0 });
        owner.nodes[c.0 as usize].rect.size.width = 777.0;
        owner.nodes[child.0 as usize].rect.size.height = 888.0;
        owner.nodes[c.0 as usize].fixed_top_left = Some(Point { x: 17.0, y: 19.0 });
        owner.set_position(d, Point { x: 600.0, y: 700.0 });
        owner.set_position(foreign_child, Point { x: 620.0, y: 720.0 });
        selected.publish_deserialized_routing((3.0, 4.0, 5.0));
        selected.store_edge_length(999, 999.0);
        original_alias.store(99, 99.0);
        assert!(!original_alias.is_current());
        snapshot.restore(&mut owner).unwrap();
        for node in [c, leaf, child] {
            assert_eq!(
                OwnerNodeGeometry::from_node(&owner.nodes[node.0 as usize]),
                before[node.0 as usize]
            );
        }
        assert_eq!(owner.position(d), Some(Point { x: 600.0, y: 700.0 }));
        assert_eq!(
            owner.position(foreign_child),
            Some(Point { x: 620.0, y: 720.0 })
        );
        assert_eq!(
            owner.nodes[c.0 as usize].fixed_top_left,
            Some(Point { x: 17.0, y: 19.0 })
        );
        assert_eq!(selected.raw(), (30.0, 40.0, 50.0));
        assert_eq!(selected.edge_length_entries(), 3);
        assert_eq!(selected.lookup_edge_length(99), None);
        assert_eq!(selected.lookup_edge_length(999), None);
        assert!(original_alias.is_current());
        original_alias.store(123, 1230.0);
        assert_eq!(selected.lookup_edge_length(123), Some(1230.0));
        original_alias.remove(123);
        assert_eq!(foreign.lookup_edge_length(55), Some(550.0));
        assert_eq!(foreign.raw(), (300.0, 400.0, 500.0));
        assert_eq!(guard.used(), 9); // explicit restore adds no guard work.
    }

    #[test]
    fn optimizer_state_direct_capture_accepts_duplicate_members_without_score_mapping() {
        let (mut owner, [c, child, d, foreign_child, leaf]) = fixture();
        let receiver = owner
            .create_owner_graph_view(vec![c, leaf], owner.edge_order.read())
            .unwrap();
        let edges = owner.owner_graph_view(receiver).unwrap().edges.read();
        owner
            .set_owner_graph_members(receiver, vec![c, leaf, c], edges.clone())
            .unwrap();
        let canonical =
            CanonicalScoreMapping::new(owner.topology_arena_id().unwrap(), vec![c, leaf, c], edges);
        owner.containers.insert(Some(c), vec![child, child]);
        owner.containers.insert(Some(child), vec![c]);
        // Actual Graph.Nodes may repeat a member without replacing the
        // pre-existing dense correspondence. Mirror the failed native hook's
        // literal member-slice mapping; capture does not require that frame.
        assert_eq!(
            owner.owner_graph_view(receiver).unwrap().local_to_canonical,
            [c, leaf]
        );
        assert_eq!(
            owner
                .owner_graph_view(receiver)
                .unwrap()
                .nodes
                .require_nodes()
                .collect::<Vec<_>>(),
            [c, leaf, c]
        );
        assert!(matches!(
            OwnerScoreSession::new(&mut owner, receiver, canonical, None),
            Err(OwnerScoreSessionError::Frame(OwnerScoreError::MappingMismatch(view)))
                if view == receiver
        ));
        let selected = owner.owner_graph_cost_cache(receiver).unwrap().clone();
        let foreign = owner.cost_cache.clone();
        selected.restore_routing((3.0, 4.0, 5.0));
        selected.store_edge_length(91, 9.1);
        selected.store_edge_length(92, 9.2);
        let original_alias = selected.edge_length_cache_alias().unwrap();
        foreign.restore_routing((30.0, 40.0, 50.0));
        foreign.store_edge_length(999, 99.0);
        let before = owner
            .nodes
            .iter()
            .map(OwnerNodeGeometry::from_node)
            .collect::<Vec<_>>();
        let mut snapshot = OwnerOptimizerMutationSnapshot::default();
        let context = MovementContext;
        let mut guard =
            OptimizationWorkGuard::new(Some(&context), "sizelessOptimizer.optimize", 1000).unwrap();
        snapshot
            .capture_into(&mut owner, receiver, &mut guard)
            .unwrap();
        assert!(snapshot.is_complete());
        assert_eq!(snapshot.receiver(), Some(receiver));
        assert_eq!(
            snapshot.captured_nodes().collect::<Vec<_>>(),
            [c, leaf, child]
        );
        // Selected cache2 + roots3 + child refs3 + nil-herds3; duplicate
        // references and the back edge charge before FIFO discovery dedup.
        assert_eq!(guard.used(), 11);
        assert_eq!(owner.topology_node_owner(c), Some(OwnerGraphId(0)));
        for node in [c, leaf, child] {
            owner.set_position(node, Point { x: 777.0, y: 888.0 });
            owner.nodes[node.0 as usize].rect.size.width = 333.0;
            owner.nodes[node.0 as usize].rect.size.height = 444.0;
        }
        owner.set_position(d, Point { x: 600.0, y: 700.0 });
        owner.set_position(foreign_child, Point { x: 620.0, y: 720.0 });
        let unrelated = [
            OwnerNodeGeometry::from_node(&owner.nodes[d.0 as usize]),
            OwnerNodeGeometry::from_node(&owner.nodes[foreign_child.0 as usize]),
        ];
        selected.publish_deserialized_routing((300.0, 400.0, 500.0));
        selected.store_edge_length(777, 77.0);
        original_alias.store(888, 88.0);
        assert!(!original_alias.is_current());
        snapshot.restore(&mut owner).unwrap();
        for node in [c, leaf, child] {
            assert_eq!(
                OwnerNodeGeometry::from_node(&owner.nodes[node.0 as usize]),
                before[node.0 as usize]
            );
        }
        assert_eq!(
            [
                OwnerNodeGeometry::from_node(&owner.nodes[d.0 as usize]),
                OwnerNodeGeometry::from_node(&owner.nodes[foreign_child.0 as usize])
            ],
            unrelated
        );
        assert_eq!(selected.raw(), (3.0, 4.0, 5.0));
        assert!(original_alias.is_current());
        assert_eq!(original_alias.entries().len(), 2);
        assert_eq!(selected.lookup_edge_length(91), Some(9.1));
        assert_eq!(selected.lookup_edge_length(92), Some(9.2));
        assert_eq!(selected.lookup_edge_length(777), None);
        assert_eq!(selected.lookup_edge_length(888), None);
        assert_eq!(foreign.raw(), (30.0, 40.0, 50.0));
        assert_eq!(foreign.lookup_edge_length(999), Some(99.0));
        assert_eq!(
            owner
                .owner_graph_view(receiver)
                .unwrap()
                .nodes
                .require_nodes()
                .collect::<Vec<_>>(),
            [c, leaf, c]
        );
        assert_eq!(guard.used(), 11); // Restore is uncharged.
        snapshot.release();
        assert!(!snapshot.is_complete());
        assert_eq!(snapshot.captured_nodes().len(), 0);
    }

    #[test]
    fn optimizer_state_snapshot_includes_noncontainer_keys_and_charges_duplicate_cycles() {
        let (mut owner, [c, child, d, foreign_child, leaf], receiver, canonical) =
            optimizer_state_fixture();
        owner.containers.insert(Some(c), vec![child, child]);
        owner.containers.insert(Some(child), vec![c]);
        owner.containers.insert(Some(leaf), vec![d]);
        assert!(!owner.nodes[leaf.0 as usize].is_container);
        owner
            .set_owner_graph_members(receiver, vec![c, leaf, c], owner.edge_order.read())
            .unwrap();
        let mut session = OwnerScoreSession::new(&mut owner, receiver, canonical, None).unwrap();
        let mut snapshot = OwnerOptimizerMutationSnapshot::default();
        let context = MovementContext;
        let mut guard =
            OptimizationWorkGuard::new(Some(&context), "sizelessOptimizer.optimize", 1000).unwrap();
        session
            .capture_optimizer_mutation_state_into(&mut guard, &mut snapshot)
            .unwrap();
        assert_eq!(
            snapshot.captured_nodes().collect::<Vec<_>>(),
            [c, leaf, child, d, foreign_child]
        );
        assert_eq!(guard.used(), 13); // roots3 + child refs5 + herd scan5; no BFS pop Steps.
    }

    #[test]
    fn optimizer_state_snapshot_restores_logical_nil_sizes_and_original_nil_cache() {
        let (mut owner, [_, child, _, _, _], receiver, canonical) = optimizer_state_fixture();
        let selected = owner.owner_graph_cost_cache(receiver).unwrap().clone();
        selected.disable_edge_length_cache();
        selected.restore_routing((-0.0, 7.0, 9.0));
        let width_bits = 0x7ff8_0000_0000_0042;
        owner.nodes[child.0 as usize].position = None;
        owner.nodes[child.0 as usize].rect.size.width = f64::from_bits(width_bits);
        owner.nodes[child.0 as usize].rect.size.height = -0.0;
        let mut snapshot = OwnerOptimizerMutationSnapshot::default();
        let context = MovementContext;
        let mut guard =
            OptimizationWorkGuard::new(Some(&context), "sizelessOptimizer.optimize", 1000).unwrap();
        {
            let mut session =
                OwnerScoreSession::new(&mut owner, receiver, canonical, None).unwrap();
            session
                .capture_optimizer_mutation_state_into(&mut guard, &mut snapshot)
                .unwrap();
        }
        owner.set_position(child, Point { x: 777.0, y: 888.0 });
        owner.nodes[child.0 as usize].rect.size = Size {
            width: 3.0,
            height: 4.0,
        };
        selected.publish_deserialized_routing((70.0, 80.0, 90.0));
        selected.store_edge_length(1, 100.0);
        snapshot.restore(&mut owner).unwrap();
        assert_eq!(owner.position(child), None);
        assert_eq!(
            owner.nodes[child.0 as usize].rect.size.width.to_bits(),
            width_bits
        );
        assert_eq!(
            owner.nodes[child.0 as usize].rect.size.height.to_bits(),
            (-0.0_f64).to_bits()
        );
        assert!(!selected.edge_length_cache_enabled());
        assert_eq!(selected.raw().0.to_bits(), (-0.0_f64).to_bits());
        assert_eq!((selected.raw().1, selected.raw().2), (7.0, 9.0));
        assert_eq!(guard.used(), 6);
    }

    struct PassSnapshotSlotContext {
        polls: std::cell::Cell<usize>,
        mutation: CaptureSlotMutation,
        root: NodeId,
        foreign: NodeId,
        retained: SharedNodeSlice,
        map: SharedSliceMap,
        replacement: Option<SharedNodeSlice>,
    }
    impl limits::Context for PassSnapshotSlotContext {
        fn done(&self) -> Option<bool> {
            None
        }
        fn error(&self) -> Option<limits::ContextError> {
            self.polls.set(self.polls.get() + 1);
            if self.polls.get() == 3 {
                match self.mutation {
                    CaptureSlotMutation::FutureSlot => self
                        .retained
                        .write_capacity_slot(62, Some(self.foreign))
                        .unwrap(),
                    CaptureSlotMutation::LoadedSlot => {
                        self.retained.write_capacity_slot(61, None).unwrap()
                    }
                    CaptureSlotMutation::ReplaceHeader => {
                        self.map
                            .replace(Some(self.root), self.replacement.as_ref().unwrap().clone())
                            .unwrap();
                    }
                }
            }
            None
        }
    }
    fn assert_pass_snapshot_live_slots(mutation: CaptureSlotMutation) {
        let (mut owner, [c, child, d, foreign_child, leaf], receiver, canonical) =
            optimizer_state_fixture();
        let retained = SharedNodeSlice::from_nodes(&[child; 65]).unwrap();
        owner.containers.replace_header(Some(c), retained.clone());
        let map = owner.containers.share_map();
        let map_address = map.map_address();
        let retained_address = retained.data_address();
        let context = PassSnapshotSlotContext {
            polls: std::cell::Cell::new(0),
            mutation,
            root: c,
            foreign: d,
            retained: retained.clone(),
            map: map.clone(),
            replacement: (mutation == CaptureSlotMutation::ReplaceHeader)
                .then(|| SharedNodeSlice::from_nodes(&[d; 65]).unwrap()),
        };
        let mut snapshot = OwnerOptimizerMutationSnapshot::default();
        let mut guard =
            OptimizationWorkGuard::new(Some(&context), "sizelessOptimizer.optimize", 1000).unwrap();
        {
            let mut session =
                OwnerScoreSession::new(&mut owner, receiver, canonical, None).unwrap();
            session
                .capture_optimizer_mutation_state_into(&mut guard, &mut snapshot)
                .unwrap();
        }
        let expected = if mutation == CaptureSlotMutation::FutureSlot {
            vec![c, leaf, child, d, foreign_child]
        } else {
            vec![c, leaf, child]
        };
        assert_eq!(snapshot.captured_nodes().collect::<Vec<_>>(), expected);
        let work = if mutation == CaptureSlotMutation::FutureSlot {
            73
        } else {
            70
        };
        assert_eq!(guard.used(), work);
        assert_eq!(context.polls.get(), 3); // NewGuard, cost Finish, actual accepted Step64.
        let saved = snapshot.captured_geometry().collect::<Vec<_>>();
        for &(node, _) in &saved {
            owner.set_position(
                node,
                Point {
                    x: 9000.0,
                    y: -9000.0,
                },
            );
            owner.nodes[node.0 as usize].rect.size.width = 900.0;
        }
        snapshot.restore(&mut owner).unwrap();
        for &(node, geometry) in &saved {
            assert_eq!(
                OwnerNodeGeometry::from_node(&owner.nodes[node.0 as usize]),
                geometry
            );
        }
        let current = map.get(&Some(c)).unwrap();
        assert_eq!(current.len(), 65);
        assert_eq!(current.capacity(), 65);
        assert_eq!(map.map_address(), map_address);
        assert_eq!(retained.data_address(), retained_address);
        match mutation {
            CaptureSlotMutation::FutureSlot => {
                assert!(current.same_backing_allocation(&retained));
                assert_eq!(retained.get(62), Some(Some(d)));
            }
            CaptureSlotMutation::LoadedSlot => {
                assert!(current.same_backing_allocation(&retained));
                assert_eq!(retained.get(61), Some(None));
            }
            CaptureSlotMutation::ReplaceHeader => {
                assert!(current.same_backing_allocation(context.replacement.as_ref().unwrap()));
                assert!(!current.same_backing_allocation(&retained));
                assert!(retained.iter_slots().all(|node| node == Some(child)));
                assert!(current.iter_slots().all(|node| node == Some(d)));
            }
        }
        assert_eq!(guard.used(), work);
        assert_eq!(context.polls.get(), 3);
    }
    #[test]
    fn optimizer_state_snapshot_reads_future_forward_slot_changed_by_poll() {
        assert_pass_snapshot_live_slots(CaptureSlotMutation::FutureSlot);
    }
    #[test]
    fn optimizer_state_snapshot_keeps_current_forward_slot_loaded_before_poll() {
        assert_pass_snapshot_live_slots(CaptureSlotMutation::LoadedSlot);
    }
    #[test]
    fn optimizer_state_snapshot_keeps_retained_forward_header_after_replacement() {
        assert_pass_snapshot_live_slots(CaptureSlotMutation::ReplaceHeader);
    }

    #[derive(Clone, Copy)]
    enum PassCachePollAction {
        CancelCharge,
        CancelAfterCopy,
        ChangeDuringChargeAndFinish,
    }
    struct PassCachePollContext {
        polls: std::cell::Cell<usize>,
        action: PassCachePollAction,
        cache: std::sync::Arc<GraphCostCache>,
        retained: SharedNodeSlice,
        foreign: NodeId,
    }
    impl limits::Context for PassCachePollContext {
        fn done(&self) -> Option<bool> {
            None
        }
        fn error(&self) -> Option<limits::ContextError> {
            self.polls.set(self.polls.get() + 1);
            if self.polls.get() == 2 {
                self.cache.store_edge_length(999, 44.0);
                self.cache.restore_routing((10.0, 20.0, 30.0));
                if !matches!(
                    self.action,
                    PassCachePollAction::ChangeDuringChargeAndFinish
                ) {
                    self.retained
                        .write_capacity_slot(0, Some(self.foreign))
                        .unwrap();
                    return Some(limits::ContextError::Cancelled);
                }
            } else if self.polls.get() == 3
                && matches!(
                    self.action,
                    PassCachePollAction::ChangeDuringChargeAndFinish
                )
            {
                self.cache.store_edge_length(999, 77.0);
                self.cache.restore_routing((70.0, 80.0, 90.0));
            }
            None
        }
    }
    #[test]
    fn optimizer_state_capture_failure_is_inert_during_charge_and_after_cost_copy() {
        for (action, entries, expected_work) in [
            (PassCachePollAction::CancelCharge, 65, 64),
            (PassCachePollAction::CancelAfterCopy, 1, 1),
        ] {
            let (mut owner, [c, child, d, _, _], receiver, canonical) = optimizer_state_fixture();
            let cache = owner.owner_graph_cost_cache(receiver).unwrap().clone();
            for key in 0..entries {
                cache.store_edge_length(key, key as f64);
            }
            let retained = owner.containers.get_header(&Some(c)).unwrap();
            assert_eq!(retained.get(0), Some(Some(child)));
            let context = PassCachePollContext {
                polls: std::cell::Cell::new(0),
                action,
                cache: cache.clone(),
                retained: retained.clone(),
                foreign: d,
            };
            let mut snapshot = OwnerOptimizerMutationSnapshot::default();
            let mut guard =
                OptimizationWorkGuard::new(Some(&context), "sizelessOptimizer.optimize", 1000)
                    .unwrap();
            {
                let mut session =
                    OwnerScoreSession::new(&mut owner, receiver, canonical, None).unwrap();
                assert_eq!(
                    session.capture_optimizer_mutation_state_into(&mut guard, &mut snapshot),
                    Err(OptimizationError::Context {
                        location: "sizelessOptimizer.optimize".into(),
                        cause: limits::ContextError::Cancelled
                    })
                );
            }
            assert!(!snapshot.is_complete());
            assert!(snapshot.cost_snapshot.is_none()); // attach only after successful Finish.
            assert_eq!(snapshot.captured_nodes().len(), 0);
            assert_eq!(guard.used(), expected_work);
            assert_eq!(context.polls.get(), 2);
            snapshot.restore(&mut owner).unwrap(); // returned nil snapshot is inert.
            snapshot.release();
            assert_eq!(retained.get(0), Some(Some(d)));
            assert_eq!(cache.lookup_edge_length(999), Some(44.0));
            assert_eq!(cache.raw(), (10.0, 20.0, 30.0));
            assert_eq!(guard.used(), expected_work);
            assert_eq!(context.polls.get(), 2);
        }
    }
    #[test]
    fn optimizer_state_cost_count_precedes_live_copy_and_copy_precedes_finish() {
        let (mut owner, [c, _, d, _, _], receiver, canonical) = optimizer_state_fixture();
        let cache = owner.owner_graph_cost_cache(receiver).unwrap().clone();
        for key in 0..65 {
            cache.store_edge_length(key, key as f64);
        }
        let context = PassCachePollContext {
            polls: std::cell::Cell::new(0),
            action: PassCachePollAction::ChangeDuringChargeAndFinish,
            cache: cache.clone(),
            retained: owner.containers.get_header(&Some(c)).unwrap(),
            foreign: d,
        };
        let mut snapshot = OwnerOptimizerMutationSnapshot::default();
        let mut guard =
            OptimizationWorkGuard::new(Some(&context), "sizelessOptimizer.optimize", 1000).unwrap();
        {
            let mut session =
                OwnerScoreSession::new(&mut owner, receiver, canonical, None).unwrap();
            session
                .capture_optimizer_mutation_state_into(&mut guard, &mut snapshot)
                .unwrap();
        }
        assert_eq!(guard.used(), 71); // initial cache65, not live copied count66, plus ordinary6.
        assert_eq!(context.polls.get(), 3);
        assert_eq!(cache.lookup_edge_length(999), Some(77.0));
        assert_eq!(cache.raw(), (70.0, 80.0, 90.0));
        snapshot.restore(&mut owner).unwrap();
        assert_eq!(cache.edge_length_entries(), 66);
        assert_eq!(cache.lookup_edge_length(999), Some(44.0));
        assert_eq!(cache.raw(), (10.0, 20.0, 30.0));
        assert_eq!(guard.used(), 71);
        assert_eq!(context.polls.get(), 3);
    }

    #[test]
    fn optimizer_state_nil_charge_precedence_and_partial_snapshot_never_restore() {
        for limit in [2, 3] {
            let (mut owner, [c, _, _, _, _], receiver, canonical) = optimizer_state_fixture();
            let header = SharedNodeSlice::with_len_cap(1, 1).unwrap();
            header.write_capacity_slot(0, None).unwrap();
            owner.containers.replace_header(Some(c), header);
            let mut snapshot = OwnerOptimizerMutationSnapshot::default();
            let context = MovementContext;
            let mut guard =
                OptimizationWorkGuard::new(Some(&context), "sizelessOptimizer.optimize", limit)
                    .unwrap();
            let err;
            {
                let mut session =
                    OwnerScoreSession::new(&mut owner, receiver, canonical, None).unwrap();
                err = session
                    .capture_optimizer_mutation_state_into(&mut guard, &mut snapshot)
                    .unwrap_err();
            }
            let expected = if limit == 2 {
                "TALA optimization resource limit exceeded: TALA sizelessOptimizer.optimize work exceeds limit 2"
            } else {
                "TALA sizelessOptimizer.optimize found a nil graph node"
            };
            assert_eq!(err.to_string(), expected);
            assert_eq!(guard.used(), limit);
            assert!(!snapshot.is_complete());
            assert_eq!(snapshot.captured_nodes().len(), 2);
            owner.set_position(c, Point { x: 777.0, y: 888.0 });
            snapshot.restore(&mut owner).unwrap();
            assert_eq!(owner.position(c), Some(Point { x: 777.0, y: 888.0 }));
        }
    }

    #[test]
    fn optimizer_state_release_reuse_and_drop_preserve_mutations_without_arming() {
        let (mut owner, [c, child, _, _, _], receiver, canonical) = optimizer_state_fixture();
        let cache = owner.owner_graph_cost_cache(receiver).unwrap().clone();
        let baseline_refs = std::sync::Arc::strong_count(&cache);
        let context = MovementContext;
        let mut snapshot = OwnerOptimizerMutationSnapshot::default();
        let mut guard =
            OptimizationWorkGuard::new(Some(&context), "sizelessOptimizer.optimize", 1000).unwrap();
        {
            let mut session =
                OwnerScoreSession::new(&mut owner, receiver, canonical.clone(), None).unwrap();
            session
                .capture_optimizer_mutation_state_into(&mut guard, &mut snapshot)
                .unwrap();
        }
        let data_address = snapshot.nodes.as_ptr();
        let capacity = snapshot.nodes.capacity();
        assert_eq!(std::sync::Arc::strong_count(&cache), baseline_refs + 1);
        owner.set_position(c, Point { x: 77.0, y: 88.0 });
        cache.store_edge_length(1, 10.0);
        snapshot.release();
        assert_eq!(std::sync::Arc::strong_count(&cache), baseline_refs);
        assert_eq!(owner.position(c), Some(Point { x: 77.0, y: 88.0 }));
        assert_eq!(cache.lookup_edge_length(1), Some(10.0));
        assert_eq!(snapshot.nodes.capacity(), capacity);
        assert_eq!(snapshot.nodes.as_ptr(), data_address);
        assert!(snapshot.seen_nodes.is_empty());
        assert_eq!(snapshot.receiver(), None);
        owner
            .set_owner_graph_members(receiver, vec![child], vec![])
            .unwrap();
        owner.set_position(child, Point { x: 55.0, y: 66.0 });
        let mut next_guard =
            OptimizationWorkGuard::new(Some(&context), "sizelessOptimizer.optimize", 1000).unwrap();
        {
            let mut session =
                OwnerScoreSession::new(&mut owner, receiver, canonical, None).unwrap();
            session
                .capture_optimizer_mutation_state_into(&mut next_guard, &mut snapshot)
                .unwrap();
        }
        assert_eq!(snapshot.captured_nodes().collect::<Vec<_>>(), [child]);
        assert_eq!(
            snapshot.captured_geometry().next().unwrap().1.position,
            Some(Point { x: 55.0, y: 66.0 })
        );
        assert_eq!(next_guard.used(), 3); // cache1 + already-seen old child root1 + herd1.
        assert_eq!(snapshot.nodes.capacity(), capacity);
        assert_eq!(snapshot.nodes.as_ptr(), data_address);
        owner.set_position(child, Point { x: 99.0, y: 100.0 });
        drop(snapshot);
        assert_eq!(owner.position(child), Some(Point { x: 99.0, y: 100.0 }));
        assert_eq!(std::sync::Arc::strong_count(&cache), baseline_refs);
    }

    #[test]
    fn optimizer_state_restore_rejects_foreign_arena_before_any_write() {
        let (mut owner, _, receiver, canonical) = optimizer_state_fixture();
        let mut snapshot = OwnerOptimizerMutationSnapshot::default();
        let context = MovementContext;
        let mut guard =
            OptimizationWorkGuard::new(Some(&context), "sizelessOptimizer.optimize", 1000).unwrap();
        {
            let mut session =
                OwnerScoreSession::new(&mut owner, receiver, canonical, None).unwrap();
            session
                .capture_optimizer_mutation_state_into(&mut guard, &mut snapshot)
                .unwrap();
        }
        let (mut foreign, [foreign_c, _, _, _, _]) = fixture();
        foreign.set_position(
            foreign_c,
            Point {
                x: 999.0,
                y: 1000.0,
            },
        );
        let before = foreign
            .nodes
            .iter()
            .map(OwnerNodeGeometry::from_node)
            .collect::<Vec<_>>();
        assert!(
            matches!(snapshot.restore(&mut foreign), Err(OptimizationError::Invalid(message)) if message.contains("ForeignArena"))
        );
        assert_eq!(
            foreign
                .nodes
                .iter()
                .map(OwnerNodeGeometry::from_node)
                .collect::<Vec<_>>(),
            before
        );
    }

    #[test]
    fn optimizer_state_snapshot_rejects_unbound_herds_and_uses_selected_group_maps() {
        let (mut owner, [c, child, _, _, _], receiver, canonical) = optimizer_state_fixture();
        let context = MovementContext;
        owner.nodes[child.0 as usize].herd_assignment = Some(HerdAssignment {
            orientation: Orientation::Top,
            value: 10.0,
            same_side_paired: BTreeSet::new(),
            opposite_side_paired: BTreeSet::new(),
        });
        let mut snapshot = OwnerOptimizerMutationSnapshot::default();
        let mut guard =
            OptimizationWorkGuard::new(Some(&context), "sizelessOptimizer.optimize", 1000).unwrap();
        {
            let mut session =
                OwnerScoreSession::new(&mut owner, receiver, canonical.clone(), None).unwrap();
            let error = session
                .capture_optimizer_mutation_state_into(&mut guard, &mut snapshot)
                .unwrap_err();
            assert!(error.to_string().contains("unsupported herd bindings"));
        }
        assert_eq!(guard.used(), 6); // same source herd Step before binding guard.
        assert!(!snapshot.is_complete());
        owner.nodes[child.0 as usize].herd_assignment = None;
        owner.clusters.push(ClusterState {
            members: vec![child],
            arrangement: ClusterArrangement::Row,
            desired_arrangement: ClusterArrangement::Row,
            padding: 10.0,
            vessel_tala_id: owner.nodes[c.0 as usize].tala_id,
            fixed_size: false,
        });
        let mut next_guard =
            OptimizationWorkGuard::new(Some(&context), "sizelessOptimizer.optimize", 1000).unwrap();
        let mut session = OwnerScoreSession::new(&mut owner, receiver, canonical, None).unwrap();
        session
            .capture_optimizer_mutation_state_into(&mut next_guard, &mut snapshot)
            .unwrap();
        assert_eq!(next_guard.used(), 6);
        assert!(snapshot.is_complete());
        assert_eq!(snapshot.captured_nodes().len(), 3);
    }

    #[test]
    fn optimizer_candidate_capture_reads_future_slot_changed_by_real_poll() {
        assert_optimizer_capture_live_slot_contract(CaptureSlotMutation::FutureSlot);
    }

    #[test]
    fn optimizer_candidate_capture_keeps_already_loaded_slot_when_poll_makes_it_nil() {
        assert_optimizer_capture_live_slot_contract(CaptureSlotMutation::LoadedSlot);
    }

    #[test]
    fn optimizer_candidate_capture_keeps_retained_header_after_poll_replaces_map_entry() {
        assert_optimizer_capture_live_slot_contract(CaptureSlotMutation::ReplaceHeader);
    }

    #[test]
    fn optimizer_candidate_moves_actual_retained_subtree_and_commits_without_refit() {
        let (mut owner, _, view, mapping, [c, child, d, foreign, _]) = initialization_fixture();
        let root_before = owner.position(c).unwrap();
        let child_before = owner.position(child).unwrap();
        let untouched = [owner.position(d), owner.position(foreign)];
        let context = MovementContext;
        let mut guard =
            OptimizationWorkGuard::new(Some(&context), "sizelessOptimizer.optimize", 1000).unwrap();
        let mut session = OwnerScoreSession::new(&mut owner, view, mapping, None).unwrap();
        let mut movement = session.optimizer_candidate_movement(c, &mut guard).unwrap();
        assert_eq!(
            movement.positions.iter().map(|p| p.0).collect::<Vec<_>>(),
            [c, child]
        );
        assert_eq!(guard.used(), 4); // push + pop + two position snapshots
        movement
            .move_abs(Point { x: 83.0, y: 85.0 }, &mut guard)
            .unwrap();
        assert_eq!(guard.used(), 8); // walk replay + validation + child write
        assert_eq!(
            movement.session().owner().position(child),
            Some(Point {
                x: child_before.x + 83.0 - root_before.x,
                y: child_before.y + 85.0 - root_before.y,
            })
        );
        movement
            .move_abs(Point { x: 83.0, y: 85.0 }, &mut guard)
            .unwrap();
        assert_eq!(guard.used(), 9);
        movement.commit();
        assert_eq!(
            [
                session.owner().position(d),
                session.owner().position(foreign)
            ],
            untouched
        );
        assert_eq!(
            session.owner().position(c),
            Some(Point { x: 83.0, y: 85.0 })
        );
        drop(session);
        assert_eq!(owner.position(c), Some(Point { x: 83.0, y: 85.0 }));
    }

    #[test]
    fn optimizer_candidate_partial_move_error_restores_only_captured_positions() {
        let (mut owner, _, view, mapping, [c, child, _, foreign, _]) = initialization_fixture();
        let before = [owner.position(c), owner.position(child)];
        let context = MovementContext;
        let mut guard =
            OptimizationWorkGuard::new(Some(&context), "sizelessOptimizer.optimize", 7).unwrap();
        let mut session = OwnerScoreSession::new(&mut owner, view, mapping, None).unwrap();
        {
            let mut movement = session.optimizer_candidate_movement(c, &mut guard).unwrap();
            // The descendant write exceeds the budget after the root moved.
            assert!(matches!(
                movement.move_abs(Point { x: 90.0, y: 95.0 }, &mut guard),
                Err(OptimizationError::ResourceLimit { .. })
            ));
            assert_eq!(
                movement.session().owner().position(c),
                Some(Point { x: 90.0, y: 95.0 })
            );
            assert_eq!(movement.session().owner().position(child), before[1]);
            movement.session().owner.nodes[c.0 as usize].rect.size.width = 77.0;
            movement
                .session()
                .owner
                .set_position(foreign, Point { x: 700.0, y: 800.0 });
        }
        assert_eq!(
            [session.owner().position(c), session.owner().position(child)],
            before
        );
        assert_eq!(session.owner().nodes[c.0 as usize].rect.size.width, 77.0);
        assert_eq!(
            session.owner().position(foreign),
            Some(Point { x: 700.0, y: 800.0 })
        );
    }

    #[test]
    fn optimizer_candidate_panic_restores_search_start_after_multiple_trials() {
        let (mut owner, _, view, mapping, [c, child, _, _, _]) = initialization_fixture();
        let before = [owner.position(c), owner.position(child)];
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let context = MovementContext;
            let mut guard =
                OptimizationWorkGuard::new(Some(&context), "sizelessOptimizer.optimize", 1000)
                    .unwrap();
            let mut session = OwnerScoreSession::new(&mut owner, view, mapping, None).unwrap();
            let mut movement = session.optimizer_candidate_movement(c, &mut guard).unwrap();
            movement
                .move_abs(Point { x: 90.0, y: 95.0 }, &mut guard)
                .unwrap();
            movement
                .move_abs(Point { x: 400.0, y: 300.0 }, &mut guard)
                .unwrap();
            panic!("candidate scorer panic");
        }));
        assert!(result.is_err());
        assert_eq!([owner.position(c), owner.position(child)], before);
    }

    #[test]
    fn optimizer_candidate_deduplicates_when_pushed_and_keeps_frozen_movement_set() {
        let (mut owner, _, view, mapping, [c, child, _, nested, leaf]) = initialization_fixture();
        owner.nodes[child.0 as usize].is_container = true;
        owner.containers.insert(Some(c), vec![child, leaf]);
        owner.containers.insert(Some(child), vec![leaf, nested, c]);
        let nested_before = owner.position(nested).unwrap();
        let context = MovementContext;
        let mut guard =
            OptimizationWorkGuard::new(Some(&context), "sizelessOptimizer.optimize", 1000).unwrap();
        let mut session = OwnerScoreSession::new(&mut owner, view, mapping, None).unwrap();
        let mut movement = session.optimizer_candidate_movement(c, &mut guard).unwrap();
        assert_eq!(
            movement.positions.iter().map(|p| p.0).collect::<Vec<_>>(),
            [c, child, nested, leaf]
        );
        assert_eq!(movement.descendant_work, 8); // 5 references + 3 pops
        // Source search topology stays fixed even if a retained header changes.
        movement
            .session()
            .owner
            .containers
            .insert(Some(child), vec![leaf]);
        movement
            .move_abs(Point { x: 90.0, y: 95.0 }, &mut guard)
            .unwrap();
        assert_eq!(
            movement.session().owner().position(nested),
            Some(Point {
                x: nested_before.x + 10.0,
                y: nested_before.y + 15.0
            })
        );
        movement.commit();
    }

    #[test]
    fn optimizer_candidate_rejects_nil_descendant_without_skipping_and_no_move_accepts_nil_position()
     {
        let (mut owner, _, view, mapping, [c, child, _, _, _]) = initialization_fixture();
        owner
            .containers
            .get_header(&Some(c))
            .unwrap()
            .write_capacity_slot(0, None)
            .unwrap();
        let context = MovementContext;
        let mut guard =
            OptimizationWorkGuard::new(Some(&context), "sizelessOptimizer.optimize", 1000).unwrap();
        let mut session = OwnerScoreSession::new(&mut owner, view, mapping, None).unwrap();
        assert_eq!(
            session
                .optimizer_candidate_movement(c, &mut guard)
                .err()
                .unwrap()
                .to_string(),
            "TALA sizelessOptimizer.optimize found a nil descendant"
        );
        assert_eq!(guard.used(), 1);
        session
            .owner
            .containers
            .get_header(&Some(c))
            .unwrap()
            .write_capacity_slot(0, Some(child))
            .unwrap();
        session.owner.clear_position(child);
        let current = session.owner.position(c).unwrap();
        let mut movement = session.optimizer_candidate_movement(c, &mut guard).unwrap();
        movement.move_abs(current, &mut guard).unwrap();
        assert_eq!(
            movement
                .move_abs(Point { x: 90.0, y: 95.0 }, &mut guard)
                .unwrap_err()
                .to_string(),
            "TALA sizelessOptimizer.optimize cannot move an unpositioned descendant"
        );
        assert_eq!(movement.session().owner().position(c), Some(current));
        drop(movement);
        assert_eq!(session.owner().position(child), None);
    }

    #[test]
    fn optimizer_candidate_leaf_capture_uses_explicit_mapping_and_one_work_unit() {
        let (mut owner, [_, _, _, _, leaf]) = fixture();
        let view = owner.create_owner_graph_view(vec![leaf], vec![]).unwrap();
        let mapping =
            CanonicalScoreMapping::new(owner.topology_arena_id().unwrap(), vec![leaf], vec![]);
        let context = MovementContext;
        let mut guard =
            OptimizationWorkGuard::new(Some(&context), "sizelessOptimizer.optimize", 1000).unwrap();
        let mut session = OwnerScoreSession::new(&mut owner, view, mapping, None).unwrap();
        let mut movement = session
            .optimizer_candidate_movement(NodeId(0), &mut guard)
            .unwrap();
        assert_eq!(movement.positions[0].0, leaf);
        assert_eq!(guard.used(), 1);
        movement
            .move_abs(Point { x: 3.0, y: 4.0 }, &mut guard)
            .unwrap();
        assert_eq!(guard.used(), 1); // nonzero leaf Translate charges no additional work
        movement.commit();
        assert_eq!(
            session.owner().position(leaf),
            Some(Point { x: 3.0, y: 4.0 })
        );
    }

    #[cfg(feature = "diagnostic-traces")]
    #[test]
    fn optimizer_state_owner_poll_reads_loaded_identity_then_live_geometry() {
        struct LiveGeometryContext {
            polls: Cell<usize>,
            retained: SharedNodeSlice,
            child: NodeId,
            foreign: NodeId,
        }
        impl limits::Context for LiveGeometryContext {
            fn done(&self) -> Option<bool> {
                None
            }
            fn error(&self) -> Option<limits::ContextError> {
                panic!("owner capture must dispatch its actual mutable owner")
            }
            fn error_with_owner(&self, owner: &mut ArenaGraph) -> Option<limits::ContextError> {
                self.polls.set(self.polls.get() + 1);
                if self.polls.get() == 3 {
                    // Slot61 has already yielded child at the real Step64.
                    self.retained
                        .write_capacity_slot(61, Some(self.foreign))
                        .unwrap();
                    owner.set_position(
                        self.child,
                        Point {
                            x: 731.25,
                            y: -19.5,
                        },
                    );
                    owner.nodes[self.child.0 as usize].rect.size = Size {
                        width: 103.5,
                        height: -0.0,
                    };
                }
                None
            }
        }
        let (mut owner, [c, child, d, foreign_child, leaf], receiver, canonical) =
            optimizer_state_fixture();
        let mut slots = vec![Some(c); 63];
        slots[61] = Some(child);
        slots[62] = Some(d);
        let retained = SharedNodeSlice::from_slots(&slots).unwrap();
        owner.containers.replace_header(Some(c), retained.clone());
        let context = LiveGeometryContext {
            polls: Cell::new(0),
            retained: retained.clone(),
            child,
            foreign: d,
        };
        let mut guard = OptimizationWorkGuard::new_with_owner(
            Some(&context),
            "sizelessOptimizer.optimize",
            1000,
            &mut owner,
        )
        .unwrap();
        let mut snapshot = OwnerOptimizerMutationSnapshot::default();
        {
            let mut session =
                OwnerScoreSession::new(&mut owner, receiver, canonical, None).unwrap();
            session
                .capture_optimizer_mutation_state_into(&mut guard, &mut snapshot)
                .unwrap();
        }
        assert!(snapshot.is_complete());
        assert_eq!(
            snapshot.captured_nodes().collect::<Vec<_>>(),
            [c, leaf, child, d, foreign_child]
        );
        assert_eq!(guard.used(), 71); // roots2 + retained slots63 + D child1 + nil herds5.
        assert_eq!(context.polls.get(), 3); // constructor, cost Finish, Step64.
        let saved_child = snapshot
            .captured_geometry()
            .find(|(node, _)| *node == child)
            .unwrap()
            .1;
        assert_eq!(
            saved_child.position,
            Some(Point {
                x: 731.25,
                y: -19.5
            })
        );
        assert_eq!(saved_child.rect.size.width, 103.5);
        assert_eq!(saved_child.rect.size.height.to_bits(), (-0.0f64).to_bits());
        assert_eq!(retained.get(61), Some(Some(d)));
        owner.set_position(
            child,
            Point {
                x: 9000.0,
                y: 9000.0,
            },
        );
        owner.nodes[child.0 as usize].rect.size = Size {
            width: 1.0,
            height: 2.0,
        };
        snapshot.restore(&mut owner).unwrap();
        assert_eq!(
            OwnerNodeGeometry::from_node(&owner.nodes[child.0 as usize]),
            saved_child
        );
        assert_eq!(retained.get(61), Some(Some(d))); // geometry restore leaves the callback header edit.
        assert_eq!(guard.used(), 71);
        assert_eq!(context.polls.get(), 3);
    }

    #[cfg(feature = "diagnostic-traces")]
    #[test]
    fn optimizer_state_owner_finish_failure_keeps_prearm_effects_and_original_panic() {
        struct FinishContext {
            polls: Cell<usize>,
            node: NodeId,
            cache: std::sync::Arc<GraphCostCache>,
            old_alias: crate::internal::model::EdgeLengthCacheAlias,
            panic_payload: Option<std::sync::Arc<u64>>,
        }
        impl limits::Context for FinishContext {
            fn done(&self) -> Option<bool> {
                None
            }
            fn error(&self) -> Option<limits::ContextError> {
                panic!("unexpected ordinary context poll")
            }
            fn error_with_owner(&self, owner: &mut ArenaGraph) -> Option<limits::ContextError> {
                self.polls.set(self.polls.get() + 1);
                if self.polls.get() != 3 {
                    return None;
                }
                owner.set_position(self.node, Point { x: -71.5, y: 84.25 });
                owner.nodes[self.node.0 as usize].rect.size = Size {
                    width: 321.0,
                    height: 123.0,
                };
                self.old_alias.store(999, 17.5);
                self.cache.publish_deserialized_routing((70.0, 80.0, 90.0));
                self.cache.store_edge_length(888, 88.0);
                if let Some(payload) = &self.panic_payload {
                    std::panic::panic_any(payload.clone());
                }
                Some(limits::ContextError::Cancelled)
            }
        }
        for panic_at_finish in [false, true] {
            let (mut owner, [_, child, _, _, _], receiver, canonical) = optimizer_state_fixture();
            let cache = owner.owner_graph_cost_cache(receiver).unwrap().clone();
            for key in 0..65 {
                cache.store_edge_length(key, key as f64);
            }
            let old_alias = cache.edge_length_cache_alias().unwrap();
            let payload = std::sync::Arc::new(0x123456789abcdef0u64);
            let context = FinishContext {
                polls: Cell::new(0),
                node: child,
                cache: cache.clone(),
                old_alias,
                panic_payload: panic_at_finish.then(|| payload.clone()),
            };
            let mut guard = OptimizationWorkGuard::new_with_owner(
                Some(&context),
                "sizelessOptimizer.optimize",
                1000,
                &mut owner,
            )
            .unwrap();
            let mut snapshot = OwnerOptimizerMutationSnapshot::default();
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut session =
                    OwnerScoreSession::new(&mut owner, receiver, canonical, None).unwrap();
                session.capture_optimizer_mutation_state_into(&mut guard, &mut snapshot)
            }));
            if panic_at_finish {
                let recovered = result
                    .unwrap_err()
                    .downcast::<std::sync::Arc<u64>>()
                    .unwrap();
                assert!(std::sync::Arc::ptr_eq(&recovered, &payload));
            } else {
                assert_eq!(
                    result.unwrap(),
                    Err(OptimizationError::Context {
                        location: "sizelessOptimizer.optimize".into(),
                        cause: limits::ContextError::Cancelled
                    })
                );
            }
            assert!(!snapshot.is_complete());
            assert!(snapshot.cost_snapshot().is_none()); // copied costs attach only AFTER successful Finish.
            assert_eq!(snapshot.captured_nodes().len(), 0);
            assert_eq!(guard.used(), 65);
            assert_eq!(context.polls.get(), 3);
            snapshot.restore(&mut owner).unwrap(); // no successful returned snapshot; no rollback is armed.
            snapshot.release();
            assert_eq!(owner.position(child), Some(Point { x: -71.5, y: 84.25 }));
            assert_eq!(
                owner.nodes[child.0 as usize].rect.size,
                Size {
                    width: 321.0,
                    height: 123.0
                }
            );
            assert!(!context.old_alias.is_current());
            assert_eq!(context.old_alias.lookup(999), Some(17.5));
            assert_eq!(cache.lookup_edge_length(999), None);
            assert_eq!(cache.lookup_edge_length(888), Some(88.0));
            assert_eq!(cache.edge_length_entries(), 1);
            assert_eq!(cache.raw(), (70.0, 80.0, 90.0));
            assert_eq!(guard.used(), 65);
            assert_eq!(context.polls.get(), 3);
        }
    }

    #[cfg(feature = "diagnostic-traces")]
    #[test]
    fn optimizer_state_owner_poll_retains_old_member_range_on_replacement_and_restore() {
        struct ReplaceMembersContext {
            polls: Cell<usize>,
            receiver: OwnerGraphId,
            replacement: NodeId,
        }
        impl limits::Context for ReplaceMembersContext {
            fn done(&self) -> Option<bool> {
                None
            }
            fn error(&self) -> Option<limits::ContextError> {
                panic!("unexpected ordinary context poll")
            }
            fn error_with_owner(&self, owner: &mut ArenaGraph) -> Option<limits::ContextError> {
                self.polls.set(self.polls.get() + 1);
                if self.polls.get() == 3 {
                    let edges = owner.owner_graph_view(self.receiver).unwrap().edges.read();
                    owner
                        .set_owner_graph_members(self.receiver, vec![self.replacement], edges)
                        .unwrap();
                }
                None
            }
        }
        let (mut owner, [c, child, d, _, _], receiver, canonical) = optimizer_state_fixture();
        let edges = owner.owner_graph_view(receiver).unwrap().edges.read();
        owner
            .set_owner_graph_members(receiver, vec![c; 65], edges)
            .unwrap();
        let context = ReplaceMembersContext {
            polls: Cell::new(0),
            receiver,
            replacement: d,
        };
        let mut guard = OptimizationWorkGuard::new_with_owner(
            Some(&context),
            "sizelessOptimizer.optimize",
            1000,
            &mut owner,
        )
        .unwrap();
        let mut snapshot = OwnerOptimizerMutationSnapshot::default();
        {
            let mut session =
                OwnerScoreSession::new(&mut owner, receiver, canonical, None).unwrap();
            session
                .capture_optimizer_mutation_state_into(&mut guard, &mut snapshot)
                .unwrap();
        }
        assert_eq!(guard.used(), 68); //65 retained roots +child1 +nil-herds2.
        assert_eq!(context.polls.get(), 3);
        assert!(snapshot.is_complete());
        assert_eq!(snapshot.captured_nodes().collect::<Vec<_>>(), [c, child]);
        let saved = owner.position(c);
        owner.set_position(
            c,
            Point {
                x: 999.0,
                y: -777.0,
            },
        );
        snapshot.restore(&mut owner).unwrap();
        assert_eq!(owner.position(c), saved);
        assert_eq!(
            owner
                .owner_graph_view(receiver)
                .unwrap()
                .nodes
                .require_nodes()
                .collect::<Vec<_>>(),
            [d]
        );
        assert_eq!(guard.used(), 68);
        assert_eq!(context.polls.get(), 3);
    }

    #[cfg(feature = "diagnostic-traces")]
    #[test]
    fn optimizer_state_member_range_loads_nullable_slot_before_step_and_reads_future_slot_live() {
        struct SlotContext {
            polls: Cell<usize>,
            header: OwnerMemberHeader,
            index: usize,
            value: Option<NodeId>,
        }
        impl limits::Context for SlotContext {
            fn done(&self) -> Option<bool> {
                None
            }
            fn error(&self) -> Option<limits::ContextError> {
                panic!("unexpected ordinary poll")
            }
            fn error_with_owner(&self, owner: &mut ArenaGraph) -> Option<limits::ContextError> {
                self.polls.set(self.polls.get() + 1);
                if self.polls.get() == 3 {
                    owner
                        .write_owner_member_capacity_slot(&self.header, self.index, self.value)
                        .unwrap();
                }
                None
            }
        }
        // Step64 loads slot63 before poll3; slot64 is fetched after that poll.
        for case in 0..4 {
            let (mut owner, [c, child, d, d_child, _], receiver, _) = optimizer_state_fixture();
            let mut values = vec![Some(c); 65];
            if case == 1 {
                values[63] = Some(d);
            }
            let header = owner.allocate_owner_member_header(&values, 70).unwrap();
            owner
                .set_owner_graph_member_header(receiver, header.clone())
                .unwrap();
            let context = SlotContext {
                polls: Cell::new(0),
                header: header.clone(),
                index: if case < 2 { 63 } else { 64 },
                value: if case == 0 || case == 2 {
                    Some(d)
                } else {
                    None
                },
            };
            let mut guard = OptimizationWorkGuard::new_with_owner(
                Some(&context),
                "sizelessOptimizer.optimize",
                1000,
                &mut owner,
            )
            .unwrap();
            let mut snapshot = OwnerOptimizerMutationSnapshot::default();
            let result = snapshot.capture_into(&mut owner, receiver, &mut guard);
            let expected = if case == 0 {
                vec![c, child]
            } else if case == 3 {
                vec![c]
            } else {
                vec![c, d, child, d_child]
            };
            assert_eq!(snapshot.captured_nodes().collect::<Vec<_>>(), expected);
            assert_eq!(
                guard.used(),
                if case == 0 {
                    68
                } else if case == 3 {
                    65
                } else {
                    71
                }
            );
            assert_eq!(context.polls.get(), 3);
            assert_eq!(header.get(context.index), Some(context.value));
            assert_eq!((header.len(), header.capacity()), (65, 70));
            assert_eq!(header.get(65), None);
            if case == 3 {
                assert_eq!(
                    result.unwrap_err().to_string(),
                    "TALA sizelessOptimizer.optimize found a nil graph node"
                );
                assert!(!snapshot.is_complete());
            } else {
                result.unwrap();
                assert!(snapshot.is_complete());
            }
            let before = owner.position(c);
            owner.set_position(
                c,
                Point {
                    x: 999.0,
                    y: -777.0,
                },
            );
            snapshot.restore(&mut owner).unwrap();
            assert_eq!(
                owner.position(c),
                if case == 3 {
                    Some(Point {
                        x: 999.0,
                        y: -777.0,
                    })
                } else {
                    before
                }
            );
            // The geometry-only optimizer restore never repairs member slots.
            assert_eq!(header.get(context.index), Some(context.value));
            snapshot.release();
            assert_eq!(header.get(context.index), Some(context.value));
        }
    }

    #[test]
    fn optimizer_state_nil_member_slot_charges_before_nil_error_and_empty_headers_stay_distinct() {
        let context = MovementContext;
        for allocated in [false, true] {
            let (mut owner, _, receiver, _) = optimizer_state_fixture();
            let header = if allocated {
                owner.allocate_owner_member_header(&[], 0).unwrap()
            } else {
                owner.nil_owner_member_header()
            };
            owner
                .set_owner_graph_member_header(receiver, header.clone())
                .unwrap();
            let mut guard =
                OptimizationWorkGuard::new(Some(&context), "sizelessOptimizer.optimize", 1000)
                    .unwrap();
            let mut snapshot = OwnerOptimizerMutationSnapshot::default();
            snapshot
                .capture_into(&mut owner, receiver, &mut guard)
                .unwrap();
            assert!(snapshot.is_complete());
            assert_eq!(snapshot.captured_nodes().len(), 0);
            assert_eq!(guard.used(), 0);
            assert_eq!(
                owner.owner_graph_member_header(receiver).unwrap().is_nil(),
                !allocated
            );
        }
        for limit in [0, 1] {
            let (mut owner, _, receiver, _) = optimizer_state_fixture();
            let header = owner.allocate_owner_member_header(&[None], 1).unwrap();
            owner
                .set_owner_graph_member_header(receiver, header)
                .unwrap();
            let mut guard =
                OptimizationWorkGuard::new(Some(&context), "sizelessOptimizer.optimize", limit)
                    .unwrap();
            let mut snapshot = OwnerOptimizerMutationSnapshot::default();
            let error = snapshot
                .capture_into(&mut owner, receiver, &mut guard)
                .unwrap_err();
            if limit == 0 {
                assert!(matches!(error, OptimizationError::ResourceLimit { .. }));
                assert_eq!(guard.used(), 0);
            } else {
                assert_eq!(
                    error.to_string(),
                    "TALA sizelessOptimizer.optimize found a nil graph node"
                );
                assert_eq!(guard.used(), 1);
            }
            assert!(!snapshot.is_complete());
            assert_eq!(snapshot.captured_nodes().len(), 0);
        }
    }

    #[test]
    fn optimizer_candidate_cancellation_at_descendant_write_keeps_actual_partial_state_until_cleanup()
     {
        struct CancelOnSecondPoll(Cell<usize>);
        impl limits::Context for CancelOnSecondPoll {
            fn done(&self) -> Option<bool> {
                None
            }
            fn error(&self) -> Option<limits::ContextError> {
                self.0.set(self.0.get() + 1);
                (self.0.get() == 2).then_some(limits::ContextError::Cancelled)
            }
        }
        let (mut owner, _, view, mapping, [c, child, _, _, _]) = initialization_fixture();
        let before = [owner.position(c), owner.position(child)];
        let context = CancelOnSecondPoll(Cell::new(0));
        let mut guard =
            OptimizationWorkGuard::new(Some(&context), "sizelessOptimizer.optimize", 1000).unwrap();
        let mut session = OwnerScoreSession::new(&mut owner, view, mapping, None).unwrap();
        {
            let mut movement = session.optimizer_candidate_movement(c, &mut guard).unwrap();
            guard.add(56).unwrap(); // child mutation reaches the real 64-unit poll
            assert_eq!(
                movement
                    .move_abs(Point { x: 90.0, y: 95.0 }, &mut guard)
                    .unwrap_err()
                    .to_string(),
                "sizelessOptimizer.optimize: context canceled"
            );
            assert_eq!(guard.used(), 64);
            assert_eq!(context.0.get(), 2);
            assert_eq!(
                movement.session().owner().position(c),
                Some(Point { x: 90.0, y: 95.0 })
            );
            assert_eq!(movement.session().owner().position(child), before[1]);
        }
        assert_eq!(
            [session.owner().position(c), session.owner().position(child)],
            before
        );
    }
}

