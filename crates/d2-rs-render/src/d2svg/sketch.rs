// SPDX-License-Identifier: MPL-2.0
// Port of D2 v0.9.0 d2renderers/d2sketch/sketch.go (the SVG renderer half;
// scene.go/scene_connection.go serve the raster scene and are not used by
// d2svg) and d2themes/sketch_overlay.go. Rough drawing itself is the
// rough-go v0.2.0 translation in the d2-rs-sketch crate.
//! Hand-drawn (`sketch: true`) shape and connection SVG, byte-identical to
//! `d2sketch` for the same target diagram.

use std::fmt::Write as _;

use d2_rs_sketch::{Drawable, Generator, Options, RoughPoint, js_number_string};

use super::color;
use super::element::ThemableElement;
use super::geo::{self, Point};
use super::label::Position;
use super::svgpath;
use super::svgtext::escape_text;
use crate::target::{TargetConnection, TargetShape};

/// d2sketch BG_COLOR (color.N7).
const BG_COLOR: &str = "N7";
const INNER_BORDER_OFFSET: i64 = 5;
const PREFIX_PADDING: f64 = 10.0;
const PREFIX_WIDTH: f64 = 20.0;
const NAME_PADDING: f64 = 10.0;
const TYPE_PADDING: f64 = 20.0;

/// sketch.go `newGenerator`.
fn new_generator() -> Generator {
    Generator::new(
        Some(&Options {
            seed: Some(1.0),
            ..Default::default()
        }),
        None,
    )
}

/// sketch.go `baseOptions`.
fn base_options(stroke_width: f64) -> Options {
    Options {
        fill: Some("#000".into()),
        stroke: Some("#000".into()),
        stroke_width: Some(stroke_width),
        fill_weight: Some(2.0),
        hachure_gap: Some(16.0),
        fill_style: Some("solid".into()),
        bowing: Some(2.0),
        seed: Some(1.0),
        ..Default::default()
    }
}

/// patternassets.StreaksSVG, already formatted with its three `%s` verbs.
fn streaks_svg(luminance_category: &str, diagram_hash: &str, fill: &str) -> String {
    format!(
        "<pattern id=\"streaks-{luminance_category}-{diagram_hash}\" x=\"0\" y=\"0\" width=\"100\" height=\"100\" patternUnits=\"userSpaceOnUse\">\n    <path fill=\"{fill}\" fill-rule=\"evenodd\" clip-rule=\"evenodd\" d=\"{}\" />\n</pattern>\n",
        crate::patternassets::streak_path()
    )
}

/// sketch.go `DefineFillPatterns`: reusable streak overlays, defined only when
/// the (already written) buffer references them.
pub(crate) fn define_fill_patterns(buf: &mut String, diagram_hash: &str) {
    let source = buf.clone();
    buf.push_str("<defs>");
    for (category, fill) in [
        ("bright", "rgba(0, 0, 0, 0.1)"),
        ("normal", "rgba(0, 0, 0, 0.16)"),
        ("dark", "rgba(0, 0, 0, 0.32)"),
        ("darker", "rgba(255, 255, 255, 0.24)"),
    ] {
        if source.contains(&format!("url(#streaks-{category}-{diagram_hash})")) {
            buf.push_str(&streaks_svg(category, diagram_hash, fill));
        }
    }
    buf.push_str("</defs>");
}

/// d2themes.ShapeTheme.
fn shape_theme(s: &TargetShape) -> (String, String) {
    if s.shape_type == "sql_table" || s.shape_type == "class" {
        (s.stroke.clone(), s.fill.clone())
    } else {
        (s.fill.clone(), s.stroke.clone())
    }
}

/// `el.Fill` after ThemableElement.Render, which rewrites a CSS gradient to
/// its `url('#id')` reference in place.
fn fill_after_render(fill: &str) -> String {
    if !color::is_theme_color(fill) && color::is_gradient(fill) {
        format!("url('#{}')", color::unique_gradient_id(fill))
    } else {
        fill.to_owned()
    }
}

/// d2themes.ThemableSketchOverlay.Render. Like the source, it appends to the
/// element's class, so a reused element accumulates suffixes.
fn render_sketch_overlay(el: &mut ThemableElement<'_>, fill: &str) -> Result<String, String> {
    if color::is_theme_color(fill) {
        write!(el.class_name, " sketch-overlay-{fill}").unwrap();
    } else {
        let lc = color::luminance_category(fill)?;
        write!(el.class_name, " sketch-overlay-{lc}").unwrap();
    }
    Ok(el.render())
}

