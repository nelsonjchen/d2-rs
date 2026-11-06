// SPDX-License-Identifier: MPL-2.0
//! d2scene/path.go: ARM64 contraction follows path.go:222–431;
//! products by 0.5 are exact and written plainly.

use std::f64::consts::PI;

use d2_rs_render::go_float::{go_fmadd, go_fmsub, gomath};
use serde::{Deserialize, Serialize};

use crate::geometry::{Bounds, Matrix, Point, finite, finite_point, go_max};
use crate::paint::{Paint, Stroke};

crate::go_enum!(FillRule { NON_ZERO = 0, EVEN_ODD = 1 });
crate::go_enum!(PathCommandKind { MOVE = 0, LINE = 1, QUADRATIC = 2, CUBIC = 3, ARC = 4, CLOSE = 5 });

/// Absolute typed geometry: P1 is a move/line endpoint; P1/P2 are a
/// quadratic's control/end; P1/P2/P3 a cubic's controls/end; P1 an arc's end
/// (rotation in radians).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct PathCommand {
    pub kind: PathCommandKind,
    #[serde(rename = "P1")]
    pub p1: Point,
    #[serde(rename = "P2")]
    pub p2: Point,
    #[serde(rename = "P3")]
    pub p3: Point,
    pub radius_x: f64,
    pub radius_y: f64,
    pub rotation: f64,
    pub large_arc: bool,
    pub sweep: bool,
}

impl PathCommand {
    pub fn move_to(x: f64, y: f64) -> Self {
        Self {
            kind: PathCommandKind::MOVE,
            p1: Point::new(x, y),
            ..Default::default()
        }
    }
    pub fn line_to(x: f64, y: f64) -> Self {
        Self {
            kind: PathCommandKind::LINE,
            p1: Point::new(x, y),
            ..Default::default()
        }
    }
    pub fn quadratic_to(cx: f64, cy: f64, x: f64, y: f64) -> Self {
        Self {
            kind: PathCommandKind::QUADRATIC,
            p1: Point::new(cx, cy),
            p2: Point::new(x, y),
            ..Default::default()
        }
    }
    pub fn cubic_to(c1x: f64, c1y: f64, c2x: f64, c2y: f64, x: f64, y: f64) -> Self {
        Self {
            kind: PathCommandKind::CUBIC,
            p1: Point::new(c1x, c1y),
            p2: Point::new(c2x, c2y),
            p3: Point::new(x, y),
            ..Default::default()
        }
    }
    #[allow(clippy::too_many_arguments)]
    pub fn arc_to(
        rx: f64,
        ry: f64,
        rotation: f64,
        large_arc: bool,
        sweep: bool,
        x: f64,
        y: f64,
    ) -> Self {
        Self {
            kind: PathCommandKind::ARC,
            p1: Point::new(x, y),
            radius_x: rx,
            radius_y: ry,
            rotation,
            large_arc,
            sweep,
            ..Default::default()
        }
    }
    pub fn close() -> Self {
        Self {
            kind: PathCommandKind::CLOSE,
            ..Default::default()
        }
    }

    fn validate(&self) -> Result<(), String> {
        let check = |p: Point| {
            if finite_point(p) {
                Ok(())
            } else {
                Err("non-finite point".to_owned())
            }
        };
        match self.kind {
            PathCommandKind::MOVE | PathCommandKind::LINE => check(self.p1),
            PathCommandKind::QUADRATIC => check(self.p1).and(check(self.p2)),
            PathCommandKind::CUBIC => check(self.p1).and(check(self.p2)).and(check(self.p3)),
            PathCommandKind::ARC => {
                check(self.p1)?;
                if !finite(self.radius_x) || !finite(self.radius_y) || !finite(self.rotation) {
                    return Err("non-finite arc".into());
                }
                Ok(())
            }
            PathCommandKind::CLOSE => Ok(()),
            _ => Err(format!("unknown kind {}", self.kind.0)),
        }
    }
}

