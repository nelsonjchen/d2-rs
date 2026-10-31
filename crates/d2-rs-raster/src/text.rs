// SPDX-License-Identifier: MPL-2.0
//! d2raster/text.go: explicit glyph placement, glyph outline flattening,
//! underline/strike decorations and the text primitive.
//!
//! PreparedFont shares authenticated bundled outlines and lazily creates
//! render-local shaping faces. Color plans are immutable and cached locally.

use std::cell::{Ref, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use d2_rs_fontface::sfnt::{Font, Hinting, SegmentOp};
use d2_rs_fontface::{
    self as fontface, BundledFaceSource, COLRv1Plan, ColorGlyphLayer, ParsedFace, ShapeFace,
    ShapeLimits,
};
use d2_rs_scene::{Bounds, Nrgba, Point, TextAnchor, TextRun, go_max};

use crate::fma::{finite, finite_point};
use crate::geometry::{
    GEOMETRY_EPSILON, Subpath, flatten_cubic, flatten_quadratic, flatten_tolerance,
    local_object_bounds,
};
use crate::paint::PreparedPaint;
use crate::pixels::union_rect;
use crate::raster::{Preflight, PreparedNode, PreparedPrimitive};

/// preparedFont: the immutable outline reader for one font asset.
#[derive(Debug)]
pub struct PreparedFont {
    pub(crate) outline: Arc<Font>,
    source: Option<Arc<BundledFaceSource>>,
    face: RefCell<Option<ParsedFace>>,
    colrv1_plans: RefCell<HashMap<u32, Result<Option<Rc<COLRv1Plan>>, String>>>,
}

impl PreparedFont {
    /// parsePreparedFont.
    pub fn parse(data: &[u8], face_index: u16) -> Result<Self, String> {
        let source = fontface::registered_bundled_face(data, face_index)
            .and_then(|found| match found {
                Some(source) => Ok(Some(source)),
                None => fontface::registered_bundled_noto_color_emoji(data, face_index),
            })
            .map_err(|e| e.to_string())?;
        let (outline, face) = if let Some(source) = &source {
            (source.outline().map_err(|e| e.to_string())?.clone(), None)
        } else {
            let face = fontface::parse_face(data, face_index).map_err(|e| e.to_string())?;
            (face.outline_arc().clone(), Some(face))
        };
        Ok(Self {
            outline,
            source,
            face: RefCell::new(face),
            colrv1_plans: RefCell::new(HashMap::new()),
        })
    }

    pub(crate) fn num_glyphs(&self) -> usize {
        self.outline.num_glyphs()
    }

    fn face_for_shaping(&self) -> Result<Ref<'_, ParsedFace>, String> {
        if self.face.borrow().is_none() {
            let source = self
                .source
                .as_ref()
                .ok_or("d2raster: prepared font has no face source")?;
            *self.face.borrow_mut() = Some(source.clone_read_only().map_err(|e| e.to_string())?);
        }
        Ok(Ref::map(self.face.borrow(), |face| face.as_ref().unwrap()))
    }

    fn glyph_data_kind(&self, glyph_id: u32) -> &'static str {
        if let Some(source) = &self.source {
            source.glyph_data_kind(glyph_id).unwrap_or("missing")
        } else {
            self.face
                .borrow()
                .as_ref()
                .map_or("missing", |face| face.shaping().glyph_data_kind(glyph_id))
        }
    }

    /// bundledCOLRv1Plan: only the bundled Noto Color Emoji has plans.
    fn bundled_colrv1_plan(&self, glyph_id: u32) -> Result<Option<Rc<COLRv1Plan>>, String> {
        let bundled = if let Some(source) = &self.source {
            source.is_bundled_noto_color_emoji()
        } else {
            self.face
                .borrow()
                .as_ref()
                .is_some_and(|f| f.is_bundled_noto_color_emoji())
        };
        if !bundled {
            return Ok(None);
        }
        if let Some(cached) = self.colrv1_plans.borrow().get(&glyph_id) {
            return cached.clone();
        }
        let plan = if let Some(source) = &self.source {
            source.compile_bundled_noto_color_emoji_colrv1_plan(glyph_id)
        } else {
            self.face
                .borrow()
                .as_ref()
                .unwrap()
                .compile_bundled_noto_color_emoji_colrv1_plan(glyph_id)
        }
        .map(|plan| plan.map(Rc::new))
        .map_err(|e| e.to_string());
        self.colrv1_plans
            .borrow_mut()
            .insert(glyph_id, plan.clone());
        plan
    }

    /// colr0GlyphLayers: whether this glyph is a COLRv0 color glyph.
    fn colr0_glyph_layers(&self, glyph_id: u32) -> Result<Option<Vec<ColorGlyphLayer>>, String> {
        if let Some(source) = &self.source {
            source.colr0_glyph_layers(glyph_id)
        } else {
            self.face
                .borrow()
                .as_ref()
                .unwrap()
                .colr0_glyph_layers(glyph_id)
        }
        .map_err(|e| e.to_string())
    }

    /// glyphRenderBounds: the loaded outline's 26.6 bounds (min x, min y,
    /// max x, max y).
    pub fn glyph_render_bounds(
        &self,
        glyph_id: u32,
        ppem: i32,
    ) -> Result<(i32, i32, i32, i32), String> {
        let (bounds, _) = if let Some(source) = &self.source {
            source.glyph_render_bounds(glyph_id, ppem)
        } else {
            self.face
                .borrow()
                .as_ref()
                .unwrap()
                .glyph_render_bounds(glyph_id, ppem)
        }
        .map_err(|e| e.to_string())?;
        Ok((bounds.min.x, bounds.min.y, bounds.max.x, bounds.max.y))
    }
}

