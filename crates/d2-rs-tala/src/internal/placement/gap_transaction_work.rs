// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Geometry-transaction work from D2 v0.9.0 layoutgraph/transaction.go.
//!
//! The loops deliberately charge individual references. An aggregate vessel
//! and the retained member occupying its Rust arena slot are separate source
//! objects. Canonical NodeId handles and separate compact-vessel namespaces keep
//! equal numeric TALA IDs from merging those objects.
//!
//! Go's randomized map traversal order is not represented by the arena's
//! ordered ownership maps. Post-operation validation uses the shared source
//! Commit validator through GeometryCheckpoint; it is not shadow accounting.

use super::optimizer_spatial_index::owner_spacing_delta;
use super::*;
use d2_rs_core::stable_sort::StableSort;

const MAX_TRANSACTION_NODES: usize = 10_000;
const MAX_TRANSACTION_EDGES: usize = 50_000;
const MAX_TRANSACTION_OVERLAP_REFERENCES: usize = 4_000_000;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct GapTransactionBox {
    pub(super) node: NodeId,
    pub(super) tala_id: i64,
    pub(super) position: Option<Point>,
    pub(super) size: Size,
}

impl GapTransactionBox {
    fn finite(&self) -> bool {
        self.position.is_some_and(|point| {
            point.x.is_finite()
                && point.y.is_finite()
                && self.size.width.is_finite()
                && self.size.height.is_finite()
                && (point.x + self.size.width).is_finite()
                && (point.y + self.size.height).is_finite()
                && self.size.width >= 0.0
                && self.size.height >= 0.0
        })
    }
}

impl ArenaGraph {
    pub(super) fn gap_transaction_boxes(&self) -> Vec<GapTransactionBox> {
        self.graph_node_order()
            .into_iter()
            .map(|node| GapTransactionBox {
                node,
                tala_id: self.active_node_tala_id(node),
                position: self.active_node_position(node),
                size: self.active_node_size(node),
            })
            .collect()
    }

    // Go range retains the header, then loads each pointer before Step. Geometry
    // is read afterward; a context callback can change both future slots and
    // the shared point of the pointer already loaded for this iteration.
    fn transaction_member_range(&self) -> Box<dyn Iterator<Item = Option<NodeId>>> {
        match self.owner_graph_member_header(super::shared_topology::OwnerGraphId(0)) {
            Ok(header) => Box::new(header.iter_slots()),
            Err(_) => Box::new(self.graph_node_order().into_iter().map(Some)),
        }
    }

    fn transaction_live_box(&self, node: NodeId) -> GapTransactionBox {
        GapTransactionBox {
            node,
            tala_id: self.active_node_tala_id(node),
            position: self.active_node_position(node),
            size: self.active_node_size(node),
        }
    }

