// SPDX-License-Identifier: MPL-2.0 AND MIT
// Copyright (c) 2019 Yusuke Inuzuka
// Rust translation Copyright 2026 Nelson Chen.
// Goldmark v1.8.5 parser/list.go, parseListItem and calcListOffset.

use super::text::is_space;
use super::{
    arena::NodeId,
    context::{Context, NodeKind},
    paragraph::BlockState,
    reader_api::BlockCursor,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListItemType {
    NotList,
    Bullet,
    Ordered,
}

/// The six offsets returned by Goldmark's private parseListItem helper.
pub fn parse_list_item(line: &[u8]) -> ([isize; 6], ListItemType) {
    let mut i = 0usize;
    let length = line.len();
    let mut result = [0isize; 6];
    while i < length && line[i] == b' ' {
        i += 1;
    }
    if i > 3 {
        return (result, ListItemType::NotList);
    }
    result[1] = i as isize;
    result[2] = i as isize;
    let kind = if i < length && matches!(line[i], b'-' | b'*' | b'+') {
        i += 1;
        result[3] = i as isize;
        ListItemType::Bullet
    } else if i < length {
        let start = i;
        while i < length && line[i].is_ascii_digit() {
            i += 1;
        }
        result[3] = i as isize;
        if i == start || i - start > 9 {
            return (result, ListItemType::NotList);
        }
        if i < length && matches!(line[i], b'.' | b')') {
            i += 1;
            result[3] = i as isize;
        } else {
            return (result, ListItemType::NotList);
        }
        ListItemType::Ordered
    } else {
        return (result, ListItemType::NotList);
    };
    if i < length && line[i] != b'\n' {
        let width = line[i..]
            .iter()
            .take_while(|&&byte| byte == b' ' || byte == b'\t')
            .count();
        if width == 0 {
            return (result, ListItemType::NotList);
        }
    }
    if i >= length {
        result[4] = -1;
        result[5] = -1;
        return (result, kind);
    }
    result[4] = i as isize;
    result[5] = length as isize;
    if line[length - 1] == b'\n' && line[i] != b'\n' {
        result[5] -= 1;
    }
    (result, kind)
}

pub fn calc_list_offset(source: &[u8], matched: [isize; 6]) -> isize {
    let start = matched[4];
    if start < 0 || source[start as usize..].iter().all(|&byte| is_space(byte)) {
        return 1;
    }
    let mut width = 0isize;
    for &byte in &source[start as usize..] {
        match byte {
            b' ' => width += 1,
            b'\t' => width += 4 - (start + width) % 4,
            _ => break,
        }
    }
    if width > 4 { 1 } else { width }
}

fn indent_position(line: &[u8], current: isize, width: isize) -> Option<(isize, isize)> {
    let mut visual = 0isize;
    let mut index = 0isize;
    while (index as usize) < line.len() && visual < width {
        match line[index as usize] {
            b' ' => visual += 1,
            b'\t' => visual += 4 - (current + visual) % 4,
            _ => break,
        }
        index += 1;
    }
    (visual >= width).then_some((index, visual - width))
}

fn last_item_offset(context: &Context, list: NodeId) -> isize {
    context.arena.node(list).last.map_or(0, |id| {
        let NodeKind::ListItem { offset, .. } = context.arena.node(id).data else {
            panic!("list child is not a list item")
        };
        offset
    })
}

fn list_compatible(context: &Context, list: NodeId, marker: u8, ordered: bool) -> bool {
    let NodeKind::List {
        marker: current, ..
    } = context.arena.node(list).data
    else {
        return false;
    };
    marker == current && ordered == matches!(current, b'.' | b')')
}

impl Context {
    pub fn open_list(
        &mut self,
        parent: NodeId,
        last_opened: Option<NodeId>,
        reader: &mut impl BlockCursor,
    ) -> (Option<NodeId>, BlockState) {
        if last_opened.is_some_and(|id| matches!(self.arena.node(id).data, NodeKind::List { .. }))
            || self.skip_list_parser
        {
            self.skip_list_parser = false;
            return (None, BlockState::NO_CHILDREN);
        }
        let (line, _) = reader.peek_line();
        let Some(line) = line else {
            return (None, BlockState::NO_CHILDREN);
        };
        let bytes = line.bytes();
        let (matched, kind) = parse_list_item(&bytes);
        if kind == ListItemType::NotList {
            return (None, BlockState::NO_CHILDREN);
        }
        let start = if kind == ListItemType::Ordered {
            std::str::from_utf8(&bytes[matched[2] as usize..matched[3] as usize - 1])
                .ok()
                .and_then(|value| value.parse::<i64>().ok())
                .unwrap_or(-1)
        } else {
            -1
        };
        if let Some(last) = last_opened
            && matches!(self.arena.node(last).data, NodeKind::Paragraph(_))
            && self.arena.node(last).parent == Some(parent)
            && ((kind == ListItemType::Ordered && start != 1)
                || matched[4] < 0
                || bytes[matched[4] as usize..matched[5] as usize]
                    .iter()
                    .all(|&byte| is_space(byte)))
        {
            return (None, BlockState::NO_CHILDREN);
        }
        let marker = bytes[matched[3] as usize - 1];
        self.empty_list_item_with_blank_lines = false;
        let node = self.arena.insert(NodeKind::List {
            marker,
            start,
            is_tight: true,
        });
        (Some(node), BlockState(8))
    }

    pub fn open_list_item(
        &mut self,
        list: NodeId,
        reader: &mut impl BlockCursor,
    ) -> (Option<NodeId>, BlockState) {
        if !matches!(self.arena.node(list).data, NodeKind::List { .. }) {
            return (None, BlockState::NO_CHILDREN);
        }
        let offset = last_item_offset(self, list);
        let (line, _) = reader.peek_line();
        let Some(line) = line else {
            return (None, BlockState::NO_CHILDREN);
        };
        let bytes = line.bytes();
        let (matched, kind) = parse_list_item(&bytes);
        if kind == ListItemType::NotList || matched[1] - offset > 3 {
            return (None, BlockState::NO_CHILDREN);
        }
        let marker = bytes[matched[3] as usize - 1];
        if !list_compatible(self, list, marker, kind == ListItemType::Ordered) {
            return (None, BlockState::NO_CHILDREN);
        }
        self.empty_list_item_with_blank_lines = false;
        let item_offset = matched[3] + calc_list_offset(&bytes, matched);
        let node = self.arena.insert(NodeKind::ListItem {
            offset: item_offset,
            blank_previous_lines: false,
        });
        if matched[4] < 0
            || bytes[matched[4] as usize..matched[5] as usize]
                .iter()
                .all(|&byte| is_space(byte))
        {
            return (Some(node), BlockState::NO_CHILDREN);
        }
        let (position, padding) = indent_position(
            &bytes[matched[4] as usize..],
            matched[4],
            calc_list_offset(&bytes, matched),
        )
        .expect("list item content must have an indent");
        reader.advance_and_set_padding(matched[3] + position, padding);
        (Some(node), BlockState(8))
    }

    pub fn continue_list(&mut self, list: NodeId, reader: &mut impl BlockCursor) -> BlockState {
        let (line, _) = reader.peek_line();
        let Some(line) = line else {
            return BlockState::CLOSE;
        };
        let bytes = line.bytes();
        let last = self.arena.node(list).last;
        let last_is_empty = last.is_none_or(|id| self.arena.node(id).child_count == 0);
        if bytes.iter().all(|&byte| is_space(byte)) {
            if last_is_empty {
                self.empty_list_item_with_blank_lines = true;
            }
            return BlockState(10);
        }
        let offset = last_item_offset(self, list);
        let width = bytes
            .iter()
            .enumerate()
            .take_while(|&(_, &byte)| byte == b' ' || byte == b'\t')
            .fold(0isize, |width, (index, &byte)| {
                width
                    + if byte == b'\t' {
                        4 - (reader.line_offset() + index as isize + width) % 4
                    } else {
                        1
                    }
            });
        if width < offset || last_is_empty {
            if width < 4 {
                let (matched, kind) = parse_list_item(&bytes);
                if kind != ListItemType::NotList && matched[1] - offset < 4 {
                    let marker = bytes[matched[3] as usize - 1];
                    if !list_compatible(self, list, marker, kind == ListItemType::Ordered) {
                        return BlockState::CLOSE;
                    }
                    if super::block::is_thematic_break(&bytes[matched[3] as usize - 1..], 0) {
                        let last_opened = self.arena.node(list).previous;
                        let is_heading = last_opened.is_some_and(|id| {
                            matches!(self.arena.node(id).data, NodeKind::Paragraph(_))
                                && super::heading::matches_setext_heading_bar(
                                    &bytes[matched[3] as usize - 1..],
                                ) == Some(b'-')
                        });
                        if !is_heading {
                            return BlockState::CLOSE;
                        }
                    }
                    return BlockState(10);
                }
            }
            if !last_is_empty {
                return BlockState::CLOSE;
            }
        }
        if last_is_empty && width < offset {
            return BlockState::CLOSE;
        }
        if self.empty_list_item_with_blank_lines {
            BlockState::CLOSE
        } else {
            BlockState(10)
        }
    }

    pub fn continue_list_item(
        &mut self,
        item: NodeId,
        reader: &mut impl BlockCursor,
    ) -> BlockState {
        let (line, _) = reader.peek_line();
        let Some(line) = line else {
            return BlockState::CLOSE;
        };
        let bytes = line.bytes();
        if bytes.iter().all(|&byte| is_space(byte)) {
            // Goldmark listItemParser.Continue consumes this line without
            // changing whether the item had blank lines before it. The next
            // item's opener owns that flag; a trailing blank is not looseness.
            reader.advance_to_eol();
            return BlockState(10);
        }
        let list = self
            .arena
            .node(item)
            .parent
            .expect("list item is unattached");
        let offset = last_item_offset(self, list);
        let indent = bytes
            .iter()
            .enumerate()
            .take_while(|&(_, &byte)| byte == b' ' || byte == b'\t')
            .fold(0isize, |width, (index, &byte)| {
                width
                    + if byte == b'\t' {
                        4 - (reader.line_offset() + index as isize + width) % 4
                    } else {
                        1
                    }
            });
        let is_empty =
            self.arena.node(item).child_count == 0 && self.empty_list_item_with_blank_lines;
        if (is_empty || indent < offset) && indent < 4 {
            let (_, kind) = parse_list_item(&bytes);
            if kind != ListItemType::NotList {
                self.skip_list_parser = true;
                return BlockState::CLOSE;
            }
            if !is_empty {
                return BlockState::CLOSE;
            }
        }
        let Some((position, padding)) = indent_position(&bytes, reader.line_offset(), offset)
        else {
            return BlockState::CLOSE;
        };
        reader.advance_and_set_padding(position, padding);
        BlockState(10)
    }

    pub fn close_list(&mut self, list: NodeId) {
        if !matches!(self.arena.node(list).data, NodeKind::List { .. }) {
            panic!("not a list")
        }
        let mut tight = true;
        let first = self.arena.node(list).first;
        let mut item = first;
        while let Some(item_id) = item {
            if Some(item_id) != first
                && matches!(
                    self.arena.node(item_id).data,
                    NodeKind::ListItem {
                        blank_previous_lines: true,
                        ..
                    }
                )
            {
                tight = false;
            }
            let mut child = self.arena.node(item_id).first;
            let first_child = child;
            while let Some(child_id) = child {
                if Some(child_id) != first_child && self.block_has_blank_previous_lines(child_id) {
                    tight = false;
                }
                child = self.arena.node(child_id).next;
            }
            item = self.arena.node(item_id).next;
        }
        if let NodeKind::List { is_tight, .. } = &mut self.arena.node_mut(list).data {
            *is_tight = tight;
        }
        if !tight {
            return;
        }
        let mut item = self.arena.node(list).first;
        while let Some(item_id) = item {
            let mut child = self.arena.node(item_id).first;
            while let Some(child_id) = child {
                child = self.arena.node(child_id).next;
                let replacement = match &self.arena.node(child_id).data {
                    NodeKind::Paragraph(paragraph) => Some(NodeKind::TextBlock {
                        lines: paragraph.lines.clone(),
                        blank_previous_lines: paragraph.blank_previous_lines,
                    }),
                    _ => None,
                };
                if let Some(replacement) = replacement {
                    let replacement_id = self.arena.insert(replacement);
                    self.arena.replace_child(item_id, child_id, replacement_id);
                }
            }
            item = self.arena.node(item_id).next;
        }
    }
}
