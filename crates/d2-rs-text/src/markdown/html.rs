// Copyright (c) 2019 Yusuke Inuzuka
// SPDX-License-Identifier: MPL-2.0 AND MIT
//! Deterministic HTML projection for the translated Goldmark-shaped Markdown
//! tree. The projection is intentionally independent of a browser or a
//! JavaScript runtime so native SVG targets and WASM callers share it.

use super::{
    arena::NodeId,
    bytes::Bytes,
    context::{Context, NodeKind},
    text::Segment,
};
use std::fmt::Write;

fn entity(name: &[u8]) -> Option<&'static str> {
    super::entities::ENTITIES
        .binary_search_by(|(key, _)| key.cmp(&name))
        .ok()
        .map(|index| super::entities::ENTITIES[index].1)
}

/// Decodes HTML5 character references using the Go 1.27 table shared by D2.
///
/// Keeping this at the Markdown boundary is important: Goldmark resolves
/// references before native text measurement, while URL import validation
/// needs the same decoding rules. Unknown references remain byte-for-byte
/// unchanged and malformed numeric values use U+FFFD like Go's
/// `html.UnescapeString`.
pub fn unescape_html(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len());
    let mut start = 0;
    while start < input.len() {
        if input[start] != b'&' {
            out.push(input[start]);
            start += 1;
            continue;
        }
        let source = &input[start..];
        let mut index = 1;
        if source.len() <= 1 || (source[1] == b'#' && source.len() <= 3) {
            out.push(b'&');
            start += 1;
            continue;
        }
        if source[1] == b'#' {
            index = 2;
            let hexadecimal = matches!(source[index], b'x' | b'X');
            if hexadecimal {
                index += 1;
            }
            let mut value = 0i32;
            while index < source.len() {
                let character = source[index];
                index += 1;
                let digit = if character.is_ascii_digit() {
                    Some(character - b'0')
                } else if hexadecimal && character.is_ascii_hexdigit() {
                    Some(character.to_ascii_lowercase() - b'a' + 10)
                } else {
                    None
                };
                if let Some(digit) = digit {
                    value = value
                        .wrapping_mul(if hexadecimal { 16 } else { 10 })
                        .wrapping_add(digit as i32);
                    continue;
                }
                if character != b';' {
                    index -= 1;
                }
                break;
            }
            if index <= 3 {
                out.push(b'&');
                start += 1;
                continue;
            }
            const REPLACE: [u32; 32] = [
                0x20ac, 0x81, 0x201a, 0x192, 0x201e, 0x2026, 0x2020, 0x2021, 0x2c6, 0x2030, 0x160,
                0x2039, 0x152, 0x8d, 0x17d, 0x8f, 0x90, 0x2018, 0x2019, 0x201c, 0x201d, 0x2022,
                0x2013, 0x2014, 0x2dc, 0x2122, 0x161, 0x203a, 0x153, 0x9d, 0x17e, 0x178,
            ];
            let value = if (0x80..=0x9f).contains(&value) {
                REPLACE[(value - 0x80) as usize]
            } else if value == 0 {
                0xfffd
            } else {
                value as u32
            };
            let character = char::from_u32(value).unwrap_or('\u{fffd}');
            let mut bytes = [0; 4];
            out.extend_from_slice(character.encode_utf8(&mut bytes).as_bytes());
            start += index;
            continue;
        }
        while index < source.len() {
            let character = source[index];
            index += 1;
            if character.is_ascii_alphanumeric() {
                continue;
            }
            if character != b';' {
                index -= 1;
            }
            break;
        }
        let name = &source[1..index];
        if let Some(value) = entity(name) {
            out.extend_from_slice(value.as_bytes());
            start += index;
            continue;
        }
        let prefix = (2..=name.len().saturating_sub(1).min(6))
            .rev()
            .find_map(|length| entity(&name[..length]).map(|value| (length, value)));
        if let Some((length, value)) = prefix {
            out.extend_from_slice(value.as_bytes());
            start += length + 1;
        } else {
            out.extend_from_slice(&source[..index]);
            start += index;
        }
    }
    out
}

