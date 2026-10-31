// SPDX-License-Identifier: MPL-2.0
//! Original d2raster/pattern.go: per-paint matrices and per-resolution resources.
use super::*;
use std::sync::Arc;

pub(super) type PatternInstances = HashMap<String, PatternPaints>;
pub(super) type PatternPaints = HashMap<[u64; 6], PreparedPattern>;
pub(super) struct PreparedPattern {
    inverse: Affine64,
    tile: Arc<PatternTile>,
}
struct PatternTile {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    pixel_width: usize,
    pixel_height: usize,
    pixels: Vec<u8>,
}
fn key(t: Affine64) -> [u64; 6] {
    [
        t.sx.to_bits(),
        t.ky.to_bits(),
        t.kx.to_bits(),
        t.sy.to_bits(),
        t.tx.to_bits(),
        t.ty.to_bits(),
    ]
}
fn matrix(device: Affine64, p: &usvg::Pattern) -> Affine64 {
    source_affine_mul(device, canonical_transform(p.transform()))
}

pub(super) fn is_prepared(
    p: &usvg::Pattern,
    mut device: Affine64,
    origin: Point64,
    instances: &PatternInstances,
) -> bool {
    device.tx += origin.x;
    device.ty += origin.y;
    instances
        .get(p.id())
        .is_some_and(|paints| paints.contains_key(&key(matrix(device, p))))
}

pub(super) fn color_at(
    p: &usvg::Pattern,
    opacity: f32,
    parsed: Transform,
    exact: Option<Affine64>,
    origin: Point64,
    x: usize,
    y: usize,
    instances: &PatternInstances,
) -> Option<[u8; 4]> {
    let mut device = exact.unwrap_or_else(|| canonical_transform(parsed));
    device.tx += origin.x;
    device.ty += origin.y;
    let prepared = instances.get(p.id())?.get(&key(matrix(device, p)))?;
    let tile = &prepared.tile;
    let px = x as f64 + 0.5 + origin.x;
    let py = y as f64 + 0.5 + origin.y;
    let inv = prepared.inverse;
    let point = Point64 {
        x: inv.sx.mul_add(px, inv.kx * py) + inv.tx,
        y: inv.ky.mul_add(px, inv.sy * py) + inv.ty,
    };
    let lx = wrapped_pattern_coordinate(point.x, tile.x, tile.width)?;
    let ly = wrapped_pattern_coordinate(point.y, tile.y, tile.height)?;
    let px =
        ((lx / tile.width * tile.pixel_width as f64).floor() as usize).min(tile.pixel_width - 1);
    let py =
        ((ly / tile.height * tile.pixel_height as f64).floor() as usize).min(tile.pixel_height - 1);
    let pixel = &tile.pixels[(py * tile.pixel_width + px) * 4..][..4];
    let alpha = pixel[3];
    if alpha == 0 {
        return None;
    }
    let channel = |c: u8| {
        if alpha == 255 {
            c
        } else {
            ((u32::from(c) * 0xffff / u32::from(alpha)) >> 8) as u8
        }
    };
    Some([
        channel(pixel[0]),
        channel(pixel[1]),
        channel(pixel[2]),
        (f32::from(alpha) * opacity).clamp(0.0, 255.0) as u8,
    ])
}

