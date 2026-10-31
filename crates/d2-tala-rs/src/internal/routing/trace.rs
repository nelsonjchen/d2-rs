// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Final clipping of route rays to exact rendered shape outlines.
//!
//! Shape-specific intersections replace box/port working endpoints, including
//! D2 rounding behavior and 3D or multiple-object modifiers.

use super::route_search_guard::{MAX_ROUTE_STAGE_WORK_UNITS, RouteStageWork};
use super::route_stage_guard::{self, RouteAtomicGuard, RouteStageError, RuntimeRouteContext};
use super::*;
use crate::EdgeId;
use crate::internal::owner_points::SharedOwnerPoint;
use crate::internal::{OwnerGraphId, bounds::modifier_element_adjustments, limits};

struct TracedPoint {
    point: Point,
    preserve: bool,
    truncate_original: bool,
}
impl TracedPoint {
    fn rounded(closest: Point, truncate_original: bool) -> Self {
        Self {
            point: Point {
                x: (closest.x as f32 as f64).round(),
                y: (closest.y as f32 as f64).round(),
            },
            preserve: false,
            truncate_original,
        }
    }
}

fn line_intersection_point(a: Point, b: Point, c: Point, d: Point) -> Option<Point> {
    d2_rs_core::shape_geometry::line_intersection_point(a, b, c, d)
}

fn extended_shape_ray(previous: Point, border: Point, rect: Rect) -> (Point, Point) {
    let dx = border.x - previous.x;
    let dy = border.y - previous.y;
    let length = dy.mul_add(dy, dx.mul_add(dx, 0.0)).sqrt();
    let extension = if previous.x == border.x {
        rect.size.height
    } else {
        rect.size.width
    };
    // Match geo.Vector.AddLength: Unit() multiplies each component by the
    // reciprocal length first, then the resulting vector is multiplied by
    // (length + extension).  The algebraically equivalent single quotient
    // changes the last bits often enough to move a rounded border point.
    let unit_x = dx * (1.0 / length);
    let unit_y = dy * (1.0 / length);
    let extended_length = length + extension;
    (
        previous,
        Point {
            x: previous.x + unit_x * extended_length,
            y: previous.y + unit_y * extended_length,
        },
    )
}

fn trace_polygon(
    rect: Rect,
    border: Point,
    previous: Point,
    normalized: &[(f64, f64)],
) -> TracedPoint {
    let (ray_start, ray_end) = extended_shape_ray(previous, border, rect);
    let path_coordinate = |value: f64| ((value * 10_000.0) as f32 as f64 / 10_000.0).round();
    let points: Vec<_> = normalized
        .iter()
        .map(|(x, y)| Point {
            x: path_coordinate(rect.origin.x + x * rect.size.width),
            y: path_coordinate(rect.origin.y + y * rect.size.height),
        })
        .collect();
    let mut closest = border;
    let mut closest_distance = f64::INFINITY;
    for index in 0..points.len() {
        let first = points[index];
        let second = points[(index + 1) % points.len()];
        if let Some(intersection) = line_intersection_point(first, second, ray_start, ray_end) {
            if crate::internal::trace_env_enabled("WEFTAN_TRACE_SHAPE_BORDER") {
                eprintln!(
                    "TRACE_RUST_POLYGON rect={:?} border={:?} previous={:?} segment={:?}->{:?} intersection={:?}",
                    rect, border, previous, first, second, intersection
                );
            }
            let distance = euclidean_distance(border, intersection);
            if distance < closest_distance {
                closest = intersection;
                closest_distance = distance;
            }
        }
    }
    TracedPoint::rounded(closest, closest_distance == f64::INFINITY)
}

fn trace_pixel_polygon(
    rect: Rect,
    border: Point,
    previous: Point,
    local_points: &[(f64, f64)],
) -> TracedPoint {
    let (ray_start, ray_end) = extended_shape_ray(previous, border, rect);
    let path_coordinate = |value: f64| ((value * 10_000.0) as f32 as f64 / 10_000.0).round();
    let points: Vec<_> = local_points
        .iter()
        .map(|(x, y)| Point {
            x: path_coordinate(rect.origin.x + x),
            y: path_coordinate(rect.origin.y + y),
        })
        .collect();
    let mut closest = border;
    let mut closest_distance = f64::INFINITY;
    for index in 0..points.len() {
        let first = points[index];
        let second = points[(index + 1) % points.len()];
        if let Some(intersection) = line_intersection_point(first, second, ray_start, ray_end) {
            let distance = euclidean_distance(border, intersection);
            if distance < closest_distance {
                closest = intersection;
                closest_distance = distance;
            }
        }
    }
    TracedPoint::rounded(closest, closest_distance == f64::INFINITY)
}

fn distance_to_segment(point: Point, start: Point, end: Point) -> f64 {
    // Direct translation of geo.Point.DistanceToLine.  D2 calls the helper
    // with a finite ray segment even though the ellipse roots came from the
    // infinite line equation.
    let a = point.x - start.x;
    let b = point.y - start.y;
    let c = end.x - start.x;
    let d = end.y - start.y;
    let dot = b.mul_add(d, a * c);
    let len_sq = d.mul_add(d, c * c);
    let param = if len_sq != 0.0 { dot / len_sq } else { -1.0 };
    let (closest_x, closest_y) = if param < 0.0 {
        (start.x, start.y)
    } else if param > 1.0 {
        (end.x, end.y)
    } else {
        (param.mul_add(c, start.x), param.mul_add(d, start.y))
    };
    let dx = point.x - closest_x;
    let dy = point.y - closest_y;
    dy.mul_add(dy, dx * dx).sqrt()
}

fn ellipse_intersections(
    center: Point,
    radius_x: f64,
    radius_y: f64,
    ray: (Point, Point),
) -> Vec<Point> {
    if radius_x <= 0.0 || radius_y <= 0.0 {
        return Vec::new();
    }
    // This follows lib/geo/ellipse.go rather than solving the segment in
    // parametric form.  The branch and root order are observable when both
    // sides of an ellipse are on the extended segment.
    let a = radius_x;
    let b = radius_y;
    let a2 = a * a;
    let b2 = b * b;
    let x1 = ray.0.x - center.x;
    let y1 = ray.0.y - center.y;
    let x2 = ray.1.x - center.x;
    let y2 = ray.1.y - center.y;

    if x1 == x2 {
        let y = b * (-x1).mul_add(x1, a2).sqrt() / a;
        let first = Point { x: x1, y };
        let second = Point { x: x1, y: -y };
        let on_vertical_segment = |point_y: f64| {
            if y1 <= y2 {
                y1 <= point_y && point_y <= y2
            } else {
                y2 <= point_y && point_y <= y1
            }
        };
        let mut intersections = Vec::new();
        if on_vertical_segment(first.y) {
            intersections.push(Point {
                x: first.x + center.x,
                y: first.y + center.y,
            });
        }
        if second.y != 0.0 && on_vertical_segment(second.y) {
            intersections.push(Point {
                x: second.x + center.x,
                y: second.y + center.y,
            });
        }
        return intersections;
    }

    let on_segment = |point: Point| {
        distance_to_segment(point, Point { x: x1, y: y1 }, Point { x: x2, y: y2 }) < 0.0001
    };
    let slope = (y2 - y1) / (x2 - x1);
    let intercept = (-slope).mul_add(x1, y1);
    let denominator = b.mul_add(b, a2 * slope * slope);
    let root = (a2 * b2 * (-intercept).mul_add(intercept, denominator)).sqrt();
    let first = Point {
        x: (-slope * intercept).mul_add(a2, root) / denominator,
        y: slope.mul_add(root, intercept * b2) / denominator,
    };
    let second = Point {
        x: (-slope * intercept).mul_add(a2, -root) / denominator,
        y: (-slope).mul_add(root, intercept * b2) / denominator,
    };
    let mut intersections = Vec::new();
    if on_segment(first) {
        intersections.push(Point {
            x: first.x + center.x,
            y: first.y + center.y,
        });
    }
    if first != second && on_segment(second) {
        intersections.push(Point {
            x: second.x + center.x,
            y: second.y + center.y,
        });
    }
    intersections
}

