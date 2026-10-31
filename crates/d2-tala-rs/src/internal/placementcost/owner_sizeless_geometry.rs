// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Source placement distances and AxisScore over actual owner fields.
use super::*;
const NIL: &str = "runtime error: invalid memory address or nil pointer dereference";
fn node(node: Option<NodeId>) -> NodeId {
    node.expect(NIL)
}
fn point(frame: &OwnerScoreFrame<'_>, node_id: Option<NodeId>) -> Point {
    frame
        .position(node(node_id))
        .expect("validated owner point")
        .expect(NIL)
}
fn size(frame: &OwnerScoreFrame<'_>, node_id: Option<NodeId>) -> Size {
    frame
        .node(node(node_id))
        .expect("validated owner node")
        .rect
        .size
}
pub(super) fn source_min(a: f64, b: f64) -> f64 {
    if a == f64::NEG_INFINITY || b == f64::NEG_INFINITY {
        return f64::NEG_INFINITY;
    }
    #[cfg(target_arch = "aarch64")]
    {
        let quiet = 1u64 << 51;
        if a.is_nan() && a.to_bits() & quiet == 0 {
            return f64::from_bits(a.to_bits() | quiet);
        }
        if b.is_nan() && b.to_bits() & quiet == 0 {
            return f64::from_bits(b.to_bits() | quiet);
        }
        if b.is_nan() {
            return b;
        }
        if a.is_nan() {
            return a;
        }
    }
    #[cfg(not(target_arch = "aarch64"))]
    if a.is_nan() || b.is_nan() {
        return f64::from_bits(0x7ff8000000000001);
    }
    if a == 0. && a == b {
        if a.is_sign_negative() { a } else { b }
    } else if a < b {
        a
    } else {
        b
    }
}
fn source_max(a: f64, b: f64) -> f64 {
    super::owner_median::source_math_max(a, b)
}
fn gap(a: f64, b: f64) -> f64 {
    if a < b {
        b - a
    } else if b < a {
        a - b
    } else {
        0.
    }
}
pub(super) fn direct_distance(
    frame: &OwnerScoreFrame<'_>,
    first: Option<NodeId>,
    second: Option<NodeId>,
) -> f64 {
    let first = point(frame, first);
    let second = point(frame, second);
    let dx = gap(first.x, second.x);
    let dy = gap(first.y, second.y);
    if dx == 0. {
        dy.abs()
    } else if dy == 0. {
        dx.abs()
    } else {
        (dx * dx + dy * dy).sqrt()
    }
}
/// node.go distanceBetweenBoxes/intervalGap with live actual Boxes.
pub(super) fn sized_distance(frame: &OwnerScoreFrame<'_>, first: NodeId, second: NodeId) -> f64 {
    let a = point(frame, Some(first));
    let b = point(frame, Some(second));
    let sa = size(frame, Some(first));
    let sb = size(frame, Some(second));
    sized_box_distance((a, sa), (b, sb))
}

pub(super) fn sized_box_distance((a, sa): (Point, Size), (b, sb): (Point, Size)) -> f64 {
    let interval = |start: f64, end: f64, other_start: f64, other_end: f64| {
        if end < other_start {
            other_start - end
        } else if other_end < start {
            start - other_end
        } else {
            0.0
        }
    };
    let dx = interval(a.x, a.x + sa.width, b.x, b.x + sb.width);
    let dy = interval(a.y, a.y + sa.height, b.y, b.y + sb.height);
    if dx == 0.0 {
        dy.abs()
    } else if dy == 0.0 {
        dx.abs()
    } else {
        (dx * dx + dy * dy).sqrt()
    }
}