fn border_label(s: &TargetShape) -> bool {
    !s.label.is_empty() && Position::from_string(s.label_position_str()).is_border()
}

/// sketch.go `roughPath` as produced by `computeRoughPaths`.
struct RoughPath {
    d: String,
    stroke: String,
    stroke_width: String,
    fill: String,
}

impl RoughPath {
    /// sketch.go `roughPath.StyleCSS`.
    fn style_css(&self) -> String {
        if self.stroke_width.is_empty() {
            String::new()
        } else {
            format!("stroke-width:{};", self.stroke_width)
        }
    }
}

/// sketch.go `computeRoughPaths`.
fn compute_rough_paths(g: &Generator, drawable: &Drawable) -> Vec<RoughPath> {
    g.to_paths(drawable)
        .into_iter()
        .map(|path| RoughPath {
            d: truncate_path_data(&path.d),
            stroke: path.stroke,
            stroke_width: js_number_string(path.stroke_width),
            fill: path.fill,
        })
        .collect()
}

/// sketch.go `computeRoughPathData`.
fn compute_rough_path_data(g: &Generator, drawable: &Drawable) -> Vec<String> {
    compute_rough_paths(g, drawable)
        .into_iter()
        .map(|p| p.d)
        .collect()
}

/// sketch.go `truncatePathData`: every `(\d+)\.(\d+)` keeps at most six
/// decimals (truncated, not rounded).
fn truncate_path_data(path_data: &str) -> String {
    let bytes = path_data.as_bytes();
    let mut out = String::with_capacity(path_data.len());
    let mut i = 0;
    while i < bytes.len() {
        if !bytes[i].is_ascii_digit() {
            let start = i;
            while i < bytes.len() && !bytes[i].is_ascii_digit() {
                i += 1;
            }
            out.push_str(&path_data[start..i]);
            continue;
        }
        let start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if i + 1 < bytes.len() && bytes[i] == b'.' && bytes[i + 1].is_ascii_digit() {
            let decimals_start = i + 1;
            let mut end = decimals_start;
            while end < bytes.len() && bytes[end].is_ascii_digit() {
                end += 1;
            }
            out.push_str(&path_data[start..decimals_start + (end - decimals_start).min(6)]);
            i = end;
        } else {
            out.push_str(&path_data[start..i]);
        }
    }
    out
}

/// The shared `pathEl` of Rect/Oval/DoubleRect/DoubleOval.
fn shape_path_el<'a>(s: &TargetShape, diagram_hash: &str, x: f64, y: f64) -> ThemableElement<'a> {
    let mut el = ThemableElement::new("path", None);
    el.set_translate(x, y);
    (el.fill, el.stroke) = shape_theme(s);
    el.fill_pattern = s.fill_pattern_str().to_owned();
    el.class_name = "shape".into();
    el.style = s.css_style();
    if border_label(s) {
        el.mask = format!("url(#{diagram_hash})");
    }
    el
}

fn render_paths(el: &mut ThemableElement<'_>, paths: &[String], output: &mut String) {
    for p in paths {
        el.d.clone_from(p);
        output.push_str(&el.render());
    }
}

/// sketch.go `Rect`.
pub(crate) fn rect(s: &TargetShape, diagram_hash: &str) -> Result<String, String> {
    let mut g = new_generator();
    let d = g.rectangle(
        0.0,
        0.0,
        s.width as f64,
        s.height as f64,
        Some(&base_options(s.stroke_width as f64)),
    );
    let paths = compute_rough_path_data(&g, &d);
    let mut output = String::new();
    let (x, y) = (s.pos.x as f64, s.pos.y as f64);
    let mut path_el = shape_path_el(s, diagram_hash, x, y);
    render_paths(&mut path_el, &paths, &mut output);
    let mut overlay = ThemableElement::new("rect", None);
    overlay.set_translate(x, y);
    overlay.width = s.width as f64;
    overlay.height = s.height as f64;
    if border_label(s) {
        overlay.mask = format!("url(#{diagram_hash})");
    }
    let fill = if paths.is_empty() {
        path_el.fill.clone()
    } else {
        fill_after_render(&path_el.fill)
    };
    output += &render_sketch_overlay(&mut overlay, &fill)?;
    Ok(output)
}

