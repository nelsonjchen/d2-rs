// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Route and pairwise safety checks from channelProposalSafe (D2 v0.9.0).
//! Node obstruction checks remain the graph adapter's responsibility.
use super::channel_points::go_hypot;
use super::channel_solver::{maximum, minimum};
use crate::Point;
const EPSILON: f64 = 1e-6;

pub(super) struct Edge<'a> {
    pub before: &'a [Point],
    pub after: &'a [Point],
    pub is_curve: bool,
}

pub(super) trait Guard {
    type Error;
    fn step(&mut self) -> Result<(), Self::Error>;
    /// Must implement changedRouteIsClear with fixedPorts=true, including
    /// endpoint/direction preservation and unrelated-node obstruction checks.
    fn route_clear(
        &mut self,
        index: usize,
        before: &[Point],
        after: &[Point],
    ) -> Result<bool, Self::Error>;
}

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

pub(super) fn check<G: Guard>(edges: &[Edge<'_>], guard: &mut G) -> Result<bool, G::Error> {
    let mut changed = vec![false; edges.len()];
    for (index, e) in edges.iter().enumerate() {
        assert_eq!(
            e.before.len(),
            e.after.len(),
            "proposal preserves point inventory"
        );
        for (before, after) in e.before.iter().zip(e.after) {
            guard.step()?;
            changed[index] |= before != after;
        }
        if !changed[index] {
            continue;
        }
        if e.is_curve {
            return Ok(false);
        }
        if !guard.route_clear(index, e.before, e.after)? {
            return Ok(false);
        }
        let mut before_length = 0.0;
        let mut after_length = 0.0;
        let mut nonzero: Vec<Point> = Vec::new();
        for (i, &point) in e.after.iter().enumerate() {
            guard.step()?;
            if !point.x.is_finite() || !point.y.is_finite() {
                return Ok(false);
            }
            if i > 0 {
                let old = e.before[i - 1];
                let previous = e.after[i - 1];
                let before = go_hypot(e.before[i].x - old.x, e.before[i].y - old.y);
                let after = go_hypot(point.x - previous.x, point.y - previous.y);
                before_length += before;
                after_length += after;
                if after > EPSILON && after + EPSILON < minimum(before, 40.0) {
                    return Ok(false);
                }
            }
            if nonzero.last() != Some(&point) {
                nonzero.push(point);
            }
        }
        if after_length > before_length + EPSILON {
            return Ok(false);
        }
        for triple in nonzero.windows(3) {
            let [a, b, c] = triple else { unreachable!() };
            if (b.x - a.x) * (c.x - b.x) + (b.y - a.y) * (c.y - b.y) < 0.0 {
                return Ok(false);
            }
        }
        for i in 0..nonzero.len().saturating_sub(1) {
            for j in i + 2..nonzero.len().saturating_sub(1) {
                guard.step()?;
                if contact(nonzero[i], nonzero[i + 1], nonzero[j], nonzero[j + 1]) {
                    return Ok(false);
                }
            }
        }
    }
    for (i, e) in edges.iter().enumerate() {
        for (j, other) in edges.iter().enumerate().skip(i + 1) {
            if !changed[i] && !changed[j] {
                continue;
            }
            if e.is_curve || other.is_curve {
                return Ok(false);
            }
            let mut before_crossings = 0;
            let mut after_crossings = 0;
            for (a, old_a) in e.before.windows(2).enumerate() {
                for (b, old_b) in other.before.windows(2).enumerate() {
                    guard.step()?;
                    let new_a = &e.after[a..a + 2];
                    let new_b = &other.after[b..b + 2];
                    before_crossings +=
                        usize::from(crossing(old_a[0], old_a[1], old_b[0], old_b[1]));
                    after_crossings +=
                        usize::from(crossing(new_a[0], new_a[1], new_b[0], new_b[1]));
                    let old = overlap(old_a[0], old_a[1], old_b[0], old_b[1]);
                    let new = overlap(new_a[0], new_a[1], new_b[0], new_b[1]);
                    if old == 0.0 && new > 0.0 {
                        return Ok(false);
                    }
                    let old_contact = contact(old_a[0], old_a[1], old_b[0], old_b[1]);
                    let new_contact = contact(new_a[0], new_a[1], new_b[0], new_b[1]);
                    if !old_contact && new_contact {
                        return Ok(false);
                    }
                }
            }
            if after_crossings > before_crossings {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

/// Graph adapter resolves this per edge: only non-endpoint ancestor containers
/// may be skipped. Keep all nodes in inventory order for work accounting.
pub(super) struct Obstacle {
    pub top_left: Option<Point>,
    pub width: f64,
    pub height: f64,
    pub allowed_ancestor: bool,
}

fn same_direction(a: Point, b: Point, c: Point, d: Point) -> bool {
    (b.x - a.x) * (d.x - c.x) + (b.y - a.y) * (d.y - c.y) > 0.0
        && (b.x - a.x) * (d.y - c.y) == (b.y - a.y) * (d.x - c.x)
}

/// changedRouteIsClear with fixedPorts=true and its open-interior node scan.
pub(super) fn route_clear<E>(
    before: &[Point],
    after: &[Point],
    nodes: &[Obstacle],
    mut reserve: impl FnMut(u64) -> Result<(), E>,
) -> Result<bool, E> {
    if after.len() < 2 || before.len() < 2 {
        return Ok(false);
    }
    if after[0] != before[0]
        || after.last() != before.last()
        || !same_direction(before[0], before[1], after[0], after[1])
        || !same_direction(
            before[before.len() - 2],
            before[before.len() - 1],
            after[after.len() - 2],
            after[after.len() - 1],
        )
    {
        return Ok(false);
    }
    for pair in after.windows(2) {
        let [a, b] = pair else { unreachable!() };
        let mut unchanged = false;
        for old in before.windows(2) {
            reserve(1)?;
            if *a == old[0] && *b == old[1] {
                unchanged = true;
                break;
            }
        }
        if unchanged {
            continue;
        }
        if a.x != b.x && a.y != b.y {
            return Ok(false);
        }
        for node in nodes {
            reserve(1)?;
            let Some(origin) = node.top_left else {
                continue;
            };
            if node.width <= 0.0 || node.height <= 0.0 || node.allowed_ancestor {
                continue;
            }
            let left = origin.x + EPSILON;
            let right = origin.x + node.width - EPSILON;
            let top = origin.y + EPSILON;
            let bottom = origin.y + node.height - EPSILON;
            let enters = if a.y == b.y {
                top < a.y && a.y < bottom && minimum(a.x, b.x) < right && maximum(a.x, b.x) > left
            } else {
                left < a.x && a.x < right && minimum(a.y, b.y) < bottom && maximum(a.y, b.y) > top
            };
            if enters {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn p(x: f64, y: f64) -> Point {
        Point { x, y }
    }
    struct TestGuard {
        remaining: usize,
        clear: bool,
    }
    impl Guard for TestGuard {
        type Error = &'static str;
        fn step(&mut self) -> Result<(), Self::Error> {
            if self.remaining == 0 {
                Err("cancelled")
            } else {
                self.remaining -= 1;
                Ok(())
            }
        }
        fn route_clear(&mut self, _: usize, _: &[Point], _: &[Point]) -> Result<bool, Self::Error> {
            Ok(self.clear)
        }
    }
    fn guard() -> TestGuard {
        TestGuard {
            remaining: 10000,
            clear: true,
        }
    }
    #[test]
    fn short_leg_is_rejected_but_normal_interior_move_is_allowed() {
        let before = [
            p(100.0, 50.0),
            p(200.0, 50.0),
            p(200.0, 250.0),
            p(300.0, 250.0),
        ];
        for (x, want) in [(105.0, false), (150.0, true)] {
            let after = [before[0], p(x, 50.0), p(x, 250.0), before[3]];
            assert_eq!(
                check(
                    &[Edge {
                        before: &before,
                        after: &after,
                        is_curve: false
                    }],
                    &mut guard()
                )
                .unwrap(),
                want
            );
        }
    }
    #[test]
    fn new_point_contact_is_rejected_even_without_new_crossing() {
        let before = [
            p(100.0, 50.0),
            p(150.0, 50.0),
            p(150.0, 250.0),
            p(300.0, 250.0),
        ];
        let after = [before[0], p(200.0, 50.0), p(200.0, 250.0), before[3]];
        let other = [p(200.0, 150.0), p(250.0, 150.0)];
        assert!(
            !check(
                &[
                    Edge {
                        before: &before,
                        after: &after,
                        is_curve: false
                    },
                    Edge {
                        before: &other,
                        after: &other,
                        is_curve: false
                    }
                ],
                &mut guard()
            )
            .unwrap()
        );
    }
    #[test]
    fn curve_obstruction_and_cancellation_reject_updates() {
        let before = [p(0.0, 0.0), p(100.0, 0.0)];
        let after = [p(0.0, 1.0), p(100.0, 1.0)];
        assert!(
            !check(
                &[Edge {
                    before: &before,
                    after: &after,
                    is_curve: true
                }],
                &mut guard()
            )
            .unwrap()
        );
        let edge = Edge {
            before: &before,
            after: &after,
            is_curve: false,
        };
        assert!(
            !check(
                &[edge],
                &mut TestGuard {
                    remaining: 10000,
                    clear: false
                }
            )
            .unwrap()
        );
        let edge = Edge {
            before: &before,
            after: &after,
            is_curve: false,
        };
        assert_eq!(
            check(
                &[edge],
                &mut TestGuard {
                    remaining: 1,
                    clear: true
                }
            ),
            Err("cancelled")
        );
    }
    #[test]
    fn clear_route_preserves_ports_and_rejects_open_node_interior() {
        let before = [p(0.0, 0.0), p(100.0, 0.0), p(100.0, 100.0), p(200.0, 100.0)];
        let after = [before[0], p(150.0, 0.0), p(150.0, 100.0), before[3]];
        let mut nodes = [Obstacle {
            top_left: Some(p(140.0, 30.0)),
            width: 20.0,
            height: 20.0,
            allowed_ancestor: false,
        }];
        let allow = |_: u64| Ok::<(), ()>(());
        assert!(!route_clear(&before, &after, &nodes, allow).unwrap());
        nodes[0].allowed_ancestor = true;
        assert!(route_clear(&before, &after, &nodes, allow).unwrap());
        nodes[0].allowed_ancestor = false;
        nodes[0].top_left = Some(p(150.0, 30.0));
        assert!(route_clear(&before, &after, &nodes, allow).unwrap());
        let mut moved_port = after;
        moved_port[0].x = 1.0;
        assert!(!route_clear(&before, &moved_port, &[], allow).unwrap());
    }
}
