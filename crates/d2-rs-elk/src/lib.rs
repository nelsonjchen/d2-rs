// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! ELK layout for d2-rs: a Rust port of upstream D2's ELK adapter
//! (`d2layouts/d2elklayout`) and the `elk-go` engine it calls, producing the
//! same layouts as Go D2.
//!
//! The `diagnostic-traces` feature enables layout traces; `oracle-fixtures`
//! compiles tests that read regenerated elk-go and Go-oracle artifacts.

#![warn(missing_docs)]

mod bend_index;
mod bends;
mod box_layout;
pub mod codec;
mod fixed_layout;
mod force_layout;
mod go_math;
mod go_trig;
mod js_trig;
pub mod layered;
mod loop_cycles;
mod margins;
pub mod model;
#[cfg(test)]
mod model_order_contract;
mod mrtree;
mod orthogonal;
mod property;
mod radial_layout;
mod random_layout;
mod rectpacking;
mod shape_trace;
mod spore;
#[cfg(feature = "diagnostic-traces")]
pub mod trace;
mod utility;

pub use codec::{
    apply_elk_graph_result, graph_to_elk_graph, graph_to_elk_graph_with_table_columns,
};
pub use layered::{
    BkAlignment, BkFixedAlignment, BkHorizontalDirection, BkVerticalDirection, GraphPadding, LEdge,
    LGraph, LGraphId, LNode, LNodeId, LPort, LPortId, Layer, LayerDirection, LayerId, LayeredArena,
    LayeredImportError, LayeredNodeType, LayeringError, NodeAlignment, PortConstraints, PortSide,
    apply_bk_inner_shifts, assign_longest_path_layers, assign_network_simplex_layers,
    break_cycles_greedy_model_order, build_bk_alignment, calculate_innermost_node_margins,
    calculate_layer_sizes_and_graph_height, choose_bk_layout, choose_bk_layout_with_alignment,
    compact_bk_alignment, count_layer_crossings, edge_route_points, join_long_edges, layer_nodes,
    mark_bk_conflicts, materialize_implicit_ports, order_layers_by_barycenter,
    order_layers_by_barycenter_with_model_order, pack_disconnected_components, pack_nested_graphs,
    place_layers_bk, place_layers_bk_with_alignment, place_layers_simple,
    postprocess_north_south_ports, preprocess_north_south_ports, refresh_implicit_port_positions,
    route_orthogonal_edges, route_polyline_edges, sort_layers_by_input_model_order,
    split_deep_cross_hierarchy_edges, split_long_edges,
};
pub use model::{ElkEdge, ElkEdgeSection, ElkGraph, ElkId, ElkLabel, ElkNode, ElkPoint, ElkPort};

#[cfg(feature = "diagnostic-traces")]
macro_rules! trace_phase {
    ($phase:expr, $operation:expr) => {
        crate::trace::emit_marker($phase, concat!($operation, ".begin"));
    };
}

#[cfg(feature = "diagnostic-traces")]
macro_rules! trace_arena {
    ($phase:expr, $operation:expr, $arena:expr) => {
        crate::trace::emit_arena($phase, $operation, $arena);
    };
}

#[cfg(not(feature = "diagnostic-traces"))]
macro_rules! trace_arena {
    ($phase:expr, $operation:expr, $arena:expr) => {};
}

#[cfg(not(feature = "diagnostic-traces"))]
macro_rules! trace_phase {
    ($phase:expr, $operation:expr) => {};
}

#[cfg(feature = "diagnostic-traces")]
macro_rules! trace_rank_phase {
    ($phase:expr, $arena:expr, $graph:expr) => {{
        let (operation, marker) = match $arena.graphs[$graph as usize].layering_strategy {
            layered::LayeringStrategy::CoffmanGraham => ("coffman_graham", "coffman_graham.begin"),
            layered::LayeringStrategy::NetworkSimplex => {
                ("network_simplex", "network_simplex.begin")
            }
        };
        crate::trace::emit_marker($phase, marker);
        crate::trace::emit_arena($phase, operation, $arena);
    }};
}
#[cfg(not(feature = "diagnostic-traces"))]
macro_rules! trace_rank_phase {
    ($phase:expr, $arena:expr, $graph:expr) => {};
}

use layered::{LEdgeId, owner_in_graph};

use d2_rs_core::{
    CandidateReport, Direction, EdgeId, Graph, LabelPosition, LayoutError, LayoutOptions,
    LayoutReport, LayoutResult, NodeLabelState, Point, Score, ShapeKind,
};
use std::collections::{BTreeMap, BTreeSet};

const DEFAULT_ELK_NODE_NODE_SPACING: f64 = 20.0;

/// ELK algorithm names accepted by D2 v0.9.0's configurable layout boundary.
///
/// `Layered` is the normal D2 path. SIMPLE Box, Fixed, Random and default Rectpacking
/// use source-shaped wire providers. Remaining elk-go algorithm families and
/// advanced modes are translated and qualified separately.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Default)]
pub enum ElkAlgorithm {
    /// ELK's layered algorithm, used by D2 by default.
    #[default]
    Layered,
    /// Preserve input positions.
    Fixed,
    /// Pack nodes using source SIMPLE Box area ordering and row placement.
    Box,
    /// Pack nodes through source greedy width estimation and compaction.
    Rectpacking,
    /// Random placement and routes using the source Java RNG and seed policy.
    Random,
    /// Source particle-force layout (Fruchterman-Reingold or Eades).
    Force,
    /// Source stress majorization, initialized by the native force provider.
    Stress,
    /// Source radial tree layout, compaction and rotation.
    Radial,
    /// Source Mr. Tree layout, ordering, compaction and routes.
    MrTree,
    /// SPoRE overlap removal using Delaunay and spanning trees.
    SporeOverlap,
    /// SPoRE compaction by shrinking a spanning tree.
    SporeCompaction,
}

impl ElkAlgorithm {
    /// Parses an ELK algorithm spelling.
    pub fn parse(value: &str) -> Result<Self, LayoutError> {
        // elk-go AlgorithmRegistry.BySuffix: exact IDs or unique dot-boundary
        // suffixes. Option spelling is neither trimmed nor case-folded.
        for (id, algorithm) in [
            ("org.eclipse.elk.layered", Self::Layered),
            ("org.eclipse.elk.fixed", Self::Fixed),
            ("org.eclipse.elk.box", Self::Box),
            ("org.eclipse.elk.rectpacking", Self::Rectpacking),
            ("org.eclipse.elk.random", Self::Random),
            ("org.eclipse.elk.force", Self::Force),
            ("org.eclipse.elk.stress", Self::Stress),
            ("org.eclipse.elk.radial", Self::Radial),
            ("org.eclipse.elk.mrtree", Self::MrTree),
            ("org.eclipse.elk.sporeOverlap", Self::SporeOverlap),
            ("org.eclipse.elk.sporeCompaction", Self::SporeCompaction),
        ] {
            if id == value
                || id
                    .strip_suffix(value)
                    .is_some_and(|prefix| prefix.ends_with('.'))
                    && !value.is_empty()
            {
                return Ok(algorithm);
            }
        }
        Err(LayoutError::ElkAlgorithmNotFound {
            algorithm: value.to_owned(),
            path: String::new(),
        })
    }
}

/// D2's configurable ELK adapter options.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ElkOptions {
    /// Algorithm selected by the caller.
    pub algorithm: ElkAlgorithm,
    /// Spacing between adjacent layers or packed nodes.
    pub node_spacing: f64,
    /// Spacing reserved between edges and nodes.
    pub edge_node_spacing: f64,
    /// Spacing between edges routed between the same pair of layers
    /// (d2elklayout ConfigurableOpts.EdgeEdgeSpacing, default 50).
    pub edge_edge_spacing: f64,
    /// Padding around a packed graph.
    pub padding: f64,
    /// Spacing around self-loop routing.
    pub self_loop_spacing: f64,
}

impl Default for ElkOptions {
    fn default() -> Self {
        Self {
            algorithm: ElkAlgorithm::Layered,
            node_spacing: 70.0,
            edge_node_spacing: 40.0,
            edge_edge_spacing: 50.0,
            padding: 50.0,
            self_loop_spacing: 50.0,
        }
    }
}

/// Runs the currently translated layered phases directly on an ELK graph.
///
/// This is the source-shaped integration boundary used by trace tooling. It
/// keeps the public D2 adapter unchanged while the complete elk-go processor
/// chain is migrated: cycle breaking, layering, ordering, BK placement,
/// long-edge processing, polyline routing, and section serialization.
pub fn layout_elk_graph_with_layered_pipeline(
    graph: &mut ElkGraph,
    options: ElkOptions,
) -> Result<(), LayeringError> {
    let mut bindings = model::EdgeBindings::take(graph);
    layout_elk_graph_with_edge_bindings(bindings.graph, &mut bindings.edges, options)
}

/// Runs the source Random provider on an ELK wire graph.
/// Nonzero `elk.randomSeed` is reproducible; zero uses source-style clock entropy.
/// Full raw import/serialization and expert nested profiles are qualified separately.
pub fn layout_elk_graph_with_random_pipeline(
    graph: &mut ElkGraph,
    options: ElkOptions,
) -> Result<(), LayeringError> {
    layout_utility_wire(graph, options, random_layout::layout)
}
/// Runs the source Force provider, retaining owned-edge carriers.
pub fn layout_elk_graph_with_force_pipeline(
    graph: &mut ElkGraph,
    options: ElkOptions,
) -> Result<(), LayeringError> {
    layout_utility_wire(graph, options, |g, o| force_layout::layout(g, o, false))
}
/// Runs source Stress initialization and localized majorization natively.
pub fn layout_elk_graph_with_stress_pipeline(
    graph: &mut ElkGraph,
    options: ElkOptions,
) -> Result<(), LayeringError> {
    layout_utility_wire(graph, options, |g, o| force_layout::layout(g, o, true))
}
/// Runs native SPoRE overlap removal with source tree growth.
pub fn layout_elk_graph_with_spore_overlap_pipeline(
    graph: &mut ElkGraph,
    options: ElkOptions,
) -> Result<(), LayeringError> {
    layout_utility_wire(graph, options, |g, o| spore::layout(g, o, true))
}
/// Runs native SPoRE compaction with source subtree translation.
pub fn layout_elk_graph_with_spore_compaction_pipeline(
    graph: &mut ElkGraph,
    options: ElkOptions,
) -> Result<(), LayeringError> {
    layout_utility_wire(graph, options, |g, o| spore::layout(g, o, false))
}
/// Runs the native source Mr. Tree provider with cycle-edge routing.
pub fn layout_elk_graph_with_mrtree_pipeline(
    graph: &mut ElkGraph,
    options: ElkOptions,
) -> Result<(), LayeringError> {
    layout_utility_wire(graph, options, mrtree::layout)
}
/// Runs the source Radial provider with native wedge placement and compactors.
pub fn layout_elk_graph_with_radial_pipeline(
    graph: &mut ElkGraph,
    options: ElkOptions,
) -> Result<(), LayeringError> {
    layout_utility_wire(graph, options, radial_layout::layout)
}
fn layout_utility_wire(
    graph: &mut ElkGraph,
    options: ElkOptions,
    execute: fn(&mut ElkGraph, ElkOptions) -> Result<(), LayeringError>,
) -> Result<(), LayeringError> {
    // Codec.importSections infers shapes only for one ordinary section whose
    // two shape references are both absent. Newly generated sections differ.
    fn imported_sections(edges: &mut [ElkEdge]) {
        for edge in edges {
            if edge.sources.len() == 1 && edge.targets.len() == 1 && edge.sections.len() == 1 {
                let s = &mut edge.sections[0];
                if s.incoming_shape.is_none() && s.outgoing_shape.is_none() {
                    s.incoming_shape = Some(edge.sources[0].clone());
                    s.outgoing_shape = Some(edge.targets[0].clone());
                }
            }
        }
    }
    fn nested_sections(nodes: &mut [ElkNode]) {
        for n in nodes {
            imported_sections(&mut n.edges);
            nested_sections(&mut n.children)
        }
    }
    imported_sections(&mut graph.edges);
    nested_sections(&mut graph.children);
    random_layout::validate_endpoints(graph)?;
    if utility::flag(&graph.layout_options, "noLayout") {
        graph.x = Some(graph.x.unwrap_or(0.0));
        graph.y = Some(graph.y.unwrap_or(0.0));
        graph.width = Some(graph.width.unwrap_or(0.0));
        graph.height = Some(graph.height.unwrap_or(0.0));
        let mut bindings = model::EdgeBindings::take(graph);
        let containers = edge_container_ids(bindings.graph, &bindings.edges);
        for (edge, container) in bindings.edges.iter_mut().zip(containers) {
            edge.container = container;
        }
        return Ok(());
    }
    let mut bindings = model::EdgeBindings::take(graph);
    // Codec never imports raw JSON junctionPoints into core.JunctionPoints.
    // Providers translate only generated junctions; untouched raw fields pass
    // through the output clone when the provider does not replace them.
    let raw_junctions: Vec<_> = bindings
        .edges
        .iter_mut()
        .map(|edge| std::mem::take(&mut edge.junction_points))
        .collect();
    std::mem::swap(&mut bindings.graph.edges, &mut bindings.edges);
    let result = execute(bindings.graph, options);
    std::mem::swap(&mut bindings.graph.edges, &mut bindings.edges);
    for (edge, raw) in bindings.edges.iter_mut().zip(raw_junctions) {
        if edge.junction_points.is_empty() {
            edge.junction_points = raw;
        }
    }
    let containers = edge_container_ids(bindings.graph, &bindings.edges);
    for (edge, container) in bindings.edges.iter_mut().zip(containers) {
        edge.container = container;
    }
    result
}

fn layout_elk_graph_with_edge_bindings(
    graph: &mut ElkGraph,
    wire_edges: &mut [ElkEdge],
    options: ElkOptions,
) -> Result<(), LayeringError> {
    trace_phase!("pipeline", "import");
    // D2 configures ELK's between-layer spacing (70px by default). The
    // cross-axis node-to-node spacing remains ELK's independent 20px default.
    // Keeping these values separate is what lets unequal-height fan-out
    // layers match the Go adapter instead of spreading siblings by 70px.
    let options = hydrate_elk_options(graph, options);
    let direction = graph
        .layout_options
        .get("elk.direction")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("RIGHT")
        .to_ascii_uppercase();
    let vertical_direction = matches!(direction.as_str(), "DOWN" | "UP");
    let (mut arena, root) = LayeredArena::from_elk_graph_edges(graph, wire_edges.iter())
        .map_err(|error| LayeringError::Import(error.to_string()))?;
    // core.Engine skips layout when the root has no layout content. Codec
    // still transfers its input shape, without adding layout padding.
    if graph.children.is_empty() && wire_edges.is_empty() {
        graph.x = Some(graph.x.unwrap_or(0.0) + 0.0);
        graph.y = Some(graph.y.unwrap_or(0.0) + 0.0);
        graph.width = Some(graph.width.unwrap_or(0.0));
        graph.height = Some(graph.height.unwrap_or(0.0));
        return Ok(());
    }
    let source_root_padding = arena.graphs[root as usize].padding;
    for (edge_id, edge) in wire_edges.iter().enumerate() {
        if edge.labels.iter().any(|label| {
            label
                .layout_options
                .as_ref()
                .and_then(|options| options.get("elk.edgeLabels.inline"))
                .and_then(serde_json::Value::as_bool)
                == Some(true)
        }) {
            arena.inline_loop_labels.insert(edge_id as LEdgeId);
        }
        let is_self_loop = arena
            .edges
            .get(edge_id)
            .is_some_and(|wire_edge| wire_edge.source == wire_edge.target);
        if !is_self_loop {
            continue;
        }
        let mut width: f64 = 0.0;
        let mut height: f64 = 0.0;
        for label in &edge.labels {
            width = width.max(label.width);
            // Labels.Add stacks original label heights; the vertical
            // processor transposes this aggregate into its horizontal axis.
            height += label.height;
        }
        let label_spacing = arena
            .edges
            .get(edge_id)
            .map(|wire_edge| arena.nodes[wire_edge.source as usize].graph)
            .map(|graph_id| arena.graphs[graph_id as usize].label_label_spacing)
            .unwrap_or(arena.graphs[0].label_label_spacing);
        if edge.labels.len() > 1 {
            height += label_spacing * (edge.labels.len() - 1) as f64;
        }
        if width > 0.0 || height > 0.0 {
            arena
                .loop_label_sizes
                .insert(edge_id as LEdgeId, (width, height));
            arena.loop_label_content_sizes.insert(
                edge_id as LEdgeId,
                (
                    width,
                    height - label_spacing * (edge.labels.len().saturating_sub(1) as f64),
                ),
            );
        }
    }
    if matches!(direction.as_str(), "DOWN" | "UP") {
        transpose_arena_for_vertical(&mut arena);
        if direction == "UP" {
            normalize_up_to_internal(&mut arena);
        }
    } else if direction == "LEFT" {
        normalize_left_to_right(&mut arena);
    }
    materialize_implicit_ports(&mut arena, root)?;
    trace_phase!("import", "materialize_ports");
    trace_arena!("import", "materialize_ports", &arena);
    split_deep_cross_hierarchy_edges(&mut arena, root)?;
    layered::hide_self_loops_for_ordering(&mut arena);
    trace_phase!("self_loops", "hide_for_ordering");
    trace_arena!("self_loops", "hide_for_ordering", &arena);
    trace_phase!("hierarchy", "split_deep_edges");
    trace_arena!("hierarchy", "split_deep_edges", &arena);
    prepare_nested_graphs(&mut arena, root, wire_edges, vertical_direction)?;
    trace_phase!("hierarchy", "prepare_nested_graphs");
    trace_arena!("hierarchy", "prepare_nested_graphs", &arena);
    // Nested sizing can change a compound node's vessel dimensions after its
    // root-facing default ports were synthesized. Re-run the positional part
    // of the importer step so those anchors follow the resized parent.
    let imported_nodes = (0..arena.nodes.len() as LNodeId).collect::<Vec<_>>();
    refresh_implicit_port_positions(&mut arena, &imported_nodes);
    trace_phase!("import", "refresh_ports");
    trace_arena!("import", "refresh_ports", &arena);
    let consider_model_order = graph
        .layout_options
        .get("elk.layered.considerModelOrder.strategy")
        .or_else(|| {
            graph
                .layout_options
                .get("org.eclipse.elk.layered.considerModelOrder.strategy")
        })
        .and_then(serde_json::Value::as_str)
        .is_some_and(|strategy| !strategy.eq_ignore_ascii_case("NONE"));
    let includes_children = graph
        .layout_options
        .get("elk.hierarchyHandling")
        .and_then(serde_json::Value::as_str)
        == Some("INCLUDE_CHILDREN");
    arena.component_sizes.clear();
    arena.component_offsets_y.clear();
    layered::run_flat_component_pipelines(&mut arena, root, |arena, scope| {
        run_root_scope_pipeline(
            arena,
            scope,
            wire_edges,
            options,
            vertical_direction,
            includes_children,
            consider_model_order,
        )
    })?;
    // Hierarchy edges are represented by retired source edges plus active
    // per-scope segments. The dedicated external-component processors have
    // already positioned those scopes; treating the retired records as
    // disconnected here would repack the root siblings a second time.
    let has_hierarchy_edges = arena
        .edges
        .iter()
        .any(|edge| !edge.hierarchy_segments.is_empty())
        || arena.nodes.iter().any(|node| node.nested_graph.is_some());
    // D2's root adapter always requests INCLUDE_CHILDREN. In elk-go that
    // selects the hierarchy pipeline, which lays isolated root nodes in one
    // BK layer rather than sending them through the standalone component
    // placer. Keep the explicit component packer for bare ELK graph callers
    // that omit hierarchy handling, while preserving the D2 source path.
    if !has_hierarchy_edges && !includes_children {
        layered::pack_disconnected_components_for_wire(&mut arena, root)?;
        trace_phase!("components", "plan_disconnected_components_publication");
        trace_arena!(
            "components",
            "plan_disconnected_components_publication",
            &arena
        );
    }

    // The translated processors use the same internal left-to-right frame as
    // elk-go's layered phases. Apply the root direction transform at the wire
    // boundary so the source-shaped entry point preserves D2's DOWN/UP/LEFT
    // coordinates as well as RIGHT. Mirrored directions use the owning
    // compound's local internal width for nested nodes; applying the root
    // extent to every descendant would mirror each child against the wrong
    // frame.
    let axis_min = 0.0;
    let axis_max = publication_axis_width(&arena, root);
    let edge_containers = edge_container_ids(graph, wire_edges);
    let root_padding = source_root_padding.unwrap_or(crate::layered::GraphPadding {
        top: 12.0,
        right: 12.0,
        bottom: 12.0,
        left: 12.0,
    });

    let node_positions = arena
        .nodes
        .iter()
        .enumerate()
        .map(|(node_id, node)| {
            let (local_axis_min, local_axis_max) =
                if let Some(component) = arena.component_transfers.get(&(node_id as LNodeId)) {
                    (0.0, component.axis_width)
                } else if matches!(direction.as_str(), "UP" | "LEFT") {
                    arena
                        .graphs
                        .get(node.graph as usize)
                        .and_then(|scope| scope.parent_node)
                        .map(|parent| (0.0, arena.nodes[parent as usize].width))
                        .unwrap_or((axis_min, axis_max))
                } else {
                    (axis_min, axis_max)
                };
            let (mut x, mut y, width, height) = transform_node_geometry(
                node.position_x,
                node.position_y,
                node.width,
                node.height,
                &direction,
                local_axis_min,
                local_axis_max,
            );
            let scope_offset =
                publication_offset(&arena, node.graph, root, &direction, root_padding);
            if let Some(&(internal_y, transfer_y)) = arena
                .nested_transfer_y
                .get(&(node_id as crate::layered::LNodeId))
            {
                // The upstream transferrer first combines Graph.Offset with
                // graph padding, then adds that combined offset to the
                // node's unpadded coordinate. Reproduce that operation order
                // instead of adding padding during graph resizing and the
                // alignment offset later; the distinction is observable in
                // exact IEEE-754 coordinates.
                match direction.as_str() {
                    "DOWN" | "UP" => x = internal_y + (transfer_y + scope_offset.x),
                    _ => y = internal_y + (transfer_y + scope_offset.y),
                }
            } else {
                if let Some(component) = arena.component_transfers.get(&(node_id as LNodeId)) {
                    x += component.offset.x;
                    y += component.offset.y;
                }
                x += scope_offset.x;
                y += scope_offset.y;
            }
            (node.source_id.clone(), (x, y, width, height))
        })
        .collect::<BTreeMap<_, _>>();
    for dummy in &arena.edge_label_dummies {
        let source_id = &arena.nodes[dummy.node as usize].source_id;
        let Some(&(x, y, width, _height)) = node_positions.get(source_id) else {
            continue;
        };
        let label_label_spacing =
            arena.graphs[arena.nodes[dummy.node as usize].graph as usize].label_label_spacing;
        let Some(edge) = wire_edges.get_mut(dummy.edge as usize) else {
            continue;
        };
        if let Some(component) = arena.component_transfers.get(&dummy.node) {
            // LabelDummyRemover places individual boxes in the local frame.
            // Component movement and root padding happen only afterwards.
            let node = &arena.nodes[dummy.node as usize];
            let scope = &arena.graphs[node.graph as usize];
            let vertical = matches!(direction.as_str(), "DOWN" | "UP");
            let below = dummy.labels_below && !dummy.inline;
            let gap = arena.edge_thickness_for_wire(dummy.edge) + scope.edge_label_spacing;
            let mut x = node.position_x;
            let mut y = node.position_y;
            if below {
                y += gap;
            }
            let space_y = if dummy.inline {
                node.height
            } else {
                node.height - gap
            };
            let indices = if direction == "UP" {
                dummy
                    .label_indices
                    .iter()
                    .copied()
                    .rev()
                    .collect::<Vec<_>>()
            } else {
                dummy.label_indices.clone()
            };
            for index in indices {
                let label = &mut edge.labels[index];
                let (label_width, label_height) = if vertical {
                    (label.height, label.width)
                } else {
                    (label.width, label.height)
                };
                let (local_x, local_y) = if vertical {
                    let local_y = if dummy.inline {
                        y + (space_y - label_height) / 2.0
                    } else if below {
                        y
                    } else {
                        y + space_y - label_height
                    };
                    (x, local_y)
                } else {
                    (x + (node.width - label_width) / 2.0, y)
                };
                let (mut px, mut py, _, _) = transform_node_geometry(
                    local_x,
                    local_y,
                    label_width,
                    label_height,
                    &direction,
                    0.0,
                    component.axis_width,
                );
                px += component.offset.x;
                py += component.offset.y;
                let offset = publication_offset(&arena, node.graph, root, &direction, root_padding);
                label.x = px + offset.x;
                label.y = py + offset.y;
                if vertical {
                    x += label_width + label_label_spacing;
                } else {
                    y += label_height + label_label_spacing;
                }
            }
            continue;
        }
        if dummy.inline {
            // UP's source reversal before final graph mirroring restores
            // original label order in the published flow-axis coordinates.
            let label_indices = dummy.label_indices.iter();
            let mut label_y = y;
            for label_index in label_indices {
                let Some(label) = edge.labels.get_mut(*label_index) else {
                    continue;
                };
                label.x = x + (width - label.width) / 2.0;
                label.y = label_y;
                label_y += label.height + label_label_spacing;
            }
        } else {
            let scope = &arena.graphs[arena.nodes[dummy.node as usize].graph as usize];
            let thickness = arena.edge_thickness_for_wire(dummy.edge);
            let gap = thickness + scope.edge_label_spacing;
            let vertical = matches!(direction.as_str(), "DOWN" | "UP");
            let mut along = 0.0;
            let indices = dummy.label_indices.iter();
            for label_index in indices {
                let Some(label) = edge.labels.get_mut(*label_index) else {
                    continue;
                };
                if vertical {
                    label.x = if dummy.labels_below {
                        x + gap
                    } else {
                        x + width - gap - label.width
                    };
                    label.y = y + along;
                    along += label.height + label_label_spacing;
                } else {
                    label.x = x + (width - label.width) / 2.0;
                    label.y = y + if dummy.labels_below { gap } else { 0.0 } + along;
                    along += label.height + label_label_spacing;
                }
            }
        }
    }

    for dummy in &arena.edge_label_dummies {
        let original = &arena.edges[dummy.edge as usize];
        if original.active || original.hierarchy_segments.is_empty() {
            continue;
        }
        let Some(edge) = wire_edges.get_mut(dummy.edge as usize) else {
            continue;
        };
        let node = &arena.nodes[dummy.node as usize];
        let graph = &arena.graphs[node.graph as usize];
        let gap = arena.edge_thickness_for_wire(dummy.edge) + graph.edge_label_spacing;
        let below = dummy.labels_below && !dummy.inline;
        let mut y = layered::source_node_y(&arena, dummy.node);
        if below {
            y += gap;
        }
        let space_y = if dummy.inline {
            node.height
        } else {
            node.height - gap
        };
        let vertical = matches!(direction.as_str(), "DOWN" | "UP");
        let reference = layered::hierarchy_reference_graph(&arena, original);
        let offset = layered::hierarchy_edge_offset_y(&arena, original);
        for index in &dummy.label_indices {
            let label = &mut edge.labels[*index];
            let mut ordinate = if vertical {
                if dummy.inline {
                    y + (space_y - label.width) / 2.0
                } else if below {
                    y
                } else {
                    y + space_y - label.width
                }
            } else {
                y
            };
            ordinate = layered::change_graph_y(&arena, ordinate, node.graph, reference);
            ordinate += offset;
            if vertical {
                label.x = ordinate;
            } else {
                label.y = ordinate;
            }
            if !vertical {
                y += label.height + graph.label_label_spacing;
            }
        }
    }

    let port_positions = arena
        .ports
        .iter()
        .filter_map(|port| {
            let owner = &arena.nodes[port.node as usize];
            let (x, y, width, height) = transform_port_geometry(
                port.x,
                port.y,
                port.width,
                port.height,
                owner.width,
                &direction,
            );
            port.source_id
                .as_ref()
                .map(|source_id| (source_id.clone(), (x, y, width, height)))
        })
        .collect::<BTreeMap<_, _>>();
    fn publish_nodes(
        nodes: &mut [crate::model::ElkNode],
        positions: &BTreeMap<String, (f64, f64, f64, f64)>,
    ) {
        for node in nodes {
            if let Some(&(x, y, width, height)) = positions.get(&node.id) {
                node.x = x;
                node.y = y;
                node.width = width;
                node.height = height;
            }
            publish_nodes(&mut node.children, positions);
        }
    }
    publish_nodes(&mut graph.children, &node_positions);
    fn publish_ports(
        nodes: &mut [crate::model::ElkNode],
        positions: &BTreeMap<String, (f64, f64, f64, f64)>,
    ) {
        for node in nodes {
            for port in &mut node.ports {
                if let Some(&(x, y, width, height)) = positions.get(&port.id) {
                    port.x = x;
                    port.y = y;
                    port.width = width;
                    port.height = height;
                }
            }
            publish_ports(&mut node.children, positions);
        }
    }
    publish_ports(&mut graph.children, &port_positions);
    let root_size = &arena.graphs[root as usize];
    let (width, height) = if matches!(direction.as_str(), "DOWN" | "UP") {
        (
            root_size.height + root_padding.left + root_padding.right,
            root_size.width + root_padding.top + root_padding.bottom,
        )
    } else {
        (
            root_size.width + root_padding.left + root_padding.right,
            root_size.height + root_padding.top + root_padding.bottom,
        )
    };
    // Codec.transferShape publishes the original root location plus the
    // zero root offset. Keep that addition: -0 + +0 publishes positive zero.
    graph.x = Some(graph.x.unwrap_or(0.0) + 0.0);
    graph.y = Some(graph.y.unwrap_or(0.0) + 0.0);
    // Original transferrer.applyParentNodeLayout calls ElkUtil.ResizeNode:
    // root MINIMUM_SIZE constraints affect its published dimensions even
    // though the placement arena's own content bounds remain unchanged.
    if !property::flag(&graph.layout_options, "nodeSize.fixedGraphSize", false) {
        let (min_width, min_height) = utility::minimum(&graph.layout_options);
        graph.width = Some(width.max(min_width));
        graph.height = Some(height.max(min_height));
    } else {
        graph.width = Some(graph.width.unwrap_or(0.0));
        graph.height = Some(graph.height.unwrap_or(0.0));
    }
    publish_self_loop_labels(&arena, wire_edges, &direction, root, root_padding)?;
    trace_phase!("publish", "labels");
    for (index, edge) in wire_edges.iter_mut().enumerate() {
        let edge_id = index as crate::layered::LEdgeId;
        let is_root_edge = edge_containers
            .get(index)
            .is_some_and(|container| container == &graph.id);
        let routed = &arena.edges[index];
        let source_cross_axis = !routed.active && !routed.hierarchy_segments.is_empty();
        let component = arena.component_transfers.get(&routed.source);
        let route_axis_max = if let Some(component) = component {
            component.axis_width
        } else if !is_root_edge && matches!(direction.as_str(), "UP" | "LEFT") {
            edge_containers
                .get(index)
                .and_then(|container| node_positions.get(container))
                .map(|(_, _, width, height)| if direction == "UP" { *height } else { *width })
                .unwrap_or(axis_max)
        } else {
            axis_max
        };
        let mut junctions = layered::edge_junction_points_for_publication(&arena, edge_id)?
            .into_iter()
            .map(|point| transform_route_point(point, &direction, axis_min, route_axis_max))
            .collect::<Vec<_>>();
        if routed.active
            && arena.nodes[routed.source as usize].graph
                == arena.nodes[routed.target as usize].graph
        {
            let scope = arena.nodes[routed.source as usize].graph;
            if scope != root {
                let offset = graph_offset_in_output_coordinates(
                    arena.graphs[scope as usize].offset_y,
                    &direction,
                );
                for point in &mut junctions {
                    point.x += offset.x;
                    point.y += offset.y;
                }
            }
            if let Some((local, shift)) = arena.nested_junction_transfer_y.get(&edge_id) {
                assert_eq!(
                    junctions.len(),
                    local.len(),
                    "complete junction transfer inventory"
                );
                let offset = arena.graphs[scope as usize].offset_y;
                for (point, ordinate) in junctions.iter_mut().zip(local) {
                    let value = *ordinate + (*shift + offset);
                    if matches!(direction.as_str(), "DOWN" | "UP") {
                        point.x = value;
                    } else {
                        point.y = value;
                    }
                }
            }
        }
        if let Some(component) = component {
            for point in &mut junctions {
                point.x += component.offset.x;
                point.y += component.offset.y;
            }
        }
        if is_root_edge {
            let offset = publication_offset(&arena, root, root, &direction, root_padding);
            for point in &mut junctions {
                if !source_cross_axis || !matches!(direction.as_str(), "DOWN" | "UP") {
                    point.x += offset.x;
                }
                if !source_cross_axis || matches!(direction.as_str(), "DOWN" | "UP") {
                    point.y += offset.y;
                }
            }
        }
        edge.junction_points = junctions
            .into_iter()
            .map(|point| ElkPoint {
                x: point.x,
                y: point.y,
            })
            .collect();
        let mut points = layered::edge_route_points_for_publication(&arena, edge_id)?
            .into_iter()
            .map(|point| transform_route_point(point, &direction, axis_min, route_axis_max))
            .collect::<Vec<_>>();
        if points.len() < 2 {
            edge.sections.clear();
            continue;
        }
        // Upstream mirrors node and port coordinates independently, then
        // computes getAbsoluteAnchor. Reflecting their sum changes IEEE-754
        // rounding. Preserve the port-local operation order for root loops.
        if routed.active
            && routed.source == routed.target
            && matches!(direction.as_str(), "LEFT" | "UP")
        {
            let owner = &arena.nodes[routed.source as usize];
            if let Some(&(internal, offset)) = arena.nested_transfer_x.get(&routed.source) {
                let mut minimum = f64::INFINITY;
                let mut maximum = f64::NEG_INFINITY;
                for (id, node) in arena
                    .nodes
                    .iter()
                    .enumerate()
                    .filter(|(_, node)| node.graph == owner.graph)
                {
                    if let Some(&(x, _)) = arena.nested_transfer_x.get(&(id as LNodeId)) {
                        minimum = minimum.min(x - node.margin_left);
                        maximum = maximum.max(x + node.width + node.margin_right);
                    }
                }
                let mirror = minimum + maximum;
                points = layered::edge_route_points_transfer(&arena, edge_id, false)?
                    .into_iter()
                    .map(|point| {
                        let reflected = (mirror - point.x) + offset;
                        if direction == "UP" {
                            Point {
                                x: point.y,
                                y: reflected,
                            }
                        } else {
                            Point {
                                x: reflected,
                                y: point.y,
                            }
                        }
                    })
                    .collect();
                let last = points.len() - 1;
                for (index, port) in [(0, routed.source_port), (last, routed.target_port)] {
                    if let Some(port) = port {
                        let port = &arena.ports[port as usize];
                        let coordinate = ((mirror - owner.width - internal)
                            + (owner.width - port.width - port.x))
                            + offset;
                        if direction == "UP" {
                            points[index].y = coordinate;
                        } else {
                            points[index].x = coordinate;
                        }
                    }
                }
            }
        }
        if routed.active
            && routed.source == routed.target
            && arena.nodes[routed.source as usize].graph == root
            && component.is_none()
            && matches!(direction.as_str(), "LEFT" | "UP")
        {
            let owner = &arena.nodes[routed.source as usize];
            let node_x = axis_min + axis_max - owner.width - owner.position_x;
            let last = points.len() - 1;
            for (point_index, port_id) in [(0, routed.source_port), (last, routed.target_port)] {
                if let Some(port_id) = port_id {
                    let port = &arena.ports[port_id as usize];
                    let anchor_x = node_x + (owner.width - port.width - port.x);
                    if direction == "LEFT" {
                        points[point_index].x = anchor_x;
                    } else {
                        points[point_index].y = anchor_x;
                    }
                }
            }
        }
        // Transferrer.edgeLayoutOffsets applies Graph.Offset to the final
        // section. Mirrored loop reconstruction above replaces the point
        // vector, so apply this offset after that reconstruction as well.
        if routed.active
            && arena.nodes[routed.source as usize].graph
                == arena.nodes[routed.target as usize].graph
            && arena.nodes[routed.source as usize].graph != root
        {
            let graph_id = arena.nodes[routed.source as usize].graph;
            let scope_offset = graph_offset_in_output_coordinates(
                arena.graphs[graph_id as usize].offset_y,
                &direction,
            );
            for point in &mut points {
                point.x += scope_offset.x;
                point.y += scope_offset.y;
            }
        }
        // Transferrer combines graph offset and padding before adding the
        // unpadded ordinate. Preserve that association: adding padding to each
        // point first changes f64 bits for fractional nested port coordinates.
        if routed.active
            && routed.source != routed.target
            && let Some((local_bends, shift)) = arena.nested_edge_transfer_y.get(&edge_id)
        {
            assert_eq!(
                points.len(),
                local_bends.len() + 2,
                "nested edge transfer must retain a complete route inventory"
            );
            let graph_id = arena.nodes[routed.source as usize].graph;
            let offset = arena.graphs[graph_id as usize].offset_y;
            let normal_axis = |point: &mut Point, value: f64| {
                if matches!(direction.as_str(), "DOWN" | "UP") {
                    point.x = value;
                } else {
                    point.y = value;
                }
            };
            let last = points.len() - 1;
            for (index, node_id, port) in [
                (0, routed.source, routed.source_port),
                (last, routed.target, routed.target_port),
            ] {
                let (internal, transfer) = arena.nested_transfer_y[&node_id];
                let port_y = port
                    .map(|p| arena.ports[p as usize].y)
                    .unwrap_or(arena.nodes[node_id as usize].height / 2.0);
                normal_axis(
                    &mut points[index],
                    (internal + port_y) + (transfer + offset),
                );
            }
            for (point, local) in points[1..last].iter_mut().zip(local_bends) {
                normal_axis(point, *local + (*shift + offset));
            }
        }
        if let Some(component) = component {
            for point in &mut points {
                point.x += component.offset.x;
                point.y += component.offset.y;
            }
            // offsetGraph moves node positions before the transferrer sums
            // port anchors; preserve that association independently per end.
            let last = points.len() - 1;
            for (index, node_id, port_id) in [
                (0, routed.source, routed.source_port),
                (last, routed.target, routed.target_port),
            ] {
                let node = &arena.nodes[node_id as usize];
                let transfer = arena.component_transfers[&node_id];
                let (mut x, mut y, _, _) = transform_node_geometry(
                    node.position_x,
                    node.position_y,
                    node.width,
                    node.height,
                    &direction,
                    0.0,
                    transfer.axis_width,
                );
                x += transfer.offset.x;
                y += transfer.offset.y;
                if let Some(port) = port_id {
                    let port = &arena.ports[port as usize];
                    let (px, py, _, _) = transform_port_geometry(
                        port.x,
                        port.y,
                        port.width,
                        port.height,
                        node.width,
                        &direction,
                    );
                    x += px;
                    y += py;
                }
                points[index] = Point { x, y };
            }
        }
        // Apply the root graph's publication offset after direction-specific
        // self-loop endpoint correction. LEFT/UP loops rebuild their points
        // above, so applying padding during the first transform would be lost.
        if is_root_edge {
            let offset = publication_offset(&arena, root, root, &direction, root_padding);
            for point in &mut points {
                if !source_cross_axis || !matches!(direction.as_str(), "DOWN" | "UP") {
                    point.x += offset.x;
                }
                if !source_cross_axis || matches!(direction.as_str(), "DOWN" | "UP") {
                    point.y += offset.y;
                }
            }
        }
        let source_shape = edge.sources.first().cloned();
        let target_shape = edge.targets.first().cloned();
        edge.sections = vec![ElkEdgeSection {
            id: Some(format!("{}_s0", edge.id)),
            start: ElkPoint {
                x: points[0].x,
                y: points[0].y,
            },
            end: ElkPoint {
                x: points[points.len() - 1].x,
                y: points[points.len() - 1].y,
            },
            bends: points[1..points.len() - 1]
                .iter()
                .map(|point| ElkPoint {
                    x: point.x,
                    y: point.y,
                })
                .collect(),
            incoming_shape: source_shape,
            outgoing_shape: target_shape,
        }];
        edge.container = edge_containers[index].clone();
    }
    Ok(())
}

