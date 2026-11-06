// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Layout orchestration: a Rust port of upstream D2's `d2layouts` package. It
//! selects the layout engine (TALA, Dagre or ELK, all in-process) and owns grid
//! layouts, sequence diagrams and `near` placement; the engines live in their
//! own crates.
//!
//! The `diagnostic-traces` feature enables engine traces; `oracle-fixtures`
//! compiles tests that read regenerated Go-oracle artifacts.

#![warn(missing_docs)]

#[cfg(test)]
mod admission_api_tests;
mod root_state;

use d2_rs_core::{
    CancellationToken, CanvasPosition, EdgeId, Graph, Insets, LabelPosition,
    LayoutError as CoreLayoutError, LayoutOptions, LayoutReport, LayoutResult, NodeId, Point, Rect,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::str::FromStr;
use thiserror::Error;

#[inline(always)]
#[allow(dead_code)]
pub(crate) fn diagnostic_env_enabled(name: &str) -> bool {
    #[cfg(feature = "diagnostic-traces")]
    {
        std::env::var_os(name).is_some()
    }
    #[cfg(not(feature = "diagnostic-traces"))]
    {
        let _ = name;
        false
    }
}

#[cfg(feature = "diagnostic-traces")]
thread_local! {
    static DIAGNOSTIC_LAYOUT_DEPTH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(feature = "diagnostic-traces")]
struct DiagnosticLayoutScope;

#[cfg(feature = "diagnostic-traces")]
impl DiagnosticLayoutScope {
    fn enter() -> Self {
        DIAGNOSTIC_LAYOUT_DEPTH.with(|depth| depth.set(depth.get() + 1));
        Self
    }

    fn is_outermost(&self) -> bool {
        DIAGNOSTIC_LAYOUT_DEPTH.with(|depth| depth.get() == 1)
    }
}

#[cfg(feature = "diagnostic-traces")]
impl Drop for DiagnosticLayoutScope {
    fn drop(&mut self) {
        DIAGNOSTIC_LAYOUT_DEPTH.with(|depth| depth.set(depth.get() - 1));
    }
}

pub mod grid;
pub mod near;
pub mod sequence;

/// Built-in layout engine names accepted by the D2 layout boundary.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum EngineKind {
    /// TALA, D2's general-purpose layout engine.
    Tala,
    /// Dagre graph layout.
    Dagre,
    /// ELK graph layout.
    Elk,
}

impl EngineKind {
    /// Returns the canonical D2 spelling for this engine.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Tala => "tala",
            Self::Dagre => "dagre",
            Self::Elk => "elk",
        }
    }
}

impl fmt::Display for EngineKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for EngineKind {
    type Err = UnknownEngine;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "tala" | "tala-rs" => Ok(Self::Tala),
            "dagre" => Ok(Self::Dagre),
            "elk" => Ok(Self::Elk),
            _ => Err(UnknownEngine(value.to_owned())),
        }
    }
}

/// An engine spelling that D2's selector does not recognize.
#[derive(Clone, Debug, Eq, Error, Hash, PartialEq)]
#[error("unknown layout engine {0:?}")]
pub struct UnknownEngine(pub String);

/// Error returned by the common layout boundary.
#[derive(Debug, Error)]
pub enum LayoutDispatchError {
    /// The selector received an engine name that is not recognized.
    #[error("layout engine {engine} is not implemented in d2-rs")]
    UnsupportedEngine {
        /// Canonical engine name.
        engine: EngineKind,
    },
    /// TALA rejected the measured graph or could not finish.
    #[error(transparent)]
    Tala(#[from] CoreLayoutError),
    /// ELK baseline rejected the measured graph or could not finish.
    #[error(transparent)]
    Elk(CoreLayoutError),
    /// A translated scope contains repeated D2 identities.
    #[error("D2 {kind} ID {identity:?} is repeated")]
    RepeatedIdentity {
        /// Object or edge identity namespace.
        kind: &'static str,
        /// Repeated absolute identity.
        identity: String,
    },
    /// The caller cancelled a dispatch before a complete result was ready.
    #[error("context canceled")]
    Cancelled,
}

impl LayoutDispatchError {
    /// Nested LayoutNested calls return the engine's original error directly.
    pub(crate) fn into_core_error(self) -> CoreLayoutError {
        match self {
            Self::Tala(error) | Self::Elk(error) => error,
            Self::Cancelled => CoreLayoutError::Cancelled,
            other => CoreLayoutError::BadRouteState(other.to_string()),
        }
    }
}

/// Common diagram category used by the orchestration layer.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum DiagramType {
    /// Ordinary graph layout.
    #[default]
    Default,
    /// A graph whose nodes are positioned relative to another object.
    ConstantNear,
    /// Explicit row/column grid layout.
    Grid,
    /// Sequence-diagram layout.
    Sequence,
}

/// Classification metadata passed from the D2 graph compiler to orchestration.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct GraphInfo {
    /// Whether this scope is a constant-near graph.
    pub is_constant_near: bool,
    /// Specialized layout category, when one was selected.
    pub diagram_type: DiagramType,
}

impl GraphInfo {
    /// Returns whether this is an ordinary, non-specialized graph scope.
    pub const fn is_default(self) -> bool {
        !self.is_constant_near && matches!(self.diagram_type, DiagramType::Default)
    }
}

/// Layout and routing entry point selected by [`EngineKind`].
#[derive(Clone, Copy, Debug, Default)]
pub struct LayoutDispatcher;

/// Results produced while D2's nested-scope queue is walked. The cache lets
/// the later geometry-assembly passes reuse those exact local layouts instead
/// of invoking an engine a second time in a different order.
#[derive(Clone)]
struct CachedLayoutResult {
    node_ids: Vec<String>,
    edge_ids: Vec<String>,
    result: LayoutResult,
}

#[derive(Default)]
pub(crate) struct DagreScopeCache {
    constant_nears: BTreeMap<String, CachedLayoutResult>,
    grid_cells: BTreeMap<String, CachedLayoutResult>,
    sequences: BTreeMap<String, CachedLayoutResult>,
}

fn cache_layout_result(graph: &Graph, result: &LayoutResult) -> CachedLayoutResult {
    CachedLayoutResult {
        node_ids: graph
            .nodes()
            .map(|(_, node)| node.external_id.clone())
            .collect(),
        edge_ids: stable_edge_identities(graph),
        result: result.clone(),
    }
}

fn restore_layout_result(graph: &Graph, cached: &CachedLayoutResult) -> Option<LayoutResult> {
    let node_ids = graph
        .nodes()
        .map(|(id, node)| (node.external_id.clone(), id))
        .collect::<BTreeMap<_, _>>();
    let edge_ids = stable_edge_identities(graph)
        .into_iter()
        .enumerate()
        .map(|(index, identity)| (identity, EdgeId(index as u32)))
        .collect::<BTreeMap<_, _>>();
    let mut result = cached.result.clone();
    result.boxes = cached
        .result
        .boxes
        .iter()
        .filter_map(|(id, rect)| {
            node_ids
                .get(cached.node_ids.get(id.0 as usize)?)
                .copied()
                .map(|target| (target, *rect))
        })
        .collect();
    result.node_labels = cached
        .result
        .node_labels
        .iter()
        .filter_map(|(id, label)| {
            node_ids
                .get(cached.node_ids.get(id.0 as usize)?)
                .copied()
                .map(|target| (target, *label))
        })
        .collect();
    result.sequence_lifelines = cached
        .result
        .sequence_lifelines
        .iter()
        .filter_map(|line| {
            let mut line = line.clone();
            line.actor = *node_ids.get(cached.node_ids.get(line.actor.0 as usize)?)?;
            Some(line)
        })
        .collect();
    result.routes = cached
        .result
        .routes
        .iter()
        .filter_map(|(id, route)| {
            edge_ids
                .get(cached.edge_ids.get(id.0 as usize)?)
                .copied()
                .map(|target| (target, route.clone()))
        })
        .collect();
    result.route_curves = cached
        .result
        .route_curves
        .iter()
        .filter_map(|(id, kind)| {
            edge_ids
                .get(cached.edge_ids.get(id.0 as usize)?)
                .copied()
                .map(|target| (target, *kind))
        })
        .collect();
    result.edge_labels = cached
        .result
        .edge_labels
        .iter()
        .filter_map(|(id, label)| {
            edge_ids
                .get(cached.edge_ids.get(id.0 as usize)?)
                .cloned()
                .map(|target| (target, label.clone()))
        })
        .collect();
    result.removed_edges = cached
        .result
        .removed_edges
        .iter()
        .filter_map(|id| edge_ids.get(cached.edge_ids.get(id.0 as usize)?).copied())
        .collect();
    result.edge_label_percentage_commits = cached
        .result
        .edge_label_percentage_commits
        .iter()
        .filter_map(|id| edge_ids.get(cached.edge_ids.get(id.0 as usize)?).copied())
        .collect();
    Some(result)
}

fn stable_edge_identities(graph: &Graph) -> Vec<String> {
    let mut ordinals = BTreeMap::<(String, String), usize>::new();
    graph
        .edges()
        .map(|(_, edge)| {
            let source = graph
                .node(edge.source)
                .map(|node| node.external_id.as_str())
                .unwrap_or("<invalid-source>");
            let target = graph
                .node(edge.target)
                .map(|node| node.external_id.as_str())
                .unwrap_or("<invalid-target>");
            let pair = (source.to_owned(), target.to_owned());
            let ordinal = ordinals.entry(pair.clone()).or_default();
            let identity = format!("{}\0{}\0{}", pair.0, pair.1, *ordinal);
            *ordinal += 1;
            identity
        })
        .collect()
}

impl LayoutDispatcher {
    /// Lays out a measured graph using the selected in-process engine.
    pub fn layout(
        &self,
        graph: &Graph,
        engine: EngineKind,
        options: &LayoutOptions,
    ) -> Result<LayoutResult, LayoutDispatchError> {
        self.layout_with_cancellation(graph, engine, options, None)
    }

    /// Applies the d2lib.compileBoard boundary: empty source boards bypass
    /// layout entirely, including engine options and cancellation checks.
    /// Direct engine callers should use `layout_with_cancellation` instead.
    pub fn layout_compiled_with_cancellation(
        &self,
        graph: &Graph,
        engine: EngineKind,
        options: &LayoutOptions,
        cancellation: Option<&CancellationToken>,
    ) -> Result<LayoutResult, LayoutDispatchError> {
        // LayoutNested dispatches core layout only when Graph.Objects is
        // nonempty. Structured roots can retain root-root connections after
        // removing every ordinary object; these still bypass engine/context
        // and seed processing at the compiled-board boundary.
        if graph.nodes.is_empty() {
            return Ok(empty_layout_result(options));
        }
        self.layout_with_cancellation(graph, engine, options, cancellation)
    }

    /// Lays out a graph while observing an optional caller-owned cancellation
    /// token. Context is forwarded to actual core invocations: TALA observes
    /// it throughout its pipeline; Dagre and ELK check before solver entry.
    /// Specialized grid/sequence algorithms retain their separate boundaries.
    pub fn layout_with_cancellation(
        &self,
        graph: &Graph,
        engine: EngineKind,
        options: &LayoutOptions,
        cancellation: Option<&CancellationToken>,
    ) -> Result<LayoutResult, LayoutDispatchError> {
        #[cfg(feature = "diagnostic-traces")]
        let diagnostic_scope = DiagnosticLayoutScope::enter();
        let mut result =
            self.layout_with_cancellation_in_scope(graph, engine, options, cancellation, false)?;
        validate_object_positions(graph, &result)?;
        order_lifelines_like_layout_nested(graph, &mut result);
        #[cfg(feature = "diagnostic-traces")]
        if diagnostic_graph_has_grid(graph)
            || (diagnostic_scope.is_outermost() && diagnostic_env_enabled("D2_TRACE_LAYOUT_FINAL"))
        {
            grid::trace_grid_snapshot(graph, &result, None, "layout-final");
        }
        Ok(result)
    }

    /// Lays out a graph extracted as a D2 `GridDiagram` scope. D2 dispatches
    /// such scopes directly to d2grid, so their outer grid node is not passed
    /// through the selected core layout engine as a placeholder.
    pub(crate) fn layout_grid_diagram(
        &self,
        graph: &Graph,
        engine: EngineKind,
        options: &LayoutOptions,
        cancellation: Option<&CancellationToken>,
    ) -> Result<LayoutResult, LayoutDispatchError> {
        self.layout_with_cancellation_in_scope(graph, engine, options, cancellation, true)
    }

    fn layout_with_cancellation_in_scope(
        &self,
        graph: &Graph,
        engine: EngineKind,
        options: &LayoutOptions,
        cancellation: Option<&CancellationToken>,
        grid_diagram_root: bool,
    ) -> Result<LayoutResult, LayoutDispatchError> {
        self.layout_with_scope_cache(
            graph,
            engine,
            options,
            cancellation,
            grid_diagram_root,
            None,
        )
    }

    fn layout_with_scope_cache(
        &self,
        graph: &Graph,
        engine: EngineKind,
        options: &LayoutOptions,
        cancellation: Option<&CancellationToken>,
        grid_diagram_root: bool,
        scope_cache: Option<&mut DagreScopeCache>,
    ) -> Result<LayoutResult, LayoutDispatchError> {
        self.layout_with_scope_cache_deferred(
            graph,
            engine,
            options,
            cancellation,
            grid_diagram_root,
            scope_cache,
            false,
        )
    }

    fn layout_with_scope_cache_deferred(
        &self,
        graph: &Graph,
        engine: EngineKind,
        options: &LayoutOptions,
        cancellation: Option<&CancellationToken>,
        grid_diagram_root: bool,
        mut scope_cache: Option<&mut DagreScopeCache>,
        defer_parent_routing: bool,
    ) -> Result<LayoutResult, LayoutDispatchError> {
        // Source lowering carries the fixed top-label band in a package's
        // content insets so the other layout adapters can recover D2's
        // measured container padding. TALA owns package geometry itself and
        // derives that band from the shape and label; passing the recovered
        // band through counts it a second time (19 px on a 36 px top label).
        // Cylinder carriers likewise include two top arcs and one bottom
        // arc for the other adapters. TALA's shape fitting owns those 72
        // units; its original D2 adapter supplies no explicit padding.
        // Both shapes therefore start at uniform 60 px in this adapter.
        // Grid cylinders use d2grid fitting and retain their carrier insets.
        let normalized_tala_graph = if matches!(&engine, EngineKind::Tala) {
            let mut normalized = graph.clone();
            for (index, node) in graph.nodes() {
                let tala_cylinder = node.shape == d2_rs_core::ShapeKind::Cylinder
                    && node.grid_rows.is_none()
                    && node.grid_columns.is_none();
                if node.shape == d2_rs_core::ShapeKind::Package || tala_cylinder {
                    normalized.node_mut(index).unwrap().content_insets = Insets::uniform(60.0);
                }
            }
            // The same adapter traversal applies to full core layout and
            // standalone routing. Preserve arena IDs while supplying Go's
            // depth-first ChildrenArray visit order to the parent engine.
            normalized.set_node_order_override(routing_node_order(&normalized));
            Some(normalized)
        } else {
            None
        };
        let graph = normalized_tala_graph.as_ref().unwrap_or(graph);
        // D2 removes constant-near graphs from the core layout input and
        // injects them after the ordinary graph has been positioned. Keeping
        // those objects in the ELK/TALA input changes the root dimensions and
        // therefore every unrelated coordinate. Do this at the shared
        // boundary so all in-process engines see the same source semantics.
        if graph
            .nodes()
            .any(|(_, node)| node.canvas_position.is_some())
        {
            if let Some(cache) = scope_cache.as_deref_mut() {
                return self.layout_constant_nears(graph, engine, options, cancellation, cache);
            }
            let mut local_cache = DagreScopeCache::default();
            return self.layout_constant_nears(
                graph,
                engine,
                options,
                cancellation,
                &mut local_cache,
            );
        }
        // The grid adapter owns one flat graph scope.  Source lowering keeps
        // the document root implicit, and nested/multi-root documents can
        // therefore contain grid attributes that belong to a different
        // scope.  Dispatching those graphs to the flat adapter would leave
        // descendants without boxes (and used to panic while routing an
        // unrelated edge).  Let the general TALA pipeline handle them until
        // their per-scope grid orchestration is translated.
        let flat_grid_scope = graph
            .nodes()
            .filter(|(_, node)| node.parent.is_none())
            .collect::<Vec<_>>();
        let synthetic_flat_grid = (graph.root_grid_rows.is_some()
            || graph.root_grid_columns.is_some())
            && !graph.root_sequence_diagram
            && !flat_grid_scope.is_empty()
            && graph.nodes().all(|(_, node)| node.parent.is_none());
        let object_flat_grid = flat_grid_scope.first().is_some_and(|(root_id, root)| {
            flat_grid_scope.len() == 1
                && !graph.is_sequence_diagram(*root_id)
                && (root.grid_rows.is_some() || root.grid_columns.is_some())
                && graph
                    .nodes()
                    .all(|(id, node)| id == *root_id || node.parent == Some(*root_id))
        });
        // A top-level object grid still leaves its fitted placeholder in
        // the containing default graph, which must invoke the core engine.
        let use_grid_adapter = synthetic_flat_grid || (grid_diagram_root && object_flat_grid);
        if use_grid_adapter {
            let result = grid::layout(graph, options).map_err(LayoutDispatchError::Tala)?;
            return Ok(result);
        }
        // An implicit document-root grid is dispatched by d2grid before the
        // selected core engine in v0.9.0. That remains true when a cell owns
        // a nested grid or an ordinary container: LayoutNested lays out those
        // extracted cell contents with the selected engine, then d2grid
        // positions the root cells. Running TALA over the collapsed root
        // graph first moves those already-positioned cells a second time.
        let nested_root_grid = !grid_diagram_root
            && !graph.root_sequence_diagram
            && (graph.root_grid_rows.is_some() || graph.root_grid_columns.is_some())
            && graph.nodes().any(|(_, node)| node.parent.is_some());
        if nested_root_grid {
            let result = layout_root_grid_with_nested_cells(graph, options, engine, cancellation)?;
            return Ok(result);
        }
        // LayoutNested dispatches diagram type before choosing the core
        // engine. Sequence roots own their layout under every core engine;
        // that engine is used only for extracted ordinary actor subgraphs.
        if graph.root_sequence_diagram && !graph.nodes.is_empty() {
            let result = sequence::layout_with_nested_and_cancellation(
                graph,
                options,
                engine.clone(),
                cancellation,
            )
            .map_err(|error| match engine {
                EngineKind::Elk => LayoutDispatchError::Elk(error),
                _ => LayoutDispatchError::Tala(error),
            })?;
            return Ok(result);
        }
        match engine {
            EngineKind::Tala => {
                // D2's LayoutNested sizes sequence scopes before laying out
                // the containing graph. Keep their descendants and internal
                // messages out of the parent TALA pass, while retaining each
                // completed sequence box as the parent's measured placeholder.
                let sequence_projection = sequence::project_for_tala_with_cache(
                    graph,
                    options,
                    scope_cache.as_deref_mut(),
                    cancellation,
                )
                .map_err(LayoutDispatchError::Tala)?;
                let layout_graph = sequence_projection
                    .as_ref()
                    .map_or(graph, |projection| &projection.graph);
                if has_nested_grid_scope(graph) {
                    let mut result = layout_tala_with_nested_grids(
                        layout_graph,
                        options,
                        cancellation,
                        defer_parent_routing,
                    )?;
                    if let Some(projection) = sequence_projection {
                        result = remap_result(result, &projection.node_map, &projection.edge_map);
                        result =
                            sequence::apply_projection_locals(graph, result, projection.locals);
                        if !defer_parent_routing {
                            reroute_sequence_external_edges(
                                graph,
                                &mut result,
                                options,
                                EngineKind::Tala,
                                cancellation,
                            )?;
                        }
                    }
                    return Ok(result);
                }
                let mut result = match cancellation {
                    Some(token) => d2_rs_tala::Engine
                        .layout_with_cancellation(layout_graph, options, token)
                        .map_err(LayoutDispatchError::Tala)?,
                    None => d2_rs_tala::Engine
                        .layout(layout_graph, options)
                        .map_err(LayoutDispatchError::Tala)?,
                };
                if let Some(projection) = sequence_projection {
                    result = remap_result(result, &projection.node_map, &projection.edge_map);
                    result = sequence::apply_projection_locals(graph, result, projection.locals);
                    if !defer_parent_routing {
                        reroute_sequence_external_edges(
                            graph,
                            &mut result,
                            options,
                            EngineKind::Tala,
                            cancellation,
                        )?;
                    }
                }
                // A flat root grid is handled by the dedicated adapter above.
                // Apply the scope pass when object-level grids are present;
                // it walks those scopes deepest-first and preserves each
                // nested container as one component for its parent.
                if graph.root_grid_rows.is_some()
                    || graph.root_grid_columns.is_some()
                    || graph
                        .nodes()
                        .any(|(_, node)| node.grid_rows.is_some() || node.grid_columns.is_some())
                {
                    result = grid::apply_scopes(graph, result, options)
                        .map_err(LayoutDispatchError::Tala)?;
                    reroute_nested_grid_external_edges(graph, &mut result, options, cancellation)?;
                }
                if graph.nodes().any(|(id, _)| graph.is_sequence_diagram(id)) {
                    result = sequence::apply_nested_scopes_with_cancellation(
                        graph,
                        result,
                        options,
                        EngineKind::Tala,
                        cancellation,
                    )
                    .map_err(LayoutDispatchError::Tala)?;
                }
                Ok(result)
            }
            EngineKind::Dagre => {
                // A diagram whose implicit document root is a grid never
                // enters d2dagrelayout in v0.9.0. It is handled entirely by
                // d2grid, regardless of the selected core engine. LayoutNested
                // still lays out ordinary container cells and recursively
                // resolves object-level grid diagrams before the root grid
                // measures those cells. Keep the flat fast path for diagrams
                // without either kind of nested scope.
                if !graph.root_sequence_diagram
                    && (graph.root_grid_rows.is_some() || graph.root_grid_columns.is_some())
                {
                    let has_nested_cells = graph
                        .nodes()
                        .any(|(id, _)| is_grid_cell_container(graph, id));
                    let has_object_grid = graph
                        .nodes()
                        .any(|(_, node)| node.grid_rows.is_some() || node.grid_columns.is_some());
                    let result = if has_nested_cells || has_object_grid {
                        layout_root_grid_with_nested_cells(
                            graph,
                            options,
                            EngineKind::Dagre,
                            cancellation,
                        )?
                    } else {
                        grid::layout(graph, options).map_err(LayoutDispatchError::Tala)?
                    };
                    return Ok(result);
                }
                let sequence_projection = if diagnostic_env_enabled("DAGRE_SKIP_SEQUENCE") {
                    None
                } else {
                    sequence::project_for_dagre_with_cache(
                        graph,
                        options,
                        scope_cache.as_deref_mut(),
                        cancellation,
                    )
                    .map_err(LayoutDispatchError::Tala)?
                };
                let layout_graph = sequence_projection
                    .as_ref()
                    .map_or(graph, |projection| &projection.graph);
                // D2's LayoutNested resolves nested grids and their measured
                // dimensions before invoking Dagre on the extracted parent.
                // Seed the shared graph with measured boxes and the adapter's
                // label state; layout_nested_grid_cells and the collapsed
                // parent pass below then perform only source-equivalent Dagro
                // invocations. A flattened provisional layout here adds a
                // Rust-only pass and can choose different label state.
                // Childless grids are not extracted by LayoutNested; they
                // still need the ordinary Dagre pass for their parent graph.
                let mut result = if has_nested_grid_scope(graph) {
                    let mut result = empty_layout_result(options);
                    for (id, node) in layout_graph.nodes() {
                        result.boxes.insert(
                            id,
                            Rect {
                                origin: Point::default(),
                                size: node.size,
                            },
                        );
                    }
                    result.node_labels = d2_rs_dagre::initial_node_label_states(layout_graph);
                    result.edge_labels = d2_rs_dagre::initial_edge_labels(layout_graph);
                    Ok(result)
                } else {
                    d2_rs_dagre::layout_with_options_and_rank_trace_with_cancellation(
                        layout_graph,
                        options,
                        d2_rs_dagre::DagreOptions::default(),
                        !has_nested_grid_scope(graph),
                        cancellation,
                    )
                }
                .map_err(LayoutDispatchError::Tala)?;
                if let Some(projection) = sequence_projection {
                    result = remap_result(result, &projection.node_map, &projection.edge_map);
                    result =
                        sequence::apply_dagre_projection_locals(graph, result, projection.locals);
                }
                if has_grid_scope(graph) {
                    result = layout_nested_grid_cells(
                        graph,
                        result,
                        options,
                        scope_cache.take(),
                        EngineKind::Dagre,
                        cancellation,
                    )?;
                }
                if has_grid_scope(graph) {
                    result = grid::apply_scopes(graph, result, options)
                        .map_err(LayoutDispatchError::Tala)?;
                    reroute_grid_scope_external_edges(
                        graph,
                        &mut result,
                        options,
                        cancellation,
                        false,
                        EngineKind::Dagre,
                    )?;
                    #[cfg(feature = "diagnostic-traces")]
                    if diagnostic_env_enabled("DAGRE_DEBUG_NESTED") {
                        eprintln!("AFTER_GRID_ROUTES {:?}", result.routes);
                    }
                }
                // Sequence scopes are laid out by their dedicated source
                // adapter after the enclosing Dagre graph establishes the
                // scope rectangle.  Keep this pass in the Dagre path too;
                // mixed documents can select Dagre while containing nested
                // sequence diagrams.
                if graph.nodes().any(|(id, _)| graph.is_sequence_diagram(id))
                    && !diagnostic_env_enabled("DAGRE_SKIP_SEQUENCE")
                {
                    result = sequence::apply_nested_scopes_with_cancellation(
                        graph,
                        result,
                        options,
                        EngineKind::Dagre,
                        cancellation,
                    )
                    .map_err(LayoutDispatchError::Tala)?;
                }
                if has_nested_grid_scope(graph) && !grid_diagram_root {
                    result = collapse_dagre_nested_grids(graph, result, options, cancellation)
                        .map_err(LayoutDispatchError::Tala)?;
                }
                reroute_sequence_external_edges(
                    graph,
                    &mut result,
                    options,
                    EngineKind::Dagre,
                    cancellation,
                )?;
                Ok(result)
            }
            EngineKind::Elk => {
                let sequence_projection = sequence::project_for_elk_with_cache(
                    graph,
                    options,
                    scope_cache.as_deref_mut(),
                    cancellation,
                )
                .map_err(LayoutDispatchError::Elk)?;
                let layout_graph = sequence_projection
                    .as_ref()
                    .map_or(graph, |projection| &projection.graph);
                let mut result = if has_nested_grid_scope(layout_graph) {
                    layout_elk_with_nested_grids(
                        layout_graph,
                        options,
                        scope_cache.take(),
                        cancellation,
                        grid_diagram_root,
                    )?
                } else {
                    let mut result =
                        d2_rs_elk::layout_with_cancellation(layout_graph, options, cancellation)
                            .map_err(LayoutDispatchError::Elk)?;
                    if has_grid_scope(layout_graph) {
                        result = grid::apply_scopes(layout_graph, result, options)
                            .map_err(LayoutDispatchError::Elk)?;
                    }
                    result
                };
                if let Some(projection) = sequence_projection {
                    result = remap_result(result, &projection.node_map, &projection.edge_map);
                    result = sequence::apply_projection_locals(graph, result, projection.locals);
                    if !defer_parent_routing {
                        reroute_sequence_external_edges(
                            graph,
                            &mut result,
                            options,
                            EngineKind::Elk,
                            cancellation,
                        )?;
                    }
                }
                Ok(result)
            }
        }
    }

