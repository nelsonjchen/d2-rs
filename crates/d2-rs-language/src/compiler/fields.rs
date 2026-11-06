// SPDX-License-Identifier: MPL-2.0
//! Field construction with explicit traversal frames from d2ir/d2ir.go. Queue scheduling in
//! ensureFieldMode is separate; this returns both selected and created fields.
use super::Compiler;
use crate::{format, ir};
use d2_rs_core::ast::Node as AstNode;
use d2_rs_core::{
    ast::{self, Shared, keywords::*},
    ir::*,
};

pub struct FieldQuery {
    pub path: Shared<ast::KeyPath>,
    pub context: Option<Shared<RefContext>>,
    pub create: bool,
    pub glob: Option<GlobId>,
    pub indexed: bool,
}
#[derive(Default)]
pub struct FieldQueryResult {
    pub fields: Vec<NodeId>,
    pub created: Vec<NodeId>,
    pub error: Option<ast::Error>,
}
// A continuation is retained for each glob sibling walk and Go defer. In
// particular, filtering a sibling must happen after its predecessor's subtree.
enum FieldFrame {
    Enter(NodeId, usize),
    Candidates {
        fields: std::vec::IntoIter<NodeId>,
        index: usize,
        pattern: Option<Vec<ast::ByteString>>,
        source: Option<ast::Child>,
    },
    Applied(NodeId, usize),
}
impl Compiler {
    /// Execute the recursive ensureField primitive, preserving partial effects
    /// on error. The caller handles leading underscores and lazy scheduling.
    pub fn construct_fields(
        &mut self,
        map: NodeId,
        start: usize,
        query: &FieldQuery,
    ) -> FieldQueryResult {
        let mut result = FieldQueryResult::default();
        result.error = self.ensure_field(map, start, query, &mut result).err();
        result
    }
    pub(super) fn path_key(&self, field: NodeId, context: &Shared<RefContext>) -> Vec<u8> {
        let triple = context
            .read()
            .key
            .as_ref()
            .expect("nil key")
            .read()
            .has_triple_glob();
        let path = if triple {
            ir::ida(&self.arena, field)
        } else {
            ir::board_ida(&self.arena, field)
        };
        let parts: Vec<_> = path
            .into_iter()
            .flatten()
            .map(|p| ast::raw_string_box(ast::ScalarBox::from_child(p).scalar_bytes(), true))
            .collect();
        format::format_key_path(&parts)
    }
    fn filter_field(&mut self, field: NodeId, passthrough: bool, q: &FieldQuery) -> bool {
        let Some(glob) = q.glob else {
            return true;
        };
        let context = q.context.as_ref().expect("nil glob reference");
        let key_string = self.path_key(field, context);
        let has_glob = q.path.read().has_glob();
        if has_glob {
            let context = context.read();
            let key = context.key.as_ref().expect("nil key").read();
            let path = q.path.read();
            let last = path
                .path
                .as_ref()
                .unwrap()
                .get(path.path.as_ref().unwrap().len() - 1)
                .unwrap()
                .read()
                .clone()
                .unwrap();
            let last = last.read();
            let terminal_pattern = last
                .unquoted_string
                .as_ref()
                .is_some_and(|s| s.read().pattern.as_ref().is_some_and(|p| !p.is_empty()));
            let same_path =
                |p: &Option<Shared<ast::KeyPath>>| p.as_ref().is_some_and(|p| p.ptr_eq(&q.path));
            if (key.edges.as_ref().is_none_or(|e| e.is_empty())
                || !terminal_pattern
                || same_path(&key.key)
                || same_path(&key.edge_key))
                && self
                    .arena
                    .glob(glob)
                    .applied_fields
                    .as_ref()
                    .is_some_and(|set| set.read().contains(&key_string))
            {
                return false;
            }
        }
        if !passthrough {
            self.arena
                .glob_mut(glob)
                .applied_fields
                .as_mut()
                .expect("nil applied field map")
                .write()
                .insert(key_string);
        }
        true
    }
    fn select_fields(
        &mut self,
        fields: impl IntoIterator<Item = NodeId>,
        q: &FieldQuery,
        result: &mut FieldQueryResult,
    ) {
        for field in fields {
            if self.filter_field(field, false, q) {
                result.fields.push(field);
            }
        }
    }
    pub(super) fn ensure_map(&mut self, field: NodeId) -> NodeId {
        if let Some(map) = self.arena.map(field) {
            return map;
        }
        let map = self.arena.insert(Some(field), Node::Map(Map::default()));
        let Node::Field(f) = self.arena.node_mut(field) else {
            panic!("expected field");
        };
        f.composite = Some(map);
        map
    }
    fn add_reference(&mut self, field: NodeId, string: Option<ast::Child>, q: &FieldQuery) {
        let Some(context) = &q.context else {
            return;
        };
        let Node::Field(f) = self.arena.node_mut(field) else {
            panic!("expected field");
        };
        f.append_reference(Shared::new(FieldReference {
            string,
            key_path: Some(q.path.clone()),
            context: Some(context.clone()),
            due_to_glob: !self.glob_ref_context_stack.is_empty(),
            due_to_lazy_glob: self.lazy_glob_being_applied,
        }));
    }
    fn parent_shape(&self, mut node: NodeId) -> Vec<u8> {
        loop {
            if matches!(self.arena.node(node), Node::Field(_))
                && let Some(map) = self.arena.map(node)
                && let Some(shape) = self
                    .arena
                    .get_field(map, &[ast::raw_string_box("shape", true)])
                && let Some(primary) = self.arena.primary(shape)
                && let Node::Scalar(s) = self.arena.node(primary)
            {
                return s.value.scalar_bytes();
            }
            let Some(parent) = self.arena.parent(node) else {
                return Vec::new();
            };
            node = parent;
        }
    }
    fn ensure_field(
        &mut self,
        map: NodeId,
        i: usize,
        q: &FieldQuery,
        result: &mut FieldQueryResult,
    ) -> Result<(), ast::Error> {
        let mut frames = Vec::new();
        let mut outcome = self.ensure_field_step(map, i, q, result, &mut frames);
        while let Some(frame) = frames.pop() {
            match frame {
                FieldFrame::Applied(field, index) => {
                    // Go's defers run on errors too, innermost first.
                    if index as isize >= q.path.read().first_glob() {
                        for context in self.glob_ref_context_stack.clone() {
                            let key = self.path_key(field, &context);
                            let glob = self
                                .get_glob_context(&context)
                                .expect("missing deferred glob context");
                            self.arena
                                .glob_mut(glob)
                                .applied_fields
                                .as_mut()
                                .expect("nil applied field map")
                                .write()
                                .insert(key);
                        }
                    }
                }
                _ if outcome.is_err() => {}
                FieldFrame::Enter(map, index) => {
                    outcome = self.ensure_field_step(map, index, q, result, &mut frames);
                }
                FieldFrame::Candidates {
                    mut fields,
                    index,
                    pattern,
                    source,
                } => {
                    while let Some(field) = fields.next() {
                        if let Some(pattern) = &pattern {
                            if q.glob.is_some() && !self.reserve_glob_work(source.as_ref(), 1) {
                                break;
                            }
                            let Node::Field(f) = self.arena.node(field) else {
                                panic!("expected field");
                            };
                            if f.name.unbox().is_none()
                                || !match_pattern(&f.name.scalar_bytes(), pattern)
                            {
                                continue;
                            }
                        }
                        if index + 1 == q.path.read().path.as_ref().unwrap().len() {
                            self.select_fields([field], q, result);
                        } else if self.filter_field(field, true, q) {
                            let child = self.ensure_map(field);
                            frames.push(FieldFrame::Candidates {
                                fields,
                                index,
                                pattern,
                                source,
                            });
                            frames.push(FieldFrame::Enter(child, index + 1));
                            break;
                        }
                    }
                }
            }
        }
        outcome
    }
    fn ensure_field_step(
        &mut self,
        map: NodeId,
        i: usize,
        q: &FieldQuery,
        result: &mut FieldQueryResult,
        frames: &mut Vec<FieldFrame>,
    ) -> Result<(), ast::Error> {
        if self.stopped() {
            return Ok(());
        }
        let path_len = q.path.read().path.as_ref().unwrap().len();
        let head = q
            .path
            .read()
            .path
            .as_ref()
            .unwrap()
            .get(i)
            .unwrap()
            .read()
            .clone()
            .unwrap();
        let head = head.read().clone();
        let pattern = head.unquoted_string.as_ref().and_then(|s| {
            s.read()
                .pattern
                .as_ref()
                .map(|p| p.iter().map(|s| s.read().clone()).collect::<Vec<_>>())
        });
        if let Some(pattern) = pattern {
            let source = self.glob_source(q.context.as_ref());
            let mut failure = None;
            let budget = &mut self.glob_expansion;
            let mut visit = |_| {
                if q.glob.is_none() {
                    return true;
                }
                match budget.reserve(1) {
                    Ok(()) => true,
                    Err(message) => {
                        failure = Some(message);
                        false
                    }
                }
            };
            let (fields, multi) = if let Some(target) = self.lazy_glob_target
                && q.glob.is_some()
                && (ast::is_double_glob(&pattern) || ast::is_triple_glob(&pattern))
            {
                (
                    self.arena.multi_glob_matches_toward_with_visit(
                        map,
                        Some(target),
                        &pattern,
                        &mut visit,
                    ),
                    true,
                )
            } else {
                self.arena.multi_glob_with_visit(map, &pattern, &mut visit)
            };
            if let Some(message) = failure {
                self.glob_failure(source.as_ref(), message);
                return Ok(());
            }
            if multi {
                if i + 1 == path_len {
                    self.select_fields(fields.into_iter().flatten(), q, result);
                } else {
                    frames.push(FieldFrame::Candidates {
                        fields: fields.into_iter().flatten().collect::<Vec<_>>().into_iter(),
                        index: i,
                        pattern: None,
                        source: None,
                    });
                }
                return Ok(());
            }
            let fields = if let Some(target) = self.lazy_glob_target
                && q.glob.is_some()
            {
                self.arena
                    .direct_child_toward(map, Some(target))
                    .into_iter()
                    .collect()
            } else {
                let Node::Map(m) = self.arena.node(map) else {
                    panic!("expected map");
                };
                m.fields.clone().unwrap_or_default()
            };
            frames.push(FieldFrame::Candidates {
                fields: fields.into_iter(),
                index: i,
                pattern: Some(pattern),
                source,
            });
            return Ok(());
        }
        let child = head.unbox().expect("nil path string");
        let raw = head.scalar_bytes();
        let lower = go_lowercase(&raw);
        let lower = std::str::from_utf8(&lower).unwrap();
        let unquoted = head.unquoted_string.is_some();
        let reserved = is_reserved_keyword(lower) && unquoted;
        let text = if reserved {
            lower.to_owned()
        } else {
            ast::ByteString::from(raw).to_text().into_owned()
        };
        let range = child.range();
        let error = |message: String| ast::Error {
            range: range.clone(),
            message: format!("{range}: {message}"),
        };
        if reserved && !COMPOSITE_RESERVED_KEYWORDS.contains(&lower) && i + 1 < path_len {
            return Err(error(format!(
                "\"{text}\" must be the last part of the key"
            )));
        }
        if unquoted && text == "_" {
            return Err(error(
                "parent \"_\" can only be used in the beginning of paths, e.g. \"_.x\"".into(),
            ));
        }
        if unquoted
            && (text == "classes" || BOARD_KEYWORDS.contains(&lower))
            && self.arena.board_kind(map).is_empty()
        {
            return Err(error(format!(
                "{text} must be declared at a board root scope"
            )));
        }
        let found = if q.indexed {
            self.arena
                .get_field_indexed(map, std::slice::from_ref(&head))
        } else {
            self.arena.get_field(map, std::slice::from_ref(&head))
        };
        if let Some(field) = found {
            self.add_reference(field, Some(child), q);
            if i + 1 == path_len {
                self.select_fields([field], q, result);
                return Ok(());
            }
            if !self.filter_field(field, true, q) {
                return Ok(());
            }
            let Node::Field(f) = self.arena.node(field) else {
                unreachable!()
            };
            if f.composite
                .is_some_and(|c| matches!(self.arena.node(c), Node::Array(_)))
            {
                return Err(error("cannot index into array".into()));
            }
            let nested = self.ensure_map(field);
            frames.push(FieldFrame::Enter(nested, i + 1));
            return Ok(());
        }
        if !q.create {
            return Ok(());
        }
        if !reserved
            && !self.glob_ref_context_stack.is_empty()
            && matches!(self.parent_shape(map).as_slice(), b"class" | b"sql_table")
        {
            return Ok(());
        }
        let source = self.glob_source(q.context.as_ref());
        if !self.glob_ref_context_stack.is_empty()
            && !self.reserve_glob_generated_field_work(map, source.as_ref())
        {
            return Ok(());
        }
        let field = self.arena.insert(
            Some(map),
            Node::Field(Field {
                name: head,
                ..Default::default()
            }),
        );
        self.add_reference(field, Some(child), q);
        // Register the defer before filtering or reserving insertion work.
        if !self.glob_ref_context_stack.is_empty() {
            frames.push(FieldFrame::Applied(field, i));
        }
        if !self.filter_field(field, true, q) {
            return Ok(());
        }
        if !self.glob_ref_context_stack.is_empty() && !self.reserve_glob_field(source.as_ref()) {
            return Ok(());
        }
        self.arena.append_field(map, field);
        result.created.push(field);
        if i + 1 == path_len {
            self.select_fields([field], q, result);
            return Ok(());
        }
        let nested = self.ensure_map(field);
        frames.push(FieldFrame::Enter(nested, i + 1));
        Ok(())
    }
}