    /// GraphState.updateContext followed by captureGeometryStateContext.
    /// Both initial construction and UpdateState execute these same loops.
    pub(super) fn gap_transaction_update_work(
        &self,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<(), limits::Error> {
        self.geometry_transaction_capture_walk(guard, &mut |_, _| {})
    }

    /// The accounting and data visitor share the source's exact poll boundary.
    pub(super) fn geometry_transaction_capture_walk<F: FnMut(&Self, GeometryCaptureEvent)>(
        &self,
        guard: &mut limits::WorkGuard<'_>,
        visitor: &mut F,
    ) -> Result<(), limits::Error> {
        self.geometry_transaction_capture_walk_with_routes(
            guard,
            visitor,
            &mut |_, _, _| Ok(()),
            true,
        )
    }

    // Packing's members are real source pointers. Older numerical geometry
    // adapters explicitly project compact carriers at their root boundary.
    pub(super) fn geometry_transaction_capture_walk_with_routes<'context, F, R>(
        &self,
        guard: &mut limits::WorkGuard<'context>,
        visitor: &mut F,
        routes: &mut R,
        project_legacy_roots: bool,
    ) -> Result<(), limits::Error>
    where
        F: FnMut(&Self, GeometryCaptureEvent),
        R: FnMut(
            &Self,
            &GeometrySeenSet,
            &mut limits::WorkGuard<'context>,
        ) -> Result<(), limits::Error>,
    {
        #[cfg(feature = "diagnostic-traces")]
        crate::internal::call_counts::count!(GeometrySnapshot);
        #[cfg(feature = "diagnostic-traces")]
        guard.observe_transaction_graph(self, limits::TransactionPollSite::General);
        use super::owner_groups::OwnerGroupKind;
        use super::shared_topology::OwnerGraphId;
        let receiver = OwnerGraphId(0);
        let members = self.owner_graph_member_header(receiver).ok();
        let count = members
            .as_ref()
            .map(|h| h.len())
            .unwrap_or(self.node_order.len());
        if count > MAX_TRANSACTION_NODES {
            return Err(limits::Error::Invalid(format!(
                "TALA transaction node count exceeds limit {MAX_TRANSACTION_NODES}"
            )));
        }
        let edges = self
            .owner_graph_edge_header(receiver)
            .map(|h| h.len())
            .unwrap_or(self.edge_order.len());
        if edges > MAX_TRANSACTION_EDGES {
            return Err(limits::Error::Invalid(format!(
                "TALA transaction edge count exceeds limit {MAX_TRANSACTION_EDGES}"
            )));
        }
        let collections = self
            .geometry_update_collection_lengths(receiver)
            .unwrap_or([
                ("container", self.containers.len()),
                ("cluster", self.clusters.len()),
                ("sequence", self.sequences.len()),
                ("tree", self.tree_sentinels.len()),
                ("node-tree", self.tree_routing_nodes.len()),
                ("hub", self.hubs.len()),
                ("direction", self.directions.len()),
                ("sibling", self.common_uncle_siblings.len()),
            ]);
        for (name, count) in collections {
            if count > MAX_TRANSACTION_NODES + 1 {
                return Err(limits::Error::Invalid(format!(
                    "TALA transaction {name} map exceeds limit {}",
                    MAX_TRANSACTION_NODES + 1
                )));
            }
        }
        visitor(self, GeometryCaptureEvent::Begin);
        let mut seen = GeometrySeenSet::new(self.nodes.len());
        let roots: Box<dyn Iterator<Item = Option<NodeId>>> = match members {
            Some(header) => Box::new(header.iter_slots()),
            None => Box::new(self.graph_node_order().into_iter().map(Some)),
        };
        for node in roots.flatten() {
            if !project_legacy_roots {
                self.geometry_record_descendants(
                    Some(GeometrySnapshotObject::Node(node)),
                    &mut seen,
                    guard,
                    visitor,
                )?;
            } else if self.active_aggregate_owner(node) == node {
                self.geometry_record_descendants(
                    Some(self.geometry_active_object(node)),
                    &mut seen,
                    guard,
                    visitor,
                )?;
            }
        }
        if let Some(map) = self.owner_container_map(receiver) {
            for (container, children) in map.live_entries() {
                guard.step()?;
                self.geometry_record_descendants(
                    container.map(GeometrySnapshotObject::Node),
                    &mut seen,
                    guard,
                    visitor,
                )?;
                for child in children.iter_slots() {
                    self.geometry_record_descendants(
                        child.map(GeometrySnapshotObject::Node),
                        &mut seen,
                        guard,
                        visitor,
                    )?;
                }
            }
        } else {
            for (container, children) in self.containers.iter() {
                guard.step()?;
                self.geometry_record_descendants(
                    container.map(GeometrySnapshotObject::Node),
                    &mut seen,
                    guard,
                    visitor,
                )?;
                for child in children {
                    self.geometry_record_descendants(
                        Some(GeometrySnapshotObject::Node(child)),
                        &mut seen,
                        guard,
                        visitor,
                    )?;
                }
            }
        }
        for kind in [OwnerGroupKind::Cluster, OwnerGroupKind::Sequence] {
            if let Ok(map) = self.owner_graph_group_map(receiver, kind) {
                for group in self
                    .owner_group_map_values(&map)
                    .expect("geometry update group map")
                {
                    guard.step()?;
                    let Some(group) = group else {
                        continue;
                    };
                    if kind == OwnerGroupKind::Cluster {
                        visitor(self, GeometryCaptureEvent::Cluster(group));
                    }
                    let vessel = self
                        .owner_group_fields(group)
                        .expect("geometry update group")
                        .vessel;
                    self.geometry_record_descendants(
                        vessel.map(GeometrySnapshotObject::Node),
                        &mut seen,
                        guard,
                        visitor,
                    )?;
                    let container = self
                        .owner_group_fields(group)
                        .expect("geometry update group")
                        .container;
                    self.geometry_record_descendants(
                        container.map(GeometrySnapshotObject::Node),
                        &mut seen,
                        guard,
                        visitor,
                    )?;
                    let members = self
                        .owner_group_fields(group)
                        .expect("geometry update group")
                        .nodes;
                    for node in members.iter_slots() {
                        self.geometry_record_descendants(
                            node.map(GeometrySnapshotObject::Node),
                            &mut seen,
                            guard,
                            visitor,
                        )?;
                    }
                }
            }
            let count = match kind {
                OwnerGroupKind::Cluster => self.clusters.len(),
                OwnerGroupKind::Sequence => self.sequences.len(),
            };
            for index in 0..count {
                let vessel = match kind {
                    OwnerGroupKind::Cluster => TopologyVessel::Cluster(index),
                    OwnerGroupKind::Sequence => TopologyVessel::Sequence(index),
                };
                if self.owner_group_binding(vessel).is_some() {
                    continue;
                }
                guard.step()?;
                self.geometry_record_descendants(
                    Some(self.geometry_vessel_object(vessel)),
                    &mut seen,
                    guard,
                    visitor,
                )?;
                let (container, nodes) = match kind {
                    OwnerGroupKind::Cluster => {
                        let group = &self.clusters[index];
                        (
                            group
                                .members
                                .first()
                                .and_then(|n| self.nodes[n.0 as usize].container),
                            &group.members,
                        )
                    }
                    OwnerGroupKind::Sequence => (
                        self.sequences[index].container,
                        &self.sequences[index].members,
                    ),
                };
                self.geometry_record_descendants(
                    container.map(GeometrySnapshotObject::Node),
                    &mut seen,
                    guard,
                    visitor,
                )?;
                for &node in nodes {
                    self.geometry_record_descendants(
                        Some(GeometrySnapshotObject::Node(node)),
                        &mut seen,
                        guard,
                        visitor,
                    )?;
                }
            }
        }
        routes(self, &seen, guard)?;
        // Every actual entry is charged, including duplicate/nil Tree values.
        if let Ok(map) = self.owner_graph_node_to_tree_map(receiver) {
            for key in map.live_keys() {
                // Go's range binds the Tree value before Step; its Orientation
                // is read afterward, even if the callback replaces the entry.
                let tree = self
                    .owner_node_to_tree_map_entry(&map, key)
                    .expect("capture NodeToTree value");
                guard.step()?;
                if let Some(Some(tree)) = tree {
                    visitor(self, GeometryCaptureEvent::Tree(tree));
                }
            }
        } else {
            for _ in &self.tree_routing_nodes {
                guard.step()?;
            }
        }
        visitor(self, GeometryCaptureEvent::End);
        guard.finish()?;
        guard.finish()
    }

