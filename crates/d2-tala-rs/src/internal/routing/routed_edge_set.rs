// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Pinned routing/ovg_edge_set.go: captured axes, retained node references.
use super::{LocalRouteSearchWork, Point};
use crate::internal::owner_points::SharedOwnerPoint;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum NodeIdentity {
    Ovg(usize),
    Synthetic(usize, usize),
    // Coordinate fixtures have no OVG object inventory. Production records
    // always use Ovg or route-local Synthetic identities.
    #[cfg(test)]
    Fixture(u64, u64),
}
#[derive(Clone, Debug)]
pub(super) struct NodeRef {
    pub(super) identity: NodeIdentity,
    pub(super) value: Point,
    pub(super) shared: Option<SharedOwnerPoint>,
}
impl NodeRef {
    pub(super) fn point(&self) -> Point {
        self.shared
            .as_ref()
            .map_or(self.value, SharedOwnerPoint::get)
    }
}
#[derive(Clone, Debug)]
pub(super) struct Edge {
    pub(super) from: NodeRef,
    pub(super) to: NodeRef,
    pub(super) distance: f64,
}
impl Edge {
    pub(super) fn points(&self) -> [Point; 2] {
        [self.from.point(), self.to.point()]
    }
    fn vertical(&self) -> bool {
        self.from.point().x == self.to.point().x
    }
    fn horizontal(&self) -> bool {
        self.from.point().y == self.to.point().y
    }
    fn key(&self) -> Option<(NodeIdentity, NodeIdentity, u64)> {
        (!self.distance.is_nan()).then(|| {
            (
                self.from.identity,
                self.to.identity,
                super::go_float_map_key(self.distance),
            )
        })
    }
    fn shares_points(&self, other: &Self) -> bool {
        self.from.point() == other.from.point()
            || self.from.point() == other.to.point()
            || self.to.point() == other.from.point()
            || self.to.point() == other.to.point()
    }
    pub(super) fn value(points: [Point; 2]) -> Self {
        Self {
            from: NodeRef {
                identity: NodeIdentity::Synthetic(usize::MAX, 0),
                value: points[0],
                shared: None,
            },
            to: NodeRef {
                identity: NodeIdentity::Synthetic(usize::MAX, 1),
                value: points[1],
                shared: None,
            },
            distance: 0.0,
        }
    }
    #[cfg(test)]
    pub(super) fn fixture(points: [Point; 2]) -> Self {
        let mut edge = Self::value(points);
        for node in [&mut edge.from, &mut edge.to] {
            let key = super::go_point_map_key(node.value);
            node.identity = NodeIdentity::Fixture(key.0, key.1);
        }
        edge
    }
}

pub(super) trait Work {
    fn step(&self) -> Result<(), ()>;
    fn add(&self, units: u64) -> Result<(), ()>;
    fn check(&self) -> Result<(), ()>;
    fn skipped(&self, amount: usize) -> Result<(), ()> {
        for _ in 0..amount {
            self.step()?;
        }
        Ok(())
    }
}
impl Work for LocalRouteSearchWork<'_> {
    fn step(&self) -> Result<(), ()> {
        self.step()
    }
    fn add(&self, units: u64) -> Result<(), ()> {
        self.add(units)
    }
    fn check(&self) -> Result<(), ()> {
        self.check()
    }
    fn skipped(&self, amount: usize) -> Result<(), ()> {
        self.charge_skipped(amount)
    }
}
type Budget<'a> = Option<&'a dyn Work>;
fn step(budget: Budget<'_>) -> Result<(), ()> {
    budget.map_or(Ok(()), Work::step)
}
fn add(budget: Budget<'_>, units: usize) -> Result<(), ()> {
    budget.map_or(Ok(()), |b| b.add(units as u64))
}
fn axis_key(value: f64) -> Option<u64> {
    (!value.is_nan()).then(|| super::go_float_map_key(value))
}
fn bucket(map: &AxisBuckets, axis: f64) -> &[Edge] {
    axis_key(axis)
        .and_then(|key| map.get(&key))
        .map_or(&[], |(_, edges)| edges.as_slice())
}
/// Axis-keyed edge buckets. Lookup-only: ordered traversal uses the sorted
/// `*_axes` vectors, so the map order never reaches output.
pub(super) type AxisBuckets = crate::internal::FloatKeyHashMap<u64, (f64, Vec<Edge>)>;

