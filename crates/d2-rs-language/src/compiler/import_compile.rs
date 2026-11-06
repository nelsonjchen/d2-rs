// SPDX-License-Identifier: MPL-2.0
//! Imported compilation and attachment from pinned d2ir/import.go/compile.go.
use super::{
    Compiler,
    edge_creation::error,
    imports::{import_template_safe_at, nil_scope_map},
};
use d2_rs_core::{
    ast::{self, Shared},
    ir::*,
};
impl Compiler {
    fn load_import_ast(
        &mut self,
        path: &str,
        import: &Shared<ast::Import>,
        errors: &mut crate::parser::ParseError,
    ) -> (Option<Shared<ast::Map>>, Option<String>) {
        if self.stopped() {
            return (None, None);
        }
        let Some(opener) = self.import_opener.as_mut() else {
            return (
                None,
                Some("d2ir: imports are disabled; provide CompileOptions.FS to enable them".into()),
            );
        };
        let (source, open_error) = self.import_state.load_stream_template(path, errors, opener);
        if let Some(fault) = errors.fault.take() {
            self.parser_fault = Some(fault);
            self.halted = true;
            return (None, None);
        }
        let Some(source) = source else {
            return (None, open_error);
        };
        if !self.reserve_variable_ast_copy(
            &ast::Child::Import(import.clone()),
            ast::Child::Map(source.clone()),
        ) {
            return (None, None);
        }
        match clone_ast_with_context(&ast::Child::Map(source), &mut || {
            self.context.as_mut().and_then(|check| check())
        }) {
            Ok(ast::Child::Map(map)) => (Some(map), None),
            Err(error) => {
                self.context_error = Some(error);
                self.halted = true;
                (None, None)
            }
            _ => unreachable!(),
        }
    }
    pub(super) fn peek_import(&mut self, import: Shared<ast::Import>) -> Option<NodeId> {
        let path = self.import_state.path(&import.read())?;
        let mut errors = crate::parser::ParseError::default();
        let (source, _) = self.load_import_ast(&path, &import, &mut errors);
        let source = source?;
        let root = self.arena.new_root();
        let Node::Field(parent) = self.arena.node(self.arena.parent(root).unwrap()) else {
            unreachable!()
        };
        parent.references.as_ref().unwrap()[0]
            .read()
            .context
            .as_ref()
            .unwrap()
            .write()
            .scope = Some(source.clone());
        self.compile_map(root, source.clone(), source);
        (!self.stopped()).then_some(root)
    }
    pub(super) fn import_node(
        &mut self,
        import: Shared<ast::Import>,
        importer: NodeId,
    ) -> Option<NodeId> {
        if self.stopped() {
            return None;
        }
        let mut errors = crate::parser::ParseError {
            errors: Some(std::mem::take(&mut self.errors)),
            ..Default::default()
        };
        let path = self.import_state.push(import.clone(), &mut errors);
        self.errors = errors.errors.unwrap_or_default();
        let path = path?;
        // Match Go's `defer c.popImportStack()`: a nested compilation can
        // panic, and callers that recover must see the same parent import
        // stack they had before entering this import.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.import_at_path(&path, import, importer)
        }));
        self.import_state.pop();
        match result {
            Ok(result) => result,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }
    fn import_at_path(
        &mut self,
        path: &str,
        import: Shared<ast::Import>,
        importer: NodeId,
    ) -> Option<NodeId> {
        if self.import_state.stack.len() == 2 && !self.seen_imports.contains(path) {
            self.imports.push(import.read().path_with_pre());
        }
        let cacheable = import_template_safe_at(&self.arena, Some(importer))
            && self.glob_contexts().is_empty()
            && self.map_ref_context_stack.is_empty();
        if cacheable && let Some(template) = self.import_templates.get_mut(path) {
            self.seen_imports.insert(path.into());
            let selective = matches!(self.arena.node(importer), Node::Field(_))
                && !import.read().spread
                && import.read().ida().is_some()
                && template.can_select(&self.arena);
            let source = template.ir;
            if selective {
                let path: Vec<_> = import
                    .read()
                    .ida()
                    .unwrap()
                    .into_iter()
                    .map(|p| ast::ValueBox::from_child(p).string_box())
                    .collect();
                if let Some(field) = self.arena.get_field_indexed(source, &path) {
                    if !self.reserve_variable_copy(&ast::Child::Import(import.clone()), field) {
                        return None;
                    }
                    let field = self.arena.clone_import_field(field);
                    nil_scope_map(&self.arena, field);
                    return Some(field);
                }
                self.import_key_error(import, &path);
                return None;
            }
            if !self.reserve_variable_copy(&ast::Child::Import(import.clone()), source) {
                return None;
            }
            let clone = self.arena.clone_import_map(source);
            return self.select_import(clone, import);
        }
        let count = self.errors.len();
        let mut errors = crate::parser::ParseError {
            errors: Some(std::mem::take(&mut self.errors)),
            ..Default::default()
        };
        let (source, open_error) = self.load_import_ast(path, &import, &mut errors);
        // Reservation diagnostics are written directly to the compiler while
        // parser diagnostics use the detached accumulator.
        let reservation_errors = std::mem::take(&mut self.errors);
        self.errors = errors.errors.unwrap_or_default();
        self.errors.extend(reservation_errors);
        let Some(source) = source else {
            if let Some(open_error) = open_error {
                self.errors.push(error(
                    &ast::Child::Import(import),
                    format!(
                        "failed to import {}: {open_error}",
                        crate::parser::quote_input(path.as_bytes())
                    ),
                ));
            }
            return None;
        };
        let root = self.arena.new_root();
        let Node::Field(parent) = self.arena.node(self.arena.parent(root).unwrap()) else {
            unreachable!()
        };
        parent.references.as_ref().unwrap()[0]
            .read()
            .context
            .as_ref()
            .unwrap()
            .write()
            .scope = Some(source.clone());
        self.compile_map(root, source.clone(), source);
        if self.stopped() {
            return None;
        }
        let saved = self.errors.clone();
        self.compile_substitutions(root, Vec::new());
        self.errors = saved;
        if self.stopped() {
            return None;
        }
        self.seen_imports.insert(path.into());
        if cacheable && self.errors.len() == count {
            if !self.reserve_variable_copy(&ast::Child::Import(import.clone()), root) {
                return None;
            }
            let clone = self.arena.clone_import_map(root);
            self.import_templates
                .insert(path.into(), ImportTemplate::new(clone));
        }
        self.select_import(root, import)
    }
    pub(super) fn set_import_ast(&mut self, node: NodeId, import: Shared<ast::Import>) {
        let children = match self.arena.node_mut(node) {
            Node::Map(m) => {
                m.import_ast = Some(ast::Child::Import(import.clone()));
                m.fields
                    .iter()
                    .flatten()
                    .chain(m.edges.iter().flatten())
                    .copied()
                    .collect::<Vec<_>>()
            }
            Node::Field(f) => {
                f.import_ast = Some(ast::Child::Import(import.clone()));
                self.arena.map(node).into_iter().collect()
            }
            Node::Edge(e) => {
                e.import_ast = Some(ast::Child::Import(import.clone()));
                self.arena.map(node).into_iter().collect()
            }
            _ => unreachable!(),
        };
        for child in children {
            self.set_import_ast(child, import.clone());
        }
    }
    pub(super) fn compile_field_import(
        &mut self,
        field: NodeId,
        import: Shared<ast::Import>,
        scope: NodeId,
    ) {
        let Some(imported) = self.import_node(import.clone(), field) else {
            return;
        };
        self.set_import_ast(imported, import.clone());
        let existing = self
            .arena
            .map(field)
            .map(|m| {
                let Node::Map(m) = self.arena.node(m) else {
                    unreachable!()
                };
                m.edges.clone().unwrap_or_default()
            })
            .unwrap_or_default();
        let original = self.arena.copy(field, Some(scope));
        match self.arena.node(imported).clone() {
            Node::Field(f) => {
                if let Some(primary) = f.primary {
                    let primary = self.arena.copy(primary, Some(field));
                    let Node::Field(f) = self.arena.node_mut(field) else {
                        unreachable!()
                    };
                    f.primary = Some(primary);
                }
                if let Some(composite) = f.composite {
                    let before = self.arena.map(field).map_or((0, 0), |m| {
                        (
                            self.arena.field_count_recursive(m),
                            self.arena.edge_count_recursive(m),
                        )
                    });
                    let composite = self.arena.copy(composite, Some(field));
                    let Node::Field(f) = self.arena.node_mut(field) else {
                        unreachable!()
                    };
                    f.composite = Some(composite);
                    let after = self.arena.map(field).map_or((0, 0), |m| {
                        (
                            self.arena.field_count_recursive(m),
                            self.arena.edge_count_recursive(m),
                        )
                    });
                    if before != after {
                        self.arena
                            .mark_structure_changed(self.arena.parent_map(field).unwrap());
                    }
                }
            }
            Node::Map(_) => {
                let map = self.arena.insert(Some(field), Node::Map(Map::default()));
                let Node::Field(f) = self.arena.node_mut(field) else {
                    unreachable!()
                };
                f.composite = Some(map);
                self.inherit_board(field);
                let map = self.arena.map(field).unwrap();
                self.arena.overlay_map(map, imported, true);
                self.extend_import_links(map, Some(field), import.read().dir().as_bytes());
                if matches!(self.arena.board_kind(field), "scenario" | "step") {
                    self.overlay_classes(map);
                }
            }
            _ => unreachable!(),
        }
        self.arena.overlay_field(field, original, true);
        if let Some(map) = self.arena.map(field) {
            for edge in existing {
                let Node::Edge(e) = self.arena.node(edge) else {
                    unreachable!()
                };
                let id = e.id.clone().unwrap();
                let Node::Map(m) = self.arena.node(map) else {
                    unreachable!()
                };
                let exists = m.edges.iter().flatten().any(|e| {
                    let Node::Edge(e) = self.arena.node(*e) else {
                        unreachable!()
                    };
                    e.id.as_ref().unwrap().read().matches(&id.read())
                });
                if !exists {
                    self.arena.append_edge(map, edge);
                }
            }
        }
    }
    pub(super) fn compile_spread_import(&mut self, dst: NodeId, import: Shared<ast::Import>) {
        let Some(imported) = self.import_node(import.clone(), dst) else {
            return;
        };
        let Some(map) = self.arena.map(imported) else {
            self.errors.push(error(
                &ast::Child::Import(import),
                "cannot spread import non map into map",
            ));
            return;
        };
        self.set_import_ast(imported, import.clone());
        let Node::Map(m) = self.arena.node(map) else {
            unreachable!()
        };
        let globs = m
            .globs
            .iter()
            .flat_map(ast::SharedSlice::iter)
            .map(|g| g.read().unwrap())
            .collect::<Vec<_>>();
        for glob in globs {
            if self
                .arena
                .glob(glob)
                .context
                .as_ref()
                .unwrap()
                .read()
                .key
                .as_ref()
                .unwrap()
                .read()
                .has_triple_glob()
            {
                let copy = self.arena.copy_glob_context(glob);
                let context = self.arena.glob(copy).context.clone().unwrap();
                context.write().scope_map = Some(dst);
                self.compile_key(context.clone());
                self.ensure_glob_context(context);
            }
        }
        for kind in ["scenarios", "steps"] {
            if let Some(board) = self
                .arena
                .get_field_indexed(map, &[ast::raw_string_box(kind, true)])
                .and_then(|f| self.arena.map(f))
            {
                let Node::Map(m) = self.arena.node(board) else {
                    unreachable!()
                };
                let fields = m.fields.clone().unwrap_or_default();
                for field in fields {
                    self.overlay_board(Some(dst), field);
                }
            }
        }
        if !self.reserve_variable_copy(&ast::Child::Import(import.clone()), map) {
            return;
        }
        self.arena.overlay_map(dst, map, true);
        self.extend_import_links(
            dst,
            self.arena.parent_field(dst),
            import.read().dir().as_bytes(),
        );
        if let Node::Field(f) = self.arena.node(imported)
            && let Some(primary) = f.primary
            && let Some(parent) = self.arena.parent_field(dst)
        {
            let Node::Field(f) = self.arena.node_mut(parent) else {
                unreachable!()
            };
            f.primary = Some(primary);
        }
    }
}

