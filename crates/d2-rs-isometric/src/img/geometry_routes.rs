// SPDX-License-Identifier: MPL-2.0
//! geometry_routes.go: path arithmetic and watertight tube tessellation.
use super::{geometry::MeshBuilder, mesh::*};
use d2_rs_render::go_float::gomath;
use d2_rs_scene::{go_max, go_min};
#[derive(Clone, Default)]
pub struct PacketRoute {
    pub points: std::vec::Vec<Vec>,
    pub lengths: std::vec::Vec<f64>,
    pub material: Option<MaterialRef>,
    pub forward: bool,
    pub reverse: bool,
}
pub fn route_lengths(points: &[Vec]) -> std::vec::Vec<f64> {
    let mut out = vec![0.0; points.len()];
    for i in 1..points.len() {
        out[i] = out[i - 1] + nlen(nsub(points[i], points[i - 1]));
    }
    out
}
pub fn path_point(points: &[Vec], lengths: &[f64], t: f64) -> Vec {
    if points.is_empty() {
        return Vec::default();
    }
    let t = go_max(0.0, go_min(1.0, t));
    let total = lengths[lengths.len() - 1];
    let distance = t * total;
    for i in 1..points.len() {
        if lengths[i] >= distance {
            let d = lengths[i] - lengths[i - 1];
            if d > 1e-12 {
                return nlerp(
                    points[i - 1],
                    points[i],
                    t.mul_add(total, -lengths[i - 1]) / d,
                );
            }
        }
    }
    points[points.len() - 1]
}
impl MeshBuilder {
    pub fn tube(&mut self, points: &[Vec], radius: f64, mat: &MaterialRef) {
        if points.len() < 2 || radius <= 0.0 {
            return;
        }
        let mut rings = vec![[Vertex::default(); 6]; points.len()];
        for (i, p) in points.iter().enumerate() {
            let a = points[i.saturating_sub(1)];
            let c = points[(i + 1).min(points.len() - 1)];
            let tangent = nunit(nsub(c, a));
            let side = nunit(ncross(tangent, nv(0.0, 1.0, 0.0)));
            let up = nunit(ncross(side, tangent));
            for j in 0..6 {
                let angle = 2.0 * std::f64::consts::PI * j as f64 / 6.0;
                let n = nadd(nmul(up, gomath::cos(angle)), nmul(side, gomath::sin(angle)));
                rings[i][j] = Vertex {
                    position: nadd(*p, nmul(n, radius)),
                    normal: n,
                    ..Default::default()
                };
            }
        }
        for i in 1..rings.len() {
            for j in 0..6 {
                let next = (j + 1) % 6;
                let (a, c, d, e) = (
                    rings[i - 1][j],
                    rings[i - 1][next],
                    rings[i][next],
                    rings[i][j],
                );
                self.triangle(a, c, d, mat, true);
                self.triangle(a, d, e, mat, true);
            }
        }
    }
}

