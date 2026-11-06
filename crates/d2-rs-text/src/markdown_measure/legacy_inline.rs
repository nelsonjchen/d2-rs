// SPDX-License-Identifier: MPL-2.0
// Original D2 v0.9.0 lib/textmeasure/markdown.go:measureMarkdownNode.
// Legacy graph sizing consumes the untouched HTML text-node boundaries.
use super::{MarkdownMeasurer, Size, normalize_html_names, render_html};
use roxmltree::Node;

#[derive(Clone, Copy)]
pub struct Context {
    pub size: f64,
    pub mono: bool,
    pub face: u8,
    pub line_height: f64,
    pub css: bool,
}

pub fn has_data(node: Node<'_, '_>) -> bool {
    if node.is_element() {
        !node.tag_name().name().trim().is_empty()
    } else {
        node.text().is_some_and(|text| !text.trim().is_empty())
    }
}

#[derive(Default, Clone, Copy)]
pub struct BlockAttrs {
    pub size: Size,
    pub top: f64,
    pub bottom: f64,
}

// Preserve the HTML projection, but admit supported HTML void elements into
// the XML-backed measurement tree. Quoted attribute delimiters stay intact.
#[doc(hidden)]
pub fn measurement_html(source: &str) -> String {
    let source = normalize_html_names(source);
    let mut output = String::new();
    let mut cursor = 0;
    let mut html_self_closing = false;
    while let Some(relative) = source[cursor..].find('<') {
        let start = cursor + relative;
        output.push_str(&source[cursor..start]);
        let mut end = start + 1;
        let mut quote = None;
        while end < source.len() {
            let byte = source.as_bytes()[end];
            if let Some(delimiter) = quote {
                if byte == delimiter {
                    quote = None;
                }
            } else if matches!(byte, b'\'' | b'"') {
                quote = Some(byte);
            } else if byte == b'>' {
                break;
            }
            end += 1;
        }
        if end == source.len() {
            output.push_str(&source[start..]);
            return output;
        }
        let tag = &source[start + 1..end];
        let name = tag
            .split(|c: char| c.is_ascii_whitespace() || c == '/')
            .next()
            .unwrap_or("");
        if !name.is_empty()
            && tag.trim_end().ends_with('/')
            && !matches!(
                name,
                "area"
                    | "base"
                    | "br"
                    | "col"
                    | "embed"
                    | "hr"
                    | "img"
                    | "input"
                    | "keygen"
                    | "link"
                    | "meta"
                    | "param"
                    | "source"
                    | "track"
                    | "wbr"
            )
        {
            html_self_closing = true;
        }
        output.push_str(&source[start..end]);
        if matches!(name, "br" | "img" | "hr") && !tag.trim_end().ends_with('/') {
            output.push('/');
        }
        output.push('>');
        cursor = end + 1;
    }
    output.push_str(&source[cursor..]);
    // x/net/html drops whitespace before entering the document body. An
    // artificial XML <body> would retain it as an inline run, swallowing a
    // leading <br> instead of reserving its empty line box.
    let before_body_whitespace = source
        .as_bytes()
        .first()
        .is_some_and(|byte| matches!(byte, b' ' | b'\n' | b'\r' | b'\t' | 12));
    if html_self_closing || before_body_whitespace {
        crate::markdown::html_tree::measurement_fragment(&source)
    } else {
        output
    }
}

