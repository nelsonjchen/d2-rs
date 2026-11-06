// SPDX-License-Identifier: MPL-2.0
//! geometry_routes.go edges: route assembly, authored arrows and caption paint.
use super::{
    geometry::{MeshBuilder, native_material},
    geometry_routes::*,
    mesh::*,
    native_solids::native_solid_contact_routes_with_context,
    route_bridges::{BRIDGE_MAX_POINTS, native_resolve_bridge_routes},
    route_caption_style::*,
    route_labels::{RouteCaptionPlacer, caption_finite, caption_rect},
    route_lanes::native_resolve_lane_routes,
    route_style::*,
    sequence_routes::*,
};
use crate::scene::{Board, Edge, Node, Scene};
use d2_rs_render::go_float::go_fmadd;
use d2_rs_scene::go_min;
pub fn native_edge_routes(
    edges: &[Edge],
    nodes: &[Node],
    boards: &[Board],
    support: &std::collections::HashMap<String, f64>,
) -> Result<
    (
        std::vec::Vec<std::vec::Vec<Vec>>,
        std::vec::Vec<std::vec::Vec<Vec>>,
    ),
    String,
> {
    native_edge_routes_with_context(edges, nodes, boards, support, &Default::default())
}
pub fn native_edge_routes_with_context(
    edges: &[Edge],
    nodes: &[Node],
    boards: &[Board],
    support: &std::collections::HashMap<String, f64>,
    context: &super::context::RenderContext,
) -> Result<
    (
        std::vec::Vec<std::vec::Vec<Vec>>,
        std::vec::Vec<std::vec::Vec<Vec>>,
    ),
    String,
