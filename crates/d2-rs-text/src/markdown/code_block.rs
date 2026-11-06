// SPDX-License-Identifier: MPL-2.0 AND MIT
// Copyright (c) 2019 Yusuke Inuzuka
// Rust translation Copyright 2026 Nelson Chen.
// Goldmark v1.8.5 parser/code_block.go and util.IndentPositionPadding.
use super::{
    arena::NodeId,
    context::{Context, NodeKind},
    paragraph::BlockState,
    reader_api::BlockCursor,
    text::{Segment, is_space},
};
fn indent_position(line: &[u8], offset: isize) -> Option<(isize, isize)> {
    let mut width = 0;
    let mut pos = 0;
    for &byte in line {
        if width >= 4 {
            break;
        }
        match byte {
            b'\t' => width += 4 - (offset + width) % 4,
            b' ' => width += 1,
            _ => break,
        }
        pos += 1;
    }
    (width >= 4).then_some((pos, width - 4))
}
fn code_segment(reader: &mut impl BlockCursor, pos: isize, padding: isize) -> Segment {
    reader.advance_and_set_padding(pos, padding);
    let (_, mut segment) = reader.peek_line();
    if segment.padding != 0 {
        let offset = reader.line_offset();
        let (line, saved) = reader.position();
        reader.set_position(line, Segment::new(saved.start - 1, saved.stop));
        if offset == reader.line_offset() {
            segment.padding = 0;
            segment.start -= 1;
        }
        reader.set_position(line, saved);
    }
    segment.force_newline = true;
    reader.advance_to_eol();
    segment
}
impl Context {
    pub fn open_code_block(
        &mut self,
        reader: &mut impl BlockCursor,
    ) -> (Option<NodeId>, BlockState) {
        let (line, _) = reader.peek_line();
        let Some(line) = line else {
            return (None, BlockState::NO_CHILDREN);
        };
        let bytes = line.bytes();
        let position = indent_position(&bytes, reader.line_offset());
        let Some((pos, padding)) = position.filter(|_| !bytes.iter().all(|&b| is_space(b))) else {
            return (None, BlockState::NO_CHILDREN);
        };
        let segment = code_segment(reader, pos, padding);
        (
            Some(self.arena.insert(NodeKind::CodeBlock {
                lines: vec![segment],
            })),
            BlockState::NO_CHILDREN,
        )
    }
    pub fn continue_code_block(
        &mut self,
        node: NodeId,
        reader: &mut impl BlockCursor,
    ) -> BlockState {
        let (line, segment) = reader.peek_line();
        let blank = line
            .as_ref()
            .is_none_or(|line| line.bytes().iter().all(|&b| is_space(b)));
        let segment = if blank {
            segment.trim_left_space_width(4, &reader.source().bytes())
        } else {
            let line = line.unwrap();
            let Some((pos, padding)) = indent_position(&line.bytes(), reader.line_offset()) else {
                return BlockState::CLOSE;
            };
            code_segment(reader, pos, padding)
        };
        let NodeKind::CodeBlock { lines } = &mut self.arena.node_mut(node).data else {
            panic!("not a code block")
        };
        lines.push(segment);
        BlockState::CONTINUE_NO_CHILDREN
    }
    pub fn close_code_block(&mut self, node: NodeId, reader: &impl BlockCursor) {
        let source = reader.source();
        let NodeKind::CodeBlock { lines } = &mut self.arena.node_mut(node).data else {
            panic!("not a code block")
        };
        while lines.last().is_some_and(|line| {
            line.value_shared(&source)
                .bytes()
                .iter()
                .all(|&b| is_space(b))
        }) {
            lines.pop();
        }
    }
}
