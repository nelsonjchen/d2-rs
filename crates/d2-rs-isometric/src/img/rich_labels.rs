// SPDX-License-Identifier: MPL-2.0
//! d2isometricimg/rich_labels.go: bounded typed Markdown, code, class and SQL
//! surface content. Geometry remains in the original compiled coordinates.
use std::rc::Rc;

use d2_rs_scene::{Box as SceneBox, Document, Node, Nrgba, ViewportAlign, ViewportFit};
use d2_rs_scenebuild::{LinkBudget, Options};

use super::{
    mesh::Texture,
    surface_document::{
        compose_surface, document_glyph_coverage, native_font_family, surface_fonts,
    },
    text::{LabelTextStyle, MAX_TEXT_LABELS, surface_texture_dimensions_at_density},
    vector_surface::{VectorCtx, retain_styled_native_vector_surface},
};
use crate::target::{Diagram, Shape, ThemeOverrides};

pub const MAX_RICH_LABEL_BYTES: usize = 64 * 1024;
pub const MAX_RICH_SOURCE_BYTES: usize = 1024 * 1024;
const MAX_RICH_ROWS: usize = 512;
pub const MAX_RICH_TOTAL_ROWS: usize = 4096;
pub const MAX_RICH_PIXELS: usize = 8 * 1024 * 1024;

