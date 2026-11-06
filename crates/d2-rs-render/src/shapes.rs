// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Source-shaped SVG paths for D2 nodes.
//!
//! The constants and control points in this module follow the pinned D2
//! `lib/shape` implementations.  Keeping them together gives rendering and
//! geometry investigations a direct place to compare against the upstream
//! `nodeshape` package; this module does not change layout sizing or routing.

use super::{
    PaletteView, escape, escape_xml_text, number, patternassets, render_text_content,
    stroke_dasharray,
};
use base64::Engine as _;
use d2_rs_core::{Point, Rect, ShapeKind, Size};
use d2_rs_language::compiler::SourceNodeStyle;
use std::fmt::Write;

const PARALLEL_WEDGE_WIDTH: f64 = 26.0;
const DOC_PATH_HEIGHT: f64 = 18.925;
const DOC_PATH_BOTTOM: f64 = 16.3;
const DEFAULT_ARC_DEPTH: f64 = 24.0;
const PAGE_CORNER_WIDTH: f64 = 20.8164;
const PAGE_CORNER_HEIGHT: f64 = 20.348;
const PACKAGE_TOP_MAX_HEIGHT: f64 = 55.0;
const PACKAGE_TOP_MIN_WIDTH: f64 = 50.0;
const PACKAGE_TOP_MAX_WIDTH: f64 = 150.0;
const PACKAGE_HORIZONTAL_SCALAR: f64 = 0.5;
const PACKAGE_VERTICAL_SCALAR: f64 = 0.2;
const STEP_WEDGE_WIDTH: f64 = 35.0;
const DEFAULT_TIP_WIDTH: f64 = 30.0;
const DEFAULT_TIP_HEIGHT: f64 = 45.0;
const STORED_DATA_WEDGE_WIDTH: f64 = 15.0;
const C4_HEAD_RADIUS_FACTOR: f64 = 0.22;
const C4_BODY_TOP_FACTOR: f64 = 0.8;
const C4_CORNER_RADIUS_FACTOR: f64 = 0.175;
const THREE_DEE_OFFSET: f64 = 15.0;

fn polygon(points: &[Point]) -> String {
    points
        .iter()
        .map(|point| format!("{},{}", number(point.x), number(point.y)))
        .collect::<Vec<_>>()
        .join(" ")
}

fn hexagon_points(rect: Rect) -> [Point; 6] {
    // D2 v0.9.0's shapeHexagon starts at x=0.25 and uses
    // 43.6/87.3 of the box height for its side vertices.
    let x_offset = rect.size.width * 0.25;
    let y_offset = rect.size.height * (43.6 / 87.3);
    [
        svg_path_point(rect.origin.x + x_offset, rect.origin.y),
        svg_path_point(rect.origin.x, rect.origin.y + y_offset),
        svg_path_point(rect.origin.x + x_offset, rect.origin.y + rect.size.height),
        svg_path_point(
            rect.origin.x + rect.size.width - x_offset,
            rect.origin.y + rect.size.height,
        ),
        svg_path_point(rect.origin.x + rect.size.width, rect.origin.y + y_offset),
        svg_path_point(rect.origin.x + rect.size.width - x_offset, rect.origin.y),
    ]
}

fn svg_path_point(x: f64, y: f64) -> Point {
    Point {
        x: svg_path_coordinate(x),
        y: svg_path_coordinate(y),
    }
}

fn svg_path_coordinate(value: f64) -> f64 {
    // lib/svg/path.go's chopPrecision narrows scaled coordinates to float32,
    // divides by 10,000, then rounds the point to an integer.
    let scaled = (value * 10_000.0) as f32;
    let rounded = (f64::from(scaled) / 10_000.0).round();
    if rounded == 0.0 { 0.0 } else { rounded }
}

fn themed_color(raw: Option<&str>, fallback: &str, palette: PaletteView<'_>) -> String {
    let Some(raw) = raw else {
        return fallback.to_owned();
    };
    palette.explicit(raw, fallback)
}

/// Stable local ID for a CSS gradient. D2 uses a SHA-1-derived identifier;
/// this FNV-1a spelling is equally deterministic and avoids pulling a hash
/// dependency into the renderer. The ID is only an intra-document reference.
pub(crate) fn gradient_id(value: &str) -> String {
    let mut hash = 2_166_136_261u32;
    for byte in value.as_bytes() {
        hash = (hash ^ u32::from(*byte)).wrapping_mul(16_777_619);
    }
    format!("grad-{hash:08x}")
}

fn split_gradient_params(value: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut nesting = 0usize;
    for character in value.chars() {
        match character {
            '(' => nesting += 1,
            ')' => nesting = nesting.saturating_sub(1),
            ',' if nesting == 0 => {
                parts.push(current.trim().to_owned());
                current.clear();
                continue;
            }
            _ => {}
        }
        current.push(character);
    }
    if !current.trim().is_empty() {
        parts.push(current.trim().to_owned());
    }
    parts
}

/// Convert the supported D2 CSS gradient syntax into a self-contained SVG
/// definition. Invalid gradients are left to the normal source validation
/// path and therefore do not produce a malformed `<defs>` block.
pub(crate) fn gradient_definition(value: &str) -> Option<String> {
    let value = value.trim();
    let (kind, params) = if let Some(params) = value
        .strip_prefix("linear-gradient(")
        .and_then(|rest| rest.strip_suffix(')'))
    {
        ("linear", params)
    } else {
        let params = value
            .strip_prefix("radial-gradient(")
            .and_then(|rest| rest.strip_suffix(')'))?;
        ("radial", params)
    };
    let mut parameters = split_gradient_params(params);
    if parameters.is_empty() {
        return None;
    }
    let direction = if (kind == "linear"
        && (parameters[0].ends_with("deg") || parameters[0].starts_with("to ")))
        || (kind == "radial" && matches!(parameters[0].as_str(), "circle" | "ellipse"))
    {
        Some(parameters.remove(0))
    } else {
        None
    };
    let stops = parameters
        .into_iter()
        .filter_map(|parameter| {
            let mut fields = parameter.split_whitespace();
            let color = fields.next()?.to_owned();
            let position = fields.next().map(str::to_owned);
            fields.next().is_none().then_some((color, position))
        })
        .collect::<Vec<_>>();
    if stops.is_empty() {
        return None;
    }
    let id = gradient_id(value);
    let mut output = String::new();
    if kind == "linear" {
        let (x1, y1, x2, y2) = match direction.as_deref() {
            Some(direction) if direction.starts_with("to ") => {
                let mut x1 = "50%".to_owned();
                let mut y1 = "50%".to_owned();
                let mut x2 = "50%".to_owned();
                let mut y2 = "50%".to_owned();
                for part in direction[3..].split_whitespace() {
                    match part {
                        "left" => {
                            x1 = "100%".to_owned();
                            x2 = "0%".to_owned();
                        }
                        "right" => {
                            x1 = "0%".to_owned();
                            x2 = "100%".to_owned();
                        }
                        "top" => {
                            y1 = "100%".to_owned();
                            y2 = "0%".to_owned();
                        }
                        "bottom" => {
                            y1 = "0%".to_owned();
                            y2 = "100%".to_owned();
                        }
                        _ => {}
                    }
                }
                (x1, y1, x2, y2)
            }
            Some(direction) if direction.ends_with("deg") => {
                let angle = direction
                    .trim_end_matches("deg")
                    .trim()
                    .parse::<f64>()
                    .ok()?;
                // d2scenebuild linearGradientVector keeps the unrounded
                // object-bounding-box end point that d2raster samples.
                let radians = (90.0 - angle) * std::f64::consts::PI / 180.0;
                let x2 = 0.5_f64.mul_add(radians.cos(), 0.5);
                let y2 = 0.5_f64.mul_add(radians.sin(), 0.5);
                (
                    "50%".to_owned(),
                    "50%".to_owned(),
                    format!("{x2}"),
                    format!("{y2}"),
                )
            }
            _ => (
                "0%".to_owned(),
                "0%".to_owned(),
                "0%".to_owned(),
                "100%".to_owned(),
            ),
        };
        let _ = writeln!(
            output,
            "<linearGradient id=\"{id}\" x1=\"{x1}\" y1=\"{y1}\" x2=\"{x2}\" y2=\"{y2}\">"
        );
    } else {
        let _ = writeln!(output, "<radialGradient id=\"{id}\">");
    }
    let count = stops.len();
    for (index, (color, position)) in stops.into_iter().enumerate() {
        let offset = position.unwrap_or_else(|| {
            if count == 1 {
                "0.00%".to_owned()
            } else {
                format!("{:.2}%", index as f64 / (count - 1) as f64 * 100.0)
            }
        });
        let _ = writeln!(
            output,
            "<stop offset=\"{}\" stop-color=\"{}\" />",
            escape(&offset),
            escape(&color),
        );
    }
    let _ = writeln!(output, "</{}Gradient>", kind,);
    Some(output)
}

