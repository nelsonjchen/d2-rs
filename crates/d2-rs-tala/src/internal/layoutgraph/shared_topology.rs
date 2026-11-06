// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Canonical owning entities and source-shaped edge/graph owner operations.
//!
//! D2 v0.9.0 layoutgraph/layout.go61–132,163–311, edge.go237–248 and
//! node.go271–277 (SetGraphReference) and 1068–1080 (incidence) mutate shared
//! entities independently of Graph.Nodes.
//! Every NodeId/EdgeId below belongs to this owning arena. Compact component
//! indices are never accepted as substitutes for their source identities.

use super::container_slices::SharedNodeSliceSnapshot;
use super::owner_groups::{
    OwnerGroupFields, OwnerGroupId, OwnerGroupKind, OwnerGroupRecord, SharedOwnerGroupMap,
    SharedOwnerGroupMapField, SharedOwnerGroupMapSnapshot,
};
use super::owner_pointer_slices::{
    SharedAbductionSlice, SharedEdgeSlice, SharedEdgeSliceSnapshot, SharedPointSlice,
    SharedPointSliceSnapshot,
};
use super::owner_points::SharedOwnerPoint;
use super::owner_tree_map::{
    OwnerTreeId, OwnerTreeRecord, OwnerTreeSlice, SharedOwnerTreeMap, SharedOwnerTreeMapSnapshot,
    SharedOwnerTreeRootsMap, SharedOwnerTreeRootsMapSnapshot,
};
use super::*;
use std::fmt;

#[path = "topology_preflight.rs"]
mod topology_preflight;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Ord, PartialOrd)]
pub(super) struct OwnerArenaId(pub(super) u64);

static NEXT_OWNER_ARENA: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn allocate_owner_arena() -> OwnerArenaId {
    OwnerArenaId(
        NEXT_OWNER_ARENA
            .fetch_update(
                std::sync::atomic::Ordering::Relaxed,
                std::sync::atomic::Ordering::Relaxed,
                |next| next.checked_add(1),
            )
            .expect("canonical owner arena identity overflow"),
    )
}

/// Namespace identity only: actual member, point and map storage remains in Arc.
/// Retained headers and transaction snapshots copy this value; a numerical fork
/// allocates a new one even though its canonical arena and entity IDs match.
#[derive(Clone, Copy, Eq, PartialEq)]
struct MemberScopeId(u64);

static NEXT_MEMBER_SCOPE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

impl Default for MemberScopeId {
    fn default() -> Self {
        Self(
            NEXT_MEMBER_SCOPE
                .fetch_update(
                    std::sync::atomic::Ordering::Relaxed,
                    std::sync::atomic::Ordering::Relaxed,
                    |next| next.checked_add(1),
                )
                .expect("canonical member scope identity overflow"),
        )
    }
}

impl MemberScopeId {
    fn same_scope(&self, other: &Self) -> bool {
        self == other
    }
}

impl fmt::Debug for MemberScopeId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Preserve the previous Arc<()> debug representation; namespace IDs
        // are process-local authority and must not enter diagnostic output.
        ().fmt(formatter)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(super) struct OwnerGraphId(pub(super) usize);

/// Retained address of an actual Node.Graph field, independent of its value.
/// Trusted callers use graph IDs registered in the owning canonical arena.
#[derive(Clone, Debug)]
pub(super) struct OwnerNodeGraphField(Arc<crate::internal::go_cell::GoCell<Option<OwnerGraphId>>>);
impl OwnerNodeGraphField {
    fn new(value: Option<OwnerGraphId>) -> Self {
        Self(Arc::new(crate::internal::go_cell::GoCell::new(value)))
    }
    pub(super) fn get(&self) -> Option<OwnerGraphId> {
        *self.0.read().expect("Node.Graph field")
    }
    pub(super) fn set(&self, value: Option<OwnerGraphId>) {
        *self.0.write().expect("Node.Graph field") = value;
    }
    pub(super) fn same_field(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
    fn fork(&self) -> Self {
        Self::new(self.get())
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Ord, PartialOrd)]
pub(super) struct OwnerAbductionId(pub(super) usize);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(super) enum TopologyVessel {
    Cluster(usize),
    Sequence(usize),
}

#[path = "vessel_map.rs"]
mod vessel_map;
use vessel_map::VesselMap;

/// Actual Graph.Nodes field. Reading a header copies its slice descriptor;
/// replacing this field does not retarget an already-retained range header.
#[derive(Clone, Debug)]
pub(super) struct OwnerGraphNodesField(Arc<crate::internal::go_cell::GoCell<SharedNodeSlice>>);
impl OwnerGraphNodesField {
    fn new(header: SharedNodeSlice) -> Self {
        Self(Arc::new(crate::internal::go_cell::GoCell::new(header)))
    }
    /// A current-field read view. Drop it before callbacks or field replacement;
    /// clone the header explicitly only for a range that crosses that boundary.
    pub(super) fn read(&self) -> crate::internal::go_cell::GoCellRef<'_, SharedNodeSlice> {
        self.0.read().expect("Graph.Nodes field")
    }
    pub(super) fn header(&self) -> SharedNodeSlice {
        self.read().clone()
    }
    pub(super) fn replace(&self, header: SharedNodeSlice) {
        *self.0.write().expect("Graph.Nodes field") = header;
    }
    pub(super) fn same_field(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
    pub(super) fn len(&self) -> usize {
        self.read().len()
    }
    pub(super) fn capacity(&self) -> usize {
        self.read().capacity()
    }
    pub(super) fn is_nil(&self) -> bool {
        self.read().is_nil()
    }
    pub(super) fn is_empty(&self) -> bool {
        self.read().is_empty()
    }
    pub(super) fn iter_slots(&self) -> impl Iterator<Item = Option<NodeId>> + use<> {
        self.header().iter_slots()
    }
    pub(super) fn require_nodes(&self) -> impl Iterator<Item = NodeId> + use<> {
        self.header().require_nodes()
    }
    pub(super) fn snapshot(&self) -> SharedNodeSliceSnapshot {
        self.header().snapshot()
    }
    pub(super) fn visible_slots(&self) -> Vec<Option<NodeId>> {
        self.read().visible_slots()
    }
}

#[derive(Debug)]
pub(super) struct OwnerContainerMapField {
    arena: OwnerArenaId,
    scope: MemberScopeId,
    field: GraphNodeMap,
}
impl Clone for OwnerContainerMapField {
    fn clone(&self) -> Self {
        Self {
            arena: self.arena,
            scope: self.scope,
            field: self.field.share_graph(),
        }
    }
}
impl OwnerContainerMapField {
    pub(super) fn header(&self) -> OwnerContainerMap {
        OwnerContainerMap {
            arena: self.arena,
            scope: self.scope,
            slices: self.field.header(),
        }
    }
    pub(super) fn replace(&self, map: OwnerContainerMap) -> Result<(), OwnerTopologyError> {
        if self.arena != map.arena {
            return Err(OwnerTopologyError::ForeignArena {
                expected: Some(self.arena),
                actual: Some(map.arena),
            });
        }
        if !MemberScopeId::same_scope(&self.scope, &map.scope) {
            return Err(OwnerTopologyError::ForeignContainerMap);
        }
        self.field.replace_header(map.slices);
        Ok(())
    }
    pub(super) fn same_field(&self, other: &Self) -> bool {
        self.field.same_field(&other.field)
    }
}

#[derive(Clone, Debug)]
pub(super) struct OwnerGraphView {
    /// Actual member slices; retained descendants do not become members.
    pub(super) nodes: OwnerGraphNodesField,
    pub(super) edges: GraphEdges,
    pub(super) common_uncle_siblings: GraphNodeMap,
    containers: Option<GraphNodeMap>,
    pub(super) hubs: GraphNodeMap,
    /// Source NewGraph leaves this map nil. CopyEntitiesFrom explicitly shares
    /// it; its selected receiver is independent of each Node.Graph pointer.
    node_to_tree: SharedOwnerTreeMap,
    /// NewGraph allocates this map independently. CopyEntitiesFrom shares
    /// its map header; each retained value is a live Tree slice header.
    tree_roots: SharedOwnerTreeRootsMap,
    clusters: SharedOwnerGroupMapField,
    sequences: SharedOwnerGroupMapField,
    /// Dense storage mapping is separate from the member slice: a retained
    /// entity can have a local slot while remaining absent from Graph.Nodes.
    pub(super) local_to_canonical: Vec<NodeId>,
    /// NewGraph creates independent scalar/cost state. The original view has
    /// None and always resolves the actual ArenaGraph fields instead of an
    /// initialization-time copy. CopyEntitiesFrom does not copy these values.
    score_state: Option<OwnerGraphScoreState>,
    /// None retains the legacy shared entity table; Some is an actual map header.
    directions: Option<OwnerGraphDirectionHeader>,
}

#[derive(Clone, Debug)]
struct OwnerGraphScoreState {
    cell_size: f64,
    /// Ordinary clones are geometry trials of the same Graph; independent
    /// owner forks explicitly detach this owner with detach_cost_owner.
    costs: Arc<GraphCostCache>,
}

impl Default for OwnerGraphScoreState {
    fn default() -> Self {
        Self {
            cell_size: 0.0,
            costs: Arc::new(GraphCostCache::new((0.0, 0.0, 0.0))),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct OwnerEdgeAbduction {
    pub(super) source_edge: Option<EdgeId>,
    pub(super) originally_from: Option<NodeId>,
    pub(super) originally_to: Option<NodeId>,
    pub(super) current_from: Option<NodeId>,
    pub(super) current_to: Option<NodeId>,
}

/// Go slices contain pointers to records. Handles preserve that identity
/// when a remaining slice is produced or an original pointer becomes a vessel.
#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub(super) struct OwnerAbductionSlice {
    pub(super) records: Vec<OwnerAbductionId>,
    pub(super) present: bool,
    pub(super) arena: Option<OwnerArenaId>,
}

#[derive(Clone, Debug, Default)]
pub(super) struct OwnerGraphOwnershipJournal {
    /// First pre-operation owner wins, including a genuine nil Graph pointer.
    prior: BTreeMap<NodeId, Option<OwnerGraphId>>,
    arena: Option<OwnerArenaId>,
}

impl OwnerGraphOwnershipJournal {
    fn bind_arena(&mut self, arena: Option<OwnerArenaId>) -> Result<(), OwnerTopologyError> {
        if let Some(existing) = self.arena {
            if Some(existing) != arena {
                return Err(OwnerTopologyError::ForeignArena {
                    expected: arena,
                    actual: self.arena,
                });
            }
        } else {
            self.arena = arena;
        }
        Ok(())
    }
    pub(super) fn original_owner(&self, node: NodeId) -> Option<Option<OwnerGraphId>> {
        self.prior.get(&node).copied()
    }

    pub(super) fn len(&self) -> usize {
        self.prior.len()
    }
}

/// Bound Node.TopLeft fields by dense node id: `None` is an unbound slot,
/// `Some(None)` a bound nil point. Every position read probes it.
#[derive(Clone, Debug, Default)]
pub(super) struct NodePointMap {
    slots: Vec<Option<Option<SharedOwnerPoint>>>,
}
impl NodePointMap {
    pub(super) fn get(&self, node: &NodeId) -> Option<&Option<SharedOwnerPoint>> {
        self.slots.get(node.0 as usize)?.as_ref()
    }
    pub(super) fn get_mut(&mut self, node: &NodeId) -> Option<&mut Option<SharedOwnerPoint>> {
        self.slots.get_mut(node.0 as usize)?.as_mut()
    }
    pub(super) fn contains_key(&self, node: &NodeId) -> bool {
        self.get(node).is_some()
    }
    pub(super) fn insert(
        &mut self,
        node: NodeId,
        point: Option<SharedOwnerPoint>,
    ) -> Option<Option<SharedOwnerPoint>> {
        let index = node.0 as usize;
        if index >= self.slots.len() {
            self.slots.resize_with(index + 1, || None);
        }
        self.slots[index].replace(point)
    }
    pub(super) fn remove(&mut self, node: &NodeId) -> Option<Option<SharedOwnerPoint>> {
        self.slots.get_mut(node.0 as usize)?.take()
    }
    pub(super) fn iter(&self) -> impl Iterator<Item = (NodeId, &Option<SharedOwnerPoint>)> {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(index, slot)| Some((NodeId(index as u32), slot.as_ref()?)))
    }
    pub(super) fn values_mut(&mut self) -> impl Iterator<Item = &mut Option<SharedOwnerPoint>> {
        self.slots.iter_mut().flatten()
    }
}
impl Extend<(NodeId, Option<SharedOwnerPoint>)> for NodePointMap {
    fn extend<I: IntoIterator<Item = (NodeId, Option<SharedOwnerPoint>)>>(&mut self, points: I) {
        for (node, point) in points {
            self.insert(node, point);
        }
    }
}

/// Node.Cluster / Node.Sequence group fields by dense node id, one slot per
/// kind: `None` is absent, `Some(None)` a present nil group.
#[derive(Clone, Debug, Default)]
pub(super) struct NodeGroupMap {
    slots: Vec<[Option<Option<OwnerGroupId>>; 2]>,
}
impl NodeGroupMap {
    fn kind_index(kind: OwnerGroupKind) -> usize {
        match kind {
            OwnerGroupKind::Cluster => 0,
            OwnerGroupKind::Sequence => 1,
        }
    }
    pub(super) fn get(
        &self,
        &(node, kind): &(NodeId, OwnerGroupKind),
    ) -> Option<&Option<OwnerGroupId>> {
        self.slots.get(node.0 as usize)?[Self::kind_index(kind)].as_ref()
    }
    pub(super) fn insert(
        &mut self,
        (node, kind): (NodeId, OwnerGroupKind),
        group: Option<OwnerGroupId>,
    ) -> Option<Option<OwnerGroupId>> {
        let index = node.0 as usize;
        if index >= self.slots.len() {
            self.slots.resize(index + 1, [None, None]);
        }
        self.slots[index][Self::kind_index(kind)].replace(group)
    }
    pub(super) fn iter(
        &self,
    ) -> impl Iterator<Item = ((NodeId, OwnerGroupKind), &Option<OwnerGroupId>)> {
        self.slots.iter().enumerate().flat_map(|(index, slots)| {
            [OwnerGroupKind::Cluster, OwnerGroupKind::Sequence]
                .into_iter()
                .zip(slots)
                .filter_map(move |(kind, slot)| {
                    Some(((NodeId(index as u32), kind), slot.as_ref()?))
                })
        })
    }
}
impl Extend<((NodeId, OwnerGroupKind), Option<OwnerGroupId>)> for NodeGroupMap {
    fn extend<I: IntoIterator<Item = ((NodeId, OwnerGroupKind), Option<OwnerGroupId>)>>(
        &mut self,
        groups: I,
    ) {
        for (key, group) in groups {
            self.insert(key, group);
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct SharedTopology {
    initialized: bool,
    arena: Option<OwnerArenaId>,
    views: Vec<Option<OwnerGraphView>>,
    // Numerical candidates preserve canonical arena IDs but own different
    // member/map allocations. Trusted headers cannot cross that boundary.
    member_scope: MemberScopeId,
    node_owners: Vec<OwnerNodeGraphField>,
    /// None marks an auxiliary runtime entity with no D2 input-node slot.
    source_input_nodes: Vec<Option<NodeId>>,
    vessels: VesselMap<Option<NodeId>>,
    /// Bound legacy references resolve these actual group fields. Their old
    /// Vessel entry is removed, leaving only one authority for that field.
    group_bindings: VesselMap<OwnerGroupId>,
    /// Explicit canonical Node.Cluster/Sequence fields override the legacy
    /// component-index adapter, including a genuine nil pointer.
    // Lookup-only: its one iteration is re-inserted into this map.
    node_groups: NodeGroupMap,
    /// Source private Node.isClusterVessel, independent of group map contents.
    cluster_vessel_flags: BTreeSet<NodeId>,
    groups: Vec<(OwnerGroupId, Arc<OwnerGroupRecord>)>,
    /// Explicit Node.HerdAssignment pointers; absent means legacy nil/unbound.
    node_herds: BTreeMap<NodeId, Option<OwnerHerd>>,
    /// Actual Node.LongDistanceNeighborRequirements headers and full values.
    pub(super) long_distance_requirements:
        BTreeMap<NodeId, super::node_requirements_map::NodeRequirementsMap>,
    /// Explicit Node.TopLeft fields; a bound field has no inline position
    /// authority. Absence means the older inline representation is unbound.
    // Lookup-heavy; its iterations are order-independent (allocation detach,
    // re-insertion) or diagnostic and sorted there.
    points: NodePointMap,
    /// Only nil endpoints need an extension; nonnil endpoints resolve live
    /// ArenaEdge fields. These disjoint representations have one authority.
    nil_edge_endpoints: BTreeSet<(EdgeId, bool)>,
    records: Vec<Option<OwnerEdgeAbduction>>,
    /// Real mutable NewTree objects remain allocated while retained map
    /// handles can refer to them, including after topology map restoration.
    trees: Vec<(OwnerTreeId, Arc<OwnerTreeRecord>)>,
    /// Graph.Directions can contain geo.NONE and diagonal orientations even
    /// though the public D2 input Direction enum has only four compass values.
    /// Together with ArenaGraph.directions these are one disjoint live map.
    /// Owner views created through CopyEntitiesFrom observe that same map.
    direction_extensions: BTreeMap<Option<NodeId>, Orientation>,
    /// Source Graph.Directions contains requested directions only. The
    /// older cardinal carrier also holds implicit numeric layout hints;
    /// production owner views must never score those hints as requests.
    placement_directions: Option<BTreeMap<Option<NodeId>, Orientation>>,
}

impl SharedTopology {
    /// A topology transaction retains the actual headers and their original
    /// backing; unlike a numerical candidate, it must repair those aliases.
    fn clone_with_shared_member_headers(&self) -> Self {
        Self {
            initialized: self.initialized,
            arena: self.arena,
            views: self.views.clone(),
            member_scope: self.member_scope,
            node_owners: self.node_owners.clone(),
            source_input_nodes: self.source_input_nodes.clone(),
            vessels: self.vessels.clone(),
            group_bindings: self.group_bindings.clone(),
            node_groups: self.node_groups.clone(),
            cluster_vessel_flags: self.cluster_vessel_flags.clone(),
            groups: self.groups.clone(),
            node_herds: self.node_herds.clone(),
            long_distance_requirements: self
                .long_distance_requirements
                .iter()
                .map(|(&node, keys)| (node, keys.retained()))
                .collect(),
            points: self.points.clone(),
            nil_edge_endpoints: self.nil_edge_endpoints.clone(),
            records: self.records.clone(),
            trees: self.trees.clone(),
            direction_extensions: self.direction_extensions.clone(),
            placement_directions: self.placement_directions.clone(),
        }
    }
}

impl Clone for SharedTopology {
    fn clone(&self) -> Self {
        self.clone_with_routes(&[], &[]).0
    }
}

impl SharedTopology {
    pub(super) fn clone_with_routes(
        &self,
        routes: &[SharedPointSlice],
        external_maps: &[SharedSliceMap],
    ) -> (Self, Vec<SharedPointSlice>, Vec<SharedSliceMap>) {
        let mut copied = self.clone_with_shared_member_headers();
        copied.node_owners = self
            .node_owners
            .iter()
            .map(OwnerNodeGraphField::fork)
            .collect();
        let mut direction_maps = BTreeMap::new();
        for view in copied.views.iter_mut().flatten() {
            if let Some(header) = view.directions.as_mut() {
                *header = header.fork(&mut direction_maps);
            }
        }
        let mut detached_points = BTreeMap::<usize, SharedOwnerPoint>::new();
        for point in copied.points.values_mut().flatten() {
            *point = detached_points
                .entry(point.allocation_key())
                .or_insert_with(|| SharedOwnerPoint::new(point.get()))
                .clone();
        }
        let detached_routes = SharedPointSlice::fork_storage_batch_map(routes, |point| {
            detached_points
                .entry(point.allocation_key())
                .or_insert_with(|| SharedOwnerPoint::new(point.get()))
                .clone()
        })
        .expect("numerical route backing");
        let edge_headers: Vec<_> = self
            .views
            .iter()
            .flatten()
            .map(|view| view.edges.header())
            .collect();
        let mut detached_edges = SharedEdgeSlice::fork_storage_batch(&edge_headers)
            .expect("numerical raw edge backing")
            .into_iter();
        for view in copied.views.iter_mut().flatten() {
            view.edges
                .replace_header(detached_edges.next().expect("detached graph edge header"));
        }
        let mut requirement_maps: Vec<(
            super::node_requirements_map::NodeRequirementsMap,
            super::node_requirements_map::NodeRequirementsMap,
        )> = Vec::new();
        for header in copied.long_distance_requirements.values_mut() {
            let detached = if let Some((_, copied)) = requirement_maps
                .iter()
                .find(|(original, _)| original.same_allocation(header))
            {
                copied.retained()
            } else {
                let copied = header.clone();
                requirement_maps.push((header.retained(), copied.retained()));
                copied
            };
            *header = detached;
        }
        let mut headers: Vec<_> = self
            .views
            .iter()
            .flatten()
            .map(|view| view.nodes.header())
            .collect();
        let member_count = headers.len();
        headers.extend(self.groups.iter().map(|(_, group)| group.fields().nodes));
        let mut maps: Vec<_> = self
            .views
            .iter()
            .flatten()
            .flat_map(|view| {
                [
                    view.hubs.header(),
                    view.common_uncle_siblings.header(),
                    view.containers
                        .as_ref()
                        .map(GraphNodeMap::header)
                        .unwrap_or_else(SharedSliceMap::nil),
                ]
            })
            .collect();
        maps.extend_from_slice(external_maps);
        let (maps, detached) = SharedSliceMap::fork_storage_batch(&maps, &headers)
            .expect("numerical owner map/member backing");
        let mut maps = maps.into_iter();
        for view in copied.views.iter_mut().flatten() {
            view.hubs.replace_header(maps.next().expect("hub map"));
            view.common_uncle_siblings
                .replace_header(maps.next().expect("sibling map"));
            let containers = maps.next().expect("container map");
            if view.containers.is_some() {
                view.containers = Some(GraphNodeMap::from_header(containers));
            }
        }
        for (view, nodes) in copied
            .views
            .iter_mut()
            .flatten()
            .zip(detached.iter().take(member_count).cloned())
        {
            view.nodes = OwnerGraphNodesField::new(nodes);
        }
        copied.groups = self
            .groups
            .iter()
            .zip(detached.into_iter().skip(member_count))
            .map(|((id, group), nodes)| (*id, Arc::new(group.numerical_copy(nodes))))
            .collect();
        let group_maps: Vec<_> = self
            .views
            .iter()
            .flatten()
            .flat_map(|view| [view.clusters.header(), view.sequences.header()])
            .collect();
        let mut group_maps = SharedOwnerGroupMap::fork_batch(&group_maps).into_iter();
        for view in copied.views.iter_mut().flatten() {
            view.clusters =
                SharedOwnerGroupMapField::new(group_maps.next().expect("detached Cluster map"));
            view.sequences =
                SharedOwnerGroupMapField::new(group_maps.next().expect("detached Sequence map"));
        }
        let tree_maps: Vec<_> = self
            .views
            .iter()
            .flatten()
            .map(|view| view.node_to_tree.clone())
            .collect();
        for (view, map) in copied
            .views
            .iter_mut()
            .flatten()
            .zip(SharedOwnerTreeMap::fork_storage_batch(&tree_maps))
        {
            view.node_to_tree = map;
        }
        let children: Vec<_> = self
            .trees
            .iter()
            .map(|(_, tree)| tree.fields().children)
            .collect();
        let root_maps: Vec<_> = self
            .views
            .iter()
            .flatten()
            .map(|view| view.tree_roots.clone())
            .collect();
        let (root_maps, children) =
            SharedOwnerTreeRootsMap::fork_with_slices(&root_maps, &children);
        for (view, roots) in copied.views.iter_mut().flatten().zip(root_maps) {
            view.tree_roots = roots;
        }
        copied.trees = self
            .trees
            .iter()
            .zip(children)
            .map(|((id, tree), children)| {
                (*id, Arc::new(tree.numerical_copy_with_children(children)))
            })
            .collect();
        let detached_scope = MemberScopeId::default();
        let mut detached_herds = BTreeMap::new();
        for herd in copied.node_herds.values_mut().flatten() {
            *herd = detached_herds
                .entry(herd.allocation_key())
                .or_insert_with(|| herd.detached(&detached_scope))
                .clone();
        }
        copied.member_scope = detached_scope;
        (copied, detached_routes, maps.collect())
    }
}

/// A trusted source map header. A cloned handle shares its actual allocation;
/// numerical graph forks have a different storage scope despite matching IDs.
#[derive(Clone, Debug)]
pub(super) struct OwnerNodeToTreeMap {
    arena: OwnerArenaId,
    scope: MemberScopeId,
    map: SharedOwnerTreeMap,
}

/// Namespace/scope validation protects the actual shared Graph.Trees map.
#[derive(Clone, Debug)]
pub(super) struct OwnerTreeRootsMap {
    arena: OwnerArenaId,
    scope: MemberScopeId,
    map: SharedOwnerTreeRootsMap,
}

impl OwnerTreeRootsMap {
    pub(super) fn live_keys(&self) -> impl Iterator<Item = Option<NodeId>> + use<> {
        self.map.entries().into_iter().map(|(node, _)| node)
    }

    pub(super) fn is_nil(&self) -> bool {
        self.map.is_nil()
    }
    pub(super) fn len(&self) -> usize {
        self.map.len()
    }
    pub(super) fn same_map_allocation(&self, other: &Self) -> bool {
        self.map.same_allocation(&other.map)
    }
}

#[derive(Clone, Debug)]
pub(super) struct OwnerTreeRootsHeader {
    arena: OwnerArenaId,
    scope: MemberScopeId,
    slice: OwnerTreeSlice,
}

impl OwnerTreeRootsHeader {
    pub(super) fn is_nil(&self) -> bool {
        self.slice.is_nil()
    }
    pub(super) fn len(&self) -> usize {
        self.slice.len()
    }
    pub(super) fn capacity(&self) -> usize {
        self.slice.capacity()
    }
    pub(super) fn get(&self, index: usize) -> Option<Option<OwnerTreeId>> {
        self.slice.get(index)
    }
    pub(super) fn read_capacity_slot(&self, index: usize) -> Option<Option<OwnerTreeId>> {
        self.slice.read_capacity_slot(index)
    }
    pub(super) fn reslice(
        &self,
        low: usize,
        high: usize,
        max: usize,
    ) -> Result<Self, OwnerTopologyError> {
        Ok(Self {
            arena: self.arena,
            scope: self.scope,
            slice: self
                .slice
                .reslice_with_capacity(low, high, max)
                .ok_or_else(|| OwnerTopologyError::ContainerSlice("Tree slice bounds".into()))?,
        })
    }
    #[cfg(any(test, feature = "diagnostic-traces"))]
    pub(super) fn data_address(&self) -> Option<usize> {
        self.slice.data_address()
    }
    #[cfg(any(test, feature = "diagnostic-traces"))]
    pub(super) fn slot_address(&self, index: usize) -> Option<usize> {
        self.slice.slot_address(index)
    }
}

impl OwnerNodeToTreeMap {
    pub(super) fn live_keys(&self) -> impl Iterator<Item = Option<NodeId>> + use<> {
        self.map.live_keys()
    }
    pub(super) fn is_nil(&self) -> bool {
        self.map.is_nil()
    }

    pub(super) fn len(&self) -> usize {
        self.map.len()
    }

    pub(super) fn same_map_allocation(&self, other: &Self) -> bool {
        self.map.same_allocation(&other.map)
    }

    #[cfg(any(test, feature = "diagnostic-traces"))]
    pub(super) fn map_address(&self) -> Option<usize> {
        self.map.map_address()
    }

    #[cfg(any(test, feature = "diagnostic-traces"))]
    pub(super) fn observed_entries(&self) -> Vec<(Option<NodeId>, Option<OwnerTreeId>)> {
        self.map.observed_entries()
    }
}

/// A real Graph.Nodes header. Header clones retain the same nullable backing;
/// replacing a view's field does not retarget an already retained range.
#[derive(Clone, Debug)]
pub(super) struct OwnerMemberHeader {
    arena: OwnerArenaId,
    scope: MemberScopeId,
    slice: SharedNodeSlice,
}

/// Geometry rollback saves the visible prefix, unlike topology's full-capacity snapshot.
#[derive(Clone, Debug)]
pub(super) struct OwnerGeometryMembersSnapshot {
    receiver: OwnerGraphId,
    field: OwnerGraphNodesField,
    header: OwnerMemberHeader,
    visible: Vec<Option<NodeId>>,
}

impl OwnerMemberHeader {
    pub(super) fn is_nil(&self) -> bool {
        self.slice.is_nil()
    }
    pub(super) fn is_empty(&self) -> bool {
        self.slice.is_empty()
    }
    pub(super) fn len(&self) -> usize {
        self.slice.len()
    }
    pub(super) fn capacity(&self) -> usize {
        self.slice.capacity()
    }
    pub(super) fn offset(&self) -> usize {
        self.slice.offset()
    }
    pub(super) fn get(&self, index: usize) -> Option<Option<NodeId>> {
        self.slice.get(index)
    }
    pub(super) fn read_capacity_slot(&self, index: usize) -> Option<Option<NodeId>> {
        self.slice.read_capacity_slot(index)
    }
    pub(super) fn iter_slots(&self) -> impl Iterator<Item = Option<NodeId>> + use<> {
        self.slice.iter_slots()
    }
    pub(super) fn visible_slots(&self) -> Vec<Option<NodeId>> {
        self.slice.visible_slots()
    }
    pub(super) fn reslice_with_capacity(
        &self,
        low: usize,
        high: usize,
        max: usize,
    ) -> Result<Self, OwnerTopologyError> {
        Ok(Self {
            arena: self.arena,
            scope: self.scope,
            slice: self
                .slice
                .reslice_with_capacity(low, high, max)
                .map_err(|error| OwnerTopologyError::ContainerSlice(error.to_string()))?,
        })
    }
    #[cfg(any(test, feature = "diagnostic-traces"))]
    pub(super) fn data_address(&self) -> Option<usize> {
        self.slice.data_address()
    }
    #[cfg(any(test, feature = "diagnostic-traces"))]
    pub(super) fn slot_address(&self, index: usize) -> Option<usize> {
        self.slice.slot_address(index)
    }
    #[cfg(any(test, feature = "diagnostic-traces"))]
    pub(super) fn backing_address(&self) -> Option<usize> {
        self.slice.backing_address()
    }
    #[cfg(any(test, feature = "diagnostic-traces"))]
    pub(super) fn capacity_slots(&self) -> Vec<Option<NodeId>> {
        self.slice.capacity_slots()
    }
}

include!("owner_groups_api.rs");
include!("owner_incidents_api.rs");
include!("owner_abductions_api.rs");
include!("owner_directions_api.rs");
include!("owner_points_api.rs");
include!("owner_routes_api.rs");
include!("owner_herds_api.rs");
include!("owner_hierarchies_api.rs");
include!("owner_decoded_records.rs");
include!("owner_cluster_reset.rs");
include!("owner_sequence_creation.rs");
include!("owner_cluster_creation.rs");
include!("owner_sequence_probe.rs");
#[cfg(feature = "diagnostic-traces")]
#[path = "owner_decoded_records_probe.rs"]
mod owner_decoded_records_probe;
#[cfg(test)]
#[path = "owner_decoded_records_tests.rs"]
mod owner_decoded_records_tests;

/// Namespace protection is separate from physical map/backing identity.
/// This handle retains a real map allocation shared by explicit owner views.
#[derive(Clone, Debug)]
pub(super) struct OwnerContainerMap {
    arena: OwnerArenaId,
    scope: MemberScopeId,
    slices: SharedSliceMap,
}

#[derive(Clone, Debug)]
pub(super) struct OwnerContainerHeader {
    arena: OwnerArenaId,
    // Explicit owner API boundary. Independent Rust candidates preserve node
    // IDs but cannot import a parent's live backing by reusing that namespace.
    map: SharedSliceMap,
    slice: SharedNodeSlice,
}

impl OwnerContainerHeader {
    pub(super) fn iter_slots(&self) -> impl Iterator<Item = Option<NodeId>> + '_ {
        self.slice.borrowed_slots()
    }
    pub(super) fn is_nil(&self) -> bool {
        self.slice.is_nil()
    }
    pub(super) fn len(&self) -> usize {
        self.slice.len()
    }
    pub(super) fn capacity(&self) -> usize {
        self.slice.capacity()
    }
    pub(super) fn get(&self, index: usize) -> Option<Option<NodeId>> {
        self.slice.get(index)
    }

    pub(super) fn offset(&self) -> usize {
        self.slice.offset()
    }

    pub(super) fn read_capacity_slot(&self, index: usize) -> Option<Option<NodeId>> {
        self.slice.read_capacity_slot(index)
    }

    #[cfg(any(test, feature = "diagnostic-traces"))]
    pub(super) fn data_address(&self) -> Option<usize> {
        self.slice.data_address()
    }

    #[cfg(any(test, feature = "diagnostic-traces"))]
    pub(super) fn slot_address(&self, index: usize) -> Option<usize> {
        self.slice.slot_address(index)
    }

    #[cfg(any(test, feature = "diagnostic-traces"))]
    pub(super) fn backing_address(&self) -> Option<usize> {
        self.slice.backing_address()
    }

    #[cfg(any(test, feature = "diagnostic-traces"))]
    pub(super) fn capacity_slots(&self) -> Vec<Option<NodeId>> {
        self.slice.capacity_slots()
    }

    /// Reslicing retains this trusted header's arena and actual map binding.
    /// Callers cannot substitute an unrelated backing into the wrapper.
    pub(super) fn reslice_with_capacity(
        &self,
        low: usize,
        high: usize,
        max: usize,
    ) -> Result<Self, OwnerTopologyError> {
        Ok(Self {
            arena: self.arena,
            map: self.map.clone(),
            slice: self
                .slice
                .reslice_with_capacity(low, high, max)
                .map_err(|error| OwnerTopologyError::ContainerSlice(error.to_string()))?,
        })
    }
}

impl OwnerContainerMap {
    pub(super) fn live_entries(
        &self,
    ) -> impl Iterator<Item = (Option<NodeId>, OwnerContainerHeader)> + use<> {
        let arena = self.arena;
        let map = self.slices.clone();
        self.slices.live_entries().map(move |(key, slice)| {
            (
                key,
                OwnerContainerHeader {
                    arena,
                    map: map.clone(),
                    slice,
                },
            )
        })
    }
    pub(super) fn is_nil(&self) -> bool {
        self.slices.is_nil()
    }
    pub(super) fn len(&self) -> usize {
        self.slices.len()
    }

    pub(super) fn get(&self, parent: Option<NodeId>) -> Option<OwnerContainerHeader> {
        self.slices.get(&parent).map(|slice| OwnerContainerHeader {
            arena: self.arena,
            map: self.slices.clone(),
            slice,
        })
    }

    /// `get(parent)`'s slice header alone, without the map binding.
    pub(super) fn get_slice(&self, parent: Option<NodeId>) -> Option<SharedNodeSlice> {
        self.slices.get(&parent)
    }

    pub(super) fn same_map_allocation(&self, other: &Self) -> bool {
        self.slices.same_map_allocation(&other.slices)
    }
}

/// Scope topology rollback, independent of geometry trial snapshots. Routes
/// and labels remain attached to the actual same owning edges. Restoring this
/// snapshot restores captured state, without destroying separately retained
/// records/views/entities allocated after capture.
#[derive(Debug)]
pub(super) struct OwnerTopologySnapshot {
    node_count: usize,
    node_owner_values: Vec<Option<OwnerGraphId>>,
    raw_incidents: Vec<(NodeId, GraphEdges, SharedEdgeSliceSnapshot)>,
    raw_graph_edges: Vec<(usize, SharedEdgeSliceSnapshot)>,
    node_nears: Vec<super::node_near_map::NodeNearMapSnapshot>,
    hierarchy_memberships: Vec<Option<HierarchyMembership>>,
    hierarchies: Vec<OwnerHierarchySnapshot>,
    requirement_keys: Vec<(
        NodeId,
        super::node_requirements_map::NodeRequirementsMapSnapshot,
    )>,
    raw_slice_maps: Vec<(usize, bool, SharedSliceMapSnapshot)>,
    raw_container_maps: Vec<(usize, SharedSliceMapSnapshot)>,
    node_links: Vec<(Option<NodeId>, Option<usize>, Option<usize>, bool)>,
    edges: Vec<ArenaEdge>,
    clusters: Vec<ClusterState>,
    sequences: Vec<SequenceState>,
    cluster_vessel_sizes: Vec<Size>,
    node_order: Vec<NodeId>,
    incident_edge_order: BTreeMap<NodeId, Vec<EdgeId>>,
    shared_topology: SharedTopology,
    member_slices: Vec<(usize, SharedNodeSliceSnapshot)>,
    node_to_tree_maps: Vec<(usize, SharedOwnerTreeMapSnapshot)>,
    tree_root_maps: Vec<(usize, SharedOwnerTreeRootsMapSnapshot)>,
    group_maps: Vec<(usize, OwnerGroupKind, SharedOwnerGroupMapSnapshot)>,
    container_slices: SharedSliceMapSnapshot,
    container_field: GraphNodeMap,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum OwnerTopologyError {
    ForeignArena {
        expected: Option<OwnerArenaId>,
        actual: Option<OwnerArenaId>,
    },
    UnknownNode(NodeId),
    UnknownEdge(EdgeId),
    UnknownRecord(OwnerAbductionId),
    UnknownTree(OwnerTreeId),
    UnknownGroup(OwnerGroupId),
    GroupKindMismatch,
    GroupBindingConflict(TopologyVessel),
    ForeignGroupScope,
    NilGroupMapAssignment,
    UnboundIncidentHeader(NodeId),
    UnboundRouteHeader(EdgeId),
    ForeignRouteScope,
    UnboundGraphEdgeHeader(OwnerGraphId),
    ForeignEdgeScope,
    ForeignAbductionScope,
    UnboundNodePoint(NodeId),
    ForeignPointScope,
    ForeignHerdScope,
    UnboundHerdAssignment(NodeId),
    ForeignHierarchyScope,
    UnboundHierarchy(NodeId),
    UnknownGraph(OwnerGraphId),
    MissingSourceLineage {
        edge: EdgeId,
        side: &'static str,
    },
    UnboundVessel(TopologyVessel),
    InvalidVessel(TopologyVessel),
    VesselAliasesMember {
        vessel: TopologyVessel,
        node: NodeId,
    },
    VesselIdentityMismatch {
        vessel: TopologyVessel,
        node: NodeId,
    },
    ContainerSlice(String),
    ForeignContainerMap,
    ForeignMemberScope,
    ForeignNodeToTreeScope,
    NilNodeToTreeMapAssignment,
    NilTreeRootsMapAssignment,
    ForeignTreeRootsScope,
    UnboundCostGeometry(NodeId),
    DirectionEntryConflict(Option<NodeId>),
}

impl fmt::Display for OwnerTopologyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "canonical owner topology: {self:?}")
    }
}
impl std::error::Error for OwnerTopologyError {}

impl ArenaGraph {
    #[cfg(any(test, feature = "diagnostic-traces"))]
    pub(super) fn probe_replace_graph_nodes(&mut self, header: SharedNodeSlice) {
        self.shared_topology.views[0]
            .as_mut()
            .unwrap()
            .nodes
            .replace(header);
    }

