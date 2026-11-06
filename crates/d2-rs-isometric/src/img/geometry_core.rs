// SPDX-License-Identifier: MPL-2.0
//! Native bounded mesh tessellation, translated from d2isometricimg geometry.go
//! and native_shapes.go. Geometry construction uses no renderer or oracle.

use super::mesh::{MaterialRef, Triangle, Vec, Vertex, nadd, ncross, ndot, nmul, nsub, nunit, nv};
use d2_rs_render::go_float::{go_fmadd, go_fmsub, gomath};
use d2_rs_scene::{go_max, go_min};

pub const MAX_NATIVE_TRIANGLES: usize = 1_000_000;

/// Append-only geometry for the source meshBuilder. A replay can retain an
/// existing prefix count without copying already-produced triangles.
#[derive(Default)]
pub struct MeshGeometry {
    pub context: super::context::RenderContext,
    pub triangles: std::vec::Vec<Triangle>,
    pub err: Option<String>,
    prefix_triangles: usize,
}
impl MeshGeometry {
    pub fn with_prefix_count(prefix_triangles: usize) -> Self {
        Self {
            prefix_triangles,
            ..Default::default()
        }
    }
    pub fn fail(&mut self, error: impl Into<String>) {
        if self.err.is_none() {
            self.err = Some(error.into());
        }
    }

    pub fn triangle(&mut self, a: Vertex, c: Vertex, d: Vertex, m: &MaterialRef, shadow: bool) {
        if self.err.is_some() || m.borrow().color.a == 0 {
            return;
        }
        if self.triangles.len().saturating_add(self.prefix_triangles) >= MAX_NATIVE_TRIANGLES {
            self.err = Some(format!(
                "isometric geometry exceeds {MAX_NATIVE_TRIANGLES} triangles"
            ));
            return;
        }
        if self.triangles.len().is_multiple_of(1024) {
            if let Err(error) = self.context.check_string() {
                self.err = Some(error);
                return;
            }
        }
        self.triangles.push(Triangle {
            v: [a, c, d],
            material: Some(m.clone()),
            cast_shadow: shadow,
            ..Default::default()
        });
    }

    pub fn triangle_opt(
        &mut self,
        a: Vertex,
        c: Vertex,
        d: Vertex,
        m: Option<&MaterialRef>,
        shadow: bool,
    ) {
        if let Some(m) = m {
            self.triangle(a, c, d, m, shadow);
        }
    }

    pub fn flat(&mut self, a: Vec, c: Vec, d: Vec, m: &MaterialRef, shadow: bool) {
        let n = nunit(ncross(nsub(c, a), nsub(d, a)));
        self.triangle(
            Vertex {
                position: a,
                normal: n,
                ..Default::default()
            },
            Vertex {
                position: c,
                normal: n,
                ..Default::default()
            },
            Vertex {
                position: d,
                normal: n,
                ..Default::default()
            },
            m,
            shadow,
        );
    }

    /// Project each face of a cube onto a rounded cuboid.
    pub fn box_(&mut self, center: Vec, size: Vec, m: &MaterialRef, r: f64) {
        if self.err.is_some() || size.x <= 0.0 || size.y <= 0.0 || size.z <= 0.0 {
            return;
        }
        let half = nmul(size, 0.5);
        let r = go_min(r, go_min(size.x, go_min(size.y, size.z)) * 0.24);
        let ext = [half.x, half.y, half.z];
        let core = nv(
            go_max(0.0, half.x - r),
            go_max(0.0, half.y - r),
            go_max(0.0, half.z - r),
        );
        for axis in 0..3 {
            let u = (axis + 1) % 3;
            let v = (axis + 2) % 3;
            let coords = |h: f64| -> std::vec::Vec<f64> {
                if r < 1e-6 {
                    return vec![-h, h];
                }
                vec![-h, -h + r, h - r, h]
            };
            let (us, vs) = (coords(ext[u]), coords(ext[v]));
            for sign in [-1.0, 1.0] {
                let vertex = |x: f64, y: f64| -> Vertex {
                    let mut a = [0.0; 3];
                    a[axis] = ext[axis] * sign;
                    a[u] = x;
                    a[v] = y;
                    let mut p = nv(a[0], a[1], a[2]);
                    let q = nv(
                        go_max(-core.x, go_min(core.x, p.x)),
                        go_max(-core.y, go_min(core.y, p.y)),
                        go_max(-core.z, go_min(core.z, p.z)),
                    );
                    let mut n = nunit(nsub(p, q));
                    if r < 1e-6 {
                        let mut a = [0.0; 3];
                        a[axis] = sign;
                        n = nv(a[0], a[1], a[2]);
                    } else {
                        p = nadd(q, nmul(n, r));
                    }
                    Vertex {
                        position: nadd(center, p),
                        normal: n,
                        ..Default::default()
                    }
                };
                for i in 1..us.len() {
                    for j in 1..vs.len() {
                        let (a, c, d, e) = (
                            vertex(us[i - 1], vs[j - 1]),
                            vertex(us[i], vs[j - 1]),
                            vertex(us[i], vs[j]),
                            vertex(us[i - 1], vs[j]),
                        );
                        if sign > 0.0 {
                            self.triangle(a, c, d, m, true);
                            self.triangle(a, d, e, m, true);
                        } else {
                            self.triangle(a, d, c, m, true);
                            self.triangle(a, e, d, m, true);
                        }
                    }
                }
            }
        }
    }

