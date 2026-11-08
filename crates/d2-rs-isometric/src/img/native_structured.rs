// SPDX-License-Identifier: MPL-2.0
//! native_structured.go: individual source-coordinate rows and printed cells.
use super::{
    geometry::{LabelSurface, MeshBuilder, native_material, native_paint},
    mesh::{OpacityGroup, nv},
    native_shapes::{native_canonical_height, native_face_source, native_shape_profiles},
    rich_labels::{
        MAX_RICH_PIXELS, MAX_RICH_SOURCE_BYTES, MAX_RICH_TOTAL_ROWS, rich_label_document,
        rich_source_size,
    },
    surface_document::surface_texture_dimensions_at_density,
};
use crate::{
    scene::Node,
    target::{MULTIPLE_OFFSET, SHAPE_CLASS, SHAPE_RECTANGLE, SHAPE_SQL_TABLE, Shape, Text},
};
use d2_rs_render::go_float::{go_fmadd, go_fmsub};
use d2_rs_scene::{
    Box as SceneBox, Document, Node as SceneNode, Primitive, ViewportFit, go_max, go_min,
};
use d2_rs_scenebuild::golib::{
    geo,
    label::{self, Position},
};
use std::{cell::RefCell, rc::Rc};
pub fn native_structured_node(n: &Node) -> bool {
    matches!(n.type_.as_str(), SHAPE_CLASS | SHAPE_SQL_TABLE)
}
#[derive(Clone)]
pub(crate) struct StructuredRow {
    pub(crate) z: f64,
    pub(crate) depth: f64,
    pub(crate) back: f64,
    pub(crate) front: f64,
    section: &'static str,
}
pub(crate) fn structured_rows(s: &Shape) -> Vec<StructuredRow> {
    let fields = s.fields.as_ref().map_or(0, Vec::len);
    let methods = s.methods.as_ref().map_or(0, Vec::len);
    let (count, header_rows) = if s.type_ == SHAPE_CLASS {
        (fields + methods, 2)
    } else {
        (s.columns.as_ref().map_or(0, Vec::len), 1)
    };
    let rh = s.height as f64 / (count + header_rows) as f64;
    let mut hh = rh * header_rows as f64;
    if s.type_ == SHAPE_CLASS {
        hh = go_max(hh, s.text.label_height as f64 + 10.0);
    }
    let mut rows = vec![StructuredRow {
        z: 0.0,
        depth: hh,
        back: 0.0,
        front: 0.0,
        section: "header",
    }];
    for i in 0..count {
        let z = hh + i as f64 * rh;
        rows.push(StructuredRow {
            z,
            depth: go_min(rh, go_max(0.0, s.height as f64 - z)),
            back: 0.0,
            front: 0.0,
            section: if s.type_ == SHAPE_CLASS && i >= fields {
                "method"
            } else {
                "field"
            },
        });
    }
    for i in 0..rows.len() {
        let gap = if i == 0 { 1.2 } else { 3.8 };
        let pad = go_max(0.0, (rows[i].depth - s.text.font_size as f64) / 2.0 - 1.0);
        rows[i].back = go_min(gap, pad * 0.5);
        rows[i].front = rows[i].back;
        if i > 1 && rows[i].section != rows[i - 1].section {
            rows[i].back = go_min(7.5, pad * 0.8);
        }
        if i > 0 && i + 1 < rows.len() && rows[i].section != rows[i + 1].section {
            rows[i].front = go_min(7.5, pad * 0.8);
        }
    }
    rows
}
fn text_row(id: &str, s: &Shape) -> isize {
    if id.contains("header-label") {
        return 0;
    }
    let parts: Vec<_> = id.split(':').collect();
    if parts.len() < 4 {
        return -1;
    }
    let Ok(mut i) = parts[2].parse::<isize>() else {
        return -1;
    };
    if parts[1] == "class-method" {
        i += s.fields.as_ref().map_or(0, Vec::len) as isize;
    }
    i + 1
}
impl MeshBuilder {
    fn structured_document(
        &mut self,
        n: &Node,
    ) -> Result<(Document, Vec<(SceneNode, isize)>), String> {
        let p = &mut self.rich;
        if p.used >= p.count {
            return Err("isometric rich label allocation exceeds its declared count".into());
        }
        let (bytes, rows) = rich_source_size(&n.metadata.original)?;
        if bytes > MAX_RICH_SOURCE_BYTES - p.source_bytes || rows > MAX_RICH_TOTAL_ROWS - p.rows {
            return Err("isometric rich labels exceed aggregate source or row budget".into());
        }
        p.source_bytes += bytes;
        p.rows += rows;
        p.used += 1;
        let surface = LabelSurface {
            width: n.size.x,
            depth: n.size.z,
            ..Default::default()
        };
        let style = self.print_style(
            &surface,
            &n.metadata.original.text,
            &n.font_color,
            n.opacity,
            "node",
        );
        let p = &self.rich;
        let doc = rich_label_document(
            &n.metadata.original,
            &style,
            p.fallback_fonts.as_ref(),
            p.theme_id,
            p.theme_overrides.clone(),
            &p.primary,
            &p.mono,
        )?;
        fn walk(node: Option<&SceneNode>, s: &Shape, out: &mut Vec<(SceneNode, isize)>) {
            let Some(node) = node else {
                return;
            };
            if let Some(Primitive::TextRun(run)) = &node.primitive {
                if !run.text.is_empty() {
                    out.push((node.clone(), text_row(&node.id, s)));
                }
            }
            for c in node.children.iter().flatten() {
                walk(Some(c), s, out);
            }
        }
        let mut cells = vec![];
        walk(doc.root.as_deref(), &n.metadata.original, &mut cells);
        Ok((doc, cells))
    }
    pub fn structured_node(&mut self, n: &Node) {
        if self.err.is_some() || n.opacity <= 0.0 {
            return;
        }
        let mut n = n.clone();
        let first = self.triangles.len();
        let opacity = n.opacity;
        n.opacity = 1.0;
        self.structured_node_body(&n);
        if opacity < 1.0 {
            let group = Rc::new(RefCell::new(OpacityGroup { opacity }));
            for t in &mut self.geometry.triangles[first..] {
                t.opacity_group = Some(group.clone());
            }
        }
    }
    fn structured_node_body(&mut self, n: &Node) {
        let (mut doc, cells) = match self.structured_document(n) {
            Ok(v) => v,
            Err(e) => {
                self.fail(format!("native structured shape {:?}: {e}", n.id));
                return;
            }
        };
        let s = &n.metadata.original;
        let rows = structured_rows(s);
        let face_budget = self.face_max_pixels;
        if face_budget > 0 {
            let copies = if s.multiple { 2 } else { 1 };
            self.face_max_pixels = 1.max(face_budget / (copies * (rows.len() + 1)) as i64);
        }
        let (sx, sz) = (n.size.x / s.width as f64, n.size.z / s.height as f64);
        let floor = n.position.y - n.size.y / 2.0;
        let height = native_canonical_height(n, self.scale);
        let (base, header, body) = (floor + 0.01, floor + height, floor + height * 0.32 / 0.46);
        for ci in (0..=1).rev() {
            if ci == 1 && !s.multiple {
                continue;
            }
            let mut copy = n.clone();
            copy.position.x = go_fmadd(ci as f64 * MULTIPLE_OFFSET as f64, sx, copy.position.x);
            copy.position.z = go_fmsub(ci as f64 * MULTIPLE_OFFSET as f64, sz, copy.position.z);
            let dy = -(ci as f64) * go_min(0.045, height * 0.25);
            self.structured_rail(
                &copy,
                0.0,
                s.height as f64,
                floor + dy + self.node_support_drop,
                base + dy,
                &n.fill,
            );
            for (i, r) in rows.iter().enumerate() {
                let (top, fill) = if i == 0 {
                    (header, &n.fill)
                } else {
                    (body, &n.stroke)
                };
                self.structured_rail(
                    &copy,
                    r.z + r.back,
                    r.depth - r.back - r.front,
                    base + dy,
                    top + dy,
                    fill,
                );
            }
        }
        // Go's part := *doc shares its asset map across every cell. Keep one
        // map here, lending it to the synchronous foundation raster call.
        let assets = Rc::new(RefCell::new(std::mem::take(&mut doc.assets)));
        for (cell, row) in &cells {
            if *row < 0 || *row as usize >= rows.len() {
                self.fail(format!(
                    "native structured shape {:?} has an unassigned text cell {:?}",
                    n.id, cell.id
                ));
                break;
            }
            self.structured_cell(
                &doc,
                &assets,
                cell,
                &rows[*row as usize],
                n,
                if *row == 0 { header } else { body },
                cells.len(),
            );
        }
        if s.icon.is_some() {
            let face = native_structured_icon_surface(n, header + 0.003);
            match self
                .icons
                .texture(s.icon.as_ref().unwrap(), face.width, face.depth, 0.0)
            {
                Ok(tex) => self.surface_texture(tex.as_ref(), &face, n.opacity),
                Err(e) => self.fail(format!("isometric icon for {:?}: {e}", n.id)),
            }
        }
        self.face_max_pixels = face_budget;
    }
    fn structured_rail(&mut self, n: &Node, z: f64, depth: f64, bottom: f64, top: f64, fill: &str) {
        if depth <= 0.0 || top <= bottom || self.err.is_some() {
            return;
        }
        let s = &n.metadata.original;
        let (sx, sz) = (n.size.x / s.width as f64, n.size.z / s.height as f64);
        let mut face = native_face_source(n, fill);
        face.type_ = SHAPE_RECTANGLE.into();
        face.text = Text::default();
        face.fill = fill.into();
        face.stroke = n.fill.clone();
        face.width = s.width;
        face.height = 1.max(depth.round() as i64);
        face.label_position.clear();
        face.border_radius = s.border_radius.min(face.height / 2);
        let profiles = match native_shape_profiles(&face) {
            Ok(p) => p,
            Err(e) => {
                self.fail(e);
                return;
            }
        };
        let first = self.triangles.len();
        let mat = native_material(fill, 0.72, 0.0, n.opacity);
        let decorated = n.opacity < 1.0
            || native_paint(&n.fill, "#263c4e").a < 255
            || n.stroke_dash != 0.0
            || s.double_border
            || !s.fill_pattern.is_empty();
        for profile in profiles {
            let world: Vec<_> = profile
                .iter()
                .map(|p| {
                    nv(
                        n.position.x - n.size.x / 2.0 + p.x * sx,
                        top,
                        n.position.z - n.size.z / 2.0 + (z + p.z * depth / face.height as f64) * sz,
                    )
                })
                .collect();
            self.extruded_profile(&world, bottom, (!decorated).then_some(&mat), &mat);
        }
        if decorated {
            let (tex, ink, mut area) = self.native_face_layers(&face, Some(n.opacity));
            area.center = nv(
                n.position.x - n.size.x / 2.0 + area.center.x * sx,
                top + 0.0005,
                n.position.z - n.size.z / 2.0
                    + (z + area.center.z * depth / face.height as f64) * sz,
            );
            area.width *= sx;
            area.depth = area.depth * depth / face.height as f64 * sz;
            let start = self.triangles.len();
            self.surface_texture(tex.as_ref(), &area, n.opacity);
            for t in &mut self.geometry.triangles[start..] {
                if let Some(m) = &t.material {
                    m.borrow_mut().unlit = false;
                }
                t.no_depth_write = false;
                t.cast_shadow = true;
            }
            self.surface_texture(ink.as_ref(), &area, n.opacity);
        }
        let mut ink = n.clone();
        ink.type_ = SHAPE_RECTANGLE.into();
        ink.fill = fill.into();
        ink.stroke = n.fill.clone();
        ink.stroke_explicit = true;
        ink.metadata.original = face;
        self.classic_ink_edges(&ink, first);
    }
    fn structured_cell(
        &mut self,
        doc: &Document,
        assets: &super::vector_surface::SharedSurfaceAssets,
        cell: &SceneNode,
        row: &StructuredRow,
        n: &Node,
        top: f64,
        count: usize,
    ) {
        if self.err.is_some() {
            return;
        }
        let s = &n.metadata.original;
        let (sx, sz) = (n.size.x / s.width as f64, n.size.z / s.height as f64);
        let bx = SceneBox {
            y: row.z,
            width: s.width as f64,
            height: row.depth,
            ..Default::default()
        };
        if bx.height <= 0.0 {
            return;
        }
        let budget = (4 * 1024 * 1024).min(MAX_RICH_PIXELS / (self.rich.count as usize).max(1))
            / (count.max(1));
        let budget = budget.max(1);
        let (w, h) = surface_texture_dimensions_at_density(
            n.size.x,
            row.depth * sz,
            4096,
            budget as i64,
            self.output_density,
        );
        if w * h > MAX_RICH_PIXELS as i64 - self.rich.pixels {
            self.fail("isometric rich labels exceed texture pixel budget");
            return;
        }
        self.rich.pixels += w * h;
        let mut part = Document {
            root: Some(Box::new(cell.clone())),
            view_box: bx,
            logical_width: w as f64,
            logical_height: h as f64,
            viewport_fit: ViewportFit::STRETCH,
            viewport_align: doc.viewport_align,
            links: doc.links.clone(),
            assets: std::mem::take(&mut *assets.borrow_mut()),
        };
        let frame = super::backends::render(&part, &super::backends::rich_raster_options());
        *assets.borrow_mut() = std::mem::take(&mut part.assets);
        let tex = match frame {
            Ok(t) => Rc::new(t),
            Err(e) => {
                self.fail(e);
                return;
            }
        };
        if let Err(e) = super::vector_surface::retain_shared_native_vector_surface(
            &self.vectors,
            &tex,
            part,
            assets,
        ) {
            self.fail(e);
            return;
        }
        let area = LabelSurface {
            center: nv(
                n.position.x,
                top + 0.003,
                go_fmadd(
                    go_fmadd(row.depth, 0.5, row.z),
                    sz,
                    go_fmsub(n.size.z, 0.5, n.position.z),
                ),
            ),
            width: bx.width * sx,
            depth: bx.height * sz,
            ..Default::default()
        };
        self.surface_texture(Some(&tex), &area, n.opacity);
    }
}
pub fn native_structured_icon_surface(n: &Node, y: f64) -> LabelSurface {
    let s = &n.metadata.original;
    let face = LabelSurface {
        center: nv(n.position.x, y, n.position.z),
        width: n.size.x,
        depth: n.size.z,
        ..Default::default()
    };
    let pos = Position::from_string(&s.icon_position);
    if !pos.is_outside() {
        return super::surface_icons::surface_icon_layout(
            &face,
            s,
            n.size.x / s.width as f64,
            "node",
        )
        .0;
    }
    let bx = geo::Box::new(geo::Point::new(0.0, 0.0), s.width as f64, s.height as f64);
    let size = super::backends::icon_size(&bx, &s.icon_position) as f64;
    let p = pos.get_point_on_box(&bx, label::PADDING, size, size);
    let (sx, sz) = (n.size.x / s.width as f64, n.size.z / s.height as f64);
    LabelSurface {
        center: nv(
            go_fmadd(
                go_fmadd(size, 0.5, p.x),
                sx,
                go_fmsub(n.size.x, 0.5, n.position.x),
            ),
            y,
            go_fmadd(
                go_fmadd(size, 0.5, p.y),
                sz,
                go_fmsub(n.size.z, 0.5, n.position.z),
            ),
        ),
        width: size * sx,
        depth: size * sz,
        ..Default::default()
    }
}

#[cfg(test)]
#[path = "structured_api_tests.rs"]
mod structured_api_tests;
