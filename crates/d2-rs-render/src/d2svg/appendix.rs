// SPDX-License-Identifier: MPL-2.0
// Port of D2 v0.9.0 d2renderers/d2svg/appendix/appendix.go (Append for SVG
// exports with --force-appendix).

use d2_rs_core::stable_sort::StableSort;
use std::sync::OnceLock;

use regex::Regex;

use super::element::ThemableElement;
use super::render::render_text;
use super::svgtext::svg_id;
use crate::target::{TargetDiagram, TargetShape};

const PAD_TOP: i64 = 50;
const PAD_SIDES: i64 = 40;
const SPACER: i64 = 20;
const FONT_SIZE: i64 = 16;
const ICON_RADIUS: i64 = 16;
const TOOLTIP_ICON: &str = include_str!("assets/tooltip.svg");
const LINK_ICON: &str = include_str!("assets/link.svg");

fn regexes() -> &'static (Regex, Regex, Regex, Regex) {
    static R: OnceLock<(Regex, Regex, Regex, Regex)> = OnceLock::new();
    R.get_or_init(|| {
        (
            Regex::new(r#"viewBox="([0-9\- ]+)""#).unwrap(),
            Regex::new(r#"width="([.0-9]+)""#).unwrap(),
            Regex::new(r#"height="([.0-9]+)""#).unwrap(),
            Regex::new(r"<svg(.*?)>").unwrap(),
        )
    })
}

/// d2graph.GetTextDimensions for an unstyled 16px SourceSansPro MText.
fn text_dimensions(text: &str) -> Result<(i64, i64), String> {
    use d2_rs_text::measure;
    let error = |e: measure::FontError| format!("{e:?}");
    let mono = measure::bundled_source_code_pro_regular().map_err(error)?;
    let face = measure::bundled_source_sans_pro_regular().map_err(error)?;
    let metrics = face.measure_with_fallback(text, FONT_SIZE as f64, &mono);
    Ok((metrics.width.ceil() as i64, metrics.height.ceil() as i64))
}

/// appendix.generateNumberedIcon.
fn numbered_icon(i: usize, x: i64, y: i64) -> String {
    format!(
        r##"<circle cx="{}" cy="{y}" r="{ICON_RADIUS}" fill="white" stroke="#DEE1EB" /><text class="text-bold" x="{}" y="{}" style="font-size: {FONT_SIZE}px;text-anchor:middle;">{i}</text>"##,
        x + ICON_RADIUS,
        x + ICON_RADIUS,
        y + 5
    )
}

/// appendix.generateLine.
fn line(i: usize, y: i64, text: &str) -> Result<(String, i64, i64), String> {
    let (w, h) = text_dimensions(text)?;
    let out = format!(
        r#"<g transform="translate(0 {y})" class="appendix-icon">{}</g><text class="text" x="{}" y="{}" style="font-size: {FONT_SIZE}px;">{}</text>"#,
        numbered_icon(i, 0, 0),
        ICON_RADIUS * 3,
        y + 5,
        render_text(text, (ICON_RADIUS * 3) as f64, h as f64)
    );
    Ok((out, w + ICON_RADIUS * 3, h.max(ICON_RADIUS * 2)))
}

/// appendix.generateAppendix.
fn generate(diagram: &TargetDiagram) -> Result<(String, i64, i64), String> {
    let ((tlx, _), (brx, bry)) = diagram.go_bounding_box()?;
    let (mut max_width, mut total_height) = (0, 0);
    let mut lines = Vec::new();
    let mut i = 1;
    for s in diagram.shapes_slice() {
        for text in [s.tooltip.as_str(), s.pretty_link_str()] {
            if !text.is_empty() {
                let (l, w, h) = line(i, bry + PAD_TOP * 2 + total_height, text)?;
                i += 1;
                lines.push(l);
                max_width = max_width.max(w);
                total_height += h + SPACER;
            }
        }
    }
    if lines.is_empty() {
        return Ok((String::new(), 0, 0));
    }
    total_height += SPACER;
    Ok((
        format!(
            "<g class=\"appendix\" x=\"{tlx}\" y=\"{bry}\" width=\"{}\" height=\"100%\">{}</g>\n",
            brx - tlx,
            lines.join("\n")
        ),
        max_width,
        total_height,
    ))
}

/// appendix.Append.
pub(crate) fn append(
    diagram: &TargetDiagram,
    diagram_hash: &str,
    input: &[u8],
) -> Result<Vec<u8>, String> {
    let mut svg = String::from_utf8(input.to_vec()).map_err(|e| e.to_string())?;
    let (mut appendix, mut w, h) = generate(diagram)?;
    if h == 0 {
        return Ok(input.to_vec());
    }
    let (viewbox_re, width_re, height_re, svg_re) = regexes();
    let viewboxes: Vec<_> = viewbox_re.captures_iter(&svg).take(2).collect();
    let (outer_match, inner_match) = (viewboxes[0][0].to_owned(), viewboxes[1][0].to_owned());
    let slice: Vec<String> = viewboxes[1][1].split(' ').map(str::to_owned).collect();
    let atoi = |s: &str| s.parse::<i64>().unwrap_or(0);
    let pad_left = atoi(&slice[0]);
    let mut viewbox_width = atoi(&slice[2]);
    let mut viewbox_height = atoi(&slice[3]);
    let ((tlx, _), (brx, bry)) = diagram.go_bounding_box()?;
    let mut separator = ThemableElement::new("line", None);
    separator.x1 = (tlx - PAD_SIDES) as f64;
    separator.y1 = (bry + PAD_TOP) as f64;
    separator.x2 = (w.max(brx) + PAD_SIDES) as f64;
    separator.y2 = (bry + PAD_TOP) as f64;
    separator.stroke = "B2".into();
    appendix = separator.render() + &appendix;
    w -= pad_left;
    w += PAD_SIDES * 2;
    if viewbox_width < w {
        viewbox_width = w;
    }
    viewbox_height += h + PAD_TOP;
    let new_outer = format!(r#"viewBox="0 0 {viewbox_width} {viewbox_height}""#);
    let new_inner = format!(
        r#"viewBox="{} {} {viewbox_width} {viewbox_height}""#,
        slice[0], slice[1]
    );
    let mut dimensions_to_update = 2;
    let outer_svg = svg_re
        .find(&svg)
        .map(|m| m.as_str().to_owned())
        .unwrap_or_default();
    if width_re.is_match(&outer_svg) {
        dimensions_to_update += 1;
    }
    let widths: Vec<String> = width_re
        .find_iter(&svg)
        .take(dimensions_to_update)
        .map(|m| m.as_str().to_owned())
        .collect();
    let heights: Vec<String> = height_re
        .find_iter(&svg)
        .take(dimensions_to_update)
        .map(|m| m.as_str().to_owned())
        .collect();
    svg = svg.replacen(&outer_match, &new_outer, 1);
    svg = svg.replacen(&inner_match, &new_inner, 1);
    for i in 0..dimensions_to_update {
        if let (Some(width), Some(height)) = (widths.get(i), heights.get(i)) {
            svg = svg.replacen(width, &format!(r#"width="{viewbox_width}""#), 1);
            svg = svg.replacen(height, &format!(r#"height="{viewbox_height}""#), 1);
        }
    }
    use d2_rs_text::d2fonts::{FontFamily, FontStyle};
    for (face, class, style) in [
        ("font-regular", "text", FontStyle::Regular),
        ("font-bold", "text-bold", FontStyle::Bold),
    ] {
        if !svg.contains(&format!(r#"font-family: "{face}""#)) {
            let encoding =
                d2_rs_text::d2fonts::font_encoding(FontFamily::SourceSansPro.font(style))
                    .ok_or("appendix: missing bundled font encoding")?;
            appendix += &format!(
                "<style type=\"text/css\"><![CDATA[\n.{class} {{\n\tfont-family: \"{face}\";\n}}\n@font-face {{\n\tfont-family: {face};\n\tsrc: url(\"{encoding}\");\n}}\n]]></style>"
            );
        }
    }
    let closing = svg
        .rfind("</svg></svg>")
        .ok_or("appendix: unterminated svg")?;
    svg.insert_str(closing, &appendix);
    // Icons are numbered in definition order but replaced in render order.
    let mut order: Vec<(usize, bool, &TargetShape)> = Vec::new();
    let mut i = 1;
    for s in diagram.shapes_slice() {
        if !s.tooltip.is_empty() {
            order.push((i, true, s));
            i += 1;
        }
        if !s.link.is_empty() {
            order.push((i, false, s));
            i += 1;
        }
    }
    order.stable_sort_by(|a, b| {
        a.2.z_index
            .cmp(&b.2.z_index)
            .then(a.2.level.cmp(&b.2.level))
    });
    for (number, is_tooltip, s) in order {
        let template = if is_tooltip { TOOLTIP_ICON } else { LINK_ICON };
        let icon = template
            .replace("%[1]s", diagram_hash)
            .replace("%[2]s", &svg_id(&s.id));
        svg = svg.replacen(&icon, &numbered_icon(number, 0, ICON_RADIUS), 1);
    }
    Ok(svg.into_bytes())
}
