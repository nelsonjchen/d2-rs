// SPDX-License-Identifier: MPL-2.0
// Derived from D2 v0.9.0 lib/geo and lib/shape perimeter tracing.
//! Shared D2 shape-border intersection behavior used by layout engines.

use crate::{Point, Rect, ShapeKind};

// The pinned Go ARM64 compiler emits a multiply followed by FMSUB for
// geo.IntersectionPoint's differences of products. Keep its first product
// rounded and fuse the second product/subtraction; halfway port rounding
// observes the final bit. Go's baseline non-ARM64 path uses separate operations.
#[inline]
fn intersection_difference(a: f64, b: f64, c: f64, d: f64) -> f64 {
    #[cfg(target_arch = "aarch64")]
    {
        let result;
        // Go emits these scalar instructions with this operand order. LLVM
        // may commute FMA operands, which changes a propagated NaN's sign.
        // SAFETY: these instructions access only their declared FP registers;
        // they neither read memory nor change the stack or control registers.
        unsafe {
            std::arch::asm!(
                "fmul {product:d}, {a:d}, {b:d}",
                "fmsub {result:d}, {c:d}, {d:d}, {product:d}",
                a = in(vreg) a, b = in(vreg) b,
                c = in(vreg) c, d = in(vreg) d,
                product = out(vreg) _, result = lateout(vreg) result,
                options(pure, nomem, nostack),
            );
        }
        result
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        a * b - c * d
    }
}

#[inline]
fn intersection_step(origin: f64, scale: f64, delta: f64) -> f64 {
    #[cfg(target_arch = "aarch64")]
    {
        let result;
        // Preserve Go's scale * delta operand order as well as the separate
        // rounding and origin addition. Two NaNs can carry different signs.
        // SAFETY: declared FP register operations only; no memory, stack or
        // control-register access. The architecture is selected at compile time.
        unsafe {
            std::arch::asm!(
                "fmul {step:d}, {scale:d}, {delta:d}",
                "frinta {step:d}, {step:d}",
                "fadd {result:d}, {origin:d}, {step:d}",
                origin = in(vreg) origin, scale = in(vreg) scale, delta = in(vreg) delta,
                step = out(vreg) _, result = lateout(vreg) result,
                options(pure, nomem, nostack),
            );
        }
        result
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        origin + crate::go_round(scale * delta)
    }
}

/// D2 `geo.IntersectionPoint`: intersect two closed segments and round the
/// displacement along the first segment before adding its origin. Parallel
/// segments return `None`; the argument order affects fractional results.
pub fn line_intersection_point(a: Point, b: Point, c: Point, d: Point) -> Option<Point> {
    // Direct translation of D2 geo.IntersectionPoint, which TALA calls while
    // tracing every shape perimeter. It rounds the displacement along the
    // first segment before adding its origin; rounding only the final point is
    // observably different when the segment starts at a fractional position.
    let a_dx = b.x - a.x;
    let c_dx = d.x - c.x;
    let ac_dx = c.x - a.x;
    let a_dy = b.y - a.y;
    let c_dy = d.y - c.y;
    let ac_dy = c.y - a.y;
    let denominator = intersection_difference(a_dy, c_dx, a_dx, c_dy);
    if denominator == 0.0 {
        return None;
    }
    let first_t = intersection_difference(ac_dy, c_dx, ac_dx, c_dy) / denominator;
    let second_t = intersection_difference(a_dx, ac_dy, ac_dx, a_dy) / denominator;
    if first_t < 0.0 || first_t > 1.0 || second_t < 0.0 || second_t > 1.0 {
        return None;
    }
    Some(Point {
        x: intersection_step(a.x, first_t, a_dx),
        y: intersection_step(a.y, first_t, a_dy),
    })
}

/// D2 `geo.Box.Intersections`, in top/right/bottom/left side order. Corner
/// intersections are retained once for each intersected side; endpoints count.
pub fn box_intersections(rect: Rect, start: Point, end: Point) -> Vec<Point> {
    let top_left = rect.origin;
    let top_right = Point {
        x: top_left.x + rect.size.width,
        y: top_left.y,
    };
    let bottom_right = Point {
        x: top_right.x,
        y: top_right.y + rect.size.height,
    };
    let bottom_left = Point {
        x: top_left.x,
        y: bottom_right.y,
    };
    [
        (top_left, top_right),
        (top_right, bottom_right),
        (bottom_right, bottom_left),
        (bottom_left, top_left),
    ]
    .into_iter()
    .filter_map(|(a, b)| line_intersection_point(start, end, a, b))
    .collect()
}

