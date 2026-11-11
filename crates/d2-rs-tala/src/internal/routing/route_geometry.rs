// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Route clearance, containment, and segment geometry checks.
//!
//! Candidate polylines are tested against non-endpoint boxes and the container
//! scopes assigned to their interior points.

use super::*;

/// The pinned v0.9.0 box predicate used by `Node.PassesThrough` and
/// `segmentIntersectsBox`. After broad-phase rejection and endpoint
/// containment, it clips against the closed box and requires a positive
/// interval. A corner touch with both endpoints outside is admissible.
#[inline(always)]
pub(super) fn recovered_segment_intersects_box(start: Point, end: Point, rect: Rect) -> bool {
    // This is TALA's layoutgraph.segmentIntersectsBox verbatim in geometric
    // terms. A single-point boundary touch is admissible; only a positive
    // length of the segment inside the closed box is an intersection.
    let (mut left, mut right) = (rect.origin.x, rect.right());
    if left > right {
        std::mem::swap(&mut left, &mut right);
    }
    let (mut top, mut bottom) = (rect.origin.y, rect.bottom());
    if top > bottom {
        std::mem::swap(&mut top, &mut bottom);
    }
    if [left, right, top, bottom]
        .iter()
        .any(|value| value.is_nan())
    {
        return false;
    }
    if (start.x < left && end.x < left)
        || (start.x > right && end.x > right)
        || (start.y < top && end.y < top)
        || (start.y > bottom && end.y > bottom)
    {
        return false;
    }
    let contains =
        |point: Point| left <= point.x && point.x <= right && top <= point.y && point.y <= bottom;
    if contains(start) || contains(end) {
        return true;
    }
    clip_segment(start, end, left, right, top, bottom)
}

// The broad phase above usually decides an OVG blocker query. Keep clipping's
// divisions out of its callers, without packing a temporary Box on the stack
// for every rejected blocker. This is the remaining tail of the same pinned
// layoutgraph.segmentIntersectsBox, with the same axis and arithmetic order.
#[inline(never)]
fn clip_segment(start: Point, end: Point, left: f64, right: f64, top: f64, bottom: f64) -> bool {
    let mut enter: f64 = 0.0;
    let mut exit: f64 = 1.0;
    let mut clip_axis = |origin: f64, delta: f64, minimum: f64, maximum: f64| {
        if delta == 0.0 {
            return minimum <= origin && origin <= maximum;
        }
        let mut first = (minimum - origin) / delta;
        let mut second = (maximum - origin) / delta;
        if first > second {
            std::mem::swap(&mut first, &mut second);
        }
        enter = super::channel_solver::maximum(enter, first);
        exit = super::channel_solver::minimum(exit, second);
        enter <= exit
    };
    clip_axis(start.x, end.x - start.x, left, right)
        && clip_axis(start.y, end.y - start.y, top, bottom)
        && enter < exit
}

/// Route.createSegmentEndpoints over the full retained OVG node sequence.
pub(super) fn segment_endpoints_from_ovg_nodes(points: &[Point]) -> Option<Vec<Point>> {
    let mut result = vec![*points.get(1)?];
    let end = points.len().checked_sub(2)?;
    for index in 2..end {
        let (previous, current, next) = (points[index - 1], points[index], points[index + 1]);
        if (current.x == previous.x && current.x != next.x)
            || (current.y == previous.y && current.y != next.y)
        {
            result.push(current);
        }
    }
    result.push(points[end]);
    Some(result)
}

pub(super) fn simplify_route(points: Vec<Point>) -> Vec<Point> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_segment_endpoints_keeps_a_one_ulp_bend() {
        let next_x = f64::from_bits(1);
        let route = vec![
            Point { x: 0.0, y: 0.0 },
            Point { x: 0.0, y: 1.0 },
            Point { x: next_x, y: 2.0 },
        ];
        assert_eq!(simplify_route(route.clone()), route);

        assert_eq!(
            simplify_route(vec![
                Point { x: 0.0, y: 0.0 },
                Point { x: 0.0, y: 1.0 },
                Point { x: 0.0, y: 2.0 },
            ]),
            vec![Point { x: 0.0, y: 0.0 }, Point { x: 0.0, y: 2.0 }]
        );
    }
}