fn rounded_result(border: Point, intersections: impl IntoIterator<Item = Point>) -> TracedPoint {
    let mut closest = border;
    let mut closest_distance = f64::INFINITY;
    for point in intersections {
        let distance = euclidean_distance(border, point);
        if distance < closest_distance {
            closest = point;
            closest_distance = distance;
        }
    }
    TracedPoint::rounded(closest, closest_distance == f64::INFINITY)
}

fn trace_ellipse(rect: Rect, border: Point, previous: Point) -> TracedPoint {
    let ray = extended_shape_ray(previous, border, rect);
    let center = Point {
        x: rect.origin.x + rect.size.width * 0.5,
        y: rect.origin.y + rect.size.height * 0.5,
    };
    rounded_result(
        border,
        ellipse_intersections(center, rect.size.width * 0.5, rect.size.height * 0.5, ray),
    )
}

#[derive(Clone, Copy)]
enum ShapePathSegment {
    Line(Point, Point),
    Cubic([Point; 4]),
}

struct ShapePath {
    segments: Vec<ShapePathSegment>,
    top_left: Point,
    scale_x: f64,
    scale_y: f64,
    start: Point,
    current: Point,
}

impl ShapePath {
    fn new(top_left: Point, scale_x: f64, scale_y: f64) -> Self {
        Self {
            segments: Vec::new(),
            top_left,
            scale_x,
            scale_y,
            start: top_left,
            current: top_left,
        }
    }

    /// D2's SVG path builder deliberately reduces each scaled coordinate to
    /// float32 before rounding the resulting coordinate so shape geometry is
    /// stable across architectures.
    fn chop_precision(value: f64) -> f64 {
        ((value * 10_000.0) as f32 as f64 / 10_000.0).round()
    }

    fn absolute(&self, x: f64, y: f64) -> Point {
        Point {
            x: Self::chop_precision(self.top_left.x + self.scale_x * x),
            y: Self::chop_precision(self.top_left.y + self.scale_y * y),
        }
    }

    fn relative(&self, x: f64, y: f64) -> Point {
        Point {
            x: Self::chop_precision(self.current.x + self.scale_x * x),
            y: Self::chop_precision(self.current.y + self.scale_y * y),
        }
    }

    fn start_at(&mut self, point: Point) {
        self.start = point;
        self.current = point;
    }

    fn horizontal(&mut self, relative: bool, x: f64) {
        let mut end = if relative {
            self.relative(x, 0.0)
        } else {
            self.absolute(x, 0.0)
        };
        end.y = self.current.y;
        self.segments
            .push(ShapePathSegment::Line(self.current, end));
        self.current = end;
    }

    fn vertical(&mut self, relative: bool, y: f64) {
        let mut end = if relative {
            self.relative(0.0, y)
        } else {
            self.absolute(0.0, y)
        };
        end.x = self.current.x;
        self.segments
            .push(ShapePathSegment::Line(self.current, end));
        self.current = end;
    }

    fn line(&mut self, relative: bool, x: f64, y: f64) {
        let end = if relative {
            self.relative(x, y)
        } else {
            self.absolute(x, y)
        };
        self.segments
            .push(ShapePathSegment::Line(self.current, end));
        self.current = end;
    }

    fn cubic(&mut self, relative: bool, values: [f64; 6]) {
        let point = |path: &Self, x, y| {
            if relative {
                path.relative(x, y)
            } else {
                path.absolute(x, y)
            }
        };
        let controls = [
            self.current,
            point(self, values[0], values[1]),
            point(self, values[2], values[3]),
            point(self, values[4], values[5]),
        ];
        self.segments.push(ShapePathSegment::Cubic(controls));
        self.current = controls[3];
    }

    fn close(&mut self) {
        self.segments
            .push(ShapePathSegment::Line(self.current, self.start));
        self.current = self.start;
    }
}

fn bezier_coefficients(p0: f64, p1: f64, p2: f64, p3: f64) -> [f64; 4] {
    [
        (-3.0_f64).mul_add(p2, 3.0_f64.mul_add(p1, -p0)) + p3,
        3.0_f64.mul_add(p2, (-6.0_f64).mul_add(p1, 3.0 * p0)),
        3.0_f64.mul_add(p1, -3.0 * p0),
        p0,
    ]
}

fn precision_compare(left: f64, right: f64, precision: f64) -> std::cmp::Ordering {
    if (left - right).abs() < precision {
        std::cmp::Ordering::Equal
    } else if left < right {
        std::cmp::Ordering::Less
    } else {
        std::cmp::Ordering::Greater
    }
}

fn sort_special_roots(mut roots: [f64; 3]) -> [f64; 3] {
    loop {
        let mut flipped = false;
        for index in 0..2 {
            let next_nonnegative = precision_compare(roots[index + 1], 0.0, 0.0001).is_ge();
            let current_greater = precision_compare(roots[index], roots[index + 1], 0.0001).is_gt();
            let current_negative = precision_compare(roots[index], 0.0, 0.0001).is_lt();
            if next_nonnegative && (current_greater || current_negative) {
                roots.swap(index, index + 1);
                flipped = true;
            }
        }
        if !flipped {
            return roots;
        }
    }
}

