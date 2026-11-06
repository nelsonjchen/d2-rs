// SPDX-License-Identifier: MPL-2.0
//! d2isometricimg native_solids.go and solid_contacts.go: cylinders, ovals,
//! hexagons and queue barrels, and route contacts with their real walls.
//! Contraction sites are marked `ARM64:`; products by 0.5 or by the constant
//! radius 1 are exact, so their fusion is unobservable.

use std::rc::Rc;

use super::math as gomath;
use d2_rs_render::go_float::{go_fmadd, go_fmsub};
use d2_rs_scene::{go_max, go_min};
use d2_rs_scenebuild::golib::label::Position;

use super::geometry::{MeshBuilder, native_material, native_paint, native_token};
use super::hierarchy_boards::hierarchy_node_relief;
use super::hierarchy_support::hierarchy_node_extension;
use super::mesh::{MaterialRef, Texture, Vec, Vertex, nadd, nmul, nsub, nunit, nv};
use super::native_edges::native_classic_rim;
use super::native_shapes::{
    native_face_source, native_node_icon_surface, native_node_label_surface,
};
use super::route_labels::caption_finite;
use super::surface_icons::surface_icon_layout;
use super::vector_surface::native_vector_solid_cap;
use crate::scene::{Edge, Node};
use crate::target::{
    MULTIPLE_OFFSET, SHAPE_CIRCLE, SHAPE_CYLINDER, SHAPE_HEXAGON, SHAPE_OVAL, SHAPE_QUEUE,
    SHAPE_RECTANGLE, THREE_DEE_OFFSET, Text,
};

pub const NATIVE_BARREL_SEGMENTS: i64 = 72;
pub const NATIVE_UPRIGHT_SEGMENTS: i64 = 72;

/// Surface decoration must not select a different physical shape.
pub fn native_solid_node(n: &Node) -> bool {
    !n.container
        && matches!(
            n.type_.as_str(),
            SHAPE_CYLINDER | SHAPE_QUEUE | SHAPE_CIRCLE | SHAPE_OVAL | SHAPE_HEXAGON
        )
}

pub fn native_solid_height(n: &Node) -> f64 {
    let mut h = go_max(0.04, n.size.y);
    if n.metadata.original.three_dee {
        let mut scale = 0.01;
        if n.metadata.original.width > 0 {
            scale = n.size.x / n.metadata.original.width as f64;
        }
        // ARM64 native_solids.go:34.
        h = go_fmadd(THREE_DEE_OFFSET as f64, scale, h);
    }
    h
}

#[derive(Clone)]
pub struct NativeSolidPaint {
    pub cap: MaterialRef,
    pub ink: MaterialRef,
    pub wall: MaterialRef,
    pub ink_fit: f64,
    pub shadow_fill_alpha: Option<u8>,
}

impl NativeSolidPaint {
    /// The native viewport includes the complete centered outline; map that
    /// viewport to the cap.
    pub fn cap_vertex(&self, at: Vec, normal: Vec, u: f64, v: f64) -> Vertex {
        let fit = if self.ink_fit <= 0.0 {
            1.0
        } else {
            self.ink_fit
        };
        Vertex {
            position: at,
            normal,
            u: 0.5 + (u - 0.5) / fit,
            v: 0.5 + (v - 0.5) / fit,
        }
    }
}

