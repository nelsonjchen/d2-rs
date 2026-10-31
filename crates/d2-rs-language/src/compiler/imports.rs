// SPDX-License-Identifier: MPL-2.0
//! Path stack and parsed-AST cache from d2ir/import.go. IR attachment is separate.
use super::edge_creation::error;
use crate::{ParseOptions, parser::ParseError};
use d2_rs_core::{
    ast::{self, Shared},
    ir::clone_ast,
};
use std::collections::HashMap;
/// Maximum imported files in one active chain, excluding the root source.
pub const MAX_IMPORT_DEPTH: usize = 128;
#[derive(Default)]
pub struct ImportState {
    pub stack: Vec<String>,
    pub parsed: HashMap<String, Shared<ast::Map>>,
    pub utf16_positions: bool,
}
impl ImportState {
    /// Resolve against the current source filename. Slash cleaning follows Go
    /// path on all hosts; absolute detection follows the native platform.
    pub fn path(&self, import: &ast::Import) -> Option<String> {
        let mut path = import.path_with_pre();
        if path.is_empty() && import.range != ast::Range::default() {
            return None;
        }
        if let Some(parent) = self.stack.last() {
            let last = path.rsplit('/').next().unwrap();
            if last.rfind('.').map(|i| &last[i..]) != Some(".d2") {
                path.push_str(".d2");
            }
            if !std::path::Path::new(&path).is_absolute() {
                let dir = parent.rfind('/').map_or(".", |i| &parent[..=i]);
                // Import.PathWithPre already implements Go's lexical path.Join.
                path = ast::Import {
                    pre: dir.into(),
                    path: ast::KeyPath::from_strings([path]).path,
                    ..Default::default()
                }
                .path_with_pre();
            }
        }
        Some(path)
    }
    pub fn push(&mut self, import: Shared<ast::Import>, errors: &mut ParseError) -> Option<String> {
        let Some(path) = self.path(&import.read()) else {
            errors.errors.get_or_insert_default().push(error(
                &ast::Child::Import(import),
                "imports must specify a path to import",
            ));
            return None;
        };
        if self.stack.len() > MAX_IMPORT_DEPTH {
            errors.errors.get_or_insert_default().push(error(
                &ast::Child::Import(import),
                format!("maximum import depth of {MAX_IMPORT_DEPTH} exceeded"),
            ));
            return None;
        }
        if let Some(i) = self.stack.iter().position(|entry| entry == &path) {
            let mut chain = self.stack[i..].to_vec();
            chain.push(self.stack[i].clone());
            errors.errors.get_or_insert_default().push(error(
                &ast::Child::Import(import),
                format!("detected cyclic import chain: {}", chain.join(" -> ")),
            ));
            return None;
        }
        self.stack.push(path.clone());
        Some(path)
    }
    pub fn pop(&mut self) {
        self.stack.pop().expect("empty import stack");
    }
    /// Load complete file bytes through an explicit provider. Provider failures
    /// return separately from parser errors, as in d2ir.loadImportAST.
    pub fn load_ast(
        &mut self,
        path: &str,
        errors: &mut ParseError,
        mut read: impl FnMut(&str) -> Result<Vec<u8>, String>,
    ) -> (Option<Shared<ast::Map>>, Option<String>) {
        self.load_stream(path, errors, |path| {
            read(path).map(|bytes| {
                Box::new(std::io::Cursor::new(bytes)) as Box<dyn crate::stream::ByteReader>
            })
        })
    }
    /// Open only on cache misses; consume and drop the returned reader before
    /// returning. Read faults become parse diagnostics; open errors stay separate.
    pub fn load_stream(
        &mut self,
        path: &str,
        errors: &mut ParseError,
        open: impl FnMut(&str) -> Result<Box<dyn crate::stream::ByteReader>, String>,
    ) -> (Option<Shared<ast::Map>>, Option<String>) {
        let (map, error) = self.load_stream_template(path, errors, open);
        (map.map(|map| copy_map(&map)), error)
    }
    // The compiler must reserve the complete copy before cloning. Keep the
    // immutable parsed template private to this boundary; public loaders still
    // return independent trees.
    pub(super) fn load_stream_template(
        &mut self,
        path: &str,
        errors: &mut ParseError,
        mut open: impl FnMut(&str) -> Result<Box<dyn crate::stream::ByteReader>, String>,
    ) -> (Option<Shared<ast::Map>>, Option<String>) {
        if let Some(map) = self.parsed.get(path) {
            return (Some(map.clone()), None);
        }
        let mut reader = match open(path) {
            Ok(reader) => reader,
            Err(error) => return (None, Some(error)),
        };
        let parsed = crate::parse_stream(
            path,
            reader.as_mut(),
            Some(ParseOptions {
                utf16_positions: self.utf16_positions,
                errors: Some(errors),
            }),
        );
        if parsed.error.is_some() {
            return (None, None);
        }
        let map = Shared::new(parsed.map);
        self.parsed.insert(path.into(), map.clone());
        (Some(map), None)
    }
}
pub(super) fn copy_map(map: &Shared<ast::Map>) -> Shared<ast::Map> {
    let ast::Child::Map(map) = clone_ast(&ast::Child::Map(map.clone())) else {
        unreachable!()
    };
    map
}