    fn layout_constant_nears(
        &self,
        graph: &Graph,
        engine: EngineKind,
        options: &LayoutOptions,
        cancellation: Option<&CancellationToken>,
        scope_cache: &mut DagreScopeCache,
    ) -> Result<LayoutResult, LayoutDispatchError> {
        let mut roots = Vec::new();
        let constant_nodes = graph
            .nodes()
            .filter_map(|(id, node)| node.canvas_position.map(|_| id))
            .collect::<BTreeSet<_>>();
        for id in constant_nodes.iter().copied() {
            let mut parent = graph.node(id).and_then(|node| node.parent);
            let mut nested = false;
            while let Some(parent_id) = parent {
                if constant_nodes.contains(&parent_id) {
                    nested = true;
                    break;
                }
                parent = graph.node(parent_id).and_then(|node| node.parent);
            }
            if !nested {
                roots.push(id);
            }
        }

        let mut groups = Vec::with_capacity(roots.len());
        let mut extracted = BTreeSet::new();
        for root in roots {
            let members = graph
                .nodes()
                .filter_map(|(id, _)| {
                    let mut cursor = Some(id);
                    while let Some(current) = cursor {
                        if current == root {
                            return Some(id);
                        }
                        cursor = graph.node(current).and_then(|node| node.parent);
                    }
                    None
                })
                .collect::<Vec<_>>();
            extracted.extend(members.iter().copied());
            groups.push((root, members));
        }

        if groups.is_empty() {
            return self.layout_without_constant_nears(graph, engine, options, cancellation);
        }

        let base_ids = graph
            .nodes()
            .filter_map(|(id, _)| (!extracted.contains(&id)).then_some(id))
            .collect::<Vec<_>>();
        let (base_graph, base_nodes, base_edges) = project_graph(graph, &base_ids, false);

        // D2's LayoutNested queue interleaves top-level constant-near, grid,
        // and sequence scopes in hierarchy order. The result-assembly code
        // below is intentionally kept separate, so precompute the source
        // queue's nested layouts here and let those later passes reuse them.
        let scope_plan = nested_dagre_scope_plan(graph);
        if matches!(engine, EngineKind::Dagre)
            && !groups.is_empty()
            && scope_plan.iter().any(|scope| {
                matches!(
                    scope,
                    NestedDagreScope::Grid(_) | NestedDagreScope::Sequence(_)
                )
            })
        {
            for scope in scope_plan {
                match scope {
                    NestedDagreScope::ConstantNear(root) => {
                        let Some((_, members)) = groups.iter().find(|(id, _)| *id == root) else {
                            continue;
                        };
                        let (mut nested_graph, nested_nodes, _) =
                            project_graph(graph, members, true);
                        let root_index = nested_nodes
                            .iter()
                            .position(|id| *id == root)
                            .map(|index| NodeId(index as u32))
                            .ok_or(LayoutDispatchError::Elk(CoreLayoutError::IdOverflow))?;
                        if let Some(root_node) = nested_graph.node_mut(root_index) {
                            root_node.canvas_position = None;
                        }
                        let nested_result = self.layout_with_scope_cache(
                            &nested_graph,
                            engine.clone(),
                            options,
                            cancellation,
                            false,
                            Some(scope_cache),
                        )?;
                        if let Some(root_node) = graph.node(root) {
                            scope_cache.constant_nears.insert(
                                root_node.external_id.clone(),
                                cache_layout_result(&nested_graph, &nested_result),
                            );
                        }
                    }
                    NestedDagreScope::Grid(root) => {
                        let members = graph
                            .nodes()
                            .filter_map(|(id, _)| node_in_scope(graph, id, root).then_some(id))
                            .collect::<Vec<_>>();
                        let (nested_graph, _, _) = project_graph(graph, &members, false);
                        let _ = self.layout_with_scope_cache(
                            &nested_graph,
                            engine.clone(),
                            options,
                            cancellation,
                            true,
                            Some(scope_cache),
                        )?;
                    }
                    NestedDagreScope::Sequence(root) => {
                        let members = graph
                            .nodes()
                            .filter_map(|(id, _)| node_in_scope(graph, id, root).then_some(id))
                            .collect::<Vec<_>>();
                        let (nested_graph, nested_nodes, _) = project_graph(graph, &members, false);
                        let root_index = nested_nodes
                            .iter()
                            .position(|id| *id == root)
                            .map(|index| NodeId(index as u32))
                            .ok_or(LayoutDispatchError::Elk(CoreLayoutError::IdOverflow))?;
                        let nested_result = sequence::layout_scope_with_cancellation(
                            &nested_graph,
                            options,
                            root_index,
                            engine.clone(),
                            cancellation,
                        )
                        .map_err(LayoutDispatchError::Tala)?;
                        if let Some(root_node) = graph.node(root) {
                            scope_cache.sequences.insert(
                                root_node.external_id.clone(),
                                cache_layout_result(&nested_graph, &nested_result),
                            );
                        }
                    }
                }
            }
        }

        // LayoutNested completes extracted constant-near subgraphs in source
        // traversal order before laying out the remaining parent graph. Keep
        // their geometry for the later placement pass, but preserve that
        // invocation order here so diagnostic engine traces match upstream.
        let mut nested_layouts = BTreeMap::new();
        for (root, members) in &groups {
            let (mut nested_graph, nested_nodes, nested_edges) =
                project_graph(graph, members, true);
            let nested_root_id = nested_nodes
                .iter()
                .position(|id| id == root)
                .map(|index| NodeId(index as u32))
                .ok_or(LayoutDispatchError::Elk(CoreLayoutError::IdOverflow))?;
            if let Some(nested_root) = nested_graph.node_mut(nested_root_id) {
                // The source nested call clears NearKey while laying out the
                // constant graph. Its anchor is restored after placement.
                nested_root.canvas_position = None;
            }
            let cache_key = graph
                .node(*root)
                .map(|node| node.external_id.as_str())
                .unwrap_or_default();
            let nested_result = if let Some(cached) = scope_cache.constant_nears.remove(cache_key) {
                restore_layout_result(&nested_graph, &cached)
                    .ok_or(LayoutDispatchError::Elk(CoreLayoutError::IdOverflow))?
            } else {
                self.layout_with_scope_cache(
                    &nested_graph,
                    engine.clone(),
                    options,
                    cancellation,
                    false,
                    Some(scope_cache),
                )?
            };
            #[cfg(feature = "diagnostic-traces")]
            if diagnostic_graph_has_grid(&nested_graph) {
                grid::trace_grid_snapshot(&nested_graph, &nested_result, None, "layout-final");
            }
            let nested_root_box = nested_result
                .boxes
                .get(&nested_root_id)
                .copied()
                .unwrap_or_else(|| {
                    graph.node(*root).map_or(Rect::default(), |node| Rect {
                        origin: Point::default(),
                        size: node.size,
                    })
                });
            nested_layouts.insert(
                *root,
                (nested_nodes, nested_edges, nested_result, nested_root_box),
            );
        }

        let mut result = if base_graph.nodes().next().is_some() {
            let base_result = self.layout_with_scope_cache_deferred(
                &base_graph,
                engine.clone(),
                options,
                cancellation,
                false,
                Some(scope_cache),
                matches!(engine, EngineKind::Tala),
            )?;
            remap_result(base_result, &base_nodes, &base_edges)
        } else {
            empty_layout_result(options)
        };

        // Constant-near extraction allocated the parent slice before core
        // layout. Respect a TALA patch that subsequently replaced that slice.
        result.source_edge_storage.record_prior_extraction();

        // Crossing grid/sequence edges belong to this parent extraction boundary.
        // Recursive base assembly has restored their routes already; defer
        // those routes until the parent's near pass completes, as LayoutNested
        // does. Retained lifelines remain fixed graph topology during the pass.
        let deferred_nested_edges = base_edges
            .iter()
            .copied()
            .filter(|id| {
                let edge = &graph.edges[id.0 as usize];
                let source = nearest_sequence_scope(graph, edge.source);
                let target = nearest_sequence_scope(graph, edge.target);
                let source_grid = outermost_grid_scope(graph, edge.source);
                let target_grid = outermost_grid_scope(graph, edge.target);
                source != target && (source.is_some() || target.is_some())
                    || source_grid != target_grid
                        && (source_grid.is_some() || target_grid.is_some())
            })
            .collect::<Vec<_>>();
        for id in &deferred_nested_edges {
            result.routes.remove(id);
        }

        // D2 places vertical-center anchors first, then horizontal-center
        // anchors, then the corner anchors. Earlier placements participate in
        // later bounding-box calculations, which is observable with multiple
        // constant-near objects.
        for position_group in [
            [CanvasPosition::TopCenter, CanvasPosition::BottomCenter].as_slice(),
            [CanvasPosition::CenterLeft, CanvasPosition::CenterRight].as_slice(),
            [
                CanvasPosition::TopLeft,
                CanvasPosition::TopRight,
                CanvasPosition::BottomLeft,
                CanvasPosition::BottomRight,
            ]
            .as_slice(),
        ] {
            for (root, _members) in groups.iter() {
                let Some(position) = graph.node(*root).and_then(|node| node.canvas_position) else {
                    continue;
                };
                if !position_group.contains(&position) {
                    continue;
                }
                let Some((nested_nodes, nested_edges, nested_result, nested_root_box)) =
                    nested_layouts.remove(root)
                else {
                    continue;
                };
                let nested_root_id = nested_nodes
                    .iter()
                    .position(|id| id == root)
                    .map(|index| NodeId(index as u32))
                    .ok_or(LayoutDispatchError::Elk(CoreLayoutError::IdOverflow))?;
                let bounds = near_bounds(graph, &result);
                let mut origin = constant_near_origin(position, bounds, nested_root_box.size);
                if let Some(label) = nested_result.node_labels.get(&nested_root_id) {
                    adjust_constant_near_for_label(
                        &mut origin,
                        position,
                        label.position,
                        label.size,
                    );
                }
                let delta = Point {
                    x: origin.x - nested_root_box.origin.x,
                    y: origin.y - nested_root_box.origin.y,
                };
                merge_nested_result(
                    &mut result,
                    nested_result,
                    &nested_nodes,
                    &nested_edges,
                    delta,
                );
                // d2near.Layout assigns the near root's position directly;
                // only its descendants and edges receive the delta. Adding
                // that delta back to the old root can lose a low bit.
                if let Some(rect) = result.boxes.get_mut(root) {
                    rect.origin = origin;
                }
            }
        }

        // Restore only edges extracted by this parent boundary. Go invokes
        // its selected edgeRouter after near placement and reinjection.
        let owner = |id| groups.iter().position(|(_, members)| members.contains(&id));
        let mut selected = graph
            .edges()
            .filter_map(|(id, edge)| {
                let source = owner(edge.source);
                let target = owner(edge.target);
                (deferred_nested_edges.contains(&id)
                    || source != target && (source.is_some() || target.is_some()))
                .then_some(id)
            })
            .collect::<Vec<_>>();
        order_extracted_edges(graph, &mut selected);
        reroute_selected_grid_edges(graph, &mut result, options, engine, cancellation, &selected)?;
        Ok(result)
    }

    fn layout_without_constant_nears(
        &self,
        graph: &Graph,
        engine: EngineKind,
        options: &LayoutOptions,
        cancellation: Option<&CancellationToken>,
    ) -> Result<LayoutResult, LayoutDispatchError> {
        self.layout_with_cancellation_without_constant_nears(graph, engine, options, cancellation)
    }

    // This indirection documents the recursion guard: projected graphs have
    // their anchor marker cleared before entering the ordinary dispatcher.
    fn layout_with_cancellation_without_constant_nears(
        &self,
        graph: &Graph,
        engine: EngineKind,
        options: &LayoutOptions,
        cancellation: Option<&CancellationToken>,
    ) -> Result<LayoutResult, LayoutDispatchError> {
        self.layout_with_cancellation(graph, engine, options, cancellation)
    }

    // helper implementations are below the dispatcher impl

    /// Routes every edge at its existing node coordinates.
    pub fn route_edges(
        &self,
        graph: &Graph,
        engine: EngineKind,
        options: &LayoutOptions,
    ) -> Result<LayoutResult, LayoutDispatchError> {
        self.route_edges_with_cancellation(graph, engine, options, None)
    }

    /// Routes every edge while observing an optional cancellation token.
    pub fn route_edges_with_cancellation(
        &self,
        graph: &Graph,
        engine: EngineKind,
        options: &LayoutOptions,
        cancellation: Option<&CancellationToken>,
    ) -> Result<LayoutResult, LayoutDispatchError> {
        match engine {
            EngineKind::Tala => match cancellation {
                Some(token) => d2_rs_tala::Engine
                    .route_edges_with_cancellation(graph, options, token)
                    .map_err(LayoutDispatchError::Tala),
                None => d2_rs_tala::Engine
                    .route_edges(graph, options)
                    .map_err(LayoutDispatchError::Tala),
            },
            EngineKind::Dagre => {
                let result =
                    d2_rs_dagre::route_edges(graph, options).map_err(LayoutDispatchError::Tala)?;
                Ok(result)
            }
            EngineKind::Elk => {
                let result =
                    d2_rs_elk::route_edges(graph, options).map_err(LayoutDispatchError::Elk)?;
                Ok(result)
            }
        }
    }

    /// Routes only the requested edges, preserving their input order.
    pub fn route_edges_selected(
        &self,
        graph: &Graph,
        engine: EngineKind,
        options: &LayoutOptions,
        edges: &[EdgeId],
    ) -> Result<LayoutResult, LayoutDispatchError> {
        self.route_edges_selected_with_cancellation(graph, engine, options, edges, None)
    }

    /// Routes selected edges while observing an optional cancellation token.
    pub fn route_edges_selected_with_cancellation(
        &self,
        graph: &Graph,
        engine: EngineKind,
        options: &LayoutOptions,
        edges: &[EdgeId],
        cancellation: Option<&CancellationToken>,
    ) -> Result<LayoutResult, LayoutDispatchError> {
        match engine {
            EngineKind::Tala => match cancellation {
                Some(token) => d2_rs_tala::Engine
                    .route_edges_selected_with_cancellation(graph, options, edges, token)
                    .map_err(LayoutDispatchError::Tala),
                None => d2_rs_tala::Engine
                    .route_edges_selected(graph, options, edges)
                    .map_err(LayoutDispatchError::Tala),
            },
            EngineKind::Dagre => {
                let result = d2_rs_dagre::route_edges_selected(graph, options, Some(edges))
                    .map_err(LayoutDispatchError::Tala)?;
                Ok(result)
            }
            EngineKind::Elk => {
                let result = d2_rs_elk::route_edges_selected(graph, options, edges)
                    .map_err(LayoutDispatchError::Elk)?;
                Ok(result)
            }
        }
    }
}

#[cfg(feature = "diagnostic-traces")]
fn diagnostic_graph_has_grid(graph: &Graph) -> bool {
    // LayoutNested's observer tests raw IsGridDiagram attributes, including
    // grids authored within sequence actors whose layout remains suppressed.
    graph.root_grid_rows.is_some()
        || graph.root_grid_columns.is_some()
        || graph
            .nodes()
            .any(|(_, node)| node.grid_rows.is_some() || node.grid_columns.is_some())
}

fn has_grid_scope(graph: &Graph) -> bool {
    (!graph.root_sequence_diagram
        && (graph.root_grid_rows.is_some() || graph.root_grid_columns.is_some()))
        || graph.nodes().any(|(id, node)| {
            (node.grid_rows.is_some() || node.grid_columns.is_some())
                && !inside_sequence_scope(graph, id)
        })
}

/// Match D2's `LayoutNested(..., GridDiagram)` path for an implicit root grid.
/// D2 recursively lays out a cell's ordinary descendants (and any nested grid
/// scopes) before the root grid measures the cell. The flat `d2grid` adapter
/// cannot see that extraction boundary, so seed boxes from measured graph
/// sizes, lay out compound grid cells, apply grid scopes from the inside out,
/// then route only edges that cross root cells.
fn layout_root_grid_with_nested_cells(
    graph: &Graph,
    options: &LayoutOptions,
    engine: EngineKind,
    cancellation: Option<&CancellationToken>,
) -> Result<LayoutResult, LayoutDispatchError> {
    let mut result = empty_layout_result(options);
    for (id, node) in graph.nodes() {
        result.boxes.insert(
            id,
            Rect {
                origin: Point::default(),
                size: node.size,
            },
        );
    }

    result = layout_nested_grid_cells(graph, result, options, None, engine.clone(), cancellation)?;
    result = grid::apply_scopes(graph, result, options).map_err(LayoutDispatchError::Tala)?;

    // Grid child diagrams restore their crossings before the enclosing root
    // restores its own. This callback also commits DefaultRouter label state
    // under Dagre/ELK (d2layouts.go:330–382).
    reroute_grid_scope_external_edges(
        graph,
        &mut result,
        options,
        cancellation,
        false,
        engine.clone(),
    )?;

    let mut extracted_edges = Vec::new();
    for (edge_id, edge) in graph.edges() {
        let source_cell = root_grid_cell(graph, edge.source);
        let target_cell = root_grid_cell(graph, edge.target);
        if source_cell.is_none() || source_cell == target_cell {
            continue;
        }
        // LayoutNested removes cross-cell edges whose endpoints are inside
        // extracted cell graphs, then routes them after reinsertion with the
        // selected engine's edge router. d2grid's straight route applies only
        // when both endpoints are the root cells themselves.
        if source_cell != Some(edge.source) || target_cell != Some(edge.target) {
            extracted_edges.push(edge_id);
            continue;
        }
        if let (Some(source), Some(target)) = (
            result.boxes.get(&edge.source),
            result.boxes.get(&edge.target),
        ) {
            let route = d2_rs_dagre::route_grid_edge(graph, edge_id, *source, *target)
                .unwrap_or_else(|| vec![source.center(), target.center()]);
            result
                .route_curves
                .insert(edge_id, graph.edge_is_curve(edge_id));
            result.routes.insert(edge_id, route);
            grid::set_grid_edge_label(graph, &mut result, edge_id);
        }
    }
    reroute_selected_grid_edges(
        graph,
        &mut result,
        options,
        engine,
        cancellation,
        &extracted_edges,
    )?;
    Ok(result)
}

fn root_grid_cell(graph: &Graph, id: NodeId) -> Option<NodeId> {
    let mut current = id;
    loop {
        let node = graph.node(current)?;
        let Some(parent) = node.parent else {
            return Some(current);
        };
        current = parent;
    }
}

fn is_grid_cell_container(graph: &Graph, id: NodeId) -> bool {
    let Some(node) = graph.node(id) else {
        return false;
    };
    let grid_parent = match node.parent {
        Some(parent) => graph
            .node(parent)
            .is_some_and(|parent| parent.grid_rows.is_some() || parent.grid_columns.is_some()),
        None => graph.root_grid_rows.is_some() || graph.root_grid_columns.is_some(),
    };
    grid_parent
        && node.grid_rows.is_none()
        && node.grid_columns.is_none()
        && node.canvas_position.is_none()
        && !is_sequence_descendant(graph, id)
        && !matches!(
            node.shape,
            d2_rs_core::ShapeKind::Class | d2_rs_core::ShapeKind::SqlTable
        )
        && graph.nodes().any(|(_, child)| child.parent == Some(id))
}

/// Lay out container and sequence cells before positioning the enclosing
/// grid. Ordinary cells use the core engine; sequence cells dispatch their
/// own contents. The initial boxes are measured-size anchors; the recursive
/// cell passes supply the geometry consumed by `d2grid`.
fn layout_nested_grid_cells(
    graph: &Graph,
    mut result: LayoutResult,
    options: &LayoutOptions,
    mut scope_cache: Option<&mut DagreScopeCache>,
    engine: EngineKind,
    cancellation: Option<&CancellationToken>,
) -> Result<LayoutResult, LayoutDispatchError> {
    let cells = ordered_nested_grid_cells(graph);
    #[cfg(feature = "diagnostic-traces")]
    if diagnostic_env_enabled("DAGRE_DEBUG_NESTED") {
        for (id, node) in graph.nodes() {
            if graph.nodes().any(|(_, child)| child.parent == Some(id)) {
                eprintln!(
                    "GRID_CELL_CANDIDATE id={id:?} external={:?} parent={:?} grid={} direction={:?} near={:?} cell={}",
                    node.external_id,
                    node.parent,
                    node.grid_rows.is_some() || node.grid_columns.is_some(),
                    node.direction,
                    node.canvas_position,
                    is_grid_cell_container(graph, id)
                );
            }
        }
    }
    // ExtractSubgraph allocates the containing graph's remainingEdges even
    // when empty (d2layouts.go:660). Reinjection preserves that empty slice.
    // A later TALA parent commit replaces it with its own initially nil slice.
    if !cells.is_empty() {
        result.source_edge_storage.record_prior_extraction();
    }
    for cell in cells {
        let mut ids = vec![cell];
        ids.extend(ordered_descendants(graph, cell));
        // LayoutNested recursively lays out a grid cell that owns another
        // special diagram before the containing grid measures that cell. Use
        // this dispatcher on the extracted subtree so nested grids and
        // sequences are resolved before the ordinary parent layout.
        let has_nested_diagram = graph.is_sequence_diagram(cell)
            || ids.iter().skip(1).any(|id| {
                graph.node(*id).is_some_and(|node| {
                    node.grid_rows.is_some()
                        || node.grid_columns.is_some()
                        || node.canvas_position.is_some()
                        || graph.is_sequence_diagram(*id)
                })
            });
        if has_nested_diagram {
            #[cfg(feature = "diagnostic-traces")]
            if diagnostic_env_enabled("DAGRE_DEBUG_NESTED") {
                eprintln!("GRID_CELL_RECURSE_NESTED_DIAGRAM id={cell:?} nodes={ids:?}");
            }
        }
        let (mut projected, node_ids, edge_ids) = project_graph(graph, &ids, false);
        // LayoutNested extracts each grid-cell container and copies that
        // object's attributes to the new root. An absent cell direction thus
        // uses Dagre's default (down), even when its former parent flowed in
        // another direction.
        projected.direction = graph
            .node(cell)
            .and_then(|node| node.direction)
            .unwrap_or_default();
        projected.root_grid_rows = None;
        projected.root_grid_columns = None;
        projected.root_grid_gap = None;
        projected.root_sequence_diagram = false;
        // ExtractSubgraph reparents an included cell below an empty-ID root.
        // The adapter allocates TALA identities from those local AbsIDs anew;
        // inherited document IDs change ID-ordered preprocessing and cycles.
        if let Some(parent) = graph.node(cell).and_then(|node| node.parent)
            && let Some(parent) = graph.node(parent)
        {
            // An ordinary included cell becomes a child of the extracted
            // empty-name root. Keep the actual local D2 names in that graph,
            // not just the hashed IDs. The outer graph retains public names.
            if !graph.is_sequence_diagram(cell) {
                let prefix = format!("{}.", parent.external_id);
                for node in &mut projected.nodes {
                    if let Some(local) = node.external_id.strip_prefix(&prefix) {
                        node.external_id = local.to_owned();
                    }
                }
            }
            rebase_routing_identities(&mut projected, &parent.external_id)?;
        }
        // Resolve nested sequence diagrams before Dagre measures this cell.
        // LayoutNested extracts their contents first and presents each
        // sequence root as one sized object in the containing Dagre graph.
        let cache_key = graph.node(cell).map(|node| node.external_id.clone());
        let cached = cache_key.as_ref().and_then(|key| {
            scope_cache
                .as_deref()
                .and_then(|cache| cache.grid_cells.get(key))
                .and_then(|cached| restore_layout_result(&projected, cached))
        });
        let local = if graph.is_sequence_diagram(cell) {
            // LayoutNested extracts a special sequence cell with includeSelf
            // false. Its contents run as a sequence root; unlike an ordinary
            // cell container, it has no extra parent core-layout invocation.
            sequence::layout_scope_with_cancellation(
                &projected,
                options,
                NodeId(0),
                engine.clone(),
                cancellation,
            )
            .map_err(LayoutDispatchError::Tala)?
        } else if has_nested_diagram {
            let local = LayoutDispatcher.layout_with_scope_cache(
                &projected,
                engine.clone(),
                options,
                cancellation,
                false,
                scope_cache.as_deref_mut(),
            )?;
            if let (Some(key), Some(cache)) = (cache_key.clone(), scope_cache.as_deref_mut()) {
                cache
                    .grid_cells
                    .insert(key, cache_layout_result(&projected, &local));
            }
            local
        } else if let Some(cached) = cached {
            cached
        } else if matches!(engine, EngineKind::Tala) {
            match cancellation {
                Some(token) => d2_rs_tala::Engine
                    .layout_with_cancellation(&projected, options, token)
                    .map_err(LayoutDispatchError::Tala)?,
                None => d2_rs_tala::Engine
                    .layout(&projected, options)
                    .map_err(LayoutDispatchError::Tala)?,
            }
        } else if matches!(engine, EngineKind::Elk) {
            let local = d2_rs_elk::layout_with_cancellation(&projected, options, cancellation)
                .map_err(LayoutDispatchError::Elk)?;
            if let (Some(key), Some(cache)) = (cache_key, scope_cache.as_deref_mut()) {
                cache
                    .grid_cells
                    .insert(key, cache_layout_result(&projected, &local));
            }
            local
        } else {
            let sequence_projection = sequence::project_for_dagre_with_cache(
                &projected,
                options,
                scope_cache.as_deref_mut(),
                cancellation,
            )
            .map_err(LayoutDispatchError::Tala)?;
            let local = if let Some(projection) = sequence_projection {
                let local =
                    d2_rs_dagre::layout_with_cancellation(&projection.graph, options, cancellation)
                        .map_err(LayoutDispatchError::Tala)?;
                let local = remap_result(local, &projection.node_map, &projection.edge_map);
                let mut local =
                    sequence::apply_dagre_projection_locals(&projected, local, projection.locals);
                reroute_sequence_external_edges(
                    &projected,
                    &mut local,
                    options,
                    EngineKind::Dagre,
                    cancellation,
                )?;
                local
            } else {
                d2_rs_dagre::layout_with_cancellation(&projected, options, cancellation)
                    .map_err(LayoutDispatchError::Tala)?
            };
            if let (Some(key), Some(cache)) = (cache_key, scope_cache.as_deref_mut()) {
                cache
                    .grid_cells
                    .insert(key, cache_layout_result(&projected, &local));
            }
            local
        };
        let local_cell = node_ids
            .iter()
            .position(|id| *id == cell)
            .map(|index| NodeId(index as u32))
            .ok_or(CoreLayoutError::IdOverflow)
            .map_err(LayoutDispatchError::Tala)?;
        #[cfg(feature = "diagnostic-traces")]
        if diagnostic_graph_has_grid(&projected) {
            // Source extracts a sequence without its placeholder, advancing
            // RootLevel; its observer only emits layout-final at RootLevel 0.
            // Keep this complete local observation under its actual boundary.
            let stage = if graph.is_sequence_diagram(cell) {
                "layout-final-sequence-cell-local"
            } else {
                "layout-final"
            };
            grid::trace_grid_snapshot(&projected, &local, None, stage);
        }
        let local_box = local.boxes.get(&local_cell).copied().unwrap_or_default();
        let anchor = result.boxes.get(&cell).copied().unwrap_or_else(|| {
            graph.node(cell).map_or(Rect::default(), |node| Rect {
                origin: Point::default(),
                size: node.size,
            })
        });
        #[cfg(feature = "diagnostic-traces")]
        if diagnostic_env_enabled("DAGRE_DEBUG_NESTED") {
            eprintln!(
                "GRID_CELL_LOCAL id={cell:?} external={:?} direction={:?} anchor={anchor:?} local_root={local_box:?}",
                graph.node(cell).map(|node| node.external_id.as_str()),
                projected.direction,
            );
            for (local_id, node) in projected.nodes() {
                eprintln!(
                    "GRID_CELL_LOCAL_NODE external={:?} parent={:?} rect={:?}",
                    node.external_id,
                    node.parent.and_then(|parent| projected
                        .node(parent)
                        .map(|parent| parent.external_id.as_str())),
                    local.boxes.get(&local_id),
                );
            }
        }
        let delta = Point {
            x: anchor.origin.x - local_box.origin.x,
            y: anchor.origin.y - local_box.origin.y,
        };
        merge_nested_result(&mut result, local, &node_ids, &edge_ids, delta);
        #[cfg(feature = "diagnostic-traces")]
        if diagnostic_env_enabled("DAGRE_DEBUG_NESTED") {
            eprintln!(
                "GRID_CELL_MERGED id={cell:?} external={:?} rect={:?}",
                graph.node(cell).map(|node| node.external_id.as_str()),
                result.boxes.get(&cell),
            );
        }
    }
    Ok(result)
}

fn ordered_hierarchy_children(graph: &Graph, parent: Option<NodeId>) -> Vec<NodeId> {
    let fallback = graph
        .nodes()
        .filter_map(|(id, node)| (node.parent == parent).then_some(id))
        .collect::<Vec<_>>();
    graph
        .hierarchy_children_order
        .get(&parent)
        .filter(|ids| !ids.is_empty())
        .cloned()
        .unwrap_or(fallback)
}

/// Enumerate source scopes in the same breadth-first order as
/// `d2layouts.LayoutNested` and `collectNestedExtractions`.
#[cfg(test)]
fn ordered_hierarchy_breadth_first(graph: &Graph) -> Vec<NodeId> {
    let mut queue = ordered_hierarchy_children(graph, None);
    let mut cursor = 0;
    let mut visited = BTreeSet::new();
    let mut ordered = Vec::with_capacity(graph.nodes().len());
    while cursor < queue.len() {
        let parent = queue[cursor];
        cursor += 1;
        if !visited.insert(parent) || graph.node(parent).is_none() {
            continue;
        }
        ordered.push(parent);
        queue.extend(ordered_hierarchy_children(graph, Some(parent)));
    }
    // Malformed and programmatically-built graphs can contain nodes that are
    // not reachable from a parentless node. Keep those available to layout in
    // stable arena order after all valid hierarchy scopes.
    ordered.extend(
        graph
            .nodes()
            .map(|(id, _)| id)
            .filter(|id| !visited.contains(id)),
    );
    ordered
}

