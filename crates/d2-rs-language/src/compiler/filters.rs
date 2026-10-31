// SPDX-License-Identifier: MPL-2.0
//! Glob predicates from pinned d2ir/compile.go.
use super::{Compiler, edge_creation::error};
use d2_rs_core::{
    ast::{self, Node as AstNode, Shared},
    ir::*,
};
impl Compiler {
    pub(super) fn filter_map(
        &mut self,
        dst: NodeId,
        source: Shared<ast::Map>,
        scope: Shared<ast::Map>,
    ) -> bool {
        for node in source.read().children() {
            let ast::Child::Key(key) = node else { continue };
            let context = Shared::new(RefContext {
                key: Some(key.clone()),
                scope: Some(source.clone()),
                scope_map: Some(dst),
                scope_ast: Some(scope.clone()),
                ..Default::default()
            });
            let mut ok = self.filter(context);
            if key.read().not_ampersand {
                ok = !ok;
            }
            if !ok {
                if let Some(parent) = self.map_ref_context_stack.last().cloned()
                    && let Some(glob) = self.get_glob_context(&parent)
                {
                    let context = self.arena.glob(glob).context.clone().unwrap();
                    let path = self.path_key(dst, &context);
                    for set in [
                        &self.arena.glob(glob).applied_fields,
                        &self.arena.glob(glob).applied_edges,
                    ]
                    .into_iter()
                    .flatten()
                    {
                        set.write().remove(&path);
                    }
                }
                return false;
            }
        }
        true
    }
    pub(super) fn filter_value_map(&mut self, node: NodeId, context: Shared<RefContext>) -> bool {
        let source = context
            .read()
            .key
            .as_ref()
            .unwrap()
            .read()
            .value
            .map
            .clone();
        let Some(source) = source.filter(|m| m.read().has_filter()) else {
            return true;
        };
        let dst = if matches!(self.arena.node(node), Node::Edge(_)) {
            self.ensure_edge_map(node)
        } else {
            self.ensure_map(node)
        };
        self.map_ref_context_stack.push(context.clone());
        let scope = context.read().scope_ast.clone().unwrap();
        let ok = self.filter_map(dst, source, scope);
        self.map_ref_context_stack.pop();
        ok
    }
    fn filter(&mut self, context: Shared<RefContext>) -> bool {
        let key = context.read().key.clone().unwrap();
        if !key.read().ampersand && !key.read().not_ampersand {
            return true;
        }
        if !self.map_ref_context_stack.last().is_some_and(|r| {
            r.read()
                .key
                .as_ref()
                .unwrap()
                .read()
                .supports_glob_filters()
        }) {
            self.errors.push(error(
                &ast::Child::Key(key),
                "glob filters cannot be used outside globs",
            ));
            return false;
        }
        if key.read().edges.as_ref().is_some_and(|e| !e.is_empty()) {
            return true;
        }
        let Some(path) = key.read().key.clone() else {
            return false;
        };
        let parts: Vec<_> = path
            .read()
            .path
            .iter()
            .flat_map(ast::SharedSlice::iter)
            .map(|s| s.read().clone().unwrap())
            .collect();
        if parts.is_empty() {
            return false;
        }
        let first = parts[0].read().scalar_bytes();
        let map = context.read().scope_map.unwrap();
        let value = key.read().value.scalar_box();
        if matches!(first.as_slice(), b"src" | b"dst") && parts.len() > 1 {
            let Some(edge) = self.arena.parent_edge(map) else {
                return false;
            };
            let boxes = self.filter_endpoint(edge, first == b"src");
            let root = self.arena.root_map(map);
            let Some(node) = self.arena.get_field_indexed(root, &boxes) else {
                return false;
            };
            let Some(node_map) = self.arena.map(node) else {
                return false;
            };
            let second = parts[1].read().scalar_bytes();
            let bytes = value.scalar_bytes();
            if parts.len() == 2 && self.filter_property(&second, &bytes, node, key.clone()) {
                return true;
            }
            let prop_path = Shared::new(ast::KeyPath {
                path: Some(path.read().path.as_ref().unwrap().suffix(1)),
                ..Default::default()
            });
            let prop_key = Shared::new(ast::Key {
                key: Some(prop_path.clone()),
                value: key.read().value.clone(),
                ..Default::default()
            });
            let prop_context = Shared::new(RefContext {
                key: Some(prop_key.clone()),
                scope_map: Some(node_map),
                scope_ast: context.read().scope_ast.clone(),
                ..Default::default()
            });
            let found = self.ensure_fields(node_map, prop_path, prop_context, false);
            return found.error.is_none()
                && found
                    .fields
                    .into_iter()
                    .any(|f| self.filter_field_value(f, prop_key.clone()));
        }
        let found = self.ensure_fields(map, path, context.clone(), false);
        if let Some(err) = found.error {
            self.errors.push(err);
            return false;
        }
        if !found.fields.is_empty() {
            return found
                .fields
                .into_iter()
                .all(|f| self.filter_field_value(f, key.clone()));
        }
        if value.unbox().is_some() && value.scalar_bytes() == b"*" {
            return false;
        }
        let last = parts.last().unwrap().read().scalar_bytes();
        let default: Option<&[u8]> = match last.as_slice() {
            b"shape" => Some(b"rectangle"),
            b"border-radius" | b"stroke-dash" => Some(b"0"),
            b"opacity" => Some(b"1"),
            b"stroke-width" => Some(b"2"),
            b"icon" | b"tooltip" | b"link" => Some(b""),
            b"shadow" | b"multiple" | b"3d" | b"animated" | b"filled" => Some(b"false"),
            _ => None,
        };
        if let Some(default) = default {
            return self.filter_scalar(Some(default.to_vec()), None, key);
        }
        if last == b"label" {
            let node = self.arena.parent(map).unwrap();
            let Some(label) = self.filter_label(node) else {
                return false;
            };
            return self.filter_scalar(Some(label), None, key);
        }
        if matches!(last.as_slice(), b"src" | b"dst") {
            let Some(edge) = self.arena.parent_edge(map) else {
                return false;
            };
            let mut parts: Vec<Vec<u8>> = self
                .filter_endpoint(edge, last == b"src")
                .iter()
                .map(|s| s.scalar_bytes())
                .collect();
            let mut parent = self.arena.parent_field(edge);
            let mut prefix = Vec::new();
            while let Some(p) = parent {
                let Node::Field(f) = self.arena.node(p) else {
                    unreachable!()
                };
                if f.name.scalar_bytes() == b"root" {
                    break;
                }
                prefix.insert(0, f.name.scalar_bytes());
                parent = self.arena.parent_field(p);
            }
            if !prefix.is_empty() && !equal_fold(&parts[0], &prefix[0]) {
                prefix.extend(parts);
                parts = prefix;
            }
            return parts.join(&b'.') == value.scalar_bytes();
        }
        let parent = self.arena.parent(map).unwrap();
        matches!(self.arena.node(parent), Node::Field(_))
            && self.filter_property(&last, &value.scalar_bytes(), parent, key)
    }
    fn filter_endpoint(&self, edge: NodeId, src: bool) -> Vec<ast::StringBox> {
        let Node::Edge(e) = self.arena.node(edge) else {
            unreachable!()
        };
        let id = e.id.as_ref().unwrap().read();
        let path = if src { &id.src_path } else { &id.dst_path };
        path.iter()
            .flatten()
            .map(|s| ast::ValueBox::from_child(s.clone()).string_box())
            .collect()
    }
    fn filter_label(&self, node: NodeId) -> Option<Vec<u8>> {
        if let Some(primary) = self.arena.primary(node) {
            let Node::Scalar(s) = self.arena.node(primary) else {
                unreachable!()
            };
            Some(s.value.scalar_bytes())
        } else if let Node::Field(f) = self.arena.node(node) {
            Some(f.name.scalar_bytes())
        } else {
            None
        }
    }
    fn filter_property(
        &mut self,
        prop: &[u8],
        value: &[u8],
        node: NodeId,
        key: Shared<ast::Key>,
    ) -> bool {
        match prop {
            b"level" => {
                let level = std::str::from_utf8(value)
                    .ok()
                    .and_then(|v| v.parse::<i64>().ok());
                let Some(level) = level else {
                    self.errors.push(error(
                        &ast::Child::Key(key),
                        format!(
                            "&level must be a non-negative integer, got {}",
                            crate::parser::quote_input(value)
                        ),
                    ));
                    return false;
                };
                if level < 0 {
                    self.errors.push(error(
                        &ast::Child::Key(key),
                        format!("&level must be a non-negative integer, got {level}"),
                    ));
                    return false;
                }
                let mut actual = 0;
                let mut parent = self.arena.parent_field(node);
                while let Some(p) = parent {
                    let Node::Field(f) = self.arena.node(p) else {
                        unreachable!()
                    };
                    if f.name.scalar_bytes() == b"root" || !self.arena.board_kind(p).is_empty() {
                        break;
                    }
                    actual += 1;
                    parent = self.arena.parent_field(p);
                }
                actual == level
            }
            b"leaf" | b"connected" => {
                let expected = match value {
                    b"1" | b"t" | b"T" | b"TRUE" | b"true" | b"True" => true,
                    b"0" | b"f" | b"F" | b"FALSE" | b"false" | b"False" => false,
                    _ => {
                        self.errors.push(error(
                            &ast::Child::Key(key),
                            format!(
                                "&{} must be \"true\" or \"false\", got {}",
                                String::from_utf8_lossy(prop),
                                crate::parser::quote_input(value)
                            ),
                        ));
                        return false;
                    }
                };
                let actual = if prop == b"leaf" {
                    !self.is_container(self.arena.map(node))
                } else {
                    let Node::Field(f) = self.arena.node(node) else {
                        unreachable!()
                    };
                    f.references.iter().flatten().any(|r| r.read().in_edge())
                };
                actual == expected
            }
            b"label" => self.filter_scalar(self.filter_label(node), None, key),
            _ => false,
        }
    }
    fn filter_field_value(&mut self, field: NodeId, key: Shared<ast::Key>) -> bool {
        let Node::Field(f) = self.arena.node(field) else {
            unreachable!()
        };
        let composite = f.composite;
        let primary = f.primary.map(|p| {
            let Node::Scalar(s) = self.arena.node(p) else {
                unreachable!()
            };
            s.value.scalar_bytes()
        });
        self.filter_scalar(primary, composite, key)
    }
    fn filter_scalar(
        &mut self,
        primary: Option<Vec<u8>>,
        composite: Option<NodeId>,
        key: Shared<ast::Key>,
    ) -> bool {
        let value = key.read().value.scalar_box();
        if value.unbox().is_none() {
            self.errors.push(error(
                &ast::Child::Key(key),
                "glob filters cannot be composites",
            ));
            return false;
        }
        let bytes = value.scalar_bytes();
        if let Some(composite) = composite
            && let Node::Array(a) = self.arena.node(composite)
        {
            for v in a.values.iter().flatten() {
                if let Node::Scalar(s) = self.arena.node(*v)
                    && s.value.scalar_bytes() == bytes
                {
                    return true;
                }
            }
        }
        let Some(primary) = primary else { return false };
        if let Some(us) = value.unquoted_string
            && let Some(pattern) = &us.read().pattern
        {
            return match_pattern(
                &primary,
                &pattern.iter().map(|p| p.read().clone()).collect::<Vec<_>>(),
            );
        }
        primary == bytes
    }
}
