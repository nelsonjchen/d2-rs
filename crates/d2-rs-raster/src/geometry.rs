// SPDX-License-Identifier: MPL-2.0
//! d2raster/geometry.go: curve flattening in local coordinates, stroke runs
//! and dashes, the stroker (segment rectangles, joins, caps) and pixel bounds.
//! ARM64 contraction sites were checked against the pinned assembly of each
//! enclosing function.

use d2_rs_scene::{endpoint_arc_to_center, go_max, go_min, CenterArc, LineCap, LineJoin, Matrix, Path, PathCommand, PathCommandKind, Point, Rect as SceneRect};

use crate::fma::{finite, finite_point, go_fmadd, go_fmsub, gomath, mmul, mpoint, translate};
use crate::pixels::Rect;
use crate::scanline::Rasterizer;

pub const MAX_CURVE_ERROR_PIXELS: f64 = 0.25;
const MAX_CURVE_DEPTH: i32 = 24;
pub const GEOMETRY_EPSILON: f64 = 1e-12;
const KAPPA: f64 = 0.5522847498307936;

pub type Count<'a> = &'a mut dyn FnMut() -> Result<(), String>;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Subpath {
    pub points: Vec<Point>,
    pub closed: bool,
}

pub type StrokeRun = Subpath;

/// The stroke parameters drawStroke and addStrokeRun need.
#[derive(Clone, Debug)]
pub struct PreparedStroke {
    pub reference_bounds: Rect,
    pub paint: crate::paint::PreparedPaint,
    pub width: f64,
    pub cap: LineCap,
    pub join: LineJoin,
    pub miter_limit: f64,
    pub dashes: Vec<f64>,
    pub dash_offset: f64,
}

pub fn flatten_tolerance(transform: &Matrix) -> f64 {
    let max_coefficient = go_max(go_max(transform.a.abs(), transform.b.abs()), go_max(transform.c.abs(), transform.d.abs()));
    if !finite(max_coefficient) {
        return f64::from_bits(1); // math.SmallestNonzeroFloat64
    }
    if max_coefficient == 0.0 {
        return f64::MAX;
    }
    let factor = Matrix {
        a: transform.a / max_coefficient,
        b: transform.b / max_coefficient,
        c: transform.c / max_coefficient,
        d: transform.d / max_coefficient,
        e: 0.0,
        f: 0.0,
    }
    .max_scale();
    let normalized_tolerance = MAX_CURVE_ERROR_PIXELS / factor;
    if max_coefficient < normalized_tolerance / f64::MAX {
        return f64::MAX;
    }
    let tolerance = normalized_tolerance / max_coefficient;
    if tolerance == 0.0 {
        return f64::from_bits(1);
    }
    tolerance
}

