// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! routing/shortcut_routes.go: bounded, atomic bend reduction.
use super::route_search_guard::{MAX_ROUTE_STAGE_WORK_UNITS, RouteStageWork};
use super::route_stage_guard::{self, RouteAtomicGuard, RouteStageError, RuntimeRouteContext};
use crate::internal::owner_pointer_slices::SharedPointSlice;
use crate::internal::owner_points::SharedOwnerPoint;
use crate::internal::{ArenaGraph, OwnerGraphId, RoutePoints, labels, limits, quality};
use crate::{EdgeId, LabelPosition, NodeId, Point, Rect, ShapeKind};
// Preserve Go math.Min/Max's NaN and signed-zero behavior, including
// direct engine callers whose node geometry is not finite-validated here.
trait GoFloatBounds {
    fn go_min(self, other: Self) -> Self;
    fn go_max(self, other: Self) -> Self;
}
impl GoFloatBounds for f64 {
    fn go_min(self, other: Self) -> Self {
        super::channel_solver::minimum(self, other)
    }
    fn go_max(self, other: Self) -> Self {
        super::channel_solver::maximum(self, other)
    }
}
const EPSILON: f64 = 1e-6;
const MAX_INPUT: usize = 256;
const MAX_ROUTE: usize = 32;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Edge, Graph, Node, Size};

    fn route() -> RoutePoints {
        RoutePoints::make(
            &[
                (20., 10.),
                (80., 10.),
                (80., 70.),
                (160., 70.),
                (160., 10.),
                (220., 10.),
            ]
            .map(|(x, y)| Point { x, y }),
            8,
        )
    }

    #[test]
    fn shortcut_keeps_surviving_point_allocations_and_old_header() {
        let old = route();
        let result = candidate(&old, 0, 4, true);
        assert_eq!((result.len(), result.capacity()), (2, 4));
        assert!(
            result
                .slot(0)
                .unwrap()
                .same_allocation(&old.slot(0).unwrap())
        );
        assert!(
            result
                .slot(1)
                .unwrap()
                .same_allocation(&old.slot(5).unwrap())
        );
        assert_eq!((old.len(), old.capacity()), (6, 8));
        old.edit_point(5, |p| p.x = 221.);
        assert_eq!(result.point(1).x, 221.);
    }

    #[test]
    fn shortcut_rejects_reversal_duplicates_and_small_diagonals() {
        for points in [
            [(0., 0.), (20., 0.), (10., 0.)],
            [(0., 0.), (20., 0.), (20., 0.)],
            [(0., 0.), (20., 0.0000001), (20., 10.)],
        ] {
            let points = RoutePoints::literal(&points.map(|(x, y)| Point { x, y }));
            assert!(!orthogonal(&points));
        }
    }

    #[test]
    fn shortcut_exhaustion_preserves_the_callers_route() {
        let mut input = Graph::default();
        let a = input.add_node(Node::new(
            "a",
            Size {
                width: 20.,
                height: 20.,
            },
        ));
        let b = input.add_node(Node::new(
            "b",
            Size {
                width: 20.,
                height: 20.,
            },
        ));
        input.add_edge(Edge {
            source: a,
            target: b,
        });
        let mut graph = ArenaGraph::from_input(&input);
        graph.set_position(a, Point { x: 0., y: 0. });
        graph.set_position(b, Point { x: 220., y: 0. });
        graph.edge_records[0].route.replace(route());
        let retained = graph.edge_records[0].route.clone();
        let work = RouteStageWork::new(173);
        assert!(
            shortcut_edge_routes_with_budget(&mut graph, &work)
                .unwrap_err()
                .is_route_work_limit()
        );
        assert_eq!(graph.edge_records[0].route.len(), retained.len());
        assert_eq!(
            graph.edge_records[0].route.header().slot_address(0),
            retained.header().slot_address(0)
        );
        assert_eq!(retained.point(1), Point { x: 80., y: 10. });
        shortcut_edge_routes_with_budget(&mut graph, &RouteStageWork::new(1_000_000)).unwrap();
        assert_eq!(graph.edge_records[0].route.len(), 2);
        assert_eq!(retained.len(), 6);
        assert!(
            graph.edge_records[0]
                .route
                .slot(1)
                .unwrap()
                .same_allocation(&retained.slot(5).unwrap())
        );
    }
}

