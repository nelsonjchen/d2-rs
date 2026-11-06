// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! OVG.Connect axis membership and addRoute's static edge publication.
#[cfg(test)]
use super::routed_edge_set::Edge;
use super::routed_edge_set::{NodeRef, Work};
use super::{Point, ovg_ports::push_pointer};
use crate::internal::owner_points::SharedOwnerPoint;
use std::collections::BTreeMap;

/// A static edge endpoint: the node's retained TopLeft allocation when it
/// has one, read live as `NodeRef::point` does, else its captured value.
/// `NodeRef` keeps both; a static edge never reads the value of a shared
/// point, so it stores only the source that `point()` would use.
#[derive(Clone, Debug)]
pub(super) enum StaticPoint {
    Shared(SharedOwnerPoint),
    Value(Point),
}
impl StaticPoint {
    fn of(value: Point, shared: Option<SharedOwnerPoint>) -> Self {
        shared.map_or(Self::Value(value), Self::Shared)
    }
    pub(super) fn point(&self) -> Point {
        match self {
            Self::Shared(point) => point.get(),
            Self::Value(point) => *point,
        }
    }
}
/// One OVG edge of the shared static index: its endpoints as positions in
/// `StaticEdges::points`. A position below `StaticEdges::ovg_points` is the
/// endpoint's OVG node slot (`NodeIdentity::Ovg`); later positions are the
/// values of coordinate-only segments.
#[derive(Clone, Copy, Debug)]
pub(super) struct StaticEdge {
    from: u32,
    to: u32,
    pub(super) distance: f64,
}