/// positionedGlyph.
#[derive(Clone, Debug)]
pub struct PositionedGlyph {
    pub id: u32,
    pub font: Rc<PreparedFont>,
    pub asset: String,
    pub position: Point,
    pub empty: bool,
    source: u32,
    source_at: i64,
}

fn unsupported_positioned_glyph_data_error(glyph: &PositionedGlyph, cause: &str) -> String {
    let kind = glyph.font.glyph_data_kind(glyph.id);
    if glyph.source != 0 {
        return format!(
            "{kind} glyph U+{:04X} at rune {} in font asset {:?} cannot be rasterized: {cause}",
            glyph.source, glyph.source_at, glyph.asset
        );
    }
    format!(
        "{kind} glyph at cluster {} in font asset {:?} cannot be rasterized: {cause}",
        glyph.source_at, glyph.asset
    )
}

#[derive(Default)]
struct PreparedTextPart {
    paths: Vec<Subpath>,
    vector: Option<PreparedNode>,
    user_paint: bool,
    foreground: bool,
    color: Nrgba,
}

impl<'d> Preflight<'d> {
    fn flush_user_paths(
        &mut self,
        node_id: &str,
        paths: &mut Vec<Subpath>,
        parts: &mut Vec<PreparedTextPart>,
    ) -> Result<(), String> {
        if paths.is_empty() {
            return Ok(());
        }
        self.add_prepared_nodes(1)
            .map_err(|e| format!("d2raster: node {node_id:?} color text: {e}"))?;
        parts.push(PreparedTextPart {
            paths: std::mem::take(paths),
            user_paint: true,
            ..Default::default()
        });
        Ok(())
    }

