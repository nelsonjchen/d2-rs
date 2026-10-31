// SPDX-License-Identifier: MPL-2.0
//! d2isometricimg/surface_document.go and surface_fonts.go shared surface
//! adapters. All rendering and shaping execute in native foundation crates.
use super::text::LabelTextStyle;
use d2_rs_scene::{Asset, Document, Node, Primitive};
use d2_rs_scenebuild::{AssetOptions, FontFallbackOptions, SvgImportBudget, SvgImportLimits};
use std::{collections::HashMap, rc::Rc};

pub fn native_font_family(name: &str, primary: &str, mono: &str) -> Result<String, String> {
    let primary = if primary.is_empty() {
        "SourceSansPro"
    } else {
        primary
    };
    let mono = if mono.is_empty() {
        "SourceCodePro"
    } else {
        mono
    };
    let family = match name.trim().to_lowercase().as_str() {
        "" | "default" => primary,
        "mono" => mono,
        "sourcesanspro" => "SourceSansPro",
        "sourcecodepro" => "SourceCodePro",
        "handdrawn" | "fuzzybubbles" => "HandDrawn",
        _ if d2_rs_text::d2fonts::registered_font_face(
            name,
            d2_rs_text::d2fonts::FontStyle::Regular,
        )
        .is_some() =>
        {
            name
        }
        _ => {
            return Err(format!(
                "isometric text font family {name:?} is not registered"
            ));
        }
    };
    Ok(family.into())
}
pub fn surface_fonts(
    supplied: Option<&FontFallbackOptions>,
) -> Result<FontFallbackOptions, String> {
    let mut options = FontFallbackOptions {
        max_assets: 96,
        max_bytes: 64 << 20,
        max_runes_per_text: 65536,
        max_total_runes: 8 << 20,
        max_coverage_checks: 8000000,
        max_shaped_glyphs: 1000000,
        max_shaping_runs: 200000,
        max_font_faces_per_text: 8,
        ..Default::default()
    };
    if let Some(s) = supplied {
        options.resolver = s.resolver;
        options.resolver_impl = s.resolver_impl.clone();
        let limit = |v: &mut i64, n| {
            if n > 0 {
                *v = (*v).min(n);
            }
        };
        limit(&mut options.max_assets, s.max_assets);
        limit(&mut options.max_bytes, s.max_bytes);
        limit(&mut options.max_runes_per_text, s.max_runes_per_text);
        limit(&mut options.max_total_runes, s.max_total_runes);
        limit(&mut options.max_coverage_checks, s.max_coverage_checks);
        limit(&mut options.max_shaped_glyphs, s.max_shaped_glyphs);
        limit(&mut options.max_shaping_runs, s.max_shaping_runs);
        limit(
            &mut options.max_font_faces_per_text,
            s.max_font_faces_per_text,
        );
    }
    if options.resolver_impl.is_none() && !options.resolver {
        options.resolver_impl = Some(std::sync::Arc::new(
            d2_rs_scenebuild::BundledFallbackResolver::new(
                None,
                d2_rs_scenebuild::BundledFallbackLimits {
                    max_requested_runes: options.max_total_runes,
                    max_bundled_bytes: 0,
                    max_resolved_bytes: options.max_bytes,
                },
            )?,
        ));
    }
    Ok(options)
}
pub fn surface_assets() -> AssetOptions {
    AssetOptions {
        svg_import_limits: SvgImportLimits {
            max_bytes: 2 << 20,
            max_depth: 64,
            max_elements: 20000,
            max_attributes: 80000,
            max_attribute_bytes: 2 << 20,
            max_path_commands: 100000,
            max_transform_functions: 40000,
            max_use_depth: 32,
            max_resources: 4096,
        },
        svg_import_budget: SvgImportBudget {
            max_source_bytes: 16 << 20,
            max_elements: 100000,
            max_attributes: 400000,
            max_attribute_bytes: 8 << 20,
            max_path_commands: 500000,
            max_transform_functions: 200000,
            max_declared_resources: 16000,
            max_expanded_use_instances: 16000,
        },
        ..Default::default()
    }
}
/// Keep missing source glyphs explicit, rather than silently emitting tofu.
pub fn document_glyph_coverage(doc: &Document) -> Result<(), String> {
    struct Coverage<'a> {
        doc: &'a Document,
        faces: HashMap<String, Rc<d2_rs_fontface::ParsedFace>>,
        cache: HashMap<(String, char), bool>,
        checks: usize,
    }
    impl Coverage<'_> {
        fn supports(&mut self, id: &str, c: char) -> Result<bool, String> {
            let key = (id.to_owned(), c);
            if let Some(v) = self.cache.get(&key) {
                return Ok(*v);
            }
            self.checks += 1;
            if self.checks > 8000000 {
                return Err("native surface glyph coverage exceeds budget".into());
            }
            if !self.faces.contains_key(id) {
                let Some(Asset::FontAsset {
                    data, face_index, ..
                }) = self.doc.assets.get(id)
                else {
                    return Err(format!("native surface font asset {id:?} is unavailable"));
                };
                let source = d2_rs_fontface::registered_bundled_face(data, *face_index)
                    .map_err(|e| e.to_string())?
                    .or(
                        d2_rs_fontface::registered_bundled_noto_color_emoji(data, *face_index)
                            .map_err(|e| e.to_string())?,
                    );
                let face = if let Some(source) = source {
                    source.clone_read_only()
                } else {
                    d2_rs_fontface::parse_face(data, *face_index)
                }
                .map_err(|e| e.to_string())?;
                self.faces.insert(id.into(), Rc::new(face));
            }
            let value = self.faces[id]
                .supports_renderable_rune(c)
                .map_err(|e| e.to_string())?;
            self.cache.insert(key, value);
            Ok(value)
        }
        fn walk(&mut self, node: &Node) -> Result<(), String> {
            if let Some(Primitive::TextRun(run)) = &node.primitive {
                for c in run.text.chars() {
                    if matches!(c, '\n' | '\r' | '\t')
                        || d2_rs_fontface::is_default_ignorable_rune(c)
                    {
                        continue;
                    }
                    let mut found = self.supports(&run.font.asset, c)?;
                    for id in run.fallbacks.iter().flatten() {
                        if found {
                            break;
                        }
                        found = self.supports(id, c)?;
                    }
                    if !found {
                        return Err(format!(
                            "native surface label {:?} has no available font for U+{:04X}; configure a font or fallback resolver covering this character",
                            node.id, c as u32
                        ));
                    }
                }
            }
            for child in node.children.iter().flatten() {
                self.walk(child)?;
            }
            Ok(())
        }
    }
    let mut coverage = Coverage {
        doc,
        faces: HashMap::new(),
        cache: HashMap::new(),
        checks: 0,
    };
    if let Some(root) = &doc.root {
        coverage.walk(root)?;
    }
    for asset in doc.assets.values() {
        if let Asset::VectorAsset {
            root: Some(root), ..
        } = asset
        {
            coverage.walk(root)?;
        }
    }
    Ok(())
}
/// draw.Draw NRGBA -> RGBA, Over background, followed by one opacity fade.
pub fn compose_surface(
    frame: &[u8],
    style: &LabelTextStyle,
    w: usize,
    h: usize,
) -> Result<Vec<u8>, String> {
    compose(frame, style, w, h, false)
}
pub fn compose_premultiplied_surface(
    frame: &[u8],
    style: &LabelTextStyle,
    w: usize,
    h: usize,
) -> Result<Vec<u8>, String> {
    compose(frame, style, w, h, true)
}
fn compose(
    frame: &[u8],
    style: &LabelTextStyle,
    w: usize,
    h: usize,
    premultiplied: bool,
) -> Result<Vec<u8>, String> {
    let mut pixels = if let Some(paint) = &style.background_paint {
        let bx = d2_rs_scene::Box {
            width: w as f64,
            height: h as f64,
            ..Default::default()
        };
        let document = Document::new(
            bx,
            Node::new(Some(Primitive::Rect(d2_rs_scene::Rect {
                r#box: bx,
                fill: Some(paint.clone()),
                ..Default::default()
            }))),
        );
        super::backends::raster_surface(&document, w, h)?
    } else {
        let c = style.background.unwrap_or_default();
        let a = c.a as u32 * 257;
        let pixel = [
            (c.r as u32 * 257 * a / 65535 >> 8) as u8,
            (c.g as u32 * 257 * a / 65535 >> 8) as u8,
            (c.b as u32 * 257 * a / 65535 >> 8) as u8,
            c.a,
        ];
        pixel.repeat(w * h)
    };
    for (src, dst) in frame.chunks_exact(4).zip(pixels.chunks_exact_mut(4)) {
        let alpha = src[3] as u32 * 257;
        let inverse = 65535 - alpha;
        for i in 0..3 {
            let premul = if premultiplied {
                src[i] as u32 * 257
            } else {
                src[i] as u32 * 257 * alpha / 65535
            };
            dst[i] = ((dst[i] as u32 * 257 * inverse / 65535 + premul) >> 8) as u8;
        }
        dst[3] = ((dst[3] as u32 * 257 * inverse / 65535 + alpha) >> 8) as u8;
        for value in dst {
            *value = (*value as f64 * style.opacity).round() as u8;
        }
    }
    Ok(pixels)
}
pub fn readable_surface_ink(fill: &str) -> &'static str {
    use d2_rs_render::go_float::{go_fmadd, gomath};
    let parsed = d2_rs_math::svg_color::parse(fill).unwrap_or([
        237.0 / 255.0,
        241.0 / 255.0,
        247.0 / 255.0,
        1.0,
    ]);
    let byte = |v: f64| go_fmadd(v, 255.0, 0.5) as i32 as u8;
    let mut c = d2_rs_scene::Nrgba::new(
        byte(parsed[0]),
        byte(parsed[1]),
        byte(parsed[2]),
        byte(parsed[3]),
    );
    if let Ok(paint) = d2_rs_scenebuild::gradient_paint(fill) {
        let stops = match paint {
            d2_rs_scene::Paint::LinearGradient(p) => p.stops,
            d2_rs_scene::Paint::RadialGradient(p) => p.stops,
            _ => vec![],
        };
        if !stops.is_empty() {
            let count = stops.len() as f64;
            let mean = |channel: fn(d2_rs_scene::Nrgba) -> u8| {
                (stops.iter().map(|s| channel(s.color) as f64).sum::<f64>() / count).round() as u8
            };
            c = d2_rs_scene::Nrgba::new(mean(|c| c.r), mean(|c| c.g), mean(|c| c.b), mean(|c| c.a));
            if !stops.iter().all(|s| s.color.a == 255) && c.a == 255 {
                c.a = 254;
            }
            if !stops.iter().all(|s| s.color.a == 0) && c.a == 0 {
                c.a = 1;
            }
        }
    }
    let alpha = c.a as f64 / 255.0;
    let mut luminance = 0.0;
    for (i, value) in [c.r, c.g, c.b].into_iter().enumerate() {
        // geometry.go:383/389: pinned ARM64's second product and running sum.
        // Opacity is exactly one here, so its product has no rounding error.
        let v = go_fmadd(247.0, 1.0 - alpha, value as f64 * alpha) / 255.0;
        let v = if v <= 0.04045 {
            v / 12.92
        } else {
            gomath::pow((v + 0.055) / 1.055, 2.4)
        };
        luminance = go_fmadd(v, [0.2126, 0.7152, 0.0722][i], luminance);
    }
    if luminance < 0.24 {
        "#f5f8fc"
    } else {
        "#253650"
    }
}

