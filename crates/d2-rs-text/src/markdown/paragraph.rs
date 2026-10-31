// SPDX-License-Identifier: MPL-2.0 AND MIT
// Copyright (c) 2019 Yusuke Inuzuka
// Rust translation Copyright 2026 Nelson Chen.
// Goldmark v1.8.5 parser/paragraph.go and ast/block.go.
use super::{
    arena::NodeId,
    context::{Context, NodeKind},
    reader_api::ReaderCursor,
    text::{Segment, is_space},
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Paragraph {
    pub lines: Vec<Segment>,
    pub blank_previous_lines: bool,
}
/// The upstream BlockParser state bits returned by this parser.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlockState(pub u8);
impl BlockState {
    pub const NO_CHILDREN: Self = Self(16);
    pub const CONTINUE_NO_CHILDREN: Self = Self(18);
    pub const CLOSE: Self = Self(4);
}
impl Context {
    pub fn open_paragraph(
        &mut self,
        reader: &mut impl ReaderCursor,
    ) -> (Option<NodeId>, BlockState) {
        let (line, segment) = reader.peek_line();
        if line.is_none_or(|v| v.bytes().iter().all(|&b| is_space(b))) {
            return (None, BlockState::NO_CHILDREN);
        }
        let node = self.arena.insert(NodeKind::Paragraph(Paragraph {
            lines: vec![segment],
            blank_previous_lines: false,
        }));
        reader.advance_to_eol();
        (Some(node), BlockState::NO_CHILDREN)
    }
    pub fn continue_paragraph(
        &mut self,
        node: NodeId,
        reader: &mut impl ReaderCursor,
    ) -> BlockState {
        let (line, segment) = reader.peek_line();
        if line.is_none_or(|v| v.bytes().iter().all(|&b| is_space(b))) {
            return BlockState::CLOSE;
        }
        let NodeKind::Paragraph(paragraph) = &mut self.arena.node_mut(node).data else {
            panic!("not a paragraph")
        };
        paragraph.lines.push(segment);
        reader.advance_to_eol();
        BlockState::CONTINUE_NO_CHILDREN
    }
    pub fn close_paragraph(&mut self, node: NodeId, reader: &impl ReaderCursor) {
        let source = reader.source();
        let NodeKind::Paragraph(paragraph) = &mut self.arena.node_mut(node).data else {
            panic!("not a paragraph")
        };
        for line in &mut paragraph.lines {
            *line = line.trim_left_space(&source.bytes());
        }
        if let Some(last) = paragraph.lines.last_mut() {
            *last = last.trim_right_space(&source.bytes());
        }
        if paragraph.lines.is_empty() {
            let parent = self
                .arena
                .node(node)
                .parent
                .expect("unattached empty paragraph");
            self.arena.remove_child(parent, node);
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReferenceDefinition {
    pub reference: super::reference::ReferenceHandle,
    pub lines: Vec<Segment>,
    pub blank_previous_lines: bool,
}

impl Context {
    /// Goldmark's reference paragraph transformer, including registration of
    /// shared AST fields before processing the next definition.
    pub fn transform_paragraph_references(&mut self, node: NodeId, source: &super::bytes::Bytes) {
        use super::{
            block_reader::BlockReader, bytes::Bytes, link::parse_link_reference_definition,
            reference::Reference,
        };
        use std::{cell::RefCell, rc::Rc};
        let NodeKind::Paragraph(paragraph) = &self.arena.node(node).data else {
            panic!("not a paragraph")
        };
        let original_lines = paragraph.lines.clone();
        let blank = paragraph.blank_previous_lines;
        let mut block = BlockReader::with_source(source.clone(), Some(&original_lines));
        let mut removes = Vec::new();
        loop {
            let (definition, start, mut end) = parse_link_reference_definition(&mut block);
            if start < 0 {
                break;
            }
            let definition = definition.expect("successful definition");
            let reference = Rc::new(RefCell::new(Reference {
                label: Some(Bytes::new(&definition.label)),
                destination: Some(definition.destination),
                title: definition.title.map(|v| Bytes::new(&v)),
            }));
            let ref_node = self
                .arena
                .insert(NodeKind::ReferenceDefinition(ReferenceDefinition {
                    reference: reference.clone(),
                    lines: vec![definition.first_segment],
                    blank_previous_lines: start == 0 && blank,
                }));
            self.references.add(reference);
            let parent = self.arena.node(node).parent.expect("unattached paragraph");
            self.arena.insert_before(parent, Some(node), ref_node);
            let NodeKind::ReferenceDefinition(record) = &mut self.arena.node_mut(ref_node).data
            else {
                unreachable!()
            };
            for i in start + 1..end {
                record.lines.push(original_lines[i as usize]);
            }
            let last = record.lines.last_mut().unwrap();
            *last = last.trim_right_space(&source.bytes());
            if start == end {
                end += 1;
            }
            removes.push((start, end));
        }
        let NodeKind::Paragraph(paragraph) = &mut self.arena.node_mut(node).data else {
            unreachable!()
        };
        let mut offset = 0;
        for (start, end) in removes {
            if paragraph.lines.is_empty() {
                break;
            }
            let suffix = paragraph.lines[(end - offset) as usize..].to_vec();
            paragraph.lines.truncate((start - offset) as usize);
            paragraph.lines.extend(suffix);
            offset = end;
        }
        if paragraph.lines.is_empty() {
            let parent = self.arena.node(node).parent.expect("unattached paragraph");
            self.arena.remove_child(parent, node);
        }
    }
}
