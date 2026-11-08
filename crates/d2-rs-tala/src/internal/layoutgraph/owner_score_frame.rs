// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Borrowed access to the actual owning nodes and captured Containers headers.
//!
//! D2 v0.9.0 placementcost/edge_length.go875–926 reads node.Graph.Containers,
//! captures slice headers, polls903/907, then reads each live pointer and its
//! geometry/Graph. The frame must be dropped before exclusive owner mutation.
//! It does not make dense solver candidates physical aliases of these nodes.

use super::container_slices::SharedNodeSlice;
use super::*;
use std::borrow::Cow;
use std::fmt;

/// Most obstruction ancestry walks visit only a few containers. Keep their
/// cycle/deduplication bookkeeping on the stack; deeper walks retain the
/// existing ordered-set fallback, allocated only by such a walk. This is
/// membership only, never traversal order, and still reports the first
/// repeated identity.
struct AncestryVisited<T: Copy + Ord> {
    inline: [Option<T>; 16],
    len: usize,
    overflow: Option<BTreeSet<T>>,
}

impl<T: Copy + Ord> AncestryVisited<T> {
    fn new() -> Self {
        Self {
            inline: [None; 16],
            len: 0,
            overflow: None,
        }
    }

    fn insert(&mut self, value: T) -> bool {
        if self.inline[..self.len].contains(&Some(value)) {
            return false;
        }
        if self.len < self.inline.len() {
            self.inline[self.len] = Some(value);
            self.len += 1;
            true
        } else {
            self.overflow
                .get_or_insert_with(BTreeSet::new)
                .insert(value)
        }
    }

    fn clear(&mut self) {
        self.len = 0;
        self.overflow = None;
    }

    fn contains(&self, value: T) -> bool {
        self.inline[..self.len].contains(&Some(value))
            || self
                .overflow
                .as_ref()
                .is_some_and(|overflow| overflow.contains(&value))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum OwnerScoreError {
    ForeignArena {
        expected: OwnerArenaId,
        actual: Option<OwnerArenaId>,
    },
    MappingMismatch(OwnerGraphId),
    UnknownLocalNode(NodeId),
    UnknownLocalEdge(EdgeId),
    MissingNodeOwner(NodeId),
    InvalidAbductionSlice,
    AncestryCycle(NodeId),
    Topology(OwnerTopologyError),
}

impl From<OwnerTopologyError> for OwnerScoreError {
    fn from(error: OwnerTopologyError) -> Self {
        Self::Topology(error)
    }
}

impl fmt::Display for OwnerScoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "borrowed owning score frame: {self:?}")
    }
}

impl std::error::Error for OwnerScoreError {}

/// A map lookup retains absence separately from a present nil/empty header.
/// Each get reads its original backing lazily. No item or geometry Vec is
/// stored here, and no lock is held while the caller polls or evaluates it.
#[derive(Clone, Debug)]
pub(super) struct OwnerObstructionSet {
    pub(super) parent: Option<NodeId>,
    // Only the slice header is retained. The OwnerContainerHeader's map
    // binding guards writes back into the map; obstruction sets only read.
    pub(super) header: Option<SharedNodeSlice>,
}

impl OwnerObstructionSet {
    pub(super) fn key_present(&self) -> bool {
        self.header.is_some()
    }

    pub(super) fn is_nil(&self) -> bool {
        self.header.as_ref().is_none_or(SharedNodeSlice::is_nil)
    }

    pub(super) fn len(&self) -> usize {
        self.header.as_ref().map_or(0, SharedNodeSlice::len)
    }

    pub(super) fn capacity(&self) -> usize {
        self.header.as_ref().map_or(0, SharedNodeSlice::capacity)
    }

    /// Call only after the source907 item poll. Some(None) is a reached nil
    /// slot, distinct from an out-of-range index. Source layout.go314–331
    /// makes nil a universal ancestor, so ordinary replacements skip it at
    /// the ancestry predicate before reading TopLeft (edge_length.go914).
    #[cfg(any(test, feature = "diagnostic-traces"))]
    pub(super) fn raw_slot(&self, index: usize) -> Option<Option<NodeId>> {
        self.header.as_ref()?.get(index)
    }

    /// edge_length.go:907-910 polls before loading each obstruction pointer.
    /// Retain the captured window while deferring its actual slot loads.
    pub(super) fn slot_readers(&self) -> super::container_slices::BorrowedNodeSlots<'_> {
        self.header
            .as_ref()
            .map(SharedNodeSlice::borrowed_slot_readers)
            .unwrap_or_default()
    }
}

/// One recorded `OwnerScoreFrame::is_descendant_of` walk from a fixed
/// descendant; see `OwnerScoreFrame::record_ancestry_chain`. The slot is
/// filled in place, so the walk's set is never moved.
pub(super) struct OwnerAncestryChain {
    nodes: AncestryVisited<NodeId>,
    end: Result<(), OwnerScoreError>,
    recorded: bool,
}

impl OwnerAncestryChain {
    /// A slot holding no walk yet.
    pub(super) fn unrecorded() -> Self {
        Self {
            nodes: AncestryVisited::new(),
            end: Ok(()),
            recorded: false,
        }
    }

    /// Forget the recorded walk; the next record walks again.
    pub(super) fn invalidate(&mut self) {
        self.recorded = false;
    }
}

// edge_length.go50,64–96,861–862: reuse empty scratch storage, releasing
// captured header references at each score's original return boundary. Each
// checkout owns its vector, so nested scoring never aliases an active buffer.
thread_local! {
    static OBSTRUCTION_SCRATCH: std::cell::RefCell<Vec<Vec<OwnerObstructionSet>>> = const { std::cell::RefCell::new(Vec::new()) };
}

#[derive(Debug)]
pub(super) struct OwnerObstructionSets(Vec<OwnerObstructionSet>);

impl OwnerObstructionSets {
    fn take() -> Self {
        Self(OBSTRUCTION_SCRATCH.with(|pool| pool.borrow_mut().pop().unwrap_or_default()))
    }

    fn push(&mut self, set: OwnerObstructionSet) {
        self.0.push(set);
    }

    #[cfg(test)]
    fn remove(&mut self, index: usize) -> OwnerObstructionSet {
        self.0.remove(index)
    }
}

impl std::ops::Deref for OwnerObstructionSets {
    type Target = [OwnerObstructionSet];
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Drop for OwnerObstructionSets {
    fn drop(&mut self) {
        self.0.clear();
        if self.0.capacity() > 4096 {
            return;
        }
        let _ = OBSTRUCTION_SCRATCH.try_with(|pool| {
            let mut pool = pool.borrow_mut();
            if pool.len() < 4 {
                pool.push(std::mem::take(&mut self.0));
            }
        });
    }
}

/// Immutable owner and explicit local mapping borrow, scoped to a scoring
/// call or Rayon join. Geometry and ownership are resolved from owner at use;
/// no copied node, replacement eligibility or historical position is stored.
#[derive(Clone, Debug)]
pub(super) struct OwnerScoreFrame<'a> {
    owner: &'a ArenaGraph,
    source_arena: OwnerArenaId,
    mapping_view: OwnerGraphId,
    local_to_canonical: &'a [NodeId],
    /// Dense edge correspondence is separate from actual Graph.Edges. Source
    /// maxEdgeLength/NodeEdgeLength still follow Node.Edges after membership
    /// replacement; callers retain this mapping across that operation.
    local_to_canonical_edges: Cow<'a, [EdgeId]>,
    abductions: Option<&'a OwnerAbductionSlice>,
}