    pub fn sphere(&mut self, center: Vec, radii: Vec, m: &MaterialRef, segments: i64, rings: i64) {
        let vertex = |i: i64, j: i64| -> Vertex {
            let phi = 2.0 * std::f64::consts::PI * i as f64 / segments as f64;
            let theta = std::f64::consts::PI * j as f64 / rings as f64;
            let p = nv(
                gomath::sin(theta) * gomath::cos(phi),
                gomath::cos(theta),
                gomath::sin(theta) * gomath::sin(phi),
            );
            let n = nunit(nv(
                p.x / go_max(radii.x, 1e-8),
                p.y / go_max(radii.y, 1e-8),
                p.z / go_max(radii.z, 1e-8),
            ));
            // ARM64 geometry.go:78 (nadd inlined into sphere.func1): FMADDD x3.
            let position = nv(
                go_fmadd(p.x, radii.x, center.x),
                go_fmadd(p.y, radii.y, center.y),
                go_fmadd(p.z, radii.z, center.z),
            );
            Vertex {
                position,
                normal: n,
                ..Default::default()
            }
        };
        for j in 0..rings {
            for i in 0..segments {
                let (a, c, d, e) = (
                    vertex(i, j),
                    vertex(i + 1, j),
                    vertex(i + 1, j + 1),
                    vertex(i, j + 1),
                );
                self.triangle(a, c, d, m, true);
                self.triangle(a, d, e, m, true);
            }
        }
    }

    /// Elliptic cylinders and frusta, optionally laid along X for queue barrels.
    #[allow(clippy::too_many_arguments)]
    pub fn cylinder(
        &mut self,
        center: Vec,
        rx: f64,
        rz: f64,
        height: f64,
        top_ratio: f64,
        body: &MaterialRef,
        cap: &MaterialRef,
        segments: i64,
        horizontal: bool,
    ) {
        let transform = |p: Vec| {
            if horizontal {
                nadd(center, nv(p.y, -p.x, p.z))
            } else {
                nadd(center, p)
            }
        };
        let normal = |p: Vec| if horizontal { nv(p.y, -p.x, p.z) } else { p };
        let vertex = |i: i64, top: bool| -> Vertex {
            let a = 2.0 * std::f64::consts::PI * i as f64 / segments as f64;
            let (ratio, y) = if top {
                (top_ratio, height / 2.0)
            } else {
                (1.0, -height / 2.0)
            };
            let p = nv(rx * ratio * gomath::cos(a), y, rz * ratio * gomath::sin(a));
            let n = nunit(nv(
                gomath::cos(a) / go_max(rx, 1e-8),
                (1.0 - top_ratio) / go_max(height, 1e-8),
                gomath::sin(a) / go_max(rz, 1e-8),
            ));
            Vertex {
                position: transform(p),
                normal: normal(n),
                ..Default::default()
            }
        };
        for i in 0..segments {
            let (a, c, d, e) = (
                vertex(i, false),
                vertex(i + 1, false),
                vertex(i + 1, true),
                vertex(i, true),
            );
            self.triangle(a, e, d, body, true);
            self.triangle(a, d, c, body, true);
            for top in [false, true] {
                let (mut p, mut q) = (vertex(i, top), vertex(i + 1, top));
                let (y, n) = if top {
                    (height / 2.0, nv(0.0, 1.0, 0.0))
                } else {
                    (-height / 2.0, nv(0.0, -1.0, 0.0))
                };
                p.normal = normal(n);
                q.normal = p.normal;
                let mid = Vertex {
                    position: transform(nv(0.0, y, 0.0)),
                    normal: p.normal,
                    ..Default::default()
                };
                if top {
                    self.triangle(mid, q, p, cap, true);
                } else {
                    self.triangle(mid, p, q, cap, true);
                }
            }
        }
    }