/// Direct port of the cubic root ordering and precision behavior used by the
/// D2 geometry dependency bundled with TALA.
fn cubic_roots(polynomial: [f64; 4]) -> [f64; 3] {
    const PRECISION: f64 = 0.0001;
    if precision_compare(polynomial[0], 0.0, PRECISION).is_eq() {
        if precision_compare(polynomial[1], 0.0, PRECISION).is_eq() {
            let mut roots = [-(polynomial[3] / polynomial[2]), -1.0, -1.0];
            if precision_compare(roots[0], 0.0, PRECISION).is_lt()
                || precision_compare(roots[0], 1.0, PRECISION).is_gt()
            {
                roots[0] = -1.0;
            }
            return sort_special_roots(roots);
        }

        let mut discriminant = (-4.0 * polynomial[1]).mul_add(polynomial[3], polynomial[2].powi(2));
        if precision_compare(discriminant, 0.0, PRECISION).is_ge() {
            discriminant = discriminant.sqrt();
            let mut roots = [
                -(discriminant + polynomial[2]) / (2.0 * polynomial[1]),
                (discriminant - polynomial[2]) / (2.0 * polynomial[1]),
                -1.0,
            ];
            // Source returns from inside this loop after validating only root zero.
            for root in roots.iter_mut().take(1) {
                if precision_compare(*root, 0.0, PRECISION).is_lt()
                    || precision_compare(*root, 1.0, PRECISION).is_gt()
                {
                    *root = -1.0;
                }
            }
            return sort_special_roots(roots);
        }
    }

    let a = polynomial[1] / polynomial[0];
    let b = polynomial[2] / polynomial[0];
    let c = polynomial[3] / polynomial[0];
    let q = 3.0_f64.mul_add(b, -a.powi(2)) / 9.0;
    let r = ((-27.0_f64).mul_add(c, 9.0 * a * b) - 2.0 * a.powi(3)) / 54.0;
    let discriminant = q.powi(3) + r.powi(2);
    let mut roots;
    if precision_compare(discriminant, 0.0, PRECISION).is_ge() {
        let signed_cube_root =
            |value: f64| (if value < 0.0 { -1.0 } else { 1.0 }) * value.abs().powf(1.0 / 3.0);
        let s = signed_cube_root(r + discriminant.sqrt());
        let t = signed_cube_root(r - discriminant.sqrt());
        roots = [
            -a / 3.0 + (s + t),
            (-0.5_f64).mul_add(s + t, -a / 3.0),
            (-0.5_f64).mul_add(s + t, -a / 3.0),
        ];
        let imaginary = (3.0_f64).sqrt() * (s - t).abs() / 2.0;
        if !precision_compare(imaginary, 0.0, PRECISION).is_eq() {
            roots[1] = -1.0;
            roots[2] = -1.0;
        }
    } else {
        let theta = (r / (-q.powi(3)).sqrt()).acos();
        roots = [
            2.0 * (-q).sqrt() * (theta / 3.0).cos() - a / 3.0,
            2.0 * (-q).sqrt() * ((theta + 2.0 * std::f64::consts::PI) / 3.0).cos() - a / 3.0,
            2.0 * (-q).sqrt() * ((theta + 4.0 * std::f64::consts::PI) / 3.0).cos() - a / 3.0,
        ];
    }
    for root in &mut roots {
        if precision_compare(*root, 0.0, PRECISION).is_lt()
            || precision_compare(*root, 1.0, PRECISION).is_gt()
        {
            *root = -1.0;
        }
    }
    sort_special_roots(roots)
}

fn cubic_line_intersections(curve: [Point; 4], line: (Point, Point)) -> Vec<Point> {
    const PRECISION: f64 = 0.0001;
    let line_a = line.1.y - line.0.y;
    let line_b = line.0.x - line.1.x;
    let line_c = line
        .0
        .y
        .mul_add(line.1.x - line.0.x, line.0.x * (line.0.y - line.1.y));
    let x = bezier_coefficients(curve[0].x, curve[1].x, curve[2].x, curve[3].x);
    let y = bezier_coefficients(curve[0].y, curve[1].y, curve[2].y, curve[3].y);
    let roots = cubic_roots([
        line_a.mul_add(x[0], line_b * y[0]),
        line_a.mul_add(x[1], line_b * y[1]),
        line_a.mul_add(x[2], line_b * y[2]),
        line_a.mul_add(x[3], line_b * y[3]) + line_c,
    ]);
    let mut intersections = Vec::new();
    for parameter in roots {
        let point = Point {
            x: x[2].mul_add(
                parameter,
                (x[0] * parameter * parameter).mul_add(parameter, x[1] * parameter * parameter),
            ) + x[3],
            y: y[2].mul_add(
                parameter,
                (y[0] * parameter * parameter).mul_add(parameter, y[1] * parameter * parameter),
            ) + y[3],
        };
        let line_parameter = if line.1.x != line.0.x {
            (point.x - line.0.x) / (line.1.x - line.0.x)
        } else {
            (point.y - line.0.y) / (line.1.y - line.0.y)
        };
        if !precision_compare(parameter, 0.0, PRECISION).is_lt()
            && !precision_compare(parameter, 1.0, PRECISION).is_gt()
            && !precision_compare(line_parameter, 0.0, PRECISION).is_lt()
            && !precision_compare(line_parameter, 1.0, PRECISION).is_gt()
        {
            intersections.push(point);
        }
    }
    intersections
}

fn shape_path_intersections(path: ShapePath, ray: (Point, Point)) -> Vec<Point> {
    let mut intersections = Vec::new();
    for segment in path.segments {
        intersections.extend(match segment {
            ShapePathSegment::Line(start, end) => line_intersection_point(start, end, ray.0, ray.1)
                .into_iter()
                .collect(),
            ShapePathSegment::Cubic(curve) => cubic_line_intersections(curve, ray),
        });
    }
    intersections
}

fn trace_shape_path(rect: Rect, border: Point, previous: Point, path: ShapePath) -> TracedPoint {
    let ray = extended_shape_ray(previous, border, rect);
    rounded_result(border, shape_path_intersections(path, ray))
}

/// Direct port of D2's `shapeDiamond.diamondPath`.
fn diamond_path(rect: Rect) -> ShapePath {
    let mut path = ShapePath::new(rect.origin, rect.size.width / 77.0, rect.size.height / 76.9);
    path.start_at(path.absolute(38.5, 76.9));
    path.cubic(true, [-0.3, 0.0, -0.5, -0.1, -0.7, -0.3]);
    path.line(false, 0.3, 39.2);
    path.cubic(true, [-0.4, -0.4, -0.4, -1.0, 0.0, -1.4]);
    path.line(false, 37.8, 0.3);
    path.cubic(true, [0.4, -0.4, 1.0, -0.4, 1.4, 0.0]);
    path.line(true, 37.5, 37.5);
    path.cubic(true, [0.4, 0.4, 0.4, 1.0, 0.0, 1.4]);
    path.line(false, 39.2, 76.6);
    path.cubic(false, [39.0, 76.8, 38.8, 76.9, 38.5, 76.9]);
    path.close();
    path
}

/// Direct port of D2's `shapeCallout.calloutPath`.
fn callout_path(rect: Rect) -> ShapePath {
    let tip_width = if rect.size.width < 60.0 {
        rect.size.width / 2.0
    } else {
        30.0
    };
    let tip_height = if rect.size.height < 90.0 {
        rect.size.height / 2.0
    } else {
        45.0
    };
    let body_height = rect.size.height - tip_height;
    let mut path = ShapePath::new(rect.origin, 1.0, 1.0);
    path.start_at(path.absolute(0.0, 0.0));
    path.vertical(true, body_height);
    path.horizontal(true, rect.size.width / 2.0);
    path.vertical(true, tip_height);
    path.line(true, tip_width, -tip_height);
    path.horizontal(true, rect.size.width / 2.0 - tip_width);
    path.vertical(true, -body_height);
    path.horizontal(true, -rect.size.width);
    path.close();
    path
}