pub(super) fn prepare(
    tree: &Tree,
    scale: u32,
    map: &mut RasterSourceMap<'_>,
    sources: &SvgImagePathSources,
) -> Option<()> {
    let mut requests = Vec::new();
    fn visit(
        group: &usvg::Group,
        scale: u32,
        map: &RasterSourceMap<'_>,
        out: &mut Vec<(Arc<usvg::Pattern>, Affine64)>,
    ) {
        if let Some(mask) = group.mask() {
            visit(mask.root(), scale, map, out);
        }
        if let Some(clip) = group.clip_path() {
            visit(clip.root(), scale, map, out);
        }
        for node in group.children() {
            match node {
                usvg::Node::Path(path) => {
                    let device = map
                        .native_path_transforms
                        .get(&(path.as_ref() as *const usvg::Path))
                        .copied()
                        .unwrap_or_else(|| {
                            canonical_transform(
                                path.abs_transform().post_scale(scale as f32, scale as f32),
                            )
                        });
                    for paint in [
                        path.fill().map(|v| v.paint()),
                        path.stroke().map(|v| v.paint()),
                    ]
                    .into_iter()
                    .flatten()
                    {
                        if let Paint::Pattern(p) = paint {
                            out.push((p.clone(), device));
                        }
                    }
                }
                usvg::Node::Group(g) => visit(g, scale, map, out),
                usvg::Node::Text(t) => visit(t.flattened(), scale, map, out),
                usvg::Node::Image(_) => {}
            }
        }
    }
    visit(tree.root(), scale, map, &mut requests);
    if requests.is_empty() {
        return Some(());
    }
    // Callers supplying only a parsed usvg tree have already chosen f32 input.
    // Materialize the parser's normalized resource XML at its maximum
    // supported decimal precision. Normal D2 rendering retains the original
    // document and never takes this metadata-less compatibility branch.
    let fallback;
    let document = if let Some(source) = sources.pattern_document() {
        source
    } else {
        fallback = tree.to_string(&usvg::WriteOptions {
            coordinates_precision: 12,
            transforms_precision: 12,
            preserve_text: true,
            ..Default::default()
        });
        &fallback
    };
    let mut tiles: HashMap<(String, usize, usize), Arc<PatternTile>> = HashMap::new();
    let mut tile_bytes = 0usize;
    for (p, device) in requests {
        let transform = matrix(device, &p);
        let signature = key(transform);
        if map
            .pattern_tiles
            .get(p.id())
            .is_some_and(|v| v.contains_key(&signature))
        {
            continue;
        }
        let rect = p.rect();
        let width = f64::from(rect.width());
        let height = f64::from(rect.height());
        let extent_x = (transform.sx * width).hypot(transform.ky * width).ceil();
        let extent_y = (transform.kx * height).hypot(transform.sy * height).ceil();
        if !extent_x.is_finite()
            || !extent_y.is_finite()
            || extent_x <= 0.0
            || extent_y <= 0.0
            || extent_x >= usize::MAX as f64
            || extent_y >= usize::MAX as f64
        {
            return None;
        }
        let (pw, ph) = (extent_x as usize, extent_y as usize);
        let tile_key = (p.id().to_owned(), pw, ph);
        let tile = if let Some(tile) = tiles.get(&tile_key) {
            tile.clone()
        } else {
            let bytes = pw.checked_mul(ph)?.checked_mul(4)?;
            tile_bytes = tile_bytes.checked_add(bytes)?;
            if tile_bytes > 256 * 1024 * 1024 {
                return None;
            }
            let xml = resource_document(document, p.id(), pw, ph, width, height)?;
            let (tagged, mut resource_sources) =
                crate::svg_precision::tag_embedded_svg_images(&xml);
            resource_sources.inherit_pattern_paths(sources);
            let options = usvg::Options {
                fontdb: tree.fontdb().clone(),
                ..Default::default()
            };
            let resource_tree = Tree::from_str(&tagged, &options).ok()?;
            let (rw, rh, pixels) = render_simple_with_sources_and_background(
                &resource_tree,
                1,
                &[],
                &resource_sources,
                [0; 4],
            )?;
            if rw as usize != pw || rh as usize != ph || pixels.len() != bytes {
                return None;
            }
            #[cfg(test)]
            if let Some(root) = std::env::var_os("D2_RS_TEST_PATTERN_OUTPUT") {
                let root = std::path::PathBuf::from(root);
                std::fs::create_dir_all(&root).unwrap();
                let stem = format!(
                    "tile-{}-{pw}x{ph}-{:x}",
                    p.id(),
                    <sha2::Sha256 as sha2::Digest>::digest(xml.as_bytes())
                );
                for (extension, bytes) in [("rgba", pixels.as_slice()), ("svg", xml.as_bytes())] {
                    let file = root.join(format!("{stem}.{extension}"));
                    if file.exists() {
                        assert_eq!(std::fs::read(file).unwrap(), bytes);
                    } else {
                        std::fs::write(file, bytes).unwrap();
                    }
                }
            }
            let tile = Arc::new(PatternTile {
                x: f64::from(rect.x()),
                y: f64::from(rect.y()),
                width,
                height,
                pixel_width: pw,
                pixel_height: ph,
                pixels,
            });
            tiles.insert(tile_key, tile.clone());
            tile
        };
        map.pattern_tiles
            .entry(p.id().to_owned())
            .or_default()
            .insert(
                signature,
                PreparedPattern {
                    inverse: {
                        let a = d2_rs_math::svg_gradient::finite_inverse([
                            transform.sx,
                            transform.ky,
                            transform.kx,
                            transform.sy,
                            transform.tx,
                            transform.ty,
                        ])?;
                        Affine64 {
                            sx: a[0],
                            ky: a[1],
                            kx: a[2],
                            sy: a[3],
                            tx: a[4],
                            ty: a[5],
                        }
                    },
                    tile,
                },
            );
    }
    Some(())
}

