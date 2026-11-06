// SPDX-License-Identifier: MPL-2.0
//! sequence_routes.go: source-aligned sequence wires, captions and marker planes.
use super::{
    geometry::{LabelSurface, MeshBuilder, native_material, native_paint, native_token},
    geometry_routes::*,
    mesh::*,
    route_caption_style::{connection_label_top_left, native_caption_on_edge},
    route_labels::{RouteCaptionPlacer, caption_finite, caption_rect},
    routes::caption_theme,
};
use crate::{
    scene::{Edge, Scene},
    target::{Connection, Text},
};
use d2_rs_render::go_float::go_fmadd;
use d2_rs_scene::go_min;
use d2_rs_scenebuild::golib::{
    geo::{self, Point},
    label::{PADDING, Position},
    svgpath::get_stroke_dash_attributes,
};
pub fn native_sequence_edge(e: &Edge) -> bool {
    e.sequence_role == "lifeline" || e.sequence_role == "message"
}
pub fn native_sequence_edge_y(e: &Edge) -> f64 {
    if e.sequence_role == "lifeline" {
        0.10
    } else {
        0.30
    }
}
struct Caption {
    style: Text,
    surface: LabelSurface,
    main: bool,
}
fn arrow_label(c: &Connection, dst: bool) -> Option<Point> {
    let style = if dst {
        c.dst_label.as_ref()?
    } else {
        c.src_label.as_ref()?
    };
    let route: Option<std::vec::Vec<_>> = c
        .route()
        .iter()
        .map(|p| p.map(|p| Point::new(p.x, p.y)))
        .collect();
    let route = route?;
    if route.len() < 2 {
        return None;
    }
    let index = if dst { route.len() - 2 } else { 0 };
    let (a, b) = (route[index], route[index + 1]);
    let (nx, ny) = geo::get_unit_normal_vector(b.x, b.y, a.x, a.y);
    let shift = go_fmadd(
        ny.abs(),
        style.label_width as f64 / 2.0 + PADDING,
        nx.abs() * (style.label_height as f64 / 2.0 + PADDING),
    );
    let length = geo::route_length(&route);
    let fraction = if dst {
        if length > 0.0 {
            1.0 - shift / length
        } else {
            1.0
        }
    } else if length > 0.0 {
        shift / length
    } else {
        0.0
    };
    let (mut p, _) = Position::UnlockedTop.get_point_on_route(
        &route,
        c.stroke_width as f64,
        fraction,
        style.label_width as f64,
        style.label_height as f64,
    );
    let size = if dst && c.dst_arrow != "none" {
        arrow_dimensions(&c.dst_arrow, c.stroke_width as f64).1
    } else if c.src_arrow != "none" {
        arrow_dimensions(&c.src_arrow, c.stroke_width as f64).1
    } else {
        0.0
    };
    if size > 0.0 {
        let offset = (size / 2.0 + 2.0) - c.stroke_width as f64 / 2.0 - PADDING;
        if offset > 0.0 {
            p.x = go_fmadd(nx, offset, p.x);
            p.y = go_fmadd(ny, offset, p.y)
        }
    }
    Some(p)
}
fn caption_data(e: &Edge, scale: f64) -> (std::vec::Vec<Caption>, LabelSurface) {
    let empty = || (vec![], LabelSurface::default());
    let points = e.points.as_deref().unwrap_or_default();
    if scale <= 0.0 || !caption_finite(&[scale]) || points.len() < 2 {
        return empty();
    }
    let mut c = e.metadata.original.clone();
    let (mut ox, mut oz) = (0.0, 0.0);
    if c.route().len() >= 2 && c.route()[0].is_some() {
        let p = c.route()[0].unwrap();
        ox = go_fmadd(-p.x, scale, points[0].x);
        oz = go_fmadd(-p.y, scale, points[0].z)
    } else {
        c.route = Some(
            points
                .iter()
                .map(|p| {
                    Some(crate::target::GeoPoint {
                        x: p.x / scale,
                        y: p.z / scale,
                    })
                })
                .collect(),
        )
    }
    for p in c.route() {
        if p.is_none_or(|p| !caption_finite(&[p.x, p.y])) {
            return empty();
        }
    }
    if Position::from_string(&c.label_position) == Position::Unset {
        c.label_position = Position::InsideMiddleCenter.string().into()
    }
    if Position::from_string(&c.icon_position) == Position::Unset {
        c.icon_position = Position::InsideMiddleCenter.string().into()
    }
    let surface = |p: Option<Point>, w: i64, h: i64| {
        let Some(p) = p.filter(|p| caption_finite(&[p.x, p.y])) else {
            return LabelSurface::default();
        };
        LabelSurface {
            center: nv(
                go_fmadd(p.x + w as f64 / 2.0, scale, ox),
                native_sequence_edge_y(e) + 0.006,
                go_fmadd(p.y + h as f64 / 2.0, scale, oz),
            ),
            width: w.max(1) as f64 * scale,
            depth: h.max(1) as f64 * scale,
            ..Default::default()
        }
    };
    let mut captions = vec![];
    if !e.label.is_empty() {
        let mut style = c.text.clone();
        style.label = e.label.clone();
        let s = surface(
            connection_label_top_left(&c),
            style.label_width,
            style.label_height,
        );
        captions.push(Caption {
            style,
            surface: s,
            main: true,
        })
    }
    for (text, dst) in [(&e.source_label, false), (&e.target_label, true)] {
        let Some(text) = text.as_ref().filter(|t| !t.label.is_empty()) else {
            continue;
        };
        if dst {
            c.dst_label = Some(text.clone())
        } else {
            c.src_label = Some(text.clone())
        }
        let mut style = text.clone();
        style.font_size = c.text.font_size;
        style.font_family = "default".into();
        style.bold = false;
        style.italic = true;
        style.underline = false;
        let s = surface(arrow_label(&c, dst), style.label_width, style.label_height);
        captions.push(Caption {
            style,
            surface: s,
            main: false,
        })
    }
    let mut icon = LabelSurface::default();
    if c.icon.is_some() {
        let p = if !c.text.label.is_empty() {
            connection_label_top_left(&c).map(|p| {
                Point::new(
                    p.x - 8.0 - 32.0,
                    p.y + c.text.label_height as f64 / 2.0 - 16.0,
                )
            })
        } else {
            let route: std::vec::Vec<_> = c
                .route()
                .iter()
                .flatten()
                .map(|p| Point::new(p.x, p.y))
                .collect();
            Position::from_string(&c.icon_position)
                .try_get_point_on_route(&route, c.stroke_width as f64, -1.0, 32.0, 32.0)
                .map(|p| p.0)
        };
        icon = surface(p, 32, 32)
    }
    (captions, icon)
}
impl MeshBuilder {
    pub fn reserve_sequence_edge(&mut self, e: &Edge, points: &[Vec], p: &mut RouteCaptionPlacer) {
        if e.stroke_width > 0 {
            p.avoid_route(points, e.stroke_width as f64 * self.scale / 2.0)
        }
        let (captions, icon) = caption_data(e, self.scale);
        for c in captions {
            p.reserve(caption_rect(&c.surface, true))
        }
        if icon.width > 0.0 && icon.depth > 0.0 {
            p.reserve(caption_rect(&icon, true))
        }
    }
    pub fn sequence_edge(
        &mut self,
        e: &Edge,
        points: &[Vec],
        s: &Scene,
    ) -> std::vec::Vec<PacketRoute> {
        if points.len() < 2 || e.opacity <= 0.0 {
            return vec![];
        }
        let tint = if e.stroke_explicit {
            e.stroke.as_str()
        } else {
            "#263c4e"
        };
        let mat = native_material(tint, 1.0, 0.0, e.opacity);
        mat.borrow_mut().unlit = true;
        let radius = e.stroke_width as f64 * self.scale / 2.0;
        let lengths = route_lengths(points);
        let total = lengths[lengths.len() - 1];
        let first = self.triangles.len();
        if radius > 0.0 && total > 0.0 {
            let (start, end) = (
                native_arrow_clearance(&e.source_arrow, e.stroke_width) / total,
                1.0 - native_arrow_clearance(&e.target_arrow, e.stroke_width) / total,
            );
            let wire = if start > 0.0 || end < 1.0 {
                native_route_section(points, &lengths, start, end)
            } else {
                points.to_vec()
            };
            if e.stroke_dash <= 0.0 {
                self.route_ink(&wire, radius, &mat)
            } else {
                let wl = route_lengths(&wire);
                if wl.len() > 1 {
                    let length = wl[wl.len() - 1];
                    let (mut dash, mut gap) =
                        get_stroke_dash_attributes(e.stroke_width as f64, e.stroke_dash);
                    dash *= self.scale;
                    gap *= self.scale;
                    if !caption_finite(&[dash, gap]) || dash <= 0.0 || gap <= 0.0 {
                        dash = e.stroke_width as f64 * e.stroke_dash * self.scale;
                        gap = dash
                    }
                    if !caption_finite(&[dash, gap])
                        || dash <= 0.0
                        || gap <= 0.0
                        || length / (dash + gap) > 50000.0
                    {
                        self.fail("isometric sequence dashes exceed geometry budget");
                        return vec![];
                    }
                    let mut at = 0.0;
                    while at < length {
                        self.route_ink(
                            &native_route_section(
                                &wire,
                                &wl,
                                at / length,
                                go_min(1.0, (at + dash) / length),
                            ),
                            radius,
                            &mat,
                        );
                        if self.err.is_some() {
                            return vec![];
                        }
                        at += dash + gap
                    }
                }
            }
        }
        let (captions, icon) = caption_data(e, self.scale);
        if native_caption_on_edge(&e.metadata.original) {
            if let Some(c) = captions.iter().find(|c| c.main) {
                self.route_caption_knockout(first, &c.surface)
            }
        }
        let marker = self.triangles.len();
        self.arrow_with_opacity(
            &e.source_arrow,
            points[0],
            nsub(points[0], points[1]),
            &mat,
            e.stroke_width,
            e.opacity,
        );
        let last = points.len() - 1;
        self.arrow_with_opacity(
            &e.target_arrow,
            points[last],
            nsub(points[last], points[last - 1]),
            &mat,
            e.stroke_width,
            e.opacity,
        );
        for t in &mut self.geometry.triangles[marker..] {
            for v in &mut t.v {
                v.position.y = native_sequence_edge_y(e);
                v.normal = nv(0.0, 1.0, 0.0)
            }
        }
        self.add_mesh_link(&e.link, &e.tooltip, first);
        let theme = caption_theme(s);
        for mut c in captions {
            let mut ink = theme.resolve(&c.style.color);
            if native_token(&c.style.color) {
                ink = "#263c4e".into()
            }
            if c.main && e.font_explicit {
                ink = e.font_color.clone()
            }
            if c.main && !e.metadata.original.fill.is_empty() {
                c.style.label_fill = e.metadata.original.fill.clone()
            }
            c.style.label_fill = theme.resolve(&c.style.label_fill);
            if c.main {
                self.add_surface_link(&e.link, &e.tooltip, &c.surface)
            }
            self.label(
                &c.style.label,
                &c.surface,
                &c.style,
                &ink,
                e.opacity,
                "edge",
            );
        }
        if icon.width > 0.0 && icon.depth > 0.0 {
            self.shape_icon(
                &crate::target::Shape {
                    id: e.id.clone(),
                    icon: e.metadata.original.icon.clone(),
                    ..Default::default()
                },
                icon.clone(),
                e.opacity,
                "edge",
                Some(e.metadata.original.icon_border_radius),
            );
            self.add_surface_link(&e.link, &e.tooltip, &icon)
        }
        if e.metadata.original.animated && e.stroke_width > 0 && mat.borrow().color.a > 0 {
            let packet = native_material("#e9f8ff", 0.2, 0.15, mat.borrow().color.a as f64 / 255.0);
            packet.borrow_mut().emissive = native_paint(tint, "#263c4e");
            let (mut forward, reverse) = (
                !e.target_arrow.is_empty() && e.target_arrow != "none",
                !e.source_arrow.is_empty() && e.source_arrow != "none",
            );
            if !forward && !reverse {
                forward = true
            }
            return vec![PacketRoute {
                points: points.to_vec(),
                lengths,
                material: Some(packet),
                forward,
                reverse,
            }];
        }
        vec![]
    }
}

#[cfg(test)]
pub(crate) fn caption_api_value(e: &Edge, scale: f64) -> serde_json::Value {
    use serde_json::json;
    fn surface(s: &LabelSurface) -> serde_json::Value {
        json!({"center":s.center,"width":s.width,"depth":s.depth,"angle":s.angle,"align":s.align})
    }
    let (c, i) = caption_data(e, scale);
    json!({"captions":c.iter().map(|c|json!({"style":c.style,"surface":surface(&c.surface),"main":c.main})).collect::<std::vec::Vec<_>>(),"icon":surface(&i)})
}
