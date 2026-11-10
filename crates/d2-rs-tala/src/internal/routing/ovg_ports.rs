// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Incremental port construction from pinned routing/ovg.go addPorts.
use super::route_search_guard::OvgBuildBudget;
use super::{ArenaGraph, NodeId, Point, Port, PortSide, Rect};
use crate::internal::go_cell::{GoCell, GoCellMut, GoCellRef};
use crate::internal::nodeshape::ports::snap_points_with_table_column_count;
use d2_rs_core::stable_sort::StableSort;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

pub(super) fn push_pointer<T>(values: &mut Vec<T>, value: T) {
    if values.len() == values.capacity() {
        let capacity = crate::internal::container_slices::go127_pointer_growth_capacity(
            values.capacity(),
            values.len() + 1,
        )
        .expect("OVG pointer slice capacity");
        values.reserve_exact(capacity - values.len());
    }
    values.push(value);
}

#[derive(Clone, Copy, Default, Debug)]
pub(super) struct Metadata {
    directions: u16,
    center: bool,
}

/// Go leaves portOwnersByNode nil on ordinary visibility nodes. Most port
/// nodes have one owner: keep that entry in a single small allocation instead
/// of allocating both a map header and its first tree node. Coincident ports
/// retain every additional owner in key order, as the previous sorted map did.
#[derive(Default, Debug)]
pub(super) struct PortOwners(Option<Box<OwnerEntries>>);

#[derive(Debug)]
struct OwnerEntries {
    first: (NodeId, Metadata),
    #[allow(clippy::box_collection)]
    rest: Option<Box<BTreeMap<NodeId, Metadata>>>,
}

impl PortOwners {
    pub(super) fn is_empty(&self) -> bool {
        self.0.is_none()
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = (&NodeId, &Metadata)> {
        self.0.iter().flat_map(|entries| {
            std::iter::once((&entries.first.0, &entries.first.1))
                .chain(entries.rest.iter().flat_map(|rest| rest.iter()))
        })
    }

    pub(super) fn keys(&self) -> impl Iterator<Item = &NodeId> {
        self.iter().map(|(owner, _)| owner)
    }

    pub(super) fn get(&self, owner: &NodeId) -> Option<&Metadata> {
        let entries = self.0.as_ref()?;
        if entries.first.0 == *owner {
            Some(&entries.first.1)
        } else {
            entries.rest.as_ref()?.get(owner)
        }
    }

    pub(super) fn contains_key(&self, owner: &NodeId) -> bool {
        self.get(owner).is_some()
    }

    fn get_or_insert(&mut self, owner: NodeId) -> &mut Metadata {
        let entries = self.0.get_or_insert_with(|| {
            Box::new(OwnerEntries {
                first: (owner, Metadata::default()),
                rest: None,
            })
        });
        if owner < entries.first.0 {
            let old = std::mem::replace(&mut entries.first, (owner, Metadata::default()));
            entries
                .rest
                .get_or_insert_with(Default::default)
                .insert(old.0, old.1);
        }
        if owner == entries.first.0 {
            &mut entries.first.1
        } else {
            entries
                .rest
                .get_or_insert_with(Default::default)
                .entry(owner)
                .or_default()
        }
    }

    pub(super) fn insert(&mut self, owner: NodeId, metadata: Metadata) -> Option<Metadata> {
        let previous = self.get(&owner).copied();
        *self.get_or_insert(owner) = metadata;
        previous
    }

    fn values_mut(&mut self) -> impl Iterator<Item = &mut Metadata> {
        self.0.iter_mut().flat_map(|entries| {
            std::iter::once(&mut entries.first.1)
                .chain(entries.rest.iter_mut().flat_map(|rest| rest.values_mut()))
        })
    }

    fn get_mut(&mut self, owner: &NodeId) -> Option<&mut Metadata> {
        let entries = self.0.as_mut()?;
        if entries.first.0 == *owner {
            Some(&mut entries.first.1)
        } else {
            entries.rest.as_mut()?.get_mut(owner)
        }
    }

    #[cfg(test)]
    pub(super) fn remove(&mut self, owner: &NodeId) -> Option<Metadata> {
        let entries = self.0.as_mut()?;
        if entries.first.0 != *owner {
            return entries.rest.as_mut()?.remove(owner);
        }
        let previous = entries.first.1;
        if let Some(next) = entries.rest.as_mut().and_then(|rest| rest.pop_first()) {
            entries.first = next;
        } else {
            self.0 = None;
        }
        Some(previous)
    }
}

impl std::ops::Index<&NodeId> for PortOwners {
    type Output = Metadata;

    fn index(&self, owner: &NodeId) -> &Metadata {
        self.get(owner).expect("missing OVG port owner")
    }
}

#[derive(Debug)]
pub(super) struct PortNode {
    pub(super) point_ref: Option<crate::internal::owner_points::SharedOwnerPoint>,
    // The two source booleans occupy the high bits of the search index.
    // This is the same 30-bit index range the routing adjacency already uses;
    // the source OVG budget permits only 200,000 nodes.
    index_flags: u32,
    // Zero is nil; every real arena slot is stored one-based. The source
    // engine admits at most 10,000 graph nodes, far below this representation.
    container: Option<std::num::NonZeroU32>,
    /// Boxed: most OVG nodes keep the nil set, and the inline set would put
    /// every node allocation in the next allocator size class.
    #[allow(clippy::box_collection)]
    pub(super) near_ports: Option<Box<BTreeSet<NodeId>>>,
    pub(super) owners: PortOwners,
    pub(super) edges: super::ovg_edge_header::EdgeHeader<super::ovg_objects::IncidentEdge>,
}
const PORT_NODE_INDEX_MASK: u32 = (1 << 30) - 1;
const PORT_NODE_CENTER: u32 = 1 << 30;
const PORT_NODE_TUNNEL: u32 = 1 << 31;
impl PortEdge {
    pub(super) fn from_slot(&self) -> usize {
        self.from as usize
    }
    pub(super) fn to_slot(&self) -> usize {
        self.to as usize
    }
}
impl PortNode {
    pub(super) fn index(&self) -> usize {
        (self.index_flags & PORT_NODE_INDEX_MASK) as usize
    }
    pub(super) fn set_index(&mut self, index: usize) {
        let index = u32::try_from(index)
            .ok()
            .filter(|&index| index <= PORT_NODE_INDEX_MASK)
            .expect("OVG node search index");
        self.index_flags = (self.index_flags & !PORT_NODE_INDEX_MASK) | index;
    }
    pub(super) fn is_node_center(&self) -> bool {
        self.index_flags & PORT_NODE_CENTER != 0
    }
    pub(super) fn set_node_center(&mut self, center: bool) {
        self.index_flags =
            (self.index_flags & !PORT_NODE_CENTER) | if center { PORT_NODE_CENTER } else { 0 };
    }
    pub(super) fn is_tunnel(&self) -> bool {
        self.index_flags & PORT_NODE_TUNNEL != 0
    }
    fn set_tunnel(&mut self) {
        self.index_flags |= PORT_NODE_TUNNEL;
    }
    pub(super) fn container(&self) -> Option<NodeId> {
        self.container.map(|owner| NodeId(owner.get() - 1))
    }
    pub(super) fn set_container(&mut self, container: Option<NodeId>) {
        self.container = container.map(|owner| {
            std::num::NonZeroU32::new(owner.0.wrapping_add(1)).expect("OVG node container slot")
        });
    }
    pub(super) fn point(&self) -> Point {
        self.point_ref
            .as_ref()
            .expect("runtime error: invalid memory address or nil pointer dereference")
            .get()
    }
}

/// An OVG stores pointers to whole nodes. A hierarchy merge must not fork the
/// fields or edge-list header when it publishes the same object in another OVG.
#[derive(Clone, Debug)]
pub(super) struct SharedPortNode(
    pub(super) Arc<GoCell<PortNode>>,
    pub(super) Arc<super::ovg_objects::ObjectRoot>,
);
impl SharedPortNode {
    fn new(node: PortNode, root: Arc<super::ovg_objects::ObjectRoot>) -> Self {
        let record = Arc::new(GoCell::new(node));
        super::ovg_objects::ObjectRoot::add_node(&root, record.clone());
        Self(record, root)
    }
    pub(super) fn read(&self) -> GoCellRef<'_, PortNode> {
        self.0
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
    pub(super) fn write(&self) -> GoCellMut<'_, PortNode> {
        self.0
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
    pub(super) fn same_allocation(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
    pub(super) fn allocation_key(&self) -> usize {
        Arc::as_ptr(&self.0) as usize
    }
    pub(super) fn point(&self) -> Point {
        self.read().point()
    }
    pub(super) fn container(&self) -> Option<NodeId> {
        self.read().container()
    }
    pub(super) fn near_ports(&self) -> Option<BTreeSet<NodeId>> {
        self.read().near_ports.as_deref().cloned()
    }
    pub(super) fn is_tunnel(&self) -> bool {
        self.read().is_tunnel()
    }
    pub(super) fn index(&self) -> usize {
        self.read().index()
    }
}
#[derive(Clone, Debug)]
pub(super) struct PortEdge {
    /// OVG node slots of the endpoints, narrowed to u32 so each edge object
    /// takes the next smaller allocation class. Read them with `from_slot()`/`to_slot()`.
    pub(super) from: u32,
    pub(super) to: u32,
    pub(super) from_object: Arc<GoCell<PortNode>>,
    pub(super) to_object: Arc<GoCell<PortNode>>,
    pub(super) distance: f64,
}
/// An `occupied` value: the node slot, plus under `diagnostic-traces` the
/// point as published, which only the lifecycle probe's sorted dump reads.
#[cfg(feature = "diagnostic-traces")]
type Occupant = (Point, usize);
#[cfg(not(feature = "diagnostic-traces"))]
type Occupant = usize;
#[inline]
fn occupant_entry(point: Point, index: usize) -> Occupant {
    #[cfg(feature = "diagnostic-traces")]
    {
        (point, index)
    }
    #[cfg(not(feature = "diagnostic-traces"))]
    {
        let _ = point;
        index
    }
}
#[inline]
fn occupant_slot(entry: &Occupant) -> usize {
    #[cfg(feature = "diagnostic-traces")]
    {
        entry.1
    }
    #[cfg(not(feature = "diagnostic-traces"))]
    {
        *entry
    }
}
/// Go's OVG.Ports contains only node pointers. Keep that compact list separate
/// from the flat routing adapter's slot/Port records, which blocker scans do
/// not read. Both arrays retain the same order, including repeated nodes.
#[derive(Default, Clone, Debug)]
pub(super) struct PortList {
    records: Vec<(usize, Port)>,
    nodes: Vec<Arc<GoCell<PortNode>>>,
}
impl PortList {
    fn with_capacity(capacity: usize) -> Self {
        Self {
            records: Vec::with_capacity(capacity),
            nodes: Vec::with_capacity(capacity),
        }
    }
    pub(super) fn nodes(&self) -> &[Arc<GoCell<PortNode>>] {
        &self.nodes
    }
    pub(super) fn as_slice(&self) -> &[(usize, Port)] {
        &self.records
    }
    fn push(&mut self, record: (usize, Port), node: &SharedPortNode) {
        self.records.push(record);
        self.nodes.push(node.0.clone());
    }
}
impl std::ops::Deref for PortList {
    type Target = Vec<(usize, Port)>;