/// Typed path geometry and a paintable primitive.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Path {
    pub commands: Vec<PathCommand>,
    pub fill_rule: FillRule,
    pub fill: Option<Paint>,
    pub stroke: Option<Stroke>,
}

impl Path {
    /// Exact analytic bounds of the centerline geometry.
    pub fn geometry_bounds(&self) -> Result<Bounds, String> {
        self.transformed_geometry_bounds(Matrix::identity())
    }

    pub(crate) fn transformed_geometry_bounds(&self, m: Matrix) -> Result<Bounds, String> {
        if !m.is_finite() {
            return Err("d2scene: non-finite path transform".into());
        }
        let mut bounds = Bounds::default();
        let mut current = Point::default();
        let mut subpath_start = Point::default();
        let mut have_current = false;
        for (i, command) in self.commands.iter().enumerate() {
            command
                .validate()
                .map_err(|e| format!("d2scene: path command {i}: {e}"))?;
            let before = |name: &str| format!("d2scene: path command {i}: {name} before move");
            match command.kind {
                PathCommandKind::MOVE => {
                    current = command.p1;
                    subpath_start = current;
                    have_current = true;
                }
                PathCommandKind::LINE => {
                    if !have_current {
                        return Err(before("line"));
                    }
                    bounds = bounds.union(Bounds::from_points(&[
                        m.point(current),
                        m.point(command.p1),
                    ]));
                    current = command.p1;
                }
                PathCommandKind::QUADRATIC => {
                    if !have_current {
                        return Err(before("quadratic"));
                    }
                    bounds = bounds.union(quadratic_bounds(
                        m.point(current),
                        m.point(command.p1),
                        m.point(command.p2),
                    ));
                    current = command.p2;
                }
                PathCommandKind::CUBIC => {
                    if !have_current {
                        return Err(before("cubic"));
                    }
                    bounds = bounds.union(cubic_bounds(
                        m.point(current),
                        m.point(command.p1),
                        m.point(command.p2),
                        m.point(command.p3),
                    ));
                    current = command.p3;
                }
                PathCommandKind::ARC => {
                    if !have_current {
                        return Err(before("arc"));
                    }
                    let arc = arc_bounds(current, command, m)
                        .map_err(|e| format!("d2scene: path command {i}: {e}"))?;
                    bounds = bounds.union(arc);
                    current = command.p1;
                }
                PathCommandKind::CLOSE => {
                    if !have_current {
                        return Err(before("close"));
                    }
                    bounds = bounds.union(Bounds::from_points(&[
                        m.point(current),
                        m.point(subpath_start),
                    ]));
                    current = subpath_start;
                }
                _ => unreachable!("validated command kind"),
            }
        }
        if !bounds.is_finite() {
            return Err("d2scene: non-finite path bounds".into());
        }
        Ok(bounds)
    }
}

fn quadratic_bounds(p0: Point, p1: Point, p2: Point) -> Bounds {
    let mut b = Bounds::from_points(&[p0, p2]);
    if let Some(t) = quadratic_extremum(p0.x, p1.x, p2.x) {
        b = b.include(quadratic_point(p0, p1, p2, t));
    }
    if let Some(t) = quadratic_extremum(p0.y, p1.y, p2.y) {
        b = b.include(quadratic_point(p0, p1, p2, t));
    }
    b
}

fn quadratic_extremum(p0: f64, p1: f64, p2: f64) -> Option<f64> {
    let denominator = p0 - 2.0 * p1 + p2;
    if denominator == 0.0 {
        return None;
    }
    let t = (p0 - p1) / denominator;
    (t > 0.0 && t < 1.0).then_some(t)
}

fn quadratic_point(p0: Point, p1: Point, p2: Point, t: f64) -> Point {
    let u = 1.0 - t;
    // ARM64 path.go:222–223: two FMADDD per coordinate.
    let at = |a: f64, b: f64, c: f64| go_fmadd(t * t, c, go_fmadd(2.0 * u * t, b, u * u * a));
    Point::new(at(p0.x, p1.x, p2.x), at(p0.y, p1.y, p2.y))
}

