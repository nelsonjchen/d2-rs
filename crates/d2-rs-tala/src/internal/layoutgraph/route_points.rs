// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// LocalRouteBuilder follows Go 1.27 runtime/slice.go growsliceBuf/moveSlice
// and cmd/compile/internal/slice/slice.go.
// Copyright 2025 The Go Authors. All rights reserved.
// The Go-derived allocation behavior is also covered by the BSD-3-Clause
// notice in crates/d2-rs-tala/THIRD_PARTY_NOTICES.md (Go standard library).
//! Edge.Points owns a slice header, not a detached array of coordinates.
use super::{Point, owner_pointer_slices::SharedPointSlice, owner_points::SharedOwnerPoint};

#[derive(Clone, Debug)]
pub(super) struct RouteField(std::sync::Arc<crate::internal::go_cell::GoCell<SharedPointSlice>>);
impl RouteField {
    pub(super) fn header(&self) -> SharedPointSlice {
        self.0.read().expect("Edge.Points field").clone()
    }
    pub(super) fn replace_header(&self, header: SharedPointSlice) {
        *self.0.write().expect("Edge.Points field") = header;
    }
}
#[derive(Debug)]
pub(super) struct RoutePoints(RouteField);
impl Clone for RoutePoints {
    fn clone(&self) -> Self {
        Self::from_header(self.header())
    }
}
impl RoutePoints {
    pub(super) fn from_header(header: SharedPointSlice) -> Self {
        Self(RouteField(std::sync::Arc::new(
            crate::internal::go_cell::GoCell::new(header),
        )))
    }
    pub(super) fn from_field(field: RouteField) -> Self {
        Self(field)
    }
    pub(super) fn header(&self) -> SharedPointSlice {
        self.0.header()
    }
    pub(super) fn retain_field(&self) -> RouteField {
        self.0.clone()
    }
    pub(super) fn replace_header(&self, header: SharedPointSlice) {
        self.0.replace_header(header);
    }
    pub(super) fn replace(&self, route: Self) {
        self.replace_header(route.header());
    }
}

/// A local, exclusively owned result assembled by simplifyPoints. The pinned
/// Go 1.27 compiler lowers that function's appends to growsliceBuf with a
/// four-pointer buffer, then moveSlice on return. While in that buffer growth
/// uses the next allocator size class (1, 2, 3, 4 pointers), not doubling.
/// Keep the buffer and header operations explicit; ordinary escaping route
/// appends continue to use growslice.
pub(super) struct LocalRouteBuilder {
    route: RoutePoints,
    buffer: SharedPointSlice,
    buffered: bool,
}
impl LocalRouteBuilder {
    pub(super) fn new(first: Option<SharedOwnerPoint>) -> Self {
        Self {
            route: RoutePoints::pointer_literal(&[first]),
            buffer: SharedPointSlice::with_len_cap(0, 4).expect("local route buffer"),
            buffered: false,
        }
    }
    pub(super) fn append_slot(&mut self, point: Option<SharedOwnerPoint>) {
        let len = self.route.len();
        if len == self.route.capacity() && len < 4 {
            if !self.buffered {
                for i in 0..len {
                    self.buffer
                        .write_capacity_slot(i, self.route.slot(i))
                        .expect("local route slot");
                }
            }
            self.route.replace_header(
                self.buffer
                    .reslice_with_capacity(0, len, len + 1)
                    .expect("local route growth"),
            );
            self.buffered = true;
        } else if len == self.route.capacity() {
            self.buffered = false;
        }
        self.route.append_slot(point);
    }
    pub(super) fn append_value(&mut self, point: Point) {
        self.append_slot(Some(SharedOwnerPoint::new(point)));
    }
    pub(super) fn finish(self) -> RoutePoints {
        if self.buffered {
            // moveSlice preserves capacity and every slot when a local buffer
            // escapes. Point allocations are retained, not copied.
            let moved = SharedPointSlice::with_len_cap(self.route.len(), self.route.capacity())
                .expect("escaped route");
            for i in 0..self.route.capacity() {
                moved
                    .write_capacity_slot(
                        i,
                        self.route
                            .header()
                            .read_capacity_slot(i)
                            .expect("local route slot"),
                    )
                    .expect("escaped route slot");
            }
            RoutePoints::from_header(moved)
        } else {
            self.route
        }
    }
}