#[derive(Debug)]
pub(super) struct Bucket {
    pub(super) axis: f64,
    pub(super) edges: Vec<usize>,
}
impl Clone for Bucket {
    fn clone(&self) -> Self {
        let mut edges = Vec::with_capacity(self.edges.capacity());
        edges.extend_from_slice(&self.edges);
        Self {
            axis: self.axis,
            edges,
        }
    }
}
#[derive(Clone, Debug, Default)]
pub(super) struct Axes {
    pub(super) buckets: BTreeMap<u64, Bucket>,
}
impl Axes {
    pub(super) fn append(&mut self, axis: f64, edge: usize) {
        // Connect selects an axis only after coordinate equality; NaN cannot
        // be that axis. Go map assignment retains the newly assigned zero sign.
        assert!(!axis.is_nan());
        let bucket = self
            .buckets
            .entry(super::go_float_map_key(axis))
            .or_insert_with(|| Bucket {
                axis,
                edges: Vec::new(),
            });
        bucket.axis = axis;
        push_pointer(&mut bucket.edges, edge);
    }
    #[cfg(test)]
    pub(super) fn clear(&mut self) {
        self.buckets.clear();
    }
}
/// The OVG's edge objects and Connect axis membership. addRoute only reads
/// them, so every flavor's index over one OVG shares a single copy.
#[derive(Debug, Default)]
pub(super) struct StaticEdges {
    pub(super) edges: Vec<StaticEdge>,
    /// Endpoint table: each OVG endpoint once, at its node slot, then the
    /// two values of every coordinate-only segment.
    points: Vec<Option<StaticPoint>>,
    ovg_points: u32,
    pub(super) horizontal: Axes,
    pub(super) vertical: Axes,
}
impl Clone for StaticEdges {
    fn clone(&self) -> Self {
        let mut edges = Vec::with_capacity(self.edges.capacity());
        edges.extend(self.edges.iter().copied());
        Self {
            edges,
            points: self.points.clone(),
            ovg_points: self.ovg_points,
            horizontal: self.horizontal.clone(),
            vertical: self.vertical.clone(),
        }
    }
}
impl StaticEdges {
    pub(super) fn from_build(build: &super::ovg_ports::PortBuild) -> Self {
        Self::with_axes(
            build,
            build.horizontal_edges.clone(),
            build.vertical_edges.clone(),
        )
    }
    /// `from_build` for a frozen build whose Connect axes nothing else reads:
    /// move them in (a clone keeps every bucket's capacity, so the moved axes
    /// are what a clone would have been) instead of keeping a second copy.
    pub(super) fn from_build_taking_axes(build: &mut super::ovg_ports::PortBuild) -> Self {
        let horizontal = std::mem::take(&mut build.horizontal_edges);
        let vertical = std::mem::take(&mut build.vertical_edges);
        Self::with_axes(build, horizontal, vertical)
    }
    fn with_axes(build: &super::ovg_ports::PortBuild, horizontal: Axes, vertical: Axes) -> Self {
        let mut points: Vec<Option<StaticPoint>> = Vec::new();
        // Each endpoint is captured at its first edge occurrence, in the
        // original edge order; a later occurrence reads the same node object.
        let mut endpoint = |id: usize| {
            let slot = u32::try_from(id).expect("OVG node slot");
            if id >= points.len() {
                points.resize(id + 1, None);
            }
            if points[id].is_none() {
                let node = &build.nodes[id].read();
                points[id] = Some(StaticPoint::of(node.point(), node.point_ref.clone()));
            }
            slot
        };
        let mut edges = Vec::with_capacity(build.edges.capacity());
        for edge in &build.edges {
            let edge = edge.read();
            let from = endpoint(edge.from_slot());
            let to = endpoint(edge.to_slot());
            edges.push(StaticEdge {
                from,
                to,
                distance: edge.distance,
            });
        }
        let ovg_points = u32::try_from(points.len()).expect("OVG node slot");
        Self {
            edges,
            points,
            ovg_points,
            horizontal,
            vertical,
        }
    }
    fn point(&self, endpoint: u32) -> Point {
        self.points[endpoint as usize]
            .as_ref()
            .expect("static edge endpoint")
            .point()
    }
    /// Edge `id`'s endpoints, read live in from/to order.
    pub(super) fn edge_points(&self, id: usize) -> [Point; 2] {
        let edge = &self.edges[id];
        [self.point(edge.from), self.point(edge.to)]
    }
    /// Edge's OVG node slots, None for a coordinate-only endpoint.
    #[cfg(any(test, feature = "diagnostic-traces"))]
    pub(super) fn edge_ovg(&self, edge: &StaticEdge) -> [Option<u32>; 2] {
        let ovg = |endpoint: u32| (endpoint < self.ovg_points).then_some(endpoint);
        [ovg(edge.from), ovg(edge.to)]
    }
}
/// One flavor's addRoute index: the shared static edges plus this router's
/// own nearby and overlapping publications. Mutable access to the static
/// part (fixtures and coordinate-only builds) copies it on write.
#[derive(Debug, Default)]
pub(super) struct StaticEdgeIndex {
    shared: std::sync::Arc<StaticEdges>,
    pub(super) nearby: EdgeFlags,
    pub(super) overlapping: EdgeLists,
}
impl std::ops::Deref for StaticEdgeIndex {
    type Target = StaticEdges;
    fn deref(&self) -> &StaticEdges {
        &self.shared
    }
}
impl std::ops::DerefMut for StaticEdgeIndex {
    fn deref_mut(&mut self) -> &mut StaticEdges {
        std::sync::Arc::make_mut(&mut self.shared)
    }
}
/// addRoute's `hasNearbyEdge` set over dense static edge slots. Lookups are
/// identity probes only; no caller depends on set iteration order.
#[derive(Debug, Default)]
pub(super) struct EdgeFlags {
    flags: Vec<bool>,
}
impl EdgeFlags {
    pub(super) fn insert(&mut self, id: usize) -> bool {
        if id >= self.flags.len() {
            self.flags.resize(id + 1, false);
        }
        !std::mem::replace(&mut self.flags[id], true)
    }
    pub(super) fn contains(&self, id: &usize) -> bool {
        self.flags.get(*id).copied().unwrap_or(false)
    }
    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.flags.iter().filter(|&&flag| flag).count()
    }
    #[cfg(test)]
    pub(super) fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Members in ascending edge-slot order.
    #[cfg(any(test, feature = "diagnostic-traces"))]
    pub(super) fn iter(&self) -> impl Iterator<Item = usize> + '_ {
        self.flags
            .iter()
            .enumerate()
            .filter_map(|(id, &flag)| flag.then_some(id))
    }
}
/// addRoute's `overlappingRoutes` lists over dense static edge slots. Each
/// list retains its own Go pointer-slice growth; slots map to list storage.
#[derive(Debug, Default)]
pub(super) struct EdgeLists {
    slots: Vec<u32>,
    lists: Vec<Vec<usize>>,
}
impl EdgeLists {
    pub(super) fn get(&self, id: &usize) -> Option<&Vec<usize>> {
        let slot = *self.slots.get(*id)?;
        (slot != 0).then(|| &self.lists[slot as usize - 1])
    }
    /// The present list, or a new empty (nil) list, as map `or_default`.
    pub(super) fn entry_or_default(&mut self, id: usize) -> &mut Vec<usize> {
        if id >= self.slots.len() {
            self.slots.resize(id + 1, 0);
        }
        if self.slots[id] == 0 {
            self.lists.push(Vec::new());
            self.slots[id] = u32::try_from(self.lists.len()).expect("static overlap slot");
        }
        &mut self.lists[self.slots[id] as usize - 1]
    }
    #[cfg(test)]
    pub(super) fn insert(&mut self, id: usize, routes: Vec<usize>) {
        *self.entry_or_default(id) = routes;
    }
    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.lists.len()
    }
    #[cfg(test)]
    pub(super) fn is_empty(&self) -> bool {
        self.lists.is_empty()
    }
    /// Entries in ascending edge-slot order.
    #[cfg(feature = "diagnostic-traces")]
    pub(super) fn iter(&self) -> impl Iterator<Item = (usize, &Vec<usize>)> + '_ {
        self.slots
            .iter()
            .enumerate()
            .filter(|(_, slot)| **slot != 0)
            .map(|(id, &slot)| (id, &self.lists[slot as usize - 1]))
    }
}
impl std::ops::Index<&usize> for EdgeLists {
    type Output = Vec<usize>;
    fn index(&self, id: &usize) -> &Vec<usize> {
        self.get(id).expect("static overlap entry")
    }
}
impl StaticEdgeIndex {
    pub(super) fn from_build(build: &super::ovg_ports::PortBuild) -> Self {
        Self::from_shared(std::sync::Arc::new(StaticEdges::from_build(build)))
    }
    /// A fresh index (no published routes) over existing static edges.
    pub(super) fn from_shared(shared: std::sync::Arc<StaticEdges>) -> Self {
        Self {
            shared,
            ..Self::default()
        }
    }
    /// Coordinate-only compatibility callers do not carry a concrete edge.
    /// Production canonical search supplies its actual OVG edge slot instead.
    pub(super) fn edge_at(&self, segment: [Point; 2]) -> Option<usize> {
        (0..self.edges.len()).position(|id| {
            let points = self.edge_points(id);
            points == segment || points == [segment[1], segment[0]]
        })
    }
    pub(super) fn from_segments(segments: impl IntoIterator<Item = [Point; 2]>) -> Self {
        let mut index = Self::default();
        for segment in segments {
            index.append_value(segment);
        }
        index
    }
    pub(super) fn append_value(&mut self, segment: [Point; 2]) -> usize {
        let shared = std::sync::Arc::make_mut(&mut self.shared);
        let id = shared.edges.len();
        let from = u32::try_from(shared.points.len()).expect("static edge endpoint");
        shared.points.push(Some(StaticPoint::Value(segment[0])));
        shared.points.push(Some(StaticPoint::Value(segment[1])));
        let edge = StaticEdge {
            from,
            to: from + 1,
            distance: super::euclidean_distance(segment[0], segment[1]),
        };
        push_pointer(&mut shared.edges, edge);
        if segment[0].x == segment[1].x {
            shared.vertical.append(segment[0].x, id);
        } else if segment[0].y == segment[1].y {
            shared.horizontal.append(segment[0].y, id);
        }
        id
    }
    pub(super) fn mark_route(
        &mut self,
        count: usize,
        route: usize,
        visible: bool,
        node: impl Fn(usize) -> NodeRef,
        work: Option<&dyn Work>,
    ) -> Result<(), ()> {
        if visible {
            for index in 1..count.saturating_sub(2) {
                work.map_or(Ok(()), Work::step)?;
                self.publish_segment(&node(index), &node(index + 1), route, work)?;
            }
        }
        Ok(())
    }
    fn publish_segment(
        &mut self,
        from: &NodeRef,
        to: &NodeRef,
        route: usize,
        work: Option<&dyn Work>,
    ) -> Result<(), ()> {
        let step = || work.map_or(Ok(()), Work::step);
        let vertical = from.point().x == to.point().x;
        let along = |point: Point| if vertical { point.y } else { point.x };
        let across = |point: Point| if vertical { point.x } else { point.y };
        let (mut low, mut high) = (along(from.point()), along(to.point()));
        if high < low {
            std::mem::swap(&mut low, &mut high);
        }
        let shared = &*self.shared;
        let axes = if vertical {
            &shared.vertical
        } else {
            &shared.horizontal
        };
        for bucket in axes.buckets.values() {
            step()?;
            if (bucket.axis - across(from.point())).abs() > 5.0 {
                continue;
            }
            for &id in &bucket.edges {
                step()?;
                let [edge_from, edge_to] = shared.edge_points(id);
                let (mut start, mut end) = (along(edge_from), along(edge_to));
                if end < start {
                    std::mem::swap(&mut start, &mut end);
                }
                if !(end < low || high < start) {
                    self.nearby.insert(id);
                    if bucket.axis == across(from.point()) {
                        push_pointer(self.overlapping.entry_or_default(id), route);
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(feature = "diagnostic-traces")]
#[path = "static_edge_index_probe.rs"]
mod probe;
#[cfg(feature = "diagnostic-traces")]
pub(in crate::internal) use probe::run as diagnostic_probe;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::internal::owner_points::SharedOwnerPoint;
    #[test]
    fn captured_axis_and_live_span_keep_distinct_overlap_lists() {
        let mut build = super::super::ovg_ports::PortBuild::default();
        let a = SharedOwnerPoint::new(Point { x: 0.0, y: 0.0 });
        let b = SharedOwnerPoint::new(Point { x: 10.0, y: 0.0 });
        for p in [a.get(), b.get(), a.get(), b.get()] {
            build.allocate_node(p);
        }
        build.nodes[0].write().point_ref = Some(a.clone());
        build.nodes[1].write().point_ref = Some(b.clone());
        build.connect(0, 1, None).unwrap();
        build.connect(2, 3, None).unwrap();
        a.set(Point { x: 0.0, y: 20.0 });
        b.set(Point { x: 10.0, y: 20.0 });
        let mut index = StaticEdgeIndex::from_build(&build);
        assert!(index.horizontal.buckets.contains_key(&0));
        assert!(!index.horizontal.buckets.contains_key(&20.0f64.to_bits()));
        let route = Edge::value([Point { x: 0.0, y: 0.0 }, Point { x: 10.0, y: 0.0 }]);
        index
            .mark_route(
                4,
                0,
                true,
                |i| {
                    if i == 1 {
                        route.from.clone()
                    } else {
                        route.to.clone()
                    }
                },
                None,
            )
            .unwrap();
        a.set(Point { x: 20.0, y: 20.0 });
        b.set(Point { x: 30.0, y: 20.0 });
        index
            .mark_route(
                4,
                1,
                true,
                |i| {
                    if i == 1 {
                        route.from.clone()
                    } else {
                        route.to.clone()
                    }
                },
                None,
            )
            .unwrap();
        assert_eq!(index.overlapping[&0], vec![0]);
        assert_eq!(index.overlapping[&1], vec![0, 1]);
    }
}
