// SPDX-License-Identifier: MPL-2.0
//! Compiler internals translated from pinned d2ir. The complete Compile entry
//! point remains incomplete; fields, edges, boards and lazy replay are connected.
mod arrays;
mod arrowhead_validation;
mod boards;
mod class_index;
pub use class_index::ClassLookup;
mod compile;
mod config;
mod edge_creation;
mod edge_expansion;
mod edge_validation;
mod edges;
mod expansion;
pub use expansion::VariableExpansionError;
mod fields;
mod filters;
mod glob_expansion;
mod go_sort;
mod graph;
mod graph_fields;
mod graph_materialization;
mod import_compile;
mod import_links;
pub mod import_urls;
pub mod imports;
mod late_validation;
#[cfg(test)]
mod master_internal;
mod near_validation;
mod scheduler;
mod source_graph;
mod style_colors;
pub use source_graph::{find_board_base_ast, source_graph_json, source_graph_json_with_context};
mod substitutions;
mod summary;
#[cfg(feature = "compiler-trace")]
mod trace;
pub use boards::{BoardTargetError, board_targets, board_targets_postorder, resolve_board_target};
pub use compile::{
    CompileContext, CompileError, CompileLimits, CompileOptions, UntranslatedFeature, compile_ir,
    compile_ir_with_context, compile_ir_with_imports, compile_ir_with_limits,
    compile_ir_with_options,
};
pub use config::{DiagramConfig, extract_config, extract_config_checked};
use d2_rs_core::{ast::Shared, ir::*};
pub use fields::{FieldQuery, FieldQueryResult};
pub use graph::{
    GraphCompileError, GraphMeasurer, SourceGraph, SourceLegendGraph, SourceNodeStyle,
    SourceSqlColumn, SourceSqlText, SourceTargetAccents, board_is_renderable, board_label,
    graph_edge_origins, lower_graph, lower_graph_with_measurer,
    lower_graph_with_measurer_and_labels, lower_graph_with_measurer_and_labels_theme,
    lower_graph_with_measurer_and_labels_theme_options, lower_legend_source_graph,
    lower_render_graph_with_measurer,
};
pub use imports::ImportOpener;
use std::collections::{HashMap, HashSet, VecDeque};
pub use summary::summarize_ir;
#[cfg(feature = "compiler-trace")]
pub use trace::KeyTrace;

