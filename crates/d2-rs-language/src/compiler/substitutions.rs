// SPDX-License-Identifier: MPL-2.0
//! Scope lookup and IR substitution from pinned d2ir/compile.go.
use super::{Compiler, edge_creation::error};
use d2_rs_core::{
    ast::{self, Shared},
    ir::*,
};
use d2_rs_text::markdown::substitute::{
    SubstitutionError, replace_substitutions_bounded_with_context,
};
use std::collections::{BTreeMap, HashSet};

fn substitution_value(substitution: Shared<ast::Substitution>) -> ast::ScalarBox {
    ast::ScalarBox {
        unquoted_string: Some(Shared::new(ast::UnquotedString {
            value: Some(
                vec![ast::InterpolationBox {
                    substitution: Some(substitution),
                    ..Default::default()
                }]
                .into(),
            ),
            ..Default::default()
        })),
        ..Default::default()
    }
}
impl Compiler {
    // d2ir.collectVariables: reserve every prepared ${name} before allocating
    // joined names, and share visitation accounting across nested namespaces.
    fn collect_markdown_variables(
        &mut self,
        map: NodeId,
        variables: &mut BTreeMap<String, String>,
        seen: &mut HashSet<NodeId>,
        source: Option<&ast::Child>,
    ) {
        if !self.visit_substitution_node(map, source, seen) {
            return;
        }
        for field in self.substitution_fields(map) {
            if !self.visit_substitution_node(field, source, seen) {
                return;
            }
            let Node::Field(f) = self.arena.node(field) else {
                unreachable!()
            };
            let name = f.name.scalar_string();
            if let Some(primary) = f.primary
                && let Node::Scalar(s) = self.arena.node(primary)
            {
                let value = s.value.clone();
                if !self.reserve_variable(source, name.len() as u64 + 3) {
                    return;
                }
                variables.insert(name, value.scalar_string());
            } else if let Some(map) = self.arena.map(field) {
                let mut nested = BTreeMap::new();
                self.collect_markdown_variables(map, &mut nested, seen, source);
                if self.stopped() {
                    return;
                }
                for (key, value) in nested {
                    if !self.reserve_variable(source, (name.len() + key.len() + 4) as u64) {
                        return;
                    }
                    variables.insert(format!("{name}.{key}"), value);
                }
                self.collect_markdown_variables(map, variables, seen, source);
                if self.stopped() {
                    return;
                }
            }
        }
    }
    pub(super) fn substitution_placeholder(
        &mut self,
        dst: NodeId,
        substitution: Shared<ast::Substitution>,
        source: Shared<ast::Map>,
        scope: Shared<ast::Map>,
    ) {
        let source_node = ast::Child::Substitution(substitution.clone());
        if !self.glob_ref_context_stack.is_empty()
            && (!self.reserve_glob_generated_field_work(dst, Some(&source_node))
                || !self.reserve_glob_field(Some(&source_node)))
        {
            return;
        }
        let field = self.arena.insert(
            Some(dst),
            Node::Field(Field {
                references: Some(vec![Shared::new(FieldReference {
                    context: Some(Shared::new(RefContext {
                        scope: Some(source),
                        scope_map: Some(dst),
                        scope_ast: Some(scope),
                        ..Default::default()
                    })),
                    ..Default::default()
                })]),
                ..Default::default()
            }),
        );
        // The Go placeholder primary has no parent; array placeholders do.
        let primary = self.arena.insert(
            None,
            Node::Scalar(Scalar {
                value: substitution_value(substitution),
            }),
        );
        let Node::Field(f) = self.arena.node_mut(field) else {
            unreachable!()
        };
        f.primary = Some(primary);
        self.arena.append_field(dst, field);
    }
    pub(super) fn substitution_array_value(
        &mut self,
        dst: NodeId,
        substitution: Shared<ast::Substitution>,
    ) -> NodeId {
        self.arena.insert(
            Some(dst),
            Node::Scalar(Scalar {
                value: substitution_value(substitution),
            }),
        )
    }
    pub(super) fn compile_substitutions(&mut self, map: NodeId, vars: Vec<NodeId>) {
        self.compile_substitutions_walk(map, vars, &mut HashSet::new(), None);
    }
    fn compile_substitutions_walk(
        &mut self,
        map: NodeId,
        vars: Vec<NodeId>,
        seen: &mut HashSet<NodeId>,
        source: Option<ast::Child>,
    ) {
        enum Frame {
            Enter(NodeId, Vec<NodeId>, Option<ast::Child>),
            Fields(NodeId, Vec<NodeId>, isize),
            Config(NodeId),
            Edges(std::vec::IntoIter<NodeId>, Vec<NodeId>),
        }
        let mut frames = vec![Frame::Enter(map, vars, source)];
        while let Some(frame) = frames.pop() {
            match frame {
                Frame::Enter(map, mut vars, source) => {
                    if !self.visit_substitution_node(map, source.as_ref(), seen) {
                        continue;
                    }
                    for field in self.substitution_fields(map) {
                        let Node::Field(f) = self.arena.node(field) else {
                            unreachable!()
                        };
                        if f.name.unquoted_string.is_some()
                            && f.name.scalar_bytes() == b"vars"
                            && let Some(map) = self.arena.map(field)
                        {
                            vars.insert(0, map);
                        }
                    }
                    frames.push(Frame::Fields(map, vars, 0));
                }
                Frame::Fields(map, vars, mut i) => {
                    // Resolve against the current slice: substitutions can remove
                    // a field and require revisiting its replacement at this index.
                    let fields = self.substitution_fields(map);
                    if i as usize >= fields.len() {
                        let Node::Map(m) = self.arena.node(map) else {
                            unreachable!()
                        };
                        frames.push(Frame::Edges(
                            m.edges.clone().unwrap_or_default().into_iter(),
                            vars,
                        ));
                        continue;
                    }
                    let field = fields[i as usize];
                    let source = self.arena.last_ref(field).and_then(|r| r.ast());
                    if !self.visit_substitution_node(field, source.as_ref(), seen) {
                        continue;
                    }
                    if let Some(primary) = self.arena.primary(field)
                        && !self.visit_substitution_node(primary, source.as_ref(), seen)
                    {
                        continue;
                    }
                    if self.arena.primary(field).is_some()
                        && self.resolve_substitutions(&vars, field)
                    {
                        i -= 1;
                    }
                    let Node::Field(f) = self.arena.node(field) else {
                        unreachable!()
                    };
                    let composite = f.composite;
                    let is_vars =
                        f.name.unquoted_string.is_some() && f.name.scalar_bytes() == b"vars";
                    if let Some(composite) = composite
                        && let Node::Array(_) = self.arena.node(composite)
                    {
                        if !self.visit_substitution_node(composite, source.as_ref(), seen) {
                            continue;
                        }
                        let values = self.array_view(composite);
                        let mut admitted = true;
                        for value_index in 0..values.len() {
                            let val = values.get(value_index);
                            if !self.visit_substitution_node(val, source.as_ref(), seen) {
                                admitted = false;
                                break;
                            }
                            if matches!(self.arena.node(val), Node::Scalar(_))
                                && self.resolve_substitutions(&vars, val)
                            {
                                i -= 1;
                            }
                        }
                        if !admitted {
                            continue;
                        }
                        frames.push(Frame::Fields(map, vars, i + 1));
                    } else if let Some(child) = self.arena.map(field) {
                        let child_vars = vars.clone();
                        frames.push(Frame::Fields(map, vars, i + 1));
                        if is_vars {
                            frames.push(Frame::Config(child));
                        }
                        frames.push(Frame::Enter(child, child_vars, source));
                    } else {
                        frames.push(Frame::Fields(map, vars, i + 1));
                    }
                }
                Frame::Config(map) => {
                    let config = self
                        .arena
                        .get_field_indexed(map, &[ast::raw_string_box("d2-config", true)]);
                    self.validate_configs(config);
                }
                Frame::Edges(mut edges, vars) => {
                    let Some(edge) = edges.next() else {
                        continue;
                    };
                    let source = self.arena.last_ref(edge).and_then(|r| r.ast());
                    if !self.visit_substitution_node(edge, source.as_ref(), seen) {
                        continue;
                    }
                    if let Some(primary) = self.arena.primary(edge) {
                        if !self.visit_substitution_node(primary, source.as_ref(), seen) {
                            continue;
                        }
                        self.resolve_substitutions(&vars, edge);
                    }
                    if let Some(child) = self.arena.map(edge) {
                        let child_vars = vars.clone();
                        frames.push(Frame::Edges(edges, vars));
                        frames.push(Frame::Enter(child, child_vars, source));
                    } else {
                        frames.push(Frame::Edges(edges, vars));
                    }
                }
            }
        }
    }
    fn substitution_fields(&self, map: NodeId) -> Vec<NodeId> {
        let Node::Map(m) = self.arena.node(map) else {
            unreachable!()
        };
        m.fields.clone().unwrap_or_default()
    }
    fn substitution_error(&mut self, node: NodeId, message: impl Into<String>) {
        let ast = self.arena.last_ref(node).unwrap().ast().unwrap();
        self.errors.push(error(&ast, message));
    }
    fn resolve_substitution(
        &mut self,
        mut vars: NodeId,
        node: NodeId,
        sub: &Shared<ast::Substitution>,
        current: bool,
    ) -> Option<NodeId> {
        let path: Vec<_> = sub
            .read()
            .path
            .iter()
            .flat_map(ast::SharedSlice::iter)
            .map(|p| p.read().clone().unwrap())
            .collect();
        for (i, p) in path.iter().enumerate() {
            let p = p.read();
            let field = self
                .arena
                .get_field_indexed(vars, std::slice::from_ref(&*p))?;
            if current
                && let Node::Field(f) = self.arena.node(node)
                && f.name.unbox().is_some()
                && f.name.scalar_bytes() == p.scalar_bytes()
            {
                let parent = self.arena.parent_field(node).unwrap();
                let Node::Field(f) = self.arena.node(parent) else {
                    unreachable!()
                };
                if f.name.unquoted_string.is_some() && f.name.scalar_bytes() == b"vars" {
                    return None;
                }
            }
            if i + 1 == path.len() {
                return Some(field);
            }
            let map = self.arena.map(field)?;
            vars = map;
        }
        None
    }
    fn resolve_substitutions(&mut self, vars: &[NodeId], node: NodeId) -> bool {
        if self.stopped() {
            return false;
        }
        let primary = self.arena.primary(node).unwrap();
        let Node::Scalar(s) = self.arena.node(primary) else {
            unreachable!()
        };
        let value = s.value.clone();
        if let Some(block) = value.block_string {
            if !block.read().value.as_bytes().windows(2).any(|v| v == b"${") {
                return false;
            }
            let source = self.arena.last_ref(node).and_then(|r| r.ast());
            let mut variables = BTreeMap::new();
            let mut seen = HashSet::new();
            for vars in vars {
                self.collect_markdown_variables(*vars, &mut variables, &mut seen, source.as_ref());
            }
            if self.stopped() {
                return false;
            }
            if !variables.is_empty() {
                let block_value = block.read().value.clone();
                let original = block_value.as_bytes();
                let remaining =
                    self.variable_expansion.limit.get() - self.variable_expansion.used.get();
                let (substituted, used) = match replace_substitutions_bounded_with_context(
                    original,
                    &variables,
                    remaining as i64,
                    &mut || self.context.as_mut().and_then(|check| check()),
                ) {
                    Ok(result) => result,
                    Err(SubstitutionError::Context(error)) => {
                        self.context_error = Some(error);
                        self.halted = true;
                        return false;
                    }
                    Err(_) => {
                        self.reserve_variable(source.as_ref(), remaining + 1);
                        return false;
                    }
                };
                if !self.reserve_variable(source.as_ref(), used) {
                    return false;
                }
                if substituted.as_bytes() != original {
                    block.write().value = substituted.into();
                }
            }
            return false;
        }
        let (slots, quoted) = if let Some(us) = &value.unquoted_string {
            (us.read().value.clone(), false)
        } else if let Some(ds) = &value.double_quoted_string {
            (ds.read().value.clone(), true)
        } else {
            return false;
        };
        let Some(slots) = slots else { return false };
        let mut subbed = false;
        let mut removed = false;
        for (i, slot) in slots.iter().enumerate() {
            let Some(sub) = slot.read().substitution.clone() else {
                continue;
            };
            let name = ast::ByteString::from(
                sub.read()
                    .path
                    .iter()
                    .flat_map(ast::SharedSlice::iter)
                    .map(|p| p.read().as_ref().unwrap().read().scalar_bytes())
                    .collect::<Vec<_>>()
                    .join(&b'.'),
            )
            .to_text()
            .into_owned();
            let mut resolved = None;
            for (i, vars) in vars.iter().enumerate() {
                resolved = self.resolve_substitution(*vars, node, &sub, i == 0);
                if let Some(field) = resolved {
                    if !quoted
                        && let Some(primary) = self.arena.primary(field)
                        && let Node::Scalar(s) = self.arena.node(primary)
                        && s.value.null.is_some()
                    {
                        resolved = None;
                    }
                    break;
                }
            }
            let Some(field) = resolved else {
                self.substitution_error(node, format!("could not resolve variable \"{name}\""));
                return removed;
            };
            let Node::Field(f) = self.arena.node(field) else {
                unreachable!()
            };
            let composite = f.composite;
            let resolved_primary = f.primary;
            if !quoted
                && let Some(composite) = composite
                && (matches!(self.arena.node(node), Node::Field(_) | Node::Edge(_))
                    || sub.read().spread)
                && (self.composite_contains(composite, node)
                    || (sub.read().spread && self.spread_references_node(composite, node, vars)))
            {
                // A substitution can be visited through multiple valid aliases.
                let identity = &*sub.read() as *const ast::Substitution as usize;
                if self.reported_composite_cycles.insert(identity) {
                    self.errors.push(error(
                        &ast::Child::Substitution(sub.clone()),
                        format!("cyclic composite variable reference \"{name}\""),
                    ));
                }
                return removed;
            }
            if quoted {
                if resolved_primary.is_none() && composite.is_some() {
                    self.substitution_error(
                        node,
                        format!("cannot substitute map variable \"{name}\" in quotes"),
                    );
                    return removed;
                }
                let Node::Scalar(s) = self
                    .arena
                    .node(resolved_primary.expect("quoted variable has no primary"))
                else {
                    unreachable!()
                };
                slot.write().string = Some(Shared::new(s.value.scalar_bytes().into()));
                subbed = true;
                continue;
            }
            if sub.read().spread {
                let Some(composite) = composite else {
                    self.errors.push(error(
                        &ast::Child::Substitution(sub.clone()),
                        "cannot spread non-composite",
                    ));
                    continue;
                };
                match self.arena.node(node) {
                    Node::Scalar(_) => {
                        if !matches!(self.arena.node(composite), Node::Array(_)) {
                            self.errors.push(error(
                                &ast::Child::Substitution(sub.clone()),
                                "cannot spread non-array into array",
                            ));
                            continue;
                        }
                        let parent = self.arena.parent(node).unwrap();
                        if !self.reserve_variable_copy(
                            &ast::Child::Substitution(sub.clone()),
                            composite,
                        ) {
                            return removed;
                        }
                        self.spread_array(parent, node, composite);
                    }
                    Node::Field(_) => {
                        let map = self.arena.parent_map(node).unwrap();
                        if let Some(resolved_map) = self.arena.map(field) {
                            if let Node::Map(m) = self.arena.node(resolved_map)
                                && m.fields.iter().flatten().any(|f| matches!(self.arena.node(*f), Node::Field(f) if f.name.unbox().is_none()))
                            {
                                self.errors.push(error(&ast::Child::Substitution(sub.clone()), format!("cannot spread composite variable \"{name}\" before its spread substitutions are resolved")));
                                return removed;
                            }
                            if !self.reserve_variable_copy(
                                &ast::Child::Substitution(sub.clone()),
                                resolved_map,
                            ) {
                                return removed;
                            }
                            self.arena
                                .expand_substitution(map, resolved_map, node, true);
                        }
                        if let Some(index) = self
                            .substitution_fields(map)
                            .iter()
                            .position(|f| *f == node)
                        {
                            self.arena.remove_field(map, index);
                            removed = true;
                        }
                        let Node::Map(m) = self.arena.node(map) else {
                            unreachable!()
                        };
                        let globs: Vec<_> = m
                            .globs
                            .iter()
                            .flat_map(ast::SharedSlice::iter)
                            .filter_map(|g| *g.read())
                            .collect();
                        if removed && !globs.is_empty() && !self.lazy_glob_being_applied {
                            let original = self.glob_context_stack.clone();
                            self.glob_context_stack.push(globs.clone());
                            for g in globs {
                                let old = self.lazy_glob_being_applied;
                                self.lazy_glob_being_applied = true;
                                let context = self.arena.glob(g).context.clone().unwrap();
                                self.compile_key(context);
                                self.lazy_glob_being_applied = old;
                            }
                            self.glob_context_stack = original;
                        }
                    }
                    _ => {}
                }
            }
            // Spread replay can modify the resolved field before value assignment.
            let Node::Field(f) = self.arena.node(field) else {
                unreachable!()
            };
            let composite = f.composite;
            let resolved_primary = f.primary;
            if let Some(resolved_primary) = resolved_primary {
                let Node::Scalar(s) = self.arena.node(resolved_primary) else {
                    unreachable!()
                };
                let replacement = s.value.clone();
                if i == 0 && slots.len() == 1 {
                    if !self.reserve_variable(
                        Some(&ast::Child::Substitution(sub.clone())),
                        replacement.scalar_bytes().len() as u64,
                    ) {
                        return removed;
                    }
                    let primary = self.arena.primary(node).unwrap();
                    let Node::Scalar(s) = self.arena.node_mut(primary) else {
                        unreachable!()
                    };
                    s.value = replacement;
                } else {
                    slot.write().string = Some(Shared::new(replacement.scalar_bytes().into()));
                    subbed = true;
                }
            } else {
                if composite.is_none() {
                    self.substitution_error(
                        node,
                        format!("cannot substitute variable without value: \"{name}\""),
                    );
                    return removed;
                }
                if slots.len() > 1 {
                    self.substitution_error(
                        node,
                        format!(
                            "cannot substitute composite variable \"{name}\" as part of a string"
                        ),
                    );
                    return removed;
                }
                match self.arena.node_mut(node) {
                    Node::Field(f) => f.primary = None,
                    Node::Edge(e) => e.primary = None,
                    _ => {}
                }
            }
            if let Some(composite) = composite {
                match self.arena.node_mut(node) {
                    Node::Field(f) => f.composite = Some(composite),
                    Node::Edge(_) => {
                        let Some(map) = self.arena.map(composite) else {
                            self.substitution_error(
                                node,
                                format!("cannot substitute array variable \"{name}\" to an edge"),
                            );
                            return removed;
                        };
                        let Node::Edge(e) = self.arena.node_mut(node) else {
                            unreachable!()
                        };
                        e.map = Some(map);
                    }
                    _ => {}
                }
            }
        }
        if subbed {
            let produced = slots
                .iter()
                .map(|slot| {
                    let slot = slot.read();
                    if slot.substitution.is_some() {
                        slot.string
                            .as_ref()
                            .map_or(0, |s| s.read().as_bytes().len() as u64)
                    } else {
                        0
                    }
                })
                .sum();
            let source = self.arena.last_ref(node).and_then(|r| r.ast());
            if !self.reserve_variable(source.as_ref(), produced) {
                return removed;
            }
            if let Some(us) = value.unquoted_string {
                us.write().coalesce()
            } else if let Some(ds) = value.double_quoted_string {
                ds.write().coalesce()
            }
        }
        removed
    }
    // Follow unresolved spread dependencies before map-copy/array-alias
    // mutation; direct child containment alone misses indirect cycles.
    fn spread_references_node(&mut self, root: NodeId, target: NodeId, vars: &[NodeId]) -> bool {
        let mut stack = vec![root];
        let mut seen = HashSet::new();
        while let Some(composite) = stack.pop() {
            if !seen.insert(composite) {
                continue;
            }
            if self.composite_contains(composite, target) {
                return true;
            }
            let mut nodes = vec![composite];
            let mut walked = HashSet::new();
            while let Some(node) = nodes.pop() {
                if !walked.insert(node) {
                    continue;
                }
                let candidate = match self.arena.node(node) {
                    Node::Field(f) => f.name.unbox().is_none(),
                    Node::Scalar(_) => self
                        .arena
                        .parent(node)
                        .is_some_and(|p| matches!(self.arena.node(p), Node::Array(_))),
                    _ => false,
                };
                if candidate
                    && let Some(primary) = self.arena.primary(node)
                    && let Node::Scalar(s) = self.arena.node(primary)
                    && let Some(unquoted) = &s.value.unquoted_string
                {
                    let slots = unquoted.read().value.clone();
                    for slot in slots.iter().flat_map(ast::SharedSlice::iter) {
                        if let Some(sub) = slot.read().substitution.clone()
                            && sub.read().spread
                        {
                            for (i, vars) in vars.iter().enumerate() {
                                if let Some(field) =
                                    self.resolve_substitution(*vars, node, &sub, i == 0)
                                {
                                    if let Node::Field(f) = self.arena.node(field)
                                        && let Some(c) = f.composite
                                    {
                                        stack.push(c);
                                    }
                                    break;
                                }
                            }
                        }
                    }
                }
                nodes.extend(super::expansion::children(&self.arena, node));
            }
        }
        false
    }
}
