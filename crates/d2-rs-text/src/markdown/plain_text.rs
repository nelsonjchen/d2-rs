// SPDX-License-Identifier: MPL-2.0
//! Semantic plain-text projection for D2 Markdown labels.
//!
//! D2's renderer keeps Markdown structure for styling, but measurement and
//! the first native SVG target need a stable text projection. This walks the
//! translated Goldmark-shaped arena so emphasis, links, code spans, lists,
//! tables, and block boundaries do not leak their source punctuation.

use super::{
    arena::NodeId,
    bytes::Bytes,
    context::{Context, NodeKind},
    text::Segment,
};

fn append_segment(output: &mut String, segment: Segment, source: &[u8]) {
    output.push_str(&String::from_utf8_lossy(&segment.value(source)));
}

fn append_lines(output: &mut String, lines: &[Segment], source: &[u8]) {
    for (index, line) in lines.iter().enumerate() {
        if index != 0 {
            output.push('\n');
        }
        append_segment(output, *line, source);
    }
}

fn is_block(node: &NodeKind) -> bool {
    matches!(
        node,
        NodeKind::Document
            | NodeKind::Container
            | NodeKind::List { .. }
            | NodeKind::ListItem { .. }
            | NodeKind::TextBlock { .. }
            | NodeKind::HtmlBlock { .. }
            | NodeKind::Blockquote
            | NodeKind::Heading { .. }
            | NodeKind::FencedCodeBlock { .. }
            | NodeKind::CodeBlock { .. }
            | NodeKind::ThematicBreak
            | NodeKind::Table { .. }
            | NodeKind::TableHeader { .. }
            | NodeKind::TableRow { .. }
            | NodeKind::TableCell { .. }
            | NodeKind::Paragraph(_)
            | NodeKind::ReferenceDefinition(_)
    )
}

fn append_children(context: &Context, node: NodeId, source: &[u8], output: &mut String) {
    let mut child = context.arena.node(node).first;
    let mut first = true;
    while let Some(id) = child {
        let child_is_block = is_block(&context.arena.node(id).data);
        if child_is_block && !first && !output.ends_with('\n') {
            output.push('\n');
        }
        append_node(context, id, source, output);
        first = false;
        child = context.arena.node(id).next;
    }
}

fn append_node(context: &Context, node: NodeId, source: &[u8], output: &mut String) {
    let value = &context.arena.node(node).data;
    match value {
        NodeKind::Text(text) => {
            append_segment(output, text.segment, source);
            if text.hard_line_break || text.soft_line_break {
                output.push('\n');
            }
        }
        NodeKind::AutoLink(link) => {
            if let NodeKind::Text(text) = &context.arena.node(link.value).data {
                append_segment(output, text.segment, source);
            }
        }
        // D2's native Markdown ruler does not fetch or reserve space for an
        // image when the renderer has no asset provider. Its alt text is
        // metadata, not visible label text.
        NodeKind::Image(_) => {
            // The surrounding paragraph supplies collapsible whitespace. Drop
            // a preceding separator here so a following separator does not
            // become a visible double space after the image is omitted.
            if output.ends_with(' ') {
                output.pop();
            }
        }
        NodeKind::FencedCodeBlock { lines, .. } | NodeKind::CodeBlock { lines } => {
            append_lines(output, lines, source);
        }
        NodeKind::HtmlBlock { lines, .. } => append_lines(output, lines, source),
        // Raw inline tags contribute styling, not visible text.  Keeping the
        // source spelling here makes the graph measurer count `<b>` and
        // `</b>` as label glyphs, unlike Goldmark's text node projection.
        // Their children (for example the text inside `<b>...</b>`) are
        // separate arena nodes and are still visited by the surrounding
        // paragraph.
        NodeKind::RawHtml { .. } => {}
        NodeKind::ThematicBreak => output.push_str("---"),
        NodeKind::LinkLabel(state) => append_segment(output, state.segment, source),
        _ => append_children(context, node, source, output),
    }
}

/// Projects one Markdown document to the text visible to a semantic target.
pub fn plain_text(source: &[u8]) -> String {
    let (context, root) = super::document::parse_document(source);
    let source_bytes = Bytes::new(source);
    let mut output = String::new();
    append_node(&context, root, &source_bytes.bytes(), &mut output);
    output.trim_end_matches('\n').to_owned()
}

#[cfg(test)]
mod tests {
    use super::plain_text;

    #[test]
    fn projects_inline_markdown_without_source_punctuation() {
        assert_eq!(
            plain_text(b"**hello** [world](https://example.com)"),
            "hello world"
        );
    }

    #[test]
    fn preserves_block_and_code_text() {
        assert_eq!(
            plain_text(b"# title\n\n```rust\nlet x = 1;\n```"),
            "title\nlet x = 1;"
        );
    }

    #[test]
    fn omits_raw_inline_tag_spelling() {
        assert_eq!(
            plain_text(b"<b>bold</b> <em>italic</em> <s>gone</s>"),
            "bold italic gone"
        );
    }

    #[test]
    fn omits_unfetched_image_alt_text() {
        assert_eq!(
            plain_text(b"before ![build passing](https://example.invalid/status.svg) after"),
            "before after"
        );
    }
}