/// Enumerate grid cells in the recursive order used by `LayoutNested`.
/// Special grid scopes are traversed before the next sibling cell, while an
/// ordinary container cell owns its descendants and is laid out recursively
/// as one extracted graph.
fn ordered_nested_grid_cells(graph: &Graph) -> Vec<NodeId> {
    fn visit(graph: &Graph, id: NodeId, cells: &mut Vec<NodeId>) {
        if is_grid_cell_container(graph, id) {
            cells.push(id);
            return;
        }
        for child in ordered_hierarchy_children(graph, Some(id)) {
            visit(graph, child, cells);
        }
    }

    let mut cells = Vec::new();
    for root in ordered_hierarchy_children(graph, None) {
        visit(graph, root, &mut cells);
    }
    cells
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NestedDagreScope {
    ConstantNear(NodeId),
    Grid(NodeId),
    Sequence(NodeId),
}

/// Find the first special scopes reached by D2's breadth-first hierarchy
/// queue. A special scope owns its descendants, which are visited by its
/// recursive layout call instead of by the parent queue.
fn nested_dagre_scope_plan(graph: &Graph) -> Vec<NestedDagreScope> {
    let mut queue = ordered_hierarchy_children(graph, None);
    let mut cursor = 0;
    let mut visited = BTreeSet::new();
    let mut scopes = Vec::new();
    while cursor < queue.len() {
        let id = queue[cursor];
        cursor += 1;
        if !visited.insert(id) {
            continue;
        }
        let Some(node) = graph.node(id) else {
            continue;
        };
        if node.canvas_position.is_some() {
            scopes.push(NestedDagreScope::ConstantNear(id));
            continue;
        }
        // LayoutNested leaves childless grids and sequences in the parent
        // graph. Only constant-near scopes are extracted without children.
        let children = ordered_hierarchy_children(graph, Some(id));
        if children.is_empty() {
            continue;
        }
        if graph.is_sequence_diagram(id) {
            scopes.push(NestedDagreScope::Sequence(id));
        } else if node.grid_rows.is_some() || node.grid_columns.is_some() {
            scopes.push(NestedDagreScope::Grid(id));
        } else {
            queue.extend(children);
        }
    }
    scopes
}

/// Match extractSubgraphs: crossings belong to the first extraction, and
/// each extraction retains the original edge order.
fn order_extracted_edges(graph: &Graph, edges: &mut [d2_rs_core::EdgeId]) {
    let scopes = nested_dagre_scope_plan(graph);
    let owner = |id| {
        scopes.iter().position(|scope| {
            let (root, include_self) = match *scope {
                NestedDagreScope::ConstantNear(root) => (root, true),
                NestedDagreScope::Grid(root) | NestedDagreScope::Sequence(root) => (root, false),
            };
            (include_self || id != root) && node_in_scope(graph, id, root)
        })
    };
    let owners = graph
        .edges()
        .map(|(id, edge)| {
            let index = owner(edge.source)
                .into_iter()
                .chain(owner(edge.target))
                .min()
                .unwrap_or(usize::MAX);
            (id, index)
        })
        .collect::<BTreeMap<_, _>>();
    edges.sort_by_key(|id| owners.get(id).copied().unwrap_or(usize::MAX));
}

fn node_in_scope(graph: &Graph, id: NodeId, scope: NodeId) -> bool {
    let mut current = Some(id);
    while let Some(candidate) = current {
        if candidate == scope {
            return true;
        }
        current = graph.node(candidate).and_then(|node| node.parent);
    }
    false
}

fn ordered_descendants(graph: &Graph, scope: NodeId) -> Vec<NodeId> {
    fn visit(graph: &Graph, parent: NodeId, output: &mut Vec<NodeId>) {
        for child in ordered_hierarchy_children(graph, Some(parent)) {
            output.push(child);
            visit(graph, child, output);
        }
    }
    let mut output = Vec::new();
    visit(graph, scope, &mut output);
    output
}

fn has_nested_grid_scope(graph: &Graph) -> bool {
    // LayoutNested extracts nonempty object grids, including a lone root
    // object. It explicitly leaves a childless grid in the ordinary parent
    // layout (d2layouts.go:214); skipping that pass loses label/font commits.
    graph.nodes().any(|(id, node)| {
        (node.grid_rows.is_some() || node.grid_columns.is_some())
            && !inside_sequence_scope(graph, id)
            && graph.nodes().any(|(_, child)| child.parent == Some(id))
    })
}

/// Resolve object-level grids before the parent ELK pass, then lay out only
/// their fitted placeholders in ELK and restore each grid's local geometry.
/// D2's nested-scope runner extracts each grid's children before calling the
/// selected core engine; including those descendants in the parent ELK graph
/// changes the root component layout.
fn layout_elk_with_nested_grids(
    graph: &Graph,
    options: &LayoutOptions,
    scope_cache: Option<&mut DagreScopeCache>,
    cancellation: Option<&CancellationToken>,
    grid_diagram_root: bool,
) -> Result<LayoutResult, LayoutDispatchError> {
    let mut nested = empty_layout_result(options);
    for (id, node) in graph.nodes() {
        nested.boxes.insert(
            id,
            Rect {
                origin: Point::default(),
                size: node.size,
            },
        );
    }
    nested = layout_nested_grid_cells(
        graph,
        nested,
        options,
        scope_cache,
        EngineKind::Elk,
        cancellation,
    )?;
    #[cfg(feature = "diagnostic-traces")]
    grid::trace_grid_snapshot(graph, &nested, None, "elk-pre-grid-scopes");
    nested = grid::apply_scopes(graph, nested, options).map_err(LayoutDispatchError::Elk)?;
    reroute_grid_scope_external_edges(
        graph,
        &mut nested,
        options,
        cancellation,
        false,
        EngineKind::Elk,
    )?;
    // LayoutNested resolves special grid cells before d2grid sizes the root.
    // A detached GridDiagram has no enclosing ordinary ELK invocation; its
    // sequence cells must retain the geometry computed above.
    if grid_diagram_root {
        return Ok(nested);
    }
    collapse_nested_grid_scopes(graph, nested, options, |projected, options| {
        d2_rs_elk::layout_with_cancellation(projected, options, cancellation)
    })
    .map_err(LayoutDispatchError::Elk)
}

/// Collapse nested grid scopes before the parent Dagre pass, then restore
/// their already-laid-out descendants. D2's `LayoutNested` removes these
/// descendants and their cross-scope edges before laying out the parent;
/// leaving them in the root graph changes disconnected-component placement.
fn collapse_dagre_nested_grids(
    graph: &Graph,
    nested: LayoutResult,
    options: &LayoutOptions,
    cancellation: Option<&CancellationToken>,
) -> Result<LayoutResult, d2_rs_core::LayoutError> {
    collapse_nested_grid_scopes(graph, nested, options, |projected, options| {
        d2_rs_dagre::layout_with_options_and_rank_trace_with_cancellation(
            projected,
            options,
            d2_rs_dagre::DagreOptions::default(),
            true,
            cancellation,
        )
    })
}

/// Collapse already-laid-out special scopes before running the selected
/// parent engine, then restore each scope's local geometry at its new origin.
/// D2's `LayoutNested` extracts grid descendants before invoking any core
/// layout engine; passing them through the parent ELK graph changes its
/// component ordering and spacing even when grid postprocessing later places
/// the cell contents correctly.
fn collapse_nested_grid_scopes(
    graph: &Graph,
    nested: LayoutResult,
    options: &LayoutOptions,
    layout_parent: impl FnOnce(&Graph, &LayoutOptions) -> Result<LayoutResult, d2_rs_core::LayoutError>,
) -> Result<LayoutResult, d2_rs_core::LayoutError> {
    let visible_ids = graph
        .nodes()
        .filter_map(|(id, _)| {
            (closest_grid_scope(graph, id).is_none() && !is_sequence_descendant(graph, id))
                .then_some(id)
        })
        .collect::<Vec<_>>();
    let mut collapsed_source = graph.clone();
    for (id, node) in graph.nodes() {
        let is_grid = node.grid_rows.is_some() || node.grid_columns.is_some();
        let is_sequence_root = graph.is_sequence_diagram(id);
        if (!is_grid && !is_sequence_root)
            || (inside_sequence_scope(graph, id) && !is_sequence_root)
        {
            continue;
        }
        if let Some(rect) = nested.boxes.get(&id)
            && let Some(collapsed) = collapsed_source.node_mut(id)
        {
            collapsed.size = rect.size;
            // d2grid.Layout sets the extracted root's label and icon defaults
            // before the parent engine measures its placeholder's spacing.
            if let Some(label) = nested.node_labels.get(&id) {
                collapsed.label_position = label.position;
                collapsed.icon_position = label.icon_position;
            }
            collapsed.fixed_width = true;
            collapsed.fixed_height = true;
        }
    }
    let (projected, node_ids, edge_ids) = project_graph(&collapsed_source, &visible_ids, false);
    let projected_result = layout_parent(&projected, options)?;
    let mut result = remap_result(projected_result, &node_ids, &edge_ids);
    // Special scopes were extracted from this parent before core layout.
    // Dagre/ELK mutate edges in place, preserving allocated storage.
    if visible_ids.len() < graph.nodes.len() {
        result.source_edge_storage.record_prior_extraction();
    }
    #[cfg(feature = "diagnostic-traces")]
    if diagnostic_env_enabled("DAGRE_DEBUG_NESTED") {
        eprintln!("COLLAPSED_ROOT {:?}", result.boxes);
    }

    let mut scope_deltas = BTreeMap::new();
    for (id, node) in graph.nodes() {
        if (node.grid_rows.is_none() && node.grid_columns.is_none())
            || inside_sequence_scope(graph, id)
        {
            continue;
        }
        let delta =
            if let (Some(before), Some(after)) = (nested.boxes.get(&id), result.boxes.get(&id)) {
                Point {
                    x: after.origin.x - before.origin.x,
                    y: after.origin.y - before.origin.y,
                }
            } else if let Some(parent_scope) = closest_grid_scope(graph, id) {
                // A nested grid is absent from the collapsed parent graph. Its
                // frame moves with the nearest extracted ancestor; its own local
                // grid placement was already committed in `nested`.
                scope_deltas.get(&parent_scope).copied().unwrap_or_default()
            } else {
                continue;
            };
        scope_deltas.insert(id, delta);
        if let Some(label) = nested.node_labels.get(&id).cloned() {
            result.node_labels.insert(id, label);
        }
    }

    let mut sequence_roots = graph
        .nodes()
        .filter_map(|(id, _)| graph.is_sequence_diagram(id).then_some(id))
        .collect::<Vec<_>>();
    sequence_roots.sort_by_key(|id| {
        let mut depth = 0;
        let mut parent = graph.node(*id).and_then(|node| node.parent);
        while let Some(current) = parent {
            depth += 1;
            parent = graph.node(current).and_then(|node| node.parent);
        }
        depth
    });
    let mut sequence_deltas = BTreeMap::new();
    for id in sequence_roots {
        // d2layouts.InjectNested restores the completed root's label/icon
        // state after the parent engine lays out its childless placeholder.
        // Visible sequence roots need this too: the descendant loop below
        // deliberately skips nodes present in the parent projection.
        // Empty sequence declarations are ordinary leaves, not extracted roots.
        if graph.nodes().any(|(_, child)| child.parent == Some(id))
            && let Some(label) = nested.node_labels.get(&id).cloned()
        {
            result.node_labels.insert(id, label);
        }
        let delta = match (nested.boxes.get(&id), result.boxes.get(&id)) {
            (Some(before), Some(after)) => Point {
                x: after.origin.x - before.origin.x,
                y: after.origin.y - before.origin.y,
            },
            // A sequence nested beneath another extracted diagram is absent
            // from the collapsed parent's Dagre graph. Its movement follows
            // the nearest enclosing grid or sequence, and must be inherited
            // by the sequence's own descendants as well as its placeholder.
            _ => closest_grid_scope(graph, id)
                .and_then(|scope| scope_deltas.get(&scope).copied())
                .or_else(|| {
                    nearest_sequence_scope(graph, id)
                        .and_then(|scope| sequence_deltas.get(&scope).copied())
                })
                .unwrap_or_default(),
        };
        sequence_deltas.insert(id, delta);
    }

    for (id, _node) in graph.nodes() {
        if visible_ids.binary_search(&id).is_ok() {
            continue;
        }
        let delta = closest_grid_scope(graph, id)
            .and_then(|scope| scope_deltas.get(&scope).copied())
            .or_else(|| {
                nearest_sequence_scope(graph, id)
                    .and_then(|scope| sequence_deltas.get(&scope).copied())
            });
        let (Some(mut rect), Some(delta)) = (nested.boxes.get(&id).copied(), delta) else {
            continue;
        };
        rect.origin.x += delta.x;
        rect.origin.y += delta.y;
        result.boxes.insert(id, rect);
        if let Some(label) = nested.node_labels.get(&id).cloned() {
            result.node_labels.insert(id, label);
        }
    }

    for mut line in nested.sequence_lifelines.clone() {
        let delta = closest_grid_scope(graph, line.actor)
            .and_then(|scope| scope_deltas.get(&scope).copied())
            .or_else(|| {
                nearest_sequence_scope(graph, line.actor)
                    .and_then(|scope| sequence_deltas.get(&scope).copied())
            })
            .unwrap_or_default();
        for point in &mut line.route {
            point.x += delta.x;
            point.y += delta.y;
        }
        result.sequence_lifelines.push(line);
    }
    result.removed_edges.extend(&nested.removed_edges);
    for (edge_id, edge) in graph.edges() {
        if result.removed_edges.contains(&edge_id) {
            continue;
        }
        if visible_ids.binary_search(&edge.source).is_ok()
            && visible_ids.binary_search(&edge.target).is_ok()
        {
            continue;
        }
        let source_sequence = nearest_sequence_scope(graph, edge.source);
        let target_sequence = nearest_sequence_scope(graph, edge.target);
        let sequence_route_delta = (source_sequence.is_some()
            && source_sequence == target_sequence)
            .then(|| source_sequence.and_then(|scope| sequence_deltas.get(&scope).copied()))
            .flatten();
        // The collapsed parent extracts the outermost grid as one graph;
        // completed crossings move with it even across nested grid scopes.
        let source_grid = outermost_grid_scope(graph, edge.source);
        let target_grid = outermost_grid_scope(graph, edge.target);
        let grid_route_delta = (source_grid.is_some() && source_grid == target_grid)
            .then(|| source_grid.and_then(|scope| scope_deltas.get(&scope).copied()))
            .flatten();
        if let Some((mut route, delta)) = nested
            .routes
            .get(&edge_id)
            .cloned()
            .zip(sequence_route_delta.or(grid_route_delta))
        {
            for point in &mut route {
                point.x += delta.x;
                point.y += delta.y;
            }
            #[cfg(feature = "diagnostic-traces")]
            if diagnostic_env_enabled("DAGRE_DEBUG_NESTED") {
                eprintln!(
                    "COLLAPSED_ROUTE edge={:?} sequence_scope={:?} grid_scope={:?} route={:?}",
                    edge_id, source_sequence, source_grid, route
                );
            }
            copy_route_kind(&mut result, edge_id, &nested, edge_id);
            result.routes.insert(edge_id, route);
            // d2layouts.InjectNested restores the same edge objects, including
            // the label state committed by the extracted grid/sequence layout.
            if let Some(label) = nested.edge_labels.get(&edge_id).cloned() {
                result.edge_labels.insert(edge_id, label);
            }
            if nested.edge_label_percentage_commits.contains(&edge_id) {
                result.edge_label_percentage_commits.insert(edge_id);
            }
            continue;
        }
        if let (Some(source), Some(target)) = (
            result.boxes.get(&edge.source),
            result.boxes.get(&edge.target),
        ) {
            // LayoutNested routes edges restored across an extracted diagram
            // with DefaultRouter: centers first, then D2's ordered shape and
            // label intersections. The rectangle-only approximation loses
            // those exact endpoints on non-rectangular and half-pixel boxes.
            let route = d2_rs_dagre::route_straight_edge(graph, edge_id, *source, *target)
                .unwrap_or_else(|| {
                    vec![
                        layout_boundary_intersection(*source, target.center()),
                        layout_boundary_intersection(*target, source.center()),
                    ]
                });
            #[cfg(feature = "diagnostic-traces")]
            if diagnostic_env_enabled("DAGRE_DEBUG_NESTED")
                && graph
                    .node(edge.source)
                    .is_some_and(|node| node.external_id == "center.cloud")
            {
                eprintln!(
                    "CROSS_SCOPE_DEFAULT_ROUTE edge={:?} source={:?} target={:?} source_box={:?} target_box={:?} points={:?}",
                    edge_id,
                    graph.node(edge.source).map(|node| &node.external_id),
                    graph.node(edge.target).map(|node| &node.external_id),
                    source,
                    target,
                    route
                );
            }
            result.routes.insert(edge_id, route);
            set_default_router_edge_label(graph, &mut result, edge_id);
        }
    }
    Ok(result)
}

/// d2sequence appends lifeline edges to its own graph's Edges, and
/// LayoutNested then appends each extracted graph's edges in injection order
/// (SaveOrder keeps new edges in that relative order). For one graph: its own
/// lifelines when it is a sequence diagram, then constant-near graphs
/// (d2near.Layout injects them first), then the other nested diagrams in the
/// breadth-first order of collectNestedExtractions, each recursively.
fn order_lifelines_like_layout_nested(graph: &Graph, result: &mut LayoutResult) {
    if result.sequence_lifelines.len() < 2 {
        return;
    }
    let mut children: BTreeMap<Option<NodeId>, Vec<NodeId>> = BTreeMap::new();
    for (id, node) in graph.nodes() {
        children.entry(node.parent).or_default().push(id);
    }
    fn visit(
        graph: &Graph,
        children: &BTreeMap<Option<NodeId>, Vec<NodeId>>,
        own: Option<Option<NodeId>>,
        roots: &[NodeId],
        top_level: bool,
        order: &mut Vec<Option<NodeId>>,
    ) {
        order.extend(own);
        let mut queue = roots
            .iter()
            .copied()
            .collect::<std::collections::VecDeque<_>>();
        let (mut nears, mut nested) = (Vec::new(), Vec::new());
        while let Some(id) = queue.pop_front() {
            let Some(node) = graph.node(id) else { continue };
            let kids = children
                .get(&Some(id))
                .map(Vec::as_slice)
                .unwrap_or_default();
            let near = top_level && node.canvas_position.is_some();
            if near {
                nears.push(id);
            } else if graph.is_sequence_diagram(id)
                || node.grid_rows.is_some()
                || node.grid_columns.is_some()
            {
                if !kids.is_empty() {
                    nested.push(id);
                }
            } else {
                queue.extend(kids.iter().copied());
            }
        }
        for id in nears {
            // The near object becomes the nested root's only child.
            visit(graph, children, None, &[id], false, order);
        }
        for id in nested {
            let kids = children
                .get(&Some(id))
                .map(Vec::as_slice)
                .unwrap_or_default();
            let own = graph.is_sequence_diagram(id).then_some(Some(id));
            visit(graph, children, own, kids, false, order);
        }
    }
    let mut order = Vec::new();
    let roots = children.get(&None).map(Vec::as_slice).unwrap_or_default();
    visit(
        graph,
        &children,
        graph.root_sequence_diagram.then_some(None),
        roots,
        true,
        &mut order,
    );
    let rank = |line: &d2_rs_core::SequenceLifeline| {
        let parent = graph.node(line.actor).and_then(|node| node.parent);
        let owner = parent.filter(|id| graph.is_sequence_diagram(*id));
        order
            .iter()
            .position(|entry| *entry == owner)
            .unwrap_or(usize::MAX)
    };
    result.sequence_lifelines.sort_by_key(rank);
}

/// d2layouts.DefaultRouter: a non-empty label on a straight-routed edge is
/// centered on it.
fn set_default_router_edge_label(graph: &Graph, result: &mut LayoutResult, id: EdgeId) {
    if let Some(mut label) = result
        .edge_labels
        .get(&id)
        .or_else(|| graph.edge_label(id))
        .filter(|label| !label.text.is_empty())
        .cloned()
    {
        label.position = LabelPosition::InsideMiddleCenter;
        result.edge_labels.insert(id, label);
    }
}

/// Intersect a center-to-center segment with an object's rectangular layout
/// box. Cross-scope Dagre edges are published after the parent projection has
/// moved both endpoints; using centers here would make the renderer shorten
/// the wire from inside the shapes, unlike D2's final TraceToShape pass.
fn layout_boundary_intersection(rect: Rect, toward: Point) -> Point {
    let center = rect.center();
    let dx = toward.x - center.x;
    let dy = toward.y - center.y;
    if dx.abs() <= f64::EPSILON && dy.abs() <= f64::EPSILON {
        return center;
    }
    let horizontal = if dx.abs() > f64::EPSILON {
        (rect.size.width / 2.0) / dx.abs()
    } else {
        f64::INFINITY
    };
    let vertical = if dy.abs() > f64::EPSILON {
        (rect.size.height / 2.0) / dy.abs()
    } else {
        f64::INFINITY
    };
    let scale = horizontal.min(vertical);
    Point {
        // d2geo's `IntersectionPoint` rounds the offset from the segment
        // start before the target adapter publishes the route. Preserve that
        // integer border point for cross-scope wires; the renderer applies
        // the later arrowhead adjustment in floating point.
        x: (center.x + dx * scale).round(),
        y: (center.y + dy * scale).round(),
    }
}

/// Match d2layouts.LayoutNested's extraction boundary for nested grids.
///
/// The ordinary TALA engine must see a grid object as one already-sized leaf;
/// feeding it the grid's descendants changes parent packing and RNG traversal.
/// We first obtain those sizes from the complete measured graph, lay out a
/// projection with grid descendants removed, then translate the nested result
/// back relative to each grid placeholder.
fn layout_tala_with_nested_grids(
    graph: &Graph,
    options: &LayoutOptions,
    cancellation: Option<&CancellationToken>,
    defer_parent_routing: bool,
) -> Result<LayoutResult, LayoutDispatchError> {
    let mut initial = empty_layout_result(options);
    for (id, node) in graph.nodes() {
        initial.boxes.insert(
            id,
            Rect {
                origin: Point::default(),
                size: node.size,
            },
        );
    }
    initial = layout_nested_grid_cells(
        graph,
        initial,
        options,
        None,
        EngineKind::Tala,
        cancellation,
    )?;
    #[cfg(feature = "diagnostic-traces")]
    grid::trace_grid_snapshot(graph, &initial, None, "layout-final-pre-grid");
    let mut nested =
        grid::apply_scopes(graph, initial, options).map_err(LayoutDispatchError::Tala)?;
    reroute_nested_grid_external_edges(graph, &mut nested, options, cancellation)?;

    let visible_ids = graph
        .nodes()
        .filter_map(|(id, _node)| closest_grid_scope(graph, id).is_none().then_some(id))
        .collect::<Vec<_>>();
    if visible_ids.len() == graph.nodes().len() {
        return Ok(nested);
    }

    let mut collapsed_source = graph.clone();
    for id in visible_ids.iter().copied() {
        if let Some(rect) = nested.boxes.get(&id)
            && let Some(node) = collapsed_source.node_mut(id)
        {
            node.size = rect.size;
            // LayoutNested fits the extracted grid before parent layout,
            // but InjectNested publishes its label/icon state only afterwards.
            // Keep authored positions on the childless placeholder: automatic
            // labels must participate in the parent's PlaceLabels pass and
            // become obstacles for later siblings. The nested label state is
            // restored below, after the parent pass has finished.
            if node.grid_rows.is_some() || node.grid_columns.is_some() {
                if node.external_label.is_some_and(|label| label.automatic) {
                    node.label_position = LabelPosition::Unset;
                    node.external_label = None;
                }
                // A placeholder has already been fitted by its nested grid;
                // parent TALA must preserve that size while packing siblings.
                node.fixed_width = true;
                node.fixed_height = true;
            }
            #[cfg(feature = "diagnostic-traces")]
            if diagnostic_env_enabled("DAGRE_DEBUG_NESTED")
                && (node.grid_rows.is_some() || node.grid_columns.is_some())
            {
                eprintln!(
                    "COLLAPSED_PLACEHOLDER id={:?} label_position={:?} label={:?} external={:?}",
                    id,
                    node.label_position,
                    nested.node_labels.get(&id),
                    node.external_label
                );
            }
        }
    }
    let (projected, node_ids, edge_ids) = project_graph(&collapsed_source, &visible_ids, false);
    let projected_result = match cancellation {
        Some(token) => d2_rs_tala::Engine
            .layout_with_cancellation(&projected, options, token)
            .map_err(LayoutDispatchError::Tala)?,
        None => d2_rs_tala::Engine
            .layout(&projected, options)
            .map_err(LayoutDispatchError::Tala)?,
    };
    let mut result = remap_result(projected_result, &node_ids, &edge_ids);

    // The extracted placeholder has no children in the parent projection, so
    // a fresh TALA pass would infer its ordinary leaf default
    // (inside-middle-center).  d2layouts.LayoutNested keeps the nested
    // diagram's target label state when it reinserts that placeholder; carry
    // the same state across so rendering and target hashes see the original
    // grid-scope placement.
    for (id, node) in graph.nodes() {
        if node.grid_rows.is_none() && node.grid_columns.is_none() {
            continue;
        }
        if let Some(label) = nested.node_labels.get(&id).cloned() {
            result.node_labels.insert(id, label);
        }
    }

    let mut scope_deltas = BTreeMap::new();
    for (id, node) in graph.nodes() {
        if node.grid_rows.is_none() && node.grid_columns.is_none() {
            continue;
        }
        let (Some(before), Some(after)) = (nested.boxes.get(&id), result.boxes.get(&id)) else {
            continue;
        };
        scope_deltas.insert(
            id,
            Point {
                x: after.origin.x - before.origin.x,
                y: after.origin.y - before.origin.y,
            },
        );
    }

    // Restore all descendants using the delta of their nearest extracted grid
    // scope. This is the same coordinate translation performed by
    // d2layouts.PositionNested after reinjection.
    for (id, _node) in graph.nodes() {
        if visible_ids.binary_search(&id).is_ok() {
            continue;
        }
        // Every descendant below a projected grid placeholder moves by the
        // placeholder's delta. A deeper grid can have its own local placement
        // in `nested`, but that grid is absent from the parent projection and
        // therefore has no `scope_deltas` entry. Use the outermost grid scope
        // below the visible parent so the entire already-laid-out subtree is
        // translated together.
        let Some(scope) = outermost_grid_scope(graph, id) else {
            continue;
        };
        let (Some(mut rect), Some(delta)) = (
            nested.boxes.get(&id).copied(),
            scope_deltas.get(&scope).copied(),
        ) else {
            continue;
        };
        rect.origin.x += delta.x;
        rect.origin.y += delta.y;
        result.boxes.insert(id, rect);
        if let Some(label) = nested.node_labels.get(&id).cloned() {
            result.node_labels.insert(id, label);
        }
    }
    result.removed_edges.extend(&nested.removed_edges);
    for (edge_id, edge) in graph.edges() {
        if result.removed_edges.contains(&edge_id) {
            continue;
        }
        if visible_ids.binary_search(&edge.source).is_ok()
            && visible_ids.binary_search(&edge.target).is_ok()
        {
            continue;
        }
        let source_scope = outermost_grid_scope(graph, edge.source);
        let target_scope = outermost_grid_scope(graph, edge.target);
        if source_scope.is_some()
            && source_scope == target_scope
            && let (Some(mut route), Some(delta)) = (
                nested.routes.get(&edge_id).cloned(),
                source_scope
                    .and_then(|scope| scope_deltas.get(&scope))
                    .copied(),
            )
        {
            for point in &mut route {
                point.x += delta.x;
                point.y += delta.y;
            }
            copy_route_kind(&mut result, edge_id, &nested, edge_id);
            result.routes.insert(edge_id, route);
            // InjectNested restores the whole edge, including the label
            // state committed by the nested grid/engine. The parent core
            // projection never saw this edge and has no replacement label.
            if let Some(label) = nested.edge_labels.get(&edge_id).cloned() {
                result.edge_labels.insert(edge_id, label);
                if nested.edge_label_percentage_commits.contains(&edge_id) {
                    result.edge_label_percentage_commits.insert(edge_id);
                }
            }
            continue;
        }
        if let (Some(source), Some(target)) = (
            result.boxes.get(&edge.source),
            result.boxes.get(&edge.target),
        ) {
            result
                .routes
                .insert(edge_id, vec![source.center(), target.center()]);
        }
    }
    for mut line in nested.sequence_lifelines.clone() {
        let delta = outermost_grid_scope(graph, line.actor)
            .and_then(|scope| scope_deltas.get(&scope).copied())
            .unwrap_or_default();
        for point in &mut line.route {
            point.x += delta.x;
            point.y += delta.y;
        }
        result.sequence_lifelines.push(line);
    }
    // The containing LayoutNested boundary restores constant-near diagrams
    // before routing all of its crossings together. A temporary base graph
    // must not commit routes or label state for that enclosing callback.
    if !defer_parent_routing {
        reroute_reinserted_grid_edges(graph, &mut result, options, EngineKind::Tala, cancellation)?;
    }
    Ok(result)
}

/// D2 routes edges crossing an extracted grid boundary after reinserting the
/// nested objects into the parent graph. At that point every node has its
/// final absolute position, so route only the cross-scope edges against the
/// complete hierarchy while preserving all those positions.
/// Restore edges crossing extracted sequence contents after reinjection.
/// Finish restored actor crossings inside the detached sequence graph, before
/// LayoutNested injects that graph into its parent.
fn reroute_sequence_actor_crossings(
    graph: &Graph,
    result: &mut LayoutResult,
    root: Option<NodeId>,
    actors: &[NodeId],
    options: &LayoutOptions,
    engine: EngineKind,
    cancellation: Option<&CancellationToken>,
) -> Result<(), LayoutDispatchError> {
    let owner = |id| {
        actors
            .iter()
            .copied()
            .find(|actor| id != *actor && node_in_scope(graph, id, *actor))
    };
    let members = graph
        .nodes()
        .filter_map(|(id, _)| {
            root.is_none_or(|root| id != root && node_in_scope(graph, id, root))
                .then_some(id)
        })
        .collect::<Vec<_>>();
    let (mut local_graph, nodes, edges) = project_graph(graph, &members, false);
    let selected = edges
        .iter()
        .enumerate()
        .filter_map(|(index, id)| {
            let edge = &graph.edges[id.0 as usize];
            let a = owner(edge.source);
            let b = owner(edge.target);
            (a != b && (a.is_some() || b.is_some())).then_some(EdgeId(index as u32))
        })
        .collect::<Vec<_>>();
    if selected.is_empty() {
        return Ok(());
    }
    let origin = root
        .and_then(|id| result.boxes.get(&id))
        .map_or(Point::default(), |rect| rect.origin);
    if let Some(root_node) = root.and_then(|id| graph.node(id)) {
        rebase_routing_identities(&mut local_graph, &root_node.external_id)?;
        let prefix = format!("{}.", root_node.external_id);
        for node in &mut local_graph.nodes {
            if let Some(name) = node.external_id.strip_prefix(&prefix) {
                node.external_id = name.to_owned();
            }
        }
    }
    local_graph.root_sequence_diagram = true;
    local_graph.root_grid_rows = None;
    local_graph.root_grid_columns = None;
    let mut local = empty_layout_result(options);
    for (index, original) in nodes.iter().enumerate() {
        let id = NodeId(index as u32);
        if let Some(rect) = result.boxes.get(original) {
            let mut rect = *rect;
            rect.origin.x -= origin.x;
            rect.origin.y -= origin.y;
            local.boxes.insert(id, rect);
        }
        if let Some(label) = result.node_labels.get(original) {
            local.node_labels.insert(id, *label);
        }
    }
    for (index, original) in edges.iter().enumerate() {
        let id = EdgeId(index as u32);
        if let Some(route) = result.routes.get(original) {
            local.routes.insert(
                id,
                route
                    .iter()
                    .map(|p| Point {
                        x: p.x - origin.x,
                        y: p.y - origin.y,
                    })
                    .collect(),
            );
        }
        if let Some(label) = result.edge_labels.get(original) {
            local.edge_labels.insert(id, label.clone());
        }
        if result.edge_label_percentage_commits.contains(original) {
            local.edge_label_percentage_commits.insert(id);
        }
    }
    for line in &result.sequence_lifelines {
        if let Some(index) = nodes.iter().position(|id| *id == line.actor) {
            let mut line = line.clone();
            line.actor = NodeId(index as u32);
            for p in &mut line.route {
                p.x -= origin.x;
                p.y -= origin.y;
            }
            local.sequence_lifelines.push(line);
        }
    }
    reroute_selected_grid_edges(
        &local_graph,
        &mut local,
        options,
        engine,
        cancellation,
        &selected,
    )?;
    for id in selected {
        let original = edges[id.0 as usize];
        if let Some(route) = local.routes.get(&id) {
            copy_route_kind(result, original, &local, id);
            result.routes.insert(
                original,
                route
                    .iter()
                    .map(|p| Point {
                        x: p.x + origin.x,
                        y: p.y + origin.y,
                    })
                    .collect(),
            );
        }
        if let Some(label) = local.edge_labels.get(&id) {
            result.edge_labels.insert(original, label.clone());
        }
        if local.edge_label_percentage_commits.contains(&id) {
            result.edge_label_percentage_commits.insert(original);
        }
    }
    Ok(())
}

fn outermost_sequence_scope(graph: &Graph, id: NodeId) -> Option<NodeId> {
    let mut current = graph.node(id).and_then(|node| node.parent);
    let mut owner = None;
    while let Some(id) = current {
        if graph.is_sequence_diagram(id) {
            owner = Some(id);
        }
        current = graph.node(id).and_then(|node| node.parent);
    }
    owner
}

fn reroute_sequence_external_edges(
    graph: &Graph,
    result: &mut LayoutResult,
    options: &LayoutOptions,
    engine: EngineKind,
    cancellation: Option<&CancellationToken>,
) -> Result<(), LayoutDispatchError> {
    let selected = graph
        .edges()
        .filter_map(|(id, edge)| {
            let source = outermost_sequence_scope(graph, edge.source);
            let target = outermost_sequence_scope(graph, edge.target);
            (source != target && (source.is_some() || target.is_some())).then_some(id)
        })
        .collect::<Vec<_>>();
    reroute_selected_grid_edges(graph, result, options, engine, cancellation, &selected)
}

fn reroute_reinserted_grid_edges(
    graph: &Graph,
    result: &mut LayoutResult,
    options: &LayoutOptions,
    engine: EngineKind,
    cancellation: Option<&CancellationToken>,
) -> Result<(), LayoutDispatchError> {
    let selected = graph
        .edges()
        .filter_map(|(id, edge)| {
            let sequence = outermost_sequence_scope(graph, edge.source);
            if sequence.is_some() && sequence == outermost_sequence_scope(graph, edge.target) {
                return None;
            }
            let source_scope = outermost_grid_scope(graph, edge.source);
            let target_scope = outermost_grid_scope(graph, edge.target);
            (source_scope != target_scope && (source_scope.is_some() || target_scope.is_some()))
                .then_some(id)
        })
        .collect::<Vec<_>>();
    reroute_selected_grid_edges(graph, result, options, engine, cancellation, &selected)
}

/// translateGraph collects depth-first ChildrenArray visits. Constant-near
/// roots are removed during core layout and appended by d2near by position
/// group before this parent routing callback; SaveOrder runs afterward.
fn routing_node_order(graph: &Graph) -> Vec<NodeId> {
    fn visit(graph: &Graph, id: NodeId, out: &mut Vec<NodeId>) {
        out.push(id);
        for child in ordered_hierarchy_children(graph, Some(id)) {
            visit(graph, child, out);
        }
    }
    let roots = ordered_hierarchy_children(graph, None);
    let near_group = |id| match graph.node(id).and_then(|node| node.canvas_position) {
        None => 0,
        Some(CanvasPosition::TopCenter | CanvasPosition::BottomCenter) => 1,
        Some(CanvasPosition::CenterLeft | CanvasPosition::CenterRight) => 2,
        Some(_) => 3,
    };
    let mut roots = roots;
    roots.sort_by_key(|id| near_group(*id));
    let mut order = Vec::with_capacity(graph.nodes.len());
    for root in roots {
        visit(graph, root, &mut order);
    }
    order
}

fn root_grid_routing_edge_order(graph: &Graph, selected: &[EdgeId]) -> Vec<EdgeId> {
    let selected_set = selected.iter().copied().collect::<BTreeSet<_>>();
    if graph.root_grid_rows.is_none() && graph.root_grid_columns.is_none() {
        let scopes = nested_dagre_scope_plan(graph);
        let owner = |id| {
            scopes.iter().position(|scope| {
                let (root, include_self) = match *scope {
                    NestedDagreScope::ConstantNear(root) => (root, true),
                    NestedDagreScope::Grid(root) | NestedDagreScope::Sequence(root) => {
                        (root, false)
                    }
                };
                (include_self || id != root) && node_in_scope(graph, id, root)
            })
        };
        let mut fixed = graph
            .edges()
            .filter_map(|(id, edge)| {
                if selected_set.contains(&id) {
                    return None;
                }
                let source = owner(edge.source);
                let target = owner(edge.target);
                let key = match source.filter(|_| source == target) {
                    None => (0, 0, 0),
                    Some(index) => match scopes[index] {
                        NestedDagreScope::ConstantNear(root) => {
                            let group = match graph.node(root).and_then(|node| node.canvas_position)
                            {
                                Some(CanvasPosition::TopCenter | CanvasPosition::BottomCenter) => 0,
                                Some(CanvasPosition::CenterLeft | CanvasPosition::CenterRight) => 1,
                                _ => 2,
                            };
                            (1, group, index)
                        }
                        _ => (2, 0, index),
                    },
                };
                Some((key, id))
            })
            .collect::<Vec<_>>();
        // Go retains parent edges, injects near graphs by position group,
        // injects other extracted graphs, then appends restored crossings.
        // Stable order also keeps new lifelines after their sequence messages.
        fixed.sort_by_key(|(key, _)| *key);
        return fixed
            .into_iter()
            .map(|(_, id)| id)
            .chain(selected.iter().copied())
            .collect();
    }
    grid_routing_edge_order(graph, selected)
}

/// A grid retains its direct-cell edges, then InjectNested appends each
/// completed cell graph, then LayoutNested restores the selected crossings.
fn grid_routing_edge_order(graph: &Graph, selected: &[EdgeId]) -> Vec<EdgeId> {
    let selected_set = selected.iter().copied().collect::<BTreeSet<_>>();
    let roots = ordered_hierarchy_children(graph, None)
        .into_iter()
        .enumerate()
        .map(|(index, id)| (id, index))
        .collect::<BTreeMap<_, _>>();
    let mut fixed = graph
        .edges()
        .filter_map(|(id, edge)| {
            if selected_set.contains(&id) {
                return None;
            }
            let source_cell = root_grid_cell(graph, edge.source);
            let target_cell = root_grid_cell(graph, edge.target);
            let key = if source_cell == Some(edge.source) && target_cell == Some(edge.target) {
                (0, 0)
            } else if source_cell == target_cell {
                (
                    1,
                    source_cell
                        .and_then(|id| roots.get(&id).copied())
                        .unwrap_or(usize::MAX),
                )
            } else {
                (0, 0)
            };
            Some((key, id))
        })
        .collect::<Vec<_>>();
    fixed.sort_by_key(|(key, _)| *key);
    fixed
        .into_iter()
        .map(|(_, id)| id)
        .chain(selected.iter().copied())
        .collect()
}

/// SaveOrder retains original edges before newly generated lifelines. Within
/// those new edges, the core sequence's own lifelines precede InjectNested's
/// child graphs, and those graphs follow the LayoutNested queue (not the order
/// in which the native pre-sizing passes happen to finish).
fn sequence_injection_order(graph: &Graph) -> Vec<Option<NodeId>> {
    fn collect(
        graph: &Graph,
        root: Option<NodeId>,
        diagram: DiagramType,
        include_self: bool,
    ) -> Vec<Option<NodeId>> {
        let mut queue = if include_self {
            root.into_iter().collect::<Vec<_>>()
        } else {
            ordered_hierarchy_children(graph, root)
        };
        let mut cursor = 0;
        let mut visited = BTreeSet::new();
        let mut near_graphs = Vec::new();
        let mut extracted = Vec::new();
        while cursor < queue.len() {
            let id = queue[cursor];
            cursor += 1;
            if !visited.insert(id) {
                continue;
            }
            let Some(node) = graph.node(id) else {
                continue;
            };
            let children = ordered_hierarchy_children(graph, Some(id));
            let is_near = node.canvas_position.is_some() && !(include_self && root == Some(id));
            let is_grid = node.grid_rows.is_some() || node.grid_columns.is_some();
            let is_sequence = graph.is_sequence_diagram(id);
            if is_near {
                let group = match node.canvas_position {
                    Some(CanvasPosition::TopCenter | CanvasPosition::BottomCenter) => 0,
                    Some(CanvasPosition::CenterLeft | CanvasPosition::CenterRight) => 1,
                    _ => 2,
                };
                near_graphs.push((group, collect(graph, Some(id), DiagramType::Default, true)));
            } else if !children.is_empty() && (is_grid || is_sequence) {
                extracted.push(collect(
                    graph,
                    Some(id),
                    if is_sequence {
                        DiagramType::Sequence
                    } else {
                        DiagramType::Grid
                    },
                    false,
                ));
            } else if diagram == DiagramType::Grid && node.parent == root && !children.is_empty() {
                // Ordinary container cells first run includeSelf=true, then
                // are extracted without self for the grid's reinsertion.
                extracted.push(collect(graph, Some(id), DiagramType::Default, true));
            } else {
                queue.extend(children);
            }
        }
        near_graphs.sort_by_key(|(group, _)| *group);
        let mut order = Vec::new();
        if diagram == DiagramType::Sequence {
            order.push(root);
        }
        for (_, children) in near_graphs {
            order.extend(children);
        }
        for children in extracted {
            order.extend(children);
        }
        order
    }
    collect(
        graph,
        None,
        if graph.root_sequence_diagram {
            DiagramType::Sequence
        } else if graph.root_grid_rows.is_some() || graph.root_grid_columns.is_some() {
            DiagramType::Grid
        } else {
            DiagramType::Default
        },
        false,
    )
}

fn append_fixed_sequence_lifelines(
    route_graph: &mut Graph,
    lines: &[d2_rs_core::SequenceLifeline],
) {
    let scope_order = sequence_injection_order(route_graph);
    let mut lines = lines.iter().collect::<Vec<_>>();
    lines.sort_by_key(|line| {
        let scope = nearest_sequence_scope(route_graph, line.actor);
        scope_order
            .iter()
            .position(|candidate| *candidate == scope)
            .unwrap_or(usize::MAX)
    });
    for line in lines {
        let actor = line.actor;
        let route = line.route.clone();
        let id = route_graph.add_edge(d2_rs_core::Edge {
            source: actor,
            target: actor,
        });
        let node = route_graph
            .node(actor)
            .expect("lifeline actor exists")
            .clone();
        let parent = node.parent.and_then(|id| route_graph.node(id));
        let prefix = parent.map_or(String::new(), |node| format!("{}.", node.external_id));
        let local = node
            .external_id
            .strip_prefix(&prefix)
            .unwrap_or(&node.external_id);
        // d2TALAEdgeAbsID replaces the synthetic destination with its
        // actor before identity allocation.
        let identity = format!("{prefix}({local} -- {local})[0]");
        let hash = identity.bytes().fold(2_166_136_261u32, |h, b| {
            (h ^ u32::from(b)).wrapping_mul(16_777_619)
        });
        route_graph.tala_edge_ids[id.0 as usize] = Some(i64::from(hash));
        #[cfg(feature = "diagnostic-traces")]
        if diagnostic_env_enabled("D2_TRACE_LIFELINE_CONTEXT") {
            eprintln!(
                "LIFELINE_CONTEXT actor={} identity={} points={}",
                node.external_id,
                identity,
                route
                    .iter()
                    .map(|p| format!("{:016x},{:016x}", p.x.to_bits(), p.y.to_bits()))
                    .collect::<Vec<_>>()
                    .join(";")
            );
        }
        route_graph.set_edge_route(id, route);
        // translateGraph reserves twice lib/label.PADDING even for an
        // unlabeled lifeline.
        route_graph.set_edge_min_dimensions(id, Some(10.0), Some(10.0));
    }
}

fn reroute_selected_grid_edges(
    graph: &Graph,
    result: &mut LayoutResult,
    options: &LayoutOptions,
    engine: EngineKind,
    cancellation: Option<&CancellationToken>,
    selected: &[EdgeId],
) -> Result<(), LayoutDispatchError> {
    if selected.is_empty() {
        return Ok(());
    }

    let selected_set = selected.iter().copied().collect::<BTreeSet<_>>();
    let mut route_graph = graph.clone();
    for (id, _) in graph.nodes() {
        let Some(rect) = result.boxes.get(&id) else {
            continue;
        };
        let Some(node) = route_graph.node_mut(id) else {
            continue;
        };
        node.size = rect.size;
        if let Some(shape) = sequence::routing_shape(graph, id) {
            node.shape = shape;
        }
        // Retained routing geometry is not a new fixed-position constraint.
        // The upstream RouteEdges adapter derives FixedTopLeft only from the
        // original left/top attributes, independently of existing TopLeft.
        node.locked_position = node.locked_position.map(|_| rect.origin);
        if let Some(label) = result.node_labels.get(&id) {
            node.label_position = label.position;
            node.label_size = label.size;
            node.font_size = label.font_size;
            node.icon_position = label.icon_position;
            if route_graph.node_font_size_declarations.contains_key(&id) {
                route_graph
                    .node_font_size_declarations
                    .insert(id, label.font_size);
            }
        }
    }
    for (id, _) in graph.nodes() {
        if let Some(rect) = result.boxes.get(&id) {
            route_graph.set_initial_position(id, Some(rect.origin));
        }
    }
    for (id, _) in graph.edges() {
        if selected_set.contains(&id) {
            route_graph.set_edge_route(id, Vec::new());
        } else if let Some(route) = result.routes.get(&id) {
            route_graph.set_edge_route(id, route.clone());
        }
        if let Some(label) = result.edge_labels.get(&id) {
            route_graph.set_edge_label(id, Some(label.clone()));
            if result.edge_label_percentage_commits.contains(&id) {
                route_graph.set_edge_label_percentage_present(id, true);
            }
        }
    }

    if matches!(engine, EngineKind::Tala) {
        append_fixed_sequence_lifelines(&mut route_graph, &result.sequence_lifelines);
        // LayoutNested retains grid-owned edges, then InjectNested appends
        // each child graph, then restores external edges. The routing graph
        // must carry that order with all dense-indexed metadata permuted.
        route_graph.set_node_order_override(routing_node_order(&route_graph));
        let order = root_grid_routing_edge_order(&route_graph, selected);
        let nodes = route_graph.nodes().map(|(id, _)| id).collect::<Vec<_>>();
        let (route_graph, _, old_edges) =
            project_graph_in_edge_order(&route_graph, &nodes, false, Some(&order));
        let permutation = old_edges
            .iter()
            .enumerate()
            .map(|(index, id)| (*id, EdgeId(index as u32)))
            .collect::<BTreeMap<_, _>>();
        let routed_selected = selected
            .iter()
            .map(|id| permutation[id])
            .collect::<Vec<_>>();
        let routed = LayoutDispatcher.route_edges_selected_with_cancellation(
            &route_graph,
            engine,
            options,
            &routed_selected,
            cancellation,
        )?;
        #[cfg(feature = "diagnostic-traces")]
        grid::trace_external_routes(
            &route_graph,
            &routed,
            &routed_selected,
            None,
            "reinserted-tala",
        );
        for (edge_id, projected_id) in selected.iter().copied().zip(routed_selected) {
            if let Some(route) = routed.routes.get(&projected_id) {
                copy_route_kind(result, edge_id, &routed, projected_id);
                result.routes.insert(edge_id, route.clone());
            }
            if let Some(label) = routed.edge_labels.get(&projected_id).cloned() {
                result.edge_labels.insert(edge_id, label);
                if routed.edge_label_percentage_commits.contains(&projected_id) {
                    result.edge_label_percentage_commits.insert(edge_id);
                }
            }
        }
    } else {
        // D2 v0.9.0 exposes edge routing as a plugin feature independently
        // from its layout engine. Dagre and ELK fall back to DefaultRouter,
        // which connects shape-boundary endpoints with a straight segment.
        for edge_id in selected {
            let Some(edge) = graph
                .edges()
                .find_map(|(id, edge)| (id == *edge_id).then_some(edge))
            else {
                continue;
            };
            let (Some(source), Some(target)) = (
                result.boxes.get(&edge.source),
                result.boxes.get(&edge.target),
            ) else {
                continue;
            };
            let route = d2_rs_dagre::route_straight_edge(&route_graph, *edge_id, *source, *target)
                .unwrap_or_else(|| vec![source.center(), target.center()]);
            result
                .route_curves
                .insert(*edge_id, graph.edge_is_curve(*edge_id));
            result.routes.insert(*edge_id, route);
            set_default_router_edge_label(graph, result, *edge_id);
        }
        #[cfg(feature = "diagnostic-traces")]
        grid::trace_external_routes(&route_graph, result, selected, None, "reinserted-straight");
    }
    Ok(())
}

/// D2 extracts edges that cross a grid-cell container boundary and routes the
/// restored edges through TALA after grid placement. The grid adapter's simple
/// straight route is only the fallback for edges whose endpoints both remain
/// direct grid children.
fn reroute_nested_grid_external_edges(
    graph: &Graph,
    result: &mut LayoutResult,
    options: &LayoutOptions,
    cancellation: Option<&CancellationToken>,
) -> Result<(), LayoutDispatchError> {
    reroute_grid_scope_external_edges(graph, result, options, cancellation, true, EngineKind::Tala)
}

fn ordered_grid_routing_scopes(graph: &Graph) -> Vec<NodeId> {
    fn visit(graph: &Graph, id: NodeId, output: &mut Vec<NodeId>) {
        for child in ordered_hierarchy_children(graph, Some(id)) {
            visit(graph, child, output);
        }
        if graph
            .node(id)
            .is_some_and(|node| node.grid_rows.is_some() || node.grid_columns.is_some())
        {
            output.push(id);
        }
    }
    let mut scopes = Vec::new();
    for root in ordered_hierarchy_children(graph, None) {
        visit(graph, root, &mut scopes);
    }
    scopes
}

/// Crossings belong to the first extracted cell whose descendants they touch.
fn order_grid_crossings(graph: &Graph, scope: Option<NodeId>, edges: &mut [EdgeId]) {
    let cells = ordered_hierarchy_children(graph, scope);
    let owner = |id| {
        let cell = scope.map_or_else(
            || root_grid_child(graph, id),
            |scope| grid::direct_grid_child(graph, id, scope),
        )?;
        if cell == id {
            return None;
        }
        cells.iter().position(|candidate| *candidate == cell)
    };
    edges.sort_by_key(|id| {
        let edge = &graph.edges[id.0 as usize];
        owner(edge.source)
            .into_iter()
            .chain(owner(edge.target))
            .min()
            .unwrap_or(usize::MAX)
    });
}

/// Allocate IDs under an empty-ID extracted root while retaining public names.
fn rebase_routing_identities(
    graph: &mut Graph,
    scope_name: &str,
) -> Result<(), LayoutDispatchError> {
    let prefix = format!("{scope_name}.");
    let mut identity_graph = graph.clone();
    for node in &mut identity_graph.nodes {
        if let Some(local) = node.external_id.strip_prefix(&prefix) {
            node.external_id = local.to_owned();
        }
    }
    let names = identity_graph
        .nodes()
        .map(|(_, node)| node.external_id.clone())
        .collect::<Vec<_>>();
    graph.tala_id_overrides = d2_rs_core::identity::allocate(
        &names.iter().map(|name| name.as_bytes()).collect::<Vec<_>>(),
    )
    .map_err(|index| LayoutDispatchError::RepeatedIdentity {
        kind: "object",
        identity: names[index].clone(),
    })?
    .into_iter()
    .map(Some)
    .collect();
    let names = identity_graph
        .edges()
        .map(|(id, _)| {
            d2_rs_core::edge_abs_id(&identity_graph, id)
                .ok_or(CoreLayoutError::MissingEndpoint { edge: id })
        })
        .collect::<Result<Vec<_>, _>>()?;
    graph.tala_edge_ids = d2_rs_core::identity::allocate(
        &names.iter().map(|name| name.as_bytes()).collect::<Vec<_>>(),
    )
    .map_err(|index| LayoutDispatchError::RepeatedIdentity {
        kind: "edge",
        identity: names[index].clone(),
    })?
    .into_iter()
    .map(Some)
    .collect();
    Ok(())
}

fn reroute_grid_scope_external_edges(
    graph: &Graph,
    result: &mut LayoutResult,
    options: &LayoutOptions,
    cancellation: Option<&CancellationToken>,
    include_root: bool,
    engine: EngineKind,
) -> Result<(), LayoutDispatchError> {
    // LayoutNested completes child diagrams (including their crossing labels)
    // before restoring parent crossings. Parent routing reserves those labels
    // as obstacles, so arena/preorder traversal can expose an unset position.
    let mut scopes = ordered_grid_routing_scopes(graph)
        .into_iter()
        .map(Some)
        .collect::<Vec<_>>();
    if include_root && (graph.root_grid_rows.is_some() || graph.root_grid_columns.is_some()) {
        scopes.push(None);
    }
    if scopes.is_empty() {
        return Ok(());
    }

    for scope in scopes {
        let members = graph
            .nodes()
            .filter_map(|(id, _)| {
                scope
                    .is_none_or(|scope| id != scope && node_in_scope(graph, id, scope))
                    .then_some(id)
            })
            .collect::<Vec<_>>();
        if members.is_empty() {
            continue;
        }
        let member_set = members.iter().copied().collect::<BTreeSet<_>>();
        let mut original_selected = Vec::new();
        for (edge_id, edge) in graph.edges() {
            if !member_set.contains(&edge.source) || !member_set.contains(&edge.target) {
                continue;
            }
            let (Some(source_cell), Some(target_cell)) = (
                scope.map_or_else(
                    || root_grid_child(graph, edge.source),
                    |scope| grid::direct_grid_child(graph, edge.source, scope),
                ),
                scope.map_or_else(
                    || root_grid_child(graph, edge.target),
                    |scope| grid::direct_grid_child(graph, edge.target, scope),
                ),
            ) else {
                continue;
            };
            if source_cell != target_cell
                && (edge.source != source_cell || edge.target != target_cell)
            {
                original_selected.push(edge_id);
            }
        }
        order_grid_crossings(graph, scope, &mut original_selected);
        // The crossing selection reads only the source graph. Without a
        // crossing, LayoutNested restores nothing in this scope: skip building the
        // routing projection it would discard.
        if original_selected.is_empty() {
            continue;
        }
        let (mut route_graph, node_ids, edge_ids) = project_graph(graph, &members, false);
        if let Some(scope) = scope {
            if let Some(node) = graph.node(scope) {
                rebase_routing_identities(&mut route_graph, &node.external_id)?;
            }
            route_graph.root_grid_rows = None;
            route_graph.root_grid_columns = None;
        }
        let edge_map = edge_ids
            .iter()
            .enumerate()
            .map(|(index, id)| (*id, EdgeId(index as u32)))
            .collect::<BTreeMap<_, _>>();
        let scope_origin = scope
            .and_then(|scope| result.boxes.get(&scope).map(|rect| rect.origin))
            .unwrap_or_default();

        for (index, original_id) in node_ids.iter().copied().enumerate() {
            let Some(rect) = result.boxes.get(&original_id).copied() else {
                continue;
            };
            let route_node = &mut route_graph.nodes[index];
            route_node.size = rect.size;
            if let Some(shape) = sequence::routing_shape(graph, original_id) {
                route_node.shape = shape;
            }
            if let Some(label) = result.node_labels.get(&original_id) {
                route_node.label_position = label.position;
                route_node.label_size = label.size;
                route_node.font_size = label.font_size;
                route_node.icon_position = label.icon_position;
                if route_graph
                    .node_font_size_declarations
                    .contains_key(&NodeId(index as u32))
                {
                    route_graph
                        .node_font_size_declarations
                        .insert(NodeId(index as u32), label.font_size);
                }
            }
            let retained_position = Point {
                x: rect.origin.x - scope_origin.x,
                y: rect.origin.y - scope_origin.y,
            };
            route_node.locked_position = route_node.locked_position.map(|_| retained_position);
            route_graph.set_initial_position(NodeId(index as u32), Some(retained_position));
        }
        for (original_id, projected_id) in &edge_map {
            if let Some(route) = result.routes.get(original_id) {
                let route = route
                    .iter()
                    .map(|point| Point {
                        x: point.x - scope_origin.x,
                        y: point.y - scope_origin.y,
                    })
                    .collect();
                route_graph.set_edge_route(*projected_id, route);
            }
        }

        for (original_id, projected_id) in &edge_map {
            if let Some(label) = result.edge_labels.get(original_id) {
                route_graph.set_edge_label(*projected_id, Some(label.clone()));
                if result.edge_label_percentage_commits.contains(original_id) {
                    route_graph.set_edge_label_percentage_present(*projected_id, true);
                }
            }
        }

        if !matches!(engine, EngineKind::Tala) {
            // LayoutNested restores crossings at every extracted grid scope,
            // even if both endpoints remain inside one enclosing root cell.
            // DefaultRouter traces shapes before the enclosing layout shifts
            // the extracted graph. Shape-border rounding must use that local
            // frame, including when the later translation is fractional.
            for original_id in original_selected {
                let Some(&projected_id) = edge_map.get(&original_id) else {
                    continue;
                };
                let Some(edge) = graph.edge(original_id) else {
                    continue;
                };
                let (Some(mut source), Some(mut target)) = (
                    result.boxes.get(&edge.source).copied(),
                    result.boxes.get(&edge.target).copied(),
                ) else {
                    continue;
                };
                for rect in [&mut source, &mut target] {
                    rect.origin.x -= scope_origin.x;
                    rect.origin.y -= scope_origin.y;
                }
                if let Some(mut route) =
                    d2_rs_dagre::route_straight_edge(&route_graph, projected_id, source, target)
                {
                    for point in &mut route {
                        point.x += scope_origin.x;
                        point.y += scope_origin.y;
                    }
                    result
                        .route_curves
                        .insert(original_id, graph.edge_is_curve(original_id));
                    result.routes.insert(original_id, route);
                    set_default_router_edge_label(graph, result, original_id);
                }
            }
            continue;
        }
        let selected = original_selected
            .iter()
            .filter_map(|id| edge_map.get(id).copied())
            .collect::<Vec<_>>();
        if selected.is_empty() {
            continue;
        }
        for edge in &selected {
            route_graph.set_edge_route(*edge, Vec::new());
        }
        // Reinjected sequence lifelines are fixed edges in the source grid
        // graph, and participate in routing obstacles and incident topology.
        let projected_nodes = node_ids
            .iter()
            .enumerate()
            .map(|(index, &id)| (id, NodeId(index as u32)))
            .collect::<BTreeMap<_, _>>();
        let local_lifelines = result
            .sequence_lifelines
            .iter()
            .filter_map(|line| {
                let mut local = line.clone();
                local.actor = *projected_nodes.get(&line.actor)?;
                for point in &mut local.route {
                    point.x -= scope_origin.x;
                    point.y -= scope_origin.y;
                }
                Some(local)
            })
            .collect::<Vec<_>>();
        append_fixed_sequence_lifelines(&mut route_graph, &local_lifelines);
        if let Some(scope) = scope {
            if let Some(node) = graph.node(scope) {
                rebase_routing_identities(&mut route_graph, &node.external_id)?;
            }
        }
        // InjectNested appends fixed child edges first, then restored crossings
        // in first-extraction order. Reproject metadata alongside dense IDs.
        let order = grid_routing_edge_order(&route_graph, &selected);
        let nodes = route_graph.nodes().map(|(id, _)| id).collect::<Vec<_>>();
        route_graph.set_node_order_override(routing_node_order(&route_graph));
        let (route_graph, _, old_edges) =
            project_graph_in_edge_order(&route_graph, &nodes, false, Some(&order));
        let permutation = old_edges
            .iter()
            .enumerate()
            .map(|(index, id)| (*id, EdgeId(index as u32)))
            .collect::<BTreeMap<_, _>>();
        let selected = selected
            .iter()
            .map(|id| permutation[id])
            .collect::<Vec<_>>();

        let routed = match cancellation {
            Some(token) => d2_rs_tala::Engine.route_edges_selected_with_cancellation(
                &route_graph,
                options,
                &selected,
                token,
            ),
            None => d2_rs_tala::Engine.route_edges_selected(&route_graph, options, &selected),
        }
        .map_err(LayoutDispatchError::Tala)?;
        #[cfg(feature = "diagnostic-traces")]
        grid::trace_external_routes(
            &route_graph,
            &routed,
            &selected,
            scope
                .and_then(|id| graph.node(id))
                .map(|node| node.external_id.as_str()),
            "grid-scope",
        );
        for (original_id, projected_id) in original_selected.into_iter().zip(selected) {
            if let Some(label) = routed.edge_labels.get(&projected_id) {
                result.edge_labels.insert(original_id, label.clone());
                if routed.edge_label_percentage_commits.contains(&projected_id) {
                    result.edge_label_percentage_commits.insert(original_id);
                }
            }
            if let Some(route) = routed.routes.get(&projected_id) {
                copy_route_kind(result, original_id, &routed, projected_id);
                result.routes.insert(
                    original_id,
                    route
                        .iter()
                        .map(|point| Point {
                            x: point.x + scope_origin.x,
                            y: point.y + scope_origin.y,
                        })
                        .collect(),
                );
            }
        }
    }
    Ok(())
}

fn root_grid_child(graph: &Graph, id: NodeId) -> Option<NodeId> {
    let mut current = id;
    loop {
        let node = graph.node(current)?;
        let Some(parent) = node.parent else {
            return Some(current);
        };
        current = parent;
    }
}

fn closest_grid_scope(graph: &Graph, id: NodeId) -> Option<NodeId> {
    let mut parent = match nearest_sequence_scope(graph, id) {
        Some(sequence) => graph.node(sequence).and_then(|node| node.parent),
        None => graph.node(id).and_then(|node| node.parent),
    };
    while let Some(candidate) = parent {
        if graph
            .node(candidate)
            .is_some_and(|node| node.grid_rows.is_some() || node.grid_columns.is_some())
        {
            return Some(candidate);
        }
        parent = graph.node(candidate).and_then(|node| node.parent);
    }
    None
}

/// Finds the outermost nested grid that owns a node's extracted subtree.
/// Nested layouts have already established all coordinates relative to that
/// outer scope; when the parent engine repositions the visible placeholder,
/// every deeper grid and node must receive the same translation.
fn outermost_grid_scope(graph: &Graph, id: NodeId) -> Option<NodeId> {
    let mut parent = match nearest_sequence_scope(graph, id) {
        Some(sequence) => graph.node(sequence).and_then(|node| node.parent),
        None => graph.node(id).and_then(|node| node.parent),
    };
    let mut outermost = None;
    while let Some(candidate) = parent {
        if graph
            .node(candidate)
            .is_some_and(|node| node.grid_rows.is_some() || node.grid_columns.is_some())
        {
            outermost = Some(candidate);
        }
        parent = graph.node(candidate).and_then(|node| node.parent);
    }
    outermost
}

fn inside_sequence_scope(graph: &Graph, id: NodeId) -> bool {
    graph.is_sequence_diagram(id) || nearest_sequence_scope(graph, id).is_some()
}

fn is_sequence_descendant(graph: &Graph, id: NodeId) -> bool {
    nearest_sequence_scope(graph, id).is_some()
}

fn nearest_sequence_scope(graph: &Graph, id: NodeId) -> Option<NodeId> {
    let mut parent = graph.node(id).and_then(|node| node.parent);
    while let Some(candidate) = parent {
        if graph.is_sequence_diagram(candidate) {
            return Some(candidate);
        }
        parent = graph.node(candidate).and_then(|node| node.parent);
    }
    None
}

fn empty_layout_result(options: &LayoutOptions) -> LayoutResult {
    LayoutResult {
        removed_edges: Default::default(),
        source_edge_storage: Default::default(),
        synthetic_root: None,
        route_curves: Default::default(),
        sequence_lifelines: Vec::new(),
        edge_label_percentage_commits: Default::default(),
        boxes: BTreeMap::new(),
        routes: BTreeMap::new(),
        edge_labels: BTreeMap::new(),
        node_labels: BTreeMap::new(),
        report: LayoutReport {
            selected_seed: options.seeds.first().copied().unwrap_or_default(),
            selected_pass: 0,
            score: Default::default(),
            improvement_passes: 0,
            candidates: Vec::new(),
            warnings: Vec::new(),
        },
    }
}

fn project_graph(
    source: &Graph,
    node_ids: &[NodeId],
    constant_scope: bool,
) -> (Graph, Vec<NodeId>, Vec<EdgeId>) {
    project_graph_in_edge_order(source, node_ids, constant_scope, None)
}

fn project_graph_in_edge_order(
    source: &Graph,
    node_ids: &[NodeId],
    constant_scope: bool,
    order: Option<&[EdgeId]>,
) -> (Graph, Vec<NodeId>, Vec<EdgeId>) {
    let node_map = node_ids
        .iter()
        .enumerate()
        .map(|(index, id)| (*id, NodeId(index as u32)))
        .collect::<BTreeMap<_, _>>();
    let mut projected = source.clone();
    projected.nodes = node_ids
        .iter()
        .filter_map(|id| source.node(*id).cloned())
        .collect();
    if constant_scope {
        // LayoutNested treats an extracted constant-near graph as an ordinary
        // graph, regardless of the enclosing root's diagram type.
        projected.root_sequence_diagram = false;
        // ExtractSubgraph creates a fresh root instead of inheriting the
        // enclosing document's grid. The near object keeps its own grid.
        projected.root_grid_rows = None;
        projected.root_grid_columns = None;
        // Go's ExtractSubgraph(includeSelf=true) copies the near object's
        // attributes onto the extracted graph root. Promote that object's
        // authored direction to the graph boundary; otherwise a nested
        // grid leaves the projected Dagre root at the document direction.
        let root_direction = projected
            .nodes
            .iter()
            .find(|node| {
                node.parent
                    .is_none_or(|parent| !node_map.contains_key(&parent))
            })
            .map(|node| node.direction.unwrap_or_default());
        if let Some(direction) = root_direction {
            projected.direction = direction;
        }
    }
    for (index, node) in projected.nodes.iter_mut().enumerate() {
        node.parent = node
            .parent
            .and_then(|parent| node_map.get(&parent).copied());
        node.near = node.near.and_then(|near| node_map.get(&near).copied());
        if constant_scope && node.canvas_position.is_some() {
            node.canvas_position = None;
            // LayoutNested temporarily clears curr.NearKey while laying out
            // its extracted constant-near subgraph (d2layouts.go237).
            node.d2_near_key = None;
        }
        // `index` is used below to keep this loop visibly tied to the
        // projected dense identity, even when source IDs have gaps.
        let _ = index;
    }

    projected.tala_id_overrides = copy_node_metadata(&source.tala_id_overrides, node_ids);
    projected.container_flags = copy_node_metadata(&source.container_flags, node_ids);
    // `ExtractSubgraph(..., includeSelf=false)` clears a grid placeholder's
    // children before the parent layout.  Its source object remains in the
    // parent graph, but it is no longer a container for TALA's root packing.
    // The shared graph metadata still carries the original container bit, so
    // clear it only for grid placeholders that have no projected children.
    for (index, node) in projected.nodes.iter().enumerate() {
        let has_projected_child = projected
            .nodes
            .iter()
            .any(|child| child.parent == Some(NodeId(index as u32)));
        if !has_projected_child
            && (node.grid_rows.is_some() || node.grid_columns.is_some())
            && let Some(flag) = projected.container_flags.get_mut(index)
        {
            *flag = false;
        }
    }
    projected.external_edge_counts = copy_node_metadata(&source.external_edge_counts, node_ids);
    projected.node_invisible = copy_node_metadata(&source.node_invisible, node_ids);
    // ExtractSubgraph keeps each object's Language on the same object.
    // Rebind it after dense projection so Dagre's positionLabelsIcons does
    // not read a different source object's language at the projected index.
    projected.node_has_language = copy_node_metadata(&source.node_has_language, node_ids);
    projected.table_column_counts = copy_node_metadata(&source.table_column_counts, node_ids);
    projected.table_column_names = node_ids
        .iter()
        .enumerate()
        .filter_map(|(i, id)| {
            source
                .table_column_names
                .get(id)
                .cloned()
                .map(|names| (NodeId(i as u32), names))
        })
        .collect();
    projected.initial_positions = copy_node_metadata(&source.initial_positions, node_ids);
    projected.grid_spacing = copy_node_metadata(&source.grid_spacing, node_ids);
    projected.sequence_diagram_nodes = copy_node_metadata(&source.sequence_diagram_nodes, node_ids);
    projected.near_targets = node_ids
        .iter()
        .map(|id| {
            source
                .near_targets
                .get(id.0 as usize)
                .into_iter()
                .flat_map(|targets| targets.iter().filter_map(|target| node_map.get(target)))
                .copied()
                .collect()
        })
        .collect();
    projected.node_labels = node_ids
        .iter()
        .filter_map(|id| {
            source
                .node_labels
                .get(id)
                .cloned()
                .map(|label| (*node_map.get(id).expect("projected node"), label))
        })
        .collect();
    // ExtractSubgraph retains Object.Label bytes on the original object.
    // Dense Rust projection must rebind those carriers with the same node map
    // as the display text, including empty and invalid-UTF8 Go strings.
    projected.node_label_bytes = node_ids
        .iter()
        .filter_map(|id| {
            source
                .node_label_bytes
                .get(id)
                .cloned()
                .map(|label| (*node_map.get(id).expect("projected node"), label))
        })
        .collect();
    projected.hierarchy_children_order = source
        .hierarchy_children_order
        .iter()
        .filter_map(|(parent, children)| {
            let projected_parent = parent.and_then(|id| node_map.get(&id).copied());
            let projected_children = children
                .iter()
                .filter_map(|id| node_map.get(id).copied())
                .collect::<Vec<_>>();
            (!projected_children.is_empty()).then_some((projected_parent, projected_children))
        })
        .collect();
    projected.node_order_override = source
        .node_order_override
        .iter()
        .filter_map(|id| node_map.get(id).copied())
        .collect();

    let mut edge_ids = source
        .edges()
        .filter_map(|(id, edge)| {
            (node_map.contains_key(&edge.source) && node_map.contains_key(&edge.target))
                .then_some(id)
        })
        .collect::<Vec<_>>();
    if let Some(order) = order {
        let positions = order
            .iter()
            .enumerate()
            .map(|(index, id)| (*id, index))
            .collect::<BTreeMap<_, _>>();
        edge_ids.sort_by_key(|id| positions.get(id).copied().unwrap_or(usize::MAX));
    }
    let edge_map = edge_ids
        .iter()
        .enumerate()
        .map(|(index, id)| (*id, EdgeId(index as u32)))
        .collect::<BTreeMap<_, _>>();
    projected.edges = edge_ids
        .iter()
        .filter_map(|id| source.edges.get(id.0 as usize).cloned())
        .collect();
    for edge in &mut projected.edges {
        edge.source = node_map[&edge.source];
        edge.target = node_map[&edge.target];
    }
    projected.tala_edge_ids = copy_edge_metadata(&source.tala_edge_ids, &edge_ids);
    projected.tala_edge_invisible = copy_edge_metadata(&source.tala_edge_invisible, &edge_ids);
    projected.tala_edge_label_fixed = copy_edge_metadata(&source.tala_edge_label_fixed, &edge_ids);
    projected.edge_indices = copy_edge_metadata(&source.edge_indices, &edge_ids);
    projected.edge_arrows = copy_edge_metadata(&source.edge_arrows, &edge_ids);
    projected.edge_arrowheads = copy_edge_metadata(&source.edge_arrowheads, &edge_ids);
    projected.edge_arrowhead_declarations =
        copy_edge_metadata(&source.edge_arrowhead_declarations, &edge_ids);
    projected.edge_labels = copy_edge_metadata(&source.edge_labels, &edge_ids);
    projected.edge_label_percentage_presence =
        copy_edge_metadata(&source.edge_label_percentage_presence, &edge_ids);
    projected.edge_font_sizes = copy_edge_metadata(&source.edge_font_sizes, &edge_ids);
    projected.edge_label_bold = copy_edge_metadata(&source.edge_label_bold, &edge_ids);
    projected.edge_arrowhead_labels = copy_edge_metadata(&source.edge_arrowhead_labels, &edge_ids);
    projected.edge_styles = copy_edge_metadata(&source.edge_styles, &edge_ids);
    projected.edge_links = copy_edge_metadata(&source.edge_links, &edge_ids);
    projected.edge_tooltips = copy_edge_metadata(&source.edge_tooltips, &edge_ids);
    projected.edge_icons = copy_edge_metadata(&source.edge_icons, &edge_ids);
    projected.edge_icon_positions = copy_edge_metadata(&source.edge_icon_positions, &edge_ids);
    projected.edge_icon_border_radii =
        copy_edge_metadata(&source.edge_icon_border_radii, &edge_ids);
    projected.edge_is_curves = copy_edge_metadata(&source.edge_is_curves, &edge_ids);
    projected.edge_table_columns = copy_edge_metadata(&source.edge_table_columns, &edge_ids);
    projected.edge_routes = copy_edge_metadata(&source.edge_routes, &edge_ids);
    projected.d2_input_routes = copy_edge_metadata(&source.d2_input_routes, &edge_ids);
    projected.d2_edge_validation = copy_edge_metadata(&source.d2_edge_validation, &edge_ids);
    projected.edge_min_widths = copy_edge_metadata(&source.edge_min_widths, &edge_ids);
    projected.edge_min_heights = copy_edge_metadata(&source.edge_min_heights, &edge_ids);
    projected.sequence_object_scopes = source
        .sequence_object_scopes
        .iter()
        .filter_map(|(id, scopes)| {
            node_map.get(id).map(|mapped| {
                (
                    *mapped,
                    scopes
                        .iter()
                        .filter_map(|scope| node_map.get(scope).copied())
                        .collect(),
                )
            })
        })
        .collect();
    projected.node_font_size_declarations = source
        .node_font_size_declarations
        .iter()
        .filter_map(|(id, declaration)| node_map.get(id).map(|mapped| (*mapped, *declaration)))
        .collect();
    projected.node_label_levels = source
        .node_label_levels
        .iter()
        .filter_map(|(id, level)| node_map.get(id).map(|mapped| (*mapped, *level)))
        .collect();
    projected.sequence_note_scopes = source
        .sequence_note_scopes
        .iter()
        .filter_map(|(id, scopes)| {
            node_map.get(id).map(|mapped| {
                (
                    *mapped,
                    scopes
                        .iter()
                        .filter_map(|scope| node_map.get(scope).copied())
                        .collect(),
                )
            })
        })
        .collect();
    projected.sequence_node_edge_refs = source
        .sequence_node_edge_refs
        .iter()
        .filter_map(|(id, has_reference)| node_map.get(id).map(|mapped| (*mapped, *has_reference)))
        .collect();
    projected.sequence_node_lines = source
        .sequence_node_lines
        .iter()
        .filter_map(|(id, line)| node_map.get(id).map(|mapped| (*mapped, *line)))
        .collect();
    projected.sequence_edge_lines = edge_ids
        .iter()
        .enumerate()
        .filter_map(|(index, id)| {
            source
                .sequence_edge_lines
                .get(id)
                .map(|line| (EdgeId(index as u32), *line))
        })
        .collect();
    projected.sequence_group_edges = source
        .sequence_group_edges
        .iter()
        .filter_map(|(node, edges)| {
            let mapped_node = node_map.get(node).copied()?;
            let mapped_edges = edges
                .iter()
                .filter_map(|edge| edge_map.get(edge).copied())
                .collect::<Vec<_>>();
            (!mapped_edges.is_empty()).then_some((mapped_node, mapped_edges))
        })
        .collect();

    // These maps use serialized IDs from the original graph. Do not carry
    // stale dense identities into an extracted layout scope.
    projected.source_edge_endpoints.clear();
    projected.source_edge_endpoints_by_id.clear();
    projected.serialized_clusters.clear();
    projected.serialized_sequences.clear();
    projected.serialized_hubs.clear();
    projected.serialized_trees.clear();
    projected.serialized_hierarchies.clear();
    projected.serialized_directions.clear();
    projected.source_positions_by_tala.clear();
    projected.source_parent_tala_by_tala.clear();
    (projected, node_ids.to_vec(), edge_ids)
}

fn copy_node_metadata<T: Clone + Default>(source: &[T], ids: &[NodeId]) -> Vec<T> {
    ids.iter()
        .map(|id| source.get(id.0 as usize).cloned().unwrap_or_default())
        .collect()
}

fn copy_edge_metadata<T: Clone + Default>(source: &[T], ids: &[EdgeId]) -> Vec<T> {
    ids.iter()
        .map(|id| source.get(id.0 as usize).cloned().unwrap_or_default())
        .collect()
}

fn remap_result(
    mut result: LayoutResult,
    node_ids: &[NodeId],
    edge_ids: &[EdgeId],
) -> LayoutResult {
    result.sequence_lifelines = result
        .sequence_lifelines
        .into_iter()
        .filter_map(|mut line| {
            line.actor = *node_ids.get(line.actor.0 as usize)?;
            Some(line)
        })
        .collect();
    result.boxes = result
        .boxes
        .into_iter()
        .filter_map(|(id, rect)| node_ids.get(id.0 as usize).copied().map(|id| (id, rect)))
        .collect();
    result.node_labels = result
        .node_labels
        .into_iter()
        .filter_map(|(id, label)| node_ids.get(id.0 as usize).copied().map(|id| (id, label)))
        .collect();
    result.routes = result
        .routes
        .into_iter()
        .filter_map(|(id, route)| edge_ids.get(id.0 as usize).copied().map(|id| (id, route)))
        .collect();
    result.route_curves = result
        .route_curves
        .into_iter()
        .filter_map(|(id, kind)| edge_ids.get(id.0 as usize).copied().map(|id| (id, kind)))
        .collect();
    result.removed_edges = result
        .removed_edges
        .into_iter()
        .filter_map(|id| edge_ids.get(id.0 as usize).copied())
        .collect();
    result.edge_label_percentage_commits = result
        .edge_label_percentage_commits
        .into_iter()
        .filter_map(|id| edge_ids.get(id.0 as usize).copied())
        .collect();
    result.edge_labels = result
        .edge_labels
        .into_iter()
        .filter_map(|(id, label)| edge_ids.get(id.0 as usize).copied().map(|id| (id, label)))
        .collect();
    result
}

fn copy_route_kind(
    result: &mut LayoutResult,
    id: EdgeId,
    source: &LayoutResult,
    source_id: EdgeId,
) {
    if let Some(&kind) = source.route_curves.get(&source_id) {
        result.route_curves.insert(id, kind);
    } else {
        result.route_curves.remove(&id);
    }
}

fn merge_nested_result(
    result: &mut LayoutResult,
    nested: LayoutResult,
    node_ids: &[NodeId],
    edge_ids: &[EdgeId],
    delta: Point,
) {
    result.removed_edges.extend(
        nested
            .removed_edges
            .into_iter()
            .filter_map(|id| edge_ids.get(id.0 as usize).copied()),
    );
    for mut line in nested.sequence_lifelines {
        let Some(actor) = node_ids.get(line.actor.0 as usize).copied() else {
            continue;
        };
        line.actor = actor;
        for point in &mut line.route {
            point.x += delta.x;
            point.y += delta.y;
        }
        result.sequence_lifelines.push(line);
    }
    for (id, mut rect) in nested.boxes {
        let Some(original_id) = node_ids.get(id.0 as usize).copied() else {
            continue;
        };
        rect.origin.x += delta.x;
        rect.origin.y += delta.y;
        result.boxes.insert(original_id, rect);
    }
    for (id, mut route) in nested.routes {
        let Some(original_id) = edge_ids.get(id.0 as usize).copied() else {
            continue;
        };
        for point in &mut route {
            point.x += delta.x;
            point.y += delta.y;
        }
        if let Some(&kind) = nested.route_curves.get(&id) {
            result.route_curves.insert(original_id, kind);
        } else {
            result.route_curves.remove(&original_id);
        }
        result.routes.insert(original_id, route);
    }
    for (id, label) in nested.node_labels {
        if let Some(original_id) = node_ids.get(id.0 as usize).copied() {
            result.node_labels.insert(original_id, label);
        }
    }
    for id in nested.edge_label_percentage_commits {
        if let Some(original_id) = edge_ids.get(id.0 as usize).copied() {
            result.edge_label_percentage_commits.insert(original_id);
        }
    }
    for (id, label) in nested.edge_labels {
        if let Some(original_id) = edge_ids.get(id.0 as usize).copied() {
            result.edge_labels.insert(original_id, label);
        }
    }
}

// d2near.boundingBox excludes an object or edge endpoint under any NearKey,
// including a relative near, through Object.OuterNearContainer.
fn is_under_near(graph: &Graph, id: NodeId) -> bool {
    let mut current = Some(id);
    while let Some(id) = current {
        let Some(node) = graph.node(id) else { break };
        if node.canvas_position.is_some() || node.near.is_some() || node.d2_near_key.is_some() {
            return true;
        }
        current = node.parent;
    }
    false
}

#[derive(Clone, Copy, Debug, Default)]
struct NearBounds {
    min_x: f64,
    min_y: f64,
    max_x: f64,
    max_y: f64,
}

fn near_bounds(graph: &Graph, result: &LayoutResult) -> NearBounds {
    let mut bounds = NearBounds {
        min_x: f64::INFINITY,
        min_y: f64::INFINITY,
        max_x: f64::NEG_INFINITY,
        max_y: f64::NEG_INFINITY,
    };
    for (id, node) in graph.nodes() {
        let Some(rect) = result.boxes.get(&id).copied() else {
            continue;
        };
        if let Some(position) = node.canvas_position {
            match position {
                CanvasPosition::TopCenter | CanvasPosition::BottomCenter => {
                    bounds.min_x = bounds.min_x.min(rect.origin.x);
                    bounds.max_x = bounds.max_x.max(rect.right());
                }
                CanvasPosition::CenterLeft | CanvasPosition::CenterRight => {
                    bounds.min_y = bounds.min_y.min(rect.origin.y);
                    bounds.max_y = bounds.max_y.max(rect.bottom());
                }
                _ => {}
            }
            continue;
        }
        if is_under_near(graph, id) {
            continue;
        }
        bounds.min_x = bounds.min_x.min(rect.origin.x);
        bounds.min_y = bounds.min_y.min(rect.origin.y);
        bounds.max_x = bounds.max_x.max(rect.right());
        bounds.max_y = bounds.max_y.max(rect.bottom());
        // d2near.boundingBox includes outside label rectangles when placing
        // a constant-near graph. The label is part of the visible shape
        // bounds even though it does not change the node rectangle itself;
        // omitting it shifts center-near anchors when a container title sits
        // above the base graph.
        let label_position = result
            .node_labels
            .get(&id)
            .map(|state| state.position)
            .unwrap_or(node.label_position);
        let label_rect = node
            .external_label
            .filter(|_| label_position.is_outside())
            .map(|label| {
                (
                    label.at_position(label_position).top_left(rect, 5.0),
                    label.size,
                )
            })
            .or_else(|| {
                let state = result.node_labels.get(&id)?;
                let size = state.size?;
                let origin = outside_label_top_left(state.position, rect, size, 5.0)?;
                Some((origin, size))
            });
        if let Some((label_origin, label_size)) = label_rect {
            bounds.min_x = bounds.min_x.min(label_origin.x);
            bounds.min_y = bounds.min_y.min(label_origin.y);
            bounds.max_x = bounds.max_x.max(label_origin.x + label_size.width);
            bounds.max_y = bounds.max_y.max(label_origin.y + label_size.height);
        }
    }
    for (edge_id, route) in &result.routes {
        let Some(edge) = graph
            .edges()
            .find(|(id, _)| id == edge_id)
            .map(|(_, edge)| edge)
        else {
            continue;
        };
        if is_under_near(graph, edge.source) || is_under_near(graph, edge.target) {
            continue;
        }
        for point in route {
            bounds.min_x = bounds.min_x.min(point.x);
            bounds.min_y = bounds.min_y.min(point.y);
            bounds.max_x = bounds.max_x.max(point.x);
            bounds.max_y = bounds.max_y.max(point.y);
        }
    }
    // d2near.boundingBox includes synthetic lifelines as graph edges.
    for line in &result.sequence_lifelines {
        if is_under_near(graph, line.actor) {
            continue;
        }
        for point in &line.route {
            bounds.min_x = bounds.min_x.min(point.x);
            bounds.min_y = bounds.min_y.min(point.y);
            bounds.max_x = bounds.max_x.max(point.x);
            bounds.max_y = bounds.max_y.max(point.y);
        }
    }
    if !bounds.min_x.is_finite() {
        bounds.min_x = 0.0;
        bounds.max_x = 0.0;
    }
    if !bounds.min_y.is_finite() {
        bounds.min_y = 0.0;
        bounds.max_y = 0.0;
    }
    #[cfg(feature = "diagnostic-traces")]
    if diagnostic_env_enabled("D2_TRACE_NEAR_BOUNDS") {
        eprintln!(
            "NEAR_BOUNDS bounds={:016x},{:016x},{:016x},{:016x}",
            bounds.min_x.to_bits(),
            bounds.min_y.to_bits(),
            bounds.max_x.to_bits(),
            bounds.max_y.to_bits()
        );
        for (id, edge) in graph.edges() {
            if is_under_near(graph, edge.source) || is_under_near(graph, edge.target) {
                continue;
            }
            if let Some(route) = result.routes.get(&id) {
                eprintln!(
                    "NEAR_BOUND_ROUTE source={:?} target={:?} points={}",
                    graph.node(edge.source).map(|n| &n.external_id),
                    graph.node(edge.target).map(|n| &n.external_id),
                    route
                        .iter()
                        .map(|p| format!("{:016x},{:016x}", p.x.to_bits(), p.y.to_bits()))
                        .collect::<Vec<_>>()
                        .join(";")
                );
            }
        }
    }
    bounds
}

fn outside_label_top_left(
    position: LabelPosition,
    rect: Rect,
    size: d2_rs_core::Size,
    padding: f64,
) -> Option<Point> {
    let center = rect.center();
    Some(match position {
        LabelPosition::OutsideTopLeft => Point {
            x: rect.origin.x - padding,
            y: rect.origin.y - padding - size.height,
        },
        LabelPosition::OutsideTopCenter => Point {
            x: center.x - size.width / 2.0,
            y: rect.origin.y - padding - size.height,
        },
        LabelPosition::OutsideTopRight => Point {
            x: rect.right() - size.width - padding,
            y: rect.origin.y - padding - size.height,
        },
        LabelPosition::OutsideLeftTop => Point {
            x: rect.origin.x - padding - size.width,
            y: rect.origin.y + padding,
        },
        LabelPosition::OutsideLeftMiddle => Point {
            x: rect.origin.x - padding - size.width,
            y: center.y - size.height / 2.0,
        },
        LabelPosition::OutsideLeftBottom => Point {
            x: rect.origin.x - padding - size.width,
            y: rect.bottom() - size.height - padding,
        },
        LabelPosition::OutsideRightTop => Point {
            x: rect.right() + padding,
            y: rect.origin.y + padding,
        },
        LabelPosition::OutsideRightMiddle => Point {
            x: rect.right() + padding,
            y: center.y - size.height / 2.0,
        },
        LabelPosition::OutsideRightBottom => Point {
            x: rect.right() + padding,
            y: rect.bottom() - size.height - padding,
        },
        LabelPosition::OutsideBottomLeft => Point {
            x: rect.origin.x + padding,
            y: rect.bottom() + padding,
        },
        LabelPosition::OutsideBottomCenter => Point {
            x: center.x - size.width / 2.0,
            y: rect.bottom() + padding,
        },
        LabelPosition::OutsideBottomRight => Point {
            x: rect.right() - size.width - padding,
            y: rect.bottom() + padding,
        },
        _ => return None,
    })
}

fn constant_near_origin(
    position: CanvasPosition,
    bounds: NearBounds,
    size: d2_rs_core::Size,
) -> Point {
    let width = bounds.max_x - bounds.min_x;
    let height = bounds.max_y - bounds.min_y;
    match position {
        CanvasPosition::TopLeft => Point {
            x: bounds.min_x - size.width - near::PAD,
            y: bounds.min_y - size.height - near::PAD,
        },
        CanvasPosition::TopCenter => Point {
            x: bounds.min_x + width / 2.0 - size.width / 2.0,
            y: bounds.min_y - size.height - near::PAD,
        },
        CanvasPosition::TopRight => Point {
            x: bounds.max_x + near::PAD,
            y: bounds.min_y - size.height - near::PAD,
        },
        CanvasPosition::CenterLeft => Point {
            x: bounds.min_x - size.width - near::PAD,
            y: bounds.min_y + height / 2.0 - size.height / 2.0,
        },
        CanvasPosition::CenterRight => Point {
            x: bounds.max_x + near::PAD,
            y: bounds.min_y + height / 2.0 - size.height / 2.0,
        },
        CanvasPosition::BottomLeft => Point {
            x: bounds.min_x - size.width - near::PAD,
            y: bounds.max_y + near::PAD,
        },
        CanvasPosition::BottomCenter => Point {
            // d2near.place subtracts from the right bound for bottom-center.
            x: bounds.max_x - width / 2.0 - size.width / 2.0,
            y: bounds.max_y + near::PAD,
        },
        CanvasPosition::BottomRight => Point {
            x: bounds.max_x + near::PAD,
            y: bounds.max_y + near::PAD,
        },
    }
}

fn adjust_constant_near_for_label(
    origin: &mut Point,
    position: CanvasPosition,
    label_position: LabelPosition,
    label_size: Option<d2_rs_core::Size>,
) {
    let Some(label_size) = label_size else {
        return;
    };
    match label_position {
        LabelPosition::OutsideTopLeft
        | LabelPosition::OutsideTopCenter
        | LabelPosition::OutsideTopRight
            if matches!(
                position,
                CanvasPosition::BottomLeft
                    | CanvasPosition::BottomCenter
                    | CanvasPosition::BottomRight
            ) =>
        {
            origin.y += label_size.height;
        }
        LabelPosition::OutsideLeftTop
        | LabelPosition::OutsideLeftMiddle
        | LabelPosition::OutsideLeftBottom
            if matches!(
                position,
                CanvasPosition::TopRight
                    | CanvasPosition::CenterRight
                    | CanvasPosition::BottomRight
            ) =>
        {
            origin.x += label_size.width;
        }
        LabelPosition::OutsideRightTop
        | LabelPosition::OutsideRightMiddle
        | LabelPosition::OutsideRightBottom
            if matches!(
                position,
                CanvasPosition::TopLeft | CanvasPosition::CenterLeft | CanvasPosition::BottomLeft
            ) =>
        {
            origin.x -= label_size.width;
        }
        LabelPosition::OutsideBottomLeft
        | LabelPosition::OutsideBottomCenter
        | LabelPosition::OutsideBottomRight
            if matches!(
                position,
                CanvasPosition::TopLeft | CanvasPosition::TopCenter | CanvasPosition::TopRight
            ) =>
        {
            origin.y -= label_size.height;
        }
        _ => {}
    }
}

/// Lays out a graph with the default in-process TALA engine.
pub fn layout_tala(
    graph: &Graph,
    options: &LayoutOptions,
) -> Result<LayoutResult, LayoutDispatchError> {
    LayoutDispatcher.layout(graph, EngineKind::Tala, options)
}

/// Routes every edge with the default in-process TALA engine.
pub fn route_edges_tala(
    graph: &Graph,
    options: &LayoutOptions,
) -> Result<LayoutResult, LayoutDispatchError> {
    LayoutDispatcher.route_edges(graph, EngineKind::Tala, options)
}

#[cfg(test)]
mod tests {
    use super::{
        DiagramType, EngineKind, GraphInfo, LayoutDispatchError, LayoutDispatcher,
        NestedDagreScope, cache_layout_result, closest_grid_scope, collapse_dagre_nested_grids,
        empty_layout_result, has_grid_scope, has_nested_grid_scope, inside_sequence_scope,
        nested_dagre_scope_plan, order_extracted_edges, ordered_grid_routing_scopes,
        ordered_hierarchy_breadth_first, ordered_nested_grid_cells, outermost_grid_scope,
        reroute_nested_grid_external_edges, reroute_reinserted_grid_edges, restore_layout_result,
    };
    use d2_rs_core::{
        CancellationToken, CanvasPosition, ContentAlignment, Direction, Edge, EdgeLabel, Graph,
        Insets, LabelPosition, LayoutOptions, Node, NodeId, Point, Rect, ShapeKind, Size,
    };
    use std::{collections::BTreeMap, str::FromStr};

    #[test]
    fn projected_edges_preserve_original_arrowhead_declarations_for_tala_validation() {
        let mut graph = Graph::default();
        let size = Size {
            width: 40.0,
            height: 40.0,
        };
        let excluded = graph.add_node(Node::new("excluded", size));
        let source = graph.add_node(Node::new("source", size));
        let target = graph.add_node(Node::new("target", size));
        graph.add_edge(Edge {
            source: excluded,
            target: source,
        });
        let kept = graph.add_edge(Edge { source, target });
        graph.set_edge_arrowheads(
            kept,
            d2_rs_core::EdgeArrowheads {
                source: Some("triangle".into()),
                target: Some("triangle".into()),
            },
        );
        graph.set_edge_arrowhead_declarations(
            kept,
            d2_rs_core::EdgeArrowheadDeclarations {
                source: None,
                target: Some("ſquare".into()),
            },
        );
        let (projected, _, edge_map) = super::project_graph(&graph, &[source, target], false);
        assert_eq!(edge_map, [kept]);
        let error = d2_rs_tala::Engine
            .layout(&projected, &LayoutOptions::default())
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "D2 edge 0 has unsupported destination arrowhead \"ſquare\""
        );
    }

    #[test]
    fn nested_scope_walk_uses_source_order_breadth_first() {
        let mut graph = Graph::default();
        let b = graph.add_node(Node::new("b", Size::default()));
        let b_child = graph.add_node(Node {
            parent: Some(b),
            ..Node::new("b.child", Size::default())
        });
        let a = graph.add_node(Node::new("a", Size::default()));
        let a_child = graph.add_node(Node {
            parent: Some(a),
            ..Node::new("a.child", Size::default())
        });
        graph.set_hierarchy_children_order(BTreeMap::from([
            (None, vec![a, b]),
            (Some(a), vec![a_child]),
            (Some(b), vec![b_child]),
        ]));

        assert_eq!(
            ordered_hierarchy_breadth_first(&graph),
            vec![a, b, a_child, b_child]
        );
    }

    #[test]
    fn nested_grid_cells_follow_recursive_ownership_and_source_order() {
        let mut graph = Graph::default();
        let mut root = Node::new("root", Size::default());
        root.grid_columns = Some(1);
        let root = graph.add_node(root);
        let region = graph.add_node(Node {
            parent: Some(root),
            ..Node::new("region", Size::default())
        });
        let mut global = Node::new("global", Size::default());
        global.parent = Some(root);
        global.grid_columns = Some(2);
        let global = graph.add_node(global);
        let first = graph.add_node(Node {
            parent: Some(global),
            ..Node::new("first", Size::default())
        });
        graph.add_node(Node {
            parent: Some(first),
            ..Node::new("first.leaf", Size::default())
        });
        let second = graph.add_node(Node {
            parent: Some(global),
            ..Node::new("second", Size::default())
        });
        graph.add_node(Node {
            parent: Some(second),
            ..Node::new("second.leaf", Size::default())
        });
        let mut nested = Node::new("nested", Size::default());
        nested.parent = Some(region);
        nested.grid_columns = Some(1);
        let nested = graph.add_node(nested);
        let nested_cell = graph.add_node(Node {
            parent: Some(nested),
            ..Node::new("nested.cell", Size::default())
        });
        graph.add_node(Node {
            parent: Some(nested_cell),
            ..Node::new("nested.cell.leaf", Size::default())
        });
        graph.set_hierarchy_children_order(BTreeMap::from([
            (None, vec![root]),
            (Some(root), vec![global, region]),
            (Some(global), vec![second, first]),
        ]));
        assert_eq!(
            ordered_nested_grid_cells(&graph),
            vec![second, first, region]
        );
    }

    #[test]
    fn tala_package_label_padding_is_derived_once_by_the_engine() {
        let mut graph = Graph::default();
        let mut package = Node::new(
            "aa",
            Size {
                width: 72.0,
                height: 92.0,
            },
        );
        package.shape = ShapeKind::Package;
        package.label_size = Some(Size {
            width: 27.0,
            height: 36.0,
        });
        package.font_size = Some(28);
        package.label_position = LabelPosition::Unset;
        package.content_insets = Insets {
            top: 77.0,
            right: 60.0,
            bottom: 60.0,
            left: 60.0,
        };
        let package_id = graph.add_node(package);
        let mut child = Node::new(
            "aa.bb",
            Size {
                width: 66.0,
                height: 92.0,
            },
        );
        child.parent = Some(package_id);
        child.shape = ShapeKind::Diamond;
        child.content_alignment = ContentAlignment::Diamond;
        child.label_size = Some(Size {
            width: 18.0,
            height: 21.0,
        });
        child.font_size = Some(16);
        child.port_spread = 12.0;
        let child_id = graph.add_node(child);

        let result = LayoutDispatcher
            .layout(
                &graph,
                EngineKind::Tala,
                &LayoutOptions {
                    seeds: vec![1],
                    max_concurrency: 1,
                },
            )
            .unwrap();

        assert_eq!(result.boxes[&package_id].size.height, 265.0);
        assert_eq!(result.boxes[&child_id].origin.y, 113.0);
    }

    #[test]
    fn root_grid_routing_keeps_parent_edges_before_child_reinsertion() {
        let mut graph = Graph::default();
        graph.root_grid_rows = Some(1);
        let a = graph.add_node(Node::new("a", Size::default()));
        let b = graph.add_node(Node::new("b", Size::default()));
        let mut child = Node::new("a.child", Size::default());
        child.parent = Some(a);
        let ac = graph.add_node(child);
        let mut child = Node::new("b.child", Size::default());
        child.parent = Some(b);
        let bc = graph.add_node(child);
        let b_fixed = graph.add_edge(d2_rs_core::Edge {
            source: b,
            target: bc,
        });
        let crossing = graph.add_edge(d2_rs_core::Edge {
            source: ac,
            target: bc,
        });
        let a_fixed = graph.add_edge(d2_rs_core::Edge {
            source: a,
            target: ac,
        });
        let parent_fixed = graph.add_edge(d2_rs_core::Edge {
            source: a,
            target: b,
        });
        assert_eq!(
            super::root_grid_routing_edge_order(&graph, &[crossing]),
            vec![parent_fixed, a_fixed, b_fixed, crossing]
        );
        // Object-level grid routing projects away the root's grid metadata.
        // Its callback must still receive the grid's reinsertion order.
        graph.root_grid_rows = None;
        assert_eq!(
            super::grid_routing_edge_order(&graph, &[crossing]),
            vec![parent_fixed, a_fixed, b_fixed, crossing]
        );
    }

    #[test]
    fn lifeline_injection_uses_scope_queue_order_and_preserves_actor_order() {
        let mut graph = Graph::default();
        let container = graph.add_node(Node::new("container", Size::default()));
        let branch = graph.add_node(Node {
            parent: Some(container),
            ..Node::new("container.branch", Size::default())
        });
        let later = graph.add_node(Node {
            parent: Some(branch),
            ..Node::new("container.branch.sequence", Size::default())
        });
        graph.set_sequence_diagram(later, true);
        let earlier = graph.add_node(Node {
            parent: Some(container),
            ..Node::new("container.sequence", Size::default())
        });
        graph.set_sequence_diagram(earlier, true);
        let actors = [
            ("container.sequence.a", earlier),
            ("container.sequence.b", earlier),
            ("container.branch.sequence.a", later),
        ]
        .map(|(name, parent)| {
            graph.add_node(Node {
                parent: Some(parent),
                ..Node::new(name, Size::default())
            })
        });
        graph.set_hierarchy_children_order(BTreeMap::from([
            (None, vec![container]),
            (Some(container), vec![branch, earlier]),
            (Some(branch), vec![later]),
            (Some(earlier), vec![actors[0], actors[1]]),
            (Some(later), vec![actors[2]]),
        ]));
        assert_eq!(
            super::sequence_injection_order(&graph),
            vec![Some(earlier), Some(later)]
        );
        // Native sizing completed the deeper sequence first. Go's breadth-first
        // queue injects the shallower sequence first; its actors keep their
        // existing order rather than being sorted by arena ID or name.
        let lines = [actors[2], actors[1], actors[0]].map(|actor| d2_rs_core::SequenceLifeline {
            actor,
            target_id: format!("lifeline-{}", actor.0),
            route: vec![
                Point {
                    x: actor.0 as f64,
                    y: 1.0,
                },
                Point {
                    x: actor.0 as f64,
                    y: 9.0,
                },
            ],
        });
        super::append_fixed_sequence_lifelines(&mut graph, &lines);
        assert_eq!(
            graph
                .edges()
                .map(|(_, edge)| edge.source)
                .collect::<Vec<_>>(),
            vec![actors[1], actors[0], actors[2]]
        );
        for (id, edge) in graph.edges() {
            assert_eq!(edge.source, edge.target);
            let expected = lines.iter().find(|line| line.actor == edge.source).unwrap();
            assert_eq!(graph.edge_route(id), expected.route.as_slice());
            assert_eq!(graph.edge_min_widths[id.0 as usize], Some(10.0));
            assert_eq!(graph.edge_min_heights[id.0 as usize], Some(10.0));
        }
    }

    #[test]
    fn mixed_special_scopes_keep_the_parent_queue_order() {
        let mut graph = Graph::default();
        let mut grid = Node::new("grid", Size::default());
        grid.grid_rows = Some(1);
        let grid = graph.add_node(grid);
        let sequence = graph.add_node(Node::new("sequence", Size::default()));
        graph.set_sequence_diagram(sequence, true);
        let grid_child = graph.add_node(Node {
            parent: Some(grid),
            ..Node::new("grid.child", Size::default())
        });
        let actor = graph.add_node(Node {
            parent: Some(sequence),
            ..Node::new("sequence.actor", Size::default())
        });
        let mut empty_grid = Node::new("empty_grid", Size::default());
        empty_grid.grid_rows = Some(1);
        let empty_grid = graph.add_node(empty_grid);
        let empty_sequence = graph.add_node(Node::new("empty_sequence", Size::default()));
        graph.set_sequence_diagram(empty_sequence, true);
        let mut near = Node::new("near", Size::default());
        near.canvas_position = Some(CanvasPosition::TopLeft);
        let near = graph.add_node(near);
        let ordinary = graph.add_node(Node::new("ordinary", Size::default()));
        let mut nested_near = Node::new("ordinary.near", Size::default());
        nested_near.parent = Some(ordinary);
        nested_near.canvas_position = Some(CanvasPosition::BottomRight);
        let nested_near = graph.add_node(nested_near);
        graph.set_hierarchy_children_order(BTreeMap::from([
            (
                None,
                vec![near, ordinary, empty_grid, grid, empty_sequence, sequence],
            ),
            (Some(grid), vec![grid_child]),
            (Some(sequence), vec![actor]),
            (Some(ordinary), vec![nested_near]),
        ]));

        assert_eq!(
            nested_dagre_scope_plan(&graph),
            vec![
                NestedDagreScope::ConstantNear(near),
                NestedDagreScope::Grid(grid),
                NestedDagreScope::Sequence(sequence),
                NestedDagreScope::ConstantNear(nested_near),
            ]
        );
    }

    #[test]
    fn routing_edges_follow_near_and_sequence_reinsertion_with_lifelines() {
        let mut graph = Graph::default();
        let mut near = Node::new("near", Size::default());
        near.canvas_position = Some(CanvasPosition::CenterLeft);
        let near = graph.add_node(near);
        let mut near_child = Node::new("near.child", Size::default());
        near_child.parent = Some(near);
        let near_child = graph.add_node(near_child);
        let sequence = graph.add_node(Node::new("sequence", Size::default()));
        graph.set_sequence_diagram(sequence, true);
        let mut actor = Node::new("sequence.actor", Size::default());
        actor.parent = Some(sequence);
        let actor = graph.add_node(actor);
        let outside = graph.add_node(Node::new("outside", Size::default()));
        let near_edge = graph.add_edge(Edge {
            source: near,
            target: near_child,
        });
        let message = graph.add_edge(Edge {
            source: actor,
            target: actor,
        });
        let retained = graph.add_edge(Edge {
            source: sequence,
            target: outside,
        });
        let crossing = graph.add_edge(Edge {
            source: actor,
            target: outside,
        });
        let lifeline = graph.add_edge(Edge {
            source: actor,
            target: actor,
        });
        assert_eq!(
            super::root_grid_routing_edge_order(&graph, &[crossing]),
            vec![retained, near_edge, message, lifeline, crossing]
        );
    }

    #[test]
    fn routing_walks_hierarchy_after_near_root_reinsertion() {
        let mut graph = Graph::default();
        let mut near = Node::new("near", Size::default());
        near.canvas_position = Some(CanvasPosition::CenterLeft);
        let near = graph.add_node(near);
        let ordinary = graph.add_node(Node::new("ordinary", Size::default()));
        let mut near_child = Node::new("near.child", Size::default());
        near_child.parent = Some(near);
        let near_child = graph.add_node(near_child);
        let mut ordinary_child = Node::new("ordinary.child", Size::default());
        ordinary_child.parent = Some(ordinary);
        let ordinary_child = graph.add_node(ordinary_child);
        assert_eq!(
            super::routing_node_order(&graph),
            vec![ordinary, ordinary_child, near, near_child]
        );
    }

    #[test]
    fn routing_projection_reorders_ids_routes_labels_and_ports_together() {
        let mut graph = Graph::default();
        let a = graph.add_node(Node::new("a", Size::default()));
        let b = graph.add_node(Node::new("b", Size::default()));
        let first = graph.add_edge(Edge {
            source: a,
            target: b,
        });
        let second = graph.add_edge(Edge {
            source: b,
            target: a,
        });
        graph.tala_edge_ids = vec![Some(11), Some(22)];
        graph
            .table_column_names
            .insert(b, vec!["id".into(), "with space".into()]);
        graph.set_edge_route(
            second,
            vec![Point { x: 3.0, y: 4.0 }, Point { x: 5.0, y: 6.0 }],
        );
        graph.set_d2_input_route(
            second,
            vec![Point { x: 5.0, y: 6.0 }, Point { x: 3.0, y: 4.0 }],
        );
        graph.set_edge_table_columns(
            second,
            d2_rs_core::EdgeTableColumns {
                source: Some(2),
                target: Some(3),
            },
        );
        graph.set_edge_label(
            second,
            Some(EdgeLabel {
                text: "second".into(),
                size: Size::default(),
                position: LabelPosition::InsideMiddleCenter,
                percentage: 0.25,
            }),
        );
        graph.set_edge_label_percentage_present(second, false);
        let validation = d2_rs_core::D2EdgeValidation {
            nil_route_points: [1].into_iter().collect(),
            external_id: Some("original edge".into()),
            source_table_column: Some(2),
            target_table_column: Some(-1),
            endpoints_reversed: true,
            label_position: Some(b"INSIDE_MIDDLE_CENTER".to_vec()),
            attributes: d2_rs_core::D2ValidationAttributes {
                label_dimensions: [500, 700],
                ..Default::default()
            },
            ..Default::default()
        };
        graph.set_d2_edge_validation(second, validation.clone());
        graph.set_edge_index(first, 2);
        graph.set_edge_index(second, 7);
        let (mut projected, _, old_ids) =
            super::project_graph_in_edge_order(&graph, &[a, b], false, Some(&[second, first]));
        assert_eq!(old_ids, vec![second, first]);
        assert_eq!(projected.edge_index(d2_rs_core::EdgeId(0)), Some(7));
        assert_eq!(projected.edge_index(d2_rs_core::EdgeId(1)), Some(2));
        assert_eq!(projected.tala_edge_ids, vec![Some(22), Some(11)]);
        assert_eq!(projected.table_column_names[&b], vec!["id", "with space"]);
        let id = d2_rs_core::EdgeId(0);
        assert_eq!(projected.edges[0].source, b);
        assert_eq!(projected.edge_route(id)[0], Point { x: 3.0, y: 4.0 });
        assert_eq!(projected.edge_label(id).unwrap().text, "second");
        assert!(!projected.edge_label_percentage_present(id));
        assert_eq!(projected.edge_table_columns(id).source, Some(2));
        assert_eq!(projected.d2_input_route(id)[0], Point { x: 5.0, y: 6.0 });
        assert_eq!(projected.d2_edge_validation(id), Some(&validation));
        assert!(
            projected
                .d2_edge_validation(d2_rs_core::EdgeId(1))
                .is_none()
        );
        projected.set_edge_route(id, vec![Point { x: 7.0, y: 8.0 }]);
        assert_eq!(projected.d2_input_route(id), projected.edge_route(id));
        assert!(
            projected
                .d2_edge_validation(id)
                .unwrap()
                .nil_route_points
                .is_empty()
        );
    }

    #[test]
    fn extracted_route_ids_use_local_hierarchy_without_changing_public_names() {
        let mut graph = Graph::default();
        let a = graph.add_node(Node::new("scope.\"a.b\"", Size::default()));
        let b = graph.add_node(Node::new("scope.target", Size::default()));
        let edge = graph.add_edge(Edge {
            source: a,
            target: b,
        });
        super::rebase_routing_identities(&mut graph, "scope").unwrap();
        let expected = d2_rs_core::identity::allocate(&[b"(\"a.b\" -- target)[0]"]).unwrap();
        assert_eq!(graph.tala_edge_ids[edge.0 as usize], Some(expected[0]));
        assert_eq!(graph.node(a).unwrap().external_id, "scope.\"a.b\"");
        assert_eq!(
            graph.tala_id_overrides[a.0 as usize],
            Some(d2_rs_core::identity::allocate(&[b"\"a.b\""]).unwrap()[0])
        );
    }

    #[test]
    fn grid_route_callbacks_finish_children_before_their_parent() {
        let mut graph = Graph::default();
        let mut outer = Node::new("outer", Size::default());
        outer.grid_rows = Some(1);
        let outer = graph.add_node(outer);
        let mut inner = Node::new("outer.inner", Size::default());
        inner.parent = Some(outer);
        inner.grid_columns = Some(1);
        let inner = graph.add_node(inner);
        let mut sibling = Node::new("sibling", Size::default());
        sibling.grid_rows = Some(1);
        let sibling = graph.add_node(sibling);
        assert_eq!(
            ordered_grid_routing_scopes(&graph),
            vec![inner, outer, sibling]
        );
    }

    #[test]
    fn restored_crossings_follow_first_extraction_and_keep_edge_order() {
        let mut graph = Graph::default();
        let mut grid = Node::new("grid", Size::default());
        grid.grid_rows = Some(1);
        let grid = graph.add_node(grid);
        let mut child = Node::new("grid.child", Size::default());
        child.parent = Some(grid);
        let child = graph.add_node(child);
        let mut near = Node::new("near", Size::default());
        near.canvas_position = Some(CanvasPosition::TopLeft);
        let near = graph.add_node(near);
        let outside = graph.add_node(Node::new("outside", Size::default()));
        let near_edge = graph.add_edge(Edge {
            source: near,
            target: outside,
        });
        let grid_edge = graph.add_edge(Edge {
            source: child,
            target: outside,
        });
        let crossing = graph.add_edge(Edge {
            source: near,
            target: child,
        });
        let placeholder = graph.add_edge(Edge {
            source: grid,
            target: outside,
        });
        let mut edges = vec![near_edge, grid_edge, crossing, placeholder];
        order_extracted_edges(&graph, &mut edges);
        assert_eq!(edges, vec![grid_edge, crossing, near_edge, placeholder]);
    }

    #[test]
    fn near_bounds_ignore_stale_outside_bands_after_label_moves_inside() {
        let mut graph = Graph::default();
        let mut node = Node::new(
            "container",
            Size {
                width: 100.0,
                height: 100.0,
            },
        );
        node.external_label = Some(d2_rs_core::ExternalLabel {
            size: Size {
                width: 80.0,
                height: 55.0,
            },
            side: d2_rs_core::ExternalSide::Top,
            alignment: Default::default(),
            automatic: true,
            reserve_space: true,
        });
        let id = graph.add_node(node);
        let mut result = super::empty_layout_result(&LayoutOptions::default());
        result.boxes.insert(
            id,
            Rect {
                origin: Point::default(),
                size: Size {
                    width: 100.0,
                    height: 100.0,
                },
            },
        );
        result.node_labels.insert(
            id,
            d2_rs_core::NodeLabelState {
                position: LabelPosition::InsideTopCenter,
                ..Default::default()
            },
        );
        assert_eq!(super::near_bounds(&graph, &result).min_y, 0.0);
        result.node_labels.get_mut(&id).unwrap().position = LabelPosition::OutsideTopCenter;
        assert_eq!(super::near_bounds(&graph, &result).min_y, -60.0);
    }

    #[test]
    fn near_bounds_exclude_relative_near_subtrees_and_edges() {
        let mut graph = Graph::default();
        let anchor = graph.add_node(Node::new("anchor", Size::default()));
        let mut near = Node::new("near", Size::default());
        near.near = Some(anchor);
        let near = graph.add_node(near);
        let mut child = Node::new("near.child", Size::default());
        child.parent = Some(near);
        let child = graph.add_node(child);
        let edge = graph.add_edge(Edge {
            source: anchor,
            target: child,
        });
        let mut result = super::empty_layout_result(&LayoutOptions::default());
        for (id, x) in [(anchor, 10.0), (near, 200.0), (child, 300.0)] {
            result.boxes.insert(
                id,
                Rect {
                    origin: Point { x, y: 10.0 },
                    size: Size {
                        width: 20.0,
                        height: 20.0,
                    },
                },
            );
        }
        result
            .routes
            .insert(edge, vec![Point { x: 500.0, y: 500.0 }]);
        let bounds = super::near_bounds(&graph, &result);
        assert_eq!(
            (bounds.min_x, bounds.max_x, bounds.max_y),
            (10.0, 30.0, 30.0)
        );
    }

    #[test]
    fn near_bounds_include_base_lifelines_and_exclude_constant_near_lifelines() {
        let mut graph = Graph::default();
        let base = graph.add_node(Node::new(
            "base",
            Size {
                width: 20.0,
                height: 20.0,
            },
        ));
        let mut near = Node::new("near", Size::default());
        near.canvas_position = Some(d2_rs_core::CanvasPosition::CenterLeft);
        let near = graph.add_node(near);
        let mut child = Node::new("near.actor", Size::default());
        child.parent = Some(near);
        let child = graph.add_node(child);
        let mut result = super::empty_layout_result(&LayoutOptions::default());
        result.boxes.insert(
            base,
            Rect {
                origin: Point { x: 10.0, y: 10.0 },
                size: Size {
                    width: 20.0,
                    height: 20.0,
                },
            },
        );
        result.sequence_lifelines = vec![
            d2_rs_core::SequenceLifeline {
                actor: base,
                target_id: "base-end".into(),
                route: vec![Point { x: 20.0, y: 30.0 }, Point { x: 20.0, y: 70.0 }],
            },
            d2_rs_core::SequenceLifeline {
                actor: child,
                target_id: "child-end".into(),
                route: vec![
                    Point {
                        x: -1000.0,
                        y: 30.0,
                    },
                    Point {
                        x: -1000.0,
                        y: 10000.0,
                    },
                ],
            },
        ];
        let bounds = super::near_bounds(&graph, &result);
        assert_eq!(bounds.min_x, 10.0);
        assert_eq!(bounds.max_x, 30.0);
        assert_eq!(bounds.max_y, 70.0);
    }

    #[test]
    fn label_commit_provenance_remaps_through_nested_reinsertion() {
        let mut local = empty_layout_result(&LayoutOptions::default());
        let committed = d2_rs_core::EdgeId(0);
        let uncommitted = d2_rs_core::EdgeId(1);
        for id in [committed, uncommitted] {
            local.edge_labels.insert(
                id,
                EdgeLabel {
                    text: "label".into(),
                    size: Size::default(),
                    position: LabelPosition::InsideMiddleCenter,
                    percentage: 0.0,
                },
            );
        }
        local.edge_label_percentage_commits.insert(committed);
        let ids = [d2_rs_core::EdgeId(8), d2_rs_core::EdgeId(3)];
        let mapped = super::remap_result(local.clone(), &[], &ids);
        assert_eq!(
            mapped.edge_label_percentage_commits,
            [ids[0]].into_iter().collect()
        );
        let mut parent = empty_layout_result(&LayoutOptions::default());
        super::merge_nested_result(&mut parent, local, &[], &ids, Point::default());
        assert_eq!(
            parent.edge_label_percentage_commits,
            mapped.edge_label_percentage_commits
        );
        assert_eq!(parent.edge_labels[&ids[1]].percentage, 0.0);
        assert!(!parent.edge_label_percentage_commits.contains(&ids[1]));
    }

    #[test]
    fn removed_edges_remap_through_reinsertion_and_cache() {
        let mut graph = Graph::default();
        let a = graph.add_node(Node::new("a", Size::default()));
        let b = graph.add_node(Node::new("b", Size::default()));
        let removed = graph.add_edge(Edge {
            source: a,
            target: b,
        });
        let mut local = empty_layout_result(&LayoutOptions::default());
        local.removed_edges.insert(removed);
        let ids = [d2_rs_core::EdgeId(7)];
        let mapped = super::remap_result(local.clone(), &[], &ids);
        assert_eq!(mapped.removed_edges, [ids[0]].into());
        let mut parent = empty_layout_result(&LayoutOptions::default());
        super::merge_nested_result(&mut parent, local.clone(), &[], &ids, Point::default());
        assert_eq!(parent.removed_edges, mapped.removed_edges);
        let cached = cache_layout_result(&graph, &local);
        graph.edges.insert(
            0,
            Edge {
                source: b,
                target: a,
            },
        );
        let restored = restore_layout_result(&graph, &cached).unwrap();
        assert_eq!(restored.removed_edges, [d2_rs_core::EdgeId(1)].into());
    }

    #[test]
    fn cached_layout_results_remap_by_stable_node_and_edge_identity() {
        let mut source = Graph::default();
        let source_a = source.add_node(Node::new("a", Size::default()));
        let source_b = source.add_node(Node::new("b", Size::default()));
        let source_edge = source.add_edge(Edge {
            source: source_a,
            target: source_b,
        });
        let mut result = empty_layout_result(&LayoutOptions::default());
        result.boxes.insert(
            source_a,
            Rect {
                origin: Point { x: 11.0, y: 13.0 },
                size: Size {
                    width: 17.0,
                    height: 19.0,
                },
            },
        );
        result.boxes.insert(
            source_b,
            Rect {
                origin: Point { x: 23.0, y: 29.0 },
                size: Size {
                    width: 31.0,
                    height: 37.0,
                },
            },
        );
        result
            .routes
            .insert(source_edge, vec![Point { x: 5.0, y: 7.0 }]);
        result.edge_label_percentage_commits.insert(source_edge);
        result.route_curves.insert(source_edge, false);
        let remapped = super::remap_result(
            result.clone(),
            &[NodeId(5), NodeId(7)],
            &[d2_rs_core::EdgeId(9)],
        );
        assert_eq!(
            remapped.route_curves,
            [(d2_rs_core::EdgeId(9), false)].into_iter().collect()
        );
        let mut merged = empty_layout_result(&LayoutOptions::default());
        super::merge_nested_result(
            &mut merged,
            result.clone(),
            &[NodeId(5), NodeId(7)],
            &[d2_rs_core::EdgeId(9)],
            Point::default(),
        );
        assert_eq!(merged.route_curves, remapped.route_curves);
        let cached = cache_layout_result(&source, &result);

        let mut target = Graph::default();
        let target_b = target.add_node(Node::new("b", Size::default()));
        let target_a = target.add_node(Node::new("a", Size::default()));
        target.add_edge(Edge {
            source: target_b,
            target: target_b,
        });
        let target_edge = target.add_edge(Edge {
            source: target_a,
            target: target_b,
        });
        let restored = restore_layout_result(&target, &cached).unwrap();

        assert_eq!(restored.boxes[&target_a], result.boxes[&source_a]);
        assert_eq!(restored.boxes[&target_b], result.boxes[&source_b]);
        assert_eq!(restored.routes[&target_edge], result.routes[&source_edge]);
        assert_eq!(
            restored.route_curves,
            [(target_edge, false)].into_iter().collect()
        );
        assert_eq!(
            restored.edge_label_percentage_commits,
            [target_edge].into_iter().collect()
        );
    }

    #[test]
    fn engine_aliases_and_canonical_names_match_d2() {
        assert_eq!(EngineKind::from_str("tala").unwrap(), EngineKind::Tala);
        assert_eq!(EngineKind::from_str("TALA-RS").unwrap(), EngineKind::Tala);
        assert_eq!(EngineKind::from_str("dagre").unwrap().as_str(), "dagre");
        assert_eq!(EngineKind::from_str("elk").unwrap().to_string(), "elk");
        assert!(EngineKind::from_str("unknown").is_err());
    }

    #[test]
    fn grid_scopes_inside_sequence_diagrams_stay_with_sequence_layout() {
        let mut graph = Graph::default();
        let size = Size {
            width: 40.0,
            height: 30.0,
        };
        let sequence = graph.add_node(Node::new("sequence", size));
        graph.set_sequence_diagram(sequence, true);
        let mut actor = Node::new("sequence.actor", size);
        actor.parent = Some(sequence);
        actor.grid_columns = Some(3);
        let actor = graph.add_node(actor);
        let mut child = Node::new("sequence.actor.child", size);
        child.parent = Some(actor);
        let child = graph.add_node(child);

        assert!(inside_sequence_scope(&graph, actor));
        assert!(inside_sequence_scope(&graph, child));
        assert_eq!(closest_grid_scope(&graph, child), None);
        assert!(!has_grid_scope(&graph));
        assert!(!has_nested_grid_scope(&graph));
    }

    #[test]
    fn nested_grid_descendants_restore_with_outer_placeholder_delta() {
        let mut graph = Graph::default();
        let mut outer = Node::new("outer", Size::default());
        outer.grid_rows = Some(1);
        let outer = graph.add_node(outer);
        let mut inner = Node::new("outer.inner", Size::default());
        inner.parent = Some(outer);
        inner.grid_columns = Some(2);
        let inner = graph.add_node(inner);
        let mut descendant = Node::new("outer.inner.leaf", Size::default());
        descendant.parent = Some(inner);
        let descendant = graph.add_node(descendant);

        assert_eq!(closest_grid_scope(&graph, descendant), Some(inner));
        assert_eq!(outermost_grid_scope(&graph, descendant), Some(outer));
    }

    #[test]
    fn tala_object_near_uses_engine_placement_without_post_layout_repositioning() {
        let mut graph = Graph {
            direction: Direction::Right,
            ..Graph::default()
        };
        let size = Size {
            width: 53.0,
            height: 66.0,
        };
        let target = graph.add_node(Node::new("a", size));
        let mut near = Node::new("b", size);
        near.near = Some(target);
        graph.add_node(near);

        let result = LayoutDispatcher
            .layout(&graph, EngineKind::Tala, &LayoutOptions::default())
            .unwrap();

        assert_eq!(result.boxes[&target].origin, Point { x: 0.0, y: 132.0 });
        assert_eq!(result.boxes[&NodeId(1)].origin, Point::default());
    }

    #[test]
    fn reinserted_grid_external_edges_use_the_full_tala_router() {
        let mut graph = Graph::default();
        let mut grid = Node::new(
            "grid",
            Size {
                width: 300.0,
                height: 200.0,
            },
        );
        grid.grid_columns = Some(1);
        let grid = graph.add_node(grid);
        let mut cell = Node::new(
            "grid.cell",
            Size {
                width: 180.0,
                height: 120.0,
            },
        );
        cell.parent = Some(grid);
        let cell = graph.add_node(cell);
        let mut source = Node::new(
            "grid.cell.source",
            Size {
                width: 40.0,
                height: 40.0,
            },
        );
        source.parent = Some(cell);
        let source = graph.add_node(source);
        let target = graph.add_node(Node::new(
            "target",
            Size {
                width: 50.0,
                height: 40.0,
            },
        ));
        let edge = graph.add_edge(Edge { source, target });

        let mut result = empty_layout_result(&LayoutOptions::default());
        result.boxes.insert(
            grid,
            Rect {
                origin: Point::default(),
                size: Size {
                    width: 300.0,
                    height: 200.0,
                },
            },
        );
        result.boxes.insert(
            cell,
            Rect {
                origin: Point { x: 40.0, y: 40.0 },
                size: Size {
                    width: 180.0,
                    height: 120.0,
                },
            },
        );
        result.boxes.insert(
            source,
            Rect {
                origin: Point { x: 70.0, y: 120.0 },
                size: Size {
                    width: 40.0,
                    height: 40.0,
                },
            },
        );
        result.boxes.insert(
            target,
            Rect {
                origin: Point { x: 400.0, y: 0.0 },
                size: Size {
                    width: 50.0,
                    height: 40.0,
                },
            },
        );

        reroute_reinserted_grid_edges(
            &graph,
            &mut result,
            &LayoutOptions {
                seeds: vec![1],
                max_concurrency: 1,
            },
            EngineKind::Tala,
            None,
        )
        .unwrap();

        assert!(
            result.routes[&edge].len() > 2,
            "cross-scope route should avoid the grid container: {:?}",
            result.routes[&edge]
        );
    }

    #[test]
    fn dagre_repositions_a_grid_placeholder_with_its_laid_out_descendants() {
        let mut graph = Graph::default();
        let mut grid = Node::new(
            "grid",
            Size {
                width: 100.0,
                height: 80.0,
            },
        );
        grid.grid_columns = Some(1);
        let grid_id = graph.add_node(grid);
        let mut child = Node::new(
            "grid.child",
            Size {
                width: 30.0,
                height: 20.0,
            },
        );
        child.parent = Some(grid_id);
        let child_id = graph.add_node(child);

        let mut nested = empty_layout_result(&LayoutOptions::default());
        nested.boxes.insert(
            grid_id,
            Rect {
                origin: Point { x: 17.0, y: 23.0 },
                size: Size {
                    width: 100.0,
                    height: 80.0,
                },
            },
        );
        nested.boxes.insert(
            child_id,
            Rect {
                origin: Point { x: 72.0, y: 83.0 },
                size: Size {
                    width: 30.0,
                    height: 20.0,
                },
            },
        );

        let result =
            collapse_dagre_nested_grids(&graph, nested, &LayoutOptions::default(), None).unwrap();

        assert_eq!(
            result.boxes[&grid_id],
            Rect {
                origin: Point::default(),
                size: Size {
                    width: 100.0,
                    height: 80.0,
                },
            }
        );
        assert_eq!(
            result.boxes[&child_id],
            Rect {
                origin: Point { x: 55.0, y: 60.0 },
                size: Size {
                    width: 30.0,
                    height: 20.0,
                },
            }
        );
    }

    #[test]
    fn synthetic_root_grid_is_ignored_for_a_root_sequence_diagram() {
        let mut graph = Graph::default();
        graph.root_sequence_diagram = true;
        graph.root_grid_columns = Some(2);
        assert!(!has_grid_scope(&graph));
    }

    #[test]
    fn empty_sequence_scopes_skip_sequence_layout() {
        // LayoutNested skips an empty root and does not extract an empty
        // special child. Neither form is a no-actors sequence-layout error.
        for implicit in [false, true] {
            let mut graph = Graph::default();
            if implicit {
                graph.root_sequence_diagram = true;
            } else {
                let root = graph.add_node(Node::new(
                    "empty",
                    Size {
                        width: 80.0,
                        height: 30.0,
                    },
                ));
                graph.set_sequence_diagram(root, true);
            }
            for engine in [EngineKind::Tala, EngineKind::Dagre, EngineKind::Elk] {
                let result = LayoutDispatcher
                    .layout(&graph, engine, &LayoutOptions::default())
                    .unwrap();
                assert_eq!(result.boxes.len(), graph.nodes.len());
                assert!(result.sequence_lifelines.is_empty());
                assert!(result.routes.is_empty());
                assert!(
                    result
                        .report
                        .warnings
                        .iter()
                        .all(|warning| !warning.contains("no actors"))
                );
            }
        }
    }

    #[test]
    fn sequence_roots_dispatch_before_all_core_engines() {
        // An implicit sequence root bypasses the core engine. An object
        // sequence leaves a fitted placeholder in its parent graph: ELK
        // places that placeholder at (12,12), while TALA/Dagre use (0,0).
        // The sequence's internal geometry remains identical relative to it.
        for implicit in [false, true] {
            let mut graph = Graph::default();
            let root = if implicit {
                graph.root_sequence_diagram = true;
                graph.root_grid_columns = Some(2);
                None
            } else {
                let root = graph.add_node(Node::new(
                    "sequence",
                    Size {
                        width: 1.0,
                        height: 1.0,
                    },
                ));
                graph.set_sequence_diagram(root, true);
                Some(root)
            };
            let actors = ["a", "b"].map(|name| {
                graph.add_node(Node {
                    parent: root,
                    ..Node::new(
                        name,
                        Size {
                            width: 80.0,
                            height: 30.0,
                        },
                    )
                })
            });
            let message = graph.add_edge(Edge {
                source: actors[0],
                target: actors[1],
            });
            let mut reference: Option<d2_rs_core::LayoutResult> = None;
            for engine in [EngineKind::Tala, EngineKind::Dagre, EngineKind::Elk] {
                let offset = if !implicit && engine == EngineKind::Elk {
                    12.0
                } else {
                    0.0
                };
                let result = LayoutDispatcher
                    .layout(&graph, engine, &LayoutOptions::default())
                    .unwrap();
                assert_eq!(result.sequence_lifelines.len(), 2);
                assert_eq!(result.routes[&message].len(), 2);
                assert_eq!(result.routes[&message][0].y, result.routes[&message][1].y);
                if let Some(root) = root {
                    assert_eq!(
                        result.boxes[&root].origin,
                        Point {
                            x: offset,
                            y: offset
                        }
                    );
                    assert_eq!(
                        result.node_labels[&root].position,
                        LabelPosition::InsideTopCenter
                    );
                }
                if let Some(expected) = &reference {
                    assert_eq!(result.boxes.len(), expected.boxes.len());
                    for (id, rect) in &expected.boxes {
                        assert_eq!(result.boxes[id].size, rect.size);
                        assert_eq!(
                            result.boxes[id].origin,
                            Point {
                                x: rect.origin.x + offset,
                                y: rect.origin.y + offset
                            }
                        );
                    }
                    assert_eq!(result.routes.len(), expected.routes.len());
                    for (id, route) in &expected.routes {
                        assert_eq!(
                            result.routes[id],
                            route
                                .iter()
                                .map(|point| Point {
                                    x: point.x + offset,
                                    y: point.y + offset
                                })
                                .collect::<Vec<_>>()
                        );
                    }
                    for (line, expected_line) in result
                        .sequence_lifelines
                        .iter()
                        .zip(&expected.sequence_lifelines)
                    {
                        assert_eq!(line.actor, expected_line.actor);
                        assert_eq!(line.target_id, expected_line.target_id);
                        assert_eq!(
                            line.route,
                            expected_line
                                .route
                                .iter()
                                .map(|point| Point {
                                    x: point.x + offset,
                                    y: point.y + offset
                                })
                                .collect::<Vec<_>>()
                        );
                    }
                } else {
                    reference = Some(result);
                }
            }
        }
    }

    #[test]
    fn graph_info_default_matches_upstream_scope_predicate() {
        assert!(GraphInfo::default().is_default());
        assert!(
            !GraphInfo {
                is_constant_near: true,
                ..GraphInfo::default()
            }
            .is_default()
        );
        assert!(
            !GraphInfo {
                diagram_type: DiagramType::Grid,
                ..GraphInfo::default()
            }
            .is_default()
        );
    }

    #[test]
    fn dispatcher_runs_elk_source_pipeline() {
        let mut graph = Graph::default();
        graph.add_node(Node::new(
            "a",
            Size {
                width: 40.0,
                height: 40.0,
            },
        ));
        let result = LayoutDispatcher
            .layout(&graph, EngineKind::Elk, &LayoutOptions::default())
            .unwrap();
        assert_eq!(result.boxes.len(), 1);
        assert!(result.report.warnings[0].contains("source-shaped"));
    }

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn elk_grid_actor_lays_out_nested_sequence_before_sizing_cells() {
        let fixture: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/elk-nested-sequence-grid-source-v090.json"
        ))
        .unwrap();
        let mut input = fixture["graph"].clone();
        let mut sequences = Vec::new();
        for (index, node) in input["nodes"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .enumerate()
        {
            if node["shape"] == "sequence_diagram" {
                sequences.push(NodeId(index as u32));
                node["shape"] = serde_json::json!("rectangle");
            }
        }
        let mut graph: Graph = serde_json::from_value(input).unwrap();
        for id in sequences {
            graph.set_sequence_diagram(id, true);
        }
        let result = LayoutDispatcher
            .layout(&graph, EngineKind::Elk, &LayoutOptions::default())
            .unwrap();
        for expected in fixture["expected"].as_array().unwrap() {
            let (id, _) = graph
                .nodes()
                .find(|(_, node)| node.external_id == expected["id"].as_str().unwrap())
                .unwrap();
            let rect = result.boxes[&id];
            let actual = serde_json::json!({"pos":{"x":rect.origin.x as i64,"y":rect.origin.y as i64},"width":rect.size.width as i64,"height":rect.size.height as i64});
            assert_eq!(
                actual,
                serde_json::json!({"pos":expected["pos"],"width":expected["width"],"height":expected["height"]}),
                "{}",
                expected["id"]
            );
        }
    }

    #[cfg(feature = "diagnostic-traces")]
    #[test]
    fn diagnostic_feature_propagates_to_elk() {
        // This is intentionally a compile-time dependency check: the layout
        // crate's diagnostic profile must enable the ELK recorder as well as
        // the Dagre recorder, otherwise `--features diagnostic-traces` would
        // silently leave one built-in engine uninstrumented.
        let _ = d2_rs_elk::trace::TraceRecorder::new();
    }

    #[test]
    fn constant_near_scope_is_removed_before_elk_layout() {
        let mut graph = Graph::default();
        let base = graph.add_node(Node::new(
            "base",
            Size {
                width: 40.0,
                height: 20.0,
            },
        ));
        let mut near = Node::new(
            "near",
            Size {
                width: 10.0,
                height: 10.0,
            },
        );
        near.canvas_position = Some(CanvasPosition::CenterRight);
        let near_id = graph.add_node(near);

        let result = LayoutDispatcher
            .layout(&graph, EngineKind::Elk, &LayoutOptions::default())
            .unwrap();
        // The ordinary ELK adapter includes elk-go's 12px default root
        // envelope before the constant-near reinsertion pass.
        assert_eq!(result.boxes[&base].origin, Point { x: 12.0, y: 12.0 });
        assert_eq!(result.boxes[&near_id].origin, Point { x: 72.0, y: 17.0 });
    }

    #[test]
    fn dagre_constant_near_cross_scope_edges_are_clipped_to_shapes() {
        let mut graph = Graph::default();
        let source = graph.add_node(Node::new(
            "source",
            Size {
                width: 40.0,
                height: 20.0,
            },
        ));
        let mut near = Node::new(
            "near",
            Size {
                width: 20.0,
                height: 20.0,
            },
        );
        near.canvas_position = Some(CanvasPosition::CenterRight);
        let target = graph.add_node(near);
        let edge = graph.add_edge(d2_rs_core::Edge { source, target });

        let result = LayoutDispatcher
            .layout(&graph, EngineKind::Dagre, &LayoutOptions::default())
            .unwrap();

        let source_box = result.boxes[&source];
        let target_box = result.boxes[&target];
        let route = &result.routes[&edge];
        let on_border = |point: Point, rect: d2_rs_core::Rect| {
            let epsilon = 1e-9;
            ((point.x - rect.origin.x).abs() < epsilon || (point.x - rect.right()).abs() < epsilon)
                && point.y >= rect.origin.y - epsilon
                && point.y <= rect.bottom() + epsilon
                || ((point.y - rect.origin.y).abs() < epsilon
                    || (point.y - rect.bottom()).abs() < epsilon)
                    && point.x >= rect.origin.x - epsilon
                    && point.x <= rect.right() + epsilon
        };

        assert!(
            on_border(route[0], source_box),
            "source endpoint: {:?}",
            route[0]
        );
        assert!(
            on_border(*route.last().unwrap(), target_box),
            "target endpoint: {:?}",
            route.last().unwrap()
        );
    }

    #[test]
    fn dispatcher_runs_dagre_in_process() {
        let mut graph = Graph::default();
        let a = graph.add_node(Node::new(
            "a",
            Size {
                width: 40.0,
                height: 40.0,
            },
        ));
        let b = graph.add_node(Node::new(
            "b",
            Size {
                width: 40.0,
                height: 40.0,
            },
        ));
        graph.add_edge(d2_rs_core::Edge {
            source: a,
            target: b,
        });
        let result = LayoutDispatcher
            .layout(&graph, EngineKind::Dagre, &LayoutOptions::default())
            .unwrap();
        assert_eq!(result.boxes.len(), 2);
        assert_eq!(result.routes.len(), 1);
    }

    #[test]
    fn dagre_root_grid_lays_out_nested_grid_cells_before_root_and_routes_cross_cell_edges() {
        let mut graph = Graph {
            root_grid_rows: Some(1),
            root_grid_columns: Some(2),
            ..Graph::default()
        };
        let a = graph.add_node(Node::new(
            "a",
            Size {
                width: 53.0,
                height: 66.0,
            },
        ));
        let b = graph.add_node(Node {
            grid_rows: Some(1),
            grid_columns: Some(1),
            ..Node::new(
                "b",
                Size {
                    width: 58.0,
                    height: 81.0,
                },
            )
        });
        let aa = graph.add_node(Node::new(
            "b.AA",
            Size {
                width: 64.0,
                height: 66.0,
            },
        ));
        let bb = graph.add_node(Node::new(
            "b.AA.BB",
            Size {
                width: 64.0,
                height: 66.0,
            },
        ));
        graph.node_mut(aa).unwrap().parent = Some(b);
        graph.node_mut(bb).unwrap().parent = Some(aa);
        graph.set_container_flag(b, true);
        graph.set_container_flag(aa, true);
        let edge = graph.add_edge(d2_rs_core::Edge {
            source: a,
            target: aa,
        });

        let result = LayoutDispatcher
            .layout(&graph, EngineKind::Dagre, &LayoutOptions::default())
            .unwrap();

        assert_eq!(result.boxes.len(), 4);
        assert_eq!(result.routes[&edge].len(), 2);
        assert!(result.boxes[&b].size.width > result.boxes[&aa].size.width);
        assert!(result.boxes[&aa].size.width > result.boxes[&bb].size.width);
        assert_eq!(result.boxes[&a].size.height, result.boxes[&b].size.height);
        assert!(result.boxes[&b].origin.x > result.boxes[&a].origin.x);
    }

    #[test]
    fn tala_root_grid_positions_nested_cells_after_the_cell_engine_pass() {
        let mut graph = Graph {
            root_grid_rows: Some(1),
            root_grid_columns: Some(2),
            ..Graph::default()
        };
        let a = graph.add_node(Node::new(
            "a",
            Size {
                width: 53.0,
                height: 66.0,
            },
        ));
        let b = graph.add_node(Node {
            grid_rows: Some(1),
            grid_columns: Some(1),
            ..Node::new(
                "b",
                Size {
                    width: 58.0,
                    height: 81.0,
                },
            )
        });
        let aa = graph.add_node(Node::new(
            "b.AA",
            Size {
                width: 64.0,
                height: 66.0,
            },
        ));
        let bb = graph.add_node(Node::new(
            "b.AA.BB",
            Size {
                width: 64.0,
                height: 66.0,
            },
        ));
        graph.node_mut(aa).unwrap().parent = Some(b);
        graph.node_mut(bb).unwrap().parent = Some(aa);
        graph.set_container_flag(b, true);
        graph.set_container_flag(aa, true);
        graph.add_edge(Edge {
            source: a,
            target: aa,
        });

        let result = LayoutDispatcher
            .layout(
                &graph,
                EngineKind::Tala,
                &LayoutOptions {
                    seeds: vec![1],
                    max_concurrency: 1,
                },
            )
            .unwrap();

        assert_eq!(result.boxes[&b].origin, Point { x: 93.0, y: 0.0 });
        assert_eq!(result.boxes[&aa].origin, Point { x: 153.0, y: 60.0 });
        assert_eq!(result.boxes[&bb].origin, Point { x: 213.0, y: 120.0 });
    }

    #[test]
    fn dispatcher_preserves_each_core_engines_context_boundary() {
        let mut graph = Graph::default();
        graph.add_node(Node::new(
            "a",
            Size {
                width: 40.0,
                height: 40.0,
            },
        ));
        let token = CancellationToken::new();
        token.cancel();
        assert!(matches!(
            LayoutDispatcher.layout_with_cancellation(
                &graph,
                EngineKind::Tala,
                &LayoutOptions::default(),
                Some(&token),
            ),
            Err(LayoutDispatchError::Tala(
                d2_rs_core::LayoutError::Cancelled
            ))
        ));
        assert_eq!(
            LayoutDispatcher
                .layout_with_cancellation(
                    &graph,
                    EngineKind::Dagre,
                    &LayoutOptions::default(),
                    Some(&token),
                )
                .unwrap_err()
                .to_string(),
            "failed to dagre layout: context canceled"
        );
        assert_eq!(
            LayoutDispatcher
                .layout_with_cancellation(
                    &graph,
                    EngineKind::Elk,
                    &LayoutOptions::default(),
                    Some(&token),
                )
                .unwrap_err()
                .to_string(),
            "failed to ELK layout: context canceled"
        );
    }

    #[test]
    fn dispatcher_routes_tala_in_process() {
        let mut graph = Graph::default();
        let a = graph.add_node(Node::new(
            "a",
            Size {
                width: 40.0,
                height: 40.0,
            },
        ));
        let b = graph.add_node(Node::new(
            "b",
            Size {
                width: 40.0,
                height: 40.0,
            },
        ));
        graph.add_edge(d2_rs_core::Edge {
            source: a,
            target: b,
        });
        let result = LayoutDispatcher
            .layout(
                &graph,
                EngineKind::Tala,
                &LayoutOptions {
                    seeds: vec![1],
                    max_concurrency: 1,
                },
            )
            .unwrap();
        assert_eq!(result.boxes.len(), 2);
        assert_eq!(result.routes.len(), 1);
    }

    #[test]
    fn nested_grid_scope_is_applied_after_general_tala() {
        let mut graph = Graph::default();
        let root = graph.add_node(Node {
            grid_rows: Some(1),
            size: Size {
                width: 80.0,
                height: 80.0,
            },
            ..Node::new(
                "root",
                Size {
                    width: 80.0,
                    height: 80.0,
                },
            )
        });
        let container = graph.add_node(Node::new(
            "root.container",
            Size {
                width: 60.0,
                height: 60.0,
            },
        ));
        let leaf = graph.add_node(Node::new(
            "root.container.leaf",
            Size {
                width: 40.0,
                height: 40.0,
            },
        ));
        graph.node_mut(container).unwrap().parent = Some(root);
        graph.node_mut(leaf).unwrap().parent = Some(container);

        let result = LayoutDispatcher
            .layout(
                &graph,
                EngineKind::Tala,
                &LayoutOptions {
                    seeds: vec![1],
                    max_concurrency: 1,
                },
            )
            .unwrap();
        assert_eq!(result.boxes.len(), 3);
    }

    #[test]
    fn nested_grid_external_edges_use_tala_after_scope_positions_are_final() {
        let mut graph = Graph::default();
        let root = graph.add_node(Node {
            grid_columns: Some(2),
            size: Size {
                width: 853.0,
                height: 306.0,
            },
            ..Node::new("root", Size::default())
        });
        let source = graph.add_node(Node {
            parent: Some(root),
            ..Node::new(
                "root.source",
                Size {
                    width: 57.0,
                    height: 186.0,
                },
            )
        });
        let container = graph.add_node(Node {
            parent: Some(root),
            ..Node::new(
                "root.container",
                Size {
                    width: 197.0,
                    height: 186.0,
                },
            )
        });
        let target = graph.add_node(Node {
            parent: Some(container),
            ..Node::new(
                "root.container.target",
                Size {
                    width: 77.0,
                    height: 66.0,
                },
            )
        });
        let edge = graph.add_edge(Edge { source, target });
        let mut result = empty_layout_result(&LayoutOptions::default());
        result.boxes.insert(
            root,
            Rect {
                origin: Point::default(),
                size: Size {
                    width: 853.0,
                    height: 306.0,
                },
            },
        );
        result.boxes.insert(
            source,
            Rect {
                origin: Point { x: 60.0, y: 60.0 },
                size: Size {
                    width: 57.0,
                    height: 186.0,
                },
            },
        );
        result.boxes.insert(
            container,
            Rect {
                origin: Point { x: 157.0, y: 60.0 },
                size: Size {
                    width: 197.0,
                    height: 186.0,
                },
            },
        );
        result.boxes.insert(
            target,
            Rect {
                origin: Point { x: 217.0, y: 120.0 },
                size: Size {
                    width: 77.0,
                    height: 66.0,
                },
            },
        );
        result.routes.insert(
            edge,
            vec![Point { x: 116.5, y: 153.0 }, Point { x: 216.5, y: 153.0 }],
        );

        reroute_nested_grid_external_edges(
            &graph,
            &mut result,
            &LayoutOptions {
                seeds: vec![1],
                max_concurrency: 1,
            },
            None,
        )
        .unwrap();

        assert_eq!(
            result.routes[&edge],
            vec![Point { x: 117.0, y: 153.0 }, Point { x: 217.0, y: 153.0 }]
        );
    }

    #[test]
    fn implicit_root_grid_external_edges_use_tala_after_cell_reinsertion() {
        let mut graph = Graph {
            root_grid_columns: Some(2),
            ..Graph::default()
        };
        let source = graph.add_node(Node::new(
            "source",
            Size {
                width: 57.0,
                height: 186.0,
            },
        ));
        let container = graph.add_node(Node::new(
            "container",
            Size {
                width: 197.0,
                height: 186.0,
            },
        ));
        let target = graph.add_node(Node {
            parent: Some(container),
            ..Node::new(
                "container.target",
                Size {
                    width: 77.0,
                    height: 66.0,
                },
            )
        });
        let edge = graph.add_edge(Edge { source, target });
        let mut result = empty_layout_result(&LayoutOptions::default());
        result.boxes.insert(
            source,
            Rect {
                origin: Point { x: 0.0, y: 0.0 },
                size: Size {
                    width: 57.0,
                    height: 186.0,
                },
            },
        );
        result.boxes.insert(
            container,
            Rect {
                origin: Point { x: 97.0, y: 0.0 },
                size: Size {
                    width: 197.0,
                    height: 186.0,
                },
            },
        );
        result.boxes.insert(
            target,
            Rect {
                origin: Point { x: 157.0, y: 60.0 },
                size: Size {
                    width: 77.0,
                    height: 66.0,
                },
            },
        );
        result.routes.insert(
            edge,
            vec![Point { x: 56.5, y: 93.0 }, Point { x: 156.5, y: 93.0 }],
        );

        reroute_nested_grid_external_edges(
            &graph,
            &mut result,
            &LayoutOptions {
                seeds: vec![1],
                max_concurrency: 1,
            },
            None,
        )
        .unwrap();

        assert_eq!(
            result.routes[&edge],
            vec![Point { x: 57.0, y: 93.0 }, Point { x: 157.0, y: 93.0 }]
        );
    }

    #[test]
    fn dagre_keeps_nested_direction_metadata_out_of_root_layout() {
        let mut graph = Graph::default();
        let scope = graph.add_node(Node {
            direction: Some(Direction::Right),
            ..Node::new(
                "scope",
                Size {
                    width: 1.0,
                    height: 1.0,
                },
            )
        });
        let left = graph.add_node(Node::new(
            "scope.left",
            Size {
                width: 30.0,
                height: 20.0,
            },
        ));
        let right = graph.add_node(Node::new(
            "scope.right",
            Size {
                width: 30.0,
                height: 20.0,
            },
        ));
        graph.node_mut(left).unwrap().parent = Some(scope);
        graph.node_mut(right).unwrap().parent = Some(scope);
        graph.add_edge(d2_rs_core::Edge {
            source: left,
            target: right,
        });

        let result = LayoutDispatcher
            .layout(&graph, EngineKind::Dagre, &LayoutOptions::default())
            .unwrap();
        let left_box = result.boxes.get(&left).unwrap();
        let right_box = result.boxes.get(&right).unwrap();
        // d2dagrelayout only reads the extracted graph root's direction. A
        // regular container is laid out as part of the root Dagro graph, so
        // its object-level direction metadata must not trigger a second
        // layout pass or change the root's top-to-bottom ranks.
        assert!((right_box.origin.x - left_box.origin.x).abs() < 1.0);
        assert!(right_box.origin.y > left_box.origin.y);
    }

    #[test]
    fn dagre_constant_near_grid_projection_uses_near_root_direction() {
        let mut graph = Graph::default();
        let near = graph.add_node(Node {
            direction: Some(d2_rs_core::Direction::Right),
            canvas_position: Some(CanvasPosition::CenterRight),
            ..Node::new(
                "near",
                Size {
                    width: 80.0,
                    height: 50.0,
                },
            )
        });
        let grid = graph.add_node(Node {
            grid_columns: Some(1),
            ..Node::new(
                "near.grid",
                Size {
                    width: 1.0,
                    height: 1.0,
                },
            )
        });
        let cell = graph.add_node(Node::new(
            "near.grid.cell",
            Size {
                width: 20.0,
                height: 20.0,
            },
        ));
        let left = graph.add_node(Node::new(
            "near.left",
            Size {
                width: 30.0,
                height: 20.0,
            },
        ));
        let right = graph.add_node(Node::new(
            "near.right",
            Size {
                width: 30.0,
                height: 20.0,
            },
        ));
        graph.node_mut(grid).unwrap().parent = Some(near);
        graph.node_mut(cell).unwrap().parent = Some(grid);
        graph.node_mut(left).unwrap().parent = Some(near);
        graph.node_mut(right).unwrap().parent = Some(near);
        graph.add_edge(d2_rs_core::Edge {
            source: left,
            target: right,
        });

        let result = LayoutDispatcher
            .layout(&graph, EngineKind::Dagre, &LayoutOptions::default())
            .unwrap();
        let left_box = result.boxes.get(&left).unwrap();
        let right_box = result.boxes.get(&right).unwrap();
        assert!(right_box.origin.x > left_box.origin.x);
        assert!((right_box.origin.y - left_box.origin.y).abs() < 1.0);
    }

    #[test]
    fn tala_constant_near_grid_root_keeps_grid_label_default() {
        let mut graph = Graph::default();
        let grid = graph.add_node(Node {
            grid_columns: Some(1),
            canvas_position: Some(CanvasPosition::CenterRight),
            label_size: Some(Size {
                width: 234.0,
                height: 36.0,
            }),
            ..Node::new(
                "near-grid",
                Size {
                    width: 279.0,
                    height: 81.0,
                },
            )
        });
        let cell = graph.add_node(Node {
            parent: Some(grid),
            ..Node::new(
                "near-grid.cell",
                Size {
                    width: 80.0,
                    height: 50.0,
                },
            )
        });
        let anchor = graph.add_node(Node::new(
            "anchor",
            Size {
                width: 50.0,
                height: 40.0,
            },
        ));
        graph.node_labels.insert(grid, "Right Constant Near".into());
        graph.set_hierarchy_children_order(BTreeMap::from([
            (None, vec![anchor, grid]),
            (Some(grid), vec![cell]),
        ]));

        let result = LayoutDispatcher
            .layout(&graph, EngineKind::Tala, &LayoutOptions::default())
            .unwrap();

        assert_eq!(
            result.node_labels[&grid].position,
            d2_rs_core::LabelPosition::InsideTopCenter
        );
    }

    #[test]
    fn dagre_grid_cell_uses_its_own_direction_default() {
        let mut graph = Graph {
            direction: d2_rs_core::Direction::Right,
            root_grid_columns: Some(1),
            ..Graph::default()
        };
        let cell = graph.add_node(Node::new(
            "cell",
            Size {
                width: 50.0,
                height: 50.0,
            },
        ));
        let first = graph.add_node(Node::new(
            "cell.first",
            Size {
                width: 30.0,
                height: 20.0,
            },
        ));
        let second = graph.add_node(Node::new(
            "cell.second",
            Size {
                width: 30.0,
                height: 20.0,
            },
        ));
        graph.node_mut(first).unwrap().parent = Some(cell);
        graph.node_mut(second).unwrap().parent = Some(cell);
        graph.add_edge(d2_rs_core::Edge {
            source: first,
            target: second,
        });

        let result = LayoutDispatcher
            .layout(&graph, EngineKind::Dagre, &LayoutOptions::default())
            .unwrap();
        let first_box = result.boxes.get(&first).unwrap();
        let second_box = result.boxes.get(&second).unwrap();
        assert!((second_box.origin.x - first_box.origin.x).abs() < 1.0);
        assert!(second_box.origin.y > first_box.origin.y);
    }

    #[test]
    fn nested_grid_scope_places_siblings_and_moves_descendants_with_container() {
        let mut graph = Graph::default();
        let root = graph.add_node(Node {
            grid_rows: Some(1),
            grid_columns: Some(2),
            size: Size {
                width: 1.0,
                height: 1.0,
            },
            ..Node::new(
                "root",
                Size {
                    width: 1.0,
                    height: 1.0,
                },
            )
        });
        graph.set_grid_spacing(
            root,
            d2_rs_core::GridSpacing {
                horizontal_gap: Some(10.0),
                ..d2_rs_core::GridSpacing::default()
            },
        );
        let a = graph.add_node(Node::new(
            "root.a",
            Size {
                width: 30.0,
                height: 20.0,
            },
        ));
        let container = graph.add_node(Node::new(
            "root.container",
            Size {
                width: 50.0,
                height: 20.0,
            },
        ));
        let leaf = graph.add_node(Node::new(
            "root.container.leaf",
            Size {
                width: 20.0,
                height: 10.0,
            },
        ));
        graph.node_mut(a).unwrap().parent = Some(root);
        graph.node_mut(container).unwrap().parent = Some(root);
        graph.node_mut(leaf).unwrap().parent = Some(container);
        graph.add_edge(d2_rs_core::Edge {
            source: a,
            target: leaf,
        });

        let result = LayoutDispatcher
            .layout(
                &graph,
                EngineKind::Tala,
                &LayoutOptions {
                    seeds: vec![1],
                    max_concurrency: 1,
                },
            )
            .unwrap();
        assert_eq!(result.boxes.len(), 4);
        assert_eq!(result.routes.len(), 1);
        assert_eq!(
            result.boxes[&container].origin.x - result.boxes[&a].origin.x,
            40.0
        );
        let container_to_leaf = Point {
            x: result.boxes[&leaf].origin.x - result.boxes[&container].origin.x,
            y: result.boxes[&leaf].origin.y - result.boxes[&container].origin.y,
        };
        assert!(container_to_leaf.x.is_finite() && container_to_leaf.y.is_finite());
    }

    #[test]
    fn sibling_root_grids_are_extracted_before_parent_tala_packing() {
        let mut graph = Graph::default();
        let first = graph.add_node(Node {
            grid_rows: Some(1),
            label_size: Some(Size {
                width: 24.0,
                height: 21.0,
            }),
            size: Size {
                width: 100.0,
                height: 81.0,
            },
            ..Node::new(
                "one",
                Size {
                    width: 100.0,
                    height: 81.0,
                },
            )
        });
        let second = graph.add_node(Node {
            grid_rows: Some(1),
            label_size: Some(Size {
                width: 24.0,
                height: 21.0,
            }),
            size: Size {
                width: 200.0,
                height: 81.0,
            },
            ..Node::new(
                "two",
                Size {
                    width: 200.0,
                    height: 81.0,
                },
            )
        });
        let first_child = graph.add_node(Node::new(
            "one.a",
            Size {
                width: 100.0,
                height: 61.0,
            },
        ));
        let second_child = graph.add_node(Node::new(
            "two.b",
            Size {
                width: 200.0,
                height: 61.0,
            },
        ));
        graph.node_mut(first_child).unwrap().parent = Some(first);
        graph.node_mut(second_child).unwrap().parent = Some(second);

        let result = LayoutDispatcher
            .layout(
                &graph,
                EngineKind::Tala,
                &LayoutOptions {
                    seeds: vec![1],
                    max_concurrency: 1,
                },
            )
            .unwrap();
        assert_eq!(result.boxes[&first].origin.y, 0.0);
        assert_eq!(result.boxes[&second].origin.y, 201.0);
        assert_eq!(
            result.boxes[&first_child].origin.y - result.boxes[&first].origin.y,
            60.0
        );
        assert_eq!(
            result.boxes[&second_child].origin.y - result.boxes[&second].origin.y,
            60.0
        );
    }

    #[test]
    fn implicit_root_grid_uses_lowered_rows_columns_and_gap() {
        let mut graph = Graph {
            root_grid_rows: Some(1),
            root_grid_columns: Some(2),
            root_grid_gap: Some(10.0),
            ..Graph::default()
        };
        let a = graph.add_node(Node::new(
            "a",
            Size {
                width: 40.0,
                height: 20.0,
            },
        ));
        let b = graph.add_node(Node::new(
            "b",
            Size {
                width: 50.0,
                height: 20.0,
            },
        ));
        graph.add_edge(d2_rs_core::Edge {
            source: a,
            target: b,
        });

        let result = LayoutDispatcher
            .layout(
                &graph,
                EngineKind::Tala,
                &LayoutOptions {
                    seeds: vec![1],
                    max_concurrency: 1,
                },
            )
            .unwrap();
        assert_eq!(result.boxes.len(), 2);
        assert_eq!(result.routes.len(), 1);
        assert_eq!(result.boxes[&b].origin.x - result.boxes[&a].origin.x, 50.0);
    }

    #[test]
    fn nested_sequence_scope_replaces_general_routes_after_tala() {
        let mut graph = Graph::default();
        let outer = graph.add_node(Node::new(
            "outer",
            Size {
                width: 140.0,
                height: 100.0,
            },
        ));
        let sequence = graph.add_node(Node::new(
            "outer.sequence",
            Size {
                width: 120.0,
                height: 80.0,
            },
        ));
        let first = graph.add_node(Node::new(
            "outer.sequence.first",
            Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        let second = graph.add_node(Node::new(
            "outer.sequence.second",
            Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        let outside = graph.add_node(Node::new(
            "outside",
            Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        graph.node_mut(sequence).unwrap().parent = Some(outer);
        graph.node_mut(first).unwrap().parent = Some(sequence);
        graph.node_mut(second).unwrap().parent = Some(sequence);
        graph.set_sequence_diagram(sequence, true);
        let inside_edge = graph.add_edge(d2_rs_core::Edge {
            source: first,
            target: second,
        });
        graph.add_edge(d2_rs_core::Edge {
            source: outside,
            target: first,
        });

        let result = LayoutDispatcher
            .layout(
                &graph,
                EngineKind::Tala,
                &LayoutOptions {
                    seeds: vec![1],
                    max_concurrency: 1,
                },
            )
            .unwrap();
        let route = &result.routes[&inside_edge];
        assert_eq!(route.len(), 2);
        assert_ne!(route[0].x, route[1].x);
        assert!(result.boxes[&first].origin.y < route[0].y);
    }

    #[test]
    fn extracted_grid_cell_cycle_is_independent_of_its_outer_namespace() {
        let make_graph = |namespace: &str| {
            let mut graph = Graph::default();
            let size = Size {
                width: 80.0,
                height: 40.0,
            };
            let mut grid = Node::new(namespace, size);
            grid.grid_columns = Some(1);
            let grid = graph.add_node(grid);
            let mut cell = Node::new(format!("{namespace}.container"), size);
            cell.parent = Some(grid);
            let cell = graph.add_node(cell);
            let actors = ["a", "b", "c"].map(|name| {
                let mut node = Node::new(format!("{namespace}.container.{name}"), size);
                node.parent = Some(cell);
                graph.add_node(node)
            });
            for (source, target) in [
                (actors[0], actors[1]),
                (actors[1], actors[2]),
                (actors[2], actors[0]),
            ] {
                graph.add_edge(Edge { source, target });
            }
            graph
        };
        let first = make_graph("outer-grid");
        let second = make_graph("renamed-grid");
        let options = LayoutOptions {
            seeds: vec![1],
            max_concurrency: 1,
        };
        let layout = |graph: &Graph| {
            LayoutDispatcher
                .layout(graph, EngineKind::Tala, &options)
                .unwrap()
        };
        let a = layout(&first);
        let b = layout(&second);
        assert_eq!(a.boxes, b.boxes);
        assert_eq!(a.routes, b.routes);
        assert_eq!(
            first.node(NodeId(2)).unwrap().external_id,
            "outer-grid.container.a"
        );
        assert_eq!(
            second.node(NodeId(2)).unwrap().external_id,
            "renamed-grid.container.a"
        );
    }

    #[test]
    fn ordinary_grid_cell_reinjects_sequence_routes_and_fixed_lifelines() {
        let mut graph = Graph::default();
        let size = Size {
            width: 100.0,
            height: 40.0,
        };
        let mut grid = Node::new("grid", size);
        grid.grid_rows = Some(2);
        let grid = graph.add_node(grid);
        let mut cell = Node::new("grid.cell", size);
        cell.parent = Some(grid);
        let cell = graph.add_node(cell);
        let mut sequence = Node::new("grid.cell.sequence", size);
        sequence.parent = Some(cell);
        let sequence = graph.add_node(sequence);
        graph.set_sequence_diagram(sequence, true);
        let mut actor = Node::new("grid.cell.sequence.a", size);
        actor.parent = Some(sequence);
        let a = graph.add_node(actor);
        let mut actor = Node::new("grid.cell.sequence.b", size);
        actor.parent = Some(sequence);
        let b = graph.add_node(actor);
        let mut other = Node::new("grid.other", size);
        other.parent = Some(grid);
        let other = graph.add_node(other);
        let message = graph.add_edge(Edge {
            source: a,
            target: b,
        });
        let crossing = graph.add_edge(Edge {
            source: other,
            target: sequence,
        });
        for engine in [EngineKind::Tala, EngineKind::Dagre] {
            let result = LayoutDispatcher
                .layout(&graph, engine, &LayoutOptions::default())
                .unwrap();
            assert_eq!(result.sequence_lifelines.len(), 2);
            let route = &result.routes[&message];
            assert_eq!(route.len(), 2);
            assert_eq!(route[0].y, route[1].y);
            assert!(result.boxes[&a].bottom() < route[0].y);
            assert!(result.boxes[&b].bottom() < route[1].y);
            for line in &result.sequence_lifelines {
                assert_eq!(line.route[0].y, result.boxes[&line.actor].bottom());
                assert!(line.route[1].y >= route[0].y);
                assert!(line.actor == a || line.actor == b);
            }
            assert!(!result.routes[&crossing].is_empty());
        }
    }

    #[test]
    fn sequence_grid_cell_runs_its_own_layout_before_grid_measurement() {
        let mut graph = Graph::default();
        let size = Size {
            width: 100.0,
            height: 40.0,
        };
        let mut grid = Node::new("grid", size);
        grid.grid_rows = Some(2);
        let grid = graph.add_node(grid);
        let mut sequence = Node::new("grid.sequence", size);
        sequence.parent = Some(grid);
        let sequence = graph.add_node(sequence);
        graph.set_sequence_diagram(sequence, true);
        let mut actor = Node::new("grid.sequence.a", size);
        actor.parent = Some(sequence);
        let a = graph.add_node(actor);
        let mut actor = Node::new("grid.sequence.b", size);
        actor.parent = Some(sequence);
        let b = graph.add_node(actor);
        let mut other = Node::new("grid.other", size);
        other.parent = Some(grid);
        let other = graph.add_node(other);
        let message = graph.add_edge(Edge {
            source: a,
            target: b,
        });
        let crossing = graph.add_edge(Edge {
            source: other,
            target: sequence,
        });
        for engine in [EngineKind::Tala, EngineKind::Dagre, EngineKind::Elk] {
            let result = LayoutDispatcher
                .layout(&graph, engine, &LayoutOptions::default())
                .unwrap();
            assert_eq!(result.sequence_lifelines.len(), 2);
            let route = &result.routes[&message];
            assert_eq!(route.len(), 2);
            assert_eq!(route[0].y, route[1].y);
            assert!(result.boxes[&a].bottom() < route[0].y);
            assert!(result.boxes[&b].bottom() < route[1].y);
            for line in &result.sequence_lifelines {
                assert_eq!(line.route[0].y, result.boxes[&line.actor].bottom());
                assert!(line.route[1].y >= route[0].y);
            }
            assert!(!result.routes[&crossing].is_empty());
        }
    }

    #[test]
    fn reinserted_grid_keeps_internal_edge_label_state() {
        let mut graph = Graph::default();
        let mut grid = Node::new(
            "grid",
            Size {
                width: 100.0,
                height: 100.0,
            },
        );
        grid.grid_columns = Some(2);
        let grid = graph.add_node(grid);
        let mut a = Node::new(
            "grid.a",
            Size {
                width: 80.0,
                height: 60.0,
            },
        );
        a.parent = Some(grid);
        let a = graph.add_node(a);
        let mut b = Node::new(
            "grid.b",
            Size {
                width: 80.0,
                height: 60.0,
            },
        );
        b.parent = Some(grid);
        let b = graph.add_node(b);
        graph.add_node(Node::new(
            "outside",
            Size {
                width: 80.0,
                height: 60.0,
            },
        ));
        let edge = graph.add_edge(Edge {
            source: a,
            target: b,
        });
        graph.set_edge_label(
            edge,
            Some(EdgeLabel {
                text: "internal".to_owned(),
                size: Size {
                    width: 60.0,
                    height: 21.0,
                },
                position: LabelPosition::Unset,
                percentage: 0.0,
            }),
        );
        for engine in [EngineKind::Tala, EngineKind::Dagre, EngineKind::Elk] {
            let result = LayoutDispatcher
                .layout(
                    &graph,
                    engine.clone(),
                    &LayoutOptions {
                        seeds: vec![1],
                        max_concurrency: 1,
                    },
                )
                .unwrap();
            assert_eq!(
                result.edge_labels[&edge].position,
                LabelPosition::InsideMiddleCenter,
                "{engine:?}"
            );
            assert_eq!(result.edge_labels[&edge].percentage, 0.0, "{engine:?}");
        }
    }

    #[test]
    fn grid_image_cells_publish_icon_position_for_every_engine() {
        let mut graph = Graph::default();
        let mut grid = Node::new(
            "grid",
            Size {
                width: 128.0,
                height: 128.0,
            },
        );
        grid.grid_columns = Some(1);
        let grid = graph.add_node(grid);
        let mut image = Node::new(
            "grid.image",
            Size {
                width: 128.0,
                height: 128.0,
            },
        );
        image.parent = Some(grid);
        image.shape = d2_rs_core::ShapeKind::Image;
        image.has_icon = true;
        // SetDimensions defaults image labels below the image before layout.
        image.label_position = LabelPosition::OutsideBottomCenter;
        image.label_size = Some(Size {
            width: 40.0,
            height: 21.0,
        });
        let image = graph.add_node(image);
        for engine in [EngineKind::Dagre, EngineKind::Elk, EngineKind::Tala] {
            let result = LayoutDispatcher
                .layout(
                    &graph,
                    engine.clone(),
                    &LayoutOptions {
                        seeds: vec![1],
                        max_concurrency: 1,
                    },
                )
                .unwrap();
            assert_eq!(
                result.node_labels[&image].icon_position,
                Some(LabelPosition::InsideMiddleCenter),
                "{engine:?}"
            );
            assert_eq!(
                result.node_labels[&image].position,
                LabelPosition::OutsideBottomCenter,
                "{engine:?}"
            );
        }
    }

    #[test]
    fn deferred_parent_routing_keeps_sequence_locals_without_committing_crossings() {
        let mut graph = Graph::default();
        let sequence = graph.add_node(Node::new(
            "sequence",
            Size {
                width: 120.0,
                height: 80.0,
            },
        ));
        graph.set_sequence_diagram(sequence, true);
        let mut actor = Node::new(
            "sequence.actor",
            Size {
                width: 80.0,
                height: 60.0,
            },
        );
        actor.parent = Some(sequence);
        let actor = graph.add_node(actor);
        let outside = graph.add_node(Node::new(
            "outside",
            Size {
                width: 80.0,
                height: 60.0,
            },
        ));
        let message = graph.add_edge(Edge {
            source: actor,
            target: actor,
        });
        let crossing = graph.add_edge(Edge {
            source: actor,
            target: outside,
        });
        graph.set_edge_label(
            crossing,
            Some(EdgeLabel {
                text: "crossing".to_owned(),
                size: Size {
                    width: 60.0,
                    height: 21.0,
                },
                position: LabelPosition::OutsideTopCenter,
                percentage: 0.0,
            }),
        );
        let options = LayoutOptions {
            seeds: vec![1],
            max_concurrency: 1,
        };
        let deferred = LayoutDispatcher
            .layout_with_scope_cache_deferred(
                &graph,
                EngineKind::Tala,
                &options,
                None,
                false,
                None,
                true,
            )
            .unwrap();
        let ordinary = LayoutDispatcher
            .layout(&graph, EngineKind::Tala, &options)
            .unwrap();
        assert_eq!(deferred.boxes, ordinary.boxes);
        assert_eq!(deferred.routes[&message], ordinary.routes[&message]);
        assert!(!deferred.routes.contains_key(&crossing));
        assert!(!deferred.edge_labels.contains_key(&crossing));
        assert!(ordinary.routes.contains_key(&crossing));
        assert!(ordinary.edge_labels.contains_key(&crossing));
    }

    #[test]
    fn nested_sequence_is_sized_before_tala_places_external_siblings() {
        let mut graph = Graph::default();
        let mut sequence = Node::new(
            "sequence",
            Size {
                width: 157.0,
                height: 81.0,
            },
        );
        sequence.label_size = Some(Size {
            width: 110.0,
            height: 36.0,
        });
        let sequence = graph.add_node(sequence);
        graph.set_sequence_diagram(sequence, true);
        let mut worker = Node::new(
            "sequence.worker",
            Size {
                width: 93.0,
                height: 66.0,
            },
        );
        worker.parent = Some(sequence);
        worker.label_size = Some(Size {
            width: 52.0,
            height: 21.0,
        });
        let worker = graph.add_node(worker);
        let mut complete = Node::new(
            "complete",
            Size {
                width: 114.0,
                height: 66.0,
            },
        );
        complete.label_size = Some(Size {
            width: 70.0,
            height: 21.0,
        });
        let complete = graph.add_node(complete);
        let message = graph.add_edge(Edge {
            source: worker,
            target: worker,
        });
        graph.set_edge_label(
            message,
            Some(EdgeLabel {
                text: "retry".to_owned(),
                size: Size {
                    width: 35.0,
                    height: 21.0,
                },
                position: LabelPosition::Unset,
                percentage: 0.0,
            }),
        );
        graph.add_edge(Edge {
            source: sequence,
            target: complete,
        });

        let result = LayoutDispatcher
            .layout(
                &graph,
                EngineKind::Tala,
                &LayoutOptions {
                    seeds: vec![1],
                    max_concurrency: 1,
                },
            )
            .unwrap();

        let sequence_box = result.boxes[&sequence];
        let complete_box = result.boxes[&complete];
        assert!(sequence_box.size.height > 81.0);
        assert!((sequence_box.center().y - complete_box.center().y).abs() <= 0.5);
        assert_eq!(result.routes[&message].len(), 4);
    }
}

// d2layouts.validateObjectPositions runs after LayoutNested has restored and
// routed its graph. It rejects infinities only; NaN is not this source error.
fn validate_object_positions(
    graph: &Graph,
    result: &LayoutResult,
) -> Result<(), LayoutDispatchError> {
    for (id, node) in graph.nodes() {
        if let Some(rect) = result.boxes.get(&id)
            && (rect.origin.x.is_infinite() || rect.origin.y.is_infinite())
        {
            return Err(CoreLayoutError::InvalidPosition {
                object: node.external_id.clone(),
                x: rect.origin.x,
                y: rect.origin.y,
            }
            .into());
        }
    }
    Ok(())
}

// Every test here reads a regenerated oracle capture (docs/FIXTURES.md).
#[cfg(all(test, feature = "oracle-fixtures"))]
mod position_validation_source_contract {
    use super::*;
    use d2_rs_core::Size;
    #[test]
    fn infinite_coordinates_use_go_quoting_and_float_format_and_nan_is_accepted() {
        let fixture: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/sequence-invalid-positions-source-v090.json"
        ))
        .unwrap();
        for (index, (name, x, y)) in [
            ("positive", f64::INFINITY, f64::INFINITY),
            ("negative", f64::NEG_INFINITY, -0.0),
            ("quoted\"", 1.2345678, f64::INFINITY),
            ("nan", f64::NAN, f64::NAN),
            ("finite", 1.0, 2.0),
        ]
        .into_iter()
        .enumerate()
        {
            let mut graph = Graph::default();
            let id = graph.add_node(d2_rs_core::Node::new(
                name,
                Size {
                    width: 1.0,
                    height: 1.0,
                },
            ));
            let mut result = empty_layout_result(&LayoutOptions::default());
            result.boxes.insert(
                id,
                Rect {
                    origin: Point { x, y },
                    size: Size {
                        width: 1.0,
                        height: 1.0,
                    },
                },
            );
            let observed = match validate_object_positions(&graph, &result) {
                Ok(()) => format!("{name} accepted"),
                Err(error) => error.to_string(),
            };
            assert_eq!(observed, fixture["validator"][index].as_str().unwrap());
        }
    }
}

#[cfg(test)]
mod collapsed_sequence_root_source_contract {
    use super::*;
    use d2_rs_core::{Node, NodeLabelState, Size};

    #[test]
    fn parent_placeholder_layout_preserves_completed_sequence_root_state() {
        // Go d2sequence.Layout commits top-center, then
        // d2layouts.InjectNested restores the root attributes after core layout.
        // Exercise both visible roots and roots hidden beneath an extracted grid.
        for (hidden, populated) in [(false, true), (true, true), (false, false)] {
            let mut graph = Graph::default();
            let grid = graph.add_node(Node {
                grid_columns: Some(1),
                ..Node::new(
                    "grid",
                    Size {
                        width: 200.0,
                        height: 300.0,
                    },
                )
            });
            let sequence = graph.add_node(Node {
                parent: hidden.then_some(grid),
                label_position: LabelPosition::InsideBottomCenter,
                ..Node::new(
                    "sequence",
                    Size {
                        width: 100.0,
                        height: 200.0,
                    },
                )
            });
            graph.set_sequence_diagram(sequence, true);
            if populated {
                graph.add_node(Node {
                    parent: Some(sequence),
                    ..Node::new(
                        "sequence.actor",
                        Size {
                            width: 40.0,
                            height: 30.0,
                        },
                    )
                });
            }
            let options = LayoutOptions::default();
            let mut nested = empty_layout_result(&options);
            for (id, node) in graph.nodes() {
                nested.boxes.insert(
                    id,
                    Rect {
                        origin: Point::default(),
                        size: node.size,
                    },
                );
            }
            let committed = NodeLabelState {
                font_size_committed: false,
                size: Some(Size {
                    width: 72.0,
                    height: 36.0,
                }),
                font_size: Some(28),
                position: LabelPosition::InsideTopCenter,
                icon_position: Some(LabelPosition::InsideTopRight),
            };
            nested.node_labels.insert(sequence, committed);
            let output =
                collapse_nested_grid_scopes(&graph, nested, &options, |projected, options| {
                    let mut parent = empty_layout_result(options);
                    for (id, node) in projected.nodes() {
                        parent.boxes.insert(
                            id,
                            Rect {
                                origin: Point { x: 500.0, y: 600.0 },
                                size: node.size,
                            },
                        );
                        parent.node_labels.insert(
                            id,
                            NodeLabelState {
                                position: LabelPosition::InsideMiddleCenter,
                                ..Default::default()
                            },
                        );
                    }
                    Ok(parent)
                })
                .unwrap();
            if populated {
                assert_eq!(output.node_labels[&sequence], committed, "hidden={hidden}");
            } else {
                assert_eq!(
                    output.node_labels[&sequence].position,
                    LabelPosition::InsideMiddleCenter
                );
            }
            assert_eq!(output.boxes[&sequence].origin, Point { x: 500.0, y: 600.0 });
            assert_eq!(
                output.boxes[&sequence].size,
                Size {
                    width: 100.0,
                    height: 200.0
                }
            );
        }
    }
}

#[cfg(test)]
mod synthetic_root_tests;
