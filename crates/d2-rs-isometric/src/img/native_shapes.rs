// SPDX-License-Identifier: MPL-2.0
//! d2isometricimg native_shapes.go: canonical D2 shape faces, source
//! contours and their physical extrusion. Contraction sites are marked
//! `ARM64:` (`.reference/isometric-port/fma-sites.txt`); products by 0.5 are
//! exact, so their fusion is unobservable.

use std::rc::Rc;

use d2_rs_render::go_float::{go_fmadd, go_fmsub};
use super::math as gomath;
use d2_rs_scene::{go_max, go_min, Box as SceneBox, Node as SceneNode, Primitive};
use d2_rs_scenebuild::golib::geo;
use d2_rs_scenebuild::golib::label::{self, Position};
use d2_rs_scenebuild::golib::shape::{self as libshape, Shape as _};
use d2_rs_scenebuild::golib::svgpath::PathCommandKind;

use super::backends::{self, rich_raster_options};
use super::geometry_core::native_polygon_area;
use super::geometry::{native_material, native_paint, native_token, readable_surface_ink, LabelSurface, MeshBuilder};
use super::markdown_cards::native_markdown_card;
use super::mesh::{nadd, ndot, nlen, nmul, nsub, nunit, nv, native_view_direction, MaterialRef, Texture, Vec, Vertex};
use super::native_edges::native_classic_rim;
use super::native_page::native_remove_page_glyph;
use super::native_solids::native_solid_node;
use super::route_labels::caption_finite;
use super::surface_document::{surface_texture_dimensions, surface_texture_dimensions_at_density};
use super::vector_surface::native_vector_aperture;
use crate::scene::Node;
use crate::target::{
    Shape, Text, MULTIPLE_OFFSET, SHAPE_C4_PERSON, SHAPE_CIRCLE, SHAPE_CLASS, SHAPE_CLOUD, SHAPE_CODE, SHAPE_HEXAGON,
    SHAPE_IMAGE, SHAPE_OVAL, SHAPE_PAGE, SHAPE_PERSON, SHAPE_RECTANGLE, SHAPE_SQL_TABLE, SHAPE_TEXT, THREE_DEE_OFFSET,
};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct NativeAnimatedNode {
    pub first: usize,
    pub last: usize,
}

/// Round solids retain their physical caps and curved walls. Every other
/// component uses D2's own contour and paint implementation.
pub fn native_canonical_node(n: &Node) -> bool {
    !native_solid_node(n)
}

pub fn native_relief_symbol(n: &Node) -> bool {
    matches!(n.type_.as_str(), SHAPE_CLOUD | SHAPE_PERSON | SHAPE_C4_PERSON)
}

/// Symbol depth follows the source footprint without changing X/Z
/// coordinates or print allocation.
pub fn native_canonical_height(n: &Node, scale: f64) -> f64 {
    let mut height = go_max(0.10, n.size.y * 1.15);
    match n.type_.as_str() {
        SHAPE_CLASS | SHAPE_SQL_TABLE => height = 0.46,
        SHAPE_CLOUD => height = 0.24 * go_min(n.size.x, n.size.z),
        SHAPE_PERSON | SHAPE_C4_PERSON => height = 0.19 * go_min(n.size.x, n.size.z),
        SHAPE_TEXT => {
            height = 0.018;
            if native_markdown_card(n) {
                height = 0.115;
            }
        }
        _ => {}
    }
    if n.metadata.original.three_dee {
        let mut scale = scale;
        if scale <= 0.0 {
            scale = 0.01;
            if n.metadata.original.width > 0 {
                scale = n.size.x / n.metadata.original.width as f64;
            }
        }
        // ARM64 native_shapes.go:61.
        height = go_fmadd(THREE_DEE_OFFSET as f64, scale, height);
    }
    height
}

