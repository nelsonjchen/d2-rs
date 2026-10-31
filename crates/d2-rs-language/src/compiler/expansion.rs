// SPDX-License-Identifier: MPL-2.0
//! d2ir/expansion.go at master 6a7b48378. Charge logical occurrences, not
//! just unique arena handles: substitutions deliberately create shared DAGs.
use super::{CompileContext, Compiler, edge_creation::error};
use d2_rs_core::{
    LayoutError,
    ast::{self, Node as _},
    ir::*,
};
use std::{cell::Cell, collections::HashSet};

/// A failed automatic IR-copy reservation. Earlier successful node charges
/// remain consumed, and the IR itself is never modified.
#[derive(Clone, Debug)]
pub enum VariableExpansionError {
    NilMap { copy: bool },
    Limit(u64),
    Context(LayoutError),
}
impl std::fmt::Display for VariableExpansionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NilMap { copy } => write!(
                f,
                "cannot reserve variable expansion{} for a nil IR map",
                if *copy { " copy" } else { "" }
            ),
            Self::Limit(limit) => write!(
                f,
                "variable substitution expansion exceeds limit of {limit} work units"
            ),
            Self::Context(error) => error.fmt(f),
        }
    }
}
impl std::error::Error for VariableExpansionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Context(error) => Some(error),
            _ => None,
        }
    }
}

pub(super) struct ExpansionBudget {
    pub limit: Cell<u64>,
    pub used: Cell<u64>,
}
impl Default for ExpansionBudget {
    fn default() -> Self {
        Self {
            limit: Cell::new(65_536),
            used: Cell::new(0),
        }
    }
}
impl ExpansionBudget {
    pub(super) fn reserve(&self, units: u64) -> Result<(), VariableExpansionError> {
        if units > self.limit.get() - self.used.get() {
            return Err(VariableExpansionError::Limit(self.limit.get()));
        }
        self.used.set(self.used.get() + units);
        Ok(())
    }
}

/// Source-order traversal of every logical occurrence, including shared DAG
/// children. Context checks belong before each node's charge, not just at the
/// copy boundary. A budget refusal leaves prior successful charges intact.
pub(super) fn walk_copy<E>(
    arena: &Arena,
    root: NodeId,
    mut visit: impl FnMut(NodeId) -> Result<(), E>,
) -> Result<(), E> {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        visit(node)?;
        stack.extend(children(arena, node).into_iter().rev());
    }
    Ok(())
}