pub(crate) fn attributes(style: &SourceNodeStyle, palette: PaletteView<'_>) -> (String, String) {
    let default_fill = palette.color("B6", palette.base.b6);
    let default_stroke = palette.color("B1", palette.base.b1);
    let fill = themed_color(style.fill.as_deref(), &default_fill, palette);
    let gradient_fill = gradient_definition(&fill)
        .map(|_| format!("url(#{})", gradient_id(&fill)))
        .unwrap_or(fill);
    let stroke = themed_color(style.stroke.as_deref(), &default_stroke, palette);
    let stroke = gradient_definition(&stroke)
        .map(|_| format!("url(#{})", gradient_id(&stroke)))
        .unwrap_or(stroke);
    let fill_class = if style
        .fill_pattern
        .as_deref()
        .is_some_and(|pattern| !matches!(pattern, "none" | "solid"))
    {
        String::new()
    } else {
        palette
            .semantic_code(style.fill.as_deref(), "B6")
            .map(|code| format!("d2-rs-fill-{code}"))
            .unwrap_or_default()
    };
    let stroke_class = palette
        .semantic_code(style.stroke.as_deref(), "B1")
        .map(|code| format!("d2-rs-stroke-{code}"))
        .unwrap_or_default();
    let pattern_class = style
        .fill_pattern
        .as_deref()
        .filter(|pattern| patternassets::id(pattern).is_some())
        .map(|pattern| format!("{pattern}-overlay"))
        .unwrap_or_default();
    let stroke_width = style.stroke_width.as_deref().unwrap_or("1");
    let pattern_fill = style
        .fill_pattern
        .as_deref()
        .filter(|pattern| !matches!(*pattern, "none" | "solid"))
        .and_then(patternassets::id)
        .map(|pattern| format!("url(#{pattern})"));
    let fill = pattern_fill.as_deref().unwrap_or(&gradient_fill);
    let mut common = format!(
        "stroke=\"{}\" stroke-width=\"{}\"",
        escape(&stroke),
        escape(stroke_width)
    );
    if let Some(opacity) = style.opacity.as_deref() {
        let _ = write!(common, " opacity=\"{}\"", escape(opacity));
    }
    if let Some(dash) = style.stroke_dash.as_deref().and_then(|value| {
        stroke_dasharray(stroke_width, Some(value)).or_else(|| Some(value.to_owned()))
    }) {
        let _ = write!(common, " stroke-dasharray=\"{}\"", escape(&dash));
    }
    let filled_classes = [
        fill_class.as_str(),
        stroke_class.as_str(),
        pattern_class.as_str(),
    ]
    .into_iter()
    .filter(|class| !class.is_empty())
    .collect::<Vec<_>>()
    .join(" ");
    let filled_class_attribute = if filled_classes.is_empty() {
        String::new()
    } else {
        format!(" class=\"{filled_classes}\"")
    };
    let open_classes = [stroke_class.as_str(), pattern_class.as_str()]
        .into_iter()
        .filter(|class| !class.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let open_class_attribute = if open_classes.is_empty() {
        String::new()
    } else {
        format!(" class=\"{open_classes}\"")
    };
    (
        format!(
            "fill=\"{}\"{} {common}",
            escape(fill),
            filled_class_attribute
        ),
        format!("fill=\"none\"{open_class_attribute} {common}"),
    )
}

fn write_path(svg: &mut String, data: &str, attrs: &str) {
    let _ = writeln!(svg, "<path d=\"{data}\" {attrs}/>");
}

fn add_svg_class(attrs: &str, class: &str) -> String {
    if let Some(class_start) = attrs.find(" class=\"") {
        let value_start = class_start + " class=\"".len();
        format!(
            "{}{} {}",
            &attrs[..value_start],
            class,
            &attrs[value_start..]
        )
    } else {
        format!("class=\"{class}\" {attrs}")
    }
}

fn style_true(value: Option<&String>) -> bool {
    value
        .and_then(|value| d2_rs_core::parse_d2_bool(value))
        .unwrap_or(false)
}

fn svg_attribute(attrs: &str, name: &str) -> Option<String> {
    let prefix = format!("{name}=\"");
    let start = attrs.find(&prefix)? + prefix.len();
    let end = attrs[start..].find('\"')? + start;
    Some(attrs[start..end].to_owned())
}

fn set_svg_attribute(attrs: &str, name: &str, value: &str) -> String {
    let prefix = format!("{name}=\"");
    if let Some(start) = attrs.find(&prefix) {
        let value_start = start + prefix.len();
        if let Some(relative_end) = attrs[value_start..].find('\"') {
            let end = value_start + relative_end;
            return format!("{}{value}{}", &attrs[..value_start], &attrs[end..]);
        }
    }
    format!("{attrs} {name}=\"{value}\"")
}

fn remove_svg_attribute(attrs: &str, name: &str) -> String {
    let prefix = format!(" {name}=\"");
    let Some(start) = attrs.find(&prefix) else {
        return attrs.to_owned();
    };
    let value_start = start + prefix.len();
    let Some(relative_end) = attrs[value_start..].find('\"') else {
        return attrs.to_owned();
    };
    let end = value_start + relative_end + 1;
    format!("{}{}", &attrs[..start], &attrs[end..])
}

fn darken_color(raw: &str) -> Option<String> {
    let color = raw.parse::<svgtypes::Color>().ok()?;
    let red = f64::from(color.red) / 255.0;
    let green = f64::from(color.green) / 255.0;
    let blue = f64::from(color.blue) / 255.0;
    let minimum = red.min(green).min(blue);
    let maximum = red.max(green).max(blue);
    let mut hue = 0.0;
    let lightness = (maximum + minimum) / 2.0;
    let delta = maximum - minimum;
    let saturation = if delta == 0.0 {
        0.0
    } else {
        delta / (1.0 - (2.0 * lightness - 1.0).abs())
    };
    if delta != 0.0 {
        hue = if maximum == red {
            ((green - blue) / delta).rem_euclid(6.0)
        } else if maximum == green {
            (blue - red) / delta + 2.0
        } else {
            (red - green) / delta + 4.0
        } / 6.0;
    }
    let lightness = (lightness - 0.1).clamp(0.0, 1.0);
    let chroma = (1.0 - (2.0 * lightness - 1.0).abs()) * saturation;
    let hue_sector = (hue * 6.0).rem_euclid(6.0);
    let intermediate = chroma * (1.0 - (hue_sector.rem_euclid(2.0) - 1.0).abs());
    let (red, green, blue) = match hue_sector as u8 {
        0 => (chroma, intermediate, 0.0),
        1 => (intermediate, chroma, 0.0),
        2 => (0.0, chroma, intermediate),
        3 => (0.0, intermediate, chroma),
        4 => (intermediate, 0.0, chroma),
        _ => (chroma, 0.0, intermediate),
    };
    let match_value = lightness - chroma / 2.0;
    Some(format!(
        "#{:02x}{:02x}{:02x}",
        ((red + match_value) * 255.0).round() as u8,
        ((green + match_value) * 255.0).round() as u8,
        ((blue + match_value) * 255.0).round() as u8,
    ))
}

fn point_path(commands: &[(char, f64, f64)]) -> String {
    commands
        .iter()
        .map(|(command, x, y)| format!("{command}{} {}", number(*x), number(*y)))
        .collect::<Vec<_>>()
        .join(" ")
}

fn three_dee_rectangle_paths(rect: Rect) -> (String, String) {
    let x = rect.origin.x;
    let y = rect.origin.y;
    let width = rect.size.width;
    let height = rect.size.height;
    let border = point_path(&[
        ('M', x, y),
        ('L', x + THREE_DEE_OFFSET, y - THREE_DEE_OFFSET),
        ('L', x + width + THREE_DEE_OFFSET, y - THREE_DEE_OFFSET),
        (
            'L',
            x + width + THREE_DEE_OFFSET,
            y + height - THREE_DEE_OFFSET,
        ),
        ('L', x + width, y + height),
        ('L', x, y + height),
        ('L', x, y),
        ('L', x + width, y),
        ('L', x + width, y + height),
        ('M', x + width, y),
        ('L', x + width + THREE_DEE_OFFSET, y - THREE_DEE_OFFSET),
    ]);
    let side = [
        Point { x, y },
        Point {
            x: x + THREE_DEE_OFFSET,
            y: y - THREE_DEE_OFFSET,
        },
        Point {
            x: x + width + THREE_DEE_OFFSET,
            y: y - THREE_DEE_OFFSET,
        },
        Point {
            x: x + width + THREE_DEE_OFFSET,
            y: y + height - THREE_DEE_OFFSET,
        },
        Point {
            x: x + width,
            y: y + height,
        },
        Point { x: x + width, y },
    ];
    (border, polygon(&side))
}

fn three_dee_hexagon_paths(rect: Rect) -> (String, String, String) {
    let x = rect.origin.x;
    let y = rect.origin.y;
    let width = rect.size.width;
    let height = rect.size.height;
    let quarter = (width * 0.25).trunc();
    let three_quarters = (width * 0.75).trunc();
    let mid_y = (height * (43.6 / 87.3)).trunc();
    let vertical_offset = (THREE_DEE_OFFSET / 2.0).trunc();
    let border = point_path(&[
        ('M', x + quarter, y),
        ('L', x + quarter + THREE_DEE_OFFSET, y - vertical_offset),
        (
            'L',
            x + three_quarters + THREE_DEE_OFFSET,
            y - vertical_offset,
        ),
        (
            'L',
            x + width + THREE_DEE_OFFSET,
            y + mid_y - vertical_offset,
        ),
        (
            'L',
            x + three_quarters + THREE_DEE_OFFSET,
            y + height - vertical_offset,
        ),
        ('L', x + three_quarters, y + height),
        ('L', x + quarter, y + height),
        ('L', x, y + mid_y),
        ('L', x + quarter, y),
        ('L', x + three_quarters, y),
        ('L', x + width, y + mid_y),
        ('L', x + three_quarters, y + height),
        ('M', x + three_quarters, y),
        (
            'L',
            x + three_quarters + THREE_DEE_OFFSET,
            y - vertical_offset,
        ),
        ('M', x + width, y + mid_y),
        (
            'L',
            x + width + THREE_DEE_OFFSET,
            y + mid_y - vertical_offset,
        ),
        ('M', x + three_quarters, y + height),
        (
            'L',
            x + three_quarters + THREE_DEE_OFFSET,
            y + height - vertical_offset,
        ),
    ]);
    let main = polygon(&[
        Point { x: x + quarter, y },
        Point {
            x: x + three_quarters,
            y,
        },
        Point {
            x: x + width,
            y: y + mid_y,
        },
        Point {
            x: x + three_quarters,
            y: y + height,
        },
        Point {
            x: x + quarter,
            y: y + height,
        },
        Point { x, y: y + mid_y },
    ]);
    let side = polygon(&[
        Point {
            x: x + quarter + THREE_DEE_OFFSET,
            y: y - vertical_offset,
        },
        Point {
            x: x + three_quarters + THREE_DEE_OFFSET,
            y: y - vertical_offset,
        },
        Point {
            x: x + width + THREE_DEE_OFFSET,
            y: y + mid_y - vertical_offset,
        },
        Point {
            x: x + three_quarters + THREE_DEE_OFFSET,
            y: y + height - vertical_offset,
        },
        Point {
            x: x + three_quarters,
            y: y + height,
        },
        Point {
            x: x + width,
            y: y + mid_y,
        },
        Point {
            x: x + three_quarters,
            y,
        },
        Point { x: x + quarter, y },
    ]);
    (border, main, side)
}

fn draw_three_dee_shape(
    svg: &mut String,
    shape: ShapeKind,
    rect: Rect,
    style: &SourceNodeStyle,
    palette: PaletteView<'_>,
    shape_id: &str,
    native_scene: bool,
) {
    let (border_path, main_points, side_points) = if shape == ShapeKind::Hexagon {
        let (border, main, side) = three_dee_hexagon_paths(rect);
        (border, Some(main), side)
    } else {
        let (border, side) = three_dee_rectangle_paths(rect);
        (border, None, side)
    };
    let id_source = if shape_id.is_empty() {
        format!(
            "{}-{}-{}-{}",
            number(rect.origin.x),
            number(rect.origin.y),
            number(rect.size.width),
            number(rect.size.height),
        )
    } else {
        shape_id.to_owned()
    };
    let mask_id = format!(
        "d2-rs-3d-mask-{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(id_source.as_bytes())
    );
    let mut paint_style = style.clone();
    if native_scene {
        // Scene patterns overlay only the main face, before side/border paints.
        paint_style.fill_pattern = None;
    }
    let (filled, open) = attributes(&paint_style, palette);
    let fallback_fill = palette.color("B6", palette.base.b6);
    let resolved_fill = themed_color(style.fill.as_deref(), &fallback_fill, palette);
    let fallback_side_fill =
        svg_attribute(&filled, "fill").unwrap_or_else(|| resolved_fill.clone());
    // lib/color.Darken selects a darker semantic theme slot before palette
    // resolution; darkening its resolved RGB instead loses theme overrides.
    let source_fill = style.fill.as_deref().unwrap_or("B6");
    let darker_slot = match source_fill {
        "B1" | "B2" => Some("B1"),
        "B3" => Some("B2"),
        "B4" => Some("B3"),
        "B5" => Some("B4"),
        "B6" => Some("B5"),
        "N1" | "N2" => Some("N1"),
        "N3" => Some("N2"),
        "N4" => Some("N3"),
        "N5" => Some("N4"),
        "N6" => Some("N5"),
        "N7" => Some("N6"),
        "AA2" | "AA4" => Some("AA2"),
        "AA5" => Some("AA4"),
        "AB4" => Some("AB4"),
        "AB5" => Some("AB5"),
        _ => None,
    };
    let side_fill = darker_slot
        .map(|slot| palette.color(slot, &fallback_side_fill))
        .or_else(|| darken_color(source_fill))
        .unwrap_or(fallback_side_fill);
    let main_attrs = remove_svg_attribute(
        &remove_svg_attribute(&set_svg_attribute(&filled, "stroke", "none"), "opacity"),
        "class",
    );
    // d2scenebuild overlayPatternNode composites a pattern overlay with
    // BlendMultiply; the `<pattern>-overlay` class carries that blend.
    let pattern_overlay_class = style
        .fill_pattern
        .as_deref()
        .filter(|pattern| patternassets::id(pattern).is_some())
        .map(|pattern| format!("{pattern}-overlay"));
    let with_pattern_class = |attrs: String| match &pattern_overlay_class {
        Some(class) if attrs.contains("fill=\"url(#") => set_svg_attribute(&attrs, "class", class),
        _ => attrs,
    };
    let main_attrs = with_pattern_class(main_attrs);
    let mut side_attrs =
        remove_svg_attribute(&set_svg_attribute(&filled, "stroke", "none"), "opacity");
    side_attrs = remove_svg_attribute(&side_attrs, "class");
    side_attrs = set_svg_attribute(&side_attrs, "fill", &escape(&side_fill));
    let border_attrs = remove_svg_attribute(&remove_svg_attribute(&open, "opacity"), "class");
    let mask_path_attrs = remove_svg_attribute(&open, "opacity");
    let mask_path_attrs = remove_svg_attribute(&mask_path_attrs, "class");
    let mask_path_attrs = set_svg_attribute(&mask_path_attrs, "stroke", "#000000");
    let mask_path_attrs = set_svg_attribute(&mask_path_attrs, "opacity", "1");
    if let Some(opacity) = style.opacity.as_deref() {
        let _ = writeln!(svg, "<g opacity=\"{}\">", escape(opacity));
    }
    if native_scene {
        // d2scenebuild/shape_effects.go paints main, sides, border directly.
        // Its native raster scene has no d2svg outline subtraction mask.
        let main = |svg: &mut String, attrs: &str| {
            if let Some(points) = main_points.as_deref() {
                let _ = writeln!(svg, "<polygon points=\"{points}\" {attrs}/>");
            } else {
                let _ = writeln!(
                    svg,
                    "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" {attrs}/>",
                    number(rect.origin.x),
                    number(rect.origin.y),
                    number(rect.size.width),
                    number(rect.size.height)
                );
            }
        };
        main(svg, &main_attrs);
        if let Some(pattern) = style.fill_pattern.as_deref().and_then(patternassets::id) {
            let overlay_attrs = with_pattern_class(set_svg_attribute(
                &main_attrs,
                "fill",
                &format!("url(#{pattern})"),
            ));
            main(svg, &overlay_attrs);
        }
        let _ = writeln!(svg, "<polygon points=\"{side_points}\" {side_attrs}/>");
        let _ = writeln!(
            svg,
            "<path id=\"{}-3d-border\" data-d2-rs-native-3d-border=\"scene\" d=\"{border_path}\" {border_attrs} stroke-linejoin=\"round\" stroke-linecap=\"butt\"/>",
            escape(shape_id)
        );
        if style.opacity.is_some() {
            svg.push_str("</g>\n");
        }
        return;
    }
    let _ = writeln!(
        svg,
        "<defs><mask id=\"{mask_id}\" maskUnits=\"userSpaceOnUse\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\">",
        number(rect.origin.x),
        number(rect.origin.y - THREE_DEE_OFFSET),
        number(rect.size.width + THREE_DEE_OFFSET),
        number(rect.size.height + THREE_DEE_OFFSET),
    );
    let _ = writeln!(
        svg,
        "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"white\"/>",
        number(rect.origin.x),
        number(rect.origin.y - THREE_DEE_OFFSET),
        number(rect.size.width + THREE_DEE_OFFSET),
        number(rect.size.height + THREE_DEE_OFFSET),
    );
    let _ = writeln!(
        svg,
        "<path d=\"{}\" {}/></mask></defs>",
        border_path, mask_path_attrs,
    );
    let mask_attr = format!(" mask=\"url(#{mask_id})\"");
    let _ = if let Some(points) = main_points {
        writeln!(
            svg,
            "<polygon points=\"{points}\" {main_attrs}{mask_attr}/>",
        )
    } else {
        writeln!(
            svg,
            "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" {main_attrs}{mask_attr}/>",
            number(rect.origin.x),
            number(rect.origin.y),
            number(rect.size.width),
            number(rect.size.height),
        )
    };
    let _ = writeln!(
        svg,
        "<polygon points=\"{side_points}\" {side_attrs}{mask_attr}/>",
    );
    let _ = writeln!(svg, "<path d=\"{border_path}\" {border_attrs}/>");
    if style.opacity.is_some() {
        svg.push_str("</g>\n");
    }
}

pub(crate) fn draw_native_three_dee_shape(
    svg: &mut String,
    shape: ShapeKind,
    rect: Rect,
    style: &SourceNodeStyle,
    palette: PaletteView<'_>,
    shape_id: &str,
) {
    draw_three_dee_shape(svg, shape, rect, style, palette, shape_id, true);
}

fn path_number(value: f64) -> String {
    number(value)
}

fn d2_path_precision(value: f64) -> f64 {
    // lib/shape's SVG path context quantizes coordinates to float32-backed
    // four-decimal values before exposing them to the renderers.
    let rounded = (f64::from((value * 10_000.0) as f32) / 10_000.0).round();
    if rounded == 0.0 { 0.0 } else { rounded }
}

struct PathBuilder {
    data: String,
    current: Option<(f64, f64)>,
    quantize: bool,
}

impl PathBuilder {
    fn new() -> Self {
        Self::with_quantization(false)
    }

    fn quantized() -> Self {
        Self::with_quantization(true)
    }

    fn with_quantization(quantize: bool) -> Self {
        Self {
            data: String::new(),
            current: None,
            quantize,
        }
    }

    fn coordinate(&self, value: f64) -> f64 {
        if self.quantize {
            d2_path_precision(value)
        } else {
            value
        }
    }

    fn command(&mut self, name: &str, values: &[f64]) -> &mut Self {
        if !self.data.is_empty() {
            self.data.push(' ');
        }
        self.data.push_str(name);
        for value in values {
            self.data.push(' ');
            self.data.push_str(&path_number(self.coordinate(*value)));
        }
        self
    }

    fn m(&mut self, x: f64, y: f64) -> &mut Self {
        let x = self.coordinate(x);
        let y = self.coordinate(y);
        self.command("M", &[x, y]);
        self.current = Some((x, y));
        self
    }

    fn l(&mut self, x: f64, y: f64) -> &mut Self {
        let x = self.coordinate(x);
        let y = self.coordinate(y);
        self.command("L", &[x, y]);
        self.current = Some((x, y));
        self
    }

    fn c(&mut self, c1x: f64, c1y: f64, c2x: f64, c2y: f64, x: f64, y: f64) -> &mut Self {
        let c1x = self.coordinate(c1x);
        let c1y = self.coordinate(c1y);
        let c2x = self.coordinate(c2x);
        let c2y = self.coordinate(c2y);
        let x = self.coordinate(x);
        let y = self.coordinate(y);
        self.command("C", &[c1x, c1y, c2x, c2y, x, y]);
        self.current = Some((x, y));
        self
    }

    fn h(&mut self, x: f64) -> &mut Self {
        let x = self.coordinate(x);
        self.command("H", &[x]);
        if let Some((_, y)) = self.current {
            self.current = Some((x, y));
        }
        self
    }

    fn v(&mut self, y: f64) -> &mut Self {
        let y = self.coordinate(y);
        self.command("V", &[y]);
        if let Some((x, _)) = self.current {
            self.current = Some((x, y));
        }
        self
    }

    fn rc(&mut self, c1x: f64, c1y: f64, c2x: f64, c2y: f64, dx: f64, dy: f64) -> &mut Self {
        let Some((x, y)) = self.current else {
            return self;
        };
        self.c(x + c1x, y + c1y, x + c2x, y + c2y, x + dx, y + dy)
    }

    fn rh(&mut self, dx: f64) -> &mut Self {
        let Some((x, _)) = self.current else {
            return self;
        };
        self.h(x + dx)
    }

    fn rl(&mut self, dx: f64, dy: f64) -> &mut Self {
        let Some((x, y)) = self.current else {
            return self;
        };
        self.l(x + dx, y + dy)
    }

    fn rv(&mut self, dy: f64) -> &mut Self {
        let Some((_, y)) = self.current else {
            return self;
        };
        self.v(y + dy)
    }

    fn close(&mut self) -> &mut Self {
        if !self.data.is_empty() {
            self.data.push(' ');
        }
        self.data.push('Z');
        self
    }

    fn finish(self) -> String {
        self.data
    }
}

fn parallelogram_path(x: f64, y: f64, width: f64, height: f64) -> String {
    let mut wedge = PARALLEL_WEDGE_WIDTH;
    if width <= wedge {
        wedge = width / 2.0;
    }
    let mut path = PathBuilder::quantized();
    path.m(x + wedge, y)
        .l(x + width, y)
        .l(x + width - wedge, y + height)
        .l(x, y + height)
        .l(x, y + height)
        .close();
    path.finish()
}

fn diamond_path(x: f64, y: f64, width: f64, height: f64) -> String {
    let scale_x = width / 77.0;
    let scale_y = height / 76.9;
    let absolute = |px: f64, py: f64| {
        (
            d2_path_precision(x + scale_x * px),
            d2_path_precision(y + scale_y * py),
        )
    };
    let relative = |base: (f64, f64), dx: f64, dy: f64| {
        (
            d2_path_precision(base.0 + scale_x * dx),
            d2_path_precision(base.1 + scale_y * dy),
        )
    };
    let mut path = PathBuilder::new();
    let start = absolute(38.5, 76.9);
    path.m(start.0, start.1);
    let c1 = relative(start, -0.3, 0.0);
    let c2 = relative(start, -0.5, -0.1);
    let end = relative(start, -0.7, -0.3);
    path.c(c1.0, c1.1, c2.0, c2.1, end.0, end.1);
    let line = absolute(0.3, 39.2);
    path.l(line.0, line.1);
    let c1 = relative(line, -0.4, -0.4);
    let c2 = relative(line, -0.4, -1.0);
    let end = relative(line, 0.0, -1.4);
    path.c(c1.0, c1.1, c2.0, c2.1, end.0, end.1);
    let line = absolute(37.8, 0.3);
    path.l(line.0, line.1);
    let c1 = relative(line, 0.4, -0.4);
    let c2 = relative(line, 1.0, -0.4);
    let end = relative(line, 1.4, 0.0);
    path.c(c1.0, c1.1, c2.0, c2.1, end.0, end.1);
    let line = relative(end, 37.5, 37.5);
    path.l(line.0, line.1);
    let c2 = relative(line, 0.4, 0.4);
    let c3 = relative(line, 0.4, 1.0);
    let end = relative(line, 0.0, 1.4);
    path.c(c2.0, c2.1, c3.0, c3.1, end.0, end.1);
    let line = absolute(39.2, 76.6);
    path.l(line.0, line.1);
    let c1 = absolute(39.0, 76.8);
    let c2 = absolute(38.8, 76.9);
    let end = absolute(38.5, 76.9);
    path.c(c1.0, c1.1, c2.0, c2.1, end.0, end.1).close();
    path.finish()
}

fn document_path(x: f64, y: f64, width: f64, height: f64) -> String {
    let bottom = DOC_PATH_BOTTOM / DOC_PATH_HEIGHT;
    let c1_y = 12.8 / DOC_PATH_HEIGHT;
    let c2_y = 19.8 / DOC_PATH_HEIGHT;
    let abs_x = |relative_x: f64| x + width * relative_x;
    let abs_y = |relative_y: f64| y + height * relative_y;
    let mut path = PathBuilder::quantized();
    path.m(abs_x(0.0), abs_y(bottom))
        .l(abs_x(0.0), abs_y(0.0))
        .l(abs_x(1.0), abs_y(0.0))
        .l(abs_x(1.0), abs_y(bottom))
        .c(
            abs_x(5.0 / 6.0),
            abs_y(c1_y),
            abs_x(2.0 / 3.0),
            abs_y(c1_y),
            abs_x(1.0 / 2.0),
            abs_y(bottom),
        )
        .c(
            abs_x(1.0 / 3.0),
            abs_y(c2_y),
            abs_x(1.0 / 6.0),
            abs_y(c2_y),
            abs_x(0.0),
            abs_y(bottom),
        )
        .close();
    path.finish()
}

fn arc_height(height: f64) -> f64 {
    if height < DEFAULT_ARC_DEPTH * 2.0 {
        height / 2.0
    } else {
        DEFAULT_ARC_DEPTH
    }
}

fn cylinder_paths(x: f64, y: f64, width: f64, height: f64) -> (String, String) {
    let arc = arc_height(height);
    let multiplier = 0.45;
    let abs_x = |local_x: f64| x + local_x;
    let abs_y = |local_y: f64| y + local_y;

    // The Go implementation sends each point through SvgPathContext's
    // float32-backed chopPrecision. Keep the source's absolute cubic points
    // and relative vertical moves so rounding occurs after the same additions.
    let mut outer = PathBuilder::quantized();
    outer
        .m(abs_x(0.0), abs_y(arc))
        .c(
            abs_x(0.0),
            abs_y(0.0),
            abs_x(width * multiplier),
            abs_y(0.0),
            abs_x(width / 2.0),
            abs_y(0.0),
        )
        .c(
            abs_x(width - width * multiplier),
            abs_y(0.0),
            abs_x(width),
            abs_y(0.0),
            abs_x(width),
            abs_y(arc),
        )
        .rv(height - arc * 2.0)
        .c(
            abs_x(width),
            abs_y(height),
            abs_x(width - width * multiplier),
            abs_y(height),
            abs_x(width / 2.0),
            abs_y(height),
        )
        .c(
            abs_x(width * multiplier),
            abs_y(height),
            abs_x(0.0),
            abs_y(height),
            abs_x(0.0),
            abs_y(height - arc),
        )
        .rv(-(height - arc * 2.0))
        .close();

    let mut inner = PathBuilder::quantized();
    inner
        .m(abs_x(0.0), abs_y(arc))
        .c(
            abs_x(0.0),
            abs_y(arc * 2.0),
            abs_x(width * multiplier),
            abs_y(arc * 2.0),
            abs_x(width / 2.0),
            abs_y(arc * 2.0),
        )
        .c(
            abs_x(width - width * multiplier),
            abs_y(arc * 2.0),
            abs_x(width),
            abs_y(arc * 2.0),
            abs_x(width),
            abs_y(arc),
        );
    (outer.finish(), inner.finish())
}

fn arc_width(width: f64) -> f64 {
    if width < DEFAULT_ARC_DEPTH * 2.0 {
        width / 2.0
    } else {
        DEFAULT_ARC_DEPTH
    }
}

fn queue_paths(x: f64, y: f64, width: f64, height: f64) -> (String, String) {
    let arc = arc_width(width);
    let multiplier = 0.45;
    // Shape paths pass through D2's SvgPathContext, which applies its
    // float32-backed chopPrecision conversion to every absolute endpoint.
    let mut outer = PathBuilder::quantized();
    outer
        .m(x + arc, y)
        .rh(width - 2.0 * arc)
        .c(
            x + width,
            y,
            x + width,
            y + height * multiplier,
            x + width,
            y + height / 2.0,
        )
        .c(
            x + width,
            y + height - height * multiplier,
            x + width,
            y + height,
            x + width - arc,
            y + height,
        )
        .rh(-(width - 2.0 * arc))
        .c(
            x,
            y + height,
            x,
            y + height - height * multiplier,
            x,
            y + height / 2.0,
        )
        .c(x, y + height * multiplier, x, y, x + arc, y)
        .close();
    let mut inner = PathBuilder::quantized();
    inner
        .m(x + width - arc, y)
        .c(
            x + width - 2.0 * arc,
            y,
            x + width - 2.0 * arc,
            y + height * multiplier,
            x + width - 2.0 * arc,
            y + height / 2.0,
        )
        .c(
            x + width - 2.0 * arc,
            y + height - height * multiplier,
            x + width - 2.0 * arc,
            y + height,
            x + width - arc,
            y + height,
        );
    (outer.finish(), inner.finish())
}

fn page_paths(x: f64, y: f64, width: f64, height: f64) -> (String, String) {
    // D2's lib/svg path context applies chopPrecision to every page control
    // point, which intentionally rounds this source curve to target integers.
    let abs_x = |local_x: f64| x + local_x;
    let abs_y = |local_y: f64| y + local_y;
    let mut outer = PathBuilder::quantized();
    outer
        .m(abs_x(0.5), abs_y(0.0))
        .h(abs_x(width - PAGE_CORNER_WIDTH))
        .c(
            abs_x(width - 19.6456),
            abs_y(0.0),
            abs_x(width - 18.521),
            abs_y(0.456297),
            abs_x(width - 17.6811),
            abs_y(1.27202),
        )
        .l(abs_x(width - 1.3647), abs_y(17.12))
        .c(
            abs_x(width - 0.4923),
            abs_y(17.9674),
            abs_x(width),
            abs_y(19.1318),
            abs_x(width),
            abs_y(PAGE_CORNER_HEIGHT),
        )
        .v(abs_y(height - 0.5))
        .c(
            abs_x(width),
            abs_y(height - 0.2239),
            abs_x(width - 0.2239),
            abs_y(height),
            abs_x(width - 0.5),
            abs_y(height),
        )
        .h(abs_x(0.499999))
        .c(
            abs_x(0.223857),
            abs_y(height),
            abs_x(0.0),
            abs_y(height - 0.2239),
            abs_x(0.0),
            abs_y(height - 0.5),
        )
        .v(abs_y(0.499999))
        .c(
            abs_x(0.0),
            abs_y(0.223857),
            abs_x(0.223857),
            abs_y(0.0),
            abs_x(0.5),
            abs_y(0.0),
        )
        .close();
    // The inner line is the folded-page detail.  It is kept as a separate
    // open path, matching D2's second GetSVGPathData entry.
    let mut inner = PathBuilder::quantized();
    inner
        .m(abs_x(width - 1.08197), abs_y(height))
        .h(abs_x(1.08196))
        .rc(-0.64918, 0.0, -1.08196, -0.43287, -1.08196, -1.08219)
        .v(abs_y(1.08219))
        .rc(0.0, -0.64931, 0.43278, -1.08219, 1.08196, -1.08219)
        .rh(width - 22.72132)
        .rc(0.64918, 0.0, 1.08196, 0.43287, 1.08196, 1.08219)
        .rv(17.09863)
        .rc(0.0, 1.29863, 0.86557, 2.38082, 2.38032, 2.38082)
        .h(abs_x(width - 1.08197))
        .rc(0.64918, 0.0, 1.08196, 0.43287, 1.08196, 1.08196)
        .v(abs_y(height - 1.0822))
        .c(
            abs_x(width - 1.0),
            abs_y(height - 0.43288),
            abs_x(width - 0.43279),
            abs_y(height),
            abs_x(width - 1.08197),
            abs_y(height),
        )
        .close();
    (outer.finish(), inner.finish())
}

#[allow(clippy::manual_clamp)]
fn package_dimensions(width: f64, height: f64) -> (f64, f64) {
    let mut top_width = width * PACKAGE_HORIZONTAL_SCALAR;
    if width >= 2.0 * PACKAGE_TOP_MIN_WIDTH {
        top_width = top_width
            .min(PACKAGE_TOP_MAX_WIDTH)
            .max(PACKAGE_TOP_MIN_WIDTH);
    }
    let top_height = (height * PACKAGE_VERTICAL_SCALAR).min(PACKAGE_TOP_MAX_HEIGHT);
    (top_width, top_height)
}

fn package_path(x: f64, y: f64, width: f64, height: f64) -> String {
    let (top_width, top_height) = package_dimensions(width, height);
    // lib/shape's SvgPathContext applies chopPrecision to every absolute
    // coordinate. That final math.Round is observable for package tabs such
    // as a 192px vessel (0.2 * 192 = 38.4 -> 38).
    let path_number = |value: f64| number(d2_path_precision(value));
    format!(
        "M {} {} L {} {} L {} {} L {} {} L {} {} L {} {} Z",
        path_number(x),
        path_number(y),
        path_number(x + top_width),
        path_number(y),
        path_number(x + top_width),
        path_number(y + top_height),
        path_number(x + width),
        path_number(y + top_height),
        path_number(x + width),
        path_number(y + height),
        path_number(x),
        path_number(y + height)
    )
}

fn step_path(x: f64, y: f64, width: f64, height: f64) -> String {
    let wedge = if width <= STEP_WEDGE_WIDTH {
        width / 2.0
    } else {
        STEP_WEDGE_WIDTH
    };
    let abs_x = |local_x: f64| x + local_x;
    let abs_y = |local_y: f64| y + local_y;
    let mut path = PathBuilder::quantized();
    path.m(abs_x(0.0), abs_y(0.0))
        .l(abs_x(width - wedge), abs_y(0.0))
        .l(abs_x(width), abs_y(height / 2.0))
        .l(abs_x(width - wedge), abs_y(height))
        .l(abs_x(0.0), abs_y(height))
        .l(abs_x(wedge), abs_y(height / 2.0))
        .close();
    path.finish()
}

fn tip_width(width: f64) -> f64 {
    if width < DEFAULT_TIP_WIDTH * 2.0 {
        width / 2.0
    } else {
        DEFAULT_TIP_WIDTH
    }
}

fn tip_height(height: f64) -> f64 {
    if height < DEFAULT_TIP_HEIGHT * 2.0 {
        height / 2.0
    } else {
        DEFAULT_TIP_HEIGHT
    }
}

fn callout_path(x: f64, y: f64, width: f64, height: f64) -> String {
    let tip_w = tip_width(width);
    let tip_h = tip_height(height);
    let mut path = PathBuilder::quantized();
    path.m(x, y)
        .rv(height - tip_h)
        .rh(width / 2.0)
        .rv(tip_h)
        .rl(tip_w, -tip_h)
        .rh(width / 2.0 - tip_w)
        .rv(-(height - tip_h))
        .rh(-width)
        .close();
    path.finish()
}

fn stored_data_path(x: f64, y: f64, width: f64, height: f64) -> String {
    let mut wedge = STORED_DATA_WEDGE_WIDTH;
    if width < wedge * 2.0 {
        wedge = width / 2.0;
    }
    let multiplier = 0.27;
    let abs_x = |local_x: f64| x + local_x;
    let abs_y = |local_y: f64| y + local_y;
    let mut path = PathBuilder::quantized();
    path.m(abs_x(wedge), abs_y(0.0))
        .rh(width - wedge)
        .c(
            abs_x(width - wedge * multiplier),
            abs_y(0.0),
            abs_x(width - wedge),
            abs_y(height * multiplier),
            abs_x(width - wedge),
            abs_y(height / 2.0),
        )
        .c(
            abs_x(width - wedge),
            abs_y(height - height * multiplier),
            abs_x(width - wedge * multiplier),
            abs_y(height),
            abs_x(width),
            abs_y(height),
        )
        .rh(-(width - wedge))
        .c(
            abs_x(wedge - wedge * multiplier),
            abs_y(height),
            abs_x(0.0),
            abs_y(height - height * multiplier),
            abs_x(0.0),
            abs_y(height / 2.0),
        )
        .c(
            abs_x(0.0),
            abs_y(height * multiplier),
            abs_x(wedge - wedge * multiplier),
            abs_y(0.0),
            abs_x(wedge),
            abs_y(0.0),
        )
        .close();
    path.finish()
}

fn person_path(x: f64, y: f64, width: f64, height: f64) -> String {
    // The upstream path is authored in a 68.3 × 77.4 coordinate system.
    let scale_x = width / 68.3;
    let scale_y = height / 77.4;
    let sx = |value: f64| scale_x * value;
    let sy = |value: f64| scale_y * value;
    let abs_x = |value: f64| x + sx(value);
    let abs_y = |value: f64| y + sy(value);
    let mut path = PathBuilder::quantized();
    path.m(abs_x(68.3), abs_y(77.4))
        .h(abs_x(0.0))
        .rv(sy(-1.1))
        .rc(sx(0.0), sy(-13.2), sx(7.5), sy(-25.1), sx(19.3), sy(-30.8))
        .c(
            abs_x(12.8),
            abs_y(40.9),
            abs_x(8.9),
            abs_y(33.4),
            abs_x(8.9),
            abs_y(25.2),
        )
        .c(
            abs_x(8.9),
            abs_y(11.3),
            abs_x(20.2),
            abs_y(0.0),
            abs_x(34.1),
            abs_y(0.0),
        )
        .rc(sx(13.9), sy(0.0), sx(25.2), sy(11.3), sx(25.2), sy(25.2))
        .rc(sx(0.0), sy(8.2), sx(-3.8), sy(15.6), sx(-10.4), sy(20.4))
        .rc(sx(11.8), sy(5.7), sx(19.3), sy(17.6), sx(19.3), sy(30.8))
        .rv(sy(1.0))
        .h(abs_x(68.3))
        .close();
    path.finish()
}

fn c4_person_paths(x: f64, y: f64, width: f64, height: f64) -> (String, String) {
    let head_radius = width * C4_HEAD_RADIUS_FACTOR;
    let body_top = head_radius + head_radius * C4_BODY_TOP_FACTOR;
    let body_width = width;
    let body_height = height - body_top;
    let corner_radius = (width * C4_CORNER_RADIUS_FACTOR).min(body_height * 0.25);
    let k = 4.0 * (2.0_f64.sqrt() - 1.0) / 3.0;

    let mut body = PathBuilder::quantized();
    body.m(x, y + body_top + corner_radius)
        .rc(
            0.0,
            -k * corner_radius,
            k * corner_radius,
            -corner_radius,
            corner_radius,
            -corner_radius,
        )
        .rh(width - 2.0 * corner_radius)
        .rc(
            k * corner_radius,
            0.0,
            corner_radius,
            k * corner_radius,
            corner_radius,
            corner_radius,
        )
        .rv(body_height - 2.0 * corner_radius)
        .rc(
            0.0,
            k * corner_radius,
            -k * corner_radius,
            corner_radius,
            -corner_radius,
            corner_radius,
        )
        .rh(-(body_width - 2.0 * corner_radius))
        .rc(
            -k * corner_radius,
            0.0,
            -corner_radius,
            -k * corner_radius,
            -corner_radius,
            -corner_radius,
        )
        .close();

    let center_x = x + width / 2.0;
    let center_y = y + head_radius;
    let head_k = head_radius * 4.0 * (2.0_f64.sqrt() - 1.0) / 3.0;
    let mut head = PathBuilder::quantized();
    head.m(center_x, center_y - head_radius)
        .c(
            center_x + head_k,
            center_y - head_radius,
            center_x + head_radius,
            center_y - head_k,
            center_x + head_radius,
            center_y,
        )
        .c(
            center_x + head_radius,
            center_y + head_k,
            center_x + head_k,
            center_y + head_radius,
            center_x,
            center_y + head_radius,
        )
        .c(
            center_x - head_k,
            center_y + head_radius,
            center_x - head_radius,
            center_y + head_k,
            center_x - head_radius,
            center_y,
        )
        .c(
            center_x - head_radius,
            center_y - head_k,
            center_x - head_k,
            center_y - head_radius,
            center_x,
            center_y - head_radius,
        );
    (body.finish(), head.finish())
}

fn cloud_path(x: f64, y: f64, width: f64, height: f64) -> String {
    // D2's cloud outline is authored in an 834 × 523 design coordinate
    // system. Relative control points are scaled independently with the box.
    let sx = |value: f64| width * value / 834.0;
    let sy = |value: f64| height * value / 523.0;
    // The upstream SvgPathContext applies its historical chopPrecision bug to
    // every coordinate: float32 conversion followed by rounding to an integer.
    // Keep this quantization scoped to the cloud path while the other source
    // shaped paths retain their already verified formatting.
    let mut cloud = PathBuilder::quantized();
    cloud
        .m(x + sx(137.833), y + sy(182.833))
        .rc(
            0.0,
            sy(5.556),
            -sx(5.556),
            sy(11.111),
            -sx(11.111),
            sy(11.111),
        )
        .rc(
            -sx(70.833),
            sy(6.944),
            -sx(126.389),
            sy(77.778),
            -sx(126.389),
            sy(163.889),
        )
        .rc(
            0.0,
            sy(91.667),
            sx(62.5),
            sy(165.278),
            sx(141.667),
            sy(165.278),
        )
        .rh(sx(537.5))
        .rc(
            sx(84.723),
            0.0,
            sx(154.167),
            -sy(79.167),
            sx(154.167),
            -sy(175.0),
        )
        .rc(
            0.0,
            -sy(91.667),
            -sx(63.89),
            -sy(168.056),
            -sx(144.444),
            -sy(173.611),
        )
        .rc(
            -sx(5.556),
            0.0,
            -sx(11.111),
            -sy(4.167),
            -sx(12.5),
            -sy(11.111),
        )
        .rc(
            -sx(18.056),
            -sy(93.055),
            -sx(101.39),
            -sy(162.5),
            -sx(198.611),
            -sy(162.5),
        )
        .rc(
            -sx(63.889),
            0.0,
            -sx(120.834),
            sy(29.167),
            -sx(156.944),
            sy(75.0),
        )
        .rc(
            -sx(4.167),
            sy(5.556),
            -sx(11.111),
            sy(6.945),
            -sx(15.278),
            sy(5.556),
        )
        .rc(
            -sx(13.889),
            -sy(5.556),
            -sx(29.166),
            -sy(8.333),
            -sx(45.833),
            -sy(8.333),
        )
        .c(
            x + sx(196.167),
            y + sy(71.722),
            x + sx(143.389),
            y + sy(120.333),
            x + sx(137.833),
            y + sy(182.833),
        )
        .close();
    cloud.finish()
}

/// A structured shape's built-in fill pattern: its SVG pattern id and the
/// multiply `<pattern>-overlay` class.
fn structured_pattern(style: &SourceNodeStyle) -> Option<(&'static str, String)> {
    let pattern = style.fill_pattern.as_deref()?;
    Some((patternassets::id(pattern)?, format!("{pattern}-overlay")))
}

/// d2scenebuild clampBorderRadius for a class/sql_table outer box.
fn structured_radius(style: &SourceNodeStyle, width: f64, height: f64) -> f64 {
    style
        .border_radius
        .as_deref()
        .and_then(|radius| radius.parse::<f64>().ok())
        .map_or(0.0, |radius| {
            radius.min(width / 2.0).min(height / 2.0).max(0.0)
        })
}

fn radius_attributes(radius: f64) -> String {
    if radius == 0.0 {
        String::new()
    } else {
        format!(" rx=\"{0}\" ry=\"{0}\"", number(radius))
    }
}

/// d2scenebuild structuredHeaderPrimitive (special_shapes.go:277–308): a
/// rounded header rounds only its top corners, unless it fills the shape.
#[allow(clippy::too_many_arguments)]
fn structured_header(
    svg: &mut String,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    outer_height: f64,
    radius: f64,
    paint: &str,
    trailing: &str,
) {
    if radius == 0.0 {
        let _ = writeln!(
            svg,
            "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"{paint}/>{trailing}",
            number(x),
            number(y),
            number(w),
            number(h),
        );
        return;
    }
    let r = radius.min(w / 2.0).min(h);
    let n = number;
    let mut d = format!(
        "M {} {} C {} {} {} {} {} {} L {} {} C {} {} {} {} {} {}",
        n(x),
        n(y + r),
        n(x),
        n(y + r),
        n(x),
        n(y),
        n(x + r),
        n(y),
        n(x + w - r),
        n(y),
        n(x + w - r),
        n(y),
        n(x + w),
        n(y),
        n(x + w),
        n(y + r),
    );
    if h >= outer_height {
        d.push_str(&format!(
            " L {} {} C {} {} {} {} {} {} L {} {} C {} {} {} {} {} {} L {} {} Z",
            n(x + w),
            n(y + h - r),
            n(x + w),
            n(y + h - r),
            n(x + w),
            n(y + h),
            n(x + w - r),
            n(y + h),
            n(x + r),
            n(y + h),
            n(x + r),
            n(y + h),
            n(x),
            n(y + h),
            n(x),
            n(y + h - r),
            n(x),
            n(y + r),
        ));
    } else {
        d.push_str(&format!(
            " L {} {} L {} {} Z",
            n(x + w),
            n(y + h),
            n(x),
            n(y + h)
        ));
    }
    let _ = writeln!(svg, "<path d=\"{d}\"{paint}/>{trailing}");
}

fn sql_table_shape(
    svg: &mut String,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    style: &SourceNodeStyle,
    palette: PaletteView<'_>,
) {
    let (filled, open) = attributes(style, palette);
    let _ = writeln!(
        svg,
        "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" {} />",
        number(x),
        number(y),
        number(width),
        number(height),
        filled,
    );
    let row_height = if style.table_columns.is_empty() {
        20.0
    } else {
        ((height - 8.0) / (style.table_columns.len() as f64 + 1.0)).max(16.0)
    };
    let header_y = y + row_height + 4.0;
    let _ = writeln!(
        svg,
        "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" {} />",
        number(x),
        number(header_y),
        number(x + width),
        number(header_y),
        open,
    );
    for (index, column) in style.table_columns.iter().enumerate() {
        let baseline = header_y + row_height * (index as f64 + 1.0);
        if index + 1 < style.table_columns.len() {
            let _ = writeln!(
                svg,
                "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" {} />",
                number(x),
                number(baseline),
                number(x + width),
                number(baseline),
                open,
            );
        }
        let text_fill = themed_color(
            style.font_color.as_deref(),
            &palette.color("N1", palette.base.neutrals.n1),
            palette,
        );
        let text_class = palette
            .semantic_code(style.font_color.as_deref(), "N1")
            .map(|code| format!(" class=\"d2-rs-text-{code}\""))
            .unwrap_or_default();
        let _ = writeln!(
            svg,
            "<text x=\"{}\" y=\"{}\" font-family=\"Source Sans Pro, sans-serif\" font-size=\"12\" fill=\"{}\"{}>{}</text>",
            number(x + 8.0),
            number(baseline - row_height * 0.28),
            escape(&text_fill),
            text_class,
            escape(column),
        );
    }
}

/// Draw the full SQL-table target, including the header, typed columns, and
/// constraint column. The source graph keeps the table's base font in
/// `SourceNodeStyle`; the four-pixel header adjustment is applied here, at the
/// target boundary, just as in D2 v0.9.0.
#[allow(clippy::too_many_arguments)]
fn sql_text_precision(prefix: Option<&str>, role: &str, index: usize, x: f64, y: f64) -> String {
    prefix.map_or_else(String::new, |prefix| format!(
        " id=\"{}-{role}-{index}\" data-d2-rs-text-x-bits=\"{:016x}\" data-d2-rs-text-y-bits=\"{:016x}\" data-d2-rs-text-raw=\"true\" xml:space=\"preserve\"",
        escape(prefix), x.to_bits(), y.to_bits()))
}

#[cfg(test)]
pub(crate) fn draw_sql_table_shape(
    svg: &mut String,
    rect: Rect,
    style: &SourceNodeStyle,
    label: &str,
    font_size: u32,
    label_size: Size,
    palette: PaletteView<'_>,
    native_geometry_id: Option<&str>,
) {
    draw_sql_table_shape_with_sketch(
        svg,
        rect,
        style,
        label,
        font_size,
        label_size,
        palette,
        native_geometry_id,
        None,
    );
}

pub(crate) fn draw_sql_table_shape_with_sketch(
    svg: &mut String,
    rect: Rect,
    style: &SourceNodeStyle,
    label: &str,
    font_size: u32,
    label_size: Size,
    palette: PaletteView<'_>,
    native_geometry_id: Option<&str>,
    mut sketch_streaks: Option<&mut std::collections::BTreeSet<&'static str>>,
) {
    let sketch = sketch_streaks.is_some();
    // d2sketch.Table insets the header and constraint text by 20 in SVG;
    // d2scenebuild's raster path keeps d2target.HeaderPadding (10).
    // d2scenebuild keeps a shape's mono family (the terminal theme sets it on
    // every shape) under sketch; d2sketch's SVG table uses the sketch face.
    let mono_text = !sketch || native_geometry_id.is_some();
    let header_inset = if sketch && native_geometry_id.is_none() {
        20.0
    } else {
        10.0
    };
    // d2themes.ShapeTheme uses the target stroke as the row background
    // and the target fill as both header and outer border.
    let mut body_style = style.clone();
    body_style.fill = Some(style.stroke.clone().unwrap_or_else(|| "N7".into()));
    body_style.stroke = Some(style.fill.clone().unwrap_or_else(|| "N1".into()));
    body_style.stroke_width.get_or_insert_with(|| "2".into());
    // d2scenebuild draws the structured base paint first and interleaves a
    // multiply pattern overlay into the outer box and header only
    // (structuredPatternNode); row rules keep the plain fill paint.
    let pattern = (!sketch).then(|| structured_pattern(style)).flatten();
    if pattern.is_some() {
        body_style.fill_pattern = None;
    }
    let (filled, open) = attributes(&body_style, palette);
    let rows = if style.table_rows.is_empty() {
        style
            .table_columns
            .iter()
            .map(|name| (name.clone(), String::new(), String::new()))
            .collect::<Vec<_>>()
    } else {
        style.table_rows.clone()
    };
    let member_font_size = style
        .font_size
        .unwrap_or_else(|| font_size.saturating_sub(4))
        .max(1);
    let header_font_size = member_font_size.saturating_add(4);
    // Scene font resolution retains the parent's family but clears bold/italic.
    let font_family = if mono_text && style.font.as_deref() == Some("mono") {
        "Source Code Pro, monospace"
    } else {
        "Source Sans Pro, sans-serif"
    };
    let row_height = rect.size.height / (rows.len() as f64 + 1.0);
    if let Some(streaks) = sketch_streaks.as_deref_mut() {
        super::sketch_shapes::draw_structured_rect(
            svg,
            rect,
            &body_style,
            palette,
            false,
            native_geometry_id.is_some(),
            streaks,
        );
    } else {
        let radius = structured_radius(style, rect.size.width, rect.size.height);
        let _ = writeln!(
            svg,
            "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"{} {}/> ",
            number(rect.origin.x),
            number(rect.origin.y),
            number(rect.size.width),
            number(rect.size.height),
            radius_attributes(radius),
            filled,
        );
        if let Some((pattern_id, class)) = &pattern {
            let _ = writeln!(
                svg,
                "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"{} fill=\"url(#{pattern_id})\" class=\"{class}\" stroke=\"none\"/> ",
                number(rect.origin.x),
                number(rect.origin.y),
                number(rect.size.width),
                number(rect.size.height),
                radius_attributes(radius),
            );
        }
    }
    let header_fill = themed_color(
        style.fill.as_deref(),
        &palette.color("N1", palette.base.neutrals.n1),
        palette,
    );
    let header_class = palette
        .semantic_code(style.fill.as_deref(), "N1")
        .map(|code| format!(" class=\"class_header d2-rs-fill-{code}\""))
        .unwrap_or_default();
    if let Some(streaks) = sketch_streaks.as_deref_mut() {
        let mut header = style.clone();
        header.fill = Some(style.fill.clone().unwrap_or_else(|| "N1".into()));
        header.stroke = Some("none".into());
        header.stroke_width = Some("1".into());
        header.stroke_dash = None;
        header.shadow = None;
        super::sketch_shapes::draw_structured_rect(
            svg,
            Rect {
                size: Size {
                    height: row_height,
                    ..rect.size
                },
                ..rect
            },
            &header,
            palette,
            false,
            native_geometry_id.is_some(),
            streaks,
        );
    } else {
        structured_header(
            svg,
            rect.origin.x,
            rect.origin.y,
            rect.size.width,
            row_height,
            rect.size.height,
            structured_radius(style, rect.size.width, rect.size.height),
            &format!(" fill=\"{}\"{}", escape(&header_fill), header_class),
            " ",
        );
        if let Some((pattern_id, class)) = &pattern {
            structured_header(
                svg,
                rect.origin.x,
                rect.origin.y,
                rect.size.width,
                row_height,
                rect.size.height,
                structured_radius(style, rect.size.width, rect.size.height),
                &format!(" fill=\"url(#{pattern_id})\" class=\"{class}\""),
                " ",
            );
        }
    }
    if !label.is_empty() {
        // Shape.GetFontColor uses the stroke for semantic theme colors;
        // explicit literal font colors take precedence for structured ink.
        let header_color = style
            .font_color
            .as_deref()
            .filter(|value| !super::THEME_CODES.contains(value))
            .or(style.stroke.as_deref());
        let header_text = themed_color(
            header_color,
            &palette.color("N7", palette.base.neutrals.n7),
            palette,
        );
        let header_text_class = palette
            .semantic_code(header_color, "N7")
            .map(|code| format!(" class=\"d2-rs-text-{code}\""))
            .unwrap_or_default();
        let header_text_class = if mono_text && style.font.as_deref() == Some("mono") {
            add_svg_class(&header_text_class, "text-mono")
        } else {
            header_text_class
        };
        let _ = writeln!(
            svg,
            "<text{} x=\"{}\" y=\"{}\" fill=\"{}\"{} font-family=\"{}\" font-size=\"{}\">{}</text>",
            sql_text_precision(
                native_geometry_id,
                "header",
                0,
                rect.origin.x + header_inset,
                rect.origin.y + row_height / 2.0 - label_size.height / 2.0
                    + label_size.height * 3.0 / 4.0
            ),
            number(rect.origin.x + header_inset),
            number(
                rect.origin.y + row_height / 2.0 - label_size.height / 2.0
                    + label_size.height * 3.0 / 4.0
            ),
            escape(&header_text),
            header_text_class,
            font_family,
            header_font_size,
            super::escape_xml_text(label),
        );
    }
    let name_width = f64::from(style.table_max_name_width);
    let name_x = rect.origin.x + 10.0;
    let type_x = name_x + name_width + 20.0;
    let name_fill = palette.color("B2", palette.base.b2);
    let type_fill = palette.color("N2", palette.base.neutrals.n2);
    let constraint_fill = palette.color("AA2", palette.base.aa2);
    let name_class = palette
        .semantic_code(Some("B2"), "B2")
        .map(|code| format!(" class=\"d2-rs-text-{code}\""))
        .unwrap_or_default();
    let name_class = if mono_text && style.font.as_deref() == Some("mono") {
        add_svg_class(&name_class, "text-mono")
    } else {
        name_class
    };
    let type_class = palette
        .semantic_code(Some("N2"), "N2")
        .map(|code| format!(" class=\"d2-rs-text-{code}\""))
        .unwrap_or_default();
    let type_class = if mono_text && style.font.as_deref() == Some("mono") {
        add_svg_class(&type_class, "text-mono")
    } else {
        type_class
    };
    let constraint_class = palette
        .semantic_code(Some("AA2"), "AA2")
        .map(|code| format!(" class=\"d2-rs-text-{code}\""))
        .unwrap_or_default();
    let constraint_class = if mono_text && style.font.as_deref() == Some("mono") {
        add_svg_class(&constraint_class, "text-mono")
    } else {
        constraint_class
    };
    // The original row boxes advance cumulatively. GetPointOnBox centers
    // the text box before adding the three-quarter-font baseline offset.
    let mut row_top = rect.origin.y + row_height;
    for (index, (name, type_name, constraint)) in rows.iter().enumerate() {
        let baseline = row_top + row_height / 2.0 - f64::from(member_font_size) / 2.0
            + f64::from(member_font_size) * 3.0 / 4.0;
        let _ = writeln!(
            svg,
            "<text{} x=\"{}\" y=\"{}\" fill=\"{}\"{} font-family=\"{}\" font-size=\"{}\">{}</text>",
            sql_text_precision(native_geometry_id, "name", index, name_x, baseline),
            number(name_x),
            number(baseline),
            escape(&name_fill),
            name_class,
            font_family,
            member_font_size,
            super::escape_xml_text(name),
        );
        let _ = writeln!(
            svg,
            "<text{} x=\"{}\" y=\"{}\" fill=\"{}\"{} font-family=\"{}\" font-size=\"{}\">{}</text>",
            sql_text_precision(native_geometry_id, "type", index, type_x, baseline),
            number(type_x),
            number(baseline),
            escape(&type_fill),
            type_class,
            font_family,
            member_font_size,
            super::escape_xml_text(type_name),
        );
        let _ = writeln!(
            svg,
            "<text{} x=\"{}\" y=\"{}\" text-anchor=\"end\" fill=\"{}\"{} font-family=\"{}\" font-size=\"{}\">{}</text>",
            sql_text_precision(
                native_geometry_id,
                "constraint",
                index,
                rect.origin.x + rect.size.width - header_inset,
                baseline
            ),
            number(rect.origin.x + rect.size.width - header_inset),
            number(baseline),
            escape(&constraint_fill),
            if sketch {
                format!("{constraint_class} letter-spacing=\"2\"")
            } else {
                constraint_class.clone()
            },
            font_family,
            member_font_size,
            super::escape_xml_text(constraint),
        );
        row_top += row_height;
        if sketch_streaks.is_some() {
            // buildSketchStructuredShape roughens the already-inset line
            // from buildSQLTableContents, including its last-row radius.
            let inset = if native_geometry_id.is_some() && index + 1 == rows.len() {
                structured_radius(style, rect.size.width, rect.size.height)
            } else {
                0.0
            };
            super::sketch_shapes::draw_structured_line(
                svg,
                Point {
                    x: rect.origin.x + inset,
                    y: row_top,
                },
                Point {
                    x: rect.origin.x + rect.size.width - inset,
                    y: row_top,
                },
                style,
                &header_fill,
            );
        } else {
            // buildSQLTableContents insets the last row's rule by the
            // clamped radius so it meets the rounded bottom corners.
            let inset = if index + 1 == rows.len() {
                structured_radius(style, rect.size.width, rect.size.height)
            } else {
                0.0
            };
            let _ = writeln!(
                svg,
                "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" {}/>",
                number(rect.origin.x + inset),
                number(row_top),
                number(rect.origin.x + rect.size.width - inset),
                number(row_top),
                open,
            );
        }
    }
    if let Some(streaks) = sketch_streaks {
        super::sketch_shapes::draw_streak_rect(svg, rect, style, palette, streaks);
    }
}

/// lib/shape.GetPathCommands supplies the same chopped M/L/C operands used
/// for ordinary SVG. No additional coordinate normalization is introduced.
pub(crate) fn typed_sketch_paths(shape: ShapeKind, rect: Rect) -> Option<Vec<String>> {
    let (x, y, w, h) = (
        rect.origin.x,
        rect.origin.y,
        rect.size.width,
        rect.size.height,
    );
    Some(match shape {
        ShapeKind::Diamond => vec![diamond_path(x, y, w, h)],
        ShapeKind::Hexagon => {
            let points = hexagon_points(rect);
            let mut data = String::new();
            for (i, p) in points.iter().enumerate() {
                let _ = write!(
                    data,
                    "{}{} {} ",
                    if i == 0 { 'M' } else { 'L' },
                    path_number(p.x),
                    path_number(p.y)
                );
            }
            data.push('Z');
            vec![data]
        }
        ShapeKind::Parallelogram => vec![parallelogram_path(x, y, w, h)],
        ShapeKind::Document => vec![document_path(x, y, w, h)],
        ShapeKind::Cylinder => {
            let (a, b) = cylinder_paths(x, y, w, h);
            vec![a, b]
        }
        ShapeKind::Queue => {
            let (a, b) = queue_paths(x, y, w, h);
            vec![a, b]
        }
        ShapeKind::Page => {
            let (a, b) = page_paths(x, y, w, h);
            vec![a, b]
        }
        ShapeKind::Package => vec![package_path(x, y, w, h)],
        ShapeKind::Step => vec![step_path(x, y, w, h)],
        ShapeKind::Callout => vec![callout_path(x, y, w, h)],
        ShapeKind::StoredData => vec![stored_data_path(x, y, w, h)],
        ShapeKind::Cloud => vec![cloud_path(x, y, w, h)],
        ShapeKind::Person => vec![person_path(x, y, w, h)],
        ShapeKind::C4Person => {
            let (a, b) = c4_person_paths(x, y, w, h);
            vec![a, b]
        }
        _ => return None,
    })
}

/// Draws a D2 node using the source-shaped path family when one is available.
pub(crate) fn draw_shape(
    svg: &mut String,
    shape: ShapeKind,
    rect: Rect,
    style: &SourceNodeStyle,
    palette: PaletteView<'_>,
) {
    draw_shape_with_id(svg, shape, rect, style, palette, "");
}

pub(crate) fn draw_shape_with_id(
    svg: &mut String,
    shape: ShapeKind,
    rect: Rect,
    style: &SourceNodeStyle,
    palette: PaletteView<'_>,
    shape_id: &str,
) {
    let shadow = style_true(style.shadow.as_ref())
        && !matches!(
            shape,
            ShapeKind::Text | ShapeKind::Code | ShapeKind::Class | ShapeKind::SqlTable
        );
    let multiple = style_true(style.multiple.as_ref());
    let three_dee = style_true(style.three_dee.as_ref());
    let double_border = style_true(style.double_border.as_ref());
    if shadow {
        svg.push_str("<g filter=\"url(#d2-rs-shadow-filter)\">");
    }
    if three_dee
        && matches!(
            shape,
            ShapeKind::Rectangle | ShapeKind::Square | ShapeKind::RealSquare | ShapeKind::Hexagon
        )
    {
        draw_three_dee_shape(svg, shape, rect, style, palette, shape_id, false);
        if shadow {
            svg.push_str("</g>\n");
        }
        return;
    }
    if multiple {
        let mut overlay = style.clone();
        overlay.multiple = None;
        overlay.three_dee = None;
        overlay.shadow = None;
        overlay.double_border = None;
        let offset = if three_dee { 15.0 } else { 10.0 };
        draw_shape_with_id(
            svg,
            shape,
            Rect {
                origin: Point {
                    x: rect.origin.x + offset,
                    y: rect.origin.y - offset,
                },
                ..rect
            },
            &overlay,
            palette,
            shape_id,
        );
    }
    let x = rect.origin.x;
    let y = rect.origin.y;
    let width = rect.size.width;
    let height = rect.size.height;
    let (filled, _) = attributes(style, palette);
    match shape {
        // Text nodes are target-only labels. D2 does not draw a container
        // outline for the `text` shape.
        ShapeKind::Text | ShapeKind::Class => {}
        ShapeKind::Oval | ShapeKind::Circle => {
            let filled = add_svg_class(&filled, "shape");
            let _ = writeln!(
                svg,
                "<ellipse cx=\"{}\" cy=\"{}\" rx=\"{}\" ry=\"{}\" {}/>",
                number(x + width / 2.0),
                number(y + height / 2.0),
                number(width / 2.0),
                number(height / 2.0),
                filled
            );
        }
        ShapeKind::Diamond => {
            let data = diamond_path(x, y, width, height);
            write_path(svg, &data, &add_svg_class(&filled, "shape"));
        }
        ShapeKind::Hexagon => {
            let points = hexagon_points(rect);
            let _ = writeln!(svg, "<polygon points=\"{}\" {}/>", polygon(&points), filled);
        }
        ShapeKind::Parallelogram => {
            write_path(svg, &parallelogram_path(x, y, width, height), &filled)
        }
        ShapeKind::Document => write_path(svg, &document_path(x, y, width, height), &filled),
        ShapeKind::Cylinder => {
            let (outer, inner) = cylinder_paths(x, y, width, height);
            write_path(svg, &outer, &filled);
            // D2 emits every path in the shape with the target fill. SVG
            // implicitly closes this open detail for fill while preserving
            // its open stroke, matching the v0.9.0 target behavior.
            write_path(svg, &inner, &filled);
        }
        ShapeKind::Queue => {
            let (outer, inner) = queue_paths(x, y, width, height);
            write_path(svg, &outer, &filled);
            write_path(svg, &inner, &filled);
        }
        ShapeKind::Page => {
            let (outer, inner) = page_paths(x, y, width, height);
            write_path(svg, &outer, &filled);
            write_path(svg, &inner, &filled);
        }
        ShapeKind::Package => write_path(svg, &package_path(x, y, width, height), &filled),
        ShapeKind::Step => write_path(svg, &step_path(x, y, width, height), &filled),
        ShapeKind::Callout => write_path(svg, &callout_path(x, y, width, height), &filled),
        ShapeKind::StoredData => write_path(svg, &stored_data_path(x, y, width, height), &filled),
        ShapeKind::Cloud => write_path(svg, &cloud_path(x, y, width, height), &filled),
        ShapeKind::Person => write_path(svg, &person_path(x, y, width, height), &filled),
        ShapeKind::C4Person => {
            let (body, head) = c4_person_paths(x, y, width, height);
            write_path(svg, &body, &filled);
            write_path(svg, &head, &filled);
        }
        ShapeKind::Image => {
            // D2 treats an image shape's icon as the image itself. Unlike an
            // ordinary node icon it fills the fitted 128px image box and does
            // not reserve a separate label/icon decoration slot.
            if let Some(icon) = style.icon.as_deref().filter(|value| !value.is_empty()) {
                let radius = style
                    .icon_border_radius
                    .as_deref()
                    .and_then(|v| v.parse::<i64>().ok())
                    .unwrap_or(0) as f64;
                let clip =
                    super::image_radius_clip(svg, rect, radius, true, &format!("{shape_id}-image"));
                let _ = writeln!(
                    svg,
                    "<image x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" href=\"{}\" xlink:href=\"{}\" preserveAspectRatio=\"xMidYMid meet\" {}{}/>",
                    number(x),
                    number(y),
                    number(width),
                    number(height),
                    escape(icon),
                    escape(icon),
                    filled,
                    clip,
                );
            }
        }
        ShapeKind::SqlTable => sql_table_shape(svg, x, y, width, height, style, palette),
        _ => {
            if let Some(radius) = style.border_radius.as_deref() {
                // d2scenebuild clampBorderRadius (build.go:1121): one radius
                // limited by both half-dimensions, so a large value draws a
                // stadium. SVG would clamp rx and ry per axis into an ellipse.
                let radius = match radius.parse::<f64>() {
                    Ok(radius) => number(radius.min(width / 2.0).min(height / 2.0).max(0.0)),
                    Err(_) => escape(radius).to_string(),
                };
                let _ = writeln!(
                    svg,
                    "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"{radius}\" ry=\"{radius}\" {}/>",
                    number(x),
                    number(y),
                    number(width),
                    number(height),
                    filled
                );
            } else {
                let _ = writeln!(
                    svg,
                    "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" {}/>",
                    number(x),
                    number(y),
                    number(width),
                    number(height),
                    filled
                );
            }
        }
    }
    if double_border {
        let mut inner = style.clone();
        inner.multiple = None;
        inner.three_dee = None;
        inner.shadow = None;
        inner.double_border = None;
        inner.fill = Some("none".to_owned());
        inner.fill_pattern = None;
        let inset = 5.0;
        if width > 2.0 * inset && height > 2.0 * inset {
            draw_shape(
                svg,
                shape,
                Rect {
                    origin: Point {
                        x: x + inset,
                        y: y + inset,
                    },
                    size: d2_rs_core::Size {
                        width: width - 2.0 * inset,
                        height: height - 2.0 * inset,
                    },
                },
                &inner,
                palette,
            );
        }
    }
    if shadow {
        svg.push_str("</g>\n");
    }
}

/// Source buildDoubleBorderShape/effectPatternNode: paint the outer pattern
/// before the inner border; the inner primitive never receives the pattern.
pub(crate) fn draw_native_double_border_pattern(
    svg: &mut String,
    shape: ShapeKind,
    rect: Rect,
    base: &SourceNodeStyle,
    overlay: &SourceNodeStyle,
    palette: PaletteView<'_>,
    shape_id: &str,
) -> bool {
    if !style_true(base.double_border.as_ref())
        || style_true(base.multiple.as_ref())
        || !matches!(
            shape,
            ShapeKind::Rectangle
                | ShapeKind::Square
                | ShapeKind::RealSquare
                | ShapeKind::Oval
                | ShapeKind::Circle
        )
    {
        return false;
    }
    let shadow = style_true(base.shadow.as_ref());
    if shadow {
        svg.push_str("<g filter=\"url(#d2-rs-shadow-filter)\">");
    }
    let mut outer = base.clone();
    outer.double_border = None;
    outer.shadow = None;
    let mut pattern = overlay.clone();
    pattern.double_border = None;
    draw_shape_with_id(svg, shape, rect, &outer, palette, shape_id);
    draw_shape_with_id(svg, shape, rect, &pattern, palette, "");
    let mut inner = outer;
    if !matches!(shape, ShapeKind::Oval | ShapeKind::Circle) {
        inner.fill = Some("none".into());
    }
    let inset = 5.0;
    if rect.size.width > 2.0 * inset && rect.size.height > 2.0 * inset {
        draw_shape(
            svg,
            shape,
            Rect {
                origin: Point {
                    x: rect.origin.x + inset,
                    y: rect.origin.y + inset,
                },
                size: d2_rs_core::Size {
                    width: rect.size.width - 2.0 * inset,
                    height: rect.size.height - 2.0 * inset,
                },
            },
            &inner,
            palette,
        );
    }
    if shadow {
        svg.push_str("</g>\n");
    }
    true
}

/// Emits pattern paint in the same primitive order as D2's `interleavePattern`.
/// This currently handles ordinary shapes composed from multiple paths; effect
/// shapes continue through the normal shape renderer until their retained
/// primitive ordering is represented explicitly.
pub(crate) fn draw_interleaved_multi_path_pattern(
    svg: &mut String,
    shape: ShapeKind,
    rect: Rect,
    base_style: &SourceNodeStyle,
    overlay_style: &SourceNodeStyle,
    palette: PaletteView<'_>,
) -> bool {
    if style_true(base_style.shadow.as_ref())
        || style_true(base_style.three_dee.as_ref())
        || style_true(base_style.double_border.as_ref())
    {
        return false;
    }
    let multiple = style_true(base_style.multiple.as_ref());
    if multiple
        && !matches!(
            shape,
            ShapeKind::Cylinder | ShapeKind::Queue | ShapeKind::Page | ShapeKind::C4Person
        )
    {
        return false;
    }
    if multiple {
        // The unpatterned back copy first; effectPatternNode interleaves the
        // pattern only into the front copy's paths below.
        let mut back = base_style.clone();
        back.multiple = None;
        draw_shape(
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
    }
    let mut base_style = base_style.clone();
    base_style.multiple = None;
    let base_style = &base_style;

    let x = rect.origin.x;
    let y = rect.origin.y;
    let width = rect.size.width;
    let height = rect.size.height;
    let paths = match shape {
        ShapeKind::Cylinder => {
            let (outer, inner) = cylinder_paths(x, y, width, height);
            vec![outer, inner]
        }
        ShapeKind::Queue => {
            let (outer, inner) = queue_paths(x, y, width, height);
            vec![outer, inner]
        }
        ShapeKind::Page => {
            let (outer, inner) = page_paths(x, y, width, height);
            vec![outer, inner]
        }
        ShapeKind::C4Person => {
            let (body, head) = c4_person_paths(x, y, width, height);
            vec![body, head]
        }
        _ => return false,
    };
    let (base_attributes, _) = attributes(base_style, palette);
    let (overlay_attributes, _) = attributes(overlay_style, palette);
    for path in paths {
        write_path(svg, &path, &base_attributes);
        write_path(svg, &path, &overlay_attributes);
    }
    true
}

// Private class TextRuns use the ordinary Source paint-reference contract.
// Prefix each owner because capture_object_paint resets the SVG buffer.
fn structured_text_precision(index: usize, x: f64, y: f64, prefix: Option<&str>) -> String {
    let Some(prefix) = prefix else {
        return String::new();
    };
    format!(
        " id=\"{prefix}-text-{index}\" data-d2-rs-text-x-bits=\"{:016x}\" data-d2-rs-text-y-bits=\"{:016x}\"",
        x.to_bits(),
        y.to_bits()
    )
}

/// Draws the class/UML target and its retained source-order member rows.
#[cfg(test)]
pub(crate) fn draw_class_shape(
    svg: &mut String,
    rect: Rect,
    style: &SourceNodeStyle,
    label: &str,
    font_size: u32,
    label_size: Size,
    palette: PaletteView<'_>,
    native_geometry_id: Option<&str>,
) {
    draw_class_shape_with_sketch(
        svg,
        rect,
        style,
        label,
        font_size,
        label_size,
        palette,
        native_geometry_id,
        None,
    );
}

pub(crate) fn draw_class_shape_with_sketch(
    svg: &mut String,
    rect: Rect,
    style: &SourceNodeStyle,
    label: &str,
    font_size: u32,
    label_size: Size,
    palette: PaletteView<'_>,
    native_geometry_id: Option<&str>,
    mut sketch_streaks: Option<&mut std::collections::BTreeSet<&'static str>>,
) {
    // d2themes.ShapeTheme uses the target stroke as the row background
    // and the target fill as both header and outer border.
    let mut body_style = style.clone();
    body_style.fill = Some(style.stroke.clone().unwrap_or_else(|| "N7".into()));
    body_style.stroke = Some(style.fill.clone().unwrap_or_else(|| "N1".into()));
    body_style.stroke_width.get_or_insert_with(|| "2".into());
    // d2scenebuild structuredPatternNode: base paint, then a multiply pattern
    // overlay on the outer box and header only.
    let pattern = sketch_streaks
        .is_none()
        .then(|| structured_pattern(style))
        .flatten();
    if pattern.is_some() {
        body_style.fill_pattern = None;
    }
    let (filled, _) = attributes(&body_style, palette);
    let width = rect.size.width;
    let height = rect.size.height;
    // With no explicit style the layout graph stores the header-adjusted
    // default (24px); an explicit style stores its base value (for example
    // 30px). Keep both cases aligned with d2target's export subtraction.
    let member_font_size = style
        .font_size
        .unwrap_or_else(|| font_size.saturating_sub(4))
        .max(1);
    let header_font_size = member_font_size.saturating_add(4);
    let row_count = 2 + style.class_fields.len() + style.class_methods.len();
    let row_height = if row_count == 0 {
        height
    } else {
        height / row_count as f64
    };
    let header_height = if sketch_streaks.is_some() {
        2.0 * row_height
    } else {
        (2.0 * row_height).max(label_size.height + 10.0)
    };
    if let Some(streaks) = sketch_streaks.as_deref_mut() {
        super::sketch_shapes::draw_structured_rect(
            svg,
            rect,
            &body_style,
            palette,
            false,
            native_geometry_id.is_some(),
            streaks,
        );
    } else {
        let radius = structured_radius(style, width, height);
        let _ = writeln!(
            svg,
            "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"{} {}/>",
            number(rect.origin.x),
            number(rect.origin.y),
            number(width),
            number(height),
            radius_attributes(radius),
            filled,
        );
        if let Some((pattern_id, class)) = &pattern {
            let _ = writeln!(
                svg,
                "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"{} fill=\"url(#{pattern_id})\" class=\"{class}\" stroke=\"none\"/>",
                number(rect.origin.x),
                number(rect.origin.y),
                number(width),
                number(height),
                radius_attributes(radius),
            );
        }
    }
    let header_fill = themed_color(
        style.fill.as_deref(),
        &palette.color("N1", palette.base.neutrals.n1),
        palette,
    );
    let header_class = palette
        .semantic_code(style.fill.as_deref(), "N1")
        .map(|code| format!(" class=\"class_header d2-rs-fill-{code}\""))
        .unwrap_or_default();
    if let Some(streaks) = sketch_streaks.as_deref_mut() {
        let mut header_style = style.clone();
        header_style.fill = Some(style.fill.clone().unwrap_or_else(|| "N1".into()));
        header_style.stroke = Some("none".into());
        header_style.stroke_width = Some("1".into());
        header_style.stroke_dash = None;
        header_style.shadow = None;
        super::sketch_shapes::draw_structured_rect(
            svg,
            Rect {
                size: Size {
                    height: header_height,
                    ..rect.size
                },
                ..rect
            },
            &header_style,
            palette,
            true,
            native_geometry_id.is_some(),
            streaks,
        );
    } else {
        structured_header(
            svg,
            rect.origin.x,
            rect.origin.y,
            width,
            header_height,
            height,
            structured_radius(style, width, height),
            &format!(" fill=\"{}\"{}", escape(&header_fill), header_class),
            "",
        );
        if let Some((pattern_id, class)) = &pattern {
            structured_header(
                svg,
                rect.origin.x,
                rect.origin.y,
                width,
                header_height,
                height,
                structured_radius(style, width, height),
                &format!(" fill=\"url(#{pattern_id})\" class=\"{class}\""),
                "",
            );
        }
    }
    // Original classHeader uses Shape.GetFontColor: a concrete text color
    // wins; palette-valued text colors use the target stroke instead.
    let header_color = style
        .font_color
        .as_deref()
        .filter(|color| d2_rs_core::theme::resolve_color(palette.base, color).is_none())
        .or(style.stroke.as_deref());
    let header_text = themed_color(
        header_color,
        &palette.color("N7", palette.base.neutrals.n7),
        palette,
    );
    let header_x = rect.origin.x + width / 2.0;
    // The native scene uses centeredTextRuns' base-font-size baseline even
    // in sketch mode; the interactive SVG sketch uses its own baseline.
    let header_y = rect.origin.y
        + (header_height - label_size.height) / 2.0
        + if sketch_streaks.is_some() && native_geometry_id.is_none() {
            label_size.height * 3.0 / 4.0
        } else {
            f64::from(member_font_size)
        };
    if !label.is_empty() {
        if native_geometry_id.is_some() {
            let lines = label.split('\n').collect::<Vec<_>>();
            let advance = label_size.height / lines.len() as f64;
            for (index, line) in lines.iter().enumerate() {
                let baseline = header_y + index as f64 * advance;
                let precision =
                    structured_text_precision(svg.len(), header_x, baseline, native_geometry_id);
                let _ = writeln!(
                    svg,
                    "<text class=\"text-mono\"{} xml:space=\"preserve\" x=\"{}\" y=\"{}\" fill=\"{}\" font-family=\"Source Code Pro, monospace\" font-size=\"{}\" text-anchor=\"middle\">{}</text>",
                    precision,
                    number(header_x),
                    number(baseline),
                    escape(&header_text),
                    header_font_size,
                    escape_xml_text(line)
                );
            }
        } else {
            let _ = writeln!(
                svg,
                "<text class=\"text-mono\" x=\"{}\" y=\"{}\" fill=\"{}\" font-family=\"Source Code Pro, monospace\" font-size=\"{}\" text-anchor=\"middle\">{}</text>",
                number(header_x),
                number(header_y),
                escape(&header_text),
                header_font_size,
                render_text_content(label, header_x, label_size.height)
            );
        }
    }
    // special_shapes.go buildClassContents paints row names with the target
    // Fill (the header color), including authored fills and C4 theme defaults.
    let member_fill = themed_color(
        style.fill.as_deref(),
        &palette.color("N1", palette.base.neutrals.n1),
        palette,
    );
    let visibility_fill = palette.color("B2", palette.base.b2);
    let type_fill = palette.color("AA2", palette.base.aa2);
    // Source advances the retained row box after each member. Preserve its
    // operation order for private raster TextRuns; public SVG keeps its format.
    let mut row_y = rect.origin.y + header_height;
    for (index, (name, value)) in style.class_fields.iter().enumerate() {
        let (visibility, name) = name
            .chars()
            .next()
            .filter(|character| matches!(character, '+' | '-' | '#'))
            .map_or(("+", name.as_str()), |character| {
                (&name[..character.len_utf8()], &name[character.len_utf8()..])
            });
        let public_baseline = rect.origin.y
            + header_height
            + row_height * index as f64
            + row_height / 2.0
            + f64::from(member_font_size) * 0.25;
        let baseline = if native_geometry_id.is_some() {
            row_y
                + (row_height - f64::from(member_font_size)) / 2.0
                + f64::from(member_font_size) * 3.0 / 4.0
        } else {
            public_baseline
        };
        let _ = writeln!(
            svg,
            "<text class=\"text-mono\"{} x=\"{}\" y=\"{}\" fill=\"{}\" font-family=\"Source Code Pro, monospace\" font-size=\"{}\">{}</text>",
            structured_text_precision(
                svg.len(),
                rect.origin.x + 10.0,
                baseline,
                native_geometry_id
            ),
            number(rect.origin.x + 10.0),
            number(baseline),
            escape(&visibility_fill),
            member_font_size,
            escape(visibility),
        );
        let _ = writeln!(
            svg,
            "<text class=\"text-mono\"{}{} x=\"{}\" y=\"{}\" fill=\"{}\" font-family=\"Source Code Pro, monospace\" font-size=\"{}\">{}</text>",
            if style
                .class_field_underlines
                .get(index)
                .copied()
                .unwrap_or(false)
            {
                " text-decoration=\"underline\""
            } else {
                ""
            },
            structured_text_precision(
                svg.len(),
                rect.origin.x + 30.0,
                baseline,
                native_geometry_id
            ),
            number(rect.origin.x + 30.0),
            number(baseline),
            escape(&member_fill),
            member_font_size,
            escape_xml_text(name),
        );
        if !value.is_empty() {
            let _ = writeln!(
                svg,
                "<text class=\"text-mono\"{} x=\"{}\" y=\"{}\" fill=\"{}\" font-family=\"Source Code Pro, monospace\" font-size=\"{}\" text-anchor=\"end\">{}</text>",
                structured_text_precision(
                    svg.len(),
                    rect.origin.x + width - 20.0,
                    baseline,
                    native_geometry_id
                ),
                number(rect.origin.x + width - 20.0),
                number(baseline),
                escape(&type_fill),
                member_font_size,
                escape_xml_text(value),
            );
        }
        row_y += row_height;
    }
    // d2svg.drawClass always paints this one-pixel rule, including at the
    // bottom edge when there are no methods.
    let separator_y = rect.origin.y + header_height + row_height * style.class_fields.len() as f64;
    if sketch_streaks.is_some() {
        // buildSketchStructuredShape keeps buildClassContents' endpoints
        // when it replaces the bottom separator with a rough line.
        let inset = if native_geometry_id.is_some() && style.class_methods.is_empty() {
            structured_radius(style, rect.size.width, rect.size.height)
        } else {
            0.0
        };
        super::sketch_shapes::draw_structured_line(
            svg,
            Point {
                x: rect.origin.x + inset,
                y: separator_y,
            },
            Point {
                x: rect.right() - inset,
                y: separator_y,
            },
            style,
            &header_fill,
        );
    } else {
        // buildClassContents insets the rule by the radius when it is the
        // bottom edge (no methods).
        let inset = if style.class_methods.is_empty() {
            structured_radius(style, rect.size.width, rect.size.height)
        } else {
            0.0
        };
        let _ = writeln!(
            svg,
            "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"{}\" stroke-width=\"1\"/>",
            number(rect.origin.x + inset),
            number(separator_y),
            number(rect.right() - inset),
            number(separator_y),
            escape(&header_fill),
        );
    }
    let method_offset = style.class_fields.len();
    for (index, (name, value)) in style.class_methods.iter().enumerate() {
        let (visibility, name) = name
            .chars()
            .next()
            .filter(|character| matches!(character, '+' | '-' | '#'))
            .map_or(("+", name.as_str()), |character| {
                (&name[..character.len_utf8()], &name[character.len_utf8()..])
            });
        let public_baseline = rect.origin.y
            + header_height
            + row_height * (method_offset + index) as f64
            + row_height / 2.0
            + f64::from(member_font_size) * 0.25;
        let baseline = if native_geometry_id.is_some() {
            row_y
                + (row_height - f64::from(member_font_size)) / 2.0
                + f64::from(member_font_size) * 3.0 / 4.0
        } else {
            public_baseline
        };
        let _ = writeln!(
            svg,
            "<text class=\"text-mono\"{} x=\"{}\" y=\"{}\" fill=\"{}\" font-family=\"Source Code Pro, monospace\" font-size=\"{}\">{}</text>",
            structured_text_precision(
                svg.len(),
                rect.origin.x + 10.0,
                baseline,
                native_geometry_id
            ),
            number(rect.origin.x + 10.0),
            number(baseline),
            escape(&visibility_fill),
            member_font_size,
            escape(visibility),
        );
        let _ = writeln!(
            svg,
            "<text class=\"text-mono\"{}{} x=\"{}\" y=\"{}\" fill=\"{}\" font-family=\"Source Code Pro, monospace\" font-size=\"{}\">{}</text>",
            if style
                .class_method_underlines
                .get(index)
                .copied()
                .unwrap_or(false)
            {
                " text-decoration=\"underline\""
            } else {
                ""
            },
            structured_text_precision(
                svg.len(),
                rect.origin.x + 30.0,
                baseline,
                native_geometry_id
            ),
            number(rect.origin.x + 30.0),
            number(baseline),
            escape(&member_fill),
            member_font_size,
            escape_xml_text(name),
        );
        if !value.is_empty() {
            let _ = writeln!(
                svg,
                "<text class=\"text-mono\"{} x=\"{}\" y=\"{}\" fill=\"{}\" font-family=\"Source Code Pro, monospace\" font-size=\"{}\" text-anchor=\"end\">{}</text>",
                structured_text_precision(
                    svg.len(),
                    rect.origin.x + width - 20.0,
                    baseline,
                    native_geometry_id
                ),
                number(rect.origin.x + width - 20.0),
                number(baseline),
                escape(&type_fill),
                member_font_size,
                escape_xml_text(value),
            );
        }
        row_y += row_height;
    }
}

#[cfg(test)]
mod tests {
    use super::{
        PaletteView, darken_color, draw_interleaved_multi_path_pattern, draw_shape,
        draw_shape_with_id, draw_sql_table_shape, gradient_definition, gradient_id, hexagon_points,
    };
    use d2_rs_core::theme::palette;
    use d2_rs_core::{Point, Rect, ShapeKind, Size};
    use d2_rs_language::compiler::SourceNodeStyle;
    use std::collections::BTreeMap;

    fn render(shape: ShapeKind) -> String {
        let mut svg = String::new();
        let base = palette(0).expect("default palette");
        let overrides = BTreeMap::new();
        draw_shape(
            &mut svg,
            shape,
            Rect {
                origin: Point { x: 10.0, y: 20.0 },
                size: Size {
                    width: 120.0,
                    height: 80.0,
                },
            },
            &SourceNodeStyle::default(),
            PaletteView::new(base, &overrides),
        );
        svg
    }

    #[test]
    fn custom_shapes_use_paths_instead_of_rect_fallbacks() {
        for shape in [
            ShapeKind::Parallelogram,
            ShapeKind::Document,
            ShapeKind::Cylinder,
            ShapeKind::Queue,
            ShapeKind::Page,
            ShapeKind::Package,
            ShapeKind::Step,
            ShapeKind::Callout,
            ShapeKind::StoredData,
            ShapeKind::Cloud,
            ShapeKind::Person,
            ShapeKind::C4Person,
        ] {
            let output = render(shape);
            assert!(output.contains("<path"), "{shape:?} did not render a path");
            assert!(!output.contains("<rect"), "{shape:?} used the fallback");
        }
    }

    #[test]
    fn multi_path_fill_patterns_interleave_each_base_and_overlay_path() {
        for shape in [
            ShapeKind::Cylinder,
            ShapeKind::Queue,
            ShapeKind::Page,
            ShapeKind::C4Person,
        ] {
            let style = SourceNodeStyle {
                fill: Some("#EDF0FD".into()),
                fill_pattern: Some("paper".into()),
                ..SourceNodeStyle::default()
            };
            let mut base = style.clone();
            base.fill_pattern = None;
            let mut overlay = style.clone();
            overlay.fill = None;
            overlay.stroke = Some("none".into());
            overlay.shadow = None;
            let mut svg = String::new();
            let base_palette = palette(0).expect("default palette");
            let overrides = BTreeMap::new();
            assert!(draw_interleaved_multi_path_pattern(
                &mut svg,
                shape,
                Rect {
                    origin: Point { x: 10.0, y: 20.0 },
                    size: Size {
                        width: 120.0,
                        height: 80.0,
                    },
                },
                &base,
                &overlay,
                PaletteView::new(base_palette, &overrides),
            ));
            let paths = svg.lines().collect::<Vec<_>>();
            assert_eq!(paths.len(), 4, "{shape:?}");
            assert!(
                !paths[0].contains("paper-overlay"),
                "{shape:?}: {}",
                paths[0]
            );
            assert!(
                paths[1].contains("paper-overlay"),
                "{shape:?}: {}",
                paths[1]
            );
            assert!(
                !paths[2].contains("paper-overlay"),
                "{shape:?}: {}",
                paths[2]
            );
            assert!(
                paths[3].contains("paper-overlay"),
                "{shape:?}: {}",
                paths[3]
            );
        }
    }

    #[test]
    fn straight_shape_boundaries_follow_the_upstream_constants() {
        assert_eq!(
            super::parallelogram_path(10.0, 20.0, 120.0, 80.0),
            "M 36 20 L 130 20 L 104 100 L 10 100 L 10 100 Z"
        );
        assert_eq!(
            super::package_path(10.0, 20.0, 120.0, 80.0),
            "M 10 20 L 70 20 L 70 36 L 130 36 L 130 100 L 10 100 Z"
        );
        assert_eq!(
            super::package_path(0.0, 0.0, 146.0, 192.0),
            "M 0 0 L 73 0 L 73 38 L 146 38 L 146 192 L 0 192 Z"
        );
        assert_eq!(
            super::callout_path(10.0, 20.0, 120.0, 80.0),
            "M 10 20 V 60 H 70 V 100 L 100 60 H 130 V 20 H 10 Z"
        );
        assert_eq!(
            super::step_path(10.0, 20.0, 120.0, 80.0),
            "M 10 20 L 95 20 L 130 60 L 95 100 L 10 100 L 45 60 Z"
        );
        assert_eq!(
            super::stored_data_path(10.0, 20.0, 120.0, 80.0),
            "M 25 20 H 130 C 126 20 115 42 115 60 C 115 78 126 100 130 100 H 25 C 21 100 10 78 10 60 C 10 42 21 20 25 20 Z"
        );
        assert_eq!(
            super::person_path(10.0, 20.0, 120.0, 80.0),
            "M 130 100 H 10 V 99 C 10 85 23 73 44 67 C 32 62 26 55 26 46 C 26 32 45 20 70 20 C 94 20 114 32 114 46 C 114 54 107 62 96 67 C 117 73 130 85 130 99 V 100 H 130 Z"
        );
        assert_eq!(
            super::document_path(10.0, 20.0, 120.0, 80.0),
            "M 10 89 L 10 20 L 130 20 L 130 89 C 110 74 90 74 70 89 C 50 104 30 104 10 89 Z"
        );
        assert_eq!(
            super::queue_paths(40.0, 381.0, 1014.0, 1016.0),
            (
                "M 64 381 H 1030 C 1054 381 1054 838 1054 889 C 1054 940 1054 1397 1030 1397 H 64 C 40 1397 40 940 40 889 C 40 838 40 381 64 381 Z".to_owned(),
                "M 1030 381 C 1006 381 1006 838 1006 889 C 1006 940 1006 1397 1030 1397".to_owned(),
            )
        );
    }

    #[test]
    fn c4_person_paths_match_v090_chop_precision() {
        assert_eq!(
            super::c4_person_paths(10.0, 20.0, 120.0, 80.0),
            (
                "M 10 76 C 10 72 14 68 18 68 H 122 C 126 68 130 72 130 76 V 92 C 130 96 126 100 122 100 H 18 C 14 100 10 96 10 92 Z".to_owned(),
                "M 70 20 C 85 20 96 32 96 46 C 96 61 85 73 70 73 C 55 73 44 61 44 46 C 44 32 55 20 70 20".to_owned(),
            )
        );
    }

    #[test]
    fn cylinder_and_page_paths_match_v090_chop_precision() {
        assert_eq!(
            super::cylinder_paths(10.0, 20.0, 120.0, 80.0),
            (
                "M 10 44 C 10 20 64 20 70 20 C 76 20 130 20 130 44 V 76 C 130 100 76 100 70 100 C 64 100 10 100 10 76 V 44 Z".to_owned(),
                "M 10 44 C 10 68 64 68 70 68 C 76 68 130 68 130 44".to_owned(),
            )
        );
        assert_eq!(
            super::page_paths(10.0, 20.0, 120.0, 80.0),
            (
                "M 11 20 H 109 C 110 20 111 20 112 21 L 129 37 C 130 38 130 39 130 40 V 100 C 130 100 130 100 130 100 H 10 C 10 100 10 100 10 100 V 20 C 10 20 10 20 11 20 Z".to_owned(),
                "M 129 100 H 11 C 10 100 10 100 10 99 V 21 C 10 20 10 20 11 20 H 108 C 109 20 109 20 109 21 V 38 C 109 39 110 40 111 40 H 129 C 130 40 130 40 130 41 V 99 C 129 100 130 100 129 100 Z".to_owned(),
            )
        );
    }

    #[test]
    fn cloud_path_uses_v090_integer_chop_precision() {
        assert_eq!(
            super::cloud_path(0.0, 0.0, 395.0, 395.0),
            "M 65 138 C 65 142 62 146 60 146 C 26 151 0 205 0 270 C 0 339 30 395 67 395 H 322 C 362 395 395 335 395 263 C 395 194 365 136 327 132 C 324 132 322 129 321 124 C 312 54 273 1 227 1 C 197 1 170 23 153 58 C 151 62 148 63 146 62 C 139 58 132 56 124 56 C 93 54 68 91 65 138 Z"
        );
    }

    #[test]
    fn hexagon_vertices_match_v090_shape_path_ratios() {
        let rect = Rect {
            origin: Point { x: 10.0, y: 20.0 },
            size: Size {
                width: 128.0,
                height: 69.0,
            },
        };
        assert_eq!(
            hexagon_points(rect),
            [
                Point { x: 42.0, y: 20.0 },
                Point { x: 10.0, y: 54.0 },
                Point { x: 42.0, y: 89.0 },
                Point { x: 106.0, y: 89.0 },
                Point { x: 138.0, y: 54.0 },
                Point { x: 106.0, y: 20.0 },
            ]
        );
    }

    #[test]
    fn multi_path_shapes_keep_upstream_open_detail_fill_semantics() {
        for shape in [ShapeKind::Cylinder, ShapeKind::Queue, ShapeKind::Page] {
            let output = render(shape);
            assert_eq!(output.matches("<path").count(), 2, "{shape:?}");
            assert_eq!(output.matches("fill=\"none\"").count(), 0, "{shape:?}");
        }
    }

    #[test]
    fn three_dee_rectangle_uses_separate_shaded_faces_and_one_outline() {
        let mut svg = String::new();
        let base = palette(0).expect("default palette");
        let overrides = BTreeMap::new();
        let style = SourceNodeStyle {
            fill: Some("red".into()),
            stroke: Some("black".into()),
            stroke_width: Some("2".into()),
            stroke_dash: Some("5".into()),
            opacity: Some("0.6".into()),
            three_dee: Some("true".into()),
            ..SourceNodeStyle::default()
        };
        draw_shape_with_id(
            &mut svg,
            ShapeKind::Rectangle,
            Rect {
                origin: Point { x: 10.0, y: 20.0 },
                size: Size {
                    width: 100.0,
                    height: 60.0,
                },
            },
            &style,
            PaletteView::new(base, &overrides),
            "sample.rect",
        );

        assert!(svg.contains("<g opacity=\"0.6\">"));
        assert!(svg.contains("<rect x=\"10\" y=\"20\" width=\"100\" height=\"60\" fill=\"red\""));
        assert!(svg.contains(
            "<polygon points=\"10,20 25,5 125,5 125,65 110,80 110,20\" fill=\"#cc0000\""
        ));
        assert!(svg.contains(
            "d=\"M10 20 L25 5 L125 5 L125 65 L110 80 L10 80 L10 20 L110 20 L110 80 M110 20 L125 5\""
        ));
        assert_eq!(
            svg.matches("<path").count(),
            2,
            "one mask outline and one visible outline"
        );
        assert_eq!(
            svg.matches("<polygon").count(),
            1,
            "one shaded side polygon"
        );
    }

    #[test]
    fn three_dee_hexagon_uses_v090_integer_point_scaling() {
        let mut svg = String::new();
        let base = palette(0).expect("default palette");
        let overrides = BTreeMap::new();
        let style = SourceNodeStyle {
            fill: Some("#ff0000".into()),
            three_dee: Some("true".into()),
            ..SourceNodeStyle::default()
        };
        draw_shape_with_id(
            &mut svg,
            ShapeKind::Hexagon,
            Rect {
                origin: Point { x: 10.0, y: 20.0 },
                size: Size {
                    width: 101.0,
                    height: 87.0,
                },
            },
            &style,
            PaletteView::new(base, &overrides),
            "sample.hex",
        );
        assert!(svg.contains("points=\"35,20 85,20 111,63 85,107 35,107 10,63\""));
        assert!(svg.contains("points=\"50,13 100,13 126,56 100,100 85,107 111,63 85,20 35,20\""));
    }

    #[test]
    fn darken_color_matches_d2_hsl_lightness_reduction() {
        assert_eq!(darken_color("red").as_deref(), Some("#cc0000"));
        assert_eq!(darken_color("#ff0000").as_deref(), Some("#cc0000"));
        assert_eq!(darken_color("linear-gradient(red, blue)"), None);
    }

    #[test]
    fn narrow_shapes_clamp_wedges_without_non_finite_path_values() {
        let mut svg = String::new();
        let overrides = BTreeMap::new();
        draw_shape(
            &mut svg,
            ShapeKind::Parallelogram,
            Rect {
                origin: Point { x: 0.0, y: 0.0 },
                size: Size {
                    width: 1.0,
                    height: 1.0,
                },
            },
            &SourceNodeStyle::default(),
            PaletteView::new(palette(0).expect("default palette"), &overrides),
        );
        assert!(!svg.contains("NaN"));
        assert!(!svg.contains("inf"));
    }

    #[test]
    fn gradients_emit_deterministic_svg_definitions_and_paint_urls() {
        let definition = gradient_definition("linear-gradient(to right, red, blue 75%)")
            .expect("valid linear gradient");
        let id = gradient_id("linear-gradient(to right, red, blue 75%)");
        assert!(definition.contains(&format!("id=\"{id}\"")));
        assert!(definition.contains("x1=\"0%\" y1=\"50%\" x2=\"100%\" y2=\"50%\""));
        assert!(definition.contains("offset=\"0.00%\""));
        assert!(definition.contains("offset=\"75%\""));

        let mut svg = String::new();
        let base = palette(0).expect("default palette");
        let overrides = BTreeMap::new();
        let style = SourceNodeStyle {
            fill: Some("radial-gradient(circle, #fff, #000)".into()),
            ..SourceNodeStyle::default()
        };
        draw_shape(
            &mut svg,
            ShapeKind::Rectangle,
            Rect {
                origin: Point { x: 0.0, y: 0.0 },
                size: Size {
                    width: 20.0,
                    height: 20.0,
                },
            },
            &style,
            PaletteView::new(base, &overrides),
        );
        assert!(svg.contains(&format!(
            "fill=\"url(#{})\"",
            gradient_id("radial-gradient(circle, #fff, #000)")
        )));
    }

    #[test]
    fn sql_tables_render_rows_and_column_names() {
        let mut svg = String::new();
        let overrides = BTreeMap::new();
        draw_shape(
            &mut svg,
            ShapeKind::SqlTable,
            Rect {
                origin: Point { x: 0.0, y: 0.0 },
                size: Size {
                    width: 140.0,
                    height: 70.0,
                },
            },
            &SourceNodeStyle {
                table_columns: vec!["id".into(), "name".into()],
                ..SourceNodeStyle::default()
            },
            PaletteView::new(palette(0).expect("default palette"), &overrides),
        );
        assert!(svg.contains("<line"));
        assert!(svg.contains(">id</text>"));
        assert!(svg.contains(">name</text>"));
    }

    #[test]
    fn sql_table_types_use_measured_widest_column_name() {
        let mut svg = String::new();
        let overrides = BTreeMap::new();
        draw_sql_table_shape(
            &mut svg,
            Rect {
                origin: Point { x: 0.0, y: 0.0 },
                size: Size {
                    width: 240.0,
                    height: 100.0,
                },
            },
            &SourceNodeStyle {
                table_rows: vec![("parent_id".into(), "int".into(), String::new())],
                table_max_name_width: 85,
                ..SourceNodeStyle::default()
            },
            "users",
            24,
            Size {
                width: 50.0,
                height: 20.0,
            },
            PaletteView::new(palette(0).expect("default palette"), &overrides),
            None,
        );
        assert!(svg.contains(
            "<text x=\"115\" y=\"80\" fill=\"#676C7E\" class=\"d2-rs-text-N2\" font-family=\"Source Sans Pro, sans-serif\" font-size=\"20\">int</text>"
        ));
    }
}

#[cfg(test)]
mod original_class_text_contract {
    #[cfg(feature = "oracle-fixtures")]
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    use std::collections::{BTreeMap, BTreeSet};

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn class_text_retains_complete_original_go_origins_and_unique_ids() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/class-text-origins-source-v090.json"
        ))
        .unwrap();
        let rows = rows.as_array().unwrap();
        assert_eq!(rows.len(), 12);
        let mut ids = BTreeSet::new();
        for (case, row) in rows.iter().enumerate() {
            let font_size = row["font_size"].as_u64().unwrap() as u32;
            let style = SourceNodeStyle {
                font_size: Some(font_size),
                class_fields: (0..row["fields"].as_u64().unwrap())
                    .map(|i| (format!("f{i}"), "int".into()))
                    .collect(),
                class_methods: (0..row["methods"].as_u64().unwrap())
                    .map(|i| (format!("m{i}()"), "void".into()))
                    .collect(),
                ..SourceNodeStyle::default()
            };
            let mut native = String::from("<svg>");
            let mut public = native.clone();
            let overrides = BTreeMap::new();
            let palette = PaletteView::new(d2_rs_core::theme::palette(0).unwrap(), &overrides);
            let rect = Rect {
                origin: Point {
                    x: 1032.0,
                    y: 101.0,
                },
                size: Size {
                    width: 590.0,
                    height: 500.0,
                },
            };
            let size = Size {
                width: 24.0,
                height: 25.0,
            };
            draw_class_shape(
                &mut native,
                rect,
                &style,
                "C",
                font_size + 4,
                size,
                palette,
                Some(&format!("class-{case}")),
            );
            draw_class_shape(
                &mut public,
                rect,
                &style,
                "C",
                font_size + 4,
                size,
                palette,
                None,
            );
            native.push_str("</svg>");
            public.push_str("</svg>");
            assert!(!public.contains("data-d2-rs-text-"));
            let document = roxmltree::Document::parse(&native).unwrap();
            let texts: Vec<_> = document
                .descendants()
                .filter(|n| n.has_tag_name("text"))
                .collect();
            let expected = row["runs"].as_array().unwrap();
            assert_eq!(texts.len(), expected.len());
            for (actual, expected) in texts.iter().zip(expected) {
                assert!(
                    ids.insert(actual.attribute("id").unwrap().to_owned()),
                    "all owners need unique IDs"
                );
                assert_eq!(actual.text().unwrap(), expected["text"].as_str().unwrap());
                assert_eq!(
                    actual.attribute("data-d2-rs-text-x-bits").unwrap(),
                    expected["x_bits"].as_str().unwrap(),
                    "{row}"
                );
                assert_eq!(
                    actual.attribute("data-d2-rs-text-y-bits").unwrap(),
                    expected["y_bits"].as_str().unwrap(),
                    "{row}"
                );
                let anchor = match actual.attribute("text-anchor") {
                    Some("middle") => 1,
                    Some("end") => 2,
                    _ => 0,
                };
                assert_eq!(anchor, expected["anchor"].as_u64().unwrap());
            }
        }
    }
}

