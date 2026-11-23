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
            Regex::new(r#" width="[.0-9]+""#).unwrap(),
            Regex::new(r#" height="[.0-9]+""#).unwrap(),
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
fn generate(
    diagram: &TargetDiagram,
    ((tlx, _), (brx, bry)): ((i64, i64), (i64, i64)),
) -> Result<(String, i64, i64), String> {
    let (mut max_width, mut total_height) = (0, 0);
    let mut lines = Vec::new();
    let mut i = 1;
    for s in diagram.shapes_slice() {
        for text in [s.tooltip.as_str(), s.appendix_link_text()] {
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
    scale: Option<f64>,
) -> Result<Vec<u8>, String> {
    let mut svg = String::from_utf8(input.to_vec()).map_err(|e| e.to_string())?;
    let (tl, mut br) = diagram.go_bounding_box()?;
    let is_frame = svg.starts_with("<g ");
    let frame = if is_frame {
        let Some(frame) = super::svgframe::read(&svg) else {
            return Ok(input.to_vec());
        };
        br.1 = br.1.max(frame.bottom - PAD_TOP);
        Some(frame)
    } else {
        None
    };
    let (tlx, _) = tl;
    let (brx, bry) = br;
    let (mut appendix, mut w, h) = generate(diagram, (tl, br))?;
    if h == 0 {
        return Ok(input.to_vec());
    }
    let mut separator = ThemableElement::new("line", None);
    separator.x1 = (tlx - PAD_SIDES) as f64;
    separator.y1 = (bry + PAD_TOP) as f64;
    separator.x2 = (w.max(brx) + PAD_SIDES) as f64;
    separator.y2 = (bry + PAD_TOP) as f64;
    separator.stroke = "B2".into();
    appendix = separator.render() + &appendix;
    if let Some(frame) = frame {
        let Some(resized) = append_to_frame(&svg, &appendix, &frame, tl, br, w, h) else {
            return Ok(input.to_vec());
        };
        return Ok(number_icons(diagram, &frame.master_id, resized).into_bytes());
    }
    let (viewbox_re, width_re, height_re, svg_re) = regexes();
    let viewboxes: Vec<_> = viewbox_re.captures_iter(&svg).take(2).collect();
    let (outer_match, inner_match) = (viewboxes[0][0].to_owned(), viewboxes[1][0].to_owned());
    let slice: Vec<String> = viewboxes[1][1].split(' ').map(str::to_owned).collect();
    let atoi = |s: &str| s.parse::<i64>().unwrap_or(0);
    let pad_left = atoi(&slice[0]);
    let mut viewbox_width = atoi(&slice[2]);
    let mut viewbox_height = atoi(&slice[3]);
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
    // The outer SVG carries display dimensions. The inner SVG and its
    // background use unscaled diagram coordinates.
    let outer_end = svg_re.find(&svg).ok_or("appendix: missing svg")?.end();
    let (mut outer, mut inner) = (svg[..outer_end].to_owned(), svg[outer_end..].to_owned());
    outer = outer.replacen(&outer_match, &new_outer, 1);
    if width_re.is_match(&outer) {
        let dimensions = super::render::scaled_dimensions(
            viewbox_width,
            viewbox_height,
            Some(scale.unwrap_or(1.0)),
        )?;
        outer = height_re.replace_all(&outer, "").into_owned();
        outer = width_re
            .replace_all(&outer, regex::NoExpand(&dimensions))
            .into_owned();
    }
    inner = inner.replacen(&inner_match, &new_inner, 1);
    let new_width = format!(r#" width="{viewbox_width}""#);
    let new_height = format!(r#" height="{viewbox_height}""#);
    for found in width_re
        .find_iter(&inner.clone())
        .take(2)
        .map(|m| m.as_str().to_owned())
    {
        inner = inner.replacen(&found, &new_width, 1);
    }
    for found in height_re
        .find_iter(&inner.clone())
        .take(2)
        .map(|m| m.as_str().to_owned())
    {
        inner = inner.replacen(&found, &new_height, 1);
    }
    svg = outer + &inner;
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
    Ok(number_icons(diagram, diagram_hash, svg).into_bytes())
}

fn number_icons(diagram: &TargetDiagram, diagram_hash: &str, mut svg: String) -> String {
    // Icons are numbered in definition order but replaced in render order.
    let mut order: Vec<(usize, bool, &TargetShape)> = Vec::new();
    let mut i = 1;
    for s in diagram.shapes_slice() {
        if !s.tooltip.is_empty() {
            order.push((i, true, s));
            i += 1;
        }
        if !s.appendix_link_text().is_empty() {
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
    svg
}

// Resize only the opening group and its background rectangles, as upstream does.
fn append_to_frame(
    svg: &str,
    appendix: &str,
    box_: &super::svgframe::Frame,
    tl: (i64, i64),
    br: (i64, i64),
    w: i64,
    h: i64,
) -> Option<String> {
    static PREFIX: OnceLock<Regex> = OnceLock::new();
    static DIMENSION: OnceLock<Regex> = OnceLock::new();
    let prefix = PREFIX
        .get_or_init(|| Regex::new(r"^<g [^>]*>(?:<rect [^>]* />)*").unwrap())
        .find(svg)?
        .as_str();
    let end = svg.rfind("</g>")?;
    if end < prefix.len() {
        return None;
    }
    let left = box_.left.min(0).min(tl.0 - PAD_SIDES);
    let right = box_.right.max(w + PAD_SIDES * 2).max(br.0 + PAD_SIDES);
    let bottom = br.1 + 2 * PAD_TOP + h;
    let width = right - left;
    let height = bottom - box_.top;
    let resized = DIMENSION
        .get_or_init(|| Regex::new(r#" (x|width|height)="(-?[0-9]+)""#).unwrap())
        .replace_all(prefix, |c: &regex::Captures<'_>| {
            let mut n = c[2].parse::<i64>().unwrap_or(0);
            n += match &c[1] {
                "x" => left - box_.left,
                "width" => width - (box_.right - box_.left),
                "height" => height - (box_.bottom - box_.top),
                _ => 0,
            };
            format!(r#" {}="{n}""#, &c[1])
        });
    let resized = regexes().0.replace_all(
        &resized,
        format!(r#"viewBox="{left} {} {width} {height}""#, box_.top),
    );
    Some(format!(
        "{resized}{}{appendix}{}",
        &svg[prefix.len()..end],
        &svg[end..]
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn board(link: &str, pretty: &str) -> serde_json::Value {
        json!({"shapes": [
            {"id":"back", "type":"rectangle", "pos":{"x":0,"y":0}, "width":100,"height":60,
             "opacity":1,"strokeWidth":2,"stroke":"N1","fill":"N7", "link":link,"prettyLink":pretty},
            {"id":"note", "type":"rectangle", "pos":{"x":120,"y":0}, "width":100,"height":60,
             "opacity":1,"strokeWidth":2,"stroke":"N1","fill":"N7", "tooltip":"tooltip for C",
             "link":"https://example.com", "prettyLink":"Example"}
        ]})
    }

    fn numbers(svg: &str) -> Vec<usize> {
        Regex::new(r#"class="appendix-icon">(?:<title>[^<]*</title>)?<circle[^>]*/><text[^>]*>(\d+)</text>"#)
            .unwrap().captures_iter(svg).map(|c| c[1].parse().unwrap()).collect()
    }

    #[test]
    fn mixed_link_tooltip_numbering_uses_one_sequence() {
        // Upstream c71ea9096: root, parent-board link, and CLI-relinked parent.
        let label_re = Regex::new(r#"<text class="text" [^>]*>(.*?)</text>"#).unwrap();
        for (link, pretty, expected) in [
            ("scenarios.one", "one", "one"),
            ("root", "", "root"),
            ("index.svg", "", "index.svg"),
        ] {
            let diagram: TargetDiagram = serde_json::from_value(board(link, pretty)).unwrap();
            let svg = super::super::render::render(&diagram, &Default::default()).unwrap();
            let out = String::from_utf8(
                append(&diagram, &diagram.hash_id(None).unwrap(), &svg, None).unwrap(),
            )
            .unwrap();
            let (markers, rows) = out.split_once("<g class=\"appendix\"").unwrap();
            assert_eq!(numbers(markers), [1, 2, 3]);
            assert_eq!(numbers(rows), [1, 2, 3]);
            let labels: Vec<_> = label_re
                .captures_iter(rows)
                .map(|c| c[1].to_owned())
                .collect();
            assert_eq!(labels, [expected, "tooltip for C", "Example"]);
        }
        let diagram: TargetDiagram =
            serde_json::from_value(board("", "stale pretty link")).unwrap();
        assert_eq!(diagram.shapes_slice()[0].appendix_link_text(), "");
    }

    #[test]
    fn animated_appendices_stay_with_frames_and_expand_canvas() {
        // Upstream 890041df0: empty root, parent link, then a wide footer.
        let tooltip = "A tooltip long enough to make this appendix wider than the diagram";
        let root = json!({"scenarios": [board("root", ""), {"shapes":[{
            "id":"note", "type":"rectangle", "width":100,"height":60,"opacity":1,
            "strokeWidth":2,"stroke":"N1","fill":"N7", "tooltip":tooltip,"link":"https://example.com"
        }]}]});
        let options = crate::RenderOptions {
            force_appendix: true,
            pad: 0.0,
            scale: Some(0.5),
            ..Default::default()
        };
        let output = super::super::render_cli_animated(
            &serde_json::to_vec(&root).unwrap(),
            &options,
            false,
            1000,
        )
        .unwrap();
        let output = String::from_utf8(output).unwrap();
        let doc = roxmltree::Document::parse(&output).unwrap();
        let outer = doc.root_element();
        let canvas = outer.children().find(|n| n.has_tag_name("svg")).unwrap();
        let rect = |n: roxmltree::Node<'_, '_>| -> Vec<i64> {
            n.attribute("viewBox")
                .unwrap()
                .split_whitespace()
                .map(|v| v.parse().unwrap())
                .collect()
        };
        let bounds = rect(canvas);
        assert_eq!(
            outer.attribute("width").unwrap().parse::<i64>().unwrap(),
            (bounds[2] + 1) / 2
        );
        assert_eq!(
            outer.attribute("height").unwrap().parse::<i64>().unwrap(),
            (bounds[3] + 1) / 2
        );
        let frames: Vec<_> = canvas.children().filter(|n| n.has_tag_name("g")).collect();
        assert_eq!(frames.len(), 3);
        for (i, frame) in frames.iter().enumerate() {
            let b = rect(*frame);
            assert!(b[0] >= bounds[0] && b[1] >= bounds[1]);
            assert!(b[0] + b[2] <= bounds[0] + bounds[2] && b[1] + b[3] <= bounds[1] + bounds[3]);
            let bg = frame.children().find(|n| n.has_tag_name("rect")).unwrap();
            for (key, expected) in ["x", "y", "width", "height"].iter().zip(&b) {
                assert_eq!(
                    bg.attribute(*key).unwrap().parse::<i64>().unwrap(),
                    *expected
                );
            }
            let footer = frame
                .children()
                .find(|n| n.attribute("class") == Some("appendix"));
            if i == 0 {
                assert!(footer.is_none());
                continue;
            }
            let footer = footer.unwrap();
            let labels: Vec<_> = footer
                .children()
                .filter(|n| n.has_tag_name("text"))
                .map(|n| {
                    assert!(
                        n.attribute("y").unwrap().parse::<f64>().unwrap() < (b[1] + b[3]) as f64
                    );
                    n.text().unwrap()
                })
                .collect();
            assert_eq!(
                labels,
                if i == 1 {
                    vec!["root", "tooltip for C", "Example"]
                } else {
                    vec![tooltip, "https://example.com"]
                }
            );
            let badges: Vec<_> = frame
                .children()
                .filter(|n| n.attribute("class") == Some("appendix-icon"))
                .flat_map(|n| n.children().filter(|n| n.has_tag_name("text")))
                .map(|n| n.text().unwrap())
                .collect();
            let rows: Vec<_> = footer
                .children()
                .filter(|n| n.has_tag_name("g"))
                .flat_map(|n| n.children().filter(|n| n.has_tag_name("text")))
                .map(|n| n.text().unwrap())
                .collect();
            let expected: Vec<_> = (1..=labels.len()).map(|n| n.to_string()).collect();
            assert_eq!(badges, expected);
            assert_eq!(rows, expected);
        }
        assert!(rect(frames[2])[2] > rect(frames[1])[2]);
        // Without the flag, animation keeps interactive icons and has no footer.
        let options = crate::RenderOptions {
            force_appendix: false,
            ..options
        };
        let out = super::super::render_cli_animated(
            &serde_json::to_vec(&root).unwrap(),
            &options,
            false,
            1000,
        )
        .unwrap();
        assert!(
            !String::from_utf8(out)
                .unwrap()
                .contains("<g class=\"appendix\"")
        );
    }
}