    pub(super) fn bind_root_proximity_fields(&mut self) {
        if let Some(view) = self.shared_topology.views.first().and_then(Option::as_ref) {
            self.hubs = view.hubs.share_graph();
            self.common_uncle_siblings = view.common_uncle_siblings.share_graph();
        }
    }

    pub(super) fn shared_topology_initialized(&self) -> bool {
        self.shared_topology.initialized
    }

    pub(super) fn topology_arena_id(&self) -> Option<OwnerArenaId> {
        self.shared_topology.arena
    }

    /// Establish the original graph view once. Subsequent scope views share
    /// these actual entities instead of reconstructing a predicted owner.
    pub(super) fn initialize_shared_topology(&mut self) {
        if self.shared_topology.initialized {
            return;
        }
        let nodes = SharedNodeSlice::from_nodes(&self.node_order)
            .expect("original owner member backing allocation");
        self.initialize_shared_topology_with_members(nodes);
    }

    /// The input factory supplies the actual append/make result. No capacity
    /// is recovered from a later numerical member vector.
    pub(super) fn initialize_shared_topology_with_members(&mut self, nodes: SharedNodeSlice) {
        assert!(
            !self.shared_topology.initialized,
            "owner graph constructed once"
        );
        self.shared_topology = SharedTopology {
            initialized: true,
            arena: Some(allocate_owner_arena()),
            views: vec![Some(OwnerGraphView {
                nodes: OwnerGraphNodesField::new(nodes),
                edges: self.edge_order.share_graph(),
                common_uncle_siblings: self.common_uncle_siblings.share_graph(),
                hubs: self.hubs.share_graph(),
                node_to_tree: SharedOwnerTreeMap::nil(),
                tree_roots: SharedOwnerTreeRootsMap::allocated(),
                clusters: SharedOwnerGroupMapField::new(SharedOwnerGroupMap::allocated()),
                sequences: SharedOwnerGroupMapField::new(SharedOwnerGroupMap::allocated()),
                local_to_canonical: (0..self.nodes.len())
                    .map(|index| NodeId(index as u32))
                    .collect(),
                score_state: None,
                directions: None,
                containers: None,
            })],
            node_owners: (0..self.nodes.len())
                .map(|_| OwnerNodeGraphField::new(Some(OwnerGraphId(0))))
                .collect(),
            source_input_nodes: self.nodes.iter().map(|node| Some(node.input_id)).collect(),
            // graphjson.MarkDecodedClusterVessel sets a node field independently
            // of member count and the later active-member publication filter.
            cluster_vessel_flags: self
                .nodes
                .iter()
                .filter(|node| {
                    self.serialized_cluster_arrangements_by_tala
                        .contains_key(&node.tala_id)
                })
                .map(|node| node.input_id)
                .collect(),
            ..SharedTopology::default()
        };
    }

    pub(super) fn topology_node(&self, node: NodeId) -> Option<&ArenaNode> {
        self.nodes.get(node.0 as usize)
    }

    pub(super) fn topology_edge(&self, edge: EdgeId) -> Option<&ArenaEdge> {
        self.edge_records.get(edge.0 as usize)
    }

    pub(super) fn topology_node_owner(&self, node: NodeId) -> Option<OwnerGraphId> {
        self.shared_topology
            .node_owners
            .get(node.0 as usize)
            .and_then(OwnerNodeGraphField::get)
    }

    pub(super) fn retain_owner_node_graph_field(
        &mut self,
        node: NodeId,
    ) -> Result<OwnerNodeGraphField, OwnerTopologyError> {
        self.initialize_shared_topology();
        self.require_topology_node(node)?;
        Ok(self.shared_topology.node_owners[node.0 as usize].clone())
    }

    /// Assign the actual Node.Graph field, including a genuine nil reference.
    /// This direct field operation is independent of member headers, dense
    /// storage mappings and the bulk descendant ownership journal.
    pub(super) fn set_owner_node_graph_reference(
        &mut self,
        node: NodeId,
        graph: Option<OwnerGraphId>,
    ) -> Result<(), OwnerTopologyError> {
        self.initialize_shared_topology();
        self.require_topology_node(node)?;
        if let Some(graph) = graph {
            self.owner_graph_view(graph)
                .ok_or(OwnerTopologyError::UnknownGraph(graph))?;
        }
        self.shared_topology.node_owners[node.0 as usize].set(graph);
        Ok(())
    }

    pub(super) fn topology_source_input_node(&self, node: NodeId) -> Option<NodeId> {
        self.shared_topology
            .source_input_nodes
            .get(node.0 as usize)
            .copied()
            .flatten()
    }

    pub(super) fn owner_topology_snapshot(&mut self) -> OwnerTopologySnapshot {
        self.initialize_shared_topology();
        // This topology-only snapshot omits the live cardinal Directions map.
        // Extended values have the same lifetime; full source graph-state map
        // restoration is a separate contract, not added by this snapshot.
        let mut shared_topology = self.shared_topology.clone_with_shared_member_headers();
        // A transaction restores the same Graph objects, including retained
        // field handles. Numerical clones still copy/detach field cells.
        for (saved, live) in shared_topology
            .views
            .iter_mut()
            .zip(&self.shared_topology.views)
        {
            if let (Some(saved), Some(live)) = (saved, live) {
                saved.hubs = live.hubs.share_graph();
                saved.common_uncle_siblings = live.common_uncle_siblings.share_graph();
                saved.edges = live.edges.share_graph();
                saved.containers = live.containers.as_ref().map(GraphNodeMap::share_graph);
            }
        }
        shared_topology.direction_extensions.clear();
        let member_slices = self
            .shared_topology
            .views
            .iter()
            .enumerate()
            .filter_map(|(index, view)| view.as_ref().map(|view| (index, view.nodes.snapshot())))
            .collect();
        let node_to_tree_maps = self
            .shared_topology
            .views
            .iter()
            .enumerate()
            .filter_map(|(index, view)| {
                view.as_ref()
                    .map(|view| (index, view.node_to_tree.snapshot()))
            })
            .collect();
        let tree_root_maps = self
            .shared_topology
            .views
            .iter()
            .enumerate()
            .filter_map(|(index, view)| {
                view.as_ref()
                    .map(|view| (index, view.tree_roots.snapshot()))
            })
            .collect();
        let group_maps = self
            .shared_topology
            .views
            .iter()
            .enumerate()
            .filter_map(|(index, view)| {
                view.as_ref().map(|view| {
                    [
                        (index, OwnerGroupKind::Cluster, view.clusters.snapshot()),
                        (index, OwnerGroupKind::Sequence, view.sequences.snapshot()),
                    ]
                })
            })
            .flatten()
            .collect();
        OwnerTopologySnapshot {
            node_count: self.nodes.len(),
            node_owner_values: self
                .shared_topology
                .node_owners
                .iter()
                .map(OwnerNodeGraphField::get)
                .collect(),
            raw_incidents: self
                .nodes
                .iter()
                .enumerate()
                .map(|(index, node)| {
                    (
                        NodeId(index as u32),
                        node.edges.retain_field(),
                        node.edges.header().snapshot(),
                    )
                })
                .collect(),
            raw_graph_edges: self
                .shared_topology
                .views
                .iter()
                .enumerate()
                .filter_map(|(index, view)| {
                    view.as_ref()
                        .map(|view| (index, view.edges.header().snapshot()))
                })
                .collect(),
            // transaction_snapshot.go captureNode193/restore211: restore the
            // original map allocation and contents, including retained aliases.
            node_nears: self
                .nodes
                .iter()
                .map(|node| node.nears.snapshot())
                .collect(),
            hierarchy_memberships: self
                .nodes
                .iter()
                .map(|node| node.hierarchy.clone())
                .collect(),
            hierarchies: self
                .nodes
                .iter()
                .filter_map(|node| node.hierarchy.as_ref().map(|h| &h.record))
                .map(|hierarchy| (hierarchy.allocation_key(), hierarchy))
                .collect::<BTreeMap<_, _>>()
                .values()
                .map(|hierarchy| hierarchy.snapshot())
                .collect(),
            requirement_keys: self
                .shared_topology
                .long_distance_requirements
                .iter()
                .map(|(&node, map)| (node, map.snapshot()))
                .collect(),
            raw_container_maps: self
                .shared_topology
                .views
                .iter()
                .enumerate()
                .filter_map(|(index, view)| {
                    view.as_ref().and_then(|v| {
                        v.containers
                            .as_ref()
                            .map(|m| (index, m.header().snapshot()))
                    })
                })
                .collect(),
            raw_slice_maps: self
                .shared_topology
                .views
                .iter()
                .enumerate()
                .flat_map(|(index, view)| {
                    view.iter().flat_map(move |view| {
                        [
                            (true, view.hubs.header()),
                            (false, view.common_uncle_siblings.header()),
                        ]
                        .into_iter()
                        .map(move |(hubs, map)| (index, hubs, map.snapshot()))
                    })
                })
                .collect(),
            node_links: self
                .nodes
                .iter()
                .map(|node| {
                    (
                        node.container,
                        node.cluster,
                        node.sequence,
                        node.is_container,
                    )
                })
                .collect(),
            edges: self.edge_records.clone(),
            clusters: self.clusters.clone(),
            sequences: self.sequences.clone(),
            cluster_vessel_sizes: self.cluster_vessel_sizes.clone(),
            node_order: self.node_order.clone(),
            incident_edge_order: self.incident_edge_order.clone(),
            shared_topology,
            member_slices,
            node_to_tree_maps,
            tree_root_maps,
            group_maps,
            container_slices: self.containers.snapshot(),
            container_field: self.containers.share_field(),
        }
    }

    pub(super) fn restore_owner_topology_snapshot(&mut self, mut snapshot: OwnerTopologySnapshot) {
        // Go restores captured objects; it does not invalidate allocations
        // held by a caller outside the restored Graph's member/map headers.
        // Keep canonical storage distinct from restored active membership.
        let captured_nodes = snapshot.node_count;
        let live_node_count = self.nodes.len();
        let live_records = self
            .shared_topology
            .records
            .split_off(snapshot.shared_topology.records.len());
        let live_views = self
            .shared_topology
            .views
            .split_off(snapshot.shared_topology.views.len());
        let live_node_owners = self.shared_topology.node_owners.split_off(captured_nodes);
        let live_source_inputs = self
            .shared_topology
            .source_input_nodes
            .split_off(captured_nodes);
        let live_points: Vec<_> = self
            .shared_topology
            .points
            .iter()
            .filter(|(node, _)| node.0 as usize >= captured_nodes)
            .map(|(k, v)| (k, v.clone()))
            .collect();
        let live_node_herds: Vec<_> = self
            .shared_topology
            .node_herds
            .iter()
            .filter(|(node, _)| node.0 as usize >= captured_nodes)
            .map(|(k, v)| (*k, v.clone()))
            .collect();
        let live_node_groups: Vec<_> = self
            .shared_topology
            .node_groups
            .iter()
            .filter(|((node, _), _)| node.0 as usize >= captured_nodes)
            .map(|(k, v)| (k, *v))
            .collect();
        let live_requirements: Vec<_> = self
            .shared_topology
            .long_distance_requirements
            .iter()
            .filter(|(node, _)| node.0 as usize >= captured_nodes)
            .map(|(k, v)| (*k, v.retained()))
            .collect();
        let live_vessel_flags: Vec<_> = self
            .shared_topology
            .cluster_vessel_flags
            .iter()
            .filter(|node| node.0 as usize >= captured_nodes)
            .copied()
            .collect();
        let live_direction_extensions =
            std::mem::take(&mut self.shared_topology.direction_extensions);
        // Source restores the map, not the lifetime of separately allocated
        // Tree objects held by retained replacement maps. These live
        // records must not be invalidated or reused on topology restoration.
        let live_trees = std::mem::take(&mut self.shared_topology.trees);
        let live_groups = std::mem::take(&mut self.shared_topology.groups);
        for (node, nears) in self.nodes.iter_mut().zip(snapshot.node_nears) {
            node.nears = nears.restore();
        }
        for (node, hierarchy) in self.nodes.iter_mut().zip(snapshot.hierarchy_memberships) {
            node.hierarchy = hierarchy;
        }
        for hierarchy in snapshot.hierarchies {
            hierarchy.restore();
        }
        for (node, map) in snapshot.requirement_keys {
            snapshot
                .shared_topology
                .long_distance_requirements
                .insert(node, map.restore());
        }
        for (index, map) in snapshot.raw_container_maps {
            snapshot.shared_topology.views[index]
                .as_ref()
                .expect("captured owner graph")
                .containers
                .as_ref()
                .expect("captured container field")
                .replace_header(map.restore());
        }
        for (index, hubs, map) in snapshot.raw_slice_maps {
            let view = snapshot.shared_topology.views[index]
                .as_mut()
                .expect("captured owner graph");
            if hubs {
                view.hubs.replace_header(map.restore());
            } else {
                view.common_uncle_siblings.replace_header(map.restore());
            }
        }
        for (node, (container, cluster, sequence, is_container)) in
            self.nodes.iter_mut().zip(snapshot.node_links)
        {
            node.container = container;
            node.cluster = cluster;
            node.sequence = sequence;
            node.is_container = is_container;
        }
        assert!(
            self.edge_records.len() >= snapshot.edges.len(),
            "canonical allocations are retained"
        );
        for (record, saved) in self.edge_records.iter_mut().zip(snapshot.edges) {
            record.restore_value(saved);
        }
        self.clusters = snapshot.clusters;
        self.sequences = snapshot.sequences;
        self.cluster_vessel_sizes = snapshot.cluster_vessel_sizes;
        self.node_order = snapshot.node_order;
        self.incident_edge_order = snapshot.incident_edge_order;
        snapshot
            .container_field
            .replace_header(snapshot.container_slices.restore());
        self.containers = ContainerMap::from_field(snapshot.container_field);
        for (index, members) in snapshot.member_slices {
            snapshot.shared_topology.views[index]
                .as_mut()
                .expect("captured owner graph view")
                .nodes
                .replace(members.restore());
        }
        for (node, field, header) in snapshot.raw_incidents {
            field.replace_header(header.restore());
            self.nodes[node.0 as usize].edges = IncidentEdges::from_field(field);
        }
        for (index, header) in snapshot.raw_graph_edges {
            snapshot.shared_topology.views[index]
                .as_mut()
                .expect("captured owner graph")
                .edges
                .replace_header(header.restore());
        }
        for (index, map) in snapshot.node_to_tree_maps {
            snapshot.shared_topology.views[index]
                .as_mut()
                .expect("captured owner graph view")
                .node_to_tree = map.restore();
        }
        for (index, map) in snapshot.tree_root_maps {
            snapshot.shared_topology.views[index]
                .as_mut()
                .expect("captured owner graph view")
                .tree_roots = map.restore();
        }
        for (index, kind, map) in snapshot.group_maps {
            let view = snapshot.shared_topology.views[index]
                .as_mut()
                .expect("captured owner graph view");
            match kind {
                OwnerGroupKind::Cluster => view.clusters.replace(map.restore()),
                OwnerGroupKind::Sequence => view.sequences.replace(map.restore()),
            }
        }
        for (field, value) in snapshot
            .shared_topology
            .node_owners
            .iter()
            .zip(snapshot.node_owner_values)
        {
            field.set(value);
        }
        self.shared_topology = snapshot.shared_topology;
        self.shared_topology.records.extend(live_records);
        self.shared_topology.views.extend(live_views);
        self.shared_topology.node_owners.extend(live_node_owners);
        self.shared_topology
            .source_input_nodes
            .extend(live_source_inputs);
        self.shared_topology.points.extend(live_points);
        self.shared_topology.node_herds.extend(live_node_herds);
        self.shared_topology.node_groups.extend(live_node_groups);
        self.shared_topology
            .long_distance_requirements
            .extend(live_requirements);
        self.shared_topology
            .cluster_vessel_flags
            .extend(live_vessel_flags);
        self.shared_topology.views[0]
            .as_mut()
            .expect("root graph")
            .local_to_canonical
            .extend((captured_nodes..live_node_count).map(|i| NodeId(i as u32)));
        self.edge_order = self.shared_topology.views[0]
            .as_ref()
            .expect("root graph")
            .edges
            .share_graph();
        self.hubs = self.shared_topology.views[0]
            .as_ref()
            .unwrap()
            .hubs
            .share_graph();
        self.common_uncle_siblings = self.shared_topology.views[0]
            .as_ref()
            .unwrap()
            .common_uncle_siblings
            .share_graph();
        self.shared_topology.direction_extensions = live_direction_extensions;
        self.shared_topology.trees = live_trees;
        self.shared_topology.groups = live_groups;
        self.node_order_membership_valid = false;
        self.node_order_membership.resize(live_node_count, false);
        self.external_edge_counts.resize(live_node_count, 0);
        self.placement_component.resize(live_node_count, None);
        self.long_distance_neighbor_data
            .resize(live_node_count, None);
        self.descendant_cache.clear();
    }

    pub(super) fn topology_vessel_node(&self, vessel: TopologyVessel) -> Option<NodeId> {
        self.topology_vessel_reference(vessel).flatten()
    }

    /// An unbound group and a bound, genuinely nil Vessel are different.
    pub(super) fn topology_vessel_reference(
        &self,
        vessel: TopologyVessel,
    ) -> Option<Option<NodeId>> {
        if let Some(&group) = self.shared_topology.group_bindings.get(&vessel) {
            return self.owner_group(group).map(|group| group.vessel());
        }
        self.shared_topology.vessels.get(&vessel).copied()
    }

    /// Direct source field assignment, separate from initial vessel creation.
    /// The source permits replacement, nil, and aliasing with another group.
    pub(super) fn set_topology_vessel_reference(
        &mut self,
        vessel: TopologyVessel,
        node: Option<NodeId>,
    ) -> Result<(), OwnerTopologyError> {
        self.initialize_shared_topology();
        let valid = match vessel {
            TopologyVessel::Cluster(index) => index < self.clusters.len(),
            TopologyVessel::Sequence(index) => index < self.sequences.len(),
        };
        if !valid {
            return Err(OwnerTopologyError::InvalidVessel(vessel));
        }
        if let Some(node) = node {
            self.require_topology_node(node)?;
        }
        if let Some(&group) = self.shared_topology.group_bindings.get(&vessel) {
            let record = self
                .owner_group(group)
                .ok_or(OwnerTopologyError::UnknownGroup(group))?;
            let mut fields = record.fields();
            fields.vessel = node;
            record.set_fields(fields);
        } else {
            self.shared_topology.vessels.insert(vessel, node);
        }
        Ok(())
    }