// HTML source is a Go byte string; only generated markup is guaranteed UTF-8.
#[derive(Default)]
struct HtmlBytes(Vec<u8>);
impl HtmlBytes {
    fn push(&mut self, character: char) {
        self.0
            .extend_from_slice(character.encode_utf8(&mut [0; 4]).as_bytes());
    }
    fn push_str(&mut self, text: &str) {
        self.0.extend_from_slice(text.as_bytes());
    }
    fn push_bytes(&mut self, bytes: &[u8]) {
        self.0.extend_from_slice(bytes);
    }
}
impl std::fmt::Write for HtmlBytes {
    fn write_str(&mut self, text: &str) -> std::fmt::Result {
        self.push_str(text);
        Ok(())
    }
}
fn escape_text(value: &[u8], output: &mut HtmlBytes) {
    // Goldmark defaultWriter.Write removes escaped punctuation while keeping
    // an escaped ampersand out of entity-reference resolution.
    let mut start = 0;
    let mut at = 0;
    while at < value.len() {
        if value[at] == b'\\' && value.get(at + 1).is_some_and(u8::is_ascii_punctuation) {
            escape_raw(&unescape_html(&value[start..at]), output);
            escape_raw(&value[at + 1..at + 2], output);
            at += 2;
            start = at;
        } else {
            at += 1;
        }
    }
    escape_raw(&unescape_html(&value[start..]), output);
}

fn escape_raw(value: &[u8], output: &mut HtmlBytes) {
    for &byte in value {
        match byte {
            0 => output.push('\u{fffd}'),
            b'&' => output.push_str("&amp;"),
            b'<' => output.push_str("&lt;"),
            b'>' => output.push_str("&gt;"),
            b'"' => output.push_str("&quot;"),
            _ => output.0.push(byte),
        }
    }
}

fn escape_attr(value: &[u8], output: &mut HtmlBytes) {
    escape_text(value, output);
}

fn segment_text(segment: Segment, source: &[u8]) -> Vec<u8> {
    segment.value(source).into_owned()
}

fn children(context: &Context, node: NodeId) -> Vec<NodeId> {
    let mut result = Vec::new();
    let mut child = context.arena.node(node).first;
    while let Some(id) = child {
        result.push(id);
        child = context.arena.node(id).next;
    }
    result
}

fn render_children(context: &Context, node: NodeId, source: &[u8], output: &mut HtmlBytes) {
    for child in children(context, node) {
        render_node(context, child, source, output);
    }
}

fn render_lines(lines: &[Segment], source: &[u8], output: &mut HtmlBytes) {
    for line in lines.iter().copied() {
        // Goldmark writeLines uses RawWrite for literal code-block content.
        escape_raw(&segment_text(line, source), output);
    }
}

fn destination(value: Option<&Bytes>) -> Option<Vec<u8>> {
    value.map(|value| value.bytes().to_vec())
}

// Pinned Goldmark util.URLEscape: copy-on-write byte slicing is deliberate,
// including incomplete leading-byte consumption and retained invalid leads.
const URL_ESCAPE_TABLE: [u8; 256] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 1, 0, 1, 1, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 1, 0, 1,
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 1,
    0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 1, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];
const UTF8_LEN_TABLE: [u8; 256] = [
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99,
    99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99,
    99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 2, 2, 2, 2, 2, 2, 2, 2,
    2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3,
    3, 3, 3, 3, 3, 3, 4, 4, 4, 4, 4, 4, 4, 4, 99, 99, 99, 99, 99, 99, 99, 99,
];
fn resolve_url_references(source: &[u8]) -> Vec<u8> {
    let mut numeric = Vec::new();
    let mut at = 0;
    while at < source.len() {
        if source[at..].starts_with(b"&#") {
            let hex = source
                .get(at + 2)
                .is_some_and(|byte| matches!(byte, b'x' | b'X'));
            let start = at + if hex { 3 } else { 2 };
            let mut end = start;
            while source.get(end).is_some_and(|byte| {
                if hex {
                    byte.is_ascii_hexdigit()
                } else {
                    byte.is_ascii_digit()
                }
            }) {
                end += 1;
            }
            if end > start && source.get(end) == Some(&b';') && (hex || end - start < 8) {
                let digits = std::str::from_utf8(&source[start..end]).unwrap();
                let radix = if hex {
                    16
                } else if digits.starts_with('0') {
                    8
                } else {
                    10
                };
                let value = match u32::from_str_radix(digits, radix) {
                    Ok(value) => value,
                    Err(error) => {
                        if matches!(error.kind(), std::num::IntErrorKind::PosOverflow) {
                            u32::MAX
                        } else {
                            0
                        }
                    }
                };
                let rune = char::from_u32(value)
                    .filter(|rune| *rune != '\0')
                    .unwrap_or('\u{fffd}');
                numeric.extend_from_slice(rune.encode_utf8(&mut [0; 4]).as_bytes());
                at = end + 1;
                continue;
            }
        }
        numeric.push(source[at]);
        at += 1;
    }
    let mut output = Vec::new();
    at = 0;
    while at < numeric.len() {
        if numeric[at] == b'&' && numeric.get(at + 1) != Some(&b'#') {
            let mut end = at + 1;
            while numeric.get(end).is_some_and(u8::is_ascii_alphanumeric) {
                end += 1;
            }
            if end > at + 1 && numeric.get(end) == Some(&b';') {
                if let Some(value) = entity(&numeric[at + 1..=end]) {
                    output.extend_from_slice(value.as_bytes());
                    at = end + 1;
                    continue;
                }
            }
        }
        output.push(numeric[at]);
        at += 1;
    }
    output
}

