// SPDX-License-Identifier: MPL-2.0
//! source_panels.go: source legends, root captions and enclosing decoration.
use super::{
    framing::ProjectedExtent,
    geometry::{LabelSurface, MeshBuilder, native_paint, native_token, readable_surface_ink},
    mesh::{Texture, Triangle, nv},
    svg::NativeScene,
};
use crate::{
    scene::Scene,
    target::{Config, Diagram, Point, SHAPE_RECTANGLE, Shape},
};
use d2_rs_render::go_float::go_fmadd;
use d2_rs_scene::{Box as SceneBox, Document, Nrgba, Primitive, go_max, go_min};
use std::rc::Rc;
#[derive(Clone, Default)]
pub struct NativePanel {
    pub document: Document,
    pub width: i64,
    pub height: i64,
    pub first: usize,
    pub last: usize,
    pub animated: bool,
}
fn source_target(s: &Scene) -> Diagram {
    Diagram {
        root: s.root.clone(),
        legend: s.legend.clone(),
        font_family: s.font_family.clone(),
        mono_font_family: s.mono_font_family.clone(),
        config: Some(Config {
            theme_id: Some(s.theme_id),
            theme_overrides: s.theme_overrides.clone(),
            ..Default::default()
        }),
        shapes: Some(
            s.nodes
                .iter()
                .map(|n| n.metadata.original.clone())
                .collect(),
        ),
        connections: Some(
            s.edges
                .iter()
                .map(|e| e.metadata.original.clone())
                .collect(),
        ),
        ..Default::default()
    }
}
fn mesh_bounds(ts: &[Triangle]) -> SceneBox {
    let (mut x0, mut z0, mut x1, mut z1) = (
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    );
    for t in ts {
        for v in t.v {
            x0 = go_min(x0, v.position.x);
            z0 = go_min(z0, v.position.z);
            x1 = go_max(x1, v.position.x);
            z1 = go_max(z1, v.position.z);
        }
    }
    if ts.is_empty() {
        SceneBox {
            width: 1.0,
            height: 1.0,
            ..Default::default()
        }
    } else {
        SceneBox {
            x: x0,
            y: z0,
            width: go_max(0.01, x1 - x0),
            height: go_max(0.01, z1 - z0),
        }
    }
}
fn bounds_target(s: &Scene, b: SceneBox, scale: f64) -> Diagram {
    let mut d = source_target(s);
    d.shapes = Some(vec![Shape {
        id: "native-bounds".into(),
        type_: SHAPE_RECTANGLE.into(),
        fill: "transparent".into(),
        stroke: "transparent".into(),
        opacity: 0.0,
        stroke_width: 2,
        text: crate::target::Text {
            bold: true,
            font_family: "DEFAULT".into(),
            ..Default::default()
        },
        pos: Point {
            x: (b.x / scale).floor() as i64,
            y: (b.y / scale).floor() as i64,
        },
        width: 1.max((b.width / scale).ceil() as i64),
        height: 1.max((b.height / scale).ceil() as i64),
        ..Default::default()
    }]);
    d.connections = None;
    d.legend = None;
    d
}
fn document_surface(doc: &Document, scale: f64, y: f64) -> LabelSurface {
    let v = doc.view_box;
    LabelSurface {
        center: nv(
            (v.x + v.width / 2.0) * scale,
            y,
            (v.y + v.height / 2.0) * scale,
        ),
        width: v.width * scale,
        depth: v.height * scale,
        ..Default::default()
    }
}
pub fn surface_document(
    d: &Diagram,
    fonts: Option<&d2_rs_scenebuild::FontFallbackOptions>,
) -> Result<Document, String> {
    surface_document_with_assets(d, fonts, None)
}