pub fn native_queue_crown(n: &Node) -> f64 {
    if n.size.x <= 0.0 || n.size.z <= 0.0 {
        return 1.0;
    }
    let s = native_face_source(n, &n.fill);
    let position = Position::from_string(&n.metadata.original.label_position);
    let mut text = native_node_label_surface(n, &s, 0.0);
    let mut extent: f64 = 0.0;
    let include = |extent: &mut f64, surface: &super::geometry::LabelSurface| {
        if surface.width > 0.0 && surface.depth > 0.0 {
            *extent = go_max(
                *extent,
                (surface.center.z - n.position.z).abs() + surface.depth / 2.0,
            );
        }
    };
    if n.metadata.original.icon.is_some() {
        let face = native_node_icon_surface(n, &s, 0.0);
        let scale = n.size.z / s.height as f64;
        let (icon, remaining) = surface_icon_layout(&face, &n.metadata.original, scale, "node");
        include(&mut extent, &icon);
        if !position.is_outside() && !position.is_border() {
            text = remaining;
            if n.metadata.original.text.label_height > 0 {
                text.depth = go_min(
                    text.depth,
                    n.metadata.original.text.label_height as f64 * scale * 1.06,
                );
            }
        }
    }
    if !n.metadata.original.text.label.is_empty() && !position.is_outside() && !position.is_border()
    {
        include(&mut extent, &text);
    }
    if extent <= 0.0 {
        return 1.0;
    }
    let extent = go_min(
        1.0,
        (extent + go_min(0.02, n.size.z * 0.02)) / (n.size.z / 2.0),
    );
    // Snap outward to a tessellation vertex.
    let step = 2.0 * std::f64::consts::PI / NATIVE_BARREL_SEGMENTS as f64;
    let angle = gomath::ceil(gomath::asin(extent) / step) * step;
    go_max(0.0, gomath::cos(angle))
}

pub fn native_barrel_point(i: i64, depth: f64, height: f64, crown: f64) -> (f64, f64) {
    let angle = i as f64 * 2.0 * std::f64::consts::PI / NATIVE_BARREL_SEGMENTS as f64;
    (
        height * (1.0 + go_min(gomath::cos(angle), crown)) / (1.0 + crown),
        gomath::sin(angle) * depth / 2.0,
    )
}

pub fn native_upright_point(i: i64, hex: bool) -> (f64, f64) {
    if hex {
        // D2's hexagon has slightly off-center side points; Go folds
        // 2*(43.6/87.3)-1 exactly to -1/873.
        let side = -1.0 / 873.0;
        let p = [
            [1.0, side],
            [0.5, 1.0],
            [-0.5, 1.0],
            [-1.0, side],
            [-0.5, -1.0],
            [0.5, -1.0],
        ][(i % 6) as usize];
        return (p[0], p[1]);
    }
    let angle = i as f64 * 2.0 * std::f64::consts::PI / NATIVE_UPRIGHT_SEGMENTS as f64;
    (gomath::cos(angle), gomath::sin(angle))
}

impl MeshBuilder {
    pub fn solid_paint(&mut self, n: &Node, kind: &str, fill: &str) -> NativeSolidPaint {
        let mut s = native_face_source(n, fill);
        s.type_ = kind.into();
        // A cap and its unwrapped patterned wall share the node's allocation.
        let budget = self.face_max_pixels;
        let patterned = !s.fill_pattern.is_empty() && s.fill_pattern != "none";
        if patterned && budget > 0 {
            self.face_max_pixels = 1.max(budget / 2);
        }
        let (tex, stroke) = if native_classic_rim(n) {
            (self.native_face_layers(&s, None).0, None)
        } else {
            let (t, i, _) = self.native_face_layers(&s, Some(n.opacity));
            (t, i)
        };
        let textured = |b: &MeshBuilder, tex: &Option<Rc<Texture>>, unlit: bool| {
            let m = native_material("white", 0.68, 0.0, n.opacity);
            {
                let mut mm = m.borrow_mut();
                mm.texture = tex.clone();
                mm.vector = tex.as_ref().and_then(|t| b.vector_for_texture(t));
                mm.unlit = unlit;
                if tex.is_none() {
                    mm.color.a = 0;
                }
            }
            m
        };
        let cap = textured(self, &tex, false);
        let ink = textured(self, &stroke, true);
        let mut wall = native_material(fill, 0.68, 0.0, n.opacity);
        if patterned && self.err.is_none() {
            let mut wall_source = s.clone();
            wall_source.type_ = SHAPE_RECTANGLE.into();
            wall_source.text = Text::default();
            wall_source.label_position.clear();
            wall_source.stroke = "transparent".into();
            wall_source.stroke_width = 0;
            wall_source.stroke_dash = 0.0;
            wall_source.double_border = false;
            wall_source.border_radius = 0;
            let (wall_tex, _) = self.native_face(&wall_source);
            wall = textured(self, &wall_tex, false);
        }
        let alpha = native_material(fill, 0.68, 0.0, n.opacity).borrow().color.a;
        // This texture paints an existing filled cap.
        let shadow_fill_alpha = (alpha > 0).then_some(alpha);
        if native_paint(fill, "transparent").a == 255 {
            cap.borrow_mut().svg_solid_texture = true;
            wall.borrow_mut().svg_solid_texture = true;
            let vector = native_vector_solid_cap(
                cap.borrow().vector.clone(),
                native_paint(fill, "transparent"),
            );
            let vector = match vector {
                Some(v) if d2_rs_scenebuild::is_gradient(fill) => {
                    let mut copy = (*v).clone();
                    copy.cap_background = None;
                    match d2_rs_scenebuild::gradient_paint(fill) {
                        Ok(paint) => copy.cap_paint = Some(paint),
                        Err(error) => {
                            copy.cap_paint = None;
                            self.err = Some(error);
                        }
                    }
                    Some(Rc::new(copy))
                }
                other => other,
            };
            cap.borrow_mut().vector = vector;
        }
        let ink_fit = self.solid_ink_fit(stroke.as_ref(), kind == SHAPE_HEXAGON);
        self.face_max_pixels = budget;
        NativeSolidPaint {
            cap,
            ink,
            wall,
            ink_fit,
            shadow_fill_alpha,
        }
    }