fn url_escape(input: &[u8], resolve: bool) -> Vec<u8> {
    let value = if resolve {
        let mut unescaped = Vec::with_capacity(input.len());
        let mut at = 0;
        while at < input.len() {
            if input[at] == b'\\' && input.get(at + 1).is_some_and(u8::is_ascii_punctuation) {
                at += 1;
            }
            unescaped.push(input[at]);
            at += 1;
        }
        resolve_url_references(&unescaped)
    } else {
        input.to_vec()
    };
    let mut output = Vec::new();
    let mut copied = false;
    let mut at = 0;
    let mut pending = 0;
    while at < value.len() {
        let byte = value[at];
        if URL_ESCAPE_TABLE[byte as usize] == 1 {
            at += 1;
            continue;
        }
        if byte == b'%' && at + 2 < value.len() && value[at + 1].is_ascii_hexdigit() {
            at += 3;
            continue;
        }
        let mut width = UTF8_LEN_TABLE[byte as usize] as usize;
        if width == 99 {
            at += 1;
            continue;
        }
        if byte == b' ' {
            output.extend_from_slice(&value[pending..at]);
            output.extend_from_slice(b"%20");
            copied = true;
            at += 1;
            pending = at;
            continue;
        }
        if width > value.len() {
            width = value.len() - 1;
        }
        if width == 0 {
            at += 1;
            pending = at;
            continue;
        }
        output.extend_from_slice(&value[pending..at]);
        copied = true;
        if at + width > value.len() {
            at += 1;
            pending = at;
            continue;
        }
        for byte in &value[at..at + width] {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
                output.push(*byte);
            } else if *byte == b' ' {
                output.push(b'+');
            } else {
                output.extend_from_slice(format!("%{byte:02X}").as_bytes());
            }
        }
        at += width;
        pending = at;
    }
    if copied {
        if pending < value.len() {
            output.extend_from_slice(&value[pending..]);
        }
        output
    } else {
        value
    }
}

