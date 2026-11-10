// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Ordinary OVG intersections: pinned ovg.go and ovg_resource.go.
use super::ovg_groups::group;
use super::ovg_ports::{PortBuild, PortList, SharedPortNode};
use super::route_search_guard::OvgBuildBudget;
use super::{ArenaGraph, NodeId, Point, Rect};
use crate::internal::owner_groups::OwnerGroupKind::{Cluster, Sequence};
use d2_rs_core::stable_sort::StableSort;
use std::collections::{BTreeMap, BTreeSet};
pub(super) type Budget<'a, 'b> = Option<&'a OvgBuildBudget<'b>>;
pub(super) fn check(budget: Budget<'_, '_>) -> Result<(), ()> {
    budget.map_or(Ok(()), |b| b.check())
}
// Keep the constant one-unit charge in each scan. An outlined adapter would
// restore the per-step call even when reserve_work's ordinary path inlines.
#[inline(always)]
pub(super) fn step(budget: Budget<'_, '_>) -> Result<(), ()> {
    match budget {
        Some(budget) => budget.reserve_work(1),
        None => Ok(()),
    }
}
pub(super) fn rect(graph: &ArenaGraph, node: NodeId) -> Rect {
    Rect {
        origin: graph
            .position(node)
            .expect("runtime error: invalid memory address or nil pointer dereference"),
        size: graph.nodes[node.0 as usize].rect.size,
    }
}

// Go float keys coalesce signed zeros and never find a NaN, including the
// exact NaN that was inserted. Retain unreachable entries for map iteration.
pub(super) struct AxisMap<T> {
    /// Finite-key values in first-insertion slots. `order` is the Go key
    /// (bit-pattern) traversal order the tree map gave; `index` answers the
    /// per-candidate lookups without a tree walk.
    slots: Vec<(f64, T)>,
    order: BTreeMap<u64, usize>,
    index: crate::internal::FloatKeyHashMap<u64, usize>,
    nan: Vec<(f64, T)>,
}
impl<T> Default for AxisMap<T> {
    fn default() -> Self {
        Self {
            slots: Vec::new(),
            order: BTreeMap::new(),
            index: Default::default(),
            nan: Vec::new(),
        }
    }
}
impl<T: Default> AxisMap<T> {
    pub(super) fn entry(&mut self, value: f64) -> &mut T {
        if value.is_nan() {
            self.nan.push((value, T::default()));
            return &mut self.nan.last_mut().unwrap().1;
        }
        let key = super::go_float_map_key(value);
        let slot = match self.index.entry(key) {
            std::collections::hash_map::Entry::Occupied(entry) => *entry.get(),
            std::collections::hash_map::Entry::Vacant(entry) => {
                let slot = self.slots.len();
                self.slots.push((value, T::default()));
                entry.insert(slot);
                self.order.insert(key, slot);
                slot
            }
        };
        let entry = &mut self.slots[slot];
        entry.0 = value; // Map assignment retains the latest signed-zero key.
        &mut entry.1
    }
    pub(super) fn get(&self, value: f64) -> Option<&T> {
        if value.is_nan() {
            None
        } else {
            self.index
                .get(&super::go_float_map_key(value))
                .map(|&slot| &self.slots[slot].1)
        }
    }
    pub(super) fn entries(&self) -> impl Iterator<Item = &(f64, T)> {
        self.order
            .values()
            .map(|&slot| &self.slots[slot])
            .chain(self.nan.iter())
    }
    fn entries_mut(&mut self) -> impl Iterator<Item = &mut (f64, T)> {
        let mut slots = self.slots.iter_mut().map(Some).collect::<Vec<_>>();
        let ordered = self
            .order
            .values()
            .map(|&slot| slots[slot].take().expect("one axis slot per key"))
            .collect::<Vec<_>>();
        ordered.into_iter().chain(self.nan.iter_mut())
    }
    pub(super) fn len(&self) -> usize {
        self.slots.len() + self.nan.len()
    }
}
fn sort(values: &mut [f64]) {
    values.stable_sort_by(|a, b| {
        if a.is_nan() {
            if b.is_nan() {
                std::cmp::Ordering::Equal
            } else {
                std::cmp::Ordering::Less
            }
        } else if b.is_nan() {
            std::cmp::Ordering::Greater
        } else {
            a.partial_cmp(b).unwrap()
        }
    });
}