pub fn flatten_scene_path(path: &Path, transform: &Matrix, count: Count<'_>) -> Result<Vec<Subpath>, String> {
    let tolerance = flatten_tolerance(transform);
    let mut result: Vec<Subpath> = Vec::new();
    let mut current = Subpath::default();
    let mut cursor = Point::default();
    let mut start = Point::default();
    let mut have_cursor = false;

    fn flush(result: &mut Vec<Subpath>, current: &mut Subpath, closed: bool) {
        if !current.points.is_empty() {
            current.closed = closed;
            result.push(std::mem::take(current));
        }
        *current = Subpath::default();
    }

    for (i, command) in path.commands.iter().enumerate() {
        let wrap = |e: String| format!("command {i}: {e}");
        match command.kind {
            PathCommandKind::MOVE => {
                flush(&mut result, &mut current, false);
                let point = command.p1;
                if !finite_point(point) {
                    return Err(wrap("non-finite point".into()));
                }
                count().map_err(wrap)?;
                current.points.push(point);
                cursor = point;
                start = point;
                have_cursor = true;
            }
            PathCommandKind::LINE => {
                if !have_cursor {
                    return Err(format!("command {i}: line before move"));
                }
                if current.points.is_empty() {
                    current.points.push(cursor);
                }
                append_unique(&mut current.points, command.p1, count).map_err(wrap)?;
                cursor = command.p1;
            }
            PathCommandKind::QUADRATIC => {
                if !have_cursor {
                    return Err(format!("command {i}: quadratic before move"));
                }
                if !finite_point(command.p1) || !finite_point(command.p2) {
                    return Err(format!("command {i}: non-finite point"));
                }
                if current.points.is_empty() {
                    current.points.push(cursor);
                }
                let points = &mut current.points;
                flatten_quadratic(cursor, command.p1, command.p2, tolerance, 0, &mut |p| append_unique(points, p, count))
                    .map_err(wrap)?;
                cursor = command.p2;
            }
            PathCommandKind::CUBIC => {
                if !have_cursor {
                    return Err(format!("command {i}: cubic before move"));
                }
                if !finite_point(command.p1) || !finite_point(command.p2) || !finite_point(command.p3) {
                    return Err(format!("command {i}: non-finite point"));
                }
                if current.points.is_empty() {
                    current.points.push(cursor);
                }
                let points = &mut current.points;
                flatten_cubic(cursor, command.p1, command.p2, command.p3, tolerance, 0, &mut |p| {
                    append_unique(points, p, count)
                })
                .map_err(wrap)?;
                cursor = command.p3;
            }
            PathCommandKind::ARC => {
                if !have_cursor {
                    return Err(format!("command {i}: arc before move"));
                }
                if !finite_point(command.p1) || !finite(command.radius_x) || !finite(command.radius_y) || !finite(command.rotation) {
                    return Err(format!("command {i}: non-finite arc"));
                }
                if current.points.is_empty() {
                    current.points.push(cursor);
                }
                let points = &mut current.points;
                flatten_arc(cursor, command, transform, MAX_CURVE_ERROR_PIXELS, &mut |p| append_unique(points, p, count))
                    .map_err(wrap)?;
                cursor = command.p1;
            }
            PathCommandKind::CLOSE => {
                if !have_cursor {
                    return Err(format!("command {i}: close before move"));
                }
                count().map_err(wrap)?;
                flush(&mut result, &mut current, true);
                cursor = start;
                have_cursor = true;
            }
            kind => return Err(format!("command {i}: unknown kind {}", kind.0)),
        }
    }
    flush(&mut result, &mut current, false);
    Ok(result)
}

/// The appendPoint closures shared by flattenScenePath and the shape
/// subpaths: validate, charge, then drop exact duplicates.
fn append_unique(points: &mut Vec<Point>, point: Point, count: Count<'_>) -> Result<(), String> {
    if !finite_point(point) {
        return Err("non-finite point".into());
    }
    count()?;
    if points.last() != Some(&point) {
        points.push(point);
    }
    Ok(())
}

fn append_charged(points: &mut Vec<Point>, point: Point, count: Count<'_>) -> Result<(), String> {
    count()?;
    if points.last() != Some(&point) {
        points.push(point);
    }
    Ok(())
}

pub fn rounded_rect_subpaths(rect: &SceneRect, tolerance: f64, count: Count<'_>) -> Result<Vec<Subpath>, String> {
    let b = rect.r#box;
    if b.width == 0.0 || b.height == 0.0 {
        return Ok(Vec::new());
    }
    let rx = go_min(rect.radius_x, b.width / 2.0);
    let ry = go_min(rect.radius_y, b.height / 2.0);
    if rx == 0.0 || ry == 0.0 {
        let points = vec![
            Point::new(b.x, b.y),
            Point::new(b.x + b.width, b.y),
            Point::new(b.x + b.width, b.y + b.height),
            Point::new(b.x, b.y + b.height),
        ];
        for _ in 0..points.len() {
            count()?;
        }
        return Ok(vec![Subpath { points, closed: true }]);
    }
    let mut points = Vec::with_capacity(8);
    append_charged(&mut points, Point::new(b.x + rx, b.y), count)?;
    append_charged(&mut points, Point::new(b.x + b.width - rx, b.y), count)?;
    let k = KAPPA;
    // ARM64 geometry.go:216–219: every `±k*r` term is fused into its sum.
    let right_in = b.x + b.width - rx;
    let bottom_in = b.y + b.height - ry;
    let curves = [
        [
            Point::new(right_in, b.y),
            Point::new(go_fmadd(k, rx, right_in), b.y),
            Point::new(b.x + b.width, go_fmsub(k, ry, b.y + ry)),
            Point::new(b.x + b.width, b.y + ry),
        ],
        [
            Point::new(b.x + b.width, bottom_in),
            Point::new(b.x + b.width, go_fmadd(k, ry, bottom_in)),
            Point::new(go_fmadd(k, rx, right_in), b.y + b.height),
            Point::new(right_in, b.y + b.height),
        ],
        [
            Point::new(b.x + rx, b.y + b.height),
            Point::new(go_fmsub(k, rx, b.x + rx), b.y + b.height),
            Point::new(b.x, go_fmadd(k, ry, bottom_in)),
            Point::new(b.x, bottom_in),
        ],
        [
            Point::new(b.x, b.y + ry),
            Point::new(b.x, go_fmsub(k, ry, b.y + ry)),
            Point::new(go_fmsub(k, rx, b.x + rx), b.y),
            Point::new(b.x + rx, b.y),
        ],
    ];
    let lines = [Point::new(b.x + b.width, bottom_in), Point::new(b.x + rx, b.y + b.height), Point::new(b.x, b.y + ry)];
    for (i, curve) in curves.iter().enumerate() {
        if i != 0 {
            append_charged(&mut points, lines[i - 1], count)?;
        }
        flatten_cubic(curve[0], curve[1], curve[2], curve[3], tolerance, 0, &mut |p| append_charged(&mut points, p, count))?;
    }
    Ok(vec![Subpath { points, closed: true }])
}