/// Returns the ELK scope that owns an edge's section coordinates.
///
/// D2 keeps its edge records on the root wire graph, but elk-go imports each
/// edge into the lowest common scope containing its endpoint nodes. An
/// endpoint node is itself a valid scope when the other endpoint is one of
/// its descendants (for example, a container-to-child edge). Sections are
/// local to that scope, so include each endpoint node in its ancestor chain.
fn edge_container_ids(graph: &ElkGraph, wire_edges: &[ElkEdge]) -> Vec<String> {
    fn collect_scopes(
        nodes: &[crate::model::ElkNode],
        scope_chain: &[String],
        scopes: &mut BTreeMap<String, Vec<String>>,
    ) {
        for node in nodes {
            let mut node_scope_chain = Vec::with_capacity(scope_chain.len() + 1);
            node_scope_chain.push(node.id.clone());
            node_scope_chain.extend(scope_chain.iter().cloned());
            scopes.insert(node.id.clone(), node_scope_chain.clone());
            for port in &node.ports {
                scopes.insert(port.id.clone(), node_scope_chain.clone());
            }
            collect_scopes(&node.children, &node_scope_chain, scopes);
        }
    }

    let mut scopes = BTreeMap::new();
    collect_scopes(
        &graph.children,
        std::slice::from_ref(&graph.id),
        &mut scopes,
    );
    wire_edges
        .iter()
        .map(|edge| {
            let endpoint_chains = edge
                .sources
                .iter()
                .chain(&edge.targets)
                .filter_map(|endpoint| scopes.get(endpoint))
                .collect::<Vec<_>>();
            let Some(first_chain) = endpoint_chains.first() else {
                return graph.id.clone();
            };
            // A node is not itself a graph scope for an edge that starts and
            // ends on that same node. Its section belongs to the containing
            // graph. Keep the node as a scope when the other endpoint is a
            // descendant, such as a container-to-child edge.
            if endpoint_chains
                .iter()
                .all(|chain| chain.first() == first_chain.first())
            {
                return first_chain
                    .get(1)
                    .cloned()
                    .unwrap_or_else(|| graph.id.clone());
            }
            first_chain
                .iter()
                .find(|scope| endpoint_chains.iter().all(|chain| chain.contains(scope)))
                .cloned()
                .unwrap_or_else(|| graph.id.clone())
        })
        .collect()
}

// GraphTransformer mirrors against the routed graph size, including reserved
// lanes. Nested coordinates already contain padding, so use their vessel box.
fn publication_axis_width(arena: &LayeredArena, scope: LGraphId) -> f64 {
    let graph = &arena.graphs[scope as usize];
    if let Some(parent) = graph.parent_node {
        return arena.nodes[parent as usize].width;
    }
    if graph.width != 0.0 {
        return graph.width;
    }
    arena
        .nodes
        .iter()
        .filter(|node| node.graph == scope)
        .map(|node| node.position_x + node.width + node.margin_right)
        .fold(0.0, f64::max)
}

// Go retains Graph.Offset until transfer, then adds Offset+Padding once.
// Folding it into nodes before port anchors changes the final floating bits.
fn publication_offset(
    arena: &LayeredArena,
    graph: LGraphId,
    root: LGraphId,
    direction: &str,
    root_padding: crate::layered::GraphPadding,
) -> Point {
    let mut offset =
        graph_offset_in_output_coordinates(arena.graphs[graph as usize].offset_y, direction);
    if graph == root {
        offset.x += root_padding.left;
        offset.y += root_padding.top;
    }
    offset
}

/// Runs the layered pipeline and returns deterministic before/after events.
///
/// This entry point exists only with `diagnostic-traces`; ordinary builds do
/// not expose or construct trace state. Detailed intermediate hooks are being
/// added phase by phase, while the published geometry snapshot already uses
/// stable identities and exact float bits.
#[cfg(feature = "diagnostic-traces")]
pub fn layout_elk_graph_with_layered_pipeline_trace(
    graph: &mut ElkGraph,
    options: ElkOptions,
) -> Result<Vec<trace::TraceEvent>, LayeringError> {
    trace::begin();
    trace::emit_marker("pipeline", "begin");
    trace::emit_graph("input", "snapshot", graph);
    if let Err(error) = layout_elk_graph_with_layered_pipeline(graph, options) {
        let _ = trace::finish();
        return Err(error);
    }
    trace::emit_graph("output", "snapshot", graph);
    trace::emit_marker("pipeline", "end");
    Ok(trace::finish())
}

fn publish_self_loop_labels(
    arena: &LayeredArena,
    wire_edges: &mut [ElkEdge],
    direction: &str,
    root: crate::layered::LGraphId,
    root_padding: crate::layered::GraphPadding,
) -> Result<(), LayeringError> {
    for (edge_index, wire_edge) in wire_edges.iter_mut().enumerate() {
        let Some(edge) = arena.edges.get(edge_index) else {
            continue;
        };
        if !edge.active || edge.source != edge.target || wire_edge.labels.is_empty() {
            continue;
        }
        let node = &arena.nodes[edge.source as usize];
        let packed = arena.component_transfers.contains_key(&edge.source);
        let node_x = if packed { 0.0 } else { node.position_x };
        let node_y = if packed { 0.0 } else { node.position_y };
        let axis_min = 0.0;
        let axis_max = arena.component_transfers.get(&edge.source).map_or_else(
            || publication_axis_width(arena, node.graph),
            |component| component.axis_width,
        );
        let points = if packed {
            layered::self_loop_node_local_points(arena, edge_index as LEdgeId)?
        } else {
            edge_route_points(arena, edge_index as LEdgeId)?
        };
        if arena.inline_loop_labels.contains(&(edge_index as LEdgeId)) {
            let source_side = edge
                .source_port
                .map_or(PortSide::North, |port| arena.ports[port as usize].side);
            let target_side = edge
                .target_port
                .map_or(source_side, |port| arena.ports[port as usize].side);
            let clockwise = layered::self_loop_routing_clockwise(
                arena,
                edge_index as LEdgeId,
                source_side,
                target_side,
            );
            if let Some(label_side) = layered::self_loop_inline_label_side(
                arena,
                edge_index as LEdgeId,
                source_side,
                target_side,
                clockwise,
            ) {
                if let Some(route_coordinate) = self_loop_route_side_coordinate(&points, label_side)
                {
                    let vertical = matches!(direction, "DOWN" | "UP");
                    let label_spacing = arena.graphs[node.graph as usize].label_label_spacing;
                    let mut aggregate_width = 0.0_f64;
                    let mut aggregate_height = 0.0_f64;
                    let mut label_sizes = Vec::with_capacity(wire_edge.labels.len());
                    for label in &wire_edge.labels {
                        let (width, height) = if vertical {
                            (label.height, label.width)
                        } else {
                            (label.width, label.height)
                        };
                        aggregate_width = aggregate_width.max(width);
                        aggregate_height += height;
                        label_sizes.push((width, height));
                    }
                    if label_sizes.len() > 1 {
                        aggregate_height +=
                            label_spacing * (label_sizes.len().saturating_sub(1) as f64);
                    }
                    // elk-go placeRoutedLabels: inline labels straddle their
                    // side's segment, which the router already moved out by
                    // half the label size. A West/East label keeps the zero
                    // Y offset left by LabelCenter alignment (node top).
                    let (aggregate_x, aggregate_y) =
                        if matches!(label_side, PortSide::North | PortSide::South) {
                            (
                                node_x + (node.width - aggregate_width) / 2.0,
                                route_coordinate - aggregate_height / 2.0,
                            )
                        } else {
                            (route_coordinate - aggregate_width / 2.0, node_y)
                        };
                    let mut stack_offset = 0.0;
                    for (label, (label_width, label_height)) in
                        wire_edge.labels.iter_mut().zip(label_sizes)
                    {
                        let x = match label_side {
                            PortSide::East => aggregate_x,
                            PortSide::West => aggregate_x + aggregate_width - label_width,
                            _ => aggregate_x + (aggregate_width - label_width) / 2.0,
                        };
                        let mut point = if packed {
                            let (px, py, _, _) = transform_node_geometry(
                                x + node.position_x,
                                (aggregate_y + stack_offset) + node.position_y,
                                label_width,
                                label_height,
                                direction,
                                axis_min,
                                axis_max,
                            );
                            Point { x: px, y: py }
                        } else {
                            transform_route_point(
                                Point {
                                    x,
                                    y: aggregate_y + stack_offset,
                                },
                                direction,
                                axis_min,
                                axis_max,
                            )
                        };
                        if let Some(component) = arena.component_transfers.get(&edge.source) {
                            point.x += component.offset.x;
                            point.y += component.offset.y;
                        }
                        let offset =
                            publication_offset(arena, node.graph, root, direction, root_padding);
                        point.x += offset.x;
                        point.y += offset.y;
                        label.x = point.x;
                        label.y = point.y;
                        stack_offset += label_height + label_spacing;
                    }
                    continue;
                }
            }
        }
        let loop_y = points.get(1).map_or(node_y, |point| point.y);
        let label_spacing = arena.graphs[node.graph as usize].edge_label_spacing;
        let label_label_spacing = arena.graphs[node.graph as usize].label_label_spacing;
        let aggregate = arena
            .loop_label_sizes
            .get(&(edge_index as LEdgeId))
            .copied()
            .unwrap_or((0.0, 0.0));
        let mut stack_offset = 0.0;
        let mut running_x = None;
        let mut running_y = None;
        for label in &mut wire_edge.labels {
            let (width, height) = if matches!(direction, "DOWN" | "UP") {
                (aggregate.1, aggregate.0)
            } else {
                aggregate
            };
            let mut x = node_x + (node.width - width) / 2.0;
            let side = edge.source_port.map(|port| arena.ports[port as usize].side);
            let mut y = if side == Some(PortSide::South) {
                loop_y + label_spacing
            } else if packed {
                (loop_y - label_spacing) - height
            } else {
                loop_y - height - label_spacing
            };
            let target_side = edge.target_port.map(|port| arena.ports[port as usize].side);
            if side != target_side {
                let source_horizontal = matches!(side, Some(PortSide::North | PortSide::South));
                let target_horizontal =
                    matches!(target_side, Some(PortSide::North | PortSide::South));
                if source_horizontal != target_horizontal {
                    let (anchor, outer, horizontal_side, other_side) = if source_horizontal {
                        (points.first(), points.get(1), side, target_side)
                    } else {
                        (
                            points.last(),
                            points.get(points.len().saturating_sub(2)),
                            target_side,
                            side,
                        )
                    };
                    if let (Some(anchor), Some(outer)) = (anchor, outer) {
                        x = anchor.x
                            - if other_side == Some(PortSide::West) {
                                width
                            } else {
                                0.0
                            };
                        y = if horizontal_side == Some(PortSide::South) {
                            outer.y + label_spacing
                        } else {
                            if packed {
                                (outer.y - label_spacing) - height
                            } else {
                                outer.y - height - label_spacing
                            }
                        };
                    }
                }
            }
            if side == target_side && matches!(side, Some(PortSide::East | PortSide::West)) {
                let route_x = points.get(1).map_or(node_x, |point| point.x);
                x = if side == Some(PortSide::East) {
                    route_x + label_spacing
                } else {
                    route_x - label_spacing - width
                };
                y = points
                    .first()
                    .zip(points.last())
                    .map_or(node_y, |(first, last)| first.y.min(last.y));
            }
            let label_side = layered::self_loop_label_interval(arena, edge_index as LEdgeId)
                .map(|(side, _, _)| side)
                .or(side);
            if matches!(direction, "DOWN" | "UP") {
                if packed {
                    x = running_x.unwrap_or(x);
                } else {
                    x += stack_offset;
                }
                if label_side == Some(PortSide::North) {
                    if packed {
                        y = (y + height) - label.width;
                    } else {
                        y += height - label.width;
                    }
                }
                if packed {
                    running_x = Some(x + (label.height + label_label_spacing));
                }
            } else {
                let align_right =
                    side == Some(PortSide::West) || target_side == Some(PortSide::West);
                let align_left =
                    side == Some(PortSide::East) || target_side == Some(PortSide::East);
                if align_right {
                    x += width - label.width;
                } else if !align_left {
                    x += (width - label.width) / 2.0;
                }
                if packed {
                    y = running_y.unwrap_or(y);
                    running_y = Some(y + (label.height + label_label_spacing));
                } else {
                    y += stack_offset;
                }
            }
            stack_offset += label.height + label_label_spacing;
            let mut point = if packed {
                let (lw, lh) = if matches!(direction, "DOWN" | "UP") {
                    (label.height, label.width)
                } else {
                    (label.width, label.height)
                };
                let (px, py, _, _) = transform_node_geometry(
                    x + node.position_x,
                    y + node.position_y,
                    lw,
                    lh,
                    direction,
                    axis_min,
                    axis_max,
                );
                Point { x: px, y: py }
            } else {
                let mut point =
                    transform_route_point(Point { x, y }, direction, axis_min, axis_max);
                if direction == "LEFT" {
                    point.x -= label.width;
                } else if direction == "UP" {
                    point.y -= label.height;
                }
                point
            };
            if let Some(component) = arena.component_transfers.get(&edge.source) {
                point.x += component.offset.x;
                point.y += component.offset.y;
            }
            let offset = publication_offset(arena, node.graph, root, direction, root_padding);
            label.x = point.x + offset.x;
            label.y = point.y + offset.y;
        }
    }
    Ok(())
}

fn self_loop_route_side_coordinate(points: &[Point], side: PortSide) -> Option<f64> {
    let coordinates = points.windows(2).filter_map(|pair| match side {
        PortSide::North | PortSide::South if pair[0].y == pair[1].y => Some(pair[0].y),
        PortSide::East | PortSide::West if pair[0].x == pair[1].x => Some(pair[0].x),
        _ => None,
    });
    match side {
        PortSide::North => coordinates.min_by(f64::total_cmp),
        PortSide::South => coordinates.max_by(f64::total_cmp),
        PortSide::East => coordinates.max_by(f64::total_cmp),
        PortSide::West => coordinates.min_by(f64::total_cmp),
        PortSide::Undefined => None,
    }
}

/// Applies the source-shaped self-loop placement boundary. ELK's self-loop
/// processors remove loop edges from the rank graph, then reserve their
/// clearance above the owning node before routing the restored north ports.
/// D2 v0.9.0 `d2elklayout/layout.go:673–740` (deconflictSelfLoopLabels):
/// a self-loop label that materially overlaps an earlier one on the same
/// object moves to the first free UNLOCKED_MIDDLE percentage.
fn deconflict_self_loop_labels(graph: &Graph, result: &mut LayoutResult) {
    const MATERIAL_OVERLAP: f64 = 5.0; // label.PADDING
    const CANDIDATES: [f64; 8] = [0.4, 0.6, 0.3, 0.7, 0.2, 0.8, 0.1, 0.9];
    let label_box = |route: &[Point], label: &d2_rs_core::EdgeLabel, position, percentage| {
        d2_rs_core::route_label::top_left(route, position, 0.0, percentage, label.size).map(
            |origin| d2_rs_core::Rect {
                origin,
                size: label.size,
            },
        )
    };
    let mut placed = BTreeMap::<d2_rs_core::NodeId, Vec<d2_rs_core::Rect>>::new();
    for (id, edge) in graph.edges() {
        if edge.source != edge.target {
            continue;
        }
        let (Some(route), Some(label)) = (result.routes.get(&id), result.edge_labels.get(&id))
        else {
            continue;
        };
        if label.text.is_empty() || route.len() < 2 {
            continue;
        }
        let Some(mut current) = label_box(route, label, label.position, label.percentage) else {
            continue;
        };
        let others = placed.entry(edge.source).or_default();
        let material = others.iter().any(|other| {
            let width = (current.right()).min(other.right()) - current.origin.x.max(other.origin.x);
            let height =
                (current.bottom()).min(other.bottom()) - current.origin.y.max(other.origin.y);
            width > MATERIAL_OVERLAP && height > MATERIAL_OVERLAP
        });
        if material {
            for candidate in CANDIDATES {
                let Some(candidate_box) =
                    label_box(route, label, LabelPosition::UnlockedMiddle, candidate)
                else {
                    continue;
                };
                if others.iter().any(|other| candidate_box.overlaps(*other)) {
                    continue;
                }
                let label = result
                    .edge_labels
                    .get_mut(&id)
                    .expect("label checked above");
                label.position = LabelPosition::UnlockedMiddle;
                label.percentage = candidate;
                result.edge_label_percentage_commits.insert(id);
                current = candidate_box;
                break;
            }
        }
        others.push(current);
    }
}

fn reserve_loop_margins(arena: &mut LayeredArena, graph_id: LGraphId) -> Result<(), LayeringError> {
    layered::capture_self_loop_routes(arena, graph_id);
    for edge_id in 0..arena.edges.len() {
        let edge = &arena.edges[edge_id];
        if !edge.active
            || edge.source != edge.target
            || arena.nodes[edge.source as usize].graph != graph_id
        {
            continue;
        }
        let node_id = edge.source;
        let side_label = edge
            .source_port
            .zip(edge.target_port)
            .and_then(|(source, target)| {
                let side = arena.ports[source as usize].side;
                (side == arena.ports[target as usize].side
                    && matches!(side, PortSide::East | PortSide::West))
                .then_some(side)
            })
            .zip(arena.loop_label_sizes.get(&(edge_id as LEdgeId)))
            .map(|(side, (width, height))| {
                let width = if matches!(
                    arena.graphs[0].direction,
                    LayerDirection::Down | LayerDirection::Up
                ) {
                    *height
                } else {
                    *width
                };
                (
                    side,
                    width,
                    arena.graphs[arena.nodes[node_id as usize].graph as usize].edge_label_spacing,
                )
            });
        let horizontal_label_interval =
            layered::self_loop_label_interval(arena, edge_id as LEdgeId);
        let node = &arena.nodes[node_id as usize];
        arena.original_loop_margins.entry(node_id).or_insert([
            node.margin_top,
            node.margin_right,
            node.margin_bottom,
            node.margin_left,
        ]);
        let points = edge_route_points(arena, edge_id as LEdgeId)?;
        let horizontal_label_bounds = layered::self_loop_label_interval(arena, edge_id as LEdgeId)
            .and_then(|(side, _, _)| {
                let (width, height) = arena.loop_label_sizes.get(&(edge_id as LEdgeId))?;
                let height = if matches!(
                    arena.graphs[0].direction,
                    LayerDirection::Down | LayerDirection::Up
                ) {
                    *width
                } else {
                    *height
                };
                let spacing =
                    arena.graphs[arena.nodes[node_id as usize].graph as usize].edge_label_spacing;
                if side == PortSide::North {
                    let baseline = points
                        .iter()
                        .map(|point| point.y)
                        .fold(f64::INFINITY, f64::min);
                    Some((baseline - spacing - height, baseline - spacing))
                } else {
                    let baseline = points
                        .iter()
                        .map(|point| point.y)
                        .fold(f64::NEG_INFINITY, f64::max);
                    Some((baseline + spacing, baseline + spacing + height))
                }
            });
        // Inline labels on a three-sided loop straddle its routed segment.
        // The bend points already include half the label height, while elk-go
        // reserves the other half when updating the node margin.
        let inline_horizontal_label_bounds =
            layered::self_loop_label_side(arena, edge_id as LEdgeId)
                .filter(|side| {
                    matches!(side, PortSide::North | PortSide::South)
                        && arena.inline_loop_labels.contains(&(edge_id as LEdgeId))
                        && layered::self_loop_occupied_sides(arena, edge_id as LEdgeId)
                            .is_some_and(|sides| sides.iter().filter(|side| **side).count() == 3)
                })
                .and_then(|side| {
                    let (width, height) = arena.loop_label_sizes.get(&(edge_id as LEdgeId))?;
                    let height = if matches!(
                        arena.graphs[0].direction,
                        LayerDirection::Down | LayerDirection::Up
                    ) {
                        *width
                    } else {
                        *height
                    };
                    let segment = if side == PortSide::North {
                        points
                            .iter()
                            .map(|point| point.y)
                            .fold(f64::INFINITY, f64::min)
                    } else {
                        points
                            .iter()
                            .map(|point| point.y)
                            .fold(f64::NEG_INFINITY, f64::max)
                    };
                    Some((side, segment, height / 2.0))
                });
        // Inline West/East labels straddle their segment from the node top
        // (OrthogonalSelfLoopRouter.updateMargins over placeRoutedLabels).
        let inline_side_label_bounds = layered::self_loop_label_side(arena, edge_id as LEdgeId)
            .filter(|side| {
                matches!(side, PortSide::West | PortSide::East)
                    && arena.inline_loop_labels.contains(&(edge_id as LEdgeId))
            })
            .and_then(|side| {
                let (width, height) = arena.loop_label_sizes.get(&(edge_id as LEdgeId))?;
                let (width, height) = if matches!(
                    arena.graphs[0].direction,
                    LayerDirection::Down | LayerDirection::Up
                ) {
                    (*height, *width)
                } else {
                    (*width, *height)
                };
                let segment = self_loop_route_side_coordinate(&points, side)?;
                Some((segment - width / 2.0, segment + width / 2.0, height))
            });
        let side_label_bounds = side_label.and_then(|(side, width, spacing)| {
            points.get(1).map(|point| {
                let left = if side == PortSide::West {
                    point.x - spacing - width
                } else {
                    point.x + spacing
                };
                (left, left + width)
            })
        });
        let node = &mut arena.nodes[node_id as usize];
        for point in points {
            node.margin_left = node.margin_left.max(node.position_x - point.x);
            node.margin_right = node
                .margin_right
                .max(point.x - node.position_x - node.width);
            node.margin_top = node.margin_top.max(node.position_y - point.y);
            node.margin_bottom = node
                .margin_bottom
                .max(point.y - node.position_y - node.height);
        }
        // Include both corners of centered and port-aligned labels.
        if let Some((_, left, right)) = horizontal_label_interval {
            node.margin_left = node.margin_left.max(-left);
            node.margin_right = node.margin_right.max(right - node.width);
        }
        if let Some((left, right)) = side_label_bounds {
            node.margin_left = node.margin_left.max(node.position_x - left);
            node.margin_right = node.margin_right.max(right - node.position_x - node.width);
        }
        if let Some((left, right, height)) = inline_side_label_bounds {
            node.margin_left = node.margin_left.max(node.position_x - left);
            node.margin_right = node.margin_right.max(right - node.position_x - node.width);
            node.margin_bottom = node.margin_bottom.max(height - node.height);
        }
        if let Some((side, segment, half_height)) = inline_horizontal_label_bounds {
            if side == PortSide::North {
                node.margin_top = node.margin_top.max(node.position_y - segment + half_height);
            } else {
                node.margin_bottom = node
                    .margin_bottom
                    .max(segment + half_height - node.position_y - node.height);
            }
        }
        if let Some((top, bottom)) = horizontal_label_bounds {
            node.margin_top = node.margin_top.max(node.position_y - top);
            node.margin_bottom = node
                .margin_bottom
                .max(bottom - node.position_y - node.height);
        }
    }
    Ok(())
}

fn align_external_edges(arena: &mut LayeredArena, graph_id: LGraphId) {
    let edges = arena.edges.clone();
    let mut changed = false;
    for edge in edges {
        if !edge.active {
            continue;
        }
        let Some(source_owner) = owner_in_graph(arena, edge.source, graph_id) else {
            continue;
        };
        let Some(target_owner) = owner_in_graph(arena, edge.target, graph_id) else {
            continue;
        };
        if source_owner == edge.source && target_owner == edge.target {
            continue;
        }
        let (Some(source_layer), Some(target_layer)) = (
            arena.nodes[source_owner as usize].layer,
            arena.nodes[target_owner as usize].layer,
        ) else {
            continue;
        };
        let source_index = arena.graphs[graph_id as usize]
            .layers
            .iter()
            .position(|layer| *layer == source_layer);
        let target_index = arena.graphs[graph_id as usize]
            .layers
            .iter()
            .position(|layer| *layer == target_layer);
        let (Some(source_index), Some(target_index)) = (source_index, target_index) else {
            continue;
        };
        if source_index == target_index {
            continue;
        }
        changed = true;
        let source_center = arena.nodes[source_owner as usize].position_y
            + arena.nodes[source_owner as usize].height / 2.0;
        let target_center = arena.nodes[target_owner as usize].position_y
            + arena.nodes[target_owner as usize].height / 2.0;
        if source_index < target_index {
            arena.nodes[target_owner as usize].position_y =
                source_center - arena.nodes[target_owner as usize].height / 2.0;
        } else {
            arena.nodes[source_owner as usize].position_y =
                target_center - arena.nodes[source_owner as usize].height / 2.0;
        }
    }
    if changed {
        let minimum_y = arena
            .nodes
            .iter()
            .filter(|node| node.graph == graph_id)
            .map(|node| node.position_y)
            .fold(f64::INFINITY, f64::min);
        if minimum_y.is_finite() && minimum_y < 0.0 {
            for node in &mut arena.nodes {
                if node.graph == graph_id {
                    node.position_y -= minimum_y;
                }
            }
        }
    }
}

fn hydrate_elk_options(graph: &ElkGraph, mut options: ElkOptions) -> ElkOptions {
    let number = |keys: &[&str]| {
        keys.iter().find_map(|key| {
            graph
                .layout_options
                .get(*key)
                .and_then(layered::parse_option_double)
        })
    };
    if let Some(value) = number(&[
        "spacing.nodeNodeBetweenLayers",
        "elk.spacing.nodeNodeBetweenLayers",
        "elk.layered.spacing.nodeNodeBetweenLayers",
        "layered.spacing.nodeNodeBetweenLayers",
        "org.eclipse.elk.spacing.nodeNodeBetweenLayers",
        "org.eclipse.elk.layered.spacing.nodeNodeBetweenLayers",
    ]) {
        options.node_spacing = value;
    }
    if let Some(value) = number(&[
        "spacing.edgeNodeBetweenLayers",
        "elk.spacing.edgeNodeBetweenLayers",
        "elk.layered.spacing.edgeNodeBetweenLayers",
        "org.eclipse.elk.spacing.edgeNodeBetweenLayers",
        "org.eclipse.elk.layered.spacing.edgeNodeBetweenLayers",
    ]) {
        options.edge_node_spacing = value;
    }
    if let Some(value) = number(&[
        "elk.spacing.nodeSelfLoop",
        "spacing.nodeSelfLoop",
        "org.eclipse.elk.spacing.nodeSelfLoop",
    ]) {
        options.self_loop_spacing = value;
    }
    options
}

