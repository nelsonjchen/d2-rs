// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Pinned routing/swap_ports.go and tunnel.go over actual owner storage.
use super::channel_solver::{maximum, minimum};
use super::route_search_guard::{MAX_ROUTE_STAGE_WORK_UNITS, RouteStageWork};
use super::route_stage_guard::{self, RouteAtomicGuard, RouteStageError, RuntimeRouteContext};
use crate::internal::owner_groups::OwnerGroupKind;
use crate::internal::owner_points::SharedOwnerPoint;
use crate::internal::{ArenaGraph, OwnerGraphId, RoutePoints, limits, quality};
use crate::{EdgeId, NodeId, Point, Rect, Size};
type Result<T> = std::result::Result<T, RouteStageError>;
type Ports = Vec<(Point, Vec<EdgeId>)>;
#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Top,
    Bottom,
    Left,
    Right,
}
impl Side {
    fn vertical(self) -> bool {
        matches!(self, Self::Top | Self::Bottom)
    }
}
fn node_owner(g: &ArenaGraph, node: NodeId) -> OwnerGraphId {
    g.topology_node_owner(node).unwrap_or_else(|| {
        std::panic::panic_any("runtime error: invalid memory address or nil pointer dereference")
    })
}
fn members(g: &ArenaGraph, receiver: OwnerGraphId) -> impl Iterator<Item = NodeId> + use<> {
    g.owner_graph_view(receiver).unwrap().nodes.require_nodes()
}
fn points(g: &ArenaGraph, edge: EdgeId, node: NodeId) -> [SharedOwnerPoint; 3] {
    let e = &g.edge_records[edge.0 as usize];
    std::array::from_fn(|i| {
        e.route
            .slot(if e.from == node {
                i
            } else {
                e.route.len() - 1 - i
            })
            .unwrap()
    })
}
fn values(g: &ArenaGraph, edge: EdgeId, node: NodeId) -> [Point; 3] {
    points(g, edge, node).map(|p| p.get())
}
fn side(p: Point, q: Point) -> Side {
    if p.x < q.x {
        Side::Right
    } else if p.x > q.x {
        Side::Left
    } else if p.y < q.y {
        Side::Bottom
    } else {
        Side::Top
    }
}
fn arrow(g: &ArenaGraph, edge: EdgeId, node: NodeId) -> &str {
    let e = &g.edge_records[edge.0 as usize];
    if e.from == node {
        e.source_arrowhead.as_deref().unwrap_or("")
    } else {
        e.target_arrowhead.as_deref().unwrap_or("")
    }
}
fn can_swap(g: &ArenaGraph, a: EdgeId, b: EdgeId, node: NodeId, ports: &Ports) -> bool {
    let count = |p: Point| {
        ports
            .iter()
            .find(|(q, _)| *q == p)
            .map_or(0, |(_, es)| es.len())
    };
    (count(values(g, a, node)[0]) == 1 && count(values(g, b, node)[0]) == 1)
        || arrow(g, a, node) == arrow(g, b, node)
}
fn intersects(g: &ArenaGraph, a: EdgeId, b: EdgeId, node: NodeId, o: Side) -> bool {
    let [p1, p2, p3] = values(g, a, node);
    let [q1, q2, q3] = values(g, b, node);
    match o {
        Side::Left => (q2.x < p2.x && p3.y > q1.y) || (p2.x < q2.x && q3.y < p1.y),
        Side::Bottom => (p2.y > q2.y && q3.x < p1.x) || (q2.y > p2.y && p3.x > q1.x),
        Side::Top => (p2.y < q2.y && q3.x < p1.x) || (q2.y < p2.y && p3.x > q1.x),
        Side::Right => (p2.x > q2.x && q3.y < p1.y) || (q2.x > p2.x && p3.y > q1.y),
    }
}
// Go's tuple assignments read both coordinates before writing either allocation.
fn swap_coordinate(a: &SharedOwnerPoint, b: &SharedOwnerPoint, x: bool) {
    let (mut p, mut q) = (a.get(), b.get());
    if x {
        std::mem::swap(&mut p.x, &mut q.x);
    } else {
        std::mem::swap(&mut p.y, &mut q.y);
    }
    a.set(p);
    b.set(q);
}
fn swap_points(a: &SharedOwnerPoint, b: &SharedOwnerPoint) {
    swap_coordinate(a, b, true);
    swap_coordinate(a, b, false);
}
fn rect(g: &ArenaGraph, node: NodeId) -> Rect {
    Rect {
        origin: g.position(node).unwrap_or_else(|| {
            std::panic::panic_any(
                "runtime error: invalid memory address or nil pointer dereference",
            )
        }),
        size: g.nodes[node.0 as usize].rect.size,
    }
}
// routing/geometry.go: keep Go's NaN-aware Min/Max even for derived overflow.
fn segment_box(a: Point, b: Point, box_: Rect) -> bool {
    let l = minimum(box_.origin.x, box_.right());
    let r = maximum(box_.origin.x, box_.right());
    let t = minimum(box_.origin.y, box_.bottom());
    let btm = maximum(box_.origin.y, box_.bottom());
    if maximum(a.x, b.x) < l
        || r < minimum(a.x, b.x)
        || maximum(a.y, b.y) < t
        || btm < minimum(a.y, b.y)
    {
        return false;
    }
    let contains = |p: Point| l <= p.x && p.x <= r && t <= p.y && p.y <= btm;
    if contains(a) || contains(b) {
        return true;
    }
    let (mut enter, mut exit) = (0.0, 1.0);
    let mut clip = |start: f64, delta: f64, lo: f64, hi: f64| {
        if delta == 0.0 {
            return lo <= start && start <= hi;
        }
        let (mut one, mut two) = ((lo - start) / delta, (hi - start) / delta);
        if one > two {
            std::mem::swap(&mut one, &mut two);
        }
        enter = maximum(enter, one);
        exit = minimum(exit, two);
        enter <= exit
    };
    clip(a.x, b.x - a.x, l, r) && clip(a.y, b.y - a.y, t, btm) && enter < exit
}
fn clear(
    g: &ArenaGraph,
    receiver: OwnerGraphId,
    ignored: &[NodeId],
    p: [&SharedOwnerPoint; 4],
    guard: &RouteAtomicGuard<'_>,
) -> Result<bool> {
    'node: for node in members(g, receiver) {
        guard.step()?;
        for &n in ignored {
            guard.step()?;
            if node == n || super::shortcut::descendant(g, Some(n), node) {
                continue 'node;
            }
        }
        let box_ = rect(g, node);
        let padded = Rect {
            origin: Point {
                x: box_.origin.x - 10.0,
                y: box_.origin.y - 10.0,
            },
            size: Size {
                width: box_.size.width + 20.0,
                height: box_.size.height + 20.0,
            },
        };
        if segment_box(p[0].get(), p[1].get(), padded)
            || segment_box(p[2].get(), p[3].get(), padded)
        {
            return Ok(false);
        }
    }
    Ok(true)
}
fn sort(
    g: &ArenaGraph,
    node: NodeId,
    o: Side,
    edges: &mut [EdgeId],
    guard: &RouteAtomicGuard<'_>,
) -> Result<()> {
    if edges.len() < 2 {
        return guard.step();
    }
    let less = |a: EdgeId, b: EdgeId| {
        let [p, _, r] = values(g, a, node);
        let [q, _, s] = values(g, b, node);
        if o.vertical() {
            if p.x == q.x { r.x < s.x } else { p.x < q.x }
        } else if p.y == q.y {
            r.y < s.y
        } else {
            p.y < q.y
        }
    };
    let mut temporary = edges.to_vec();
    let mut width = 1;
    while width < edges.len() {
        for start in (0..edges.len()).step_by(2 * width) {
            let mid = (start + width).min(edges.len());
            let end = (start + 2 * width).min(edges.len());
            let (mut left, mut right, mut out) = (start, mid, start);
            while left < mid && right < end {
                guard.step()?;
                if less(edges[right], edges[left]) {
                    temporary[out] = edges[right];
                    right += 1;
                } else {
                    temporary[out] = edges[left];
                    left += 1;
                }
                out += 1;
            }
            while left < mid {
                guard.step()?;
                temporary[out] = edges[left];
                left += 1;
                out += 1;
            }
            while right < end {
                guard.step()?;
                temporary[out] = edges[right];
                right += 1;
                out += 1;
            }
        }
        guard.add(edges.len() as u64)?;
        edges.copy_from_slice(&temporary);
        if width > edges.len() / 2 {
            break;
        }
        width *= 2;
    }
    Ok(())
}
fn copy_route(g: &ArenaGraph, id: EdgeId, guard: &RouteAtomicGuard<'_>) -> Result<RoutePoints> {
    let route = &g.edge_records[id.0 as usize].route;
    let mut out = Vec::with_capacity(route.len());
    for i in 0..route.len() {
        guard.step()?;
        out.push(Some(SharedOwnerPoint::new(route.point(i))));
    }
    Ok(RoutePoints::pointer_literal(&out))
}
fn insert(
    g: &mut ArenaGraph,
    id: EdgeId,
    p: SharedOwnerPoint,
    index: usize,
    guard: &RouteAtomicGuard<'_>,
) -> Result<()> {
    let route = &g.edge_records[id.0 as usize].route;
    let mut out = Vec::with_capacity(route.len() + 1);
    for i in 0..route.len() {
        guard.step()?;
        if i == index {
            out.push(Some(p.clone()));
        }
        out.push(route.slot(i));
    }
    if index == route.len() {
        out.push(Some(p));
    }
    g.edge_records[id.0 as usize]
        .route
        .replace(RoutePoints::pointer_literal(&out));
    Ok(())
}
fn remove(
    g: &mut ArenaGraph,
    id: EdgeId,
    indices: &[usize],
    guard: &RouteAtomicGuard<'_>,
) -> Result<()> {
    let route = &g.edge_records[id.0 as usize].route;
    let mut out = Vec::with_capacity(route.len() - indices.len());
    'point: for i in 0..route.len() {
        guard.step()?;
        for &j in indices {
            guard.step()?;
            if i == j {
                continue 'point;
            }
        }
        out.push(route.slot(i));
    }
    g.edge_records[id.0 as usize]
        .route
        .replace(RoutePoints::pointer_literal(&out));
    Ok(())
}
fn occupied(g: &ArenaGraph, node: NodeId, guard: &RouteAtomicGuard<'_>) -> Result<Vec<Point>> {
    let mut out = Vec::new();
    for id in g
        .owner_node_incident_header(node)
        .unwrap()
        .iter_slots()
        .map(Option::unwrap)
    {
        guard.step()?;
        let e = &g.edge_records[id.0 as usize];
        let p = e
            .route
            .point(if e.from == node { 0 } else { e.route.len() - 1 });
        if !out.contains(&p) {
            out.push(p);
        }
    }
    Ok(out)
}
fn is_u(p: &[Point]) -> bool {
    if p.len() != 4 {
        return false;
    }
    let sign = |x: f64| {
        if x > 0.0 {
            1
        } else if x < 0.0 {
            -1
        } else {
            0
        }
    };
    if p[0].x == p[1].x {
        sign(p[0].y - p[1].y) == sign(p[3].y - p[2].y) && p[1].y == p[2].y
    } else {
        sign(p[0].x - p[1].x) == sign(p[3].x - p[2].x) && p[1].x == p[2].x
    }
}
fn is_s(p: &[Point]) -> bool {
    p.len() == 4 && !is_u(p)
}
fn is_l(p: &[Point]) -> bool {
    p.len() == 3 && ((p[0].x == p[1].x) == (p[1].y == p[2].y))
}
fn descendant(
    g: &ArenaGraph,
    mut current: Option<NodeId>,
    ancestor: NodeId,
    guard: &RouteAtomicGuard<'_>,
) -> Result<bool> {
    loop {
        guard.step()?;
        if current == Some(ancestor) {
            return Ok(true);
        }
        let Some(node) = current else {
            return Ok(false);
        };
        current = quality::owner_ancestry_parent(g, node);
        if current.is_none() {
            return Ok(false);
        }
    }
}
fn blocked(o: Rect, a: Rect, b: Rect, h: bool) -> bool {
    if h {
        o.origin.x >= a.right()
            && o.right() <= b.origin.x
            && o.origin.y <= maximum(a.origin.y, b.origin.y)
            && o.bottom() >= minimum(a.bottom(), b.bottom())
    } else {
        o.origin.y >= a.bottom()
            && o.bottom() <= b.origin.y
            && o.origin.x <= maximum(a.origin.x, b.origin.x)
            && o.right() >= minimum(a.right(), b.right())
    }
}
fn sequence_right(g: &ArenaGraph, node: NodeId, r: Rect) -> f64 {
    if let Some(group) = g
        .owner_node_group_id(node, OwnerGroupKind::Sequence)
        .unwrap()
    {
        let fields = g.owner_group_fields(group).unwrap();
        if fields.nodes.get(fields.nodes.len() - 1).unwrap() != Some(node) {
            return r.right() - 35.0;
        }
    }
    r.right()
}
fn tunnels(
    g: &ArenaGraph,
    receiver: OwnerGraphId,
    a: NodeId,
    b: NodeId,
    guard: &RouteAtomicGuard<'_>,
) -> Result<(Vec<(f64, f64)>, bool)> {
    guard.step()?;
    let ra = rect(g, a);
    let rb = rect(g, b);
    let horizontal = !(ra.origin.y > rb.bottom() || ra.bottom() < rb.origin.y);
    if !horizontal {
        guard.step()?;
        let ra = rect(g, a);
        let rb = rect(g, b);
        if ra.origin.x > rb.right() || ra.right() < rb.origin.x {
            return Ok((Vec::new(), false));
        }
    }
    let ra = rect(g, a);
    let rb = rect(g, b);
    let (start, end) = if horizontal {
        (
            maximum(ra.origin.y, rb.origin.y),
            minimum(ra.bottom(), rb.bottom()),
        )
    } else {
        (
            maximum(ra.origin.x, rb.origin.x),
            minimum(sequence_right(g, a, ra), sequence_right(g, b, rb)),
        )
    };
    guard.step()?;
    let mut ranges = vec![(start, end)];
    for node in members(g, receiver) {
        guard.step()?;
        if node == a || node == b {
            continue;
        }
        let below_a = descendant(g, Some(node), a, guard)?;
        let above_a = descendant(g, Some(a), node, guard)?;
        let below_b = descendant(g, Some(node), b, guard)?;
        let above_b = descendant(g, Some(b), node, guard)?;
        if below_a || above_a || below_b || above_b {
            continue;
        }
        let (o, ra, rb) = (rect(g, node), rect(g, a), rect(g, b));
        if blocked(o, ra, rb, horizontal) || blocked(o, rb, ra, horizontal) {
            return Ok((Vec::new(), false));
        }
        let (a, b, v) = if horizontal {
            (ra.origin.x, rb.origin.x, o.origin.x)
        } else {
            (ra.origin.y, rb.origin.y, o.origin.y)
        };
        if !((a < v && v < b) || (b < v && v < a)) {
            continue;
        }
        let mut new = Vec::new();
        for (start, end) in ranges {
            guard.step()?;
            let o = rect(g, node);
            let (lo, hi) = if horizontal {
                (o.origin.y, o.bottom())
            } else {
                (o.origin.x, o.right())
            };
            if lo <= start && hi >= end {
                continue;
            }
            if lo > start && hi < end {
                new.extend([(start, lo), (hi, end)]);
                continue;
            }
            if lo <= start && hi < end && hi > start {
                new.push((hi, end));
                continue;
            }
            if lo > start && hi >= end && lo < end {
                new.push((start, lo));
                continue;
            }
            new.push((start, end));
        }
        guard.finish()?;
        ranges = Vec::new();
        for range in new {
            guard.step()?;
            ranges.push(range);
        }
    }
    Ok((ranges, horizontal))
}
fn straight(
    g: &mut ArenaGraph,
    receiver: OwnerGraphId,
    id: EdgeId,
    ports: &[Point],
    guard: &RouteAtomicGuard<'_>,
) -> Result<bool> {
    let e = &g.edge_records[id.0 as usize];
    let (a, b) = (e.from, e.to);
    let from = e.route.slot(0).unwrap();
    let to = e.route.slot(e.route.len() - 1).unwrap();
    let (ranges, horizontal) = tunnels(g, receiver, a, b, guard)?;
    for (start, end) in ranges {
        guard.step()?;
        let mut v = start + 5.0;
        while v <= end - 5.0 {
            guard.step()?;
            let (from, to) = if horizontal {
                (
                    Point {
                        x: from.get().x,
                        y: v,
                    },
                    Point {
                        x: to.get().x,
                        y: v,
                    },
                )
            } else {
                (
                    Point {
                        x: v,
                        y: from.get().y,
                    },
                    Point {
                        x: v,
                        y: to.get().y,
                    },
                )
            };
            if !ports.contains(&from) && !ports.contains(&to) {
                g.edge_records[id.0 as usize]
                    .route
                    .replace(RoutePoints::literal(&[from, to]));
                return Ok(true);
            }
            v += 5.0;
        }
    }
    Ok(false)
}
fn refine(
    g: &mut ArenaGraph,
    receiver: OwnerGraphId,
    id: EdgeId,
    guard: &RouteAtomicGuard<'_>,
) -> Result<bool> {
    guard.step()?;
    let e = &g.edge_records[id.0 as usize];
    let (a, b) = (e.from, e.to);
    if a == b
        || e.route.len() <= 2
        || (e.source_table_column.is_some() && e.target_table_column.is_some())
    {
        return Ok(false);
    }
    if g.owner_graph_node_to_tree_entry(node_owner(g, a), Some(a))
        .unwrap()
        .is_some()
        || g.owner_graph_node_to_tree_entry(node_owner(g, b), Some(b))
            .unwrap()
            .is_some()
    {
        return Ok(false);
    }
    let mut ports = occupied(g, a, guard)?;
    for port in occupied(g, b, guard)? {
        guard.step()?;
        if !ports.contains(&port) {
            ports.push(port);
        }
    }
    let route = &g.edge_records[id.0 as usize].route;
    let p = route.read();
    if is_s(&p) || is_l(&p) {
        return straight(g, receiver, id, &ports, guard);
    }
    if is_u(&p) {
        return Ok(false);
    }
    if is_s(&p[..4]) {
        let bend = SharedOwnerPoint::new(if p[0].x == p[1].x {
            Point {
                x: p[1].x,
                y: p[3].y,
            }
        } else {
            Point {
                x: p[3].x,
                y: p[1].y,
            }
        });
        let first = route.slot(0).unwrap();
        let fourth = route.slot(3).unwrap();
        if clear(g, receiver, &[a, b], [&first, &bend, &bend, &fourth], guard)? {
            remove(g, id, &[1, 2, 3], guard)?;
            insert(g, id, bend, 1, guard)?;
            return Ok(true);
        }
    }
    Ok(false)
}
fn intersection(u: Point, v: Point, p: Point, q: Point) -> Option<Point> {
    let (udx, vdx, uvdx, udy, vdy, uvdy) = (
        v.x - u.x,
        q.x - p.x,
        p.x - u.x,
        v.y - u.y,
        q.y - p.y,
        p.y - u.y,
    );
    let denominator = udy * vdx - udx * vdy;
    if denominator == 0.0 {
        return None;
    }
    let s = (vdx * uvdy - vdy * uvdx) / denominator;
    let t = (udx * uvdy - udy * uvdx) / denominator;
    if s < 0.0 || s > 1.0 || t < 0.0 || t > 1.0 {
        return None;
    }
    Some(Point {
        x: u.x + (s * udx).round(),
        y: u.y + (s * udy).round(),
    })
}
fn insert_intersection(
    g: &mut ArenaGraph,
    id: EdgeId,
    node: NodeId,
    p: Point,
    second: &SharedOwnerPoint,
    guard: &RouteAtomicGuard<'_>,
) -> Result<()> {
    let intersection = SharedOwnerPoint::new(p);
    let e = &g.edge_records[id.0 as usize];
    let index = if e.from == node { 2 } else { e.route.len() - 2 };
    insert(g, id, intersection.clone(), index, guard)?;
    let mut p = intersection.get();
    let mut q = second.get();
    if p.x == q.x {
        p.x += 2.5;
        q.x += 2.5;
    } else {
        p.y -= 2.5;
        q.y -= 2.5;
    }
    intersection.set(p);
    second.set(q);
    Ok(())
}
fn node(g: &mut ArenaGraph, node: NodeId, guard: &RouteAtomicGuard<'_>) -> Result<bool> {
    let mut groups: Vec<(Side, Vec<EdgeId>)> = Vec::new();
    let mut ports: Ports = Vec::new();
    for id in g
        .owner_node_incident_header(node)
        .unwrap()
        .iter_slots()
        .map(Option::unwrap)
    {
        guard.step()?;
        let e = &g.edge_records[id.0 as usize];
        if e.from == e.to
            || e.route.len() < 3
            || e.has_table_column()
            || e.source_arrowhead_label
                .as_ref()
                .is_some_and(|l| l.text.len() > 3)
            || e.target_arrowhead_label
                .as_ref()
                .is_some_and(|l| l.text.len() > 3)
        {
            continue;
        }
        let [p, q, _] = values(g, id, node);
        let o = side(p, q);
        if let Some((_, edges)) = groups.iter_mut().find(|(x, _)| *x == o) {
            edges.push(id);
        } else {
            groups.push((o, vec![id]));
        }
        if let Some((_, edges)) = ports.iter_mut().find(|(x, _)| *x == p) {
            edges.push(id);
        } else {
            ports.push((p, vec![id]));
        }
    }
    let mut swapped = false;
    for (o, edges) in &mut groups {
        guard.step()?;
        sort(g, node, *o, edges, guard)?;
        for i in 0..edges.len().saturating_sub(1) {
            guard.step()?;
            let (a, b) = (edges[i], edges[i + 1]);
            if intersects(g, a, b, node, *o) && can_swap(g, a, b, node, &ports) {
                let [p1, p2, _] = points(g, a, node);
                let [q1, q2, _] = points(g, b, node);
                swap_coordinate(&p1, &q1, o.vertical());
                swap_coordinate(&p2, &q2, o.vertical());
                if clear(g, node_owner(g, node), &[node], [&p1, &p2, &q1, &q2], guard)? {
                    edges.swap(i, i + 1);
                    swapped = true;
                } else {
                    swap_coordinate(&p1, &q1, o.vertical());
                    swap_coordinate(&p2, &q2, o.vertical());
                }
            }
        }
    }
    let group = |o| {
        groups
            .iter()
            .find(|(side, _)| *side == o)
            .map(|(_, es)| es.clone())
            .unwrap_or_default()
    };
    let vertical = [group(Side::Top), group(Side::Bottom)].concat();
    let horizontal = [group(Side::Left), group(Side::Right)].concat();
    for a in vertical {
        guard.step()?;
        for &b in &horizontal {
            guard.step()?;
            if g.edge_records[a.0 as usize].route.len() <= 2 {
                break;
            }
            if g.edge_records[b.0 as usize].route.len() <= 2 || !can_swap(g, a, b, node, &ports) {
                continue;
            }
            let [a1, a2, a3] = points(g, a, node);
            let [b1, b2, b3] = points(g, b, node);
            let Some(at) = intersection(a2.get(), a3.get(), b2.get(), b3.get()) else {
                continue;
            };
            let before_a = copy_route(g, a, guard)?;
            let before_b = copy_route(g, b, guard)?;
            swap_points(&a1, &b1);
            swap_points(&a2, &b2);
            insert_intersection(g, a, node, at, &a2, guard)?;
            insert_intersection(g, b, node, at, &b2, guard)?;
            let changed_a = refine(g, node_owner(g, node), a, guard)?;
            let changed_b = refine(g, node_owner(g, node), b, guard)?;
            if !changed_a && !changed_b {
                g.edge_records[a.0 as usize].route.replace(before_a);
                g.edge_records[b.0 as usize].route.replace(before_b);
            } else {
                swapped = true;
            }
        }
    }
    Ok(swapped)
}
pub(super) fn swap_with_context(
    g: &mut ArenaGraph,
    context: Option<&dyn limits::Context>,
    budget: &RouteStageWork,
) -> Result<bool> {
    let mut swapped = false;
    route_stage_guard::run_atomic_route_stage_typed(
        g,
        Some(OwnerGraphId(0)),
        context,
        "SwapEdgePorts",
        &[],
        budget,
        |g, guard| {
            for n in members(g, OwnerGraphId(0)) {
                guard.step()?;
                swapped |= node(g, n, guard)?;
            }
            Ok(())
        },
    )?;
    Ok(swapped)
}
pub(in crate::internal) fn swap_edge_ports_with_budget(
    g: &mut ArenaGraph,
    budget: &RouteStageWork,
) -> Result<()> {
    swap_with_context(g, Some(&RuntimeRouteContext), budget).map(|_| ())
}
pub(in crate::internal) fn swap_edge_ports(g: &mut ArenaGraph) -> bool {
    swap_with_context(
        g,
        Some(&RuntimeRouteContext),
        &RouteStageWork::new(MAX_ROUTE_STAGE_WORK_UNITS),
    )
    .unwrap_or(false)
}