#[derive(Default)]
pub struct Compiler {
    context: Option<CompileContext>,
    /// Original cooperative cancellation/deadline cause, ahead of source errors.
    pub context_error: Option<d2_rs_core::LayoutError>,
    glob_expansion: glob_expansion::GlobExpansionBudget,
    edge_expansion: edge_expansion::EdgeExpansion,
    variable_expansion: std::rc::Rc<expansion::ExpansionBudget>,
    variable_expansion_error: Option<String>,
    pub(super) halted: bool,
    pub(super) nil_board_base: bool,
    pub(super) parser_fault: Option<crate::parser::ParserFault>,
    pub(super) reported_composite_cycles: HashSet<usize>,
    // Graph compilation has a board-local class namespace even when an IR
    // composite is shared with a variable definition in an ancestor board.
    pub(crate) graph_class_root: Option<NodeId>,
    // Shared only by immutable graph-stage views in one lowering session.
    graph_class_index: std::rc::Rc<std::cell::RefCell<class_index::ClassIndexState>>,
    graph_field_index: std::cell::RefCell<Option<graph_fields::FieldIndex>>,
    graph_context: Option<std::rc::Rc<std::cell::RefCell<graph_materialization::GraphContext>>>,
    graph_copy_error: std::cell::RefCell<Option<GraphCompileError>>,
    graph_copy_failed: std::cell::Cell<bool>,
    graph_copy_costs: std::cell::RefCell<Vec<Option<u64>>>,
    graph_copies_accounted: std::cell::Cell<bool>,
    // Graph-stage validation of this compilation's board roots, shared by
    // every graph view lowered from it once its owner opts in.
    arrowhead_validation_memo: arrowhead_validation::ValidationMemo,
    pub arena: Arena,
    pub import_state: imports::ImportState,
    pub imports: Vec<String>,
    pub seen_imports: HashSet<String>,
    pub import_templates: HashMap<String, ImportTemplate>,
    pub(crate) import_opener: Option<imports::ImportOpener>,
    pub untranslated: Vec<UntranslatedFeature>,
    array_views: HashMap<NodeId, arrays::ArrayView>,
    #[cfg(feature = "compiler-trace")]
    pub key_trace: Vec<KeyTrace>,
    pub errors: Vec<d2_rs_core::ast::Error>,
    pub map_ref_context_stack: Vec<Shared<RefContext>>,
    pub applying_lazy_worklist: bool,
    pub lazy_glob_worklist: VecDeque<NodeId>,
    pub lazy_glob_queued: Option<HashSet<NodeId>>,
    pub lazy_settled_versions: Option<HashMap<NodeId, u64>>,
    pub lazy_post_targets: Vec<NodeId>,
    pub lazy_post_queued: Option<HashSet<NodeId>>,
    pub glob_context_stack: Vec<Vec<GlobId>>,
    pub glob_ref_context_stack: Vec<Shared<RefContext>>,
    pub lazy_glob_being_applied: bool,
    pub lazy_glob_target: Option<NodeId>,
}
impl Compiler {
    /// Let every graph lowering of this compilation reuse one validation pass
    /// per board root (CLI planning lowers a board before rendering it).
    /// Only for a compilation whose IR and errors no longer change: the
    /// results are not invalidated by later edits to `arena` or `errors`.
    pub fn share_graph_validation(&self) {
        self.arrowhead_validation_memo
            .borrow_mut()
            .get_or_insert_with(HashMap::new);
    }

    /// Work already reserved by this compilation, including graph-stage copies.
    pub fn variable_expansion_usage(&self) -> (u64, u64) {
        (
            self.variable_expansion.limit.get(),
            self.variable_expansion.used.get(),
        )
    }

    /// Reset the compilation-wide copy budget to a usage this compiler
    /// reported earlier, normally the usage when IR compilation returned.
    /// Graph-stage views charge this shared budget, so a caller that lowers
    /// one compilation more than once restores it to see a fresh compile's
    /// budget; lowering otherwise reads the IR without changing it.
    pub fn restore_variable_expansion_used(&self, used: u64) {
        self.variable_expansion.used.set(used);
    }

    /// Cost of copying one IR node and its owned slices and scalar payload.
    /// Descendant nodes are charged separately by a copy reservation.
    pub fn variable_expansion_node_units(&self, node: NodeId) -> u64 {
        expansion::node_units(&self.arena, node)
    }

    pub fn glob_contexts(&self) -> &[GlobId] {
        self.glob_context_stack
            .last()
            .expect("empty glob context stack")
    }
    pub fn get_glob_context(&self, context: &Shared<RefContext>) -> Option<GlobId> {
        self.glob_contexts().iter().copied().find(|id| {
            self.arena
                .glob(*id)
                .context
                .as_ref()
                .expect("nil glob context")
                .read()
                .equals(&context.read())
        })
    }
    pub fn ensure_glob_context(&mut self, context: Shared<RefContext>) -> GlobId {
        if let Some(id) = self.get_glob_context(&context) {
            return id;
        }
        let id = self.arena.insert_glob(GlobContext {
            context: Some(context),
            applied_fields: Some(Shared::new(HashSet::new())),
            applied_edges: Some(Shared::new(HashSet::new())),
            ..Default::default()
        });
        self.arena.glob_mut(id).root = Some(id);
        self.glob_context_stack
            .last_mut()
            .expect("empty glob context stack")
            .push(id);
        id
    }
}