/// Completes the root/component P1-P5 pipeline while retaining the dedicated
/// hierarchy scheduler for child scopes.
fn run_root_scope_pipeline(
    arena: &mut LayeredArena,
    root: LGraphId,
    wire_edges: &[ElkEdge],
    options: ElkOptions,
    vertical_direction: bool,
    includes_children: bool,
    consider_model_order: bool,
) -> Result<(), LayeringError> {
    const DEFAULT_NODE_NODE_SPACING: f64 = 20.0;
    layered::reverse_constraint_edges_for_layout(arena, root);
    trace_phase!("constraints", "external_port_edge_direction");
    trace_arena!("constraints", "external_port_edge_direction", arena);
    layered::break_cycles_for_layout(arena, root)?;
    trace_phase!("cycle_breaking", "break_cycles");
    trace_arena!("cycle_breaking", "break_cycles", arena);
    arena.insert_center_edge_label_dummies(root, wire_edges, vertical_direction)?;
    trace_phase!("labels", "insert_center_edge_dummies");
    trace_arena!("labels", "insert_center_edge_dummies", arena);
    layered::assign_layers_for_layout(arena, root)?;
    trace_rank_phase!("layering", arena, root);
    // ELK inserts long-edge dummies before node ordering. The dummies must
    // participate in the barycenter sweep; appending them after ordering
    // leaves the source graph order stale and changes BK's conflict window.
    layered::split_long_edges_for_layout(arena, root)?;
    trace_phase!("hierarchy", "split_long_edges");
    trace_arena!("hierarchy", "split_long_edges", arena);
    layered::process_inverted_ports_for_layout(arena, root);
    trace_phase!("ports", "inverted_port_preprocessor");
    trace_arena!("ports", "inverted_port_preprocessor", arena);
    let root_random_seed = arena.graphs[root as usize].random_seed;
    layered::sort_port_lists_for_layout(arena, root)?;
    trace_phase!("ordering", "port_list_sorter");
    trace_arena!("ordering", "port_list_sorter", arena);
    if consider_model_order {
        sort_layers_by_input_model_order(arena, root);
        trace_phase!("ordering", "sort_by_input_model_order");
        trace_arena!("ordering", "sort_by_input_model_order", arena);
    }
    preprocess_north_south_ports(arena, root);
    trace_phase!("ordering", "north_south_port_preprocessor");
    trace_arena!("ordering", "north_south_port_preprocessor", arena);
    if includes_children {
        order_layers_by_barycenter_with_model_order(
            arena,
            root,
            root_random_seed,
            consider_model_order,
        )?;
    } else {
        layered::order_layers_by_barycenter_with_components(
            arena,
            root,
            root_random_seed,
            consider_model_order,
        )?;
    }
    trace_phase!("ordering", "barycenter");
    trace_arena!("ordering", "barycenter", arena);
    layered::restore_pure_loop_port_areas_for_graph(arena, root);
    // runHierarchy resumes complete P4/P5 pipelines bottom-up. Finish child
    // resizes before the root's LabelAndNodeSizeProcessor fixes port positions.
    finish_nested_graphs(arena, root)?;
    trace_phase!("hierarchy", "finish_nested_graphs");
    trace_arena!("hierarchy", "finish_nested_graphs", arena);
    layered::redistribute_ordered_implicit_ports(arena, root);
    trace_phase!("ports", "redistribute_after_ordering");
    trace_arena!("ports", "redistribute_after_ordering", arena);
    // Flat ordinary graphs retain the balanced median used by the v0.9.0
    // wire witnesses. Orthogonal long-edge dummies and compound scopes use
    // the directional FavorStraightEdges boundary instead.
    calculate_innermost_node_margins(arena, root);
    trace_phase!("placement", "innermost_node_margins");
    trace_arena!("placement", "innermost_node_margins", arena);
    layered::switch_center_edge_label_dummies(arena, root)?;
    trace_phase!("labels", "switch_center_label_dummies");
    trace_arena!("labels", "switch_center_label_dummies", arena);
    if includes_children {
        arena.select_inline_edge_label_sides(root);
    } else {
        layered::for_each_separate_component(arena, root, |arena, component| {
            arena.select_inline_edge_label_sides(component);
            Ok(())
        })?;
    }
    trace_phase!("labels", "select_center_edge_sides");
    trace_arena!("labels", "select_center_edge_sides", arena);
    if includes_children {
        layered::merge_hyperedge_dummies(arena, root)?;
    } else {
        layered::for_each_separate_component(arena, root, |arena, graph| {
            layered::merge_hyperedge_dummies(arena, graph)
        })?;
    }
    trace_phase!("hierarchy", "merge_hyperedge_dummies");
    // OrthogonalEdgeRouter schedules loop port restoration and routing before
    // P4. Its node-local routes and labels contribute to the placement margin.
    reserve_loop_margins(arena, root)?;
    trace_phase!("self_loops", "reserve_margins_before_placement");
    trace_arena!("self_loops", "reserve_margins_before_placement", arena);
    if includes_children {
        place_bk_for_scope(arena, root, DEFAULT_NODE_NODE_SPACING)?;
    } else {
        layered::for_each_separate_component(arena, root, |arena, component| {
            place_bk_for_scope(arena, component, DEFAULT_NODE_NODE_SPACING)
        })?;
    }
    trace_phase!("placement", "brandes_koepf");
    trace_arena!("placement", "brandes_koepf", arena);
    align_external_edges(arena, root);
    trace_phase!("ports", "align_external_edges");
    trace_arena!("ports", "align_external_edges", arena);
    if includes_children {
        calculate_layer_sizes_and_graph_height(arena, root)?;
    } else {
        layered::for_each_separate_component(arena, root, |arena, component| {
            calculate_layer_sizes_and_graph_height(arena, component)?;
            let size = (
                arena.graphs[component as usize].width,
                arena.graphs[component as usize].height,
            );
            for node_id in arena.graphs[component as usize]
                .layerless_nodes
                .iter()
                .copied()
            {
                arena.component_sizes.insert(node_id, size);
                arena
                    .component_offsets_y
                    .insert(node_id, arena.graphs[component as usize].offset_y);
            }
            Ok(())
        })?;
    }
    trace_phase!("sizing", "calculate_graph_bounds");
    trace_arena!("sizing", "calculate_graph_bounds", arena);
    if includes_children {
        let edge_edge_spacing = arena.graphs[root as usize].edge_edge_spacing_between_layers;
        route_orthogonal_edges(
            arena,
            root,
            options.node_spacing,
            edge_edge_spacing,
            options.edge_node_spacing,
        )?;
    } else {
        layered::for_each_separate_component(arena, root, |arena, component| {
            let edge_edge_spacing =
                arena.graphs[component as usize].edge_edge_spacing_between_layers;
            route_orthogonal_edges(
                arena,
                component,
                options.node_spacing,
                edge_edge_spacing,
                options.edge_node_spacing,
            )?;
            let width = arena.graphs[component as usize].width;
            for node_id in arena.graphs[component as usize].layerless_nodes.iter() {
                if let Some(size) = arena.component_sizes.get_mut(node_id) {
                    size.0 = width;
                }
            }
            Ok(())
        })?;
    }
    trace_phase!("routing", "orthogonal_edges");
    trace_arena!("routing", "orthogonal_edges", arena);
    layered::restore_self_loop_edges_after_routing(arena, root);
    trace_phase!("self_loops", "restore_edges_after_routing");
    trace_arena!("self_loops", "restore_edges_after_routing", arena);
    postprocess_north_south_ports(arena, root);
    trace_phase!("routing", "north_south_port_postprocessor");
    trace_arena!("routing", "north_south_port_postprocessor", arena);
    join_long_edges(arena, root)?;
    trace_phase!("routing", "join_long_edges");
    trace_arena!("routing", "join_long_edges", arena);
    layered::remove_center_edge_label_dummies(arena, root)?;
    trace_phase!("labels", "remove_center_edge_dummies");
    trace_arena!("labels", "remove_center_edge_dummies", arena);
    layered::restore_reversed_edges_for_layout(arena, root);
    trace_phase!("routing", "restore_reversed_edges");
    trace_arena!("routing", "restore_reversed_edges", arena);
    Ok(())
}

fn prepare_nested_graphs(
    arena: &mut LayeredArena,
    graph_id: LGraphId,
    wire_edges: &[crate::model::ElkEdge],
    labels_transposed: bool,
) -> Result<(), LayeringError> {
    // elk-go's runHierarchy walks the graph tree breadth-first and reverses
    // that list before processing child scopes. This makes deeper scopes run
    // first and reverses sibling order; preserve that observable phase and
    // trace order here as well.
    let graph_ids = graphs_bottom_up(arena, graph_id);
    for nested_graph_id in graph_ids {
        if nested_graph_id == graph_id {
            continue;
        }
        prepare_nested_graph_scope(arena, nested_graph_id, wire_edges, labels_transposed)?;
    }
    Ok(())
}

fn graphs_bottom_up(arena: &LayeredArena, root_graph: LGraphId) -> Vec<LGraphId> {
    let mut queue = vec![root_graph];
    let mut cursor = 0;
    while cursor < queue.len() {
        let graph_id = queue[cursor];
        cursor += 1;
        let child_graphs = arena.graphs[graph_id as usize]
            .layerless_nodes
            .iter()
            .filter_map(|node_id| arena.nodes[*node_id as usize].nested_graph)
            .collect::<Vec<_>>();
        queue.extend(child_graphs);
    }
    queue.reverse();
    queue
}

fn prepare_nested_graph_scope(
    arena: &mut LayeredArena,
    graph_id: LGraphId,
    wire_edges: &[crate::model::ElkEdge],
    labels_transposed: bool,
) -> Result<(), LayeringError> {
    layered::reverse_constraint_edges_for_layout(arena, graph_id);
    trace_phase!("nested", "external_port_edge_direction");
    trace_arena!("nested", "external_port_edge_direction", arena);
    layered::break_cycles_for_layout(arena, graph_id)?;
    trace_phase!("nested", "break_cycles");
    trace_arena!("nested", "break_cycles", arena);
    arena.insert_center_edge_label_dummies(graph_id, wire_edges, labels_transposed)?;
    trace_phase!("nested", "insert_center_edge_dummies");
    trace_arena!("nested", "insert_center_edge_dummies", arena);
    layered::assign_layers_for_layout(arena, graph_id)?;
    trace_rank_phase!("nested", arena, graph_id);
    split_long_edges(arena, graph_id)?;
    layered::process_inverted_ports_for_layout(arena, graph_id);
    trace_phase!("nested", "inverted_port_preprocessor");
    trace_arena!("nested", "inverted_port_preprocessor", arena);
    layered::sort_port_lists_for_layout(arena, graph_id)?;
    trace_phase!("nested", "port_list_sorter");
    trace_arena!("nested", "port_list_sorter", arena);
    // runHierarchy executes every graph's pre-P3 processors before the shared
    // sweep. SortByInputModel is graph-local, including its port reordering.
    if arena.graphs[graph_id as usize].consider_model_order {
        sort_layers_by_input_model_order(arena, graph_id);
        trace_phase!("nested", "sort_by_input_model_order");
        trace_arena!("nested", "sort_by_input_model_order", arena);
    }
    preprocess_north_south_ports(arena, graph_id);
    trace_phase!("nested", "north_south_port_preprocessor");
    trace_arena!("nested", "north_south_port_preprocessor", arena);
    trace_phase!("nested", "barycenter_queued_for_hierarchy_scheduler");
    trace_arena!("nested", "barycenter_queued_for_hierarchy_scheduler", arena);
    Ok(())
}

/// Resumes hierarchy scopes after root P3, matching elk-go's bottom-up P4/P5
/// traversal. Child extents and external-port anchors are copied to their
/// owner before the owning graph is placed and routed.
fn finish_nested_graphs(
    arena: &mut LayeredArena,
    root_graph: LGraphId,
) -> Result<(), LayeringError> {
    for graph_id in graphs_bottom_up(arena, root_graph) {
        if graph_id == root_graph {
            continue;
        }
        finish_nested_graph_scope(arena, graph_id)?;
        transfer_external_ports_to_parent(arena, graph_id);
    }
    Ok(())
}

fn finish_nested_graph_scope(
    arena: &mut LayeredArena,
    graph_id: LGraphId,
) -> Result<(), LayeringError> {
    // ELK's `spacing.nodeNodeBetweenLayers` controls the rank gap. The
    // cross-axis gap between independent nodes in one rank remains ELK's
    // default 20px unless `spacing.nodeNode` is explicitly supplied; D2's
    // adapter only sets the former option.
    let rank_spacing = arena.graphs[graph_id as usize].layer_spacing;
    let edge_node_spacing = arena.graphs[graph_id as usize].edge_node_spacing;
    // Nested scopes use the directional BK boundary used by the hierarchy
    // processors; their external-port constraints are not the flat graph's
    // balanced-median case.
    layered::restore_pure_loop_port_areas_for_graph(arena, graph_id);
    layered::redistribute_ordered_implicit_ports(arena, graph_id);
    trace_phase!("nested", "redistribute_after_ordering");
    trace_arena!("nested", "redistribute_after_ordering", arena);
    calculate_innermost_node_margins(arena, graph_id);
    trace_phase!("nested", "innermost_node_margins");
    trace_arena!("nested", "innermost_node_margins", arena);
    layered::switch_center_edge_label_dummies(arena, graph_id)?;
    trace_phase!("nested", "switch_center_label_dummies");
    trace_arena!("nested", "switch_center_label_dummies", arena);
    arena.select_inline_edge_label_sides(graph_id);
    trace_phase!("nested", "select_center_edge_sides");
    trace_arena!("nested", "select_center_edge_sides", arena);
    layered::merge_hyperedge_dummies(arena, graph_id)?;
    trace_phase!("nested", "merge_hyperedge_dummies");
    reserve_loop_margins(arena, graph_id)?;
    trace_phase!("nested", "reserve_loop_margins_before_placement");
    trace_arena!("nested", "reserve_loop_margins_before_placement", arena);
    place_bk_for_scope(arena, graph_id, DEFAULT_ELK_NODE_NODE_SPACING)?;
    trace_phase!("nested", "brandes_koepf");
    trace_arena!("nested", "brandes_koepf", arena);
    calculate_layer_sizes_and_graph_height(arena, graph_id)?;
    trace_phase!("nested", "calculate_graph_bounds");
    trace_arena!("nested", "calculate_graph_bounds", arena);
    let edge_edge_spacing = arena.graphs[graph_id as usize].edge_edge_spacing_between_layers;
    route_orthogonal_edges(
        arena,
        graph_id,
        rank_spacing,
        edge_edge_spacing,
        edge_node_spacing,
    )?;
    trace_phase!("nested", "orthogonal_edges");
    trace_arena!("nested", "orthogonal_edges", arena);
    layered::restore_self_loop_edges_after_routing(arena, graph_id);
    trace_phase!("self_loops", "restore_edges_after_routing");
    trace_arena!("self_loops", "restore_edges_after_routing", arena);
    postprocess_north_south_ports(arena, graph_id);
    trace_phase!("nested", "north_south_port_postprocessor");
    trace_arena!("nested", "north_south_port_postprocessor", arena);
    join_long_edges(arena, graph_id)?;
    trace_phase!("nested", "join_long_edges");
    trace_arena!("nested", "join_long_edges", arena);
    layered::remove_center_edge_label_dummies(arena, graph_id)?;
    trace_phase!("labels", "remove_center_edge_dummies");
    trace_arena!("labels", "remove_center_edge_dummies", arena);
    layered::restore_reversed_edges_for_layout(arena, graph_id);
    trace_phase!("nested", "restore_reversed_edges");
    trace_arena!("nested", "restore_reversed_edges", arena);
    pad_nested_graph(arena, graph_id);
    trace_phase!("nested", "pad_graph");
    trace_arena!("nested", "pad_graph", arena);
    Ok(())
}

/// Selects Brandes–Koepf alignment the way elk-go's node placer does for one
/// graph scope. An explicit fixed alignment wins; otherwise ELK's
/// `favorStraightEdges` setting selects the balanced median or smallest valid
/// directional candidate.
fn place_bk_for_scope(
    arena: &mut LayeredArena,
    graph_id: LGraphId,
    node_spacing: f64,
) -> Result<(), LayeringError> {
    let graph = &arena.graphs[graph_id as usize];
    let alignment = graph.fixed_alignment;
    let default_balanced = !graph.favor_straight_edges;
    match alignment {
        BkFixedAlignment::LeftUp
        | BkFixedAlignment::RightUp
        | BkFixedAlignment::LeftDown
        | BkFixedAlignment::RightDown => {
            place_layers_bk_with_alignment(arena, graph_id, node_spacing, alignment)
        }
        BkFixedAlignment::Balanced => place_layers_bk(arena, graph_id, node_spacing, true),
        BkFixedAlignment::None => place_layers_bk(arena, graph_id, node_spacing, default_balanced),
    }
}

fn pad_nested_graph(arena: &mut LayeredArena, graph_id: LGraphId) {
    let padding = arena.graphs[graph_id as usize]
        .padding
        .unwrap_or(crate::layered::GraphPadding {
            top: 12.0,
            right: 12.0,
            bottom: 12.0,
            left: 12.0,
        });
    let node_ids = graph_all_node_ids(arena, graph_id);
    if node_ids.is_empty() {
        return;
    }
    let has_external_port_dummies = arena
        .external_port_origins
        .keys()
        .any(|dummy| arena.nodes[*dummy as usize].graph == graph_id);
    if has_external_port_dummies {
        pad_nested_graph_with_external_ports(arena, graph_id, &node_ids, padding);
        return;
    }
    let mut minimum_x = node_ids
        .iter()
        .map(|node_id| {
            let node = &arena.nodes[*node_id as usize];
            let loop_margin = arena.original_loop_margins.contains_key(node_id);
            node.position_x - if loop_margin { node.margin_left } else { 0.0 }
        })
        .fold(f64::INFINITY, f64::min);
    let mut minimum_y = node_ids
        .iter()
        .map(|node_id| {
            let node = &arena.nodes[*node_id as usize];
            let loop_margin = arena.original_loop_margins.contains_key(node_id);
            node.position_y - if loop_margin { node.margin_top } else { 0.0 }
        })
        .fold(f64::INFINITY, f64::min);
    // ComponentsCompactor computes a component hull from both node boxes and
    // routed edge points. Self-loops are excluded here because their
    // temporary importer bends are replaced by the node-local loop router;
    // their final clearance is carried by `original_loop_margins` below.
    for edge in &arena.edges {
        if !edge.active
            || edge.source == edge.target
            || arena.nodes[edge.source as usize].graph != graph_id
            || arena.nodes[edge.target as usize].graph != graph_id
        {
            continue;
        }
        for point in edge.bend_points.iter().chain(edge.junction_points.iter()) {
            minimum_x = minimum_x.min(point.x);
            minimum_y = minimum_y.min(point.y);
        }
    }
    for node_id in &node_ids {
        let node = &arena.nodes[*node_id as usize];
        for port_id in &node.ports {
            if !arena.port_border_offsets.contains_key(port_id) {
                continue;
            }
            let port = &arena.ports[*port_id as usize];
            match port.side {
                PortSide::West => minimum_x = minimum_x.min(node.position_x + port.x),
                PortSide::North => minimum_y = minimum_y.min(node.position_y + port.y),
                _ => {}
            }
        }
    }
    let shift_x = padding.left - minimum_x;
    // P4 records the normalization in Graph.Offset rather than moving nodes.
    // Compute the hull in that frame; publication applies the offset once.
    let shift_y = padding.top - (minimum_y + arena.graphs[graph_id as usize].offset_y);
    for node_id in &node_ids {
        arena
            .nested_transfer_x
            .entry(*node_id)
            .and_modify(|(internal, _)| *internal += shift_x)
            .or_insert((arena.nodes[*node_id as usize].position_x, shift_x));
        arena
            .nested_transfer_y
            .entry(*node_id)
            .and_modify(|(internal, _)| *internal += shift_y)
            .or_insert((arena.nodes[*node_id as usize].position_y, shift_y));
        arena.nodes[*node_id as usize].position_x += shift_x;
        arena.nodes[*node_id as usize].position_y += shift_y;
    }
    for (edge_id, edge) in arena.edges.iter_mut().enumerate() {
        if edge.active
            && arena.nodes[edge.source as usize].graph == graph_id
            && arena.nodes[edge.target as usize].graph == graph_id
        {
            if edge.source != edge.target {
                arena.nested_edge_transfer_y.insert(
                    edge_id as LEdgeId,
                    (
                        edge.bend_points.iter().map(|point| point.y).collect(),
                        shift_y,
                    ),
                );
                arena.nested_junction_transfer_y.insert(
                    edge_id as LEdgeId,
                    (
                        edge.junction_points.iter().map(|point| point.y).collect(),
                        shift_y,
                    ),
                );
            }
            for point in edge
                .bend_points
                .iter_mut()
                .chain(edge.junction_points.iter_mut())
            {
                point.x += shift_x;
                point.y += shift_y;
            }
        }
    }
    shift_removed_center_label_proxies(arena, graph_id, &node_ids, shift_x, shift_y);
    let node_width = node_ids
        .iter()
        .map(|node_id| {
            let node = &arena.nodes[*node_id as usize];
            let loop_right_margin =
                arena
                    .original_loop_margins
                    .get(node_id)
                    .map_or(0.0, |original| {
                        if node.margins_in_wire_dimensions {
                            (node.margin_right - original[1]).max(0.0)
                        } else {
                            node.margin_right
                        }
                    });
            let width = if node.nested_graph.is_some() {
                // The compound node's wire width is already the minimum
                // vessel dimension. Its outside decoration margin still
                // participates in the parent hull; subtracting it here
                // drops the right/bottom lane a second time.
                node.width + loop_right_margin
            } else {
                node.width + node.label_margin_left + node.label_margin_right + loop_right_margin
            };
            let origin_x = if node.nested_graph.is_some() {
                node.position_x - node.margin_left
            } else {
                node.position_x
            };
            origin_x + width
        })
        .fold(0.0, f64::max);
    let edge_width = arena
        .edges
        .iter()
        .filter(|edge| {
            edge.active
                && edge.source != edge.target
                && arena.nodes[edge.source as usize].graph == graph_id
                && arena.nodes[edge.target as usize].graph == graph_id
        })
        .flat_map(|edge| edge.bend_points.iter().chain(edge.junction_points.iter()))
        .map(|point| point.x)
        .fold(0.0, f64::max);
    let port_width = node_ids
        .iter()
        .flat_map(|node_id| arena.nodes[*node_id as usize].ports.iter().copied())
        .filter_map(|port_id| {
            arena.port_border_offsets.get(&port_id)?;
            let port = &arena.ports[port_id as usize];
            let node = &arena.nodes[port.node as usize];
            Some(match port.side {
                PortSide::East => node.position_x + port.x,
                PortSide::West => node.position_x + port.x + port.width,
                _ => node.position_x + port.x + port.width,
            })
        })
        .fold(0.0, f64::max);
    let mut width = node_width.max(port_width).max(edge_width) + padding.right;
    let node_height = node_ids
        .iter()
        .map(|node_id| {
            let node = &arena.nodes[*node_id as usize];
            let height = if node.nested_graph.is_some() {
                node.height
            } else {
                node.height + node.label_margin_top + node.label_margin_bottom
            };
            let loop_bottom_margin =
                arena
                    .original_loop_margins
                    .get(node_id)
                    .map_or(0.0, |original| {
                        if node.margins_in_wire_dimensions {
                            (node.margin_bottom - original[2]).max(0.0)
                        } else {
                            node.margin_bottom
                        }
                    });
            let origin_y = if node.nested_graph.is_some() {
                node.position_y - node.margin_top
            } else {
                node.position_y
            };
            origin_y + height + loop_bottom_margin
        })
        .fold(0.0, f64::max);
    let edge_height = arena
        .edges
        .iter()
        .filter(|edge| {
            edge.active
                && edge.source != edge.target
                && arena.nodes[edge.source as usize].graph == graph_id
                && arena.nodes[edge.target as usize].graph == graph_id
        })
        .flat_map(|edge| edge.bend_points.iter().chain(edge.junction_points.iter()))
        .map(|point| point.y)
        .fold(0.0, f64::max);
    let port_height = node_ids
        .iter()
        .flat_map(|node_id| arena.nodes[*node_id as usize].ports.iter().copied())
        .filter_map(|port_id| {
            arena.port_border_offsets.get(&port_id)?;
            let port = &arena.ports[port_id as usize];
            let node = &arena.nodes[port.node as usize];
            Some(match port.side {
                PortSide::South => node.position_y + port.y,
                PortSide::North => node.position_y + port.y + port.height,
                _ => node.position_y + port.y + port.height,
            })
        })
        .fold(0.0, f64::max);
    // P4 keeps cross-axis normalization in Graph.Offset. The local nodes,
    // ports and bends above still exclude that offset, whereas the compound
    // hull and GraphActualSize are in the normalized content frame.
    let mut height = node_height.max(port_height).max(edge_height)
        + arena.graphs[graph_id as usize].offset_y
        + padding.bottom;
    // HierarchicalNodeResizer uses GraphActualSize from P4/P5, even after
    // long-edge dummies are removed. Their thickness remains in graph size.
    width = width.max(arena.graphs[graph_id as usize].width + padding.left + padding.right);
    height = height.max(arena.graphs[graph_id as usize].height + padding.top + padding.bottom);
    let mut envelope_shift_x = 0.0;
    let mut envelope_shift_y = 0.0;
    if let Some(side_mask) = arena.reused_hierarchy_sides.get(&graph_id).copied() {
        // components_compactor_external.go reserves one edge-to-edge slot on
        // each axis around a reused endpoint. The hierarchy resizer therefore
        // opens the compound by 10px on both axes; an east/north endpoint
        // also shifts the child content to keep its outside side fixed.
        const EXTERNAL_COMPONENT_ENVELOPE: f64 = 10.0;
        let east = side_mask & (1 << PortSide::East as u8) != 0;
        let west = side_mask & (1 << PortSide::West as u8) != 0;
        let north = side_mask & (1 << PortSide::North as u8) != 0;
        let south = side_mask & (1 << PortSide::South as u8) != 0;
        if east || west {
            width += EXTERNAL_COMPONENT_ENVELOPE;
            height += EXTERNAL_COMPONENT_ENVELOPE;
            if east {
                envelope_shift_x = EXTERNAL_COMPONENT_ENVELOPE;
            }
        }
        if north || south {
            height += EXTERNAL_COMPONENT_ENVELOPE;
            width += EXTERNAL_COMPONENT_ENVELOPE;
            if north {
                envelope_shift_y = EXTERNAL_COMPONENT_ENVELOPE;
            }
        }
        if envelope_shift_x != 0.0 || envelope_shift_y != 0.0 {
            for node_id in &node_ids {
                arena.nodes[*node_id as usize].position_x += envelope_shift_x;
                arena.nodes[*node_id as usize].position_y += envelope_shift_y;
            }
            for (edge_id, edge) in arena.edges.iter_mut().enumerate() {
                if edge.active
                    && arena.nodes[edge.source as usize].graph == graph_id
                    && arena.nodes[edge.target as usize].graph == graph_id
                {
                    if let Some((_, shift)) =
                        arena.nested_edge_transfer_y.get_mut(&(edge_id as LEdgeId))
                    {
                        *shift += envelope_shift_y;
                    }
                    for point in &mut edge.bend_points {
                        point.x += envelope_shift_x;
                        point.y += envelope_shift_y;
                    }
                }
            }
        }
    }

    // D2 marks explicitly declared container dimensions as fixed axes.  The
    // layered child layout still computes its natural envelope first, but
    // ELK centers that envelope inside a larger declared axis.  Preserve the
    // source dimension as a minimum and move the local geometry together so
    // node, bend-point, and deferred nested-transfer coordinates stay in
    // lockstep.
    let parent_size = arena.graphs[graph_id as usize]
        .parent_node
        .map(|parent_node| {
            let parent = &arena.nodes[parent_node as usize];
            let minimum = arena
                .node_size_minimum
                .get(&parent_node)
                .copied()
                .unwrap_or((0.0, 0.0));
            let decorated = parent.margin_top > 0.0
                || parent.margin_right > 0.0
                || parent.margin_bottom > 0.0
                || parent.margin_left > 0.0;
            (
                parent.fixed_width,
                parent.fixed_height,
                parent.width,
                parent.height,
                minimum,
                decorated,
            )
        });
    let (mut target_width, mut target_height) = (width, height);
    if let Some((fixed_width, fixed_height, declared_width, declared_height, minimum, decorated)) =
        parent_size
    {
        target_width = target_width.max(minimum.0);
        target_height = target_height.max(minimum.1);
        if fixed_width || decorated {
            target_width = target_width.max(declared_width);
        }
        if fixed_height || decorated {
            target_height = target_height.max(declared_height);
        }
    }
    let fixed_shift_x = (target_width - width) / 2.0;
    let fixed_shift_y = (target_height - height) / 2.0;
    width = target_width;
    height = target_height;
    // resizeHierarchyGraph retains content centering in Graph.Offset. Joined
    // hierarchy routes read that same offset for both anchors and bends.
    arena.graphs[graph_id as usize].offset_y += fixed_shift_y;
    if fixed_shift_x != 0.0 {
        for node_id in &node_ids {
            let node = &mut arena.nodes[*node_id as usize];
            node.position_x += fixed_shift_x;
            if let Some((internal, _)) = arena.nested_transfer_x.get_mut(node_id) {
                *internal += fixed_shift_x;
            }
        }
        for edge in &mut arena.edges {
            if edge.active
                && edge.source != edge.target
                && arena.nodes[edge.source as usize].graph == graph_id
                && arena.nodes[edge.target as usize].graph == graph_id
            {
                for point in &mut edge.bend_points {
                    point.x += fixed_shift_x;
                }
            }
        }
    }
    arena.graphs[graph_id as usize].width = width;
    arena.graphs[graph_id as usize].height = height;
    if let Some(parent_node) = arena.graphs[graph_id as usize].parent_node {
        resize_hierarchy_node(arena, parent_node, width, height);
    }
}

/// Resizes a hierarchy scope using ELK's graph size plus padding. External
/// port dummies sit on the graph boundary and routed edge bends do not expand
/// the compound hull; the hierarchy transfer applies padding when publishing
/// child coordinates and maps each dummy to its final parent port.
fn pad_nested_graph_with_external_ports(
    arena: &mut LayeredArena,
    graph_id: LGraphId,
    node_ids: &[LNodeId],
    padding: crate::layered::GraphPadding,
) {
    let content_width = arena.graphs[graph_id as usize].width;
    let content_height = arena.graphs[graph_id as usize].height;
    let mut width = content_width + padding.left + padding.right;
    let mut height = content_height + padding.top + padding.bottom;
    let fixed_axes = arena.graphs[graph_id as usize]
        .parent_node
        .map(|parent_node| {
            let parent = &arena.nodes[parent_node as usize];
            (
                parent.fixed_width,
                parent.fixed_height,
                parent.width,
                parent.height,
            )
        });
    let mut fixed_shift_x = 0.0;
    let mut fixed_shift_y = 0.0;
    if let Some((fixed_width, fixed_height, declared_width, declared_height)) = fixed_axes {
        if fixed_width && declared_width > width {
            fixed_shift_x = (declared_width - width) / 2.0;
            width = declared_width;
        }
        if fixed_height && declared_height > height {
            fixed_shift_y = (declared_height - height) / 2.0;
            height = declared_height;
        }
    }
    let width_before_minimum = width;
    let height_before_minimum = height;
    if let Some(parent_node) = arena.graphs[graph_id as usize].parent_node
        && let Some((minimum_width, minimum_height)) =
            arena.node_size_minimum.get(&parent_node).copied()
    {
        if minimum_width > width {
            fixed_shift_x += (minimum_width - width) / 2.0;
            width = minimum_width;
        }
        if minimum_height > height {
            fixed_shift_y += (minimum_height - height) / 2.0;
            height = minimum_height;
        }
    }
    let minimum_width_delta = width - width_before_minimum;
    let minimum_height_delta = height - height_before_minimum;
    if minimum_width_delta != 0.0 || minimum_height_delta != 0.0 {
        for node_id in node_ids {
            if arena.nodes[*node_id as usize].node_type != LayeredNodeType::ExternalPort {
                continue;
            }
            let Some(origin_port) = arena.external_port_origins.get(node_id).copied() else {
                continue;
            };
            match arena.ports[origin_port as usize].side {
                PortSide::East => arena.nodes[*node_id as usize].position_x += minimum_width_delta,
                PortSide::South => {
                    arena.nodes[*node_id as usize].position_y += minimum_height_delta
                }
                PortSide::North | PortSide::West | PortSide::Undefined => {}
            }
        }
    }
    // elk-go's resizeHierarchyGraph keeps center-alignment displacement in
    // Graph.Offset, then transferHierarchyGraphToNode adds that offset while
    // deriving the boundary port. Keep the same split here: padding is part of
    // the child-local coordinates, while the cross-axis center displacement
    // remains a graph offset consumed by both port transfer and publication.
    arena.graphs[graph_id as usize].offset_y += fixed_shift_y;
    let shift_x = padding.left + fixed_shift_x;
    let shift_y = padding.top;
    for node_id in node_ids {
        if arena.nodes[*node_id as usize].node_type == LayeredNodeType::ExternalPort {
            continue;
        }
        let node = &mut arena.nodes[*node_id as usize];
        arena
            .nested_transfer_x
            .entry(*node_id)
            .and_modify(|(internal, _)| *internal += shift_x)
            .or_insert((node.position_x, shift_x));
        arena
            .nested_transfer_y
            .entry(*node_id)
            .and_modify(|(internal, _)| *internal += shift_y)
            .or_insert((node.position_y, shift_y));
        node.position_x += shift_x;
        node.position_y += shift_y;
    }
    for (edge_id, edge) in arena.edges.iter_mut().enumerate() {
        if edge.active
            && arena.nodes[edge.source as usize].graph == graph_id
            && arena.nodes[edge.target as usize].graph == graph_id
        {
            // Retain unpadded ordinates just as the non-boundary resize path
            // does. The transferrer adds (Graph.Offset + padding) after
            // summing node and port positions; summing padded nodes first
            // changes the final bits of fractional anchors and bends.
            if edge.source != edge.target {
                arena.nested_edge_transfer_y.insert(
                    edge_id as LEdgeId,
                    (
                        edge.bend_points.iter().map(|point| point.y).collect(),
                        shift_y,
                    ),
                );
                arena.nested_junction_transfer_y.insert(
                    edge_id as LEdgeId,
                    (
                        edge.junction_points.iter().map(|point| point.y).collect(),
                        shift_y,
                    ),
                );
            }
            for point in edge
                .bend_points
                .iter_mut()
                .chain(edge.junction_points.iter_mut())
            {
                point.x += shift_x;
                point.y += shift_y;
            }
        }
    }
    shift_removed_center_label_proxies(arena, graph_id, node_ids, shift_x, shift_y);
    arena.graphs[graph_id as usize].width = width;
    arena.graphs[graph_id as usize].height = height;
    if let Some(parent_node) = arena.graphs[graph_id as usize].parent_node {
        resize_hierarchy_node(arena, parent_node, width, height);
    }
}

