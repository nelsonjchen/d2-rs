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
    /// Validate graph-stage copies and class applications across the complete
    /// board tree. Shares the IR compilation's expansion budget, as Go's
    /// d2compiler.Compile does before handing any graphs to layout.
    pub fn validate_graph_materialization(
        &self,
        root: ir::NodeId,
    ) -> Result<(), GraphCompileError> {
        let _session = self.graph_field_session();
        super::arrowhead_validation::validate_graph_materialization(self, root)
    }

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
    pub(super) fn graph_copy_check(&self) -> Result<(), GraphCompileError> {
        if self.graph_copy_failed.get()
            && let Some(error) = self.graph_copy_error.borrow().as_ref()
        {
            return Err(error.clone());
        }
        Ok(())
    }
    pub(super) fn fail_graph_copy(&self, error: GraphCompileError) {
        *self.graph_copy_error.borrow_mut() = Some(error);
        self.graph_copy_failed.set(true);
    }
    // Charge logical occurrences before copy allocates or traverses an alias DAG.
    // Failed reservations preserve the last successful used count.
    pub(super) fn reserve_graph_copy(&self, root: ir::NodeId) -> Result<(), GraphCompileError> {
        self.graph_copy_check()?;
        // Views read immutable IR. Repeated applications of a class have the
        // same logical copy cost; avoid allocating a traversal stack each time.
        // Context-bearing walks still poll every node. A refusal also walks
        // normally, retaining Go's exact partial charges before the failing node.
        let cacheable = self.graph_context.is_none();
        if cacheable {
            self.graph_check()?;
            let cost = self
                .graph_copy_costs
                .borrow()
                .get(root.0)
                .copied()
                .flatten();
            if let Some(cost) = cost
                && cost <= self.variable_expansion.limit.get() - self.variable_expansion.used.get()
            {
                return self.variable_expansion.reserve(cost).map_err(|_| {
                    GraphCompileError::VariableExpansionLimit(self.variable_expansion.limit.get())
                });
            }
        }
        let mut cost = 0;
        expansion::walk_copy(&self.arena, root, |node| {
            self.graph_check()?;
            let units = expansion::node_units(&self.arena, node);
            cost += units;
            self.variable_expansion.reserve(units).map_err(|_| {
                GraphCompileError::VariableExpansionLimit(self.variable_expansion.limit.get())
            })
        })?;
        if cacheable {
            let mut costs = self.graph_copy_costs.borrow_mut();
            if costs.len() <= root.0 {
                costs.resize(root.0 + 1, None);
            }
            costs[root.0] = Some(cost);
        }
        Ok(())
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
        self.graph_copy_check()?;
        self.graph_check()?;
        // d2compiler.compileBoardsField substitutes an empty map for a board
        // field without a map. Do this in the graph view so the public IR keeps
        // the authored primary value and absent composite unchanged.
        if matches!(self.arena.node(root), ir::Node::Field(_)) && self.arena.map(root).is_none() {
            return Ok(self.arena.new_root());
        }
        // Native graph lowering reads immutable IR, and graph_class_root gives
        // each board its own class namespace. Keep the original lexical parent
        // for graph IDs and document-only sorting; an orphaned arena copy would
        // incorrectly make every child board look like the document root.
        Ok(root)
    }
}