fn queue_path(rect: Rect) -> ShapePath {
    let arc = 24.0_f64.min(rect.size.width * 0.5);
    let mut path = ShapePath::new(rect.origin, 1.0, 1.0);
    path.start_at(path.absolute(arc, 0.0));
    path.horizontal(true, rect.size.width - 2.0 * arc);
    path.cubic(
        false,
        [
            rect.size.width,
            0.0,
            rect.size.width,
            rect.size.height * 0.45,
            rect.size.width,
            rect.size.height * 0.5,
        ],
    );
    path.cubic(
        false,
        [
            rect.size.width,
            rect.size.height * 0.55,
            rect.size.width,
            rect.size.height,
            rect.size.width - arc,
            rect.size.height,
        ],
    );
    path.horizontal(true, -(rect.size.width - 2.0 * arc));
    path.cubic(
        false,
        [
            0.0,
            rect.size.height,
            0.0,
            rect.size.height * 0.55,
            0.0,
            rect.size.height * 0.5,
        ],
    );
    path.cubic(false, [0.0, rect.size.height * 0.45, 0.0, 0.0, arc, 0.0]);
    path.close();
    path
}

fn cylinder_path(rect: Rect) -> ShapePath {
    let arc = 24.0_f64.min(rect.size.height * 0.5);
    let mut path = ShapePath::new(rect.origin, 1.0, 1.0);
    path.start_at(path.absolute(0.0, arc));
    path.cubic(
        false,
        [
            0.0,
            0.0,
            rect.size.width * 0.45,
            0.0,
            rect.size.width * 0.5,
            0.0,
        ],
    );
    path.cubic(
        false,
        [
            rect.size.width * 0.55,
            0.0,
            rect.size.width,
            0.0,
            rect.size.width,
            arc,
        ],
    );
    path.vertical(true, rect.size.height - 2.0 * arc);
    path.cubic(
        false,
        [
            rect.size.width,
            rect.size.height,
            rect.size.width * 0.55,
            rect.size.height,
            rect.size.width * 0.5,
            rect.size.height,
        ],
    );
    path.cubic(
        false,
        [
            rect.size.width * 0.45,
            rect.size.height,
            0.0,
            rect.size.height,
            0.0,
            rect.size.height - arc,
        ],
    );
    path.vertical(true, -(rect.size.height - 2.0 * arc));
    path.close();
    path
}

/// Exact `storedDataPath` from the D2 shape library linked into TALA.
fn stored_data_path(rect: Rect) -> ShapePath {
    let mut wedge_width = 15.0;
    const MULTIPLIER: f64 = 0.27;
    if rect.size.width < wedge_width * 2.0 {
        wedge_width = rect.size.width * 0.5;
    }

    let mut path = ShapePath::new(rect.origin, 1.0, 1.0);
    path.start_at(path.absolute(wedge_width, 0.0));
    path.horizontal(true, rect.size.width - wedge_width);
    path.cubic(
        false,
        [
            rect.size.width - wedge_width * MULTIPLIER,
            0.0,
            rect.size.width - wedge_width,
            rect.size.height * MULTIPLIER,
            rect.size.width - wedge_width,
            rect.size.height * 0.5,
        ],
    );
    path.cubic(
        false,
        [
            rect.size.width - wedge_width,
            rect.size.height - rect.size.height * MULTIPLIER,
            rect.size.width - wedge_width * MULTIPLIER,
            rect.size.height,
            rect.size.width,
            rect.size.height,
        ],
    );
    path.horizontal(true, -(rect.size.width - wedge_width));
    path.cubic(
        false,
        [
            wedge_width - wedge_width * MULTIPLIER,
            rect.size.height,
            0.0,
            rect.size.height - rect.size.height * MULTIPLIER,
            0.0,
            rect.size.height * 0.5,
        ],
    );
    path.cubic(
        false,
        [
            0.0,
            rect.size.height * MULTIPLIER,
            wedge_width - wedge_width * MULTIPLIER,
            0.0,
            wedge_width,
            0.0,
        ],
    );
    path.close();
    path
}

/// Exact outer page perimeter from the D2 shape library linked into TALA.
fn page_outer_path(rect: Rect) -> ShapePath {
    let mut path = ShapePath::new(rect.origin, 1.0, 1.0);
    path.start_at(path.absolute(0.5, 0.0));
    path.horizontal(false, rect.size.width - 20.8164);
    path.cubic(
        false,
        [
            rect.size.width - 19.6456,
            0.0,
            rect.size.width - 18.521,
            0.456297,
            rect.size.width - 17.6811,
            1.27202,
        ],
    );
    path.line(false, rect.size.width - 1.3647, 17.12);
    path.cubic(
        false,
        [
            rect.size.width - 0.4923,
            17.9674,
            rect.size.width,
            19.1318,
            rect.size.width,
            20.348,
        ],
    );
    path.vertical(false, rect.size.height - 0.5);
    path.cubic(
        false,
        [
            rect.size.width,
            rect.size.height - 0.2239,
            rect.size.width - 0.2239,
            rect.size.height,
            rect.size.width - 0.5,
            rect.size.height,
        ],
    );
    path.horizontal(false, 0.499999);
    path.cubic(
        false,
        [
            0.223857,
            rect.size.height,
            0.0,
            rect.size.height - 0.2239,
            0.0,
            rect.size.height - 0.5,
        ],
    );
    path.vertical(false, 0.499999);
    path.cubic(false, [0.0, 0.223857, 0.223857, 0.0, 0.5, 0.0]);
    path.close();
    path
}

/// Exact document perimeter from the D2 shape library linked into TALA.
fn document_path(rect: Rect) -> ShapePath {
    const PATH_HEIGHT: f64 = 18.925;
    const PATH_BOTTOM: f64 = 16.3;

    let mut path = ShapePath::new(rect.origin, rect.size.width, rect.size.height);
    path.start_at(path.absolute(0.0, PATH_BOTTOM / PATH_HEIGHT));
    path.line(false, 0.0, 0.0);
    path.line(false, 1.0, 0.0);
    path.line(false, 1.0, PATH_BOTTOM / PATH_HEIGHT);
    path.cubic(
        false,
        [
            5.0 / 6.0,
            12.8 / PATH_HEIGHT,
            2.0 / 3.0,
            12.8 / PATH_HEIGHT,
            0.5,
            PATH_BOTTOM / PATH_HEIGHT,
        ],
    );
    path.cubic(
        false,
        [
            1.0 / 3.0,
            19.8 / PATH_HEIGHT,
            1.0 / 6.0,
            19.8 / PATH_HEIGHT,
            0.0,
            PATH_BOTTOM / PATH_HEIGHT,
        ],
    );
    path.close();
    path
}

