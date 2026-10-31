// SPDX-License-Identifier: MPL-2.0
//! Source d2raster/colrv1.go, gradient.go and color_space.go paint pipeline.
use super::*;
use crate::colr_plan::{Paint as ColorPaint, Plan, Stop};
const IDENTITY: Affine64 = Affine64 {
    sx: 1.0,
    sy: 1.0,
    kx: 0.0,
    ky: 0.0,
    tx: 0.0,
    ty: 0.0,
};
struct Node {
    kind: Kind,
    bounds: [f64; 4],
}
enum Kind {
    Leaf {
        paths: Vec<Vec<Point64>>,
        paint: Shader,
    },
    Layers(Vec<Node>),
    Composite {
        source: Box<Node>,
        backdrop: Box<Node>,
        mode: u8,
    },
    Clip {
        child: Box<Node>,
        points: Vec<Point64>,
    },
}
struct Shader {
    paint: ColorPaint,
    inverse: Affine64,
}
fn matrix(m: [f64; 6]) -> Affine64 {
    Affine64 {
        sx: m[0],
        ky: m[1],
        kx: m[2],
        sy: m[3],
        tx: m[4],
        ty: m[5],
    }
}
fn extent(paths: &[Vec<Point64>], w: usize, h: usize) -> [f64; 4] {
    let mut b = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for p in paths.iter().flatten() {
        b[0] = b[0].min(p.x);
        b[1] = b[1].min(p.y);
        b[2] = b[2].max(p.x);
        b[3] = b[3].max(p.y);
    }
    [
        (b[0] - 1.0).floor().max(0.0),
        (b[1] - 1.0).floor().max(0.0),
        (b[2] + 1.0).ceil().min(w as f64),
        (b[3] + 1.0).ceil().min(h as f64),
    ]
}
fn intersect(a: [f64; 4], b: [f64; 4]) -> [f64; 4] {
    [
        a[0].max(b[0]),
        a[1].max(b[1]),
        a[2].min(b[2]),
        a[3].min(b[3]),
    ]
}
fn union(a: [f64; 4], b: [f64; 4]) -> [f64; 4] {
    [
        a[0].min(b[0]),
        a[1].min(b[1]),
        a[2].max(b[2]),
        a[3].max(b[3]),
    ]
}
fn rectangle(b: [f64; 4], t: Affine64) -> Vec<Point64> {
    [(b[0], b[1]), (b[2], b[1]), (b[2], b[3]), (b[0], b[3])]
        .into_iter()
        .map(|(x, y)| map_point(Point64 { x, y }, t))
        .collect()
}
fn shader(p: &ColorPaint, t: Affine64) -> Option<Shader> {
    match p {
        ColorPaint::Transform { matrix: m, paint } => shader(paint, affine_mul(t, matrix(*m))),
        ColorPaint::Solid { color } => Some(Shader {
            paint: ColorPaint::Solid { color: *color },
            inverse: IDENTITY,
        }),
        ColorPaint::Linear { stops, points } => Some(Shader {
            paint: ColorPaint::Linear {
                stops: copy_stops(stops),
                points: *points,
            },
            inverse: affine_inverse(t)?,
        }),
        ColorPaint::Radial { stops, circles } => Some(Shader {
            paint: ColorPaint::Radial {
                stops: copy_stops(stops),
                circles: *circles,
            },
            inverse: affine_inverse(t)?,
        }),
        _ => None,
    }
}
fn copy_stops(s: &[Stop]) -> Vec<Stop> {
    s.iter()
        .map(|s| Stop {
            offset: s.offset,
            color: s.color,
        })
        .collect()
}
fn compile(
    p: &ColorPaint,
    current: Affine64,
    base: Affine64,
    clip: Option<[f64; 4]>,
    face: &ttf_parser::Face<'_>,
    w: usize,
    h: usize,
) -> Option<Node> {
    let t = affine_mul(base, current);
    Some(match p {
        ColorPaint::Layers { paints } => {
            let children = paints
                .iter()
                .map(|p| compile(p, current, base, clip, face, w, h))
                .collect::<Option<Vec<_>>>()?;
            let bounds = children
                .iter()
                .map(|c| c.bounds)
                .reduce(union)
                .unwrap_or([0.0; 4]);
            Node {
                kind: Kind::Layers(children),
                bounds,
            }
        }
        ColorPaint::Transform { matrix: m, paint } => compile(
            paint,
            affine_mul(current, matrix(*m)),
            base,
            clip,
            face,
            w,
            h,
        )?,
        ColorPaint::Glyph { glyph_id, paint } => {
            let raw = raw_go_glyph(face, u16::try_from(*glyph_id).ok()?, 0)?;
            let path = fixed_path_from_raw(&raw, IDENTITY, 1.0, 1.0);
            let tolerance = 0.25 / affine_max_scale(t);
            let paths = collect_fixed_subpaths(&path, tolerance)
                .into_iter()
                .map(|(p, _)| p.into_iter().map(|p| map_point(p, t)).collect())
                .collect::<Vec<_>>();
            let bounds = extent(&paths, w, h);
            Node {
                kind: Kind::Leaf {
                    paths,
                    paint: shader(paint, t)?,
                },
                bounds,
            }
        }
        ColorPaint::Composite {
            source,
            backdrop,
            mode,
        } => {
            let backdrop = Box::new(compile(backdrop, current, base, clip, face, w, h)?);
            let source = Box::new(compile(source, current, base, clip, face, w, h)?);
            let bounds = if *mode == 0 {
                intersect(source.bounds, backdrop.bounds)
            } else {
                union(source.bounds, backdrop.bounds)
            };
            Node {
                kind: Kind::Composite {
                    source,
                    backdrop,
                    mode: *mode,
                },
                bounds,
            }
        }
        _ => {
            let paths = vec![rectangle(clip?, base)];
            let bounds = extent(&paths, w, h);
            Node {
                kind: Kind::Leaf {
                    paths,
                    paint: shader(p, t)?,
                },
                bounds,
            }
        }
    })
}
fn decode(v: f64) -> f64 {
    let v = v.clamp(0.0, 1.0);
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}
fn encode(v: f64) -> f64 {
    let v = v.clamp(0.0, 1.0);
    if v <= 0.0031308 {
        12.92 * v
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}
fn byte(v: f64) -> u8 {
    v.round().clamp(0.0, 255.0) as u8
}
fn interpolate(stops: &[Stop], v: f64) -> [u8; 4] {
    if stops.len() == 1 || v < stops[0].offset {
        return stops[0].color;
    }
    if v >= stops.last().unwrap().offset {
        return stops.last().unwrap().color;
    }
    let right = stops.partition_point(|s| s.offset <= v);
    let (a, b) = (&stops[right - 1], &stops[right]);
    let amount = (v - a.offset) / (b.offset - a.offset);
    let aa = f64::from(a.color[3]) / 255.0;
    let ba = f64::from(b.color[3]) / 255.0;
    let alpha = aa + (ba - aa) * amount;
    let mut c = [0, 0, 0, byte(alpha * 255.0)];
    if alpha <= 0.0 {
        return c;
    }
    for i in 0..3 {
        let left = decode(f64::from(a.color[i]) / 255.0) * aa;
        let right = decode(f64::from(b.color[i]) / 255.0) * ba;
        c[i] = byte(encode((left + (right - left) * amount) / alpha) * 255.0);
    }
    c
}
pub(crate) fn radial(c: [f64; 6], p: Point64) -> Option<f64> {
    let (dx, dy, dr) = (c[3] - c[0], c[4] - c[1], c[5] - c[2]);
    let (qx, qy) = (p.x - c[0], p.y - c[1]);
    let a = dx * dx + dy * dy - dr * dr;
    let cc = qx * qx + qy * qy - c[2] * c[2];
    let b = qx * dx + qy * dy + c[2] * dr;
    let valid = |t: f64| t.is_finite() && (c[2] + t * dr) >= -1e-12;
    if dx == 0.0 && dy == 0.0 && c[2] == 0.0 {
        let root = (-a * cc).sqrt();
        return [root / a, -root / a].into_iter().find(|t| valid(*t));
    }
    if a.abs() <= 1e-14 * (dx * dx + dy * dy + dr * dr + 1.0) {
        if b.abs() <= 1e-14 * ((qx * dx).abs() + (qy * dy).abs() + (c[2] * dr).abs() + 1.0) {
            return None;
        }
        let t = 0.5 * cc / b;
        return valid(t).then_some(t);
    }
    let mut d = b * b - a * cc;
    if d < 0.0 {
        if d >= -1e-14 * ((b * b).abs() + (a * cc).abs() + 1.0) {
            d = 0.0;
        } else {
            return None;
        }
    }
    let root = d.sqrt();
    [(b + root) / a, (b - root) / a]
        .into_iter()
        .find(|t| valid(*t))
}
impl Shader {
    fn sample(&self, x: usize, y: usize) -> Option<[u8; 4]> {
        let p = map_point(
            Point64 {
                x: x as f64 + 0.5,
                y: y as f64 + 0.5,
            },
            self.inverse,
        );
        let (stops, v) = match &self.paint {
            ColorPaint::Solid { color } => return Some(*color),
            ColorPaint::Linear { stops, points: a } => {
                let (vx, vy, rx, ry) = (a[2] - a[0], a[3] - a[1], a[4] - a[0], a[5] - a[1]);
                let r2 = rx * rx + ry * ry;
                if r2 == 0.0 {
                    return None;
                }
                let projection = (vx * rx + vy * ry) / r2;
                let (wx, wy) = (vx - projection * rx, vy - projection * ry);
                let den = wx * wx + wy * wy;
                if den <= 1e-18 {
                    return None;
                }
                (stops, ((p.x - a[0]) * wx + (p.y - a[1]) * wy) / den)
            }
            ColorPaint::Radial { stops, circles } => (stops, radial(*circles, p)?),
            _ => return None,
        };
        v.is_finite().then(|| interpolate(stops, v.clamp(0.0, 1.0)))
    }
}
pub(super) fn merge(dst: &mut [u8], src: &[u8]) {
    for (d, s) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
        if s[3] != 0 {
            blend_premultiplied_pixel(d, s.try_into().unwrap());
        }
    }
}
fn soft(dst: &mut [u8], src: &[u8]) {
    for (d, s) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
        let sa = f64::from(s[3]) / 255.0;
        let da = f64::from(d[3]) / 255.0;
        if sa == 0.0 {
            continue;
        }
        let alpha = sa + da * (1.0 - sa);
        let ab = byte(alpha * 255.0);
        for i in 0..3 {
            let sl = decode(f64::from(s[i]) / f64::from(s[3]));
            let dl = if d[3] == 0 {
                0.0
            } else {
                decode(f64::from(d[i]) / f64::from(d[3]))
            };
            let mixed = if sl <= 0.5 {
                dl - (1.0 - 2.0 * sl) * dl * (1.0 - dl)
            } else {
                let g = if dl <= 0.25 {
                    ((16.0 * dl - 12.0) * dl + 4.0) * dl
                } else {
                    dl.sqrt()
                };
                dl + (2.0 * sl - 1.0) * (g - dl)
            };
            let v = sa * (1.0 - da) * sl + sa * da * mixed + da * (1.0 - sa) * dl;
            d[i] = byte(encode(v / alpha) * alpha * 255.0).min(ab);
        }
        d[3] = ab;
    }
}
fn render(node: &Node, dst: &mut [u8], w: usize, h: usize, reference: Point64) {
    match &node.kind {
        Kind::Leaf { paths, paint } => {
            let mut r = Rasterizer::new(w, h, 0.375);
            r.set_reference_origin(reference);
            for p in paths {
                r.add_fill_polyline(p, true);
            }
            // Source finishPrimitive dispatches solid colors to DrawRGBA's
            // wide-alpha kernel; gradient samples use the byte compositor.
            if let ColorPaint::Solid { color } = &paint.paint {
                r.paint(dst, *color);
            } else {
                r.paint_with_go_blend(dst, |x, y| paint.sample(x, y));
            }
        }
        Kind::Layers(children) => {
            for c in children {
                render(c, dst, w, h, reference)
            }
        }
        Kind::Composite {
            source,
            backdrop,
            mode,
        } => {
            let reference = Point64 {
                x: node.bounds[0],
                y: node.bounds[1],
            };
            let mut a = vec![0; dst.len()];
            let mut b = vec![0; dst.len()];
            render(backdrop, &mut b, w, h, reference);
            render(source, &mut a, w, h, reference);
            if *mode == 0 {
                for (s, m) in a.chunks_exact_mut(4).zip(b.chunks_exact(4)) {
                    scale_premultiplied(s, m[3]);
                }
                merge(dst, &a);
            } else {
                soft(&mut b, &a);
                merge(dst, &b);
            }
        }
        Kind::Clip { child, points } => {
            let reference = Point64 {
                x: node.bounds[0],
                y: node.bounds[1],
            };
            let mut layer = vec![0; dst.len()];
            let mut mask = vec![0; dst.len()];
            render(child, &mut layer, w, h, reference);
            let mut r = Rasterizer::new(w, h, 0.375);
            r.set_reference_origin(reference);
            r.add_fill_polyline(points, true);
            r.paint(&mut mask, [255; 4]);
            for (p, m) in layer.chunks_exact_mut(4).zip(mask.chunks_exact(4)) {
                scale_premultiplied(p, m[3]);
            }
            merge(dst, &layer);
        }
    }
}
pub(super) fn glyph(
    plan: &Plan,
    face: &ttf_parser::Face<'_>,
    base: Affine64,
    w: usize,
    h: usize,
) -> Option<Vec<u8>> {
    let mut node = compile(&plan.root, IDENTITY, base, plan.clip, face, w, h)?;
    if let Some(b) = plan.clip {
        let points = rectangle(b, base);
        let cb = extent(&[points.clone()], w, h);
        node = Node {
            bounds: intersect(node.bounds, cb),
            kind: Kind::Clip {
                child: Box::new(node),
                points,
            },
        };
    }
    let mut pixels = vec![0; w.checked_mul(h)?.checked_mul(4)?];
    render(&node, &mut pixels, w, h, Point64 { x: 0.0, y: 0.0 });
    Some(pixels)
}

