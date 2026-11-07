// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Visibility graph construction and shortest-path search.
//!
//! Nodes represent ports, corners, tunnels, and orthogonal intersections.
//! Coordinate identity, insertion order, and equal-cost predecessor replacement
//! are behavioral state and must remain stable.

use super::LocalRouteSearchWork;
use super::priority_queue::{Handle, PriorityQueue};
use super::route_search_guard::OvgBuildBudget;
use super::{ArenaGraph, NodeId, Orientation, Point, Port, PortSide, Rect};
use crate::Size;
use std::collections::{BTreeMap, BTreeSet};

/// Source search evaluates both ancestry walks before the fixed-overlap
/// exception. Keep successful first walks from short-circuiting work/cancel.
fn route_container_is_admissible_guarded(
    graph: &ArenaGraph,
    source: Option<NodeId>,
    target: Option<NodeId>,
    candidate: Option<NodeId>,
    fixed_overlap: bool,
    budget: Option<&LocalRouteSearchWork>,
) -> Option<bool> {
    let Some(candidate) = candidate else {
        return Some(true);
    };
    let source_inside =
        super::route_search_guard::is_descendant_of(graph, source, Some(candidate), budget)?;
    let target_inside =
        super::route_search_guard::is_descendant_of(graph, target, Some(candidate), budget)?;
    Some(source_inside || target_inside || fixed_overlap)
}

/// Ordinary turns visit ideal axes in order and stop at the first match.
fn ideal_turn_multiplier_guarded(
    point: Point,
    axes: &[(bool, f64)],
    budget: Option<&LocalRouteSearchWork>,
) -> Option<f64> {
    for &(vertical, position) in axes {
        if let Some(budget) = budget {
            budget.step().ok()?;
        }
        let coordinate = if vertical { point.x } else { point.y };
        if (coordinate - position).abs() <= 4.0 {
            return Some(0.98);
        }
    }
    Some(1.0)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Axis {
    None,
    Horizontal,
    Vertical,
}

impl Axis {
    fn index(self) -> usize {
        match self {
            Self::Horizontal => 0,
            Self::Vertical => 1,
            Self::None => panic!("geometric Axis::None is not a recovered search state"),
        }
    }
}

fn search_axis(first: Point, second: Point) -> Axis {
    if first.y == second.y {
        Axis::Horizontal
    } else {
        // Recovered search has only horizontal and vertical carriers. Every
        // non-horizontal hop, including a diagonal center connector imported
        // with geometric Axis::None, occupies the vertical carrier.
        Axis::Vertical
    }
}

/// The three node-indexed arrays initialized at the start of Go search.
/// Preparing them before port/label setup preserves allocation and work-limit
/// boundaries while allowing the caller to derive source-shaped options.
pub(super) struct PreparedRouteSearch {
    ancestry: Option<(bool, bool)>,
    arrowhead_nodes: Vec<NodeId>,
    blocked_ports_prepared: bool,
    symmetry_prepared: bool,
    fixed_overlaps: Option<std::sync::Arc<BTreeSet<NodeId>>>,
    blocked_source: BTreeSet<(u64, u64)>,
    blocked_target: BTreeSet<(u64, u64)>,
    // Per-node search state is sized by the OVG for every route search.
    slots: Vec<SearchSlot>,
}

/// One search node's directional distances, predecessor hops and queue
/// entries. A hop reads and updates them together, so they share one
/// record. Previous hops keep only the node, as Go's vertical/horizontal
/// hop arrays do. One-based indices and queue handles give their Options a
/// zero niche, keeping a record to 32 bytes.
#[derive(Clone, Copy, Debug)]
struct SearchSlot {
    distances: [f64; 2],
    previous: [Option<std::num::NonZeroU32>; 2],
    entries: [Option<Handle>; 2],
}

/// The state of a node that no search has reached.
const INITIAL_SEARCH_SLOT: SearchSlot = SearchSlot {
    distances: [f64::INFINITY; 2],
    previous: [None; 2],
    entries: [None; 2],
};

fn predecessor(node: usize) -> std::num::NonZeroU32 {
    std::num::NonZeroU32::new(compact_search_index(node).wrapping_add(1))
        .expect("route search predecessor")
}

fn predecessor_index(node: std::num::NonZeroU32) -> usize {
    (node.get() - 1) as usize
}

impl Drop for PreparedRouteSearch {
    fn drop(&mut self) {
        // A search moves the slots out first; slots still here were never
        // searched, so every slot keeps its initial value.
        return_search_slots(std::mem::take(&mut self.slots));
    }
}

/// Search storage kept between this thread's route searches, so a search
/// neither reallocates nor reinitializes whole OVG-length arrays. Whenever
/// slots are pooled, every slot is `INITIAL_SEARCH_SLOT`.
struct SearchPool {
    slots: Option<Vec<SearchSlot>>,
    queue: Option<PriorityQueue<PackedQueueState>>,
    touched: Vec<usize>,
}

thread_local! {
    static SEARCH_POOL: std::cell::RefCell<SearchPool> = const {
        std::cell::RefCell::new(SearchPool {
            slots: None,
            queue: None,
            touched: Vec::new(),
        })
    };
}

/// `node_count` initial slots, as `vec![INITIAL_SEARCH_SLOT; node_count]`.
/// Pooled slots are all initial, so resizing either way yields exactly that.
fn take_search_slots(node_count: usize) -> Vec<SearchSlot> {
    let mut slots = SEARCH_POOL
        .try_with(|pool| pool.borrow_mut().slots.take())
        .ok()
        .flatten()
        .unwrap_or_default();
    slots.resize(node_count, INITIAL_SEARCH_SLOT);
    slots
}

/// Pool clean slots for the next search; a panicking search frees them.
fn return_search_slots(slots: Vec<SearchSlot>) {
    if slots.capacity() == 0 || std::thread::panicking() {
        return;
    }
    let _ = SEARCH_POOL.try_with(|pool| {
        let mut pool = pool.borrow_mut();
        if pool
            .slots
            .as_ref()
            .is_none_or(|pooled| pooled.capacity() < slots.capacity())
        {
            pool.slots = Some(slots);
        }
    });
}

/// One search's slots and queue. The first write to a slot records its
/// search index in `touched` (every write starts with a finite distance, so
/// a slot whose distances are both still infinite is unwritten); dropping
/// the state restores exactly those slots and pools the storage again.
struct SearchState {
    slots: Vec<SearchSlot>,
    touched: Vec<usize>,
    queue: PriorityQueue<PackedQueueState>,
}

impl SearchState {
    fn new(slots: Vec<SearchSlot>) -> Self {
        let (queue, touched) = SEARCH_POOL
            .try_with(|pool| {
                let mut pool = pool.borrow_mut();
                (pool.queue.take(), std::mem::take(&mut pool.touched))
            })
            .unwrap_or_default();
        // A reset queue is a new queue whose entry storage is retained.
        let queue = queue.map_or_else(PriorityQueue::new, |mut queue| {
            queue.reset();
            queue
        });
        Self {
            slots,
            touched,
            queue,
        }
    }
}

impl Drop for SearchState {
    fn drop(&mut self) {
        if std::thread::panicking() {
            return;
        }
        for &index in &self.touched {
            self.slots[index] = INITIAL_SEARCH_SLOT;
        }
        self.touched.clear();
        let mut queue = std::mem::replace(&mut self.queue, PriorityQueue::new());
        queue.reset();
        let touched = std::mem::take(&mut self.touched);
        let _ = SEARCH_POOL.try_with(|pool| {
            let mut pool = pool.borrow_mut();
            if pool.queue.is_none() {
                pool.queue = Some(queue);
                pool.touched = touched;
            }
        });
        return_search_slots(std::mem::take(&mut self.slots));
    }
}

/// A search-node index in the compact form `PreparedRouteSearch` stores.
/// Indices address an in-memory OVG, so they always fit.
fn compact_search_index(index: usize) -> u32 {
    u32::try_from(index).expect("route search node index")
}

impl PreparedRouteSearch {
    pub(super) fn set_fixed_overlaps(&mut self, overlaps: std::sync::Arc<BTreeSet<NodeId>>) {
        self.fixed_overlaps = Some(overlaps);
    }

    pub(super) fn mark_symmetry_prepared(&mut self) {
        self.symmetry_prepared = true;
    }

    /// Source non-table blocked-port setup: reserve the OVG inventory product
    /// before finding coincident physical ports. Ancestor overlaps are exempt.
    pub(super) fn prepare_blocked_ports(
        &mut self,
        graph: &ArenaGraph,
        source: NodeId,
        target: NodeId,
        boxes: &BTreeMap<NodeId, Rect>,
        source_count: usize,
        target_count: usize,
        has_table_column: bool,
        budget: Option<&LocalRouteSearchWork>,
    ) -> Option<()> {
        // Table hydration retains its existing path until its guarded port
        // walks are translated; do not treat it as qualified non-table setup.
        if has_table_column {
            return Some(());
        }
        if let Some(budget) = budget {
            budget
                .reserve_product(source_count as u64, target_count as u64)
                .ok()?;
        }
        let mut blocked = BTreeSet::new();
        let (source_inside, target_inside) = self.ancestry?;
        if !source_inside && !target_inside {
            let source_node = &graph.nodes[source.0 as usize];
            let target_node = &graph.nodes[target.0 as usize];
            let source_box = boxes
                .get(&source)
                .copied()
                .or_else(|| super::node_rect(graph, source))?;
            let target_box = boxes
                .get(&target)
                .copied()
                .or_else(|| super::node_rect(graph, target))?;
            let from = super::ports_with_table_column_count(
                source_box,
                source_node.shape,
                source_node.table_column_count,
            );
            let to = super::ports_with_table_column_count(
                target_box,
                target_node.shape,
                target_node.table_column_count,
            );
            for port in from {
                if to.iter().any(|other| other.point == port.point) {
                    blocked.insert(super::go_point_map_key(port.point));
                }
            }
        }
        self.blocked_source = blocked.clone();
        self.blocked_target = blocked;
        self.blocked_ports_prepared = true;
        Some(())
    }

    /// Table search blocks every raw OVG port except the requested row's
    /// selected ports. These maps also constrain intermediate port visits.
    pub(super) fn prepare_table_blocked_ports(
        &mut self,
        source_ports: &[Port],
        target_ports: &[Port],
        allowed_source: Option<&[Port]>,
        allowed_target: Option<&[Port]>,
        budget: Option<&LocalRouteSearchWork>,
    ) -> Option<()> {
        self.blocked_source.clear();
        self.blocked_target.clear();
        for (raw, allowed, blocked) in [
            (source_ports, allowed_source, &mut self.blocked_source),
            (target_ports, allowed_target, &mut self.blocked_target),
        ] {
            if let Some(allowed) = allowed {
                for port in raw {
                    if let Some(guard) = budget {
                        guard.step().ok()?;
                    }
                    if !allowed.iter().any(|selected| selected.point == port.point) {
                        blocked.insert(super::go_point_map_key(port.point));
                    }
                }
            }
        }
        self.blocked_ports_prepared = true;
        Some(())
    }

    pub(super) fn prepare_arrowhead_nodes(
        &mut self,
        graph: &ArenaGraph,
        edge_index: usize,
        budget: Option<&LocalRouteSearchWork>,
    ) -> Option<()> {
        let edge = &graph.edge_records[edge_index];
        self.arrowhead_nodes.clear();
        if edge.source_arrowhead_label.is_none() && edge.target_arrowhead_label.is_none() {
            return Some(());
        }
        self.arrowhead_nodes =
            super::route_search_guard::filter_edge_ancestors(graph, edge_index, budget)?;
        Some(())
    }

    pub(super) fn prepare_ancestry(
        &mut self,
        graph: &ArenaGraph,
        source: NodeId,
        target: NodeId,
        budget: Option<&LocalRouteSearchWork>,
    ) -> Option<()> {
        let source_inside =
            super::route_search_guard::is_descendant_of(graph, Some(source), Some(target), budget)?;
        let target_inside =
            super::route_search_guard::is_descendant_of(graph, Some(target), Some(source), budget)?;
        self.ancestry = Some((source_inside, target_inside));
        Some(())
    }
}

/// One node's per-hop search fields, packed so a hop reads one record
/// instead of five node-indexed vectors. Owner-set flags only record
/// emptiness; membership is still read from the sets themselves. The
/// search index is stored as u32 (see `compact_search_index`), so a record
/// is 16 bytes.
#[derive(Clone, Copy, Debug)]
struct SearchNode {
    search_index: u32,
    container: Option<NodeId>,
    flags: u8,
}

impl SearchNode {
    #[inline]
    fn context(self) -> usize {
        self.search_index as usize
    }
}

const SEARCH_NODE_PORT_OWNED: u8 = 1;
const SEARCH_NODE_NEAR_PORT: u8 = 2;
const SEARCH_NODE_CENTER: u8 = 4;

#[derive(Clone, Copy, Debug)]
struct QueueState {
    node: usize,
    axis: Axis,
}

/// QueueState as the queue stores it in one u32: the node index (an
/// in-memory OVG index, below 2^30) and the axis code in the top two bits.
#[derive(Clone, Copy, Debug)]
struct PackedQueueState(u32);

impl PackedQueueState {
    fn pack(state: QueueState) -> Self {
        let node = compact_search_index(state.node);
        assert!(node <= ADJACENCY_NEIGHBOR_MASK, "route search node index");
        let axis = match state.axis {
            Axis::None => 0,
            Axis::Horizontal => 1,
            Axis::Vertical => 2,
        };
        Self(node | axis << ADJACENCY_NEIGHBOR_BITS)
    }

    fn unpack(self) -> QueueState {
        QueueState {
            node: (self.0 & ADJACENCY_NEIGHBOR_MASK) as usize,
            axis: match self.0 >> ADJACENCY_NEIGHBOR_BITS {
                0 => Axis::None,
                1 => Axis::Horizontal,
                _ => Axis::Vertical,
            },
        }
    }
}

fn queue_push(
    queue: &mut PriorityQueue<PackedQueueState>,
    budget: Option<&LocalRouteSearchWork>,
    priority: f64,
    value: QueueState,
) -> Option<Handle> {
    let value = PackedQueueState::pack(value);
    match budget {
        Some(budget) => queue
            .push_guarded(priority, value, |units| budget.reserve(units))
            .ok(),
        None => Some(queue.push(priority, value)),
    }
}

fn queue_pop(
    queue: &mut PriorityQueue<PackedQueueState>,
    budget: Option<&LocalRouteSearchWork>,
) -> Option<(QueueState, f64)> {
    if queue.len() != 0
        && let Some(budget) = budget
    {
        budget.step().ok()?;
    }
    let (value, priority) = match budget {
        Some(budget) => queue
            .pop_guarded(|units| budget.reserve(units))
            .ok()
            .map(|entry| entry.into_parts()),
        None => queue.pop(),
    }?;
    Some((value.unpack(), priority))
}

fn queue_decrease(
    queue: &mut PriorityQueue<PackedQueueState>,
    budget: Option<&LocalRouteSearchWork>,
    handle: Handle,
    priority: f64,
) -> Result<(), ()> {
    match budget {
        Some(budget) => queue
            .decrease_guarded(handle, priority, |units| budget.reserve(units))
            .map_err(|_| ()),
        None => queue.decrease(handle, priority).map_err(|_| ()),
    }
}

include!("visibility_canonical.rs");

/// One adjacency slot of the search graph: the neighbor's index, the OVG
/// edge distance, the geometric axis and, in a canonical view, the slot's
/// canonical edge identity. 16 bytes instead of a tuple plus a separate
/// identity list.
#[derive(Clone, Copy, Debug)]
struct AdjacencyEntry {
    distance: f64,
    /// Neighbor index in the low 30 bits, axis code in the top two.
    neighbor_axis: u32,
    /// Encoded canonical edge identity; see `encode_edge_identity`.
    edge: u32,
}

const ADJACENCY_NEIGHBOR_BITS: u32 = 30;
const ADJACENCY_NEIGHBOR_MASK: u32 = (1 << ADJACENCY_NEIGHBOR_BITS) - 1;
/// No edge identity: a non-canonical graph's slot, or a nil canonical slot.
const ADJACENCY_NO_EDGE: u32 = u32::MAX;
/// Marks a foreign identity `usize::MAX - k`, stored as `k`.
const ADJACENCY_FOREIGN_EDGE: u32 = 1 << 31;

/// Canonical edge identities are build edge indices or, for foreign
/// incidences, `usize::MAX - k`; both stay far below 2^31 in any OVG that
/// fits in memory. Decoding returns the original value exactly.
fn encode_edge_identity(edge: Option<usize>) -> u32 {
    let Some(edge) = edge else {
        return ADJACENCY_NO_EDGE;
    };
    if let Ok(index) = u32::try_from(edge)
        && index < ADJACENCY_FOREIGN_EDGE
    {
        return index;
    }
    let foreign = u32::try_from(usize::MAX - edge)
        .ok()
        .filter(|&foreign| foreign < ADJACENCY_FOREIGN_EDGE - 1)
        .expect("OVG edge identity");
    ADJACENCY_FOREIGN_EDGE | foreign
}

impl AdjacencyEntry {
    fn new(neighbor: usize, distance: f64, axis: Axis, edge: Option<usize>) -> Self {
        let neighbor = u32::try_from(neighbor)
            .ok()
            .filter(|&neighbor| neighbor <= ADJACENCY_NEIGHBOR_MASK)
            .expect("OVG adjacency neighbor index");
        let axis = match axis {
            Axis::None => 0,
            Axis::Horizontal => 1,
            Axis::Vertical => 2,
        };
        Self {
            distance,
            neighbor_axis: neighbor | axis << ADJACENCY_NEIGHBOR_BITS,
            edge: encode_edge_identity(edge),
        }
    }

    #[inline]
    fn neighbor(self) -> usize {
        (self.neighbor_axis & ADJACENCY_NEIGHBOR_MASK) as usize
    }

    fn axis(self) -> Axis {
        match self.neighbor_axis >> ADJACENCY_NEIGHBOR_BITS {
            0 => Axis::None,
            1 => Axis::Horizontal,
            _ => Axis::Vertical,
        }
    }

    #[inline]
    fn edge(self) -> Option<usize> {
        match self.edge {
            ADJACENCY_NO_EDGE => None,
            edge if edge & ADJACENCY_FOREIGN_EDGE != 0 => {
                Some(usize::MAX - (edge & !ADJACENCY_FOREIGN_EDGE) as usize)
            }
            edge => Some(edge as usize),
        }
    }

    /// `(neighbor, distance, axis)`, the slot as a nested adjacency list
    /// stores it.
    fn tuple(self) -> (usize, f64, Axis) {
        (self.neighbor(), self.distance, self.axis())
    }
}

/// Search adjacency in one flat table: node `i`'s slots, in OVG slot
/// order, are `entries[offsets[i]..offsets[i + 1]]`. Indexing a node
/// outside the table panics, as indexing a nested list would.
#[derive(Clone, Debug, Default)]
struct Adjacency {
    offsets: Vec<u32>,
    entries: Vec<AdjacencyEntry>,
}

impl Adjacency {
    fn with_capacity(nodes: usize) -> Self {
        let mut offsets = Vec::with_capacity(nodes + 1);
        offsets.push(0);
        Self {
            offsets,
            entries: Vec::new(),
        }
    }

    /// The node count.
    fn len(&self) -> usize {
        self.offsets.len().saturating_sub(1)
    }

    fn get(&self, node: usize) -> Option<&[AdjacencyEntry]> {
        let start = *self.offsets.get(node)? as usize;
        let end = *self.offsets.get(node + 1)? as usize;
        Some(&self.entries[start..end])
    }

    fn tuples(&self, node: usize) -> impl Iterator<Item = (usize, f64, Axis)> + '_ {
        self[node].iter().map(|slot| slot.tuple())
    }

    /// Append a slot to the last node, which is still being built.
    fn push_slot(&mut self, slot: AdjacencyEntry) {
        self.entries.push(slot);
    }

    /// Close the last node's slots and start the next node.
    fn finish_node(&mut self) {
        if self.offsets.is_empty() {
            self.offsets.push(0);
        }
        self.offsets
            .push(u32::try_from(self.entries.len()).expect("OVG adjacency slot count"));
    }

    /// Append a slot to the end of `node`'s slots, after its existing ones.
    /// This shifts every later node; only non-canonical fixed-route import
    /// adds slots to a finished table.
    fn append_slot(&mut self, node: usize, slot: AdjacencyEntry) {
        let end = self.offsets[node + 1] as usize;
        self.entries.insert(end, slot);
        for offset in &mut self.offsets[node + 1..] {
            *offset += 1;
        }
    }
}

impl std::ops::Index<usize> for Adjacency {
    type Output = [AdjacencyEntry];

    fn index(&self, node: usize) -> &[AdjacencyEntry] {
        let start = self.offsets[node] as usize;
        let end = self.offsets[node + 1] as usize;
        &self.entries[start..end]
    }
}

impl Adjacency {
    /// Freeze a nested adjacency list (non-canonical construction) into
    /// the flat table, keeping every node's slot order.
    #[cfg(test)]
    fn from_nested(nested: Vec<Vec<(usize, f64, Axis)>>) -> Self {
        let mut adjacency = Self::with_capacity(nested.len());
        for slots in nested {
            for (neighbor, distance, axis) in slots {
                adjacency.push_slot(AdjacencyEntry::new(neighbor, distance, axis, None));
            }
            adjacency.finish_node();
        }
        adjacency
    }
}

/// Lookup-only copies of OVG port ownership. Go stores these sets only on
/// their OVG nodes; the search view needs membership and emptiness, so keep
/// the same sorted owners in one flat table instead of a tree per node.
#[derive(Clone, Debug, Default)]
struct OwnerSets {
    offsets: Vec<u32>,
    owners: Vec<NodeId>,
}

impl OwnerSets {
    fn with_capacity(nodes: usize) -> Self {
        let mut offsets = Vec::with_capacity(nodes + 1);
        offsets.push(0);
        Self {
            offsets,
            owners: Vec::new(),
        }
    }

    fn clear(&mut self) {
        self.offsets.clear();
        self.owners.clear();
        self.offsets.push(0);
    }

    fn push(&mut self, owners: impl IntoIterator<Item = NodeId>) {
        if self.offsets.is_empty() {
            self.offsets.push(0);
        }
        self.owners.extend(owners);
        self.offsets
            .push(u32::try_from(self.owners.len()).expect("OVG port owner count"));
    }
}

impl FromIterator<BTreeSet<NodeId>> for OwnerSets {
    fn from_iter<T: IntoIterator<Item = BTreeSet<NodeId>>>(sets: T) -> Self {
        let sets = sets.into_iter();
        let mut result = Self::with_capacity(sets.size_hint().0);
        for owners in sets {
            result.push(owners);
        }
        result
    }
}

impl From<Vec<BTreeSet<NodeId>>> for OwnerSets {
    fn from(sets: Vec<BTreeSet<NodeId>>) -> Self {
        sets.into_iter().collect()
    }
}

impl std::ops::Index<usize> for OwnerSets {
    type Output = [NodeId];

    fn index(&self, node: usize) -> &[NodeId] {
        &self.owners[self.offsets[node] as usize..self.offsets[node + 1] as usize]
    }
}

#[derive(Clone, Debug, Default)]
pub(super) struct VisibilityGraph {
    canonical: Option<Box<CanonicalView>>,
    points: Vec<Point>,
    edges: Adjacency,
    /// Search arrays use the source Index, independently of object identity
    /// and membership; retained objects may be absent from Nodes.
    search_indices: Vec<usize>,
    centers: BTreeMap<NodeId, usize>,
    center_owners: Vec<Option<NodeId>>,
    point_containers: Vec<Option<NodeId>>,
    port_owners: OwnerSets,
    near_port_owners: OwnerSets,
    /// Hierarchy intersection coordinates rejected by the hierarchy OVG's own
    /// port visibility test. They are retained for diagnostics and prevent
    /// slingshot walks from traversing a point the upstream OVG omits.
    rejected_hierarchy_points: BTreeSet<(u64, u64)>,
    /// Tunnel membership is queried for every static OVG segment during
    /// route-index construction. Keep the geometric key directly instead of
    /// doing a point-to-index tree lookup for each query; the key is the same
    /// bitwise coordinate identity used by OVG interning.
    tunnel_points: BTreeSet<(u64, u64)>,
    added_tree_ports: BTreeMap<NodeId, Vec<Port>>,
}

pub(super) struct AnyTargetRoute {
    pub(super) points: Vec<Point>,
    /// Exact OVG identities, including both owner-specific center nodes.
    /// These survive the search so route publication does not infer pointer
    /// identity from coordinate equality.
    pub(super) node_ids: Vec<usize>,
    pub(super) source: Port,
    pub(super) target: Port,
    pub(super) cost: f64,
}

fn point_key(point: Point) -> (u64, u64) {
    super::go_point_map_key(point)
}

fn ordered_box_ids(graph_nodes: &[NodeId], boxes: &BTreeMap<NodeId, Rect>) -> Vec<NodeId> {
    let mut order = Vec::with_capacity(boxes.len());
    let mut seen = BTreeSet::new();
    for node in graph_nodes.iter().copied() {
        if boxes.contains_key(&node) && seen.insert(node) {
            order.push(node);
        }
    }
    for node in boxes.keys().copied() {
        if seen.insert(node) {
            order.push(node);
        }
    }
    order
}