/// Exact package perimeter from the D2 shape library linked into TALA.
fn package_path(rect: Rect) -> ShapePath {
    let mut top_width = rect.size.width * 0.5;
    if rect.size.width >= 100.0 {
        top_width = top_width.clamp(50.0, 150.0);
    }
    let top_height = 55.0_f64.min(rect.size.height * 0.2);

    let mut path = ShapePath::new(rect.origin, 1.0, 1.0);
    path.start_at(path.absolute(0.0, 0.0));
    path.line(false, top_width, 0.0);
    path.line(false, top_width, top_height);
    path.line(false, rect.size.width, top_height);
    path.line(false, rect.size.width, rect.size.height);
    path.line(false, 0.0, rect.size.height);
    path.close();
    path
}

fn c4_person_body_path(rect: Rect) -> ShapePath {
    const HEAD_RADIUS_FACTOR: f64 = 0.22;
    const BODY_TOP_FACTOR: f64 = 0.8;
    const CORNER_RADIUS_FACTOR: f64 = 0.175;

    let head_radius = rect.size.width * HEAD_RADIUS_FACTOR;
    let body_top = head_radius + head_radius * BODY_TOP_FACTOR;
    let body_height = rect.size.height - body_top;
    let corner_radius = (rect.size.width * CORNER_RADIUS_FACTOR).min(body_height * 0.25);
    let curve_factor = 4.0 * (2.0_f64.sqrt() - 1.0) / 3.0;

    let mut path = ShapePath::new(rect.origin, 1.0, 1.0);
    path.start_at(path.absolute(0.0, body_top + corner_radius));
    path.cubic(
        true,
        [
            0.0,
            -curve_factor * corner_radius,
            curve_factor * corner_radius,
            -corner_radius,
            corner_radius,
            -corner_radius,
        ],
    );
    path.horizontal(true, rect.size.width - 2.0 * corner_radius);
    path.cubic(
        true,
        [
            curve_factor * corner_radius,
            0.0,
            corner_radius,
            curve_factor * corner_radius,
            corner_radius,
            corner_radius,
        ],
    );
    path.vertical(true, body_height - 2.0 * corner_radius);
    path.cubic(
        true,
        [
            0.0,
            curve_factor * corner_radius,
            -curve_factor * corner_radius,
            corner_radius,
            -corner_radius,
            corner_radius,
        ],
    );
    path.horizontal(true, -(rect.size.width - 2.0 * corner_radius));
    path.cubic(
        true,
        [
            -curve_factor * corner_radius,
            0.0,
            -corner_radius,
            -curve_factor * corner_radius,
            -corner_radius,
            -corner_radius,
        ],
    );
    path.close();
    path
}

fn trace_c4_person(rect: Rect, border: Point, previous: Point) -> TracedPoint {
    const HEAD_RADIUS_FACTOR: f64 = 0.22;

    let ray = extended_shape_ray(previous, border, rect);
    let head_radius = rect.size.width * HEAD_RADIUS_FACTOR;
    let head_center = Point {
        x: rect.origin.x + rect.size.width * 0.5,
        y: rect.origin.y + head_radius,
    };
    let mut intersections = shape_path_intersections(c4_person_body_path(rect), ray);
    intersections.extend(ellipse_intersections(
        head_center,
        head_radius,
        head_radius,
        ray,
    ));
    rounded_result(border, intersections)
}

fn cloud_path(rect: Rect) -> ShapePath {
    let mut path = ShapePath::new(
        rect.origin,
        rect.size.width / 834.0,
        rect.size.height / 523.0,
    );
    path.start_at(path.absolute(137.833, 182.833));
    path.cubic(true, [0.0, 5.556, -5.556, 11.111, -11.111, 11.111]);
    path.cubic(true, [-70.833, 6.944, -126.389, 77.778, -126.389, 163.889]);
    path.cubic(true, [0.0, 91.667, 62.5, 165.278, 141.667, 165.278]);
    path.horizontal(true, 537.5);
    path.cubic(true, [84.723, 0.0, 154.167, -79.167, 154.167, -175.0]);
    path.cubic(true, [0.0, -91.667, -63.89, -168.056, -144.444, -173.611]);
    path.cubic(true, [-5.556, 0.0, -11.111, -4.167, -12.5, -11.111]);
    path.cubic(true, [-18.056, -93.055, -101.39, -162.5, -198.611, -162.5]);
    path.cubic(true, [-63.889, 0.0, -120.834, 29.167, -156.944, 75.0]);
    path.cubic(true, [-4.167, 5.556, -11.111, 6.945, -15.278, 5.556]);
    path.cubic(true, [-13.889, -5.556, -29.166, -8.333, -45.833, -8.333]);
    path.cubic(false, [196.167, 71.722, 143.389, 120.333, 137.833, 182.833]);
    path.close();
    path
}

/// Exact D2 hexagon perimeter used by the TALA release.
///
/// The right/left tips use `43.6 / 87.3`, not an exact half. `ShapePath`
/// also applies D2's float32 precision reduction to every path coordinate
/// before intersection. For odd heights this preserves a center snap point
/// that sits one pixel beyond the rounded polygon tip.
fn hexagon_path(rect: Rect) -> ShapePath {
    let mut path = ShapePath::new(rect.origin, rect.size.width, rect.size.height);
    path.start_at(path.absolute(0.25, 0.0));
    path.line(false, 0.0, 43.6 / 87.3);
    path.line(false, 0.25, 1.0);
    path.line(false, 0.75, 1.0);
    path.line(false, 1.0, 43.6 / 87.3);
    path.line(false, 0.75, 0.0);
    path.close();
    path
}

/// D2's person silhouette used by `shapePerson.TraceToShapeBorder`.
///
/// This is the release D2 `personPath` verbatim in the local path-builder
/// vocabulary.  In particular, the shoulder/head coordinates are expressed
/// in the original 68.3 × 77.4 design space; normalizing them to a unit box
/// changes the cubic intersection by a pixel for horizontal routes.
fn person_path(rect: Rect) -> ShapePath {
    let mut path = ShapePath::new(rect.origin, rect.size.width / 68.3, rect.size.height / 77.4);
    path.start_at(path.absolute(68.3, 77.4));
    path.horizontal(false, 0.0);
    path.vertical(true, -1.1);
    path.cubic(true, [0.0, -13.2, 7.5, -25.1, 19.3, -30.8]);
    path.cubic(false, [12.8, 40.9, 8.9, 33.4, 8.9, 25.2]);
    path.cubic(false, [8.9, 11.3, 20.2, 0.0, 34.1, 0.0]);
    path.cubic(true, [13.9, 0.0, 25.2, 11.3, 25.2, 25.2]);
    path.cubic(true, [0.0, 8.2, -3.8, 15.6, -10.4, 20.4]);
    path.cubic(true, [11.8, 5.7, 19.3, 17.6, 19.3, 30.8]);
    path.vertical(true, 1.0);
    path.horizontal(false, 68.3);
    path.close();
    path
}

