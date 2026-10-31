// SPDX-License-Identifier: MPL-2.0
//! Initial connected AST-to-IR compiler. Untranslated language features are
//! rejected explicitly before compilation; they must not masquerade as passing.
use super::Compiler;
use d2_rs_core::{
    ast::{self, Node as AstNode, Shared},
    ir::*,
};

#[derive(Debug, Clone)]
pub struct UntranslatedFeature {
    pub feature: &'static str,
    pub range: ast::Range,
}
impl std::fmt::Display for UntranslatedFeature {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}: compiler feature not translated: {}",
            self.range, self.feature
        )
    }
}
impl std::error::Error for UntranslatedFeature {}

/// Compile the currently translated language subset. The returned compiler
/// retains partial IR and ordered D2 errors for diagnostics. A successful Rust
/// Result is not a successful D2 compile unless `compiler.errors` is empty.
pub fn compile_ir(source: Shared<ast::Map>) -> Result<(Compiler, NodeId), UntranslatedFeature> {
    compile_ir_with_options(source, CompileOptions::default())
}
/// Compile with an explicit native or virtual filesystem provider.
pub fn compile_ir_with_imports(
    source: Shared<ast::Map>,
    opener: super::imports::ImportOpener,
) -> Result<(Compiler, NodeId), UntranslatedFeature> {
    compile_ir_with_options(
        source,
        CompileOptions {
            import_opener: Some(opener),
            ..Default::default()
        },
    )
}
/// Options for the translated IR compiler. Source AST positions are supplied by
/// the caller; `utf16_positions` controls parsing of newly opened imports.
#[derive(Default)]
pub struct CompileOptions {
    pub import_opener: Option<super::imports::ImportOpener>,
    pub utf16_positions: bool,
}
/// Work limits for native compilation. Kept separate from filesystem options
/// so existing explicit `CompileOptions` initializers remain compatible.
#[derive(Default)]
pub struct CompileLimits {
    /// Glob traversal/materialization work; zero selects 65,536.
    pub max_glob_expansion: i64,
    /// Distinct glob segment/endpoint combinations; zero selects 1,024.
    pub max_edge_expansion: i64,
    /// All glob endpoint-pair examinations, including replays; zero selects 65,536.
    pub max_edge_expansion_work: i64,
    /// Compilation-wide substitution/copy work. Zero selects the pinned
    /// upstream default (65,536); negative values are invalid.
    pub max_variable_expansion: i64,
}
/// Cooperative context check for an IR compilation. Return the original cause
/// (for example `LayoutError::Cancelled` or `DeadlineExceeded`) to stop work.
/// A shared `CancellationToken` can be used with `Box::new(move || token.error())`.
pub type CompileContext = Box<dyn FnMut() -> Option<d2_rs_core::LayoutError>>;
/// Compile with explicit filesystem and imported source-position options.
pub fn compile_ir_with_options(
    source: Shared<ast::Map>,
    options: CompileOptions,
) -> Result<(Compiler, NodeId), UntranslatedFeature> {
    compile_ir_with_limits(source, options, CompileLimits::default())
}
/// Compile with explicit work limits, sharing the default substitution limit.
pub fn compile_ir_with_limits(
    source: Shared<ast::Map>,
    options: CompileOptions,
    limits: CompileLimits,
) -> Result<(Compiler, NodeId), UntranslatedFeature> {
    compile_ir_with_context(source, options, limits, None)
}
/// Compile a parsed AST with a caller-provided context. Checks precede work
/// limit validation and recur during IR construction and expansion. A canceled
/// result retains diagnostic partial IR, but must never be published as success:
/// `context_error` holds the typed cause and `errors` contains its exact text.
/// This context does not control the caller's earlier parse or later graph pass.
pub fn compile_ir_with_context(
    source: Shared<ast::Map>,
    options: CompileOptions,
    limits: CompileLimits,
    context: Option<CompileContext>,
) -> Result<(Compiler, NodeId), UntranslatedFeature> {
    let mut c = Compiler {
        context,
        import_opener: options.import_opener,
        import_state: super::imports::ImportState {
            utf16_positions: options.utf16_positions,
            ..Default::default()
        },
        ..Default::default()
    };
    let root = c.arena.new_root();
    if c.stopped() {
        c.publish_context_error();
        return Ok((c, root));
    }
    // Upstream validates variable, glob, edge and edge-work limits in order.
    for (name, limit) in [
        ("MaxVariableExpansion", limits.max_variable_expansion),
        ("MaxGlobExpansion", limits.max_glob_expansion),
    ] {
        if limit < 0 {
            c.errors.push(ast::Error {
                message: format!("{name} must not be negative"),
                ..Default::default()
            });
            return Ok((c, root));
        }
    }
    if limits.max_glob_expansion > 0 {
        c.glob_expansion.limit = limits.max_glob_expansion as u64;
    }
    for (name, limit) in [
        ("MaxEdgeExpansion", limits.max_edge_expansion),
        ("MaxEdgeExpansionWork", limits.max_edge_expansion_work),
    ] {
        if limit < 0 {
            c.errors.push(ast::Error {
                message: format!("{name} must not be negative"),
                ..Default::default()
            });
            return Ok((c, root));
        }
    }
    if limits.max_edge_expansion > 0 {
        c.edge_expansion.limit = limits.max_edge_expansion as u64;
    }
    if limits.max_edge_expansion_work > 0 {
        c.edge_expansion.work_limit = limits.max_edge_expansion_work as u64;
    }
    if limits.max_variable_expansion > 0 {
        c.variable_expansion
            .limit
            .set(limits.max_variable_expansion as u64);
    }
    c.import_state.stack.push(source.read().range.path.clone());
    let parent = c.arena.parent(root).unwrap();
    let Node::Field(f) = c.arena.node(parent) else {
        unreachable!()
    };
    {
        let context = f.references.as_ref().unwrap()[0]
            .read()
            .context
            .clone()
            .unwrap();
        let mut context = context.write();
        context.scope = Some(source.clone());
        context.scope_ast = Some(source.clone());
    }
    c.compile_map(root, source.clone(), source);
    c.compile_substitutions(root, Vec::new());
    if !c.stopped() {
        c.overlay_classes(root);
    }
    if !c.stopped() {
        c.reserve_variable_aliases(root);
    }
    if !c.stopped() {
        c.remove_suspended_fields(root);
    }
    c.import_state.pop();
    // Go returns a context cause ahead of accumulated source diagnostics.
    c.stopped();
    c.publish_context_error();
    if c.errors.is_empty()
        && let Some(message) = c.variable_expansion_error.clone()
    {
        c.errors.push(ast::Error {
            message,
            ..Default::default()
        });
    }
    if let Some(pending) = c.untranslated.first() {
        return Err(pending.clone());
    }
    Ok((c, root))
}
impl Compiler {
    pub(super) fn stopped(&mut self) -> bool {
        if self.halted {
            return true;
        }
        if let Some(error) = self.context.as_mut().and_then(|check| check()) {
            self.context_error = Some(error);
            self.halted = true;
            return true;
        }
        false
    }
    fn publish_context_error(&mut self) {
        if let Some(error) = &self.context_error {
            self.errors = vec![ast::Error {
                message: error.to_string(),
                ..Default::default()
            }];
        }
    }
    pub(super) fn compile_map(
        &mut self,
        dst: NodeId,
        source: Shared<ast::Map>,
        scope: Shared<ast::Map>,
    ) {
        if self.stopped() {
            return;
        }
        self.push_map_globs(dst);
        let nodes = if self.filter_map(dst, source.clone(), scope.clone()) {
            source.read().children()
        } else {
            Vec::new()
        };
        for node in nodes {
            if self.stopped() {
                break;
            }
            match node {
                ast::Child::Key(key) => self.compile_key(Shared::new(RefContext {
                    key: Some(key),
                    scope: Some(source.clone()),
                    scope_map: Some(dst),
                    scope_ast: Some(scope.clone()),
                    ..Default::default()
                })),
                ast::Child::Import(import) => self.compile_spread_import(dst, import),
                ast::Child::Substitution(sub) => {
                    self.substitution_placeholder(dst, sub, source.clone(), scope.clone())
                }
                ast::Child::Comment(_) | ast::Child::BlockComment(_) => {}
                _ => unreachable!("untranslated map entry passed preflight"),
            }
        }
        let globs = self.glob_context_stack.pop().unwrap();
        let Node::Map(m) = self.arena.node_mut(dst) else {
            unreachable!()
        };
        m.globs =
            (!globs.is_empty()).then(|| globs.into_iter().map(Some).collect::<Vec<_>>().into());
    }
    pub(super) fn compile_field(&mut self, context: Shared<RefContext>) {
        let map = context.read().scope_map.unwrap();
        let path = context
            .read()
            .key
            .as_ref()
            .unwrap()
            .read()
            .key
            .clone()
            .unwrap();
        self.compile_field_path(map, path, context);
    }
    pub(super) fn compile_field_path(
        &mut self,
        map: NodeId,
        path: Shared<ast::KeyPath>,
        context: Shared<RefContext>,
    ) {
        if self.stopped() {
            return;
        }
        let key = context.read().key.clone().unwrap();
        if key.read().ampersand || key.read().not_ampersand {
            return;
        }
        let result = self.ensure_fields(map, path, context.clone(), true);
        if let Some(error) = result.error {
            self.errors.push(error);
            return;
        }
        for field in result.fields {
            self.apply_field_value(field, context.clone())
        }
    }
    pub(super) fn ignore_lazy_glob(&self, node: NodeId) -> bool {
        self.lazy_glob_being_applied
            && self.arena.primary(node).is_some()
            && self
                .arena
                .last_primary_ref(node)
                .is_some_and(|r| !r.due_to_lazy_glob())
    }
    pub(super) fn set_primary(&mut self, field: NodeId, value: ast::ScalarBox) {
        let scalar = self
            .arena
            .insert(Some(field), Node::Scalar(Scalar { value }));
        match self.arena.node_mut(field) {
            Node::Field(f) => f.primary = Some(scalar),
            Node::Edge(e) => e.primary = Some(scalar),
            _ => unreachable!(),
        }
    }
    fn apply_field_value(&mut self, field: NodeId, context: Shared<RefContext>) {
        if !self.filter_value_map(field, context.clone()) {
            return;
        }
        let key = context.read().key.clone().unwrap();
        let (primary, value) = {
            let key = key.read();
            (key.primary.clone(), key.value.clone())
        };
        let no_edges = key.read().edges.as_ref().is_none_or(|e| e.is_empty());
        if no_edges
            && (primary.null.is_some() || value.null.is_some())
            && !self.arena.is_var(self.arena.parent_map(field).unwrap())
        {
            let Node::Field(f) = self.arena.node(field) else {
                unreachable!()
            };
            let name = ast::ByteString::from(f.name.scalar_bytes());
            self.arena
                .delete_field(self.arena.parent_map(field).unwrap(), &[name]);
            return;
        }
        if let Some(suspension) = primary.suspension.as_ref().or(value.suspension.as_ref())
            && no_edges
        {
            if !self.lazy_glob_being_applied {
                let Node::Field(f) = self.arena.node_mut(field) else {
                    unreachable!()
                };
                f.suspended = suspension.read().value
            }
            return;
        }
        if primary.unbox().is_some() {
            if self.ignore_lazy_glob(field) {
                return;
            }
            self.set_primary(field, primary)
        }
        if let Some(array) = value.array {
            let id = self
                .arena
                .insert(Some(field), Node::Array(Array::default()));
            self.compile_array(id, array, context.read().scope_ast.clone().unwrap());
            let Node::Field(f) = self.arena.node_mut(field) else {
                unreachable!()
            };
            f.composite = Some(id);
        } else if let Some(map) = value.map {
            let mut scope = map.clone();
            if self.arena.map(field).is_none() {
                self.ensure_map(field);
                self.inherit_board(field);
                if self.arena.board_kind(field).is_empty() {
                    scope = context.read().scope_ast.clone().unwrap()
                }
            } else {
                scope = context.read().scope_ast.clone().unwrap()
            }
            let dst = self.arena.map(field).unwrap();
            self.map_ref_context_stack.push(context.clone());
            self.compile_map(dst, map, scope);
            self.map_ref_context_stack.pop();
            if matches!(self.arena.board_kind(field), "scenario" | "step") {
                self.overlay_classes(self.arena.map(field).unwrap())
            }
        } else if let Some(import) = value.import {
            self.compile_field_import(field, import, context.read().scope_map.unwrap());
        } else if value.scalar_box().unbox().is_some() {
            if self.ignore_lazy_glob(field) {
                return;
            }
            self.set_primary(field, value.scalar_box());
            let Node::Field(f) = self.arena.node(field) else {
                unreachable!()
            };
            if f.name.scalar_bytes() == b"link" && f.name.unquoted_string.is_some() {
                self.compile_link(field, &context)
            }
        }
    }
    pub(super) fn compile_array(
        &mut self,
        dst: NodeId,
        array: Shared<ast::Array>,
        scope: Shared<ast::Map>,
    ) {
        if self.stopped() {
            return;
        }
        self.register_array(dst);
        for value in array.read().children() {
            if self.stopped() {
                return;
            }
            if !self.glob_ref_context_stack.is_empty() && !self.reserve_glob_work(Some(&value), 1) {
                return;
            }
            let node = match value {
                ast::Child::Array(array) => {
                    let node = self.arena.insert(Some(dst), Node::Array(Array::default()));
                    self.compile_array(node, array, scope.clone());
                    node
                }
                ast::Child::Map(map) => {
                    let node = self.arena.insert(Some(dst), Node::Map(Map::default()));
                    self.compile_map(node, map, scope.clone());
                    node
                }
                ast::Child::Import(import) => {
                    let Some(node) = self.import_node(import.clone(), dst) else {
                        continue;
                    };
                    self.set_import_ast(node, import.clone());
                    let spread = import.read().spread;
                    match self.arena.node(node).clone() {
                        Node::Field(field) => {
                            if spread {
                                let Some(array) = field
                                    .composite
                                    .filter(|a| matches!(self.arena.node(*a), Node::Array(_)))
                                else {
                                    self.errors.push(super::edge_creation::error(
                                        &ast::Child::Import(import),
                                        "can only spread import array into array",
                                    ));
                                    continue;
                                };
                                if !self.reserve_variable_copy(
                                    &ast::Child::Import(import.clone()),
                                    array,
                                ) {
                                    return;
                                }
                                let Node::Array(array) = self.arena.node(array) else {
                                    unreachable!()
                                };
                                let values = array.values.clone().unwrap_or_default();
                                self.append_compiled_array(dst, &values);
                                continue;
                            }
                            field
                                .composite
                                .or(field.primary)
                                .unwrap_or_else(|| self.arena.nil_scalar())
                        }
                        Node::Map(_) => {
                            if spread {
                                self.errors.push(super::edge_creation::error(
                                    &ast::Child::Import(import),
                                    "can only spread import array into array",
                                ));
                                continue;
                            }
                            node
                        }
                        _ => unreachable!(),
                    }
                }
                ast::Child::Substitution(sub) => self.substitution_array_value(dst, sub),
                ast::Child::Comment(_) => continue,
                value => self.arena.insert(
                    Some(dst),
                    Node::Scalar(Scalar {
                        value: ast::ScalarBox::from_child(Some(value)),
                    }),
                ),
            };
            if self.stopped() {
                return;
            }
            self.append_compiled_array(dst, &[node]);
        }
    }
    fn remove_suspended_fields(&mut self, map: NodeId) {
        let Node::Map(m) = self.arena.node(map) else {
            unreachable!()
        };
        let fields = m.fields.clone().unwrap_or_default();
        let edges = m.edges.clone().unwrap_or_default();
        for field in &fields {
            if let Some(map) = self.arena.map(*field) {
                self.remove_suspended_fields(map)
            }
        }
        for field in fields.into_iter().rev() {
            let Node::Field(f) = self.arena.node(field) else {
                unreachable!()
            };
            if f.suspended
                && f.name.unbox().is_some()
                && !std::str::from_utf8(&f.name.scalar_bytes())
                    .is_ok_and(ast::keywords::is_reserved_keyword)
            {
                let bytes = f.name.scalar_bytes();
                self.arena.delete_field(map, &[bytes.into()]);
            }
        }
        for edge in edges {
            if let Some(map) = self.arena.map(edge) {
                self.remove_suspended_fields(map)
            }
        }
        let Node::Map(m) = self.arena.node(map) else {
            unreachable!()
        };
        let edges = m.edges.clone().unwrap_or_default();
        for edge in edges.into_iter().rev() {
            let Node::Edge(e) = self.arena.node(edge) else {
                unreachable!()
            };
            if e.suspended {
                let id = e.id.as_ref().unwrap().read().clone();
                self.arena.delete_edge(map, Some(&id));
            }
        }
    }
}
impl Compiler {
    fn compile_link(&mut self, field: NodeId, context: &Shared<RefContext>) {
        let value = context
            .read()
            .key
            .as_ref()
            .unwrap()
            .read()
            .value
            .scalar_box()
            .scalar_bytes();
        let Ok(link) = crate::parse_key(&value) else {
            return;
        };
        let mut scope =
            crate::ir::ida(&self.arena, context.read().scope_map.unwrap()).unwrap_or_default();
        let mut link = link.ida().unwrap_or_default();
        if scope.is_empty() || link.is_empty() {
            return;
        }
        let bytes = |p: &Option<ast::Child>| ast::ScalarBox::from_child(p.clone()).scalar_bytes();
        let unquoted = |p: &Option<ast::Child>| matches!(p, Some(ast::Child::Unquoted(_)));
        let board = |p: &Option<ast::Child>| {
            [b"layers".as_slice(), b"scenarios", b"steps"]
                .iter()
                .any(|b| equal_fold(&bytes(p), b))
        };
        if !unquoted(&link[0]) || (!board(&link[0]) && bytes(&link[0]) != b"_") {
            return;
        }
        for i in (1..scope.len()).rev() {
            if unquoted(&scope[i - 1]) && board(&scope[i - 1]) {
                scope.truncate(i + 1);
                break;
            }
            if unquoted(&scope[i - 1]) && bytes(&scope[i - 1]) == b"root" {
                scope.truncate(i);
                break;
            }
        }
        while !link.is_empty() && unquoted(&link[0]) && bytes(&link[0]) == b"_" {
            if scope.len() < 2 {
                break;
            }
            scope.truncate(scope.len() - 2);
            link.remove(0);
        }
        if scope.is_empty() {
            scope.push(Some(ast::Child::Unquoted(Shared::new(
                ast::UnquotedString::flat("root"),
            ))))
        }
        scope.extend(link);
        let path = ast::KeyPath::from_strings(scope.iter().map(bytes));
        let value = crate::format::format(&ast::Child::KeyPath(Shared::new(path)));
        let scalar = self.arena.primary(field).unwrap();
        let Node::Scalar(s) = self.arena.node_mut(scalar) else {
            unreachable!()
        };
        s.value = ast::ScalarBox::from_child(Some(ast::Child::Unquoted(Shared::new(
            ast::UnquotedString::flat(value),
        ))));
    }
}