/// sketch.go `DoubleRect`.
pub(crate) fn double_rect(s: &TargetShape, diagram_hash: &str) -> Result<String, String> {
    let mut g = new_generator();
    let big = g.rectangle(
        0.0,
        0.0,
        s.width as f64,
        s.height as f64,
        Some(&base_options(s.stroke_width as f64)),
    );
    let paths_big = compute_rough_path_data(&g, &big);
    let small = g.rectangle(
        0.0,
        0.0,
        (s.width - INNER_BORDER_OFFSET * 2) as f64,
        (s.height - INNER_BORDER_OFFSET * 2) as f64,
        Some(&base_options(s.stroke_width as f64)),
    );
    let paths_small = compute_rough_path_data(&g, &small);
    let mut output = String::new();
    let (x, y) = (s.pos.x as f64, s.pos.y as f64);
    let mut path_el = shape_path_el(s, diagram_hash, x, y);
    render_paths(&mut path_el, &paths_big, &mut output);
    let ib = INNER_BORDER_OFFSET as f64;
    let mut inner = shape_path_el(s, diagram_hash, x + ib, y + ib);
    // No need for inner to double paint.
    inner.fill = "transparent".into();
    inner.fill_pattern.clear();
    render_paths(&mut inner, &paths_small, &mut output);
    let mut overlay = ThemableElement::new("rect", None);
    overlay.set_translate(x, y);
    overlay.width = s.width as f64;
    overlay.height = s.height as f64;
    if border_label(s) {
        overlay.mask = format!("url(#{diagram_hash})");
    }
    output += &render_sketch_overlay(&mut overlay, &s.fill)?;
    Ok(output)
}

fn oval_overlay<'a>(s: &TargetShape, diagram_hash: &str) -> ThemableElement<'a> {
    let mut el = ThemableElement::new("ellipse", None);
    el.set_translate(
        (s.pos.x + s.width / 2) as f64,
        (s.pos.y + s.height / 2) as f64,
    );
    el.rx = (s.width / 2) as f64;
    el.ry = (s.height / 2) as f64;
    if border_label(s) {
        el.mask = format!("url(#{diagram_hash})");
    }
    el
}

/// sketch.go `Oval`.
pub(crate) fn oval(s: &TargetShape, diagram_hash: &str) -> Result<String, String> {
    let mut g = new_generator();
    let d = g.ellipse(
        (s.width / 2) as f64,
        (s.height / 2) as f64,
        s.width as f64,
        s.height as f64,
        Some(&base_options(s.stroke_width as f64)),
    );
    let paths = compute_rough_path_data(&g, &d);
    let mut output = String::new();
    let mut path_el = shape_path_el(s, diagram_hash, s.pos.x as f64, s.pos.y as f64);
    render_paths(&mut path_el, &paths, &mut output);
    let fill = if paths.is_empty() {
        path_el.fill.clone()
    } else {
        fill_after_render(&path_el.fill)
    };
    output += &render_sketch_overlay(&mut oval_overlay(s, diagram_hash), &fill)?;
    Ok(output)
}

/// sketch.go `DoubleOval`.
pub(crate) fn double_oval(s: &TargetShape, diagram_hash: &str) -> Result<String, String> {
    let mut g = new_generator();
    let (cx, cy) = ((s.width / 2) as f64, (s.height / 2) as f64);
    let big = g.ellipse(
        cx,
        cy,
        s.width as f64,
        s.height as f64,
        Some(&base_options(s.stroke_width as f64)),
    );
    let paths_big = compute_rough_path_data(&g, &big);
    let small = g.ellipse(
        cx,
        cy,
        (s.width - INNER_BORDER_OFFSET * 2) as f64,
        (s.height - INNER_BORDER_OFFSET * 2) as f64,
        Some(&base_options(s.stroke_width as f64)),
    );
    let paths_small = compute_rough_path_data(&g, &small);
    let mut output = String::new();
    let (x, y) = (s.pos.x as f64, s.pos.y as f64);
    let mut path_el = shape_path_el(s, diagram_hash, x, y);
    render_paths(&mut path_el, &paths_big, &mut output);
    let mut inner = shape_path_el(s, diagram_hash, x, y);
    // No need for inner to double paint.
    inner.fill = "transparent".into();
    inner.fill_pattern.clear();
    render_paths(&mut inner, &paths_small, &mut output);
    output += &render_sketch_overlay(&mut oval_overlay(s, diagram_hash), &s.fill)?;
    Ok(output)
}