    /// Fit the complete painted extent inside the actual convex cap polygon.
    pub fn solid_ink_fit(&mut self, tex: Option<&Rc<Texture>>, hex: bool) -> f64 {
        let Some(tex) = tex else {
            return 1.0;
        };
        let mut radius: f64 = 1.0;
        let cos_step = gomath::cos(std::f64::consts::PI / NATIVE_UPRIGHT_SEGMENTS as f64);
        let mut include = |x: f64, y: f64| {
            if !hex {
                radius = go_max(radius, gomath::hypot(x, y) / cos_step);
                return;
            }
            for i in 0..6 {
                let (ax, ay) = native_upright_point(i, true);
                let (bx, by) = native_upright_point(i + 1, true);
                let (nx, ny) = (by - ay, ax - bx);
                // ARM64 native_solids.go:130 rounds ny*y and ny*ay,
                // then fuses nx*x and nx*ax into those products.
                radius = go_max(radius, go_fmadd(nx, x, ny * y) / go_fmadd(nx, ax, ny * ay));
            }
        };
        let pixels = tex.pixels.borrow();
        let (w, h) = (tex.width as i64, tex.height as i64);
        for y in 0..h {
            if y % 64 == 0 {
                if let Err(error) = self.context.check_string() {
                    self.err = Some(error);
                    return 1.0;
                }
            }
            let (mut x0, mut x1) = (w, 0);
            for x in 0..w {
                if pixels[((y * w + x) * 4 + 3) as usize] != 0 {
                    x0 = x0.min(x);
                    x1 = x1.max(x + 1);
                }
            }
            if x1 <= x0 {
                continue;
            }
            for x in [x0, x1] {
                for py in [y, y + 1] {
                    include(
                        2.0 * x as f64 / w as f64 - 1.0,
                        2.0 * py as f64 / h as f64 - 1.0,
                    );
                }
            }
        }
        1.0 / radius
    }