fn search(len: usize, mut predicate: impl FnMut(usize) -> bool) -> usize {
    let (mut low, mut high) = (0, len);
    while low < high {
        let middle = low + (high - low) / 2;
        if predicate(middle) {
            high = middle;
        } else {
            low = middle + 1;
        }
    }
    low
}

// Use source element widths, independent of the larger Rust retained references.
fn grow<T>(values: &mut Vec<T>, source_bytes: usize, scanned: bool) {
    if values.len() == values.capacity() {
        let capacity = crate::internal::container_slices::go127_slice_growth_capacity(
            values.capacity(),
            values.len() + 1,
            source_bytes,
            scanned,
        )
        .expect("edge-set slice capacity");
        values.reserve_exact(capacity - values.len());
    }
}

#[derive(Debug)]
pub(super) struct EdgeSet {
    pub(super) horizontal: AxisBuckets,
    pub(super) vertical: AxisBuckets,
    pub(super) horizontal_axes: Vec<f64>,
    pub(super) vertical_axes: Vec<f64>,
    keys: BTreeMap<(NodeIdentity, NodeIdentity, u64), Edge>,
    // Go NaN Distance entries are always distinct and never findable.
    nan_edges: Vec<Edge>,
}
impl Default for EdgeSet {
    fn default() -> Self {
        Self {
            horizontal: AxisBuckets::default(),
            vertical: AxisBuckets::default(),
            horizontal_axes: Vec::with_capacity(16),
            vertical_axes: Vec::with_capacity(16),
            keys: BTreeMap::new(),
            nan_edges: Vec::new(),
        }
    }
}
impl EdgeSet {
    pub(super) fn len(&self) -> usize {
        self.keys.len() + self.nan_edges.len()
    }
    pub(super) fn contains(&self, edge: &Edge) -> bool {
        edge.key().is_some_and(|key| self.keys.contains_key(&key))
    }
    fn insert_axis(axes: &mut Vec<f64>, value: f64) {
        // sort.SearchFloat64s: NaN sorts before all other values.
        let index = search(axes.len(), |i| axes[i] >= value || axes[i].is_nan());
        if index < axes.len() && axes[index] == value {
            return;
        }
        grow(axes, 8, false);
        axes.insert(index, value);
    }
    fn insert_edge(edges: &mut Vec<Edge>, edge: Edge, vertical: bool) {
        let minimum = |edge: &Edge| {
            let [a, b] = edge.points();
            let (a, b) = if vertical { (a.y, b.y) } else { (a.x, b.x) };
            if a < b { a } else { b }
        };
        let value = minimum(&edge);
        let index = search(edges.len(), |i| minimum(&edges[i]) >= value);
        grow(edges, 24, true);
        edges.insert(index, edge);
    }
    pub(super) fn add(&mut self, edge: Edge) {
        if self.contains(&edge) {
            return;
        }
        if let Some(key) = edge.key() {
            self.keys.insert(key, edge.clone());
        } else {
            self.nan_edges.push(edge.clone());
        }
        let vertical = edge.vertical();
        if vertical || edge.horizontal() {
            let axis = if vertical {
                edge.from.point().x
            } else {
                edge.from.point().y
            };
            // The chosen axis compares equal at both ends and cannot be NaN.
            let key = axis_key(axis).expect("axis selected by equal coordinates");
            let (map, axes) = if vertical {
                (&mut self.vertical, &mut self.vertical_axes)
            } else {
                (&mut self.horizontal, &mut self.horizontal_axes)
            };
            let (stored_axis, edges) = map.entry(key).or_insert_with(|| {
                Self::insert_axis(axes, axis);
                (axis, Vec::with_capacity(16))
            });
            // Go float map assignment refreshes the stored sign of zero.
            *stored_axis = axis;
            Self::insert_edge(edges, edge, vertical);
        }
    }
    pub(super) fn add_guarded(&mut self, edge: Edge, budget: Budget<'_>) -> Result<(), ()> {
        step(budget)?;
        if self.contains(&edge) {
            return Ok(());
        }
        if edge.vertical() {
            if axis_key(edge.from.point().x).is_none_or(|key| !self.vertical.contains_key(&key)) {
                add(budget, self.vertical_axes.len() + 1)?;
            }
            add(
                budget,
                bucket(&self.vertical, edge.from.point().x).len() + 1,
            )?;
        } else if edge.horizontal() {
            if axis_key(edge.from.point().y).is_none_or(|key| !self.horizontal.contains_key(&key)) {
                add(budget, self.horizontal_axes.len() + 1)?;
            }
            add(
                budget,
                bucket(&self.horizontal, edge.from.point().y).len() + 1,
            )?;
        }
        // add reevaluates orientation and axis after all callback boundaries.
        self.add(edge);
        budget.map_or(Ok(()), Work::check)
    }
    pub(super) fn overlapping(&self, edge: &Edge, budget: Budget<'_>) -> Result<Vec<Edge>, ()> {
        let candidates = if edge.vertical() {
            bucket(&self.vertical, edge.from.point().x)
        } else if edge.horizontal() {
            bucket(&self.horizontal, edge.from.point().y)
        } else {
            &[]
        };
        let mut result = Vec::new();
        for candidate in candidates {
            step(budget)?;
            let [a, b] = edge.points();
            let [c, d] = candidate.points();
            if intersects(a, b, c, d) {
                grow(&mut result, 24, true);
                result.push(candidate.clone());
            }
        }
        Ok(result)
    }
    pub(super) fn intersects(&self, edge: &Edge, budget: Budget<'_>) -> Result<bool, ()> {
        self.intersects_with(edge, budget)
    }
    /// `intersects` over a concrete guard type, so search's per-candidate
    /// steps are direct calls rather than trait-object dispatch.
    pub(super) fn intersects_with<W: Work + ?Sized>(
        &self,
        edge: &Edge,
        budget: Option<&W>,
    ) -> Result<bool, ()> {
        let step = || budget.map_or(Ok(()), Work::step);
        // Intersections test horizontal first, including coincident endpoints.
        let horizontal = edge.horizontal();
        if !horizontal && !edge.vertical() {
            return Ok(false);
        }
        let coordinate = |p: Point| if horizontal { p.x } else { p.y };
        let axis = |p: Point| if horizontal { p.y } else { p.x };
        let (mut low, mut high) = (coordinate(edge.from.point()), coordinate(edge.to.point()));
        if high < low {
            std::mem::swap(&mut low, &mut high);
        }
        let (parallel, perpendicular, axes) = if horizontal {
            (&self.horizontal, &self.vertical, &self.vertical_axes)
        } else {
            (&self.vertical, &self.horizontal, &self.horizontal_axes)
        };
        for candidate in bucket(parallel, axis(edge.from.point())) {
            step()?;
            let (a, b) = (
                coordinate(candidate.from.point()),
                coordinate(candidate.to.point()),
            );
            if a < b {
                if high <= a {
                    break;
                }
                if b <= low {
                    continue;
                }
            } else {
                if high <= b {
                    break;
                }
                if a <= low {
                    continue;
                }
            }
            if !edge.shares_points(candidate) {
                return Ok(true);
            }
        }
        let edge_axis = axis(edge.from.point());
        let start = search(axes.len(), |i| !(axes[i] < low));
        if let Some(budget) = budget {
            budget.skipped(start)?;
        }
        for &line in &axes[start..] {
            step()?;
            if line < low {
                continue;
            }
            if high < line {
                break;
            }
            for candidate in bucket(perpendicular, line) {
                step()?;
                let (a, b) = (axis(candidate.from.point()), axis(candidate.to.point()));
                if a < b {
                    if edge_axis < a {
                        break;
                    }
                    if b < edge_axis {
                        continue;
                    }
                } else {
                    if edge_axis < b {
                        break;
                    }
                    if a < edge_axis {
                        continue;
                    }
                }
                if !edge.shares_points(candidate) {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }
}

// geometry.go's exact orientation/closed-interval predicate. No epsilon,
// bounding-box shortcut or alternate algebraic arrangement is equivalent here.
fn intersects(a: Point, b: Point, c: Point, d: Point) -> bool {
    let orientation =
        |a: Point, b: Point, c: Point| (b.y - a.y) * (c.x - a.x) - (b.x - a.x) * (c.y - a.y);
    let [s1, s2, s3, s4] = [
        orientation(a, b, c),
        orientation(a, b, d),
        orientation(c, d, a),
        orientation(c, d, b),
    ];
    if s1 == 0.0 && s2 == 0.0 && s3 == 0.0 && s4 == 0.0 {
        let overlap = |a: f64, b: f64, c: f64, d: f64| {
            ![a, b, c, d].iter().any(|v| v.is_nan()) && a.min(b) <= c.max(d) && c.min(d) <= a.max(b)
        };
        return overlap(a.x, b.x, c.x, d.x) && overlap(a.y, b.y, c.y, d.y);
    }
    let straddles = |a: f64, b: f64| a == 0.0 || b == 0.0 || (a < 0.0) != (b < 0.0);
    straddles(s1, s2) && straddles(s3, s4)
}

#[cfg(feature = "diagnostic-traces")]
#[path = "routed_edge_set_probe.rs"]
mod probe;
#[cfg(feature = "diagnostic-traces")]
pub(in crate::internal) use probe::run as diagnostic_probe;

#[cfg(test)]
mod tests {
    use super::*;
    fn node(id: usize, x: f64, y: f64) -> NodeRef {
        let value = Point { x, y };
        NodeRef {
            identity: NodeIdentity::Ovg(id),
            value,
            shared: Some(SharedOwnerPoint::new(value)),
        }
    }
    #[test]
    fn identity_keys_keep_coincident_nodes_distance_and_direction() {
        let a = Edge {
            from: node(0, 0.0, 0.0),
            to: node(1, 10.0, 0.0),
            distance: 0.0,
        };
        let b = Edge {
            from: node(2, 0.0, 0.0),
            to: node(3, 10.0, 0.0),
            distance: 0.0,
        };
        let mut set = EdgeSet::default();
        set.add(a.clone());
        set.add(b.clone());
        set.add(Edge {
            distance: -0.0,
            ..a.clone()
        });
        assert_eq!(set.len(), 2);
        set.add(Edge {
            from: a.to.clone(),
            to: a.from.clone(),
            distance: 0.0,
        });
        set.add(Edge {
            distance: 1.0,
            ..a.clone()
        });
        for _ in 0..2 {
            set.add(Edge {
                distance: f64::NAN,
                ..a.clone()
            });
        }
        assert_eq!(set.len(), 6);
        assert!(!set.contains(&Edge {
            distance: f64::NAN,
            ..a.clone()
        }));
        let overlap = set.overlapping(&a, None).unwrap();
        assert_eq!(overlap.len(), 6);
        assert_eq!(overlap[4].from.identity, b.from.identity); // ties insert before existing entries
    }
    #[test]
    fn captured_axis_bucket_reads_live_points_and_source_growth() {
        let edge = Edge {
            from: node(0, 5.0, -10.0),
            to: node(1, 5.0, 10.0),
            distance: 0.0,
        };
        let query = Edge::value([Point { x: 0.0, y: 0.0 }, Point { x: 10.0, y: 0.0 }]);
        let mut set = EdgeSet::default();
        set.add(edge.clone());
        assert_eq!(set.intersects(&query, None), Ok(true));
        edge.from
            .shared
            .as_ref()
            .unwrap()
            .set(Point { x: 20.0, y: 20.0 });
        edge.to
            .shared
            .as_ref()
            .unwrap()
            .set(Point { x: 20.0, y: 40.0 });
        assert_eq!(set.vertical_axes, vec![5.0]);
        assert_eq!(set.intersects(&query, None), Ok(false));
        for id in 1..17 {
            set.add(Edge {
                from: node(2 * id, 5.0, -10.0),
                to: node(2 * id + 1, 5.0, 10.0),
                distance: 0.0,
            });
        }
        assert_eq!(set.vertical[&5.0_f64.to_bits()].1.capacity(), 37);
    }
}
