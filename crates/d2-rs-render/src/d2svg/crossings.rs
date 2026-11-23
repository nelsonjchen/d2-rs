// SPDX-License-Identifier: MPL-2.0
//! Port of upstream d2target `connection_path.go`, `connection_crossings.go`,
//! `connection_dash.go` and `connection_sketch_crossing.go` (d2lang/d2
//! #3011, "render empty gaps at connection crossings by default").
//!
//! The SVG renderer and the native scene builder share these: both draw the
//! same centerline and cut the same gaps into the back connection's stroke.

use super::geo::{self, Point, go_fmadd, go_fmsub, go_max, go_min, gomath};
use super::svgpath::get_stroke_dash_attributes;
use super::target_methods::arrowhead_dimensions;

/// The connection fields the centerline and crossing search read.
#[derive(Clone, Debug, Default)]
pub struct CrossingConnection<'a> {
    pub route: Vec<(f64, f64)>,
    pub is_curve: bool,
    pub border_radius: f64,
    pub src_arrow: &'a str,
    pub dst_arrow: &'a str,
    pub stroke_width: i64,
    /// StrokeWidth of the source and destination shapes (0 when missing).
    pub src_shape_stroke_width: i64,
    pub dst_shape_stroke_width: i64,
    pub opacity: f64,
    pub stroke: &'a str,
    pub stroke_dash: f64,
    pub animated: bool,
    pub z_index: i64,
}

/// d2target.ConnectionPathCommand: absolute M, L, C or S on the centerline.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PathCommand {
    pub kind: u8,
    pub control1: (f64, f64),
    pub control2: (f64, f64),
    pub end: (f64, f64),
}

/// d2target.ConnectionGap: a transparent opening in one connection stroke.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ConnectionGap {
    pub center: (f64, f64),
    pub radius: f64,
}

const NO_ARROWHEAD: &str = "none";

fn pt(p: (f64, f64)) -> Point {
    Point::new(p.0, p.1)
}

fn tup(p: Point) -> (f64, f64) {
    (p.x, p.y)
}

fn hypot(x: f64, y: f64) -> f64 {
    gomath::hypot(x, y)
}

/// Connection.ArrowheadAdjustments: inset the endpoints for strokes and markers.
pub fn arrowhead_adjustments(c: &CrossingConnection<'_>) -> ((f64, f64), (f64, f64)) {
    let inset = |start: Point, end: Point, arrow: &str, shape_width: i64| {
        let mut distance = (c.stroke_width as f64 + shape_width as f64) / 2.0;
        if arrow != NO_ARROWHEAD {
            distance += c.stroke_width as f64;
        }
        let v = start.vector_to(&end);
        if v.length() == 0.0 {
            return (0.0, 0.0);
        }
        tup(v.unit().multiply(-distance).to_point())
    };
    let route = &c.route;
    let last = route.len() - 1;
    (
        inset(
            pt(route[1]),
            pt(route[0]),
            c.src_arrow,
            c.src_shape_stroke_width,
        ),
        inset(
            pt(route[last - 1]),
            pt(route[last]),
            c.dst_arrow,
            c.dst_shape_stroke_width,
        ),
    )
}

