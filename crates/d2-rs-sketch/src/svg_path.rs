// SPDX-License-Identifier: MIT
// Translated from rough-go v0.2.0 renderer.go / Rough.js.
// Copyright (c) 2019, 2020 Preet Shihn; 2026 rough-go contributors.
use super::{
    Point, RandomState, SketchOptions, SolidSketchPaths, append_rough_line, number, offset,
};
use std::fmt::Write;

/// Absolute commands reachable from D2 connection paths and crow's feet.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SvgSketchCommand {
    Move(Point),
    Line(Point),
    Cubic(Point, Point, Point),
    Close,
}

/// Original svgPath: moves consume no randomness, and cubics use two direct
/// perturbed curves rather than the native scene's fixed-step polyline.
pub fn rough_svg_path(commands: &[SvgSketchCommand], options: SketchOptions) -> String {
    generate(
        commands,
        &options,
        &mut RandomState::new(options.seed as f64),
    )
}

fn generate(
    commands: &[SvgSketchCommand],
    options: &SketchOptions,
    random: &mut RandomState,
) -> String {
    let mut out = String::new();
    let mut current = Point::default();
    let mut first = None;
    for command in commands {
        match *command {
            SvgSketchCommand::Move(p) => {
                current = p;
                first = Some(p);
            }
            SvgSketchCommand::Line(p) => {
                append_rough_line(&mut out, current, p, options, random, false);
                if !options.disable_multi_stroke {
                    append_rough_line(&mut out, current, p, options, random, true);
                }
                current = p;
            }
            SvgSketchCommand::Cubic(c1, c2, end) => {
                let initial = if options.max_randomness_offset == 0.0 {
                    1.0
                } else {
                    options.max_randomness_offset
                };
                for pass in 0..if options.disable_multi_stroke { 1 } else { 2 } {
                    let mut start = current;
                    if pass != 0 && !options.preserve_vertices {
                        start.x += offset(initial, 1.0, options, random);
                        start.y += offset(initial, 1.0, options, random);
                    }
                    let radius = initial + if pass == 0 { 0.0 } else { 0.3 };
                    let mut finish = end;
                    if !options.preserve_vertices {
                        finish.x += offset(radius, 1.0, options, random);
                        finish.y += offset(radius, 1.0, options, random);
                    }
                    let a = Point {
                        x: c1.x + offset(radius, 1.0, options, random),
                        y: c1.y + offset(radius, 1.0, options, random),
                    };
                    let b = Point {
                        x: c2.x + offset(radius, 1.0, options, random),
                        y: c2.y + offset(radius, 1.0, options, random),
                    };
                    let _ = write!(
                        out,
                        "M{} {} C{} {}, {} {}, {} {} ",
                        number(start.x),
                        number(start.y),
                        number(a.x),
                        number(a.y),
                        number(b.x),
                        number(b.y),
                        number(finish.x),
                        number(finish.y)
                    );
                }
                // The next command starts at the authored endpoint, not the
                // perturbed end of either visible pass.
                current = end;
            }
            SvgSketchCommand::Close => {
                if let Some(p) = first {
                    append_rough_line(&mut out, current, p, options, random, false);
                    if !options.disable_multi_stroke {
                        append_rough_line(&mut out, current, p, options, random, true);
                    }
                    current = p;
                }
            }
        }
    }
    out.trim_end().to_owned()
}

/// Generator.Path's solid single-line fill. D2 uses this for cf-one; the
/// cloned fill options continue the outline RNG with roughness increased0.8.
pub fn rough_solid_open_line_paths(
    start: Point,
    end: Point,
    options: SketchOptions,
) -> SolidSketchPaths {
    let commands = [SvgSketchCommand::Move(start), SvgSketchCommand::Line(end)];
    let mut random = RandomState::new(options.seed as f64);
    let outline = generate(&commands, &options, &mut random);
    let fill_options = SketchOptions {
        disable_multi_stroke: true,
        roughness: if options.roughness == 0.0 {
            0.0
        } else {
            options.roughness + 0.8
        },
        ..options
    };
    let fill = generate(&commands, &fill_options, &mut random);
    SolidSketchPaths { fill, outline }
}