/// Go fixedOverlapsForRoute: derive once per flavor, publishing only a complete
/// scan. Hierarchy queries and early overlap exits retain their source order.
pub(super) fn fixed_overlaps_for_route(
    graph: &ArenaGraph,
    nodes: &[NodeId],
    budget: Option<&LocalRouteSearchWork>,
) -> Option<BTreeSet<NodeId>> {
    let step = || -> Option<()> {
        if let Some(budget) = budget {
            budget.step().ok()?;
        }
        Some(())
    };
    let has_fixed_ancestor = |node: NodeId| -> Option<bool> {
        let mut current = Some(node);
        while let Some(node) = current {
            step()?;
            if graph.nodes[node.0 as usize].fixed_top_left.is_some() {
                return Some(true);
            }
            let owner = &graph.nodes[node.0 as usize];
            // OwningContainer follows this node's actual group backlink;
            // placement's cached aggregate owner can skip an intervening
            // container and is not the source hierarchy walk.
            current = if let Some(cluster) = owner
                .cluster
                .filter(|&index| graph.cluster_is_active(&graph.clusters[index]))
            {
                let group = &graph.clusters[cluster];
                if let Some(vessel) = graph
                    .nodes
                    .iter()
                    .find(|candidate| candidate.tala_id == group.vessel_tala_id)
                {
                    vessel.container
                } else if let Some(container) = graph
                    .serialized_cluster_containers_by_tala
                    .get(&group.vessel_tala_id)
                {
                    container.and_then(|id| {
                        graph
                            .nodes
                            .iter()
                            .find(|candidate| candidate.tala_id == id)
                            .map(|node| node.input_id)
                    })
                } else {
                    group
                        .members
                        .first()
                        .and_then(|id| graph.nodes[id.0 as usize].container)
                }
            } else if let Some(sequence) = owner
                .sequence
                .filter(|&index| graph.sequence_is_active(&graph.sequences[index]))
            {
                graph.sequences[sequence].container
            } else {
                owner.container
            };
        }
        Some(false)
    };
    let mut fixed_nodes = Vec::new();
    for &node in nodes {
        step()?;
        if has_fixed_ancestor(node)? {
            fixed_nodes.push(node);
        }
    }
    let mut overlaps = BTreeSet::new();
    for node in fixed_nodes {
        step()?;
        if overlaps.contains(&node) {
            continue;
        }
        for &other in nodes {
            step()?;
            let node_below = super::route_search_guard::is_descendant_of(
                graph,
                Some(node),
                Some(other),
                budget,
            )?;
            let other_below = super::route_search_guard::is_descendant_of(
                graph,
                Some(other),
                Some(node),
                budget,
            )?;
            if node == other || node_below || other_below {
                continue;
            }
            if super::node_rect(graph, node)?.overlaps(super::node_rect(graph, other)?) {
                overlaps.insert(node);
                if has_fixed_ancestor(other)? {
                    overlaps.insert(other);
                }
                break;
            }
        }
    }
    if let Some(budget) = budget {
        budget.check().ok()?;
    }
    Some(overlaps)
}

fn compute_fixed_overlaps(
    graph: &ArenaGraph,
    boxes: &BTreeMap<NodeId, Rect>,
    box_order: &[NodeId],
) -> BTreeSet<NodeId> {
    let fixed_nodes = box_order
        .iter()
        .copied()
        .filter(|node| {
            let mut current = Some(*node);
            while let Some(candidate) = current {
                if graph.nodes[candidate.0 as usize].fixed_top_left.is_some() {
                    return true;
                }
                current = graph.nodes[candidate.0 as usize].container;
            }
            false
        })
        .collect::<Vec<_>>();
    let mut fixed_overlaps = BTreeSet::new();
    for node in fixed_nodes.iter().copied() {
        let Some(node_box) = boxes.get(&node).copied() else {
            continue;
        };
        for other in box_order.iter().copied() {
            if other == node
                || graph.is_descendant_of_scope(node, Some(other))
                || graph.is_descendant_of_scope(other, Some(node))
            {
                continue;
            }
            let Some(other_box) = boxes.get(&other).copied() else {
                continue;
            };
            if node_box.overlaps(other_box) {
                fixed_overlaps.insert(node);
                if fixed_nodes.contains(&other) {
                    fixed_overlaps.insert(other);
                }
                break;
            }
        }
    }
    fixed_overlaps
}

fn fixed_overlaps_for_routing(
    graph: &ArenaGraph,
    boxes: &BTreeMap<NodeId, Rect>,
    box_order: &[NodeId],
) -> BTreeSet<NodeId> {
    compute_fixed_overlaps(graph, boxes, box_order)
}

fn coordinate_key(coordinate: f64) -> u64 {
    super::go_float_map_key(coordinate)
}

fn unique_axis_coordinates(values: impl Iterator<Item = f64>) -> Vec<f64> {
    let mut unique = BTreeMap::<u64, f64>::new();
    let mut nan_values = Vec::new();
    for value in values {
        if value.is_nan() {
            // Go map keys containing NaN are never equal, even to an
            // identical NaN, so each serialized NaN remains a candidate.
            nan_values.push(value);
            continue;
        }
        unique
            .entry(coordinate_key(value))
            .and_modify(|stored| {
                // total ordering sorts -0 before +0 and Vec::dedup keeps the
                // negative zero representative when both occur.
                if value == 0.0 && value.is_sign_negative() {
                    *stored = value;
                }
            })
            .or_insert(value);
    }
    unique.into_values().chain(nan_values).collect()
}

fn outward(side: PortSide) -> Point {
    match side {
        PortSide::Top => Point { x: 0.0, y: -1.0 },
        PortSide::Left => Point { x: -1.0, y: 0.0 },
        PortSide::Bottom => Point { x: 0.0, y: 1.0 },
        PortSide::Right => Point { x: 1.0, y: 0.0 },
    }
}

fn follows_outward(delta: Point, side: PortSide) -> bool {
    let expected = outward(side);
    delta.x * expected.x + delta.y * expected.y > 0.0
        && (delta.x * expected.y - delta.y * expected.x).abs() < f64::EPSILON
}

fn simplify(points: Vec<Point>) -> Vec<Point> {
    let mut result: Vec<Point> = Vec::with_capacity(points.len());
    for point in points {
        if result.last() == Some(&point) {
            continue;
        }
        while result.len() >= 2 {
            let a = result[result.len() - 2];
            let b = result[result.len() - 1];
            if (a.x == b.x && b.x == point.x) || (a.y == b.y && b.y == point.y) {
                result.pop();
            } else {
                break;
            }
        }
        result.push(point);
    }
    result
}

fn point_near_rect(point: Point, rect: Rect, delta: f64) -> bool {
    rect.origin.x - delta <= point.x
        && point.x <= rect.right() + delta
        && rect.origin.y - delta <= point.y
        && point.y <= rect.bottom() + delta
}

#[cfg(test)]
fn add_corner_nodes_not_near(
    points: &mut Vec<Point>,
    tight_left: f64,
    tight_top: f64,
    tight_right: f64,
    tight_bottom: f64,
    is_point_near: impl Fn(Point) -> bool,
    mut added: impl FnMut(Point),
) {
    for layer in 1..=3 {
        let delta = f64::from(layer) * 20.0;
        let left = tight_left - delta;
        let right = tight_right + delta;
        let top = tight_top - delta;
        let bottom = tight_bottom + delta;
        for point in [
            Point { x: left, y: top },
            Point { x: right, y: top },
            Point {
                x: right,
                y: bottom,
            },
            Point { x: left, y: bottom },
        ] {
            if !is_point_near(point) {
                added(point);
                points.push(point);
            }
        }
    }
}

/// Add the port-aligned lanes emitted by TALA's hierarchy OVG.
///
/// The ordinary OVG and each hierarchy OVG are built separately upstream.
/// Their merged graph therefore contains level lanes that cannot be recovered
/// from ordinary port intersections or edge halfway points alone.  Keep the
/// construction in the flattened graph, using the durable hierarchy id and
/// level carried by each arena node.
fn add_hierarchy_level_points(
    graph: &ArenaGraph,
    graph_nodes: &[NodeId],
    boxes: &BTreeMap<NodeId, Rect>,
    ports_by_node: &BTreeMap<NodeId, Vec<Port>>,
    points: &mut Vec<Point>,
    allow_intersection: impl Fn(&[NodeId], Point) -> bool,
    rejected: &mut BTreeSet<(u64, u64)>,
) {
    let mut hierarchies = BTreeMap::<usize, BTreeMap<i64, Vec<NodeId>>>::new();
    for node in graph_nodes.iter().copied() {
        let Some(membership) = graph.nodes[node.0 as usize].hierarchy.clone() else {
            continue;
        };
        hierarchies
            .entry(membership.id)
            .or_default()
            .entry(membership.level())
            .or_default()
            .push(node);
    }

    for (hierarchy_id, levels) in hierarchies.iter_mut() {
        if levels.len() < 2 {
            continue;
        }
        let members = levels.values().flatten().copied().collect::<Vec<_>>();
        let hierarchy_start_points = points.len();
        if crate::internal::trace_env_enabled("TALA_TRACE_OVG_STAGES") {
            let counts = members
                .iter()
                .map(|node| ports_by_node.get(node).map_or(0, Vec::len))
                .collect::<Vec<_>>();
            eprintln!(
                "OVG_HIER_RUST members={:?} port_counts={:?}",
                members
                    .iter()
                    .map(|node| graph.nodes[node.0 as usize].tala_id)
                    .collect::<Vec<_>>(),
                counts
            );
        }
        // `newOVGForHierarchy` creates a canonical top-to-bottom OVG. A
        // horizontal container is transposed for that construction and
        // transformed back before its non-port nodes are merged into the
        // ordinary graph. Keep the same local frame here; otherwise level
        // lanes are emitted along the wrong axis (and every 20-unit lane
        // becomes an extra route vertex).
        let is_horizontal = graph
            .directions
            // Go's hierarchy.IsHorizontal asks the first node for its
            // owning container direction. Graph.Direction returns NONE when
            // that container has no explicit direction; it does not fall
            // back to the root direction. Falling back here transposes every
            // root hierarchy whose global direction is horizontal, even
            // though the hierarchy itself is vertical.
            .get(&graph.nodes[members[0].0 as usize].container)
            .is_some_and(|direction| {
                matches!(direction, crate::Direction::Left | crate::Direction::Right)
            })
            || members
                .iter()
                .any(|node| graph.nodes[node.0 as usize].shape == crate::ShapeKind::SqlTable);
        let local_point = |point: Point| {
            if is_horizontal {
                Point {
                    x: point.y,
                    y: point.x,
                }
            } else {
                point
            }
        };
        let output_point = local_point;
        let local_rect = |node: NodeId| {
            let rect = boxes[&node];
            if is_horizontal {
                Rect {
                    origin: local_point(rect.origin),
                    size: Size {
                        width: rect.size.height,
                        height: rect.size.width,
                    },
                }
            } else {
                rect
            }
        };
        let transpose_side = |side: PortSide| match side {
            PortSide::Top => PortSide::Left,
            PortSide::Bottom => PortSide::Right,
            PortSide::Left => PortSide::Top,
            PortSide::Right => PortSide::Bottom,
        };
        // The hierarchy OVG is transposed only for a horizontal container.
        // Vertical top-to-bottom hierarchies keep their port sides unchanged;
        // rotating those sides unconditionally drops the port-aligned lanes
        // emitted between levels and changes the routing topology.
        let local_side = |side: PortSide| {
            if is_horizontal {
                transpose_side(side)
            } else {
                side
            }
        };
        let mut hierarchy_points_local = members
            .iter()
            .flat_map(|node| ports_by_node.get(node).into_iter().flatten())
            .map(|port| local_point(port.point))
            .collect::<Vec<_>>();
        hierarchy_points_local.dedup();
        let side_ports = |node: NodeId, side: PortSide| {
            ports_by_node
                .get(&node)
                .into_iter()
                .flatten()
                .copied()
                .filter(move |port| local_side(port.direction) == side)
                .map(|port| Port {
                    point: local_point(port.point),
                    direction: local_side(port.direction),
                    side: local_side(port.side),
                    ..port
                })
                .collect::<Vec<_>>()
        };
        macro_rules! push_hierarchy_point {
            ($global:expr, $point:expr) => {{
                let point = $point;
                if point.x.is_finite()
                    && point.y.is_finite()
                    && !hierarchy_points_local.contains(&point)
                {
                    hierarchy_points_local.push(point);
                    let output = local_point(point);
                    // mergeNonPorts uses OVG.AddNode, so hierarchy vertices
                    // that coincide with an already interned ordinary vertex
                    // do not create a second global node. Keep the point in
                    // the hierarchy-local inventory for subsequent stages,
                    // but preserve the ordinary graph's canonical owner.
                    if !$global
                        .iter()
                        .any(|candidate| point_key(*candidate) == point_key(output))
                    {
                        $global.push(output);
                    }
                }
            }};
        }
        let member_set = members.iter().copied().collect::<BTreeSet<_>>();
        let local_rect_for_node = |node: NodeId| {
            if member_set.contains(&node) {
                local_rect(node)
            } else {
                boxes[&node]
            }
        };
        let local_near_node = |point: Point| {
            graph_nodes.iter().copied().any(|node| {
                let arena_node = &graph.nodes[node.0 as usize];
                !arena_node.is_container && point_near_rect(point, local_rect_for_node(node), 20.0)
            })
        };
        let local_port_infos = members
            .iter()
            .flat_map(|node| {
                ports_by_node
                    .get(node)
                    .into_iter()
                    .flatten()
                    .copied()
                    .map(|port| PortVisibilityInfo {
                        owner: *node,
                        port: Port {
                            point: local_point(port.point),
                            direction: local_side(port.direction),
                            side: local_side(port.side),
                            ..port
                        },
                        owner_box: local_rect(*node),
                        owner_sequence: graph.nodes[node.0 as usize].sequence,
                        owner_invisible: graph.nodes[node.0 as usize].is_invisible,
                    })
            })
            .collect::<Vec<_>>();
        let local_obstructions = graph_nodes
            .iter()
            .copied()
            .map(|node| {
                let arena_node = &graph.nodes[node.0 as usize];
                (
                    node,
                    local_rect_for_node(node),
                    arena_node.sequence,
                    arena_node.is_container,
                    false,
                )
            })
            .collect::<Vec<_>>();
        // The hierarchy OVG runs its own port Cartesian-product pass before
        // level lanes. The merged ordinary graph's intersections are broader
        // and cannot stand in for this local pass after a horizontal
        // transpose.
        let mut hierarchy_xs = local_port_infos
            .iter()
            .map(|info| info.port.point.x)
            .collect::<Vec<_>>();
        let mut hierarchy_ys = local_port_infos
            .iter()
            .map(|info| info.port.point.y)
            .collect::<Vec<_>>();
        hierarchy_xs.sort_by(f64::total_cmp);
        hierarchy_xs.dedup();
        hierarchy_ys.sort_by(f64::total_cmp);
        hierarchy_ys.dedup();
        for x in hierarchy_xs {
            for y in hierarchy_ys.iter().copied() {
                let point = Point { x, y };
                let too_close = local_port_infos.iter().any(|info| {
                    info.port.point.y == y && (info.port.point.x - x).abs() <= 20.0
                        || info.port.point.x == x && (info.port.point.y - y).abs() <= 20.0
                });
                if too_close || local_near_node(point) {
                    continue;
                }
                if !has_unobstructed_line_to_ports_with_obstructions(
                    point,
                    &local_port_infos,
                    &local_obstructions,
                    2,
                ) {
                    continue;
                }
                push_hierarchy_point!(points, point);
            }
        }
        if crate::internal::trace_env_enabled("TALA_TRACE_OVG_STAGES") {
            eprintln!(
                "OVG_HIER_RUST stage=after-addNodesIntersections nodes={} added={} levels={}",
                points.len(),
                points.len().saturating_sub(hierarchy_start_points),
                levels.len()
            );
        }
        for nodes in levels.values_mut() {
            nodes.sort_by(|left, right| {
                local_rect(*left)
                    .origin
                    .x
                    .total_cmp(&local_rect(*right).origin.x)
                    .then_with(|| {
                        graph.nodes[left.0 as usize]
                            .tala_id
                            .cmp(&graph.nodes[right.0 as usize].tala_id)
                    })
            });
        }
        // Go derives hierarchy level bounds from the OVG's port vertices, not
        // from node rectangles. Outside labels and rounded snap points can
        // extend beyond a node's box (the queue witness reaches x=-46), and
        // those coordinates determine the aligned 20-unit lanes.
        let Some(min_x) = members
            .iter()
            .flat_map(|node| {
                ports_by_node
                    .get(node)
                    .into_iter()
                    .flatten()
                    .map(|port| local_point(port.point).x)
            })
            .min_by(f64::total_cmp)
        else {
            continue;
        };
        let Some(max_right) = members
            .iter()
            .flat_map(|node| {
                ports_by_node
                    .get(node)
                    .into_iter()
                    .flatten()
                    .map(|port| local_point(port.point).x)
            })
            .max_by(f64::total_cmp)
        else {
            continue;
        };
        let left_bound = ((min_x - 20.0) / 20.0).floor() * 20.0;
        let width = ((max_right - left_bound) / 20.0).ceil() * 20.0;
        let right_bound = left_bound + width + 20.0;
        let mut level_ys = BTreeSet::<u64>::new();

        for level in 1..levels.len() as i64 {
            let Some(nodes_above) = levels.get(&(level - 1)) else {
                continue;
            };
            let Some(nodes_below) = levels.get(&level) else {
                continue;
            };
            let Some(below_min_y) = nodes_below
                .iter()
                .map(|node| local_rect(*node).origin.y)
                .min_by(f64::total_cmp)
            else {
                continue;
            };
            let Some(above_max_y) = nodes_above
                .iter()
                .map(|node| local_rect(*node).bottom())
                .max_by(f64::total_cmp)
            else {
                continue;
            };
            let level_distance = below_min_y - above_max_y;
            if level_distance <= 0.0 {
                continue;
            }
            let mut edges_above: f64 = 1.0;
            for node in nodes_below {
                for edge_id in &graph.nodes[node.0 as usize].edges.read() {
                    let edge = &graph.edge_records[edge_id.0 as usize];
                    if edge.from == edge.to {
                        continue;
                    }
                    let adjacent = if edge.from == *node {
                        edge.to
                    } else {
                        edge.from
                    };
                    if graph.nodes[adjacent.0 as usize]
                        .hierarchy
                        .clone()
                        .is_some_and(|membership| {
                            membership.id == *hierarchy_id && membership.level() < level
                        })
                    {
                        edges_above += 1.0;
                    }
                }
            }
            let max_lines = level_distance / 20.0;
            let line_count = edges_above.min(max_lines);
            if line_count <= 1.0 {
                continue;
            }
            let padding = (level_distance / line_count).ceil().max(20.0);
            let mut y = below_min_y;
            for _ in 0..(line_count as usize - 1) {
                y -= padding;
                push_hierarchy_point!(points, Point { x: left_bound, y });
                push_hierarchy_point!(points, Point { x: right_bound, y });
                level_ys.insert(coordinate_key(y));
                for node in nodes_above {
                    for port in side_ports(*node, PortSide::Bottom) {
                        push_hierarchy_point!(points, Point { x: port.point.x, y });
                    }
                }
                for node in nodes_below {
                    for port in side_ports(*node, PortSide::Top) {
                        push_hierarchy_point!(points, Point { x: port.point.x, y });
                    }
                }
            }
            if crate::internal::trace_env_enabled("TALA_TRACE_OVG_STAGES") {
                eprintln!(
                    "OVG_HIER_RUST level={} after-boundary nodes={} lines={}",
                    level,
                    points.len(),
                    (line_count as usize).saturating_sub(1)
                );
            }
        }
        if crate::internal::trace_env_enabled("TALA_TRACE_OVG_STAGES") {
            eprintln!(
                "OVG_HIER_RUST stage=after-addHierarchyLevelNodes nodes={} added={} levels={}",
                points.len(),
                points.len().saturating_sub(hierarchy_start_points),
                levels.len()
            );
        }

        let mut level_xs = BTreeSet::<u64>::new();
        for nodes in levels.values() {
            let Some(first) = nodes.first().copied() else {
                continue;
            };
            let mut node_index = 0usize;
            let mut current = first;
            let mut current_ports = side_ports(current, PortSide::Left);
            // The upstream loop performs one transition for the final node:
            // it replaces that node's left ports with its right ports and
            // jumps to the aligned right edge. `is_last_node` records that
            // transition; deriving it from the index before the transition
            // would leave the final node's left ports active across the rest
            // of the lane and materialize hundreds of extra vertices.
            let mut is_last_node = false;
            let mut x = left_bound;
            loop {
                if x > right_bound + f64::EPSILON {
                    break;
                }
                level_xs.insert(coordinate_key(x));
                for port in current_ports.iter().copied() {
                    push_hierarchy_point!(points, Point { x, y: port.point.y });
                }
                if !is_last_node && x >= local_rect(current).origin.x - 20.0 {
                    let next_x = local_rect(current).right();
                    x = (next_x / 20.0).ceil() * 20.0;
                    current_ports = side_ports(current, PortSide::Right);
                    is_last_node = node_index + 1 >= nodes.len();
                    if !is_last_node {
                        node_index += 1;
                        current = nodes[node_index];
                        current_ports.extend(side_ports(current, PortSide::Left));
                    }
                    x += 20.0;
                } else {
                    x += 20.0;
                }
            }
        }
        if crate::internal::trace_env_enabled("TALA_TRACE_OVG_STAGES") {
            eprintln!(
                "OVG_HIER_RUST after-level-nodes nodes={} xs={} ys={}",
                points.len(),
                level_xs.len(),
                level_ys.len()
            );
        }

        // `addIntersections` receives every level X/Y axis. The complete
        // visibility predicate is applied later by the ordinary connection
        // pass; retaining the axes here is enough to materialize the same
        // candidate points without introducing obstacle-crossing shortcuts.
        for x_bits in level_xs {
            let x = f64::from_bits(x_bits);
            for y_bits in &level_ys {
                let point = Point {
                    x,
                    y: f64::from_bits(*y_bits),
                };
                let output = output_point(point);
                let already_present = points.contains(&output);
                let allowed = allow_intersection(&members, output);
                if !allowed && !already_present {
                    rejected.insert(point_key(output));
                    continue;
                }
                push_hierarchy_point!(points, point);
            }
        }
        if crate::internal::trace_env_enabled("TALA_TRACE_OVG_STAGES") {
            eprintln!(
                "OVG_HIER_RUST stage=after-level-intersections nodes={} added={} levels={}",
                points.len(),
                points.len().saturating_sub(hierarchy_start_points),
                levels.len()
            );
        }
        // The hierarchy OVG has its own three boundary sweeps and corners.
        // They use the hierarchy-local tight bounds (after transpose), then
        // the completed point inventory is transformed back into the merged
        // graph frame.
        let hierarchy_left = hierarchy_points_local
            .iter()
            .map(|point| point.x)
            .min_by(f64::total_cmp)
            .unwrap_or(0.0);
        let hierarchy_top = hierarchy_points_local
            .iter()
            .map(|point| point.y)
            .min_by(f64::total_cmp)
            .unwrap_or(0.0);
        let hierarchy_right = hierarchy_points_local
            .iter()
            .map(|point| point.x)
            .max_by(f64::total_cmp)
            .unwrap_or(0.0);
        let hierarchy_bottom = hierarchy_points_local
            .iter()
            .map(|point| point.y)
            .max_by(f64::total_cmp)
            .unwrap_or(0.0);
        for layer in 1..=3 {
            let delta = f64::from(layer) * 20.0;
            let existing = hierarchy_points_local.clone();
            for point in existing {
                let mut boundary = None;
                if point.y == hierarchy_top {
                    boundary = Some(Point {
                        x: point.x,
                        y: point.y - delta,
                    });
                }
                if point.y == hierarchy_bottom {
                    boundary = Some(Point {
                        x: point.x,
                        y: point.y + delta,
                    });
                }
                if point.x == hierarchy_left {
                    boundary = Some(Point {
                        x: point.x - delta,
                        y: point.y,
                    });
                }
                if point.x == hierarchy_right {
                    boundary = Some(Point {
                        x: point.x + delta,
                        y: point.y,
                    });
                }
                if let Some(boundary) = boundary
                    && !local_near_node(boundary)
                {
                    push_hierarchy_point!(points, boundary);
                }
            }
        }
        if crate::internal::trace_env_enabled("TALA_TRACE_OVG_STAGES") {
            eprintln!(
                "OVG_HIER_RUST stage=after-addNewBoundaryLayers nodes={} added={} levels={}",
                points.len(),
                points.len().saturating_sub(hierarchy_start_points),
                levels.len()
            );
        }
        for layer in 1..=3 {
            let delta = f64::from(layer) * 20.0;
            for corner in [
                Point {
                    x: hierarchy_left - delta,
                    y: hierarchy_top - delta,
                },
                Point {
                    x: hierarchy_right + delta,
                    y: hierarchy_top - delta,
                },
                Point {
                    x: hierarchy_right + delta,
                    y: hierarchy_bottom + delta,
                },
                Point {
                    x: hierarchy_left - delta,
                    y: hierarchy_bottom + delta,
                },
            ] {
                if !local_near_node(corner) {
                    push_hierarchy_point!(points, corner);
                }
            }
        }
        if crate::internal::trace_env_enabled("TALA_TRACE_OVG_STAGES") {
            eprintln!(
                "OVG_HIER_RUST stage=after-addCornerNodes nodes={} added={} levels={}",
                points.len(),
                points.len().saturating_sub(hierarchy_start_points),
                levels.len()
            );
        }
        // The hierarchy point inventory is now complete and merged into the
        // ordinary OVG in the same order as the upstream implementation.
    }
    // The per-hierarchy source dump is intentionally emitted after all
    // hierarchies have contributed; this keeps the probe out of ordinary
    // routing and makes the generated point inventory diffable.
}

fn point_on_rect(point: Point, rect: Rect) -> bool {
    rect.origin.x <= point.x
        && point.x <= rect.right()
        && rect.origin.y <= point.y
        && point.y <= rect.bottom()
}

fn segment_crosses_rect_interior(first: Point, second: Point, rect: Rect) -> bool {
    if first.y == second.y {
        first.y > rect.origin.y
            && first.y < rect.bottom()
            && first.x.min(second.x) < rect.right()
            && first.x.max(second.x) > rect.origin.x
    } else if first.x == second.x {
        first.x > rect.origin.x
            && first.x < rect.right()
            && first.y.min(second.y) < rect.bottom()
            && first.y.max(second.y) > rect.origin.y
    } else {
        true
    }
}

/// Axis-aligned specialization of recovered `segmentIntersectsBox`.
///
/// Unlike the ordinary visibility obstruction test, TALA treats contact with
/// the box boundary as an intersection here.
fn segment_intersects_rect(first: Point, second: Point, rect: Rect) -> bool {
    first.x.min(second.x) <= rect.right()
        && rect.origin.x <= first.x.max(second.x)
        && first.y.min(second.y) <= rect.bottom()
        && rect.origin.y <= first.y.max(second.y)
}

