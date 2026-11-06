// SPDX-License-Identifier: MPL-2.0 AND MIT
// Copyright (c) 2019 Yusuke Inuzuka
// Rust translation Copyright 2026 Nelson Chen.
// Goldmark v1.8.5 parser/link.go and ast/inline.go.
use super::{
    arena::NodeId,
    block_reader::BlockReader,
    bytes::Bytes,
    closure::FindClosureOptions,
    context::{Context, NodeKind},
    link::{normalize_reference_label, parse_link_destination, parse_link_title},
    text::{Segment, is_space},
};
use serde::{Deserialize, Serialize};
use std::{cell::RefCell, rc::Rc};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReferenceLink {
    pub typ: u8,
    pub value: Vec<u8>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Link {
    pub destination: Option<Bytes>,
    pub title: Option<Bytes>,
    pub reference: Option<Rc<RefCell<ReferenceLink>>>,
    pub position: isize,
}
impl Default for Link {
    fn default() -> Self {
        Self {
            destination: None,
            title: None,
            reference: None,
            position: -1,
        }
    }
}
impl Context {
    fn contains_link(&self, node: Option<NodeId>) -> bool {
        let mut current = node;
        while let Some(id) = current {
            let node = self.arena.node(id);
            if matches!(node.data, NodeKind::Link(_)) || self.contains_link(node.first) {
                return true;
            }
            current = node.next;
        }
        false
    }
    fn process_link_label(&mut self, parent: NodeId, link: NodeId, last: NodeId) {
        let bottom = self.pop_link_bottom();
        self.process_delimiters_with_bottom(bottom);
        let mut child = self.arena.node(last).next;
        while let Some(id) = child {
            let next = self.arena.node(id).next;
            self.arena.remove_child(parent, id);
            self.arena.append_child(link, id);
            child = next;
        }
    }
    fn failed_link_label(&mut self, last: NodeId) {
        let parent = self.arena.node(last).parent.expect("unattached link label");
        self.merge_or_replace_text_segment(parent, last, self.link_label(last).segment);
        self.pop_link_bottom();
    }
    fn inline_link(
        &mut self,
        parent: NodeId,
        last: NodeId,
        block: &mut BlockReader<'_>,
    ) -> Option<NodeId> {
        block.advance(1);
        block.skip_spaces();
        let mut data = Link::default();
        if block.peek() == b')' {
            block.advance(1);
        } else {
            let (destination, ok) = parse_link_destination(block);
            if !ok {
                return None;
            }
            data.destination = destination;
            block.skip_spaces();
            if block.peek() == b')' {
                block.advance(1);
            } else {
                let (title, ok) = parse_link_title(block);
                if !ok {
                    return None;
                }
                data.title = title.map(|v| Bytes::new(&v));
                block.skip_spaces();
                if block.peek() != b')' {
                    return None;
                }
                block.advance(1);
            }
        }
        let link = self.arena.insert(NodeKind::Link(Link::default()));
        self.process_link_label(parent, link, last);
        self.arena.node_mut(link).data = NodeKind::Link(data);
        Some(link)
    }
    fn reference_link(
        &mut self,
        parent: NodeId,
        last: NodeId,
        block: &mut BlockReader<'_>,
    ) -> (Option<NodeId>, bool) {
        let original = block.position().1;
        block.advance(1);
        let Some(segments) = block.find_closure(
            b'[',
            b']',
            FindClosureOptions {
                newline: true,
                advance: true,
                ..Default::default()
            },
        ) else {
            return (None, false);
        };
        let mut value: Vec<u8> = segments.into_iter().flat_map(|s| block.value(s)).collect();
        let mut typ = 1;
        if value.iter().all(|&b| is_space(b)) {
            value = block.value(Segment::new(
                self.link_label(last).segment.stop,
                original.start - 1,
            ));
            typ = 2;
        }
        if value.len() > 999 {
            return (None, true);
        }
        let Some(reference) = self.references.get(&normalize_reference_label(&value)) else {
            return (None, true);
        };
        let link = self.arena.insert(NodeKind::Link(Link::default()));
        self.process_link_label(parent, link, last);
        let r = reference.borrow();
        self.arena.node_mut(link).data = NodeKind::Link(Link {
            destination: r.destination.clone(),
            title: r.title.clone(),
            reference: Some(Rc::new(RefCell::new(ReferenceLink { typ, value }))),
            position: -1,
        });
        (Some(link), true)
    }
    /// Goldmark linkParser.Parse. The inline dispatcher must restore the saved
    /// reader position on None and attach returned nodes to parent.
    pub fn parse_link(&mut self, parent: NodeId, block: &mut BlockReader<'_>) -> Option<NodeId> {
        let (line, segment) = block.peek_line();
        let byte = line.expect("link parser reached EOF").bytes()[0];
        if byte == b'!' || byte == b'[' {
            return self.parse_link_open(block);
        }
        let list = self.link_labels?;
        let Some(last) = self.link_label(list).last else {
            self.pop_link_bottom();
            return None;
        };
        block.advance(1);
        self.remove_link_label(last);
        if self.link_label_length(Some(list)) > 998
            || (!self.link_label(last).is_image && self.contains_link(Some(last)))
        {
            self.failed_link_label(last);
            return None;
        }
        let saved = block.position();
        let mut link = None;
        match block.peek() {
            b'(' => link = self.inline_link(parent, last, block),
            b'[' => {
                let (value, has_value) = self.reference_link(parent, last, block);
                if value.is_none() && has_value {
                    self.failed_link_label(last);
                    return None;
                }
                link = value;
            }
            _ => {}
        }
        let link = if let Some(link) = link {
            link
        } else {
            block.set_position(saved.0, saved.1);
            let value = block.value(Segment::new(
                self.link_label(last).segment.stop,
                segment.start,
            ));
            if value.len() > 999 {
                self.failed_link_label(last);
                return None;
            }
            let Some(reference) = self.references.get(&normalize_reference_label(&value)) else {
                self.failed_link_label(last);
                return None;
            };
            let link = self.arena.insert(NodeKind::Link(Link::default()));
            self.process_link_label(parent, link, last);
            let r = reference.borrow();
            self.arena.node_mut(link).data = NodeKind::Link(Link {
                destination: r.destination.clone(),
                title: r.title.clone(),
                reference: Some(Rc::new(RefCell::new(ReferenceLink { typ: 3, value }))),
                position: -1,
            });
            link
        };
        let label = self.link_label(last).clone();
        let label_parent = self.arena.node(last).parent.expect("unattached link label");
        self.arena.remove_child(label_parent, last);
        let output = if label.is_image {
            let NodeKind::Link(data) = self.arena.node(link).data.clone() else {
                unreachable!()
            };
            let image = self.arena.insert(NodeKind::Image(data));
            while let Some(child) = self.arena.node(link).first {
                self.arena.remove_child(link, child);
                self.arena.append_child(image, child);
            }
            image
        } else {
            link
        };
        match &mut self.arena.node_mut(output).data {
            NodeKind::Link(data) | NodeKind::Image(data) => data.position = label.segment.start,
            _ => unreachable!(),
        }
        Some(output)
    }
}
