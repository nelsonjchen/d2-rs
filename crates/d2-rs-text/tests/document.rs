// SPDX-License-Identifier: MPL-2.0
use d2_rs_text::markdown::{context::NodeKind, document::parse_document};

#[test]
fn document_wires_heading_paragraph_and_inline_stages() {
    let (context, root) = parse_document(b"# title\nhello *world*\n");
    let heading = context.arena.node(root).first.unwrap();
    assert!(matches!(
        context.arena.node(heading).data,
        NodeKind::Heading { level: 1, .. }
    ));
    let paragraph = context.arena.node(heading).next.unwrap();
    assert!(matches!(
        context.arena.node(paragraph).data,
        NodeKind::Paragraph(_)
    ));
    assert!(context.arena.node(paragraph).first.is_some());
}

#[test]
fn document_does_not_spin_on_indented_blockquote_marker() {
    let (context, root) = parse_document(b"${missing}\n    >x");
    let paragraph = context.arena.node(root).first.unwrap();
    assert!(matches!(
        context.arena.node(paragraph).data,
        NodeKind::Paragraph(_)
    ));
    assert!(context.arena.node(paragraph).next.is_none());
}

#[test]
fn document_wires_fenced_code_and_lists() {
    let (context, root) = parse_document(b"```\ncode\n```\n- item\n");
    let fence = context.arena.node(root).first.unwrap();
    assert!(matches!(
        context.arena.node(fence).data,
        NodeKind::FencedCodeBlock { .. }
    ));
    let list = context.arena.node(fence).next.unwrap();
    assert!(matches!(
        context.arena.node(list).data,
        NodeKind::List { .. }
    ));
    assert!(context.arena.node(list).first.is_some());
}

#[test]
fn document_retains_every_line_in_a_fenced_code_block() {
    let (context, root) = parse_document(b"```\none\ntwo\nthree\n```\n");
    let fence = context.arena.node(root).first.unwrap();
    let NodeKind::FencedCodeBlock { lines, .. } = &context.arena.node(fence).data else {
        panic!("expected fenced code")
    };
    assert_eq!(lines.len(), 3);
}

#[test]
fn document_applies_list_interruption_rules_to_paragraphs() {
    let (context, root) = parse_document(b"text\n- item\n1. ordered\n2. continued\n");
    let paragraph = context.arena.node(root).first.unwrap();
    let list = context.arena.node(paragraph).next.unwrap();
    assert!(matches!(
        context.arena.node(paragraph).data,
        NodeKind::Paragraph(_)
    ));
    assert!(matches!(
        context.arena.node(list).data,
        NodeKind::List { .. }
    ));
    let ordered = context.arena.node(list).next.unwrap();
    assert!(matches!(
        context.arena.node(ordered).data,
        NodeKind::List { start: 1, .. }
    ));
    let final_paragraph = context.arena.node(ordered).next;
    assert!(final_paragraph.is_none());
}

#[test]
fn document_keeps_non_one_ordered_markers_in_paragraphs() {
    let (context, root) = parse_document(b"text\n2. continued\n");
    let paragraph = context.arena.node(root).first.unwrap();
    assert!(matches!(
        context.arena.node(paragraph).data,
        NodeKind::Paragraph(_)
    ));
    let NodeKind::Paragraph(value) = &context.arena.node(paragraph).data else {
        unreachable!()
    };
    assert_eq!(value.lines.len(), 2);
}

#[test]
fn document_keeps_invalid_atx_lines_in_paragraphs() {
    let (context, root) = parse_document(b"text\n#not a heading\n####### too many\n*word\n");
    let paragraph = context.arena.node(root).first.unwrap();
    assert!(matches!(
        context.arena.node(paragraph).data,
        NodeKind::Paragraph(_)
    ));
    let NodeKind::Paragraph(value) = &context.arena.node(paragraph).data else {
        unreachable!()
    };
    assert_eq!(value.lines.len(), 4);
    assert!(context.arena.node(paragraph).next.is_none());
}