fn cubic_bounds(p0: Point, p1: Point, p2: Point, p3: Point) -> Bounds {
    let mut b = Bounds::from_points(&[p0, p3]);
    for t in cubic_extrema(p0.x, p1.x, p2.x, p3.x) {
        b = b.include(cubic_point(p0, p1, p2, p3, t));
    }
    for t in cubic_extrema(p0.y, p1.y, p2.y, p3.y) {
        b = b.include(cubic_point(p0, p1, p2, p3, t));
    }
    b
}

fn cubic_extrema(p0: f64, p1: f64, p2: f64, p3: f64) -> Vec<f64> {
    // ARM64 path.go:240: FMADDD then FMSUBD.
    let a = go_fmsub(3.0, p2, go_fmadd(3.0, p1, -p0)) + p3;
    let b = 2.0 * (p0 - 2.0 * p1 + p2);
    let c = p1 - p0;
    roots_in_unit_interval(a, b, c)
}

fn roots_in_unit_interval(a: f64, b: f64, c: f64) -> Vec<f64> {
    if a == 0.0 {
        if b == 0.0 {
            return Vec::new();
        }
        let t = -c / b;
        return if t > 0.0 && t < 1.0 {
            vec![t]
        } else {
            Vec::new()
        };
    }
    // ARM64 path.go:257: FMSUBD.
    let discriminant = go_fmsub(4.0 * a, c, b * b);
    if discriminant < 0.0 {
        return Vec::new();
    }
    let root = discriminant.sqrt();
    let t0 = (-b - root) / (2.0 * a);
    let t1 = (-b + root) / (2.0 * a);
    let mut roots = Vec::with_capacity(2);
    if t0 > 0.0 && t0 < 1.0 {
        roots.push(t0);
    }
    if t1 > 0.0 && t1 < 1.0 && t1 != t0 {
        roots.push(t1);
    }
    roots
}

fn cubic_point(p0: Point, p1: Point, p2: Point, p3: Point, t: f64) -> Point {
    let u = 1.0 - t;
    let (uu, tt) = (u * u, t * t);
    // ARM64 path.go:278–279: three FMADDD per coordinate.
    let at = |a: f64, b: f64, c: f64, d: f64| {
        go_fmadd(
            tt * t,
            d,
            go_fmadd(3.0 * u * tt, c, go_fmadd(3.0 * uu * t, b, uu * u * a)),
        )
    };
    Point::new(at(p0.x, p1.x, p2.x, p3.x), at(p0.y, p1.y, p2.y, p3.y))
}

/// SVG endpoint arc geometry in center parameterization.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CenterArc {
    center: Point,
    u: Point,
    v: Point,
    start: f64,
    delta: f64,
}

impl CenterArc {
    pub fn start_angle(&self) -> f64 {
        self.start
    }
    pub fn delta_angle(&self) -> f64 {
        self.delta
    }
    pub fn point_at(&self, theta: f64) -> Point {
        let (s, c) = (gomath::sin(theta), gomath::cos(theta));
        // ARM64 path.go:299.
        Point::new(
            go_fmadd(self.v.x, s, go_fmadd(self.u.x, c, self.center.x)),
            go_fmadd(self.v.y, s, go_fmadd(self.u.y, c, self.center.y)),
        )
    }
    /// Conservative second-derivative bound after the linear part of m.
    pub fn transformed_radius_bound(&self, m: Matrix) -> f64 {
        let u = m.vector(self.u);
        let v = m.vector(self.v);
        gomath::hypot(gomath::hypot(u.x, u.y), gomath::hypot(v.x, v.y))
    }
}

