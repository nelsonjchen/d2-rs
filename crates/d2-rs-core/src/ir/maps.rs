// SPDX-License-Identifier: MPL-2.0
//! Source-ordered IR mutations and derived lookup indexes from d2ir/d2ir.go.
use super::{Arena, EdgeId, Node, NodeId, fold};
use crate::ast::{ByteString, ScalarBox, StringBox, keywords::is_reserved_keyword};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct FieldKey {
    folded: String,
    unquoted_reserved: bool,
}
#[derive(Debug)]
pub(super) struct FieldIndex {
    size: usize,
    entries: HashMap<FieldKey, NodeId>,
}
#[derive(Debug)]
pub(super) struct EdgeIndex {
    size: usize,
    entries: HashMap<String, Vec<NodeId>>,
}
/// Canonical Unicode simple-fold key, matching Go strings.EqualFold.
pub fn fold_string(mut bytes: &[u8]) -> String {
    let mut folded = String::with_capacity(bytes.len());
    while let Some(&first) = bytes.first() {
        if first.is_ascii() {
            folded.push(first.to_ascii_uppercase() as char);
            bytes = &bytes[1..];
        } else {
            let (rune, width) = crate::go_quote::decode_rune(bytes);
            folded.push(fold::simple_fold(rune));
            bytes = &bytes[width..];
        }
    }
    folded
}
fn reserved_bytes(bytes: &[u8]) -> bool {
    if bytes.is_ascii() {
        // Every reserved keyword is ASCII and at most 16 bytes. Keep the
        // Unicode ToLower fallback: EqualFold would wrongly reserve long-s.
        let mut lower = [0; 32];
        if bytes.len() > lower.len() {
            return false;
        }
        for (out, byte) in lower.iter_mut().zip(bytes) {
            *out = byte.to_ascii_lowercase();
        }
        return is_reserved_keyword(std::str::from_utf8(&lower[..bytes.len()]).unwrap());
    }
    is_reserved_keyword(
        &ByteString::from(bytes)
            .to_text()
            .chars()
            .map(fold::to_lower)
            .collect::<String>(),
    )
}
fn field_key(name: &StringBox) -> FieldKey {
    name.with_scalar_bytes(|bytes| FieldKey {
        folded: fold_string(bytes),
        unquoted_reserved: reserved_bytes(bytes) && name.unquoted_string.is_some(),
    })
}
fn edge_key(eid: &EdgeId) -> String {
    let path = |path: &Option<Vec<Option<crate::ast::Child>>>, arrow: bool| {
        let mut key = format!("{}:", path.as_ref().map_or(0, Vec::len));
        for part in path.iter().flatten() {
            assert!(part.is_some(), "nil edge component");
            let value = fold_string(&ScalarBox::from_child(part.clone()).scalar_bytes());
            key.push_str(&format!("{}:{value}", value.len()));
        }
        key.push(if arrow { '*' } else { '-' });
        key
    };
    let mut a = path(&eid.src_path, eid.src_arrow);
    let mut b = path(&eid.dst_path, eid.dst_arrow);
    if a > b {
        std::mem::swap(&mut a, &mut b)
    }
    format!("{}:{a}{}:{b}", a.len(), b.len())
}
impl Arena {
    fn map_fields(&self, map: NodeId) -> &[NodeId] {
        let Node::Map(m) = self.node(map) else {
            panic!("expected IR map")
        };
        m.fields.as_deref().unwrap_or_default()
    }
    fn map_edges(&self, map: NodeId) -> &[NodeId] {
        let Node::Map(m) = self.node(map) else {
            panic!("expected IR map")
        };
        m.edges.as_deref().unwrap_or_default()
    }
    fn node_field_key(&self, id: NodeId) -> Option<FieldKey> {
        let Node::Field(f) = self.node(id) else {
            panic!("expected IR field")
        };
        f.name.unbox().map(|_| field_key(&f.name))
    }
    fn node_edge_key(&self, id: NodeId) -> Option<String> {
        let Node::Edge(e) = self.node(id) else {
            panic!("expected IR edge")
        };
        e.id.as_ref().map(|id| edge_key(&id.read()))
    }
    /// Advance this map and every ancestor map, preserving unsigned wraparound.
    pub fn mark_structure_changed(&mut self, mut map: NodeId) {
        loop {
            let Node::Map(m) = self.node_mut(map) else {
                panic!("expected IR map")
            };
            m.structure_version = m.structure_version.wrapping_add(1);
            let Some(parent) = self.parent_map(map) else {
                break;
            };
            map = parent;
        }
    }
    pub fn append_field(&mut self, map: NodeId, field: NodeId) {
        let old_len = self.map_fields(map).len();
        let key = self.node_field_key(field);
        let Node::Map(m) = self.node_mut(map) else {
            unreachable!()
        };
        m.fields.get_or_insert_default().push(field);
        self.extend_field_index(map, old_len, field, key);
        self.mark_structure_changed(map);
    }
    pub fn insert_field(&mut self, map: NodeId, index: usize, field: NodeId) {
        let old_len = self.map_fields(map).len();
        let key = self.node_field_key(field);
        let Node::Map(m) = self.node_mut(map) else {
            unreachable!()
        };
        m.fields.get_or_insert_default().insert(index, field);
        self.extend_field_index(map, old_len, field, key);
        self.mark_structure_changed(map);
    }
    fn extend_field_index(
        &mut self,
        map: NodeId,
        old_len: usize,
        field: NodeId,
        key: Option<FieldKey>,
    ) {
        if let Some(cache) = self.field_indexes.get_mut(&map)
            && cache.size == old_len
            && let Some(key) = key
        {
            // Upstream keeps an existing cached winner even when a duplicate
            // field is inserted before it. Rebuilding may choose another field.
            cache.entries.entry(key).or_insert(field);
            cache.size += 1;
        } else {
            self.field_indexes.remove(&map);
        }
    }
    pub fn remove_field(&mut self, map: NodeId, index: usize) -> NodeId {
        let Node::Map(m) = self.node_mut(map) else {
            panic!("expected IR map")
        };
        let field = m.fields.as_mut().expect("nil map fields").remove(index);
        self.field_indexes.remove(&map);
        self.mark_structure_changed(map);
        field
    }
    pub fn append_edge(&mut self, map: NodeId, edge: NodeId) {
        let old_len = self.map_edges(map).len();
        let key = self.node_edge_key(edge);
        let Node::Map(m) = self.node_mut(map) else {
            unreachable!()
        };
        m.edges.get_or_insert_default().push(edge);
        if let Some(cache) = self.edge_indexes.get_mut(&map)
            && cache.size == old_len
            && let Some(key) = key
        {
            cache.entries.entry(key).or_default().push(edge);
            cache.size += 1;
        } else {
            self.edge_indexes.remove(&map);
        }
        self.mark_structure_changed(map);
    }
    pub fn remove_edge(&mut self, map: NodeId, index: usize) -> NodeId {
        let Node::Map(m) = self.node_mut(map) else {
            panic!("expected IR map")
        };
        let edge = m.edges.as_mut().expect("nil map edges").remove(index);
        self.edge_indexes.remove(&map);
        self.mark_structure_changed(map);
        edge
    }
    /// Compiler lookup uses a derived cache. Direct mutation of an existing
    /// field name does not invalidate that cache, matching the source API.
    pub fn lookup_field(&mut self, map: NodeId, name: &StringBox) -> Option<NodeId> {
        name.unbox()?;
        let fields = self.map_fields(map);
        if self
            .field_indexes
            .get(&map)
            .is_none_or(|c| c.size != fields.len())
        {
            let mut cache = FieldIndex {
                size: fields.len(),
                entries: HashMap::new(),
            };
            for field in fields {
                if let Some(key) = self.node_field_key(*field) {
                    cache.entries.entry(key).or_insert(*field);
                }
            }
            self.field_indexes.insert(map, cache);
        }
        self.field_indexes[&map]
            .entries
            .get(&field_key(name))
            .copied()
    }
    /// Public lookup scans source order, including caller-mutated field slices.
    pub fn get_field(&self, mut map: NodeId, path: &[StringBox]) -> Option<NodeId> {
        while path.first().is_some_and(underscore) {
            map = self.parent_map(map)?;
        }
        self.get_field_linear(map, path)
    }
    fn get_field_linear(&self, map: NodeId, path: &[StringBox]) -> Option<NodeId> {
        let (name, rest) = path.split_first()?;
        if underscore(name) {
            return None;
        }
        name.with_scalar_bytes(|name_bytes| {
            let name_reserved = reserved_bytes(name_bytes);
            for id in self.map_fields(map) {
                let Node::Field(f) = self.node(*id) else {
                    panic!("expected IR field")
                };
                if f.name.unbox().is_none()
                    || !f
                        .name
                        .with_scalar_bytes(|bytes| super::equal_fold(bytes, name_bytes))
                {
                    continue;
                }
                if name_reserved
                    && f.name.unquoted_string.is_some() != name.unquoted_string.is_some()
                {
                    continue;
                }
                if rest.is_empty() {
                    return Some(*id);
                }
                if let Some(map) = self.map(*id) {
                    return self.get_field_linear(map, rest);
                }
            }
            None
        })
    }
    /// Source-ordered lookup for compiler property paths already stored as
    /// unquoted strings. Avoid constructing shared AST nodes just to query IR.
    /// Like GetField, this reads live field names and skips scalar duplicates
    /// when a longer path requires a map.
    pub fn get_field_unquoted(&self, mut map: NodeId, path: &[&str]) -> Option<NodeId> {
        // Keep GetField's leading-underscore behavior (including its unchanged
        // path) rather than using compiler-owned indexed lookup semantics.
        while path.first() == Some(&"_") {
            map = self.parent_map(map)?;
        }
        self.get_field_unquoted_linear(map, path)
    }
    fn get_field_unquoted_linear(&self, map: NodeId, path: &[&str]) -> Option<NodeId> {
        let (name, rest) = path.split_first()?;
        if *name == "_" {
            return None;
        }
        let name_reserved = reserved_bytes(name.as_bytes());
        for id in self.map_fields(map) {
            let Node::Field(field) = self.node(*id) else {
                panic!("expected IR field")
            };
            if field.name.unbox().is_none()
                || (name_reserved && field.name.unquoted_string.is_none())
                || !field
                    .name
                    .with_scalar_bytes(|bytes| super::equal_fold(bytes, name.as_bytes()))
            {
                continue;
            }
            if rest.is_empty() {
                return Some(*id);
            }
            if let Some(map) = self.map(*id) {
                return self.get_field_unquoted_linear(map, rest);
            }
        }
        None
    }
    pub fn get_field_indexed(&mut self, mut map: NodeId, path: &[StringBox]) -> Option<NodeId> {
        while path.first().is_some_and(underscore) {
            map = self.parent_map(map)?;
        }
        let (name, rest) = path.split_first()?;
        let field = self.lookup_field(map, name)?;
        if rest.is_empty() {
            return Some(field);
        }
        self.get_field_indexed(self.map(field)?, rest)
    }
    /// Query already-resolved local endpoints. Edge resolution and glob matching
    /// are separate stages; this cache preserves source order within each bucket.
    pub fn lookup_edges_local(&mut self, map: NodeId, eid: &EdgeId) -> Vec<NodeId> {
        let edges = self.map_edges(map);
        if self
            .edge_indexes
            .get(&map)
            .is_none_or(|c| c.size != edges.len())
        {
            let mut cache = EdgeIndex {
                size: edges.len(),
                entries: HashMap::new(),
            };
            for edge in edges {
                if let Some(key) = self.node_edge_key(*edge) {
                    cache.entries.entry(key).or_default().push(*edge);
                }
            }
            self.edge_indexes.insert(map, cache);
        }
        self.edge_indexes[&map]
            .entries
            .get(&edge_key(eid))
            .into_iter()
            .flatten()
            .copied()
            .filter(|edge| {
                let Node::Edge(e) = self.node(*edge) else {
                    unreachable!()
                };
                e.id.as_ref().unwrap().read().matches(eid)
            })
            .collect()
    }
}
fn underscore(name: &StringBox) -> bool {
    name.unquoted_string.is_some() && name.with_scalar_bytes(|bytes| bytes == b"_")
}
