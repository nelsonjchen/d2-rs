// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Rough, hand-drawn primitives for D2's sketch mode: a port of the `rough-go`
//! library that upstream D2's `d2renderers/d2sketch` uses. The sketch renderer
//! itself is in `d2-rs-render`.
//!
//! [`generator`] is the port of rough-go's `Generator` (options, drawables, SVG
//! path commands, every filler and `ToPaths`); the point and path helpers in
//! this module serve the native scene renderer. The `oracle-fixtures` feature
//! compiles tests that read regenerated Go-oracle artifacts.
//!
//! The translated rough-go/Rough.js portions retain their MIT terms; see
//! `THIRD_PARTY_NOTICES.md` in this crate.

use d2_rs_core::{Point, Rect};

mod zigzag;
pub use zigzag::{ZigzagError, ZigzagSketchPaths, rough_zigzag_polygon_paths};
mod svg_path;
pub use svg_path::{SvgSketchCommand, rough_solid_open_line_paths, rough_svg_path};
pub mod generator;
mod gomath;
pub use generator::{
    Drawable, Generator, OpSet, OpSetType, OpType, Options, PathInfo, PatternInfo, ResolvedOptions,
    RoughLine, RoughPoint, Surface, js_number_string,
};

/// Controls deterministic roughness for a generated path.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SketchOptions {
    /// Seed for the local Rough.js-compatible pseudo-random stream.
    pub seed: u64,
    /// Rough.js `roughness` multiplier.
    pub roughness: f64,
    /// Maximum randomness offset. Rough-go defaults this to `2`.
    pub max_randomness_offset: f64,
    /// Rough.js bowing multiplier.
    pub bowing: f64,
    /// Preserve the input vertices instead of perturbing endpoints.
    pub preserve_vertices: bool,
    /// Emit Rough.js's second overlay stroke.
    pub disable_multi_stroke: bool,
}

impl Default for SketchOptions {
    fn default() -> Self {
        Self {
            seed: 1,
            roughness: 1.0,
            max_randomness_offset: 2.0,
            bowing: 1.0,
            preserve_vertices: false,
            disable_multi_stroke: false,
        }
    }
}

/// Generates a two-point roughened line as sampled points.
///
/// This compatibility helper remains useful to callers that consume a point
/// list. SVG rendering should use [`rough_line_path`], which preserves the
/// cubic operations emitted by rough-go.
pub fn rough_line(start: Point, end: Point, options: SketchOptions) -> Vec<Point> {
    let dx = end.x - start.x;
    let dy = end.y - start.y;
    let length = dx.hypot(dy).max(f64::EPSILON);
    let normal = Point {
        x: -dy / length,
        y: dx / length,
    };
    let midpoint = Point {
        x: (start.x + end.x) / 2.0,
        y: (start.y + end.y) / 2.0,
    };
    let jitter = (unit(options.seed).mul_add(2.0, -1.0)) * options.roughness;
    vec![
        start,
        Point {
            x: midpoint.x + normal.x * jitter,
            y: midpoint.y + normal.y * jitter,
        },
        end,
    ]
}

/// Generates the Rough.js-compatible SVG path for one line.
pub fn rough_line_path(start: Point, end: Point, options: SketchOptions) -> String {
    let mut random = RandomState::new(options.seed as f64);
    let mut output = String::new();
    append_rough_line(&mut output, start, end, &options, &mut random, false);
    if !options.disable_multi_stroke {
        append_rough_line(&mut output, start, end, &options, &mut random, true);
    }
    output.trim_end().to_owned()
}

/// Generates a closed roughened rectangle in the same operation order as
/// rough-go's `Generator.Rectangle` outline.
pub fn rough_rect_path(rect: Rect, options: SketchOptions) -> String {
    let mut random = RandomState::new(options.seed as f64);
    rough_polygon_outline(&rect_corners(rect), &options, &mut random)
}

fn rect_corners(rect: Rect) -> [Point; 4] {
    [
        rect.origin,
        Point {
            x: rect.right(),
            y: rect.origin.y,
        },
        Point {
            x: rect.right(),
            y: rect.bottom(),
        },
        Point {
            x: rect.origin.x,
            y: rect.bottom(),
        },
    ]
}