pub fn ellipse_subpaths(center: Point, rx: f64, ry: f64, tolerance: f64, count: Count<'_>) -> Result<Vec<Subpath>, String> {
    if rx == 0.0 || ry == 0.0 {
        return Ok(Vec::new());
    }
    let k = KAPPA;
    // ARM64 geometry.go:248–250: every `±k*r` term is fused.
    let (cx, cy) = (center.x, center.y);
    let curves = [
        [Point::new(cx + rx, cy), Point::new(cx + rx, go_fmadd(k, ry, cy)), Point::new(go_fmadd(k, rx, cx), cy + ry), Point::new(cx, cy + ry)],
        [Point::new(cx, cy + ry), Point::new(go_fmsub(k, rx, cx), cy + ry), Point::new(cx - rx, go_fmadd(k, ry, cy)), Point::new(cx - rx, cy)],
        [Point::new(cx - rx, cy), Point::new(cx - rx, go_fmsub(k, ry, cy)), Point::new(go_fmsub(k, rx, cx), cy - ry), Point::new(cx, cy - ry)],
        [Point::new(cx, cy - ry), Point::new(go_fmadd(k, rx, cx), cy - ry), Point::new(cx + rx, go_fmsub(k, ry, cy)), Point::new(cx + rx, cy)],
    ];
    let mut points = Vec::with_capacity(32);
    append_charged(&mut points, curves[0][0], count)?;
    for curve in &curves {
        flatten_cubic(curve[0], curve[1], curve[2], curve[3], tolerance, 0, &mut |p| append_charged(&mut points, p, count))?;
    }
    Ok(vec![Subpath { points, closed: true }])
}

pub type AppendPoint<'a> = &'a mut dyn FnMut(Point) -> Result<(), String>;

pub fn flatten_quadratic(p0: Point, p1: Point, p2: Point, tolerance: f64, depth: i32, append: AppendPoint<'_>) -> Result<(), String> {
    if point_line_distance(p1, p0, p2) <= tolerance && controls_progress(p0, p2, tolerance, &[p1]) {
        return append(p2);
    }
    if depth >= MAX_CURVE_DEPTH {
        return Err("quadratic cannot be flattened within tolerance".into());
    }
    let p01 = midpoint(p0, p1);
    let p12 = midpoint(p1, p2);
    let p012 = midpoint(p01, p12);
    flatten_quadratic(p0, p01, p012, tolerance, depth + 1, append)?;
    flatten_quadratic(p012, p12, p2, tolerance, depth + 1, append)
}

