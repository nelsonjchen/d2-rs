// SPDX-License-Identifier: MPL-2.0
use d2_rs_text::markdown::{
    bytes::Bytes,
    context::{Context, NodeKind},
    paragraph::Paragraph,
    text::Segment,
};

fn transform(
    source: &[u8],
) -> (
    Context,
    d2_rs_text::markdown::arena::NodeId,
    d2_rs_text::markdown::arena::NodeId,
) {
    let mut c = Context::default();
    let root = c.arena.insert(NodeKind::Document);
    let mut start = 0;
    let lines = source
        .split_inclusive(|&b| b == b'\n')
        .map(|line| {
            let segment = Segment::new(start, start + line.len() as isize);
            start += line.len() as isize;
            segment
        })
        .collect();
    let paragraph = c.arena.insert(NodeKind::Paragraph(Paragraph {
        lines,
        blank_previous_lines: false,
    }));
    c.arena.append_child(root, paragraph);
    c.transform_paragraph_table(paragraph, &Bytes::new(source));
    (c, root, paragraph)
}

#[test]
fn preceding_paragraph_and_shared_alignment() {
    let (c, root, paragraph) = transform(b"intro\na|b\n:-|-:\nc|d\n");
    assert_eq!(c.arena.node(root).first, Some(paragraph));
    let NodeKind::Paragraph(p) = &c.arena.node(paragraph).data else {
        panic!()
    };
    assert_eq!(p.lines, vec![Segment::new(0, 5)]);
    let table = c.arena.node(paragraph).next.unwrap();
    assert_eq!(c.node_position(table), 0);
    let header = c.arena.node(table).first.unwrap();
    assert!(matches!(
        &c.arena.node(header).data,
        NodeKind::TableHeader { alignments: None }
    ));
    assert_eq!(c.node_position(header), 6);
    let row = c.arena.node(header).next.unwrap();
    let NodeKind::Table {
        alignments: Some(a),
    } = &c.arena.node(table).data
    else {
        panic!()
    };
    let NodeKind::TableRow {
        alignments: Some(b),
    } = &c.arena.node(row).data
    else {
        panic!()
    };
    assert!(std::rc::Rc::ptr_eq(a, b));
    a.borrow_mut()[0] = 3;
    assert_eq!(b.borrow()[0], 3);
    let cell = c.arena.node(row).first.unwrap();
    assert!(matches!(
        c.arena.node(cell).data,
        NodeKind::TableCell { alignment: 1, .. }
    ));
}

#[test]
fn replacement_and_width_failure() {
    let (c, root, paragraph) = transform(b"a|b\n-|-\nc|d\n");
    assert_eq!(c.arena.node(paragraph).parent, None);
    assert!(matches!(
        c.arena.node(c.arena.node(root).first.unwrap()).data,
        NodeKind::Table { .. }
    ));
    let (c, root, paragraph) = transform(b"a|b|c\n-|-\nx|y\n-|-\n");
    assert_eq!(c.arena.node(root).first, Some(paragraph));
    assert_eq!(c.arena.node(paragraph).next, None);
    let NodeKind::Paragraph(p) = &c.arena.node(paragraph).data else {
        panic!()
    };
    assert_eq!(p.lines.len(), 4);
}

#[test]
fn escaped_pipes_are_removed_only_inside_code_spans() {
    use d2_rs_text::markdown::{block_reader::BlockReader, dispatcher::InlineOptions};
    let source = b"`a\\|b\\|c`|d\\|e\n-|-\n";
    let (mut c, root, _) = transform(source);
    let table = c.arena.node(root).first.unwrap();
    let header = c.arena.node(table).first.unwrap();
    let first = c.arena.node(header).first.unwrap();
    let second = c.arena.node(first).next.unwrap();
    for cell in [first, second] {
        let NodeKind::TableCell { lines, .. } = &c.arena.node(cell).data else {
            panic!()
        };
        let lines = lines.clone();
        let mut reader = BlockReader::new(source, Some(&lines));
        c.parse_inline_block(cell, &mut reader, &lines, false, InlineOptions::default());
    }
    assert!(c.escaped_pipe_cells.is_some());
    let second_before = c.arena.node(second).first;
    c.transform_table_escaped_pipes();
    assert!(c.escaped_pipe_cells.is_none());
    assert_eq!(c.arena.node(second).first, second_before);
    let code = c.arena.node(first).first.unwrap();
    assert!(matches!(c.arena.node(code).data, NodeKind::CodeSpan));
    let mut child = c.arena.node(code).first;
    let mut value = Vec::new();
    let mut segments = Vec::new();
    while let Some(id) = child {
        let NodeKind::Text(t) = &c.arena.node(id).data else {
            panic!()
        };
        assert!(t.raw);
        segments.push(t.segment);
        value.extend_from_slice(&source[t.segment.start as usize..t.segment.stop as usize]);
        child = c.arena.node(id).next;
    }
    assert_eq!(value, b"a|b|c");
    assert_eq!(segments.len(), 3);
    let children_before = c.arena.node(code).first;
    c.transform_table_escaped_pipes();
    assert_eq!(c.arena.node(code).first, children_before);
}