fn render_node(context: &Context, node: NodeId, source: &[u8], output: &mut HtmlBytes) {
    let value = context.arena.node(node).data.clone();
    match value {
        NodeKind::Document | NodeKind::Container => render_children(context, node, source, output),
        NodeKind::Text(text) => {
            if text.raw {
                escape_raw(&segment_text(text.segment, source), output);
            } else {
                escape_text(&segment_text(text.segment, source), output);
            }
            if text.hard_line_break {
                output.push_str("<br />\n");
            } else if text.soft_line_break {
                output.push('\n');
            }
        }
        NodeKind::Paragraph(_) => {
            output.push_str("<p>");
            render_children(context, node, source, output);
            output.push_str("</p>\n");
        }
        NodeKind::TextBlock { .. } => {
            render_children(context, node, source, output);
            // Goldmark v1.8.5 html.Renderer.renderTextBlock does not wrap
            // tight-list content in a paragraph (which adds loose margins).
            let block = context.arena.node(node);
            if block.next.is_some() && block.first.is_some() {
                output.push('\n');
            }
        }
        NodeKind::Heading { level, .. } => {
            let level = level.clamp(1, 6);
            let _ = write!(output, "<h{level}>");
            render_children(context, node, source, output);
            let _ = writeln!(output, "</h{level}>");
        }
        NodeKind::Blockquote => {
            output.push_str("<blockquote>\n");
            render_children(context, node, source, output);
            output.push_str("</blockquote>\n");
        }
        NodeKind::List { marker, start, .. } => {
            if marker.is_ascii_digit() || start >= 0 {
                if start > 0 && start != 1 {
                    let _ = writeln!(output, "<ol start=\"{start}\">");
                } else {
                    output.push_str("<ol>\n");
                }
                render_children(context, node, source, output);
                output.push_str("</ol>\n");
            } else {
                output.push_str("<ul>\n");
                render_children(context, node, source, output);
                output.push_str("</ul>\n");
            }
        }
        NodeKind::ListItem { .. } => {
            output.push_str("<li>");
            if context.arena.node(node).first.is_some_and(|id| {
                !matches!(context.arena.node(id).data, NodeKind::TextBlock { .. })
            }) {
                output.push('\n');
            }
            render_children(context, node, source, output);
            output.push_str("</li>\n");
        }
        NodeKind::FencedCodeBlock { info, lines } => {
            output.push_str("<pre><code");
            if let Some(info) = info
                && let NodeKind::Text(text) = &context.arena.node(info).data
            {
                output.push_str(" class=\"language-");
                escape_attr(&segment_text(text.segment, source), output);
                output.push('"');
            }
            output.push('>');
            render_lines(&lines, source, output);
            output.push_str("</code></pre>\n");
        }
        NodeKind::CodeBlock { lines } => {
            output.push_str("<pre><code>");
            render_lines(&lines, source, output);
            output.push_str("</code></pre>\n");
        }
        NodeKind::CodeSpan => {
            output.push_str("<code>");
            render_children(context, node, source, output);
            output.push_str("</code>");
        }
        NodeKind::Emphasis { level, .. } => {
            let tag = if level >= 2 { "strong" } else { "em" };
            let _ = write!(output, "<{tag}>");
            render_children(context, node, source, output);
            let _ = write!(output, "</{tag}>");
        }
        NodeKind::Strikethrough { .. } => {
            output.push_str("<del>");
            render_children(context, node, source, output);
            output.push_str("</del>");
        }
        NodeKind::Link(link) => {
            output.push_str("<a href=\"");
            if let Some(value) = destination(link.destination.as_ref()) {
                escape_raw(&url_escape(&value, true), output);
            }
            output.push('"');
            if let Some(title) = destination(link.title.as_ref()) {
                output.push_str(" title=\"");
                escape_attr(&title, output);
                output.push('"');
            }
            output.push('>');
            render_children(context, node, source, output);
            output.push_str("</a>");
        }
        NodeKind::Image(link) => {
            output.push_str("<img src=\"");
            if let Some(value) = destination(link.destination.as_ref()) {
                escape_raw(&url_escape(&value, true), output);
            }
            output.push_str("\" alt=\"");
            render_children(context, node, source, output);
            output.push_str("\" />");
        }
        NodeKind::AutoLink(link) => {
            let label = context.autolink_label(node, &Bytes::new(source));
            let href = context.autolink_url(node, &Bytes::new(source));
            output.push_str("<a href=\"");
            if link.typ == 1 && link.protocol.is_none() {
                output.push_str("mailto:");
            }
            escape_raw(&url_escape(&href.bytes(), false), output);
            output.push_str("\">");
            escape_text(&label.bytes(), output);
            output.push_str("</a>");
        }
        NodeKind::RawHtml { segments } => {
            for segment in segments {
                output.push_bytes(&segment_text(segment, source));
            }
        }
        NodeKind::HtmlBlock {
            lines,
            closure_line,
            ..
        } => {
            for line in lines.into_iter().chain(closure_line) {
                for byte in segment_text(line, source) {
                    if byte == 0 {
                        output.push('\u{fffd}');
                    } else {
                        output.0.push(byte);
                    }
                }
            }
        }
        NodeKind::ThematicBreak => output.push_str("<hr />\n"),
        NodeKind::Table { .. } => {
            output.push_str("<table>\n");
            render_children(context, node, source, output);
            output.push_str("</table>\n");
        }
        NodeKind::TableHeader { .. } => {
            output.push_str("<thead>\n<tr>\n");
            render_children(context, node, source, output);
            output.push_str("</tr>\n</thead>\n");
            if context.arena.node(node).next.is_some() {
                output.push_str("<tbody>\n");
            }
        }
        NodeKind::TableRow { .. } => {
            output.push_str("<tr>\n");
            render_children(context, node, source, output);
            output.push_str("</tr>\n");
            if context.arena.node(node).next.is_none() {
                output.push_str("</tbody>\n");
            }
        }
        NodeKind::TableCell { alignment, .. } => {
            let tag = context
                .arena
                .node(node)
                .parent
                .and_then(|parent| {
                    matches!(
                        context.arena.node(parent).data,
                        NodeKind::TableHeader { .. }
                    )
                    .then_some("th")
                })
                .unwrap_or("td");
            let _ = write!(output, "<{tag}");
            if let Some(value) = match alignment {
                1 => Some("left"),
                2 => Some("right"),
                3 => Some("center"),
                _ => None,
            } {
                let _ = write!(output, " align=\"{value}\"");
            }
            output.push('>');
            render_children(context, node, source, output);
            let _ = writeln!(output, "</{tag}>");
        }
        NodeKind::LinkLabel(state) => escape_text(&segment_text(state.segment, source), output),
        NodeKind::ReferenceDefinition(_) => {}
        NodeKind::Delimiter(delimiter) => {
            escape_text(&segment_text(delimiter.value.segment, source), output);
        }
    }
}