pub fn flatten_cubic(p0: Point, p1: Point, p2: Point, p3: Point, tolerance: f64, depth: i32, append: AppendPoint<'_>) -> Result<(), String> {
    if go_max(point_line_distance(p1, p0, p3), point_line_distance(p2, p0, p3)) <= tolerance
        && controls_progress(p0, p3, tolerance, &[p1, p2])
    {
        return append(p3);
    }
    if depth >= MAX_CURVE_DEPTH {
        return Err("cubic cannot be flattened within tolerance".into());
    }
    let p01 = midpoint(p0, p1);
    let p12 = midpoint(p1, p2);
    let p23 = midpoint(p2, p3);
    let p012 = midpoint(p01, p12);
    let p123 = midpoint(p12, p23);
    let p0123 = midpoint(p012, p123);
    flatten_cubic(p0, p01, p012, p0123, tolerance, depth + 1, append)?;
    flatten_cubic(p0123, p123, p23, p3, tolerance, depth + 1, append)
}

fn flatten_arc(start: Point, command: &PathCommand, transform: &Matrix, tolerance: f64, append: AppendPoint<'_>) -> Result<(), String> {
    if !finite(tolerance) || tolerance <= 0.0 {
        return Err("invalid arc flattening tolerance".into());
    }
    let end = command.p1;
    if start == end {
        return append(end);
    }
    if command.radius_x == 0.0 || command.radius_y == 0.0 {
        return append(end);
    }
    let Some(arc) = endpoint_arc_to_center(start, command)? else {
        return append(end);
    };
    let radius_bound = arc.transformed_radius_bound(*transform);
    if !finite(radius_bound) {
        return Err("transformed arc radius is outside the numeric domain".into());
    }
    flatten_arc_interval(&arc, arc.start_angle(), arc.start_angle() + arc.delta_angle(), end, radius_bound, tolerance, 0, append)
}

#[allow(clippy::too_many_arguments)]
fn flatten_arc_interval(
    arc: &CenterArc,
    theta0: f64,
    theta1: f64,
    end: Point,
    radius_bound: f64,
    tolerance: f64,
    depth: i32,
    append: AppendPoint<'_>,
) -> Result<(), String> {
    let span = (theta1 - theta0).abs();
    let deviation_factor = span * span / 8.0;
    if deviation_factor == 0.0 || radius_bound <= tolerance / deviation_factor {
        return append(end);
    }
    if depth >= MAX_CURVE_DEPTH {
        return Err("arc cannot be flattened within tolerance".into());
    }
    // ARM64 geometry.go:365: the half is exact, so fusion is unobservable.
    let middle_theta = theta0 + (theta1 - theta0) / 2.0;
    let middle = arc.point_at(middle_theta);
    if !finite_point(middle) {
        return Err("arc is outside the numeric domain".into());
    }
    flatten_arc_interval(arc, theta0, middle_theta, middle, radius_bound, tolerance, depth + 1, append)?;
    flatten_arc_interval(arc, middle_theta, theta1, end, radius_bound, tolerance, depth + 1, append)
}

fn point_line_distance(point: Point, start: Point, end: Point) -> f64 {
    let (dx, dy) = (end.x - start.x, end.y - start.y);
    let length = gomath::hypot(dx, dy);
    if length == 0.0 {
        return gomath::hypot(point.x - start.x, point.y - start.y);
    }
    // ARM64 geometry.go:382: FMSUBD of the subtrahend.
    go_fmsub(start.x - point.x, dy, dx * (start.y - point.y)).abs() / length
}

fn controls_progress(start: Point, end: Point, tolerance: f64, controls: &[Point]) -> bool {
    let (dx, dy) = (end.x - start.x, end.y - start.y);
    let length = gomath::hypot(dx, dy);
    if length == 0.0 {
        for control in controls {
            if gomath::hypot(control.x - start.x, control.y - start.y) > tolerance {
                return false;
            }
        }
        return true;
    }
    let (unit_x, unit_y) = (dx / length, dy / length);
    let mut previous = 0.0;
    for control in controls {
        let (control_x, control_y) = (control.x - start.x, control.y - start.y);
        // ARM64 geometry.go:400–401: controlY*unitY is the fused product.
        let projection = go_fmadd(control_y, unit_y, control_x * unit_x);
        let perpendicular = go_fmsub(control_y, unit_x, control_x * unit_y).abs();
        let mut excess = 0.0;
        if projection < 0.0 {
            excess = -projection;
        } else if projection > length {
            excess = projection - length;
        }
        if !finite(projection) || !finite(perpendicular) || gomath::hypot(perpendicular, excess) > tolerance || projection < previous - tolerance {
            return false;
        }
        previous = projection;
    }
    true
}