impl MarkdownMeasurer<'_> {
    fn legacy_inline_node(&self, node: Node<'_, '_>, mut ctx: Context) -> Option<Size> {
        if node.is_text() {
            let mut text = node.text().unwrap_or_default().to_owned();
            if text.trim_matches(['\n', '\t', '\u{8}']).is_empty() {
                return Some(Size::default());
            }
            let font = match (ctx.mono, ctx.face) {
                (true, 1) => self.mono_bold_font,
                (true, 2) => self.mono_italic_font,
                (true, 3) => self.mono_semibold_font,
                (true, _) => self.mono_font,
                (false, 1) => self.bold_font,
                (false, 2) => self.italic_font,
                (false, 3) => self.semibold_font,
                (false, _) => self.font,
            };
            let code = node
                .parent()
                .is_some_and(|p| p.has_tag_name("code") || p.has_tag_name("pre"));
            let mut spaces = 0.0;
            if !code {
                text = text.replace(['\n', '\t'], " ");
                if text.starts_with(' ') {
                    text.remove(0);
                    if ctx.css || node.prev_siblings().skip(1).any(has_data) {
                        spaces += font.advance(' ', ctx.size);
                    }
                }
                if text.ends_with(' ') {
                    text.pop();
                    if ctx.css || node.next_siblings().skip(1).any(has_data) {
                        spaces += font.advance(' ', ctx.size);
                    }
                }
            }
            let mut metrics = if code {
                font.measure_markdown_raw(&text, ctx.size)
            } else {
                let mono = match ctx.face {
                    1 => self.mono_bold_font,
                    2 => self.mono_italic_font,
                    3 => self.mono_semibold_font,
                    _ => self.mono_font,
                };
                font.measure_markdown_legacy(&text, ctx.size, mono)
            };
            if code {
                let line_factor = if node.parent().is_some_and(|n| n.has_tag_name("pre")) {
                    1.45
                } else {
                    ctx.line_height
                };
                metrics = font.measure_markdown_code(&text, ctx.size, line_factor);
            }
            let heading_code = node.ancestors().any(|p| {
                p.is_element()
                    && p.tag_name()
                        .name()
                        .strip_prefix('h')
                        .is_some_and(|v| matches!(v, "1" | "2" | "3" | "4" | "5" | "6"))
            });
            let scale = if code && !(ctx.css && heading_code) {
                0.85
            } else {
                1.0
            };
            return Some(Size {
                width: if ctx.css {
                    font.measure_markdown_css_advance(&text, ctx.size) * scale + spaces
                } else {
                    metrics.width * scale + spaces
                },
                height: if text.is_empty() {
                    0.0
                } else {
                    metrics.height * scale
                },
            });
        }
        if !node.is_element() {
            return Some(Size::default());
        }
        let tag = node.tag_name().name();
        if matches!(
            tag,
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
        ) {
            return self.legacy_block_node(node, ctx).map(|a| a.size);
        }
        if tag == "table" {
            return self.legacy_table_node(node, ctx);
        }
        if matches!(tag, "td" | "th") {
            if tag == "th" {
                ctx.face = 3;
            }
            // Historical measureMarkdownNode deliberately replaces inline
            // aggregation in cells: maximum child width, summed child heights.
            let mut measured = Size::default();
            for child in node.children() {
                let child = self.legacy_inline_node(child, ctx)?;
                measured.width = measured.width.max(child.width);
                measured.height += child.height;
            }
            if measured.height > 0.0 {
                measured.height = measured.height.max(ctx.size * ctx.line_height);
            }
            return Some(measured);
        }
        if !matches!(
            tag,
            "p" | "a"
                | "span"
                | "b"
                | "strong"
                | "em"
                | "i"
                | "s"
                | "strike"
                | "del"
                | "code"
                | "img"
                | "br"
        ) {
            return None;
        }
        match tag {
            "strong" | "b" => ctx.face = 1,
            "em" => ctx.face = 2,
            "code" => {
                ctx.mono = true;
                ctx.face = 0;
            }
            _ => {}
        }
        let code = tag == "code";
        let line_height = ctx.size * ctx.line_height;
        let mut block = Size::default();
        let mut inline = Size::default();
        let mut has_inline = false;
        let finish = |block: &mut Size, inline: &mut Size, has_inline: &mut bool| {
            if *has_inline {
                block.width = block.width.max(inline.width);
                block.height += if !code && inline.height > 0.0 {
                    if ctx.css {
                        line_height
                    } else {
                        inline.height.max(line_height)
                    }
                } else {
                    inline.height
                };
                *inline = Size::default();
                *has_inline = false;
            }
        };
        for child in node.children() {
            if child.has_tag_name("br") {
                if has_inline {
                    finish(&mut block, &mut inline, &mut has_inline);
                } else {
                    block.height += line_height;
                }
            } else {
                let measured = self.legacy_inline_node(child, ctx)?;
                if measured != Size::default() {
                    inline.width += measured.width;
                    inline.height = inline.height.max(measured.height);
                    has_inline = true;
                }
            }
        }
        finish(&mut block, &mut inline, &mut has_inline);
        if code && !node.parent().is_some_and(|n| n.has_tag_name("pre")) {
            block.width += 2.0 * 0.4 * ctx.size;
            block.height += 2.0 * 0.2 * ctx.size;
        }
        if !ctx.css && block.height > 0.0 {
            block.height = block.height.max(line_height);
        }
        Some(block)
    }

    fn legacy_table_node(&self, table: Node<'_, '_>, ctx: Context) -> Option<Size> {
        let mut widths: Vec<f64> = Vec::new();
        let mut height = 0.0;
        for child in table.children().filter(Node::is_element) {
            let rows: Vec<_> = match child.tag_name().name() {
                "thead" | "tbody" | "tfoot" => {
                    child.children().filter(|n| n.has_tag_name("tr")).collect()
                }
                "tr" => vec![child],
                _ => continue,
            };
            for row in rows {
                let header = row.ancestors().any(|n| n.has_tag_name("thead"));
                let mut row_height: f64 = 0.0;
                for (column, cell) in row
                    .children()
                    .filter(|n| n.has_tag_name("td") || n.has_tag_name("th"))
                    .enumerate()
                {
                    let mut cell_ctx = ctx;
                    if header || cell.has_tag_name("th") {
                        cell_ctx.face = 3;
                    }
                    let measured = self.legacy_inline_node(cell, cell_ctx)?;
                    let width = measured.width + 26.0;
                    if column == widths.len() {
                        widths.push(width);
                    } else {
                        widths[column] = widths[column].max(width);
                    }
                    row_height = row_height.max(measured.height + 12.0);
                }
                height += (row_height + 1.0).max(ctx.size * ctx.line_height);
            }
        }
        Some(Size {
            width: if widths.is_empty() {
                0.0
            } else {
                widths.iter().sum::<f64>() + widths.len() as f64 + 1.0
            },
            height: (height + 2.0).max(ctx.size * ctx.line_height),
        })
    }

    // Original measureMarkdownNode keeps dimensions and collapsed margins
    // independently until the enclosing HTML block consumes them.
    pub fn legacy_block_node(&self, node: Node<'_, '_>, mut ctx: Context) -> Option<BlockAttrs> {
        let tag = if node.is_element() {
            node.tag_name().name()
        } else {
            ""
        };
        if tag != "pre"
            && !matches!(
                tag,
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
            )
        {
            return self.legacy_inline_node(node, ctx).map(|size| BlockAttrs {
                size,
                bottom: if ctx.css && tag == "table" { 16.0 } else { 0.0 },
                ..BlockAttrs::default()
            });
        }
        if let Some(level) = tag
            .strip_prefix('h')
            .and_then(|v| v.parse::<usize>().ok())
            .filter(|v| (1..=6).contains(v))
        {
            ctx.size *= [2.0, 1.5, 1.25, 1.0, 0.875, 0.85][level - 1];
            if !ctx.css {
                ctx.size = ctx.size.trunc();
            }
            ctx.face = 3;
            ctx.line_height = 1.25;
        }
        if tag == "pre" {
            ctx.mono = true;
            ctx.face = 0;
        }
        let line_height = ctx.size * ctx.line_height;
        let mut result = BlockAttrs::default();
        let mut blocks = Vec::new();
        let mut inline: Option<BlockAttrs> = None;
        let children = node.children().collect::<Vec<_>>();
        let first = children.iter().copied().find(|n| has_data(*n));
        let last = children.iter().copied().rev().find(|n| has_data(*n));
        let flush = |blocks: &mut Vec<BlockAttrs>, inline: &mut Option<BlockAttrs>| {
            if let Some(mut a) = inline.take() {
                if tag != "pre" && a.size.height > 0.0 {
                    a.size.height = if ctx.css {
                        if tag == "pre" {
                            a.size.height
                        } else {
                            line_height
                        }
                    } else {
                        a.size.height.max(line_height)
                    };
                }
                blocks.push(a);
            }
        };
        for child in children {
            let mut attrs = self.legacy_block_node(child, ctx)?;
            let child_tag = if child.is_element() {
                child.tag_name().name()
            } else {
                ""
            };
            if matches!(
                child_tag,
                "blockquote"
                    | "div"
                    | "p"
                    | "h1"
                    | "h2"
                    | "h3"
                    | "h4"
                    | "h5"
                    | "h6"
                    | "hr"
                    | "li"
                    | "ul"
                    | "ol"
                    | "pre"
                    | "table"
                    | "thead"
                    | "tbody"
                    | "tr"
                    | "td"
                    | "th"
            ) {
                flush(&mut blocks, &mut inline);
                if tag == "blockquote" {
                    if Some(child) == first {
                        attrs.top = 0.0;
                    }
                    if Some(child) == last {
                        attrs.bottom = 0.0;
                    }
                }
                blocks.push(attrs);
            } else if child_tag == "br" {
                if inline.is_some() {
                    flush(&mut blocks, &mut inline);
                } else {
                    result.size.height += line_height;
                }
            } else if attrs.size != Size::default() {
                if let Some(a) = &mut inline {
                    a.size.width += attrs.size.width;
                    a.size.height = a.size.height.max(attrs.size.height);
                    a.top = a.top.max(attrs.top);
                    a.bottom = a.bottom.max(attrs.bottom);
                } else {
                    inline = Some(attrs);
                }
            }
        }
        flush(&mut blocks, &mut inline);
        let mut previous_bottom = 0.0;
        for (i, a) in blocks.iter().enumerate() {
            if i == 0 {
                result.top = result.top.max(a.top);
            } else {
                let difference = a.top - previous_bottom;
                if difference > 0.0 {
                    result.size.height += difference;
                }
            }
            if i == blocks.len() - 1 {
                result.bottom = result.bottom.max(a.bottom);
            } else {
                result.size.height += a.bottom;
                previous_bottom = a.bottom;
            }
            result.size.height += a.size.height;
            result.size.width = result.size.width.max(a.size.width);
        }
        match tag {
            "pre" => {
                let text = node
                    .descendants()
                    .filter(|&n| n.is_text())
                    .map(|n| n.text().unwrap_or_default())
                    .collect::<String>();
                let count = if text.is_empty() {
                    0
                } else {
                    text.split('\n').count() - usize::from(text.ends_with('\n'))
                };
                if ctx.css {
                    result.size.height = result
                        .size
                        .height
                        .max(count as f64 * ctx.size * 0.85 * 1.45);
                }
                result.size.height += 32.0;
                result.size.width += 32.0;
                result.bottom = result.bottom.max(16.0);
            }
            "blockquote" => {
                result.size.width += 2.0 * ctx.size + 0.25 * ctx.size;
                result.bottom = result.bottom.max(16.0);
            }
            "p" => {
                if node.parent().is_some_and(|n| n.has_tag_name("li")) {
                    result.top = result.top.max(16.0);
                }
                result.bottom = result.bottom.max(16.0);
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                result.top = result.top.max(24.0);
                result.bottom = result.bottom.max(16.0);
                if matches!(tag, "h1" | "h2") {
                    result.size.height += 0.3 * ctx.size + 1.0;
                }
            }
            "li" => {
                result.size.width += 2.0 * ctx.size;
                if ctx.css && result.size.height == 0.0 {
                    result.size.height = line_height;
                }
                if node.prev_siblings().skip(1).any(has_data) {
                    result.top = result.top.max(0.25 * ctx.size);
                }
            }
            "ul" | "ol" => {
                if node
                    .ancestors()
                    .skip(1)
                    .any(|n| n.has_tag_name("ul") || n.has_tag_name("ol"))
                {
                    result.top = 0.0;
                    result.bottom = 0.0;
                } else {
                    result.bottom = result.bottom.max(16.0);
                }
            }
            "hr" => {
                result.size.height += 0.25 * ctx.size;
                result.top = result.top.max(24.0);
                result.bottom = result.bottom.max(24.0);
            }
            _ => (),
        }
        if !ctx.css && result.size.height > 0.0 {
            result.size.height = result.size.height.max(line_height);
        }
        Some(result)
    }

    pub(super) fn legacy_paragraph_dimensions(
        &self,
        source: &str,
        font_size: u32,
        mono: bool,
    ) -> Option<Size> {
        self.legacy_html_dimensions(&render_html(source.as_bytes()), font_size, mono)
    }

    #[doc(hidden)]
    pub fn legacy_html_dimensions(&self, html: &str, font_size: u32, mono: bool) -> Option<Size> {
        let html = format!("<body>{}</body>", measurement_html(html));
        let document = roxmltree::Document::parse(&html).ok()?;
        let body = document.root_element();
        let ctx = Context {
            size: f64::from(font_size),
            mono,
            face: 0,
            line_height: 1.5,
            css: false,
        };
        let dimensions = self.legacy_block_node(body, ctx)?.size;
        Some(Size {
            width: dimensions.width.ceil(),
            height: dimensions.height.ceil(),
        })
    }
}
