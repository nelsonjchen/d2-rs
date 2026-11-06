// SPDX-License-Identifier: MPL-2.0
// Port of D2 d2renderers/d2svg/d2svg.go: Render, RenderMultiboard,
// drawShape, drawConnection, arrowhead markers, path data, label masks and
// text.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use base64::Engine as _;

use super::color;
use super::element::{ThemableElement, Theme};
use super::geo::{self, Point, Vector};
use super::label::{self, Position};
use super::shape;
use super::svgpath;
use super::svgtext::{escape_text, format_float, scoped_id, svg_id};
use super::target_methods::*;
use super::theme_css;
use crate::target::{TargetConnection, TargetDiagram, TargetShape, TargetThemeOverrides};

pub(crate) const DEFAULT_PADDING: i64 = 100;
pub(crate) const BASE_STYLESHEET: &str = include_str!("assets/style.css");
const DOTS: &str = include_str!("assets/dots.txt");
const LINES: &str = include_str!("assets/lines.txt");
/// d2ast.FillPatterns, in declaration order.
const FILL_PATTERNS: [&str; 4] = ["dots", "lines", "grain", "paper"];

/// d2svg.RenderOpts.
#[derive(Clone, Debug, Default)]
pub struct RenderOpts {
    pub pad: Option<i64>,
    pub sketch: Option<bool>,
    /// Mode dispatch belongs to d2-rs-isometric::svg, above the flat renderer
    /// in the dependency graph. The flat-only entry points do not dispatch.
    pub isometric: Option<bool>,
    pub center: Option<bool>,
    pub theme_id: Option<i64>,
    pub dark_theme_id: Option<i64>,
    pub theme_overrides: Option<TargetThemeOverrides>,
    pub dark_theme_overrides: Option<TargetThemeOverrides>,
    pub scale: Option<f64>,
    pub master_id: String,
    /// Diagram.Bytes to hash instead of the target's own (see
    /// RenderOptions::d2svg_hash_input).
    pub hash_input_override: Option<Vec<u8>>,
    pub no_xml_tag: Option<bool>,
    pub salt: Option<String>,
    pub omit_version: Option<bool>,
    pub custom_fonts: d2_rs_text::d2fonts::CustomFonts,
}

pub(crate) const VERSION: &str = "v0.9.0";

/// d2svg.RenderMultiboard.
pub(crate) fn render_multiboard(
    diagram: &TargetDiagram,
    opts: &RenderOpts,
) -> Result<Vec<Vec<u8>>, String> {
    super::target_validation::validate(diagram)?;
    render_multiboard_validated(diagram, opts, &mut Ok)
}

pub(crate) fn render_multiboard_with(
    diagram: &TargetDiagram,
    opts: &RenderOpts,
    process: &mut impl FnMut(Vec<u8>) -> Result<Vec<u8>, String>,
) -> Result<Vec<Vec<u8>>, String> {
    super::target_validation::validate(diagram)?;
    render_multiboard_validated(diagram, opts, process)
}

fn render_multiboard_validated(
    diagram: &TargetDiagram,
    opts: &RenderOpts,
    process: &mut impl FnMut(Vec<u8>) -> Result<Vec<u8>, String>,
) -> Result<Vec<Vec<u8>>, String> {
    let mut boards = Vec::new();
    for child in diagram
        .layers
        .iter()
        .flatten()
        .chain(diagram.scenarios.iter().flatten())
        .chain(diagram.steps.iter().flatten())
        .flatten()
    {
        boards.extend(render_multiboard_validated(child, opts, process)?);
    }
    if !diagram.is_folder_only {
        super::paint_validation::links(diagram)?;
        boards.insert(0, process(render_validated(diagram, opts)?)?);
    }
    Ok(boards)
}

fn invalid_padding_error(pad: i64) -> String {
    format!("padding {pad} produces invalid SVG dimensions")
}

/// d2svg.validatePadding (Go int is 64-bit on the pinned platforms).
fn validate_padding(tl: (i64, i64), br: (i64, i64), pad: i64) -> Result<i64, String> {
    let double_pad = pad
        .checked_add(pad)
        .ok_or_else(|| invalid_padding_error(pad))?;
    for (low, high) in [(tl.0, br.0), (tl.1, br.1)] {
        low.checked_sub(pad)
            .ok_or_else(|| invalid_padding_error(pad))?;
        let size = high
            .checked_sub(low)
            .ok_or_else(|| invalid_padding_error(pad))?;
        size.checked_add(double_pad)
            .ok_or_else(|| invalid_padding_error(pad))?;
    }
    Ok(pad)
}

/// d2svg.expandDimensions.
fn expand_dimensions(
    left: i64,
    top: i64,
    width: i64,
    height: i64,
    amount: i64,
) -> Result<(i64, i64, i64, i64), ()> {
    let double = amount.checked_add(amount).ok_or(())?;
    Ok((
        left.checked_sub(amount).ok_or(())?,
        top.checked_sub(amount).ok_or(())?,
        width.checked_add(double).ok_or(())?,
        height.checked_add(double).ok_or(())?,
    ))
}

/// d2svg.dimensions.
fn dimensions(
    diagram: &TargetDiagram,
    pad: i64,
    tl: (i64, i64),
    br: (i64, i64),
) -> Result<(i64, i64, i64, i64), String> {
    let mut dims = (
        tl.0 - pad,
        tl.1 - pad,
        br.0 - tl.0 + pad * 2,
        br.1 - tl.1 + pad * 2,
    );
    super::legend::expand_dimensions_for_legend(diagram, pad, tl, br, &mut dims, false)?;
    Ok(dims)
}