fn surface_document_with_assets(
    d: &Diagram,
    fonts: Option<&d2_rs_scenebuild::FontFallbackOptions>,
    assets: Option<&d2_rs_scenebuild::AssetOptions>,
) -> Result<Document, String> {
    // surface_fonts.go:19–54. Only an explicitly supplied resolver authorizes
    // external icons; the default resolver below is for embedded data only.
    if assets.is_none_or(|a| a.resolver_impl.is_none()) {
        let icons = std::iter::once(d.root.icon.as_ref())
            .chain(d.shapes.iter().flatten().map(|s| s.icon.as_ref()))
            .chain(d.connections.iter().flatten().map(|e| e.icon.as_ref()))
            .chain(
                d.legend
                    .iter()
                    .flat_map(|l| l.shapes.iter().flatten())
                    .map(|s| s.icon.as_ref()),
            )
            .chain(
                d.legend
                    .iter()
                    .flat_map(|l| l.connections.iter().flatten())
                    .map(|e| e.icon.as_ref()),
            );
        if icons
            .flatten()
            .any(|icon| !icon.scheme.eq_ignore_ascii_case("data"))
        {
            return Err("native surface external icon requires an explicit asset resolver".into());
        }
    }
    let cfg = d.config.as_ref();
    let options = d2_rs_scenebuild::Options {
        pad: Some(0),
        max_nodes: 200000,
        max_path_commands: 2000000,
        theme_id: cfg.and_then(|c| c.theme_id),
        theme_overrides: cfg.and_then(|c| c.theme_overrides.clone()),
        assets: Some(super::surface_icons::SurfaceIconPainter::surface_assets(
            assets,
        )?),
        fonts: Some(super::surface_document::surface_fonts(fonts)?),
        link_budget: d2_rs_scenebuild::LinkBudget {
            max_regions: 20000,
            max_string_bytes: 8 * 1024 * 1024,
        },
        ..Default::default()
    };
    let doc = d2_rs_scenebuild::build(d, &options).map_err(|e| e.to_string())?;
    super::surface_document::document_glyph_coverage(&doc)?;
    Ok(doc)
}
impl MeshBuilder {
    fn source_panel(
        &mut self,
        doc: Document,
        surface: LabelSurface,
        planned: usize,
    ) -> Result<(), String> {
        if !(1..=2).contains(&planned) {
            return Err(
                "isometric source panel count must be at most a legend and root frame".into(),
            );
        }
        if !doc.view_box.width.is_finite()
            || !doc.view_box.height.is_finite()
            || doc.view_box.width <= 0.0
            || doc.view_box.height <= 0.0
        {
            return Err("isometric source panel has invalid dimensions".into());
        }
        let ratio = go_min(
            2.0,
            2048.0 / go_max(doc.view_box.width, doc.view_box.height),
        );
        let (mut w, mut h) = (
            1.max((doc.view_box.width * ratio).ceil() as i64),
            1.max((doc.view_box.height * ratio).ceil() as i64),
        );
        if self.output_density > 0.0 {
            (w, h) = super::surface_document::surface_texture_dimensions_at_density(
                surface.width,
                surface.depth,
                4096,
                (8 << 20) / planned as i64,
                self.output_density,
            );
        }
        if w * h + self.panels.iter().map(|p| p.width * p.height).sum::<i64>() > 8 << 20 {
            return Err("isometric source panels exceed 8M texture pixels".into());
        }
        let tex = Rc::new(Texture::new(
            w as usize,
            h as usize,
            super::backends::raster_surface(&doc, w as usize, h as usize)?,
        ));
        let mut view = doc.clone();
        view.logical_width = w as f64;
        view.logical_height = h as f64;
        view.viewport_fit = d2_rs_scene::ViewportFit::MEET;
        view.viewport_align = d2_rs_scene::ViewportAlign::X_MID_Y_MID;
        self.retain_vector_surface(&tex, &view)?;
        let first = self.triangles.len();
        self.surface_texture(Some(&tex), &surface, 1.0);
        let mut linkdoc = doc.clone();
        linkdoc.logical_width = w as f64;
        linkdoc.logical_height = h as f64;
        linkdoc.viewport_fit = d2_rs_scene::ViewportFit::MEET;
        linkdoc.viewport_align = d2_rs_scene::ViewportAlign::X_MID_Y_MID;
        self.add_document_links(Some(&linkdoc), &surface);
        for t in &mut self.geometry.triangles[first..] {
            t.no_depth_write = false;
        }
        let last = self.triangles.len();
        let animated = scene_has_animation(doc.root.as_deref());
        self.panels.push(NativePanel {
            document: doc,
            width: w,
            height: h,
            first,
            last,
            animated,
        });
        self.err.clone().map_or(Ok(()), Err)
    }
    pub fn root_decorations(
        &mut self,
        s: &Scene,
        assets: Option<&d2_rs_scenebuild::AssetOptions>,
    ) -> Result<(), String> {
        let panels = s.legend.is_some() as usize
            + (s.root.stroke_width > 0 || !s.root.fill_pattern.is_empty() || s.root.double_border)
                as usize;
        if s.legend.is_some() {
            let mut d = bounds_target(s, mesh_bounds(&self.triangles), self.scale);
            d.legend = s.legend.clone();
            let mut doc = surface_document_with_assets(&d, self.fonts.as_ref(), assets)
                .map_err(|e| format!("isometric legend: {e}"))?;
            let legend = doc
                .root
                .as_ref()
                .and_then(|r| r.children.as_ref())
                .and_then(|c| c.iter().find(|n| n.id == "legend"))
                .cloned();
            if let Some(legend) = legend {
                let panel = legend
                    .children
                    .iter()
                    .flatten()
                    .find(|c| c.id == "legend:panel");
                if let Some(panel) = panel {
                    let Some(Primitive::Rect(r)) = &panel.primitive else {
                        return Err("isometric legend panel is not a rectangle".into());
                    };
                    doc.view_box = SceneBox {
                        x: r.r#box.x - 12.0,
                        y: r.r#box.y - 12.0,
                        width: r.r#box.width + 24.0,
                        height: r.r#box.height + 24.0,
                    };
                    doc.logical_width = doc.view_box.width;
                    doc.logical_height = doc.view_box.height;
                    doc.root = Some(Box::new(legend));
                    let surface = document_surface(&doc, self.scale, 0.065);
                    self.source_panel(doc, surface, panels)?;
                }
            }
        }
        let mut root = s.root.clone();
        if !root.text.label.is_empty() || root.icon.is_some() || !s.description.is_empty() {
            let bounds = mesh_bounds(&self.triangles);
            let mut theme = crate::theme::Theme::find(s.theme_id)
                .unwrap_or_else(|| crate::theme::Theme::find(0).unwrap());
            theme.apply_overrides(s.theme_overrides.as_ref());
            root.text.color = theme.resolve(&root.text.color);
            root.text.label_fill = theme.resolve(&root.text.label_fill);
            if root.text.font_size <= 0 {
                root.text.font_size = 28;
            }
            let font = root.text.font_size as f64 * self.scale;
            let mut w = go_max(bounds.width, root.text.label_width as f64 * self.scale);
            if root.icon.is_some() {
                w = go_max(w, font * 4.0);
            }
            let mut h = go_max(font * 1.5, root.text.label_height as f64 * self.scale);
            if root.text.label_height <= 0 {
                let lines: usize = root
                    .text
                    .label
                    .split('\n')
                    .map(|l| 1.max((l.chars().count() as f64 * font * 0.56 / w).ceil() as usize))
                    .sum();
                h = go_max(h, lines as f64 * font * 1.4);
            }
            let mut bottom = bounds.y - 0.35;
            let mut ink = readable_surface_ink(&s.background, 1.0);
            if !native_token(&root.text.color) {
                ink = root.text.color.clone();
            }
            if !s.description.is_empty() {
                let mut style = root.text.clone();
                style.label = s.description.clone();
                style.language.clear();
                style.font_size = 16;
                style.bold = false;
                style.label_width = 0;
                style.label_height = 0;
                let dh = go_max(
                    0.25,
                    (style.label.chars().count() as f64 * 0.085 / w).ceil() * 0.25,
                );
                self.label(
                    &style.label,
                    &LabelSurface {
                        center: nv(bounds.x + w / 2.0, 0.066, bottom - dh / 2.0),
                        width: w,
                        depth: dh,
                        align: "left".into(),
                        ..Default::default()
                    },
                    &style,
                    &ink,
                    1.0,
                    "board",
                );
                bottom -= dh + 0.18;
            }
            if !root.text.label.is_empty() || root.icon.is_some() {
                let surface = LabelSurface {
                    center: nv(bounds.x + w / 2.0, 0.066, bottom - h / 2.0),
                    width: w,
                    depth: h,
                    align: "left".into(),
                    ..Default::default()
                };
                let surface = self.shape_icon(&root, surface, 1.0, "board", None);
                self.shape_label(&root, &surface, &ink, 1.0, "board");
            }
        }
        if root.stroke_width > 0 || !root.fill_pattern.is_empty() || root.double_border {
            let mut bounds = mesh_bounds(&self.triangles);
            bounds.x -= 0.25;
            bounds.y -= 0.25;
            bounds.width += 0.5;
            bounds.height += 0.5;
            let d = bounds_target(s, bounds, self.scale);
            let mut doc = surface_document_with_assets(&d, self.fonts.as_ref(), assets)
                .map_err(|e| format!("isometric root frame: {e}"))?;
            if let Some(r) = doc.root.as_mut() {
                if let Some(c) = r.children.as_mut() {
                    c.retain(|n| n.id.starts_with("root:"));
                }
            }
            let surface = document_surface(&doc, self.scale, -0.045);
            self.source_panel(doc, surface, panels)?;
        }
        self.err.clone().map_or(Ok(()), Err)
    }
    pub fn finish_scene(
        mut self,
        s: &Scene,
        width: i64,
        height: i64,
        packets: &[super::geometry_routes::PacketRoute],
    ) -> Result<NativeScene, String> {
        self.prepare_gradient_materials();
        if let Some(e) = self.err.clone() {
            return Err(e);
        }
        let mut background = Nrgba::new(245, 247, 251, 255);
        if !native_token(&s.root.fill) && !d2_rs_scenebuild::is_gradient(&s.background) {
            let c = native_paint(&s.background, "#f5f7fb");
            let a = c.a as f64 / 255.0;
            background = Nrgba::new(
                go_fmadd(c.r as f64, a, background.r as f64 * (1.0 - a)) as u8,
                go_fmadd(c.g as f64, a, background.g as f64 * (1.0 - a)) as u8,
                go_fmadd(c.b as f64, a, background.b as f64 * (1.0 - a)) as u8,
                255,
            );
        }
        let mut extents = ProjectedExtent::default();
        extents.animated_mesh(&self.triangles, &self.animated_nodes, self.scale);
        extents.traffic_routes(packets);
        let camera = self
            .options
            .camera
            .unwrap_or_else(|| extents.camera(width, height, self.options.fit_content));
        let background_paint =
            if !native_token(&s.root.fill) && d2_rs_scenebuild::is_gradient(&s.background) {
                let paint = d2_rs_scenebuild::gradient_paint(&s.background)?;
                self.gradient_texture(&s.background);
                Some(Rc::new(super::vector_surface::VectorSurface {
                    document: Rc::new(std::cell::RefCell::new(
                        super::gradient_paints::native_paint_document(paint),
                    )),
                    ..Default::default()
                }))
            } else {
                None
            };
        if let Some(e) = self.err.clone() {
            return Err(e);
        }
        Ok(NativeScene {
            triangles: self.geometry.triangles,
            camera,
            width: camera.width,
            height: camera.height,
            background,
            background_paint,
            links: self.links,
            packets: packets.to_vec(),
            panels: self.panels,
            animated_nodes: self.animated_nodes,
            pixel_scale: self.scale,
        })
    }
}

fn scene_has_animation(node: Option<&d2_rs_scene::Node>) -> bool {
    let Some(n) = node else {
        return false;
    };
    n.animations.as_ref().is_some_and(|a| !a.is_empty())
        || n.children
            .iter()
            .flatten()
            .any(|c| scene_has_animation(Some(c)))
        || n.mask
            .as_ref()
            .is_some_and(|m| scene_has_animation(m.root.as_deref()))
}

#[cfg(test)]
#[path = "panel_api_tests.rs"]
mod panel_api_tests;
