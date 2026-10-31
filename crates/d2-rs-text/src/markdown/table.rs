// SPDX-License-Identifier: MPL-2.0 AND MIT
// Copyright (c) 2019 Yusuke Inuzuka
// Rust translation Copyright 2026 Nelson Chen.
// Goldmark v1.8.5 extension/table.go and extension/ast/table.go.
use std::{cell::RefCell, rc::Rc};
pub type Alignments = Rc<RefCell<Vec<u8>>>;
use super::{
    arena::NodeId,
    bytes::Bytes,
    context::{Context, NodeKind},
    text::{Segment, is_space},
};
#[derive(Clone, Debug)]
pub struct EscapedPipeCell {
    pub cell: NodeId,
    pub positions: Vec<isize>,
    pub transformed: bool,
}
pub fn parse_table_delimiter(segment: Segment, source: &Bytes) -> Option<Vec<u8>> {
    let line = segment.value_shared(source);
    let bytes = line.bytes();
    let mut width = 0;
    for &b in bytes.iter() {
        match b {
            b' ' => width += 1,
            b'\t' => width += 4 - width % 4,
            _ => break,
        }
    }
    if width > 3
        || bytes.iter().all(|&b| b == b'-')
        || bytes
            .iter()
            .any(|&b| !is_space(b) && !matches!(b, b'-' | b'|' | b':'))
    {
        return None;
    }
    let mut cols: Vec<_> = bytes.split(|&b| b == b'|').collect();
    if cols[0].iter().all(|&b| is_space(b)) {
        cols.remove(0);
    }
    if cols.last().is_some_and(|v| v.iter().all(|&b| is_space(b))) {
        cols.pop();
    }
    let mut result = Vec::new();
    for col in cols {
        let start = col.iter().position(|&b| !is_space(b)).unwrap_or(col.len());
        let end = col
            .iter()
            .rposition(|&b| !is_space(b))
            .map_or(start, |n| n + 1);
        let mut value = &col[start..end];
        let left = value.first() == Some(&b':');
        if left {
            value = &value[1..];
        }
        let right = value.last() == Some(&b':');
        if right {
            value = &value[..value.len() - 1];
        }
        if value.is_empty() || !value.iter().all(|&b| b == b'-') {
            return None;
        }
        result.push(match (left, right) {
            (true, false) => 1,
            (false, true) => 2,
            (true, true) => 3,
            _ => 4,
        });
    }
    if result.is_empty() {
        None
    } else {
        Some(result)
    }
}
impl Context {
    pub fn parse_table_row(
        &mut self,
        original: Segment,
        alignments: Option<&[u8]>,
        header: bool,
        source: &Bytes,
    ) -> NodeId {
        self.parse_table_row_shared(
            original,
            alignments.map(|v| Rc::new(RefCell::new(v.to_vec()))),
            header,
            source,
        )
    }
    pub fn parse_table_row_shared(
        &mut self,
        original: Segment,
        alignments: Option<Alignments>,
        header: bool,
        source: &Bytes,
    ) -> NodeId {
        let segment = original
            .trim_left_space(&source.bytes())
            .trim_right_space(&source.bytes());
        let line = segment.value_shared(source);
        let bytes = line.bytes();
        let row = self.arena.insert(NodeKind::TableRow {
            alignments: alignments.clone(),
        });
        self.set_node_position(row, original.start);
        let borrowed = alignments.as_ref().map(|v| v.borrow());
        let alignments = borrowed.as_deref().map_or(&[][..], |v| v.as_slice());
        let mut pos = usize::from(bytes.first() == Some(&b'|'));
        let limit = bytes.len() - usize::from(bytes.last() == Some(&b'|'));
        let mut i = 0;
        while pos < limit {
            if i >= alignments.len() && !header {
                return row;
            }
            let alignment = alignments.get(i).copied().unwrap_or(4);
            let cell = self.arena.insert(NodeKind::TableCell {
                alignment,
                lines: Vec::new(),
            });
            self.set_node_position(cell, original.start + pos as isize - original.padding);
            let mut backtick = false;
            let mut closure = pos;
            let mut escaped_cell = None;
            while closure < limit {
                if bytes[closure] == 96 {
                    backtick = true;
                }
                if bytes[closure] == b'|' {
                    if closure == 0 || bytes[closure - 1] != b'\\' {
                        break;
                    }
                    if backtick {
                        let cells = self.escaped_pipe_cells.get_or_insert_with(Vec::new);
                        let index = *escaped_cell.get_or_insert_with(|| {
                            let index = cells.len();
                            cells.push(EscapedPipeCell {
                                cell,
                                positions: Vec::new(),
                                transformed: false,
                            });
                            index
                        });
                        cells[index]
                            .positions
                            .push(segment.start + closure as isize - 1);
                    }
                }
                closure += 1;
            }
            let span = Segment::new(
                segment.start + pos as isize,
                segment.start + closure as isize,
            )
            .trim_left_space(&source.bytes())
            .trim_right_space(&source.bytes());
            let NodeKind::TableCell { lines, .. } = &mut self.arena.node_mut(cell).data else {
                unreachable!()
            };
            lines.push(span);
            self.arena.append_child(row, cell);
            pos = closure + 1;
            i += 1;
        }
        while i < alignments.len() {
            let cell = self.arena.insert(NodeKind::TableCell {
                alignment: 4,
                lines: Vec::new(),
            });
            self.arena.append_child(row, cell);
            i += 1;
        }
        row
    }
}