    pub(super) fn topology_node_vessel(&self, node: NodeId) -> Option<TopologyVessel> {
        if let Some(vessel) =
            self.shared_topology
                .group_bindings
                .iter()
                .find_map(|(&kind, &group)| {
                    (self.owner_group(group)?.vessel() == Some(node)).then_some(kind)
                })
        {
            return Some(vessel);
        }
        self.shared_topology
            .vessels
            .iter()
            .find_map(|(&kind, &candidate)| (candidate == Some(node)).then_some(kind))
    }

    /// Temporary nonnil traversal adapter over the sole live map. Nil/empty
    /// and map absence remain distinct; nil children are never discarded.
    pub(super) fn topology_container_slice(
        &self,
        parent: Option<NodeId>,
    ) -> Option<Option<Vec<NodeId>>> {
        let header = self.containers.get_header(&parent)?;
        Some((!header.is_nil()).then(|| header.require_nodes().collect()))
    }

    pub(super) fn set_topology_container_slice(
        &mut self,
        parent: Option<NodeId>,
        children: Option<Vec<NodeId>>,
    ) {
        self.initialize_shared_topology();
        let header = children.map_or_else(SharedNodeSlice::nil, |children| {
            SharedNodeSlice::from_nodes(&children).expect("owning container make")
        });
        self.containers.replace_header(parent, header);
    }

    /// Retain the actual Graph.Containers field, independently of its map
    /// header. Distinct Graph objects keep distinct field cells even when a
    /// CopyEntitiesFrom-style assignment shares their current map allocation.
    pub(super) fn retain_owner_graph_container_map_field(
        &self,
        receiver: OwnerGraphId,
    ) -> Result<OwnerContainerMapField, OwnerTopologyError> {
        let view = self
            .owner_graph_view(receiver)
            .ok_or(OwnerTopologyError::UnknownGraph(receiver))?;
        let field = if receiver == OwnerGraphId(0) {
            self.containers.share_field()
        } else {
            view.containers
                .as_ref()
                .expect("NewGraph container field")
                .share_graph()
        };
        Ok(OwnerContainerMapField {
            arena: self.topology_arena_id().unwrap(),
            scope: self.shared_topology.member_scope,
            field,
        })
    }

    /// Explicit CopyEntitiesFrom-style map sharing between owner views. This
    /// retains the sole production map, distinct from compact numerical maps.
    pub(super) fn replace_owner_common_uncle_map(
        &mut self,
        receiver: OwnerGraphId,
        map: Option<SharedSliceMap>,
    ) -> Result<Option<SharedSliceMap>, OwnerTopologyError> {
        self.initialize_shared_topology();
        let view = self
            .shared_topology
            .views
            .get_mut(receiver.0)
            .and_then(Option::as_mut)
            .ok_or(OwnerTopologyError::UnknownGraph(receiver))?;
        Ok(Some(
            view.common_uncle_siblings
                .replace_header(map.unwrap_or_else(SharedSliceMap::nil)),
        ))
    }

    pub(super) fn share_owner_common_uncle_map(
        &mut self,
        source: OwnerGraphId,
        destination: OwnerGraphId,
    ) -> Result<(), OwnerTopologyError> {
        let map = self
            .owner_graph_view(source)
            .ok_or(OwnerTopologyError::UnknownGraph(source))?
            .common_uncle_siblings
            .header();
        self.replace_owner_common_uncle_map(destination, Some(map))?;
        Ok(())
    }

    pub(super) fn share_owner_proximity_maps(
        &mut self,
        source: OwnerGraphId,
        destination: OwnerGraphId,
    ) -> Result<(), OwnerTopologyError> {
        let source_view = self
            .owner_graph_view(source)
            .ok_or(OwnerTopologyError::UnknownGraph(source))?;
        let hubs = source_view.hubs.header();
        let siblings = source_view.common_uncle_siblings.header();
        let destination_view = self
            .owner_graph_view(destination)
            .ok_or(OwnerTopologyError::UnknownGraph(destination))?;
        destination_view.hubs.replace_header(hubs);
        destination_view
            .common_uncle_siblings
            .replace_header(siblings);
        Ok(())
    }

    pub(super) fn set_owner_common_uncle_members(
        &mut self,
        receiver: OwnerGraphId,
        node: Option<NodeId>,
        header: OwnerMemberHeader,
    ) -> Result<(), OwnerTopologyError> {
        self.validate_owner_member_header(&header)?;
        if let Some(node) = node {
            self.require_topology_node(node)?;
        }
        let view = self
            .shared_topology
            .views
            .get_mut(receiver.0)
            .and_then(Option::as_mut)
            .ok_or(OwnerTopologyError::UnknownGraph(receiver))?;
        view.common_uncle_siblings
            .header()
            .replace(node, header.slice)
            .map_err(|error| OwnerTopologyError::ContainerSlice(error.to_string()))?;
        Ok(())
    }
    /// Copy the current sibling values within one callback-free borrow scope.
    pub(super) fn owner_common_uncle_values(
        &self,
        receiver: OwnerGraphId,
        node: NodeId,
    ) -> Result<Option<Vec<Option<NodeId>>>, OwnerTopologyError> {
        self.require_topology_node(node)?;
        let view = self
            .owner_graph_view(receiver)
            .ok_or(OwnerTopologyError::UnknownGraph(receiver))?;
        Ok(view
            .common_uncle_siblings
            .read_header()
            .with_entry(&Some(node), |slice| {
                slice.map(SharedNodeSlice::visible_slots)
            }))
    }
    pub(super) fn owner_common_uncle_header(
        &self,
        receiver: OwnerGraphId,
        node: NodeId,
    ) -> Result<Option<OwnerMemberHeader>, OwnerTopologyError> {
        self.require_topology_node(node)?;
        let view = self
            .owner_graph_view(receiver)
            .ok_or(OwnerTopologyError::UnknownGraph(receiver))?;
        let slice = view.common_uncle_siblings.header().get(&Some(node));
        Ok(slice.map(|slice| OwnerMemberHeader {
            arena: self.topology_arena_id().unwrap(),
            scope: self.shared_topology.member_scope,
            slice,
        }))
    }

    /// Explicit Graph.Containers header assignment; legacy numerical views
    /// keep their existing fallback until an actual receiver map is bound.
    pub(super) fn allocate_owner_container_map(&mut self, allocated: bool) -> OwnerContainerMap {
        self.initialize_shared_topology();
        OwnerContainerMap {
            arena: self.topology_arena_id().unwrap(),
            scope: self.shared_topology.member_scope,
            slices: if allocated {
                SharedSliceMap::new()
            } else {
                SharedSliceMap::nil()
            },
        }
    }
    pub(super) fn set_owner_graph_container_map(
        &mut self,
        receiver: OwnerGraphId,
        map: OwnerContainerMap,
    ) -> Result<(), OwnerTopologyError> {
        self.initialize_shared_topology();
        if Some(map.arena) != self.topology_arena_id() {
            return Err(OwnerTopologyError::ForeignArena {
                expected: self.topology_arena_id(),
                actual: Some(map.arena),
            });
        }
        if !MemberScopeId::same_scope(&map.scope, &self.shared_topology.member_scope) {
            return Err(OwnerTopologyError::ForeignContainerMap);
        }
        self.owner_graph_view(receiver)
            .ok_or(OwnerTopologyError::UnknownGraph(receiver))?;
        if receiver == OwnerGraphId(0) {
            self.containers.replace_map(map.slices);
        } else {
            let view = self.shared_topology.views[receiver.0].as_mut().unwrap();
            if let Some(field) = &view.containers {
                field.replace_header(map.slices);
            } else {
                view.containers = Some(GraphNodeMap::from_header(map.slices));
            }
        }
        Ok(())
    }
    pub(super) fn write_owner_container_map_entry(
        &mut self,
        map: &OwnerContainerMap,
        parent: Option<NodeId>,
        children: &[Option<NodeId>],
    ) -> Result<(), OwnerTopologyError> {
        if Some(map.arena) != self.topology_arena_id() {
            return Err(OwnerTopologyError::ForeignArena {
                expected: self.topology_arena_id(),
                actual: Some(map.arena),
            });
        }
        if !MemberScopeId::same_scope(&map.scope, &self.shared_topology.member_scope) {
            return Err(OwnerTopologyError::ForeignContainerMap);
        }
        for node in parent.into_iter().chain(children.iter().copied().flatten()) {
            self.require_topology_node(node)?;
        }
        let slice = SharedNodeSlice::with_len_cap(children.len(), children.len())
            .map_err(|e| OwnerTopologyError::ContainerSlice(e.to_string()))?;
        for (i, child) in children.iter().enumerate() {
            slice
                .write_capacity_slot(i, *child)
                .map_err(|e| OwnerTopologyError::ContainerSlice(e.to_string()))?;
        }
        map.slices
            .replace(parent, slice)
            .map_err(|e| OwnerTopologyError::ContainerSlice(e.to_string()))?;
        Ok(())
    }
    /// `owner_container_map(owner)?.get(parent)`, read in place: no map or
    /// header handle is cloned or retained past `read`.
    pub(super) fn with_owner_container_entry<R>(
        &self,
        owner: OwnerGraphId,
        parent: Option<NodeId>,
        read: impl FnOnce(Option<&SharedNodeSlice>) -> R,
    ) -> Option<R> {
        let view = self.owner_graph_view(owner)?;
        self.topology_arena_id()?;
        Some(match view.containers.as_ref() {
            Some(field) => field.read_header().with_entry(&parent, read),
            None => self.containers.with_header_entry(&parent, read),
        })
    }

    pub(super) fn owner_container_map(&self, owner: OwnerGraphId) -> Option<OwnerContainerMap> {
        let view = self.owner_graph_view(owner)?;
        Some(OwnerContainerMap {
            arena: self.topology_arena_id()?,
            scope: self.shared_topology.member_scope,
            slices: view
                .containers
                .as_ref()
                .map(GraphNodeMap::header)
                .unwrap_or_else(|| self.containers.share_map()),
        })
    }

    /// Construct an explicitly shaped header over canonical node handles.
    /// Keeping this separate from Vec adaptation avoids inventing a source
    /// capacity for existing dense graphs.
    pub(super) fn allocate_owner_container_header(
        &mut self,
        children: &[NodeId],
        capacity: usize,
    ) -> Result<OwnerContainerHeader, OwnerTopologyError> {
        self.initialize_shared_topology();
        for &node in children {
            self.require_topology_node(node)?;
        }
        let slice = SharedNodeSlice::with_len_cap(children.len(), capacity)
            .map_err(|error| OwnerTopologyError::ContainerSlice(error.to_string()))?;
        for (index, &node) in children.iter().enumerate() {
            slice
                .write_capacity_slot(index, Some(node))
                .map_err(|error| OwnerTopologyError::ContainerSlice(error.to_string()))?;
        }
        Ok(OwnerContainerHeader {
            arena: self.topology_arena_id().expect("initialized owning arena"),
            map: self.containers.share_map(),
            slice,
        })
    }

    pub(super) fn set_owner_container_header(
        &mut self,
        parent: Option<NodeId>,
        header: OwnerContainerHeader,
    ) -> Result<(), OwnerTopologyError> {
        self.initialize_shared_topology();
        if Some(header.arena) != self.topology_arena_id() {
            return Err(OwnerTopologyError::ForeignArena {
                expected: self.topology_arena_id(),
                actual: Some(header.arena),
            });
        }
        if !header.map.same_map_allocation(&self.containers.share_map()) {
            return Err(OwnerTopologyError::ForeignContainerMap);
        }
        if let Some(parent) = parent {
            self.require_topology_node(parent)?;
        }
        for node in header.slice.iter_slots().flatten() {
            self.require_topology_node(node)?;
        }
        self.containers.replace_header(parent, header.slice);
        Ok(())
    }

    /// graph.go246–255 appends to the existing header; it does not redirect
    /// Graph ownership or add the child to the graph's member slice.
    pub(super) fn append_owner_container_node(
        &mut self,
        parent: Option<NodeId>,
        child: NodeId,
    ) -> Result<OwnerContainerHeader, OwnerTopologyError> {
        self.initialize_shared_topology();
        if let Some(parent) = parent {
            self.require_topology_node(parent)?;
        }
        self.require_topology_node(child)?;
        let slice = self.containers.append_node(parent, child);
        self.nodes[child.0 as usize].container = parent;
        if let Some(parent) = parent {
            self.nodes[parent.0 as usize].is_container = true;
        }
        self.descendant_cache.clear();
        Ok(OwnerContainerHeader {
            arena: self.topology_arena_id().expect("initialized owning arena"),
            map: self.containers.share_map(),
            slice,
        })
    }

    /// Retain an actual supplied runtime entity outside Graph.Nodes. The
    /// canonical input_id indexes owning entity storage, NOT the public D2
    /// input. Use topology_source_input_node before accessing input metadata.
    /// A vessel's real box/identity must be constructed by its allocating
    /// algorithm; this method never clones a member to fabricate a vessel.
    pub(super) fn retain_topology_node(&mut self, mut node: ArenaNode) -> NodeId {
        self.initialize_shared_topology();
        let id = NodeId(u32::try_from(self.nodes.len()).expect("owning node arena overflow"));
        node.input_id = id;
        self.nodes.push(node);
        self.shared_topology
            .node_owners
            .push(OwnerNodeGraphField::new(None));
        self.shared_topology.source_input_nodes.push(None);
        self.shared_topology.views[0]
            .as_mut()
            .expect("initialized original graph view")
            .local_to_canonical
            .push(id);
        self.node_order_membership_valid = false;
        self.node_order_membership.resize(self.nodes.len(), false);
        self.external_edge_counts.resize(self.nodes.len(), 0);
        self.placement_component.resize(self.nodes.len(), None);
        self.long_distance_neighbor_data
            .resize(self.nodes.len(), None);
        self.descendant_cache.clear();
        id
    }

    /// Bind a separate real vessel entity. A member carrier is never accepted
    /// as an alias for the vessel, even when its projected box is identical.
    pub(super) fn bind_topology_vessel(
        &mut self,
        vessel: TopologyVessel,
        node: NodeId,
    ) -> Result<(), OwnerTopologyError> {
        self.initialize_shared_topology();
        let actual = self
            .topology_node(node)
            .ok_or(OwnerTopologyError::UnknownNode(node))?;
        let (members, tala_id) = match vessel {
            TopologyVessel::Cluster(index) => {
                let cluster = self
                    .clusters
                    .get(index)
                    .ok_or(OwnerTopologyError::InvalidVessel(vessel))?;
                (&cluster.members, cluster.vessel_tala_id)
            }
            TopologyVessel::Sequence(index) => {
                let sequence = self
                    .sequences
                    .get(index)
                    .ok_or(OwnerTopologyError::InvalidVessel(vessel))?;
                (&sequence.members, sequence.vessel_tala_id)
            }
        };
        if members.contains(&node) {
            return Err(OwnerTopologyError::VesselAliasesMember { vessel, node });
        }
        if actual.tala_id != tala_id
            || self
                .topology_node_vessel(node)
                .is_some_and(|kind| kind != vessel)
        {
            return Err(OwnerTopologyError::VesselIdentityMismatch { vessel, node });
        }
        if self
            .topology_vessel_node(vessel)
            .is_some_and(|existing| existing != node)
        {
            return Err(OwnerTopologyError::VesselIdentityMismatch { vessel, node });
        }
        self.set_topology_vessel_reference(vessel, Some(node))?;
        Ok(())
    }