fn fixed_ancestor(graph: &ArenaGraph, node: NodeId, budget: Budget<'_, '_>) -> Result<bool, ()> {
    let mut current = Some(node);
    while let Some(node) = current {
        step(budget)?;
        let owner = &graph.nodes[node.0 as usize];
        if owner.fixed_top_left.is_some() {
            return Ok(true);
        }
        match crate::internal::owner_placement::owner_owning_container(graph, node) {
            Ok(parent) => {
                current = parent;
                continue;
            }
            Err(crate::internal::shared_topology::OwnerTopologyError::UnboundVessel(_)) => {}
            Err(error) => panic!("routing owning container: {error:?}"),
        }
        current = if let Some(cluster) = owner
            .cluster
            .filter(|&i| graph.cluster_is_active(&graph.clusters[i]))
        {
            let group = &graph.clusters[cluster];
            if let Some(vessel) = graph
                .nodes
                .iter()
                .find(|n| n.tala_id == group.vessel_tala_id)
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
                        .find(|n| n.tala_id == id)
                        .map(|n| n.input_id)
                })
            } else {
                group
                    .members
                    .first()
                    .and_then(|id| graph.nodes[id.0 as usize].container)
            }
        } else if let Some(sequence) = owner
            .sequence
            .filter(|&i| graph.sequence_is_active(&graph.sequences[i]))
        {
            graph.sequences[sequence].container
        } else {
            owner.container
        };
    }
    Ok(false)
}
pub(super) fn descendant(
    graph: &ArenaGraph,
    node: Option<NodeId>,
    ancestor: NodeId,
    budget: Budget<'_, '_>,
) -> Result<bool, ()> {
    descendant_optional(graph, node, Some(ancestor), budget)
}
pub(super) fn descendant_optional(
    graph: &ArenaGraph,
    mut node: Option<NodeId>,
    ancestor: Option<NodeId>,
    budget: Budget<'_, '_>,
) -> Result<bool, ()> {
    loop {
        step(budget)?;
        if node == ancestor {
            return Ok(true);
        }
        let Some(id) = node else {
            return Ok(ancestor.is_none());
        };
        let n = &graph.nodes[id.0 as usize];
        node = if let Some(container) = n.container {
            Some(container)
        } else if let Some(cluster) = group(graph, id, Cluster) {
            cluster.vessel(graph)
        } else if let Some(sequence) = group(graph, id, Sequence) {
            sequence.vessel(graph)
        } else {
            return Ok(ancestor.is_none());
        };
    }
}

#[derive(Clone, Debug)]
pub(super) struct FixedCache {
    graph: usize,
    nodes: Vec<NodeId>,
    overlaps: BTreeSet<NodeId>,
}
impl PortBuild {
    pub(super) fn fixed_overlaps(
        &mut self,
        graph: &ArenaGraph,
        nodes: &[NodeId],
        budget: Budget<'_, '_>,
    ) -> Result<BTreeSet<NodeId>, ()> {
        let identity = std::ptr::from_ref(graph) as usize;
        for entry in &self.fixed_cache {
            step(budget)?;
            let mut same = entry.nodes.len() == nodes.len();
            if same {
                for (a, b) in entry.nodes.iter().zip(nodes) {
                    step(budget)?;
                    if a != b {
                        same = false;
                        break;
                    }
                }
                if same {
                    check(budget)?;
                }
            } else {
                check(budget)?;
            }
            if entry.graph == identity && same {
                return Ok(entry.overlaps.clone());
            }
        }
        let mut fixed = Vec::new();
        for &node in nodes {
            step(budget)?;
            if fixed_ancestor(graph, node, budget)? {
                fixed.push(node);
            }
        }
        let mut overlaps = BTreeSet::new();
        for node in fixed {
            if overlaps.contains(&node) {
                continue;
            }
            for &other in nodes {
                step(budget)?;
                let a = descendant(graph, Some(node), other, budget)?;
                let b = descendant(graph, Some(other), node, budget)?;
                if node == other || a || b {
                    continue;
                }
                if rect(graph, node).overlaps(rect(graph, other)) {
                    overlaps.insert(node);
                    if fixed_ancestor(graph, other, budget)? {
                        overlaps.insert(other);
                    }
                    break;
                }
            }
        }
        self.fixed_cache.push(FixedCache {
            graph: identity,
            nodes: nodes.to_vec(),
            overlaps: overlaps.clone(),
        });
        Ok(overlaps)
    }