#[test]
fn document_keeps_short_and_invalid_fences_in_paragraphs() {
    let (context, root) = parse_document(b"text\n``\n```bad`info\n");
    let paragraph = context.arena.node(root).first.unwrap();
    assert!(matches!(
        context.arena.node(paragraph).data,
        NodeKind::Paragraph(_)
    ));
    let NodeKind::Paragraph(value) = &context.arena.node(paragraph).data else {
        unreachable!()
    };
    assert_eq!(value.lines.len(), 3);
    assert!(context.arena.node(paragraph).next.is_none());
}

#[test]
fn document_interrupts_a_paragraph_for_valid_heading_and_fence() {
    let (context, root) = parse_document(b"text\n## heading\n```rust\ncode\n```\n");
    let paragraph = context.arena.node(root).first.unwrap();
    let heading = context.arena.node(paragraph).next.unwrap();
    let fence = context.arena.node(heading).next.unwrap();
    assert!(matches!(
        context.arena.node(paragraph).data,
        NodeKind::Paragraph(_)
    ));
    assert!(matches!(
        context.arena.node(heading).data,
        NodeKind::Heading { level: 2, .. }
    ));
    assert!(matches!(
        context.arena.node(fence).data,
        NodeKind::FencedCodeBlock { .. }
    ));
}

#[test]
fn document_parses_multiple_list_items_with_text_blocks() {
    let (context, root) = parse_document(b"- one\n- two\n");
    let list = context.arena.node(root).first.unwrap();
    let first = context.arena.node(list).first.unwrap();
    let second = context.arena.node(first).next.unwrap();
    assert!(matches!(
        context.arena.node(first).data,
        NodeKind::ListItem { .. }
    ));
    assert!(matches!(
        context
            .arena
            .node(first)
            .first
            .map(|id| &context.arena.node(id).data),
        Some(NodeKind::TextBlock { .. })
    ));
    assert!(matches!(
        context
            .arena
            .node(second)
            .first
            .map(|id| &context.arena.node(id).data),
        Some(NodeKind::TextBlock { .. })
    ));
}

#[test]
fn document_parses_nested_list_items() {
    let (context, root) = parse_document(b"- parent\n   - child\n- sibling\n");
    let list = context.arena.node(root).first.unwrap();
    let item = context.arena.node(list).first.unwrap();
    let nested = context.arena.node(item).last.unwrap();
    assert!(matches!(
        context.arena.node(nested).data,
        NodeKind::List { .. }
    ));
    let child = context.arena.node(nested).first.unwrap();
    assert!(matches!(
        context.arena.node(child).data,
        NodeKind::ListItem { .. }
    ));
    let sibling = context.arena.node(item).next.unwrap();
    assert!(matches!(
        context.arena.node(sibling).data,
        NodeKind::ListItem { .. }
    ));
}

#[test]
fn document_parses_headings_as_list_item_children() {
    let (context, root) = parse_document(b"- # heading\n");
    let list = context.arena.node(root).first.unwrap();
    let item = context.arena.node(list).first.unwrap();
    let child = context.arena.node(item).first.unwrap();
    assert!(matches!(
        context.arena.node(child).data,
        NodeKind::Heading { level: 1, .. }
    ));
}

#[test]
fn document_parses_fenced_code_as_list_item_children() {
    let (context, root) = parse_document(b"- ```rust\n  code\n  ```\n");
    let list = context.arena.node(root).first.unwrap();
    let item = context.arena.node(list).first.unwrap();
    let child = context.arena.node(item).first.unwrap();
    assert!(matches!(
        context.arena.node(child).data,
        NodeKind::FencedCodeBlock { .. }
    ));
    let NodeKind::FencedCodeBlock { lines, .. } = &context.arena.node(child).data else {
        unreachable!()
    };
    assert_eq!(lines.len(), 1);
}

#[test]
fn document_parses_indented_code_as_list_item_child() {
    let (context, root) = parse_document(b"*\t\t<>[${name}]\n${name}");
    let list = context.arena.node(root).first.unwrap();
    let item = context.arena.node(list).first.unwrap();
    let code = context.arena.node(item).first.unwrap();
    assert!(matches!(
        context.arena.node(code).data,
        NodeKind::CodeBlock { .. }
    ));
    assert!(context.arena.node(list).next.is_some());
}