/// sketch.go `Paths`.
pub(crate) fn paths(
    s: &TargetShape,
    diagram_hash: &str,
    paths: &[String],
) -> Result<String, String> {
    let mut output = String::new();
    for path in paths {
        let mut g = new_generator();
        let d = g.path(path, Some(&base_options(s.stroke_width as f64)));
        let sketch_paths = compute_rough_path_data(&g, &d);
        let mut path_el = ThemableElement::new("path", None);
        (path_el.fill, path_el.stroke) = shape_theme(s);
        path_el.fill_pattern = s.fill_pattern_str().to_owned();
        path_el.class_name = "shape".into();
        path_el.style = s.css_style();
        if border_label(s) {
            path_el.mask = format!("url(#{diagram_hash})");
        }
        render_paths(&mut path_el, &sketch_paths, &mut output);
        let fill = if sketch_paths.is_empty() {
            path_el.fill.clone()
        } else {
            fill_after_render(&path_el.fill)
        };
        // One overlay element is reused for every rough path.
        let mut overlay = ThemableElement::new("path", None);
        if border_label(s) {
            overlay.mask = format!("url(#{diagram_hash})");
        }
        for p in &sketch_paths {
            overlay.d.clone_from(p);
            output += &render_sketch_overlay(&mut overlay, &fill)?;
        }
    }
    Ok(output)
}

/// sketch.go `Connection`.
pub(crate) fn connection(c: &TargetConnection, path: &str, attrs: &str) -> Result<String, String> {
    let attributes = super::markup::parse_attributes(attrs)?;
    connection_with_attributes(c, path, &attributes)
}

/// Master sketch.go `ConnectionWithAttributes`: reject caller conflicts before
/// rough drawing or SVG serialization, including duplicate typed attributes.
pub(crate) fn connection_with_attributes(
    c: &TargetConnection,
    path: &str,
    attributes: &[super::markup::Attribute],
) -> Result<String, String> {
    let mut seen = std::collections::HashSet::from(["class", "d", "fill", "stroke", "style"]);
    for attribute in attributes {
        let name = attribute.name();
        if name.is_empty() {
            return Err("invalid zero-value SVG attribute".into());
        }
        if !seen.insert(name) {
            return Err(format!(
                "connection attribute {name:?} conflicts with a renderer-controlled attribute"
            ));
        }
    }
    let animated_class = if c.animated {
        " animated-connection"
    } else {
        ""
    };
    if c.animated {
        let both_or_neither = (c.dst_arrow == "none") == (c.src_arrow == "none");
        let new_el = |d: String| {
            let mut el = ThemableElement::new("path", None);
            el.d = d;
            el.fill = "none".into();
            el.stroke.clone_from(&c.stroke);
            el.class_name = format!("connection{animated_class}");
            el.style = c.css_style();
            el.add_attributes(attributes.iter().cloned());
            el
        };
        if both_or_neither {
            // There is no pure CSS way to animate bidirectional connections in
            // two directions, so the path is split.
            let (path1, path2) = svgpath::split_path(path, 0.5)?;
            let mut el1 = new_el(path1);
            el1.style += "animation-direction: reverse;";
            let el2 = new_el(path2);
            return Ok(el1.render() + " " + &el2.render());
        }
        return Ok(new_el(path.to_owned()).render());
    }
    let mut g = new_generator();
    let d = g.path(
        path,
        Some(&Options {
            roughness: Some(0.5),
            seed: Some(1.0),
            ..Default::default()
        }),
    );
    let paths = compute_rough_path_data(&g, &d);
    let mut output = String::new();
    let mut el = ThemableElement::new("path", None);
    el.fill = "none".into();
    el.stroke.clone_from(&c.stroke);
    el.class_name = format!("connection{animated_class}");
    el.style = c.css_style();
    el.add_attributes(attributes.iter().cloned());
    render_paths(&mut el, &paths, &mut output);
    Ok(output)
}

