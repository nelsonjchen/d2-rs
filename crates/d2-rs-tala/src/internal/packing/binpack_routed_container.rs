// SPDX-License-Identifier: MPL-2.0
//! Original packing/routed_container.go: three-way routed-box safety proof.
use super::*;
use binpack_traversal::PackingNodeRef;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Decision {
    Defer,
    Keep,
    Proposed,
}
#[derive(Debug)]
pub(super) struct Failure {
    pub(super) decision: Decision,
    pub(super) error: limits::Error,
}
impl From<limits::Error> for Failure {
    fn from(error: limits::Error) -> Self {
        Self {
            decision: Decision::Keep,
            error,
        }
    }
}
fn defer(error: limits::Error) -> Failure {
    Failure {
        decision: Decision::Defer,
        error,
    }
}
fn endpoints(g: &ArenaGraph, e: EdgeId) -> (Option<NodeId>, Option<NodeId>) {
    let record = &g.edge_records[e.0 as usize];
    (
        g.owner_edge_endpoint(e, false).unwrap_or(Some(record.from)),
        g.owner_edge_endpoint(e, true).unwrap_or(Some(record.to)),
    )
}
fn box_of(g: &ArenaGraph, n: NodeId) -> Option<Rect> {
    Some(Rect {
        origin: g.position(n)?,
        size: g.nodes[n.0 as usize].rect.size,
    })
}
fn finite(b: Rect) -> bool {
    b.origin.x.is_finite()
        && b.origin.y.is_finite()
        && b.size.width > 0.
        && b.size.height > 0.
        && b.size.width.is_finite()
        && b.size.height.is_finite()
        && (b.origin.x + b.size.width).is_finite()
        && (b.origin.y + b.size.height).is_finite()
}
fn covers(a: Rect, b: Rect) -> bool {
    b.origin.x >= a.origin.x
        && b.origin.y >= a.origin.y
        && b.origin.x + b.size.width <= a.origin.x + a.size.width
        && b.origin.y + b.size.height <= a.origin.y + a.size.height
}
fn contains(b: Rect, p: Point) -> bool {
    p.x >= b.origin.x
        && p.x <= b.origin.x + b.size.width
        && p.y >= b.origin.y
        && p.y <= b.origin.y + b.size.height
}
fn point(g: &ArenaGraph, e: EdgeId, i: usize) -> Option<Point> {
    g.edge_records[e.0 as usize].route.slot(i).map(|p| p.get())
}
fn point_finite(p: Option<Point>) -> bool {
    p.is_some_and(|p| p.x.is_finite() && p.y.is_finite())
}
fn sides(b: Rect, p: Option<Point>) -> u8 {
    if !finite(b) || !point_finite(p) {
        return 0;
    }
    let p = p.unwrap();
    let mut s = 0;
    if p.y == b.origin.y && p.x >= b.origin.x && p.x <= b.origin.x + b.size.width {
        s |= 1
    }
    if p.x == b.origin.x && p.y >= b.origin.y && p.y <= b.origin.y + b.size.height {
        s |= 2
    }
    if p.y == b.origin.y + b.size.height && p.x >= b.origin.x && p.x <= b.origin.x + b.size.width {
        s |= 4
    }
    if p.x == b.origin.x + b.size.width && p.y >= b.origin.y && p.y <= b.origin.y + b.size.height {
        s |= 8
    }
    s
}
fn keeps_sides(a: Rect, b: Rect, p: Option<Point>) -> bool {
    let s = sides(a, p);
    if s == 0 {
        return false;
    }
    !(s & 1 != 0
        && (b.origin.y != a.origin.y
            || b.origin.x != a.origin.x
            || b.origin.x + b.size.width != a.origin.x + a.size.width)
        || s & 2 != 0
            && (b.origin.x != a.origin.x
                || b.origin.y != a.origin.y
                || b.origin.y + b.size.height != a.origin.y + a.size.height)
        || s & 4 != 0
            && (b.origin.y + b.size.height != a.origin.y + a.size.height
                || b.origin.x != a.origin.x
                || b.origin.x + b.size.width != a.origin.x + a.size.width)
        || s & 8 != 0
            && (b.origin.x + b.size.width != a.origin.x + a.size.width
                || b.origin.y != a.origin.y
                || b.origin.y + b.size.height != a.origin.y + a.size.height))
}
pub(super) fn decision(
    graph: Option<&ArenaGraph>,
    root: Option<NodeId>,
    original: Option<Rect>,
    incident: &[Option<EdgeId>],
    guard: &mut limits::WorkGuard<'_>,
) -> Result<Decision, Failure> {
    guard.step().map_err(defer)?;
    let (Some(g), Some(root)) = (graph, root) else {
        return Ok(Decision::Keep);
    };
    if g.shared_topology_initialized() && g.topology_node_owner(root) != Some(OwnerGraphId(0)) {
        return Ok(Decision::Keep);
    }
    if incident.is_empty() {
        guard.finish().map_err(defer)?;
        return Ok(Decision::Defer);
    }
    let (Some(proposed), Some(original)) = (box_of(g, root), original) else {
        return Ok(Decision::Keep);
    };
    let descendants = g
        .packing_descendants_checked(PackingNodeRef::Node(root), guard)
        .map_err(defer)?;
    let mut descendant_set = BTreeSet::new();
    for n in descendants {
        guard.step().map_err(defer)?;
        descendant_set.insert(n);
    }
    let mut affected = Vec::new();
    let mut affected_set = BTreeSet::new();
    let graph_edges = g
        .owner_graph_edge_header(OwnerGraphId(0))
        .map(|h| h.iter_slots().collect::<Vec<_>>())
        .unwrap_or_else(|_| g.edge_order.read().iter().copied().map(Some).collect());
    for edge in graph_edges {
        guard.step().map_err(defer)?;
        let Some(edge) = edge else {
            return Ok(Decision::Keep);
        };
        let (Some(from), Some(to)) = endpoints(g, edge) else {
            return Ok(Decision::Keep);
        };
        if from != root
            && to != root
            && !descendant_set.contains(&PackingNodeRef::Node(from))
            && !descendant_set.contains(&PackingNodeRef::Node(to))
        {
            continue;
        }
        if !affected_set.insert(edge) {
            return Ok(Decision::Keep);
        }
        affected.push(edge);
    }
    let mut graph_incident = BTreeSet::new();
    for &edge in incident {
        guard.step()?;
        let Some(edge) = edge else {
            return Ok(Decision::Keep);
        };
        let (from, to) = endpoints(g, edge);
        if from != Some(root) && to != Some(root)
            || !graph_incident.insert(edge)
            || !affected_set.contains(&edge)
        {
            return Ok(Decision::Keep);
        }
    }
    let node = &g.nodes[root.0 as usize];
    if !node.is_container
        || !node.shape.can_contain()
        || !matches!(
            node.shape,
            ShapeKind::Rectangle | ShapeKind::Square | ShapeKind::RealSquare
        )
    {
        return Ok(Decision::Keep);
    }
    let (dx, dy) = bounds::modifier_element_adjustments(node);
    if dx != 0. || dy != 0. {
        return Ok(Decision::Keep);
    }
    if !finite(proposed) || !finite(original) || !covers(original, proposed) {
        return Ok(Decision::Keep);
    }
    if node.fixed_top_left.is_some() && proposed.origin != original.origin {
        return Ok(Decision::Keep);
    }
    let roots = g
        .owner_node_incident_slots(root)
        .unwrap_or_else(|_| node.edges.read().iter().copied().map(Some).collect());
    let mut root_edges = BTreeSet::new();
    let mut checked = false;
    for edge in roots {
        guard.step()?;
        let Some(edge) = edge else {
            return Ok(Decision::Keep);
        };
        let (Some(from), Some(to)) = endpoints(g, edge) else {
            return Ok(Decision::Keep);
        };
        let len = g.edge_records[edge.0 as usize].route.len();
        if from == to || len < 2 || !graph_incident.contains(&edge) || !root_edges.insert(edge) {
            return Ok(Decision::Keep);
        }
        let mut attached = false;
        if from == root {
            attached = true;
            checked = true;
            guard.step()?;
            if !keeps_sides(original, proposed, point(g, edge, 0)) {
                return Ok(Decision::Keep);
            }
        }
        if to == root {
            attached = true;
            checked = true;
            guard.step()?;
            if !keeps_sides(original, proposed, point(g, edge, len - 1)) {
                return Ok(Decision::Keep);
            }
        }
        if !attached {
            return Ok(Decision::Keep);
        }
        let adjacent = if from == root { to } else { from };
        if g.packing_descendant_checked(
            Some(PackingNodeRef::Node(adjacent)),
            Some(PackingNodeRef::Node(root)),
            true,
            guard,
        )? {
            return Ok(Decision::Keep);
        }
    }
    if root_edges.len() != graph_incident.len() || !checked {
        return Ok(Decision::Keep);
    }
    let preserved = routes_preserved(g, &affected, original, proposed, guard)?;
    guard.finish()?;
    Ok(if preserved {
        Decision::Proposed
    } else {
        Decision::Keep
    })
}
fn routes_preserved(
    g: &ArenaGraph,
    edges: &[EdgeId],
    a: Rect,
    b: Rect,
    guard: &mut limits::WorkGuard<'_>,
) -> Result<bool, limits::Error> {
    for &e in edges {
        guard.step()?;
        let (from, to) = endpoints(g, e);
        let record = &g.edge_records[e.0 as usize];
        let len = record.route.len();
        if from.is_none() || to.is_none() || len < 2 || record.is_curve {
            return Ok(false);
        }
        for i in 0..len {
            guard.step()?;
            if !point_finite(point(g, e, i)) {
                return Ok(false);
            }
        }
        if !rounded_preserved(g, e, a, b, guard)? {
            return Ok(false);
        }
        for i in 0..len - 1 {
            guard.step()?;
            if !segment_preserved(Some(a), Some(b), point(g, e, i), point(g, e, i + 1), guard)? {
                return Ok(false);
            }
        }
    }
    guard.finish().map(|()| true)
}
fn rounded_preserved(
    g: &ArenaGraph,
    e: EdgeId,
    a: Rect,
    b: Rect,
    guard: &mut limits::WorkGuard<'_>,
) -> Result<bool, limits::Error> {
    let record = &g.edge_records[e.0 as usize];
    let radius = match record.style.border_radius.as_deref() {
        None => 10.,
        Some(s) => {
            if s.len() > 64 {
                return Ok(false);
            }
            let Some(v) = d2_rs_core::parse_d2_float(s) else {
                return Ok(false);
            };
            v
        }
    };
    if !radius.is_finite() || radius < 0. {
        return Ok(false);
    }
    for i in 1..record.route.len() - 1 {
        guard.step()?;
        let previous = point(g, e, i - 1).unwrap();
        let corner = point(g, e, i).unwrap();
        let next = point(g, e, i + 1).unwrap();
        let ix = corner.x - previous.x;
        let iy = corner.y - previous.y;
        let ox = next.x - corner.x;
        let oy = next.y - corner.y;
        let il = ix.hypot(iy);
        let ol = ox.hypot(oy);
        if !il.is_finite() || il <= 0. || !ol.is_finite() || ol <= 0. {
            return Ok(false);
        }
        if radius == 0. {
            continue;
        }
        if il < radius || ol / 2. < radius {
            return Ok(false);
        }
        let entry = Point {
            x: corner.x - ix / il * radius,
            y: corner.y - iy / il * radius,
        };
        let exit = Point {
            x: corner.x + ox / ol * radius,
            y: corner.y + oy / ol * radius,
        };
        if !point_finite(Some(entry))
            || !point_finite(Some(exit))
            || !hull_preserved(a, b, &[entry, corner, exit])
        {
            return Ok(false);
        }
    }
    Ok(true)
}
fn hull_preserved(a: Rect, b: Rect, points: &[Point]) -> bool {
    let (mut inside, mut left, mut top, mut right, mut bottom) = (true, true, true, true, true);
    for &p in points {
        if !point_finite(Some(p)) {
            return false;
        }
        inside = inside && contains(b, p);
        left = left && p.x < a.origin.x;
        top = top && p.y < a.origin.y;
        right = right && p.x > a.origin.x + a.size.width;
        bottom = bottom && p.y > a.origin.y + a.size.height;
    }
    inside || left || top || right || bottom
}
pub(super) fn segment_preserved(
    a: Option<Rect>,
    b: Option<Rect>,
    start: Option<Point>,
    end: Option<Point>,
    guard: &mut limits::WorkGuard<'_>,
) -> Result<bool, limits::Error> {
    let (Some(a), Some(b), Some(start), Some(end)) = (a, b, start, end) else {
        return Ok(false);
    };
    if !finite(a)
        || !finite(b)
        || !point_finite(Some(start))
        || !point_finite(Some(end))
        || start == end
    {
        return Ok(false);
    }
    let (enter, exit, intersects, valid) = interval(a, start, end, guard)?;
    if !valid {
        return Ok(false);
    }
    if !intersects {
        return Ok(true);
    }
    let (be, bx, bi, bv) = interval(b, start, end, guard)?;
    Ok(bv && bi && be <= enter && bx >= exit)
}
fn interval(
    b: Rect,
    start: Point,
    end: Point,
    guard: &mut limits::WorkGuard<'_>,
) -> Result<(f64, f64, bool, bool), limits::Error> {
    let dx = end.x - start.x;
    let dy = end.y - start.y;
    if !dx.is_finite() || !dy.is_finite() {
        return Ok((0., 0., false, false));
    }
    let constraints = [
        (-dx, start.x - b.origin.x),
        (dx, b.origin.x + b.size.width - start.x),
        (-dy, start.y - b.origin.y),
        (dy, b.origin.y + b.size.height - start.y),
    ];
    let (mut enter, mut exit) = (0., 1.);
    for (p, q) in constraints {
        guard.step()?;
        if p == 0. {
            if q < 0. {
                return Ok((0., 0., false, true));
            }
            continue;
        }
        let ratio = q / p;
        if !ratio.is_finite() {
            return Ok((0., 0., false, false));
        }
        if p < 0. {
            if ratio > exit {
                return Ok((0., 0., false, true));
            }
            if ratio > enter {
                enter = ratio
            }
        } else {
            if ratio < enter {
                return Ok((0., 0., false, true));
            }
            if ratio < exit {
                exit = ratio
            }
        }
    }
    Ok((enter, exit, true, true))
}
