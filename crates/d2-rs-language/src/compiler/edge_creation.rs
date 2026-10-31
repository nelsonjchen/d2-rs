// SPDX-License-Identifier: MPL-2.0
//! Compiler-aware edge expansion and indexing from d2ir/d2ir.go.
use super::Compiler;
use d2_rs_core::{
    ast::{self, Node as AstNode, Shared, keywords::*},
    ir::*,
};
pub(super) fn error(node: &ast::Child, message: impl Into<String>) -> ast::Error {
    let range = node.range();
    ast::Error {
        message: format!("{range}: {}", message.into()),
        range,
    }
}
fn scalar(node: &Option<ast::Child>) -> Vec<u8> {
    ast::ScalarBox::from_child(node.clone()).scalar_bytes()
}
fn edge_ast(context: &Shared<RefContext>) -> Shared<ast::Edge> {
    context.read().edge.clone().unwrap()
}
fn edge_node(context: &Shared<RefContext>) -> ast::Child {
    ast::Child::Edge(edge_ast(context))
}
impl Compiler {
    fn common_edge_path(
        &self,
        common: Vec<Option<ast::Child>>,
        context: &Shared<RefContext>,
    ) -> Shared<ast::KeyPath> {
        let path = ast::KeyPath::from_strings(common.iter().map(scalar));
        let source = edge_ast(context).read().src.clone().unwrap();
        let source = source.read();
        let source = source.path.as_ref().unwrap();
        let mut last = 0;
        for slot in path.path.as_ref().unwrap().iter() {
            let value = slot.read().as_ref().unwrap().read().scalar_bytes();
            // Updating last affects the next outer iteration, not this loop.
            let start = last;
            for j in start..source.len() {
                let real = source.get(j).unwrap().read().clone().unwrap();
                if value == real.read().scalar_bytes() {
                    *slot.write() = Some(real.clone());
                    last += j + 1;
                }
            }
        }
        Shared::new(path)
    }
    pub(super) fn is_array_field(&self, field: NodeId) -> bool {
        matches!(self.arena.node(field),Node::Field(f) if f.composite.is_some_and(|n|matches!(self.arena.node(n),Node::Array(_))))
    }
    pub(super) fn is_container(&mut self, map: Option<NodeId>) -> bool {
        let Some(map) = map else { return false };
        let parent = self.arena.parent(map).unwrap();
        let Node::Field(f) = self.arena.node(parent) else {
            panic!("container map parent is not a field")
        };
        let f = f.clone();
        for reference in f.references.iter().flatten() {
            let reference = reference.read();
            if !reference.primary() {
                continue;
            }
            let context = reference.context.as_ref().unwrap().read();
            let Some(key) = &context.key else { continue };
            let map = key.read().value.map.clone();
            if let Some(map) = map {
                for node in map.read().children() {
                    if let ast::Child::Import(import) = &node
                        && let Some(imported) = self.peek_import(import.clone())
                    {
                        let Node::Map(m) = self.arena.node(imported) else {
                            unreachable!()
                        };
                        if m.fields.iter().flatten().any(|id| {
                            let Node::Field(f) = self.arena.node(*id) else {
                                unreachable!()
                            };
                            !(std::str::from_utf8(&f.name.scalar_bytes())
                                .is_ok_and(is_reserved_keyword)
                                && f.name.unquoted_string.is_some())
                        }) {
                            return true;
                        }
                    }
                    if let ast::Child::Key(key) = node {
                        let key = key.read();
                        if key.edges.as_ref().is_some_and(|e| !e.is_empty()) {
                            return true;
                        }
                        if let Some(path) = &key.key {
                            let first = path
                                .read()
                                .path
                                .as_ref()
                                .unwrap()
                                .get(0)
                                .unwrap()
                                .read()
                                .clone()
                                .unwrap();
                            let bytes = first.read().scalar_bytes();
                            let reserved =
                                std::str::from_utf8(&bytes).is_ok_and(is_reserved_keyword);
                            if !(reserved && f.name.unquoted_string.is_some()) {
                                return true;
                            }
                        }
                    }
                }
            }
        }
        let Node::Map(m) = self.arena.node(map) else {
            unreachable!()
        };
        m.fields.iter().flatten().any(|f| {
            let Node::Field(f) = self.arena.node(*f) else {
                unreachable!()
            };
            let bytes = f.name.scalar_bytes();
            !(std::str::from_utf8(&bytes).is_ok_and(is_reserved_keyword)
                && f.name.unquoted_string.is_some())
        })
    }
    pub(super) fn create_edges(
        &mut self,
        map: NodeId,
        id: &EdgeId,
        context: &Shared<RefContext>,
    ) -> (Vec<NodeId>, Option<ast::Error>) {
        let glob = context
            .read()
            .key
            .as_ref()
            .unwrap()
            .read()
            .has_glob()
            .then(|| self.ensure_glob_context(context.clone()));
        let mut edges = Vec::new();
        let err = self.create_edge(map, id, context, glob, &mut edges).err();
        if !self.stopped() && !edges.is_empty() && self.glob_ref_context_stack.is_empty() {
            for g in self.glob_contexts().to_vec() {
                let context = self.arena.glob(g).context.clone().unwrap();
                let old = self.lazy_glob_being_applied;
                self.lazy_glob_being_applied = true;
                self.compile_key(context);
                self.lazy_glob_being_applied = old;
            }
        }
        (edges, err)
    }
    fn create_edge(
        &mut self,
        map: NodeId,
        id: &EdgeId,
        context: &Shared<RefContext>,
        glob: Option<GlobId>,
        out: &mut Vec<NodeId>,
    ) -> Result<(), ast::Error> {
        if self.arena.parent_edge(map).is_some() {
            return Err(error(&edge_node(context), "cannot create edge inside edge"));
        }
        let resolved = id
            .resolve(&self.arena, map)
            .map_err(|e| error(&edge_node(context), e.to_string()))?;
        let map = resolved.map;
        let id = resolved.id;
        if let Some(common) = resolved.common {
            let path = self.common_edge_path(common, context);
            let fields = self.ensure_fields_mode(map, path, None, true, true);
            if let Some(e) = fields.error {
                return Err(e);
            }
            for field in fields.fields {
                if self.is_array_field(field) {
                    return Err(error(
                        &ast::Child::KeyPath(edge_ast(context).read().src.clone().unwrap()),
                        "cannot index into array",
                    ));
                }
                let map = self.ensure_map(field);
                self.create_edge(map, &id, context, glob, out)?
            }
            return Ok(());
        }
        for (path, source) in [(&id.src_path, true), (&id.dst_path, false)] {
            let path = path.as_ref().unwrap();
            let original = if source {
                edge_ast(context).read().src.clone().unwrap()
            } else {
                edge_ast(context).read().dst.clone().unwrap()
            };
            let at = |i| {
                original
                    .read()
                    .path
                    .as_ref()
                    .unwrap()
                    .get(i)
                    .unwrap()
                    .read()
                    .as_ref()
                    .unwrap()
                    .read()
                    .unbox()
                    .unwrap()
            };
            if let Some(i) = path.iter().position(|p| {
                matches!(p, Some(ast::Child::Unquoted(_))) && {
                    let text = scalar(p);
                    SIMPLE_RESERVED_KEYWORDS
                        .iter()
                        .chain(RESERVED_KEYWORD_HOLDERS)
                        .any(|k| k.as_bytes() == text)
                }
            }) {
                return Err(error(&at(i), "reserved keywords are prohibited in edges"));
            }
            if let Some(i) = path.iter().position(|p| {
                matches!(p, Some(ast::Child::Unquoted(_))) && {
                    let text = go_lowercase(&scalar(p));
                    BOARD_KEYWORDS.iter().any(|k| k.as_bytes() == text)
                }
            }) && i + 1 == path.len()
            {
                return Err(error(
                    &at(i),
                    "edge with board keyword alone doesn't make sense",
                ));
            }
        }
        let edge = edge_ast(context);
        let src = edge.read().src.clone().unwrap();
        let dst = edge.read().dst.clone().unwrap();
        let scope = context.read().scope_map.unwrap();
        let sources = self.ensure_fields(scope, src.clone(), context.clone(), true);
        if let Some(e) = sources.error {
            return Err(e);
        }
        let dests = self.ensure_fields(scope, dst.clone(), context.clone(), true);
        if let Some(e) = dests.error {
            return Err(e);
        }
        for source in sources.fields {
            for dest in &dests.fields {
                if self.stopped()
                    || ((src.read().has_glob() || dst.read().has_glob())
                        && !self.reserve_edge_expansion(&edge, glob, source, *dest, false))
                {
                    return Ok(());
                }
                if source == *dest && (src.read().has_glob() || dst.read().has_glob()) {
                    continue;
                }
                if src.read().has_multi_glob()
                    && (self.is_container(self.arena.map(source))
                        || !self.arena.board_kind(source).is_empty()
                        || self.arena.parent_board(source) != self.arena.parent_board(*dest))
                {
                    continue;
                }
                if dst.read().has_multi_glob()
                    && (self.is_container(self.arena.map(*dest))
                        || !self.arena.board_kind(*dest).is_empty()
                        || self.arena.parent_board(source) != self.arena.parent_board(*dest))
                {
                    continue;
                }
                let mut next = id.clone();
                next.src_path = crate::ir::relative_ida(&self.arena, Some(map), source);
                next.dst_path = crate::ir::relative_ida(&self.arena, Some(map), *dest);
                out.extend(self.create_edge_pair(map, &next, context, glob, source, *dest)?);
            }
        }
        Ok(())
    }
    fn create_edge_pair(
        &mut self,
        map: NodeId,
        id: &EdgeId,
        context: &Shared<RefContext>,
        glob: Option<GlobId>,
        source: NodeId,
        dest: NodeId,
    ) -> Result<Vec<NodeId>, ast::Error> {
        for (node, src) in [(source, true), (dest, false)] {
            if !self.arena.board_kind(node).is_empty() {
                let edge = edge_ast(context);
                let path = if src {
                    edge.read().src.clone().unwrap()
                } else {
                    edge.read().dst.clone().unwrap()
                };
                return Err(error(
                    &ast::Child::KeyPath(path),
                    "cannot create edges between boards",
                ));
            }
        }
        if self.arena.parent_board(source) != self.arena.parent_board(dest) {
            return Err(error(
                &edge_node(context),
                "cannot create edges between boards",
            ));
        }
        let resolved = id
            .resolve(&self.arena, map)
            .map_err(|e| error(&edge_node(context), e.to_string()))?;
        let map = resolved.map;
        let mut id = resolved.id;
        if let Some(common) = resolved.common {
            let path = self.common_edge_path(common, context);
            let fields = self.ensure_fields_mode(map, path, None, true, true);
            if let Some(e) = fields.error {
                return Err(e);
            }
            let mut edges = Vec::new();
            for field in fields.fields {
                if self.is_array_field(field) {
                    return Err(error(
                        &ast::Child::KeyPath(edge_ast(context).read().src.clone().unwrap()),
                        "cannot index into array",
                    ));
                }
                let map = self.ensure_map(field);
                edges.extend(self.create_edge_pair(map, &id, context, glob, source, dest)?)
            }
            return Ok(edges);
        }
        id.index = None;
        id.glob = true;
        let index = self
            .arena
            .get_edges_indexed(map, &id)
            .map_or(0, |e| e.len());
        id.index = Some(Shared::new(index as i64));
        id.glob = false;
        let edge = self.arena.insert(
            Some(map),
            Node::Edge(Edge {
                id: Some(Shared::new(id)),
                references: Some(vec![Shared::new(EdgeReference {
                    context: Some(context.clone()),
                    due_to_glob: !self.glob_ref_context_stack.is_empty(),
                    due_to_lazy_glob: self.lazy_glob_being_applied,
                })]),
                ..Default::default()
            }),
        );
        if let Some(glob) = glob {
            let copy = self.arena.copy(edge, Some(map));
            let Node::Edge(e) = self.arena.node(copy) else {
                unreachable!()
            };
            e.id.as_ref().unwrap().write().index = None;
            let key = self.path_key(copy, context);
            let set = self.arena.glob(glob).applied_edges.as_ref().unwrap();
            if !set.write().insert(key) {
                return Ok(Vec::new());
            }
        }
        self.arena.append_edge(map, edge);
        Ok(vec![edge])
    }
    pub(super) fn query_edges(
        &mut self,
        map: NodeId,
        id: &EdgeId,
        context: &Shared<RefContext>,
    ) -> Vec<NodeId> {
        let glob = context
            .read()
            .key
            .as_ref()
            .unwrap()
            .read()
            .has_glob()
            .then(|| self.ensure_glob_context(context.clone()));
        let mut out = Vec::new();
        let _ = self.query_edge(map, id, context, glob, &mut out);
        out
    }
    fn query_edge(
        &mut self,
        map: NodeId,
        id: &EdgeId,
        context: &Shared<RefContext>,
        glob: Option<GlobId>,
        out: &mut Vec<NodeId>,
    ) -> Result<(), ast::Error> {
        let resolved = id
            .resolve(&self.arena, map)
            .map_err(|e| error(&edge_node(context), e.to_string()))?;
        let map = resolved.map;
        let id = resolved.id;
        if let Some(common) = resolved.common {
            let path = self.common_edge_path(common, context);
            let fields = self.ensure_fields_mode(map, path, None, false, false);
            if fields.error.is_some() {
                return Ok(());
            }
            for field in fields.fields {
                if self.is_array_field(field) {
                    return Err(error(
                        &ast::Child::KeyPath(edge_ast(context).read().src.clone().unwrap()),
                        "cannot index into array",
                    ));
                }
                let map = self.ensure_map(field);
                self.query_edge(map, &id, context, glob, out)?
            }
            return Ok(());
        }
        let edge = edge_ast(context);
        let scope = context.read().scope_map.unwrap();
        let src =
            self.ensure_fields_mode(scope, edge.read().src.clone().unwrap(), None, false, false);
        if let Some(e) = src.error {
            return Err(e);
        }
        let dst =
            self.ensure_fields_mode(scope, edge.read().dst.clone().unwrap(), None, false, false);
        if let Some(e) = dst.error {
            return Err(e);
        }
        for src in src.fields {
            for dst in &dst.fields {
                if self.stopped()
                    || ((edge.read().src.as_ref().unwrap().read().has_glob()
                        || edge.read().dst.as_ref().unwrap().read().has_glob())
                        && !self.reserve_edge_expansion(&edge, glob, src, *dst, true))
                {
                    return Ok(());
                }
                let mut id = id.clone();
                id.src_path = crate::ir::relative_ida(&self.arena, Some(map), src);
                id.dst_path = crate::ir::relative_ida(&self.arena, Some(map), *dst);
                for edge in self.arena.get_edges_indexed(map, &id).into_iter().flatten() {
                    if let Some(glob) = glob {
                        let key = self.path_key(edge, context);
                        if !self
                            .arena
                            .glob(glob)
                            .applied_edges
                            .as_ref()
                            .unwrap()
                            .write()
                            .insert(key)
                        {
                            continue;
                        }
                    }
                    out.push(edge)
                }
            }
        }
        Ok(())
    }
}