/// sketch.go `Table`.
pub(crate) fn table(s: &TargetShape) -> Result<String, String> {
    let mut output = String::new();
    let mut g = new_generator();
    let d = g.rectangle(
        0.0,
        0.0,
        s.width as f64,
        s.height as f64,
        Some(&base_options(s.stroke_width as f64)),
    );
    let paths = compute_rough_path_data(&g, &d);
    let (x, y) = (s.pos.x as f64, s.pos.y as f64);
    let mut path_el = ThemableElement::new("path", None);
    path_el.set_translate(x, y);
    (path_el.fill, path_el.stroke) = shape_theme(s);
    path_el.fill_pattern = s.fill_pattern_str().to_owned();
    path_el.class_name = "shape".into();
    path_el.style = s.css_style();
    render_paths(&mut path_el, &paths, &mut output);

    let b = geo::Box::new(Point::new(x, y), s.width as f64, s.height as f64);
    let columns = s.columns.as_deref().unwrap_or(&[]);
    let row_height = b.height / (1 + columns.len()) as f64;
    let header = geo::Box::new(b.top_left, b.width, row_height);

    let d = g.rectangle(
        0.0,
        0.0,
        s.width as f64,
        row_height,
        Some(&base_options(1.0)),
    );
    let paths = compute_rough_path_data(&g, &d);
    let mut header_el = ThemableElement::new("path", None);
    header_el.set_translate(x, y);
    header_el.fill.clone_from(&s.fill);
    header_el.fill_pattern = s.fill_pattern_str().to_owned();
    header_el.class_name = "class_header".into();
    render_paths(&mut header_el, &paths, &mut output);
    let header_fill = if paths.is_empty() {
        header_el.fill.clone()
    } else {
        fill_after_render(&header_el.fill)
    };

    if !s.label.is_empty() {
        let tl = Position::InsideMiddleLeft.get_point_on_box(
            &header,
            20.0,
            s.label_width as f64,
            s.label_height as f64,
        );
        let mut text = ThemableElement::new("text", None);
        text.x = tl.x;
        text.y = tl.y + s.label_height as f64 * 3.0 / 4.0;
        text.fill = s.get_font_color();
        text.class_name = "text".into();
        text.style = format!("text-anchor:start;font-size:{}px", 4 + s.font_size);
        text.content = escape_text(&s.label);
        output += &text.render();
    }

    let longest_name_width = columns.iter().map(|c| c.name.label_width).fold(0, i64::max);
    let font_size = s.font_size as f64;
    let mut row = geo::Box::new(b.top_left, b.width, row_height);
    row.top_left.y += header.height;
    for column in columns {
        let name_tl =
            Position::InsideMiddleLeft.get_point_on_box(&row, NAME_PADDING, row.width, font_size);
        let constraint_tr =
            Position::InsideMiddleRight.get_point_on_box(&row, TYPE_PADDING, 0.0, font_size);
        let mut text = ThemableElement::new("text", None);
        text.x = name_tl.x;
        text.y = name_tl.y + font_size * 3.0 / 4.0;
        text.fill.clone_from(&s.primary_accent_color);
        text.class_name = "text".into();
        text.style = format!(
            "text-anchor:start;font-size:{}px",
            geo::go_format_v(font_size)
        );
        text.content = escape_text(&column.name.label);
        output += &text.render();

        text.x = name_tl.x + longest_name_width as f64 + 2.0 * NAME_PADDING;
        text.fill.clone_from(&s.neutral_accent_color);
        text.content = escape_text(&column.type_text.label);
        output += &text.render();

        text.x = constraint_tr.x;
        text.y = constraint_tr.y + font_size * 3.0 / 4.0;
        text.fill.clone_from(&s.secondary_accent_color);
        text.style = format!(
            "text-anchor:end;font-size:{}px;letter-spacing:2px",
            geo::go_format_v(font_size)
        );
        text.set_text(&super::classtable::constraint_abbr(column));
        output += &text.render();

        row.top_left.y += row_height;

        let d = g.line(
            row.top_left.x,
            row.top_left.y,
            row.top_left.x + row.width,
            row.top_left.y,
            Some(&base_options(1.0)),
        );
        let paths = compute_rough_path_data(&g, &d);
        let mut line_el = ThemableElement::new("path", None);
        line_el.fill.clone_from(&s.fill);
        line_el.fill_pattern = s.fill_pattern_str().to_owned();
        render_paths(&mut line_el, &paths, &mut output);
    }

    let mut overlay = ThemableElement::new("rect", None);
    overlay.set_translate(x, y);
    overlay.width = s.width as f64;
    overlay.height = s.height as f64;
    output += &render_sketch_overlay(&mut overlay, &header_fill)?;
    Ok(output)
}