/// Parses Markdown and returns a deterministic HTML fragment.
pub fn render_html_bytes(source: &[u8]) -> Vec<u8> {
    let (context, root) = super::document::parse_document(source);
    let mut output = HtmlBytes::default();
    render_node(&context, root, source, &mut output);
    output.0
}

/// UTF-8 presentation for consumers that paint or display Markdown. Compiler
/// XML validation uses render_html_bytes before any presentation conversion.
pub fn render_html(source: &[u8]) -> String {
    String::from_utf8_lossy(&render_html_bytes(source)).into_owned()
}

/// D2's RenderMarkdown adds lib/textmeasure/links.go's sanitizeLinks pass
/// after Goldmark rendering. Keep the raw Goldmark projection above available
/// for callers testing the dependency's separate HTML contract.
pub fn render_markdown_html_bytes(source: &[u8]) -> Vec<u8> {
    sanitize_links(render_html_bytes(source))
}

pub fn render_markdown_html(source: &[u8]) -> String {
    String::from_utf8_lossy(&render_markdown_html_bytes(source)).into_owned()
}

fn sanitize_links(input: Vec<u8>) -> Vec<u8> {
    fn replace_all(input: &[u8], from: &[u8], to: &[u8]) -> Vec<u8> {
        let mut output = Vec::with_capacity(input.len());
        let mut at = 0;
        while let Some(offset) = input[at..]
            .windows(from.len())
            .position(|part| part == from)
        {
            let start = at + offset;
            output.extend_from_slice(&input[at..start]);
            output.extend_from_slice(to);
            at = start + from.len();
        }
        output.extend_from_slice(&input[at..]);
        output
    }
    let Some(first) = input.windows(6).position(|part| part == b"href=\"") else {
        return input;
    };
    let mut output = Vec::with_capacity(input.len());
    let mut at = 0;
    let mut start = first;
    loop {
        let value_start = start + 6;
        let Some(length) = input[value_start..].iter().position(|byte| *byte == b'"') else {
            break;
        };
        let end = value_start + length;
        output.extend_from_slice(&input[at..value_start]);
        // Preserve the source's literal TEMP_AMP replacement semantics,
        // including existing occurrences inside a matched attribute value.
        let value = replace_all(&input[value_start..end], b"&amp;", b"TEMP_AMP");
        let value = replace_all(&value, b"&", b"&amp;");
        output.extend_from_slice(&replace_all(&value, b"TEMP_AMP", b"&amp;"));
        output.push(b'"');
        at = end + 1;
        let Some(offset) = input[at..].windows(6).position(|part| part == b"href=\"") else {
            break;
        };
        start = at + offset;
    }
    output.extend_from_slice(&input[at..]);
    output
}

#[cfg(test)]
mod tests {
    use super::render_html;