#[test]
fn document_prefers_indented_code_over_a_fence_after_a_tab() {
    let (context, root) = parse_document(b"\t~~~</div>`\\#######\n-?>1.\n");
    let code = context.arena.node(root).first.unwrap();
    assert!(matches!(
        context.arena.node(code).data,
        NodeKind::CodeBlock { .. }
    ));
}

#[test]
fn document_parses_thematic_break_as_a_list_item_child() {
    let (context, root) = parse_document(b"+ ---\n");
    let list = context.arena.node(root).first.unwrap();
    let item = context.arena.node(list).first.unwrap();
    let child = context.arena.node(item).first.unwrap();
    assert!(matches!(
        context.arena.node(child).data,
        NodeKind::ThematicBreak
    ));
}

#[test]
fn document_parses_html_as_a_list_item_child() {
    let (context, root) = parse_document(b"- <div>\n  body\n  </div>\n");
    let list = context.arena.node(root).first.unwrap();
    let item = context.arena.node(list).first.unwrap();
    let child = context.arena.node(item).first.unwrap();
    assert!(matches!(
        context.arena.node(child).data,
        NodeKind::HtmlBlock { .. }
    ));
}

#[test]
fn document_parses_blockquotes_as_list_item_children() {
    let (context, root) = parse_document(b"- > one\n  > two\n");
    let list = context.arena.node(root).first.unwrap();
    let item = context.arena.node(list).first.unwrap();
    let quote = context.arena.node(item).first.unwrap();
    assert!(matches!(
        context.arena.node(quote).data,
        NodeKind::Blockquote
    ));
    let paragraph = context.arena.node(quote).first.unwrap();
    assert_eq!(context.arena.node(paragraph).child_count, 2);
}

#[test]
fn document_keeps_blank_lines_inside_one_loose_list() {
    let (context, root) = parse_document(b"- one\n\n- two\n");
    let list = context.arena.node(root).first.unwrap();
    let NodeKind::List { is_tight, .. } = context.arena.node(list).data else {
        panic!("expected list")
    };
    assert!(!is_tight);
    let first = context.arena.node(list).first.unwrap();
    let second = context.arena.node(first).next.unwrap();
    assert!(matches!(
        context
            .arena
            .node(first)
            .first
            .map(|id| &context.arena.node(id).data),
        Some(NodeKind::Paragraph(_))
    ));
    assert!(matches!(
        context
            .arena
            .node(second)
            .first
            .map(|id| &context.arena.node(id).data),
        Some(NodeKind::Paragraph(_))
    ));
    assert!(context.arena.node(second).next.is_none());
}

#[test]
fn document_keeps_indented_paragraphs_inside_a_list_item() {
    let (context, root) = parse_document(b"- one\n\n  two\n");
    let list = context.arena.node(root).first.unwrap();
    let item = context.arena.node(list).first.unwrap();
    assert_eq!(context.arena.node(item).child_count, 2);
    let first = context.arena.node(item).first.unwrap();
    let second = context.arena.node(first).next.unwrap();
    assert!(matches!(
        context.arena.node(second).data,
        NodeKind::Paragraph(_)
    ));
}

#[test]
fn document_wires_repeated_blockquote_lines() {
    let (context, root) = parse_document(b"> one\n> two\nplain\n");
    let quote = context.arena.node(root).first.unwrap();
    assert!(matches!(
        context.arena.node(quote).data,
        NodeKind::Blockquote
    ));
    let paragraph = context.arena.node(quote).first.unwrap();
    assert!(matches!(
        context.arena.node(paragraph).data,
        NodeKind::Paragraph(_)
    ));
    assert_eq!(context.arena.node(paragraph).child_count, 3);
    assert!(context.arena.node(quote).next.is_none());
}

#[test]
fn document_keeps_lazy_paragraph_continuations_inside_blockquotes() {
    let (context, root) = parse_document(b"> one\ntwo\n");
    let quote = context.arena.node(root).first.unwrap();
    let paragraph = context.arena.node(quote).first.unwrap();
    assert!(matches!(
        context.arena.node(quote).data,
        NodeKind::Blockquote
    ));
    let NodeKind::Paragraph(value) = &context.arena.node(paragraph).data else {
        panic!("expected quote paragraph")
    };
    assert_eq!(value.lines.len(), 2);
    assert!(context.arena.node(quote).next.is_none());
}