#[cfg(test)]
mod multiline_class_header_source_contract {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};
    #[test]
    fn original_multiline_header_runs_keep_regular_face_and_independent_baselines() {
        // Original d2scenebuild/special_shapes_test.go TestBuildClassShapeMultilineHeader.
        let style = SourceNodeStyle {
            font_size: Some(16),
            ..Default::default()
        };
        let overrides = BTreeMap::new();
        let palette = PaletteView::new(d2_rs_core::theme::palette(0).unwrap(), &overrides);
        let mut svg = String::from("<svg>");
        draw_class_shape(
            &mut svg,
            Rect {
                origin: Point { x: 0.0, y: 0.0 },
                size: Size {
                    width: 200.0,
                    height: 120.0,
                },
            },
            &style,
            "First\nSecond\nThird",
            20,
            Size {
                width: 100.0,
                height: 60.0,
            },
            palette,
            Some("class"),
        );
        svg.push_str("</svg>");
        let document = roxmltree::Document::parse(&svg).unwrap();
        let texts = document
            .descendants()
            .filter(|n| n.has_tag_name("text"))
            .collect::<Vec<_>>();
        assert_eq!(texts.len(), 3);
        let mut ids = BTreeSet::new();
        for (index, (run, text)) in texts.iter().zip(["First", "Second", "Third"]).enumerate() {
            assert!(ids.insert(run.attribute("id").unwrap()));
            assert_eq!(run.text(), Some(text));
            assert_eq!(run.attribute("font-size"), Some("20"));
            assert!(run.attribute("font-weight").is_none());
            assert!(run.attribute("font-style").is_none());
            assert_eq!(
                run.attribute("data-d2-rs-text-x-bits").unwrap(),
                format!("{:016x}", 100.0f64.to_bits())
            );
            assert_eq!(
                run.attribute("data-d2-rs-text-y-bits").unwrap(),
                format!("{:016x}", (46.0 + index as f64 * 20.0).to_bits())
            );
        }
    }
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_compiled_class_row_underlines_reach_each_name_only() {
        let fixture: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/class-header-underline-source-v090.json"
        ))
        .unwrap();
        assert_eq!(fixture["cases"].as_array().unwrap().len(), 28);
        for row in fixture["cases"].as_array().unwrap() {
            let parsed =
                d2_rs_language::parse("class.d2", row["source"].as_str().unwrap().as_bytes(), None);
            assert!(parsed.error.is_none());
            let (compiler, root) =
                d2_rs_language::compiler::compile_ir(d2_rs_core::ast::Shared::new(parsed.map))
                    .unwrap();
            let (graph, _, styles) =
                d2_rs_language::compiler::lower_graph_with_measurer_and_labels(
                    &compiler,
                    root,
                    &|_: &str| Size {
                        width: 20.0,
                        height: 20.0,
                    },
                )
                .unwrap();
            let (id, _) = graph.nodes().find(|(_, n)| n.external_id == "x").unwrap();
            let style = &styles[&id];
            let shape = &row["diagram"]["shapes"][0];
            for (actual, expected) in [
                (&style.class_field_underlines, &shape["fields"]),
                (&style.class_method_underlines, &shape["methods"]),
            ] {
                assert_eq!(
                    *actual,
                    expected
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|m| m["underline"].as_bool().unwrap())
                        .collect::<Vec<_>>(),
                    "{row}"
                );
            }
            let overrides = BTreeMap::new();
            let palette = PaletteView::new(d2_rs_core::theme::palette(0).unwrap(), &overrides);
            let mut svg = String::from("<svg>");
            draw_class_shape(
                &mut svg,
                Rect {
                    origin: Point { x: 0.0, y: 0.0 },
                    size: Size {
                        width: 200.0,
                        height: 120.0,
                    },
                },
                style,
                "Header",
                24,
                Size {
                    width: 100.0,
                    height: 20.0,
                },
                palette,
                None,
            );
            svg.push_str("</svg>");
            let document = roxmltree::Document::parse(&svg).unwrap();
            for (name, underline) in [
                ("f", style.class_field_underlines[0]),
                ("m()", style.class_method_underlines[0]),
            ] {
                let text = document
                    .descendants()
                    .find(|n| n.has_tag_name("text") && n.text() == Some(name))
                    .unwrap();
                assert_eq!(
                    text.attribute("text-decoration"),
                    underline.then_some("underline")
                );
            }
        }
    }
}