    /// Actual map headers govern geometry update's preflight checks. Compact
    /// groups without owner bindings remain separate numerical source objects.
    pub(super) fn geometry_update_collection_lengths(
        &self,
        receiver: OwnerGraphId,
    ) -> Option<[(&'static str, usize); 8]> {
        let view = self.owner_graph_view(receiver)?;
        let clusters = self
            .clusters
            .iter()
            .enumerate()
            .filter(|(i, _)| {
                self.owner_group_binding(TopologyVessel::Cluster(*i))
                    .is_none()
            })
            .count();
        let sequences = self
            .sequences
            .iter()
            .enumerate()
            .filter(|(i, _)| {
                self.owner_group_binding(TopologyVessel::Sequence(*i))
                    .is_none()
            })
            .count();
        let directions = view
            .directions
            .as_ref()
            .map(|header| {
                header
                    .entries
                    .as_ref()
                    .map(|entries| entries.read().expect("direction map").len())
                    .unwrap_or(0)
            })
            .unwrap_or_else(|| {
                self.shared_topology
                    .placement_directions
                    .as_ref()
                    .map(|map| map.len())
                    .unwrap_or(
                        self.directions.len() + self.shared_topology.direction_extensions.len(),
                    )
            });
        Some([
            ("container", self.owner_container_map(receiver)?.len()),
            ("cluster", view.clusters.header().len() + clusters),
            ("sequence", view.sequences.header().len() + sequences),
            ("tree", view.tree_roots.len()),
            ("node-tree", view.node_to_tree.len()),
            ("hub", view.hubs.len()),
            ("direction", directions),
            ("sibling", view.common_uncle_siblings.len()),
        ])
    }

    pub(super) fn owner_graph_view(&self, owner: OwnerGraphId) -> Option<&OwnerGraphView> {
        self.shared_topology.views.get(owner.0)?.as_ref()
    }

    pub(super) fn bind_owner_placement_directions(&mut self) {
        self.initialize_shared_topology();
        if self.shared_topology.placement_directions.is_none() {
            self.shared_topology.placement_directions = Some(
                self.scoring_directions
                    .iter()
                    .map(|(&key, &direction)| (key, layout_orientation(direction)))
                    .collect(),
            );
        }
    }

    /// graph.go102–106 preserves a missing key separately from a present
    /// geo.NONE value. Legacy member views share the entity table; explicit
    /// headers support independent maps and CopyEntitiesFrom aliases.
    /// Read the live cardinal entries rather than freezing an initial copy.
    pub(super) fn owner_graph_direction_entry(
        &self,
        owner: OwnerGraphId,
        container: Option<NodeId>,
    ) -> Result<Option<Orientation>, OwnerTopologyError> {
        self.owner_graph_view(owner)
            .ok_or(OwnerTopologyError::UnknownGraph(owner))?;
        if let Some(container) = container {
            self.require_topology_node(container)?;
        }
        if let Some(header) = &self.owner_graph_view(owner).unwrap().directions {
            return Ok(header.get(container));
        }
        if let Some(directions) = &self.shared_topology.placement_directions {
            return Ok(directions.get(&container).copied());
        }
        match (
            self.directions.get(&container),
            self.shared_topology.direction_extensions.get(&container),
        ) {
            (Some(_), Some(_)) => Err(OwnerTopologyError::DirectionEntryConflict(container)),
            (Some(direction), None) => Ok(Some(layout_orientation(*direction))),
            (None, Some(direction)) => Ok(Some(*direction)),
            (None, None) => Ok(None),
        }
    }

    /// Some(Orientation::None) is an actual present geo.NONE entry; None
    /// deletes the key. A checked assignment replaces either representation,
    /// including recovery from a conflicting direct legacy cardinal edit.
    pub(super) fn set_owner_graph_direction_entry(
        &mut self,
        owner: OwnerGraphId,
        container: Option<NodeId>,
        direction: Option<Orientation>,
    ) -> Result<(), OwnerTopologyError> {
        self.owner_graph_view(owner)
            .ok_or(OwnerTopologyError::UnknownGraph(owner))?;
        if let Some(container) = container {
            self.require_topology_node(container)?;
        }
        if let Some(header) = &self.owner_graph_view(owner).unwrap().directions {
            header.set(container, direction);
            return Ok(());
        }
        if let Some(directions) = &mut self.shared_topology.placement_directions {
            if let Some(direction) = direction {
                directions.insert(container, direction);
            } else {
                directions.remove(&container);
            }
            return Ok(());
        }
        self.directions.remove(&container);
        self.shared_topology.direction_extensions.remove(&container);
        if let Some(direction) = direction {
            let cardinal = match direction {
                Orientation::Right => Some(Direction::Right),
                Orientation::Left => Some(Direction::Left),
                Orientation::Top => Some(Direction::Up),
                Orientation::Bottom => Some(Direction::Down),
                _ => None,
            };
            if let Some(cardinal) = cardinal {
                self.directions.insert(container, cardinal);
            } else {
                self.shared_topology
                    .direction_extensions
                    .insert(container, direction);
            }
        }
        Ok(())
    }

    /// Graph.CellSize belongs to the actual Graph receiver (graph.go30).
    /// View0 delegates to the live field, including later stage publication.
    pub(super) fn owner_graph_cell_size(
        &self,
        owner: OwnerGraphId,
    ) -> Result<f64, OwnerTopologyError> {
        let view = self
            .owner_graph_view(owner)
            .ok_or(OwnerTopologyError::UnknownGraph(owner))?;
        Ok(view
            .score_state
            .as_ref()
            .map_or(self.cell_size, |state| state.cell_size))
    }

    pub(super) fn set_owner_graph_cell_size(
        &mut self,
        owner: OwnerGraphId,
        value: f64,
    ) -> Result<(), OwnerTopologyError> {
        let view = self
            .shared_topology
            .views
            .get_mut(owner.0)
            .and_then(Option::as_mut)
            .ok_or(OwnerTopologyError::UnknownGraph(owner))?;
        if let Some(state) = view.score_state.as_mut() {
            state.cell_size = value;
        } else {
            self.cell_size = value;
        }
        Ok(())
    }

    /// graph.go290–311 uses only Graph.Nodes' actual dimensions. It does not
    /// reset routing costs, use retained non-members, or inherit the parent.
    pub(super) fn compute_owner_graph_cell_size(
        &mut self,
        owner: OwnerGraphId,
    ) -> Result<f64, OwnerTopologyError> {
        let view = self
            .owner_graph_view(owner)
            .ok_or(OwnerTopologyError::UnknownGraph(owner))?;
        let mut min_width = f64::INFINITY;
        let mut min_height = f64::INFINITY;
        let mut max_width = f64::NEG_INFINITY;
        let mut max_height = f64::NEG_INFINITY;
        for node in view.nodes.iter_slots() {
            let node = node.expect("nil graph member");
            let actual = self
                .topology_node(node)
                .ok_or(OwnerTopologyError::UnknownNode(node))?;
            min_width = routing::channel_solver::minimum(min_width, actual.rect.size.width);
            min_height = routing::channel_solver::minimum(min_height, actual.rect.size.height);
            max_width = routing::channel_solver::maximum(max_width, actual.rect.size.width);
            max_height = routing::channel_solver::maximum(max_height, actual.rect.size.height);
        }
        let min_length = routing::channel_solver::minimum(min_width, min_height);
        let max_length = routing::channel_solver::maximum(max_width, max_height);
        let value = if max_length < 3.0 * min_length {
            max_length.ceil()
        } else {
            ((3.0 * min_length) / 2.0).ceil()
        };
        let value = routing::channel_solver::maximum(value, 10.0);
        self.set_owner_graph_cell_size(owner, value)?;
        Ok(value)
    }

    pub(super) fn owner_graph_cost_cache(
        &self,
        owner: OwnerGraphId,
    ) -> Result<&Arc<GraphCostCache>, OwnerTopologyError> {
        let view = self
            .owner_graph_view(owner)
            .ok_or(OwnerTopologyError::UnknownGraph(owner))?;
        Ok(view
            .score_state
            .as_ref()
            .map_or(&self.cost_cache, |state| &state.costs))
    }

    /// optimization_guard.go825: count this actual Graph's cache before the
    /// placement domain charges its work. Retained nodes' Graphs do not select
    /// the receiver of a whole-pass snapshot.
    pub(super) fn owner_graph_edge_length_cache_entries(
        &self,
        owner: OwnerGraphId,
    ) -> Result<usize, OwnerTopologyError> {
        Ok(self.owner_graph_cost_cache(owner)?.edge_length_entries())
    }

    /// placement_cost.go55–68: inert raw values and the original optional map
    /// allocation. The placement caller supplies charging and rollback arming.
    pub(super) fn owner_graph_snapshot_placement_costs(
        &self,
        owner: OwnerGraphId,
    ) -> Result<PlacementCostSnapshot, OwnerTopologyError> {
        Ok(self.owner_graph_cost_cache(owner)?.snapshot_placement())
    }

    pub(super) fn owner_graph_routing_costs(
        &self,
        owner: OwnerGraphId,
    ) -> Result<(f64, f64, f64), OwnerTopologyError> {
        Ok(self.owner_graph_cost_cache(owner)?.raw())
    }

    pub(super) fn restore_owner_graph_routing_costs(
        &self,
        owner: OwnerGraphId,
        values: (f64, f64, f64),
    ) -> Result<(), OwnerTopologyError> {
        self.owner_graph_cost_cache(owner)?.restore_routing(values);
        Ok(())
    }

    pub(super) fn reset_owner_graph_turn_cost(
        &self,
        owner: OwnerGraphId,
    ) -> Result<(), OwnerTopologyError> {
        self.owner_graph_cost_cache(owner)?
            .set(RoutingCostField::Turn, 0.0);
        Ok(())
    }

    pub(super) fn halve_owner_graph_turn_cost(
        &self,
        owner: OwnerGraphId,
    ) -> Result<(), OwnerTopologyError> {
        self.owner_graph_cost_cache(owner)?.halve_turn();
        Ok(())
    }

    /// graph.go627–647 scans Graph.Nodes and every actual Node.Edges pointer,
    /// including an edge omitted from Graph.Edges. Node.distanceTo(true) is
    /// shortest Euclidean box distance, without center bias or turn penalties.
    pub(super) fn owner_graph_max_edge_length(
        &self,
        owner: OwnerGraphId,
    ) -> Result<f64, OwnerTopologyError> {
        let view = self
            .owner_graph_view(owner)
            .ok_or(OwnerTopologyError::UnknownGraph(owner))?;
        let mut has_length = false;
        let mut max_length = 60.0;
        for node in view.nodes.iter_slots() {
            let node = node.expect("nil graph member");
            let actual = self
                .topology_node(node)
                .ok_or(OwnerTopologyError::UnknownNode(node))?;
            let Some(position) = self.position(node) else {
                continue;
            };
            for &edge in &actual.edges.read() {
                let edge = self
                    .topology_edge(edge)
                    .ok_or(OwnerTopologyError::UnknownEdge(edge))?;
                let adjacent = if edge.from == node {
                    edge.to
                } else {
                    edge.from
                };
                let adjacent_actual = self
                    .topology_node(adjacent)
                    .ok_or(OwnerTopologyError::UnknownNode(adjacent))?;
                let Some(adjacent_position) = self.position(adjacent) else {
                    continue;
                };
                // True vessel allocation/publication remains a separate
                // contract. Historical member carriers cannot supply it.
                for (id, operand) in [(node, actual), (adjacent, adjacent_actual)] {
                    if operand.cluster.is_some() || operand.sequence.is_some() {
                        return Err(OwnerTopologyError::UnboundCostGeometry(id));
                    }
                }
                has_length = true;
                let interval_gap = |first_start, first_end, second_start, second_end| {
                    if first_end < second_start {
                        second_start - first_end
                    } else if second_end < first_start {
                        first_start - second_end
                    } else {
                        0.0
                    }
                };
                let dx: f64 = interval_gap(
                    position.x,
                    position.x + actual.rect.size.width,
                    adjacent_position.x,
                    adjacent_position.x + adjacent_actual.rect.size.width,
                );
                let dy: f64 = interval_gap(
                    position.y,
                    position.y + actual.rect.size.height,
                    adjacent_position.y,
                    adjacent_position.y + adjacent_actual.rect.size.height,
                );
                let distance = if dx == 0.0 {
                    dy.abs()
                } else if dy == 0.0 {
                    dx.abs()
                } else {
                    (dx * dx + dy * dy).sqrt()
                };
                max_length = routing::channel_solver::maximum(max_length, distance);
            }
        }
        Ok(if has_length { max_length } else { 0.0 })
    }

    fn owner_graph_lazy_cost(
        &self,
        owner: OwnerGraphId,
        field: RoutingCostField,
        compute: impl FnOnce() -> Result<Option<f64>, OwnerTopologyError>,
    ) -> Result<f64, OwnerTopologyError> {
        let mut failure = None;
        let value = self
            .owner_graph_cost_cache(owner)?
            .get_or_compute_optional(field, || match compute() {
                Ok(value) => value,
                Err(error) => {
                    failure = Some(error);
                    None
                }
            });
        failure.map_or(Ok(value), Err)
    }

    pub(super) fn owner_graph_turn_cost(
        &self,
        owner: OwnerGraphId,
    ) -> Result<f64, OwnerTopologyError> {
        self.owner_graph_lazy_cost(owner, RoutingCostField::Turn, || {
            Ok(Some(
                (0.125 * self.owner_graph_view(owner).unwrap().edges.len() as f64)
                    * self.owner_graph_max_edge_length(owner)?,
            ))
        })
    }

    pub(super) fn owner_graph_crossing_cost(
        &self,
        owner: OwnerGraphId,
    ) -> Result<f64, OwnerTopologyError> {
        self.owner_graph_lazy_cost(owner, RoutingCostField::Crossing, || {
            Ok(Some(
                (0.110592_f64 * self.owner_graph_view(owner).unwrap().edges.len() as f64)
                    * self.owner_graph_max_edge_length(owner)?,
            ))
        })
    }

    pub(super) fn owner_graph_non_center_port_cost(
        &self,
        owner: OwnerGraphId,
    ) -> Result<f64, OwnerTopologyError> {
        self.owner_graph_lazy_cost(owner, RoutingCostField::NonCenterPort, || {
            let view = self.owner_graph_view(owner).unwrap();
            if view.edges.is_empty() {
                return Ok(None);
            }
            let mut cost = (0.04287499999999999_f64 * view.edges.len() as f64)
                * self.owner_graph_max_edge_length(owner)?;
            let mut min_size = f64::INFINITY;
            for node in view.nodes.iter_slots() {
                let node = node.expect("nil graph member");
                let actual = self
                    .topology_node(node)
                    .ok_or(OwnerTopologyError::UnknownNode(node))?;
                if actual.is_container {
                    continue;
                }
                min_size = routing::channel_solver::minimum(min_size, actual.rect.size.height);
                min_size = routing::channel_solver::minimum(min_size, actual.rect.size.width);
            }
            if min_size != f64::INFINITY {
                cost = routing::channel_solver::maximum(cost, min_size / 3.0);
            }
            Ok(Some(cost))
        })
    }

    /// Source Clone copies raw routing values into independent Graph owners
    /// with new empty placement maps. Snapshot clones retain the same owner
    /// instead; callers invoke this only at the independent-fork boundary.
    pub(super) fn detach_owner_graph_score_states(&mut self) {
        for view in self.shared_topology.views.iter_mut().flatten() {
            if let Some(state) = view.score_state.as_mut() {
                state.costs = Arc::new(GraphCostCache::new(state.costs.raw()));
            }
        }
    }

    /// Source NewTree permits a nil Node and leaves the other fields at their
    /// zero values except for an allocated empty Children slice.
    pub(super) fn new_owner_tree(
        &mut self,
        node: Option<NodeId>,
    ) -> Result<OwnerTreeId, OwnerTopologyError> {
        self.initialize_shared_topology();
        if let Some(node) = node {
            self.require_topology_node(node)?;
        }
        let tree = OwnerTreeId::new(
            self.topology_arena_id().expect("initialized owner graph"),
            self.shared_topology.trees.len(),
        );
        self.shared_topology
            .trees
            .push((tree, Arc::new(OwnerTreeRecord::new(node))));
        Ok(tree)
    }

    pub(super) fn owner_tree(&self, tree: OwnerTreeId) -> Option<&OwnerTreeRecord> {
        (Some(tree.arena()) == self.topology_arena_id())
            .then(|| self.shared_topology.trees.get(tree.index()))
            .flatten()
            .filter(|(actual, _)| *actual == tree)
            .map(|(_, record)| record.as_ref())
    }

    pub(super) fn retain_owner_tree(&self, tree: OwnerTreeId) -> Option<Arc<OwnerTreeRecord>> {
        (Some(tree.arena()) == self.topology_arena_id())
            .then(|| self.shared_topology.trees.get(tree.index()))
            .flatten()
            .filter(|(actual, _)| *actual == tree)
            .map(|(_, record)| Arc::clone(record))
    }
    #[cfg(feature = "diagnostic-traces")]
    pub(super) fn retain_probe_tree(&self, tree: OwnerTreeId) -> Option<Arc<OwnerTreeRecord>> {
        self.retain_owner_tree(tree)
    }

    fn require_owner_tree(&self, tree: OwnerTreeId) -> Result<(), OwnerTopologyError> {
        if Some(tree.arena()) != self.topology_arena_id() {
            return Err(OwnerTopologyError::ForeignArena {
                expected: self.topology_arena_id(),
                actual: Some(tree.arena()),
            });
        }
        self.owner_tree(tree)
            .ok_or(OwnerTopologyError::UnknownTree(tree))?;
        Ok(())
    }

    pub(super) fn set_owner_tree_orientation(
        &mut self,
        tree: OwnerTreeId,
        orientation: i64,
    ) -> Result<(), OwnerTopologyError> {
        self.require_owner_tree(tree)?;
        self.owner_tree(tree)
            .expect("validated Tree")
            .set_orientation(orientation);
        Ok(())
    }

    pub(super) fn set_owner_tree_fields(
        &mut self,
        tree: OwnerTreeId,
        fields: super::owner_tree_map::OwnerTreeFields,
    ) -> Result<(), OwnerTopologyError> {
        self.require_owner_tree(tree)?;
        if let Some(node) = fields.node {
            self.require_topology_node(node)?;
        }
        if let Some(parent) = fields.parent {
            self.require_owner_tree(parent)?;
        }
        if let Some(edge) = fields.sentinel_edge {
            self.require_topology_edge(edge)?;
        }
        for index in 0..fields.children.capacity() {
            if let Some(Some(child)) = fields.children.read_capacity_slot(index) {
                self.require_owner_tree(child)?;
            }
        }
        self.owner_tree(tree)
            .expect("validated Tree")
            .set_fields(fields);
        Ok(())
    }

    pub(super) fn owner_graph_tree_roots_map(
        &self,
        receiver: OwnerGraphId,
    ) -> Result<OwnerTreeRootsMap, OwnerTopologyError> {
        let view = self
            .owner_graph_view(receiver)
            .ok_or(OwnerTopologyError::UnknownGraph(receiver))?;
        Ok(OwnerTreeRootsMap {
            arena: self.topology_arena_id().expect("initialized owner graph"),
            scope: self.shared_topology.member_scope,
            map: view.tree_roots.clone(),
        })
    }

    pub(super) fn allocate_owner_tree_roots_map(&mut self, present: bool) -> OwnerTreeRootsMap {
        self.initialize_shared_topology();
        OwnerTreeRootsMap {
            arena: self.topology_arena_id().expect("initialized owner graph"),
            scope: self.shared_topology.member_scope,
            map: if present {
                SharedOwnerTreeRootsMap::allocated()
            } else {
                SharedOwnerTreeRootsMap::nil()
            },
        }
    }

    fn validate_tree_roots_scope(
        &self,
        arena: OwnerArenaId,
        scope: &MemberScopeId,
    ) -> Result<(), OwnerTopologyError> {
        if Some(arena) != self.topology_arena_id() {
            return Err(OwnerTopologyError::ForeignArena {
                expected: self.topology_arena_id(),
                actual: Some(arena),
            });
        }
        if !MemberScopeId::same_scope(scope, &self.shared_topology.member_scope) {
            return Err(OwnerTopologyError::ForeignTreeRootsScope);
        }
        Ok(())
    }

    fn validate_tree_roots_slice(&self, roots: &OwnerTreeSlice) -> Result<(), OwnerTopologyError> {
        for index in 0..roots.capacity() {
            if let Some(Some(tree)) = roots.read_capacity_slot(index) {
                self.require_owner_tree(tree)?;
            }
        }
        Ok(())
    }

    pub(super) fn set_owner_graph_tree_roots_map(
        &mut self,
        receiver: OwnerGraphId,
        map: OwnerTreeRootsMap,
    ) -> Result<(), OwnerTopologyError> {
        self.owner_graph_view(receiver)
            .ok_or(OwnerTopologyError::UnknownGraph(receiver))?;
        self.validate_tree_roots_scope(map.arena, &map.scope)?;
        for (node, roots) in map.map.entries() {
            if let Some(node) = node {
                self.require_topology_node(node)?;
            }
            self.validate_tree_roots_slice(&roots)?;
        }
        self.shared_topology.views[receiver.0]
            .as_mut()
            .expect("validated owner graph")
            .tree_roots = map.map;
        Ok(())
    }

    pub(super) fn copy_owner_graph_tree_roots_from(
        &mut self,
        receiver: OwnerGraphId,
        source: OwnerGraphId,
    ) -> Result<(), OwnerTopologyError> {
        let map = self.owner_graph_tree_roots_map(source)?;
        self.set_owner_graph_tree_roots_map(receiver, map)
    }

    pub(super) fn allocate_owner_tree_roots_header(
        &mut self,
        slots: &[Option<OwnerTreeId>],
        capacity: usize,
        present: bool,
    ) -> Result<OwnerTreeRootsHeader, OwnerTopologyError> {
        self.initialize_shared_topology();
        let slice = if present {
            OwnerTreeSlice::from_slots(slots, capacity)
                .ok_or_else(|| OwnerTopologyError::ContainerSlice("Tree slice bounds".into()))?
        } else {
            if !slots.is_empty() || capacity != 0 {
                return Err(OwnerTopologyError::ContainerSlice(
                    "nil Tree slice bounds".into(),
                ));
            }
            OwnerTreeSlice::nil()
        };
        self.validate_tree_roots_slice(&slice)?;
        Ok(OwnerTreeRootsHeader {
            arena: self.topology_arena_id().expect("initialized owner graph"),
            scope: self.shared_topology.member_scope,
            slice,
        })
    }

    /// Root and Children headers may intentionally overlap the same backing.
    pub(super) fn owner_tree_children_header(
        &self,
        tree: OwnerTreeId,
    ) -> Result<OwnerTreeRootsHeader, OwnerTopologyError> {
        self.require_owner_tree(tree)?;
        Ok(OwnerTreeRootsHeader {
            arena: self.topology_arena_id().expect("initialized owner graph"),
            scope: self.shared_topology.member_scope,
            slice: self
                .owner_tree(tree)
                .expect("validated Tree")
                .fields()
                .children,
        })
    }

    pub(super) fn set_owner_tree_children_header(
        &mut self,
        tree: OwnerTreeId,
        children: &OwnerTreeRootsHeader,
    ) -> Result<(), OwnerTopologyError> {
        self.require_owner_tree(tree)?;
        self.validate_tree_roots_scope(children.arena, &children.scope)?;
        self.validate_tree_roots_slice(&children.slice)?;
        let mut fields = self.owner_tree(tree).expect("validated Tree").fields();
        fields.children = children.slice.clone();
        self.set_owner_tree_fields(tree, fields)
    }

    pub(super) fn owner_tree_roots_map_entry(
        &self,
        map: &OwnerTreeRootsMap,
        node: Option<NodeId>,
    ) -> Result<Option<OwnerTreeRootsHeader>, OwnerTopologyError> {
        self.validate_tree_roots_scope(map.arena, &map.scope)?;
        if let Some(node) = node {
            self.require_topology_node(node)?;
        }
        let Some(slice) = map.map.get(node) else {
            return Ok(None);
        };
        self.validate_tree_roots_slice(&slice)?;
        Ok(Some(OwnerTreeRootsHeader {
            arena: map.arena,
            scope: map.scope,
            slice,
        }))
    }

    pub(super) fn owner_graph_tree_roots_entry(
        &self,
        receiver: OwnerGraphId,
        node: Option<NodeId>,
    ) -> Result<Option<OwnerTreeRootsHeader>, OwnerTopologyError> {
        let view = self
            .owner_graph_view(receiver)
            .ok_or(OwnerTopologyError::UnknownGraph(receiver))?;
        if let Some(node) = node {
            self.require_topology_node(node)?;
        }
        let Some(slice) = view.tree_roots.get(node) else {
            return Ok(None);
        };
        self.validate_tree_roots_slice(&slice)?;
        Ok(Some(OwnerTreeRootsHeader {
            arena: self.topology_arena_id().expect("initialized owner graph"),
            scope: self.shared_topology.member_scope,
            slice,
        }))
    }

    pub(super) fn write_owner_tree_roots_entry(
        &mut self,
        map: &OwnerTreeRootsMap,
        node: Option<NodeId>,
        roots: &OwnerTreeRootsHeader,
    ) -> Result<(), OwnerTopologyError> {
        self.validate_tree_roots_scope(map.arena, &map.scope)?;
        self.validate_tree_roots_scope(roots.arena, &roots.scope)?;
        if let Some(node) = node {
            self.require_topology_node(node)?;
        }
        self.validate_tree_roots_slice(&roots.slice)?;
        map.map
            .insert(node, roots.slice.clone())
            .map_err(|()| OwnerTopologyError::NilTreeRootsMapAssignment)
    }

    pub(super) fn delete_owner_tree_roots_entry(
        &mut self,
        map: &OwnerTreeRootsMap,
        node: Option<NodeId>,
    ) -> Result<(), OwnerTopologyError> {
        self.validate_tree_roots_scope(map.arena, &map.scope)?;
        if let Some(node) = node {
            self.require_topology_node(node)?;
        }
        map.map.delete(node);
        Ok(())
    }

    pub(super) fn write_owner_tree_roots_slot(
        &mut self,
        roots: &OwnerTreeRootsHeader,
        index: usize,
        tree: Option<OwnerTreeId>,
    ) -> Result<(), OwnerTopologyError> {
        self.validate_tree_roots_scope(roots.arena, &roots.scope)?;
        if let Some(tree) = tree {
            self.require_owner_tree(tree)?;
        }
        if roots.slice.write_capacity_slot(index, tree) {
            Ok(())
        } else {
            Err(OwnerTopologyError::ContainerSlice(
                "Tree slice bounds".into(),
            ))
        }
    }

    /// The outer Option reports key presence. Some(None) is a present nil
    /// Tree value, which still excludes the node in source optimizerEligible.
    /// No Node.Graph or graph-membership substitution occurs here.
    pub(super) fn owner_graph_node_to_tree_entry(
        &self,
        receiver: OwnerGraphId,
        node: Option<NodeId>,
    ) -> Result<Option<Option<OwnerTreeId>>, OwnerTopologyError> {
        let view = self
            .owner_graph_view(receiver)
            .ok_or(OwnerTopologyError::UnknownGraph(receiver))?;
        if let Some(node) = node {
            self.require_topology_node(node)?;
        }
        let value = view.node_to_tree.get(node);
        if let Some(Some(tree)) = value {
            self.require_owner_tree(tree)?;
        }
        Ok(value)
    }

    /// Read from a retained range header rather than a possibly replaced field.
    pub(super) fn owner_node_to_tree_map_entry(
        &self,
        map: &OwnerNodeToTreeMap,
        node: Option<NodeId>,
    ) -> Result<Option<Option<OwnerTreeId>>, OwnerTopologyError> {
        self.validate_owner_node_to_tree_map(map)?;
        if let Some(node) = node {
            self.require_topology_node(node)?;
        }
        let value = map.map.get(node);
        if let Some(Some(tree)) = value {
            self.require_owner_tree(tree)?;
        }
        Ok(value)
    }

    pub(super) fn owner_graph_node_to_tree_map(
        &self,
        receiver: OwnerGraphId,
    ) -> Result<OwnerNodeToTreeMap, OwnerTopologyError> {
        let view = self
            .owner_graph_view(receiver)
            .ok_or(OwnerTopologyError::UnknownGraph(receiver))?;
        Ok(OwnerNodeToTreeMap {
            arena: self.topology_arena_id().expect("initialized owner graph"),
            scope: self.shared_topology.member_scope,
            map: view.node_to_tree.clone(),
        })
    }

    pub(super) fn allocate_owner_node_to_tree_map(&mut self, present: bool) -> OwnerNodeToTreeMap {
        self.initialize_shared_topology();
        OwnerNodeToTreeMap {
            arena: self.topology_arena_id().expect("initialized owner graph"),
            scope: self.shared_topology.member_scope,
            map: if present {
                SharedOwnerTreeMap::allocated()
            } else {
                SharedOwnerTreeMap::nil()
            },
        }
    }

    fn validate_owner_node_to_tree_map(
        &self,
        map: &OwnerNodeToTreeMap,
    ) -> Result<(), OwnerTopologyError> {
        if Some(map.arena) != self.topology_arena_id() {
            return Err(OwnerTopologyError::ForeignArena {
                expected: self.topology_arena_id(),
                actual: Some(map.arena),
            });
        }
        if !MemberScopeId::same_scope(&map.scope, &self.shared_topology.member_scope) {
            return Err(OwnerTopologyError::ForeignNodeToTreeScope);
        }
        Ok(())
    }

    pub(super) fn set_owner_graph_node_to_tree_map(
        &mut self,
        receiver: OwnerGraphId,
        map: OwnerNodeToTreeMap,
    ) -> Result<(), OwnerTopologyError> {
        self.owner_graph_view(receiver)
            .ok_or(OwnerTopologyError::UnknownGraph(receiver))?;
        self.validate_owner_node_to_tree_map(&map)?;
        map.map.validate_entries(|node, tree| {
            if let Some(node) = node {
                self.require_topology_node(node)?;
            }
            if let Some(tree) = tree {
                self.require_owner_tree(tree)?;
            }
            Ok::<(), OwnerTopologyError>(())
        })?;
        self.shared_topology.views[receiver.0]
            .as_mut()
            .expect("validated owner graph")
            .node_to_tree = map.map;
        Ok(())
    }

    /// Graph.CopyEntitiesFrom assigns this header, including a nil one. It
    /// does not redirect any Node.Graph pointer or copy a member slice.
    pub(super) fn copy_owner_graph_node_to_tree_from(
        &mut self,
        receiver: OwnerGraphId,
        source: OwnerGraphId,
    ) -> Result<(), OwnerTopologyError> {
        let map = self.owner_graph_node_to_tree_map(source)?;
        self.set_owner_graph_node_to_tree_map(receiver, map)
    }

    pub(super) fn write_owner_node_to_tree_entry(
        &self,
        map: &OwnerNodeToTreeMap,
        node: Option<NodeId>,
        tree: Option<OwnerTreeId>,
    ) -> Result<(), OwnerTopologyError> {
        self.validate_owner_node_to_tree_map(map)?;
        if let Some(node) = node {
            self.require_topology_node(node)?;
        }
        if let Some(tree) = tree {
            self.require_owner_tree(tree)?;
        }
        map.map
            .insert(node, tree)
            .map_err(|()| OwnerTopologyError::NilNodeToTreeMapAssignment)
    }

    pub(super) fn delete_owner_node_to_tree_entry(
        &mut self,
        map: &OwnerNodeToTreeMap,
        node: Option<NodeId>,
    ) -> Result<(), OwnerTopologyError> {
        self.validate_owner_node_to_tree_map(map)?;
        if let Some(node) = node {
            self.require_topology_node(node)?;
        }
        map.map.delete(node);
        Ok(())
    }

    /// Borrow current Graph.Nodes without retaining its backing. This view is
    /// restricted to the interval between owner callbacks/mutations.
    pub(super) fn owner_graph_members_read(
        &self,
        receiver: OwnerGraphId,
    ) -> Result<crate::internal::go_cell::GoCellRef<'_, SharedNodeSlice>, OwnerTopologyError> {
        let view = self
            .owner_graph_view(receiver)
            .ok_or(OwnerTopologyError::UnknownGraph(receiver))?;
        Ok(view.nodes.read())
    }

    pub(super) fn owner_graph_member_header(
        &self,
        receiver: OwnerGraphId,
    ) -> Result<OwnerMemberHeader, OwnerTopologyError> {
        let view = self
            .owner_graph_view(receiver)
            .ok_or(OwnerTopologyError::UnknownGraph(receiver))?;
        Ok(OwnerMemberHeader {
            arena: self.topology_arena_id().expect("initialized owner graph"),
            scope: self.shared_topology.member_scope,
            slice: view.nodes.header(),
        })
    }

    /// An authored make(len, cap) retains nil slots and exact capacity. Rust
    /// Vec capacity is never used as a substitute for the source header.
    pub(super) fn allocate_owner_member_header(
        &mut self,
        slots: &[Option<NodeId>],
        capacity: usize,
    ) -> Result<OwnerMemberHeader, OwnerTopologyError> {
        self.initialize_shared_topology();
        for &node in slots.iter().flatten() {
            self.require_topology_node(node)?;
        }
        let slice = SharedNodeSlice::with_len_cap(slots.len(), capacity)
            .map_err(|error| OwnerTopologyError::ContainerSlice(error.to_string()))?;
        for (index, &slot) in slots.iter().enumerate() {
            slice
                .write_capacity_slot(index, slot)
                .map_err(|error| OwnerTopologyError::ContainerSlice(error.to_string()))?;
        }
        Ok(OwnerMemberHeader {
            arena: self.topology_arena_id().expect("initialized owner graph"),
            scope: self.shared_topology.member_scope,
            slice,
        })
    }

    pub(super) fn nil_owner_member_header(&mut self) -> OwnerMemberHeader {
        self.initialize_shared_topology();
        OwnerMemberHeader {
            arena: self.topology_arena_id().expect("initialized owner graph"),
            scope: self.shared_topology.member_scope,
            slice: SharedNodeSlice::nil(),
        }
    }

    fn validate_owner_member_header(
        &self,
        header: &OwnerMemberHeader,
    ) -> Result<(), OwnerTopologyError> {
        if Some(header.arena) != self.topology_arena_id() {
            return Err(OwnerTopologyError::ForeignArena {
                expected: self.topology_arena_id(),
                actual: Some(header.arena),
            });
        }
        if !MemberScopeId::same_scope(&header.scope, &self.shared_topology.member_scope) {
            return Err(OwnerTopologyError::ForeignMemberScope);
        }
        Ok(())
    }

    pub(super) fn retain_owner_graph_nodes_field(
        &self,
        receiver: OwnerGraphId,
    ) -> Result<OwnerGraphNodesField, OwnerTopologyError> {
        Ok(self
            .owner_graph_view(receiver)
            .ok_or(OwnerTopologyError::UnknownGraph(receiver))?
            .nodes
            .clone())
    }

    pub(super) fn geometry_members_snapshot(
        &self,
        receiver: OwnerGraphId,
    ) -> Result<OwnerGeometryMembersSnapshot, OwnerTopologyError> {
        let header = self.owner_graph_member_header(receiver)?;
        Ok(OwnerGeometryMembersSnapshot {
            receiver,
            field: self.retain_owner_graph_nodes_field(receiver)?,
            visible: header.visible_slots(),
            header,
        })
    }

    pub(super) fn restore_geometry_members(
        &mut self,
        saved: OwnerGeometryMembersSnapshot,
    ) -> Result<(), OwnerTopologyError> {
        self.validate_owner_member_header(&saved.header)?;
        let field = self.retain_owner_graph_nodes_field(saved.receiver)?;
        if !field.same_field(&saved.field) {
            return Err(OwnerTopologyError::ForeignMemberScope);
        }
        for &node in saved.visible.iter().flatten() {
            self.require_topology_node(node)?;
        }
        // transaction.go1412 copies original length only. Spare-capacity writes
        // and longer held aliases must retain their live tail slots.
        for (index, node) in saved.visible.into_iter().enumerate() {
            saved
                .header
                .slice
                .write_capacity_slot(index, node)
                .map_err(|error| OwnerTopologyError::ContainerSlice(error.to_string()))?;
        }
        saved.field.replace(saved.header.slice);
        Ok(())
    }

    pub(super) fn set_owner_graph_member_header(
        &mut self,
        receiver: OwnerGraphId,
        header: OwnerMemberHeader,
    ) -> Result<(), OwnerTopologyError> {
        self.owner_graph_view(receiver)
            .ok_or(OwnerTopologyError::UnknownGraph(receiver))?;
        self.validate_owner_member_header(&header)?;
        // Reslicing can expose spare slots later, so validate all retained
        // capacity slots before publishing any field assignment.
        for index in 0..header.capacity() {
            if let Some(Some(node)) = header.read_capacity_slot(index) {
                self.require_topology_node(node)?;
            }
        }
        self.shared_topology.views[receiver.0]
            .as_mut()
            .expect("validated owner graph")
            .nodes
            .replace(header.slice);
        Ok(())
    }

    pub(super) fn write_owner_member_capacity_slot(
        &mut self,
        header: &OwnerMemberHeader,
        index: usize,
        node: Option<NodeId>,
    ) -> Result<(), OwnerTopologyError> {
        self.validate_owner_member_header(header)?;
        if let Some(node) = node {
            self.require_topology_node(node)?;
        }
        header
            .slice
            .write_capacity_slot(index, node)
            .map_err(|error| OwnerTopologyError::ContainerSlice(error.to_string()))
    }

    /// Source append returns a new header while writing existing spare slots
    /// when available; the retained old header keeps its old length.
    pub(super) fn append_owner_member_header(
        &self,
        header: &OwnerMemberHeader,
        slots: &[Option<NodeId>],
    ) -> Result<OwnerMemberHeader, OwnerTopologyError> {
        self.validate_owner_member_header(header)?;
        for &node in slots.iter().flatten() {
            self.require_topology_node(node)?;
        }
        Ok(OwnerMemberHeader {
            arena: header.arena,
            scope: header.scope,
            slice: header
                .slice
                .append(slots)
                .map_err(|error| OwnerTopologyError::ContainerSlice(error.to_string()))?,
        })
    }

    /// Tree publication can change a graph's member slices without changing
    /// any Node.Graph pointer. Keep that operation separate from ownership.
    pub(super) fn set_owner_graph_members(
        &mut self,
        owner: OwnerGraphId,
        nodes: Vec<NodeId>,
        edges: Vec<EdgeId>,
    ) -> Result<(), OwnerTopologyError> {
        self.initialize_shared_topology();
        for &node in &nodes {
            self.require_topology_node(node)?;
        }
        for &edge in &edges {
            self.require_topology_edge(edge)?;
        }
        let view = self
            .shared_topology
            .views
            .get_mut(owner.0)
            .and_then(Option::as_mut)
            .ok_or(OwnerTopologyError::UnknownGraph(owner))?;
        view.nodes.replace(
            SharedNodeSlice::from_nodes(&nodes)
                .map_err(|error| OwnerTopologyError::ContainerSlice(error.to_string()))?,
        );
        view.edges.replace_values(&edges);
        Ok(())
    }

    /// Graph.Edges membership changes independently of Node.Edges incidence
    /// and the explicit dense edge mapping (graph.go270–287,791–806). Validate
    /// all canonical handles before replacing the actual ordered member slice.
    pub(super) fn set_owner_view_edge_membership(
        &mut self,
        owner: OwnerGraphId,
        edges: Vec<EdgeId>,
    ) -> Result<(), OwnerTopologyError> {
        for &edge in &edges {
            self.require_topology_edge(edge)?;
        }
        self.shared_topology
            .views
            .get_mut(owner.0)
            .and_then(Option::as_mut)
            .ok_or(OwnerTopologyError::UnknownGraph(owner))?
            .edges
            .replace_values(&edges);
        Ok(())
    }

    /// Current Graph.Nodes membership, distinct from retained arena storage.
    /// grouping.AddCluster / AddSequences replace members with a fresh vessel;
    /// the numerical workspace can still use a first-member carrier.
    pub(super) fn original_owner_node(&self, node: NodeId) -> NodeId {
        self.active_cluster_index(node)
            .and_then(|index| self.topology_vessel_node(TopologyVessel::Cluster(index)))
            .or_else(|| {
                self.active_sequence_index(node)
                    .and_then(|index| self.topology_vessel_node(TopologyVessel::Sequence(index)))
            })
            .unwrap_or(node)
    }

    pub(super) fn original_owner_member_nodes(&self) -> Vec<NodeId> {
        self.node_order
            .iter()
            .map(|&node| self.original_owner_node(node))
            .collect()
    }

    pub(super) fn synchronize_original_owner_graph_view(&mut self) {
        self.initialize_shared_topology();
        let nodes = self.original_owner_member_nodes();
        self.shared_topology.views[0]
            .as_mut()
            .expect("initialized original graph view")
            .nodes
            .replace(
                SharedNodeSlice::from_nodes(&nodes)
                    .expect("published original owner member backing allocation"),
            );
        self.shared_topology.views[0]
            .as_mut()
            .expect("initialized original graph view")
            .edges
            .replace_header(self.edge_order.header());
    }

    pub(super) fn owner_view_canonical_node(
        &self,
        owner: OwnerGraphId,
        local: NodeId,
    ) -> Option<NodeId> {
        self.owner_graph_view(owner)?
            .local_to_canonical
            .get(local.0 as usize)
            .copied()
    }

    pub(super) fn owner_view_local_node(
        &self,
        owner: OwnerGraphId,
        canonical: NodeId,
    ) -> Option<NodeId> {
        self.owner_graph_view(owner)?
            .local_to_canonical
            .iter()
            .position(|&node| node == canonical)
            .map(|index| NodeId(index as u32))
    }