fn arc_bounds(start: Point, command: &PathCommand, m: Matrix) -> Result<Bounds, String> {
    let end = command.p1;
    let transformed_start = m.point(start);
    let transformed_end = m.point(end);
    if start == end {
        return Ok(Bounds::from_points(&[transformed_start]));
    }
    let Some(arc) = endpoint_arc_to_center(start, command)? else {
        return Ok(Bounds::from_points(&[transformed_start, transformed_end]));
    };
    let center = m.point(arc.center);
    let u = m.vector(arc.u);
    let v = m.vector(arc.v);
    let point_at = |theta: f64| {
        let (s, c) = (gomath::sin(theta), gomath::cos(theta));
        // ARM64 path.go:331.
        Point::new(
            go_fmadd(v.x, s, go_fmadd(u.x, c, center.x)),
            go_fmadd(v.y, s, go_fmadd(u.y, c, center.y)),
        )
    };
    let mut b = Bounds::from_points(&[
        transformed_start,
        transformed_end,
        point_at(arc.start),
        point_at(arc.start + arc.delta),
    ]);
    let candidates = [
        gomath::atan2(v.x, u.x),
        gomath::atan2(v.x, u.x) + PI,
        gomath::atan2(v.y, u.y),
        gomath::atan2(v.y, u.y) + PI,
    ];
    for theta in candidates {
        if angle_on_arc(theta, arc.start, arc.delta) {
            b = b.include(point_at(theta));
        }
    }
    if !b.is_finite() {
        return Err("arc bounds are outside the numeric domain".into());
    }
    Ok(b)
}

/// SVG 2 endpoint-to-center conversion; None for identical endpoints or a
/// zero radius.
pub fn endpoint_arc_to_center(
    start: Point,
    command: &PathCommand,
) -> Result<Option<CenterArc>, String> {
    if command.kind != PathCommandKind::ARC {
        return Err("endpoint arc conversion requires an arc command".into());
    }
    if !finite_point(start) {
        return Err("non-finite arc start".into());
    }
    command.validate()?;
    let end = command.p1;
    if start == end {
        return Ok(None);
    }
    let mut rx = command.radius_x.abs();
    let mut ry = command.radius_y.abs();
    if rx == 0.0 || ry == 0.0 {
        return Ok(None);
    }
    let (sin_phi, cos_phi) = (gomath::sin(command.rotation), gomath::cos(command.rotation));
    let dx = start.x / 2.0 - end.x / 2.0;
    let dy = start.y / 2.0 - end.y / 2.0;
    // ARM64 path.go:380–381.
    let x_prime = go_fmadd(sin_phi, dy, cos_phi * dx);
    let y_prime = go_fmadd(cos_phi, dy, -sin_phi * dx);
    if !finite(x_prime) || !finite(y_prime) {
        return Err("arc is outside the numeric domain".into());
    }
    let (normalized_x, normalized_y, ratio_mantissa, ratio_exponent) =
        normalized_radius_ratios(x_prime, y_prime, rx, ry)?;
    let ratio_ge_one = ratio_exponent > 0 || ratio_exponent == 0 && ratio_mantissa >= 1.0;
    let mut ratio_length = 1.0;
    if ratio_ge_one {
        rx = multiply_binary_scale(rx, ratio_mantissa, ratio_exponent);
        ry = multiply_binary_scale(ry, ratio_mantissa, ratio_exponent);
        if !finite(rx) || !finite(ry) {
            return Err("arc corrected radii are outside the numeric domain".into());
        }
    } else {
        ratio_length = gomath::ldexp(ratio_mantissa, ratio_exponent);
    }
    // ARM64 path.go:401: FMSUBD.
    let mut root = go_max(0.0, go_fmsub(ratio_length, ratio_length, 1.0)).sqrt();
    if command.large_arc == command.sweep {
        root = -root;
    }
    let cx_prime = (root * rx) * normalized_y;
    let cy_prime = (-root * ry) * normalized_x;
    // ARM64 path.go:409–410: FMSUBD/FMADDD, then the exact halves.
    let center = Point::new(
        go_fmsub(sin_phi, cy_prime, cos_phi * cx_prime) + start.x / 2.0 + end.x / 2.0,
        go_fmadd(cos_phi, cy_prime, sin_phi * cx_prime) + start.y / 2.0 + end.y / 2.0,
    );
    let u = Point::new(rx * cos_phi, rx * sin_phi);
    let v = Point::new(-ry * sin_phi, ry * cos_phi);
    if !finite_point(center) || !finite_point(u) || !finite_point(v) {
        return Err("arc is outside the numeric domain".into());
    }
    let (mut endpoint_x, mut endpoint_y) = (normalized_x, normalized_y);
    if !ratio_ge_one {
        endpoint_x *= ratio_length;
        endpoint_y *= ratio_length;
    }
    let cx_ratio = cx_prime / rx;
    let cy_ratio = cy_prime / ry;
    let ux = endpoint_x - cx_ratio;
    let uy = endpoint_y - cy_ratio;
    let vx = -endpoint_x - cx_ratio;
    let vy = -endpoint_y - cy_ratio;
    let start_angle = gomath::atan2(uy, ux);
    // ARM64 path.go:431.
    let mut delta = gomath::atan2(go_fmsub(uy, vx, ux * vy), go_fmadd(uy, vy, ux * vx));
    if command.sweep && delta < 0.0 {
        delta += 2.0 * PI;
    } else if !command.sweep && delta > 0.0 {
        delta -= 2.0 * PI;
    }
    if !finite(start_angle) || !finite(delta) {
        return Err("arc angles are outside the numeric domain".into());
    }
    Ok(Some(CenterArc {
        center,
        u,
        v,
        start: start_angle,
        delta,
    }))
}