/// Connection.PathCommands: the final routed path, including arrowhead insets
/// and rounded polyline corners. Callers validate the route first.
pub fn path_commands(
    c: &CrossingConnection<'_>,
    src_adj: (f64, f64),
    dst_adj: (f64, f64),
) -> Vec<PathCommand> {
    let route: Vec<Point> = c.route.iter().copied().map(pt).collect();
    let last = route.len() - 1;
    let end = (route[last].x + dst_adj.0, route[last].y + dst_adj.1);
    let command =
        |kind: u8, control1: (f64, f64), control2: (f64, f64), end: (f64, f64)| PathCommand {
            kind,
            control1,
            control2,
            end,
        };
    let zero = (0.0, 0.0);
    let mut out = Vec::with_capacity(route.len() + 1);
    out.push(command(
        b'M',
        zero,
        zero,
        (route[0].x + src_adj.0, route[0].y + src_adj.1),
    ));
    if c.is_curve {
        let mut i = 1;
        while i < route.len() {
            let mut p = tup(route[i + 2]);
            if i + 2 == last {
                p = end;
            }
            out.push(command(b'C', tup(route[i]), tup(route[i + 1]), p));
            i += 3;
        }
        return out;
    }
    let mut i = 1;
    while i < last {
        let (prev, corner, next) = (route[i - 1], route[i], route[i + 1]);
        if prev == corner || corner == next {
            out.push(command(b'L', zero, zero, tup(corner)));
            i += 1;
            continue;
        }
        let units = go_min(
            c.border_radius,
            geo::euclidean_distance(corner.x, corner.y, next.x, next.y) / 2.0,
        );
        let approach = prev.vector_to(&corner).unit().multiply(units).to_point();
        let mut leave = corner.vector_to(&next).unit().multiply(units).to_point();
        let line_end = (corner.x - approach.x, corner.y - approach.y);
        out.push(command(b'L', zero, zero, line_end));
        if units < c.border_radius && i < last - 1 {
            // A short intermediate segment joins two bends in one cubic.
            if next == route[i + 2] {
                i += 1;
                continue;
            }
            i += 1;
            leave = next
                .vector_to(&route[i + 1])
                .unit()
                .multiply(units)
                .to_point();
            out.push(command(
                b'C',
                (corner.x + approach.x, corner.y + approach.y),
                (next.x - leave.x, next.y - leave.y),
                (next.x + leave.x, next.y + leave.y),
            ));
        } else {
            out.push(command(
                b'S',
                line_end,
                tup(corner),
                (corner.x + leave.x, corner.y + leave.y),
            ));
        }
        i += 1;
    }
    out.push(command(b'L', zero, zero, end));
    out
}

/// d2svg.pathData: serializes the same centerline used to find crossings.
pub(crate) fn path_data(commands: &[PathCommand]) -> String {
    let f = super::svgtext::format_float;
    let mut path = String::new();
    let write_point = |path: &mut String, p: (f64, f64)| {
        path.push(' ');
        path.push_str(&f(p.0));
        path.push(' ');
        path.push_str(&f(p.1));
    };
    for cmd in commands {
        if !path.is_empty() {
            path.push(' ');
        }
        path.push(cmd.kind as char);
        if cmd.kind == b'C' {
            write_point(&mut path, cmd.control1);
        }
        if cmd.kind == b'C' || cmd.kind == b'S' {
            write_point(&mut path, cmd.control2);
        }
        write_point(&mut path, cmd.end);
    }
    path
}

#[derive(Clone, Copy, Debug, Default)]
struct DashPattern {
    on: f64,
    period: f64,
    cap: f64,
    slack: f64,
    unknown_phase: bool,
}

#[derive(Clone, Debug, Default)]
struct Route {
    points: Vec<Point>,
    src_marker: f64,
    dst_marker: f64,
    dash: DashPattern,
    folds: bool,
}

#[derive(Clone, Copy, Debug)]
struct Segment {
    edge: usize,
    start: usize,
    /// Centerline length up to the start of this segment.
    distance: f64,
    min_x: f64,
    max_x: f64,
    min_y: f64,
    max_y: f64,
}

impl Segment {
    fn distance_to(&self, p: Point, routes: &[Route]) -> f64 {
        let start = routes[self.edge].points[self.start];
        self.distance + hypot(p.x - start.x, p.y - start.y)
    }
}

#[derive(Clone, Copy, Debug)]
struct LocatedGap {
    index: usize,
    distance: f64,
    max_radius: f64,
    /// Smallest opening that contains a hand-drawn intersection.
    min_radius: f64,
}

