// SPDX-License-Identifier: MPL-2.0
//! render.go and geometry.go: native vector export and source-order mesh assembly.
use super::{
    framing::{NativeSceneOptions, scene_output_density},
    geometry::{LabelSurface, MeshBuilder, native_paint, native_token, readable_surface_ink},
    ground_receivers::{HierarchyShadowSpan, hierarchy_shadow_receivers},
    hierarchy_boards::*,
    hierarchy_headers::hierarchy_board_header_surface,
    hierarchy_support::hierarchy_support_offsets,
    markdown_cards::native_markdown_card,
    mesh::{nadd, nsub, nv},
    native_shapes::NativeAnimatedNode,
    native_solids::native_solid_node,
    native_structured::native_structured_node,
    native_style::native_classic_node,
    rich_labels::{RichLabelPainter, is_rich_label},
    route_labels::RouteCaptionPlacer,
    svg::NativeScene,
    text::TextPainter,
    vector_surface::{
        native_vector_context, native_vector_for_texture, retain_native_vector_surface,
    },
};
use crate::{
    scene::{Node, RenderOpts, Scene, build_scene},
    target::Diagram,
};
use d2_rs_scene::{go_max, go_min};
use std::collections::{BTreeSet, HashMap};
#[derive(Clone, Default)]
pub struct Options {
    pub render: RenderOpts,
    pub width: i64,
    pub height: i64,
    pub fit_content: bool,
    pub fonts: Option<d2_rs_scenebuild::FontFallbackOptions>,
    pub assets: Option<d2_rs_scenebuild::AssetOptions>,
    /// Empty selects PNG, matching the public Go image API.
    pub format: String,
    /// Nanoseconds; zero selects two minutes. Signed to preserve admission errors.
    pub timeout_ns: i64,
    pub link_budget: Option<d2_rs_scenebuild::LinkBudget>,
}
/// render.go:61–96. Validate before scene construction or resource acquisition.
pub fn normalize(options: Option<&Options>) -> Result<Options, String> {
    let mut o = options.cloned().unwrap_or_default();
    if o.format.is_empty() {
        o.format = "png".into();
    }
    if !matches!(o.format.as_str(), "svg" | "png" | "gif") {
        return Err(format!(
            "unsupported isometric image format {} (use svg, png or gif)",
            d2_rs_core::quote_go_bytes(o.format.as_bytes())
        ));
    }
    if o.width == 0 && o.height == 0 {
        (o.width, o.height) = if o.format == "gif" {
            (1000, 625)
        } else {
            (1600, 1000)
        };
    }
    if o.width < 64
        || o.height < 64
        || o.width > 4096
        || o.height > 4096
        || o.width * o.height > 12_000_000
    {
        return Err(format!(
            "isometric image dimensions {}x{} exceed limits (64..4096 per side and at most 12000000 pixels)",
            o.width, o.height
        ));
    }
    if o.format == "gif" && o.width * o.height * 104 > 100_000_000 {
        return Err(
            "isometric GIF pixels exceed 100000000 across the traffic cycle; reduce --scale".into(),
        );
    }
    if o.timeout_ns == 0 {
        o.timeout_ns = 120_000_000_000;
    }
    if o.timeout_ns < 0 || o.timeout_ns > 120_000_000_000 {
        return Err("isometric image timeout must be positive and at most two minutes".into());
    }
    if o.link_budget.is_some_and(|b| {
        b.max_regions <= 0
            || b.max_regions > 4096
            || b.max_string_bytes <= 0
            || b.max_string_bytes > 1 << 20
    }) {
        return Err(
            "isometric page link budget must be within 4096 regions and 1 MiB of text".into(),
        );
    }
    Ok(o)
}
pub fn render_svg(d: &Diagram, o: &Options) -> Result<String, String> {
    render_svg_with_context(None, Some(d), Some(o)).map_err(|e| e.to_string())
}
/// render.go Render: validate options, then select SVG, PNG or looping GIF.
pub fn render_with_context(
    token: Option<&d2_rs_core::CancellationToken>,
    diagram: Option<&Diagram>,
    options: Option<&Options>,
) -> Result<Vec<u8>, super::context::RenderError> {
    let o = normalize(options)?;
    match o.format.as_str() {
        "svg" => render_svg_with_context(token, diagram, Some(&o)).map(String::into_bytes),
        "png" => render_png_with_context(token, diagram, Some(&o)),
        "gif" => super::gif::render_gif_with_context(token, diagram, Some(&o)),
        _ => unreachable!("normalized format"),
    }
}
/// Lossless still PNG with the public image API's admission, caller cause and
/// child deadline. Like the SVG convenience entry point, this selects its format.
pub fn render_png_with_context(
    token: Option<&d2_rs_core::CancellationToken>,
    diagram: Option<&Diagram>,
    options: Option<&Options>,
) -> Result<Vec<u8>, super::context::RenderError> {
    let mut options = options.cloned().unwrap_or_default();
    options.format = "png".into();
    let o = normalize(Some(&options))?;
    token
        .map(super::context::RenderContext::from_token)
        .unwrap_or_default()
        .check()?;
    let ctx = super::context::RenderContext::new(token, o.timeout_ns);
    let d = diagram
        .ok_or_else(|| super::context::RenderError::Message("isometric: nil diagram".into()))?;
    render_static_png(&ctx, d, &o).map(|(png, _)| png)
}

