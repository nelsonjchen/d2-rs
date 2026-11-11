// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Route and pairwise safety checks from channelProposalSafe (D2 v0.9.0).
//! Node obstruction checks remain the graph adapter's responsibility.
use super::channel_solver::{maximum, minimum};
use crate::Point;

pub(super) fn contact(a: Point, b: Point, c: Point, d: Point) -> bool {
    let udx = b.x - a.x;
    let vdx = d.x - c.x;
    let uvdx = c.x - a.x;
    let udy = b.y - a.y;
    let vdy = d.y - c.y;
    let uvdy = c.y - a.y;
    let denom = udy * vdx - udx * vdy;
    if denom == 0.0 {
        return false;
    }
    let s = (vdx * uvdy - vdy * uvdx) / denom;
    let t = (udx * uvdy - udy * uvdx) / denom;
    // Use explicit comparisons to retain upstream NaN behavior.
    #[allow(clippy::manual_range_contains)]
    {
        !(s < 0.0 || s > 1.0 || t < 0.0 || t > 1.0)
    }
}
fn orientation(a: Point, b: Point, c: Point) -> f64 {
    (b.y - a.y) * (c.x - a.x) - (b.x - a.x) * (c.y - a.y)
}
fn equal_signs(a: f64, b: f64) -> bool {
    (a > 0.0 && b > 0.0) || (a == 0.0 && b == 0.0) || (a < 0.0 && b < 0.0)
}
pub(in crate::internal) fn crossing(a: Point, b: Point, c: Point, d: Point) -> bool {
    if equal_signs(orientation(a, b, c), orientation(a, b, d))
        || equal_signs(orientation(c, d, a), orientation(c, d, b))
    {
        return false;
    }
    if a.x == b.x && (a.x == c.x || a.x == d.x) {
        return false;
    }
    if a.y == b.y && (a.y == c.y || a.y == d.y) {
        return false;
    }
    true
}
pub(super) fn overlap(a: Point, b: Point, c: Point, d: Point) -> f64 {
    if a.x == b.x && c.x == d.x && a.x == c.x {
        maximum(
            0.0,
            minimum(maximum(a.y, b.y), maximum(c.y, d.y))
                - maximum(minimum(a.y, b.y), minimum(c.y, d.y)),
        )
    } else if a.y == b.y && c.y == d.y && a.y == c.y {
        maximum(
            0.0,
            minimum(maximum(a.x, b.x), maximum(c.x, d.x))
                - maximum(minimum(a.x, b.x), minimum(c.x, d.x)),
        )
    } else {
        0.0
    }
}