fn nodes(graph: &ArenaGraph) -> impl Iterator<Item = NodeId> + use<> {
    graph
        .owner_graph_view(OwnerGraphId(0))
        .unwrap()
        .nodes
        .require_nodes()
}
fn edges(graph: &ArenaGraph) -> impl Iterator<Item = EdgeId> + use<> {
    graph
        .edge_order
        .header()
        .iter_slots()
        .map(|edge| edge.expect("nil graph edge"))
}
pub(super) fn input_too_large(graph: &ArenaGraph) -> bool {
    if graph.owner_graph_view(OwnerGraphId(0)).unwrap().nodes.len() > MAX_INPUT
        || graph.edge_order.len() > MAX_INPUT
    {
        return true;
    }
    let mut remaining = MAX_INPUT;
    for edge in graph.edge_order.header().iter_slots().flatten() {
        let len = graph.edge_records[edge.0 as usize].route.len();
        if len > remaining {
            return true;
        }
        remaining -= len;
    }
    false
}
fn same_direction(a: Point, b: Point, c: Point, d: Point) -> bool {
    let first = Point {
        x: b.x - a.x,
        y: b.y - a.y,
    };
    let second = Point {
        x: d.x - c.x,
        y: d.y - c.y,
    };
    first.x * second.x + first.y * second.y > 0.0 && first.x * second.y == first.y * second.x
}
fn orthogonal(points: &RoutePoints) -> bool {
    for i in 0..points.len() {
        let Some(p) = points.slot(i).map(|p| p.get()) else {
            return false;
        };
        if !p.x.is_finite() || !p.y.is_finite() {
            return false;
        }
        if i > 0 {
            let a = points.point(i - 1);
            if p == a || p.x != a.x && p.y != a.y {
                return false;
            }
        }
        if i > 1 {
            let a = points.point(i - 2);
            let b = points.point(i - 1);
            if !same_direction(a, b, b, p) && (a.x == p.x || a.y == p.y) {
                return false;
            }
        }
    }
    points.len() >= 2
}
fn length(points: &RoutePoints) -> f64 {
    let mut result = 0.0;
    for i in 1..points.len() {
        let a = points.point(i - 1);
        let b = points.point(i);
        result += (b.x - a.x).abs() + (b.y - a.y).abs();
    }
    result
}
fn bends(points: &RoutePoints) -> usize {
    let mut result = 0;
    for i in 2..points.len() {
        let a = points.point(i - 2);
        let b = points.point(i - 1);
        let c = points.point(i);
        if (b.x - a.x) * (c.y - b.y) != (b.y - a.y) * (c.x - b.x) {
            result += 1;
        }
    }
    result
}
/// Source make(0,len(raw)) retains surviving pointers and the capacity values
/// left by removing same-direction collinear points. Reversals are retained.
pub(super) fn candidate(
    points: &RoutePoints,
    start: usize,
    end: usize,
    horizontal: bool,
) -> RoutePoints {
    let a = points.point(start);
    let b = points.point(end);
    let elbow = if horizontal {
        Point { x: b.x, y: a.y }
    } else {
        Point { x: a.x, y: b.y }
    };
    let mut raw = (0..=start).map(|i| points.slot(i)).collect::<Vec<_>>();
    raw.push(Some(SharedOwnerPoint::new(elbow)));
    raw.extend((end..points.len()).map(|i| points.slot(i)));
    let mut result = SharedPointSlice::with_len_cap(0, raw.len()).unwrap();
    for p in raw {
        let value = p
            .as_ref()
            .expect("runtime error: invalid memory address or nil pointer dereference")
            .get();
        if result.len() > 0 && result.get(result.len() - 1).flatten().unwrap().get() == value {
            continue;
        }
        while result.len() >= 2 {
            let a = result.get(result.len() - 2).flatten().unwrap().get();
            let b = result.get(result.len() - 1).flatten().unwrap().get();
            if !same_direction(a, b, b, value) {
                break;
            }
            result = result
                .reslice_with_capacity(0, result.len() - 1, result.capacity())
                .unwrap();
        }
        result = result.append(&[p]).unwrap();
    }
    RoutePoints::from_header(result)
}
fn coordinate(p: Point, horizontal: bool) -> f64 {
    if horizontal { p.x } else { p.y }
}
fn point_on_segment(p: Point, a: Point, b: Point) -> bool {
    ((a.x == b.x && p.x == a.x) || (a.y == b.y && p.y == a.y))
        && p.x >= a.x.go_min(b.x) - EPSILON
        && p.x <= a.x.go_max(b.x) + EPSILON
        && p.y >= a.y.go_min(b.y) - EPSILON
        && p.y <= a.y.go_max(b.y) + EPSILON
}
fn segments_meet(a: Point, b: Point, c: Point, d: Point) -> bool {
    a.x.go_min(b.x).go_max(c.x.go_min(d.x)) <= a.x.go_max(b.x).go_min(c.x.go_max(d.x))
        && a.y.go_min(b.y).go_max(c.y.go_min(d.y)) <= a.y.go_max(b.y).go_min(c.y.go_max(d.y))
}
fn enters(rect: Rect, a: Point, b: Point) -> bool {
    let left = rect.origin.x + EPSILON;
    let right = rect.right() - EPSILON;
    let top = rect.origin.y + EPSILON;
    let bottom = rect.bottom() - EPSILON;
    if a.y == b.y {
        top < a.y && a.y < bottom && a.x.go_min(b.x) < right && a.x.go_max(b.x) > left
    } else if a.x == b.x {
        left < a.x && a.x < right && a.y.go_min(b.y) < bottom && a.y.go_max(b.y) > top
    } else {
        true
    }
}
pub(super) fn descendant(graph: &ArenaGraph, mut node: Option<NodeId>, ancestor: NodeId) -> bool {
    loop {
        if node == Some(ancestor) {
            return true;
        }
        let Some(current) = node else {
            return false;
        };
        node = quality::owner_ancestry_parent(graph, current);
    }
}
pub(super) fn changed_route_clear(
    graph: &ArenaGraph,
    edge: EdgeId,
    points: &RoutePoints,
    guard: &RouteAtomicGuard<'_>,
) -> Result<bool, RouteStageError> {
    changed_route_clear_between(
        graph,
        edge,
        &graph.edge_records[edge.0 as usize].route,
        points,
        guard,
    )
}
pub(super) fn changed_route_clear_between(
    graph: &ArenaGraph,
    edge: EdgeId,
    before: &RoutePoints,
    points: &RoutePoints,
    guard: &RouteAtomicGuard<'_>,
) -> Result<bool, RouteStageError> {
    let e = &graph.edge_records[edge.0 as usize];
    if points.len() < 2 || before.len() < 2 {
        return Ok(false);
    }
    if points.point(0) != before.point(0)
        || points.point(points.len() - 1) != before.point(before.len() - 1)
        || !same_direction(
            before.point(0),
            before.point(1),
            points.point(0),
            points.point(1),
        )
        || !same_direction(
            before.point(before.len() - 2),
            before.point(before.len() - 1),
            points.point(points.len() - 2),
            points.point(points.len() - 1),
        )
    {
        return Ok(false);
    }
    for i in 1..points.len() {
        let a = points.point(i - 1);
        let b = points.point(i);
        let mut unchanged = false;
        for j in 1..before.len() {
            guard.step()?;
            if a == before.point(j - 1) && b == before.point(j) {
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
        for node in nodes(graph) {
            guard.step()?;
            let n = &graph.nodes[node.0 as usize];
            let Some(origin) = graph.position(node) else {
                continue;
            };
            if n.rect.size.width <= 0.0 || n.rect.size.height <= 0.0 {
                continue;
            }
            if node != e.from
                && node != e.to
                && (descendant(graph, Some(e.from), node) || descendant(graph, Some(e.to), node))
            {
                continue;
            }
            if enters(
                Rect {
                    origin,
                    size: n.rect.size,
                },
                a,
                b,
            ) {
                return Ok(false);
            }
        }
    }
    Ok(true)
}
fn through_rays(
    points: &RoutePoints,
    p: Point,
    guard: &RouteAtomicGuard<'_>,
) -> Result<(bool, bool), RouteStageError> {
    let (mut left, mut right, mut above, mut below) = (false, false, false, false);
    for i in 1..points.len() {
        guard.step()?;
        let a = points.point(i - 1);
        let b = points.point(i);
        if !point_on_segment(p, a, b) {
            continue;
        }
        if a.y == b.y {
            left |= a.x.go_min(b.x) < p.x;
            right |= a.x.go_max(b.x) > p.x;
        } else {
            above |= a.y.go_min(b.y) < p.y;
            below |= a.y.go_max(b.y) > p.y;
        }
    }
    Ok((left && right, above && below))
}
fn contacts_preserved(
    before: &RoutePoints,
    after: &RoutePoints,
    other: &RoutePoints,
    guard: &RouteAtomicGuard<'_>,
) -> Result<bool, RouteStageError> {
    for i in 1..after.len() {
        for j in 1..other.len() {
            guard.step()?;
            let (a, b, c, d) = (
                after.point(i - 1),
                after.point(i),
                other.point(j - 1),
                other.point(j),
            );
            if !segments_meet(a, b, c, d) {
                continue;
            }
            if (a.x == b.x) != (c.x == d.x) {
                let p = if a.y == b.y {
                    Point { x: c.x, y: a.y }
                } else {
                    Point { x: a.x, y: c.y }
                };
                let mut covered = false;
                for k in 1..before.len() {
                    guard.step()?;
                    covered = covered || point_on_segment(p, before.point(k - 1), before.point(k));
                }
                let (old_h, old_v) = through_rays(before, p, guard)?;
                let (new_h, new_v) = through_rays(after, p, guard)?;
                let (other_h, other_v) = through_rays(other, p, guard)?;
                if !covered
                    || ((new_h && other_v) || (new_v && other_h))
                        && !((old_h && other_v) || (old_v && other_h))
                {
                    return Ok(false);
                }
                continue;
            }
            let horizontal = a.y == b.y;
            let position = coordinate(a, !horizontal);
            let lo = coordinate(a, horizontal)
                .go_min(coordinate(b, horizontal))
                .go_max(coordinate(c, horizontal).go_min(coordinate(d, horizontal)));
            let hi = coordinate(a, horizontal)
                .go_max(coordinate(b, horizontal))
                .go_min(coordinate(c, horizontal).go_max(coordinate(d, horizontal)));
            let mut cursor = lo;
            loop {
                let (mut next, mut contains) = (cursor, false);
                for k in 1..before.len() {
                    guard.step()?;
                    let u = before.point(k - 1);
                    let v = before.point(k);
                    if coordinate(u, !horizontal) != position
                        || coordinate(v, !horizontal) != position
                    {
                        continue;
                    }
                    let left = coordinate(u, horizontal).go_min(coordinate(v, horizontal));
                    let right = coordinate(u, horizontal).go_max(coordinate(v, horizontal));
                    if left <= cursor + EPSILON && right >= cursor - EPSILON {
                        contains = true;
                        next = next.go_max(right);
                    }
                }
                if contains && next >= hi - EPSILON {
                    break;
                }
                if next <= cursor + EPSILON {
                    return Ok(false);
                }
                cursor = next;
            }
        }
    }
    Ok(true)
}
fn wall_clearance(
    graph: &ArenaGraph,
    before: &RoutePoints,
    a: Point,
    b: Point,
    guard: &RouteAtomicGuard<'_>,
) -> Result<bool, RouteStageError> {
    let horizontal = a.y == b.y;
    let position = coordinate(a, !horizontal);
    let lo = coordinate(a, horizontal).go_min(coordinate(b, horizontal));
    let hi = coordinate(a, horizontal).go_max(coordinate(b, horizontal));
    for node in nodes(graph) {
        guard.step()?;
        let n = &graph.nodes[node.0 as usize];
        let origin = graph
            .position(node)
            .expect("runtime error: invalid memory address or nil pointer dereference");
        let nlo = coordinate(origin, horizontal);
        let (nhi, wall, span) = if horizontal {
            (nlo + n.rect.size.width, origin.y, n.rect.size.height)
        } else {
            (nlo + n.rect.size.height, origin.x, n.rect.size.width)
        };
        if hi.go_min(nhi) - lo.go_max(nlo) <= EPSILON {
            continue;
        }
        for wall in [wall, wall + span] {
            let gap = (position - wall).abs();
            if gap >= 40.0 - EPSILON {
                continue;
            }
            let mut minimum: f64 = 40.0;
            for j in 1..before.len() {
                guard.step()?;
                let u = before.point(j - 1);
                let v = before.point(j);
                let old_position = coordinate(u, !horizontal);
                if old_position != coordinate(v, !horizontal)
                    || (old_position - wall) * (position - wall) < 0.0
                {
                    continue;
                }
                let old_lo = coordinate(u, horizontal).go_min(coordinate(v, horizontal));
                let old_hi = coordinate(u, horizontal).go_max(coordinate(v, horizontal));
                if old_hi.go_min(nhi) - old_lo.go_max(nlo) > EPSILON {
                    minimum = minimum.go_min((old_position - wall).abs());
                }
            }
            if gap + EPSILON < minimum {
                return Ok(false);
            }
        }
    }
    Ok(true)
}
fn parallel_clearance(
    graph: &ArenaGraph,
    edge: EdgeId,
    a: Point,
    b: Point,
    guard: &RouteAtomicGuard<'_>,
) -> Result<bool, RouteStageError> {
    let horizontal = a.y == b.y;
    let position = coordinate(a, !horizontal);
    let lo = coordinate(a, horizontal).go_min(coordinate(b, horizontal));
    let hi = coordinate(a, horizontal).go_max(coordinate(b, horizontal));
    let e = &graph.edge_records[edge.0 as usize];
    for other_id in edges(graph) {
        if other_id == edge {
            continue;
        }
        let other = &graph.edge_records[other_id.0 as usize];
        for j in 1..other.route.len() {
            guard.step()?;
            let c = other.route.point(j - 1);
            let d = other.route.point(j);
            let wall = coordinate(c, !horizontal);
            if wall != coordinate(d, !horizontal) {
                continue;
            }
            let other_lo = coordinate(c, horizontal).go_min(coordinate(d, horizontal));
            let other_hi = coordinate(c, horizontal).go_max(coordinate(d, horizontal));
            if hi.go_min(other_hi) - lo.go_max(other_lo) <= EPSILON {
                continue;
            }
            let gap = (position - wall).abs();
            if gap <= EPSILON || gap >= 40.0 - EPSILON {
                continue;
            }
            let mut minimum: f64 = 40.0;
            for k in 1..e.route.len() {
                guard.step()?;
                let u = e.route.point(k - 1);
                let v = e.route.point(k);
                let old_position = coordinate(u, !horizontal);
                if old_position != coordinate(v, !horizontal) {
                    continue;
                }
                let old_lo = coordinate(u, horizontal).go_min(coordinate(v, horizontal));
                let old_hi = coordinate(u, horizontal).go_max(coordinate(v, horizontal));
                let old_gap = (old_position - wall).abs();
                if old_gap > EPSILON && old_hi.go_min(other_hi) - old_lo.go_max(other_lo) > EPSILON
                {
                    minimum = minimum.go_min(old_gap);
                }
            }
            if gap + EPSILON < minimum {
                return Ok(false);
            }
        }
    }
    Ok(true)
}
fn candidate_safe(
    graph: &ArenaGraph,
    edge: EdgeId,
    points: &RoutePoints,
    boxes: &[Rect],
    guard: &RouteAtomicGuard<'_>,
) -> Result<bool, RouteStageError> {
    let e = &graph.edge_records[edge.0 as usize];
    if !orthogonal(points)
        || bends(points) >= bends(&e.route)
        || length(points) > length(&e.route) + EPSILON
    {
        return Ok(false);
    }
    if !changed_route_clear(graph, edge, points, guard)? {
        return Ok(false);
    }
    for i in 1..points.len() {
        let a = points.point(i - 1);
        let b = points.point(i);
        let segment_length = (b.x - a.x).abs() + (b.y - a.y).abs();
        let mut minimum: f64 = 40.0;
        let mut unchanged = false;
        for j in 1..e.route.len() {
            guard.step()?;
            unchanged = unchanged || (a == e.route.point(j - 1) && b == e.route.point(j));
        }
        if i == 1 {
            let a = e.route.point(0);
            let b = e.route.point(1);
            minimum = minimum.go_min((b.x - a.x).abs() + (b.y - a.y).abs());
        }
        if i == points.len() - 1 {
            let a = e.route.point(e.route.len() - 2);
            let b = e.route.point(e.route.len() - 1);
            minimum = minimum.go_min((b.x - a.x).abs() + (b.y - a.y).abs());
        }
        if !unchanged && segment_length + EPSILON < minimum {
            return Ok(false);
        }
        for j in i + 2..points.len() {
            guard.step()?;
            if segments_meet(a, b, points.point(j - 1), points.point(j)) {
                return Ok(false);
            }
        }
        if unchanged {
            continue;
        }
        for &rect in boxes {
            guard.step()?;
            if enters(rect, a, b) {
                return Ok(false);
            }
        }
        if !wall_clearance(graph, &e.route, a, b, guard)? {
            return Ok(false);
        }
        if !parallel_clearance(graph, edge, a, b, guard)? {
            return Ok(false);
        }
    }
    for other in edges(graph) {
        if other != edge
            && !contacts_preserved(
                &e.route,
                points,
                &graph.edge_records[other.0 as usize].route,
                guard,
            )?
        {
            return Ok(false);
        }
    }
    Ok(true)
}
fn label_boxes(
    graph: &ArenaGraph,
    guard: &RouteAtomicGuard<'_>,
) -> Result<Vec<Rect>, RouteStageError> {
    let mut boxes = Vec::new();
    let node_count = graph.owner_graph_view(OwnerGraphId(0)).unwrap().nodes.len();
    for node in nodes(graph) {
        guard.add((1 + 4 * node_count) as u64)?;
        let n = &graph.nodes[node.0 as usize];
        if n.is_invisible {
            continue;
        }
        if n.label_position != LabelPosition::Unset
            && let Some(rect) = labels::positioned_node_label_rect(graph, node)
        {
            boxes.push(rect);
        }
        if n.has_icon
            && n.shape != ShapeKind::Image
            && let Some(rect) = labels::positioned_icon_label_rect(graph, node)
        {
            boxes.push(rect);
        }
    }
    for edge in edges(graph) {
        let e = &graph.edge_records[edge.0 as usize];
        guard.add((1 + 3 * e.route.len()) as u64)?;
        if e.is_invisible() || e.route.len() < 2 {
            continue;
        }
        if let Some(label) = &e.label
            && label.position != LabelPosition::Unset
            && let Some(rect) = labels::label_rect(e, label.position, label.percentage)
        {
            boxes.push(rect);
        }
        for target in [false, true] {
            if let Some(rect) = labels::arrowhead_label_rect_for_route(e, &e.route.read(), target) {
                boxes.push(rect);
            }
        }
    }
    Ok(boxes)
}
fn shortcut_guarded(
    graph: &mut ArenaGraph,
    guard: &RouteAtomicGuard<'_>,
) -> Result<(), RouteStageError> {
    let mut boxes = None;
    let mut inventory_checked = false;
    for edge in edges(graph) {
        guard.step()?;
        let e = &graph.edge_records[edge.0 as usize];
        let from_tree = graph
            .owner_graph_node_to_tree_entry(OwnerGraphId(0), Some(e.from))
            .unwrap()
            .is_some();
        let to_tree = graph
            .owner_graph_node_to_tree_entry(OwnerGraphId(0), Some(e.to))
            .unwrap()
            .is_some();
        if e.is_invisible()
            || e.is_curve
            || e.from == e.to
            || from_tree
            || to_tree
            || e.label.is_some()
            || e.source_arrowhead_label.is_some()
            || e.target_arrowhead_label.is_some()
            || e.route.len() < 5
            || e.route.len() > MAX_ROUTE
            || !orthogonal(&e.route)
        {
            continue;
        }
        let mut unsupported = false;
        for other in edges(graph) {
            guard.step()?;
            let other = &graph.edge_records[other.0 as usize];
            unsupported = unsupported || other.is_curve || !orthogonal(&other.route);
        }
        if unsupported {
            return Ok(());
        }
        if boxes.is_none() {
            boxes = Some(label_boxes(graph, guard)?);
        }
        let mut best = e.route.clone();
        let mut best_length = length(&best);
        let before_bends = bends(&best);
        let mut best_bends = before_bends;
        for start in 0..e.route.len() - 3 {
            for end in start + 3..e.route.len() {
                for horizontal in [true, false] {
                    guard.add(e.route.len() as u64)?;
                    let proposal = candidate(&e.route, start, end, horizontal);
                    let candidate_length = length(&proposal);
                    let candidate_bends = bends(&proposal);
                    if candidate_bends >= before_bends
                        || candidate_bends > best_bends
                        || candidate_length > length(&e.route) + EPSILON
                        || (candidate_bends == best_bends
                            && candidate_length >= best_length - EPSILON)
                    {
                        continue;
                    }
                    if candidate_safe(graph, edge, &proposal, boxes.as_ref().unwrap(), guard)? {
                        best = proposal;
                        best_length = candidate_length;
                        best_bends = candidate_bends;
                    }
                }
            }
        }
        if best_bends == before_bends {
            continue;
        }
        if !inventory_checked {
            if !route_stage_guard::capture_inventory_is_closed(graph, OwnerGraphId(0), guard)? {
                return Ok(());
            }
            inventory_checked = true;
        }
        #[cfg(feature = "diagnostic-traces")]
        if crate::internal::trace_env_enabled("WEFTAN_TRACE_SHORTCUT") {
            eprintln!(
                "SHORTCUT_RUST edge={} old_bends={} new_bends={} old={:?} new={:?}",
                edge.0,
                before_bends,
                best_bends,
                graph.edge_records[edge.0 as usize].route.read(),
                best.read()
            );
        }
        let before = quality::inspect_owner(graph, OwnerGraphId(0), guard.context())?;
        let original = graph.edge_records[edge.0 as usize].route.clone();
        graph.edge_records[edge.0 as usize].route.replace(best);
        let after = quality::inspect_owner(graph, OwnerGraphId(0), guard.context())?;
        if after.route_obstructions > before.route_obstructions
            || after.crossings > before.crossings
            || after.text_occlusions > before.text_occlusions
            || after.route_length > before.route_length + EPSILON
        {
            graph.edge_records[edge.0 as usize].route.replace(original);
        }
    }
    Ok(())
}
pub(super) fn shortcut_with_context(
    graph: &mut ArenaGraph,
    context: Option<&dyn limits::Context>,
    budget: &RouteStageWork,
) -> Result<(), RouteStageError> {
    if let Some(context) = context {
        if let Some(cause) = context.error() {
            return Err(limits::Error::Context {
                location: String::new(),
                cause,
            }
            .into());
        }
        if input_too_large(graph) {
            return context.error().map_or(Ok(()), |cause| {
                Err(limits::Error::Context {
                    location: String::new(),
                    cause,
                }
                .into())
            });
        }
    }
    route_stage_guard::run_atomic_route_stage_typed(
        graph,
        Some(OwnerGraphId(0)),
        context,
        "ShortcutEdgeRoutes",
        &[],
        budget,
        shortcut_guarded,
    )
}
pub(in crate::internal) fn shortcut_edge_routes_with_budget(
    graph: &mut ArenaGraph,
    budget: &RouteStageWork,
) -> Result<(), RouteStageError> {
    shortcut_with_context(graph, Some(&RuntimeRouteContext), budget)
}
pub(in crate::internal) fn shortcut_edge_routes(graph: &mut ArenaGraph) {
    let budget = RouteStageWork::new(MAX_ROUTE_STAGE_WORK_UNITS);
    let _ = shortcut_edge_routes_with_budget(graph, &budget);
}