// Recovered `Node.passesThrough` delegates to segmentIntersectsBox, whose
// axis-range precheck is followed by endpoint containment and the four
// orthogonal edge tests below. A bounding-box overlap alone is too broad for
// this obstruction predicate (it admits diagonal segments that miss the box).
fn segment_intersects_box_recovered(first: Point, second: Point, rect: Rect) -> bool {
    let left = rect.origin.x;
    let right = rect.right();
    let top = rect.origin.y;
    let bottom = rect.bottom();

    if first.x < second.x {
        if second.x < left || right < first.x {
            return false;
        }
    } else if first.x < left || right < second.x {
        return false;
    }
    if first.y < second.y {
        if second.y < top || bottom < first.y {
            return false;
        }
    } else if first.y < top || bottom < second.y {
        return false;
    }

    let contains =
        |point: Point| left <= point.x && point.x <= right && top <= point.y && point.y <= bottom;
    if contains(first) || contains(second) {
        return true;
    }

    let orientation =
        |p: Point, q: Point, r: Point| (q.y - p.y) * (r.x - q.x) - (q.x - p.x) * (r.y - q.y);
    let equal_signs = |left: f64, right: f64| {
        (left > 0.0 && right > 0.0) || (left == 0.0 && right == 0.0) || (left < 0.0 && right < 0.0)
    };
    let on_segment = |point: Point, start: Point, end: Point| {
        let within_x = if start.x < end.x {
            start.x <= point.x && point.x <= end.x
        } else {
            end.x <= point.x && point.x <= start.x
        };
        let within_y = if start.y < end.y {
            start.y <= point.y && point.y <= end.y
        } else {
            end.y <= point.y && point.y <= start.y
        };
        within_x && within_y
    };
    let intersects = |a: Point, b: Point, c: Point, d: Point| {
        let first_orientation = orientation(a, b, c);
        if first_orientation == 0.0 && on_segment(c, a, b) {
            return true;
        }
        let second_orientation = orientation(a, b, d);
        if second_orientation == 0.0 && on_segment(d, a, b) {
            return true;
        }
        let third_orientation = orientation(c, d, a);
        if third_orientation == 0.0 && on_segment(a, c, d) {
            return true;
        }
        let fourth_orientation = orientation(c, d, b);
        if fourth_orientation == 0.0 && on_segment(b, c, d) {
            return true;
        }
        !equal_signs(first_orientation, second_orientation)
            && !equal_signs(third_orientation, fourth_orientation)
    };

    let top_left = Point { x: left, y: top };
    let top_right = Point { x: right, y: top };
    let bottom_right = Point {
        x: right,
        y: bottom,
    };
    let bottom_left = Point { x: left, y: bottom };
    intersects(
        first,
        Point {
            x: second.x,
            y: second.y - 1.0,
        },
        top_left,
        top_right,
    ) || intersects(
        first,
        Point {
            x: second.x - 1.0,
            y: second.y,
        },
        top_left,
        bottom_left,
    ) || intersects(
        first,
        Point {
            x: second.x + 1.0,
            y: second.y,
        },
        top_right,
        bottom_right,
    ) || intersects(
        first,
        Point {
            x: second.x,
            y: second.y + 1.0,
        },
        bottom_left,
        bottom_right,
    )
}

fn passes_through_allowing_direction(
    first: Point,
    second: Point,
    rect: Rect,
    ports: &[Port],
    direction: Option<PortSide>,
) -> bool {
    if let Some(direction) = direction {
        let delta = Point {
            x: second.x - first.x,
            y: second.y - first.y,
        };
        for port in ports {
            // Node.passesThroughAllowingPorts applies portExceptionDirection
            // to the original p1 -> p2 vector regardless of which endpoint
            // matches a port. It does not reverse the vector for a p2 match.
            if (port.point == first || port.point == second) && follows_outward(delta, direction) {
                return false;
            }
        }
    }
    segment_intersects_rect(first, second, rect)
}

/// Apply Go's `OVGNode.portDirectionsForObstacle(...).any(...)` semantics.
/// A coordinate can represent several snapped sides of the same owner (for
/// example the left and bottom sides of a 1x1 node).  The obstacle check must
/// evaluate every represented direction; one outward direction exempts the
/// segment, while any other represented direction that intersects the box
/// keeps it blocked.  Collapsing that metadata to the first side changes the
/// visibility graph and can select a different routing corridor.
fn passes_through_for_obstacle(
    first: Point,
    second: Point,
    rect: Rect,
    ports: &[Port],
    directions: &[PortSide],
) -> bool {
    if directions.is_empty() {
        return segment_intersects_rect(first, second, rect);
    }
    directions.iter().copied().all(|direction| {
        passes_through_allowing_direction(first, second, rect, ports, Some(direction))
    })
}

#[derive(Clone, Copy)]
struct PortVisibilityInfo {
    owner: NodeId,
    port: Port,
    owner_box: Rect,
    owner_sequence: Option<usize>,
    owner_invisible: bool,
}

/// Translation of recovered `OVGNode.hasUnobstructedLineToPorts`.
///
/// `addIntersections` does not retain the Cartesian product of all port axes.
/// A candidate must be reachable along an unobstructed axis from ports owned
/// by `required_owners` distinct nodes. Lines tangent to the owning node are
/// not port rays, containers are not ordinary obstacles, and the port owner
/// itself is excluded from the obstruction scan.
fn has_unobstructed_line_to_ports(
    point: Point,
    graph: &ArenaGraph,
    boxes: &BTreeMap<NodeId, Rect>,
    graph_nodes: &[NodeId],
    ports: &[(NodeId, Port)],
    fixed_overlaps: &BTreeSet<NodeId>,
    required_owners: usize,
) -> bool {
    let obstruction_nodes = graph_nodes
        .iter()
        .filter_map(|other| {
            boxes.get(other).copied().map(|rect| {
                let node = &graph.nodes[other.0 as usize];
                (
                    *other,
                    rect,
                    node.sequence,
                    node.is_container,
                    fixed_overlaps.contains(other),
                )
            })
        })
        .collect::<Vec<_>>();
    let port_visibility = ports
        .iter()
        .filter_map(|(owner, port)| {
            boxes.get(owner).copied().map(|owner_box| {
                let node = &graph.nodes[owner.0 as usize];
                PortVisibilityInfo {
                    owner: *owner,
                    port: *port,
                    owner_box,
                    owner_sequence: node.sequence,
                    owner_invisible: node.is_invisible,
                }
            })
        })
        .collect::<Vec<_>>();
    has_unobstructed_line_to_ports_with_obstructions(
        point,
        &port_visibility,
        &obstruction_nodes,
        required_owners,
    )
}

fn has_unobstructed_line_to_ports_with_obstructions(
    point: Point,
    ports: &[PortVisibilityInfo],
    obstruction_nodes: &[(NodeId, Rect, Option<usize>, bool, bool)],
    required_owners: usize,
) -> bool {
    // Recovered callers pass one or two required owners. Keep those common
    // cardinality checks allocation-free; retain a set fallback so this
    // helper remains correct if a later source-shaped caller asks for more.
    let mut reachable_owner_a = None;
    let mut reachable_owner_b = None;
    let mut reachable_owners = (required_owners > 2).then(BTreeSet::new);
    let mut saw_aligned_port = false;

    for port_info in ports {
        let owner = port_info.owner;
        let port = port_info.port;
        if port.point.x != point.x && port.point.y != point.y {
            continue;
        }

        let owner_box = port_info.owner_box;
        if port.point.x == point.x
            && (port.point.x == owner_box.origin.x || port.point.x == owner_box.right())
        {
            saw_aligned_port = true;
            continue;
        }
        if port.point.y == point.y
            && (port.point.y == owner_box.origin.y || port.point.y == owner_box.bottom())
        {
            saw_aligned_port = true;
            continue;
        }

        let obstructed = obstruction_nodes.iter().any(
            |(other, rect, other_sequence, other_is_container, fixed_overlap)| {
                let same_sequence = port_info.owner_sequence.zip(*other_sequence).is_some_and(
                    |(owner_sequence, other_sequence)| owner_sequence == other_sequence,
                );

                *other != owner
                    && !port_info.owner_invisible
                    && !*other_is_container
                    && !same_sequence
                    && !*fixed_overlap
                    && segment_intersects_box_recovered(port.point, point, *rect)
            },
        );
        if obstructed {
            saw_aligned_port = true;
            continue;
        }

        let reached_required = if required_owners > 2 {
            reachable_owners
                .as_mut()
                .expect("reachable owner fallback should exist")
                .insert(owner)
                && reachable_owners
                    .as_ref()
                    .is_some_and(|owners| owners.len() == required_owners)
        } else if reachable_owner_a == Some(owner) || reachable_owner_b == Some(owner) {
            false
        } else if reachable_owner_a.is_none() {
            reachable_owner_a = Some(owner);
            required_owners == 1
        } else {
            reachable_owner_b = Some(owner);
            required_owners == 2
        };
        if reached_required {
            return true;
        }
        saw_aligned_port = true;
    }

    !saw_aligned_port
}

impl VisibilityGraph {
    pub(super) fn debug_neighbors(&self, point: Point) -> Option<Vec<Point>> {
        let index = self.index_of(point)?;
        Some(
            self.edges
                .tuples(index)
                .map(|(next, _, _)| self.point(next))
                .collect(),
        )
    }
    /// Ports published by recovered `OVG.addTreeNodes` after the ordinary
    /// port/intersection stages. They are endpoint candidates for later
    /// ordinary edges even though they must not influence those earlier
    /// stages.
    pub(super) fn added_tree_ports(&self, node: NodeId) -> &[Port] {
        self.added_tree_ports
            .get(&node)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    pub(super) fn center_point(&self, node: NodeId) -> Option<Point> {
        self.centers.get(&node).map(|index| self.point(*index))
    }

    fn search_index(&self, node_index: usize) -> usize {
        self.canonical.as_ref().map_or_else(
            || self.search_indices[node_index],
            |view| view.search_nodes[node_index].context(),
        )
    }

    /// The per-hop search records: refresh's copy for a canonical graph,
    /// whose inputs change only with that refresh, or a fresh table.
    fn search_nodes(&self) -> std::borrow::Cow<'_, [SearchNode]> {
        match &self.canonical {
            Some(view) => std::borrow::Cow::Borrowed(&view.search_nodes),
            None => std::borrow::Cow::Owned(self.build_search_nodes()),
        }
    }

    /// Dense copy of the per-node fields one search reads per hop. They are
    /// plain fields of this graph, which the search borrows immutably.
    fn build_search_nodes(&self) -> Vec<SearchNode> {
        (0..self.edges.len())
            .map(|index| SearchNode {
                search_index: compact_search_index(self.search_indices[index]),
                container: self.point_containers[index],
                flags: if self.port_owners[index].is_empty() {
                    0
                } else {
                    SEARCH_NODE_PORT_OWNED
                } | if self.near_port_owners[index].is_empty() {
                    0
                } else {
                    SEARCH_NODE_NEAR_PORT
                } | if self.is_node_center(index) {
                    SEARCH_NODE_CENTER
                } else {
                    0
                },
            })
            .collect()
    }

    pub(super) fn is_tunnel_point(&self, point: Point) -> bool {
        self.tunnel_points.contains(&point_key(point))
    }