#[cfg(test)]
mod expansion_tests {
    use super::*;

    #[test]
    fn map_import_spread_reserves_the_overlay_copy_before_attachment() {
        // Pinned d2ir/compile.go reserves impn.Map() at the spread import
        // before overlayMapIndexed. Go accepts the small control and rejects
        // the 500-field stress2 witness at input.d2:1:1 with the default cap.
        for count in [8, 500] {
            let library = (0..count)
                .map(|i| format!("shared{i}: shared {i}\n"))
                .collect::<String>();
            let input = "...@lib\n".to_owned()
                + &(0..count)
                    .map(|i| format!("o{i}: obj {i}\n"))
                    .collect::<String>()
                + &(0..count)
                    .map(|i| format!("o{i} -> shared{i}\n"))
                    .collect::<String>();
            let parsed = crate::parse("input.d2", input.as_bytes(), None);
            assert!(parsed.error.is_none());
            let (compiler, _) = crate::compiler::compile_ir_with_limits(
                Shared::new(parsed.map),
                crate::compiler::CompileOptions {
                    import_opener: Some(Box::new(move |path| {
                        assert_eq!(path, "lib.d2");
                        Ok(Box::new(std::io::Cursor::new(library.as_bytes().to_vec())))
                    })),
                    ..Default::default()
                },
                crate::compiler::CompileLimits::default(),
            )
            .unwrap();
            if count == 8 {
                assert!(compiler.errors.is_empty(), "{:?}", compiler.errors);
            } else {
                assert_eq!(compiler.errors.len(), 1);
                assert_eq!(
                    compiler.errors[0].to_string(),
                    "input.d2:1:1: variable substitution expansion exceeds limit of 65536 work units"
                );
            }
        }
    }