> {
    let mut lanes = vec![vec![]; edges.len()];
    let mut paths = lanes.clone();
    let (mut ordinary, mut indices) = (vec![], vec![]);
    let mut total = 0;
    for (i, e) in edges.iter().enumerate() {
        let points = e.points.as_deref().unwrap_or_default();
        if points.len() > 10000 || total > BRIDGE_MAX_POINTS - points.len() {
            return Err("isometric routes exceed point budget".into());
        }
        total += points.len();
        if !native_sequence_edge(e) {
            ordinary.push(e.clone());
            indices.push(i);
            continue;
        }
        let mut points = points.to_vec();
        for p in &mut points {
            if !caption_finite(&[p.x, p.y, p.z]) {
                return Err(format!(
                    "isometric sequence route {:?} has invalid coordinates",
                    e.id
                ));
            }
            p.y = native_sequence_edge_y(e)
        }
        lanes[i] = points.clone();
        paths[i] = points;
    }
    let ordinary_lanes =
        native_resolve_lane_routes(Some(context), &ordinary, nodes, boards, 25000000)?;
    let mut resolved = ordinary.clone();
    for (i, e) in resolved.iter_mut().enumerate() {
        e.points = Some(ordinary_lanes[i].clone())
    }
    let ordinary_paths = native_resolve_bridge_routes(Some(context), &resolved, 20000000)?;
    let ordinary_paths = native_solid_contact_routes_with_context(
        Some(context),
        &ordinary,
        nodes,
        &ordinary_paths,
        Some(support),
    )?;
    for (i, &index) in indices.iter().enumerate() {
        lanes[index] = ordinary_lanes[i].clone();
        paths[index] = ordinary_paths[i].clone()
    }
    Ok((lanes, paths))
}
pub fn caption_theme(s: &Scene) -> crate::theme::Theme {
    let mut t = crate::theme::Theme::find(s.theme_id)
        .unwrap_or_else(|| crate::theme::Theme::find(0).unwrap());
    t.apply_overrides(s.theme_overrides.as_ref());
    t
}
impl MeshBuilder {
    pub fn edges(
        &mut self,
        edges: &[Edge],
        placer: &mut RouteCaptionPlacer,
        s: &Scene,
    ) -> std::vec::Vec<PacketRoute> {
        let mut packets = vec![];
        let theme = caption_theme(s);
        let (lanes, paths) = match native_edge_routes_with_context(
            edges,
            &s.nodes,
            &s.boards,
            &self.hierarchy_supports,
            &self.geometry.context,
        ) {
            Ok(p) => p,
            Err(e) => {
                self.fail(e);
                return packets;
            }
        };
        for (i, e) in edges.iter().enumerate() {
            if e.opacity <= 0.0 || lanes[i].len() < 2 {
                continue;
            }
            if native_sequence_edge(e) {
                self.reserve_sequence_edge(e, &paths[i], placer);
                continue;
            }
            if e.stroke_width > 0 {
                placer.avoid_route(&paths[i], native_route_radius(e) * 1.6)
            }
            let icon = !e.icon.is_empty() && e.metadata.original.icon.is_some();
            if e.label.is_empty() && !icon {
                continue;
            }
            let (pw, pd) = native_route_caption_size(&e.metadata.original.text, icon, self.scale);
            if let Some(mut surface) = native_connection_caption_surface(
                &lanes[i],
                &e.metadata.original,
                pw,
                pd,
                self.scale,
            ) {
                if native_caption_on_edge(&e.metadata.original) {
                    surface.center.y += native_route_radius(e)
                }
                placer.reserve(caption_rect(&surface, true));
            }
        }
        for (i, e) in edges.iter().enumerate() {
            if self.err.is_some() {
                return packets;
            }
            if e.points.as_ref().map_or(0, |p| p.len()) < 2 || e.opacity <= 0.0 {
                continue;
            }
            if native_sequence_edge(e) {
                packets.extend(self.sequence_edge(e, &paths[i], s));
                continue;
            }
            let tint = native_route_tint(e);
            let mat = native_material(&tint, 0.6, 0.05, e.opacity);
            mat.borrow_mut().unlit = true;
            let wire_opacity = mat.borrow().color.a as f64 / 255.0;
            let points = &paths[i];
            if points.len() < 2 {
                continue;
            }
            let lengths = route_lengths(points);
            let length = lengths[lengths.len() - 1];
            let (start, end) = self.visible_arrow_range(e, points);
            let display = if start > 0.0 || end < 1.0 {
                native_route_section(points, &lengths, start, end)
            } else {
                points.clone()
            };
            if display.len() < 2 {
                self.fail("isometric endpoint visibility exhausted route");
                return packets;
            }
            let (mut wire_start, mut wire_end) = (start, end);
            if length > 1e-9 {
                wire_start += native_arrow_clearance(&e.source_arrow, e.stroke_width) / length;
                wire_end -= native_arrow_clearance(&e.target_arrow, e.stroke_width) / length
            }
            let wire = if wire_start > 0.0 || wire_end < 1.0 {
                native_route_section(points, &lengths, wire_start, wire_end)
            } else {
                display.clone()
            };
            let first = self.triangles.len();
            if e.stroke_width > 0 && wire.len() > 1 {
                let radius = native_route_radius(e);
                if e.stroke_dash <= 0.0 {
                    self.route_casing(&wire, radius, wire_opacity);
                    self.route_ink(&wire, radius, &mat)
                } else {
                    for seg in native_route_dashes(
                        points,
                        &lengths,
                        wire_start,
                        wire_end,
                        e.stroke_dash,
                        4.max(8000 / edges.len().max(1)),
                    ) {
                        self.route_casing(&seg, radius, wire_opacity);
                        self.route_ink(&seg, radius, &mat)
                    }
                }
            }
            let icon = !e.icon.is_empty() && e.metadata.original.icon.is_some();
            if (!e.label.is_empty() || icon) && native_caption_on_edge(&e.metadata.original) {
                let (pw, pd) =
                    native_route_caption_size(&e.metadata.original.text, icon, self.scale);
                if let Some(mut surface) = native_connection_caption_surface(
                    &lanes[i],
                    &e.metadata.original,
                    pw,
                    pd,
                    self.scale,
                ) {
                    surface.center.y += native_route_radius(e);
                    if !icon && e.metadata.original.text.label_width > 0 {
                        surface.width = go_min(
                            surface.width,
                            // ARM64 geometry_routes.go:436 contracts this width;
                            // nativeRouteCaptionSize explicitly rounds its product.
                            go_fmadd(e.metadata.original.text.label_width as f64, self.scale, 0.1),
                        )
                    }
                    self.route_caption_knockout(first, &surface)
                }
            }
            self.arrow_with_opacity(
                &e.source_arrow,
                display[0],
                nsub(display[0], display[1]),
                &mat,
                e.stroke_width,
                e.opacity,
            );
            let last = display.len() - 1;
            self.arrow_with_opacity(
                &e.target_arrow,
                display[last],
                nsub(display[last], display[last - 1]),
                &mat,
                e.stroke_width,
                e.opacity,
            );
            if e.metadata.original.animated && e.stroke_width > 0 && wire_opacity > 0.0 {
                let packet = native_material("#e9f8ff", 0.2, 0.15, wire_opacity);
                packet.borrow_mut().emissive = super::geometry::native_paint(&tint, "#657e9e");
                let (mut forward, reverse) = (
                    !e.target_arrow.is_empty() && e.target_arrow != "none",
                    !e.source_arrow.is_empty() && e.source_arrow != "none",
                );
                if !forward && !reverse {
                    forward = true
                }
                packets.push(PacketRoute {
                    points: points.clone(),
                    lengths: lengths.clone(),
                    material: Some(packet),
                    forward,
                    reverse,
                })
            }
            let mut style = e.metadata.original.text.clone();
            if !e.metadata.original.fill.is_empty() {
                style.label_fill = theme.resolve(&e.metadata.original.fill)
            }
            let ink = if e.font_explicit {
                e.font_color.clone()
            } else {
                tint.clone()
            };
            let mut captions = vec![(e.label.clone(), 0.5, style, ink)];
            if let Some(c) = &e.source_label {
                captions.push((c.label.clone(), 0.08, c.clone(), c.color.clone()))
            }
            if let Some(c) = &e.target_label {
                captions.push((c.label.clone(), 0.92, c.clone(), c.color.clone()))
            }
            for (ci, (text, fraction, mut style, ink)) in captions.into_iter().enumerate() {
                style.label_fill = theme.resolve(&style.label_fill);
                let ink = theme.resolve(&ink);
                let icon = ci == 0 && icon;
                if text.is_empty() && !icon {
                    continue;
                }
                let (pw, pd) = native_route_caption_size(&style, icon, self.scale);
                let fixed = if ci == 0 {
                    native_connection_caption_surface(
                        &lanes[i],
                        &e.metadata.original,
                        pw,
                        pd,
                        self.scale,
                    )
                } else {
                    None
                };
                let mut surface = if let Some(mut surface) = fixed {
                    if native_caption_on_edge(&e.metadata.original) {
                        surface.center.y += native_route_radius(e)
                    }
                    surface
                } else {
                    placer.place(if ci > 0 { &display } else { &lanes[i] }, fraction, pw, pd)
                };
                if ci == 0 {
                    self.add_surface_link(&e.link, &e.tooltip, &surface)
                }
                if icon {
                    let shape = crate::target::Shape {
                        id: e.id.clone(),
                        text: e.metadata.original.text.clone(),
                        icon: e.metadata.original.icon.clone(),
                        icon_position: e.metadata.original.icon_position.clone(),
                        ..Default::default()
                    };
                    surface = self.shape_icon(
                        &shape,
                        surface,
                        e.opacity,
                        "edge",
                        Some(e.metadata.original.icon_border_radius),
                    )
                }
                self.label(&text, &surface, &style, &ink, e.opacity, "edge");
            }
        }
        packets
    }
}