fn trace_shape_result(shape: ShapeKind, rect: Rect, border: Point, previous: Point) -> TracedPoint {
    match shape {
        ShapeKind::Rectangle
        | ShapeKind::Square
        | ShapeKind::RealSquare
        | ShapeKind::SqlTable
        | ShapeKind::Class
        | ShapeKind::Text
        | ShapeKind::Code
        | ShapeKind::Image => TracedPoint {
            point: border,
            preserve: true,
            truncate_original: false,
        },
        ShapeKind::Circle | ShapeKind::Oval => trace_ellipse(rect, border, previous),
        ShapeKind::Cloud => trace_shape_path(rect, border, previous, cloud_path(rect)),
        ShapeKind::Diamond => trace_shape_path(rect, border, previous, diamond_path(rect)),
        ShapeKind::Hexagon => trace_shape_path(rect, border, previous, hexagon_path(rect)),
        ShapeKind::Parallelogram => {
            // D2 keeps the 26px wedge for every box wider than the wedge;
            // only collapsed boxes use half their width.
            let wedge_width = if rect.size.width <= 26.0 {
                rect.size.width / 2.0
            } else {
                26.0
            };
            trace_pixel_polygon(
                rect,
                border,
                previous,
                &[
                    (wedge_width, 0.0),
                    (rect.size.width, 0.0),
                    (rect.size.width - wedge_width, rect.size.height),
                    (0.0, rect.size.height),
                    (0.0, rect.size.height),
                ],
            )
        }
        ShapeKind::Step => {
            let wedge_width = if rect.size.width <= 35.0 {
                rect.size.width / 2.0
            } else {
                35.0
            };
            trace_pixel_polygon(
                rect,
                border,
                previous,
                &[
                    (0.0, 0.0),
                    (rect.size.width - wedge_width, 0.0),
                    (rect.size.width, rect.size.height / 2.0),
                    (rect.size.width - wedge_width, rect.size.height),
                    (0.0, rect.size.height),
                    (wedge_width, rect.size.height / 2.0),
                ],
            )
        }
        ShapeKind::Callout => trace_shape_path(rect, border, previous, callout_path(rect)),
        ShapeKind::Cylinder => trace_shape_path(rect, border, previous, cylinder_path(rect)),
        ShapeKind::Queue => trace_shape_path(rect, border, previous, queue_path(rect)),
        ShapeKind::Person => trace_shape_path(rect, border, previous, person_path(rect)),
        ShapeKind::StoredData => trace_shape_path(rect, border, previous, stored_data_path(rect)),
        ShapeKind::Page => trace_shape_path(rect, border, previous, page_outer_path(rect)),
        ShapeKind::Document => trace_shape_path(rect, border, previous, document_path(rect)),
        ShapeKind::Package => trace_shape_path(rect, border, previous, package_path(rect)),
        ShapeKind::C4Person => trace_c4_person(rect, border, previous),
    }
}