    #[test]
    fn list_item_first_block_can_be_another_list() {
        // Goldmark parser/parser.go openBlocks descends into HasChildren
        // immediately, including a list item whose content starts with a list.
        for (source, expected) in [
            ("- -", "<ul>\n<li>\n<ul>\n<li></li>\n</ul>\n</li>\n</ul>\n"),
            (
                "- - item",
                "<ul>\n<li>\n<ul>\n<li>item</li>\n</ul>\n</li>\n</ul>\n",
            ),
            (
                "- 1. item",
                "<ul>\n<li>\n<ol>\n<li>item</li>\n</ol>\n</li>\n</ul>\n",
            ),
            ("- - -", "<hr />\n"),
        ] {
            assert_eq!(render_html(source.as_bytes()), expected, "{source:?}");
        }
    }

    #[test]
    fn renders_inline_structure_and_attributes() {
        assert_eq!(
            render_html(b"**bold** [link](https://example.com \"title\")"),
            "<p><strong>bold</strong> <a href=\"https://example.com\" title=\"title\">link</a></p>\n"
        );
    }

    #[test]
    fn renders_lists_tables_and_code() {
        let html = render_html(b"- one\n- two\n\n| a | b |\n| :- | -: |\n| x | y |\n\n`code`");
        assert!(html.contains("<ul>"));
        assert!(html.contains("<table>"));
        assert!(html.contains("<th align=\"left\">a</th>\n"));
        assert!(html.contains("<th align=\"right\">b</th>\n"));
        assert!(html.contains("<code>code</code>"));
    }

    #[test]
    fn headings_respect_block_indentation() {
        // Goldmark v1.8.5 parser.openBlocks checks IndentWidth against
        // ATXHeadingParser.CanAcceptIndentedLine before opening a heading.
        for (source, expected) in [
            ("\t# ", "<pre><code># \n</code></pre>\n"),
            ("    # ", "<pre><code># \n</code></pre>\n"),
            ("   # ", "<h1></h1>\n"),
            ("\t# title\n", "<pre><code># title\n</code></pre>\n"),
            ("paragraph\n\t# ", "<p>paragraph\n#</p>\n"),
            ("- item\n\t# ", "<ul>\n<li>item\n<h1></h1>\n</li>\n</ul>\n"),
            ("- item\n      # ", "<ul>\n<li>item\n#</li>\n</ul>\n"),
            ("> \t# ", "<blockquote>\n<h1></h1>\n</blockquote>\n"),
            (
                ">     # \nplain",
                "<blockquote>\n<pre><code># \n</code></pre>\n</blockquote>\n<p>plain</p>\n",
            ),
            ("- \t# ", "<ul>\n<li>\n<h1></h1>\n</li>\n</ul>\n"),
            (
                "-     # ",
                "<ul>\n<li>\n<pre><code># \n</code></pre>\n</li>\n</ul>\n",
            ),
        ] {
            assert_eq!(render_html(source.as_bytes()), expected, "{source:?}");
        }
    }

    #[test]
    fn list_paragraphs_keep_lazy_continuations() {
        // Goldmark v1.8.5 parser.parseBlocks/openBlocks keeps the last
        // paragraph open after a container's Continue returns Close.
        for source in [
            "- first item\ncontinued on next line",
            "* first item\ncontinued on next line",
            "+ first item\ncontinued on next line",
            "- first item\n continued on next line",
            "- first item\n  continued on next line",
        ] {
            assert_eq!(
                render_html(source.as_bytes()),
                "<ul>\n<li>first item\ncontinued on next line</li>\n</ul>\n",
                "{source:?}"
            );
        }
        for (source, expected) in [
            (
                "1. step\ncontinued",
                "<ol>\n<li>step\ncontinued</li>\n</ol>\n",
            ),
            (
                "- a\n  - b\nplain line",
                "<ul>\n<li>a\n<ul>\n<li>b\nplain line</li>\n</ul>\n</li>\n</ul>\n",
            ),
            (
                "# Heading\n\n- item\ncontinued",
                "<h1>Heading</h1>\n<ul>\n<li>item\ncontinued</li>\n</ul>\n",
            ),
            (
                "- item\n\nparagraph",
                "<ul>\n<li>item</li>\n</ul>\n<p>paragraph</p>\n",
            ),
            (
                "- item\n# heading",
                "<ul>\n<li>item</li>\n</ul>\n<h1>heading</h1>\n",
            ),
            (
                "- # Heading\nplain",
                "<ul>\n<li>\n<h1>Heading</h1>\n</li>\n</ul>\n<p>plain</p>\n",
            ),
            ("- item\n---", "<ul>\n<li>item</li>\n</ul>\n<hr />\n"),
            ("- item\n===", "<ul>\n<li>item\n===</li>\n</ul>\n"),
        ] {
            assert_eq!(render_html(source.as_bytes()), expected, "{source:?}");
        }
    }