    /// `segments()` in the same order, without materializing the canonical
    /// inventory for callers that only walk it once.
    pub(super) fn segment_iter(&self) -> Box<dyn Iterator<Item = [Point; 2]> + '_> {
        match &self.canonical {
            Some(view) if view.frozen => {
                let edges = view.static_edges.get().unwrap();
                Box::new((0..edges.edges.len()).map(|index| edges.edge_points(index)))
            }
            Some(view) => Box::new(view.build.edges.iter().map(move |edge| {
                let edge = edge.read();
                [
                    view.build.nodes[edge.from_slot()].point(),
                    view.build.nodes[edge.to_slot()].point(),
                ]
            })),
            None => Box::new(self.segments().into_iter()),
        }
    }

    pub(super) fn segments(&self) -> Vec<[Point; 2]> {
        if self.canonical.as_ref().is_some_and(|view| view.frozen) {
            return self.segment_iter().collect();
        }
        if let Some(view) = &self.canonical {
            return view
                .build
                .edges
                .iter()
                .map(|edge| {
                    let edge = edge.read();
                    [
                        view.build.nodes[edge.from_slot()].point(),
                        view.build.nodes[edge.to_slot()].point(),
                    ]
                })
                .collect();
        }
        let mut segments = Vec::new();
        for from in 0..self.edges.len() {
            for (to, _, _) in self.edges.tuples(from) {
                if from < to {
                    segments.push([self.point(from), self.point(to)]);
                }
            }
        }
        segments
    }

    /// Add one fixed standalone route after `NewOVGFromGraph` construction.
    /// Recovered `routeAdditionalEdges` interns each serialized route point
    /// against the current occupied-point map, connects center -> points ->
    /// center, and appends otherwise unseen points without port ownership.
    pub(super) fn add_existing_route(
        &mut self,
        source: NodeId,
        target: NodeId,
        route: &[Point],
    ) -> Vec<usize> {
        self.add_existing_route_checked(source, target, route, None)
            .expect("unbudgeted fixed-route import cannot fail")
    }

    /// coordinator.go's addPoint closure checks every input point, including
    /// adjacent duplicates, then inserts and connects it before reading the
    /// next point. Resource reservations use the same guard as the base OVG.
    pub(super) fn add_existing_route_checked(
        &mut self,
        source: NodeId,
        target: NodeId,
        route: &[Point],
        budget: Option<&OvgBuildBudget<'_>>,
    ) -> Result<Vec<usize>, ()> {
        let (Some(&source_center), Some(&target_center)) =
            (self.centers.get(&source), self.centers.get(&target))
        else {
            return Ok(Vec::new());
        };
        if self.canonical.is_some() {
            use crate::internal::owner_points::SharedOwnerPoint;
            let mut imported = super::ovg_import::ImportedRoute::default();
            let points = std::iter::once(self.point(source_center))
                .chain(route.iter().copied())
                .chain(std::iter::once(self.point(target_center)))
                .collect::<Vec<_>>();
            let mut result = Ok(());
            for point in points {
                result = self.canonical.as_mut().unwrap().build.import_route_point(
                    Some(SharedOwnerPoint::new(point)),
                    &mut imported,
                    budget,
                );
                if result.is_err() {
                    break;
                }
            }
            self.refresh_canonical();
            result?;
            let view = self.canonical.as_ref().unwrap();
            return Ok(imported
                .nodes
                .iter()
                .map(|node| view.indices[node])
                .collect());
        }
        let source_point = self.point(source_center);
        let target_point = self.point(target_center);
        let mut indices = Vec::with_capacity(route.len() + 2);
        let mut last_point = None;
        for point in std::iter::once(source_point)
            .chain(route.iter().copied())
            .chain(std::iter::once(target_point))
        {
            if let Some(budget) = budget {
                budget.reserve_work(1)?;
            }
            if last_point == Some(point) {
                continue;
            }
            last_point = Some(point);
            // OccupiedPoints uses the latest equal coordinate, including for
            // the source/target center and any coincident center identities.
            let index = if let Some(index) = self.points.iter().rposition(|p| *p == point) {
                index
            } else {
                if let Some(budget) = budget {
                    budget.reserve_work(1)?; // addNodeUnchecked
                    budget.reserve_nodes(1)?;
                }
                let index = self.points.len();
                self.points.push(point);
                self.edges.finish_node();
                self.search_indices.push(index);
                self.center_owners.push(None);
                self.point_containers.push(None);
                self.port_owners.push(BTreeSet::new());
                self.near_port_owners.push(BTreeSet::new());
                index
            };
            let previous = indices.last().copied();
            indices.push(index);
            let Some(first) = previous else {
                continue;
            };
            if let Some(budget) = budget {
                budget.reserve_work(1)?;
            }
            let first_is_center = self.center_owners[first].is_some();
            let second_is_center = self.center_owners[index].is_some();
            // guard.connect checks center compatibility before reserving an
            // edge; OVG.Connect repeats that predicate without extra work.
            if first_is_center && self.port_owners[index].is_empty()
                || second_is_center && self.port_owners[first].is_empty()
            {
                continue;
            }
            if let Some(budget) = budget {
                budget.reserve_edges(1)?;
            }
            let axis = if self.point(first).y == self.point(index).y {
                Axis::Horizontal
            } else if self.point(first).x == self.point(index).x {
                Axis::Vertical
            } else {
                Axis::None
            };
            if first == index {
                continue;
            }
            let dx = self.point(first).x - self.point(index).x;
            let dy = self.point(first).y - self.point(index).y;
            let distance = dy.mul_add(dy, dx * dx).sqrt();
            self.edges
                .append_slot(first, AdjacencyEntry::new(index, distance, axis, None));
            self.edges
                .append_slot(index, AdjacencyEntry::new(first, distance, axis, None));
        }
        Ok(indices)
    }

    pub(super) fn point(&self, index: usize) -> Point {
        self.canonical.as_ref().map_or_else(
            || self.points[index],
            |view| Self::point_at_canonical(view, index),
        )
    }

    pub(super) fn index_of(&self, point: Point) -> Option<usize> {
        if let Some(view) = &self.canonical {
            return view
                .build
                .occupied_node(point)
                .map(|node| view.indices[&node]);
        }
        self.points
            .iter()
            .rposition(|candidate| *candidate == point)
    }

    /// Static port-node identity, excluding later unchecked imports or centers
    /// that can have the same coordinates as a canonical port.
    pub(super) fn port_index(&self, owner: NodeId, point: Point) -> Option<usize> {
        if let Some(view) = &self.canonical {
            return view.build.ports.get(&owner)?.iter().find_map(|(node, _)| {
                (Self::point_at_canonical(view, view.indices[node]) == point)
                    .then_some(view.indices[node])
            });
        }
        self.points
            .iter()
            .enumerate()
            .find_map(|(index, candidate)| {
                (*candidate == point && self.port_owners[index].contains(&owner)).then_some(index)
            })
    }

    pub(super) fn center_index(&self, node: NodeId) -> Option<usize> {
        self.centers.get(&node).copied()
    }

    pub(super) fn neighbors(&self, index: usize) -> impl Iterator<Item = usize> + '_ {
        self.edges[index].iter().map(|slot| slot.neighbor())
    }

    pub(super) fn is_rejected_hierarchy_point(&self, index: usize) -> bool {
        self.rejected_hierarchy_points
            .contains(&point_key(self.point(index)))
    }

    pub(super) fn segment_is_visible(&self, first: Point, second: Point) -> bool {
        let Some(first) = self.index_of(first) else {
            return false;
        };
        let Some(second) = self.index_of(second) else {
            return false;
        };
        self.edges[first]
            .iter()
            .any(|slot| slot.neighbor() == second)
            || (self.point(first).x == self.point(second).x
                || self.point(first).y == self.point(second).y)
                && fill_visible_path(self, first, second)
    }

    /// Return every OVG vertex encountered while walking one axis-aligned
    /// segment, including both endpoints. TALA's `routeInSShape` retains these
    /// intermediate nodes on tree routes so later edge searches recognize a
    /// shared crossing vertex instead of charging the same crossing twice.
    pub(super) fn orthogonal_path(&self, first: Point, second: Point) -> Option<Vec<Point>> {
        let first_index = self.index_of(first)?;
        let second_index = self.index_of(second)?;
        let mut path = vec![first];
        let mut current = first_index;
        let mut visited = BTreeSet::from([current]);
        while current != second_index {
            let current_point = self.point(current);
            let next = self.edges[current]
                .iter()
                .map(|slot| slot.neighbor())
                .find(|neighbor| {
                    let point = self.point(*neighbor);
                    if first.x == second.x && point.x == current_point.x {
                        (first.y < second.y && current_point.y < point.y)
                            || (first.y > second.y && current_point.y > point.y)
                    } else if first.y == second.y && point.y == current_point.y {
                        (first.x < second.x && current_point.x < point.x)
                            || (first.x > second.x && current_point.x > point.x)
                    } else {
                        false
                    }
                })?;
            if !visited.insert(next) {
                return None;
            }
            current = next;
            path.push(self.point(current));
        }
        Some(path)
    }

    /// Direct translation of TALA's `routeInSShape` walk. The orientation is
    /// the source-port side, and deliberately remains unchanged for the final
    /// target-midpoint-to-target-port leg. That detail is observable: a route
    /// can fail even when the target port is geometrically reachable in the
    /// opposite direction, in which case TALA leaves the edge to ordinary OVG
    /// routing.
    pub(super) fn s_shape_route(
        &self,
        source: Point,
        source_midpoint: Point,
        target_midpoint: Point,
        target: Point,
        orientation: Orientation,
    ) -> Option<Vec<Point>> {
        self.s_shape_route_indices(
            source,
            source_midpoint,
            target_midpoint,
            target,
            orientation,
        )
        .map(|indices| indices.into_iter().map(|index| self.point(index)).collect())
    }

    pub(super) fn s_shape_route_indices(
        &self,
        source: Point,
        source_midpoint: Point,
        target_midpoint: Point,
        target: Point,
        orientation: Orientation,
    ) -> Option<Vec<usize>> {
        fn next_in_direction(
            graph: &VisibilityGraph,
            current: usize,
            direction: Orientation,
        ) -> Option<usize> {
            let current_point = graph.point(current);
            graph.edges[current]
                .iter()
                .map(|entry| entry.neighbor())
                .filter(|neighbor| {
                    let point = graph.point(*neighbor);
                    match direction {
                        Orientation::Top => point.x == current_point.x && point.y > current_point.y,
                        Orientation::Right => {
                            point.y == current_point.y && point.x < current_point.x
                        }
                        Orientation::Left => {
                            point.y == current_point.y && point.x > current_point.x
                        }
                        _ => point.x == current_point.x && point.y < current_point.y,
                    }
                })
                .min_by(|left, right| {
                    let left_point = graph.point(*left);
                    let right_point = graph.point(*right);
                    // Preserve the recovered closure's directional nearest-
                    // neighbor comparisons.  In the Left arm the release
                    // compares the candidate X values in the opposite order
                    // from a conventional `min_by`, so the larger forward X
                    // wins (the same source-shaped predicate as the Go body).
                    if direction == Orientation::Left {
                        left_point.x.total_cmp(&right_point.x)
                    } else {
                        let left_distance = (left_point.x - current_point.x).abs()
                            + (left_point.y - current_point.y).abs();
                        let right_distance = (right_point.x - current_point.x).abs()
                            + (right_point.y - current_point.y).abs();
                        left_distance.total_cmp(&right_distance)
                    }
                })
        }

        let source_index = self.index_of(source)?;
        let mut current = source_index;
        let mut visited = BTreeSet::from([current]);
        let mut path = vec![source_index];
        let walk = |current: &mut usize,
                    destination: Point,
                    direction: Orientation,
                    path: &mut Vec<usize>,
                    visited: &mut BTreeSet<usize>| {
            // routeInSShape compares point coordinates, not OVG identity.
            // connectPortsToCenter can publish a container center at a midpoint's
            // coordinates, replacing OccupiedPoints with a distinct node.
            // The connected intersection still completes this route leg.
            while self.point(*current) != destination {
                let next = next_in_direction(self, *current, direction)?;
                if !visited.insert(next) {
                    return None;
                }
                *current = next;
                path.push(*current);
            }
            Some(())
        };
        walk(
            &mut current,
            source_midpoint,
            orientation,
            &mut path,
            &mut visited,
        )?;
        let cross_direction = if orientation.is_horizontal() {
            if source.y < target.y {
                Orientation::Top
            } else {
                Orientation::Bottom
            }
        } else if source.x < target.x {
            Orientation::Left
        } else {
            Orientation::Right
        };
        walk(
            &mut current,
            target_midpoint,
            cross_direction,
            &mut path,
            &mut visited,
        )?;
        walk(&mut current, target, orientation, &mut path, &mut visited)?;
        Some(path)
    }

    pub(super) fn contains_point(&self, point: Point) -> bool {
        self.index_of(point).is_some()
    }

    fn connect(
        edges: &mut [Vec<(usize, f64, Axis)>],
        points: &[Point],
        first: usize,
        second: usize,
        axis: Axis,
    ) {
        if first == second {
            return;
        }
        let distance =
            (points[first].x - points[second].x).abs() + (points[first].y - points[second].y).abs();
        edges[first].push((second, distance, axis));
        edges[second].push((first, distance, axis));
    }

    /// Source-shaped subset of recovered `NewOVGFromGraph`:
    ///
    /// * add every ordinary port;
    /// * add legal X/Y port-axis intersections;
    /// * add three 20-unit boundary layers and port connections to them;
    /// * connect nearest visible nodes sharing an axis.
    pub(super) fn from_ports(
        graph: &ArenaGraph,
        boxes: &BTreeMap<NodeId, Rect>,
        graph_nodes: &[NodeId],
        tree_node_order: &[NodeId],
        all_ports: impl Iterator<Item = (NodeId, Port)>,
    ) -> Self {
        Self::from_ports_checked(graph, boxes, graph_nodes, tree_node_order, all_ports, None)
            .expect("unbudgeted OVG construction cannot fail")
    }

    /// Build an OVG while enforcing v0.9.0's independent construction
    /// resources. The completed graph is returned only after the local build
    /// has passed its candidate/node/edge/work reservations.
    pub(super) fn from_ports_checked(
        graph: &ArenaGraph,
        boxes: &BTreeMap<NodeId, Rect>,
        graph_nodes: &[NodeId],
        tree_node_order: &[NodeId],
        all_ports: impl Iterator<Item = (NodeId, Port)>,
        budget: Option<&OvgBuildBudget>,
    ) -> Result<Self, String> {
        let box_order = ordered_box_ids(graph_nodes, boxes);
        Self::from_ports_ordered_checked(
            graph,
            boxes,
            &box_order,
            graph_nodes,
            tree_node_order,
            all_ports,
            budget,
            None,
            None,
        )
    }

    /// `box_order` is TALA's `OVG.NodesInsideBoundingBox` order: nearby
    /// nodes, routed-scope nodes, then hierarchy members merged by source ID.
    pub(super) fn from_ports_ordered_checked(
        graph: &ArenaGraph,
        boxes: &BTreeMap<NodeId, Rect>,
        box_order: &[NodeId],
        graph_nodes: &[NodeId],
        tree_node_order: &[NodeId],
        all_ports: impl Iterator<Item = (NodeId, Port)>,
        budget: Option<&OvgBuildBudget>,
        prepared_ports: Option<&mut super::ovg_ports::PortBuild>,
        hierarchies: Option<&[super::ovg_hierarchy::HierarchyOvg]>,
    ) -> Result<Self, String> {
        #[cfg(feature = "diagnostic-traces")]
        crate::internal::call_counts::count!(VisibilityBuild);
        let box_order = ordered_box_ids(box_order, boxes);
        let mut ports = all_ports.collect::<Vec<_>>();
        #[cfg(feature = "diagnostic-traces")]
        macro_rules! trace_ovg_stage {
            ($stage:expr, $points:expr, $edges:expr) => {
                if std::env::var("D2_TRACE_OVG_STAGE_SCOPE").ok().as_deref()==Some(format!("{}:{}",graph.node_order.len(),graph.edge_order.len()).as_str()) {
                    eprintln!("{}",serde_json::json!({"event":"ovg-construction-counts","stage":$stage,"scope_nodes":graph.node_order.len(),"scope_edges":graph.edge_order.len(),"nodes":$points.len(),"edges":$edges}));
                }
                if crate::internal::trace_env_enabled("TALA_TRACE_OVG_STAGES") {
                    eprintln!(
                        "OVG_STAGE_RUST stage={} nodes={} inside={} ports={} edges={} probe1951={} probe1952={}",
                        $stage,
                        $points.len(),
                        graph_nodes.len(),
                        ports.len(),
                        $edges,
                        $points.iter().any(|point| point.x == 1951.0 && point.y == 1404.0),
                        $points.iter().any(|point| point.x == 1952.0 && point.y == 1404.0)
                    );
                    if crate::internal::trace_env_enabled("TALA_TRACE_OVG_POINTS") {
                        for (index, point) in $points.iter().enumerate() {
                            eprintln!(
                                "OVG_POINT_RUST stage={} index={} x={:.17} y={:.17}",
                                $stage, index, point.x, point.y
                            );
                        }
                    }
                }
            };
        }
        #[cfg(not(feature = "diagnostic-traces"))]
        macro_rules! trace_ovg_stage {
            ($stage:expr, $points:expr, $edges:expr) => {
                ()
            };
        }
        let charge = |result: Result<(), ()>| {
            result.map_err(|_| "TALA OVG resource limit exceeded".to_owned())
        };
        // NewOVGFromGraph builds the ordinary OVG first (`addPorts`), then
        // merges each hierarchy OVG with unchecked nodes. Preserve that stage
        // partition before interning coordinates; otherwise a hierarchy
        // owner can claim a coincident point first and force an ordinary owner
        // into a spurious duplicate vertex.
        ports.sort_by_key(|(owner, _)| graph.nodes[owner.0 as usize].hierarchy.is_some());
        let mut ordinary_ports = ports
            .iter()
            .copied()
            // NewOVGFromGraph starts with non-hierarchy ports, then merges
            // hierarchy ports before addNodesIntersections. This flattened
            // inventory therefore includes every non-tunnel port; hierarchy
            // non-port obstacles are handled separately by the box scan.
            .filter(|(_, port)| port.tunnel.is_none())
            .collect::<Vec<_>>();
        let mut ports_by_node = BTreeMap::<NodeId, Vec<Port>>::new();
        for (owner, port) in ordinary_ports.iter().copied() {
            ports_by_node.entry(owner).or_default().push(port);
        }
        // `addNodesIntersections` and `addEdgesNodes` use the current split
        // graph's proximity index. Keep this scope-local even though the
        // arena retains nodes from other split graphs.
        let non_container_boxes = graph_nodes
            .iter()
            .filter_map(|node_id| {
                let node = &graph.nodes[node_id.0 as usize];
                (!node.is_container).then(|| {
                    node.position.map(|origin| Rect {
                        origin,
                        size: node.rect.size,
                    })
                })
            })
            .flatten()
            .collect::<Vec<_>>();
        let point_is_near_non_container = |point| {
            non_container_boxes
                .iter()
                .any(|rect| point_near_rect(point, *rect, 20.0))
        };
        // Recovered OVGNode.hasUnobstructedLineToPorts and
        // connectNodesOnSameLine share Graph.getFixedOverlaps. A fixed node
        // that overlaps another fixed node is omitted from the obstruction
        // scan; only unrelated geometry blocks a candidate line.
        let fixed_overlaps = fixed_overlaps_for_routing(graph, boxes, &box_order);
        let obstruction_nodes = graph_nodes
            .iter()
            .filter_map(|other| {
                boxes.get(other).copied().map(|rect| {
                    let node = &graph.nodes[other.0 as usize];
                    (
                        *other,
                        rect,
                        node.sequence,
                        node.is_container,
                        fixed_overlaps.contains(other),
                    )
                })
            })
            .collect::<Vec<_>>();
        let mut tunnel_points = BTreeMap::<usize, Vec<Point>>::new();
        let mut tunnel_owners = BTreeMap::<usize, BTreeSet<NodeId>>::new();
        for (owner, port) in &ports {
            if let Some(tunnel) = port.tunnel {
                tunnel_points.entry(tunnel).or_default().push(port.point);
                tunnel_owners.entry(tunnel).or_default().insert(*owner);
            }
        }
        let native_tunnels = prepared_ports.is_some();
        let mut imported_ports = super::ovg_ports::PortBuild::default();
        let intersections = if let Some(prepared) = prepared_ports {
            prepared
        } else {
            charge(imported_ports.import_ports(&ordinary_ports, budget))?;
            &mut imported_ports
        };
        trace_ovg_stage!(
            "after-addPorts",
            intersections
                .member_nodes()
                .map(|n| n.point())
                .collect::<Vec<_>>(),
            0
        );
        if let Some(hierarchies) = hierarchies {
            for hierarchy in hierarchies {
                if let Some(b) = budget {
                    charge(b.reserve_work(1))?;
                }
                charge(intersections.merge_hierarchy_ports(
                    &hierarchy.ports,
                    hierarchy.nodes.len(),
                    budget,
                ))?;
            }
        }
        charge(intersections.add_nodes_intersections(graph, graph_nodes, budget))?;

        let mut points = intersections
            .member_nodes()
            .map(|node| node.point())
            .collect::<Vec<_>>();
        if crate::internal::trace_env_enabled("TALA_TRACE_OVG_COUNTS") {
            eprintln!(
                "OVG_COUNTS_RUST intersections_total={} newly_added={}",
                points.len(),
                points.len().saturating_sub(ordinary_ports.len()),
            );
        }
        trace_ovg_stage!("after-addNodesIntersections", points, 0);
        // NewOVGFromGraph merges hierarchy OVG non-port nodes immediately
        // after the ordinary intersection pass. The later edge-fill scan
        // must see those canonical vertices and skip same-hierarchy edges.
        let mut rejected_hierarchy_points = BTreeSet::new();
        if let Some(hierarchies) = hierarchies {
            for hierarchy in hierarchies {
                if let Some(b) = budget {
                    charge(b.reserve_work(1))?;
                }
                charge(intersections.merge_hierarchy_nonports(&hierarchy.ports, budget))?;
            }
            points = intersections.member_nodes().map(|n| n.point()).collect();
            trace_ovg_stage!("after-mergeNonPorts", points, 0);
        } else {
            add_hierarchy_level_points(
                graph,
                graph_nodes,
                boxes,
                &ports_by_node,
                &mut points,
                |hierarchy_nodes, point| {
                    if point_is_near_non_container(point) {
                        return false;
                    }
                    let local_ports = hierarchy_nodes
                        .iter()
                        .flat_map(|node| {
                            ports_by_node
                                .get(node)
                                .into_iter()
                                .flatten()
                                .copied()
                                .map(|port| (*node, port))
                        })
                        .collect::<Vec<_>>();
                    if local_ports.iter().any(|(_, port)| {
                        (port.point.y == point.y && (port.point.x - point.x).abs() <= 20.0)
                            || (port.point.x == point.x && (port.point.y - point.y).abs() <= 20.0)
                    }) {
                        return false;
                    }
                    let local_visibility = local_ports
                        .iter()
                        .filter_map(|(owner, port)| {
                            boxes.get(owner).copied().map(|owner_box| {
                                let node = &graph.nodes[owner.0 as usize];
                                PortVisibilityInfo {
                                    owner: *owner,
                                    port: *port,
                                    owner_box,
                                    owner_sequence: node.sequence,
                                    owner_invisible: node.is_invisible,
                                }
                            })
                        })
                        .collect::<Vec<_>>();
                    has_unobstructed_line_to_ports_with_obstructions(
                        point,
                        &local_visibility,
                        &obstruction_nodes,
                        2,
                    )
                },
                &mut rejected_hierarchy_points,
            );
            trace_ovg_stage!("after-mergeNonPorts", points, 0);
            // The legacy hierarchy merge already materialized these points.
            // Register occupancy without pretending its allocations were guarded;
            // later accounting still reserves those uncharged surviving nodes.
            for &point in &points {
                intersections
                    .add_point(point, None)
                    .expect("unguarded point insertion");
            }
        }
        let first_edge_node = intersections.members.len();
        charge(intersections.add_edge_nodes(graph, graph_nodes, budget))?;
        points.extend(
            intersections.members[first_edge_node..]
                .iter()
                .map(|&node| intersections.nodes[node].point()),
        );
        trace_ovg_stage!("after-addEdgesNodes", points, 0);
        // Preserve canonical owner/arrowhead identities and source allocation
        // order through the tree stage. Later legacy passes consume its view.
        let first_tree_node = intersections.members.len();
        let added_tree_ports = intersections
            .add_tree_nodes(graph, tree_node_order, budget)
            .map_err(|_| "TALA OVG resource limit exceeded".to_owned())?;
        points.extend(
            intersections.members[first_tree_node..]
                .iter()
                .map(|&node| intersections.nodes[node].point()),
        );
        for (&owner, added) in &added_tree_ports {
            for &port in added {
                ports.push((owner, port));
                ordinary_ports.push((owner, port));
                ports_by_node.entry(owner).or_default().push(port);
            }
        }
        trace_ovg_stage!("after-addTreeNodes", points, 0);
        let bounds = super::edge_nodes::tight_bounds(graph, &box_order, budget)
            .map_err(|_| "TALA OVG resource limit exceeded".to_owned())?;
        let first_boundary_node = intersections.members.len();
        charge(intersections.add_boundary_layers(graph, graph_nodes, bounds, budget))?;
        points.extend(
            intersections.members[first_boundary_node..]
                .iter()
                .map(|&node| intersections.nodes[node].point()),
        );
        trace_ovg_stage!("after-addNewBoundaryLayers", points, 0);
        let first_boundary_node = intersections.members.len();
        charge(intersections.add_port_boundary_nodes(
            graph,
            graph_nodes,
            &box_order,
            bounds,
            budget,
        ))?;
        points.extend(
            intersections.members[first_boundary_node..]
                .iter()
                .map(|&node| intersections.nodes[node].point()),
        );
        trace_ovg_stage!("after-addPortConnectionNodesAtBoundaries", points, 0);
        let first_corner = intersections.members.len();
        charge(intersections.add_corners(graph, graph_nodes, bounds, budget))?;
        points.extend(
            intersections.members[first_corner..]
                .iter()
                .map(|&node| intersections.nodes[node].point()),
        );
        trace_ovg_stage!("after-addCornerNodes", points, 0);
        // Build detached tunnels at the source stage, then publish canonical
        // nodes/metadata/edges immediately. Explicit from_ports test callers
        // retain their supplied tunnel inventory.
        let mut ordered_tunnel_ids = Vec::<usize>::new();
        if native_tunnels {
            let added = intersections
                .add_tunnels(graph, graph_nodes, budget)
                .map_err(|_| "TALA OVG resource limit exceeded".to_owned())?;
            ports.extend(added);
        } else {
            let mut seen_tunnel_ids = BTreeSet::new();
            for &node_id in &graph.node_order {
                for edge_id in graph.active_edge_ids(node_id) {
                    let edge = &graph.edge_records[edge_id.0 as usize];
                    let other = if edge.from == node_id {
                        edge.to
                    } else {
                        edge.from
                    };
                    for (id, owners) in &tunnel_owners {
                        if owners.contains(&node_id)
                            && owners.contains(&other)
                            && seen_tunnel_ids.insert(*id)
                        {
                            ordered_tunnel_ids.push(*id);
                        }
                    }
                }
            }
            for id in tunnel_points.keys() {
                if seen_tunnel_ids.insert(*id) {
                    ordered_tunnel_ids.push(*id);
                }
            }
            let mut supplied = Vec::new();
            for id in &ordered_tunnel_ids {
                let endpoints = ports
                    .iter()
                    .filter(|(_, port)| port.tunnel == Some(*id))
                    .map(|(owner, port)| super::ovg_tunnels::Entry {
                        owner: *owner,
                        point: port.point,
                        side: port.side,
                    })
                    .collect::<Vec<_>>();
                if let [first, second] = endpoints.as_slice() {
                    supplied.push([*first, *second]);
                }
            }
            ports.retain(|(_, port)| port.tunnel.is_none());
            ports.extend(
                intersections
                    .append_tunnels(supplied, budget)
                    .map_err(|_| "TALA OVG resource limit exceeded".to_owned())?,
            );
        }
        points = intersections
            .member_nodes()
            .map(|node| node.point())
            .collect();
        if crate::internal::trace_env_enabled("TALA_TRACE_DUMP_OVG_POINTS") {
            eprintln!("OVG_POINTS_RUST_BEGIN n={}", points.len());
            for (index, point) in points.iter().enumerate() {
                eprintln!("OVG_POINTS_RUST {} {:.17} {:.17}", index, point.x, point.y);
            }
            eprintln!("OVG_POINTS_RUST_END");
        }
        trace_ovg_stage!("after-addTunnels", points, intersections.edges.len());
        intersections.connect_nodes(graph, &box_order, budget)?;
        trace_ovg_stage!("after-connectNodes", points, intersections.edges.len());
        charge(intersections.connect_centers(graph, &box_order, budget))?;
        trace_ovg_stage!(
            "after-connectPortsToCenter",
            intersections
                .member_nodes()
                .map(|node| node.point())
                .collect::<Vec<_>>(),
            intersections.edges.len()
        );
        charge(intersections.remove_isolated(budget))?;
        trace_ovg_stage!(
            "after-removeIsolatedNodes",
            intersections
                .member_nodes()
                .map(|node| node.point())
                .collect::<Vec<_>>(),
            intersections.edges.len()
        );

        charge(intersections.map_containers(graph, budget))?;
        charge(intersections.flag_near_ports(budget))?;
        charge(intersections.assign_indices(budget))?;

        // Search storage retains neighbors independently of the source Nodes slice.
        intersections.retain_incident_objects();
        let mut objects = intersections.members.clone();
        let mut member_indices = SlotIndices::default();
        for (index, &node) in objects.iter().enumerate() {
            member_indices.insert(node, index);
        }
        for node in 0..intersections.nodes.len() {
            if !member_indices.contains(node) {
                member_indices.insert(node, objects.len());
                objects.push(node);
            }
        }
        points = objects
            .iter()
            .map(|&node| intersections.nodes[node].point())
            .collect();
        // retain_canonical's refresh republishes the search tables (edges,
        // indices, centers, containers, owners) from the canonical build, so
        // only the topology trace still needs this provisional edge table.
        #[cfg(feature = "diagnostic-traces")]
        let edges = objects
            .iter()
            .map(|&node| {
                let header = intersections.nodes[node].read().edges.clone();
                header
                    .iter_slots()
                    .map(|edge_id| {
                        let edge_id = edge_id.expect(
                            "runtime error: invalid memory address or nil pointer dereference",
                        );
                        let adjacent = edge_id.adjacent(&intersections.nodes[node]);
                        let other = intersections.object_slot(&adjacent);
                        let record = edge_id.record();
                        let edge = record.read();
                        let first = intersections.nodes[node].point();
                        let second = intersections.nodes[other].point();
                        let axis = if first.x == second.x {
                            Axis::Vertical
                        } else if first.y == second.y {
                            Axis::Horizontal
                        } else {
                            Axis::Vertical
                        };
                        (member_indices[&other], edge.distance, axis)
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let tunnel_point_keys = objects
            .iter()
            .map(|&node| &intersections.nodes[node])
            .filter(|node| node.is_tunnel())
            .map(|node| point_key(node.point()))
            .collect::<BTreeSet<_>>();
        // These inventories only construct the numerical preview above.
        // refresh_canonical publishes its own dense slot table; retaining the
        // preview while that table is built stacks both allocations at peak.
        drop(member_indices);
        drop(objects);
        if let Some(budget) = budget
            && intersections.unreserved_member_count() != 0
        {
            // Legacy hierarchy imports are the only surviving memberships
            // not yet charged at publication. Centers, including removed ones,
            // were already charged by their source-shaped construction stage.
            // An absent import has no source reservation: even reserve_nodes(0)
            // would add an observable caller-context check after assign_indices.
            charge(budget.reserve_nodes(intersections.unreserved_member_count() as u64))?;
        }
        #[cfg(feature = "diagnostic-traces")]
        if crate::internal::trace_env_enabled("TALA_TRACE_OVG_JSONL") {
            // Go's encoding/json writes an integral float64 as a JSON integer
            // (`686`), while serde_json otherwise emits Rust's `686.0`.
            // Keep this diagnostic wire form identical without changing the
            // routing values or release path.
            let trace_number = |value: f64| {
                if value.is_finite() && value.fract() == 0.0 {
                    serde_json::json!(value as i64)
                } else {
                    serde_json::json!(value)
                }
            };
            let ordered_boxes = box_order
                .iter()
                .filter_map(|node| boxes.get(node).map(|rect| (*node, *rect)))
                .collect::<Vec<_>>();
            eprintln!(
                "{}",
                serde_json::json!({
                    "event": "ovg-topology",
                    "scope": graph_nodes.iter().map(|node| graph.nodes[node.0 as usize].tala_id).collect::<Vec<_>>(),
                    "boxes": ordered_boxes.iter().map(|(node, rect)| serde_json::json!({"id":graph.nodes[node.0 as usize].tala_id,"x":trace_number(rect.origin.x),"y":trace_number(rect.origin.y),"w":trace_number(rect.size.width),"h":trace_number(rect.size.height)})).collect::<Vec<_>>(),
                    "points": points.iter().enumerate().take(intersections.members.len()).map(|(index, point)| serde_json::json!({"x":trace_number(point.x),"y":trace_number(point.y),"neighbors":edges[index].iter().map(|(next,_,_)| [trace_number(points[*next].x),trace_number(points[*next].y)]).collect::<Vec<_>>()})).collect::<Vec<_>>(),
                })
            );
        }
        let mut result = Self {
            canonical: None,
            points,
            edges: Adjacency::default(),
            search_indices: Vec::new(),
            centers: BTreeMap::new(),
            center_owners: Vec::new(),
            point_containers: Vec::new(),
            port_owners: OwnerSets::default(),
            near_port_owners: OwnerSets::default(),
            rejected_hierarchy_points,
            tunnel_points: tunnel_point_keys,
            added_tree_ports,
        };
        // The caller reads the prepared build's ports back from the canonical
        // view, so hand the build over instead of keeping a second copy.
        result.retain_canonical(intersections.take_as_clone());
        Ok(result)
    }

    pub(super) fn route(
        &self,
        graph: &ArenaGraph,
        source_port: Port,
        target_port: Port,
        source_node: NodeId,
        target_node: NodeId,
    ) -> Vec<Point> {
        let interaction_index = super::search::RouteInteractionIndex::new(self, graph);
        self.route_scored(
            graph,
            source_port,
            target_port,
            source_node,
            target_node,
            None,
            &interaction_index,
            &BTreeMap::new(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn route_scored(
        &self,
        graph: &ArenaGraph,
        source_port: Port,
        target_port: Port,
        source_node: NodeId,
        target_node: NodeId,
        edge_index: Option<usize>,
        interaction_index: &super::search::RouteInteractionIndex,
        boxes: &BTreeMap<NodeId, Rect>,
    ) -> Vec<Point> {
        self.route_to_any(
            graph,
            source_port,
            &[(target_port, 0.0)],
            source_node,
            target_node,
            edge_index,
            interaction_index,
            boxes,
        )
        .map_or_else(Vec::new, |route| route.points)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn route_to_any(
        &self,
        graph: &ArenaGraph,
        source_port: Port,
        target_ports: &[(Port, f64)],
        source_node: NodeId,
        target_node: NodeId,
        edge_index: Option<usize>,
        interaction_index: &super::search::RouteInteractionIndex,
        boxes: &BTreeMap<NodeId, Rect>,
    ) -> Option<AnyTargetRoute> {
        self.route_to_any_with_budget(
            graph,
            source_port,
            target_ports,
            source_node,
            target_node,
            edge_index,
            interaction_index,
            boxes,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn route_to_any_with_budget(
        &self,
        graph: &ArenaGraph,
        source_port: Port,
        target_ports: &[(Port, f64)],
        source_node: NodeId,
        target_node: NodeId,
        edge_index: Option<usize>,
        interaction_index: &super::search::RouteInteractionIndex,
        boxes: &BTreeMap<NodeId, Rect>,
        budget: Option<&LocalRouteSearchWork>,
    ) -> Option<AnyTargetRoute> {
        self.route_between_any_with_budget(
            graph,
            &[(source_port, 0.0)],
            target_ports,
            source_node,
            target_node,
            edge_index,
            interaction_index,
            boxes,
            false,
            budget,
        )
        .map(|mut route| {
            // The historical single-source API starts at the source port,
            // whereas TALA's search includes the unit source-center edge.
            route.cost -= 1.0;
            route
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn route_between_any(
        &self,
        graph: &ArenaGraph,
        source_ports: &[(Port, f64)],
        target_ports: &[(Port, f64)],
        source_node: NodeId,
        target_node: NodeId,
        edge_index: Option<usize>,
        interaction_index: &super::search::RouteInteractionIndex,
        boxes: &BTreeMap<NodeId, Rect>,
        overlap: bool,
    ) -> Option<AnyTargetRoute> {
        self.route_between_any_with_budget(
            graph,
            source_ports,
            target_ports,
            source_node,
            target_node,
            edge_index,
            interaction_index,
            boxes,
            overlap,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn route_between_any_with_budget(
        &self,
        graph: &ArenaGraph,
        source_ports: &[(Port, f64)],
        target_ports: &[(Port, f64)],
        source_node: NodeId,
        target_node: NodeId,
        edge_index: Option<usize>,
        interaction_index: &super::search::RouteInteractionIndex,
        boxes: &BTreeMap<NodeId, Rect>,
        overlap: bool,
        budget: Option<&LocalRouteSearchWork>,
    ) -> Option<AnyTargetRoute> {
        let source_costs = source_ports
            .iter()
            .map(|(port, penalty)| (*port, 1.0 + penalty))
            .collect::<Vec<_>>();
        let target_costs = target_ports
            .iter()
            .map(|(port, penalty)| (*port, 1.0 + penalty))
            .collect::<Vec<_>>();
        self.route_between_connectors_with_budget(
            graph,
            &source_costs,
            &target_costs,
            source_node,
            target_node,
            edge_index,
            interaction_index,
            boxes,
            overlap,
            budget,
        )
    }

    /// Candidates carry complete center/port connector costs, accumulated in
    /// upstream operation order; do not decompose and reconstruct those floats.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn route_between_connectors_with_budget(
        &self,
        graph: &ArenaGraph,
        source_ports: &[(Port, f64)],
        target_ports: &[(Port, f64)],
        source_node: NodeId,
        target_node: NodeId,
        edge_index: Option<usize>,
        interaction_index: &super::search::RouteInteractionIndex,
        boxes: &BTreeMap<NodeId, Rect>,
        overlap: bool,
        budget: Option<&LocalRouteSearchWork>,
    ) -> Option<AnyTargetRoute> {
        let mut prepared = self.prepare_route_search(budget)?;
        if let Some(index) = edge_index {
            prepared.prepare_arrowhead_nodes(graph, index, budget)?;
        }
        prepared.prepare_ancestry(graph, source_node, target_node, budget)?;
        prepared.prepare_blocked_ports(
            graph,
            source_node,
            target_node,
            boxes,
            source_ports.len(),
            target_ports.len(),
            edge_index.is_some_and(|index| graph.edge_records[index].has_table_column()),
            budget,
        )?;
        self.route_between_connectors_with_prepared_state(
            graph,
            source_ports,
            target_ports,
            source_node,
            target_node,
            edge_index,
            interaction_index,
            boxes,
            overlap,
            budget,
            prepared,
        )
    }

    pub(super) fn prepare_route_search(
        &self,
        budget: Option<&LocalRouteSearchWork>,
    ) -> Option<PreparedRouteSearch> {
        let node_count = self.member_count();
        if let Some(budget) = budget {
            budget.reserve_product(node_count as u64, 3).ok()?;
        }
        let slots = take_search_slots(node_count);
        Some(PreparedRouteSearch {
            ancestry: None,
            arrowhead_nodes: Vec::new(),
            blocked_ports_prepared: false,
            symmetry_prepared: false,
            fixed_overlaps: None,
            blocked_source: BTreeSet::new(),
            blocked_target: BTreeSet::new(),
            slots,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn route_between_connectors_with_prepared_state(
        &self,
        graph: &ArenaGraph,
        source_ports: &[(Port, f64)],
        target_ports: &[(Port, f64)],
        source_node: NodeId,
        target_node: NodeId,
        edge_index: Option<usize>,
        interaction_index: &super::search::RouteInteractionIndex,
        boxes: &BTreeMap<NodeId, Rect>,
        overlap: bool,
        budget: Option<&LocalRouteSearchWork>,
        prepared: PreparedRouteSearch,
    ) -> Option<AnyTargetRoute> {
        #[cfg(feature = "diagnostic-traces")]
        crate::internal::call_counts::count!(RouteSearch);
        self.route_between_any_group(
            graph,
            source_ports,
            target_ports,
            source_node,
            target_node,
            edge_index,
            interaction_index,
            boxes,
            overlap,
            budget,
            prepared,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn route_between_any_group(
        &self,
        graph: &ArenaGraph,
        source_ports: &[(Port, f64)],
        target_ports: &[(Port, f64)],
        source_node: NodeId,
        target_node: NodeId,
        edge_index: Option<usize>,
        interaction_index: &super::search::RouteInteractionIndex,
        boxes: &BTreeMap<NodeId, Rect>,
        overlap: bool,
        budget: Option<&LocalRouteSearchWork>,
        prepared: PreparedRouteSearch,
    ) -> Option<AnyTargetRoute> {
        // Symmetry inventory still reserves its source sum here. Non-table
        // blocked maps were already computed before usedPorts by the caller.
        if let Some(budget) = budget {
            if !prepared.symmetry_prepared {
                budget
                    .reserve_sum(source_ports.len() as u64, target_ports.len() as u64)
                    .ok()?;
            }
            if !prepared.blocked_ports_prepared {
                budget
                    .reserve_product(source_ports.len() as u64, target_ports.len() as u64)
                    .ok()?;
            }
        }
        // Go searches the retained Ports nodes. OccupiedPoints can later be
        // overwritten by a coincident center or imported route node.
        let mut sources = crate::internal::IdHashMap::<usize, Vec<(Port, f64)>>::default();
        for (port, penalty) in source_ports {
            if let Some(index) = self.port_index(source_node, port.point) {
                sources.entry(index).or_default().push((*port, *penalty));
            }
        }
        if sources.is_empty() {
            return None;
        }
        let mut targets = crate::internal::IdHashMap::<usize, Vec<(Port, f64)>>::default();
        for (port, penalty) in target_ports {
            if let Some(index) = self.port_index(target_node, port.point) {
                targets.entry(index).or_default().push((*port, *penalty));
            }
        }
        if targets.is_empty() {
            return None;
        }
        // The handful of target port nodes, for the per-hop membership test.
        let target_port_nodes = targets.keys().copied().collect::<Vec<_>>();
        let source_index = *self.centers.get(&source_node)?;
        let target_index = *self.centers.get(&target_node)?;
        let search_node_count = self.member_count();
        let source_container = graph.nodes[source_node.0 as usize].container;
        let target_container = graph.nodes[target_node.0 as usize].container;
        let axes = super::search::ideal_turn_axes_for_slingshot(
            boxes.get(&source_node).copied().unwrap_or(Rect {
                origin: Point::default(),
                size: graph.nodes[source_node.0 as usize].rect.size,
            }),
            boxes.get(&target_node).copied().unwrap_or(Rect {
                origin: Point::default(),
                size: graph.nodes[target_node.0 as usize].rect.size,
            }),
        );
        let mut prepared = prepared;
        let ancestry = prepared.ancestry;
        let arrowhead_nodes = std::mem::take(&mut prepared.arrowhead_nodes);
        let blocked_source = std::mem::take(&mut prepared.blocked_source);
        let blocked_target = std::mem::take(&mut prepared.blocked_target);
        let fixed_overlaps = prepared.fixed_overlaps.take();
        let mut search = SearchState::new(std::mem::take(&mut prepared.slots));
        drop(prepared);
        let SearchState {
            slots,
            touched,
            queue,
        } = &mut search;
        let (source_descendant_of_target, target_descendant_of_source) = ancestry?;
        debug_assert_eq!(slots.len(), search_node_count);
        let search_nodes = self.search_nodes();
        // Empty blocked maps reject no point; skip their per-hop key.
        let any_blocked = !blocked_source.is_empty() || !blocked_target.is_empty();
        #[cfg(feature = "diagnostic-traces")]
        let trace_heap = |operation: &str,
                          node: usize,
                          axis: Axis,
                          priority: f64,
                          old_priority: Option<f64>,
                          queue_len: usize| {
            if let (Some(flavor), Some(edge_index)) = (&interaction_index.trace_flavor, edge_index)
                && ((std::env::var("D2_TRACE_SEARCH_HEAP").as_deref() == Ok("1")
                    && std::env::var("D2_TRACE_SEARCH_FLAVOR").as_deref() == Ok(flavor.as_str()))
                    || (crate::internal::trace_env_value("TALA_TRACE_SEARCH_HEAP").as_deref()
                        == Some("1")
                        && crate::internal::trace_env_value("TALA_TRACE_SEARCH_FLAVOR").as_deref()
                            == Some(flavor.as_str())))
                && std::env::var("D2_TRACE_SEARCH_EDGE")
                    .ok()
                    .is_none_or(|filter| {
                        filter.is_empty()
                            || filter == graph.edge_records[edge_index].tala_id.to_string()
                    })
            {
                eprintln!(
                    "{}",
                    serde_json::json!({
                        "event": "route-search-heap", "flavor": flavor,
                        "edge": graph.edge_records[edge_index].tala_id,
                        "work": budget.map(LocalRouteSearchWork::used),
                        "scope_nodes": graph.node_order.len(), "scope_edges": graph.edge_order.len(),
                        "operation": operation,
                        "x_bits": format!("0x{:016x}", self.point(node).x.to_bits()),
                        "y_bits": format!("0x{:016x}", self.point(node).y.to_bits()),
                        "horizontal": matches!(axis, Axis::Horizontal),
                        "priority_bits": format!("0x{:016x}", priority.to_bits()),
                        "old_priority_bits": old_priority.map(|value| format!("0x{:016x}", value.to_bits())),
                        "queue_len": queue_len,
                    })
                );
            }
        };
        queue_push(
            queue,
            budget,
            0.0,
            QueueState {
                node: source_index,
                axis: Axis::Vertical,
            },
        )?;
        #[cfg(feature = "diagnostic-traces")]
        trace_heap("push", source_index, Axis::Vertical, 0.0, None, queue.len());
        queue_push(
            queue,
            budget,
            0.0,
            QueueState {
                node: source_index,
                axis: Axis::Horizontal,
            },
        )?;
        #[cfg(feature = "diagnostic-traces")]
        trace_heap(
            "push",
            source_index,
            Axis::Horizontal,
            0.0,
            None,
            queue.len(),
        );

        #[cfg(feature = "diagnostic-traces")]
        let trace_neighbor = |from: usize, to: usize, axis: Axis| {
            if let (Some(flavor), Some(edge_index)) = (&interaction_index.trace_flavor, edge_index)
                && crate::internal::trace_env_value("TALA_TRACE_SEARCH_FLAVOR").as_deref()
                    == Some(flavor.as_str())
            {
                eprintln!(
                    "{}",
                    serde_json::json!({
                        "event": "route-search-neighbor", "flavor": flavor,
                        "edge": graph.edge_records[edge_index].tala_id,
                        "from_x_bits": format!("0x{:016x}", self.point(from).x.to_bits()),
                        "from_y_bits": format!("0x{:016x}", self.point(from).y.to_bits()),
                        "to_x_bits": format!("0x{:016x}", self.point(to).x.to_bits()),
                        "to_y_bits": format!("0x{:016x}", self.point(to).y.to_bits()),
                        "horizontal": matches!(axis, Axis::Horizontal),
                    })
                );
            }
        };
        #[cfg(feature = "diagnostic-traces")]
        let trace_cost =
            |from: usize, to: usize, axis: Axis, candidate: f64, previous: f64, has_entry: bool| {
                if let (Some(flavor), Some(edge_index)) =
                    (&interaction_index.trace_flavor, edge_index)
                    && crate::internal::trace_env_value("TALA_TRACE_SEARCH_FLAVOR").as_deref()
                        == Some(flavor.as_str())
                {
                    let decision = if candidate < previous {
                        if has_entry { "decrease" } else { "push" }
                    } else {
                        "reject"
                    };
                    eprintln!(
                        "{}",
                        serde_json::json!({
                            "event": "route-search-cost", "flavor": flavor,
                            "edge": graph.edge_records[edge_index].tala_id,
                            "from_x_bits": format!("0x{:016x}", self.point(from).x.to_bits()),
                            "from_y_bits": format!("0x{:016x}", self.point(from).y.to_bits()),
                            "to_x_bits": format!("0x{:016x}", self.point(to).x.to_bits()),
                            "to_y_bits": format!("0x{:016x}", self.point(to).y.to_bits()),
                            "horizontal": matches!(axis, Axis::Horizontal),
                            "candidate_bits": format!("0x{:016x}", candidate.to_bits()),
                            "previous_bits": format!("0x{:016x}", previous.to_bits()),
                            "decision": decision,
                        })
                    );
                }
            };
        #[cfg(feature = "diagnostic-traces")]
        let trace_cost_breakdown = |from: usize,
                                    to: usize,
                                    axis: Axis,
                                    candidate: f64,
                                    base: f64,
                                    structural: f64,
                                    interaction: f64,
                                    label: f64,
                                    arrow: f64,
                                    turn: f64,
                                    final_cost: f64| {
            if let (Some(flavor), Some(edge_index)) = (&interaction_index.trace_flavor, edge_index)
                && crate::internal::trace_env_value("TALA_TRACE_SEARCH_COSTS").as_deref()
                    == Some("1")
                && crate::internal::trace_env_value("TALA_TRACE_SEARCH_FLAVOR").as_deref()
                    == Some(flavor.as_str())
            {
                eprintln!(
                    "{}",
                    serde_json::json!({
                        "event": "route-search-cost-breakdown", "flavor": flavor,
                        "edge": graph.edge_records[edge_index].tala_id,
                        "from_x_bits": format!("0x{:016x}", self.point(from).x.to_bits()),
                        "from_y_bits": format!("0x{:016x}", self.point(from).y.to_bits()),
                        "to_x_bits": format!("0x{:016x}", self.point(to).x.to_bits()),
                        "to_y_bits": format!("0x{:016x}", self.point(to).y.to_bits()),
                        "horizontal": matches!(axis, Axis::Horizontal),
                        "base_bits": format!("0x{:016x}", base.to_bits()),
                        "structural_bits": format!("0x{:016x}", structural.to_bits()),
                        "interaction_bits": format!("0x{:016x}", interaction.to_bits()),
                        "label_bits": format!("0x{:016x}", label.to_bits()),
                        "arrow_bits": format!("0x{:016x}", arrow.to_bits()),
                        "turn_bits": format!("0x{:016x}", turn.to_bits()),
                        "final_bits": format!("0x{:016x}", final_cost.to_bits()),
                        "candidate_bits": format!("0x{:016x}", candidate.to_bits()),
                    })
                );
            }
        };
        // The turn clearance test reads the same two fixed endpoint boxes
        // for every turning hop.
        let source_box = boxes.get(&source_node).copied();
        let target_box = boxes.get(&target_node).copied();
        let mut reached_target = false;
        let mut best_cost = f64::INFINITY;
        while let Some((entry, entry_cost)) = queue_pop(queue, budget) {
            #[cfg(feature = "diagnostic-traces")]
            trace_heap("pop", entry.node, entry.axis, entry_cost, None, queue.len());
            #[cfg(feature = "diagnostic-traces")]
            if let (Some(flavor), Some(edge_index)) = (&interaction_index.trace_flavor, edge_index)
                && crate::internal::trace_env_value("TALA_TRACE_SEARCH_FLAVOR").as_deref()
                    == Some(flavor.as_str())
            {
                let point = self.point(entry.node);
                eprintln!(
                    "{}",
                    serde_json::json!({
                        "event": "route-search-pop", "flavor": flavor,
                        "edge": graph.edge_records[edge_index].tala_id,
                        "x_bits": format!("0x{:016x}", point.x.to_bits()),
                        "y_bits": format!("0x{:016x}", point.y.to_bits()),
                        "cost_bits": format!("0x{:016x}", entry_cost.to_bits()),
                        "horizontal": matches!(entry.axis, Axis::Horizontal),
                    })
                );
            }
            // Keep the hot search loop cooperative. In ordinary builds this
            // predicate is a const-false diagnostic-free inline, while a
            // caller-owned cancellation token can stop before another heap
            // expansion publishes route state.
            if crate::internal::cancellation_requested() {
                return None;
            }
            if entry.node == target_index {
                best_cost = entry_cost;
                reached_target = true;
                break;
            }
            // The pop's pointToRoute lookup also answers each hop's occupied
            // routes while the entry point's key is unchanged.
            let mut occupied_memo: super::search::OccupiedMemo<'_> = None;
            if let Some(budget) = budget {
                interaction_index.charge_occupied_routes_memo(
                    (entry.node != source_index).then_some(self.point(entry.node)),
                    budget,
                    &mut occupied_memo,
                )?;
            }
            if entry.node == source_index {
                // `connectPortsToCenter` appends the source center's port
                // connectors in graph-port order. Search expands those actual
                // adjacency entries; candidate filtering and its recovered
                // source-port surcharge are applied at the connected point.
                for source in self.edges[source_index].iter().map(|slot| slot.neighbor()) {
                    if let Some(budget) = budget {
                        budget.step().ok()?;
                    }
                    #[cfg(feature = "diagnostic-traces")]
                    trace_neighbor(entry.node, source, entry.axis);
                    if blocked_source.contains(&super::go_point_map_key(self.point(source)))
                        && self.port_owners[source].contains(&source_node)
                    {
                        continue;
                    }
                    let Some(source_ports_at_point) = sources.get(&source) else {
                        continue;
                    };
                    let axis = search_axis(self.point(source_index), self.point(source));
                    // Go merges coincident port aliases into one OVG node.
                    // Keep every alias for direction checks, but evaluate the
                    // connector once using its combined preferred-port cost.
                    if let Some((_port, penalty)) = source_ports_at_point
                        .iter()
                        .min_by(|left, right| left.1.total_cmp(&right.1))
                    {
                        let mut hop_cost = *penalty;
                        // TALA applies the same-scope boundary penalty to the
                        // synthetic center-to-port launch before its ordinary
                        // port scoring. A source center can be inside a child
                        // container while the border port belongs to its
                        // parent shape; preserve that distinction here.
                        if source_container.is_some()
                            && source_container == target_container
                            && !source_descendant_of_target
                            && !target_descendant_of_source
                            && search_nodes[source_index].container
                                != search_nodes[source].container
                            && search_nodes[source].container != source_container
                        {
                            hop_cost += 4.0 * graph.routing_costs.1;
                        }
                        #[cfg(feature = "diagnostic-traces")]
                        let structural_launch_cost = hop_cost;
                        // Go applies positioned/node-label obstacles to every
                        // accepted connector, including center-to-port launches.
                        // Score the actual segment before publishing its queue
                        // state; misses and early label hits retain guard work.
                        if let Some(edge_index) = edge_index {
                            hop_cost = interaction_index.apply_label_obstacle_cost_guarded(
                                graph,
                                edge_index,
                                [self.point(source_index), self.point(source)],
                                hop_cost,
                                budget,
                            )?;
                        }
                        let cost = entry_cost + hop_cost;
                        let source_context = self.search_index(source);
                        #[cfg(feature = "diagnostic-traces")]
                        trace_cost_breakdown(
                            entry.node,
                            source,
                            axis,
                            cost,
                            structural_launch_cost,
                            structural_launch_cost,
                            hop_cost,
                            hop_cost,
                            hop_cost,
                            hop_cost,
                            hop_cost,
                        );
                        #[cfg(feature = "diagnostic-traces")]
                        trace_cost(
                            entry.node,
                            source,
                            axis,
                            cost,
                            slots[source_context].distances[axis.index()],
                            slots[source_context].entries[axis.index()].is_some(),
                        );
                        if cost >= slots[source_context].distances[axis.index()] {
                            continue;
                        }
                        if slots[source_context].distances == INITIAL_SEARCH_SLOT.distances {
                            touched.push(source_context);
                        }
                        slots[source_context].distances[axis.index()] = cost;
                        slots[source_context].previous[axis.index()] =
                            Some(predecessor(source_index));
                        let handle =
                            queue_push(queue, budget, cost, QueueState { node: source, axis })?;
                        slots[source_context].entries[axis.index()] = Some(handle);
                        #[cfg(feature = "diagnostic-traces")]
                        trace_heap("push", source, axis, cost, None, queue.len());
                    }
                }
                continue;
            }
            let entry_node = search_nodes[entry.node];
            let entry_context = entry_node.context();
            let previous_node =
                slots[entry_context].previous[entry.axis.index()].map(predecessor_index);
            // The search-local port inventories, owner sets, containers and
            // canonical edge identities are fixed while this state expands.
            let entry_owners = &self.port_owners[entry.node];
            let entry_ports = sources.get(&entry.node);
            let entry_source_ports = entry_ports.filter(|_| entry_owners.contains(&source_node));
            let entry_container = entry_node.container;
            let entry_edge_slots = self.canonical_adjacency(entry.node);
            for (adjacency, slot) in self.edges[entry.node].iter().enumerate() {
                let (next, length) = (slot.neighbor(), slot.distance);
                if let Some(budget) = budget {
                    budget.step().ok()?;
                }
                // Search never moves OVG points: read each endpoint once per hop,
                // at the first place the hop reads it.
                let entry_point = self.point(entry.node);
                let next_point = self.point(next);
                #[cfg(feature = "diagnostic-traces")]
                trace_neighbor(entry.node, next, entry.axis);
                let axis = search_axis(entry_point, next_point);
                #[cfg(feature = "diagnostic-traces")]
                if crate::internal::trace_env_enabled("TALA_TRACE_SEARCH_FILTERS") {
                    eprintln!(
                        "{}",
                        serde_json::json!({
                            "diagnostic": "search-filter-input",
                            "edge": edge_index.map(|index| graph.edge_records[index].tala_id),
                            "flavor": interaction_index.trace_flavor,
                            "from": [entry_point.x, entry_point.y],
                            "to": [next_point.x, next_point.y],
                            "next": next, "previous": previous_node,
                            "source": source_index, "target": target_index,
                            "source_ports": format!("{:?}", sources.get(&entry.node)),
                            "target_ports": format!("{:?}", targets.get(&next)),
                            "container": search_nodes[next].container.map(|id|graph.nodes[id.0 as usize].tala_id),
                        })
                    );
                }
                // Recovered search never immediately traverses back to the
                // predecessor stored for the dequeued directional state.
                if previous_node == Some(next) {
                    continue;
                }
                let next_node = search_nodes[next];
                let next_owned = next_node.flags & SEARCH_NODE_PORT_OWNED != 0;
                let next_owners = &self.port_owners[next];
                if any_blocked {
                    let next_key = super::go_point_map_key(next_point);
                    if (blocked_source.contains(&next_key)
                        && next_owned
                        && next_owners.contains(&source_node))
                        || (blocked_target.contains(&next_key)
                            && next_owned
                            && next_owners.contains(&target_node))
                    {
                        continue;
                    }
                }
                if next == target_index {
                    let Some(targets_at_point) = targets.get(&entry.node) else {
                        continue;
                    };
                    // Go merges coincident port aliases into one OVG node.
                    // Keep every alias for direction checks, but evaluate the
                    // connector once using its combined preferred-port cost.
                    if let Some((_port, penalty)) = targets_at_point
                        .iter()
                        .min_by(|left, right| left.1.total_cmp(&right.1))
                    {
                        // Target-center connectors reach the same positioned
                        // and node-label scans as other accepted source hops.
                        let mut connector_cost = *penalty;
                        if let Some(edge_index) = edge_index {
                            connector_cost = interaction_index.apply_label_obstacle_cost_guarded(
                                graph,
                                edge_index,
                                [entry_point, next_point],
                                connector_cost,
                                budget,
                            )?;
                        }
                        let candidate = entry_cost + connector_cost;
                        #[cfg(feature = "diagnostic-traces")]
                        if crate::internal::trace_env_value("TALA_TRACE_CONNECTOR_DETAIL")
                            .as_deref()
                            == Some("1")
                        {
                            eprintln!(
                                "CONNECTOR_TARGET_RUST edge={} from=({:.17e},{:.17e}) to=({:.17e},{:.17e}) entry_bits=0x{:016x} penalty_bits=0x{:016x} candidate_bits=0x{:016x} ports={:?}",
                                edge_index
                                    .map(|index| graph.edge_records[index].tala_id)
                                    .unwrap_or_default(),
                                entry_point.x,
                                entry_point.y,
                                next_point.x,
                                next_point.y,
                                entry_cost.to_bits(),
                                penalty.to_bits(),
                                candidate.to_bits(),
                                targets_at_point
                                    .iter()
                                    .map(|(port, cost)| {
                                        (
                                            port.point,
                                            port.side,
                                            port.direction,
                                            port.is_center,
                                            cost.to_bits(),
                                        )
                                    })
                                    .collect::<Vec<_>>(),
                            );
                        }
                        let target_context = search_nodes[target_index].context();
                        let previous_cost = slots[target_context].distances[axis.index()];
                        #[cfg(feature = "diagnostic-traces")]
                        trace_cost_breakdown(
                            entry.node,
                            next,
                            axis,
                            candidate,
                            *penalty,
                            *penalty,
                            connector_cost,
                            connector_cost,
                            connector_cost,
                            connector_cost,
                            connector_cost,
                        );
                        #[cfg(feature = "diagnostic-traces")]
                        trace_cost(
                            entry.node,
                            next,
                            axis,
                            candidate,
                            previous_cost,
                            slots[target_context].entries[axis.index()].is_some(),
                        );
                        if candidate >= previous_cost {
                            continue;
                        }
                        // TALA runs target-center connectors through the same
                        // directional update block as every other OVG hop.  An
                        // improvement therefore requeues the displaced target
                        // predecessor before decreasing the target entry; the
                        // extra carrier participates in the insertion order
                        // of equal-priority states.
                        if let Some(old_hop) = slots[target_context].previous[axis.index()] {
                            let old_hop = predecessor_index(old_hop);
                            let old_hop_context = search_nodes[old_hop].context();
                            let old_hop_cost = slots[old_hop_context].distances[axis.index()];
                            queue_push(
                                queue,
                                budget,
                                old_hop_cost,
                                QueueState {
                                    node: old_hop,
                                    axis,
                                },
                            )?;
                            #[cfg(feature = "diagnostic-traces")]
                            trace_heap("push", old_hop, axis, old_hop_cost, None, queue.len());
                        }
                        if slots[target_context].distances == INITIAL_SEARCH_SLOT.distances {
                            touched.push(target_context);
                        }
                        slots[target_context].distances[axis.index()] = candidate;
                        slots[target_context].previous[axis.index()] =
                            Some(predecessor(entry.node));
                        if let Some(handle) = slots[target_context].entries[axis.index()] {
                            queue_decrease(queue, budget, handle, candidate).ok()?;
                            #[cfg(feature = "diagnostic-traces")]
                            trace_heap(
                                "decrease",
                                target_index,
                                axis,
                                candidate,
                                Some(previous_cost),
                                queue.len(),
                            );
                        } else {
                            let handle = queue_push(
                                queue,
                                budget,
                                candidate,
                                QueueState {
                                    node: target_index,
                                    axis,
                                },
                            )?;
                            slots[target_context].entries[axis.index()] = Some(handle);
                            #[cfg(feature = "diagnostic-traces")]
                            trace_heap("push", target_index, axis, candidate, None, queue.len());
                        }
                    }
                    continue;
                }
                let delta = Point {
                    x: next_point.x - entry_point.x,
                    y: next_point.y - entry_point.y,
                };
                // Candidate inventories can include other cluster/sequence
                // members. Go's portMetadataFor(gSource/gTarget) checks the
                // actual OVG owner before constraining entry/exit direction.
                if let Some(source_ports_at_point) = entry_source_ports {
                    let admissible = source_ports_at_point.iter().any(|(port, _)| {
                        if overlap {
                            match port.direction {
                                PortSide::Top | PortSide::Bottom => delta.y != 0.0,
                                PortSide::Left | PortSide::Right => delta.x != 0.0,
                            }
                        } else {
                            follows_outward(delta, port.direction)
                        }
                    });
                    if !admissible {
                        continue;
                    }
                }
                let next_is_target_port = next_owned && next_owners.contains(&target_node);
                if next_is_target_port
                    && let Some(targets_at_point) = targets.get(&next)
                    && !targets_at_point.iter().any(|(port, _)| {
                        if overlap {
                            match port.direction {
                                PortSide::Top | PortSide::Bottom => delta.y != 0.0,
                                PortSide::Left | PortSide::Right => delta.x != 0.0,
                            }
                        } else {
                            follows_outward(
                                Point {
                                    x: -delta.x,
                                    y: -delta.y,
                                },
                                port.direction,
                            )
                        }
                    })
                {
                    continue;
                }
                let candidate_container = next_node.container;
                // Source gates ordinary points by actual endpoint ownership,
                // then walks both container ancestries with the route guard.
                if !(next_owned && next_owners.contains(&source_node))
                    && !next_is_target_port
                    && !route_container_is_admissible_guarded(
                        graph,
                        source_container,
                        target_container,
                        candidate_container,
                        candidate_container.is_some_and(|container| {
                            fixed_overlaps
                                .as_ref()
                                .is_some_and(|overlaps| overlaps.contains(&container))
                        }),
                        budget,
                    )?
                {
                    continue;
                }
                // Source checks directions and container ancestry before
                // rejecting ordinary diagonals. Those rejected hops still
                // perform the ancestry work and cancellation checks.
                if next != target_index
                    && entry.node != source_index
                    && next_point.x != entry_point.x
                    && next_point.y != entry_point.y
                {
                    continue;
                }
                let adjacent_is_center = next_node.flags & SEARCH_NODE_CENTER != 0;
                // Go initializes every ordinary hop from its geometric
                // distance and only replaces that value with the center
                // sentinel after the container/port-owner penalties. Keep
                // those checkpoints observable in the same order; the
                // final value is unchanged for ordinary nodes.
                let mut hop_cost = length;
                #[cfg(feature = "diagnostic-traces")]
                let base_hop_cost = hop_cost;
                if source_container.is_some()
                    && source_container == target_container
                    && !source_descendant_of_target
                    && !target_descendant_of_source
                    && entry_container != candidate_container
                    && candidate_container != source_container
                {
                    hop_cost += 4.0 * graph.routing_costs.1;
                }
                if next_owned
                    && entry_owners
                        .iter()
                        .any(|owner| next_owners.binary_search(owner).is_ok())
                {
                    hop_cost += 4.0 * graph.routing_costs.1;
                }
                if adjacent_is_center {
                    hop_cost = 10_000_000.0;
                }
                #[cfg(feature = "diagnostic-traces")]
                let mut structural_hop_cost = hop_cost;
                #[cfg(feature = "diagnostic-traces")]
                let mut interaction_hop_cost = hop_cost;
                #[cfg(feature = "diagnostic-traces")]
                let mut label_hop_cost = hop_cost;
                #[cfg(feature = "diagnostic-traces")]
                let mut arrow_hop_cost = hop_cost;
                #[cfg(feature = "diagnostic-traces")]
                let turn_hop_cost;
                if let Some(edge_index) = edge_index {
                    let endpoint_segment = [entry_point, next_point];
                    if !adjacent_is_center {
                        // Prohibited sharing replaces TALA's accumulated base
                        // hop cost; crossing merely adds to it.
                        let occupied =
                            interaction_index.occupied_routes_memo(entry_point, &mut occupied_memo);
                        hop_cost = interaction_index.apply_route_interaction_cost_with_occupied(
                            graph,
                            edge_index,
                            endpoint_segment,
                            hop_cost,
                            entry_edge_slots
                                .map(|slots| Self::canonical_slot_edge(slots, adjacency)),
                            occupied,
                            budget,
                        )?;
                    }
                    #[cfg(feature = "diagnostic-traces")]
                    {
                        // Go captures its structural checkpoint after route
                        // sharing and crossing costs have been applied.
                        structural_hop_cost = hop_cost;
                    }
                    // Accepted arrowhead labels and node labels are scored
                    // after the center/prohibited/ordinary branch in TALA.
                    hop_cost = interaction_index.apply_label_obstacle_cost_guarded(
                        graph,
                        edge_index,
                        endpoint_segment,
                        hop_cost,
                        budget,
                    )?;
                    #[cfg(feature = "diagnostic-traces")]
                    {
                        // Go captures both interaction and label checkpoints
                        // after positioned-label and node-label obstacles.
                        interaction_hop_cost = hop_cost;
                        label_hop_cost = hop_cost;
                    }
                    if entry_ports.is_some() {
                        hop_cost += interaction_index
                            .candidate_arrowhead_label_cost_for_end_guarded(
                                graph,
                                edge_index,
                                &endpoint_segment,
                                false,
                                &arrowhead_nodes,
                                budget,
                            )?;
                    }
                    if target_port_nodes.contains(&next) {
                        hop_cost += interaction_index
                            .candidate_arrowhead_label_cost_for_end_guarded(
                                graph,
                                edge_index,
                                &endpoint_segment,
                                true,
                                &arrowhead_nodes,
                                budget,
                            )?;
                    }
                    #[cfg(feature = "diagnostic-traces")]
                    {
                        arrow_hop_cost = hop_cost;
                    }
                }
                // In TALA the predecessor of an initialized port state is the
                // synthetic source-center node. The line-993 turn gate
                // explicitly exempts that first ordinary OVG hop.
                if slots[entry_context].previous[entry.axis.index()]
                    .is_some_and(|node| predecessor_index(node) != source_index)
                    && entry.axis != Axis::None
                    && entry.axis != axis
                {
                    let turn_point = entry_point;
                    let ideal_axis_point = next_point;
                    let mut multiplier =
                        ideal_turn_multiplier_guarded(ideal_axis_point, &axes, budget)?;
                    if multiplier != 1.0
                        && graph.nodes[source_node.0 as usize]
                            .cluster
                            .is_some_and(|cluster| {
                                graph.clusters[cluster].arrangement
                                    == graph.clusters[cluster].desired_arrangement
                                    && graph.clusters[cluster].members.len().is_multiple_of(2)
                            })
                    {
                        multiplier = 0.1;
                    }
                    // The pinned Go 1.27 ARM64 oracle emits FMADDD for
                    // nextDistance += turnCost * multiplier. Separate Rust
                    // operations lose one bit in label_positions' search.
                    hop_cost = graph.routing_costs.1.mul_add(multiplier, hop_cost);
                    if !source_descendant_of_target && !target_descendant_of_source {
                        for endpoint_box in [source_box, target_box] {
                            if endpoint_box.is_some_and(|rect| {
                                super::search::distance_to_boundary_for_ovg(turn_point, rect)
                                    <= 20.0
                            }) {
                                hop_cost += graph.routing_costs.1;
                            }
                        }
                    }
                }
                #[cfg(feature = "diagnostic-traces")]
                {
                    turn_hop_cost = hop_cost;
                }
                // Both endpoint ports are ordinary OVG nodes. Only TALA's
                // synthetic source/target centers are exempt from the
                // line-1045 near-port increment.
                if next_node.flags & SEARCH_NODE_NEAR_PORT != 0
                    && (self.near_port_owners[next].contains(&source_node)
                        || self.near_port_owners[next].contains(&target_node))
                {
                    hop_cost += 1.0;
                }
                #[cfg(feature = "diagnostic-traces")]
                trace_cost_breakdown(
                    entry.node,
                    next,
                    axis,
                    entry_cost + hop_cost,
                    base_hop_cost,
                    structural_hop_cost,
                    interaction_hop_cost,
                    label_hop_cost,
                    arrow_hop_cost,
                    turn_hop_cost,
                    hop_cost,
                );
                let next_cost = entry_cost + hop_cost;
                let next_context = next_node.context();
                let previous_cost = slots[next_context].distances[axis.index()];
                #[cfg(feature = "diagnostic-traces")]
                trace_cost(
                    entry.node,
                    next,
                    axis,
                    next_cost,
                    previous_cost,
                    slots[next_context].entries[axis.index()].is_some(),
                );
                if next_cost >= previous_cost {
                    continue;
                }
                // TALA deliberately requeues the displaced predecessor before
                // replacing a hop and decreasing the destination entry. This
                // preserves the upstream insertion order for equal priorities.
                if let Some(old_hop) = slots[next_context].previous[axis.index()] {
                    let old_hop = predecessor_index(old_hop);
                    let old_hop_context = search_nodes[old_hop].context();
                    let old_hop_cost = slots[old_hop_context].distances[axis.index()];
                    queue_push(
                        queue,
                        budget,
                        old_hop_cost,
                        QueueState {
                            node: old_hop,
                            axis,
                        },
                    )?;
                    #[cfg(feature = "diagnostic-traces")]
                    trace_heap("push", old_hop, axis, old_hop_cost, None, queue.len());
                }
                if slots[next_context].distances == INITIAL_SEARCH_SLOT.distances {
                    touched.push(next_context);
                }
                slots[next_context].distances[axis.index()] = next_cost;
                slots[next_context].previous[axis.index()] = Some(predecessor(entry.node));
                if let Some(handle) = slots[next_context].entries[axis.index()] {
                    queue_decrease(queue, budget, handle, next_cost).ok()?;
                    #[cfg(feature = "diagnostic-traces")]
                    trace_heap(
                        "decrease",
                        next,
                        axis,
                        next_cost,
                        Some(previous_cost),
                        queue.len(),
                    );
                } else {
                    let handle =
                        queue_push(queue, budget, next_cost, QueueState { node: next, axis })?;
                    slots[next_context].entries[axis.index()] = Some(handle);
                    #[cfg(feature = "diagnostic-traces")]
                    trace_heap("push", next, axis, next_cost, None, queue.len());
                }
            }
        }
        if !reached_target {
            return None;
        }
        // Direct translation of recovered `ovgEdgeRouter.getBestRoute`.
        // Search terminates when either directional target entry is dequeued,
        // but reconstruction compares both directional hop tables at every
        // node and may switch tables to avoid a turn.
        let mut sequence = Vec::new();
        let mut current = Some(target_index);
        let mut emitted: Option<usize> = None;
        let mut seen = BTreeSet::new();
        while let Some(current_index) = current {
            // bestRoute charges before even the cycle check.
            if let Some(budget) = budget {
                budget.step().ok()?;
            }
            if !seen.insert(current_index) {
                break;
            }
            sequence.push(current_index);
            // `source` is the synthetic route root. TALA's source-port
            // outward-direction gate keeps it predecessor-free; stop here
            // explicitly as the equivalent invariant for the restricted
            // single-source wrapper as well.
            if current_index == source_index {
                break;
            }

            let current_context = self.search_index(current_index);
            let vertical_hop =
                slots[current_context].previous[Axis::Vertical.index()].map(predecessor_index);
            let horizontal_hop =
                slots[current_context].previous[Axis::Horizontal.index()].map(predecessor_index);
            let vertical_distance = slots[current_context].distances[Axis::Vertical.index()];
            let horizontal_distance = slots[current_context].distances[Axis::Horizontal.index()];
            let (mut next, selected_vertical_hop) = match (vertical_hop, horizontal_hop) {
                (Some(vertical), Some(horizontal)) => {
                    if vertical_distance < horizontal_distance {
                        (Some(vertical), true)
                    } else {
                        (Some(horizontal), false)
                    }
                }
                (Some(vertical), None) => (Some(vertical), true),
                (None, Some(horizontal)) => (Some(horizontal), false),
                (None, None) => (None, false),
            };

            if let (Some(previous_index), Some(next_index)) = (emitted, next)
                && next_index != source_index
                && ((self.point(previous_index).x == self.point(current_index).x
                    && !selected_vertical_hop)
                    || (self.point(previous_index).y == self.point(current_index).y
                        && selected_vertical_hop))
            {
                let mut multiplier =
                    ideal_turn_multiplier_guarded(self.point(current_index), &axes, budget)?;
                if multiplier != 1.0
                    && graph.nodes[source_node.0 as usize]
                        .cluster
                        .is_some_and(|cluster| {
                            graph.clusters[cluster].arrangement
                                == graph.clusters[cluster].desired_arrangement
                                && graph.clusters[cluster].members.len().is_multiple_of(2)
                        })
                {
                    multiplier = 0.1;
                }

                let (alternative, alternative_distance) = if !selected_vertical_hop
                    && super::search::routing_precision_compare(
                        horizontal_distance + multiplier * graph.routing_costs.1,
                        vertical_distance,
                    )
                    .is_gt()
                {
                    (vertical_hop, vertical_distance)
                } else if selected_vertical_hop
                    && super::search::routing_precision_compare(
                        vertical_distance + multiplier * graph.routing_costs.1,
                        horizontal_distance,
                    )
                    .is_gt()
                {
                    (horizontal_hop, horizontal_distance)
                } else {
                    (None, f64::INFINITY)
                };
                if alternative.is_some() && alternative_distance < 10_000_000.0 {
                    next = alternative;
                }
            }
            emitted = Some(current_index);
            current = next;
        }

        // Source reverses with one guarded swap per pair, so exhaustion
        // during reconstruction cannot publish a partial drawable route.
        for left in 0..sequence.len() / 2 {
            if let Some(budget) = budget {
                budget.step().ok()?;
            }
            let right = sequence.len() - 1 - left;
            sequence.swap(left, right);
        }
        if sequence.first() == Some(&source_index) {
            sequence.remove(0);
        }
        if sequence.last() == Some(&target_index) {
            sequence.pop();
        }
        let source_port_index = *sequence.first()?;
        let target_port_index = *sequence.last()?;
        let source = sources.get(&source_port_index)?.first()?.0;
        let target = targets.get(&target_port_index)?.first()?.0;
        sequence.insert(0, source_index);
        sequence.push(target_index);
        // Keep the complete OVG-node path for route-search scoring. Collinear
        // nodes are semantically relevant while later edges inspect occupied
        // points and decide whether leaving a shared corridor is permitted.
        // The winning flavor is simplified only after all edges are routed.
        let route = sequence
            .iter()
            .skip(1)
            .take(sequence.len().saturating_sub(2))
            .map(|index| self.point(*index))
            .collect::<Vec<_>>();
        // generateRoutes validates the full OVG path, including its two
        // centers. A single shared port is a valid three-node path;
        // createSegmentEndpoints publishes two distinct copies of that port.
        Some(AnyTargetRoute {
            points: route,
            node_ids: sequence,
            source,
            target,
            cost: best_cost,
        })
    }
}

fn fill_visible_path(graph: &VisibilityGraph, from: usize, to: usize) -> bool {
    let start = graph.point(from);
    let end = graph.point(to);
    let mut current = from;
    let mut visited = BTreeSet::new();
    while current != to && visited.insert(current) {
        let point = graph.point(current);
        let Some(next) = graph.edges[current]
            .iter()
            .map(|entry| entry.neighbor())
            .find(|neighbor| {
                let candidate = graph.point(*neighbor);
                if start.x == end.x {
                    candidate.x == point.x
                        && ((start.y < end.y && point.y < candidate.y && candidate.y <= end.y)
                            || (start.y > end.y && point.y > candidate.y && candidate.y >= end.y))
                } else {
                    candidate.y == point.y
                        && ((start.x < end.x && point.x < candidate.x && candidate.x <= end.x)
                            || (start.x > end.x && point.x > candidate.x && candidate.x >= end.x))
                }
            })
        else {
            return false;
        };
        current = next;
    }
    current == to
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ideal_turn_scan_charges_misses_and_stops_at_first_match() {
        let axes = [(true, 0.0), (false, 22.0), (true, 10.0)];
        let point = Point { x: 10.0, y: 20.0 };
        let work = LocalRouteSearchWork::new(2);
        assert_eq!(
            ideal_turn_multiplier_guarded(point, &axes, Some(&work)),
            Some(0.98)
        );
        assert_eq!(work.used(), 2);
        for limit in 0..2 {
            let work = LocalRouteSearchWork::new(limit);
            assert_eq!(
                ideal_turn_multiplier_guarded(point, &axes, Some(&work)),
                None
            );
            assert!(work.exhausted());
        }
        let work = LocalRouteSearchWork::new(3);
        assert_eq!(
            ideal_turn_multiplier_guarded(Point { x: 100.0, y: 100.0 }, &axes, Some(&work)),
            Some(1.0)
        );
        assert_eq!(work.used(), 3);
        let work = LocalRouteSearchWork::new(0);
        assert_eq!(
            ideal_turn_multiplier_guarded(point, &[], Some(&work)),
            Some(1.0)
        );
        assert_eq!(work.used(), 0);
    }

    #[test]
    fn ordinary_container_gate_walks_both_endpoints_before_exemption() {
        let mut input = Graph::default();
        let root = input.add_node(node("root", 20.0, 20.0));
        let mut child = node("child", 20.0, 20.0);
        child.parent = Some(root);
        let child = input.add_node(child);
        let other = input.add_node(node("other", 20.0, 20.0));
        let graph = ArenaGraph::from_input(&input);
        // Source succeeds after two steps; target still visits its unrelated
        // root. Even a fixed-overlap exemption cannot bypass those walks.
        for fixed in [false, true] {
            let work = LocalRouteSearchWork::new(3);
            assert_eq!(
                route_container_is_admissible_guarded(
                    &graph,
                    Some(child),
                    Some(other),
                    Some(root),
                    fixed,
                    Some(&work)
                ),
                Some(true)
            );
            assert_eq!(work.used(), 3);
            for limit in 0..3 {
                let work = LocalRouteSearchWork::new(limit);
                assert_eq!(
                    route_container_is_admissible_guarded(
                        &graph,
                        Some(child),
                        Some(other),
                        Some(root),
                        fixed,
                        Some(&work)
                    ),
                    None
                );
                assert!(work.exhausted());
            }
        }
        let work = LocalRouteSearchWork::new(0);
        assert_eq!(
            route_container_is_admissible_guarded(&graph, None, None, None, false, Some(&work)),
            Some(true)
        );
        assert_eq!(work.used(), 0);
        let work = LocalRouteSearchWork::new(2);
        assert_eq!(
            route_container_is_admissible_guarded(
                &graph,
                None,
                None,
                Some(root),
                false,
                Some(&work)
            ),
            Some(false)
        );
        assert_eq!(work.used(), 2);
    }

    #[test]
    fn fixed_overlap_scan_marks_both_fixed_nodes_and_preserves_early_exits() {
        let mut input = Graph::default();
        let a = input.add_node(node("a", 20.0, 20.0));
        let b = input.add_node(node("b", 20.0, 20.0));
        let c = input.add_node(node("c", 20.0, 20.0));
        let mut graph = ArenaGraph::from_input(&input);
        for (id, x) in [(a, 0.0), (b, 10.0), (c, 100.0)] {
            graph.set_position(id, Point { x, y: 0.0 });
        }
        graph.nodes[a.0 as usize].fixed_top_left = Some(Point { x: 0.0, y: 0.0 });
        graph.nodes[b.0 as usize].fixed_top_left = Some(Point { x: 10.0, y: 0.0 });
        let nodes = [a, b, c];
        let work = LocalRouteSearchWork::new(15);
        assert_eq!(
            fixed_overlaps_for_route(&graph, &nodes, Some(&work)),
            Some(BTreeSet::from([a, b]))
        );
        assert_eq!(work.used(), 15);
        for limit in 0..15 {
            let work = LocalRouteSearchWork::new(limit);
            assert!(fixed_overlaps_for_route(&graph, &nodes, Some(&work)).is_none());
            assert_eq!(work.used(), limit);
        }
        // An enclosed descendant is fixed through its parent, but the pair
        // must remain exempt from overlap marking in both directions.
        graph.nodes[b.0 as usize].container = Some(a);
        graph.nodes[b.0 as usize].fixed_top_left = None;
        assert_eq!(
            fixed_overlaps_for_route(&graph, &[a, b], None),
            Some(BTreeSet::new())
        );
        // Placement's cached aggregate view can omit this container hop.
        // Routing guards must inspect the actual backlink, not that cache.
        graph.nodes[a.0 as usize].fixed_top_left = None;
        graph.node_order_membership_valid = true;
        graph.active_node_containers = vec![None; graph.nodes.len()].into();
        let work = LocalRouteSearchWork::new(5);
        assert_eq!(
            fixed_overlaps_for_route(&graph, &[a, b], Some(&work)),
            Some(BTreeSet::new())
        );
        assert_eq!(work.used(), 5);
    }

    #[test]
    fn blocked_port_setup_reserves_before_publication_and_exempts_ancestors() {
        let mut input = Graph::default();
        let source = input.add_node(node("source", 20.0, 20.0));
        let target = input.add_node(node("target", 20.0, 20.0));
        let mut graph = ArenaGraph::from_input(&input);
        graph.set_position(source, Point { x: 0.0, y: 0.0 });
        graph.set_position(target, Point { x: 20.0, y: 0.0 });
        let mut state = PreparedRouteSearch {
            ancestry: Some((false, false)),
            arrowhead_nodes: Vec::new(),
            blocked_ports_prepared: false,
            symmetry_prepared: false,
            fixed_overlaps: None,
            blocked_source: BTreeSet::new(),
            blocked_target: BTreeSet::new(),
            slots: vec![],
        };
        let too_small = LocalRouteSearchWork::new(11);
        assert!(
            state
                .prepare_blocked_ports(
                    &graph,
                    source,
                    target,
                    &BTreeMap::new(),
                    3,
                    4,
                    false,
                    Some(&too_small)
                )
                .is_none()
        );
        assert!(!state.blocked_ports_prepared && state.blocked_source.is_empty());
        let work = LocalRouteSearchWork::new(12);
        state
            .prepare_blocked_ports(
                &graph,
                source,
                target,
                &BTreeMap::new(),
                3,
                4,
                false,
                Some(&work),
            )
            .unwrap();
        assert!(state.blocked_ports_prepared);
        assert!(
            state
                .blocked_source
                .contains(&super::super::go_point_map_key(Point { x: 20.0, y: 10.0 }))
        );
        assert_eq!(state.blocked_source, state.blocked_target);
        assert_eq!(work.used(), 12);
        state.ancestry = Some((true, false));
        state
            .prepare_blocked_ports(&graph, source, target, &BTreeMap::new(), 3, 4, false, None)
            .unwrap();
        assert!(state.blocked_source.is_empty() && state.blocked_target.is_empty());
        let raw = super::super::ports(
            Rect {
                origin: Point::default(),
                size: Size {
                    width: 20.0,
                    height: 20.0,
                },
            },
            ShapeKind::Rectangle,
        );
        let selected = [raw[0]];
        let table_work = LocalRouteSearchWork::new(raw.len() as u64);
        state
            .prepare_table_blocked_ports(&raw, &raw, None, Some(&selected), Some(&table_work))
            .unwrap();
        assert_eq!(table_work.used(), raw.len() as u64);
        assert!(state.blocked_source.is_empty());
        assert!(
            !state
                .blocked_target
                .contains(&super::super::go_point_map_key(selected[0].point))
        );
        for port in &raw {
            assert_eq!(
                state
                    .blocked_target
                    .contains(&super::super::go_point_map_key(port.point)),
                port.point != selected[0].point
            );
        }
        for limit in 0..raw.len() as u64 {
            let guard = LocalRouteSearchWork::new(limit);
            assert!(
                state
                    .prepare_table_blocked_ports(&raw, &raw, None, Some(&selected), Some(&guard))
                    .is_none()
            );
            assert!(guard.exhausted());
        }
    }

    #[test]
    fn go_ovg_coordinate_map_keys_coalesce_signed_zero() {
        let negative_zero = Point { x: -0.0, y: 12.0 };
        let positive_zero = Point { x: 0.0, y: 12.0 };

        assert_eq!(point_key(negative_zero), point_key(positive_zero));
        assert_eq!(coordinate_key(-0.0), coordinate_key(0.0));
    }
    use crate::internal::TreeRoutingNode;
    use crate::{
        ArrowheadLabel, ContentAlignment, Edge, EdgeArrowheadLabels, Graph, Insets, LabelPosition,
        Node, ShapeKind, Size,
    };

    #[test]
    fn axis_coordinates_preserve_go_map_zero_and_nan_keys() {
        let nan = f64::from_bits(0x7ff8_0000_0000_0001);
        let mut coordinates = unique_axis_coordinates(
            [0.0, -0.0, 2.0, 2.0, f64::INFINITY, f64::INFINITY, nan, nan].into_iter(),
        );
        coordinates.sort_by(f64::total_cmp);

        assert_eq!(coordinates.len(), 5);
        assert!(coordinates.contains(&2.0));
        assert!(coordinates.iter().any(|value| value.is_infinite()));
        assert!(coordinates.iter().filter(|value| value.is_nan()).count() == 2);
        let zero = coordinates.iter().find(|value| **value == 0.0).unwrap();
        assert!(zero.is_sign_negative());
    }

    fn node(name: &str, width: f64, height: f64) -> Node {
        Node {
            d2_near_key: None,
            external_id: name.into(),
            size: Size { width, height },
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
            person: false,
            is_3d: false,
            is_multiple: false,
            shape: ShapeKind::Rectangle,
        }
    }

    #[test]
    fn nearby_edges_do_not_generate_active_scope_fill_points() {
        let mut input = Graph::default();
        let a = input.add_node(node("a", 80.0, 60.0));
        let b = input.add_node(node("b", 80.0, 60.0));
        let c = input.add_node(node("c", 80.0, 60.0));
        let d = input.add_node(node("d", 80.0, 60.0));
        let active = input.add_edge(Edge {
            source: a,
            target: b,
        });
        input.add_edge(Edge {
            source: c,
            target: d,
        });
        let mut graph = ArenaGraph::from_input(&input);
        for (id, x) in [(a, 0.0), (b, 300.0), (c, 1000.0), (d, 1300.0)] {
            graph.set_position(id, Point { x, y: 0.0 });
        }
        let boxes = graph
            .nodes
            .iter()
            .map(|n| {
                (
                    n.input_id,
                    Rect {
                        origin: n.position.unwrap(),
                        size: n.rect.size,
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        let ports = boxes
            .iter()
            .map(|(id, rect)| (*id, super::super::ports(*rect, ShapeKind::Rectangle)))
            .collect::<BTreeMap<_, _>>();
        let points = super::super::edge_nodes::edge_nodes(&graph, &[a, b], &boxes, &ports);
        let all = super::super::edge_nodes::edge_nodes(&graph, &[a, b, c, d], &boxes, &ports);
        assert!(!points.is_empty());
        assert_ne!(
            points, all,
            "the neighboring edge must exercise fill generation"
        );
        graph.edge_order.retain(|id| *id == active);
        assert_eq!(
            points,
            super::super::edge_nodes::edge_nodes(&graph, &[a, b], &boxes, &ports)
        );
    }

    #[test]
    fn recovered_segment_intersects_box_rejects_bbox_only_diagonal() {
        let box_rect = Rect {
            origin: Point { x: 0.0, y: 10.0 },
            size: crate::Size {
                width: 2.0,
                height: 2.0,
            },
        };
        assert!(!segment_intersects_box_recovered(
            Point { x: 0.0, y: 0.0 },
            Point { x: 10.0, y: 10.0 },
            box_rect,
        ));
    }

    #[test]
    fn recovered_segment_intersects_box_accepts_axis_boundary_contact() {
        let box_rect = Rect {
            origin: Point { x: 0.0, y: 10.0 },
            size: crate::Size {
                width: 2.0,
                height: 2.0,
            },
        };
        assert!(segment_intersects_box_recovered(
            Point { x: -10.0, y: 11.0 },
            Point { x: 10.0, y: 11.0 },
            box_rect,
        ));
    }

    #[test]
    fn fixed_route_import_preserves_generic_nodes_and_fmadd_distance() {
        let source = NodeId(0);
        let target = NodeId(1);
        let points = vec![Point { x: 0.0, y: 0.0 }, Point { x: 20.0, y: 20.0 }];
        let mut graph = VisibilityGraph {
            canonical: None,
            points: points.clone(),
            edges: Adjacency::from_nested(vec![Vec::new(); points.len()]),
            search_indices: (0..points.len()).collect(),
            centers: BTreeMap::from([(source, 0), (target, 1)]),
            center_owners: vec![Some(source), Some(target)],
            point_containers: vec![None; points.len()],
            port_owners: vec![BTreeSet::new(); points.len()].into(),
            near_port_owners: vec![BTreeSet::new(); points.len()].into(),
            rejected_hierarchy_points: BTreeSet::new(),
            tunnel_points: BTreeSet::new(),
            added_tree_ports: BTreeMap::new(),
        };
        let first = Point { x: 1.25, y: 2.5 };
        let second = Point { x: 4.75, y: 7.125 };
        graph.add_existing_route(source, target, &[first, second]);

        assert_eq!(graph.search_indices[2..], [2, 3]);
        assert!(
            graph.edges[0].is_empty(),
            "center-to-generic must be refused"
        );
        assert!(
            graph.edges[1].is_empty(),
            "generic-to-center must be refused"
        );
        let imported = graph.edges[2]
            .iter()
            .map(|entry| entry.tuple())
            .find(|(neighbor, ..)| *neighbor == 3)
            .unwrap();
        let dx = first.x - second.x;
        let dy = first.y - second.y;
        assert_eq!(
            imported.1.to_bits(),
            dy.mul_add(dy, dx * dx).sqrt().to_bits()
        );
        assert_eq!(imported.2, Axis::None);
        assert_eq!(search_axis(first, second), Axis::Vertical);
        assert_eq!(
            search_axis(
                first,
                Point {
                    y: first.y,
                    ..second
                }
            ),
            Axis::Horizontal
        );
    }

    #[test]
    fn shared_endpoint_port_keeps_both_ovg_centers_and_two_published_endpoints() {
        let mut input = Graph::default();
        let source = input.add_node(node("source", 20.0, 20.0));
        let target = input.add_node(node("target", 40.0, 40.0));
        let graph = ArenaGraph::from_input(&input);
        let shared = Point { x: 10.0, y: 0.0 };
        let points = vec![Point { x: 0.0, y: 0.0 }, Point { x: 20.0, y: 0.0 }, shared];
        let mut edges = vec![Vec::new(); points.len()];
        VisibilityGraph::connect(&mut edges, &points, 0, 2, Axis::Horizontal);
        VisibilityGraph::connect(&mut edges, &points, 1, 2, Axis::Horizontal);
        let mut port_owners = vec![BTreeSet::new(); points.len()];
        port_owners[2].extend([source, target]);
        let visibility = VisibilityGraph {
            canonical: None,
            points,
            edges: Adjacency::from_nested(edges),
            search_indices: (0..3).collect(),
            centers: BTreeMap::from([(source, 0), (target, 1)]),
            center_owners: vec![Some(source), Some(target), None],
            point_containers: vec![None; 3],
            port_owners: port_owners.into(),
            near_port_owners: vec![BTreeSet::new(); 3].into(),
            rejected_hierarchy_points: BTreeSet::new(),
            tunnel_points: BTreeSet::new(),
            added_tree_ports: BTreeMap::new(),
        };
        let source_port = Port {
            point: shared,
            direction: PortSide::Right,
            side: PortSide::Right,
            index: 0,
            tunnel: None,
            is_center: true,
        };
        let target_port = Port {
            point: shared,
            direction: PortSide::Left,
            side: PortSide::Left,
            index: 0,
            tunnel: None,
            is_center: true,
        };
        let boxes = BTreeMap::from([
            (
                source,
                Rect {
                    origin: Point { x: -10.0, y: -10.0 },
                    size: Size {
                        width: 20.0,
                        height: 20.0,
                    },
                },
            ),
            (
                target,
                Rect {
                    origin: Point { x: 0.0, y: -20.0 },
                    size: Size {
                        width: 40.0,
                        height: 40.0,
                    },
                },
            ),
        ]);
        let interaction = super::super::search::RouteInteractionIndex::new(&visibility, &graph);

        let route = visibility
            .route_between_any(
                &graph,
                &[(source_port, 0.0)],
                &[(target_port, 0.0)],
                source,
                target,
                None,
                &interaction,
                &boxes,
                false,
            )
            .expect("the source accepts a shared port between distinct OVG centers");
        assert_eq!(route.points, vec![shared]);
        assert_eq!(route.node_ids, vec![0, 2, 1]);
        let nodes = route
            .node_ids
            .iter()
            .map(|&i| visibility.point(i))
            .collect::<Vec<_>>();
        assert_eq!(
            super::super::route_geometry::segment_endpoints_from_ovg_nodes(&nodes),
            Some(vec![shared, shared])
        );
    }

    #[test]
    fn invisible_node_center_is_not_marked_as_a_node_center() {
        let mut input = Graph::default();
        let invisible = input.add_node(node("invisible", 40.0, 40.0));
        input.set_node_invisible(invisible, true);
        let mut arena = ArenaGraph::from_input(&input);
        arena.set_position(invisible, Point { x: 10.0, y: 20.0 });
        let boxes = BTreeMap::from([(
            invisible,
            Rect {
                origin: Point { x: 10.0, y: 20.0 },
                size: Size {
                    width: 40.0,
                    height: 40.0,
                },
            },
        )]);
        let ports = super::super::ports(boxes[&invisible], ShapeKind::Rectangle)
            .into_iter()
            .map(|port| (invisible, port));
        let visibility = VisibilityGraph::from_ports(
            &arena,
            &boxes,
            &arena.graph_node_order(),
            &[invisible],
            ports,
        );
        let center = visibility.centers[&invisible];

        assert_eq!(visibility.point(center), Point { x: 30.0, y: 40.0 });
        assert!(!visibility.is_node_center(center));
        assert!(!visibility.edges[center].is_empty());
        let array_units = visibility.member_count() as u64 * 3;
        let limited = LocalRouteSearchWork::new(array_units - 1);
        assert!(visibility.prepare_route_search(Some(&limited)).is_none());
        assert_eq!(limited.used(), 0);
        let exact = LocalRouteSearchWork::new(array_units);
        assert!(visibility.prepare_route_search(Some(&exact)).is_some());
        assert_eq!(exact.used(), array_units);
    }

    #[test]
    fn invisible_port_owner_ignores_ordinary_node_obstructions() {
        let mut input = Graph::default();
        let owner = input.add_node(node("owner", 40.0, 40.0));
        let blocker = input.add_node(node("blocker", 20.0, 20.0));
        let mut arena = ArenaGraph::from_input(&input);
        let boxes = BTreeMap::from([
            (
                owner,
                Rect {
                    origin: Point { x: 0.0, y: 0.0 },
                    size: Size {
                        width: 40.0,
                        height: 40.0,
                    },
                },
            ),
            (
                blocker,
                Rect {
                    origin: Point { x: 10.0, y: -40.0 },
                    size: Size {
                        width: 20.0,
                        height: 20.0,
                    },
                },
            ),
        ]);
        let port = Port {
            point: Point { x: 20.0, y: 0.0 },
            direction: PortSide::Top,
            side: PortSide::Top,
            index: 0,
            tunnel: None,
            is_center: false,
        };
        let candidate = Point { x: 20.0, y: -60.0 };
        assert!(!has_unobstructed_line_to_ports(
            candidate,
            &arena,
            &boxes,
            &arena.graph_node_order(),
            &[(owner, port)],
            &BTreeSet::new(),
            1,
        ));

        arena.nodes[owner.0 as usize].is_invisible = true;
        assert!(has_unobstructed_line_to_ports(
            candidate,
            &arena,
            &boxes,
            &arena.graph_node_order(),
            &[(owner, port)],
            &BTreeSet::new(),
            1,
        ));
    }

    #[test]
    fn fixed_route_import_appends_parallel_ovg_edges() {
        let source = NodeId(0);
        let target = NodeId(1);
        let source_port = Point { x: 10.0, y: 0.0 };
        let target_port = Point { x: 90.0, y: 0.0 };
        let points = vec![
            Point { x: 0.0, y: 0.0 },
            Point { x: 100.0, y: 0.0 },
            source_port,
            target_port,
        ];
        let mut edges = vec![Vec::new(); points.len()];
        VisibilityGraph::connect(&mut edges, &points, 0, 2, Axis::Horizontal);
        VisibilityGraph::connect(&mut edges, &points, 2, 3, Axis::Horizontal);
        VisibilityGraph::connect(&mut edges, &points, 3, 1, Axis::Horizontal);
        let mut port_owners = vec![BTreeSet::new(); points.len()];
        port_owners[2].insert(source);
        port_owners[3].insert(target);
        let mut graph = VisibilityGraph {
            canonical: None,
            points: points.clone(),
            edges: Adjacency::from_nested(edges),
            search_indices: (0..points.len()).collect(),
            centers: BTreeMap::from([(source, 0), (target, 1)]),
            center_owners: vec![Some(source), Some(target), None, None],
            point_containers: vec![None; points.len()],
            port_owners: port_owners.into(),
            near_port_owners: vec![BTreeSet::new(); points.len()].into(),
            rejected_hierarchy_points: BTreeSet::new(),
            tunnel_points: BTreeSet::new(),
            added_tree_ports: BTreeMap::new(),
        };

        graph.add_existing_route(source, target, &[source_port, target_port]);
        assert_eq!(
            graph.edges[2]
                .iter()
                .filter(|entry| entry.neighbor() == 3)
                .count(),
            2
        );
    }

    #[test]
    fn fixed_route_target_center_is_deduplicated_by_coordinate() {
        let source = NodeId(0);
        let target = NodeId(1);
        let later_same_center = NodeId(2);
        let target_point = Point { x: 100.0, y: 0.0 };
        let source_port = Point { x: 10.0, y: 0.0 };
        let points = vec![
            Point { x: 0.0, y: 0.0 },
            target_point,
            target_point,
            source_port,
        ];
        let mut port_owners = vec![BTreeSet::new(); points.len()];
        port_owners[3].insert(source);
        let mut graph = VisibilityGraph {
            canonical: None,
            points: points.clone(),
            edges: Adjacency::from_nested(vec![Vec::new(); points.len()]),
            search_indices: (0..points.len()).collect(),
            centers: BTreeMap::from([(source, 0), (target, 1), (later_same_center, 2)]),
            center_owners: vec![Some(source), Some(target), Some(later_same_center), None],
            point_containers: vec![None; points.len()],
            port_owners: port_owners.into(),
            near_port_owners: vec![BTreeSet::new(); points.len()].into(),
            rejected_hierarchy_points: BTreeSet::new(),
            tunnel_points: BTreeSet::new(),
            added_tree_ports: BTreeMap::new(),
        };

        graph.add_existing_route(source, target, &[source_port, target_point]);
        assert!(
            graph.edges[2].iter().all(|entry| entry.neighbor() != 1),
            "equal-coordinate target center was appended as a zero-length hop"
        );
        assert!(graph.edges[1].is_empty());
    }

    #[test]
    fn fixed_route_import_rejects_center_to_center_connections() {
        let source = NodeId(0);
        let target = NodeId(1);
        let unrelated = NodeId(2);
        let points = vec![
            Point { x: 0.0, y: 0.0 },
            Point { x: 100.0, y: 0.0 },
            Point { x: 50.0, y: 0.0 },
        ];
        let mut graph = VisibilityGraph {
            canonical: None,
            points: points.clone(),
            edges: Adjacency::from_nested(vec![Vec::new(); points.len()]),
            search_indices: (0..points.len()).collect(),
            centers: BTreeMap::from([(source, 0), (target, 1), (unrelated, 2)]),
            center_owners: vec![Some(source), Some(target), Some(unrelated)],
            point_containers: vec![None; points.len()],
            port_owners: vec![BTreeSet::new(); points.len()].into(),
            near_port_owners: vec![BTreeSet::new(); points.len()].into(),
            rejected_hierarchy_points: BTreeSet::new(),
            tunnel_points: BTreeSet::new(),
            added_tree_ports: BTreeMap::new(),
        };

        graph.add_existing_route(source, target, &[points[2]]);
        assert!((0..graph.edges.len()).all(|node| graph.edges[node].is_empty()));
    }

    #[test]
    fn accepted_arrowhead_labels_are_hard_search_obstacles() {
        let mut input = Graph::default();
        let first = input.add_node(node("first", 40.0, 40.0));
        let second = input.add_node(node("second", 40.0, 40.0));
        let third = input.add_node(node("third", 40.0, 40.0));
        let fixed = input.add_edge(Edge {
            source: first,
            target: second,
        });
        input.set_edge_arrowhead_labels(
            fixed,
            EdgeArrowheadLabels {
                source: Some(ArrowheadLabel {
                    text: "port".into(),
                    size: Size {
                        width: 20.0,
                        height: 10.0,
                    },
                    color: None,
                }),
                target: None,
            },
        );
        input.add_edge(Edge {
            source: first,
            target: third,
        });
        let mut arena = ArenaGraph::from_input(&input);
        arena.set_position(first, Point { x: 0.0, y: 0.0 });
        arena.set_position(second, Point { x: 100.0, y: 0.0 });
        arena.set_position(third, Point { x: 200.0, y: 0.0 });
        let points = vec![
            Point { x: 20.0, y: 20.0 },
            Point { x: 120.0, y: 20.0 },
            Point { x: 220.0, y: 20.0 },
        ];
        let visibility = VisibilityGraph {
            canonical: None,
            points: points.clone(),
            edges: Adjacency::from_nested(vec![Vec::new(); points.len()]),
            search_indices: (0..points.len()).collect(),
            centers: BTreeMap::from([(first, 0), (second, 1), (third, 2)]),
            center_owners: vec![Some(first), Some(second), Some(third)],
            point_containers: vec![None; points.len()],
            port_owners: vec![BTreeSet::new(); points.len()].into(),
            near_port_owners: vec![BTreeSet::new(); points.len()].into(),
            rejected_hierarchy_points: BTreeSet::new(),
            tunnel_points: BTreeSet::new(),
            added_tree_ports: BTreeMap::new(),
        };
        let fixed_route = [Point { x: 0.0, y: 0.0 }, Point { x: 100.0, y: 0.0 }];
        let rect = crate::internal::labels::arrowhead_label_rect_for_route(
            &arena.edge_records[0],
            &fixed_route,
            false,
        )
        .unwrap();
        let mut index = super::super::search::RouteInteractionIndex::new(&visibility, &arena);
        index.add_route(&arena, 0, &fixed_route, "test");
        let y = rect.origin.y + rect.size.height * 0.5;
        let candidate = [
            Point {
                x: rect.origin.x - 1.0,
                y,
            },
            Point {
                x: rect.right() + 1.0,
                y,
            },
        ];
        assert_eq!(index.apply_cost(&arena, 1, candidate, 7.0), 10_000_007.0);
    }

    #[test]
    fn standalone_search_charges_positioned_node_label_intersections() {
        let mut input = Graph::default();
        let source = input.add_node(node("source", 40.0, 40.0));
        let mut obstacle_node = node("obstacle", 80.0, 60.0);
        obstacle_node.label_size = Some(Size {
            width: 30.0,
            height: 20.0,
        });
        obstacle_node.label_position = LabelPosition::InsideMiddleCenter;
        let obstacle = input.add_node(obstacle_node);
        let target = input.add_node(node("target", 40.0, 40.0));
        input.add_edge(Edge { source, target });
        let mut arena = ArenaGraph::from_input(&input);
        arena.set_position(source, Point { x: 0.0, y: 0.0 });
        arena.set_position(obstacle, Point { x: 100.0, y: 0.0 });
        arena.set_position(target, Point { x: 240.0, y: 0.0 });
        arena.set_turn_cost(42.0);
        arena.routing_costs = arena.raw_routing_costs();
        let points = vec![
            Point { x: 20.0, y: 20.0 },
            Point { x: 140.0, y: 30.0 },
            Point { x: 260.0, y: 20.0 },
        ];
        let visibility = VisibilityGraph {
            canonical: None,
            points: points.clone(),
            edges: Adjacency::from_nested(vec![Vec::new(); points.len()]),
            search_indices: (0..points.len()).collect(),
            centers: BTreeMap::from([(source, 0), (obstacle, 1), (target, 2)]),
            center_owners: vec![Some(source), Some(obstacle), Some(target)],
            point_containers: vec![None; points.len()],
            port_owners: vec![BTreeSet::new(); points.len()].into(),
            near_port_owners: vec![BTreeSet::new(); points.len()].into(),
            rejected_hierarchy_points: BTreeSet::new(),
            tunnel_points: BTreeSet::new(),
            added_tree_ports: BTreeMap::new(),
        };
        let rect = crate::internal::labels::positioned_node_label_rect(&arena, obstacle).unwrap();
        let y = rect.origin.y + rect.size.height * 0.5;
        let candidate = [
            Point {
                x: rect.origin.x - 1.0,
                y,
            },
            Point {
                x: rect.right() + 1.0,
                y,
            },
        ];
        let index = super::super::search::RouteInteractionIndex::new_with_node_labels(
            &visibility,
            &arena,
            true,
        );
        assert_eq!(index.apply_cost(&arena, 0, candidate, 0.0), 42.0);
        // Source charges misses as well as hits, and endpoint labels are excluded.
        let guard = LocalRouteSearchWork::new(1);
        assert_eq!(
            index.apply_label_obstacle_cost_guarded(&arena, 0, candidate, 0.0, Some(&guard)),
            Some(42.0)
        );
        assert_eq!(guard.used(), 1);
        let miss = [
            Point {
                x: -100.0,
                y: -100.0,
            },
            Point {
                x: -80.0,
                y: -100.0,
            },
        ];
        let guard = LocalRouteSearchWork::new(1);
        assert_eq!(
            index.apply_label_obstacle_cost_guarded(&arena, 0, miss, 7.0, Some(&guard)),
            Some(7.0)
        );
        assert_eq!(guard.used(), 1);
        let exhausted = LocalRouteSearchWork::new(0);
        assert_eq!(
            index.apply_label_obstacle_cost_guarded(&arena, 0, miss, 7.0, Some(&exhausted)),
            None
        );
        assert!(exhausted.exhausted());
    }

    #[test]
    fn invisible_route_skips_only_nearby_and_overlap_publication() {
        let mut input = Graph::default();
        let source = input.add_node(node("source", 40.0, 40.0));
        let target = input.add_node(node("target", 40.0, 40.0));
        let fixed = input.add_edge(Edge { source, target });
        input.set_edge_style(
            fixed,
            crate::EdgeStyle {
                opacity: Some("0".into()),
                ..crate::EdgeStyle::default()
            },
        );
        let mut arena = ArenaGraph::from_input(&input);
        arena.set_position(source, Point { x: 0.0, y: 0.0 });
        arena.set_position(target, Point { x: 100.0, y: 0.0 });
        assert!(arena.edge_records[0].is_invisible());
        assert_eq!(
            (
                arena.edge_records[0].min_width,
                arena.edge_records[0].min_height
            ),
            (0.0, 0.0)
        );

        let points = vec![
            Point { x: 0.0, y: 0.0 },
            Point { x: 20.0, y: 20.0 },
            Point { x: 80.0, y: 20.0 },
            Point { x: 100.0, y: 0.0 },
        ];
        let edges = vec![Vec::new(); points.len()];
        let visibility = VisibilityGraph {
            canonical: None,
            points: points.clone(),
            edges: Adjacency::from_nested(edges),
            search_indices: (0..points.len()).collect(),
            centers: BTreeMap::from([(source, 0), (target, 3)]),
            center_owners: vec![Some(source), None, None, Some(target)],
            point_containers: vec![None; points.len()],
            port_owners: vec![BTreeSet::new(); points.len()].into(),
            near_port_owners: vec![BTreeSet::new(); points.len()].into(),
            rejected_hierarchy_points: BTreeSet::new(),
            tunnel_points: BTreeSet::new(),
            added_tree_ports: BTreeMap::new(),
        };
        let mut index = super::super::search::RouteInteractionIndex::new(&visibility, &arena);
        index.add_route(&arena, 0, &points[1..3], "test");

        let (point_routes, routed_segments, nearby_edges, overlaps) = index.publication_counts();
        assert_eq!(point_routes, 4);
        assert_eq!(routed_segments, 1);
        assert_eq!(nearby_edges, 0);
        assert_eq!(overlaps, 0);
    }

    #[test]
    fn orthogonal_path_retains_intermediate_crossing_vertices() {
        let points = vec![
            Point { x: 10.0, y: 0.0 },
            Point { x: 10.0, y: 5.0 },
            Point { x: 10.0, y: 10.0 },
        ];
        let mut edges = vec![Vec::new(); points.len()];
        VisibilityGraph::connect(&mut edges, &points, 0, 1, Axis::Vertical);
        VisibilityGraph::connect(&mut edges, &points, 1, 2, Axis::Vertical);
        let graph = VisibilityGraph {
            canonical: None,
            points: points.clone(),
            edges: Adjacency::from_nested(edges),
            search_indices: (0..points.len()).collect(),
            centers: BTreeMap::new(),
            center_owners: vec![None; 3],
            point_containers: vec![None; 3],
            port_owners: vec![BTreeSet::new(); 3].into(),
            near_port_owners: vec![BTreeSet::new(); 3].into(),
            rejected_hierarchy_points: BTreeSet::new(),
            tunnel_points: BTreeSet::new(),
            added_tree_ports: BTreeMap::new(),
        };

        assert_eq!(graph.orthogonal_path(points[0], points[2]), Some(points));
    }

    #[test]
    fn tree_s_shape_keeps_source_orientation_for_target_leg() {
        let points = vec![
            Point { x: 100.0, y: 0.0 },
            Point { x: 50.0, y: 0.0 },
            Point { x: 50.0, y: 20.0 },
            Point { x: 100.0, y: 20.0 },
        ];
        let mut edges = vec![Vec::new(); points.len()];
        VisibilityGraph::connect(&mut edges, &points, 0, 1, Axis::Horizontal);
        VisibilityGraph::connect(&mut edges, &points, 1, 2, Axis::Vertical);
        let graph = VisibilityGraph {
            canonical: None,
            points: points.clone(),
            edges: Adjacency::from_nested(edges),
            search_indices: (0..points.len()).collect(),
            centers: BTreeMap::new(),
            center_owners: vec![None; points.len()],
            point_containers: vec![None; points.len()],
            port_owners: vec![BTreeSet::new(); points.len()].into(),
            near_port_owners: vec![BTreeSet::new(); points.len()].into(),
            rejected_hierarchy_points: BTreeSet::new(),
            tunnel_points: BTreeSet::new(),
            added_tree_ports: BTreeMap::new(),
        };

        // TALA's routeInSShape uses the source port side for both horizontal
        // legs. The final leg therefore cannot walk rightward here, and the
        // sentinel route must fall back to ordinary OVG routing.
        assert_eq!(
            graph.s_shape_route(
                points[0],
                points[1],
                points[2],
                points[3],
                Orientation::Right,
            ),
            None
        );
    }

    #[test]
    fn tree_midpoints_are_visibility_vertices_before_sentinel_routing() {
        let mut input = Graph::default();
        let parent = input.add_node(node("parent", 40.0, 40.0));
        let child = input.add_node(node("child", 40.0, 40.0));
        let sentinel_edge = input.add_edge(Edge {
            source: parent,
            target: child,
        });
        let mut graph = ArenaGraph::from_input(&input);
        graph.set_position(parent, Point { x: 0.0, y: 0.0 });
        graph.set_position(child, Point { x: 0.0, y: 200.0 });
        graph.tree_routing_nodes.insert(
            child,
            TreeRoutingNode {
                parent,
                sentinel_edge,
                // TALA's tree carrier is the geometry orientation: Bottom
                // selects the parent's bottom port and the child's top port
                // for a child below the parent. The serialized edge's
                // source-to-target orientation is derived separately.
                orientation: super::super::Orientation::Bottom,
                extraction_order: 0,
            },
        );
        graph.publish_preprocessed_owner_trees();
        let boxes = graph
            .nodes
            .iter()
            .map(|node| {
                (
                    node.input_id,
                    Rect {
                        origin: node.position.unwrap(),
                        size: node.rect.size,
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        let all_ports = graph
            .nodes
            .iter()
            .flat_map(|node| {
                super::super::ports(boxes[&node.input_id], node.shape)
                    .into_iter()
                    .map(move |port| (node.input_id, port))
            })
            .collect::<Vec<_>>();

        let ovg = VisibilityGraph::from_ports(
            &graph,
            &boxes,
            &graph.graph_node_order(),
            &graph.graph_node_order(),
            all_ports.into_iter(),
        );

        assert!(ovg.contains_point(Point { x: 20.0, y: 90.0 }));
    }

    #[test]
    fn aligned_tree_child_port_joins_the_visibility_port_inventory() {
        let mut input = Graph::default();
        let parent = input.add_node(node("parent", 52.0, 40.0));
        let child = input.add_node(node("child", 53.0, 40.0));
        let sentinel_edge = input.add_edge(Edge {
            source: parent,
            target: child,
        });
        let mut graph = ArenaGraph::from_input(&input);
        graph.set_position(parent, Point { x: 0.0, y: 0.0 });
        graph.set_position(child, Point { x: 0.0, y: 200.0 });
        graph.tree_routing_nodes.insert(
            child,
            TreeRoutingNode {
                parent,
                sentinel_edge,
                // TALA's tree carrier is the geometry orientation: Bottom
                // selects the parent's bottom port and the child's top port
                // for a child below the parent. The serialized edge's
                // source-to-target orientation is derived separately.
                orientation: super::super::Orientation::Bottom,
                extraction_order: 0,
            },
        );
        graph.publish_preprocessed_owner_trees();
        let boxes = graph
            .nodes
            .iter()
            .map(|node| {
                (
                    node.input_id,
                    Rect {
                        origin: node.position.unwrap(),
                        size: node.rect.size,
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        let all_ports = graph
            .nodes
            .iter()
            .flat_map(|node| {
                super::super::ports(boxes[&node.input_id], node.shape)
                    .into_iter()
                    .map(move |port| (node.input_id, port))
            })
            .collect::<Vec<_>>();

        let ovg = VisibilityGraph::from_ports(
            &graph,
            &boxes,
            &graph.graph_node_order(),
            &[parent, child],
            all_ports.clone().into_iter(),
        );

        assert!(ovg.contains_point(Point { x: 26.0, y: 200.0 }));
        assert_eq!(ovg.center_point(child), Some(Point { x: 26.5, y: 220.0 }));
        assert_eq!(ovg.added_tree_ports(child).len(), 1);
        assert_eq!(
            ovg.added_tree_ports(child)[0].point,
            Point { x: 26.0, y: 200.0 }
        );
        // Earlier topology can already own this coordinate for another node.
        // Canonicalization must add the child's ownership, not suppress it.
        let mut coincident_ports = all_ports;
        let mut earlier_port = ovg.added_tree_ports(child)[0];
        earlier_port.index = usize::MAX;
        coincident_ports.push((parent, earlier_port));
        let reused = VisibilityGraph::from_ports(
            &graph,
            &boxes,
            &graph.graph_node_order(),
            &[parent, child],
            coincident_ports.into_iter(),
        );
        assert_eq!(reused.added_tree_ports(child).len(), 1);
        assert_eq!(
            reused.port_index(parent, earlier_port.point),
            reused.port_index(child, earlier_port.point)
        );
        assert!(reused.port_index(child, earlier_port.point).is_some());
    }

    #[test]
    fn boundary_layers_connect_same_side_ports_around_an_obstacle() {
        let mut input = Graph::default();
        let source = input.add_node(node("source", 40.0, 40.0));
        let obstacle = input.add_node(node("obstacle", 80.0, 80.0));
        let target = input.add_node(node("target", 40.0, 40.0));
        let mut graph = ArenaGraph::from_input(&input);
        graph.set_position(source, Point { x: 0.0, y: 100.0 });
        graph.set_position(obstacle, Point { x: 80.0, y: 60.0 });
        graph.set_position(target, Point { x: 200.0, y: 100.0 });

        let boxes = graph
            .nodes
            .iter()
            .map(|node| {
                (
                    node.input_id,
                    Rect {
                        origin: node.position.unwrap(),
                        size: node.rect.size,
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        let all_ports = graph
            .nodes
            .iter()
            .flat_map(|node| {
                super::super::ports(boxes[&node.input_id], node.shape)
                    .into_iter()
                    .map(move |port| (node.input_id, port))
            })
            .collect::<Vec<_>>();
        let source_port = super::super::ports(boxes[&source], ShapeKind::Rectangle)
            .into_iter()
            .find(|port| port.side == PortSide::Top && port.is_center)
            .unwrap();
        let target_port = super::super::ports(boxes[&target], ShapeKind::Rectangle)
            .into_iter()
            .find(|port| port.side == PortSide::Top && port.is_center)
            .unwrap();

        let ovg = VisibilityGraph::from_ports(
            &graph,
            &boxes,
            &graph.graph_node_order(),
            &graph.graph_node_order(),
            all_ports.into_iter(),
        );
        let route = ovg.route(&graph, source_port, target_port, source, target);

        assert_eq!(route.first(), Some(&source_port.point));
        assert_eq!(route.last(), Some(&target_port.point));
        assert!(route.len() > 2);
        assert!(
            route
                .windows(2)
                .all(|segment| !segment_crosses_rect_interior(
                    segment[0],
                    segment[1],
                    boxes[&obstacle]
                ))
        );
    }

    #[test]
    fn corner_nodes_inside_the_twenty_unit_node_band_are_rejected() {
        let node_rect = Rect {
            origin: Point { x: 0.0, y: 0.0 },
            size: Size {
                width: 40.0,
                height: 40.0,
            },
        };
        let mut corners = Vec::new();
        add_corner_nodes_not_near(
            &mut corners,
            0.0,
            0.0,
            40.0,
            40.0,
            |point| point_near_rect(point, node_rect, 20.0),
            |_| {},
        );

        assert_eq!(
            corners,
            vec![
                Point { x: -40.0, y: -40.0 },
                Point { x: 80.0, y: -40.0 },
                Point { x: 80.0, y: 80.0 },
                Point { x: -40.0, y: 80.0 },
                Point { x: -60.0, y: -60.0 },
                Point { x: 100.0, y: -60.0 },
                Point { x: 100.0, y: 100.0 },
                Point { x: -60.0, y: 100.0 },
            ]
        );
    }

    #[test]
    fn search_can_reenter_an_admissible_source_port_across_container_boundaries() {
        let mut input = Graph::default();
        let source_parent = input.add_node(node("source-parent", 100.0, 100.0));
        let mut source_node = node("source", 20.0, 20.0);
        source_node.parent = Some(source_parent);
        let source = input.add_node(source_node);
        let mut source_child = node("source-child", 1.0, 1.0);
        source_child.parent = Some(source);
        input.add_node(source_child);
        let target_parent = input.add_node(node("target-parent", 100.0, 100.0));
        let mut target_node = node("target", 20.0, 20.0);
        target_node.parent = Some(target_parent);
        let target = input.add_node(target_node);
        let graph = ArenaGraph::from_input(&input);

        let source_center = Point { x: 20.0, y: 0.0 };
        let target_center = Point { x: -30.0, y: 0.0 };
        let source_top = Port {
            point: Point { x: 20.0, y: -10.0 },
            direction: PortSide::Top,
            side: PortSide::Top,
            index: 0,
            tunnel: None,
            is_center: true,
        };
        let source_left = Port {
            point: Point { x: 10.0, y: 0.0 },
            direction: PortSide::Left,
            side: PortSide::Left,
            index: 0,
            tunnel: None,
            is_center: true,
        };
        let target_right = Port {
            point: Point { x: -20.0, y: 0.0 },
            direction: PortSide::Right,
            side: PortSide::Right,
            index: 0,
            tunnel: None,
            is_center: true,
        };
        let points = vec![
            source_center,
            target_center,
            source_top.point,
            Point { x: 20.0, y: -20.0 },
            Point { x: 10.0, y: -20.0 },
            source_left.point,
            Point { x: 0.0, y: 0.0 },
            target_right.point,
        ];
        let mut edges = vec![Vec::new(); points.len()];
        for (first, second, axis) in [
            (0, 2, Axis::Vertical),
            (0, 5, Axis::Horizontal),
            (2, 3, Axis::Vertical),
            (3, 4, Axis::Horizontal),
            (4, 5, Axis::Vertical),
            (5, 6, Axis::Horizontal),
            (6, 7, Axis::Horizontal),
            (7, 1, Axis::Horizontal),
        ] {
            VisibilityGraph::connect(&mut edges, &points, first, second, axis);
        }
        let mut port_owners = vec![BTreeSet::new(); points.len()];
        port_owners[2].insert(source);
        port_owners[5].insert(source);
        port_owners[7].insert(target);
        let visibility = VisibilityGraph {
            canonical: None,
            points: points.clone(),
            edges: Adjacency::from_nested(edges),
            search_indices: (0..points.len()).collect(),
            centers: BTreeMap::from([(source, 0), (target, 1)]),
            center_owners: vec![
                Some(source),
                Some(target),
                None,
                None,
                None,
                None,
                None,
                None,
            ],
            point_containers: vec![
                Some(source),
                Some(target),
                Some(source),
                None,
                None,
                Some(source),
                None,
                Some(target),
            ],
            port_owners: port_owners.into(),
            near_port_owners: vec![BTreeSet::new(); points.len()].into(),
            rejected_hierarchy_points: BTreeSet::new(),
            tunnel_points: BTreeSet::new(),
            added_tree_ports: BTreeMap::new(),
        };
        let boxes = BTreeMap::from([
            (
                source,
                Rect {
                    origin: Point { x: 10.0, y: -10.0 },
                    size: Size {
                        width: 20.0,
                        height: 20.0,
                    },
                },
            ),
            (
                target,
                Rect {
                    origin: Point { x: -40.0, y: -10.0 },
                    size: Size {
                        width: 20.0,
                        height: 20.0,
                    },
                },
            ),
        ]);
        assert!(!graph.visibility_container_is_admissible(
            Some(source_parent),
            Some(target_parent),
            Some(source),
            false,
        ));

        let interaction = super::super::search::RouteInteractionIndex::new(&visibility, &graph);
        let route = visibility
            .route_between_any(
                &graph,
                &[(source_top, 0.0), (source_left, 1_000_000.0)],
                &[(target_right, 0.0)],
                source,
                target,
                None,
                &interaction,
                &boxes,
                false,
            )
            .unwrap();

        assert_eq!(route.source.point, source_top.point);
        assert_eq!(route.source.side, source_top.side);
        assert_eq!(
            route.points,
            vec![
                source_top.point,
                Point { x: 20.0, y: -20.0 },
                Point { x: 10.0, y: -20.0 },
                source_left.point,
                Point { x: 0.0, y: 0.0 },
                target_right.point,
            ]
        );
        assert_eq!(route.node_ids.len(), route.points.len() + 2);
        assert_eq!(route.node_ids.first(), visibility.centers.get(&source));
        assert_eq!(route.node_ids.last(), visibility.centers.get(&target));
        assert_eq!(
            route
                .node_ids
                .iter()
                .skip(1)
                .take(route.points.len())
                .map(|index| visibility.point(*index))
                .collect::<Vec<_>>(),
            route.points
        );
    }

    #[test]
    fn intersections_require_unobstructed_axes_to_two_distinct_port_owners() {
        let mut input = Graph::default();
        let first = input.add_node(node("first", 40.0, 40.0));
        let second = input.add_node(node("second", 40.0, 40.0));
        let mut graph = ArenaGraph::from_input(&input);
        graph.set_position(first, Point { x: 0.0, y: 0.0 });
        graph.set_position(second, Point { x: 80.0, y: 80.0 });

        let boxes = [first, second]
            .into_iter()
            .map(|node| {
                (
                    node,
                    Rect {
                        origin: graph.position(node).unwrap(),
                        size: graph.nodes[node.0 as usize].rect.size,
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        let first_port = Port {
            point: Point { x: 40.0, y: 20.0 },
            direction: PortSide::Right,
            side: PortSide::Right,
            index: 0,
            is_center: true,
            tunnel: None,
        };
        let second_port = Port {
            point: Point { x: 100.0, y: 80.0 },
            direction: PortSide::Top,
            side: PortSide::Top,
            index: 0,
            is_center: true,
            tunnel: None,
        };
        let intersection = Point { x: 100.0, y: 20.0 };

        assert!(!has_unobstructed_line_to_ports(
            intersection,
            &graph,
            &boxes,
            &graph.graph_node_order(),
            &[(first, first_port)],
            &BTreeSet::new(),
            2,
        ));
        assert!(has_unobstructed_line_to_ports(
            intersection,
            &graph,
            &boxes,
            &graph.graph_node_order(),
            &[(first, first_port), (second, second_port)],
            &BTreeSet::new(),
            2,
        ));
    }

    #[test]
    fn sequence_members_do_not_obstruct_each_others_port_axes() {
        let mut input = Graph::default();
        let first = input.add_node(node("first", 100.0, 100.0));
        let second = input.add_node(node("second", 100.0, 100.0));
        let mut graph = ArenaGraph::from_input(&input);
        graph.set_position(first, Point { x: 0.0, y: 0.0 });
        graph.set_position(second, Point { x: 65.0, y: 0.0 });
        graph.nodes[first.0 as usize].sequence = Some(0);
        graph.nodes[second.0 as usize].sequence = Some(0);

        let boxes = [
            (
                first,
                Rect {
                    origin: Point { x: 0.0, y: 0.0 },
                    size: Size {
                        width: 100.0,
                        height: 100.0,
                    },
                },
            ),
            (
                second,
                Rect {
                    origin: Point { x: 65.0, y: 0.0 },
                    size: Size {
                        width: 100.0,
                        height: 100.0,
                    },
                },
            ),
        ]
        .into_iter()
        .collect::<BTreeMap<_, _>>();
        let ports = [
            (
                first,
                Port {
                    point: Point { x: 0.0, y: 25.0 },
                    direction: PortSide::Left,
                    side: PortSide::Left,
                    index: 0,
                    tunnel: None,
                    is_center: false,
                },
            ),
            (
                second,
                Port {
                    point: Point { x: 65.0, y: 25.0 },
                    direction: PortSide::Left,
                    side: PortSide::Left,
                    index: 0,
                    tunnel: None,
                    is_center: false,
                },
            ),
        ];

        assert!(has_unobstructed_line_to_ports(
            Point { x: 200.0, y: 25.0 },
            &graph,
            &boxes,
            &[first, second],
            &ports,
            &BTreeSet::new(),
            2,
        ));

        graph.nodes[second.0 as usize].sequence = None;
        assert!(!has_unobstructed_line_to_ports(
            Point { x: 200.0, y: 25.0 },
            &graph,
            &boxes,
            &[first, second],
            &ports,
            &BTreeSet::new(),
            2,
        ));
    }

    #[test]
    fn nearby_boxes_are_not_ovg_intersection_obstacles_for_the_split_graph() {
        let mut input = Graph::default();
        let first = input.add_node(node("first", 40.0, 40.0));
        let second = input.add_node(node("second", 40.0, 40.0));
        let nearby = input.add_node(node("nearby", 20.0, 20.0));
        let mut graph = ArenaGraph::from_input(&input);
        graph.set_position(first, Point { x: 0.0, y: 0.0 });
        graph.set_position(second, Point { x: 80.0, y: 0.0 });
        graph.set_position(nearby, Point { x: 40.0, y: 10.0 });
        let boxes = [
            (
                first,
                Rect {
                    origin: Point { x: 0.0, y: 0.0 },
                    size: Size {
                        width: 40.0,
                        height: 40.0,
                    },
                },
            ),
            (
                second,
                Rect {
                    origin: Point { x: 80.0, y: 0.0 },
                    size: Size {
                        width: 40.0,
                        height: 40.0,
                    },
                },
            ),
            (
                nearby,
                Rect {
                    origin: Point { x: 40.0, y: 10.0 },
                    size: Size {
                        width: 20.0,
                        height: 20.0,
                    },
                },
            ),
        ]
        .into_iter()
        .collect::<BTreeMap<_, _>>();
        let ports = [
            (
                first,
                Port {
                    point: Point { x: 20.0, y: 20.0 },
                    direction: PortSide::Right,
                    side: PortSide::Right,
                    index: 0,
                    tunnel: None,
                    is_center: false,
                },
            ),
            (
                second,
                Port {
                    point: Point { x: 100.0, y: 20.0 },
                    direction: PortSide::Left,
                    side: PortSide::Left,
                    index: 0,
                    tunnel: None,
                    is_center: false,
                },
            ),
        ];

        assert!(has_unobstructed_line_to_ports(
            Point { x: 60.0, y: 20.0 },
            &graph,
            &boxes,
            &[first, second],
            &ports,
            &BTreeSet::new(),
            2,
        ));
    }

    #[test]
    fn tunnel_ports_are_added_after_ordinary_intersections() {
        let mut input = Graph::default();
        let axis_owner = input.add_node(node("axis-owner", 20.0, 40.0));
        let first_tunnel_owner = input.add_node(node("first-tunnel-owner", 10.0, 22.0));
        let second_tunnel_owner = input.add_node(node("second-tunnel-owner", 10.0, 22.0));
        let mut graph = ArenaGraph::from_input(&input);
        graph.nodes[first_tunnel_owner.0 as usize].is_container = true;
        graph.nodes[second_tunnel_owner.0 as usize].is_container = true;
        graph.set_position(axis_owner, Point { x: 5.0, y: -40.0 });
        graph.set_position(first_tunnel_owner, Point { x: 50.0, y: 50.0 });
        graph.set_position(second_tunnel_owner, Point { x: 90.0, y: 50.0 });
        let boxes = graph
            .nodes
            .iter()
            .map(|node| {
                (
                    node.input_id,
                    Rect {
                        origin: node.position.unwrap(),
                        size: node.rect.size,
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        let ordinary = Port {
            point: Point { x: 15.0, y: 0.0 },
            direction: PortSide::Bottom,
            side: PortSide::Bottom,
            index: 0,
            tunnel: None,
            is_center: true,
        };
        let first_tunnel = Port {
            point: Point { x: 50.0, y: 61.0 },
            direction: PortSide::Left,
            side: PortSide::Left,
            index: 0,
            tunnel: Some(0),
            is_center: true,
        };
        let second_tunnel = Port {
            point: Point { x: 90.0, y: 61.0 },
            direction: PortSide::Right,
            side: PortSide::Right,
            index: 0,
            tunnel: Some(0),
            is_center: true,
        };

        let ovg = VisibilityGraph::from_ports(
            &graph,
            &boxes,
            &graph.graph_node_order(),
            &graph.graph_node_order(),
            [
                (axis_owner, ordinary),
                (first_tunnel_owner, first_tunnel),
                (second_tunnel_owner, second_tunnel),
            ]
            .into_iter(),
        );

        assert!(ovg.contains_point(first_tunnel.point));
        assert!(ovg.contains_point(second_tunnel.point));
        assert!(ovg.segment_is_visible(first_tunnel.point, second_tunnel.point));
        assert!(!ovg.contains_point(Point { x: 15.0, y: 61.0 }));
    }

    #[test]
    fn tunnel_center_adjacency_follows_global_discovery_order() {
        let mut input = Graph::default();
        let first = input.add_node(node("first", 40.0, 40.0));
        let middle = input.add_node(node("middle", 40.0, 40.0));
        let last = input.add_node(node("last", 40.0, 40.0));
        // Middle's own incident list starts with the last node, but global
        // buildTunnels visits first before middle and discovers that pair first.
        input.add_edge(Edge {
            source: middle,
            target: last,
        });
        input.add_edge(Edge {
            source: first,
            target: middle,
        });
        let mut graph = ArenaGraph::from_input(&input);
        for (id, x) in [(first, 0.0), (middle, 100.0), (last, 200.0)] {
            graph.nodes[id.0 as usize].is_container = true;
            graph.set_position(id, Point { x, y: 0.0 });
        }
        let boxes = graph
            .nodes
            .iter()
            .map(|n| (n.input_id, n.rect))
            .collect::<BTreeMap<_, _>>();
        let port = |x, tunnel, side| Port {
            point: Point { x, y: 20.0 },
            direction: side,
            side,
            index: 0,
            tunnel: Some(tunnel),
            is_center: true,
        };
        let ovg = VisibilityGraph::from_ports(
            &graph,
            &boxes,
            &graph.graph_node_order(),
            &graph.graph_node_order(),
            [
                (middle, port(140.0, 3, PortSide::Right)),
                (last, port(200.0, 3, PortSide::Left)),
                (first, port(40.0, 7, PortSide::Right)),
                (middle, port(100.0, 7, PortSide::Left)),
            ]
            .into_iter(),
        );
        let adjacent = ovg.edges[ovg.centers[&middle]]
            .iter()
            .map(|slot| ovg.point(slot.neighbor()))
            .collect::<Vec<_>>();
        assert_eq!(
            adjacent,
            vec![Point { x: 100.0, y: 20.0 }, Point { x: 140.0, y: 20.0 }]
        );
    }

    #[test]
    fn centers_retain_removed_objects_and_surviving_bounding_box_order() {
        let mut input = Graph::default();
        let first = input.add_node(node("first", 40.0, 40.0));
        let second = input.add_node(node("second", 40.0, 40.0));
        let portless = input.add_node(node("portless", 40.0, 40.0));
        let mut graph = ArenaGraph::from_input(&input);
        graph.set_position(first, Point { x: 0.0, y: 0.0 });
        graph.set_position(second, Point { x: 100.0, y: 0.0 });
        graph.set_position(portless, Point { x: 200.0, y: 0.0 });
        let boxes = graph
            .nodes
            .iter()
            .map(|node| {
                (
                    node.input_id,
                    Rect {
                        origin: node.position.unwrap(),
                        size: node.rect.size,
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        let first_port = super::super::ports(boxes[&first], ShapeKind::Rectangle)[0];
        let second_port = super::super::ports(boxes[&second], ShapeKind::Rectangle)[0];

        let ovg = VisibilityGraph::from_ports(
            &graph,
            &boxes,
            &graph.graph_node_order(),
            &graph.graph_node_order(),
            [(second, second_port), (first, first_port)].into_iter(),
        );

        assert!(ovg.centers[&first] < ovg.centers[&second]);
        // Go removeIsolatedNodes only replaces Nodes; Centers and occupied
        // entries still reference the disconnected center allocation.
        let removed = ovg.centers[&portless];
        assert_eq!(ovg.point(removed), Point { x: 220.0, y: 20.0 });
        assert!(ovg.edges[removed].is_empty());
        assert_eq!(ovg.search_index(removed), 0);
        assert_eq!(ovg.index_of(ovg.point(removed)), Some(removed));
        let view = ovg.canonical.as_ref().unwrap();
        assert!(!view.build.members.contains(&view.objects[removed]));
    }

    #[test]
    fn nested_node_centers_retain_their_visibility_container() {
        let mut input = Graph::default();
        let container = input.add_node(node("container", 300.0, 240.0));
        let mut nested = node("nested", 40.0, 40.0);
        nested.parent = Some(container);
        let nested = input.add_node(nested);
        let mut graph = ArenaGraph::from_input(&input);
        graph.set_position(container, Point { x: 0.0, y: 0.0 });
        graph.set_position(nested, Point { x: 80.0, y: 70.0 });
        let boxes = graph
            .nodes
            .iter()
            .map(|node| {
                (
                    node.input_id,
                    Rect {
                        origin: node.position.unwrap(),
                        size: node.rect.size,
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        let all_ports = graph
            .nodes
            .iter()
            .flat_map(|node| {
                super::super::ports(boxes[&node.input_id], node.shape)
                    .into_iter()
                    .map(move |port| (node.input_id, port))
            })
            .collect::<Vec<_>>();

        let ovg = VisibilityGraph::from_ports(
            &graph,
            &boxes,
            &graph.graph_node_order(),
            &graph.graph_node_order(),
            all_ports.into_iter(),
        );

        assert_eq!(
            ovg.search_nodes()[ovg.centers[&nested]].container,
            Some(container)
        );
    }

    #[test]
    fn accepted_route_indexes_match_geometric_hop_scoring() {
        let mut input = Graph::default();
        let first = input.add_node(node("first", 40.0, 40.0));
        let second = input.add_node(node("second", 40.0, 40.0));
        let third = input.add_node(node("third", 40.0, 40.0));
        input.add_edge(Edge {
            source: first,
            target: second,
        });
        input.add_edge(Edge {
            source: second,
            target: third,
        });
        let mut graph = ArenaGraph::from_input(&input);
        graph.set_position(first, Point { x: 0.0, y: 0.0 });
        graph.set_position(second, Point { x: 100.0, y: 80.0 });
        graph.set_position(third, Point { x: 200.0, y: 0.0 });
        let boxes = graph
            .nodes
            .iter()
            .map(|node| {
                (
                    node.input_id,
                    Rect {
                        origin: node.position.unwrap(),
                        size: node.rect.size,
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        let all_ports = graph
            .nodes
            .iter()
            .flat_map(|node| {
                super::super::ports(boxes[&node.input_id], node.shape)
                    .into_iter()
                    .map(move |port| (node.input_id, port))
            })
            .collect::<Vec<_>>();
        let ovg = VisibilityGraph::from_ports(
            &graph,
            &boxes,
            &graph.graph_node_order(),
            &graph.graph_node_order(),
            all_ports.into_iter(),
        );
        let segments = ovg.segments();
        let accepted = segments
            .iter()
            .copied()
            .find(|segment| segment[0].x == segment[1].x)
            .unwrap();
        let routes = vec![accepted.to_vec(), Vec::new()];
        let mut index = super::super::search::RouteInteractionIndex::new(&ovg, &graph);
        index.add_route(&graph, 0, &routes[0], "test");

        for candidate in segments {
            assert_eq!(
                index.apply_cost(&graph, 1, candidate, 0.0),
                super::super::search::route_interaction_cost(&graph, 1, &candidate, &routes,),
                "indexed interaction drifted for {candidate:?}",
            );
        }
    }

    #[test]
    fn same_line_port_exception_uses_the_line_endpoints_direction() {
        let rect = Rect {
            origin: Point { x: 0.0, y: 0.0 },
            size: Size {
                width: 300.0,
                height: 300.0,
            },
        };
        let first = Point { x: 150.0, y: 300.0 };
        let second = Point { x: 150.0, y: 320.0 };
        let ports = [Port {
            point: first,
            direction: PortSide::Top,
            side: PortSide::Top,
            index: 0,
            tunnel: None,
            is_center: false,
        }];

        assert!(!passes_through_allowing_direction(
            first,
            second,
            rect,
            &ports,
            Some(PortSide::Bottom),
        ));
        assert!(passes_through_allowing_direction(
            first,
            second,
            rect,
            &ports,
            Some(PortSide::Top),
        ));

        let second_endpoint_port = [Port {
            point: second,
            direction: PortSide::Top,
            side: PortSide::Top,
            index: 0,
            tunnel: None,
            is_center: false,
        }];
        assert!(!passes_through_allowing_direction(
            first,
            second,
            rect,
            &second_endpoint_port,
            Some(PortSide::Bottom),
        ));
    }

    #[test]
    fn ordinary_duplicate_port_coordinates_share_the_ovg_node() {
        let mut input = Graph::default();
        let first = input.add_node(node("first", 100.0, 80.0));
        let second = input.add_node(node("second", 100.0, 80.0));
        let mut graph = ArenaGraph::from_input(&input);
        graph.set_position(first, Point { x: 0.0, y: 0.0 });
        graph.set_position(second, Point { x: 0.0, y: 0.0 });
        let boxes = graph
            .nodes
            .iter()
            .map(|node| {
                (
                    node.input_id,
                    Rect {
                        origin: node.position.unwrap(),
                        size: node.rect.size,
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        let first_port = super::super::ports(boxes[&first], ShapeKind::Rectangle)[0];
        let second_port = super::super::ports(boxes[&second], ShapeKind::Rectangle)[0];
        let shared_point = first_port.point;
        let ovg = VisibilityGraph::from_ports(
            &graph,
            &boxes,
            &graph.graph_node_order(),
            &graph.graph_node_order(),
            [(first, first_port), (second, second_port)].into_iter(),
        );

        let shared_nodes = (0..ovg.edges.len())
            .filter(|&index| ovg.point(index) == shared_point)
            .collect::<Vec<_>>();
        assert_eq!(shared_nodes.len(), 1);
        for owner in [first, second] {
            let owned = shared_nodes
                .iter()
                .copied()
                .filter(|index| ovg.port_owners[*index].contains(&owner))
                .collect::<Vec<_>>();
            assert_eq!(owned.len(), 1);
            let center = ovg.centers[&owner];
            assert!(
                ovg.edges[center]
                    .iter()
                    .any(|slot| slot.neighbor() == owned[0])
            );
        }
    }

    #[test]
    fn hierarchy_coincident_ports_share_the_canonical_node_and_owner_roles() {
        let mut input = Graph::default();
        let first = input.add_node(node("first", 100.0, 80.0));
        let second = input.add_node(node("second", 100.0, 80.0));
        let mut graph = ArenaGraph::from_input(&input);
        graph.assign_hierarchy_member(first, 0, None, 0, 1);
        graph.assign_hierarchy_member(second, 0, None, 0, 1);
        graph.set_position(first, Point { x: 0.0, y: 0.0 });
        graph.set_position(second, Point { x: 0.0, y: 0.0 });
        let boxes = graph
            .nodes
            .iter()
            .map(|node| {
                (
                    node.input_id,
                    Rect {
                        origin: node.position.unwrap(),
                        size: node.rect.size,
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        let first_port = super::super::ports(boxes[&first], ShapeKind::Rectangle)[0];
        let second_port = super::super::ports(boxes[&second], ShapeKind::Rectangle)[0];
        let shared_point = first_port.point;
        let ovg = VisibilityGraph::from_ports(
            &graph,
            &boxes,
            &graph.graph_node_order(),
            &graph.graph_node_order(),
            [(first, first_port), (second, second_port)].into_iter(),
        );
        assert_eq!(
            (0..ovg.edges.len())
                .filter(|&index| ovg.point(index) == shared_point)
                .count(),
            1
        );
        let index = (0..ovg.edges.len())
            .find(|&index| ovg.point(index) == shared_point)
            .unwrap();
        assert_eq!(
            ovg.port_owners[index]
                .iter()
                .copied()
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([first, second])
        );
        for owner in [first, second] {
            assert!(
                ovg.edges[ovg.centers[&owner]]
                    .iter()
                    .any(|slot| slot.neighbor() == index)
            );
        }
    }
}