/// The pinned swap_ports_test.go helper applies one node under its owner's
/// complete atomic boundary, including the extra incident-edge references.
#[cfg(test)]
pub(in crate::internal) fn swap_node_for_test(g: &mut ArenaGraph, n: NodeId) -> Result<bool> {
    let receiver = g.topology_node_owner(n).ok_or_else(|| {
        RouteStageError::Validation(limits::Error::Invalid(
            "TALA SwapEdgePorts requires a graph node".into(),
        ))
    })?;
    let extra = g
        .owner_node_incident_header(n)
        .unwrap()
        .iter_slots()
        .collect::<Vec<_>>();
    let mut swapped = false;
    route_stage_guard::run_atomic_route_stage_typed(
        g,
        Some(receiver),
        Some(&RuntimeRouteContext),
        "SwapEdgePorts",
        &extra,
        &RouteStageWork::new(MAX_ROUTE_STAGE_WORK_UNITS),
        |g, guard| {
            swapped = node(g, n, guard)?;
            Ok(())
        },
    )?;
    Ok(swapped)
}

#[cfg(test)]
mod tests {
    use super::*;
    const CONTAINED: &[u8] = br#"{"nodes":[{"id":1,"topLeft":{"x":0,"y":150},"width":200,"height":100},{"id":2,"topLeft":{"x":200,"y":0},"width":50,"height":50},{"id":3,"topLeft":{"x":100,"y":175},"width":50,"height":50}],"edges":[{"id":11,"from":3,"to":2,"points":[{"x":138,"y":175},{"x":138,"y":25},{"x":200,"y":25}]},{"id":12,"from":2,"to":3,"points":[{"x":225,"y":50},{"x":225,"y":75},{"x":113,"y":75},{"x":113,"y":175}]}],"containers":{"0":[1,2],"1":[3]}}"#;
    const SAME_SIDE: &[u8] = br#"{"nodes":[{"id":1,"topLeft":{"x":0,"y":100},"width":100,"height":100},{"id":2,"topLeft":{"x":300,"y":-300},"width":50,"height":50},{"id":3,"topLeft":{"x":-300,"y":-300},"width":50,"height":50}],"edges":[{"id":11,"from":1,"to":2,"points":[{"x":25,"y":100},{"x":25,"y":20},{"x":200,"y":20}]},{"id":12,"from":1,"to":3,"points":[{"x":75,"y":100},{"x":75,"y":50},{"x":-100,"y":50}]}],"containers":{"0":[1,2,3]}}"#;
    const REJECTED: &[u8] = br#"{"nodes":[{"id":1,"topLeft":{"x":0,"y":100},"width":100,"height":100},{"id":2,"topLeft":{"x":300,"y":-300},"width":50,"height":50},{"id":3,"topLeft":{"x":-300,"y":-300},"width":50,"height":50}],"edges":[{"id":11,"from":1,"to":2,"points":[{"x":25,"y":100},{"x":25,"y":20},{"x":200,"y":20}]},{"id":12,"from":1,"to":3,"points":[{"x":100,"y":125},{"x":150,"y":125},{"x":150,"y":-100}]}],"containers":{"0":[1,2,3]}}"#;
    fn fixture(input: &[u8]) -> ArenaGraph {
        ArenaGraph::from_input(&crate::graphjson::graphjson_to_graph(input).unwrap())
    }
    #[test]
    fn same_side_swaps_live_points_without_replacing_storage() {
        let mut g = fixture(SAME_SIDE);
        let retained = g.edge_records[0].route.clone();
        let point = retained.slot(1).unwrap();
        let work = RouteStageWork::new(100000);
        assert!(swap_with_context(&mut g, Some(&RuntimeRouteContext), &work).unwrap());
        assert_eq!(work.used(), 65);
        assert_eq!(point.get().x, 75.0);
        assert!(point.same_allocation(&g.edge_records[0].route.slot(1).unwrap()));
        for i in 0..retained.capacity() {
            assert_eq!(
                retained.header().slot_address(i),
                g.edge_records[0].route.header().slot_address(i)
            );
        }
    }
    #[test]
    fn adjacent_swap_retains_old_points_and_publishes_source_capacity() {
        let mut g = fixture(CONTAINED);
        let old = g.edge_records[0].route.clone();
        let point = old.slot(1).unwrap();
        let work = RouteStageWork::new(100000);
        assert!(swap_with_context(&mut g, Some(&RuntimeRouteContext), &work).unwrap());
        assert_eq!(work.used(), 120);
        assert_eq!((old.len(), old.capacity()), (3, 4));
        assert_eq!(
            (
                g.edge_records[0].route.len(),
                g.edge_records[0].route.capacity()
            ),
            (4, 4)
        );
        assert_eq!(
            (
                g.edge_records[1].route.len(),
                g.edge_records[1].route.capacity()
            ),
            (3, 3)
        );
        assert!(point.same_allocation(&g.edge_records[0].route.slot(2).unwrap()));
        assert_eq!(point.get(), Point { x: 225.0, y: 72.5 });
    }
    #[test]
    fn rejected_adjacent_swap_restores_copies_but_retains_tentative_alias_mutation() {
        let mut g = fixture(REJECTED);
        let old = g.edge_records[0].route.clone();
        let values = old.read();
        let point = old.slot(0).unwrap();
        let work = RouteStageWork::new(100000);
        assert!(!swap_with_context(&mut g, Some(&RuntimeRouteContext), &work).unwrap());
        assert_eq!(work.used(), 84);
        assert_eq!(g.edge_records[0].route.read(), values);
        assert_eq!(g.edge_records[0].route.capacity(), 3);
        assert_eq!(old.capacity(), 4);
        assert!(!point.same_allocation(&g.edge_records[0].route.slot(0).unwrap()));
        assert_eq!(point.get(), Point { x: 100.0, y: 125.0 });
    }
    #[test]
    fn every_contained_swap_work_limit_restores_exact_retained_storage() {
        for limit in 0..120 {
            let mut g = fixture(CONTAINED);
            let headers = g
                .edge_records
                .iter()
                .map(|e| e.route.clone())
                .collect::<Vec<_>>();
            let values = headers.iter().map(|h| h.read()).collect::<Vec<_>>();
            let slots = headers
                .iter()
                .map(|h| {
                    (0..h.capacity())
                        .map(|i| h.header().read_capacity_slot(i).unwrap())
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();
            assert!(
                swap_with_context(
                    &mut g,
                    Some(&RuntimeRouteContext),
                    &RouteStageWork::new(limit)
                )
                .is_err(),
                "limit {limit}"
            );
            for (edge, h) in headers.iter().enumerate() {
                assert_eq!(h.read(), values[edge]);
                assert_eq!(g.edge_records[edge].route.read(), values[edge]);
                for (i, point) in slots[edge].iter().enumerate() {
                    assert_eq!(
                        h.header().slot_address(i),
                        g.edge_records[edge].route.header().slot_address(i)
                    );
                    let current = h.header().read_capacity_slot(i).unwrap();
                    assert!(match (point, current) {
                        (Some(a), Some(b)) => a.same_allocation(&b),
                        (None, None) => true,
                        _ => false,
                    });
                }
            }
        }
    }
}
