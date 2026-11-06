// SPDX-License-Identifier: MPL-2.0
//! Session-local acceleration of d2ir/d2ir.go:1106–1150 GetField.
//!
//! Graph construction reads immutable IR. Keep both the first scalar candidate
//! and the first map candidate: GetField skips scalar duplicates for a longer
//! path, but never retries another map after the first map's nested miss.
use super::Compiler;
use d2_rs_core::{ast::keywords::is_reserved_keyword, ir};
use std::{collections::HashMap, rc::Rc};

#[derive(Default)]
struct Candidates {
    first: Option<ir::NodeId>,
    map: Option<ir::NodeId>,
    unquoted: Option<ir::NodeId>,
    unquoted_map: Option<ir::NodeId>,
}

// Standard property names are read repeatedly during graph lowering. A sparse
// bitset gives them lookup slots without hashing strings or reclassifying keys.
// Only present properties occupy candidate storage; arbitrary names retain the
// original folded-name table. These tables are never traversed for output.
fn property_slot(name: &str) -> Option<u32> {
    Some(match name {
        "label" => 0,
        "shape" => 1,
        "icon" => 2,
        "constraint" => 3,
        "tooltip" => 4,
        "link" => 5,
        "near" => 6,
        "width" => 7,
        "height" => 8,
        "direction" => 9,
        "top" => 10,
        "left" => 11,
        "grid-rows" => 12,
        "grid-columns" => 13,
        "grid-gap" => 14,
        "vertical-gap" => 15,
        "horizontal-gap" => 16,
        "class" => 17,
        "vars" => 18,
        "style" => 19,
        "source-arrowhead" => 20,
        "target-arrowhead" => 21,
        "classes" => 22,
        "layers" => 23,
        "scenarios" => 24,
        "steps" => 25,
        "opacity" => 26,
        "stroke" => 27,
        "fill" => 28,
        "fill-pattern" => 29,
        "stroke-width" => 30,
        "stroke-dash" => 31,
        "border-radius" => 32,
        "font" => 33,
        "font-size" => 34,
        "font-color" => 35,
        "bold" => 36,
        "italic" => 37,
        "underline" => 38,
        "text-transform" => 39,
        "shadow" => 40,
        "multiple" => 41,
        "double-border" => 42,
        "3d" => 43,
        "animated" => 44,
        "filled" => 45,
        _ => return None,
    })
}

#[derive(Default)]
struct MapFields {
    present: u64,
    properties: Vec<Candidates>,
    named: HashMap<String, Candidates>,
}
impl MapFields {
    fn property(&self, slot: u32) -> Option<&Candidates> {
        let bit = 1_u64 << slot;
        if self.present & bit == 0 {
            return None;
        }
        self.properties
            .get((self.present & (bit - 1)).count_ones() as usize)
    }
    fn get(&self, key: &str) -> Option<&Candidates> {
        match property_slot(key) {
            Some(slot) => self.property(slot),
            None => self.named.get(key),
        }
    }
    fn entry(&mut self, key: String) -> &mut Candidates {
        let Some(slot) = property_slot(&key) else {
            return self.named.entry(key).or_default();
        };
        let bit = 1_u64 << slot;
        let position = (self.present & (bit - 1)).count_ones() as usize;
        if self.present & bit == 0 {
            self.properties.insert(position, Candidates::default());
            self.present |= bit;
        }
        &mut self.properties[position]
    }
}

// Most objects have no classes. Keep that result allocation-free while
// sharing nonempty lists across immutable property reads.
#[derive(Clone, Default)]
pub(super) struct ClassMaps(Option<Rc<[ir::NodeId]>>);
impl From<Vec<ir::NodeId>> for ClassMaps {
    fn from(maps: Vec<ir::NodeId>) -> Self {
        Self((!maps.is_empty()).then(|| maps.into()))
    }
}
impl std::ops::Deref for ClassMaps {
    type Target = [ir::NodeId];
    fn deref(&self) -> &Self::Target {
        self.0.as_deref().unwrap_or_default()
    }
}