/// Solid fill and outline paths in rough-go's drawable paint order.
///
/// The outline consumes randomness first, although the fill is painted first.
/// The fill has no explicit close operation: SVG fill closes it implicitly,
/// while the stroke must retain the original open path.
#[derive(Clone, Debug, PartialEq)]
pub struct SolidSketchPaths {
    pub fill: String,
    pub outline: String,
}

/// Generates both solid rectangle paths from one local random stream.
pub fn rough_solid_rect_paths(rect: Rect, options: SketchOptions) -> SolidSketchPaths {
    rough_solid_polygon_paths(&rect_corners(rect), options)
}

/// Generates rough-go's solid polygon drawable, including degenerate inputs.
pub fn rough_solid_polygon_paths(points: &[Point], options: SketchOptions) -> SolidSketchPaths {
    let mut random = RandomState::new(options.seed as f64);
    let outline = rough_polygon_outline(points, &options, &mut random);
    let mut fill = String::new();
    if points.len() > 2 {
        use std::fmt::Write;
        for (index, point) in points.iter().enumerate() {
            let x = point.x + offset(options.max_randomness_offset, 1.0, &options, &mut random);
            let y = point.y + offset(options.max_randomness_offset, 1.0, &options, &mut random);
            let _ = write!(
                fill,
                "{}{} {} ",
                if index == 0 { 'M' } else { 'L' },
                number(x),
                number(y)
            );
        }
    }
    SolidSketchPaths {
        fill: fill.trim_end().to_owned(),
        outline,
    }
}

fn rough_polygon_outline(
    points: &[Point],
    options: &SketchOptions,
    random: &mut RandomState,
) -> String {
    let mut output = String::new();
    if points.len() < 2 {
        return output;
    }
    let edge_count = if points.len() == 2 { 1 } else { points.len() };
    for index in 0..edge_count {
        let start = points[index];
        let end = points[(index + 1) % points.len()];
        append_rough_line(&mut output, start, end, options, random, false);
        if !options.disable_multi_stroke {
            append_rough_line(&mut output, start, end, options, random, true);
        }
    }
    output.trim_end().to_owned()
}

/// Generates the Rough.js-compatible SVG path for an ellipse or circle.
pub fn rough_ellipse_path(
    center: Point,
    width: f64,
    height: f64,
    options: SketchOptions,
) -> String {
    let mut random = RandomState::new(options.seed as f64);
    let params = ellipse_parameters(width, height, &options, &mut random);
    ellipse_response(center, params, &options, &mut random)
}

/// Original Generator.Ellipse generates the outline first, then a second
/// ellipse fill using the same parameters and continuing random stream.
pub fn rough_solid_ellipse_paths(
    center: Point,
    width: f64,
    height: f64,
    options: SketchOptions,
) -> SolidSketchPaths {
    let mut random = RandomState::new(options.seed as f64);
    let params = ellipse_parameters(width, height, &options, &mut random);
    let outline = ellipse_response(center, params, &options, &mut random);
    let fill = ellipse_response(center, params, &options, &mut random);
    SolidSketchPaths { fill, outline }
}

/// Original Generator.LinearPath retains one stream across all open edges.
pub fn rough_linear_path(points: &[Point], options: SketchOptions) -> String {
    let mut random = RandomState::new(options.seed as f64);
    let mut out = String::new();
    for edge in points.windows(2) {
        append_rough_line(&mut out, edge[0], edge[1], &options, &mut random, false);
        if !options.disable_multi_stroke {
            append_rough_line(&mut out, edge[0], edge[1], &options, &mut random, true);
        }
    }
    out.trim_end().to_owned()
}