/// A read-only coordinate image for numerical helpers. Deliberately has no
/// DerefMut: writing a temporary image must never silently discard a mutation.
#[derive(Debug, serde::Serialize)]
#[serde(transparent)]
pub(super) struct RouteValues(Vec<Point>);
impl std::ops::Deref for RouteValues {
    type Target = [Point];
    fn deref(&self) -> &[Point] {
        &self.0
    }
}
impl RouteValues {
    pub(super) fn clone(&self) -> Vec<Point> {
        self.0.clone()
    }
}
impl PartialEq<Vec<Point>> for RouteValues {
    fn eq(&self, other: &Vec<Point>) -> bool {
        &self.0 == other
    }
}
impl PartialEq<RouteValues> for Vec<Point> {
    fn eq(&self, other: &RouteValues) -> bool {
        self == &other.0
    }
}
impl PartialEq for RouteValues {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}
impl RoutePoints {
    pub(super) fn read(&self) -> RouteValues {
        // Edge.Points is read without a callback or a retained range here.
        // Borrow the field and pointer slots; only coordinates leave the read.
        let header = self.0.0.read().expect("Edge.Points field");
        let mut values = Vec::with_capacity(header.len());
        header.for_each_visible(|point| {
            values.push(point.as_ref().expect("nil owner pointer slot").get());
        });
        RouteValues(values)
    }
    /// Source slice literal: a fresh point for each supplied coordinate.
    pub(super) fn literal(points: &[Point]) -> Self {
        Self::from_header(
            SharedPointSlice::from_values(
                &points
                    .iter()
                    .copied()
                    .map(SharedOwnerPoint::new)
                    .collect::<Vec<_>>(),
            )
            .expect("route literal"),
        )
    }
    /// graphjson.deserializeEdge starts at NewEdge's allocated-empty slice
    /// and appends each decoded nullable pointer separately.
    pub(super) fn decoded(points: impl IntoIterator<Item = Option<Point>>) -> Self {
        let mut slice = SharedPointSlice::allocated_empty();
        for point in points {
            slice = slice
                .append(&[point.map(SharedOwnerPoint::new)])
                .expect("decoded route append");
        }
        Self::from_header(slice)
    }
    pub(super) fn edit_point(&self, index: usize, edit: impl FnOnce(&mut Point)) {
        let point = self
            .header()
            .get(index)
            .expect("route index")
            .expect("nil route point");
        let mut value = point.get();
        edit(&mut value);
        point.set(value);
    }
    /// Assignment to the pointer slot allocates a new geo.Point. In-place
    /// coordinate changes must instead use edit_point to retain aliases.
    pub(super) fn replace_point(&self, index: usize, point: Point) {
        assert!(index < self.header().len());
        self.header()
            .write_capacity_slot(index, Some(SharedOwnerPoint::new(point)))
            .expect("route index");
    }
}