/// Clear only reference scope maps, preserving AST scopes and shared contexts.
/// Like upstream, traversal deliberately does not descend into arrays.
pub fn nil_scope_map(arena: &d2_rs_core::ir::Arena, node: d2_rs_core::ir::NodeId) {
    use d2_rs_core::ir::Node;
    match arena.node(node) {
        Node::Map(map) => {
            for child in map
                .fields
                .iter()
                .flatten()
                .chain(map.edges.iter().flatten())
            {
                nil_scope_map(arena, *child);
            }
        }
        Node::Field(field) => {
            for reference in field.references.iter().flatten() {
                reference
                    .read()
                    .context
                    .as_ref()
                    .expect("nil reference context")
                    .write()
                    .scope_map = None;
            }
            if let Some(map) = arena.map(node) {
                nil_scope_map(arena, map);
            }
        }
        Node::Edge(edge) => {
            for reference in edge.references.iter().flatten() {
                reference
                    .read()
                    .context
                    .as_ref()
                    .expect("nil reference context")
                    .write()
                    .scope_map = None;
            }
            if let Some(map) = arena.map(node) {
                nil_scope_map(arena, map);
            }
        }
        _ => {}
    }
}
/// Board attachment establishes inheritance, so it cannot reuse compiled IR.
pub fn import_template_safe_at(
    arena: &d2_rs_core::ir::Arena,
    mut node: Option<d2_rs_core::ir::NodeId>,
) -> bool {
    while let Some(id) = node {
        if !arena.board_kind(id).is_empty() && !arena.root(id) {
            return false;
        }
        node = arena.parent(id);
    }
    true
}
impl super::Compiler {
    /// Select a map or field after clearing imported reference scopes.
    pub fn select_import(
        &mut self,
        root: d2_rs_core::ir::NodeId,
        import: Shared<ast::Import>,
    ) -> Option<d2_rs_core::ir::NodeId> {
        nil_scope_map(&self.arena, root);
        let ida = import.read().ida().unwrap_or_default();
        if ida.is_empty() {
            return Some(root);
        }
        let path: Vec<_> = ida
            .into_iter()
            .map(|child| ast::ValueBox::from_child(child).string_box())
            .collect();
        let result = self.arena.get_field_indexed(root, &path);
        if result.is_none() {
            self.import_key_error(import, &path);
        }
        result
    }
    pub(super) fn import_key_error(
        &mut self,
        import: Shared<ast::Import>,
        path: &[ast::StringBox],
    ) {
        // Upstream uses %q on AST string pointers, producing an unstable
        // fmt failure with heap addresses. This readable diagnostic remains
        // a recorded parity failure (retired docs/import-selection-parity.json;
        // see docs/RETIRED-EVIDENCE.md).
        let quoted = path
            .iter()
            .map(|part| crate::parser::quote_input(&part.scalar_bytes()))
            .collect::<Vec<_>>()
            .join(" ");
        self.errors.push(error(
            &ast::Child::Import(import),
            format!("import key [{quoted}] doesn't exist inside import"),
        ));
    }
}

/// Filesystem boundary for native and WASM callers.
pub type ImportOpener = Box<dyn FnMut(&str) -> Result<Box<dyn crate::stream::ByteReader>, String>>;
