// SPDX-License-Identifier: MPL-2.0
//! Thin native foundation adapters for isometric surfaces.
pub fn normalized_png(data: &[u8]) -> Result<Vec<u8>, String> {
    normalized_raster_png(data, "image/png")
}
pub fn normalized_raster_png(data: &[u8], mime_type: &str) -> Result<Vec<u8>, String> {
    d2_rs_raster::normalize_first_frame_png(data, mime_type)
}

// Oracle tier: reads the regenerated static-png.json (docs/FIXTURES.md).
#[cfg(all(test, feature = "oracle-fixtures"))]
mod tests {
    use super::*;
    use base64::Engine as _;

    #[test]
    fn go_static_png_contracts() {
        // Palette bit depths, transparent palettes, opaque/alpha NRGBA and
        // Gray16 use their Go concrete-image encoding contracts. Inputs use
        // BestSpeed; expected PNGs use png.Encode's default compression.
        let records: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/static-png.json"
        ))
        .unwrap();
        for record in records.as_array().unwrap() {
            let input = base64::engine::general_purpose::STANDARD
                .decode(record["Input"].as_str().unwrap())
                .unwrap();
            let expected = base64::engine::general_purpose::STANDARD
                .decode(record["Expected"].as_str().unwrap())
                .unwrap();
            assert_eq!(
                normalized_png(&input).unwrap(),
                expected,
                "{}",
                record["Name"]
            );
        }
    }
}

/// surface_document.go rasterNativeSurfaceDocument's frame adapter. The
/// public foundation renderer returns straight alpha; textures are RGBA.
pub fn raster_surface_straight(
    document: &d2_rs_scene::Document,
    width: usize,
    height: usize,
) -> Result<Vec<u8>, String> {
    raster_surface_straight_at(document, width, height, 0.0)
}

fn raster_surface_straight_at(
    document: &d2_rs_scene::Document,
    width: usize,
    height: usize,
    seconds: f64,
) -> Result<Vec<u8>, String> {
    use d2_rs_scene::{ViewportAlign, ViewportFit};
    if width == 0 || height == 0 || width > 8192 || height > 8192 || width * height > 8 << 20 {
        return Err("native surface has invalid document or texture dimensions".into());
    }
    if !seconds.is_finite() || seconds < 0.0 || seconds > i64::MAX as f64 / 1e9 {
        return Err("native surface has invalid frame time".into());
    }
    let mut view = document.clone();
    view.logical_width = width as f64;
    view.logical_height = height as f64;
    view.viewport_fit = ViewportFit::MEET;
    view.viewport_align = ViewportAlign::X_MID_Y_MID;
    let frame = d2_rs_raster::render(
        &view,
        &d2_rs_raster::FrameOptions {
            scale: 1.0,
            time: (seconds * 1e9) as i64,
            max_width: width as i64,
            max_height: height as i64,
            max_pixels: (width * height) as i64,
            max_nodes: 200000,
            max_depth: 256,
            max_path_commands: 2000000,
            max_text_runes_per_run: 65536,
            max_text_coverage_checks: 8000000,
            max_text_shaping_runs: 200000,
            max_font_faces_per_text: 8,
            max_animation_tracks: 20000,
            max_animation_keyframes: 200000,
            max_assets: 20000,
            max_asset_bytes: 128 << 20,
            max_decoded_asset_bytes: 128 << 20,
            max_import_depth: 32,
            max_offscreen_bytes: 256 << 20,
            max_even_odd_clip_work: 64000000,
            max_scanline_work: 500000000,
            ..Default::default()
        },
    )
    .map_err(|e| e.to_string())?;
    Ok(frame.pix)
}

pub fn raster_surface(
    document: &d2_rs_scene::Document,
    width: usize,
    height: usize,
) -> Result<Vec<u8>, String> {
    raster_surface_at(document, width, height, 0.0)
}