fn ellipse_parameters(
    width: f64,
    height: f64,
    options: &SketchOptions,
    random: &mut RandomState,
) -> (f64, f64, f64) {
    let curve_step_count: f64 = 9.0;
    let curve_fitting: f64 = 0.95;
    let psq = (std::f64::consts::PI
        * 2.0
        * (((width / 2.0).powi(2) + (height / 2.0).powi(2)) / 2.0).sqrt())
    .sqrt();
    let step_count = curve_step_count.max((curve_step_count / 200.0_f64.sqrt()) * psq);
    let increment = std::f64::consts::TAU / step_count.ceil();
    let rx = (width / 2.0).abs()
        + offset(
            (width / 2.0).abs() * (1.0 - curve_fitting),
            1.0,
            options,
            random,
        );
    let ry = (height / 2.0).abs()
        + offset(
            (height / 2.0).abs() * (1.0 - curve_fitting),
            1.0,
            options,
            random,
        );
    (increment, rx, ry)
}
fn ellipse_response(
    center: Point,
    (increment, rx, ry): (f64, f64, f64),
    options: &SketchOptions,
    random: &mut RandomState,
) -> String {
    let overlap = increment
        * offset_range(
            0.1,
            offset_range(0.4, 1.0, options, random),
            options,
            random,
        );
    let mut output = String::new();
    let first = ellipse_points(increment, center, rx, ry, 1.0, overlap, options, random);
    append_rough_curve(&mut output, &first);
    if !options.disable_multi_stroke && options.roughness != 0.0 {
        let second = ellipse_points(increment, center, rx, ry, 1.5, 0.0, options, random);
        append_rough_curve(&mut output, &second);
    }
    output.trim_end().to_owned()
}

/// Generates a closed roughened rectangle as sampled points for legacy users.
pub fn rough_rect(rect: Rect, options: SketchOptions) -> Vec<Point> {
    let corners = [
        rect.origin,
        Point {
            x: rect.right(),
            y: rect.origin.y,
        },
        Point {
            x: rect.right(),
            y: rect.bottom(),
        },
        Point {
            x: rect.origin.x,
            y: rect.bottom(),
        },
    ];
    let mut output = Vec::with_capacity(9);
    for (index, window) in corners
        .iter()
        .cycle()
        .take(5)
        .collect::<Vec<_>>()
        .windows(2)
        .enumerate()
    {
        let mut segment = rough_line(
            *window[0],
            *window[1],
            SketchOptions {
                seed: options.seed.wrapping_add(index as u64),
                ..options
            },
        );
        if index > 0 {
            segment.remove(0);
        }
        output.extend(segment);
    }
    output
}

#[derive(Clone, Copy, Debug)]
struct RandomState {
    seed: f64,
}

impl RandomState {
    fn new(seed: f64) -> Self {
        Self { seed }
    }

    fn next(&mut self) -> f64 {
        if self.seed == 0.0 || self.seed.is_nan() {
            // D2 always supplies a non-zero seed. Keep the fallback stable so
            // accidental zero seeds never make SVG output nondeterministic.
            self.seed = 1.0;
        }
        let seed = js_to_i32(self.seed) as u32;
        let next = seed.wrapping_mul(48_271) as i32;
        self.seed = f64::from(next);
        f64::from((next as u32) & 0x7fff_ffff) / 2_147_483_648.0
    }
}

fn append_rough_line(
    output: &mut String,
    start: Point,
    end: Point,
    options: &SketchOptions,
    random: &mut RandomState,
    overlay: bool,
) {
    let length_squared = (start.x - end.x).powi(2) + (start.y - end.y).powi(2);
    let length = length_squared.sqrt();
    let roughness_gain = if length < 200.0 {
        1.0
    } else if length > 500.0 {
        0.4
    } else {
        (-0.0016668 * length) + 1.233334
    };

    let mut random_offset = options.max_randomness_offset;
    if random_offset * random_offset * 100.0 > length_squared {
        random_offset = length / 10.0;
    }
    let half_offset = random_offset / 2.0;
    // Original Go's arm64 roughLine fuses these multiply/add expressions.
    // Keep that rounding explicit rather than depending on Rust optimization.
    let diverge_point = go_mul_add(random.next(), 0.2, 0.2);
    let mid_disp_x = offset(
        options.bowing * options.max_randomness_offset * (end.y - start.y) / 200.0,
        roughness_gain,
        options,
        random,
    );
    let mid_disp_y = offset(
        options.bowing * options.max_randomness_offset * (start.x - end.x) / 200.0,
        roughness_gain,
        options,
        random,
    );
    let perturb = |random: &mut RandomState| {
        offset(
            if overlay { half_offset } else { random_offset },
            roughness_gain,
            options,
            random,
        )
    };
    let first_x = start.x
        + if options.preserve_vertices {
            0.0
        } else {
            perturb(random)
        };
    let first_y = start.y
        + if options.preserve_vertices {
            0.0
        } else {
            perturb(random)
        };
    let c1_x = go_mul_add(end.x - start.x, diverge_point, mid_disp_x + start.x) + perturb(random);
    let c1_y = go_mul_add(end.y - start.y, diverge_point, mid_disp_y + start.y) + perturb(random);
    let c2_x =
        go_mul_add(2.0 * (end.x - start.x), diverge_point, mid_disp_x + start.x) + perturb(random);
    let c2_y =
        go_mul_add(2.0 * (end.y - start.y), diverge_point, mid_disp_y + start.y) + perturb(random);
    let last_x = end.x
        + if options.preserve_vertices {
            0.0
        } else {
            perturb(random)
        };
    let last_y = end.y
        + if options.preserve_vertices {
            0.0
        } else {
            perturb(random)
        };

    use std::fmt::Write;
    let _ = write!(
        output,
        "M{} {} C{} {}, {} {}, {} {} ",
        number(first_x),
        number(first_y),
        number(c1_x),
        number(c1_y),
        number(c2_x),
        number(c2_y),
        number(last_x),
        number(last_y),
    );
}