impl<'a> IntoIterator for &'a RouteValues {
    type Item = &'a Point;
    type IntoIter = std::slice::Iter<'a, Point>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

pub(super) struct RoutePointEdit {
    point: SharedOwnerPoint,
    value: Point,
}
impl std::ops::Deref for RoutePointEdit {
    type Target = Point;
    fn deref(&self) -> &Point {
        &self.value
    }
}
impl std::ops::DerefMut for RoutePointEdit {
    fn deref_mut(&mut self) -> &mut Point {
        &mut self.value
    }
}
impl Drop for RoutePointEdit {
    fn drop(&mut self) {
        self.point.set(self.value);
    }
}
impl RoutePoints {
    pub(super) fn nil() -> Self {
        Self::from_header(SharedPointSlice::nil())
    }
    pub(super) fn len(&self) -> usize {
        self.0.0.read().expect("Edge.Points field").len()
    }
    pub(super) fn is_empty(&self) -> bool {
        self.0.0.read().expect("Edge.Points field").is_empty()
    }
    pub(super) fn capacity(&self) -> usize {
        self.0.0.read().expect("Edge.Points field").capacity()
    }
    pub(super) fn point(&self, index: usize) -> Point {
        self.coordinate_slot(index)
            .expect("route index")
            .unwrap_or_else(|| {
                std::panic::panic_any(
                    "runtime error: invalid memory address or nil pointer dereference",
                )
            })
    }
    /// Read a nullable point without retaining its slice backing or point
    /// allocation. No callback can replace the header or slot during this read.
    pub(super) fn coordinate_slot(&self, index: usize) -> Option<Option<Point>> {
        self.0
            .0
            .read()
            .expect("Edge.Points field")
            .coordinate_slot(index)
    }
    pub(super) fn slot(&self, index: usize) -> Option<SharedOwnerPoint> {
        self.header().get(index).expect("route index")
    }
    pub(super) fn pointer_literal(points: &[Option<SharedOwnerPoint>]) -> Self {
        Self::from_header(SharedPointSlice::from_slots(points).expect("route literal"))
    }
    pub(super) fn append_slot(&mut self, point: Option<SharedOwnerPoint>) {
        self.replace_header(self.header().append(&[point]).expect("route append"));
    }
    pub(super) fn append_value(&mut self, point: Point) {
        self.append_slot(Some(SharedOwnerPoint::new(point)));
    }
    pub(super) fn point_mut(&self, index: usize) -> RoutePointEdit {
        let point = self
            .header()
            .get(index)
            .expect("route index")
            .expect("nil route point");
        RoutePointEdit {
            value: point.get(),
            point,
        }
    }
    pub(super) fn iter_mut(&self) -> impl Iterator<Item = RoutePointEdit> + use<> {
        self.header().require_values().map(|point| RoutePointEdit {
            value: point.get(),
            point,
        })
    }
    /// Source append(points[:start], points[end:]...). Retains tail slots.
    pub(super) fn remove_in_place(&mut self, range: std::ops::Range<usize>) {
        let tail = self
            .header()
            .reslice_with_capacity(range.end, self.len(), self.capacity())
            .expect("route tail");
        let prefix = self
            .header()
            .reslice_with_capacity(0, range.start, self.capacity())
            .expect("route prefix");
        self.replace_header(
            prefix
                .append(&tail.iter_slots().collect::<Vec<_>>())
                .expect("route remove"),
        );
    }
    /// removeEdgePointsGuarded constructs an exact fresh pointer slice.
    pub(super) fn remove_fresh(&mut self, range: std::ops::Range<usize>) {
        let slots = self
            .header()
            .iter_slots()
            .enumerate()
            .filter_map(|(i, p)| (!range.contains(&i)).then_some(p))
            .collect::<Vec<_>>();
        self.replace_header(SharedPointSlice::from_slots(&slots).expect("route removal"));
    }
    /// insertEdgePointGuarded constructs a new exact len+1 backing.
    pub(super) fn insert_fresh(&mut self, index: usize, point: Point) {
        let mut slots = self.header().iter_slots().collect::<Vec<_>>();
        slots.insert(index, Some(SharedOwnerPoint::new(point)));
        self.replace_header(SharedPointSlice::from_slots(&slots).expect("route insertion"));
    }
    pub(super) fn swap_slots(&self, first: usize, other: &Self, second: usize) {
        let a = self.header().get(first).expect("route index");
        let b = other.header().get(second).expect("route index");
        self.header()
            .write_capacity_slot(first, b)
            .expect("route index");
        other
            .header()
            .write_capacity_slot(second, a)
            .expect("route index");
    }
}

impl RoutePoints {
    pub(super) fn segments(&self) -> impl Iterator<Item = [Point; 2]> + use<> {
        let header = self.clone();
        (1..self.len()).map(move |i| [header.point(i - 1), header.point(i)])
    }
}

#[derive(Clone, Debug)]
pub(super) struct RouteSnapshot {
    backing: super::owner_pointer_slices::SharedPointSliceSnapshot,
    points: Vec<(SharedOwnerPoint, Point)>,
}
impl RoutePoints {
    pub(super) fn snapshot(&self) -> RouteSnapshot {
        RouteSnapshot {
            backing: self.header().snapshot(),
            points: (0..self.capacity())
                .filter_map(|i| self.header().read_capacity_slot(i).flatten())
                .map(|p| {
                    let v = p.get();
                    (p, v)
                })
                .collect(),
        }
    }
    #[cfg(any(test, feature = "diagnostic-traces"))]
    pub(super) fn backing_identity(&self) -> Option<usize> {
        self.header().data_address()
    }
}
impl RouteSnapshot {
    pub(super) fn restore(&self) -> RoutePoints {
        for (point, value) in &self.points {
            point.set(*value);
        }
        RoutePoints::from_header(self.backing.restore())
    }
}

impl<const N: usize> PartialEq<[Point; N]> for RouteValues {
    fn eq(&self, other: &[Point; N]) -> bool {
        self.0.as_slice() == other
    }
}
impl RoutePoints {
    pub(super) fn reverse(&self) {
        for i in 0..self.len() / 2 {
            self.swap_slots(i, self, self.len() - 1 - i);
        }
    }
}

impl RoutePoints {
    pub(super) fn make(points: &[Point], capacity: usize) -> Self {
        let slice = SharedPointSlice::with_len_cap(points.len(), capacity).expect("route make");
        for (i, &point) in points.iter().enumerate() {
            slice
                .write_capacity_slot(i, Some(SharedOwnerPoint::new(point)))
                .expect("route index");
        }
        Self::from_header(slice)
    }
    /// balance_route_guard.go::removeDuplicatePointsGuarded retains pointers,
    /// allocates exactly the input length, and appends one new point only for
    /// the degenerate all-equal route.
    pub(super) fn deduplicate(&mut self) {
        if self.len() <= 1 {
            return;
        }
        let mut result = SharedPointSlice::with_len_cap(0, self.len()).expect("deduplicated route");
        result = result
            .append(&[self.header().get(0).unwrap()])
            .expect("route append");
        for i in 1..self.len() {
            if self.point(i - 1) != self.point(i) {
                result = result
                    .append(&[self.header().get(i).unwrap()])
                    .expect("route append");
            }
        }
        if result.len() == 1 {
            let mut second = self.point(0);
            second.x += 1.0;
            result = result
                .append(&[Some(SharedOwnerPoint::new(second))])
                .expect("route append");
        }
        self.replace_header(result);
    }
}

// Restoring an existing Edge copies its value into the same source object.
// Keep its actual Points field alive while replacing the saved header.
impl super::ArenaEdge {
    pub(super) fn restore_value(&mut self, mut saved: Self) {
        let field = self.route.retain_field();
        field.replace_header(saved.route.header());
        saved.route = RoutePoints::from_field(field);
        *self = saved;
    }