#[derive(Default)]
pub struct RichLabelPainter {
    pub context: super::context::RenderContext,
    pub last_document: Option<Rc<Document>>,
    pub count: i64,
    pub used: i64,
    pub pixels: i64,
    pub(crate) source_bytes: usize,
    pub(crate) rows: usize,
    pub primary: String,
    pub mono: String,
    pub output_density: f64,
    pub theme_id: Option<i64>,
    pub theme_overrides: Option<ThemeOverrides>,
    pub fallback_fonts: Option<d2_rs_scenebuild::FontFallbackOptions>,
    pub vectors: VectorCtx,
}
impl RichLabelPainter {
    pub fn new(count: i64, vectors: VectorCtx) -> Result<Self, String> {
        if !(0..=MAX_TEXT_LABELS as i64).contains(&count) {
            return Err(format!(
                "isometric rich labels exceed {MAX_TEXT_LABELS} labels"
            ));
        }
        Ok(Self {
            count,
            vectors,
            ..Default::default()
        })
    }
    pub fn texture(
        &mut self,
        original: &Shape,
        style: &LabelTextStyle,
    ) -> Result<Option<Rc<Texture>>, String> {
        self.context.check_string()?;
        self.last_document = None;
        if self.used >= self.count {
            return Err("isometric rich label allocation exceeds its declared count".into());
        }
        if !is_rich_label(original) {
            return Err("isometric rich label has no supported rich content".into());
        }
        if [
            style.width,
            style.depth,
            style.pixel_scale,
            style.font_size,
            style.opacity,
        ]
        .iter()
        .any(|v| !v.is_finite())
            || style.width <= 0.0
            || style.depth <= 0.0
            || style.pixel_scale <= 0.0
            || style.font_size < 0.0
            || !(0.0..=1.0).contains(&style.opacity)
        {
            return Err("isometric rich label has invalid surface dimensions or style".into());
        }
        let (bytes, rows) = rich_source_size(original)?;
        if bytes > MAX_RICH_SOURCE_BYTES - self.source_bytes
            || rows > MAX_RICH_TOTAL_ROWS - self.rows
        {
            return Err("isometric rich labels exceed aggregate source or row budget".into());
        }
        self.source_bytes += bytes;
        self.rows += rows;
        self.used += 1;
        let mut document = rich_label_document(
            original,
            style,
            self.fallback_fonts.as_ref(),
            self.theme_id,
            self.theme_overrides.clone(),
            &self.primary,
            &self.mono,
        )
        .map_err(|e| format!("isometric rich label {:?}: {e}", original.id))?;
        let structured = matches!(original.type_.as_str(), "sql_table" | "class");
        if !structured {
            fit_rich_viewport(&mut document, style)?;
        }
        let budget = (4 * 1024 * 1024).min(MAX_RICH_PIXELS / (self.count as usize).max(1));
        let (w, h) = surface_texture_dimensions_at_density(
            style.width,
            style.depth,
            4096,
            budget,
            self.output_density,
        );
        if w * h > MAX_RICH_PIXELS - self.pixels as usize {
            return Err("isometric rich labels exceed texture pixel budget".into());
        }
        self.pixels += (w * h) as i64;
        document.logical_width = w as f64;
        document.logical_height = h as f64;
        document.viewport_fit = if structured {
            ViewportFit::STRETCH
        } else {
            ViewportFit::MEET
        };
        document.viewport_align = ViewportAlign::X_MID_Y_MID;
        let frame =
            d2_rs_raster::render(&document, &rich_raster_options()).map_err(|e| e.to_string())?;
        let pixels = compose_surface(&frame.pix, style, w, h)?;
        let texture = Rc::new(Texture::new(w, h, pixels));
        self.last_document = Some(Rc::new(document.clone()));
        retain_styled_native_vector_surface(
            &self.vectors,
            &texture,
            &document,
            style
                .background_paint
                .as_ref()
                .cloned()
                .or_else(|| {
                    style
                        .background
                        .map(|color| d2_rs_scene::Paint::SolidPaint { color })
                })
                .as_ref(),
            style.opacity,
        )?;
        Ok(Some(texture))
    }
}
pub fn is_rich_label(s: &Shape) -> bool {
    matches!(s.type_.as_str(), "sql_table" | "class" | "code") || !s.text.language.trim().is_empty()
}
pub fn rich_color(c: Nrgba) -> String {
    format!("#{:02x}{:02x}{:02x}{:02x}", c.r, c.g, c.b, c.a)
}
pub fn rich_source_size(s: &Shape) -> Result<(usize, usize), String> {
    let rows = s.fields.as_ref().map_or(0, Vec::len)
        + s.methods.as_ref().map_or(0, Vec::len)
        + s.columns.as_ref().map_or(0, Vec::len);
    if rows > MAX_RICH_ROWS {
        return Err(format!(
            "isometric rich label exceeds {MAX_RICH_ROWS} structured rows"
        ));
    }
    let mut size = 0;
    let mut add = |v: &str| -> Result<(), String> {
        if v.len() > MAX_RICH_LABEL_BYTES - size {
            return Err(
                "isometric rich label exceeds source byte budget or contains invalid UTF-8".into(),
            );
        }
        size += v.len();
        Ok(())
    };
    add(&s.text.label)?;
    for f in s.fields.iter().flatten() {
        for v in [&f.name, &f.type_, &f.visibility] {
            add(v)?;
        }
    }
    for f in s.methods.iter().flatten() {
        for v in [&f.name, &f.return_, &f.visibility] {
            add(v)?;
        }
    }
    for c in s.columns.iter().flatten() {
        if c.constraint.as_ref().map_or(0, Vec::len) > MAX_RICH_ROWS {
            return Err("isometric rich label exceeds constraint limit".into());
        }
        add(&c.name.label)?;
        add(&c.type_.label)?;
        for v in c.constraint.iter().flatten() {
            add(v)?;
        }
    }
    Ok((size, rows))
}
pub fn rich_label_document(
    original: &Shape,
    style: &LabelTextStyle,
    fonts: Option<&d2_rs_scenebuild::FontFallbackOptions>,
    theme_id: Option<i64>,
    overrides: Option<ThemeOverrides>,
    primary: &str,
    mono: &str,
) -> Result<Document, String> {
    let mut s = Shape {
        id: "surface".into(),
        type_: original.type_.clone(),
        opacity: 1.0,
        stroke: "none".into(),
        fill: "transparent".into(),
        text: original.text.clone(),
        fields: original.fields.clone(),
        methods: original.methods.clone(),
        columns: original.columns.clone(),
        label_position: "INSIDE_MIDDLE_CENTER".into(),
        ..Default::default()
    };
    s.text.language = s.text.language.trim().to_lowercase();
    if s.text.language == "md" {
        s.text.language = "markdown".into();
    }
    if s.text.font_size <= 0 {
        s.text.font_size = (style.font_size.round() as i64).max(1);
    }
    if s.text.font_size <= 0 {
        s.text.font_size = 16;
    }
    if s.text.font_size > 10000 {
        return Err("rich font size exceeds limit".into());
    }
    let primary = native_font_family(&original.text.font_family, primary, mono)?;
    let mono = if mono.is_empty() {
        "SourceCodePro"
    } else {
        mono
    };
    s.text.font_family = "DEFAULT".into();
    let ink = if style.foreground_paint.is_some() {
        style.foreground.clone()
    } else {
        rich_color(style.color)
    };
    s.text.color = ink.clone();
    if s.text.language == "latex" {
        s.stroke = ink.clone();
    }
    let structured = matches!(s.type_.as_str(), "sql_table" | "class");
    if structured {
        s.width = original.width;
        s.height = original.height;
        s.fill = original.fill.clone();
        s.stroke = original.stroke.clone();
        s.text.color = original.text.color.clone();
        if s.fill.is_empty() {
            s.fill = ink.clone();
        }
        if s.stroke.is_empty() {
            s.stroke = "transparent".into();
        }
        if s.text.color.is_empty() {
            s.text.color = super::surface_document::readable_surface_ink(&s.fill).into();
        }
        s.border_radius = original.border_radius;
        s.primary_accent_color = if original.primary_accent_color.is_empty() {
            ink.clone()
        } else {
            original.primary_accent_color.clone()
        };
        s.secondary_accent_color = if original.secondary_accent_color.is_empty() {
            ink.clone()
        } else {
            original.secondary_accent_color.clone()
        };
        s.neutral_accent_color = if original.neutral_accent_color.is_empty() {
            ink.clone()
        } else {
            original.neutral_accent_color.clone()
        };
    } else {
        s.text.label_width = original.text.label_width;
        s.text.label_height = original.text.label_height;
        if s.text.label_width <= 0 {
            s.text.label_width = (style.width / style.pixel_scale).min(100000.0).max(1.0) as i64;
        }
        if s.text.label_height <= 0 {
            s.text.label_height = (style.depth / style.pixel_scale).min(100000.0).max(1.0) as i64;
        }
        s.width = s.text.label_width;
        s.height = s.text.label_height;
        if matches!(s.text.language.as_str(), "markdown" | "latex") {
            s.type_ = "text".into();
        } else {
            s.type_ = "code".into();
            if s.text.language.is_empty() {
                s.text.language = "text".into();
            }
            s.width += s.text.font_size;
            s.height += s.text.font_size;
        }
    }
    if s.width <= 0
        || s.height <= 0
        || s.width > 100000
        || s.height > 100000
        || s.text.label_width < 0
        || s.text.label_height < 0
        || s.text.label_width > 100000
        || s.text.label_height > 100000
    {
        return Err("rich label dimensions exceed limits".into());
    }
    let (w, h) = (s.width, s.height);
    let diagram = Diagram {
        root: Shape {
            fill: "transparent".into(),
            stroke: "none".into(),
            ..Default::default()
        },
        font_family: Some(primary),
        mono_font_family: Some(mono.into()),
        shapes: Some(vec![s]),
        ..Default::default()
    };
    let theme = theme_id.unwrap_or(
        if style.color.r as i64 * 299 + style.color.g as i64 * 587 + style.color.b as i64 * 114
            > 180000
        {
            200
        } else {
            0
        },
    );
    let mut doc = d2_rs_scenebuild::build(
        &diagram,
        &Options {
            assets: Some(super::surface_document::surface_assets()),
            pad: Some(0),
            theme_id: Some(theme),
            theme_overrides: overrides,
            max_nodes: 20000,
            max_path_commands: 250000,
            link_budget: LinkBudget {
                max_regions: 2048,
                max_string_bytes: MAX_RICH_LABEL_BYTES as i64,
            },
            fonts: Some(surface_fonts(fonts)?),
            ..Default::default()
        },
    )
    .map_err(|e| e.to_string())?;
    document_glyph_coverage(&doc)?;
    doc.view_box = SceneBox {
        width: w as f64,
        height: h as f64,
        ..Default::default()
    };
    if structured {
        if let Some(root) = &mut doc.root {
            strip_rich_bodies(root);
        }
    }
    Ok(doc)
}
pub fn fit_rich_viewport(document: &mut Document, style: &LabelTextStyle) -> Result<(), String> {
    let (mut w, mut h) = (
        style.width / style.pixel_scale,
        style.depth / style.pixel_scale,
    );
    if !w.is_finite() || !h.is_finite() || w <= 0.0 || h <= 0.0 {
        return Err("isometric rich label has invalid physical pixel dimensions".into());
    }
    let source = document.view_box;
    let fit = 1.0f64.min((w / source.width).min(h / source.height));
    w /= fit;
    h /= fit;
    document.view_box = SceneBox {
        x: source.x + (source.width - w) / 2.0,
        y: source.y + (source.height - h) / 2.0,
        width: w,
        height: h,
    };
    Ok(())
}
fn strip_rich_bodies(n: &mut Node) {
    if n.id == "surface:outline" {
        n.primitive = None;
    }
    for child in n.children.iter_mut().flatten() {
        strip_rich_bodies(child);
    }
}
pub(crate) fn rich_raster_options() -> d2_rs_raster::FrameOptions {
    d2_rs_raster::FrameOptions {
        scale: 1.0,
        max_width: 4096,
        max_height: 4096,
        max_pixels: MAX_RICH_PIXELS as i64,
        max_nodes: 100000,
        max_depth: 128,
        max_path_commands: 500000,
        max_text_runes_per_run: MAX_RICH_LABEL_BYTES as i64,
        max_text_coverage_checks: 2000000,
        max_text_shaping_runs: 20000,
        max_font_faces_per_text: 4,
        max_animation_tracks: 1,
        max_animation_keyframes: 1,
        max_assets: 4096,
        max_asset_bytes: 32 << 20,
        max_decoded_asset_bytes: 1,
        max_import_depth: 8,
        max_offscreen_bytes: 32 << 20,
        max_even_odd_clip_work: 20000000,
        max_scanline_work: 100000000,
        ..Default::default()
    }
}