    pub(super) fn set_owner_view_node_mapping(
        &mut self,
        owner: OwnerGraphId,
        local_to_canonical: Vec<NodeId>,
    ) -> Result<(), OwnerTopologyError> {
        self.initialize_shared_topology();
        for &node in &local_to_canonical {
            self.require_topology_node(node)?;
        }
        self.shared_topology
            .views
            .get_mut(owner.0)
            .and_then(Option::as_mut)
            .ok_or(OwnerTopologyError::UnknownGraph(owner))?
            .local_to_canonical = local_to_canonical;
        Ok(())
    }

    /// Creating a member view does not redirect Graph pointers. The source
    /// AddNodeUnchecked/SetGraphReference operations below perform that change.
    pub(super) fn create_owner_graph_view(
        &mut self,
        nodes: Vec<NodeId>,
        edges: Vec<EdgeId>,
    ) -> Result<OwnerGraphId, OwnerTopologyError> {
        self.initialize_shared_topology();
        for &node in &nodes {
            self.require_topology_node(node)?;
        }
        for &edge in &edges {
            self.require_topology_edge(edge)?;
        }
        let id = OwnerGraphId(self.shared_topology.views.len());
        self.shared_topology.views.push(Some(OwnerGraphView {
            local_to_canonical: nodes.clone(),
            nodes: OwnerGraphNodesField::new(
                SharedNodeSlice::from_nodes(&nodes)
                    .map_err(|error| OwnerTopologyError::ContainerSlice(error.to_string()))?,
            ),
            edges: edges.into(),
            common_uncle_siblings: GraphNodeMap::nil(),
            hubs: GraphNodeMap::new(),
            node_to_tree: SharedOwnerTreeMap::nil(),
            tree_roots: SharedOwnerTreeRootsMap::allocated(),
            clusters: SharedOwnerGroupMapField::new(SharedOwnerGroupMap::allocated()),
            sequences: SharedOwnerGroupMapField::new(SharedOwnerGroupMap::allocated()),
            score_state: Some(OwnerGraphScoreState::default()),
            directions: None,
            containers: Some(GraphNodeMap::from_header(self.containers.share_map())),
        }));
        Ok(id)
    }

    pub(super) fn add_owner_graph_node(
        &mut self,
        owner: OwnerGraphId,
        node: NodeId,
        journal: &mut OwnerGraphOwnershipJournal,
    ) -> Result<(), OwnerTopologyError> {
        self.initialize_shared_topology();
        journal.bind_arena(self.topology_arena_id())?;
        self.require_topology_node(node)?;
        let view = self
            .shared_topology
            .views
            .get_mut(owner.0)
            .and_then(Option::as_mut)
            .ok_or(OwnerTopologyError::UnknownGraph(owner))?;
        // AddNodeUnchecked deliberately permits duplicates. The actual append
        // may reuse spare backing or grow; Node.Graph changes only afterward.
        view.nodes.replace(
            view.nodes
                .header()
                .append_node(node)
                .map_err(|error| OwnerTopologyError::ContainerSlice(error.to_string()))?,
        );
        view.local_to_canonical.push(node);
        journal
            .prior
            .entry(node)
            .or_insert(self.shared_topology.node_owners[node.0 as usize].get());
        self.shared_topology.node_owners[node.0 as usize].set(Some(owner));
        Ok(())
    }

    pub(super) fn add_owner_graph_edge(
        &mut self,
        owner: OwnerGraphId,
        edge: EdgeId,
    ) -> Result<(), OwnerTopologyError> {
        self.initialize_shared_topology();
        self.require_topology_edge(edge)?;
        let view = self
            .shared_topology
            .views
            .get_mut(owner.0)
            .and_then(Option::as_mut)
            .ok_or(OwnerTopologyError::UnknownGraph(owner))?;
        if !view.edges.contains(&edge) {
            view.edges.push(edge);
        } // Graph.AddEdge.
        Ok(())
    }

    pub(super) fn set_owner_graph_reference(
        &mut self,
        owner: OwnerGraphId,
        roots: &[NodeId],
        include_descendants: bool,
        journal: &mut OwnerGraphOwnershipJournal,
    ) -> Result<(), OwnerTopologyError> {
        self.initialize_shared_topology();
        journal.bind_arena(self.topology_arena_id())?;
        if self.owner_graph_view(owner).is_none() {
            return Err(OwnerTopologyError::UnknownGraph(owner));
        }
        let nodes = if include_descendants {
            self.topology_retained_closure(roots)?
        } else {
            for &node in roots {
                self.require_topology_node(node)?;
            }
            roots.to_vec()
        };
        for node in nodes {
            journal
                .prior
                .entry(node)
                .or_insert(self.shared_topology.node_owners[node.0 as usize].get());
            self.shared_topology.node_owners[node.0 as usize].set(Some(owner));
        }
        Ok(())
    }

    /// Restores pointer ownership only. Member slices belong to graph views
    /// and are not rewound by NodeGraphOwnershipJournal.Restore.
    pub(super) fn restore_owner_graph_ownership(&mut self, journal: &OwnerGraphOwnershipJournal) {
        assert!(
            journal.prior.is_empty() || journal.arena == self.topology_arena_id(),
            "foreign canonical graph owner journal"
        );
        for (&node, &owner) in &journal.prior {
            self.shared_topology.node_owners[node.0 as usize].set(owner);
        }
    }

    /// Source restore's breadth-first entity closure: roots, container
    /// children, sequence members, cluster members, in that order. This is
    /// not membership in Graph.Nodes and never appends nodes to a member view.
    pub(super) fn topology_retained_closure(
        &self,
        roots: &[NodeId],
    ) -> Result<Vec<NodeId>, OwnerTopologyError> {
        let mut queue = VecDeque::from(roots.to_vec());
        let mut seen = BTreeSet::new();
        let mut ordered = Vec::new();
        while let Some(node) = queue.pop_front() {
            self.require_topology_node(node)?;
            if !seen.insert(node) {
                continue;
            }
            ordered.push(node);
            if self.nodes[node.0 as usize].is_container {
                if let Some(Some(children)) = self.topology_container_slice(Some(node)) {
                    queue.extend(children.iter().copied());
                }
            }
            // Retain the source's sequence-before-cluster traversal.
            for kind in [true, false] {
                // Bound references have moved their Vessel authority out of
                // the dense adapter. Read their live member header as well.
                // allDescendantNodesGuarded skips nil descendant slots.
                for (&reference, &group) in self.shared_topology.group_bindings.iter() {
                    let selected = matches!(reference, TopologyVessel::Sequence(_)) == kind;
                    if !selected {
                        continue;
                    }
                    let fields = self
                        .owner_group(group)
                        .ok_or(OwnerTopologyError::UnknownGroup(group))?
                        .fields();
                    if fields.vessel == Some(node) {
                        queue.extend(fields.nodes.iter_slots().flatten());
                    }
                }
                for (&vessel, &vessel_node) in self.shared_topology.vessels.iter() {
                    if vessel_node != Some(node) {
                        continue;
                    }
                    match vessel {
                        TopologyVessel::Sequence(index) if kind => {
                            queue.extend(self.sequences[index].members.iter().copied())
                        }
                        TopologyVessel::Cluster(index) if !kind => {
                            queue.extend(self.clusters[index].members.iter().copied())
                        }
                        _ => {}
                    }
                }
            }
        }
        Ok(ordered)
    }

    fn require_topology_node(&self, node: NodeId) -> Result<(), OwnerTopologyError> {
        self.topology_node(node)
            .map(|_| ())
            .ok_or(OwnerTopologyError::UnknownNode(node))
    }

    fn require_topology_edge(&self, edge: EdgeId) -> Result<(), OwnerTopologyError> {
        self.edge_records
            .get(edge.0 as usize)
            .map(|_| ())
            .ok_or(OwnerTopologyError::UnknownEdge(edge))
    }

    pub(super) fn owner_edge_abduction(
        &self,
        record: OwnerAbductionId,
    ) -> Option<&OwnerEdgeAbduction> {
        self.shared_topology
            .records
            .get(record.0)
            .and_then(Option::as_ref)
    }

    pub(super) fn register_owner_edge_abductions(
        &mut self,
        records: Vec<OwnerEdgeAbduction>,
        present: bool,
    ) -> Result<OwnerAbductionSlice, OwnerTopologyError> {
        self.initialize_shared_topology();
        assert!(
            present || records.is_empty(),
            "nil abduction slice cannot contain records"
        );
        for record in &records {
            if let Some(edge) = record.source_edge {
                self.require_topology_edge(edge)?;
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
                self.require_topology_node(node)?;
            }
        }
        let start = self.shared_topology.records.len();
        let result = OwnerAbductionSlice {
            records: (start..start + records.len())
                .map(OwnerAbductionId)
                .collect(),
            present,
            arena: present.then_some(self.topology_arena_id().expect("initialized owner arena")),
        };
        self.shared_topology
            .records
            .extend(records.into_iter().map(Some));
        Ok(result)
    }

    /// Production construction boundary. The source owning identities must
    /// exist; projected local indices and signed-ID scans cannot supply them.
    pub(super) fn from_sized_owner_abductions(
        &mut self,
        records: &[SizedEdgeAbduction],
        present: bool,
    ) -> Result<OwnerAbductionSlice, OwnerTopologyError> {
        self.initialize_shared_topology();
        let mut resolved = Vec::with_capacity(records.len());
        for record in records {
            let original = |reference: Option<AbductedNodeRef>,
                            side|
             -> Result<Option<NodeId>, OwnerTopologyError> {
                reference
                    .map(|reference| {
                        let node = reference.source_node.ok_or(
                            OwnerTopologyError::MissingSourceLineage {
                                edge: record.source_edge,
                                side,
                            },
                        )?;
                        self.require_topology_node(node)?;
                        Ok(node)
                    })
                    .transpose()
            };
            let current = |reference: Option<AbductionCurrentNodeRef>,
                           side|
             -> Result<Option<NodeId>, OwnerTopologyError> {
                reference
                    .map(|reference| {
                        let node = reference.source_node.ok_or(
                            OwnerTopologyError::MissingSourceLineage {
                                edge: record.source_edge,
                                side,
                            },
                        )?;
                        self.require_topology_node(node)?;
                        if self.topology_node_vessel(node).is_some() {
                            return Ok(node);
                        }
                        if let Some(index) = self.active_cluster_index(node) {
                            let kind = TopologyVessel::Cluster(index);
                            return self
                                .topology_vessel_node(kind)
                                .ok_or(OwnerTopologyError::UnboundVessel(kind));
                        }
                        if let Some(index) = self.active_sequence_index(node) {
                            let kind = TopologyVessel::Sequence(index);
                            return self
                                .topology_vessel_node(kind)
                                .ok_or(OwnerTopologyError::UnboundVessel(kind));
                        }
                        if reference.is_aggregate_vessel {
                            return Err(OwnerTopologyError::MissingSourceLineage {
                                edge: record.source_edge,
                                side,
                            });
                        }
                        Ok(node)
                    })
                    .transpose()
            };
            resolved.push(OwnerEdgeAbduction {
                source_edge: Some(record.source_edge),
                originally_from: original(record.originally_from, "original-from")?,
                originally_to: original(record.originally_to, "original-to")?,
                current_from: current(record.current_from, "current-from")?,
                current_to: current(record.current_to, "current-to")?,
            });
        }
        self.register_owner_edge_abductions(resolved, present)
    }

    fn remove_first_owner_incidence(&mut self, node: NodeId, edge: EdgeId) {
        self.nodes[node.0 as usize].edges.remove_first(edge);
        if let Some(order) = self.incident_edge_order.get_mut(&node) {
            *order = self.nodes[node.0 as usize].edges.read();
        }
    }

    fn append_owner_incidence(&mut self, node: NodeId, edge: EdgeId) {
        self.nodes[node.0 as usize].edges.push(edge);
        if let Some(order) = self.incident_edge_order.get_mut(&node) {
            *order = self.nodes[node.0 as usize].edges.read();
        }
    }

    /// Actual Edge.reconnect. Only the changed endpoint's incidence is
    /// removed/appended; even a same-pointer reconnect reorders its first
    /// occurrence. Never deduplicate or sort a source Node.Edges slice.
    pub(super) fn reconnect_owner_edge_endpoint(
        &mut self,
        edge: EdgeId,
        endpoint: NodeId,
        is_to: bool,
    ) {
        assert!(
            self.topology_node(endpoint).is_some(),
            "unknown canonical endpoint"
        );
        let old = self
            .owner_edge_endpoint(edge, is_to)
            .expect("known edge")
            .expect("runtime error: invalid memory address or nil pointer dereference");
        self.remove_first_owner_incidence(old, edge);
        self.append_owner_incidence(endpoint, edge);
        self.set_owner_edge_endpoint(edge, is_to, Some(endpoint))
            .expect("known endpoint");
    }

    pub(super) fn detach_owner_edge_incidence(&mut self, edge: EdgeId) {
        let from = self
            .owner_edge_endpoint(edge, false)
            .expect("known edge")
            .expect("runtime error: invalid memory address or nil pointer dereference");
        let to = self
            .owner_edge_endpoint(edge, true)
            .expect("known edge")
            .expect("runtime error: invalid memory address or nil pointer dereference");
        self.remove_first_owner_incidence(from, edge);
        self.remove_first_owner_incidence(to, edge);
    }

    /// First application also detaches both-current-nil intra-container
    /// records. Later ApplyEdgeAbductions deliberately does not detach them.
    pub(super) fn abduct_owner_edge_abductions(&mut self, slice: &OwnerAbductionSlice) {
        self.require_owner_slice_arena(slice)
            .expect("foreign canonical abduction slice");
        for &id in &slice.records {
            let record = *self
                .owner_edge_abduction(id)
                .expect("unknown canonical abduction");
            if record.current_from.is_none() && record.current_to.is_none() {
                self.detach_owner_edge_incidence(
                    record
                        .source_edge
                        .expect("runtime error: invalid memory address or nil pointer dereference"),
                );
            } else {
                self.apply_owner_abduction_record(record);
            }
        }
    }

    fn apply_owner_abduction_record(&mut self, record: OwnerEdgeAbduction) {
        for (original, current, is_to) in [
            (record.originally_from, record.current_from, false),
            (record.originally_to, record.current_to, true),
        ] {
            if let (Some(original), Some(current)) = (original, current) {
                if original != current {
                    self.reconnect_owner_edge_endpoint(
                        record.source_edge.expect(
                            "runtime error: invalid memory address or nil pointer dereference",
                        ),
                        current,
                        is_to,
                    );
                }
            }
        }
    }

    pub(super) fn apply_owner_edge_abductions(&mut self, slice: &OwnerAbductionSlice) {
        self.require_owner_slice_arena(slice)
            .expect("foreign canonical abduction slice");
        for &id in &slice.records {
            let record = *self
                .owner_edge_abduction(id)
                .expect("unknown canonical abduction");
            self.apply_owner_abduction_record(record);
        }
    }

    fn restored_owner_endpoint(&self, mut node: NodeId) -> Result<NodeId, OwnerTopologyError> {
        // Source replaces the pointer after the cluster check, then checks
        // Sequence on that new pointer, rather than on the original member.
        if let Some(index) = self.nodes[node.0 as usize].cluster {
            let kind = TopologyVessel::Cluster(index);
            node = self
                .topology_vessel_node(kind)
                .ok_or(OwnerTopologyError::UnboundVessel(kind))?;
        }
        if let Some(index) = self.nodes[node.0 as usize].sequence {
            let kind = TopologyVessel::Sequence(index);
            node = self
                .topology_vessel_node(kind)
                .ok_or(OwnerTopologyError::UnboundVessel(kind))?;
        }
        Ok(node)
    }

    /// Source restore returns a genuinely nil remaining slice when no
    /// foreign records remain. Record handles are retained unchanged; only
    /// original pointers may be reassigned to their real aggregate vessels.
    pub(super) fn restore_owner_edge_abductions(
        &mut self,
        slice: &OwnerAbductionSlice,
        roots: &[NodeId],
    ) -> Result<OwnerAbductionSlice, OwnerTopologyError> {
        self.require_owner_slice_arena(slice)?;
        let closure: BTreeSet<_> = self.topology_retained_closure(roots)?.into_iter().collect();
        let mut remaining = OwnerAbductionSlice::default();
        // Preflight exact vessel resolution before mutating any owner edge.
        let mut eligible = Vec::new();
        for &id in &slice.records {
            let record = *self
                .owner_edge_abduction(id)
                .ok_or(OwnerTopologyError::UnknownRecord(id))?;
            let from_exists = record
                .originally_from
                .or(record.current_from)
                .is_some_and(|node| closure.contains(&node));
            let to_exists = record
                .originally_to
                .or(record.current_to)
                .is_some_and(|node| closure.contains(&node));
            if !from_exists || !to_exists {
                remaining.records.push(id);
                remaining.present = true;
                remaining.arena = slice.arena;
                continue;
            }
            let from = record
                .originally_from
                .map(|node| self.restored_owner_endpoint(node))
                .transpose()?;
            let to = record
                .originally_to
                .map(|node| self.restored_owner_endpoint(node))
                .transpose()?;
            eligible.push((id, record, from, to));
        }
        for (id, record, from, to) in eligible {
            let stored = self.shared_topology.records[id.0]
                .as_mut()
                .expect("eligible canonical abduction");
            stored.originally_from = from;
            stored.originally_to = to;
            for (original, current, is_to) in [
                (from, record.current_from, false),
                (to, record.current_to, true),
            ] {
                if let Some(original) = original {
                    if current != Some(original) {
                        self.reconnect_owner_edge_endpoint(
                            record.source_edge.expect(
                                "runtime error: invalid memory address or nil pointer dereference",
                            ),
                            original,
                            is_to,
                        );
                    }
                }
            }
        }
        Ok(remaining)
    }