pub fn midpoint(a: Point, b: Point) -> Point {
    Point::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0)
}

fn interpolate(a: Point, b: Point, t: f64) -> Point {
    // ARM64 geometry.go:429.
    Point::new(go_fmadd(b.x - a.x, t, a.x), go_fmadd(b.y - a.y, t, a.y))
}

pub fn stroke_extent(stroke: &PreparedStroke, transform: &Matrix) -> f64 {
    let expansion = stroke.width * transform.max_scale() / 2.0;
    let mut factor = 1.0;
    if stroke.cap == LineCap::SQUARE {
        factor = std::f64::consts::SQRT_2;
    }
    if stroke.join == LineJoin::MITER && stroke.miter_limit > factor {
        factor = stroke.miter_limit;
    }
    expansion * factor
}

pub fn painted_stroke_pixel_bounds(runs: &[StrokeRun], transform: &Matrix, stroke: &PreparedStroke, canvas: Rect) -> Rect {
    stroke_run_pixel_bounds(runs, transform, stroke_extent(stroke, transform), canvas)
}

pub fn stroke_run_pixel_bounds(runs: &[StrokeRun], transform: &Matrix, expansion: f64, canvas: Rect) -> Rect {
    subpath_pixel_bounds(runs, transform, expansion, canvas)
}

pub fn subpath_pixel_bounds(paths: &[Subpath], transform: &Matrix, expansion: f64, canvas: Rect) -> Rect {
    let (mut min_x, mut min_y) = (f64::INFINITY, f64::INFINITY);
    let (mut max_x, mut max_y) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
    for path in paths {
        for local in &path.points {
            let point = mpoint(transform, *local);
            min_x = go_min(min_x, point.x);
            min_y = go_min(min_y, point.y);
            max_x = go_max(max_x, point.x);
            max_y = go_max(max_y, point.y);
        }
    }
    if min_x == f64::INFINITY {
        return Rect::default();
    }
    let expansion = expansion + 1.0;
    let (mut min_x, mut min_y) = (min_x - expansion, min_y - expansion);
    let (mut max_x, mut max_y) = (max_x + expansion, max_y + expansion);
    if max_x < canvas.min.x as f64 || max_y < canvas.min.y as f64 || min_x > canvas.max.x as f64 || min_y > canvas.max.y as f64 {
        return Rect::default();
    }
    min_x = go_max(min_x, canvas.min.x as f64);
    min_y = go_max(min_y, canvas.min.y as f64);
    max_x = go_min(max_x, canvas.max.x as f64);
    max_y = go_min(max_y, canvas.max.y as f64);
    Rect::new(min_x.floor() as i64, min_y.floor() as i64, max_x.ceil() as i64, max_y.ceil() as i64).intersect(canvas)
}