    /// The same source paint is applied to a convex cap and a rounded
    /// sidewall.
    pub fn solid_node(&mut self, n: &Node) {
        let mut fill = n.fill.clone();
        if native_token(&n.metadata.original.fill) && !n.fill_explicit {
            fill = "#dce5eb".into();
        }
        let mut kind = n.type_.as_str();
        if matches!(kind, SHAPE_CYLINDER | SHAPE_QUEUE | SHAPE_CIRCLE) {
            kind = SHAPE_OVAL;
        }
        let paint = self.solid_paint(n, kind, &fill);
        if self.err.is_some() {
            return;
        }
        let (w, d) = (n.size.x, n.size.z);
        let h = native_solid_height(n);
        let floor = n.position.y - n.size.y / 2.0;
        let scale = if self.scale <= 0.0 { 0.01 } else { self.scale };
        for copy in [1i64, 0] {
            if copy == 1 && !n.metadata.original.multiple {
                continue;
            }
            let c = copy as f64;
            // ARM64 native_solids.go:185–186.
            let center = nv(
                go_fmadd(c * MULTIPLE_OFFSET as f64, scale, n.position.x),
                go_fmsub(c, go_min(0.08, h * 0.18), floor + self.node_support_drop),
                go_fmsub(c * MULTIPLE_OFFSET as f64, scale, n.position.z),
            );
            if n.type_ == SHAPE_QUEUE {
                self.solid_barrel(
                    center,
                    w,
                    d,
                    h - self.node_support_drop,
                    &paint,
                    Some(native_queue_crown(n)),
                );
            } else {
                self.solid_upright(
                    center,
                    w,
                    d,
                    h - self.node_support_drop,
                    n.type_ == SHAPE_HEXAGON,
                    &paint,
                );
            }
        }
        // Plan support from the final painted content.
        let s = native_face_source(n, &fill);
        let first = self.triangles.len();
        self.canonical_node_content(n, &s, &fill, floor + h);
        if n.type_ != SHAPE_QUEUE {
            self.solid_content_support(n, &fill, floor, h, first);
        }
    }

    pub fn solid_content_support(
        &mut self,
        n: &Node,
        fill: &str,
        floor: f64,
        height: f64,
        first: usize,
    ) {
        if self.err.is_some() || first == self.triangles.len() {
            return;
        }
        let (left, right) = (n.position.x - n.size.x / 2.0, n.position.x + n.size.x / 2.0);
        let (front, back) = (n.position.z - n.size.z / 2.0, n.position.z + n.size.z / 2.0);
        let (mut x0, mut z0, mut x1, mut z1) = (
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        );
        let mut order: std::vec::Vec<MaterialRef> = vec![];
        let mut groups: std::vec::Vec<std::vec::Vec<usize>> = vec![];
        for (i, tri) in self.triangles[first..].iter().enumerate() {
            let Some(m) = &tri.material else { continue };
            if !tri.no_depth_write || m.borrow().texture.is_none() {
                continue;
            }
            match order.iter().position(|o| Rc::ptr_eq(o, m)) {
                Some(k) => groups[k].push(first + i),
                None => {
                    order.push(m.clone());
                    groups.push(vec![first + i]);
                }
            }
        }
        for (k, mat) in order.iter().enumerate() {
            let (mut lo_x, mut lo_z, mut hi_x, mut hi_z) = (
                f64::INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::NEG_INFINITY,
            );
            for &i in &groups[k] {
                for v in &self.triangles[i].v {
                    lo_x = go_min(lo_x, v.position.x);
                    lo_z = go_min(lo_z, v.position.z);
                    hi_x = go_max(hi_x, v.position.x);
                    hi_z = go_max(hi_z, v.position.z);
                }
            }
            let texture = mat.borrow().texture.clone().expect("textured");
            let Some([ix0, iy0, ix1, iy1]) = solid_texture_ink(&texture) else {
                continue;
            };
            let (tw, th) = (texture.width as f64, texture.height as f64);
            let a = go_max(left, lo_x + (hi_x - lo_x) * ix0 as f64 / tw);
            let c = go_max(front, lo_z + (hi_z - lo_z) * iy0 as f64 / th);
            let b = go_min(right, lo_x + (hi_x - lo_x) * ix1 as f64 / tw);
            let d = go_min(back, lo_z + (hi_z - lo_z) * iy1 as f64 / th);
            if b <= a || d <= c {
                continue; // authored outside captions stay outside
            }
            x0 = go_min(x0, a);
            z0 = go_min(z0, c);
            x1 = go_max(x1, b);
            z1 = go_max(z1, d);
        }
        if x1 <= x0 || z1 <= z0 {
            return;
        }
        let mut needs_plate = false;
        if n.type_ != SHAPE_HEXAGON {
            let inradius = gomath::cos(std::f64::consts::PI / NATIVE_UPRIGHT_SEGMENTS as f64);
            for x in [x0, x1] {
                for z in [z0, z1] {
                    let (qx, qz) = (
                        (x - n.position.x) / (n.size.x / 2.0),
                        (z - n.position.z) / (n.size.z / 2.0),
                    );
                    // ARM64 native_solids.go:250.
                    needs_plate = needs_plate || go_fmadd(qz, qz, qx * qx) > inradius * inradius;
                }
            }
        }
        if !needs_plate {
            return;
        }
        let (x0, z0, x1, z1) = (
            go_max(left, x0 - 0.025),
            go_max(front, z0 - 0.025),
            go_min(right, x1 + 0.025),
            go_min(back, z1 + 0.025),
        );
        let depth = go_min(0.02, height * 0.15);
        let content = self.triangles.split_off(first);
        let m = native_material(fill, 0.68, 0.0, n.opacity);
        self.box_(
            nv(
                (x0 + x1) / 2.0,
                floor + height - depth / 2.0,
                (z0 + z1) / 2.0,
            ),
            nv(x1 - x0, depth, z1 - z0),
            &m,
            0.008,
        );
        // Keep the label last for outside-caption clearance and its links.
        self.triangles.extend(content);
    }