/// sketch.go `Class`.
pub(crate) fn class(s: &TargetShape) -> Result<String, String> {
    let mut output = String::new();
    let mut g = new_generator();
    let d = g.rectangle(
        0.0,
        0.0,
        s.width as f64,
        s.height as f64,
        Some(&base_options(s.stroke_width as f64)),
    );
    let paths = compute_rough_path_data(&g, &d);
    let (x, y) = (s.pos.x as f64, s.pos.y as f64);
    let mut path_el = ThemableElement::new("path", None);
    path_el.set_translate(x, y);
    (path_el.fill, path_el.stroke) = shape_theme(s);
    path_el.fill_pattern = s.fill_pattern_str().to_owned();
    path_el.class_name = "shape".into();
    path_el.style = s.css_style();
    render_paths(&mut path_el, &paths, &mut output);

    let b = geo::Box::new(Point::new(x, y), s.width as f64, s.height as f64);
    let fields = s.fields.as_deref().unwrap_or(&[]);
    let methods = s.methods.as_deref().unwrap_or(&[]);
    let row_height = b.height / (2 + fields.len() + methods.len()) as f64;
    let header = geo::Box::new(b.top_left, b.width, 2.0 * row_height);

    let d = g.rectangle(
        0.0,
        0.0,
        s.width as f64,
        header.height,
        Some(&base_options(1.0)),
    );
    let paths = compute_rough_path_data(&g, &d);
    let mut header_el = ThemableElement::new("path", None);
    header_el.set_translate(x, y);
    header_el.fill.clone_from(&s.fill);
    header_el.fill_pattern = s.fill_pattern_str().to_owned();
    header_el.class_name = "class_header".into();
    render_paths(&mut header_el, &paths, &mut output);
    let header_fill = if paths.is_empty() {
        header_el.fill.clone()
    } else {
        fill_after_render(&header_el.fill)
    };

    let mut overlay = ThemableElement::new("rect", None);
    overlay.set_translate(x, y);
    overlay.width = s.width as f64;
    overlay.height = header.height;
    output += &render_sketch_overlay(&mut overlay, &header_fill)?;

    if !s.label.is_empty() {
        let tl = Position::InsideMiddleCenter.get_point_on_box(
            &header,
            0.0,
            s.label_width as f64,
            s.label_height as f64,
        );
        let mut text = ThemableElement::new("text", None);
        text.x = tl.x + s.label_width as f64 / 2.0;
        text.y = tl.y + s.label_height as f64 * 3.0 / 4.0;
        text.fill = s.get_font_color();
        text.class_name = "text-mono".into();
        text.style = format!("text-anchor:middle;font-size:{}px", 4 + s.font_size);
        text.content = escape_text(&s.label);
        output += &text.render();
    }

    let font_size = s.font_size as f64;
    let mut row = geo::Box::new(b.top_left, b.width, row_height);
    row.top_left.y += header.height;
    for f in fields {
        output += &class_row(
            s,
            &row,
            super::classtable::visibility_token(&f.visibility),
            &f.name,
            &f.type_text,
            font_size,
        );
        row.top_left.y += row_height;
    }

    let d = g.line(
        row.top_left.x,
        row.top_left.y,
        row.top_left.x + row.width,
        row.top_left.y,
        Some(&base_options(1.0)),
    );
    let paths = compute_rough_path_data(&g, &d);
    let mut line_el = ThemableElement::new("path", None);
    line_el.fill.clone_from(&s.fill);
    line_el.fill_pattern = s.fill_pattern_str().to_owned();
    line_el.class_name = "class_header".into();
    render_paths(&mut line_el, &paths, &mut output);

    for m in methods {
        output += &class_row(
            s,
            &row,
            super::classtable::visibility_token(&m.visibility),
            &m.name,
            &m.return_type,
            font_size,
        );
        row.top_left.y += row_height;
    }
    Ok(output)
}

/// sketch.go `classRow`.
fn class_row(
    s: &TargetShape,
    b: &geo::Box,
    prefix: &str,
    name_text: &str,
    type_text: &str,
    font_size: f64,
) -> String {
    let prefix_tl =
        Position::InsideMiddleLeft.get_point_on_box(b, PREFIX_PADDING, b.width, font_size);
    let type_tr = Position::InsideMiddleRight.get_point_on_box(b, TYPE_PADDING, 0.0, font_size);
    let mut text = ThemableElement::new("text", None);
    text.x = prefix_tl.x;
    text.y = prefix_tl.y + font_size * 3.0 / 4.0;
    text.fill.clone_from(&s.primary_accent_color);
    text.class_name = "text-mono".into();
    text.style = format!(
        "text-anchor:start;font-size:{}px",
        geo::go_format_v(font_size)
    );
    text.content = prefix.to_owned();
    let mut output = text.render();

    text.x = prefix_tl.x + PREFIX_WIDTH;
    text.fill.clone_from(&s.fill);
    text.content = escape_text(name_text);
    output += &text.render();

    text.x = type_tr.x;
    text.y = type_tr.y + font_size * 3.0 / 4.0;
    text.fill.clone_from(&s.secondary_accent_color);
    text.style = format!(
        "text-anchor:end;font-size:{}px",
        geo::go_format_v(font_size)
    );
    text.content = escape_text(type_text);
    output += &text.render();
    output
}

