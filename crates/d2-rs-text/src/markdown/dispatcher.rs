// SPDX-License-Identifier: MPL-2.0 AND MIT
// Copyright (c) 2019 Yusuke Inuzuka
// Rust translation Copyright 2026 Nelson Chen.
// Goldmark v1.8.5 parser/parser.go parseBlock and ast/inline.go text merging.
use super::{
    arena::NodeId,
    block_reader::BlockReader,
    context::{Context, NodeKind},
    inline::Text,
    text::{Segment, is_space},
};
#[derive(Clone, Copy, Debug, Default)]
pub struct InlineOptions {
    pub escaped_space: bool,
    pub strikethrough: bool,
}
impl Context {
    pub fn node_position(&self, id: NodeId) -> isize {
        let n = self.arena.node(id);
        match &n.data {
            NodeKind::Document => 0,
            NodeKind::Text(t) => t.segment.start,
            NodeKind::Paragraph(p) => p.lines.first().map_or(-1, |s| s.start),
            NodeKind::ReferenceDefinition(p) => p.lines.first().map_or(-1, |s| s.start),
            NodeKind::Link(v) | NodeKind::Image(v) => v.position,
            NodeKind::Emphasis { position, .. } | NodeKind::Strikethrough { position } => *position,
            _ => n.position.unwrap_or(-1),
        }
    }
    pub fn set_node_position(&mut self, id: NodeId, value: isize) {
        let n = self.arena.node_mut(id);
        match &mut n.data {
            NodeKind::Link(v) | NodeKind::Image(v) => v.position = value,
            NodeKind::Emphasis { position, .. } | NodeKind::Strikethrough { position } => {
                *position = value
            }
            _ => n.position = Some(value),
        }
    }
    pub fn merge_or_append_text_segment(&mut self, parent: NodeId, segment: Segment) {
        if let Some(previous) = self.arena.node(parent).last
            && let NodeKind::Text(text) = &mut self.arena.node_mut(previous).data
            && text.segment.stop == segment.start
            && !text.soft_line_break
        {
            text.segment = text.segment.with_stop(segment.stop);
        } else {
            let node = self.arena.insert(NodeKind::Text(Text::new(segment)));
            self.arena.append_child(parent, node);
        }
    }
    /// Inline parsing over an already constructed block's line segments. Block
    /// discovery and paragraph transformations precede this entry point.
    pub fn parse_inline_block<'a>(
        &mut self,
        parent: NodeId,
        block: &mut BlockReader<'a>,
        lines: &'a [Segment],
        raw_parent: bool,
        options: InlineOptions,
    ) {
        if raw_parent {
            return;
        }
        let mut escaped = false;
        let source = block.source();
        block.reset(lines);
        'retry: loop {
            let (line, _) = block.peek_line();
            let Some(line) = line else { break };
            let (length, hard, visible, soft) = {
                let bytes = line.bytes();
                let len = bytes.len();
                let newline = bytes[len - 1] == b'\n';
                if newline
                    && ((len >= 3 && bytes[len - 2] == b'\\' && bytes[len - 3] != b'\\')
                        || (len == 2 && bytes[len - 2] == b'\\'))
                {
                    (len - 2, true, true, false)
                } else if newline
                    && ((len >= 4
                        && bytes[len - 3] == b'\\'
                        && bytes[len - 2] == b'\r'
                        && bytes[len - 4] != b'\\')
                        || (len == 3 && bytes[len - 3] == b'\\' && bytes[len - 2] == b'\r'))
                {
                    (len - 3, true, true, false)
                } else if newline && len >= 3 && bytes[len - 3] == b' ' && bytes[len - 2] == b' ' {
                    (len - 3, true, false, false)
                } else if newline
                    && len >= 4
                    && bytes[len - 4] == b' '
                    && bytes[len - 3] == b' '
                    && bytes[len - 2] == b'\r'
                {
                    (len - 4, true, false, false)
                } else {
                    (len, false, false, newline)
                }
            };
            let (start_line, mut start) = block.position();
            let mut n = 0;
            for i in 0..length {
                let c = line.bytes()[i];
                if c == b'\n' {
                    break;
                }
                let space = is_space(c) && c != b'\r' && c != b'\n';
                let punct = c.is_ascii_punctuation();
                if (punct && !escaped) || (space && !(escaped && options.escaped_space)) || i == 0 {
                    let trigger = if space || (i == 0 && !punct) { b' ' } else { c };
                    if matches!(trigger, 96 | b'[' | b']' | b'!' | b'<' | b'*' | b'_')
                        || (trigger == b'~' && options.strikethrough)
                    {
                        block.advance(n);
                        n = 0;
                        let saved = block.position();
                        if i != 0 {
                            self.merge_or_append_text_segment(
                                parent,
                                start.between(block.position().1),
                            );
                            start = block.position().1;
                        }
                        let node = match trigger {
                            96 => Some(self.parse_code_span(block)),
                            b'[' | b']' | b'!' => self.parse_link(parent, block),
                            b'<' => {
                                let node = self.parse_autolink(block);
                                if node.is_none() {
                                    block.set_position(saved.0, saved.1);
                                    self.parse_raw_html(block)
                                } else {
                                    node
                                }
                            }
                            b'*' | b'_' => self.parse_emphasis(block),
                            b'~' => self.parse_strikethrough(block),
                            _ => unreachable!(),
                        };
                        if let Some(node) = node {
                            if self.node_position(node) < 0 {
                                self.set_node_position(node, start.start);
                            }
                            self.arena.append_child(parent, node);
                            continue 'retry;
                        }
                        block.set_position(saved.0, saved.1);
                    }
                }
                if escaped {
                    escaped = false;
                    n += 1;
                    continue;
                }
                if c == b'\\' {
                    escaped = true;
                    n += 1;
                    continue;
                }
                escaped = false;
                n += 1;
            }
            if n != 0 {
                block.advance(n);
            }
            let (current_line, current) = block.position();
            if current_line != start_line {
                continue;
            }
            let diff = start.between(current);
            let segment = if hard && visible {
                diff
            } else {
                diff.trim_right_space(&source.bytes())
            };
            let node = self.arena.insert(NodeKind::Text(Text {
                soft_line_break: soft,
                hard_line_break: hard,
                ..Text::new(segment)
            }));
            self.arena.append_child(parent, node);
            block.advance_line();
        }
        self.process_delimiters(None);
        self.close_link_block();
    }
}
