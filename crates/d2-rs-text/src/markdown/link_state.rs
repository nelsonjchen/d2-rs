// SPDX-License-Identifier: MPL-2.0 AND MIT
// Copyright (c) 2019 Yusuke Inuzuka
// Rust translation Copyright 2026 Nelson Chen.
// Goldmark v1.8.5 parser/link.go linkLabelState and linked-list operations.
use super::{
    arena::NodeId,
    context::{Context, NodeKind},
    text::Segment,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LinkLabelState {
    pub segment: Segment,
    pub is_image: bool,
    pub previous: Option<NodeId>,
    pub next: Option<NodeId>,
    pub first: Option<NodeId>,
    pub last: Option<NodeId>,
}

impl Context {
    pub fn new_link_label(&mut self, segment: Segment, is_image: bool) -> NodeId {
        self.arena.insert(NodeKind::LinkLabel(LinkLabelState {
            segment,
            is_image,
            previous: None,
            next: None,
            first: None,
            last: None,
        }))
    }
    pub fn link_label(&self, id: NodeId) -> &LinkLabelState {
        let NodeKind::LinkLabel(state) = &self.arena.node(id).data else {
            panic!("not a link label")
        };
        state
    }
    fn link_label_mut(&mut self, id: NodeId) -> &mut LinkLabelState {
        let NodeKind::LinkLabel(state) = &mut self.arena.node_mut(id).data else {
            panic!("not a link label")
        };
        state
    }
    pub fn push_link_label(&mut self, id: NodeId) {
        if let Some(list) = self.link_labels {
            let last = self.link_label(list).last.expect("missing label tail");
            self.link_label_mut(list).last = Some(id);
            self.link_label_mut(last).next = Some(id);
            self.link_label_mut(id).previous = Some(last);
        } else {
            self.link_label_mut(id).first = Some(id);
            self.link_label_mut(id).last = Some(id);
            self.link_labels = Some(id);
        }
    }
    pub fn remove_link_label(&mut self, id: NodeId) {
        let Some(mut list) = self.link_labels else {
            return;
        };
        let state = self.link_label(id).clone();
        let mut list_present = true;
        if let Some(previous) = state.previous {
            self.link_label_mut(previous).next = state.next;
            if let Some(next) = state.next {
                self.link_label_mut(next).previous = Some(previous);
            }
        } else if let Some(next) = state.next {
            list = next;
            // Source deliberately retains the removed node as First.
            self.link_label_mut(list).first = Some(id);
            self.link_label_mut(list).last = state.last;
            self.link_label_mut(list).previous = None;
            self.link_labels = Some(list);
        } else {
            self.link_labels = None;
            list_present = false;
        }
        if list_present && state.next.is_none() {
            self.link_label_mut(list).last = state.previous;
        }
        let state = self.link_label_mut(id);
        state.next = None;
        state.previous = None;
        state.first = None;
        state.last = None;
    }
    pub fn link_label_length(&self, id: Option<NodeId>) -> isize {
        let Some(id) = id else { return 0 };
        let state = self.link_label(id);
        let (Some(first), Some(last)) = (state.first, state.last) else {
            return 0;
        };
        self.link_label(last).segment.stop - self.link_label(first).segment.start
    }
}

impl Context {
    /// Opening branches of Goldmark linkParser.Parse. Closing brackets are
    /// handled separately by the link resolver. The caller attaches the node.
    pub fn parse_link_open(
        &mut self,
        reader: &mut impl super::reader_api::ReaderCursor,
    ) -> Option<NodeId> {
        let (line, segment) = reader.peek_line();
        let line = line.expect("link parser reached EOF");
        let bytes = line.bytes();
        let is_image = bytes[0] == b'!';
        let pos = if is_image {
            if bytes.len() <= 1 || bytes[1] != b'[' {
                return None;
            }
            reader.advance(1);
            segment.start + 1
        } else if bytes[0] == b'[' {
            segment.start
        } else {
            return None;
        };
        self.push_link_bottom();
        let start = if is_image { pos - 1 } else { pos };
        let id = self.new_link_label(Segment::new(start, pos + 1), is_image);
        self.push_link_label(id);
        reader.advance(1);
        Some(id)
    }

    /// Replace all unmatched label states with fresh text nodes, preserving the
    /// source's lack of text merging and each label's actual AST parent.
    pub fn close_link_block(&mut self) {
        self.clear_link_bottoms();
        let mut current = self.link_labels;
        while let Some(id) = current {
            let state = self.link_label(id).clone();
            current = state.next;
            self.remove_link_label(id);
            let parent = self.arena.node(id).parent.expect("unattached link label");
            let text = self
                .arena
                .insert(NodeKind::Text(super::inline::Text::new(state.segment)));
            self.arena.replace_child(parent, id, text);
        }
    }
}