struct ClassMapEntry {
    root: ir::NodeId,
    maps: ClassMaps,
}

// compileMap applies class maps in order, then the object map. Resolve the
// scalar winners once, rather than walking that list for each property. Only
// present root/style properties occupy storage. IDs refer to immutable scalar
// nodes; no cloned AST values or text allocations are retained here.
#[derive(Default)]
struct ScalarFields {
    present: u128,
    values: Vec<ir::NodeId>,
}
impl ScalarFields {
    fn get(&self, slot: u32) -> Option<ir::NodeId> {
        let bit = 1_u128 << slot;
        (self.present & bit != 0)
            .then(|| self.values[(self.present & (bit - 1)).count_ones() as usize])
    }
    fn insert_first(&mut self, slot: u32, scalar: ir::NodeId) {
        let bit = 1_u128 << slot;
        if self.present & bit == 0 {
            self.values
                .insert((self.present & (bit - 1)).count_ones() as usize, scalar);
            self.present |= bit;
        }
    }
}
struct InheritedScalars {
    root: ir::NodeId,
    fields: ScalarFields,
}
fn scalar_slot(path: &[&str]) -> Option<u32> {
    match path {
        [key] => property_slot(key),
        ["style", key] => property_slot(key)
            .filter(|slot| *slot >= 26)
            .map(|slot| slot + 20),
        _ => None,
    }
}

#[derive(Default)]
pub(super) struct FieldIndex {
    // IR identities are dense arena offsets. Only visited maps need a field
    // table; the slot vector keeps other IR nodes to one usize apiece.
    // usize::MAX denotes an unvisited map. Neither vector is iterated for output.
    map_slots: Vec<usize>,
    maps: Vec<MapFields>,
    /// One immutable class list per dense node identity. Check the board root
    /// on every hit: substituted composites may be queried in another board.
    /// Lists are shared locally rather than copied on every property read.
    class_maps: Vec<Option<ClassMapEntry>>,
    inherited_scalars: Vec<Option<InheritedScalars>>,
}

impl FieldIndex {
    fn map_fields(&mut self, arena: &ir::Arena, map: ir::NodeId) -> &MapFields {
        if self.map_slots.len() <= map.0 {
            self.map_slots.resize(map.0 + 1, usize::MAX);
        }
        let slot = &mut self.map_slots[map.0];
        if *slot == usize::MAX {
            let ir::Node::Map(source) = arena.node(map) else {
                panic!("expected IR map")
            };
            let mut fields = MapFields::default();
            for id in source.fields.iter().flatten() {
                let ir::Node::Field(field) = arena.node(*id) else {
                    panic!("expected IR field")
                };
                if field.name.unbox().is_none() {
                    continue;
                }
                let mut key = field.name.with_scalar_bytes(ir::fold_string);
                key.make_ascii_lowercase();
                let candidates = fields.entry(key);
                candidates.first.get_or_insert(*id);
                let has_map = arena.map(*id).is_some();
                if has_map {
                    candidates.map.get_or_insert(*id);
                }
                if field.name.unquoted_string.is_some() {
                    candidates.unquoted.get_or_insert(*id);
                    if has_map {
                        candidates.unquoted_map.get_or_insert(*id);
                    }
                }
            }
            *slot = self.maps.len();
            self.maps.push(fields);
        }
        &self.maps[self.map_slots[map.0]]
    }

    fn add_scalar_fields(&mut self, arena: &ir::Arena, map: ir::NodeId, output: &mut ScalarFields) {
        let fields = self.map_fields(arena, map);
        let mut present = fields.present;
        for candidates in &fields.properties {
            let slot = present.trailing_zeros();
            present &= present - 1;
            if let Some(primary) = candidates.unquoted.and_then(|field| arena.primary(field))
                && matches!(arena.node(primary), ir::Node::Scalar(_))
            {
                output.insert_first(slot, primary);
            }
        }
        // For a nested path GetField chooses the first map candidate, even
        // when a preceding duplicate scalar supplied the root property.
        let style = fields
            .property(19)
            .and_then(|c| c.unquoted_map)
            .and_then(|field| arena.map(field));
        if let Some(style) = style {
            let fields = self.map_fields(arena, style);
            let mut present = fields.present;
            for candidates in &fields.properties {
                let slot = present.trailing_zeros();
                present &= present - 1;
                if slot < 26 {
                    continue;
                }
                if let Some(primary) = candidates.unquoted.and_then(|field| arena.primary(field))
                    && matches!(arena.node(primary), ir::Node::Scalar(_))
                {
                    output.insert_first(slot + 20, primary);
                }
            }
        }
    }