    #[test]
    fn imported_array_spread_charges_inserted_members() {
        // Original master TestImportedArraySpreadChargesInsertedMembers.
        let used = |input: &str| {
            let parsed = crate::parse("index.d2", input.as_bytes(), None);
            assert!(parsed.error.is_none());
            let (compiler, _) = crate::compiler::compile_ir_with_limits(
                Shared::new(parsed.map),
                crate::compiler::CompileOptions {
                    import_opener: Some(Box::new(|path| {
                        assert_eq!(path, "library.d2");
                        Ok(Box::new(std::io::Cursor::new(
                            b"items: [a; b; c; d]".to_vec(),
                        )))
                    })),
                    ..Default::default()
                },
                crate::compiler::CompileLimits {
                    max_variable_expansion: 1_024,
                    ..Default::default()
                },
            )
            .unwrap();
            assert!(compiler.errors.is_empty(), "{:?}", compiler.errors);
            compiler.variable_expansion.used.get()
        };
        assert_eq!(
            used("out: [...@library.items]") - used("out: [@library.items]"),
            21
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2_rs_core::ast::Node as _;
    use std::io::Cursor;

    #[test]
    fn nested_import_panic_restores_parent_import_stack() {
        let source = crate::parse("main.d2", b"seed: value", None);
        let (mut compiler, root) = super::super::compile::compile_ir(Shared::new(source.map))
            .expect("plain source compiles");
        compiler.glob_context_stack.push(Vec::new());
        compiler.import_state.stack.push("main.d2".into());
        compiler.import_opener = Some(Box::new(|path| {
            let source = match path {
                "lib.d2" => b"x: [@empty.empty]".as_slice(),
                "empty.d2" => b"empty".as_slice(),
                _ => return Err(format!("unexpected import path {path}")),
            };
            Ok(Box::new(Cursor::new(source.to_vec())) as Box<dyn crate::stream::ByteReader>)
        }));

        let import_source = crate::parse("main.d2", b"x: @lib", None);
        let ast::Child::Key(key) = import_source.map.children()[0].clone() else {
            panic!("expected field key")
        };
        let import = key.read().value.import.clone().expect("field import");

        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            compiler.import_node(import, root)
        }));
        assert!(
            panic.is_err(),
            "nested nil import should retain the Go panic"
        );
        assert_eq!(compiler.import_state.stack, ["main.d2"]);
    }
}