fn normalized_radius_ratios(
    x_prime: f64,
    y_prime: f64,
    rx: f64,
    ry: f64,
) -> Result<(f64, f64, f64, i32), String> {
    let x = ratio_mantissa_exponent(x_prime, rx);
    let y = ratio_mantissa_exponent(y_prime, ry);
    if x.is_none() && y.is_none() {
        return Err("arc endpoint ratio is zero".into());
    }
    let (x_mantissa, x_exponent) = x.unwrap_or((0.0, 0));
    let (y_mantissa, y_exponent) = y.unwrap_or((0.0, 0));
    let mut max_exponent = x_exponent;
    if x.is_none() || y.is_some() && y_exponent > max_exponent {
        max_exponent = y_exponent;
    }
    let x_scaled = if x.is_some() {
        gomath::ldexp(x_mantissa, x_exponent - max_exponent)
    } else {
        0.0
    };
    let y_scaled = if y.is_some() {
        gomath::ldexp(y_mantissa, y_exponent - max_exponent)
    } else {
        0.0
    };
    let norm = gomath::hypot(x_scaled, y_scaled);
    if norm == 0.0 || !finite(norm) {
        return Err("arc endpoint ratio is outside the numeric domain".into());
    }
    let (length_mantissa, length_exponent) = gomath::frexp(norm);
    Ok((
        x_scaled / norm,
        y_scaled / norm,
        length_mantissa,
        max_exponent + length_exponent,
    ))
}

fn ratio_mantissa_exponent(numerator: f64, denominator: f64) -> Option<(f64, i32)> {
    if numerator == 0.0 {
        return None;
    }
    let (numerator_mantissa, numerator_exponent) = gomath::frexp(numerator);
    let (denominator_mantissa, denominator_exponent) = gomath::frexp(denominator);
    let (ratio_mantissa, ratio_exponent) = gomath::frexp(numerator_mantissa / denominator_mantissa);
    Some((
        ratio_mantissa,
        numerator_exponent - denominator_exponent + ratio_exponent,
    ))
}

fn multiply_binary_scale(value: f64, mantissa: f64, exponent: i32) -> f64 {
    let (value_mantissa, value_exponent) = gomath::frexp(value);
    gomath::ldexp(value_mantissa * mantissa, value_exponent + exponent)
}

fn angle_on_arc(theta: f64, start: f64, delta: f64) -> bool {
    const TOLERANCE: f64 = 1e-12;
    if delta >= 0.0 {
        return positive_angle(theta - start) <= delta + TOLERANCE;
    }
    positive_angle(start - theta) <= -delta + TOLERANCE
}

fn positive_angle(theta: f64) -> f64 {
    // math.Mod is exact, like Rust's `%` on f64.
    let mut theta = theta % (2.0 * PI);
    if theta < 0.0 {
        theta += 2.0 * PI;
    }
    theta
}