fn interpolation_units(values: &Option<ast::SharedSlice<ast::InterpolationBox>>) -> u64 {
    values.as_ref().map_or(0, |values| {
        values.len() as u64
            + values
                .iter()
                .map(|slot| {
                    let slot = slot.read();
                    slot.string
                        .as_ref()
                        .map_or(0, |s| s.read().as_bytes().len() as u64)
                        + slot
                            .raw_string
                            .as_ref()
                            .map_or(0, |s| s.read().as_bytes().len() as u64)
                        + slot
                            .substitution
                            .as_ref()
                            .map_or(0, |s| s.read().path.as_ref().map_or(0, |p| p.len() as u64))
                })
                .sum::<u64>()
    })
}
pub(super) fn scalar_units(value: &ast::ScalarBox) -> u64 {
    match value.unbox() {
        Some(ast::Child::Unquoted(v)) => {
            let v = v.read();
            v.pattern.as_ref().map_or(0, |p| p.len() as u64) + interpolation_units(&v.value)
        }
        Some(ast::Child::DoubleQuoted(v)) => interpolation_units(&v.read().value),
        Some(ast::Child::SingleQuoted(v)) => v.read().value.as_bytes().len() as u64,
        Some(ast::Child::Block(v)) => v.read().value.as_bytes().len() as u64,
        Some(ast::Child::Number(v)) => {
            let v = v.read();
            v.raw.len() as u64
                + v.value.as_ref().map_or(0, |n| {
                    let n = n.read();
                    n.numer().bits().div_ceil(8) + n.denom().bits().div_ceil(8)
                })
        }
        _ => 0,
    }
}
fn scalar_identity(value: &ast::ScalarBox) -> Option<usize> {
    // Addresses are used only as transient identity keys, never serialized.
    macro_rules! identity {
        ($v:expr) => {
            Some(&*$v.read() as *const _ as usize)
        };
    }
    match value.unbox()? {
        ast::Child::Unquoted(v) => identity!(v),
        ast::Child::DoubleQuoted(v) => identity!(v),
        ast::Child::SingleQuoted(v) => identity!(v),
        ast::Child::Block(v) => identity!(v),
        ast::Child::Number(v) => identity!(v),
        ast::Child::Boolean(v) => identity!(v),
        ast::Child::Null(v) => identity!(v),
        ast::Child::Suspension(v) => identity!(v),
        _ => None,
    }
}
pub(super) fn node_units(arena: &Arena, node: NodeId) -> u64 {
    1 + match arena.node(node) {
        Node::Map(m) => {
            (m.fields.as_ref().map_or(0, Vec::len) + m.edges.as_ref().map_or(0, Vec::len)) as u64
        }
        Node::Array(a) => a.values.as_ref().map_or(0, Vec::len) as u64,
        Node::Scalar(s) => scalar_units(&s.value),
        Node::Field(f) => {
            f.references.as_ref().map_or(0, Vec::len) as u64
                + scalar_units(&ast::ScalarBox::from_child(f.name.unbox()))
        }
        Node::Edge(e) => {
            e.references.as_ref().map_or(0, Vec::len) as u64
                + e.id.as_ref().map_or(0, |id| {
                    let id = id.read();
                    id.src_path
                        .iter()
                        .chain(id.dst_path.iter())
                        .map(|p| {
                            p.len() as u64
                                + p.iter()
                                    .flatten()
                                    .map(|p| {
                                        scalar_units(&ast::ScalarBox::from_child(Some(p.clone())))
                                    })
                                    .sum::<u64>()
                        })
                        .sum::<u64>()
                })
        }
    }
}
pub(super) fn children(arena: &Arena, node: NodeId) -> Vec<NodeId> {
    match arena.node(node) {
        Node::Map(m) => m
            .fields
            .iter()
            .flatten()
            .chain(m.edges.iter().flatten())
            .copied()
            .collect(),
        Node::Field(f) => f
            .primary
            .iter()
            .chain(f.composite.iter())
            .copied()
            .collect(),
        Node::Edge(e) => e.primary.iter().chain(e.map.iter()).copied().collect(),
        Node::Array(a) => a.values.clone().unwrap_or_default(),
        Node::Scalar(_) => Vec::new(),
    }
}
impl Compiler {
    /// Reserve an automatic copy of an IR map using the original compilation's
    /// substitution budget. Descendants share that budget with their root.
    /// Failure does not change IR, reset the budget, or halt later operations.
    pub fn reserve_variable_expansion_copy(
        &self,
        root: impl Into<Option<NodeId>>,
        mut context: Option<CompileContext>,
    ) -> Result<(), VariableExpansionError> {
        let root = root
            .into()
            .ok_or(VariableExpansionError::NilMap { copy: true })?;
        walk_copy(&self.arena, root, |node| {
            if let Some(error) = context.as_mut().and_then(|check| check()) {
                return Err(VariableExpansionError::Context(error));
            }
            self.variable_expansion
                .reserve(node_units(&self.arena, node))
        })
    }
    /// Account for logical occurrences reached repeatedly through IR aliases,
    /// sharing the original compilation's budget. Distinct scalar nodes that
    /// share a mutable AST value charge that value separately on repetition.
    /// A nil map is rejected before polling context; earlier charges survive
    /// cancellation or a later refusal. This operation does not change IR.
    pub fn reserve_variable_expansion_aliases(
        &self,
        root: impl Into<Option<NodeId>>,
        mut context: Option<CompileContext>,
    ) -> Result<(), VariableExpansionError> {
        let root = root
            .into()
            .ok_or(VariableExpansionError::NilMap { copy: false })?;
        let mut seen = HashSet::new();
        let mut scalars = HashSet::new();
        walk_copy(&self.arena, root, |node| {
            if let Some(error) = context.as_mut().and_then(|check| check()) {
                return Err(VariableExpansionError::Context(error));
            }
            let charge = !seen.insert(node);
            if charge {
                self.variable_expansion
                    .reserve(node_units(&self.arena, node))?;
            }
            if let Node::Scalar(s) = self.arena.node(node)
                && let Some(value) = scalar_identity(&s.value)
                && !scalars.insert(value)
                && !charge
            {
                self.variable_expansion.reserve(scalar_units(&s.value))?;
            }
            Ok(())
        })
    }
    pub(super) fn reserve_variable_ast_copy(
        &mut self,
        source: &ast::Child,
        root: ast::Child,
    ) -> bool {
        let mut stack = vec![root];
        while let Some(node) = stack.pop() {
            let extra = match &node {
                ast::Child::Map(v) => v.read().nodes.as_ref().map_or(0, |nodes| nodes.len()) as u64,
                ast::Child::Array(v) => {
                    v.read().nodes.as_ref().map_or(0, |nodes| nodes.len()) as u64
                }
                ast::Child::Key(v) => v.read().edges.as_ref().map_or(0, |edges| edges.len()) as u64,
                ast::Child::KeyPath(v) => {
                    v.read().path.as_ref().map_or(0, |path| path.len()) as u64
                }
                ast::Child::Import(v) => v.read().path.as_ref().map_or(0, |path| path.len()) as u64,
                ast::Child::Substitution(v) => {
                    v.read().path.as_ref().map_or(0, |path| path.len()) as u64
                }
                _ => scalar_units(&ast::ScalarBox::from_child(Some(node.clone()))),
            };
            if !self.reserve_variable(Some(source), 1 + extra) {
                return false;
            }
            stack.extend(node.children().into_iter().rev());
        }
        true
    }
    pub(super) fn reserve_variable(&mut self, source: Option<&ast::Child>, units: u64) -> bool {
        if self.stopped() {
            return false;
        }
        if let Err(cause) = self.variable_expansion.reserve(units) {
            let message = cause.to_string();
            self.variable_expansion_error = Some(message.clone());
            self.errors.push(source.map_or_else(
                || ast::Error {
                    message: message.clone(),
                    ..Default::default()
                },
                |s| error(s, message.clone()),
            ));
            self.halted = true;
            return false;
        }
        true
    }
    pub(super) fn reserve_variable_copy(&mut self, source: &ast::Child, root: NodeId) -> bool {
        let mut stack = vec![root];
        while let Some(node) = stack.pop() {
            if !self.reserve_variable(Some(source), node_units(&self.arena, node)) {
                return false;
            }
            stack.extend(children(&self.arena, node).into_iter().rev());
        }
        true
    }
    pub(super) fn visit_substitution_node(
        &mut self,
        node: NodeId,
        source: Option<&ast::Child>,
        seen: &mut HashSet<NodeId>,
    ) -> bool {
        if self.stopped() {
            return false;
        }
        seen.insert(node) || self.reserve_variable(source, node_units(&self.arena, node))
    }
    pub(super) fn reserve_variable_aliases(&mut self, root: NodeId) {
        let mut stack = vec![root];
        let mut seen = HashSet::new();
        let mut scalars = HashSet::new();
        while let Some(node) = stack.pop() {
            let charge = !seen.insert(node);
            if charge && !self.reserve_variable(None, node_units(&self.arena, node)) {
                return;
            }
            if let Node::Scalar(s) = self.arena.node(node)
                && let Some(value) = scalar_identity(&s.value)
            {
                if !scalars.insert(value) {
                    if !charge && !self.reserve_variable(None, scalar_units(&s.value)) {
                        return;
                    }
                }
            }
            stack.extend(children(&self.arena, node).into_iter().rev());
        }
    }
    pub(super) fn composite_contains(&self, root: NodeId, target: NodeId) -> bool {
        let mut stack = vec![root];
        let mut seen = HashSet::new();
        while let Some(node) = stack.pop() {
            if !seen.insert(node) {
                continue;
            }
            if node == target {
                return true;
            }
            stack.extend(children(&self.arena, node));
        }
        false
    }
}
