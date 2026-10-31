// SPDX-License-Identifier: MPL-2.0
//! d2compiler/compile.go:113–225 and d2ir/expansion.go:180–265.
use super::{CompileContext, Compiler, GraphCompileError, expansion};
use d2_rs_core::{LayoutError, ast, ir};

pub(super) struct GraphContext {
    check: Option<CompileContext>,
    cause: Option<LayoutError>,
}
impl GraphContext {
    pub(super) fn new(check: Option<CompileContext>) -> Self {
        Self { check, cause: None }
    }
    fn poll(&mut self) -> Option<LayoutError> {
        if self.cause.is_none() {
            self.cause = self.check.as_mut().and_then(|check| check());
        }
        self.cause.clone()
    }
}
impl Compiler {
    pub(super) fn graph_context_cause(&self) -> Option<LayoutError> {
        self.graph_context
            .as_ref()
            .and_then(|ctx| ctx.borrow_mut().poll())
    }
    pub(super) fn graph_check(&self) -> Result<(), GraphCompileError> {
        match self.graph_context_cause() {
            None => Ok(()),
            Some(LayoutError::Cancelled) => Err(GraphCompileError::Cancelled),
            Some(LayoutError::DeadlineExceeded) => Err(GraphCompileError::DeadlineExceeded),
            Some(error) => Err(GraphCompileError::ContextFailure(error.to_string())),
        }
    }
    // Charge logical occurrences before copy allocates or traverses an alias DAG.
    // Failed reservations preserve the last successful used count.
    pub(super) fn reserve_graph_copy(&mut self, root: ir::NodeId) -> Result<(), GraphCompileError> {
        expansion::walk_copy(&self.arena, root, |node| {
            self.graph_check()?;
            let units = expansion::node_units(&self.arena, node);
            self.variable_expansion.reserve(units).map_err(|_| {
                GraphCompileError::VariableExpansionLimit(self.variable_expansion.limit.get())
            })
        })
    }
    pub(super) fn has_graph_board_fields(&self, root: ir::NodeId) -> bool {
        ["layers", "scenarios", "steps"].iter().any(|name| {
            self.arena
                .get_field(root, &[ast::raw_string_box(*name, true)])
                .is_some()
        })
    }
    pub(super) fn prepare_graph_board(
        &mut self,
        root: ir::NodeId,
    ) -> Result<ir::NodeId, GraphCompileError> {
        self.graph_check()?;
        if !self.arena.root(root) || self.has_graph_board_fields(root) {
            self.reserve_graph_copy(root)?;
        }
        // Native graph lowering reads immutable IR, and graph_class_root gives
        // each board its own class namespace. Keep the original lexical parent
        // for graph IDs and document-only sorting; an orphaned arena copy would
        // incorrectly make every child board look like the document root.
        Ok(root)
    }
}
