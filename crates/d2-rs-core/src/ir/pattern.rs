// SPDX-License-Identifier: MPL-2.0
//! Pattern matching and recursive glob traversal from pinned d2ir/pattern.go.
use super::{Arena, Node, NodeId, fold};
use crate::ast::{
    ByteString, is_double_glob, is_triple_glob,
    keywords::{BOARD_KEYWORDS, is_reserved_keyword},
};
fn reserved(bytes: &[u8]) -> bool {
    std::str::from_utf8(bytes).is_ok_and(is_reserved_keyword)
}
/// Unicode lowercase using the pinned Go tables and invalid-byte decoding.
pub fn go_lowercase(bytes: &[u8]) -> Vec<u8> {
    // ASCII is valid UTF-8 and unicode.ToLower maps only A-Z within it.
    if bytes.is_ascii() {
        return bytes.to_ascii_lowercase();
    }
    ByteString::from(bytes)
        .to_text()
        .chars()
        .map(fold::to_lower)
        .collect::<String>()
        .into_bytes()
}
/// Preserve Go's raw-byte consumption after case conversion, including its
/// bounds panic when case-equivalent strings have incompatible byte lengths.
pub fn match_pattern(mut text: &[u8], pattern: &[ByteString]) -> bool {
    if pattern.is_empty() {
        return true;
    }
    if reserved(text) {
        return false;
    }
    let mut i = 0;
    while i < pattern.len() {
        let part = pattern[i].as_bytes();
        if part == b"*" {
            if i == pattern.len() - 1 {
                return true;
            }
            let next = go_lowercase(pattern[i + 1].as_bytes());
            let folded = go_lowercase(text);
            if i + 1 == pattern.len() - 1 {
                return folded.ends_with(&next);
            }
            let index = if next.is_empty() {
                Some(0)
            } else {
                folded.windows(next.len()).position(|s| s == next)
            };
            let Some(index) = index else {
                return false;
            };
            text = &text[index + pattern[i + 1].as_bytes().len()..];
            i += 1;
        } else {
            if !go_lowercase(text).starts_with(&go_lowercase(part)) {
                return false;
            }
            text = &text[part.len()..];
        }
        i += 1;
    }
    text.is_empty()
}
impl Arena {
    pub fn multi_glob(&self, map: NodeId, pattern: &[ByteString]) -> (Option<Vec<NodeId>>, bool) {
        self.multi_glob_with_visit(map, pattern, &mut |_| true)
    }
    /// Visit every candidate before filtering, stopping traversal on refusal.
    pub fn multi_glob_with_visit(
        &self,
        map: NodeId,
        pattern: &[ByteString],
        visit: &mut impl FnMut(NodeId) -> bool,
    ) -> (Option<Vec<NodeId>>, bool) {
        let triple = is_triple_glob(pattern);
        if !triple && !is_double_glob(pattern) {
            return (None, false);
        }
        let mut fields = Vec::new();
        self.recursive_glob(map, triple, &mut fields, visit);
        ((!fields.is_empty()).then_some(fields), true)
    }
    pub fn path_to_field(&self, map: NodeId, mut target: Option<NodeId>) -> Option<Vec<NodeId>> {
        let mut reverse = Vec::new();
        while let Some(current) = target {
            assert!(
                matches!(self.node(current), Node::Field(_)),
                "expected target field"
            );
            let parent = self.parent_map(current)?;
            reverse.push(current);
            if parent == map {
                reverse.reverse();
                return Some(reverse);
            }
            target = self.parent_field(parent);
        }
        None
    }
    pub fn direct_child_toward(&self, map: NodeId, target: Option<NodeId>) -> Option<NodeId> {
        self.path_to_field(map, target)
            .and_then(|p| p.first().copied())
    }
    /// Lazy replay revisits the complete top-level branch, retaining traversal
    /// and reference order rather than visiting only the changed target path.
    pub fn multi_glob_matches_toward(
        &self,
        map: NodeId,
        target: Option<NodeId>,
        pattern: &[ByteString],
    ) -> Option<Vec<NodeId>> {
        self.multi_glob_matches_toward_with_visit(map, target, pattern, &mut |_| true)
    }
    pub fn multi_glob_matches_toward_with_visit(
        &self,
        map: NodeId,
        target: Option<NodeId>,
        pattern: &[ByteString],
        visit: &mut impl FnMut(NodeId) -> bool,
    ) -> Option<Vec<NodeId>> {
        let field = self.direct_child_toward(map, target)?;
        let triple = is_triple_glob(pattern);
        if !triple && !is_double_glob(pattern) {
            return None;
        }
        let mut fields = Vec::new();
        let Node::Field(f) = self.node(field) else {
            panic!("expected glob field")
        };
        // Branch helpers skip unnamed placeholders before the visitor; full
        // traversal visits them before the nil-name guard (pattern.go).
        if f.name.unbox().is_some() && visit(field) {
            self.glob_field(field, triple, &mut fields, visit);
        }
        (!fields.is_empty()).then_some(fields)
    }
    fn recursive_glob(
        &self,
        map: NodeId,
        triple: bool,
        out: &mut Vec<NodeId>,
        visit: &mut impl FnMut(NodeId) -> bool,
    ) -> bool {
        let Node::Map(m) = self.node(map) else {
            panic!("expected glob map")
        };
        for field in m.fields.iter().flatten() {
            if !visit(*field) || !self.glob_field(*field, triple, out, visit) {
                return false;
            }
        }
        true
    }
    fn glob_field(
        &self,
        field: NodeId,
        triple: bool,
        out: &mut Vec<NodeId>,
        visit: &mut impl FnMut(NodeId) -> bool,
    ) -> bool {
        let Node::Field(f) = self.node(field) else {
            panic!("expected glob field")
        };
        if f.name.unbox().is_none() {
            return true;
        }
        let name = f.name.scalar_bytes();
        let board = BOARD_KEYWORDS.iter().any(|b| b.as_bytes() == name);
        if reserved(&name) && f.name.unquoted_string.is_some() {
            if (triple && !board) || (!triple && (board || name == b"classes" || name == b"vars")) {
                return true;
            }
            if let Some(map) = self.map(field) {
                return self.recursive_glob(map, triple, out, visit);
            }
            return true;
        }
        if self.board_kind(field).is_empty() {
            out.push(field);
        }
        if let Some(map) = self.map(field) {
            return self.recursive_glob(map, triple, out, visit);
        }
        true
    }
}