    fn get(&mut self, arena: &ir::Arena, mut map: ir::NodeId, path: &[&str]) -> Option<ir::NodeId> {
        // Preserve GetField's unchanged leading-underscore path, including
        // parent traversal, instead of treating '_' as a removable component.
        if path.first() == Some(&"_") {
            return arena.get_field_unquoted(map, path);
        }
        for (position, name) in path.iter().enumerate() {
            if *name == "_" {
                return None;
            }
            let fields = self.map_fields(arena, map);
            let (candidates, reserved) = if let Some(slot) = property_slot(name) {
                (fields.property(slot)?, true)
            } else {
                let mut lower = [0; 64];
                let folded;
                let ascii = name.is_ascii();
                let key = if ascii && !name.bytes().any(|byte| byte.is_ascii_uppercase()) {
                    *name
                } else if ascii && name.len() <= lower.len() {
                    for (i, byte) in name.bytes().enumerate() {
                        lower[i] = byte.to_ascii_lowercase();
                    }
                    std::str::from_utf8(&lower[..name.len()]).unwrap()
                } else {
                    let mut key = ir::fold_string(name.as_bytes());
                    key.make_ascii_lowercase();
                    folded = key;
                    folded.as_str()
                };
                let candidates = fields.get(key)?;
                let reserved = if ascii {
                    is_reserved_keyword(key)
                } else {
                    is_reserved_keyword(
                        std::str::from_utf8(&ir::go_lowercase(name.as_bytes())).unwrap(),
                    )
                };
                (candidates, reserved)
            };
            let last = position + 1 == path.len();
            let field = match (reserved, last) {
                (false, true) => candidates.first,
                (false, false) => candidates.map,
                (true, true) => candidates.unquoted,
                (true, false) => candidates.unquoted_map,
            }?;
            if last {
                return Some(field);
            }
            map = arena.map(field)?;
        }
        None
    }
}