#[test]
fn document_closes_blockquotes_before_interrupting_lazy_blocks() {
    let (context, root) = parse_document(b"> one\n# heading\n- item\n");
    let quote = context.arena.node(root).first.unwrap();
    let heading = context.arena.node(quote).next.unwrap();
    let list = context.arena.node(heading).next.unwrap();
    assert!(matches!(
        context.arena.node(heading).data,
        NodeKind::Heading { level: 1, .. }
    ));
    assert!(matches!(
        context.arena.node(list).data,
        NodeKind::List { .. }
    ));
}

#[test]
fn document_keeps_blank_lines_between_quote_paragraphs() {
    let (context, root) = parse_document(b"> one\n>\n> two\n");
    let quote = context.arena.node(root).first.unwrap();
    let first = context.arena.node(quote).first.unwrap();
    let second = context.arena.node(first).next.unwrap();
    assert!(matches!(
        context.arena.node(first).data,
        NodeKind::Paragraph(_)
    ));
    assert!(matches!(
        context.arena.node(second).data,
        NodeKind::Paragraph(_)
    ));
    assert!(context.arena.node(second).next.is_none());
}

#[test]
fn document_preserves_nested_blockquote_containers() {
    let (context, root) = parse_document(b"> outer\n> > inner\n> after\n");
    let quote = context.arena.node(root).first.unwrap();
    let outer_paragraph = context.arena.node(quote).first.unwrap();
    assert!(matches!(
        context.arena.node(outer_paragraph).data,
        NodeKind::Paragraph(_)
    ));
    let nested = context.arena.node(outer_paragraph).next.unwrap();
    assert!(matches!(
        context.arena.node(nested).data,
        NodeKind::Blockquote
    ));
    let inner = context.arena.node(nested).first.unwrap();
    assert!(matches!(
        context.arena.node(inner).data,
        NodeKind::Paragraph(_)
    ));
    let trailing = context.arena.node(nested).next.unwrap();
    assert!(matches!(
        context.arena.node(trailing).data,
        NodeKind::Paragraph(_)
    ));
}

#[test]
fn document_parses_headings_inside_blockquotes() {
    let (context, root) = parse_document(b"> # title\n> body\n");
    let quote = context.arena.node(root).first.unwrap();
    assert!(matches!(
        context
            .arena
            .node(quote)
            .first
            .map(|id| &context.arena.node(id).data),
        Some(NodeKind::Heading { level: 1, .. })
    ));
    let heading = context.arena.node(quote).first.unwrap();
    let paragraph = context.arena.node(heading).next.unwrap();
    assert!(matches!(
        context.arena.node(paragraph).data,
        NodeKind::Paragraph(_)
    ));
}

#[test]
fn document_promotes_setext_headings_inside_blockquotes() {
    let (context, root) = parse_document(b"> title\n> ---\n> after\n");
    let quote = context.arena.node(root).first.unwrap();
    let heading = context.arena.node(quote).first.unwrap();
    assert!(matches!(
        context.arena.node(heading).data,
        NodeKind::Heading { level: 2, .. }
    ));
    let paragraph = context.arena.node(heading).next.unwrap();
    assert!(matches!(
        context.arena.node(paragraph).data,
        NodeKind::Paragraph(_)
    ));
}

#[test]
fn document_parses_fenced_code_inside_blockquotes() {
    let (context, root) = parse_document(b"> ```rust\n> code\n> ```\n> after\n");
    let quote = context.arena.node(root).first.unwrap();
    let fence = context.arena.node(quote).first.unwrap();
    assert!(matches!(
        context.arena.node(fence).data,
        NodeKind::FencedCodeBlock { .. }
    ));
    let paragraph = context.arena.node(fence).next.unwrap();
    assert!(matches!(
        context.arena.node(paragraph).data,
        NodeKind::Paragraph(_)
    ));
}