    pub(super) fn add_nodes_intersections(
        &mut self,
        graph: &ArenaGraph,
        graph_nodes: &[NodeId],
        budget: Budget<'_, '_>,
    ) -> Result<(), ()> {
        check(budget)?;
        let mut xs = AxisMap::<()>::default();
        let mut ys = AxisMap::<()>::default();
        for node in self.member_nodes() {
            step(budget)?;
            xs.entry(node.point().x);
            ys.entry(node.point().y);
        }
        self.add_intersections(graph, graph_nodes, xs, ys, budget)
    }

    pub(super) fn add_intersections(
        &mut self,
        graph: &ArenaGraph,
        graph_nodes: &[NodeId],
        xs: AxisMap<()>,
        ys: AxisMap<()>,
        budget: Budget<'_, '_>,
    ) -> Result<(), ()> {
        check(budget)?;
        let count = xs
            .len()
            .checked_mul(ys.len())
            .and_then(|n| u64::try_from(n).ok())
            .ok_or(())?;
        if let Some(b) = budget {
            b.reserve_candidates(count)?;
            b.reserve_work(count)?;
        }
        let mut x_order = Vec::with_capacity(xs.len());
        for &(x, ()) in xs.entries() {
            check(budget)?;
            x_order.push(x);
        }
        if let Some(b) = budget {
            b.reserve_sort_work(x_order.len())?;
        }
        sort(&mut x_order);
        let mut y_order = Vec::with_capacity(ys.len());
        for &(y, ()) in ys.entries() {
            check(budget)?;
            y_order.push(y);
        }
        if let Some(b) = budget {
            b.reserve_sort_work(y_order.len())?;
        }
        sort(&mut y_order);
        let fixed = self.fixed_overlaps(graph, graph_nodes, budget)?;
        let index = PortIndex::new(self, graph, graph_nodes, &fixed, budget)?;
        let mut proximity = AxisMap::<Vec<NodeId>>::default();
        for &x in &x_order {
            step(budget)?;
            for &id in graph_nodes {
                step(budget)?;
                if graph.nodes[id.0 as usize].is_container {
                    continue;
                }
                let r = rect(graph, id);
                if r.origin.x - 20.0 <= x && r.right() + 20.0 >= x {
                    proximity.entry(x).push(id);
                }
            }
        }
        check(budget)?;
        for x in x_order {
            check(budget)?;
            for &y in &y_order {
                check(budget)?;
                if index.too_close(x, y, budget)? {
                    continue;
                }
                step(budget)?;
                let point = Point { x, y };
                step(budget)?;
                let mut near = false;
                for &id in proximity.get(x).into_iter().flatten() {
                    step(budget)?;
                    let r = rect(graph, id);
                    if r.origin.x - 20.0 <= x
                        && x <= r.right() + 20.0
                        && r.origin.y - 20.0 <= y
                        && y <= r.bottom() + 20.0
                    {
                        near = true;
                        break;
                    }
                }
                if near {
                    continue;
                }
                if index.unobstructed(self, graph, point, 2, budget)? {
                    self.add_point(point, budget)?;
                }
            }
        }
        check(budget)
    }
}

