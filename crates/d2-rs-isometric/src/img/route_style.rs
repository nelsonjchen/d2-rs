// SPDX-License-Identifier: MPL-2.0
//! route_style.go: deterministic route paints and flat ink/casing ribbons.
use super::{
    geometry::{MeshBuilder, native_material},
    mesh::*,
};
use crate::scene::Edge;
use d2_rs_scene::{go_max, go_min};
pub fn native_route_tint(e: &Edge) -> String {
    if e.stroke_explicit {
        return e.stroke.clone();
    }
    let mut hash = 2166136261u32;
    for b in e.id.bytes() {
        hash = (hash ^ b as u32).wrapping_mul(16777619)
    }
    [
        "#3d6d8f", "#36776b", "#786391", "#957245", "#586d86", "#8c606b",
    ][hash as usize % 6]
        .into()
}
pub fn native_route_radius(e: &Edge) -> f64 {
    go_max(0.012, go_min(0.075, e.stroke_width as f64 * 0.012))
}
impl MeshBuilder {
    pub fn route_ink(&mut self, points: &[Vec], radius: f64, mat: &MaterialRef) {
        if points.len() < 2 || radius <= 0.0 {
            return;
        }
        let mut left = vec![Vertex::default(); points.len()];
        let mut right = left.clone();
        for (i, &p) in points.iter().enumerate() {
            let a = points[i.saturating_sub(1)];
            let c = points[(i + 1).min(points.len() - 1)];
            let tangent = nunit(nsub(c, a));
            let side = nunit(ncross(tangent, nv(0.0, 1.0, 0.0)));
            let up = nunit(ncross(side, tangent));
            left[i] = Vertex {
                position: nadd(p, nmul(side, radius)),
                normal: up,
                ..Default::default()
            };
            right[i] = Vertex {
                position: nsub(p, nmul(side, radius)),
                normal: up,
                ..Default::default()
            };
        }
        for i in 1..points.len() {
            self.triangle(left[i - 1], right[i], right[i - 1], mat, false);
            self.triangle(left[i - 1], left[i], right[i], mat, false);
        }
    }
    pub fn route_casing(&mut self, points: &[Vec], radius: f64, opacity: f64) {
        if points.len() < 2 {
            return;
        }
        let mat = native_material("#f5f7fb", 1.0, 0.0, opacity);
        mat.borrow_mut().unlit = true;
        let mut left = vec![Vertex::default(); points.len()];
        let mut right = left.clone();
        for (i, &p) in points.iter().enumerate() {
            let a = points[i.saturating_sub(1)];
            let c = points[(i + 1).min(points.len() - 1)];
            let tangent = nunit(nsub(c, a));
            let side = nunit(ncross(tangent, nv(0.0, 1.0, 0.0)));
            let up = nunit(ncross(side, tangent));
            let mut offset = radius * 0.8;
            if self.route_casing_floor > 0.0 && up.y > 1e-9 {
                offset = go_min(offset, go_max(0.0, (p.y - self.route_casing_floor) / up.y))
            }
            let center = nsub(p, nmul(up, offset));
            left[i] = Vertex {
                position: nadd(center, nmul(side, radius * 1.3)),
                normal: up,
                ..Default::default()
            };
            right[i] = Vertex {
                position: nsub(center, nmul(side, radius * 1.3)),
                normal: up,
                ..Default::default()
            };
        }
        for i in 1..points.len() {
            self.triangle(left[i - 1], right[i], right[i - 1], &mat, false);
            self.triangle(left[i - 1], left[i], right[i], &mat, false);
        }
    }
}