#[allow(clippy::too_many_arguments)]
fn ellipse_points(
    increment: f64,
    center: Point,
    rx: f64,
    ry: f64,
    random_offset: f64,
    overlap: f64,
    options: &SketchOptions,
    random: &mut RandomState,
) -> Vec<Point> {
    if options.roughness == 0.0 {
        let step = increment / 4.0;
        let mut points = vec![Point {
            x: center.x + rx * (-step).cos(),
            y: center.y + ry * (-step).sin(),
        }];
        let mut angle = 0.0;
        while angle <= std::f64::consts::TAU {
            points.push(Point {
                x: center.x + rx * angle.cos(),
                y: center.y + ry * angle.sin(),
            });
            angle += step;
        }
        points.push(Point {
            x: center.x + rx,
            y: center.y,
        });
        points.push(Point {
            x: center.x + rx * step.cos(),
            y: center.y + ry * step.sin(),
        });
        return points;
    }

    let rad_offset = offset(0.5, 1.0, options, random) - std::f64::consts::FRAC_PI_2;
    let mut points = vec![Point {
        x: offset(random_offset, 1.0, options, random)
            + center.x
            + 0.9 * rx * (rad_offset - increment).cos(),
        y: offset(random_offset, 1.0, options, random)
            + center.y
            + 0.9 * ry * (rad_offset - increment).sin(),
    }];
    let mut angle = rad_offset;
    while angle < std::f64::consts::TAU + rad_offset - 0.01 {
        points.push(Point {
            x: offset(random_offset, 1.0, options, random) + center.x + rx * angle.cos(),
            y: offset(random_offset, 1.0, options, random) + center.y + ry * angle.sin(),
        });
        angle += increment;
    }
    points.push(Point {
        x: offset(random_offset, 1.0, options, random)
            + center.x
            + rx * (rad_offset + std::f64::consts::TAU + overlap * 0.5).cos(),
        y: offset(random_offset, 1.0, options, random)
            + center.y
            + ry * (rad_offset + std::f64::consts::TAU + overlap * 0.5).sin(),
    });
    points.push(Point {
        x: offset(random_offset, 1.0, options, random)
            + center.x
            + 0.98 * rx * (rad_offset + overlap).cos(),
        y: offset(random_offset, 1.0, options, random)
            + center.y
            + 0.98 * ry * (rad_offset + overlap).sin(),
    });
    points.push(Point {
        x: offset(random_offset, 1.0, options, random)
            + center.x
            + 0.9 * rx * (rad_offset + overlap * 0.5).cos(),
        y: offset(random_offset, 1.0, options, random)
            + center.y
            + 0.9 * ry * (rad_offset + overlap * 0.5).sin(),
    });
    points
}