#[derive(Default)]
pub(super) struct PortIndex {
    x_by_y: AxisMap<Vec<f64>>,
    y_by_x: AxisMap<Vec<f64>>,
    owners: Vec<NodeId>,
    owners_by_x: AxisMap<Vec<usize>>,
    owners_by_y: AxisMap<Vec<usize>>,
    vertical: AxisMap<Vec<NodeId>>,
    horizontal: AxisMap<Vec<NodeId>>,
}
impl PortIndex {
    pub(super) fn new(
        build: &PortBuild,
        graph: &ArenaGraph,
        graph_nodes: &[NodeId],
        fixed: &BTreeSet<NodeId>,
        budget: Budget<'_, '_>,
    ) -> Result<Self, ()> {
        let mut out = Self::default();
        for (&owner, ports) in build.ports.iter() {
            step(budget)?;
            let owner_index = out.owners.len();
            out.owners.push(owner);
            for &(index, _) in ports {
                step(budget)?;
                let Point { x, y } = build.nodes[index].point();
                out.x_by_y.entry(y).push(x);
                out.y_by_x.entry(x).push(y);
                out.owners_by_x.entry(x).push(owner_index);
                out.owners_by_y.entry(y).push(owner_index);
            }
        }
        for (_, values) in out.x_by_y.entries_mut().chain(out.y_by_x.entries_mut()) {
            if let Some(b) = budget {
                b.reserve_sort_work(values.len())?;
            }
            sort(values);
        }
        for (owners, blockers, horizontal) in [
            (&out.owners_by_x, &mut out.vertical, false),
            (&out.owners_by_y, &mut out.horizontal, true),
        ] {
            for &(axis, _) in owners.entries() {
                step(budget)?;
                for &id in graph_nodes {
                    step(budget)?;
                    let n = &graph.nodes[id.0 as usize];
                    let Some(origin) = graph.position(id) else {
                        continue;
                    };
                    if n.is_container || fixed.contains(&id) {
                        continue;
                    }
                    let (start, end) = if horizontal {
                        (origin.y, origin.y + n.rect.size.height)
                    } else {
                        (origin.x, origin.x + n.rect.size.width)
                    };
                    // math.Min/Max propagate NaN; comparisons then reject it.
                    if !start.is_nan()
                        && !end.is_nan()
                        && start.min(end) <= axis
                        && axis <= start.max(end)
                    {
                        blockers.entry(axis).push(id);
                    }
                }
            }
        }
        Ok(out)
    }
    fn too_close(&self, x: f64, y: f64, budget: Budget<'_, '_>) -> Result<bool, ()> {
        fn within(values: &[f64], coordinate: f64, budget: Budget<'_, '_>) -> Result<bool, ()> {
            let (mut low, mut high) = (0, values.len());
            let threshold = coordinate - 20.0;
            while low < high {
                step(budget)?;
                let mid = (low + high) / 2;
                if values[mid] < threshold {
                    low = mid + 1;
                } else {
                    high = mid;
                }
            }
            let result = low < values.len() && values[low] <= coordinate + 20.0;
            check(budget)?;
            Ok(result)
        }
        if within(self.x_by_y.get(y).map_or(&[], Vec::as_slice), x, budget)? {
            return Ok(true);
        }
        within(self.y_by_x.get(x).map_or(&[], Vec::as_slice), y, budget)
    }
    pub(super) fn unobstructed(
        &self,
        build: &PortBuild,
        graph: &ArenaGraph,
        point: Point,
        required: usize,
        budget: Budget<'_, '_>,
    ) -> Result<bool, ()> {
        self.unobstructed_with_point(build, graph, || Some(point), required, budget)
    }

    /// Source receiver-bound query: entry, per-port alignment and each blocker
    /// read the current Point field after their respective context callbacks.
    pub(super) fn unobstructed_node(
        &self,
        build: &PortBuild,
        graph: &ArenaGraph,
        node: &SharedPortNode,
        required: usize,
        budget: Budget<'_, '_>,
    ) -> Result<bool, ()> {
        self.unobstructed_with_point(
            build,
            graph,
            || node.read().point_ref.as_ref().map(|point| point.get()),
            required,
            budget,
        )
    }