    fn require_owner_slice_arena(
        &self,
        slice: &OwnerAbductionSlice,
    ) -> Result<(), OwnerTopologyError> {
        if !slice.records.is_empty() && slice.arena != self.topology_arena_id() {
            return Err(OwnerTopologyError::ForeignArena {
                expected: self.topology_arena_id(),
                actual: slice.arena,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_to_tree_map_distinguishes_nil_empty_absence_and_present_nil_values() {
        let (mut owner, [first, second, foreign], _) = score_view_fixture();
        let selected = owner
            .create_owner_graph_view(vec![first], Vec::new())
            .unwrap();
        let nil = owner.owner_graph_node_to_tree_map(selected).unwrap();
        assert!(nil.is_nil());
        assert_eq!(nil.len(), 0);
        assert_eq!(nil.map_address(), None);
        assert_eq!(
            owner.owner_graph_node_to_tree_entry(selected, None),
            Ok(None)
        );
        assert_eq!(
            owner.write_owner_node_to_tree_entry(&nil, Some(first), None),
            Err(OwnerTopologyError::NilNodeToTreeMapAssignment)
        );
        owner
            .delete_owner_node_to_tree_entry(&nil, Some(first))
            .unwrap();
        let map = owner.allocate_owner_node_to_tree_map(true);
        owner
            .set_owner_graph_node_to_tree_map(selected, map.clone())
            .unwrap();
        assert!(!map.is_nil());
        assert!(map.map_address().is_some());
        assert_eq!(
            owner.owner_graph_node_to_tree_entry(selected, Some(first)),
            Ok(None)
        );
        owner
            .write_owner_node_to_tree_entry(&map, Some(first), None)
            .unwrap();
        owner
            .write_owner_node_to_tree_entry(&map, None, None)
            .unwrap();
        // Membership and Node.Graph do not constrain legal source map keys.
        let tree = owner.new_owner_tree(Some(second)).unwrap();
        owner
            .write_owner_node_to_tree_entry(&map, Some(foreign), Some(tree))
            .unwrap();
        assert_eq!(
            owner.owner_graph_node_to_tree_entry(selected, Some(first)),
            Ok(Some(None))
        );
        assert_eq!(
            owner.owner_graph_node_to_tree_entry(selected, None),
            Ok(Some(None))
        );
        assert_eq!(
            owner.owner_graph_node_to_tree_entry(selected, Some(foreign)),
            Ok(Some(Some(tree)))
        );
        assert_eq!(
            owner.owner_graph_node_to_tree_entry(OwnerGraphId(0), Some(first)),
            Ok(None)
        );
        assert_eq!(map.len(), 3);
        let actual = owner.owner_tree(tree).unwrap().fields();
        assert_eq!(actual.node, Some(second));
        assert_eq!(actual.parent, None);
        assert!(actual.children.is_empty());
        assert_eq!(actual.sentinel_edge, None);
        assert_eq!(actual.orientation, 0); // Go geo.TopLeft.
        let actual_pointer = owner.owner_tree(tree).unwrap() as *const _;
        let nil_node_tree = owner.new_owner_tree(None).unwrap();
        assert_eq!(owner.owner_tree(nil_node_tree).unwrap().fields().node, None);
        assert_ne!(
            owner.owner_tree(nil_node_tree).unwrap() as *const _,
            actual_pointer
        );
    }

    #[test]
    fn selected_node_to_tree_copy_shares_writes_but_field_replacement_keeps_old_aliases() {
        let (mut owner, [first, second, _], _) = score_view_fixture();
        let selected = owner
            .create_owner_graph_view(vec![first], Vec::new())
            .unwrap();
        let other = owner
            .create_owner_graph_view(vec![second], Vec::new())
            .unwrap();
        let members_before = owner
            .owner_graph_member_header(selected)
            .unwrap()
            .visible_slots();
        let owners_before = owner
            .shared_topology
            .node_owners
            .iter()
            .map(OwnerNodeGraphField::get)
            .collect::<Vec<_>>();
        let original = owner.allocate_owner_node_to_tree_map(true);
        owner
            .set_owner_graph_node_to_tree_map(selected, original.clone())
            .unwrap();
        owner
            .copy_owner_graph_node_to_tree_from(other, selected)
            .unwrap();
        assert!(
            owner
                .owner_graph_node_to_tree_map(other)
                .unwrap()
                .same_map_allocation(&original)
        );
        owner
            .write_owner_node_to_tree_entry(&original, Some(first), None)
            .unwrap();
        assert_eq!(
            owner.owner_graph_node_to_tree_entry(other, Some(first)),
            Ok(Some(None))
        );
        owner
            .delete_owner_node_to_tree_entry(&original, Some(first))
            .unwrap();
        assert_eq!(
            owner.owner_graph_node_to_tree_entry(selected, Some(first)),
            Ok(None)
        );
        let replacement = owner.allocate_owner_node_to_tree_map(true);
        let tree = owner.new_owner_tree(Some(second)).unwrap();
        owner
            .write_owner_node_to_tree_entry(&replacement, Some(second), Some(tree))
            .unwrap();
        owner
            .set_owner_graph_node_to_tree_map(selected, replacement.clone())
            .unwrap();
        owner
            .write_owner_node_to_tree_entry(&original, Some(first), None)
            .unwrap();
        assert_eq!(
            owner.owner_graph_node_to_tree_entry(selected, Some(first)),
            Ok(None)
        );
        assert_eq!(
            owner.owner_graph_node_to_tree_entry(selected, Some(second)),
            Ok(Some(Some(tree)))
        );
        assert_eq!(
            owner.owner_graph_node_to_tree_entry(other, Some(first)),
            Ok(Some(None))
        );
        assert!(!replacement.same_map_allocation(&original));
        owner
            .copy_owner_graph_node_to_tree_from(selected, OwnerGraphId(0))
            .unwrap();
        assert!(
            owner
                .owner_graph_node_to_tree_map(selected)
                .unwrap()
                .is_nil()
        );
        assert_eq!(replacement.observed_entries(), [(Some(second), Some(tree))]);
        assert_eq!(
            owner
                .owner_graph_member_header(selected)
                .unwrap()
                .visible_slots(),
            members_before
        );
        assert_eq!(
            owner
                .shared_topology
                .node_owners
                .iter()
                .map(OwnerNodeGraphField::get)
                .collect::<Vec<_>>(),
            owners_before
        );
    }

    #[test]
    fn numerical_node_to_tree_clone_detaches_maps_and_records_with_checked_provenance() {
        let (mut owner, [first, second, _], _) = score_view_fixture();
        let first_view = owner
            .create_owner_graph_view(vec![first], Vec::new())
            .unwrap();
        let second_view = owner
            .create_owner_graph_view(vec![second], Vec::new())
            .unwrap();
        let empty_view = owner
            .create_owner_graph_view(Vec::new(), Vec::new())
            .unwrap();
        let empty_map = owner.allocate_owner_node_to_tree_map(true);
        owner
            .set_owner_graph_node_to_tree_map(empty_view, empty_map.clone())
            .unwrap();
        let map = owner.allocate_owner_node_to_tree_map(true);
        let tree = owner.new_owner_tree(Some(first)).unwrap();
        owner
            .write_owner_node_to_tree_entry(&map, Some(first), Some(tree))
            .unwrap();
        owner
            .set_owner_graph_node_to_tree_map(first_view, map.clone())
            .unwrap();
        owner
            .copy_owner_graph_node_to_tree_from(second_view, first_view)
            .unwrap();
        let mut fork = owner.clone();
        let first_fork = fork.owner_graph_node_to_tree_map(first_view).unwrap();
        let second_fork = fork.owner_graph_node_to_tree_map(second_view).unwrap();
        assert!(first_fork.same_map_allocation(&second_fork));
        assert!(!first_fork.same_map_allocation(&map));
        let empty_fork = fork.owner_graph_node_to_tree_map(empty_view).unwrap();
        assert!(!empty_fork.is_nil());
        assert_eq!(empty_fork.len(), 0);
        assert!(!empty_fork.same_map_allocation(&first_fork));
        assert!(!empty_fork.same_map_allocation(&empty_map));
        assert!(
            fork.owner_graph_node_to_tree_map(OwnerGraphId(0))
                .unwrap()
                .is_nil()
        );
        assert_eq!(
            fork.owner_graph_node_to_tree_entry(first_view, Some(first)),
            Ok(Some(Some(tree)))
        );
        assert_ne!(
            fork.owner_tree(tree).unwrap() as *const _,
            owner.owner_tree(tree).unwrap() as *const _
        );
        fork.write_owner_node_to_tree_entry(&first_fork, Some(second), None)
            .unwrap();
        assert_eq!(
            fork.owner_graph_node_to_tree_entry(second_view, Some(second)),
            Ok(Some(None))
        );
        assert_eq!(
            owner.owner_graph_node_to_tree_entry(first_view, Some(second)),
            Ok(None)
        );
        assert_eq!(
            fork.set_owner_graph_node_to_tree_map(first_view, map.clone()),
            Err(OwnerTopologyError::ForeignNodeToTreeScope)
        );
        assert_eq!(
            owner.set_owner_graph_node_to_tree_map(first_view, first_fork.clone()),
            Err(OwnerTopologyError::ForeignNodeToTreeScope)
        );
        // Forks preserve IDs for existing records, but independently created
        // records at the same registry index must never validate as each other.
        let parent_new_tree = owner.new_owner_tree(Some(second)).unwrap();
        let fork_new_tree = fork.new_owner_tree(Some(first)).unwrap();
        assert_eq!(parent_new_tree.index(), fork_new_tree.index());
        assert_ne!(parent_new_tree, fork_new_tree);
        assert_eq!(
            owner.write_owner_node_to_tree_entry(&map, Some(second), Some(fork_new_tree)),
            Err(OwnerTopologyError::UnknownTree(fork_new_tree))
        );
        assert_eq!(
            fork.write_owner_node_to_tree_entry(&first_fork, Some(first), Some(parent_new_tree)),
            Err(OwnerTopologyError::UnknownTree(parent_new_tree))
        );
        let (mut foreign_owner, _, _) = score_view_fixture();
        let foreign_tree = foreign_owner.new_owner_tree(None).unwrap();
        assert_eq!(
            owner.write_owner_node_to_tree_entry(&map, Some(first), Some(foreign_tree)),
            Err(OwnerTopologyError::ForeignArena {
                expected: owner.topology_arena_id(),
                actual: foreign_owner.topology_arena_id()
            })
        );
        let unknown_tree = OwnerTreeId::new(owner.topology_arena_id().unwrap(), tree.index() + 100);
        assert_eq!(
            owner.write_owner_node_to_tree_entry(&map, Some(first), Some(unknown_tree)),
            Err(OwnerTopologyError::UnknownTree(unknown_tree))
        );
        assert_eq!(
            owner.write_owner_node_to_tree_entry(&map, Some(NodeId(999)), None),
            Err(OwnerTopologyError::UnknownNode(NodeId(999)))
        );
        assert_eq!(map.observed_entries(), [(Some(first), Some(tree))]);
    }

    #[test]
    fn topology_restore_repairs_original_tree_map_and_keeps_retained_new_tree_allocations() {
        let (mut owner, [first, second, foreign], _) = score_view_fixture();
        let selected = owner
            .create_owner_graph_view(vec![first], Vec::new())
            .unwrap();
        let other = owner
            .create_owner_graph_view(vec![second], Vec::new())
            .unwrap();
        let empty_view = owner
            .create_owner_graph_view(Vec::new(), Vec::new())
            .unwrap();
        let empty_map = owner.allocate_owner_node_to_tree_map(true);
        owner
            .set_owner_graph_node_to_tree_map(empty_view, empty_map.clone())
            .unwrap();
        let original = owner.allocate_owner_node_to_tree_map(true);
        let old_tree = owner.new_owner_tree(Some(first)).unwrap();
        owner
            .write_owner_node_to_tree_entry(&original, Some(first), Some(old_tree))
            .unwrap();
        owner
            .write_owner_node_to_tree_entry(&original, None, None)
            .unwrap();
        owner
            .set_owner_graph_node_to_tree_map(selected, original.clone())
            .unwrap();
        owner
            .copy_owner_graph_node_to_tree_from(other, selected)
            .unwrap();
        let old_tree_pointer = owner.owner_tree(old_tree).unwrap() as *const _;
        let snapshot = owner.owner_topology_snapshot();
        let new_tree = owner.new_owner_tree(Some(foreign)).unwrap();
        let new_tree_pointer = owner.owner_tree(new_tree).unwrap() as *const _;
        let replacement = owner.allocate_owner_node_to_tree_map(true);
        owner
            .write_owner_node_to_tree_entry(&replacement, Some(foreign), Some(new_tree))
            .unwrap();
        owner
            .set_owner_graph_node_to_tree_map(selected, replacement.clone())
            .unwrap();
        owner
            .set_owner_graph_node_to_tree_map(OwnerGraphId(0), replacement.clone())
            .unwrap();
        owner
            .delete_owner_node_to_tree_entry(&original, Some(first))
            .unwrap();
        owner
            .write_owner_node_to_tree_entry(&original, Some(second), None)
            .unwrap();
        owner
            .write_owner_node_to_tree_entry(&empty_map, Some(first), None)
            .unwrap();
        owner.restore_owner_topology_snapshot(snapshot);
        assert!(
            owner
                .owner_graph_node_to_tree_map(selected)
                .unwrap()
                .same_map_allocation(&original)
        );
        assert!(
            owner
                .owner_graph_node_to_tree_map(other)
                .unwrap()
                .same_map_allocation(&original)
        );
        assert_eq!(
            original.observed_entries(),
            [(None, None), (Some(first), Some(old_tree))]
        );
        assert!(
            owner
                .owner_graph_node_to_tree_map(OwnerGraphId(0))
                .unwrap()
                .is_nil()
        );
        assert_eq!(
            replacement.observed_entries(),
            [(Some(foreign), Some(new_tree))]
        );
        let restored_empty = owner.owner_graph_node_to_tree_map(empty_view).unwrap();
        assert!(!restored_empty.is_nil());
        assert!(restored_empty.same_map_allocation(&empty_map));
        assert_eq!(empty_map.len(), 0);
        assert_eq!(
            owner.owner_tree(old_tree).unwrap() as *const _,
            old_tree_pointer
        );
        assert_eq!(
            owner.owner_tree(new_tree).unwrap() as *const _,
            new_tree_pointer
        );
        owner
            .set_owner_graph_node_to_tree_map(selected, replacement)
            .unwrap();
        assert_eq!(
            owner.owner_graph_node_to_tree_entry(selected, Some(foreign)),
            Ok(Some(Some(new_tree)))
        );
        let later_tree = owner.new_owner_tree(None).unwrap();
        assert!(later_tree.index() > new_tree.index());
    }

    #[test]
    fn topology_restore_repairs_original_near_map_aliases_and_preserves_replacement() {
        use super::super::node_near_map::NodeNearMap;
        let (mut owner, [first, second, foreign], _) = score_view_fixture();
        let original = NodeNearMap::from_raw_keys([Some(second), None]);
        owner.nodes[first.0 as usize].nears = original.retained();
        owner.nodes[second.0 as usize].nears = original.retained();
        owner.nodes[foreign.0 as usize].nears = NodeNearMap::nil();
        let snapshot = owner.owner_topology_snapshot();
        original.delete(Some(second));
        original.insert(Some(foreign)).unwrap();
        let replacement = NodeNearMap::from_raw_keys([Some(first)]);
        owner.nodes[first.0 as usize].nears = replacement.retained();
        owner.nodes[foreign.0 as usize].nears = NodeNearMap::allocated();
        owner.restore_owner_topology_snapshot(snapshot);
        assert!(
            owner.nodes[first.0 as usize]
                .nears
                .same_allocation(&original)
        );
        assert!(
            owner.nodes[second.0 as usize]
                .nears
                .same_allocation(&original)
        );
        assert_eq!(original.raw_keys(), [Some(second), None]);
        assert_eq!(replacement.raw_keys(), [Some(first)]);
        assert!(owner.nodes[foreign.0 as usize].nears.is_nil());

        // An allocated empty original must remain allocated and empty even
        // when a retained alias acquired a raw nil key after capture.
        let empty = NodeNearMap::allocated();
        owner.nodes[foreign.0 as usize].nears = empty.retained();
        let snapshot = owner.owner_topology_snapshot();
        empty.insert(None).unwrap();
        owner.nodes[foreign.0 as usize].nears = NodeNearMap::nil();
        owner.restore_owner_topology_snapshot(snapshot);
        assert!(
            owner.nodes[foreign.0 as usize]
                .nears
                .same_allocation(&empty)
        );
        assert!(!empty.is_nil());
        assert!(empty.is_empty());
    }

    #[test]
    fn optimizer_geometry_restore_leaves_tree_map_header_contents_and_new_records_live() {
        use super::super::limits::{Context, ContextError, OptimizationWorkGuard};
        use super::super::owner_score_session::OwnerOptimizerMutationSnapshot;
        struct BackgroundContext;
        impl Context for BackgroundContext {
            fn done(&self) -> Option<bool> {
                None
            }
            fn error(&self) -> Option<ContextError> {
                None
            }
        }
        let context = BackgroundContext;
        let (mut owner, [first, second, _], _) = score_view_fixture();
        let selected = owner
            .create_owner_graph_view(vec![first], Vec::new())
            .unwrap();
        let original = owner.allocate_owner_node_to_tree_map(true);
        owner
            .set_owner_graph_node_to_tree_map(selected, original.clone())
            .unwrap();
        let old_position = owner.nodes[first.0 as usize].position;
        let original_nears = super::super::node_near_map::NodeNearMap::allocated();
        owner.nodes[first.0 as usize].nears = original_nears.retained();
        let mut guard =
            OptimizationWorkGuard::new(Some(&context), "sizelessOptimizer.optimize", 1000).unwrap();
        let mut snapshot = OwnerOptimizerMutationSnapshot::default();
        snapshot
            .capture_into(&mut owner, selected, &mut guard)
            .unwrap();
        owner.nodes[first.0 as usize].position = Some(Point { x: 123.0, y: 456.0 });
        original_nears.insert(Some(second)).unwrap();
        let replacement_nears = super::super::node_near_map::NodeNearMap::from_raw_keys([None]);
        owner.nodes[first.0 as usize].nears = replacement_nears.retained();
        let tree = owner.new_owner_tree(Some(second)).unwrap();
        owner
            .write_owner_node_to_tree_entry(&original, Some(first), None)
            .unwrap();
        let replacement = owner.allocate_owner_node_to_tree_map(true);
        owner
            .write_owner_node_to_tree_entry(&replacement, Some(second), Some(tree))
            .unwrap();
        owner
            .set_owner_graph_node_to_tree_map(selected, replacement.clone())
            .unwrap();
        snapshot.restore(&mut owner).unwrap();
        assert_eq!(owner.nodes[first.0 as usize].position, old_position);
        assert!(
            owner.nodes[first.0 as usize]
                .nears
                .same_allocation(&replacement_nears)
        );
        assert_eq!(original_nears.raw_keys(), [Some(second)]);
        assert_eq!(replacement_nears.raw_keys(), [None]);
        assert!(
            owner
                .owner_graph_node_to_tree_map(selected)
                .unwrap()
                .same_map_allocation(&replacement)
        );
        assert_eq!(
            owner.owner_graph_node_to_tree_entry(selected, Some(second)),
            Ok(Some(Some(tree)))
        );
        assert_eq!(original.observed_entries(), [(Some(first), None)]);
        assert_eq!(owner.owner_tree(tree).unwrap().fields().node, Some(second));
        snapshot.release();
        assert!(
            owner
                .owner_graph_node_to_tree_map(selected)
                .unwrap()
                .same_map_allocation(&replacement)
        );
    }

    #[test]
    fn member_headers_preserve_nil_empty_capacity_and_independent_dense_mapping() {
        let (mut owner, [first, second, foreign], [edge, _]) = score_view_fixture();
        let selected = owner
            .create_owner_graph_view(vec![first, second], vec![edge])
            .unwrap();
        let initial = owner.owner_graph_member_header(selected).unwrap();
        assert_eq!((initial.len(), initial.capacity()), (2, 2));
        let nodes_before = owner.node_order.clone();
        let owners_before = owner
            .shared_topology
            .node_owners
            .iter()
            .map(OwnerNodeGraphField::get)
            .collect::<Vec<_>>();
        let dense_before = owner
            .owner_graph_view(selected)
            .unwrap()
            .local_to_canonical
            .clone();
        let nullable = owner
            .allocate_owner_member_header(&[Some(foreign), None, Some(first)], 7)
            .unwrap();
        owner
            .write_owner_member_capacity_slot(&nullable, 6, Some(second))
            .unwrap();
        owner
            .set_owner_graph_member_header(selected, nullable.clone())
            .unwrap();
        assert_eq!((nullable.len(), nullable.capacity()), (3, 7));
        assert_eq!(nullable.get(1), Some(None));
        assert_eq!(nullable.get(3), None);
        assert_eq!(nullable.read_capacity_slot(6), Some(Some(second)));
        assert_eq!(nullable.read_capacity_slot(7), None);
        assert_eq!(nullable.visible_slots(), [Some(foreign), None, Some(first)]);
        assert_eq!(
            owner.owner_graph_view(selected).unwrap().local_to_canonical,
            dense_before
        );
        assert_eq!(
            owner
                .shared_topology
                .node_owners
                .iter()
                .map(OwnerNodeGraphField::get)
                .collect::<Vec<_>>(),
            owners_before
        );
        assert_eq!(owner.node_order, nodes_before);

        let before = nullable.capacity_slots();
        assert_eq!(
            owner.write_owner_member_capacity_slot(&nullable, 4, Some(NodeId(999))),
            Err(OwnerTopologyError::UnknownNode(NodeId(999)))
        );
        assert_eq!(nullable.capacity_slots(), before);
        assert!(
            owner
                .allocate_owner_member_header(&[Some(first)], 0)
                .is_err()
        );
        let nil = owner.nil_owner_member_header();
        owner.set_owner_graph_member_header(selected, nil).unwrap();
        assert!(owner.owner_graph_member_header(selected).unwrap().is_nil());
        let empty = owner.allocate_owner_member_header(&[], 0).unwrap();
        owner
            .set_owner_graph_member_header(selected, empty.clone())
            .unwrap();
        assert!(!empty.is_nil());
        assert!(empty.is_empty());
        assert_eq!((empty.len(), empty.capacity()), (0, 0));
        let spare_empty = owner.allocate_owner_member_header(&[], 4).unwrap();
        owner
            .set_owner_graph_member_header(selected, spare_empty.clone())
            .unwrap();
        assert_eq!(spare_empty.capacity_slots(), [None; 4]);
        assert_eq!((spare_empty.len(), spare_empty.capacity()), (0, 4));
        assert!(!initial.is_nil());
        assert_eq!(initial.visible_slots(), [Some(first), Some(second)]);
    }

    #[test]
    fn retained_member_range_loads_live_future_slots_but_keeps_its_original_header() {
        let (mut owner, [first, second, foreign], _) = score_view_fixture();
        let selected = owner
            .create_owner_graph_view(vec![first], Vec::new())
            .unwrap();
        let original = owner
            .allocate_owner_member_header(&[Some(first); 65], 70)
            .unwrap();
        owner
            .set_owner_graph_member_header(selected, original.clone())
            .unwrap();
        let retained = owner.owner_graph_member_header(selected).unwrap();
        let mut range = retained.iter_slots();
        for _ in 0..63 {
            assert_eq!(range.next(), Some(Some(first)));
        }
        // The pointer at index63 has already been yielded before a Step poll.
        let loaded = range.next();
        owner
            .write_owner_member_capacity_slot(&original, 63, None)
            .unwrap();
        owner
            .write_owner_member_capacity_slot(&original, 64, Some(second))
            .unwrap();
        let replacement = owner
            .allocate_owner_member_header(&[Some(foreign); 65], 72)
            .unwrap();
        owner
            .set_owner_graph_member_header(selected, replacement.clone())
            .unwrap();
        assert_eq!(loaded, Some(Some(first)));
        assert_eq!(range.next(), Some(Some(second)));
        assert_eq!(range.next(), None);
        assert_eq!((retained.len(), retained.capacity()), (65, 70));
        assert_eq!(retained.get(63), Some(None));
        assert_eq!(retained.backing_address(), original.backing_address());
        assert_ne!(retained.backing_address(), replacement.backing_address());
        assert_eq!(
            owner.owner_graph_member_header(selected).unwrap().get(64),
            Some(Some(foreign))
        );
        assert_eq!(
            owner
                .owner_graph_member_header(selected)
                .unwrap()
                .capacity(),
            72
        );
    }

    #[test]
    fn source_member_append_retains_old_length_and_publishes_duplicates_before_owner_redirect() {
        let (mut owner, [first, second, foreign], _) = score_view_fixture();
        let selected = owner
            .create_owner_graph_view(Vec::new(), Vec::new())
            .unwrap();
        let old = owner
            .allocate_owner_member_header(&[Some(first)], 2)
            .unwrap();
        owner
            .set_owner_graph_member_header(selected, old.clone())
            .unwrap();
        let mut journal = OwnerGraphOwnershipJournal::default();
        owner
            .add_owner_graph_node(selected, first, &mut journal)
            .unwrap();
        let appended = owner.owner_graph_member_header(selected).unwrap();
        assert_eq!((old.len(), old.capacity()), (1, 2));
        assert_eq!(old.visible_slots(), [Some(first)]);
        assert_eq!(old.read_capacity_slot(1), Some(Some(first)));
        assert_eq!(appended.visible_slots(), [Some(first), Some(first)]);
        assert_eq!(appended.backing_address(), old.backing_address());
        assert_eq!(owner.topology_node_owner(first), Some(selected));
        assert_eq!(journal.original_owner(first), Some(Some(OwnerGraphId(0))));
        owner
            .add_owner_graph_node(selected, second, &mut journal)
            .unwrap();
        let grown = owner.owner_graph_member_header(selected).unwrap();
        assert_eq!((grown.len(), grown.capacity()), (3, 4));
        assert_ne!(grown.backing_address(), old.backing_address());
        assert_eq!(
            grown.visible_slots(),
            [Some(first), Some(first), Some(second)]
        );
        let with_nil = owner.append_owner_member_header(&grown, &[None]).unwrap();
        assert_eq!((with_nil.len(), with_nil.capacity()), (4, 4));
        assert_eq!(grown.read_capacity_slot(3), Some(None));
        assert_eq!(with_nil.get(3), Some(None));
        assert_eq!(owner.topology_node_owner(foreign), Some(OwnerGraphId(0)));
        let before = with_nil.capacity_slots();
        assert_eq!(
            owner
                .append_owner_member_header(&with_nil, &[Some(NodeId(999))])
                .unwrap_err(),
            OwnerTopologyError::UnknownNode(NodeId(999))
        );
        assert_eq!(with_nil.capacity_slots(), before);
    }

    #[test]
    fn numerical_owner_clone_detaches_member_backing_and_preserves_internal_header_aliases() {
        let (mut owner, [first, second, foreign], _) = score_view_fixture();
        let first_view = owner
            .create_owner_graph_view(vec![first], Vec::new())
            .unwrap();
        let second_view = owner
            .create_owner_graph_view(vec![second], Vec::new())
            .unwrap();
        let nil_view = owner
            .create_owner_graph_view(Vec::new(), Vec::new())
            .unwrap();
        let empty_view = owner
            .create_owner_graph_view(Vec::new(), Vec::new())
            .unwrap();
        let all = owner
            .allocate_owner_member_header(
                &[
                    Some(first),
                    Some(second),
                    None,
                    Some(foreign),
                    Some(first),
                    Some(second),
                ],
                6,
            )
            .unwrap();
        let first_window = all.reslice_with_capacity(1, 3, 5).unwrap();
        let second_window = all.reslice_with_capacity(2, 4, 6).unwrap();
        owner
            .set_owner_graph_member_header(first_view, first_window.clone())
            .unwrap();
        owner
            .set_owner_graph_member_header(second_view, second_window.clone())
            .unwrap();
        let nil = owner.nil_owner_member_header();
        owner.set_owner_graph_member_header(nil_view, nil).unwrap();
        let mut copied = owner.clone();
        assert_eq!(copied.topology_arena_id(), owner.topology_arena_id());
        let copied_first = copied.owner_graph_member_header(first_view).unwrap();
        let copied_second = copied.owner_graph_member_header(second_view).unwrap();
        assert_ne!(
            copied_first.backing_address(),
            first_window.backing_address()
        );
        assert_eq!(
            copied_first.backing_address(),
            copied_second.backing_address()
        );
        assert_eq!(copied_first.slot_address(1), copied_second.slot_address(0));
        assert_eq!(
            (
                copied_first.offset(),
                copied_first.len(),
                copied_first.capacity()
            ),
            (1, 2, 4)
        );
        assert_eq!(
            (
                copied_second.offset(),
                copied_second.len(),
                copied_second.capacity()
            ),
            (2, 2, 4)
        );
        assert_eq!(copied_first.capacity_slots(), first_window.capacity_slots());
        copied
            .write_owner_member_capacity_slot(&copied_first, 1, Some(first))
            .unwrap();
        copied
            .write_owner_member_capacity_slot(&copied_second, 3, None)
            .unwrap();
        assert_eq!(copied_second.get(0), Some(Some(first)));
        assert_eq!(first_window.get(1), Some(None));
        assert_eq!(all.read_capacity_slot(5), Some(Some(second)));
        assert_eq!(copied_second.read_capacity_slot(3), Some(None));
        assert!(copied.owner_graph_member_header(nil_view).unwrap().is_nil());
        let copied_empty = copied.owner_graph_member_header(empty_view).unwrap();
        assert!(!copied_empty.is_nil());
        assert_eq!((copied_empty.len(), copied_empty.capacity()), (0, 0));
        assert_eq!(
            copied.set_owner_graph_member_header(first_view, first_window.clone()),
            Err(OwnerTopologyError::ForeignMemberScope)
        );
        assert_eq!(
            copied.write_owner_member_capacity_slot(&first_window, 0, None),
            Err(OwnerTopologyError::ForeignMemberScope)
        );
        assert_eq!(
            copied
                .owner_graph_member_header(first_view)
                .unwrap()
                .backing_address(),
            copied_first.backing_address()
        );
        let (mut unrelated, _, _) = score_view_fixture();
        assert!(matches!(
            unrelated.set_owner_graph_member_header(OwnerGraphId(0), first_window),
            Err(OwnerTopologyError::ForeignArena { .. })
        ));
    }

    #[test]
    fn topology_restore_repairs_original_member_capacity_window_without_touching_replacement() {
        let (mut owner, [first, second, foreign], _) = score_view_fixture();
        let selected = owner
            .create_owner_graph_view(vec![first], Vec::new())
            .unwrap();
        let all = owner
            .allocate_owner_member_header(
                &[
                    Some(first),
                    Some(second),
                    None,
                    Some(foreign),
                    Some(first),
                    Some(second),
                ],
                6,
            )
            .unwrap();
        let window = all.reslice_with_capacity(1, 3, 4).unwrap();
        owner
            .set_owner_graph_member_header(selected, window.clone())
            .unwrap();
        let snapshot = owner.owner_topology_snapshot();
        let replacement = owner
            .allocate_owner_member_header(&[Some(foreign)], 2)
            .unwrap();
        owner
            .set_owner_graph_member_header(selected, replacement.clone())
            .unwrap();
        owner
            .write_owner_member_capacity_slot(&all, 0, Some(foreign))
            .unwrap();
        owner
            .write_owner_member_capacity_slot(&all, 2, Some(first))
            .unwrap();
        owner
            .write_owner_member_capacity_slot(&all, 3, None)
            .unwrap();
        owner
            .write_owner_member_capacity_slot(&all, 4, Some(second))
            .unwrap();
        owner
            .write_owner_member_capacity_slot(&replacement, 0, Some(second))
            .unwrap();
        owner
            .write_owner_member_capacity_slot(&replacement, 1, Some(first))
            .unwrap();
        owner.restore_owner_topology_snapshot(snapshot);
        let restored = owner.owner_graph_member_header(selected).unwrap();
        assert_eq!(restored.data_address(), window.data_address());
        assert_eq!(
            (restored.offset(), restored.len(), restored.capacity()),
            (1, 2, 3)
        );
        assert_eq!(window.capacity_slots(), [Some(second), None, Some(foreign)]);
        assert_eq!(
            all.visible_slots(),
            [
                Some(foreign),
                Some(second),
                None,
                Some(foreign),
                Some(second),
                Some(second)
            ]
        );
        assert_eq!(replacement.capacity_slots(), [Some(second), Some(first)]);
        assert_ne!(restored.backing_address(), replacement.backing_address());
        owner
            .set_owner_graph_member_header(selected, window)
            .unwrap();
    }

    #[test]
    fn optimizer_geometry_restore_does_not_restore_live_member_header_or_slot_edits() {
        use super::super::limits::{Context, ContextError, OptimizationWorkGuard};
        use super::super::owner_score_session::OwnerOptimizerMutationSnapshot;
        struct BackgroundContext;
        impl Context for BackgroundContext {
            fn done(&self) -> Option<bool> {
                None
            }
            fn error(&self) -> Option<ContextError> {
                None
            }
        }
        let context = BackgroundContext;
        let (mut owner, [first, second, foreign], _) = score_view_fixture();
        let selected = owner
            .create_owner_graph_view(vec![first], Vec::new())
            .unwrap();
        let old = owner
            .allocate_owner_member_header(&[Some(first)], 3)
            .unwrap();
        owner
            .set_owner_graph_member_header(selected, old.clone())
            .unwrap();
        let original_position = owner.nodes[first.0 as usize].position;
        let original_size = owner.nodes[first.0 as usize].rect.size;
        let mut guard =
            OptimizationWorkGuard::new(Some(&context), "sizelessOptimizer.optimize", 1000).unwrap();
        let mut snapshot = OwnerOptimizerMutationSnapshot::default();
        snapshot
            .capture_into(&mut owner, selected, &mut guard)
            .unwrap();
        owner.nodes[first.0 as usize].position = Some(Point { x: 123.0, y: 456.0 });
        owner.nodes[first.0 as usize].rect.size.width = 789.0;
        owner
            .write_owner_member_capacity_slot(&old, 0, None)
            .unwrap();
        owner
            .write_owner_member_capacity_slot(&old, 2, Some(second))
            .unwrap();
        let replacement = owner
            .allocate_owner_member_header(&[Some(foreign), None], 4)
            .unwrap();
        owner
            .set_owner_graph_member_header(selected, replacement.clone())
            .unwrap();
        snapshot.restore(&mut owner).unwrap();
        assert_eq!(owner.nodes[first.0 as usize].position, original_position);
        assert_eq!(owner.nodes[first.0 as usize].rect.size, original_size);
        assert_eq!(
            owner
                .owner_graph_member_header(selected)
                .unwrap()
                .data_address(),
            replacement.data_address()
        );
        assert_eq!(
            owner
                .owner_graph_member_header(selected)
                .unwrap()
                .visible_slots(),
            [Some(foreign), None]
        );
        assert_eq!(old.capacity_slots(), [None, None, Some(second)]);
        snapshot.release();
        assert_eq!(
            owner
                .owner_graph_member_header(selected)
                .unwrap()
                .data_address(),
            replacement.data_address()
        );
    }

    #[test]
    fn owner_edge_membership_validation_is_atomic_and_does_not_rewrite_incidence() {
        let (mut owner, [first, _, _], [edge, foreign_edge]) = score_view_fixture();
        let selected = owner
            .create_owner_graph_view(vec![first], vec![edge])
            .unwrap();
        let before = owner.nodes[first.0 as usize].edges.read();
        assert_eq!(
            owner.set_owner_view_edge_membership(selected, vec![foreign_edge, EdgeId(999)]),
            Err(OwnerTopologyError::UnknownEdge(EdgeId(999)))
        );
        assert_eq!(owner.owner_graph_view(selected).unwrap().edges, [edge]);
        assert_eq!(owner.nodes[first.0 as usize].edges, before);
        owner
            .set_owner_view_edge_membership(selected, Vec::new())
            .unwrap();
        assert!(owner.owner_graph_view(selected).unwrap().edges.is_empty());
        assert_eq!(owner.nodes[first.0 as usize].edges, before);
        assert_eq!(owner.owner_graph_turn_cost(selected).unwrap(), 0.0);
        assert_eq!(owner.owner_graph_max_edge_length(selected).unwrap(), 290.0);
    }

    fn score_view_fixture() -> (ArenaGraph, [NodeId; 3], [EdgeId; 2]) {
        let mut input = Graph::default();
        let first = add_node(&mut input, "first", None, 101);
        let second = add_node(&mut input, "second", None, 102);
        let foreign = add_node(&mut input, "foreign", None, 103);
        input.set_initial_position(first, Some(Point { x: 0.0, y: 0.0 }));
        input.set_initial_position(second, Some(Point { x: 100.0, y: 0.0 }));
        input.set_initial_position(foreign, Some(Point { x: 300.0, y: 0.0 }));
        let first_edge = input.add_edge(crate::Edge {
            source: first,
            target: second,
        });
        let foreign_edge = input.add_edge(crate::Edge {
            source: first,
            target: foreign,
        });
        let mut owner = Pipeline::new_untraced(&input, 0, true, true).graph;
        owner.initialize_shared_topology();
        (owner, [first, second, foreign], [first_edge, foreign_edge])
    }

    #[test]
    fn production_owner_direction_binding_omits_implicit_layout_hints() {
        let (mut owner, [first, second, foreign], [edge, _]) = score_view_fixture();
        let selected = owner
            .create_owner_graph_view(vec![first, second], vec![edge])
            .unwrap();
        owner.scoring_directions.clear();
        owner
            .scoring_directions
            .insert(Some(foreign), Direction::Left);
        let implicit = owner.directions.get(&None).copied();
        owner.bind_owner_placement_directions();
        assert_eq!(owner.owner_graph_direction_entry(selected, None), Ok(None));
        assert_eq!(
            owner.owner_graph_direction_entry(selected, Some(foreign)),
            Ok(Some(Orientation::Left))
        );
        assert_eq!(owner.directions.get(&None).copied(), implicit);
        owner
            .set_owner_graph_direction_entry(selected, None, Some(Orientation::None))
            .unwrap();
        assert_eq!(
            owner.owner_graph_direction_entry(OwnerGraphId(0), None),
            Ok(Some(Orientation::None))
        );
        owner
            .set_owner_graph_direction_entry(selected, None, Some(Orientation::Right))
            .unwrap();
        assert_eq!(
            owner.owner_graph_direction_entry(OwnerGraphId(0), None),
            Ok(Some(Orientation::Right))
        );
        owner
            .set_owner_graph_direction_entry(OwnerGraphId(0), None, None)
            .unwrap();
        assert_eq!(owner.owner_graph_direction_entry(selected, None), Ok(None));
    }

    #[test]
    fn owner_direction_map_preserves_present_none_and_live_shared_view_updates() {
        let (mut owner, [first, second, foreign], [edge, _]) = score_view_fixture();
        let selected = owner
            .create_owner_graph_view(vec![first, second], vec![edge])
            .unwrap();
        owner
            .set_owner_graph_direction_entry(selected, Some(foreign), Some(Orientation::None))
            .unwrap();
        for view in [OwnerGraphId(0), selected] {
            assert_eq!(
                owner.owner_graph_direction_entry(view, Some(foreign)),
                Ok(Some(Orientation::None))
            );
        }
        assert!(!owner.directions.contains_key(&Some(foreign)));
        owner
            .set_owner_graph_direction_entry(
                OwnerGraphId(0),
                Some(foreign),
                Some(Orientation::TopRight),
            )
            .unwrap();
        assert_eq!(
            owner.owner_graph_direction_entry(selected, Some(foreign)),
            Ok(Some(Orientation::TopRight))
        );
        owner
            .set_owner_graph_direction_entry(selected, Some(foreign), Some(Orientation::Left))
            .unwrap();
        assert_eq!(owner.directions.get(&Some(foreign)), Some(&Direction::Left));
        assert!(
            !owner
                .shared_topology
                .direction_extensions
                .contains_key(&Some(foreign))
        );
        // Existing cardinal map mutation must remain visible to every owner
        // view; no initialization-time direction snapshot supplies the read.
        owner.directions.insert(Some(foreign), Direction::Up);
        assert_eq!(
            owner.owner_graph_direction_entry(selected, Some(foreign)),
            Ok(Some(Orientation::Top))
        );
        owner
            .set_owner_graph_direction_entry(OwnerGraphId(0), Some(foreign), None)
            .unwrap();
        assert_eq!(
            owner.owner_graph_direction_entry(selected, Some(foreign)),
            Ok(None)
        );
    }

    #[test]
    fn owner_direction_map_checks_receiver_key_and_disjoint_representation() {
        let (mut owner, [first, _, _], _) = score_view_fixture();
        owner
            .set_owner_graph_direction_entry(OwnerGraphId(0), Some(first), Some(Orientation::None))
            .unwrap();
        let cardinal_before = owner.directions.clone();
        let extended_before = owner.shared_topology.direction_extensions.clone();
        assert_eq!(
            owner.set_owner_graph_direction_entry(
                OwnerGraphId(999),
                Some(first),
                Some(Orientation::Right),
            ),
            Err(OwnerTopologyError::UnknownGraph(OwnerGraphId(999)))
        );
        assert_eq!(
            owner.set_owner_graph_direction_entry(
                OwnerGraphId(0),
                Some(NodeId(999)),
                Some(Orientation::Right),
            ),
            Err(OwnerTopologyError::UnknownNode(NodeId(999)))
        );
        assert_eq!(owner.directions, cardinal_before);
        assert_eq!(owner.shared_topology.direction_extensions, extended_before);
        assert_eq!(
            owner.owner_graph_direction_entry(OwnerGraphId(999), Some(first)),
            Err(OwnerTopologyError::UnknownGraph(OwnerGraphId(999)))
        );
        assert_eq!(
            owner.owner_graph_direction_entry(OwnerGraphId(0), Some(NodeId(999))),
            Err(OwnerTopologyError::UnknownNode(NodeId(999)))
        );
        owner.directions.insert(Some(first), Direction::Right);
        assert_eq!(
            owner.owner_graph_direction_entry(OwnerGraphId(0), Some(first)),
            Err(OwnerTopologyError::DirectionEntryConflict(Some(first)))
        );
        // A checked assignment is an explicit map overwrite, so it resolves
        // an inconsistent direct legacy edit instead of choosing a winner.
        owner
            .set_owner_graph_direction_entry(
                OwnerGraphId(0),
                Some(first),
                Some(Orientation::Bottom),
            )
            .unwrap();
        assert_eq!(
            owner.owner_graph_direction_entry(OwnerGraphId(0), Some(first)),
            Ok(Some(Orientation::Bottom))
        );
        assert!(
            !owner
                .shared_topology
                .direction_extensions
                .contains_key(&Some(first))
        );
    }

    #[test]
    fn owner_topology_restore_leaves_cardinal_and_extended_direction_edits_live() {
        let (mut owner, [first, second, _], _) = score_view_fixture();
        owner
            .set_owner_graph_direction_entry(OwnerGraphId(0), Some(first), Some(Orientation::None))
            .unwrap();
        owner
            .set_owner_graph_direction_entry(
                OwnerGraphId(0),
                Some(second),
                Some(Orientation::Right),
            )
            .unwrap();
        let snapshot = owner.owner_topology_snapshot();
        owner
            .set_owner_graph_direction_entry(OwnerGraphId(0), Some(first), Some(Orientation::Top))
            .unwrap();
        owner
            .set_owner_graph_direction_entry(OwnerGraphId(0), Some(second), Some(Orientation::None))
            .unwrap();
        owner.restore_owner_topology_snapshot(snapshot);
        assert_eq!(
            owner.owner_graph_direction_entry(OwnerGraphId(0), Some(first)),
            Ok(Some(Orientation::Top))
        );
        assert_eq!(
            owner.owner_graph_direction_entry(OwnerGraphId(0), Some(second)),
            Ok(Some(Orientation::None))
        );
        assert!(
            !owner
                .shared_topology
                .direction_extensions
                .contains_key(&Some(first))
        );
        assert!(!owner.directions.contains_key(&Some(second)));
    }

    #[test]
    fn ordinary_rust_owner_clone_copies_both_direction_representations_independently() {
        let (mut owner, [first, second, _], _) = score_view_fixture();
        owner
            .set_owner_graph_direction_entry(OwnerGraphId(0), Some(first), Some(Orientation::None))
            .unwrap();
        owner
            .set_owner_graph_direction_entry(
                OwnerGraphId(0),
                Some(second),
                Some(Orientation::Right),
            )
            .unwrap();
        let mut candidate = owner.clone();
        candidate
            .set_owner_graph_direction_entry(OwnerGraphId(0), Some(first), None)
            .unwrap();
        candidate
            .set_owner_graph_direction_entry(
                OwnerGraphId(0),
                Some(second),
                Some(Orientation::BottomLeft),
            )
            .unwrap();
        assert_eq!(
            owner.owner_graph_direction_entry(OwnerGraphId(0), Some(first)),
            Ok(Some(Orientation::None))
        );
        assert_eq!(
            owner.owner_graph_direction_entry(OwnerGraphId(0), Some(second)),
            Ok(Some(Orientation::Right))
        );
        assert_eq!(
            candidate.owner_graph_direction_entry(OwnerGraphId(0), Some(first)),
            Ok(None)
        );
        assert_eq!(
            candidate.owner_graph_direction_entry(OwnerGraphId(0), Some(second)),
            Ok(Some(Orientation::BottomLeft))
        );
    }

    #[test]
    fn owner_graph_scalar_and_cache_receivers_are_live_and_independent() {
        let (mut owner, [first, second, _], [edge, _]) = score_view_fixture();
        owner.cell_size = 25.0;
        owner.restore_routing_costs((7.0, 9.0, 11.0));
        let selected = owner
            .create_owner_graph_view(vec![first, second], vec![edge])
            .unwrap();
        let sibling = owner
            .create_owner_graph_view(vec![first], vec![edge])
            .unwrap();
        assert_eq!(owner.owner_graph_cell_size(OwnerGraphId(0)).unwrap(), 25.0);
        assert_eq!(owner.owner_graph_cell_size(selected).unwrap(), 0.0);
        assert_eq!(
            owner.owner_graph_routing_costs(selected).unwrap(),
            (0.0, 0.0, 0.0)
        );
        owner.cell_size = 75.0;
        owner.restore_routing_costs((17.0, 19.0, 21.0));
        assert_eq!(owner.owner_graph_cell_size(OwnerGraphId(0)).unwrap(), 75.0);
        assert_eq!(
            owner.owner_graph_routing_costs(OwnerGraphId(0)).unwrap(),
            (17.0, 19.0, 21.0)
        );
        assert_eq!(owner.compute_owner_graph_cell_size(selected).unwrap(), 10.0);
        owner.set_owner_graph_cell_size(selected, 40.0).unwrap();
        owner
            .restore_owner_graph_routing_costs(selected, (2.0, 4.0, 6.0))
            .unwrap();
        owner.halve_owner_graph_turn_cost(selected).unwrap();
        assert_eq!(
            owner.owner_graph_routing_costs(selected).unwrap(),
            (2.0, 2.0, 6.0)
        );
        assert_eq!(owner.owner_graph_cell_size(sibling).unwrap(), 0.0);
        assert_eq!(
            owner.owner_graph_routing_costs(sibling).unwrap(),
            (0.0, 0.0, 0.0)
        );
        assert_eq!(owner.cell_size, 75.0);
    }

    #[test]
    fn owner_graph_cost_reads_actual_incidence_and_retains_first_reader_until_reset() {
        let (mut owner, [first, _, foreign], [edge, _]) = score_view_fixture();
        let selected = owner
            .create_owner_graph_view(vec![first], vec![edge])
            .unwrap();
        // The far edge and its adjacent node are absent from selected member
        // slices, but source maxEdgeLength follows the live first.Edges.
        assert_eq!(owner.owner_graph_max_edge_length(selected).unwrap(), 290.0);
        assert_eq!(owner.owner_graph_turn_cost(selected).unwrap(), 36.25);
        owner.nodes[foreign.0 as usize].position = Some(Point { x: 500.0, y: 0.0 });
        assert_eq!(owner.owner_graph_turn_cost(selected).unwrap(), 36.25);
        owner.reset_owner_graph_turn_cost(selected).unwrap();
        assert_eq!(owner.owner_graph_turn_cost(selected).unwrap(), 61.25);
        assert_eq!(
            owner.owner_graph_crossing_cost(selected).unwrap(),
            0.110592_f64 * 490.0
        );
    }

    #[test]
    fn owner_graph_cost_reads_bound_top_left_instead_of_inline_geometry() {
        let (mut owner, [first, _, foreign], [edge, _]) = score_view_fixture();
        let selected = owner
            .create_owner_graph_view(vec![first], vec![edge])
            .unwrap();
        let point = owner.allocate_owner_node_point(Some(Point { x: 500.0, y: 0.0 }));
        owner.bind_owner_node_point(foreign, point.clone()).unwrap();
        assert_eq!(owner.nodes[foreign.0 as usize].position, None);
        assert_eq!(owner.owner_graph_max_edge_length(selected).unwrap(), 490.0);
        owner
            .write_owner_point(&point, Point { x: 700.0, y: 0.0 })
            .unwrap();
        assert_eq!(owner.owner_graph_max_edge_length(selected).unwrap(), 690.0);
    }

    #[test]
    fn independent_owner_forks_detach_all_component_caches_but_trials_alias_them() {
        let (mut owner, [first, _, _], [edge, _]) = score_view_fixture();
        let selected = owner
            .create_owner_graph_view(vec![first], vec![edge])
            .unwrap();
        owner
            .restore_owner_graph_routing_costs(selected, (7.0, 9.0, 11.0))
            .unwrap();
        let mut candidate = owner.clone();
        candidate
            .restore_owner_graph_routing_costs(selected, (17.0, 19.0, 21.0))
            .unwrap();
        assert_eq!(
            owner.owner_graph_routing_costs(selected).unwrap(),
            (17.0, 19.0, 21.0)
        );
        candidate.detach_owner_graph_score_states();
        candidate
            .restore_owner_graph_routing_costs(selected, (27.0, 29.0, 31.0))
            .unwrap();
        assert_eq!(
            owner.owner_graph_routing_costs(selected).unwrap(),
            (17.0, 19.0, 21.0)
        );
        assert_eq!(
            candidate.owner_graph_routing_costs(selected).unwrap(),
            (27.0, 29.0, 31.0)
        );
        assert!(!Arc::ptr_eq(
            owner.owner_graph_cost_cache(selected).unwrap(),
            candidate.owner_graph_cost_cache(selected).unwrap()
        ));
    }

    #[test]
    fn owner_graph_cost_errors_are_atomic_and_cached_values_do_not_read_geometry() {
        let (mut owner, [first, _, _], [edge, _]) = score_view_fixture();
        let selected = owner
            .create_owner_graph_view(vec![first], vec![edge])
            .unwrap();
        owner.nodes[first.0 as usize].cluster = Some(99);
        owner
            .restore_owner_graph_routing_costs(selected, (7.0, 9.0, 11.0))
            .unwrap();
        assert_eq!(owner.owner_graph_turn_cost(selected).unwrap(), 9.0);
        owner.reset_owner_graph_turn_cost(selected).unwrap();
        assert_eq!(
            owner.owner_graph_turn_cost(selected),
            Err(OwnerTopologyError::UnboundCostGeometry(first))
        );
        assert_eq!(
            owner.owner_graph_routing_costs(selected).unwrap(),
            (7.0, 0.0, 11.0)
        );
        let unknown = OwnerGraphId(999);
        assert!(owner.set_owner_graph_cell_size(unknown, 200.0).is_err());
        assert!(
            owner
                .restore_owner_graph_routing_costs(unknown, (20.0, 30.0, 40.0))
                .is_err()
        );
        assert_eq!(
            owner.owner_graph_routing_costs(selected).unwrap(),
            (7.0, 0.0, 11.0)
        );
    }

    #[test]
    fn empty_owner_graph_cell_and_noncenter_cost_match_source_sentinels() {
        let (mut owner, _, _) = score_view_fixture();
        let empty = owner
            .create_owner_graph_view(Vec::new(), Vec::new())
            .unwrap();
        assert_eq!(owner.compute_owner_graph_cell_size(empty).unwrap(), 10.0);
        owner
            .restore_owner_graph_routing_costs(empty, (-0.0, -0.0, -0.0))
            .unwrap();
        assert_eq!(
            owner
                .owner_graph_non_center_port_cost(empty)
                .unwrap()
                .to_bits(),
            0.0_f64.to_bits()
        );
        assert_eq!(
            owner.owner_graph_routing_costs(empty).unwrap().2.to_bits(),
            (-0.0_f64).to_bits()
        );
        assert_eq!(
            owner.owner_graph_turn_cost(empty).unwrap().to_bits(),
            0.0_f64.to_bits()
        );
        assert_eq!(
            owner.owner_graph_routing_costs(empty).unwrap().1.to_bits(),
            0.0_f64.to_bits()
        );
    }

    fn add_node(graph: &mut Graph, name: &str, parent: Option<NodeId>, tala_id: i64) -> NodeId {
        let mut node = crate::Node::new(
            name,
            Size {
                width: 10.0,
                height: 10.0,
            },
        );
        node.parent = parent;
        let id = graph.add_node(node);
        graph.tala_id_overrides[id.0 as usize] = Some(tala_id);
        graph.set_initial_position(
            id,
            Some(Point {
                x: f64::from(id.0) * 20.0,
                y: 20.0,
            }),
        );
        id
    }

    /// Real production scope construction supplies the nullable records.
    /// The test authors nodes/edges, never projected scorer records.
    fn live_scope() -> (Pipeline, PlacementScope, [NodeId; 6], [EdgeId; 2]) {
        live_scope_with_direction(false)
    }

    fn live_scope_with_direction(
        reverse: bool,
    ) -> (Pipeline, PlacementScope, [NodeId; 6], [EdgeId; 2]) {
        let mut graph = Graph::default();
        let c = add_node(&mut graph, "C", None, 1);
        let child_c = add_node(&mut graph, "c", Some(c), 2);
        let d = add_node(&mut graph, "D", None, 3);
        let child_d = add_node(&mut graph, "d", Some(d), 4);
        let x = add_node(&mut graph, "X", None, 5);
        let y = add_node(&mut graph, "Y", None, 6);
        let first = graph.add_edge(if reverse {
            crate::Edge {
                source: x,
                target: child_d,
            }
        } else {
            crate::Edge {
                source: child_d,
                target: x,
            }
        });
        let second = graph.add_edge(if reverse {
            crate::Edge {
                source: y,
                target: child_d,
            }
        } else {
            crate::Edge {
                source: child_d,
                target: y,
            }
        });
        let mut pipeline = Pipeline::new_untraced(&graph, 0, true, true);
        // Direct NewGraph/Connect/AbductEdges fixture, before tree extraction.
        pipeline.graph.preprocessed_tree_children.clear();
        pipeline.graph.tree_routing_nodes.clear();
        pipeline.graph.tree_sentinels.clear();
        pipeline.graph.restored_tree_edges.clear();
        pipeline.graph.published_tree_node_edge_order.clear();
        pipeline.graph.edge_order.replace_values(&[first, second]);
        pipeline.graph.node_order = vec![c, child_c, d, child_d, x, y];
        pipeline.graph.nodes[child_d.0 as usize].edges = vec![first, second].into();
        pipeline.graph.nodes[d.0 as usize].edges.clear();
        pipeline.graph.nodes[x.0 as usize].edges = vec![first].into();
        pipeline.graph.nodes[y.0 as usize].edges = vec![second].into();
        let placement = pipeline.placement_scope_graph(None).unwrap();
        assert_eq!(placement.sized_edge_abductions.len(), 2);
        (
            pipeline,
            placement,
            [c, child_c, d, child_d, x, y],
            [first, second],
        )
    }

    #[test]
    fn production_records_mutate_owner_restore_foreign_subset_and_reapply_whole_slice() {
        let (mut pipeline, placement, [c, _, d, child, x, y], [first, second]) = live_scope();
        let owner = &mut pipeline.graph;
        let member_order = owner.node_order.clone();
        let slice = owner
            .from_sized_owner_abductions(&placement.sized_edge_abductions, true)
            .unwrap();
        assert_eq!(owner.edge_records[first.0 as usize].from, child);
        assert_eq!(owner.nodes[child.0 as usize].edges, [first, second]);
        owner.abduct_owner_edge_abductions(&slice);
        assert_eq!(owner.edge_records[first.0 as usize].from, d);
        assert_eq!(owner.edge_records[second.0 as usize].from, d);
        assert!(owner.nodes[child.0 as usize].edges.is_empty());
        assert_eq!(owner.nodes[d.0 as usize].edges, [first, second]);
        assert_eq!(owner.nodes[x.0 as usize].edges, [first]);
        assert_eq!(owner.nodes[y.0 as usize].edges, [second]);
        assert_eq!(
            owner.restore_owner_edge_abductions(&slice, &[c]).unwrap(),
            slice
        );
        let remaining = owner
            .restore_owner_edge_abductions(&slice, &[d, x])
            .unwrap();
        assert_eq!(remaining.records, [slice.records[1]]);
        assert!(remaining.present);
        assert_eq!(owner.edge_records[first.0 as usize].from, child);
        assert_eq!(owner.edge_records[second.0 as usize].from, d);
        assert_eq!(owner.nodes[child.0 as usize].edges, [first]);
        assert_eq!(owner.nodes[d.0 as usize].edges, [second]);
        // Ordinary placement reuses the original whole slice, including the
        // still-active second record. Source Apply reorders that side too.
        owner.apply_owner_edge_abductions(&slice);
        assert_eq!(owner.nodes[d.0 as usize].edges, [first, second]);
        let remaining = owner
            .restore_owner_edge_abductions(&slice, &member_order)
            .unwrap();
        assert_eq!(remaining, OwnerAbductionSlice::default());
        assert!(owner.nodes[d.0 as usize].edges.is_empty());
        assert_eq!(owner.nodes[child.0 as usize].edges, [first, second]);
        assert_eq!(owner.node_order, member_order);
        assert_eq!(
            owner
                .owner_edge_abduction(slice.records[0])
                .unwrap()
                .originally_to,
            None
        );
    }

    #[test]
    fn reconnect_removes_only_first_incidence_and_leaves_opposite_side_untouched() {
        let (mut pipeline, _, [_, _, d, child, x, _], [first, second]) = live_scope();
        let owner = &mut pipeline.graph;
        // Node.addEdge permits duplicate pointer entries. An Edge.reconnect
        // must remove only the first entry, not normalize the entire slice.
        owner.nodes[child.0 as usize].edges = vec![first, second, first].into();
        owner.nodes[d.0 as usize].edges = vec![second].into();
        owner.reconnect_owner_edge_endpoint(first, d, false);
        assert_eq!(owner.nodes[child.0 as usize].edges, [second, first]);
        assert_eq!(owner.nodes[d.0 as usize].edges, [second, first]);
        assert_eq!(owner.nodes[x.0 as usize].edges, [first]);
        owner.nodes[d.0 as usize].edges = vec![first, second, first].into();
        owner.reconnect_owner_edge_endpoint(first, d, false);
        assert_eq!(owner.nodes[d.0 as usize].edges, [second, first, first]);
        assert_eq!(owner.edge_records[first.0 as usize].from, d);
        assert_eq!(owner.edge_records[first.0 as usize].to, x);
    }

    #[test]
    fn reversed_production_records_reconnect_only_to_side_and_keep_nil_from_original() {
        let (mut pipeline, placement, [_, _, d, child, x, y], [first, second]) =
            live_scope_with_direction(true);
        let owner = &mut pipeline.graph;
        let slice = owner
            .from_sized_owner_abductions(&placement.sized_edge_abductions, true)
            .unwrap();
        let record = owner.owner_edge_abduction(slice.records[0]).unwrap();
        assert_eq!(
            (record.originally_from, record.originally_to),
            (None, Some(child))
        );
        assert_eq!((record.current_from, record.current_to), (Some(x), Some(d)));
        owner.abduct_owner_edge_abductions(&slice);
        assert_eq!(
            (
                owner.edge_records[first.0 as usize].from,
                owner.edge_records[first.0 as usize].to
            ),
            (x, d)
        );
        assert_eq!(owner.nodes[x.0 as usize].edges, [first]);
        assert_eq!(owner.nodes[y.0 as usize].edges, [second]);
        let remaining = owner
            .restore_owner_edge_abductions(&slice, &[d, x])
            .unwrap();
        assert_eq!(remaining.records, [slice.records[1]]);
        assert_eq!(
            (
                owner.edge_records[first.0 as usize].from,
                owner.edge_records[first.0 as usize].to
            ),
            (x, child)
        );
        owner.apply_owner_edge_abductions(&slice);
        assert_eq!(owner.nodes[d.0 as usize].edges, [first, second]);
        assert_eq!(
            owner
                .owner_edge_abduction(slice.records[0])
                .unwrap()
                .originally_from,
            None
        );
    }

    #[test]
    fn detached_records_restore_both_sides_but_reapply_does_not_detach() {
        let mut graph = Graph::default();
        let d = add_node(&mut graph, "D", None, 1);
        let child = add_node(&mut graph, "d", Some(d), 2);
        let edge = graph.add_edge(crate::Edge {
            source: d,
            target: child,
        });
        let mut pipeline = Pipeline::new_untraced(&graph, 0, true, true);
        pipeline.graph.tree_routing_nodes.clear();
        pipeline.graph.tree_sentinels.clear();
        pipeline.graph.preprocessed_tree_children.clear();
        pipeline.graph.restored_tree_edges.clear();
        pipeline.graph.nodes[d.0 as usize].edges = vec![edge].into();
        pipeline.graph.nodes[child.0 as usize].edges = vec![edge].into();
        let placement = pipeline.placement_scope_graph(None).unwrap();
        let owner = &mut pipeline.graph;
        let slice = owner
            .from_sized_owner_abductions(&placement.sized_edge_abductions, true)
            .unwrap();
        assert_eq!(slice.records.len(), 1);
        let record = owner.owner_edge_abduction(slice.records[0]).unwrap();
        assert_eq!((record.current_from, record.current_to), (None, None));
        owner.abduct_owner_edge_abductions(&slice);
        assert!(owner.nodes[d.0 as usize].edges.is_empty());
        assert!(owner.nodes[child.0 as usize].edges.is_empty());
        assert_eq!(
            (
                owner.edge_records[edge.0 as usize].from,
                owner.edge_records[edge.0 as usize].to
            ),
            (d, child)
        );
        assert_eq!(
            owner.restore_owner_edge_abductions(&slice, &[d]).unwrap(),
            OwnerAbductionSlice::default()
        );
        assert_eq!(owner.nodes[d.0 as usize].edges, [edge]);
        assert_eq!(owner.nodes[child.0 as usize].edges, [edge]);
        owner.apply_owner_edge_abductions(&slice);
        assert_eq!(owner.nodes[d.0 as usize].edges, [edge]);
        assert_eq!(owner.nodes[child.0 as usize].edges, [edge]);
    }

    #[test]
    fn member_views_and_exact_owner_journal_preserve_hidden_descendants() {
        let (mut pipeline, _, [c, child_c, d, child_d, x, _], [first, _]) = live_scope();
        let owner = &mut pipeline.graph;
        owner.initialize_shared_topology();
        let order = owner.node_order.clone();
        let temp = owner
            .create_owner_graph_view(Vec::new(), Vec::new())
            .unwrap();
        let mut before_split = OwnerGraphOwnershipJournal::default();
        owner
            .add_owner_graph_node(temp, c, &mut before_split)
            .unwrap();
        owner
            .add_owner_graph_node(temp, d, &mut before_split)
            .unwrap();
        assert_eq!(owner.topology_node_owner(d), Some(temp));
        assert_eq!(owner.topology_node_owner(child_d), Some(OwnerGraphId(0)));
        let component = owner
            .create_owner_graph_view(vec![d, x], vec![first])
            .unwrap();
        let mut split = OwnerGraphOwnershipJournal::default();
        owner
            .set_owner_graph_reference(component, &[d, x], true, &mut split)
            .unwrap();
        assert_eq!(
            owner
                .owner_graph_view(component)
                .unwrap()
                .nodes
                .visible_slots(),
            [Some(d), Some(x)]
        );
        assert_eq!(
            owner.owner_view_canonical_node(component, NodeId(0)),
            Some(d)
        );
        assert_eq!(owner.owner_view_local_node(component, child_d), None);
        owner
            .set_owner_view_node_mapping(component, vec![x, d, child_d])
            .unwrap();
        assert_eq!(
            owner.owner_view_canonical_node(component, NodeId(0)),
            Some(x)
        );
        assert_eq!(
            owner.owner_view_local_node(component, child_d),
            Some(NodeId(2))
        );
        assert_eq!(
            owner
                .owner_graph_view(component)
                .unwrap()
                .nodes
                .visible_slots(),
            [Some(d), Some(x)]
        );
        assert_eq!(
            owner.topology_retained_closure(&[d, x]).unwrap(),
            [d, x, child_d]
        );
        assert_eq!(owner.topology_node_owner(child_d), Some(component));
        assert_eq!(split.original_owner(d), Some(Some(temp)));
        assert_eq!(split.original_owner(child_d), Some(Some(OwnerGraphId(0))));
        // Repeated assignment cannot overwrite the exact first owner.
        owner
            .set_owner_graph_reference(component, &[d], true, &mut split)
            .unwrap();
        owner.restore_owner_graph_ownership(&split);
        assert_eq!(owner.topology_node_owner(d), Some(temp));
        assert_eq!(owner.topology_node_owner(child_d), Some(OwnerGraphId(0)));
        assert_eq!(owner.topology_node_owner(x), Some(OwnerGraphId(0)));
        assert_eq!(owner.topology_node_owner(child_c), Some(OwnerGraphId(0)));
        assert_eq!(
            owner
                .owner_graph_view(component)
                .unwrap()
                .nodes
                .visible_slots(),
            [Some(d), Some(x)]
        );
        assert_eq!(owner.node_order, order);
    }

    #[test]
    fn container_absent_nil_and_empty_remain_distinct() {
        let (mut pipeline, _, [c, _, d, _, _, _], _) = live_scope();
        let owner = &mut pipeline.graph;
        assert_eq!(owner.topology_container_slice(Some(NodeId(999))), None);
        owner.set_topology_container_slice(Some(c), None);
        assert_eq!(owner.topology_container_slice(Some(c)), Some(None));
        owner.set_topology_container_slice(Some(d), Some(Vec::new()));
        assert_eq!(
            owner.topology_container_slice(Some(d)),
            Some(Some(Vec::new()))
        );
        assert_eq!(owner.topology_retained_closure(&[c, d]).unwrap(), [c, d]);
        let map = owner.owner_container_map(OwnerGraphId(0)).unwrap();
        assert!(map.get(Some(NodeId(999))).is_none());
        assert!(map.get(Some(c)).unwrap().slice.is_nil());
        let empty = map.get(Some(d)).unwrap().slice;
        assert!(!empty.is_nil());
        assert_eq!((empty.len(), empty.capacity()), (0, 0));
    }

    #[test]
    fn live_descendants_follow_retained_container_header_writes() {
        let (mut pipeline, _, [c, cc, _, _, x, _], _) = live_scope();
        let owner = &mut pipeline.graph;
        owner.initialize_shared_topology();
        let retained = owner
            .owner_container_map(OwnerGraphId(0))
            .unwrap()
            .get(Some(c))
            .unwrap();
        assert_eq!(owner.descendants(c), [cc]);
        retained.slice.write_capacity_slot(0, Some(x)).unwrap();
        // No writer-local invalidation: the retained actual backing is the
        // sole authority, including for owner closure traversal.
        assert_eq!(owner.descendants(c), [x]);
        assert_eq!(owner.topology_retained_closure(&[c]).unwrap(), [c, x]);
        owner.containers.insert(Some(c), vec![cc]);
        assert_eq!(owner.descendants(c), [cc]);
        assert_eq!(retained.slice.visible_slots(), [Some(x)]);
    }

    #[test]
    fn owner_views_retain_real_map_headers_across_append_and_replacement() {
        // Original-Go API controls: spare cap6, full cap3, and offset [1:3:3].
        // These are explicit header states, not adapter-capacity claims.
        for mode in 0..3 {
            let (mut pipeline, _, [c, cc, d, _, x, y], _) = live_scope();
            let owner = &mut pipeline.graph;
            let backing = owner
                .allocate_owner_container_header(&[c, cc, d], 6)
                .unwrap();
            let initial = match mode {
                0 => backing.reslice_with_capacity(0, 3, 6).unwrap(),
                1 => backing.reslice_with_capacity(0, 3, 3).unwrap(),
                _ => backing.reslice_with_capacity(1, 3, 3).unwrap(),
            };
            owner.set_owner_container_header(None, initial).unwrap();
            let view_id = owner
                .create_owner_graph_view(Vec::new(), Vec::new())
                .unwrap();
            let original = owner.owner_container_map(OwnerGraphId(0)).unwrap();
            let view = owner.owner_container_map(view_id).unwrap();
            assert!(original.same_map_allocation(&view));
            assert_eq!(original.slices.map_address(), view.slices.map_address());
            let captured = view.get(None).unwrap();
            let captured_len = captured.slice.len();
            let captured_cap = captured.slice.capacity();
            let added = owner.retain_topology_node(owner.nodes[x.0 as usize].clone());
            let members = owner.node_order.clone();
            assert_eq!(owner.topology_node_owner(added), None);
            let appended = owner.append_owner_container_node(None, added).unwrap();
            assert_eq!(owner.topology_node_owner(added), None);
            assert_eq!(owner.node_order, members);
            assert!(owner.owner_graph_view(view_id).unwrap().nodes.is_empty());
            assert_eq!(
                (captured.slice.len(), captured.slice.capacity()),
                (captured_len, captured_cap)
            );
            assert_eq!(view.get(None).unwrap().slice.len(), captured_len + 1);
            assert_eq!(
                appended.slice.same_backing_allocation(&captured.slice),
                mode == 0
            );
            assert_eq!(
                appended.slice.data_address() == captured.slice.data_address(),
                mode == 0
            );
            assert_eq!(
                appended.slice.capacity(),
                if mode == 0 {
                    6
                } else if mode == 1 {
                    6
                } else {
                    4
                }
            );
            if mode == 0 {
                assert_eq!(captured.slice.read_capacity_slot(3), Some(Some(added)));
            } else {
                assert_eq!(
                    captured.slice.visible_slots(),
                    if mode == 1 {
                        vec![Some(c), Some(cc), Some(d)]
                    } else {
                        vec![Some(cc), Some(d)]
                    }
                );
            }
            let replacement = owner.allocate_owner_container_header(&[y, c], 2).unwrap();
            owner
                .set_owner_container_header(None, replacement.clone())
                .unwrap();
            assert!(
                view.get(None)
                    .unwrap()
                    .slice
                    .same_backing_allocation(&replacement.slice)
            );
            assert!(!captured.slice.same_backing_allocation(&replacement.slice));
            captured.slice.write_capacity_slot(0, Some(x)).unwrap();
            assert_eq!(captured.slice.get(0), Some(Some(x)));
            assert_eq!(
                view.get(None).unwrap().slice.visible_slots(),
                [Some(y), Some(c)]
            );
        }
    }

    #[test]
    fn owner_topology_rollback_repairs_real_header_window_and_retains_replacement() {
        let (mut pipeline, _, [c, cc, d, dd, x, y], _) = live_scope();
        let owner = &mut pipeline.graph;
        let all = owner
            .allocate_owner_container_header(&[c, cc, d, dd, x, y], 6)
            .unwrap();
        let window = all.reslice_with_capacity(1, 3, 4).unwrap();
        owner
            .set_owner_container_header(None, window.clone())
            .unwrap();
        let view_id = owner.create_owner_graph_view(vec![x], Vec::new()).unwrap();
        let view = owner.owner_container_map(view_id).unwrap();
        let captured = view.get(None).unwrap();
        let snapshot = owner.owner_topology_snapshot();
        let retained_replacement = owner.allocate_owner_container_header(&[y], 2).unwrap();
        owner
            .set_owner_container_header(None, retained_replacement.clone())
            .unwrap();
        all.slice.write_capacity_slot(0, Some(y)).unwrap();
        all.slice.write_capacity_slot(2, Some(x)).unwrap();
        all.slice.write_capacity_slot(4, Some(c)).unwrap();
        retained_replacement
            .slice
            .write_capacity_slot(0, Some(x))
            .unwrap();
        retained_replacement
            .slice
            .write_capacity_slot(1, Some(d))
            .unwrap();
        owner.restore_owner_topology_snapshot(snapshot);
        let restored = owner.owner_container_map(OwnerGraphId(0)).unwrap();
        assert!(restored.same_map_allocation(&view));
        assert_eq!(
            restored.get(None).unwrap().slice.data_address(),
            captured.slice.data_address()
        );
        assert_eq!(
            (
                restored.get(None).unwrap().slice.offset(),
                restored.get(None).unwrap().slice.len(),
                restored.get(None).unwrap().slice.capacity()
            ),
            (1, 2, 3)
        );
        assert_eq!(
            captured.slice.capacity_slots(),
            [Some(cc), Some(d), Some(dd)]
        );
        assert_eq!(
            all.slice.visible_slots(),
            [Some(y), Some(cc), Some(d), Some(dd), Some(c), Some(y)]
        );
        assert_eq!(
            retained_replacement.slice.capacity_slots(),
            [Some(x), Some(d)]
        );
        assert!(
            !restored
                .get(None)
                .unwrap()
                .slice
                .same_backing_allocation(&retained_replacement.slice)
        );
        assert_eq!(owner.containers.nodes(&None), [cc, d]);
        assert_eq!(
            owner
                .owner_graph_view(view_id)
                .unwrap()
                .nodes
                .visible_slots(),
            [Some(x)]
        );
    }

    #[test]
    fn independent_arena_candidates_detach_map_and_preserve_overlapping_headers() {
        let (mut pipeline, _, [c, cc, d, dd, x, y], _) = live_scope();
        let owner = &mut pipeline.graph;
        let all = owner
            .allocate_owner_container_header(&[c, cc, d, dd, x, y], 6)
            .unwrap();
        let first = all.reslice_with_capacity(1, 3, 5).unwrap();
        let second = all.reslice_with_capacity(2, 4, 6).unwrap();
        owner
            .set_owner_container_header(None, first.clone())
            .unwrap();
        owner
            .set_owner_container_header(Some(c), second.clone())
            .unwrap();
        let view_id = owner.create_owner_graph_view(vec![c], Vec::new()).unwrap();
        let mut candidate = owner.clone();
        let source_map = owner.owner_container_map(OwnerGraphId(0)).unwrap();
        let copy_map = candidate.owner_container_map(OwnerGraphId(0)).unwrap();
        let copy_view = candidate.owner_container_map(view_id).unwrap();
        assert!(!source_map.same_map_allocation(&copy_map));
        assert!(copy_map.same_map_allocation(&copy_view));
        let copied_first = copy_map.get(None).unwrap().slice;
        let copied_second = copy_map.get(Some(c)).unwrap().slice;
        assert!(matches!(
            candidate.set_owner_container_header(None, first.clone()),
            Err(OwnerTopologyError::ForeignContainerMap)
        ));
        assert_eq!(
            copy_map.get(None).unwrap().slice.capacity_slots(),
            copied_first.capacity_slots()
        );
        let candidate_header = copy_map.get(None).unwrap();
        candidate
            .set_owner_container_header(None, candidate_header)
            .unwrap();
        assert!(!copied_first.same_backing_allocation(&first.slice));
        assert!(copied_first.same_backing_allocation(&copied_second));
        assert_eq!(
            (
                copied_first.offset(),
                copied_first.len(),
                copied_first.capacity()
            ),
            (1, 2, 4)
        );
        assert_eq!(copied_first.capacity_slots(), first.slice.capacity_slots());
        copied_first.write_capacity_slot(1, Some(y)).unwrap();
        assert_eq!(copied_second.get(0), Some(Some(y)));
        assert_eq!(first.slice.get(1), Some(Some(d)));
        first.slice.write_capacity_slot(1, Some(x)).unwrap();
        assert_eq!(copied_first.get(1), Some(Some(y)));
        assert_eq!(source_map.get(None).unwrap().slice.get(1), Some(Some(x)));
    }

    #[test]
    fn retained_container_headers_cannot_cross_owner_arena_namespaces() {
        let (mut first, _, [c, _, _, _, _, _], _) = live_scope();
        let (mut second, _, _, _) = live_scope();
        let foreign = first
            .graph
            .allocate_owner_container_header(&[c], 4)
            .unwrap();
        second.graph.initialize_shared_topology();
        let before = second
            .graph
            .owner_container_map(OwnerGraphId(0))
            .unwrap()
            .get(None)
            .unwrap()
            .slice
            .capacity_slots();
        assert!(matches!(
            second.graph.set_owner_container_header(None, foreign),
            Err(OwnerTopologyError::ForeignArena { .. })
        ));
        assert_eq!(
            second
                .graph
                .owner_container_map(OwnerGraphId(0))
                .unwrap()
                .get(None)
                .unwrap()
                .slice
                .capacity_slots(),
            before
        );
    }

    #[test]
    fn real_separate_cluster_then_sequence_vessels_replace_original_record_pointer() {
        let mut graph = Graph::default();
        let member = add_node(&mut graph, "member", None, 1);
        let x = add_node(&mut graph, "X", None, 2);
        let cluster_vessel = add_node(&mut graph, "cluster-vessel", None, 501);
        let sequence_vessel = add_node(&mut graph, "sequence-vessel", None, 502);
        let edge = graph.add_edge(crate::Edge {
            source: member,
            target: x,
        });
        let mut owner = ArenaGraph::from_input(&graph);
        owner.node_order = vec![sequence_vessel, x];
        owner.nodes[member.0 as usize].cluster = Some(0);
        owner.nodes[cluster_vessel.0 as usize].sequence = Some(0);
        owner.clusters.push(ClusterState {
            members: vec![member],
            arrangement: ClusterArrangement::Row,
            desired_arrangement: ClusterArrangement::Row,
            padding: 0.0,
            vessel_tala_id: 501,
            fixed_size: false,
        });
        owner.sequences.push(SequenceState {
            members: vec![cluster_vessel],
            vessel_tala_id: 502,
            container: None,
            has_edge_abductions: true,
        });
        assert!(matches!(
            owner.bind_topology_vessel(TopologyVessel::Cluster(0), member),
            Err(OwnerTopologyError::VesselAliasesMember { .. })
        ));
        owner
            .bind_topology_vessel(TopologyVessel::Cluster(0), cluster_vessel)
            .unwrap();
        owner
            .bind_topology_vessel(TopologyVessel::Sequence(0), sequence_vessel)
            .unwrap();
        let slice = owner
            .register_owner_edge_abductions(
                vec![OwnerEdgeAbduction {
                    source_edge: Some(edge),
                    originally_from: Some(member),
                    originally_to: None,
                    current_from: Some(sequence_vessel),
                    current_to: Some(x),
                }],
                true,
            )
            .unwrap();
        let same_records = slice.clone();
        owner.abduct_owner_edge_abductions(&slice);
        assert_eq!(owner.edge_records[edge.0 as usize].from, sequence_vessel);
        assert_eq!(
            owner
                .topology_retained_closure(&[sequence_vessel, x])
                .unwrap(),
            [sequence_vessel, x, cluster_vessel, member]
        );
        assert_eq!(
            owner
                .restore_owner_edge_abductions(&slice, &[sequence_vessel, x])
                .unwrap(),
            OwnerAbductionSlice::default()
        );
        assert_eq!(
            owner
                .owner_edge_abduction(same_records.records[0])
                .unwrap()
                .originally_from,
            Some(sequence_vessel)
        );
        assert_eq!(owner.edge_records[edge.0 as usize].from, sequence_vessel);
        assert!(owner.nodes[member.0 as usize].edges.is_empty());
        assert_eq!(owner.nodes[sequence_vessel.0 as usize].edges, [edge]);
        assert_eq!(owner.node_order, [sequence_vessel, x]);
    }

    #[test]
    fn missing_production_lineage_is_rejected_and_topology_rollback_is_exact() {
        let (mut pipeline, placement, [c, _, d, child, x, _], [first, second]) = live_scope();
        let owner = &mut pipeline.graph;
        let before_edges: Vec<_> = owner
            .edge_records
            .iter()
            .map(|edge| (edge.from, edge.to))
            .collect();
        let before_incident: Vec<_> = owner.nodes.iter().map(|node| node.edges.read()).collect();
        let snapshot = owner.owner_topology_snapshot();
        let mut projected = placement.sized_edge_abductions.clone();
        projected[0].current_from.as_mut().unwrap().source_node = None;
        assert!(matches!(
            owner.from_sized_owner_abductions(&projected, true),
            Err(OwnerTopologyError::MissingSourceLineage { .. })
        ));
        let slice = owner
            .from_sized_owner_abductions(&placement.sized_edge_abductions, true)
            .unwrap();
        let retained_records: Vec<_> = slice
            .records
            .iter()
            .map(|&id| *owner.owner_edge_abduction(id).unwrap())
            .collect();
        owner.abduct_owner_edge_abductions(&slice);
        let component = owner
            .create_owner_graph_view(vec![d, x], vec![first])
            .unwrap();
        let mut journal = OwnerGraphOwnershipJournal::default();
        owner
            .set_owner_graph_reference(component, &[d, x], true, &mut journal)
            .unwrap();
        owner.set_topology_container_slice(Some(c), None);
        owner.restore_owner_topology_snapshot(snapshot);
        assert_eq!(
            owner
                .edge_records
                .iter()
                .map(|edge| (edge.from, edge.to))
                .collect::<Vec<_>>(),
            before_edges
        );
        assert_eq!(
            owner
                .nodes
                .iter()
                .map(|node| node.edges.read())
                .collect::<Vec<_>>(),
            before_incident
        );
        assert_eq!(owner.nodes[child.0 as usize].edges, [first, second]);
        assert_eq!(owner.topology_node_owner(d), Some(OwnerGraphId(0)));
        assert_eq!(
            slice
                .records
                .iter()
                .map(|&id| *owner.owner_edge_abduction(id).unwrap())
                .collect::<Vec<_>>(),
            retained_records
        );
        assert_eq!(
            owner
                .owner_graph_view(component)
                .unwrap()
                .nodes
                .visible_slots(),
            [Some(d), Some(x)]
        );
        assert!(
            matches!(owner.topology_container_slice(Some(c)), Some(Some(children)) if children.len() == 1)
        );
    }

    #[test]
    fn aggregate_carrier_without_real_vessel_is_rejected_before_owner_mutation() {
        let (mut pipeline, placement, [_, _, d, child, _, y], [first, _]) = live_scope();
        let owner = &mut pipeline.graph;
        owner.nodes[d.0 as usize].cluster = Some(0);
        owner.nodes[y.0 as usize].cluster = Some(0);
        owner.node_order.retain(|&node| node != y);
        owner.node_order_membership_valid = false;
        owner.clusters.push(ClusterState {
            members: vec![d, y],
            arrangement: ClusterArrangement::Row,
            desired_arrangement: ClusterArrangement::Row,
            padding: 0.0,
            vessel_tala_id: 501,
            fixed_size: false,
        });
        assert_eq!(
            owner.from_sized_owner_abductions(&placement.sized_edge_abductions, true),
            Err(OwnerTopologyError::UnboundVessel(TopologyVessel::Cluster(
                0
            )))
        );
        assert_eq!(owner.edge_records[first.0 as usize].from, child);
        assert!(owner.nodes[d.0 as usize].edges.is_empty());
        assert!(owner.shared_topology.records.is_empty());
    }

    #[test]
    fn retained_runtime_vessel_has_distinct_storage_and_no_public_input_slot() {
        let (mut pipeline, _, [_, _, d, _, _, y], _) = live_scope();
        let owner = &mut pipeline.graph;
        let order = owner.node_order.clone();
        let original_count = owner.nodes.len();
        let snapshot = owner.owner_topology_snapshot();
        let mut vessel_graph = Graph::default();
        add_node(&mut vessel_graph, "runtime-vessel", None, 501);
        let vessel_entity = ArenaGraph::from_input(&vessel_graph).nodes.remove(0);
        let vessel = owner.retain_topology_node(vessel_entity);
        assert_eq!(vessel.0 as usize, original_count);
        assert_ne!(vessel, d);
        assert_eq!(owner.topology_source_input_node(vessel), None);
        assert_eq!(owner.topology_node_owner(vessel), None);
        assert_eq!(owner.node_order, order);
        owner.clusters.push(ClusterState {
            members: vec![d, y],
            arrangement: ClusterArrangement::Row,
            desired_arrangement: ClusterArrangement::Row,
            padding: 0.0,
            vessel_tala_id: 501,
            fixed_size: false,
        });
        owner
            .bind_topology_vessel(TopologyVessel::Cluster(0), vessel)
            .unwrap();
        assert_eq!(
            owner.topology_retained_closure(&[vessel]).unwrap(),
            [vessel, d, y, NodeId(3)]
        );
        assert_eq!(owner.node_order, order);
        owner.restore_owner_topology_snapshot(snapshot);
        assert_eq!(owner.nodes.len(), original_count + 1);
        assert!(owner.clusters.is_empty());
        assert_eq!(owner.topology_node(vessel).unwrap().tala_id, 501);
        assert_eq!(owner.topology_source_input_node(vessel), None);
        assert_eq!(owner.node_order, order);
        assert!(
            owner
                .topology_vessel_node(TopologyVessel::Cluster(0))
                .is_none()
        );
    }

    #[test]
    fn original_owner_roots_reach_every_cluster_member_without_changing_carriers() {
        let mut graph = Graph::default();
        let container = add_node(&mut graph, "container", None, 10);
        let child = add_node(&mut graph, "child", Some(container), 11);
        let first = add_node(&mut graph, "first", None, 12);
        let second = add_node(&mut graph, "second", None, 13);
        let vessel = add_node(&mut graph, "vessel", None, 14);
        let edges = [first, second].map(|source| {
            graph.add_edge(crate::Edge {
                source,
                target: child,
            })
        });
        let mut owner = ArenaGraph::from_input(&graph);
        owner.node_order = vec![container, child, first];
        owner.invalidate_node_order_membership();
        owner.clusters.push(ClusterState {
            members: vec![first, second],
            arrangement: ClusterArrangement::Row,
            desired_arrangement: ClusterArrangement::Row,
            padding: 0.0,
            vessel_tala_id: 14,
            fixed_size: false,
        });
        for node in [first, second] {
            owner.nodes[node.0 as usize].cluster = Some(0);
        }
        owner
            .bind_topology_vessel(TopologyVessel::Cluster(0), vessel)
            .unwrap();
        let records = owner
            .register_owner_edge_abductions(
                edges
                    .into_iter()
                    .zip([first, second])
                    .map(|(source_edge, member)| OwnerEdgeAbduction {
                        source_edge: Some(source_edge),
                        originally_from: Some(member),
                        originally_to: Some(child),
                        current_from: Some(vessel),
                        current_to: Some(container),
                    })
                    .collect(),
                true,
            )
            .unwrap();
        owner.abduct_owner_edge_abductions(&records);
        let carrier_order = owner.node_order.clone();
        assert!(
            !owner
                .topology_retained_closure(&carrier_order)
                .unwrap()
                .contains(&second)
        );
        owner.synchronize_original_owner_graph_view();
        let roots = owner
            .owner_graph_view(OwnerGraphId(0))
            .unwrap()
            .nodes
            .require_nodes()
            .collect::<Vec<_>>();
        assert_eq!(roots, [container, child, vessel]);
        assert_eq!(owner.node_order, carrier_order);
        assert_eq!(
            owner.owner_view_local_node(OwnerGraphId(0), second),
            Some(second)
        );
        assert_eq!(
            owner.topology_retained_closure(&roots).unwrap(),
            [container, child, vessel, first, second]
        );
        assert!(
            owner
                .restore_owner_edge_abductions(&records, &roots)
                .unwrap()
                .records
                .is_empty()
        );
        for edge in edges {
            assert_eq!(
                (
                    owner.edge_records[edge.0 as usize].from,
                    owner.edge_records[edge.0 as usize].to
                ),
                (vessel, child)
            );
        }
        // Once the aggregate is expanded, original members remain themselves.
        owner.node_order = vec![container, child, first, second];
        owner.invalidate_node_order_membership();
        owner.synchronize_original_owner_graph_view();
        assert_eq!(
            owner
                .owner_graph_view(OwnerGraphId(0))
                .unwrap()
                .nodes
                .require_nodes()
                .collect::<Vec<_>>(),
            owner.node_order
        );
    }

    #[test]
    fn live_original_member_publication_preserves_dense_mapping_and_node_owners() {
        let (mut pipeline, _, [c, child_c, d, _, x, _], [first, second]) = live_scope();
        let owner = &mut pipeline.graph;
        owner.initialize_shared_topology();
        owner.node_order = vec![x, d, c];
        owner.edge_order.replace_values(&[second, first]);
        owner.synchronize_original_owner_graph_view();
        assert_eq!(
            owner
                .owner_graph_view(OwnerGraphId(0))
                .unwrap()
                .nodes
                .visible_slots(),
            [Some(x), Some(d), Some(c)]
        );
        assert_eq!(
            owner.owner_graph_view(OwnerGraphId(0)).unwrap().edges,
            [second, first]
        );
        assert_eq!(
            owner.owner_view_local_node(OwnerGraphId(0), child_c),
            Some(child_c)
        );
        assert_eq!(owner.topology_node_owner(child_c), Some(OwnerGraphId(0)));
    }

    #[test]
    fn arena_namespaces_reject_foreign_slices_and_journals_before_mutation() {
        let (mut first, placement, [_, _, d, _, x, _], [edge, _]) = live_scope();
        let (mut second, _, _, _) = live_scope();
        let slice = first
            .graph
            .from_sized_owner_abductions(&placement.sized_edge_abductions, true)
            .unwrap();
        second.graph.initialize_shared_topology();
        let first_arena = first.graph.topology_arena_id();
        let second_arena = second.graph.topology_arena_id();
        assert_ne!(first_arena, second_arena);
        assert_eq!(first.graph.clone().topology_arena_id(), first_arena);
        let before_endpoint = (
            second.graph.edge_records[edge.0 as usize].from,
            second.graph.edge_records[edge.0 as usize].to,
        );
        let before_incidence: Vec<_> = second
            .graph
            .nodes
            .iter()
            .map(|node| node.edges.read())
            .collect();
        assert_eq!(
            second.graph.restore_owner_edge_abductions(&slice, &[d, x]),
            Err(OwnerTopologyError::ForeignArena {
                expected: second_arena,
                actual: first_arena
            })
        );
        assert_eq!(
            (
                second.graph.edge_records[edge.0 as usize].from,
                second.graph.edge_records[edge.0 as usize].to
            ),
            before_endpoint
        );
        assert_eq!(
            second
                .graph
                .nodes
                .iter()
                .map(|node| node.edges.read())
                .collect::<Vec<_>>(),
            before_incidence
        );
        let first_view = first
            .graph
            .create_owner_graph_view(vec![d, x], vec![edge])
            .unwrap();
        let second_view = second
            .graph
            .create_owner_graph_view(vec![d, x], vec![edge])
            .unwrap();
        let mut journal = OwnerGraphOwnershipJournal::default();
        first
            .graph
            .set_owner_graph_reference(first_view, &[d, x], true, &mut journal)
            .unwrap();
        assert_eq!(
            second
                .graph
                .set_owner_graph_reference(second_view, &[d, x], true, &mut journal),
            Err(OwnerTopologyError::ForeignArena {
                expected: second_arena,
                actual: first_arena
            })
        );
        assert_eq!(second.graph.topology_node_owner(d), Some(OwnerGraphId(0)));
        assert_eq!(second.graph.topology_node_owner(x), Some(OwnerGraphId(0)));
    }

    #[test]
    fn post_snapshot_record_and_view_handles_remain_live_and_never_alias_later_allocations() {
        let (mut pipeline, placement, [_, _, d, _, x, _], [edge, _]) = live_scope();
        let owner = &mut pipeline.graph;
        let snapshot = owner.owner_topology_snapshot();
        let arena = owner.topology_arena_id();
        let retained = owner
            .from_sized_owner_abductions(&placement.sized_edge_abductions, true)
            .unwrap();
        let retained_view = owner
            .create_owner_graph_view(vec![d, x], vec![edge])
            .unwrap();
        owner.abduct_owner_edge_abductions(&retained);
        owner.restore_owner_topology_snapshot(snapshot);
        assert_eq!(owner.topology_arena_id(), arena);
        assert!(owner.owner_edge_abduction(retained.records[0]).is_some());
        assert!(owner.owner_graph_view(retained_view).is_some());
        let fresh = owner
            .from_sized_owner_abductions(&placement.sized_edge_abductions, true)
            .unwrap();
        let fresh_view = owner
            .create_owner_graph_view(vec![d, x], vec![edge])
            .unwrap();
        assert!(fresh.records[0].0 > retained.records.last().unwrap().0);
        assert!(fresh_view.0 > retained_view.0);
        assert!(
            owner
                .restore_owner_edge_abductions(&retained, &[d, x])
                .is_ok()
        );
        assert_eq!(
            owner.set_owner_graph_members(retained_view, vec![d], vec![]),
            Ok(())
        );
        assert_eq!(
            owner
                .owner_graph_view(fresh_view)
                .unwrap()
                .nodes
                .visible_slots(),
            [Some(d), Some(x)]
        );
        assert_eq!(fresh.arena, arena);
    }
}

#[cfg(test)]
mod live_node_graph_field_tests {
    use super::*;
    #[test]
    fn retained_owner_field_tracks_journal_and_topology_restore_and_detaches_for_candidates() {
        let mut input = Graph::default();
        let node = input.add_node(crate::Node::new(
            "a",
            Size {
                width: 20.0,
                height: 30.0,
            },
        ));
        let mut owner = ArenaGraph::from_input(&input);
        let field = owner.retain_owner_node_graph_field(node).unwrap();
        let alias = owner.retain_owner_node_graph_field(node).unwrap();
        assert!(field.same_field(&alias));
        let other = owner
            .create_owner_graph_view(Vec::new(), Vec::new())
            .unwrap();
        field.set(None);
        assert_eq!(owner.topology_node_owner(node), None);
        let mut journal = OwnerGraphOwnershipJournal::default();
        owner
            .add_owner_graph_node(other, node, &mut journal)
            .unwrap();
        assert_eq!(field.get(), Some(other));
        assert_eq!(journal.original_owner(node), Some(None));
        owner.restore_owner_graph_ownership(&journal);
        assert_eq!(field.get(), None);
        let snapshot = owner.owner_topology_snapshot();
        field.set(Some(other));
        owner.restore_owner_topology_snapshot(snapshot);
        assert_eq!(field.get(), None);
        assert!(field.same_field(&owner.retain_owner_node_graph_field(node).unwrap()));
        let mut candidate = owner.clone();
        let detached = candidate.retain_owner_node_graph_field(node).unwrap();
        assert!(!field.same_field(&detached));
        detached.set(Some(other));
        assert_eq!(owner.topology_node_owner(node), None);
        assert_eq!(candidate.topology_node_owner(node), Some(other));
    }
}

#[cfg(test)]
mod live_graph_nodes_field_tests {
    use super::*;
    #[test]
    fn member_field_replacement_keeps_ranges_and_rollback_repairs_original_aliases() {
        let mut input = Graph::default();
        for name in ["a", "b", "c"] {
            input.add_node(crate::Node::new(
                name,
                Size {
                    width: 20.0,
                    height: 30.0,
                },
            ));
        }
        let mut graph = ArenaGraph::from_input(&input);
        graph.initialize_shared_topology();
        let header = graph
            .allocate_owner_member_header(&[Some(NodeId(0)), Some(NodeId(1))], 2)
            .unwrap();
        graph
            .set_owner_graph_member_header(OwnerGraphId(0), header)
            .unwrap();
        let field = graph
            .retain_owner_graph_nodes_field(OwnerGraphId(0))
            .unwrap();
        let original = field.header();
        let snapshot = graph.owner_topology_snapshot();
        let mut range = original.iter_slots();
        field.replace(SharedNodeSlice::from_nodes(&[NodeId(2)]).unwrap());
        assert_eq!(range.next(), Some(Some(NodeId(0))));
        original.write_capacity_slot(1, Some(NodeId(2))).unwrap();
        assert_eq!(range.next(), Some(Some(NodeId(2))));
        assert_eq!(range.next(), None);
        assert_eq!(field.visible_slots(), vec![Some(NodeId(2))]);
        graph.restore_owner_topology_snapshot(snapshot);
        assert!(
            field.same_field(
                &graph
                    .retain_owner_graph_nodes_field(OwnerGraphId(0))
                    .unwrap()
            )
        );
        assert_eq!(
            field.visible_slots(),
            vec![Some(NodeId(0)), Some(NodeId(1))]
        );
        assert_eq!(original.visible_slots(), field.visible_slots());
        let candidate = graph.clone();
        let other = candidate
            .retain_owner_graph_nodes_field(OwnerGraphId(0))
            .unwrap();
        assert!(!field.same_field(&other));
        other.replace(SharedNodeSlice::nil());
        assert_eq!(field.len(), 2);
        assert!(other.is_nil());
    }
}

#[cfg(test)]
#[path = "owner_container_fields_tests.rs"]
mod owner_container_fields_tests;