// elk-go lgraph_util.go ResizeNode(moveLabels=true), followed by the label
// part of InnermostNodeMarginCalculator. The imported overflow is relative
// to the old box; retaining it after a compound grows enlarges its parent.
fn resize_hierarchy_node(arena: &mut LayeredArena, id: LNodeId, width: f64, height: f64) {
    let node = &mut arena.nodes[id as usize];
    if let Some(labels) = arena.compound_label_boxes.get_mut(&id) {
        let width_diff = f64::from((width - node.width) as f32);
        let height_diff = f64::from((height - node.height) as f32);
        let mut margins = [0.0_f64; 4];
        for [x, y, w, h] in labels {
            let middle_x = *x + *w / 2.0;
            let middle_y = *y + *h / 2.0;
            let width_percent = middle_x / node.width;
            let height_percent = middle_y / node.height;
            if width_percent + height_percent >= 1.0 {
                if width_percent - height_percent > 0.0 && middle_y >= 0.0 {
                    *x += width_diff;
                    *y = d2_rs_core::go_fmadd(height_diff, height_percent, *y);
                } else if width_percent - height_percent < 0.0 && middle_x >= 0.0 {
                    *x = d2_rs_core::go_fmadd(width_diff, width_percent, *x);
                    *y += height_diff;
                }
            }
            margins[0] = margins[0].max(-*y);
            margins[1] = margins[1].max(*x + *w - width);
            margins[2] = margins[2].max(*y + *h - height);
            margins[3] = margins[3].max(-*x);
        }
        node.label_margin_top = margins[0];
        node.label_margin_right = margins[1];
        node.label_margin_bottom = margins[2];
        node.label_margin_left = margins[3];
    }
    node.width = width;
    node.height = height;
}

/// Returns the nodes that elk-go's `GraphAllNodes` exposes: graph-level
/// layerless nodes followed by the nodes still present in each layer. Retired
/// edge-label and long-edge dummies remain in the arena for output/trace
/// bookkeeping, but do not participate in hierarchy sizing after removal.
fn graph_all_node_ids(arena: &LayeredArena, graph_id: LGraphId) -> Vec<LNodeId> {
    let graph = &arena.graphs[graph_id as usize];
    let mut seen = BTreeSet::new();
    graph
        .layerless_nodes
        .iter()
        .copied()
        .chain(
            graph
                .layers
                .iter()
                .flat_map(|layer_id| arena.layers[*layer_id as usize].nodes.iter().copied()),
        )
        .filter(|node_id| seen.insert(*node_id))
        .collect()
}

/// Label positions are kept in their proxy nodes until output publication.
/// ELK removes those nodes from the graph before applying padding to edge
/// labels, so carry the same local-frame translation without letting proxies
/// affect the resized graph hull.
fn shift_removed_center_label_proxies(
    arena: &mut LayeredArena,
    graph_id: LGraphId,
    active_node_ids: &[LNodeId],
    shift_x: f64,
    shift_y: f64,
) {
    let proxy_ids = arena
        .edge_label_dummies
        .iter()
        .map(|dummy| dummy.node)
        .filter(|node_id| arena.nodes[*node_id as usize].graph == graph_id)
        .collect::<Vec<_>>();
    for node_id in proxy_ids {
        if active_node_ids.contains(&node_id) {
            continue;
        }
        let node = &mut arena.nodes[node_id as usize];
        node.position_x += shift_x;
        node.position_y += shift_y;
    }
}

/// Transfers the laid-out position of an external-port dummy back to the
/// boundary port on its compound parent. This mirrors ELK's
/// `transferHierarchyGraphToNode`: the dummy is outside the child graph's
/// normal content hull, while the parent port remains fixed at the graph
/// border plus its edge-to-edge offset.
fn transfer_external_ports_to_parent(arena: &mut LayeredArena, graph_id: LGraphId) {
    let width = arena.graphs[graph_id as usize].width;
    let height = arena.graphs[graph_id as usize].height;
    let padding = arena.graphs[graph_id as usize]
        .padding
        .unwrap_or(crate::layered::GraphPadding {
            top: 12.0,
            right: 12.0,
            bottom: 12.0,
            left: 12.0,
        });
    let offset_y = arena.graphs[graph_id as usize].offset_y;
    let mut dummies = arena.graphs[graph_id as usize].layerless_nodes.clone();
    for layer_id in arena.graphs[graph_id as usize].layers.iter().copied() {
        dummies.extend(arena.layers[layer_id as usize].nodes.iter().copied());
    }
    let mut fixed_parent_nodes = BTreeSet::new();
    for dummy_id in dummies {
        if arena.nodes[dummy_id as usize].node_type != LayeredNodeType::ExternalPort {
            continue;
        }
        let Some(&boundary_id) = arena.external_port_origins.get(&dummy_id) else {
            continue;
        };
        fixed_parent_nodes.insert(arena.ports[boundary_id as usize].node);
        let side = arena.ports[boundary_id as usize].side;
        let (dummy_x, dummy_y, dummy_width, dummy_height) = {
            let dummy = &arena.nodes[dummy_id as usize];
            (
                dummy.position_x,
                dummy.position_y,
                dummy.width,
                dummy.height,
            )
        };
        // elk-go's ExternalPortPosition derives the parent port from the
        // laid-out external dummy's center. The connected edge endpoint is
        // deliberately not used here: port ordering and node placement have
        // already determined the dummy's position. Sum the dummy center
        // before adding (padding + graph offset - half the port size), as
        // lgraph_util.go does; reassociation changes fractional boundary ports.
        let offset = arena
            .port_border_offsets
            .get(&boundary_id)
            .copied()
            .unwrap_or(0.0);
        let content_width = width - padding.left - padding.right;
        let content_height = height - padding.top - padding.bottom;
        let (final_dummy_x, final_dummy_y, port_x, port_y) = match side {
            PortSide::East => (
                content_width + padding.right + offset,
                dummy_y,
                width + offset,
                (dummy_y + dummy_height * 0.5)
                    + (padding.top + offset_y - arena.ports[boundary_id as usize].height / 2.0),
            ),
            PortSide::West => (
                -(padding.left + offset),
                dummy_y,
                -arena.ports[boundary_id as usize].width - offset,
                (dummy_y + dummy_height * 0.5)
                    + (padding.top + offset_y - arena.ports[boundary_id as usize].height / 2.0),
            ),
            PortSide::North => (
                dummy_x,
                -(padding.top + offset),
                (dummy_x + dummy_width * 0.5)
                    + (padding.left - arena.ports[boundary_id as usize].width / 2.0),
                -arena.ports[boundary_id as usize].height - offset,
            ),
            PortSide::South => (
                dummy_x,
                content_height + padding.bottom + offset,
                (dummy_x + dummy_width * 0.5)
                    + (padding.left - arena.ports[boundary_id as usize].width / 2.0),
                height + offset,
            ),
            PortSide::Undefined => continue,
        };
        arena.nodes[dummy_id as usize].position_x = final_dummy_x;
        arena.nodes[dummy_id as usize].position_y = final_dummy_y;
        let port = &mut arena.ports[boundary_id as usize];
        port.x = port_x;
        port.y = port_y;
        #[cfg(feature = "diagnostic-traces")]
        crate::trace::emit_value(
            "hierarchy",
            "external_port_transfer",
            &format!(
                "graphs[{graph_id}].nodes[{}].ports[{boundary_id}]",
                arena.nodes[dummy_id as usize].source_id
            ),
            &format!(
                "dummy_id={dummy_id};dummy_y_bits={:016x};padding_top_bits={:016x};graph_offset_y_bits={:016x};port_y_bits={:016x};side={side:?}",
                final_dummy_y.to_bits(),
                arena.graphs[graph_id as usize]
                    .padding
                    .map_or(12.0, |padding| padding.top)
                    .to_bits(),
                arena.graphs[graph_id as usize].offset_y.to_bits(),
                port.y.to_bits(),
            ),
        );
    }
    // ELK's transferHierarchyGraphToNode fixes the owning compound's ports
    // after copying positions from its external dummies. Keeping the previous
    // FIXED_SIDE state lets the next scope redistribute those transferred
    // coordinates, moving sibling boundary ports away from their source P5
    // lanes.
    for parent_node in fixed_parent_nodes {
        arena.nodes[parent_node as usize].port_constraints = PortConstraints::FixedPosition;
        #[cfg(feature = "diagnostic-traces")]
        crate::trace::emit_value(
            "hierarchy",
            "external_port_parent_constraints",
            &format!("nodes[{}]", arena.nodes[parent_node as usize].source_id),
            "FixedPosition",
        );
    }
}

fn transform_node_geometry(
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    direction: &str,
    axis_min: f64,
    axis_max: f64,
) -> (f64, f64, f64, f64) {
    match direction {
        "DOWN" => (y, x, height, width),
        "UP" => (y, (axis_min + axis_max - width) - x, height, width),
        "LEFT" => ((axis_min + axis_max - width) - x, y, width, height),
        _ => (x, y, width, height),
    }
}

fn transform_port_geometry(
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    owner_width: f64,
    direction: &str,
) -> (f64, f64, f64, f64) {
    let (x, y, width, height) = match direction {
        "DOWN" => (y, x, height, width),
        "UP" => (y, (owner_width - width) - x, height, width),
        "LEFT" => ((owner_width - width) - x, y, width, height),
        _ => (x, y, width, height),
    };
    // core.Codec transferShape adds the (default local) zero offset after
    // direction transformation. Keep that arithmetic at publication; internal
    // negative-zero port positions remain observable in diagnostic stages.
    (x + 0.0, y + 0.0, width, height)
}

fn graph_offset_in_output_coordinates(offset_y: f64, direction: &str) -> Point {
    match direction {
        "DOWN" => Point {
            x: offset_y,
            y: 0.0,
        },
        // GraphTransformer transposes Offset for UP as for DOWN;
        // mirroring changes node/port coordinates, not the offset's sign.
        "UP" => Point {
            x: offset_y,
            y: 0.0,
        },
        _ => Point {
            x: 0.0,
            y: offset_y,
        },
    }
}

fn transpose_arena_for_vertical(arena: &mut LayeredArena) {
    arena.transpose_label_side_modes();
    for labels in arena.compound_label_boxes.values_mut() {
        for [x, y, width, height] in labels {
            std::mem::swap(x, y);
            std::mem::swap(width, height);
        }
    }
    for node in &mut arena.nodes {
        node.alignment = node.alignment.transposed();
        std::mem::swap(&mut node.width, &mut node.height);
        std::mem::swap(&mut node.fixed_width, &mut node.fixed_height);
        // Leaf wire boxes already carry their source-space outside lanes in
        // the rotated dimension. Compound vessels, however, are resized
        // again from their child graph and need the owning margin orientation
        // for the hierarchy transfer boundary.
        if node.nested_graph.is_some() {
            let (top, right, bottom, left) = (
                node.margin_top,
                node.margin_right,
                node.margin_bottom,
                node.margin_left,
            );
            node.margin_top = left;
            node.margin_right = bottom;
            node.margin_bottom = right;
            node.margin_left = top;
        }
        let (top, right, bottom, left) = (
            node.label_margin_top,
            node.label_margin_right,
            node.label_margin_bottom,
            node.label_margin_left,
        );
        node.label_margin_top = left;
        node.label_margin_right = bottom;
        node.label_margin_bottom = right;
        node.label_margin_left = top;
    }
    for port in &mut arena.ports {
        std::mem::swap(&mut port.x, &mut port.y);
        std::mem::swap(&mut port.width, &mut port.height);
        port.side = match port.side {
            PortSide::North => PortSide::West,
            PortSide::East => PortSide::South,
            PortSide::South => PortSide::East,
            PortSide::West => PortSide::North,
            PortSide::Undefined => PortSide::Undefined,
        };
    }
    for graph in &mut arena.graphs {
        if let Some(padding) = &mut graph.padding {
            let (top, right, bottom, left) =
                (padding.top, padding.right, padding.bottom, padding.left);
            padding.top = left;
            padding.right = bottom;
            padding.bottom = right;
            padding.left = top;
        }
    }
}

// The provider deliberately calls TransformGraph(ToInputDirection) before
// layout. For UP/READING_DIRECTION it mirrors original Y then transposes;
// after our transpose this is a mirror of internal X, including fixed ports.
fn normalize_up_to_internal(arena: &mut LayeredArena) {
    for scope in 0..arena.graphs.len() {
        let nodes = arena.graphs[scope].layerless_nodes.clone();
        let offset = nodes
            .iter()
            .map(|id| {
                let node = &arena.nodes[*id as usize];
                node.position_x + node.width + node.margin_right
            })
            .fold(0.0, f64::max);
        for id in nodes {
            let node = &mut arena.nodes[id as usize];
            node.alignment = node.alignment.mirrored();
            node.position_x = (offset - node.width) - node.position_x;
            std::mem::swap(&mut node.label_margin_left, &mut node.label_margin_right);
            if let Some(labels) = arena.compound_label_boxes.get_mut(&id) {
                for [x, _, width, _] in labels {
                    *x = (node.width - *width) - *x;
                }
            }
            for port in node.ports.clone() {
                let port = &mut arena.ports[port as usize];
                port.x = (node.width - port.width) - port.x;
                port.side = match port.side {
                    PortSide::East => PortSide::West,
                    PortSide::West => PortSide::East,
                    side => side,
                };
            }
        }
        if let Some(padding) = &mut arena.graphs[scope].padding {
            std::mem::swap(&mut padding.left, &mut padding.right);
        }
    }
}

/// Mirrors LEFT-directed scopes into elk-go's internal left-to-right frame.
/// The source DirectionPreprocessor performs this before placement and swaps
/// the padding, margins, port sides, and port positions along with node
/// coordinates. Keeping the arena in that frame lets the ordinary placement
/// and routing phases work with the same east-going assumptions as RIGHT.
fn normalize_left_to_right(arena: &mut LayeredArena) {
    for graph_id in 0..arena.graphs.len() {
        if arena.graphs[graph_id].direction != LayerDirection::Left {
            continue;
        }
        let node_ids = arena.graphs[graph_id].layerless_nodes.clone();
        let offset = node_ids
            .iter()
            .map(|node_id| {
                let node = &arena.nodes[*node_id as usize];
                node.position_x + node.width + node.margin_right
            })
            .fold(0.0_f64, f64::max);
        for node_id in node_ids {
            let node = &mut arena.nodes[node_id as usize];
            node.alignment = node.alignment.mirrored();
            node.position_x = offset - node.width - node.position_x;
            std::mem::swap(&mut node.margin_left, &mut node.margin_right);
            std::mem::swap(&mut node.label_margin_left, &mut node.label_margin_right);
            if let Some(labels) = arena.compound_label_boxes.get_mut(&node_id) {
                for [x, _, width, _] in labels {
                    *x = node.width - *width - *x;
                }
            }
            for port_id in &node.ports {
                let port = &mut arena.ports[*port_id as usize];
                port.x = node.width - port.width - port.x;
                port.side = match port.side {
                    crate::layered::PortSide::East => crate::layered::PortSide::West,
                    crate::layered::PortSide::West => crate::layered::PortSide::East,
                    side => side,
                };
            }
        }
        if let Some(padding) = &mut arena.graphs[graph_id].padding {
            std::mem::swap(&mut padding.left, &mut padding.right);
        }
        arena.graphs[graph_id].direction = LayerDirection::Right;
        #[cfg(feature = "diagnostic-traces")]
        crate::trace::emit_value(
            "direction",
            "normalize_left_to_right",
            &format!("graphs[{graph_id}]"),
            &format!(
                "nodes={};offset_bits={}",
                arena.graphs[graph_id].layerless_nodes.len(),
                offset.to_bits()
            ),
        );
    }
}

