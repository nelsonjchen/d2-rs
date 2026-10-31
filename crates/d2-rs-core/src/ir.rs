// SPDX-License-Identifier: MPL-2.0
//! Resolved-language IR, separate from AST and measured graph types.
//! Tree ownership and edge identifiers; compilation is still being translated.
mod deletion;
mod edge_resolution;
mod fold;
pub use edge_resolution::{InvalidUnderscore, ResolvedEdge};
mod import_cache;
mod import_contexts;
pub use import_contexts::{GlobContext, GlobId, ImportTemplate};
mod maps;
pub use maps::fold_string;
mod merge;
pub use import_cache::{clone_ast, clone_ast_with_context};
mod pattern;
mod projection;
pub use pattern::{go_lowercase, match_pattern};
mod references;
mod tree;
use crate::ast::{self, ByteString, Child, ScalarBox, Shared};
pub use references::*;
use serde::Serialize;
pub use tree::*;

/// Source-order endpoint components retain shared AST node identities.
#[derive(Debug, Default, Serialize)]
pub struct EdgeId {
    pub src_path: Option<Vec<Option<Child>>>,
    pub src_arrow: bool,
    pub dst_path: Option<Vec<Option<Child>>>,
    pub dst_arrow: bool,
    /// Missing indices match any index; the glob flag does not affect Match.
    pub index: Option<Shared<i64>>,
    pub glob: bool,
}
impl Clone for EdgeId {
    /// Copy path slots and the index allocation, retaining endpoint AST nodes.
    fn clone(&self) -> Self {
        Self {
            src_path: self.src_path.as_ref().filter(|p| !p.is_empty()).cloned(),
            src_arrow: self.src_arrow,
            dst_path: self.dst_path.as_ref().filter(|p| !p.is_empty()).cloned(),
            dst_arrow: self.dst_arrow,
            index: self.index.as_ref().map(|i| Shared::new(*i.read())),
            glob: self.glob,
        }
    }
}
impl EdgeId {
    pub fn matches(&self, other: &Self) -> bool {
        if let (Some(a), Some(b)) = (&self.index, &other.index)
            && *a.read() != *b.read()
        {
            return false;
        }
        (path_equal(&self.src_path, &other.src_path)
            && self.src_arrow == other.src_arrow
            && path_equal(&self.dst_path, &other.dst_path)
            && self.dst_arrow == other.dst_arrow)
            || (path_equal(&self.src_path, &other.dst_path)
                && self.src_arrow == other.dst_arrow
                && path_equal(&self.dst_path, &other.src_path)
                && self.dst_arrow == other.src_arrow)
    }
}
/// Translate every AST edge in a chain; arrow decorations other than `<` and
/// `>` do not set endpoint direction flags. Indices initially alias the AST.
pub fn new_edge_ids(key: &ast::Key) -> Option<Vec<EdgeId>> {
    let edges = key.edges.as_ref().filter(|edges| !edges.is_empty())?;
    Some(
        edges
            .iter()
            .map(|slot| {
                let slot = slot.read();
                let edge = slot.as_ref().expect("nil AST edge").read();
                let index = key.edge_index.as_ref().map(|i| i.read());
                EdgeId {
                    src_path: edge.src.as_ref().expect("nil edge source").read().ida(),
                    src_arrow: edge.src_arrow == "<",
                    dst_path: edge
                        .dst
                        .as_ref()
                        .expect("nil edge destination")
                        .read()
                        .ida(),
                    dst_arrow: edge.dst_arrow == ">",
                    index: index.as_ref().and_then(|i| i.int.clone()),
                    glob: index.as_ref().is_some_and(|i| i.glob),
                }
            })
            .collect(),
    )
}
fn path_equal(a: &Option<Vec<Option<Child>>>, b: &Option<Vec<Option<Child>>>) -> bool {
    let a = a.as_deref().unwrap_or_default();
    let b = b.as_deref().unwrap_or_default();
    a.len() == b.len()
        && a.iter().zip(b).all(|(a, b)| {
            let a = ByteString::from(ScalarBox::from_child(a.clone()).scalar_bytes());
            let b = ByteString::from(ScalarBox::from_child(b.clone()).scalar_bytes());
            a.to_text()
                .chars()
                .map(fold::simple_fold)
                .eq(b.to_text().chars().map(fold::simple_fold))
        })
}