pub(super) fn placement_distance(
    frame: &OwnerScoreFrame<'_>,
    first: Option<NodeId>,
    second: Option<NodeId>,
) -> f64 {
    let distance = direct_distance(frame, first, second);
    let a = point(frame, first);
    let b = point(frame, second);
    distance + source_min((a.x - b.x).abs(), (a.y - b.y).abs()) / 20.
}
fn orientation(frame: &OwnerScoreFrame<'_>, a: Option<NodeId>, b: Option<NodeId>) -> Orientation {
    let Some(p) = frame.position(node(a)).expect("validated owner point") else {
        return Orientation::None;
    };
    let Some(q) = frame.position(node(b)).expect("validated owner point") else {
        return Orientation::None;
    };
    let a = size(frame, a);
    let b = size(frame, b);
    if p.y + a.height < q.y {
        if p.x + a.width < q.x {
            return Orientation::TopLeft;
        }
        if q.x + b.width < p.x {
            return Orientation::TopRight;
        }
        return Orientation::Top;
    }
    if q.y + b.height < p.y {
        if p.x + a.width < q.x {
            return Orientation::BottomLeft;
        }
        if q.x + b.width < p.x {
            return Orientation::BottomRight;
        }
        return Orientation::Bottom;
    }
    if q.x + b.width < p.x {
        return Orientation::Right;
    }
    if p.x + a.width < q.x {
        return Orientation::Left;
    }
    Orientation::None
}
fn turn(p: Point, q: Point, r: Point) -> f64 {
    let pq_x = q.x - p.x;
    let pq_y = q.y - p.y;
    let pr_x = r.x - p.x;
    let pr_y = r.y - p.y;
    pq_y * pr_x - pq_x * pr_y
}
fn interval(a: f64, b: f64, c: f64, d: f64) -> bool {
    source_min(a, b) <= source_max(c, d) && source_min(c, d) <= source_max(a, b)
}
fn crosses(a: Point, b: Point, c: Point, d: Point) -> bool {
    let c_side = turn(a, b, c);
    let d_side = turn(a, b, d);
    let a_side = turn(c, d, a);
    let b_side = turn(c, d, b);
    if c_side == 0. && d_side == 0. && a_side == 0. && b_side == 0. {
        return interval(a.x, b.x, c.x, d.x) && interval(a.y, b.y, c.y, d.y);
    }
    let straddles = |a: f64, b: f64| a == 0. || b == 0. || (a < 0.) != (b < 0.);
    straddles(c_side, d_side) && straddles(a_side, b_side)
}
fn overlaps(frame: &OwnerScoreFrame<'_>, id: Option<NodeId>, a: Point, b: Point) -> bool {
    let p = point(frame, id);
    let s = size(frame, id);
    let l = p.x;
    let r = p.x + s.width;
    let t = p.y;
    let d = p.y + s.height;
    let contains = |q: Point| l <= q.x && r >= q.x && t <= q.y && d >= q.y;
    if contains(a) || contains(b) {
        return true;
    }
    let tl = Point { x: l, y: t };
    let tr = Point { x: r, y: t };
    let bl = Point { x: l, y: d };
    let br = Point { x: r, y: d };
    crosses(tl, tr, a, b) || crosses(tr, br, a, b) || crosses(br, bl, a, b) || crosses(bl, tl, a, b)
}
fn center(frame: &OwnerScoreFrame<'_>, id: Option<NodeId>) -> Point {
    let p = point(frame, id);
    let s = size(frame, id);
    Point {
        x: p.x + s.width / 2.,
        y: p.y + s.height / 2.,
    }
}
pub(super) fn axis_score(frame: &OwnerScoreFrame<'_>, nodes: &[Option<NodeId>]) -> f64 {
    if nodes.len() < 2 {
        return 1.;
    }
    if nodes.len() == 2 {
        return if orientation(frame, nodes[0], nodes[1]).is_diagonal() {
            0.
        } else {
            1.
        };
    }
    let mut width = f64::NEG_INFINITY;
    let mut height = f64::NEG_INFINITY;
    let mut widest = None;
    let mut tallest = None;
    for &id in nodes {
        let s = size(frame, id);
        if s.width > width {
            width = s.width;
            widest = id
        }
        if s.height > height {
            height = s.height;
            tallest = id
        }
    }
    let mut vertical = true;
    for &id in nodes {
        if id == tallest {
            continue;
        }
        if !orientation(frame, id, tallest).is_horizontal() {
            vertical = false;
            break;
        }
    }
    if vertical {
        return axis_aligned(frame, nodes, tallest, false);
    }
    let mut horizontal = true;
    for &id in nodes {
        if id == widest {
            continue;
        }
        if !orientation(frame, id, widest).is_vertical() {
            horizontal = false;
            break;
        }
    }
    if horizontal {
        return axis_aligned(frame, nodes, widest, true);
    }
    0.
}
fn axis_aligned(
    frame: &OwnerScoreFrame<'_>,
    nodes: &[Option<NodeId>],
    reference: Option<NodeId>,
    x_axis: bool,
) -> f64 {
    let c = center(frame, reference);
    let p = point(frame, reference);
    let s = size(frame, reference);
    let reference_extent = if x_axis { s.width } else { s.height };
    let mut total = 0.;
    for &id in nodes {
        if id == reference {
            continue;
        }
        let mut score = 0.;
        for fraction in [0.25, 0.5, 0.75] {
            let other = center(frame, id);
            let (a, b) = if x_axis {
                let x = p.x + fraction * s.width;
                (Point { x, y: c.y }, Point { x, y: other.y })
            } else {
                let y = p.y + fraction * s.height;
                (Point { x: c.x, y }, Point { x: other.x, y })
            };
            if overlaps(frame, id, a, b) {
                score += 0.33
            }
        }
        let other = size(frame, id);
        let extent = if x_axis { other.width } else { other.height };
        if extent < 0.25 * reference_extent {
            score *= 3.
        } else if extent < 0.75 * reference_extent {
            score *= 2.
        }
        if score == 0.99 {
            total += 1.
        } else {
            total += score
        }
    }
    source_max(0.33, total / (nodes.len() - 1) as f64)
}