fn extended_shape_ray(previous: Point, border: Point, rect: Rect) -> (Point, Point) {
    let dx = border.x - previous.x;
    let dy = border.y - previous.y;
    let length = (dx * dx + dy * dy).sqrt();
    if length <= 1e-9 {
        return (previous, border);
    }
    let extension = if dx.abs() <= 1e-9 {
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

fn trace_polygon(rect: Rect, border: Point, previous: Point, normalized: &[(f64, f64)]) -> Point {
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
            let distance =
                ((intersection.x - border.x).powi(2) + (intersection.y - border.y).powi(2)).sqrt();
            if distance < closest_distance {
                closest = intersection;
                closest_distance = distance;
            }
        }
    }
    Point {
        x: (closest.x as f32 as f64).round(),
        y: (closest.y as f32 as f64).round(),
    }
}

fn trace_pixel_polygon(
    rect: Rect,
    border: Point,
    previous: Point,
    local_points: &[(f64, f64)],
) -> Point {
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
            let distance =
                ((intersection.x - border.x).powi(2) + (intersection.y - border.y).powi(2)).sqrt();
            if distance < closest_distance {
                closest = intersection;
                closest_distance = distance;
            }
        }
    }
    Point {
        x: (closest.x as f32 as f64).round(),
        y: (closest.y as f32 as f64).round(),
    }
}