    #[test]
    fn d2_document_extension_renders_strikethrough() {
        assert_eq!(render_html(b"~~removed~~"), "<p><del>removed</del></p>\n");
    }

    #[test]
    fn email_autolink_adds_mailto_scheme_to_href() {
        assert_eq!(
            render_html(b"email <test@example.com>"),
            "<p>email <a href=\"mailto:test@example.com\">test@example.com</a></p>\n"
        );
    }

    #[test]
    fn decodes_named_numeric_and_escaped_character_references() {
        assert_eq!(
            render_html(b"&copy; &nbsp; &#x1F642; &amp;copy; &amp;amp;"),
            "<p>©   🙂 &amp;copy; &amp;amp;</p>\n"
        );
    }

    #[test]
    fn triple_emphasis_projection_matches_goldmark() {
        assert_eq!(
            render_html(b"***bold italic***"),
            "<p><em><strong>bold italic</strong></em></p>\n"
        );
    }
}

#[cfg(test)]
mod byte_html_xml_source_contract {
    #[cfg(feature = "oracle-fixtures")]
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    fn decode(text: &str) -> Vec<u8> {
        text.as_bytes()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
            .collect()
    }
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn complete_original_byte_html_and_xml_error_inventory_matches() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/markdown-byte-html-xml-source-v090.json"
        ))
        .unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 128);
        for row in rows.as_array().unwrap() {
            let error = if let Some(source) = row["source_hex"].as_str() {
                let html = render_html_bytes(&decode(source));
                assert_eq!(
                    html,
                    decode(row["html_hex"].as_str().unwrap()),
                    "{}",
                    row["name"]
                );
                crate::xml::validate_markdown_html_bytes(&html)
                    .err()
                    .unwrap_or_default()
            } else {
                crate::xml::validate_xml_value_bytes(&decode(row["xml_hex"].as_str().unwrap()))
                    .err()
                    .unwrap_or_default()
            };
            assert_eq!(error, row["xml_error"].as_str().unwrap(), "{}", row["name"]);
        }
    }
}

#[cfg(test)]
mod writer_source_contract {
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_normal_text_raw_code_and_code_block_writers() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/markdown-writer-source-v090.json"
        ))
        .unwrap();
        let rows = rows.as_array().unwrap();
        assert_eq!(rows.len(), 120);
        let mut output = Vec::new();
        for row in rows {
            assert_eq!(row["error"], "");
            let actual =
                super::render_html(row["request"]["Markdown"].as_str().unwrap().as_bytes());
            assert_eq!(
                actual,
                row["html"].as_str().unwrap(),
                "{}",
                row["request"]["Name"]
            );
            output.push(serde_json::json!({"request":row["request"],"html":actual,"error":""}));
        }
        if let Some(path) = std::env::var_os("D2_RS_TEST_WRITER_OUTPUT") {
            std::fs::write(path, serde_json::to_vec_pretty(&output).unwrap()).unwrap();
        }
    }
}

#[cfg(test)]
mod list_grammar_source_contract {
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn complete_original_list_document_html() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/markdown-list-grammar-source-v090.json"
        ))
        .unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 55);
        let mut failures = Vec::new();
        let mut outcomes = Vec::new();
        for row in rows.as_array().unwrap() {
            let r = &row["request"];
            let actual = super::render_html(r["Markdown"].as_str().unwrap().as_bytes());
            let matched = Some(actual.as_str()) == row["html"].as_str();
            if !matched {
                failures.push(r["Name"].clone());
            }
            outcomes.push(serde_json::json!({"request":r,"actual":actual,"expected":row["html"],"matched":matched}));
        }
        if let Ok(path) = std::env::var("D2_RS_TEST_LIST_GRAMMAR_OUTPUT") {
            std::fs::write(path, serde_json::to_vec(&outcomes).unwrap()).unwrap();
        }
        assert!(
            failures.is_empty(),
            "Original list HTML differences: {failures:?}"
        );
    }
}