fn append_rough_curve(output: &mut String, points: &[Point]) {
    if points.len() <= 2 {
        return;
    }
    use std::fmt::Write;
    let _ = write!(output, "M{} {} ", number(points[1].x), number(points[1].y));
    for index in 1..points.len().saturating_sub(2) {
        let current = points[index];
        let next = points[index + 1];
        let previous = points[index - 1];
        let following = points[index + 2];
        let b1 = Point {
            x: current.x + (next.x - previous.x) / 6.0,
            y: current.y + (next.y - previous.y) / 6.0,
        };
        let b2 = Point {
            x: next.x + (current.x - following.x) / 6.0,
            y: next.y + (current.y - following.y) / 6.0,
        };
        let _ = write!(
            output,
            "C{} {}, {} {}, {} {} ",
            number(b1.x),
            number(b1.y),
            number(b2.x),
            number(b2.y),
            number(next.x),
            number(next.y)
        );
    }
}

fn offset(
    base: f64,
    roughness_gain: f64,
    options: &SketchOptions,
    random: &mut RandomState,
) -> f64 {
    // Source offset multiplies roughness and gain before applying the random
    // displacement. Moving gain after the displacement changes float bits.
    (options.roughness * roughness_gain) * go_mul_add(random.next(), base - (-base), -base)
}

fn offset_range(
    minimum: f64,
    maximum: f64,
    options: &SketchOptions,
    random: &mut RandomState,
) -> f64 {
    options.roughness * go_mul_add(random.next(), maximum - minimum, minimum)
}

use d2_rs_core::go_fmadd as go_mul_add;

fn js_to_i32(value: f64) -> i32 {
    if value == 0.0 || value.is_nan() || value.is_infinite() {
        return 0;
    }
    let mut value = value.trunc() % 4_294_967_296.0;
    if value < 0.0 {
        value += 4_294_967_296.0;
    }
    if value >= 2_147_483_648.0 {
        value -= 4_294_967_296.0;
    }
    value as i32
}

fn number(value: f64) -> String {
    if value == 0.0 {
        return "0".to_owned();
    }
    let mut text = value.to_string();
    if let Some(index) = text.find('e') {
        let (mantissa, exponent) = text.split_at(index);
        let exponent = exponent.trim_start_matches('e');
        let sign = exponent.chars().next().unwrap_or('+');
        let digits = exponent
            .trim_start_matches(['+', '-'])
            .trim_start_matches('0');
        text = format!(
            "{}e{}{}",
            mantissa,
            sign,
            if digits.is_empty() { "0" } else { digits }
        );
    }
    text
}

fn unit(mut value: u64) -> f64 {
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^= value >> 31;
    (value as f64) / (u64::MAX as f64)
}

#[cfg(test)]
mod tests {
    use super::{
        SketchOptions, rough_ellipse_path, rough_line, rough_line_path, rough_rect, rough_rect_path,
    };
    use d2_rs_core::{Point, Rect, Size};

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn solid_polygon_drawables_match_complete_original_go_paths() {
        // Compare SVG operation order and exact numeric values; serialization
        // spelling (for example Go's shortest-number formatting) is incidental.
        fn compare(actual: &str, expected: &str, case: &str) {
            fn split(path: &str) -> Vec<String> {
                let mut expanded = String::new();
                for c in path.chars() {
                    if matches!(c, 'M' | 'L' | 'C') {
                        expanded.push(' ');
                        expanded.push(c);
                        expanded.push(' ');
                    } else if c == ',' {
                        expanded.push(' ');
                    } else {
                        expanded.push(c);
                    }
                }
                expanded.split_whitespace().map(str::to_owned).collect()
            }
            let actual = split(actual);
            let expected = split(expected);
            assert_eq!(actual.len(), expected.len(), "{case}");
            for (a, e) in actual.iter().zip(&expected) {
                match (a.parse::<f64>(), e.parse::<f64>()) {
                    (Ok(a), Ok(e)) => assert_eq!(a.to_bits(), e.to_bits(), "{case}: {a} != {e}"),
                    _ => assert_eq!(a, e, "{case}"),
                }
            }
        }
        let fixture = d2_rs_testdata::oracle_fixture!("tests/fixtures/solid-paths-source-v090.tsv");
        assert_eq!(fixture.lines().count(), 72);
        for row in fixture.lines() {
            let columns: Vec<_> = row.split('\t').collect();
            assert_eq!(columns.len(), 5);
            let options = SketchOptions {
                seed: columns[0].parse().unwrap(),
                bowing: 2.0,
                preserve_vertices: columns[1] == "preserve",
                disable_multi_stroke: columns[1] == "single",
                roughness: if columns[1] == "zero" { 0.0 } else { 1.0 },
                ..Default::default()
            };
            let paths = match columns[2] {
                "rect" | "rect-long" => super::rough_solid_rect_paths(
                    Rect {
                        origin: Point { x: 2.25, y: -3.5 },
                        size: Size {
                            width: if columns[2] == "rect-long" {
                                350.0
                            } else {
                                140.0
                            },
                            height: if columns[2] == "rect-long" {
                                700.0
                            } else {
                                80.0
                            },
                        },
                    },
                    options,
                ),
                shape => {
                    let points = match shape {
                        "triangle" => vec![
                            Point { x: 0.0, y: 0.0 },
                            Point { x: 100.5, y: 10.25 },
                            Point { x: 33.0, y: 77.0 },
                        ],
                        "line" => vec![Point { x: 0.0, y: 0.0 }, Point { x: 100.5, y: 10.25 }],
                        "point" => vec![Point { x: 3.0, y: 5.0 }],
                        "empty" => vec![],
                        _ => panic!("unexpected fixture shape"),
                    };
                    super::rough_solid_polygon_paths(&points, options)
                }
            };
            compare(&paths.fill, columns[3], row);
            compare(&paths.outline, columns[4], row);
        }
    }

