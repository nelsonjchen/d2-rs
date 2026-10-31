// SPDX-License-Identifier: MPL-2.0
use d2_rs_text::markdown::{
    block::{is_thematic_break, process_blockquote},
    reader::Reader,
};
#[test]
fn thematic_marker_and_indent_contract() {
    for marker in *b"-*_" {
        assert!(is_thematic_break(
            &[marker, b' ', marker, b'\t', marker, b'\n'],
            0
        ));
        assert!(!is_thematic_break(&[marker, marker], 0));
    }
    assert!(!is_thematic_break(b"    ---", 0));
    assert!(!is_thematic_break(b"\t---", 0));
    assert!(is_thematic_break(b"\t---", 1));
    assert!(!is_thematic_break(b"-_*", 0));
    assert!(!is_thematic_break(b"---\x0c", 0));
}
#[test]
fn blockquote_tabs_leave_virtual_padding() {
    let mut reader = Reader::new(b">\ttext\n");
    assert!(process_blockquote(&mut reader));
    let (line, segment) = reader.peek_line();
    assert_eq!(segment.start, 2);
    assert_eq!(segment.padding, 2);
    assert_eq!(&*line.unwrap().bytes(), b"  text\n");
    let before = reader.position();
    assert!(!process_blockquote(&mut reader));
    assert_eq!(reader.position(), before);
}

#[test]
fn indented_code_retains_internal_blank_lines_and_trims_tail() {
    use d2_rs_text::markdown::{
        context::{Context, NodeKind},
        paragraph::BlockState,
    };
    let mut reader = Reader::new(b"    code\n\n    next\n\nplain");
    let mut c = Context::default();
    let (node, state) = c.open_code_block(&mut reader);
    assert_eq!(state, BlockState::NO_CHILDREN);
    let node = node.unwrap();
    assert_eq!(c.node_position(node), -1);
    for _ in 0..3 {
        reader.advance_line();
        assert_eq!(
            c.continue_code_block(node, &mut reader),
            BlockState::CONTINUE_NO_CHILDREN
        );
    }
    reader.advance_line();
    assert_eq!(c.continue_code_block(node, &mut reader), BlockState::CLOSE);
    c.close_code_block(node, &reader);
    let NodeKind::CodeBlock { lines } = &c.arena.node(node).data else {
        panic!()
    };
    assert_eq!(lines.len(), 3);
    assert!(lines[0].force_newline);
    assert!(lines[2].force_newline);
    assert_eq!(lines[1].start, 9);
}

#[test]
fn fenced_code_closes_only_on_matching_sufficient_fence() {
    use d2_rs_text::markdown::{
        context::{Context, NodeKind},
        paragraph::BlockState,
    };
    let mut reader = Reader::new(b"~~~ rust\nbody\n~~\n~~~~\n");
    let mut c = Context::default();
    let (node, _) = c.open_fenced_code(&mut reader, 0);
    let node = node.unwrap();
    for _ in 0..2 {
        reader.advance_line();
        assert_eq!(
            c.continue_fenced_code(node, &mut reader),
            BlockState::CONTINUE_NO_CHILDREN
        );
    }
    reader.advance_line();
    assert_eq!(c.continue_fenced_code(node, &mut reader), BlockState::CLOSE);
    let NodeKind::FencedCodeBlock { info, lines } = &c.arena.node(node).data else {
        panic!()
    };
    assert!(info.is_some());
    assert_eq!(lines.len(), 2);
    c.close_fenced_code(node);
    assert!(c.fence.is_none());
}

#[test]
fn setext_close_restores_bar_when_paragraph_was_emptied() {
    use d2_rs_text::markdown::{
        context::{Context, NodeKind},
        paragraph::{BlockState, Paragraph},
        text::Segment,
    };
    for emptied in [false, true] {
        let mut c = Context::default();
        let root = c.arena.insert(NodeKind::Document);
        let p = c.arena.insert(NodeKind::Paragraph(Paragraph {
            lines: vec![Segment::new(0, 5)],
            blank_previous_lines: true,
        }));
        c.arena.append_child(root, p);
        let mut r = Reader::new(b"title\n---\n");
        r.advance_line();
        let (heading, state) = c.open_setext_heading(root, Some(p), &mut r);
        assert_eq!(state, BlockState(48));
        let heading = heading.unwrap();
        c.arena.append_child(root, heading);
        if emptied {
            let NodeKind::Paragraph(p) = &mut c.arena.node_mut(p).data else {
                panic!()
            };
            p.lines.clear();
        }
        c.close_setext_heading(heading, &r);
        assert!(c.temporary_paragraph.is_none());
        if emptied {
            assert!(c.arena.node(heading).parent.is_none());
            let next = c.arena.node(p).next.unwrap();
            let NodeKind::Paragraph(p) = &c.arena.node(next).data else {
                panic!()
            };
            assert_eq!(p.lines[0], Segment::new(6, 10));
        } else {
            assert!(c.arena.node(p).parent.is_none());
            assert_eq!(c.node_position(heading), 0);
            let NodeKind::Heading {
                level,
                blank_previous_lines,
                ..
            } = &c.arena.node(heading).data
            else {
                panic!()
            };
            assert_eq!(*level, 2);
            assert!(*blank_previous_lines);
        }
    }
}