fn arrow_options(stroke: &str, stroke_width: i64, seed: f64) -> Options {
    Options {
        stroke_width: Some(stroke_width as f64),
        stroke: Some(stroke.to_owned()),
        seed: Some(seed),
        ..Default::default()
    }
}

fn solid_arrow_options(
    stroke: &str,
    fill: &str,
    stroke_width: i64,
    fill_weight: f64,
    seed: f64,
) -> Options {
    let mut o = arrow_options(stroke, stroke_width, seed);
    o.fill = Some(fill.to_owned());
    o.fill_style = Some("solid".into());
    if fill_weight >= 0.0 {
        o.fill_weight = Some(fill_weight);
    }
    o
}

/// sketch.go `arrowheadDrawable`; seeds were chosen upstream per arrowhead.
fn arrowhead_drawable(
    g: &mut Generator,
    arrowhead: &str,
    stroke: &str,
    stroke_width: i64,
) -> Option<(Drawable, Option<Drawable>)> {
    const TRIANGLE: [RoughPoint; 3] = [[-10.0, -4.0], [0.0, 0.0], [-10.0, 4.0]];
    const DIAMOND: [RoughPoint; 5] = [
        [-20.0, 0.0],
        [-10.0, 5.0],
        [0.0, 0.0],
        [-10.0, -5.0],
        [-20.0, 0.0],
    ];
    const BOX: [RoughPoint; 4] = [[0.0, -10.0], [0.0, 10.0], [-20.0, 10.0], [-20.0, -10.0]];
    let sw = stroke_width;
    let mut extra = None;
    let arrow = match arrowhead {
        "arrow" => g.linear_path(&TRIANGLE, Some(&arrow_options(stroke, sw, 3.0))),
        "triangle" => g.polygon(
            &TRIANGLE,
            Some(&solid_arrow_options(stroke, stroke, sw, -1.0, 2.0)),
        ),
        "unfilled-triangle" => g.polygon(
            &TRIANGLE,
            Some(&solid_arrow_options(stroke, BG_COLOR, sw, -1.0, 2.0)),
        ),
        "diamond" => g.polygon(
            &DIAMOND,
            Some(&solid_arrow_options(stroke, BG_COLOR, sw, -1.0, 1.0)),
        ),
        "filled-diamond" => {
            let mut o = solid_arrow_options(stroke, stroke, sw, 4.0, 1.0);
            o.fill_style = Some("zigzag".into());
            g.polygon(&DIAMOND, Some(&o))
        }
        "cross" => g.linear_path(
            &[
                [-6.0, -6.0],
                [6.0, 6.0],
                [0.0, 0.0],
                [-6.0, 6.0],
                [0.0, 0.0],
                [6.0, -6.0],
            ],
            Some(&arrow_options(stroke, sw, 3.0)),
        ),
        "cf-many-required" => g.path(
            "M-15,-10 -15,10 M0,10 -15,0 M0,-10 -15,0",
            Some(&solid_arrow_options(stroke, stroke, sw, 4.0, 2.0)),
        ),
        "cf-many" => {
            let arrow = g.path(
                "M0,10 -15,0 M0,-10 -15,0",
                Some(&solid_arrow_options(stroke, stroke, sw, 4.0, 8.0)),
            );
            extra = Some(g.circle(
                -20.0,
                0.0,
                8.0,
                Some(&solid_arrow_options(stroke, BG_COLOR, sw, 1.0, 4.0)),
            ));
            arrow
        }
        "cf-one-required" => g.path(
            "M-15,-10 -15,10 M-10,-10 -10,10",
            Some(&solid_arrow_options(stroke, stroke, sw, 4.0, 2.0)),
        ),
        "cf-one" => {
            let arrow = g.path(
                "M-10,-10 -10,10",
                Some(&solid_arrow_options(stroke, stroke, sw, 4.0, 3.0)),
            );
            extra = Some(g.circle(
                -20.0,
                0.0,
                8.0,
                Some(&solid_arrow_options(stroke, BG_COLOR, sw, 1.0, 5.0)),
            ));
            arrow
        }
        "circle" => g.circle(
            -2.0,
            -1.0,
            8.0,
            Some(&solid_arrow_options(stroke, BG_COLOR, sw, 1.0, 5.0)),
        ),
        "box" => g.polygon(
            &BOX,
            Some(&solid_arrow_options(stroke, BG_COLOR, sw, -1.0, 1.0)),
        ),
        "filled-box" => g.polygon(
            &BOX,
            Some(&solid_arrow_options(stroke, stroke, sw, -1.0, 1.0)),
        ),
        _ => return None,
    };
    Some((arrow, extra))
}