pub fn make_stroke_runs(path: &Subpath, dashes: &[f64], offset: f64, count: Count<'_>) -> Result<Vec<StrokeRun>, String> {
    let points = clean_points(&path.points, path.closed);
    if points.len() < 2 {
        return Ok(Vec::new());
    }
    if dashes.is_empty() {
        return Ok(vec![StrokeRun { points, closed: path.closed }]);
    }
    let pattern: Vec<f64> = if dashes.len() % 2 != 0 { dashes.iter().chain(dashes.iter()).copied().collect() } else { dashes.to_vec() };
    let mut total = 0.0;
    for length in &pattern {
        total += *length;
    }
    // math.Mod is exact, like Rust's `%` on f64.
    let mut phase = offset % total;
    if phase < 0.0 {
        phase += total;
    }
    let mut pattern_index = 0usize;
    while phase >= pattern[pattern_index] {
        let next_phase = phase - pattern[pattern_index];
        if next_phase == phase {
            return Err("dash offset is too large relative to the dash pattern".into());
        }
        phase = next_phase;
        pattern_index = (pattern_index + 1) % pattern.len();
    }
    let mut remaining = pattern[pattern_index] - phase;
    let mut on = pattern_index % 2 == 0;
    let initial_on = on;
    let mut runs: Vec<StrokeRun> = Vec::new();
    let mut current: Vec<Point> = Vec::new();

    let edge_count = if path.closed { points.len() } else { points.len() - 1 };
    for edge in 0..edge_count {
        let start = points[edge];
        let next_index = if edge + 1 == points.len() { 0 } else { edge + 1 };
        let end = points[next_index];
        let (dx, dy) = (end.x - start.x, end.y - start.y);
        let length = gomath::hypot(dx, dy);
        if length == 0.0 {
            continue;
        }
        let mut position = 0.0;
        while position < length {
            let available = length - position;
            let hit_boundary = remaining <= available;
            let step = if hit_boundary { remaining } else { available };
            let next_position = position + step;
            if next_position <= position {
                return Err("dash pattern is below the path's floating-point resolution".into());
            }
            let a = interpolate(start, end, position / length);
            let b = interpolate(start, end, next_position / length);
            if on {
                if current.is_empty() {
                    count()?;
                    if a == b {
                        current.push(a);
                    } else {
                        count()?;
                        current.push(a);
                        current.push(b);
                    }
                } else {
                    if *current.last().unwrap() != a {
                        count()?;
                        current.push(a);
                    }
                    if *current.last().unwrap() != b {
                        count()?;
                        current.push(b);
                    }
                }
            }
            position = next_position;
            if hit_boundary {
                if on && current.len() >= 2 {
                    runs.push(StrokeRun { points: std::mem::take(&mut current), closed: false });
                }
                pattern_index = (pattern_index + 1) % pattern.len();
                on = pattern_index % 2 == 0;
                remaining = pattern[pattern_index];
            } else {
                remaining -= step;
            }
        }
    }
    if on && current.len() >= 2 {
        runs.push(StrokeRun { points: current, closed: false });
    }
    if !path.closed || runs.is_empty() {
        return Ok(runs);
    }
    let origin = points[0];
    let first_starts_at_origin = runs[0].points[0] == origin;
    let last_ends_at_origin = *runs.last().unwrap().points.last().unwrap() == origin;
    if runs.len() == 1 && first_starts_at_origin && last_ends_at_origin {
        runs[0].points.pop();
        runs[0].closed = true;
        return Ok(runs);
    }
    if initial_on && first_starts_at_origin && last_ends_at_origin && runs.len() > 1 {
        let last = runs.pop().unwrap().points;
        let first = &runs[0].points;
        let mut merged = Vec::with_capacity(last.len() + first.len() - 1);
        merged.extend_from_slice(&last);
        merged.extend_from_slice(&first[1..]);
        runs[0] = StrokeRun { points: merged, closed: false };
    }
    Ok(runs)
}

pub fn add_stroke_run(rasterizer: &mut Rasterizer, run: &StrokeRun, transform: &Matrix, stroke: &PreparedStroke) {
    let points = &run.points;
    let half_width = stroke.width / 2.0;
    if points.len() < 2 || half_width <= 0.0 {
        return;
    }
    let edge_count = if run.closed { points.len() } else { points.len() - 1 };
    for edge in 0..edge_count {
        let mut start = points[edge];
        let next_index = if edge + 1 == points.len() { 0 } else { edge + 1 };
        let mut end = points[next_index];
        let (dx, dy) = (end.x - start.x, end.y - start.y);
        let length = gomath::hypot(dx, dy);
        if length == 0.0 {
            continue;
        }
        let (unit_x, unit_y) = (dx / length, dy / length);
        if !run.closed && stroke.cap == LineCap::SQUARE {
            // ARM64 geometry.go:707–712.
            if edge == 0 {
                start.x = go_fmsub(unit_x, half_width, start.x);
                start.y = go_fmsub(unit_y, half_width, start.y);
            }
            if edge == edge_count - 1 {
                end.x = go_fmadd(unit_x, half_width, end.x);
                end.y = go_fmadd(unit_y, half_width, end.y);
            }
        }
        // ARM64 geometry.go:717–720: each normal offset is fused.
        let (nx, ny) = (-unit_y, unit_x);
        add_polygon(
            rasterizer,
            transform,
            &[
                Point::new(go_fmadd(nx, half_width, start.x), go_fmadd(ny, half_width, start.y)),
                Point::new(go_fmadd(nx, half_width, end.x), go_fmadd(ny, half_width, end.y)),
                Point::new(go_fmsub(nx, half_width, end.x), go_fmsub(ny, half_width, end.y)),
                Point::new(go_fmsub(nx, half_width, start.x), go_fmsub(ny, half_width, start.y)),
            ],
        );
    }

    let (join_start, join_end) = if run.closed { (0, points.len()) } else { (1, points.len() - 1) };
    match stroke.join {
        LineJoin::ROUND => {
            for index in join_start..join_end {
                add_circle(rasterizer, transform, points[index], half_width);
            }
        }
        LineJoin::MITER | LineJoin::BEVEL => {
            let previous_index = if join_start == 0 { points.len() - 1 } else { join_start - 1 };
            let mut incoming = unit_vector(points[previous_index], points[join_start % points.len()]);
            for index in join_start..join_end {
                let next_index = if index + 1 == points.len() { 0 } else { index + 1 };
                let vertex = points[index];
                let outgoing = unit_vector(vertex, points[next_index]);
                let mut polygon = [Point::default(); 4];
                let count = stroke_join_polygon_from_units(&mut polygon, vertex, incoming, outgoing, half_width, stroke.join, stroke.miter_limit);
                if count != 0 {
                    add_polygon(rasterizer, transform, &polygon[..count]);
                }
                incoming = outgoing;
            }
        }
        _ => {}
    }
    if !run.closed && stroke.cap == LineCap::ROUND {
        add_circle(rasterizer, transform, points[0], half_width);
        add_circle(rasterizer, transform, points[points.len() - 1], half_width);
    }
}