fn distance_to_segment(point: Point, start: Point, end: Point) -> f64 {
    // Direct translation of geo.Point.DistanceToLine.  D2 calls the helper
    // with a finite ray segment even though the ellipse roots came from the
    // infinite line equation.
    let a = point.x - start.x;
    let b = point.y - start.y;
    let c = end.x - start.x;
    let d = end.y - start.y;
    let dot = a * c + b * d;
    let len_sq = c * c + d * d;
    let param = if len_sq != 0.0 { dot / len_sq } else { -1.0 };
    let (closest_x, closest_y) = if param < 0.0 {
        (start.x, start.y)
    } else if param > 1.0 {
        (end.x, end.y)
    } else {
        (start.x + param * c, start.y + param * d)
    };
    let dx = point.x - closest_x;
    let dy = point.y - closest_y;
    (dx * dx + dy * dy).sqrt()
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
    let on_segment = |point: Point| {
        distance_to_segment(point, Point { x: x1, y: y1 }, Point { x: x2, y: y2 }) < 0.0001
    };

    if x1 == x2 {
        let y = b * (a2 - x1 * x1).sqrt() / a;
        let first = Point { x: x1, y };
        let second = Point { x: x1, y: -y };
        let mut intersections = Vec::new();
        if on_segment(first) {
            intersections.push(Point {
                x: first.x + center.x,
                y: first.y + center.y,
            });
        }
        if second.y != 0.0 && on_segment(second) {
            intersections.push(Point {
                x: second.x + center.x,
                y: second.y + center.y,
            });
        }
        return intersections;
    }

    let slope = (y2 - y1) / (x2 - x1);
    let intercept = y1 - slope * x1;
    let denominator = a2 * slope * slope + b2;
    let root = (a2 * b2 * (denominator - intercept * intercept)).sqrt();
    let first = Point {
        x: (-slope * intercept * a2 + root) / denominator,
        y: (intercept * b2 + slope * root) / denominator,
    };
    let second = Point {
        x: (-slope * intercept * a2 - root) / denominator,
        y: (intercept * b2 - slope * root) / denominator,
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

fn rounded_closest(border: Point, intersections: impl IntoIterator<Item = Point>) -> Point {
    let mut closest = border;
    let mut closest_distance = f64::INFINITY;
    for point in intersections {
        let distance = ((point.x - border.x).powi(2) + (point.y - border.y).powi(2)).sqrt();
        if distance < closest_distance {
            closest = point;
            closest_distance = distance;
        }
    }
    Point {
        x: (closest.x as f32 as f64).round(),
        y: (closest.y as f32 as f64).round(),
    }
}

fn trace_ellipse(rect: Rect, border: Point, previous: Point) -> Point {
    let ray = extended_shape_ray(previous, border, rect);
    let center = Point {
        x: rect.origin.x + rect.size.width * 0.5,
        y: rect.origin.y + rect.size.height * 0.5,
    };
    rounded_closest(
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
        -p0 + 3.0 * p1 - 3.0 * p2 + p3,
        3.0 * p0 - 6.0 * p1 + 3.0 * p2,
        -3.0 * p0 + 3.0 * p1,
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
            let mut roots = [-polynomial[3] / polynomial[2], -1.0, -1.0];
            if precision_compare(roots[0], 0.0, PRECISION).is_lt()
                || precision_compare(roots[0], 1.0, PRECISION).is_gt()
            {
                roots[0] = -1.0;
            }
            return sort_special_roots(roots);
        }

        let mut discriminant = polynomial[2].powi(2) - 4.0 * polynomial[1] * polynomial[3];
        if precision_compare(discriminant, 0.0, PRECISION).is_ge() {
            discriminant = discriminant.sqrt();
            let mut roots = [
                -(discriminant + polynomial[2]) / (2.0 * polynomial[1]),
                (discriminant - polynomial[2]) / (2.0 * polynomial[1]),
                -1.0,
            ];
            for root in roots.iter_mut().take(2) {
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
    let q = (3.0 * b - a.powi(2)) / 9.0;
    let r = (9.0 * a * b - 27.0 * c - 2.0 * a.powi(3)) / 54.0;
    let discriminant = q.powi(3) + r.powi(2);
    let mut roots;
    if precision_compare(discriminant, 0.0, PRECISION).is_ge() {
        let signed_cube_root = |value: f64| value.signum() * value.abs().powf(1.0 / 3.0);
        let s = signed_cube_root(r + discriminant.sqrt());
        let t = signed_cube_root(r - discriminant.sqrt());
        roots = [
            -a / 3.0 + s + t,
            -a / 3.0 - (s + t) / 2.0,
            -a / 3.0 - (s + t) / 2.0,
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
    let line_c = line.0.x * (line.0.y - line.1.y) + line.0.y * (line.1.x - line.0.x);
    let x = bezier_coefficients(curve[0].x, curve[1].x, curve[2].x, curve[3].x);
    let y = bezier_coefficients(curve[0].y, curve[1].y, curve[2].y, curve[3].y);
    let roots = cubic_roots([
        line_a * x[0] + line_b * y[0],
        line_a * x[1] + line_b * y[1],
        line_a * x[2] + line_b * y[2],
        line_a * x[3] + line_b * y[3] + line_c,
    ]);
    let mut intersections = Vec::new();
    for parameter in roots {
        let point = Point {
            x: x[0] * parameter.powi(3) + x[1] * parameter.powi(2) + x[2] * parameter + x[3],
            y: y[0] * parameter.powi(3) + y[1] * parameter.powi(2) + y[2] * parameter + y[3],
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

fn trace_shape_path(rect: Rect, border: Point, previous: Point, path: ShapePath) -> Point {
    let ray = extended_shape_ray(previous, border, rect);
    rounded_closest(border, shape_path_intersections(path, ray))
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

fn trace_c4_person(rect: Rect, border: Point, previous: Point) -> Point {
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
    rounded_closest(border, intersections)
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

pub fn trace_shape_border(shape: ShapeKind, rect: Rect, border: Point, previous: Point) -> Point {
    match shape {
        ShapeKind::Rectangle
        | ShapeKind::Square
        | ShapeKind::RealSquare
        | ShapeKind::SqlTable
        | ShapeKind::Class
        | ShapeKind::Text
        | ShapeKind::Code
        | ShapeKind::Image => border,
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
            let wedge = wedge_width / rect.size.width.max(1.0);
            trace_polygon(
                rect,
                border,
                previous,
                &[(wedge, 0.0), (1.0, 0.0), (1.0 - wedge, 1.0), (0.0, 1.0)],
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
mod tests {

    #[cfg(target_arch = "aarch64")]
    #[test]
    fn intersection_preserves_go_arm64_nan_payload_and_operand_order() {
        let nan = f64::from_bits(0x7ff8_0000_0000_0042);
        let result = super::line_intersection_point(
            Point { x: nan, y: 0.0 },
            Point { x: 0.0, y: nan },
            Point { x: 0.0, y: 1.0 },
            Point { x: 1.0, y: 0.0 },
        )
        .unwrap();
        assert_eq!(result.x.to_bits(), 0x7ff8_0000_0000_0042);
        assert_eq!(result.y.to_bits(), 0xfff8_0000_0000_0042);
    }
    #[test]
    fn box_intersections_preserve_side_order_duplicates_and_closed_endpoints() {
        let rect = Rect {
            origin: Point { x: 0.25, y: 0.25 },
            size: Size {
                width: 10.0,
                height: 10.0,
            },
        };
        assert_eq!(
            super::box_intersections(
                rect,
                Point { x: -5.75, y: 5.25 },
                Point { x: 15.25, y: 5.25 }
            ),
            vec![Point { x: 10.25, y: 5.25 }, Point { x: 0.25, y: 5.25 }]
        );
        assert_eq!(
            super::box_intersections(rect, Point { x: 0.25, y: 0.25 }, Point { x: 5.25, y: 5.25 }),
            vec![Point { x: 0.25, y: 0.25 }; 2]
        );
    }

    #[test]
    fn segment_intersection_rounds_displacement_before_fractional_origin() {
        let point = super::line_intersection_point(
            Point { x: 0.25, y: 0.25 },
            Point { x: 2.25, y: 2.25 },
            Point { x: 0.75, y: -1.0 },
            Point { x: 0.75, y: 3.0 },
        )
        .unwrap();
        assert_eq!(point, Point { x: 1.25, y: 1.25 });
        let reverse = super::line_intersection_point(
            Point { x: 2.25, y: 2.25 },
            Point { x: 0.25, y: 0.25 },
            Point { x: 0.75, y: -1.0 },
            Point { x: 0.75, y: 3.0 },
        )
        .unwrap();
        assert_eq!(reverse, Point { x: 0.25, y: 0.25 });
    }

    use super::trace_shape_border;
    use crate::{Point, Rect, ShapeKind, Size};

    #[test]
    fn callout_vertical_route_matches_v090_collinear_perimeter_intersection() {
        let rect = Rect {
            origin: Point { x: 677.0, y: 51.0 },
            size: Size {
                width: 95.0,
                height: 91.0,
            },
        };
        assert_eq!(
            trace_shape_border(
                ShapeKind::Callout,
                rect,
                Point { x: 724.5, y: 142.5 },
                Point { x: 724.5, y: 192.0 },
            ),
            Point { x: 725.0, y: 97.0 }
        );
    }
}

#[cfg(all(test, target_arch = "aarch64", feature = "oracle-fixtures"))]
mod badge_ray_source_contract {
    use super::*;
    #[test]
    fn compact_diamond_ray_matches_pristine_arm64_halfway_rounding() {
        let source: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/badge-ray-source-v090-arm64.json"
        ))
        .unwrap();
        let rect = Rect {
            origin: Point { x: 0.0, y: 0.0 },
            size: crate::Size {
                width: 10.0,
                height: 10.0,
            },
        };
        let corner = Point { x: 10.0, y: 0.0 };
        let previous = Point { x: 15.0, y: -5.0 };
        let ray = extended_shape_ray(previous, corner, rect);
        let source_end: Point = serde_json::from_value(source["ray"]["End"].clone()).unwrap();
        assert_eq!(ray.1, source_end);
        let row = source["segments"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| {
                row["intersections"]
                    .as_array()
                    .is_some_and(|points| !points.is_empty())
            })
            .unwrap();
        let start: Point = serde_json::from_value(row["line"]["Start"].clone()).unwrap();
        let end: Point = serde_json::from_value(row["line"]["End"].clone()).unwrap();
        let udx = end.x - start.x;
        let vdx = ray.1.x - ray.0.x;
        let uvdx = ray.0.x - start.x;
        let udy = end.y - start.y;
        let vdy = ray.1.y - ray.0.y;
        let uvdy = ray.0.y - start.y;
        let denominator = intersection_difference(udy, vdx, udx, vdy);
        let numerator = intersection_difference(vdx, uvdy, vdy, uvdx);
        let ratio = numerator / denominator;
        assert_eq!(denominator.to_bits(), row["denom"].as_u64().unwrap());
        assert_eq!(numerator.to_bits(), row["numerator"].as_u64().unwrap());
        assert_eq!(ratio.to_bits(), row["ratio"].as_u64().unwrap());
        assert_eq!((ratio * udx).to_bits(), row["x_step"].as_u64().unwrap());
        assert_eq!((ratio * udy).to_bits(), row["y_step"].as_u64().unwrap());
        let point: Point = serde_json::from_value(source["point"].clone()).unwrap();
        assert_eq!(
            line_intersection_point(start, end, ray.0, ray.1),
            Some(point)
        );
        assert_eq!(
            trace_shape_border(ShapeKind::Diamond, rect, corner, previous),
            point
        );
    }
}