#[test]
fn document_parses_html_blocks_inside_blockquotes() {
    let (context, root) = parse_document(b"> <!--\n> body\n> -->\n> after\n");
    let quote = context.arena.node(root).first.unwrap();
    let html = context.arena.node(quote).first.unwrap();
    assert!(matches!(
        context.arena.node(html).data,
        NodeKind::HtmlBlock { .. }
    ));
    let paragraph = context.arena.node(html).next.unwrap();
    assert!(matches!(
        context.arena.node(paragraph).data,
        NodeKind::Paragraph(_)
    ));
}

#[test]
fn document_does_not_skip_after_single_line_html_in_a_quote() {
    let (context, root) = parse_document(b"> <!-- one -->\n> after\n");
    let quote = context.arena.node(root).first.unwrap();
    let html = context.arena.node(quote).first.unwrap();
    assert!(matches!(
        context.arena.node(html).data,
        NodeKind::HtmlBlock { .. }
    ));
    let paragraph = context.arena.node(html).next.unwrap();
    assert!(matches!(
        context.arena.node(paragraph).data,
        NodeKind::Paragraph(_)
    ));
}

#[test]
fn document_keeps_html_before_setext_inside_blockquotes() {
    let (context, root) = parse_document(b"> <div>\n> ---\n");
    let quote = context.arena.node(root).first.unwrap();
    assert!(matches!(
        context
            .arena
            .node(quote)
            .first
            .map(|id| &context.arena.node(id).data),
        Some(NodeKind::HtmlBlock { .. })
    ));
}

#[test]
fn document_transforms_references_inside_blockquotes() {
    let (context, root) = parse_document(b"> [site]: https://example.com\n>\n> [home][site]\n");
    let quote = context.arena.node(root).first.unwrap();
    let definition = context.arena.node(quote).first.unwrap();
    assert!(matches!(
        context.arena.node(definition).data,
        NodeKind::ReferenceDefinition(_)
    ));
    let paragraph = context.arena.node(definition).next.unwrap();
    let link = context.arena.node(paragraph).first.unwrap();
    assert!(matches!(context.arena.node(link).data, NodeKind::Link(_)));
}

#[test]
fn document_parses_indented_code_inside_blockquotes() {
    let (context, root) = parse_document(b">     code\n>     more\n> after\n");
    let quote = context.arena.node(root).first.unwrap();
    let code = context.arena.node(quote).first.unwrap();
    assert!(matches!(
        context.arena.node(code).data,
        NodeKind::CodeBlock { .. }
    ));
    let paragraph = context.arena.node(code).next.unwrap();
    assert!(matches!(
        context.arena.node(paragraph).data,
        NodeKind::Paragraph(_)
    ));
}

#[test]
fn document_parses_thematic_breaks_inside_blockquotes() {
    let (context, root) = parse_document(b"> ---\n> after\n");
    let quote = context.arena.node(root).first.unwrap();
    let thematic = context.arena.node(quote).first.unwrap();
    assert!(matches!(
        context.arena.node(thematic).data,
        NodeKind::ThematicBreak
    ));
    let paragraph = context.arena.node(thematic).next.unwrap();
    assert!(matches!(
        context.arena.node(paragraph).data,
        NodeKind::Paragraph(_)
    ));
}

#[test]
fn document_promotes_pipe_rows_to_a_table() {
    let (context, root) = parse_document(b"| name | value |\n| :--- | ---: |\n| a | 1 |\n");
    let table = context.arena.node(root).first.unwrap();
    assert!(matches!(
        context.arena.node(table).data,
        NodeKind::Table { .. }
    ));
    let header = context.arena.node(table).first.unwrap();
    assert!(matches!(
        context.arena.node(header).data,
        NodeKind::TableHeader { .. }
    ));
    assert_eq!(context.arena.node(header).child_count, 2);
    let row = context.arena.node(table).last.unwrap();
    assert!(matches!(
        context.arena.node(row).data,
        NodeKind::TableRow { .. }
    ));
    assert_eq!(context.arena.node(row).child_count, 2);
    let first_cell = context.arena.node(header).first.unwrap();
    assert!(context.arena.node(first_cell).first.is_some());
}

#[test]
fn document_promotes_tables_inside_blockquotes() {
    let (context, root) = parse_document(b"> | name |\n> | --- |\n> | value |\n");
    let quote = context.arena.node(root).first.unwrap();
    let table = context.arena.node(quote).first.unwrap();
    assert!(matches!(
        context.arena.node(table).data,
        NodeKind::Table { .. }
    ));
    assert!(context.arena.node(table).first.is_some());
}