/// strokeJoinPolygonFromUnits: the outer wedge between two segment
/// rectangles; returns the point count (0, 3 or 4).
pub fn stroke_join_polygon_from_units(
    destination: &mut [Point; 4],
    vertex: Point,
    incoming: Option<Point>,
    outgoing: Option<Point>,
    half_width: f64,
    join: LineJoin,
    miter_limit: f64,
) -> usize {
    let (Some(incoming), Some(outgoing)) = (incoming, outgoing) else {
        return 0;
    };
    let turn = cross_product(incoming, outgoing);
    if turn.abs() <= GEOMETRY_EPSILON {
        return 0;
    }
    let side = if turn > 0.0 { -1.0 } else { 1.0 };
    // ARM64 geometry.go:787–790: the normals' final products by side are
    // fused into the vertex sums.
    let outer_incoming = Point::new(
        go_fmadd(-(incoming.y * half_width), side, vertex.x),
        go_fmadd(incoming.x * half_width, side, vertex.y),
    );
    let outer_outgoing = Point::new(
        go_fmadd(-(outgoing.y * half_width), side, vertex.x),
        go_fmadd(outgoing.x * half_width, side, vertex.y),
    );
    destination[0] = outer_incoming;
    destination[1] = vertex;
    destination[2] = outer_outgoing;
    if join != LineJoin::MITER {
        return 3;
    }
    let denominator = cross_product(incoming, outgoing);
    let delta = Point::new(outer_outgoing.x - outer_incoming.x, outer_outgoing.y - outer_incoming.y);
    let distance_along_incoming = cross_product(delta, outgoing) / denominator;
    // ARM64 geometry.go:800–801.
    let miter = Point::new(
        go_fmadd(distance_along_incoming, incoming.x, outer_incoming.x),
        go_fmadd(distance_along_incoming, incoming.y, outer_incoming.y),
    );
    let miter_distance = gomath::hypot(miter.x - vertex.x, miter.y - vertex.y);
    let ratio = miter_distance / half_width;
    if !finite_point(miter) || !finite(ratio) || ratio > miter_limit {
        return 3;
    }
    destination[0] = outer_incoming;
    destination[1] = miter;
    destination[2] = outer_outgoing;
    destination[3] = vertex;
    4
}

pub fn unit_vector(start: Point, end: Point) -> Option<Point> {
    let (dx, dy) = (end.x - start.x, end.y - start.y);
    let length = gomath::hypot(dx, dy);
    if !finite(length) || length == 0.0 {
        return None;
    }
    Some(Point::new(dx / length, dy / length))
}

fn cross_product(a: Point, b: Point) -> f64 {
    // ARM64 geometry.go:822: FMSUBD of the subtrahend.
    go_fmsub(a.y, b.x, a.x * b.y)
}