    pub fn extruded_profile(
        &mut self,
        p: &[Vec],
        base: f64,
        top: Option<&MaterialRef>,
        side: &MaterialRef,
    ) {
        if p.len() < 3 {
            return;
        }
        let area = native_polygon_area(p);
        let sign = if area < 0.0 { -1.0 } else { 1.0 };
        let n = p.len();
        let mut edges = vec![Vec::default(); n];
        let mut corners = vec![Vec::default(); n];
        let mut smooth = vec![false; n];
        for (i, a) in p.iter().enumerate() {
            let c = p[(i + 1) % n];
            edges[i] = nunit(nv((c.z - a.z) * sign, 0.0, (a.x - c.x) * sign));
        }
        for (i, at) in p.iter().enumerate() {
            let (previous, next) = ((i + n - 1) % n, (i + 1) % n);
            let (incoming, outgoing) = (nsub(*at, p[previous]), nsub(p[next], *at));
            // ARM64 native_shapes.go:669: FMSUBD.
            let turn = go_fmsub(incoming.z, outgoing.x, incoming.x * outgoing.z) * sign;
            // Smooth only gentle convex turns.
            if turn >= 0.0
                && ndot(edges[previous], edges[i]) > gomath::cos(std::f64::consts::PI / 6.0)
            {
                smooth[i] = true;
                corners[i] = nunit(nadd(edges[previous], edges[i]));
            }
        }
        let normal = |vertex: usize, edge: usize| {
            if smooth[vertex] {
                corners[vertex]
            } else {
                edges[edge]
            }
        };
        for i in 0..n {
            let next = (i + 1) % n;
            let (mut a, mut c) = (p[i], p[next]);
            let (mut na, mut nc) = (normal(i, i), normal(next, i));
            if area < 0.0 {
                std::mem::swap(&mut a, &mut c);
                std::mem::swap(&mut na, &mut nc);
            }
            let (mut d, mut e) = (a, c);
            d.y = base;
            e.y = base;
            let va = Vertex {
                position: a,
                normal: na,
                ..Default::default()
            };
            let vc = Vertex {
                position: c,
                normal: nc,
                ..Default::default()
            };
            let vd = Vertex {
                position: d,
                normal: na,
                ..Default::default()
            };
            let ve = Vertex {
                position: e,
                normal: nc,
                ..Default::default()
            };
            self.triangle(va, vc, vd, side, true);
            self.triangle(vc, ve, vd, side, true);
        }
        let Some(top) = top else {
            return;
        };
        let mut indices: std::vec::Vec<usize> = (0..n).collect();
        // ARM64 native_shapes.go:705: FMSUBD.
        let cross = |a: Vec, c: Vec, d: Vec| {
            go_fmsub(c.z - a.z, d.x - a.x, (c.x - a.x) * (d.z - a.z)) * sign
        };
        let mut work = 0;
        while indices.len() > 2 {
            let mut found = false;
            let len = indices.len();
            for j in 0..len {
                let k = indices[j];
                let (ia, id) = (indices[(j + len - 1) % len], indices[(j + 1) % len]);
                let (a, c, d) = (p[ia], p[k], p[id]);
                if cross(a, c, d) <= 1e-12 {
                    continue;
                }
                let mut empty = true;
                for &q in &indices {
                    work += 1;
                    if work > 4_000_000 {
                        self.fail("native shape triangulation exceeds work budget");
                        return;
                    }
                    if q == ia || q == k || q == id {
                        continue;
                    }
                    let v = p[q];
                    if cross(a, c, v) > 1e-10 && cross(c, d, v) > 1e-10 && cross(d, a, v) > 1e-10 {
                        empty = false;
                        break;
                    }
                }
                if empty {
                    self.flat(a, d, c, top, true);
                    indices.remove(j);
                    found = true;
                    break;
                }
            }
            if !found {
                // Only redundant collinear contour points can remain.
                return;
            }
        }
    }
}

pub fn native_polygon_area(p: &[Vec]) -> f64 {
    let mut a = 0.0;
    for i in 0..p.len() {
        let j = (i + 1) % p.len();
        // ARM64 native_shapes.go:646: FMSUBD.
        a += go_fmsub(p[j].x, p[i].z, p[i].x * p[j].z);
    }
    a / 2.0
}