    pub(crate) fn text(
        &mut self,
        node_id: &str,
        text: &'d TextRun,
        transform: &d2_rs_scene::Matrix,
        animation: &crate::raster::AnimationOverrides,
        import_depth: i64,
    ) -> Result<PreparedPrimitive, String> {
        let size = text.font.size();
        if !finite_point(text.origin)
            || !finite(size)
            || size <= 0.0
            || size > i32::MAX as f64 / 64.0
        {
            return Err(format!(
                "d2raster: node {node_id:?} text has invalid origin or font size"
            ));
        }
        if text.anchor.0 > TextAnchor::END.0 {
            return Err(format!(
                "d2raster: node {node_id:?} text has invalid anchor {}",
                text.anchor.0
            ));
        }
        if !text.ink.is_finite() {
            return Err(format!(
                "d2raster: node {node_id:?} text has invalid ink bounds"
            ));
        }
        if text.font.asset.is_empty() {
            return Err(format!(
                "d2raster: node {node_id:?} text has empty font asset ID"
            ));
        }
        let Some(primary) = self.fonts.get(&text.font.asset).cloned() else {
            if self.document.assets.contains_key(&text.font.asset) {
                return Err(format!(
                    "d2raster: node {node_id:?} text asset {:?} is not a usable font",
                    text.font.asset
                ));
            }
            return Err(format!(
                "d2raster: node {node_id:?} text references missing font asset {:?}",
                text.font.asset
            ));
        };
        let ppem = (size * 64.0).round() as i32;
        let (glyphs, advance) = self
            .position_glyphs(text, ppem)
            .map_err(|e| format!("d2raster: node {node_id:?} text: {e}"))?;
        let anchor_offset = match text.anchor {
            TextAnchor::MIDDLE => -advance / 2.0,
            TextAnchor::END => -advance,
            _ => 0.0,
        };
        let tolerance = flatten_tolerance(transform);
        let mut paths: Vec<Subpath> = Vec::new();
        let mut parts = Vec::new();
        let mut all_paths = Vec::new();
        let mut color_local_bounds = Bounds::default();
        let mut has_color_glyph = false;
        for (index, glyph) in glyphs.iter().enumerate() {
            if glyph.empty {
                continue;
            }
            let origin = Point::new(
                text.origin.x + anchor_offset + glyph.position.x,
                text.origin.y + glyph.position.y,
            );
            let plan = glyph
                .font
                .bundled_colrv1_plan(glyph.id)
                .map_err(|e| unsupported_positioned_glyph_data_error(glyph, &e))?;
            if let Some(plan) = plan {
                if !has_color_glyph {
                    self.add_prepared_nodes(1)
                        .map_err(|e| format!("d2raster: node {node_id:?} color text root: {e}"))?;
                    all_paths = paths.clone();
                    has_color_glyph = true;
                }
                self.flush_user_paths(node_id, &mut paths, &mut parts)?;
                let (vector, local_bounds) = self
                    .colrv1_text_glyph(node_id, &glyph.font, &plan, origin, ppem, transform)
                    .map_err(|e| unsupported_positioned_glyph_data_error(glyph, &e))?;
                if let Some(vector) = vector {
                    parts.push(PreparedTextPart {
                        vector: Some(vector),
                        ..Default::default()
                    });
                }
                color_local_bounds = color_local_bounds.union(local_bounds);
                continue;
            }
            let layers = glyph
                .font
                .colr0_glyph_layers(glyph.id)
                .map_err(|e| unsupported_positioned_glyph_data_error(glyph, &e))?;
            let Some(layers) = layers else {
                let segments = glyph
                    .font
                    .outline
                    .load_glyph(glyph.id as u16, ppem)
                    .map_err(|e| unsupported_positioned_glyph_data_error(glyph, &e.to_string()))?;
                let glyph_paths = flatten_glyph(&segments, origin, tolerance, &mut || {
                    self.add_path_segment()
                })
                .map_err(|e| format!("flatten glyph {index} (ID {}): {e}", glyph.id))?;
                if has_color_glyph {
                    all_paths.extend(glyph_paths.iter().cloned());
                }
                paths.extend(glyph_paths);
                continue;
            };
            if !has_color_glyph {
                self.add_prepared_nodes(1)
                    .map_err(|e| format!("d2raster: node {node_id:?} color text root: {e}"))?;
                all_paths = paths.clone();
                has_color_glyph = true;
            }
            self.flush_user_paths(node_id, &mut paths, &mut parts)?;
            for (layer_index, layer) in layers.iter().enumerate() {
                self.add_prepared_nodes(1).map_err(|e| format!("d2raster: node {node_id:?} glyph {index} (ID {}) COLRv0 layer {layer_index}: {e}", glyph.id))?;
                self.add_path_segment().map_err(|e| {
                    format!(
                        "glyph {index} (ID {}) COLRv0 layer {layer_index}: {e}",
                        glyph.id
                    )
                })?;
                let segments = glyph
                    .font
                    .outline
                    .load_glyph(layer.glyph_id as u16, ppem)
                    .map_err(|e| {
                        unsupported_positioned_glyph_data_error(
                            glyph,
                            &format!("load COLRv0 layer glyph {}: {e}", layer.glyph_id),
                        )
                    })?;
                let layer_paths = flatten_glyph(&segments, origin, tolerance, &mut || {
                    self.add_path_segment()
                })
                .map_err(|e| {
                    format!(
                        "flatten glyph {index} (ID {}) COLRv0 layer {layer_index}: {e}",
                        glyph.id
                    )
                })?;
                all_paths.extend(layer_paths.iter().cloned());
                if !layer_paths.is_empty() {
                    parts.push(PreparedTextPart {
                        paths: layer_paths,
                        foreground: layer.foreground,
                        color: Nrgba::new(
                            layer.color.r,
                            layer.color.g,
                            layer.color.b,
                            layer.color.a,
                        ),
                        ..Default::default()
                    });
                }
            }
        }
        if has_color_glyph {
            self.flush_user_paths(node_id, &mut paths, &mut parts)?;
        }
        let mut decorations = Vec::new();

        if (text.underline || text.strike) && advance > 0.0 {
            let metrics = primary.outline.metrics(ppem, Hinting::None);
            let mut min_thickness = 1.0;
            let scale = transform.max_scale();
            if finite(scale) && scale > GEOMETRY_EPSILON {
                min_thickness = 1.0 / scale;
            }
            let thickness = go_max(size / 16.0, min_thickness);
            let left = text.origin.x + anchor_offset;
            if text.underline {
                let descent = metrics.descent as f64 / 64.0;
                let top = text.origin.y + go_max(size / 12.0, descent * 0.25);
                let decoration = self
                    .decoration_subpath(left, top, advance, thickness)
                    .map_err(|e| format!("d2raster: node {node_id:?} underline: {e}"))?;
                if has_color_glyph {
                    all_paths.push(decoration.clone());
                    decorations.push(decoration);
                } else {
                    paths.push(decoration);
                }
            }
            if text.strike {
                let mut x_height = metrics.x_height as f64 / 64.0;
                if x_height <= 0.0 {
                    x_height = size * 0.5;
                }
                let top = text.origin.y - x_height / 2.0 - thickness / 2.0;
                let decoration = self
                    .decoration_subpath(left, top, advance, thickness)
                    .map_err(|e| format!("d2raster: node {node_id:?} strike: {e}"))?;
                if has_color_glyph {
                    all_paths.push(decoration.clone());
                    decorations.push(decoration);
                } else {
                    paths.push(decoration);
                }
            }
        }
        if has_color_glyph && !decorations.is_empty() {
            self.add_prepared_nodes(1)
                .map_err(|e| format!("d2raster: node {node_id:?} color text: {e}"))?;
            parts.push(PreparedTextPart {
                paths: decorations,
                user_paint: true,
                ..Default::default()
            });
        }
        if !has_color_glyph {
            all_paths = paths;
        }
        let object_bounds = if text.ink.valid {
            text.ink.to_box()
        } else {
            let bounds = local_object_bounds(&all_paths);
            if color_local_bounds.valid {
                if all_paths.is_empty() {
                    color_local_bounds.to_box()
                } else {
                    bounds.bounds().union(color_local_bounds).to_box()
                }
            } else {
                bounds
            }
        };
        let fill = self
            .prepare_animated_paint(
                text.fill.as_ref(),
                animation.fill_color,
                object_bounds,
                transform,
                import_depth,
            )
            .map_err(|e| format!("d2raster: node {node_id:?} text fill: {e}"))?;
        let stroke = self
            .prepare_animated_stroke(
                text.stroke.as_ref(),
                animation.stroke_color,
                animation.dash_offset,
                object_bounds,
                transform,
                import_depth,
            )
            .map_err(|e| format!("d2raster: node {node_id:?} text stroke: {e}"))?;
        if !has_color_glyph {
            return self.finish_primitive(node_id, all_paths, transform, fill, stroke);
        }
        let mut root = PreparedNode::group(Vec::new(), None);
        for part in parts {
            if let Some(vector) = part.vector {
                crate::colrv1::append_prepared_child(&mut root, vector);
                continue;
            }
            let (part_fill, part_stroke) = if part.user_paint {
                (fill.clone(), stroke.clone())
            } else if part.foreground {
                (fill.clone(), None)
            } else {
                (Some(PreparedPaint::solid(part.color)), None)
            };
            let primitive =
                self.finish_primitive(node_id, part.paths, transform, part_fill, part_stroke)?;
            let bounds = primitive.bounds;
            let mut child = PreparedNode::group(Vec::new(), None);
            child.primitive = Some(Box::new(primitive));
            child.bounds = bounds;
            child.content_bounds = bounds;
            root.bounds = union_rect(root.bounds, bounds);
            root.children.push(child);
        }
        root.content_bounds = root.bounds;
        let bounds = root.bounds;
        Ok(PreparedPrimitive {
            vector: Some(Box::new(root)),
            bounds,
            ..Default::default()
        })
    }