/// One end of sketch.go `Arrowheads`; `None` is the source's early `"", nil`.
fn arrowhead_paths(
    c: &TargetConnection,
    arrowhead: &str,
    at: Point,
    angle: f64,
    out: &mut Vec<String>,
) -> Option<()> {
    let mut g = new_generator();
    let (arrow, extra) = arrowhead_drawable(&mut g, arrowhead, &c.stroke, c.stroke_width)?;
    let transform = format!(
        r#"transform="translate({:.6} {:.6}) rotate({})""#,
        at.x,
        at.y,
        geo::go_format_v(angle)
    );
    let mut rough_paths = compute_rough_paths(&g, &arrow);
    if let Some(extra) = extra {
        rough_paths.extend(compute_rough_paths(&g, &extra));
    }
    let mut el = ThemableElement::new("path", None);
    el.class_name = "connection".into();
    el.attributes = transform;
    for rp in &rough_paths {
        el.d.clone_from(&rp.d);
        el.fill.clone_from(&rp.fill);
        el.stroke.clone_from(&rp.stroke);
        el.style = rp.style_css();
        out.push(el.render());
    }
    Some(())
}

/// sketch.go `Arrowheads`.
pub(crate) fn arrowheads(
    c: &TargetConnection,
    route: &[Point],
    src_adj: Point,
    dst_adj: Point,
) -> String {
    let mut arrow_paths = Vec::new();
    if c.src_arrow != "none" {
        let starting = geo::Segment::new(route[0], route[1]);
        let angle = starting.to_vector().reverse().degrees();
        let at = Point::new(starting.start.x + src_adj.x, starting.start.y + src_adj.y);
        if arrowhead_paths(c, &c.src_arrow, at, angle, &mut arrow_paths).is_none() {
            return String::new();
        }
    }
    if c.dst_arrow != "none" {
        let length = route.len();
        let ending = geo::Segment::new(route[length - 2], route[length - 1]);
        let angle = ending.to_vector().degrees();
        let at = Point::new(ending.end.x + dst_adj.x, ending.end.y + dst_adj.y);
        if arrowhead_paths(c, &c.dst_arrow, at, angle, &mut arrow_paths).is_none() {
            return String::new();
        }
    }
    arrow_paths.join(" ")
}

#[cfg(test)]
mod tests {
    use super::truncate_path_data;

    #[test]
    fn truncates_decimals_like_sketch_float_re() {
        assert_eq!(
            truncate_path_data("M0.8572634477168322 -1.5 C1.234e-7, 12.3456789"),
            "M0.857263 -1.5 C1.234e-7, 12.345678"
        );
        assert_eq!(truncate_path_data("M1. 2.x .5"), "M1. 2.x .5");
    }

    #[test]
    fn rough_rect_matches_upstream_sketch_golden() {
        // e2etests/testdata/txtar/sketch-cross-arrowhead/dagre/sketch.exp.svg,
        // shape "start" (86x66, stroke-width 2): solid fill, then outline.
        let mut g = super::new_generator();
        let d = g.rectangle(0.0, 0.0, 86.0, 66.0, Some(&super::base_options(2.0)));
        let paths = super::compute_rough_path_data(&g, &d);
        assert_eq!(paths.len(), 2);
        assert_eq!(
            paths[0],
            "M-1.600310 -0.578379 L87.045551 1.811030 L86.253697 64.234072 L0.925556 67.532483"
        );
        assert_eq!(
            paths[1],
            "M0.857263 0.963884 C16.880632 0.814602, 33.840649 -2.078734, 85.206405 0.392335 M-0.648665 0.264598 C19.475715 0.071148, 38.158917 -0.498524, 85.419625 0.752815 M87.536704 -1.749433 C86.489431 15.585410, 87.180967 27.069513, 87.390547 65.130645 M86.297677 -0.799274 C86.657560 16.854002, 85.681091 35.455552, 86.406876 66.352243 M87.052801 65.786559 C68.482595 67.508497, 47.220740 66.964972, 1.836456 65.596476 M85.056573 65.856267 C53.583644 66.460688, 22.138958 66.189224, 0.938949 66.041844 M-0.720604 65.718532 C0.302797 45.542204, -1.429636 28.321166, 0.591800 -1.206080 M0.217956 66.998223 C-1.587850 41.337487, -1.081795 17.082362, 0.440740 0.988030"
        );
    }
}
