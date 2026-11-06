// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Source-shaped pieces of ELK Layered's graph arena.
//!
//! The pinned [`elk-go`] implementation does not layer the JSON model
//! directly. Its importer creates an arena of `LGraph`, `LNode`, and `LEdge`
//! objects, then the five phase pipeline mutates that arena. Keeping this
//! small arena explicit gives the Rust port a place to compare intermediate
//! state with the Go implementation. This module intentionally starts with
//! the first useful solver phase: the ELK 0.8.1-compatible longest-path
//! layerer. Cycle breaking, ordering, placement, and routing remain separate
//! phases and are not hidden behind this API.

use crate::model::ElkGraph;
use d2_rs_core::Point;
use serde_json::Value;
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

#[cfg(feature = "diagnostic-traces")]
macro_rules! trace_rng {
    ($stream:expr, $operation:expr, $draw_count:expr, $float_bits:expr, $value:expr $(,)?) => {
        crate::trace::emit_rng($stream, $operation, $draw_count, $float_bits, $value);
    };
}

#[cfg(feature = "diagnostic-traces")]
macro_rules! trace_candidate {
    ($layer_id:expr, $node_id:expr, $barycenter:expr, $model_position:expr, $arena:expr $(,)?) => {
        crate::trace::emit_candidate($layer_id, $node_id, $barycenter, $model_position, $arena);
    };
}

#[cfg(feature = "diagnostic-traces")]
macro_rules! trace_bk_candidate {
    ($index:expr, $vertical:expr, $horizontal:expr, $layout_size:expr, $valid:expr $(,)?) => {
        crate::trace::emit_bk_candidate(
            $index,
            &format!("{:?}/{:?}", $vertical, $horizontal),
            $layout_size,
            $valid,
        );
    };
}

#[cfg(feature = "diagnostic-traces")]
macro_rules! trace_bk_candidate_positions {
    ($index:expr, $positions:expr, $alignment:expr, $arena:expr) => {
        crate::trace::emit_bk_candidate_positions($index, $positions, $alignment, $arena);
    };
}

#[cfg(not(feature = "diagnostic-traces"))]
macro_rules! trace_bk_candidate {
    ($index:expr, $vertical:expr, $horizontal:expr, $layout_size:expr, $valid:expr $(,)?) => {};
}

#[cfg(not(feature = "diagnostic-traces"))]
macro_rules! trace_bk_candidate_positions {
    ($index:expr, $positions:expr, $alignment:expr, $arena:expr) => {};
}

#[cfg(feature = "diagnostic-traces")]
macro_rules! trace_ordered {
    ($layer_id:expr, $node_id:expr, $position:expr, $arena:expr) => {
        crate::trace::emit_ordered($layer_id, $node_id, $position, $arena);
    };
}

#[cfg(feature = "diagnostic-traces")]
macro_rules! trace_port_rank {
    ($free:expr, $fixed_port:expr, $rank:expr, $node_relative:expr, $forward:expr, $arena:expr $(,)?) => {
        crate::trace::emit_port_rank($free, $fixed_port, $rank, $node_relative, $forward, $arena);
    };
}

#[cfg(not(feature = "diagnostic-traces"))]
macro_rules! trace_rng {
    ($stream:expr, $operation:expr, $draw_count:expr, $float_bits:expr, $value:expr $(,)?) => {};
}

#[cfg(feature = "diagnostic-traces")]
macro_rules! trace_order_comparison {
    ($scope:expr, $index:expr, $first:expr, $second:expr, $result:expr $(,)?) => {
        crate::trace::emit_order_comparison($scope, $index, $first, $second, $result);
    };
}

#[cfg(not(feature = "diagnostic-traces"))]
macro_rules! trace_order_comparison {
    ($scope:expr, $index:expr, $first:expr, $second:expr, $result:expr $(,)?) => {};
}

#[cfg(feature = "diagnostic-traces")]
macro_rules! trace_barycenter_layer_start {
    ($graph_id:expr, $layer_index:expr, $randomized:expr, $forward:expr) => {
        crate::trace::emit_value(
            "ordering",
            "p3_layer_start",
            &format!("graphs[{}]/layers[{}]", $graph_id, $layer_index),
            &format!(
                "graph_index={};layer={};randomized={};forward={}",
                $graph_id, $layer_index, $randomized, $forward
            ),
        );
    };
}

#[cfg(not(feature = "diagnostic-traces"))]
macro_rules! trace_barycenter_layer_start {
    ($graph_id:expr, $layer_index:expr, $randomized:expr, $forward:expr) => {};
}

mod coffman_graham;
mod coordinates;
mod forster;
pub(crate) use coordinates::{
    change_graph_y, hierarchy_edge_offset_y, hierarchy_reference_graph, source_node_y,
};
mod greedy_port_distributor;
pub(crate) mod gwt_priority_queue;
mod hyperedge_merger;
mod network_simplex;

/// Stable arena identity for an imported graph.
pub type LGraphId = u32;
/// Stable arena identity for an imported node.
pub type LNodeId = u32;
/// Stable arena identity for an imported port.
pub type LPortId = u32;
/// Stable arena identity for an imported edge.
pub type LEdgeId = u32;
/// Stable arena identity for a layer.
pub type LayerId = u32;

/// Port side values used by ELK's layered processors.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum PortSide {
    /// No side constraint was imported.
    #[default]
    Undefined,
    /// North side.
    North,
    /// East side.
    East,
    /// South side.
    South,
    /// West side.
    West,
}

/// Port placement constraint imported from `elk.portConstraints`.
///
/// ELK's importer uses this node-level value to decide whether an explicit
/// port's supplied coordinate is authoritative. Keeping the distinction in
/// the arena prevents the generic side distributor from overwriting D2's
/// fixed table-column ports.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PortConstraints {
    /// No constraint was supplied; side-constrained ports are distributed.
    #[default]
    Undefined,
    /// Ports may move freely.
    Free,
    /// Only the side is fixed.
    FixedSide,
    /// Side and order are fixed.
    FixedOrder,
    /// Side and relative ratio are fixed.
    FixedRatio,
    /// The imported port position is fixed.
    FixedPosition,
}

/// ELK's self-loop port-side distribution strategy.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SelfLoopDistribution {
    /// Put every freely orderable loop on the north side.
    #[default]
    North,
    /// Balance loops between the north and south sides.
    NorthSouth,
    /// Assign loops around all four sides in ELK's stable target order.
    Equally,
}

/// Input flow direction retained for ELK's direction pre/postprocessors.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LayerDirection {
    /// Left-to-right internal direction.
    #[default]
    Right,
    /// Right-to-left flow.
    Left,
    /// Top-to-bottom flow.
    Down,
    /// Bottom-to-top flow.
    Up,
}

/// Explicit graph padding imported from `elk.padding`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GraphPadding {
    /// Top padding.
    pub top: f64,
    /// Right padding.
    pub right: f64,
    /// Bottom padding.
    pub bottom: f64,
    /// Left padding.
    pub left: f64,
}

/// Dummy-node classifications used by ELK's layered phases.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LayeredNodeType {
    /// A normal imported node.
    #[default]
    Normal,
    /// A dummy inserted for a long edge.
    LongEdge,
    /// A dummy representing a hierarchical external port.
    ExternalPort,
    /// A north/south port dummy.
    NorthSouthPort,
    /// A dummy carrying a label.
    Label,
    /// A breaking-point dummy.
    BreakingPoint,
    /// A placeholder dummy.
    Placeholder,
    /// A placeholder that cannot shift.
    NonShiftingPlaceholder,
}

/// Source phase-two strategy supported by the native layout pipeline.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LayeringStrategy {
    /// Default network-simplex ranking.
    #[default]
    NetworkSimplex,
    /// Transitive reduction and bounded-width Coffman–Graham ranking.
    CoffmanGraham,
}

/// A graph scope in the layered arena.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LGraph {
    /// Original importer GraphPropertyHyperedges, retained across reversals.
    has_hyperedges: bool,
    /// Nodes not assigned to a layer yet, in importer/model order.
    pub layerless_nodes: Vec<LNodeId>,
    /// Layers in source order, from the first rank to the last rank.
    pub layers: Vec<LayerId>,
    /// Owning container node, when this is a nested graph.
    pub parent_node: Option<LNodeId>,
    /// Source-selected phase-two rank assignment.
    pub layering_strategy: LayeringStrategy,
    /// Optional Coffman–Graham layer bound; source default is i32::MAX.
    pub coffman_graham_layer_bound: Option<i32>,
    /// Included scopes use the hierarchy runner instead of component splitting.
    pub includes_children: bool,
    /// Provider has already performed ComponentsProcessor.Split before P1.
    pub(crate) separated_component: bool,
    /// Input direction before the internal left-to-right normalization pass.
    pub direction: LayerDirection,
    /// Width accumulated by the layer-size phase.
    pub width: f64,
    /// Height accumulated by the layer-size phase.
    pub height: f64,
    /// Vertical offset accumulated by the layer-size phase.
    pub offset_y: f64,
    /// Explicit ELK graph padding, when supplied by the wire model.
    pub padding: Option<GraphPadding>,
    /// Edge-to-edge spacing used for hierarchy boundary offsets.
    pub edge_edge_spacing: f64,
    /// Edge-to-edge spacing between adjacent layout layers. ELK exposes this
    /// separately from the general `spacing.edgeEdge` property.
    pub edge_edge_spacing_between_layers: f64,
    /// Spacing between nodes in adjacent layers for this scope.
    pub node_node_spacing: f64,
    /// Spacing between the rank envelopes of adjacent layers.
    pub layer_spacing: f64,
    /// Spacing between nodes and routed edges in adjacent layers.
    pub edge_node_spacing: f64,
    /// Cross-axis node-to-edge spacing, independent of between-layer clearance.
    pub edge_node_vertical_spacing: f64,
    /// Vertical spacing between hierarchical external-port dummies.
    pub port_port_spacing: f64,
    /// Spacing between a self-loop route and its label.
    pub edge_label_spacing: f64,
    /// Spacing between labels belonging to one self-loop edge.
    pub label_label_spacing: f64,
    /// Clearance reserved between a self-loop and its owning node.
    pub self_loop_spacing: f64,
    /// Network-simplex iteration budget multiplier from ELK's
    /// `elk.layered.thoroughness` option.
    ///
    /// ELK scales the budget as `thoroughness * 4 * sqrt(component_size)`.
    /// D2 supplies 8 for its layered graphs; the standalone ELK default is 7.
    pub thoroughness: usize,
    /// Java-compatible random seed used by hierarchy-aware ordering.
    pub random_seed: i64,
    /// Whether this graph's first P3 sweep must preserve the input model order.
    pub consider_model_order: bool,
    /// GREEDY_MODEL_ORDER uses model ties; the source default GREEDY uses RNG.
    pub cycle_breaking_model_order: bool,
    /// Whether P1 consumed the graph RNG before P3 initializes its holder.
    cycle_random_consumed: bool,
    /// Whether fixed-side port lists use ELK's degree sorting strategy.
    pub port_sorting_by_degree: bool,
    /// ELK's hierarchy-sweep threshold. `None` selects the source default
    /// (0.1) for `elk.layered.crossingMinimization.hierarchicalSweepiness`.
    pub hierarchical_sweepiness: Option<f64>,
    /// Whether disconnected components are split and packed after layout.
    pub separate_components: bool,
    pub(crate) greedy_switch: GreedySwitchMode,
    pub(crate) greedy_switch_activation_threshold: usize,
    pub(crate) semi_interactive: bool,
    pub(crate) interactive_crossing_minimization: bool,
    /// Brandes–Kopf fixed alignment selected for this graph scope.
    pub fixed_alignment: BkFixedAlignment,
    /// Whether node placement should prefer straight edges. ELK defaults this
    /// to true for orthogonal routing, selecting the smallest valid BK layout.
    pub favor_straight_edges: bool,
    /// Whether ELK's default BK edge-straightening threshold is enabled.
    pub bk_edge_straightening: bool,
    /// Target aspect ratio used by the simple connected-component row placer.
    pub aspect_ratio: f64,
    /// Gap between packed connected components.
    pub component_spacing: f64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum GreedySwitchMode {
    OneSided,
    #[default]
    TwoSided,
    Off,
}

/// Source outer-layer constraint, also used by pre-cycle edge reversal.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NodeLayerConstraint {
    /// No explicit boundary constraint.
    #[default]
    None,
    /// First ordinary layer.
    First,
    /// Separate first boundary layer.
    FirstSeparate,
    /// Last ordinary layer.
    Last,
    /// Separate last boundary layer.
    LastSeparate,
}

fn parse_node_layer_constraint(options: Option<&Value>) -> NodeLayerConstraint {
    let value = options
        .and_then(|o| {
            o.get("org.eclipse.elk.layered.layering.layerConstraint")
                .or_else(|| o.get("elk.layered.layering.layerConstraint"))
        })
        .and_then(Value::as_str)
        .unwrap_or("NONE")
        .to_uppercase();
    match value.as_str() {
        "FIRST" => NodeLayerConstraint::First,
        "FIRST_SEPARATE" => NodeLayerConstraint::FirstSeparate,
        "LAST" => NodeLayerConstraint::Last,
        "LAST_SEPARATE" => NodeLayerConstraint::LastSeparate,
        _ => NodeLayerConstraint::None,
    }
}

/// Node alignment interpreted by the source horizontal layer placer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NodeAlignment {
    /// Bias according to connected input/output ports.
    #[default]
    Automatic,
    /// Use the left layer boundary.
    Left,
    /// Use the right layer boundary.
    Right,
    /// Center inside the layer.
    Center,
    /// Top in the input frame; transposed for vertical layouts.
    Top,
    /// Bottom in the input frame; transposed for vertical layouts.
    Bottom,
}

impl NodeAlignment {
    pub(crate) fn transposed(self) -> Self {
        match self {
            Self::Left => Self::Top,
            Self::Right => Self::Bottom,
            Self::Top => Self::Left,
            Self::Bottom => Self::Right,
            other => other,
        }
    }
    pub(crate) fn mirrored(self) -> Self {
        match self {
            Self::Left => Self::Right,
            Self::Right => Self::Left,
            other => other,
        }
    }
}

fn parse_node_alignment(options: Option<&serde_json::Value>) -> NodeAlignment {
    let value = options
        .and_then(|o| {
            o.get("org.eclipse.elk.alignment")
                .or_else(|| o.get("elk.alignment"))
                .or_else(|| o.get("alignment"))
        })
        .and_then(serde_json::Value::as_str)
        .unwrap_or("AUTOMATIC")
        .to_uppercase();
    match value.as_str() {
        "LEFT" => NodeAlignment::Left,
        "RIGHT" => NodeAlignment::Right,
        "CENTER" => NodeAlignment::Center,
        "TOP" => NodeAlignment::Top,
        "BOTTOM" => NodeAlignment::Bottom,
        _ => NodeAlignment::Automatic,
    }
}

/// A node in the layered arena.
#[derive(Clone, Debug, PartialEq)]
pub struct LNode {
    /// Explicit source outer-layer constraint.
    pub layer_constraint: NodeLayerConstraint,
    /// Direction-normalized node alignment.
    pub alignment: NodeAlignment,
    /// Owning graph scope.
    pub graph: LGraphId,
    /// Temporary source-order index used by the Go longest-path phase.
    pub traversal_index: usize,
    /// Assigned layer, once a phase has placed the node.
    pub layer: Option<LayerId>,
    /// Source/model identifier used to map diagnostics back to the wire graph.
    pub source_id: String,
    /// ELK dummy classification.
    pub node_type: LayeredNodeType,
    /// Child graph for a compound node, if present.
    pub nested_graph: Option<LGraphId>,
    /// Ports owned by this node in importer order.
    pub ports: Vec<LPortId>,
    /// Node-level ELK port placement constraint.
    pub port_constraints: PortConstraints,
    /// Self-loop side assignment used for implicit loop endpoints.
    pub self_loop_distribution: SelfLoopDistribution,
    /// Measured width from the wire model.
    pub width: f64,
    /// Measured height from the wire model.
    pub height: f64,
    /// Whether the source graph declared a width that ELK must preserve as a
    /// minimum while compound children are fitted.
    pub fixed_width: bool,
    /// Whether the source graph declared a height that ELK must preserve as a
    /// minimum while compound children are fitted.
    pub fixed_height: bool,
    /// Top margin imported from `org.eclipse.elk.margins`.
    pub margin_top: f64,
    /// Right margin imported from `org.eclipse.elk.margins`.
    pub margin_right: f64,
    /// Bottom margin imported from `org.eclipse.elk.margins`.
    pub margin_bottom: f64,
    /// Left margin imported from `org.eclipse.elk.margins`.
    pub margin_left: f64,
    /// Whether these margins were transferred from D2's source decoration
    /// lanes, which are already part of the imported wire dimensions.
    pub margins_in_wire_dimensions: bool,
    /// Margins recomputed from node and port bounds after label placement,
    /// ordered top, right, bottom, left.
    pub computed_margins: [f64; 4],
    /// Top overhang introduced by imported node labels. ELK computes this
    /// after label placement; it is kept separate from D2's source spacing
    /// margins because the latter are already folded into the wire size.
    pub label_margin_top: f64,
    /// Right overhang introduced by imported node labels.
    pub label_margin_right: f64,
    /// Bottom overhang introduced by imported node labels.
    pub label_margin_bottom: f64,
    /// Left overhang introduced by imported node labels.
    pub label_margin_left: f64,
    /// Relative x position assigned by a placement phase.
    pub position_x: f64,
    /// Relative y position assigned by a placement phase.
    pub position_y: f64,
}

/// A port in the layered arena.
#[derive(Clone, Debug, PartialEq)]
pub struct LPort {
    /// Owning node.
    pub node: LNodeId,
    /// Original wire-model identifier, absent for synthesized phase ports.
    pub source_id: Option<String>,
    /// Original input position assigned by ELK's model-order importer.
    pub model_order: Option<usize>,
    /// Imported side constraint.
    pub side: PortSide,
    /// Optional stable order from `elk.port.index`.
    pub index: Option<i32>,
    /// Relative wire-model position.
    pub x: f64,
    /// Relative wire-model position.
    pub y: f64,
    /// Port width.
    pub width: f64,
    /// Port height.
    pub height: f64,
    /// Whether this port was synthesized for a direct node endpoint by the
    /// importer-side default-port step.
    pub implicit: bool,
    /// Edges entering this port in arena order.
    pub incoming_edges: Vec<LEdgeId>,
    /// Edges leaving this port in arena order.
    pub outgoing_edges: Vec<LEdgeId>,
}

/// A directed edge in the layered arena.
#[derive(Clone, Debug, PartialEq)]
pub struct LEdge {
    /// Source node.
    pub source: LNodeId,
    /// Target node.
    pub target: LNodeId,
    /// Source port, when the wire edge names a port endpoint.
    pub source_port: Option<LPortId>,
    /// Target port, when the wire edge names a port endpoint.
    pub target_port: Option<LPortId>,
    /// Whether cycle breaking reversed this edge from its imported direction.
    pub reversed: bool,
    /// Whether this edge is still part of the active intermediate graph.
    ///
    /// ELK keeps arena IDs stable while `JoinLongEdges` retires synthetic
    /// outgoing segments.  Retaining the slot lets trace consumers compare
    /// allocation order without exposing those segments to later processors.
    pub active: bool,
    /// Interior and port-exit points produced by the polyline router.
    pub bend_points: Vec<Point>,
    /// Shared route junction points recorded by the router.
    pub junction_points: Vec<Point>,
    /// Synthetic per-scope segments created for a deep cross-hierarchy edge.
    /// The original edge remains the stable wire identity and is routed by
    /// joining these segment routes after the processors finish.
    pub hierarchy_segments: Vec<LEdgeId>,
}

/// A layer containing nodes in model order. Node ordering is a later phase.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Layer {
    /// Owning graph scope.
    pub graph: LGraphId,
    /// Nodes assigned to this layer.
    pub nodes: Vec<LNodeId>,
    /// Height occupied by this layer after placement.
    pub height: f64,
    /// Width occupied by this layer after node placement.
    pub width: f64,
}

/// Component-local direction and row placement retained until publication.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct ComponentTransfer {
    /// Internal flow-axis extent used by the component's direction postpass.
    pub axis_width: f64,
    /// Combined world-space row placement and source Graph.Offset.
    pub offset: Point,
}

/// Arena-backed intermediate representation matching the ownership boundary
/// used by `elk-go/internal/layered`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LayeredArena {
    /// Imported graph scopes.
    pub graphs: Vec<LGraph>,
    /// Imported nodes.
    pub nodes: Vec<LNode>,
    /// Imported ports.
    pub ports: Vec<LPort>,
    /// Imported edges.
    pub edges: Vec<LEdge>,
    /// Java-compatible per-graph random streams. ELK stores these in graph
    /// properties and shares them across phases, so P5 cycle breaking must
    /// continue from the state left by P1/P3 rather than reseeding.
    graph_randoms: Vec<JavaRandom>,
    /// Source self-loop preprocessing hides these ports/edges through P3.
    hidden_loop_ports: BTreeMap<LNodeId, Vec<LPortId>>,
    hidden_loop_edges: BTreeSet<LEdgeId>,
    /// Graph scopes whose translated P3 processor currently preserves ELK's
    /// shared RNG state for the following P5 cycle detector.
    shared_graph_randoms: BTreeSet<LGraphId>,
    /// Source ELK edge priority weights used by network-simplex layering.
    /// Synthetic edges default to unit weight when no source entry exists.
    edge_priority_shortness: BTreeMap<LEdgeId, f64>,
    /// P1 counts each edge with max(priority.direction, 0) + 1.
    edge_direction_weight: BTreeMap<LEdgeId, usize>,
    /// Source edge thickness used to size long-edge dummies.
    edge_thickness: BTreeMap<LEdgeId, f64>,
    /// Imported edge model order, inherited by synthetic phase segments.
    edge_model_order: BTreeMap<LEdgeId, usize>,
    /// Source label-side options and LongEdgeSource/LongEdgeTarget ports.
    edge_label_side_modes: BTreeMap<LGraphId, EdgeLabelSideSelection>,
    long_edge_endpoints: BTreeMap<LNodeId, (LPortId, LPortId)>,
    /// Optional source/target property presence retained by HyperedgeDummyMerger.
    long_edge_merge_ends: BTreeMap<LNodeId, (Option<LPortId>, Option<LPortId>)>,
    /// LongEdgeHasLabelDummies propagated by the source splitter.
    long_edge_label_dummies: BTreeSet<LNodeId>,
    /// ELK minimum size for compound nodes, imported before direction
    /// normalization and transposed with the graph when needed.
    pub node_size_minimum: BTreeMap<LNodeId, (f64, f64)>,
    /// Compound label boxes `(x, y, width, height)` retained for ResizeNode.
    /// Margins must be measured again after hierarchy transfer changes size.
    pub(crate) compound_label_boxes: BTreeMap<LNodeId, Vec<[f64; 4]>>,
    /// Maximum `(width, height)` of labels attached to direct self-loops.
    /// The source processor uses the flow-axis dimension after transposition.
    pub loop_label_sizes: BTreeMap<LEdgeId, (f64, f64)>,
    /// Self-loop labels explicitly marked inline in the ELK wire model.
    /// ELK uses this property when choosing label sides and corner clearances.
    pub inline_loop_labels: BTreeSet<LEdgeId>,
    /// Center-label proxy nodes inserted before layered rank assignment.
    /// D2 emits inline center labels; retaining the original edge and label
    /// indices lets the wire adapter publish their final boxes after routing.
    pub edge_label_dummies: Vec<EdgeLabelDummy>,
    /// Label content extent without inter-label gaps, used for loop clearance.
    pub loop_label_content_sizes: BTreeMap<LEdgeId, (f64, f64)>,
    /// Layout extents recorded while elk-go's disconnected component graphs
    /// are independently sized, keyed by every node in each component.
    pub component_sizes: BTreeMap<LNodeId, (f64, f64)>,
    /// Independently sized component normalization, before graph restoration.
    pub(crate) component_offsets_y: BTreeMap<LNodeId, f64>,
    /// Deferred source direction/packing transfers for wire publication.
    pub(crate) component_transfers: BTreeMap<LNodeId, ComponentTransfer>,
    /// Margins before loop routing expands the placement envelope.
    pub original_loop_margins: BTreeMap<LNodeId, [f64; 4]>,
    /// Internal horizontal position and deferred compound transfer offset.
    pub nested_transfer_x: BTreeMap<LNodeId, (f64, f64)>,
    /// Internal vertical position and deferred compound transfer offset.
    pub nested_transfer_y: BTreeMap<LNodeId, (f64, f64)>,
    /// Unpadded bend ordinates and padding shift retained for source-ordered
    /// final edge transfer: local point + (Graph.Offset + Padding).
    pub nested_edge_transfer_y: BTreeMap<LEdgeId, (Vec<f64>, f64)>,
    /// Unpadded junction ordinates and graph-padding shift for exact transfer.
    pub(crate) nested_junction_transfer_y: BTreeMap<LEdgeId, (Vec<f64>, f64)>,
    /// Allocated layers.
    pub layers: Vec<Layer>,
    /// Original parent ports represented by synthesized external dummies.
    pub external_port_origins: BTreeMap<LNodeId, LPortId>,
    /// Whether each external-port dummy belongs in a separate boundary layer.
    /// `None` is used for north/south ports, which remain in their assigned rank.
    external_port_first_separate: BTreeMap<LNodeId, Option<bool>>,
    /// Border offsets carried by hierarchy boundary ports.
    pub port_border_offsets: BTreeMap<LPortId, f64>,
    /// Hierarchy scopes whose external dummy represents an original endpoint
    /// on the owning compound node. These scopes receive the external
    /// component envelope during hierarchical resizing.
    pub reused_hierarchy_sides: BTreeMap<LGraphId, u8>,
    /// Imported implicit parent ports reused by a deep hierarchy edge. Their
    /// vertical coordinate is fixed by hierarchy transfer and must survive
    /// the root-size refresh that updates only their outward x coordinate.
    pub reused_parent_ports: BTreeSet<LPortId>,
    /// Fixed north/south ports represented by temporary in-layer dummy nodes.
    /// The source port and edge identities are restored immediately after
    /// routing, while the dummy's cross-axis coordinate becomes a route bend.
    north_south_port_dummies: Vec<NorthSouthPortDummy>,
    /// Port-level source override suppressing north/south successor constraints.
    non_flow_switch_ports: BTreeSet<LPortId>,
    /// Resolver layout-unit membership captured before a graph's first sweep.
    source_forster_constraints: BTreeMap<LGraphId, forster::Constraints>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct NorthSouthPortDummy {
    graph: LGraphId,
    node: LNodeId,
    input_port: Option<LPortId>,
    output_port: Option<LPortId>,
    origin_port: LPortId,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct P3SweepDecision {
    pub use_bottom_up: bool,
    pub paths_to_random: usize,
    pub paths_to_hierarchical: usize,
    pub threshold: f64,
    pub reason: &'static str,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct P3HierarchyPlan {
    /// ELK's breadth-first graph-holder initialization order.
    pub breadth_first_graphs: Vec<LGraphId>,
    /// Independent bottom-up sweep order, formed by prepending eligible
    /// holders while traversing the breadth-first list.
    pub graphs_to_sweep_on: Vec<LGraphId>,
    /// Per-arena-scope eligibility decisions.
    pub decisions: BTreeMap<LGraphId, P3SweepDecision>,
}

#[derive(Clone, Copy, Debug, Default)]
struct P3NodeInfo {
    connected_edges: usize,
    hierarchical_influence: usize,
    random_influence: usize,
}

impl P3NodeInfo {
    fn transfer_to(self, target: &mut Self) {
        target.hierarchical_influence += self.hierarchical_influence;
        target.random_influence += self.random_influence;
        target.connected_edges += self.connected_edges + 1;
    }
}

/// Applies elk-go v0.2.0's `LayerSweepTypeDecider.UseBottomUp` rule to one
/// already-layered graph. It is isolated from P3 ordering so the hierarchy
/// schedule can be qualified against source before controlling layout.
pub(crate) fn p3_sweep_decision(arena: &LayeredArena, graph_id: LGraphId) -> P3SweepDecision {
    let graph = &arena.graphs[graph_id as usize];
    let threshold = graph.hierarchical_sweepiness.unwrap_or(0.1);
    let Some(parent_id) = graph.parent_node else {
        return P3SweepDecision {
            use_bottom_up: true,
            paths_to_random: 0,
            paths_to_hierarchical: 0,
            threshold,
            reason: "root",
        };
    };
    let parent = &arena.nodes[parent_id as usize];
    if threshold < -1.0 {
        return P3SweepDecision {
            use_bottom_up: true,
            paths_to_random: 0,
            paths_to_hierarchical: 0,
            threshold,
            reason: "threshold_below_minus_one",
        };
    }
    if matches!(
        parent.port_constraints,
        PortConstraints::FixedOrder | PortConstraints::FixedRatio | PortConstraints::FixedPosition
    ) {
        return P3SweepDecision {
            use_bottom_up: true,
            paths_to_random: 0,
            paths_to_hierarchical: 0,
            threshold,
            reason: "fixed_parent_port_order",
        };
    }
    let east_ports = parent
        .ports
        .iter()
        .filter(|port_id| arena.ports[**port_id as usize].side == PortSide::East)
        .count();
    let west_ports = parent
        .ports
        .iter()
        .filter(|port_id| arena.ports[**port_id as usize].side == PortSide::West)
        .count();
    if east_ports < 2 && west_ports < 2 {
        return P3SweepDecision {
            use_bottom_up: true,
            paths_to_random: 0,
            paths_to_hierarchical: 0,
            threshold,
            reason: "fewer_than_two_parent_in_out_ports",
        };
    }

    let mut node_info = vec![P3NodeInfo::default(); arena.nodes.len()];
    let mut paths_to_random = 0;
    let mut paths_to_hierarchical = 0;
    for layer_id in &graph.layers {
        let Some(layer) = arena.layers.get(*layer_id as usize) else {
            continue;
        };
        let mut north_south_dummies = Vec::new();
        for node_id in &layer.nodes {
            let node = &arena.nodes[*node_id as usize];
            if node.node_type == LayeredNodeType::NorthSouthPort {
                north_south_dummies.push(*node_id);
                continue;
            }

            if node.node_type == LayeredNodeType::ExternalPort {
                node_info[*node_id as usize].hierarchical_influence = 1;
                if arena
                    .external_port_origins
                    .get(node_id)
                    .is_some_and(|port_id| arena.ports[*port_id as usize].side == PortSide::East)
                {
                    paths_to_hierarchical += node_info[*node_id as usize].connected_edges;
                }
            } else if p3_has_no_active_ports_on_side(arena, *node_id, PortSide::West) {
                node_info[*node_id as usize].random_influence = 1;
            } else if p3_has_no_active_ports_on_side(arena, *node_id, PortSide::East) {
                paths_to_random += node_info[*node_id as usize].connected_edges;
            }

            for (_, edge) in arena.outgoing_edges(*node_id) {
                let source_info = node_info[*node_id as usize];
                paths_to_random += source_info.random_influence;
                paths_to_hierarchical += source_info.hierarchical_influence;
                source_info.transfer_to(&mut node_info[edge.target as usize]);
            }

            // elk-go propagates influence directly from each owner port to
            // its north/south port dummy. The Rust preprocessor rewires the
            // real edge endpoints through the dummy, so this relationship is
            // represented separately from the ordinary outgoing-edge list.
            for side in [PortSide::North, PortSide::South] {
                for dummy in arena.north_south_port_dummies.iter().filter(|dummy| {
                    dummy.graph == graph_id
                        && (dummy.origin_port as usize) < arena.ports.len()
                        && arena.ports[dummy.origin_port as usize].node == *node_id
                        && arena.ports[dummy.origin_port as usize].side == side
                }) {
                    let source_info = node_info[*node_id as usize];
                    paths_to_random += source_info.random_influence;
                    paths_to_hierarchical += source_info.hierarchical_influence;
                    source_info.transfer_to(&mut node_info[dummy.node as usize]);
                }
            }
        }

        // ELK defers north/south port-dummy propagation until after the
        // ordinary nodes of this layer, preserving its initializer order.
        for node_id in north_south_dummies {
            for (_, edge) in arena.outgoing_edges(node_id) {
                let source_info = node_info[node_id as usize];
                paths_to_random += source_info.random_influence;
                paths_to_hierarchical += source_info.hierarchical_influence;
                source_info.transfer_to(&mut node_info[edge.target as usize]);
            }
        }
    }

    let all_paths = paths_to_random + paths_to_hierarchical;
    let normalized = if all_paths == 0 {
        f64::INFINITY
    } else {
        (paths_to_random as f64 - paths_to_hierarchical as f64) / all_paths as f64
    };
    P3SweepDecision {
        use_bottom_up: normalized >= threshold,
        paths_to_random,
        paths_to_hierarchical,
        threshold,
        reason: "path_influence_threshold",
    }
}

/// Builds the source-shaped holder and independent-sweep order for one
/// already-layered hierarchy.
pub(crate) fn p3_hierarchy_plan(arena: &LayeredArena, root: LGraphId) -> P3HierarchyPlan {
    let mut breadth_first_graphs = vec![root];
    let mut index = 0;
    while index < breadth_first_graphs.len() {
        let graph_id = breadth_first_graphs[index];
        let graph = &arena.graphs[graph_id as usize];
        for layer_id in &graph.layers {
            let Some(layer) = arena.layers.get(*layer_id as usize) else {
                continue;
            };
            for node_id in &layer.nodes {
                if let Some(child_graph) = arena.nodes[*node_id as usize].nested_graph {
                    breadth_first_graphs.push(child_graph);
                }
            }
        }
        index += 1;
    }

    let decisions = breadth_first_graphs
        .iter()
        .map(|graph_id| (*graph_id, p3_sweep_decision(arena, *graph_id)))
        .collect::<BTreeMap<_, _>>();
    let graphs_to_sweep_on = breadth_first_graphs
        .iter()
        .rev()
        .filter(|graph_id| decisions[graph_id].use_bottom_up)
        .copied()
        .collect();
    P3HierarchyPlan {
        breadth_first_graphs,
        graphs_to_sweep_on,
        decisions,
    }
}

fn p3_has_no_active_ports_on_side(arena: &LayeredArena, node_id: LNodeId, side: PortSide) -> bool {
    arena.nodes[node_id as usize]
        .ports
        .iter()
        .filter(|port_id| arena.ports[**port_id as usize].side == side)
        .all(|port_id| {
            let port = &arena.ports[*port_id as usize];
            port.incoming_edges
                .iter()
                .chain(&port.outgoing_edges)
                .all(|edge_id| !arena.edges[*edge_id as usize].active)
        })
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum EdgeLabelSideSelection {
    AlwaysUp,
    AlwaysDown,
    DirectionUp,
    DirectionDown,
    SmartUp,
    #[default]
    SmartDown,
}

impl EdgeLabelSideSelection {
    fn transposed(self) -> Self {
        match self {
            Self::AlwaysUp => Self::AlwaysDown,
            Self::AlwaysDown => Self::AlwaysUp,
            Self::DirectionUp => Self::DirectionDown,
            Self::DirectionDown => Self::DirectionUp,
            Self::SmartUp => Self::SmartDown,
            Self::SmartDown => Self::SmartUp,
        }
    }
}

fn parse_edge_label_side_selection(options: &Value) -> EdgeLabelSideSelection {
    match options
        .get("elk.layered.edgeLabels.sideSelection")
        .or_else(|| options.get("org.eclipse.elk.layered.edgeLabels.sideSelection"))
        .and_then(Value::as_str)
        .map(str::to_ascii_uppercase)
        .as_deref()
    {
        Some("ALWAYS_UP") => EdgeLabelSideSelection::AlwaysUp,
        Some("ALWAYS_DOWN") => EdgeLabelSideSelection::AlwaysDown,
        Some("DIRECTION_UP") => EdgeLabelSideSelection::DirectionUp,
        Some("DIRECTION_DOWN") => EdgeLabelSideSelection::DirectionDown,
        Some("SMART_UP") => EdgeLabelSideSelection::SmartUp,
        _ => EdgeLabelSideSelection::SmartDown,
    }
}

/// A label proxy inserted into an edge before layering.
#[derive(Clone, Debug, PartialEq)]
pub struct EdgeLabelDummy {
    /// Arena node whose dimensions reserve the label's lane.
    pub node: LNodeId,
    /// Original wire edge index.
    pub edge: LEdgeId,
    /// Source node of the local edge segment before the proxy was inserted.
    pub source: LNodeId,
    /// Target node of the local edge segment before the proxy was inserted.
    pub target: LNodeId,
    /// Labels represented by this proxy, in source order.
    pub label_indices: Vec<usize>,
    /// Whether every represented label requests ELK's inline edge-label side.
    pub inline: bool,
    /// Selected non-inline side in the internal left-to-right frame.
    pub labels_below: bool,
}

#[derive(Clone, Copy, Debug)]
struct EndpointRef {
    node: LNodeId,
    port: Option<LPortId>,
}

impl LayeredArena {
    /// Imports the root graph and all nested nodes in stable wire order.
    ///
    /// The pinned Go importer allocates arena objects as it walks the model;
    /// this function follows that order and resolves edge endpoints by their
    /// stable ELK IDs. Edge lists are traversed root-first, then recursively
    /// by child owner, independently of their computed layout scope.
    pub fn from_elk_graph(graph: &ElkGraph) -> Result<(Self, LGraphId), LayeredImportError> {
        Self::from_elk_graph_edges(graph, graph.edges_in_source_order())
    }

    pub(crate) fn from_elk_graph_edges<'a>(
        graph: &ElkGraph,
        edges: impl IntoIterator<Item = &'a crate::model::ElkEdge>,
    ) -> Result<(Self, LGraphId), LayeredImportError> {
        let mut arena = Self::default();
        let root = arena.add_graph(
            None,
            parse_direction(&graph.layout_options),
            parse_graph_padding(&graph.layout_options),
            parse_node_node_spacing(&graph.layout_options),
            parse_layer_spacing(&graph.layout_options),
            parse_edge_node_spacing(&graph.layout_options),
            parse_edge_node_vertical_spacing(&graph.layout_options),
            parse_port_port_spacing(&graph.layout_options),
            parse_edge_edge_spacing(&graph.layout_options),
            parse_edge_edge_spacing_between_layers(&graph.layout_options),
            parse_edge_label_spacing(&graph.layout_options),
            parse_label_label_spacing(&graph.layout_options),
            parse_self_loop_spacing(&graph.layout_options),
            parse_thoroughness(&graph.layout_options),
            parse_random_seed(&graph.layout_options),
            parse_hierarchical_sweepiness(&graph.layout_options),
            parse_separate_components(&graph.layout_options),
            parse_aspect_ratio(&graph.layout_options),
            parse_component_spacing(&graph.layout_options),
            parse_bk_fixed_alignment(&graph.layout_options),
            parse_favor_straight_edges(&graph.layout_options),
            parse_bk_edge_straightening(&graph.layout_options),
        );
        arena
            .edge_label_side_modes
            .insert(root, parse_edge_label_side_selection(&graph.layout_options));
        arena.graphs[root as usize].consider_model_order =
            parse_consider_model_order(&graph.layout_options);
        arena.graphs[root as usize].cycle_breaking_model_order =
            parse_model_order_cycle_breaking(Some(&graph.layout_options));
        arena.graphs[root as usize].port_sorting_by_degree =
            parse_port_sorting_by_degree(&graph.layout_options);
        arena.graphs[root as usize].greedy_switch = parse_greedy_switch_mode(&graph.layout_options);
        arena.graphs[root as usize].greedy_switch_activation_threshold =
            parse_greedy_switch_activation_threshold(&graph.layout_options);
        arena.graphs[root as usize].semi_interactive =
            parse_semi_interactive(&graph.layout_options);
        arena.graphs[root as usize].interactive_crossing_minimization =
            parse_interactive_crossing_minimization(&graph.layout_options);
        let mut ids = BTreeMap::<String, EndpointRef>::new();
        let includes_children = hierarchy_includes_children(&graph.layout_options, false);
        arena.graphs[root as usize].includes_children = includes_children;
        arena.graphs[root as usize].layering_strategy =
            parse_layering_strategy(&graph.layout_options);
        arena.graphs[root as usize].coffman_graham_layer_bound =
            parse_coffman_bound(&graph.layout_options);
        for node in &graph.children {
            arena.import_node(root, node, &mut ids, includes_children)?;
        }
        let mut layout_edges = Vec::new();
        for edge in edges {
            let source_id = edge.sources.first().cloned().unwrap_or_default();
            let source = ids.get(&source_id).copied().ok_or_else(|| {
                LayeredImportError::UnknownEndpoint {
                    edge: edge.id.clone(),
                    endpoint: source_id,
                }
            })?;
            let target_id = edge.targets.first().cloned().unwrap_or_default();
            let target = ids.get(&target_id).copied().ok_or_else(|| {
                LayeredImportError::UnknownEndpoint {
                    edge: edge.id.clone(),
                    endpoint: target_id,
                }
            })?;
            let edge_id = arena.add_edge(root, source, target);
            let no_layout = edge
                .layout_options
                .as_ref()
                .and_then(|v| {
                    v.get("elk.noLayout")
                        .or_else(|| v.get("org.eclipse.elk.noLayout"))
                })
                .is_some_and(|v| {
                    v.as_bool() == Some(true)
                        || v.as_str().is_some_and(|v| v.eq_ignore_ascii_case("true"))
                });
            if !no_layout {
                layout_edges.push(edge_id);
            }

            arena.edge_priority_shortness.insert(
                edge_id,
                parse_edge_priority_shortness(edge.layout_options.as_ref()),
            );
            let direction_weight = edge
                .layout_options
                .as_ref()
                .and_then(|options| {
                    options
                        .get("elk.layered.priority.direction")
                        .or_else(|| options.get("org.eclipse.elk.layered.priority.direction"))
                })
                .and_then(Value::as_i64)
                .unwrap_or(0)
                .max(0) as usize
                + 1;
            arena
                .edge_direction_weight
                .insert(edge_id, direction_weight);
            arena
                .edge_thickness
                .insert(edge_id, parse_edge_thickness(edge.layout_options.as_ref()));
        }
        arena.record_source_edge_import_order(root, &layout_edges);
        Ok((arena, root))
    }

    fn record_source_edge_import_order(&mut self, root: LGraphId, edges: &[LEdgeId]) {
        // Codec.UpdateContainment assigns the nearest containing node, then
        // ElkGraphImporter visits those containers breadth-first. Property
        // detection uses adjacency counts at import time, not after P1.
        let mut graphs = vec![root];
        let mut cursor = 0;
        while cursor < graphs.len() {
            let graph = graphs[cursor];
            for node in &self.graphs[graph as usize].layerless_nodes {
                if let Some(child) = self.nodes[*node as usize].nested_graph {
                    graphs.push(child);
                }
            }
            cursor += 1;
        }
        let ranks = graphs
            .iter()
            .enumerate()
            .map(|(i, g)| (*g, i))
            .collect::<BTreeMap<_, _>>();
        let layout_edges = edges.iter().copied().collect::<BTreeSet<_>>();
        let mut ordered = self
            .edges
            .iter()
            .enumerate()
            .map(|(id, edge)| {
                let graph = if node_is_ancestor(self, edge.source, edge.target) {
                    self.nodes[edge.source as usize]
                        .nested_graph
                        .unwrap_or(root)
                } else if node_is_ancestor(self, edge.target, edge.source) {
                    self.nodes[edge.target as usize]
                        .nested_graph
                        .unwrap_or(root)
                } else {
                    lowest_common_graph(
                        self,
                        self.nodes[edge.source as usize].graph,
                        self.nodes[edge.target as usize].graph,
                        root,
                    )
                };
                (id as LEdgeId, graph)
            })
            .collect::<Vec<_>>();
        ordered.sort_by_key(|(_, graph)| ranks.get(graph).copied().unwrap_or(usize::MAX));
        let mut incoming = vec![0_usize; self.ports.len()];
        let mut outgoing = incoming.clone();
        for (model_order, (edge, graph)) in ordered.into_iter().enumerate() {
            // importEdges assigns MODEL_ORDER in breadth-first containment
            // order before filtering NoLayout. Wire IDs retain their identity.
            self.edge_model_order.insert(edge, model_order);
            if !layout_edges.contains(&edge) {
                continue;
            }
            let edge = &self.edges[edge as usize];
            if let Some(port) = edge.source_port {
                outgoing[port as usize] += 1;
            }
            if let Some(port) = edge.target_port {
                incoming[port as usize] += 1;
            }
            if [edge.source_port, edge.target_port]
                .into_iter()
                .flatten()
                .any(|port| incoming[port as usize] > 1 || outgoing[port as usize] > 1)
            {
                self.graphs[graph as usize].has_hyperedges = true;
            }
        }
        let order = &self.edge_model_order;
        for port in &mut self.ports {
            port.incoming_edges.sort_by_key(|id| order[id]);
            port.outgoing_edges.sort_by_key(|id| order[id]);
        }
    }

    /// Inserts the pinned ELK center-label proxy used by the layered pipeline.
    ///
    /// D2 marks its ordinary edge labels inline. The source ELK processor
    /// turns those labels into real rank nodes before layering, allowing both
    /// the label dimensions and the two adjacent inter-layer gaps to affect
    /// node coordinates. `labels_transposed` reflects direction normalization
    /// that has already happened to node and port geometry.
    pub fn insert_center_edge_label_dummies(
        &mut self,
        graph_id: LGraphId,
        wire_edges: &[crate::model::ElkEdge],
        labels_transposed: bool,
    ) -> Result<(), LayeringError> {
        let root = self
            .graphs
            .iter()
            .position(|graph| graph.parent_node.is_none())
            .unwrap_or(0) as LGraphId;
        let edge_label_spacing = self.graphs[graph_id as usize].edge_label_spacing;
        let label_label_spacing = self.graphs[graph_id as usize].label_label_spacing;
        // LabelDummyInserter visits a snapshot of layerless nodes, then each
        // node's stored port/outgoing-edge order. Wire-array order changes
        // proxy allocation and the identities receiving later P3 random ties.
        let mut edge_visit_order = BTreeMap::new();
        for node in self.graphs[graph_id as usize].layerless_nodes.clone() {
            let mut outgoing = self.node_outgoing_edges(node);
            // Direct stage callers may not have materialized default ports.
            // Those ports append in edge import order after explicit ports.
            outgoing.extend(
                self.outgoing_edges(node)
                    .filter_map(|(id, edge)| edge.source_port.is_none().then_some(id)),
            );
            for edge in outgoing {
                let index = edge_visit_order.len();
                edge_visit_order.insert(edge, index);
            }
        }
        let mut pending = Vec::new();
        for (edge_index, wire_edge) in wire_edges.iter().enumerate() {
            let Some(original_edge) = self.edges.get(edge_index) else {
                continue;
            };
            if original_edge.source == original_edge.target {
                continue;
            }
            let labels = wire_edge
                .labels
                .iter()
                .enumerate()
                .filter(|(_, label)| {
                    let placement = label
                        .layout_options
                        .as_ref()
                        .and_then(|options| {
                            options
                                .get("elk.edgeLabels.placement")
                                .or_else(|| options.get("org.eclipse.elk.edgeLabels.placement"))
                        })
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("CENTER");
                    placement.eq_ignore_ascii_case("CENTER")
                })
                .map(|(index, label)| {
                    let (width, height) = if labels_transposed {
                        (label.height, label.width)
                    } else {
                        (label.width, label.height)
                    };
                    let inline = label
                        .layout_options
                        .as_ref()
                        .and_then(|options| {
                            options
                                .get("elk.edgeLabels.inline")
                                .or_else(|| options.get("org.eclipse.elk.edgeLabels.inline"))
                        })
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false);
                    (index, width.max(0.0), height.max(0.0), inline)
                })
                .collect::<Vec<_>>();
            if labels.is_empty() {
                continue;
            }

            // elk-go first splits cross-hierarchy connections into one local
            // edge per graph, then inserts each centered label in the
            // endpoint LCA's graph. Edges whose endpoints already share a
            // child graph keep their labels there. The D2 wire format still
            // stores every edge at the root, so select that scope from the
            // endpoint ownership rather than from the wire edge's array.
            //
            // compound.go moveCrossHierarchyLabels assigns CENTER labels to
            // shallowestHierarchySegment. That is the central LCA segment
            // when one exists. An edge between an ancestor and its
            // descendant reuses the ancestor's endpoint and has no central
            // segment, so its label moves to the shallowest boundary segment,
            // which lies in the ancestor's own child graph.
            let source_graph = self.nodes[original_edge.source as usize].graph;
            let target_graph = self.nodes[original_edge.target as usize].graph;
            let label_graph = if node_is_ancestor(self, original_edge.source, original_edge.target)
            {
                self.nodes[original_edge.source as usize]
                    .nested_graph
                    .unwrap_or(root)
            } else if node_is_ancestor(self, original_edge.target, original_edge.source) {
                self.nodes[original_edge.target as usize]
                    .nested_graph
                    .unwrap_or(root)
            } else {
                lowest_common_graph(self, source_graph, target_graph, root)
            };
            if label_graph != graph_id {
                continue;
            }
            let layout_edge_id = if original_edge.active {
                edge_index as LEdgeId
            } else {
                original_edge
                    .hierarchy_segments
                    .iter()
                    .copied()
                    .find(|segment_id| {
                        let segment = &self.edges[*segment_id as usize];
                        segment.active
                            && self.nodes[segment.source as usize].graph == graph_id
                            && self.nodes[segment.target as usize].graph == graph_id
                    })
                    .unwrap_or(edge_index as LEdgeId)
            };
            let Some(layout_edge) = self
                .edges
                .get(layout_edge_id as usize)
                .filter(|edge| edge.active)
            else {
                continue;
            };
            if !edge_visit_order.contains_key(&layout_edge_id) {
                return Err(LayeringError::Import(format!(
                    "layered: label edge {layout_edge_id} is absent from graph {graph_id} outgoing traversal"
                )));
            }
            let source = layout_edge.source;
            let old_target = layout_edge.target;
            let thickness = self
                .edge_thickness
                .get(&(edge_index as LEdgeId))
                .copied()
                .unwrap_or(1.0);
            // createLabelDummy first calls SplitEdgeAtDummy, which installs
            // edge thickness in Size.Y before represented labels are added.
            let mut width: f64 = 0.0;
            let mut height: f64 = thickness;
            for (_, label_width, label_height, _) in &labels {
                if labels_transposed {
                    width += *label_width + label_label_spacing;
                    height = height.max(*label_height);
                } else {
                    width = width.max(*label_width);
                    height += *label_height + label_label_spacing;
                }
            }
            if labels_transposed {
                width -= label_label_spacing;
                height += edge_label_spacing + thickness;
            } else {
                height += edge_label_spacing - label_label_spacing + thickness;
            }
            let inline = labels.iter().all(|(_, _, _, inline)| *inline);
            pending.push((
                edge_index as LEdgeId,
                layout_edge_id,
                source,
                old_target,
                labels,
                width,
                height,
                thickness,
                inline,
            ));
        }

        pending.sort_by_key(|entry| edge_visit_order[&entry.1]);
        for (
            edge_id,
            layout_edge_id,
            source,
            old_target,
            labels,
            width,
            height,
            thickness,
            inline,
        ) in pending
        {
            let node_id = self.nodes.len() as LNodeId;
            let traversal_index = self.graphs[graph_id as usize].layerless_nodes.len();
            let source_id = format!("__edge_label_dummy_{edge_id}");
            self.nodes.push(LNode {
                layer_constraint: NodeLayerConstraint::None,
                alignment: NodeAlignment::Automatic,
                graph: graph_id,
                traversal_index,
                layer: None,
                source_id,
                node_type: LayeredNodeType::Label,
                nested_graph: None,
                ports: Vec::new(),
                port_constraints: PortConstraints::FixedPosition,
                self_loop_distribution: SelfLoopDistribution::North,
                width,
                height,
                fixed_width: false,
                fixed_height: false,
                margin_top: 0.0,
                margin_right: 0.0,
                margin_bottom: 0.0,
                margin_left: 0.0,
                margins_in_wire_dimensions: false,
                computed_margins: [0.0; 4],
                label_margin_top: 0.0,
                label_margin_right: 0.0,
                label_margin_bottom: 0.0,
                label_margin_left: 0.0,
                position_x: 0.0,
                position_y: 0.0,
            });
            self.graphs[graph_id as usize].layerless_nodes.push(node_id);
            self.split_edge_at_dummy(layout_edge_id, node_id, old_target)?;
            let port_y = (thickness / 2.0).floor();
            for port in self.nodes[node_id as usize].ports.clone() {
                self.ports[port as usize].y = port_y;
            }
            self.edge_label_dummies.push(EdgeLabelDummy {
                node: node_id,
                edge: edge_id,
                source,
                target: old_target,
                label_indices: labels.into_iter().map(|(index, _, _, _)| index).collect(),
                inline,
                labels_below: true,
            });
        }
        Ok(())
    }

    pub(crate) fn edge_thickness_for_wire(&self, edge: LEdgeId) -> f64 {
        self.edge_thickness.get(&edge).copied().unwrap_or(1.0)
    }

    /// Retains the existing entry point while applying the complete center-
    /// label side selection, including non-inline proxy ports.
    pub fn select_inline_edge_label_sides(&mut self, graph_id: LGraphId) {
        self.select_center_edge_label_sides(graph_id);
    }

    pub(crate) fn transpose_label_side_modes(&mut self) {
        for mode in self.edge_label_side_modes.values_mut() {
            *mode = mode.transposed();
        }
    }

    fn label_chain_end_nodes(&self, node: LNodeId) -> Option<(LNodeId, LNodeId)> {
        self.long_edge_endpoints.get(&node).map(|(source, target)| {
            (
                self.ports[*source as usize].node,
                self.ports[*target as usize].node,
            )
        })
    }

    fn apply_center_label_side(&mut self, node_id: LNodeId, below: bool) {
        let Some(index) = self
            .edge_label_dummies
            .iter()
            .position(|dummy| dummy.node == node_id)
        else {
            return;
        };
        let graph_id = self.nodes[node_id as usize].graph;
        let dummy = &mut self.edge_label_dummies[index];
        dummy.labels_below = below;
        let thickness = self.edge_thickness.get(&dummy.edge).copied().unwrap_or(1.0);
        let node = &mut self.nodes[node_id as usize];
        let port_y = if dummy.inline {
            let spacing = self.graphs[graph_id as usize].edge_label_spacing;
            let port_y = (node.height - spacing - thickness).ceil() / 2.0;
            node.height -= spacing;
            node.height -= thickness;
            port_y
        } else if !below {
            node.height - (thickness / 2.0).ceil()
        } else {
            return;
        };
        for port in &node.ports {
            self.ports[*port as usize].y = port_y;
        }
    }

    fn apply_center_label_run(&mut self, run: &[LNodeId], below: bool) {
        if run.len() == 2 {
            self.apply_center_label_side(run[0], false);
            self.apply_center_label_side(run[1], true);
        } else {
            for node in run {
                self.apply_center_label_side(*node, below);
            }
        }
    }

    fn select_smart_center_run(
        &mut self,
        run: &[LNodeId],
        labels: usize,
        top: bool,
        bottom: bool,
        below: bool,
    ) {
        if run.is_empty() {
            return;
        }
        if top
            && (!bottom || run.len() > 1)
            && labels == 1
            && self.nodes[run[0] as usize].node_type == LayeredNodeType::Label
        {
            self.apply_center_label_side(run[0], false);
        } else if bottom
            && (!top || run.len() > 1)
            && labels == 1
            && self.nodes[*run.last().unwrap() as usize].node_type == LayeredNodeType::Label
        {
            self.apply_center_label_side(*run.last().unwrap(), true);
        } else if run.len() == 2 {
            self.apply_center_label_side(run[0], false);
            self.apply_center_label_side(run[1], true);
        } else {
            let mut start = 0;
            let mut previous = None;
            for (index, node) in run.iter().enumerate() {
                let ends = self.label_chain_end_nodes(*node);
                if ends != previous {
                    self.apply_center_label_run(&run[start..index], below);
                    start = index;
                    previous = ends;
                }
            }
            self.apply_center_label_run(&run[start..], below);
        }
    }

    fn select_center_edge_label_sides(&mut self, graph_id: LGraphId) {
        let mode = self
            .edge_label_side_modes
            .get(&graph_id)
            .copied()
            .unwrap_or_default();
        let below = matches!(
            mode,
            EdgeLabelSideSelection::AlwaysDown
                | EdgeLabelSideSelection::DirectionDown
                | EdgeLabelSideSelection::SmartDown
        );
        let smart = matches!(
            mode,
            EdgeLabelSideSelection::SmartUp | EdgeLabelSideSelection::SmartDown
        );
        for layer in self.graphs[graph_id as usize].layers.clone() {
            let nodes = self.layers[layer as usize].nodes.clone();
            if smart {
                let mut run = Vec::new();
                let mut labels = 0;
                let mut top = true;
                for node in nodes {
                    match self.nodes[node as usize].node_type {
                        LayeredNodeType::Label => {
                            run.push(node);
                            labels += 1;
                        }
                        LayeredNodeType::LongEdge => run.push(node),
                        _ => {
                            self.select_smart_center_run(&run, labels, top, false, below);
                            run.clear();
                            labels = 0;
                            top = false;
                        }
                    }
                }
                self.select_smart_center_run(&run, labels, top, true, below);
            } else {
                for node in nodes {
                    if self.nodes[node as usize].node_type != LayeredNodeType::Label {
                        continue;
                    }
                    let directed = matches!(
                        mode,
                        EdgeLabelSideSelection::DirectionUp | EdgeLabelSideSelection::DirectionDown
                    );
                    let points_right = if directed {
                        let incoming = self.nodes[node as usize]
                            .ports
                            .iter()
                            .flat_map(|p| self.ports[*p as usize].incoming_edges.iter())
                            .next();
                        let outgoing = self.nodes[node as usize]
                            .ports
                            .iter()
                            .flat_map(|p| self.ports[*p as usize].outgoing_edges.iter())
                            .next();
                        incoming.zip(outgoing).is_none_or(|(a, b)| {
                            !self.edges[*a as usize].reversed || !self.edges[*b as usize].reversed
                        })
                    } else {
                        true
                    };
                    self.apply_center_label_side(node, if points_right { below } else { !below });
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn add_graph(
        &mut self,
        parent_node: Option<LNodeId>,
        direction: LayerDirection,
        padding: Option<GraphPadding>,
        node_node_spacing: f64,
        layer_spacing: f64,
        edge_node_spacing: f64,
        edge_node_vertical_spacing: f64,
        port_port_spacing: f64,
        edge_edge_spacing: f64,
        edge_edge_spacing_between_layers: f64,
        edge_label_spacing: f64,
        label_label_spacing: f64,
        self_loop_spacing: f64,
        thoroughness: usize,
        random_seed: i64,
        hierarchical_sweepiness: Option<f64>,
        separate_components: bool,
        aspect_ratio: f64,
        component_spacing: f64,
        fixed_alignment: BkFixedAlignment,
        favor_straight_edges: bool,
        bk_edge_straightening: bool,
    ) -> LGraphId {
        let id = self.graphs.len() as LGraphId;
        self.graphs.push(LGraph {
            has_hyperedges: false,
            parent_node,
            direction,
            padding,
            node_node_spacing,
            layer_spacing,
            edge_node_spacing,
            edge_node_vertical_spacing,
            port_port_spacing,
            edge_edge_spacing,
            edge_edge_spacing_between_layers,
            edge_label_spacing,
            label_label_spacing,
            self_loop_spacing,
            thoroughness,
            random_seed,
            hierarchical_sweepiness,
            separate_components,
            aspect_ratio,
            component_spacing,
            fixed_alignment,
            favor_straight_edges,
            bk_edge_straightening,
            ..LGraph::default()
        });
        id
    }

    fn take_graph_random(&mut self, graph_id: LGraphId) -> JavaRandom {
        let graph_index = graph_id as usize;
        while self.graph_randoms.len() <= graph_index {
            let index = self.graph_randoms.len();
            let seed = self.graphs[index].random_seed;
            #[cfg(feature = "diagnostic-traces")]
            let stream = format!("elk-graph-{index}");
            #[cfg(not(feature = "diagnostic-traces"))]
            let stream = "elk-graph";
            // The traced stream name is a String; the untraced one is a &str.
            #[cfg_attr(not(feature = "diagnostic-traces"), allow(clippy::needless_borrow))]
            self.graph_randoms
                .push(JavaRandom::with_stream(seed, &stream));
        }
        std::mem::replace(
            &mut self.graph_randoms[graph_index],
            JavaRandom::with_stream(0, "elk-rng-temporary"),
        )
    }

    fn ordering_random(&mut self, graph_id: LGraphId, seed: i64, stream: &str) -> JavaRandom {
        if self.graphs[graph_id as usize].cycle_random_consumed
            || self.shared_graph_randoms.contains(&graph_id)
        {
            self.take_graph_random(graph_id)
        } else {
            JavaRandom::with_stream(seed, stream)
        }
    }

    fn restore_graph_random(&mut self, graph_id: LGraphId, random: JavaRandom) {
        let graph_index = graph_id as usize;
        while self.graph_randoms.len() <= graph_index {
            let index = self.graph_randoms.len();
            let seed = self.graphs[index].random_seed;
            #[cfg(feature = "diagnostic-traces")]
            let stream = format!("elk-graph-{index}");
            #[cfg(not(feature = "diagnostic-traces"))]
            let stream = "elk-graph";
            // The traced stream name is a String; the untraced one is a &str.
            #[cfg_attr(not(feature = "diagnostic-traces"), allow(clippy::needless_borrow))]
            self.graph_randoms
                .push(JavaRandom::with_stream(seed, &stream));
        }
        self.graph_randoms[graph_index] = random;
    }

    fn import_node(
        &mut self,
        graph_id: LGraphId,
        node: &crate::model::ElkNode,
        ids: &mut BTreeMap<String, EndpointRef>,
        includes_children: bool,
    ) -> Result<LNodeId, LayeredImportError> {
        if ids.contains_key(&node.id) {
            return Err(LayeredImportError::DuplicateNodeId(node.id.clone()));
        }
        if node
            .layout_options
            .as_ref()
            .and_then(|v| {
                v.get("elk.hypernode")
                    .or_else(|| v.get("org.eclipse.elk.hypernode"))
            })
            .is_some_and(|v| {
                v.as_bool() == Some(true)
                    || v.as_str().is_some_and(|v| v.eq_ignore_ascii_case("true"))
            })
        {
            self.graphs[graph_id as usize].has_hyperedges = true;
        }
        let node_id = self.nodes.len() as LNodeId;
        let traversal_index = self.graphs[graph_id as usize].layerless_nodes.len();
        let node_size_minimum = node
            .layout_options
            .as_ref()
            .and_then(parse_node_size_minimum);
        let margins = parse_node_margins(node.layout_options.as_ref());
        // D2's private transfer fields are not registered ELK margins.
        // Its outside bands already form part of the input width/height;
        // elk-go imports only the public org.eclipse.elk.margins property.
        let margins_in_wire_dimensions = false;
        // The v0.9.0 ELK importer initially leaves D2 node labels at their
        // default (0,0) placement. Its innermost-margin pass therefore keeps
        // any label overflow to the right or below the node as a component
        // margin. Capture that geometry before the vertical transpose.
        let label_margin_right = node
            .labels
            .iter()
            .map(|label| (label.width - node.width).max(0.0))
            .fold(0.0, f64::max);
        let label_margin_bottom = node
            .labels
            .iter()
            .map(|label| (label.height - node.height).max(0.0))
            .fold(0.0, f64::max);
        self.nodes.push(LNode {
            layer_constraint: parse_node_layer_constraint(node.layout_options.as_ref()),
            alignment: parse_node_alignment(node.layout_options.as_ref()),
            graph: graph_id,
            traversal_index,
            layer: None,
            source_id: node.id.clone(),
            node_type: LayeredNodeType::Normal,
            nested_graph: None,
            ports: Vec::new(),
            port_constraints: parse_port_constraints(node.layout_options.as_ref()),
            self_loop_distribution: parse_self_loop_distribution(node.layout_options.as_ref()),
            width: node.width,
            height: node.height,
            fixed_width: node
                .layout_options
                .as_ref()
                .and_then(|options| options.get("d2-rs.fixedWidth"))
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
            fixed_height: node
                .layout_options
                .as_ref()
                .and_then(|options| options.get("d2-rs.fixedHeight"))
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
            margin_top: margins[0],
            margin_right: margins[1],
            margin_bottom: margins[2],
            margin_left: margins[3],
            margins_in_wire_dimensions,
            computed_margins: [0.0; 4],
            label_margin_top: 0.0,
            label_margin_right,
            label_margin_bottom,
            label_margin_left: 0.0,
            position_x: node.x,
            position_y: node.y,
        });
        if let Some(minimum) = node_size_minimum {
            self.node_size_minimum.insert(node_id, minimum);
        }
        if !node.children.is_empty() && !node.labels.is_empty() {
            self.compound_label_boxes.insert(
                node_id,
                node.labels
                    .iter()
                    .map(|label| [label.x, label.y, label.width, label.height])
                    .collect(),
            );
        }
        self.graphs[graph_id as usize].layerless_nodes.push(node_id);
        ids.insert(
            node.id.clone(),
            EndpointRef {
                node: node_id,
                port: None,
            },
        );
        // ELK edges may target a port instead of a node. The Java importer
        // resolves both IDs to the owning LNode before entering the arena.
        let mut port_ids = Vec::with_capacity(node.ports.len());
        for (model_order, port) in node.ports.iter().enumerate() {
            if ids.contains_key(&port.id) {
                return Err(LayeredImportError::DuplicateNodeId(port.id.clone()));
            }
            let port_id = self.add_port(node_id, port, model_order);
            ids.insert(
                port.id.clone(),
                EndpointRef {
                    node: node_id,
                    port: Some(port_id),
                },
            );
            port_ids.push(port_id);
        }
        self.nodes[node_id as usize].ports = port_ids;

        if !node.children.is_empty() {
            // Importer.createGraph copies this compound's properties. Apart
            // from hierarchy direction, absent values use source defaults.
            let nested_options = node.layout_options.as_ref().unwrap_or(&Value::Null);
            let nested_includes_children =
                hierarchy_includes_children(nested_options, includes_children);
            // Importer.importNodes uses one direction for every graph in the
            // included hierarchy, even when a compound has its own option.
            let nested_direction = if includes_children && nested_includes_children {
                self.graphs[graph_id as usize].direction
            } else {
                node.layout_options
                    .as_ref()
                    .and_then(parse_direction_option)
                    .unwrap_or(self.graphs[graph_id as usize].direction)
            };
            let nested = self.add_graph(
                Some(node_id),
                nested_direction,
                node.layout_options.as_ref().and_then(parse_graph_padding),
                parse_node_node_spacing(nested_options),
                parse_layer_spacing(nested_options),
                parse_edge_node_spacing(nested_options),
                parse_edge_node_vertical_spacing(nested_options),
                parse_port_port_spacing(nested_options),
                parse_edge_edge_spacing(nested_options),
                parse_edge_edge_spacing_between_layers(nested_options),
                parse_edge_label_spacing(nested_options),
                parse_label_label_spacing(nested_options),
                parse_self_loop_spacing(nested_options),
                parse_thoroughness(nested_options),
                parse_random_seed(nested_options),
                parse_hierarchical_sweepiness(nested_options),
                parse_separate_components(nested_options),
                parse_aspect_ratio(nested_options),
                parse_component_spacing(nested_options),
                parse_bk_fixed_alignment(nested_options),
                parse_favor_straight_edges(nested_options),
                parse_bk_edge_straightening(nested_options),
            );
            self.graphs[nested as usize].includes_children = nested_includes_children;
            self.graphs[nested as usize].layering_strategy =
                parse_layering_strategy(nested_options);
            self.graphs[nested as usize].coffman_graham_layer_bound =
                parse_coffman_bound(nested_options);
            self.edge_label_side_modes
                .insert(nested, parse_edge_label_side_selection(nested_options));
            self.graphs[nested as usize].consider_model_order =
                parse_consider_model_order(nested_options);
            self.graphs[nested as usize].cycle_breaking_model_order =
                parse_model_order_cycle_breaking(Some(nested_options));
            self.graphs[nested as usize].port_sorting_by_degree =
                parse_port_sorting_by_degree(nested_options);
            self.graphs[nested as usize].greedy_switch = parse_greedy_switch_mode(nested_options);
            self.graphs[nested as usize].greedy_switch_activation_threshold =
                parse_greedy_switch_activation_threshold(nested_options);
            self.graphs[nested as usize].semi_interactive = parse_semi_interactive(nested_options);
            self.graphs[nested as usize].interactive_crossing_minimization =
                parse_interactive_crossing_minimization(nested_options);
            self.nodes[node_id as usize].nested_graph = Some(nested);
            for child in &node.children {
                self.import_node(nested, child, ids, nested_includes_children)?;
            }
        }
        Ok(node_id)
    }

    fn add_port(
        &mut self,
        node: LNodeId,
        port: &crate::model::ElkPort,
        model_order: usize,
    ) -> LPortId {
        let id = self.ports.len() as LPortId;
        let switch_sides = port
            .layout_options
            .get("org.eclipse.elk.layered.allowNonFlowPortsToSwitchSides")
            .or_else(|| {
                port.layout_options
                    .get("elk.layered.allowNonFlowPortsToSwitchSides")
            });
        if switch_sides.is_some_and(|value| {
            value.as_bool() == Some(true)
                || value
                    .as_str()
                    .is_some_and(|value| value.eq_ignore_ascii_case("true"))
        }) {
            self.non_flow_switch_ports.insert(id);
        }
        self.ports.push(LPort {
            node,
            source_id: Some(port.id.clone()),
            model_order: Some(model_order),
            side: parse_port_side(&port.layout_options),
            index: parse_port_index(&port.layout_options),
            x: port.x,
            y: port.y,
            width: port.width,
            height: port.height,
            implicit: false,
            incoming_edges: Vec::new(),
            outgoing_edges: Vec::new(),
        });
        id
    }

    fn add_implicit_port(&mut self, node: LNodeId, source: bool) -> LPortId {
        let owner = &self.nodes[node as usize];
        let port = LPort {
            node,
            source_id: None,
            model_order: None,
            side: if source {
                PortSide::East
            } else {
                PortSide::West
            },
            index: None,
            x: if source { owner.width } else { 0.0 },
            y: owner.height / 2.0,
            width: 0.0,
            height: 0.0,
            implicit: true,
            incoming_edges: Vec::new(),
            outgoing_edges: Vec::new(),
        };
        let id = self.ports.len() as LPortId;
        self.ports.push(port);
        self.nodes[node as usize].ports.push(id);
        id
    }

    fn add_edge(
        &mut self,
        _graph_id: LGraphId,
        source: EndpointRef,
        target: EndpointRef,
    ) -> LEdgeId {
        let id = self.edges.len() as LEdgeId;
        self.edges.push(LEdge {
            source: source.node,
            target: target.node,
            source_port: source.port,
            target_port: target.port,
            reversed: false,
            active: true,
            bend_points: Vec::new(),
            junction_points: Vec::new(),
            hierarchy_segments: Vec::new(),
        });
        self.edge_model_order.insert(id, id as usize);
        self.edge_priority_shortness.insert(id, 1.0);
        self.edge_thickness.insert(id, 1.0);
        if let Some(port) = source.port {
            self.ports[port as usize].outgoing_edges.push(id);
        }
        if let Some(port) = target.port {
            self.ports[port as usize].incoming_edges.push(id);
        }
        id
    }

    fn add_long_edge_dummy(
        &mut self,
        graph_id: LGraphId,
        layer_id: LayerId,
        edge_id: LEdgeId,
    ) -> LNodeId {
        let node_id = self.nodes.len() as LNodeId;
        self.nodes.push(LNode {
            layer_constraint: NodeLayerConstraint::None,
            alignment: NodeAlignment::Automatic,
            graph: graph_id,
            traversal_index: self.layers[layer_id as usize].nodes.len(),
            layer: Some(layer_id),
            source_id: format!("__long_edge_dummy_{edge_id}_{node_id}"),
            node_type: LayeredNodeType::LongEdge,
            nested_graph: None,
            ports: Vec::new(),
            // intermediate/long_edge.go fixes both dummy ports at their
            // imported center. Leaving this undefined lets the generic port
            // distributor move the dummy endpoints before BK sees them.
            port_constraints: PortConstraints::FixedPosition,
            self_loop_distribution: SelfLoopDistribution::North,
            width: 0.0,
            height: self.edge_thickness.get(&edge_id).copied().unwrap_or(1.0),
            fixed_width: false,
            fixed_height: false,
            margin_top: 0.0,
            margin_right: 0.0,
            margin_bottom: 0.0,
            margin_left: 0.0,
            margins_in_wire_dimensions: false,
            computed_margins: [0.0; 4],
            label_margin_top: 0.0,
            label_margin_right: 0.0,
            label_margin_bottom: 0.0,
            label_margin_left: 0.0,
            position_x: 0.0,
            position_y: 0.0,
        });
        self.layers[layer_id as usize].nodes.push(node_id);
        node_id
    }

    fn add_dummy_port(&mut self, node: LNodeId, side: PortSide) -> LPortId {
        let port_id = self.ports.len() as LPortId;
        self.ports.push(LPort {
            node,
            source_id: None,
            model_order: None,
            side,
            index: None,
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
            implicit: false,
            incoming_edges: Vec::new(),
            outgoing_edges: Vec::new(),
        });
        self.nodes[node as usize].ports.push(port_id);
        port_id
    }

    fn split_edge_at_dummy(
        &mut self,
        edge_id: LEdgeId,
        dummy: LNodeId,
        old_target: LNodeId,
    ) -> Result<LEdgeId, LayeringError> {
        let original = self
            .edges
            .get(edge_id as usize)
            .ok_or(LayeringError::UnknownEdge(edge_id))?
            .clone();
        let has_label =
            if self.nodes[original.source as usize].node_type == LayeredNodeType::LongEdge {
                self.long_edge_label_dummies.contains(&original.source)
            } else {
                self.nodes[original.source as usize].node_type == LayeredNodeType::Label
                    || self.nodes[old_target as usize].node_type == LayeredNodeType::Label
            };
        if has_label {
            self.long_edge_label_dummies.insert(dummy);
        }
        let inherited = if matches!(
            self.nodes[original.source as usize].node_type,
            LayeredNodeType::LongEdge | LayeredNodeType::Label
        ) {
            self.long_edge_endpoints.get(&original.source).copied()
        } else if self.nodes[old_target as usize].node_type == LayeredNodeType::Label {
            self.long_edge_endpoints.get(&old_target).copied()
        } else {
            original.source_port.zip(original.target_port)
        };
        if let Some(ends) = inherited {
            self.long_edge_endpoints.insert(dummy, ends);
        }
        let target_port = {
            let edge = self
                .edges
                .get(edge_id as usize)
                .ok_or(LayeringError::UnknownEdge(edge_id))?;
            edge.target_port
        };
        if let Some(port_id) = target_port {
            self.ports[port_id as usize]
                .incoming_edges
                .retain(|candidate| *candidate != edge_id);
        }
        let input = self.add_dummy_port(dummy, PortSide::West);
        let output = self.add_dummy_port(dummy, PortSide::East);
        {
            let edge = &mut self.edges[edge_id as usize];
            edge.target = dummy;
            edge.target_port = Some(input);
        }
        self.ports[input as usize].incoming_edges.push(edge_id);
        let new_edge_id = self.edges.len() as LEdgeId;
        self.edges.push(LEdge {
            source: dummy,
            target: old_target,
            source_port: Some(output),
            target_port,
            reversed: original.reversed,
            active: true,
            bend_points: Vec::new(),
            junction_points: Vec::new(),
            hierarchy_segments: Vec::new(),
        });
        if let Some(order) = self.edge_model_order.get(&edge_id).copied() {
            self.edge_model_order.insert(new_edge_id, order);
        }
        if let Some(weight) = self.edge_priority_shortness.get(&edge_id).copied() {
            self.edge_priority_shortness.insert(new_edge_id, weight);
        }
        if let Some(weight) = self.edge_direction_weight.get(&edge_id).copied() {
            self.edge_direction_weight.insert(new_edge_id, weight);
        }
        if let Some(thickness) = self.edge_thickness.get(&edge_id).copied() {
            self.edge_thickness.insert(new_edge_id, thickness);
        }
        self.ports[output as usize].outgoing_edges.push(new_edge_id);
        if let Some(port_id) = target_port {
            self.ports[port_id as usize]
                .incoming_edges
                .push(new_edge_id);
        }
        Ok(new_edge_id)
    }

    fn reverse_edge(&mut self, edge_id: LEdgeId) {
        let (source_port, target_port) = {
            let edge = &self.edges[edge_id as usize];
            (edge.source_port, edge.target_port)
        };
        if let Some(port_id) = source_port {
            self.ports[port_id as usize]
                .outgoing_edges
                .retain(|candidate| *candidate != edge_id);
        }
        if let Some(port_id) = target_port {
            self.ports[port_id as usize]
                .incoming_edges
                .retain(|candidate| *candidate != edge_id);
        }
        let edge = &mut self.edges[edge_id as usize];
        std::mem::swap(&mut edge.source, &mut edge.target);
        std::mem::swap(&mut edge.source_port, &mut edge.target_port);
        edge.reversed = !edge.reversed;
        edge.bend_points.reverse();
        if let Some(port_id) = edge.source_port {
            self.ports[port_id as usize].outgoing_edges.push(edge_id);
        }
        if let Some(port_id) = edge.target_port {
            self.ports[port_id as usize].incoming_edges.push(edge_id);
        }
    }

    /// Returns outgoing edges in arena insertion order.
    pub fn outgoing_edges(&self, node: LNodeId) -> impl Iterator<Item = (LEdgeId, &LEdge)> {
        self.edges
            .iter()
            .enumerate()
            .filter(move |(_, edge)| edge.active && edge.source == node)
            .map(|(id, edge)| (id as LEdgeId, edge))
    }

    /// Returns a node's outgoing edges in elk-go's node-port order.
    ///
    /// The layered implementation walks `Node.Ports` and then each port's
    /// outgoing edge slice. Scanning the arena by endpoint is not equivalent:
    /// removed/replaced or cross-scope edges can retain the same node endpoint
    /// while no longer belonging to that node's live port adjacency.
    fn node_outgoing_edges(&self, node: LNodeId) -> Vec<LEdgeId> {
        self.nodes[node as usize]
            .ports
            .iter()
            .flat_map(|port_id| self.ports[*port_id as usize].outgoing_edges.iter().copied())
            .filter(|edge_id| self.edges[*edge_id as usize].active)
            .collect()
    }

    /// Returns incoming edges in arena insertion order.
    pub fn incoming_edges(&self, node: LNodeId) -> impl Iterator<Item = (LEdgeId, &LEdge)> {
        self.edges
            .iter()
            .enumerate()
            .filter(move |(_, edge)| edge.active && edge.target == node)
            .map(|(id, edge)| (id as LEdgeId, edge))
    }

    /// Returns a node's incoming edges in elk-go's node-port order.
    fn node_incoming_edges(&self, node: LNodeId) -> Vec<LEdgeId> {
        self.nodes[node as usize]
            .ports
            .iter()
            .flat_map(|port_id| self.ports[*port_id as usize].incoming_edges.iter().copied())
            .filter(|edge_id| self.edges[*edge_id as usize].active)
            .collect()
    }

    /// Returns all incident edges in arena insertion order.
    pub fn connected_edges(&self, node: LNodeId) -> impl Iterator<Item = (LEdgeId, &LEdge)> {
        self.edges
            .iter()
            .enumerate()
            .filter(move |(_, edge)| edge.active && (edge.source == node || edge.target == node))
            .map(|(id, edge)| (id as LEdgeId, edge))
    }

    /// Returns incident edges in elk-go's node-port, incoming-then-outgoing order.
    fn node_connected_edges_source_order(&self, node: LNodeId) -> Vec<(LEdgeId, &LEdge)> {
        let mut connected = Vec::new();
        let mut seen = BTreeSet::new();
        for &port_id in &self.nodes[node as usize].ports {
            let port = &self.ports[port_id as usize];
            for &edge_id in port.incoming_edges.iter().chain(&port.outgoing_edges) {
                if let Some(edge) = self.edges.get(edge_id as usize).filter(|edge| edge.active) {
                    connected.push((edge_id, edge));
                    seen.insert(edge_id);
                }
            }
        }
        // Direct Rust phase tests may keep node endpoints without materializing
        // ELK's implicit ports. The wire pipeline always does materialize them;
        // retain arena order for those endpoint edges so the standalone phase
        // API remains useful on its smaller graph representation.
        for (edge_id, edge) in self.edges.iter().enumerate() {
            let edge_id = edge_id as LEdgeId;
            if edge.active
                && (edge.source == node || edge.target == node)
                && !seen.contains(&edge_id)
            {
                connected.push((edge_id, edge));
            }
        }
        connected
    }

    /// Returns the opposite endpoint of an incident edge.
    #[must_use]
    pub fn other_node(&self, edge: &LEdge, node: LNodeId) -> Option<LNodeId> {
        if edge.source == node {
            Some(edge.target)
        } else if edge.target == node {
            Some(edge.source)
        } else {
            None
        }
    }
}

/// Materializes the default east/west ports that `elk-go` creates for an edge
/// whose JSON endpoint names a node directly. The wire importer keeps node
/// endpoints unresolved so transfer-boundary tests can inspect the original
/// IDs; the solver calls this preprocessor before layering and routing.
pub fn materialize_implicit_ports(
    arena: &mut LayeredArena,
    graph_id: LGraphId,
) -> Result<(), LayeringError> {
    if graph_id as usize >= arena.graphs.len() {
        return Err(LayeringError::UnknownGraph(graph_id));
    }
    // Source importEdges creates implicit ports and endpoint adjacency in
    // breadth-first containing-graph order, independently of wire edge IDs.
    let mut imported_edges = (0..arena.edges.len() as LEdgeId).collect::<Vec<_>>();
    imported_edges.sort_by_key(|id| {
        arena
            .edge_model_order
            .get(id)
            .copied()
            .unwrap_or(*id as usize)
    });
    for edge_id in imported_edges {
        let (source, target, source_port, target_port) = {
            let edge = &arena.edges[edge_id as usize];
            (edge.source, edge.target, edge.source_port, edge.target_port)
        };
        if source_port.is_none() {
            let port = arena.add_implicit_port(source, true);
            arena.edges[edge_id as usize].source_port = Some(port);
            arena.ports[port as usize].outgoing_edges.push(edge_id);
        }
        if target_port.is_none() {
            let port = arena.add_implicit_port(target, false);
            arena.edges[edge_id as usize].target_port = Some(port);
            arena.ports[port as usize].incoming_edges.push(edge_id);
        }
    }
    assign_implicit_self_loop_sides(arena);
    position_constrained_ports(arena);
    distribute_implicit_port_positions(arena);
    restore_pure_loop_port_areas(arena);
    Ok(())
}

/// Refreshes implicit-port coordinates after a compound node changes size.
///
/// Unlike `materialize_implicit_ports`, this does not create ports, choose
/// endpoint sides, or reorder them. It is safe after P3 has established the
/// source port order, and keeps endpoint anchors centered on the final vessel
/// dimensions produced while finishing nested graphs.
pub fn refresh_implicit_port_positions(arena: &mut LayeredArena, node_ids: &[LNodeId]) {
    for &node_id in node_ids {
        distribute_implicit_port_positions_for_node(arena, node_id);
    }
}

// PortRestorer's start/middle/end areas for nodes containing only implicit
// self-loop ports. Merging restored ports with ordinary edges remains a
// separate processor boundary.
pub(crate) fn restore_pure_loop_port_areas(arena: &mut LayeredArena) {
    restore_pure_loop_port_areas_impl(arena, None);
}

pub(crate) fn restore_pure_loop_port_areas_for_graph(arena: &mut LayeredArena, graph: LGraphId) {
    restore_self_loop_ordering_state(arena, graph);
    restore_pure_loop_port_areas_impl(arena, Some(graph));
}

/// SelfLoopPreprocessor detaches loop edges and freely orderable pure loop
/// ports before P1-P3. Preserve their arena identity for node-local routing.
pub(crate) fn hide_self_loops_for_ordering(arena: &mut LayeredArena) {
    for node_id in 0..arena.nodes.len() as LNodeId {
        let loops = arena
            .edges
            .iter()
            .enumerate()
            .filter_map(|(id, edge)| {
                (edge.active && edge.source == node_id && edge.target == node_id)
                    .then_some(id as LEdgeId)
            })
            .collect::<BTreeSet<_>>();
        if loops.is_empty() {
            continue;
        }
        let order_fixed = matches!(
            arena.nodes[node_id as usize].port_constraints,
            PortConstraints::FixedOrder
                | PortConstraints::FixedRatio
                | PortConstraints::FixedPosition
        );
        let hierarchy = arena.nodes[node_id as usize]
            .nested_graph
            .is_some_and(|graph| {
                arena
                    .external_port_origins
                    .keys()
                    .any(|dummy| arena.nodes[*dummy as usize].graph == graph)
            });
        if !order_fixed && !hierarchy {
            let hidden = arena.nodes[node_id as usize]
                .ports
                .iter()
                .copied()
                .filter(|id| {
                    let port = &arena.ports[*id as usize];
                    let edges = port
                        .incoming_edges
                        .iter()
                        .chain(&port.outgoing_edges)
                        .filter(|id| arena.edges[**id as usize].active)
                        .copied()
                        .collect::<Vec<_>>();
                    !edges.is_empty() && edges.iter().all(|id| loops.contains(id))
                })
                .collect::<Vec<_>>();
            arena.nodes[node_id as usize]
                .ports
                .retain(|id| !hidden.contains(id));
            arena.hidden_loop_ports.insert(node_id, hidden);
        }
        for port in &mut arena.ports {
            if port.node != node_id {
                continue;
            }
            port.incoming_edges.retain(|id| !loops.contains(id));
            port.outgoing_edges.retain(|id| !loops.contains(id));
        }
        arena.hidden_loop_edges.extend(loops);
    }
}

fn restore_self_loop_ordering_state(arena: &mut LayeredArena, graph: LGraphId) {
    let nodes = arena
        .hidden_loop_ports
        .keys()
        .copied()
        .filter(|node| arena.nodes[*node as usize].graph == graph)
        .collect::<Vec<_>>();
    for node in nodes {
        let hidden = arena.hidden_loop_ports.remove(&node).unwrap();
        let mut areas: [[Vec<LPortId>; 3]; 5] =
            std::array::from_fn(|_| std::array::from_fn(|_| Vec::new()));
        for edge in arena
            .edges
            .iter()
            .filter(|edge| edge.active && edge.source == node && edge.target == node)
        {
            let (Some(source), Some(target)) = (edge.source_port, edge.target_port) else {
                continue;
            };
            let a = arena.ports[source as usize].side;
            let b = arena.ports[target as usize].side;
            if a == b {
                areas[a as usize][1].insert(0, source);
                areas[a as usize][1].push(target);
            } else {
                let (first, second) = if (a == PortSide::West && b == PortSide::North)
                    || ((a as usize) < b as usize && !(a == PortSide::North && b == PortSide::West))
                {
                    (source, target)
                } else {
                    (target, source)
                };
                areas[arena.ports[first as usize].side as usize][2].insert(0, first);
                areas[arena.ports[second as usize].side as usize][0].push(second);
            }
        }
        let old = arena.nodes[node as usize].ports.clone();
        let mut restored = Vec::new();
        let mut seen = BTreeSet::new();
        for side in [
            PortSide::North,
            PortSide::East,
            PortSide::South,
            PortSide::West,
        ] {
            let visible = old
                .iter()
                .copied()
                .filter(|id| arena.ports[*id as usize].side == side)
                .collect::<Vec<_>>();
            let before_middle = |port: LPortId| {
                arena.north_south_port_dummies.iter().any(|dummy| {
                    dummy.origin_port == port
                        && if side == PortSide::North {
                            dummy.input_port.is_some()
                        } else if side == PortSide::South {
                            dummy.output_port.is_some()
                        } else {
                            false
                        }
                })
            };
            let mut values = areas[side as usize][0]
                .iter()
                .copied()
                .filter(|id| hidden.contains(id))
                .collect::<Vec<_>>();
            if side == PortSide::West {
                values.extend(visible.iter().copied());
            } else if matches!(side, PortSide::North | PortSide::South) {
                values.extend(visible.iter().copied().take_while(|id| before_middle(*id)));
            }
            values.extend(
                areas[side as usize][1]
                    .iter()
                    .copied()
                    .filter(|id| hidden.contains(id)),
            );
            if side != PortSide::West {
                values.extend(visible.iter().copied());
            }
            values.extend(
                areas[side as usize][2]
                    .iter()
                    .copied()
                    .filter(|id| hidden.contains(id)),
            );
            for id in values {
                if seen.insert(id) {
                    restored.push(id);
                }
            }
        }
        arena.nodes[node as usize].ports = restored;
    }
}

/// SelfLoopPostprocessor restores edge adjacency only after P5. PortRestorer
/// restores port membership before P4 while the loop edges stay detached.
pub(crate) fn restore_self_loop_edges_after_routing(arena: &mut LayeredArena, graph: LGraphId) {
    let edges = arena
        .hidden_loop_edges
        .iter()
        .copied()
        .filter(|id| arena.nodes[arena.edges[*id as usize].source as usize].graph == graph)
        .collect::<Vec<_>>();
    for id in edges {
        let edge = &arena.edges[id as usize];
        if let Some(port) = edge.source_port {
            arena.ports[port as usize].outgoing_edges.push(id);
        }
        if let Some(port) = edge.target_port {
            arena.ports[port as usize].incoming_edges.push(id);
        }
        arena.hidden_loop_edges.remove(&id);
    }
}

fn restore_pure_loop_port_areas_impl(arena: &mut LayeredArena, graph: Option<LGraphId>) {
    for node_id in 0..arena.nodes.len() as LNodeId {
        if graph.is_some_and(|graph| arena.nodes[node_id as usize].graph != graph) {
            continue;
        }
        let ports = &arena.nodes[node_id as usize].ports;
        if ports.is_empty()
            || ports.iter().any(|id| {
                let port = &arena.ports[*id as usize];
                !port.implicit
                    || port
                        .incoming_edges
                        .iter()
                        .chain(&port.outgoing_edges)
                        .any(|id| {
                            arena.edges[*id as usize].source != arena.edges[*id as usize].target
                        })
            })
        {
            continue;
        }
        let mut areas: [[Vec<LPortId>; 3]; 5] =
            std::array::from_fn(|_| std::array::from_fn(|_| Vec::new()));
        for edge in arena
            .edges
            .iter()
            .filter(|edge| edge.active && edge.source == node_id && edge.target == node_id)
        {
            let (Some(source), Some(target)) = (edge.source_port, edge.target_port) else {
                continue;
            };
            let a = arena.ports[source as usize].side;
            let b = arena.ports[target as usize].side;
            if a == b {
                areas[a as usize][1].insert(0, source);
                areas[a as usize][1].push(target);
            } else {
                let (first, second) = if (a == PortSide::West && b == PortSide::North)
                    || ((a as usize) < b as usize && !(a == PortSide::North && b == PortSide::West))
                {
                    (source, target)
                } else {
                    (target, source)
                };
                areas[arena.ports[first as usize].side as usize][2].insert(0, first);
                areas[arena.ports[second as usize].side as usize][0].push(second);
            }
        }
        let mut perimeter_order = Vec::new();
        for side in [
            PortSide::North,
            PortSide::East,
            PortSide::South,
            PortSide::West,
        ] {
            let mut ordered = areas[side as usize]
                .iter()
                .flatten()
                .copied()
                .collect::<Vec<_>>();
            perimeter_order.extend(ordered.iter().copied());
            if matches!(side, PortSide::South | PortSide::West) {
                ordered.reverse();
            }
            let node = &arena.nodes[node_id as usize];
            let horizontal = matches!(side, PortSide::North | PortSide::South);
            let extent = if horizontal { node.width } else { node.height };
            let count = ordered.len() as f64 + 1.0;
            let gap = 10.0 + (extent - 10.0 * count) / count;
            let mut position = gap;
            for id in ordered {
                let port = &mut arena.ports[id as usize];
                if horizontal {
                    port.x = position;
                } else {
                    port.y = position;
                }
                position += gap;
            }
        }
        arena.nodes[node_id as usize].ports = perimeter_order;
    }
}

/// Per-side compacted slots for the currently resolved loop model.
/// None denotes an unoccupied side or an unresolved processor input.
pub fn self_loop_compacted_slots(
    arena: &LayeredArena,
    node_id: LNodeId,
) -> Option<BTreeMap<LEdgeId, [Option<usize>; 5]>> {
    let raw = self_loop_raw_slots(arena, node_id)?;
    let node = arena.nodes.get(node_id as usize)?;
    let mut result = raw
        .keys()
        .map(|id| (*id, [None; 5]))
        .collect::<BTreeMap<_, _>>();
    let activities = raw
        .keys()
        .map(|id| Some((*id, self_loop_port_activity(arena, *id)?)))
        .collect::<Option<BTreeMap<_, _>>>()?;
    let mut next_free = vec![0usize; node.ports.len()];
    for side in [
        PortSide::North,
        PortSide::East,
        PortSide::South,
        PortSide::West,
    ] {
        let indexes = node
            .ports
            .iter()
            .enumerate()
            .filter_map(|(i, port)| (arena.ports[*port as usize].side == side).then_some(i))
            .collect::<Vec<_>>();
        if indexes.is_empty() {
            // slot_assigner.go shiftSide: a side without ports numbers the
            // loops passing it in raw-slot order.
            let mut passing = raw
                .keys()
                .copied()
                .filter(|id| {
                    self_loop_occupied_sides(arena, *id).is_some_and(|sides| sides[side as usize])
                })
                .collect::<Vec<_>>();
            passing.sort_by_key(|id| raw[id]);
            for (slot, id) in passing.into_iter().enumerate() {
                result.get_mut(&id)?[side as usize] = Some(slot);
            }
            continue;
        }
        let mut loops = raw
            .keys()
            .copied()
            .filter(|id| indexes.iter().any(|i| activities[id][*i]))
            .collect::<Vec<_>>();
        loops.sort_by_key(|id| raw[id]);
        let mut label_slots = BTreeMap::new();
        for id in loops {
            let activity = &activities[&id];
            let mut lowest = indexes
                .iter()
                .filter(|i| activity[**i])
                .map(|i| next_free[*i])
                .max()
                .unwrap_or(0);
            while label_slots
                .iter()
                .any(|(other, slot)| *slot == lowest && self_loop_labels_overlap(arena, id, *other))
            {
                lowest += 1;
            }
            result.get_mut(&id)?[side as usize] = Some(lowest);
            for i in &indexes {
                if activity[*i] {
                    next_free[*i] = lowest + 1;
                }
            }
            if arena.loop_label_sizes.contains_key(&id) {
                label_slots.insert(id, lowest);
            }
        }
    }
    Some(result)
}

/// Raw loop slots before per-side compaction. The source slot assigner uses
/// the owning graph's Java random stream to break positive-weight feedback
/// cycles before topological numbering.
pub fn self_loop_raw_slots(
    arena: &LayeredArena,
    node_id: LNodeId,
) -> Option<BTreeMap<LEdgeId, usize>> {
    let dependencies = self_loop_dependencies(arena, node_id)?;
    let mut slots = arena
        .edges
        .iter()
        .enumerate()
        .filter(|(_, edge)| edge.active && edge.source == node_id && edge.target == node_id)
        .map(|(id, _)| (id as LEdgeId, 0usize))
        .collect::<BTreeMap<_, _>>();
    let ids = slots.keys().copied().collect::<Vec<_>>();
    let indexed = dependencies
        .iter()
        .map(|(source, target, weight)| {
            Some((
                ids.iter().position(|id| id == source)?,
                ids.iter().position(|id| id == target)?,
                *weight,
            ))
        })
        .collect::<Option<Vec<_>>>()?;
    let graph_id = arena.nodes.get(node_id as usize)?.graph;
    let random_seed = arena.graphs.get(graph_id as usize)?.random_seed;
    let mut random = JavaRandom::with_stream(random_seed, "self-loop-slots");
    let dependencies = crate::loop_cycles::break_cycles(ids.len(), &indexed, |bound| {
        Some(random.next_int_n(bound))
    })?
    .into_iter()
    .map(|(source, target, weight)| (ids[source], ids[target], weight))
    .collect::<Vec<_>>();
    let mut outgoing = slots
        .keys()
        .map(|id| (*id, 0usize))
        .collect::<BTreeMap<_, _>>();
    for (source, _, _) in &dependencies {
        *outgoing.get_mut(source)? += 1;
    }
    let mut queue = outgoing
        .iter()
        .filter_map(|(id, count)| (*count == 0).then_some(*id))
        .collect::<Vec<_>>();
    let mut next = 0;
    while next < queue.len() {
        let target = queue[next];
        next += 1;
        let next_slot = slots[&target] + 1;
        for (source, destination, _) in &dependencies {
            if *destination != target {
                continue;
            }
            let slot = slots.get_mut(source)?;
            *slot = (*slot).max(next_slot);
            let count = outgoing.get_mut(source)?;
            *count -= 1;
            if *count == 0 {
                queue.push(*source);
            }
        }
    }
    (next == slots.len()).then_some(slots)
}

/// Horizontal label interval before routing, following LabelPlacer's
/// centered one-side and port-aligned adjacent-corner rules.
pub(crate) fn self_loop_label_interval(
    arena: &LayeredArena,
    id: LEdgeId,
) -> Option<(PortSide, f64, f64)> {
    let edge = arena.edges.get(id as usize)?;
    let (width, height) = *arena.loop_label_sizes.get(&id)?;
    let width = if matches!(
        arena.graphs[0].direction,
        LayerDirection::Down | LayerDirection::Up
    ) {
        height
    } else {
        width
    };
    let source = &arena.ports[edge.source_port? as usize];
    let target = &arena.ports[edge.target_port? as usize];
    if source.side == target.side {
        if !matches!(source.side, PortSide::North | PortSide::South) {
            return None;
        }
        let start = (arena.nodes[edge.source as usize].width - width) / 2.0;
        return Some((source.side, start, start + width));
    }
    let (horizontal, other) = if matches!(source.side, PortSide::North | PortSide::South) {
        (source, target)
    } else {
        (target, source)
    };
    if !matches!(horizontal.side, PortSide::North | PortSide::South)
        || !matches!(other.side, PortSide::East | PortSide::West)
    {
        return None;
    }
    let start = horizontal.x + horizontal.width / 2.0
        - if other.side == PortSide::West {
            width
        } else {
            0.0
        };
    Some((horizontal.side, start, start + width))
}

fn self_loop_labels_overlap(arena: &LayeredArena, first: LEdgeId, second: LEdgeId) -> bool {
    match (
        self_loop_label_interval(arena, first),
        self_loop_label_interval(arena, second),
    ) {
        (Some((a, start, end)), Some((b, other_start, other_end))) => {
            a == b && start <= other_end && end >= other_start
        }
        _ => false,
    }
}

/// Weighted outer-to-inner dependencies for resolved two-port loops.
pub fn self_loop_dependencies(
    arena: &LayeredArena,
    node_id: LNodeId,
) -> Option<Vec<(LEdgeId, LEdgeId, usize)>> {
    let node = arena.nodes.get(node_id as usize)?;
    let loops = arena
        .edges
        .iter()
        .enumerate()
        .filter(|(_, edge)| edge.active && edge.source == node_id && edge.target == node_id)
        .map(|(id, _)| id as LEdgeId)
        .collect::<Vec<_>>();
    let mut activities = Vec::new();
    for id in &loops {
        activities.push(self_loop_port_activity(arena, *id)?);
    }
    let crossings = |upper: LEdgeId, lower: &[bool]| -> Option<usize> {
        let edge = &arena.edges[upper as usize];
        let mut count = 0;
        let source = edge.source_port?;
        let target = edge.target_port?;
        for port in [source, target].into_iter().collect::<BTreeSet<_>>() {
            let index = node.ports.iter().position(|id| *id == port)?;
            count += usize::from(lower[index]);
        }
        Some(count)
    };
    let mut dependencies = Vec::new();
    for first in 0..loops.len() {
        for second in first + 1..loops.len() {
            let a = crossings(loops[first], &activities[second])?;
            let b = crossings(loops[second], &activities[first])?;
            if a < b {
                dependencies.push((loops[first], loops[second], b - a));
            } else if b < a {
                dependencies.push((loops[second], loops[first], a - b));
            } else if a != 0 || self_loop_labels_overlap(arena, loops[first], loops[second]) {
                dependencies.push((loops[first], loops[second], 0));
                dependencies.push((loops[second], loops[first], 0));
            }
        }
    }
    Some(dependencies)
}

/// Inclusive perimeter activity for one-side and adjacent-side self-loops.
/// Opposing-side loops require RoutingDirector's penalty-based choice and
/// are deliberately left unresolved here.
pub fn self_loop_port_activity(arena: &LayeredArena, edge_id: LEdgeId) -> Option<Vec<bool>> {
    let edge = arena.edges.get(edge_id as usize)?;
    if !edge.active || edge.source != edge.target {
        return None;
    }
    let source = edge.source_port?;
    let target = edge.target_port?;
    let ports = &arena.nodes.get(edge.source as usize)?.ports;
    let a = ports.iter().position(|port| *port == source)?;
    let b = ports.iter().position(|port| *port == target)?;
    let sa = arena.ports[source as usize].side;
    let sb = arena.ports[target as usize].side;
    if sa == PortSide::Undefined || sb == PortSide::Undefined {
        return None;
    }
    let (left, right) = if sa == sb {
        (a.min(b), a.max(b))
    } else if (sa as i32 - sb as i32).abs() == 2 {
        // RoutingDirector picks the cheaper arc; Leftmost..Rightmost follow it.
        if self_loop_routing_clockwise(arena, edge_id, sa, sb) {
            (a, b)
        } else {
            (b, a)
        }
    } else if (sa == PortSide::West && sb == PortSide::North)
        || ((sa as usize) < sb as usize && !(sa == PortSide::North && sb == PortSide::West))
    {
        (a, b)
    } else {
        (b, a)
    };
    let mut activity = vec![false; ports.len()];
    let mut index = left;
    loop {
        activity[index] = true;
        if index == right {
            break;
        }
        index = (index + 1) % ports.len();
    }
    Some(activity)
}

/// Applies ELK's `PortSideAssigner` target order to freely orderable direct
/// self-loops. Explicit ports keep their imported side constraints.
fn assign_implicit_self_loop_sides(arena: &mut LayeredArena) {
    const EQUALLY_TARGETS: [(PortSide, PortSide); 8] = [
        (PortSide::North, PortSide::North),
        (PortSide::South, PortSide::South),
        (PortSide::East, PortSide::East),
        (PortSide::West, PortSide::West),
        (PortSide::West, PortSide::North),
        (PortSide::North, PortSide::East),
        (PortSide::South, PortSide::West),
        (PortSide::East, PortSide::South),
    ];
    for node_id in 0..arena.nodes.len() as LNodeId {
        let distribution = arena.nodes[node_id as usize].self_loop_distribution;
        let loops = arena
            .edges
            .iter()
            .filter_map(|edge| {
                if !edge.active || edge.source != node_id || edge.target != node_id {
                    return None;
                }
                let source = edge.source_port?;
                let target = edge.target_port?;
                (arena.ports[source as usize].implicit && arena.ports[target as usize].implicit)
                    .then_some((source, target))
            })
            .collect::<Vec<_>>();
        let mut north = 0usize;
        let mut south = 0usize;
        for (index, (source, target)) in loops.into_iter().enumerate() {
            let (source_side, target_side) = match distribution {
                SelfLoopDistribution::North => (PortSide::North, PortSide::North),
                SelfLoopDistribution::NorthSouth => {
                    if north <= south {
                        north += 2;
                        (PortSide::North, PortSide::North)
                    } else {
                        south += 2;
                        (PortSide::South, PortSide::South)
                    }
                }
                SelfLoopDistribution::Equally => EQUALLY_TARGETS[index % EQUALLY_TARGETS.len()],
            };
            arena.ports[source as usize].side = source_side;
            arena.ports[target as usize].side = target_side;
        }
    }
}

fn direction_boundary_side(direction: LayerDirection, output: bool) -> PortSide {
    match (direction, output) {
        (LayerDirection::Left, true) => PortSide::West,
        (LayerDirection::Left, false) => PortSide::East,
        (_, true) => PortSide::East,
        (_, false) => PortSide::West,
    }
}

fn opposite_port_side(side: PortSide) -> PortSide {
    match side {
        PortSide::North => PortSide::South,
        PortSide::East => PortSide::West,
        PortSide::South => PortSide::North,
        PortSide::West => PortSide::East,
        PortSide::Undefined => PortSide::Undefined,
    }
}

fn port_constraints_fix_side(constraints: PortConstraints) -> bool {
    matches!(
        constraints,
        PortConstraints::FixedSide
            | PortConstraints::FixedOrder
            | PortConstraints::FixedRatio
            | PortConstraints::FixedPosition
    )
}

fn add_boundary_port(
    arena: &mut LayeredArena,
    node: LNodeId,
    side: PortSide,
    border_offset: f64,
) -> LPortId {
    let port_id = arena.ports.len() as LPortId;
    arena.ports.push(LPort {
        node,
        source_id: None,
        model_order: None,
        side,
        index: None,
        x: 0.0,
        y: 0.0,
        width: 0.0,
        height: 0.0,
        implicit: false,
        incoming_edges: Vec::new(),
        outgoing_edges: Vec::new(),
    });
    arena.nodes[node as usize].ports.push(port_id);
    arena.port_border_offsets.insert(port_id, border_offset);
    port_id
}

fn boundary_port_for_hierarchy(
    arena: &mut LayeredArena,
    node: LNodeId,
    side: PortSide,
    border_offset: f64,
    opposite_port: LPortId,
) -> (LPortId, bool) {
    if arena.ports[opposite_port as usize].node == node {
        // compound.go reuses the original endpoint when a hierarchy edge
        // terminates on the compound node itself. In particular, this keeps
        // ancestor-to-descendant edges connected to the imported implicit
        // port instead of manufacturing a second parent endpoint.
        (opposite_port, true)
    } else {
        (add_boundary_port(arena, node, side, border_offset), false)
    }
}

/// Mirrors compound.go `compoundPortNetFlow` (ELK's
/// `CompoundGraphPreprocessor.calculateNetFlow`). Positive values make a port
/// an output of its compound node; negative values make it an input. An edge
/// into the compound's own child graph votes for input, and an edge out of it
/// votes for output, so an ancestor-to-child edge leaves through the input
/// side while an ancestor-to-grandchild edge still leaves as an output. D2
/// never activates inside self loops, so every self loop counts as outside.
fn compound_port_net_flow(arena: &LayeredArena, port_id: LPortId) -> i32 {
    let port = &arena.ports[port_id as usize];
    let node_id = port.node;
    let mut output_votes = 0;
    let mut input_votes = 0;
    for edge_id in &port.outgoing_edges {
        let edge = &arena.edges[*edge_id as usize];
        if edge.source == edge.target {
            output_votes += 1;
        } else if arena.graphs[arena.nodes[edge.target as usize].graph as usize].parent_node
            == Some(node_id)
        {
            input_votes += 1;
        } else {
            output_votes += 1;
        }
    }
    for edge_id in &port.incoming_edges {
        let edge = &arena.edges[*edge_id as usize];
        if edge.source == edge.target {
            input_votes += 1;
        } else if arena.graphs[arena.nodes[edge.source as usize].graph as usize].parent_node
            == Some(node_id)
        {
            output_votes += 1;
        } else {
            input_votes += 1;
        }
    }
    output_votes - input_votes
}

fn add_external_port_dummy(
    arena: &mut LayeredArena,
    graph: LGraphId,
    side: PortSide,
    origin_port: LPortId,
    net_flow: i32,
) -> (LNodeId, LPortId) {
    let owner = arena.ports[origin_port as usize].node;
    let constraints = arena.nodes[owner as usize].port_constraints;
    let side = if port_constraints_fix_side(constraints) && side != PortSide::Undefined {
        side
    } else if port_constraints_fix_side(constraints) {
        // LGraphUtil.createExternalPortDummy falls through to East for an
        // undefined side (ELK's effective RIGHT-direction default).
        PortSide::East
    } else {
        direction_boundary_side(arena.graphs[graph as usize].direction, net_flow >= 0)
    };
    // elk-go updates the owning compound port when side constraints are free.
    // A reused ancestor endpoint follows its own net flow, even when the
    // current hierarchy segment travels in the opposite direction.
    arena.ports[origin_port as usize].side = side;
    // CompoundGraphPreprocessor.externalDummyForBoundary fixes the owning
    // compound node's side constraints as soon as a boundary dummy is made.
    arena.nodes[owner as usize].port_constraints = PortConstraints::FixedSide;
    let node_id = arena.nodes.len() as LNodeId;
    let traversal_index = arena.graphs[graph as usize].layerless_nodes.len();
    arena.nodes.push(LNode {
        layer_constraint: NodeLayerConstraint::None,
        alignment: NodeAlignment::Automatic,
        graph,
        traversal_index,
        layer: None,
        source_id: format!("__external_port_dummy_{node_id}"),
        node_type: LayeredNodeType::ExternalPort,
        nested_graph: None,
        ports: Vec::new(),
        port_constraints: PortConstraints::FixedPosition,
        self_loop_distribution: SelfLoopDistribution::North,
        width: 0.0,
        height: 0.0,
        fixed_width: false,
        fixed_height: false,
        margin_top: 0.0,
        margin_right: 0.0,
        margin_bottom: 0.0,
        margin_left: 0.0,
        margins_in_wire_dimensions: false,
        computed_margins: [0.0; 4],
        label_margin_top: 0.0,
        label_margin_right: 0.0,
        label_margin_bottom: 0.0,
        label_margin_left: 0.0,
        position_x: 0.0,
        position_y: 0.0,
    });
    arena.graphs[graph as usize].layerless_nodes.push(node_id);
    arena.external_port_origins.insert(node_id, origin_port);
    let first_separate = match side {
        PortSide::West => Some(true),
        PortSide::East => Some(false),
        PortSide::North | PortSide::South | PortSide::Undefined => None,
    };
    arena
        .external_port_first_separate
        .insert(node_id, first_separate);
    let port_id = arena.ports.len() as LPortId;
    arena.ports.push(LPort {
        node: node_id,
        source_id: None,
        model_order: None,
        side: opposite_port_side(side),
        index: None,
        x: 0.0,
        y: 0.0,
        width: 0.0,
        height: 0.0,
        implicit: false,
        incoming_edges: Vec::new(),
        outgoing_edges: Vec::new(),
    });
    arena.nodes[node_id as usize].ports.push(port_id);
    (node_id, port_id)
}

fn add_hierarchy_segment(
    arena: &mut LayeredArena,
    source: LNodeId,
    source_port: LPortId,
    target: LNodeId,
    target_port: LPortId,
    priority_shortness: f64,
) -> LEdgeId {
    let edge_id = arena.edges.len() as LEdgeId;
    arena.edges.push(LEdge {
        source,
        target,
        source_port: Some(source_port),
        target_port: Some(target_port),
        reversed: false,
        active: true,
        bend_points: Vec::new(),
        junction_points: Vec::new(),
        hierarchy_segments: Vec::new(),
    });
    arena.ports[source_port as usize]
        .outgoing_edges
        .push(edge_id);
    arena.ports[target_port as usize]
        .incoming_edges
        .push(edge_id);
    arena
        .edge_priority_shortness
        .insert(edge_id, priority_shortness.max(1.0));
    edge_id
}

fn graph_ancestors(arena: &LayeredArena, mut graph: LGraphId, root: LGraphId) -> Vec<LGraphId> {
    let mut result = vec![graph];
    while graph != root {
        let Some(parent_node) = arena.graphs[graph as usize].parent_node else {
            break;
        };
        graph = arena.nodes[parent_node as usize].graph;
        result.push(graph);
    }
    result
}

fn lowest_common_graph(
    arena: &LayeredArena,
    first: LGraphId,
    second: LGraphId,
    root: LGraphId,
) -> LGraphId {
    let first_ancestors = graph_ancestors(arena, first, root);
    let second_ancestors = graph_ancestors(arena, second, root);
    second_ancestors
        .into_iter()
        .find(|graph| first_ancestors.contains(graph))
        .unwrap_or(root)
}

/// Splits cross-hierarchy edges into the external-port dummy chain used by
/// `internal/layered/compound.go`, including edges that cross only one
/// container boundary. That shallow case still needs the same child dummy and
/// parent boundary port as a deeper edge so placement and routing see the
/// hierarchy boundary before node ordering.
pub fn split_deep_cross_hierarchy_edges(
    arena: &mut LayeredArena,
    root: LGraphId,
) -> Result<(), LayeringError> {
    let original_edge_count = arena.edges.len();
    // CompoundGraphPreprocessor walks the LGraph arena in edge allocation
    // order, which ElkGraphImporter assigns by visiting containers breadth
    // first (the order recorded as edge model order), not by wire position.
    // The order is observable: the first boundary dummy fixes the compound's
    // port sides, so later reused endpoints keep their imported sides.
    let mut import_order = (0..original_edge_count as LEdgeId).collect::<Vec<_>>();
    import_order.sort_by_key(|edge_id| {
        arena
            .edge_model_order
            .get(edge_id)
            .copied()
            .unwrap_or(usize::MAX)
    });
    for edge_id in import_order {
        let edge = arena.edges[edge_id as usize].clone();
        if !edge.active {
            continue;
        }
        let source_graph = arena.nodes[edge.source as usize].graph;
        let target_graph = arena.nodes[edge.target as usize].graph;
        if source_graph == target_graph {
            continue;
        }
        let lca = lowest_common_graph(arena, source_graph, target_graph, root);

        let mut segments = Vec::new();
        let priority_shortness = arena
            .edge_priority_shortness
            .get(&edge_id)
            .copied()
            .unwrap_or(1.0);
        let mut source_node = edge.source;
        let mut source_port = edge.source_port.ok_or_else(|| {
            LayeringError::Import(format!("deep edge {edge_id} has no source port"))
        })?;
        let mut source_scope = source_graph;
        while source_scope != lca {
            let parent_node = arena.graphs[source_scope as usize]
                .parent_node
                .ok_or_else(|| LayeringError::Import("deep edge source has no parent".into()))?;
            let parent_scope = arena.nodes[parent_node as usize].graph;
            let side = direction_boundary_side(arena.graphs[parent_scope as usize].direction, true);
            let border_offset = arena.graphs[source_scope as usize].edge_edge_spacing / 2.0;
            let (boundary, reused) = boundary_port_for_hierarchy(
                arena,
                parent_node,
                side,
                border_offset,
                edge.target_port.expect("materialized target port"),
            );
            if reused {
                let endpoint_side = arena.ports[boundary as usize].side;
                arena.reused_parent_ports.insert(boundary);
                let sides = arena
                    .reused_hierarchy_sides
                    .entry(source_scope)
                    .or_default();
                *sides |= 1 << endpoint_side as u8;
            }
            // compound.go externalDummyForBoundary: a segment ending at the
            // compound parent reuses the original endpoint, whose side follows
            // compoundPortNetFlow over all of that port's connections.
            let net_flow = if reused && edge.target_port == Some(boundary) {
                compound_port_net_flow(arena, boundary)
            } else {
                1
            };
            // externalDummyForBoundary passes the boundary port's own side:
            // a reused endpoint keeps the side it was imported with, which
            // matters once the compound's port sides are already fixed.
            let side = arena.ports[boundary as usize].side;
            let (dummy, dummy_port) =
                add_external_port_dummy(arena, source_scope, side, boundary, net_flow);
            segments.push(add_hierarchy_segment(
                arena,
                source_node,
                source_port,
                dummy,
                dummy_port,
                priority_shortness,
            ));
            source_node = parent_node;
            source_port = boundary;
            source_scope = parent_scope;
        }

        let mut target_segments = Vec::new();
        let mut target_node = edge.target;
        let mut target_port = edge.target_port.ok_or_else(|| {
            LayeringError::Import(format!("deep edge {edge_id} has no target port"))
        })?;
        let mut target_scope = target_graph;
        while target_scope != lca {
            let parent_node = arena.graphs[target_scope as usize]
                .parent_node
                .ok_or_else(|| LayeringError::Import("deep edge target has no parent".into()))?;
            let parent_scope = arena.nodes[parent_node as usize].graph;
            let side =
                direction_boundary_side(arena.graphs[parent_scope as usize].direction, false);
            let border_offset = arena.graphs[target_scope as usize].edge_edge_spacing / 2.0;
            let (boundary, reused) = boundary_port_for_hierarchy(
                arena,
                parent_node,
                side,
                border_offset,
                edge.source_port.expect("materialized source port"),
            );
            if reused {
                let endpoint_side = arena.ports[boundary as usize].side;
                arena.reused_parent_ports.insert(boundary);
                let sides = arena
                    .reused_hierarchy_sides
                    .entry(target_scope)
                    .or_default();
                *sides |= 1 << endpoint_side as u8;
            }
            let net_flow = if reused && edge.source_port == Some(boundary) {
                compound_port_net_flow(arena, boundary)
            } else {
                -1
            };
            // externalDummyForBoundary passes the boundary port's own side:
            // a reused endpoint keeps the side it was imported with, which
            // matters once the compound's port sides are already fixed.
            let side = arena.ports[boundary as usize].side;
            let (dummy, dummy_port) =
                add_external_port_dummy(arena, target_scope, side, boundary, net_flow);
            target_segments.push(add_hierarchy_segment(
                arena,
                dummy,
                dummy_port,
                target_node,
                target_port,
                priority_shortness,
            ));
            target_node = parent_node;
            target_port = boundary;
            target_scope = parent_scope;
        }
        if source_node != target_node {
            segments.push(add_hierarchy_segment(
                arena,
                source_node,
                source_port,
                target_node,
                target_port,
                priority_shortness,
            ));
        }
        segments.extend(target_segments.into_iter().rev());
        if let Some(order) = arena.edge_model_order.get(&edge_id).copied() {
            for segment in &segments {
                arena.edge_model_order.insert(*segment, order);
            }
        }
        if let Some(weight) = arena.edge_direction_weight.get(&edge_id).copied() {
            for segment_id in &segments {
                arena.edge_direction_weight.insert(*segment_id, weight);
            }
        }
        if let Some(thickness) = arena.edge_thickness.get(&edge_id).copied() {
            for segment_id in &segments {
                arena.edge_thickness.insert(*segment_id, thickness);
            }
        }
        arena.edges[edge_id as usize].active = false;
        arena.edges[edge_id as usize].hierarchy_segments = segments;
        detach_hierarchy_edge_from_port_adjacency(arena, edge_id);
    }
    ensure_compound_port_dummies(arena, root);
    reorder_compound_port_dummies(arena, root);
    Ok(())
}

/// Removes the superseded cross-hierarchy edge from its original endpoint
/// ports while retaining its endpoint metadata for route reconstruction.
///
/// elk-go calls `SetEdgeSource(edge, nil)` and `SetEdgeTarget(edge, nil)` after
/// replacing a cross-hierarchy edge with per-scope segments. Keeping the
/// inactive edge in these adjacency lists makes P5's connected-port traversal
/// walk both the replacement segments and the stale original connection,
/// inventing routing slots across graph scopes.
fn detach_hierarchy_edge_from_port_adjacency(arena: &mut LayeredArena, edge_id: LEdgeId) {
    let source_port = arena.edges[edge_id as usize].source_port;
    let target_port = arena.edges[edge_id as usize].target_port;
    if let Some(port_id) = source_port {
        arena.ports[port_id as usize]
            .outgoing_edges
            .retain(|candidate| *candidate != edge_id);
    }
    if let Some(port_id) = target_port {
        arena.ports[port_id as usize]
            .incoming_edges
            .retain(|candidate| *candidate != edge_id);
    }
}

/// Mirrors CompoundGraphPreprocessor.ensureCompoundPortDummies: when one port
/// of a compound node crosses its hierarchy boundary, every port on that
/// compound is represented by an external dummy in the child graph. Unused
/// ports still constrain layer order and the hierarchical node resizer.
fn ensure_compound_port_dummies(arena: &mut LayeredArena, graph_id: LGraphId) {
    let node_ids = arena.graphs[graph_id as usize].layerless_nodes.clone();
    for node_id in node_ids {
        let Some(child_graph) = arena.nodes[node_id as usize].nested_graph else {
            continue;
        };
        ensure_compound_port_dummies(arena, child_graph);
        let has_external_ports = arena
            .external_port_origins
            .keys()
            .any(|dummy| arena.nodes[*dummy as usize].graph == child_graph);
        if !has_external_ports {
            continue;
        }

        let ports = arena.nodes[node_id as usize].ports.clone();
        for port_id in ports {
            let already_represented = arena
                .external_port_origins
                .values()
                .any(|origin| *origin == port_id);
            if already_represented {
                continue;
            }
            let net_flow = arena.ports[port_id as usize].outgoing_edges.len() as i32
                - arena.ports[port_id as usize].incoming_edges.len() as i32;
            let side = arena.ports[port_id as usize].side;
            // `add_external_port_dummy` already registers the node in the
            // child's layerless list. Appending it again here duplicates the
            // same external port in P2 and P3 traversal order.
            add_external_port_dummy(arena, child_graph, side, port_id, net_flow);
        }
    }
}

/// Ordering metadata used to reconstruct ELK's hierarchy-port dummy order.
#[derive(Clone, Debug)]
struct CompoundPortDummyOrder {
    node: LNodeId,
    connected: bool,
    path: Vec<usize>,
    port_index: usize,
    edge_order: usize,
    stable_rank: usize,
}

/// Mirrors elk-go's `reorderCompoundPortDummies` after hierarchy-edge
/// preprocessing. The Java importer creates these dummies in a recursive,
/// post-order walk; Rust creates hierarchy segments in edge order, so restore
/// the source traversal order before P1/P2 consume the layerless nodes.
fn reorder_compound_port_dummies(arena: &mut LayeredArena, graph_id: LGraphId) {
    let nodes = arena.graphs[graph_id as usize].layerless_nodes.clone();
    for node_id in nodes {
        if let Some(child_graph) = arena.nodes[node_id as usize].nested_graph {
            reorder_compound_port_dummies(arena, child_graph);
        }
    }

    let nodes = arena.graphs[graph_id as usize].layerless_nodes.clone();
    let mut ordinary_nodes = Vec::with_capacity(nodes.len());
    let mut dummies = Vec::new();
    for (stable_rank, node_id) in nodes.iter().copied().enumerate() {
        if arena.nodes[node_id as usize].node_type != LayeredNodeType::ExternalPort {
            ordinary_nodes.push(node_id);
            continue;
        }

        let mut order = CompoundPortDummyOrder {
            node: node_id,
            connected: false,
            path: Vec::new(),
            port_index: usize::MAX,
            edge_order: usize::MAX,
            stable_rank,
        };
        if let Some((inside_port, edge_id)) = compound_dummy_inside_endpoint(arena, node_id) {
            order.connected = true;
            (order.path, order.port_index) =
                compound_inside_traversal_key(arena, graph_id, inside_port);
            order.edge_order = original_edge_model_order(arena, edge_id);
        } else if let Some(origin_port) = arena.external_port_origins.get(&node_id).copied() {
            let owner = arena.ports[origin_port as usize].node;
            order.port_index = node_port_index(arena, owner, origin_port);
        }
        dummies.push(order);
    }

    if dummies.len() < 2 {
        return;
    }
    #[cfg(feature = "diagnostic-traces")]
    let before_order = dummies
        .iter()
        .map(|dummy| {
            let source_id = &arena.nodes[dummy.node as usize].source_id;
            let origin = arena.external_port_origins.get(&dummy.node).copied();
            format!(
                "{}:origin={origin:?}:connected={}:path={:?}:port={}:edge={}:rank={}",
                source_id,
                dummy.connected,
                dummy.path,
                dummy.port_index,
                dummy.edge_order,
                dummy.stable_rank
            )
        })
        .collect::<Vec<_>>()
        .join("|");
    dummies.sort_by(|first, second| {
        second
            .connected
            .cmp(&first.connected)
            .then_with(|| compare_compound_traversal_paths(&first.path, &second.path))
            .then_with(|| first.port_index.cmp(&second.port_index))
            .then_with(|| first.edge_order.cmp(&second.edge_order))
            .then_with(|| first.stable_rank.cmp(&second.stable_rank))
    });

    #[cfg(feature = "diagnostic-traces")]
    {
        let after_order = dummies
            .iter()
            .map(|dummy| arena.nodes[dummy.node as usize].source_id.as_str())
            .collect::<Vec<_>>()
            .join(",");
        crate::trace::emit_value(
            "hierarchy",
            "compound_port_dummy_order",
            &format!("graphs[{graph_id}]"),
            &format!("before={before_order};after={after_order}"),
        );
    }

    ordinary_nodes.extend(dummies.into_iter().map(|dummy| dummy.node));
    arena.graphs[graph_id as usize].layerless_nodes = ordinary_nodes;
}

/// Finds the port and edge on the inside of an external-port dummy. elk-go
/// checks the dummy port's outgoing edges before its incoming edges.
fn compound_dummy_inside_endpoint(
    arena: &LayeredArena,
    dummy_id: LNodeId,
) -> Option<(LPortId, LEdgeId)> {
    for &port_id in &arena.nodes[dummy_id as usize].ports {
        let port = &arena.ports[port_id as usize];
        for &edge_id in &port.outgoing_edges {
            let edge = &arena.edges[edge_id as usize];
            if edge.active
                && let Some(inside_port) = edge.target_port
            {
                return Some((inside_port, edge_id));
            }
        }
        for &edge_id in &port.incoming_edges {
            let edge = &arena.edges[edge_id as usize];
            if edge.active
                && let Some(inside_port) = edge.source_port
            {
                return Some((inside_port, edge_id));
            }
        }
    }
    None
}

fn compound_inside_traversal_key(
    arena: &LayeredArena,
    graph_id: LGraphId,
    port_id: LPortId,
) -> (Vec<usize>, usize) {
    let port = &arena.ports[port_id as usize];
    let node_id = port.node;
    let mut path = vec![graph_node_index(arena, graph_id, node_id)];
    if let Some(child_graph) = arena.nodes[node_id as usize].nested_graph
        && let Some((&dummy_id, _)) =
            arena
                .external_port_origins
                .iter()
                .find(|(dummy_id, origin_port)| {
                    **origin_port == port_id
                        && arena.nodes[**dummy_id as usize].graph == child_graph
                })
        && let Some((inside_port, _)) = compound_dummy_inside_endpoint(arena, dummy_id)
    {
        let (nested_path, terminal_port_index) =
            compound_inside_traversal_key(arena, child_graph, inside_port);
        path.extend(nested_path);
        return (path, terminal_port_index);
    }
    (path, node_port_index(arena, node_id, port_id))
}

fn graph_node_index(arena: &LayeredArena, graph_id: LGraphId, wanted: LNodeId) -> usize {
    arena.graphs[graph_id as usize]
        .layerless_nodes
        .iter()
        .copied()
        .filter(|node_id| arena.nodes[*node_id as usize].node_type != LayeredNodeType::ExternalPort)
        .position(|node_id| node_id == wanted)
        .unwrap_or(usize::MAX)
}

fn node_port_index(arena: &LayeredArena, node_id: LNodeId, wanted: LPortId) -> usize {
    arena.nodes[node_id as usize]
        .ports
        .iter()
        .position(|port_id| *port_id == wanted)
        .unwrap_or(usize::MAX)
}

fn compare_compound_traversal_paths(first: &[usize], second: &[usize]) -> Ordering {
    for (first_index, second_index) in first.iter().zip(second) {
        match first_index.cmp(second_index) {
            Ordering::Equal => {}
            ordering => return ordering,
        }
    }
    // The Java transform recurses before processing ports directly on a
    // compound child, so a descendant path sorts before its prefix.
    second.len().cmp(&first.len())
}

/// Mirrors `LayerSweepCrossingMinimizer.setPortOrderOnParentGraph`: a nested
/// graph's selected external-dummy order becomes the owning compound node's
/// fixed port order before its parent graph is swept.
fn copy_ordered_external_ports_to_parent(
    arena: &mut LayeredArena,
    child_graph_id: LGraphId,
) -> Result<(), LayeringError> {
    let Some(parent_node) = arena.graphs[child_graph_id as usize].parent_node else {
        return Ok(());
    };
    let has_external_ports = arena
        .external_port_origins
        .keys()
        .any(|dummy_id| arena.nodes[*dummy_id as usize].graph == child_graph_id);
    if !has_external_ports {
        return Ok(());
    }

    sort_parent_ports_by_external_dummy_order(arena, child_graph_id, parent_node, true)?;
    sort_parent_ports_by_external_dummy_order(arena, child_graph_id, parent_node, false)?;
    arena.nodes[parent_node as usize].port_constraints = PortConstraints::FixedOrder;
    Ok(())
}

/// Replaces east/west parent ports in source port-list order with their
/// external dummies in the corresponding end layer, as elk-go does after P3.
fn sort_parent_ports_by_external_dummy_order(
    arena: &mut LayeredArena,
    child_graph_id: LGraphId,
    parent_node: LNodeId,
    on_rightmost_layer: bool,
) -> Result<(), LayeringError> {
    let layer_ids = &arena.graphs[child_graph_id as usize].layers;
    let end_layer_index = if on_rightmost_layer {
        layer_ids.len().checked_sub(1)
    } else {
        Some(0)
    };
    let Some(end_layer_id) = end_layer_index
        .and_then(|index| layer_ids.get(index))
        .copied()
    else {
        return Ok(());
    };
    let end_layer = arena.layers[end_layer_id as usize].nodes.clone();
    let mut dummy_index = if on_rightmost_layer {
        0_isize
    } else {
        end_layer.len() as isize - 1
    };
    if usize::try_from(dummy_index)
        .ok()
        .and_then(|index| end_layer.get(index))
        .is_none_or(|node_id| {
            arena.nodes[*node_id as usize].node_type != LayeredNodeType::ExternalPort
        })
    {
        return Ok(());
    }

    let mut parent_ports = arena.nodes[parent_node as usize].ports.clone();
    for (parent_index, port_id) in parent_ports.clone().into_iter().enumerate() {
        let port = &arena.ports[port_id as usize];
        let on_end_side = if on_rightmost_layer {
            port.side == PortSide::East
        } else {
            port.side == PortSide::West
        };
        let inside_connection =
            arena
                .external_port_origins
                .iter()
                .any(|(dummy_id, origin_port)| {
                    *origin_port == port_id
                        && arena.nodes[*dummy_id as usize].graph == child_graph_id
                });
        if !on_end_side || !inside_connection {
            continue;
        }
        let Some(dummy_id) = usize::try_from(dummy_index)
            .ok()
            .and_then(|index| end_layer.get(index))
            .copied()
        else {
            let port_state = parent_ports
                .iter()
                .map(|candidate| {
                    let port = &arena.ports[*candidate as usize];
                    let represented = arena.external_port_origins.iter().any(|(dummy, origin)| {
                        *origin == *candidate
                            && arena.nodes[*dummy as usize].graph == child_graph_id
                    });
                    format!("{candidate}:{:?}:inside={represented}", port.side)
                })
                .collect::<Vec<_>>()
                .join(",");
            let layer_state = end_layer
                .iter()
                .map(|candidate| arena.nodes[*candidate as usize].source_id.as_str())
                .collect::<Vec<_>>()
                .join(",");
            return Err(LayeringError::Import(format!(
                "hierarchical port/dummy order mismatch for parent {parent_node}, graph {child_graph_id}, rightmost={on_rightmost_layer}, dummy_index={dummy_index}, ports=[{port_state}], end_layer=[{layer_state}]"
            )));
        };
        let Some(origin_port) = arena.external_port_origins.get(&dummy_id).copied() else {
            return Err(LayeringError::Import(format!(
                "external port dummy {dummy_id} has no parent-port origin"
            )));
        };
        parent_ports[parent_index] = origin_port;
        if on_rightmost_layer {
            dummy_index += 1;
        } else {
            dummy_index -= 1;
        }
    }
    arena.nodes[parent_node as usize].ports = parent_ports;
    Ok(())
}

/// Packs a nested graph that has no local edges using the default ELK
/// component stack. ELK's compound importer performs this before the owning
/// graph is layered; keeping it as a separate preprocessor lets the root
/// solver consume the resized vessel and still publish child coordinates in
/// the owner's local frame.
pub fn pack_nested_graphs(
    arena: &mut LayeredArena,
    graph_id: LGraphId,
) -> Result<(), LayeringError> {
    let node_ids = arena
        .nodes
        .iter()
        .enumerate()
        .filter_map(|(node_id, node)| (node.graph == graph_id).then_some(node_id as LNodeId))
        .collect::<Vec<_>>();
    for node_id in &node_ids {
        if let Some(child_graph) = arena.nodes[*node_id as usize].nested_graph {
            pack_nested_graphs(arena, child_graph)?;
        }
    }
    if arena.graphs[graph_id as usize].parent_node.is_none() {
        return Ok(());
    }
    let has_local_edges = arena.edges.iter().any(|edge| {
        edge.active
            && arena.nodes[edge.source as usize].graph == graph_id
            && arena.nodes[edge.target as usize].graph == graph_id
    });
    if has_local_edges || node_ids.is_empty() {
        return Ok(());
    }
    const NODE_NODE_SPACING: f64 = 20.0;
    let padding = arena.graphs[graph_id as usize]
        .padding
        .unwrap_or(GraphPadding {
            top: 12.0,
            right: 12.0,
            bottom: 12.0,
            left: 12.0,
        });
    let mut y = padding.top;
    let mut width: f64 = 0.0;
    for node_id in &node_ids {
        let node = &mut arena.nodes[*node_id as usize];
        node.position_x = padding.left;
        node.position_y = y;
        let height = if node.nested_graph.is_some() {
            (node.height - node.margin_top - node.margin_bottom).max(0.0)
        } else {
            node.height
        };
        let node_width = if node.nested_graph.is_some() {
            (node.width - node.margin_left - node.margin_right).max(0.0)
        } else {
            node.width
        };
        y += height + NODE_NODE_SPACING;
        width = width.max(node_width);
    }
    let mut width = width + padding.left + padding.right;
    let mut height = y - NODE_NODE_SPACING + padding.bottom;
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
    if fixed_shift_x != 0.0 || fixed_shift_y != 0.0 {
        for node_id in &node_ids {
            let node = &mut arena.nodes[*node_id as usize];
            node.position_x += fixed_shift_x;
            node.position_y += fixed_shift_y;
            if let Some((internal, _)) = arena.nested_transfer_x.get_mut(node_id) {
                *internal += fixed_shift_x;
            }
            if let Some((internal, _)) = arena.nested_transfer_y.get_mut(node_id) {
                *internal += fixed_shift_y;
            }
        }
    }
    // The imported compound node already includes its source-space label and
    // icon lane. ELK treats that wire box as a minimum while fitting the
    // child graph; replacing it with the child-only envelope drops outside
    // decoration bands. Preserve both axes before publishing the scope.
    if let Some(parent_node) = arena.graphs[graph_id as usize].parent_node {
        let parent = &arena.nodes[parent_node as usize];
        if parent.margin_left > 0.0 || parent.margin_right > 0.0 {
            width = width.max(parent.width);
        }
        if parent.margin_top > 0.0 || parent.margin_bottom > 0.0 {
            height = height.max(parent.height);
        }
    }
    let graph = &mut arena.graphs[graph_id as usize];
    graph.width = width;
    graph.height = height;
    if let Some(parent_node) = graph.parent_node {
        arena.nodes[parent_node as usize].width = width;
        arena.nodes[parent_node as usize].height = height;
    }
    Ok(())
}

/// Positions explicit side-constrained ports before layering. ELK distributes
/// a port group over the owning side when no fixed coordinate survives import;
/// index order is reversed on west/south sides because those sides face the
/// preceding layer in the normalized left-to-right frame.
fn position_constrained_ports(arena: &mut LayeredArena) {
    for node_id in 0..arena.nodes.len() as LNodeId {
        let node = arena.nodes[node_id as usize].clone();
        for side in [
            PortSide::East,
            PortSide::West,
            PortSide::North,
            PortSide::South,
        ] {
            let mut ports = node
                .ports
                .iter()
                .copied()
                .filter(|port_id| {
                    let port = &arena.ports[*port_id as usize];
                    !port.implicit
                        && port.side == side
                        && !arena.port_border_offsets.contains_key(port_id)
                })
                .collect::<Vec<_>>();
            if ports.is_empty() {
                continue;
            }
            ports.sort_by_key(|port_id| {
                let port = &arena.ports[*port_id as usize];
                (port.index.unwrap_or(i32::MAX), *port_id)
            });
            if matches!(side, PortSide::West | PortSide::South) {
                ports.reverse();
            }
            let count = ports.len() as f64;
            for (position, port_id) in ports.into_iter().enumerate() {
                let port = &mut arena.ports[port_id as usize];
                match side {
                    PortSide::East => {
                        port.x = node.width - port.width;
                        if node.port_constraints != PortConstraints::FixedPosition {
                            port.y = node.height * (position as f64 + 1.0) / (count + 1.0)
                                - port.height / 2.0;
                        }
                    }
                    PortSide::West => {
                        port.x = 0.0;
                        if node.port_constraints != PortConstraints::FixedPosition {
                            port.y = node.height * (position as f64 + 1.0) / (count + 1.0)
                                - port.height / 2.0;
                        }
                    }
                    PortSide::North => {
                        if node.port_constraints != PortConstraints::FixedPosition {
                            port.x = node.width * (position as f64 + 1.0) / (count + 1.0)
                                - port.width / 2.0;
                        }
                        port.y = 0.0;
                    }
                    PortSide::South => {
                        if node.port_constraints != PortConstraints::FixedPosition {
                            port.x = node.width * (position as f64 + 1.0) / (count + 1.0)
                                - port.width / 2.0;
                        }
                        port.y = node.height - port.height;
                    }
                    PortSide::Undefined => {}
                }
            }
        }
    }
}

fn distribute_implicit_port_positions(arena: &mut LayeredArena) {
    for node_id in 0..arena.nodes.len() as LNodeId {
        distribute_implicit_port_positions_for_node(arena, node_id);
    }
}

fn distribute_implicit_port_positions_for_node(arena: &mut LayeredArena, node_id: LNodeId) {
    let Some(node) = arena.nodes.get(node_id as usize) else {
        return;
    };
    let node_height = node.height;
    let node_width = node.width;
    let ports = node.ports.clone();
    let self_loop_ports = arena
        .edges
        .iter()
        .filter(|edge| edge.active && edge.source == node_id && edge.target == node_id)
        .filter_map(|edge| Some((edge.source_port?, edge.target_port?)))
        .filter(|(source, target)| {
            arena.ports[*source as usize].implicit
                && arena.ports[*target as usize].implicit
                && arena.ports[*source as usize].side == arena.ports[*target as usize].side
        })
        .collect::<Vec<_>>();
    for side in [
        PortSide::North,
        PortSide::East,
        PortSide::South,
        PortSide::West,
    ] {
        let side_loops = self_loop_ports
            .iter()
            .copied()
            .filter(|(source, _)| arena.ports[*source as usize].side == side)
            .collect::<Vec<_>>();
        let port_count = side_loops.len() * 2;
        for (loop_index, (source, target)) in side_loops.iter().copied().enumerate() {
            // ELK's self-loop restorer puts the first loop in the inner
            // slot pair and grows subsequent loops outward.
            let mut source_index = side_loops.len() - 1 - loop_index;
            let mut target_index = side_loops.len() + loop_index;
            if matches!(side, PortSide::South | PortSide::West) {
                std::mem::swap(&mut source_index, &mut target_index);
            }
            for (port_id, index) in [(source, source_index), (target, target_index)] {
                let port = &mut arena.ports[port_id as usize];
                let horizontal = matches!(side, PortSide::North | PortSide::South);
                let extent = if horizontal { node_width } else { node_height };
                // label_and_node_size.go advances a running position;
                // multiplication changes the final bits for many ports.
                let initial_gap = 10.0;
                let gap = initial_gap
                    + (extent - initial_gap * (port_count as f64 + 1.0))
                        / (port_count as f64 + 1.0);
                let mut coordinate = gap;
                for _ in 0..index {
                    coordinate += gap;
                }
                port.x = if horizontal {
                    coordinate
                } else if side == PortSide::East {
                    node_width
                } else {
                    0.0
                };
                port.y = if !horizontal {
                    coordinate
                } else if side == PortSide::South {
                    node_height
                } else {
                    0.0
                };
            }
        }
    }
    let self_loop_port_ids = self_loop_ports
        .iter()
        .flat_map(|(source, target)| [*source, *target])
        .collect::<BTreeSet<_>>();
    for side in [PortSide::North, PortSide::South] {
        let implicit = ports
            .iter()
            .copied()
            .filter(|port| {
                let value = &arena.ports[*port as usize];
                value.implicit && value.side == side && !self_loop_port_ids.contains(port)
            })
            .collect::<Vec<_>>();
        let count = implicit.len();
        if count == 0 {
            continue;
        }
        for (index, port_id) in implicit.into_iter().enumerate() {
            let port = &mut arena.ports[port_id as usize];
            port.x = node_width * ((index as f64 + 1.0) / (count as f64 + 1.0));
            port.y = if side == PortSide::North {
                0.0
            } else {
                node_height
            };
        }
    }
    for side in [PortSide::East, PortSide::West] {
        let implicit = ports
            .iter()
            .copied()
            .filter(|port| {
                let value = &arena.ports[*port as usize];
                value.implicit && value.side == side && !self_loop_port_ids.contains(port)
            })
            .collect::<Vec<_>>();
        let count = implicit.len();
        if count == 0 {
            continue;
        }
        for port_id in &implicit {
            arena.ports[*port_id as usize].x = if side == PortSide::East {
                arena.nodes[node_id as usize].width
            } else {
                0.0
            };
        }
        for (index, port_id) in implicit.into_iter().enumerate() {
            if arena.reused_parent_ports.contains(&port_id) {
                continue;
            }
            let reversed_index = index + 1;
            arena.ports[port_id as usize].y =
                node_height * (reversed_index as f64 / (count as f64 + 1.0));
        }
    }
}

fn parse_port_side(options: &serde_json::Value) -> PortSide {
    let value = options
        .get("elk.port.side")
        .or_else(|| options.get("org.eclipse.elk.port.side"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    match value.to_ascii_uppercase().as_str() {
        "NORTH" => PortSide::North,
        "EAST" => PortSide::East,
        "SOUTH" => PortSide::South,
        "WEST" => PortSide::West,
        _ => PortSide::Undefined,
    }
}

fn parse_port_index(options: &serde_json::Value) -> Option<i32> {
    options
        .get("elk.port.index")
        .or_else(|| options.get("org.eclipse.elk.port.index"))
        .and_then(serde_json::Value::as_i64)
        .and_then(|index| i32::try_from(index).ok())
}

fn parse_port_constraints(options: Option<&serde_json::Value>) -> PortConstraints {
    let value = options
        .and_then(|options| {
            options
                .get("elk.portConstraints")
                .or_else(|| options.get("org.eclipse.elk.portConstraints"))
        })
        .and_then(Value::as_str)
        .unwrap_or_default();
    match value.to_ascii_uppercase().as_str() {
        "FREE" => PortConstraints::Free,
        "FIXED_SIDE" => PortConstraints::FixedSide,
        "FIXED_ORDER" => PortConstraints::FixedOrder,
        "FIXED_RATIO" => PortConstraints::FixedRatio,
        "FIXED_POS" | "FIXED_POSITION" => PortConstraints::FixedPosition,
        _ => PortConstraints::Undefined,
    }
}

fn parse_model_order_cycle_breaking(options: Option<&Value>) -> bool {
    options
        .and_then(|options| {
            options
                .get("elk.layered.cycleBreaking.strategy")
                .or_else(|| options.get("org.eclipse.elk.layered.cycleBreaking.strategy"))
        })
        .and_then(Value::as_str)
        .is_some_and(|value| value.eq_ignore_ascii_case("GREEDY_MODEL_ORDER"))
}

fn parse_self_loop_distribution(options: Option<&serde_json::Value>) -> SelfLoopDistribution {
    let value = options
        .and_then(|options| {
            options
                .get("elk.layered.edgeRouting.selfLoopDistribution")
                .or_else(|| options.get("org.eclipse.elk.layered.edgeRouting.selfLoopDistribution"))
        })
        .and_then(Value::as_str)
        .unwrap_or("NORTH");
    match value.to_ascii_uppercase().as_str() {
        "NORTH_SOUTH" => SelfLoopDistribution::NorthSouth,
        "EQUALLY" => SelfLoopDistribution::Equally,
        _ => SelfLoopDistribution::North,
    }
}

fn parse_graph_padding(options: &serde_json::Value) -> Option<GraphPadding> {
    let value = options
        .get("elk.padding")
        .or_else(|| options.get("org.eclipse.elk.padding"))?;
    if let Some(uniform) = value.as_f64() {
        return Some(GraphPadding {
            top: uniform,
            right: uniform,
            bottom: uniform,
            left: uniform,
        });
    }
    if let Some(object) = value.as_object() {
        return Some(GraphPadding {
            top: object.get("top").and_then(Value::as_f64).unwrap_or(0.0),
            right: object.get("right").and_then(Value::as_f64).unwrap_or(0.0),
            bottom: object.get("bottom").and_then(Value::as_f64).unwrap_or(0.0),
            left: object.get("left").and_then(Value::as_f64).unwrap_or(0.0),
        });
    }
    let string = value.as_str()?;
    let mut padding = GraphPadding::default();
    let mut found = false;
    for part in string.trim_matches(['[', ']']).split(',') {
        let Some((name, number)) = part.split_once('=') else {
            continue;
        };
        let Ok(number) = number.trim().parse::<f64>() else {
            continue;
        };
        match name.trim().to_ascii_lowercase().as_str() {
            "top" => padding.top = number,
            "right" => padding.right = number,
            "bottom" => padding.bottom = number,
            "left" => padding.left = number,
            _ => continue,
        }
        found = true;
    }
    if found {
        Some(padding)
    } else {
        string
            .trim()
            .parse::<f64>()
            .ok()
            .map(|uniform| GraphPadding {
                top: uniform,
                right: uniform,
                bottom: uniform,
                left: uniform,
            })
    }
}

fn parse_layering_strategy(options: &Value) -> LayeringStrategy {
    let value = options
        .get("org.eclipse.elk.layered.layering.strategy")
        .or_else(|| options.get("elk.layered.layering.strategy"))
        .or_else(|| options.get("layered.layering.strategy"));
    // JSON options pass through core.EnumOption, whose names are case-sensitive
    // and whose ordinal three names COFFMAN_GRAHAM.
    if value.is_some_and(|value| {
        value.as_f64() == Some(3.0)
            || value
                .as_str()
                .is_some_and(|text| text == "COFFMAN_GRAHAM" || text.parse::<i32>() == Ok(3))
    }) {
        LayeringStrategy::CoffmanGraham
    } else {
        LayeringStrategy::NetworkSimplex
    }
}

fn parse_coffman_bound(options: &Value) -> Option<i32> {
    let value = options
        .get("org.eclipse.elk.layered.layering.coffmanGraham.layerBound")
        .or_else(|| options.get("elk.layered.layering.coffmanGraham.layerBound"))
        .or_else(|| options.get("layered.layering.coffmanGraham.layerBound"))?;
    value
        .as_i64()
        .and_then(|n| i32::try_from(n).ok())
        .or_else(|| {
            value
                .as_f64()
                .filter(|n| {
                    n.is_finite()
                        && n.fract() == 0.0
                        && *n >= i32::MIN as f64
                        && *n <= i32::MAX as f64
                })
                .map(|n| n as i32)
        })
        .or_else(|| value.as_str()?.trim().parse().ok())
}

/// Shared finite-decimal DoubleOption path used by layout option import.
/// Nonfinite, hexadecimal and range-error equivalence are qualified separately.
pub(crate) fn parse_option_double(value: &Value) -> Option<f64> {
    value.as_f64().or_else(|| {
        let mut number = value.as_str()?.trim();
        for suffix in ['d', 'D', 'f', 'F'] {
            number = number.strip_suffix(suffix).unwrap_or(number);
        }
        number.parse().ok()
    })
}

fn parse_port_port_spacing(options: &Value) -> f64 {
    options
        .get("org.eclipse.elk.spacing.portPort")
        .or_else(|| options.get("elk.spacing.portPort"))
        .or_else(|| options.get("spacing.portPort"))
        .and_then(parse_option_double)
        .unwrap_or(10.0)
}

fn parse_edge_edge_spacing(options: &serde_json::Value) -> f64 {
    options
        .get("org.eclipse.elk.spacing.edgeEdge")
        .or_else(|| options.get("elk.spacing.edgeEdge"))
        .or_else(|| options.get("spacing.edgeEdge"))
        .and_then(parse_option_double)
        .unwrap_or(10.0)
}

fn parse_edge_edge_spacing_between_layers(options: &serde_json::Value) -> f64 {
    options
        .get("org.eclipse.elk.layered.spacing.edgeEdgeBetweenLayers")
        .or_else(|| options.get("elk.layered.spacing.edgeEdgeBetweenLayers"))
        .or_else(|| options.get("layered.spacing.edgeEdgeBetweenLayers"))
        .or_else(|| options.get("spacing.edgeEdgeBetweenLayers"))
        .or_else(|| options.get("org.eclipse.elk.spacing.edgeEdgeBetweenLayers"))
        .or_else(|| options.get("elk.spacing.edgeEdgeBetweenLayers"))
        .and_then(parse_option_double)
        .unwrap_or(10.0)
}

fn parse_node_node_spacing(options: &serde_json::Value) -> f64 {
    options
        .get("spacing.nodeNode")
        .or_else(|| options.get("elk.spacing.nodeNode"))
        .or_else(|| options.get("org.eclipse.elk.spacing.nodeNode"))
        .and_then(parse_option_double)
        .unwrap_or(20.0)
}

fn parse_layer_spacing(options: &serde_json::Value) -> f64 {
    options
        .get("org.eclipse.elk.layered.spacing.nodeNodeBetweenLayers")
        .or_else(|| options.get("elk.layered.spacing.nodeNodeBetweenLayers"))
        .or_else(|| options.get("layered.spacing.nodeNodeBetweenLayers"))
        .or_else(|| options.get("spacing.nodeNodeBetweenLayers"))
        .or_else(|| options.get("elk.spacing.nodeNodeBetweenLayers"))
        .or_else(|| options.get("org.eclipse.elk.spacing.nodeNodeBetweenLayers"))
        .and_then(parse_option_double)
        .unwrap_or(20.0)
}

fn parse_edge_node_vertical_spacing(options: &Value) -> f64 {
    options
        .get("org.eclipse.elk.spacing.edgeNode")
        .or_else(|| options.get("elk.spacing.edgeNode"))
        .or_else(|| options.get("spacing.edgeNode"))
        .and_then(parse_option_double)
        .unwrap_or(10.0)
}

fn parse_edge_node_spacing(options: &serde_json::Value) -> f64 {
    options
        .get("org.eclipse.elk.layered.spacing.edgeNodeBetweenLayers")
        .or_else(|| options.get("elk.layered.spacing.edgeNodeBetweenLayers"))
        .or_else(|| options.get("layered.spacing.edgeNodeBetweenLayers"))
        .or_else(|| options.get("spacing.edgeNodeBetweenLayers"))
        .or_else(|| options.get("elk.spacing.edgeNodeBetweenLayers"))
        .or_else(|| options.get("org.eclipse.elk.spacing.edgeNodeBetweenLayers"))
        .and_then(parse_option_double)
        .unwrap_or(10.0)
}

fn parse_self_loop_spacing(options: &serde_json::Value) -> f64 {
    options
        .get("elk.spacing.nodeSelfLoop")
        .or_else(|| options.get("org.eclipse.elk.spacing.nodeSelfLoop"))
        .and_then(parse_option_double)
        .unwrap_or(10.0)
}

fn parse_thoroughness(options: &serde_json::Value) -> usize {
    options
        .get("elk.layered.thoroughness")
        .or_else(|| options.get("org.eclipse.elk.layered.thoroughness"))
        .and_then(|value| {
            value
                .as_u64()
                .or_else(|| {
                    value.as_f64().and_then(|value| {
                        (value.is_finite() && value >= 0.0 && value.fract() == 0.0)
                            .then_some(value as u64)
                    })
                })
                .or_else(|| value.as_str().and_then(|value| value.parse::<u64>().ok()))
        })
        .map(|value| value.min(usize::MAX as u64) as usize)
        .unwrap_or(7)
}

fn parse_random_seed(options: &serde_json::Value) -> i64 {
    options
        .get("elk.randomSeed")
        .or_else(|| options.get("org.eclipse.elk.randomSeed"))
        .and_then(|value| {
            value
                .as_i64()
                .or_else(|| value.as_u64().and_then(|value| i64::try_from(value).ok()))
                .or_else(|| {
                    value.as_f64().and_then(|value| {
                        (value.is_finite()
                            && value.fract() == 0.0
                            && value >= i64::MIN as f64
                            && value <= i64::MAX as f64)
                            .then_some(value as i64)
                    })
                })
                .or_else(|| value.as_str().and_then(|value| value.parse::<i64>().ok()))
        })
        .unwrap_or(1)
}

fn parse_consider_model_order(options: &serde_json::Value) -> bool {
    options
        .get("elk.layered.considerModelOrder.strategy")
        .or_else(|| options.get("org.eclipse.elk.layered.considerModelOrder.strategy"))
        .and_then(serde_json::Value::as_str)
        .is_some_and(|strategy| !strategy.eq_ignore_ascii_case("NONE"))
}

fn parse_port_sorting_by_degree(options: &serde_json::Value) -> bool {
    options
        .get("elk.layered.portSortingStrategy")
        .or_else(|| options.get("org.eclipse.elk.layered.portSortingStrategy"))
        .and_then(serde_json::Value::as_str)
        .is_some_and(|strategy| strategy.eq_ignore_ascii_case("PORT_DEGREE"))
}

fn parse_hierarchical_sweepiness(options: &serde_json::Value) -> Option<f64> {
    options
        .get("elk.layered.crossingMinimization.hierarchicalSweepiness")
        .or_else(|| {
            options.get("org.eclipse.elk.layered.crossingMinimization.hierarchicalSweepiness")
        })
        .and_then(parse_option_double)
}

fn parse_separate_components(options: &serde_json::Value) -> bool {
    options
        .get("elk.separateConnectedComponents")
        .or_else(|| options.get("org.eclipse.elk.separateConnectedComponents"))
        .and_then(|value| {
            value
                .as_bool()
                .or_else(|| value.as_str().and_then(|value| value.parse::<bool>().ok()))
        })
        .unwrap_or(true)
}

fn parse_aspect_ratio(options: &serde_json::Value) -> f64 {
    options
        .get("elk.aspectRatio")
        .or_else(|| options.get("org.eclipse.elk.aspectRatio"))
        .and_then(parse_option_double)
        .filter(|value| value.is_finite() && *value > 0.0)
        // ELK's AspectRatio property defaults to 1.6. This feeds the
        // SimpleRowGraphPlacer's max row width, so using the common 1.0
        // aspect-ratio fallback changes disconnected-component placement.
        .unwrap_or(1.6)
}

fn parse_component_spacing(options: &serde_json::Value) -> f64 {
    options
        .get("elk.spacing.componentComponent")
        .or_else(|| options.get("org.eclipse.elk.spacing.componentComponent"))
        .and_then(parse_option_double)
        .filter(|value| value.is_finite() && *value >= 0.0)
        .unwrap_or(20.0)
}

fn parse_greedy_switch_mode(options: &serde_json::Value) -> GreedySwitchMode {
    let value = options
        .get("elk.layered.crossingMinimization.greedySwitch.type")
        .or_else(|| options.get("org.eclipse.elk.layered.crossingMinimization.greedySwitch.type"));
    match value {
        Some(Value::Number(value)) => match value.as_u64() {
            Some(0) => GreedySwitchMode::OneSided,
            Some(1) => GreedySwitchMode::TwoSided,
            Some(2) => GreedySwitchMode::Off,
            _ => GreedySwitchMode::TwoSided,
        },
        Some(Value::String(value)) => match value.to_ascii_uppercase().as_str() {
            "ONE_SIDED" | "ONESIDED" => GreedySwitchMode::OneSided,
            "OFF" => GreedySwitchMode::Off,
            _ => GreedySwitchMode::TwoSided,
        },
        _ => GreedySwitchMode::TwoSided,
    }
}

fn parse_greedy_switch_activation_threshold(options: &serde_json::Value) -> usize {
    options
        .get("elk.layered.crossingMinimization.greedySwitch.activationThreshold")
        .or_else(|| {
            options.get(
                "org.eclipse.elk.layered.crossingMinimization.greedySwitch.activationThreshold",
            )
        })
        .and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_str().and_then(|value| value.parse::<u64>().ok()))
        })
        .map(|value| value.min(usize::MAX as u64) as usize)
        .unwrap_or(40)
}

fn parse_semi_interactive(options: &serde_json::Value) -> bool {
    options
        .get("elk.layered.crossingMinimization.semiInteractive")
        .or_else(|| options.get("org.eclipse.elk.layered.crossingMinimization.semiInteractive"))
        .and_then(|value| {
            value
                .as_bool()
                .or_else(|| value.as_str().and_then(|value| value.parse::<bool>().ok()))
        })
        .unwrap_or(false)
}

fn parse_interactive_crossing_minimization(options: &serde_json::Value) -> bool {
    options
        .get("elk.layered.crossingMinimization.strategy")
        .or_else(|| options.get("org.eclipse.elk.layered.crossingMinimization.strategy"))
        .and_then(Value::as_str)
        .is_some_and(|value| value.eq_ignore_ascii_case("INTERACTIVE"))
}

fn parse_edge_priority_shortness(options: Option<&serde_json::Value>) -> f64 {
    let Some(options) = options else {
        return 1.0;
    };
    let Some(value) = options
        .get("elk.layered.priority.shortness")
        .or_else(|| options.get("org.eclipse.elk.layered.priority.shortness"))
    else {
        return 1.0;
    };
    let parsed = value
        .as_i64()
        .map(|value| value as f64)
        .or_else(|| value.as_u64().map(|value| value as f64))
        .or_else(|| value.as_f64().filter(|value| value.is_finite()))
        .or_else(|| value.as_str().and_then(|value| value.parse::<f64>().ok()));
    parsed.filter(|value| *value >= 1.0).unwrap_or(1.0)
}

fn parse_edge_thickness(options: Option<&serde_json::Value>) -> f64 {
    options
        .and_then(|options| {
            options
                .get("elk.edge.thickness")
                .or_else(|| options.get("org.eclipse.elk.edge.thickness"))
        })
        .and_then(parse_option_double)
        .filter(|value| value.is_finite() && *value >= 0.0)
        .unwrap_or(1.0)
}

fn parse_edge_label_spacing(options: &serde_json::Value) -> f64 {
    options
        .get("elk.spacing.edgeLabel")
        .or_else(|| options.get("org.eclipse.elk.spacing.edgeLabel"))
        .and_then(parse_option_double)
        .unwrap_or(2.0)
}

fn parse_label_label_spacing(options: &serde_json::Value) -> f64 {
    options
        .get("elk.spacing.labelLabel")
        .or_else(|| options.get("org.eclipse.elk.spacing.labelLabel"))
        .and_then(parse_option_double)
        .unwrap_or(0.0)
}

/// Reads ELK's spacing object in the forms emitted by D2 and elk-go.
///
/// The importer keeps this deliberately small: margins are already resolved
/// by the public adapter in normal D2 layouts, while this accepts the object,
/// uniform-number, and `[top=..,left=..]` string forms used by the oracle
/// fixtures.
fn parse_node_margins(options: Option<&Value>) -> [f64; 4] {
    let Some(options) = options else {
        return [0.0; 4];
    };
    let value = options
        .get("org.eclipse.elk.margins")
        .or_else(|| options.get("elk.margins"));
    let Some(value) = value else {
        return [0.0; 4];
    };
    if let Some(uniform) = value.as_f64() {
        return [uniform; 4];
    }
    if let Some(object) = value.as_object() {
        return [
            object.get("top").and_then(Value::as_f64).unwrap_or(0.0),
            object.get("right").and_then(Value::as_f64).unwrap_or(0.0),
            object.get("bottom").and_then(Value::as_f64).unwrap_or(0.0),
            object.get("left").and_then(Value::as_f64).unwrap_or(0.0),
        ];
    }
    let Some(string) = value.as_str() else {
        return [0.0; 4];
    };
    let mut result = [0.0; 4];
    let mut seen = [false; 4];
    for part in string.trim_matches(['[', ']']).split(',') {
        let Some((name, number)) = part.split_once('=') else {
            continue;
        };
        let Ok(number) = number.trim().parse::<f64>() else {
            continue;
        };
        let index = match name.trim().to_ascii_lowercase().as_str() {
            "top" => Some(0),
            "right" => Some(1),
            "bottom" => Some(2),
            "left" => Some(3),
            _ => None,
        };
        if let Some(index) = index {
            result[index] = number;
            seen[index] = true;
        }
    }
    if !seen.iter().any(|seen| *seen)
        && let Ok(uniform) = string.trim().parse::<f64>()
    {
        return [uniform; 4];
    }
    result
}

fn parse_node_size_minimum(options: &Value) -> Option<(f64, f64)> {
    let value = options
        .get("elk.nodeSize.minimum")
        .or_else(|| options.get("org.eclipse.elk.nodeSize.minimum"))?;
    if let Some(values) = value.as_array() {
        return Some((values.first()?.as_f64()?, values.get(1)?.as_f64()?));
    }
    if let Some(object) = value.as_object() {
        let width = object.get("x").or_else(|| object.get("width"))?.as_f64()?;
        let height = object.get("y").or_else(|| object.get("height"))?.as_f64()?;
        return Some((width, height));
    }
    let string = value.as_str()?.trim();
    let pair = string.trim_start_matches('(').trim_end_matches(')');
    let (width, height) = pair.split_once(',')?;
    Some((width.trim().parse().ok()?, height.trim().parse().ok()?))
}

fn parse_direction(options: &serde_json::Value) -> LayerDirection {
    parse_direction_option(options).unwrap_or(LayerDirection::Right)
}

// core.Engine.evaluateHierarchyHandling resolves INHERIT against the parent;
// a root without an explicit policy starts a separate hierarchy.
fn hierarchy_includes_children(options: &Value, inherited: bool) -> bool {
    match options
        .get("elk.hierarchyHandling")
        .or_else(|| options.get("org.eclipse.elk.hierarchyHandling"))
        .and_then(Value::as_str)
    {
        Some(value) if value.eq_ignore_ascii_case("INCLUDE_CHILDREN") => true,
        Some(value) if value.eq_ignore_ascii_case("SEPARATE_CHILDREN") => false,
        _ => inherited,
    }
}

fn parse_direction_option(options: &serde_json::Value) -> Option<LayerDirection> {
    let value = options
        .get("elk.direction")
        .or_else(|| options.get("org.eclipse.elk.direction"))
        .and_then(serde_json::Value::as_str)?;
    match value.to_ascii_uppercase().as_str() {
        "LEFT" => Some(LayerDirection::Left),
        "DOWN" => Some(LayerDirection::Down),
        "UP" => Some(LayerDirection::Up),
        "RIGHT" => Some(LayerDirection::Right),
        _ => None,
    }
}

fn parse_bk_fixed_alignment(options: &serde_json::Value) -> BkFixedAlignment {
    let value = options
        .get("elk.layered.nodePlacement.bk.fixedAlignment")
        .or_else(|| options.get("org.eclipse.elk.layered.nodePlacement.bk.fixedAlignment"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    match value.to_ascii_uppercase().as_str() {
        "LEFTUP" => BkFixedAlignment::LeftUp,
        "RIGHTUP" => BkFixedAlignment::RightUp,
        "LEFTDOWN" => BkFixedAlignment::LeftDown,
        "RIGHTDOWN" => BkFixedAlignment::RightDown,
        "BALANCED" => BkFixedAlignment::Balanced,
        _ => BkFixedAlignment::None,
    }
}

fn parse_favor_straight_edges(options: &serde_json::Value) -> bool {
    options
        .get("elk.layered.nodePlacement.favorStraightEdges")
        .or_else(|| options.get("org.eclipse.elk.layered.nodePlacement.favorStraightEdges"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or_else(|| {
            options
                .get("elk.edgeRouting")
                .or_else(|| options.get("org.eclipse.elk.edgeRouting"))
                .and_then(serde_json::Value::as_str)
                .is_none_or(|routing| routing.eq_ignore_ascii_case("ORTHOGONAL"))
        })
}

fn parse_bk_edge_straightening(options: &serde_json::Value) -> bool {
    options
        .get("elk.layered.nodePlacement.bk.edgeStraightening")
        .or_else(|| options.get("org.eclipse.elk.layered.nodePlacement.bk.edgeStraightening"))
        .and_then(serde_json::Value::as_str)
        .is_none_or(|value| !value.eq_ignore_ascii_case("NONE"))
}

/// Errors raised while translating the wire model into the layered arena.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum LayeredImportError {
    /// Two nodes used the same stable ELK ID.
    #[error("duplicate ELK node id {0:?}")]
    DuplicateNodeId(String),
    /// An edge referenced a node that was not imported.
    #[error("edge {edge:?} references unknown endpoint {endpoint:?}")]
    UnknownEndpoint {
        /// Edge identifier from the wire graph.
        edge: String,
        /// Missing source or target identifier.
        endpoint: String,
    },
}

/// Errors raised by the standalone longest-path phase.
#[derive(Clone, Debug, Error, PartialEq)]
pub enum LayeringError {
    /// A registered provider rejected an imported graph during layout.
    #[error("layout algorithm {algorithm}: {message}")]
    Provider {
        /// Canonical registered provider identifier.
        algorithm: &'static str,
        /// Original provider diagnostic.
        message: String,
    },
    /// The wire graph could not be imported into the layered arena.
    #[error("could not import layered graph: {0}")]
    Import(String),
    /// Longest-path layering requires the cycle-breaking phase first.
    #[error("longest-path layering encountered a cycle at node {0}")]
    Cycle(LNodeId),
    /// The graph ID was not present in the arena.
    #[error("unknown layered graph {0}")]
    UnknownGraph(LGraphId),
    /// An edge ID did not exist while a phase was mutating the arena.
    #[error("unknown layered edge {0}")]
    UnknownEdge(LEdgeId),
    /// The simple placer received an invalid spacing value.
    #[error("layer spacing must be finite and non-negative, got {0}")]
    InvalidSpacing(f64),
    /// ELK cannot sort a fixed-order port that has no side.
    #[error("layered: cannot sort a port with undefined side")]
    PortWithUndefinedSide,
}

pub(crate) fn owner_in_graph(
    arena: &LayeredArena,
    mut node_id: LNodeId,
    graph_id: LGraphId,
) -> Option<LNodeId> {
    let mut visited = BTreeSet::new();
    loop {
        if !visited.insert(node_id) {
            return None;
        }
        let node = arena.nodes.get(node_id as usize)?;
        if node.graph == graph_id {
            return Some(node_id);
        }
        let parent_graph = node.graph;
        node_id = arena.graphs.get(parent_graph as usize)?.parent_node?;
    }
}

/// Reverses cyclic edges using ELK's `GREEDY_MODEL_ORDER` cycle breaker.
///
/// The implementation follows `p1cycles/greedy.go`: sinks are removed first,
/// then sources, and a remaining node with maximum outflow is selected using
/// model order for ties. Reversal mutates the arena endpoints and records the
/// direction on each edge, which is the state consumed by later phases.
pub fn break_cycles_greedy_model_order(
    arena: &mut LayeredArena,
    graph_id: LGraphId,
) -> Result<Vec<LEdgeId>, LayeringError> {
    break_cycles_greedy_impl(arena, graph_id, true)
}

pub(crate) fn break_cycles_for_layout(
    arena: &mut LayeredArena,
    graph_id: LGraphId,
) -> Result<Vec<LEdgeId>, LayeringError> {
    let model_order = arena
        .graphs
        .get(graph_id as usize)
        .ok_or(LayeringError::UnknownGraph(graph_id))?
        .cycle_breaking_model_order;
    let graph = &arena.graphs[graph_id as usize];
    let external = graph
        .layerless_nodes
        .iter()
        .any(|node| arena.nodes[*node as usize].node_type == LayeredNodeType::ExternalPort);
    let external_order_fixed = graph.parent_node.is_some_and(|parent| {
        matches!(
            arena.nodes[parent as usize].port_constraints,
            PortConstraints::FixedOrder
                | PortConstraints::FixedRatio
                | PortConstraints::FixedPosition
        )
    });
    if graph.separated_component
        || graph.includes_children
        || !graph.separate_components
        || (external && external_order_fixed)
    {
        return break_cycles_greedy_impl(arena, graph_id, model_order);
    }
    // ComponentsProcessor.Split discovers this order before P1, including a
    // single connected component. Model order remains an independent property.
    let original = graph.layerless_nodes.clone();
    let node_set = original.iter().copied().collect::<BTreeSet<_>>();
    let edges = arena
        .edges
        .iter()
        .enumerate()
        .filter_map(|(id, edge)| {
            (edge.active && node_set.contains(&edge.source) && node_set.contains(&edge.target))
                .then_some(id as LEdgeId)
        })
        .collect::<Vec<_>>();
    let mut components = source_layering_connected_components(arena, &original, &edges);
    let positions = original
        .iter()
        .enumerate()
        .map(|(i, node)| (*node, i))
        .collect::<BTreeMap<_, _>>();
    components.sort_by_key(|component| component.iter().map(|node| positions[node]).min());
    let result = (|| {
        let mut reversed = Vec::new();
        for component in components {
            arena.graphs[graph_id as usize].layerless_nodes = component;
            reversed.extend(break_cycles_greedy_impl(arena, graph_id, model_order)?);
        }
        Ok(reversed)
    })();
    arena.graphs[graph_id as usize].layerless_nodes = original;
    result
}

fn break_cycles_greedy_impl(
    arena: &mut LayeredArena,
    graph_id: LGraphId,
    model_order: bool,
) -> Result<Vec<LEdgeId>, LayeringError> {
    if graph_id as usize >= arena.graphs.len() {
        return Err(LayeringError::UnknownGraph(graph_id));
    }
    let mut random = (!model_order).then(|| arena.take_graph_random(graph_id));
    let nodes = arena.graphs[graph_id as usize].layerless_nodes.clone();
    let count = nodes.len();
    let mut indegree = vec![0usize; count];
    let mut outdegree = vec![0usize; count];
    let mut node_index = BTreeMap::<LNodeId, usize>::new();
    for (index, node_id) in nodes.iter().copied().enumerate() {
        node_index.insert(node_id, index);
    }
    for (edge_id, edge) in arena.edges.iter().enumerate() {
        if !edge.active {
            continue;
        }
        let (Some(source), Some(target)) = (
            owner_in_graph(arena, edge.source, graph_id),
            owner_in_graph(arena, edge.target, graph_id),
        ) else {
            continue;
        };
        let (Some(&source), Some(&target)) = (node_index.get(&source), node_index.get(&target))
        else {
            continue;
        };
        if source == target {
            continue;
        }
        let weight = arena
            .edge_direction_weight
            .get(&(edge_id as LEdgeId))
            .copied()
            .unwrap_or(1);
        outdegree[source] += weight;
        indegree[target] += weight;
    }

    let mut sources = nodes
        .iter()
        .copied()
        .enumerate()
        .filter_map(|(index, node)| (indegree[index] == 0 && outdegree[index] > 0).then_some(node))
        .collect::<Vec<_>>();
    let mut sinks = nodes
        .iter()
        .copied()
        .enumerate()
        .filter_map(|(index, node)| (outdegree[index] == 0).then_some(node))
        .collect::<Vec<_>>();
    let mut marks = vec![0isize; count];
    let mut unprocessed = count;
    let mut next_right = -1isize;
    let mut next_left = 1isize;

    #[cfg(feature = "diagnostic-traces")]
    trace_cycle_state(
        arena, graph_id, "begin", -1, &nodes, &indegree, &outdegree, &marks, &sources, &sinks,
    );
    while unprocessed > 0 {
        while !sinks.is_empty() {
            let node = sinks.remove(0);
            let index = node_index[&node];
            if marks[index] != 0 {
                continue;
            }
            marks[index] = next_right;
            next_right -= 1;
            update_cycle_neighbors(
                arena,
                graph_id,
                node,
                &node_index,
                &mut indegree,
                &mut outdegree,
                &marks,
                &mut sources,
                &mut sinks,
            );
            #[cfg(feature = "diagnostic-traces")]
            trace_cycle_state(
                arena,
                graph_id,
                "sink",
                i64::from(node),
                &nodes,
                &indegree,
                &outdegree,
                &marks,
                &sources,
                &sinks,
            );
            unprocessed -= 1;
        }
        while !sources.is_empty() {
            let node = sources.remove(0);
            let index = node_index[&node];
            if marks[index] != 0 {
                continue;
            }
            marks[index] = next_left;
            next_left += 1;
            update_cycle_neighbors(
                arena,
                graph_id,
                node,
                &node_index,
                &mut indegree,
                &mut outdegree,
                &marks,
                &mut sources,
                &mut sinks,
            );
            #[cfg(feature = "diagnostic-traces")]
            trace_cycle_state(
                arena,
                graph_id,
                "source",
                i64::from(node),
                &nodes,
                &indegree,
                &outdegree,
                &marks,
                &sources,
                &sinks,
            );
            unprocessed -= 1;
        }
        if unprocessed == 0 {
            break;
        }
        let maximum_outflow = nodes
            .iter()
            .enumerate()
            .filter(|(index, _)| marks[*index] == 0)
            .map(|(index, _)| outdegree[index] as isize - indegree[index] as isize)
            .max()
            .ok_or(LayeringError::Cycle(nodes[0]))?;
        let choices = nodes
            .iter()
            .enumerate()
            .filter_map(|(index, node)| {
                (marks[index] == 0
                    && outdegree[index] as isize - indegree[index] as isize == maximum_outflow)
                    .then_some(*node)
            })
            .collect::<Vec<_>>();
        let ordered = model_order
            .then(|| {
                choices
                    .iter()
                    .copied()
                    .filter(|node| arena.nodes[*node as usize].node_type == LayeredNodeType::Normal)
                    .min_by_key(|node| arena.nodes[*node as usize].traversal_index)
            })
            .flatten();
        let chosen = if let Some(node) = ordered {
            node
        } else {
            if random.is_none() {
                random = Some(arena.take_graph_random(graph_id));
            }
            arena.graphs[graph_id as usize].cycle_random_consumed = true;
            choices[random
                .as_mut()
                .expect("cycle RNG")
                .next_int_n(choices.len())]
        };
        let index = node_index[&chosen];
        marks[index] = next_left;
        next_left += 1;
        update_cycle_neighbors(
            arena,
            graph_id,
            chosen,
            &node_index,
            &mut indegree,
            &mut outdegree,
            &marks,
            &mut sources,
            &mut sinks,
        );
        #[cfg(feature = "diagnostic-traces")]
        trace_cycle_state(
            arena,
            graph_id,
            "maximum",
            i64::from(chosen),
            &nodes,
            &indegree,
            &outdegree,
            &marks,
            &sources,
            &sinks,
        );
        unprocessed -= 1;
    }

    let shift = count as isize + 1;
    for mark in &mut marks {
        if *mark < 0 {
            *mark += shift;
        }
    }
    let mut reversed = Vec::new();
    for node in &nodes {
        let edges = if arena.nodes[*node as usize].ports.is_empty() {
            arena
                .outgoing_edges(*node)
                .map(|(id, _)| id)
                .collect::<Vec<_>>()
        } else {
            arena.node_outgoing_edges(*node)
        };
        for edge_id in edges {
            let edge = &arena.edges[edge_id as usize];
            let (Some(source), Some(target)) = (
                owner_in_graph(arena, edge.source, graph_id),
                owner_in_graph(arena, edge.target, graph_id),
            ) else {
                continue;
            };
            let (Some(&source), Some(&target)) = (node_index.get(&source), node_index.get(&target))
            else {
                continue;
            };
            if marks[source] > marks[target] {
                arena.reverse_edge(edge_id);
                reversed.push(edge_id);
            }
        }
    }
    #[cfg(feature = "diagnostic-traces")]
    trace_cycle_state(
        arena, graph_id, "end", -1, &nodes, &indegree, &outdegree, &marks, &sources, &sinks,
    );
    if let Some(random) = random {
        arena.restore_graph_random(graph_id, random);
    }
    Ok(reversed)
}

#[cfg(feature = "diagnostic-traces")]
#[allow(clippy::too_many_arguments)]
fn trace_cycle_state(
    arena: &LayeredArena,
    graph: LGraphId,
    operation: &str,
    chosen: i64,
    nodes: &[LNodeId],
    indegree: &[usize],
    outdegree: &[usize],
    marks: &[isize],
    sources: &[LNodeId],
    sinks: &[LNodeId],
) {
    let mut edges = Vec::new();
    for node in nodes {
        for id in arena.node_outgoing_edges(*node) {
            let e = &arena.edges[id as usize];
            edges.push(serde_json::json!({"id":id,"source":e.source,"target":e.target,"source_port":e.source_port,"target_port":e.target_port,"reversed":e.reversed}));
        }
    }
    let value = serde_json::json!({"operation":operation,"chosen":chosen,"nodes":nodes,"indegree":indegree,"outdegree":outdegree,"marks":marks,"sources":sources,"sinks":sinks,"edges":edges});
    crate::trace::emit_value(
        "cycle_breaking",
        "p1_cycle_state",
        &format!("graphs[{graph}]"),
        &value.to_string(),
    );
}

#[allow(clippy::too_many_arguments)]
fn update_cycle_neighbors(
    arena: &LayeredArena,
    graph_id: LGraphId,
    node: LNodeId,
    node_index: &BTreeMap<LNodeId, usize>,
    indegree: &mut [usize],
    outdegree: &mut [usize],
    marks: &[isize],
    sources: &mut Vec<LNodeId>,
    sinks: &mut Vec<LNodeId>,
) {
    for (edge_id, edge) in arena.node_connected_edges_source_order(node) {
        if !edge.active {
            continue;
        }
        let weight = arena
            .edge_direction_weight
            .get(&edge_id)
            .copied()
            .unwrap_or(1);
        let source = owner_in_graph(arena, edge.source, graph_id);
        let target = owner_in_graph(arena, edge.target, graph_id);
        if source == Some(node) && target != Some(node) {
            if let Some(&index) = target.as_ref().and_then(|id| node_index.get(id))
                && marks[index] == 0
            {
                indegree[index] = indegree[index].saturating_sub(weight);
                if indegree[index] == 0 && outdegree[index] > 0 {
                    sources.push(target.unwrap());
                }
            }
        } else if target == Some(node)
            && source != Some(node)
            && let Some(&index) = source.as_ref().and_then(|id| node_index.get(id))
            && marks[index] == 0
        {
            outdegree[index] = outdegree[index].saturating_sub(weight);
            if outdegree[index] == 0 && indegree[index] > 0 {
                sinks.push(source.unwrap());
            }
        }
    }
}

/// Assigns nodes to layers using `elk-go`'s `LongestPathLayerer` algorithm.
///
/// This preserves the source implementation's details: traversal starts in
/// `layerless_nodes` order, self-loops are ignored, newly allocated layers are
/// prepended, and the layerless list is cleared after assignment. It is a
/// phase primitive rather than a complete ELK layout.
pub fn assign_longest_path_layers(
    arena: &mut LayeredArena,
    graph_id: LGraphId,
) -> Result<(), LayeringError> {
    if graph_id as usize >= arena.graphs.len() {
        return Err(LayeringError::UnknownGraph(graph_id));
    }
    let nodes = arena.graphs[graph_id as usize].layerless_nodes.clone();
    let mut heights = vec![None::<usize>; nodes.len()];
    for (index, node_id) in nodes.iter().copied().enumerate() {
        arena.nodes[node_id as usize].traversal_index = index;
    }

    fn visit(
        arena: &mut LayeredArena,
        graph_id: LGraphId,
        node_id: LNodeId,
        heights: &mut [Option<usize>],
        visiting: &mut [bool],
    ) -> Result<usize, LayeringError> {
        let index = arena.nodes[node_id as usize].traversal_index;
        if let Some(height) = heights[index] {
            return Ok(height);
        }
        if visiting[index] {
            return Err(LayeringError::Cycle(node_id));
        }
        visiting[index] = true;
        let mut maximum = 1usize;
        let targets = arena
            .edges
            .iter()
            .filter(|edge| {
                edge.active && owner_in_graph(arena, edge.source, graph_id) == Some(node_id)
            })
            .filter_map(|edge| owner_in_graph(arena, edge.target, graph_id))
            .collect::<Vec<_>>();
        for target in targets {
            if target == node_id {
                continue;
            }
            let height = visit(arena, graph_id, target, heights, visiting)? + 1;
            maximum = maximum.max(height);
        }
        while arena.graphs[graph_id as usize].layers.len() < maximum {
            let layer_id = arena.layers.len() as LayerId;
            arena.layers.push(Layer {
                graph: graph_id,
                nodes: Vec::new(),
                height: 0.0,
                width: 0.0,
            });
            let graph = &mut arena.graphs[graph_id as usize];
            graph.layers.insert(0, layer_id);
        }
        let layer_id = arena.graphs[graph_id as usize].layers
            [arena.graphs[graph_id as usize].layers.len() - maximum];
        arena.nodes[node_id as usize].layer = Some(layer_id);
        arena.layers[layer_id as usize].nodes.push(node_id);
        heights[index] = Some(maximum);
        visiting[index] = false;
        Ok(maximum)
    }

    let mut visiting = vec![false; nodes.len()];
    for node_id in nodes.iter().copied() {
        visit(arena, graph_id, node_id, &mut heights, &mut visiting)?;
    }
    arena.graphs[graph_id as usize].layerless_nodes.clear();
    Ok(())
}

/// Assigns the default layered rank strategy used by elk-go's v0.9.0
/// `NetworkSimplexLayerer` for the source graph boundary.
///
/// Runs elk-go's network-simplex rank phase over each connected component.
/// The translated solver keeps its feasible tight tree, cut values, entering
/// and leaving edge exchanges, normalization, and balanced-layer pass. The
/// surrounding arena still owns hierarchy envelopes and layer allocation.
pub fn assign_network_simplex_layers(
    arena: &mut LayeredArena,
    graph_id: LGraphId,
) -> Result<(), LayeringError> {
    assign_layers_with_external_ports(arena, graph_id, false)
}

pub(crate) fn assign_layers_for_layout(
    arena: &mut LayeredArena,
    graph_id: LGraphId,
) -> Result<(), LayeringError> {
    let strategy = arena
        .graphs
        .get(graph_id as usize)
        .ok_or(LayeringError::UnknownGraph(graph_id))?
        .layering_strategy;
    assign_layers_with_external_ports(arena, graph_id, strategy == LayeringStrategy::CoffmanGraham)
}

fn assign_layers_with_external_ports(
    arena: &mut LayeredArena,
    graph_id: LGraphId,
    coffman: bool,
) -> Result<(), LayeringError> {
    if graph_id as usize >= arena.graphs.len() {
        return Err(LayeringError::UnknownGraph(graph_id));
    }
    // ELK's port dummies stay in Graph.LayerlessNodes model order through
    // LayerConstraint preprocessing. Iterating `external_port_origins` here
    // would sort them by arena ID and discard the post-order established by
    // CompoundGraphPreprocessor.reorderCompoundPortDummies.
    let separate_ports = arena.graphs[graph_id as usize]
        .layerless_nodes
        .iter()
        .copied()
        .filter_map(|node_id| {
            let node = arena.nodes.get(node_id as usize)?;
            if node.graph != graph_id
                || node.layer.is_some()
                || !arena.external_port_origins.contains_key(&node_id)
            {
                return None;
            }
            let first_separate = arena
                .external_port_first_separate
                .get(&node_id)
                .copied()??;
            Some((node_id, first_separate))
        })
        .collect::<Vec<_>>();
    if separate_ports.is_empty() {
        assign_rank_core(arena, graph_id, coffman)?;
        move_first_and_last_layer_constraint_nodes(
            arena,
            graph_id,
            &BTreeMap::new(),
            &BTreeSet::new(),
        );
        return Ok(());
    }

    // elk-go hides first/last-separate external-port dummies and their edges
    // while assigning ordinary ranks. It restores each dummy in a boundary
    // layer before node ordering, which also lets LongEdgeSplitter materialize
    // the complete route lane from the real endpoint to the port.
    let separate_nodes = separate_ports
        .iter()
        .map(|(node_id, _)| *node_id)
        .collect::<BTreeSet<_>>();
    let derived_constraints = hidden_node_layer_constraints(arena, graph_id, &separate_ports);
    let detached_edges = arena
        .edges
        .iter_mut()
        .enumerate()
        .filter_map(|(edge_id, edge)| {
            (edge.active
                && (separate_nodes.contains(&edge.source) || separate_nodes.contains(&edge.target)))
            .then(|| {
                edge.active = false;
                edge_id as LEdgeId
            })
        })
        .collect::<Vec<_>>();
    arena.graphs[graph_id as usize]
        .layerless_nodes
        .retain(|node_id| !separate_nodes.contains(node_id));

    let result = assign_rank_core(arena, graph_id, coffman);
    for edge_id in detached_edges {
        arena.edges[edge_id as usize].active = true;
    }
    result?;
    move_first_and_last_layer_constraint_nodes(
        arena,
        graph_id,
        &derived_constraints,
        &separate_nodes,
    );

    let first = separate_ports
        .iter()
        .filter_map(|(node, first)| (*first).then_some(*node))
        .collect::<Vec<_>>();
    let last = separate_ports
        .iter()
        .filter_map(|(node, is_first)| (!*is_first).then_some(*node))
        .collect::<Vec<_>>();
    for (nodes, at_front) in [(first, true), (last, false)] {
        if nodes.is_empty() {
            continue;
        }
        let layer_id = arena.layers.len() as LayerId;
        arena.layers.push(Layer {
            graph: graph_id,
            nodes: nodes.clone(),
            height: 0.0,
            width: 0.0,
        });
        let graph = &mut arena.graphs[graph_id as usize];
        if at_front {
            graph.layers.insert(0, layer_id);
        } else {
            graph.layers.push(layer_id);
        }
        for node_id in nodes {
            arena.nodes[node_id as usize].layer = Some(layer_id);
        }
    }
    Ok(())
}

/// Source LayerConstraintPreprocessor.updateOppositeNodeLayerConstraint
/// (`layer_constraint_preprocessor.go:107–130`). A node that has no edges
/// left once the separate dummies are hidden inherits FIRST or LAST from
/// them; connections to both kinds leave it unconstrained.
fn hidden_node_layer_constraints(
    arena: &LayeredArena,
    graph_id: LGraphId,
    separate_ports: &[(LNodeId, bool)],
) -> BTreeMap<LNodeId, NodeLayerConstraint> {
    let separate = separate_ports.iter().copied().collect::<BTreeMap<_, _>>();
    // (connects to FIRST_SEPARATE, connects to LAST_SEPARATE, keeps an edge)
    let mut connections = BTreeMap::<LNodeId, (bool, bool, bool)>::new();
    for edge in arena.edges.iter().filter(|edge| edge.active) {
        for (node, opposite) in [(edge.source, edge.target), (edge.target, edge.source)] {
            if separate.contains_key(&node) || arena.nodes[node as usize].graph != graph_id {
                continue;
            }
            let entry = connections.entry(node).or_default();
            match separate.get(&opposite) {
                Some(true) => entry.0 = true,
                Some(false) => entry.1 = true,
                None => entry.2 = true,
            }
        }
    }
    connections
        .into_iter()
        .filter_map(|(node, (first, last, visible))| {
            if visible
                || first == last
                || arena.nodes[node as usize].layer_constraint != NodeLayerConstraint::None
            {
                return None;
            }
            Some((
                node,
                if first {
                    NodeLayerConstraint::First
                } else {
                    NodeLayerConstraint::Last
                },
            ))
        })
        .collect()
}

/// Source LayerConstraintPostprocessor (`layer_constraint_postprocessor.go:14–131`),
/// before hidden nodes are restored. SetNodeLayer always re-appends, so a
/// FIRST node already in the first layer still moves behind its neighbors.
fn move_first_and_last_layer_constraint_nodes(
    arena: &mut LayeredArena,
    graph_id: LGraphId,
    derived: &BTreeMap<LNodeId, NodeLayerConstraint>,
    hidden: &BTreeSet<LNodeId>,
) {
    let layers = arena.graphs[graph_id as usize].layers.clone();
    let (Some(&first_layer), Some(&last_layer)) = (layers.first(), layers.last()) else {
        return;
    };
    let mut label_layers: [Vec<LNodeId>; 2] = Default::default();
    let move_node = |arena: &mut LayeredArena, node: LNodeId, layer: Option<LayerId>| {
        if let Some(old) = arena.nodes[node as usize].layer {
            arena.layers[old as usize].nodes.retain(|id| *id != node);
        }
        arena.nodes[node as usize].layer = layer;
        if let Some(layer) = layer {
            arena.layers[layer as usize].nodes.push(node);
        }
    };
    for layer_id in &layers {
        for node in arena.layers[*layer_id as usize].nodes.clone() {
            let constraint = match arena.nodes[node as usize].layer_constraint {
                NodeLayerConstraint::None => derived
                    .get(&node)
                    .copied()
                    .unwrap_or(NodeLayerConstraint::None),
                constraint => constraint,
            };
            let (target, incoming, slot) = match constraint {
                NodeLayerConstraint::First => (first_layer, true, 0),
                NodeLayerConstraint::Last => (last_layer, false, 1),
                _ => continue,
            };
            move_node(arena, node, Some(target));
            // moveAdjacentLabelDummies; hidden edges are still detached in elk-go.
            let labels = arena
                .edges
                .iter()
                .filter(|edge| {
                    edge.active
                        && if incoming {
                            edge.target == node
                        } else {
                            edge.source == node
                        }
                })
                .map(|edge| if incoming { edge.source } else { edge.target })
                .filter(|other| {
                    !hidden.contains(other)
                        && arena.nodes[*other as usize].node_type == LayeredNodeType::Label
                })
                .collect::<Vec<_>>();
            for label in labels {
                move_node(arena, label, None);
                label_layers[slot].retain(|id| *id != label);
                label_layers[slot].push(label);
            }
        }
    }
    let mut kept = layers
        .into_iter()
        .filter(|layer_id| !arena.layers[*layer_id as usize].nodes.is_empty())
        .collect::<Vec<_>>();
    for (slot, nodes) in label_layers.into_iter().enumerate() {
        if nodes.is_empty() {
            continue;
        }
        let layer_id = arena.layers.len() as LayerId;
        for node in &nodes {
            arena.nodes[*node as usize].layer = Some(layer_id);
        }
        arena.layers.push(Layer {
            graph: graph_id,
            nodes,
            height: 0.0,
            width: 0.0,
        });
        if slot == 0 {
            kept.insert(0, layer_id);
        } else {
            kept.push(layer_id);
        }
    }
    arena.graphs[graph_id as usize].layers = kept;
}

fn assign_rank_core(
    arena: &mut LayeredArena,
    graph: LGraphId,
    coffman: bool,
) -> Result<(), LayeringError> {
    if coffman {
        coffman_graham::assign(arena, graph)
    } else {
        assign_network_simplex_layers_core(arena, graph)
    }
}

fn assign_network_simplex_layers_core(
    arena: &mut LayeredArena,
    graph_id: LGraphId,
) -> Result<(), LayeringError> {
    // NetworkSimplexLayerer consumes LayerlessNodes directly. Seeding graph
    // layers with LongestPathLayerer first reverses their allocation order
    // (that processor prepends layers), then makes the same rank indices map
    // onto opposite layer IDs. That changes rank direction for ordinary
    // RIGHT/DOWN layouts even though the solver's node ranks are correct.
    let ordered_nodes = arena.graphs[graph_id as usize].layerless_nodes.clone();
    if ordered_nodes.is_empty() {
        return Ok(());
    }
    let node_set = ordered_nodes.iter().copied().collect::<BTreeSet<_>>();
    let edge_ids = arena
        .edges
        .iter()
        .enumerate()
        .filter_map(|(edge_id, edge)| {
            (edge.active && node_set.contains(&edge.source) && node_set.contains(&edge.target))
                .then_some(edge_id as LEdgeId)
        })
        .collect::<Vec<_>>();
    // Network simplex has no rank constraints when this scope is edge-free.
    // elk-go keeps the isolated children in one model-order layer; assigning
    // each singleton component independently creates artificial inter-layer
    // gaps in otherwise flat compounds (especially visible with outside
    // icon/label margins). Preserve the importer order in the existing first
    // layer and let the ordinary placer apply node-to-node spacing once.
    if edge_ids.is_empty() {
        let layer_id = if let Some(&layer_id) = arena.graphs[graph_id as usize].layers.first() {
            layer_id
        } else {
            let layer_id = arena.layers.len() as LayerId;
            arena.layers.push(Layer {
                graph: graph_id,
                nodes: Vec::new(),
                height: 0.0,
                width: 0.0,
            });
            arena.graphs[graph_id as usize].layers.push(layer_id);
            layer_id
        };
        arena.layers[layer_id as usize].nodes.clear();
        for node_id in ordered_nodes {
            arena.nodes[node_id as usize].layer = Some(layer_id);
            arena.layers[layer_id as usize].nodes.push(node_id);
        }
        return Ok(());
    }
    let components = source_layering_connected_components(arena, &ordered_nodes, &edge_ids);
    let mut previous_layering_node_counts = Vec::new();
    for component in components {
        let component_set = component.iter().copied().collect::<BTreeSet<_>>();
        let component_edges = edge_ids
            .iter()
            .copied()
            .filter(|edge_id| {
                let edge = &arena.edges[*edge_id as usize];
                component_set.contains(&edge.source) && component_set.contains(&edge.target)
            })
            .collect::<Vec<_>>();
        let assignment = network_simplex::layer_component(
            arena,
            &component,
            &component_edges,
            &previous_layering_node_counts,
            arena.graphs[graph_id as usize].thoroughness,
        );
        let required_layers = assignment.values().copied().max().unwrap_or(0) + 1;
        while arena.graphs[graph_id as usize].layers.len() < required_layers {
            let layer_id = arena.layers.len() as LayerId;
            arena.layers.push(Layer {
                graph: graph_id,
                nodes: Vec::new(),
                height: 0.0,
                width: 0.0,
            });
            arena.graphs[graph_id as usize].layers.push(layer_id);
        }
        let graph_layers = arena.graphs[graph_id as usize].layers.clone();
        for node_id in component {
            let target_index = assignment[&node_id];
            let layer_id = graph_layers[target_index];
            arena.nodes[node_id as usize].layer = Some(layer_id);
            arena.layers[layer_id as usize].nodes.push(node_id);
        }
        previous_layering_node_counts = graph_layers
            .iter()
            .map(|layer_id| arena.layers[*layer_id as usize].nodes.len())
            .collect();
    }
    Ok(())
}

/// ELK's P2 connected-component walk is depth-first in node-port-edge order.
/// That traversal order is fed directly to network simplex and later becomes
/// the initial order within equal-rank layers, so a BFS here changes P3 ties.
fn source_layering_connected_components(
    arena: &LayeredArena,
    nodes: &[LNodeId],
    edge_ids: &[LEdgeId],
) -> Vec<Vec<LNodeId>> {
    fn visit(
        arena: &LayeredArena,
        node_id: LNodeId,
        node_set: &BTreeSet<LNodeId>,
        edge_set: &BTreeSet<LEdgeId>,
        visited: &mut BTreeSet<LNodeId>,
        component: &mut Vec<LNodeId>,
    ) {
        if !visited.insert(node_id) {
            return;
        }
        component.push(node_id);
        for (edge_id, edge) in arena.node_connected_edges_source_order(node_id) {
            if !edge_set.contains(&edge_id) {
                continue;
            }
            let other = if edge.source == node_id {
                edge.target
            } else {
                edge.source
            };
            if node_set.contains(&other) && !visited.contains(&other) {
                visit(arena, other, node_set, edge_set, visited, component);
            }
        }
    }

    let node_set = nodes.iter().copied().collect::<BTreeSet<_>>();
    let edge_set = edge_ids.iter().copied().collect::<BTreeSet<_>>();
    let mut visited = BTreeSet::new();
    let mut components = Vec::<Vec<LNodeId>>::new();
    for &node_id in nodes {
        if visited.contains(&node_id) {
            continue;
        }
        let mut component = Vec::new();
        visit(
            arena,
            node_id,
            &node_set,
            &edge_set,
            &mut visited,
            &mut component,
        );
        if components
            .first()
            .is_none_or(|largest| largest.len() < component.len())
        {
            components.insert(0, component);
        } else {
            components.push(component);
        }
    }
    components
}

/// Inserts one `LongEdge` dummy into each intervening layer of every edge.
///
/// This is the source `intermediate/long_edge.go::SplitLongEdges` boundary.
/// It deliberately retains the original edge ID for the incoming segment and
/// appends each outgoing segment, so traces can compare the same allocation
/// order as elk-go before the later join phase restores the original edge.
pub fn split_long_edges(arena: &mut LayeredArena, graph_id: LGraphId) -> Result<(), LayeringError> {
    split_long_edges_impl(arena, graph_id)
}

/// Runs the source-shaped topology-only long-edge splitting stage.
pub(crate) fn split_long_edges_for_layout(
    arena: &mut LayeredArena,
    graph_id: LGraphId,
) -> Result<(), LayeringError> {
    split_long_edges_impl(arena, graph_id)
}

/// Runs the source port-side and inverted-port preprocessors for one scope.
///
/// elk-go fixes sides after layering, when imported and hierarchy-generated
/// edges have their final local incidence. If an external-port dummy marks a
/// compound boundary, its side is inherited by the paired node port. The
/// following inverted-port pass then inserts the same-layer long-edge dummy
/// used for an input on the east side or an output on the west side.
pub(crate) fn process_inverted_ports_for_layout(arena: &mut LayeredArena, graph_id: LGraphId) {
    process_port_sides(arena, graph_id);

    let layer_ids = arena.graphs[graph_id as usize].layers.clone();
    let mut previous_layer = None;
    let mut unassigned = Vec::new();
    for layer_id in &layer_ids {
        for dummy in unassigned.drain(..) {
            assign_inverted_port_dummy(arena, dummy, previous_layer.unwrap_or(*layer_id));
        }
        let nodes = arena.layers[*layer_id as usize].nodes.clone();
        for node_id in nodes {
            if arena.nodes[node_id as usize].node_type != LayeredNodeType::Normal
                || !port_constraints_fix_side(arena.nodes[node_id as usize].port_constraints)
            {
                continue;
            }
            let ports = arena.nodes[node_id as usize].ports.clone();
            for port_id in &ports {
                if arena.ports[*port_id as usize].side != PortSide::East {
                    continue;
                }
                let incoming = arena.ports[*port_id as usize].incoming_edges.clone();
                for edge_id in incoming {
                    if edge_endpoint_node_for_port(arena, edge_id, *port_id, false)
                        .is_some_and(|source| source != node_id)
                    {
                        let dummy = create_inverted_port_dummy(arena, graph_id, edge_id, true);
                        unassigned.push(dummy);
                    }
                }
            }
            for port_id in &ports {
                if arena.ports[*port_id as usize].side != PortSide::West {
                    continue;
                }
                let outgoing = arena.ports[*port_id as usize].outgoing_edges.clone();
                for edge_id in outgoing {
                    if edge_endpoint_node_for_port(arena, edge_id, *port_id, true)
                        .is_some_and(|target| target != node_id)
                    {
                        let dummy = create_inverted_port_dummy(arena, graph_id, edge_id, false);
                        unassigned.push(dummy);
                    }
                }
            }
        }
        previous_layer = Some(*layer_id);
    }
    if let Some(last_layer) = previous_layer {
        for dummy in unassigned {
            assign_inverted_port_dummy(arena, dummy, last_layer);
        }
    }
}

/// Applies the external-port edge constraints from ELK's cycle-breaking
/// preprocessor. East ports accept incoming edges and west ports emit outgoing
/// edges; hierarchy splitting can initially produce the opposite direction.
fn node_layer_constraint(arena: &LayeredArena, node: LNodeId) -> NodeLayerConstraint {
    let constraint = arena.nodes[node as usize].layer_constraint;
    if constraint != NodeLayerConstraint::None {
        return constraint;
    }
    match arena
        .external_port_first_separate
        .get(&node)
        .copied()
        .flatten()
    {
        Some(true) => NodeLayerConstraint::FirstSeparate,
        Some(false) => NodeLayerConstraint::LastSeparate,
        None => NodeLayerConstraint::None,
    }
}

fn reverse_node_constraint_edges(
    arena: &mut LayeredArena,
    node: LNodeId,
    constraint: NodeLayerConstraint,
) {
    use NodeLayerConstraint::{First, FirstSeparate, Last, LastSeparate, None};
    for port in arena.nodes[node as usize].ports.clone() {
        if matches!(constraint, None | Last | LastSeparate) {
            for edge_id in arena.ports[port as usize].outgoing_edges.clone() {
                let edge = &arena.edges[edge_id as usize];
                if !edge.active || edge.reversed {
                    continue;
                }
                let target = edge.target;
                if node_layer_constraint(arena, target) != LastSeparate
                    && !(constraint == Last
                        && arena.nodes[target as usize].node_type == LayeredNodeType::Label)
                {
                    arena.reverse_edge(edge_id);
                }
            }
        }
        if matches!(constraint, None | First | FirstSeparate) {
            for edge_id in arena.ports[port as usize].incoming_edges.clone() {
                let edge = &arena.edges[edge_id as usize];
                if !edge.active || edge.reversed {
                    continue;
                }
                let source = edge.source;
                if node_layer_constraint(arena, source) != FirstSeparate
                    && !(constraint == First
                        && arena.nodes[source as usize].node_type == LayeredNodeType::Label)
                {
                    arena.reverse_edge(edge_id);
                }
            }
        }
    }
}

/// Source EdgeAndLayerConstraintEdgeReverser, before P1. Fixed-side feedback
/// nodes reverse only when every port has opposite net flow; constraint
/// guards and already-reversed edges follow the original traversal order.
pub(crate) fn reverse_constraint_edges_for_layout(arena: &mut LayeredArena, graph_id: LGraphId) {
    use NodeLayerConstraint::{First, FirstSeparate, Last, LastSeparate, None};
    let mut remaining = Vec::new();
    for node in arena.graphs[graph_id as usize].layerless_nodes.clone() {
        let constraint = node_layer_constraint(arena, node);
        if constraint == None {
            remaining.push(node);
        } else {
            reverse_node_constraint_edges(arena, node, constraint);
        }
    }
    for node in remaining {
        let model = &arena.nodes[node as usize];
        if model.ports.is_empty()
            || !matches!(
                model.port_constraints,
                PortConstraints::FixedSide
                    | PortConstraints::FixedOrder
                    | PortConstraints::FixedRatio
                    | PortConstraints::FixedPosition
            )
        {
            continue;
        }
        let all_reversed = model.ports.iter().all(|port| {
            let port = &arena.ports[*port as usize];
            let incoming = port
                .incoming_edges
                .iter()
                .filter(|e| arena.edges[**e as usize].active)
                .count();
            let outgoing = port
                .outgoing_edges
                .iter()
                .filter(|e| arena.edges[**e as usize].active)
                .count();
            ((port.side == PortSide::East && incoming > outgoing)
                || (port.side == PortSide::West && outgoing > incoming))
                && port
                    .outgoing_edges
                    .iter()
                    .filter(|e| arena.edges[**e as usize].active)
                    .all(|e| {
                        !matches!(
                            node_layer_constraint(arena, arena.edges[*e as usize].target),
                            Last | LastSeparate
                        )
                    })
                && port
                    .incoming_edges
                    .iter()
                    .filter(|e| arena.edges[**e as usize].active)
                    .all(|e| {
                        !matches!(
                            node_layer_constraint(arena, arena.edges[*e as usize].source),
                            First | FirstSeparate
                        )
                    })
        });
        if all_reversed {
            reverse_node_constraint_edges(arena, node, None);
        }
    }
}

/// Applies elk-go's `PortListSorter` immediately before node ordering.
///
/// The reverse helper deliberately preserves ELK 0.8.1's exclusive upper
/// bound and short-range behavior. Those details affect implicit edge-port
/// order after cycle breaking, so replacing this with a conventional reverse
/// changes barycenter ranks.
pub fn sort_port_lists_for_layout(
    arena: &mut LayeredArena,
    graph_id: LGraphId,
) -> Result<(), LayeringError> {
    let Some(graph) = arena.graphs.get(graph_id as usize) else {
        return Err(LayeringError::UnknownGraph(graph_id));
    };
    let sort_by_degree = graph.port_sorting_by_degree;
    let mut node_ids = graph.layerless_nodes.clone();
    for layer_id in &graph.layers {
        if let Some(layer) = arena.layers.get(*layer_id as usize) {
            node_ids.extend(layer.nodes.iter().copied());
        }
    }
    let mut seen = BTreeSet::new();
    node_ids.retain(|node_id| seen.insert(*node_id));

    for node_id in node_ids {
        let node = &arena.nodes[node_id as usize];
        let constraints = node.port_constraints;
        if matches!(
            constraints,
            PortConstraints::FixedOrder
                | PortConstraints::FixedRatio
                | PortConstraints::FixedPosition
        ) {
            if node
                .ports
                .iter()
                .any(|port_id| arena.ports[*port_id as usize].side == PortSide::Undefined)
            {
                return Err(LayeringError::PortWithUndefinedSide);
            }
            arena.nodes[node_id as usize]
                .ports
                .sort_by(|first_id, second_id| {
                    let first = &arena.ports[*first_id as usize];
                    let second = &arena.ports[*second_id as usize];
                    port_side_order(first.side)
                        .cmp(&port_side_order(second.side))
                        .then_with(|| {
                            let (first_position, second_position) = match first.side {
                                PortSide::North => (first.x, second.x),
                                PortSide::East => (first.y, second.y),
                                PortSide::South => (-first.x, -second.x),
                                PortSide::West => (-first.y, -second.y),
                                PortSide::Undefined => unreachable!("checked above"),
                            };
                            first_position
                                .partial_cmp(&second_position)
                                .unwrap_or(Ordering::Equal)
                        })
                });
        } else if constraints == PortConstraints::FixedSide {
            let mut ports = arena.nodes[node_id as usize].ports.clone();
            ports.sort_by_key(|port_id| port_side_order(arena.ports[*port_id as usize].side));
            reverse_port_side(arena, &mut ports, PortSide::South);
            reverse_port_side(arena, &mut ports, PortSide::West);
            if sort_by_degree {
                let mut start = 0;
                while start < ports.len() {
                    let side = arena.ports[ports[start] as usize].side;
                    let mut end = start + 1;
                    while end < ports.len() && arena.ports[ports[end] as usize].side == side {
                        end += 1;
                    }
                    if matches!(side, PortSide::East | PortSide::West) {
                        ports[start..end].sort_by(|first_id, second_id| {
                            let incoming = side == PortSide::West;
                            let first_degree = real_port_degree(arena, *first_id, incoming);
                            let second_degree = real_port_degree(arena, *second_id, incoming);
                            if side == PortSide::East {
                                second_degree.cmp(&first_degree)
                            } else {
                                first_degree.cmp(&second_degree)
                            }
                        });
                    }
                    start = end;
                }
            }
            arena.nodes[node_id as usize].ports = ports;
        }
    }
    Ok(())
}

fn reverse_port_side(arena: &LayeredArena, ports: &mut [LPortId], side: PortSide) {
    let (low, high) = find_port_side_range(arena, ports, side);
    if high <= low + 2 {
        return;
    }
    for index in 0..(high - low) / 2 {
        ports.swap(low + index, high - index - 1);
    }
}

fn find_port_side_range(arena: &LayeredArena, ports: &[LPortId], side: PortSide) -> (usize, usize) {
    if ports.is_empty() {
        return (0, 0);
    }
    let mut current_side = arena.ports[ports[0] as usize].side;
    let mut low = 0;
    while low < ports.len() - 1 && port_side_order(current_side) < port_side_order(side) {
        low += 1;
        current_side = arena.ports[ports[low] as usize].side;
    }
    let mut high = low;
    while high < ports.len() - 1 && port_side_order(current_side) < port_side_order(side) + 1 {
        high += 1;
        // This intentionally mirrors ELK 0.8.1: it reads the low port while
        // advancing the exclusive high bound instead of reading `ports[high]`.
        current_side = arena.ports[ports[low] as usize].side;
    }
    (low, high)
}

fn real_port_degree(arena: &LayeredArena, port_id: LPortId, incoming: bool) -> usize {
    let edges = if incoming {
        &arena.ports[port_id as usize].incoming_edges
    } else {
        &arena.ports[port_id as usize].outgoing_edges
    };
    edges
        .iter()
        .filter(|edge_id| !arena.edges[**edge_id as usize].reversed)
        .count()
}

fn process_port_sides(arena: &mut LayeredArena, graph_id: LGraphId) {
    let mut node_ids = arena.graphs[graph_id as usize].layerless_nodes.clone();
    for layer_id in arena.graphs[graph_id as usize].layers.clone() {
        node_ids.extend(arena.layers[layer_id as usize].nodes.iter().copied());
    }
    for node_id in node_ids {
        if arena.nodes[node_id as usize].node_type != LayeredNodeType::Normal {
            continue;
        }
        let constraints = arena.nodes[node_id as usize].port_constraints;
        let port_ids = arena.nodes[node_id as usize].ports.clone();
        for port_id in port_ids {
            let only_self_loops = arena.ports[port_id as usize]
                .incoming_edges
                .iter()
                .chain(&arena.ports[port_id as usize].outgoing_edges)
                .filter(|edge_id| arena.edges[**edge_id as usize].active)
                .all(|edge_id| {
                    let edge = &arena.edges[*edge_id as usize];
                    edge.source == node_id && edge.target == node_id
                });
            if only_self_loops
                && arena.ports[port_id as usize].side != PortSide::Undefined
                && arena.ports[port_id as usize]
                    .incoming_edges
                    .iter()
                    .chain(&arena.ports[port_id as usize].outgoing_edges)
                    .any(|edge_id| arena.edges[*edge_id as usize].active)
            {
                // elk-go detaches pure self-loop ports before this processor
                // assigns ordinary edge sides, then restores their saved loop
                // side before placement. Retain the side assigned during the
                // corresponding import/preprocessing step.
                continue;
            }
            if port_constraints_fix_side(constraints)
                && arena.ports[port_id as usize].side != PortSide::Undefined
            {
                continue;
            }
            arena.ports[port_id as usize].side = port_side_for_layout(arena, port_id);
        }
        if !port_constraints_fix_side(constraints) {
            arena.nodes[node_id as usize].port_constraints = PortConstraints::FixedSide;
        }
    }
}

fn port_side_for_layout(arena: &LayeredArena, port_id: LPortId) -> PortSide {
    // elk-go keeps the association in the opposite direction: PortDummy on
    // the original parent port points at its ExternalPort node, whose side is
    // authoritative even though the local hierarchy edge is attached to a
    // child-side dummy. The arena records dummy -> origin, so restore the same
    // side from that association before inferring direction from local edges.
    if let Some(side) = arena
        .external_port_origins
        .iter()
        .find_map(|(dummy, origin)| {
            (*origin == port_id).then(|| match arena.external_port_first_separate.get(dummy) {
                Some(Some(true)) => PortSide::West,
                Some(Some(false)) => PortSide::East,
                _ => PortSide::Undefined,
            })
        })
        && side != PortSide::Undefined
    {
        return side;
    }

    let mut incoming = 0usize;
    let mut outgoing = 0usize;
    let mut external_side = None;
    for edge in &arena.edges {
        if !edge.active {
            continue;
        }
        if edge.source_port == Some(port_id) {
            outgoing += 1;
            if arena.nodes[edge.target as usize].node_type == LayeredNodeType::ExternalPort {
                external_side =
                    arena.nodes[edge.target as usize]
                        .ports
                        .first()
                        .map(|external_port| {
                            opposite_port_side(arena.ports[*external_port as usize].side)
                        });
            }
        }
        if edge.target_port == Some(port_id) {
            incoming += 1;
            if arena.nodes[edge.source as usize].node_type == LayeredNodeType::ExternalPort {
                external_side =
                    arena.nodes[edge.source as usize]
                        .ports
                        .first()
                        .map(|external_port| {
                            opposite_port_side(arena.ports[*external_port as usize].side)
                        });
            }
        }
    }
    external_side.unwrap_or(if outgoing > incoming {
        PortSide::East
    } else {
        PortSide::West
    })
}

fn edge_endpoint_node_for_port(
    arena: &LayeredArena,
    edge_id: LEdgeId,
    port_id: LPortId,
    source_port: bool,
) -> Option<LNodeId> {
    let edge = arena.edges.get(edge_id as usize)?;
    if !edge.active {
        return None;
    }
    if source_port && edge.source_port == Some(port_id) {
        Some(edge.target)
    } else if !source_port && edge.target_port == Some(port_id) {
        Some(edge.source)
    } else {
        None
    }
}

fn create_inverted_port_dummy(
    arena: &mut LayeredArena,
    graph_id: LGraphId,
    edge_id: LEdgeId,
    east_input: bool,
) -> LNodeId {
    let original = arena.edges[edge_id as usize].clone();
    let node_id = arena.nodes.len() as LNodeId;
    let traversal_index = arena.graphs[graph_id as usize].layerless_nodes.len();
    arena.nodes.push(LNode {
        layer_constraint: NodeLayerConstraint::None,
        alignment: NodeAlignment::Automatic,
        graph: graph_id,
        traversal_index,
        layer: None,
        source_id: format!("__inverted_port_dummy_{node_id}"),
        node_type: LayeredNodeType::LongEdge,
        nested_graph: None,
        ports: Vec::new(),
        port_constraints: PortConstraints::FixedPosition,
        self_loop_distribution: SelfLoopDistribution::North,
        width: 0.0,
        height: 0.0,
        fixed_width: false,
        fixed_height: false,
        margin_top: 0.0,
        margin_right: 0.0,
        margin_bottom: 0.0,
        margin_left: 0.0,
        margins_in_wire_dimensions: false,
        computed_margins: [0.0; 4],
        label_margin_top: 0.0,
        label_margin_right: 0.0,
        label_margin_bottom: 0.0,
        label_margin_left: 0.0,
        position_x: 0.0,
        position_y: 0.0,
    });
    arena.graphs[graph_id as usize]
        .layerless_nodes
        .push(node_id);
    let input_port = arena.add_dummy_port(node_id, PortSide::West);
    let output_port = arena.add_dummy_port(node_id, PortSide::East);

    if east_input {
        if let Some(target_port) = original.target_port {
            arena.ports[target_port as usize]
                .incoming_edges
                .retain(|candidate| *candidate != edge_id);
        }
        arena.edges[edge_id as usize].target = node_id;
        arena.edges[edge_id as usize].target_port = Some(input_port);
        arena.ports[input_port as usize]
            .incoming_edges
            .push(edge_id);
        let duplicate = arena.add_edge(
            graph_id,
            EndpointRef {
                node: node_id,
                port: Some(output_port),
            },
            EndpointRef {
                node: original.target,
                port: original.target_port,
            },
        );
        copy_edge_phase_weights(arena, edge_id, duplicate);
        arena.edges[duplicate as usize].reversed = original.reversed;
    } else {
        if let Some(target_port) = original.target_port {
            arena.ports[target_port as usize]
                .incoming_edges
                .retain(|candidate| *candidate != edge_id);
        }
        arena.edges[edge_id as usize].target = node_id;
        arena.edges[edge_id as usize].target_port = Some(input_port);
        arena.ports[input_port as usize]
            .incoming_edges
            .push(edge_id);
        let duplicate = arena.add_edge(
            graph_id,
            EndpointRef {
                node: node_id,
                port: Some(output_port),
            },
            EndpointRef {
                node: original.target,
                port: original.target_port,
            },
        );
        copy_edge_phase_weights(arena, edge_id, duplicate);
        arena.edges[duplicate as usize].reversed = original.reversed;
    }
    let source = arena.ports[input_port as usize]
        .incoming_edges
        .first()
        .and_then(|edge| arena.edges[*edge as usize].source_port);
    let target = arena.ports[output_port as usize]
        .outgoing_edges
        .first()
        .and_then(|edge| arena.edges[*edge as usize].target_port);
    let source = source.and_then(|port| {
        let node = arena.ports[port as usize].node;
        if arena.nodes[node as usize].node_type == LayeredNodeType::LongEdge {
            arena
                .long_edge_merge_ends
                .get(&node)
                .map(|ends| ends.0)
                .unwrap_or_else(|| arena.long_edge_endpoints.get(&node).map(|ends| ends.0))
        } else {
            Some(port)
        }
    });
    let target = target.and_then(|port| {
        let node = arena.ports[port as usize].node;
        if arena.nodes[node as usize].node_type == LayeredNodeType::LongEdge {
            arena
                .long_edge_merge_ends
                .get(&node)
                .map(|ends| ends.1)
                .unwrap_or_else(|| arena.long_edge_endpoints.get(&node).map(|ends| ends.1))
        } else {
            Some(port)
        }
    });
    arena.long_edge_merge_ends.insert(node_id, (source, target));
    node_id
}

fn copy_edge_phase_weights(arena: &mut LayeredArena, source: LEdgeId, target: LEdgeId) {
    if let Some(order) = arena.edge_model_order.get(&source).copied() {
        arena.edge_model_order.insert(target, order);
    }
    if let Some(priority) = arena.edge_priority_shortness.get(&source).copied() {
        arena.edge_priority_shortness.insert(target, priority);
    }
    if let Some(priority) = arena.edge_direction_weight.get(&source).copied() {
        arena.edge_direction_weight.insert(target, priority);
    }
    if let Some(thickness) = arena.edge_thickness.get(&source).copied() {
        arena.edge_thickness.insert(target, thickness);
    }
}

fn assign_inverted_port_dummy(arena: &mut LayeredArena, node_id: LNodeId, layer_id: LayerId) {
    arena.graphs[arena.layers[layer_id as usize].graph as usize]
        .layerless_nodes
        .retain(|candidate| *candidate != node_id);
    arena.nodes[node_id as usize].layer = Some(layer_id);
    arena.nodes[node_id as usize].traversal_index = arena.layers[layer_id as usize].nodes.len();
    arena.layers[layer_id as usize].nodes.push(node_id);
}

fn split_long_edges_impl(
    arena: &mut LayeredArena,
    graph_id: LGraphId,
) -> Result<(), LayeringError> {
    let Some(graph) = arena.graphs.get(graph_id as usize) else {
        return Err(LayeringError::UnknownGraph(graph_id));
    };
    if graph.layers.len() <= 2 {
        return Ok(());
    }
    let layer_ids = graph.layers.clone();
    for layer_index in 0..layer_ids.len() - 1 {
        let layer_id = layer_ids[layer_index];
        let next_layer_id = layer_ids[layer_index + 1];
        let nodes = arena.layers[layer_id as usize].nodes.clone();
        for node_id in nodes {
            let edge_ids = arena
                .outgoing_edges(node_id)
                .map(|(edge_id, _)| edge_id)
                .collect::<Vec<_>>();
            for edge_id in edge_ids {
                let (target, target_layer) = {
                    let edge = &arena.edges[edge_id as usize];
                    (edge.target, arena.nodes[edge.target as usize].layer)
                };
                let Some(target_layer) = target_layer else {
                    continue;
                };
                if target_layer == layer_id || target_layer == next_layer_id {
                    continue;
                }
                let dummy = arena.add_long_edge_dummy(graph_id, next_layer_id, edge_id);
                arena.split_edge_at_dummy(edge_id, dummy, target)?;
            }
        }
    }
    Ok(())
}

/// Matches the port-position part of elk-go's post-ordering
/// `ProcessLabelAndNodeSizes` stage. The barycenter distributor has already
/// established `node.ports` order; ELK redistributes every non-fixed port on
/// each side and then places it on the node border, including hierarchy ports
/// with a non-zero border offset.
pub(crate) fn redistribute_ordered_implicit_ports(arena: &mut LayeredArena, graph_id: LGraphId) {
    let node_ids = arena
        .nodes
        .iter()
        .enumerate()
        .filter_map(|(node_id, node)| {
            (node.graph == graph_id && node.node_type == LayeredNodeType::Normal)
                .then_some(node_id as LNodeId)
        })
        .collect::<Vec<_>>();
    for node_id in node_ids {
        let node = &arena.nodes[node_id as usize];
        if matches!(
            node.port_constraints,
            PortConstraints::FixedRatio | PortConstraints::FixedPosition
        ) {
            // Source placePortOnBorder preserves only the along-side
            // coordinate for FIXED_RATIO/FIXED_POS, including after a proxy
            // crosses its owner and SweepCopy changes that port's side.
            let width = node.width;
            let height = node.height;
            let ports = node.ports.clone();
            for port_id in ports {
                let offset = arena
                    .port_border_offsets
                    .get(&port_id)
                    .copied()
                    .unwrap_or(0.0);
                let port = &mut arena.ports[port_id as usize];
                match port.side {
                    PortSide::North => port.y = -port.height - offset,
                    PortSide::East => port.x = width + offset,
                    PortSide::South => port.y = height + offset,
                    PortSide::West => port.x = -port.width - offset,
                    PortSide::Undefined => {}
                }
            }
            continue;
        }
        let port_order = node.ports.clone();
        let width = arena.nodes[node_id as usize].width;
        let height = arena.nodes[node_id as usize].height;
        for side in [
            PortSide::North,
            PortSide::East,
            PortSide::South,
            PortSide::West,
        ] {
            let ports = port_order
                .iter()
                .copied()
                .filter(|port_id| arena.ports[*port_id as usize].side == side)
                .collect::<Vec<_>>();
            if ports.is_empty() {
                continue;
            }
            let mut ports = ports;
            if matches!(side, PortSide::South | PortSide::West) {
                ports.reverse();
            }
            let horizontal = matches!(side, PortSide::North | PortSide::South);
            let extent = if horizontal { width } else { height };
            let count = ports.len();
            let occupied = ports
                .iter()
                .map(|port_id| {
                    let port = &arena.ports[*port_id as usize];
                    if horizontal { port.width } else { port.height }
                })
                .sum::<f64>();
            let mut gap = 10.0;
            let placement_size = occupied + gap * (count.saturating_sub(1) as f64);
            let mut position = if count == 1 {
                (extent - occupied) / 2.0
            } else {
                // Keep ELK's DISTRIBUTED arithmetic order to preserve exact
                // IEEE-754 coordinates in port and node margins.
                gap += (extent - (occupied + gap * (count as f64 + 1.0))) / (count as f64 + 1.0);
                let _ = placement_size;
                gap
            };
            for port_id in ports {
                let port = &mut arena.ports[port_id as usize];
                match side {
                    PortSide::North => {
                        port.x = position;
                        port.y = -port.height
                            - arena
                                .port_border_offsets
                                .get(&port_id)
                                .copied()
                                .unwrap_or(0.0);
                        position += port.width + gap;
                    }
                    PortSide::East => {
                        port.x = width
                            + arena
                                .port_border_offsets
                                .get(&port_id)
                                .copied()
                                .unwrap_or(0.0);
                        port.y = position;
                        position += port.height + gap;
                    }
                    PortSide::South => {
                        port.x = position;
                        port.y = height
                            + arena
                                .port_border_offsets
                                .get(&port_id)
                                .copied()
                                .unwrap_or(0.0);
                        position += port.width + gap;
                    }
                    PortSide::West => {
                        port.x = -port.width
                            - arena
                                .port_border_offsets
                                .get(&port_id)
                                .copied()
                                .unwrap_or(0.0);
                        port.y = position;
                        position += port.height + gap;
                    }
                    PortSide::Undefined => {}
                }
            }
        }
        arena.nodes[node_id as usize].port_constraints = PortConstraints::FixedPosition;
    }
}

/// Inserts the fixed north/south-port dummies used by elk-go before its node
/// ordering phase. A side-fixed node's connected cross-axis ports otherwise
/// have no representation in the rank ordering, which changes both crossing
/// minimization and the orthogonal route's endpoint lane.
pub fn preprocess_north_south_ports(arena: &mut LayeredArena, graph_id: LGraphId) {
    let layer_ids = arena.graphs[graph_id as usize].layers.clone();
    for layer_id in layer_ids {
        let node_snapshot = arena.layers[layer_id as usize].nodes.clone();
        for node_id in node_snapshot {
            let node = &arena.nodes[node_id as usize];
            if node.graph != graph_id
                || node.node_type != LayeredNodeType::Normal
                || !matches!(
                    node.port_constraints,
                    PortConstraints::FixedSide
                        | PortConstraints::FixedOrder
                        | PortConstraints::FixedRatio
                        | PortConstraints::FixedPosition
                )
            {
                continue;
            }
            // NorthSouthPortPreprocessor processes north first, then the
            // reversed south list. Each side creates input-only, output-only,
            // then bidirectional dummies, rather than walking node.Ports.
            let model_order = arena.graphs[graph_id as usize].consider_model_order;
            let mut ports = Vec::new();
            for side in [PortSide::North, PortSide::South] {
                let mut side_ports = node
                    .ports
                    .iter()
                    .copied()
                    .filter(|id| arena.ports[*id as usize].side == side)
                    .collect::<Vec<_>>();
                if side == PortSide::South {
                    side_ports.reverse();
                }
                if model_order {
                    let (incoming, mut outgoing): (Vec<_>, Vec<_>) = side_ports
                        .into_iter()
                        .partition(|id| !arena.ports[*id as usize].incoming_edges.is_empty());
                    outgoing.reverse();
                    side_ports = outgoing.into_iter().chain(incoming).collect();
                }
                for kind in [0, 1, 2] {
                    ports.extend(side_ports.iter().copied().filter(|id| {
                        let port = &arena.ports[*id as usize];
                        match (
                            !port.incoming_edges.is_empty(),
                            !port.outgoing_edges.is_empty(),
                        ) {
                            (true, false) => kind == 0,
                            (false, true) => kind == 1,
                            (true, true) => kind == 2,
                            _ => false,
                        }
                    }));
                }
            }
            let mut north_count = 0;
            let mut south_count = 0;
            for origin_port in ports {
                let port = &arena.ports[origin_port as usize];
                if !matches!(port.side, PortSide::North | PortSide::South) {
                    continue;
                }
                let incoming = port
                    .incoming_edges
                    .iter()
                    .copied()
                    .filter(|edge| {
                        let edge = &arena.edges[*edge as usize];
                        edge.active && !(edge.source == node_id && edge.target == node_id)
                    })
                    .collect::<Vec<_>>();
                let outgoing = port
                    .outgoing_edges
                    .iter()
                    .copied()
                    .filter(|edge| {
                        let edge = &arena.edges[*edge as usize];
                        edge.active && !(edge.source == node_id && edge.target == node_id)
                    })
                    .collect::<Vec<_>>();
                if incoming.is_empty() && outgoing.is_empty() {
                    continue;
                }

                let dummy_id = arena.nodes.len() as LNodeId;
                let traversal_index = arena.layers[layer_id as usize].nodes.len();
                arena.nodes.push(LNode {
                    layer_constraint: NodeLayerConstraint::None,
                    alignment: NodeAlignment::Automatic,
                    graph: graph_id,
                    traversal_index,
                    layer: Some(layer_id),
                    source_id: format!("__north_south_port_dummy_{origin_port}"),
                    node_type: LayeredNodeType::NorthSouthPort,
                    nested_graph: None,
                    ports: Vec::new(),
                    port_constraints: PortConstraints::FixedPosition,
                    self_loop_distribution: SelfLoopDistribution::North,
                    width: 0.0,
                    height: 0.0,
                    fixed_width: false,
                    fixed_height: false,
                    margin_top: 0.0,
                    margin_right: 0.0,
                    margin_bottom: 0.0,
                    margin_left: 0.0,
                    margins_in_wire_dimensions: false,
                    computed_margins: [0.0; 4],
                    label_margin_top: 0.0,
                    label_margin_right: 0.0,
                    label_margin_bottom: 0.0,
                    label_margin_left: 0.0,
                    position_x: 0.0,
                    position_y: 0.0,
                });
                let input_port =
                    (!incoming.is_empty()).then(|| arena.add_dummy_port(dummy_id, PortSide::West));
                let output_port =
                    (!outgoing.is_empty()).then(|| arena.add_dummy_port(dummy_id, PortSide::East));

                if let Some(dummy_port) = input_port {
                    for edge_id in incoming {
                        rewire_edge_endpoint(arena, edge_id, false, dummy_id, dummy_port);
                    }
                }
                if let Some(dummy_port) = output_port {
                    for edge_id in outgoing {
                        rewire_edge_endpoint(arena, edge_id, true, dummy_id, dummy_port);
                    }
                }

                // Go repeatedly inserts north proxies at a fixed insertion
                // point, while advancing the south insertion pointer.
                let nodes = &mut arena.layers[layer_id as usize].nodes;
                let owner_index = nodes
                    .iter()
                    .position(|candidate| *candidate == node_id)
                    .expect("normal node remains in its assigned layer");
                let insert_at = if arena.ports[origin_port as usize].side == PortSide::North {
                    let at = owner_index - north_count;
                    north_count += 1;
                    at
                } else {
                    south_count += 1;
                    owner_index + south_count
                };
                nodes.insert(insert_at, dummy_id);
                arena.north_south_port_dummies.push(NorthSouthPortDummy {
                    graph: graph_id,
                    node: dummy_id,
                    input_port,
                    output_port,
                    origin_port,
                });
            }
        }
    }
}

/// Restores original port endpoints after routing and transfers each dummy's
/// cross-axis coordinate into the route as the orthogonal port-exit bend.
pub fn postprocess_north_south_ports(arena: &mut LayeredArena, graph_id: LGraphId) {
    let dummy_indices = arena
        .north_south_port_dummies
        .iter()
        .enumerate()
        .filter_map(|(index, dummy)| (dummy.graph == graph_id).then_some(index))
        .collect::<Vec<_>>();
    for dummy_index in dummy_indices {
        let dummy = arena.north_south_port_dummies[dummy_index].clone();
        let origin = dummy.origin_port;
        let owner = arena.ports[origin as usize].node;
        let point = Point {
            x: absolute_port_anchor(arena, origin).x,
            y: arena.nodes[dummy.node as usize].position_y,
        };
        let add_junction = dummy.input_port.is_some() && dummy.output_port.is_some();

        if let Some(input_port) = dummy.input_port {
            let incoming = arena.ports[input_port as usize].incoming_edges.clone();
            for edge_id in incoming {
                rewire_edge_endpoint(arena, edge_id, false, owner, origin);
                #[cfg(feature = "diagnostic-traces")]
                trace_routing_bend(
                    arena,
                    graph_id,
                    "north_south_port_postprocessor",
                    edge_id,
                    point,
                    "back",
                );
                arena.edges[edge_id as usize].bend_points.push(point);
                if add_junction {
                    arena.edges[edge_id as usize].junction_points.push(point);
                }
            }
        }
        if let Some(output_port) = dummy.output_port {
            let outgoing = arena.ports[output_port as usize].outgoing_edges.clone();
            for edge_id in outgoing {
                rewire_edge_endpoint(arena, edge_id, true, owner, origin);
                #[cfg(feature = "diagnostic-traces")]
                trace_routing_bend(
                    arena,
                    graph_id,
                    "north_south_port_postprocessor",
                    edge_id,
                    point,
                    "front",
                );
                arena.edges[edge_id as usize].bend_points.insert(0, point);
                if add_junction {
                    arena.edges[edge_id as usize].junction_points.push(point);
                }
            }
        }

        if let Some(layer_id) = arena.nodes[dummy.node as usize].layer.take() {
            arena.layers[layer_id as usize]
                .nodes
                .retain(|node| *node != dummy.node);
        }
    }
}

fn rewire_edge_endpoint(
    arena: &mut LayeredArena,
    edge_id: LEdgeId,
    source: bool,
    node_id: LNodeId,
    port_id: LPortId,
) {
    let previous_port = if source {
        arena.edges[edge_id as usize].source_port
    } else {
        arena.edges[edge_id as usize].target_port
    };
    if let Some(previous_port) = previous_port {
        let incidents = if source {
            &mut arena.ports[previous_port as usize].outgoing_edges
        } else {
            &mut arena.ports[previous_port as usize].incoming_edges
        };
        incidents.retain(|candidate| *candidate != edge_id);
    }
    if source {
        arena.edges[edge_id as usize].source = node_id;
        arena.edges[edge_id as usize].source_port = Some(port_id);
        if !arena.ports[port_id as usize]
            .outgoing_edges
            .contains(&edge_id)
        {
            arena.ports[port_id as usize].outgoing_edges.push(edge_id);
        }
    } else {
        arena.edges[edge_id as usize].target = node_id;
        arena.edges[edge_id as usize].target_port = Some(port_id);
        if !arena.ports[port_id as usize]
            .incoming_edges
            .contains(&edge_id)
        {
            arena.ports[port_id as usize].incoming_edges.push(edge_id);
        }
    }
}

/// Restores cycle-breaking directions after routing, in source layer/port order.
/// Outgoing adjacency is copied because reversal mutates both endpoint lists.
pub(crate) fn restore_reversed_edges_for_layout(arena: &mut LayeredArena, graph_id: LGraphId) {
    let layers = arena.graphs[graph_id as usize].layers.clone();
    for layer in layers {
        for node in arena.layers[layer as usize].nodes.clone() {
            for port in arena.nodes[node as usize].ports.clone() {
                for edge in arena.ports[port as usize].outgoing_edges.clone() {
                    if arena.edges[edge as usize].active && arena.edges[edge as usize].reversed {
                        arena.reverse_edge(edge);
                    }
                }
            }
        }
    }
}

/// Coalesces adjacent long-edge proxies belonging to one orthogonal hyperedge.
pub(crate) fn merge_hyperedge_dummies(
    arena: &mut LayeredArena,
    graph: LGraphId,
) -> Result<(), LayeringError> {
    #[cfg(feature = "diagnostic-traces")]
    crate::trace::emit_value(
        "hierarchy",
        "hyperedge_merger_configuration",
        &format!("graphs[{graph}]"),
        &arena.graphs[graph as usize].has_hyperedges.to_string(),
    );
    if !arena.graphs[graph as usize].has_hyperedges {
        return Ok(());
    }
    hyperedge_merger::merge(arena, graph)
}

/// Joins the active segments created by [`split_long_edges`].
///
/// This mirrors `intermediate/long_edge.go::JoinLongEdges` from elk-go.  The
/// incoming segment retains its arena ID; each outgoing synthetic segment is
/// marked inactive and its target is attached to that incoming edge.  Arena
/// slots are deliberately retained so diagnostic traces can compare the
/// allocation order before and after the post-processing phase.
pub fn join_long_edges(arena: &mut LayeredArena, graph_id: LGraphId) -> Result<(), LayeringError> {
    let Some(graph) = arena.graphs.get(graph_id as usize) else {
        return Err(LayeringError::UnknownGraph(graph_id));
    };
    let layer_ids = graph.layers.clone();
    for layer_id in layer_ids {
        let nodes = arena.layers[layer_id as usize].nodes.clone();
        for dummy in nodes {
            if arena.nodes[dummy as usize].node_type != LayeredNodeType::LongEdge {
                continue;
            }
            join_edge_segments_at_dummy(arena, dummy)?;
            arena.layers[layer_id as usize]
                .nodes
                .retain(|node| *node != dummy);
        }
    }
    Ok(())
}

/// Removes center-label proxy nodes after routing and rejoins their two edge
/// segments, matching elk-go's `LabelDummyRemover` and `JoinLongEdgeAt` order.
pub fn remove_center_edge_label_dummies(
    arena: &mut LayeredArena,
    graph_id: LGraphId,
) -> Result<(), LayeringError> {
    let Some(graph) = arena.graphs.get(graph_id as usize) else {
        return Err(LayeringError::UnknownGraph(graph_id));
    };
    let layer_ids = graph.layers.clone();
    let label_nodes = arena
        .edge_label_dummies
        .iter()
        .filter_map(|label| {
            (arena.nodes[label.node as usize].graph == graph_id).then_some(label.node)
        })
        .collect::<Vec<_>>();
    for dummy in label_nodes {
        let Some(layer_id) = arena.nodes[dummy as usize].layer else {
            continue;
        };
        if !layer_ids.contains(&layer_id) {
            continue;
        }
        join_edge_segments_at_dummy(arena, dummy)?;
        arena.layers[layer_id as usize]
            .nodes
            .retain(|node| *node != dummy);
    }
    Ok(())
}

/// Joins all matching west-incoming/east-outgoing pairs through one dummy.
/// The target's incoming-edge slot is retained, as in elk-go's
/// `SetEdgeTargetAt`, so later source-order traversals see the same adjacency.
fn join_edge_segments_at_dummy(
    arena: &mut LayeredArena,
    dummy: LNodeId,
) -> Result<(), LayeringError> {
    let west = arena.nodes[dummy as usize]
        .ports
        .iter()
        .copied()
        .find(|port| arena.ports[*port as usize].side == PortSide::West);
    let east = arena.nodes[dummy as usize]
        .ports
        .iter()
        .copied()
        .find(|port| arena.ports[*port as usize].side == PortSide::East);
    let (Some(west), Some(east)) = (west, east) else {
        return Err(LayeringError::UnknownEdge(dummy as LEdgeId));
    };
    let incoming = arena.ports[west as usize].incoming_edges.clone();
    let outgoing = arena.ports[east as usize].outgoing_edges.clone();
    if outgoing.len() < incoming.len() {
        return Err(LayeringError::UnknownEdge(
            incoming.get(outgoing.len()).copied().unwrap_or_default(),
        ));
    }
    for (surviving, dropped) in incoming.into_iter().zip(outgoing) {
        if !arena.edges[surviving as usize].active || !arena.edges[dropped as usize].active {
            continue;
        }
        let (target, target_port) = {
            let edge = &arena.edges[dropped as usize];
            (edge.target, edge.target_port)
        };
        let target_index = target_port.and_then(|port| {
            arena.ports[port as usize]
                .incoming_edges
                .iter()
                .position(|edge| *edge == dropped)
        });
        // SetEdgeTargetAt and clearing the dropped edge also detach both
        // old proxy adjacencies. Retired proxies must not keep live edges.
        arena.ports[west as usize]
            .incoming_edges
            .retain(|edge| *edge != surviving);
        arena.ports[east as usize]
            .outgoing_edges
            .retain(|edge| *edge != dropped);
        let dropped_bends = std::mem::take(&mut arena.edges[dropped as usize].bend_points);
        let dropped_junctions = std::mem::take(&mut arena.edges[dropped as usize].junction_points);
        if let Some(port) = target_port {
            let incoming_edges = &mut arena.ports[port as usize].incoming_edges;
            incoming_edges.retain(|edge| *edge != dropped);
            if !incoming_edges.contains(&surviving) {
                incoming_edges.insert(
                    target_index
                        .unwrap_or(incoming_edges.len())
                        .min(incoming_edges.len()),
                    surviving,
                );
            }
        }
        arena.edges[surviving as usize].target = target;
        arena.edges[surviving as usize].target_port = target_port;
        arena.edges[surviving as usize]
            .bend_points
            .extend(dropped_bends);
        arena.edges[surviving as usize]
            .junction_points
            .extend(dropped_junctions);
        arena.edges[dropped as usize].active = false;
        arena.edges[dropped as usize].source_port = None;
        arena.edges[dropped as usize].target_port = None;
    }
    Ok(())
}

#[derive(Clone, Debug)]
struct CenterLabelDummyInfo {
    label: LNodeId,
    left_long_edges: Vec<LNodeId>,
    right_long_edges: Vec<LNodeId>,
    leftmost_layer: usize,
    rightmost_layer: usize,
}

impl CenterLabelDummyInfo {
    fn total_dummy_count(&self) -> usize {
        self.rightmost_layer - self.leftmost_layer + 1
    }

    fn dummy_at(&self, offset: usize) -> Option<LNodeId> {
        if offset < self.left_long_edges.len() {
            return self.left_long_edges.get(offset).copied();
        }
        if offset == self.left_long_edges.len() {
            return Some(self.label);
        }
        self.right_long_edges
            .get(offset - self.left_long_edges.len() - 1)
            .copied()
    }
}

/// Moves each center-label proxy to elk-go's default median long-edge layer.
///
/// The proxy is introduced before network-simplex layering, where it may be
/// assigned to either endpoint side of a long edge. ELK's label-dummy switcher
/// runs after crossing minimization and swaps the proxy with the corresponding
/// long-edge dummy before BK placement. The edge chain stays continuous by
/// rewiring the four incident port lists in source order.
pub fn switch_center_edge_label_dummies(
    arena: &mut LayeredArena,
    graph_id: LGraphId,
) -> Result<(), LayeringError> {
    let Some(graph) = arena.graphs.get(graph_id as usize) else {
        return Err(LayeringError::UnknownGraph(graph_id));
    };
    let layer_ids = graph.layers.clone();
    let label_dummies = layer_ids
        .iter()
        .flat_map(|layer_id| arena.layers[*layer_id as usize].nodes.iter().copied())
        .filter(|node_id| arena.nodes[*node_id as usize].node_type == LayeredNodeType::Label)
        .collect::<Vec<_>>();

    for label in label_dummies {
        let info = gather_center_label_dummy_info(arena, graph_id, label)?;
        let target_layer = info.leftmost_layer + (info.total_dummy_count() - 1) / 2;
        let current_layer = info.leftmost_layer + info.left_long_edges.len();
        if target_layer == current_layer {
            continue;
        }
        let long_dummy = info
            .dummy_at(target_layer - info.leftmost_layer)
            .ok_or_else(|| {
                LayeringError::Import(format!(
                    "center-label proxy {label} has no long-edge dummy at layer {target_layer}"
                ))
            })?;
        swap_label_and_long_edge_dummies(arena, label, long_dummy)?;
    }
    Ok(())
}

fn gather_center_label_dummy_info(
    arena: &LayeredArena,
    graph_id: LGraphId,
    label: LNodeId,
) -> Result<CenterLabelDummyInfo, LayeringError> {
    let graph_layers = &arena.graphs[graph_id as usize].layers;
    let layer_index = |node_id: LNodeId| -> Result<usize, LayeringError> {
        let layer = arena.nodes[node_id as usize]
            .layer
            .ok_or_else(|| LayeringError::Import(format!("node {node_id} has no layer")))?;
        graph_layers
            .iter()
            .position(|candidate| *candidate == layer)
            .ok_or_else(|| {
                LayeringError::Import(format!("node {node_id} is outside graph {graph_id}"))
            })
    };
    let source_node = |edge_id: LEdgeId| {
        let edge = &arena.edges[edge_id as usize];
        edge.source_port
            .map(|port_id| arena.ports[port_id as usize].node)
            .unwrap_or(edge.source)
    };
    let target_node = |edge_id: LEdgeId| {
        let edge = &arena.edges[edge_id as usize];
        edge.target_port
            .map(|port_id| arena.ports[port_id as usize].node)
            .unwrap_or(edge.target)
    };

    let mut left_long_edges = Vec::new();
    let mut current = label;
    loop {
        let Some(edge_id) = arena.node_incoming_edges(current).first().copied() else {
            return Err(LayeringError::Import(format!(
                "center-label proxy {label} has no incoming edge chain"
            )));
        };
        current = source_node(edge_id);
        if arena.nodes[current as usize].node_type != LayeredNodeType::LongEdge {
            break;
        }
        left_long_edges.push(current);
    }
    left_long_edges.reverse();

    let mut right_long_edges = Vec::new();
    current = label;
    loop {
        let Some(edge_id) = arena.node_outgoing_edges(current).first().copied() else {
            return Err(LayeringError::Import(format!(
                "center-label proxy {label} has no outgoing edge chain"
            )));
        };
        current = target_node(edge_id);
        if arena.nodes[current as usize].node_type != LayeredNodeType::LongEdge {
            break;
        }
        right_long_edges.push(current);
    }

    let leftmost_node = left_long_edges.first().copied().unwrap_or(label);
    let rightmost_node = right_long_edges.last().copied().unwrap_or(label);
    Ok(CenterLabelDummyInfo {
        label,
        left_long_edges,
        right_long_edges,
        leftmost_layer: layer_index(leftmost_node)?,
        rightmost_layer: layer_index(rightmost_node)?,
    })
}

fn swap_label_and_long_edge_dummies(
    arena: &mut LayeredArena,
    label: LNodeId,
    long_dummy: LNodeId,
) -> Result<(), LayeringError> {
    let label_layer = arena.nodes[label as usize]
        .layer
        .ok_or_else(|| LayeringError::Import(format!("label dummy {label} has no layer")))?;
    let long_layer = arena.nodes[long_dummy as usize].layer.ok_or_else(|| {
        LayeringError::Import(format!("long-edge dummy {long_dummy} has no layer"))
    })?;
    let position = |layer_id: LayerId, node_id: LNodeId| {
        arena.layers[layer_id as usize]
            .nodes
            .iter()
            .position(|candidate| *candidate == node_id)
            .ok_or_else(|| {
                LayeringError::Import(format!("node {node_id} is absent from layer {layer_id}"))
            })
    };
    let label_position = position(label_layer, label)?;
    let long_position = position(long_layer, long_dummy)?;
    let (label_input, label_output) = dummy_input_output_ports(arena, label)?;
    let (long_input, long_output) = dummy_input_output_ports(arena, long_dummy)?;
    let long_incoming = arena.ports[long_input as usize].incoming_edges.clone();
    let long_outgoing = arena.ports[long_output as usize].outgoing_edges.clone();
    let label_incoming = arena.ports[label_input as usize].incoming_edges.clone();
    let label_outgoing = arena.ports[label_output as usize].outgoing_edges.clone();

    if label_layer != long_layer {
        arena.layers[label_layer as usize].nodes[label_position] = long_dummy;
        arena.layers[long_layer as usize].nodes[long_position] = label;
        arena.nodes[label as usize].layer = Some(long_layer);
        arena.nodes[long_dummy as usize].layer = Some(label_layer);
        for (index, node_id) in arena.layers[label_layer as usize]
            .nodes
            .iter()
            .copied()
            .enumerate()
        {
            arena.nodes[node_id as usize].traversal_index = index;
        }
        for (index, node_id) in arena.layers[long_layer as usize]
            .nodes
            .iter()
            .copied()
            .enumerate()
        {
            arena.nodes[node_id as usize].traversal_index = index;
        }
    }

    for edge_id in long_incoming {
        set_edge_target_port(arena, edge_id, label_input);
    }
    for edge_id in long_outgoing {
        set_edge_source_port(arena, edge_id, label_output);
    }
    for edge_id in label_incoming {
        set_edge_target_port(arena, edge_id, long_input);
    }
    for edge_id in label_outgoing {
        set_edge_source_port(arena, edge_id, long_output);
    }
    Ok(())
}

fn dummy_input_output_ports(
    arena: &LayeredArena,
    node_id: LNodeId,
) -> Result<(LPortId, LPortId), LayeringError> {
    let ports = &arena.nodes[node_id as usize].ports;
    let input = ports.iter().copied().find(|port_id| {
        arena.ports[*port_id as usize]
            .incoming_edges
            .iter()
            .any(|edge_id| arena.edges[*edge_id as usize].active)
    });
    let output = ports.iter().copied().find(|port_id| {
        arena.ports[*port_id as usize]
            .outgoing_edges
            .iter()
            .any(|edge_id| arena.edges[*edge_id as usize].active)
    });
    match (input, output) {
        (Some(input), Some(output)) => Ok((input, output)),
        _ => Err(LayeringError::Import(format!(
            "dummy node {node_id} has no input/output port"
        ))),
    }
}

fn set_edge_source_port(arena: &mut LayeredArena, edge_id: LEdgeId, port_id: LPortId) {
    let old_port = arena.edges[edge_id as usize].source_port;
    if let Some(old_port) = old_port
        && old_port != port_id
    {
        arena.ports[old_port as usize]
            .outgoing_edges
            .retain(|candidate| *candidate != edge_id);
    }
    arena.edges[edge_id as usize].source = arena.ports[port_id as usize].node;
    arena.edges[edge_id as usize].source_port = Some(port_id);
    if !arena.ports[port_id as usize]
        .outgoing_edges
        .contains(&edge_id)
    {
        arena.ports[port_id as usize].outgoing_edges.push(edge_id);
    }
}

fn set_edge_target_port(arena: &mut LayeredArena, edge_id: LEdgeId, port_id: LPortId) {
    let old_port = arena.edges[edge_id as usize].target_port;
    if let Some(old_port) = old_port
        && old_port != port_id
    {
        arena.ports[old_port as usize]
            .incoming_edges
            .retain(|candidate| *candidate != edge_id);
    }
    arena.edges[edge_id as usize].target = arena.ports[port_id as usize].node;
    arena.edges[edge_id as usize].target_port = Some(port_id);
    if !arena.ports[port_id as usize]
        .incoming_edges
        .contains(&edge_id)
    {
        arena.ports[port_id as usize].incoming_edges.push(edge_id);
    }
}

const POLYLINE_MINIMUM_VERTICAL_DIFFERENCE: f64 = 1.0;
const POLYLINE_LAYER_SPACE_FACTOR: f64 = 0.4;

/// Runs the v0.9.0 source-shaped polyline edge router over one flat graph.
///
/// The pinned Go router places each layer horizontally, routes in-layer edges
/// through a midpoint, and then adds port-exit bends for inter-layer edges.
/// This function keeps those operations on the arena so the intermediate
/// points and their insertion order can be compared before codec conversion.
pub fn route_polyline_edges(
    arena: &mut LayeredArena,
    graph_id: LGraphId,
    node_spacing: f64,
    edge_spacing: f64,
) -> Result<(), LayeringError> {
    route_polyline_edges_with_mode(arena, graph_id, node_spacing, edge_spacing, None)
}

/// Runs the source-shaped orthogonal phase used by D2's default ELK profile.
///
/// The orthogonal router reserves the configured edge-to-node clearance on
/// both sides of a layer gap and then shares routing lanes between compatible
/// edges. The older polyline helper remains available for direct phase tests;
/// this entry point is the qualification path for the pinned D2 adapter.
pub fn route_orthogonal_edges(
    arena: &mut LayeredArena,
    graph_id: LGraphId,
    node_spacing: f64,
    edge_spacing: f64,
    edge_node_spacing: f64,
) -> Result<(), LayeringError> {
    route_polyline_edges_with_mode(
        arena,
        graph_id,
        node_spacing,
        edge_spacing,
        Some(edge_node_spacing),
    )
}

fn route_polyline_edges_with_mode(
    arena: &mut LayeredArena,
    graph_id: LGraphId,
    node_spacing: f64,
    edge_spacing: f64,
    orthogonal_edge_node_spacing: Option<f64>,
) -> Result<(), LayeringError> {
    if graph_id as usize >= arena.graphs.len() {
        return Err(LayeringError::UnknownGraph(graph_id));
    }
    if !node_spacing.is_finite()
        || node_spacing < 0.0
        || !edge_spacing.is_finite()
        || edge_spacing < 0.0
    {
        return Err(LayeringError::InvalidSpacing(
            if !node_spacing.is_finite() || node_spacing < 0.0 {
                node_spacing
            } else {
                edge_spacing
            },
        ));
    }
    if let Some(edge_node_spacing) = orthogonal_edge_node_spacing {
        if !edge_node_spacing.is_finite() || edge_node_spacing < 0.0 {
            return Err(LayeringError::InvalidSpacing(edge_node_spacing));
        }
        return route_elk_orthogonal_layers(
            arena,
            graph_id,
            node_spacing,
            edge_spacing,
            edge_node_spacing,
        );
    }
    let layer_ids = arena.graphs[graph_id as usize].layers.clone();
    if layer_ids.is_empty() {
        arena.graphs[graph_id as usize].width = 0.0;
        return Ok(());
    }
    let edge_space_factor = if node_spacing == 0.0 {
        0.0
    } else {
        (edge_spacing / node_spacing).min(1.0)
    };
    let mut x_position = 0.0;
    if let (Some(edge_node_spacing), Some(&first_layer)) =
        (orthogonal_edge_node_spacing, layer_ids.first())
    {
        let initial_slots = long_edge_boundary_slots(arena, graph_id, first_layer);
        #[cfg(feature = "diagnostic-traces")]
        {
            let candidates = arena.layers[first_layer as usize]
                .nodes
                .iter()
                .filter_map(|node_id| {
                    let node = &arena.nodes[*node_id as usize];
                    (node.node_type == LayeredNodeType::LongEdge).then(|| {
                        let ports = node
                            .ports
                            .iter()
                            .map(|port_id| {
                                let port = &arena.ports[*port_id as usize];
                                format!(
                                    "{}:{:?}:in={:?}:out={:?}",
                                    port_id, port.side, port.incoming_edges, port.outgoing_edges,
                                )
                            })
                            .collect::<Vec<_>>();
                        format!("{}:{:?}:{ports:?}", node.source_id, node.layer)
                    })
                })
                .collect::<Vec<_>>();
            crate::trace::emit_value(
                "routing",
                "initial_long_edge_boundary_slots",
                &format!("graphs[{graph_id}].layers[{first_layer}]"),
                &format!("slots={initial_slots};candidates={candidates:?}"),
            );
        }
        if initial_slots > 0 {
            // ELK invokes its orthogonal generator once with an empty left
            // layer. An inverted-port segment can still occupy a boundary
            // slot there, which advances the first real layer by one
            // edge-node clearance before ordinary rank gaps are processed.
            x_position += (initial_slots - 1) as f64 * edge_spacing + edge_node_spacing;
        }
    }
    for layer_index in 0..layer_ids.len() {
        let layer_id = layer_ids[layer_index];
        let next_layer = layer_ids.get(layer_index + 1).copied();
        let orthogonal_lane_count = orthogonal_edge_node_spacing.map(|_| {
            next_layer.map_or(1, |next| {
                orthogonal_lane_count(arena, graph_id, layer_id, next)
            })
        });
        let layer_width = layer_width(arena, layer_id);
        arena.layers[layer_id as usize].width = layer_width;
        place_nodes_horizontally(arena, layer_id, x_position, layer_width);

        let external_edge = next_layer.is_some_and(|next_layer| {
            arena.edges.iter().any(|edge| {
                edge.active
                    && owner_in_graph(arena, edge.source, graph_id)
                        .and_then(|node| arena.nodes[node as usize].layer)
                        == Some(layer_id)
                    && owner_in_graph(arena, edge.target, graph_id)
                        .and_then(|node| arena.nodes[node as usize].layer)
                        == Some(next_layer)
                    && (arena.nodes[edge.source as usize].graph != graph_id
                        || arena.nodes[edge.target as usize].graph != graph_id)
            })
        });
        let hierarchy_dummy_edge = next_layer.is_some_and(|next_layer| {
            arena.edges.iter().any(|edge| {
                edge.active
                    && owner_in_graph(arena, edge.source, graph_id)
                        .and_then(|node| arena.nodes[node as usize].layer)
                        == Some(layer_id)
                    && owner_in_graph(arena, edge.target, graph_id)
                        .and_then(|node| arena.nodes[node as usize].layer)
                        == Some(next_layer)
                    && (arena.nodes[edge.source as usize].node_type
                        == LayeredNodeType::ExternalPort
                        || arena.nodes[edge.target as usize].node_type
                            == LayeredNodeType::ExternalPort)
            })
        });
        #[cfg(feature = "diagnostic-traces")]
        let hierarchy_boundary_edge = next_layer.is_some_and(|next_layer| {
            arena.edges.iter().any(|edge| {
                edge.active
                    && owner_in_graph(arena, edge.source, graph_id)
                        .and_then(|node| arena.nodes[node as usize].layer)
                        == Some(layer_id)
                    && owner_in_graph(arena, edge.target, graph_id)
                        .and_then(|node| arena.nodes[node as usize].layer)
                        == Some(next_layer)
                    && [edge.source_port, edge.target_port]
                        .into_iter()
                        .flatten()
                        .any(|port| arena.port_border_offsets.contains_key(&port))
            })
        });

        #[cfg(feature = "diagnostic-traces")]
        crate::trace::emit_value(
            "routing",
            "layer_spacing_classification",
            &format!("graphs[{graph_id}].layers[{layer_id}->{next_layer:?}]"),
            &format!(
                "lanes={orthogonal_lane_count:?};external={external_edge};hierarchy_dummy={hierarchy_dummy_edge};hierarchy_boundary={hierarchy_boundary_edge};node_spacing={:016x};edge_node_spacing={:016x}",
                node_spacing.to_bits(),
                orthogonal_edge_node_spacing.unwrap_or(0.0).to_bits(),
            ),
        );

        // External endpoints, hierarchy boundary ports, and long-edge dummies
        // can reserve a source-shaped lane even when no bend segment remains.
        let spacing_lane_count = orthogonal_lane_count.map(|count| {
            if count == 0 && (external_edge || hierarchy_dummy_edge) {
                1
            } else {
                count
            }
        });

        let mut maximum_vertical_difference: f64 = 0.0;
        let nodes = arena.layers[layer_id as usize].nodes.clone();
        for node_id in nodes {
            let outgoing = arena
                .outgoing_edges(node_id)
                .map(|(edge_id, _)| edge_id)
                .collect::<Vec<_>>();
            for edge_id in outgoing {
                let (source_port, target_port, target) = {
                    let edge = &arena.edges[edge_id as usize];
                    (edge.source_port, edge.target_port, edge.target)
                };
                let (Some(source_port), Some(target_port)) = (source_port, target_port) else {
                    continue;
                };
                let source_y = absolute_port_anchor(arena, source_port).y;
                let target_y = absolute_port_anchor(arena, target_port).y;
                maximum_vertical_difference =
                    maximum_vertical_difference.max((target_y - source_y).abs());
                if arena.nodes[target as usize].layer != Some(layer_id) {
                    continue;
                }
                let middle_y = (source_y + target_y) / 2.0;
                let bend_x = if arena.ports[source_port as usize].side == PortSide::East {
                    x_position + layer_width + edge_spacing
                } else {
                    x_position - edge_spacing
                };
                #[cfg(feature = "diagnostic-traces")]
                trace_routing_bend(
                    arena,
                    graph_id,
                    "in_layer_port_bend",
                    edge_id,
                    Point {
                        x: bend_x,
                        y: middle_y,
                    },
                    "front",
                );
                insert_bend_at_front(
                    &mut arena.edges[edge_id as usize].bend_points,
                    Point {
                        x: bend_x,
                        y: middle_y,
                    },
                );
            }
            // ELK's default orthogonal router adds routing bends from its lane
            // generator. Port-exit bends belong only to the separate polyline
            // router; applying them here creates an extra elbow on external
            // hierarchy edges that the v0.9.0 orthogonal router never emits.
            if orthogonal_edge_node_spacing.is_none() {
                let node_output_difference = process_polyline_node(
                    arena,
                    node_id,
                    x_position,
                    layer_width,
                    edge_spacing,
                    POLYLINE_LAYER_SPACE_FACTOR * edge_space_factor,
                );
                maximum_vertical_difference =
                    maximum_vertical_difference.max(node_output_difference);
            }
        }
        if let Some(next_layer) = layer_ids.get(layer_index + 1).copied() {
            maximum_vertical_difference =
                maximum_vertical_difference.max(west_in_layer_edge_y_difference(arena, next_layer));
        }
        let mut layer_spacing = if let Some(edge_node_spacing) = orthogonal_edge_node_spacing {
            // elk-go's orthogonal generator reports zero routing slots when
            // every inter-layer edge is already aligned (including an empty
            // layer pair). In that case the router advances by the configured
            // node-to-node layer spacing and does not reserve two edge-node
            // clearances. Treating the implicit straight edge as one lane
            // inflated the simple `a -> b` gap from 70px to 80px.
            if orthogonal_lane_count == Some(0) && !external_edge && !hierarchy_dummy_edge {
                node_spacing
            } else {
                // OrthogonalEdgeRouter's routing generator starts and ends its
                // slots one edge-node spacing away from each layer. A single
                // compatible lane therefore occupies two clearances; additional
                // lanes consume edge-edge spacing. The source generator may add
                // more slots after conflict analysis; preserve those channels.
                let lane_count = spacing_lane_count.unwrap_or(1);
                let clearance = if external_edge && lane_count == 1 {
                    node_spacing
                } else {
                    2.0 * edge_node_spacing
                };
                if hierarchy_dummy_edge {
                    0.0
                } else {
                    (clearance + lane_count.saturating_sub(1) as f64 * edge_spacing)
                        .max(if external_edge { 0.0 } else { node_spacing })
                }
            }
        } else {
            POLYLINE_LAYER_SPACE_FACTOR * edge_space_factor * maximum_vertical_difference
        };
        if layer_index + 1 < layer_ids.len() && orthogonal_edge_node_spacing.is_none() {
            layer_spacing += node_spacing;
        }
        // The final boundary has no inter-layer segments. Upstream's empty
        // right-node list is external for spacing purposes and reserves no
        // node-node gap or edge-node clearances when RouteEdges returns zero.
        if next_layer.is_none() && orthogonal_edge_node_spacing.is_some() {
            layer_spacing = 0.0;
        }
        x_position += layer_width + layer_spacing;
    }
    if let Some(edge_node_spacing) = orthogonal_edge_node_spacing {
        if let Some(&last_layer) = layer_ids.last() {
            let terminal_slots = long_edge_boundary_slots(arena, graph_id, last_layer);
            #[cfg(feature = "diagnostic-traces")]
            crate::trace::emit_value(
                "routing",
                "terminal_long_edge_boundary_slots",
                &format!("graphs[{graph_id}].layers[{last_layer}]"),
                &format!("slots={terminal_slots}"),
            );
            if terminal_slots > 0 {
                // ELK runs one final RouteEdges pass with an empty right
                // layer. Inverted-port long-edge dummies can occupy its
                // boundary slots, extending the graph by the same clearance
                // used by the virtual empty-left pass above.
                x_position += (terminal_slots - 1) as f64 * edge_spacing + edge_node_spacing;
            }
        }
        route_orthogonal_inter_layer_edges(arena, graph_id, edge_spacing, edge_node_spacing)?;
    } else {
        route_unconstrained_inter_layer_edges(arena, graph_id)?;
    }
    arena.graphs[graph_id as usize].width = x_position;
    Ok(())
}

/// Runs elk-go's interleaved P5 node placement and orthogonal routing loop.
fn route_elk_orthogonal_layers(
    arena: &mut LayeredArena,
    graph_id: LGraphId,
    node_spacing: f64,
    edge_spacing: f64,
    edge_node_spacing: f64,
) -> Result<(), LayeringError> {
    if graph_id as usize >= arena.graphs.len() {
        return Err(LayeringError::UnknownGraph(graph_id));
    }
    if !node_spacing.is_finite()
        || node_spacing < 0.0
        || !edge_spacing.is_finite()
        || edge_spacing < 0.0
        || !edge_node_spacing.is_finite()
        || edge_node_spacing < 0.0
    {
        return Err(LayeringError::InvalidSpacing(
            [node_spacing, edge_spacing, edge_node_spacing]
                .into_iter()
                .find(|value| !value.is_finite() || *value < 0.0)
                .unwrap_or(edge_spacing),
        ));
    }

    let layer_ids = arena.graphs[graph_id as usize].layers.clone();
    if layer_ids.is_empty() {
        arena.graphs[graph_id as usize].width = 0.0;
        return Ok(());
    }
    let shared_random = arena.shared_graph_randoms.contains(&graph_id);
    let seed = arena.graphs[graph_id as usize].random_seed;
    let mut random = if shared_random {
        arena.take_graph_random(graph_id)
    } else {
        JavaRandom::with_stream(seed, "p5-orthogonal-routing")
    };
    let mut x_position = 0.0;
    let mut left_layer = None;
    let mut left_nodes = Vec::<LNodeId>::new();
    #[cfg(feature = "diagnostic-traces")]
    let mut left_layer_index = -1isize;
    let mut next_layer_index = 0usize;

    loop {
        #[cfg(feature = "diagnostic-traces")]
        let layer_start_x = x_position;
        let right_layer = layer_ids.get(next_layer_index).copied();
        let right_nodes = right_layer
            .map(|layer| arena.layers[layer as usize].nodes.clone())
            .unwrap_or_default();
        if let Some(layer_id) = left_layer {
            let width = layer_width(arena, layer_id);
            arena.layers[layer_id as usize].width = width;
            place_nodes_horizontally(arena, layer_id, x_position, width);
            x_position += width;
        }
        let route_start_x = x_position
            + if left_layer.is_some() {
                edge_node_spacing
            } else {
                0.0
            };
        let slots = crate::orthogonal::route_adjacent_layers(
            arena,
            graph_id,
            &left_nodes,
            &right_nodes,
            route_start_x,
            edge_spacing,
            &mut random,
        );
        let left_external = all_external_west_or_east_ports(arena, &left_nodes);
        let right_external = all_external_west_or_east_ports(arena, &right_nodes);
        let mut routing_width = 0.0;
        if slots > 0 {
            routing_width = (slots - 1) as f64 * edge_spacing;
            if left_layer.is_some() {
                routing_width += edge_node_spacing;
            }
            if right_layer.is_some() {
                routing_width += edge_node_spacing;
            }
            if routing_width < node_spacing && !left_external && !right_external {
                routing_width = node_spacing;
            }
        } else if !left_external && !right_external {
            routing_width = node_spacing;
        }
        x_position += routing_width;

        #[cfg(feature = "diagnostic-traces")]
        {
            let layer_id = left_layer.map_or(-1, |layer| layer as i64);
            let layer_width = left_layer.map_or(0.0, |layer| arena.layers[layer as usize].width);
            let nodes = left_nodes
                .iter()
                .map(|node_id| {
                    let node = &arena.nodes[*node_id as usize];
                    format!(
                        "{}:{:016x}:{:016x}:{:016x}:{:016x}:{:016x}:{:016x}",
                        node.source_id,
                        node.position_x.to_bits(),
                        node.position_y.to_bits(),
                        node.width.to_bits(),
                        node.height.to_bits(),
                        node.margin_left.to_bits(),
                        node.margin_right.to_bits(),
                    )
                })
                .collect::<Vec<_>>()
                .join(",");
            crate::trace::emit_value(
                "routing",
                "p5_orthogonal_layer",
                &format!("graphs[{graph_id}].layers[{left_layer_index}->{next_layer_index}]"),
                &format!(
                    "layer={layer_id};start={:016x};layer_width={:016x};route={:016x};slots={slots};left_layer_present={};right_layer_present={};node_spacing={:016x};edge_spacing={:016x};edge_node_spacing={:016x};left_external={left_external};right_external={right_external};routing_width={:016x};next={:016x};nodes={nodes}",
                    layer_start_x.to_bits(),
                    layer_width.to_bits(),
                    route_start_x.to_bits(),
                    left_layer.is_some(),
                    right_layer.is_some(),
                    node_spacing.to_bits(),
                    edge_spacing.to_bits(),
                    edge_node_spacing.to_bits(),
                    routing_width.to_bits(),
                    x_position.to_bits(),
                ),
            );
        }

        if let Some(layer_id) = right_layer {
            next_layer_index += 1;
            left_layer = Some(layer_id);
            left_nodes = right_nodes;
            #[cfg(feature = "diagnostic-traces")]
            {
                left_layer_index = next_layer_index as isize - 1;
            }
        } else {
            break;
        }
    }
    arena.graphs[graph_id as usize].width = x_position;
    if shared_random {
        arena.restore_graph_random(graph_id, random);
    }
    Ok(())
}

fn all_external_west_or_east_ports(arena: &LayeredArena, nodes: &[LNodeId]) -> bool {
    nodes.iter().all(|node_id| {
        if arena.nodes[*node_id as usize].node_type != LayeredNodeType::ExternalPort {
            return false;
        }
        let Some(port_id) = arena.external_port_origins.get(node_id) else {
            return false;
        };
        matches!(
            arena.ports[*port_id as usize].side,
            PortSide::West | PortSide::East
        )
    })
}

/// Counts non-zero segments emitted by the former shortcut boundary pass.
/// Retained for the polyline and diagnostic compatibility paths.
fn long_edge_boundary_slots(
    arena: &LayeredArena,
    graph_id: LGraphId,
    first_layer: LayerId,
) -> usize {
    let first_nodes = &arena.layers[first_layer as usize].nodes;
    let mut visited = BTreeSet::<LPortId>::new();
    let mut slots = 0;
    for node_id in first_nodes {
        let node = &arena.nodes[*node_id as usize];
        if node.node_type != LayeredNodeType::LongEdge {
            continue;
        }
        for seed in node.ports.iter().copied().filter(|port_id| {
            let port = &arena.ports[*port_id as usize];
            port.side == PortSide::East && !port.outgoing_edges.is_empty()
        }) {
            if visited.contains(&seed) {
                continue;
            }
            let mut pending = vec![seed];
            while let Some(port_id) = pending.pop() {
                if !visited.insert(port_id) {
                    continue;
                }
                let port = &arena.ports[port_id as usize];
                let owner = &arena.nodes[port.node as usize];
                if owner.graph != graph_id {
                    continue;
                }
                for edge_id in port.incoming_edges.iter().chain(&port.outgoing_edges) {
                    let edge = &arena.edges[*edge_id as usize];
                    let Some(source_port) = edge.source_port else {
                        continue;
                    };
                    let Some(target_port) = edge.target_port else {
                        continue;
                    };
                    if arena.nodes[arena.ports[source_port as usize].node as usize].graph
                        != graph_id
                        || arena.nodes[arena.ports[target_port as usize].node as usize].graph
                            != graph_id
                    {
                        continue;
                    }
                    pending.push(if source_port == port_id {
                        target_port
                    } else {
                        source_port
                    });
                }
            }
            // Long-edge dummies stand in for ELK's PortDummy association, so
            // the source segment must be counted even when its local edge
            // endpoints alone appear aligned.
            slots += 1;
        }
    }
    slots
}

fn edge_has_rightward_layer_ports(arena: &LayeredArena, edge: &LEdge) -> bool {
    edge.source_port.is_none_or(|port| {
        arena.ports[port as usize].source_id.is_none()
            || arena.port_border_offsets.contains_key(&port)
    }) && edge.target_port.is_none_or(|port| {
        arena.ports[port as usize].source_id.is_none()
            || arena.port_border_offsets.contains_key(&port)
    })
}

fn inter_layer_edge_coordinates(
    arena: &LayeredArena,
    graph_id: LGraphId,
    left: LayerId,
    right: LayerId,
) -> Vec<(LEdgeId, f64, f64)> {
    arena
        .edges
        .iter()
        .enumerate()
        .filter_map(|(edge_id, edge)| {
            if !edge.active
                || !edge_has_rightward_layer_ports(arena, edge)
                || owner_in_graph(arena, edge.source, graph_id)
                    .and_then(|node| arena.nodes[node as usize].layer)
                    != Some(left)
                || owner_in_graph(arena, edge.target, graph_id)
                    .and_then(|node| arena.nodes[node as usize].layer)
                    != Some(right)
            {
                return None;
            }
            let source_y = edge_endpoint_anchor(arena, edge, graph_id, true).y;
            let target_y = edge_endpoint_anchor(arena, edge, graph_id, false).y;
            if (source_y - target_y).abs() < POLYLINE_MINIMUM_VERTICAL_DIFFERENCE {
                return None;
            }
            Some((edge_id as LEdgeId, source_y, target_y))
        })
        .collect()
}

fn orthogonal_lane_count(
    arena: &LayeredArena,
    graph_id: LGraphId,
    left: LayerId,
    right: LayerId,
) -> usize {
    let edges = inter_layer_edge_coordinates(arena, graph_id, left, right);
    if edges.is_empty() {
        // The Go generator has no nonzero segment when all endpoints are
        // already aligned; it returns zero routing slots for that pair.
        return 0;
    }
    let mut crossings = 0usize;
    for first in 0..edges.len() {
        for second in first + 1..edges.len() {
            if (edges[first].1 - edges[second].1) * (edges[first].2 - edges[second].2)
                < -POLYLINE_MINIMUM_VERTICAL_DIFFERENCE
            {
                crossings += 1;
            }
        }
    }
    if crossings == 0 {
        1
    } else if edges.iter().any(|(edge_id, _, _)| {
        let edge = &arena.edges[*edge_id as usize];
        [edge.source_port, edge.target_port]
            .into_iter()
            .flatten()
            .any(|port| arena.port_border_offsets.contains_key(&port))
    }) {
        // elk-go groups crossing hierarchy segments into shared slots. One
        // crossing pair needs one lane in a hierarchy boundary layer.
        crossings
    } else {
        // A flat edge crossing occupies the two outer slots and the opposing
        // segment slot in the source orthogonal router.
        crossings.saturating_add(2)
    }
}

fn crossing_edge_ids(
    arena: &LayeredArena,
    graph_id: LGraphId,
    left: LayerId,
    right: LayerId,
) -> BTreeSet<LEdgeId> {
    let edges = inter_layer_edge_coordinates(arena, graph_id, left, right);
    let mut result = BTreeSet::new();
    for first in 0..edges.len() {
        for second in first + 1..edges.len() {
            if (edges[first].1 - edges[second].1) * (edges[first].2 - edges[second].2)
                < -POLYLINE_MINIMUM_VERTICAL_DIFFERENCE
            {
                result.insert(edges[first].0);
                result.insert(edges[second].0);
            }
        }
    }
    result
}

fn route_orthogonal_inter_layer_edges(
    arena: &mut LayeredArena,
    graph_id: LGraphId,
    edge_spacing: f64,
    edge_node_spacing: f64,
) -> Result<(), LayeringError> {
    if graph_id as usize >= arena.graphs.len() {
        return Err(LayeringError::UnknownGraph(graph_id));
    }
    let layer_ids = arena.graphs[graph_id as usize].layers.clone();
    for pair in layer_ids.windows(2) {
        let left_layer = pair[0];
        let right_layer = pair[1];
        let lane_count = orthogonal_lane_count(arena, graph_id, left_layer, right_layer);
        let crossing_edges = crossing_edge_ids(arena, graph_id, left_layer, right_layer);
        #[cfg(feature = "diagnostic-traces")]
        {
            let edge_coordinates =
                inter_layer_edge_coordinates(arena, graph_id, left_layer, right_layer)
                    .into_iter()
                    .map(|(edge_id, source_y, target_y)| {
                        let edge = &arena.edges[edge_id as usize];
                        format!(
                            "{}>{}@{:016x}:{:016x}",
                            arena.nodes[edge.source as usize].source_id,
                            arena.nodes[edge.target as usize].source_id,
                            source_y.to_bits(),
                            target_y.to_bits(),
                        )
                    })
                    .collect::<Vec<_>>();
            let crossing_pairs = crossing_edges
                .iter()
                .map(|edge_id| {
                    let edge = &arena.edges[*edge_id as usize];
                    format!(
                        "{}>{}",
                        arena.nodes[edge.source as usize].source_id,
                        arena.nodes[edge.target as usize].source_id,
                    )
                })
                .collect::<Vec<_>>();
            let mut border_ports = arena
                .port_border_offsets
                .iter()
                .filter_map(|(port_id, offset)| {
                    let port = &arena.ports[*port_id as usize];
                    let node = &arena.nodes[port.node as usize];
                    (node.layer == Some(left_layer) || node.layer == Some(right_layer)).then(|| {
                        format!(
                            "{}:{:?}:{:016x}",
                            node.source_id,
                            port.side,
                            offset.to_bits()
                        )
                    })
                })
                .collect::<Vec<_>>();
            border_ports.sort();
            crate::trace::emit_value(
                "routing",
                "orthogonal_pair_lanes",
                &format!("graphs[{graph_id}].layers[{left_layer}->{right_layer}]"),
                &format!(
                    "lane_count={lane_count};crossing_edges={crossing_pairs:?};edges={edge_coordinates:?};border_ports={border_ports:?}"
                ),
            );
        }
        let left_right = arena.layers[left_layer as usize]
            .nodes
            .iter()
            .map(|node_id| rightward_routing_extent(arena, *node_id))
            .fold(f64::NEG_INFINITY, f64::max);
        let right_left = arena.layers[right_layer as usize]
            .nodes
            .iter()
            .map(|node| arena.nodes[*node as usize].position_x)
            .fold(f64::INFINITY, f64::min);
        let left_nodes = &arena.layers[left_layer as usize].nodes;
        let right_nodes = &arena.layers[right_layer as usize].nodes;
        let left_boundary_offset = [PortSide::East, PortSide::West]
            .into_iter()
            .map(|side| boundary_port_routing_offset(arena, left_nodes, side))
            .fold(0.0, f64::max);
        let right_boundary_offset = [PortSide::East, PortSide::West]
            .into_iter()
            .map(|side| boundary_port_routing_offset(arena, right_nodes, side))
            .fold(0.0, f64::max);
        let compound_edge = arena.edges.iter().any(|edge| {
            edge.active
                && owner_in_graph(arena, edge.source, graph_id)
                    .and_then(|node| arena.nodes[node as usize].layer)
                    == Some(left_layer)
                && owner_in_graph(arena, edge.target, graph_id)
                    .and_then(|node| arena.nodes[node as usize].layer)
                    == Some(right_layer)
                && (owner_in_graph(arena, edge.source, graph_id)
                    .is_some_and(|node| arena.nodes[node as usize].nested_graph.is_some())
                    || owner_in_graph(arena, edge.target, graph_id)
                        .is_some_and(|node| arena.nodes[node as usize].nested_graph.is_some()))
        });
        let effective_clearance = if compound_edge && lane_count == 1 {
            ((right_left - left_right) / 2.0).min(edge_node_spacing)
        } else {
            edge_node_spacing
        };
        let start_lane = left_right + effective_clearance + left_boundary_offset;
        let end_lane = right_left - effective_clearance - right_boundary_offset;
        #[cfg(feature = "diagnostic-traces")]
        crate::trace::emit_value(
            "routing",
            "orthogonal_pair_extents",
            &format!("graphs[{graph_id}].layers[{left_layer}->{right_layer}]"),
            &format!(
                "left={:016x};right={:016x};clearance={:016x};start={:016x};end={:016x}",
                left_right.to_bits(),
                right_left.to_bits(),
                effective_clearance.to_bits(),
                start_lane.to_bits(),
                end_lane.to_bits()
            ),
        );
        for edge_id in 0..arena.edges.len() as LEdgeId {
            let edge = &arena.edges[edge_id as usize];
            if !edge.active
                || edge.source_port.is_none()
                || edge.target_port.is_none()
                || !edge_has_rightward_layer_ports(arena, edge)
                || arena.nodes[edge.source as usize].node_type == LayeredNodeType::ExternalPort
                || arena.nodes[edge.target as usize].node_type == LayeredNodeType::ExternalPort
                || owner_in_graph(arena, edge.source, graph_id)
                    .and_then(|node| arena.nodes[node as usize].layer)
                    != Some(left_layer)
                || owner_in_graph(arena, edge.target, graph_id)
                    .and_then(|node| arena.nodes[node as usize].layer)
                    != Some(right_layer)
            {
                continue;
            }
            let source_point = edge_endpoint_anchor(arena, edge, graph_id, true);
            let target_point = edge_endpoint_anchor(arena, edge, graph_id, false);
            if (source_point.y - target_point.y).abs() < POLYLINE_MINIMUM_VERTICAL_DIFFERENCE {
                continue;
            }
            let bends = if lane_count == 1 || !crossing_edges.contains(&edge_id) {
                vec![
                    Point {
                        x: start_lane,
                        y: source_point.y,
                    },
                    Point {
                        x: start_lane,
                        y: target_point.y,
                    },
                ]
            } else if source_point.y < target_point.y {
                let middle_y = (source_point.y + target_point.y) / 2.0;
                vec![
                    Point {
                        x: start_lane,
                        y: source_point.y,
                    },
                    Point {
                        x: start_lane,
                        y: middle_y,
                    },
                    Point {
                        x: end_lane,
                        y: middle_y,
                    },
                    Point {
                        x: end_lane,
                        y: target_point.y,
                    },
                ]
            } else {
                let lane = start_lane + (lane_count / 2) as f64 * edge_spacing;
                vec![
                    Point {
                        x: lane,
                        y: source_point.y,
                    },
                    Point {
                        x: lane,
                        y: target_point.y,
                    },
                ]
            };
            #[cfg(feature = "diagnostic-traces")]
            for bend in &bends {
                trace_routing_bend(
                    arena,
                    graph_id,
                    "orthogonal_inter_layer",
                    edge_id,
                    *bend,
                    "back",
                );
            }
            arena.edges[edge_id as usize].bend_points.extend(bends);
        }
    }
    Ok(())
}

/// Adds the default ELK layered bends for ordinary (non-port) edges.
///
/// `elk-go` leaves a single edge between equally aligned nodes as a straight
/// section. When the endpoints occupy different cross-axis positions, its
/// orthogonal router inserts a shared lane in the gap between the two layers:
/// horizontal from the source, vertical through the lane, then horizontal to
/// the target. The earlier Rust phase only handled explicit port and in-layer
/// bends, which made every ordinary fan-out edge diagonal. Keep this pass
/// after all node placement so it uses the final layer gap and remains easy to
/// compare with the Go p5 edge-router trace.
fn route_unconstrained_inter_layer_edges(
    arena: &mut LayeredArena,
    graph_id: LGraphId,
) -> Result<(), LayeringError> {
    if graph_id as usize >= arena.graphs.len() {
        return Err(LayeringError::UnknownGraph(graph_id));
    }
    for edge_id in 0..arena.edges.len() as LEdgeId {
        let edge = &arena.edges[edge_id as usize];
        if !edge.active {
            continue;
        }
        let implicit_endpoints = edge
            .source_port
            .is_none_or(|port| arena.ports[port as usize].implicit)
            && edge
                .target_port
                .is_none_or(|port| arena.ports[port as usize].implicit);
        if !implicit_endpoints {
            continue;
        }
        let source = &arena.nodes[edge.source as usize];
        let target = &arena.nodes[edge.target as usize];
        let (Some(source_layer), Some(target_layer)) = (source.layer, target.layer) else {
            continue;
        };
        if source_layer == target_layer {
            continue;
        }
        let source_point = edge.source_port.map_or(
            Point {
                x: source.position_x + source.width,
                y: source.position_y + source.height / 2.0,
            },
            |port| absolute_port_anchor(arena, port),
        );
        let target_point = edge.target_port.map_or(
            Point {
                x: target.position_x,
                y: target.position_y + target.height / 2.0,
            },
            |port| absolute_port_anchor(arena, port),
        );
        if (source_point.y - target_point.y).abs() < POLYLINE_MINIMUM_VERTICAL_DIFFERENCE {
            continue;
        }
        // The normal form is left-to-right, including for DOWN/UP roots before
        // the wire-level direction transform. Preserve a sensible lane for a
        // reversed cycle-broken edge as well.
        let lane_x = (source_point.x + target_point.x) / 2.0;
        let bends = [
            Point {
                x: lane_x,
                y: source_point.y,
            },
            Point {
                x: lane_x,
                y: target_point.y,
            },
        ];
        #[cfg(feature = "diagnostic-traces")]
        for bend in bends {
            trace_routing_bend(
                arena,
                graph_id,
                "unconstrained_inter_layer",
                edge_id,
                bend,
                "back",
            );
        }
        arena.edges[edge_id as usize].bend_points.extend(bends);
    }
    Ok(())
}

/// Returns a routed edge as `[start, bends..., end]` in arena coordinates.
///
/// The helper is intentionally separate from the wire codec: callers can
/// inspect the exact intermediate route before it is serialized as an ELK
/// edge section.
pub fn edge_route_points(
    arena: &LayeredArena,
    edge_id: LEdgeId,
) -> Result<Vec<Point>, LayeringError> {
    edge_route_points_transfer(arena, edge_id, true)
}

pub(crate) fn edge_route_points_transfer(
    arena: &LayeredArena,
    edge_id: LEdgeId,
    apply_transfer: bool,
) -> Result<Vec<Point>, LayeringError> {
    edge_route_points_in_frame(arena, edge_id, apply_transfer, false, false)
}

/// Junctions in the same frame used by edge route publication. Source
/// compound.JoinHierarchyEdges concatenates segment junctions in segment order.
pub(crate) fn edge_junction_points_for_publication(
    arena: &LayeredArena,
    edge_id: LEdgeId,
) -> Result<Vec<Point>, LayeringError> {
    edge_junction_points_in_frame(arena, edge_id, true)
}
fn edge_junction_points_in_frame(
    arena: &LayeredArena,
    edge_id: LEdgeId,
    source_publication: bool,
) -> Result<Vec<Point>, LayeringError> {
    let edge = arena
        .edges
        .get(edge_id as usize)
        .ok_or(LayeringError::UnknownEdge(edge_id))?;
    if edge.active {
        return Ok(edge.junction_points.clone());
    }
    let root = arena
        .graphs
        .iter()
        .position(|g| g.parent_node.is_none())
        .unwrap_or(0) as LGraphId;
    let mut points = Vec::new();
    for segment_id in &edge.hierarchy_segments {
        let segment = arena
            .edges
            .get(*segment_id as usize)
            .ok_or(LayeringError::UnknownEdge(*segment_id))?;
        if !segment.active {
            continue;
        }
        // The codec adds the edge container's origin after publication. Keep
        // the flow axis in that container's frame, as edgeLayoutOffsets does;
        // translating it to the root would add nested ancestors twice.
        let flow_graph = if source_publication {
            lowest_common_graph(
                arena,
                arena.nodes[edge.source as usize].graph,
                arena.nodes[edge.target as usize].graph,
                root,
            )
        } else {
            root
        };
        let offset = translate_point_to_graph(arena, segment.source, flow_graph, Point::default());
        let reference = coordinates::hierarchy_reference_graph(arena, edge);
        let transfer = coordinates::hierarchy_edge_offset_y(arena, edge);
        let source_offset = coordinates::change_graph_y(
            arena,
            0.0,
            arena.nodes[segment.source as usize].graph,
            reference,
        );
        let local = arena.nested_junction_transfer_y.get(segment_id);
        if let Some((ys, _)) = local {
            assert_eq!(
                ys.len(),
                segment.junction_points.len(),
                "complete junction frame inventory"
            );
        }
        points.extend(
            segment
                .junction_points
                .iter()
                .enumerate()
                .map(|(index, p)| Point {
                    x: p.x + offset.x,
                    y: if source_publication {
                        (local.map_or(p.y, |entry| entry.0[index]) + source_offset) + transfer
                    } else {
                        p.y + offset.y
                    },
                }),
        );
    }
    if node_is_ancestor(arena, edge.source, edge.target) {
        let node = &arena.nodes[edge.source as usize];
        let origin = translate_point_to_graph(
            arena,
            edge.source,
            if source_publication {
                arena.nodes[edge.source as usize].graph
            } else {
                root
            },
            Point {
                x: node.position_x,
                y: node.position_y,
            },
        );
        for point in &mut points {
            point.x -= origin.x;
            if !source_publication {
                point.y -= origin.y;
            }
        }
    }
    Ok(points)
}

/// Routed self-loop points before the source PostprocessSelfLoops node offset.
pub(crate) fn self_loop_node_local_points(
    arena: &LayeredArena,
    edge_id: LEdgeId,
) -> Result<Vec<Point>, LayeringError> {
    edge_route_points_in_frame(arena, edge_id, false, true, false)
}

/// Hierarchy cross-axis ordinates include source edgeLayoutOffsets; the flow
/// axis stays in the existing arena frame for direction mirroring. The codec
/// adds only its flow-axis publication offset for these retired source edges.
pub(crate) fn edge_route_points_for_publication(
    arena: &LayeredArena,
    edge_id: LEdgeId,
) -> Result<Vec<Point>, LayeringError> {
    edge_route_points_in_frame(arena, edge_id, true, false, true)
}

fn edge_route_points_in_frame(
    arena: &LayeredArena,
    edge_id: LEdgeId,
    apply_transfer: bool,
    node_local: bool,
    source_publication: bool,
) -> Result<Vec<Point>, LayeringError> {
    let edge = arena
        .edges
        .get(edge_id as usize)
        .ok_or(LayeringError::UnknownEdge(edge_id))?;
    // ELK removes self-loops from layering and routes them in a separate
    // node-local processor. Direct node endpoints are restored as two north
    // ports, with the loop clearance above the node. Keep that processor
    // boundary explicit here so ordinary edge routing cannot turn a loop into
    // a single generic horizontal bend.
    if edge.active && edge.source == edge.target {
        let node = &arena.nodes[edge.source as usize];
        let mut node_x = if node_local { 0.0 } else { node.position_x };
        let node_y = if node_local { 0.0 } else { node.position_y };
        let mut source = edge_endpoint_anchor(arena, edge, node.graph, true);
        let mut target = edge_endpoint_anchor(arena, edge, node.graph, false);
        if node_local {
            let source_port =
                &arena.ports[edge.source_port.expect("routed loop source port") as usize];
            let target_port =
                &arena.ports[edge.target_port.expect("routed loop target port") as usize];
            source = Point {
                x: source_port.x,
                y: source_port.y,
            };
            target = Point {
                x: target_port.x,
                y: target_port.y,
            };
        }
        if !node_local && let Some(&(internal, offset)) = arena.nested_transfer_x.get(&edge.source)
        {
            let offset = if apply_transfer { offset } else { 0.0 };
            node_x = internal + offset;
            if let Some(port) = edge.source_port {
                source.x = (internal + arena.ports[port as usize].x) + offset;
            }
            if let Some(port) = edge.target_port {
                target.x = (internal + arena.ports[port as usize].x) + offset;
            }
        }
        if !node_local && let Some(&(internal, offset)) = arena.nested_transfer_y.get(&edge.source)
        {
            if let Some(port) = edge.source_port {
                source.y = (internal + arena.ports[port as usize].y) + offset;
            }
            if let Some(port) = edge.target_port {
                target.y = (internal + arena.ports[port as usize].y) + offset;
            }
        }
        let side = edge
            .source_port
            .map_or(PortSide::North, |port| arena.ports[port as usize].side);
        let loop_edges = arena
            .edges
            .iter()
            .filter(|candidate| {
                candidate.active
                    && candidate.source == edge.source
                    && candidate.target == edge.target
                    && candidate
                        .source_port
                        .map_or(PortSide::North, |port| arena.ports[port as usize].side)
                        == side
            })
            .collect::<Vec<_>>();
        let loop_index = loop_edges
            .iter()
            .position(|candidate| std::ptr::eq(*candidate, edge))
            .unwrap_or(0);
        let loop_spacing = arena.graphs[node.graph as usize].self_loop_spacing;
        let edge_spacing = arena.graphs[node.graph as usize].edge_edge_spacing;
        let compacted = self_loop_compacted_slots(arena, edge.source);
        let clearance = |side: PortSide| {
            let slot = compacted
                .as_ref()
                .and_then(|slots| slots.get(&edge_id))
                .and_then(|slots| slots[side as usize])
                .unwrap_or(loop_index);
            let vertical = matches!(
                arena.graphs[0].direction,
                LayerDirection::Down | LayerDirection::Up
            );
            let mut distance = loop_spacing;
            for previous_slot in 0..slot {
                let extent = if let Some(slots) = &compacted {
                    slots
                        .iter()
                        .filter(|(id, assigned)| {
                            assigned[side as usize] == Some(previous_slot)
                                && self_loop_label_side(arena, **id) == Some(side)
                        })
                        .filter_map(|(id, _)| arena.loop_label_sizes.get(id))
                        .map(|(w, h)| if vertical { *w } else { *h })
                        .fold(0.0_f64, f64::max)
                } else {
                    loop_edges
                        .get(previous_slot)
                        .and_then(|previous| {
                            let id = arena
                                .edges
                                .iter()
                                .position(|candidate| std::ptr::eq(candidate, *previous))?
                                as LEdgeId;
                            (self_loop_label_side(arena, id) == Some(side))
                                .then(|| arena.loop_label_sizes.get(&id))
                                .flatten()
                        })
                        .map_or(0.0, |(w, h)| if vertical { *w } else { *h })
                };
                let label_gap = if matches!(side, PortSide::North | PortSide::South) && extent > 0.0
                {
                    extent + arena.graphs[node.graph as usize].edge_label_spacing
                } else {
                    0.0
                };
                distance += label_gap + edge_spacing;
            }
            distance
        };
        // OrthogonalSelfLoopRouter's baselinePosition and outerBendPoint:
        // retain the anchor along the side and displace its normal axis.
        let target_side = edge
            .target_port
            .map_or(side, |port| arena.ports[port as usize].side);
        let margins = arena
            .original_loop_margins
            .get(&edge.source)
            .copied()
            .unwrap_or([
                node.margin_top,
                node.margin_right,
                node.margin_bottom,
                node.margin_left,
            ]);
        let outer = |anchor: Point, side: PortSide| match side {
            PortSide::South => Point {
                x: anchor.x,
                y: node_y + node.height + margins[2] + clearance(side),
            },
            PortSide::East => Point {
                x: node_x + node.width + margins[1] + clearance(side),
                y: anchor.y,
            },
            PortSide::West => Point {
                x: node_x - margins[3] - clearance(side),
                y: anchor.y,
            },
            _ => Point {
                x: anchor.x,
                y: node_y - margins[0] - clearance(side),
            },
        };
        let source_outer = outer(source, side);
        let target_outer = outer(target, target_side);
        let clockwise = self_loop_routing_clockwise(arena, edge_id, side, target_side);
        let label_side = self_loop_inline_label_side(arena, edge_id, side, target_side, clockwise);
        let loop_label_size = arena
            .loop_label_sizes
            .get(&edge_id)
            .map(|(width, height)| {
                if matches!(
                    arena.graphs[0].direction,
                    LayerDirection::Down | LayerDirection::Up
                ) {
                    Point {
                        x: *height,
                        y: *width,
                    }
                } else {
                    Point {
                        x: *width,
                        y: *height,
                    }
                }
            })
            .unwrap_or_default();
        let side_vector = |side| {
            let point = outer(Point::default(), side);
            if matches!(side, PortSide::North | PortSide::South) {
                Point { x: 0.0, y: point.y }
            } else {
                Point { x: point.x, y: 0.0 }
            }
        };
        let mut route = vec![source, source_outer];
        let mut current = side;
        let mut turns = 0;
        while current != target_side && turns < 4 {
            let next = if clockwise {
                loop_side_right(current)
            } else {
                loop_side_left(current)
            };
            if next == PortSide::Undefined {
                return Err(LayeringError::PortWithUndefinedSide);
            }
            let mut current_part = side_vector(current);
            let mut next_part = side_vector(next);
            if let Some(label_side) = label_side {
                if current == label_side {
                    offset_loop_label_side(&mut current_part, label_side, loop_label_size);
                } else if next == label_side {
                    offset_loop_label_side(&mut next_part, label_side, loop_label_size);
                }
            }
            route.push(Point {
                x: current_part.x + next_part.x,
                y: current_part.y + next_part.y,
            });
            current = next;
            turns += 1;
        }
        if current != target_side {
            return Err(LayeringError::PortWithUndefinedSide);
        }
        route.extend([target_outer, target]);
        return Ok(route);
    }
    if !edge.active {
        if !edge.hierarchy_segments.is_empty() {
            let root = arena
                .graphs
                .iter()
                .position(|graph| graph.parent_node.is_none())
                .unwrap_or(0) as LGraphId;
            // compound.JoinHierarchyEdges and Transferrer.edgeLayoutOffsets
            // publish sections relative to their original edge container.
            // The cross axis below already follows those source operations;
            // the flow axis must stop at the common graph as well.
            let root = if source_publication {
                lowest_common_graph(
                    arena,
                    arena.nodes[edge.source as usize].graph,
                    arena.nodes[edge.target as usize].graph,
                    root,
                )
            } else {
                root
            };
            let mut joined = Vec::new();
            let mut source_anchor = edge_endpoint_anchor(arena, edge, root, true);
            let mut target_anchor = edge_endpoint_anchor(arena, edge, root, false);
            // Source publication moves the whole joined route into the
            // transferrer's frame below; only the arena-frame helper keeps
            // this endpoint-local origin.
            if !source_publication && node_is_ancestor(arena, edge.source, edge.target) {
                let source_node = &arena.nodes[edge.source as usize];
                let origin = translate_point_to_graph(
                    arena,
                    edge.source,
                    root,
                    Point {
                        x: source_node.position_x,
                        y: source_node.position_y,
                    },
                );
                source_anchor.x -= origin.x;
                source_anchor.y -= origin.y;
                target_anchor.x -= origin.x;
                target_anchor.y -= origin.y;
            }
            let reference = coordinates::hierarchy_reference_graph(arena, edge);
            if source_publication {
                source_anchor.y = coordinates::hierarchy_endpoint_y(arena, edge, true);
                target_anchor.y = coordinates::hierarchy_endpoint_y(arena, edge, false);
            }
            joined.push(source_anchor);
            let mut last_point: Option<Point> = None;
            let mut last_segment_target: Option<LNodeId> = None;
            for segment_id in &edge.hierarchy_segments {
                let segment = arena
                    .edges
                    .get(*segment_id as usize)
                    .ok_or(LayeringError::UnknownEdge(*segment_id))?;
                if !segment.active {
                    continue;
                }
                let points = edge_route_points(arena, *segment_id)?;
                let mut translated_points = Vec::with_capacity(points.len());
                let source_ys = if source_publication {
                    Some(coordinates::segment_y(arena, *segment_id)?)
                } else {
                    None
                };
                if let Some(ys) = &source_ys {
                    assert_eq!(ys.len(), points.len(), "complete source segment inventory");
                }
                let cross_offset = if source_publication {
                    coordinates::change_graph_y(
                        arena,
                        0.0,
                        arena.nodes[segment.source as usize].graph,
                        reference,
                    )
                } else {
                    0.0
                };
                for (index, point) in points.into_iter().enumerate() {
                    let mut translated = if arena.nodes[segment.source as usize].graph == root {
                        point
                    } else {
                        translate_point_to_graph(arena, segment.source, root, point)
                    };
                    if let Some(ys) = &source_ys {
                        translated.y = ys[index] + cross_offset;
                    }
                    translated_points.push(translated);
                }
                #[cfg(feature = "diagnostic-traces")]
                crate::trace::emit_value(
                    "hierarchy",
                    "compound_join_segment",
                    &format!("edges[{edge_id}].segments[{segment_id}]"),
                    &format!(
                        "graph={};cross_graph={};source_publication={};source={};target={};points={}",
                        arena.nodes[segment.source as usize].graph,
                        if source_publication { reference } else { root },
                        source_publication,
                        arena.nodes[segment.source as usize].source_id,
                        arena.nodes[segment.target as usize].source_id,
                        translated_points
                            .iter()
                            .map(|point| format!(
                                "{:016x}:{:016x}",
                                point.x.to_bits(),
                                point.y.to_bits()
                            ))
                            .collect::<Vec<_>>()
                            .join(","),
                    ),
                );
                let Some(source_point) = translated_points.first().copied() else {
                    continue;
                };
                let Some(target_point) = translated_points.last().copied() else {
                    continue;
                };
                let bends = translated_points
                    .get(1..translated_points.len().saturating_sub(1))
                    .unwrap_or_default();
                if let Some(last) = last_point {
                    let next = bends.first().copied().unwrap_or(target_point);
                    let x_different = (last.x - next.x).abs() > 0.00001;
                    let y_different = (last.y - next.y).abs() > 0.00001;
                    if x_different && y_different {
                        let crosses_hierarchy_boundary = arena.nodes[segment.source as usize]
                            .node_type
                            == LayeredNodeType::ExternalPort
                            || last_segment_target.is_some_and(|node| {
                                arena.nodes[node as usize].node_type
                                    == LayeredNodeType::ExternalPort
                            });
                        if !crosses_hierarchy_boundary {
                            joined.push(source_point);
                        }
                    }
                }
                joined.extend_from_slice(bends);
                last_point = Some(bends.last().copied().unwrap_or(source_point));
                last_segment_target = Some(segment.target);
            }
            if joined.last().is_none_or(|last: &Point| {
                (last.x - target_anchor.x).abs() > 1e-9 || (last.y - target_anchor.y).abs() > 1e-9
            }) {
                joined.push(target_anchor);
            }
            remove_collinear_route_points(&mut joined, &edge.junction_points);
            if source_publication {
                let offset = coordinates::hierarchy_edge_offset_y(arena, edge);
                let offset_x = coordinates::hierarchy_edge_offset_x(arena, edge);
                for point in &mut joined {
                    point.x += offset_x;
                    point.y += offset;
                }
            }
            return Ok(joined);
        }
        return Ok(Vec::new());
    }
    let routing_graph =
        if arena.nodes[edge.source as usize].graph == arena.nodes[edge.target as usize].graph {
            arena.nodes[edge.source as usize].graph
        } else {
            arena
                .graphs
                .iter()
                .position(|graph| graph.parent_node.is_none())
                .unwrap_or(0) as LGraphId
        };
    let mut points = Vec::with_capacity(edge.bend_points.len() + 2);
    points.push(edge_endpoint_anchor(arena, edge, routing_graph, true));
    points.extend(edge.bend_points.iter().copied());
    points.push(edge_endpoint_anchor(arena, edge, routing_graph, false));
    Ok(points)
}

fn loop_side_right(side: PortSide) -> PortSide {
    match side {
        PortSide::North => PortSide::East,
        PortSide::East => PortSide::South,
        PortSide::South => PortSide::West,
        PortSide::West => PortSide::North,
        PortSide::Undefined => PortSide::Undefined,
    }
}

fn loop_side_left(side: PortSide) -> PortSide {
    match side {
        PortSide::North => PortSide::West,
        PortSide::West => PortSide::South,
        PortSide::South => PortSide::East,
        PortSide::East => PortSide::North,
        PortSide::Undefined => PortSide::Undefined,
    }
}

fn loop_port_rank(arena: &LayeredArena, node_id: LNodeId, port_id: LPortId) -> Option<usize> {
    arena
        .nodes
        .get(node_id as usize)?
        .ports
        .iter()
        .position(|candidate| *candidate == port_id)
}

fn loop_port_interval_penalty(
    arena: &LayeredArena,
    node_id: LNodeId,
    left: usize,
    right: usize,
) -> Option<usize> {
    let ports = &arena.nodes.get(node_id as usize)?.ports;
    if ports.is_empty() {
        return None;
    }
    let mut penalty = 0;
    let mut index = (left + 1) % ports.len();
    while index != right {
        let port = arena.ports.get(ports[index] as usize)?;
        penalty += if port.incoming_edges.is_empty() && port.outgoing_edges.is_empty() {
            1
        } else {
            3
        };
        index = (index + 1) % ports.len();
    }
    Some(penalty)
}

/// Reproduces elk-go's `RoutingDirector` choice for a two-port self-loop.
/// The opposing-side case compares the connected-port penalty on both arcs,
/// then the router follows the selected clockwise interval.
pub(crate) fn self_loop_routing_clockwise(
    arena: &LayeredArena,
    edge_id: LEdgeId,
    source_side: PortSide,
    target_side: PortSide,
) -> bool {
    let Some(edge) = arena.edges.get(edge_id as usize) else {
        return true;
    };
    let Some(source_port) = edge.source_port else {
        return true;
    };
    let Some(target_port) = edge.target_port else {
        return true;
    };
    let node_id = edge.source;
    let Some(source_rank) = loop_port_rank(arena, node_id, source_port) else {
        return true;
    };
    let Some(target_rank) = loop_port_rank(arena, node_id, target_port) else {
        return true;
    };
    if source_side == target_side {
        return source_rank < target_rank;
    }
    if loop_side_right(source_side) == target_side {
        return true;
    }
    if loop_side_left(source_side) == target_side {
        return false;
    }

    let (first_side, second_side) = if source_side < target_side {
        (source_side, target_side)
    } else {
        (target_side, source_side)
    };
    let ranks_for_side = |side| {
        [source_port, target_port]
            .into_iter()
            .filter_map(|port| {
                (arena.ports[port as usize].side == side)
                    .then(|| loop_port_rank(arena, node_id, port))
                    .flatten()
            })
            .collect::<Vec<_>>()
    };
    let first_ranks = ranks_for_side(first_side);
    let second_ranks = ranks_for_side(second_side);
    if first_ranks.is_empty() || second_ranks.is_empty() {
        return true;
    }
    let first_left = *first_ranks.iter().min().unwrap();
    let first_right = *second_ranks.iter().max().unwrap();
    let second_left = *second_ranks.iter().min().unwrap();
    let second_right = *first_ranks.iter().max().unwrap();
    let first_penalty =
        loop_port_interval_penalty(arena, node_id, first_left, first_right).unwrap_or(usize::MAX);
    let second_penalty =
        loop_port_interval_penalty(arena, node_id, second_left, second_right).unwrap_or(usize::MAX);
    let (left_side, right_side) = if first_penalty <= second_penalty {
        (first_side, second_side)
    } else {
        (second_side, first_side)
    };

    let mut occupied = [false; 5];
    let mut current = left_side;
    let mut traversed = 0;
    while current != right_side && traversed < 4 {
        occupied[current as usize] = true;
        current = loop_side_right(current);
        traversed += 1;
    }
    occupied[current as usize] = true;
    occupied[loop_side_right(source_side) as usize]
}

pub(crate) fn self_loop_inline_label_side(
    arena: &LayeredArena,
    edge_id: LEdgeId,
    source_side: PortSide,
    target_side: PortSide,
    clockwise: bool,
) -> Option<PortSide> {
    if !arena.inline_loop_labels.contains(&edge_id) {
        return None;
    }
    let mut occupied = [false; 5];
    let mut current = source_side;
    let mut traversed = 0;
    while current != target_side && traversed < 4 {
        occupied[current as usize] = true;
        current = if clockwise {
            loop_side_right(current)
        } else {
            loop_side_left(current)
        };
        traversed += 1;
    }
    if current == PortSide::Undefined {
        return None;
    }
    occupied[current as usize] = true;
    // LabelPlacer clears inline labels for one-sided and corner loops, and
    // for four-sided loops. Only opposing/three-sided routes retain inline.
    if occupied.iter().filter(|side| **side).count() != 3 {
        return None;
    }
    if !occupied[PortSide::North as usize] {
        Some(PortSide::South)
    } else if !occupied[PortSide::South as usize] {
        Some(PortSide::North)
    } else if !occupied[PortSide::West as usize] {
        Some(PortSide::East)
    } else if !occupied[PortSide::East as usize] {
        Some(PortSide::West)
    } else {
        None
    }
}

/// HyperLoop.OccupiedSides for a two-port loop along its routed direction.
pub(crate) fn self_loop_occupied_sides(
    arena: &LayeredArena,
    edge_id: LEdgeId,
) -> Option<[bool; 5]> {
    let edge = arena.edges.get(edge_id as usize)?;
    let source_side = arena.ports[edge.source_port? as usize].side;
    let target_side = arena.ports[edge.target_port? as usize].side;
    let clockwise = self_loop_routing_clockwise(arena, edge_id, source_side, target_side);
    let mut occupied = [false; 5];
    let mut current = source_side;
    let mut traversed = 0;
    while current != target_side && traversed < 4 {
        occupied[current as usize] = true;
        current = if clockwise {
            loop_side_right(current)
        } else {
            loop_side_left(current)
        };
        traversed += 1;
    }
    occupied[current as usize] = true;
    Some(occupied)
}

/// Side chosen by elk-go `loops/routing/label_placer.go:20–145` (PlaceLabels)
/// for a labeled two-port loop. OrthogonalSelfLoopRouter widens a routing
/// slot only for labels on that side (`initializeWithMaxLabelHeight`).
pub(crate) fn self_loop_label_side(arena: &LayeredArena, edge_id: LEdgeId) -> Option<PortSide> {
    if !arena.loop_label_sizes.contains_key(&edge_id) {
        return None;
    }
    let edge = arena.edges.get(edge_id as usize)?;
    let source_side = arena.ports[edge.source_port? as usize].side;
    let target_side = arena.ports[edge.target_port? as usize].side;
    let clockwise = self_loop_routing_clockwise(arena, edge_id, source_side, target_side);
    let occupied = self_loop_occupied_sides(arena, edge_id)?;
    let (left, right) = if clockwise {
        (source_side, target_side)
    } else {
        (target_side, source_side)
    };
    let inline = arena.inline_loop_labels.contains(&edge_id);
    let side = |side: PortSide| occupied[side as usize];
    match occupied.iter().filter(|side| **side).count() {
        1 => Some(source_side),
        2 => [left, right]
            .into_iter()
            .find(|side| *side == PortSide::North)
            .or_else(|| {
                [left, right]
                    .into_iter()
                    .find(|side| *side == PortSide::South)
            }),
        3 if !side(PortSide::North) => Some(PortSide::South),
        3 if !side(PortSide::South) => Some(PortSide::North),
        3 if !side(PortSide::West) => Some(if inline {
            PortSide::East
        } else {
            PortSide::North
        }),
        3 => Some(if inline {
            PortSide::West
        } else {
            PortSide::North
        }),
        // ELK 0.8.1 reads the leftmost port twice for four-sided loops.
        4 => Some(if left == PortSide::North {
            PortSide::South
        } else {
            PortSide::North
        }),
        _ => None,
    }
}

fn offset_loop_label_side(point: &mut Point, side: PortSide, size: Point) {
    match side {
        PortSide::North => point.y -= size.y / 2.0,
        PortSide::South => point.y += size.y / 2.0,
        PortSide::East => point.x += size.x / 2.0,
        PortSide::West => point.x -= size.x / 2.0,
        PortSide::Undefined => {}
    }
}

fn remove_collinear_route_points(points: &mut Vec<Point>, junctions: &[Point]) {
    let mut index = 1;
    while index + 1 < points.len() {
        let previous = points[index - 1];
        let current = points[index];
        let next = points[index + 1];
        let collinear = (previous.x == current.x && current.x == next.x)
            || (previous.y == current.y && current.y == next.y);
        if collinear && !junctions.contains(&current) {
            points.remove(index);
            index = index.saturating_sub(1).max(1);
        } else {
            index += 1;
        }
    }
}

#[inline]
fn margin_top(node: &LNode) -> f64 {
    if node.margins_in_wire_dimensions {
        node.computed_margins[0]
    } else {
        node.margin_top
    }
}

#[inline]
fn margin_right(node: &LNode) -> f64 {
    if node.margins_in_wire_dimensions {
        node.computed_margins[1]
    } else {
        node.margin_right
    }
}

#[inline]
fn margin_bottom(node: &LNode) -> f64 {
    if node.margins_in_wire_dimensions {
        node.computed_margins[2]
    } else {
        node.margin_bottom
    }
}

#[inline]
fn margin_left(node: &LNode) -> f64 {
    if node.margins_in_wire_dimensions {
        node.computed_margins[3]
    } else {
        node.margin_left
    }
}

#[inline]
fn bk_margin_bottom(node: &LNode) -> f64 {
    margin_bottom(node)
}

#[inline]
fn bk_margin_top(node: &LNode) -> f64 {
    margin_top(node)
}

fn layer_width(arena: &LayeredArena, layer_id: LayerId) -> f64 {
    arena.layers[layer_id as usize]
        .nodes
        .iter()
        .map(|node_id| {
            let node = &arena.nodes[*node_id as usize];
            margin_left(node) + node.width + margin_right(node)
        })
        .fold(0.0, f64::max)
}

fn place_nodes_horizontally(
    arena: &mut LayeredArena,
    layer_id: LayerId,
    x_offset: f64,
    layer_width: f64,
) {
    let nodes = arena.layers[layer_id as usize].nodes.clone();
    let max_left_margin = nodes
        .iter()
        .map(|node_id| {
            let node = &arena.nodes[*node_id as usize];
            margin_left(node)
        })
        .fold(0.0, f64::max);
    let max_right_margin = nodes
        .iter()
        .map(|node_id| {
            let node = &arena.nodes[*node_id as usize];
            margin_right(node)
        })
        .fold(0.0, f64::max);
    for node_id in nodes {
        let (width, margin_left, margin_right, incoming, outgoing) = {
            let node = &arena.nodes[node_id as usize];
            // Source counts nonempty port adjacency, not edge multiplicity.
            // Retired wire records are not part of the live adjacency view.
            let incoming = node
                .ports
                .iter()
                .filter(|port| {
                    arena.ports[**port as usize]
                        .incoming_edges
                        .iter()
                        .any(|edge| arena.edges[*edge as usize].active)
                })
                .count();
            let outgoing = node
                .ports
                .iter()
                .filter(|port| {
                    arena.ports[**port as usize]
                        .outgoing_edges
                        .iter()
                        .any(|edge| arena.edges[*edge as usize].active)
                })
                .count();
            (
                node.width,
                margin_left(node),
                margin_right(node),
                incoming,
                outgoing,
            )
        };
        let ratio = match arena.nodes[node_id as usize].alignment {
            NodeAlignment::Left => 0.0,
            NodeAlignment::Right => 1.0,
            NodeAlignment::Center => 0.5,
            _ if incoming + outgoing == 0 => 0.5,
            _ => outgoing as f64 / (incoming + outgoing) as f64,
        };
        let mut relative = (layer_width - width) * ratio;
        if ratio > 0.5 {
            relative -= max_right_margin * 2.0 * (ratio - 0.5);
        } else if ratio < 0.5 {
            relative += max_left_margin * 2.0 * (0.5 - ratio);
        }
        relative = relative.max(margin_left);
        relative = relative.min((layer_width - margin_right - width).max(margin_left));
        arena.nodes[node_id as usize].position_x = x_offset + relative;
    }
}

fn absolute_port_anchor(arena: &LayeredArena, port_id: LPortId) -> Point {
    let port = &arena.ports[port_id as usize];
    let node = &arena.nodes[port.node as usize];
    Point {
        x: node.position_x + port.x,
        y: node.position_y + port.y,
    }
}

fn rightward_routing_extent(arena: &LayeredArena, node_id: LNodeId) -> f64 {
    let node = &arena.nodes[node_id as usize];
    let node_extent = if node.nested_graph.is_some() {
        node.width
    } else {
        margin_left(node) + node.width + margin_right(node)
    };
    node.position_x + node_extent
}

fn boundary_port_routing_offset(arena: &LayeredArena, node_ids: &[LNodeId], side: PortSide) -> f64 {
    node_ids
        .iter()
        .flat_map(|node_id| arena.nodes[*node_id as usize].ports.iter().copied())
        .filter(|port_id| arena.ports[*port_id as usize].side == side)
        .filter_map(|port_id| arena.port_border_offsets.get(&port_id).copied())
        .fold(0.0, f64::max)
}

fn edge_endpoint_anchor(
    arena: &LayeredArena,
    edge: &LEdge,
    graph_id: LGraphId,
    source: bool,
) -> Point {
    let node_id = if source { edge.source } else { edge.target };
    if arena.nodes[node_id as usize].graph == graph_id {
        let port = if source {
            edge.source_port
        } else {
            edge.target_port
        };
        if let Some(port) = port {
            return absolute_port_anchor(arena, port);
        }
    }
    if let Some(port) = if source {
        edge.source_port
    } else {
        edge.target_port
    } {
        return translate_point_to_graph(
            arena,
            node_id,
            graph_id,
            absolute_port_anchor(arena, port),
        );
    }
    let node = &arena.nodes[node_id as usize];
    let port_point = if source {
        Point {
            x: node.position_x + node.width,
            y: node.position_y + node.height / 2.0,
        }
    } else {
        Point {
            x: node.position_x,
            y: node.position_y + node.height / 2.0,
        }
    };
    translate_point_to_graph(arena, node_id, graph_id, port_point)
}

fn translate_point_to_graph(
    arena: &LayeredArena,
    mut node_id: LNodeId,
    graph_id: LGraphId,
    mut point: Point,
) -> Point {
    let mut visited = BTreeSet::new();
    while arena.nodes[node_id as usize].graph != graph_id {
        if !visited.insert(node_id) {
            break;
        }
        let graph = arena.nodes[node_id as usize].graph;
        let Some(parent_node) = arena.graphs[graph as usize].parent_node else {
            break;
        };
        point.y += arena.graphs[graph as usize].offset_y;
        point.x += arena.nodes[parent_node as usize].position_x;
        point.y += arena.nodes[parent_node as usize].position_y;
        node_id = parent_node;
    }
    point
}

fn node_is_ancestor(arena: &LayeredArena, ancestor: LNodeId, descendant: LNodeId) -> bool {
    let mut graph = arena.nodes[descendant as usize].graph;
    while let Some(parent_node) = arena.graphs[graph as usize].parent_node {
        if parent_node == ancestor {
            return true;
        }
        graph = arena.nodes[parent_node as usize].graph;
    }
    false
}

fn insert_bend_at_front(points: &mut Vec<Point>, point: Point) {
    points.push(Point::default());
    let previous_len = points.len() - 1;
    points.copy_within(..previous_len, 1);
    points[0] = point;
}

#[cfg(feature = "diagnostic-traces")]
fn trace_routing_bend(
    arena: &LayeredArena,
    graph_id: LGraphId,
    operation: &str,
    edge_id: LEdgeId,
    point: Point,
    insertion: &str,
) {
    let edge = &arena.edges[edge_id as usize];
    let source = &arena.nodes[edge.source as usize].source_id;
    let target = &arena.nodes[edge.target as usize].source_id;
    crate::trace::emit_value(
        "routing",
        operation,
        &format!("graphs[{graph_id}].edges[{source}>{target}].bend"),
        &format!(
            "insertion={insertion};x={:016x};y={:016x}",
            point.x.to_bits(),
            point.y.to_bits()
        ),
    );
}

fn process_polyline_node(
    arena: &mut LayeredArena,
    node_id: LNodeId,
    layer_left: f64,
    layer_width: f64,
    _edge_spacing: f64,
    maximum_acceptable_difference: f64,
) -> f64 {
    let layer_right = layer_left + layer_width;
    let ports = arena.nodes[node_id as usize].ports.clone();
    let mut maximum_difference: f64 = 0.0;
    for port_id in ports {
        let port = arena.ports[port_id as usize].clone();
        let absolute_anchor = absolute_port_anchor(arena, port_id);
        let Some(bend_x) = (match port.side {
            PortSide::East => Some(layer_right),
            PortSide::West => Some(layer_left),
            PortSide::North | PortSide::South | PortSide::Undefined => None,
        }) else {
            continue;
        };
        let bend = Point {
            x: bend_x,
            y: absolute_anchor.y,
        };
        let difference = (absolute_anchor.x - bend.x).abs();
        maximum_difference = maximum_difference.max(difference);
        if difference <= maximum_acceptable_difference
            || arena.nodes[node_id as usize].node_type == LayeredNodeType::LongEdge
        {
            continue;
        }
        let connected = port
            .incoming_edges
            .iter()
            .chain(port.outgoing_edges.iter())
            .copied()
            .filter(|edge| arena.edges[*edge as usize].active)
            .collect::<Vec<_>>();
        let add_junction = connected.len() > 1;
        for edge_id in connected {
            let other_port = if arena.edges[edge_id as usize].source_port == Some(port_id) {
                arena.edges[edge_id as usize].target_port
            } else {
                arena.edges[edge_id as usize].source_port
            };
            let Some(other_port) = other_port else {
                continue;
            };
            if (absolute_port_anchor(arena, other_port).y - bend.y).abs()
                <= POLYLINE_MINIMUM_VERTICAL_DIFFERENCE
            {
                continue;
            }
            if arena.edges[edge_id as usize].source_port == Some(port_id) {
                #[cfg(feature = "diagnostic-traces")]
                trace_routing_bend(
                    arena,
                    arena.nodes[node_id as usize].graph,
                    "orthogonal_port_exit",
                    edge_id,
                    bend,
                    "front",
                );
                insert_bend_at_front(&mut arena.edges[edge_id as usize].bend_points, bend);
            } else {
                #[cfg(feature = "diagnostic-traces")]
                trace_routing_bend(
                    arena,
                    arena.nodes[node_id as usize].graph,
                    "orthogonal_port_exit",
                    edge_id,
                    bend,
                    "back",
                );
                arena.edges[edge_id as usize].bend_points.push(bend);
            }
            if add_junction {
                let junctions = &mut arena.edges[edge_id as usize].junction_points;
                if !junctions.contains(&bend) {
                    junctions.push(bend);
                }
            }
        }
    }
    maximum_difference
}

fn west_in_layer_edge_y_difference(arena: &LayeredArena, layer_id: LayerId) -> f64 {
    let mut maximum: f64 = 0.0;
    let nodes = arena.layers[layer_id as usize].nodes.clone();
    for node_id in nodes {
        for (edge_id, edge) in arena.outgoing_edges(node_id) {
            if arena.nodes[edge.target as usize].layer != Some(layer_id) {
                continue;
            }
            let Some(source_port) = edge.source_port else {
                continue;
            };
            if arena.ports[source_port as usize].side != PortSide::West {
                continue;
            }
            let Some(target_port) = edge.target_port else {
                continue;
            };
            maximum = maximum.max(
                (absolute_port_anchor(arena, target_port).y
                    - absolute_port_anchor(arena, source_port).y)
                    .abs(),
            );
            let _ = edge_id;
        }
    }
    maximum
}

/// Returns layer node IDs in source order, suitable for trace snapshots.
#[must_use]
pub fn layer_nodes(arena: &LayeredArena, graph_id: LGraphId) -> Vec<Vec<LNodeId>> {
    arena
        .graphs
        .get(graph_id as usize)
        .map(|graph| {
            graph
                .layers
                .iter()
                .map(|layer| arena.layers[*layer as usize].nodes.clone())
                .collect()
        })
        .unwrap_or_default()
}

/// Restores input model order for one graph scope.
///
/// This ports the D2 v0.9.0 `NODES_AND_EDGES` path of ELK's
/// `SortByInputOrderOfModel`. It runs after ranking and long-edge splitting,
/// before north/south port dummies are inserted. Each layer receives the
/// source's stable insertion sort with comparison history, so long-edge
/// dummies can move relative to normal nodes. Ports then follow adjacent-layer
/// source order and source edge model order, subject to fixed port constraints.
pub fn sort_layers_by_input_model_order(arena: &mut LayeredArena, graph_id: LGraphId) {
    let Some(layers) = arena
        .graphs
        .get(graph_id as usize)
        .map(|graph| graph.layers.clone())
    else {
        return;
    };
    for (layer_index, layer_id) in layers.iter().copied().enumerate() {
        let previous_layer = layers[layer_index.saturating_sub(1)];
        let previous_nodes = arena.layers[previous_layer as usize].nodes.clone();
        sort_nodes_by_input_model_order(arena, layer_id, &previous_nodes, true);
        let node_snapshot = arena.layers[layer_id as usize].nodes.clone();
        for node_id in node_snapshot {
            let node = &arena.nodes[node_id as usize];
            if matches!(
                node.port_constraints,
                PortConstraints::FixedOrder | PortConstraints::FixedPosition
            ) {
                continue;
            }
            let (target_by_port, target_orders) = outgoing_model_order_targets(arena, node_id);
            let mut ports = node.ports.clone();
            {
                let mut comparator = InputModelPortComparator::new(
                    arena,
                    &previous_nodes,
                    &target_by_port,
                    &target_orders,
                    node_id,
                );
                gwt_stable_sort(&mut ports, &mut |first, second| {
                    comparator.compare(first, second)
                });
            }
            arena.nodes[node_id as usize].ports = ports;
        }
        sort_nodes_by_input_model_order(arena, layer_id, &previous_nodes, false);
    }
}

fn sort_nodes_by_input_model_order(
    arena: &mut LayeredArena,
    layer_id: LayerId,
    previous_layer: &[LNodeId],
    before_ports: bool,
) {
    let Some(layer) = arena.layers.get(layer_id as usize) else {
        return;
    };
    let mut nodes = layer.nodes.clone();
    {
        let mut comparator =
            InputModelNodeComparator::new(arena, previous_layer, before_ports, layer_id);
        for index in 1..nodes.len() {
            let value = nodes[index];
            let mut position = index;
            while position > 0
                && comparator.compare(nodes[position - 1], value) == Ordering::Greater
            {
                nodes[position] = nodes[position - 1];
                position -= 1;
            }
            nodes[position] = value;
        }
    }
    arena.layers[layer_id as usize].nodes = nodes;
}

/// Stateful equivalent of ELK's `ModelOrderNodeComparator` for the D2
/// `NODES_AND_EDGES` profile. Keeping comparison history matters because the
/// source uses insertion sort with a non-transitive comparator around long
/// edge dummies.
struct InputModelNodeComparator<'a> {
    arena: &'a LayeredArena,
    previous_layer: &'a [LNodeId],
    before_ports: bool,
    #[cfg(feature = "diagnostic-traces")]
    layer_id: LayerId,
    #[cfg(feature = "diagnostic-traces")]
    comparison_index: usize,
    bigger_than: BTreeMap<LNodeId, BTreeSet<LNodeId>>,
    smaller_than: BTreeMap<LNodeId, BTreeSet<LNodeId>>,
}

impl<'a> InputModelNodeComparator<'a> {
    fn new(
        arena: &'a LayeredArena,
        previous_layer: &'a [LNodeId],
        before_ports: bool,
        _layer_id: LayerId,
    ) -> Self {
        Self {
            arena,
            previous_layer,
            before_ports,
            #[cfg(feature = "diagnostic-traces")]
            layer_id: _layer_id,
            #[cfg(feature = "diagnostic-traces")]
            comparison_index: 0,
            bigger_than: BTreeMap::new(),
            smaller_than: BTreeMap::new(),
        }
    }

    #[cfg(feature = "diagnostic-traces")]
    fn next_comparison_index(&mut self) -> usize {
        let index = self.comparison_index;
        self.comparison_index += 1;
        index
    }

    fn compare(&mut self, first: LNodeId, second: LNodeId) -> Ordering {
        #[cfg(feature = "diagnostic-traces")]
        let comparison_index = self.next_comparison_index();
        trace_order_comparison!(
            &format!(
                "node.layer[{}/before_ports={}]",
                self.layer_id, self.before_ports
            ),
            comparison_index,
            first,
            second,
            None::<Ordering>
        );
        let result = self.compare_inner(first, second);
        trace_order_comparison!(
            &format!(
                "node.layer[{}/before_ports={}]",
                self.layer_id, self.before_ports
            ),
            comparison_index,
            first,
            second,
            Some(result)
        );
        result
    }

    fn compare_inner(&mut self, first: LNodeId, second: LNodeId) -> Ordering {
        if let Some(result) = self.initialize_and_recall(first, second) {
            return result;
        }
        let first_has_model_order = self.has_model_order(first);
        let second_has_model_order = self.has_model_order(second);
        let first_source = self.incoming_source_from_previous_layer(first);
        let second_source = self.incoming_source_from_previous_layer(second);

        if (!first_has_model_order || !second_has_model_order)
            && let (Some((first_port, first_node)), Some((second_port, second_node))) =
                (first_source, second_source)
        {
            if first_node == second_node {
                let source_ports = &self.arena.nodes[first_node as usize].ports;
                let first_position = source_ports.iter().position(|port| *port == first_port);
                let second_position = source_ports.iter().position(|port| *port == second_port);
                if let (Some(first_position), Some(second_position)) =
                    (first_position, second_position)
                {
                    // Source walks the source-port list and tests first
                    // before second, including when both are the same port.
                    let order = first_position.cmp(&second_position).then(Ordering::Less);
                    return self.record(first, second, order);
                }
                return self.record(
                    first,
                    second,
                    self.connected_edge_model_order(first)
                        .cmp(&self.connected_edge_model_order(second)),
                );
            }
            let first_position = self
                .previous_layer
                .iter()
                .position(|node| *node == first_node);
            let second_position = self
                .previous_layer
                .iter()
                .position(|node| *node == second_node);
            if let (Some(first_position), Some(second_position)) = (first_position, second_position)
                && first_position != second_position
            {
                return self.record(first, second, first_position.cmp(&second_position));
            }
        }

        if (!first_has_model_order || !second_has_model_order)
            && (first_source.is_some() != second_source.is_some()
                || (first_source.is_none() && second_source.is_none()))
        {
            let helper_result = self.handle_helper_dummy_nodes(first, second);
            if helper_result != Ordering::Equal {
                return self.record(first, second, helper_result);
            }
            if first_source.is_some() != second_source.is_some()
                && (!first_has_model_order || !second_has_model_order)
            {
                return self.record(
                    first,
                    second,
                    self.connected_edge_model_order(first)
                        .cmp(&self.connected_edge_model_order(second)),
                );
            }
        }

        if first_has_model_order && second_has_model_order {
            return self.record(
                first,
                second,
                self.arena.nodes[first as usize]
                    .traversal_index
                    .cmp(&self.arena.nodes[second as usize].traversal_index),
            );
        }
        self.record(first, second, Ordering::Less)
    }

    fn initialize_and_recall(&mut self, first: LNodeId, second: LNodeId) -> Option<Ordering> {
        self.bigger_than.entry(first).or_default();
        self.bigger_than.entry(second).or_default();
        self.smaller_than.entry(first).or_default();
        self.smaller_than.entry(second).or_default();
        if self.bigger_than[&first].contains(&second) {
            return Some(Ordering::Greater);
        }
        if self.bigger_than[&second].contains(&first) {
            return Some(Ordering::Less);
        }
        if self.smaller_than[&first].contains(&second) {
            return Some(Ordering::Less);
        }
        // Match elk-go's second biggerThan lookup in this branch verbatim.
        if self.smaller_than[&second].contains(&first) && self.bigger_than[&second].contains(&first)
        {
            return Some(Ordering::Greater);
        }
        None
    }

    fn has_model_order(&self, node: LNodeId) -> bool {
        self.arena.nodes[node as usize].node_type == LayeredNodeType::Normal
    }

    fn incoming_source_from_previous_layer(&self, node: LNodeId) -> Option<(LPortId, LNodeId)> {
        let current = &self.arena.nodes[node as usize];
        let layers = &self.arena.graphs[current.graph as usize].layers;
        let index = layers.iter().position(|id| Some(*id) == current.layer)?;
        let previous_layer_id = layers[index.checked_sub(1)?];
        for port_id in &self.arena.nodes[node as usize].ports {
            let port = &self.arena.ports[*port_id as usize];
            let Some(edge_id) = port
                .incoming_edges
                .iter()
                .copied()
                .find(|edge_id| self.arena.edges[*edge_id as usize].active)
            else {
                continue;
            };
            let edge = &self.arena.edges[edge_id as usize];
            if self.arena.nodes[edge.source as usize].layer == Some(previous_layer_id)
                && let Some(source_port) = edge.source_port
            {
                return Some((source_port, edge.source));
            }
        }
        None
    }

    fn connected_edge_model_order(&self, node: LNodeId) -> usize {
        for port_id in &self.arena.nodes[node as usize].ports {
            let port = &self.arena.ports[*port_id as usize];
            let Some(edge_id) = port
                .incoming_edges
                .iter()
                .copied()
                .find(|edge_id| self.arena.edges[*edge_id as usize].active)
            else {
                continue;
            };
            return self.original_edge_model_order(edge_id);
        }
        usize::MAX
    }

    fn original_edge_model_order(&self, mut edge_id: LEdgeId) -> usize {
        if let Some(order) = self.arena.edge_model_order.get(&edge_id) {
            return *order;
        }
        let mut visited = BTreeSet::new();
        loop {
            if !visited.insert(edge_id) {
                return edge_id as usize;
            }
            let edge = &self.arena.edges[edge_id as usize];
            if !self.is_long_edge_dummy(edge.source) {
                return edge_id as usize;
            }
            let source = edge.source;
            let Some(previous_edge) = self.arena.nodes[source as usize]
                .ports
                .iter()
                .flat_map(|port_id| self.arena.ports[*port_id as usize].incoming_edges.iter())
                .copied()
                .find(|candidate| self.arena.edges[*candidate as usize].active)
            else {
                return edge_id as usize;
            };
            edge_id = previous_edge;
        }
    }

    fn handle_helper_dummy_nodes(&mut self, first: LNodeId, second: LNodeId) -> Ordering {
        let first_type = self.is_long_edge_dummy(first);
        let second_type = self.is_long_edge_dummy(second);
        match (first_type, second_type) {
            (true, false) if self.has_model_order(second) => {
                let Some((_, _, source, target)) = self.dummy_endpoint_ports(first) else {
                    return Ordering::Equal;
                };
                let dummy_layer = self.arena.nodes[first as usize].layer;
                if self.arena.nodes[source as usize].layer != dummy_layer
                    && self.arena.nodes[target as usize].layer != dummy_layer
                {
                    return Ordering::Equal;
                }
                if source == second || target == second {
                    return Ordering::Greater;
                }
                self.compare(source, second)
            }
            (false, true) if self.has_model_order(first) => {
                let Some((_, _, source, target)) = self.dummy_endpoint_ports(second) else {
                    return Ordering::Equal;
                };
                let dummy_layer = self.arena.nodes[second as usize].layer;
                if self.arena.nodes[source as usize].layer != dummy_layer
                    && self.arena.nodes[target as usize].layer != dummy_layer
                {
                    return Ordering::Equal;
                }
                if source == first || target == first {
                    return Ordering::Less;
                }
                self.compare(first, source)
            }
            (true, true) => {
                let Some((first_source_port, _, first_source, first_target)) =
                    self.dummy_endpoint_ports(first)
                else {
                    return Ordering::Equal;
                };
                let Some((second_source_port, _, second_source, second_target)) =
                    self.dummy_endpoint_ports(second)
                else {
                    return Ordering::Equal;
                };
                let first_layer = self.arena.nodes[first as usize].layer;
                let second_layer = self.arena.nodes[second as usize].layer;
                let first_source_feedback =
                    self.arena.nodes[first_source as usize].layer == first_layer;
                let first_target_feedback =
                    self.arena.nodes[first_target as usize].layer == first_layer;
                let second_source_feedback =
                    self.arena.nodes[second_source as usize].layer == second_layer;
                let second_target_feedback =
                    self.arena.nodes[second_target as usize].layer == second_layer;
                let first_reference = if first_source_feedback {
                    first_source
                } else if first_target_feedback {
                    first_target
                } else {
                    first
                };
                let second_reference = if second_source_feedback {
                    second_source
                } else if second_target_feedback {
                    second_target
                } else {
                    second
                };
                if first_reference == second_reference {
                    if self.before_ports && first_source_feedback && second_source_feedback {
                        let ports = &self.arena.nodes[first_reference as usize].ports;
                        let first_position =
                            ports.iter().position(|port| *port == first_source_port);
                        let second_position =
                            ports.iter().position(|port| *port == second_source_port);
                        if let (Some(first_position), Some(second_position)) =
                            (first_position, second_position)
                        {
                            return if first_position > second_position {
                                Ordering::Greater
                            } else {
                                Ordering::Less
                            };
                        }
                    } else if !self.before_ports {
                        let ports = &self.arena.nodes[first_reference as usize].ports;
                        if let Some(position) =
                            ports.iter().position(|port| *port == first_source_port)
                            && ports
                                .iter()
                                .position(|port| *port == second_source_port)
                                .is_some_and(|other| position < other)
                        {
                            return Ordering::Less;
                        }
                        if let Some(position) =
                            ports.iter().position(|port| *port == second_source_port)
                            && ports
                                .iter()
                                .position(|port| *port == first_source_port)
                                .is_some_and(|other| position < other)
                        {
                            return Ordering::Greater;
                        }
                    }
                }
                if first_reference == first && second_reference == second {
                    Ordering::Equal
                } else {
                    self.compare(first_reference, second_reference)
                }
            }
            _ => Ordering::Equal,
        }
    }

    fn dummy_endpoint_ports(&self, node: LNodeId) -> Option<(LPortId, LPortId, LNodeId, LNodeId)> {
        let mut incoming_port = None;
        let mut outgoing_port = None;
        for port_id in &self.arena.nodes[node as usize].ports {
            let port = &self.arena.ports[*port_id as usize];
            if incoming_port.is_none()
                && let Some(edge_id) = port.incoming_edges.first().copied()
            {
                let edge = &self.arena.edges[edge_id as usize];
                if edge.active
                    && let Some(source_port) = edge.source_port
                {
                    incoming_port = Some((source_port, edge.source));
                }
            }
            if outgoing_port.is_none()
                && let Some(edge_id) = port.outgoing_edges.first().copied()
            {
                let edge = &self.arena.edges[edge_id as usize];
                if edge.active
                    && let Some(target_port) = edge.target_port
                {
                    outgoing_port = Some((target_port, edge.target));
                }
            }
        }
        let (incoming_port, source) = incoming_port?;
        let (outgoing_port, target) = outgoing_port?;
        Some((incoming_port, outgoing_port, source, target))
    }

    fn is_long_edge_dummy(&self, node: LNodeId) -> bool {
        // Inverted-port helpers are LongEdge nodes too; their diagnostic
        // names do not define the model-order comparator's node type.
        self.arena.nodes[node as usize].node_type == LayeredNodeType::LongEdge
    }

    fn record(&mut self, first: LNodeId, second: LNodeId, result: Ordering) -> Ordering {
        match result {
            Ordering::Greater => self.update(first, second),
            Ordering::Less | Ordering::Equal => self.update(second, first),
        }
        result
    }

    fn update(&mut self, bigger: LNodeId, smaller: LNodeId) {
        self.bigger_than.entry(bigger).or_default().insert(smaller);
        self.smaller_than.entry(smaller).or_default().insert(bigger);
        let very_small = self.bigger_than.get(&smaller).cloned().unwrap_or_default();
        let very_big = self.smaller_than.get(&bigger).cloned().unwrap_or_default();
        for node in &very_small {
            self.bigger_than.entry(bigger).or_default().insert(*node);
            self.smaller_than.entry(*node).or_default().insert(bigger);
            for descendant in &very_big {
                self.smaller_than
                    .entry(*node)
                    .or_default()
                    .insert(*descendant);
            }
        }
        for node in &very_big {
            self.smaller_than.entry(smaller).or_default().insert(*node);
            self.bigger_than.entry(*node).or_default().insert(smaller);
            for ancestor in &very_small {
                self.bigger_than.entry(*node).or_default().insert(*ancestor);
            }
        }
    }
}

/// GWT 2.11's stable object-array merge sort. Its comparator call order is
/// observable because elk-go's port comparator accumulates transitive order.
fn gwt_stable_sort<T: Copy>(values: &mut [T], compare: &mut impl FnMut(T, T) -> Ordering) {
    let mut source = values.to_vec();
    let length = values.len();
    gwt_merge_sort(&mut source, values, 0, length, compare);
}

fn gwt_merge_sort<T: Copy>(
    source: &mut [T],
    destination: &mut [T],
    low: usize,
    high: usize,
    compare: &mut impl FnMut(T, T) -> Ordering,
) {
    let length = high - low;
    if length < 7 {
        for index in low + 1..high {
            let mut current = index;
            while current > low
                && compare(destination[current - 1], destination[current]) == Ordering::Greater
            {
                destination.swap(current - 1, current);
                current -= 1;
            }
        }
        return;
    }

    let middle = low + (length >> 1);
    gwt_merge_sort(destination, source, low, middle, compare);
    gwt_merge_sort(destination, source, middle, high, compare);
    if compare(source[middle - 1], source[middle]) != Ordering::Greater {
        destination[low..high].copy_from_slice(&source[low..high]);
        return;
    }

    let mut left = low;
    let mut right = middle;
    for target in &mut destination[low..high] {
        if right >= high
            || (left < middle && compare(source[left], source[right]) != Ordering::Greater)
        {
            *target = source[left];
            left += 1;
        } else {
            *target = source[right];
            right += 1;
        }
    }
}

/// Stateful `ModelOrderPortComparator` for D2's `NODES_AND_EDGES` profile.
struct InputModelPortComparator<'a> {
    arena: &'a LayeredArena,
    previous_layer: &'a [LNodeId],
    target_by_port: &'a BTreeMap<LPortId, LNodeId>,
    target_orders: &'a BTreeMap<LNodeId, usize>,
    _owner: LNodeId,
    #[cfg(feature = "diagnostic-traces")]
    comparison_index: usize,
    bigger_than: BTreeMap<LPortId, BTreeSet<LPortId>>,
    smaller_than: BTreeMap<LPortId, BTreeSet<LPortId>>,
}

impl<'a> InputModelPortComparator<'a> {
    fn new(
        arena: &'a LayeredArena,
        previous_layer: &'a [LNodeId],
        target_by_port: &'a BTreeMap<LPortId, LNodeId>,
        target_orders: &'a BTreeMap<LNodeId, usize>,
        _owner: LNodeId,
    ) -> Self {
        Self {
            arena,
            previous_layer,
            target_by_port,
            target_orders,
            _owner,
            #[cfg(feature = "diagnostic-traces")]
            comparison_index: 0,
            bigger_than: BTreeMap::new(),
            smaller_than: BTreeMap::new(),
        }
    }

    #[cfg(feature = "diagnostic-traces")]
    fn next_comparison_index(&mut self) -> usize {
        let index = self.comparison_index;
        self.comparison_index += 1;
        index
    }

    fn compare(&mut self, first_id: LPortId, second_id: LPortId) -> Ordering {
        #[cfg(feature = "diagnostic-traces")]
        let comparison_index = self.next_comparison_index();
        trace_order_comparison!(
            &format!("port.owner[{}]", self._owner),
            comparison_index,
            first_id,
            second_id,
            None::<Ordering>
        );
        let result = self.compare_inner(first_id, second_id);
        trace_order_comparison!(
            &format!("port.owner[{}]", self._owner),
            comparison_index,
            first_id,
            second_id,
            Some(result)
        );
        result
    }

    fn compare_inner(&mut self, first_id: LPortId, second_id: LPortId) -> Ordering {
        if let Some(result) = self.initialize_and_recall(first_id, second_id) {
            return result;
        }
        let first = self.arena.ports[first_id as usize].clone();
        let second = self.arena.ports[second_id as usize].clone();
        let side_order = port_side_order(first.side).cmp(&port_side_order(second.side));
        if side_order != Ordering::Equal {
            return self.record(first_id, second_id, side_order);
        }

        let mut reverse_order = if matches!(
            first.side,
            PortSide::West | PortSide::North | PortSide::South
        ) {
            -1_i8
        } else {
            1_i8
        };
        let first_incoming = first
            .incoming_edges
            .iter()
            .copied()
            .find(|edge_id| self.arena.edges[*edge_id as usize].active);
        let second_incoming = second
            .incoming_edges
            .iter()
            .copied()
            .find(|edge_id| self.arena.edges[*edge_id as usize].active);

        if let (Some(first_edge), Some(second_edge)) = (first_incoming, second_incoming) {
            let first_edge = &self.arena.edges[first_edge as usize];
            let second_edge = &self.arena.edges[second_edge as usize];
            let first_source = first_edge.source;
            let second_source = second_edge.source;
            if first_source == second_source
                && let (Some(first_source_port), Some(second_source_port)) =
                    (first_edge.source_port, second_edge.source_port)
            {
                let source_ports = &self.arena.nodes[first_source as usize].ports;
                let first_position = source_ports
                    .iter()
                    .position(|port_id| *port_id == first_source_port);
                let second_position = source_ports
                    .iter()
                    .position(|port_id| *port_id == second_source_port);
                if let (Some(first_position), Some(second_position)) =
                    (first_position, second_position)
                {
                    return self.record(
                        first_id,
                        second_id,
                        apply_reverse_order(
                            first_position.cmp(&second_position).then(Ordering::Less),
                            reverse_order,
                        ),
                    );
                }
            }

            let current_layer = self.arena.nodes[first.node as usize].layer;
            let first_source_is_long_dummy = self.is_long_edge_dummy(first_source);
            let second_source_is_long_dummy = self.is_long_edge_dummy(second_source);
            if first_source != second_source
                && first_source_is_long_dummy
                && second_source_is_long_dummy
                && self.arena.nodes[first_source as usize].layer
                    == self.arena.nodes[second_source as usize].layer
                && self.arena.nodes[first_source as usize].layer == current_layer
            {
                if first.side == PortSide::East {
                    reverse_order = -reverse_order;
                }
                let layer_order =
                    self.arena.nodes[first_source as usize]
                        .layer
                        .and_then(|layer_id| {
                            let layer = &self.arena.layers[layer_id as usize].nodes;
                            let first_position =
                                layer.iter().position(|node_id| *node_id == first_source)?;
                            let second_position =
                                layer.iter().position(|node_id| *node_id == second_source)?;
                            Some(first_position.cmp(&second_position))
                        });
                if let Some(order) = layer_order.filter(|order| *order != Ordering::Equal) {
                    return self.record(
                        first_id,
                        second_id,
                        apply_reverse_order(order, reverse_order),
                    );
                }
            }

            let first_position = self
                .previous_layer
                .iter()
                .position(|node_id| *node_id == first_source);
            let second_position = self
                .previous_layer
                .iter()
                .position(|node_id| *node_id == second_source);
            if let (Some(first_position), Some(second_position)) = (first_position, second_position)
                && first_position != second_position
            {
                return self.record(
                    first_id,
                    second_id,
                    apply_reverse_order(first_position.cmp(&second_position), reverse_order),
                );
            }
        }

        let first_outgoing = first
            .outgoing_edges
            .iter()
            .copied()
            .find(|edge_id| self.arena.edges[*edge_id as usize].active);
        let second_outgoing = second
            .outgoing_edges
            .iter()
            .copied()
            .find(|edge_id| self.arena.edges[*edge_id as usize].active);
        if let (Some(first_edge), Some(second_edge)) = (first_outgoing, second_outgoing) {
            reverse_order = if matches!(first.side, PortSide::West | PortSide::South) {
                -1
            } else {
                1
            };
            let first_target = self.target_by_port.get(&first_id).copied();
            let second_target = self.target_by_port.get(&second_id).copied();
            let first_order = original_edge_model_order(self.arena, first_edge);
            let second_order = original_edge_model_order(self.arena, second_edge);
            let order = if first_target.is_some() && first_target == second_target {
                first_order.cmp(&second_order)
            } else {
                let first_target_order = first_target
                    .and_then(|target| self.target_orders.get(&target).copied())
                    .unwrap_or(first_order);
                let second_target_order = second_target
                    .and_then(|target| self.target_orders.get(&target).copied())
                    .unwrap_or(second_order);
                first_target_order.cmp(&second_target_order)
            };
            if order != Ordering::Equal {
                return self.record(
                    first_id,
                    second_id,
                    apply_reverse_order(order, reverse_order),
                );
            }
        }

        if first_incoming.is_some() && second_outgoing.is_some() {
            return self.record(first_id, second_id, Ordering::Greater);
        }
        if first_outgoing.is_some() && second_incoming.is_some() {
            return self.record(first_id, second_id, Ordering::Less);
        }
        if let (Some(first_order), Some(second_order)) = (first.model_order, second.model_order) {
            let reverse_order = if matches!(first.side, PortSide::West | PortSide::South) {
                -1
            } else {
                1
            };
            return self.record(
                first_id,
                second_id,
                apply_reverse_order(first_order.cmp(&second_order), reverse_order),
            );
        }
        self.record(
            first_id,
            second_id,
            if reverse_order < 0 {
                Ordering::Greater
            } else {
                Ordering::Less
            },
        )
    }

    fn initialize_and_recall(&mut self, first: LPortId, second: LPortId) -> Option<Ordering> {
        self.bigger_than.entry(first).or_default();
        self.bigger_than.entry(second).or_default();
        self.smaller_than.entry(first).or_default();
        self.smaller_than.entry(second).or_default();
        if self.bigger_than[&first].contains(&second) {
            return Some(Ordering::Greater);
        }
        if self.bigger_than[&second].contains(&first) {
            return Some(Ordering::Less);
        }
        if self.smaller_than[&first].contains(&second) {
            return Some(Ordering::Less);
        }
        if self.smaller_than[&second].contains(&first) && self.bigger_than[&second].contains(&first)
        {
            return Some(Ordering::Greater);
        }
        None
    }

    fn is_long_edge_dummy(&self, node: LNodeId) -> bool {
        // Inverted-port helpers are LongEdge nodes too; their diagnostic
        // names do not define the model-order comparator's node type.
        self.arena.nodes[node as usize].node_type == LayeredNodeType::LongEdge
    }

    fn record(&mut self, first: LPortId, second: LPortId, result: Ordering) -> Ordering {
        match result {
            Ordering::Greater => self.update(first, second),
            Ordering::Less | Ordering::Equal => self.update(second, first),
        }
        result
    }

    fn update(&mut self, bigger: LPortId, smaller: LPortId) {
        self.bigger_than.entry(bigger).or_default().insert(smaller);
        self.smaller_than.entry(smaller).or_default().insert(bigger);
        let very_small = self.bigger_than.get(&smaller).cloned().unwrap_or_default();
        let very_big = self.smaller_than.get(&bigger).cloned().unwrap_or_default();
        for node in &very_small {
            self.bigger_than.entry(bigger).or_default().insert(*node);
            self.smaller_than.entry(*node).or_default().insert(bigger);
            for descendant in &very_big {
                self.smaller_than
                    .entry(*node)
                    .or_default()
                    .insert(*descendant);
            }
        }
        for node in &very_big {
            self.smaller_than.entry(smaller).or_default().insert(*node);
            self.bigger_than.entry(*node).or_default().insert(smaller);
            for ancestor in &very_small {
                self.bigger_than.entry(*node).or_default().insert(*ancestor);
            }
        }
    }
}

fn apply_reverse_order(order: Ordering, reverse_order: i8) -> Ordering {
    if reverse_order < 0 {
        order.reverse()
    } else {
        order
    }
}

fn port_side_order(side: PortSide) -> u8 {
    match side {
        PortSide::Undefined => 0,
        PortSide::North => 1,
        PortSide::East => 2,
        PortSide::South => 3,
        PortSide::West => 4,
    }
}

fn long_edge_target_node(arena: &LayeredArena, first_edge: LEdgeId) -> Option<LNodeId> {
    let mut node_id = arena.edges.get(first_edge as usize)?.target;
    let mut visited = BTreeSet::new();
    loop {
        if !visited.insert(node_id) {
            return None;
        }
        if arena.nodes[node_id as usize].node_type == LayeredNodeType::Normal {
            return Some(node_id);
        }
        let next_edge = arena
            .outgoing_edges(node_id)
            .map(|(edge_id, _)| edge_id)
            .next()?;
        node_id = arena.edges[next_edge as usize].target;
    }
}

fn original_edge_model_order(arena: &LayeredArena, mut edge_id: LEdgeId) -> usize {
    if let Some(order) = arena.edge_model_order.get(&edge_id) {
        return *order;
    }
    let mut visited = BTreeSet::new();
    loop {
        if !visited.insert(edge_id) {
            return edge_id as usize;
        }
        let edge = &arena.edges[edge_id as usize];
        let source = &arena.nodes[edge.source as usize];
        if !source.source_id.starts_with("__long_edge_dummy_") {
            return edge_id as usize;
        }
        let Some(previous_edge) = source
            .ports
            .iter()
            .flat_map(|port_id| arena.ports[*port_id as usize].incoming_edges.iter())
            .copied()
            .find(|candidate| arena.edges[*candidate as usize].active)
        else {
            return edge_id as usize;
        };
        edge_id = previous_edge;
    }
}

fn outgoing_model_order_targets(
    arena: &LayeredArena,
    owner: LNodeId,
) -> (BTreeMap<LPortId, LNodeId>, BTreeMap<LNodeId, usize>) {
    let mut target_by_port = BTreeMap::new();
    let mut target_orders = BTreeMap::new();
    for port_id in &arena.nodes[owner as usize].ports {
        let port = &arena.ports[*port_id as usize];
        let Some(edge_id) = port
            .outgoing_edges
            .iter()
            .copied()
            .find(|edge_id| arena.edges[*edge_id as usize].active)
        else {
            continue;
        };
        let Some(target) = long_edge_target_node(arena, edge_id) else {
            continue;
        };
        target_by_port.insert(*port_id, target);
        if arena.edges[edge_id as usize].reversed {
            continue;
        }
        target_orders
            .entry(target)
            .and_modify(|order: &mut usize| {
                *order = (*order).min(original_edge_model_order(arena, edge_id));
            })
            .or_insert_with(|| original_edge_model_order(arena, edge_id));
    }
    (target_by_port, target_orders)
}

/// Performs one deterministic barycenter sweep over a flat layered graph.
///
/// This is the source-shaped core of ELK's phase-three
/// `BarycenterHeuristic`: a node is ordered by the mean position of its
/// neighbors in the already-fixed adjacent layer, and nodes without a
/// neighbor retain their current model order. The full v0.9.0 processor also
/// handles ports, constraints, randomized sweeps, and compound graphs; those
/// concerns remain separate so this primitive can be compared in isolation.
pub fn order_layers_by_barycenter(
    arena: &mut LayeredArena,
    graph_id: LGraphId,
    forward: bool,
) -> Result<(), LayeringError> {
    order_layers_by_barycenter_impl(
        arena,
        graph_id,
        BarycenterSweepOptions {
            forward,
            random: None,
            preordered: false,
            node_relative_port_ranks: false,
            use_source_port_ranks: false,
            randomize_boundary: false,
            hierarchy: None,
            hierarchy_first_sweep: None,
        },
    )
}

/// Assigns ELK's port ranks on one fixed layer. The distributor's kind is
/// selected once per graph from its Java-compatible random stream.
fn calculate_port_ranks(
    arena: &LayeredArena,
    layer: &[LNodeId],
    input: bool,
    node_relative: bool,
) -> Vec<f32> {
    let mut ranks = vec![0.0_f32; arena.ports.len()];
    let mut rank_sum = 0.0_f32;
    for node_id in layer {
        let node = &arena.nodes[*node_id as usize];
        if input {
            let incident_count = node
                .ports
                .iter()
                .filter(|port_id| !arena.ports[**port_id as usize].incoming_edges.is_empty())
                .count();
            let north_count = node
                .ports
                .iter()
                .filter(|port_id| {
                    let port = &arena.ports[**port_id as usize];
                    !port.incoming_edges.is_empty() && port.side == PortSide::North
                })
                .count();
            if node_relative {
                let increment = 1.0_f32 / (incident_count + 1) as f32;
                let mut north_position = rank_sum + north_count as f32 * increment;
                let mut other_position = rank_sum + 1.0_f32 - increment;
                for port_id in &node.ports {
                    let port = &arena.ports[*port_id as usize];
                    if port.incoming_edges.is_empty() {
                        continue;
                    }
                    let rank = if port.side == PortSide::North {
                        let current = north_position;
                        north_position -= increment;
                        current
                    } else {
                        let current = other_position;
                        other_position -= increment;
                        current
                    };
                    ranks[*port_id as usize] = rank;
                }
                rank_sum += 1.0_f32;
            } else {
                let mut north_position = rank_sum + north_count as f32;
                let mut other_position = rank_sum + incident_count as f32;
                for port_id in &node.ports {
                    let port = &arena.ports[*port_id as usize];
                    if port.incoming_edges.is_empty() {
                        continue;
                    }
                    let rank = if port.side == PortSide::North {
                        let current = north_position;
                        north_position -= 1.0_f32;
                        current
                    } else {
                        let current = other_position;
                        other_position -= 1.0_f32;
                        current
                    };
                    ranks[*port_id as usize] = rank;
                }
                rank_sum += incident_count as f32;
            }
        } else {
            let incident_count = node
                .ports
                .iter()
                .filter(|port_id| !arena.ports[**port_id as usize].outgoing_edges.is_empty())
                .count();
            if node_relative {
                let increment = 1.0_f32 / (incident_count + 1) as f32;
                let mut position = rank_sum + increment;
                for port_id in &node.ports {
                    let port = &arena.ports[*port_id as usize];
                    if !port.outgoing_edges.is_empty() {
                        ranks[*port_id as usize] = position;
                        position += increment;
                    }
                }
                rank_sum += 1.0_f32;
            } else {
                let mut position = 0.0_f32;
                for port_id in &node.ports {
                    let port = &arena.ports[*port_id as usize];
                    if !port.outgoing_edges.is_empty() {
                        position += 1.0_f32;
                        ranks[*port_id as usize] = rank_sum + position;
                    }
                }
                rank_sum += position;
            }
        }
    }
    ranks
}

/// Applies elk-go's barycenter port distributor around one node-order sweep.
///
/// Port ranks are attached to stable arena port IDs, while the mutable
/// `node.ports` lists retain the source processor's per-sweep ordering. The
/// source assigns scratch IDs during initialization; using the arena IDs here
/// avoids a redundant mapping without changing rank arithmetic or traversal.
fn distribute_ports_while_sweeping(
    arena: &mut LayeredArena,
    layer_ids: &[LayerId],
    current_index: usize,
    forward: bool,
    node_relative: bool,
    port_barycenters: &mut [f32],
    node_positions: &mut BTreeMap<LNodeId, usize>,
) -> Option<Vec<f32>> {
    let current_layer = layer_ids[current_index];
    let current_nodes = arena.layers[current_layer as usize].nodes.clone();
    update_node_positions(&current_nodes, node_positions);
    let side = if forward {
        PortSide::West
    } else {
        PortSide::East
    };
    let first_layer = if forward {
        current_index == 0
    } else {
        current_index + 1 == layer_ids.len()
    };

    if first_layer {
        let empty_ranks = vec![0.0; arena.ports.len()];
        for node_id in current_nodes {
            distribute_barycenter_ports(
                arena,
                node_id,
                side,
                &empty_ranks,
                port_barycenters,
                node_positions,
            );
        }
        return None;
    }

    let fixed_index = if forward {
        current_index - 1
    } else {
        current_index + 1
    };
    let fixed_layer = layer_ids[fixed_index];
    let fixed_nodes = arena.layers[fixed_layer as usize].nodes.clone();
    let fixed_port_ranks = calculate_port_ranks(arena, &fixed_nodes, !forward, node_relative);

    for node_id in &current_nodes {
        distribute_barycenter_ports(
            arena,
            *node_id,
            side,
            &fixed_port_ranks,
            port_barycenters,
            node_positions,
        );
    }

    // The distributor calculates ranks on the free layer after its ports have
    // been reordered, then distributes the corresponding side of the fixed
    // layer. The caller uses the ranks captured above for this node sort.
    let free_port_ranks = calculate_port_ranks(arena, &current_nodes, forward, node_relative);
    let fixed_side = if forward {
        PortSide::East
    } else {
        PortSide::West
    };
    for node_id in fixed_nodes {
        if arena.nodes[node_id as usize].nested_graph.is_none() {
            distribute_barycenter_ports(
                arena,
                node_id,
                fixed_side,
                &free_port_ranks,
                port_barycenters,
                node_positions,
            );
        }
    }
    Some(fixed_port_ranks)
}

fn update_node_positions(layer_nodes: &[LNodeId], node_positions: &mut BTreeMap<LNodeId, usize>) {
    for (position, node_id) in layer_nodes.iter().copied().enumerate() {
        node_positions.insert(node_id, position);
    }
}

fn distribute_barycenter_ports(
    arena: &mut LayeredArena,
    node_id: LNodeId,
    side: PortSide,
    port_ranks: &[f32],
    port_barycenters: &mut [f32],
    node_positions: &BTreeMap<LNodeId, usize>,
) {
    if matches!(
        arena.nodes[node_id as usize].port_constraints,
        PortConstraints::FixedOrder | PortConstraints::FixedRatio | PortConstraints::FixedPosition
    ) {
        return;
    }

    for port_side in [side, PortSide::South, PortSide::North] {
        let ports = arena.nodes[node_id as usize]
            .ports
            .iter()
            .copied()
            .filter(|port_id| arena.ports[*port_id as usize].side == port_side)
            .collect::<Vec<_>>();
        distribute_barycenter_port_list(
            arena,
            node_id,
            &ports,
            port_ranks,
            port_barycenters,
            node_positions,
        );
    }

    let mut ports = arena.nodes[node_id as usize].ports.clone();
    ports.sort_by(|first, second| {
        let first_port = &arena.ports[*first as usize];
        let second_port = &arena.ports[*second as usize];
        port_side_order(first_port.side)
            .cmp(&port_side_order(second_port.side))
            .then_with(|| {
                let first_barycenter = port_barycenters[*first as usize];
                let second_barycenter = port_barycenters[*second as usize];
                match (first_barycenter == 0.0, second_barycenter == 0.0) {
                    (true, true) => Ordering::Equal,
                    (true, false) => Ordering::Less,
                    (false, true) => Ordering::Greater,
                    (false, false) => first_barycenter
                        .partial_cmp(&second_barycenter)
                        .unwrap_or(Ordering::Equal),
                }
            })
    });
    arena.nodes[node_id as usize].ports = ports;
}

fn distribute_barycenter_port_list(
    arena: &LayeredArena,
    node_id: LNodeId,
    ports: &[LPortId],
    port_ranks: &[f32],
    port_barycenters: &mut [f32],
    node_positions: &BTreeMap<LNodeId, usize>,
) {
    let owner_layer = arena.nodes[node_id as usize].layer;
    let layer_size = owner_layer
        .map(|layer_id| arena.layers[layer_id as usize].nodes.len())
        .unwrap_or(0);
    let absurdly_large = (2 * layer_size + 1) as f32;
    let mut minimum_barycenter = 0.0_f32;
    let mut maximum_barycenter = 0.0_f32;
    let mut in_layer_ports = Vec::new();

    for port_id in ports.iter().copied() {
        let port = &arena.ports[port_id as usize];
        let north_south = matches!(port.side, PortSide::North | PortSide::South);
        let mut sum = 0.0_f32;
        if north_south {
            let Some(dummy) = arena
                .north_south_port_dummies
                .iter()
                .find(|dummy| dummy.origin_port == port_id)
            else {
                continue;
            };
            sum +=
                north_south_port_barycenter(arena, port_id, dummy, absurdly_large, node_positions);
        } else {
            let mut in_layer = false;
            for edge_id in &port.outgoing_edges {
                let edge = &arena.edges[*edge_id as usize];
                if !edge.active {
                    continue;
                }
                let Some(connected_port) = edge.target_port else {
                    continue;
                };
                let connected_node = arena.ports[connected_port as usize].node;
                if arena.nodes[connected_node as usize].layer == owner_layer {
                    in_layer_ports.push(port_id);
                    in_layer = true;
                    break;
                }
                sum += port_ranks
                    .get(connected_port as usize)
                    .copied()
                    .unwrap_or_default();
            }
            if in_layer {
                continue;
            }
            for edge_id in &port.incoming_edges {
                let edge = &arena.edges[*edge_id as usize];
                if !edge.active {
                    continue;
                }
                let Some(connected_port) = edge.source_port else {
                    continue;
                };
                let connected_node = arena.ports[connected_port as usize].node;
                if arena.nodes[connected_node as usize].layer == owner_layer {
                    in_layer_ports.push(port_id);
                    in_layer = true;
                    break;
                }
                sum -= port_ranks
                    .get(connected_port as usize)
                    .copied()
                    .unwrap_or_default();
            }
            if in_layer {
                continue;
            }
        }

        let degree = port
            .incoming_edges
            .iter()
            .chain(&port.outgoing_edges)
            .filter(|edge_id| arena.edges[**edge_id as usize].active)
            .count();
        if degree > 0 {
            let barycenter = sum / degree as f32;
            port_barycenters[port_id as usize] = barycenter;
            minimum_barycenter = minimum_barycenter.min(barycenter);
            maximum_barycenter = maximum_barycenter.max(barycenter);
        } else if north_south {
            port_barycenters[port_id as usize] = sum;
        }
    }

    let Some(node_position) = node_positions.get(&node_id).copied() else {
        return;
    };
    for port_id in in_layer_ports {
        let port = &arena.ports[port_id as usize];
        let mut sum = 0_usize;
        let mut connections = 0_usize;
        for edge_id in port
            .incoming_edges
            .iter()
            .chain(&port.outgoing_edges)
            .copied()
            .filter(|edge_id| arena.edges[*edge_id as usize].active)
        {
            let edge = &arena.edges[edge_id as usize];
            let connected_port = if edge.source_port == Some(port_id) {
                edge.target_port
            } else {
                edge.source_port
            };
            let Some(connected_port) = connected_port else {
                continue;
            };
            let connected_node = arena.ports[connected_port as usize].node;
            if arena.nodes[connected_node as usize].layer == owner_layer {
                sum += node_positions.get(&connected_node).copied().unwrap_or(0) + 1;
                connections += 1;
            }
        }
        if connections == 0 {
            continue;
        }
        let barycenter = sum as f32 / connections as f32;
        let node_position = (node_position + 1) as f32;
        let layer_size = (layer_size + 1) as f32;
        port_barycenters[port_id as usize] = match port.side {
            PortSide::East if barycenter < node_position => minimum_barycenter - barycenter,
            PortSide::East => maximum_barycenter + layer_size - barycenter,
            PortSide::West if barycenter < node_position => maximum_barycenter + barycenter,
            PortSide::West => minimum_barycenter - (layer_size - barycenter),
            _ => port_barycenters[port_id as usize],
        };
    }
}

fn north_south_port_barycenter(
    arena: &LayeredArena,
    port_id: LPortId,
    dummy: &NorthSouthPortDummy,
    absurdly_large: f32,
    node_positions: &BTreeMap<LNodeId, usize>,
) -> f32 {
    let input = dummy.input_port.is_some_and(|port| {
        arena.ports[port as usize]
            .incoming_edges
            .iter()
            .any(|edge| arena.edges[*edge as usize].active)
    });
    let output = dummy.output_port.is_some_and(|port| {
        arena.ports[port as usize]
            .outgoing_edges
            .iter()
            .any(|edge| arena.edges[*edge as usize].active)
    });
    let position = node_positions.get(&dummy.node).copied().unwrap_or_default() as f32;
    match (input, output, arena.ports[port_id as usize].side) {
        (true, false, PortSide::North) => -position,
        (true, false, PortSide::South) => absurdly_large - position,
        (false, true, _) => position + 1.0,
        (true, true, PortSide::South) => absurdly_large / 2.0,
        _ => 0.0,
    }
}

/// Runs the hierarchy sweep's first randomized ordering boundary.
///
/// ELK's hierarchy-aware provider gives each child graph its own
/// `java.util.Random` seeded from `elk.randomSeed`.  The first sweep assigns
/// random barycenters to the boundary rank, then ordinary barycenters carry
/// that order through disconnected chains.  Keeping this opt-in preserves
/// the deterministic primitive above for callers that are comparing only the
/// arithmetic sweep.
pub fn order_layers_by_barycenter_with_random_seed(
    arena: &mut LayeredArena,
    graph_id: LGraphId,
    seed: i64,
) -> Result<(), LayeringError> {
    #[cfg(feature = "diagnostic-traces")]
    crate::trace::emit_marker("ordering", "p3_hierarchy_begin");
    #[cfg(feature = "diagnostic-traces")]
    crate::trace::emit_graph_scope("ordering", "barycenter_scope_start", graph_id, arena);
    #[cfg(feature = "diagnostic-traces")]
    crate::trace::emit_value(
        "ordering",
        "p3_graph_sweep_start",
        &format!("graphs[{graph_id}]"),
        &format!("arena_graph={graph_id}"),
    );
    let nested = arena
        .graphs
        .get(graph_id as usize)
        .is_some_and(|graph| graph.parent_node.is_some());
    let nested_edge_free = nested
        && !arena.edges.iter().any(|edge| {
            edge.active
                && arena.nodes[edge.source as usize].graph == graph_id
                && arena.nodes[edge.target as usize].graph == graph_id
        });
    let mut layout_random = arena.ordering_random(graph_id, seed, "layout-seed");
    let sweep_seed = layout_random.next_long();
    // GraphInfoHolder selects the root port-rank distributor from this same
    // graph RNG immediately after the minimizer captures its sweep seed.
    let root_node_relative_port_ranks = layout_random.next_boolean();
    let mut sweep_random = JavaRandom::with_stream(sweep_seed, "sweep-order");
    let mut child_random = JavaRandom::with_stream(seed, "child-order");
    // Nested graph holders can carry a separately seeded graph RNG; the root
    // shares the minimizer RNG and therefore uses the post-NextLong draw above.
    let node_relative_port_ranks = if nested {
        child_random.next_boolean()
    } else {
        root_node_relative_port_ranks
    };
    if !nested {
        let thoroughness = arena.graphs[graph_id as usize].thoroughness;
        layout_random.set_seed(sweep_seed);
        let result = minimize_randomized_barycenter_scope_with_random(
            arena,
            graph_id,
            &mut layout_random,
            node_relative_port_ranks,
            thoroughness,
            false,
        );
        arena.restore_graph_random(graph_id, layout_random);
        if result.is_ok() {
            arena.shared_graph_randoms.insert(graph_id);
        }
        #[cfg(feature = "diagnostic-traces")]
        crate::trace::emit_marker("ordering", "p3_hierarchy_end");
        return result;
    }
    let has_long_edge_dummies = arena
        .nodes
        .iter()
        .any(|node| node.graph == graph_id && node.node_type == LayeredNodeType::LongEdge);
    let randomize = has_long_edge_dummies || nested_edge_free;
    let hierarchy_boundary = arena.edges.iter().any(|edge| {
        edge.active
            && (arena
                .nodes
                .get(edge.source as usize)
                .is_some_and(|node| node.graph != graph_id)
                || arena
                    .nodes
                    .get(edge.target as usize)
                    .is_some_and(|node| node.graph != graph_id))
    });
    let forward = if has_long_edge_dummies {
        sweep_random.next_boolean()
    } else {
        hierarchy_boundary
    };
    let random = if randomize {
        if nested {
            Some(&mut child_random)
        } else {
            Some(&mut sweep_random)
        }
    } else {
        None
    };
    let randomize_boundary = random.is_some();
    let result = order_layers_by_barycenter_impl(
        arena,
        graph_id,
        BarycenterSweepOptions {
            forward,
            random,
            preordered: false,
            node_relative_port_ranks,
            use_source_port_ranks: true,
            randomize_boundary,
            hierarchy: None,
            hierarchy_first_sweep: None,
        },
    );
    #[cfg(feature = "diagnostic-traces")]
    crate::trace::emit_marker("ordering", "p3_hierarchy_end");
    result
}

#[derive(Clone)]
struct P3OrderSnapshot {
    layers: Vec<(LayerId, Vec<LNodeId>)>,
    ports: Vec<(LNodeId, Vec<LPortId>)>,
}

fn capture_p3_order(arena: &LayeredArena, graph_id: LGraphId) -> P3OrderSnapshot {
    let layers = arena.graphs[graph_id as usize]
        .layers
        .iter()
        .map(|layer_id| (*layer_id, arena.layers[*layer_id as usize].nodes.clone()))
        .collect::<Vec<_>>();
    let members = layers
        .iter()
        .flat_map(|(_, nodes)| nodes.iter().copied())
        .collect::<BTreeSet<_>>();
    P3OrderSnapshot {
        layers,
        ports: arena
            .nodes
            .iter()
            .enumerate()
            .filter(|(id, _)| members.contains(&(*id as LNodeId)))
            .map(|(node_id, node)| (node_id as LNodeId, node.ports.clone()))
            .collect(),
    }
}

fn restore_p3_order(arena: &mut LayeredArena, snapshot: &P3OrderSnapshot, freeze_ports: bool) {
    for (layer_id, nodes) in &snapshot.layers {
        arena.layers[*layer_id as usize].nodes.clone_from(nodes);
    }
    for (node_id, ports) in &snapshot.ports {
        let node = &mut arena.nodes[*node_id as usize];
        node.ports.clone_from(ports);
        if freeze_ports
            && !matches!(
                node.port_constraints,
                PortConstraints::FixedOrder
                    | PortConstraints::FixedRatio
                    | PortConstraints::FixedPosition
            )
        {
            // SweepCopy.TransferNodeAndPortOrdersToGraph(..., true).
            node.port_constraints = PortConstraints::FixedOrder;
        }
    }
    // SweepCopy.assertCorrectPortSides uses the saved dummy/owner positions.
    // A permitted proxy crossing changes the original non-flow port's side.
    let mut update = Vec::new();
    for (layer_id, nodes) in &snapshot.layers {
        for dummy in nodes {
            let Some(record) = arena
                .north_south_port_dummies
                .iter()
                .find(|record| record.node == *dummy)
            else {
                continue;
            };
            let port_id = record.origin_port;
            let owner = arena.ports[port_id as usize].node;
            let Some(owner_position) = nodes.iter().position(|node| *node == owner) else {
                continue;
            };
            let dummy_position = arena.layers[*layer_id as usize]
                .nodes
                .iter()
                .position(|node| node == dummy)
                .unwrap();
            let port = &mut arena.ports[port_id as usize];
            if port.side == PortSide::North && dummy_position > owner_position {
                port.side = PortSide::South;
            } else if port.side == PortSide::South && owner_position > dummy_position {
                port.side = PortSide::North;
            }
            for node in [owner, *dummy] {
                if !update.contains(&node) {
                    update.push(node);
                }
            }
        }
    }
    for node_id in update {
        let constraints = arena.nodes[node_id as usize].port_constraints;
        let mut ports = arena.nodes[node_id as usize].ports.clone();
        ports.sort_by(|a, b| {
            let a = &arena.ports[*a as usize];
            let b = &arena.ports[*b as usize];
            port_side_order(a.side)
                .cmp(&port_side_order(b.side))
                .then_with(|| {
                    if constraints == PortConstraints::FixedOrder
                        && let (Some(a), Some(b)) = (a.index, b.index)
                        && a != b
                    {
                        return a.cmp(&b);
                    }
                    let (a, b) = match a.side {
                        PortSide::North => (a.x, b.x),
                        PortSide::East => (a.y, b.y),
                        PortSide::South => (-a.x, -b.x),
                        PortSide::West => (-a.y, -b.y),
                        PortSide::Undefined => return Ordering::Equal,
                    };
                    a.partial_cmp(&b).unwrap_or(Ordering::Equal)
                })
        });
        arena.nodes[node_id as usize].ports = ports;
    }
}

fn minimize_randomized_barycenter_scope_with_random(
    arena: &mut LayeredArena,
    graph_id: LGraphId,
    random: &mut JavaRandom,
    node_relative_port_ranks: bool,
    thoroughness: usize,
    consider_model_order: bool,
) -> Result<(), LayeringError> {
    arena
        .source_forster_constraints
        .insert(graph_id, forster::Constraints::from_graph(arena, graph_id));
    let mut best_crossings = usize::MAX;
    let mut best_order = None;
    let mut did_sweep = false;
    for _trial_index in 0..thoroughness {
        let random_direction = random.next_boolean();
        let initial_model_order = consider_model_order && _trial_index == 0;
        let mut forward = if initial_model_order {
            true
        } else {
            random_direction
        };
        // Source FirstTry/SecondTry share one property key; clearing the
        // second flag clears the first. Only the first trial keeps input order.
        if initial_model_order && count_p3_all_crossings(arena, graph_id) == 0 {
            best_order = Some(capture_p3_order(arena, graph_id));
            break;
        }
        order_layers_by_barycenter_impl(
            arena,
            graph_id,
            BarycenterSweepOptions {
                forward,
                random: Some(random),
                preordered: initial_model_order,
                node_relative_port_ranks,
                use_source_port_ranks: true,
                randomize_boundary: !initial_model_order,
                hierarchy: None,
                hierarchy_first_sweep: None,
            },
        )?;

        did_sweep = true;
        let mut crossings = count_p3_all_crossings(arena, graph_id);
        #[cfg(feature = "diagnostic-traces")]
        crate::trace::emit_value(
            "ordering",
            "p3_crossing_count",
            &format!("graphs[{graph_id}]/trials[{_trial_index}]/first_sweep"),
            &crossings.to_string(),
        );
        let mut trial_best_crossings = crossings;
        let mut trial_best_order = capture_p3_order(arena, graph_id);
        while crossings > 0 {
            let old_crossings = crossings;
            forward = !forward;
            order_layers_by_barycenter_impl(
                arena,
                graph_id,
                BarycenterSweepOptions {
                    forward,
                    random: Some(random),
                    preordered: true,
                    node_relative_port_ranks,
                    use_source_port_ranks: true,
                    randomize_boundary: false,
                    hierarchy: None,
                    hierarchy_first_sweep: None,
                },
            )?;
            crossings = count_p3_all_crossings(arena, graph_id);
            #[cfg(feature = "diagnostic-traces")]
            crate::trace::emit_value(
                "ordering",
                "p3_crossing_count",
                &format!("graphs[{graph_id}]/trials[{_trial_index}]/reversed_sweep"),
                &crossings.to_string(),
            );
            if crossings < trial_best_crossings {
                trial_best_crossings = crossings;
                trial_best_order = capture_p3_order(arena, graph_id);
            }
            if old_crossings <= crossings {
                break;
            }
        }
        if trial_best_crossings < best_crossings {
            best_crossings = trial_best_crossings;
            best_order = Some(trial_best_order);
        }
        #[cfg(feature = "diagnostic-traces")]
        crate::trace::emit_value(
            "ordering",
            "p3_trial",
            &format!("graphs[{graph_id}]/trials[{_trial_index}]"),
            &format!("trial_crossings={trial_best_crossings};best_crossings={best_crossings}"),
        );
        if best_crossings == 0 {
            break;
        }
    }
    if let Some(best_order) = best_order {
        restore_p3_order(arena, &best_order, did_sweep);
    }
    Ok(())
}

fn order_flat_component_with_shared_random(
    arena: &mut LayeredArena,
    graph_id: LGraphId,
    random: &mut JavaRandom,
    consider_model_order: bool,
) -> Result<(), LayeringError> {
    #[cfg(feature = "diagnostic-traces")]
    let thoroughness = arena.graphs[graph_id as usize].thoroughness;
    #[cfg(feature = "diagnostic-traces")]
    crate::trace::emit_value(
        "ordering",
        "p3_config",
        "pipeline",
        &format!("thoroughness={thoroughness};cross_min_type=0"),
    );
    #[cfg(feature = "diagnostic-traces")]
    crate::trace::emit_value("ordering", "p3_phase", "pipeline", "begin");
    let sweep_seed = random.next_long();
    let node_relative_port_ranks = random.next_boolean();
    #[cfg(feature = "diagnostic-traces")]
    crate::trace::emit_graph_scope("ordering", "barycenter_scope_start", graph_id, arena);
    #[cfg(feature = "diagnostic-traces")]
    crate::trace::emit_value(
        "ordering",
        "p3_graph_sweep_start",
        "graphs[0]",
        &format!("arena_graph={graph_id}"),
    );
    random.set_seed(sweep_seed);
    let thoroughness = arena.graphs[graph_id as usize].thoroughness;
    let result = minimize_randomized_barycenter_scope_with_random(
        arena,
        graph_id,
        random,
        node_relative_port_ranks,
        thoroughness,
        consider_model_order,
    );
    #[cfg(feature = "diagnostic-traces")]
    crate::trace::emit_value("ordering", "p3_phase", "pipeline", "end");
    if result.is_ok() {
        run_greedy_switch_component(arena, graph_id, random);
    }
    result
}

fn run_greedy_switch_component(
    arena: &mut LayeredArena,
    graph_id: LGraphId,
    random: &mut JavaRandom,
) {
    let graph = &arena.graphs[graph_id as usize];
    let mode = graph.greedy_switch;
    let node_count = graph.layerless_nodes.len();
    let threshold = graph.greedy_switch_activation_threshold;
    #[cfg(feature = "diagnostic-traces")]
    let thoroughness = graph.thoroughness;
    if mode == GreedySwitchMode::Off
        || graph.semi_interactive
        || graph.interactive_crossing_minimization
        || (threshold != 0 && threshold <= node_count)
        || graph.layers.len() < 2
    {
        return;
    }

    // GraphConfigurator inserts this second P3 minimizer before node
    // placement. It initializes its own holder and consumes the shared graph
    // RNG even when no adjacent nodes need to move.
    #[cfg(feature = "diagnostic-traces")]
    crate::trace::emit_value(
        "ordering",
        "p3_config",
        "pipeline",
        &format!(
            "thoroughness={};cross_min_type={}",
            thoroughness,
            match mode {
                GreedySwitchMode::OneSided => 1,
                GreedySwitchMode::TwoSided => 2,
                GreedySwitchMode::Off => 0,
            }
        ),
    );
    #[cfg(feature = "diagnostic-traces")]
    crate::trace::emit_value("ordering", "p3_phase", "pipeline", "begin");
    let _sweep_seed = random.next_long();
    #[cfg(feature = "diagnostic-traces")]
    crate::trace::emit_graph_scope("ordering", "barycenter_scope_start", graph_id, arena);
    #[cfg(feature = "diagnostic-traces")]
    crate::trace::emit_value(
        "ordering",
        "p3_graph_sweep_start",
        "graphs[0]",
        &format!("arena_graph={graph_id}"),
    );
    let mut port_distributor = greedy_port_distributor::GreedyPortDistributor::new(arena);
    let mut forward = random.next_boolean();
    loop {
        let mut improved = false;
        let layer_ids = arena.graphs[graph_id as usize].layers.clone();
        let boundary = if forward { 0 } else { layer_ids.len() - 1 };
        improved |= greedy_switch_layer(arena, graph_id, layer_ids[boundary], mode, None);
        if mode == GreedySwitchMode::TwoSided {
            improved |= port_distributor.distribute(arena, graph_id, boundary, forward);
        }
        let indices: Box<dyn Iterator<Item = usize>> = if forward {
            Box::new(1..layer_ids.len())
        } else {
            Box::new((0..layer_ids.len() - 1).rev())
        };
        for index in indices {
            loop {
                let changed =
                    greedy_switch_layer(arena, graph_id, layer_ids[index], mode, Some(forward));
                improved |= changed;
                if !changed {
                    break;
                }
            }
            if mode == GreedySwitchMode::TwoSided {
                improved |= port_distributor.distribute(arena, graph_id, index, forward);
            }
        }
        if !improved {
            break;
        }
        forward = !forward;
    }
    let published = capture_p3_order(arena, graph_id);
    restore_p3_order(arena, &published, true);
    #[cfg(feature = "diagnostic-traces")]
    crate::trace::emit_value("ordering", "p3_phase", "pipeline", "end");
}

/// Makes one stable adjacent-node scan, accepting only strict crossing
/// reductions as the upstream SwitchDecider does.
fn greedy_switch_layer(
    arena: &mut LayeredArena,
    graph_id: LGraphId,
    layer_id: LayerId,
    mode: GreedySwitchMode,
    one_sided_direction: Option<bool>,
) -> bool {
    let mut changed = false;
    let mut upper = 0;
    loop {
        let nodes = arena.layers[layer_id as usize].nodes.clone();
        if upper + 1 >= nodes.len() {
            break;
        }
        let _first = nodes[upper];
        let _second = nodes[upper + 1];
        let before =
            count_p3_switch_crossings(arena, graph_id, layer_id, mode, one_sided_direction);
        arena.layers[layer_id as usize].nodes.swap(upper, upper + 1);
        let after = count_p3_switch_crossings(arena, graph_id, layer_id, mode, one_sided_direction);
        if after < before {
            changed = true;
            #[cfg(feature = "diagnostic-traces")]
            crate::trace::emit_value(
                "ordering",
                "p3_greedy_switch_accept",
                &format!("graphs[{graph_id}].layers[{layer_id}].nodes[{upper}]"),
                &format!("first={_first};second={_second};before={before};after={after}"),
            );
        } else {
            arena.layers[layer_id as usize].nodes.swap(upper, upper + 1);
            #[cfg(feature = "diagnostic-traces")]
            crate::trace::emit_value(
                "ordering",
                "p3_greedy_switch_reject",
                &format!("graphs[{graph_id}].layers[{layer_id}].nodes[{upper}]"),
                &format!("first={_first};second={_second};before={before};after={after}"),
            );
        }
        upper += 1;
    }
    changed
}

fn count_p3_switch_crossings(
    arena: &LayeredArena,
    graph_id: LGraphId,
    layer_id: LayerId,
    mode: GreedySwitchMode,
    one_sided_direction: Option<bool>,
) -> usize {
    if mode == GreedySwitchMode::TwoSided {
        return count_p3_inter_layer_crossings(arena, graph_id);
    }
    let layers = &arena.graphs[graph_id as usize].layers;
    let Some(layer_index) = layers.iter().position(|candidate| *candidate == layer_id) else {
        return 0;
    };
    let boundary = match one_sided_direction {
        Some(true) => layer_index.checked_sub(1),
        Some(false) if layer_index + 1 < layers.len() => Some(layer_index),
        _ => None,
    };
    boundary.map_or(0, |boundary| {
        count_p3_inter_layer_crossings_between(arena, graph_id, Some(boundary))
    })
}

/// Runs the v0.9.0 first model-order sweep for a flat D2 graph.
///
/// D2 configures `NODES_AND_EDGES`. ELK preserves a crossing-free model order;
/// otherwise it uses the source port distributor, resets the shared Java RNG
/// to the crossing-minimizer seed, and performs its first sweep forward.
pub fn order_layers_by_barycenter_with_model_order(
    arena: &mut LayeredArena,
    graph_id: LGraphId,
    seed: i64,
    consider_model_order: bool,
) -> Result<(), LayeringError> {
    let Some(graph) = arena.graphs.get(graph_id as usize) else {
        return Err(LayeringError::UnknownGraph(graph_id));
    };
    if graph.parent_node.is_some() {
        return order_layers_by_barycenter_with_random_seed(arena, graph_id, seed);
    }

    let has_nested_graphs = arena
        .nodes
        .iter()
        .any(|node| node.graph == graph_id && node.nested_graph.is_some());

    if has_nested_graphs {
        let plan = p3_hierarchy_plan(arena, graph_id);
        return order_hierarchical_layers_by_barycenter(
            arena,
            graph_id,
            seed,
            consider_model_order,
            plan,
        );
    }

    if !consider_model_order {
        return order_layers_by_barycenter_with_random_seed(arena, graph_id, seed);
    }

    #[cfg(feature = "diagnostic-traces")]
    crate::trace::emit_graph_scope("ordering", "barycenter_scope_start", graph_id, arena);

    let mut random = arena.take_graph_random(graph_id);
    let sweep_seed = random.next_long();
    // GraphInfoHolder selects node-relative ranks when this draw is true.
    let node_relative_port_ranks = random.next_boolean();
    // LayerSweepCrossingMinimizer resets the shared stream once after
    // initialize() captures its seed. The randomized trial consumes that
    // restored stream directly; a second reset is output-neutral but changes
    // the source-visible RNG operation trace.
    random.set_seed(sweep_seed);
    let thoroughness = arena.graphs[graph_id as usize].thoroughness;
    let result = minimize_randomized_barycenter_scope_with_random(
        arena,
        graph_id,
        &mut random,
        node_relative_port_ranks,
        thoroughness,
        true,
    );
    arena.restore_graph_random(graph_id, random);
    if result.is_ok() {
        arena.shared_graph_randoms.insert(graph_id);
    }
    result
}

/// ComponentsProcessor.Split / provider.runComponents: finish a complete
/// flat component pipeline before advancing the shared graph Random object.
/// Hierarchy and external-port scopes retain their dedicated scheduler.
pub(crate) fn run_flat_component_pipelines<F>(
    arena: &mut LayeredArena,
    graph_id: LGraphId,
    mut processor: F,
) -> Result<(), LayeringError>
where
    F: FnMut(&mut LayeredArena, LGraphId) -> Result<(), LayeringError>,
{
    let graph = &arena.graphs[graph_id as usize];
    if graph.includes_children
        || !graph.separate_components
        || graph.parent_node.is_some()
        || graph.layerless_nodes.iter().any(|id| {
            let node = &arena.nodes[*id as usize];
            node.nested_graph.is_some() || node.node_type == LayeredNodeType::ExternalPort
        })
    {
        return processor(arena, graph_id);
    }
    let template = graph.clone();
    let nodes = template.layerless_nodes.clone();
    let members = nodes.iter().copied().collect::<BTreeSet<_>>();
    let edges = arena
        .edges
        .iter()
        .enumerate()
        .filter_map(|(id, edge)| {
            (edge.active && members.contains(&edge.source) && members.contains(&edge.target))
                .then_some(id as LEdgeId)
        })
        .collect::<Vec<_>>();
    let mut components = source_layering_connected_components(arena, &nodes, &edges);
    let positions = nodes
        .iter()
        .enumerate()
        .map(|(i, id)| (*id, i))
        .collect::<BTreeMap<_, _>>();
    components.sort_by_key(|component| component.iter().map(|id| positions[id]).min());
    let mut combined_layers = Vec::new();
    for component in components {
        let scope = arena.graphs.len() as LGraphId;
        let mut graph = template.clone();
        graph.layerless_nodes = component;
        graph.separated_component = true;
        graph.layers.clear();
        graph.width = 0.0;
        graph.height = 0.0;
        graph.offset_y = 0.0;
        arena.graphs.push(graph);
        // This property lives outside LGraph, but participates in the source
        // Properties.Clone just like spacing and ordering configuration.
        if let Some(mode) = arena.edge_label_side_modes.get(&graph_id).copied() {
            arena.edge_label_side_modes.insert(scope, mode);
        }
        for id in arena.graphs[scope as usize].layerless_nodes.clone() {
            arena.nodes[id as usize].graph = scope;
        }
        let random = arena.take_graph_random(graph_id);
        arena.restore_graph_random(scope, random);
        arena.shared_graph_randoms.insert(scope);
        let result = processor(arena, scope);
        let random = arena.take_graph_random(scope);
        arena.restore_graph_random(graph_id, random);
        for node in &mut arena.nodes {
            if node.graph == scope {
                node.graph = graph_id;
            }
        }
        for id in arena.graphs[scope as usize].layers.clone() {
            arena.layers[id as usize].graph = graph_id;
            combined_layers.push(id);
        }
        result?;
    }
    arena.graphs[graph_id as usize].layers = combined_layers;
    Ok(())
}

/// Runs one processor over elk-go's non-hierarchical connected-component
/// scopes. ELK splits before P1-P5 and combines the completed components only
/// after routing, so sharing a root layer here can leak ordering, placement,
/// and obstacle state between disconnected subgraphs.
pub fn for_each_separate_component<F>(
    arena: &mut LayeredArena,
    graph_id: LGraphId,
    mut processor: F,
) -> Result<(), LayeringError>
where
    F: FnMut(&mut LayeredArena, LGraphId) -> Result<(), LayeringError>,
{
    let Some(graph) = arena.graphs.get(graph_id as usize) else {
        return Err(LayeringError::UnknownGraph(graph_id));
    };
    let graph_has_children = arena
        .nodes
        .iter()
        .any(|node| node.graph == graph_id && node.nested_graph.is_some());
    let has_external_ports = arena
        .nodes
        .iter()
        .any(|node| node.graph == graph_id && node.node_type == LayeredNodeType::ExternalPort);
    // Source component membership is fixed before P1. Port preprocessing
    // can detach all ordinary edges from a host while its dummy associations
    // still belong to the same component; do not split that scope again.
    if graph.separated_component
        || graph.parent_node.is_some()
        || graph_has_children
        || has_external_ports
        || !graph.separate_components
    {
        return processor(arena, graph_id);
    }

    let original_layer_ids = graph.layers.clone();
    let original_layerless_nodes = graph.layerless_nodes.clone();
    let original_graph_geometry = (graph.width, graph.height, graph.offset_y);
    let original_arena_layer_len = arena.layers.len();
    let original_layers = original_layer_ids
        .iter()
        .map(|layer_id| (*layer_id, arena.layers[*layer_id as usize].nodes.clone()))
        .collect::<Vec<_>>();
    let nodes = original_layers
        .iter()
        .flat_map(|(_, nodes)| nodes.iter().copied())
        .collect::<Vec<_>>();
    let node_set = nodes.iter().copied().collect::<BTreeSet<_>>();
    let edge_ids = arena
        .edges
        .iter()
        .enumerate()
        .filter_map(|(edge_id, edge)| {
            (edge.active && node_set.contains(&edge.source) && node_set.contains(&edge.target))
                .then_some(edge_id as LEdgeId)
        })
        .collect::<Vec<_>>();
    let mut components = source_layering_connected_components(arena, &nodes, &edge_ids);
    let model_positions = original_layerless_nodes
        .iter()
        .enumerate()
        .map(|(position, node_id)| (*node_id, position))
        .collect::<BTreeMap<_, _>>();
    // ComponentsProcessor.Split discovers components from LayerlessNodes in
    // model order. P2's shared-root solver stores the same membership in
    // largest-first order, so restore the source split order here.
    components.sort_by_key(|component| {
        component
            .iter()
            .filter_map(|node_id| model_positions.get(node_id))
            .copied()
            .min()
            .unwrap_or(usize::MAX)
    });
    if components.len() <= 1 {
        return processor(arena, graph_id);
    }

    let original_edge_activity = arena
        .edges
        .iter()
        .map(|edge| edge.active)
        .collect::<Vec<_>>();
    let mut ordered_component_layers = Vec::<(LayerId, Vec<LNodeId>)>::new();
    let mut first_error = None;
    for component in components {
        let component_set = component.iter().copied().collect::<BTreeSet<_>>();
        let mut component_layer_ids = Vec::new();
        let mut layer_origins = Vec::new();
        for (original_layer_id, original_nodes) in &original_layers {
            let component_nodes = original_nodes
                .iter()
                .copied()
                .filter(|node_id| component_set.contains(node_id))
                .collect::<Vec<_>>();
            if component_nodes.is_empty() {
                continue;
            }
            let original_layer = &arena.layers[*original_layer_id as usize];
            let component_layer_id = arena.layers.len() as LayerId;
            arena.layers.push(Layer {
                graph: graph_id,
                nodes: component_nodes.clone(),
                height: original_layer.height,
                width: original_layer.width,
            });
            for node_id in &component_nodes {
                arena.nodes[*node_id as usize].layer = Some(component_layer_id);
            }
            component_layer_ids.push(component_layer_id);
            layer_origins.push((*original_layer_id, component_layer_id));
        }

        arena.graphs[graph_id as usize]
            .layers
            .clone_from(&component_layer_ids);
        arena.graphs[graph_id as usize]
            .layerless_nodes
            .clone_from(&component);
        {
            let graph = &mut arena.graphs[graph_id as usize];
            (graph.width, graph.height, graph.offset_y) = original_graph_geometry;
        }
        for (edge_id, edge) in arena.edges.iter_mut().enumerate() {
            edge.active = original_edge_activity[edge_id]
                && component_set.contains(&edge.source)
                && component_set.contains(&edge.target);
        }
        let result = processor(arena, graph_id);
        for (original_layer_id, component_layer_id) in layer_origins {
            ordered_component_layers.push((
                original_layer_id,
                arena.layers[component_layer_id as usize].nodes.clone(),
            ));
        }
        for (original_layer_id, original_nodes) in &original_layers {
            for node_id in original_nodes {
                if component_set.contains(node_id) {
                    arena.nodes[*node_id as usize].layer = Some(*original_layer_id);
                }
            }
        }
        arena.layers.truncate(original_arena_layer_len);
        arena.graphs[graph_id as usize]
            .layers
            .clone_from(&original_layer_ids);
        arena.graphs[graph_id as usize]
            .layerless_nodes
            .clone_from(&original_layerless_nodes);
        {
            let graph = &mut arena.graphs[graph_id as usize];
            (graph.width, graph.height, graph.offset_y) = original_graph_geometry;
        }
        for (edge_id, edge) in arena.edges.iter_mut().enumerate() {
            edge.active = original_edge_activity[edge_id];
        }
        if first_error.is_none() {
            first_error = result.err();
        }
    }

    // Merge each independent component's new order into its original layer,
    // replacing only the slots owned by that component so unrelated
    // components retain the source model ordering until the packer runs.
    for (original_layer_id, ordered_nodes) in ordered_component_layers {
        let ordered_set = ordered_nodes.iter().copied().collect::<BTreeSet<_>>();
        let mut ordered = ordered_nodes.into_iter();
        for node_id in &mut arena.layers[original_layer_id as usize].nodes {
            if ordered_set.contains(node_id)
                && let Some(next) = ordered.next()
            {
                *node_id = next;
            }
        }
    }
    if let Some(error) = first_error {
        Err(error)
    } else {
        Ok(())
    }
}

/// Runs phase-three ordering independently in each non-hierarchical component.
pub fn order_layers_by_barycenter_with_components(
    arena: &mut LayeredArena,
    graph_id: LGraphId,
    seed: i64,
    consider_model_order: bool,
) -> Result<(), LayeringError> {
    let mut random = arena.ordering_random(graph_id, seed, "elk-components");
    let result = for_each_separate_component(arena, graph_id, |arena, component_graph| {
        let graph = &arena.graphs[component_graph as usize];
        let single_node =
            graph.layers.len() == 1 && arena.layers[graph.layers[0] as usize].nodes.len() == 1;
        if single_node {
            // ComponentsProcessor's P3 phase returns before creating its
            // random stream for a singleton component.
            return Ok(());
        }
        order_flat_component_with_shared_random(
            arena,
            component_graph,
            &mut random,
            consider_model_order,
        )
    });
    arena.restore_graph_random(graph_id, random);
    result
}

struct BarycenterSweepOptions<'a> {
    forward: bool,
    random: Option<&'a mut JavaRandom>,
    preordered: bool,
    node_relative_port_ranks: bool,
    use_source_port_ranks: bool,
    randomize_boundary: bool,
    hierarchy: Option<&'a mut BarycenterHierarchyContext>,
    hierarchy_first_sweep: Option<bool>,
}

struct BarycenterHierarchyContext {
    graph_indices: BTreeMap<LGraphId, u32>,
    use_bottom_up: BTreeMap<LGraphId, bool>,
    node_relative_port_ranks: BTreeMap<LGraphId, bool>,
    randoms: BTreeMap<LGraphId, Option<JavaRandom>>,
}

fn order_hierarchical_layers_by_barycenter(
    arena: &mut LayeredArena,
    root: LGraphId,
    seed: i64,
    consider_model_order: bool,
    plan: P3HierarchyPlan,
) -> Result<(), LayeringError> {
    #[cfg(feature = "diagnostic-traces")]
    crate::trace::emit_marker("ordering", "p3_hierarchy_begin");
    #[cfg(feature = "diagnostic-traces")]
    crate::trace::emit_graph_scope("ordering", "barycenter_scope_start", root, arena);

    for graph in &plan.breadth_first_graphs {
        arena
            .source_forster_constraints
            .insert(*graph, forster::Constraints::from_graph(arena, *graph));
    }
    let mut root_random = arena.ordering_random(root, seed, "elk-root");
    let sweep_seed = root_random.next_long();
    let root_node_relative_port_ranks = root_random.next_boolean();
    let mut context = BarycenterHierarchyContext {
        graph_indices: BTreeMap::new(),
        use_bottom_up: BTreeMap::new(),
        node_relative_port_ranks: BTreeMap::new(),
        randoms: BTreeMap::new(),
    };

    for (index, graph_id) in plan.breadth_first_graphs.iter().copied().enumerate() {
        context.graph_indices.insert(graph_id, index as u32);
        context
            .use_bottom_up
            .insert(graph_id, plan.decisions[&graph_id].use_bottom_up);
        if graph_id == root {
            context
                .node_relative_port_ranks
                .insert(graph_id, root_node_relative_port_ranks);
            continue;
        }

        #[cfg(feature = "diagnostic-traces")]
        let stream = format!("elk-graph-{index}");
        #[cfg(not(feature = "diagnostic-traces"))]
        let stream = "elk-graph";
        let child_seed = arena.graphs[graph_id as usize].random_seed;
        // The traced stream name is a String; the untraced one is a &str.
        #[cfg_attr(not(feature = "diagnostic-traces"), allow(clippy::needless_borrow))]
        let mut random = arena.ordering_random(graph_id, child_seed, &stream);
        let node_relative_port_ranks = random.next_boolean();
        context
            .node_relative_port_ranks
            .insert(graph_id, node_relative_port_ranks);
        context.randoms.insert(graph_id, Some(random));

        #[cfg(feature = "diagnostic-traces")]
        crate::trace::emit_graph_scope("ordering", "p3_graph_holder_init", graph_id, arena);
        #[cfg(feature = "diagnostic-traces")]
        if let Some(parent_node) = arena.graphs[graph_id as usize].parent_node {
            let parent_ports = arena.nodes[parent_node as usize]
                .ports
                .iter()
                .map(|port_id| {
                    let port = &arena.ports[*port_id as usize];
                    format!(
                        "{}:{:?}",
                        port.source_id.as_deref().unwrap_or("<implicit>"),
                        port.side
                    )
                })
                .collect::<Vec<_>>()
                .join(",");
            let decision = plan.decisions[&graph_id];
            crate::trace::emit_value(
                "ordering",
                "p3_parent_ports",
                &format!("graphs[{}]", context.graph_indices[&graph_id]),
                &format!(
                    "ports={parent_ports};reason={};random={};hierarchical={}",
                    decision.reason, decision.paths_to_random, decision.paths_to_hierarchical
                ),
            );
        }
    }

    // elk-go resets the shared crossing-minimizer stream before each
    // independent graph trial. The graph holders keep their own Random
    // streams for layer randomization and barycenter perturbations.
    for graph_id in plan.graphs_to_sweep_on.iter().copied() {
        #[cfg(feature = "diagnostic-traces")]
        let graph_index = context
            .graph_indices
            .get(&graph_id)
            .copied()
            .unwrap_or(graph_id);
        #[cfg(feature = "diagnostic-traces")]
        crate::trace::emit_value(
            "ordering",
            "p3_graph_sweep_start",
            &format!("graphs[{graph_index}]"),
            &format!("arena_graph={graph_id}"),
        );
        root_random.set_seed(sweep_seed);
        let random_direction = root_random.next_boolean();
        let model_order = if graph_id == root {
            consider_model_order
        } else {
            arena.graphs[graph_id as usize].consider_model_order
        };
        let source_crossings = if model_order {
            count_hierarchy_all_p3_crossings(arena, graph_id)
        } else {
            1
        };
        #[cfg(feature = "diagnostic-traces")]
        if model_order {
            crate::trace::emit_value(
                "ordering",
                "p3_model_order_crossing_guard",
                &format!(
                    "graphs[{}]",
                    context
                        .graph_indices
                        .get(&graph_id)
                        .copied()
                        .unwrap_or(graph_id)
                ),
                &format!("source_crossings={source_crossings}"),
            );
            let graph = &arena.graphs[graph_id as usize];
            let layer_state = graph
                .layers
                .iter()
                .enumerate()
                .map(|(layer_index, layer_id)| {
                    let nodes = &arena.layers[*layer_id as usize].nodes;
                    let node_state = nodes
                        .iter()
                        .map(|node_id| {
                            let node = &arena.nodes[*node_id as usize];
                            let ports = node
                                .ports
                                .iter()
                                .map(|port_id| {
                                    let port = &arena.ports[*port_id as usize];
                                    format!(
                                        "{}:{:?}:in={:?}:out={:?}",
                                        port_id,
                                        port.side,
                                        port.incoming_edges,
                                        port.outgoing_edges
                                    )
                                })
                                .collect::<Vec<_>>()
                                .join(",");
                            format!("{}({}):[{}]", node_id, node.source_id, ports)
                        })
                        .collect::<Vec<_>>()
                        .join(";");
                    format!("layer{layer_index}=[{node_state}]")
                })
                .collect::<Vec<_>>()
                .join(";");
            crate::trace::emit_value(
                "ordering",
                "p3_model_order_counter_graph",
                &format!(
                    "graphs[{}]",
                    context
                        .graph_indices
                        .get(&graph_id)
                        .copied()
                        .unwrap_or(graph_id)
                ),
                &layer_state,
            );
        }
        if model_order && source_crossings == 0 {
            continue;
        }
        let node_relative_port_ranks = context
            .node_relative_port_ranks
            .get(&graph_id)
            .copied()
            .unwrap_or(false);
        let mut graph_random = if graph_id == root {
            None
        } else {
            context.randoms.get_mut(&graph_id).and_then(Option::take)
        };
        let result = minimize_hierarchical_randomized_scope(
            arena,
            graph_id,
            &mut root_random,
            graph_random.as_mut(),
            node_relative_port_ranks,
            model_order,
            random_direction,
            &mut context,
        );
        if graph_id != root {
            context.randoms.insert(graph_id, graph_random);
        }
        result?;
        if graph_id != root {
            // The shared P3 sweep transfers port order only. The post-ordering
            // size processor must wait for bottom-up P4, after child resizes.
            copy_ordered_external_ports_to_parent(arena, graph_id)?;
        }
    }

    for (graph_id, random) in context.randoms {
        if let Some(random) = random {
            arena.restore_graph_random(graph_id, random);
            if arena.graphs[graph_id as usize].cycle_random_consumed {
                arena.shared_graph_randoms.insert(graph_id);
            }
        }
    }
    arena.restore_graph_random(root, root_random);
    if arena.graphs[root as usize].cycle_random_consumed {
        arena.shared_graph_randoms.insert(root);
    }
    #[cfg(feature = "diagnostic-traces")]
    {
        for graph_id in plan.breadth_first_graphs.iter().copied() {
            crate::trace::emit_graph_scope("ordering", "p3_graph_scope_end", graph_id, arena);
        }
        crate::trace::emit_marker("ordering", "p3_hierarchy_end");
    }
    Ok(())
}

// SweepCopy saves node/port orders for every graph changed by a scope's
// sweeps, including descendants swept top-down with that scope.
fn capture_hierarchy_p3_order(
    arena: &LayeredArena,
    graph_id: LGraphId,
    hierarchy: &BarycenterHierarchyContext,
) -> Vec<P3OrderSnapshot> {
    let mut snapshots = vec![capture_p3_order(arena, graph_id)];
    for node in arena.nodes.iter().filter(|node| node.graph == graph_id) {
        if let Some(child) = node.nested_graph
            && !hierarchy.use_bottom_up.get(&child).copied().unwrap_or(true)
        {
            snapshots.extend(capture_hierarchy_p3_order(arena, child, hierarchy));
        }
    }
    snapshots
}

fn minimize_hierarchical_randomized_scope(
    arena: &mut LayeredArena,
    graph_id: LGraphId,
    minimizer_random: &mut JavaRandom,
    mut graph_random: Option<&mut JavaRandom>,
    node_relative_port_ranks: bool,
    model_order: bool,
    initial_forward: bool,
    hierarchy: &mut BarycenterHierarchyContext,
) -> Result<(), LayeringError> {
    let mut best_crossings = usize::MAX;
    let mut did_sweep = false;
    let mut best_order = Vec::new();
    let thoroughness = arena.graphs[graph_id as usize].thoroughness;
    for trial in 0..thoroughness {
        // The minimizer and holder share the root RNG but children keep a
        // separate holder RNG. Direction draws always use the minimizer.
        let random_forward = if trial == 0 {
            initial_forward
        } else {
            minimizer_random.next_boolean()
        };
        let first_with_model_order = model_order && trial == 0;
        let mut forward = if first_with_model_order {
            true
        } else {
            random_forward
        };
        order_layers_by_barycenter_impl(
            arena,
            graph_id,
            BarycenterSweepOptions {
                forward,
                random: Some(
                    graph_random
                        .as_deref_mut()
                        .unwrap_or(&mut *minimizer_random),
                ),
                preordered: first_with_model_order,
                node_relative_port_ranks,
                use_source_port_ranks: true,
                randomize_boundary: !first_with_model_order,
                hierarchy: Some(hierarchy),
                hierarchy_first_sweep: Some(true),
            },
        )?;
        did_sweep = true;
        let mut crossings = count_hierarchy_all_p3_crossings(arena, graph_id);
        #[cfg(feature = "diagnostic-traces")]
        crate::trace::emit_value(
            "ordering",
            "p3_crossing_count",
            &format!("graphs[{graph_id}]/trials[{trial}]/first_sweep"),
            &crossings.to_string(),
        );
        let mut trial_best_crossings = crossings;
        let mut trial_best_order = capture_hierarchy_p3_order(arena, graph_id, hierarchy);
        while crossings > 0 {
            let old_crossings = crossings;
            forward = !forward;
            order_layers_by_barycenter_impl(
                arena,
                graph_id,
                BarycenterSweepOptions {
                    forward,
                    random: Some(
                        graph_random
                            .as_deref_mut()
                            .unwrap_or(&mut *minimizer_random),
                    ),
                    preordered: true,
                    node_relative_port_ranks,
                    use_source_port_ranks: true,
                    randomize_boundary: false,
                    hierarchy: Some(hierarchy),
                    hierarchy_first_sweep: Some(false),
                },
            )?;
            crossings = count_hierarchy_all_p3_crossings(arena, graph_id);
            #[cfg(feature = "diagnostic-traces")]
            crate::trace::emit_value(
                "ordering",
                "p3_crossing_count",
                &format!("graphs[{graph_id}]/trials[{trial}]/reversed_sweep"),
                &crossings.to_string(),
            );
            if crossings < trial_best_crossings {
                trial_best_crossings = crossings;
                trial_best_order = capture_hierarchy_p3_order(arena, graph_id, hierarchy);
            }
            if old_crossings <= crossings {
                break;
            }
        }
        if trial_best_crossings < best_crossings {
            best_crossings = trial_best_crossings;
            best_order = trial_best_order;
        }
        #[cfg(feature = "diagnostic-traces")]
        crate::trace::emit_value(
            "ordering",
            "p3_trial",
            &format!("graphs[{graph_id}]/trials[{trial}]"),
            &format!("trial_crossings={trial_best_crossings};best_crossings={best_crossings}"),
        );
        if best_crossings == 0 {
            break;
        }
    }
    for snapshot in &best_order {
        restore_p3_order(arena, snapshot, did_sweep);
    }
    Ok(())
}

/// Orders a deferred child's external boundary from the owning node's
/// current port list before sweeping into it. CrossMinUtil preserves EAST
/// order and reverses WEST order, independent of coordinate values.
fn sort_external_boundary_from_parent_ports(
    arena: &mut LayeredArena,
    parent: LNodeId,
    child_graph: LGraphId,
    boundary_layer: LayerId,
    forward: bool,
) {
    let side = if forward {
        PortSide::West
    } else {
        PortSide::East
    };
    let mut ports = arena.nodes[parent as usize]
        .ports
        .iter()
        .copied()
        .filter(|port| arena.ports[*port as usize].side == side)
        .collect::<Vec<_>>();
    if side == PortSide::West {
        ports.reverse();
    }
    let mut by_origin = BTreeMap::new();
    for (dummy, origin) in &arena.external_port_origins {
        if arena.nodes[*dummy as usize].graph == child_graph {
            assert!(
                by_origin.insert(*origin, *dummy).is_none(),
                "layered: a parent port must identify one child boundary dummy"
            );
        }
    }
    let ordered = ports
        .into_iter()
        .filter_map(|port| by_origin.get(&port).copied())
        .collect::<Vec<_>>();
    let expected = arena.layers[boundary_layer as usize].nodes.len();
    assert!(
        ordered.len() >= expected,
        "layered: expected {expected} hierarchical ports, but found only {}",
        ordered.len()
    );
    arena.layers[boundary_layer as usize].nodes = ordered;
}

fn sweep_deferred_children(
    arena: &mut LayeredArena,
    layer_nodes: &[LNodeId],
    forward: bool,
    first_sweep: bool,
    hierarchy: &mut BarycenterHierarchyContext,
) -> Result<(), LayeringError> {
    for node_id in layer_nodes.iter().copied() {
        let Some(child_graph) = arena.nodes[node_id as usize].nested_graph else {
            continue;
        };
        if hierarchy
            .use_bottom_up
            .get(&child_graph)
            .copied()
            .unwrap_or(true)
        {
            continue;
        }

        let Some(mut random) = hierarchy
            .randoms
            .get_mut(&child_graph)
            .and_then(Option::take)
        else {
            continue;
        };
        let layer_ids = arena.graphs[child_graph as usize].layers.clone();
        let boundary_layer = if forward {
            layer_ids.first()
        } else {
            layer_ids.last()
        };
        let randomize_boundary = boundary_layer.is_some_and(|layer_id| {
            arena.layers[*layer_id as usize]
                .nodes
                .first()
                .is_none_or(|first| {
                    arena.nodes[*first as usize].node_type != LayeredNodeType::ExternalPort
                })
        });
        if !randomize_boundary && let Some(boundary_layer) = boundary_layer {
            sort_external_boundary_from_parent_ports(
                arena,
                node_id,
                child_graph,
                *boundary_layer,
                forward,
            );
        }
        let node_relative_port_ranks = hierarchy
            .node_relative_port_ranks
            .get(&child_graph)
            .copied()
            .unwrap_or(false);
        let result = order_layers_by_barycenter_impl(
            arena,
            child_graph,
            BarycenterSweepOptions {
                forward,
                random: Some(&mut random),
                preordered: !first_sweep,
                node_relative_port_ranks,
                use_source_port_ranks: true,
                randomize_boundary,
                hierarchy: Some(hierarchy),
                hierarchy_first_sweep: Some(first_sweep),
            },
        );
        hierarchy.randoms.insert(child_graph, Some(random));
        result?;
        sort_parent_ports_by_external_dummy_order(arena, child_graph, node_id, forward)?;
    }
    Ok(())
}

// Go 1.27 arm64 fills an unknown barycenter with one fused f64 rounding.
fn source_unknown_barycenter(draw: f32, maximum: f64) -> f64 {
    f64::from(draw).mul_add(maximum, -1.0)
}

#[derive(Clone, Copy, Debug, Default)]
struct SourceBarycenterState {
    visited: bool,
    degree: usize,
    summed_weight: f64,
    barycenter: Option<f64>,
}

/// BarycenterHeuristic.calculateBarycenter/addFixedPort: same-layer edges and
/// north/south associates recurse in stored port/edge/associate order. Marking
/// before recursion makes cycles observe the source's partially accumulated
/// state, rather than recomputing a node or consuming another random draw.
fn calculate_source_barycenter(
    arena: &LayeredArena,
    node_id: LNodeId,
    forward: bool,
    port_ranks: &[f32],
    associates: &BTreeMap<LNodeId, Vec<LNodeId>>,
    states: &mut [SourceBarycenterState],
    random: &mut Option<&mut JavaRandom>,
    _node_relative_port_ranks: bool,
) {
    let index = node_id as usize;
    if states[index].visited {
        return;
    }
    states[index] = SourceBarycenterState {
        visited: true,
        ..Default::default()
    };
    let layer = arena.nodes[index].layer;
    for free_port in &arena.nodes[index].ports {
        let port = &arena.ports[*free_port as usize];
        let edges = if forward {
            &port.incoming_edges
        } else {
            &port.outgoing_edges
        };
        for edge_id in edges {
            let edge = &arena.edges[*edge_id as usize];
            if !edge.active {
                continue;
            }
            let Some(fixed_port) = (if forward {
                edge.source_port
            } else {
                edge.target_port
            }) else {
                continue;
            };
            let fixed_node = arena.ports[fixed_port as usize].node;
            if arena.nodes[fixed_node as usize].layer == layer {
                if fixed_node == node_id {
                    continue;
                }
                calculate_source_barycenter(
                    arena,
                    fixed_node,
                    forward,
                    port_ranks,
                    associates,
                    states,
                    random,
                    _node_relative_port_ranks,
                );
                let fixed = states[fixed_node as usize];
                states[index].degree += fixed.degree;
                states[index].summed_weight += fixed.summed_weight;
            } else {
                states[index].summed_weight += f64::from(port_ranks[fixed_port as usize]);
                states[index].degree += 1;
                #[cfg(feature = "diagnostic-traces")]
                trace_port_rank!(
                    node_id,
                    fixed_port,
                    port_ranks[fixed_port as usize],
                    _node_relative_port_ranks,
                    forward,
                    arena
                );
            }
        }
    }
    if let Some(nodes) = associates.get(&node_id) {
        for associate in nodes {
            if arena.nodes[*associate as usize].layer != layer {
                continue;
            }
            calculate_source_barycenter(
                arena,
                *associate,
                forward,
                port_ranks,
                associates,
                states,
                random,
                _node_relative_port_ranks,
            );
            let associated = states[*associate as usize];
            states[index].degree += associated.degree;
            states[index].summed_weight += associated.summed_weight;
        }
    }
    if states[index].degree > 0 {
        if let Some(random) = random.as_deref_mut() {
            // Optimized Go 1.27 arm64 fuses the source float32 expression.
            states[index].summed_weight +=
                f64::from(random.next_float().mul_add(0.07_f32, -0.035_f32));
        }
        states[index].barycenter = Some(states[index].summed_weight / states[index].degree as f64);
    }
}

fn order_layers_by_barycenter_impl(
    arena: &mut LayeredArena,
    graph_id: LGraphId,
    options: BarycenterSweepOptions<'_>,
) -> Result<(), LayeringError> {
    let BarycenterSweepOptions {
        forward,
        mut random,
        preordered,
        node_relative_port_ranks,
        use_source_port_ranks,
        randomize_boundary,
        mut hierarchy,
        hierarchy_first_sweep,
    } = options;
    let Some(graph) = arena.graphs.get(graph_id as usize) else {
        return Err(LayeringError::UnknownGraph(graph_id));
    };
    let layer_ids = graph.layers.clone();
    if layer_ids.is_empty() {
        return Ok(());
    }
    let boundary = if forward { 0 } else { layer_ids.len() - 1 };
    #[cfg(feature = "diagnostic-traces")]
    let _trace_graph_index = hierarchy
        .as_deref()
        .and_then(|context| context.graph_indices.get(&graph_id).copied())
        .unwrap_or(graph_id);
    #[cfg(not(feature = "diagnostic-traces"))]
    let _trace_graph_index = graph_id;
    let mut positions = BTreeMap::<LNodeId, usize>::new();
    for layer_id in &layer_ids {
        for (position, node_id) in arena.layers[*layer_id as usize]
            .nodes
            .iter()
            .copied()
            .enumerate()
        {
            positions.insert(node_id, position);
        }
    }

    let constraints = if use_source_port_ranks {
        let value = arena
            .source_forster_constraints
            .get(&graph_id)
            .cloned()
            .unwrap_or_else(|| forster::Constraints::from_graph(arena, graph_id));
        arena
            .source_forster_constraints
            .insert(graph_id, value.clone());
        Some(value)
    } else {
        None
    };

    // `SetFirstLayerOrder` randomizes the layer at the start of the sweep.
    // Stable sorting preserves the source order for equal random values, as
    // does ELK's GWT stable sort.
    if randomize_boundary && let Some(random) = random.as_deref_mut() {
        trace_barycenter_layer_start!(_trace_graph_index, boundary, true, forward);
        let mut keyed = arena.layers[layer_ids[boundary] as usize]
            .nodes
            .iter()
            .copied()
            .map(|node_id| (node_id, random.next_double()))
            .collect::<Vec<_>>();
        #[cfg(feature = "diagnostic-traces")]
        for (position, (node_id, barycenter)) in keyed.iter().enumerate() {
            trace_candidate!(boundary as u32, *node_id, *barycenter, position, arena);
        }
        keyed.sort_by(|left, right| left.1.total_cmp(&right.1));
        let mut ordered = keyed.iter().map(|entry| entry.0).collect::<Vec<_>>();
        if let Some(constraints) = &constraints {
            let mut states = vec![SourceBarycenterState::default(); arena.nodes.len()];
            for (node, value) in &keyed {
                states[*node as usize].barycenter = Some(*value);
            }
            constraints.process(arena, &mut ordered, &mut states);
        }
        #[cfg(feature = "diagnostic-traces")]
        for (position, node_id) in ordered.iter().enumerate() {
            trace_ordered!(boundary as u32, *node_id, position, arena);
        }
        arena.layers[layer_ids[boundary] as usize].nodes = ordered;
        positions.clear();
        for layer_id in &layer_ids {
            for (position, node_id) in arena.layers[*layer_id as usize]
                .nodes
                .iter()
                .copied()
                .enumerate()
            {
                positions.insert(node_id, position);
            }
        }
    }
    let mut barycenter_associates = BTreeMap::<LNodeId, Vec<LNodeId>>::new();
    for dummy in &arena.north_south_port_dummies {
        if dummy.graph == graph_id {
            let owner = arena.ports[dummy.origin_port as usize].node;
            barycenter_associates
                .entry(owner)
                .or_default()
                .push(dummy.node);
        }
    }
    let mut port_barycenters = vec![0.0_f32; arena.ports.len()];
    let first_index = if forward { 0 } else { layer_ids.len() - 1 };
    let _ = distribute_ports_while_sweeping(
        arena,
        &layer_ids,
        first_index,
        forward,
        node_relative_port_ranks,
        &mut port_barycenters,
        &mut positions,
    );
    if let Some(hierarchy) = hierarchy.as_deref_mut() {
        let boundary_nodes = arena.layers[layer_ids[first_index] as usize].nodes.clone();
        sweep_deferred_children(
            arena,
            &boundary_nodes,
            forward,
            hierarchy_first_sweep.unwrap_or(!preordered),
            hierarchy,
        )?;
    }
    if layer_ids.len() < 2 {
        return Ok(());
    }

    let layer_range: Box<dyn Iterator<Item = usize>> = if forward {
        Box::new(1..layer_ids.len())
    } else {
        Box::new((0..layer_ids.len() - 1).rev())
    };
    for free_index in layer_range {
        trace_barycenter_layer_start!(_trace_graph_index, free_index, false, forward);
        let fixed_index = if forward {
            free_index - 1
        } else {
            free_index + 1
        };
        let fixed_layer = layer_ids[fixed_index];
        let fixed_nodes = arena.layers[fixed_layer as usize].nodes.clone();
        let fixed_set = fixed_nodes
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        let fixed_port_ranks = use_source_port_ranks
            .then(|| calculate_port_ranks(arena, &fixed_nodes, !forward, node_relative_port_ranks));
        let free_layer = layer_ids[free_index];
        let current = arena.layers[free_layer as usize].nodes.clone();
        let layer_preordered = preordered
            || current.first().is_some_and(|node| {
                arena.nodes[*node as usize].node_type == LayeredNodeType::ExternalPort
            });
        let mut states = vec![SourceBarycenterState::default(); arena.nodes.len()];
        let mut keyed = Vec::<(LNodeId, Option<f64>, usize)>::with_capacity(current.len());
        if let Some(port_ranks) = &fixed_port_ranks {
            for node_id in &current {
                calculate_source_barycenter(
                    arena,
                    *node_id,
                    forward,
                    port_ranks,
                    &barycenter_associates,
                    &mut states,
                    &mut random,
                    node_relative_port_ranks,
                );
            }
            for (model_position, node_id) in current.iter().copied().enumerate() {
                keyed.push((node_id, states[node_id as usize].barycenter, model_position));
            }
        } else {
            for (model_position, node_id) in current.iter().copied().enumerate() {
                let mut neighbors = Vec::new();
                for edge in &arena.edges {
                    if !edge.active {
                        continue;
                    }
                    let neighbor = if forward && edge.target == node_id {
                        Some(edge.source)
                    } else if !forward && edge.source == node_id {
                        Some(edge.target)
                    } else {
                        None
                    };
                    if let Some(neighbor) = neighbor
                        && fixed_set.contains(&neighbor)
                    {
                        let mut rank = positions[&neighbor] as f64;
                        if arena.nodes[neighbor as usize].node_type == LayeredNodeType::ExternalPort
                        {
                            rank += 0.5;
                        }
                        neighbors.push(rank);
                    }
                }
                let barycenter = if neighbors.is_empty() {
                    None
                } else {
                    let mut value = neighbors.iter().sum::<f64>() / neighbors.len() as f64;
                    if let Some(random) = random.as_deref_mut() {
                        value += (random.next_float() * 0.07_f32 - 0.035_f32) as f64;
                    }
                    Some(value)
                };
                keyed.push((node_id, barycenter, model_position));
            }
        }

        let mut maximum = keyed
            .iter()
            .filter_map(|(_, barycenter, _)| *barycenter)
            .fold(0.0_f64, f64::max)
            + 2.0;
        if !maximum.is_finite() {
            maximum = 2.0;
        }
        let mut last_value = -1.0;
        for index in 0..keyed.len() {
            if keyed[index].1.is_none() {
                let value = if layer_preordered {
                    let next_value = keyed[index + 1..]
                        .iter()
                        .find_map(|(_, barycenter, _)| *barycenter)
                        .unwrap_or(last_value + 1.0);
                    (last_value + next_value) / 2.0
                } else if let Some(random) = random.as_deref_mut() {
                    // Go 1.27 arm64 emits FNMSUBD for this f64 expression.
                    // Separate multiplication/subtraction can change a bit.
                    source_unknown_barycenter(random.next_float(), maximum)
                } else {
                    keyed[index].2 as f64
                };
                keyed[index].1 = Some(value);
            }
            last_value = keyed[index].1.unwrap_or(last_value);
        }
        #[cfg(feature = "diagnostic-traces")]
        for (node_id, barycenter, model_position) in &keyed {
            trace_candidate!(
                free_index as u32,
                *node_id,
                barycenter.unwrap_or_default(),
                *model_position,
                arena,
            );
        }
        keyed.sort_by(|first, second| {
            first
                .1
                .unwrap_or_default()
                .partial_cmp(&second.1.unwrap_or_default())
                .unwrap_or(Ordering::Equal)
                .then(first.2.cmp(&second.2))
        });
        let mut ordered = keyed.iter().map(|entry| entry.0).collect::<Vec<_>>();
        if let Some(constraints) = &constraints {
            for (node, value, _) in &keyed {
                states[*node as usize].barycenter = *value;
            }
            constraints.process(arena, &mut ordered, &mut states);
        }
        #[cfg(feature = "diagnostic-traces")]
        for (position, node_id) in ordered.iter().enumerate() {
            trace_ordered!(free_index as u32, *node_id, position, arena);
        }
        arena.layers[free_layer as usize].nodes = ordered;
        for (position, node_id) in arena.layers[free_layer as usize]
            .nodes
            .iter()
            .copied()
            .enumerate()
        {
            positions.insert(node_id, position);
        }
        let _ = distribute_ports_while_sweeping(
            arena,
            &layer_ids,
            free_index,
            forward,
            node_relative_port_ranks,
            &mut port_barycenters,
            &mut positions,
        );
        if let Some(hierarchy) = hierarchy.as_deref_mut() {
            let layer_nodes = arena.layers[free_layer as usize].nodes.clone();
            sweep_deferred_children(
                arena,
                &layer_nodes,
                forward,
                hierarchy_first_sweep.unwrap_or(!preordered),
                hierarchy,
            )?;
        }
    }
    Ok(())
}

/// The 48-bit linear-congruential generator used by Java's `Random`.
///
/// ELK's Go port intentionally reproduces Java 8's stream because the
/// crossing minimizer observes its draw order. Rust's general-purpose RNGs
/// do not have the same sequence, so this tiny private type stays local to
/// the source-shaped ELK processor.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct JavaRandom {
    state: u64,
    #[cfg(feature = "diagnostic-traces")]
    stream: String,
    #[cfg(feature = "diagnostic-traces")]
    draw_count: u64,
}

impl JavaRandom {
    const MULTIPLIER: u64 = 0x5DEECE66D;
    const ADDEND: u64 = 0xB;
    const MASK: u64 = (1 << 48) - 1;

    #[cfg(test)]
    fn new(seed: i64) -> Self {
        Self::with_stream(seed, "java-random")
    }

    pub(crate) fn with_stream(seed: i64, stream: &str) -> Self {
        #[cfg(not(feature = "diagnostic-traces"))]
        let _ = stream;
        Self {
            state: (seed as u64 ^ Self::MULTIPLIER) & Self::MASK,
            #[cfg(feature = "diagnostic-traces")]
            stream: stream.to_owned(),
            #[cfg(feature = "diagnostic-traces")]
            draw_count: 0,
        }
    }

    fn set_seed(&mut self, seed: i64) {
        self.state = (seed as u64 ^ Self::MULTIPLIER) & Self::MASK;
        #[cfg(feature = "diagnostic-traces")]
        {
            self.draw_count = 0;
        }
        trace_rng!(
            self.stream.as_str(),
            "set_seed",
            0,
            None,
            Some(seed.to_string().as_str()),
        );
    }

    fn next(&mut self, bits: u32) -> u64 {
        self.state = (self
            .state
            .wrapping_mul(Self::MULTIPLIER)
            .wrapping_add(Self::ADDEND))
            & Self::MASK;
        #[cfg(feature = "diagnostic-traces")]
        {
            self.draw_count += 1;
        }
        self.state >> (48 - bits)
    }

    fn next_boolean(&mut self) -> bool {
        let result = self.next(1) != 0;
        trace_rng!(
            self.stream.as_str(),
            "next_boolean",
            self.draw_count,
            None,
            Some(if result { "true" } else { "false" }),
        );
        result
    }

    /// Implements `java.util.Random.nextInt(bound)` with Java's rejection
    /// step, including the biased-tail overflow check for non-power-of-two
    /// bounds. ELK uses this exact stream when it breaks routing cycles.
    pub(crate) fn next_int_n(&mut self, bound: usize) -> usize {
        assert!(bound > 0 && bound <= i32::MAX as usize);
        let result = if bound.is_power_of_two() {
            ((bound as u64 * self.next(31)) >> 31) as usize
        } else {
            loop {
                let bits = self.next(31) as usize;
                let value = bits % bound;
                // Java performs this expression in signed int32 arithmetic. For
                // the supported positive bounds, usize gives the same result.
                if bits - value + (bound - 1) <= i32::MAX as usize {
                    break value;
                }
            }
        };
        trace_rng!(
            self.stream.as_str(),
            "next_int",
            self.draw_count,
            None,
            Some(result.to_string().as_str()),
        );
        result
    }

    fn next_long(&mut self) -> i64 {
        // Java adds two signed int32 draws. The second draw is sign-extended
        // before addition; bitwise-OR only agrees when that low draw is
        // non-negative, which hid this divergence in the first component.
        let high = (self.next(32) as i32 as i64) << 32;
        let low = self.next(32) as i32 as i64;
        let result = high.wrapping_add(low);
        trace_rng!(
            self.stream.as_str(),
            "next_long",
            self.draw_count,
            None,
            Some(result.to_string().as_str()),
        );
        result
    }

    pub(crate) fn next_double(&mut self) -> f64 {
        let high = self.next(26) << 27;
        let low = self.next(27);
        let result = (high + low) as f64 / (1_u64 << 53) as f64;
        trace_rng!(
            self.stream.as_str(),
            "next_double",
            self.draw_count,
            Some(result.to_bits()),
            None,
        );
        result
    }

    pub(crate) fn next_float(&mut self) -> f32 {
        let result = self.next(24) as f32 / (1_u32 << 24) as f32;
        trace_rng!(
            self.stream.as_str(),
            "next_float",
            self.draw_count,
            Some(result.to_bits() as u64),
            None,
        );
        result
    }
}

/// Counts crossings between consecutive layers using the current arena order.
///
/// It is intentionally a diagnostic helper: ELK's production crossing counter
/// includes port ranks, long-edge dummies, and model-order influence.
#[must_use]
pub fn count_layer_crossings(arena: &LayeredArena, graph_id: LGraphId) -> usize {
    let Some(graph) = arena.graphs.get(graph_id as usize) else {
        return 0;
    };
    let mut layer_of = BTreeMap::<LNodeId, usize>::new();
    let mut position_of = BTreeMap::<LNodeId, usize>::new();
    for (layer_index, layer_id) in graph.layers.iter().copied().enumerate() {
        for (position, node_id) in arena.layers[layer_id as usize]
            .nodes
            .iter()
            .copied()
            .enumerate()
        {
            layer_of.insert(node_id, layer_index);
            position_of.insert(node_id, position);
        }
    }
    let mut crossings = 0;
    for (left_index, left) in arena.edges.iter().enumerate() {
        if !left.active {
            continue;
        }
        for right in arena.edges.iter().skip(left_index + 1) {
            if !right.active {
                continue;
            }
            let (Some(&left_source_layer), Some(&right_source_layer)) =
                (layer_of.get(&left.source), layer_of.get(&right.source))
            else {
                continue;
            };
            let (Some(&left_target_layer), Some(&right_target_layer)) =
                (layer_of.get(&left.target), layer_of.get(&right.target))
            else {
                continue;
            };
            if left_source_layer != right_source_layer
                || left_target_layer != right_target_layer
                || left_source_layer.abs_diff(left_target_layer) != 1
            {
                continue;
            }
            let source_order = position_of[&left.source].cmp(&position_of[&right.source]);
            let target_order = position_of[&left.target].cmp(&position_of[&right.target]);
            if source_order != Ordering::Equal && source_order != target_order {
                crossings += 1;
            }
        }
    }
    crossings
}

/// Counts inter-layer crossings by ELK's East/West port order.
///
/// P3's retry decision depends on edge endpoints at ports, not just on the
/// owning nodes. Each adjacent layer pair gets a counter-clockwise port rank:
/// East ports from the left layer top-to-bottom, followed by West ports from
/// the right layer bottom-to-top. Edge pairs whose endpoint orders invert are
/// crossings. In-layer, north/south, and compound-boundary crossings remain
/// separate counter work.
fn count_p3_inter_layer_crossings(arena: &LayeredArena, graph_id: LGraphId) -> usize {
    count_p3_inter_layer_crossings_between(arena, graph_id, None)
}

fn count_p3_inter_layer_crossings_between(
    arena: &LayeredArena,
    graph_id: LGraphId,
    only_layer_pair: Option<usize>,
) -> usize {
    let Some(graph) = arena.graphs.get(graph_id as usize) else {
        return 0;
    };
    let mut layer_of = BTreeMap::<LNodeId, usize>::new();
    for (layer_index, layer_id) in graph.layers.iter().copied().enumerate() {
        for node_id in arena.layers[layer_id as usize].nodes.iter().copied() {
            layer_of.insert(node_id, layer_index);
        }
    }

    let mut crossings = 0;
    for layer_index in 0..graph.layers.len().saturating_sub(1) {
        if only_layer_pair.is_some_and(|only| only != layer_index) {
            continue;
        }
        let left_layer = &arena.layers[graph.layers[layer_index] as usize];
        let right_layer = &arena.layers[graph.layers[layer_index + 1] as usize];
        let mut port_position = BTreeMap::<LPortId, usize>::new();
        let mut next_position = 0;
        for node_id in &left_layer.nodes {
            for port_id in arena.nodes[*node_id as usize].ports.iter().copied() {
                if arena.ports[port_id as usize].side == PortSide::East {
                    port_position.insert(port_id, next_position);
                    next_position += 1;
                }
            }
        }
        for node_id in right_layer.nodes.iter().rev() {
            for port_id in arena.nodes[*node_id as usize].ports.iter().copied() {
                if arena.ports[port_id as usize].side == PortSide::West {
                    port_position.insert(port_id, next_position);
                    next_position += 1;
                }
            }
        }

        let mut endpoints = Vec::<(usize, usize)>::new();
        for edge in &arena.edges {
            if !edge.active {
                continue;
            }
            let (Some(&source_layer), Some(&target_layer)) =
                (layer_of.get(&edge.source), layer_of.get(&edge.target))
            else {
                continue;
            };
            let (left_port, right_port) =
                if source_layer == layer_index && target_layer == layer_index + 1 {
                    (edge.source_port, edge.target_port)
                } else if target_layer == layer_index && source_layer == layer_index + 1 {
                    (edge.target_port, edge.source_port)
                } else {
                    continue;
                };
            let (Some(left_port), Some(right_port)) = (left_port, right_port) else {
                continue;
            };
            let (Some(&left_position), Some(&right_position)) = (
                port_position.get(&left_port),
                port_position.get(&right_port),
            ) else {
                continue;
            };
            endpoints.push((left_position, right_position));
        }
        for (index, &(left, right)) in endpoints.iter().enumerate() {
            for &(other_left, other_right) in endpoints.iter().skip(index + 1) {
                if (left < other_left && right < other_right)
                    || (left > other_left && right > other_right)
                {
                    crossings += 1;
                }
            }
        }
    }
    crossings
}

/// Counts the integer crossing total used by elk-go's `AllCrossingsCounter`
/// for one P3 graph. The model-order fast path must use this source counter:
/// node-order-only counts miss port ranks, in-layer edges, hyperedges, and
/// north/south port dummies.
fn count_p3_all_crossings(arena: &LayeredArena, graph_id: LGraphId) -> usize {
    let Some(graph) = arena.graphs.get(graph_id as usize) else {
        return 0;
    };
    let layer_nodes = graph
        .layers
        .iter()
        .map(|layer_id| arena.layers[*layer_id as usize].nodes.as_slice())
        .collect::<Vec<_>>();
    if layer_nodes.is_empty() {
        return 0;
    }

    let mut has_hyper_edges_east = vec![false; layer_nodes.len()];
    let mut has_north_south_ports = vec![false; layer_nodes.len()];
    for (layer_index, nodes) in layer_nodes.iter().enumerate() {
        for node_id in *nodes {
            let node = &arena.nodes[*node_id as usize];
            has_north_south_ports[layer_index] |= node.node_type == LayeredNodeType::NorthSouthPort;
            for port_id in &node.ports {
                let port = &arena.ports[*port_id as usize];
                let degree = p3_connected_edges(arena, *port_id).len();
                if degree > 1 {
                    if port.side == PortSide::East {
                        has_hyper_edges_east[layer_index] = true;
                    } else if port.side == PortSide::West && layer_index > 0 {
                        has_hyper_edges_east[layer_index - 1] = true;
                    }
                }
            }
        }
    }

    let mut counter = P3CrossingsCounter {
        port_positions: vec![0; arena.ports.len()],
    };
    let mut crossings =
        counter.count_in_layer_crossings_on_side(arena, layer_nodes[0], PortSide::West);
    crossings += counter.count_in_layer_crossings_on_side(
        arena,
        layer_nodes[layer_nodes.len() - 1],
        PortSide::East,
    );
    for layer_index in 0..layer_nodes.len() {
        if layer_index + 1 < layer_nodes.len() {
            let left = layer_nodes[layer_index];
            let right = layer_nodes[layer_index + 1];
            if has_hyper_edges_east[layer_index] {
                crossings +=
                    p3_count_hyperedge_crossings(arena, &mut counter.port_positions, left, right);
                crossings += counter.count_in_layer_crossings_on_side(arena, left, PortSide::East);
                crossings += counter.count_in_layer_crossings_on_side(arena, right, PortSide::West);
            } else {
                crossings += counter.count_crossings_between_layers(arena, left, right);
            }
        }
        if has_north_south_ports[layer_index] {
            crossings += counter.count_north_south_port_crossings(arena, layer_nodes[layer_index]);
        }
    }
    crossings
}

fn count_hierarchy_all_p3_crossings(arena: &LayeredArena, graph_id: LGraphId) -> usize {
    let mut crossings = count_p3_all_crossings(arena, graph_id);
    for node in arena.nodes.iter().filter(|node| node.graph == graph_id) {
        let Some(child_graph) = node.nested_graph else {
            continue;
        };
        if !p3_sweep_decision(arena, child_graph).use_bottom_up {
            crossings += count_hierarchy_all_p3_crossings(arena, child_graph);
        }
    }
    crossings
}

struct P3CrossingsCounter {
    /// Scratch ranks are keyed by stable arena port ID. elk-go keys them by a
    /// traversal-assigned ID; the identity mapping preserves all comparisons.
    port_positions: Vec<usize>,
}

impl P3CrossingsCounter {
    fn count_crossings_between_layers(
        &mut self,
        arena: &LayeredArena,
        left_nodes: &[LNodeId],
        right_nodes: &[LNodeId],
    ) -> usize {
        let mut ports = Vec::new();
        self.init_positions(arena, left_nodes, PortSide::East, true, &mut ports);
        self.init_positions(arena, right_nodes, PortSide::West, false, &mut ports);
        let mut tree = P3BinaryIndexedTree::new(ports.len());
        p3_count_crossings_on_ports(arena, &self.port_positions, &ports, &mut tree)
    }

    fn count_in_layer_crossings_on_side(
        &mut self,
        arena: &LayeredArena,
        nodes: &[LNodeId],
        side: PortSide,
    ) -> usize {
        let mut ports = Vec::new();
        self.init_positions(arena, nodes, side, true, &mut ports);
        let mut tree = P3BinaryIndexedTree::new(ports.len());
        let mut crossings = 0;
        for port_id in ports {
            let position = self.port_positions[port_id as usize];
            tree.remove_all(position);
            let mut between_layer_edges = 0;
            let mut ends = Vec::new();
            for edge_id in p3_connected_edges(arena, port_id) {
                if p3_edge_is_in_layer(arena, &arena.edges[edge_id as usize]) {
                    if let Some(other_port) = p3_other_port(&arena.edges[edge_id as usize], port_id)
                    {
                        let end_position = self.port_positions[other_port as usize];
                        if end_position > position {
                            crossings += tree.rank(end_position);
                            ends.push(end_position);
                        }
                    }
                } else {
                    between_layer_edges += 1;
                }
            }
            crossings += tree.size * between_layer_edges;
            for end in ends {
                tree.add(end);
            }
        }
        crossings
    }

    fn count_north_south_port_crossings(
        &mut self,
        arena: &LayeredArena,
        nodes: &[LNodeId],
    ) -> usize {
        let ports = self.init_north_south_positions(arena, nodes);
        let mut tree = P3BinaryIndexedTree::new(ports.len());
        let mut crossings = 0;
        for port_id in ports {
            let position = self.port_positions[port_id as usize];
            tree.remove_all(position);
            let node_id = arena.ports[port_id as usize].node;
            let node = &arena.nodes[node_id as usize];
            let mut targets = Vec::<(LPortId, usize)>::new();
            match node.node_type {
                LayeredNodeType::Normal => {
                    if let Some(dummy) = arena
                        .north_south_port_dummies
                        .iter()
                        .find(|dummy| dummy.origin_port == port_id)
                    {
                        for target_id in &arena.nodes[dummy.node as usize].ports {
                            targets.push((*target_id, p3_connected_edges(arena, *target_id).len()));
                        }
                    }
                }
                LayeredNodeType::LongEdge => {
                    if let Some(target_id) = node.ports.iter().copied().find(|id| *id != port_id) {
                        targets.push((target_id, p3_connected_edges(arena, target_id).len()));
                    }
                }
                LayeredNodeType::NorthSouthPort => {
                    if let Some(dummy) = arena
                        .north_south_port_dummies
                        .iter()
                        .find(|dummy| dummy.node == node_id)
                    {
                        targets.push((dummy.origin_port, p3_connected_edges(arena, port_id).len()));
                    }
                }
                _ => {}
            }
            let mut ends = Vec::new();
            for (target_id, degree) in targets {
                let end_position = self.port_positions[target_id as usize];
                if end_position > position {
                    crossings += tree.rank(end_position) * degree;
                    ends.push(end_position);
                }
            }
            for end in ends {
                tree.add(end);
            }
        }
        crossings
    }

    fn init_positions(
        &mut self,
        arena: &LayeredArena,
        nodes: &[LNodeId],
        side: PortSide,
        top_down: bool,
        ports: &mut Vec<LPortId>,
    ) {
        let node_indices: Box<dyn Iterator<Item = usize>> = if top_down {
            Box::new(0..nodes.len())
        } else {
            Box::new((0..nodes.len()).rev())
        };
        for node_index in node_indices {
            let mut node_ports = p3_node_ports_on_side(arena, nodes[node_index], side);
            let reverse =
                (side == PortSide::East && !top_down) || (side != PortSide::East && top_down);
            if reverse {
                node_ports.reverse();
            }
            for port_id in node_ports {
                self.port_positions[port_id as usize] = ports.len();
                ports.push(port_id);
            }
        }
    }

    fn init_north_south_positions(
        &mut self,
        arena: &LayeredArena,
        nodes: &[LNodeId],
    ) -> Vec<LPortId> {
        let mut ports = Vec::new();
        let mut stack = Vec::<LNodeId>::new();
        let mut last_layout_unit = None;
        let mut index = 0;
        for node_id in nodes.iter().copied() {
            let layout_unit = p3_in_layer_layout_unit(arena, node_id);
            if let (Some(last), Some(current)) = (last_layout_unit, layout_unit)
                && last != current
            {
                index = p3_empty_north_south_stack(
                    arena,
                    &mut self.port_positions,
                    &mut stack,
                    &mut ports,
                    index,
                );
            }
            if layout_unit.is_some() {
                last_layout_unit = layout_unit;
            }
            let node = &arena.nodes[node_id as usize];
            match node.node_type {
                LayeredNodeType::Normal => {
                    for side in [PortSide::North, PortSide::South] {
                        if side == PortSide::South {
                            index = p3_empty_north_south_stack(
                                arena,
                                &mut self.port_positions,
                                &mut stack,
                                &mut ports,
                                index,
                            );
                        }
                        for origin_port in p3_node_ports_on_side(arena, node_id, side) {
                            if arena
                                .north_south_port_dummies
                                .iter()
                                .any(|dummy| dummy.origin_port == origin_port)
                            {
                                self.port_positions[origin_port as usize] = index;
                                index += 1;
                                ports.push(origin_port);
                            }
                        }
                    }
                }
                LayeredNodeType::NorthSouthPort => {
                    if let Some(port_id) = p3_node_ports_on_side(arena, node_id, PortSide::West)
                        .first()
                        .copied()
                    {
                        self.port_positions[port_id as usize] = index;
                        index += 1;
                        ports.push(port_id);
                    }
                    if !p3_node_ports_on_side(arena, node_id, PortSide::East).is_empty() {
                        stack.push(node_id);
                    }
                }
                LayeredNodeType::LongEdge => {
                    for port_id in p3_node_ports_on_side(arena, node_id, PortSide::West) {
                        self.port_positions[port_id as usize] = index;
                        index += 1;
                        ports.push(port_id);
                    }
                    for _ in p3_node_ports_on_side(arena, node_id, PortSide::East) {
                        stack.push(node_id);
                    }
                }
                _ => {}
            }
        }
        p3_empty_north_south_stack(
            arena,
            &mut self.port_positions,
            &mut stack,
            &mut ports,
            index,
        );
        ports
    }
}

#[derive(Clone, Debug)]
struct P3BinaryIndexedTree {
    sums: Vec<usize>,
    entries_per_index: Vec<usize>,
    size: usize,
}

impl P3BinaryIndexedTree {
    fn new(max_num: usize) -> Self {
        Self {
            sums: vec![0; max_num + 1],
            entries_per_index: vec![0; max_num],
            size: 0,
        }
    }

    fn add(&mut self, index: usize) {
        self.size += 1;
        self.entries_per_index[index] += 1;
        let mut next = index + 1;
        while next < self.sums.len() {
            self.sums[next] += 1;
            next += next.isolate_lowest_one();
        }
    }

    fn rank(&self, index: usize) -> usize {
        let mut next = index;
        let mut sum = 0;
        while next > 0 {
            sum += self.sums[next];
            next -= next.isolate_lowest_one();
        }
        sum
    }

    fn remove_all(&mut self, index: usize) {
        let entries = self.entries_per_index[index];
        if entries == 0 {
            return;
        }
        self.entries_per_index[index] = 0;
        self.size -= entries;
        let mut next = index + 1;
        while next < self.sums.len() {
            self.sums[next] -= entries;
            next += next.isolate_lowest_one();
        }
    }
}

fn p3_node_ports_on_side(arena: &LayeredArena, node_id: LNodeId, side: PortSide) -> Vec<LPortId> {
    let mut result = Vec::new();
    let mut started = false;
    for port_id in &arena.nodes[node_id as usize].ports {
        if arena.ports[*port_id as usize].side == side {
            started = true;
            result.push(*port_id);
        } else if started {
            break;
        }
    }
    result
}

fn p3_connected_edges(arena: &LayeredArena, port_id: LPortId) -> Vec<LEdgeId> {
    let port = &arena.ports[port_id as usize];
    port.incoming_edges
        .iter()
        .chain(&port.outgoing_edges)
        .copied()
        .filter(|edge_id| arena.edges[*edge_id as usize].active)
        .collect()
}

fn p3_other_port(edge: &LEdge, port_id: LPortId) -> Option<LPortId> {
    if edge.source_port == Some(port_id) {
        edge.target_port
    } else if edge.target_port == Some(port_id) {
        edge.source_port
    } else {
        None
    }
}

fn p3_edge_is_in_layer(arena: &LayeredArena, edge: &LEdge) -> bool {
    if edge.source == edge.target {
        return false;
    }
    arena.nodes[edge.source as usize].layer == arena.nodes[edge.target as usize].layer
}

fn p3_count_crossings_on_ports(
    arena: &LayeredArena,
    port_positions: &[usize],
    ports: &[LPortId],
    tree: &mut P3BinaryIndexedTree,
) -> usize {
    let mut crossings = 0;
    for port_id in ports {
        let position = port_positions[*port_id as usize];
        tree.remove_all(position);
        let mut ends = Vec::new();
        for edge_id in p3_connected_edges(arena, *port_id) {
            if let Some(other_port) = p3_other_port(&arena.edges[edge_id as usize], *port_id) {
                let end_position = port_positions[other_port as usize];
                if end_position > position {
                    crossings += tree.rank(end_position);
                    ends.push(end_position);
                }
            }
        }
        for end in ends {
            tree.add(end);
        }
    }
    crossings
}

fn p3_in_layer_layout_unit(arena: &LayeredArena, node_id: LNodeId) -> Option<LNodeId> {
    let node = &arena.nodes[node_id as usize];
    if node.node_type == LayeredNodeType::Normal
        && matches!(
            node.port_constraints,
            PortConstraints::FixedSide
                | PortConstraints::FixedOrder
                | PortConstraints::FixedRatio
                | PortConstraints::FixedPosition
        )
    {
        return Some(node_id);
    }
    if node.node_type == LayeredNodeType::NorthSouthPort {
        return arena
            .north_south_port_dummies
            .iter()
            .find(|dummy| dummy.node == node_id)
            .map(|dummy| arena.ports[dummy.origin_port as usize].node);
    }
    None
}

fn p3_empty_north_south_stack(
    arena: &LayeredArena,
    port_positions: &mut [usize],
    stack: &mut Vec<LNodeId>,
    ports: &mut Vec<LPortId>,
    mut index: usize,
) -> usize {
    while let Some(node_id) = stack.pop() {
        if let Some(port_id) = p3_node_ports_on_side(arena, node_id, PortSide::East)
            .first()
            .copied()
        {
            port_positions[port_id as usize] = index;
            index += 1;
            ports.push(port_id);
        }
    }
    index
}

#[derive(Clone, Copy, Debug)]
struct P3Hyperedge {
    id: usize,
    upper_left: usize,
    lower_left: usize,
    upper_right: usize,
    lower_right: usize,
}

fn p3_count_hyperedge_crossings(
    arena: &LayeredArena,
    port_positions: &mut [usize],
    left_layer: &[LNodeId],
    right_layer: &[LNodeId],
) -> usize {
    if left_layer.is_empty() || right_layer.is_empty() {
        return 0;
    }
    let mut source_count = 0;
    for node_id in left_layer {
        for port_id in &arena.nodes[*node_id as usize].ports {
            let has_cross_layer_edge = arena.ports[*port_id as usize]
                .outgoing_edges
                .iter()
                .filter(|edge_id| arena.edges[**edge_id as usize].active)
                .any(|edge_id| {
                    arena.nodes[arena.edges[*edge_id as usize].target as usize].layer
                        != arena.nodes[*node_id as usize].layer
                });
            if has_cross_layer_edge {
                port_positions[*port_id as usize] = source_count;
                source_count += 1;
            }
        }
    }

    let mut target_count = 0;
    for node_id in right_layer {
        let node = &arena.nodes[*node_id as usize];
        let mut north_input_ports = 0;
        for port_id in &node.ports {
            if arena.ports[*port_id as usize].side != PortSide::North {
                break;
            }
            if arena.ports[*port_id as usize]
                .incoming_edges
                .iter()
                .filter(|edge_id| arena.edges[**edge_id as usize].active)
                .any(|edge_id| {
                    arena.nodes[arena.edges[*edge_id as usize].source as usize].layer != node.layer
                })
            {
                north_input_ports += 1;
            }
        }
        let mut other_input_ports = 0;
        for port_id in node.ports.iter().rev() {
            let has_cross_layer_edge = arena.ports[*port_id as usize]
                .incoming_edges
                .iter()
                .filter(|edge_id| arena.edges[**edge_id as usize].active)
                .any(|edge_id| {
                    arena.nodes[arena.edges[*edge_id as usize].source as usize].layer != node.layer
                });
            if !has_cross_layer_edge {
                continue;
            }
            if arena.ports[*port_id as usize].side == PortSide::North {
                port_positions[*port_id as usize] = target_count;
                target_count += 1;
            } else {
                port_positions[*port_id as usize] =
                    target_count + north_input_ports + other_input_ports;
                other_input_ports += 1;
            }
        }
        target_count += other_input_ports;
    }

    let left_layer_id = arena.nodes[left_layer[0] as usize].layer;
    let right_layer_id = arena.nodes[right_layer[0] as usize].layer;
    let mut hyperedges = Vec::<P3Hyperedge>::new();
    let mut port_to_hyperedge = vec![None; arena.ports.len()];
    let mut live = Vec::<usize>::new();
    for node_id in left_layer {
        for source_port in &arena.nodes[*node_id as usize].ports {
            for edge_id in arena.ports[*source_port as usize]
                .outgoing_edges
                .iter()
                .copied()
            {
                let edge = &arena.edges[edge_id as usize];
                if !edge.active {
                    continue;
                }
                let Some(target_port) = edge.target_port else {
                    continue;
                };
                let target_node = arena.ports[target_port as usize].node;
                if arena.nodes[*node_id as usize].layer == arena.nodes[target_node as usize].layer {
                    continue;
                }
                let source_hyperedge = port_to_hyperedge[*source_port as usize];
                let target_hyperedge = port_to_hyperedge[target_port as usize];
                match (source_hyperedge, target_hyperedge) {
                    (None, None) => {
                        let id = hyperedges.len();
                        hyperedges.push(P3Hyperedge {
                            id,
                            upper_left: source_count,
                            lower_left: 0,
                            upper_right: target_count,
                            lower_right: 0,
                        });
                        live.push(id);
                        port_to_hyperedge[*source_port as usize] = Some(id);
                        port_to_hyperedge[target_port as usize] = Some(id);
                    }
                    (None, Some(target_id)) => {
                        port_to_hyperedge[*source_port as usize] = Some(target_id);
                    }
                    (Some(source_id), None) => {
                        port_to_hyperedge[target_port as usize] = Some(source_id);
                    }
                    (Some(source_id), Some(target_id)) if source_id != target_id => {
                        for hyperedge_owner in &mut port_to_hyperedge {
                            if *hyperedge_owner == Some(target_id) {
                                *hyperedge_owner = Some(source_id);
                            }
                        }
                        live.retain(|id| *id != target_id);
                    }
                    _ => {}
                }
            }
        }
    }

    for hyperedge_id in &live {
        let hyperedge = &mut hyperedges[*hyperedge_id];
        for (port_index, owner) in arena.ports.iter().enumerate() {
            if port_to_hyperedge[port_index] != Some(*hyperedge_id) {
                continue;
            }
            let position = port_positions[port_index];
            let port_layer = arena.nodes[owner.node as usize].layer;
            if port_layer == left_layer_id {
                hyperedge.upper_left = hyperedge.upper_left.min(position);
                hyperedge.lower_left = hyperedge.lower_left.max(position);
            } else if port_layer == right_layer_id {
                hyperedge.upper_right = hyperedge.upper_right.min(position);
                hyperedge.lower_right = hyperedge.lower_right.max(position);
            }
        }
    }

    let mut sorted = live.iter().map(|id| hyperedges[*id]).collect::<Vec<_>>();
    sorted.sort_by_key(|edge| (edge.upper_left, edge.upper_right, edge.id));
    let mut south_sequence = sorted
        .iter()
        .map(|edge| edge.upper_right)
        .collect::<Vec<_>>();
    let mut compress_deltas = vec![0_isize; target_count + 1];
    for position in &south_sequence {
        compress_deltas[*position] = 1;
    }
    let mut delta = 0_isize;
    for compress_delta in &mut compress_deltas {
        if *compress_delta == 1 {
            *compress_delta = delta;
        } else {
            delta -= 1;
        }
    }
    let mut q = 0;
    for position in &mut south_sequence {
        *position = (*position as isize + compress_deltas[*position] as isize) as usize;
        q = q.max(*position + 1);
    }
    let mut first_index = 1;
    while first_index < q {
        first_index *= 2;
    }
    let mut tree = vec![0_usize; first_index * 2 - 1];
    first_index -= 1;
    let mut crossings = 0;
    for position in south_sequence {
        let mut index = position + first_index;
        tree[index] += 1;
        while index > 0 {
            if index % 2 > 0 {
                crossings += tree[index + 1];
            }
            index = (index - 1) / 2;
            tree[index] += 1;
        }
    }

    let mut left_corners = Vec::with_capacity(sorted.len() * 2);
    let mut right_corners = Vec::with_capacity(sorted.len() * 2);
    for edge in &sorted {
        left_corners.push((edge.upper_left, edge.lower_left, edge.id, 0_u8));
        left_corners.push((edge.lower_left, edge.upper_left, edge.id, 1_u8));
        right_corners.push((edge.upper_right, edge.lower_right, edge.id, 0_u8));
        right_corners.push((edge.lower_right, edge.upper_right, edge.id, 1_u8));
    }
    crossings += p3_count_corner_overlaps(&mut left_corners);
    crossings += p3_count_corner_overlaps(&mut right_corners);
    crossings
}

fn p3_count_corner_overlaps(corners: &mut [(usize, usize, usize, u8)]) -> usize {
    corners.sort_unstable();
    let mut open = 0_usize;
    let mut crossings = 0;
    for (_, _, _, corner_type) in corners {
        if *corner_type == 0 {
            open += 1;
        } else {
            open -= 1;
            crossings += open;
        }
    }
    crossings
}

/// Applies ELK's source-shaped simple node placer to a flat layered graph.
///
/// Each layer is stacked along the cross-axis and centered around the tallest
/// layer. The caller supplies the effective node-to-node spacing after ELK's
/// option inheritance has been resolved. Horizontal placement and the
/// Brandes–Köpf strategies are separate phase-four work.
pub fn place_layers_simple(
    arena: &mut LayeredArena,
    graph_id: LGraphId,
    node_spacing: f64,
) -> Result<(), LayeringError> {
    if graph_id as usize >= arena.graphs.len() {
        return Err(LayeringError::UnknownGraph(graph_id));
    }
    if !node_spacing.is_finite() || node_spacing < 0.0 {
        return Err(LayeringError::InvalidSpacing(node_spacing));
    }
    let layer_ids = arena.graphs[graph_id as usize].layers.clone();
    let mut maximum_height = 0.0_f64;
    for layer_id in &layer_ids {
        let nodes = arena.layers[*layer_id as usize].nodes.clone();
        let height = nodes
            .iter()
            .map(|node_id| {
                let node = &arena.nodes[*node_id as usize];
                margin_top(node) + node.height + margin_bottom(node)
            })
            .sum::<f64>()
            + node_spacing * nodes.len().saturating_sub(1) as f64;
        arena.layers[*layer_id as usize].height = height;
        maximum_height = maximum_height.max(height);
    }
    for layer_id in &layer_ids {
        let layer = &arena.layers[*layer_id as usize];
        let mut position = (maximum_height - layer.height) / 2.0;
        for (index, node_id) in layer.nodes.iter().copied().enumerate() {
            let node = &mut arena.nodes[node_id as usize];
            position += margin_top(node);
            node.position_y = position;
            position += node.height;
            position += margin_bottom(node);
            if index + 1 < layer.nodes.len() {
                position += node_spacing;
            }
        }
    }
    Ok(())
}

/// Computes layer extents and graph height after node placement.
///
/// This mirrors `intermediate/layer_size.go` for the geometry retained by the
/// Rust arena: layer width is the widest node, layer height spans the first
/// node's top to the last node's bottom, and the graph offset records the
/// minimum cross-axis coordinate.
pub fn calculate_layer_sizes_and_graph_height(
    arena: &mut LayeredArena,
    graph_id: LGraphId,
) -> Result<(), LayeringError> {
    let Some(graph) = arena.graphs.get(graph_id as usize) else {
        return Err(LayeringError::UnknownGraph(graph_id));
    };
    let layer_ids = graph.layers.clone();
    let mut minimum_y = f64::INFINITY;
    let mut maximum_y = f64::NEG_INFINITY;
    let mut graph_width: f64 = 0.0;
    let mut found_nodes = false;
    for layer_id in layer_ids {
        let nodes = arena.layers[layer_id as usize].nodes.clone();
        if nodes.is_empty() {
            arena.layers[layer_id as usize].width = 0.0;
            arena.layers[layer_id as usize].height = 0.0;
            continue;
        }
        found_nodes = true;
        let width = nodes
            .iter()
            .map(|node_id| {
                let node = &arena.nodes[*node_id as usize];
                margin_left(node) + node.width + margin_right(node)
            })
            .fold(0.0, f64::max);
        let first = &arena.nodes[nodes[0] as usize];
        let last = &arena.nodes[*nodes.last().unwrap() as usize];
        let top = first.position_y - margin_top(first);
        let bottom = last.position_y + last.height + margin_bottom(last);
        arena.layers[layer_id as usize].width = width;
        arena.layers[layer_id as usize].height = bottom - top;
        minimum_y = minimum_y.min(top);
        maximum_y = maximum_y.max(bottom);
        graph_width = graph_width.max(width);
    }
    let graph = &mut arena.graphs[graph_id as usize];
    if found_nodes {
        graph.height = maximum_y - minimum_y;
        graph.offset_y -= minimum_y;
    } else {
        graph.height = 0.0;
    }
    graph.width = graph_width;
    Ok(())
}

/// Recomputes the margins used by ELK's `InnermostNodeMarginCalculator`.
/// The upstream pass runs after ordering and before node placement: port boxes
/// that protrude beyond a node participate in the layer envelope and in the
/// horizontal alignment of nodes in that layer.
pub fn calculate_innermost_node_margins(arena: &mut LayeredArena, graph_id: LGraphId) {
    let Some(graph) = arena.graphs.get(graph_id as usize) else {
        return;
    };
    let node_ids = graph
        .layers
        .iter()
        .flat_map(|layer_id| arena.layers[*layer_id as usize].nodes.iter().copied())
        .collect::<Vec<_>>();
    for node_id in node_ids {
        let node = &arena.nodes[node_id as usize];
        let left = node.position_x;
        let top = node.position_y;
        let right = left + node.width;
        let bottom = top + node.height;
        let mut bounds_left = left;
        let mut bounds_top = top;
        let mut bounds_right = right + node.label_margin_right;
        let mut bounds_bottom = bottom + node.label_margin_bottom;
        bounds_left -= node.label_margin_left;
        bounds_top -= node.label_margin_top;
        for port_id in &node.ports {
            let port = &arena.ports[*port_id as usize];
            let port_left = left + port.x;
            let port_top = top + port.y;
            bounds_left = bounds_left.min(port_left);
            bounds_top = bounds_top.min(port_top);
            bounds_right = bounds_right.max(port_left + port.width);
            bounds_bottom = bounds_bottom.max(port_top + port.height);
        }
        let computed = [
            (top - bounds_top).max(0.0),
            (bounds_right - right).max(0.0),
            (bounds_bottom - bottom).max(0.0),
            (left - bounds_left).max(0.0),
        ];
        let node = &mut arena.nodes[node_id as usize];
        node.computed_margins = computed;
        if node.margins_in_wire_dimensions {
            // D2 has already folded these imported lanes into the dimensions;
            // retain their record while tracking computed label and port
            // overhangs separately for ELK's placement and graph sizing.
            node.margin_top = node.margin_top.max(computed[0]);
            node.margin_right = node.margin_right.max(computed[1]);
            node.margin_bottom = node.margin_bottom.max(computed[2]);
            node.margin_left = node.margin_left.max(computed[3]);
        } else {
            node.margin_top = computed[0];
            node.margin_right = computed[1];
            node.margin_bottom = computed[2];
            node.margin_left = computed[3];
        }
    }
}

/// Packs disconnected flat components using elk-go's stable simple-row rule.
///
/// The layered processors lay out each component independently and the
/// components processor combines their finished boxes afterwards. Keeping
/// this boundary explicit matters for graphs that mix an edge component with
/// isolated nodes: letting those nodes share the BK cross-axis changes both
/// the published root extent and the component-local coordinates.
pub fn pack_disconnected_components(
    arena: &mut LayeredArena,
    graph_id: LGraphId,
) -> Result<(), LayeringError> {
    pack_disconnected_components_impl(arena, graph_id, false)
}

pub(crate) fn pack_disconnected_components_for_wire(
    arena: &mut LayeredArena,
    graph_id: LGraphId,
) -> Result<(), LayeringError> {
    pack_disconnected_components_impl(arena, graph_id, true)
}

fn pack_disconnected_components_impl(
    arena: &mut LayeredArena,
    graph_id: LGraphId,
    wire_publication: bool,
) -> Result<(), LayeringError> {
    let Some(graph) = arena.graphs.get(graph_id as usize) else {
        return Err(LayeringError::UnknownGraph(graph_id));
    };
    if !graph.separate_components || graph.parent_node.is_some() {
        return Ok(());
    }
    // The simple row placer is only safe for a flat graph. Hierarchy and
    // external-port components have their own envelope processors later in
    // the source pipeline; moving their top-level vessels here would apply a
    // second, incorrect translation to descendants.
    if arena
        .nodes
        .iter()
        .any(|node| node.graph == graph_id && node.nested_graph.is_some())
        || arena.edges.iter().any(|edge| {
            edge.active
                && (arena.nodes[edge.source as usize].graph != graph_id
                    || arena.nodes[edge.target as usize].graph != graph_id)
        })
    {
        return Ok(());
    }
    let node_ids = graph
        .layers
        .iter()
        .flat_map(|layer_id| arena.layers[*layer_id as usize].nodes.iter().copied())
        .collect::<Vec<_>>();
    if node_ids.is_empty()
        || (!wire_publication && node_ids.len() < 2)
        || node_ids.iter().any(|node_id| {
            let node = &arena.nodes[*node_id as usize];
            node.graph != graph_id || node.node_type != LayeredNodeType::Normal
        })
    {
        return Ok(());
    }
    let node_set = node_ids.iter().copied().collect::<BTreeSet<_>>();
    let edge_ids = arena
        .edges
        .iter()
        .enumerate()
        .filter_map(|(edge_id, edge)| {
            (edge.active && node_set.contains(&edge.source) && node_set.contains(&edge.target))
                .then_some(edge_id as LEdgeId)
        })
        .collect::<Vec<_>>();
    let components = network_simplex::connected_components(arena, &node_ids, &edge_ids);
    // SimpleRowGraphPlacer.combine moves even a single component into
    // its target, applying Graph.Offset before summing node/port anchors.
    if components.is_empty() || (!wire_publication && components.len() < 2) {
        return Ok(());
    }

    #[derive(Clone, Debug)]
    struct ComponentBox {
        nodes: Vec<LNodeId>,
        min_x: f64,
        min_y: f64,
        width: f64,
        height: f64,
        area: f64,
        model_order: usize,
        axis_width: f64,
        offset_y: f64,
    }

    let vertical =
        wire_publication && matches!(graph.direction, LayerDirection::Down | LayerDirection::Up);
    let mut boxes = components
        .into_iter()
        .map(|nodes| {
            let component_set = nodes.iter().copied().collect::<BTreeSet<_>>();
            let mut min_x = f64::INFINITY;
            let mut min_y = f64::INFINITY;
            let mut max_x = f64::NEG_INFINITY;
            let mut max_y = f64::NEG_INFINITY;
            for &node_id in &nodes {
                let node = &arena.nodes[node_id as usize];
                min_x = min_x.min(node.position_x - margin_left(node));
                min_y = min_y.min(node.position_y - margin_top(node));
                max_x = max_x.max(node.position_x + node.width + margin_right(node));
                max_y = max_y.max(node.position_y + node.height + margin_bottom(node));
            }
            for edge in &arena.edges {
                if !edge.active
                    || !component_set.contains(&edge.source)
                    || !component_set.contains(&edge.target)
                {
                    continue;
                }
                for point in edge.bend_points.iter().chain(edge.junction_points.iter()) {
                    min_x = min_x.min(point.x);
                    min_y = min_y.min(point.y);
                    max_x = max_x.max(point.x);
                    max_y = max_y.max(point.y);
                }
            }
            let (width, height) = nodes
                .iter()
                .find_map(|node_id| arena.component_sizes.get(node_id).copied())
                .unwrap_or_else(|| ((max_x - min_x).max(0.0), (max_y - min_y).max(0.0)));
            let axis_width = width;
            let offset_y = if wire_publication {
                nodes
                    .iter()
                    .find_map(|node| arena.component_offsets_y.get(node))
                    .copied()
                    .expect("every packed component must retain its source graph offset")
            } else {
                0.0
            };
            let (width, height) = if vertical {
                (height, width)
            } else {
                (width, height)
            };
            ComponentBox {
                axis_width,
                offset_y,
                model_order: nodes
                    .iter()
                    .map(|node_id| arena.nodes[*node_id as usize].traversal_index)
                    .min()
                    .unwrap_or(usize::MAX),
                nodes,
                min_x,
                min_y,
                width,
                height,
                area: width * height,
            }
        })
        .collect::<Vec<_>>();
    // SimpleRowGraphPlacer sorts by priority, then ascending area, then source
    // model order. Components without explicit priority therefore keep their
    // original order when equal-sized isolated nodes are packed.
    boxes.sort_by(|left, right| {
        left.area
            .total_cmp(&right.area)
            .then_with(|| left.model_order.cmp(&right.model_order))
    });
    let total_area = boxes.iter().map(|component| component.area).sum::<f64>();
    let area_width = (total_area.sqrt() as f32 as f64) * graph.aspect_ratio;
    let max_row_width = boxes
        .iter()
        .map(|component| component.width)
        .fold(area_width, f64::max);
    let component_spacing = graph.component_spacing;
    let mut x = 0.0;
    let mut y = 0.0;
    let mut row_height = 0.0;
    // SimpleRowGraphPlacer.combine transfers a sole component directly,
    // before the row placer initializes its width to componentSpacing.
    // Applying that row minimum to a single narrow component inflates its
    // enclosing graph even though every child keeps its correct dimensions.
    let mut broadest_row = if boxes.len() == 1 {
        0.0
    } else {
        component_spacing
    };
    for component in &boxes {
        if x + component.width > max_row_width && x > 0.0 {
            x = 0.0;
            y += row_height + component_spacing;
            row_height = 0.0;
        }
        if wire_publication {
            let transfer = ComponentTransfer {
                axis_width: component.axis_width,
                offset: if vertical {
                    Point {
                        x: x + component.offset_y,
                        y,
                    }
                } else {
                    Point {
                        x,
                        y: y + component.offset_y,
                    }
                },
            };
            let members = component.nodes.iter().copied().collect::<BTreeSet<_>>();
            for node in &component.nodes {
                arena.component_transfers.insert(*node, transfer);
            }
            for dummy in &arena.edge_label_dummies {
                if members.contains(&arena.edges[dummy.edge as usize].source) {
                    arena.component_transfers.insert(dummy.node, transfer);
                }
            }
            broadest_row = broadest_row.max(x + component.width);
            row_height = row_height.max(component.height);
            x += component.width + component_spacing;
            continue;
        }
        let delta_x = x - component.min_x;
        let delta_y = y - component.min_y;
        for &node_id in &component.nodes {
            let node = &mut arena.nodes[node_id as usize];
            node.position_x += delta_x;
            node.position_y += delta_y;
        }
        let component_set = component.nodes.iter().copied().collect::<BTreeSet<_>>();
        for edge in &mut arena.edges {
            if !edge.active
                || !component_set.contains(&edge.source)
                || !component_set.contains(&edge.target)
            {
                continue;
            }
            for point in edge
                .bend_points
                .iter_mut()
                .chain(edge.junction_points.iter_mut())
            {
                point.x += delta_x;
                point.y += delta_y;
            }
        }
        broadest_row = broadest_row.max(x + component.width);
        row_height = row_height.max(component.height);
        x += component.width + component_spacing;
    }
    let graph = &mut arena.graphs[graph_id as usize];
    (graph.width, graph.height) = if vertical {
        (y + row_height, broadest_row)
    } else {
        (broadest_row, y + row_height)
    };
    graph.offset_y = 0.0;
    Ok(())
}

/// Vertical direction used while constructing a Brandes–Köpf alignment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BkVerticalDirection {
    /// Visit each layer from its first node to its last node.
    Down,
    /// Visit each layer from its last node to its first node.
    Up,
}

/// Horizontal direction used to select predecessor or successor neighbors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BkHorizontalDirection {
    /// Use the preceding layer as the fixed neighbor side.
    Right,
    /// Use the following layer as the fixed neighbor side.
    Left,
}

/// Fixed-alignment choices exposed by ELK's
/// `layered.nodePlacement.bk.fixedAlignment` option.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BkFixedAlignment {
    /// Evaluate all four directions and use the balanced median.
    #[default]
    None,
    /// Align from the left and traverse upward.
    LeftUp,
    /// Align from the right and traverse upward.
    RightUp,
    /// Align from the left and traverse downward.
    LeftDown,
    /// Align from the right and traverse downward.
    RightDown,
    /// Evaluate all four directions and force the balanced median.
    Balanced,
}

/// Intermediate block-alignment state produced by ELK's BK aligner.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BkAlignment {
    /// Root node for each node's aligned block.
    pub roots: BTreeMap<LNodeId, LNodeId>,
    /// Next node in each aligned block's circular list.
    pub aligns: BTreeMap<LNodeId, LNodeId>,
    /// Per-node inner shift, initially zero before port-aware sizing.
    pub inner_shifts: BTreeMap<LNodeId, f64>,
    /// Cross-axis height of each aligned block root after inner shifting.
    pub block_sizes: BTreeMap<LNodeId, f64>,
    /// Whether the root's aligned block contains only long-edge dummies.
    pub od: BTreeMap<LNodeId, bool>,
    /// Roots already used to straighten another block.
    pub su: BTreeMap<LNodeId, bool>,
}

/// Builds one directional BK vertical alignment pass.
///
/// This is the direct median-neighbor portion of
/// `p4nodes/bk_aligner.go`. It preserves ELK's traversal order, median choice,
/// and strict position gate. Marked edges represent type-one conflicts found
/// by the preceding conflict pass and are never used to join blocks.
pub fn build_bk_alignment(
    arena: &LayeredArena,
    graph_id: LGraphId,
    vertical: BkVerticalDirection,
    horizontal: BkHorizontalDirection,
    marked_edges: &BTreeSet<LEdgeId>,
) -> Result<BkAlignment, LayeringError> {
    let Some(graph) = arena.graphs.get(graph_id as usize) else {
        return Err(LayeringError::UnknownGraph(graph_id));
    };
    let layer_ids = graph.layers.clone();
    let mut node_layer = BTreeMap::<LNodeId, usize>::new();
    let mut node_position = BTreeMap::<LNodeId, usize>::new();
    for (layer_index, layer_id) in layer_ids.iter().copied().enumerate() {
        for (position, node_id) in arena.layers[layer_id as usize]
            .nodes
            .iter()
            .copied()
            .enumerate()
        {
            node_layer.insert(node_id, layer_index);
            node_position.insert(node_id, position);
        }
    }

    let reversed = bk_reversed(arena, &node_layer);
    let mut alignment = BkAlignment::default();
    for node_id in node_layer.keys().copied() {
        alignment.roots.insert(node_id, node_id);
        alignment.aligns.insert(node_id, node_id);
        alignment.inner_shifts.insert(node_id, 0.0);
        alignment.od.insert(node_id, true);
        alignment.su.insert(node_id, false);
    }

    let mut layers = (0..layer_ids.len()).collect::<Vec<_>>();
    if horizontal == BkHorizontalDirection::Left {
        layers.reverse();
    }
    for layer_index in layers {
        let mut nodes = arena.layers[layer_ids[layer_index] as usize].nodes.clone();
        if vertical == BkVerticalDirection::Up {
            nodes.reverse();
        }
        let mut r = match vertical {
            BkVerticalDirection::Down => -1isize,
            BkVerticalDirection::Up => isize::MAX,
        };
        for node_id in nodes {
            let neighbors = bk_neighbors(
                arena,
                node_id,
                layer_index,
                horizontal,
                reversed,
                &node_layer,
                &node_position,
            );
            if neighbors.is_empty() {
                continue;
            }
            let count = neighbors.len();
            let low = (((count + 1) as f64) / 2.0).floor() as usize - 1;
            let high = (((count + 1) as f64) / 2.0).ceil() as usize - 1;
            let median_indices = match vertical {
                BkVerticalDirection::Down => (low..=high).collect::<Vec<_>>(),
                BkVerticalDirection::Up => (low..=high).rev().collect::<Vec<_>>(),
            };
            for median in median_indices {
                let (neighbor, edge_id, neighbor_position) = neighbors[median];
                if alignment.aligns.get(&node_id).copied() != Some(node_id)
                    || marked_edges.contains(&edge_id)
                {
                    continue;
                }
                let allowed = match vertical {
                    BkVerticalDirection::Down => r < neighbor_position as isize,
                    BkVerticalDirection::Up => r > neighbor_position as isize,
                };
                if !allowed {
                    continue;
                }
                alignment.aligns.insert(neighbor, node_id);
                let root = alignment.roots[&neighbor];
                alignment.roots.insert(node_id, root);
                alignment.aligns.insert(node_id, root);
                let root_is_only_long_edges = alignment.od.entry(root).or_insert(true);
                *root_is_only_long_edges &=
                    arena.nodes[node_id as usize].node_type == LayeredNodeType::LongEdge;
                r = neighbor_position as isize;
            }
        }
    }
    Ok(alignment)
}

/// Marks BK type-one conflicts between inner segments and ordinary edges.
///
/// This follows `p4nodes/bk_node_placer.go::markConflicts`: an inner long-edge
/// segment narrows the admissible predecessor interval for the nodes below it;
/// predecessor edges outside that interval are excluded from every directional
/// alignment. The arena only marks edge IDs here, leaving the four alignment
/// passes independent and deterministic.
#[allow(clippy::needless_range_loop)]
pub fn mark_bk_conflicts(
    arena: &LayeredArena,
    graph_id: LGraphId,
) -> Result<BTreeSet<LEdgeId>, LayeringError> {
    let Some(graph) = arena.graphs.get(graph_id as usize) else {
        return Err(LayeringError::UnknownGraph(graph_id));
    };
    if graph.layers.len() < 3 {
        return Ok(BTreeSet::new());
    }
    let mut node_layer = BTreeMap::<LNodeId, usize>::new();
    let mut node_position = BTreeMap::<LNodeId, usize>::new();
    let mut layer_sizes = Vec::with_capacity(graph.layers.len());
    for (layer_index, &layer_id) in graph.layers.iter().enumerate() {
        let nodes = &arena.layers[layer_id as usize].nodes;
        layer_sizes.push(nodes.len());
        for (position, &node_id) in nodes.iter().enumerate() {
            node_layer.insert(node_id, layer_index);
            node_position.insert(node_id, position);
        }
    }
    let reversed = bk_reversed(arena, &node_layer);
    let mut marked = BTreeSet::new();
    for layer_index in 1..graph.layers.len() - 1 {
        let current_nodes = arena.layers[graph.layers[layer_index + 1] as usize]
            .nodes
            .clone();
        let mut k0 = 0usize;
        let mut lower_start = 0usize;
        for lower_end in 0..current_nodes.len() {
            let node_id = current_nodes[lower_end];
            let inner = incident_to_bk_inner_segment(
                arena,
                node_id,
                layer_index + 1,
                layer_index,
                &node_layer,
            );
            if lower_end + 1 != current_nodes.len() && !inner {
                continue;
            }
            let mut k1 = layer_sizes[layer_index].saturating_sub(1);
            if inner
                && let Some((neighbor, _, _)) = bk_neighbors(
                    arena,
                    node_id,
                    layer_index + 1,
                    BkHorizontalDirection::Right,
                    reversed,
                    &node_layer,
                    &node_position,
                )
                .first()
                .copied()
            {
                k1 = node_position[&neighbor];
            }
            while lower_start <= lower_end {
                let current = current_nodes[lower_start];
                if !incident_to_bk_inner_segment(
                    arena,
                    current,
                    layer_index + 1,
                    layer_index,
                    &node_layer,
                ) {
                    for (_, edge_id, position) in bk_neighbors(
                        arena,
                        current,
                        layer_index + 1,
                        BkHorizontalDirection::Right,
                        reversed,
                        &node_layer,
                        &node_position,
                    ) {
                        if position < k0 || position > k1 {
                            marked.insert(edge_id);
                        }
                    }
                }
                lower_start += 1;
            }
            k0 = k1;
        }
    }
    Ok(marked)
}

fn incident_to_bk_inner_segment(
    arena: &LayeredArena,
    node_id: LNodeId,
    node_layer_index: usize,
    source_layer_index: usize,
    node_layer: &BTreeMap<LNodeId, usize>,
) -> bool {
    if arena.nodes[node_id as usize].node_type != LayeredNodeType::LongEdge {
        return false;
    }
    arena.incoming_edges(node_id).any(|(_, edge)| {
        arena.nodes[edge.source as usize].node_type == LayeredNodeType::LongEdge
            && node_layer.get(&edge.source).copied() == Some(source_layer_index)
            && node_layer.get(&node_id).copied() == Some(node_layer_index)
    })
}

/// Applies the source-shaped `InsideBlockShift` calculation to a BK block.
///
/// Port centers are used as the anchor because the wire model does not carry
/// ELK's mutable anchor object separately. The resulting `inner_shifts` and
/// `block_sizes` are the state consumed by horizontal compaction.
pub fn apply_bk_inner_shifts(
    arena: &LayeredArena,
    alignment: &mut BkAlignment,
    horizontal: BkHorizontalDirection,
) {
    let mut roots = alignment.roots.values().copied().collect::<BTreeSet<_>>();
    while let Some(root) = roots.pop_first() {
        if alignment.block_sizes.contains_key(&root) {
            continue;
        }
        let root_node = &arena.nodes[root as usize];
        let mut space_above: f64 = margin_top(root_node);
        let mut space_below = root_node.height + margin_bottom(root_node);
        let mut current = root;
        let mut visited = BTreeSet::new();
        loop {
            if !visited.insert(current) {
                break;
            }
            let next = alignment.aligns[&current];
            if next == root {
                break;
            }
            let Some(edge) = arena
                .node_connected_edges_source_order(current)
                .into_iter()
                .find(|(_, edge)| arena.other_node(edge, current) == Some(next))
            else {
                break;
            };
            let (_edge_id, edge) = edge;
            // elk-go's InsideBlockShift uses the edge's original source and
            // target ports even when the alignment walks the block backward.
            // Rebinding them to `current` reverses the sign for leftward
            // candidates and changes the balanced median.
            let port_delta = match (edge.source_port, edge.target_port) {
                (Some(source), Some(target)) => {
                    let source = &arena.ports[source as usize];
                    let target = &arena.ports[target as usize];
                    let source_center = source.y + source.height / 2.0;
                    let target_center = target.y + target.height / 2.0;
                    match horizontal {
                        BkHorizontalDirection::Right => source_center - target_center,
                        BkHorizontalDirection::Left => target_center - source_center,
                    }
                }
                _ => 0.0,
            };
            #[cfg(feature = "diagnostic-traces")]
            crate::trace::emit_bk_inner_shift(
                current, next, _edge_id, edge, horizontal, port_delta, arena,
            );
            let shift = alignment.inner_shifts[&current] + port_delta;
            alignment.inner_shifts.insert(next, shift);
            let next_node = &arena.nodes[next as usize];
            space_above = space_above.max(margin_top(next_node) - shift);
            space_below = space_below.max(shift + next_node.height + margin_bottom(next_node));
            current = next;
        }
        let mut current = root;
        let mut visited = BTreeSet::new();
        loop {
            if !visited.insert(current) {
                break;
            }
            let shift = alignment.inner_shifts[&current] + space_above;
            alignment.inner_shifts.insert(current, shift);
            current = alignment.aligns[&current];
        }
        alignment
            .block_sizes
            .insert(root, space_above + space_below);
    }
}

/// Computes a deterministic block placement from BK alignment state.
///
/// This is the source-shaped difference-constraint core of
/// `BKCompactor::HorizontalCompaction`: adjacent nodes in each layer retain
/// their order, aligned members share a root coordinate plus inner shift, and
/// the direction selects max (down) or min (up) separation. Class sinks,
/// threshold straightening, and compound margins are later refinements.
pub fn compact_bk_alignment(
    arena: &LayeredArena,
    graph_id: LGraphId,
    alignment: &BkAlignment,
    node_spacing: f64,
    vertical: BkVerticalDirection,
) -> Result<BTreeMap<LNodeId, f64>, LayeringError> {
    if graph_id as usize >= arena.graphs.len() {
        return Err(LayeringError::UnknownGraph(graph_id));
    }
    if !node_spacing.is_finite() || node_spacing < 0.0 {
        return Err(LayeringError::InvalidSpacing(node_spacing));
    }
    let layer_ids = arena.graphs[graph_id as usize].layers.clone();
    let mut root_positions = BTreeMap::<LNodeId, f64>::new();
    for root in alignment.roots.values().copied().collect::<BTreeSet<_>>() {
        root_positions.entry(root).or_insert(0.0);
    }
    let mut layers = (0..layer_ids.len()).collect::<Vec<_>>();
    if vertical == BkVerticalDirection::Up {
        layers.reverse();
    }
    for layer_index in layers {
        let nodes = &arena.layers[layer_ids[layer_index] as usize].nodes;
        let mut ordered = nodes.clone();
        if vertical == BkVerticalDirection::Up {
            ordered.reverse();
        }
        for pair in ordered.windows(2) {
            let first = pair[0];
            let second = pair[1];
            let first_root = alignment.roots[&first];
            let second_root = alignment.roots[&second];
            let first_position = root_positions[&first_root] + alignment.inner_shifts[&first];
            let second_shift = alignment.inner_shifts[&second];
            let first_node = &arena.nodes[first as usize];
            let second_node = &arena.nodes[second as usize];
            let candidate = match vertical {
                BkVerticalDirection::Down => {
                    first_position
                        + first_node.height
                        + first_node.margin_bottom
                        + node_spacing
                        + second_node.margin_top
                        - second_shift
                }
                BkVerticalDirection::Up => {
                    first_position
                        - first_node.margin_top
                        - node_spacing
                        - second_node.margin_bottom
                        - second_node.height
                        - second_shift
                }
            };
            match vertical {
                BkVerticalDirection::Down => {
                    let entry = root_positions.entry(second_root).or_insert(candidate);
                    *entry = entry.max(candidate);
                }
                BkVerticalDirection::Up => {
                    let entry = root_positions.entry(second_root).or_insert(candidate);
                    *entry = entry.min(candidate);
                }
            }
        }
    }
    Ok(alignment
        .roots
        .iter()
        .map(|(&node, &root)| (node, root_positions[&root] + alignment.inner_shifts[&node]))
        .collect())
}

#[derive(Default)]
struct BkClassNode {
    outgoing: Vec<(usize, f64)>,
    indegree: usize,
    shift: Option<f64>,
}

#[derive(Clone, Copy)]
struct BkPostprocessable {
    free: LNodeId,
    is_root: bool,
    edge: Option<LEdgeId>,
}

#[allow(clippy::too_many_arguments)]
fn calculate_bk_threshold(
    arena: &LayeredArena,
    alignment: &mut BkAlignment,
    finished_blocks: &BTreeSet<LNodeId>,
    root: LNodeId,
    current: LNodeId,
    root_positions: &BTreeMap<LNodeId, f64>,
    postprocess_queue: &mut Vec<BkPostprocessable>,
    vertical: BkVerticalDirection,
    horizontal: BkHorizontalDirection,
    old_threshold: f64,
) -> f64 {
    let graph = &arena.graphs[arena.nodes[current as usize].graph as usize];
    if !graph.bk_edge_straightening {
        return match vertical {
            BkVerticalDirection::Down => f64::NEG_INFINITY,
            BkVerticalDirection::Up => f64::INFINITY,
        };
    }

    let is_root = root == current;
    let is_last = alignment.aligns.get(&current).copied() == Some(root);
    if !is_root && !is_last {
        return old_threshold;
    }

    let mut threshold = old_threshold;
    if is_root {
        threshold = bk_threshold_bound(
            arena,
            alignment,
            finished_blocks,
            root_positions,
            postprocess_queue,
            root,
            true,
            horizontal,
            vertical,
        );
    }
    if threshold.is_infinite() && is_last {
        threshold = bk_threshold_bound(
            arena,
            alignment,
            finished_blocks,
            root_positions,
            postprocess_queue,
            current,
            false,
            horizontal,
            vertical,
        );
    }
    threshold
}

#[allow(clippy::too_many_arguments)]
fn bk_threshold_bound(
    arena: &LayeredArena,
    alignment: &mut BkAlignment,
    finished_blocks: &BTreeSet<LNodeId>,
    root_positions: &BTreeMap<LNodeId, f64>,
    postprocess_queue: &mut Vec<BkPostprocessable>,
    free: LNodeId,
    is_root: bool,
    horizontal: BkHorizontalDirection,
    vertical: BkVerticalDirection,
) -> f64 {
    let invalid = match vertical {
        BkVerticalDirection::Down => f64::NEG_INFINITY,
        BkVerticalDirection::Up => f64::INFINITY,
    };
    let edges = match (is_root, horizontal) {
        (true, BkHorizontalDirection::Right) | (false, BkHorizontalDirection::Left) => {
            arena.node_incoming_edges(free)
        }
        _ => arena.node_outgoing_edges(free),
    };
    let root = alignment.roots[&free];
    let mut has_edges = false;
    let mut selected_edge = None;
    for edge_id in edges {
        let edge = &arena.edges[edge_id as usize];
        let other = if edge.source == free {
            edge.target
        } else {
            edge.source
        };
        let only_dummy_block = alignment.od.get(&root).copied().unwrap_or(true);
        if !only_dummy_block && p3_edge_is_in_layer(arena, edge) {
            continue;
        }
        if alignment.su.get(&root).copied().unwrap_or(false) {
            continue;
        }
        has_edges = true;
        let other_root = *alignment.roots.get(&other).unwrap_or_else(|| {
            panic!(
                "BK threshold edge endpoint is outside the active alignment: graph={} free={} edge={} other={} source={} target={} source_id={:?} target_id={:?} source_port_node={:?} target_port_node={:?} source_graph={} target_graph={} source_layer={:?} target_layer={:?} source_layer_graph={:?} target_layer_graph={:?}",
                arena.nodes[free as usize].graph,
                free,
                edge_id,
                other,
                edge.source,
                edge.target,
                arena.nodes[edge.source as usize].source_id,
                arena.nodes[edge.target as usize].source_id,
                edge.source_port.map(|port| arena.ports[port as usize].node),
                edge.target_port.map(|port| arena.ports[port as usize].node),
                arena.nodes[edge.source as usize].graph,
                arena.nodes[edge.target as usize].graph,
                arena.nodes[edge.source as usize].layer,
                arena.nodes[edge.target as usize].layer,
                arena.nodes[edge.source as usize].layer.map(|layer| arena.layers[layer as usize].graph),
                arena.nodes[edge.target as usize].layer.map(|layer| arena.layers[layer as usize].graph),
            )
        });
        if finished_blocks.contains(&other_root) {
            selected_edge = Some(edge_id);
            break;
        }
    }
    let Some(edge_id) = selected_edge else {
        if has_edges {
            postprocess_queue.push(BkPostprocessable {
                free,
                is_root,
                edge: None,
            });
        }
        return invalid;
    };

    let edge = &arena.edges[edge_id as usize];
    let (source_port, target_port) = (edge.source_port, edge.target_port);
    let (Some(source_port), Some(target_port)) = (source_port, target_port) else {
        return invalid;
    };
    let (root_port, other_port) = match (is_root, horizontal) {
        (true, BkHorizontalDirection::Right) | (false, BkHorizontalDirection::Left) => {
            (target_port, source_port)
        }
        _ => (source_port, target_port),
    };
    let root_port_node = arena.ports[root_port as usize].node;
    let other_port_node = arena.ports[other_port as usize].node;
    let other_root = alignment.roots[&other_port_node];
    let port_center_y = |port_id: LPortId| {
        let port = &arena.ports[port_id as usize];
        port.y + port.height / 2.0
    };
    let other_position = root_positions.get(&other_root).copied().unwrap_or(0.0)
        + alignment.inner_shifts[&other_port_node]
        + port_center_y(other_port)
        - alignment.inner_shifts[&root_port_node]
        - port_center_y(root_port);
    alignment.su.insert(
        alignment.roots[&arena.ports[source_port as usize].node],
        true,
    );
    alignment.su.insert(
        alignment.roots[&arena.ports[target_port as usize].node],
        true,
    );
    other_position
}

/// Runs the source-shaped BK compactor with sink classes.
///
/// The public [`compact_bk_alignment`] function remains the small rightward
/// probe used by existing callers. The directional chooser uses this path,
/// which mirrors `BKCompactor::placeBlock` and `placeClasses`: blocks are
/// placed recursively, separations between different sinks become class
/// constraints, and the class DAG is solved in source insertion order.
struct BkCompaction {
    node_positions: BTreeMap<LNodeId, f64>,
    block_positions: BTreeMap<LNodeId, f64>,
}

fn compact_bk_alignment_directional(
    arena: &LayeredArena,
    graph_id: LGraphId,
    alignment: &BkAlignment,
    node_spacing: f64,
    vertical: BkVerticalDirection,
    horizontal: BkHorizontalDirection,
) -> Result<BkCompaction, LayeringError> {
    if graph_id as usize >= arena.graphs.len() {
        return Err(LayeringError::UnknownGraph(graph_id));
    }
    if !node_spacing.is_finite() || node_spacing < 0.0 {
        return Err(LayeringError::InvalidSpacing(node_spacing));
    }
    let mut alignment = alignment.clone();
    let graph = &arena.graphs[graph_id as usize];
    let layer_ids = graph.layers.clone();
    let mut positions = BTreeMap::<LNodeId, usize>::new();
    let mut layer_nodes = BTreeMap::<LNodeId, Vec<LNodeId>>::new();
    for &layer_id in &layer_ids {
        let nodes = arena.layers[layer_id as usize].nodes.clone();
        for (index, &node_id) in nodes.iter().enumerate() {
            positions.insert(node_id, index);
        }
        layer_nodes.insert(layer_id as LNodeId, nodes);
    }
    let mut root_positions = BTreeMap::<LNodeId, f64>::new();
    let mut finished_blocks = BTreeSet::<LNodeId>::new();
    let mut sinks = BTreeMap::<LNodeId, LNodeId>::new();
    let mut classes = Vec::<BkClassNode>::new();
    let mut class_indices = BTreeMap::<LNodeId, usize>::new();
    let mut postprocess_queue = Vec::<BkPostprocessable>::new();

    fn class_index(
        sink: LNodeId,
        classes: &mut Vec<BkClassNode>,
        class_indices: &mut BTreeMap<LNodeId, usize>,
    ) -> usize {
        if let Some(&index) = class_indices.get(&sink) {
            return index;
        }
        let index = classes.len();
        classes.push(BkClassNode::default());
        class_indices.insert(sink, index);
        index
    }

    #[allow(clippy::too_many_arguments)]
    fn place_block(
        arena: &LayeredArena,
        alignment: &mut BkAlignment,
        layer_nodes: &BTreeMap<LNodeId, Vec<LNodeId>>,
        positions: &BTreeMap<LNodeId, usize>,
        node_spacing: f64,
        vertical: BkVerticalDirection,
        horizontal: BkHorizontalDirection,
        root: LNodeId,
        root_positions: &mut BTreeMap<LNodeId, f64>,
        finished_blocks: &mut BTreeSet<LNodeId>,
        sinks: &mut BTreeMap<LNodeId, LNodeId>,
        classes: &mut Vec<BkClassNode>,
        class_indices: &mut BTreeMap<LNodeId, usize>,
        postprocess_queue: &mut Vec<BkPostprocessable>,
    ) {
        if root_positions.contains_key(&root) {
            return;
        }
        root_positions.insert(root, 0.0);
        sinks.entry(root).or_insert(root);
        let mut current = root;
        let mut initial_assignment = true;
        let mut threshold = match vertical {
            BkVerticalDirection::Down => f64::NEG_INFINITY,
            BkVerticalDirection::Up => f64::INFINITY,
        };
        loop {
            let node = &arena.nodes[current as usize];
            let layer_id = node.layer.expect("BK compactor requires assigned layers");
            let nodes = &layer_nodes[&(layer_id as LNodeId)];
            let current_position = positions[&current];
            let neighbor_index = match vertical {
                BkVerticalDirection::Down => current_position.checked_sub(1),
                BkVerticalDirection::Up => {
                    (current_position + 1 < nodes.len()).then_some(current_position + 1)
                }
            };
            if let Some(neighbor_index) = neighbor_index {
                let neighbor = nodes[neighbor_index];
                let neighbor_root = alignment.roots[&neighbor];
                place_block(
                    arena,
                    alignment,
                    layer_nodes,
                    positions,
                    node_spacing,
                    vertical,
                    horizontal,
                    neighbor_root,
                    root_positions,
                    finished_blocks,
                    sinks,
                    classes,
                    class_indices,
                    postprocess_queue,
                );
                threshold = calculate_bk_threshold(
                    arena,
                    alignment,
                    finished_blocks,
                    root,
                    current,
                    root_positions,
                    postprocess_queue,
                    vertical,
                    horizontal,
                    threshold,
                );
                #[cfg(feature = "diagnostic-traces")]
                crate::trace::emit_value(
                    "placement",
                    "bk_threshold",
                    &format!(
                        "graphs[{}].nodes[{root}->{current}]",
                        arena.nodes[current as usize].graph
                    ),
                    &format!(
                        "vertical={vertical:?};threshold_bits={:016x}",
                        threshold.to_bits()
                    ),
                );
                let root_sink = *sinks.get(&root).unwrap_or(&root);
                let neighbor_sink = *sinks.get(&neighbor_root).unwrap_or(&neighbor_root);
                if root_sink == root {
                    sinks.insert(root, neighbor_sink);
                }
                let root_sink = *sinks.get(&root).unwrap_or(&root);
                if root_sink == neighbor_sink {
                    let neighbor_position =
                        root_positions[&neighbor_root] + alignment.inner_shifts[&neighbor];
                    let spacing = bk_vertical_spacing(arena, neighbor, current, node_spacing);
                    let candidate = match vertical {
                        BkVerticalDirection::Down => {
                            neighbor_position
                                + arena.nodes[neighbor as usize].height
                                + bk_margin_bottom(&arena.nodes[neighbor as usize])
                                + spacing
                                + bk_margin_top(node)
                                - alignment.inner_shifts[&current]
                        }
                        BkVerticalDirection::Up => {
                            neighbor_position
                                - bk_margin_top(&arena.nodes[neighbor as usize])
                                - spacing
                                - bk_margin_bottom(node)
                                - node.height
                                - alignment.inner_shifts[&current]
                        }
                    };
                    #[cfg(feature = "diagnostic-traces")]
                    crate::trace::emit_value(
                        "placement",
                        "bk_spacing_constraint",
                        &format!(
                            "graphs[{}].nodes[{current}->{neighbor}]",
                            arena.nodes[current as usize].graph
                        ),
                        &format!(
                            "vertical={vertical:?};same_sink=true;spacing_bits={:016x};position_bits={:016x}",
                            spacing.to_bits(),
                            candidate.to_bits(),
                        ),
                    );
                    let candidate = match vertical {
                        BkVerticalDirection::Down => candidate.max(threshold),
                        BkVerticalDirection::Up => candidate.min(threshold),
                    };
                    let root_position = root_positions.get_mut(&root).unwrap();
                    if initial_assignment {
                        *root_position = candidate;
                    } else if vertical == BkVerticalDirection::Down {
                        *root_position = root_position.max(candidate);
                    } else {
                        *root_position = root_position.min(candidate);
                    }
                    initial_assignment = false;
                } else {
                    let neighbor_position =
                        root_positions[&neighbor_root] + alignment.inner_shifts[&neighbor];
                    // elk-go uses SpacingNodeNode between different BK sink
                    // classes. The type-aware VerticalSpacing rule applies
                    // only while extending one aligned block above.
                    let spacing = node_spacing;
                    let required = match vertical {
                        BkVerticalDirection::Down => {
                            root_positions[&root] + alignment.inner_shifts[&current]
                                - bk_margin_top(node)
                                - root_positions[&neighbor_root]
                                - alignment.inner_shifts[&neighbor]
                                - arena.nodes[neighbor as usize].height
                                - bk_margin_bottom(&arena.nodes[neighbor as usize])
                                - spacing
                        }
                        BkVerticalDirection::Up => {
                            root_positions[&root]
                                + alignment.inner_shifts[&current]
                                + node.height
                                + bk_margin_bottom(node)
                                + spacing
                                - (neighbor_position
                                    - bk_margin_top(&arena.nodes[neighbor as usize]))
                        }
                    };
                    #[cfg(feature = "diagnostic-traces")]
                    crate::trace::emit_value(
                        "placement",
                        "bk_spacing_constraint",
                        &format!(
                            "graphs[{}].nodes[{current}->{neighbor}]",
                            arena.nodes[current as usize].graph
                        ),
                        &format!(
                            "vertical={vertical:?};same_sink=false;spacing_bits={:016x};position_bits={:016x}",
                            spacing.to_bits(),
                            required.to_bits(),
                        ),
                    );
                    let source = class_index(root_sink, classes, class_indices);
                    let target = class_index(neighbor_sink, classes, class_indices);
                    #[cfg(feature = "diagnostic-traces")]
                    crate::trace::emit_value(
                        "placement",
                        "bk_class_constraint",
                        &format!(
                            "graphs[{}].classes[{}->{}]",
                            arena.nodes[current as usize].graph,
                            bk_trace_node_id(arena, root_sink),
                            bk_trace_node_id(arena, neighbor_sink),
                        ),
                        &format!(
                            "vertical={vertical:?};horizontal={horizontal:?};separation_bits={:016x}",
                            required.to_bits(),
                        ),
                    );
                    classes[source].outgoing.push((target, required));
                    classes[target].indegree += 1;
                }
            } else {
                threshold = calculate_bk_threshold(
                    arena,
                    alignment,
                    finished_blocks,
                    root,
                    current,
                    root_positions,
                    postprocess_queue,
                    vertical,
                    horizontal,
                    threshold,
                );
                #[cfg(feature = "diagnostic-traces")]
                crate::trace::emit_value(
                    "placement",
                    "bk_threshold",
                    &format!(
                        "graphs[{}].nodes[{root}->{current}]",
                        arena.nodes[current as usize].graph
                    ),
                    &format!(
                        "vertical={vertical:?};threshold_bits={:016x}",
                        threshold.to_bits()
                    ),
                );
            }
            current = alignment.aligns[&current];
            if current == root {
                break;
            }
        }
        finished_blocks.insert(root);
    }

    let mut ordered_layers = layer_ids.clone();
    if horizontal == BkHorizontalDirection::Left {
        ordered_layers.reverse();
    }
    for layer_id in ordered_layers {
        let mut nodes = arena.layers[layer_id as usize].nodes.clone();
        if vertical == BkVerticalDirection::Up {
            nodes.reverse();
        }
        for node_id in nodes {
            if alignment.roots[&node_id] == node_id {
                place_block(
                    arena,
                    &mut alignment,
                    &layer_nodes,
                    &positions,
                    node_spacing,
                    vertical,
                    horizontal,
                    node_id,
                    &mut root_positions,
                    &mut finished_blocks,
                    &mut sinks,
                    &mut classes,
                    &mut class_indices,
                    &mut postprocess_queue,
                );
            }
        }
    }

    let mut queue = classes
        .iter()
        .enumerate()
        .filter_map(|(index, class)| (class.indegree == 0).then_some(index))
        .collect::<Vec<_>>();
    let mut queue_index = 0;
    while queue_index < queue.len() {
        let index = queue[queue_index];
        queue_index += 1;
        let shift = classes[index].shift.unwrap_or(0.0);
        let outgoing = classes[index].outgoing.clone();
        for (target, separation) in outgoing {
            let candidate = shift + separation;
            let target_shift = &mut classes[target].shift;
            *target_shift = Some(match target_shift {
                Some(current) if vertical == BkVerticalDirection::Down => current.min(candidate),
                Some(current) => current.max(candidate),
                None => candidate,
            });
            classes[target].indegree = classes[target].indegree.saturating_sub(1);
            if classes[target].indegree == 0 {
                queue.push(target);
            }
        }
    }
    let class_shifts = class_indices
        .iter()
        .map(|(&sink, &index)| (sink, classes[index].shift.unwrap_or(0.0)))
        .collect::<BTreeMap<_, _>>();
    #[cfg(feature = "diagnostic-traces")]
    for (&sink, &shift) in &class_shifts {
        crate::trace::emit_value(
            "placement",
            "bk_class_shift",
            &format!(
                "graphs[{}].classes[{}]",
                arena.nodes[sink as usize].graph,
                bk_trace_node_id(arena, sink),
            ),
            &format!(
                "vertical={vertical:?};horizontal={horizontal:?};shift_bits={:016x}",
                shift.to_bits(),
            ),
        );
    }
    let mut placed_root_positions = alignment
        .roots
        .iter()
        .filter_map(|(&node, &root)| (node == root).then_some(root))
        .map(|root| {
            let sink = sinks.get(&root).copied().unwrap_or(root);
            let shift = class_shifts.get(&sink).copied().unwrap_or(0.0);
            (root, root_positions[&root] + shift)
        })
        .collect::<BTreeMap<_, _>>();
    if arena.graphs[graph_id as usize].bk_edge_straightening {
        let mut postprocess_stack = Vec::<BkPostprocessable>::new();
        for mut processable in postprocess_queue {
            processable.edge = bk_pick_postprocess_edge(
                arena,
                &alignment,
                &finished_blocks,
                processable,
                horizontal,
            );
            let Some(edge_id) = processable.edge else {
                continue;
            };
            let root = alignment.roots[&processable.free];
            if !alignment.od.get(&root).copied().unwrap_or(true)
                && arena.nodes[arena.edges[edge_id as usize].source as usize].layer
                    == arena.nodes[arena.edges[edge_id as usize].target as usize].layer
            {
                continue;
            }
            if !bk_process_postprocessable(
                arena,
                &alignment,
                &layer_nodes,
                &positions,
                &mut placed_root_positions,
                processable,
                vertical,
                horizontal,
            ) {
                postprocess_stack.push(processable);
            }
        }
        while let Some(processable) = postprocess_stack.pop() {
            bk_process_postprocessable(
                arena,
                &alignment,
                &layer_nodes,
                &positions,
                &mut placed_root_positions,
                processable,
                vertical,
                horizontal,
            );
        }
    }
    let node_positions = alignment
        .roots
        .iter()
        .map(|(&node, &root)| {
            (
                node,
                placed_root_positions[&root] + alignment.inner_shifts[&node],
            )
        })
        .collect();
    Ok(BkCompaction {
        node_positions,
        block_positions: placed_root_positions,
    })
}

fn bk_pick_postprocess_edge(
    arena: &LayeredArena,
    alignment: &BkAlignment,
    finished_blocks: &BTreeSet<LNodeId>,
    processable: BkPostprocessable,
    horizontal: BkHorizontalDirection,
) -> Option<LEdgeId> {
    let edges: Vec<(LEdgeId, &LEdge)> = match (processable.is_root, horizontal) {
        (true, BkHorizontalDirection::Right) | (false, BkHorizontalDirection::Left) => arena
            .node_incoming_edges(processable.free)
            .into_iter()
            .map(|edge_id| (edge_id, &arena.edges[edge_id as usize]))
            .collect(),
        _ => arena
            .node_outgoing_edges(processable.free)
            .into_iter()
            .map(|edge_id| (edge_id, &arena.edges[edge_id as usize]))
            .collect(),
    };
    let root = alignment.roots[&processable.free];
    edges.into_iter().find_map(|(edge_id, edge)| {
        let other = if edge.source == processable.free {
            edge.target
        } else {
            edge.source
        };
        if !alignment.od.get(&root).copied().unwrap_or(true) && p3_edge_is_in_layer(arena, edge) {
            return None;
        }
        if alignment.su.get(&root).copied().unwrap_or(false) {
            return None;
        }
        let other_root = alignment.roots[&other];
        finished_blocks.contains(&other_root).then_some(edge_id)
    })
}

#[allow(clippy::too_many_arguments)]
#[cfg_attr(not(feature = "diagnostic-traces"), allow(unused_variables))]
fn bk_process_postprocessable(
    arena: &LayeredArena,
    alignment: &BkAlignment,
    layer_nodes: &BTreeMap<LNodeId, Vec<LNodeId>>,
    positions: &BTreeMap<LNodeId, usize>,
    root_positions: &mut BTreeMap<LNodeId, f64>,
    processable: BkPostprocessable,
    vertical: BkVerticalDirection,
    horizontal: BkHorizontalDirection,
) -> bool {
    let Some(edge_id) = processable.edge else {
        return true;
    };
    let edge = &arena.edges[edge_id as usize];
    let (Some(source_port), Some(target_port)) = (edge.source_port, edge.target_port) else {
        return true;
    };
    let (fix_port, block_port) = if edge.source == processable.free {
        (target_port, source_port)
    } else {
        (source_port, target_port)
    };
    let port_center = |port_id: LPortId| {
        let port = &arena.ports[port_id as usize];
        port.y + port.height / 2.0
    };
    let port_position = |port_id: LPortId| {
        let node_id = arena.ports[port_id as usize].node;
        let root = alignment.roots[&node_id];
        root_positions[&root] + alignment.inner_shifts[&node_id] + port_center(port_id)
    };
    let delta = port_position(block_port) - port_position(fix_port);
    let block_node = arena.ports[block_port as usize].node;
    let block_root = alignment.roots[&block_node];
    #[cfg(feature = "diagnostic-traces")]
    let before = root_positions[&block_root];
    let (available, shift) = if delta > 0.0 {
        (
            bk_check_space_above(
                arena,
                alignment,
                layer_nodes,
                positions,
                root_positions,
                block_node,
                delta,
            ),
            -1.0,
        )
    } else if delta < 0.0 {
        (
            bk_check_space_below(
                arena,
                alignment,
                layer_nodes,
                positions,
                root_positions,
                block_node,
                -delta,
            ),
            1.0,
        )
    } else {
        (0.0, 0.0)
    };
    *root_positions
        .get_mut(&block_root)
        .expect("BK root position") += shift * available;
    #[cfg(feature = "diagnostic-traces")]
    let after = root_positions[&block_root];
    #[cfg(feature = "diagnostic-traces")]
    crate::trace::emit_value(
        "placement",
        "bk_postprocess",
        &format!(
            "graphs[{}].nodes[{}]",
            arena.nodes[block_node as usize].graph,
            bk_trace_node_id(arena, block_node),
        ),
        &format!(
            "vertical={vertical:?};horizontal={horizontal:?};free={};edge={edge_id};block={};delta_bits={:016x};available_bits={:016x};before_bits={:016x};after_bits={:016x}",
            bk_trace_node_id(arena, processable.free),
            bk_trace_node_id(arena, block_node),
            delta.to_bits(),
            available.to_bits(),
            before.to_bits(),
            after.to_bits(),
        ),
    );
    available > 0.0
}

fn bk_check_space_above(
    arena: &LayeredArena,
    alignment: &BkAlignment,
    layer_nodes: &BTreeMap<LNodeId, Vec<LNodeId>>,
    positions: &BTreeMap<LNodeId, usize>,
    root_positions: &BTreeMap<LNodeId, f64>,
    block_root: LNodeId,
    mut available: f64,
) -> f64 {
    let mut current = block_root;
    loop {
        current = alignment.aligns[&current];
        let root = alignment.roots[&current];
        let node = &arena.nodes[current as usize];
        let minimum = root_positions[&root] + alignment.inner_shifts[&current] - margin_top(node);
        let layer_id = node.layer.expect("BK aligned node requires assigned layer") as LNodeId;
        let index = positions[&current];
        if index > 0 {
            let neighbor = layer_nodes[&layer_id][index - 1];
            let neighbor_root = alignment.roots[&neighbor];
            let neighbor_node = &arena.nodes[neighbor as usize];
            let maximum = root_positions[&neighbor_root]
                + alignment.inner_shifts[&neighbor]
                + neighbor_node.height
                + margin_bottom(neighbor_node);
            let spacing = bk_vertical_spacing(arena, current, neighbor, 0.0);
            available = available.min(minimum - (maximum + spacing));
        }
        if current == block_root {
            return available;
        }
    }
}

fn bk_check_space_below(
    arena: &LayeredArena,
    alignment: &BkAlignment,
    layer_nodes: &BTreeMap<LNodeId, Vec<LNodeId>>,
    positions: &BTreeMap<LNodeId, usize>,
    root_positions: &BTreeMap<LNodeId, f64>,
    block_root: LNodeId,
    mut available: f64,
) -> f64 {
    let mut current = block_root;
    loop {
        current = alignment.aligns[&current];
        let root = alignment.roots[&current];
        let node = &arena.nodes[current as usize];
        let maximum = root_positions[&root]
            + alignment.inner_shifts[&current]
            + node.height
            + margin_bottom(node);
        let layer_id = node.layer.expect("BK aligned node requires assigned layer") as LNodeId;
        let index = positions[&current];
        let nodes = &layer_nodes[&layer_id];
        if index + 1 < nodes.len() {
            let neighbor = nodes[index + 1];
            let neighbor_root = alignment.roots[&neighbor];
            let neighbor_node = &arena.nodes[neighbor as usize];
            let minimum = root_positions[&neighbor_root] + alignment.inner_shifts[&neighbor]
                - margin_top(neighbor_node);
            let spacing = bk_vertical_spacing(arena, current, neighbor, 0.0);
            available = available.min(minimum - (maximum + spacing));
        }
        if current == block_root {
            return available;
        }
    }
}

/// Selects a BK placement from the four directional layouts.
///
/// With `balanced` enabled this follows ELK's median-of-four selection after
/// aligning candidates to the smallest directional layout. Otherwise it
/// chooses the smallest valid candidate, preserving directional declaration
/// order for ties. This keeps the source selection boundary observable before
/// the full conflict and threshold machinery is added.
pub fn choose_bk_layout(
    arena: &LayeredArena,
    graph_id: LGraphId,
    node_spacing: f64,
    balanced: bool,
    marked_edges: &BTreeSet<LEdgeId>,
) -> Result<BTreeMap<LNodeId, f64>, LayeringError> {
    let directions = [
        (BkVerticalDirection::Down, BkHorizontalDirection::Right),
        (BkVerticalDirection::Up, BkHorizontalDirection::Right),
        (BkVerticalDirection::Down, BkHorizontalDirection::Left),
        (BkVerticalDirection::Up, BkHorizontalDirection::Left),
    ];
    let mut candidates = Vec::with_capacity(directions.len());
    for (vertical, horizontal) in directions {
        let mut alignment =
            build_bk_alignment(arena, graph_id, vertical, horizontal, marked_edges)?;
        apply_bk_inner_shifts(arena, &mut alignment, horizontal);
        let compaction = compact_bk_alignment_directional(
            arena,
            graph_id,
            &alignment,
            node_spacing,
            vertical,
            horizontal,
        )?;
        let candidate = (
            compaction.node_positions,
            alignment,
            vertical,
            compaction.block_positions,
        );
        trace_bk_candidate!(
            candidates.len(),
            vertical,
            horizontal,
            candidate_layout_size(arena, &candidate),
            candidate_order_is_valid(arena, graph_id, &candidate.0),
        );
        trace_bk_candidate_positions!(candidates.len(), &candidate.0, &candidate.1, arena);
        candidates.push(candidate);
    }
    if balanced {
        let minimum_width_layout = candidates
            .iter()
            .enumerate()
            .min_by(|(_, first), (_, second)| {
                candidate_layout_size(arena, first).total_cmp(&candidate_layout_size(arena, second))
            })
            .map(|(index, _)| index)
            .unwrap_or(0);
        let extents = candidates
            .iter()
            .map(|candidate| candidate_extents(arena, candidate))
            .collect::<Vec<_>>();
        let shifts = candidates
            .iter()
            .enumerate()
            .map(|(index, (_, _, vertical, _))| match vertical {
                BkVerticalDirection::Down => extents[minimum_width_layout].0 - extents[index].0,
                BkVerticalDirection::Up => extents[minimum_width_layout].1 - extents[index].1,
            })
            .collect::<Vec<_>>();
        let mut result = BTreeMap::new();
        let node_ids = candidates[0].0.keys().copied().collect::<Vec<_>>();
        for node_id in node_ids {
            let mut values = candidates
                .iter()
                .enumerate()
                .map(|(index, candidate)| candidate.0[&node_id] + shifts[index])
                .collect::<Vec<_>>();
            values.sort_by(f64::total_cmp);
            result.insert(node_id, (values[1] + values[2]) / 2.0);
        }
        if candidate_order_is_valid(arena, graph_id, &result) {
            return Ok(result);
        }
    }
    candidates
        .iter()
        .filter(|candidate| candidate_order_is_valid(arena, graph_id, &candidate.0))
        .min_by(|first, second| {
            candidate_layout_size(arena, first).total_cmp(&candidate_layout_size(arena, second))
        })
        .map(|candidate| candidate.0.clone())
        .or_else(|| candidates.first().map(|candidate| candidate.0.clone()))
        .ok_or(LayeringError::UnknownGraph(graph_id))
}

/// Selects a BK layout using ELK's fixed-alignment option.
pub fn choose_bk_layout_with_alignment(
    arena: &LayeredArena,
    graph_id: LGraphId,
    node_spacing: f64,
    alignment: BkFixedAlignment,
    marked_edges: &BTreeSet<LEdgeId>,
) -> Result<BTreeMap<LNodeId, f64>, LayeringError> {
    let direction = match alignment {
        BkFixedAlignment::LeftUp => Some((BkVerticalDirection::Up, BkHorizontalDirection::Left)),
        BkFixedAlignment::RightUp => Some((BkVerticalDirection::Up, BkHorizontalDirection::Right)),
        BkFixedAlignment::LeftDown => {
            Some((BkVerticalDirection::Down, BkHorizontalDirection::Left))
        }
        BkFixedAlignment::RightDown => {
            Some((BkVerticalDirection::Down, BkHorizontalDirection::Right))
        }
        BkFixedAlignment::None | BkFixedAlignment::Balanced => None,
    };
    let Some((vertical, horizontal)) = direction else {
        return choose_bk_layout(arena, graph_id, node_spacing, true, marked_edges);
    };
    let mut state = build_bk_alignment(arena, graph_id, vertical, horizontal, marked_edges)?;
    apply_bk_inner_shifts(arena, &mut state, horizontal);
    compact_bk_alignment_directional(arena, graph_id, &state, node_spacing, vertical, horizontal)
        .map(|compaction| compaction.node_positions)
}

/// Runs the bounded BK node-placement phase and writes cross-axis positions.
///
/// Ranking and crossing minimization remain explicit caller phases. This
/// function performs the source order boundary for phase four: mark conflicts,
/// build all directional alignments, compact their sink classes, select the
/// balanced or smallest valid result, and copy the result into the arena.
pub fn place_layers_bk(
    arena: &mut LayeredArena,
    graph_id: LGraphId,
    node_spacing: f64,
    balanced: bool,
) -> Result<(), LayeringError> {
    let marked = mark_bk_conflicts(arena, graph_id)?;
    let positions = choose_bk_layout(arena, graph_id, node_spacing, balanced, &marked)?;
    for (node_id, position) in positions {
        arena.nodes[node_id as usize].position_y = position;
    }
    Ok(())
}

/// Runs BK placement with an explicit ELK fixed-alignment choice.
pub fn place_layers_bk_with_alignment(
    arena: &mut LayeredArena,
    graph_id: LGraphId,
    node_spacing: f64,
    alignment: BkFixedAlignment,
) -> Result<(), LayeringError> {
    let marked = mark_bk_conflicts(arena, graph_id)?;
    let positions =
        choose_bk_layout_with_alignment(arena, graph_id, node_spacing, alignment, &marked)?;
    for (node_id, position) in positions {
        arena.nodes[node_id as usize].position_y = position;
    }
    Ok(())
}

fn candidate_layout_size(
    arena: &LayeredArena,
    candidate: &(
        BTreeMap<LNodeId, f64>,
        BkAlignment,
        BkVerticalDirection,
        BTreeMap<LNodeId, f64>,
    ),
) -> f64 {
    let (_, alignment, _, block_positions) = candidate;
    let mut minimum = f64::INFINITY;
    let mut maximum = f64::NEG_INFINITY;
    for root in alignment.roots.values().copied().collect::<BTreeSet<_>>() {
        // Source BKAlignedLayout.LayoutSize reads stored block Y. Adding an
        // inner shift and subtracting it again can change a bit and select a
        // different directional layout instead of preserving a source tie.
        let root_position = block_positions[&root];
        let block_size = alignment
            .block_sizes
            .get(&root)
            .copied()
            .unwrap_or_else(|| {
                let node = &arena.nodes[root as usize];
                margin_top(node) + node.height + margin_bottom(node)
            });
        minimum = minimum.min(root_position);
        maximum = maximum.max(root_position + block_size);
    }
    if minimum.is_finite() && maximum.is_finite() {
        maximum - minimum
    } else {
        0.0
    }
}

fn bk_vertical_spacing(
    arena: &LayeredArena,
    first: LNodeId,
    second: LNodeId,
    fallback: f64,
) -> f64 {
    let first_type = arena.nodes[first as usize].node_type;
    let second_type = arena.nodes[second as usize].node_type;
    match (first_type, second_type) {
        // layered/spacings.go uses portPort for external/external. Source
        // nodeSpacing compares both owning graph values in this operand order.
        (LayeredNodeType::ExternalPort, LayeredNodeType::ExternalPort) => {
            let first_spacing = arena
                .graphs
                .get(arena.nodes[first as usize].graph as usize)
                .map(|graph| graph.port_port_spacing)
                .unwrap_or(fallback);
            let second_spacing = arena
                .graphs
                .get(arena.nodes[second as usize].graph as usize)
                .map(|graph| graph.port_port_spacing)
                .unwrap_or(fallback);
            if first_spacing > second_spacing {
                first_spacing
            } else {
                second_spacing
            }
        }
        (LayeredNodeType::LongEdge, LayeredNodeType::LongEdge)
        | (LayeredNodeType::NorthSouthPort, LayeredNodeType::NorthSouthPort)
        | (LayeredNodeType::LongEdge, LayeredNodeType::NorthSouthPort)
        | (LayeredNodeType::NorthSouthPort, LayeredNodeType::LongEdge)
        | (LayeredNodeType::NorthSouthPort, LayeredNodeType::ExternalPort)
        | (LayeredNodeType::ExternalPort, LayeredNodeType::NorthSouthPort)
        | (LayeredNodeType::Label, LayeredNodeType::Label) => arena
            .graphs
            .get(arena.nodes[first as usize].graph as usize)
            .map(|graph| graph.edge_edge_spacing)
            .unwrap_or(fallback),
        (LayeredNodeType::Normal, LayeredNodeType::Normal) => arena
            .graphs
            .get(arena.nodes[first as usize].graph as usize)
            .map(|graph| graph.node_node_spacing)
            .unwrap_or(fallback),
        (LayeredNodeType::Normal, LayeredNodeType::Label)
        | (LayeredNodeType::Label, LayeredNodeType::Normal) => arena
            .graphs
            .get(arena.nodes[first as usize].graph as usize)
            .map(|graph| graph.node_node_spacing)
            .unwrap_or(fallback),
        _ => arena
            .graphs
            .get(arena.nodes[first as usize].graph as usize)
            .map(|graph| graph.edge_node_vertical_spacing)
            .unwrap_or(fallback),
    }
}

#[cfg(feature = "diagnostic-traces")]
fn bk_trace_node_id(arena: &LayeredArena, node_id: LNodeId) -> String {
    let source_id = &arena.nodes[node_id as usize].source_id;
    if source_id.starts_with("__long_edge_dummy_") {
        source_id
            .strip_prefix("__long_edge_dummy_")
            .and_then(|suffix| suffix.split_once('_'))
            .map(|(edge_id, _)| edge_id)
            .map(|edge_id| format!("edge-segment:{edge_id}"))
            .unwrap_or_else(|| source_id.clone())
    } else {
        source_id.clone()
    }
}

fn candidate_extents(
    arena: &LayeredArena,
    candidate: &(
        BTreeMap<LNodeId, f64>,
        BkAlignment,
        BkVerticalDirection,
        BTreeMap<LNodeId, f64>,
    ),
) -> (f64, f64) {
    let (positions, _, _, _) = candidate;
    let mut minimum = f64::INFINITY;
    let mut maximum = f64::NEG_INFINITY;
    for (&node_id, &position) in positions {
        let node = &arena.nodes[node_id as usize];
        minimum = minimum.min(position);
        maximum = maximum.max(position + node.height);
    }
    (minimum, maximum)
}

fn candidate_order_is_valid(
    arena: &LayeredArena,
    graph_id: LGraphId,
    positions: &BTreeMap<LNodeId, f64>,
) -> bool {
    let Some(graph) = arena.graphs.get(graph_id as usize) else {
        return false;
    };
    for &layer_id in &graph.layers {
        let mut previous_bottom = f64::NEG_INFINITY;
        for &node_id in &arena.layers[layer_id as usize].nodes {
            let node = &arena.nodes[node_id as usize];
            let position = positions.get(&node_id).copied().unwrap_or(0.0);
            let top = position - margin_top(node);
            let bottom = position + node.height + margin_bottom(node);
            if top <= previous_bottom || bottom <= previous_bottom {
                return false;
            }
            previous_bottom = bottom;
        }
    }
    true
}

fn bk_reversed(arena: &LayeredArena, node_layer: &BTreeMap<LNodeId, usize>) -> bool {
    // Some Rust layer builders retain ranks in reverse source order. Infer
    // that orientation from the active semantic edges. External-port edges
    // can point against the main graph flow, so use normal-to-normal edges
    // first and fall back to dummy edges only for graphs without such edges.
    let orientation_votes = |normal_nodes_only: bool| {
        let mut forward = 0usize;
        let mut backward = 0usize;
        for edge in arena.edges.iter().filter(|edge| edge.active) {
            let Some(&source_layer) = node_layer.get(&edge.source) else {
                continue;
            };
            let Some(&target_layer) = node_layer.get(&edge.target) else {
                continue;
            };
            if source_layer == target_layer {
                continue;
            }
            if normal_nodes_only
                && (arena.nodes[edge.source as usize].node_type != LayeredNodeType::Normal
                    || arena.nodes[edge.target as usize].node_type != LayeredNodeType::Normal)
            {
                continue;
            }
            if source_layer < target_layer {
                forward += 1;
            } else {
                backward += 1;
            }
        }
        (forward, backward)
    };
    let (forward, backward) = orientation_votes(true);
    let (forward, backward) = if forward == 0 && backward == 0 {
        orientation_votes(false)
    } else {
        (forward, backward)
    };
    // External-port and retired edges can point against the main graph flow.
    // Infer the layer-list direction from semantic node edges first, then use
    // active dummy edges only when a graph has no normal-to-normal edges.
    backward > forward
}

fn bk_neighbors(
    arena: &LayeredArena,
    node_id: LNodeId,
    layer_index: usize,
    horizontal: BkHorizontalDirection,
    reversed: bool,
    node_layer: &BTreeMap<LNodeId, usize>,
    node_position: &BTreeMap<LNodeId, usize>,
) -> Vec<(LNodeId, LEdgeId, usize)> {
    let wanted_layer = match horizontal {
        BkHorizontalDirection::Right if reversed => Some(layer_index + 1),
        BkHorizontalDirection::Right => layer_index.checked_sub(1),
        BkHorizontalDirection::Left if reversed => layer_index.checked_sub(1),
        BkHorizontalDirection::Left => Some(layer_index + 1),
    };
    let Some(wanted_layer) = wanted_layer else {
        return Vec::new();
    };
    let mut neighbors = Vec::new();
    for (edge_id, edge) in arena.connected_edges(node_id) {
        let (neighbor, is_predecessor) = if edge.source == node_id {
            (edge.target, false)
        } else {
            (edge.source, true)
        };
        let Some(&neighbor_layer) = node_layer.get(&neighbor) else {
            continue;
        };
        let valid = match horizontal {
            BkHorizontalDirection::Right => is_predecessor && neighbor_layer == wanted_layer,
            BkHorizontalDirection::Left => !is_predecessor && neighbor_layer == wanted_layer,
        };
        if valid {
            neighbors.push((neighbor, edge_id, node_position[&neighbor]));
        }
    }
    neighbors.sort_by_key(|neighbor| neighbor.2);
    neighbors
}

#[cfg(test)]
mod tests {
    use super::{
        BkFixedAlignment, BkHorizontalDirection, BkVerticalDirection, JavaRandom, LNodeId, Layer,
        LayerDirection, LayeredArena, LayeringError, PortSide, apply_bk_inner_shifts,
        assign_longest_path_layers, assign_network_simplex_layers, break_cycles_greedy_model_order,
        build_bk_alignment, calculate_layer_sizes_and_graph_height, choose_bk_layout,
        compact_bk_alignment, count_layer_crossings, distribute_barycenter_ports, join_long_edges,
        layer_nodes, mark_bk_conflicts, materialize_implicit_ports, order_layers_by_barycenter,
        order_layers_by_barycenter_with_model_order, p3_hierarchy_plan, p3_sweep_decision,
        place_layers_bk, place_layers_bk_with_alignment, place_layers_simple,
        redistribute_ordered_implicit_ports, refresh_implicit_port_positions, route_polyline_edges,
        sort_layers_by_input_model_order, split_deep_cross_hierarchy_edges, split_long_edges,
    };
    use crate::model::{ElkEdge, ElkGraph, ElkNode, ElkPort};
    use d2_rs_core::Point;
    use serde_json::json;
    use std::collections::BTreeMap;

    #[test]
    fn original_go_model_order_ties_do_not_follow_component_discovery_order() {
        let elk = graph(
            vec![node("a"), node("b"), node("c")],
            vec![("a", "b"), ("b", "c"), ("c", "a")],
        );
        let (mut arena, root) = LayeredArena::from_elk_graph(&elk).unwrap();
        materialize_implicit_ports(&mut arena, root).unwrap();
        arena.graphs[root as usize].layerless_nodes = vec![2, 1, 0];
        assert_eq!(
            break_cycles_greedy_model_order(&mut arena, root).unwrap(),
            vec![2]
        );
        assert_eq!((arena.edges[2].source, arena.edges[2].target), (0, 2));
    }

    #[test]
    fn original_go_sweep_publication_freezes_only_member_ports() {
        let elk = graph(vec![node("a"), node("b")], Vec::new());
        let (mut arena, root) = LayeredArena::from_elk_graph(&elk).unwrap();
        arena.layers.push(Layer {
            graph: root,
            nodes: vec![0],
            width: 0.0,
            height: 0.0,
        });
        arena.graphs[root as usize].layers = vec![0];
        arena.nodes[0].layer = Some(0);
        let other_constraints = arena.nodes[1].port_constraints;
        let snapshot = super::capture_p3_order(&arena, root);
        super::restore_p3_order(&mut arena, &snapshot, false);
        assert_ne!(
            arena.nodes[0].port_constraints,
            super::PortConstraints::FixedOrder
        );
        super::restore_p3_order(&mut arena, &snapshot, true);
        assert_eq!(
            arena.nodes[0].port_constraints,
            super::PortConstraints::FixedOrder
        );
        assert_eq!(arena.nodes[1].port_constraints, other_constraints);
    }

    #[test]
    fn original_go_unknown_barycenter_fill_has_one_f64_rounding() {
        let draw = f32::from_bits(0x3f63d927);
        let maximum = f64::from_bits(0x400647eeba800000);
        assert_eq!(
            super::source_unknown_barycenter(draw, maximum).to_bits(),
            0x3ff7a95e1b61e9d3
        );
        assert_ne!(
            (f64::from(draw) * maximum - 1.0).to_bits(),
            0x3ff7a95e1b61e9d3
        );
    }

    #[test]
    fn original_go_bk_selection_reads_stored_block_y() {
        // Original dense seed7 DOWN candidate0. Reconstructing root6's Y
        // from placed_y - inner_shift changes the bit that decides the tie.
        let arena = LayeredArena::default();
        let mut alignment = super::BkAlignment::default();
        alignment.roots.extend([(0, 0), (6, 6)]);
        alignment
            .inner_shifts
            .extend([(0, 17.916666666666664), (6, 28.666666666666664)]);
        alignment
            .block_sizes
            .extend([(0, 47.916666666666664), (6, 43.0)]);
        let stored_y = 85.20833333333333_f64;
        assert_ne!(
            (113.875_f64 - alignment.inner_shifts[&6]).to_bits(),
            stored_y.to_bits()
        );
        let candidate = (
            BTreeMap::from([(0, 17.916666666666664), (6, 113.875)]),
            alignment,
            BkVerticalDirection::Down,
            BTreeMap::from([(0, 0.0), (6, stored_y)]),
        );
        assert_eq!(
            super::candidate_layout_size(&arena, &candidate).to_bits(),
            0x406006aaaaaaaaaa
        );
    }

    #[test]
    fn original_go_hierarchy_boundary_uses_parent_order_and_rejects_missing_dummy() {
        for (side, side_name, forward) in [
            (PortSide::East, "EAST", false),
            (PortSide::West, "WEST", true),
        ] {
            let mut parent = node("group");
            parent.layout_options = Some(json!({"elk.portConstraints":"FIXED_SIDE"}));
            parent.children.push(node("child"));
            parent.ports = (0..2)
                .map(|index| ElkPort {
                    id: format!("group.p{index}"),
                    x: 0.0,
                    y: index as f64,
                    width: 0.0,
                    height: 0.0,
                    layout_options: json!({"elk.port.side":side_name}),
                })
                .collect();
            let (mut arena, _) =
                LayeredArena::from_elk_graph(&graph(vec![parent], vec![])).unwrap();
            let child_graph = arena.nodes[0].nested_graph.unwrap();
            let ports = arena.nodes[0].ports.clone();
            let a = super::add_external_port_dummy(&mut arena, child_graph, side, ports[0], 0).0;
            let b = super::add_external_port_dummy(&mut arena, child_graph, side, ports[1], 1).0;
            let boundary = arena.layers.len() as super::LayerId;
            arena.layers.push(Layer {
                graph: child_graph,
                nodes: vec![a, b],
                ..Layer::default()
            });
            arena.nodes[0].ports.reverse();
            // Port and dummy ordinates deliberately disagree with list order.
            arena.ports[ports[0] as usize].y = 1000.0;
            arena.ports[ports[1] as usize].y = -1000.0;
            arena.nodes[a as usize].position_y = -20.0;
            arena.nodes[b as usize].position_y = 20.0;
            super::sort_external_boundary_from_parent_ports(
                &mut arena,
                0,
                child_graph,
                boundary,
                forward,
            );
            let expected = if forward { vec![a, b] } else { vec![b, a] };
            assert_eq!(arena.layers[boundary as usize].nodes, expected);
            arena.external_port_origins.remove(&b);
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                super::sort_external_boundary_from_parent_ports(
                    &mut arena,
                    0,
                    child_graph,
                    boundary,
                    forward,
                );
            }));
            assert!(
                outcome.is_err(),
                "missing a source boundary must fail rather than drop it"
            );
            assert_eq!(arena.layers[boundary as usize].nodes, expected);
        }
    }

    #[test]
    fn p3_hierarchy_decider_matches_external_port_path_threshold() {
        let port = |id: &str, side: &str| ElkPort {
            id: id.into(),
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
            layout_options: json!({"elk.port.side": side}),
        };
        let mut container = node("container");
        container.ports = vec![
            port("east-a", "EAST"),
            port("east-b", "EAST"),
            port("west-a", "WEST"),
            port("west-b", "WEST"),
        ];
        let mut child = node("child");
        child.ports.push(port("child.east", "EAST"));
        container.children.push(child);
        let (mut arena, root) =
            LayeredArena::from_elk_graph(&graph(vec![container], vec![])).unwrap();
        let child_graph = arena.nodes[0].nested_graph.unwrap();
        let child_node = arena.graphs[child_graph as usize].layerless_nodes[0];
        let source_port = arena.nodes[child_node as usize].ports[0];
        let external_dummy =
            super::add_external_port_dummy(&mut arena, child_graph, PortSide::East, source_port, 1);
        let edge = super::add_hierarchy_segment(
            &mut arena,
            child_node,
            source_port,
            external_dummy.0,
            external_dummy.1,
            1.0,
        );
        assert!(arena.edges[edge as usize].active);
        assert_eq!(
            arena.nodes[child_node as usize].node_type,
            super::LayeredNodeType::Normal
        );
        assert_eq!(arena.nodes[child_node as usize].ports.len(), 1);
        assert_eq!(arena.ports[source_port as usize].side, PortSide::East);
        assert!(super::p3_has_no_active_ports_on_side(
            &arena,
            child_node,
            PortSide::West
        ));
        assert_eq!(arena.outgoing_edges(child_node).count(), 1);
        arena.nodes[external_dummy.0 as usize].layer = Some(1);
        arena.nodes[child_node as usize].layer = Some(0);
        arena.layers = vec![
            Layer {
                graph: child_graph,
                nodes: vec![child_node],
                ..Layer::default()
            },
            Layer {
                graph: child_graph,
                nodes: vec![external_dummy.0],
                ..Layer::default()
            },
        ];
        arena.graphs[child_graph as usize].layers = vec![0, 1];
        arena.nodes[0].layer = Some(2);
        arena.layers.push(Layer {
            graph: root,
            nodes: vec![0],
            ..Layer::default()
        });
        arena.graphs[root as usize].layers = vec![2];

        let decision = p3_sweep_decision(&arena, child_graph);
        assert_eq!(decision.paths_to_random, 1, "{decision:?}");
        assert_eq!(decision.paths_to_hierarchical, 1);
        assert!(!decision.use_bottom_up);
        assert_eq!(decision.reason, "path_influence_threshold");
        assert!(p3_sweep_decision(&arena, root).use_bottom_up);
        let plan = p3_hierarchy_plan(&arena, root);
        assert_eq!(plan.breadth_first_graphs, vec![root, child_graph]);
        assert_eq!(plan.graphs_to_sweep_on, vec![root]);
    }

    #[test]
    fn p3_hierarchy_decider_propagates_north_south_dummy_influence() {
        let port = |id: &str, side: &str| ElkPort {
            id: id.into(),
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
            layout_options: json!({"elk.port.side": side}),
        };
        let mut container = node("container");
        container.ports = vec![
            port("container.east-a", "EAST"),
            port("container.east-b", "EAST"),
            port("container.west-a", "WEST"),
            port("container.west-b", "WEST"),
        ];
        let mut source = node("container.a");
        source.layout_options = Some(json!({"elk.portConstraints": "FIXED_SIDE"}));
        source.ports.push(port("container.a.north", "NORTH"));
        let mut target = node("container.b");
        target.ports.push(port("container.b.west", "WEST"));
        container.children = vec![source, target];
        let elk = graph(
            vec![container],
            vec![("container.a.north", "container.b.west")],
        );
        let (mut arena, root) = LayeredArena::from_elk_graph(&elk).unwrap();
        let child_graph = arena.nodes[0].nested_graph.unwrap();
        let source = arena.graphs[child_graph as usize].layerless_nodes[0];
        let target = arena.graphs[child_graph as usize].layerless_nodes[1];
        arena.nodes[source as usize].layer = Some(0);
        arena.nodes[target as usize].layer = Some(1);
        arena.layers = vec![
            Layer {
                graph: child_graph,
                nodes: vec![source],
                ..Layer::default()
            },
            Layer {
                graph: child_graph,
                nodes: vec![target],
                ..Layer::default()
            },
            Layer {
                graph: root,
                nodes: vec![0],
                ..Layer::default()
            },
        ];
        arena.graphs[child_graph as usize].layers = vec![0, 1];
        arena.graphs[root as usize].layers = vec![2];
        arena.nodes[0].layer = Some(2);

        super::preprocess_north_south_ports(&mut arena, child_graph);
        let decision = p3_sweep_decision(&arena, child_graph);
        assert_eq!(decision.paths_to_random, 4);
        assert_eq!(decision.paths_to_hierarchical, 0);
        assert!(decision.use_bottom_up);
    }

    #[test]
    fn p3_hierarchy_decider_shortcuts_when_parent_has_few_ports() {
        let mut container = node("container");
        container.children.push(node("child"));
        let (arena, _) = LayeredArena::from_elk_graph(&graph(vec![container], vec![])).unwrap();
        let decision = p3_sweep_decision(&arena, 1);
        assert!(decision.use_bottom_up);
        assert_eq!(decision.reason, "fewer_than_two_parent_in_out_ports");
    }

    fn node(id: &str) -> ElkNode {
        ElkNode {
            edges: Vec::new(),
            id: id.to_owned(),
            x: 0.0,
            y: 0.0,
            width: 40.0,
            height: 30.0,
            children: Vec::new(),
            ports: Vec::new(),
            labels: Vec::new(),
            layout_options: None,
        }
    }

    fn graph(nodes: Vec<ElkNode>, edges: Vec<(&str, &str)>) -> ElkGraph {
        ElkGraph {
            children_present: false,
            edges_present: false,
            x: None,
            y: None,
            width: None,
            height: None,
            id: "root".into(),
            layout_options: json!({"elk.direction": "DOWN"}),
            children: nodes,
            edges: edges
                .into_iter()
                .enumerate()
                .map(|(index, (source, target))| ElkEdge {
                    junction_points: Vec::new(),
                    id: format!("e{index}"),
                    sources: vec![source.into()],
                    targets: vec![target.into()],
                    sections: Vec::new(),
                    labels: Vec::new(),
                    container: "root".into(),
                    layout_options: None,
                })
                .collect(),
        }
    }

    #[test]
    fn java_random_matches_the_v090_mixed_stream() {
        let mut random = JavaRandom::new(42);
        assert_eq!(random.next_long(), -5_025_562_857_975_149_833);
        assert!(random.next_boolean());
        assert_eq!(random.next_double().to_bits(), 0x3fa88b8027841e70);
    }

    #[test]
    fn java_random_next_long_sign_extends_low_word_after_component_sweep() {
        let mut random = JavaRandom::new(1);
        let first_sweep_seed = random.next_long();
        assert_eq!(first_sweep_seed, -4_964_420_948_893_066_024);
        random.set_seed(first_sweep_seed);
        random.next_boolean();
        random.next_double();
        random.next_double();
        random.next_float();
        random.next_float();
        assert_eq!(random.next_long(), 5_344_530_164_410_839_401);
        random.next_boolean();
        // Go 1.27.0 / elk-go v0.2.0 uses Java's signed low-word addition.
        assert_eq!(random.next_long(), 5_020_000_651_067_704_409);
    }

    #[cfg(feature = "diagnostic-traces")]
    #[test]
    fn java_random_trace_records_exact_operations_and_draws() {
        crate::trace::begin();
        let mut random = JavaRandom::with_stream(42, "trace-test");
        assert_eq!(random.next_long(), -5_025_562_857_975_149_833);
        assert!(random.next_boolean());
        assert_eq!(random.next_double().to_bits(), 0x3fa88b8027841e70);
        let events = crate::trace::finish()
            .into_iter()
            .filter(|event| event.phase == "rng")
            .collect::<Vec<_>>();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].operation, "next_long");
        assert_eq!(events[0].stream.as_deref(), Some("trace-test"));
        assert_eq!(events[0].value.as_deref(), Some("-5025562857975149833"));
        assert_eq!(events[0].draw_count, Some(2));
        assert_eq!(events[1].operation, "next_boolean");
        assert_eq!(events[1].value.as_deref(), Some("true"));
        assert_eq!(events[1].draw_count, Some(3));
        assert_eq!(events[2].operation, "next_double");
        assert_eq!(events[2].float_bits, Some(0x3fa88b8027841e70));
        assert_eq!(events[2].draw_count, Some(5));
    }

    #[test]
    fn java_random_matches_the_v090_bounded_int_stream() {
        for (seed, bound, expected) in [
            (0, 100, &[60, 48, 29, 47, 15][..]),
            (42, 16, &[11, 0, 10, 0, 4][..]),
            (42, 7, &[1, 5, 6, 3, 5][..]),
            (
                i64::MIN,
                i32::MAX as usize,
                &[1_569_741_360, 1_785_505_948, 516_548_029][..],
            ),
        ] {
            let mut random = JavaRandom::new(seed);
            for &value in expected {
                assert_eq!(random.next_int_n(bound), value);
            }
        }
    }

    #[test]
    fn loop_raw_slots_resolve_zero_weight_cycles_in_source_order() {
        let elk = graph(vec![node("a")], vec![("a", "a"); 2]);
        let (mut arena, root) = LayeredArena::from_elk_graph(&elk).unwrap();
        materialize_implicit_ports(&mut arena, root).unwrap();
        arena.nodes[0].ports = vec![0, 2, 1, 3];
        assert_eq!(
            super::self_loop_dependencies(&arena, 0).unwrap(),
            vec![(0, 1, 0), (1, 0, 0)]
        );
        assert_eq!(
            super::self_loop_raw_slots(&arena, 0)
                .unwrap()
                .into_values()
                .collect::<Vec<_>>(),
            vec![0, 1]
        );
    }

    #[test]
    fn loop_dependencies_distinguish_nested_and_disjoint_spans() {
        let elk = graph(vec![node("a")], vec![("a", "a"); 3]);
        let (mut arena, root) = LayeredArena::from_elk_graph(&elk).unwrap();
        materialize_implicit_ports(&mut arena, root).unwrap();
        assert_eq!(
            super::self_loop_dependencies(&arena, 0).unwrap(),
            vec![(1, 0, 2), (2, 0, 2), (2, 1, 2)]
        );
        assert_eq!(
            super::self_loop_raw_slots(&arena, 0)
                .unwrap()
                .into_values()
                .collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        arena.nodes[0].self_loop_distribution = super::SelfLoopDistribution::Equally;
        materialize_implicit_ports(&mut arena, root).unwrap();
        assert!(super::self_loop_dependencies(&arena, 0).unwrap().is_empty());
        assert_eq!(
            super::self_loop_raw_slots(&arena, 0)
                .unwrap()
                .into_values()
                .collect::<Vec<_>>(),
            vec![0, 0, 0]
        );
        arena.loop_label_sizes.insert(0, (20.0, 10.0));
        assert!(super::self_loop_dependencies(&arena, 0).unwrap().is_empty());
        assert_eq!(
            super::self_loop_raw_slots(&arena, 0)
                .unwrap()
                .into_values()
                .collect::<Vec<_>>(),
            vec![0, 0, 0]
        );
    }

    #[test]
    fn pure_loop_restorer_publishes_clockwise_area_order() {
        let mut owner = node("a");
        owner.layout_options =
            Some(json!({"elk.layered.edgeRouting.selfLoopDistribution": "EQUALLY"}));
        let elk = graph(vec![owner], vec![("a", "a"); 5]);
        let (mut arena, root) = LayeredArena::from_elk_graph(&elk).unwrap();
        materialize_implicit_ports(&mut arena, root).unwrap();
        // The fifth loop wraps W -> N. Its target starts the north area,
        // its source ends the west area; middle loops keep stacked order.
        assert_eq!(arena.nodes[0].ports, vec![9, 0, 1, 4, 5, 2, 3, 6, 7, 8]);
        assert_eq!(
            super::self_loop_port_activity(&arena, 4).unwrap(),
            vec![
                true, false, false, false, false, false, false, false, false, true
            ]
        );
        assert_eq!(
            super::self_loop_port_activity(&arena, 0).unwrap(),
            vec![
                false, true, true, false, false, false, false, false, false, false
            ]
        );
        let slots = super::self_loop_compacted_slots(&arena, 0).unwrap();
        assert_eq!(slots[&0][super::PortSide::North as usize], Some(0));
        assert_eq!(slots[&4][super::PortSide::North as usize], Some(0));
        assert_eq!(slots[&4][super::PortSide::West as usize], Some(0));
        assert_eq!(slots[&4][super::PortSide::East as usize], None);
        materialize_implicit_ports(&mut arena, root).unwrap();
        assert_eq!(arena.nodes[0].ports, vec![9, 0, 1, 4, 5, 2, 3, 6, 7, 8]);
    }

    #[test]
    fn self_loop_baselines_include_existing_node_margins() {
        let mut owner = node("a");
        owner.width = 40.0;
        owner.height = 30.0;
        owner.layout_options = Some(json!({
            "elk.layered.edgeRouting.selfLoopDistribution": "EQUALLY",
            "elk.margins": {"top": 3, "right": 5, "bottom": 7, "left": 11}
        }));
        let mut elk = graph(vec![owner], vec![("a", "a"); 4]);
        elk.layout_options = json!({"elk.spacing.nodeSelfLoop": 50});
        let (mut arena, root) = LayeredArena::from_elk_graph(&elk).unwrap();
        materialize_implicit_ports(&mut arena, root).unwrap();
        arena.nodes[0].position_x = 100.0;
        arena.nodes[0].position_y = 200.0;
        // baselinePosition offsets each side by its original margin before
        // applying nodeSelfLoop; node translation is applied once.
        for (id, horizontal, coordinate) in [
            (0, true, 147.0),
            (1, true, 287.0),
            (2, false, 195.0),
            (3, false, 39.0),
        ] {
            let route = super::edge_route_points(&arena, id).unwrap();
            for bend in &route[1..3] {
                assert_eq!(if horizontal { bend.y } else { bend.x }, coordinate);
            }
        }
    }

    #[test]
    fn adjacent_side_self_loop_preserves_corner_in_both_directions() {
        use super::PortSide::{East, North, South, West};
        // OrthogonalSelfLoopRouter::addCornerBendPoints for each adjacent
        // pair, with fixed anchors and one routing slot on each side.
        for (first, second, corner) in [
            (North, East, (90.0, -50.0)),
            (East, South, (90.0, 80.0)),
            (South, West, (-50.0, 80.0)),
            (West, North, (-50.0, -50.0)),
        ] {
            for reverse in [false, true] {
                let mut owner = node("a");
                owner.width = 40.0;
                owner.height = 30.0;
                let mut elk = graph(vec![owner], vec![("a", "a")]);
                elk.layout_options = json!({"elk.spacing.nodeSelfLoop": 50});
                let (mut arena, root) = LayeredArena::from_elk_graph(&elk).unwrap();
                materialize_implicit_ports(&mut arena, root).unwrap();
                let sides = if reverse {
                    [second, first]
                } else {
                    [first, second]
                };
                for (port, side) in arena.ports.iter_mut().zip(sides) {
                    port.side = side;
                    (port.x, port.y) = match side {
                        North => (20.0, 0.0),
                        East => (40.0, 15.0),
                        South => (20.0, 30.0),
                        West => (0.0, 15.0),
                        _ => unreachable!(),
                    };
                }
                let route = super::edge_route_points(&arena, 0).unwrap();
                assert_eq!(route.len(), 5);
                assert_eq!((route[2].x, route[2].y), corner);
                for segment in route.windows(2) {
                    assert!(segment[0].x == segment[1].x || segment[0].y == segment[1].y);
                }
            }
        }
    }

    #[test]
    fn same_side_loop_bends_follow_upstream_side_baselines() {
        let mut owner = node("a");
        owner.width = 40.0;
        owner.height = 30.0;
        owner.layout_options =
            Some(json!({"elk.layered.edgeRouting.selfLoopDistribution": "EQUALLY"}));
        let mut elk = graph(vec![owner], vec![("a", "a"); 4]);
        elk.layout_options = json!({"elk.direction": "RIGHT", "elk.spacing.nodeSelfLoop": 50});
        let (mut arena, root) = LayeredArena::from_elk_graph(&elk).unwrap();
        materialize_implicit_ports(&mut arena, root).unwrap();
        // Pinned elk-go four-loop witness, translated to node-local space.
        let expected = [
            [(40.0 / 3.0, -50.0), (80.0 / 3.0, -50.0)],
            [(80.0 / 3.0, 80.0), (40.0 / 3.0, 80.0)],
            [(90.0, 10.0), (90.0, 20.0)],
            [(-50.0, 20.0), (-50.0, 10.0)],
        ];
        for (id, bends) in expected.into_iter().enumerate() {
            let route = super::edge_route_points(&arena, id as u32).unwrap();
            assert_eq!((route[1].x, route[1].y), bends[0]);
            assert_eq!((route[2].x, route[2].y), bends[1]);
        }
    }

    #[test]
    fn self_loop_distribution_matches_upstream_target_sequence() {
        use super::PortSide::{East, North, South, West};
        // elk-go intermediate/selfloop_test.go:
        // TestSelfLoopEqualDistributionUsesUpstreamTargetSequence.
        let mut owner = node("a");
        owner.layout_options =
            Some(json!({"elk.layered.edgeRouting.selfLoopDistribution": "EQUALLY"}));
        let elk = graph(vec![owner], vec![("a", "a"); 8]);
        let (mut arena, root) = LayeredArena::from_elk_graph(&elk).unwrap();
        materialize_implicit_ports(&mut arena, root).unwrap();
        let actual = arena
            .edges
            .iter()
            .map(|edge| {
                (
                    arena.ports[edge.source_port.unwrap() as usize].side,
                    arena.ports[edge.target_port.unwrap() as usize].side,
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            actual,
            vec![
                (North, North),
                (South, South),
                (East, East),
                (West, West),
                (West, North),
                (North, East),
                (South, West),
                (East, South)
            ]
        );
        let snapshot = arena.ports.clone();
        materialize_implicit_ports(&mut arena, root).unwrap();
        assert_eq!(
            arena.ports, snapshot,
            "resizing refresh must preserve side assignment"
        );
    }

    #[test]
    fn imports_stable_arena_order_and_matches_longest_path_layers() {
        let elk = graph(
            vec![node("a"), node("b"), node("c")],
            vec![("a", "b"), ("b", "c")],
        );
        let (mut arena, root) = LayeredArena::from_elk_graph(&elk).unwrap();
        assert_eq!(arena.graphs[root as usize].direction, LayerDirection::Down);
        assign_longest_path_layers(&mut arena, root).unwrap();
        assert_eq!(layer_nodes(&arena, root), vec![vec![0], vec![1], vec![2]]);
        assert!(arena.graphs[root as usize].layerless_nodes.is_empty());
        assert_eq!(arena.nodes[0].source_id, "a");
        assert_eq!(arena.nodes[2].source_id, "c");
    }

    #[test]
    fn importer_carries_network_simplex_thoroughness_with_elk_defaults() {
        let elk = graph(vec![node("a")], vec![]);
        let (arena, root) = LayeredArena::from_elk_graph(&elk).unwrap();
        assert_eq!(arena.graphs[root as usize].thoroughness, 7);

        let mut configured = graph(vec![node("a")], vec![]);
        configured.layout_options["elk.layered.thoroughness"] = json!(3);
        let (arena, root) = LayeredArena::from_elk_graph(&configured).unwrap();
        assert_eq!(arena.graphs[root as usize].thoroughness, 3);

        configured.layout_options["elk.layered.thoroughness"] = json!("5");
        let (arena, root) = LayeredArena::from_elk_graph(&configured).unwrap();
        assert_eq!(arena.graphs[root as usize].thoroughness, 5);

        configured.layout_options["elk.layered.thoroughness"] = json!(4.0);
        let (arena, root) = LayeredArena::from_elk_graph(&configured).unwrap();
        assert_eq!(arena.graphs[root as usize].thoroughness, 4);

        configured.layout_options["elk.layered.thoroughness"] = json!(4.5);
        let (arena, root) = LayeredArena::from_elk_graph(&configured).unwrap();
        assert_eq!(arena.graphs[root as usize].thoroughness, 7);

        configured.layout_options["elk.layered.thoroughness"] = json!(0);
        let (arena, root) = LayeredArena::from_elk_graph(&configured).unwrap();
        assert_eq!(arena.graphs[root as usize].thoroughness, 0);
    }

    #[test]
    fn original_go_phase_splits_inherit_edge_model_order() {
        let mut group = node("group");
        group.children = vec![node("group.a"), node("group.b"), node("group.c")];
        let elk = graph(
            vec![group, node("outside")],
            vec![
                ("outside", "group.a"),
                ("group.a", "group.b"),
                ("group.b", "group.c"),
                ("group.a", "group.c"),
            ],
        );
        let (mut arena, root) = LayeredArena::from_elk_graph(&elk).unwrap();
        arena.edge_model_order.insert(0, 71);
        arena.edge_model_order.insert(3, 29);
        materialize_implicit_ports(&mut arena, root).unwrap();
        split_deep_cross_hierarchy_edges(&mut arena, root).unwrap();
        assert!(!arena.edges[0].hierarchy_segments.is_empty());
        for segment in &arena.edges[0].hierarchy_segments {
            assert_eq!(arena.edge_model_order[segment], 71);
            assert_eq!(super::original_edge_model_order(&arena, *segment), 71);
        }
        let child = arena.nodes[0].nested_graph.unwrap();
        let before = arena.edges.len();
        assign_network_simplex_layers(&mut arena, child).unwrap();
        split_long_edges(&mut arena, child).unwrap();
        let copied = (before..arena.edges.len())
            .filter(|id| arena.edge_model_order.get(&(*id as u32)) == Some(&29))
            .collect::<Vec<_>>();
        assert!(
            !copied.is_empty(),
            "the long-edge outgoing segment retains ModelOrder"
        );
        for id in copied {
            assert_eq!(super::original_edge_model_order(&arena, id as u32), 29);
        }
    }

    #[test]
    fn original_go_label_pair_and_edge_node_spacing_are_independent() {
        let mut group = node("group");
        group.children = vec![node("child.a"), node("child.b")];
        group.layout_options = Some(json!({
            "elk.spacing.edgeNode": 13, "spacing.edgeNodeBetweenLayers": 61,
            "elk.spacing.edgeEdge": 11,
        }));
        let mut elk = graph(vec![group, node("other")], vec![]);
        elk.layout_options["elk.spacing.edgeNode"] = json!(40);
        elk.layout_options["elk.spacing.edgeEdge"] = json!(17.25);
        let (mut arena, root) = LayeredArena::from_elk_graph(&elk).unwrap();
        let child = arena.nodes[0].nested_graph.unwrap();
        assert_eq!(arena.graphs[root as usize].edge_node_spacing, 10.0);
        assert_eq!(arena.graphs[root as usize].edge_node_vertical_spacing, 40.0);
        assert_eq!(arena.graphs[child as usize].edge_node_spacing, 61.0);
        assert_eq!(
            arena.graphs[child as usize].edge_node_vertical_spacing,
            13.0
        );
        for id in [0, 1, 2, 3] {
            arena.nodes[id].node_type = super::LayeredNodeType::Label;
        }
        assert_eq!(super::bk_vertical_spacing(&arena, 0, 3, 99.0), 17.25);
        assert_eq!(super::bk_vertical_spacing(&arena, 1, 2, 99.0), 11.0);
        arena.nodes[0].node_type = super::LayeredNodeType::LongEdge;
        arena.nodes[3].node_type = super::LayeredNodeType::Normal;
        assert_eq!(super::bk_vertical_spacing(&arena, 0, 3, 99.0), 40.0);
    }

    #[test]
    fn original_go_external_port_spacing_uses_port_property_and_both_scopes() {
        use super::{LayeredNodeType, bk_vertical_spacing};
        for key in [
            "org.eclipse.elk.spacing.portPort",
            "elk.spacing.portPort",
            "spacing.portPort",
        ] {
            for value in [
                json!(23.57),
                json!("23.57"),
                json!(" 23.57f "),
                json!("23.57D"),
            ] {
                let mut group = node("group");
                group.children.push(node("child"));
                group.layout_options = Some(json!({key: value}));
                let mut elk = graph(vec![group], vec![]);
                elk.layout_options["elk.spacing.edgeNode"] = json!(40);
                let (mut arena, root) = LayeredArena::from_elk_graph(&elk).unwrap();
                let nested = arena.nodes[0].nested_graph.unwrap();
                assert_eq!(arena.graphs[root as usize].port_port_spacing, 10.0);
                assert_eq!(arena.graphs[nested as usize].port_port_spacing, 23.57);
                arena.nodes[0].node_type = LayeredNodeType::ExternalPort;
                arena.nodes[1].node_type = LayeredNodeType::ExternalPort;
                assert_eq!(bk_vertical_spacing(&arena, 0, 1, 99.0), 23.57);
                assert_eq!(bk_vertical_spacing(&arena, 1, 0, 99.0), 23.57);
                arena.graphs[root as usize].port_port_spacing = 50.0;
                assert_eq!(bk_vertical_spacing(&arena, 0, 1, 99.0), 50.0);
                assert_eq!(bk_vertical_spacing(&arena, 1, 0, 99.0), 50.0);
            }
        }
    }

    #[test]
    fn importer_keeps_general_and_between_layer_edge_spacing_separate() {
        let mut group = node("group");
        group.children.push(node("child"));
        group.layout_options = Some(json!({
            "elk.spacing.edgeEdge": 17,
            "elk.layered.spacing.edgeEdgeBetweenLayers": 70,
        }));
        let mut elk = graph(vec![group], vec![]);
        elk.layout_options["elk.spacing.edgeEdge"] = json!(11);
        elk.layout_options["elk.layered.spacing.edgeEdgeBetweenLayers"] = json!(50);

        let (arena, root) = LayeredArena::from_elk_graph(&elk).unwrap();
        let nested = arena.nodes[0].nested_graph.unwrap();
        assert_eq!(arena.graphs[root as usize].edge_edge_spacing, 11.0);
        assert_eq!(
            arena.graphs[root as usize].edge_edge_spacing_between_layers,
            50.0
        );
        assert_eq!(arena.graphs[nested as usize].edge_edge_spacing, 17.0);
        assert_eq!(
            arena.graphs[nested as usize].edge_edge_spacing_between_layers,
            70.0
        );
    }

    #[test]
    fn importer_carries_bk_fixed_alignment_per_graph_scope() {
        let mut group = node("group");
        group.children.push(node("child"));
        group.layout_options = Some(json!({
            "elk.layered.nodePlacement.bk.fixedAlignment": "RIGHTDOWN"
        }));
        let mut elk = graph(vec![group], vec![]);
        elk.layout_options["elk.layered.nodePlacement.bk.fixedAlignment"] = json!("BALANCED");

        let (arena, root) = LayeredArena::from_elk_graph(&elk).unwrap();
        let nested = arena.nodes[0].nested_graph.unwrap();
        assert_eq!(
            arena.graphs[root as usize].fixed_alignment,
            BkFixedAlignment::Balanced
        );
        assert_eq!(
            arena.graphs[nested as usize].fixed_alignment,
            BkFixedAlignment::RightDown
        );
    }

    #[test]
    fn importer_carries_edge_priority_shortness_into_network_simplex() {
        let mut configured = graph(vec![node("a"), node("b")], vec![("a", "b")]);
        configured.edges[0].layout_options = Some(json!({
            "org.eclipse.elk.layered.priority.shortness": 3
        }));
        let (arena, _) = LayeredArena::from_elk_graph(&configured).unwrap();
        assert_eq!(arena.edge_priority_shortness.get(&0), Some(&3.0));

        configured.edges[0].layout_options = Some(json!({
            "elk.layered.priority.shortness": "0"
        }));
        let (arena, _) = LayeredArena::from_elk_graph(&configured).unwrap();
        assert_eq!(arena.edge_priority_shortness.get(&0), Some(&1.0));
    }

    #[test]
    fn network_simplex_starts_disconnected_components_at_the_first_rank() {
        let elk = graph(
            vec![node("a"), node("b"), node("c"), node("d"), node("e")],
            vec![("a", "b"), ("b", "c"), ("d", "e")],
        );
        let (mut arena, root) = LayeredArena::from_elk_graph(&elk).unwrap();
        assign_network_simplex_layers(&mut arena, root).unwrap();
        assert_eq!(
            layer_nodes(&arena, root),
            vec![vec![0, 3], vec![1, 4], vec![2]]
        );
    }

    #[test]
    fn network_simplex_keeps_source_layers_before_long_edge_targets() {
        let mut elk = graph(
            vec![node("A"), node("B"), node("X"), node("C")],
            vec![("A", "C"), ("A", "X"), ("B", "X"), ("X", "C")],
        );
        elk.layout_options = json!({
            "elk.direction": "RIGHT",
            "elk.layered.considerModelOrder.strategy": "NODES_AND_EDGES",
            "elk.layered.cycleBreaking.strategy": "GREEDY_MODEL_ORDER",
            "elk.hierarchyHandling": "INCLUDE_CHILDREN",
            "spacing.nodeNodeBetweenLayers": 70,
            "spacing.edgeNodeBetweenLayers": 40,
            "elk.spacing.nodeSelfLoop": 50,
            "elk.spacing.edgeNode": 40,
            "elk.layered.nodePlacement.bk.fixedAlignment": "BALANCED",
            "elk.layered.thoroughness": 8,
            "elk.layered.spacing.edgeEdgeBetweenLayers": 50,
            "elk.nodeSize.constraints": "MINIMUM_SIZE",
            "elk.contentAlignment": "H_CENTER V_CENTER",
        });

        let (mut arena, root) = LayeredArena::from_elk_graph(&elk).unwrap();
        materialize_implicit_ports(&mut arena, root).unwrap();
        break_cycles_greedy_model_order(&mut arena, root).unwrap();
        assign_network_simplex_layers(&mut arena, root).unwrap();

        assert_eq!(
            layer_nodes(&arena, root),
            vec![vec![0, 1], vec![2], vec![3]],
            "elk-go's NetworkSimplexLayerer maps rank zero to the source layer"
        );
    }

    #[test]
    fn importer_preserves_explicit_graph_padding() {
        let mut elk = graph(vec![node("a")], Vec::new());
        elk.layout_options = json!({
            "elk.direction": "RIGHT",
            "elk.padding": "[top=3,left=5,bottom=7,right=9]",
        });
        let (arena, root) = LayeredArena::from_elk_graph(&elk).unwrap();
        assert_eq!(
            arena.graphs[root as usize].padding,
            Some(super::GraphPadding {
                top: 3.0,
                right: 9.0,
                bottom: 7.0,
                left: 5.0,
            })
        );
    }

    #[test]
    fn deep_cross_hierarchy_edges_allocate_one_segment_per_boundary() {
        let mut outer = node("outer");
        let mut middle = node("middle");
        middle.children = vec![node("leaf")];
        outer.children = vec![middle];
        let elk = ElkGraph {
            children_present: false,
            edges_present: false,
            x: None,
            y: None,
            width: None,
            height: None,
            id: "root".into(),
            layout_options: json!({"elk.direction": "RIGHT"}),
            children: vec![outer, node("target")],
            edges: vec![ElkEdge {
                junction_points: Vec::new(),
                id: "e".into(),
                sources: vec!["leaf".into()],
                targets: vec!["target".into()],
                sections: Vec::new(),
                labels: Vec::new(),
                container: "root".into(),
                layout_options: Some(json!({
                    "org.eclipse.elk.layered.priority.shortness": 3
                })),
            }],
        };
        let (mut arena, root) = LayeredArena::from_elk_graph(&elk).unwrap();
        materialize_implicit_ports(&mut arena, root).unwrap();
        split_deep_cross_hierarchy_edges(&mut arena, root).unwrap();
        assert!(!arena.edges[0].active);
        assert_eq!(arena.edges[0].hierarchy_segments.len(), 3);
        assert!(
            arena.edges[0]
                .hierarchy_segments
                .iter()
                .all(|edge_id| arena.edges[*edge_id as usize].active)
        );
        assert!(
            arena.edges[0]
                .hierarchy_segments
                .iter()
                .all(|edge_id| { arena.edge_priority_shortness.get(edge_id) == Some(&3.0) })
        );
    }

    #[test]
    fn longest_path_keeps_model_order_for_equal_rank_nodes() {
        let elk = graph(
            vec![node("a"), node("b"), node("c")],
            vec![("a", "c"), ("b", "c")],
        );
        let (mut arena, root) = LayeredArena::from_elk_graph(&elk).unwrap();
        assign_longest_path_layers(&mut arena, root).unwrap();
        assert_eq!(layer_nodes(&arena, root), vec![vec![0, 1], vec![2]]);
    }

    #[test]
    fn self_loop_is_ignored_but_a_real_cycle_is_reported() {
        let elk = graph(vec![node("a")], vec![("a", "a")]);
        let (mut arena, root) = LayeredArena::from_elk_graph(&elk).unwrap();
        assign_longest_path_layers(&mut arena, root).unwrap();
        assert_eq!(layer_nodes(&arena, root), vec![vec![0]]);

        let elk = graph(vec![node("a"), node("b")], vec![("a", "b"), ("b", "a")]);
        let (mut arena, root) = LayeredArena::from_elk_graph(&elk).unwrap();
        assert_eq!(
            assign_longest_path_layers(&mut arena, root),
            Err(LayeringError::Cycle(0))
        );
    }

    #[test]
    fn greedy_model_order_reverses_the_later_back_edge() {
        let elk = graph(vec![node("a"), node("b")], vec![("a", "b"), ("b", "a")]);
        let (mut arena, root) = LayeredArena::from_elk_graph(&elk).unwrap();
        let reversed = break_cycles_greedy_model_order(&mut arena, root).unwrap();
        assert_eq!(reversed, vec![1]);
        assert_eq!(
            arena
                .edges
                .iter()
                .map(|edge| (edge.source, edge.target))
                .collect::<Vec<_>>(),
            vec![(0, 1), (0, 1)]
        );
        assert!(!arena.edges[0].reversed);
        assert!(arena.edges[1].reversed);
        assign_longest_path_layers(&mut arena, root).unwrap();
        assert_eq!(layer_nodes(&arena, root), vec![vec![0], vec![1]]);
    }

    #[test]
    fn nested_nodes_allocate_a_child_graph_after_the_parent() {
        let mut container = node("container");
        container.layout_options = Some(json!({"elk.randomSeed": "17"}));
        container.children = vec![node("child")];
        let mut elk = graph(vec![container, node("sibling")], vec![]);
        elk.layout_options["elk.direction"] = json!("LEFT");
        elk.layout_options["org.eclipse.elk.randomSeed"] = json!(9);
        let (arena, root) = LayeredArena::from_elk_graph(&elk).unwrap();
        assert_eq!(arena.graphs.len(), 2);
        assert_eq!(arena.graphs[1].parent_node, Some(0));
        assert_eq!(arena.graphs[1].layerless_nodes, vec![1]);
        assert_eq!(arena.graphs[root as usize].layerless_nodes, vec![0, 2]);
        assert_eq!(arena.graphs[root as usize].random_seed, 9);
        assert_eq!(arena.graphs[1].random_seed, 17);
        assert_eq!(arena.graphs[root as usize].direction, LayerDirection::Left);
        assert_eq!(arena.graphs[1].direction, LayerDirection::Left);
    }

    #[test]
    fn importer_resolves_port_endpoints_to_the_owning_node() {
        let mut source = node("source");
        source.ports.push(ElkPort {
            id: "source.port".into(),
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
            layout_options: json!({"org.eclipse.elk.port.side": "EAST"}),
        });
        let elk = graph(
            vec![source, node("target")],
            vec![("source.port", "target")],
        );
        let (arena, root) = LayeredArena::from_elk_graph(&elk).unwrap();
        assert_eq!(arena.edges[0].source, 0);
        assert_eq!(arena.edges[0].target, 1);
        assert_eq!(arena.edges[0].source_port, Some(0));
        assert_eq!(arena.ports[0].side, super::PortSide::East);
        assert_eq!(arena.ports[0].outgoing_edges, vec![0]);
        assert_eq!(arena.nodes[0].ports, vec![0]);
        assert_eq!(arena.graphs[root as usize].layerless_nodes, vec![0, 1]);
    }

    #[test]
    fn barycenter_sweep_removes_a_simple_two_edge_crossing() {
        let elk = graph(
            vec![node("a"), node("b"), node("c"), node("d")],
            vec![("a", "c"), ("b", "d")],
        );
        let (mut arena, root) = LayeredArena::from_elk_graph(&elk).unwrap();
        assign_longest_path_layers(&mut arena, root).unwrap();
        let target_layer = arena.graphs[root as usize].layers[1];
        arena.layers[target_layer as usize].nodes.reverse();
        assert_eq!(count_layer_crossings(&arena, root), 1);
        order_layers_by_barycenter(&mut arena, root, true).unwrap();
        assert_eq!(count_layer_crossings(&arena, root), 0);
        assert_eq!(layer_nodes(&arena, root), vec![vec![0, 1], vec![2, 3]]);
    }

    #[test]
    fn node_relative_port_ranks_match_v090_layer_cumulative_values() {
        let elk = graph(
            vec![node("a"), node("b"), node("c"), node("d")],
            vec![("a", "c"), ("b", "d")],
        );
        let (mut arena, root) = LayeredArena::from_elk_graph(&elk).unwrap();
        materialize_implicit_ports(&mut arena, root).unwrap();
        assign_longest_path_layers(&mut arena, root).unwrap();
        let fixed_nodes = arena.layers[arena.graphs[root as usize].layers[0] as usize]
            .nodes
            .clone();
        let ranks = super::calculate_port_ranks(&arena, &fixed_nodes, false, true);
        let source_ports = fixed_nodes
            .iter()
            .map(|node_id| arena.nodes[*node_id as usize].ports[0])
            .collect::<Vec<_>>();
        assert_eq!(ranks[source_ports[0] as usize], 0.5);
        assert_eq!(ranks[source_ports[1] as usize], 1.5);
    }

    #[test]
    fn model_order_sweep_uses_v090_port_ranks_and_resolves_crossed_diamond() {
        let mut elk = graph(
            vec![node("A"), node("B"), node("C"), node("D")],
            vec![("A", "D"), ("B", "C")],
        );
        elk.layout_options = json!({"elk.direction":"RIGHT"});
        let (mut arena, root) = LayeredArena::from_elk_graph(&elk).unwrap();
        materialize_implicit_ports(&mut arena, root).unwrap();
        break_cycles_greedy_model_order(&mut arena, root).unwrap();
        assign_network_simplex_layers(&mut arena, root).unwrap();
        assert_eq!(layer_nodes(&arena, root), vec![vec![0, 1], vec![3, 2]]);
        sort_layers_by_input_model_order(&mut arena, root);
        assert_eq!(layer_nodes(&arena, root), vec![vec![0, 1], vec![2, 3]]);

        order_layers_by_barycenter_with_model_order(&mut arena, root, 1, true).unwrap();

        assert_eq!(count_layer_crossings(&arena, root), 0);
        assert_eq!(
            layer_nodes(&arena, root),
            vec![vec![0, 1], vec![3, 2]],
            "the v0.9.0 layer-total port ranks order D before C"
        );
    }

    #[test]
    fn model_order_sorts_target_ports_by_previous_layer_order() {
        let mut elk = graph(
            vec![node("A"), node("B"), node("T")],
            vec![("A", "T"), ("B", "T")],
        );
        elk.layout_options = json!({
            "elk.direction":"RIGHT",
            "elk.layered.considerModelOrder.strategy":"NODES_AND_EDGES",
        });
        let (mut arena, root) = LayeredArena::from_elk_graph(&elk).unwrap();
        materialize_implicit_ports(&mut arena, root).unwrap();
        break_cycles_greedy_model_order(&mut arena, root).unwrap();
        assign_network_simplex_layers(&mut arena, root).unwrap();
        split_long_edges(&mut arena, root).unwrap();

        let target = arena
            .nodes
            .iter()
            .position(|node| node.source_id == "T")
            .unwrap() as u32;
        let incoming_sources = |arena: &LayeredArena, node_id: u32| {
            arena.nodes[node_id as usize]
                .ports
                .iter()
                .filter_map(|port_id| {
                    let port = &arena.ports[*port_id as usize];
                    let edge_id = *port.incoming_edges.first()?;
                    Some(
                        arena.nodes[arena.edges[edge_id as usize].source as usize]
                            .source_id
                            .clone(),
                    )
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(incoming_sources(&arena, target), vec!["A", "B"]);

        #[cfg(feature = "diagnostic-traces")]
        crate::trace::begin();
        sort_layers_by_input_model_order(&mut arena, root);

        assert_eq!(incoming_sources(&arena, target), vec!["B", "A"]);

        #[cfg(feature = "diagnostic-traces")]
        {
            crate::trace::emit_arena("ordering", "sort_by_input_model_order", &arena);
            let events = crate::trace::finish();
            let comparator_prefix = "sort.port.owner[2].compare[0]";
            let first_comparison = events
                .iter()
                .filter(|event| {
                    event.phase == "ordering"
                        && event.operation == "sort_by_input_model_order"
                        && (event.path == format!("{comparator_prefix}.begin")
                            || event.path == format!("{comparator_prefix}.result"))
                })
                .map(|event| (event.path.as_str(), event.value.as_deref()))
                .collect::<Vec<_>>();
            assert_eq!(first_comparison.len(), 2);
            assert_eq!(
                first_comparison[0],
                (
                    format!("{comparator_prefix}.begin").as_str(),
                    Some("first=1;second=3")
                )
            );
            assert_eq!(
                first_comparison[1],
                (
                    format!("{comparator_prefix}.result").as_str(),
                    Some("Greater")
                )
            );
            let target_port_order = events
                .iter()
                .filter(|event| {
                    event.phase == "ordering"
                        && event.operation == "sort_by_input_model_order"
                        && event.path.starts_with("arena.nodes[2](T).ports[")
                })
                .filter_map(|event| event.value.as_deref())
                .collect::<Vec<_>>();
            assert_eq!(target_port_order, vec!["3", "1"]);
        }
    }

    #[test]
    fn barycenter_port_distributor_orders_free_ports_by_fixed_endpoint_rank() {
        let port = |id: &str, side: &str| ElkPort {
            id: id.into(),
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
            layout_options: json!({"elk.port.side": side}),
        };
        let mut source_a = node("A");
        source_a.ports.push(port("A.out", "EAST"));
        let mut source_b = node("B");
        source_b.ports.push(port("B.out", "EAST"));
        let mut target = node("T");
        target.ports.push(port("T.fromA", "WEST"));
        target.ports.push(port("T.fromB", "WEST"));
        let mut elk = graph(vec![source_a, source_b, target], vec![]);
        elk.layout_options = json!({"elk.direction":"RIGHT"});
        elk.edges = vec![
            ElkEdge {
                junction_points: Vec::new(),
                id: "A-to-T".into(),
                sources: vec!["A.out".into()],
                targets: vec!["T.fromA".into()],
                sections: Vec::new(),
                labels: Vec::new(),
                container: "root".into(),
                layout_options: None,
            },
            ElkEdge {
                junction_points: Vec::new(),
                id: "B-to-T".into(),
                sources: vec!["B.out".into()],
                targets: vec!["T.fromB".into()],
                sections: Vec::new(),
                labels: Vec::new(),
                container: "root".into(),
                layout_options: None,
            },
        ];
        let (mut arena, root) = LayeredArena::from_elk_graph(&elk).unwrap();
        assign_network_simplex_layers(&mut arena, root).unwrap();
        let target = arena
            .nodes
            .iter()
            .position(|node| node.source_id == "T")
            .unwrap() as LNodeId;
        let mut node_positions = BTreeMap::new();
        for layer_id in &arena.graphs[root as usize].layers {
            for (position, node_id) in arena.layers[*layer_id as usize]
                .nodes
                .iter()
                .copied()
                .enumerate()
            {
                node_positions.insert(node_id, position);
            }
        }
        let mut ranks = vec![0.0_f32; arena.ports.len()];
        ranks[0] = 1.0;
        ranks[1] = 2.0;
        let mut barycenters = vec![0.0_f32; arena.ports.len()];

        distribute_barycenter_ports(
            &mut arena,
            target,
            super::PortSide::West,
            &ranks,
            &mut barycenters,
            &node_positions,
        );

        assert_eq!(
            arena.nodes[target as usize].ports,
            vec![3, 2],
            "the port with the higher fixed-side rank receives the smaller barycenter"
        );
    }

    #[test]
    fn model_order_moves_long_edge_dummy_before_later_incoming_node() {
        let mut elk = graph(
            vec![node("A"), node("B"), node("X"), node("C")],
            vec![("A", "C"), ("A", "X"), ("B", "X"), ("X", "C")],
        );
        elk.layout_options = json!({
            "elk.direction":"RIGHT",
            "elk.layered.considerModelOrder.strategy":"NODES_AND_EDGES",
        });
        let (mut arena, root) = LayeredArena::from_elk_graph(&elk).unwrap();
        materialize_implicit_ports(&mut arena, root).unwrap();
        break_cycles_greedy_model_order(&mut arena, root).unwrap();
        assign_network_simplex_layers(&mut arena, root).unwrap();
        split_long_edges(&mut arena, root).unwrap();

        let x = arena
            .nodes
            .iter()
            .position(|node| node.source_id == "X")
            .unwrap() as super::LNodeId;
        let layer_id = arena.nodes[x as usize].layer.unwrap();
        let dummy = arena.layers[layer_id as usize]
            .nodes
            .iter()
            .copied()
            .find(|node_id| {
                arena.nodes[*node_id as usize].node_type == super::LayeredNodeType::LongEdge
            })
            .expect("the A to C edge spans the middle layer");
        assert_eq!(
            arena.layers[layer_id as usize].nodes,
            vec![x, dummy],
            "the splitter appends dummies before the input-model pass"
        );

        #[cfg(feature = "diagnostic-traces")]
        crate::trace::begin();
        sort_layers_by_input_model_order(&mut arena, root);

        assert_eq!(
            arena.layers[layer_id as usize].nodes,
            vec![dummy, x],
            "elk-go orders the dummy by its source A, before X's source B"
        );

        #[cfg(feature = "diagnostic-traces")]
        {
            crate::trace::emit_arena("ordering", "sort_by_input_model_order", &arena);
            let events = crate::trace::finish();
            let comparison_prefix =
                format!("sort.node.layer[{layer_id}/before_ports=true].compare[0]");
            let expected_begin_path = format!("{comparison_prefix}.begin");
            let expected_result_path = format!("{comparison_prefix}.result");
            let expected_begin_value = format!("first={x};second={dummy}");
            let first_comparison = events
                .iter()
                .filter(|event| {
                    event.phase == "ordering"
                        && event.operation == "sort_by_input_model_order"
                        && (event.path == expected_begin_path || event.path == expected_result_path)
                })
                .map(|event| (event.path.as_str(), event.value.as_deref()))
                .collect::<Vec<_>>();
            assert_eq!(first_comparison.len(), 2);
            assert_eq!(
                first_comparison[0],
                (
                    expected_begin_path.as_str(),
                    Some(expected_begin_value.as_str())
                )
            );
            assert_eq!(
                first_comparison[1],
                (expected_result_path.as_str(), Some("Greater"))
            );
            let ordered_node_ids = events
                .iter()
                .filter(|event| {
                    event.phase == "ordering"
                        && event.operation == "sort_by_input_model_order"
                        && event
                            .path
                            .starts_with(&format!("arena.layers[{layer_id}].nodes["))
                })
                .filter_map(|event| event.value.as_deref())
                .collect::<Vec<_>>();
            let want_node_ids = [dummy.to_string(), x.to_string()];
            assert_eq!(
                ordered_node_ids,
                want_node_ids.iter().map(String::as_str).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn gwt_stable_sort_matches_v090_comparator_call_order() {
        let mut values = vec![6, 5, 4, 3, 2, 1, 0];
        let mut comparisons = Vec::new();
        super::gwt_stable_sort(&mut values, &mut |first, second| {
            comparisons.push((first, second));
            first.cmp(&second)
        });

        assert_eq!(values, vec![0, 1, 2, 3, 4, 5, 6]);
        assert_eq!(
            comparisons,
            vec![
                (6, 5),
                (6, 4),
                (5, 4),
                (3, 2),
                (3, 1),
                (2, 1),
                (3, 0),
                (2, 0),
                (1, 0),
                (6, 0),
                (4, 0),
                (4, 1),
                (4, 2),
                (4, 3),
            ]
        );

        let mut short_values = vec![3, 2, 1];
        let mut short_comparisons = Vec::new();
        super::gwt_stable_sort(&mut short_values, &mut |first, second| {
            short_comparisons.push((first, second));
            first.cmp(&second)
        });
        assert_eq!(short_values, vec![1, 2, 3]);
        assert_eq!(short_comparisons, vec![(3, 2), (3, 1), (2, 1)]);
    }

    #[test]
    fn model_order_groups_outgoing_ports_by_first_edge_to_target() {
        let mut elk = graph(
            vec![node("S"), node("T1"), node("T2")],
            vec![("S", "T2"), ("S", "T1"), ("S", "T1")],
        );
        elk.layout_options = json!({
            "elk.direction":"RIGHT",
            "elk.layered.considerModelOrder.strategy":"NODES_AND_EDGES",
        });
        let (mut arena, root) = LayeredArena::from_elk_graph(&elk).unwrap();
        materialize_implicit_ports(&mut arena, root).unwrap();
        break_cycles_greedy_model_order(&mut arena, root).unwrap();
        assign_network_simplex_layers(&mut arena, root).unwrap();
        split_long_edges(&mut arena, root).unwrap();

        let source = arena
            .nodes
            .iter()
            .position(|node| node.source_id == "S")
            .unwrap() as u32;
        let imported_ports = arena.nodes[source as usize].ports.clone();
        arena.nodes[source as usize].ports =
            vec![imported_ports[1], imported_ports[2], imported_ports[0]];

        sort_layers_by_input_model_order(&mut arena, root);

        let ordered_targets = arena.nodes[source as usize]
            .ports
            .iter()
            .map(|port_id| {
                let edge_id = arena.ports[*port_id as usize].outgoing_edges[0];
                arena.nodes[arena.edges[edge_id as usize].target as usize]
                    .source_id
                    .clone()
            })
            .collect::<Vec<_>>();
        assert_eq!(ordered_targets, vec!["T2", "T1", "T1"]);
    }

    #[test]
    fn model_order_follows_long_edge_ports_to_their_ultimate_targets() {
        let mut elk = graph(
            vec![node("S"), node("M"), node("T")],
            vec![("S", "T"), ("S", "M"), ("M", "T")],
        );
        elk.layout_options = json!({
            "elk.direction":"RIGHT",
            "elk.layered.considerModelOrder.strategy":"NODES_AND_EDGES",
        });
        let (mut arena, root) = LayeredArena::from_elk_graph(&elk).unwrap();
        materialize_implicit_ports(&mut arena, root).unwrap();
        break_cycles_greedy_model_order(&mut arena, root).unwrap();
        assign_network_simplex_layers(&mut arena, root).unwrap();
        split_long_edges(&mut arena, root).unwrap();

        let source = arena
            .nodes
            .iter()
            .position(|node| node.source_id == "S")
            .unwrap() as u32;
        let imported_ports = arena.nodes[source as usize].ports.clone();
        arena.nodes[source as usize].ports.reverse();

        sort_layers_by_input_model_order(&mut arena, root);

        let ordered_targets = arena.nodes[source as usize]
            .ports
            .iter()
            .map(|port_id| {
                let edge_id = arena.ports[*port_id as usize].outgoing_edges[0];
                let target = super::long_edge_target_node(&arena, edge_id).unwrap();
                arena.nodes[target as usize].source_id.clone()
            })
            .collect::<Vec<_>>();
        let first_target = arena.ports[imported_ports[0] as usize].outgoing_edges[0];
        assert_eq!(
            super::long_edge_target_node(&arena, first_target)
                .map(|node_id| arena.nodes[node_id as usize].source_id.as_str()),
            Some("T")
        );
        assert_eq!(ordered_targets, vec!["T", "M"]);
    }

    #[cfg(feature = "diagnostic-traces")]
    #[test]
    fn model_order_sweep_trace_matches_v090_rank_scores_and_rng_draws() {
        let mut elk = graph(
            vec![node("A"), node("B"), node("C"), node("D")],
            vec![("A", "D"), ("B", "C")],
        );
        elk.layout_options = json!({"elk.direction":"RIGHT"});
        let (mut arena, root) = LayeredArena::from_elk_graph(&elk).unwrap();
        materialize_implicit_ports(&mut arena, root).unwrap();
        break_cycles_greedy_model_order(&mut arena, root).unwrap();
        assign_network_simplex_layers(&mut arena, root).unwrap();
        assert_eq!(layer_nodes(&arena, root), vec![vec![0, 1], vec![3, 2]]);
        sort_layers_by_input_model_order(&mut arena, root);
        assert_eq!(layer_nodes(&arena, root), vec![vec![0, 1], vec![2, 3]]);

        crate::trace::begin();
        order_layers_by_barycenter_with_model_order(&mut arena, root, 1, true).unwrap();
        let events = crate::trace::finish();
        let candidates = events
            .iter()
            .filter(|event| event.operation == "candidate")
            .map(|event| (event.path.as_str(), event.float_bits.unwrap()))
            .collect::<Vec<_>>();
        assert_eq!(
            candidates,
            vec![
                ("layer[1].nodes[C]", 4611748415850676224),
                ("layer[1].nodes[D]", 4607280455757070336),
            ]
        );
        let rng = events
            .iter()
            .filter(|event| event.phase == "rng")
            .map(|event| (event.operation.as_str(), event.draw_count.unwrap()))
            .collect::<Vec<_>>();
        assert_eq!(
            rng,
            vec![
                ("next_long", 2),
                ("next_boolean", 3),
                ("set_seed", 0),
                ("next_boolean", 1),
                ("next_float", 2),
                ("next_float", 3),
            ]
        );
    }

    #[cfg(feature = "diagnostic-traces")]
    #[test]
    fn flat_root_sweep_trace_matches_v090_port_mode_and_long_edge_scores() {
        let mut nodes = vec![node("a"), node("b"), node("c")];
        for node in &mut nodes {
            node.width = 20.0;
            node.height = 20.0;
        }
        let mut elk = graph(nodes, vec![("a", "b"), ("b", "c"), ("a", "c")]);
        elk.layout_options = json!({"elk.direction":"RIGHT"});
        let (mut arena, root) = LayeredArena::from_elk_graph(&elk).unwrap();
        materialize_implicit_ports(&mut arena, root).unwrap();
        break_cycles_greedy_model_order(&mut arena, root).unwrap();
        assign_network_simplex_layers(&mut arena, root).unwrap();
        split_long_edges(&mut arena, root).unwrap();

        crate::trace::begin();
        super::order_layers_by_barycenter_with_random_seed(&mut arena, root, 1).unwrap();
        let events = crate::trace::finish();
        let port_mode = events
            .iter()
            .find(|event| event.operation == "port_rank")
            .map(|event| event.value.as_deref());
        assert_eq!(
            port_mode,
            Some(Some(
                "node_relative=false;forward=false;free_node_id=1;fixed_node_id=2;fixed_port_id=3;fixed_port_source=synthetic;fixed_port_order=3,5"
            ))
        );

        let candidates = events
            .iter()
            .filter(|event| event.operation == "candidate")
            .map(|event| (event.path.as_str(), event.float_bits.unwrap()))
            .collect::<Vec<_>>();
        assert_eq!(
            candidates,
            vec![
                // Go's BarycenterHeuristic traces the randomized boundary
                // before its len <= 1 early return, so the singleton `c`
                // still has one source-visible score.
                ("layer[2].nodes[c]", 4606244386374476520),
                ("layer[1].nodes[b]", 4611757313458438144),
                ("layer[1].nodes[__long_edge_dummy_2_3]", 4606946219841814528),
                ("layer[0].nodes[a]", 4609360582313771008),
            ],
            "Go 1.27.0 / elk-go v0.2.0 emits these exact seed-1 scores"
        );

        let port_mode_draw = events.iter().find(|event| {
            event.phase == "rng"
                && event.stream.as_deref() == Some("layout-seed")
                && event.operation == "next_boolean"
        });
        assert_eq!(port_mode_draw.and_then(|event| event.draw_count), Some(3));
        assert_eq!(
            port_mode_draw.and_then(|event| event.value.as_deref()),
            Some("false")
        );
    }

    #[test]
    fn go127_barycenter_perturbation_uses_fused_float32_arithmetic() {
        let draw = 0.407_439_77_f32;
        let perturbation = draw.mul_add(0.07_f32, -0.035_f32);
        assert_eq!(perturbation.to_bits(), 0xbbd4_4f9b);
    }

    #[test]
    fn simple_placer_stacks_a_layer_and_centers_shorter_layers() {
        let elk = graph(vec![node("a"), node("b")], vec![]);
        let (mut arena, root) = LayeredArena::from_elk_graph(&elk).unwrap();
        assign_longest_path_layers(&mut arena, root).unwrap();
        arena.nodes[0].height = 10.0;
        arena.nodes[1].height = 20.0;
        place_layers_simple(&mut arena, root, 20.0).unwrap();
        calculate_layer_sizes_and_graph_height(&mut arena, root).unwrap();
        assert_eq!(arena.nodes[0].position_y, 0.0);
        assert_eq!(arena.nodes[1].position_y, 30.0);
        assert_eq!(arena.layers[0].width, 40.0);
        assert_eq!(arena.layers[0].height, 50.0);
        assert_eq!(arena.graphs[root as usize].height, 50.0);
    }

    #[test]
    fn bk_alignment_uses_the_first_median_and_builds_a_diamond_block() {
        let elk = graph(
            vec![node("a"), node("b"), node("c"), node("d")],
            vec![("a", "b"), ("a", "c"), ("b", "d"), ("c", "d")],
        );
        let (mut arena, root) = LayeredArena::from_elk_graph(&elk).unwrap();
        assign_longest_path_layers(&mut arena, root).unwrap();
        let mut alignment = build_bk_alignment(
            &arena,
            root,
            BkVerticalDirection::Down,
            BkHorizontalDirection::Right,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(alignment.roots[&1], 0);
        assert_eq!(alignment.roots[&3], 0);
        assert_eq!(alignment.aligns[&1], 3);
        assert_eq!(alignment.aligns[&3], 0);
        apply_bk_inner_shifts(&arena, &mut alignment, BkHorizontalDirection::Right);
        assert_eq!(alignment.block_sizes[&0], 30.0);
        let positions =
            compact_bk_alignment(&arena, root, &alignment, 20.0, BkVerticalDirection::Down)
                .unwrap();
        assert_eq!(positions[&0], 0.0);
        assert_eq!(positions[&1], 0.0);
        assert_eq!(positions[&2], 50.0);
        assert_eq!(positions[&3], 0.0);
    }

    #[test]
    fn bk_directional_selection_returns_a_valid_balanced_order() {
        let elk = graph(
            vec![node("a"), node("b"), node("c"), node("d")],
            vec![("a", "b"), ("a", "c"), ("b", "d"), ("c", "d")],
        );
        let (mut arena, root) = LayeredArena::from_elk_graph(&elk).unwrap();
        assign_longest_path_layers(&mut arena, root).unwrap();
        let positions = choose_bk_layout(&arena, root, 20.0, true, &Default::default()).unwrap();
        assert!(positions.values().all(|position| position.is_finite()));
        assert!(positions[&1] < positions[&2]);
        assert_eq!(positions[&0], positions[&3]);
    }

    #[test]
    fn bk_balanced_diamond_matches_the_v090_source_witness() {
        let mut elk = graph(
            vec![node("a"), node("b"), node("c"), node("d")],
            vec![("a", "b"), ("a", "c"), ("b", "d"), ("c", "d")],
        );
        elk.children[1].height = 10.0;
        elk.children[2].height = 10.0;
        elk.children[1].layout_options = Some(json!({
            "org.eclipse.elk.margins": "[top=2,bottom=3]"
        }));
        let (mut arena, root) = LayeredArena::from_elk_graph(&elk).unwrap();
        assign_longest_path_layers(&mut arena, root).unwrap();
        place_layers_bk(&mut arena, root, 20.0, true).unwrap();
        assert_eq!(arena.nodes[0].position_y, 8.5);
        assert_eq!(arena.nodes[1].position_y, -8.0);
        assert_eq!(arena.nodes[2].position_y, 25.0);
        assert_eq!(arena.nodes[3].position_y, 8.5);
    }

    #[test]
    fn bk_fixed_left_up_matches_the_v090_source_witness() {
        let mut elk = graph(
            vec![node("a"), node("b"), node("c"), node("d")],
            vec![("a", "b"), ("a", "c"), ("b", "d"), ("c", "d")],
        );
        elk.children[1].height = 10.0;
        elk.children[2].height = 10.0;
        elk.children[1].layout_options = Some(json!({
            "org.eclipse.elk.margins": "[top=2,bottom=3]"
        }));
        let (mut arena, root) = LayeredArena::from_elk_graph(&elk).unwrap();
        assign_longest_path_layers(&mut arena, root).unwrap();
        place_layers_bk_with_alignment(&mut arena, root, 20.0, BkFixedAlignment::LeftUp).unwrap();
        assert_eq!(arena.nodes[0].position_y, 0.0);
        assert_eq!(arena.nodes[1].position_y, -33.0);
        assert_eq!(arena.nodes[2].position_y, 0.0);
        assert_eq!(arena.nodes[3].position_y, 0.0);
    }

    #[test]
    fn bk_compactor_keeps_independent_nodes_separated() {
        let mut elk = graph(vec![node("upper"), node("lower")], vec![]);
        elk.children[0].height = 10.0;
        elk.children[1].height = 10.0;
        let (mut arena, root) = LayeredArena::from_elk_graph(&elk).unwrap();
        assign_longest_path_layers(&mut arena, root).unwrap();
        place_layers_bk_with_alignment(&mut arena, root, 20.0, BkFixedAlignment::RightDown)
            .unwrap();
        assert_eq!(arena.nodes[1].position_y - arena.nodes[0].position_y, 30.0);
    }

    #[test]
    fn split_long_edges_inserts_ordered_dummies_and_ports() {
        let elk = graph(
            vec![node("a"), node("b"), node("c")],
            vec![("a", "b"), ("b", "c"), ("a", "c")],
        );
        let (mut arena, root) = LayeredArena::from_elk_graph(&elk).unwrap();
        assign_longest_path_layers(&mut arena, root).unwrap();
        split_long_edges(&mut arena, root).unwrap();
        assert_eq!(arena.edges.len(), 4);
        let middle = arena.graphs[root as usize].layers[1];
        let dummy = *arena.layers[middle as usize]
            .nodes
            .iter()
            .find(|&&node_id| {
                arena.nodes[node_id as usize].node_type == super::LayeredNodeType::LongEdge
            })
            .unwrap();
        assert_eq!(arena.nodes[dummy as usize].ports.len(), 2);
        assert_eq!(arena.edges[2].target, dummy);
        assert_eq!(arena.edges[3].source, dummy);
        assert_eq!(arena.edges[3].target, 2);
    }

    #[test]
    fn original_go_split_edge_preserves_reversed_and_chain_endpoints() {
        // long_edge.go SplitEdgeAtDummy copies Reversed; its dummy endpoint
        // properties retain original ports even when splitting a segment again.
        let elk = graph(
            vec![node("a"), node("b"), node("c")],
            vec![("a", "b"), ("b", "c"), ("a", "c")],
        );
        let (mut arena, root) = LayeredArena::from_elk_graph(&elk).unwrap();
        materialize_implicit_ports(&mut arena, root).unwrap();
        assign_longest_path_layers(&mut arena, root).unwrap();
        let original = arena.edges[2].clone();
        let endpoints = (original.source_port.unwrap(), original.target_port.unwrap());
        arena.edges[2].reversed = true;
        split_long_edges(&mut arena, root).unwrap();
        let first_dummy = arena.edges[2].target;
        assert_eq!(arena.long_edge_endpoints[&first_dummy], endpoints);
        let continuation =
            arena.ports[arena.nodes[first_dummy as usize].ports[1] as usize].outgoing_edges[0];
        assert!(arena.edges[2].reversed);
        assert!(arena.edges[continuation as usize].reversed);
        let middle = arena.graphs[root as usize].layers[1];
        let second_dummy = arena.add_long_edge_dummy(root, middle, continuation);
        let next = arena
            .split_edge_at_dummy(continuation, second_dummy, original.target)
            .unwrap();
        assert!(arena.edges[next as usize].reversed);
        assert_eq!(arena.long_edge_endpoints[&second_dummy], endpoints);
        assert_eq!(arena.edges[next as usize].target_port, original.target_port);
    }

    #[test]
    fn splitting_long_edges_preserves_endpoint_port_ranks() {
        let mut elk = graph(
            vec![node("A"), node("X"), node("C")],
            vec![("A", "C"), ("A", "X"), ("X", "C")],
        );
        elk.layout_options = json!({"elk.direction":"RIGHT"});
        let (mut arena, root) = LayeredArena::from_elk_graph(&elk).unwrap();
        materialize_implicit_ports(&mut arena, root).unwrap();
        break_cycles_greedy_model_order(&mut arena, root).unwrap();
        assign_network_simplex_layers(&mut arena, root).unwrap();

        let source_ports = [
            arena.edges[0].source_port.unwrap(),
            arena.edges[1].source_port.unwrap(),
        ];
        let target_ports = [
            arena.edges[0].target_port.unwrap(),
            arena.edges[2].target_port.unwrap(),
        ];
        let source_ranks = source_ports.map(|port| arena.ports[port as usize].y);
        let target_ranks = target_ports.map(|port| arena.ports[port as usize].y);

        split_long_edges(&mut arena, root).unwrap();

        assert_eq!(
            source_ports.map(|port| arena.ports[port as usize].y),
            source_ranks,
            "elk-go's long-edge splitter changes the edge topology without reversing source ports"
        );
        assert_eq!(
            target_ports.map(|port| arena.ports[port as usize].y),
            target_ranks,
            "elk-go's long-edge splitter changes the edge topology without reversing target ports"
        );
    }

    #[test]
    fn ordered_implicit_ports_receive_post_sweep_positions() {
        let mut nodes = vec![node("a"), node("b"), node("c")];
        for node in &mut nodes {
            node.width = 20.0;
            node.height = 20.0;
        }
        let elk = graph(nodes, vec![("a", "b"), ("b", "c"), ("a", "c")]);
        let (mut arena, root) = LayeredArena::from_elk_graph(&elk).unwrap();
        materialize_implicit_ports(&mut arena, root).unwrap();
        let first_source_port = arena.edges[0].source_port.unwrap();
        let long_source_port = arena.edges[2].source_port.unwrap();
        assert_eq!(
            arena.ports[first_source_port as usize].y,
            20.0 * (1.0 / 3.0)
        );
        assert_eq!(arena.ports[long_source_port as usize].y, 20.0 * (2.0 / 3.0));

        // Phase three has selected the long edge's port as the first port on
        // the east side. ELK's post-ordering size pass places ports from that
        // final list order before node placement and routing.
        arena.nodes[0].ports.reverse();
        redistribute_ordered_implicit_ports(&mut arena, root);

        assert_eq!(arena.ports[long_source_port as usize].y, 6.666666666666666);
        assert_eq!(
            arena.ports[first_source_port as usize].y,
            13.333333333333332
        );
    }

    #[test]
    fn hierarchy_scope_distributes_west_ports_in_source_order() {
        let elk = graph(
            vec![node("a"), node("b"), node("c")],
            vec![("a", "c"), ("b", "c")],
        );
        let (mut arena, root) = LayeredArena::from_elk_graph(&elk).unwrap();
        materialize_implicit_ports(&mut arena, root).unwrap();
        let first_target = arena.edges[0].target_port.unwrap();
        let second_target = arena.edges[1].target_port.unwrap();
        super::add_external_port_dummy(&mut arena, root, PortSide::West, first_target, -1);

        redistribute_ordered_implicit_ports(&mut arena, root);

        assert_eq!(arena.ports[first_target as usize].y, 20.0);
        assert_eq!(arena.ports[second_target as usize].y, 10.0);
    }

    #[test]
    fn join_long_edges_restores_the_original_edge_and_retires_segments() {
        let elk = graph(
            vec![node("a"), node("b"), node("c")],
            vec![("a", "b"), ("b", "c"), ("a", "c")],
        );
        let (mut arena, root) = LayeredArena::from_elk_graph(&elk).unwrap();
        assign_longest_path_layers(&mut arena, root).unwrap();
        split_long_edges(&mut arena, root).unwrap();
        let middle = arena.graphs[root as usize].layers[1];
        let dummy = arena.layers[middle as usize].nodes[1];
        arena.edges[2].bend_points.push(Point { x: 1.0, y: 2.0 });
        arena.edges[3].bend_points.push(Point { x: 3.0, y: 4.0 });
        arena.edges[3]
            .junction_points
            .push(Point { x: 3.0, y: 4.0 });
        join_long_edges(&mut arena, root).unwrap();

        assert_eq!(layer_nodes(&arena, root), vec![vec![0], vec![1], vec![2]]);
        assert_eq!(arena.edges[2].source, 0);
        assert_eq!(arena.edges[2].target, 2);
        assert!(arena.edges[2].active);
        assert!(!arena.edges[3].active);
        assert_eq!(
            arena.edges[2].bend_points,
            vec![Point { x: 1.0, y: 2.0 }, Point { x: 3.0, y: 4.0 }]
        );
        assert_eq!(
            arena.edges[2].junction_points,
            vec![Point { x: 3.0, y: 4.0 }]
        );
        assert!(arena.outgoing_edges(dummy).next().is_none());
        assert!(arena.incoming_edges(dummy).next().is_none());
    }

    #[test]
    fn polyline_router_matches_the_v090_in_layer_bend_order() {
        let mut source = node("source");
        source.width = 20.0;
        source.height = 20.0;
        source.ports.push(ElkPort {
            id: "source.port".into(),
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
            layout_options: json!({"org.eclipse.elk.port.side": "EAST"}),
        });
        let mut target = node("target");
        target.width = 20.0;
        target.height = 20.0;
        target.ports.push(ElkPort {
            id: "target.port".into(),
            x: 0.0,
            y: 30.0,
            width: 0.0,
            height: 0.0,
            layout_options: json!({"org.eclipse.elk.port.side": "EAST"}),
        });
        let elk = graph(vec![source, target], vec![]);
        let (mut arena, root) = LayeredArena::from_elk_graph(&elk).unwrap();
        assign_longest_path_layers(&mut arena, root).unwrap();
        arena.edges.push(super::LEdge {
            source: 0,
            target: 1,
            source_port: Some(0),
            target_port: Some(1),
            reversed: false,
            active: true,
            bend_points: Vec::new(),
            junction_points: Vec::new(),
            hierarchy_segments: Vec::new(),
        });
        arena.ports[0].outgoing_edges.push(0);
        arena.ports[1].incoming_edges.push(0);
        route_polyline_edges(&mut arena, root, 20.0, 6.0).unwrap();
        assert_eq!(
            arena.edges[0].bend_points,
            vec![
                Point { x: 20.0, y: 0.0 },
                Point { x: 26.0, y: 15.0 },
                Point { x: 20.0, y: 30.0 },
            ]
        );
    }

    #[test]
    fn polyline_router_reserves_slope_space_between_layers() {
        let mut source = node("source");
        source.width = 10.0;
        source.height = 10.0;
        source.ports.push(ElkPort {
            id: "source.port".into(),
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
            layout_options: json!({"org.eclipse.elk.port.side": "EAST"}),
        });
        let mut target = node("target");
        target.width = 10.0;
        target.height = 10.0;
        target.ports.push(ElkPort {
            id: "target.port".into(),
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
            layout_options: json!({"org.eclipse.elk.port.side": "WEST"}),
        });
        let elk = graph(vec![source, target], vec![("source.port", "target.port")]);
        let (mut arena, root) = LayeredArena::from_elk_graph(&elk).unwrap();
        arena.nodes[1].position_y = 30.0;
        assign_longest_path_layers(&mut arena, root).unwrap();
        route_polyline_edges(&mut arena, root, 20.0, 10.0).unwrap();
        assert_eq!(arena.nodes[1].position_x, 36.0);
        assert_eq!(arena.graphs[root as usize].width, 46.0);
        assert_eq!(arena.edges[0].bend_points, vec![Point { x: 10.0, y: 0.0 }]);
    }

    #[test]
    fn ordinary_inter_layer_edges_get_elk_orthogonal_lanes() {
        let elk = graph(
            vec![node("a"), node("b"), node("c"), node("d")],
            vec![("a", "b"), ("a", "c"), ("b", "d"), ("c", "d")],
        );
        let (mut arena, root) = LayeredArena::from_elk_graph(&elk).unwrap();
        assign_longest_path_layers(&mut arena, root).unwrap();
        place_layers_bk(&mut arena, root, 20.0, true).unwrap();
        route_polyline_edges(&mut arena, root, 20.0, 10.0).unwrap();

        assert_eq!(
            arena
                .nodes
                .iter()
                .map(|node| (node.position_x, node.position_y))
                .collect::<Vec<_>>(),
            vec![(0.0, 25.0), (60.0, 0.0), (60.0, 50.0), (120.0, 25.0)]
        );

        for edge_id in 0..arena.edges.len() {
            let edge = &arena.edges[edge_id];
            assert_eq!(edge.bend_points.len(), 2, "edge {edge_id}");
            assert_eq!(edge.bend_points[0].x, edge.bend_points[1].x);
            assert_eq!(
                edge.bend_points[0].y,
                arena.nodes[edge.source as usize].position_y
                    + arena.nodes[edge.source as usize].height / 2.0
            );
            assert_eq!(
                edge.bend_points[1].y,
                arena.nodes[edge.target as usize].position_y
                    + arena.nodes[edge.target as usize].height / 2.0
            );
        }
    }

    #[test]
    fn materialize_implicit_ports_matches_elk_default_endpoint_sides() {
        let elk = graph(
            vec![node("a"), node("b"), node("c")],
            vec![("a", "b"), ("a", "c")],
        );
        let (mut arena, root) = LayeredArena::from_elk_graph(&elk).unwrap();
        materialize_implicit_ports(&mut arena, root).unwrap();

        assert_eq!(arena.edges[0].source_port, Some(0));
        assert_eq!(arena.edges[0].target_port, Some(1));
        assert_eq!(arena.edges[1].source_port, Some(2));
        assert_eq!(arena.ports[0].node, 0);
        assert_eq!(arena.ports[0].side, super::PortSide::East);
        assert_eq!(arena.ports[0].x, 40.0);
        assert_eq!(arena.ports[0].y, 10.0);
        assert_eq!(arena.ports[1].node, 1);
        assert_eq!(arena.ports[1].side, super::PortSide::West);
        assert_eq!(arena.ports[1].x, 0.0);
        assert_eq!(arena.ports[1].y, 15.0);
        assert_eq!(arena.ports[2].node, 0);
        assert_eq!(arena.ports[2].side, super::PortSide::East);
        assert_eq!(arena.ports[2].y, 20.0);
    }

    #[test]
    fn refresh_implicit_port_positions_keeps_p3_order_after_resize() {
        let elk = graph(
            vec![node("a"), node("b"), node("c")],
            vec![("a", "b"), ("a", "c")],
        );
        let (mut arena, root) = LayeredArena::from_elk_graph(&elk).unwrap();
        materialize_implicit_ports(&mut arena, root).unwrap();
        arena.nodes[0].ports.reverse();
        arena.nodes[0].height = 60.0;

        refresh_implicit_port_positions(&mut arena, &[0]);

        assert_eq!(arena.nodes[0].ports, vec![2, 0]);
        assert_eq!(arena.ports[2].y, 20.0);
        assert_eq!(arena.ports[0].y, 40.0);
    }

    #[test]
    fn importer_carries_node_margins_into_the_arena() {
        let mut source = node("source");
        source.layout_options = Some(json!({
            "org.eclipse.elk.margins": "[top=2,left=3,bottom=5,right=7]"
        }));
        let elk = graph(vec![source], vec![]);
        let (arena, _) = LayeredArena::from_elk_graph(&elk).unwrap();
        assert_eq!(
            [
                arena.nodes[0].margin_top,
                arena.nodes[0].margin_right,
                arena.nodes[0].margin_bottom,
                arena.nodes[0].margin_left
            ],
            [2.0, 7.0, 5.0, 3.0]
        );
    }

    #[test]
    fn port_list_sorter_preserves_v081_west_range_behavior() {
        let mut sink = node("sink");
        sink.layout_options = Some(json!({"elk.portConstraints": "FIXED_SIDE"}));
        sink.ports = (0..4)
            .map(|index| ElkPort {
                id: format!("p{index}"),
                x: 0.0,
                y: index as f64,
                width: 0.0,
                height: 0.0,
                layout_options: json!({"elk.port.side": "WEST"}),
            })
            .collect();
        let (mut arena, root) = LayeredArena::from_elk_graph(&graph(vec![sink], vec![])).unwrap();

        super::sort_port_lists_for_layout(&mut arena, root).unwrap();

        assert_eq!(arena.nodes[0].ports, vec![2, 1, 0, 3]);
    }

    #[test]
    fn bk_conflicts_mark_edges_outside_an_inner_segment_interval() {
        let elk = graph(
            vec![
                node("a"),
                node("b"),
                node("x"),
                node("y"),
                node("w"),
                node("z"),
            ],
            vec![("a", "x"), ("x", "z"), ("b", "y"), ("y", "w")],
        );
        let (mut arena, root) = LayeredArena::from_elk_graph(&elk).unwrap();
        assign_longest_path_layers(&mut arena, root).unwrap();
        let bottom_layer = arena.graphs[root as usize].layers[2];
        arena.layers[bottom_layer as usize].nodes.reverse();
        arena.nodes[2].node_type = super::LayeredNodeType::LongEdge;
        arena.nodes[5].node_type = super::LayeredNodeType::LongEdge;
        let marked = mark_bk_conflicts(&arena, root).unwrap();
        assert!(marked.contains(&3));
    }
    #[test]
    fn original_go_shared_source_port_node_order_uses_first_port_match_and_history() {
        let graph: ElkGraph = serde_json::from_value(json!({"id":"root","children":[
            {"id":"d","width":50,"height":50,"ports":[{"id":"d.p","x":50,"y":25,"width":0,"height":0,"layoutOptions":{"elk.port.side":"EAST"}}]},
            {"id":"b","width":50,"height":50},{"id":"c","width":50,"height":50}],
            "edges":[{"id":"dc","sources":["d.p"],"targets":["c"]},{"id":"db","sources":["d.p"],"targets":["b"]}]})).unwrap();
        let (mut arena, root) = LayeredArena::from_elk_graph(&graph).unwrap();
        materialize_implicit_ports(&mut arena, root).unwrap();
        assign_longest_path_layers(&mut arena, root).unwrap();
        arena.nodes[2].node_type = super::LayeredNodeType::LongEdge;
        let previous = [0];
        let layer = arena.nodes[1].layer.unwrap();
        let mut compare = super::InputModelNodeComparator::new(&arena, &previous, true, layer);
        assert_eq!(compare.compare(1, 2), std::cmp::Ordering::Less);
        assert_eq!(compare.compare(2, 1), std::cmp::Ordering::Greater);
        let mut fresh = super::InputModelNodeComparator::new(&arena, &previous, true, layer);
        assert_eq!(fresh.compare(2, 1), std::cmp::Ordering::Less);
    }

    #[test]
    fn original_go_shared_source_port_input_order_preserves_west_reverse_rule() {
        let graph: ElkGraph = serde_json::from_value(json!({"id":"root","children":[
            {"id":"d","width":50,"height":50,"ports":[{"id":"d.p","x":50,"y":25,"width":0,"height":0,"layoutOptions":{"elk.port.side":"EAST"}}]},
            {"id":"c","width":50,"height":50,"ports":[
                {"id":"c.p1","x":0,"y":15,"width":0,"height":0,"layoutOptions":{"elk.port.side":"WEST"}},
                {"id":"c.p2","x":0,"y":35,"width":0,"height":0,"layoutOptions":{"elk.port.side":"WEST"}}]}],
            "edges":[{"id":"e1","sources":["d.p"],"targets":["c.p1"]},{"id":"e2","sources":["d.p"],"targets":["c.p2"]}]})).unwrap();
        let (mut arena, root) = LayeredArena::from_elk_graph(&graph).unwrap();
        materialize_implicit_ports(&mut arena, root).unwrap();
        assign_longest_path_layers(&mut arena, root).unwrap();
        let previous = [0];
        let targets = BTreeMap::new();
        let orders = BTreeMap::new();
        let mut compare =
            super::InputModelPortComparator::new(&arena, &previous, &targets, &orders, 1);
        assert_eq!(compare.compare(1, 2), std::cmp::Ordering::Greater);
        assert_eq!(compare.compare(2, 1), std::cmp::Ordering::Less);
    }

    #[test]
    fn original_go_fixed_side_feedback_respects_first_constraint_and_reversal_state() {
        for (constraint, reversed) in [
            ("NONE", true),
            ("FIRST", false),
            ("FIRST_SEPARATE", false),
            ("LAST", true),
        ] {
            let graph: ElkGraph = serde_json::from_value(json!({"id":"root","children":[
                {"id":"a","width":50,"height":50,"layoutOptions":{"elk.layered.layering.layerConstraint":constraint}},
                {"id":"b","width":50,"height":50,"layoutOptions":{"elk.portConstraints":"FIXED_POS"},
                 "ports":[{"id":"b.p","x":50,"y":25,"width":0,"height":0,"layoutOptions":{"elk.port.side":"EAST"}}]}],
                "edges":[{"id":"ab","sources":["a"],"targets":["b.p"]}]})).unwrap();
            let (mut arena, root) = LayeredArena::from_elk_graph(&graph).unwrap();
            materialize_implicit_ports(&mut arena, root).unwrap();
            super::reverse_constraint_edges_for_layout(&mut arena, root);
            assert_eq!(arena.edges[0].reversed, reversed, "{constraint}");
            let endpoints = (arena.edges[0].source, arena.edges[0].target);
            super::reverse_constraint_edges_for_layout(&mut arena, root);
            assert_eq!((arena.edges[0].source, arena.edges[0].target), endpoints);
        }
    }

    #[test]
    fn original_go_recursive_barycenter_uses_port_order_and_partial_cycle_state() {
        let graph: ElkGraph = serde_json::from_value(serde_json::json!({
        "id":"root","children":[
            {"id":"a","width":40,"height":30,"ports":[{"id":"a.fixed","x":0,"y":0,"width":0,"height":0,"layoutOptions":{}},{"id":"a.same","x":0,"y":0,"width":0,"height":0,"layoutOptions":{}}]},
            {"id":"b","width":40,"height":30,"ports":[{"id":"b.same","x":0,"y":0,"width":0,"height":0,"layoutOptions":{}},{"id":"b.fixed","x":0,"y":0,"width":0,"height":0,"layoutOptions":{}}]},
            {"id":"c","width":40,"height":30,"ports":[{"id":"c.one","x":0,"y":0,"width":0,"height":0,"layoutOptions":{}},{"id":"c.three","x":0,"y":0,"width":0,"height":0,"layoutOptions":{}}]}
        ],"edges":[
            {"id":"same0","sources":["a.same"],"targets":["b.same"]},
            {"id":"same1","sources":["b.same"],"targets":["a.same"]},
            {"id":"fixed0","sources":["a.fixed"],"targets":["c.one"]},
            {"id":"fixed1","sources":["b.fixed"],"targets":["c.three"]}
        ]}))
        .unwrap();
        let (mut arena, root) = LayeredArena::from_elk_graph(&graph).unwrap();
        arena.layers = vec![
            Layer {
                graph: root,
                nodes: vec![0, 1],
                ..Default::default()
            },
            Layer {
                graph: root,
                nodes: vec![2],
                ..Default::default()
            },
        ];
        arena.nodes[0].layer = Some(0);
        arena.nodes[1].layer = Some(0);
        arena.nodes[2].layer = Some(1);
        let mut ranks = vec![0.0; arena.ports.len()];
        ranks[4] = 1.0;
        ranks[5] = 3.0;
        let mut states = vec![super::SourceBarycenterState::default(); arena.nodes.len()];
        super::calculate_source_barycenter(
            &arena,
            0,
            false,
            &ranks,
            &BTreeMap::new(),
            &mut states,
            &mut None,
            false,
        );
        assert_eq!(
            (
                states[1].degree,
                states[1].summed_weight,
                states[1].barycenter
            ),
            (2, 4.0, Some(2.0))
        );
        assert_eq!(
            (
                states[0].degree,
                states[0].summed_weight,
                states[0].barycenter
            ),
            (3, 5.0, Some(5.0 / 3.0))
        );
        let before = states.clone();
        super::calculate_source_barycenter(
            &arena,
            1,
            false,
            &ranks,
            &BTreeMap::new(),
            &mut states,
            &mut None,
            false,
        );
        for (a, b) in states.iter().zip(before) {
            assert_eq!(
                (a.degree, a.summed_weight, a.barycenter),
                (b.degree, b.summed_weight, b.barycenter)
            );
        }
    }
}

#[cfg(test)]
mod source_label_import_order_contract {
    use super::*;
    use serde_json::json;

    #[test]
    fn original_go_label_proxies_follow_node_port_and_stored_outgoing_order() {
        let mut input = json!({"id":"root","children":[
            {"id":"a","width":40,"height":30,"ports":[
                {"id":"a.second","x":40,"y":10,"width":0,"height":0,"layoutOptions":{}},
                {"id":"a.first","x":40,"y":20,"width":0,"height":0,"layoutOptions":{}}]},
            {"id":"b","width":40,"height":30},
            {"id":"c","width":40,"height":30}],"edges":[]});
        for (id, source, target) in [
            ("bc", "b", "c"),
            ("ab", "a.first", "b"),
            ("ac", "a.second", "c"),
            ("ac2", "a.first", "c"),
            ("loop", "a", "a"),
        ] {
            input["edges"]
                .as_array_mut()
                .unwrap()
                .push(json!({"id":id,"sources":[source],"targets":[target],
                "labels":[{"text":id,"width":20,"height":10}]}));
        }
        let wire: crate::model::ElkGraph = serde_json::from_value(input).unwrap();
        let (mut arena, root) = LayeredArena::from_elk_graph(&wire).unwrap();
        materialize_implicit_ports(&mut arena, root).unwrap();
        arena
            .insert_center_edge_label_dummies(root, &wire.edges, false)
            .unwrap();
        assert_eq!(
            arena
                .edge_label_dummies
                .iter()
                .map(|d| d.edge)
                .collect::<Vec<_>>(),
            vec![2, 1, 3, 0]
        );
        // Every original edge is split once; the self-loop is left for its
        // own processor. New proxy nodes never enter this traversal.
        assert_eq!(arena.nodes.len(), 7);
        assert_eq!(arena.edges[4].source, arena.edges[4].target);
        for dummy in &arena.edge_label_dummies {
            let incoming = arena.node_incoming_edges(dummy.node);
            let outgoing = arena.node_outgoing_edges(dummy.node);
            assert_eq!(incoming, vec![dummy.edge]);
            assert_eq!(outgoing.len(), 1);
            assert_eq!(arena.edges[outgoing[0] as usize].target, dummy.target);
        }
    }

    #[test]
    fn original_go_imports_boundary_edges_before_deeper_edges_without_renaming_ids() {
        let input = json!({"id":"root","layoutOptions":{"elk.hierarchyHandling":"INCLUDE_CHILDREN","elk.layered.considerModelOrder.strategy":"NODES_AND_EDGES"},
            "children":[{"id":"g","width":0,"height":0,"children":[
                {"id":"a","width":40,"height":30},
                {"id":"b","width":40,"height":30,"ports":[
                    {"id":"b.shared","x":0,"y":10,"width":0,"height":0,"layoutOptions":{}}]}]},
                {"id":"outside","width":40,"height":30}],
            "edges":[
                {"id":"inside","sources":["a"],"targets":["b.shared"]},
                {"id":"boundary","sources":["outside"],"targets":["b.shared"]},
                {"id":"unported","sources":["outside"],"targets":["a"]}]});
        let wire: crate::model::ElkGraph = serde_json::from_value(input).unwrap();
        let (mut arena, root) = LayeredArena::from_elk_graph(&wire).unwrap();
        assert_eq!(
            (
                arena.edge_model_order[&0],
                arena.edge_model_order[&1],
                arena.edge_model_order[&2]
            ),
            (2, 0, 1)
        );
        let shared = arena.edges[0].target_port.unwrap();
        assert_eq!(arena.ports[shared as usize].incoming_edges, vec![1, 0]);
        materialize_implicit_ports(&mut arena, root).unwrap();
        let a = arena.edges[0].source;
        let attached = arena.nodes[a as usize]
            .ports
            .iter()
            .map(|p| {
                let port = &arena.ports[*p as usize];
                port.incoming_edges
                    .first()
                    .or_else(|| port.outgoing_edges.first())
                    .copied()
                    .unwrap()
            })
            .collect::<Vec<_>>();
        assert_eq!(attached, vec![2, 0]);
        assert_eq!(wire.edges[0].id, "inside");
    }
}