/// surface_fonts.go nativeFallbackTextTexture: preserve source ink bounds,
/// grapheme-aware shaping and the two RGBA composition steps.
pub fn fallback_text_texture(
    value: &str,
    style: &LabelTextStyle,
    width: usize,
    height: usize,
    primary: &str,
    mono: &str,
    fonts: Option<&FontFallbackOptions>,
    vectors: &super::vector_surface::VectorCtx,
) -> Result<(Option<Rc<super::mesh::Texture>>, super::text::TextLayout), String> {
    use crate::target::{Diagram, Shape, Text};
    use d2_rs_scene::{Box as SceneBox, Matrix, TextAnchor, ViewportAlign, ViewportFit};
    let primary = if primary.is_empty() {
        "SourceSansPro"
    } else {
        primary
    };
    let mono = if mono.is_empty() {
        "SourceCodePro"
    } else {
        mono
    };
    let normalized = super::text::normalize_label(value);
    let lines: Vec<String> = normalized.split('\n').map(str::to_owned).collect();
    let font_size = if style.font_size <= 0.0 {
        23.5
    } else {
        style.font_size
    };
    if font_size > 100000.0 {
        return Err("native fallback text font size exceeds limit".into());
    }
    let w = (style.width / style.pixel_scale)
        .ceil()
        .min(100000.0)
        .max(1.0) as i64;
    let h = (lines.len() as f64 * font_size * 1.2)
        .ceil()
        .min(100000.0)
        .max(1.0) as i64;
    let s = Shape {
        id: "surface-text".into(),
        type_: "text".into(),
        width: w,
        height: h,
        fill: "transparent".into(),
        stroke: "none".into(),
        opacity: 1.0,
        label_position: "INSIDE_MIDDLE_CENTER".into(),
        text: Text {
            label: normalized,
            font_size: (font_size.round() as i64).max(1),
            label_width: w,
            label_height: h,
            font_family: "default".into(),
            color: if style.foreground_paint.is_some() {
                style.foreground.clone()
            } else {
                super::rich_labels::rich_color(style.color)
            },
            bold: style.bold,
            italic: style.italic,
            underline: style.underline,
            ..Default::default()
        },
        ..Default::default()
    };
    let diagram = Diagram {
        root: Shape {
            fill: "transparent".into(),
            stroke: "none".into(),
            ..Default::default()
        },
        font_family: Some(primary.into()),
        mono_font_family: Some(mono.into()),
        shapes: Some(vec![s]),
        ..Default::default()
    };
    let mut document = d2_rs_scenebuild::build(
        &diagram,
        &d2_rs_scenebuild::Options {
            pad: Some(0),
            max_nodes: 200000,
            max_path_commands: 2000000,
            assets: Some(surface_assets()),
            link_budget: d2_rs_scenebuild::LinkBudget {
                max_regions: 20000,
                max_string_bytes: 8 << 20,
            },
            fonts: Some(surface_fonts(fonts)?),
            ..Default::default()
        },
    )
    .map_err(|e| e.to_string())?;
    document_glyph_coverage(&document)?;
    let (mut max_w, mut min_y, mut max_y) = (0.0f64, f64::INFINITY, f64::NEG_INFINITY);
    fn walk(
        n: &mut Node,
        style: &LabelTextStyle,
        font_size: f64,
        max_w: &mut f64,
        min_y: &mut f64,
        max_y: &mut f64,
    ) {
        if let Some(Primitive::TextRun(run)) = &mut n.primitive {
            let (mut lo, mut hi) = (0.0f64, 0.0f64);
            if run.ink.valid {
                lo = run.ink.min.x;
                hi = run.ink.max.x;
                *min_y = (*min_y).min(run.origin.y + run.ink.min.y);
                *max_y = (*max_y).max(run.origin.y + run.ink.max.y);
            }
            for g in run.glyphs.iter().flatten() {
                hi = hi.max(g.position.x + g.advance);
            }
            if style.bold && style.italic {
                n.transform = n.transform.mul(Matrix {
                    a: 1.0,
                    c: -0.20,
                    d: 1.0,
                    e: 0.20 * run.origin.y,
                    ..Default::default()
                });
                // Pinned ARM64 surface_fonts.go:248–249 emits one FMSUBD
                // and one FMADDD for the synthetic bold/italic ink bounds.
                lo = d2_rs_render::go_float::go_fmsub(font_size, 0.20, lo);
                hi = d2_rs_render::go_float::go_fmadd(font_size, 0.20, hi);
            }
            *max_w = (*max_w).max(hi - lo);
            run.origin.x = -(lo + hi) / 2.0;
            run.anchor = TextAnchor::START;
            if style.align == "left" {
                run.origin.x = 1.0 - lo;
            }
        }
        for child in n.children.iter_mut().flatten() {
            walk(child, style, font_size, max_w, min_y, max_y);
        }
    }
    if let Some(root) = &mut document.root {
        walk(root, style, font_size, &mut max_w, &mut min_y, &mut max_y);
    }
    if !min_y.is_finite() || !max_y.is_finite() {
        min_y = 0.0;
        max_y = h as f64;
    }
    let mut bx = SceneBox {
        x: -(max_w + 2.0) / 2.0,
        y: 0.0f64.min(min_y - 1.0),
        width: 1.0f64.max(max_w + 2.0),
        height: (h as f64).max(max_y + 1.0) - 0.0f64.min(min_y - 1.0),
    };
    if style.align == "left" {
        bx.x = 0.0;
    }
    document.view_box = bx;
    let fit = 1.0f64.min(
        (style.width / style.pixel_scale / bx.width)
            .min(style.depth / style.pixel_scale / bx.height),
    );
    super::rich_labels::fit_rich_viewport(&mut document, style)?;
    let frame = super::backends::raster_surface(&document, width, height)?;
    let pixels = compose_premultiplied_surface(&frame, style, width, height)?;
    let texture = Rc::new(super::mesh::Texture::new(width, height, pixels));
    document.logical_width = width as f64;
    document.logical_height = height as f64;
    document.viewport_fit = ViewportFit::MEET;
    document.viewport_align = ViewportAlign::X_MID_Y_MID;
    let backing = style
        .background_paint
        .clone()
        .or_else(|| style.background.map(d2_rs_scene::Paint::solid));
    super::vector_surface::retain_styled_native_vector_surface(
        vectors,
        &texture,
        &document,
        backing.as_ref(),
        style.opacity,
    )?;
    Ok((
        Some(texture),
        super::text::TextLayout {
            lines,
            font_size: font_size * style.pixel_scale * fit,
            line_height: font_size * style.pixel_scale * fit * 1.2,
            ..Default::default()
        },
    ))
}

/// Integer-boundary adapter for the previously translated mesh builder.
pub fn surface_texture_dimensions(w: f64, h: f64, maximum: i64, budget: i64) -> (i64, i64) {
    surface_texture_dimensions_at_density(w, h, maximum, budget, 0.0)
}
pub fn surface_texture_dimensions_at_density(
    w: f64,
    h: f64,
    maximum: i64,
    budget: i64,
    density: f64,
) -> (i64, i64) {
    let (w, h) = super::text::surface_texture_dimensions_at_density(
        w,
        h,
        maximum as usize,
        budget as usize,
        density,
    );
    (w as i64, h as i64)
}
