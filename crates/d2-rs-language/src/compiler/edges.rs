// SPDX-License-Identifier: MPL-2.0
//! Edge key compilation from pinned d2ir/compile.go, including bounded endpoint
//! expansion and the shared filtered-map/import paths.
use super::{Compiler, edge_creation::error};
use d2_rs_core::{
    ast::{self, Shared},
    ir::*,
};
impl Compiler {
    pub(super) fn ensure_edge_map(&mut self, edge: NodeId) -> NodeId {
        if let Some(map) = self.arena.map(edge) {
            return map;
        }
        let map = self.arena.insert(Some(edge), Node::Map(Map::default()));
        let Node::Edge(e) = self.arena.node_mut(edge) else {
            unreachable!()
        };
        e.map = Some(map);
        map
    }
    pub(super) fn compile_edges(&mut self, context: Shared<RefContext>) {
        let common = context.read().key.as_ref().unwrap().read().key.clone();
        let scope = context.read().scope_map.unwrap();
        let Some(common) = common else {
            self.compile_edges_inner(context);
            return;
        };
        let fields = self.ensure_fields(scope, common.clone(), context.clone(), true);
        if let Some(e) = fields.error {
            self.errors.push(e);
            return;
        }
        for field in fields.fields {
            if self.is_array_field(field) {
                self.errors.push(error(
                    &ast::Child::KeyPath(common.clone()),
                    "cannot index into array",
                ));
                return;
            }
            let map = self.ensure_map(field);
            let mut context = context.read().clone();
            context.scope_map = Some(map);
            self.compile_edges_inner(Shared::new(context))
        }
    }
    fn compile_edges_inner(&mut self, mut context: Shared<RefContext>) {
        let key = context.read().key.clone().unwrap();
        let ids = new_edge_ids(&key.read()).unwrap_or_default();
        for (i, id) in ids.into_iter().enumerate() {
            if self.stopped() {
                return;
            }
            let scope = context.read().scope_map.unwrap();
            let (primary, value, edge_key) = {
                let key = key.read();
                (key.primary.clone(), key.value.clone(), key.edge_key.clone())
            };
            let null = primary.null.is_some() || value.null.is_some();
            if !id.glob && null {
                self.arena.delete_edge(scope, Some(&id));
                continue;
            }
            let mut next = context.read().clone();
            next.edge = key
                .read()
                .edges
                .as_ref()
                .unwrap()
                .get(i)
                .unwrap()
                .read()
                .clone();
            context = Shared::new(next);
            let edge_ast = context.read().edge.clone().unwrap();
            let edges = if id.index.is_some() || id.glob {
                let edges = self.query_edges(scope, &id, &context);
                if self.stopped() {
                    return;
                }
                if edges.is_empty() {
                    if !id.glob {
                        self.errors.push(error(
                            &ast::Child::Edge(edge_ast.clone()),
                            "indexed edge does not exist",
                        ))
                    }
                    continue;
                }
                for edge in &edges {
                    if null {
                        let Node::Edge(e) = self.arena.node(*edge) else {
                            unreachable!()
                        };
                        let id = e.id.as_ref().unwrap().read().clone();
                        self.arena.delete_edge(scope, Some(&id));
                        continue;
                    }
                    if !self.filter_value_map(*edge, context.clone()) {
                        continue;
                    }
                    if let Some(suspension) =
                        primary.suspension.as_ref().or(value.suspension.as_ref())
                        && !self.lazy_glob_being_applied
                    {
                        if !self.filter_value_map(*edge, context.clone()) {
                            continue;
                        }
                        let suspended = suspension.read().value;
                        let Node::Edge(e) = self.arena.node_mut(*edge) else {
                            unreachable!()
                        };
                        e.suspended = suspended;
                        if !suspended {
                            self.unsuspend_endpoints(*edge, scope)
                        }
                    }
                    let reference = Shared::new(EdgeReference {
                        context: Some(context.clone()),
                        due_to_glob: !self.glob_ref_context_stack.is_empty(),
                        due_to_lazy_glob: self.lazy_glob_being_applied,
                    });
                    let Node::Edge(e) = self.arena.node_mut(*edge) else {
                        unreachable!()
                    };
                    e.append_reference(reference);
                    self.append_field_references(
                        scope,
                        0,
                        edge_ast.read().src.clone().unwrap(),
                        context.clone(),
                    );
                    self.append_field_references(
                        scope,
                        0,
                        edge_ast.read().dst.clone().unwrap(),
                        context.clone(),
                    );
                }
                edges
            } else {
                let (edges, err) = self.create_edges(scope, &id, &context);
                if self.stopped() {
                    return;
                }
                if let Some(err) = err {
                    self.errors.push(err);
                    continue;
                }
                edges
            };
            for edge in edges {
                if let Some(path) = &edge_key {
                    let map = self.ensure_edge_map(edge);
                    self.compile_field_path(map, path.clone(), context.clone());
                } else {
                    if primary.unbox().is_some() && primary.suspension.is_none() {
                        if self.ignore_lazy_glob(edge) {
                            return;
                        }
                        self.set_primary(edge, primary.clone())
                    }
                    if let Some(array) = &value.array {
                        self.errors.push(error(
                            &ast::Child::Array(array.clone()),
                            "edges cannot be assigned arrays",
                        ));
                        continue;
                    } else if let Some(map) = &value.map {
                        let dst = self.ensure_edge_map(edge);
                        self.map_ref_context_stack.push(context.clone());
                        self.compile_map(
                            dst,
                            map.clone(),
                            context.read().scope_ast.clone().unwrap(),
                        );
                        self.map_ref_context_stack.pop();
                    } else if value.scalar_box().unbox().is_some() && value.suspension.is_none() {
                        if self.ignore_lazy_glob(edge) {
                            return;
                        }
                        self.set_primary(edge, value.scalar_box())
                    }
                }
            }
        }
    }
    fn append_field_references(
        &mut self,
        map: NodeId,
        i: usize,
        path: Shared<ast::KeyPath>,
        context: Shared<RefContext>,
    ) {
        let head = path
            .read()
            .path
            .as_ref()
            .unwrap()
            .get(i)
            .unwrap()
            .read()
            .clone()
            .unwrap();
        let head = head.read();
        let Some(field) = self.arena.get_field(map, std::slice::from_ref(&*head)) else {
            return;
        };
        let reference = Shared::new(FieldReference {
            string: head.unbox(),
            key_path: Some(path.clone()),
            context: Some(context.clone()),
            due_to_glob: !self.glob_ref_context_stack.is_empty(),
            due_to_lazy_glob: self.lazy_glob_being_applied,
        });
        let Node::Field(f) = self.arena.node_mut(field) else {
            unreachable!()
        };
        f.append_reference(reference);
        if i + 1 < path.read().path.as_ref().unwrap().len()
            && let Some(map) = self.arena.map(field)
        {
            self.append_field_references(map, i + 1, path.clone(), context)
        }
    }
    fn unsuspend_endpoints(&mut self, edge: NodeId, scope: NodeId) {
        let Node::Edge(e) = self.arena.node(edge) else {
            unreachable!()
        };
        let id = e.id.as_ref().unwrap().read();
        let mut paths = [
            id.src_path.clone().unwrap_or_default(),
            id.dst_path.clone().unwrap_or_default(),
        ];
        drop(id);
        let mut container_path = Vec::new();
        let mut current = self.arena.parent_field(edge);
        while let Some(field) = current {
            let Node::Field(f) = self.arena.node(field) else {
                unreachable!()
            };
            if f.name.scalar_bytes() == b"root" {
                break;
            }
            container_path.insert(0, f.name.unbox());
            current = self.arena.parent_field(field);
        }
        let raw = |p: &Option<ast::Child>| ast::ScalarBox::from_child(p.clone()).scalar_bytes();
        let root = self.arena.root_map(scope);
        for path in &mut paths {
            if !path.is_empty()
                && !container_path.is_empty()
                && !equal_fold(&raw(&path[0]), &raw(&container_path[0]))
            {
                let mut absolute = container_path.clone();
                absolute.append(path);
                *path = absolute
            }
            let boxes: Vec<_> = path
                .iter()
                .map(|p| ast::ValueBox::from_child(p.clone()).string_box())
                .collect();
            let Some(field) = self.arena.get_field_indexed(root, &boxes) else {
                continue;
            };
            let mut current = Some(field);
            while let Some(node) = current {
                let Node::Field(f) = self.arena.node_mut(node) else {
                    unreachable!()
                };
                if node != field && f.name.scalar_bytes() == b"root" {
                    break;
                }
                f.suspended = false;
                current = self.arena.parent_field(node);
            }
        }
    }
}
