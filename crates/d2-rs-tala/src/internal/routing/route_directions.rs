// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Upstream 64288a936 route_directions.go. Preserve segment order per line.
use super::{LocalRouteSearchWork, Point};
use std::collections::BTreeMap;
#[derive(Default)]
pub(super) struct DirectionIndex(BTreeMap<(u64, bool), Vec<(f64, f64)>>);
fn step(work: Option<&LocalRouteSearchWork>) -> Option<()> {
    if let Some(work) = work {
        work.step().ok()?;
    }
    Some(())
}
impl DirectionIndex {
    pub(super) fn new(points: &[Point], work: Option<&LocalRouteSearchWork>) -> Option<Self> {
        let mut index = Self::default();
        for i in 0..points.len().saturating_sub(2) {
            step(work)?;
            let (a, b) = (points[i], points[i + 1]);
            if a.y == b.y && a.x != b.x {
                index
                    .0
                    .entry((super::go_float_map_key(a.y), false))
                    .or_default()
                    .push((a.x, b.x));
            } else if a.x == b.x && a.y != b.y {
                index
                    .0
                    .entry((super::go_float_map_key(a.x), true))
                    .or_default()
                    .push((a.y, b.y));
            }
        }
        Some(index)
    }
    pub(super) fn opposes(
        &self,
        from: Point,
        to: Point,
        work: Option<&LocalRouteSearchWork>,
    ) -> Option<bool> {
        let (axis, a, b) = if from.y == to.y {
            ((super::go_float_map_key(from.y), false), from.x, to.x)
        } else if from.x == to.x {
            ((super::go_float_map_key(from.x), true), from.y, to.y)
        } else {
            return Some(false);
        };
        for &(start, end) in self.0.get(&axis).into_iter().flatten() {
            step(work)?;
            let (stop, opposing) = opposes(start, end, a, b);
            if stop {
                return Some(opposing);
            }
        }
        Some(false)
    }
}
fn opposes(start: f64, end: f64, from: f64, to: f64) -> (bool, bool) {
    let (lo, hi) = (start.min(end), start.max(end));
    if from.max(to) < lo || hi < from.min(to) {
        return (false, false);
    }
    if ((from == lo || from == hi) && (to < lo || to > hi))
        || ((to == lo || to == hi) && (from < lo || from > hi))
    {
        return (true, false);
    }
    let opposing = (end > start && to < from) || (end < start && to > from);
    (opposing, opposing)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn upstream_route_opposing_collinear() {
        let points = [
            (0., 0.),
            (100., 0.),
            (100., 100.),
            (200., 100.),
            (210., 100.),
        ]
        .map(|(x, y)| Point { x, y });
        let index = DirectionIndex::new(&points, None).unwrap();
        for (a, b, expected) in [
            ((0., 0.), (100., 0.), false),
            ((100., 0.), (0., 0.), true),
            ((100., 0.), (100., 100.), false),
            ((100., 100.), (100., 0.), true),
            ((190., 0.), (100., 0.), false),
            ((100., 0.), (190., 0.), false),
            ((90., 0.), (120., 0.), false),
            ((120., 0.), (90., 0.), true),
            ((101., 0.), (200., 0.), false),
            ((0., 0.), (100., 100.), false),
        ] {
            assert_eq!(
                index.opposes(Point { x: a.0, y: a.1 }, Point { x: b.0, y: b.1 }, None),
                Some(expected)
            );
        }
    }
}