    pub fn solid_cap_triangle(
        &mut self,
        a: Vertex,
        c: Vertex,
        d: Vertex,
        paint: &NativeSolidPaint,
    ) {
        let first = self.triangles.len();
        self.triangle(a, c, d, &paint.cap, true);
        for t in &mut self.triangles[first..] {
            t.shadow_fill_alpha = paint.shadow_fill_alpha;
        }
        let first = self.triangles.len();
        self.triangle(a, c, d, &paint.ink, false);
        for t in &mut self.triangles[first..] {
            // A raster depth bias separates coincident printed ink.
            t.depth_bias = 0.0005;
        }
    }

    pub fn solid_upright(
        &mut self,
        center: Vec,
        w: f64,
        d: f64,
        h: f64,
        hex: bool,
        paint: &NativeSolidPaint,
    ) {
        let count = if hex { 6 } else { NATIVE_UPRIGHT_SEGMENTS };
        let point = |i: i64| native_upright_point(i, hex);
        for i in 0..count {
            let vertex = |index: i64, height: f64| {
                let (x, z) = point(index);
                let (mut nx, mut nz) = (x / (w / 2.0), z / (d / 2.0));
                if hex {
                    let (ax, az) = point(i);
                    let (bx, bz) = point(i + 1);
                    (nx, nz) = ((bz - az) * d / 2.0, (ax - bx) * w / 2.0);
                }
                Vertex {
                    position: nadd(center, nv(x * w / 2.0, height, z * d / 2.0)),
                    normal: nunit(nv(nx, 0.0, nz)),
                    u: index as f64 / count as f64,
                    v: 1.0 - height / h,
                }
            };
            let (a, c, e, f) = (
                vertex(i, 0.0),
                vertex(i + 1, 0.0),
                vertex(i + 1, h),
                vertex(i, h),
            );
            self.triangle(a, f, e, &paint.wall, true);
            self.triangle(a, e, c, &paint.wall, true);
            let cap = |index: i64| {
                let (x, z) = point(index);
                paint.cap_vertex(
                    nadd(center, nv(x * w / 2.0, h, z * d / 2.0)),
                    nv(0.0, 1.0, 0.0),
                    (x + 1.0) / 2.0,
                    (z + 1.0) / 2.0,
                )
            };
            let mid = paint.cap_vertex(nadd(center, nv(0.0, h, 0.0)), nv(0.0, 1.0, 0.0), 0.5, 0.5);
            self.solid_cap_triangle(mid, cap(i + 1), cap(i), paint);
        }
    }