/// Restore an outer session even on early errors or unwinding. No index lives
/// across public lowering calls, board mutations or canonical-property forks.
pub(super) struct Session<'a> {
    compiler: &'a Compiler,
    previous: Option<FieldIndex>,
}
impl Drop for Session<'_> {
    fn drop(&mut self) {
        self.compiler
            .graph_field_index
            .replace(self.previous.take());
    }
}
impl Compiler {
    pub(super) fn graph_field_session(&self) -> Session<'_> {
        Session {
            compiler: self,
            previous: self.graph_field_index.replace(Some(FieldIndex::default())),
        }
    }
    pub(super) fn remember_class_maps(
        &self,
        node: ir::NodeId,
        root: ir::NodeId,
        maps: impl FnOnce() -> Vec<ir::NodeId>,
    ) {
        // Map/field/edge holders of the same composite share its class list.
        let node = self.arena.map(node).unwrap_or(node);
        let mut session = self.graph_field_index.borrow_mut();
        let Some(index) = session.as_mut() else {
            return;
        };
        if index.class_maps.len() <= node.0 {
            index.class_maps.resize_with(node.0 + 1, || None);
        }
        let slot = &mut index.class_maps[node.0];
        if slot.as_ref().is_none_or(|cached| cached.root != root) {
            *slot = Some(ClassMapEntry {
                root,
                maps: maps().into(),
            });
        }
    }

    pub(super) fn session_class_maps(
        &self,
        node: ir::NodeId,
        root: ir::NodeId,
        resolve: impl FnOnce() -> Vec<ir::NodeId>,
    ) -> ClassMaps {
        let node = self.arena.map(node).unwrap_or(node);
        let cached = self.graph_field_index.borrow().as_ref().map(|index| {
            index
                .class_maps
                .get(node.0)
                .and_then(Option::as_ref)
                .filter(|cached| cached.root == root)
                .map(|cached| cached.maps.clone())
        });
        match cached {
            Some(Some(maps)) => return maps,
            Some(None) => {}
            None => return resolve().into(),
        }
        // Resolution reads fields through this index; the borrow is released.
        let maps: ClassMaps = resolve().into();
        if let Some(index) = self.graph_field_index.borrow_mut().as_mut() {
            if index.class_maps.len() <= node.0 {
                index.class_maps.resize_with(node.0 + 1, || None);
            }
            index.class_maps[node.0] = Some(ClassMapEntry {
                root,
                maps: maps.clone(),
            });
        }
        maps
    }
    /// Outer None leaves arbitrary paths and callers without an immutable
    /// lowering session on the original lookup. Inner None is a cached miss.
    pub(super) fn graph_scalar_inherited(
        &self,
        node: ir::NodeId,
        path: &[&str],
        classes: impl FnOnce() -> ClassMaps,
    ) -> Option<Option<ir::NodeId>> {
        let slot = scalar_slot(path)?;
        let Some(map) = self.arena.map(node) else {
            return Some(None);
        };
        let root = self
            .graph_class_root
            .unwrap_or_else(|| self.arena.root_map(map));
        {
            let session = self.graph_field_index.borrow();
            let index = session.as_ref()?;
            if let Some(entry) = index.inherited_scalars.get(map.0).and_then(Option::as_ref)
                && entry.root == root
            {
                return Some(entry.fields.get(slot));
            }
        }
        // Class resolution also reads the field index. Release its borrow
        // before resolving; diagnostics still execute in the validation walk.
        let classes = classes();
        let mut session = self.graph_field_index.borrow_mut();
        let index = session.as_mut()?;
        let mut fields = ScalarFields::default();
        index.add_scalar_fields(&self.arena, map, &mut fields);
        for class in classes.iter().rev().copied() {
            index.add_scalar_fields(&self.arena, class, &mut fields);
        }
        let value = fields.get(slot);
        if index.inherited_scalars.len() <= map.0 {
            index.inherited_scalars.resize_with(map.0 + 1, || None);
        }
        index.inherited_scalars[map.0] = Some(InheritedScalars { root, fields });
        Some(value)
    }

    pub(super) fn graph_field(&self, map: ir::NodeId, path: &[&str]) -> Option<ir::NodeId> {
        match self.graph_field_index.borrow_mut().as_mut() {
            Some(index) => index.get(&self.arena, map, path),
            None => self.arena.get_field_unquoted(map, path),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2_rs_core::ast;

    fn field(compiler: &mut Compiler, map: ir::NodeId, name: &str, unquoted: bool) -> ir::NodeId {
        let name = if unquoted {
            ast::raw_string_box(name, true)
        } else {
            ast::StringBox {
                double_quoted_string: Some(ast::Shared::new(ast::DoubleQuotedString::flat(name))),
                ..Default::default()
            }
        };
        let id = compiler.arena.insert(
            Some(map),
            ir::Node::Field(ir::Field {
                name,
                ..Default::default()
            }),
        );
        compiler.arena.append_field(map, id);
        id
    }
    fn child_map(compiler: &mut Compiler, field: ir::NodeId) -> ir::NodeId {
        let map = compiler
            .arena
            .insert(Some(field), ir::Node::Map(ir::Map::default()));
        let ir::Node::Field(value) = compiler.arena.node_mut(field) else {
            unreachable!()
        };
        value.composite = Some(map);
        map
    }

    #[test]
    fn indexed_properties_preserve_source_order_quoting_and_nested_misses() {
        let mut compiler = Compiler::default();
        let root = compiler.arena.new_root();
        field(&mut compiler, root, "label", false);
        let label = field(&mut compiler, root, "LABEL", true);
        field(&mut compiler, root, "K", false);
        let first_map_field = field(&mut compiler, root, "k", true);
        let first_map = child_map(&mut compiler, first_map_field);
        let nested = field(&mut compiler, first_map, "stroke", true);
        let second_map_field = field(&mut compiler, root, "K", true);
        let second_map = child_map(&mut compiler, second_map_field);
        field(&mut compiler, second_map, "fill", true);
        field(&mut compiler, root, "ſhape", false);
        let shape = field(&mut compiler, root, "SHAPE", true);
        field(&mut compiler, root, "a".repeat(100).as_str(), true);
        let nil = compiler
            .arena
            .insert(Some(root), ir::Node::Field(ir::Field::default()));
        compiler.arena.append_field(root, nil);
        let paths: &[&[&str]] = &[
            &[],
            &["label"],
            &["LABEL"],
            &["shape"],
            &["ſhape"],
            &["K"],
            &["k", "stroke"],
            &["k", "fill"],
            &["k", "_"],
            &["missing"],
            &["_", "label"],
            &[
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            ],
        ];
        for _ in 0..2 {
            let _session = compiler.graph_field_session();
            for _ in 0..3 {
                for path in paths {
                    let ast_path: Vec<_> = path
                        .iter()
                        .map(|name| ast::raw_string_box(*name, true))
                        .collect();
                    assert_eq!(
                        compiler.graph_field(root, path),
                        compiler.arena.get_field(root, &ast_path),
                        "{path:?}"
                    );
                }
            }
            assert_eq!(compiler.graph_field(root, &["label"]), Some(label));
            assert_eq!(compiler.graph_field(root, &["shape"]), Some(shape));
            assert_eq!(compiler.graph_field(root, &["k", "stroke"]), Some(nested));
            assert_eq!(compiler.graph_field(root, &["k", "fill"]), None);
        }
        assert!(compiler.graph_field_index.borrow().is_none());
        // A fresh session must see changed public slices and shared names.
        let ir::Node::Map(map) = compiler.arena.node_mut(root) else {
            unreachable!()
        };
        map.fields.as_mut().unwrap().reverse();
        let ir::Node::Field(value) = compiler.arena.node(label) else {
            unreachable!()
        };
        value.name.unquoted_string.as_ref().unwrap().write().value = None;
        let _session = compiler.graph_field_session();
        for path in paths {
            assert_eq!(
                compiler.graph_field(root, path),
                compiler.arena.get_field_unquoted(root, path),
                "mutated {path:?}"
            );
        }
    }

    #[test]
    fn standard_properties_match_public_lookup_with_duplicates_and_unicode_aliases() {
        use d2_rs_core::ast::keywords::{
            COMPOSITE_RESERVED_KEYWORDS, SIMPLE_RESERVED_KEYWORDS, STYLE_KEYWORDS,
        };
        let mut names: Vec<_> = SIMPLE_RESERVED_KEYWORDS
            .iter()
            .chain(STYLE_KEYWORDS)
            .chain(COMPOSITE_RESERVED_KEYWORDS)
            .copied()
            .collect();
        names.sort_unstable();
        names.dedup();
        let mut compiler = Compiler::default();
        let root = compiler.arena.new_root();
        // Insert opposite to query order: slot order must never select a later
        // duplicate or retry a later map after the first nested miss.
        for name in names.iter().rev() {
            assert!(property_slot(name).is_some(), "{name}");
            field(&mut compiler, root, name, false);
            field(&mut compiler, root, name, true);
            let first = field(&mut compiler, root, &name.to_uppercase(), true);
            let map = child_map(&mut compiler, first);
            field(&mut compiler, map, "found", true);
            let later = field(&mut compiler, root, name, true);
            let map = child_map(&mut compiler, later);
            field(&mut compiler, map, "later-only", true);
        }
        // Unicode EqualFold aliases may resolve the same candidate table while
        // remaining non-reserved under Go's distinct strings.ToLower rule.
        let aliases: Vec<_> = names.iter().map(|name| name.replace('s', "ſ")).collect();
        let _session = compiler.graph_field_session();
        for name in names
            .iter()
            .copied()
            .chain(aliases.iter().map(String::as_str))
        {
            for spelling in [name.to_owned(), name.to_uppercase()] {
                for tail in [None, Some("found"), Some("later-only"), Some("absent")] {
                    let mut path = vec![spelling.as_str()];
                    path.extend(tail);
                    let ast_path: Vec<_> = path
                        .iter()
                        .map(|name| ast::raw_string_box(*name, true))
                        .collect();
                    assert_eq!(
                        compiler.graph_field(root, &path),
                        compiler.arena.get_field(root, &ast_path),
                        "{path:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn inherited_scalar_tables_preserve_precedence_duplicates_and_fresh_sessions() {
        fn scalar(compiler: &mut Compiler, field: ir::NodeId) -> ir::NodeId {
            let id = compiler
                .arena
                .insert(Some(field), ir::Node::Scalar(ir::Scalar::default()));
            let ir::Node::Field(value) = compiler.arena.node_mut(field) else {
                unreachable!()
            };
            value.primary = Some(id);
            id
        }
        fn original(compiler: &Compiler, maps: &[ir::NodeId], path: &[&str]) -> Option<ir::NodeId> {
            maps.iter().find_map(|map| {
                let field = compiler.arena.get_field_unquoted(*map, path)?;
                let primary = compiler.arena.primary(field)?;
                matches!(compiler.arena.node(primary), ir::Node::Scalar(_)).then_some(primary)
            })
        }
        let mut compiler = Compiler::default();
        let root = compiler.arena.new_root();
        let node = field(&mut compiler, root, "object", true);
        let map = child_map(&mut compiler, node);
        let first = compiler.arena.new_root();
        let second = compiler.arena.new_root();
        for class in [first, second] {
            let label = field(&mut compiler, class, "label", true);
            scalar(&mut compiler, label);
            let shape = field(&mut compiler, class, "ſhape", true);
            scalar(&mut compiler, shape);
            let style = field(&mut compiler, class, "style", true);
            let style = child_map(&mut compiler, style);
            for name in ["fill", "stroke", "bold", "filled"] {
                let field = field(&mut compiler, style, name, true);
                scalar(&mut compiler, field);
            }
        }
        let quoted = field(&mut compiler, map, "label", false);
        scalar(&mut compiler, quoted);
        let label = field(&mut compiler, map, "LABEL", true);
        let direct_label = scalar(&mut compiler, label);
        let duplicate = field(&mut compiler, map, "label", true);
        scalar(&mut compiler, duplicate);
        // A direct field without a primary does not erase a class scalar.
        field(&mut compiler, map, "shape", true);
        // Nested lookups skip a scalar holder, then stop at the first map.
        let style_scalar = field(&mut compiler, map, "style", true);
        scalar(&mut compiler, style_scalar);
        let style = field(&mut compiler, map, "STYLE", true);
        let style = child_map(&mut compiler, style);
        let fill = field(&mut compiler, style, "fill", true);
        let direct_fill = scalar(&mut compiler, fill);
        field(&mut compiler, style, "stroke", true);
        let later = field(&mut compiler, map, "style", true);
        let later = child_map(&mut compiler, later);
        for name in ["bold", "italic"] {
            let field = field(&mut compiler, later, name, true);
            scalar(&mut compiler, field);
        }
        let paths: &[&[&str]] = &[
            &["label"],
            &["shape"],
            &["height"],
            &["style"],
            &["style", "fill"],
            &["style", "stroke"],
            &["style", "bold"],
            &["style", "italic"],
            &["style", "filled"],
        ];
        {
            let _session = compiler.graph_field_session();
            for path in paths {
                assert_eq!(
                    compiler
                        .graph_scalar_inherited(node, path, || vec![first, second, first].into()),
                    Some(original(&compiler, &[map, first, second, first], path)),
                    "{path:?}"
                );
                assert_eq!(
                    compiler.graph_scalar_inherited(map, path, || panic!("holder/map share table")),
                    Some(original(&compiler, &[map, first, second, first], path)),
                    "{path:?}"
                );
            }
            assert_eq!(
                compiler.graph_scalar_inherited(node, &["label"], || panic!("cached")),
                Some(Some(direct_label))
            );
            assert_eq!(
                compiler.graph_scalar_inherited(node, &["style", "fill"], || panic!("cached")),
                Some(Some(direct_fill))
            );
            assert_eq!(
                compiler.graph_scalar_inherited(node, &["LABEL"], || panic!("fallback")),
                None
            );
            assert_eq!(
                compiler.graph_scalar_inherited(node, &["source-arrowhead", "shape"], || panic!(
                    "fallback"
                )),
                None
            );
            {
                let _nested = compiler.graph_field_session();
                for path in paths {
                    assert_eq!(
                        compiler.graph_scalar_inherited(node, path, ClassMaps::default),
                        Some(original(&compiler, &[map], path))
                    );
                }
            }
            for path in paths {
                assert_eq!(
                    compiler.graph_scalar_inherited(node, path, || panic!("outer restored")),
                    Some(original(&compiler, &[map, first, second, first], path))
                );
            }
        }
        let replacement = scalar(&mut compiler, label);
        let _session = compiler.graph_field_session();
        assert_eq!(
            compiler.graph_scalar_inherited(node, &["label"], ClassMaps::default),
            Some(Some(replacement))
        );
        assert!(
            compiler
                .graph_scalar_inherited(node, &["style", "italic"], ClassMaps::default)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn class_map_sessions_preserve_occurrences_board_roots_and_fresh_ir() {
        let mut compiler = Compiler::default();
        let root = compiler.arena.new_root();
        let board = compiler.arena.new_root();
        let node = field(&mut compiler, root, "object", true);
        let map = child_map(&mut compiler, node);
        let first = field(&mut compiler, root, "first", true);
        let second = field(&mut compiler, board, "second", true);
        {
            let _session = compiler.graph_field_session();
            compiler.remember_class_maps(node, root, || vec![first, second, first]);
            let maps = compiler.session_class_maps(map, root, || panic!("already resolved"));
            assert_eq!(&*maps, &[first, second, first]);
            assert!(Rc::ptr_eq(
                maps.0.as_ref().unwrap(),
                compiler
                    .session_class_maps(node, root, || panic!("cached"))
                    .0
                    .as_ref()
                    .unwrap()
            ));
            // The same substituted node can be read through a different
            // board namespace. Its previous result must never leak across.
            assert_eq!(
                &*compiler.session_class_maps(node, board, || vec![second]),
                &[second]
            );
            assert_eq!(&*maps, &[first, second, first]);
            assert_eq!(
                &*compiler.session_class_maps(node, root, || vec![first]),
                &[first]
            );
            {
                let _nested = compiler.graph_field_session();
                assert_eq!(&*compiler.session_class_maps(node, root, Vec::new), &[]);
            }
            assert_eq!(
                &*compiler.session_class_maps(node, root, || panic!("outer restored")),
                &[first]
            );
        }
        // IR may change between public lowering calls. A new session sees
        // the new resolution, including an empty class list.
        let _session = compiler.graph_field_session();
        let empty = compiler.session_class_maps(node, root, Vec::new);
        assert_eq!(&*empty, &[]);
        assert!(empty.0.is_none());
    }

    #[test]
    fn nested_sessions_restore_the_outer_index() {
        let mut compiler = Compiler::default();
        let root = compiler.arena.new_root();
        let label = field(&mut compiler, root, "label", true);
        let _outer = compiler.graph_field_session();
        assert_eq!(compiler.graph_field(root, &["label"]), Some(label));
        assert_eq!(
            compiler
                .graph_field_index
                .borrow()
                .as_ref()
                .unwrap()
                .maps
                .len(),
            1
        );
        {
            let _inner = compiler.graph_field_session();
            assert!(
                compiler
                    .graph_field_index
                    .borrow()
                    .as_ref()
                    .unwrap()
                    .maps
                    .is_empty()
            );
        }
        assert_eq!(
            compiler
                .graph_field_index
                .borrow()
                .as_ref()
                .unwrap()
                .maps
                .len(),
            1
        );
    }
}