#[test]
fn document_registers_reference_definitions_before_inline_links() {
    let (context, root) = parse_document(b"[site]: https://example.com\n\n[home][site]\n");
    let definition = context.arena.node(root).first.unwrap();
    assert!(matches!(
        context.arena.node(definition).data,
        NodeKind::ReferenceDefinition(_)
    ));
    let paragraph = context.arena.node(definition).next.unwrap();
    assert!(matches!(
        context.arena.node(paragraph).data,
        NodeKind::Paragraph(_)
    ));
    let link = context.arena.node(paragraph).first.unwrap();
    assert!(matches!(context.arena.node(link).data, NodeKind::Link(_)));
}

#[test]
fn document_parses_list_blocks_inside_blockquotes() {
    let (context, root) = parse_document(b"> - one\n> - two\n");
    let quote = context.arena.node(root).first.unwrap();
    let list = context.arena.node(quote).first.unwrap();
    assert!(matches!(
        context.arena.node(list).data,
        NodeKind::List { .. }
    ));
    assert_eq!(context.arena.node(list).child_count, 2);
}

#[test]
fn document_keeps_blank_lines_inside_a_quote_list() {
    let (context, root) = parse_document(b"> - one\n>\n> - two\n");
    let quote = context.arena.node(root).first.unwrap();
    let list = context.arena.node(quote).first.unwrap();
    let NodeKind::List { is_tight, .. } = context.arena.node(list).data else {
        panic!("expected quote list")
    };
    assert!(!is_tight);
    assert_eq!(context.arena.node(list).child_count, 2);
}

#[test]
fn document_wires_html_comment_and_block_tag() {
    let (context, root) = parse_document(b"<!-- one\n two -->\n<div>\nbody\n</div>\n");
    let comment = context.arena.node(root).first.unwrap();
    assert!(matches!(
        context.arena.node(comment).data,
        NodeKind::HtmlBlock { .. }
    ));
    let div = context.arena.node(comment).next.unwrap();
    assert!(matches!(
        context.arena.node(div).data,
        NodeKind::HtmlBlock { .. }
    ));
}

#[test]
fn document_does_not_skip_after_single_line_html_blocks() {
    let (context, root) = parse_document(b"<!-- one -->\ntext\n");
    let html = context.arena.node(root).first.unwrap();
    assert!(matches!(
        context.arena.node(html).data,
        NodeKind::HtmlBlock { .. }
    ));
    let paragraph = context.arena.node(html).next.unwrap();
    assert!(matches!(
        context.arena.node(paragraph).data,
        NodeKind::Paragraph(_)
    ));
}

#[test]
fn document_html_block_interrupts_paragraph_only_for_interrupting_types() {
    let (context, root) = parse_document(b"text\n<div>\nbody\n\ntext2\n<custom>\nmore\n");
    let paragraph = context.arena.node(root).first.unwrap();
    assert!(matches!(
        context.arena.node(paragraph).data,
        NodeKind::Paragraph(_)
    ));
    let div = context.arena.node(paragraph).next.unwrap();
    assert!(matches!(
        context.arena.node(div).data,
        NodeKind::HtmlBlock { .. }
    ));
    let custom = context.arena.node(div).next.unwrap();
    assert!(matches!(
        context.arena.node(custom).data,
        NodeKind::Paragraph(_)
    ));
    let NodeKind::Paragraph(value) = &context.arena.node(custom).data else {
        unreachable!()
    };
    assert_eq!(value.lines.len(), 3);
}

#[test]
fn document_promotes_setext_paragraphs_and_parses_heading_inlines() {
    let (context, root) = parse_document(b"title *em*\n---\n");
    let heading = context.arena.node(root).first.unwrap();
    let NodeKind::Heading { level, lines, .. } = &context.arena.node(heading).data else {
        panic!("expected setext heading")
    };
    assert_eq!(*level, 2);
    assert_eq!(lines.len(), 1);
    assert!(context.arena.node(heading).first.is_some());
}
