// SPDX-License-Identifier: MPL-2.0 AND MIT
// Copyright (c) 2019 Yusuke Inuzuka
// Rust translation Copyright 2026 Nelson Chen.
// Goldmark v1.8.5 parser/code_span.go and ast/inline.go. MIT terms in notices.
use super::{
    bytes::Bytes,
    reader_api::ReaderCursor,
    text::{Segment, is_space},
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Text {
    pub segment: Segment,
    pub raw: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub soft_line_break: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub hard_line_break: bool,
}

fn is_false(value: &bool) -> bool {
    !value
}

impl Text {
    pub fn new(segment: Segment) -> Self {
        Self {
            segment,
            ..Self::default()
        }
    }

    /// Goldmark Text.Merge; unlike the parent append helper this checks rawness,
    /// source newlines and right-hand padding, and copies right-hand break flags.
    pub fn merge(&mut self, node: &InlineNode, source: &Bytes) -> bool {
        let InlineNode::Text(other) = node else {
            return false;
        };
        if self.segment.stop != other.segment.start
            || other.segment.padding != 0
            || source.bytes()[usize::try_from(self.segment.stop - 1).unwrap()] == b'\n'
            || self.raw != other.raw
        {
            return false;
        }
        self.segment.stop = other.segment.stop;
        self.soft_line_break = other.soft_line_break;
        self.hard_line_break = other.hard_line_break;
        true
    }
}

pub fn merge_or_append_text_segment(children: &mut Vec<InlineNode>, segment: Segment) {
    if let Some(InlineNode::Text(last)) = children.last_mut()
        && last.segment.stop == segment.start
        && !last.soft_line_break
    {
        last.segment = last.segment.with_stop(segment.stop);
        return;
    }
    children.push(InlineNode::Text(Text::new(segment)));
}

pub fn merge_or_replace_text_segment(
    children: &mut Vec<InlineNode>,
    index: usize,
    segment: Segment,
) {
    assert!(index < children.len());
    if index > 0
        && let InlineNode::Text(previous) = &mut children[index - 1]
        && previous.segment.stop == segment.start
        && !previous.soft_line_break
    {
        previous.segment = previous.segment.with_stop(segment.stop);
        children.remove(index);
        return;
    }
    children[index] = InlineNode::Text(Text::new(segment));
}

/// Initial inline AST records; the remaining inline node kinds are pending.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum InlineNode {
    Text(Text),
    CodeSpan { children: Vec<InlineNode> },
}

impl InlineNode {
    pub fn is_blank_code_span(&self, source: &Bytes) -> bool {
        let Self::CodeSpan { children } = self else {
            panic!("not a code span");
        };
        children.iter().all(|child| {
            let Self::Text(Text { segment, .. }) = child else {
                panic!("non-text code child");
            };
            segment
                .value_shared(source)
                .bytes()
                .iter()
                .all(|b| is_space(*b))
        })
    }
}

/// Parse at the code-span trigger (`). Unclosed openers become ordinary Text.
/// Like upstream this accepts a reader directly; block parsing supplies its
/// segments. It is not a complete Markdown parse entry point.
pub fn parse_code_span(block: &mut impl ReaderCursor) -> InlineNode {
    let mut context = super::context::Context::default();
    let id = context.parse_code_span(block);
    fn project(context: &super::context::Context, id: super::arena::NodeId) -> InlineNode {
        use super::context::NodeKind;
        let node = context.arena.node(id);
        match &node.data {
            NodeKind::Text(text) => InlineNode::Text(text.clone()),
            NodeKind::CodeSpan => {
                let mut children = Vec::new();
                let mut child = node.first;
                while let Some(id) = child {
                    children.push(project(context, id));
                    child = context.arena.node(id).next;
                }
                InlineNode::CodeSpan { children }
            }
            _ => unreachable!(),
        }
    }
    project(&context, id)
}

impl super::context::Context {
    /// Original code-span parser with allocation order and abandoned nodes
    /// preserved in the shared AST arena.
    pub fn parse_code_span(&mut self, block: &mut impl ReaderCursor) -> super::arena::NodeId {
        use super::context::NodeKind;
        let (line, start_segment) = block.peek_line();
        let opener = line.map_or(0, |v| v.bytes().iter().take_while(|&&b| b == b'`').count());
        block.advance(opener as isize);
        let saved = block.position();
        let node = self.arena.insert(NodeKind::CodeSpan);
        'scan: loop {
            let (line, mut segment) = block.peek_line();
            let Some(line) = line else {
                block.set_position(saved.0, saved.1);
                return self.arena.insert(NodeKind::Text(Text::new(
                    start_segment.with_stop(start_segment.start + opener as isize),
                )));
            };
            let line = line.bytes();
            let mut i = 0;
            while i < line.len() {
                if line[i] == b'`' {
                    let old = i;
                    while i < line.len() && line[i] == b'`' {
                        i += 1;
                    }
                    let closure = i - old;
                    if closure == opener && (i >= line.len() || line[i] != b'`') {
                        segment = segment.with_stop(segment.start + (i - closure) as isize);
                        if !segment.is_empty() {
                            let child = self.arena.insert(NodeKind::Text(Text {
                                raw: true,
                                ..Text::new(segment)
                            }));
                            self.arena.append_child(node, child);
                        }
                        block.advance(i as isize);
                        break 'scan;
                    }
                }
                i += 1;
            }
            let child = self.arena.insert(NodeKind::Text(Text {
                raw: true,
                ..Text::new(segment)
            }));
            self.arena.append_child(node, child);
            block.advance_line();
        }
        let source = block.source();
        let mut blank = true;
        let mut child = self.arena.node(node).first;
        while let Some(id) = child {
            let NodeKind::Text(text) = &self.arena.node(id).data else {
                unreachable!()
            };
            if !text
                .segment
                .value_shared(&source)
                .bytes()
                .iter()
                .all(|&b| is_space(b))
            {
                blank = false;
                break;
            }
            child = self.arena.node(id).next;
        }
        if !blank {
            let first = self.arena.node(node).first.unwrap();
            let last = self.arena.node(node).last.unwrap();
            let NodeKind::Text(first_text) = &self.arena.node(first).data else {
                unreachable!()
            };
            let a = first_text.segment;
            let NodeKind::Text(last_text) = &self.arena.node(last).data else {
                unreachable!()
            };
            let b = last_text.segment;
            if !a.is_empty()
                && matches!(source.bytes()[a.start as usize], b' ' | b'\n')
                && !b.is_empty()
                && matches!(source.bytes()[b.stop as usize - 1], b' ' | b'\n')
            {
                let NodeKind::Text(text) = &mut self.arena.node_mut(first).data else {
                    unreachable!()
                };
                text.segment = text.segment.with_start(text.segment.start + 1);
                let NodeKind::Text(text) = &mut self.arena.node_mut(last).data else {
                    unreachable!()
                };
                text.segment = text.segment.with_stop(text.segment.stop - 1);
            }
        }
        node
    }
}
