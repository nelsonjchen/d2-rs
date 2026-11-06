// SPDX-License-Identifier: MPL-2.0
//! Source-ordered lazy glob worklist from d2ir/compile.go.
use super::{Compiler, FieldQuery, FieldQueryResult};
use d2_rs_core::{
    ast::{self, Node as AstNode, Shared},
    ir::*,
};
impl Compiler {
    pub(super) fn version(&self, map: NodeId) -> u64 {
        let Node::Map(m) = self.arena.node(map) else {
            panic!("expected map")
        };
        m.structure_version
    }
    pub(super) fn ensure_fields(
        &mut self,
        map: NodeId,
        path: Shared<ast::KeyPath>,
        context: Shared<RefContext>,
        create: bool,
    ) -> FieldQueryResult {
        self.ensure_fields_mode(map, path, Some(context), create, true)
    }
    pub(super) fn ensure_fields_mode(
        &mut self,
        mut map: NodeId,
        path: Shared<ast::KeyPath>,
        context: Option<Shared<RefContext>>,
        create: bool,
        active: bool,
    ) -> FieldQueryResult {
        let mut i = 0;
        loop {
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
            if head.scalar_bytes() != b"_" || head.unquoted_string.is_none() {
                break;
            }
            let range = head.unbox().unwrap().range();
            let failure = |message: &str| FieldQueryResult {
                error: Some(ast::Error {
                    range: range.clone(),
                    message: format!("{range}: {message}"),
                }),
                ..Default::default()
            };
            let Some(parent) = self.arena.parent_map(map) else {
                return failure("invalid underscore: no parent");
            };
            map = parent;
            if i + 1 == path.read().path.as_ref().unwrap().len() {
                return failure("field key must contain more than underscores");
            }
            i += 1;
        }
        let glob = context
            .as_ref()
            .filter(|c| active && c.read().key.as_ref().unwrap().read().has_glob())
            .and_then(|c| self.get_glob_context(c));
        let result = self.construct_fields(
            map,
            i,
            &FieldQuery {
                path,
                context,
                create,
                glob,
                indexed: true,
            },
        );
        if active
            && self.applying_lazy_worklist
            && !self.glob_ref_context_stack.is_empty()
            && !result.created.is_empty()
        {
            self.enqueue_lazy_fields(&result.fields)
        }
        if active && !result.fields.is_empty() && self.glob_ref_context_stack.is_empty() {
            if !result.created.is_empty() {
                self.enqueue_post_targets(&result.fields);
                self.apply_lazy_globs(&result.fields)
            } else {
                self.apply_lazy_globs(&[])
            }
        }
        result
    }
    pub(super) fn compile_key(&mut self, context: Shared<RefContext>) {
        #[cfg(feature = "compiler-trace")]
        {
            self.trace_key("enter", &context);
            self.compile_key_body(context.clone());
            self.trace_key("exit", &context);
        }
        #[cfg(not(feature = "compiler-trace"))]
        self.compile_key_body(context);
    }
    fn compile_key_body(&mut self, context: Shared<RefContext>) {
        if self.stopped() {
            return;
        }
        let post_start = self.lazy_post_targets.len();
        let has_glob = context.read().key.as_ref().unwrap().read().has_glob();
        if (has_glob || !self.glob_ref_context_stack.is_empty())
            && !self.reserve_glob_work(self.glob_source(Some(&context)).as_ref(), 1)
        {
            return;
        }
        if has_glob {
            if self
                .glob_ref_context_stack
                .iter()
                .any(|c| c.read().equals(&context.read()))
            {
                return;
            }
            self.glob_ref_context_stack.push(context.clone());
            self.ensure_glob_context(context.clone());
        }
        let map = context.read().scope_map.unwrap();
        let old_version = self.version(map);
        if context
            .read()
            .key
            .as_ref()
            .unwrap()
            .read()
            .edges
            .as_ref()
            .is_some_and(|e| !e.is_empty())
        {
            self.compile_edges(context.clone())
        } else {
            self.compile_field(context.clone())
        }
        let root = self.arena.root_map(map);
        let mut settled = self
            .lazy_settled_versions
            .as_ref()
            .is_some_and(|v| v.get(&root).copied().unwrap_or(0) == self.version(root));
        if old_version != self.version(map)
            && !self.applying_lazy_worklist
            && self.lazy_post_targets.len() > post_start
        {
            let targets = self.take_post_targets(post_start);
            self.apply_lazy_globs(&targets);
            settled = true;
        }
        if old_version != self.version(map) && !self.applying_lazy_worklist && !settled {
            for glob in self.glob_contexts().to_vec() {
                let context = self.arena.glob(glob).context.clone().unwrap();
                let old = self.lazy_glob_being_applied;
                self.lazy_glob_being_applied = true;
                self.compile_key(context);
                self.lazy_glob_being_applied = old;
            }
        }
        if has_glob {
            self.glob_ref_context_stack.pop();
        }
    }
    fn take_post_targets(&mut self, start: usize) -> Vec<NodeId> {
        let targets = self.lazy_post_targets.split_off(start);
        if let Some(queued) = &mut self.lazy_post_queued {
            for target in &targets {
                queued.remove(target);
            }
            if queued.is_empty() {
                self.lazy_post_queued = None
            }
        }
        targets
    }
    fn enqueue_post_targets(&mut self, fields: &[NodeId]) {
        let queued = self.lazy_post_queued.get_or_insert_default();
        for field in fields {
            if queued.insert(*field) {
                self.lazy_post_targets.push(*field)
            }
        }
    }
    fn enqueue_lazy_fields(&mut self, fields: &[NodeId]) {
        if fields.is_empty() {
            return;
        }
        let queued = self.lazy_glob_queued.get_or_insert_default();
        for field in fields {
            if queued.insert(*field) {
                self.lazy_glob_worklist.push_back(*field)
            }
        }
    }
    fn apply_lazy_globs(&mut self, created: &[NodeId]) {
        if self.stopped() {
            return;
        }
        if created.is_empty() {
            for glob in self.glob_contexts().to_vec() {
                let context = self.arena.glob(glob).context.clone().unwrap();
                let old = self.lazy_glob_being_applied;
                self.lazy_glob_being_applied = true;
                self.compile_key(context);
                self.lazy_glob_being_applied = old;
            }
            return;
        }
        if self.applying_lazy_worklist {
            self.enqueue_lazy_fields(created);
            return;
        }
        let root = self
            .arena
            .root_map(self.arena.parent_map(created[0]).unwrap());
        if self.glob_contexts().is_empty() {
            let version = self.version(root);
            self.lazy_settled_versions
                .get_or_insert_default()
                .insert(root, version);
            return;
        }
        self.applying_lazy_worklist = true;
        self.enqueue_lazy_fields(created);
        while let Some(target) = self.lazy_glob_worklist.pop_front() {
            if self.stopped() {
                break;
            }
            let mut edge_globs = Vec::new();
            for glob in self.glob_contexts().to_vec() {
                let context = self.arena.glob(glob).context.clone().unwrap();
                if context
                    .read()
                    .key
                    .as_ref()
                    .unwrap()
                    .read()
                    .edges
                    .as_ref()
                    .is_some_and(|e| !e.is_empty())
                {
                    edge_globs.push(context);
                    continue;
                }
                self.lazy_glob_target = Some(target);
                let old = self.lazy_glob_being_applied;
                self.lazy_glob_being_applied = true;
                self.compile_key(context);
                self.lazy_glob_being_applied = old;
            }
            self.lazy_glob_target = None;
            let root = self.arena.root_map(self.arena.parent_map(target).unwrap());
            while !edge_globs.is_empty() {
                let before = self.version(root);
                for context in &edge_globs {
                    let old = self.lazy_glob_being_applied;
                    self.lazy_glob_being_applied = true;
                    self.compile_key(context.clone());
                    self.lazy_glob_being_applied = old;
                }
                if before == self.version(root) {
                    break;
                }
            }
        }
        let version = self.version(root);
        self.lazy_settled_versions
            .get_or_insert_default()
            .insert(root, version);
        self.applying_lazy_worklist = false;
        self.lazy_glob_target = None;
        self.lazy_glob_worklist.clear();
        self.lazy_glob_queued = None;
    }
}