/// Case-insensitive string equality using the pinned Go SimpleFold table.
pub fn equal_fold(a: &[u8], b: &[u8]) -> bool {
    let (mut a, mut b) = (a, b);
    while let (Some(&left), Some(&right)) = (a.first(), b.first()) {
        if left.is_ascii() && right.is_ascii() {
            if !left.eq_ignore_ascii_case(&right) {
                return false;
            }
            a = &a[1..];
            b = &b[1..];
        } else {
            // Go consumes one byte per invalid UTF-8 rune. Do not use Rust's
            // lossy decoder, which coalesces malformed byte sequences.
            let (left, left_width) = crate::go_quote::decode_rune(a);
            let (right, right_width) = crate::go_quote::decode_rune(b);
            if left != right && fold::simple_fold(left) != fold::simple_fold(right) {
                return false;
            }
            a = &a[left_width..];
            b = &b[right_width..];
        }
    }
    a.is_empty() && b.is_empty()
}

#[cfg(test)]
mod fold_tests {
    use super::*;
    use crate::ast::StringNode;

    #[test]
    fn streaming_equal_fold_preserves_pinned_unicode_and_invalid_bytes() {
        let reference_key = |bytes: &[u8]| {
            ByteString::from(bytes)
                .to_text()
                .chars()
                .map(fold::simple_fold)
                .collect::<String>()
        };
        // Compare every scalar to the existing pinned canonicalization,
        // including equivalences crossing the ASCII/Unicode fast paths.
        for rune in (0..=0x10ffff).filter_map(char::from_u32) {
            let mut left = [0; 4];
            let mut right = [0; 4];
            let a = rune.encode_utf8(&mut left).as_bytes();
            let b = fold::simple_fold(rune).encode_utf8(&mut right).as_bytes();
            assert!(equal_fold(a, b), "{rune:?}");
            assert_eq!(fold_string(a), reference_key(a), "{rune:?}");
        }
        let mut values: Vec<Vec<u8>> = (0..=255).map(|b| vec![b]).collect();
        values.extend([
            vec![0xe2, 0x82],
            vec![0xef, 0xbf, 0xbd],
            vec![0xff, 0xfe],
            "KKk".as_bytes().to_vec(),
            "kKK".as_bytes().to_vec(),
            "Sſs".as_bytes().to_vec(),
            "sSſ".as_bytes().to_vec(),
            "Σσς".as_bytes().to_vec(),
            "σςΣ".as_bytes().to_vec(),
            vec![],
        ]);
        for a in &values {
            for b in &values {
                assert_eq!(
                    equal_fold(a, b),
                    reference_key(a) == reference_key(b),
                    "{a:?} {b:?}"
                );
            }
        }
    }

    #[test]
    fn borrowed_property_lookup_preserves_live_order_and_reserved_quoting() {
        let mut arena = Arena::default();
        let root = arena.new_root();
        for (bytes, quoted, child) in [
            ("label", true, false),
            ("LABEL", false, false),
            ("K", false, false),
            ("k", false, true),
            ("ſhape", true, false),
            ("SHAPE", false, false),
        ] {
            let field = arena.insert(
                Some(root),
                Node::Field(Field {
                    name: ast::raw_string_box(bytes, !quoted),
                    ..Default::default()
                }),
            );
            arena.append_field(root, field);
            if child {
                let map = arena.insert(Some(field), Node::Map(Map::default()));
                let Node::Field(f) = arena.node_mut(field) else {
                    unreachable!()
                };
                f.composite = Some(map);
                let nested = arena.insert(
                    Some(map),
                    Node::Field(Field {
                        name: ast::raw_string_box("stroke", true),
                        ..Default::default()
                    }),
                );
                arena.append_field(map, nested);
            }
        }
        let paths: &[&[&str]] = &[
            &[],
            &["label"],
            &["LABEL"],
            &["shape"],
            &["ſhape"],
            &["K"],
            &["k", "stroke"],
            &["absent"],
            &["_", "label"],
        ];
        for phase in 0..3 {
            for path in paths {
                let ast_path = path
                    .iter()
                    .map(|s| ast::raw_string_box(*s, true))
                    .collect::<Vec<_>>();
                assert_eq!(
                    arena.get_field_unquoted(root, path),
                    arena.get_field(root, &ast_path)
                );
            }
            // Public Fields mutations and shared AST changes remain visible.
            if phase == 0 {
                let Node::Map(map) = arena.node_mut(root) else {
                    unreachable!()
                };
                map.fields.as_mut().unwrap().reverse();
            } else if phase == 1 {
                let Node::Map(map) = arena.node(root) else {
                    unreachable!()
                };
                let Node::Field(field) = arena.node(map.fields.as_ref().unwrap()[0]) else {
                    unreachable!()
                };
                field
                    .name
                    .unquoted_string
                    .as_ref()
                    .unwrap()
                    .write()
                    .set_string("other".into());
            }
        }
    }
}