#[cfg(test)]
mod original_sql_scene_origin_contract {
    #[cfg(feature = "oracle-fixtures")]
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn sql_text_runs_retain_original_origins_and_raw_text() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/sql-text-origins-source-v090.json"
        ))
        .unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 48);
        for row in rows.as_array().unwrap() {
            let count = row["count"].as_u64().unwrap() as usize;
            let size = row["size"].as_u64().unwrap() as u32;
            let overrides = std::collections::BTreeMap::new();
            let style = SourceNodeStyle {
                table_rows: (0..count)
                    .map(|i| {
                        (
                            format!("name{i}"),
                            if i == 1 {
                                "var  char".into()
                            } else {
                                "varchar".into()
                            },
                            "PK".into(),
                        )
                    })
                    .collect(),
                table_max_name_width: if count == 0 {
                    0
                } else {
                    35 + (count as u32 - 1) * 7
                },
                ..Default::default()
            };
            let mut svg = String::from("<svg>");
            draw_sql_table_shape(
                &mut svg,
                Rect {
                    origin: Point {
                        x: 10.,
                        y: row["y"].as_f64().unwrap(),
                    },
                    size: Size {
                        width: 240.,
                        height: row["height"].as_f64().unwrap(),
                    },
                },
                &style,
                "users",
                size + 4,
                Size {
                    width: 50.,
                    height: 20.,
                },
                PaletteView::new(d2_rs_core::theme::palette(0).unwrap(), &overrides),
                Some("sql-owner"),
            );
            svg.push_str("</svg>");
            let doc = roxmltree::Document::parse(&svg).unwrap();
            let actual = doc
                .descendants()
                .filter(|n| n.has_tag_name("text"))
                .collect::<Vec<_>>();
            let expected = row["runs"].as_array().unwrap();
            assert_eq!(actual.len(), expected.len(), "{row}");
            let mut ids = std::collections::BTreeSet::new();
            for (node, source) in actual.iter().zip(expected) {
                assert_eq!(
                    node.text().unwrap_or(""),
                    source["text"].as_str().unwrap(),
                    "{row}"
                );
                assert!(ids.insert(node.attribute("id").unwrap()), "{row}");
                assert_eq!(node.attribute("data-d2-rs-text-raw"), Some("true"));
                for (attribute, key) in [
                    ("data-d2-rs-text-x-bits", "x_bits"),
                    ("data-d2-rs-text-y-bits", "y_bits"),
                ] {
                    let actual =
                        u64::from_str_radix(node.attribute(attribute).unwrap(), 16).unwrap();
                    assert_eq!(actual, source[key].as_u64().unwrap(), "{row}: {attribute}");
                }
            }
        }
    }
}