pub fn native_route_section(
    points: &[Vec],
    lengths: &[f64],
    start: f64,
    end: f64,
) -> std::vec::Vec<Vec> {
    if points.len() < 2 || lengths.len() != points.len() || end <= start {
        return vec![];
    }
    let start = go_max(0.0, go_min(1.0, start));
    let end = go_max(0.0, go_min(1.0, end));
    let total = lengths[lengths.len() - 1];
    let mut out = vec![path_point(points, lengths, start)];
    let first = lengths.partition_point(|&l| l <= start * total);
    for i in first..points.len() {
        if lengths[i] >= end * total {
            break;
        }
        out.push(points[i]);
    }
    out.push(path_point(points, lengths, end));
    out
}
pub fn native_route_dashes(
    points: &[Vec],
    lengths: &[f64],
    start: f64,
    end: f64,
    stroke_dash: f64,
    budget: usize,
) -> std::vec::Vec<std::vec::Vec<Vec>> {
    if points.len() < 2 || lengths.len() != points.len() || end <= start {
        return vec![];
    }
    let dash = go_max(0.18, stroke_dash * 0.07);
    let steps = 1.max(
        budget
            .max(1)
            .min((lengths[lengths.len() - 1] / (dash * 1.65)).ceil() as usize),
    );
    let mut out = vec![];
    for i in 0..steps {
        let from = go_max(start, i as f64 / steps as f64);
        let to = go_min(end, go_min(1.0, (i as f64 + 0.62) / steps as f64));
        if to > from {
            out.push(native_route_section(points, lengths, from, to))
        }
    }
    out
}
pub fn native_rounded_route(points: &[Vec]) -> std::vec::Vec<Vec> {
    let mut clean = vec![];
    for &p in points {
        if clean.last().is_none_or(|&last| nlen(nsub(p, last)) > 1e-6) {
            clean.push(p)
        }
    }
    let points = clean;
    if points.len() < 2 {
        return points;
    }
    let mut out = vec![points[0]];
    for i in 1..points.len() - 1 {
        let (p, a, c) = (points[i], points[i - 1], points[i + 1]);
        let r = go_min(0.18, go_min(nlen(nsub(p, a)) / 4.0, nlen(nsub(c, p)) / 4.0));
        let entry = nadd(p, nmul(nunit(nsub(a, p)), r));
        let exit = nadd(p, nmul(nunit(nsub(c, p)), r));
        out.push(entry);
        for j in 1..=12 {
            let t = j as f64 / 12.0;
            let mut q = nadd(
                nadd(
                    nmul(entry, (1.0 - t) * (1.0 - t)),
                    nmul(p, 2.0 * (1.0 - t) * t),
                ),
                nmul(exit, t * t),
            );
            if entry.y == p.y && p.y == exit.y {
                q.y = p.y
            }
            out.push(q)
        }
    }
    out.push(points[points.len() - 1]);
    out
}
/// d2target.Arrowhead.Dimensions; thin adapter until shared target API is public.
pub fn arrow_dimensions(kind: &str, stroke: f64) -> (f64, f64) {
    let (w, h, wm, hm) = match kind {
        "arrow" => (4.0, 4.0, 4.0, 4.0),
        "triangle" => (4.0, 4.0, 3.0, 4.0),
        "unfilled-triangle" => (7.0, 7.0, 3.0, 4.0),
        "line" => (0.0, 0.0, 5.0, 8.0),
        "filled-diamond" => (11.0, 7.0, 5.5, 3.5),
        "diamond" => (11.0, 9.0, 5.5, 4.5),
        "cross" => (7.0, 7.0, 5.0, 5.0),
        "filled-circle" | "circle" => (8.0, 8.0, 5.0, 5.0),
        "filled-box" | "box" => (6.0, 6.0, 5.0, 5.0),
        "cf-one" | "cf-many" | "cf-one-required" | "cf-many-required" => (9.0, 9.0, 4.5, 4.5),
        _ => (0.0, 0.0, 0.0, 0.0),
    };
    let stroke = go_max(2.0, stroke);
    (stroke.mul_add(wm, w), stroke.mul_add(hm, h))
}
pub fn native_arrow_clearance(kind: &str, width: i64) -> f64 {
    let (w, _) = arrow_dimensions(kind, width as f64);
    match kind {
        "arrow" => w * 0.0075,
        "triangle" => w * 0.01,
        "filled-diamond" | "filled-circle" | "filled-box" => w * 0.005,
        "unfilled-triangle" | "diamond" | "circle" | "box" | "cross" => w * 0.01,
        "cf-one" | "cf-many" | "cf-one-required" | "cf-many-required" => {
            (width as f64).mul_add(1.8, w + 3.0) * 0.01
        }
        _ => 0.0,
    }
}
impl MeshBuilder {
    pub fn arrow_with_opacity(
        &mut self,
        kind: &str,
        mut at: Vec,
        direction: Vec,
        mat: &MaterialRef,
        width: i64,
        opacity: f64,
    ) {
        use super::geometry::{native_material, native_paint};
        use std::rc::Rc;
        if kind.is_empty() || kind == "none" {
            return;
        }
        let stroke = width.max(0) as f64;
        let scale = if self.scale <= 0.0 { 0.01 } else { self.scale };
        let (w, h) = arrow_dimensions(kind, stroke);
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        let along = nunit(nv(direction.x, 0.0, direction.z));
        let side = nv(-along.z, 0.0, along.x);
        let point = |at: Vec, x: f64, y: f64| {
            nadd(
                at,
                nadd(
                    nmul(along, (x - w) * scale),
                    nmul(side, (y - h / 2.0) * scale),
                ),
            )
        };
        let line = |b: &mut MeshBuilder, at: Vec, coords: &[[f64; 2]]| {
            if stroke <= 0.0 {
                return;
            }
            let pts: std::vec::Vec<_> = coords.iter().map(|p| point(at, p[0], p[1])).collect();
            b.tube(&pts, stroke * scale / 2.0, mat);
        };
        // Go passes a nil side material for printed marker polygons.
        let no_side = material(Material::default());
        let polygon = |b: &mut MeshBuilder, at: Vec, coords: &[[f64; 2]], paint: &MaterialRef| {
            let p: std::vec::Vec<_> = coords.iter().map(|p| point(at, p[0], p[1])).collect();
            b.extruded_profile(&p, at.y, Some(paint), &no_side);
        };
        let background = native_material(&self.arrow_background, 0.8, 0.0, opacity);
        {
            let mut m = background.borrow_mut();
            m.color = native_paint(&self.arrow_background, "#ffffff");
            m.color.a = (m.color.a as f64 * go_max(0.0, go_min(1.0, opacity))).round() as u8;
            m.unlit = true;
        }
        let first = self.triangles.len();
        let circle = |b: &mut MeshBuilder, at: Vec, cx: f64, cy: f64, r: f64, solid: bool| {
            let p: std::vec::Vec<_> = (0..49)
                .map(|i| {
                    let a = 2.0 * std::f64::consts::PI * i as f64 / 48.0;
                    [r.mul_add(gomath::cos(a), cx), r.mul_add(gomath::sin(a), cy)]
                })
                .collect();
            polygon(b, at, &p[..48], if solid { mat } else { &background });
            if !solid {
                line(b, at, &p)
            }
        };
        let inset = stroke / 2.0;
        match kind {
            "arrow" => polygon(
                self,
                at,
                &[[0.0, 0.0], [w, h / 2.0], [0.0, h], [w / 4.0, h / 2.0]],
                mat,
            ),
            "triangle" => polygon(self, at, &[[0.0, 0.0], [w, h / 2.0], [0.0, h]], mat),
            "line" => line(
                self,
                at,
                &[[inset, inset], [w - inset, h / 2.0], [inset, h - inset]],
            ),
            "unfilled-triangle" => {
                let p = [
                    [inset, inset],
                    [w - inset, h / 2.0],
                    [inset, h - inset],
                    [inset, inset],
                ];
                polygon(self, at, &p[..3], &background);
                line(self, at, &p)
            }
            "diamond" => {
                let p = [
                    [0.0, h / 2.0],
                    [w / 2.0, h / 8.0],
                    [w, h / 2.0],
                    [w / 2.0, h * 0.9],
                    [0.0, h / 2.0],
                ];
                polygon(self, at, &p[..4], &background);
                line(self, at, &p)
            }
            "filled-diamond" => polygon(
                self,
                at,
                &[[0.0, h / 2.0], [w / 2.0, 0.0], [w, h / 2.0], [w / 2.0, h]],
                mat,
            ),
            "circle" | "filled-circle" => circle(
                self,
                at,
                w / 2.0 + inset,
                h / 2.0,
                go_max(
                    0.0,
                    w / 2.0
                        - if kind == "filled-circle" {
                            inset
                        } else {
                            stroke
                        },
                ),
                kind == "filled-circle",
            ),
            "box" => {
                let p = [
                    [inset, inset],
                    [w - inset, inset],
                    [w - inset, h - inset],
                    [inset, h - inset],
                    [inset, inset],
                ];
                polygon(self, at, &p[..4], &background);
                line(self, at, &p)
            }
            "filled-box" => polygon(self, at, &[[0.0, 0.0], [w, 0.0], [w, h], [0.0, h]], mat),
            "cross" => {
                let q = stroke / 8.0;
                let mut p = vec![
                    [0.0, h / 2.0 + q],
                    [w / 2.0 - q, h / 2.0 + q],
                    [w / 2.0 - q, h],
                    [w / 2.0 + q, h],
                    [w / 2.0 + q, h / 2.0 + q],
                    [w, h / 2.0 + q],
                    [w, h / 2.0 - q],
                    [w / 2.0 + q, h / 2.0 - q],
                    [w / 2.0 + q, 0.0],
                    [w / 2.0 - q, 0.0],
                    [w / 2.0 - q, h / 2.0 - q],
                    [0.0, h / 2.0 - q],
                ];
                for p in &mut p {
                    let (x, y) = (p[0] - w / 2.0, p[1] - h / 2.0);
                    *p = [
                        w / 2.0 + (x - y) / std::f64::consts::SQRT_2,
                        h / 2.0 + (x + y) / std::f64::consts::SQRT_2,
                    ]
                }
                p.push(p[0]);
                polygon(self, at, &p[..p.len() - 1], &background);
                line(self, at, &p);
                line(self, at, &[[w / 2.0, h / 2.0], [w, h / 2.0]])
            }
            "cf-one" | "cf-many" | "cf-one-required" | "cf-many-required" => {
                let offset = stroke.mul_add(1.8, 3.0);
                at = nsub(at, nmul(along, offset * scale));
                if kind.ends_with("-required") {
                    line(self, at, &[[offset, 0.0], [offset, h]])
                } else {
                    circle(self, at, offset / 2.0 + 2.0, h / 2.0, offset / 2.0, false)
                }
                line(self, at, &[[w - 3.0, h / 2.0], [w + offset, h / 2.0]]);
                if kind.starts_with("cf-many") {
                    line(
                        self,
                        at,
                        &[[w + offset, 0.0], [offset + 3.0, h / 2.0], [w + offset, h]],
                    )
                } else {
                    line(self, at, &[[offset * 2.0, 0.0], [offset * 2.0, h]])
                }
            }
            _ => {}
        }
        let bias = go_max(0.012, go_min(0.075, stroke * 0.012)) * 1.1;
        for t in &mut self.geometry.triangles[first..] {
            t.depth_bias = bias;
            t.cast_shadow = false;
            if !t
                .material
                .as_ref()
                .is_some_and(|m| Rc::ptr_eq(m, &background))
            {
                t.depth_bias += 0.002
            }
        }
    }
}
