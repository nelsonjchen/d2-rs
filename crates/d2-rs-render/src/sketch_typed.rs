// SPDX-License-Identifier: MPL-2.0
//! d2scenebuild/sketch_shapes.go: typed path flattening and rough paint order.
use super::*;

fn midpoint(a: Point, b: Point) -> Point {
    Point {
        x: (a.x + b.x) / 2.0,
        y: (a.y + b.y) / 2.0,
    }
}
fn distance_squared(p: Point, a: Point, b: Point) -> f64 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let denominator = dx * dx + dy * dy;
    if denominator == 0.0 {
        return (p.x - a.x) * (p.x - a.x) + (p.y - a.y) * (p.y - a.y);
    }
    let numerator = dy * p.x - dx * p.y + b.x * a.y - b.y * a.x;
    numerator * numerator / denominator
}
fn cubic(a: Point, b: Point, c: Point, d: Point, depth: usize, out: &mut Vec<Point>) {
    if depth >= 8 || (distance_squared(b, a, d) <= 0.1225 && distance_squared(c, a, d) <= 0.1225) {
        out.push(d);
        return;
    }
    let (ab, bc, cd) = (midpoint(a, b), midpoint(b, c), midpoint(c, d));
    let (abc, bcd) = (midpoint(ab, bc), midpoint(bc, cd));
    let mid = midpoint(abc, bcd);
    cubic(a, ab, abc, mid, depth + 1, out);
    cubic(mid, bcd, cd, d, depth + 1, out);
}

/// Input is the source-shaped, chopped M/L/C geometry; H/V are an equivalent
/// spelling emitted by PathBuilder. Only adjacent duplicate vertices vanish.
fn flatten(data: &str) -> Option<Vec<(Vec<Point>, bool)>> {
    use svgtypes::PathSegment::*;
    let mut out = Vec::new();
    let mut points: Vec<Point> = Vec::new();
    let mut closed = false;
    let mut current = Point::default();
    let mut start = current;
    for segment in svgtypes::PathParser::from(data) {
        let mut next = Vec::new();
        match segment.ok()? {
            MoveTo { abs: true, x, y } => {
                if !points.is_empty() {
                    out.push((std::mem::take(&mut points), closed));
                }
                closed = false;
                start = Point { x, y };
                next.push(start);
            }
            LineTo { abs: true, x, y } => {
                if points.is_empty() {
                    return None;
                }
                next.push(Point { x, y });
            }
            HorizontalLineTo { abs: true, x } => {
                if points.is_empty() {
                    return None;
                }
                next.push(Point { x, y: current.y });
            }
            VerticalLineTo { abs: true, y } => {
                if points.is_empty() {
                    return None;
                }
                next.push(Point { x: current.x, y });
            }
            CurveTo {
                abs: true,
                x1,
                y1,
                x2,
                y2,
                x,
                y,
            } => {
                if points.is_empty() {
                    return None;
                }
                cubic(
                    current,
                    Point { x: x1, y: y1 },
                    Point { x: x2, y: y2 },
                    Point { x, y },
                    0,
                    &mut next,
                );
            }
            ClosePath { .. } => {
                if points.is_empty() {
                    return None;
                }
                closed = true;
                current = start;
            }
            _ => return None,
        }
        for p in next {
            if !p.x.is_finite() || !p.y.is_finite() {
                return None;
            }
            current = p;
            if points.last() != Some(&p) {
                points.push(p);
            }
        }
    }
    if !points.is_empty() {
        out.push((points, closed));
    }
    (!out.is_empty()).then_some(out)
}

pub(crate) fn draw_typed(
    svg: &mut String,
    shape: ShapeKind,
    rect: Rect,
    style: &SourceNodeStyle,
    palette: PaletteView<'_>,
    mask_id: Option<&str>,
    streaks: &mut BTreeSet<&'static str>,
    native_scene: bool,
) -> bool {
    if style_flag(Some(style), |s| s.three_dee.as_ref())
        || style_flag(Some(style), |s| s.double_border.as_ref())
    {
        return false;
    }
    let Some(data) = shapes::typed_sketch_paths(shape, rect) else {
        return false;
    };
    // Complete preparation precedes painting, so an unsupported operand can
    // never leave a half-painted native shape in the output.
    let Some(parts) = data.iter().map(|d| flatten(d)).collect::<Option<Vec<_>>>() else {
        return false;
    };
    let options = d2_rs_sketch::SketchOptions {
        seed: 1,
        bowing: 2.0,
        ..Default::default()
    };
    let mut paths = Vec::new();
    for (points, closed) in parts.into_iter().flatten() {
        if points.len() < 2 {
            return false;
        }
        if closed {
            let pair = d2_rs_sketch::rough_solid_polygon_paths(&points, options);
            if !pair.fill.is_empty() {
                paths.push(pair.fill);
            }
            paths.push(pair.outline);
        } else {
            paths.push(d2_rs_sketch::rough_linear_path(&points, options));
        }
    }
    if native_scene {
        for path in &mut paths {
            *path = structured_scene_path(path);
        }
    }
    let shadow = style_flag(Some(style), |s| s.shadow.as_ref());
    if shadow {
        svg.push_str("<g filter=\"url(#d2-rs-shadow-filter)\">");
    }
    if style_flag(Some(style), |s| s.multiple.as_ref()) {
        let mut back = style.clone();
        back.multiple = None;
        back.shadow = None;
        back.fill_pattern = None;
        svg.push_str("<g stroke-linecap=\"butt\" stroke-linejoin=\"round\">");
        shapes::draw_shape(
            svg,
            shape,
            Rect {
                origin: Point {
                    x: rect.origin.x + 10.0,
                    y: rect.origin.y - 10.0,
                },
                ..rect
            },
            &back,
            palette,
        );
        svg.push_str("</g>");
    }
    let mut base = style.clone();
    base.multiple = None;
    base.shadow = None;
    base.fill_pattern = None;
    let (attrs, _) = shapes::attributes(&base, palette);
    let mask = mask_id
        .map(|id| format!(" mask=\"url(#{})\"", escape(id)))
        .unwrap_or_default();
    for path in &paths {
        let _ = writeln!(
            svg,
            "<g{mask}><path data-d2-rs-sketch-set=\"rough\" d=\"{}\" {attrs} stroke-linecap=\"round\" stroke-linejoin=\"round\"/></g>",
            escape(path)
        );
        if let Some(pattern) = style.fill_pattern.as_deref().and_then(patternassets::id) {
            let _ = writeln!(
                svg,
                "<g style=\"mix-blend-mode:multiply\"{mask}><path data-d2-rs-sketch-set=\"rough\" d=\"{}\" fill=\"url(#{pattern})\" stroke=\"none\"/></g>",
                escape(path)
            );
        }
    }
    let fill = themed_color(style.fill.as_deref(), "none", palette);
    if let Some(streak) = streak_style(&fill) {
        if streaks.insert(streak.category) {
            svg.push_str(&streak_definition(streak));
        }
        for path in &paths {
            let _ = writeln!(
                svg,
                "<g class=\"sketch-streak-overlay sketch-streak-{}\" style=\"mix-blend-mode:{}\"{mask}><path data-d2-rs-sketch-set=\"rough\" d=\"{}\" fill=\"url(#d2-rs-sketch-streak-{})\" stroke=\"none\"/></g>",
                streak.category,
                streak.blend,
                escape(path),
                streak.category
            );
        }
    }
    if shadow {
        svg.push_str("</g>");
    }
    true
}