impl Context {
    pub fn transform_paragraph_table(&mut self, node: NodeId, source: &Bytes) {
        let position = self.node_position(node);
        let NodeKind::Paragraph(paragraph) = &self.arena.node(node).data else {
            panic!("not a paragraph")
        };
        let lines = paragraph.lines.clone();
        for i in 1..lines.len() {
            let Some(values) = parse_table_delimiter(lines[i], source) else {
                continue;
            };
            let alignments = Rc::new(RefCell::new(values));
            let header_row =
                self.parse_table_row_shared(lines[i - 1], Some(alignments.clone()), true, source);
            if alignments.borrow().len() as isize != self.arena.node(header_row).child_count {
                return;
            }
            let table = self.arena.insert(NodeKind::Table {
                alignments: Some(alignments.clone()),
            });
            self.set_node_position(table, position);
            let header = self
                .arena
                .insert(NodeKind::TableHeader { alignments: None });
            self.set_node_position(header, self.node_position(header_row));
            while let Some(child) = self.arena.node(header_row).first {
                self.arena.append_child(header, child);
            }
            self.arena.append_child(table, header);
            for &line in &lines[i + 1..] {
                let row =
                    self.parse_table_row_shared(line, Some(alignments.clone()), false, source);
                self.arena.append_child(table, row);
            }
            let NodeKind::Paragraph(paragraph) = &mut self.arena.node_mut(node).data else {
                unreachable!()
            };
            paragraph.lines.truncate(i - 1);
            let parent = self.arena.node(node).parent.expect("unattached paragraph");
            self.arena.insert_after(parent, node, table);
            if i == 1 {
                self.arena.remove_child(parent, node);
            } else {
                let NodeKind::Paragraph(paragraph) = &mut self.arena.node_mut(node).data else {
                    unreachable!()
                };
                // Source modifies Stop directly, preserving ForceNewline.
                paragraph.lines[i - 2].stop -= 1;
            }
            // Source's loop sees the shortened line list and terminates.
            return;
        }
    }
}

impl Context {
    /// Goldmark tableASTTransformer.Transform. Consume the deferred records
    /// before walking, including records whose cells are no longer attached.
    pub fn transform_table_escaped_pipes(&mut self) {
        let Some(mut entries) = self.escaped_pipe_cells.take() else {
            return;
        };
        for index in 0..entries.len() {
            if !entries[index].transformed {
                self.transform_table_pipe_subtree(entries[index].cell, &mut entries);
            }
        }
    }

    fn transform_table_pipe_subtree(&mut self, node: NodeId, entries: &mut [EscapedPipeCell]) {
        if matches!(self.arena.node(node).data, NodeKind::CodeSpan) {
            let mut child = self.arena.node(node).first;
            while let Some(original) = child {
                child = self.arena.node(original).next;
                let NodeKind::Text(text) = &self.arena.node(original).data else {
                    continue;
                };
                // Upstream keeps the original segment pointer for membership
                // checks even after replacing the current tail multiple times.
                let original_segment = text.segment;
                let mut tail = original;
                for entry in entries.iter_mut() {
                    for &position in &entry.positions {
                        if original_segment.start <= position && position < original_segment.stop {
                            let NodeKind::Text(text) = &self.arena.node(tail).data else {
                                unreachable!()
                            };
                            let segment = text.segment;
                            let first = self.arena.insert(NodeKind::Text(super::inline::Text {
                                segment: segment.with_stop(position),
                                raw: true,
                                ..Default::default()
                            }));
                            let second = self.arena.insert(NodeKind::Text(super::inline::Text {
                                segment: segment.with_start(position + 1),
                                raw: true,
                                ..Default::default()
                            }));
                            self.arena.insert_after(node, tail, first);
                            self.arena.insert_after(node, first, second);
                            self.arena.remove_child(node, tail);
                            tail = second;
                            entry.transformed = true;
                        }
                    }
                }
            }
        }
        let mut child = self.arena.node(node).first;
        while let Some(id) = child {
            self.transform_table_pipe_subtree(id, entries);
            child = self.arena.node(id).next;
        }
    }
}