#[cfg(test)]
mod class_sketch_scene_source_contract {
    #[cfg(feature = "oracle-fixtures")]
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    use std::collections::{BTreeMap, BTreeSet};
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn twelve_complete_source_class_scenes_retain_geometry_order_and_regular_mono_text() {
        let controls: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/class-sketch-scenes-source-v090.json"
        ))
        .unwrap();
        for control in controls.as_array().unwrap() {
            let field_count = control["fields"].as_u64().unwrap() as usize;
            let method_count = control["methods"].as_u64().unwrap() as usize;
            let font_size = control["font_size"].as_u64().unwrap() as u32;
            let style = SourceNodeStyle {
                fill: Some("#112233".into()),
                stroke: Some("#ffffff".into()),
                stroke_width: Some("2".into()),
                font_size: Some(font_size),
                bold: true,
                italic: true,
                class_fields: (0..field_count)
                    .map(|i| (format!("f{i}"), "int".into()))
                    .collect(),
                class_methods: (0..method_count)
                    .map(|i| (format!("m{i}()"), "void".into()))
                    .collect(),
                ..Default::default()
            };
            let mut svg = String::new();
            draw_class_shape_with_sketch(
                &mut svg,
                Rect {
                    origin: Point { x: 10.0, y: 20.0 },
                    size: Size {
                        width: 200.0,
                        height: 120.0,
                    },
                },
                &style,
                "C",
                font_size,
                Size {
                    width: 24.0,
                    height: 25.0,
                },
                PaletteView::new(d2_rs_core::theme::palette(0).unwrap(), &BTreeMap::new()),
                Some("c"),
                Some(&mut BTreeSet::new()),
            );
            let svg = format!("<svg>{svg}</svg>");
            let doc = roxmltree::Document::parse(&svg).unwrap();
            let actual = doc
                .descendants()
                .filter(|n| {
                    n.is_element()
                        && (n.attribute("data-d2-rs-sketch-set") == Some("rough")
                            || n.has_tag_name("text")
                            || (n.has_tag_name("rect")
                                && n.parent()
                                    .and_then(|p| p.attribute("class"))
                                    .is_some_and(|c| c.starts_with("sketch-streak-overlay"))))
                })
                .collect::<Vec<_>>();
            let expected = control["nodes"].as_array().unwrap();
            assert_eq!(actual.len(), expected.len(), "{control}");
            for (node, want) in actual.iter().zip(expected) {
                match want["kind"].as_str().unwrap() {
                    "path" => {
                        assert!(node.has_tag_name("path"));
                        let mut kinds = Vec::new();
                        let mut values = Vec::new();
                        for segment in svgtypes::PathParser::from(node.attribute("d").unwrap()) {
                            match segment.unwrap() {
                                svgtypes::PathSegment::MoveTo { abs: true, x, y } => {
                                    kinds.push(0);
                                    values.extend([x, y]);
                                }
                                svgtypes::PathSegment::LineTo { abs: true, x, y } => {
                                    kinds.push(1);
                                    values.extend([x, y]);
                                }
                                svgtypes::PathSegment::CurveTo {
                                    abs: true,
                                    x1,
                                    y1,
                                    x2,
                                    y2,
                                    x,
                                    y,
                                } => {
                                    kinds.push(3);
                                    values.extend([x1, y1, x2, y2, x, y]);
                                }
                                other => panic!("unexpected {other:?}"),
                            }
                        }
                        assert_eq!(serde_json::json!(kinds), want["kinds"]);
                        let coords = want["coordinates"].as_array().unwrap();
                        assert_eq!(values.len(), coords.len());
                        for (value, expected) in values.iter().zip(coords) {
                            assert_eq!(
                                value.to_bits(),
                                expected.as_str().unwrap().parse::<f64>().unwrap().to_bits(),
                                "{} {control}",
                                want["id"]
                            );
                        }
                        let header = want["id"].as_str().unwrap().contains("class-header");
                        let separator = want["id"].as_str().unwrap().contains("separator");
                        assert_eq!(
                            node.attribute("fill"),
                            Some(if header || separator {
                                "#112233"
                            } else {
                                "#ffffff"
                            })
                        );
                        assert_eq!(
                            node.attribute("stroke"),
                            Some(if header || separator {
                                "none"
                            } else {
                                "#112233"
                            })
                        );
                        if !separator {
                            assert_eq!(node.attribute("transform"), Some("translate(10 20)"));
                        }
                    }
                    "text" => {
                        assert!(node.has_tag_name("text"));
                        assert_eq!(node.text(), want["text"].as_str());
                        assert_eq!(node.attribute("class"), Some("text-mono"));
                        assert_eq!(
                            node.attribute("font-family"),
                            Some("Source Code Pro, monospace")
                        );
                        assert_eq!(
                            node.attribute("font-size").unwrap().parse::<u32>().unwrap() as u64,
                            want["font"]["Size"].as_u64().unwrap()
                        );
                        assert!(
                            node.attribute("font-weight").is_none()
                                && node.attribute("font-style").is_none()
                        );
                    }
                    "streak" => {
                        assert!(node.has_tag_name("rect"));
                        for (attr, key) in [
                            ("x", "X"),
                            ("y", "Y"),
                            ("width", "Width"),
                            ("height", "Height"),
                        ] {
                            assert_eq!(
                                node.attribute(attr).unwrap().parse::<f64>().unwrap(),
                                want["box"][key].as_f64().unwrap()
                            );
                        }
                    }
                    other => panic!("unexpected {other}"),
                }
            }
        }
    }
}