    /// Queue is a horizontal barrel with real end caps and a flat print crown.
    pub fn solid_barrel(
        &mut self,
        center: Vec,
        w: f64,
        d: f64,
        h: f64,
        paint: &NativeSolidPaint,
        crown: Option<f64>,
    ) {
        let cut = crown.map_or(1.0, |c| go_max(0.0, go_min(1.0, c)));
        let segments = NATIVE_BARREL_SEGMENTS as f64;
        for i in 0..NATIVE_BARREL_SEGMENTS {
            let (y0, _) = native_barrel_point(i, d, h, cut);
            let (y1, _) = native_barrel_point(i + 1, d, h, cut);
            let flat = (y0 - h).abs() < 1e-12 && (y1 - h).abs() < 1e-12;
            let point = |index: i64, x: f64| {
                let angle = index as f64 * 2.0 * std::f64::consts::PI / segments;
                let (y, z) = native_barrel_point(index, d, h, cut);
                let mut normal = nunit(nv(
                    0.0,
                    gomath::cos(angle) / (h / (1.0 + cut)),
                    gomath::sin(angle) / (d / 2.0),
                ));
                if flat {
                    normal = nv(0.0, 1.0, 0.0);
                }
                Vertex {
                    position: nadd(center, nv(x, y, z)),
                    normal,
                    u: x / w + 0.5,
                    v: index as f64 / segments,
                }
            };
            let (a, c, e, f) = (
                point(i, -w / 2.0),
                point(i + 1, -w / 2.0),
                point(i + 1, w / 2.0),
                point(i, w / 2.0),
            );
            self.triangle(a, e, f, &paint.wall, true);
            self.triangle(a, c, e, &paint.wall, true);
            for side in [-1.0, 1.0] {
                let cap = |index: i64| {
                    let angle = index as f64 * 2.0 * std::f64::consts::PI / segments;
                    let (y, z) = native_barrel_point(index, d, h, cut);
                    paint.cap_vertex(
                        nadd(center, nv(side * w / 2.0, y, z)),
                        nv(side, 0.0, 0.0),
                        (gomath::sin(angle) + 1.0) / 2.0,
                        (1.0 - gomath::cos(angle)) / 2.0,
                    )
                };
                let (p, q) = (cap(i), cap(i + 1));
                let mid = paint.cap_vertex(
                    nadd(center, nv(side * w / 2.0, h / 2.0, 0.0)),
                    nv(side, 0.0, 0.0),
                    0.5,
                    0.5,
                );
                if side > 0.0 {
                    self.solid_cap_triangle(mid, p, q, paint);
                } else {
                    self.solid_cap_triangle(mid, q, p, paint);
                }
            }
        }
    }
}