    fn deref(&self) -> &Self::Target {
        &self.records
    }
}
impl<'a> IntoIterator for &'a PortList {
    type Item = &'a (usize, Port);
    type IntoIter = std::slice::Iter<'a, (usize, Port)>;

    fn into_iter(self) -> Self::IntoIter {
        self.records.iter()
    }
}

#[derive(Default, Clone, Debug)]
pub(super) struct PortLists<T = PortList> {
    // OVG.Ports is indexed by dense arena owner id so repeated blocker queries
    // need no tree search. A missing slot differs from a present empty list,
    // just as a missing Go map key differs from an assigned nil slice.
    // Sparse subgraphs can contain only high owner ids. Keep their lists in
    // overflow until a dense prefix fits within twice the number of owners;
    // storage must follow this OVG's owner count, not the whole arena's size.
    // The dense prefix followed by overflow preserves ascending traversal.
    slots: Vec<Option<(NodeId, T)>>,
    overflow: BTreeMap<NodeId, T>,
    len: usize,
}
impl<T> PortLists<T> {
    pub(super) fn get(&self, owner: &NodeId) -> Option<&T> {
        if let Some(slot) = self.slots.get(owner.0 as usize) {
            slot.as_ref().map(|(_, ports)| ports)
        } else {
            self.overflow.get(owner)
        }
    }
    fn get_mut(&mut self, owner: &NodeId) -> Option<&mut T> {
        if let Some(slot) = self.slots.get_mut(owner.0 as usize) {
            slot.as_mut().map(|(_, ports)| ports)
        } else {
            self.overflow.get_mut(owner)
        }
    }
    fn contains_key(&self, owner: &NodeId) -> bool {
        self.get(owner).is_some()
    }
    fn insert(&mut self, owner: NodeId, ports: T) {
        if !self.contains_key(&owner) {
            self.len += 1;
        }
        let index = owner.0 as usize;
        if index >= self.slots.len() && index < self.len.saturating_mul(2) {
            self.slots.resize_with(index + 1, || None);
            // Every remaining overflow key stays beyond the dense prefix.
            while self
                .overflow
                .first_key_value()
                .is_some_and(|(owner, _)| (owner.0 as usize) < self.slots.len())
            {
                let (owner, ports) = self.overflow.pop_first().unwrap();
                self.slots[owner.0 as usize] = Some((owner, ports));
            }
        }
        if let Some(slot) = self.slots.get_mut(index) {
            *slot = Some((owner, ports));
        } else {
            self.overflow.insert(owner, ports);
        }
    }
    pub(super) fn iter(&self) -> impl Iterator<Item = (&NodeId, &T)> {
        self.slots
            .iter()
            .flatten()
            .map(|(owner, ports)| (owner, ports))
            .chain(self.overflow.iter())
    }
    fn keys(&self) -> impl Iterator<Item = &NodeId> {
        self.iter().map(|(owner, _)| owner)
    }
    #[cfg(test)]
    fn values(&self) -> impl Iterator<Item = &T> {
        self.iter().map(|(_, ports)| ports)
    }
    fn values_mut(&mut self) -> impl Iterator<Item = &mut T> {
        self.slots
            .iter_mut()
            .flatten()
            .map(|(_, ports)| ports)
            .chain(self.overflow.values_mut())
    }
}
impl PortLists {
    fn get_or_default(&mut self, owner: NodeId) -> &mut PortList {
        if !self.contains_key(&owner) {
            self.insert(owner, PortList::default());
        }
        self.get_mut(&owner).unwrap()
    }
    /// OVG.Ports retains only node pointers in Go (ovg.go:25). Once build
    /// mutation ends, the adapter's Port values and owning node handles are
    /// unnecessary: routing reads the same live points by canonical index.
    /// Preserve every owner's list order, repeated nodes and empty entries.
    fn into_indices(self, indices: &[usize]) -> PortLists<Vec<usize>> {
        let compact = |ports: PortList| {
            ports
                .records
                .iter()
                .map(|(slot, _)| indices[*slot])
                .collect()
        };
        PortLists {
            slots: self
                .slots
                .into_iter()
                .map(|slot| slot.map(|(owner, ports)| (owner, compact(ports))))
                .collect(),
            overflow: self
                .overflow
                .into_iter()
                .map(|(owner, ports)| (owner, compact(ports)))
                .collect(),
            len: self.len,
        }
    }
}
impl<T> std::ops::Index<&NodeId> for PortLists<T> {
    type Output = T;