pub(super) fn plain(paths: &[Vec<Point64>], color: [u8; 4], pixels: &mut [u8], w: usize, h: usize) {
    let mut rasterizer = Rasterizer::new(w, h, 0.375);
    for path in paths {
        rasterizer.add_fill_polyline(path, true);
    }
    rasterizer.paint(pixels, color);
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use base64::Engine as _;
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn complete_bundled_color_glyph_frames_match_pristine_source() {
        compare_frames(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/colrv1-paint-source-v090.json"
        ));
    }
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn complete_transformed_color_glyph_frames_match_pristine_source() {
        compare_frames(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/colrv1-paint-transform-source-v090.json"
        ));
    }
    #[cfg(feature = "oracle-fixtures")]
    fn compare_frames(fixture: &str) {
        let rows: serde_json::Value = serde_json::from_str(fixture).unwrap();
        let mut database = std::sync::Arc::new(usvg::fontdb::Database::new());
        let id = crate::bundled_emoji::Resolver::default()
            .select('😀', &[], &mut database)
            .unwrap();
        let data = database
            .with_face_data(id, |bytes, _| bytes.to_vec())
            .unwrap();
        let face = ttf_parser::Face::parse(&data, 0).unwrap();
        let compiler = crate::colr_plan::Compiler::new(&data).unwrap();
        let mut failures = Vec::new();
        for row in rows.as_array().unwrap() {
            assert_eq!(row["error"], "");
            let gid = row["glyph_id"].as_u64().unwrap() as u32;
            let size = (row["size"].as_f64().unwrap() * 64.0).round() / 64.0;
            let units = f64::from(face.units_per_em());
            let base = Affine64 {
                sx: size / units,
                sy: -size / units,
                kx: 0.0,
                ky: 0.0,
                tx: 8.25,
                ty: 48.75,
            };
            let base = if let Some(values) = row["matrix"].as_array() {
                let m: [f64; 6] = std::array::from_fn(|i| values[i].as_f64().unwrap());
                affine_mul(matrix(m), base)
            } else {
                base
            };
            let plan = compiler.compile(gid).unwrap().unwrap();
            let mut actual = glyph(&plan, &face, base, 96, 96).unwrap();
            crate::unpremultiply_rgba(&mut actual);
            let expected = base64::engine::general_purpose::STANDARD
                .decode(row["pixels"].as_str().unwrap())
                .unwrap();
            assert_eq!(actual.len(), expected.len());
            let different = actual
                .chunks_exact(4)
                .zip(expected.chunks_exact(4))
                .filter(|(a, b)| a != b)
                .count();
            if different != 0 {
                let first = actual
                    .chunks_exact(4)
                    .zip(expected.chunks_exact(4))
                    .enumerate()
                    .filter(|(_, (a, b))| a != b)
                    .take(4)
                    .map(|(i, (a, b))| (i % 96, i / 96, a.to_vec(), b.to_vec()))
                    .collect::<Vec<_>>();
                failures.push((gid, row["size"].as_f64().unwrap(), different, first));
            }
        }
        assert!(
            failures.is_empty(),
            "complete Source frame differences: {failures:?}"
        );
    }
}
