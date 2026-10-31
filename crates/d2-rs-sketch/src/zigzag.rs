// SPDX-License-Identifier: MIT
// Translated from rough-go v0.2.0 / Rough.js 4.6.6 and hachure-fill 0.5.2.
// Copyright (c) 2019, 2023 Preet Shihn; 2026 rough-go contributors.
use super::{Point, RandomState, SketchOptions, append_rough_line, rough_polygon_outline};

/// The fill is stroked in the fill color, before the ordinary outline.
#[derive(Clone, Debug, PartialEq)]
pub struct ZigzagSketchPaths {
    pub fill: String,
    pub outline: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZigzagError {
    NonFiniteGeometry,
    NonAdvancingScan,
}

/// rough-go Polygon with its default zigzag fill angle/gap. The outline and
/// filler share one randomizer; `stroke_width` determines the default gap.
pub fn rough_zigzag_polygon_paths(
    points: &[Point],
    stroke_width: f64,
    options: SketchOptions,
) -> Result<ZigzagSketchPaths, ZigzagError> {
    if !stroke_width.is_finite() || points.iter().any(|p| !p.x.is_finite() || !p.y.is_finite()) {
        return Err(ZigzagError::NonFiniteGeometry);
    }
    let mut random = RandomState::new(options.seed as f64);
    let outline = rough_polygon_outline(points, &options, &mut random);
    let gap = (stroke_width * 4.0).max(0.1);
    let rounded_gap = js_round(gap);
    let step = if options.roughness >= 1.0 && random.next() > 0.7 {
        rounded_gap
    } else {
        1.0
    };
    let mut rotated = points.iter().map(|&p| rotate(p, 49.0)).collect::<Vec<_>>();
    stabilize(points, &mut rotated, 49.0);
    let lines = scan_lines(&rotated, rounded_gap, step)?;
    let angle = std::f64::consts::PI / 180.0 * -41.0;
    let (half_gap, cos, sin) = (gap * 0.5, angle.cos(), angle.sin());
    let mut fill = String::new();
    for (a, b) in lines {
        let (a, b) = (rotate(a, -49.0), rotate(b, -49.0));
        if ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt() == 0.0 {
            continue;
        }
        // Go arm64 fuses the final gap products into each endpoint offset.
        for start in [
            Point {
                x: (-half_gap).mul_add(cos, a.x),
                y: half_gap.mul_add(sin, a.y),
            },
            Point {
                x: half_gap.mul_add(cos, a.x),
                y: (-half_gap).mul_add(sin, a.y),
            },
        ] {
            append_rough_line(&mut fill, start, b, &options, &mut random, false);
            // DisableMultiStrokeFill defaults false independently of outline.
            append_rough_line(&mut fill, start, b, &options, &mut random, true);
        }
    }
    Ok(ZigzagSketchPaths {
        fill: fill.trim_end().to_owned(),
        outline,
    })
}

fn js_round(v: f64) -> f64 {
    if !v.is_finite() || v == 0.0 || v.abs() >= 4_503_599_627_370_496.0 {
        return v;
    }
    if (-0.5..0.0).contains(&v) {
        return -0.0;
    }
    (v + 0.5).floor()
}

fn rotate(p: Point, degrees: f64) -> Point {
    let angle = (std::f64::consts::PI / 180.0) * degrees;
    let (sin, cos) = (angle.sin(), angle.cos());
    // Source materializes each product to preserve JavaScript rounding.
    Point {
        x: p.x * cos - p.y * sin,
        y: p.x * sin + p.y * cos,
    }
}

fn stabilize(original: &[Point], rotated: &mut [Point], degrees: f64) {
    if rotated.len() < 2 {
        return;
    }
    let angle = (std::f64::consts::PI / 180.0) * degrees;
    let (sin, cos) = (angle.sin(), angle.cos());
    for i in 0..rotated.len() {
        let j = (i + 1) % rotated.len();
        if rotated[i].y != rotated[j].y || original[i] == original[j] {
            continue;
        }
        let (a, b) = (original[i], original[j]);
        let scale = 1.0_f64
            .max(a.x.abs())
            .max(a.y.abs())
            .max(b.x.abs())
            .max(b.y.abs());
        let tolerance = 8.0 * (scale.next_up() - scale);
        if (a.x - b.x).abs() > tolerance || (a.y - b.y).abs() > tolerance {
            continue;
        }
        let dy = (a.y - b.y).mul_add(cos, (a.x - b.x) * sin);
        if dy > 0.0 {
            rotated[i].y = rotated[i].y.next_up();
        }
        if dy < 0.0 {
            rotated[i].y = rotated[i].y.next_down();
        }
    }
}

#[derive(Clone, Copy)]
struct Edge {
    ymin: f64,
    ymax: f64,
    x: f64,
    slope: f64,
}

fn scan_lines(points: &[Point], gap: f64, step: f64) -> Result<Vec<(Point, Point)>, ZigzagError> {
    if points.len() < 3 {
        return Ok(Vec::new());
    }
    let mut vertices = points.to_vec();
    if vertices.first() != vertices.last() {
        vertices.push(vertices[0]);
    }
    let mut edges = Vec::new();
    for pair in vertices.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        if a.y == b.y {
            continue;
        }
        let ymin = a.y.min(b.y);
        edges.push(Edge {
            ymin,
            ymax: a.y.max(b.y),
            x: if ymin == a.y { a.x } else { b.x },
            slope: (b.x - a.x) / (b.y - a.y),
        });
    }
    edges.sort_by(|a, b| {
        a.ymin
            .partial_cmp(&b.ymin)
            .unwrap()
            .then_with(|| a.x.partial_cmp(&b.x).unwrap())
            .then_with(|| a.ymax.partial_cmp(&b.ymax).unwrap())
    });
    if edges.is_empty() {
        return Ok(Vec::new());
    }
    let mut active: Vec<Edge> = Vec::new();
    let mut next_edge = 0;
    let mut y = edges[0].ymin;
    let mut iteration = 0.0;
    let mut lines = Vec::new();
    while !active.is_empty() || next_edge < edges.len() {
        while next_edge < edges.len() && edges[next_edge].ymin <= y {
            active.push(edges[next_edge]);
            next_edge += 1;
        }
        active.retain(|edge| edge.ymax > y);
        active.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap());
        if step != 1.0 || iteration % gap == 0.0 {
            for pair in active.chunks_exact(2) {
                lines.push((
                    Point {
                        x: js_round(pair[0].x),
                        y,
                    },
                    Point {
                        x: js_round(pair[1].x),
                        y,
                    },
                ));
            }
        }
        let next_y = y + step;
        if next_y == y {
            return Err(ZigzagError::NonAdvancingScan);
        }
        y = next_y;
        // The original Go arm64 scan loop uses FMADDD here.
        for edge in &mut active {
            edge.x = step.mul_add(edge.slope, edge.x);
        }
        iteration += 1.0;
    }
    Ok(lines)
}