    fn index(&self, owner: &NodeId) -> &Self::Output {
        self.get(owner).expect("missing port owner")
    }
}

#[derive(Default, Clone, Debug)]
pub(super) struct PortBuild {
    // Stable identity storage, separate from the source Nodes slice. Maps
    // retain these objects after removal and before unchecked publication.
    root: Arc<super::ovg_objects::ObjectRoot>,
    pub(super) nodes: Vec<SharedPortNode>,
    // Keyed by allocation address; lookup-only.
    node_slots: crate::internal::FloatKeyHashMap<usize, usize>,
    pub(super) members: Vec<usize>,
    pub(super) members_allocated: bool,
    member_reservations: Vec<bool>,
    pub(super) centers: BTreeMap<NodeId, usize>,
    pub(super) fixed_cache: Vec<super::ovg_intersections::FixedCache>,
    // Point-bit keys; lookup-only outside a sorted diagnostic dump.
    occupied: crate::internal::FloatKeyHashMap<(u64, u64), Occupant>,
    nan_occupied: Vec<(Point, usize)>,
    pub(super) ports: PortLists,
    pub(super) port_order: Vec<NodeId>,
    pub(super) edges: Vec<super::ovg_objects::SharedPortEdge>,
    pub(super) edges_allocated: bool,
    pub(super) horizontal_edges: super::static_edge_index::Axes,
    pub(super) vertical_edges: super::static_edge_index::Axes,
}
impl PortBuild {
    pub(super) fn occupied_node(&self, point: Point) -> Option<usize> {
        if point.x.is_nan() || point.y.is_nan() {
            None
        } else {
            self.occupied
                .get(&super::go_point_map_key(point))
                .map(occupant_slot)
        }
    }
    pub(super) fn owner_directions(
        &self,
        index: usize,
    ) -> impl Iterator<Item = (NodeId, u16)> + '_ {
        let node = self.nodes[index].clone();
        let owners = node.read().owners.keys().copied().collect::<Vec<_>>();
        owners.into_iter().filter_map(move |owner| {
            node.read()
                .owners
                .get(&owner)
                .map(|metadata| (owner, metadata.directions))
        })
    }
    pub(super) fn connect(
        &mut self,
        from: usize,
        to: usize,
        budget: super::ovg_intersections::Budget<'_, '_>,
    ) -> Result<(), ()> {
        super::ovg_intersections::step(budget)?;
        if (self.nodes[from].read().is_node_center() && self.nodes[to].read().owners.is_empty())
            || (self.nodes[to].read().is_node_center() && self.nodes[from].read().owners.is_empty())
        {
            return Ok(());
        }
        if let Some(b) = budget {
            b.reserve_edges(1)?;
        }
        let index = self.edges.len();
        let edge =
            super::ovg_objects::SharedPortEdge::new(&self.nodes[from], &self.nodes[to], from, to);
        let incident = edge.incident(index);
        push_pointer(&mut self.edges, edge);
        // Go publishes the edge and both incidences before dereferencing Point.
        self.edges_allocated = true;
        self.nodes[from].write().edges.push(incident.clone());
        self.nodes[to].write().edges.push(incident);
        let a = self.nodes[from].point();
        let b = self.nodes[to].point();
        if a.x == b.x {
            self.vertical_edges.append(a.x, index);
        } else if a.y == b.y {
            self.horizontal_edges.append(a.y, index);
        }
        self.edges[index].write().distance = super::euclidean_distance(a, b);

        Ok(())
    }
    pub(super) fn add_point(
        &mut self,
        point: Point,
        budget: Option<&OvgBuildBudget<'_>>,
    ) -> Result<usize, ()> {
        if let Some(budget) = budget {
            budget.reserve_work(1)?;
        }
        let reflexive = !point.x.is_nan() && !point.y.is_nan();
        let key = super::go_point_map_key(point);
        if reflexive && let Some(index) = self.occupied.get(&key).map(occupant_slot) {
            return Ok(index);
        }
        if let Some(budget) = budget {
            budget.reserve_nodes(1)?;
        }
        let index = self.allocate_node(point);
        self.publish_member(index, budget.is_some());
        Ok(index)
    }

    pub(super) fn allocate_node(&mut self, point: Point) -> usize {
        let index = self.nodes.len();
        self.nodes.push(SharedPortNode::new(
            PortNode {
                point_ref: Some(crate::internal::owner_points::SharedOwnerPoint::new(point)),
                index_flags: 0,
                owners: PortOwners::default(),
                edges: super::ovg_edge_header::EdgeHeader::default(),
                container: None,
                near_ports: None,
            },
            self.root.clone(),
        ));
        self.node_slots
            .insert(self.nodes[index].allocation_key(), index);
        index
    }
    fn import_node(&mut self, other: &Self, foreign: usize) -> usize {
        self.import_object(other.nodes[foreign].clone())
    }
    pub(super) fn import_object(&mut self, node: SharedPortNode) -> usize {
        let key = node.allocation_key();
        if let Some(&local) = self.node_slots.get(&key) {
            return local;
        }
        let local = self.nodes.len();
        self.nodes.push(node.clone());
        self.node_slots.insert(key, local);
        local
    }
    pub(super) fn object_slot(&self, node: &SharedPortNode) -> usize {
        self.node_slots[&node.allocation_key()]
    }
    pub(super) fn object_slot_by_key(&self, key: usize) -> usize {
        self.node_slots[&key]
    }
    pub(super) fn retain_incident_objects(&mut self) {
        let mut cursor = 0;
        while cursor < self.nodes.len() {
            let node = self.nodes[cursor].clone();
            // Go's adjacency walk selects a pointer. Only a newly discovered
            // foreign object needs an owning handle in our identity storage.
            node.read().edges.with_slots(|slots| {
                for incident in slots {
                    incident
                        .as_ref()
                        .expect("runtime error: invalid memory address or nil pointer dereference")
                        .with_record(|edge| {
                            let other = if Arc::ptr_eq(&node.0, &edge.from_object) {
                                &edge.to_object
                            } else {
                                &edge.from_object
                            };
                            if !self.node_slots.contains_key(&(Arc::as_ptr(other) as usize)) {
                                self.import_object(SharedPortNode(other.clone(), node.1.clone()));
                            }
                        });
                }
            });
            cursor += 1;
        }
    }
    pub(super) fn publish_member(&mut self, index: usize, reserved: bool) {
        push_pointer(&mut self.members, index);
        self.members_allocated = true;
        self.member_reservations.push(reserved);
        let point = self.nodes[index].point();
        if point.x.is_nan() || point.y.is_nan() {
            self.nan_occupied.push((point, index));
        } else {
            self.occupied
                .insert(super::go_point_map_key(point), occupant_entry(point, index));
        }
    }
    pub(super) fn member_nodes(&self) -> impl Iterator<Item = &SharedPortNode> {
        self.members.iter().map(|&index| &self.nodes[index])
    }
    /// Drop the allocation-slot map, publication reservations and fixed
    /// overlap cache, read only while importing objects, publishing members
    /// or constructing the OVG. Lookups (`occupied`, `ports`, `centers`,
    /// `nodes`, `edges`, axes) remain.
    pub(super) fn release_mutation_tables(&mut self) {
        self.node_slots = Default::default();
        self.member_reservations = Vec::new();
        self.fixed_cache = Vec::new();
    }
    /// Publish the surviving lookups in the frozen search's canonical index
    /// space. They retain Go's object identities directly, including ghosts
    /// and the last position of a repeated member, without a construction-slot
    /// translation on every read. Object fields and live Point allocations
    /// already belong to the search and static edge tables; no mutation can
    /// follow. The caller retains the source member count for array sizing.
    pub(super) fn freeze_lookups(&mut self, indices: &[usize]) -> PortLists<Vec<usize>> {
        for entry in self.occupied.values_mut() {
            #[cfg(feature = "diagnostic-traces")]
            let slot = &mut entry.1;
            #[cfg(not(feature = "diagnostic-traces"))]
            let slot = entry;
            *slot = indices[*slot];
        }
        for (_, slot) in &mut self.nan_occupied {
            *slot = indices[*slot];
        }
        for slot in self.centers.values_mut() {
            *slot = indices[*slot];
        }
        let ports = std::mem::take(&mut self.ports).into_indices(indices);
        self.port_order = Vec::new();
        self.nodes = Vec::new();
        self.members = Vec::new();
        self.edges = Vec::new();
        self.root = Default::default();
        self.horizontal_edges = Default::default();
        self.vertical_edges = Default::default();
        ports
    }
    /// Move this build out exactly as `clone` would copy it, leaving an
    /// empty build behind. Shared node/edge objects and the root are the same
    /// allocations either way and maps keep their tables; every `Vec` (and
    /// each port list) is re-laid with capacity equal to its length, as
    /// `Vec::clone` allocates, so later Go-style growth sees the same caps.
    pub(super) fn take_as_clone(&mut self) -> Self {
        fn exact<T>(values: Vec<T>) -> Vec<T> {
            let mut copy = Vec::with_capacity(values.len());
            copy.extend(values);
            copy
        }
        let mut build = std::mem::take(self);
        build.nodes = exact(std::mem::take(&mut build.nodes));
        build.members = exact(std::mem::take(&mut build.members));
        build.member_reservations = exact(std::mem::take(&mut build.member_reservations));
        build.fixed_cache = exact(std::mem::take(&mut build.fixed_cache));
        build.nan_occupied = exact(std::mem::take(&mut build.nan_occupied));
        build.port_order = exact(std::mem::take(&mut build.port_order));
        build.edges = exact(std::mem::take(&mut build.edges));
        for entries in build.ports.values_mut() {
            entries.records = exact(std::mem::take(&mut entries.records));
            entries.nodes = exact(std::mem::take(&mut entries.nodes));
        }
        build
    }
    pub(super) fn unreserved_member_count(&self) -> usize {
        self.member_reservations
            .iter()
            .filter(|reserved| !**reserved)
            .count()
    }
    pub(super) fn add_existing_node(
        &mut self,
        index: usize,
        budget: Option<&OvgBuildBudget<'_>>,
    ) -> Result<usize, ()> {
        if let Some(b) = budget {
            b.reserve_work(1)?;
        }
        let point = self.nodes[index].point();
        if !point.x.is_nan()
            && !point.y.is_nan()
            && let Some(occupant) = self
                .occupied
                .get(&super::go_point_map_key(point))
                .map(occupant_slot)
        {
            return Ok(occupant);
        }
        if let Some(b) = budget {
            b.reserve_nodes(1)?;
        }
        self.publish_member(index, budget.is_some());
        Ok(index)
    }