/// The visible (alpha > 3) pixel bounds as [x0, y0, x1, y1].
pub fn solid_texture_ink(tex: &Texture) -> Option<[usize; 4]> {
    let pixels = tex.pixels.borrow();
    let (mut x0, mut y0, mut x1, mut y1) = (tex.width, tex.height, 0, 0);
    for y in 0..tex.height {
        for x in 0..tex.width {
            if pixels[(y * tex.width + x) * 4 + 3] > 3 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    (x1 > x0 && y1 > y0).then_some([x0, y0, x1, y1])
}

/// Source ports lie on the D2 outline, which can extend beyond a solid's
/// curved wall; add only the short inward segment to reach the solid.
pub fn native_solid_contact_routes(
    edges: &[Edge],
    nodes: &[Node],
    paths: &[std::vec::Vec<Vec>],
    support: Option<&std::collections::HashMap<String, f64>>,
) -> Result<std::vec::Vec<std::vec::Vec<Vec>>, String> {
    native_solid_contact_routes_with_context(
        Some(&Default::default()),
        edges,
        nodes,
        paths,
        support,
    )
}

/// Keep the caller's cancellation cause through the final contact-routing stage.
/// solid_contacts.go:79–84,111–115 checks context before each route.
pub fn native_solid_contact_routes_with_context(
    context: Option<&super::context::RenderContext>,
    edges: &[Edge],
    nodes: &[Node],
    paths: &[std::vec::Vec<Vec>],
    support: Option<&std::collections::HashMap<String, f64>>,
) -> Result<std::vec::Vec<std::vec::Vec<Vec>>, String> {
    let Some(context) = context else {
        return Err("native solid contacts require a context".into());
    };
    if edges.len() != paths.len() {
        return Err("native solid contacts require one route per edge".into());
    }
    let solids: std::collections::HashMap<&str, &Node> = nodes
        .iter()
        .filter(|n| native_solid_node(n) && n.opacity > 0.0)
        .map(|n| (n.id.as_str(), n))
        .collect();
    let contact = |n: &Node, mut start: Vec, inward: Vec| -> (Vec, bool) {
        let y = start.y;
        if let Some(support) = support {
            let drop = support.get(&n.board_id).copied().unwrap_or(0.0);
            if drop < 0.0 {
                start.y = hierarchy_node_extension(n, drop).inverse_y(start.y);
            }
        }
        let (mut at, moved) = native_solid_contact(n, start, inward);
        at.y = y;
        (at, moved)
    };
    let mut out = std::vec::Vec::with_capacity(paths.len());
    for (i, points) in paths.iter().enumerate() {
        context.check_string()?;
        if points.len() < 2 || edges[i].opacity <= 0.0 {
            out.push(points.clone());
            continue;
        }
        let last = points.len() - 1;
        let (mut source, mut target) = (points[0], points[last]);
        let (mut source_moved, mut target_moved) = (false, false);
        if let Some(n) = solids.get(edges[i].source.as_str()) {
            (source, source_moved) = contact(n, points[0], nsub(points[0], points[1]));
        }
        if let Some(n) = solids.get(edges[i].target.as_str()) {
            (target, target_moved) = contact(n, points[last], nsub(points[last], points[last - 1]));
        }
        if !source_moved && !target_moved {
            out.push(points.clone());
            continue;
        }
        let mut route = std::vec::Vec::with_capacity(points.len() + 2);
        if source_moved {
            route.push(source);
        }
        route.extend_from_slice(points);
        if target_moved {
            route.push(target);
        }
        out.push(route);
    }
    Ok(out)
}

pub fn native_solid_contact(n: &Node, start: Vec, inward: Vec) -> (Vec, bool) {
    if n.type_ == SHAPE_QUEUE {
        return native_queue_contact(n, start, inward);
    }
    native_upright_contact(n, start, inward)
}

fn contact_inputs_finite(n: &Node, start: Vec, inward: Vec) -> bool {
    caption_finite(&[
        n.size.x,
        n.size.y,
        n.size.z,
        n.position.x,
        n.position.y,
        n.position.z,
        start.x,
        start.y,
        start.z,
        inward.x,
        inward.z,
    ])
}

/// Intersect the actual upright mesh's horizontal cross-section.
pub fn native_upright_contact(n: &Node, start: Vec, inward: Vec) -> (Vec, bool) {
    if !contact_inputs_finite(n, start, inward) || n.size.x <= 0.0 || n.size.z <= 0.0 {
        return (start, false);
    }
    let (w, d, height) = (n.size.x, n.size.z, native_solid_height(n));
    let relief = hierarchy_node_relief(n);
    let floor = n.position.y - n.size.y / 2.0;
    let y = (start.y - floor) / relief;
    if y <= 0.0 || y >= height {
        return (start, false);
    }
    let length = gomath::hypot(inward.x, inward.z);
    if length <= 1e-9 {
        return (start, false);
    }
    let direction = nv(inward.x / length, 0.0, inward.z / length);
    let (x, z) = (start.x - n.position.x, start.z - n.position.z);
    let mut scale = 0.01;
    if n.metadata.original.width > 0 {
        scale = w / n.metadata.original.width as f64;
    }
    let margin = go_min(0.05, go_max(1e-7, n.stroke_width as f64 * scale / 2.0));
    if x.abs() > w / 2.0 + margin || z.abs() > d / 2.0 + margin {
        return (start, false);
    }
    let hex = n.type_ == SHAPE_HEXAGON;
    let count = if hex { 6 } else { NATIVE_UPRIGHT_SEGMENTS };
    // radius is the constant 1, so the fused products are exact.
    let point = |i: i64| {
        let (x, z) = native_upright_point(i, hex);
        (x * w / 2.0, z * d / 2.0)
    };
    let (mut enter, mut leave) = (0.0, gomath::hypot(w, d) + 2.0 * margin);
    for i in 0..count {
        let (ax, az) = point(i);
        let (bx, bz) = point(i + 1);
        let (nx, nz) = (bz - az, ax - bx);
        // ARM64 solid_contacts.go:190.
        let room = go_fmadd(nz, az - z, nx * (ax - x));
        let rate = go_fmadd(nz, direction.z, nx * direction.x);
        if rate.abs() < 1e-12 {
            if room < -1e-10 {
                return (start, false);
            }
            continue;
        }
        let at = room / rate;
        if rate > 0.0 {
            leave = go_min(leave, at);
        } else {
            enter = go_max(enter, at);
        }
        if enter > leave + 1e-10 {
            return (start, false);
        }
    }
    if enter <= 1e-7 || !caption_finite(&[enter]) {
        return (start, false);
    }
    let mut contact = nadd(start, nmul(direction, enter));
    contact.y = start.y;
    (contact, true)
}

/// Clip a horizontal ray against the actual convex polygonal barrel.
pub fn native_queue_contact(n: &Node, start: Vec, inward: Vec) -> (Vec, bool) {
    if !contact_inputs_finite(n, start, inward) || n.size.x <= 0.0 || n.size.z <= 0.0 {
        return (start, false);
    }
    let (w, d, height) = (n.size.x, n.size.z, native_solid_height(n));
    let relief = hierarchy_node_relief(n);
    let (floor, h) = (n.position.y - n.size.y / 2.0, height * relief);
    // ARM64 solid_contacts.go:225: floor+h fuses height*relief.
    if start.y <= floor || start.y >= go_fmadd(height, relief, floor) {
        return (start, false);
    }
    let length = gomath::hypot(inward.x, inward.z);
    if length <= 1e-9 {
        return (start, false);
    }
    let direction = nv(inward.x / length, 0.0, inward.z / length);
    let (x, z) = (start.x - n.position.x, start.z - n.position.z);
    let mut scale = 0.01;
    if n.metadata.original.width > 0 {
        scale = w / n.metadata.original.width as f64;
    }
    let margin = go_min(0.05, go_max(1e-7, n.stroke_width as f64 * scale / 2.0));
    if x.abs() > w / 2.0 + margin || z.abs() > d / 2.0 + margin {
        return (start, false);
    }
    let (mut enter, mut leave) = (0.0, gomath::hypot(w, d) + 2.0 * margin);
    let mut clip = |rate: f64, room: f64| -> bool {
        if rate.abs() < 1e-12 {
            return room >= -1e-10;
        }
        let at = room / rate;
        if rate > 0.0 {
            leave = go_min(leave, at);
        } else {
            enter = go_max(enter, at);
        }
        enter <= leave + 1e-10
    };
    if !clip(direction.x, w / 2.0 - x) || !clip(-direction.x, w / 2.0 + x) {
        return (start, false);
    }
    let crown = native_queue_crown(n);
    for i in 0..NATIVE_BARREL_SEGMENTS {
        let (ay, az) = native_barrel_point(i, d, h, crown);
        let (by, bz) = native_barrel_point(i + 1, d, h, crown);
        let (ny, nz) = (bz - az, ay - by);
        // ARM64 solid_contacts.go:266.
        let room = go_fmadd(nz, az - z, ny * (ay - (start.y - floor)));
        if !clip(nz * direction.z, room) {
            return (start, false);
        }
    }
    if enter <= 1e-7 || !caption_finite(&[enter]) {
        return (start, false);
    }
    let mut contact = nadd(start, nmul(direction, enter));
    contact.y = start.y;
    (contact, true)
}

#[cfg(test)]
#[path = "solid_api_tests.rs"]
pub(crate) mod solid_api_tests;

#[cfg(test)]
#[path = "shape_api_tests.rs"]
mod shape_api_tests;