/// native.go openCapture/frame's shared still path. Page adds preflight and
/// projects annotations after this same capture; the caller owns every result.
pub(crate) fn render_static_png(
    ctx: &super::context::RenderContext,
    diagram: &Diagram,
    o: &Options,
) -> Result<(Vec<u8>, NativeScene), super::context::RenderError> {
    let scene = build_scene(diagram, Some(&o.render))?;
    ctx.check()?;
    let native = build_native_scene_mode(&scene, o.width, o.height, o, false, ctx)
        .map_err(|e| ctx.error(format!("initialize native isometric renderer: {e}")))?;
    ctx.check()?;
    let frame = super::raster::render(&native)
        .map_err(|e| ctx.error(format!("render isometric frame at 0.0000s: {e}")))?;
    ctx.check()?;
    let png = d2_rs_raster::encode_png_default(&frame)
        .map_err(|e| ctx.error(format!("encode isometric PNG: {e}")))?;
    if png.len() > 32 << 20 {
        return Err("isometric PNG exceeds output byte limit".to_string().into());
    }
    ctx.check()?;
    Ok((png, native))
}
/// Native SVG export with caller cancellation and a bounded renderer deadline.
/// The convenience SVG entry point selects SVG regardless of Options.format.
pub fn render_svg_with_context(
    token: Option<&d2_rs_core::CancellationToken>,
    diagram: Option<&Diagram>,
    options: Option<&Options>,
) -> Result<String, super::context::RenderError> {
    let mut options = options.cloned().unwrap_or_default();
    options.format = "svg".into();
    let o = normalize(Some(&options))?;
    let ctx = super::context::RenderContext::new(token, o.timeout_ns);
    ctx.check()?;
    let d = diagram.ok_or_else(|| {
        super::context::RenderError::Message("native SVG requires a diagram".into())
    })?;
    let (width, height) = (o.width, o.height);
    let budget = o.link_budget.unwrap_or(d2_rs_scenebuild::LinkBudget {
        max_regions: 4096,
        max_string_bytes: 1 << 20,
    });
    let (mut bytes, mut regions) = (0, 0);
    for (link, tooltip) in d
        .shapes
        .iter()
        .flatten()
        .map(|s| (&s.link, &s.tooltip))
        .chain(
            d.connections
                .iter()
                .flatten()
                .map(|c| (&c.link, &c.tooltip)),
        )
    {
        ctx.check()?;
        if link.is_empty() && tooltip.is_empty() {
            continue;
        }
        regions += 1;
        if regions > budget.max_regions {
            return Err(super::context::RenderError::Message(
                "native SVG exceeds link region budget".into(),
            ));
        }
        for value in [link, tooltip] {
            if value.len() as i64 > budget.max_string_bytes - bytes {
                return Err(super::context::RenderError::Message(
                    "native SVG exceeds link text budget".into(),
                ));
            }
            bytes += value.len() as i64;
            for c in value.chars() {
                ctx.check()?;
                super::page::validate_page_link_text(c.encode_utf8(&mut [0; 4]))?;
            }
        }
        super::page::validate_native_link(link)?;
    }
    ctx.check()?;
    let scene = build_scene(d, Some(&o.render))?;
    let mut native = build_native_scene_mode(&scene, width, height, &o, true, &ctx)
        .map_err(|e| ctx.error(format!("initialize native SVG: {e}")))?;
    let surface_context = ctx.clone();
    super::svg::native_scene_svg_with_context(
        &mut native,
        &move |surface, prefix| {
            super::vector_surface_svg::native_surface_svg_with_context(
                surface,
                prefix,
                &surface_context,
            )
        },
        &ctx,
    )
    .map_err(|e| ctx.error(e))
}
pub fn build_native_scene(
    s: &Scene,
    width: i64,
    height: i64,
    o: &Options,
) -> Result<NativeScene, String> {
    build_native_scene_mode(s, width, height, o, true, &Default::default())
}
/// The ordinary raster mesh omits SVG-only solid-cap coverage triangles.
pub fn build_native_scene_for_raster(
    s: &Scene,
    width: i64,
    height: i64,
    o: &Options,
) -> Result<NativeScene, String> {
    build_native_scene_mode(s, width, height, o, false, &Default::default())
}
pub(crate) fn build_native_scene_mode(
    s: &Scene,
    width: i64,
    height: i64,
    o: &Options,
    vector: bool,
    ctx: &super::context::RenderContext,
) -> Result<NativeScene, String> {
    build_native_scene_with_camera(s, width, height, o, vector, ctx, None)
}

