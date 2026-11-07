// SPDX-License-Identifier: MPL-2.0
// Source: D2 d2scene geometry.go/path.go/primitive.go/paint.go;
// d2svgimport/import.go validatePrimitiveBounds/validateImportedBounds.
use super::{
    svg_path::Command,
    svg_primitive::{Primitive, Stroke},
    svg_scene::Scene,
};
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Bounds {
    pub min: [f64; 2],
    pub max: [f64; 2],
    pub valid: bool,
}
fn min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else {
        a.min(b)
    }
}
fn max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else {
        a.max(b)
    }
}
impl Bounds {
    fn include(&mut self, p: [f64; 2]) {
        if !self.valid {
            self.min = p;
            self.max = p;
            self.valid = true;
        } else {
            for axis in 0..2 {
                self.min[axis] = min(self.min[axis], p[axis]);
                self.max[axis] = max(self.max[axis], p[axis]);
            }
        }
    }
    fn union(&mut self, b: Self) {
        if !self.valid {
            *self = b;
        } else if b.valid {
            self.include(b.min);
            self.include(b.max);
        }
    }
    fn points(points: &[[f64; 2]]) -> Self {
        let mut b = Self::default();
        for &p in points {
            b.include(p);
        }
        b
    }
    fn finite(self) -> bool {
        !self.valid || self.min.into_iter().chain(self.max).all(f64::is_finite)
    }
}
fn point(m: [f64; 6], p: [f64; 2]) -> [f64; 2] {
    [
        m[0].mul_add(p[0], m[2] * p[1]) + m[4],
        m[1].mul_add(p[0], m[3] * p[1]) + m[5],
    ]
}
fn vector(m: [f64; 6], p: [f64; 2]) -> [f64; 2] {
    [
        m[0].mul_add(p[0], m[2] * p[1]),
        m[1].mul_add(p[0], m[3] * p[1]),
    ]
}
fn finite(p: [f64; 2]) -> bool {
    p.iter().all(|v| v.is_finite())
}
pub fn max_scale(m: [f64; 6]) -> f64 {
    let maximum = max(max(m[0].abs(), m[1].abs()), max(m[2].abs(), m[3].abs()));
    if maximum == 0. || !maximum.is_finite() {
        return maximum;
    }
    let [a, b, c, d] = [
        m[0] / maximum,
        m[1] / maximum,
        m[2] / maximum,
        m[3] / maximum,
    ];
    let trace = d.mul_add(d, c.mul_add(c, b.mul_add(b, a * a)));
    let det = (-b).mul_add(c, a * d);
    let disc = max(0., (-4. * det).mul_add(det, trace * trace));
    maximum * max(0., (trace + disc.sqrt()) / 2.).sqrt()
}
fn expand(mut b: Bounds, stroke: Option<&Stroke>, m: [f64; 6]) -> Bounds {
    let Some(stroke) = stroke.filter(|stroke| stroke.width > 0.) else {
        return b;
    };
    if !b.valid {
        return b;
    }
    let mut factor = if stroke.cap == 2 {
        std::f64::consts::SQRT_2
    } else {
        1.
    };
    if stroke.join == 0 {
        let limit = if stroke.miter_limit <= 0. {
            4.
        } else {
            stroke.miter_limit
        };
        if limit > factor {
            factor = limit;
        }
    }
    let expansion = (stroke.width / 2. * factor) * max_scale(m);
    for axis in 0..2 {
        b.min[axis] -= expansion;
        b.max[axis] += expansion;
    }
    if b.min[0] > b.max[0] || b.min[1] > b.max[1] {
        return Bounds::default();
    }
    b
}
fn quadratic(p: [[f64; 2]; 3]) -> Bounds {
    let mut b = Bounds::points(&[p[0], p[2]]);
    for axis in 0..2 {
        let denominator = p[0][axis] - 2. * p[1][axis] + p[2][axis];
        if denominator == 0. {
            continue;
        }
        let t = (p[0][axis] - p[1][axis]) / denominator;
        if t > 0. && t < 1. {
            let u = 1. - t;
            b.include(std::array::from_fn(|i| {
                (t * t).mul_add(p[2][i], (2. * u * t).mul_add(p[1][i], u * u * p[0][i]))
            }));
        }
    }
    b
}
fn roots(a: f64, b: f64, c: f64) -> Vec<f64> {
    if a == 0. {
        if b == 0. {
            return Vec::new();
        }
        let t = -c / b;
        return if t > 0. && t < 1. {
            vec![t]
        } else {
            Vec::new()
        };
    }
    let discriminant = (-4. * a).mul_add(c, b * b);
    if discriminant < 0. {
        return Vec::new();
    }
    let s = discriminant.sqrt();
    let t0 = (-b - s) / (2. * a);
    let t1 = (-b + s) / (2. * a);
    let mut out = Vec::new();
    if t0 > 0. && t0 < 1. {
        out.push(t0);
    }
    if t1 > 0. && t1 < 1. && t1 != t0 {
        out.push(t1);
    }
    out
}
fn cubic(p: [[f64; 2]; 4]) -> Bounds {
    let mut b = Bounds::points(&[p[0], p[3]]);
    for axis in 0..2 {
        let a = -p[0][axis] + 3. * p[1][axis] - 3. * p[2][axis] + p[3][axis];
        let c = p[1][axis] - p[0][axis];
        for t in roots(a, 2. * (p[0][axis] - 2. * p[1][axis] + p[2][axis]), c) {
            let u = 1. - t;
            let uu = u * u;
            let tt = t * t;
            b.include(std::array::from_fn(|i| {
                (tt * t).mul_add(
                    p[3][i],
                    (3. * u * tt)
                        .mul_add(p[2][i], (3. * uu * t).mul_add(p[1][i], uu * u * p[0][i])),
                )
            }));
        }
    }
    b
}
fn ratio(n: f64, d: f64) -> Option<(f64, i32)> {
    if n == 0. {
        return None;
    }
    let (nm, ne) = libm::frexp(n);
    let (dm, de) = libm::frexp(d);
    let (rm, re) = libm::frexp(nm / dm);
    Some((rm, ne - de + re))
}
fn binary_scale(value: f64, mantissa: f64, exponent: i32) -> f64 {
    let (m, e) = libm::frexp(value);
    libm::ldexp(m * mantissa, e + exponent)
}
fn arc(start: [f64; 2], command: &Command, m: [f64; 6]) -> Result<Bounds, String> {
    let end = command.points[0];
    let ts = point(m, start);
    let te = point(m, end);
    if start == end {
        return Ok(Bounds::points(&[ts]));
    }
    let mut rx = command.radii[0].abs();
    let mut ry = command.radii[1].abs();
    if rx == 0. || ry == 0. {
        return Ok(Bounds::points(&[ts, te]));
    }
    let (s, c) = command.rotation.sin_cos();
    let dx = start[0] / 2. - end[0] / 2.;
    let dy = start[1] / 2. - end[1] / 2.;
    let xp = c.mul_add(dx, s * dy);
    let yp = (-s).mul_add(dx, c * dy);
    if !finite([xp, yp]) {
        return Err("arc is outside the numeric domain".into());
    }
    let xr = ratio(xp, rx);
    let yr = ratio(yp, ry);
    let exponent = match (xr, yr) {
        (None, None) => return Err("arc endpoint ratio is zero".into()),
        (Some((_, e)), None) | (None, Some((_, e))) => e,
        (Some((_, x)), Some((_, y))) => x.max(y),
    };
    let scaled = |r: Option<(f64, i32)>| r.map_or(0., |(a, e)| libm::ldexp(a, e - exponent));
    let x = scaled(xr);
    let y = scaled(yr);
    let norm = x.hypot(y);
    if norm == 0. || !norm.is_finite() {
        return Err("arc endpoint ratio is outside the numeric domain".into());
    }
    let x = x / norm;
    let y = y / norm;
    let (mantissa, e) = libm::frexp(norm);
    let exponent = exponent + e;
    let corrected = exponent > 0 || (exponent == 0 && mantissa >= 1.);
    let length = if corrected {
        rx = binary_scale(rx, mantissa, exponent);
        ry = binary_scale(ry, mantissa, exponent);
        if !finite([rx, ry]) {
            return Err("arc corrected radii are outside the numeric domain".into());
        }
        1.
    } else {
        libm::ldexp(mantissa, exponent)
    };
    let mut root = max(0., (-length).mul_add(length, 1.)).sqrt();
    if command.large_arc == command.sweep {
        root = -root;
    }
    let cx = (root * rx) * y;
    let cy = (-root * ry) * x;
    let center = [
        (-s).mul_add(cy, c * cx) + start[0] / 2. + end[0] / 2.,
        c.mul_add(cy, s * cx) + start[1] / 2. + end[1] / 2.,
    ];
    let u = [rx * c, rx * s];
    let v = [-ry * s, ry * c];
    if !finite(center) || !finite(u) || !finite(v) {
        return Err("arc is outside the numeric domain".into());
    }
    let (x, y) = if corrected {
        (x, y)
    } else {
        (x * length, y * length)
    };
    let ux = x - cx / rx;
    let uy = y - cy / ry;
    let vx = -x - cx / rx;
    let vy = -y - cy / ry;
    let angle = uy.atan2(ux);
    let mut delta = (-uy).mul_add(vx, ux * vy).atan2(uy.mul_add(vy, ux * vx));
    if command.sweep && delta < 0. {
        delta += 2. * std::f64::consts::PI;
    } else if !command.sweep && delta > 0. {
        delta -= 2. * std::f64::consts::PI;
    }
    if !finite([angle, delta]) {
        return Err("arc angles are outside the numeric domain".into());
    }
    let center = point(m, center);
    let u = vector(m, u);
    let v = vector(m, v);
    let at = |theta: f64| {
        let (s, c) = theta.sin_cos();
        [
            v[0].mul_add(s, u[0].mul_add(c, center[0])),
            v[1].mul_add(s, u[1].mul_add(c, center[1])),
        ]
    };
    let mut b = Bounds::points(&[ts, te, at(angle), at(angle + delta)]);
    let positive = |theta: f64| {
        let mut t = theta % (2. * std::f64::consts::PI);
        if t < 0. {
            t += 2. * std::f64::consts::PI;
        }
        t
    };
    for theta in [
        v[0].atan2(u[0]),
        v[0].atan2(u[0]) + std::f64::consts::PI,
        v[1].atan2(u[1]),
        v[1].atan2(u[1]) + std::f64::consts::PI,
    ] {
        if if delta >= 0. {
            positive(theta - angle) <= delta + 1e-12
        } else {
            positive(angle - theta) <= -delta + 1e-12
        } {
            b.include(at(theta));
        }
    }
    if !b.finite() {
        return Err("arc bounds are outside the numeric domain".into());
    }
    Ok(b)
}
fn path(commands: &[Command], m: [f64; 6]) -> Result<Bounds, String> {
    let mut bounds = Bounds::default();
    let mut current = [0.; 2];
    let mut start = [0.; 2];
    let mut have = false;
    for (index, command) in commands.iter().enumerate() {
        let error = |reason: String| format!("d2scene: path command {index}: {reason}");
        let needed = match command.kind {
            0 | 1 | 4 => 1,
            2 => 2,
            3 => 3,
            5 => 0,
            _ => return Err(error(format!("unknown kind {}", command.kind))),
        };
        if command.points[..needed].iter().any(|p| !finite(*p)) {
            return Err(error("non-finite point".into()));
        }
        if command.kind == 4
            && (!command.radii.iter().all(|v| v.is_finite()) || !command.rotation.is_finite())
        {
            return Err(error("non-finite arc".into()));
        }
        if command.kind != 0 && !have {
            return Err(error(format!(
                "{} before move",
                match command.kind {
                    1 => "line",
                    2 => "quadratic",
                    3 => "cubic",
                    4 => "arc",
                    _ => "close",
                }
            )));
        }
        match command.kind {
            0 => {
                current = command.points[0];
                start = current;
                have = true;
            }
            1 => {
                bounds.union(Bounds::points(&[
                    point(m, current),
                    point(m, command.points[0]),
                ]));
                current = command.points[0];
            }
            2 => {
                bounds.union(quadratic([
                    point(m, current),
                    point(m, command.points[0]),
                    point(m, command.points[1]),
                ]));
                current = command.points[1];
            }
            3 => {
                bounds.union(cubic([
                    point(m, current),
                    point(m, command.points[0]),
                    point(m, command.points[1]),
                    point(m, command.points[2]),
                ]));
                current = command.points[2];
            }
            4 => {
                bounds.union(arc(current, command, m).map_err(error)?);
                current = command.points[0];
            }
            5 => {
                bounds.union(Bounds::points(&[point(m, current), point(m, start)]));
                current = start;
            }
            _ => unreachable!(),
        }
    }
    if !bounds.finite() {
        return Err("d2scene: non-finite path bounds".into());
    }
    Ok(bounds)
}
pub fn primitive(primitive: &Primitive, m: [f64; 6]) -> Result<Bounds, String> {
    primitive_commands(primitive, m, &primitive.geometry.commands)
}
fn primitive_commands(p: &Primitive, m: [f64; 6], commands: &[Command]) -> Result<Bounds, String> {
    if !m.iter().all(|v| v.is_finite()) {
        return Err("d2scene: non-finite primitive transform".into());
    }
    let visible = p.fill.is_some() || p.stroke.as_ref().is_some_and(|s| s.width > 0.);
    let g = &p.geometry;
    let b = match g.kind {
        4 => {
            if g.asset.is_empty() {
                return Err("d2scene: image: empty asset ID".into());
            }
            let [x, y, w, h] = g.bounds;
            if !g.bounds.iter().all(|v| v.is_finite()) {
                return Err("d2scene: image: non-finite box".into());
            }
            if w < 0. || h < 0. {
                return Err("d2scene: image: negative box size".into());
            }
            Bounds::points(&[
                point(m, [x, y]),
                point(m, [x + w, y]),
                point(m, [x + w, y + h]),
                point(m, [x, y + h]),
            ])
        }
        1 => {
            let [x, y, w, h] = g.bounds;
            if !g.bounds.iter().all(|v| v.is_finite()) || w < 0. || h < 0. {
                return Err(format!(
                    "d2scene: rectangle: {}",
                    if g.bounds.iter().all(|v| v.is_finite()) {
                        "negative box size"
                    } else {
                        "non-finite box"
                    }
                ));
            }
            if g.radii.iter().any(|v| !v.is_finite() || *v < 0.) {
                return Err("d2scene: rectangle: invalid corner radius".into());
            }
            if !visible {
                return Ok(Bounds::default());
            }
            let lo = [min(x, x + w), min(y, y + h)];
            let hi = [max(x, x + w), max(y, y + h)];
            Bounds::points(&[
                point(m, lo),
                point(m, [hi[0], lo[1]]),
                point(m, hi),
                point(m, [lo[0], hi[1]]),
            ])
        }
        2 => {
            if !finite(g.center) || g.radii.iter().any(|v| !v.is_finite() || *v < 0.) {
                return Err("d2scene: ellipse: invalid geometry".into());
            }
            if !visible {
                return Ok(Bounds::default());
            }
            let center = point(m, g.center);
            let rx = (m[0] * g.radii[0]).hypot(m[2] * g.radii[1]);
            let ry = (m[1] * g.radii[0]).hypot(m[3] * g.radii[1]);
            Bounds::points(&[
                [center[0] - rx, center[1] - ry],
                [center[0] + rx, center[1] + ry],
            ])
        }
        3 => {
            if !visible {
                return Ok(Bounds::default());
            }
            path(commands, m)?
        }
        _ => return Err("native SVG image bounds stage is not implemented".into()),
    };
    Ok(if g.kind == 4 {
        b
    } else {
        expand(b, p.stroke.as_ref(), m)
    })
}
fn context(cancel: Option<&d2_rs_core::CancellationToken>) -> Result<(), String> {
    match cancel.and_then(d2_rs_core::CancellationToken::error) {
        Some(d2_rs_core::LayoutError::DeadlineExceeded) => Err("context deadline exceeded".into()),
        Some(_) => Err("context canceled".into()),
        None => Ok(()),
    }
}
pub fn validate_primitive(
    p: Option<&Primitive>,
    m: [f64; 6],
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<(), String> {
    let Some(p) = p else { return Ok(()) };
    if p.geometry.kind != 3 {
        let b = primitive(p, m).map_err(|_| "invalid primitive bounds")?;
        return if b.finite() {
            Ok(())
        } else {
            Err("invalid primitive bounds".into())
        };
    }
    let mut chunk = Vec::with_capacity(257);
    let mut current = [0.; 2];
    let mut start = [0.; 2];
    let mut have = false;
    for (index, command) in p.geometry.commands.iter().enumerate() {
        if index % 256 == 0 {
            context(cancel)?;
        }
        if chunk.is_empty() && have && command.kind != 0 {
            chunk.push(Command {
                kind: 0,
                points: [current, [0.; 2], [0.; 2]],
                ..Default::default()
            });
        }
        let mut command = command.clone();
        match command.kind {
            0 => {
                current = command.points[0];
                start = current;
                have = true;
            }
            1 | 4 => current = command.points[0],
            2 => current = command.points[1],
            3 => current = command.points[2],
            5 => {
                command = Command {
                    kind: 1,
                    points: [start, [0.; 2], [0.; 2]],
                    ..Default::default()
                };
                current = start;
            }
            _ => {}
        }
        chunk.push(command);
        if (index + 1) % 256 == 0 {
            let b = primitive_commands(p, m, &chunk).map_err(|_| "invalid path bounds")?;
            if !b.finite() {
                return Err("invalid path bounds".into());
            }
            chunk.clear();
            context(cancel)?;
        }
    }
    if !chunk.is_empty() {
        let b = primitive_commands(p, m, &chunk).map_err(|_| "invalid path bounds")?;
        if !b.finite() {
            return Err("invalid path bounds".into());
        }
        context(cancel)?;
    }
    Ok(())
}
pub fn validate_imported(
    scene: &Scene,
    root: Option<usize>,
    viewport: [f64; 6],
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<(), String> {
    let mut stack = vec![(root, viewport)];
    while let Some((index, parent)) = stack.pop() {
        context(cancel)?;
        let Some(index) = index else { continue };
        let node = &scene.nodes[index];
        let transform = super::svg_transform::mul(parent, node.transform);
        if !transform.iter().all(|v| v.is_finite()) {
            return Err("non-finite composed transform".into());
        }
        validate_primitive(node.primitive.as_ref(), transform, cancel)
            .map_err(|_| "invalid primitive bounds")?;
        if let Some(clip) = &node.clip {
            if !clip.transform.iter().all(|v| v.is_finite()) || clip.fill_rule > 1 {
                return Err("invalid clip".into());
            }
            let p = Primitive {
                geometry: super::svg_geometry::Geometry {
                    kind: 3,
                    commands: clip.commands.clone(),
                    ..Default::default()
                },
                fill: None,
                stroke: None,
                fill_rule: clip.fill_rule,
            };
            if validate_primitive(
                Some(&p),
                super::svg_transform::mul(transform, clip.transform),
                cancel,
            )
            .is_err()
            {
                context(cancel)?;
                return Err("invalid clip bounds".into());
            }
        }
        context(cancel)?;
        stack.extend(
            node.children
                .iter()
                .rev()
                .map(|&child| (Some(child), transform)),
        );
    }
    Ok(())
}
#[cfg(all(test, feature = "oracle-fixtures"))]
mod tests {
    use super::*;
    #[test]
    fn original_primitive_bounds_domain_and_chunked_validation() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/svg-bounds-v090.json"
        ))
        .unwrap();
        for row in rows.as_array().unwrap() {
            let p = Primitive {
                geometry: serde_json::from_value(row["geometry"].clone()).unwrap(),
                fill: serde_json::from_value(row["fill"].clone()).unwrap(),
                stroke: serde_json::from_value(row["stroke"].clone()).unwrap(),
                fill_rule: 0,
            };
            let m = serde_json::from_value(row["matrix"].clone()).unwrap();
            let result = primitive(&p, m);
            assert_eq!(
                result.as_ref().err().map(String::as_str).unwrap_or(""),
                row["error"].as_str().unwrap(),
                "{row}"
            );
            let bounds = result.unwrap_or_default();
            assert_eq!(bounds.finite(), row["finite"].as_bool().unwrap(), "{row}");
            assert_eq!(bounds.valid, row["valid"].as_bool().unwrap(), "{row}");
            let token = d2_rs_core::CancellationToken::new();
            let cancel = match row["cancel"].as_str().unwrap() {
                "cancelled" => {
                    token.cancel();
                    Some(&token)
                }
                "deadline" => {
                    token.expire();
                    Some(&token)
                }
                _ => None,
            };
            assert_eq!(
                validate_primitive(Some(&p), m, cancel)
                    .err()
                    .unwrap_or_default(),
                row["validation_error"].as_str().unwrap(),
                "{row}"
            );
        }
    }
}
#[cfg(all(test, feature = "oracle-fixtures"))]
mod imported_tests {
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_imported_scene_transform_primitive_clip_and_context_order() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/svg-imported-bounds-v090.json"
        ))
        .unwrap();
        for row in rows.as_array().unwrap() {
            let mode = row["mode"].as_str().unwrap();
            let scale = row["scale"].as_f64().unwrap();
            let clip_scale = row["clip_scale"].as_f64().unwrap();
            let mut scene = Scene::default();
            let make = |transform| super::super::svg_scene::Node {
                id: String::new(),
                classes: Vec::new(),
                transform,
                opacity: 0.,
                clip: None,
                primitive: None,
                children: Vec::new(),
            };
            scene.nodes.push(make([scale, 0., 0., scale, 0., 0.]));
            scene.root = if mode == "nil" { None } else { Some(0) };
            if matches!(mode, "rect" | "transparent-rect" | "overflow-rect") {
                let (x, w) = if mode == "overflow-rect" {
                    (1e308, 1e308)
                } else {
                    (1., 2.)
                };
                scene.nodes[0].primitive = Some(Primitive {
                    geometry: super::super::svg_geometry::Geometry {
                        kind: 1,
                        bounds: [x, 2., w, 3.],
                        ..Default::default()
                    },
                    fill: Some(super::super::svg_primitive::Paint::Solid {
                        color: [0, 0, 0, if mode == "transparent-rect" { 0 } else { 255 }],
                    }),
                    stroke: None,
                    fill_rule: 0,
                });
            }
            if mode == "child-overflow" {
                scene.nodes.push(make([1e308, 0., 0., 1e308, 0., 0.]));
                scene.nodes[0].children.push(1);
            }
            let commands = || {
                vec![
                    Command {
                        kind: 0,
                        ..Default::default()
                    },
                    Command {
                        kind: 1,
                        points: [
                            [if mode == "path-overflow" { 1e308 } else { 2. }, 3.],
                            [0.; 2],
                            [0.; 2],
                        ],
                        ..Default::default()
                    },
                ]
            };
            if matches!(mode, "clip" | "clip-empty" | "clip-rule") {
                scene.nodes[0].clip = Some(super::super::svg_scene::Clip {
                    commands: if mode == "clip-empty" {
                        Vec::new()
                    } else {
                        commands()
                    },
                    fill_rule: if mode == "clip-rule" { 2 } else { 0 },
                    transform: [clip_scale, 0., 0., clip_scale, 0., 0.],
                });
            }
            if matches!(mode, "path" | "path-overflow") {
                scene.nodes[0].primitive = Some(Primitive {
                    geometry: super::super::svg_geometry::Geometry {
                        kind: 3,
                        commands: commands(),
                        ..Default::default()
                    },
                    fill: Some(super::super::svg_primitive::Paint::Solid {
                        color: [0, 0, 0, 255],
                    }),
                    stroke: None,
                    fill_rule: 0,
                });
            }
            let token = d2_rs_core::CancellationToken::new();
            let cancel = match row["cancel"].as_str().unwrap() {
                "cancelled" => {
                    token.cancel();
                    Some(&token)
                }
                "deadline" => {
                    token.expire();
                    Some(&token)
                }
                _ => None,
            };
            assert_eq!(
                validate_imported(&scene, scene.root, [1., 0., 0., 1., 0., 0.], cancel)
                    .err()
                    .unwrap_or_default(),
                row["error"].as_str().unwrap(),
                "{row}"
            );
        }
    }
}