impl<'a> OwnerScoreFrame<'a> {
    pub(super) fn new(
        owner: &'a ArenaGraph,
        source_arena: OwnerArenaId,
        mapping_view: OwnerGraphId,
        local_to_canonical: &'a [NodeId],
    ) -> Result<Self, OwnerScoreError> {
        Self::new_with_edges(owner, source_arena, mapping_view, local_to_canonical, None)
    }

    /// Sessions already own a stable dense correspondence. Borrow it rather
    /// than first copying Graph.Edges and immediately replacing that copy.
    /// Graph.Edges membership can differ from this mapping; neither it nor
    /// live geometry is cached. Validation retains new/with_edge_mapping order.
    pub(super) fn new_with_edge_mapping(
        owner: &'a ArenaGraph,
        source_arena: OwnerArenaId,
        mapping_view: OwnerGraphId,
        local_to_canonical: &'a [NodeId],
        edges: &'a [EdgeId],
    ) -> Result<Self, OwnerScoreError> {
        Self::new_with_edges(
            owner,
            source_arena,
            mapping_view,
            local_to_canonical,
            Some(edges),
        )
    }

    /// The frame `new_with_edge_mapping` returns for these same inputs, for
    /// a caller that has already received it from them with no owner
    /// mutation since. Every check would pass again and is not repeated.
    pub(super) fn validated_edge_mapping(
        owner: &'a ArenaGraph,
        source_arena: OwnerArenaId,
        mapping_view: OwnerGraphId,
        local_to_canonical: &'a [NodeId],
        edges: &'a [EdgeId],
    ) -> Self {
        Self {
            owner,
            source_arena,
            mapping_view,
            local_to_canonical,
            local_to_canonical_edges: Cow::Borrowed(edges),
            abductions: None,
        }
    }

    fn new_with_edges(
        owner: &'a ArenaGraph,
        source_arena: OwnerArenaId,
        mapping_view: OwnerGraphId,
        local_to_canonical: &'a [NodeId],
        edges: Option<&'a [EdgeId]>,
    ) -> Result<Self, OwnerScoreError> {
        if !owner.shared_topology_initialized() || owner.topology_arena_id() != Some(source_arena) {
            return Err(OwnerScoreError::ForeignArena {
                expected: source_arena,
                actual: owner.topology_arena_id(),
            });
        }
        let view = owner
            .owner_graph_view(mapping_view)
            .ok_or(OwnerTopologyError::UnknownGraph(mapping_view))?;
        if view.local_to_canonical.as_slice() != local_to_canonical {
            return Err(OwnerScoreError::MappingMismatch(mapping_view));
        }
        for &canonical in local_to_canonical {
            owner
                .topology_node(canonical)
                .ok_or(OwnerTopologyError::UnknownNode(canonical))?;
        }
        let frame = Self {
            owner,
            source_arena,
            mapping_view,
            local_to_canonical,
            local_to_canonical_edges: match edges {
                Some(edges) => {
                    // Preserve the original snapshot reader's raw nil-member
                    // panic, before explicit-edge validation, without copying.
                    view.edges.assert_values_non_nil();
                    Cow::Borrowed(edges)
                }
                None => Cow::Owned(view.edges.read()),
            },
            abductions: None,
        };
        match edges {
            Some(edges) => frame.with_edge_mapping(edges),
            None => Ok(frame),
        }
    }

    /// The frame's owning arena was validated by new. Each supplied handle
    /// must resolve in that same canonical arena; no Tala ID or local numeric
    /// equality infers a missing binding. Membership does not constrain it.
    pub(super) fn with_edge_mapping(
        mut self,
        edges: &'a [EdgeId],
    ) -> Result<Self, OwnerScoreError> {
        if self.owner.topology_arena_id() != Some(self.source_arena) {
            return Err(OwnerScoreError::ForeignArena {
                expected: self.source_arena,
                actual: self.owner.topology_arena_id(),
            });
        }
        for &edge in edges {
            self.edge(edge)?;
        }
        self.local_to_canonical_edges = Cow::Borrowed(edges);
        Ok(self)
    }

    /// Retain the actual ordered source-record slice by borrow. Genuine nil
    /// has no records/arena; a present empty slice still binds its namespace.
    /// Repeated record pointers keep their order and identity unchanged.
    pub(super) fn with_abductions(
        mut self,
        slice: &'a OwnerAbductionSlice,
    ) -> Result<Self, OwnerScoreError> {
        if !slice.present {
            if !slice.records.is_empty() || slice.arena.is_some() {
                return Err(OwnerScoreError::InvalidAbductionSlice);
            }
        } else if slice.arena != Some(self.source_arena) {
            return Err(OwnerScoreError::ForeignArena {
                expected: self.source_arena,
                actual: slice.arena,
            });
        }
        for &handle in &slice.records {
            let record = self
                .owner
                .owner_edge_abduction(handle)
                .ok_or(OwnerTopologyError::UnknownRecord(handle))?;
            if let Some(edge) = record.source_edge {
                self.edge(edge)?;
            }
            for node in [
                record.originally_from,
                record.originally_to,
                record.current_from,
                record.current_to,
            ]
            .into_iter()
            .flatten()
            {
                self.node(node)?;
            }
        }
        self.abductions = Some(slice);
        Ok(self)
    }

