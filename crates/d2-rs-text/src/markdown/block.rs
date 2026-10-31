// SPDX-License-Identifier: MPL-2.0 AND MIT
// Copyright (c) 2019 Yusuke Inuzuka
// Rust translation Copyright 2026 Nelson Chen.
// Goldmark v1.8.5 parser/{blockquote,thematic_break}.go and util.IndentWidth.
use super::{
    arena::NodeId,
    context::{Context, NodeKind},
    paragraph::BlockState,
    reader_api::BlockCursor,
    text::is_space,
};

pub fn indent_width(line: &[u8], offset: isize) -> (isize, usize) {
    let mut width = 0;
    for (pos, &byte) in line.iter().enumerate() {
        match byte {
            b' ' => width += 1,
            b'\t' => width += 4 - (offset + width) % 4,
            _ => return (width, pos),
        }
    }
    (width, line.len())
}
pub fn is_thematic_break(line: &[u8], offset: isize) -> bool {
    let (width, pos) = indent_width(line, offset);
    if width > 3 {
        return false;
    }
    let mut mark = 0;
    let mut count = 0;
    for &byte in &line[pos..] {
        if is_space(byte) {
            continue;
        }
        if mark == 0 {
            mark = byte;
            count = 1;
            if matches!(mark, b'*' | b'-' | b'_') {
                continue;
            }
            return false;
        }
        if byte != mark {
            return false;
        }
        count += 1;
    }
    count > 2
}
pub fn process_blockquote(reader: &mut impl BlockCursor) -> bool {
    let (line, _) = reader.peek_line();
    let Some(line) = line else { return false };
    let line = line.bytes();
    let (width, mut pos) = indent_width(&line, reader.line_offset());
    if width > 3 || pos >= line.len() || line[pos] != b'>' {
        return false;
    }
    pos += 1;
    reader.advance(pos as isize);
    if pos >= line.len() || line[pos] == b'\n' {
        return true;
    }
    if matches!(line[pos], b' ' | b'\t') {
        let padding = if line[pos] == b'\t' {
            4 - reader.line_offset() % 4 - 1
        } else {
            0
        };
        reader.advance_and_set_padding(1, padding);
    }
    true
}
impl Context {
    pub fn open_thematic_break(
        &mut self,
        reader: &mut impl BlockCursor,
    ) -> (Option<NodeId>, BlockState) {
        let (line, _) = reader.peek_line();
        let matches =
            line.is_some_and(|line| is_thematic_break(&line.bytes(), reader.line_offset()));
        if !matches {
            return (None, BlockState::NO_CHILDREN);
        }
        reader.advance_to_eol();
        (
            Some(self.arena.insert(NodeKind::ThematicBreak)),
            BlockState::NO_CHILDREN,
        )
    }
    pub fn open_blockquote(
        &mut self,
        reader: &mut impl BlockCursor,
    ) -> (Option<NodeId>, BlockState) {
        if process_blockquote(reader) {
            (Some(self.arena.insert(NodeKind::Blockquote)), BlockState(8))
        } else {
            (None, BlockState::NO_CHILDREN)
        }
    }
    pub fn continue_blockquote(&mut self, reader: &mut impl BlockCursor) -> BlockState {
        if process_blockquote(reader) {
            BlockState(10)
        } else {
            BlockState::CLOSE
        }
    }
}