pub fn native_face_source(n: &Node, fill: &str) -> Shape {
    let mut s = n.metadata.original.clone();
    s.id = "native-face".into();
    s.pos = Default::default();
    if s.width <= 0 {
        s.width = 1.max(gomath::ceil(n.size.x / 0.01) as i64);
    }
    if s.height <= 0 {
        s.height = 1.max(gomath::ceil(n.size.z / 0.01) as i64);
    }
    s.text = Text::default();
    s.icon = None;
    s.link.clear();
    s.tooltip.clear();
    s.pretty_link.clear();
    s.tooltip_position.clear();
    s.label_position.clear();
    // Keep the source border-label aperture without painting its text twice.
    let original = &n.metadata.original;
    if Position::from_string(&original.label_position).is_border() && !original.text.label.is_empty() {
        s.text = Text {
            label: " ".into(),
            label_width: original.text.label_width,
            label_height: original.text.label_height,
            font_size: 1.max(original.text.font_size),
            color: "transparent".into(),
            label_fill: "transparent".into(),
            ..Default::default()
        };
        s.label_position = original.label_position.clone();
    }
    s.classes = None;
    s.fields = None;
    s.methods = None;
    s.columns = None;
    s.multiple = false;
    s.shadow = false;
    s.animated = false;
    s.three_dee = false;
    s.blend = false;
    s.opacity = 1.0;
    s.fill = fill.to_owned();
    s.stroke = n.stroke.clone();
    s.stroke_width = n.stroke_width;
    s.stroke_dash = n.stroke_dash;
    if n.type_ == SHAPE_CLASS || n.type_ == SHAPE_SQL_TABLE {
        // The structured document owns row/header fills.
        s.fill = "transparent".into();
        s.stroke = n.fill.clone();
    }
    if matches!(s.type_.as_str(), SHAPE_CLASS | SHAPE_SQL_TABLE | SHAPE_TEXT | SHAPE_CODE | SHAPE_IMAGE) {
        s.type_ = SHAPE_RECTANGLE.into();
    }
    s
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeFaceLayer {
    Combined,
    Fill,
    Ink,
}

pub fn native_select_face_layer(n: &mut SceneNode, layer: NativeFaceLayer) {
    if layer == NativeFaceLayer::Combined {
        return;
    }
    let fill_only = layer == NativeFaceLayer::Fill;
    match &mut n.primitive {
        Some(Primitive::Rect(p)) => {
            if fill_only {
                p.stroke = None
            } else {
                p.fill = None
            }
        }
        Some(Primitive::Ellipse(p)) => {
            if fill_only {
                p.stroke = None
            } else {
                p.fill = None
            }
        }
        Some(Primitive::Path(p)) => {
            if fill_only {
                p.stroke = None
            } else {
                p.fill = None
            }
        }
        // The only face text is an invisible border-aperture placeholder.
        Some(Primitive::TextRun(_)) => n.primitive = None,
        _ => {}
    }
    for child in n.children.iter_mut().flatten() {
        native_select_face_layer(child, layer);
    }
}

impl MeshBuilder {
    /// Face paint is generated by the same bounded native scene renderer as
    /// SVG export.
    pub fn native_face(&mut self, s: &Shape) -> (Option<Rc<Texture>>, LabelSurface) {
        self.native_face_layer(s, NativeFaceLayer::Combined)
    }

    /// Both layers retain the original document viewport.
    pub fn native_face_layers(
        &mut self,
        s: &Shape,
        opacity: Option<f64>,
    ) -> (Option<Rc<Texture>>, Option<Rc<Texture>>, LabelSurface) {
        let budget = self.face_max_pixels;
        if budget > 0 {
            self.face_max_pixels = 1.max(budget / 2);
        }
        let (fill, area) = self.native_face_layer(s, NativeFaceLayer::Fill);
        let mut ink = None;
        if self.err.is_none() {
            ink = self.native_face_layer(s, NativeFaceLayer::Ink).0;
        }
        if let (None, Some(opacity), Some(f), Some(i)) = (&self.err, opacity, &fill, &ink) {
            if opacity < 1.0 {
                self.native_face_opacity(f, i, go_max(0.0, opacity));
            }
        }
        self.face_max_pixels = budget;
        (fill, ink, area)
    }

    /// Splitting one source paint into two physical materials must not apply
    /// its group opacity twice where ink covers fill.
    pub fn native_face_opacity(&mut self, fill: &Rc<Texture>, ink: &Rc<Texture>, opacity: f64) {
        if let Some(surface) = self.vector_for_texture(fill) {
            *surface.ink_coverage.borrow_mut() = self.vector_for_texture(ink);
            surface.coverage_opacity.set(opacity);
        }
        let ink_pixels = ink.pixels.borrow();
        let mut pixels = fill.pixels.borrow_mut();
        for y in 0..fill.height {
            for x in 0..fill.width {
                let a = ink_pixels[(y * ink.width + x) * 4 + 3] as f64 / 255.0;
                // ARM64 native_shapes.go:152: FMSUBD.
                let factor = (1.0 - a) / go_fmsub(a, opacity, 1.0);
                let i = (y * fill.width + x) * 4;
                for channel in 0..4 {
                    pixels[i + channel] = (pixels[i + channel] as f64 * factor).round() as u8;
                }
            }
        }
    }

    pub fn native_face_layer(&mut self, s: &Shape, layer: NativeFaceLayer) -> (Option<Rc<Texture>>, LabelSurface) {
        // native_shapes.go:196 passes the builder context into scenebuild.Build.
        // Reject cancellation before the adapter builds or allocates a face.
        if let Err(error) = self.context.check_string() {
            self.err = Some(format!("native shape face: {error}"));
            return (None, LabelSurface::default());
        }
        let mut doc = match backends::build_face_document(s) {
            Ok(doc) => doc,
            Err(error) => {
                self.fail(format!("native shape face: {error}"));
                return (None, LabelSurface::default());
            }
        };
        if let Some(root) = doc.root.as_deref_mut() {
            if s.type_ == SHAPE_PAGE && layer == NativeFaceLayer::Ink && native_paint(&s.fill, "white").a > 0 {
                native_remove_page_glyph(root);
            }
            native_select_face_layer(root, layer);
        }
        let mut opts = rich_raster_options();
        let (vw, vh) = (doc.view_box.width, doc.view_box.height);
        opts.scale = go_min(2.0, 1024.0 / go_max(vw, vh));
        if self.face_max_pixels > 0 {
            opts.scale = go_min(opts.scale, gomath::sqrt(self.face_max_pixels as f64 / (vw * vh))) * 0.999;
        }
        opts.max_decoded_asset_bytes = 32 * 1024 * 1024;
        let mut pixels = gomath::ceil(vw * opts.scale) as i64 * gomath::ceil(vh * opts.scale) as i64;
        let mut max_pixels: i64 = 1024 * 1024;
        if self.face_max_pixels > 0 && pixels > self.face_max_pixels {
            // A square-root scale limit alone can overshoot by one row/column.
            let maximum = 1.max(gomath::ceil(go_max(vw, vh) * opts.scale) as i64);
            let (w, h) = surface_texture_dimensions(vw, vh, maximum, self.face_max_pixels);
            doc.logical_width = w as f64;
            doc.logical_height = h as f64;
            doc.viewport_fit = d2_rs_scene::ViewportFit::STRETCH;
            opts.scale = 1.0;
            pixels = w * h;
        }
        if self.output_density > 0.0 && caption_finite(&[self.output_density]) {
            let scale = if self.scale <= 0.0 { 0.01 } else { self.scale };
            max_pixels = 4 * 1024 * 1024;
            let mut budget = max_pixels;
            if self.face_max_pixels > 0 {
                budget = budget.min(self.face_max_pixels);
            }
            let (w, h) = surface_texture_dimensions_at_density(vw * scale, vh * scale, 4096, budget, self.output_density);
            doc.logical_width = w as f64;
            doc.logical_height = h as f64;
            doc.viewport_fit = d2_rs_scene::ViewportFit::STRETCH;
            opts.scale = 1.0;
            pixels = w * h;
        }
        if pixels <= 0 || pixels > max_pixels || self.face_pixels > 24 * 1024 * 1024 - pixels {
            self.fail("native shape faces exceed texture pixel budget");
            return (None, LabelSurface::default());
        }
        self.face_pixels += pixels;
        let tex = match backends::render(&doc, &opts) {
            Ok(tex) => Rc::new(tex),
            Err(error) => {
                self.fail(format!("native shape face raster: {error}"));
                return (None, LabelSurface::default());
            }
        };
        if let Err(error) = self.retain_vector_surface(&tex, &doc) {
            self.fail(error);
            return (None, LabelSurface::default());
        }
        let area = LabelSurface {
            center: nv(doc.view_box.x + vw / 2.0, 0.0, doc.view_box.y + vh / 2.0),
            width: vw,
            depth: vh,
            ..Default::default()
        };
        if layer != NativeFaceLayer::Fill {
            self.native_face_border_aperture(&tex, &area, s);
        }
        let visible = tex.pixels.borrow().chunks_exact(4).any(|p| p[3] != 0);
        (visible.then_some(tex), area)
    }

    /// Border labels have a narrow opening in the shape outline.
    pub fn native_face_border_aperture(&mut self, tex: &Rc<Texture>, area: &LabelSurface, s: &Shape) {
        let position = Position::from_string(&s.label_position);
        if s.text.label.is_empty() || !position.is_border() || s.stroke_width <= 0 {
            return;
        }
        let bx = geo::Box::new(geo::Point::new(s.pos.x as f64, s.pos.y as f64), s.width as f64, s.height as f64);
        let p = position.get_point_on_box(&bx, label::PADDING, s.text.label_width as f64, s.text.label_height as f64);
        let stroke = s.stroke_width as f64;
        let (lw, lh) = ((s.text.label_width + 4) as f64, (s.text.label_height + 4) as f64);
        use Position::*;
        let (x, y, w, h) = match position {
            BorderTopLeft | BorderTopCenter | BorderTopRight => (p.x - 2.0, bx.top_left.y - stroke / 2.0, lw, stroke),
            BorderBottomLeft | BorderBottomCenter | BorderBottomRight => {
                (p.x - 2.0, bx.top_left.y + bx.height - stroke / 2.0, lw, stroke)
            }
            BorderLeftTop | BorderLeftMiddle | BorderLeftBottom => (bx.top_left.x - stroke / 2.0, p.y - 2.0, stroke, lh),
            BorderRightTop | BorderRightMiddle | BorderRightBottom => {
                (bx.top_left.x + bx.width - stroke / 2.0, p.y - 2.0, stroke, lh)
            }
            _ => (0.0, 0.0, 0.0, 0.0),
        };
        if let Some(surface) = self.vector_for_texture(tex) {
            native_vector_aperture(&surface, SceneBox { x, y, width: w, height: h });
        }
        let mut pixels = tex.pixels.borrow_mut();
        let sx = tex.width as f64 / area.width;
        let sy = tex.height as f64 / area.depth;
        let x0 = (x - area.center.x + area.width / 2.0) * sx;
        let y0 = (y - area.center.z + area.depth / 2.0) * sy;
        // ARM64 native_shapes.go:301.
        let (x1, y1) = (go_fmadd(w, sx, x0), go_fmadd(h, sy, y0));
        let py_end = (tex.height as i64).min(gomath::ceil(y1) as i64);
        let px_end = (tex.width as i64).min(gomath::ceil(x1) as i64);
        let mut py = 0.max(gomath::floor(y0) as i64);
        while py < py_end {
            let mut px = 0.max(gomath::floor(x0) as i64);
            while px < px_end {
                let cx = go_min((px + 1) as f64, x1) - go_max(px as f64, x0);
                let cy = go_min((py + 1) as f64, y1) - go_max(py as f64, y0);
                let i = (py as usize * tex.width + px as usize) * 4;
                for c in 0..4 {
                    // ARM64 native_shapes.go:307: `1 - coverage` is FMSUBD.
                    pixels[i + c] = (pixels[i + c] as f64 * go_fmsub(cx, cy, 1.0)).round() as u8;
                }
                px += 1;
            }
            py += 1;
        }
    }

    pub fn canonical_node(&mut self, n: &Node, _tint: &str) {
        let scale = if self.scale <= 0.0 { 0.01 } else { self.scale };
        let mut fill = n.fill.clone();
        if native_token(&n.metadata.original.fill) && !n.fill_explicit {
            fill = "#dce5eb".into();
        }
        if n.type_ == SHAPE_CLASS || n.type_ == SHAPE_SQL_TABLE {
            fill = n.stroke.clone();
        }
        let s = native_face_source(n, &fill);
        let profiles = match native_shape_profiles(&s) {
            Ok(p) => p,
            Err(error) => {
                self.fail(error);
                return;
            }
        };
        let paint_opacity = if native_classic_rim(n) { 1.0 } else { n.opacity };
        let (tex, ink, face) = self.native_face_layers(&s, Some(paint_opacity));
        if self.err.is_some() {
            return;
        }
        let (w, d) = (n.size.x, n.size.z);
        let (sx, sz) = (w / s.width as f64, d / s.height as f64);
        let base = n.position.y - n.size.y / 2.0;
        let height = native_canonical_height(n, scale);
        let top = base + height;
        let mat = native_material(&fill, 0.68, 0.0, n.opacity);
        let side = super::mesh::material(mat.borrow().clone());
        for copy_index in [1i64, 0] {
            if copy_index == 1 && !n.metadata.original.multiple {
                continue;
            }
            // D2's multiple modifier preserves its source offset right and up.
            let c = copy_index as f64;
            let (dx_k, dz_k, dy_k) = (c * MULTIPLE_OFFSET as f64, -c * MULTIPLE_OFFSET as f64, -c);
            let dy_m = go_min(0.045, height * 0.25);
            // ARM64 native_shapes.go:356/369/380: the +dx, +dz and +dy
            // products fuse into their sums.
            let place_x = |v: f64| go_fmadd(dx_k, scale, v);
            let place_z = |v: f64| go_fmadd(dz_k, scale, v);
            let place_y = |v: f64| go_fmadd(dy_k, dy_m, v);
            let mut page_profile: std::vec::Vec<Vec> = vec![];
            for profile in &profiles {
                let world: std::vec::Vec<Vec> = profile
                    .iter()
                    .map(|p| nv(place_x(n.position.x - w / 2.0 + p.x * sx), place_y(top), place_z(n.position.z - d / 2.0 + p.z * sz)))
                    .collect();
                if n.type_ == SHAPE_PAGE {
                    page_profile = world.clone();
                }
                let mut cap = None;
                if self.options.vector && n.opacity == 1.0 && native_paint(&fill, "transparent").a == 255 {
                    cap = Some(mat.clone());
                }
                let start = self.triangles.len();
                let bottom = place_y(base) + self.node_support_drop;
                self.extruded_profile(&world, bottom, cap.as_ref(), &side);
                if let Some(cap) = &cap {
                    for t in &mut self.triangles[start..] {
                        if t.material.as_ref().is_some_and(|m| Rc::ptr_eq(m, cap)) {
                            t.svg_coverage_only = true;
                            t.cast_shadow = false;
                        }
                    }
                }
            }
            let mut paint = face.clone();
            paint.center = nv(
                place_x(n.position.x - w / 2.0 + face.center.x * sx),
                place_y(top) + 0.0005,
                place_z(n.position.z - d / 2.0 + face.center.z * sz),
            );
            paint.width = face.width * sx;
            paint.depth = face.depth * sz;
            let before = self.triangles.len();
            self.surface_texture(tex.as_ref(), &paint, n.opacity);
            for t in &mut self.triangles[before..] {
                if let Some(m) = &t.material {
                    let mut m = m.borrow_mut();
                    m.unlit = false;
                    m.roughness = 0.68;
                    m.metalness = 0.0;
                }
                t.no_depth_write = false;
                t.cast_shadow = true;
            }
            // Authored linework keeps its actual ink color.
            if !native_classic_rim(n) {
                self.surface_texture(ink.as_ref(), &paint, n.opacity);
            }
            if n.type_ == SHAPE_PAGE {
                self.native_page_fold(n, &paint, tex.as_ref(), &mat, c * MULTIPLE_OFFSET as f64 * scale, -c * MULTIPLE_OFFSET as f64 * scale, place_y(top), &page_profile);
            }
        }
        self.canonical_node_content(n, &s, &fill, top);
    }

    pub fn canonical_node_content(&mut self, n: &Node, s: &Shape, fill: &str, top: f64) {
        let mut surface = native_node_label_surface(n, s, top + 0.0015);
        let mut ink = readable_surface_ink(fill, n.opacity);
        if n.font_explicit {
            ink = n.font_color.clone();
        }
        if n.metadata.original.icon.is_some() {
            let icon_face = native_node_icon_surface(n, s, top + 0.0015);
            let mut icon_source = n.metadata.original.clone();
            if n.type_ == SHAPE_IMAGE {
                icon_source.text.label.clear();
            }
            let inside_label = self.shape_icon(&icon_source, icon_face, n.opacity, "node", None);
            let position = Position::from_string(&n.metadata.original.label_position);
            if n.type_ != SHAPE_IMAGE && !position.is_outside() && !position.is_border() {
                surface = inside_label;
            }
        }
        self.shape_label(&n.metadata.original, &surface, &ink, n.opacity, "node");
    }

    /// A front-facing sidewall occupies some of the outside caption's
    /// projected gap; expand that gap by the wall's projected height.
    pub fn clear_outside_caption(&mut self, n: &Node, first: usize, relief: f64) {
        if self.err.is_some() || n.metadata.original.text.label.is_empty() || self.triangles.len() < first + 2 {
            return;
        }
        let len = self.triangles.len();
        for t in &self.triangles[len - 2..] {
            if !t.no_depth_write || t.material.as_ref().is_none_or(|m| m.borrow().texture.is_none()) {
                return;
            }
        }
        let direction = native_view_direction();
        let height = go_max(0.0, self.triangles[len - 2].v[0].position.y - (n.position.y - n.size.y / 2.0)) * relief;
        let horizontal = direction.x * direction.x + direction.z * direction.z;
        let (mut dx, mut dz) = (0.0, 0.0);
        use Position::*;
        match Position::from_string(&n.metadata.original.label_position) {
            OutsideBottomLeft | OutsideBottomCenter | OutsideBottomRight => {
                dz = height * horizontal / (direction.y * direction.z)
            }
            OutsideRightTop | OutsideRightMiddle | OutsideRightBottom => dx = height * horizontal / (direction.y * direction.x),
            _ => return,
        }
        for t in &mut self.triangles[len - 2..] {
            for v in &mut t.v {
                v.position.x += dx;
                v.position.z += dz;
            }
        }
    }


}

pub fn native_node_icon_surface(n: &Node, s: &Shape, y: f64) -> LabelSurface {
    if matches!(n.type_.as_str(), SHAPE_IMAGE | SHAPE_CLASS | SHAPE_SQL_TABLE) {
        return LabelSurface { center: nv(n.position.x, y, n.position.z), width: n.size.x, depth: n.size.z, ..Default::default() };
    }
    let geometry = libshape::new_shape(
        libshape::dsl_shape_to_shape_type(&s.type_),
        geo::Box::new(geo::Point::new(0.0, 0.0), s.width as f64, s.height as f64),
    );
    let bx = geometry.get_inner_box();
    let (sx, sz) = (n.size.x / s.width as f64, n.size.z / s.height as f64);
    // ARM64 native_shapes.go:436.
    LabelSurface {
        center: nv(
            go_fmadd(bx.top_left.x + bx.width / 2.0, sx, n.position.x - n.size.x / 2.0),
            y,
            go_fmadd(bx.top_left.y + bx.height / 2.0, sz, n.position.z - n.size.z / 2.0),
        ),
        width: bx.width * sx,
        depth: bx.height * sz,
        ..Default::default()
    }
}

pub fn native_node_label_surface(n: &Node, s: &Shape, y: f64) -> LabelSurface {
    let (sx, sz) = (n.size.x / s.width as f64, n.size.z / s.height as f64);
    if n.type_ == SHAPE_CLASS || n.type_ == SHAPE_SQL_TABLE {
        return LabelSurface { center: nv(n.position.x, y, n.position.z), width: n.size.x, depth: n.size.z, ..Default::default() };
    }
    let geometry = libshape::new_shape(
        libshape::dsl_shape_to_shape_type(&s.type_),
        geo::Box::new(geo::Point::new(0.0, 0.0), s.width as f64, s.height as f64),
    );
    let mut position = Position::from_string(&n.metadata.original.label_position);
    if position == Position::Unset {
        position = Position::InsideMiddleCenter;
    }
    let mut bx = geometry.get_inner_box();
    if position.is_outside() || position.is_border() {
        bx = geometry.get_box();
        if n.metadata.original.three_dee {
            let mut offset_y = THREE_DEE_OFFSET as f64;
            if n.type_ == SHAPE_HEXAGON {
                offset_y /= 2.0;
            }
            bx.top_left.y -= offset_y;
            bx.height += offset_y;
            bx.width += THREE_DEE_OFFSET as f64;
        } else if n.metadata.original.multiple {
            bx.top_left.y -= MULTIPLE_OFFSET as f64;
            bx.height += MULTIPLE_OFFSET as f64;
            bx.width += MULTIPLE_OFFSET as f64;
        }
    }
    let (mut w, mut h) = (n.metadata.original.text.label_width as f64, n.metadata.original.text.label_height as f64);
    if w <= 0.0 || h <= 0.0 {
        (w, h) = (bx.width, bx.height);
    }
    let tl = position.get_point_on_box(&bx, label::PADDING, w, h);
    // ARM64 native_shapes.go:504.
    LabelSurface {
        center: nv(go_fmadd(tl.x + w / 2.0, sx, n.position.x - n.size.x / 2.0), y, go_fmadd(tl.y + h / 2.0, sz, n.position.z - n.size.z / 2.0)),
        width: w * sx,
        depth: h * sz,
        ..Default::default()
    }
}

/// Flatten source paths in source pixels before applying the scene scale.
pub fn native_shape_profile(s: &Shape, mut contours: Option<&mut std::vec::Vec<std::vec::Vec<Vec>>>) -> Result<std::vec::Vec<Vec>, String> {
    let (w, h) = (1.max(s.width) as f64, 1.max(s.height) as f64);
    let obj = libshape::new_shape(libshape::dsl_shape_to_shape_type(&s.type_), geo::Box::new(geo::Point::new(0.0, 0.0), w, h));
    let paths = obj.get_path_commands().unwrap_or_default();
    if paths.is_empty() {
        if s.type_ == SHAPE_CIRCLE || s.type_ == SHAPE_OVAL {
            let count = 96;
            return Ok((0..count)
                .map(|i| {
                    let a = 2.0 * std::f64::consts::PI * i as f64 / count as f64;
                    // ARM64 native_shapes.go:519.
                    nv(go_fmadd(w / 2.0, gomath::cos(a), w / 2.0), 0.0, go_fmadd(h / 2.0, gomath::sin(a), h / 2.0))
                })
                .collect());
        }
        let r = go_min(s.border_radius as f64, go_min(w, h) / 2.0);
        if r <= 0.0 {
            return Ok(vec![nv(0.0, 0.0, 0.0), nv(w, 0.0, 0.0), nv(w, 0.0, h), nv(0.0, 0.0, h)]);
        }
        let mut p = std::vec::Vec::with_capacity(36);
        for (i, c) in [nv(w - r, 0.0, r), nv(w - r, 0.0, h - r), nv(r, 0.0, h - r), nv(r, 0.0, r)].into_iter().enumerate() {
            for k in 0..=8 {
                // The π/2 and π/16 products are exact scalings.
                let a = -std::f64::consts::PI / 2.0 + i as f64 * std::f64::consts::PI / 2.0 + k as f64 * std::f64::consts::PI / 16.0;
                // ARM64 native_shapes.go:531.
                p.push(nv(go_fmadd(r, gomath::cos(a), c.x), 0.0, go_fmadd(r, gomath::sin(a), c.z)));
            }
        }
        return Ok(p);
    }
    let mut best: std::vec::Vec<Vec> = vec![];
    let mut best_area = 0.0;
    for commands in &paths {
        let mut p: std::vec::Vec<Vec> = vec![];
        let mut at = Vec::default();
        let mut closed = false;
        let add = |p: &mut std::vec::Vec<Vec>, v: Vec| -> Result<(), String> {
            if p.len() >= 2048 {
                return Err("native shape profile exceeds 2048 vertices".into());
            }
            if p.last().is_none_or(|last| nlen(nsub(*last, v)) > 1e-8) {
                p.push(v);
            }
            Ok(())
        };
        for c in commands {
            let end = nv(c.end.x, 0.0, c.end.y);
            match c.kind {
                PathCommandKind::Close => closed = true,
                PathCommandKind::Move | PathCommandKind::Line => {
                    add(&mut p, end)?;
                    at = end;
                }
                PathCommandKind::Cubic => {
                    let (c1, c2) = (nv(c.control1.x, 0.0, c.control1.y), nv(c.control2.x, 0.0, c.control2.y));
                    let length = nlen(nsub(at, c1)) + nlen(nsub(c1, c2)) + nlen(nsub(c2, end));
                    let steps = 4.max(64.min(gomath::ceil(length / 4.0) as i64));
                    for j in 1..=steps {
                        let t = j as f64 / steps as f64;
                        let u = 1.0 - t;
                        let v = nadd(nadd(nmul(at, u * u * u), nmul(c1, 3.0 * u * u * t)), nadd(nmul(c2, 3.0 * u * t * t), nmul(end, t * t * t)));
                        add(&mut p, v)?;
                    }
                    at = end;
                }
                #[allow(unreachable_patterns)]
                _ => {}
            }
        }
        if p.len() > 1 && nlen(nsub(p[0], p[p.len() - 1])) < 1e-7 {
            closed = true;
            p.pop();
        }
        let a = native_polygon_area(&p).abs();
        if closed && p.len() >= 3 {
            if let Some(contours) = contours.as_deref_mut() {
                contours.push(p.clone());
            }
        }
        if a > best_area {
            best = p;
            best_area = a;
        }
    }
    if best.len() < 3 {
        return Err(format!("native shape {:?} has no closed profile", s.type_));
    }
    Ok(best)
}

pub fn native_shape_profiles(s: &Shape) -> Result<std::vec::Vec<std::vec::Vec<Vec>>, String> {
    let mut contours = vec![];
    let outer = native_shape_profile(s, Some(&mut contours))?;
    if contours.is_empty() {
        return Ok(vec![outer]);
    }
    let inside = |p: Vec, polygon: &[Vec]| -> bool {
        let mut hit = false;
        for (i, a) in polygon.iter().enumerate() {
            let c = polygon[(i + 1) % polygon.len()];
            let (dx, dz) = (c.x - a.x, c.z - a.z);
            // ARM64 native_shapes.go:607: FMSUBD.
            if go_fmsub(p.z - a.z, dx, (p.x - a.x) * dz).abs() < 1e-6
                && p.x >= go_min(a.x, c.x) - 1e-6
                && p.x <= go_max(a.x, c.x) + 1e-6
                && p.z >= go_min(a.z, c.z) - 1e-6
                && p.z <= go_max(a.z, c.z) + 1e-6
            {
                return true;
            }
            if (a.z > p.z) != (c.z > p.z) && p.x < (c.x - a.x) * (p.z - a.z) / (c.z - a.z) + a.x {
                hit = !hit;
            }
        }
        hit
    };
    let mut result = vec![];
    for (i, p) in contours.iter().enumerate() {
        let contained = contours.iter().enumerate().any(|(j, q)| {
            i != j && native_polygon_area(q).abs() > native_polygon_area(p).abs() && p.iter().all(|v| inside(*v, q))
        });
        if !contained {
            result.push(p.clone());
        }
    }
    Ok(result)
}
