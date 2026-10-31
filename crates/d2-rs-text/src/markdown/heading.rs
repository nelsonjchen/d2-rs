// SPDX-License-Identifier: MPL-2.0 AND MIT
// Copyright (c) 2019 Yusuke Inuzuka
// Rust translation Copyright 2026 Nelson Chen.
// Goldmark v1.8.5 parser/setext_headings.go, matchesSetextHeadingBar.
use super::text::is_space;
pub fn matches_setext_heading_bar(line: &[u8]) -> Option<u8> {
    let start = line.iter().take_while(|&&b| b == b' ').count();
    if start > 3 {
        return None;
    }
    let level1 = line[start..].iter().take_while(|&&b| b == b'=').count();
    let (mark, count) = if level1 == 0 {
        (
            b'-',
            line[start..].iter().take_while(|&&b| b == b'-').count(),
        )
    } else {
        (b'=', level1)
    };
    let mut end = line.len();
    // Preserve the original helper's empty-input precondition.
    if is_space(line[end - 1]) {
        end -= line[start..]
            .iter()
            .rev()
            .take_while(|&&b| is_space(b))
            .count();
    }
    (count > 0 && start + count == end).then_some(mark)
}

use super::{
    arena::NodeId,
    context::{Context, NodeKind},
    paragraph::{BlockState, Paragraph},
    reader_api::BlockCursor,
};
impl Context {
    /// Default Goldmark heading configuration (attributes and auto IDs disabled).
    pub fn open_setext_heading(
        &mut self,
        parent: NodeId,
        last_opened: Option<NodeId>,
        reader: &mut impl BlockCursor,
    ) -> (Option<NodeId>, BlockState) {
        let Some(last) = last_opened else {
            return (None, BlockState::NO_CHILDREN);
        };
        if !matches!(self.arena.node(last).data, NodeKind::Paragraph(_))
            || self.arena.node(last).parent != Some(parent)
        {
            return (None, BlockState::NO_CHILDREN);
        }
        let (line, segment) = reader.peek_line();
        let line = line.expect("setext parser at EOF");
        let Some(mark) = matches_setext_heading_bar(&line.bytes()) else {
            return (None, BlockState::NO_CHILDREN);
        };
        let node = self.arena.insert(NodeKind::Heading {
            level: if mark == b'-' { 2 } else { 1 },
            lines: vec![segment],
            blank_previous_lines: false,
        });
        self.temporary_paragraph = Some(last);
        (Some(node), BlockState(48))
    }
    pub fn close_setext_heading(&mut self, node: NodeId, reader: &impl BlockCursor) {
        let NodeKind::Heading { lines, .. } = &mut self.arena.node_mut(node).data else {
            panic!("not a heading")
        };
        let segment = lines[0];
        lines.clear();
        let temporary = self
            .temporary_paragraph
            .take()
            .expect("missing temporary paragraph");
        let NodeKind::Paragraph(paragraph) = &self.arena.node(temporary).data else {
            panic!("not a paragraph")
        };
        if paragraph.lines.is_empty() {
            let segment = segment.trim_left_space(&reader.source().bytes());
            let parent = self.arena.node(node).parent.expect("unattached heading");
            let next = self.arena.node(node).next;
            if let Some(next) = next
                && let NodeKind::Paragraph(paragraph) = &mut self.arena.node_mut(next).data
            {
                // Segments.Unshift first slices values[:1], bounded by capacity.
                assert!(paragraph.lines.capacity() > 0, "empty Segments.Unshift");
                paragraph.lines.insert(0, segment);
            } else {
                let paragraph = self.arena.insert(NodeKind::Paragraph(Paragraph {
                    lines: vec![segment],
                    blank_previous_lines: false,
                }));
                self.arena.insert_after(parent, node, paragraph);
            }
            self.arena.remove_child(parent, node);
        } else {
            let first = paragraph.lines[0].start;
            let copied_lines = paragraph.lines.clone();
            let blank = paragraph.blank_previous_lines;
            self.set_node_position(node, first);
            let NodeKind::Heading {
                lines,
                blank_previous_lines,
                ..
            } = &mut self.arena.node_mut(node).data
            else {
                unreachable!()
            };
            *lines = copied_lines;
            *blank_previous_lines = blank;
            if let Some(parent) = self.arena.node(temporary).parent {
                self.arena.remove_child(parent, temporary);
            }
        }
    }
}

impl Context {
    /// Goldmark ATX Open with its default heading configuration.
    pub fn open_atx_heading(
        &mut self,
        reader: &mut impl BlockCursor,
        block_offset: isize,
    ) -> (Option<NodeId>, BlockState) {
        let (line, segment) = reader.peek_line();
        if block_offset < 0 {
            return (None, BlockState::NO_CHILDREN);
        }
        let line = line.expect("ATX parser at EOF");
        let line = line.bytes();
        let pos = block_offset as usize;
        let mut end = pos;
        while end < line.len() && line[end] == b'#' {
            end += 1;
        }
        let level = end - pos;
        if level == 0 || level > 6 {
            return (None, BlockState::NO_CHILDREN);
        }
        if end == line.len() {
            return (
                Some(self.arena.insert(NodeKind::Heading {
                    level: level as u8,
                    lines: Vec::new(),
                    blank_previous_lines: false,
                })),
                BlockState::NO_CHILDREN,
            );
        }
        let left = line[end..].iter().take_while(|&&b| is_space(b)).count();
        if left == 0 {
            return (None, BlockState::NO_CHILDREN);
        }
        let start = (end + left).min(line.len() - 1);
        let node = self.arena.insert(NodeKind::Heading {
            level: level as u8,
            lines: Vec::new(),
            blank_previous_lines: false,
        });
        let source = reader.source();
        let mut span = super::text::Segment::new(
            segment.start + start as isize - segment.padding,
            segment.start + line.len() as isize - segment.padding,
        )
        .trim_right_space(&source.bytes());
        if span.start == span.stop {
            reader.advance_to_eol();
            return (Some(node), BlockState::NO_CHILDREN);
        }
        let value = span.value_shared(&source);
        let value = value.bytes();
        let mut stop = value.len();
        if stop != 0 {
            let mut i = stop - 1;
            while value[i] == b'#' && i > 0 {
                i -= 1;
            }
            if i == 0 && value[0] == b'#' {
                reader.advance_to_eol();
                return (Some(node), BlockState::NO_CHILDREN);
            }
            if i != stop - 1 && is_space(value[i]) {
                stop = i;
                stop -= value[..stop]
                    .iter()
                    .rev()
                    .take_while(|&&b| is_space(b))
                    .count();
            }
        }
        span.stop = span.start + stop as isize;
        let NodeKind::Heading { lines, .. } = &mut self.arena.node_mut(node).data else {
            unreachable!()
        };
        lines.push(span);
        reader.advance_to_eol();
        (Some(node), BlockState::NO_CHILDREN)
    }
}
