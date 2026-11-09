// SPDX-License-Identifier: MPL-2.0
// Original D2 v0.9.0 markdown_svg.go paintBlock/paintFlow/flowUnits.
use super::*;
use markdown_measure::legacy_inline::{BlockAttrs, Context, has_data};
use roxmltree::Node;

#[derive(Clone, Copy)]
struct PaintContext {
    size: u32,
    family: MarkdownFamily,
    muted: bool,
}
struct Painter<'a> {
    html: &'a str,
    fill: &'a str,
    class: Option<&'a str>,
    precision: bool,
    viewport: Size,
}
fn block(n: Node<'_, '_>) -> bool {
    n.is_element()
        && matches!(
            n.tag_name().name(),
            "body"
                | "div"
                | "blockquote"
                | "p"
                | "h1"
                | "h2"
                | "h3"
                | "h4"
                | "h5"
                | "h6"
                | "ul"
                | "ol"
                | "li"
                | "hr"
                | "pre"
                | "table"
                | "thead"
                | "tbody"
                | "tfoot"
                | "tr"
                | "td"
                | "th"
        )
}
fn level(n: Node<'_, '_>) -> Option<usize> {
    n.is_element()
        .then(|| {
            n.tag_name()
                .name()
                .strip_prefix('h')
                .and_then(|s| s.parse().ok())
        })
        .flatten()
        .filter(|v| (1..=6).contains(v))
}
fn inner(n: Node<'_, '_>, html: &str) -> String {
    n.children().map(|c| html[c.range()].to_owned()).collect()
}
// Existing inline painters return a viewport wrapper. The document owns that
// viewport once; child primitives retain global coordinates and paint order.
fn primitives(svg: &str) -> Option<&str> {
    let begin = svg.find('>')? + 1;
    let content = svg.get(begin..svg.len().checked_sub("</svg>".len())?)?;
    let group = content.find('>')? + 1;
    content.get(group..content.len().checked_sub("</g>".len())?)
}
fn ordered_marker(mut index: i64, style: &str) -> String {
    let text = if index <= 0 {
        index.to_string()
    } else {
        match style.to_ascii_lowercase().as_str() {
            "a" => {
                let mut bytes = Vec::new();
                while index > 0 {
                    index -= 1;
                    bytes.push(b'a' + (index % 26) as u8);
                    index /= 26;
                }
                bytes.reverse();
                String::from_utf8(bytes).unwrap()
            }
            "i" => {
                let mut out = String::new();
                for (value, letters) in [
                    (1000, "m"),
                    (900, "cm"),
                    (500, "d"),
                    (400, "cd"),
                    (100, "c"),
                    (90, "xc"),
                    (50, "l"),
                    (40, "xl"),
                    (10, "x"),
                    (9, "ix"),
                    (5, "v"),
                    (4, "iv"),
                    (1, "i"),
                ] {
                    while index >= value {
                        out.push_str(letters);
                        index -= value;
                    }
                }
                out
            }
            _ => index.to_string(),
        }
    };
    format!("{text}.")
}
impl Painter<'_> {
    fn colors(&self, ctx: PaintContext) -> (&str, Option<&str>) {
        if ctx.muted {
            ("#6F7E8B", Some("fill-N2"))
        } else {
            (self.fill, self.class)
        }
    }
    fn attrs(&self, n: Node<'_, '_>, ctx: PaintContext) -> Option<BlockAttrs> {
        let fonts = native_markdown_measurer(ctx.family)?;
        let measurer = fonts.measurer();
        measurer.legacy_block_node(
            n,
            Context {
                size: f64::from(ctx.size),
                mono: ctx.family.uses_mono_css(),
                face: 0,
                line_height: 1.5,
                css: true,
            },
        )
    }
    fn inline(
        &self,
        svg: &mut String,
        nodes: &[Node<'_, '_>],
        origin: Point,
        width: f64,
        ctx: PaintContext,
    ) -> Option<f64> {
        let markup = nodes
            .iter()
            .map(|n| self.html[n.range()].to_owned())
            .collect::<String>();
        let runs = native_markdown_html_inline(&format!("<p>{markup}</p>\n"))?;
        if runs.is_empty() {
            return Some(0.0);
        }
        let (fill, class) = self.colors(ctx);
        let mut child = String::new();
        let mut height = 0.0;
        // Inline origin is global. The painter interprets width as the right edge.
        if !draw_native_markdown_inline_context(
            &mut child,
            &split_native_markdown_lines(&runs),
            Rect {
                origin: Point::default(),
                size: Size {
                    width: origin.x + width,
                    height: self.viewport.height,
                },
            },
            ctx.size,
            fill,
            class,
            self.precision,
            ctx.family,
            origin,
            false,
            "",
            Some(&mut height),
        ) {
            return None;
        }
        svg.push_str(primitives(&child)?);
        Some(height)
    }
    fn flush<'d, 'i>(
        &self,
        units: &mut Vec<(Option<Node<'d, 'i>>, Vec<Node<'d, 'i>>, BlockAttrs)>,
        inline: &mut Vec<Node<'d, 'i>>,
        ctx: PaintContext,
    ) -> Option<()> {
        if inline.is_empty() {
            return Some(());
        }
        let mut attrs = BlockAttrs::default();
        for child in inline.iter() {
            let a = self.attrs(*child, ctx)?;
            attrs.size.width += a.size.width;
            attrs.size.height = attrs.size.height.max(a.size.height);
            attrs.top = attrs.top.max(a.top);
            attrs.bottom = attrs.bottom.max(a.bottom);
        }
        if attrs.size != Size::default() {
            attrs.size.height = f64::from(ctx.size) * 1.5;
            units.push((None, std::mem::take(inline), attrs));
        } else {
            inline.clear();
        }
        Some(())
    }
    fn flow(
        &self,
        svg: &mut String,
        n: Node<'_, '_>,
        origin: Point,
        width: f64,
        ctx: PaintContext,
    ) -> Option<f64> {
        let children = n.children().collect::<Vec<_>>();
        let first = children.iter().copied().find(|n| has_data(*n));
        let last = children.iter().copied().rev().find(|n| has_data(*n));
        let mut units: Vec<(Option<Node<'_, '_>>, Vec<Node<'_, '_>>, BlockAttrs)> = Vec::new();
        let mut inline = Vec::new();
        for child in children {
            if block(child) {
                self.flush(&mut units, &mut inline, ctx)?;
                let mut a = self.attrs(child, ctx)?;
                if n.has_tag_name("blockquote") {
                    if Some(child) == first {
                        a.top = 0.0;
                    }
                    if Some(child) == last {
                        a.bottom = 0.0;
                    }
                }
                units.push((Some(child), Vec::new(), a));
            } else if child.has_tag_name("br") {
                // normalizeMarkdownWhitespaceTree clears collapsible text
                // after a break before paintFlow groups inline nodes. Without
                // that step, the source newline between consecutive <br>s is
                // mistaken for a nonempty unit and consumes one empty line.
                if inline
                    .iter()
                    .all(|node| node.is_text() && node.text().unwrap_or_default().trim().is_empty())
                {
                    inline.clear();
                }
                if inline.is_empty() {
                    units.push((
                        None,
                        Vec::new(),
                        BlockAttrs {
                            size: Size {
                                width: 0.0,
                                height: f64::from(ctx.size) * 1.5,
                            },
                            ..Default::default()
                        },
                    ));
                } else {
                    self.flush(&mut units, &mut inline, ctx)?;
                }
            } else if child.is_text() || child.is_element() {
                inline.push(child);
            }
        }
        self.flush(&mut units, &mut inline, ctx)?;
        let mut y = origin.y;
        let mut previous: f64 = 0.0;
        for (i, (child, nodes, a)) in units.iter().enumerate() {
            if i == 0
                && n.has_tag_name("body")
                && child.is_some_and(|c| c.has_tag_name("ul") || c.has_tag_name("ol"))
            {
                y += a.top;
            } else if i > 0 {
                y += previous.max(a.top);
            }
            let height = if let Some(child) = child {
                self.paint(svg, *child, Point { x: origin.x, y }, width, ctx)?
            } else {
                a.size
                    .height
                    .max(self.inline(svg, nodes, Point { x: origin.x, y }, width, ctx)?)
            };
            y += height;
            previous = a.bottom;
        }
        Some(y - origin.y)
    }
    fn marker(
        &self,
        svg: &mut String,
        n: Node<'_, '_>,
        origin: Point,
        ctx: PaintContext,
        inset: f64,
    ) -> Option<()> {
        let fonts = native_markdown_measurer(ctx.family)?;
        let m = fonts.measurer();
        let size = f64::from(ctx.size);
        let (a, d) = m.font.css_box_metrics(size);
        let mut baseline = origin.y + (size * 1.5 - a - d).max(0.0) / 2.0 + a;
        if let Some(first) = n.children().find(|n| has_data(*n)) {
            if let Some(l) = level(first) {
                let hs = size * [2.0, 1.5, 1.25, 1.0, 0.875, 0.85][l - 1];
                let (a, d) = m.semibold_font.css_box_metrics(hs);
                baseline = baseline.max(origin.y + (hs * 1.25 - a - d).max(0.0) / 2.0 + a);
            }
        }
        let (fill, class) = self.colors(ctx);
        if n.parent().is_some_and(|p| p.has_tag_name("ul")) {
            let side = native_markdown_snap(0.3 * size).max(3.0);
            let x = native_markdown_snap(origin.x + inset - size / 2.0 - 6.5 - side / 2.0);
            let depth = n
                .ancestors()
                .skip(2)
                .filter(|p| p.has_tag_name("ul") || p.has_tag_name("ol"))
                .count();
            let color_class = class.unwrap_or("");
            let stroke_class = color_class
                .replace("d2-rs-text-", "d2-rs-stroke-")
                .replace("fill-", "stroke-");
            let paint = if depth == 1 {
                format!(
                    "fill=\"none\" stroke=\"{}\" stroke-width=\"1.000\" class=\"{}\"",
                    escape(fill),
                    escape(&stroke_class)
                )
            } else {
                format!(
                    "fill=\"{}\" class=\"{}\"",
                    escape(fill),
                    escape(color_class)
                )
            };
            let _ = write!(
                svg,
                "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"{}\" {}/>",
                native_markdown_number(x),
                native_markdown_text_number(baseline - (1.5 * side).floor(), self.precision),
                native_markdown_number(side),
                native_markdown_number(side),
                native_markdown_number(if depth <= 1 { side / 2.0 } else { 0.0 }),
                paint
            );
        } else {
            let parent = n.parent()?;
            let start = parent
                .attribute("start")
                .and_then(|s| s.parse::<i64>().ok())
                .unwrap_or(1);
            let index = n
                .prev_siblings()
                .skip(1)
                .filter(|s| s.has_tag_name("li"))
                .count() as i64;
            let depth = n
                .ancestors()
                .skip(2)
                .filter(|p| p.has_tag_name("ul") || p.has_tag_name("ol"))
                .count();
            let style = parent.attribute("type").unwrap_or(if depth == 0 {
                "1"
            } else if depth == 1 {
                "i"
            } else {
                "a"
            });
            let marker = ordered_marker(start + index, style);
            let width = m.font.measure_with_dot(&marker, size).width;
            let mut runs = parse_native_markdown_inline(&marker)?;
            let mut run = runs.remove(0);
            run.primary_mono = ctx.family.uses_mono_css();
            native_markdown_text(
                svg,
                origin.x + inset - width - m.font.measure_advance(" ", size),
                baseline,
                &run,
                size,
                fill,
                class.unwrap_or(""),
                self.precision,
            );
        }
        Some(())
    }
    fn paint(
        &self,
        svg: &mut String,
        n: Node<'_, '_>,
        origin: Point,
        mut width: f64,
        mut ctx: PaintContext,
    ) -> Option<f64> {
        let attrs = self.attrs(n, ctx)?;
        let tag = n.tag_name().name();
        let size = f64::from(ctx.size);
        if tag == "blockquote" {
            ctx.muted = true;
        }
        let (fill, class) = self.colors(ctx);
        if tag == "hr" {
            let x = native_markdown_snap(origin.x);
            let y = native_markdown_snap(origin.y);
            let w =
                native_markdown_snap(origin.x + if width > 0.0 { width } else { attrs.size.width })
                    - x;
            let h = native_markdown_snap(origin.y + 0.25 * size) - y;
            let _ = write!(
                svg,
                "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"0.000\" fill=\"#0D32B2\" class=\"fill-B1\"/>",
                native_markdown_number(x),
                native_markdown_number(y),
                native_markdown_number(w),
                native_markdown_number(h)
            );
            return Some(attrs.size.height);
        }
        if tag == "table" {
            return markdown_table::paint_node(
                svg,
                n,
                self.html,
                origin,
                width,
                ctx.size,
                fill,
                class,
                self.precision,
                ctx.family,
            );
        }
        if let Some(l) = level(n) {
            // Go paintBlock still paints an h1/h2 rule when the heading has
            // no inline children. An empty run list is success here rather
            // than a signal to fall back to the plain-paragraph painter.
            let heading = inner(n, self.html);
            let runs = native_markdown_html_inline(&format!("<p>{heading}</p>\n"))?;
            let mut child = String::new();
            let mut height = 0.0;
            if !draw_native_markdown_heading_at_origin(
                &mut child,
                &runs,
                l,
                Rect {
                    origin: Point::default(),
                    size: Size {
                        width,
                        height: self.viewport.height,
                    },
                },
                ctx.size,
                fill,
                class,
                self.precision,
                ctx.family,
                Some(&mut height),
                origin,
            ) {
                return None;
            }
            svg.push_str(primitives(&child)?);
            return Some(height);
        }
        if tag == "pre" {
            let text = n
                .descendants()
                .filter(|&n| n.is_text())
                .map(|n| n.text().unwrap_or_default())
                .collect::<String>();
            let mut lines = if text.is_empty() {
                Vec::new()
            } else {
                text.split('\n').collect::<Vec<_>>()
            };
            if text.ends_with('\n') {
                lines.pop();
            }
            let fonts = native_markdown_measurer(ctx.family)?;
            let mono = fonts.measurer().mono_font;
            let font_size = 0.85 * size;
            let line_height = 1.45 * font_size;
            let height = lines.len() as f64 * line_height + 32.0;
            let _ = write!(
                svg,
                "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"6.000\" fill=\"#EEF1F8\" class=\"fill-N6\"/>",
                native_markdown_text_number(origin.x, self.precision),
                native_markdown_text_number(origin.y, self.precision),
                native_markdown_text_number(attrs.size.width.max(width), self.precision),
                native_markdown_text_number(height, self.precision)
            );
            let (a, d) = mono.css_box_metrics(font_size);
            let leading = (line_height - a - d).max(0.0);
            for (i, line) in lines.iter().enumerate() {
                if line.is_empty() {
                    continue;
                }
                let mut expanded = String::new();
                let mut column = 0;
                use unicode_segmentation::UnicodeSegmentation;
                for grapheme in line.graphemes(true) {
                    if grapheme == "\t" {
                        let spaces = 4 - column % 4;
                        expanded.push_str(&" ".repeat(spaces));
                        column += spaces;
                    } else {
                        expanded.push_str(grapheme);
                        column += d2_rs_text::uniseg::string_width(grapheme).max(1) as usize;
                    }
                }
                let mut run = parse_native_markdown_inline("x")?.remove(0);
                run.text = expanded;
                run.primary_mono = true;
                native_markdown_text(
                    svg,
                    origin.x + 16.0,
                    (origin.y + 16.0 + i as f64 * line_height) + leading / 2.0 + a,
                    &run,
                    font_size,
                    fill,
                    class.unwrap_or(""),
                    self.precision,
                );
            }
            return Some(height);
        }
        let mut content = origin;
        let mut border = 0.0;
        if tag == "blockquote" {
            border = (0.25 * size).floor().max(1.0);
            let inset = border + size;
            content.x += inset;
            width = (width - inset - size).max(0.0);
        }
        if tag == "li" {
            let inset = 2.0 * size;
            content.x += inset;
            width = (width - inset).max(0.0);
            self.marker(svg, n, origin, ctx, inset)?;
        }
        let painted = attrs
            .size
            .height
            .max(self.flow(svg, n, content, width, ctx)?);
        if tag == "blockquote" {
            let y = native_markdown_snap(origin.y);
            let h = native_markdown_snap(origin.y + painted) - y;
            let _ = write!(
                svg,
                "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"0.000\" fill=\"#0D32B2\" class=\"fill-B1\"/>",
                native_markdown_number(native_markdown_snap(origin.x)),
                native_markdown_number(y),
                native_markdown_number(border),
                native_markdown_number(h)
            );
        }
        Some(painted)
    }
}
pub(super) fn draw(
    svg: &mut String,
    text: &str,
    rect: Rect,
    font_size: u32,
    fill: &str,
    class: Option<&str>,
    precision: bool,
    family: MarkdownFamily,
) -> bool {
    let html = format!(
        "<body>{}</body>",
        markdown_measure::legacy_inline::measurement_html(&render_html(text.as_bytes()))
    );
    let Ok(document) = roxmltree::Document::parse(&html) else {
        return false;
    };
    let painter = Painter {
        html: &html,
        fill,
        class,
        precision,
        viewport: rect.size,
    };
    let mut content = String::new();
    if painter
        .paint(
            &mut content,
            document.root_element(),
            Point::default(),
            rect.size.width,
            PaintContext {
                size: font_size,
                family,
                muted: false,
            },
        )
        .is_none()
    {
        return false;
    }
    let _ = write!(
        svg,
        "<svg x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\" overflow=\"hidden\"><g class=\"md md-native{}\">{}</g></svg>",
        number(rect.origin.x),
        number(rect.origin.y),
        number(rect.size.width),
        number(rect.size.height),
        number(rect.size.width),
        number(rect.size.height),
        if family.uses_mono_css() {
            " md-primary-mono"
        } else {
            ""
        },
        content
    );
    true
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod source_contract {
    use super::*;
    #[test]
    fn complete_original_recursive_block_primitives() {
        let rows: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/markdown-recursive-paint-source-v090.json"
        ))
        .unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 864);
        let mut outcomes = Vec::new();
        let mut failures = Vec::new();
        for row in rows.as_array().unwrap() {
            let r = &row["request"];
            if row.get("error").is_some() {
                let error = first_unsupported_markdown_html(r["Markdown"].as_str().unwrap())
                    .map(|element| RenderError::UnsupportedMarkdownHtml { element }.to_string())
                    .unwrap_or_default();
                assert_eq!(error, row["error"].as_str().unwrap());
                outcomes.push(serde_json::json!({"request":r,"error":error}));
                continue;
            }
            let family = match r["Family"].as_str().unwrap() {
                "mono" => MarkdownFamily::Mono,
                "hand" => MarkdownFamily::HandDrawn,
                _ => MarkdownFamily::Sans,
            };
            let mut svg = String::new();
            let supported = draw(
                &mut svg,
                r["Markdown"].as_str().unwrap(),
                Rect {
                    origin: Point::default(),
                    size: Size {
                        width: row["layout"]["Width"].as_f64().unwrap(),
                        height: row["layout"]["Height"].as_f64().unwrap(),
                    },
                },
                r["FontSize"].as_u64().unwrap() as u32,
                "#0A0F25",
                None,
                false,
                family,
            );
            let actual = markdown_wrap::source_contract::primitives(&svg);
            let expected = markdown_wrap::source_contract::primitives(row["svg"].as_str().unwrap());
            let html = render_html(r["Markdown"].as_str().unwrap().as_bytes());
            let html_matched = html == row["rendered_html"].as_str().unwrap();
            let matched = supported && actual == expected && html_matched;
            if !matched {
                failures.push(r["Name"].clone());
            }
            outcomes.push(serde_json::json!({"request":r,"supported":supported,"matched":matched,"actual":actual,"expected":expected,"html":html,"html_matched":html_matched,"source_html":row["rendered_html"]}));
        }
        if let Ok(path) = std::env::var("D2_RS_TEST_RECURSIVE_BLOCK_OUTPUT") {
            std::fs::write(path, serde_json::to_vec(&outcomes).unwrap()).unwrap();
        }
        assert!(
            failures.is_empty(),
            "recursive original block differences: {failures:?}"
        );
    }
}