    pub(super) fn connect_centers(
        &mut self,
        graph: &ArenaGraph,
        bounding: &[NodeId],
        budget: super::ovg_intersections::Budget<'_, '_>,
    ) -> Result<(), ()> {
        use super::ovg_intersections::{check, rect, step};
        check(budget)?;
        for &owner in bounding {
            step(budget)?;
            let rect = rect(graph, owner);
            let point = Point {
                x: rect.origin.x + rect.size.width / 2.0,
                y: rect.origin.y + rect.size.height / 2.0,
            };
            // newCandidateNode reads its argument before its guarded step.
            step(budget)?;
            let center = self.allocate_node(point);
            self.centers.insert(owner, center);
            self.nodes[center]
                .write()
                .set_node_center(!graph.nodes[owner.0 as usize].is_invisible);
            step(budget)?;
            if let Some(b) = budget {
                b.reserve_nodes(1)?;
            }
            self.publish_member(center, budget.is_some());
            let count = self.ports.get(&owner).map_or(0, |ports| ports.len());
            for slot in 0..count {
                let port = self.ports[&owner][slot].0;
                self.connect(center, port, budget)?;
            }
        }
        check(budget)
    }
    pub(super) fn remove_isolated(
        &mut self,
        budget: super::ovg_intersections::Budget<'_, '_>,
    ) -> Result<(), ()> {
        use super::ovg_intersections::{check, step};
        check(budget)?;
        let mut members = Vec::new();
        let mut reservations = Vec::new();
        for (&node, &reserved) in self.members.iter().zip(&self.member_reservations) {
            step(budget)?;
            if !self.nodes[node].read().edges.is_empty() {
                push_pointer(&mut members, node);
                reservations.push(reserved);
            }
        }
        self.members = members;
        self.member_reservations = reservations;
        self.members_allocated = true;
        check(budget)
    }

    pub(super) fn add_tree_port_owner(
        &mut self,
        index: usize,
        owner: NodeId,
        raw_direction: i64,
        physical_side: PortSide,
    ) -> Option<Port> {
        let identity = index;
        if self.nodes[identity].read().owners.contains_key(&owner) {
            return None;
        }
        if !self.ports.contains_key(&owner) {
            self.port_order.push(owner);
        }
        let entries = self.ports.get_or_default(owner);
        // Preserve the full source direction in canonical metadata, including
        // a Tree.Orientation changed by a callback after center-port selection.
        // The legacy flat Port view only represents cardinal directions.
        let direction = match raw_direction {
            4 => PortSide::Top,
            5 => PortSide::Right,
            6 => PortSide::Bottom,
            7 => PortSide::Left,
            _ => physical_side,
        };
        let port = Port {
            point: self.nodes[index].point(),
            side: physical_side,
            direction,
            index: entries.len(),
            tunnel: None,
            is_center: false,
        };
        entries.push((identity, port), &self.nodes[identity]);
        let directions = if (0..=8).contains(&raw_direction) {
            1 << raw_direction
        } else {
            0
        };
        self.nodes[identity].write().owners.insert(
            owner,
            Metadata {
                directions,
                center: false,
            },
        );
        Some(port)
    }

    pub(super) fn port_directions(&self, index: usize, owner: NodeId) -> u16 {
        self.nodes[index]
            .read()
            .owners
            .get(&owner)
            .map_or(0, |metadata| metadata.directions)
    }

    pub(super) fn add_tunnels(
        &mut self,
        graph: &ArenaGraph,
        nodes: &[NodeId],
        budget: super::ovg_intersections::Budget<'_, '_>,
    ) -> Result<Vec<(NodeId, Port)>, ()> {
        use super::ovg_intersections::check;
        check(budget)?;
        let tunnels = super::ovg_tunnels::build(graph, nodes, budget)?;
        self.append_tunnels(tunnels, budget)
    }

    pub(super) fn append_tunnels(
        &mut self,
        tunnels: Vec<[super::ovg_tunnels::Entry; 2]>,
        budget: super::ovg_intersections::Budget<'_, '_>,
    ) -> Result<Vec<(NodeId, Port)>, ()> {
        use super::ovg_intersections::{check, step};
        let mut added = Vec::new();
        for (tunnel_id, tunnel) in tunnels.into_iter().enumerate() {
            step(budget)?;
            let mut endpoints = [0; 2];
            for (side, entry) in tunnel.into_iter().enumerate() {
                step(budget)?;
                let before = self.nodes.len();
                let index = self.add_point(entry.point, budget)?;
                let identity = index;
                if index < before {
                    // A detached endpoint has exactly one owner, with an
                    // empty direction set. Merge preserves existing roles.
                    step(budget)?;
                }
                self.nodes[identity]
                    .write()
                    .owners
                    .get_or_insert(entry.owner);
                self.nodes[identity].write().set_tunnel();
                if !self.ports.contains_key(&entry.owner) {
                    self.port_order.push(entry.owner);
                }
                let metadata = self.nodes[identity].read().owners[&entry.owner];
                let point = self.nodes[index].point();
                let ports = self.ports.get_or_default(entry.owner);
                let mut port = ports
                    .iter()
                    .find(|(_, p)| p.point == point)
                    .map(|(_, p)| *p)
                    .unwrap_or(Port {
                        point,
                        side: entry.side,
                        direction: entry.side,
                        index: ports.len(),
                        tunnel: None,
                        is_center: false,
                    });
                port.point = point;
                port.is_center = metadata.center;
                port.tunnel = Some(tunnel_id);
                ports.push((identity, port), &self.nodes[identity]);
                added.push((entry.owner, port));
                endpoints[side] = identity;
            }
            let [from, to] = endpoints;
            // Self connections retain two incidences of the same edge pointer.
            self.connect(from, to, budget)?;
        }
        check(budget)?;
        Ok(added)
    }