pub fn raster_surface_at(
    document: &d2_rs_scene::Document,
    width: usize,
    height: usize,
    seconds: f64,
) -> Result<Vec<u8>, String> {
    let pixels = raster_surface_straight_at(document, width, height, seconds)?;
    Ok(pixels
        .chunks_exact(4)
        .flat_map(|c| {
            let a = c[3] as u32 * 257;
            [
                ((c[0] as u32 * 257 * a / 65535) >> 8) as u8,
                ((c[1] as u32 * 257 * a / 65535) >> 8) as u8,
                ((c[2] as u32 * 257 * a / 65535) >> 8) as u8,
                c[3],
            ]
        })
        .collect())
}

pub(crate) use super::rich_labels::rich_raster_options;
/// Native face documents use ordinary D2's shape grammar with empty padding.
pub fn build_face_document(s: &crate::target::Shape) -> Result<d2_rs_scene::Document, String> {
    let d = crate::target::Diagram {
        root: crate::target::Shape {
            fill: "transparent".into(),
            stroke: "none".into(),
            ..Default::default()
        },
        shapes: Some(vec![s.clone()]),
        ..Default::default()
    };
    d2_rs_scenebuild::build(
        &d,
        &d2_rs_scenebuild::Options {
            pad: Some(0),
            max_nodes: 20000,
            max_path_commands: 250000,
            link_budget: d2_rs_scenebuild::LinkBudget {
                max_regions: 2,
                max_string_bytes: 4096,
            },
            ..Default::default()
        },
    )
    .map_err(|e| e.to_string())
}
pub fn render(
    doc: &d2_rs_scene::Document,
    opts: &d2_rs_raster::FrameOptions,
) -> Result<super::mesh::Texture, String> {
    let f = d2_rs_raster::render(doc, opts).map_err(|e| e.to_string())?;
    let pixels = f
        .pix
        .chunks_exact(4)
        .flat_map(|c| {
            let a = c[3] as u32 * 257;
            [
                ((c[0] as u32 * 257 * a / 65535) >> 8) as u8,
                ((c[1] as u32 * 257 * a / 65535) >> 8) as u8,
                ((c[2] as u32 * 257 * a / 65535) >> 8) as u8,
                c[3],
            ]
        })
        .collect();
    Ok(super::mesh::Texture::new(f.width(), f.height(), pixels))
}
pub fn icon_size(b: &d2_rs_scenebuild::golib::geo::Box, position: &str) -> i64 {
    use d2_rs_scenebuild::golib::label::Position;
    let pos = Position::from_string(position);
    let min_dim = b.width.min(b.height) as i64;
    let half = (0.5 * min_dim as f64).ceil() as i64;
    let mut size = if pos == Position::InsideMiddleCenter {
        half
    } else {
        min_dim.min(32.max(half))
    };
    size = size.min(64);
    if !pos.is_outside() {
        size = size.min(
            (b.width as i64 - 10)
                .max(0)
                .min((b.height as i64 - 10).max(0)),
        );
    }
    size
}
/// Thin adapter for lib/color.ParseGradient, whose foundation parser is private.
pub fn gradient_parts(raw: &str) -> Result<(String, String, Vec<(String, String)>), String> {
    let raw = raw.trim();
    let (kind, rest) = if let Some(r) = raw.strip_prefix("linear-gradient(") {
        ("linear", r)
    } else if let Some(r) = raw.strip_prefix("radial-gradient(") {
        ("radial", r)
    } else {
        return Err("invalid gradient syntax".into());
    };
    let params = rest
        .strip_suffix(')')
        .filter(|p| !p.contains('\n'))
        .ok_or("invalid gradient syntax")?;
    let (mut list, mut buf, mut nesting) = (vec![], String::new(), 0usize);
    for c in params.chars() {
        match c {
            ',' if nesting == 0 => {
                list.push(std::mem::take(&mut buf));
                continue;
            }
            '(' => nesting += 1,
            ')' => nesting = nesting.saturating_sub(1),
            _ => {}
        }
        buf.push(c);
    }
    if !buf.is_empty() {
        list.push(buf);
    }
    let first = list.first().ok_or("no parameters in gradient")?.trim();
    let has_direction = (kind == "linear" && (first.ends_with("deg") || first.starts_with("to ")))
        || (kind == "radial" && (first == "circle" || first == "ellipse"));
    let direction = if has_direction {
        first.to_owned()
    } else {
        String::new()
    };
    let start = has_direction as usize;
    if start == list.len() {
        return Err("no color stops in gradient".into());
    }
    let mut stops = vec![];
    for (index, p) in list[start..].iter().enumerate() {
        let p = p.trim();
        let parts: Vec<_> = p.split_whitespace().collect();
        match parts.as_slice() {
            [c] => stops.push((c.to_string(), String::new())),
            [c, pos] => {
                // lib/color/gradient.go:130–144. Admission happens before the
                // opacity wash; never drop a malformed stop or defer its error
                // until the rewritten face paint is built by scenebuild.
                let number = pos.strip_suffix('%').unwrap_or(pos);
                let value = d2_rs_core::parse_d2_float(number)
                    .filter(|v| v.is_finite())
                    .ok_or_else(|| {
                        format!(
                            "color stop {index} position {}: must be a finite number or percentage",
                            d2_rs_core::quote_go_bytes(pos.as_bytes())
                        )
                    })?;
                let mut canonical = super::vector_surface_svg::format_g(value);
                if pos.ends_with('%') {
                    canonical.push('%');
                }
                stops.push((c.to_string(), canonical));
            }
            _ => {
                return Err(format!(
                    "invalid color stop {index} {}",
                    d2_rs_core::quote_go_bytes(p.as_bytes())
                ));
            }
        }
    }
    Ok((kind.into(), direction, stops))
}