/// Source prepares a pattern root in a separate pixel surface, scaled by the
/// rounded tile dimensions. Preserve original XML/precision metadata rather
/// than serializing the main scene's already narrowed geometry.
fn resource_document(
    source: &str,
    id: &str,
    pw: usize,
    ph: usize,
    width: f64,
    height: f64,
) -> Option<String> {
    let doc = roxmltree::Document::parse(source).ok()?;
    let pattern = doc
        .descendants()
        .find(|n| n.has_tag_name("pattern") && n.attribute("id") == Some(id))?;
    let body = pattern
        .children()
        .filter(|n| n.is_element())
        .map(|n| &source[n.range()])
        .collect::<String>();
    if body.is_empty() {
        return None;
    }
    let defs = doc
        .descendants()
        .filter(|n| {
            n.has_tag_name("defs") && !n.ancestors().skip(1).any(|a| a.has_tag_name("defs"))
        })
        .map(|n| {
            let range = n.range();
            let selected = pattern.range();
            let mut xml = source[range.clone()].to_owned();
            if range.start <= selected.start && selected.end <= range.end {
                xml.replace_range(selected.start - range.start..selected.end - range.start, "");
            }
            xml
        })
        .collect::<String>();
    let styles = doc
        .descendants()
        .filter(|n| n.has_tag_name("style"))
        .map(|n| &source[n.range()])
        .collect::<String>();
    let class = doc.root_element().attribute("class").unwrap_or_default();
    let identities = doc
        .descendants()
        .filter_map(|n| n.attribute("id"))
        .collect::<std::collections::HashSet<_>>();
    let mut suffix = 0;
    while identities.contains(format!("d2-rs-native-pattern-root-{suffix}").as_str())
        || identities.contains(format!("d2-rs-native-canvas-viewport-pattern-{suffix}").as_str())
    {
        suffix += 1;
    }
    let sx = pw as f64 / width;
    let sy = ph as f64 / height;
    Some(format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" class="{}" width="{pw}" height="{ph}" viewBox="0 0 {pw} {ph}" preserveAspectRatio="xMinYMin meet">{defs}{styles}<svg id="d2-rs-native-canvas-viewport-pattern-{suffix}" class="d2-svg" data-d2-rs-canvas-reference="ordinary" width="{pw}" height="{ph}" viewBox="0 0 {pw} {ph}"><g id="d2-rs-native-pattern-root-{suffix}" transform="matrix({sx} 0 0 {sy} 0 0)">{body}</g></svg></svg>"##,
        class
            .replace('&', "&amp;")
            .replace('"', "&quot;")
            .replace('<', "&lt;")
    ))
}

