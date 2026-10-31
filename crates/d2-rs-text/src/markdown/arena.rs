// SPDX-License-Identifier: MPL-2.0 AND MIT
// Copyright (c) 2019 Yusuke Inuzuka
// Rust translation Copyright 2026 Nelson Chen.
// Goldmark v1.8.5 ast/ast.go BaseNode child and sibling mutation methods.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NodeId(pub usize);

#[derive(Clone, Debug, Serialize)]
pub struct Node<T> {
    pub data: T,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<isize>,
    pub parent: Option<NodeId>,
    pub previous: Option<NodeId>,
    pub next: Option<NodeId>,
    pub first: Option<NodeId>,
    pub last: Option<NodeId>,
    pub child_count: isize,
}

/// Stable identities survive detachment and movement between parents.
/// Payloads contain node-specific state; delimiter links are distinct from
/// these AST links. Attribute and owner-document APIs remain separate work.
#[derive(Clone, Debug)]
pub struct Arena<T> {
    nodes: Vec<Node<T>>,
}

impl<T> Default for Arena<T> {
    fn default() -> Self {
        Self { nodes: Vec::new() }
    }
}

impl<T> Arena<T> {
    pub fn insert(&mut self, data: T) -> NodeId {
        let id = NodeId(self.nodes.len());
        self.nodes.push(Node {
            data,
            position: None,
            parent: None,
            previous: None,
            next: None,
            first: None,
            last: None,
            child_count: 0,
        });
        id
    }
    pub fn nodes(&self) -> &[Node<T>] {
        &self.nodes
    }
    pub fn node(&self, id: NodeId) -> &Node<T> {
        &self.nodes[id.0]
    }
    pub fn node_mut(&mut self, id: NodeId) -> &mut Node<T> {
        &mut self.nodes[id.0]
    }
    pub fn ensure_isolated(&mut self, child: NodeId) {
        if let Some(parent) = self.node(child).parent {
            self.remove_child(parent, child);
        }
    }
    pub fn remove_child(&mut self, parent: NodeId, child: NodeId) {
        if self.node(child).parent != Some(parent) {
            return;
        }
        self.node_mut(parent).child_count -= 1;
        let previous = self.node(child).previous;
        let next = self.node(child).next;
        if let Some(previous) = previous {
            self.node_mut(previous).next = next;
        } else {
            self.node_mut(parent).first = next;
        }
        if let Some(next) = next {
            self.node_mut(next).previous = previous;
        } else {
            self.node_mut(parent).last = previous;
        }
        let node = self.node_mut(child);
        node.parent = None;
        node.previous = None;
        node.next = None;
    }
    pub fn remove_children(&mut self, parent: NodeId) {
        let mut child = self.node(parent).first;
        while let Some(id) = child {
            let node = self.node_mut(id);
            node.parent = None;
            node.previous = None;
            child = node.next;
            node.next = None;
        }
        let node = self.node_mut(parent);
        node.first = None;
        node.last = None;
        node.child_count = 0;
    }
    pub fn append_child(&mut self, parent: NodeId, child: NodeId) {
        self.ensure_isolated(child);
        if self.node(parent).first.is_none() {
            self.node_mut(parent).first = Some(child);
            self.node_mut(child).next = None;
            self.node_mut(child).previous = None;
        } else {
            let last = self.node(parent).last.expect("missing last child");
            self.node_mut(last).next = Some(child);
            self.node_mut(child).previous = Some(last);
        }
        self.node_mut(child).parent = Some(parent);
        self.node_mut(parent).last = Some(child);
        self.node_mut(parent).child_count += 1;
    }
    pub fn insert_before(&mut self, parent: NodeId, target: Option<NodeId>, child: NodeId) {
        // This increment precedes AppendChild in the nil-target branch in Go.
        // Preserve the observable count, even when it differs from link count.
        self.node_mut(parent).child_count += 1;
        let Some(target) = target else {
            self.append_child(parent, child);
            return;
        };
        self.ensure_isolated(child);
        if self.node(target).parent == Some(parent) {
            let previous = self.node(target).previous;
            if let Some(previous) = previous {
                self.node_mut(previous).next = Some(child);
                self.node_mut(child).previous = Some(previous);
            } else {
                self.node_mut(parent).first = Some(child);
                self.node_mut(child).previous = None;
            }
            self.node_mut(child).next = Some(target);
            self.node_mut(target).previous = Some(child);
            self.node_mut(child).parent = Some(parent);
        }
    }
    pub fn insert_after(&mut self, parent: NodeId, target: NodeId, child: NodeId) {
        self.insert_before(parent, self.node(target).next, child);
    }
    pub fn replace_child(&mut self, parent: NodeId, target: NodeId, child: NodeId) {
        self.insert_before(parent, Some(target), child);
        self.remove_child(parent, target);
    }
}
