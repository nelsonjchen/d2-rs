// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Edge AST records and traversal from pinned d2ast/d2ast.go.
use super::*;
use serde::Serialize;
use std::borrow::Cow;

#[derive(Clone, Debug, Default, Serialize)]
pub struct Edge {
    pub range: Range,
    pub src: Option<Shared<KeyPath>>,
    pub src_arrow: String,
    pub dst: Option<Shared<KeyPath>>,
    pub dst_arrow: String,
}
impl Node for Edge {
    fn node_type(&self) -> Cow<'_, str> {
        "edge".into()
    }
    fn range(&self) -> Range {
        self.range.clone()
    }
    fn children(&self) -> Vec<Child> {
        self.src
            .iter()
            .chain(self.dst.iter())
            .cloned()
            .map(Child::KeyPath)
            .collect()
    }
}
impl Edge {
    /// Compare endpoint scalar paths and arrows; source locations are ignored.
    /// Missing endpoints are invalid for equality, as in upstream.
    pub fn equals(&self, other: &Self) -> bool {
        self.src
            .as_ref()
            .expect("nil edge source")
            .read()
            .equals(&other.src.as_ref().expect("nil edge source").read())
            && self.src_arrow == other.src_arrow
            && self
                .dst
                .as_ref()
                .expect("nil edge destination")
                .read()
                .equals(&other.dst.as_ref().expect("nil edge destination").read())
            && self.dst_arrow == other.dst_arrow
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct EdgeIndex {
    pub range: Range,
    pub int: Option<Shared<i64>>,
    pub glob: bool,
}
impl Node for EdgeIndex {
    fn node_type(&self) -> Cow<'_, str> {
        "edge index".into()
    }
    fn range(&self) -> Range {
        self.range.clone()
    }
    fn children(&self) -> Vec<Child> {
        Vec::new()
    }
}
impl EdgeIndex {
    /// Upstream deliberately compares integer pointer identity, not its value.
    pub fn equals(&self, other: &Self) -> bool {
        let same_pointer = match (&self.int, &other.int) {
            (None, None) => true,
            (Some(a), Some(b)) => a.ptr_eq(b),
            _ => false,
        };
        same_pointer && self.glob == other.glob
    }
}

/// Preorder traversal. Returning false prunes only the current subtree.
pub fn walk(node: Option<&dyn Node>, visit: &mut impl FnMut(&dyn Node) -> bool) {
    let Some(node) = node else { return };
    if !visit(node) {
        return;
    }
    for child in node.children() {
        walk(Some(&child), visit);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn edge_index_compares_pointer_identity_and_glob() {
        let first = EdgeIndex {
            int: Some(Shared::new(3)),
            ..Default::default()
        };
        let mut copied = first.clone();
        let separate = EdgeIndex {
            int: Some(Shared::new(3)),
            ..Default::default()
        };
        assert!(first.equals(&copied));
        assert!(!first.equals(&separate));
        copied.glob = true;
        assert!(!first.equals(&copied));
        assert!(EdgeIndex::default().equals(&EdgeIndex::default()));
    }
    #[test]
    fn walk_prunes_source_without_skipping_destination() {
        let edge = Edge {
            src: Some(Shared::new(KeyPath::from_strings(["a"]))),
            dst: Some(Shared::new(KeyPath::from_strings(["b"]))),
            ..Default::default()
        };
        let mut types = Vec::new();
        let mut paths = 0;
        walk(Some(&edge), &mut |node| {
            types.push(node.node_type().into_owned());
            if node.node_type() == "key path" {
                paths += 1;
                return paths != 1;
            }
            true
        });
        assert_eq!(types, ["edge", "key path", "key path", "unquoted string"]);
    }
}