    fn decoration_subpath(
        &mut self,
        left: f64,
        top: f64,
        width: f64,
        height: f64,
    ) -> Result<Subpath, String> {
        let points = vec![
            Point::new(left, top),
            Point::new(left + width, top),
            Point::new(left + width, top + height),
            Point::new(left, top + height),
        ];
        for _ in 0..points.len() {
            self.add_path_segment()?;
        }
        Ok(Subpath {
            points,
            closed: true,
        })
    }

    /// positionGlyphs: explicit scene glyphs, or shaping of raw text.
    pub(crate) fn position_glyphs(
        &mut self,
        text: &TextRun,
        ppem: i32,
    ) -> Result<(Vec<PositionedGlyph>, f64), String> {
        let explicit = text.glyphs.as_deref().unwrap_or_default();
        if !explicit.is_empty() {
            if explicit.len() as i64 > self.options.max_text_runes_per_run {
                return Err(format!(
                    "explicit glyph count {} exceeds per-run limit {}",
                    explicit.len(),
                    self.options.max_text_runes_per_run
                ));
            }
            if explicit.len() as i64 > self.options.max_path_commands - self.shaped_glyphs {
                return Err(format!(
                    "shaped glyph count exceeds limit {}",
                    self.options.max_path_commands
                ));
            }
            let mut glyphs = Vec::with_capacity(explicit.len());
            let mut advance = 0.0f64;
            for (index, glyph) in explicit.iter().enumerate() {
                let asset_id = if glyph.asset.is_empty() {
                    &text.font.asset
                } else {
                    &glyph.asset
                };
                let Some(parsed) = self.fonts.get(asset_id).cloned() else {
                    return Err(format!(
                        "glyph {index} references unusable font asset {asset_id:?}"
                    ));
                };
                if glyph.empty {
                    if glyph.id != 0 {
                        return Err(format!(
                            "empty glyph at index {index} has non-zero glyph ID {}",
                            glyph.id
                        ));
                    }
                    if !finite_point(glyph.position)
                        || !finite(glyph.advance)
                        || !glyph.ink.is_finite()
                    {
                        return Err(format!("glyph {index} has invalid geometry"));
                    }
                    glyphs.push(PositionedGlyph {
                        id: 0,
                        font: parsed,
                        asset: asset_id.clone(),
                        position: glyph.position,
                        empty: true,
                        source: 0,
                        source_at: 0,
                    });
                    advance = go_max(advance, glyph.position.x + glyph.advance);
                    continue;
                }
                if glyph.id == 0 {
                    return Err(format!("missing glyph at index {index}: glyph ID 0"));
                }
                if glyph.id > u16::MAX as u32 || glyph.id as usize >= parsed.num_glyphs() {
                    return Err(format!(
                        "glyph ID {} at index {index} is out of range",
                        glyph.id
                    ));
                }
                if !finite_point(glyph.position) || !finite(glyph.advance) || !glyph.ink.is_finite()
                {
                    return Err(format!("glyph {index} has invalid geometry"));
                }
                glyphs.push(PositionedGlyph {
                    id: glyph.id,
                    font: parsed,
                    asset: asset_id.clone(),
                    position: glyph.position,
                    empty: false,
                    source: 0,
                    source_at: 0,
                });
                advance = go_max(advance, glyph.position.x + glyph.advance);
            }
            self.shaped_glyphs += glyphs.len() as i64;
            return Ok((glyphs, advance));
        }

        let remaining_runes = self.options.max_path_commands - self.text_runes;
        if remaining_runes <= 0 {
            return Err(format!(
                "text rune count exceeds aggregate shaping-input limit {}",
                self.options.max_path_commands
            ));
        }
        let fallbacks = text.fallbacks.as_deref().unwrap_or_default();
        if fallbacks.len() as i64 > self.options.max_assets {
            return Err(format!(
                "fallback font reference count {} exceeds asset limit {}",
                fallbacks.len(),
                self.options.max_assets
            ));
        }
        let mut seen: Vec<&str> = Vec::new();
        for index in 0..=fallbacks.len() {
            let id = if index == 0 {
                text.font.asset.as_str()
            } else {
                fallbacks[index - 1].as_str()
            };
            if seen.contains(&id) {
                continue;
            }
            seen.push(id);
            if !self.fonts.contains_key(id) {
                return Err(format!("text references unusable font asset {id:?}"));
            }
        }
        if self.options.max_text_coverage_checks - self.text_coverage_checks <= 0 {
            return Err(format!(
                "text font coverage checks exceed aggregate limit {}",
                self.options.max_text_coverage_checks
            ));
        }
        if self.options.max_text_shaping_runs - self.text_shaping_runs <= 0 {
            return Err(format!(
                "text shaping run count exceeds aggregate limit {}",
                self.options.max_text_shaping_runs
            ));
        }
        if self.options.max_path_commands - self.shaped_glyphs <= 0 {
            return Err(format!(
                "shaped glyph count exceeds limit {}",
                self.options.max_path_commands
            ));
        }
        let prepared_faces: Vec<_> = seen.iter().map(|id| self.fonts[*id].clone()).collect();
        let parsed_faces = prepared_faces
            .iter()
            .zip(&seen)
            .map(|(font, id)| {
                font.face_for_shaping()
                    .map_err(|e| format!("prepare shaping font asset {id:?}: {e}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let shape_faces: Vec<_> = parsed_faces
            .iter()
            .zip(&seen)
            .map(|(face, id)| ShapeFace { id, face })
            .collect();
        let shaped = self
            .shaping_workspace
            .shape_text_transient(
                &(),
                &text.text,
                ppem,
                &shape_faces,
                ShapeLimits {
                    runes: self.options.max_text_runes_per_run.min(remaining_runes),
                    faces: self.options.max_font_faces_per_text,
                    coverage_checks: self.options.max_text_coverage_checks
                        - self.text_coverage_checks,
                    runs: self.options.max_text_shaping_runs - self.text_shaping_runs,
                    glyphs: self.options.max_path_commands - self.shaped_glyphs,
                },
            )
            .map_err(|e| e.to_string())?;
        self.text_runes += shaped.runes;
        self.text_coverage_checks += shaped.coverage_checks;
        self.text_shaping_runs += shaped.runs;
        self.shaped_glyphs += shaped.glyphs.len() as i64;
        let glyphs = shaped
            .glyphs
            .iter()
            .map(|glyph| PositionedGlyph {
                id: glyph.id,
                font: prepared_faces[glyph.face].clone(),
                asset: seen[glyph.face].to_string(),
                position: Point::new(glyph.position_x, glyph.position_y),
                empty: glyph.empty,
                source: glyph.source as u32,
                source_at: glyph.source_index,
            })
            .collect();
        Ok((glyphs, shaped.advance))
    }
}

/// flattenGlyph: 26.6 glyph segments to closed device-independent subpaths.
pub(crate) fn flatten_glyph(
    segments: &[d2_rs_fontface::sfnt::Segment],
    origin: Point,
    tolerance: f64,
    count: &mut dyn FnMut() -> Result<(), String>,
) -> Result<Vec<Subpath>, String> {
    let mut paths = Vec::new();
    let mut points: Vec<Point> = Vec::new();
    let mut cursor = Point::default();
    let mut have_cursor = false;
    let point = |value: d2_rs_fontface::fixed::Point26_6| {
        Point::new(
            origin.x + value.x as f64 / 64.0,
            origin.y + value.y as f64 / 64.0,
        )
    };
    fn flush(paths: &mut Vec<Subpath>, points: &mut Vec<Point>) {
        if !points.is_empty() {
            paths.push(Subpath {
                points: std::mem::take(points),
                closed: true,
            });
        }
    }
    for (index, segment) in segments.iter().enumerate() {
        let mut append = |points: &mut Vec<Point>, p: Point| -> Result<(), String> {
            if !finite_point(p) {
                return Err("non-finite point".into());
            }
            count()?;
            if points.last() != Some(&p) {
                points.push(p);
            }
            Ok(())
        };
        match segment.op {
            SegmentOp::MoveTo => {
                flush(&mut paths, &mut points);
                cursor = point(segment.args[0]);
                append(&mut points, cursor).map_err(|e| format!("segment {index}: {e}"))?;
                have_cursor = true;
            }
            SegmentOp::LineTo => {
                if !have_cursor {
                    return Err(format!("segment {index}: line before move"));
                }
                cursor = point(segment.args[0]);
                append(&mut points, cursor).map_err(|e| format!("segment {index}: {e}"))?;
            }
            SegmentOp::QuadTo => {
                if !have_cursor {
                    return Err(format!("segment {index}: quadratic before move"));
                }
                let (control, end) = (point(segment.args[0]), point(segment.args[1]));
                flatten_quadratic(cursor, control, end, tolerance, 0, &mut |p| {
                    append(&mut points, p)
                })
                .map_err(|e| format!("segment {index}: {e}"))?;
                cursor = end;
            }
            SegmentOp::CubeTo => {
                if !have_cursor {
                    return Err(format!("segment {index}: cubic before move"));
                }
                let (c1, c2, end) = (
                    point(segment.args[0]),
                    point(segment.args[1]),
                    point(segment.args[2]),
                );
                flatten_cubic(cursor, c1, c2, end, tolerance, 0, &mut |p| {
                    append(&mut points, p)
                })
                .map_err(|e| format!("segment {index}: {e}"))?;
                cursor = end;
            }
        }
    }
    flush(&mut paths, &mut points);
    Ok(paths)
}