#[test]
fn list_open_and_item_open_preserve_marker_offset_and_padding() {
    use d2_rs_text::markdown::{
        context::{Context, NodeKind},
        paragraph::BlockState,
    };
    let mut context = Context::default();
    let root = context.arena.insert(NodeKind::Document);
    let mut reader = Reader::new(b"- item\n");
    let (list, state) = context.open_list(root, None, &mut reader);
    assert_eq!(state, BlockState(8));
    let list = list.unwrap();
    let NodeKind::List {
        marker,
        start,
        is_tight,
    } = context.arena.node(list).data
    else {
        panic!()
    };
    assert_eq!((marker, start, is_tight), (b'-', -1, true));
    context.arena.append_child(root, list);
    let (item, item_state) = context.open_list_item(list, &mut reader);
    assert_eq!(item_state, BlockState(8));
    let item = item.unwrap();
    let NodeKind::ListItem { offset, .. } = context.arena.node(item).data else {
        panic!()
    };
    assert_eq!(offset, 2);
    context.arena.append_child(list, item);
    assert_eq!(reader.position().1.start, 2);
}

#[test]
fn ordered_list_interruption_requires_one() {
    use d2_rs_text::markdown::{
        context::{Context, NodeKind},
        paragraph::Paragraph,
        text::Segment,
    };
    let mut context = Context::default();
    let root = context.arena.insert(NodeKind::Document);
    let paragraph = context.arena.insert(NodeKind::Paragraph(Paragraph {
        lines: vec![Segment::new(0, 5)],
        blank_previous_lines: false,
    }));
    context.arena.append_child(root, paragraph);
    let mut reader = Reader::new(b"2. item\n");
    let (list, state) = context.open_list(root, Some(paragraph), &mut reader);
    assert_eq!(list, None);
    assert_eq!(
        state,
        d2_rs_text::markdown::paragraph::BlockState::NO_CHILDREN
    );
}

#[test]
fn list_continuation_accepts_same_marker_and_rejects_other_markers() {
    use d2_rs_text::markdown::{
        context::{Context, NodeKind},
        paragraph::BlockState,
    };
    let mut context = Context::default();
    let root = context.arena.insert(NodeKind::Document);
    let mut reader = Reader::new(b"- a\n- b\n+ c\n");
    let (list, _) = context.open_list(root, None, &mut reader);
    let list = list.unwrap();
    context.arena.append_child(root, list);
    let (first, _) = context.open_list_item(list, &mut reader);
    let first = first.unwrap();
    context.arena.append_child(list, first);
    reader.advance_line();
    assert_eq!(context.continue_list(list, &mut reader), BlockState(10));
    let (second, _) = context.open_list_item(list, &mut reader);
    context.arena.append_child(list, second.unwrap());
    reader.advance_line();
    assert_eq!(context.continue_list(list, &mut reader), BlockState::CLOSE);
}

#[test]
fn tight_list_converts_paragraph_children_to_text_blocks() {
    use d2_rs_text::markdown::{
        context::{Context, NodeKind},
        paragraph::Paragraph,
        text::Segment,
    };
    let mut context = Context::default();
    let root = context.arena.insert(NodeKind::Document);
    let list = context.arena.insert(NodeKind::List {
        marker: b'-',
        start: -1,
        is_tight: true,
    });
    let item = context.arena.insert(NodeKind::ListItem {
        offset: 2,
        blank_previous_lines: false,
    });
    let paragraph = context.arena.insert(NodeKind::Paragraph(Paragraph {
        lines: vec![Segment::new(2, 4)],
        blank_previous_lines: false,
    }));
    context.arena.append_child(root, list);
    context.arena.append_child(list, item);
    context.arena.append_child(item, paragraph);
    context.close_list(list);
    assert!(matches!(
        context.arena.node(paragraph).data,
        NodeKind::Paragraph(_)
    ));
    let replacement = context.arena.node(item).first.unwrap();
    assert!(matches!(
        context.arena.node(replacement).data,
        NodeKind::TextBlock { .. }
    ));
}