// Thin ownership bridge for d2svgimport's public native arena. The shared
// scenebuild bridge is private; keep primitive conversion here until exposed.
mod svg_ownership {
    use d2_rs_math::{svg_path::Command, svg_primitive};
    use d2_rs_scene::*;
    fn point(p: [f64; 2]) -> Point {
        Point::new(p[0], p[1])
    }
    fn matrix(m: [f64; 6]) -> Matrix {
        Matrix {
            a: m[0],
            b: m[1],
            c: m[2],
            d: m[3],
            e: m[4],
            f: m[5],
        }
    }
    fn bounds(b: [f64; 4]) -> Box {
        Box {
            x: b[0],
            y: b[1],
            width: b[2],
            height: b[3],
        }
    }
    fn color(c: [u8; 4]) -> Nrgba {
        Nrgba::new(c[0], c[1], c[2], c[3])
    }
    fn paint(p: &svg_primitive::Paint) -> Paint {
        match p {
            svg_primitive::Paint::Solid { color: c } => Paint::solid(color(*c)),
            svg_primitive::Paint::Linear {
                start,
                end,
                stops,
                units,
                transform,
                spread,
            } => Paint::LinearGradient(LinearGradient {
                start: point(*start),
                end: point(*end),
                stops: stops
                    .iter()
                    .map(|s| GradientStop {
                        offset: s.offset,
                        color: color(s.color),
                    })
                    .collect(),
                units: PaintUnits(*units),
                transform: matrix(*transform),
                spread: SpreadMethod(*spread),
            }),
        }
    }
    fn commands(cs: &[Command]) -> Vec<PathCommand> {
        cs.iter()
            .map(|c| PathCommand {
                kind: PathCommandKind(c.kind),
                p1: point(c.points[0]),
                p2: point(c.points[1]),
                p3: point(c.points[2]),
                radius_x: c.radii[0],
                radius_y: c.radii[1],
                rotation: c.rotation,
                large_arc: c.large_arc,
                sweep: c.sweep,
            })
            .collect()
    }
    fn primitive(p: &svg_primitive::Primitive) -> Primitive {
        let fill = p.fill.as_ref().map(paint);
        let stroke = p.stroke.as_ref().map(|s| Stroke {
            paint: Some(paint(&s.paint)),
            width: s.width,
            cap: LineCap(s.cap),
            join: LineJoin(s.join),
            miter_limit: s.miter_limit,
            dashes: s.dashes.clone(),
            dash_offset: s.dash_offset,
        });
        let g = &p.geometry;
        match g.kind {
            1 => Primitive::Rect(Rect {
                r#box: bounds(g.bounds),
                radius_x: g.radii[0],
                radius_y: g.radii[1],
                fill,
                stroke,
            }),
            2 => Primitive::Ellipse(Ellipse {
                center: point(g.center),
                radius_x: g.radii[0],
                radius_y: g.radii[1],
                fill,
                stroke,
            }),
            3 => Primitive::Path(Path {
                commands: commands(&g.commands),
                fill_rule: FillRule(p.fill_rule),
                fill,
                stroke,
            }),
            4 => Primitive::Image(Image {
                asset: g.asset.clone(),
                r#box: bounds(g.bounds),
                aspect: AspectRatio {
                    align: AspectAlign(g.aspect[0]),
                    fit: AspectFit(g.aspect[1]),
                },
            }),
            _ => unreachable!("importer emitted an unknown primitive"),
        }
    }