    pub(super) fn abductions(&self) -> Option<&'a OwnerAbductionSlice> {
        self.abductions
    }

    pub(super) fn owner(&self) -> &'a ArenaGraph {
        self.owner
    }

    pub(super) fn source_arena(&self) -> OwnerArenaId {
        self.source_arena
    }

    pub(super) fn mapping_view(&self) -> OwnerGraphId {
        self.mapping_view
    }

    pub(super) fn local_to_canonical(&self) -> &'a [NodeId] {
        self.local_to_canonical
    }

    pub(super) fn local_to_canonical_edges(&self) -> &[EdgeId] {
        &self.local_to_canonical_edges
    }

    pub(super) fn canonical_edge(&self, local: EdgeId) -> Result<EdgeId, OwnerScoreError> {
        self.local_to_canonical_edges
            .get(local.0 as usize)
            .copied()
            .ok_or(OwnerScoreError::UnknownLocalEdge(local))
    }

    pub(super) fn canonical_node(&self, local: NodeId) -> Result<NodeId, OwnerScoreError> {
        self.local_to_canonical
            .get(local.0 as usize)
            .copied()
            .ok_or(OwnerScoreError::UnknownLocalNode(local))
    }

    pub(super) fn node(&self, canonical: NodeId) -> Result<&'a ArenaNode, OwnerScoreError> {
        self.owner
            .topology_node(canonical)
            .ok_or_else(|| OwnerTopologyError::UnknownNode(canonical).into())
    }

    pub(super) fn edge(&self, canonical: EdgeId) -> Result<&'a ArenaEdge, OwnerScoreError> {
        self.owner
            .topology_edge(canonical)
            .ok_or_else(|| OwnerTopologyError::UnknownEdge(canonical).into())
    }

    /// Source lineage with another namespace is rejected even if raw NodeId
    /// values happen to identify a dense node in this frame.
    pub(super) fn source_node(
        &self,
        source_arena: OwnerArenaId,
        canonical: NodeId,
    ) -> Result<&'a ArenaNode, OwnerScoreError> {
        if source_arena != self.source_arena {
            return Err(OwnerScoreError::ForeignArena {
                expected: self.source_arena,
                actual: Some(source_arena),
            });
        }
        self.node(canonical)
    }

    pub(super) fn local_position(&self, local: NodeId) -> Result<Option<Point>, OwnerScoreError> {
        self.position(self.canonical_node(local)?)
    }

    pub(super) fn position(&self, canonical: NodeId) -> Result<Option<Point>, OwnerScoreError> {
        self.node(canonical)?;
        Ok(self.owner.position(canonical))
    }

    /// `position` and the node's Box size with one handle validation.
    pub(super) fn position_and_size(
        &self,
        canonical: NodeId,
    ) -> Result<(Option<Point>, Size), OwnerScoreError> {
        let node = self.node(canonical)?;
        Ok((self.owner.position(canonical), node.rect.size))
    }

    pub(super) fn graph_id(
        &self,
        canonical: NodeId,
    ) -> Result<Option<OwnerGraphId>, OwnerScoreError> {
        self.node(canonical)?;
        let owner = self.owner.topology_node_owner(canonical);
        if let Some(owner) = owner {
            self.owner
                .owner_graph_view(owner)
                .ok_or(OwnerTopologyError::UnknownGraph(owner))?;
        }
        Ok(owner)
    }

    pub(super) fn graph(
        &self,
        canonical: NodeId,
    ) -> Result<Option<&'a OwnerGraphView>, OwnerScoreError> {
        Ok(self
            .graph_id(canonical)?
            .map(|owner| self.owner.owner_graph_view(owner).expect("validated graph")))
    }

    pub(super) fn receiver_for_local(
        &self,
        local: NodeId,
    ) -> Result<OwnerGraphId, OwnerScoreError> {
        let canonical = self.canonical_node(local)?;
        self.graph_id(canonical)?
            .ok_or(OwnerScoreError::MissingNodeOwner(canonical))
    }

    pub(super) fn receiver_cell_size(&self, local: NodeId) -> Result<f64, OwnerScoreError> {
        Ok(self
            .owner
            .owner_graph_cell_size(self.receiver_for_local(local)?)?)
    }

    pub(super) fn receiver_turn_cost(&self, local: NodeId) -> Result<f64, OwnerScoreError> {
        Ok(self
            .owner
            .owner_graph_turn_cost(self.receiver_for_local(local)?)?)
    }

    pub(super) fn receiver_crossing_cost(&self, local: NodeId) -> Result<f64, OwnerScoreError> {
        Ok(self
            .owner
            .owner_graph_crossing_cost(self.receiver_for_local(local)?)?)
    }

    /// node.go2419–2420 reads this canonical node's own Graph.Directions and
    /// its live effective container, including a retained foreign original.
    /// CopyEntitiesFrom shares that actual direction map between these views.
    pub(super) fn container_direction(
        &self,
        canonical: NodeId,
    ) -> Result<Orientation, OwnerScoreError> {
        let receiver = self
            .graph_id(canonical)?
            .ok_or(OwnerScoreError::MissingNodeOwner(canonical))?;
        let parent = self.effective_container(Some(canonical))?;
        Ok(self
            .owner
            .owner_graph_direction_entry(receiver, parent)?
            .unwrap_or(Orientation::None))
    }

    /// geometry.go153–157 uses raw Container ancestry, independently of the
    /// active cluster/sequence EffectiveContainer ancestry used elsewhere.
    pub(super) fn raw_container_depth(&self, canonical: NodeId) -> Result<usize, OwnerScoreError> {
        let mut current = Some(canonical);
        let mut depth = 0;
        let mut seen = AncestryVisited::new();
        while let Some(node) = current {
            if !seen.insert(node) {
                return Err(OwnerScoreError::AncestryCycle(node));
            }
            depth += 1;
            current = self.checked_parent(self.node(node)?.container)?;
        }
        Ok(depth)
    }

    /// edge_length.go1188–1194 chooses one outer replacement by raw Container
    /// pointer/depth, then asks that pointer's own Graph for its direction.
    pub(super) fn outer_direction_is_set(
        &self,
        node_replacement: NodeId,
        adjacent_replacement: NodeId,
    ) -> Result<bool, OwnerScoreError> {
        let mut outer = node_replacement;
        if self.node(node_replacement)?.container != self.node(adjacent_replacement)?.container
            && self.raw_container_depth(node_replacement)?
                > self.raw_container_depth(adjacent_replacement)?
        {
            outer = adjacent_replacement;
        }
        Ok(self.container_direction(outer)? != Orientation::None)
    }

    fn vessel_node(&self, vessel: TopologyVessel) -> Result<NodeId, OwnerScoreError> {
        let exists = match vessel {
            TopologyVessel::Cluster(index) => self.owner.clusters.get(index).is_some(),
            TopologyVessel::Sequence(index) => self.owner.sequences.get(index).is_some(),
        };
        if !exists {
            return Err(OwnerTopologyError::InvalidVessel(vessel).into());
        }
        let canonical = self
            .owner
            .topology_vessel_node(vessel)
            .ok_or(OwnerTopologyError::UnboundVessel(vessel))?;
        self.node(canonical)?;
        Ok(canonical)
    }

    /// quality_api.go26–31 and node.go2344–2352. Active means the actual
    /// bound vessel.Graph is nonnil (cluster.go222, sequence.go160), regardless
    /// of position or member ordering. A first member is never a vessel.
    pub(super) fn effective_container(
        &self,
        canonical: Option<NodeId>,
    ) -> Result<Option<NodeId>, OwnerScoreError> {
        let Some(canonical) = canonical else {
            return Ok(None);
        };
        let node = self.node(canonical)?;
        for kind in [
            super::owner_groups::OwnerGroupKind::Cluster,
            super::owner_groups::OwnerGroupKind::Sequence,
        ] {
            if let Some(vessel) = self.owner.owner_node_group_vessel(canonical, kind)? {
                let vessel = vessel
                    .expect("runtime error: invalid memory address or nil pointer dereference");
                if self.graph_id(vessel)?.is_some() {
                    return self.checked_parent(self.node(vessel)?.container);
                }
            }
        }
        self.checked_parent(node.container)
    }

    /// Source edge_length.go:928–933 skips an endpoint's own cluster vessel:
    /// its cluster penalty is added separately. Read the actual raw Cluster
    /// fields, even when that cluster is inactive (Vessel.Graph is nil).
    pub(super) fn is_endpoint_cluster_obstruction(
        &self,
        obstruction: NodeId,
        node_replacement: NodeId,
        adjacent_replacement: NodeId,
    ) -> Result<bool, OwnerScoreError> {
        self.node(obstruction)?;
        if !self.owner.owner_is_cluster_vessel(obstruction) {
            return Ok(false);
        }
        for endpoint in [node_replacement, adjacent_replacement] {
            if self
                .owner
                .owner_node_group_vessel(endpoint, super::owner_groups::OwnerGroupKind::Cluster)?
                == Some(Some(obstruction))
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// placementcost/cluster.go: actual CurrentFrom/CurrentTo pointers, in
    /// header order, with live TopLeft and the first member's Graph identity.
    /// A third unique pointer makes exactlyTwo false but does not stop reads.
    pub(super) fn cluster_external_connected_nodes(
        &self,
        group: super::owner_groups::OwnerGroupId,
    ) -> Result<Option<[NodeId; 2]>, OwnerScoreError> {
        let fields = self.owner.read_owner_group_fields(group)?;
        let first_member = fields
            .nodes
            .get(0)
            .flatten()
            .ok_or(OwnerScoreError::InvalidAbductionSlice)?;
        let current_graph = self.graph_id(first_member)?;
        let policy = self.owner.read_owner_cluster_policy(group)?;
        let mut first = None;
        let mut second = None;
        let mut more_than_two = false;
        if let Some(header) = policy.edge_abductions.as_ref() {
            self.owner.validate_owner_abduction_header(header)?;
            for slot in header.borrowed_slots() {
                let handle = slot.ok_or(OwnerScoreError::InvalidAbductionSlice)?;
                let record = self
                    .owner
                    .owner_edge_abduction(handle)
                    .ok_or(OwnerTopologyError::UnknownRecord(handle))?;
                let external = match (record.originally_from, record.originally_to) {
                    (None, Some(_)) => record
                        .current_from
                        .ok_or(OwnerScoreError::InvalidAbductionSlice)?,
                    (Some(_), None) => record
                        .current_to
                        .ok_or(OwnerScoreError::InvalidAbductionSlice)?,
                    _ => continue,
                };
                if self.position(external)?.is_none()
                    || self.graph_id(external)? != current_graph
                    || Some(external) == first
                    || Some(external) == second
                {
                    continue;
                }
                if first.is_none() {
                    first = Some(external);
                } else if second.is_none() {
                    second = Some(external);
                } else {
                    more_than_two = true;
                }
            }
        }
        Ok(if more_than_two {
            None
        } else {
            first.zip(second).map(|(a, b)| [a, b])
        })
    }

    fn checked_parent(&self, parent: Option<NodeId>) -> Result<Option<NodeId>, OwnerScoreError> {
        if let Some(parent) = parent {
            self.node(parent)?;
        }
        Ok(parent)
    }

    /// layout.go314–331 / topology_preflight.go107–121: direct Container wins,
    /// then Cluster vessel, then Sequence vessel. Group ancestry is through
    /// the vessel itself and does not depend on whether it is active.
    fn ancestry_parent(&self, canonical: NodeId) -> Result<Option<NodeId>, OwnerScoreError> {
        let node = self.node(canonical)?;
        if node.container.is_some() {
            return self.checked_parent(node.container);
        }
        for kind in [
            super::owner_groups::OwnerGroupKind::Cluster,
            super::owner_groups::OwnerGroupKind::Sequence,
        ] {
            if let Some(vessel) = self.owner.owner_node_group_vessel(canonical, kind)? {
                return self.checked_parent(vessel);
            }
        }
        Ok(None)
    }

    pub(super) fn is_descendant_of(
        &self,
        mut descendant: Option<NodeId>,
        ancestor: Option<NodeId>,
    ) -> Result<bool, OwnerScoreError> {
        if let Some(ancestor) = ancestor {
            self.node(ancestor)?;
        }
        let mut seen = AncestryVisited::new();
        loop {
            if descendant == ancestor {
                return Ok(true);
            }
            let Some(canonical) = descendant else {
                return Ok(false);
            };
            if !seen.insert(canonical) {
                return Err(OwnerScoreError::AncestryCycle(canonical));
            }
            descendant = self.ancestry_parent(canonical)?;
        }
    }

    /// The complete walk `is_descendant_of(descendant, _)` performs when no
    /// ancestor matches: the set of visited identities, then how it ended.
    /// Vessel ancestry is live, so a chain is valid only until the next poll.
    /// Records that walk into `chain` unless it already holds one.
    pub(super) fn record_ancestry_chain<'c>(
        &self,
        chain: &'c mut OwnerAncestryChain,
        mut descendant: Option<NodeId>,
    ) -> &'c OwnerAncestryChain {
        if chain.recorded {
            return chain;
        }
        // A replay only asks whether an identity was visited, so the walk's
        // own cycle set is the complete record.
        chain.nodes.clear();
        chain.end = loop {
            let Some(canonical) = descendant else {
                break Ok(());
            };
            if !chain.nodes.insert(canonical) {
                break Err(OwnerScoreError::AncestryCycle(canonical));
            }
            match self.ancestry_parent(canonical) {
                Ok(parent) => descendant = parent,
                Err(error) => break Err(error),
            }
        };
        chain.recorded = true;
        chain
    }

    /// `is_descendant_of(chain's descendant, ancestor)`, replayed from a chain
    /// captured since the last poll. The walk compares each identity before
    /// its cycle check and parent read, so a match precedes the walk's error.
    pub(super) fn chain_is_descendant_of(
        &self,
        chain: &OwnerAncestryChain,
        ancestor: Option<NodeId>,
    ) -> Result<bool, OwnerScoreError> {
        if let Some(ancestor) = ancestor {
            self.node(ancestor)?;
        }
        if ancestor.is_some_and(|ancestor| chain.nodes.contains(ancestor)) {
            return Ok(true);
        }
        match &chain.end {
            Ok(()) => Ok(ancestor.is_none()),
            Err(error) => Err(error.clone()),
        }
    }

    fn container_level(&self, mut current: Option<NodeId>) -> Result<usize, OwnerScoreError> {
        let mut level = 0;
        let mut seen = AncestryVisited::new();
        while let Some(canonical) = current {
            if !seen.insert(canonical) {
                return Err(OwnerScoreError::AncestryCycle(canonical));
            }
            level += 1;
            current = self.effective_container(Some(canonical))?;
        }
        Ok(level)
    }

    /// Direct translation of node.go2380–2410, using canonical live effective
    /// containers. Nil is the common outer ancestor.
    pub(super) fn nearest_shared_ancestor(
        &self,
        left: NodeId,
        right: NodeId,
    ) -> Result<Option<NodeId>, OwnerScoreError> {
        let mut left_container = self.effective_container(Some(left))?;
        let mut right_container = self.effective_container(Some(right))?;
        if left_container.is_none() || right_container.is_none() {
            return Ok(None);
        }
        let mut left_level = self.container_level(Some(left))?;
        let mut right_level = self.container_level(Some(right))?;
        while left_container != right_container {
            if left_level == right_level {
                left_container = self.effective_container(left_container)?;
                left_level -= 1;
                right_container = self.effective_container(right_container)?;
                right_level -= 1;
            } else if left_level > right_level {
                left_container = self.effective_container(left_container)?;
                left_level -= 1;
            } else {
                right_container = self.effective_container(right_container)?;
                right_level -= 1;
            }
            if left_container.is_none() || right_container.is_none() {
                return Ok(None);
            }
        }
        Ok(left_container)
    }

    /// edge_length.go875–901: use the scoring node's current Graph for both
    /// replacement chains, deduplicate keys in encounter order, and retain
    /// each actual lookup header. The caller owns the 903/907 poll sequence.
    pub(super) fn capture_obstructions(
        &self,
        scoring_local: NodeId,
        node_replacement: NodeId,
        adjacent_replacement: NodeId,
    ) -> Result<OwnerObstructionSets, OwnerScoreError> {
        let receiver = self.receiver_for_local(scoring_local)?;
        let map = self
            .owner
            .owner_container_map(receiver)
            .ok_or(OwnerTopologyError::UnknownGraph(receiver))?;
        let ancestor = self.nearest_shared_ancestor(node_replacement, adjacent_replacement)?;
        let mut sets = OwnerObstructionSets::take();
        let mut added = AncestryVisited::new();
        for replacement in [node_replacement, adjacent_replacement] {
            let mut current = self.effective_container(Some(replacement))?;
            loop {
                if !added.insert(current) {
                    break;
                }
                sets.push(OwnerObstructionSet {
                    parent: current,
                    header: map.get_slice(current),
                });
                if current.is_none() {
                    break;
                }
                current = self.effective_container(current)?;
                if current == ancestor {
                    break;
                }
            }
        }
        Ok(sets)
    }

    #[cfg(any(test, feature = "diagnostic-traces"))]
    pub(super) fn node_address(&self, canonical: NodeId) -> Result<usize, OwnerScoreError> {
        Ok(self.node(canonical)? as *const ArenaNode as usize)
    }

    #[cfg(any(test, feature = "diagnostic-traces"))]
    pub(super) fn position_address(
        &self,
        canonical: NodeId,
    ) -> Result<Option<usize>, OwnerScoreError> {
        self.node(canonical)?;
        if let Some(point) = self.owner.owner_bound_point_address(canonical) {
            return Ok(point);
        }
        Ok(self
            .node(canonical)?
            .position
            .as_ref()
            .map(|point| point as *const Point as usize))
    }

    #[cfg(any(test, feature = "diagnostic-traces"))]
    pub(super) fn graph_address(
        &self,
        canonical: NodeId,
    ) -> Result<Option<usize>, OwnerScoreError> {
        Ok(self
            .graph(canonical)?
            .map(|graph| graph as *const OwnerGraphView as usize))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deep_ancestry_retains_nil_precedence_and_first_cycle_identity() {
        let mut input = Graph::default();
        let mut nodes = Vec::new();
        for index in 0..40 {
            let mut node = crate::Node::new(index.to_string(), Size::default());
            node.parent = nodes.last().copied();
            nodes.push(input.add_node(node));
        }
        let mut owner = ArenaGraph::from_input(&input);
        owner.initialize_shared_topology();
        let view = owner
            .create_owner_graph_view(nodes.clone(), Vec::new())
            .unwrap();
        let arena = owner.topology_arena_id().unwrap();
        {
            let frame = OwnerScoreFrame::new(&owner, arena, view, &nodes).unwrap();
            assert_eq!(frame.raw_container_depth(nodes[39]), Ok(40));
            assert_eq!(frame.container_level(Some(nodes[39])), Ok(40));
            assert_eq!(
                frame.is_descendant_of(Some(nodes[39]), Some(nodes[0])),
                Ok(true)
            );
            assert_eq!(frame.is_descendant_of(Some(nodes[39]), None), Ok(true));
            assert_eq!(frame.is_descendant_of(None, Some(nodes[39])), Ok(false));
            assert_eq!(
                frame.nearest_shared_ancestor(nodes[39], nodes[38]),
                Ok(Some(nodes[37]))
            );
        }
        // A cycle beyond the inline prefix must still fail at its first
        // repeated node; an ancestor reached before that repetition wins.
        owner.nodes[nodes[0].0 as usize].container = Some(nodes[20]);
        let frame = OwnerScoreFrame::new(&owner, arena, view, &nodes).unwrap();
        assert_eq!(
            frame.raw_container_depth(nodes[39]),
            Err(OwnerScoreError::AncestryCycle(nodes[20]))
        );
        assert_eq!(
            frame.container_level(Some(nodes[39])),
            Err(OwnerScoreError::AncestryCycle(nodes[20]))
        );
        assert_eq!(
            frame.is_descendant_of(Some(nodes[39]), None),
            Err(OwnerScoreError::AncestryCycle(nodes[20]))
        );
        assert_eq!(
            frame.is_descendant_of(Some(nodes[39]), Some(nodes[0])),
            Ok(true)
        );
    }

    #[test]
    fn mapped_frames_preserve_snapshots_live_membership_and_validation_precedence() {
        let (owner, selected, [c, child_c, _, _, leaf]) = fixture();
        let nodes = [c, leaf];
        let edges = [EdgeId(0)];
        let arena = owner.topology_arena_id().unwrap();
        owner
            .owner_graph_view(selected)
            .unwrap()
            .edges
            .replace_values(&edges);
        let snapshot = OwnerScoreFrame::new(&owner, arena, selected, &nodes).unwrap();
        let mapped =
            OwnerScoreFrame::new_with_edge_mapping(&owner, arena, selected, &nodes, &edges)
                .unwrap();
        let cloned = mapped.clone();
        // A live Graph.Edges replacement does not replace either retained
        // correspondence. Raw graph reads and incident pointers remain live.
        owner
            .owner_graph_view(selected)
            .unwrap()
            .edges
            .replace_values(&[]);
        for frame in [&snapshot, &mapped, &cloned] {
            assert_eq!(frame.local_to_canonical_edges(), edges);
            assert!(frame.graph(child_c).unwrap().unwrap().edges.is_empty());
            assert_eq!(frame.canonical_edge(EdgeId(0)), Ok(EdgeId(0)));
        }
        // Node-map errors still precede edge validation, and the first invalid
        // edge still wins even when Graph.Edges is empty.
        let invalid = [EdgeId(999), EdgeId(998)];
        assert!(matches!(
            OwnerScoreFrame::new_with_edge_mapping(&owner, arena, selected, &[leaf, c], &invalid),
            Err(OwnerScoreError::MappingMismatch(view)) if view == selected
        ));
        assert!(matches!(
            OwnerScoreFrame::new_with_edge_mapping(&owner, arena, selected, &nodes, &invalid),
            Err(OwnerScoreError::Topology(OwnerTopologyError::UnknownEdge(
                EdgeId(999)
            )))
        ));
        owner
            .owner_graph_view(selected)
            .unwrap()
            .edges
            .replace_header(
                super::super::owner_pointer_slices::SharedEdgeSlice::from_slots(&[None]).unwrap(),
            );
        // The old snapshot's nil panic also precedes explicit-edge errors.
        for borrowed in [false, true] {
            let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                if borrowed {
                    OwnerScoreFrame::new_with_edge_mapping(
                        &owner, arena, selected, &nodes, &invalid,
                    )
                } else {
                    OwnerScoreFrame::new(&owner, arena, selected, &nodes)
                        .and_then(|frame| frame.with_edge_mapping(&invalid))
                }
            }))
            .unwrap_err();
            let message = panic
                .downcast_ref::<&str>()
                .copied()
                .or_else(|| panic.downcast_ref::<String>().map(String::as_str));
            assert_eq!(message, Some("nil owner pointer"));
        }
    }

    #[test]
    fn explicit_edge_mapping_retains_incident_pointer_after_graph_edge_membership_clear() {
        let (mut owner, selected, [c, child_c, _, _, leaf]) = fixture();
        let mapping = [c, leaf];
        let edges = [EdgeId(0)];
        let arena = owner.topology_arena_id().unwrap();
        owner
            .set_owner_view_edge_membership(selected, Vec::new())
            .unwrap();
        assert_eq!(owner.nodes[child_c.0 as usize].edges, [EdgeId(0)]);
        let frame = OwnerScoreFrame::new(&owner, arena, selected, &mapping)
            .unwrap()
            .with_edge_mapping(&edges)
            .unwrap();
        assert!(frame.graph(child_c).unwrap().unwrap().edges.is_empty());
        assert_eq!(frame.local_to_canonical_edges(), [EdgeId(0)]);
        assert_eq!(frame.canonical_edge(EdgeId(0)).unwrap(), EdgeId(0));
        assert!(matches!(
            frame.canonical_edge(EdgeId(1)),
            Err(OwnerScoreError::UnknownLocalEdge(EdgeId(1)))
        ));
        assert!(std::ptr::eq(
            frame
                .edge(frame.canonical_edge(EdgeId(0)).unwrap())
                .unwrap(),
            &owner.edge_records[0]
        ));
        let invalid_edges = [EdgeId(999)];
        assert!(matches!(
            OwnerScoreFrame::new(&owner, arena, selected, &mapping)
                .unwrap()
                .with_edge_mapping(&invalid_edges),
            Err(OwnerScoreError::Topology(OwnerTopologyError::UnknownEdge(
                EdgeId(999)
            )))
        ));
    }

    #[test]
    fn receiver_scalars_and_costs_follow_actual_graph_reference_not_mapping_view() {
        let (mut owner, selected, [c, _, _, _, leaf]) = fixture();
        owner.cell_size = 100.0;
        owner.restore_routing_costs((30.0, 50.0, 70.0));
        owner.set_owner_graph_cell_size(selected, 10.0).unwrap();
        owner
            .restore_owner_graph_routing_costs(selected, (3.0, 5.0, 7.0))
            .unwrap();
        let arena = owner.topology_arena_id().unwrap();
        let mapping = [c, leaf];
        {
            let frame = OwnerScoreFrame::new(&owner, arena, selected, &mapping).unwrap();
            assert_eq!(frame.receiver_cell_size(NodeId(0)).unwrap(), 10.0);
            assert_eq!(frame.receiver_turn_cost(NodeId(0)).unwrap(), 5.0);
            assert_eq!(frame.receiver_crossing_cost(NodeId(0)).unwrap(), 3.0);
        }
        owner
            .set_owner_graph_reference(
                OwnerGraphId(0),
                &[c],
                false,
                &mut OwnerGraphOwnershipJournal::default(),
            )
            .unwrap();
        let frame = OwnerScoreFrame::new(&owner, arena, selected, &mapping).unwrap();
        assert_eq!(
            frame.receiver_for_local(NodeId(0)).unwrap(),
            OwnerGraphId(0)
        );
        assert_eq!(frame.receiver_cell_size(NodeId(0)).unwrap(), 100.0);
        assert_eq!(frame.receiver_turn_cost(NodeId(0)).unwrap(), 50.0);
        assert_eq!(frame.receiver_crossing_cost(NodeId(0)).unwrap(), 30.0);
    }

    #[test]
    fn outer_direction_uses_raw_container_depth_and_selected_outer_pointer() {
        let (mut owner, selected, [c, child_c, _, _, leaf]) = fixture();
        owner.directions.clear();
        owner.directions.insert(Some(c), Direction::Down);
        let arena = owner.topology_arena_id().unwrap();
        let mapping = [c, leaf];
        {
            let frame = OwnerScoreFrame::new(&owner, arena, selected, &mapping).unwrap();
            assert_eq!(frame.raw_container_depth(child_c).unwrap(), 2);
            assert_eq!(frame.raw_container_depth(leaf).unwrap(), 1);
            assert_eq!(
                frame.container_direction(child_c).unwrap(),
                Orientation::Bottom
            );
            assert!(!frame.outer_direction_is_set(child_c, leaf).unwrap());
            assert!(!frame.outer_direction_is_set(leaf, child_c).unwrap());
        }
        owner.directions.insert(None, Direction::Left);
        let frame = OwnerScoreFrame::new(&owner, arena, selected, &mapping).unwrap();
        assert!(frame.outer_direction_is_set(child_c, leaf).unwrap());
        assert!(frame.outer_direction_is_set(leaf, child_c).unwrap());
    }

    #[test]
    fn actual_present_none_direction_remains_unset_for_scoring_and_outer_gate() {
        let (mut owner, selected, [c, child_c, _, _, leaf]) = fixture();
        owner
            .set_owner_graph_direction_entry(selected, None, Some(Orientation::None))
            .unwrap();
        owner
            .set_owner_graph_direction_entry(selected, Some(c), Some(Orientation::Right))
            .unwrap();
        let arena = owner.topology_arena_id().unwrap();
        let mapping = [c, leaf];
        {
            let frame = OwnerScoreFrame::new(&owner, arena, selected, &mapping).unwrap();
            assert_eq!(
                owner.owner_graph_direction_entry(selected, None),
                Ok(Some(Orientation::None))
            );
            assert_eq!(frame.container_direction(c).unwrap(), Orientation::None);
            assert_eq!(
                frame.container_direction(child_c).unwrap(),
                Orientation::Right
            );
            assert_eq!(frame.container_direction(leaf).unwrap(), Orientation::None);
            assert!(!frame.outer_direction_is_set(child_c, leaf).unwrap());
            assert!(!frame.outer_direction_is_set(leaf, child_c).unwrap());
        }
        // A direct cardinal edit while an extended entry is present cannot
        // silently override the actual source-shaped map value.
        owner.directions.insert(None, Direction::Left);
        {
            let frame = OwnerScoreFrame::new(&owner, arena, selected, &mapping).unwrap();
            assert_eq!(
                frame.container_direction(c),
                Err(OwnerScoreError::Topology(
                    OwnerTopologyError::DirectionEntryConflict(None)
                ))
            );
        }
        owner
            .set_owner_graph_direction_entry(selected, None, Some(Orientation::BottomRight))
            .unwrap();
        let frame = OwnerScoreFrame::new(&owner, arena, selected, &mapping).unwrap();
        assert_eq!(
            frame.container_direction(c).unwrap(),
            Orientation::BottomRight
        );
        assert!(frame.outer_direction_is_set(child_c, leaf).unwrap());
    }

    fn fixture() -> (ArenaGraph, OwnerGraphId, [NodeId; 5]) {
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
        let mut owner = ArenaGraph::from_input(&input);
        owner.initialize_shared_topology();
        owner.set_position(c, Point { x: 80.0, y: 80.0 });
        owner.set_position(child_c, Point { x: 100.0, y: 100.0 });
        owner.set_position(d, Point { x: 300.0, y: 80.0 });
        owner.set_position(child_d, Point { x: 320.0, y: 100.0 });
        owner.set_position(leaf, Point { x: 380.0, y: 100.0 });
        let selected = owner
            .create_owner_graph_view(vec![c, leaf], Vec::new())
            .unwrap();
        owner
            .set_owner_graph_reference(
                selected,
                &[c, leaf],
                true,
                &mut OwnerGraphOwnershipJournal::default(),
            )
            .unwrap();
        (owner, selected, [c, child_c, d, child_d, leaf])
    }

    #[test]
    fn explicit_namespace_and_exact_mapping_reject_compact_identity_substitution() {
        let (owner, selected, [c, _, d, _, leaf]) = fixture();
        let arena = owner.topology_arena_id().unwrap();
        let mapping = [c, leaf];
        let frame = OwnerScoreFrame::new(&owner, arena, selected, &mapping).unwrap();
        assert!(std::ptr::eq(
            frame
                .node(frame.canonical_node(NodeId(1)).unwrap())
                .unwrap(),
            &owner.nodes[leaf.0 as usize]
        ));
        assert!(matches!(
            frame.canonical_node(NodeId(2)),
            Err(OwnerScoreError::UnknownLocalNode(NodeId(2)))
        ));
        assert!(
            matches!(OwnerScoreFrame::new(&owner, arena, selected, &[c, d]), Err(OwnerScoreError::MappingMismatch(view)) if view == selected)
        );
        let (other, _, _) = fixture();
        let other_arena = other.topology_arena_id().unwrap();
        assert!(matches!(
            OwnerScoreFrame::new(&other, arena, OwnerGraphId(0), &[]),
            Err(OwnerScoreError::ForeignArena { .. })
        ));
        assert!(matches!(
            frame.source_node(other_arena, c),
            Err(OwnerScoreError::ForeignArena { .. })
        ));
    }

    #[test]
    fn foreign_geometry_and_graph_reassignment_resolve_actual_nodes_between_frames() {
        let (mut owner, selected, [c, _, d, child_d, leaf]) = fixture();
        let arena = owner.topology_arena_id().unwrap();
        let mapping = [c, leaf];
        let (node_address, point_address, original_graph_address) = {
            let frame = OwnerScoreFrame::new(&owner, arena, selected, &mapping).unwrap();
            assert_eq!(
                frame.position(d).unwrap(),
                Some(Point { x: 300.0, y: 80.0 })
            );
            assert_eq!(frame.graph_id(d).unwrap(), Some(OwnerGraphId(0)));
            assert!(
                !frame
                    .graph(c)
                    .unwrap()
                    .unwrap()
                    .nodes
                    .iter_slots()
                    .any(|slot| slot == Some(d))
            );
            (
                frame.node_address(d).unwrap(),
                frame.position_address(d).unwrap(),
                frame.graph_address(d).unwrap(),
            )
        };
        owner.move_node_abs_with_children(d, Point { x: 40.0, y: 80.0 });
        {
            let frame = OwnerScoreFrame::new(&owner, arena, selected, &mapping).unwrap();
            assert_eq!(frame.node_address(d).unwrap(), node_address);
            assert_eq!(frame.position_address(d).unwrap(), point_address);
            assert_eq!(frame.position(d).unwrap(), Some(Point { x: 40.0, y: 80.0 }));
            assert_eq!(
                frame.position(child_d).unwrap(),
                Some(Point { x: 60.0, y: 100.0 })
            );
            assert_eq!(frame.graph_address(d).unwrap(), original_graph_address);
        }
        let mut journal = OwnerGraphOwnershipJournal::default();
        owner
            .set_owner_graph_reference(selected, &[d], true, &mut journal)
            .unwrap();
        {
            let frame = OwnerScoreFrame::new(&owner, arena, selected, &mapping).unwrap();
            assert!(std::ptr::eq(
                frame.graph(d).unwrap().unwrap(),
                frame.graph(c).unwrap().unwrap()
            ));
            assert!(
                !frame
                    .graph(d)
                    .unwrap()
                    .unwrap()
                    .nodes
                    .iter_slots()
                    .any(|slot| slot == Some(d))
            );
            assert_eq!(frame.graph_id(child_d).unwrap(), Some(selected));
            assert_eq!(frame.node_address(d).unwrap(), node_address);
            assert_eq!(frame.position_address(d).unwrap(), point_address);
        }
        owner.restore_owner_graph_ownership(&journal);
        let frame = OwnerScoreFrame::new(&owner, arena, selected, &mapping).unwrap();
        assert_eq!(frame.graph_address(d).unwrap(), original_graph_address);
        assert_eq!(frame.position(d).unwrap(), Some(Point { x: 40.0, y: 80.0 }));
    }

    #[test]
    fn retained_obstruction_header_reads_original_backing_after_map_replacement() {
        let (owner, selected, [c, _, d, child_d, leaf]) = fixture();
        let mapping = [c, leaf];
        let frame = OwnerScoreFrame::new(
            &owner,
            owner.topology_arena_id().unwrap(),
            selected,
            &mapping,
        )
        .unwrap();
        let sets = frame.capture_obstructions(NodeId(0), c, leaf).unwrap();
        assert_eq!(sets.len(), 1);
        assert_eq!(sets[0].parent, None);
        assert!(sets[0].key_present());
        assert_eq!(sets[0].raw_slot(1), Some(Some(d)));
        let actual_header = owner.containers.get_header(&None).unwrap();
        actual_header.write_capacity_slot(1, Some(child_d)).unwrap();
        assert_eq!(sets[0].raw_slot(1), Some(Some(child_d)));
        owner.containers.insert(None, vec![leaf]);
        assert_eq!(owner.containers.nodes(&None), [leaf]);
        assert_eq!(sets[0].len(), 3);
        assert_eq!(sets[0].raw_slot(1), Some(Some(child_d)));
        assert_eq!(
            frame.capture_obstructions(NodeId(0), c, leaf).unwrap()[0].raw_slot(0),
            Some(Some(leaf))
        );
        // Reusing a returned scratch buffer must neither append stale sets
        // nor replace a still-active outer capture's original header.
        for _ in 0..3 {
            let current = frame.capture_obstructions(NodeId(0), c, leaf).unwrap();
            assert_eq!(current.len(), 1);
            assert_eq!(current[0].len(), 1);
            assert_eq!(current[0].raw_slot(0), Some(Some(leaf)));
            assert_eq!(sets[0].len(), 3);
            assert_eq!(sets[0].raw_slot(1), Some(Some(child_d)));
        }
    }

    #[test]
    fn nil_slots_preserve_their_indices_and_source_universal_ancestor_predicate() {
        let (owner, selected, [c, _, _, _, leaf]) = fixture();
        let mapping = [c, leaf];
        owner
            .containers
            .get_header(&None)
            .unwrap()
            .write_capacity_slot(1, None)
            .unwrap();
        let frame = OwnerScoreFrame::new(
            &owner,
            owner.topology_arena_id().unwrap(),
            selected,
            &mapping,
        )
        .unwrap();
        let sets = frame.capture_obstructions(NodeId(0), c, leaf).unwrap();
        assert_eq!(sets[0].len(), 3);
        assert_eq!(sets[0].raw_slot(1), Some(None));
        assert_eq!(sets[0].raw_slot(3), None);
        assert!(
            frame
                .is_descendant_of(Some(c), sets[0].raw_slot(1).unwrap())
                .unwrap()
        );
        assert!(frame.is_descendant_of(None, None).unwrap());
        assert!(!frame.is_descendant_of(None, Some(c)).unwrap());
    }

    #[test]
    fn absent_present_nil_and_allocated_empty_lookups_remain_distinct() {
        let (mut owner, selected, [c, _, _, _, leaf]) = fixture();
        let mapping = [c, leaf];
        let arena = owner.topology_arena_id().unwrap();
        owner.set_topology_container_slice(None, None);
        let present_nil = OwnerScoreFrame::new(&owner, arena, selected, &mapping)
            .unwrap()
            .capture_obstructions(NodeId(0), c, leaf)
            .unwrap()
            .remove(0);
        assert!(present_nil.key_present() && present_nil.is_nil());
        assert_eq!(present_nil.len(), 0);
        owner.containers.remove(&None);
        let absent = OwnerScoreFrame::new(&owner, arena, selected, &mapping)
            .unwrap()
            .capture_obstructions(NodeId(0), c, leaf)
            .unwrap()
            .remove(0);
        assert!(!absent.key_present());
        assert_eq!(absent.len(), 0);
        owner.containers.insert(None, Vec::new());
        let empty = OwnerScoreFrame::new(&owner, arena, selected, &mapping)
            .unwrap()
            .capture_obstructions(NodeId(0), c, leaf)
            .unwrap()
            .remove(0);
        assert!(empty.key_present() && !empty.is_nil());
        assert_eq!(empty.len(), 0);
        assert_eq!(empty.capacity(), 0);
    }

    #[test]
    fn source_chain_order_uses_effective_containers_and_shared_ancestor_stop() {
        let (owner, selected, [c, child_c, d, child_d, leaf]) = fixture();
        let mapping = [c, leaf];
        let frame = OwnerScoreFrame::new(
            &owner,
            owner.topology_arena_id().unwrap(),
            selected,
            &mapping,
        )
        .unwrap();
        assert_eq!(
            frame.nearest_shared_ancestor(child_c, child_c).unwrap(),
            Some(c)
        );
        assert_eq!(
            frame.nearest_shared_ancestor(child_c, child_d).unwrap(),
            None
        );
        let sets = frame
            .capture_obstructions(NodeId(0), child_c, child_d)
            .unwrap();
        assert_eq!(
            sets.iter().map(|set| set.parent).collect::<Vec<_>>(),
            [Some(c), Some(d)]
        );
        assert_eq!(sets[0].raw_slot(0), Some(Some(child_c)));
        assert_eq!(sets[1].raw_slot(0), Some(Some(child_d)));
    }

    #[test]
    fn aggregate_container_requires_distinct_bound_vessel_and_actual_graph_activity() {
        let (mut owner, selected, [c, _, d, _, leaf]) = fixture();
        owner.clusters.push(ClusterState {
            members: vec![c],
            arrangement: ClusterArrangement::Row,
            desired_arrangement: ClusterArrangement::Row,
            padding: 0.0,
            vessel_tala_id: 10_001,
            fixed_size: false,
        });
        owner.nodes[c.0 as usize].cluster = Some(0);
        let arena = owner.topology_arena_id().unwrap();
        let mapping = [c, leaf];
        {
            let frame = OwnerScoreFrame::new(&owner, arena, selected, &mapping).unwrap();
            assert!(matches!(
                frame.effective_container(Some(c)),
                Err(OwnerScoreError::Topology(
                    OwnerTopologyError::UnboundVessel(TopologyVessel::Cluster(0))
                ))
            ));
        }
        let mut vessel_entity = owner.nodes[d.0 as usize].clone();
        vessel_entity.tala_id = 10_001;
        vessel_entity.external_id = "actual-vessel".into();
        vessel_entity.container = Some(leaf);
        vessel_entity.position = None;
        let vessel = owner.retain_topology_node(vessel_entity);
        owner
            .bind_topology_vessel(TopologyVessel::Cluster(0), vessel)
            .unwrap();
        {
            let frame = OwnerScoreFrame::new(&owner, arena, selected, &mapping).unwrap();
            assert_eq!(frame.effective_container(Some(c)).unwrap(), None);
            assert!(frame.is_descendant_of(Some(c), Some(vessel)).unwrap());
        }
        owner
            .set_owner_graph_reference(
                selected,
                &[vessel],
                false,
                &mut OwnerGraphOwnershipJournal::default(),
            )
            .unwrap();
        let frame = OwnerScoreFrame::new(&owner, arena, selected, &mapping).unwrap();
        assert_eq!(frame.position(vessel).unwrap(), None);
        assert_eq!(frame.effective_container(Some(c)).unwrap(), Some(leaf));
        assert!(std::ptr::eq(
            frame.node(vessel).unwrap(),
            &owner.nodes[vessel.0 as usize]
        ));
        assert!(
            !frame
                .graph(vessel)
                .unwrap()
                .unwrap()
                .nodes
                .iter_slots()
                .any(|slot| slot == Some(vessel))
        );
    }

    #[test]
    fn cluster_external_pointer_order_reads_live_records_graph_and_top_left() {
        let (mut owner, selected, [c, child_c, d, child_d, leaf]) = fixture();
        let group = owner.new_owner_group(super::super::owner_groups::OwnerGroupKind::Cluster);
        let nodes = owner.allocate_owner_member_header(&[Some(c)], 1).unwrap();
        owner
            .set_owner_group_fields(
                group,
                super::super::shared_topology::OwnerGroupFieldView {
                    vessel: Some(d),
                    nodes,
                    graph: Some(selected),
                    container: None,
                },
            )
            .unwrap();
        let record = |external| OwnerEdgeAbduction {
            source_edge: Some(EdgeId(0)),
            originally_from: Some(c),
            originally_to: None,
            current_from: Some(d),
            current_to: Some(external),
        };
        let records = owner
            .register_owner_edge_abductions(
                vec![record(leaf), record(child_c), record(child_d)],
                true,
            )
            .unwrap();
        let header = owner
            .allocate_owner_abduction_header(
                &[
                    Some(records.records[0]),
                    Some(records.records[0]),
                    Some(records.records[1]),
                    Some(records.records[2]),
                ],
                4,
                true,
            )
            .unwrap();
        owner
            .set_owner_cluster_policy(
                group,
                super::super::owner_groups::OwnerClusterPolicy {
                    edge_abductions: Some(header),
                    ..Default::default()
                },
            )
            .unwrap();
        let mapping = [c, leaf];
        let read = |owner: &ArenaGraph| {
            OwnerScoreFrame::new(
                owner,
                owner.topology_arena_id().unwrap(),
                selected,
                &mapping,
            )
            .unwrap()
            .cluster_external_connected_nodes(group)
            .unwrap()
        };
        // Duplicate pointer is ignored; positioned foreign Graph is excluded.
        assert_eq!(read(&owner), Some([leaf, child_c]));
        owner
            .set_owner_graph_reference(
                selected,
                &[child_d],
                false,
                &mut OwnerGraphOwnershipJournal::default(),
            )
            .unwrap();
        assert_eq!(read(&owner), None); // three distinct current pointers
        owner.clear_position(child_d);
        assert_eq!(read(&owner), Some([leaf, child_c]));
        owner
            .set_owner_edge_abduction(records.records[1], record(leaf))
            .unwrap();
        assert_eq!(read(&owner), None); // record identity retained, current pointer changed
        owner.set_position(child_d, Point { x: 0.0, y: 0.0 });
        assert_eq!(read(&owner), Some([leaf, child_d]));
        let mut invalid = record(leaf);
        invalid.current_to = None;
        owner
            .set_owner_edge_abduction(records.records[0], invalid)
            .unwrap();
        let frame = OwnerScoreFrame::new(
            &owner,
            owner.topology_arena_id().unwrap(),
            selected,
            &mapping,
        )
        .unwrap();
        assert_eq!(
            frame.cluster_external_connected_nodes(group),
            Err(OwnerScoreError::InvalidAbductionSlice)
        );
    }

    #[test]
    fn actual_ordered_abduction_slice_is_borrowed_and_validated_without_record_copies() {
        let (mut owner, selected, [c, child_c, _, _, leaf]) = fixture();
        let arena = owner.topology_arena_id().unwrap();
        let mapping = [c, leaf];
        let mut slice = owner
            .register_owner_edge_abductions(
                vec![OwnerEdgeAbduction {
                    source_edge: Some(EdgeId(0)),
                    originally_from: Some(child_c),
                    originally_to: None,
                    current_from: Some(c),
                    current_to: Some(leaf),
                }],
                true,
            )
            .unwrap();
        slice.records.push(slice.records[0]);
        let frame = OwnerScoreFrame::new(&owner, arena, selected, &mapping)
            .unwrap()
            .with_abductions(&slice)
            .unwrap();
        assert!(std::ptr::eq(frame.abductions().unwrap(), &slice));
        assert_eq!(
            frame.abductions().unwrap().records,
            [slice.records[0], slice.records[0]]
        );
        assert!(std::ptr::eq(
            frame.edge(EdgeId(0)).unwrap(),
            &owner.edge_records[0]
        ));
        let invalid_nil = OwnerAbductionSlice {
            present: false,
            ..slice.clone()
        };
        assert!(matches!(
            OwnerScoreFrame::new(&owner, arena, selected, &mapping)
                .unwrap()
                .with_abductions(&invalid_nil),
            Err(OwnerScoreError::InvalidAbductionSlice)
        ));
        let unknown = OwnerAbductionSlice {
            records: vec![OwnerAbductionId(999)],
            ..slice.clone()
        };
        assert!(matches!(
            OwnerScoreFrame::new(&owner, arena, selected, &mapping)
                .unwrap()
                .with_abductions(&unknown),
            Err(OwnerScoreError::Topology(
                OwnerTopologyError::UnknownRecord(OwnerAbductionId(999))
            ))
        ));
        let nil = OwnerAbductionSlice::default();
        assert!(
            OwnerScoreFrame::new(&owner, arena, selected, &mapping)
                .unwrap()
                .with_abductions(&nil)
                .unwrap()
                .abductions()
                .is_some()
        );
        let present_empty = OwnerAbductionSlice {
            records: Vec::new(),
            present: true,
            arena: Some(arena),
        };
        assert!(
            OwnerScoreFrame::new(&owner, arena, selected, &mapping)
                .unwrap()
                .with_abductions(&present_empty)
                .is_ok()
        );
        let foreign_empty = OwnerAbductionSlice {
            arena: Some(OwnerArenaId(arena.0 + 1)),
            ..present_empty
        };
        assert!(matches!(
            OwnerScoreFrame::new(&owner, arena, selected, &mapping)
                .unwrap()
                .with_abductions(&foreign_empty),
            Err(OwnerScoreError::ForeignArena { .. })
        ));
    }

    #[test]
    fn borrowed_frame_and_retained_headers_keep_safe_parallel_read_traits() {
        fn require_send_sync<T: Send + Sync>() {}
        require_send_sync::<OwnerScoreFrame<'static>>();
        require_send_sync::<OwnerObstructionSet>();
    }
}
