// SPDX-License-Identifier: MPL-2.0
//! d2isometricimg/vector_surface_text.go: shaped text to reusable glyph paths.
use super::vector_surface_svg::{SurfaceSvg, attr};
use d2_rs_fontface::{self as fontface, ParsedFace, ShapeFace, ShapeLimits, ShapingWorkspace};
use d2_rs_scene::*;
use fontface::sfnt::Hinting;
use std::{collections::HashSet, rc::Rc};

impl SurfaceSvg<'_> {
    pub fn font(&mut self, id: &str) -> Result<Rc<ParsedFace>, String> {
        if let Some(f) = self.fonts.get(id) {
            return Ok(f.clone());
        }
        let Some(Asset::FontAsset {
            data, face_index, ..
        }) = self.assets.get(id)
        else {
            return Err(format!("native SVG missing font asset {id:?}"));
        };
        if self.fonts.len() >= 96 || data.len() > 64 << 20 {
            return Err("native SVG font assets exceed limit".into());
        }
        let source = fontface::registered_bundled_face(data, *face_index)
            .map_err(|e| e.to_string())?
            .or(
                fontface::registered_bundled_noto_color_emoji(data, *face_index)
                    .map_err(|e| e.to_string())?,
            );
        let face = if let Some(source) = source {
            source.clone_read_only()
        } else {
            fontface::parse_face(data, *face_index)
        }
        .map_err(|e| e.to_string())?;
        let face = Rc::new(face);
        self.fonts.insert(id.into(), face.clone());
        Ok(face)
    }
    pub fn text(&mut self, run: &TextRun, depth: usize) -> String {
        match self.text_result(run, depth) {
            Ok(s) => s,
            Err(e) => {
                self.fail(e);
                String::new()
            }
        }
    }
    fn text_result(&mut self, run: &TextRun, depth: usize) -> Result<String, String> {
        if !self.admit(depth) {
            return Ok(String::new());
        }
        let primary = self.font(&run.font.asset)?;
        let ppem = (run.font.size() * 64.0).round() as i32;
        if ppem <= 0 {
            return Err("native SVG has invalid text size".into());
        }
        let mut glyphs = run.glyphs.clone().unwrap_or_default();
        let mut advance = 0.0;
        if glyphs.is_empty() && !run.text.is_empty() {
            let mut seen = HashSet::new();
            let mut ids = vec![run.font.asset.clone()];
            ids.extend(run.fallbacks.clone().unwrap_or_default());
            ids.retain(|id| seen.insert(id.clone()));
            let faces = ids
                .iter()
                .map(|id| self.font(id))
                .collect::<Result<Vec<_>, _>>()?;
            let shape_faces = ids
                .iter()
                .zip(&faces)
                .map(|(id, f)| ShapeFace { id, face: f })
                .collect::<Vec<_>>();
            let shaped = ShapingWorkspace::new()
                .shape_text_transient(
                    &(),
                    &run.text,
                    ppem,
                    &shape_faces,
                    ShapeLimits {
                        runes: 65536,
                        faces: 8,
                        coverage_checks: 8000000,
                        runs: 200000,
                        glyphs: 1000000,
                    },
                )
                .map_err(|e| e.to_string())?;
            advance = shaped.advance;
            glyphs = shaped
                .glyphs
                .iter()
                .map(|g| Glyph {
                    id: g.id,
                    empty: g.empty,
                    asset: ids[g.face].clone(),
                    position: Point::new(g.position_x, g.position_y),
                    advance: g.advance,
                    ..Default::default()
                })
                .collect();
        } else {
            for g in &glyphs {
                advance = go_max(advance, g.position.x + g.advance)
            }
        }
        if glyphs.len() > 65536 {
            return Err("native SVG exceeds glyph count".into());
        }
        let anchor = if run.anchor == TextAnchor::MIDDLE {
            -advance / 2.0
        } else if run.anchor == TextAnchor::END {
            -advance
        } else {
            0.0
        };
        let mut body = String::new();
        for g in glyphs {
            if g.empty {
                continue;
            }
            let face = self.font(if g.asset.is_empty() {
                &run.font.asset
            } else {
                &g.asset
            })?;
            let origin = Point::new(
                run.origin.x + anchor + g.position.x,
                run.origin.y + g.position.y,
            );
            if face.is_bundled_noto_color_emoji() {
                if let Some(plan) = face
                    .compile_bundled_noto_color_emoji_colrv1_plan(g.id)
                    .map_err(|e| e.to_string())?
                {
                    let s = self.color_glyph(&face, &plan, origin, run.font.size(), depth + 1);
                    self.append(&mut body, s);
                    continue;
                }
            }
            if let Some(layers) = face.colr0_glyph_layers(g.id).map_err(|e| e.to_string())? {
                for layer in layers {
                    let c = layer.color;
                    let fill = if layer.foreground {
                        run.fill.clone()
                    } else {
                        Some(Paint::solid(Nrgba::new(c.r, c.g, c.b, c.a)))
                    };
                    let s = self.glyph(
                        &face,
                        layer.glyph_id,
                        ppem,
                        origin,
                        fill.as_ref(),
                        None,
                        depth + 1,
                    )?;
                    self.append(&mut body, s);
                }
                continue;
            }
            let s = self.glyph(
                &face,
                g.id,
                ppem,
                origin,
                run.fill.as_ref(),
                run.stroke.as_ref(),
                depth + 1,
            )?;
            self.append(&mut body, s);
        }
        if (run.underline || run.strike) && advance > 0.0 {
            let metrics = primary.outline().metrics(ppem, Hinting::None);
            let scale = self.transform.max_scale();
            let thickness = go_max(run.font.size() / 16.0, 1.0 / go_max(1e-12, scale));
            let mut add = |top| {
                let s = self.primitive(
                    Some(&Primitive::Rect(Rect {
                        r#box: Box {
                            x: run.origin.x + anchor,
                            y: top,
                            width: advance,
                            height: thickness,
                        },
                        fill: run.fill.clone(),
                        ..Default::default()
                    })),
                    depth + 1,
                );
                self.append(&mut body, s)
            };
            if run.underline {
                add(run.origin.y
                    + go_max(run.font.size() / 12.0, metrics.descent as f64 / 64.0 * 0.25))
            }
            if run.strike {
                let mut x_height = metrics.x_height as f64 / 64.0;
                if x_height <= 0.0 {
                    x_height = run.font.size() * 0.5
                }
                add(run.origin.y - x_height / 2.0 - thickness / 2.0)
            }
        }
        Ok(body)
    }
    pub fn glyph_definition(
        &mut self,
        face: &ParsedFace,
        id: u32,
        ppem: i32,
    ) -> Result<String, String> {
        let key = (face as *const ParsedFace as usize, id, ppem);
        if let Some(name) = self.glyphs.get(&key) {
            return Ok(name.clone());
        }
        let path = glyph_path(face, id, ppem, Point::default())?;
        let name = self.id("glyph");
        let path = self.path(&path);
        self.def(format!("<path{}{}/>", attr("id", &name), attr("d", &path)));
        self.glyphs.insert(key, name.clone());
        Ok(name)
    }
    fn glyph(
        &mut self,
        face: &ParsedFace,
        id: u32,
        ppem: i32,
        origin: Point,
        fill: Option<&Paint>,
        stroke: Option<&Stroke>,
        depth: usize,
    ) -> Result<String, String> {
        if !self.admit(depth) {
            return Ok(String::new());
        }
        let solid = |p: Option<&Paint>| p.is_none_or(|p| matches!(p, Paint::SolidPaint { .. }));
        if !solid(fill) || stroke.is_some_and(|s| !solid(s.paint.as_ref())) {
            let mut path = glyph_path(face, id, ppem, origin)?;
            path.fill = fill.cloned();
            path.stroke = stroke.cloned();
            return Ok(self.primitive(Some(&Primitive::Path(path)), depth + 1));
        }
        let name = self.glyph_definition(face, id, ppem)?;
        let style = self.style(fill, stroke, depth + 1);
        Ok(format!(
            "<use{}{}{style}/>",
            attr("href", &format!("#{name}")),
            attr(
                "transform",
                &format!(
                    "translate({} {})",
                    super::vector_surface_svg::number(origin.x),
                    super::vector_surface_svg::number(origin.y)
                )
            )
        ))
    }
}
fn glyph_path(face: &ParsedFace, id: u32, ppem: i32, origin: Point) -> Result<Path, String> {
    let commands = d2_rs_fontface::glyph_path(face, id, ppem, [origin.x, origin.y])?;
    Ok(Path {
        commands: commands
            .into_iter()
            .map(|c| PathCommand {
                kind: d2_rs_scene::PathCommandKind(c.kind),
                p1: Point::new(c.points[0][0], c.points[0][1]),
                p2: Point::new(c.points[1][0], c.points[1][1]),
                p3: Point::new(c.points[2][0], c.points[2][1]),
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    })
}
