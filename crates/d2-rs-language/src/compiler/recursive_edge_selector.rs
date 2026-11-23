// SPDX-License-Identifier: MPL-2.0
//! d2ir/recursive_edge_selector.go: select real connections, with bounded planning.
use super::Compiler;
use d2_rs_core::{
    ast::{
        self, Shared,
        keywords::{BOARD_KEYWORDS, is_reserved_keyword},
    },
    ir::*,
};
use std::collections::HashMap;

pub(super) fn same_recursive_endpoints(context: &Shared<RefContext>) -> bool {
    let edge = context.read().edge.clone().unwrap();
    let edge = edge.read();
    let src = edge.src.as_ref().unwrap().read();
    let dst = edge.dst.as_ref().unwrap().read();
    src.path.as_ref().is_some_and(|p| p.len() == 1)
        && dst.path.as_ref().is_some_and(|p| p.len() == 1)
        && src.has_multi_glob()
        && dst.has_multi_glob()
        && src
            .path
            .as_ref()
            .unwrap()
            .get(0)
            .unwrap()
            .read()
            .as_ref()
            .unwrap()
            .read()
            .scalar_bytes()
            == dst
                .path
                .as_ref()
                .unwrap()
                .get(0)
                .unwrap()
                .read()
                .as_ref()
                .unwrap()
                .read()
                .scalar_bytes()
}
fn skip_field(arena: &Arena, field: NodeId, triple: bool) -> bool {
    let Node::Field(f) = arena.node(field) else {
        return true;
    };
    if f.name.unbox().is_none() {
        return true;
    }
    let name = f.name.scalar_string();
    f.name.unquoted_string.is_some()
        && is_reserved_keyword(&name)
        && if triple {
            !BOARD_KEYWORDS.contains(&name.as_str())
        } else {
            BOARD_KEYWORDS.contains(&name.as_str()) || name == "classes" || name == "vars"
        }
}
impl Compiler {
    fn recursive_glob_contains(&self, map: NodeId, field: Option<NodeId>, triple: bool) -> bool {
        let Some(field) = field else { return false };
        let Node::Field(f) = self.arena.node(field) else {
            return false;
        };
        if f.name.unbox().is_none()
            || !self.arena.board_kind(field).is_empty()
            || (f.name.unquoted_string.is_some() && is_reserved_keyword(&f.name.scalar_string()))
        {
            return false;
        }
        self.arena
            .path_to_field(map, Some(field))
            .is_some_and(|path| {
                !path.is_empty()
                    && path
                        .into_iter()
                        .all(|f| !skip_field(&self.arena, f, triple))
            })
    }
    pub(super) fn select_recursive_edges(
        &mut self,
        map: NodeId,
        id: &EdgeId,
        context: &Shared<RefContext>,
        glob: GlobId,
    ) -> Vec<NodeId> {
        if !self.arena.may_have_edges(map) {
            return Vec::new();
        }
        let edge = context.read().edge.clone().unwrap();
        let src = edge.read().src.clone().unwrap();
        let triple = src.read().has_triple_glob();
        let pattern: Vec<ast::ByteString> = if triple {
            vec!["*", "", "*", "", "*"]
        } else {
            vec!["*", "", "*"]
        }
        .into_iter()
        .map(Into::into)
        .collect();
        let source = ast::Child::Edge(edge.clone());
        let mut scopes = vec![map];
        let mut edge_count = 0;
        let mut i = 0;
        while i < scopes.len() {
            let scope = scopes[i];
            let Node::Map(m) = self.arena.node(scope) else {
                unreachable!()
            };
            edge_count += m.edges.as_ref().map_or(0, Vec::len);
            let mut field = m.first_edge_field;
            while let Some(f) = field {
                if !self.reserve_glob_work(Some(&source), 1) {
                    return Vec::new();
                }
                let Node::Field(value) = self.arena.node(f) else {
                    unreachable!()
                };
                field = value.next_edge_field;
                if skip_field(&self.arena, f, triple) || value.name.scalar_bytes().contains(&b'*') {
                    continue;
                }
                if let Some(child) = self.arena.map(f).filter(|m| self.arena.may_have_edges(*m)) {
                    scopes.push(child);
                }
            }
            i += 1;
        }
        if edge_count > 1 {
            // Planning is bounded independently of, and does not charge, the glob budget.
            let mut visits = 0;
            let mut complete = true;
            let mut count = 0;
            let mut cancellation = None;
            let context_check = &mut self.context;
            let (fields, _) = self
                .arena
                .multi_glob_with_visit(map, &pattern, &mut |field| {
                    visits += 1;
                    if visits > edge_count || (count != 0 && count > edge_count / count) {
                        complete = false;
                        return false;
                    }
                    if let Some(error) = context_check.as_mut().and_then(|check| check()) {
                        cancellation = Some(error);
                        complete = false;
                        return false;
                    }
                    // The traversal appends a nonreserved, nonboard field after visiting it.
                    let Node::Field(f) = self.arena.node(field) else {
                        unreachable!()
                    };
                    if f.name.unbox().is_some()
                        && self.arena.board_kind(field).is_empty()
                        && !(f.name.unquoted_string.is_some()
                            && is_reserved_keyword(&f.name.scalar_string()))
                    {
                        count += 1;
                    }
                    true
                });
            if let Some(error) = cancellation {
                self.context_error = Some(error);
                self.halted = true;
                return Vec::new();
            }
            let fields = fields.unwrap_or_default();
            let n = fields.len();
            if complete && (n == 0 || n <= edge_count / n) {
                let mut out = Vec::new();
                self.query_edges_between(map, id, context, Some(glob), &fields, &fields, &mut out);
                return out;
            }
        }
        let mut candidates = Vec::new();
        for scope in scopes {
            let Node::Map(m) = self.arena.node(scope) else {
                unreachable!()
            };
            let edges = m.edges.clone().unwrap_or_default();
            for e in edges {
                if !self.reserve_edge_work(&edge) {
                    return Vec::new();
                }
                let Node::Edge(value) = self.arena.node(e) else {
                    unreachable!()
                };
                let eid = value.id.as_ref().unwrap().read().clone();
                let direct = id.src_arrow == eid.src_arrow && id.dst_arrow == eid.dst_arrow;
                let reverse = id.src_arrow == eid.dst_arrow && id.dst_arrow == eid.src_arrow;
                if !direct && !reverse {
                    continue;
                }
                let key = self.path_key(e, context);
                if self
                    .arena
                    .glob(glob)
                    .applied_edges
                    .as_ref()
                    .unwrap()
                    .read()
                    .contains(&key)
                {
                    continue;
                }
                let path = |p: &Option<Vec<Option<ast::Child>>>| {
                    p.iter()
                        .flatten()
                        .map(|c| ast::ValueBox::from_child(c.clone()).string_box())
                        .collect::<Vec<_>>()
                };
                let mut src = self.arena.get_field_indexed(scope, &path(&eid.src_path));
                let mut dst = self.arena.get_field_indexed(scope, &path(&eid.dst_path));
                if !direct {
                    std::mem::swap(&mut src, &mut dst);
                }
                if !self.recursive_glob_contains(map, src, triple)
                    || !self.recursive_glob_contains(map, dst, triple)
                {
                    continue;
                }
                let (src, dst) = (src.unwrap(), dst.unwrap());
                if !self.reserve_edge_pair(&edge, Some(glob), src, dst, true) {
                    return Vec::new();
                }
                candidates.push((e, src, dst, key));
            }
        }
        if candidates.len() > 1 {
            let mut failure = None;
            let mut cancellation = None;
            let context_check = &mut self.context;
            let budget = &mut self.glob_expansion;
            let (fields, _) = self.arena.multi_glob_with_visit(map, &pattern, &mut |_| {
                if let Some(error) = context_check.as_mut().and_then(|check| check()) {
                    cancellation = Some(error);
                    return false;
                }
                if let Err(message) = budget.reserve(1) {
                    failure = Some(message);
                    false
                } else {
                    true
                }
            });
            if let Some(error) = cancellation {
                self.context_error = Some(error);
                self.halted = true;
                return Vec::new();
            }
            if let Some(message) = failure {
                self.glob_failure(Some(&source), message);
                return Vec::new();
            }
            let order: HashMap<_, _> = fields
                .unwrap_or_default()
                .into_iter()
                .enumerate()
                .map(|(i, f)| (f, i))
                .collect();
            if id.src_arrow == id.dst_arrow {
                for (_, src, dst, _) in &mut candidates {
                    if order[dst] < order[src] {
                        std::mem::swap(src, dst);
                    }
                }
            }
            candidates.sort_by_key(|(_, src, dst, _)| (order[src], order[dst]));
        }
        let applied = self.arena.glob(glob).applied_edges.as_ref().unwrap();
        candidates
            .into_iter()
            .filter_map(|(edge, _, _, key)| applied.write().insert(key).then_some(edge))
            .collect()
    }
}