    #[cfg(feature = "diagnostic-traces")]
    pub(super) fn observe_tunnels(&self, graph: &ArenaGraph) -> serde_json::Value {
        use serde_json::json;
        let mut out = self.observe(graph);
        for (record, identity) in out["nodes"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .zip(self.member_nodes())
        {
            record["is_tunnel"] = json!(identity.is_tunnel());
            record["edges"] = json!(identity.read().edges);
        }
        let members = self
            .members
            .iter()
            .enumerate()
            .map(|(i, &node)| (node, i))
            .collect::<BTreeMap<_, _>>();
        out["edges"] = json!(self.edges.iter().map(|edge| { let edge = edge.read(); json!({"from":members[&edge.from_slot()],"to":members[&edge.to_slot()],"distance":format!("{:016x}",edge.distance.to_bits())}) }).collect::<Vec<_>>());
        out
    }
    #[cfg(feature = "diagnostic-traces")]
    pub(super) fn observe_connections(&self, graph: &ArenaGraph) -> serde_json::Value {
        use serde_json::json;
        let mut out = self.observe_tunnels(graph);
        out["edge_capacity"] = json!(self.edges.capacity());
        out["edges_nil"] = json!(!self.edges_allocated);
        for (i, node) in out["nodes"].as_array_mut().unwrap().iter_mut().enumerate() {
            let edges = &self.nodes[self.members[i]].read().edges;
            node["edge_capacity"] = json!(edges.capacity());
            node["edges_nil"] = json!(edges.is_nil());
        }
        out
    }

    #[cfg(feature = "diagnostic-traces")]
    pub(super) fn observe_lifecycle(&self, graph: &ArenaGraph) -> serde_json::Value {
        self.observe_lifecycle_stage(graph, false)
    }

    #[cfg(feature = "diagnostic-traces")]
    pub(super) fn observe_lifecycle_stage(
        &self,
        graph: &ArenaGraph,
        finalized: bool,
    ) -> serde_json::Value {
        self.observe_lifecycle_detail(graph, finalized, None)
    }
    #[cfg(feature = "diagnostic-traces")]
    pub(super) fn observe_imports(
        &self,
        graph: &ArenaGraph,
        imports: &[super::ovg_import::ImportedRoute],
    ) -> serde_json::Value {
        self.observe_lifecycle_detail(graph, true, Some(imports))
    }
    #[cfg(feature = "diagnostic-traces")]
    fn observe_lifecycle_detail(
        &self,
        graph: &ArenaGraph,
        finalized: bool,
        imports: Option<&[super::ovg_import::ImportedRoute]>,
    ) -> serde_json::Value {
        use serde_json::json;
        fn add(pool: &mut Vec<usize>, ids: &mut BTreeMap<usize, usize>, node: usize) {
            if let std::collections::btree_map::Entry::Vacant(entry) = ids.entry(node) {
                entry.insert(pool.len());
                pool.push(node);
            }
        }
        let mut pool = Vec::new();
        let mut ids = BTreeMap::new();
        for &node in &self.members {
            add(&mut pool, &mut ids, node);
        }
        let mut owners = self
            .centers
            .keys()
            .chain(self.ports.keys())
            .copied()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        owners.stable_sort_by_key(|owner| graph.nodes[owner.0 as usize].tala_id);
        for owner in &owners {
            if let Some(&node) = self.centers.get(owner) {
                add(&mut pool, &mut ids, node);
            }
        }
        for owner in &owners {
            for &(node, _) in self.ports.get(owner).into_iter().flatten() {
                add(&mut pool, &mut ids, node);
            }
        }
        let mut occupied = self
            .occupied
            .values()
            .copied()
            .chain(self.nan_occupied.iter().copied())
            .collect::<Vec<_>>();
        occupied.stable_sort_by_key(|(point, node)| {
            (
                point.x.to_bits(),
                point.y.to_bits(),
                ids.get(node).copied().unwrap_or(usize::MAX),
            )
        });
        for &(_, node) in &occupied {
            add(&mut pool, &mut ids, node);
        }
        for route in imports.into_iter().flatten() {
            for &node in &route.nodes {
                add(&mut pool, &mut ids, node);
            }
        }
        let bits = |point: Point| {
            [
                format!("{:016x}", point.x.to_bits()),
                format!("{:016x}", point.y.to_bits()),
            ]
        };
        let objects = pool.iter().map(|&index| {
            let node = &self.nodes[index].read();
            let owners = node.owners.iter().map(|(owner,metadata)| (graph.nodes[owner.0 as usize].tala_id.to_string(),json!({"directions":metadata.directions,"center":metadata.center}))).collect::<serde_json::Map<_,_>>();
            let mut value = json!({"point":bits(node.point()),"owners":owners,"index":node.index(),"is_tunnel":node.is_tunnel(),"is_node_center":node.is_node_center(),"edges":node.edges,"edge_capacity":node.edges.capacity(),"edges_nil":node.edges.is_nil()});
            if finalized {
                value["container"] = json!(node.container().map(|owner| graph.nodes[owner.0 as usize].tala_id));
                value["near_ports"] = json!(node.near_ports.as_ref().map(|owners| owners.iter().map(|owner| graph.nodes[owner.0 as usize].tala_id).collect::<BTreeSet<_>>()));
            }
            value
        }).collect::<Vec<_>>();
        let ports = self
            .ports
            .iter()
            .map(|(owner, entries)| {
                (
                    graph.nodes[owner.0 as usize].tala_id.to_string(),
                    json!(
                        entries
                            .iter()
                            .map(|(node, _)| ids[node])
                            .collect::<Vec<_>>()
                    ),
                )
            })
            .collect::<serde_json::Map<_, _>>();
        let centers = self
            .centers
            .iter()
            .map(|(owner, node)| {
                (
                    graph.nodes[owner.0 as usize].tala_id.to_string(),
                    json!(ids[node]),
                )
            })
            .collect::<serde_json::Map<_, _>>();
        let mut output = json!({"objects":objects,"nodes":self.members.iter().map(|node|ids[node]).collect::<Vec<_>>(),"nodes_nil":!self.members_allocated,"node_capacity":self.members.capacity(),"ports":ports,"centers":centers,
            "occupied":occupied.iter().map(|(point,node)|json!({"point":bits(*point),"node":ids[node]})).collect::<Vec<_>>(),
            "edges":self.edges.iter().map(|edge| { let edge = edge.read(); json!({"from":ids[&edge.from_slot()],"to":ids[&edge.to_slot()],"distance":format!("{:016x}",edge.distance.to_bits())}) }).collect::<Vec<_>>(),"edge_capacity":self.edges.capacity(),"edges_nil":!self.edges_allocated});
        if let Some(imports) = imports {
            use crate::internal::owner_points::SharedOwnerPoint;
            let mut point_ids = BTreeMap::<(bool, usize), usize>::new();
            let mut point_id = |key: Option<(bool, usize)>| {
                key.map(|key| {
                    let next = point_ids.len();
                    *point_ids.entry(key).or_insert(next)
                })
            };
            let shared_key = |point: &SharedOwnerPoint| (true, point.allocation_key());
            output["point_graph_nodes"] = json!(
                graph
                    .graph_node_order()
                    .iter()
                    .map(|&node| point_id(
                        graph
                            .owner_node_point(node)
                            .unwrap()
                            .shared_point()
                            .as_ref()
                            .map(shared_key)
                    ))
                    .collect::<Vec<_>>()
            );
            output["point_graph_routes"] = json!(
                graph
                    .edge_order
                    .header()
                    .iter_slots()
                    .map(|edge| {
                        let edge = edge.expect("nil observed edge");
                        let points = &graph.edge_records[edge.0 as usize].route;
                        (0..points.len())
                            .map(|i| point_id(points.slot(i).as_ref().map(shared_key)))
                            .collect::<Vec<_>>()
                    })
                    .collect::<Vec<_>>()
            );
            output["point_refs"] = json!(
                pool.iter()
                    .map(|&node| point_id(Some(
                        self.nodes[node]
                            .read()
                            .point_ref
                            .as_ref()
                            .map_or((false, node), shared_key)
                    )))
                    .collect::<Vec<_>>()
            );
            output["imports"] = json!(imports.iter().map(|route|json!({"nodes":route.nodes.iter().map(|node|ids[node]).collect::<Vec<_>>(),"node_capacity":route.nodes.capacity(),"nodes_nil":route.nodes.is_empty(),"last_point":route.last.as_ref().map(|p|bits(p.get())),"last_point_ref":point_id(route.last.as_ref().map(shared_key)),"endpoints":route.endpoints.map(|values|values.map(bits))})).collect::<Vec<_>>());
        }
        output
    }

    /// Transform each published object and every owner's complete direction set,
    /// then rebuild coordinate occupancy (ovg_hierarchy.go).
    pub(super) fn transform_hierarchy(&mut self, transpose: bool, x: bool, y: bool) {
        fn direction(mut d: u32, transpose: bool, x: bool, y: bool) -> u32 {
            if transpose {
                d = match d {
                    4 => 7,
                    6 => 5,
                    7 => 4,
                    5 => 6,
                    1 => 3,
                    3 => 1,
                    _ => d,
                };
            }
            if x {
                d = match d {
                    7 => 5,
                    5 => 7,
                    0 => 1,
                    1 => 0,
                    3 => 2,
                    2 => 3,
                    _ => d,
                };
            }
            if y {
                d = match d {
                    4 => 6,
                    6 => 4,
                    0 => 3,
                    1 => 2,
                    3 => 0,
                    2 => 1,
                    _ => d,
                };
            }
            d
        }
        fn side(d: PortSide, transpose: bool, x: bool, y: bool) -> PortSide {
            let raw = match d {
                PortSide::Top => 4,
                PortSide::Right => 5,
                PortSide::Bottom => 6,
                PortSide::Left => 7,
            };
            match direction(raw, transpose, x, y) {
                4 => PortSide::Top,
                5 => PortSide::Right,
                6 => PortSide::Bottom,
                _ => PortSide::Left,
            }
        }
        for &id in &self.members {
            let mut node = self.nodes[id].write();
            let mut point = node.point();
            if transpose {
                std::mem::swap(&mut point.x, &mut point.y);
            }
            if x {
                point.x = -point.x;
            }
            if y {
                point.y = -point.y;
            }
            if let Some(shared) = &node.point_ref {
                shared.set(point);
            }
            for metadata in node.owners.values_mut() {
                let mut bits = 0;
                for d in 0..=8 {
                    if metadata.directions & (1 << d) != 0 {
                        bits |= 1 << direction(d, transpose, x, y);
                    }
                }
                metadata.directions = bits;
            }
        }
        self.occupied.clear();
        self.nan_occupied.clear();
        for &id in &self.members {
            let point = self.nodes[id].point();
            if point.x.is_nan() || point.y.is_nan() {
                self.nan_occupied.push((point, id));
            } else {
                self.occupied
                    .insert(super::go_point_map_key(point), occupant_entry(point, id));
            }
        }
        for entries in self.ports.values_mut() {
            for (id, port) in &mut entries.records {
                port.point = self.nodes[*id].point();
                port.side = side(port.side, transpose, x, y);
                port.direction = side(port.direction, transpose, x, y);
            }
        }
    }
    pub(super) fn merge_hierarchy_ports(
        &mut self,
        other: &PortBuild,
        graph_node_count: usize,
        budget: super::ovg_intersections::Budget<'_, '_>,
    ) -> Result<(), ()> {
        use super::ovg_intersections::{check, step};
        check(budget)?;
        let mut imported = BTreeMap::new();
        for (&owner, ports) in other.ports.iter() {
            if let Some(b) = budget {
                b.reserve_work(ports.len() as u64 + 1)?;
            }
            let mut canonical_ports = PortList::with_capacity(ports.len());
            for &(foreign, mut port) in ports.iter() {
                let local = *imported
                    .entry(foreign)
                    .or_insert_with(|| self.import_node(other, foreign));
                let canonical = self.add_existing_node(local, budget)?;
                if !self.nodes[canonical].same_allocation(&self.nodes[local]) {
                    let owners = other.nodes[foreign]
                        .read()
                        .owners
                        .keys()
                        .copied()
                        .collect::<Vec<_>>();
                    for owner in owners {
                        let Some(meta) = other.nodes[foreign].read().owners.get(&owner).copied()
                        else {
                            continue;
                        };
                        step(budget)?;
                        let mut node = self.nodes[canonical].write();
                        let target = node.owners.get_or_insert(owner);
                        target.directions |= meta.directions;
                        target.center |= meta.center;
                    }
                }
                port.point = self.nodes[canonical].point();
                canonical_ports.push((canonical, port), &self.nodes[canonical]);
            }
            if !self.ports.contains_key(&owner) {
                self.port_order.push(owner);
            }
            self.ports.insert(owner, canonical_ports);
        }
        if let Some(b) = budget {
            b.reserve_work(graph_node_count as u64)?;
        }
        check(budget)
    }
    pub(super) fn merge_hierarchy_nonports(
        &mut self,
        other: &PortBuild,
        budget: super::ovg_intersections::Budget<'_, '_>,
    ) -> Result<(), ()> {
        use super::ovg_intersections::{check, step};
        check(budget)?;
        let mut imported = BTreeMap::new();
        for &foreign in &other.members {
            step(budget)?;
            if !other.nodes[foreign].read().owners.is_empty() {
                continue;
            }
            let local = *imported
                .entry(foreign)
                .or_insert_with(|| self.import_node(other, foreign));
            self.add_existing_node(local, budget)?;
        }
        check(budget)
    }

    pub(super) fn add_ports(
        &mut self,
        graph: &ArenaGraph,
        nodes: &[NodeId],
        budget: Option<&OvgBuildBudget<'_>>,
    ) -> Result<(), ()> {
        if let Some(budget) = budget {
            budget.check()?;
        }
        let mut skip_pathing = BTreeSet::new();
        for children in graph.containers.values() {
            if let Some(budget) = budget {
                budget.reserve_work(1)?;
            }
            let mut connected = false;
            for &child in &children {
                if let Some(budget) = budget {
                    budget.reserve_work(1)?;
                }
                if !graph.nodes[child.0 as usize].edges.is_empty() {
                    connected = true;
                    break;
                }
            }
            if !connected {
                for child in children {
                    if let Some(budget) = budget {
                        budget.reserve_work(1)?;
                    }
                    skip_pathing.insert(child);
                }
            }
        }
        for &owner in nodes {
            if let Some(budget) = budget {
                budget.reserve_work(1)?;
            }
            if skip_pathing.contains(&owner) {
                continue;
            }
            if !self.ports.contains_key(&owner) {
                self.port_order.push(owner);
            }
            self.ports.insert(owner, PortList::with_capacity(12));
            let node = &graph.nodes[owner.0 as usize];
            // Shape percentages are prepared once. Each absolute point below
            // reads the current owner position after its own guarded step.
            let size = node.rect.size;
            let relative = snap_points_with_table_column_count(
                Rect {
                    origin: Point { x: 0.0, y: 0.0 },
                    size,
                },
                node.shape,
                node.table_column_count,
            );
            for side in [
                PortSide::Top,
                PortSide::Left,
                PortSide::Bottom,
                PortSide::Right,
            ] {
                if let Some(budget) = budget {
                    budget.reserve_work(1)?;
                }
                for snap in relative.iter().copied().filter(|point| point.side == side) {
                    if let Some(budget) = budget {
                        budget.reserve_work(1)?;
                    }
                    let rect = Rect {
                        origin: graph.position(owner).expect(
                            "runtime error: invalid memory address or nil pointer dereference",
                        ),
                        size,
                    };
                    let mut port = snap.at(rect);
                    let index = self.add_point(port.point, budget)?;
                    // Center metadata is published in the separate source
                    // CenterPortIndices loop, after every side is complete.
                    port.point = self.nodes[index].point();
                    port.is_center = false;
                    self.ports
                        .get_mut(&owner)
                        .unwrap()
                        .push((index, port), &self.nodes[index]);
                    let direction = match side {
                        PortSide::Top => 4,
                        PortSide::Left => 7,
                        PortSide::Bottom => 6,
                        PortSide::Right => 5,
                    };
                    self.nodes[index]
                        .write()
                        .owners
                        .get_or_insert(owner)
                        .directions |= 1 << direction;
                }
            }
            for (index, _) in relative
                .iter()
                .enumerate()
                .filter(|(_, point)| point.is_center)
            {
                if let Some(budget) = budget {
                    budget.reserve_work(1)?;
                }
                let node_index = self.ports[&owner][index].0;
                self.nodes[node_index]
                    .write()
                    .owners
                    .get_mut(&owner)
                    .unwrap()
                    .center = true;
            }
        }
        if let Some(budget) = budget {
            budget.check()?;
        }
        Ok(())
    }

    // Helper entry for callers supplying already materialized ports. Normal
    // routing passes its incremental builder directly instead.
    pub(super) fn import_ports(
        &mut self,
        ports: &[(NodeId, Port)],
        budget: Option<&OvgBuildBudget<'_>>,
    ) -> Result<(), ()> {
        for &(owner, mut port) in ports {
            let index = self.add_point(port.point, budget)?;
            port.point = self.nodes[index].point();
            if !self.ports.contains_key(&owner) {
                self.port_order.push(owner);
            }
            self.ports
                .get_or_default(owner)
                .push((index, port), &self.nodes[index]);
            let direction = match port.side {
                PortSide::Top => 4,
                PortSide::Right => 5,
                PortSide::Bottom => 6,
                PortSide::Left => 7,
            };
            let mut node = self.nodes[index].write();
            let meta = node.owners.get_or_insert(owner);
            meta.directions |= 1 << direction;
            meta.center |= port.is_center;
        }
        Ok(())
    }

    pub(super) fn ports_by_orientation(
        &self,
        owner: NodeId,
        side: PortSide,
        budget: Option<&OvgBuildBudget<'_>>,
    ) -> Result<Vec<usize>, ()> {
        let direction = match side {
            PortSide::Top => 4,
            PortSide::Right => 5,
            PortSide::Bottom => 6,
            PortSide::Left => 7,
        };
        let mut seen = BTreeSet::new();
        let mut result = Vec::new();
        for &(index, _) in self.ports.get(&owner).into_iter().flatten() {
            if let Some(b) = budget {
                b.reserve_work(1)?;
            }
            if !seen.insert(index) {
                continue;
            }
            if self.port_directions(index, owner) & (1 << direction) != 0 {
                result.push(index);
            }
        }
        if let Some(b) = budget {
            b.check()?;
        }
        Ok(result)
    }

    pub(super) fn node_ports(&self) -> BTreeMap<NodeId, Vec<Port>> {
        self.ports
            .iter()
            .map(|(&owner, entries)| {
                (
                    owner,
                    entries
                        .iter()
                        .map(|&(index, mut port)| {
                            port.is_center = self.nodes[index].read().owners[&owner].center;
                            port
                        })
                        .collect(),
                )
            })
            .collect()
    }

    #[cfg(feature = "diagnostic-traces")]
    pub(super) fn observe(&self, graph: &ArenaGraph) -> serde_json::Value {
        use serde_json::json;
        let nodes = self.member_nodes().map(|node| {
            let node = node.read();
            let owners = node.owners.iter().map(|(owner, meta)| (graph.nodes[owner.0 as usize].tala_id.to_string(), json!({"directions":meta.directions,"center":meta.center}))).collect::<serde_json::Map<_,_>>();
            json!({"point":[format!("{:016x}",node.point().x.to_bits()),format!("{:016x}",node.point().y.to_bits())],"owners":owners,"index":0})
        }).collect::<Vec<_>>();
        let members = self
            .members
            .iter()
            .enumerate()
            .map(|(i, &node)| (node, i))
            .collect::<BTreeMap<_, _>>();
        let ports = self
            .ports
            .iter()
            .map(|(owner, entries)| {
                (
                    graph.nodes[owner.0 as usize].tala_id.to_string(),
                    json!(
                        entries
                            .iter()
                            .map(|(index, _)| members[index])
                            .collect::<Vec<_>>()
                    ),
                )
            })
            .collect::<serde_json::Map<_, _>>();
        json!({"nodes":nodes,"ports":ports})
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::internal::limits::{Context, ContextError};
    use std::cell::Cell;

    struct Caller {
        polls: Cell<u64>,
        cancel_at: u64,
        mutate_at: u64,
        point: Option<crate::internal::owner_points::SharedOwnerPoint>,
    }
    impl Context for Caller {
        fn done(&self) -> Option<bool> {
            None
        }
        fn error(&self) -> Option<ContextError> {
            let poll = self.polls.get() + 1;
            self.polls.set(poll);
            if poll == self.mutate_at
                && let Some(point) = &self.point
            {
                point.set(Point {
                    x: 9876.5,
                    y: -1234.5,
                });
            }
            (self.cancel_at != 0 && poll >= self.cancel_at).then_some(ContextError::Cancelled)
        }
    }
    fn graph(size: f64, separated: bool) -> ArenaGraph {
        let value = serde_json::json!({"nodes":[
            {"id":1,"width":size,"height":size,"topLeft":{"x":0,"y":0}},
            {"id":2,"width":size,"height":size,"topLeft":{"x":if separated {200.0} else {0.0},"y":0}}
        ],"edges":[{"id":11,"from":1,"to":2,"points":[]}],"containers":{"0":[1,2]}});
        let input =
            crate::graphjson::graphjson_to_graph(&serde_json::to_vec(&value).unwrap()).unwrap();
        ArenaGraph::from_input(&input)
    }

    #[test]
    fn sparse_subgraph_port_owners_retain_live_nodes_when_lower_owners_are_added() {
        // ovg.go addPorts assigns each owner's slice independently; adding
        // other owners cannot detach the earlier OVGNode.Point allocations.
        let nodes = (0..65)
            .map(|i| {
                serde_json::json!({
                    "id": i + 1, "width": 100, "height": 100,
                    "topLeft": {"x": i * 200, "y": 0}
                })
            })
            .collect::<Vec<_>>();
        let input = serde_json::json!({"nodes": nodes, "edges": []});
        let graph = ArenaGraph::from_input(
            &crate::graphjson::graphjson_to_graph(&serde_json::to_vec(&input).unwrap()).unwrap(),
        );
        let mut build = PortBuild::default();
        let owner = NodeId(63);
        build.add_ports(&graph, &[owner], None).unwrap();
        assert!(build.ports.get(&NodeId(0)).is_none());
        let original = build.nodes[build.ports[&owner][0].0].clone();

        let remaining = (0..63).chain([64]).map(NodeId).collect::<Vec<_>>();
        build.add_ports(&graph, &remaining, None).unwrap();
        assert_eq!(
            build.ports.keys().copied().collect::<Vec<_>>(),
            (0..65).map(NodeId).collect::<Vec<_>>()
        );
        assert!(build.ports.values().all(|ports| ports.len() == 12));
        let retained = &build.nodes[build.ports[&owner][0].0];
        assert!(retained.same_allocation(&original));
        let next = Point { x: -20.0, y: 17.0 };
        original.read().point_ref.as_ref().unwrap().set(next);
        assert_eq!(retained.point(), next);

        // ovg_resource.go passesThroughAllowingPorts dereferences the retained
        // OVGNode.Point, including replacement of the point allocation itself.
        // An endpoint inside the box is admitted only by this live port.
        let inside = Point {
            x: graph.position(owner).unwrap().x + 20.0,
            y: 17.0,
        };
        original.write().point_ref =
            Some(crate::internal::owner_points::SharedOwnerPoint::new(inside));
        assert!(
            !build
                .passes_through_allowing_ports(
                    &graph,
                    owner,
                    inside,
                    Point {
                        x: inside.x,
                        y: -20.0
                    },
                    4,
                    None,
                )
                .unwrap()
        );
    }
    #[test]
    fn center_publication_precedes_member_insertion_and_exit_check() {
        for (at, count) in [(4, 0), (5, 0), (6, 1)] {
            let graph = graph(40.0, true);
            let context = Caller {
                polls: Cell::new(0),
                cancel_at: at,
                mutate_at: 0,
                point: None,
            };
            let budget = OvgBuildBudget::with_context(&context);
            let mut ports = PortBuild::default();
            assert!(
                ports
                    .connect_centers(&graph, &[NodeId(0)], Some(&budget))
                    .is_err()
            );
            let center = ports.centers[&NodeId(0)];
            assert_eq!(ports.nodes[center].point(), Point { x: 20.0, y: 20.0 });
            assert!(ports.nodes[center].read().is_node_center());
            assert_eq!(ports.members.len(), count);
            assert_eq!(ports.members_allocated, count != 0);
            assert_eq!(ports.occupied.len(), count);
        }
    }
    #[test]
    fn center_argument_is_captured_before_candidate_callback() {
        let mut graph = graph(40.0, true);
        graph.materialize_owner_node_point(NodeId(0)).unwrap();
        let point = graph
            .owner_node_point(NodeId(0))
            .unwrap()
            .shared_point()
            .unwrap();
        let context = Caller {
            polls: Cell::new(0),
            cancel_at: 0,
            mutate_at: 3,
            point: Some(point),
        };
        let budget = OvgBuildBudget::with_context(&context);
        let mut ports = PortBuild::default();
        ports
            .connect_centers(&graph, &[NodeId(0), NodeId(0)], Some(&budget))
            .unwrap();
        assert_eq!(
            ports.nodes[ports.members[0]].read().point(),
            Point { x: 20.0, y: 20.0 }
        );
        assert_eq!(
            ports.nodes[ports.members[1]].read().point(),
            Point {
                x: 9896.5,
                y: -1214.5
            }
        );
        assert_eq!(ports.centers[&NodeId(0)], ports.members[1]);
    }
    #[test]
    fn isolated_removal_publishes_only_after_the_scan_and_keeps_maps() {
        for at_end in [false, true] {
            let graph = graph(40.0, true);
            let mut ports = PortBuild::default();
            ports
                .add_ports(&graph, &graph.graph_node_order(), None)
                .unwrap();
            let before = ports.members.clone();
            let context = Caller {
                polls: Cell::new(0),
                cancel_at: if at_end { before.len() as u64 + 2 } else { 2 },
                mutate_at: 0,
                point: None,
            };
            let budget = OvgBuildBudget::with_context(&context);
            assert!(ports.remove_isolated(Some(&budget)).is_err());
            assert_eq!(ports.members.len(), if at_end { 0 } else { before.len() });
            assert!(ports.members_allocated);
            assert_eq!(ports.nodes.len(), before.len());
            assert_eq!(ports.occupied.len(), before.len());
            assert_eq!(
                ports.ports.values().map(|ports| ports.len()).sum::<usize>(),
                before.len()
            );
        }
    }
    #[test]
    fn nan_memberships_share_one_object_and_centers_overwrite_occupancy() {
        let mut ports = PortBuild::default();
        let node = ports
            .add_point(
                Point {
                    x: f64::NAN,
                    y: 0.0,
                },
                None,
            )
            .unwrap();
        ports.add_existing_node(node, None).unwrap();
        assert_eq!(ports.nodes.len(), 1);
        assert_eq!(ports.members, [node, node]);
        assert_eq!(ports.nan_occupied.len(), 2);
        ports.connect(node, node, None).unwrap();
        ports.remove_isolated(None).unwrap();
        assert_eq!(ports.members, [node, node]);
        assert_eq!(ports.nodes[node].read().edges, [0, 0]);
        let graph = graph(0.0, false);
        let mut ports = PortBuild::default();
        ports
            .add_ports(&graph, &graph.graph_node_order(), None)
            .unwrap();
        let original = ports.members[0];
        ports.connect_centers(&graph, &[NodeId(0)], None).unwrap();
        let center = ports.centers[&NodeId(0)];
        assert_ne!(center, original);
        assert_eq!(
            ports.add_point(Point { x: 0.0, y: 0.0 }, None).unwrap(),
            center
        );
        assert!(ports.nodes[center].read().owners.is_empty());
        assert!(!ports.nodes[original].read().owners.is_empty());
    }
    #[test]
    fn nonport_owner_maps_stay_nil_through_reads_and_hierarchy_transforms() {
        // routing/ovg_node.go NewOVGNode and ovg_hierarchy.go's direction
        // transform leave the nil portOwnersByNode map unallocated.
        let mut ports = PortBuild::default();
        let node = ports.add_point(Point { x: 10.0, y: 20.0 }, None).unwrap();
        assert_eq!(ports.owner_directions(node).count(), 0);
        assert_eq!(ports.port_directions(node, NodeId(0)), 0);
        assert!(ports.nodes[node].read().owners.0.is_none());
        ports.transform_hierarchy(true, true, false);
        assert_eq!(ports.nodes[node].point(), Point { x: -20.0, y: 10.0 });
        assert!(ports.nodes[node].read().owners.0.is_none());
        ports.nodes[node].write().owners.insert(
            NodeId(0),
            Metadata {
                directions: 1 << 4,
                center: true,
            },
        );
        let shared = ports.nodes[node].clone();
        ports.transform_hierarchy(true, false, false);
        assert_eq!(shared.read().owners[&NodeId(0)].directions, 1 << 7);
        assert!(shared.read().owners[&NodeId(0)].center);
        assert!(ports.nodes[node].same_allocation(&shared));
    }
    #[test]
    fn coincident_ports_share_one_allocation_and_keep_all_owner_roles() {
        // Pinned addPorts observer: work=68, nodes=1, Err polls=72.
        let graph = graph(0.0, false);
        let context = Caller {
            polls: Cell::new(0),
            cancel_at: 0,
            mutate_at: 0,
            point: None,
        };
        let budget = OvgBuildBudget::with_context(&context);
        budget.check().unwrap();
        let mut ports = PortBuild::default();
        ports
            .add_ports(&graph, &graph.graph_node_order(), Some(&budget))
            .unwrap();
        assert_eq!(budget.observed_resources(), [68, 1, 0, 0]);
        assert_eq!(context.polls.get(), 72);
        assert_eq!(ports.nodes.len(), 1);
        for owner in [NodeId(0), NodeId(1)] {
            let metadata = ports.nodes[0].read().owners[&owner];
            assert_eq!(metadata.directions, 240);
            assert!(metadata.center);
            assert_eq!(ports.ports[&owner].len(), 12);
            assert!(ports.ports[&owner].iter().all(|(index, _)| *index == 0));
        }
    }
    #[test]
    fn coincident_ports_keep_every_owner_when_added_in_reverse_order() {
        // ovg_node.go addPortMetadata merges roles on the same OVGNode;
        // ovg_hierarchy.go transforms that shared object's complete owner map.
        let nodes = (0..65)
            .map(|i| {
                serde_json::json!({
                    "id": i + 1, "width": 0, "height": 0,
                    "topLeft": {"x": 0, "y": 0}
                })
            })
            .collect::<Vec<_>>();
        let graph = ArenaGraph::from_input(
            &crate::graphjson::graphjson_to_graph(
                &serde_json::to_vec(&serde_json::json!({"nodes": nodes, "edges": []})).unwrap(),
            )
            .unwrap(),
        );
        let mut build = PortBuild::default();
        for owner in (0..65).rev().map(NodeId) {
            build.add_ports(&graph, &[owner], None).unwrap();
        }
        assert_eq!(build.nodes.len(), 1);
        let shared = build.nodes[0].clone();
        build.transform_hierarchy(true, true, false);
        assert!(shared.same_allocation(&build.nodes[0]));
        let node = shared.read();
        assert_eq!(
            node.owners.keys().copied().collect::<Vec<_>>(),
            (0..65).map(NodeId).collect::<Vec<_>>()
        );
        for owner in (0..65).map(NodeId) {
            assert_eq!(node.owners[&owner].directions, 240);
            assert!(node.owners[&owner].center);
            assert!(build.ports[&owner].iter().all(|(slot, _)| *slot == 0));
        }
        // Freeze keeps repeated identities, present empty lists and sparse
        // owners distinct from missing owners, as Go's Ports map does.
        let sparse = NodeId(100_000);
        build.ports.insert(sparse, build.ports[&NodeId(64)].clone());
        build.ports.insert(NodeId(65), PortList::default());
        let frozen = std::mem::take(&mut build.ports).into_indices(&[7]);
        assert_eq!(
            frozen.keys().copied().collect::<Vec<_>>(),
            (0..66).map(NodeId).chain([sparse]).collect::<Vec<_>>()
        );
        for owner in (0..65).map(NodeId).chain([sparse]) {
            assert_eq!(frozen[&owner], vec![7; 12]);
        }
        assert_eq!(frozen.get(&NodeId(65)), Some(&Vec::new()));
        assert!(frozen.get(&NodeId(66)).is_none());
    }

    #[test]
    fn port_origin_is_read_before_add_point_check_and_again_for_the_next_port() {
        let mut graph = graph(40.0, true);
        graph.materialize_owner_node_point(NodeId(0)).unwrap();
        let point = graph
            .owner_node_point(NodeId(0))
            .unwrap()
            .shared_point()
            .unwrap();
        let context = Caller {
            polls: Cell::new(0),
            cancel_at: 0,
            mutate_at: 8,
            point: Some(point),
        };
        let budget = OvgBuildBudget::with_context(&context);
        budget.check().unwrap();
        let mut ports = PortBuild::default();
        ports
            .add_ports(&graph, &graph.graph_node_order(), Some(&budget))
            .unwrap();
        assert_eq!(ports.nodes[0].point(), Point { x: 10.0, y: 0.0 });
        assert_eq!(
            ports.nodes[1].point(),
            Point {
                x: 9896.5,
                y: -1234.5
            }
        );
        assert_eq!(
            graph.position(NodeId(0)),
            Some(Point {
                x: 9876.5,
                y: -1234.5
            })
        );
    }
    #[test]
    fn rejected_first_port_preserves_the_precise_partial_work_and_empty_header() {
        for (at, work) in [(8, 5), (9, 6)] {
            let graph = graph(40.0, true);
            let context = Caller {
                polls: Cell::new(0),
                cancel_at: at,
                mutate_at: 0,
                point: None,
            };
            let budget = OvgBuildBudget::with_context(&context);
            budget.check().unwrap();
            let mut ports = PortBuild::default();
            assert!(
                ports
                    .add_ports(&graph, &graph.graph_node_order(), Some(&budget))
                    .is_err()
            );
            assert_eq!(budget.observed_resources(), [work, 0, 0, 0]);
            assert_eq!(context.polls.get(), at);
            assert!(ports.nodes.is_empty());
            assert!(ports.ports[&NodeId(0)].is_empty());
            assert!(!ports.ports.contains_key(&NodeId(1)));
            assert_eq!(
                budget.take_context_error().unwrap().to_string(),
                "EdgeRouting: context canceled"
            );
        }
    }
}
