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
    intersection_with(a, b, c, d, intersection_difference)
}

#[inline(always)]
fn intersection_with(
    a: Point,
    b: Point,
    c: Point,
    d: Point,
    intersection_difference: impl Fn(f64, f64, f64, f64) -> f64,
) -> Option<Point> {
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

/// One build of D2's `lib/geo` arithmetic. `fma` is how that build evaluates
/// `x*y + z` (and `z - x*y` as `fma(-x, y, z)`): the pinned Go compiler fuses
/// these on arm64 and never on amd64 (lib/geo has no FMA instruction in an
/// amd64 build), so the operand chosen for fusion follows the arm64 assembly.
trait GeoArith {
    fn fma(x: f64, y: f64, z: f64) -> f64;
    fn intersection(a: Point, b: Point, c: Point, d: Point) -> Option<Point> {
        line_intersection_point(a, b, c, d)
    }
}

/// Go `lib/geo` as compiled for this host's architecture.
struct GoNative;
impl GeoArith for GoNative {
    #[inline(always)]
    fn fma(x: f64, y: f64, z: f64) -> f64 {
        #[cfg(target_arch = "aarch64")]
        {
            x.mul_add(y, z)
        }
        #[cfg(not(target_arch = "aarch64"))]
        {
            x * y + z
        }
    }
}

fn extended_shape_ray<A: GeoArith>(previous: Point, border: Point, rect: Rect) -> (Point, Point) {
    let dx = border.x - previous.x;
    let dy = border.y - previous.y;
    // geo.Vector.Length sums `comp*comp` from zero.
    let length = A::fma(dy, dy, A::fma(dx, dx, 0.0)).sqrt();
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

/// geo.EuclideanDistance.
#[inline(always)]
fn border_distance<A: GeoArith>(border: Point, point: Point) -> f64 {
    if border.x == point.x {
        (border.y - point.y).abs()
    } else if border.y == point.y {
        (border.x - point.x).abs()
    } else {
        let dx = border.x - point.x;
        let dy = border.y - point.y;
        A::fma(dy, dy, dx * dx).sqrt()
    }
}

/// The closest intersection, truncated to float32 and rounded as in
/// shape.TraceToShapeBorder, and whether any intersection was found.
fn rounded_closest<A: GeoArith>(
    border: Point,
    intersections: impl IntoIterator<Item = Point>,
) -> (Point, bool) {
    let mut closest = border;
    let mut closest_distance = f64::INFINITY;
    for point in intersections {
        let distance = border_distance::<A>(border, point);
        if distance < closest_distance {
            closest = point;
            closest_distance = distance;
        }
    }
    (
        Point {
            x: (closest.x as f32 as f64).round(),
            y: (closest.y as f32 as f64).round(),
        },
        closest_distance != f64::INFINITY,
    )
}

/// Trace a closed polygon whose vertices are `origin + scale * local`.
fn trace_polygon<A: GeoArith>(
    rect: Rect,
    border: Point,
    previous: Point,
    local_points: &[(f64, f64)],
    scale: (f64, f64),
) -> (Point, bool) {
    let (ray_start, ray_end) = extended_shape_ray::<A>(previous, border, rect);
    let path_coordinate = |value: f64| ((value * 10_000.0) as f32 as f64 / 10_000.0).round();
    let points: Vec<_> = local_points
        .iter()
        .map(|(x, y)| Point {
            x: path_coordinate(rect.origin.x + x * scale.0),
            y: path_coordinate(rect.origin.y + y * scale.1),
        })
        .collect();
    rounded_closest::<A>(
        border,
        (0..points.len()).filter_map(|index| {
            let second = points[(index + 1) % points.len()];
            A::intersection(points[index], second, ray_start, ray_end)
        }),
    )
}

fn distance_to_segment<A: GeoArith>(point: Point, start: Point, end: Point) -> f64 {
    // Direct translation of geo.Point.DistanceToLine.  D2 calls the helper
    // with a finite ray segment even though the ellipse roots came from the
    // infinite line equation.
    let a = point.x - start.x;
    let b = point.y - start.y;
    let c = end.x - start.x;
    let d = end.y - start.y;
    let dot = A::fma(b, d, a * c);
    let len_sq = A::fma(d, d, c * c);
    let param = if len_sq != 0.0 { dot / len_sq } else { -1.0 };
    let (closest_x, closest_y) = if param < 0.0 {
        (start.x, start.y)
    } else if param > 1.0 {
        (end.x, end.y)
    } else {
        (A::fma(param, c, start.x), A::fma(param, d, start.y))
    };
    let dx = point.x - closest_x;
    let dy = point.y - closest_y;
    A::fma(dy, dy, dx * dx).sqrt()
}

fn ellipse_intersections<A: GeoArith>(
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
        distance_to_segment::<A>(point, Point { x: x1, y: y1 }, Point { x: x2, y: y2 }) < 0.0001
    };
    let mut intersections = Vec::new();
    let mut push = |point: Point| {
        intersections.push(Point {
            x: point.x + center.x,
            y: point.y + center.y,
        })
    };

    if x1 == x2 {
        let y = b * A::fma(-x1, x1, a2).sqrt() / a;
        // Go keeps a vertical root when it is the median of {y, y1, y2}.
        let on_vertical = |point: Point| {
            if y1 <= y2 {
                y1 <= point.y && point.y <= y2
            } else {
                y2 <= point.y && point.y <= y1
            }
        };
        for (index, point) in [Point { x: x1, y }, Point { x: x1, y: -y }]
            .into_iter()
            .enumerate()
        {
            // When y = 0 the second root duplicates the first.
            if (index == 0 || point.y != 0.0) && on_vertical(point) {
                push(point);
            }
        }
        return intersections;
    }

    let slope = (y2 - y1) / (x2 - x1);
    let intercept = A::fma(-slope, x1, y1);
    let denominator = A::fma(b, b, a2 * slope * slope);
    let root = (a2 * b2 * A::fma(-intercept, intercept, denominator)).sqrt();
    let first = Point {
        x: A::fma(-slope * intercept, a2, root) / denominator,
        y: A::fma(slope, root, intercept * b2) / denominator,
    };
    let second = Point {
        x: A::fma(-slope * intercept, a2, -root) / denominator,
        y: A::fma(-slope, root, intercept * b2) / denominator,
    };
    if on_segment(first) {
        push(first);
    }
    if first != second && on_segment(second) {
        push(second);
    }
    intersections
}

fn trace_ellipse<A: GeoArith>(rect: Rect, border: Point, previous: Point) -> (Point, bool) {
    let ray = extended_shape_ray::<A>(previous, border, rect);
    let center = Point {
        x: rect.origin.x + rect.size.width * 0.5,
        y: rect.origin.y + rect.size.height * 0.5,
    };
    rounded_closest::<A>(
        border,
        ellipse_intersections::<A>(center, rect.size.width * 0.5, rect.size.height * 0.5, ray),
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

fn bezier_coefficients<A: GeoArith>(p0: f64, p1: f64, p2: f64, p3: f64) -> [f64; 4] {
    [
        A::fma(-3.0, p2, A::fma(3.0, p1, -p0)) + p3,
        A::fma(3.0, p2, A::fma(-6.0, p1, 3.0 * p0)),
        A::fma(3.0, p1, -3.0 * p0),
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

/// Direct port of lib/geo/bezier.go cubicRoots, including its ordering and
/// precision behavior.
fn cubic_roots<A: GeoArith>(polynomial: [f64; 4]) -> [f64; 3] {
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

        let mut discriminant = A::fma(-4.0 * polynomial[1], polynomial[3], polynomial[2].powi(2));
        if precision_compare(discriminant, 0.0, PRECISION).is_ge() {
            discriminant = discriminant.sqrt();
            let mut roots = [
                -(discriminant + polynomial[2]) / (2.0 * polynomial[1]),
                (discriminant - polynomial[2]) / (2.0 * polynomial[1]),
                -1.0,
            ];
            // Go returns from inside this loop after validating only root zero.
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
    let q = A::fma(3.0, b, -a.powi(2)) / 9.0;
    let r = (A::fma(-27.0, c, 9.0 * a * b) - 2.0 * a.powi(3)) / 54.0;
    let discriminant = q.powi(3) + r.powi(2);
    let mut roots;
    if precision_compare(discriminant, 0.0, PRECISION).is_ge() {
        let signed_cube_root = |value: f64| {
            let sign = if value < 0.0 { -1.0 } else { 1.0 };
            sign * value.abs().powf(1.0 / 3.0)
        };
        let s = signed_cube_root(r + discriminant.sqrt());
        let t = signed_cube_root(r - discriminant.sqrt());
        let other = A::fma(-0.5, s + t, -a / 3.0);
        roots = [-a / 3.0 + (s + t), other, other];
        let imaginary = (3.0_f64).sqrt() * (s - t).abs() / 2.0;
        if !precision_compare(imaginary, 0.0, PRECISION).is_eq() {
            roots[1] = -1.0;
            roots[2] = -1.0;
        }
    } else {
        let theta = (r / (-q.powi(3)).sqrt()).acos();
        let scale = 2.0 * (-q).sqrt();
        roots = [0.0, 2.0, 4.0].map(|turns: f64| {
            A::fma(
                scale,
                ((theta + turns * std::f64::consts::PI) / 3.0).cos(),
                -(a / 3.0),
            )
        });
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

/// lib/geo/bezier.go ComputeIntersections.
fn cubic_line_intersections<A: GeoArith>(curve: [Point; 4], line: (Point, Point)) -> Vec<Point> {
    const PRECISION: f64 = 0.0001;
    let line_a = line.1.y - line.0.y;
    let line_b = line.0.x - line.1.x;
    let line_c = A::fma(
        line.0.y,
        line.1.x - line.0.x,
        line.0.x * (line.0.y - line.1.y),
    );
    let x = bezier_coefficients::<A>(curve[0].x, curve[1].x, curve[2].x, curve[3].x);
    let y = bezier_coefficients::<A>(curve[0].y, curve[1].y, curve[2].y, curve[3].y);
    let roots = cubic_roots::<A>([
        A::fma(line_a, x[0], line_b * y[0]),
        A::fma(line_a, x[1], line_b * y[1]),
        A::fma(line_a, x[2], line_b * y[2]),
        A::fma(line_a, x[3], line_b * y[3]) + line_c,
    ]);
    let evaluate =
        |c: [f64; 4], t: f64| A::fma(c[2], t, A::fma(c[0] * t * t, t, c[1] * t * t)) + c[3];
    let mut intersections = Vec::new();
    for parameter in roots {
        let point = Point {
            x: evaluate(x, parameter),
            y: evaluate(y, parameter),
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

fn shape_path_intersections<A: GeoArith>(path: ShapePath, ray: (Point, Point)) -> Vec<Point> {
    let mut intersections = Vec::new();
    for segment in path.segments {
        match segment {
            ShapePathSegment::Line(start, end) => {
                intersections.extend(A::intersection(start, end, ray.0, ray.1))
            }
            ShapePathSegment::Cubic(curve) => {
                intersections.extend(cubic_line_intersections::<A>(curve, ray))
            }
        }
    }
    intersections
}

fn trace_shape_path<A: GeoArith>(
    rect: Rect,
    border: Point,
    previous: Point,
    path: ShapePath,
) -> (Point, bool) {
    let ray = extended_shape_ray::<A>(previous, border, rect);
    rounded_closest::<A>(border, shape_path_intersections::<A>(path, ray))
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

fn trace_c4_person<A: GeoArith>(rect: Rect, border: Point, previous: Point) -> (Point, bool) {
    const HEAD_RADIUS_FACTOR: f64 = 0.22;

    let ray = extended_shape_ray::<A>(previous, border, rect);
    let head_radius = rect.size.width * HEAD_RADIUS_FACTOR;
    let head_center = Point {
        x: rect.origin.x + rect.size.width * 0.5,
        y: rect.origin.y + head_radius,
    };
    let mut intersections = shape_path_intersections::<A>(c4_person_body_path(rect), ray);
    intersections.extend(ellipse_intersections::<A>(
        head_center,
        head_radius,
        head_radius,
        ray,
    ));
    rounded_closest::<A>(border, intersections)
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

/// `None` for rectangular shapes, which D2 never traces.
fn trace_shape<A: GeoArith>(
    shape: ShapeKind,
    rect: Rect,
    border: Point,
    previous: Point,
) -> Option<(Point, bool)> {
    let path =
        |path: fn(Rect) -> ShapePath| trace_shape_path::<A>(rect, border, previous, path(rect));
    let pixels =
        |points: &[(f64, f64)]| trace_polygon::<A>(rect, border, previous, points, (1.0, 1.0));
    Some(match shape {
        ShapeKind::Rectangle
        | ShapeKind::Square
        | ShapeKind::RealSquare
        | ShapeKind::SqlTable
        | ShapeKind::Class
        | ShapeKind::Text
        | ShapeKind::Code
        | ShapeKind::Image => return None,
        ShapeKind::Circle | ShapeKind::Oval => trace_ellipse::<A>(rect, border, previous),
        ShapeKind::Cloud => path(cloud_path),
        ShapeKind::Diamond => path(diamond_path),
        ShapeKind::Hexagon => path(hexagon_path),
        ShapeKind::Parallelogram => {
            // D2 keeps the 26px wedge for every box wider than the wedge;
            // only collapsed boxes use half their width.
            let wedge_width = if rect.size.width <= 26.0 {
                rect.size.width / 2.0
            } else {
                26.0
            };
            // shape_parallelogram.go parallelogramPath, whose final
            // L(0, height) repeats the previous vertex before closing.
            pixels(&[
                (wedge_width, 0.0),
                (rect.size.width, 0.0),
                (rect.size.width - wedge_width, rect.size.height),
                (0.0, rect.size.height),
                (0.0, rect.size.height),
            ])
        }
        ShapeKind::Step => {
            let wedge_width = if rect.size.width <= 35.0 {
                rect.size.width / 2.0
            } else {
                35.0
            };
            pixels(&[
                (0.0, 0.0),
                (rect.size.width - wedge_width, 0.0),
                (rect.size.width, rect.size.height / 2.0),
                (rect.size.width - wedge_width, rect.size.height),
                (0.0, rect.size.height),
                (wedge_width, rect.size.height / 2.0),
            ])
        }
        ShapeKind::Callout => path(callout_path),
        ShapeKind::Cylinder => path(cylinder_path),
        ShapeKind::Queue => path(queue_path),
        ShapeKind::Person => path(person_path),
        ShapeKind::StoredData => path(stored_data_path),
        ShapeKind::Page => path(page_outer_path),
        ShapeKind::Document => path(document_path),
        ShapeKind::Package => path(package_path),
        ShapeKind::C4Person => trace_c4_person::<A>(rect, border, previous),
    })
}

/// Go `shape.TraceToShapeBorder` as dagre and ELK use it: the traced point,
/// or the border point itself for rectangular shapes.
pub fn trace_shape_border(shape: ShapeKind, rect: Rect, border: Point, previous: Point) -> Point {
    trace_shape::<GoNative>(shape, rect, border, previous).map_or(border, |(point, _)| point)
}

/// Result of [`trace_to_shape_border`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ShapeBorderTrace {
    /// Rectangular shapes return the border point itself (Go returns the
    /// same pointer).
    Unchanged,
    /// A new rounded point. `intersected` is false when the ray missed the
    /// perimeter; Go then rounds the border point after truncating it to
    /// float32 in place.
    Traced { point: Point, intersected: bool },
}

/// Go-exact lib/shape.TraceToShapeBorder for this host's architecture.
#[inline]
pub fn trace_to_shape_border(
    shape: ShapeKind,
    rect: Rect,
    border: Point,
    previous: Point,
) -> ShapeBorderTrace {
    trace_shape::<GoNative>(shape, rect, border, previous)
        .map_or(ShapeBorderTrace::Unchanged, |(point, intersected)| {
            ShapeBorderTrace::Traced { point, intersected }
        })
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

    use super::{ShapeBorderTrace, trace_shape_border, trace_to_shape_border};
    use crate::{Point, Rect, ShapeKind, Size};

    #[test]
    fn dagre_and_elk_tracing_is_the_go_trace() {
        // Dagre and ELK clip edges with Go's shape.TraceToShapeBorder, the same
        // function the renderer ports: rectangles keep the border point, other
        // shapes take the rounded closest intersection (or the rounded border
        // point when the ray misses).
        let rect = Rect {
            origin: Point { x: 10.0, y: 20.0 },
            size: Size {
                width: 26.0,
                height: 74.0,
            },
        };
        let border = Point { x: 23.0, y: 20.0 };
        for previous in [
            Point { x: 23.0, y: -40.0 },
            Point { x: 60.0, y: -40.0 },
            Point { x: 23.0, y: 20.0 },
        ] {
            for shape in [
                ShapeKind::Rectangle,
                ShapeKind::Oval,
                ShapeKind::Circle,
                ShapeKind::Parallelogram,
                ShapeKind::Step,
                ShapeKind::Cloud,
                ShapeKind::Diamond,
                ShapeKind::Person,
                ShapeKind::C4Person,
            ] {
                let expected = match trace_to_shape_border(shape, rect, border, previous) {
                    ShapeBorderTrace::Unchanged => border,
                    ShapeBorderTrace::Traced { point, .. } => point,
                };
                let actual = trace_shape_border(shape, rect, border, previous);
                assert_eq!(
                    (actual.x.to_bits(), actual.y.to_bits()),
                    (expected.x.to_bits(), expected.y.to_bits()),
                    "{shape:?} from {previous:?}"
                );
            }
        }
    }

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

/// Pins Go lib/shape.TraceToShapeBorder on both architectures. Expected
/// points come from the pinned Go source run on arm64 twice: as compiled, and
/// with `-gcflags=all=-d=fmahash=n`, which disables multiply-add fusion and so
/// reproduces amd64 (the amd64 build of lib/geo has no FMA instruction).
#[cfg(test)]
mod go_architecture_contract {
    use super::*;
    use crate::Size;

    struct GoArm64;
    impl GeoArith for GoArm64 {
        fn fma(x: f64, y: f64, z: f64) -> f64 {
            x.mul_add(y, z)
        }
        fn intersection(a: Point, b: Point, c: Point, d: Point) -> Option<Point> {
            // FMUL then FMSUB: the second product is fused into the difference.
            intersection_with(a, b, c, d, |p, q, r, s| (-r).mul_add(s, p * q))
        }
    }
    struct GoAmd64;
    impl GeoArith for GoAmd64 {
        fn fma(x: f64, y: f64, z: f64) -> f64 {
            x * y + z
        }
        fn intersection(a: Point, b: Point, c: Point, d: Point) -> Option<Point> {
            intersection_with(a, b, c, d, |p, q, r, s| p * q - r * s)
        }
    }

    fn rect(x: f64, y: f64, width: f64, height: f64) -> Rect {
        Rect {
            origin: Point { x, y },
            size: Size { width, height },
        }
    }
    fn traced<A: GeoArith>(shape: ShapeKind, rect: Rect, border: Point, previous: Point) -> Point {
        trace_shape::<A>(shape, rect, border, previous).unwrap().0
    }

    /// (shape, [x, y, width, height, border x, border y, previous x,
    /// previous y], arm64 result, amd64 result)
    const CASES: [(ShapeKind, [f64; 8], (f64, f64), (f64, f64)); 13] = [
        (
            ShapeKind::Cloud,
            [207.0, 368.5, 229.0, 209.0, 214.5, 577.0, 256.5, 577.0],
            (246.0, 577.0),
            (215.0, 577.0),
        ),
        (
            ShapeKind::Callout,
            [184.0, 996.5, 208.0, 250.0, 320.0, 1076.5, 284.5, 1041.0],
            (392.0, 1148.0),
            (392.0, 1149.0),
        ),
        (
            ShapeKind::Page,
            [157.5, 163.5, 208.0, 239.0, 161.0, 214.5, 209.0, 390.5],
            (158.0, 204.0),
            (158.0, 203.0),
        ),
        (
            ShapeKind::Diamond,
            [614.0, 291.0, 21.0, 62.0, 634.0, 321.0, 526.5, 170.0],
            (620.0, 302.0),
            (634.0, 321.0),
        ),
        (
            ShapeKind::C4Person,
            [166.0, 393.5, 219.0, 317.0, 384.0, 627.5, 210.0, 208.0],
            (384.0, 627.0),
            (384.0, 628.0),
        ),
        (
            ShapeKind::Cylinder,
            [108.0, 357.5, 83.0, 308.0, 108.0, 426.5, 108.0, 350.5],
            (108.0, 642.0),
            (108.0, 427.0),
        ),
        (
            ShapeKind::StoredData,
            [200.0, 609.0, 211.0, 50.0, 296.0, 659.0, 233.5, 659.0],
            (411.0, 659.0),
            (296.0, 659.0),
        ),
        (
            ShapeKind::Hexagon,
            [905.5, 589.0, 171.0, 163.0, 916.5, 690.5, 1093.5, 817.0],
            (916.0, 690.0),
            (917.0, 691.0),
        ),
        (
            ShapeKind::Person,
            [159.5, 221.0, 182.0, 271.0, 315.5, 492.0, 312.5, 341.0],
            (316.0, 492.0),
            (315.0, 492.0),
        ),
        (
            ShapeKind::Document,
            [909.0, 581.5, 258.0, 60.0, 991.0, 584.0, 1167.0, 520.0],
            (997.0, 582.0),
            (996.0, 582.0),
        ),
        (
            ShapeKind::Parallelogram,
            [844.5, 144.5, 185.0, 44.0, 974.5, 179.0, 824.5, 229.0],
            (945.0, 189.0),
            (944.0, 189.0),
        ),
        (
            ShapeKind::Step,
            [342.5, 535.0, 272.0, 45.0, 511.5, 580.0, 410.0, 531.5],
            (512.0, 580.0),
            (511.0, 580.0),
        ),
        (
            ShapeKind::Queue,
            [378.0, 709.5, 320.0, 105.0, 601.5, 813.5, 692.5, 768.0],
            (599.0, 815.0),
            (598.0, 815.0),
        ),
    ];

    #[test]
    fn traces_follow_go_multiply_add_fusion_per_architecture() {
        for (shape, [x, y, w, h, bx, by, px, py], arm64, amd64) in CASES {
            let (rect, border, previous) = (
                rect(x, y, w, h),
                Point { x: bx, y: by },
                Point { x: px, y: py },
            );
            let arm64 = Point {
                x: arm64.0,
                y: arm64.1,
            };
            let amd64 = Point {
                x: amd64.0,
                y: amd64.1,
            };
            assert_eq!(
                traced::<GoArm64>(shape, rect, border, previous),
                arm64,
                "{shape:?} arm64"
            );
            assert_eq!(
                traced::<GoAmd64>(shape, rect, border, previous),
                amd64,
                "{shape:?} amd64"
            );
            // On amd64 some rays miss without fused multiply-add (Cloud
            // returns the rounded border point), so the native trace must
            // match the emulated architecture's intersection flag too.
            let (native, intersected) = if cfg!(target_arch = "aarch64") {
                (
                    arm64,
                    trace_shape::<GoArm64>(shape, rect, border, previous)
                        .unwrap()
                        .1,
                )
            } else {
                (
                    amd64,
                    trace_shape::<GoAmd64>(shape, rect, border, previous)
                        .unwrap()
                        .1,
                )
            };
            assert_eq!(
                trace_to_shape_border(shape, rect, border, previous),
                ShapeBorderTrace::Traced {
                    point: native,
                    intersected
                },
                "{shape:?} native"
            );
        }
    }

    #[test]
    fn huge_circle_ellipse_roots_follow_go_fusion_per_architecture() {
        let source = rect(0.0, 0.0, 1e20, 1e20);
        let target = rect(300.0, 0.0, 1e20, 1e20);
        let start = traced::<GoAmd64>(
            ShapeKind::Circle,
            source,
            Point { x: 100.0, y: 0.0 },
            Point { x: 300.0, y: 100.0 },
        );
        assert_eq!(start, Point { x: 100.0, y: 0.0 });
        assert_eq!(
            traced::<GoArm64>(
                ShapeKind::Circle,
                source,
                Point { x: 100.0, y: 0.0 },
                Point { x: 300.0, y: 100.0 }
            ),
            start
        );
        let end = |point: Point| (point.x.to_bits(), point.y.to_bits());
        assert_eq!(
            end(traced::<GoArm64>(
                ShapeKind::Circle,
                target,
                Point { x: 300.0, y: 100.0 },
                start
            )),
            (0x43f1_58e4_6000_0000, 0x43e1_58e4_6000_0000)
        );
        assert_eq!(
            end(traced::<GoAmd64>(
                ShapeKind::Circle,
                target,
                Point { x: 300.0, y: 100.0 },
                start
            )),
            (300.0f64.to_bits(), 100.0f64.to_bits())
        );
    }

    #[test]
    fn go_trace_keeps_rectangles_and_reports_missed_rays() {
        let square = rect(0.0, 0.0, 10.0, 10.0);
        let border = Point { x: 10.0, y: 5.0 };
        assert_eq!(
            trace_to_shape_border(ShapeKind::Square, square, border, Point { x: 20.0, y: 5.0 }),
            ShapeBorderTrace::Unchanged
        );
        assert_eq!(
            trace_to_shape_border(
                ShapeKind::Circle,
                rect(0.0, 0.0, 100.0, 100.0),
                Point {
                    x: 20.123456789,
                    y: 10.987654321
                },
                Point {
                    x: 20.123456789,
                    y: 10.987654321
                },
            ),
            ShapeBorderTrace::Traced {
                point: Point { x: 20.0, y: 11.0 },
                intersected: false
            }
        );
    }

    #[test]
    fn step_quantizes_svg_path_vertices_before_intersection() {
        assert_eq!(
            trace_to_shape_border(
                ShapeKind::Step,
                rect(2021.0, 2446.0, 116.0, 101.0),
                Point {
                    x: 2038.0,
                    y: 2496.0
                },
                Point {
                    x: 598.0,
                    y: 2496.0
                },
            ),
            ShapeBorderTrace::Traced {
                point: Point {
                    x: 2055.0,
                    y: 2496.0
                },
                intersected: true
            }
        );
    }

    #[test]
    fn short_nonzero_ray_is_extended_with_exact_axis_selection() {
        assert_eq!(
            trace_to_shape_border(
                ShapeKind::Diamond,
                rect(0.0, 0.0, 100.0, 10.0),
                Point { x: 0.0, y: 0.0 },
                Point {
                    x: -5.0e-10,
                    y: -1.0e-10
                },
            ),
            ShapeBorderTrace::Traced {
                point: Point { x: 17.0, y: 3.0 },
                intersected: true
            }
        );
    }

    #[test]
    fn vertical_ellipse_intersections_use_exact_segment_bounds() {
        let center = Point { x: 0.0, y: 0.0 };
        let beyond_endpoint = ellipse_intersections::<GoNative>(
            center,
            1.0,
            1.0,
            (Point { x: 0.0, y: 1.00005 }, Point { x: 0.0, y: 2.0 }),
        );
        let on_endpoint = ellipse_intersections::<GoNative>(
            center,
            1.0,
            1.0,
            (Point { x: 0.0, y: 1.0 }, Point { x: 0.0, y: 2.0 }),
        );
        assert!(beyond_endpoint.is_empty());
        assert_eq!(on_endpoint, vec![Point { x: 0.0, y: 1.0 }]);
    }

    #[test]
    fn border_selection_uses_go_axis_distance_when_squares_overflow() {
        let (closest, intersected) = rounded_closest::<GoNative>(
            Point { x: 0.0, y: 0.0 },
            [Point { x: 0.0, y: 9.0e199 }, Point { x: 1.0e200, y: 0.0 }],
        );
        assert!(intersected);
        assert_eq!(closest.x, 0.0);
        assert!(closest.y.is_infinite());
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
        let ray = extended_shape_ray::<GoNative>(previous, corner, rect);
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
