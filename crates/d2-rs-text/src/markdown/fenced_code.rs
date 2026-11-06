// SPDX-License-Identifier: MPL-2.0 AND MIT
// Copyright (c) 2019 Yusuke Inuzuka
// Rust translation Copyright 2026 Nelson Chen.
// Goldmark v1.8.5 parser/fcode_block.go.
use super::{
    arena::NodeId,
    context::{Context, NodeKind},
    inline::Text,
    paragraph::BlockState,
    reader_api::BlockCursor,
    text::{Segment, is_space},
};
#[derive(Clone, Debug)]
pub struct FenceData {
    pub character: u8,
    pub indent: isize,
    pub length: usize,
    pub node: NodeId,
}
impl Context {
    pub fn open_fenced_code(
        &mut self,
        reader: &mut impl BlockCursor,
        block_offset: isize,
    ) -> (Option<NodeId>, BlockState) {
        let (line, segment) = reader.peek_line();
        if block_offset < 0 {
            return (None, BlockState::NO_CHILDREN);
        }
        let line = line.expect("fence parser at EOF");
        let line = line.bytes();
        let pos = block_offset as usize;
        let character = line[pos];
        if !matches!(character, b'~' | 96) {
            return (None, BlockState::NO_CHILDREN);
        }
        let mut end = pos;
        while end < line.len() && line[end] == character {
            end += 1;
        }
        let length = end - pos;
        if length < 3 {
            return (None, BlockState::NO_CHILDREN);
        }
        let mut info = None;
        if end < line.len() - 1 {
            let rest = &line[end..];
            let left = rest.iter().take_while(|&&b| is_space(b)).count();
            let right = rest.iter().rev().take_while(|&&b| is_space(b)).count();
            if left < rest.len() - right {
                let start = segment.start - segment.padding + (end + left) as isize;
                let stop = segment.stop - right as isize;
                if character == 96 && rest[left..rest.len() - right].contains(&96) {
                    return (None, BlockState::NO_CHILDREN);
                }
                if start != stop {
                    info = Some(
                        self.arena
                            .insert(NodeKind::Text(Text::new(Segment::new(start, stop)))),
                    );
                }
            }
        }
        let node = self.arena.insert(NodeKind::FencedCodeBlock {
            info,
            lines: Vec::new(),
        });
        self.fence = Some(FenceData {
            character,
            indent: block_offset,
            length,
            node,
        });
        (Some(node), BlockState::NO_CHILDREN)
    }
    pub fn continue_fenced_code(
        &mut self,
        node: NodeId,
        reader: &mut impl BlockCursor,
    ) -> BlockState {
        let (line, segment) = reader.peek_line();
        let line = line.map(|line| line.bytes().to_vec()).unwrap_or_default();
        let fence = self.fence.as_ref().expect("missing fence context");
        let offset = reader.line_offset();
        let mut width = 0;
        let mut pos = 0;
        while pos < line.len() {
            match line[pos] {
                b' ' => width += 1,
                b'\t' => width += 4 - (offset + width) % 4,
                _ => break,
            }
            pos += 1;
        }
        if width < 4 {
            let mut end = pos;
            while end < line.len() && line[end] == fence.character {
                end += 1;
            }
            if end - pos >= fence.length && line[end..].iter().all(|&b| is_space(b)) {
                reader.advance_to_eol();
                return BlockState::CLOSE;
            }
        }
        let mut consumed = 0isize;
        let mut width = 0;
        let mut virtual_padding = segment.padding;
        if fence.indent != 0 {
            for &byte in &line {
                if virtual_padding > 0 {
                    virtual_padding -= 1;
                    width += 1;
                    consumed += 1;
                    continue;
                }
                if width >= fence.indent {
                    break;
                }
                match byte {
                    b' ' => width += 1,
                    b'\t' => width += 4 - (offset + width) % 4,
                    _ => break,
                }
                consumed += 1;
            }
        }
        let (pos, padding) = if fence.indent == 0 {
            (0, segment.padding)
        } else if width >= fence.indent {
            (consumed - segment.padding, width - fence.indent)
        } else {
            (
                line.iter()
                    .position(|&b| b != b' ' && b != b'\t')
                    .filter(|&i| line[i] != b'\n')
                    .unwrap_or(0) as isize
                    - segment.padding,
                0,
            )
        };
        let mut span = Segment::new(segment.start + pos, segment.stop);
        span.padding = padding;
        if padding != 0 {
            let offset = reader.line_offset() + fence.indent;
            let (line, saved) = reader.position();
            reader.set_position(line, Segment::new(saved.start - 1, saved.stop));
            if offset == reader.line_offset() {
                span.padding = 0;
                span.start -= 1;
            }
            reader.set_position(line, saved);
        }
        span.force_newline = true;
        let NodeKind::FencedCodeBlock { lines, .. } = &mut self.arena.node_mut(node).data else {
            panic!("not fenced code")
        };
        lines.push(span);
        reader.advance_to_eol();
        BlockState::CONTINUE_NO_CHILDREN
    }
    pub fn close_fenced_code(&mut self, node: NodeId) {
        if self.fence.as_ref().expect("missing fence context").node == node {
            self.fence = None;
        }
    }
}