#[cfg(test)]
fn trace_shape_border(shape: ShapeKind, rect: Rect, border: Point, previous: Point) -> Point {
    trace_shape_result(shape, rect, border, previous).point
}
#[cfg(test)]
fn rounded_closest(border: Point, intersections: impl IntoIterator<Item = Point>) -> Point {
    rounded_result(border, intersections).point
}
fn actual_rect(graph: &ArenaGraph, node: NodeId) -> Rect {
    Rect {
        origin: graph.position(node).unwrap_or_else(|| {
            std::panic::panic_any(
                "runtime error: invalid memory address or nil pointer dereference",
            )
        }),
        size: graph.nodes[node.0 as usize].rect.size,
    }
}
// The source searches Top, Right, Left, Bottom, independently of port storage order.
fn port_orientation(graph: &ArenaGraph, node: NodeId, point: Point) -> Option<PortSide> {
    let n = &graph.nodes[node.0 as usize];
    let ports =
        ports_with_table_column_count(actual_rect(graph, node), n.shape, n.table_column_count);
    [
        PortSide::Top,
        PortSide::Right,
        PortSide::Left,
        PortSide::Bottom,
    ]
    .into_iter()
    .find(|&side| ports.iter().any(|p| p.side == side && p.point == point))
}
fn modifier_endpoint(
    graph: &ArenaGraph,
    node: NodeId,
    endpoint: &SharedOwnerPoint,
    previous: &SharedOwnerPoint,
    originals: &mut Vec<(SharedOwnerPoint, Point)>,
) {
    let (dx, dy) = modifier_element_adjustments(&graph.nodes[node.0 as usize]);
    if dx == 0.0 && dy == 0.0 {
        return;
    }
    let top_left = graph
        .owner_node_point(node)
        .unwrap()
        .shared_point()
        .unwrap_or_else(|| {
            std::panic::panic_any(
                "runtime error: invalid memory address or nil pointer dereference",
            )
        });
    let r = actual_rect(graph, node);
    let start = endpoint.get();
    if !(start.x > r.origin.x + dx && start.y < r.origin.y + r.size.height - dy) {
        return;
    }
    let original = top_left.get();
    originals.push((top_left.clone(), original));
    let orientation = port_orientation(graph, node, endpoint.get());
    let candidate =
        if orientation == Some(PortSide::Right) || endpoint.get().x == original.x + r.size.width {
            let mut p = top_left.get();
            p.x += dx;
            top_left.set(p);
            let top_right = Point {
                x: top_left.get().x + r.size.width,
                y: top_left.get().y,
            };
            let bottom_right = Point {
                x: top_right.x,
                y: top_right.y + r.size.height,
            };
            line_intersection_point(endpoint.get(), previous.get(), top_right, bottom_right)
        } else if orientation == Some(PortSide::Top) || endpoint.get().y == original.y {
            let mut p = top_left.get();
            p.y -= dy;
            top_left.set(p);
            let top_right = Point {
                x: top_left.get().x + r.size.width,
                y: top_left.get().y,
            };
            line_intersection_point(endpoint.get(), previous.get(), top_left.get(), top_right)
        } else {
            None
        };
    if let Some(point) = candidate {
        endpoint.set(point);
    }
}
fn trace_endpoint(graph: &ArenaGraph, edge: EdgeId, node: NodeId, slot: usize, previous: usize) {
    let route = &graph.edge_records[edge.0 as usize].route;
    let point = route.slot(slot).unwrap();
    let before = point.get();
    let traced = trace_shape_result(
        graph.nodes[node.0 as usize].shape,
        actual_rect(graph, node),
        before,
        route.point(previous),
    );
    if traced.preserve {
        return;
    }
    if traced.truncate_original {
        point.set(Point {
            x: before.x as f32 as f64,
            y: before.y as f32 as f64,
        });
    }
    route.replace_point(slot, traced.point);
}
// trace.go: preserve shared TopLeft/route points and reverse modifier restoration.
fn trace_edge(graph: &mut ArenaGraph, edge: EdgeId) {
    let e = &graph.edge_records[edge.0 as usize];
    if e.route.len() < 2 {
        return;
    }
    let (from, to, last) = (e.from, e.to, e.route.len() - 1);
    let mut materialized = Vec::new();
    for node in [from, to] {
        if graph.materialize_owner_node_point(node).unwrap() {
            materialized.push(node);
        }
    }
    let mut originals = Vec::new();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let route = &graph.edge_records[edge.0 as usize].route;
        modifier_endpoint(
            graph,
            from,
            &route.slot(0).unwrap(),
            &route.slot(1).unwrap(),
            &mut originals,
        );
        modifier_endpoint(
            graph,
            to,
            &route.slot(last).unwrap(),
            &route.slot(last - 1).unwrap(),
            &mut originals,
        );
        trace_endpoint(graph, edge, from, 0, 1);
        trace_endpoint(graph, edge, to, last, last - 1);
    }));
    while let Some((point, value)) = originals.pop() {
        point.set(value);
    }
    for node in materialized {
        graph.release_materialized_owner_node_point(node);
    }
    if let Err(payload) = result {
        std::panic::resume_unwind(payload);
    }
}
pub(super) fn trace_edge_guarded(
    graph: &mut ArenaGraph,
    edge: EdgeId,
    guard: &RouteAtomicGuard<'_>,
) -> Result<(), RouteStageError> {
    guard.step()?;
    let e = &graph.edge_records[edge.0 as usize];
    if e.route.len() < 2 {
        return Ok(());
    }
    // Public atomic snapshot restores all captured identities on panic; the
    // private route helper additionally restores each endpoint's TopLeft.
    let nodes = [e.from, e.to];
    let mut materialized = Vec::new();
    let mut snapshots = Vec::new();
    for node in nodes {
        if graph.materialize_owner_node_point(node).unwrap() {
            materialized.push(node);
        }
    }
    for node in nodes {
        snapshots.push((node, graph.owner_node_point(node).unwrap().snapshot()));
    }
    let outcome =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| trace_edge(graph, edge)));
    if outcome.is_err() {
        for (node, snapshot) in snapshots {
            graph.restore_owner_node_point(node, &snapshot).unwrap();
        }
    }
    for node in materialized {
        graph.release_materialized_owner_node_point(node);
    }
    if let Err(payload) = outcome {
        std::panic::resume_unwind(payload);
    }
    guard.step()
}
pub(super) fn trace_with_context(
    graph: &mut ArenaGraph,
    context: Option<&dyn limits::Context>,
    budget: &RouteStageWork,
) -> Result<(), RouteStageError> {
    route_stage_guard::run_atomic_route_stage_typed(
        graph,
        Some(OwnerGraphId(0)),
        context,
        "TraceEdgesToShapeBorder",
        &[],
        budget,
        |g, guard| {
            for edge in super::nudge::edges(g) {
                trace_edge_guarded(g, edge, guard)?;
            }
            Ok(())
        },
    )
}
/// Source atomic TraceEdgesToShapeBorder, retaining typed errors and work order.
pub(in crate::internal) fn trace_edges_to_shape_border_with_budget(
    graph: &mut ArenaGraph,
    budget: &RouteStageWork,
) -> Result<(), RouteStageError> {
    trace_with_context(graph, Some(&RuntimeRouteContext), budget)
}
pub(in crate::internal) fn trace_edges_to_shape_border(graph: &mut ArenaGraph) {
    let _ = trace_edges_to_shape_border_with_budget(
        graph,
        &RouteStageWork::new(MAX_ROUTE_STAGE_WORK_UNITS),
    );
}
/// Source private per-edge body used by the standalone additional-edge caller.
pub(in crate::internal) fn trace_edges_to_shape_border_in(graph: &mut ArenaGraph, edges: &[usize]) {
    for &edge in edges {
        trace_edge(graph, EdgeId(edge as u32));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Edge, Graph, Node, Size};

    #[test]
    fn modifier_loop_traces_both_ends_against_the_same_shifted_node() {
        let mut input = Graph::default();
        let mut node = Node::new(
            "hexagon",
            Size {
                width: 100.0,
                height: 80.0,
            },
        );
        node.shape = ShapeKind::Hexagon;
        node.is_3d = true;
        let id = input.add_node(node);
        input.set_initial_position(id, Some(Point::default()));
        let edge = input.add_edge(Edge {
            source: id,
            target: id,
        });
        input.set_edge_route(
            edge,
            vec![
                Point { x: 13.0, y: 20.0 },
                Point { x: -17.0, y: 20.0 },
                Point { x: -17.0, y: -30.0 },
                Point { x: 25.0, y: -30.0 },
                Point { x: 25.0, y: 0.0 },
            ],
        );
        let mut graph = ArenaGraph::from_input(&input);
        trace_edges_to_shape_border(&mut graph);
        assert_eq!(
            graph.edge_records[edge.0 as usize].route.point(0),
            Point { x: 7.0, y: 20.0 }
        );
        assert_eq!(graph.position(id), Some(Point::default()));
        assert_eq!(graph.nodes[id.0 as usize].rect.origin, Point::default());
    }

    #[test]
    fn segment_intersection_rounds_displacement_before_origin() {
        let intersection = line_intersection_point(
            Point { x: 0.4, y: 0.0 },
            Point { x: 1.4, y: 1.0 },
            Point { x: 0.0, y: 0.4 },
            Point { x: 2.0, y: 0.4 },
        )
        .unwrap();

        assert_eq!(intersection, Point { x: 0.4, y: 0.0 });
    }

    #[test]
    fn step_trace_quantizes_svg_path_vertices_before_intersection() {
        let traced = trace_shape_border(
            ShapeKind::Step,
            Rect {
                origin: Point {
                    x: 2021.0,
                    y: 2446.0,
                },
                size: Size {
                    width: 116.0,
                    height: 101.0,
                },
            },
            Point {
                x: 2038.0,
                y: 2496.0,
            },
            Point {
                x: 598.0,
                y: 2496.0,
            },
        );

        assert_eq!(
            traced,
            Point {
                x: 2055.0,
                y: 2496.0
            }
        );
    }

    #[test]
    fn short_nonzero_shape_trace_ray_is_extended_with_exact_axis_selection() {
        let mut input = Graph::default();
        let mut from = Node::new(
            "from",
            Size {
                width: 100.0,
                height: 10.0,
            },
        );
        from.shape = ShapeKind::Diamond;
        let from_id = input.add_node(from);
        let to_id = input.add_node(Node::new(
            "to",
            Size {
                width: 10.0,
                height: 10.0,
            },
        ));
        input.set_initial_position(from_id, Some(Point { x: 0.0, y: 0.0 }));
        input.set_initial_position(to_id, Some(Point { x: 200.0, y: 0.0 }));
        let edge = input.add_edge(Edge {
            source: from_id,
            target: to_id,
        });
        input.set_edge_route(
            edge,
            vec![
                Point { x: 0.0, y: 0.0 },
                Point {
                    x: -5.0e-10,
                    y: -1.0e-10,
                },
            ],
        );
        let mut graph = ArenaGraph::from_input(&input);
        trace_edges_to_shape_border(&mut graph);

        assert_eq!(
            graph.edge_records[edge.0 as usize].route.point(0),
            Point { x: 17.0, y: 3.0 }
        );
    }

    #[test]
    fn vertical_ellipse_intersections_use_exact_segment_bounds() {
        let beyond_endpoint = ellipse_intersections(
            Point { x: 0.0, y: 0.0 },
            1.0,
            1.0,
            (Point { x: 0.0, y: 1.00005 }, Point { x: 0.0, y: 2.0 }),
        );
        let on_endpoint = ellipse_intersections(
            Point { x: 0.0, y: 0.0 },
            1.0,
            1.0,
            (Point { x: 0.0, y: 1.0 }, Point { x: 0.0, y: 2.0 }),
        );

        assert!(beyond_endpoint.is_empty());
        assert_eq!(on_endpoint, vec![Point { x: 0.0, y: 1.0 }]);
    }

    #[test]
    fn shape_border_selection_uses_go_axis_distance_when_squares_overflow() {
        let closest = rounded_closest(
            Point { x: 0.0, y: 0.0 },
            [Point { x: 0.0, y: 9.0e199 }, Point { x: 1.0e200, y: 0.0 }],
        );

        assert_eq!(closest.x, 0.0);
        assert!(closest.y.is_infinite());
    }

    fn source_trace_fixture(shape: &str, modifier: bool) -> ArenaGraph {
        let value = serde_json::json!({"nodes":[{"id":1,"topLeft":{"x":0,"y":0},"width":100,"height":100,"shapeType":shape,"is3D":modifier},{"id":2,"topLeft":{"x":300,"y":0},"width":100,"height":100,"shapeType":shape,"is3D":modifier}],"edges":[{"id":11,"from":1,"to":2,"points":[{"x":100,"y":0},{"x":300,"y":100}]}],"containers":{"0":[1,2]}});
        ArenaGraph::from_input(
            &crate::graphjson::graphjson_to_graph(&serde_json::to_vec(&value).unwrap()).unwrap(),
        )
    }
    #[test]
    fn shape_trace_rectangles_preserve_endpoint_allocations() {
        let mut g = source_trace_fixture("Square", false);
        let header = g.edge_records[0].route.clone();
        let first = header.slot(0).unwrap();
        let last = header.slot(1).unwrap();
        let work = RouteStageWork::new(10000);
        trace_edges_to_shape_border_with_budget(&mut g, &work).unwrap();
        assert_eq!(work.used(), 20);
        assert!(
            header
                .header()
                .same_backing_allocation(&g.edge_records[0].route.header())
        );
        assert!(first.same_allocation(&header.slot(0).unwrap()));
        assert!(last.same_allocation(&header.slot(1).unwrap()));
    }
    #[test]
    fn shape_trace_modifiers_mutate_old_points_before_slot_replacement() {
        let mut g = source_trace_fixture("Circle", true);
        let old = g.edge_records[0].route.slot(0).unwrap();
        let header = g.edge_records[0].route.clone();
        trace_edges_to_shape_border_with_budget(&mut g, &RouteStageWork::new(10000)).unwrap();
        assert_eq!(old.get(), Point { x: 115.0, y: 8.0 });
        assert!(!old.same_allocation(&g.edge_records[0].route.slot(0).unwrap()));
        assert!(
            header
                .header()
                .same_backing_allocation(&g.edge_records[0].route.header())
        );
        assert_eq!(g.position(NodeId(0)), Some(Point { x: 0.0, y: 0.0 }));
    }
    #[test]
    fn shape_trace_no_intersection_truncates_the_retained_fallback_point() {
        let mut g = source_trace_fixture("Circle", false);
        let value = Point {
            x: 20.123456789,
            y: 10.987654321,
        };
        g.edge_records[0]
            .route
            .replace(RoutePoints::literal(&[value, value]));
        let old = g.edge_records[0].route.slot(0).unwrap();
        trace_edges_to_shape_border_with_budget(&mut g, &RouteStageWork::new(10000)).unwrap();
        assert_eq!(
            old.get(),
            Point {
                x: value.x as f32 as f64,
                y: value.y as f32 as f64
            }
        );
        assert_eq!(g.edge_records[0].route.point(0), Point { x: 20.0, y: 11.0 });
        assert!(!old.same_allocation(&g.edge_records[0].route.slot(0).unwrap()));
    }
    #[test]
    fn shape_trace_every_work_limit_restores_slots_and_mutated_points() {
        for limit in 0..20 {
            let mut g = source_trace_fixture("Circle", true);
            let header = g.edge_records[0].route.clone();
            let points = [header.slot(0).unwrap(), header.slot(1).unwrap()];
            let values = points.each_ref().map(|p| p.get());
            let error =
                trace_edges_to_shape_border_with_budget(&mut g, &RouteStageWork::new(limit))
                    .unwrap_err();
            assert!(error.is_route_work_limit(), "limit {limit}: {error}");
            assert!(
                header
                    .header()
                    .same_backing_allocation(&g.edge_records[0].route.header())
            );
            for i in 0..2 {
                assert_eq!(points[i].get(), values[i]);
                assert!(points[i].same_allocation(&g.edge_records[0].route.slot(i).unwrap()));
            }
        }
    }
    #[test]
    fn shape_trace_final_cancellation_restores_original_route_points() {
        struct Cancel(std::cell::Cell<usize>);
        impl limits::Context for Cancel {
            fn done(&self) -> Option<bool> {
                None
            }
            fn error(&self) -> Option<limits::ContextError> {
                let n = self.0.get() + 1;
                self.0.set(n);
                (n == 11).then_some(limits::ContextError::Cancelled)
            }
        }
        let mut g = source_trace_fixture("Circle", true);
        let old = g.edge_records[0].route.slot(0).unwrap();
        let context = Cancel(std::cell::Cell::new(0));
        let error =
            trace_with_context(&mut g, Some(&context), &RouteStageWork::new(10000)).unwrap_err();
        assert_eq!(
            error.to_string(),
            "TraceEdgesToShapeBorder: context canceled"
        );
        assert_eq!(context.0.get(), 11);
        assert!(old.same_allocation(&g.edge_records[0].route.slot(0).unwrap()));
        assert_eq!(old.get(), Point { x: 100.0, y: 0.0 });
    }
    #[test]
    fn shape_trace_cylinder_preserves_source_negative_zero() {
        let mut g = source_trace_fixture("Cylinder", false);
        g.edge_records[0].route.replace(RoutePoints::literal(&[
            Point { x: 50.0, y: 0.0 },
            Point { x: 50.0, y: -100.0 },
        ]));
        trace_edges_to_shape_border_with_budget(&mut g, &RouteStageWork::new(10000)).unwrap();
        assert_eq!(
            g.edge_records[0].route.point(0).y.to_bits(),
            (-0.0_f64).to_bits()
        );
    }
    #[test]
    fn shape_trace_large_circle_keeps_fused_ellipse_intersection() {
        let mut g = source_trace_fixture("Circle", false);
        for node in &mut g.nodes {
            node.rect.size = Size {
                width: 1e20,
                height: 1e20,
            };
        }
        trace_edges_to_shape_border_with_budget(&mut g, &RouteStageWork::new(10000)).unwrap();
        assert_eq!(
            g.edge_records[0].route.point(1).x.to_bits(),
            0x43f158e460000000
        );
    }
}
