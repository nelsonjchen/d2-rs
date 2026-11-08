// SPDX-License-Identifier: MPL-2.0
//! d2isometricimg/text.go: bounded plain labels, shaped at a fixed reference
//! size, with float32 glyph paths shared by raster and retained SVG output.
//! Complex/color text uses the bounded native scene fallback path. Externally
//! registered families use explicit process registrations.
use super::{
    mesh::Texture,
    vector_surface::{VectorCtx, retain_native_vector_surface},
};
use d2_rs_fontface::{
    ParsedFace, ShapeFace, ShapeLimits, ShapedText, ShapingWorkspace, sfnt::SegmentOp,
};
use d2_rs_scene::{
    Box as SceneBox, Document, Node, Nrgba, Paint, Path, PathCommand, Primitive, Rect, go_max,
    go_min,
};
use d2_rs_text::d2fonts::FontStyle;
use std::{collections::HashMap, rc::Rc};

pub const MAX_TEXT_PIXELS: usize = 8 * 1024 * 1024;
pub const MAX_TEXT_LABELS: usize = 19000;
pub const MAX_LABEL_RUNES: usize = 64 * 1024;
pub const MAX_TEXT_SOURCE_BYTES: usize = 8 * 1024 * 1024;
const MAX_TEXT_FONT_BYTES: usize = 32 * 1024 * 1024;
const MAX_TEXT_WORK: usize = 8_000_000;

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct LabelTextStyle {
    pub width: f64,
    pub depth: f64,
    pub font_size: f64,
    pub pixel_scale: f64,
    pub font_family: String,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub color: Nrgba,
    pub background: Option<Nrgba>,
    pub foreground_paint: Option<Paint>,
    pub background_paint: Option<Paint>,
    pub foreground: String,
    pub opacity: f64,
    pub align: String,
    pub max_lines: i64,
}
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct TextLayout {
    pub lines: Vec<String>,
    pub font_size: f64,
    pub line_height: f64,
    pub truncated: bool,
}
#[derive(Clone, Default)]
struct ShapedLine {
    text: ShapedText,
    min_x: f64,
    max_x: f64,
    min_y: f64,
    max_y: f64,
}
#[derive(Default)]
pub struct TextPainter {
    pub context: super::context::RenderContext,
    pub vectors: VectorCtx,
    pub fallback_fonts: Option<d2_rs_scenebuild::FontFallbackOptions>,
    count: usize,
    used: usize,
    pixels: usize,
    source_bytes: usize,
    font_bytes: usize,
    glyph_work: usize,
    outline_work: usize,
    pub primary: String,
    pub mono: String,
    pub output_density: f64,
    faces: HashMap<(String, bool, bool), Rc<ParsedFace>>,
    shaper: ShapingWorkspace,
}
impl TextPainter {
    pub fn new(label_count: i64, vectors: VectorCtx) -> Result<Self, String> {
        if !(0..=MAX_TEXT_LABELS as i64).contains(&label_count) {
            return Err(format!("isometric text exceeds {MAX_TEXT_LABELS} labels"));
        }
        Ok(Self {
            count: label_count as usize,
            vectors,
            ..Default::default()
        })
    }
    fn face(&mut self, style: &LabelTextStyle) -> Result<Rc<ParsedFace>, String> {
        // surface_document.go nativeFontFamily resolves roles before text.go's
        // registry lookup. An unavailable configured family is distinct from
        // an unknown explicit label family, and names its selected face style.
        let family = super::surface_document::native_font_family(
            &style.font_family,
            &self.primary,
            &self.mono,
        )?;
        let key = (family.clone(), style.bold, !style.bold && style.italic);
        if let Some(face) = self.faces.get(&key) {
            return Ok(face.clone());
        }
        let font_style = if style.bold {
            FontStyle::Bold
        } else if style.italic {
            FontStyle::Italic
        } else {
            FontStyle::Regular
        };
        let data =
            d2_rs_text::d2fonts::registered_font_face(&family, font_style).ok_or_else(|| {
                format!(
                    "isometric text font {family}/{} is not loaded",
                    font_style.as_str()
                )
            })?;
        if self.faces.len() >= 24 || data.len() > MAX_TEXT_FONT_BYTES - self.font_bytes {
            return Err("isometric text exceeds font asset budget".into());
        }
        // text.go textPainter.face shares the registered bundled source;
        // only explicit custom font bytes need a new parse.
        let source = d2_rs_fontface::registered_bundled_face(&data, 0)
            .map_err(|e| format!("isometric text font: {e}"))?;
        let face = Rc::new(
            if let Some(source) = source {
                source.clone_read_only()
            } else {
                d2_rs_fontface::parse_face(&data, 0)
            }
            .map_err(|e| format!("isometric text font parse: {e}"))?,
        );
        self.font_bytes += data.len();
        self.faces.insert(key, face.clone());
        Ok(face)
    }
    fn shape(
        &mut self,
        line: &str,
        face: &ParsedFace,
        shear: bool,
        cache: &mut HashMap<String, ShapedLine>,
    ) -> Result<ShapedLine, String> {
        if let Some(v) = cache.get(line) {
            return Ok(v.clone());
        }
        let remaining = MAX_TEXT_WORK.saturating_sub(self.glyph_work);
        if remaining == 0 {
            return Err(format!(
                "isometric text glyph shaping work exceeds {MAX_TEXT_WORK}"
            ));
        }
        let text = self
            .shaper
            .shape_text_transient(
                &(),
                line,
                64 * 64,
                &[ShapeFace {
                    id: "surface",
                    face,
                }],
                ShapeLimits {
                    runes: MAX_LABEL_RUNES as i64 + 1,
                    faces: 1,
                    coverage_checks: remaining as i64,
                    runs: MAX_LABEL_RUNES as i64 + 1,
                    glyphs: remaining.min((MAX_LABEL_RUNES + 1) * 4) as i64,
                },
            )
            .map_err(|e| format!("isometric text shaping: {e}"))?;
        self.glyph_work += text.glyphs.len().max(text.coverage_checks as usize);
        let mut out = ShapedLine {
            max_x: text.advance,
            text,
            ..Default::default()
        };
        for g in &out.text.glyphs {
            if !g.has_ink {
                continue;
            }
            for x in [g.ink.min.x as f64 / 64.0, g.ink.max.x as f64 / 64.0] {
                for y in [g.ink.min.y as f64 / 64.0, g.ink.max.y as f64 / 64.0] {
                    let (mut x, y) = (x + g.position_x, y + g.position_y);
                    if shear {
                        x -= y * 0.20;
                    }
                    out.min_x = go_min(out.min_x, x);
                    out.max_x = go_max(out.max_x, x);
                    out.min_y = go_min(out.min_y, y);
                    out.max_y = go_max(out.max_y, y);
                }
            }
        }
        cache.insert(line.into(), out.clone());
        Ok(out)
    }
    pub fn texture(
        &mut self,
        value: &str,
        style: &LabelTextStyle,
    ) -> Result<(Option<Rc<Texture>>, TextLayout), String> {
        self.context.check_string()?;
        if self.used >= self.count {
            return Err("isometric text label allocation exceeds its declared count".into());
        }
        if [style.width, style.depth, style.pixel_scale]
            .iter()
            .any(|v| !v.is_finite() || *v <= 0.0)
        {
            return Err("isometric text has an invalid physical print area or pixel scale".into());
        }
        if !style.font_size.is_finite()
            || style.font_size < 0.0
            || !style.opacity.is_finite()
            || !(0.0..=1.0).contains(&style.opacity)
        {
            return Err("isometric text has invalid font size or opacity".into());
        }
        if value.chars().count() > MAX_LABEL_RUNES
            || value.len() > MAX_TEXT_SOURCE_BYTES - self.source_bytes
        {
            return Err("isometric text exceeds source budget or contains invalid UTF-8".into());
        }
        self.source_bytes += value.len();
        let face = self.face(style)?;
        let budget = MAX_TEXT_PIXELS / self.count.max(1);
        let (w, h) = surface_texture_dimensions_at_density(
            style.width,
            style.depth,
            4096,
            budget,
            self.output_density,
        );
        let (lw, lh) =
            surface_texture_dimensions_at_density(style.width, style.depth, 4096, budget, 0.0);
        if w * h > MAX_TEXT_PIXELS - self.pixels {
            return Err(format!(
                "isometric text exceeds its {MAX_TEXT_PIXELS} pixel texture budget"
            ));
        }
        self.used += 1;
        self.pixels += w * h;
        for c in value.chars() {
            if matches!(c, '\n' | '\r' | '\t') || d2_rs_fontface::is_default_ignorable_rune(c) {
                continue;
            }
            if c as u32 > 255
                || face.is_bundled_noto_color_emoji()
                || !face
                    .supports_renderable_rune(c)
                    .map_err(|e| e.to_string())?
            {
                let family = super::surface_document::native_font_family(
                    &style.font_family,
                    &self.primary,
                    &self.mono,
                )?;
                return super::surface_document::fallback_text_texture(
                    value,
                    style,
                    w,
                    h,
                    &family,
                    &self.mono,
                    self.fallback_fonts.as_ref(),
                    &self.vectors,
                );
            }
        }
        let shear = style.bold && style.italic;
        let mut cache = HashMap::new();
        let scale = lw as f64 / style.width;
        let preferred = if style.font_size > 0.0 {
            style.font_size * style.pixel_scale * scale
        } else {
            0.235 * scale
        };
        let inset = go_min(2.0, go_min(lw as f64, lh as f64) * 0.05);
        let mut layout = fit_native_label(
            value,
            lw as f64 - 2.0 * inset,
            lh as f64 - 2.0 * inset,
            preferred,
            |line| {
                let shape = self.shape(line, &face, shear, &mut cache)?;
                Ok((shape.max_x - shape.min_x) / 64.0)
            },
        )?;
        let font_pixels = layout.font_size;
        let unit_scale = font_pixels / 64.0;
        let mut min_y = f64::INFINITY;
        let mut max_y = f64::NEG_INFINITY;
        let mut lines = Vec::new();
        for (i, line) in layout.lines.iter().enumerate() {
            let shape = self.shape(line, &face, shear, &mut cache)?;
            min_y = go_min(
                min_y,
                i as f64 * layout.line_height + shape.min_y * unit_scale,
            );
            max_y = go_max(
                max_y,
                i as f64 * layout.line_height + shape.max_y * unit_scale,
            );
            lines.push(shape);
        }
        let baseline = (lh as f64 - (max_y - min_y)) / 2.0 - min_y;
        let (sx, sy) = (w as f64 / lw as f64, h as f64 / lh as f64);
        let mut outline = Path::default();
        // Go's explicit float64 products prevent contraction before translation.
        for (i, line) in lines.iter().enumerate() {
            let mut x = (lw as f64 - (line.max_x - line.min_x) * unit_scale) / 2.0
                - line.min_x * unit_scale;
            if style.align == "left" {
                x = inset - line.min_x * unit_scale;
            }
            let y = baseline + i as f64 * layout.line_height;
            for glyph in &line.text.glyphs {
                if glyph.empty || !glyph.has_ink {
                    continue;
                }
                let segments = face
                    .outline()
                    .load_glyph(glyph.id as u16, 64 * 64)
                    .map_err(|e| format!("isometric text glyph outline: {e}"))?;
                self.outline_work += segments.len();
                if self.outline_work > MAX_TEXT_WORK {
                    return Err(format!(
                        "isometric text outline work exceeds {MAX_TEXT_WORK}"
                    ));
                }
                let transform = |p: d2_rs_fontface::fixed::Point26_6| {
                    let (mut gx, gy) = (
                        p.x as f64 / 64.0 + glyph.position_x,
                        p.y as f64 / 64.0 + glyph.position_y,
                    );
                    if shear {
                        gx -= gy * 0.20;
                    }
                    (
                        ((x + gx * unit_scale) * sx) as f32 as f64,
                        ((y + gy * unit_scale) * sy) as f32 as f64,
                    )
                };
                let mut open = false;
                for s in segments {
                    let (a, b, c) = (
                        transform(s.args[0]),
                        transform(s.args[1]),
                        transform(s.args[2]),
                    );
                    let command = match s.op {
                        SegmentOp::MoveTo => {
                            if open {
                                outline.commands.push(PathCommand::close());
                            }
                            open = true;
                            PathCommand::move_to(a.0, a.1)
                        }
                        SegmentOp::LineTo => PathCommand::line_to(a.0, a.1),
                        SegmentOp::QuadTo => PathCommand::quadratic_to(a.0, a.1, b.0, b.1),
                        SegmentOp::CubeTo => PathCommand::cubic_to(a.0, a.1, b.0, b.1, c.0, c.1),
                    };
                    outline.commands.push(command);
                }
                if open {
                    outline.commands.push(PathCommand::close());
                }
            }
            if style.underline && line.max_x > line.min_x {
                let (left, right) = (
                    (x + line.min_x * unit_scale) * sx,
                    (x + line.max_x * unit_scale) * sx,
                );
                let (top, bottom) = ((y + font_pixels * 0.10) * sy, (y + font_pixels * 0.15) * sy);
                outline.commands.extend([
                    PathCommand::move_to(left, top),
                    PathCommand::line_to(right, top),
                    PathCommand::line_to(right, bottom),
                    PathCommand::line_to(left, bottom),
                    PathCommand::close(),
                ]);
            }
        }
        let fade = |mut c: Nrgba| {
            c.a = (c.a as f64 * style.opacity).round() as u8;
            c
        };
        let foreground = fade(style.color);
        let background = style.background.map(fade);
        let mut pixels = super::text_raster::draw(&outline, w, h, foreground, background);
        outline.fill = Some(Paint::SolidPaint { color: foreground });
        let mut root = Node::new(None);
        let bx = SceneBox {
            width: w as f64,
            height: h as f64,
            ..Default::default()
        };
        let mut children = vec![];
        if let Some(background) = background {
            children.push(Node::new(Some(Primitive::Rect(Rect {
                r#box: bx,
                fill: Some(Paint::SolidPaint { color: background }),
                ..Default::default()
            }))));
        }
        if style.foreground_paint.is_some() || style.background_paint.is_some() {
            children.clear();
            root.opacity = style.opacity;
            if let Some(paint) = style
                .background_paint
                .clone()
                .or_else(|| style.background.map(|color| Paint::SolidPaint { color }))
            {
                children.push(Node::new(Some(Primitive::Rect(Rect {
                    r#box: bx,
                    fill: Some(paint),
                    ..Default::default()
                }))));
            }
            outline.fill = Some(
                style
                    .foreground_paint
                    .clone()
                    .unwrap_or(Paint::SolidPaint { color: style.color }),
            );
        }
        children.push(Node::new(Some(Primitive::Path(outline))));
        root.children = Some(children);
        let document = Document::new(bx, root);
        if style.foreground_paint.is_some() || style.background_paint.is_some() {
            pixels = super::backends::raster_surface(&document, w, h)?;
        }
        let texture = Rc::new(Texture::new(w, h, pixels));
        retain_native_vector_surface(&self.vectors, &texture, &document)?;
        layout.font_size /= scale;
        layout.line_height /= scale;
        Ok((Some(texture), layout))
    }
}
pub fn normalize_label(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}
pub fn fit_native_label(
    text: &str,
    width: f64,
    height: f64,
    preferred: f64,
    mut measure: impl FnMut(&str) -> Result<f64, String>,
) -> Result<TextLayout, String> {
    let (width, height) = (go_max(width, 1e-9), go_max(height, 1e-9));
    let lines: Vec<_> = normalize_label(text)
        .split('\n')
        .map(str::to_owned)
        .collect();
    let mut size = go_min(preferred, height / (lines.len() as f64 * 1.18));
    for line in &lines {
        let w = measure(line)?;
        if w > 0.0 {
            size = go_min(size, width / w);
        }
    }
    Ok(TextLayout {
        lines,
        font_size: size,
        line_height: size * 1.18,
        truncated: false,
    })
}
pub fn surface_texture_dimensions_at_density(
    width: f64,
    depth: f64,
    maximum: usize,
    budget: usize,
    density: f64,
) -> (usize, usize) {
    let long_side = go_max(width, depth);
    let (x, z) = (width / long_side, depth / long_side);
    let mut long = go_min(maximum as f64, (budget as f64 / (x * z)).sqrt());
    if density > 0.0 && density.is_finite() {
        long = go_min(long, long_side * density * 1.5);
    }
    let (mut w, mut h) = (
        ((long * x).floor() as usize).max(1),
        ((long * z).floor() as usize).max(1),
    );
    if w * h > budget {
        if w >= h {
            w = (budget / h).max(1);
        } else {
            h = (budget / w).max(1);
        }
    }
    (w, h)
}

#[cfg(test)]
#[path = "text_api_tests.rs"]
pub(crate) mod api_tests;

#[cfg(test)]
#[path = "font_api_tests.rs"]
mod font_api_tests;