    fn unobstructed_with_point(
        &self,
        build: &PortBuild,
        graph: &ArenaGraph,
        get_point: impl Fn() -> Option<Point>,
        required: usize,
        budget: Budget<'_, '_>,
    ) -> Result<bool, ()> {
        check(budget)?;
        let point =
            get_point().expect("runtime error: invalid memory address or nil pointer dereference");
        let mut by_x = self.owners_by_x.get(point.x).map_or(&[][..], Vec::as_slice);
        let mut by_y = self.owners_by_y.get(point.y).map_or(&[][..], Vec::as_slice);
        let aligned = !by_x.is_empty() || !by_y.is_empty();
        let mut distinct = 0;
        loop {
            step(budget)?;
            if by_x.is_empty() && by_y.is_empty() {
                break;
            }
            let index = if by_y.is_empty() || (!by_x.is_empty() && by_x[0] < by_y[0]) {
                by_x[0]
            } else {
                by_y[0]
            };
            while by_x.first() == Some(&index) {
                step(budget)?;
                by_x = &by_x[1..];
            }
            while by_y.first() == Some(&index) {
                step(budget)?;
                by_y = &by_y[1..];
            }
            let owner = self.owners[index];
            let owner_node = &graph.nodes[owner.0 as usize];
            // Source map lookup yields a nil slice when this indexed owner
            // has since been removed. Keep the captured alignment inventory,
            // but walk the current entry (including an absent/empty one).
            let ports = build
                .ports
                .get(&owner)
                .map(PortList::nodes)
                .unwrap_or_default();
            for node in ports {
                step(budget)?;
                let port = node
                    .read()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .point();
                let point = get_point()
                    .expect("runtime error: invalid memory address or nil pointer dereference");
                if port.x != point.x && port.y != point.y {
                    continue;
                }
                let r = rect(graph, owner);
                if port.x == point.x && (port.x == r.origin.x || port.x == r.right()) {
                    continue;
                }
                if port.y == point.y && (port.y == r.origin.y || port.y == r.bottom()) {
                    continue;
                }
                let blockers = if port.x == point.x {
                    self.vertical.get(port.x)
                } else {
                    self.horizontal.get(port.y)
                };
                let mut direct = true;
                for &other in blockers.into_iter().flatten() {
                    step(budget)?;
                    if owner == other || owner_node.is_invisible {
                        continue;
                    }
                    if let Some(sequence) = group(graph, owner, Sequence)
                        && Some(sequence) == group(graph, other, Sequence)
                    {
                        continue;
                    }
                    let Some(origin) = graph.position(other) else {
                        continue;
                    };
                    // Source passes portNode.Point after the blocker callback.
                    // The earlier alignment/boundary reads cannot substitute
                    // for this live reference. A late nil point is accepted by
                    // layoutgraph.segmentIntersectsBox and does not obstruct.
                    let live_port = node
                        .read()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .point_ref
                        .as_ref()
                        .map(|p| p.get());
                    let live_query = get_point();
                    let blocked = match (live_port, live_query) {
                        (Some(live_port), Some(live_query)) => {
                            super::route_geometry::recovered_segment_intersects_box(
                                live_port,
                                live_query,
                                Rect {
                                    origin,
                                    size: graph.nodes[other.0 as usize].rect.size,
                                },
                            )
                        }
                        _ => false,
                    };
                    if blocked {
                        direct = false;
                        break;
                    }
                }
                if direct {
                    distinct += 1;
                    if distinct == required {
                        return Ok(true);
                    }
                    break;
                }
            }
        }
        check(budget)?;
        Ok(!aligned)
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
    }
    impl Context for Caller {
        fn done(&self) -> Option<bool> {
            None
        }
        fn error(&self) -> Option<ContextError> {
            let poll = self.polls.get() + 1;
            self.polls.set(poll);
            (self.cancel_at != 0 && poll >= self.cancel_at).then_some(ContextError::Cancelled)
        }
    }
    fn graph(size: f64, origin: Point) -> ArenaGraph {
        let input = serde_json::json!({"nodes":[
            {"id":1,"width":size,"height":size,"topLeft":{"x":0,"y":0}},
            {"id":2,"width":size,"height":size,"topLeft":{"x":origin.x,"y":origin.y}}
        ],"edges":[{"id":11,"from":1,"to":2,"points":[]}],"containers":{"0":[1,2]}});
        ArenaGraph::from_input(
            &crate::graphjson::graphjson_to_graph(&serde_json::to_vec(&input).unwrap()).unwrap(),
        )
    }
    #[test]
    fn coincident_ports_charge_one_cartesian_candidate_and_no_extra_allocation() {
        // Independent pinned-source observer: 24 ports share one OVG node.
        let graph = graph(0.0, Point { x: 0.0, y: 0.0 });
        let caller = Caller {
            polls: Cell::new(0),
            cancel_at: 0,
        };
        let budget = OvgBuildBudget::with_context(&caller);
        budget.check().unwrap();
        let mut build = PortBuild::default();
        let nodes = graph.graph_node_order();
        build.add_ports(&graph, &nodes, Some(&budget)).unwrap();
        build
            .add_nodes_intersections(&graph, &nodes, Some(&budget))
            .unwrap();
        assert_eq!(build.nodes.len(), 1);
        assert_eq!(budget.observed_resources(), [354, 1, 0, 1]);
        assert_eq!(caller.polls.get(), 132);
    }
    #[test]
    fn proximity_index_keeps_live_owner_positions_after_construction() {
        // A position changed by a caller callback must affect the geometry
        // query without changing which node identities the index retained.
        let mut graph = graph(40.0, Point { x: 200.0, y: 120.0 });
        graph.materialize_owner_node_point(NodeId(0)).unwrap();
        let point = graph
            .owner_node_point(NodeId(0))
            .unwrap()
            .shared_point()
            .unwrap();
        let nodes = graph.graph_node_order();
        let mut build = PortBuild::default();
        build.add_ports(&graph, &nodes, None).unwrap();
        let index = PortIndex::new(&build, &graph, &nodes, &BTreeSet::new(), None).unwrap();
        assert!(index.vertical.get(10.0).unwrap().contains(&NodeId(0)));
        assert!(
            !index
                .unobstructed(&build, &graph, Point { x: 0.0, y: 130.0 }, 2, None)
                .unwrap()
        );
        point.set(Point {
            x: 1000.0,
            y: 1000.0,
        });
        assert_eq!(rect(&graph, NodeId(0)).origin, point.get());
        // Alignment is against the original ports, while the boundary
        // rejection and clipping read the now-moved owner.
        assert!(
            index
                .unobstructed(&build, &graph, Point { x: 0.0, y: 130.0 }, 2, None)
                .unwrap()
        );
    }
    #[test]
    fn float_axis_maps_keep_nan_entries_unfindable_and_last_zero_sign() {
        let mut map = AxisMap::<Vec<usize>>::default();
        map.entry(-0.0).push(1);
        map.entry(0.0).push(2);
        map.entry(f64::NAN).push(3);
        map.entry(f64::NAN).push(4);
        assert_eq!(map.len(), 3);
        assert_eq!(map.get(-0.0).unwrap(), &[1, 2]);
        assert_eq!(map.entries().next().unwrap().0.to_bits(), 0);
        assert!(map.get(f64::NAN).is_none());
    }
    #[test]
    fn separated_and_negative_boxes_reserve_each_accepted_intersection_once() {
        // Four unmodified Go observer repetitions agree on these inventories.
        for (size, work, polls) in [(40.0, 2024, 2207), (-40.0, 1984, 2167)] {
            let graph = graph(size, Point { x: 200.0, y: 120.0 });
            let caller = Caller {
                polls: Cell::new(0),
                cancel_at: 0,
            };
            let budget = OvgBuildBudget::with_context(&caller);
            budget.check().unwrap();
            let mut build = PortBuild::default();
            let nodes = graph.graph_node_order();
            build.add_ports(&graph, &nodes, Some(&budget)).unwrap();
            assert_eq!(build.nodes.len(), 24);
            build
                .add_nodes_intersections(&graph, &nodes, Some(&budget))
                .unwrap();
            assert_eq!(build.nodes.len(), 42);
            assert_eq!(budget.observed_resources(), [work, 42, 0, 100]);
            assert_eq!(caller.polls.get(), polls);
        }
    }
}