pub(crate) fn build_native_scene_with_camera(
    s: &Scene,
    width: i64,
    height: i64,
    o: &Options,
    vector: bool,
    ctx: &super::context::RenderContext,
    camera: Option<super::mesh::Camera>,
) -> Result<NativeScene, String> {
    ctx.check_string()?;
    let options = NativeSceneOptions {
        camera,
        vector,
        retain_vector_surfaces: vector,
        defer_raster: true,
        fit_content: o.fit_content,
        output_density: scene_output_density(s, width, height, camera.as_ref()),
        links: Some(o.link_budget.unwrap_or(d2_rs_scenebuild::LinkBudget {
            max_regions: 4096,
            max_string_bytes: 1 << 20,
        })),
        ..Default::default()
    };
    build_native_scene_with_options(s, width, height, o, options, ctx)
}

/// geometry.go newNativeSceneWithOptions: preserve the caller's explicit
/// density and framing options, including legacy density zero.
pub(crate) fn build_native_scene_with_options(
    s: &Scene,
    width: i64,
    height: i64,
    o: &Options,
    options: NativeSceneOptions,
    ctx: &super::context::RenderContext,
) -> Result<NativeScene, String> {
    ctx.check_string()?;
    // geometry.go passes the caller context to each painter. Only an SVG
    // context retains vector documents; ordinary raster calls retain pixels.
    let vectors = if options.retain_vector_surfaces {
        native_vector_context()
    } else {
        None
    };
    let mut text = TextPainter::new(
        (s.boards.len() + s.nodes.len() + s.edges.len() * 3 + 2) as i64,
        vectors.clone(),
    )?;
    text.context = ctx.clone();
    text.primary = s.font_family.clone().unwrap_or_default();
    text.mono = s.mono_font_family.clone().unwrap_or_default();
    text.fallback_fonts = o.fonts.clone();
    let density = options.output_density;
    text.output_density = density;
    let mut rich_count = s
        .nodes
        .iter()
        .filter(|n| is_rich_label(&n.metadata.original))
        .count()
        + is_rich_label(&s.root) as usize;
    for edge in &s.edges {
        for text in [
            Some(&edge.metadata.original.text),
            edge.source_label.as_ref(),
            edge.target_label.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            let shape = crate::target::Shape {
                text: text.clone(),
                ..Default::default()
            };
            rich_count += is_rich_label(&shape) as usize;
        }
    }
    let mut rich = RichLabelPainter::new(rich_count as i64, vectors.clone())?;
    rich.context = ctx.clone();
    rich.primary = text.primary.clone();
    rich.mono = text.mono.clone();
    rich.theme_id = Some(s.theme_id);
    rich.theme_overrides = s.theme_overrides.clone();
    rich.fallback_fonts = o.fonts.clone();
    rich.output_density = density;
    let icon_count = s.root.icon.is_some() as usize
        + s.nodes.iter().filter(|n| !n.icon.is_empty()).count()
        + s.edges.iter().filter(|e| !e.icon.is_empty()).count();
    let mut icons = super::surface_icons::SurfaceIconPainter::new_with_context(
        ctx.clone(),
        icon_count as i64,
        o.assets.as_ref(),
        vectors.clone(),
    )?;
    icons.output_density = density;
    let mut b = MeshBuilder {
        text,
        rich,
        icons,
        scale: if s.pixel_scale <= 0.0 {
            0.01
        } else {
            s.pixel_scale
        },
        vectors,
        fonts: o.fonts.clone(),
        output_density: density,
        options,
        ..Default::default()
    };
    b.geometry.context = ctx.clone();
    b.face_max_pixels = 64.max(24 * 1024 * 1024 / (s.nodes.len() + s.boards.len()).max(1) as i64);
    let mut theme = crate::theme::Theme::find(s.theme_id)
        .unwrap_or_else(|| crate::theme::Theme::find(0).unwrap());
    theme.apply_overrides(s.theme_overrides.as_ref());
    b.arrow_background = theme.resolve("N7");
    let nodes: NodeIndex = s.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    let boards = hierarchy_presentation_boards(&s.boards, &nodes);
    b.route_casing_floor = hierarchy_casing_floor(&boards, &nodes);
    b.hierarchy_supports = hierarchy_support_offsets(&boards);
    let header_nodes = hierarchy_render_nodes(&s.nodes, Some(&b.hierarchy_supports));
    let levels: Vec<_> = s
        .boards
        .iter()
        .map(|b| b.level)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let colors = ["#7898ba", "#9287b7", "#7da89c", "#b69b80"];
    let mut tints: HashMap<String, String> = HashMap::new();
    let mut board_index = HashMap::new();
    for board in &boards {
        let tint = colors[levels.binary_search(&board.level).unwrap_or_else(|i| i) % colors.len()];
        tints.insert(
            board.id.clone(),
            hierarchy_board_tint(nodes.get(board.source_id.as_str()).copied(), tint),
        );
        board_index.insert(board.id.clone(), board.clone());
    }
    let background = if native_token(&s.root.fill) {
        "#f5f7fb"
    } else {
        &s.background
    };
    let mut captions = RouteCaptionPlacer::new();
    let mut spans = vec![];
    for board in &boards {
        ctx.check_string()?;
        let owner = nodes.get(board.source_id.as_str()).copied();
        let opacity = owner.map_or(1.0, |n| n.opacity);
        let tint = tints.get(&board.id).cloned().unwrap_or_default();
        if opacity <= 0.0 {
            continue;
        }
        let (w, d) = (go_max(0.01, board.size.x), go_max(0.01, board.size.z));
        let (first, animation) = (b.triangles.len(), b.animated_nodes.len());
        b.hierarchy_board(board, owner, &tint, opacity);
        if !board.label.is_empty() || owner.is_some_and(|n| !n.icon.is_empty()) {
            let style = owner
                .map(|n| n.metadata.original.text.clone())
                .unwrap_or_default();
            let mut ink = hierarchy_header_ink(board, &board_index, &nodes, &tints, background);
            if let Some(n) = owner {
                if n.font_explicit {
                    ink = n.font_color.clone();
                }
            }
            let hd = if board.header_depth <= 0.0 {
                0.62
            } else {
                board.header_depth
            };
            let lw = if style.label_width <= 0 {
                board.label.chars().count() as f64 * 14.0
            } else {
                style.label_width as f64
            };
            let lh = if style.label_height <= 0 {
                30.0
            } else {
                style.label_height as f64
            };
            let (mut pw, pd) = (go_min(w, lw * b.scale), go_min(d, lh * b.scale));
            if owner.is_some_and(|n| !n.icon.is_empty()) {
                pw = go_min(go_max(0.0, w - 0.7), pw + pd + 0.12);
            }
            let mut surface = LabelSurface {
                center: nadd(
                    board.position,
                    nv(-w / 2.0 + 0.35 + pw / 2.0, 0.032, -d / 2.0 + hd / 2.0),
                ),
                width: pw,
                depth: pd,
                align: "left".into(),
                ..Default::default()
            };
            if let Some(owner) = owner {
                surface.center.y = hierarchy_surface_y(board) + 0.00006;
                surface = hierarchy_board_header_surface(
                    surface,
                    board,
                    owner,
                    &header_nodes,
                    b.scale,
                    Some(&boards),
                );
            }
            captions.avoid(surface.center, pw, pd);
            if let Some(owner) = owner {
                surface = b.shape_icon(&owner.metadata.original, surface, opacity, "board", None);
                let mut original = owner.metadata.original.clone();
                if owner.font_explicit
                    && native_token(&original.text.label_fill)
                    && hierarchy_header_ink(board, &board_index, &nodes, &tints, background)
                        == readable_surface_ink(&ink, 1.0)
                {
                    original.text.label_fill = tint.clone();
                    if native_paint(&tint, "transparent").a == 0
                        || readable_surface_ink(&tint, 1.0) == readable_surface_ink(&ink, 1.0)
                    {
                        original.text.label_fill = readable_surface_ink(&ink, 1.0);
                    }
                }
                b.shape_label(&original, &surface, &ink, opacity, "board");
            } else {
                b.label(&board.label, &surface, &style, &ink, opacity, "board");
            }
        }
        if let Some(owner) = owner {
            physical_shadows(
                &mut b.geometry.triangles[first..],
                owner.metadata.original.shadow,
            );
            b.add_mesh_link(
                &owner.metadata.original.link,
                &owner.metadata.original.tooltip,
                first,
            );
            if owner.metadata.original.animated && b.triangles.len() > first {
                let last = b.triangles.len();
                if b.animated_nodes.len() == animation {
                    b.animated_nodes.push(NativeAnimatedNode { first, last });
                } else {
                    b.animated_nodes[animation].last = last;
                }
            }
        }
        if let Some(owner) = owner {
            b.remember_arrow_owner(owner, Some(board), first);
        }
        spans.push(HierarchyShadowSpan {
            first,
            last: b.triangles.len(),
            board: board.id.clone(),
        });
    }
    for n in &s.nodes {
        ctx.check_string()?;
        if !n.container && n.opacity > 0.0 && !hierarchy_spacer(n) {
            let first = b.triangles.len();
            let tint = tints.get(&n.board_id).map_or("", String::as_str);
            b.sequence_node(n, tint);
            captions.avoid_mesh(&b.triangles[first..]);
            b.add_mesh_link(
                &n.metadata.original.link,
                &n.metadata.original.tooltip,
                first,
            );
            b.remember_arrow_owner(n, None, first);
            spans.push(HierarchyShadowSpan {
                first,
                last: b.triangles.len(),
                board: n.board_id.clone(),
            });
        }
    }
    ctx.check_string()?;
    let packets = b.edges(&s.edges, &mut captions, s);
    if let Some(e) = b.err.clone() {
        return Err(e);
    }
    b.root_decorations(s, o.assets.as_ref())?;
    hierarchy_shadow_receivers(&mut b.geometry.triangles, &boards, &spans);
    ctx.check_string()?;
    b.finish_scene(s, width, height, &packets)
}
pub fn physical_shadows(ts: &mut [super::mesh::Triangle], authored: bool) {
    for t in ts {
        if t.cast_shadow {
            t.shadow_opacity = Some(if authored { 1.0 } else { 0.72 });
        }
    }
}
impl MeshBuilder {
    pub fn vector_for_texture(
        &self,
        t: &std::rc::Rc<super::mesh::Texture>,
    ) -> Option<std::rc::Rc<super::vector_surface::VectorSurface>> {
        native_vector_for_texture(&self.vectors, t)
    }
    pub fn retain_vector_surface(
        &self,
        t: &std::rc::Rc<super::mesh::Texture>,
        d: &d2_rs_scene::Document,
    ) -> Result<(), String> {
        retain_native_vector_surface(&self.vectors, t, d)
    }
    pub fn node(&mut self, n: &Node, tint: &str, relief: Option<f64>) {
        let n = native_classic_node(n);
        let first = self.triangles.len();
        let first_link = self.links.len();
        if native_markdown_card(&n) {
            self.markdown_card(&n, tint);
        } else if native_structured_node(&n) {
            self.structured_node(&n);
        } else if native_solid_node(&n) {
            self.solid_node(&n);
        } else {
            self.canonical_node(&n, tint);
        }
        let factor = relief.unwrap_or(1.0);
        let before = if self.triangles.len() > first {
            self.triangles.last().unwrap().v[0].position
        } else {
            Default::default()
        };
        if n.sequence_role.is_empty() && !native_structured_node(&n) && !native_markdown_card(&n) {
            self.clear_outside_caption(&n, first, factor);
        }
        if self.triangles.len() > first {
            let delta = nsub(self.triangles.last().unwrap().v[0].position, before);
            for link in &mut self.links[first_link..] {
                for p in &mut link.points {
                    *p = nadd(*p, delta);
                }
            }
        }
        if relief.is_none() && !native_structured_node(&n) && !native_markdown_card(&n) {
            self.classic_ink_edges(&n, first);
        }
        physical_shadows(
            &mut self.geometry.triangles[first..],
            n.metadata.original.shadow,
        );
        if n.metadata.original.animated && self.triangles.len() > first {
            self.animated_nodes.push(NativeAnimatedNode {
                first,
                last: self.triangles.len(),
            });
        }
        self.mark_paint_owner(first);
    }
    pub fn sequence_node(&mut self, n: &Node, tint: &str) {
        let mut node = n.clone();
        match node.sequence_role.as_str() {
            "group" => {
                let first_animation = self.animated_nodes.len();
                let owner = native_classic_node(&node);
                let board = crate::scene::Board {
                    id: node.id.clone(),
                    source_id: node.id.clone(),
                    kind: "sequence-group".into(),
                    level: node.metadata.original.level,
                    position: node.position,
                    size: node.size,
                    ..Default::default()
                };
                self.hierarchy_board(&board, Some(&owner), &node.fill, node.opacity);
                let source = super::native_shapes::native_face_source(&owner, &owner.fill);
                self.canonical_node_content(&owner, &source, &owner.fill, 0.29);
                if self.animated_nodes.len() > first_animation {
                    self.animated_nodes[first_animation].last = self.triangles.len();
                }
            }
            "span" => {
                let top = 0.28 + node.metadata.original.level.clamp(0, 128) as f64 * 0.00001;
                node.size.y = (top - 0.10) / 1.15;
                node.position.y = top
                    - super::native_shapes::native_canonical_height(&node, self.scale)
                    + node.size.y / 2.0;
                self.node(&node, tint, None);
            }
            "note" => {
                node.position.y = 0.16 + node.size.y / 2.0;
                self.hierarchy_node(&node, tint);
            }
            _ => self.hierarchy_node(&node, tint),
        }
    }
}