#[cfg(test)]
mod source_contract {
    use super::*;
    use sha2::{Digest, Sha256};
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_go_whole_frames_cover_all_builtin_patterns_and_affine_tiles() {
        use d2_rs_core::{Graph, LayoutOptions, Node, Size};
        use d2_rs_language::compiler::SourceNodeStyle;
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/pattern-transforms-source-v090.json"))
        .unwrap();
        let mut rows = rows.as_array().unwrap().clone();
        let doubles: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/pattern-double-border-source-v090.json"))
        .unwrap();
        rows.extend(doubles.as_array().unwrap().iter().cloned());
        assert_eq!(rows.len(), 80);
        let capture = std::env::var_os("D2_RS_TEST_PATTERN_OUTPUT").map(std::path::PathBuf::from);
        if let Some(root) = &capture {
            std::fs::create_dir_all(root).unwrap();
        }
        let mut templates = HashMap::new();
        for (double, kind) in [false, true]
            .into_iter()
            .flat_map(|double| ["dots", "lines", "grain", "paper"].map(|kind| (double, kind)))
        {
            let mut graph = Graph::default();
            let id = graph.add_node(Node::new(
                "probe",
                Size {
                    width: 120.0,
                    height: 80.0,
                },
            ));
            let mut layout = d2_tala_rs::Engine
                .layout(&graph, &LayoutOptions::default())
                .unwrap();
            layout.boxes.insert(
                id,
                d2_rs_core::Rect {
                    origin: d2_rs_core::Point::default(),
                    size: Size {
                        width: 120.0,
                        height: 80.0,
                    },
                },
            );
            let styles = BTreeMap::from([(
                id,
                SourceNodeStyle {
                    fill: Some("#f7f8fe".into()),
                    stroke: Some(if double { "#000410" } else { "none" }.into()),
                    stroke_width: Some(if double { "2" } else { "0" }.into()),
                    double_border: double.then(|| "true".into()),
                    fill_pattern: Some(kind.into()),
                    ..Default::default()
                },
            )]);
            let (svg, _) = d2_rs_render::render_svg_with_raster_sources_checked(
                &graph,
                &layout,
                &BTreeMap::new(),
                &styles,
                Default::default(),
                false,
            )
            .unwrap();
            let svg = crate::strip_embedded_font_faces(std::str::from_utf8(&svg).unwrap());
            let doc = roxmltree::Document::parse(&svg).unwrap();
            let defs = doc
                .descendants()
                .filter(|n| {
                    n.has_tag_name("defs") && !n.ancestors().skip(1).any(|a| a.has_tag_name("defs"))
                })
                .map(|n| &svg[n.range()])
                .collect::<String>();
            let css = doc
                .descendants()
                .filter(|n| n.has_tag_name("style"))
                .map(|n| &svg[n.range()])
                .collect::<String>();
            let pattern_id = doc
                .descendants()
                .find(|n| n.has_tag_name("pattern"))
                .unwrap()
                .attribute("id")
                .unwrap()
                .to_owned();
            let body = double.then(|| {
                let node = doc
                    .descendants()
                    .find(|n| n.has_tag_name("g") && n.attribute("class") == Some("shape"))
                    .expect("actual generated shape geometry");
                svg[node.range()].to_owned()
            });
            templates.insert((kind, double), (defs, css, pattern_id, body));
        }
        let mut mismatches = Vec::new();
        for row in &rows {
            let name = row["name"].as_str().unwrap();
            let kind = row["pattern"].as_str().unwrap();
            let double = name.starts_with("double-");
            let (defs, css, pid, body) = &templates[&(kind, double)];
            let fallback = format!(
                r##"<rect x="0" y="0" width="120" height="80" fill="#f7f8fe"/><rect x="0" y="0" width="120" height="80" fill="url(#{pid})" class="{kind}-overlay"/>"##
            );
            let body = body.as_deref().unwrap_or(&fallback);
            let m = row["matrix"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_f64().unwrap().to_string())
                .collect::<Vec<_>>()
                .join(" ");
            let svg = format!(
                r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" class="d2-rs-theme" width="192" height="128" viewBox="0 0 192 128" preserveAspectRatio="xMinYMin meet">{defs}{css}<svg id="d2-rs-native-canvas-viewport-pattern-test" class="d2-svg" data-d2-rs-canvas-reference="ordinary" width="192" height="128" viewBox="0 0 192 128"><g id="d2-rs-native-pattern-test-shape" stroke-linecap="butt" stroke-linejoin="round" transform="matrix({m})">{body}</g></svg></svg>"##
            );
            let (tagged, sources) = crate::svg_precision::tag_embedded_svg_images(&svg);
            let tree = Tree::from_str(&tagged, &usvg::Options::default()).unwrap();
            if let Some(root) = &capture {
                std::fs::write(root.join(format!("{name}.svg")), &svg).unwrap();
            }
            let Some((w, h, pixels)) = render_simple_with_sources(
                &tree,
                row["scale"].as_u64().unwrap() as u32,
                &[],
                &sources,
            ) else {
                mismatches.push((name.to_owned(), "native backend unsupported".into()));
                continue;
            };
            assert_eq!(
                (w, h),
                (
                    row["width"].as_u64().unwrap() as u32,
                    row["height"].as_u64().unwrap() as u32
                )
            );
            if let Some(root) = &capture {
                std::fs::write(root.join(format!("{name}.rgba")), &pixels).unwrap();
            }
            let digest = format!("{:x}", Sha256::digest(&pixels));
            if digest != row["rgba_sha256"].as_str().unwrap() {
                mismatches.push((name.to_owned(), digest));
            }
        }
        assert!(
            mismatches.is_empty(),
            "Whole source pattern frame mismatches: {mismatches:?}"
        );
    }
}