// Bound the extra work for unusually large or coincident routes. If a board
// exceeds a limit, render its original paths rather than a partial set of gaps.
const MAX_CROSSING_SEGMENTS: usize = 128 * 1024;
const MAX_CROSSING_COMPARISONS: usize = 4 * 1024 * 1024;
const MAX_CROSSING_GAPS: usize = 64 * 1024;

fn cleared(n: usize) -> Vec<Vec<ConnectionGap>> {
    vec![Vec::new(); n]
}

/// d2target.ConnectionCrossings: one list for each input connection in
/// original diagram order. A higher z-index passes in front; ties use stable
/// paint order.
pub fn connection_crossings(
    connections: &[CrossingConnection<'_>],
    sketch: bool,
    stroke_visible: &mut dyn FnMut(&str) -> bool,
) -> Vec<Vec<ConnectionGap>> {
    let n = connections.len();
    let mut gaps = cleared(n);
    if n < 2 {
        return gaps;
    }
    let mut routes = vec![Route::default(); n];
    let mut locations: Vec<Vec<LocatedGap>> = vec![Vec::new(); n];
    let mut segments: Vec<Segment> = Vec::new();
    let (mut span_x, mut span_y) = (0.0f64, 0.0f64);
    let (mut min_x, mut max_x, mut min_y, mut max_y) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    for (i, c) in connections.iter().enumerate() {
        if c.opacity == 0.0 || c.stroke_width == 0 || !stroke_visible(c.stroke) {
            continue;
        }
        let route = &mut routes[i];
        route.dash = new_dash_pattern(c, sketch);
        let track_slack = route.dash.period != 0.0;
        let (src, dst) = arrowhead_adjustments(c);
        for cmd in path_commands(c, src, dst) {
            if cmd.kind == b'C' || cmd.kind == b'S' {
                let a = *route.points.last().expect("path starts with M");
                let slack = if track_slack {
                    Some(&mut route.dash.slack)
                } else {
                    None
                };
                flatten_cubic(
                    &mut route.points,
                    a,
                    pt(cmd.control1),
                    pt(cmd.control2),
                    pt(cmd.end),
                    0,
                    slack,
                );
            } else if route.points.last() != Some(&pt(cmd.end)) {
                route.points.push(pt(cmd.end));
            }
            if segments.len() + route.points.len() - 1 > MAX_CROSSING_SEGMENTS {
                return gaps;
            }
        }
        route.src_marker = arrowhead_dimensions(c.src_arrow, c.stroke_width as f64).0;
        route.dst_marker = arrowhead_dimensions(c.dst_arrow, c.stroke_width as f64).0;
        let mut distance = 0.0;
        let (first, last) = (route.points[0], route.points[route.points.len() - 1]);
        let (net_x, net_y) = (last.x - first.x, last.y - first.y);
        for j in 1..route.points.len() {
            let (a, b) = (route.points[j - 1], route.points[j]);
            let s = Segment {
                edge: i,
                start: j - 1,
                distance,
                min_x: go_min(a.x, b.x),
                max_x: go_max(a.x, b.x),
                min_y: go_min(a.y, b.y),
                max_y: go_max(a.y, b.y),
            };
            let (dx, dy) = (b.x - a.x, b.y - a.y);
            distance += hypot(dx, dy);
            if (dx != 0.0 && dx * net_x <= 0.0) || (dy != 0.0 && dy * net_y <= 0.0) {
                route.folds = true;
            }
            if segments.is_empty() {
                (min_x, max_x, min_y, max_y) = (s.min_x, s.max_x, s.min_y, s.max_y);
            }
            min_x = go_min(min_x, s.min_x);
            max_x = go_max(max_x, s.max_x);
            min_y = go_min(min_y, s.min_y);
            max_y = go_max(max_y, s.max_y);
            span_x += s.max_x - s.min_x;
            span_y += s.max_y - s.min_y;
            segments.push(s);
        }
    }
    // Sweep along the axis on which segment bounds cover the smaller
    // fraction of the drawing. This also handles stacks of long parallel lines.
    let (extent_x, extent_y) = (max_x - min_x, max_y - min_y);
    if span_x * extent_y > span_y * extent_x {
        for s in &mut segments {
            std::mem::swap(&mut s.min_x, &mut s.min_y);
            std::mem::swap(&mut s.max_x, &mut s.max_y);
        }
    }
    // sort.SliceStable with `less = a.minX < b.minX`.
    segments.sort_by(|a, b| {
        if a.min_x < b.min_x {
            std::cmp::Ordering::Less
        } else if b.min_x < a.min_x {
            std::cmp::Ordering::Greater
        } else {
            std::cmp::Ordering::Equal
        }
    });
    let (mut comparisons, mut count) = (0usize, 0usize);
    for i in 0..segments.len() {
        let a = segments[i];
        for &b in &segments[i + 1..] {
            if b.min_x > a.max_x {
                break;
            }
            comparisons += 1;
            if comparisons > MAX_CROSSING_COMPARISONS || count >= MAX_CROSSING_GAPS {
                return cleared(n);
            }
            if a.edge == b.edge || a.max_y < b.min_y || b.max_y < a.min_y {
                continue;
            }
            let (mut lower, mut upper) = (a.edge, b.edge);
            let (zl, zu) = (connections[lower].z_index, connections[upper].z_index);
            if zl > zu || (zl == zu && lower > upper) {
                std::mem::swap(&mut lower, &mut upper);
            }
            let Some(p) = proper_crossing(
                &routes[a.edge].points,
                a.start,
                &routes[b.edge].points,
                b.start,
            ) else {
                continue;
            };
            let lower_width = connections[lower].stroke_width as f64;
            let widest = connections[lower]
                .stroke_width
                .max(connections[upper].stroke_width);
            let mut radius = go_fmadd(2.0, widest as f64, 2.0);
            if routes[a.edge].touches_endpoint(p, radius)
                || routes[b.edge].touches_endpoint(p, radius)
            {
                continue;
            }
            let (under, over) = if lower == b.edge { (b, a) } else { (a, b) };
            if routes[lower].folds {
                // A mask applies to this entire edge, including any nearby,
                // unrelated fold. Account for the bounded route scan.
                comparisons += routes[lower].points.len();
                if comparisons > MAX_CROSSING_COMPARISONS {
                    return cleared(n);
                }
                radius = routes[lower].clear_of_other_legs(under.start, p, radius, lower_width);
                if radius <= lower_width / 2.0 {
                    continue;
                }
            }
            let mut min_radius = 0.0;
            if sketch {
                min_radius = minimum_sketch_gap_radius(
                    &connections[lower],
                    &connections[upper],
                    under,
                    over,
                    &routes,
                );
                if radius < min_radius {
                    continue;
                }
            }
            let dash = routes[upper].dash;
            if dash.unknown_phase
                || dash.period != 0.0
                    && dash.empty_at_crossing(over, under, p, &routes, lower_width, radius)
            {
                continue;
            }
            locations[lower].push(LocatedGap {
                index: gaps[lower].len(),
                distance: under.distance_to(p, &routes),
                max_radius: radius,
                min_radius,
            });
            gaps[lower].push(ConnectionGap {
                center: (p.x, p.y),
                radius,
            });
            count += 1;
        }
    }
    for (edge, on_route) in locations.into_iter().enumerate() {
        if on_route.len() > 1 {
            let edge_gaps = std::mem::take(&mut gaps[edge]);
            gaps[edge] =
                space_crossing_gaps(edge_gaps, on_route, connections[edge].stroke_width as f64);
        }
    }
    gaps
}

/// Space neighboring openings on the back route so at least one stroke width
/// remains visible between them. Mask elements keep their stable discovery order.
fn space_crossing_gaps(
    mut gaps: Vec<ConnectionGap>,
    mut locations: Vec<LocatedGap>,
    stroke_width: f64,
) -> Vec<ConnectionGap> {
    locations.sort_by(|a, b| {
        if a.distance < b.distance {
            std::cmp::Ordering::Less
        } else if b.distance < a.distance {
            std::cmp::Ordering::Greater
        } else {
            std::cmp::Ordering::Equal
        }
    });
    let mut unique: Vec<LocatedGap> = Vec::with_capacity(locations.len());
    for g in locations {
        // Several foreground edges can meet the back edge at the same point.
        // They need one opening sized for the widest of those strokes.
        if let Some(prev) = unique.last_mut()
            && g.distance - prev.distance < 1e-6
        {
            prev.max_radius = go_max(prev.max_radius, g.max_radius);
            prev.min_radius = go_max(prev.min_radius, g.min_radius);
            gaps[prev.index].radius = prev.max_radius;
            gaps[g.index].radius = 0.0;
            continue;
        }
        unique.push(g);
    }
    for i in 1..unique.len() {
        let (left, right) = (gaps[unique[i - 1].index], gaps[unique[i].index]);
        let room = go_max(
            0.0,
            hypot(
                left.center.0 - right.center.0,
                left.center.1 - right.center.1,
            ) - stroke_width,
        );
        // When one opening is narrow, the other can use the remaining room.
        unique[i - 1].max_radius = go_min(
            unique[i - 1].max_radius,
            room - go_min(room / 2.0, right.radius),
        );
        unique[i].max_radius = go_min(unique[i].max_radius, room - go_min(room / 2.0, left.radius));
    }
    for g in &unique {
        gaps[g.index].radius = g.max_radius;
        if g.max_radius < g.min_radius {
            gaps[g.index].radius = 0.0;
        }
    }
    // A circle that cannot cut across the back stroke would leave a notch.
    gaps.retain(|g| g.radius > stroke_width / 2.0);
    gaps
}

impl Route {
    /// A route monotone in both coordinates passes through a centered disk only
    /// once. For a folding route, limit the radius against any separate visits
    /// after following the crossing's own passage to the boundary on each side.
    fn clear_of_other_legs(&self, segment: usize, center: Point, radius: f64, width: f64) -> f64 {
        const FLATNESS: f64 = 0.125;
        let margin = width / 2.0 + FLATNESS;
        let inside = |p: Point| hypot(p.x - center.x, p.y - center.y) <= radius + margin;
        let (mut lo, mut hi) = (segment, segment + 1);
        while lo > 0 && inside(self.points[lo]) {
            lo -= 1;
        }
        while hi < self.points.len() - 1 && inside(self.points[hi]) {
            hi += 1;
        }
        let mut radius = radius;
        for i in 1..self.points.len() {
            if i > lo && i <= hi {
                continue;
            }
            radius = go_min(
                radius,
                center.distance_to_line(&self.points[i - 1], &self.points[i]) - margin,
            );
        }
        radius
    }

    fn touches_endpoint(&self, p: Point, radius: f64) -> bool {
        let (src, dst) = (self.points[0], self.points[self.points.len() - 1]);
        hypot(src.x - p.x, src.y - p.y) <= radius + self.src_marker
            || hypot(dst.x - p.x, dst.y - p.y) <= radius + self.dst_marker
    }
}

/// The convex hull of a cubic lies within its control points. When each
/// control is within a fraction of a pixel of the finite chord, that chord is
/// sufficient for positioning a gap. Check the finite segment so collinear
/// cubics that double back retain their extrema.
fn flatten_cubic(
    points: &mut Vec<Point>,
    a: Point,
    b: Point,
    c: Point,
    d: Point,
    depth: u32,
    dash_slack: Option<&mut f64>,
) {
    if depth >= 8 || (b.distance_to_line(&a, &d) <= 0.125 && c.distance_to_line(&a, &d) <= 0.125) {
        if let Some(slack) = dash_slack {
            // Chord and control-polygon lengths bound the true arc length.
            // Charge the full uncertainty before comparing a crossing's dash phase.
            let polygon = hypot(a.x - b.x, a.y - b.y)
                + hypot(b.x - c.x, b.y - c.y)
                + hypot(c.x - d.x, c.y - d.y);
            *slack += go_max(0.0, polygon - hypot(a.x - d.x, a.y - d.y));
        }
        if points.last() != Some(&d) {
            points.push(d);
        }
        return;
    }
    let mid = |p: Point, q: Point| Point::new((p.x + q.x) / 2.0, (p.y + q.y) / 2.0);
    let (ab, bc, cd) = (mid(a, b), mid(b, c), mid(c, d));
    let (abc, bcd) = (mid(ab, bc), mid(bc, cd));
    let m = mid(abc, bcd);
    let mut dash_slack = dash_slack;
    flatten_cubic(points, a, ab, abc, m, depth + 1, dash_slack.as_deref_mut());
    flatten_cubic(points, m, bcd, cd, d, depth + 1, dash_slack);
}

fn proper_crossing(a: &[Point], i: usize, b: &[Point], j: usize) -> Option<Point> {
    // Keep subpixel coordinates: flattened curve vertices and adjacent segments
    // must agree about whether the intersection falls on a vertex.
    let (ax, ay) = (a[i + 1].x - a[i].x, a[i + 1].y - a[i].y);
    let (bx, by) = (b[j + 1].x - b[j].x, b[j + 1].y - b[j].y);
    let denom = go_fmsub(ay, bx, ax * by);
    if denom == 0.0 {
        return None;
    }
    let (dx, dy) = (b[j].x - a[i].x, b[j].y - a[i].y);
    let t = go_fmsub(dy, bx, dx * by) / denom;
    let u = go_fmsub(dy, ax, dx * ay) / denom;
    // Half-open segments give a vertex to its outgoing segment exactly once.
    const EPSILON: f64 = 1e-10;
    if t < -EPSILON || t >= 1.0 - EPSILON || u < -EPSILON || u >= 1.0 - EPSILON {
        return None;
    }
    let p = Point::new(go_fmadd(t, ax, a[i].x), go_fmadd(t, ay, a[i].y));
    let (at_a, at_b) = (t <= EPSILON, u <= EPSILON);
    if !at_a && !at_b {
        return Some(p);
    }
    if (at_a && i == 0) || (at_b && j == 0) {
        return None;
    }
    let prev_a = if at_a { a[i - 1] } else { a[i] };
    let prev_b = if at_b { b[j - 1] } else { b[j] };
    // At a bend, a mere touch also intersects a segment. The four incident
    // rays alternate around the point only for a true crossing.
    let angle = |q: Point| gomath::atan2(q.y - p.y, q.x - p.x);
    let base = angle(prev_a);
    let two_pi = 2.0 * std::f64::consts::PI;
    let relative = |q: Point| (angle(q) - base + two_pi) % two_pi;
    let (end_a, first_b, last_b) = (relative(a[i + 1]), relative(prev_b), relative(b[j + 1]));
    const ANGULAR_EPSILON: f64 = 1e-9;
    for v in [first_b, last_b] {
        if v < ANGULAR_EPSILON
            || two_pi - v < ANGULAR_EPSILON
            || (end_a - v).abs() < ANGULAR_EPSILON
        {
            return None;
        }
    }
    if (first_b < end_a) == (last_b < end_a) {
        return None;
    }
    Some(p)
}

/// newCrossingDashPattern. A static dash pattern can leave a crossing
/// entirely free of paint. The renderer draws round caps, so both painted
/// intervals extend half a stroke width beyond their ends.
fn new_dash_pattern(c: &CrossingConnection<'_>, sketch: bool) -> DashPattern {
    if c.animated || c.stroke_dash <= 0.0 {
        return DashPattern::default();
    }
    let mut slack = 0.0;
    if sketch {
        // Rough strokes restart the dash phase on each generated subpath.
        // Only a single straight route has an unambiguous nominal phase.
        if c.route.len() != 2 {
            return DashPattern {
                unknown_phase: true,
                ..DashPattern::default()
            };
        }
        // Allow for endpoint jitter and the bow in very long hand-drawn lines.
        slack = go_fmadd(
            0.01,
            hypot(c.route[1].0 - c.route[0].0, c.route[1].1 - c.route[0].1),
            2.0,
        );
    }
    let width = c.stroke_width as f64;
    let (on, off) = get_stroke_dash_attributes(width, c.stroke_dash);
    DashPattern {
        on,
        period: on + off,
        cap: width / 2.0,
        slack,
        unknown_phase: false,
    }
}

impl DashPattern {
    fn empty_at_crossing(
        &self,
        over: Segment,
        under: Segment,
        p: Point,
        routes: &[Route],
        under_width: f64,
        radius: f64,
    ) -> bool {
        let fore = &routes[over.edge].points[over.start..over.start + 2];
        let back = &routes[under.edge].points[under.start..under.start + 2];
        let dist = |a: Point, b: Point| hypot(a.x - b.x, a.y - b.y);
        let along = dist(fore[0], p);
        let phase = (over.distance + along) % self.period;
        // At a bend, clearing the entire mask disk of painted ink is sufficient.
        let mut clearance = radius + self.cap + 0.5;
        let nearest = go_min(
            go_min(along, dist(fore[1], p)),
            go_min(dist(back[0], p), dist(back[1], p)),
        );
        if nearest > clearance {
            // Away from bends, the distance from a dash end to the back
            // centerline is its along-route distance times sin(crossing angle).
            // A rounded dash cap can reach half a foreground stroke width
            // beyond that end; also include the back stroke and subpixel error.
            let (fx, fy) = (fore[1].x - fore[0].x, fore[1].y - fore[0].y);
            let (bx, by) = (back[1].x - back[0].x, back[1].y - back[0].y);
            let sine = go_fmsub(fy, bx, fx * by).abs() / (hypot(fx, fy) * hypot(bx, by));
            clearance = go_min(clearance, (self.cap + under_width / 2.0 + 0.5) / sine);
        }
        clearance += self.slack;
        phase - self.on > clearance && self.period - phase > clearance
    }
}

/// Rough-go changes its line randomness gain above this length.
const SKETCH_LONG_LINE_LENGTH: f64 = 500.0;

/// minimumSketchGapRadius: a rough-go straight line is painted as two
/// separately jittered cubics. Bound the opening needed to contain the
/// painted intersection.
fn minimum_sketch_gap_radius(
    back: &CrossingConnection<'_>,
    front: &CrossingConnection<'_>,
    under: Segment,
    over: Segment,
    routes: &[Route],
) -> f64 {
    if back.animated && front.animated {
        return 0.0;
    }
    let a = &routes[under.edge].points[under.start..under.start + 2];
    let b = &routes[over.edge].points[over.start..over.start + 2];
    let (mut ax, mut ay) = (a[1].x - a[0].x, a[1].y - a[0].y);
    let (mut bx, mut by) = (b[1].x - b[0].x, b[1].y - b[0].y);
    let (al, bl) = (hypot(ax, ay), hypot(bx, by));
    // On compound paths, the local chord also captures long straight legs.
    // Keep the original placement on ordinary bends and flattened curves.
    if (back.route.len() != 2 || front.route.len() != 2)
        && (back.animated || al <= SKETCH_LONG_LINE_LENGTH)
        && (front.animated || bl <= SKETCH_LONG_LINE_LENGTH)
    {
        return 0.0;
    }
    (ax, ay, bx, by) = (ax / al, ay / al, bx / bl, by / bl);
    let sine = go_fmsub(ay, bx, ax * by).abs();
    let cosine = go_fmadd(ay, by, ax * bx).abs();
    // Each stroke lies in a strip around its centerline. The greatest
    // possible intersection displacement is the distance to a corner of
    // the parallelogram formed by those strips.
    let x = sketch_line_deviation(al, back.animated);
    let y = sketch_line_deviation(bl, front.animated);
    let distance = go_fmadd(2.0 * x * y, cosine, go_fmadd(y, y, x * x)).sqrt() / sine;
    distance + back.stroke_width as f64 / 2.0
}

fn sketch_line_deviation(length: f64, animated: bool) -> f64 {
    if animated {
        return 0.0; // Animated sketch routes use the original centerline.
    }
    // Roughness .5 with rough-go's default randomness offset 2 and bowing 1.
    // Endpoint jitter is at most one unit per axis. Both inner controls can
    // bow by .005 * length; their combined Bernstein weight peaks at .75.
    // Rough-go reduces both contributions to 40% above 500 units.
    let mut bound = go_fmadd(0.75 * 0.005, length, std::f64::consts::SQRT_2);
    if length > SKETCH_LONG_LINE_LENGTH {
        bound *= 0.4;
    }
    bound
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(route: &[(f64, f64)]) -> CrossingConnection<'static> {
        CrossingConnection {
            route: route.to_vec(),
            src_arrow: "none",
            dst_arrow: "triangle",
            stroke_width: 2,
            opacity: 1.0,
            stroke: "#0D32B2",
            ..Default::default()
        }
    }

    #[test]
    fn later_connection_cuts_the_earlier_one() {
        let conns = [
            line(&[(0.0, 50.0), (100.0, 50.0)]),
            line(&[(50.0, 0.0), (50.0, 100.0)]),
        ];
        let gaps = connection_crossings(&conns, false, &mut |_| true);
        assert_eq!(
            gaps,
            vec![
                vec![ConnectionGap {
                    center: (50.0, 50.0),
                    radius: 6.0
                }],
                vec![]
            ]
        );
    }

    #[test]
    fn higher_z_index_passes_in_front() {
        let mut first = line(&[(0.0, 50.0), (100.0, 50.0)]);
        first.z_index = 1;
        let conns = [first, line(&[(50.0, 0.0), (50.0, 100.0)])];
        let gaps = connection_crossings(&conns, false, &mut |_| true);
        assert!(gaps[0].is_empty());
        assert_eq!(gaps[1].len(), 1);
    }

    #[test]
    fn invisible_strokes_and_endpoint_touches_leave_no_gap() {
        let conns = [
            line(&[(0.0, 50.0), (100.0, 50.0)]),
            line(&[(50.0, 0.0), (50.0, 100.0)]),
        ];
        let gaps = connection_crossings(&conns, false, &mut |_| false);
        assert!(gaps.iter().all(Vec::is_empty));
        let near_end = [
            line(&[(0.0, 50.0), (100.0, 50.0)]),
            line(&[(95.0, 0.0), (95.0, 100.0)]),
        ];
        let gaps = connection_crossings(&near_end, false, &mut |_| true);
        assert!(gaps.iter().all(Vec::is_empty));
    }

    #[test]
    fn path_data_matches_the_rounded_corner_serialization() {
        let mut c = line(&[(0.0, 0.0), (0.0, 50.0), (50.0, 50.0)]);
        c.border_radius = 10.0;
        c.dst_arrow = "none";
        let (src, dst) = arrowhead_adjustments(&c);
        assert_eq!(
            path_data(&path_commands(&c, src, dst)),
            "M 0 1 L 0 40 S 0 50 10 50 L 49 50"
        );
    }
}