    #[test]
    fn rough_lines_are_deterministic_and_keep_endpoints() {
        let options = SketchOptions {
            seed: 42,
            roughness: 1.5,
            ..Default::default()
        };
        let start = Point { x: 0.0, y: 0.0 };
        let end = Point { x: 100.0, y: 0.0 };
        let first = rough_line(start, end, options);
        assert_eq!(first, rough_line(start, end, options));
        assert_eq!(first.first(), Some(&start));
        assert_eq!(first.last(), Some(&end));
        assert_ne!(first[1].y, 0.0);
    }

    #[test]
    fn rough_rect_is_closed() {
        let path = rough_rect(
            Rect {
                origin: Point { x: 2.0, y: 3.0 },
                size: Size {
                    width: 20.0,
                    height: 10.0,
                },
            },
            SketchOptions::default(),
        );
        assert_eq!(path.first(), path.last());
        assert_eq!(path.len(), 9);
    }

    #[test]
    fn rough_paths_emit_two_cubic_strokes_per_line() {
        let path = rough_line_path(
            Point { x: 0.0, y: 0.0 },
            Point { x: 100.0, y: 0.0 },
            SketchOptions::default(),
        );
        assert_eq!(path.matches('M').count(), 2);
        assert_eq!(path.matches('C').count(), 2);
        assert!(path.contains("M0"));
        assert_eq!(
            path,
            "M0.8572634477168322 0.9638847392052412 C19.680695734918118 0.9605148276314139, 39.440775437280536 -1.9328214088454843, 99.20640534348786 0.3923355024307966 M-0.6486655632033944 0.26459860894829035 C22.574641727842394 0.3819836974143982, 44.356770697049804 -0.1876889169216156, 99.41962571348995 0.752815873362124"
        );
    }

    #[test]
    fn rough_rect_path_emits_each_edge_in_source_order() {
        let path = rough_rect_path(
            Rect {
                origin: Point { x: 2.0, y: 3.0 },
                size: Size {
                    width: 20.0,
                    height: 10.0,
                },
            },
            SketchOptions::default(),
        );
        assert_eq!(path.matches('M').count(), 8);
        assert_eq!(path.matches('C').count(), 8);
        assert!(path.starts_with('M'));
    }

    #[test]
    fn rough_ellipse_path_is_seeded_and_uses_two_curve_passes() {
        let options = SketchOptions {
            seed: 1,
            ..Default::default()
        };
        let first = rough_ellipse_path(Point { x: 50.0, y: 40.0 }, 100.0, 80.0, options);
        assert_eq!(
            first,
            rough_ellipse_path(Point { x: 50.0, y: 40.0 }, 100.0, 80.0, options)
        );
        assert_eq!(first.matches('M').count(), 2);
        assert!(first.matches('C').count() > 10);
    }
}