enum Object<'a> {
    Shape(&'a TargetShape),
    Connection(&'a TargetConnection),
}

/// d2svg.sortObjects (sort.SliceStable).
fn sort_objects(objects: &mut [Object<'_>]) {
    objects.sort_by(|a, b| {
        let z = |o: &Object<'_>| match o {
            Object::Shape(s) => s.z_index,
            Object::Connection(c) => c.z_index,
        };
        let (za, zb) = (z(a), z(b));
        if za != zb {
            return za.cmp(&zb);
        }
        match (a, b) {
            (Object::Shape(sa), Object::Shape(sb)) => sa.level.cmp(&sb.level),
            (Object::Shape(_), Object::Connection(_)) => std::cmp::Ordering::Less,
            (Object::Connection(_), Object::Shape(_)) => std::cmp::Ordering::Greater,
            (Object::Connection(_), Object::Connection(_)) => std::cmp::Ordering::Equal,
        }
    });
}

/// d2svg.hash: FNV-1a 32 of s + "lalalas", in decimal.
pub(crate) fn hash(s: &str) -> String {
    let mut h = 2_166_136_261u32;
    for byte in s.bytes().chain("lalalas".bytes()) {
        h = (h ^ u32::from(byte)).wrapping_mul(16_777_619);
    }
    h.to_string()
}

/// d2svg.Render.
pub(crate) fn render(diagram: &TargetDiagram, opts: &RenderOpts) -> Result<Vec<u8>, String> {
    super::paint_validation::links(diagram)?;
    super::target_validation::validate(diagram)?;
    render_validated(diagram, opts)
}

pub(crate) fn render_validated(
    diagram: &TargetDiagram,
    opts: &RenderOpts,
) -> Result<Vec<u8>, String> {
    super::paint_validation::validate(diagram, opts)?;
    let (tl, br) = diagram.go_bounding_box()?;
    let pad = match opts.pad {
        Some(pad) => validate_padding(tl, br, pad)?,
        None => DEFAULT_PADDING,
    };
    let sketch = opts.sketch == Some(true);
    let theme_id = opts.theme_id.unwrap_or(0);
    let dark_theme_id = opts.dark_theme_id;
    let scale = opts.scale;

    let mut buf = String::new();
    let shapes = diagram.shapes_slice();
    let connections = diagram.connections_slice();
    if shapes.iter().any(|s| s.shadow) {
        define_shadow_filter(&mut buf);
    }
    let root = &diagram.root;
    // d2svg.defineGradients, in Render's order.
    let mut define = |value: &str| {
        if color::is_gradient(value) {
            let svg = color::parse_gradient(value)
                .map(|g| color::gradient_to_svg(&g))
                .unwrap_or_default();
            write!(buf, "<defs>{svg}</defs>").unwrap();
        }
    };
    define(&root.fill);
    define(&root.stroke);
    for s in shapes {
        define(&s.fill);
        define(&s.stroke);
        define(&s.color);
    }
    for c in connections {
        define(&c.stroke);
        define(c.fill_str());
    }

    let mut diagram_hash = match &opts.hash_input_override {
        Some(bytes) => crate::target_hash_id_from_input(bytes, opts.salt.as_deref()),
        None => diagram.hash_id(opts.salt.as_deref())?,
    };
    let isolated_diagram_hash = diagram_hash.clone();
    if !opts.master_id.is_empty() {
        diagram_hash.clone_from(&opts.master_id);
    }

    let mut objects: Vec<Object<'_>> = shapes.iter().map(Object::Shape).collect();
    objects.extend(connections.iter().map(Object::Connection));
    sort_objects(&mut objects);
    let id_to_shape: std::collections::HashMap<&str, &TargetShape> =
        shapes.iter().map(|s| (s.id.as_str(), s)).collect();

    let mut appendix_buf = String::new();
    let mut label_masks = Vec::new();
    let mut markers = BTreeSet::new();
    let inline_theme_owned = dark_theme_id.is_none().then(|| {
        let mut theme = Theme::find(theme_id);
        theme.apply_overrides(opts.theme_overrides.as_ref());
        theme
    });
    let inline_theme = inline_theme_owned.as_ref();
    let mut markdown = super::markdown::MarkdownRenderer::new(
        diagram.font_family.as_deref(),
        diagram.mono_font_family.as_deref(),
        inline_theme,
    );
    markdown.set_custom_fonts(&opts.custom_fonts);
    for object in &objects {
        let label_mask = match object {
            Object::Connection(c) => draw_connection(
                &mut buf,
                &isolated_diagram_hash,
                c,
                &mut markers,
                &id_to_shape,
                sketch,
                inline_theme,
                &mut markdown,
            )?,
            Object::Shape(s) => draw_shape(
                &mut buf,
                &mut appendix_buf,
                &diagram_hash,
                s,
                sketch,
                inline_theme,
                &mut markdown,
            )?,
        };
        if !label_mask.is_empty() {
            label_masks.push(label_mask);
        }
    }
    buf += &appendix_buf;
    if super::legend::has_items(diagram) {
        let mut legend_buf = String::new();
        super::legend::render_legend(&mut legend_buf, diagram, &diagram_hash, inline_theme)?;
        buf += &legend_buf;
    }

    let (left, top, w, h) = {
        let mut dims = dimensions(diagram, pad, tl, br)?;
        super::legend::expand_dimensions_for_legend(diagram, pad, tl, br, &mut dims, true)?;
        dims
    };
    if w < 0 || h < 0 {
        return Err(invalid_padding_error(pad));
    }
    buf += &[
        format!(r#"<mask id="{isolated_diagram_hash}" maskUnits="userSpaceOnUse" x="{left}" y="{top}" width="{w}" height="{h}">"#),
        format!(r#"<rect x="{left}" y="{top}" width="{w}" height="{h}" fill="white"></rect>"#),
        label_masks.join("\n"),
        "</mask>".to_owned(),
    ]
    .join("\n");

    let mut theme_stylesheet = String::new();
    if opts.master_id.is_empty() {
        theme_stylesheet = theme_css::theme_css(
            &diagram_hash,
            Some(theme_id),
            dark_theme_id,
            opts.theme_overrides.as_ref(),
            opts.dark_theme_overrides.as_ref(),
        )?;
    }

    let stroke_padding = (root.stroke_width as f64 / 2.0).ceil() as i64;
    let invalid = |_| invalid_padding_error(pad);
    let (mut left, mut top, mut w, mut h) =
        expand_dimensions(left, top, w, h, stroke_padding).map_err(invalid)?;
    let mut background = ThemableElement::new("rect", inline_theme);
    background.x = left as f64;
    background.y = top as f64;
    background.width = w as f64;
    background.height = h as f64;
    background.fill.clone_from(&root.fill);
    background.stroke.clone_from(&root.stroke);
    background.fill_pattern = root.fill_pattern_str().to_owned();
    background.rx = root.border_radius as f64;
    if root.stroke_dash.0 != 0.0 {
        let (dash, gap) =
            svgpath::get_stroke_dash_attributes(root.stroke_width as f64, root.stroke_dash.0);
        background.stroke_dash_array = format!("{}, {}", format_float(dash), format_float(gap));
    }
    background.attributes = format!(r#"stroke-width="{}""#, root.stroke_width);
    (left, top, w, h) = expand_dimensions(left, top, w, h, stroke_padding).map_err(invalid)?;

    let mut double_border = String::new();
    if root.double_border {
        (left, top, w, h) =
            expand_dimensions(left, top, w, h, stroke_padding + INNER_BORDER_OFFSET)
                .map_err(invalid)?;
        let mut outer = background.clone();
        background.fill = "transparent".into();
        outer.x = left as f64;
        outer.y = top as f64;
        outer.width = w as f64;
        outer.height = h as f64;
        double_border = outer.render();
        (left, top, w, h) = expand_dimensions(left, top, w, h, stroke_padding).map_err(invalid)?;
    }
    let background_str = background.render();

    let mut upper = String::new();
    if opts.master_id.is_empty() {
        let (corpora, classes) = super::fonts::collect_font_corpora_and_classes(&buf);
        let mut corpora = corpora;
        if shapes
            .iter()
            .any(|s| !s.tooltip.is_empty() || !s.link.is_empty() || !s.pretty_link_str().is_empty())
        {
            corpora = None;
        }
        super::fonts::embed_fonts(
            &mut upper,
            &diagram_hash,
            &buf,
            diagram.font_family.as_deref(),
            diagram.mono_font_family.as_deref(),
            &(diagram.get_corpus()? + &markdown.generated_corpus()),
            corpora.as_ref(),
            &opts.custom_fonts,
        )?;
        theme_stylesheet = match classes {
            None => theme_css::prune_theme_css(
                &theme_stylesheet,
                &[&buf, &background_str, &double_border],
            ),
            Some(classes) => theme_css::prune_theme_css_with_classes(
                &theme_stylesheet,
                Some(classes),
                &[&background_str, &double_border],
            ),
        };
        write!(
            upper,
            r#"<style type="text/css"><![CDATA[{BASE_STYLESHEET}{theme_stylesheet}]]></style>"#
        )
        .unwrap();
        if sketch {
            super::sketch::define_fill_patterns(&mut upper, &diagram_hash);
        }
    }

    let mut pattern_defs = String::new();
    for pattern in FILL_PATTERNS {
        if buf.contains(&format!("{pattern}-overlay")) || root.fill_pattern_str() == pattern {
            if pattern_defs.is_empty() {
                upper += r#"<style type="text/css"><![CDATA["#;
            }
            pattern_defs += &match pattern {
                "dots" => DOTS.replace("%s", &diagram_hash),
                "lines" => LINES.replace("%s", &diagram_hash),
                "grain" => {
                    crate::patternassets::grain_svg_template().replace("%[1]s", &diagram_hash)
                }
                _ => crate::patternassets::paper_svg_template().replace("%[1]s", &diagram_hash),
            };
            write!(
                upper,
                "\n.{pattern}-overlay {{\n\tfill: url(#{pattern}-{diagram_hash});\n\tmix-blend-mode: multiply;\n}}"
            )
            .unwrap();
        }
    }
    if !pattern_defs.is_empty() {
        upper += "]]></style><defs>";
        upper += &pattern_defs;
        upper += "</defs>";
    }

    let dimensions = scale
        .map(|scale| {
            format!(
                r#" width="{}" height="{}""#,
                (scale * w as f64).ceil() as i64,
                (scale * h as f64).ceil() as i64
            )
        })
        .unwrap_or_default();
    let alignment = if opts.center == Some(true) {
        "xMidYMid"
    } else {
        "xMinYMin"
    };
    let (mut opening, mut xml_tag, mut closing, mut id_attr, mut tag) =
        (String::new(), "", "", "", "g");
    if opts.master_id.is_empty() {
        let version = if opts.omit_version == Some(true) {
            String::new()
        } else {
            format!(r#"data-d2-version="{VERSION}""#)
        };
        opening = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" {version} preserveAspectRatio="{alignment} meet" viewBox="0 0 {w} {h}"{dimensions}>"#
        );
        if opts.no_xml_tag != Some(true) {
            xml_tag = r#"<?xml version="1.0" encoding="utf-8"?>"#;
        }
        closing = "</svg>";
        id_attr = "d2-svg";
        tag = "svg";
    }
    Ok(format!(
        r#"{xml_tag}{opening}<{tag} class="{diagram_hash} {id_attr}" width="{w}" height="{h}" viewBox="{left} {top} {w} {h}">{double_border}{background_str}{upper}{buf}</{tag}>{closing}"#
    )
    .into_bytes())
}

/// d2svg.defineShadowFilter.
fn define_shadow_filter(buf: &mut String) {
    buf.push_str(
        "<defs>\n\t<filter id=\"shadow-filter\" width=\"200%\" height=\"200%\" x=\"-50%\" y=\"-50%\">\n\t\t<feGaussianBlur stdDeviation=\"1.7 \" in=\"SourceGraphic\"></feGaussianBlur>\n\t\t<feFlood flood-color=\"#3d4574\" flood-opacity=\"0.4\" result=\"ShadowFeFlood\" in=\"SourceGraphic\"></feFlood>\n\t\t<feComposite in=\"ShadowFeFlood\" in2=\"SourceAlpha\" operator=\"in\" result=\"ShadowFeComposite\"></feComposite>\n\t\t<feOffset dx=\"3\" dy=\"5\" result=\"ShadowFeOffset\" in=\"ShadowFeComposite\"></feOffset>\n\t\t<feBlend in=\"SourceGraphic\" in2=\"ShadowFeOffset\" mode=\"normal\" result=\"ShadowFeBlend\"></feBlend>\n\t</filter>\n</defs>",
    );
}

/// d2svg.arrowheadMarkerID.
fn arrowhead_marker_id(diagram_hash: &str, is_target: bool, c: &TargetConnection) -> String {
    let arrowhead = if is_target {
        &c.dst_arrow
    } else {
        &c.src_arrow
    };
    format!(
        "mk-{diagram_hash}-{}",
        hash(&format!(
            "{arrowhead},{is_target},{},{}",
            c.stroke_width, c.stroke
        ))
    )
}

fn points(values: &[f64]) -> String {
    values
        .chunks(2)
        .map(|pair| format!("{},{}", format_float(pair[0]), format_float(pair[1])))
        .collect::<Vec<_>>()
        .join(" ")
}

/// d2svg.arrowheadMarker.
fn arrowhead_marker(
    is_target: bool,
    id: &str,
    c: &TargetConnection,
    theme: Option<&Theme>,
) -> String {
    let arrowhead = if is_target {
        c.dst_arrow.as_str()
    } else {
        c.src_arrow.as_str()
    };
    let stroke_width = c.stroke_width as f64;
    let (mut width, height) = arrowhead_dimensions(arrowhead, stroke_width);
    let stroke_attr = format!(r#"stroke-width="{}""#, c.stroke_width);
    let path = match arrowhead {
        "arrow" => {
            let mut el = ThemableElement::new("polygon", theme);
            el.fill.clone_from(&c.stroke);
            el.class_name = "connection".into();
            el.attributes.clone_from(&stroke_attr);
            el.points = if is_target {
                points(&[
                    0.,
                    0.,
                    width,
                    height / 2.,
                    0.,
                    height,
                    width / 4.,
                    height / 2.,
                ])
            } else {
                points(&[
                    0.,
                    height / 2.,
                    width,
                    0.,
                    width * 3. / 4.,
                    height / 2.,
                    width,
                    height,
                ])
            };
            el.render()
        }
        "unfilled-triangle" => {
            let mut el = ThemableElement::new("polygon", theme);
            el.fill = BG_COLOR.into();
            el.stroke.clone_from(&c.stroke);
            el.class_name = "connection".into();
            el.attributes.clone_from(&stroke_attr);
            let inset = stroke_width / 2.;
            el.points = if is_target {
                points(&[
                    inset,
                    inset,
                    width - inset,
                    height / 2.0,
                    inset,
                    height - inset,
                ])
            } else {
                points(&[
                    width - inset,
                    inset,
                    inset,
                    height / 2.0,
                    width - inset,
                    height - inset,
                ])
            };
            el.render()
        }
        "triangle" => {
            let mut el = ThemableElement::new("polygon", theme);
            el.fill.clone_from(&c.stroke);
            el.class_name = "connection".into();
            el.attributes.clone_from(&stroke_attr);
            el.points = if is_target {
                points(&[0., 0., width, height / 2.0, 0., height])
            } else {
                points(&[width, 0., 0., height / 2.0, width, height])
            };
            el.render()
        }
        "line" => {
            let mut el = ThemableElement::new("polyline", theme);
            el.fill = "none".into();
            el.class_name = "connection".into();
            el.stroke.clone_from(&c.stroke);
            el.attributes.clone_from(&stroke_attr);
            let s = stroke_width / 2.;
            el.points = if is_target {
                points(&[s, s, width - s, height / 2., s, height - s])
            } else {
                points(&[width - s, s, s, height / 2., width - s, height - s])
            };
            el.render()
        }
        "filled-diamond" => {
            let mut el = ThemableElement::new("polygon", theme);
            el.class_name = "connection".into();
            el.fill.clone_from(&c.stroke);
            el.attributes.clone_from(&stroke_attr);
            el.points = points(&[
                0.,
                height / 2.0,
                width / 2.0,
                0.,
                width,
                height / 2.0,
                width / 2.0,
                height,
            ]);
            el.render()
        }
        "diamond" => {
            let mut el = ThemableElement::new("polygon", theme);
            el.class_name = "connection".into();
            el.fill = BG_COLOR.into();
            el.stroke.clone_from(&c.stroke);
            el.attributes.clone_from(&stroke_attr);
            el.points = if is_target {
                points(&[
                    0.,
                    height / 2.0,
                    width / 2.,
                    height / 8.,
                    width,
                    height / 2.0,
                    width / 2.0,
                    height * 0.9,
                ])
            } else {
                points(&[
                    width / 8.,
                    height / 2.0,
                    width * 0.6,
                    height / 8.,
                    width * 1.1,
                    height / 2.0,
                    width * 0.6,
                    height * 7. / 8.,
                ])
            };
            el.render()
        }
        "filled-circle" | "circle" => {
            let radius = width / 2.;
            let mut el = ThemableElement::new("circle", theme);
            el.cy = radius;
            if arrowhead == "filled-circle" {
                el.r = radius - stroke_width / 2.;
                el.fill.clone_from(&c.stroke);
                el.class_name = "connection".into();
            } else {
                el.r = radius - stroke_width;
                el.fill = BG_COLOR.into();
                el.stroke.clone_from(&c.stroke);
            }
            el.attributes.clone_from(&stroke_attr);
            el.cx = if is_target {
                radius + stroke_width / 2.
            } else {
                radius - stroke_width / 2.
            };
            el.render()
        }
        "cross" => {
            let inset = stroke_width / 8.;
            let angle = std::f64::consts::PI / 4.;
            let origin = Point::new(width / 2., height / 2.);
            // Go ARM64: cos*x - sin*y and sin*x + cos*y fuse the second product.
            let (sin, cos) = (geo::gomath::sin(angle), geo::gomath::cos(angle));
            let new_origin = Point::new(
                (-sin).mul_add(origin.y, cos * origin.x),
                cos.mul_add(origin.y, sin * origin.x),
            );
            let mut cross = ThemableElement::new("polygon", theme);
            let p = |x: f64, y: f64| format!("{},{}", format_float(x), format_float(y));
            cross.points = format!(
                "{} {} {} {}, {} {} {} {} {} {} {} {}",
                p(0., height / 2. + inset),
                p(width / 2. - inset, height / 2. + inset),
                p(width / 2. - inset, height),
                p(width / 2. + inset, height),
                p(width / 2. + inset, height / 2. + inset),
                p(width, height / 2. + inset),
                p(width, height / 2. - inset),
                p(width / 2. + inset, height / 2. - inset),
                p(width / 2. + inset, 0.),
                p(width / 2. - inset, 0.),
                p(width / 2. - inset, height / 2. - inset),
                p(0., height / 2. - inset),
            );
            cross.transform = format!(
                "translate({}, {}) rotate(45)",
                format_float(-new_origin.x + width / 2.),
                format_float(-new_origin.y + height / 2.)
            );
            let mut child = ThemableElement::new("path", theme);
            child.d = if is_target {
                format!("M{} {}", p(width / 2., height / 2.), p(width, height / 2.))
            } else {
                format!("M{} {}", p(width / 2., height / 2.), p(0., height / 2.))
            };
            let mut g = ThemableElement::new("g", theme);
            g.fill = BG_COLOR.into();
            g.stroke.clone_from(&c.stroke);
            g.class_name = "connection".into();
            g.attributes.clone_from(&stroke_attr);
            g.content = format!("{}{}", cross.render(), child.render());
            g.render()
        }
        "filled-box" => {
            let mut el = ThemableElement::new("polygon", theme);
            el.class_name = "connection".into();
            el.fill.clone_from(&c.stroke);
            el.attributes.clone_from(&stroke_attr);
            el.points = points(&[0., 0., 0., height, width, height, width, 0.]);
            el.render()
        }
        "box" => {
            let mut el = ThemableElement::new("polygon", theme);
            el.class_name = "connection".into();
            el.fill = BG_COLOR.into();
            el.stroke.clone_from(&c.stroke);
            el.attributes.clone_from(&stroke_attr);
            el.style = format!("{}stroke-linejoin:miter;", el.style);
            let inset = stroke_width / 2.;
            el.points = points(&[
                inset,
                inset,
                inset,
                height - inset,
                width - inset,
                height - inset,
                width - inset,
                inset,
            ]);
            el.render()
        }
        "cf-one" | "cf-many" | "cf-one-required" | "cf-many-required" => {
            // Go ARM64 fuses 3.0 + sw*1.8.
            let offset = (c.stroke_width as f64).mul_add(1.8, 3.0);
            let p = |x: f64, y: f64| format!("{},{}", format_float(x), format_float(y));
            let mut modifier;
            if arrowhead == "cf-one-required" || arrowhead == "cf-many-required" {
                modifier = ThemableElement::new("path", theme);
                modifier.d = format!("M{} {}", p(offset, 0.), p(offset, height));
            } else {
                modifier = ThemableElement::new("circle", theme);
                modifier.cx = offset / 2.0 + 2.0;
                modifier.cy = height / 2.0;
                modifier.r = offset / 2.0;
            }
            modifier.fill = BG_COLOR.into();
            modifier.stroke.clone_from(&c.stroke);
            modifier.class_name = "connection".into();
            modifier.attributes.clone_from(&stroke_attr);
            let mut child = ThemableElement::new("path", theme);
            child.d = if arrowhead == "cf-many" || arrowhead == "cf-many-required" {
                format!(
                    "M{} {} M{} {} M{} {}",
                    p(width - 3.0, height / 2.0),
                    p(width + offset, height / 2.0),
                    p(offset + 3.0, height / 2.0),
                    p(width + offset, 0.),
                    p(offset + 3.0, height / 2.0),
                    p(width + offset, height)
                )
            } else {
                format!(
                    "M{} {} M{} {}",
                    p(width - 3.0, height / 2.0),
                    p(width + offset, height / 2.0),
                    p(offset * 2.0, 0.),
                    p(offset * 2.0, height)
                )
            };
            let mut g = ThemableElement::new("g", theme);
            if !is_target {
                g.transform = format!(
                    "scale(-1) translate(-{}, -{})",
                    format_float(width),
                    format_float(height)
                );
            }
            g.fill = BG_COLOR.into();
            g.stroke.clone_from(&c.stroke);
            g.class_name = "connection".into();
            g.attributes.clone_from(&stroke_attr);
            g.content = format!("{}{}", modifier.render(), child.render());
            g.render()
        }
        _ => return String::new(),
    };
    let ref_y = height / 2.;
    let ref_x;
    if arrowhead == "diamond" {
        // Go ARM64 fuses width - 0.6*sw and width/8 + 0.6*sw.
        ref_x = if is_target {
            (-0.6f64).mul_add(stroke_width, width)
        } else {
            0.6f64.mul_add(stroke_width, width / 8.)
        };
        width *= 1.1;
    } else {
        ref_x = if is_target {
            (-1.5f64).mul_add(stroke_width, width)
        } else {
            1.5 * stroke_width
        };
    }
    [
        format!(
            r#"<marker id="{id}" markerWidth="{}" markerHeight="{}" refX="{}" refY="{}""#,
            format_float(width),
            format_float(height),
            format_float(ref_x),
            format_float(ref_y)
        ),
        format!(
            r#"viewBox="0 0 {} {}""#,
            format_float(width),
            format_float(height)
        ),
        r#"orient="auto" markerUnits="userSpaceOnUse">"#.to_owned(),
        path,
        "</marker>".to_owned(),
    ]
    .join(" ")
}

/// d2svg.arrowheadAdjustment.
fn arrowhead_adjustment(
    start: Point,
    end: Point,
    arrowhead: &str,
    edge_stroke_width: i64,
    shape_stroke_width: i64,
) -> Point {
    let mut distance = (edge_stroke_width as f64 + shape_stroke_width as f64) / 2.0;
    if arrowhead != "none" {
        distance += edge_stroke_width as f64;
    }
    Vector::new(&[end.x - start.x, end.y - start.y])
        .unit()
        .multiply(-distance)
        .to_point()
}

/// d2svg.getArrowheadAdjustments.
fn get_arrowhead_adjustments(
    c: &TargetConnection,
    route: &[Point],
    id_to_shape: &std::collections::HashMap<&str, &TargetShape>,
) -> Result<(Point, Point), String> {
    if route.len() < 2 {
        return Err(crate::RenderError::SourceRouteIndex {
            index: 1,
            length: route.len(),
        }
        .to_string());
    }
    let src_width = id_to_shape
        .get(c.src.as_str())
        .map_or(0, |s| s.stroke_width);
    let dst_width = id_to_shape
        .get(c.dst.as_str())
        .map_or(0, |s| s.stroke_width);
    Ok((
        arrowhead_adjustment(route[1], route[0], &c.src_arrow, c.stroke_width, src_width),
        arrowhead_adjustment(
            route[route.len() - 2],
            route[route.len() - 1],
            &c.dst_arrow,
            c.stroke_width,
            dst_width,
        ),
    ))
}

/// d2svg.pathData.
fn path_data(c: &TargetConnection, route: &[Point], src_adj: Point, dst_adj: Point) -> String {
    let f = format_float;
    let mut path = vec![format!(
        "M {} {}",
        f(route[0].x + src_adj.x),
        f(route[0].y + src_adj.y)
    )];
    if c.is_curve {
        let mut i = 1;
        while i + 3 < route.len() {
            path.push(format!(
                "C {} {} {} {} {} {}",
                f(route[i].x),
                f(route[i].y),
                f(route[i + 1].x),
                f(route[i + 1].y),
                f(route[i + 2].x),
                f(route[i + 2].y)
            ));
            i += 3;
        }
        path.push(format!(
            "C {} {} {} {} {} {}",
            f(route[i].x),
            f(route[i].y),
            f(route[i + 1].x),
            f(route[i + 1].y),
            f(route[i + 2].x + dst_adj.x),
            f(route[i + 2].y + dst_adj.y)
        ));
    } else {
        let mut i = 1;
        while i + 1 < route.len() {
            let prev_source = route[i - 1];
            let prev_target = route[i];
            let curr_target = route[i + 1];
            let prev_vector = prev_source.vector_to(&prev_target);
            let curr_vector = prev_target.vector_to(&curr_target);
            let dist =
                geo::euclidean_distance(prev_target.x, prev_target.y, curr_target.x, curr_target.y);
            let radius = c.border_radius.0;
            let units = radius.min(dist / 2.);
            let prev_t = prev_vector.unit().multiply(units).to_point();
            let curr_t = curr_vector.unit().multiply(units).to_point();
            path.push(format!(
                "L {} {}",
                f(prev_target.x - prev_t.x),
                f(prev_target.y - prev_t.y)
            ));
            if units < radius && i + 2 < route.len() {
                let next_target = route[i + 2];
                let next_vector =
                    Vector::new(&[next_target.x - curr_target.x, next_target.y - curr_target.y]);
                i += 1;
                let next_t = next_vector.unit().multiply(units).to_point();
                path.push(format!(
                    "C {} {} {} {} {} {}",
                    f(prev_target.x + prev_t.x),
                    f(prev_target.y + prev_t.y),
                    f(curr_target.x - next_t.x),
                    f(curr_target.y - next_t.y),
                    f(curr_target.x + next_t.x),
                    f(curr_target.y + next_t.y)
                ));
            } else {
                path.push(format!(
                    "S {} {} {} {}",
                    f(prev_target.x),
                    f(prev_target.y),
                    f(prev_target.x + curr_t.x),
                    f(prev_target.y + curr_t.y)
                ));
            }
            i += 1;
        }
        let last = route[route.len() - 1];
        path.push(format!(
            "L {} {}",
            f(last.x + dst_adj.x),
            f(last.y + dst_adj.y)
        ));
    }
    path.join(" ")
}

/// d2svg.makeLabelMask.
fn make_label_mask(tl: Point, width: i64, height: i64, opacity: f64) -> String {
    let fill = if opacity != 1.0 {
        format!("rgba(0,0,0,{opacity:.2})")
    } else {
        "black".into()
    };
    format!(
        r#"<rect x="{}" y="{}" width="{}" height="{height}" fill="{fill}"></rect>"#,
        format_float(tl.x - 2.),
        format_float(tl.y),
        width + 4
    )
}

/// d2svg.makeBorderLabelMask. Sketch mode passes the shape position: shapes
/// are drawn translated there, and the hand-drawn border is wider (3x).
#[allow(clippy::too_many_arguments)]
fn make_border_label_mask(
    position: Position,
    tl: Point,
    label_width: i64,
    label_height: i64,
    b: &geo::Box,
    stroke_width: i64,
    opacity: f64,
    shape_pos: Option<Point>,
) -> String {
    let fill = if opacity != 1.0 {
        format!("rgba(0,0,0,{opacity:.2})")
    } else {
        "black".into()
    };
    let mut sw = stroke_width as f64;
    if shape_pos.is_some() {
        sw *= 3.;
    }
    use Position::*;
    let (x, y, w, h) = match position {
        BorderTopLeft | BorderTopCenter | BorderTopRight => (
            tl.x - 2.,
            b.top_left.y - sw / 2.,
            (label_width + 4) as f64,
            sw,
        ),
        BorderBottomLeft | BorderBottomCenter | BorderBottomRight => (
            tl.x - 2.,
            b.top_left.y + b.height - sw / 2.,
            (label_width + 4) as f64,
            sw,
        ),
        BorderLeftTop | BorderLeftMiddle | BorderLeftBottom => (
            b.top_left.x - sw / 2.,
            tl.y - 2.,
            sw,
            (label_height + 4) as f64,
        ),
        BorderRightTop | BorderRightMiddle | BorderRightBottom => (
            b.top_left.x + b.width - sw / 2.,
            tl.y - 2.,
            sw,
            (label_height + 4) as f64,
        ),
        _ => return String::new(),
    };
    let (x, y) = match shape_pos {
        Some(p) => (x - p.x, y - p.y),
        None => (x, y),
    };
    format!(
        r#"<rect x="{}" y="{}" width="{}" height="{}" fill="{fill}"></rect>"#,
        format_float(x),
        format_float(y),
        format_float(w),
        format_float(h)
    )
}

/// d2svg.RenderText.
pub(crate) fn render_text(text: &str, x: f64, height: f64) -> String {
    if !text.contains('\n') {
        return escape_text(text);
    }
    let lines: Vec<&str> = text.split('\n').collect();
    let mut out = String::new();
    for (i, line) in lines.iter().enumerate() {
        let dy = if i == 0 {
            0.0
        } else {
            height / lines.len() as f64
        };
        let mut escaped = escape_text(line);
        if escaped.is_empty() {
            escaped = " ".into();
        }
        write!(
            out,
            r#"<tspan x="{}" dy="{}">{escaped}</tspan>"#,
            format_float(x),
            format_float(dy)
        )
        .unwrap();
    }
    out
}

fn base64_class(id: &str) -> String {
    base64::engine::general_purpose::URL_SAFE.encode(escape_text(id))
}

fn font_class(family: &str, bold: bool, italic: bool, underline: bool) -> String {
    let mut class = if family == "mono" {
        "text-mono".to_owned()
    } else {
        "text".to_owned()
    };
    if bold {
        class += "-bold";
    } else if italic {
        class += "-italic";
    }
    if underline {
        class += " text-underline";
    }
    class
}

/// d2svg.drawConnection.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_connection(
    w: &mut String,
    diagram_hash: &str,
    c: &TargetConnection,
    markers: &mut BTreeSet<String>,
    id_to_shape: &std::collections::HashMap<&str, &TargetShape>,
    sketch: bool,
    theme: Option<&Theme>,
    markdown: &mut super::markdown::MarkdownRenderer<'_>,
) -> Result<String, String> {
    let opacity_style = if c.opacity.0 != 1.0 {
        format!(" style='opacity:{}'", format_float(c.opacity.0))
    } else {
        String::new()
    };
    let mut classes = vec![base64_class(&c.id)];
    classes.extend(c.classes.iter().cloned());
    write!(
        w,
        r#"<g class="{}"{opacity_style}>"#,
        super::markup::escape_attribute(classes.join(" "))
    )
    .unwrap();
    let mut marker_start = String::new();
    if c.src_arrow != "none" {
        let id = arrowhead_marker_id(diagram_hash, false, c);
        if markers.insert(id.clone()) {
            let marker = arrowhead_marker(false, &id, c, theme);
            if marker.is_empty() {
                return Err(format!("received empty arrow head marker for: {}", c.id));
            }
            w.push_str(&marker);
        }
        marker_start = format!(r#"marker-start="url(#{id})" "#);
    }
    let mut marker_end = String::new();
    if c.dst_arrow != "none" {
        let id = arrowhead_marker_id(diagram_hash, true, c);
        if markers.insert(id.clone()) {
            let marker = arrowhead_marker(true, &id, c, theme);
            if marker.is_empty() {
                return Err(format!("received empty arrow head marker for: {}", c.id));
            }
            w.push_str(&marker);
        }
        marker_end = format!(r#"marker-end="url(#{id})" "#);
    }
    if let Some(icon) = &c.icon {
        if let Some(position) = c.get_icon_position() {
            let clip = if c.icon_border_radius.0 != 0.0 {
                format!(
                    r#" clip-path="inset(0 round {}px)""#,
                    format_float(c.icon_border_radius.0)
                )
            } else {
                String::new()
            };
            write!(
                w,
                r#"<image href="{}" x="{}" y="{}" width="{DEFAULT_ICON_SIZE}" height="{DEFAULT_ICON_SIZE}"{clip} />"#,
                super::classtable::html_escape(&crate::target::url(icon)),
                format_float(position.x),
                format_float(position.y)
            )
            .unwrap();
        }
    }
    let mut label_tl = Point::new(0., 0.);
    let mut label_mask = String::new();
    if !c.label.is_empty() {
        if c.route_points().len() < 2 {
            return Err(crate::RenderError::SourceNilPointer {
                source_site: "d2svg.drawConnection:1136",
            }
            .to_string());
        }
        label_tl = c.get_label_top_left();
        label_tl.x = label_tl.x.round();
        label_tl.y = label_tl.y.round();
        let mut mask_tl = label_tl;
        let mut width = c.label_width;
        if c.icon.is_some() {
            width += CONNECTION_ICON_LABEL_GAP + DEFAULT_ICON_SIZE;
            mask_tl.x -= (CONNECTION_ICON_LABEL_GAP + DEFAULT_ICON_SIZE) as f64;
        }
        let opacity = if Position::from_string(&c.label_position).is_on_edge() {
            1.0
        } else {
            0.75
        };
        label_mask = make_label_mask(mask_tl, width, c.label_height, opacity);
    } else if c.icon.is_some() {
        if let Some(position) = c.get_icon_position() {
            let edge = Position::from_string(c.icon_position.as_deref().unwrap_or("")).is_on_edge();
            label_mask = make_label_mask(
                position,
                DEFAULT_ICON_SIZE,
                DEFAULT_ICON_SIZE,
                if edge { 1.0 } else { 0.75 },
            );
        }
    }
    let route = c.route_points();
    let (src_adj, dst_adj) = get_arrowhead_adjustments(c, &route, id_to_shape)?;
    let path = path_data(c, &route, src_adj, dst_adj);
    let mask = format!(r#"mask="url(#{diagram_hash})""#);
    let animated_class = if c.animated {
        " animated-connection"
    } else {
        ""
    };
    let both_or_neither = (c.dst_arrow == "none") == (c.src_arrow == "none");
    if sketch {
        w.push_str(&super::sketch::connection(c, &path, &mask)?);
        // Sketch arrowheads are rendered separately.
        w.push_str(&super::sketch::arrowheads(c, &route, src_adj, dst_adj));
    } else if c.animated && both_or_neither {
        let (path1, path2) = svgpath::split_path(&path, 0.5)?;
        let mut el1 = ThemableElement::new("path", theme);
        el1.d = path1;
        el1.fill = "none".into();
        el1.stroke.clone_from(&c.stroke);
        el1.class_name = format!("connection{animated_class}");
        el1.style = c.css_style() + "animation-direction: reverse;";
        el1.attributes = format!("{marker_start}{mask}");
        w.push_str(&el1.render());
        let mut el2 = ThemableElement::new("path", theme);
        el2.d = path2;
        el2.fill = "none".into();
        el2.stroke.clone_from(&c.stroke);
        el2.class_name = format!("connection{animated_class}");
        el2.style = c.css_style();
        el2.attributes = format!("{marker_end}{mask}");
        w.push_str(&el2.render());
    } else {
        let mut el = ThemableElement::new("path", theme);
        el.d = path;
        el.fill = "none".into();
        el.stroke.clone_from(&c.stroke);
        el.class_name = format!("connection{animated_class}");
        el.style = c.css_style();
        el.attributes = format!("{marker_start}{marker_end}{mask}");
        w.push_str(&el.render());
    }
    if !c.label.is_empty() && !c.language.is_empty() {
        let rich_label_link = !c.link.is_empty();
        if rich_label_link {
            write!(w, r#"<a href="{}" xlink:href="{0}">"#, escape_text(&c.link)).unwrap();
        }
        match c.language.as_str() {
            "latex" => super::latex::connection_label(w, c, label_tl, theme)?,
            "markdown" => w.push_str(&markdown.render_connection_label(c, label_tl)?),
            _ => super::code::connection_label(w, c, label_tl, theme)?,
        }
        if rich_label_link {
            w.push_str("</a>");
        }
    } else if !c.label.is_empty() {
        let class = font_class(&c.font_family, c.bold, c.italic, c.underline);
        if !c.fill_str().is_empty() {
            let mut rect = ThemableElement::new("rect", theme);
            rect.rx = 10.;
            rect.x = label_tl.x - 4.;
            rect.y = label_tl.y - 3.;
            rect.width = c.label_width as f64 + 8.;
            rect.height = c.label_height as f64 + 6.;
            rect.fill = c.fill_str().to_owned();
            w.push_str(&rect.render());
        }
        let mut text = ThemableElement::new("text", theme);
        text.x = label_tl.x + c.label_width as f64 / 2.;
        text.y = label_tl.y + c.font_size as f64;
        text.class_name = class;
        text.style = format!("text-anchor:middle;font-size:{}px", c.font_size);
        text.content = render_text(&c.label, text.x, c.label_height as f64);
        if !c.link.is_empty() {
            text.class_name += " text-underline text-link";
            write!(w, r#"<a href="{}" xlink:href="{0}">"#, escape_text(&c.link)).unwrap();
        } else {
            text.fill = c.get_font_color();
        }
        w.push_str(&text.render());
        if !c.link.is_empty() {
            w.push_str("</a>");
        }
    }
    for (is_dst, label) in [(false, &c.src_label), (true, &c.dst_label)] {
        if let Some(label) = label.as_ref().filter(|l| !l.label.is_empty()) {
            w.push_str(&render_arrowhead_label(c, &label.label, is_dst, theme));
        }
    }
    w.push_str("</g>");
    Ok(label_mask)
}

/// d2svg.renderArrowheadLabel.
fn render_arrowhead_label(
    c: &TargetConnection,
    text: &str,
    is_dst: bool,
    theme: Option<&Theme>,
) -> String {
    let label = if is_dst {
        c.dst_label.as_ref()
    } else {
        c.src_label.as_ref()
    }
    .expect("arrowhead label");
    let (width, height) = (label.label_width as f64, label.label_height as f64);
    let tl = c.get_arrowhead_label_position(is_dst);
    let mut el = ThemableElement::new("text", theme);
    el.x = tl.x + width / 2.;
    el.y = tl.y + c.font_size as f64;
    el.fill = FG_COLOR.into();
    if !label.color.is_empty() {
        el.fill.clone_from(&label.color);
    }
    el.class_name = "text-italic".into();
    el.style = format!("text-anchor:middle;font-size:{}px", c.font_size);
    el.content = render_text(text, el.x, height);
    el.render()
}

/// d2svg.renderOval.
fn render_oval(
    tl: Point,
    width: f64,
    height: f64,
    fill: &str,
    fill_pattern: &str,
    stroke: &str,
    style: &str,
    theme: Option<&Theme>,
) -> String {
    let mut el = ThemableElement::new("ellipse", theme);
    el.rx = width / 2.;
    el.ry = height / 2.;
    el.cx = tl.x + el.rx;
    el.cy = tl.y + el.ry;
    el.fill = fill.into();
    el.stroke = stroke.into();
    el.fill_pattern = fill_pattern.into();
    el.class_name = "shape".into();
    el.style = style.into();
    el.render()
}

/// d2svg.renderDoubleOval.
fn render_double_oval(
    tl: Point,
    width: f64,
    height: f64,
    fill: &str,
    fill_stroke: &str,
    stroke: &str,
    style: &str,
    theme: Option<&Theme>,
) -> String {
    let offset = INNER_BORDER_OFFSET as f64;
    let inner = tl.add_vector(&Vector::new(&[offset, offset]));
    render_oval(tl, width, height, fill, fill_stroke, stroke, style, theme)
        + &render_oval(
            inner,
            width - 10.,
            height - 10.,
            fill,
            "",
            stroke,
            style,
            theme,
        )
}

/// d2themes.ShapeTheme.
fn shape_theme(s: &TargetShape) -> (String, String) {
    if s.shape_type == "sql_table" || s.shape_type == "class" {
        (s.stroke.clone(), s.fill.clone())
    } else {
        (s.fill.clone(), s.stroke.clone())
    }
}

/// d2svg.render3DRect.
fn render_3d_rect(diagram_hash: &str, s: &TargetShape, theme: Option<&Theme>) -> String {
    let (px, py) = (s.pos.x, s.pos.y);
    let off = THREE_DEE_OFFSET;
    let move_to = |x: i64, y: i64| format!("M{},{}", x + px, y + py);
    let line_to = |x: i64, y: i64| format!("L{},{}", x + px, y + py);
    let mut border_segments = vec![move_to(0, 0)];
    for (x, y) in [
        (off, -off),
        (s.width + off, -off),
        (s.width + off, s.height - off),
        (s.width, s.height),
        (0, s.height),
        (0, 0),
        (s.width, 0),
        (s.width, s.height),
    ] {
        border_segments.push(line_to(x, y));
    }
    border_segments.push(move_to(s.width, 0));
    border_segments.push(line_to(s.width + off, -off));
    let mut border = ThemableElement::new("path", theme);
    border.d = border_segments.join(" ");
    border.fill = "none".into();
    border.stroke = shape_theme(s).1;
    let border_style = s.css_style();
    border.style.clone_from(&border_style);
    let rendered_border = border.render();
    let mask_id = scoped_id(&format!("border-mask-{diagram_hash}-"), &s.id, "");
    let border_mask = [
        format!(
            r#"<defs><mask id="{mask_id}" maskUnits="userSpaceOnUse" x="{px}" y="{}" width="{}" height="{}">"#,
            py - off,
            s.width + off,
            s.height + off
        ),
        format!(r#"<rect x="{px}" y="{}" width="{}" height="{}" fill="white"></rect>"#, py - off, s.width + off, s.height + off),
        format!(r#"<path d="{}" style="{border_style};stroke:#000;fill:none;opacity:1;"/></mask></defs>"#, border_segments.concat()),
    ]
    .join("\n");
    let mut main = ThemableElement::new("rect", theme);
    main.x = px as f64;
    main.y = py as f64;
    main.width = s.width as f64;
    main.height = s.height as f64;
    main.set_mask_url(&mask_id);
    main.fill = shape_theme(s).0;
    main.fill_pattern = s.fill_pattern_str().to_owned();
    main.stroke = "none".into();
    main.style = s.css_style();
    let main_rendered = main.render();
    let side_points: Vec<String> = [
        (0, 0),
        (off, -off),
        (s.width + off, -off),
        (s.width + off, s.height - off),
        (s.width, s.height),
        (s.width, 0),
    ]
    .iter()
    .map(|(x, y)| format!("{},{}", x + px, y + py))
    .collect();
    let darker = color::darken(&s.fill).unwrap_or_else(|_| s.fill.clone());
    let mut side = ThemableElement::new("polygon", theme);
    side.fill = darker;
    side.points = side_points.join(" ");
    side.set_mask_url(&mask_id);
    side.style = s.css_style();
    border_mask + &main_rendered + &side.render() + &rendered_border
}

/// d2svg.render3DHexagon.
fn render_3d_hexagon(diagram_hash: &str, s: &TargetShape, theme: Option<&Theme>) -> String {
    let (px, py) = (s.pos.x, s.pos.y);
    let off = THREE_DEE_OFFSET;
    let move_to = |x: i64, y: i64| format!("M{},{}", x + px, y + py);
    let line_to = |x: i64, y: i64| format!("L{},{}", x + px, y + py);
    let scale = |n: i64, f: f64| (n as f64 * f) as i64;
    let half_y = 43.6 / 87.3;
    let y_off = off / 2;
    let (w, h) = (s.width, s.height);
    let mut border_segments = vec![move_to(scale(w, 0.25), 0)];
    for (x, y) in [
        (scale(w, 0.25) + off, -y_off),
        (scale(w, 0.75) + off, -y_off),
        (w + off, scale(h, half_y) - y_off),
        (scale(w, 0.75) + off, h - y_off),
        (scale(w, 0.75), h),
        (scale(w, 0.25), h),
        (0, scale(h, half_y)),
        (scale(w, 0.25), 0),
        (scale(w, 0.75), 0),
        (w, scale(h, half_y)),
        (scale(w, 0.75), h),
    ] {
        border_segments.push(line_to(x, y));
    }
    for (x, y) in [
        (scale(w, 0.75), 0),
        (w, scale(h, half_y)),
        (scale(w, 0.75), h),
    ] {
        border_segments.push(move_to(x, y));
        border_segments.push(line_to(x + off, y - y_off));
    }
    let mut border = ThemableElement::new("path", theme);
    border.d = border_segments.join(" ");
    border.fill = "none".into();
    border.stroke = shape_theme(s).1;
    let border_style = s.css_style();
    border.style.clone_from(&border_style);
    let rendered_border = border.render();
    let main_points: Vec<String> = [
        (scale(w, 0.25), 0),
        (scale(w, 0.75), 0),
        (w, scale(h, half_y)),
        (scale(w, 0.75), h),
        (scale(w, 0.25), h),
        (0, scale(h, half_y)),
    ]
    .iter()
    .map(|(x, y)| format!("{},{}", x + px, y + py))
    .collect();
    let mask_id = scoped_id(&format!("border-mask-{diagram_hash}-"), &s.id, "");
    let border_mask = [
        format!(r#"<defs><mask id="{mask_id}" maskUnits="userSpaceOnUse" x="{px}" y="{}" width="{}" height="{}">"#, py - off, w + off, h + off),
        format!(r#"<rect x="{px}" y="{}" width="{}" height="{}" fill="white"></rect>"#, py - off, w + off, h + off),
        format!(r#"<path d="{}" style="{border_style};stroke:#000;fill:none;opacity:1;"/></mask></defs>"#, border_segments.concat()),
    ]
    .join("\n");
    let mut main = ThemableElement::new("polygon", theme);
    main.x = px as f64;
    main.y = py as f64;
    main.points = main_points.join(" ");
    main.set_mask_url(&mask_id);
    main.fill_pattern = s.fill_pattern_str().to_owned();
    main.fill = shape_theme(s).0;
    main.stroke = "none".into();
    main.style = s.css_style();
    let main_rendered = main.render();
    let side_points: Vec<String> = [
        (scale(w, 0.25) + off, -y_off),
        (scale(w, 0.75) + off, -y_off),
        (w + off, scale(h, half_y) - y_off),
        (scale(w, 0.75) + off, h - y_off),
        (scale(w, 0.75), h),
        (w, scale(h, half_y)),
        (scale(w, 0.75), 0),
        (scale(w, 0.25), 0),
    ]
    .iter()
    .map(|(x, y)| format!("{},{}", x + px, y + py))
    .collect();
    let darker = color::darken(&s.fill).unwrap_or_else(|_| s.fill.clone());
    let mut side = ThemableElement::new("polygon", theme);
    side.fill = darker;
    side.points = side_points.join(" ");
    side.set_mask_url(&mask_id);
    side.style = s.css_style();
    border_mask + &main_rendered + &side.render() + &rendered_border
}

/// d2svg.drawShape.
pub(crate) fn draw_shape(
    w: &mut String,
    appendix: &mut String,
    diagram_hash: &str,
    s: &TargetShape,
    sketch: bool,
    theme: Option<&Theme>,
    markdown: &mut super::markdown::MarkdownRenderer<'_>,
) -> Result<String, String> {
    let mut label_mask = String::new();
    let mut closing_tag = "</g>".to_owned();
    if !s.link.is_empty() {
        write!(w, r#"<a href="{}" xlink:href="{0}">"#, escape_text(&s.link)).unwrap();
        closing_tag += "</a>";
    }
    // This clipPath must be defined outside the shape's `g` element.
    if s.border_radius != 0 && matches!(s.shape_type.as_str(), "class" | "sql_table") {
        w.push_str(&super::classtable::clip_path_for_border_radius(
            diagram_hash,
            s,
        ));
    }
    let mut icon_border_radius = s.icon_border_radius;
    let mut icon_clip_path_id = String::new();
    if s.icon_border_radius != 0 && s.shape_type == "image" {
        icon_border_radius = icon_border_radius.min(s.width.min(s.height) / 2);
        icon_clip_path_id = format!("{diagram_hash}-{}-icon", svg_id(&s.id));
        w.push_str(&apply_icon_border_radius(
            &icon_clip_path_id,
            s,
            icon_border_radius,
        ));
    }
    let opacity_style = if s.opacity.0 != 1.0 {
        format!(" style='opacity:{}'", format_float(s.opacity.0))
    } else {
        String::new()
    };
    let mut classes = vec![base64_class(&s.id)];
    if s.animated {
        classes.push("animated-shape".into());
    }
    classes.extend(s.classes.iter().cloned());
    write!(
        w,
        r#"<g class="{}"{opacity_style}>"#,
        super::markup::escape_attribute(classes.join(" "))
    )
    .unwrap();
    let tl = s.top_left();
    let (width, height) = (s.width as f64, s.height as f64);
    let (fill, stroke) = shape_theme(s);
    let style = s.css_style();
    let shape_type = shape::dsl_shape_to_shape_type(&s.shape_type);
    let mut sh = shape::new_shape(shape_type, geo::Box::new(tl, width, height));
    if shape_type == shape::CLOUD_TYPE {
        if let Some(ratio) = s.content_aspect_ratio {
            sh.set_inner_box_aspect_ratio(ratio.0);
        }
    }
    let shadow_attr = if s.shadow
        && !matches!(
            s.shape_type.as_str(),
            "text" | "code" | "class" | "sql_table"
        ) {
        r#"filter="url(#shadow-filter)" "#
    } else {
        ""
    };
    let blend = if s.blend { " blend" } else { "" };
    write!(w, r#"<g class="shape{blend}" {shadow_attr}>"#).unwrap();
    let multiple_tl = tl.add_vector(&Vector::new(&[
        MULTIPLE_OFFSET as f64,
        -(MULTIPLE_OFFSET as f64),
    ]));
    let border_label =
        !s.label.is_empty() && Position::from_string(s.label_position_str()).is_border();
    let fill_pattern = s.fill_pattern_str();
    match s.shape_type.as_str() {
        "class" | "sql_table" => {
            if sketch {
                w.push_str(&if s.shape_type == "class" {
                    super::sketch::class(s)?
                } else {
                    super::sketch::table(s)?
                });
            } else if s.shape_type == "class" {
                super::classtable::draw_class(w, diagram_hash, s, theme)?;
            } else {
                super::classtable::draw_table(w, diagram_hash, s, theme)?;
            }
            add_appendix_items(appendix, diagram_hash, s, sh.as_ref(), markdown)?;
            w.push_str("</g>");
            w.push_str(&closing_tag);
            return Ok(label_mask);
        }
        "image" => {
            let mut el = ThemableElement::new("image", theme);
            el.x = s.pos.x as f64;
            el.y = s.pos.y as f64;
            el.width = s.width as f64;
            el.height = s.height as f64;
            el.href = s.icon.as_ref().map(crate::target::url).unwrap_or_default();
            el.fill.clone_from(&fill);
            el.stroke.clone_from(&stroke);
            el.style.clone_from(&style);
            if icon_border_radius != 0 {
                el.clip_path.clone_from(&icon_clip_path_id);
            }
            w.push_str(&el.render());
        }
        "oval" => {
            if s.double_border {
                if s.multiple {
                    w.push_str(&render_double_oval(
                        multiple_tl,
                        width,
                        height,
                        &fill,
                        "",
                        &stroke,
                        &style,
                        theme,
                    ));
                }
                if sketch {
                    w.push_str(&super::sketch::double_oval(s, diagram_hash)?);
                } else {
                    w.push_str(&render_double_oval(
                        tl,
                        width,
                        height,
                        &fill,
                        fill_pattern,
                        &stroke,
                        &style,
                        theme,
                    ));
                }
            } else {
                if s.multiple {
                    w.push_str(&render_oval(
                        multiple_tl,
                        width,
                        height,
                        &fill,
                        "",
                        &stroke,
                        &style,
                        theme,
                    ));
                }
                if sketch {
                    w.push_str(&super::sketch::oval(s, diagram_hash)?);
                } else {
                    w.push_str(&render_oval(
                        tl,
                        width,
                        height,
                        &fill,
                        fill_pattern,
                        &stroke,
                        &style,
                        theme,
                    ));
                }
            }
        }
        "rectangle" | "sequence_diagram" | "hierarchy" | "" => {
            let border_radius = if s.border_radius != 0 {
                s.border_radius as f64
            } else {
                f64::MAX
            };
            if s.three_dee {
                w.push_str(&render_3d_rect(diagram_hash, s, theme));
            } else {
                let rect = |x: f64, y: f64, w2: f64, h2: f64, fill: &str, pattern: &str| {
                    let mut el = ThemableElement::new("rect", theme);
                    el.x = x;
                    el.y = y;
                    el.width = w2;
                    el.height = h2;
                    el.fill = fill.into();
                    el.fill_pattern = pattern.into();
                    el.stroke.clone_from(&stroke);
                    el.style.clone_from(&style);
                    el.rx = border_radius;
                    el
                };
                let (x, y) = (s.pos.x as f64, s.pos.y as f64);
                let ib = INNER_BORDER_OFFSET as f64;
                if !s.double_border {
                    if s.multiple {
                        w.push_str(&rect(x + 10., y - 10., width, height, &fill, "").render());
                    }
                    if sketch {
                        w.push_str(&super::sketch::rect(s, diagram_hash)?);
                    } else {
                        let mut el = rect(x, y, width, height, &fill, fill_pattern);
                        if border_label {
                            el.mask = format!("url(#{diagram_hash})");
                        }
                        w.push_str(&el.render());
                    }
                } else {
                    if s.multiple {
                        w.push_str(
                            &rect(x + 10., y - 10., width, height, &fill, fill_pattern).render(),
                        );
                        w.push_str(
                            &rect(
                                x + 10. + ib,
                                y - 10. + ib,
                                width - 2. * ib,
                                height - 2. * ib,
                                &fill,
                                "",
                            )
                            .render(),
                        );
                    }
                    if sketch {
                        w.push_str(&super::sketch::double_rect(s, diagram_hash)?);
                    } else {
                        w.push_str(&rect(x, y, width, height, &fill, fill_pattern).render());
                        w.push_str(
                            &rect(
                                x + ib,
                                y + ib,
                                width - 2. * ib,
                                height - 2. * ib,
                                "transparent",
                                "",
                            )
                            .render(),
                        );
                    }
                }
            }
        }
        "hexagon" if s.three_dee => w.push_str(&render_3d_hexagon(diagram_hash, s, theme)),
        "text" | "code" => {}
        _ => {
            if s.multiple {
                let multiple =
                    shape::new_shape(shape_type, geo::Box::new(multiple_tl, width, height));
                let mut el = ThemableElement::new("path", theme);
                el.fill.clone_from(&fill);
                el.stroke.clone_from(&stroke);
                el.style.clone_from(&style);
                for d in multiple.get_svg_path_data() {
                    el.d = d;
                    w.push_str(&el.render());
                }
            }
            if sketch {
                w.push_str(&super::sketch::paths(
                    s,
                    diagram_hash,
                    &sh.get_svg_path_data(),
                )?);
            } else {
                let mut el = ThemableElement::new("path", theme);
                el.fill.clone_from(&fill);
                el.fill_pattern = fill_pattern.to_owned();
                el.stroke.clone_from(&stroke);
                el.style.clone_from(&style);
                // drawShape's hexagon branch never masks border labels.
                if border_label && s.shape_type != "hexagon" {
                    el.mask = format!("url(#{diagram_hash})");
                }
                for d in sh.get_svg_path_data() {
                    el.d = d;
                    w.push_str(&el.render());
                }
            }
        }
    }
    w.push_str("</g>");
    if let Some(icon) = s
        .icon
        .as_ref()
        .filter(|_| s.shape_type != "image" && s.opacity.0 != 0.0)
    {
        let icon_position = Position::from_string(&s.icon_position);
        let b = if icon_position.is_outside() {
            sh.get_box()
        } else {
            sh.get_inner_box()
        };
        let size = get_icon_size(&b, &s.icon_position);
        let tl = icon_position.get_point_on_box(&b, label::PADDING, size as f64, size as f64);
        let clip = if s.icon_border_radius != 0 {
            format!(r#" clip-path="inset(0 round {}px)""#, s.icon_border_radius)
        } else {
            String::new()
        };
        write!(
            w,
            r#"<image href="{}" x="{}" y="{}" width="{size}" height="{size}"{clip} />"#,
            super::classtable::html_escape(&crate::target::url(icon)),
            format_float(tl.x),
            format_float(tl.y)
        )
        .unwrap();
    }
    if !s.label.is_empty() && s.opacity.0 != 0.0 {
        let mut position = Position::from_string(s.label_position_str());
        // The foreignObject-era Markdown renderer defaulted an unset position
        // to the middle of the shape; the native renderer keeps that.
        if s.language == "markdown" && position == Position::Unset {
            position = Position::InsideMiddleCenter;
        }
        let b = if position.is_outside() || position.is_border() {
            let mut b = sh.get_box();
            if s.three_dee {
                let offset_y = if s.shape_type == "hexagon" {
                    THREE_DEE_OFFSET / 2
                } else {
                    THREE_DEE_OFFSET
                };
                b.top_left.y -= offset_y as f64;
                b.height += offset_y as f64;
                b.width += THREE_DEE_OFFSET as f64;
            } else if s.multiple {
                b.top_left.y -= MULTIPLE_OFFSET as f64;
                b.height += MULTIPLE_OFFSET as f64;
                b.width += MULTIPLE_OFFSET as f64;
            }
            b
        } else {
            sh.get_inner_box()
        };
        let label_tl = position.get_point_on_box(
            &b,
            label::PADDING,
            s.label_width as f64,
            s.label_height as f64,
        );
        if position.is_border() {
            let shape_pos = sketch.then_some(tl);
            label_mask = make_border_label_mask(
                position,
                label_tl,
                s.label_width,
                s.label_height,
                &b,
                s.stroke_width,
                1.0,
                shape_pos,
            );
        }
        if s.language == "latex" {
            super::latex::shape_label(w, s, sh.as_ref(), theme)?;
        } else if s.language == "markdown" {
            w.push_str(&markdown.render_shape_label(s, label_tl)?);
        } else if !s.language.is_empty() {
            super::code::shape_code(w, s, &b, theme)?;
        } else {
            if let Some(label_fill) = s.label_fill.as_deref().filter(|f| !f.is_empty()) {
                let mut rect = ThemableElement::new("rect", theme);
                rect.x = label_tl.x;
                rect.y = label_tl.y;
                rect.width = s.label_width as f64;
                rect.height = s.label_height as f64;
                rect.fill = label_fill.to_owned();
                w.push_str(&rect.render());
            }
            let mut text = ThemableElement::new("text", theme);
            text.x = label_tl.x + s.label_width as f64 / 2.;
            text.y = label_tl.y + s.font_size as f64;
            text.fill = s.get_font_color();
            text.class_name = font_class(&s.font_family, s.bold, s.italic, s.underline);
            text.style = format!("text-anchor:middle;font-size:{}px", s.font_size);
            text.content = render_text(&s.label, text.x, s.label_height as f64);
            w.push_str(&text.render());
        }
    }
    if !s.tooltip.is_empty() && s.tooltip_position_str().is_empty() {
        write!(w, "<title>{}</title>", escape_text(&s.tooltip)).unwrap();
    }
    add_appendix_items(appendix, diagram_hash, s, sh.as_ref(), markdown)?;
    w.push_str(&closing_tag);
    Ok(label_mask)
}

const APPENDIX_ICON_RADIUS: i64 = 16;
const TOOLTIP_ICON: &str = include_str!("assets/tooltip.svg");
const LINK_ICON: &str = include_str!("assets/link.svg");

/// d2svg.applyIconBorderRadius.
fn apply_icon_border_radius(id: &str, s: &TargetShape, radius: i64) -> String {
    let f = format_float;
    let r = radius as f64;
    let (x, y, w, h) = (
        s.pos.x as f64,
        s.pos.y as f64,
        s.width as f64,
        s.height as f64,
    );
    let (top_x, top_y) = (x + w, y);
    let mut out = format!(r#"<clipPath id="{id}">"#);
    write!(
        out,
        r#"<path d="M {} {} L {} {} S {} {} {} {} "#,
        f(x),
        f(y + r),
        f(x),
        f(y + r),
        f(x),
        f(y),
        f(x + r),
        f(y)
    )
    .unwrap();
    write!(
        out,
        "L {} {} L {} {} ",
        f(x + w - r),
        f(y),
        f(top_x - r),
        f(top_y)
    )
    .unwrap();
    write!(
        out,
        "S {} {} {} {} ",
        f(top_x),
        f(top_y),
        f(top_x),
        f(top_y + r)
    )
    .unwrap();
    write!(out, "L {} {} ", f(top_x), f(top_y + h - r)).unwrap();
    write!(
        out,
        "S {}  {} {} {} ",
        f(top_x),
        f(top_y + h),
        f(top_x - r),
        f(top_y + h)
    )
    .unwrap();
    write!(out, "L {} {} ", f(x + r), f(y + h)).unwrap();
    write!(out, "S {} {} {} {}", f(x), f(y + h), f(x), f(y + h - r)).unwrap();
    write!(out, "L {} {}", f(x), f(y + r)).unwrap();
    write!(out, r#"Z {} {}" "#, f(x), f(y)).unwrap();
    out + r#"fill="none" /> </clipPath>"#
}

/// d2svg.addAppendixItems.
fn add_appendix_items(
    w: &mut String,
    diagram_hash: &str,
    s: &TargetShape,
    sh: &dyn shape::Shape,
    markdown: &mut super::markdown::MarkdownRenderer<'_>,
) -> Result<(), String> {
    if s.tooltip.is_empty() && s.link.is_empty() {
        return Ok(());
    }
    let both = !s.tooltip.is_empty() && !s.link.is_empty();
    let mut corner = Point::new((s.pos.x + s.width) as f64, s.pos.y as f64);
    let mut center = Point::new(
        s.pos.x as f64 + s.width as f64 / 2.,
        s.pos.y as f64 + s.height as f64 / 2.,
    );
    let offset = Vector::new(&[-2.0 * APPENDIX_ICON_RADIUS as f64, 0.0]);
    let mut left_on_shape = false;
    match sh.get_type() {
        shape::STEP_TYPE | shape::HEXAGON_TYPE | shape::QUEUE_TYPE | shape::PAGE_TYPE => {
            center.y = s.pos.y as f64
        }
        shape::PACKAGE_TYPE => center.x = (s.pos.x + s.width) as f64,
        shape::CIRCLE_TYPE
        | shape::OVAL_TYPE
        | shape::DIAMOND_TYPE
        | shape::PERSON_TYPE
        | shape::CLOUD_TYPE
        | shape::CYLINDER_TYPE
            if both =>
        {
            left_on_shape = true;
            corner = corner.add_vector(&offset);
        }
        _ => {}
    }
    let v1 = center.vector_to(&corner);
    let mut p1 = shape::trace_to_shape_border(sh, corner, corner.add_vector(&v1));
    let mut p2 = None;
    if both {
        if left_on_shape {
            p2 = Some(p1);
            p1 = p1.add_vector(&offset.reverse());
        } else {
            p2 = Some(p1.add_vector(&offset));
        }
    }
    if !s.tooltip.is_empty() && !s.tooltip_position_str().is_empty() {
        w.push_str(&super::tooltip::render_positioned(s, markdown)?);
    } else if !s.tooltip.is_empty() {
        let (x, y) = (p1.x.ceil() as i64, p1.y.ceil() as i64);
        let icon = TOOLTIP_ICON
            .replace("%[1]s", diagram_hash)
            .replace("%[2]s", &svg_id(&s.id));
        write!(
            w,
            r#"<g transform="translate({} {})" class="appendix-icon"><title>{}</title>{icon}</g>"#,
            x - APPENDIX_ICON_RADIUS,
            y - APPENDIX_ICON_RADIUS,
            escape_text(&s.tooltip)
        )
        .unwrap();
    }
    if !s.link.is_empty() {
        let p = p2.unwrap_or(p1);
        let (x, y) = (p.x.ceil() as i64, p.y.ceil() as i64);
        let icon = LINK_ICON
            .replace("%[1]s", diagram_hash)
            .replace("%[2]s", &svg_id(&s.id));
        write!(
            w,
            r#"<g transform="translate({} {})" class="appendix-icon">{icon}</g>"#,
            x - APPENDIX_ICON_RADIUS,
            y - APPENDIX_ICON_RADIUS
        )
        .unwrap();
    }
    Ok(())
}