    /// Consume the arena iteratively. Reverse traversal guarantees children are
    /// moved exactly once, without recursive cloning or a source-depth stack.
    fn imported_root(scene: d2_rs_math::svg_scene::Scene) -> Node {
        let mut mapped: Vec<Option<Node>> = (0..scene.nodes.len()).map(|_| None).collect();
        let root = scene.root.expect("importer always emits a root");
        let mut stack = vec![(root, false)];
        while let Some((index, visited)) = stack.pop() {
            let n = &scene.nodes[index];
            if !visited {
                stack.push((index, true));
                stack.extend(n.children.iter().rev().map(|&c| (c, false)));
                continue;
            }
            let mut out = Node::new(n.primitive.as_ref().map(primitive));
            out.id = n.id.clone();
            out.classes = (!n.classes.is_empty()).then(|| n.classes.clone());
            out.transform = matrix(n.transform);
            out.opacity = n.opacity;
            out.clip = n.clip.as_ref().map(|c| Clip {
                path: Path {
                    commands: commands(&c.commands),
                    fill_rule: FillRule(c.fill_rule),
                    ..Default::default()
                },
                transform: matrix(c.transform),
            });
            out.children = (!n.children.is_empty()).then(|| {
                n.children
                    .iter()
                    .map(|&c| mapped[c].take().expect("owned child"))
                    .collect()
            });
            mapped[index] = Some(out);
        }
        mapped[root].take().unwrap()
    }

    pub(crate) fn import(
        data: &[u8],
        limits: [usize; 9],
    ) -> Result<d2_rs_math::svg_import::Imported, String> {
        match d2_rs_math::svg_import::import_labeled("surface icon", data, limits, None, None)? {
            d2_rs_math::svg_import::Outcome::Imported(imported) => Ok(imported),
            _ => Err("isometric surface icon SVG importer returned pending image".into()),
        }
    }
    pub(crate) fn asset(
        imported: d2_rs_math::svg_import::Imported,
    ) -> (Asset, std::collections::BTreeMap<String, Asset>) {
        let viewport = Box {
            width: imported.viewport.width,
            height: imported.viewport.height,
            ..Default::default()
        };
        let mut content = Node::new(None);
        content.transform = matrix(imported.viewport.transform);
        content.children = Some(vec![imported_root(imported.scene)]);
        let mut root = Node::new(None);
        root.clip = Some(Clip {
            transform: Matrix::identity(),
            path: Path {
                commands: vec![
                    PathCommand::move_to(0., 0.),
                    PathCommand::line_to(viewport.width, 0.),
                    PathCommand::line_to(viewport.width, viewport.height),
                    PathCommand::line_to(0., viewport.height),
                    PathCommand::close(),
                ],
                ..Default::default()
            },
        });
        root.children = Some(vec![content]);
        let assets = imported
            .assets
            .entries
            .into_iter()
            .map(|(id, a)| {
                let a = a.expect("imported asset is complete");
                (
                    id,
                    Asset::RasterAsset {
                        mime_type: a.mime,
                        data: a.data,
                        pixel_width: a.width,
                        pixel_height: a.height,
                        decoded_bytes: a.decoded_bytes,
                    },
                )
            })
            .collect();
        (
            Asset::VectorAsset {
                view_box: viewport,
                root: Some(std::boxed::Box::new(root)),
            },
            assets,
        )
    }
}
pub(crate) use svg_ownership::{asset as imported_icon_asset, import as import_icon_svg};