    fn geometry_vessel_object(&self, vessel: TopologyVessel) -> GeometrySnapshotObject {
        self.topology_vessel_node(vessel)
            .map(GeometrySnapshotObject::Node)
            .unwrap_or_else(|| match vessel {
                TopologyVessel::Cluster(i) => GeometrySnapshotObject::Cluster(i),
                TopologyVessel::Sequence(i) => GeometrySnapshotObject::Sequence(i),
            })
    }
    fn geometry_active_object(&self, node: NodeId) -> GeometrySnapshotObject {
        if self.topology_node_vessel(node).is_some() {
            return GeometrySnapshotObject::Node(node);
        }
        if let Some(index) = self.active_cluster_index(node) {
            return self.geometry_vessel_object(TopologyVessel::Cluster(index));
        }
        if let Some(index) = self.active_sequence_index(node) {
            return self.geometry_vessel_object(TopologyVessel::Sequence(index));
        }
        GeometrySnapshotObject::Node(node)
    }
    fn geometry_record_descendants<F: FnMut(&Self, GeometryCaptureEvent)>(
        &self,
        root: Option<GeometrySnapshotObject>,
        seen: &mut GeometrySeenSet,
        guard: &mut limits::WorkGuard<'_>,
        visitor: &mut F,
    ) -> Result<(), limits::Error> {
        let mut pending = root.into_iter().collect::<Vec<_>>();
        while let Some(object) = pending.pop() {
            if seen.contains(&object) {
                continue;
            }
            if seen.len() >= MAX_TRANSACTION_NODES {
                return Err(limits::Error::Invalid(format!(
                    "TALA transaction unique node snapshot exceeds limit {MAX_TRANSACTION_NODES}"
                )));
            }
            guard.step()?;
            visitor(self, GeometryCaptureEvent::Node(object));
            seen.insert(object);
            if let GeometrySnapshotObject::Node(node) = object {
                let receiver = super::shared_topology::OwnerGraphId(0);
                let children_len = self
                    .with_owner_container_entry(receiver, Some(node), |slice| {
                        slice.map(|slice| slice.len())
                    })
                    .flatten();
                if let Some(len) = children_len {
                    for _ in 0..len {
                        guard.step()?;
                    }
                }
                // Source re-reads g.Containers[node] for append after charging
                // the earlier range length; callbacks can replace that header.
                self.with_owner_container_entry(receiver, Some(node), |slice| {
                    if let Some(slice) = slice {
                        pending.extend(
                            slice
                                .borrowed_slots()
                                .flatten()
                                .map(GeometrySnapshotObject::Node),
                        );
                    }
                });
            }
        }
        Ok(())
    }