fn add_polygon(rasterizer: &mut Rasterizer, transform: &Matrix, points: &[Point]) {
    if points.len() < 3 {
        return;
    }
    let area = transformed_polygon_signed_area2(points, transform);
    if area == 0.0 || !finite(area) {
        return;
    }
    let want_negative = transform.determinant() >= 0.0;
    let reverse = (area < 0.0) != want_negative;
    let first_index = if reverse { points.len() - 1 } else { 0 };
    let first = mpoint(transform, points[first_index]);
    rasterizer.move_to64(first.x, first.y);
    if reverse {
        for index in (0..points.len() - 1).rev() {
            let point = mpoint(transform, points[index]);
            rasterizer.line_to64(point.x, point.y);
        }
    } else {
        for local in &points[1..] {
            let point = mpoint(transform, *local);
            rasterizer.line_to64(point.x, point.y);
        }
    }
    rasterizer.close_path();
}

fn transformed_polygon_signed_area2(points: &[Point], transform: &Matrix) -> f64 {
    if points.len() < 3 {
        return 0.0;
    }
    let origin = mpoint(transform, points[0]);
    let mut area = 0.0;
    let mut index = 1;
    while index + 1 < points.len() {
        let point_a = mpoint(transform, points[index]);
        let point_b = mpoint(transform, points[index + 1]);
        let a = Point::new(point_a.x - origin.x, point_a.y - origin.y);
        let b = Point::new(point_b.x - origin.x, point_b.y - origin.y);
        area += cross_product(a, b);
        index += 1;
    }
    area
}

fn add_circle(rasterizer: &mut Rasterizer, transform: &Matrix, center: Point, radius: f64) {
    // ARM64 geometry.go:880–893: k is a product, so every `±k` is fused.
    let (cx, cy) = (center.x, center.y);
    let plus = |base: f64| go_fmadd(KAPPA, radius, base);
    let minus = |base: f64| go_fmsub(KAPPA, radius, base);
    let start = mpoint(transform, Point::new(cx + radius, cy));
    rasterizer.move_to64(start.x, start.y);
    let mut cube = |c1: Point, c2: Point, end: Point| {
        let c1 = mpoint(transform, c1);
        let c2 = mpoint(transform, c2);
        let end = mpoint(transform, end);
        rasterizer.cube_to64(c1.x, c1.y, c2.x, c2.y, end.x, end.y);
    };
    cube(Point::new(cx + radius, minus(cy)), Point::new(plus(cx), cy - radius), Point::new(cx, cy - radius));
    cube(Point::new(minus(cx), cy - radius), Point::new(cx - radius, minus(cy)), Point::new(cx - radius, cy));
    cube(Point::new(cx - radius, plus(cy)), Point::new(minus(cx), cy + radius), Point::new(cx, cy + radius));
    cube(Point::new(plus(cx), cy + radius), Point::new(cx + radius, plus(cy)), Point::new(cx + radius, cy));
    rasterizer.close_path();
}

pub fn clean_points(points: &[Point], closed: bool) -> Vec<Point> {
    let mut cleaned: Vec<Point> = Vec::with_capacity(points.len());
    for point in points {
        if cleaned.last() != Some(point) {
            cleaned.push(*point);
        }
    }
    if closed && cleaned.len() > 1 && cleaned[0] == cleaned[cleaned.len() - 1] {
        cleaned.pop();
    }
    cleaned
}

pub fn add_fill_subpath(rasterizer: &mut Rasterizer, path: &Subpath, transform: &Matrix) {
    if path.points.len() < 2 {
        return;
    }
    let first = mpoint(transform, path.points[0]);
    rasterizer.move_to64(first.x, first.y);
    for point in &path.points[1..] {
        let point = mpoint(transform, *point);
        rasterizer.line_to64(point.x, point.y);
    }
    rasterizer.close_path();
}

/// `d2scene.Translate(-x, -y).Mul(transform)`.
pub fn shifted(origin: crate::pixels::Pt, transform: &Matrix) -> Matrix {
    mmul(&translate(-(origin.x as f64), -(origin.y as f64)), transform)
}

pub fn local_object_bounds(paths: &[Subpath]) -> d2_rs_scene::Box {
    let mut bounds = d2_rs_scene::Bounds::default();
    for path in paths {
        for point in &path.points {
            bounds = bounds.include(*point);
        }
    }
    bounds.to_box()
}