fn transform_route_point(point: Point, direction: &str, axis_min: f64, axis_max: f64) -> Point {
    match direction {
        "DOWN" => Point {
            x: point.y,
            y: point.x,
        },
        "UP" => Point {
            x: point.y,
            y: axis_min + axis_max - point.x,
        },
        "LEFT" => Point {
            x: axis_min + axis_max - point.x,
            y: point.y,
        },
        _ => point,
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct NodeDegree {
    incoming: usize,
    outgoing: usize,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct GraphStats {
    degrees: BTreeMap<d2_rs_core::NodeId, NodeDegree>,
    max_loop_label_by_owner: BTreeMap<Option<d2_rs_core::NodeId>, f64>,
}

/// Collect the graph-wide measurements used by D2's ELK adapter.
fn collect_graph_stats(graph: &Graph, vertical: bool) -> GraphStats {
    let mut stats = GraphStats::default();
    for (edge_id, edge) in graph.edges() {
        stats.degrees.entry(edge.source).or_default().outgoing += 1;
        stats.degrees.entry(edge.target).or_default().incoming += 1;
        if edge.source == edge.target
            && graph
                .edge_label(edge_id)
                .is_some_and(|label| !label.text.is_empty())
        {
            let label_size = graph.edge_label(edge_id).map_or(0.0, |label| {
                if vertical {
                    label.size.width
                } else {
                    label.size.height
                }
            });
            let owner = graph.node(edge.source).and_then(|node| node.parent);
            let entry = stats.max_loop_label_by_owner.entry(owner).or_default();
            *entry = (*entry).max(label_size);
        }
    }
    stats
}

fn prepare_layered_graph(graph: &Graph, options: ElkOptions) -> (Graph, ElkOptions) {
    let vertical = matches!(graph.direction, Direction::Down | Direction::Up);
    let stats = collect_graph_stats(graph, vertical);
    prepare_layered_graph_with_stats(graph, options, &stats)
}

fn prepare_layered_graph_with_stats(
    graph: &Graph,
    options: ElkOptions,
    stats: &GraphStats,
) -> (Graph, ElkOptions) {
    let vertical = matches!(graph.direction, Direction::Down | Direction::Up);
    let mut prepared = graph.clone();
    for id in graph.nodes().map(|(id, _)| id) {
        let degree = stats.degrees.get(&id).copied().unwrap_or_default();
        let required = degree.incoming.max(degree.outgoing) as f64 * 40.0;
        if required > 0.0
            && let Some(node) = prepared.node_mut(id)
            && (degree.incoming >= 2 || degree.outgoing >= 2)
        {
            if vertical && !node.fixed_width {
                node.size.width = node.size.width.max(required);
            } else if !vertical && !node.fixed_height {
                node.size.height = node.size.height.max(required);
            }
        }
        if let Some(node) = prepared.node_mut(id) {
            // Resolve positions before measuring outside bands, matching
            // d2elklayout.positionLabelsIcons, including oversized leaf labels.
            // Publication uses this same resolver for the rendered label state.
            let (label_position, icon_position) = codec::source_node_positions(graph, id);
            node.label_position = label_position;
            node.icon_position = icon_position;
            // Image assets do not reserve the ordinary icon lane (HasIcon).
            let spacing_has_icon = node.has_icon && node.shape != ShapeKind::Image;
            // d2elklayout.Layout grows icon-bearing nodes before it calls
            // SpacingOpt, so GetIconSize sees the post-label height. Keep the
            // same order here; otherwise outside icon lanes are undersized by
            // the label line and compound siblings acquire the wrong gap.
            if spacing_has_icon
                && !matches!(
                    node.shape,
                    ShapeKind::Text | ShapeKind::Class | ShapeKind::SqlTable | ShapeKind::Code
                )
                && node
                    .label_size
                    .is_some_and(|size| size.width > 0.0 || size.height > 0.0)
            {
                let label_height = node.label_size.map_or(0.0, |size| size.height);
                node.size.height += label_height + 5.0;
            }
            let mut margins = node.layout_margins;
            let has_label = !matches!(
                node.shape,
                ShapeKind::Text | ShapeKind::Class | ShapeKind::SqlTable | ShapeKind::Code
            ) && graph
                .node_labels
                .get(&id)
                .is_some_and(|label| !label.is_empty())
                && node
                    .label_size
                    .is_some_and(|size| size.width > 0.0 || size.height > 0.0);
            if has_label {
                let label_width = node.label_size.map_or(0.0, |size| size.width) + 5.0;
                let label_height = node.label_size.map_or(0.0, |size| size.height) + 5.0;
                match node.label_position {
                    LabelPosition::OutsideTopLeft
                    | LabelPosition::OutsideTopCenter
                    | LabelPosition::OutsideTopRight => margins.top = label_height,
                    LabelPosition::OutsideBottomLeft
                    | LabelPosition::OutsideBottomCenter
                    | LabelPosition::OutsideBottomRight => margins.bottom = label_height,
                    LabelPosition::OutsideLeftTop
                    | LabelPosition::OutsideLeftMiddle
                    | LabelPosition::OutsideLeftBottom => margins.left = label_width,
                    LabelPosition::OutsideRightTop
                    | LabelPosition::OutsideRightMiddle
                    | LabelPosition::OutsideRightBottom => margins.right = label_width,
                    _ => {}
                }
            }
            if spacing_has_icon {
                // d2graph.SpacingOpt uses the measured icon size for ordinary
                // leaves (`maxIconSize=false`), rather than reserving the
                // 64px maximum unconditionally. This matters for outside
                // icons: small boxes get a 32px icon plus 5px padding, while
                // larger boxes grow up to d2target.MAX_ICON_SIZE.
                let icon_margin = icon_size_with_padding(node);
                match node
                    .icon_position
                    .unwrap_or(LabelPosition::InsideMiddleCenter)
                {
                    LabelPosition::OutsideTopLeft
                    | LabelPosition::OutsideTopCenter
                    | LabelPosition::OutsideTopRight => margins.top = margins.top.max(icon_margin),
                    LabelPosition::OutsideBottomLeft
                    | LabelPosition::OutsideBottomCenter
                    | LabelPosition::OutsideBottomRight => {
                        margins.bottom = margins.bottom.max(icon_margin)
                    }
                    LabelPosition::OutsideLeftTop
                    | LabelPosition::OutsideLeftMiddle
                    | LabelPosition::OutsideLeftBottom => {
                        margins.left = margins.left.max(icon_margin)
                    }
                    LabelPosition::OutsideRightTop
                    | LabelPosition::OutsideRightMiddle
                    | LabelPosition::OutsideRightBottom => {
                        margins.right = margins.right.max(icon_margin)
                    }
                    _ => {}
                }
            }
            // SpacingOpt adds effect adjustments after label/icon maxima.
            // Hexagon's Go integer THREE_DEE_OFFSET/2 is 7, not 7.5.
            let (dx, dy) = if node.is_3d {
                (
                    15.0,
                    if node.shape == ShapeKind::Hexagon {
                        7.0
                    } else {
                        15.0
                    },
                )
            } else if node.is_multiple {
                (10.0, 10.0)
            } else {
                (0.0, 0.0)
            };
            margins.right += dx;
            margins.top += dy;
            node.layout_margins = margins;
            // ELK receives outside labels as label records on the node. The
            // v0.9.0 Go adapter does not fold those labels into the node's
            // width or height; the renderer places them outside the shape
            // after layout. Keeping the measured box unchanged is essential
            // for person/image nodes (and for any explicit outside label).
        }
    }
    let mut options = options;
    let root_loop_label = stats
        .max_loop_label_by_owner
        .get(&None)
        .copied()
        .unwrap_or(0.0);
    // The Go adapter measures label extents as integers and divides the
    // extent by two using integer division before adding its five-pixel
    // clearance. Match that truncation for odd-width labels.
    let source_loop_clearance = (root_loop_label.floor() / 2.0).floor() + 5.0;
    if options.self_loop_spacing == ElkOptions::default().self_loop_spacing {
        options.self_loop_spacing = options.self_loop_spacing.max(source_loop_clearance);
    }
    (prepared, options)
}

/// Return the icon lane used by `d2graph.SpacingOpt` for a measured leaf.
///
/// The Go target truncates the minimum box dimension before taking the
/// half-dimension ceiling. Keep that order here so small icon-bearing shapes
/// receive the same outside margin and do not inherit the container-only
/// 64px maximum lane.
fn icon_size_with_padding(node: &d2_rs_core::Node) -> f64 {
    const DEFAULT_ICON_SIZE: f64 = 32.0;
    const MAX_ICON_SIZE: f64 = 64.0;
    const LABEL_PADDING: f64 = 5.0;
    let min_dimension = node.size.width.min(node.size.height).max(0.0).floor();
    let half_min_dimension = (0.5 * min_dimension).ceil();
    let position = node
        .icon_position
        .unwrap_or(LabelPosition::InsideMiddleCenter);
    let mut size = if position == LabelPosition::InsideMiddleCenter {
        half_min_dimension
    } else {
        min_dimension.min(DEFAULT_ICON_SIZE.max(half_min_dimension))
    };
    size = size.min(MAX_ICON_SIZE);
    if !position.is_outside() {
        size = size
            .min((node.size.width - 2.0 * LABEL_PADDING).max(0.0))
            .min((node.size.height - 2.0 * LABEL_PADDING).max(0.0));
    }
    size + LABEL_PADDING
}

/// Move route endpoints to the fixed or dynamically spread table ports used
/// by D2's ELK adapter. ELK returns sections in graph coordinates, while the
/// shared route result uses the same absolute rectangles as the other engines.
fn apply_table_ports(graph: &Graph, result: &mut LayoutResult) {
    let mut dynamic_counts = BTreeMap::<(d2_rs_core::NodeId, bool), usize>::new();
    let mut dynamic_seen = BTreeMap::<(d2_rs_core::NodeId, bool), usize>::new();
    for (edge_id, edge) in graph.edges() {
        let columns = graph.edge_table_columns(edge_id);
        if graph
            .node(edge.source)
            .is_some_and(|node| node.shape == ShapeKind::SqlTable && columns.source.is_none())
        {
            *dynamic_counts.entry((edge.source, true)).or_default() += 1;
        }
        if graph
            .node(edge.target)
            .is_some_and(|node| node.shape == ShapeKind::SqlTable && columns.target.is_none())
        {
            *dynamic_counts.entry((edge.target, false)).or_default() += 1;
        }
    }

    for (edge_id, edge) in graph.edges() {
        let columns = graph.edge_table_columns(edge_id);
        let source_port = table_port(
            graph,
            result,
            edge.source,
            columns.source,
            true,
            &dynamic_counts,
            &mut dynamic_seen,
        );
        let target_port = table_port(
            graph,
            result,
            edge.target,
            columns.target,
            false,
            &dynamic_counts,
            &mut dynamic_seen,
        );
        if let Some(route) = result.routes.get_mut(&edge_id) {
            if let Some(point) = source_port
                && let Some(first) = route.first_mut()
            {
                *first = point;
            }
            if let Some(point) = target_port
                && let Some(last) = route.last_mut()
            {
                *last = point;
            }
        }
    }
}

fn table_port(
    graph: &Graph,
    result: &LayoutResult,
    node_id: d2_rs_core::NodeId,
    column: Option<usize>,
    source: bool,
    dynamic_counts: &BTreeMap<(d2_rs_core::NodeId, bool), usize>,
    dynamic_seen: &mut BTreeMap<(d2_rs_core::NodeId, bool), usize>,
) -> Option<Point> {
    let node = graph.node(node_id)?;
    if node.shape != ShapeKind::SqlTable {
        return None;
    }
    let rect = *result.boxes.get(&node_id)?;
    if let Some(column) = column {
        let count = graph
            .table_column_count(node_id)
            .unwrap_or(0)
            .max(column + 1);
        let cell_height = rect.size.height / (count as f64 + 1.0);
        let y = rect.origin.y + (column as f64 + 1.5) * cell_height;
        let west = graph.direction == Direction::Left && source
            || graph.direction != Direction::Left && !source;
        return Some(Point {
            x: if west { rect.origin.x } else { rect.right() },
            y,
        });
    }

    let key = (node_id, source);
    let index = dynamic_seen.entry(key).or_default();
    let count = dynamic_counts.get(&key).copied().unwrap_or(1);
    let x = rect.origin.x + rect.size.width * (*index as f64 + 1.0) / (count as f64 + 1.0);
    *index += 1;
    let north = matches!(graph.direction, Direction::Up) && source
        || !matches!(graph.direction, Direction::Up) && !source;
    Some(Point {
        x,
        y: if north { rect.origin.y } else { rect.bottom() },
    })
}

/// Lays out a graph through the in-process ELK implementation.
pub fn layout(graph: &Graph, options: &LayoutOptions) -> Result<LayoutResult, LayoutError> {
    // The source-shaped ELK Graph adapter already publishes elk-go's default
    // root padding. Applying a second D2-side translation here double-counts
    // that envelope for the ordinary in-process layout API.
    layout_with_options(graph, options, ElkOptions::default())
}

/// Applies d2elklayout.Layout's admission at the actual core invocation.
/// Specialized grid/sequence layout and the default crossing-edge router
/// bypass this core-engine admission boundary.
pub fn layout_with_cancellation(
    graph: &Graph,
    options: &LayoutOptions,
    cancellation: Option<&d2_rs_core::CancellationToken>,
) -> Result<LayoutResult, LayoutError> {
    d2_rs_core::layout_guard::check("ELK", graph, cancellation).map_err(|error| {
        LayoutError::Context {
            context: "failed to ELK layout",
            source: Box::new(error),
        }
    })?;
    layout(graph, options)
}

/// Runs the translated source-shaped layered processor chain on a measured
/// graph.
///
/// This is also the implementation behind the ordinary [`layout`] entry point
/// for `ElkAlgorithm::Layered`. Keeping the named entry point public lets
/// parity tests exercise cycle breaking, layering, ordering, BK placement,
/// long-edge handling, and section publication directly.
pub fn layout_with_layered_pipeline(
    graph: &Graph,
    options: &LayoutOptions,
    elk_options: ElkOptions,
) -> Result<LayoutResult, LayoutError> {
    layout_with_wire_pipeline(
        graph,
        options,
        elk_options,
        layout_elk_graph_with_layered_pipeline,
    )
}

fn layout_with_wire_pipeline(
    graph: &Graph,
    options: &LayoutOptions,
    elk_options: ElkOptions,
    execute: fn(&mut ElkGraph, ElkOptions) -> Result<(), LayeringError>,
) -> Result<LayoutResult, LayoutError> {
    validate_options(elk_options)?;
    let mut elk = graph_to_elk_graph(graph, &graph.node_labels, elk_options);
    execute(&mut elk, elk_options).map_err(|error| {
        if matches!(error, LayeringError::Provider { .. }) {
            // Preserve elk-go Engine -> LayoutJSON -> d2elklayout contexts.
            // A provider rejection is a user-input diagnostic, not a broken
            // routing invariant.
            [
                "layout ELK graph",
                "native ELK layout failed",
                "failed to ELK layout",
            ]
            .into_iter()
            .fold(
                LayoutError::EngineValidation(error.to_string()),
                |source, context| LayoutError::Context {
                    context,
                    source: Box::new(source),
                },
            )
        } else {
            LayoutError::BadRouteState(error.to_string())
        }
    })?;
    // Keep the shared report/label contract while replacing its geometry with
    // the source-shaped ELK result. This base contains no solver geometry;
    // `apply_elk_graph_result` overwrites every node box and route below.
    // ELK never reads TALA seed options; this value is report metadata only.
    let seed = options.seeds.first().copied().unwrap_or(1);
    let base = LayoutResult {
        removed_edges: Default::default(),
        source_edge_storage: Default::default(),
        synthetic_root: None,
        route_curves: Default::default(),
        sequence_lifelines: Vec::new(),
        edge_label_percentage_commits: Default::default(),
        boxes: graph
            .nodes()
            .map(|(id, node)| {
                (
                    id,
                    d2_rs_core::Rect {
                        origin: node.locked_position.unwrap_or_default(),
                        size: node.size,
                    },
                )
            })
            .collect(),
        routes: BTreeMap::new(),
        edge_labels: graph
            .edges()
            .filter_map(|(id, _)| {
                graph.edge_label(id).cloned().map(|mut label| {
                    if !label.text.is_empty() && label.position == LabelPosition::Unset {
                        label.position = LabelPosition::InsideMiddleCenter;
                    }
                    (id, label)
                })
            })
            .collect(),
        node_labels: graph
            .nodes()
            .map(|(id, node)| {
                let (position, icon_position) = codec::source_node_positions(graph, id);
                (
                    id,
                    NodeLabelState {
                        font_size_committed: false,
                        size: node.label_size,
                        font_size: node.font_size,
                        position,
                        icon_position,
                    },
                )
            })
            .collect(),
        report: LayoutReport {
            selected_seed: seed,
            selected_pass: 0,
            score: Score::default(),
            improvement_passes: 1,
            candidates: vec![CandidateReport {
                seed,
                pass: 0,
                score: Score::default(),
                race_score: None,
                race_score_components: None,
                race_seed_edge_order: None,
                accepted: true,
            }],
            warnings: Vec::new(),
        },
    };
    let mut result = apply_elk_graph_result(graph, &elk, &base)?;
    // The wire solver already routed to the table's fixed/dynamic ports.
    // d2elklayout preserves these endpoints while removing outside margins;
    // recomputing them from the shrunken table moves column attachments.
    shape_trace::trace_routes(graph, &mut result);
    bends::delete_bends(graph, &mut result);
    deconflict_self_loop_labels(graph, &mut result);
    // Go ELK publishes orthogonal sections without setting Edge.IsCurve.
    // Authored curve flags, if present on an API graph, remain unchanged.
    for (id, _) in graph.edges() {
        result.route_curves.insert(id, graph.edge_is_curve(id));
    }
    result.report.warnings.insert(
        0,
        if elk_options.algorithm == ElkAlgorithm::Fixed {
            "ELK source Fixed wire provider (broader raw contracts remain unqualified)."
        } else if elk_options.algorithm == ElkAlgorithm::MrTree {
            "ELK source Mr. Tree provider (treeification, ordering, compaction and routes)."
        } else if elk_options.algorithm == ElkAlgorithm::Radial {
            "ELK source Radial provider (wedge placement, ordering and compaction)."
        } else if matches!(
            elk_options.algorithm,
            ElkAlgorithm::Force | ElkAlgorithm::Stress
        ) {
            "ELK source Force/Stress provider (seeded native particle and majorization paths)."
        } else if elk_options.algorithm == ElkAlgorithm::Random {
            "ELK source Random provider (the default zero seed uses clock entropy)."
        } else if elk_options.algorithm == ElkAlgorithm::Rectpacking {
            "ELK source default Rectpacking chain (advanced modes remain unqualified)."
        } else if elk_options.algorithm == ElkAlgorithm::Box {
            "ELK source Box packing (SIMPLE and three grouping modes)."
        } else {
            "ELK source-shaped layered processor chain (qualification path)."
        }
        .to_owned(),
    );
    Ok(result)
}

/// Lays out a graph with explicit ELK adapter options.
pub fn layout_with_options(
    graph: &Graph,
    options: &LayoutOptions,
    mut elk_options: ElkOptions,
) -> Result<LayoutResult, LayoutError> {
    d2_rs_core::layout_guard::check("ELK", graph, None).map_err(|error| LayoutError::Context {
        context: "failed to ELK layout",
        source: Box::new(error),
    })?;
    let configured = &graph.layout_engine_options;
    if let Some(value) = configured.elk_algorithm.as_deref() {
        // resolveAlgorithms supplies the default only for an empty option and
        // ignores unresolved algorithms on a scope without layout content.
        elk_options.algorithm = if value.is_empty() || graph.nodes().next().is_none() {
            ElkAlgorithm::Layered
        } else {
            ElkAlgorithm::parse(value).map_err(|error| {
                [
                    "layout ELK graph",
                    "native ELK layout failed",
                    "failed to ELK layout",
                ]
                .into_iter()
                .fold(error, |source, context| LayoutError::Context {
                    context,
                    source: Box::new(source),
                })
            })?
        };
    }
    if let Some(value) = configured.elk_node_spacing {
        elk_options.node_spacing = value as f64;
    }
    if let Some(value) = configured.elk_edge_node_spacing {
        elk_options.edge_node_spacing = value as f64;
    }
    if let Some(value) = configured.elk_edge_edge_spacing {
        elk_options.edge_edge_spacing = value as f64;
    }
    if let Some(value) = configured.elk_self_loop_spacing {
        elk_options.self_loop_spacing = value as f64;
    }
    validate_options(elk_options)?;
    match elk_options.algorithm {
        ElkAlgorithm::Layered => layout_with_layered_pipeline(graph, options, elk_options),
        ElkAlgorithm::Fixed => {
            layout_with_wire_pipeline(graph, options, elk_options, fixed_layout::layout)
        }
        ElkAlgorithm::Box => {
            layout_with_wire_pipeline(graph, options, elk_options, box_layout::layout)
        }
        ElkAlgorithm::Rectpacking => {
            layout_with_wire_pipeline(graph, options, elk_options, rectpacking::layout)
        }
        ElkAlgorithm::Random => layout_with_wire_pipeline(
            graph,
            options,
            elk_options,
            layout_elk_graph_with_random_pipeline,
        ),
        ElkAlgorithm::Force => layout_with_wire_pipeline(
            graph,
            options,
            elk_options,
            layout_elk_graph_with_force_pipeline,
        ),
        ElkAlgorithm::Stress => layout_with_wire_pipeline(
            graph,
            options,
            elk_options,
            layout_elk_graph_with_stress_pipeline,
        ),
        ElkAlgorithm::SporeOverlap => layout_with_wire_pipeline(
            graph,
            options,
            elk_options,
            layout_elk_graph_with_spore_overlap_pipeline,
        ),
        ElkAlgorithm::SporeCompaction => layout_with_wire_pipeline(
            graph,
            options,
            elk_options,
            layout_elk_graph_with_spore_compaction_pipeline,
        ),
        ElkAlgorithm::MrTree => layout_with_wire_pipeline(
            graph,
            options,
            elk_options,
            layout_elk_graph_with_mrtree_pipeline,
        ),
        ElkAlgorithm::Radial => layout_with_wire_pipeline(
            graph,
            options,
            elk_options,
            layout_elk_graph_with_radial_pipeline,
        ),
    }
}

fn validate_options(options: ElkOptions) -> Result<(), LayoutError> {
    for (name, value) in [
        ("node_spacing", options.node_spacing),
        ("edge_node_spacing", options.edge_node_spacing),
        ("padding", options.padding),
        ("self_loop_spacing", options.self_loop_spacing),
    ] {
        if !value.is_finite() || value < 0.0 {
            return Err(LayoutError::BadRouteState(format!(
                "ELK option {name} must be finite and non-negative"
            )));
        }
    }
    Ok(())
}

/// Routes graph edges through the in-process ELK compatibility adapter.
pub fn route_edges(graph: &Graph, options: &LayoutOptions) -> Result<LayoutResult, LayoutError> {
    route_edges_with_options(graph, options, ElkOptions::default())
}

/// Routes graph edges with explicit ELK adapter options.
pub fn route_edges_with_options(
    graph: &Graph,
    options: &LayoutOptions,
    elk_options: ElkOptions,
) -> Result<LayoutResult, LayoutError> {
    validate_options(elk_options)?;
    let (prepared, prepared_options) = prepare_layered_graph(graph, elk_options);
    let mut result = d2_rs_dagre::route_edges_selected_with_options(
        &prepared,
        options,
        None,
        d2_rs_dagre::DagreOptions {
            node_sep: prepared_options.node_spacing,
            edge_sep: prepared_options.edge_node_spacing,
            rank_sep: None,
            self_loop_spacing: prepared_options.self_loop_spacing,
        },
    )?;
    apply_table_ports(graph, &mut result);
    result.report.warnings.insert(
        0,
        "ELK routes with the in-process deterministic adapter; exact elk-go parity is pending."
            .to_owned(),
    );
    Ok(result)
}

/// Routes only the requested edges through the ELK compatibility adapter.
pub fn route_edges_selected(
    graph: &Graph,
    options: &LayoutOptions,
    edges: &[EdgeId],
) -> Result<LayoutResult, LayoutError> {
    let (prepared, prepared_options) = prepare_layered_graph(graph, ElkOptions::default());
    let mut result = d2_rs_dagre::route_edges_selected_with_options(
        &prepared,
        options,
        Some(edges),
        d2_rs_dagre::DagreOptions {
            self_loop_spacing: prepared_options.self_loop_spacing,
            ..d2_rs_dagre::DagreOptions::default()
        },
    )?;
    apply_table_ports(graph, &mut result);
    result.report.warnings.insert(
        0,
        "ELK baseline routes selected edges with deterministic layered geometry; exact elk-go parity is pending."
            .to_owned(),
    );
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::{
        ElkAlgorithm, ElkEdge, ElkGraph, ElkLabel, ElkNode, ElkOptions, ElkPort,
        edge_container_ids, graph_to_elk_graph, layout, layout_elk_graph_with_layered_pipeline,
        layout_with_layered_pipeline, layout_with_options, prepare_layered_graph,
        route_edges_selected, route_edges_with_options,
    };
    use d2_rs_core::{
        Direction, Edge, EdgeId, EdgeLabel, Graph, LabelPosition, LayoutOptions, Node, Point, Size,
    };
    use serde_json::json;
    use std::collections::BTreeMap;

    #[test]
    fn algorithm_lookup_preserves_source_suffix_and_error_contract() {
        for (name, expected) in [
            ("layered", ElkAlgorithm::Layered),
            ("fixed", ElkAlgorithm::Fixed),
            ("box", ElkAlgorithm::Box),
            ("rectpacking", ElkAlgorithm::Rectpacking),
            ("random", ElkAlgorithm::Random),
        ] {
            for prefix in ["", "elk.", "eclipse.elk.", "org.eclipse.elk."] {
                assert_eq!(
                    ElkAlgorithm::parse(&format!("{prefix}{name}")).unwrap(),
                    expected
                );
            }
        }
        for name in [
            "unknown", "Layered", " layered", "layered ", "ayered", "elk", "",
        ] {
            assert!(
                matches!(ElkAlgorithm::parse(name), Err(d2_rs_core::LayoutError::ElkAlgorithmNotFound { algorithm, path }) if algorithm == name && path.is_empty())
            );
        }
    }

    #[test]
    fn configured_algorithm_resolution_checks_layout_content_and_wraps_source_error() {
        let mut graph = Graph::default();
        graph.layout_engine_options.elk_algorithm = Some("unknown".into());
        assert!(layout(&graph, &LayoutOptions::default()).is_ok());
        graph.add_node(Node::new(
            "a",
            Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        let error = layout(&graph, &LayoutOptions::default()).unwrap_err();
        assert_eq!(
            error.to_string(),
            "failed to ELK layout: native ELK layout failed: layout ELK graph: layout algorithm \"unknown\" not found for "
        );
        graph.layout_engine_options.elk_algorithm = Some(String::new());
        assert!(layout(&graph, &LayoutOptions::default()).is_ok());
    }

    #[test]
    fn nested_graph_scopes_follow_elk_go_reverse_breadth_first_order() {
        let leaf = |id: &str| ElkNode {
            edges: Vec::new(),
            id: id.into(),
            x: 0.0,
            y: 0.0,
            width: 20.0,
            height: 20.0,
            children: Vec::new(),
            ports: Vec::new(),
            labels: Vec::new(),
            layout_options: None,
        };
        let container = |id: &str, children| ElkNode {
            edges: Vec::new(),
            id: id.into(),
            x: 0.0,
            y: 0.0,
            width: 40.0,
            height: 40.0,
            children,
            ports: Vec::new(),
            labels: Vec::new(),
            layout_options: None,
        };
        let graph = ElkGraph {
            children_present: false,
            edges_present: false,
            x: None,
            y: None,
            width: None,
            height: None,
            id: "root".into(),
            layout_options: json!({"elk.hierarchyHandling": "INCLUDE_CHILDREN"}),
            children: vec![
                container("A", vec![container("A1", vec![leaf("a")])]),
                container("B", vec![leaf("b")]),
            ],
            edges: Vec::new(),
        };
        let (arena, root) = crate::layered::LayeredArena::from_elk_graph(&graph).unwrap();

        // The source makes a breadth-first list [root, A, B, A1] and reverses
        // it, so the deeper A1 scope runs before B, then A, then root.
        assert_eq!(super::graphs_bottom_up(&arena, root), [2, 3, 1, 0]);
    }

    // Most phase tests below qualify relative layout geometry and predate
    // publishing elk-go's default root envelope. Strip that envelope before
    // checking solver-relative positions; the wire parity fixture separately
    // asserts the public 12px root padding.
    fn strip_default_root_padding(graph: &mut ElkGraph) {
        const PADDING: f64 = 12.0;
        for node in &mut graph.children {
            node.x -= PADDING;
            node.y -= PADDING;
        }
        for edge in &mut graph.edges {
            if edge.container.is_empty() || edge.container == graph.id {
                for section in &mut edge.sections {
                    section.start.x -= PADDING;
                    section.start.y -= PADDING;
                    section.end.x -= PADDING;
                    section.end.y -= PADDING;
                    for point in &mut section.bends {
                        point.x -= PADDING;
                        point.y -= PADDING;
                    }
                }
            }
        }
        if let Some(width) = &mut graph.width {
            *width -= 2.0 * PADDING;
        }
        if let Some(height) = &mut graph.height {
            *height -= 2.0 * PADDING;
        }
    }

    fn strip_default_root_padding_result(result: &mut super::LayoutResult) {
        const PADDING: f64 = 12.0;
        for rectangle in result.boxes.values_mut() {
            rectangle.origin.x -= PADDING;
            rectangle.origin.y -= PADDING;
        }
        for route in result.routes.values_mut() {
            for point in route {
                point.x -= PADDING;
                point.y -= PADDING;
            }
        }
    }

    #[test]
    fn edge_sections_use_the_lowest_common_container_scope() {
        let leaf = |id: &str| ElkNode {
            edges: Vec::new(),
            id: id.into(),
            x: 0.0,
            y: 0.0,
            width: 40.0,
            height: 30.0,
            children: Vec::new(),
            ports: Vec::new(),
            labels: Vec::new(),
            layout_options: None,
        };
        let mut a = leaf("group.a");
        a.ports.push(ElkPort {
            id: "group.a.port".into(),
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
            layout_options: json!({}),
        });
        let edge = |source: &str, target: &str| ElkEdge {
            junction_points: Vec::new(),
            id: "edge".into(),
            sources: vec![source.into()],
            targets: vec![target.into()],
            sections: Vec::new(),
            labels: Vec::new(),
            container: String::new(),
            layout_options: None,
        };
        let graph = ElkGraph {
            children_present: false,
            edges_present: false,
            x: None,
            y: None,
            width: None,
            height: None,
            id: "root".into(),
            layout_options: json!({}),
            children: vec![
                ElkNode {
                    edges: Vec::new(),
                    id: "group".into(),
                    x: 0.0,
                    y: 0.0,
                    width: 100.0,
                    height: 80.0,
                    children: vec![a, leaf("group.b")],
                    ports: Vec::new(),
                    labels: Vec::new(),
                    layout_options: None,
                },
                leaf("outside"),
            ],
            edges: vec![
                edge("group.a", "group.b"),
                edge("group.a.port", "group.b"),
                edge("group.a", "outside"),
                edge("group", "outside"),
                edge("group.a", "group.a"),
                edge("outside", "outside"),
            ],
        };
        assert_eq!(
            edge_container_ids(&graph, &graph.edges),
            ["group", "group", "root", "root", "group", "root"]
        );
    }

    #[test]
    fn p3_repeated_random_hierarchy_matches_elk_go_bk_selection_and_routing() {
        let mut graph: ElkGraph = serde_json::from_str(include_str!(
            "../../../tests/elk/d2-p3-repeated-random-trial-hierarchy.json"
        ))
        .unwrap();
        let elk_defaults = ElkOptions {
            node_spacing: 20.0,
            edge_node_spacing: 10.0,
            ..ElkOptions::default()
        };
        layout_elk_graph_with_layered_pipeline(&mut graph, elk_defaults).unwrap();

        assert_eq!((graph.width, graph.height), (Some(104.0), Some(209.0)));
        assert_eq!(
            graph
                .children
                .iter()
                .map(|node| (node.id.as_str(), node.x, node.y))
                .collect::<Vec<_>>(),
            vec![
                ("n0", 12.0, 117.0),
                ("n1", 12.0, 67.0),
                ("n2", 12.0, 17.0),
                ("n3", 12.0, 167.0),
                ("n4", 62.0, 112.0),
                ("n5", 62.0, 162.0),
                ("n6", 62.0, 62.0),
                ("n7", 62.0, 12.0),
            ]
        );
        assert_eq!(
            graph
                .edges
                .iter()
                .map(|edge| {
                    let section = &edge.sections[0];
                    let mut points = vec![(section.start.x, section.start.y)];
                    points.extend(section.bends.iter().map(|point| (point.x, point.y)));
                    points.push((section.end.x, section.end.y));
                    (edge.id.as_str(), points)
                })
                .collect::<Vec<_>>(),
            vec![
                ("e0", vec![(42.0, 127.0), (62.0, 127.0)]),
                (
                    "e1",
                    vec![(42.0, 137.0), (52.0, 137.0), (52.0, 172.0), (62.0, 172.0)]
                ),
                (
                    "e2",
                    vec![(42.0, 37.0), (52.0, 37.0), (52.0, 77.0), (62.0, 77.0)]
                ),
                ("e3", vec![(42.0, 27.0), (62.0, 27.0)]),
                ("e4", vec![(42.0, 182.0), (62.0, 182.0)]),
            ]
        );
    }

    #[cfg(feature = "diagnostic-traces")]
    #[test]
    fn diagnostic_pipeline_emits_intermediate_phase_snapshots() {
        let mut graph = ElkGraph {
            children_present: false,
            edges_present: false,
            x: None,
            y: None,
            width: None,
            height: None,
            id: "root".into(),
            layout_options: json!({"elk.algorithm": "layered", "elk.direction": "RIGHT"}),
            children: vec![ElkNode {
                edges: Vec::new(),
                id: "a".into(),
                x: 0.0,
                y: 0.0,
                width: 20.0,
                height: 20.0,
                children: Vec::new(),
                ports: Vec::new(),
                labels: Vec::new(),
                layout_options: None,
            }],
            edges: Vec::new(),
        };
        let events =
            super::layout_elk_graph_with_layered_pipeline_trace(&mut graph, ElkOptions::default())
                .unwrap();
        assert!(events.iter().any(|event| event.phase == "layering"));
        assert!(
            events
                .iter()
                .any(|event| event.path.starts_with("arena.nodes[0]"))
        );
        assert!(
            events
                .iter()
                .any(|event| event.path == "graph.width" && event.float_bits.is_some())
        );
    }

    #[cfg(feature = "diagnostic-traces")]
    #[test]
    fn p3_schedules_independent_scopes_bottom_up_with_source_rng_draws() {
        let mut graph: ElkGraph = serde_json::from_str(include_str!(
            "../../../tests/elk/d2-hierarchy-balanced-bk-external-port.json"
        ))
        .unwrap();
        let events =
            super::layout_elk_graph_with_layered_pipeline_trace(&mut graph, ElkOptions::default())
                .unwrap();

        let sweep_order = events
            .iter()
            .filter(|event| event.operation == "p3_graph_sweep_start")
            .map(|event| event.path.as_str())
            .collect::<Vec<_>>();
        assert_eq!(sweep_order, ["graphs[1]", "graphs[0]"]);

        let layer_starts = events
            .iter()
            .filter(|event| event.operation == "p3_layer_start")
            .map(|event| event.value.as_deref().unwrap_or_default())
            .collect::<Vec<_>>();
        assert_eq!(
            layer_starts,
            [
                "graph_index=1;layer=1;randomized=true;forward=false",
                "graph_index=1;layer=0;randomized=false;forward=false",
            ]
        );

        let rng = events
            .iter()
            .filter(|event| {
                event.phase == "rng"
                    && event.operation != "set_seed"
                    && matches!(
                        event.operation.as_str(),
                        "next_long" | "next_boolean" | "next_double" | "next_float"
                    )
            })
            .map(|event| {
                (
                    event.operation.as_str(),
                    event.draw_count,
                    event.value.as_deref(),
                    event.float_bits,
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            rng,
            [
                ("next_long", Some(2), Some("-4964420948893066024"), None),
                ("next_boolean", Some(3), Some("false"), None),
                ("next_boolean", Some(1), Some("true"), None),
                ("next_boolean", Some(1), Some("false"), None),
                ("next_double", Some(3), None, Some(0x3fb9_b89c_da3e_c398)),
                ("next_float", Some(4), None, Some(0x3ed0_9bf2)),
                ("next_boolean", Some(1), Some("false"), None),
            ]
        );
    }

    #[cfg(feature = "diagnostic-traces")]
    #[test]
    fn p3_bottom_up_child_crossing_matches_source_order_and_geometry() {
        let mut graph: ElkGraph = serde_json::from_str(include_str!(
            "../../../tests/elk/d2-p3-independent-nested-crossing.json"
        ))
        .unwrap();
        let events =
            super::layout_elk_graph_with_layered_pipeline_trace(&mut graph, ElkOptions::default())
                .unwrap();

        let sweep_order = events
            .iter()
            .filter(|event| event.operation == "p3_graph_sweep_start")
            .map(|event| event.path.as_str())
            .collect::<Vec<_>>();
        assert_eq!(sweep_order, ["graphs[1]", "graphs[0]"]);
        let layer_starts = events
            .iter()
            .filter(|event| event.operation == "p3_layer_start")
            .map(|event| event.value.as_deref().unwrap_or_default())
            .collect::<Vec<_>>();
        assert_eq!(
            layer_starts,
            [
                "graph_index=1;layer=1;randomized=true;forward=false",
                "graph_index=1;layer=0;randomized=false;forward=false",
                "graph_index=0;layer=0;randomized=true;forward=false",
            ]
        );
        let children = &graph.children[0].children;
        assert_eq!(
            children
                .iter()
                .map(|node| (node.x, node.y))
                .collect::<Vec<_>>(),
            [(12.0, 12.0), (12.0, 62.0), (62.0, 62.0), (62.0, 12.0)]
        );
    }

    #[test]
    fn layered_pipeline_publishes_sections_without_the_dagre_adapter() {
        let mut graph = Graph::default();
        let source = graph.add_node(Node::new(
            "a",
            Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        let target = graph.add_node(Node::new(
            "b",
            Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        graph.add_edge(Edge { source, target });
        let mut elk = graph_to_elk_graph(&graph, &BTreeMap::new(), ElkOptions::default());
        layout_elk_graph_with_layered_pipeline(&mut elk, ElkOptions::default()).unwrap();
        assert_eq!(elk.children.len(), 2);
        assert_eq!(elk.edges[0].sections.len(), 1);
        assert!(elk.edges[0].sections[0].start.y < elk.edges[0].sections[0].end.y);
        let section = &elk.edges[0].sections[0];
        assert_eq!(section.id.as_deref(), Some("(a -- b)[0]_s0"));
        assert_eq!(section.incoming_shape.as_deref(), Some("a"));
        assert_eq!(section.outgoing_shape.as_deref(), Some("b"));
        assert!(elk.edges[0].container.is_empty());
    }

    #[test]
    fn disconnected_components_match_v090_simple_row_packing() {
        let nodes = (b'a'..=b'g')
            .map(|id| ElkNode {
                edges: Vec::new(),
                id: (id as char).to_string(),
                x: 0.0,
                y: 0.0,
                width: 20.0,
                height: 20.0,
                children: Vec::new(),
                ports: Vec::new(),
                labels: Vec::new(),
                layout_options: None,
            })
            .collect::<Vec<_>>();
        let mut graph = ElkGraph {
            children_present: false,
            edges_present: false,
            x: None,
            y: None,
            width: None,
            height: None,
            id: "root".into(),
            layout_options: json!({
                "elk.algorithm": "layered",
                "elk.direction": "RIGHT",
                "spacing.nodeNodeBetweenLayers": 20,
                "spacing.edgeNodeBetweenLayers": 10,
                "elk.layered.spacing.edgeEdgeBetweenLayers": 50,
                "elk.padding": "[top=0,left=0,bottom=0,right=0]"
            }),
            children: nodes,
            edges: vec![ElkEdge {
                junction_points: Vec::new(),
                id: "e0".into(),
                sources: vec!["a".into()],
                targets: vec!["f".into()],
                sections: Vec::new(),
                labels: Vec::new(),
                container: String::new(),
                layout_options: None,
            }],
        };
        layout_elk_graph_with_layered_pipeline(
            &mut graph,
            ElkOptions {
                node_spacing: 20.0,
                edge_node_spacing: 10.0,
                ..ElkOptions::default()
            },
        )
        .unwrap();
        assert_eq!((graph.width, graph.height), (Some(60.0), Some(140.0)));
        let positions = graph
            .children
            .iter()
            .map(|node| (node.id.as_str(), node.x, node.y))
            .collect::<Vec<_>>();
        assert_eq!(
            positions,
            vec![
                ("a", 0.0, 120.0),
                ("b", 0.0, 0.0),
                ("c", 40.0, 0.0),
                ("d", 0.0, 40.0),
                ("e", 40.0, 40.0),
                ("f", 40.0, 120.0),
                ("g", 0.0, 80.0),
            ]
        );
        let section = &graph.edges[0].sections[0];
        assert_eq!((section.start.x, section.start.y), (20.0, 130.0));
        assert_eq!((section.end.x, section.end.y), (40.0, 130.0));
    }

    #[test]
    fn disconnected_randomized_components_match_v090_geometry_and_routes() {
        let mut graph: ElkGraph = serde_json::from_str(include_str!(
            "../../../tests/elk/d2-p3-repeated-random-trial.json"
        ))
        .unwrap();
        layout_elk_graph_with_layered_pipeline(
            &mut graph,
            ElkOptions {
                node_spacing: 20.0,
                edge_node_spacing: 10.0,
                ..ElkOptions::default()
            },
        )
        .unwrap();

        assert_eq!((graph.width, graph.height), (Some(154.0), Some(209.0)));
        assert_eq!(
            graph
                .children
                .iter()
                .map(|node| (node.id.as_str(), node.x, node.y))
                .collect::<Vec<_>>(),
            vec![
                ("n0", 12.0, 117.0),
                ("n1", 12.0, 12.0),
                ("n2", 62.0, 17.0),
                ("n3", 12.0, 167.0),
                ("n4", 62.0, 112.0),
                ("n5", 62.0, 162.0),
                ("n6", 112.0, 12.0),
                ("n7", 112.0, 62.0),
            ]
        );
        let edge_routes = graph
            .edges
            .iter()
            .map(|edge| {
                let section = &edge.sections[0];
                let mut points = vec![(section.start.x, section.start.y)];
                points.extend(section.bends.iter().map(|point| (point.x, point.y)));
                points.push((section.end.x, section.end.y));
                (edge.id.as_str(), points)
            })
            .collect::<Vec<_>>();
        assert_eq!(
            edge_routes,
            vec![
                ("e0", vec![(42.0, 127.0), (62.0, 127.0)]),
                (
                    "e1",
                    vec![(42.0, 137.0), (52.0, 137.0), (52.0, 172.0), (62.0, 172.0)]
                ),
                ("e2", vec![(92.0, 27.0), (112.0, 27.0)]),
                (
                    "e3",
                    vec![(92.0, 37.0), (102.0, 37.0), (102.0, 77.0), (112.0, 77.0)]
                ),
                ("e4", vec![(42.0, 182.0), (62.0, 182.0)]),
            ]
        );
    }

    #[test]
    fn layered_pipeline_self_loop_matches_elk_source_shape() {
        let close = |actual: f64, expected: f64| {
            assert!((actual - expected).abs() < 1e-9, "{actual} != {expected}");
        };
        for (direction, expected_node, expected_route) in [
            (
                "RIGHT",
                (0.0, 50.0),
                [
                    (13.333333333333334, 50.0),
                    (13.333333333333334, 0.0),
                    (26.666666666666668, 0.0),
                    (26.666666666666668, 50.0),
                ],
            ),
            (
                "DOWN",
                (50.0, 0.0),
                [(50.0, 10.0), (0.0, 10.0), (0.0, 20.0), (50.0, 20.0)],
            ),
            (
                "UP",
                (50.0, 0.0),
                [(50.0, 20.0), (0.0, 20.0), (0.0, 10.0), (50.0, 10.0)],
            ),
            (
                "LEFT",
                (0.0, 50.0),
                [
                    (26.666666666666668, 50.0),
                    (26.666666666666668, 0.0),
                    (13.333333333333334, 0.0),
                    (13.333333333333334, 50.0),
                ],
            ),
        ] {
            let mut graph = ElkGraph {
                children_present: false,
                edges_present: false,
                x: None,
                y: None,
                width: None,
                height: None,
                id: "root".into(),
                layout_options: json!({
                    "elk.algorithm": "layered",
                    "elk.direction": direction,
                    "elk.hierarchyHandling": "INCLUDE_CHILDREN",
                    "elk.nodeSize.constraints": "MINIMUM_SIZE",
                    "elk.contentAlignment": "H_CENTER V_CENTER",
                    "elk.spacing.nodeNodeBetweenLayers": 20,
                    "elk.spacing.edgeNodeBetweenLayers": 10,
                    "elk.layered.spacing.edgeEdgeBetweenLayers": 50,
                    "elk.spacing.nodeSelfLoop": 50,
                }),
                children: vec![ElkNode {
                    edges: Vec::new(),
                    id: "A".into(),
                    x: 0.0,
                    y: 0.0,
                    width: 40.0,
                    height: 30.0,
                    children: Vec::new(),
                    ports: Vec::new(),
                    labels: Vec::new(),
                    layout_options: None,
                }],
                edges: vec![ElkEdge {
                    junction_points: Vec::new(),
                    id: "loop".into(),
                    sources: vec!["A".into()],
                    targets: vec!["A".into()],
                    sections: Vec::new(),
                    labels: Vec::new(),
                    container: "root".into(),
                    layout_options: None,
                }],
            };
            layout_elk_graph_with_layered_pipeline(&mut graph, ElkOptions::default()).unwrap();
            assert_eq!(graph.edges[0].container, "root");
            strip_default_root_padding(&mut graph);
            close(graph.children[0].x, expected_node.0);
            close(graph.children[0].y, expected_node.1);
            let section = &graph.edges[0].sections[0];
            let actual = std::iter::once((section.start.x, section.start.y))
                .chain(section.bends.iter().map(|point| (point.x, point.y)))
                .chain(std::iter::once((section.end.x, section.end.y)))
                .collect::<Vec<_>>();
            assert_eq!(actual.len(), expected_route.len());
            for ((actual_x, actual_y), (expected_x, expected_y)) in
                actual.into_iter().zip(expected_route)
            {
                close(actual_x, expected_x);
                close(actual_y, expected_y);
            }
        }
    }

    #[test]
    fn north_south_loops_reserve_only_north_lanes_above_node() {
        let mut graph: ElkGraph = serde_json::from_value(json!({
            "id": "root",
            "layoutOptions": {"elk.algorithm": "layered", "elk.direction": "RIGHT", "elk.spacing.nodeSelfLoop": 50},
            "children": [{"id": "A", "x": 0, "y": 0, "width": 40, "height": 30,
                "layoutOptions": {"elk.layered.edgeRouting.selfLoopDistribution": "NORTH_SOUTH"}}],
            "edges": (0..4).map(|i| json!({"id": format!("e{i}"), "sources": ["A"], "targets": ["A"], "container": "root"})).collect::<Vec<_>>()
        })).unwrap();
        layout_elk_graph_with_layered_pipeline(&mut graph, ElkOptions::default()).unwrap();
        assert!(graph.edges.iter().all(|edge| edge.container == "root"));
        strip_default_root_padding(&mut graph);
        // elk-go v0.2.0, Go 1.27.0: root padding (12,12) removed.
        assert_eq!((graph.children[0].x, graph.children[0].y), (0.0, 60.0));
        let expected = [
            [(16.0, 60.0), (16.0, 10.0), (24.0, 10.0), (24.0, 60.0)],
            [(24.0, 90.0), (24.0, 140.0), (16.0, 140.0), (16.0, 90.0)],
            [(8.0, 60.0), (8.0, 0.0), (32.0, 0.0), (32.0, 60.0)],
            [(32.0, 90.0), (32.0, 150.0), (8.0, 150.0), (8.0, 90.0)],
        ];
        for (edge, expected) in graph.edges.iter().zip(expected) {
            let section = &edge.sections[0];
            let actual = std::iter::once(&section.start)
                .chain(&section.bends)
                .chain(std::iter::once(&section.end))
                .map(|point| (point.x, point.y))
                .collect::<Vec<_>>();
            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn layered_pipeline_multiple_self_loops_matches_elk_slot_order() {
        let mut graph = ElkGraph {
            children_present: false,
            edges_present: false,
            x: None,
            y: None,
            width: None,
            height: None,
            id: "root".into(),
            layout_options: json!({
                "elk.algorithm": "layered",
                "elk.direction": "RIGHT",
                "elk.hierarchyHandling": "INCLUDE_CHILDREN",
                "elk.spacing.nodeSelfLoop": 50,
            }),
            children: vec![ElkNode {
                edges: Vec::new(),
                id: "A".into(),
                x: 0.0,
                y: 0.0,
                width: 40.0,
                height: 30.0,
                children: Vec::new(),
                ports: Vec::new(),
                labels: Vec::new(),
                layout_options: None,
            }],
            edges: (0..2)
                .map(|index| ElkEdge {
                    junction_points: Vec::new(),
                    id: format!("loop{index}"),
                    sources: vec!["A".into()],
                    targets: vec!["A".into()],
                    sections: Vec::new(),
                    labels: Vec::new(),
                    container: "root".into(),
                    layout_options: None,
                })
                .collect(),
        };
        layout_elk_graph_with_layered_pipeline(&mut graph, ElkOptions::default()).unwrap();
        strip_default_root_padding(&mut graph);
        assert_eq!((graph.children[0].x, graph.children[0].y), (0.0, 60.0));
        let expected = [
            [(16.0, 60.0), (16.0, 10.0), (24.0, 10.0), (24.0, 60.0)],
            [(8.0, 60.0), (8.0, 0.0), (32.0, 0.0), (32.0, 60.0)],
        ];
        for (section, expected) in graph
            .edges
            .iter()
            .map(|edge| &edge.sections[0])
            .zip(expected)
        {
            let actual = std::iter::once((section.start.x, section.start.y))
                .chain(section.bends.iter().map(|point| (point.x, point.y)))
                .chain(std::iter::once((section.end.x, section.end.y)))
                .collect::<Vec<_>>();
            for ((actual_x, actual_y), (expected_x, expected_y)) in actual.into_iter().zip(expected)
            {
                assert!((actual_x - expected_x).abs() < 1e-9);
                assert!((actual_y - expected_y).abs() < 1e-9);
            }
        }
    }

    #[test]
    #[cfg(feature = "oracle-fixtures")]
    fn inline_center_label_hierarchy_matches_go_v090_output_exactly() {
        let mut actual: ElkGraph = serde_json::from_str(include_str!(
            "../../../tests/elk/inline-center-label-minimum-size-wire.json"
        ))
        .unwrap();
        let expected: ElkGraph = serde_json::from_str(d2_rs_testdata::load_str(
            concat!(env!("CARGO_MANIFEST_DIR"), "/../.."),
            "tests/elk/inline-center-label-minimum-size-go-v090.json",
        ))
        .unwrap();
        let options = ElkOptions {
            node_spacing: 20.0,
            edge_node_spacing: 10.0,
            ..ElkOptions::default()
        };

        layout_elk_graph_with_layered_pipeline(&mut actual, options).unwrap();

        fn point(x: f64, y: f64) -> [u64; 2] {
            [x.to_bits(), y.to_bits()]
        }
        fn node_geometry(nodes: &[ElkNode], records: &mut Vec<serde_json::Value>) {
            for node in nodes {
                records.push(serde_json::json!({
                    "id": node.id,
                    "box": [node.x.to_bits(), node.y.to_bits(), node.width.to_bits(), node.height.to_bits()],
                    "labels": node.labels.iter().map(|label| serde_json::json!([
                        label.text, label.x.to_bits(), label.y.to_bits(),
                        label.width.to_bits(), label.height.to_bits()
                    ])).collect::<Vec<_>>(),
                }));
                node_geometry(&node.children, records);
            }
        }
        fn edge_geometry(edges: &[ElkEdge]) -> Vec<serde_json::Value> {
            edges
                .iter()
                .map(|edge| {
                    serde_json::json!({
                        "id": edge.id,
                        "container": edge.container,
                        "sources": edge.sources,
                        "targets": edge.targets,
                        "sections": edge.sections.iter().map(|section| serde_json::json!({
                            "id": section.id,
                            "start": point(section.start.x, section.start.y),
                            "end": point(section.end.x, section.end.y),
                            "bends": section.bends.iter().map(|bend| point(bend.x, bend.y)).collect::<Vec<_>>(),
                            "incoming": section.incoming_shape,
                            "outgoing": section.outgoing_shape,
                        })).collect::<Vec<_>>(),
                        "labels": edge.labels.iter().map(|label| serde_json::json!([
                            label.text, label.x.to_bits(), label.y.to_bits(),
                            label.width.to_bits(), label.height.to_bits()
                        ])).collect::<Vec<_>>(),
                    })
                })
                .collect()
        }
        assert_eq!(
            actual.width.unwrap().to_bits(),
            expected.width.unwrap().to_bits()
        );
        assert_eq!(
            actual.height.unwrap().to_bits(),
            expected.height.unwrap().to_bits()
        );
        let mut actual_nodes = Vec::new();
        let mut expected_nodes = Vec::new();
        node_geometry(&actual.children, &mut actual_nodes);
        node_geometry(&expected.children, &mut expected_nodes);
        assert_eq!(actual_nodes, expected_nodes);
        assert_eq!(edge_geometry(&actual.edges), edge_geometry(&expected.edges));
    }

    #[test]
    fn center_edge_labels_reserve_the_pinned_elk_rank_lane() {
        let chain = |with_labels: bool| ElkGraph {
            children_present: false,
            edges_present: false,
            x: None,
            y: None,
            width: None,
            height: None,
            id: "root".into(),
            layout_options: json!({
                "elk.algorithm": "layered",
                "elk.direction": "DOWN",
                "elk.hierarchyHandling": "INCLUDE_CHILDREN",
                "spacing.nodeNodeBetweenLayers": 70,
                "elk.layered.spacing.edgeEdgeBetweenLayers": 50,
                "spacing.edgeNodeBetweenLayers": 40,
                "elk.spacing.nodeSelfLoop": 50,
            }),
            children: ["a", "b", "c"]
                .into_iter()
                .map(|id| ElkNode {
                    edges: Vec::new(),
                    id: id.into(),
                    x: 0.0,
                    y: 0.0,
                    width: 53.0,
                    height: 66.0,
                    children: Vec::new(),
                    ports: Vec::new(),
                    labels: Vec::new(),
                    layout_options: None,
                })
                .collect(),
            edges: [
                ("ab", "a", "b", "label", 37.0),
                ("bc", "b", "c", "another label", 93.0),
            ]
            .into_iter()
            .map(|(id, source, target, text, width)| ElkEdge {
                junction_points: Vec::new(),
                id: id.into(),
                sources: vec![source.into()],
                targets: vec![target.into()],
                sections: Vec::new(),
                labels: if with_labels {
                    vec![ElkLabel {
                        id: None,
                        text: text.into(),
                        x: 0.0,
                        y: 0.0,
                        width,
                        height: 21.0,
                        layout_options: Some(json!({"elk.edgeLabels.inline": true})),
                    }]
                } else {
                    Vec::new()
                },
                container: "root".into(),
                layout_options: None,
            })
            .collect(),
        };

        let mut labeled = chain(true);
        layout_elk_graph_with_layered_pipeline(&mut labeled, ElkOptions::default()).unwrap();
        let labeled_intervals = [
            labeled.children[1].y - labeled.children[0].y,
            labeled.children[2].y - labeled.children[1].y,
        ];
        assert_eq!(labeled_intervals, [227.0, 227.0]);
        for (edge, source) in labeled.edges.iter().zip(&labeled.children[..2]) {
            let label = &edge.labels[0];
            assert_eq!(label.y - source.y - source.height, 70.0);
            assert_eq!(label.x + label.width / 2.0, source.x + source.width / 2.0);
        }

        let mut unlabeled = chain(false);
        layout_elk_graph_with_layered_pipeline(&mut unlabeled, ElkOptions::default()).unwrap();
        assert_eq!(unlabeled.children[1].y - unlabeled.children[0].y, 136.0);
        assert_eq!(unlabeled.children[2].y - unlabeled.children[1].y, 136.0);
    }

    #[test]
    fn layered_pipeline_self_loop_label_uses_flow_axis_clearance() {
        for (direction, expected_node, expected_label) in [
            ("RIGHT", (12.0, 74.0), (22.0, 12.0)),
            ("DOWN", (84.0, 12.0), (12.0, 22.0)),
            ("UP", (84.0, 12.0), (12.0, 22.0)),
            ("LEFT", (12.0, 74.0), (22.0, 12.0)),
        ] {
            let mut graph = ElkGraph {
                children_present: false,
                edges_present: false,
                x: None,
                y: None,
                width: None,
                height: None,
                id: "root".into(),
                layout_options: json!({
                    "elk.algorithm": "layered",
                    "elk.direction": direction,
                    "elk.spacing.nodeSelfLoop": 50,
                }),
                children: vec![ElkNode {
                    edges: Vec::new(),
                    id: "A".into(),
                    x: 0.0,
                    y: 0.0,
                    width: 40.0,
                    height: 30.0,
                    children: Vec::new(),
                    ports: Vec::new(),
                    labels: Vec::new(),
                    layout_options: None,
                }],
                edges: vec![ElkEdge {
                    junction_points: Vec::new(),
                    id: "loop".into(),
                    sources: vec!["A".into()],
                    targets: vec!["A".into()],
                    sections: Vec::new(),
                    labels: vec![ElkLabel {
                        id: None,
                        text: "x".into(),
                        x: 0.0,
                        y: 0.0,
                        width: 20.0,
                        height: 10.0,
                        layout_options: None,
                    }],
                    container: "root".into(),
                    layout_options: None,
                }],
            };
            layout_elk_graph_with_layered_pipeline(&mut graph, ElkOptions::default()).unwrap();
            // Original Go transferrer adds root padding to labels as well.
            assert_eq!((graph.children[0].x, graph.children[0].y), expected_node);
            assert_eq!(
                (graph.edges[0].labels[0].x, graph.edges[0].labels[0].y),
                expected_label
            );
        }
    }

    #[test]
    fn opposing_inline_self_loop_labels_match_go_v090_slots() {
        let mut graph: ElkGraph = serde_json::from_str(include_str!(
            "../../../tests/elk/sql-table-inline-self-loop-labels-wire.json"
        ))
        .unwrap();
        for node in &mut graph.children {
            let options = node
                .layout_options
                .get_or_insert_with(|| serde_json::json!({}));
            for key in [
                "d2-rs.marginTop",
                "d2-rs.marginRight",
                "d2-rs.marginBottom",
                "d2-rs.marginLeft",
            ] {
                options[key] = serde_json::json!(0.0);
            }
        }

        layout_elk_graph_with_layered_pipeline(&mut graph, ElkOptions::default()).unwrap();

        assert_eq!(
            graph
                .children
                .iter()
                .map(|node| (node.id.as_str(), node.x, node.y))
                .collect::<Vec<_>>(),
            [
                ("User", 374.0, 110.0),
                ("Pet", 12.0, 573.0),
                ("Card", 225.0, 573.0),
                ("Post", 438.0, 573.0),
                ("Metadata", 680.0, 573.0),
                ("Info", 845.0, 573.0),
            ]
        );

        for (edge_id, expected_label, expected_bends) in [
            (
                "(User <-> User)[0]",
                (238.0, 171.5),
                [
                    (409.0, 342.0),
                    (262.0, 342.0),
                    (262.0, 22.0),
                    (467.3333333333333, 22.0),
                ],
            ),
            (
                "(User <-> User)[1]",
                (59.0, 171.5),
                [
                    (444.0, 352.0),
                    (142.5, 352.0),
                    (142.5, 12.0),
                    (560.6666666666666, 12.0),
                ],
            ),
        ] {
            let edge = graph.edges.iter().find(|edge| edge.id == edge_id).unwrap();
            let label = &edge.labels[0];
            assert_eq!((label.x, label.y), expected_label, "{edge_id} label");
            assert_eq!(
                edge.sections[0]
                    .bends
                    .iter()
                    .map(|point| (point.x, point.y))
                    .collect::<Vec<_>>(),
                expected_bends,
                "{edge_id} route"
            );
        }
    }

    #[test]
    fn layered_pipeline_fixed_north_ports_use_self_loop_router() {
        let mut graph = ElkGraph {
            children_present: false,
            edges_present: false,
            x: None,
            y: None,
            width: None,
            height: None,
            id: "root".into(),
            layout_options: json!({
                "elk.algorithm": "layered",
                "elk.direction": "RIGHT",
                "elk.spacing.nodeSelfLoop": 50,
            }),
            children: vec![ElkNode {
                edges: Vec::new(),
                id: "A".into(),
                x: 0.0,
                y: 0.0,
                width: 40.0,
                height: 30.0,
                children: Vec::new(),
                ports: vec![
                    ElkPort {
                        id: "p1".into(),
                        x: 0.0,
                        y: 0.0,
                        width: 0.0,
                        height: 0.0,
                        layout_options: json!({"elk.port.side": "NORTH"}),
                    },
                    ElkPort {
                        id: "p2".into(),
                        x: 0.0,
                        y: 0.0,
                        width: 0.0,
                        height: 0.0,
                        layout_options: json!({"elk.port.side": "NORTH"}),
                    },
                ],
                labels: Vec::new(),
                layout_options: None,
            }],
            edges: vec![ElkEdge {
                junction_points: Vec::new(),
                id: "loop".into(),
                sources: vec!["p1".into()],
                targets: vec!["p2".into()],
                sections: Vec::new(),
                labels: Vec::new(),
                container: "root".into(),
                layout_options: None,
            }],
        };
        layout_elk_graph_with_layered_pipeline(&mut graph, ElkOptions::default()).unwrap();
        strip_default_root_padding(&mut graph);
        assert_eq!((graph.children[0].x, graph.children[0].y), (0.0, 50.0));
        assert!((graph.children[0].ports[0].x - 13.333333333333334).abs() < 1e-9);
        assert!((graph.children[0].ports[1].x - 26.666666666666668).abs() < 1e-9);
        let section = &graph.edges[0].sections[0];
        assert!((section.start.x - 13.333333333333334).abs() < 1e-9);
        assert_eq!(section.start.y, 50.0);
        assert!((section.bends[0].x - 13.333333333333334).abs() < 1e-9);
        assert_eq!(section.bends[0].y, 0.0);
        assert!((section.bends[1].x - 26.666666666666668).abs() < 1e-9);
        assert_eq!(section.bends[1].y, 0.0);
        assert!((section.end.x - 26.666666666666668).abs() < 1e-9);
        assert_eq!(section.end.y, 50.0);
    }

    #[test]
    fn nested_self_loop_uses_local_default_clearance_and_resizes_parent() {
        let mut graph = ElkGraph {
            children_present: false,
            edges_present: false,
            x: None,
            y: None,
            width: None,
            height: None,
            id: "root".into(),
            layout_options: json!({
                "elk.algorithm": "layered",
                "elk.direction": "RIGHT",
                "elk.hierarchyHandling": "INCLUDE_CHILDREN",
                "elk.nodeSize.constraints": "MINIMUM_SIZE",
                "elk.contentAlignment": "H_CENTER V_CENTER",
                "elk.spacing.nodeSelfLoop": 50,
            }),
            children: vec![ElkNode {
                edges: Vec::new(),
                id: "A".into(),
                x: 0.0,
                y: 0.0,
                width: 40.0,
                height: 30.0,
                children: vec![ElkNode {
                    edges: Vec::new(),
                    id: "B".into(),
                    x: 0.0,
                    y: 0.0,
                    width: 40.0,
                    height: 30.0,
                    children: Vec::new(),
                    ports: Vec::new(),
                    labels: Vec::new(),
                    layout_options: None,
                }],
                ports: Vec::new(),
                labels: Vec::new(),
                layout_options: None,
            }],
            edges: vec![ElkEdge {
                junction_points: Vec::new(),
                id: "loop".into(),
                sources: vec!["B".into()],
                targets: vec!["B".into()],
                sections: Vec::new(),
                labels: Vec::new(),
                container: "A".into(),
                layout_options: None,
            }],
        };
        layout_elk_graph_with_layered_pipeline(&mut graph, ElkOptions::default()).unwrap();
        strip_default_root_padding(&mut graph);
        let parent = &graph.children[0];
        let child = &parent.children[0];
        assert_eq!(
            (parent.x, parent.y, parent.width, parent.height),
            (0.0, 0.0, 64.0, 64.0)
        );
        assert_eq!((child.x, child.y), (12.0, 22.0));
        let section = &graph.edges[0].sections[0];
        assert_eq!(
            (section.start.x, section.start.y),
            (25.333333333333336, 22.0)
        );
        assert_eq!(
            section
                .bends
                .iter()
                .map(|point| (point.x, point.y))
                .collect::<Vec<_>>(),
            vec![(25.333333333333336, 12.0), (38.66666666666667, 12.0)]
        );
        assert_eq!((section.end.x, section.end.y), (38.66666666666667, 22.0));
    }

    #[test]
    fn public_layered_layout_runs_the_source_pipeline_with_shared_geometry() {
        let mut graph = Graph::default();
        let source = graph.add_node(Node::new(
            "a",
            Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        let target = graph.add_node(Node::new(
            "b",
            Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        graph.add_edge(Edge { source, target });
        let raw =
            layout_with_options(&graph, &LayoutOptions::default(), ElkOptions::default()).unwrap();
        let result = layout(&graph, &LayoutOptions::default()).unwrap();
        assert_eq!(result.boxes.len(), 2);
        assert_eq!(result.routes.len(), 1);
        assert!(result.report.warnings[0].contains("source-shaped"));
        assert_eq!(result.boxes[&source].origin, raw.boxes[&source].origin);
        assert_eq!(result.boxes[&source].origin, Point { x: 12.0, y: 12.0 });
        assert_eq!(
            result.routes[&EdgeId(0)]
                .iter()
                .zip(raw.routes[&EdgeId(0)].iter())
                .map(|(actual, expected)| (actual.x - expected.x, actual.y - expected.y))
                .collect::<Vec<_>>(),
            vec![(0.0, 0.0); raw.routes[&EdgeId(0)].len()]
        );
    }

    #[test]
    fn source_shaped_layered_entrypoint_publishes_native_geometry() {
        let mut graph = Graph::default();
        let source = graph.add_node(Node::new(
            "a",
            Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        let target = graph.add_node(Node::new(
            "b",
            Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        graph.add_edge(Edge { source, target });

        let result = layout_with_layered_pipeline(
            &graph,
            &LayoutOptions::default(),
            ElkOptions {
                node_spacing: 20.0,
                edge_node_spacing: 10.0,
                ..ElkOptions::default()
            },
        )
        .unwrap();
        assert_eq!(result.boxes.len(), 2);
        assert_eq!(result.routes.len(), 1);
        assert!(result.boxes[&source].origin.y < result.boxes[&target].origin.y);
        assert!(result.report.warnings[0].contains("source-shaped"));
    }

    #[test]
    fn unequal_width_chain_matches_v090_bk_centering_and_straight_routes() {
        let node = |id: &str, width: f64, height: f64| ElkNode {
            edges: Vec::new(),
            id: id.into(),
            x: 0.0,
            y: 0.0,
            width,
            height,
            children: Vec::new(),
            ports: Vec::new(),
            labels: Vec::new(),
            layout_options: None,
        };
        let mut graph = ElkGraph {
            children_present: false,
            edges_present: false,
            x: None,
            y: None,
            width: None,
            height: None,
            id: "root".into(),
            layout_options: json!({
                "elk.algorithm": "layered",
                "elk.direction": "RIGHT",
                "spacing.nodeNodeBetweenLayers": 20,
                "spacing.edgeNodeBetweenLayers": 10,
                "elk.spacing.nodeSelfLoop": 10,
                "elk.layered.spacing.edgeEdgeBetweenLayers": 50,
                "elk.padding": "[top=0,left=0,bottom=0,right=0]",
            }),
            children: vec![
                node("a", 30.0, 30.0),
                node("b", 40.0, 20.0),
                node("c", 30.0, 30.0),
            ],
            edges: vec![
                ElkEdge {
                    junction_points: Vec::new(),
                    id: "e1".into(),
                    sources: vec!["a".into()],
                    targets: vec!["b".into()],
                    sections: Vec::new(),
                    labels: Vec::new(),
                    container: String::new(),
                    layout_options: None,
                },
                ElkEdge {
                    junction_points: Vec::new(),
                    id: "e2".into(),
                    sources: vec!["b".into()],
                    targets: vec!["c".into()],
                    sections: Vec::new(),
                    labels: Vec::new(),
                    container: String::new(),
                    layout_options: None,
                },
            ],
        };
        layout_elk_graph_with_layered_pipeline(&mut graph, ElkOptions::default()).unwrap();
        assert_eq!((graph.width, graph.height), (Some(140.0), Some(30.0)));
        assert_eq!(
            graph
                .children
                .iter()
                .map(|node| (node.x, node.y))
                .collect::<Vec<_>>(),
            vec![(0.0, 0.0), (50.0, 5.0), (110.0, 0.0)]
        );
        for edge in &graph.edges {
            let section = &edge.sections[0];
            assert!(section.bends.is_empty(), "{} retained bends", edge.id);
            assert_eq!(section.start.y, 15.0);
            assert_eq!(section.end.y, 15.0);
        }
    }

    #[test]
    fn long_edge_diamond_matches_v090_geometry_and_routes_exactly() {
        let node = |id: &str| ElkNode {
            edges: Vec::new(),
            id: id.into(),
            x: 0.0,
            y: 0.0,
            width: 20.0,
            height: 20.0,
            children: Vec::new(),
            ports: Vec::new(),
            labels: Vec::new(),
            layout_options: None,
        };
        let edge = |id: &str, source: &str, target: &str| ElkEdge {
            junction_points: Vec::new(),
            id: id.into(),
            sources: vec![source.into()],
            targets: vec![target.into()],
            sections: Vec::new(),
            labels: Vec::new(),
            container: String::new(),
            layout_options: None,
        };
        let mut graph = ElkGraph {
            children_present: false,
            edges_present: false,
            x: None,
            y: None,
            width: None,
            height: None,
            id: "root".into(),
            layout_options: json!({
                "elk.algorithm": "layered",
                "elk.direction": "RIGHT",
                "spacing.nodeNodeBetweenLayers": 20,
                "spacing.edgeNodeBetweenLayers": 10,
                "elk.layered.spacing.edgeEdgeBetweenLayers": 50,
                "elk.padding": "[top=0,left=0,bottom=0,right=0]",
            }),
            children: vec![node("a"), node("b"), node("c")],
            edges: vec![
                edge("e1", "a", "b"),
                edge("e2", "b", "c"),
                edge("e3", "a", "c"),
            ],
        };

        layout_elk_graph_with_layered_pipeline(&mut graph, ElkOptions::default()).unwrap();

        assert_eq!((graph.width, graph.height), (Some(100.0), Some(31.0)));
        assert_eq!(
            graph
                .children
                .iter()
                .map(|node| (node.x, node.y))
                .collect::<Vec<_>>(),
            vec![
                (0.0, 7.666666666666668),
                (40.0, 11.0),
                (80.0, 7.666666666666668),
            ]
        );

        for (edge, (start_x, end_x)) in graph.edges[..2].iter().zip([(20.0, 40.0), (60.0, 80.0)]) {
            let section = &edge.sections[0];
            assert!(section.bends.is_empty(), "{} retained bends", edge.id);
            assert_eq!((section.start.x, section.start.y), (start_x, 21.0));
            assert_eq!((section.end.x, section.end.y), (end_x, 21.0));
        }

        let long_edge = &graph.edges[2].sections[0];
        assert_eq!(
            (long_edge.start.x, long_edge.start.y),
            (20.0, 14.333333333333334)
        );
        assert_eq!(
            long_edge
                .bends
                .iter()
                .map(|point| (point.x, point.y))
                .collect::<Vec<_>>(),
            vec![
                (30.0, 14.333333333333334),
                (30.0, 0.0),
                (70.0, 0.0),
                (70.0, 14.333333333333334),
            ]
        );
        assert_eq!(
            (long_edge.end.x, long_edge.end.y),
            (80.0, 14.333333333333334)
        );
    }

    #[test]
    fn model_order_long_edge_crossing_matches_elk_go_v090_exactly() {
        let mut graph: ElkGraph = serde_json::from_value(json!({
            "id": "root",
            "layoutOptions": {
                "elk.algorithm": "layered",
                "elk.direction": "RIGHT",
                "elk.layered.considerModelOrder.strategy": "NODES_AND_EDGES",
                "elk.hierarchyHandling": "INCLUDE_CHILDREN",
                "spacing.nodeNodeBetweenLayers": 70,
                "spacing.edgeNodeBetweenLayers": 40,
                "elk.spacing.nodeSelfLoop": 50,
                "elk.spacing.edgeNode": 40,
                "elk.layered.nodePlacement.bk.fixedAlignment": "BALANCED",
                "elk.layered.thoroughness": 8,
                "elk.layered.spacing.edgeEdgeBetweenLayers": 50,
                "elk.layered.cycleBreaking.strategy": "GREEDY_MODEL_ORDER",
                "elk.nodeSize.constraints": "MINIMUM_SIZE",
                "elk.contentAlignment": "H_CENTER V_CENTER",
                "elk.padding": "[top=0,left=0,bottom=0,right=0]"
            },
            "children": [
                {"id":"A","width":40,"height":30},
                {"id":"B","width":40,"height":30},
                {"id":"X","width":40,"height":30},
                {"id":"C","width":40,"height":30}
            ],
            "edges": [
                {"id":"A-to-C","sources":["A"],"targets":["C"]},
                {"id":"A-to-X","sources":["A"],"targets":["X"]},
                {"id":"B-to-X","sources":["B"],"targets":["X"]},
                {"id":"X-to-C","sources":["X"],"targets":["C"]}
            ]
        }))
        .unwrap();

        layout_elk_graph_with_layered_pipeline(&mut graph, ElkOptions::default()).unwrap();

        // Go 1.27.0 / elk-go v0.2.0, matching D2 v0.9.0's wire options.
        assert_eq!((graph.width, graph.height), (Some(280.0), Some(86.0)));
        assert_eq!(
            graph
                .children
                .iter()
                .map(|node| (node.id.as_str(), node.x, node.y))
                .collect::<Vec<_>>(),
            vec![
                ("A", 0.0, 0.0),
                ("B", 0.0, 56.0),
                ("X", 120.0, 51.0),
                ("C", 240.0, 23.0),
            ]
        );
        let expected = [
            (
                "A-to-C",
                (40.0, 10.0),
                vec![(200.0, 10.0), (200.0, 33.0)],
                (240.0, 33.0),
            ),
            (
                "A-to-X",
                (40.0, 20.0),
                vec![(80.0, 20.0), (80.0, 61.0)],
                (120.0, 61.0),
            ),
            ("B-to-X", (40.0, 71.0), vec![], (120.0, 71.0)),
            (
                "X-to-C",
                (160.0, 66.0),
                vec![(200.0, 66.0), (200.0, 43.0)],
                (240.0, 43.0),
            ),
        ];
        for (edge, (id, start, bends, end)) in graph.edges.iter().zip(expected) {
            assert_eq!(edge.id, id);
            let section = &edge.sections[0];
            assert_eq!((section.start.x, section.start.y), start);
            assert_eq!(
                section
                    .bends
                    .iter()
                    .map(|point| (point.x, point.y))
                    .collect::<Vec<_>>(),
                bends
            );
            assert_eq!((section.end.x, section.end.y), end);
        }
    }

    #[test]
    fn aligned_orthogonal_edge_uses_node_layer_spacing_without_edge_clearances() {
        let mut graph = Graph::with_direction(Direction::Down);
        let source = graph.add_node(Node::new(
            "a",
            Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        let target = graph.add_node(Node::new(
            "b",
            Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        graph.add_edge(Edge { source, target });

        let result = layout_with_layered_pipeline(
            &graph,
            &LayoutOptions::default(),
            ElkOptions {
                node_spacing: 20.0,
                edge_node_spacing: 40.0,
                ..ElkOptions::default()
            },
        )
        .unwrap();
        let mut result = result;
        strip_default_root_padding_result(&mut result);

        // The v0.9.0 orthogonal generator returns zero routing slots for an
        // already aligned edge, so the layer gap is 20px rather than the
        // two 40px edge-node clearances used by a routed lane.
        assert_eq!(result.boxes[&source].origin.y, 0.0);
        assert_eq!(result.boxes[&target].origin.y, 50.0);
        assert_eq!(
            result.routes[&EdgeId(0)],
            vec![Point { x: 20.0, y: 30.0 }, Point { x: 20.0, y: 50.0 },]
        );
    }

    #[test]
    fn orthogonal_diamond_matches_go_v090_relative_geometry() {
        let mut graph = Graph::with_direction(Direction::Right);
        let a = graph.add_node(Node::new(
            "A",
            Size {
                width: 56.0,
                height: 80.0,
            },
        ));
        let b = graph.add_node(Node::new(
            "B",
            Size {
                width: 54.0,
                height: 66.0,
            },
        ));
        let c = graph.add_node(Node::new(
            "C",
            Size {
                width: 54.0,
                height: 66.0,
            },
        ));
        let d = graph.add_node(Node::new(
            "D",
            Size {
                width: 54.0,
                height: 80.0,
            },
        ));
        for (source, target) in [(a, b), (a, c), (b, d), (c, d)] {
            graph.add_edge(Edge { source, target });
        }

        let result =
            layout_with_layered_pipeline(&graph, &LayoutOptions::default(), ElkOptions::default())
                .unwrap();
        let mut result = result;
        strip_default_root_padding_result(&mut result);
        let expected_boxes = [
            (a, (0.0, 36.0)),
            (b, (136.0, 0.0)),
            (c, (136.0, 86.0)),
            (d, (270.0, 36.0)),
        ];
        for (node, (x, y)) in expected_boxes {
            let box_ = result.boxes[&node];
            assert!((box_.origin.x - x).abs() < 1e-9);
            assert!((box_.origin.y - y).abs() < 1e-9);
        }
        let expected_routes = [
            [
                (56.0, 62.66666666666666),
                (96.0, 62.66666666666666),
                (96.0, 33.0),
                (136.0, 33.0),
            ],
            [
                (56.0, 89.33333333333333),
                (96.0, 89.33333333333333),
                (96.0, 119.0),
                (136.0, 119.0),
            ],
            [
                (190.0, 33.0),
                (230.0, 33.0),
                (230.0, 62.66666666666666),
                (270.0, 62.66666666666666),
            ],
            [
                (190.0, 119.0),
                (230.0, 119.0),
                (230.0, 89.33333333333333),
                (270.0, 89.33333333333333),
            ],
        ];
        for (edge_id, expected) in expected_routes.into_iter().enumerate() {
            let route = &result.routes[&d2_rs_core::EdgeId(edge_id as u32)];
            assert_eq!(route.len(), expected.len());
            for (point, (x, y)) in route.iter().zip(expected) {
                assert!((point.x - x).abs() < 1e-9);
                assert!((point.y - y).abs() < 1e-9);
            }
        }
    }

    #[test]
    fn orthogonal_crossing_lanes_match_go_v090_relative_geometry() {
        let mut graph = Graph::with_direction(Direction::Right);
        let a = graph.add_node(Node::new(
            "A",
            Size {
                width: 56.0,
                height: 80.0,
            },
        ));
        let b = graph.add_node(Node::new(
            "B",
            Size {
                width: 54.0,
                height: 66.0,
            },
        ));
        let c = graph.add_node(Node::new(
            "C",
            Size {
                width: 54.0,
                height: 66.0,
            },
        ));
        let d = graph.add_node(Node::new(
            "D",
            Size {
                width: 56.0,
                height: 80.0,
            },
        ));
        for (source, target) in [(a, c), (a, d), (b, c), (b, d)] {
            graph.add_edge(Edge { source, target });
        }

        let result =
            layout_with_layered_pipeline(&graph, &LayoutOptions::default(), ElkOptions::default())
                .unwrap();
        let mut result = result;
        strip_default_root_padding_result(&mut result);
        assert_eq!(result.boxes[&a].origin, Point { x: 0.0, y: 0.0 });
        assert_eq!(result.boxes[&b].origin, Point { x: 2.0, y: 100.0 });
        assert_eq!(result.boxes[&c].origin, Point { x: 236.0, y: 0.0 });
        assert_eq!(result.boxes[&d].origin, Point { x: 236.0, y: 100.0 });
        let expected_routes = vec![
            vec![(56.0, 26.666666666666668), (236.0, 26.666666666666668)],
            vec![
                (56.0, 53.333333333333336),
                (96.0, 53.333333333333336),
                (96.0, 90.0),
                (196.0, 90.0),
                (196.0, 126.66666666666667),
                (236.0, 126.66666666666667),
            ],
            vec![
                (56.0, 126.66666666666667),
                (146.0, 126.66666666666667),
                (146.0, 53.333333333333336),
                (236.0, 53.333333333333336),
            ],
            vec![(56.0, 153.33333333333334), (236.0, 153.33333333333334)],
        ];
        for (edge_id, expected) in expected_routes.into_iter().enumerate() {
            let route = &result.routes[&d2_rs_core::EdgeId(edge_id as u32)];
            assert_eq!(route.len(), expected.len());
            for (point, (x, y)) in route.iter().zip(expected) {
                assert!((point.x - x).abs() < 1e-9);
                assert!((point.y - y).abs() < 1e-9);
            }
        }
    }

    #[test]
    fn orthogonal_down_diamond_matches_go_v090_relative_geometry() {
        let mut graph = Graph::with_direction(Direction::Down);
        let a = graph.add_node(Node::new(
            "A",
            Size {
                width: 56.0,
                height: 80.0,
            },
        ));
        let b = graph.add_node(Node::new(
            "B",
            Size {
                width: 54.0,
                height: 66.0,
            },
        ));
        let c = graph.add_node(Node::new(
            "C",
            Size {
                width: 54.0,
                height: 66.0,
            },
        ));
        let d = graph.add_node(Node::new(
            "D",
            Size {
                width: 54.0,
                height: 80.0,
            },
        ));
        for (source, target) in [(a, b), (a, c), (b, d), (c, d)] {
            graph.add_edge(Edge { source, target });
        }

        let result =
            layout_with_layered_pipeline(&graph, &LayoutOptions::default(), ElkOptions::default())
                .unwrap();
        let mut result = result;
        strip_default_root_padding_result(&mut result);
        for (node, (x, y)) in [
            (a, (24.0, 0.0)),
            (b, (0.0, 160.0)),
            (c, (74.0, 160.0)),
            (d, (24.0, 306.0)),
        ] {
            let origin = result.boxes[&node].origin;
            assert!((origin.x - x).abs() < 1e-9);
            assert!((origin.y - y).abs() < 1e-9);
        }
        let expected_routes = [
            [
                (50.66666666666666, 80.0),
                (50.66666666666666, 120.0),
                (27.0, 120.0),
                (27.0, 160.0),
            ],
            [
                (77.33333333333333, 80.0),
                (77.33333333333333, 120.0),
                (101.0, 120.0),
                (101.0, 160.0),
            ],
            [
                (27.0, 226.0),
                (27.0, 266.0),
                (50.66666666666666, 266.0),
                (50.66666666666666, 306.0),
            ],
            [
                (101.0, 226.0),
                (101.0, 266.0),
                (77.33333333333333, 266.0),
                (77.33333333333333, 306.0),
            ],
        ];
        for (edge_id, expected) in expected_routes.into_iter().enumerate() {
            let route = &result.routes[&d2_rs_core::EdgeId(edge_id as u32)];
            assert_eq!(route.len(), expected.len());
            for (point, (x, y)) in route.iter().zip(expected) {
                assert!((point.x - x).abs() < 1e-9);
                assert!((point.y - y).abs() < 1e-9);
            }
        }
    }

    #[test]
    fn nested_container_matches_go_v090_relative_geometry() {
        let leaf = |id: &str| ElkNode {
            edges: Vec::new(),
            id: id.into(),
            x: 0.0,
            y: 0.0,
            width: 40.0,
            height: 30.0,
            children: Vec::new(),
            ports: Vec::new(),
            labels: Vec::new(),
            layout_options: None,
        };
        let mut graph = ElkGraph {
            children_present: false,
            edges_present: false,
            x: None,
            y: None,
            width: None,
            height: None,
            id: "root".into(),
            layout_options: json!({
                "elk.algorithm": "layered",
                "elk.direction": "RIGHT",
            }),
            children: vec![
                ElkNode {
                    edges: Vec::new(),
                    id: "group".into(),
                    x: 0.0,
                    y: 0.0,
                    width: 100.0,
                    height: 100.0,
                    children: vec![leaf("a"), leaf("b")],
                    ports: Vec::new(),
                    labels: Vec::new(),
                    layout_options: None,
                },
                leaf("c"),
            ],
            edges: vec![ElkEdge {
                junction_points: Vec::new(),
                id: "e".into(),
                sources: vec!["group".into()],
                targets: vec!["c".into()],
                sections: Vec::new(),
                labels: Vec::new(),
                container: "root".into(),
                layout_options: None,
            }],
        };
        layout_elk_graph_with_layered_pipeline(&mut graph, ElkOptions::default()).unwrap();
        strip_default_root_padding(&mut graph);

        let group = &graph.children[0];
        assert_eq!(
            (group.x, group.y, group.width, group.height),
            (0.0, 0.0, 64.0, 104.0)
        );
        assert_eq!((group.children[0].x, group.children[0].y), (12.0, 12.0));
        assert_eq!((group.children[1].x, group.children[1].y), (12.0, 62.0));
        assert_eq!((graph.children[1].x, graph.children[1].y), (134.0, 37.0));
        let section = &graph.edges[0].sections[0];
        assert_eq!((section.start.x, section.start.y), (64.0, 52.0));
        assert_eq!((section.end.x, section.end.y), (134.0, 52.0));
        assert!(section.bends.is_empty());
    }

    #[test]
    fn down_nested_external_port_graph_publishes_its_cross_axis_offset() {
        let mut graph: ElkGraph =
            serde_json::from_str(include_str!("../../../tests/elk/nested-left-wire.json")).unwrap();
        layout_elk_graph_with_layered_pipeline(&mut graph, ElkOptions::default()).unwrap();

        let group = graph
            .children
            .iter()
            .find(|node| node.id == "group")
            .unwrap();
        let expected_nodes = [
            ("group.1", 77.0, 50.0),
            ("group.2", 124.00000000000003, 196.0),
            ("group.3", 91.00000000000003, 342.0),
            ("group.4", 225.00000000000003, 342.0),
        ];
        for (id, expected_x, expected_y) in expected_nodes {
            let node = group.children.iter().find(|node| node.id == id).unwrap();
            assert_eq!((node.x, node.y), (expected_x, expected_y), "{id}");
        }
        let outside = graph
            .children
            .iter()
            .find(|node| node.id == "outside")
            .unwrap();
        let another = graph
            .children
            .iter()
            .find(|node| node.id == "another")
            .unwrap();
        assert_eq!((outside.x, outside.y), (148.50000000000003, 545.0));
        assert_eq!((another.x, another.y), (12.0, 545.0));
        assert_eq!(graph.height, Some(623.0));

        let first_edge = graph.edges.iter().find(|edge| edge.id == "edge-0").unwrap();
        let section = &first_edge.sections[0];
        assert_eq!(
            (section.start.x, section.start.y),
            (130.33333333333334, 116.0)
        );
        assert_eq!((section.end.x, section.end.y), (184.00000000000003, 196.0));

        let external_edge = graph.edges.iter().find(|edge| edge.id == "edge-3").unwrap();
        let section = &external_edge.sections[0];
        assert_eq!(
            (
                section.start.x,
                section.start.y,
                section.end.x,
                section.end.y
            ),
            (197.50000000000003, 274.0, 197.50000000000003, 545.0)
        );

        let second_external_edge = graph.edges.iter().find(|edge| edge.id == "edge-4").unwrap();
        let section = &second_external_edge.sections[0];
        assert_eq!(
            (
                section.start.x,
                section.start.y,
                section.end.x,
                section.end.y
            ),
            (117.16666666666667, 128.0, 63.5, 545.0)
        );
        assert_eq!(
            section
                .bends
                .iter()
                .map(|point| (point.x, point.y))
                .collect::<Vec<_>>(),
            [(117.16666666666667, 168.0), (63.5, 168.0)]
        );
    }

    #[test]
    fn multi_level_nested_container_matches_go_v090_relative_geometry() {
        let leaf = |id: &str| ElkNode {
            edges: Vec::new(),
            id: id.into(),
            x: 0.0,
            y: 0.0,
            width: 40.0,
            height: 30.0,
            children: Vec::new(),
            ports: Vec::new(),
            labels: Vec::new(),
            layout_options: None,
        };
        let mut graph = ElkGraph {
            children_present: false,
            edges_present: false,
            x: None,
            y: None,
            width: None,
            height: None,
            id: "root".into(),
            layout_options: json!({
                "elk.algorithm": "layered",
                "elk.direction": "RIGHT",
            }),
            children: vec![
                ElkNode {
                    edges: Vec::new(),
                    id: "G".into(),
                    x: 0.0,
                    y: 0.0,
                    width: 100.0,
                    height: 100.0,
                    children: vec![ElkNode {
                        edges: Vec::new(),
                        id: "H".into(),
                        x: 0.0,
                        y: 0.0,
                        width: 80.0,
                        height: 80.0,
                        children: vec![leaf("a"), leaf("b")],
                        ports: Vec::new(),
                        labels: Vec::new(),
                        layout_options: None,
                    }],
                    ports: Vec::new(),
                    labels: Vec::new(),
                    layout_options: None,
                },
                leaf("z"),
            ],
            edges: vec![ElkEdge {
                junction_points: Vec::new(),
                id: "e".into(),
                sources: vec!["G".into()],
                targets: vec!["z".into()],
                sections: Vec::new(),
                labels: Vec::new(),
                container: "root".into(),
                layout_options: None,
            }],
        };
        layout_elk_graph_with_layered_pipeline(&mut graph, ElkOptions::default()).unwrap();
        strip_default_root_padding(&mut graph);
        let outer = &graph.children[0];
        assert_eq!(
            (outer.x, outer.y, outer.width, outer.height),
            (0.0, 0.0, 88.0, 128.0)
        );
        let inner = &outer.children[0];
        assert_eq!(
            (inner.x, inner.y, inner.width, inner.height),
            (12.0, 12.0, 64.0, 104.0)
        );
        assert_eq!((inner.children[0].x, inner.children[0].y), (12.0, 12.0));
        assert_eq!((inner.children[1].x, inner.children[1].y), (12.0, 62.0));
        assert_eq!((graph.children[1].x, graph.children[1].y), (158.0, 49.0));
        let section = &graph.edges[0].sections[0];
        assert_eq!((section.start.x, section.start.y), (88.0, 64.0));
        assert_eq!((section.end.x, section.end.y), (158.0, 64.0));
        assert!(section.bends.is_empty());
    }

    #[test]
    fn nested_local_edge_matches_go_v090_relative_geometry() {
        let leaf = |id: &str| ElkNode {
            edges: Vec::new(),
            id: id.into(),
            x: 0.0,
            y: 0.0,
            width: 40.0,
            height: 30.0,
            children: Vec::new(),
            ports: Vec::new(),
            labels: Vec::new(),
            layout_options: None,
        };
        let mut graph = ElkGraph {
            children_present: false,
            edges_present: false,
            x: None,
            y: None,
            width: None,
            height: None,
            id: "root".into(),
            layout_options: json!({"elk.algorithm": "layered", "elk.direction": "RIGHT"}),
            children: vec![
                ElkNode {
                    edges: Vec::new(),
                    id: "G".into(),
                    x: 0.0,
                    y: 0.0,
                    width: 100.0,
                    height: 100.0,
                    children: vec![leaf("a"), leaf("b")],
                    ports: Vec::new(),
                    labels: Vec::new(),
                    layout_options: None,
                },
                leaf("z"),
            ],
            edges: vec![
                ElkEdge {
                    junction_points: Vec::new(),
                    id: "inner".into(),
                    sources: vec!["a".into()],
                    targets: vec!["b".into()],
                    sections: Vec::new(),
                    labels: Vec::new(),
                    container: String::new(),
                    layout_options: None,
                },
                ElkEdge {
                    junction_points: Vec::new(),
                    id: "outer".into(),
                    sources: vec!["G".into()],
                    targets: vec!["z".into()],
                    sections: Vec::new(),
                    labels: Vec::new(),
                    container: "root".into(),
                    layout_options: None,
                },
            ],
        };
        layout_elk_graph_with_layered_pipeline(&mut graph, ElkOptions::default()).unwrap();
        strip_default_root_padding(&mut graph);
        assert_eq!(graph.edges[0].container, "G");
        assert_eq!(graph.edges[1].container, "root");
        let group = &graph.children[0];
        assert_eq!(
            (group.x, group.y, group.width, group.height),
            (0.0, 0.0, 124.0, 54.0)
        );
        assert_eq!((group.children[0].x, group.children[0].y), (12.0, 12.0));
        assert_eq!((group.children[1].x, group.children[1].y), (72.0, 12.0));
        assert_eq!((graph.children[1].x, graph.children[1].y), (194.0, 12.0));
        let inner = &graph.edges[0].sections[0];
        assert_eq!((inner.start.x, inner.start.y), (52.0, 27.0));
        assert_eq!((inner.end.x, inner.end.y), (72.0, 27.0));
        let outer = &graph.edges[1].sections[0];
        assert_eq!((outer.start.x, outer.start.y), (124.0, 27.0));
        assert_eq!((outer.end.x, outer.end.y), (194.0, 27.0));
    }

    #[test]
    fn external_nested_edge_matches_go_v090_relative_geometry() {
        let leaf = |id: &str| ElkNode {
            edges: Vec::new(),
            id: id.into(),
            x: 0.0,
            y: 0.0,
            width: 40.0,
            height: 30.0,
            children: Vec::new(),
            ports: Vec::new(),
            labels: Vec::new(),
            layout_options: None,
        };
        let mut graph = ElkGraph {
            children_present: false,
            edges_present: false,
            x: None,
            y: None,
            width: None,
            height: None,
            id: "root".into(),
            layout_options: json!({"elk.algorithm": "layered", "elk.direction": "RIGHT"}),
            children: vec![
                ElkNode {
                    edges: Vec::new(),
                    id: "G".into(),
                    x: 0.0,
                    y: 0.0,
                    width: 100.0,
                    height: 100.0,
                    children: vec![leaf("a")],
                    ports: Vec::new(),
                    labels: Vec::new(),
                    layout_options: None,
                },
                leaf("z"),
            ],
            edges: vec![ElkEdge {
                junction_points: Vec::new(),
                id: "outer".into(),
                sources: vec!["a".into()],
                targets: vec!["z".into()],
                sections: Vec::new(),
                labels: Vec::new(),
                container: "root".into(),
                layout_options: None,
            }],
        };
        let mut reverse = graph.clone();
        reverse.edges[0].sources = vec!["z".into()];
        reverse.edges[0].targets = vec!["a".into()];
        layout_elk_graph_with_layered_pipeline(&mut graph, ElkOptions::default()).unwrap();
        strip_default_root_padding(&mut graph);
        let group = &graph.children[0];
        assert_eq!(
            (group.x, group.y, group.width, group.height),
            (0.0, 0.0, 64.0, 54.0)
        );
        assert_eq!((group.children[0].x, group.children[0].y), (12.0, 12.0));
        assert_eq!((graph.children[1].x, graph.children[1].y), (139.0, 12.0));
        let section = &graph.edges[0].sections[0];
        assert_eq!((section.start.x, section.start.y), (52.0, 27.0));
        assert_eq!((section.end.x, section.end.y), (139.0, 27.0));
        assert!(section.bends.is_empty());
        layout_elk_graph_with_layered_pipeline(&mut reverse, ElkOptions::default()).unwrap();
        strip_default_root_padding(&mut reverse);
        assert_eq!((reverse.children[0].x, reverse.children[0].y), (115.0, 0.0));
        assert_eq!((reverse.children[1].x, reverse.children[1].y), (0.0, 12.0));
        let reverse_section = &reverse.edges[0].sections[0];
        assert_eq!(
            (reverse_section.start.x, reverse_section.start.y),
            (40.0, 27.0)
        );
        assert_eq!(
            (reverse_section.end.x, reverse_section.end.y),
            (127.0, 27.0)
        );
    }

    #[test]
    fn bk_edge_straightening_postprocess_matches_go_long_edge_bends() {
        let mut graph: ElkGraph = serde_json::from_str(include_str!(
            "../../../tests/elk/d2-p3-greedy-switch-accepted.json"
        ))
        .unwrap();

        layout_elk_graph_with_layered_pipeline(
            &mut graph,
            ElkOptions {
                node_spacing: 20.0,
                edge_node_spacing: 10.0,
                ..ElkOptions::default()
            },
        )
        .unwrap();

        let edge = graph.edges.iter().find(|edge| edge.id == "e4").unwrap();
        let section = &edge.sections[0];
        assert_eq!(
            section
                .bends
                .iter()
                .map(|point| (point.x, point.y))
                .collect::<Vec<_>>(),
            [(102.0, 34.0), (102.0, 29.0), (222.0, 29.0), (222.0, 58.5)]
        );
    }

    #[test]
    fn d2_hierarchy_profile_keeps_balanced_bk_external_port_alignment() {
        let mut graph: ElkGraph = serde_json::from_str(include_str!(
            "../../../tests/elk/d2-hierarchy-balanced-bk-external-port.json"
        ))
        .unwrap();

        layout_elk_graph_with_layered_pipeline(&mut graph, ElkOptions::default()).unwrap();
        strip_default_root_padding(&mut graph);

        // elk-go's default root envelope is 12px and is added by the public
        // D2 adapter. These assertions use that envelope-free solver frame.
        assert_eq!((graph.width, graph.height), (Some(255.0), Some(130.0)));
        let group = &graph.children[0];
        assert_eq!(
            (group.x, group.y, group.width, group.height),
            (0.0, 0.0, 140.0, 130.0)
        );
        assert_eq!((group.children[0].x, group.children[0].y), (50.0, 50.0));
        assert_eq!((graph.children[1].x, graph.children[1].y), (215.0, 50.0));
        let section = &graph.edges[0].sections[0];
        assert_eq!((section.start.x, section.start.y), (90.0, 65.0));
        assert_eq!((section.end.x, section.end.y), (215.0, 65.0));
        assert!(section.bends.is_empty());
    }

    #[test]
    fn deep_external_edge_matches_go_v090_relative_geometry() {
        let leaf = |id: &str| ElkNode {
            edges: Vec::new(),
            id: id.into(),
            x: 0.0,
            y: 0.0,
            width: 40.0,
            height: 30.0,
            children: Vec::new(),
            ports: Vec::new(),
            labels: Vec::new(),
            layout_options: None,
        };
        let mut graph = ElkGraph {
            children_present: false,
            edges_present: false,
            x: None,
            y: None,
            width: None,
            height: None,
            id: "root".into(),
            layout_options: json!({
                "elk.algorithm": "layered",
                "elk.direction": "RIGHT",
                "elk.hierarchyHandling": "INCLUDE_CHILDREN",
                "elk.nodeSize.constraints": "MINIMUM_SIZE",
                "elk.contentAlignment": "H_CENTER V_CENTER",
                "elk.spacing.nodeNodeBetweenLayers": 20,
                "elk.spacing.edgeNodeBetweenLayers": 10,
                "elk.layered.spacing.edgeEdgeBetweenLayers": 50,
            }),
            children: vec![
                ElkNode {
                    edges: Vec::new(),
                    id: "A".into(),
                    x: 0.0,
                    y: 0.0,
                    width: 40.0,
                    height: 30.0,
                    children: vec![ElkNode {
                        edges: Vec::new(),
                        id: "B".into(),
                        x: 0.0,
                        y: 0.0,
                        width: 40.0,
                        height: 30.0,
                        children: vec![leaf("C")],
                        ports: Vec::new(),
                        labels: Vec::new(),
                        layout_options: None,
                    }],
                    ports: Vec::new(),
                    labels: Vec::new(),
                    layout_options: None,
                },
                leaf("Z"),
            ],
            edges: vec![ElkEdge {
                junction_points: Vec::new(),
                id: "e".into(),
                sources: vec!["C".into()],
                targets: vec!["Z".into()],
                sections: Vec::new(),
                labels: Vec::new(),
                container: "root".into(),
                layout_options: None,
            }],
        };
        let mut reverse = graph.clone();
        reverse.edges[0].sources = vec!["Z".into()];
        reverse.edges[0].targets = vec!["C".into()];
        let mut ancestor_source = graph.clone();
        ancestor_source.edges[0].sources = vec!["A".into()];
        ancestor_source.edges[0].targets = vec!["C".into()];
        let mut ancestor_target = graph.clone();
        ancestor_target.edges[0].sources = vec!["C".into()];
        ancestor_target.edges[0].targets = vec!["A".into()];
        layout_elk_graph_with_layered_pipeline(&mut graph, ElkOptions::default()).unwrap();
        strip_default_root_padding(&mut graph);
        assert_eq!((graph.children[0].x, graph.children[0].y), (0.0, 0.0));
        assert_eq!(
            (
                graph.children[0].width,
                graph.children[0].height,
                graph.children[0].children[0].x,
                graph.children[0].children[0].y,
                graph.children[0].children[0].width,
                graph.children[0].children[0].height,
            ),
            (93.0, 78.0, 12.0, 12.0, 64.0, 54.0)
        );
        assert_eq!((graph.children[1].x, graph.children[1].y), (118.0, 24.0));
        let section = &graph.edges[0].sections[0];
        assert_eq!((section.start.x, section.start.y), (64.0, 39.0));
        assert_eq!((section.end.x, section.end.y), (118.0, 39.0));
        assert!(section.bends.is_empty());

        layout_elk_graph_with_layered_pipeline(&mut reverse, ElkOptions::default()).unwrap();
        strip_default_root_padding(&mut reverse);
        assert_eq!((reverse.children[0].x, reverse.children[0].y), (65.0, 0.0));
        assert_eq!(
            (
                reverse.children[0].width,
                reverse.children[0].height,
                reverse.children[0].children[0].x,
                reverse.children[0].children[0].y,
            ),
            (93.0, 78.0, 17.0, 12.0)
        );
        assert_eq!((reverse.children[1].x, reverse.children[1].y), (0.0, 24.0));
        let reverse_section = &reverse.edges[0].sections[0];
        assert_eq!(
            (reverse_section.start.x, reverse_section.start.y),
            (40.0, 39.0)
        );
        assert_eq!((reverse_section.end.x, reverse_section.end.y), (94.0, 39.0));
        assert!(reverse_section.bends.is_empty());

        layout_elk_graph_with_layered_pipeline(&mut ancestor_source, ElkOptions::default())
            .unwrap();
        strip_default_root_padding(&mut ancestor_source);
        assert_eq!(
            (
                ancestor_source.children[0].x,
                ancestor_source.children[0].y,
                ancestor_source.children[0].width,
                ancestor_source.children[0].height,
                ancestor_source.children[0].children[0].x,
                ancestor_source.children[0].children[0].y,
            ),
            (0.0, 0.0, 103.0, 88.0, 27.0, 12.0)
        );
        assert_eq!(
            (ancestor_source.children[1].x, ancestor_source.children[1].y),
            (31.5, 108.0)
        );
        let ancestor_source_section = &ancestor_source.edges[0].sections[0];
        assert_eq!(
            (
                ancestor_source_section.start.x,
                ancestor_source_section.start.y,
                ancestor_source_section.end.x,
                ancestor_source_section.end.y,
            ),
            (103.0, 76.0, 39.0, 39.0)
        );
        assert_eq!(ancestor_source_section.bends.len(), 2);
        assert_eq!(
            (
                ancestor_source_section.bends[0].x,
                ancestor_source_section.bends[0].y,
                ancestor_source_section.bends[1].x,
                ancestor_source_section.bends[1].y,
            ),
            (12.0, 76.0, 12.0, 39.0)
        );

        layout_elk_graph_with_layered_pipeline(&mut ancestor_target, ElkOptions::default())
            .unwrap();
        strip_default_root_padding(&mut ancestor_target);
        assert_eq!(
            (
                ancestor_target.children[0].x,
                ancestor_target.children[0].y,
                ancestor_target.children[0].width,
                ancestor_target.children[0].height,
                ancestor_target.children[0].children[0].x,
                ancestor_target.children[0].children[0].y,
            ),
            (0.0, 0.0, 103.0, 88.0, 12.0, 12.0)
        );
        assert_eq!(
            (ancestor_target.children[1].x, ancestor_target.children[1].y),
            (31.5, 108.0)
        );
        let ancestor_target_section = &ancestor_target.edges[0].sections[0];
        assert_eq!(
            (
                ancestor_target_section.start.x,
                ancestor_target_section.start.y,
                ancestor_target_section.end.x,
                ancestor_target_section.end.y,
            ),
            (64.0, 39.0, 0.0, 76.0)
        );
        assert_eq!(ancestor_target_section.bends.len(), 2);
        assert_eq!(
            (
                ancestor_target_section.bends[0].x,
                ancestor_target_section.bends[0].y,
                ancestor_target_section.bends[1].x,
                ancestor_target_section.bends[1].y,
            ),
            (91.0, 39.0, 91.0, 76.0)
        );
    }

    #[test]
    fn multi_external_fanout_matches_go_v090_relative_geometry() {
        let mut graph: ElkGraph = serde_json::from_str(include_str!(
            "../../../tests/elk/d2-multi-external-fanout-root.json"
        ))
        .unwrap();
        layout_elk_graph_with_layered_pipeline(&mut graph, ElkOptions::default()).unwrap();
        strip_default_root_padding(&mut graph);
        assert_eq!(
            (
                graph.children[0].x,
                graph.children[0].y,
                graph.children[0].width,
                graph.children[0].height,
                graph.children[0].children[0].x,
                graph.children[0].children[0].y,
            ),
            (0.0, 0.0, 93.0, 78.0, 12.0, 12.0)
        );
        assert_eq!((graph.children[1].x, graph.children[1].y), (118.0, 19.0));
        assert_eq!((graph.children[2].x, graph.children[2].y), (118.0, 69.0));
        let straight = &graph.edges[0].sections[0];
        assert_eq!((straight.start.x, straight.start.y), (64.0, 34.0));
        assert_eq!((straight.end.x, straight.end.y), (118.0, 34.0));
        assert!(straight.bends.is_empty());
        let routed = &graph.edges[1].sections[0];
        assert_eq!((routed.start.x, routed.start.y), (64.0, 44.0));
        assert_eq!((routed.end.x, routed.end.y), (118.0, 84.0));
        assert_eq!(routed.bends.len(), 2);
        assert_eq!((routed.bends[0].x, routed.bends[0].y), (108.0, 44.0));
        assert_eq!((routed.bends[1].x, routed.bends[1].y), (108.0, 84.0));
    }

    #[test]
    fn multi_external_fanout_down_matches_go_v090_relative_geometry() {
        let leaf = |id: &str| ElkNode {
            edges: Vec::new(),
            id: id.into(),
            x: 0.0,
            y: 0.0,
            width: 40.0,
            height: 30.0,
            children: Vec::new(),
            ports: Vec::new(),
            labels: Vec::new(),
            layout_options: None,
        };
        let mut a = leaf("A");
        let mut b = leaf("B");
        b.children = vec![leaf("C")];
        a.children = vec![b];
        let mut graph = ElkGraph {
            children_present: false,
            edges_present: false,
            x: None,
            y: None,
            width: None,
            height: None,
            id: "root".into(),
            layout_options: json!({
                "elk.algorithm": "layered",
                "elk.direction": "DOWN",
                "elk.hierarchyHandling": "INCLUDE_CHILDREN",
                "elk.nodeSize.constraints": "MINIMUM_SIZE",
                "elk.contentAlignment": "H_CENTER V_CENTER",
                "elk.spacing.nodeNodeBetweenLayers": 20,
                "elk.spacing.edgeNodeBetweenLayers": 10,
                "elk.layered.spacing.edgeEdgeBetweenLayers": 50,
            }),
            children: vec![a, leaf("Z"), leaf("Y")],
            edges: vec![
                ElkEdge {
                    junction_points: Vec::new(),
                    id: "e0".into(),
                    sources: vec!["C".into()],
                    targets: vec!["Z".into()],
                    sections: Vec::new(),
                    labels: Vec::new(),
                    container: "root".into(),
                    layout_options: None,
                },
                ElkEdge {
                    junction_points: Vec::new(),
                    id: "e1".into(),
                    sources: vec!["C".into()],
                    targets: vec!["Y".into()],
                    sections: Vec::new(),
                    labels: Vec::new(),
                    container: "root".into(),
                    layout_options: None,
                },
            ],
        };
        layout_elk_graph_with_layered_pipeline(&mut graph, ElkOptions::default()).unwrap();
        strip_default_root_padding(&mut graph);
        assert_eq!(
            (
                graph.children[0].x,
                graph.children[0].y,
                graph.children[0].width,
                graph.children[0].height,
                graph.children[0].children[0].x,
                graph.children[0].children[0].y,
            ),
            (29.33333333333333, 0.0, 88.0, 83.0, 12.0, 12.0)
        );
        assert_eq!((graph.children[1].x, graph.children[1].y), (0.0, 108.0));
        assert_eq!((graph.children[2].x, graph.children[2].y), (60.0, 108.0));
        let routed = &graph.edges[0].sections[0];
        assert!((routed.start.x - 66.66666666666667).abs() < 1e-9);
        assert_eq!(routed.start.y, 54.0);
        assert_eq!((routed.end.x, routed.end.y), (20.0, 108.0));
        assert_eq!(routed.bends.len(), 2);
        assert!((routed.bends[0].x - 66.66666666666667).abs() < 1e-9);
        assert_eq!(routed.bends[0].y, 98.0);
        assert_eq!((routed.bends[1].x, routed.bends[1].y), (20.0, 98.0));
        let straight = &graph.edges[1].sections[0];
        assert_eq!((straight.start.x, straight.start.y), (80.0, 54.0));
        assert_eq!((straight.end.x, straight.end.y), (80.0, 108.0));
        assert!(straight.bends.is_empty());
    }

    #[test]
    fn deep_external_edge_down_matches_go_v090_relative_geometry() {
        let leaf = |id: &str| ElkNode {
            edges: Vec::new(),
            id: id.into(),
            x: 0.0,
            y: 0.0,
            width: 40.0,
            height: 30.0,
            children: Vec::new(),
            ports: Vec::new(),
            labels: Vec::new(),
            layout_options: None,
        };
        let mut a = leaf("A");
        let mut b = leaf("B");
        b.children = vec![leaf("C")];
        a.children = vec![b];
        let mut graph = ElkGraph {
            children_present: false,
            edges_present: false,
            x: None,
            y: None,
            width: None,
            height: None,
            id: "root".into(),
            layout_options: json!({
                "elk.algorithm": "layered",
                "elk.direction": "DOWN",
                "elk.hierarchyHandling": "INCLUDE_CHILDREN",
                "elk.nodeSize.constraints": "MINIMUM_SIZE",
                "elk.contentAlignment": "H_CENTER V_CENTER",
                "elk.spacing.nodeNodeBetweenLayers": 20,
                "elk.spacing.edgeNodeBetweenLayers": 10,
                "elk.layered.spacing.edgeEdgeBetweenLayers": 50,
            }),
            children: vec![a, leaf("Z")],
            edges: vec![ElkEdge {
                junction_points: Vec::new(),
                id: "e".into(),
                sources: vec!["C".into()],
                targets: vec!["Z".into()],
                sections: Vec::new(),
                labels: Vec::new(),
                container: "root".into(),
                layout_options: None,
            }],
        };
        layout_elk_graph_with_layered_pipeline(&mut graph, ElkOptions::default()).unwrap();
        strip_default_root_padding(&mut graph);
        assert_eq!(
            (
                graph.children[0].x,
                graph.children[0].y,
                graph.children[0].width,
                graph.children[0].height,
                graph.children[0].children[0].x,
                graph.children[0].children[0].y,
            ),
            (0.0, 0.0, 88.0, 83.0, 12.0, 12.0)
        );
        assert_eq!((graph.children[1].x, graph.children[1].y), (24.0, 108.0));
        let section = &graph.edges[0].sections[0];
        assert_eq!((section.start.x, section.start.y), (44.0, 54.0));
        assert_eq!((section.end.x, section.end.y), (44.0, 108.0));
        assert!(section.bends.is_empty());
    }

    #[test]
    fn multi_external_fanout_up_matches_go_v090_local_geometry() {
        let leaf = |id: &str| ElkNode {
            edges: Vec::new(),
            id: id.into(),
            x: 0.0,
            y: 0.0,
            width: 40.0,
            height: 30.0,
            children: Vec::new(),
            ports: Vec::new(),
            labels: Vec::new(),
            layout_options: None,
        };
        let mut a = leaf("A");
        let mut b = leaf("B");
        b.children = vec![leaf("C")];
        a.children = vec![b];
        let mut graph = ElkGraph {
            children_present: false,
            edges_present: false,
            x: None,
            y: None,
            width: None,
            height: None,
            id: "root".into(),
            layout_options: json!({
                "elk.algorithm": "layered",
                "elk.direction": "UP",
                "elk.hierarchyHandling": "INCLUDE_CHILDREN",
                "elk.nodeSize.constraints": "MINIMUM_SIZE",
                "elk.contentAlignment": "H_CENTER V_CENTER",
                "elk.spacing.nodeNodeBetweenLayers": 20,
                "elk.spacing.edgeNodeBetweenLayers": 10,
                "elk.layered.spacing.edgeEdgeBetweenLayers": 50,
            }),
            children: vec![a, leaf("Z"), leaf("Y")],
            edges: vec![
                ElkEdge {
                    junction_points: Vec::new(),
                    id: "e0".into(),
                    sources: vec!["C".into()],
                    targets: vec!["Z".into()],
                    sections: Vec::new(),
                    labels: Vec::new(),
                    container: "root".into(),
                    layout_options: None,
                },
                ElkEdge {
                    junction_points: Vec::new(),
                    id: "e1".into(),
                    sources: vec!["C".into()],
                    targets: vec!["Y".into()],
                    sections: Vec::new(),
                    labels: Vec::new(),
                    container: "root".into(),
                    layout_options: None,
                },
            ],
        };
        layout_elk_graph_with_layered_pipeline(&mut graph, ElkOptions::default()).unwrap();
        strip_default_root_padding(&mut graph);
        assert_eq!(
            (
                graph.children[0].x,
                graph.children[0].y,
                graph.children[0].width,
                graph.children[0].height,
                graph.children[0].children[0].x,
                graph.children[0].children[0].y,
                graph.children[0].children[0].children[0].x,
                graph.children[0].children[0].children[0].y,
            ),
            (29.33333333333333, 55.0, 88.0, 83.0, 12.0, 17.0, 12.0, 12.0)
        );
        assert_eq!((graph.children[1].x, graph.children[1].y), (0.0, 0.0));
        assert_eq!((graph.children[2].x, graph.children[2].y), (60.0, 0.0));
        let routed = &graph.edges[0].sections[0];
        assert!((routed.start.x - 66.66666666666667).abs() < 1e-9);
        assert_eq!(routed.start.y, 84.0);
        assert_eq!((routed.end.x, routed.end.y), (20.0, 30.0));
        assert_eq!(routed.bends.len(), 2);
        assert!((routed.bends[0].x - 66.66666666666667).abs() < 1e-9);
        assert_eq!(routed.bends[0].y, 40.0);
        assert_eq!((routed.bends[1].x, routed.bends[1].y), (20.0, 40.0));
        let straight = &graph.edges[1].sections[0];
        assert_eq!((straight.start.x, straight.start.y), (80.0, 84.0));
        assert_eq!((straight.end.x, straight.end.y), (80.0, 30.0));
        assert!(straight.bends.is_empty());
    }

    #[test]
    fn multi_external_fanout_left_matches_go_v090_local_geometry() {
        let leaf = |id: &str| ElkNode {
            edges: Vec::new(),
            id: id.into(),
            x: 0.0,
            y: 0.0,
            width: 40.0,
            height: 30.0,
            children: Vec::new(),
            ports: Vec::new(),
            labels: Vec::new(),
            layout_options: None,
        };
        let mut a = leaf("A");
        let mut b = leaf("B");
        b.children = vec![leaf("C")];
        a.children = vec![b];
        let mut graph = ElkGraph {
            children_present: false,
            edges_present: false,
            x: None,
            y: None,
            width: None,
            height: None,
            id: "root".into(),
            layout_options: json!({
                "elk.algorithm": "layered",
                "elk.direction": "LEFT",
                "elk.hierarchyHandling": "INCLUDE_CHILDREN",
                "elk.nodeSize.constraints": "MINIMUM_SIZE",
                "elk.contentAlignment": "H_CENTER V_CENTER",
                "elk.spacing.nodeNodeBetweenLayers": 20,
                "elk.spacing.edgeNodeBetweenLayers": 10,
                "elk.layered.spacing.edgeEdgeBetweenLayers": 50,
            }),
            children: vec![a, leaf("Z"), leaf("Y")],
            edges: vec![
                ElkEdge {
                    junction_points: Vec::new(),
                    id: "e0".into(),
                    sources: vec!["C".into()],
                    targets: vec!["Z".into()],
                    sections: Vec::new(),
                    labels: Vec::new(),
                    container: "root".into(),
                    layout_options: None,
                },
                ElkEdge {
                    junction_points: Vec::new(),
                    id: "e1".into(),
                    sources: vec!["C".into()],
                    targets: vec!["Y".into()],
                    sections: Vec::new(),
                    labels: Vec::new(),
                    container: "root".into(),
                    layout_options: None,
                },
            ],
        };
        layout_elk_graph_with_layered_pipeline(&mut graph, ElkOptions::default()).unwrap();
        strip_default_root_padding(&mut graph);
        assert_eq!(
            (
                graph.children[0].x,
                graph.children[0].y,
                graph.children[0].width,
                graph.children[0].height,
                graph.children[0].children[0].x,
                graph.children[0].children[0].y,
                graph.children[0].children[0].children[0].x,
                graph.children[0].children[0].children[0].y,
            ),
            (65.0, 0.0, 93.0, 78.0, 17.0, 12.0, 12.0, 12.0)
        );
        assert_eq!((graph.children[1].x, graph.children[1].y), (0.0, 19.0));
        assert_eq!((graph.children[2].x, graph.children[2].y), (0.0, 69.0));
        let straight = &graph.edges[0].sections[0];
        assert_eq!((straight.start.x, straight.start.y), (94.0, 34.0));
        assert_eq!((straight.end.x, straight.end.y), (40.0, 34.0));
        assert!(straight.bends.is_empty());
        let routed = &graph.edges[1].sections[0];
        assert_eq!((routed.start.x, routed.start.y), (94.0, 44.0));
        assert_eq!((routed.end.x, routed.end.y), (40.0, 84.0));
        assert_eq!(routed.bends.len(), 2);
        assert_eq!((routed.bends[0].x, routed.bends[0].y), (50.0, 44.0));
        assert_eq!((routed.bends[1].x, routed.bends[1].y), (50.0, 84.0));
    }

    #[test]
    fn multi_external_fanin_matches_go_v090_relative_geometry() {
        let leaf = |id: &str| ElkNode {
            edges: Vec::new(),
            id: id.into(),
            x: 0.0,
            y: 0.0,
            width: 40.0,
            height: 30.0,
            children: Vec::new(),
            ports: Vec::new(),
            labels: Vec::new(),
            layout_options: None,
        };
        let mut a = leaf("A");
        let mut b = leaf("B");
        b.children = vec![leaf("C")];
        a.children = vec![b];
        let mut graph = ElkGraph {
            children_present: false,
            edges_present: false,
            x: None,
            y: None,
            width: None,
            height: None,
            id: "root".into(),
            layout_options: json!({
                "elk.algorithm": "layered",
                "elk.direction": "RIGHT",
                "elk.hierarchyHandling": "INCLUDE_CHILDREN",
                "elk.nodeSize.constraints": "MINIMUM_SIZE",
                "elk.contentAlignment": "H_CENTER V_CENTER",
                "elk.spacing.nodeNodeBetweenLayers": 20,
                "elk.spacing.edgeNodeBetweenLayers": 10,
                "elk.layered.spacing.edgeEdgeBetweenLayers": 50,
            }),
            children: vec![a, leaf("Z"), leaf("Y")],
            edges: vec![
                ElkEdge {
                    junction_points: Vec::new(),
                    id: "e0".into(),
                    sources: vec!["Z".into()],
                    targets: vec!["C".into()],
                    sections: Vec::new(),
                    labels: Vec::new(),
                    container: "root".into(),
                    layout_options: None,
                },
                ElkEdge {
                    junction_points: Vec::new(),
                    id: "e1".into(),
                    sources: vec!["Y".into()],
                    targets: vec!["C".into()],
                    sections: Vec::new(),
                    labels: Vec::new(),
                    container: "root".into(),
                    layout_options: None,
                },
            ],
        };
        layout_elk_graph_with_layered_pipeline(&mut graph, ElkOptions::default()).unwrap();
        strip_default_root_padding(&mut graph);
        assert_eq!(
            (
                graph.children[0].x,
                graph.children[0].y,
                graph.children[0].width,
                graph.children[0].height,
                graph.children[0].children[0].x,
                graph.children[0].children[0].y,
            ),
            (65.0, 0.0, 93.0, 78.0, 17.0, 12.0)
        );
        assert_eq!((graph.children[1].x, graph.children[1].y), (0.0, 69.0));
        assert_eq!((graph.children[2].x, graph.children[2].y), (0.0, 19.0));
        let routed = &graph.edges[0].sections[0];
        assert_eq!((routed.start.x, routed.start.y), (40.0, 84.0));
        assert_eq!((routed.end.x, routed.end.y), (94.0, 44.0));
        assert_eq!(routed.bends.len(), 2);
        assert_eq!((routed.bends[0].x, routed.bends[0].y), (50.0, 84.0));
        assert_eq!((routed.bends[1].x, routed.bends[1].y), (50.0, 44.0));
        let straight = &graph.edges[1].sections[0];
        assert_eq!((straight.start.x, straight.start.y), (40.0, 34.0));
        assert_eq!((straight.end.x, straight.end.y), (94.0, 34.0));
        assert!(straight.bends.is_empty());
    }

    #[test]
    fn multi_external_fanin_down_matches_go_v090_relative_geometry() {
        let leaf = |id: &str| ElkNode {
            edges: Vec::new(),
            id: id.into(),
            x: 0.0,
            y: 0.0,
            width: 40.0,
            height: 30.0,
            children: Vec::new(),
            ports: Vec::new(),
            labels: Vec::new(),
            layout_options: None,
        };
        let mut a = leaf("A");
        let mut b = leaf("B");
        b.children = vec![leaf("C")];
        a.children = vec![b];
        let mut graph = ElkGraph {
            children_present: false,
            edges_present: false,
            x: None,
            y: None,
            width: None,
            height: None,
            id: "root".into(),
            layout_options: json!({
                "elk.algorithm": "layered",
                "elk.direction": "DOWN",
                "elk.hierarchyHandling": "INCLUDE_CHILDREN",
                "elk.nodeSize.constraints": "MINIMUM_SIZE",
                "elk.contentAlignment": "H_CENTER V_CENTER",
                "elk.spacing.nodeNodeBetweenLayers": 20,
                "elk.spacing.edgeNodeBetweenLayers": 10,
                "elk.layered.spacing.edgeEdgeBetweenLayers": 50,
            }),
            children: vec![a, leaf("Z"), leaf("Y")],
            edges: vec![
                ElkEdge {
                    junction_points: Vec::new(),
                    id: "e0".into(),
                    sources: vec!["Z".into()],
                    targets: vec!["C".into()],
                    sections: Vec::new(),
                    labels: Vec::new(),
                    container: "root".into(),
                    layout_options: None,
                },
                ElkEdge {
                    junction_points: Vec::new(),
                    id: "e1".into(),
                    sources: vec!["Y".into()],
                    targets: vec!["C".into()],
                    sections: Vec::new(),
                    labels: Vec::new(),
                    container: "root".into(),
                    layout_options: None,
                },
            ],
        };
        layout_elk_graph_with_layered_pipeline(&mut graph, ElkOptions::default()).unwrap();
        assert_eq!(
            (
                graph.children[0].x,
                graph.children[0].y,
                graph.children[0].width,
                graph.children[0].height,
                graph.children[0].children[0].x,
                graph.children[0].children[0].y,
            ),
            (41.33333333333333, 67.0, 88.0, 83.0, 12.0, 17.0)
        );
        assert_eq!((graph.children[1].x, graph.children[1].y), (72.0, 12.0));
        assert_eq!((graph.children[2].x, graph.children[2].y), (12.0, 12.0));
        let straight = &graph.edges[0].sections[0];
        assert_eq!((straight.start.x, straight.start.y), (92.0, 42.0));
        assert_eq!((straight.end.x, straight.end.y), (92.0, 96.0));
        assert!(straight.bends.is_empty());
        let routed = &graph.edges[1].sections[0];
        assert_eq!((routed.start.x, routed.start.y), (32.0, 42.0));
        assert_eq!((routed.end.x, routed.end.y), (78.66666666666666, 96.0));
        assert_eq!(routed.bends.len(), 2);
        assert_eq!((routed.bends[0].x, routed.bends[0].y), (32.0, 52.0));
        assert_eq!(
            (routed.bends[1].x, routed.bends[1].y),
            (78.66666666666666, 52.0)
        );
    }

    #[test]
    fn ancestor_external_fanout_matches_go_v090_relative_geometry() {
        let leaf = |id: &str| ElkNode {
            edges: Vec::new(),
            id: id.into(),
            x: 0.0,
            y: 0.0,
            width: 40.0,
            height: 30.0,
            children: Vec::new(),
            ports: Vec::new(),
            labels: Vec::new(),
            layout_options: None,
        };
        let mut a = leaf("A");
        let mut b = leaf("B");
        b.children = vec![leaf("C")];
        a.children = vec![b];
        let mut graph = ElkGraph {
            children_present: false,
            edges_present: false,
            x: None,
            y: None,
            width: None,
            height: None,
            id: "root".into(),
            layout_options: json!({
                "elk.algorithm": "layered",
                "elk.direction": "RIGHT",
                "elk.hierarchyHandling": "INCLUDE_CHILDREN",
                "elk.nodeSize.constraints": "MINIMUM_SIZE",
                "elk.contentAlignment": "H_CENTER V_CENTER",
                "elk.spacing.nodeNodeBetweenLayers": 20,
                "elk.spacing.edgeNodeBetweenLayers": 10,
                "elk.layered.spacing.edgeEdgeBetweenLayers": 50,
            }),
            children: vec![a, leaf("Z"), leaf("Y")],
            edges: vec![
                ElkEdge {
                    junction_points: Vec::new(),
                    id: "e0".into(),
                    sources: vec!["A".into()],
                    targets: vec!["C".into()],
                    sections: Vec::new(),
                    labels: Vec::new(),
                    container: "root".into(),
                    layout_options: None,
                },
                ElkEdge {
                    junction_points: Vec::new(),
                    id: "e1".into(),
                    sources: vec!["A".into()],
                    targets: vec!["Y".into()],
                    sections: Vec::new(),
                    labels: Vec::new(),
                    container: "root".into(),
                    layout_options: None,
                },
            ],
        };
        layout_elk_graph_with_layered_pipeline(&mut graph, ElkOptions::default()).unwrap();
        strip_default_root_padding(&mut graph);
        assert_eq!(
            (
                graph.children[0].x,
                graph.children[0].y,
                graph.children[0].width,
                graph.children[0].height,
                graph.children[0].children[0].x,
                graph.children[0].children[0].y,
            ),
            (0.0, 50.0, 103.0, 98.0, 27.0, 12.0)
        );
        assert_eq!((graph.children[1].x, graph.children[1].y), (31.5, 0.0));
        assert_eq!((graph.children[2].x, graph.children[2].y), (123.0, 121.0));
        let deep = &graph.edges[0].sections[0];
        assert_eq!((deep.start.x, deep.start.y), (103.0, 76.0));
        assert_eq!((deep.end.x, deep.end.y), (39.0, 39.0));
        assert_eq!(deep.bends.len(), 2);
        assert_eq!((deep.bends[0].x, deep.bends[0].y), (12.0, 76.0));
        assert_eq!((deep.bends[1].x, deep.bends[1].y), (12.0, 39.0));
        let direct = &graph.edges[1].sections[0];
        assert_eq!((direct.start.x, direct.start.y), (103.0, 136.0));
        assert_eq!((direct.end.x, direct.end.y), (123.0, 136.0));
        assert!(direct.bends.is_empty());
    }

    #[test]
    fn explicit_ports_match_go_v090_relative_geometry() {
        let port = |id: &str, side: &str, index: i64| ElkPort {
            id: id.into(),
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
            layout_options: json!({
                "elk.port.side": side,
                "elk.port.index": index,
            }),
        };
        let node = |id: &str, ports: Vec<ElkPort>| ElkNode {
            edges: Vec::new(),
            id: id.into(),
            x: 0.0,
            y: 0.0,
            width: 60.0,
            height: 80.0,
            children: Vec::new(),
            ports,
            labels: Vec::new(),
            layout_options: None,
        };
        let mut graph = ElkGraph {
            children_present: false,
            edges_present: false,
            x: None,
            y: None,
            width: None,
            height: None,
            id: "root".into(),
            layout_options: json!({
                "elk.algorithm": "layered",
                "elk.direction": "RIGHT",
            }),
            children: vec![
                node("A", vec![port("a1", "EAST", 0), port("a2", "EAST", 1)]),
                node("B", vec![port("b1", "WEST", 0), port("b2", "WEST", 1)]),
            ],
            edges: vec![
                ElkEdge {
                    junction_points: Vec::new(),
                    id: "e1".into(),
                    sources: vec!["a1".into()],
                    targets: vec!["b2".into()],
                    sections: Vec::new(),
                    labels: Vec::new(),
                    container: "root".into(),
                    layout_options: None,
                },
                ElkEdge {
                    junction_points: Vec::new(),
                    id: "e2".into(),
                    sources: vec!["a2".into()],
                    targets: vec!["b1".into()],
                    sections: Vec::new(),
                    labels: Vec::new(),
                    container: "root".into(),
                    layout_options: None,
                },
            ],
        };
        layout_elk_graph_with_layered_pipeline(&mut graph, ElkOptions::default()).unwrap();
        strip_default_root_padding(&mut graph);
        assert_eq!((graph.children[0].x, graph.children[0].y), (0.0, 0.0));
        assert_eq!((graph.children[1].x, graph.children[1].y), (130.0, 0.0));
        assert_eq!(graph.children[0].ports[0].x, 60.0);
        assert!((graph.children[0].ports[0].y - 26.666666666666668).abs() < 1e-9);
        assert_eq!(
            (graph.children[0].ports[1].x, graph.children[0].ports[1].y),
            (60.0, 53.333333333333336)
        );
        assert_eq!(
            (graph.children[1].ports[0].x, graph.children[1].ports[0].y),
            (0.0, 53.333333333333336)
        );
        assert_eq!(
            (graph.children[1].ports[1].x, graph.children[1].ports[1].y),
            (0.0, 26.666666666666668)
        );
        for (section, y) in graph
            .edges
            .iter()
            .map(|edge| &edge.sections[0])
            .zip([26.666666666666668, 53.333333333333336])
        {
            assert_eq!(section.start.x, 60.0);
            assert!((section.start.y - y).abs() < 1e-9);
            assert_eq!(section.end.x, 130.0);
            assert!((section.end.y - y).abs() < 1e-9);
            assert!(section.bends.is_empty());
        }
    }

    #[test]
    fn fixed_position_ports_preserve_imported_cross_axis_coordinate() {
        let zero = 0.0;
        let port = |id: &str, side: &str| ElkPort {
            id: id.into(),
            x: zero,
            y: 73.0,
            width: zero,
            height: zero,
            layout_options: json!({"elk.port.side": side}),
        };
        let node = |id: &str, port: ElkPort| ElkNode {
            edges: Vec::new(),
            id: id.into(),
            x: 0.0,
            y: 0.0,
            width: 80.0,
            height: 100.0,
            children: Vec::new(),
            ports: vec![port],
            labels: Vec::new(),
            layout_options: Some(json!({"elk.portConstraints": "FIXED_POS"})),
        };
        let mut graph = ElkGraph {
            children_present: false,
            edges_present: false,
            x: None,
            y: None,
            width: None,
            height: None,
            id: "G".into(),
            layout_options: json!({
                "elk.algorithm": "layered",
                "elk.direction": "RIGHT",
                "elk.hierarchyHandling": "INCLUDE_CHILDREN",
                "elk.spacing.nodeNodeBetweenLayers": 20,
                "elk.spacing.edgeNodeBetweenLayers": 10,
                "elk.layered.spacing.edgeEdgeBetweenLayers": 50,
            }),
            children: vec![node("A", port("a", "EAST")), node("B", port("b", "WEST"))],
            edges: vec![ElkEdge {
                junction_points: Vec::new(),
                id: "e".into(),
                sources: vec!["a".into()],
                targets: vec!["b".into()],
                sections: Vec::new(),
                labels: Vec::new(),
                container: "G".into(),
                layout_options: None,
            }],
        };
        layout_elk_graph_with_layered_pipeline(&mut graph, ElkOptions::default()).unwrap();
        strip_default_root_padding(&mut graph);
        assert_eq!((graph.children[0].x, graph.children[0].y), (0.0, 0.0));
        assert_eq!((graph.children[1].x, graph.children[1].y), (100.0, 0.0));
        assert_eq!(
            (graph.children[0].ports[0].x, graph.children[0].ports[0].y),
            (80.0, 73.0)
        );
        assert_eq!(
            (graph.children[1].ports[0].x, graph.children[1].ports[0].y),
            (0.0, 73.0)
        );
        let section = &graph.edges[0].sections[0];
        assert_eq!((section.start.x, section.start.y), (80.0, 73.0));
        assert_eq!((section.end.x, section.end.y), (100.0, 73.0));
    }

    #[test]
    fn fixed_north_table_port_uses_in_layer_dummy_and_matches_go_geometry() {
        let mut graph: ElkGraph = serde_json::from_value(json!({
            "id": "",
            "layoutOptions": {
                "elk.algorithm": "layered",
                "spacing.nodeNodeBetweenLayers": 70.0,
                "spacing.edgeNodeBetweenLayers": 40.0,
                "elk.spacing.nodeSelfLoop": 50.0,
                "elk.spacing.edgeNode": 40,
                "elk.layered.nodePlacement.bk.fixedAlignment": "BALANCED",
                "elk.layered.thoroughness": 8,
                "elk.layered.spacing.edgeEdgeBetweenLayers": 50,
                "elk.direction": "DOWN",
                "elk.hierarchyHandling": "INCLUDE_CHILDREN",
                "elk.nodeSize.constraints": "MINIMUM_SIZE",
                "elk.contentAlignment": "H_CENTER V_CENTER",
                "elk.layered.considerModelOrder.strategy": "NODES_AND_EDGES",
                "elk.layered.cycleBreaking.strategy": "GREEDY_MODEL_ORDER"
            },
            "children": [
                {
                    "id": "db", "width": 145.0, "height": 108.0,
                    "ports": [
                        {"id":"db.0.src","x":0.0,"y":54.0,"width":0.0,"height":0.0,"layoutOptions":{"elk.port.side":"EAST"}},
                        {"id":"db.0.dst","x":0.0,"y":54.0,"width":0.0,"height":0.0,"layoutOptions":{"elk.port.side":"WEST"}},
                        {"id":"db.1.src","x":0.0,"y":90.0,"width":0.0,"height":0.0,"layoutOptions":{"elk.port.side":"EAST"}},
                        {"id":"db.1.dst","x":0.0,"y":90.0,"width":0.0,"height":0.0,"layoutOptions":{"elk.port.side":"WEST"}}
                    ],
                    "layoutOptions": {"elk.portConstraints":"FIXED_POS"}
                },
                {
                    "id":"service","width":96.0,"height":66.0,
                    "labels":[{"text":"service","width":51.0,"height":21.0}]
                }
            ],
            "edges": [{"id":"edge-0","sources":["service"],"targets":["db.0.dst"],"container":""}]
        }))
        .unwrap();

        layout_elk_graph_with_layered_pipeline(&mut graph, ElkOptions::default()).unwrap();
        strip_default_root_padding(&mut graph);

        let db = &graph.children[0];
        let service = &graph.children[1];
        assert_eq!(
            (db.x, db.y, db.width, db.height),
            (88.0, 136.0, 145.0, 108.0)
        );
        assert_eq!((service.x, service.y), (0.0, 0.0));
        let section = &graph.edges[0].sections[0];
        assert_eq!((section.start.x, section.start.y), (48.0, 66.0));
        assert_eq!((section.end.x, section.end.y), (88.0, 190.0));
        assert_eq!(section.bends.len(), 1);
        assert_eq!((section.bends[0].x, section.bends[0].y), (48.0, 190.0));
    }

    #[test]
    fn north_south_ports_match_go_v090_relative_geometry() {
        let port = |id: &str, side: &str| ElkPort {
            id: id.into(),
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
            layout_options: json!({
                "elk.port.side": side,
                "elk.port.index": 0,
            }),
        };
        let node = |id: &str, ports: Vec<ElkPort>| ElkNode {
            edges: Vec::new(),
            id: id.into(),
            x: 0.0,
            y: 0.0,
            width: 60.0,
            height: 50.0,
            children: Vec::new(),
            ports,
            labels: Vec::new(),
            layout_options: None,
        };
        let mut graph = ElkGraph {
            children_present: false,
            edges_present: false,
            x: None,
            y: None,
            width: None,
            height: None,
            id: "root".into(),
            layout_options: json!({
                "elk.algorithm": "layered",
                "elk.direction": "DOWN",
            }),
            children: vec![
                node("A", vec![port("a1", "SOUTH")]),
                node("B", vec![port("b1", "NORTH")]),
            ],
            edges: vec![ElkEdge {
                junction_points: Vec::new(),
                id: "e".into(),
                sources: vec!["a1".into()],
                targets: vec!["b1".into()],
                sections: Vec::new(),
                labels: Vec::new(),
                container: "root".into(),
                layout_options: None,
            }],
        };
        layout_elk_graph_with_layered_pipeline(&mut graph, ElkOptions::default()).unwrap();
        strip_default_root_padding(&mut graph);
        assert_eq!((graph.children[0].x, graph.children[0].y), (0.0, 0.0));
        assert_eq!((graph.children[1].x, graph.children[1].y), (0.0, 120.0));
        assert_eq!(
            (graph.children[0].ports[0].x, graph.children[0].ports[0].y),
            (30.0, 50.0)
        );
        assert_eq!(
            (graph.children[1].ports[0].x, graph.children[1].ports[0].y),
            (30.0, 0.0)
        );
        let section = &graph.edges[0].sections[0];
        assert_eq!((section.start.x, section.start.y), (30.0, 50.0));
        assert_eq!((section.end.x, section.end.y), (30.0, 120.0));
        assert!(section.bends.is_empty());
    }

    #[test]
    fn source_shaped_pipeline_applies_root_direction_transform() {
        for (direction, relation) in [
            (Direction::Right, (true, false)),
            (Direction::Left, (false, false)),
            (Direction::Down, (false, true)),
            (Direction::Up, (false, false)),
        ] {
            let mut graph = Graph::with_direction(direction);
            let source = graph.add_node(Node::new(
                "a",
                Size {
                    width: 40.0,
                    height: 30.0,
                },
            ));
            let target = graph.add_node(Node::new(
                "b",
                Size {
                    width: 40.0,
                    height: 30.0,
                },
            ));
            graph.add_edge(Edge { source, target });
            let result = layout_with_layered_pipeline(
                &graph,
                &LayoutOptions::default(),
                ElkOptions::default(),
            )
            .unwrap();
            let source_box = result.boxes[&source];
            let target_box = result.boxes[&target];
            let x_order = source_box.origin.x < target_box.origin.x;
            let y_order = source_box.origin.y < target_box.origin.y;
            assert_eq!((x_order, y_order), relation, "direction={direction:?}");
        }
    }

    #[test]
    fn elk_baseline_preserves_selected_route_scope() {
        let mut graph = Graph::default();
        let a = graph.add_node(Node::new(
            "a",
            Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        let b = graph.add_node(Node::new(
            "b",
            Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        let c = graph.add_node(Node::new(
            "c",
            Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        let first = graph.add_edge(Edge {
            source: a,
            target: b,
        });
        graph.add_edge(Edge {
            source: b,
            target: c,
        });
        let result = route_edges_selected(&graph, &LayoutOptions::default(), &[first]).unwrap();
        assert_eq!(result.routes.len(), 1);
        assert!(result.routes.contains_key(&first));
    }

    #[test]
    fn fixed_algorithm_uses_source_wire_positions_without_synthetic_routes() {
        let mut graph = Graph::default();
        let first = graph.add_node(Node::new(
            "a",
            Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        let second = graph.add_node(Node::new(
            "b",
            Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        graph.node_mut(first).unwrap().locked_position =
            Some(d2_rs_core::Point { x: 12.0, y: 18.0 });
        graph.node_mut(second).unwrap().locked_position =
            Some(d2_rs_core::Point { x: 140.0, y: 72.0 });
        graph.add_edge(Edge {
            source: first,
            target: second,
        });
        let result = layout_with_options(
            &graph,
            &LayoutOptions::default(),
            ElkOptions {
                algorithm: ElkAlgorithm::Fixed,
                ..ElkOptions::default()
            },
        )
        .unwrap();
        assert_eq!(result.boxes[&first].origin, d2_rs_core::Point::default());
        assert_eq!(result.boxes[&second].origin, d2_rs_core::Point::default());
        assert!(result.routes[&EdgeId(0)].is_empty());
    }

    #[test]
    fn rectpacking_uses_stable_non_overlapping_rows() {
        let mut graph = Graph::default();
        for name in ["a", "b", "c", "d"] {
            graph.add_node(Node::new(
                name,
                Size {
                    width: 40.0,
                    height: 30.0,
                },
            ));
        }
        let result = layout_with_options(
            &graph,
            &LayoutOptions::default(),
            ElkOptions {
                algorithm: ElkAlgorithm::Rectpacking,
                ..ElkOptions::default()
            },
        )
        .unwrap();
        let rects = result.boxes.values().collect::<Vec<_>>();
        assert_eq!(rects.len(), 4);
        assert!(rects.windows(2).all(|pair| !pair[0].overlaps(*pair[1])));
    }

    #[test]
    fn self_loop_spacing_changes_route_clearance() {
        let mut graph = Graph::default();
        let node = graph.add_node(Node::new(
            "a",
            Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        graph.add_edge(Edge {
            source: node,
            target: node,
        });

        let compact = layout_with_options(
            &graph,
            &LayoutOptions::default(),
            ElkOptions {
                self_loop_spacing: 20.0,
                ..ElkOptions::default()
            },
        )
        .unwrap();
        let spacious = layout_with_options(
            &graph,
            &LayoutOptions::default(),
            ElkOptions {
                self_loop_spacing: 140.0,
                ..ElkOptions::default()
            },
        )
        .unwrap();

        let extent = |result: &d2_rs_core::LayoutResult| {
            let route = &result.routes[&d2_rs_core::EdgeId(0)];
            let min_x = route
                .iter()
                .map(|point| point.x)
                .fold(f64::INFINITY, f64::min);
            let max_x = route
                .iter()
                .map(|point| point.x)
                .fold(f64::NEG_INFINITY, f64::max);
            max_x - min_x
        };
        assert!(extent(&spacious) - extent(&compact) > 80.0);
    }

    #[test]
    fn high_degree_nodes_reserve_port_clearance() {
        let mut graph = Graph::default();
        let source_a = graph.add_node(Node::new(
            "a",
            Size {
                width: 20.0,
                height: 20.0,
            },
        ));
        let source_b = graph.add_node(Node::new(
            "b",
            Size {
                width: 20.0,
                height: 20.0,
            },
        ));
        let target = graph.add_node(Node::new(
            "c",
            Size {
                width: 20.0,
                height: 20.0,
            },
        ));
        graph.add_edge(Edge {
            source: source_a,
            target,
        });
        graph.add_edge(Edge {
            source: source_b,
            target,
        });
        let result =
            route_edges_with_options(&graph, &LayoutOptions::default(), ElkOptions::default())
                .unwrap();
        assert!(result.boxes[&target].size.width >= 80.0);
    }

    #[test]
    fn loop_label_raises_default_clearance() {
        let mut graph = Graph::default();
        let node = graph.add_node(Node::new(
            "a",
            Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        let edge = graph.add_edge(Edge {
            source: node,
            target: node,
        });
        graph.set_edge_label(
            edge,
            Some(EdgeLabel {
                text: "a very wide loop label".into(),
                size: Size {
                    width: 240.0,
                    height: 16.0,
                },
                position: LabelPosition::Unset,
                percentage: 0.5,
            }),
        );
        let result = layout(&graph, &LayoutOptions::default()).unwrap();
        let route = &result.routes[&edge];
        let min_x = route
            .iter()
            .map(|point| point.x)
            .fold(f64::INFINITY, f64::min);
        let max_x = route
            .iter()
            .map(|point| point.x)
            .fold(f64::NEG_INFINITY, f64::max);
        assert!(max_x - min_x >= 100.0);
    }

    #[test]
    fn odd_loop_label_width_uses_go_integer_clearance() {
        let mut graph = Graph::default();
        let node = graph.add_node(Node::new(
            "a",
            Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        let edge = graph.add_edge(Edge {
            source: node,
            target: node,
        });
        graph.set_edge_label(
            edge,
            Some(EdgeLabel {
                text: "odd width".into(),
                size: Size {
                    width: 167.0,
                    height: 16.0,
                },
                position: LabelPosition::Unset,
                percentage: 0.5,
            }),
        );

        let (_, options) = prepare_layered_graph(&graph, ElkOptions::default());
        assert_eq!(options.self_loop_spacing, 88.0);
    }

    #[test]
    fn nested_loop_labels_do_not_change_root_clearance() {
        let mut graph = Graph::default();
        let parent = graph.add_node(Node::new(
            "container",
            Size {
                width: 160.0,
                height: 120.0,
            },
        ));
        let child = graph.add_node(Node::new(
            "container.child",
            Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        graph.node_mut(child).unwrap().parent = Some(parent);
        let edge = graph.add_edge(Edge {
            source: child,
            target: child,
        });
        graph.set_edge_label(
            edge,
            Some(EdgeLabel {
                text: "nested loop".into(),
                size: Size {
                    width: 300.0,
                    height: 16.0,
                },
                position: LabelPosition::Unset,
                percentage: 0.5,
            }),
        );

        let (_, options) = prepare_layered_graph(&graph, ElkOptions::default());
        assert_eq!(options.self_loop_spacing, 50.0);
        // d2layouts/d2elklayout/layout.go sets the root and each container
        // from their own self-loop label maxima.
        let wire = crate::codec::graph_to_elk_graph(&graph, &Default::default(), options);
        assert_eq!(
            wire.children[0].layout_options.as_ref().unwrap()["elk.spacing.nodeSelfLoop"],
            155.0
        );
        let custom = ElkOptions {
            self_loop_spacing: 20.0,
            ..ElkOptions::default()
        };
        let wire = crate::codec::graph_to_elk_graph(&graph, &Default::default(), custom);
        assert_eq!(
            wire.children[0].layout_options.as_ref().unwrap()["elk.spacing.nodeSelfLoop"],
            20.0
        );
    }

    #[test]
    fn icon_lanes_are_reserved_but_external_labels_stay_outside_the_box() {
        let mut graph = Graph::default();
        let node = graph.add_node(Node::new(
            "decorated",
            Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        let value = graph.node_mut(node).unwrap();
        value.has_icon = true;
        value.label_size = Some(Size {
            width: 50.0,
            height: 12.0,
        });
        value.external_label = Some(d2_rs_core::ExternalLabel {
            size: Size {
                width: 32.0,
                height: 9.0,
            },
            side: d2_rs_core::ExternalSide::Bottom,
            alignment: d2_rs_core::ExternalAlignment::Center,
            automatic: false,
            reserve_space: true,
        });

        let (prepared, _) = prepare_layered_graph(&graph, ElkOptions::default());
        assert_eq!(prepared.node(node).unwrap().size.width, 40.0);
        // The icon lane is part of the measured node, while the outside label
        // is an ELK label record and does not enlarge the shape box.
        assert_eq!(prepared.node(node).unwrap().size.height, 47.0);
    }

    #[test]
    fn sql_table_column_edges_use_fixed_side_ports() {
        let mut graph = Graph::default();
        let table = graph.add_node(Node::new(
            "users",
            Size {
                width: 100.0,
                height: 120.0,
            },
        ));
        graph.node_mut(table).unwrap().shape = d2_rs_core::ShapeKind::SqlTable;
        graph.set_table_column_count(table, Some(2));
        let target = graph.add_node(Node::new(
            "service",
            Size {
                width: 60.0,
                height: 40.0,
            },
        ));
        let edge = graph.add_edge(Edge {
            source: table,
            target,
        });
        graph.set_edge_table_columns(
            edge,
            d2_rs_core::EdgeTableColumns {
                source: Some(1),
                target: None,
            },
        );
        let result = layout(&graph, &LayoutOptions::default()).unwrap();
        let port = result.routes[&edge][0];
        let table_box = result.boxes[&table];
        assert_eq!(port.x, table_box.right());
        assert!(port.y > table_box.origin.y && port.y < table_box.bottom());
    }

    #[test]
    fn sql_table_dynamic_ports_follow_edge_order() {
        let mut graph = Graph::default();
        let table = graph.add_node(Node::new(
            "users",
            Size {
                width: 100.0,
                height: 80.0,
            },
        ));
        graph.node_mut(table).unwrap().shape = d2_rs_core::ShapeKind::SqlTable;
        graph.set_table_column_count(table, Some(1));
        let first_target = graph.add_node(Node::new(
            "a",
            Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        let second_target = graph.add_node(Node::new(
            "b",
            Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        let first = graph.add_edge(Edge {
            source: table,
            target: first_target,
        });
        let second = graph.add_edge(Edge {
            source: table,
            target: second_target,
        });
        let result = layout(&graph, &LayoutOptions::default()).unwrap();
        let first_port = result.routes[&first][0];
        let second_port = result.routes[&second][0];
        let table_box = result.boxes[&table];
        assert_eq!(first_port.y, table_box.bottom());
        assert_eq!(second_port.y, table_box.bottom());
        assert!(first_port.x < second_port.x);
    }

    #[test]
    fn sql_table_ports_follow_all_source_directions() {
        for direction in [
            d2_rs_core::Direction::Down,
            d2_rs_core::Direction::Up,
            d2_rs_core::Direction::Right,
            d2_rs_core::Direction::Left,
        ] {
            let mut graph = Graph {
                direction,
                ..Graph::default()
            };
            let source_table = graph.add_node(Node::new(
                "source",
                Size {
                    width: 100.0,
                    height: 120.0,
                },
            ));
            graph.node_mut(source_table).unwrap().shape = d2_rs_core::ShapeKind::SqlTable;
            graph.set_table_column_count(source_table, Some(2));
            let target_table = graph.add_node(Node::new(
                "target",
                Size {
                    width: 100.0,
                    height: 120.0,
                },
            ));
            graph.node_mut(target_table).unwrap().shape = d2_rs_core::ShapeKind::SqlTable;
            graph.set_table_column_count(target_table, Some(2));
            let edge = graph.add_edge(Edge {
                source: source_table,
                target: target_table,
            });
            graph.set_edge_table_columns(
                edge,
                d2_rs_core::EdgeTableColumns {
                    source: Some(0),
                    target: Some(1),
                },
            );

            let result = layout(&graph, &LayoutOptions::default()).unwrap();
            let route = &result.routes[&edge];
            let source_point = route[0];
            let target_point = *route.last().unwrap();
            let source_box = result.boxes[&source_table];
            let target_box = result.boxes[&target_table];
            if direction == d2_rs_core::Direction::Left {
                assert_eq!(source_point.x, source_box.origin.x);
                assert_eq!(target_point.x, target_box.right());
            } else {
                assert_eq!(source_point.x, source_box.right());
                assert_eq!(target_point.x, target_box.origin.x);
            }
            assert!(source_point.y > source_box.origin.y);
            assert!(source_point.y < source_box.bottom());
            assert!(target_point.y > target_box.origin.y);
            assert!(target_point.y < target_box.bottom());
        }
    }

    #[test]
    fn original_go_port_transfer_adds_local_offset_after_direction_transform() {
        let minus_zero = -0.0_f64;
        for direction in ["RIGHT", "DOWN", "LEFT", "UP"] {
            let (x, y, w, h) = super::transform_port_geometry(
                minus_zero, minus_zero, minus_zero, minus_zero, 0.0, direction,
            );
            assert_eq!(x.to_bits(), 0.0_f64.to_bits(), "{direction} x");
            assert_eq!(y.to_bits(), 0.0_f64.to_bits(), "{direction} y");
            assert_eq!(w.to_bits(), minus_zero.to_bits());
            assert_eq!(h.to_bits(), minus_zero.to_bits());
        }
    }
}