    /// NewRequestTransaction's padding and initial overlap sweep. UpdateState
    /// intentionally does not repeat this work or replace its overlap baseline.
    pub(super) fn gap_transaction_initial_overlap_work(
        &self,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<(), limits::Error> {
        self.transaction_overlap_snapshot_checked(guard).map(|_| ())
    }

    /// Source buildTransactionOverlaps: return the very overlap exceptions
    /// constructed by the charged sweep, so callers do not run a second,
    /// unguarded all-pairs scan or charge synthetic transaction work.
    pub(super) fn transaction_overlap_snapshot_checked(
        &self,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<(BTreeSet<(NodeId, NodeId)>, BTreeSet<(NodeId, NodeId)>), limits::Error> {
        self.transaction_overlap_snapshot_with_reference_limit(
            guard,
            MAX_TRANSACTION_OVERLAP_REFERENCES as i64,
        )
    }

    pub(super) fn transaction_overlap_snapshot_with_reference_limit(
        &self,
        guard: &mut limits::WorkGuard<'_>,
        reference_limit: i64,
    ) -> Result<(BTreeSet<(NodeId, NodeId)>, BTreeSet<(NodeId, NodeId)>), limits::Error> {
        let mut overlaps = BTreeSet::new();
        let mut exact_overlaps = BTreeSet::new();
        let padding = self.gap_transaction_sweep_padding_work(guard)?;
        let mut placed = Vec::new();
        for (index, node) in self.transaction_member_range().enumerate() {
            guard.step()?;
            let node =
                node.expect("runtime error: invalid memory address or nil pointer dereference");
            if self.transaction_live_box(node).finite() {
                placed.push((index, node));
            }
        }
        guard.step()?;
        // Collection retains pointers, not boxes. The pre-sort Step can mutate
        // positions after their finite-box check, including introducing NaN.
        placed.stable_sort_by(|(left_index, left), (right_index, right)| {
            let left = self.transaction_live_box(*left);
            let right = self.transaction_live_box(*right);
            let left_x = left
                .position
                .expect("runtime error: invalid memory address or nil pointer dereference")
                .x;
            let right_x = right
                .position
                .expect("runtime error: invalid memory address or nil pointer dereference")
                .x;
            if left_x != right_x {
                return if left_x < right_x {
                    std::cmp::Ordering::Less
                } else if right_x < left_x {
                    std::cmp::Ordering::Greater
                } else {
                    std::cmp::Ordering::Equal
                };
            }
            left.tala_id
                .cmp(&right.tala_id)
                .then(left_index.cmp(right_index))
        });
        let mut active: Vec<NodeId> = Vec::new();
        let mut retained_references = 0;
        for (_, current_node) in placed {
            guard.step()?;
            let mut kept = Vec::with_capacity(active.len() + 1);
            for other_node in active {
                guard.step()?;
                if other_node == current_node {
                    continue;
                }
                let current = self.transaction_live_box(current_node);
                let other = self.transaction_live_box(other_node);
                let position = current
                    .position
                    .expect("runtime error: invalid memory address or nil pointer dereference");
                let other_position = other
                    .position
                    .expect("runtime error: invalid memory address or nil pointer dereference");
                if other_position.x + other.size.width + padding <= position.x {
                    continue;
                }
                kept.push(other_node);
                if other_position.y >= position.y + current.size.height + padding
                    || position.y >= other_position.y + other.size.height + padding
                {
                    continue;
                }
                let pair = if current.node < other.node {
                    (current.node, other.node)
                } else {
                    (other.node, current.node)
                };
                if gap_boxes_overlap(current, other, 0.0) {
                    gap_retain_pair(&mut retained_references, reference_limit)?;
                    exact_overlaps.insert(pair);
                }
                // Ordinary source doesOverlap validates raw incidences and
                // short-circuits the reverse predicate. Dense numeric readers
                // can discard nil slots and cannot implement this contract.
                if gap_boxes_overlap(
                    current,
                    other,
                    owner_spacing_delta(self, current.node, other.node, position),
                ) || gap_boxes_overlap(
                    other,
                    current,
                    owner_spacing_delta(self, other.node, current.node, other_position),
                ) {
                    gap_retain_pair(&mut retained_references, reference_limit)?;
                    overlaps.insert(pair);
                }
            }
            kept.push(current_node);
            active = kept;
        }
        guard.finish()?;
        Ok((overlaps, exact_overlaps))
    }

    pub(super) fn gap_transaction_sweep_padding_work(
        &self,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<f64, limits::Error> {
        let mut padding: f64 = 120.0;
        let mut max_margin: f64 = 0.0;
        let mut max_loop_offset: f64 = 0.0;
        let mut edges = BTreeSet::new();
        for slot in self.transaction_member_range() {
            guard.step()?;
            let node = slot.ok_or_else(|| {
                limits::Error::Invalid("TALA transaction contains a nil node".into())
            })?;
            let current = self.transaction_live_box(node);
            let node = &self.nodes[current.node.0 as usize];
            if current.position.is_some() && !current.finite() {
                if node.is_container && self.container_node_order(Some(current.node)).is_empty() {
                    continue;
                }
                let point = current.position.expect("positioned invalid box");
                let display = super::owner_placement::go_general_float;
                return Err(limits::Error::Invalid(format!(
                    "layout invariant violated: transaction node {} has invalid geometry (x={} y={} width={} height={})",
                    current.tala_id,
                    display(point.x),
                    display(point.y),
                    display(current.size.width),
                    display(current.size.height),
                )));
            }
            let is_vessel = current.tala_id != node.tala_id;
            let margins = if is_vessel {
                Insets::uniform(0.0)
            } else {
                node.layout_margins
            };
            for margin in [margins.top, margins.right, margins.bottom, margins.left] {
                if !margin.is_finite() {
                    return Err(limits::Error::Invalid(format!(
                        "layout invariant violated: transaction node {} has non-finite margin",
                        current.tala_id
                    )));
                }
                max_margin = max_margin.max(margin);
            }
            if !is_vessel && let Some(offsets) = node.loop_offsets.as_ref() {
                for &offset in offsets.values() {
                    guard.step()?;
                    if !offset.is_finite() {
                        return Err(limits::Error::Invalid(format!(
                            "layout invariant violated: transaction node {} has a non-finite loop offset",
                            current.tala_id
                        )));
                    }
                    max_loop_offset = max_loop_offset.max(offset);
                }
            }
            // This is the unfiltered Node.Edges slice, including references
            // absent from Graph.Edges and repeated aggregate incidences.
            for edge in self.nodes[current.node.0 as usize]
                .edges
                .header()
                .iter_slots()
            {
                guard.step()?;
                let edge = edge.ok_or_else(|| {
                    limits::Error::Invalid(format!(
                        "TALA transaction node {} contains a nil edge",
                        current.tala_id
                    ))
                })?;
                edges.insert(edge);
            }
        }
        let primary = self
            .owner_graph_edge_header(super::shared_topology::OwnerGraphId(0))
            .map(|header| Box::new(header.iter_slots()) as Box<dyn Iterator<Item = Option<EdgeId>>>)
            .unwrap_or_else(|_| Box::new(self.edge_order.header().iter_slots()));
        for edge in primary {
            guard.step()?;
            edges.insert(edge.ok_or_else(|| {
                limits::Error::Invalid("TALA transaction contains a nil edge".into())
            })?);
        }
        for edge in edges {
            guard.step()?;
            let edge = &self.edge_records[edge.0 as usize];
            padding = padding.max(edge.min_width).max(edge.min_height);
        }
        padding = padding
            .max(2.0 * max_margin)
            .max(20.0 + 2.0 * max_loop_offset);
        if !padding.is_finite() {
            return Err(limits::Error::Invalid(
                "layout invariant violated: transaction overlap padding is non-finite".into(),
            ));
        }
        guard.finish()?;
        Ok(padding)
    }
}

#[derive(Clone, Copy)]
pub(super) enum GeometryCaptureEvent {
    Begin,
    End,
    Node(GeometrySnapshotObject),
    Cluster(super::owner_groups::OwnerGroupId),
    Tree(super::owner_tree_map::OwnerTreeId),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(super) enum GeometrySnapshotObject {
    Node(NodeId),
    Cluster(usize),
    Sequence(usize),
}

/// The capture walk's ordered seen set. Canonical nodes are dense flags;
/// vessels and out-of-range node witnesses keep the ordered set. Iteration
/// matches `BTreeSet<GeometrySnapshotObject>`: in-range nodes ascend, then
/// the remaining entries, whose node keys all exceed the dense range.
pub(super) struct GeometrySeenSet {
    nodes: Vec<bool>,
    node_count: usize,
    other: BTreeSet<GeometrySnapshotObject>,
}
impl GeometrySeenSet {
    fn new(node_count: usize) -> Self {
        Self {
            nodes: vec![false; node_count],
            node_count: 0,
            other: BTreeSet::new(),
        }
    }
    fn contains(&self, object: &GeometrySnapshotObject) -> bool {
        if let GeometrySnapshotObject::Node(node) = *object {
            if let Some(&seen) = self.nodes.get(node.0 as usize) {
                return seen;
            }
        }
        self.other.contains(object)
    }
    fn insert(&mut self, object: GeometrySnapshotObject) {
        if let GeometrySnapshotObject::Node(node) = object {
            if let Some(seen) = self.nodes.get_mut(node.0 as usize) {
                self.node_count += usize::from(!std::mem::replace(seen, true));
                return;
            }
        }
        self.other.insert(object);
    }
    fn len(&self) -> usize {
        self.node_count + self.other.len()
    }
    pub(super) fn iter(&self) -> impl Iterator<Item = GeometrySnapshotObject> + '_ {
        self.nodes
            .iter()
            .enumerate()
            .filter(|&(_, &seen)| seen)
            .map(|(index, _)| GeometrySnapshotObject::Node(NodeId(index as _)))
            .chain(self.other.iter().copied())
    }
}

fn gap_sort_sweep_boxes(boxes: &mut [(usize, GapTransactionBox)]) {
    boxes.stable_sort_by(|(left_index, left), (right_index, right)| {
        let left_x = left.position.expect("finite transaction box").x;
        let right_x = right.position.expect("finite transaction box").x;
        if left_x < right_x {
            std::cmp::Ordering::Less
        } else if right_x < left_x {
            std::cmp::Ordering::Greater
        } else {
            left.tala_id
                .cmp(&right.tala_id)
                .then(left_index.cmp(right_index))
        }
    });
}

fn gap_boxes_overlap(left: GapTransactionBox, right: GapTransactionBox, delta: f64) -> bool {
    let (Some(left_position), Some(right_position)) = (left.position, right.position) else {
        return false;
    };
    // node.go boxesOverlapWithPadding rejects separated X intervals first.
    // This is observably different from positive X overlap tests when a
    // callback introduces NaN after the initial finite-box collection.
    if left_position.x >= right_position.x + right.size.width + delta
        || right_position.x >= left_position.x + left.size.width + delta
    {
        return false;
    }
    left_position.y < right_position.y + right.size.height + delta
        && right_position.y < left_position.y + left.size.height + delta
}

fn gap_retain_pair(retained: &mut i64, reference_limit: i64) -> Result<(), limits::Error> {
    if reference_limit < 2 || *retained > reference_limit - 2 {
        return Err(limits::Error::Invalid(format!(
            "TALA transaction overlap references exceed limit {reference_limit}"
        )));
    }
    *retained += 2;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Background;
    impl limits::Context for Background {
        fn done(&self) -> Option<bool> {
            None
        }
        fn error(&self) -> Option<limits::ContextError> {
            None
        }
    }

    fn two_node_graph() -> ArenaGraph {
        let mut input = Graph::default();
        let a = input.add_node(crate::Node::new(
            "a",
            Size {
                width: 10.0,
                height: 10.0,
            },
        ));
        let b = input.add_node(crate::Node::new(
            "b",
            Size {
                width: 10.0,
                height: 10.0,
            },
        ));
        input.add_edge(crate::Edge {
            source: a,
            target: b,
        });
        let mut graph = ArenaGraph::from_input(&input);
        graph.containers.clear();
        for (index, node) in [a, b].into_iter().enumerate() {
            graph.nodes[node.0 as usize].tala_id = index as i64 + 1;
            // The prepared Go control creates Node directly, without Prepare;
            // its LoopOffsets map is nil until Prepare runs.
            assert!(graph.nodes[node.0 as usize].loop_offsets.is_none());
            graph.set_position(
                node,
                Point {
                    x: index as f64 * 1000.0,
                    y: 0.0,
                },
            );
        }
        graph
    }

    #[test]
    fn initial_transaction_work_traverses_capture_padding_and_sweep() {
        let graph = two_node_graph();
        let mut guard = limits::WorkGuard::new(Some(&Background), "Transaction", 1000).unwrap();
        graph.gap_transaction_update_work(&mut guard).unwrap();
        assert_eq!(guard.used(), 2);
        graph
            .gap_transaction_initial_overlap_work(&mut guard)
            .unwrap();
        assert_eq!(guard.used(), 14);
        graph.gap_transaction_update_work(&mut guard).unwrap();
        assert_eq!(
            guard.used(),
            16,
            "UpdateState does not rebuild initial overlaps"
        );
    }

    #[test]
    fn initial_empty_transaction_charges_the_presort_reference() {
        let graph = ArenaGraph::from_input(&Graph::default());
        let mut guard = limits::WorkGuard::new(Some(&Background), "Transaction", 1000).unwrap();
        graph.gap_transaction_update_work(&mut guard).unwrap();
        assert_eq!(guard.used(), 0);
        graph
            .gap_transaction_initial_overlap_work(&mut guard)
            .unwrap();
        assert_eq!(guard.used(), 1);
    }

    #[test]
    fn rejected_initial_unit_preserves_the_first_rejected_counter() {
        let graph = two_node_graph();
        let mut guard = limits::WorkGuard::new(Some(&Background), "Transaction", 13).unwrap();
        graph.gap_transaction_update_work(&mut guard).unwrap();
        assert!(matches!(
            graph.gap_transaction_initial_overlap_work(&mut guard),
            Err(limits::Error::Limit { limit: 13, .. })
        ));
        assert_eq!(guard.used(), 14);
    }

    #[test]
    fn real_commit_noop_and_moved_validation_match_source_work() {
        let mut graph = two_node_graph();
        let mut capture = limits::WorkGuard::new(Some(&Background), "Transaction", 1000).unwrap();
        let prior = GeometryCheckpoint::capture(&graph, &mut capture).unwrap();
        let overlaps = BTreeSet::new();
        let mut noop = limits::WorkGuard::new(Some(&Background), "Transaction", 1000).unwrap();
        noop.step().unwrap();
        assert!(
            prior
                .validate_commit(&graph, &overlaps, &overlaps, &Background, &mut noop)
                .unwrap()
        );
        assert_eq!(noop.used(), 11);
        graph.set_position(NodeId(1), Point { x: 160.0, y: 0.0 });
        let mut moved = limits::WorkGuard::new(Some(&Background), "Transaction", 1000).unwrap();
        moved.step().unwrap();
        assert!(
            prior
                .validate_commit(&graph, &overlaps, &overlaps, &Background, &mut moved)
                .unwrap()
        );
        assert_eq!(moved.used(), 20);
    }
}
