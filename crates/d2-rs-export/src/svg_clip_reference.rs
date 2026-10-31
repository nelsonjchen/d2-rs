// SPDX-License-Identifier: MPL-2.0
use super::*;

// Original d2raster/bands.go setBandReferenceBounds and gradient.go
// subpathPixelBounds. A retained nested viewport owns the union of its child
// ink, clipped to its viewport and parent paint surface. Float32 submission
// uses that full-render origin even when the actual destination is a band.
pub(super) fn intrinsic_clip_paint_bounds(
    group: &usvg::Group,
    parent: Transform,
    scale: f32,
    raw: Option<&[SvgPath]>,
    by_id: Option<&HashMap<String, SvgPath>>,
    start: usize,
    canvas: TextPaintBounds,
) -> Option<TextPaintBounds> {
    fn include(bounds: &mut Option<TextPaintBounds>, points: impl Iterator<Item = Point64>) {
        let mut min = Point64 {
            x: f64::INFINITY,
            y: f64::INFINITY,
        };
        let mut max = Point64 {
            x: f64::NEG_INFINITY,
            y: f64::NEG_INFINITY,
        };
        for point in points {
            min.x = min.x.min(point.x);
            min.y = min.y.min(point.y);
            max.x = max.x.max(point.x);
            max.y = max.y.max(point.y);
        }
        if !min.x.is_finite() {
            return;
        }
        let b = TextPaintBounds {
            left: (min.x - 1.0).floor() as i64,
            top: (min.y - 1.0).floor() as i64,
            right: (max.x + 1.0).ceil() as i64,
            bottom: (max.y + 1.0).ceil() as i64,
        };
        *bounds = Some(bounds.map_or(b, |old| TextPaintBounds {
            left: old.left.min(b.left),
            top: old.top.min(b.top),
            right: old.right.max(b.right),
            bottom: old.bottom.max(b.bottom),
        }));
    }
    fn visit(
        group: &usvg::Group,
        parent: Transform,
        scale: f32,
        raw: Option<&[SvgPath]>,
        by_id: Option<&HashMap<String, SvgPath>>,
        index: &mut usize,
        canvas: TextPaintBounds,
    ) -> Option<TextPaintBounds> {
        let mut bounds = None;
        for node in group.children() {
            match node {
                usvg::Node::Group(child) => {
                    let b = visit(child, parent, scale, raw, by_id, index, canvas)?;
                    bounds = Some(bounds.map_or(b, |old: TextPaintBounds| TextPaintBounds {
                        left: old.left.min(b.left),
                        top: old.top.min(b.top),
                        right: old.right.max(b.right),
                        bottom: old.bottom.max(b.bottom),
                    }));
                }
                usvg::Node::Path(path) => {
                    if !path.is_visible() || (path.fill().is_none() && path.stroke().is_none()) {
                        continue;
                    }
                    let source = by_id
                        .and_then(|m| m.get(path.id()))
                        .or_else(|| raw.and_then(|r| r.get(*index)))?;
                    *index += 1;
                    // Raw-path rendering currently qualifies filled, unstroked
                    // geometry. Preserve the existing fallback for other scenes.
                    if path.fill().is_none() || path.stroke().is_some() {
                        return None;
                    }
                    let transform =
                        intrinsic_svg_transform(source, parent, scale, Point64::default())?;
                    let paths =
                        collect_raw_subpaths(source, flatten_tolerance64(transform), transform)?;
                    include(
                        &mut bounds,
                        paths
                            .iter()
                            .flat_map(|(points, _)| points.iter().copied())
                            .map(|p| map_point(p, transform)),
                    );
                }
                _ => return None,
            }
        }
        let mut bounds = bounds?;
        if let Some((path, clip)) = intrinsic_group_clip(group, raw) {
            let mut transform = path.scene_transform.map_or_else(
                || canonical_transform(parent.post_scale(scale, scale)),
                |m| {
                    let k = f64::from(scale);
                    Affine64 {
                        sx: m[0] * k,
                        ky: m[1] * k,
                        kx: m[2] * k,
                        sy: m[3] * k,
                        tx: m[4] * k,
                        ty: m[5] * k,
                    }
                },
            );
            for m in &clip.transforms {
                transform = source_affine_mul(
                    transform,
                    Affine64 {
                        sx: m[0],
                        ky: m[1],
                        kx: m[2],
                        sy: m[3],
                        tx: m[4],
                        ty: m[5],
                    },
                );
            }
            let mut viewport = None;
            include(
                &mut viewport,
                [
                    (0.0, 0.0),
                    (clip.size[0], 0.0),
                    (clip.size[0], clip.size[1]),
                    (0.0, clip.size[1]),
                ]
                .into_iter()
                .map(|(x, y)| map_point(Point64 { x, y }, transform)),
            );
            bounds = bounds.intersect(viewport?)?;
        }
        bounds.intersect(canvas)
    }
    let mut index = start;
    visit(group, parent, scale, raw, by_id, &mut index, canvas)
}

// Preserve source normalized singular-value arithmetic in f64. SVG's f32
// matrix is not the source flattening transform for retained scene geometry.
pub(super) fn flatten_tolerance64(transform: Affine64) -> f64 {
    let max = transform
        .sx
        .abs()
        .max(transform.ky.abs())
        .max(transform.kx.abs())
        .max(transform.sy.abs());
    if !max.is_finite() {
        return f64::MIN_POSITIVE;
    }
    if max == 0.0 {
        return f64::MAX;
    }
    let normalized = Affine64 {
        sx: transform.sx / max,
        ky: transform.ky / max,
        kx: transform.kx / max,
        sy: transform.sy / max,
        tx: 0.0,
        ty: 0.0,
    };
    let tolerance = 0.25 / affine_max_scale(normalized);
    if max < tolerance / f64::MAX {
        return f64::MAX;
    }
    let result = tolerance / max;
    if result == 0.0 {
        f64::MIN_POSITIVE
    } else {
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn retained_f64_tolerances_match_all_original_go_path_matrices() {
        let rows: Vec<serde_json::Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/svg-clip-tolerance-source-v090.json"))
        .unwrap();
        assert_eq!(rows.len(), 30);
        for row in rows {
            let m = &row["matrix"];
            let transform = Affine64 {
                sx: m["A"].as_f64().unwrap(),
                ky: m["B"].as_f64().unwrap(),
                kx: m["C"].as_f64().unwrap(),
                sy: m["D"].as_f64().unwrap(),
                tx: m["E"].as_f64().unwrap(),
                ty: m["F"].as_f64().unwrap(),
            };
            assert_eq!(
                flatten_tolerance64(transform),
                row["tolerance"].as_f64().unwrap(),
                "source case {} path {}",
                row["case"],
                row["path"]
            );
        }
    }
}