    /// `restore_value(saved.clone())` for a caller that replaces this edge's
    /// route header immediately afterwards: the value fields are assigned in
    /// place and the Points field is left for that replacement.
    pub(super) fn restore_value_except_route(&mut self, saved: &Self) {
        let Self {
            input_id,
            tala_id,
            invisible,
            label_position_fixed,
            source_label_positions,
            from,
            to,
            route: _,
            is_curve,
            source_arrow,
            target_arrow,
            source_arrowhead,
            target_arrowhead,
            label,
            source_arrowhead_label,
            target_arrowhead_label,
            style,
            source_table_column,
            target_table_column,
            source_table_column_count,
            target_table_column_count,
            min_width,
            min_height,
        } = self;
        *input_id = saved.input_id;
        *tala_id = saved.tala_id;
        *invisible = saved.invisible;
        *label_position_fixed = saved.label_position_fixed;
        *source_label_positions = saved.source_label_positions;
        *from = saved.from;
        *to = saved.to;
        *is_curve = saved.is_curve;
        *source_arrow = saved.source_arrow;
        *target_arrow = saved.target_arrow;
        source_arrowhead.clone_from(&saved.source_arrowhead);
        target_arrowhead.clone_from(&saved.target_arrowhead);
        label.clone_from(&saved.label);
        source_arrowhead_label.clone_from(&saved.source_arrowhead_label);
        target_arrowhead_label.clone_from(&saved.target_arrowhead_label);
        style.clone_from(&saved.style);
        *source_table_column = saved.source_table_column;
        *target_table_column = saved.target_table_column;
        *source_table_column_count = saved.source_table_column_count;
        *target_table_column_count = saved.target_table_column_count;
        *min_width = saved.min_width;
        *min_height = saved.min_height;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mutation_and_removal_preserve_live_pointer_and_tail_aliases() {
        let mut route = RoutePoints::decoded((0..4).map(|i| {
            Some(Point {
                x: i as f64,
                y: 10.0,
            })
        }));
        let retained = route.clone();
        let old_second = route.header().get(1).unwrap().unwrap();
        route.point_mut(1).x = 99.0;
        assert_eq!(old_second.get().x, 99.0);
        route.remove_in_place(1..3);
        assert!(route.header().same_backing_allocation(&retained.header()));
        assert_eq!((route.len(), route.capacity()), (2, 4));
        assert_eq!(retained.point(1).x, 3.0);
        assert_eq!(retained.point(2).x, 2.0);
        assert_eq!(retained.point(3).x, 3.0);
        assert_eq!(old_second.get().x, 99.0);
    }
    #[test]
    fn deduplication_retains_point_pointers_and_exact_input_capacity() {
        let mut route = RoutePoints::literal(&[
            Point { x: 2.0, y: 3.0 },
            Point { x: 2.0, y: 3.0 },
            Point { x: 8.0, y: 9.0 },
            Point { x: 8.0, y: 9.0 },
        ]);
        let first = route.header().get(0).unwrap().unwrap();
        let third = route.header().get(2).unwrap().unwrap();
        let original = route.clone();
        route.deduplicate();
        assert_eq!((route.len(), route.capacity()), (2, 4));
        assert!(!route.header().same_backing_allocation(&original.header()));
        assert!(
            route
                .header()
                .get(0)
                .unwrap()
                .unwrap()
                .same_allocation(&first)
        );
        assert!(
            route
                .header()
                .get(1)
                .unwrap()
                .unwrap()
                .same_allocation(&third)
        );
        assert!(route.header().read_capacity_slot(2).unwrap().is_none());
        let snapshot = route.snapshot();
        route.replace_point(0, Point::default());
        third.set(Point::default());
        route = snapshot.restore();
        assert!(
            route
                .header()
                .get(0)
                .unwrap()
                .unwrap()
                .same_allocation(&first)
        );
        assert_eq!(third.get(), Point { x: 8.0, y: 9.0 });
    }
}
