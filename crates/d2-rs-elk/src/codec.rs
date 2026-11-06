// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Translation of the measured Rust graph into D2 v0.9.0's ELK wire model.
//!
//! The pinned Go adapter constructs this graph before invoking `elk-go`.
//! Keeping the translation independent from the solver makes it possible to
//! compare the two transfer boundaries while the layered algorithm is being
//! migrated.

use crate::model::{ElkEdge, ElkGraph, ElkLabel, ElkNode, ElkPort};
use crate::{ElkOptions, collect_graph_stats, prepare_layered_graph_with_stats};
use d2_rs_core::{
    Direction, EdgeId, Graph, Insets, LabelPosition, LayoutError, LayoutResult, NodeId, Point,
    Rect, ShapeKind,
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

/// Builds the pre-layout ELK graph used by D2 v0.9.0.
///
/// `node_labels` carries the source label text because the shared measured
/// graph intentionally stores only label dimensions. Missing labels are
/// omitted, exactly as the Go adapter omits a label when `HasLabel` is false.
/// The graph is cloned and prepared with the same degree, icon, outside-label,
/// and self-loop measurements used by [`crate::layout`].
#[must_use]
pub fn graph_to_elk_graph(
    graph: &Graph,
    node_labels: &BTreeMap<NodeId, String>,
    options: ElkOptions,
) -> ElkGraph {
    let labels = if node_labels.is_empty() {
        &graph.node_labels
    } else {
        node_labels
    };
    graph_to_elk_graph_with_table_columns(graph, labels, &graph.table_column_names, options)
}

/// Builds an ELK graph while preserving source SQL-table column names.
///
/// The ordinary [`graph_to_elk_graph`] helper uses stable column indexes when
/// only the measured graph is available. The compiler can pass this map to
/// reproduce D2's `srcPortID`/`dstPortID` strings exactly.
#[must_use]
pub fn graph_to_elk_graph_with_table_columns(
    graph: &Graph,
    node_labels: &BTreeMap<NodeId, String>,
    table_columns: &BTreeMap<NodeId, Vec<String>>,
    options: ElkOptions,
) -> ElkGraph {
    let stats = collect_graph_stats(
        graph,
        matches!(graph.direction, Direction::Down | Direction::Up),
    );
    let (prepared, root_options) = prepare_layered_graph_with_stats(graph, options, &stats);
    let direction = direction_name(prepared.direction);
    let mut children = Vec::new();
    let mut emitted = BTreeSet::new();
    let root_children = ordered_children(&prepared, None);
    for id in root_children {
        children.push(build_node(
            &prepared,
            id,
            node_labels,
            table_columns,
            &options,
            &stats,
            direction,
            &mut emitted,
        ));
    }
    // Malformed or programmatically-built graphs can contain an orphaned node
    // whose parent is outside the arena. Preserve it in model order instead of
    // silently dropping it from the transfer boundary.
    for (id, _) in prepared.nodes() {
        if !emitted.contains(&id) {
            children.push(build_node(
                &prepared,
                id,
                node_labels,
                table_columns,
                &options,
                &stats,
                direction,
                &mut emitted,
            ));
        }
    }

    let mut edges = prepared
        .edges()
        .map(|(id, edge)| {
            build_edge(
                &prepared,
                id,
                edge.source,
                edge.target,
                table_columns,
                direction,
            )
        })
        .collect::<Vec<_>>();
    add_dynamic_table_ports(&prepared, &mut children, &edges, direction);
    add_edge_labels(&prepared, &mut edges);

    ElkGraph {
        children_present: false,
        edges_present: false,
        x: None,
        y: None,
        width: None,
        height: None,
        id: String::new(),
        layout_options: root_layout_options(&root_options, direction),
        children,
        edges,
    }
}

/// Applies an ELK result graph to a shared layout result.
///
/// ELK coordinates are relative to the owning container. D2 publishes
/// absolute coordinates for the shared Rust result, so this function walks
/// the returned hierarchy first and then flattens each edge section using its
/// `container` origin. Node dimensions follow the Go adapter's `math.Ceil`
/// rule after removing the source adapter's resolved spacing margins.
/// Existing report and label metadata are retained.
pub fn apply_elk_graph_result(
    graph: &Graph,
    elk: &ElkGraph,
    result: &LayoutResult,
) -> Result<LayoutResult, LayoutError> {
    let mut published = result.clone();
    let node_ids = graph
        .nodes()
        .map(|(id, node)| (node.external_id.clone(), id))
        .collect::<BTreeMap<_, _>>();
    let mut origins = BTreeMap::<String, Point>::new();
    let mut adjustments = BTreeMap::new();
    for node in &elk.children {
        apply_node_result(
            node,
            Point::default(),
            graph,
            &node_ids,
            &mut origins,
            &mut adjustments,
            &mut published,
        )?;
    }

    let mut edge_ids = BTreeMap::<String, EdgeId>::new();
    for (id, _) in graph.edges() {
        let generated = format!("edge-{}", id.0);
        edge_ids.insert(generated, id);
        if let Some(abs_id) = d2_rs_core::edge_abs_id(graph, id) {
            edge_ids.insert(abs_id, id);
        }
        if let Some(tala_id) = graph.tala_edge_ids.get(id.0 as usize).copied().flatten() {
            edge_ids.insert(tala_id.to_string(), id);
        }
    }
    for (position, edge) in elk.edges.iter().enumerate() {
        let edge_id = edge_ids
            .get(&edge.id)
            .copied()
            .or_else(|| graph.edges().nth(position).map(|(id, _)| id));
        let Some(edge_id) = edge_id else {
            continue;
        };
        let container_origin = if edge.container.is_empty() || edge.container == elk.id {
            Point::default()
        } else {
            origins.get(&edge.container).copied().ok_or_else(|| {
                LayoutError::BadRouteState(format!(
                    "ELK edge {} references unknown container {:?}",
                    edge.id, edge.container
                ))
            })?
        };
        let mut route = Vec::new();
        for section in &edge.sections {
            route.push(offset_point(section.start, container_origin));
            route.extend(
                section
                    .bends
                    .iter()
                    .copied()
                    .map(|point| offset_point(point, container_origin)),
            );
            route.push(offset_point(section.end, container_origin));
        }
        published.routes.insert(edge_id, route);
    }
    super::margins::apply(graph, &mut published, &adjustments);
    Ok(published)
}

fn apply_node_result(
    node: &ElkNode,
    parent_origin: Point,
    graph: &Graph,
    node_ids: &BTreeMap<String, NodeId>,
    origins: &mut BTreeMap<String, Point>,
    adjustments: &mut BTreeMap<NodeId, Insets>,
    result: &mut LayoutResult,
) -> Result<(), LayoutError> {
    let Some(id) = node_ids.get(&node.id).copied() else {
        return Err(LayoutError::BadRouteState(format!(
            "ELK result references unknown node {:?}",
            node.id
        )));
    };
    let wire_origin = Point {
        x: parent_origin.x + node.x,
        y: parent_origin.y + node.y,
    };
    let margin = graph
        .node(id)
        .map(|value| value.layout_margins)
        .unwrap_or_else(|| d2_rs_core::Insets::uniform(0.0));
    let margin = if margin == d2_rs_core::Insets::uniform(0.0) {
        node.layout_options
            .as_ref()
            .and_then(Value::as_object)
            .and_then(|options| {
                Some(d2_rs_core::Insets {
                    top: options.get("d2-rs.marginTop")?.as_f64()?,
                    right: options.get("d2-rs.marginRight")?.as_f64()?,
                    bottom: options.get("d2-rs.marginBottom")?.as_f64()?,
                    left: options.get("d2-rs.marginLeft")?.as_f64()?,
                })
            })
            .unwrap_or(margin)
    } else {
        margin
    };
    adjustments.insert(id, margin);
    origins.insert(node.id.clone(), wire_origin);
    result.boxes.insert(
        id,
        Rect {
            origin: wire_origin,
            size: d2_rs_core::Size {
                width: node.width.ceil(),
                height: node.height.ceil(),
            },
        },
    );
    for child in &node.children {
        apply_node_result(
            child,
            wire_origin,
            graph,
            node_ids,
            origins,
            adjustments,
            result,
        )?;
    }
    Ok(())
}

fn offset_point(point: crate::model::ElkPoint, origin: Point) -> Point {
    Point {
        x: origin.x + point.x,
        y: origin.y + point.y,
    }
}

fn direction_name(direction: Direction) -> &'static str {
    match direction {
        Direction::Down => "DOWN",
        Direction::Up => "UP",
        Direction::Right => "RIGHT",
        Direction::Left => "LEFT",
    }
}

fn root_layout_options(options: &ElkOptions, direction: &str) -> Value {
    json!({
        "elk.algorithm": algorithm_name(options),
        "spacing.nodeNodeBetweenLayers": options.node_spacing,
        // d2elklayout's newRootLayoutOptions deliberately leaves padding
        // unset. elk-go supplies its 12px default at the root; containers
        // receive explicit shape-aware padding below.
        "spacing.edgeNodeBetweenLayers": options.edge_node_spacing,
        "elk.spacing.nodeSelfLoop": options.self_loop_spacing,
        "elk.spacing.edgeNode": 40,
        "elk.layered.nodePlacement.bk.fixedAlignment": "BALANCED",
        "elk.layered.thoroughness": 8,
        // Master moved this into ConfigurableOpts (json
        // `spacing.edgeEdgeBetweenLayers`), applied at every level.
        "spacing.edgeEdgeBetweenLayers": options.edge_edge_spacing,
        "elk.direction": direction,
        "elk.hierarchyHandling": "INCLUDE_CHILDREN",
        "elk.layered.considerModelOrder.strategy": "NODES_AND_EDGES",
        "elk.layered.cycleBreaking.strategy": "GREEDY_MODEL_ORDER",
        "elk.nodeSize.constraints": "MINIMUM_SIZE",
        "elk.contentAlignment": "H_CENTER V_CENTER",
    })
}

fn container_layout_options(options: &ElkOptions, _direction: &str) -> Value {
    json!({
        "spacing.nodeNodeBetweenLayers": options.node_spacing,
        "elk.padding": padding_string(options.padding),
        "spacing.edgeNodeBetweenLayers": options.edge_node_spacing,
        "elk.spacing.nodeSelfLoop": options.self_loop_spacing,
        "elk.spacing.edgeNode": 40,
        "elk.layered.nodePlacement.bk.fixedAlignment": "BALANCED",
        "elk.layered.thoroughness": 8,
        "spacing.edgeEdgeBetweenLayers": options.edge_edge_spacing,
        "elk.hierarchyHandling": "INCLUDE_CHILDREN",
        "elk.layered.considerModelOrder.strategy": "NONE",
        "elk.layered.cycleBreaking.strategy": "GREEDY",
        "elk.nodeSize.constraints": "MINIMUM_SIZE",
        "elk.contentAlignment": "H_CENTER V_CENTER",
        "elk.direction": "",
    })
}

fn leaf_layout_options() -> Value {
    json!({"elk.direction":"", "elk.spacing.nodeSelfLoop":0, "elk.layered.edgeRouting.selfLoopDistribution": "EQUALLY"})
}

fn port_layout_options(side: &str) -> Value {
    json!({"elk.direction":"", "elk.spacing.nodeSelfLoop":0, "elk.port.side": side})
}

fn inline_label_layout_options() -> Value {
    json!({"elk.direction":"", "elk.spacing.nodeSelfLoop":0, "elk.edgeLabels.inline": true})
}

#[cfg(test)]
#[path = "elk_options_api_tests.rs"]
mod options_api_tests;

fn algorithm_name(options: &ElkOptions) -> &'static str {
    match options.algorithm {
        crate::ElkAlgorithm::Layered => "layered",
        crate::ElkAlgorithm::Fixed => "fixed",
        crate::ElkAlgorithm::Box => "box",
        crate::ElkAlgorithm::Rectpacking => "rectpacking",
        crate::ElkAlgorithm::Random => "random",
        crate::ElkAlgorithm::Force => "force",
        crate::ElkAlgorithm::Stress => "stress",
        crate::ElkAlgorithm::Radial => "radial",
        crate::ElkAlgorithm::MrTree => "mrtree",
        crate::ElkAlgorithm::SporeOverlap => "sporeOverlap",
        crate::ElkAlgorithm::SporeCompaction => "sporeCompaction",
    }
}

fn padding_string(value: f64) -> String {
    let value = if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        value.to_string()
    };
    format!("[top={value},left={value},bottom={value},right={value}]")
}

fn ordered_children(graph: &Graph, parent: Option<NodeId>) -> Vec<NodeId> {
    let mut result = Vec::new();
    let mut seen = BTreeSet::new();
    if let Some(order) = graph.hierarchy_children_order.get(&parent) {
        for id in order.iter().copied() {
            if graph.node(id).is_some_and(|node| node.parent == parent) && seen.insert(id) {
                result.push(id);
            }
        }
    }
    for (id, node) in graph.nodes() {
        if node.parent == parent && seen.insert(id) {
            result.push(id);
        }
    }
    result
}

fn build_node(
    graph: &Graph,
    id: NodeId,
    node_labels: &BTreeMap<NodeId, String>,
    table_columns: &BTreeMap<NodeId, Vec<String>>,
    options: &ElkOptions,
    stats: &crate::GraphStats,
    direction: &str,
    emitted: &mut BTreeSet<NodeId>,
) -> ElkNode {
    if !emitted.insert(id) {
        // A cyclic parent graph is malformed, but a zero-sized record keeps
        // the JSON diagnostic useful without recursing forever.
        return ElkNode {
            edges: Vec::new(),
            id: graph
                .node(id)
                .map(|node| node.external_id.clone())
                .unwrap_or_else(|| format!("node-{}", id.0)),
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
            children: Vec::new(),
            ports: Vec::new(),
            labels: Vec::new(),
            layout_options: None,
        };
    }
    let Some(node) = graph.node(id) else {
        return ElkNode {
            edges: Vec::new(),
            id: format!("node-{}", id.0),
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
            children: Vec::new(),
            ports: Vec::new(),
            labels: Vec::new(),
            layout_options: None,
        };
    };
    let child_ids = ordered_children(graph, Some(id));
    let is_container = !child_ids.is_empty()
        || graph
            .container_flags
            .get(id.0 as usize)
            .copied()
            .unwrap_or(false);
    // d2elklayout.SpacingOpt inflates the wire node by resolved outside
    // label/icon margins. The Go adapter removes these bands after ELK
    // returns, so keep the uninflated dimensions in Graph and widen only the
    // source-shaped transfer record here.
    let margin = node.layout_margins;
    let wire_width = node.size.width + margin.left + margin.right;
    let wire_height = node.size.height + margin.top + margin.bottom;
    let mut layout_options = if is_container {
        let mut value = container_layout_options(options, direction);
        if options.self_loop_spacing == ElkOptions::default().self_loop_spacing {
            let label = stats
                .max_loop_label_by_owner
                .get(&Some(id))
                .copied()
                .unwrap_or(0.0);
            let spacing = options
                .self_loop_spacing
                .max((label.floor() / 2.0).floor() + 5.0);
            value["elk.spacing.nodeSelfLoop"] = Value::from(spacing);
        }
        let padding = source_container_padding(graph, id, &child_ids, node_labels, options.padding);
        if let Some(object) = value.as_object_mut() {
            object.insert(
                "elk.padding".to_owned(),
                Value::String(padding_string_from_insets(padding)),
            );
            let (minimum_width, minimum_height) = if matches!(direction, "DOWN" | "UP") {
                (wire_height.ceil(), wire_width.ceil())
            } else {
                (wire_width.ceil(), wire_height.ceil())
            };
            object.insert(
                "elk.nodeSize.minimum".to_owned(),
                Value::String(format!("({minimum_width:.0}, {minimum_height:.0})")),
            );
        }
        value
    } else {
        leaf_layout_options()
    };
    if let Some(object) = layout_options.as_object_mut() {
        object.insert("d2-rs.marginTop".to_owned(), Value::from(margin.top));
        object.insert("d2-rs.marginRight".to_owned(), Value::from(margin.right));
        object.insert("d2-rs.marginBottom".to_owned(), Value::from(margin.bottom));
        object.insert("d2-rs.marginLeft".to_owned(), Value::from(margin.left));
        // The v0.9.0 graph compiler carries explicit width/height declarations
        // through the ELK adapter. Keep them as private transfer metadata so
        // compound fitting can grow around children without shrinking a
        // declared axis.
        object.insert("d2-rs.fixedWidth".to_owned(), Value::Bool(node.fixed_width));
        object.insert(
            "d2-rs.fixedHeight".to_owned(),
            Value::Bool(node.fixed_height),
        );
    }
    let mut elk_node = ElkNode {
        edges: Vec::new(),
        id: node.external_id.clone(),
        x: 0.0,
        y: 0.0,
        width: wire_width,
        height: wire_height,
        children: child_ids
            .iter()
            .copied()
            .map(|child| {
                build_node(
                    graph,
                    child,
                    node_labels,
                    table_columns,
                    options,
                    stats,
                    direction,
                    emitted,
                )
            })
            .collect(),
        ports: Vec::new(),
        labels: Vec::new(),
        layout_options: Some(layout_options),
    };
    if node.shape == ShapeKind::SqlTable
        && let Some(count) = graph.table_column_count(id)
    {
        elk_node.ports = table_column_ports(
            node,
            count,
            table_columns.get(&id).map(Vec::as_slice),
            direction,
            wire_height,
        );
        if let Some(layout_options) = elk_node
            .layout_options
            .as_mut()
            .and_then(Value::as_object_mut)
        {
            layout_options.insert(
                "elk.portConstraints".to_owned(),
                Value::String("FIXED_POS".to_owned()),
            );
        }
    }
    // d2graph.Object.HasLabel deliberately excludes text, class, SQL-table,
    // and code shapes: their content is rendered by the structured target
    // rather than as an ELK node label. Keep the same exclusion at the wire
    // boundary even though the compiler retains their measured label text.
    if !matches!(
        node.shape,
        ShapeKind::Text | ShapeKind::Class | ShapeKind::SqlTable | ShapeKind::Code
    ) && let (Some(text), Some(size)) = (node_labels.get(&id), node.label_size)
        && !text.is_empty()
    {
        elk_node.labels.push(ElkLabel {
            id: None,
            text: text.clone(),
            x: 0.0,
            y: 0.0,
            width: size.width,
            height: size.height,
            layout_options: None,
        });
    }
    elk_node
}

/// Reproduces d2elklayout.adjustPadding at the Rust/ELK wire boundary.
///
/// The Go adapter first resolves an implicit container label position, then
/// estimates the container's content box from its largest child and asks the
/// shape for the inner box of that estimate. The resulting shape clearance is
/// merged with ELK's default 50px padding. Doing this before the nested scope
/// is laid out is what moves children below package/diamond title bands.
fn source_container_padding(
    graph: &Graph,
    id: NodeId,
    child_ids: &[NodeId],
    labels: &BTreeMap<NodeId, String>,
    default_padding: f64,
) -> Insets {
    // d2elklayout.parsePadding uses case-sensitive `side=(\d+)` searches,
    // independent of elk-go's more general graph-padding option parser.
    let configured_padding = graph
        .layout_engine_options
        .elk_padding
        .as_deref()
        .map(|text| {
            let side = |name: &str| {
                let marker = format!("{name}=");
                let mut remaining = text;
                while let Some(index) = remaining.find(&marker) {
                    remaining = &remaining[index + marker.len()..];
                    let digits = remaining.bytes().take_while(u8::is_ascii_digit).count();
                    if digits != 0 {
                        return remaining[..digits].parse::<i64>().ok().unwrap_or(0) as f64;
                    }
                }
                0.0
            };
            Insets {
                top: side("top"),
                right: side("right"),
                bottom: side("bottom"),
                left: side("left"),
            }
        });
    let Some(node) = graph.node(id) else {
        return configured_padding.unwrap_or_else(|| Insets::uniform(default_padding));
    };
    let mut padding = configured_padding.unwrap_or_else(|| Insets::uniform(default_padding));
    let has_label = !matches!(
        node.shape,
        ShapeKind::Text | ShapeKind::Class | ShapeKind::SqlTable | ShapeKind::Code
    ) && labels.get(&id).is_some_and(|text| !text.is_empty())
        && node
            .label_size
            .is_some_and(|size| size.width > 0.0 || size.height > 0.0);
    let label_position = effective_container_label_position(node, has_label);
    let mut extra = Insets::uniform(0.0);
    if has_label {
        let label_size = node.label_size.unwrap_or_default();
        let label_width = label_size.width + 10.0;
        let label_height = label_size.height + 10.0;
        match label_position {
            LabelPosition::InsideTopLeft
            | LabelPosition::InsideTopCenter
            | LabelPosition::InsideTopRight => extra.top = label_height,
            LabelPosition::InsideBottomLeft
            | LabelPosition::InsideBottomCenter
            | LabelPosition::InsideBottomRight => extra.bottom = label_height,
            LabelPosition::InsideMiddleLeft => extra.left = label_width,
            LabelPosition::InsideMiddleRight => extra.right = label_width,
            _ => {}
        }
    }
    if node.has_icon {
        // d2target.MAX_ICON_SIZE is 64px and adjustPadding adds two label
        // padding bands around an inside icon.
        let icon_size = 64.0 + 10.0;
        let icon_position = node.icon_position.unwrap_or({
            if !child_ids.is_empty() {
                LabelPosition::InsideTopLeft
            } else {
                LabelPosition::InsideMiddleCenter
            }
        });
        match icon_position {
            LabelPosition::InsideTopLeft
            | LabelPosition::InsideTopCenter
            | LabelPosition::InsideTopRight => extra.top = extra.top.max(icon_size),
            LabelPosition::InsideBottomLeft
            | LabelPosition::InsideBottomCenter
            | LabelPosition::InsideBottomRight => extra.bottom = extra.bottom.max(icon_size),
            LabelPosition::InsideMiddleLeft => extra.left = extra.left.max(icon_size),
            LabelPosition::InsideMiddleRight => extra.right = extra.right.max(icon_size),
            _ => {}
        }
    }

    let max_child_width = child_ids
        .iter()
        .filter_map(|child| graph.node(*child))
        .map(|child| child.size.width + child.layout_margins.left + child.layout_margins.right)
        .fold(0.0, f64::max);
    let max_child_height = child_ids
        .iter()
        .filter_map(|child| graph.node(*child))
        .map(|child| child.size.height + child.layout_margins.top + child.layout_margins.bottom)
        .fold(0.0, f64::max);
    let estimated_width = node.size.width
        + node.layout_margins.left
        + node.layout_margins.right
        + max_child_width
        + extra.left
        + extra.right;
    let estimated_height = node.size.height
        + node.layout_margins.top
        + node.layout_margins.bottom
        + max_child_height
        + extra.top
        + extra.bottom;
    let shape_insets = source_shape_inner_insets(node.shape, estimated_width, estimated_height);
    padding.top = padding.top.max(shape_insets.top.ceil() + extra.top);
    padding.right = padding.right.max(shape_insets.right.ceil() + extra.right);
    padding.bottom = padding
        .bottom
        .max(shape_insets.bottom.ceil() + extra.bottom);
    padding.left = padding.left.max(shape_insets.left.ceil() + extra.left);
    padding
}

/// d2elklayout.positionLabelsIcons resolves these fields before ELK runs.
/// Preserve that source adapter state when publishing the layered result,
/// including leaves and intrinsic shapes whose labels are not ELK labels.
pub(crate) fn source_node_positions(
    graph: &Graph,
    id: NodeId,
) -> (LabelPosition, Option<LabelPosition>) {
    let node = graph.node(id).expect("source node position owner");
    let has_children = graph.nodes().any(|(_, child)| child.parent == Some(id));
    let intrinsic = matches!(
        node.shape,
        ShapeKind::Text | ShapeKind::Class | ShapeKind::SqlTable | ShapeKind::Code
    );
    let has_label = !intrinsic
        && graph
            .node_labels
            .get(&id)
            .is_some_and(|text| !text.is_empty());
    let mut position = node.label_position;
    let mut icon = node.icon_position;
    if node.has_icon && icon.is_none() {
        if has_children {
            icon = Some(LabelPosition::InsideTopLeft);
            if position == LabelPosition::Unset {
                return (LabelPosition::InsideTopRight, icon);
            }
        } else {
            icon = Some(if intrinsic {
                LabelPosition::OutsideTopLeft
            } else {
                LabelPosition::InsideMiddleCenter
            });
        }
    }
    if has_label && position == LabelPosition::Unset {
        position = if has_children {
            LabelPosition::InsideTopCenter
        } else if matches!(node.shape, ShapeKind::Image | ShapeKind::Person) {
            LabelPosition::OutsideBottomCenter
        } else if node.has_icon {
            LabelPosition::InsideTopCenter
        } else {
            LabelPosition::InsideMiddleCenter
        };
        if node
            .label_size
            .is_some_and(|size| size.width > node.size.width || size.height > node.size.height)
        {
            position = if has_children {
                LabelPosition::OutsideTopCenter
            } else {
                LabelPosition::OutsideBottomCenter
            };
        }
    }
    (position, icon)
}

fn effective_container_label_position(node: &d2_rs_core::Node, has_label: bool) -> LabelPosition {
    if !has_label {
        return node.label_position;
    }
    if node.label_position != LabelPosition::Unset {
        return node.label_position;
    }
    if node.has_icon
        && node.icon_position.is_none()
        && node
            .label_size
            .is_some_and(|size| size.width <= node.size.width && size.height <= node.size.height)
    {
        return LabelPosition::InsideTopRight;
    }
    if node
        .label_size
        .is_some_and(|size| size.width > node.size.width || size.height > node.size.height)
    {
        return LabelPosition::OutsideTopCenter;
    }
    LabelPosition::InsideTopCenter
}

/// The inner-box offsets used by the v0.9.0 shape package. Shapes whose
/// content box is rectangular have zero offsets; shaped containers retain the
/// geometry that affects label-aware ELK padding.
fn source_shape_inner_insets(shape: ShapeKind, width: f64, height: f64) -> Insets {
    match shape {
        ShapeKind::Diamond => Insets {
            top: height / 4.0,
            right: width / 4.0,
            bottom: height / 4.0,
            left: width / 4.0,
        },
        ShapeKind::Hexagon => Insets {
            top: height / 6.0,
            right: width / 6.0,
            bottom: height / 6.0,
            left: width / 6.0,
        },
        ShapeKind::Package => Insets {
            top: (height * 0.2).min(55.0),
            right: 0.0,
            bottom: 0.0,
            left: 0.0,
        },
        ShapeKind::Parallelogram => Insets {
            top: 0.0,
            right: 26.0,
            bottom: 0.0,
            left: 26.0,
        },
        ShapeKind::StoredData | ShapeKind::Step => Insets {
            top: 0.0,
            right: if shape == ShapeKind::StoredData {
                15.0
            } else {
                35.0
            },
            bottom: 0.0,
            left: if shape == ShapeKind::StoredData {
                15.0
            } else {
                35.0
            },
        },
        ShapeKind::Queue => {
            let arc = 24.0_f64.min(width / 2.0);
            Insets {
                top: 0.0,
                // Go's inner box loses three arcs in total, with one arc
                // already accounted for by its shifted left edge.
                right: 2.0 * arc,
                bottom: 0.0,
                left: arc,
            }
        }
        ShapeKind::Cylinder => {
            let arc = 24.0_f64.min(height / 2.0);
            Insets {
                top: 2.0 * arc,
                right: 0.0,
                bottom: arc,
                left: 0.0,
            }
        }
        ShapeKind::Callout => Insets {
            top: 0.0,
            right: 0.0,
            bottom: 45.0_f64.min(height / 2.0),
            left: 0.0,
        },
        ShapeKind::Document => Insets {
            top: 0.0,
            right: 0.0,
            bottom: height * (1.0 - 14.0 / 18.925),
            left: 0.0,
        },
        ShapeKind::Page => Insets {
            top: 0.0,
            right: if height < 3.0 * 20.348 { 20.8164 } else { 0.0 },
            bottom: 0.0,
            left: 0.0,
        },
        ShapeKind::Person => {
            let shoulder = 20.2 / 68.3 * width;
            Insets {
                top: 0.0,
                right: shoulder,
                bottom: 0.0,
                left: shoulder,
            }
        }
        ShapeKind::C4Person => {
            let head_radius = width * 0.22;
            let body_top = head_radius + head_radius * 0.8;
            Insets {
                top: body_top + height * 0.03,
                right: width * 0.05,
                bottom: height * 0.03,
                left: width * 0.05,
            }
        }
        ShapeKind::Cloud => {
            let aspect = width / height.max(1.0);
            let (x, y, w, h) = if aspect > (1.0 + 0.819 / 0.548) / 2.0 {
                (0.085, 0.409, 0.819, 0.548)
            } else if aspect < (1.0 + 0.549 / 0.820) / 2.0 {
                (0.228, 0.179, 0.549, 0.820)
            } else {
                (0.167, 0.335, 0.663, 0.663)
            };
            Insets {
                top: y * height,
                right: (1.0 - x - w) * width,
                bottom: (1.0 - y - h) * height,
                left: x * width,
            }
        }
        ShapeKind::Circle => {
            // shapeCircle.GetInsidePlacement uses the width for both axes,
            // including the non-square estimate passed by adjustPadding.
            let radius = width / 2.0;
            let half_length = radius * std::f64::consts::SQRT_2 / 2.0;
            Insets::uniform((radius - half_length).ceil())
        }
        ShapeKind::Oval => {
            // The ellipse inner placement uses the diagonal radius. This is
            // the same ceil operation as shapeOval.GetInnerBox.
            let theta = ((height / 2.0).atan2(width / 2.0) as f32) as f64;
            let rx = width / 2.0;
            let ry = height / 2.0;
            let sin = theta.sin();
            let cos = theta.cos();
            let radius = rx * ry / (rx * sin).hypot(ry * cos).max(f64::EPSILON);
            let left = (rx - cos * radius).ceil();
            let top = (ry - sin * radius).ceil();
            Insets {
                top,
                right: left,
                bottom: top,
                left,
            }
        }
        _ => Insets::uniform(0.0),
    }
}

fn padding_string_from_insets(value: Insets) -> String {
    format!(
        "[top={},left={},bottom={},right={}]",
        number_string(value.top),
        number_string(value.left),
        number_string(value.bottom),
        number_string(value.right)
    )
}

fn number_string(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        value.to_string()
    }
}

fn table_column_ports(
    node: &d2_rs_core::Node,
    count: usize,
    names: Option<&[String]>,
    direction: &str,
    wire_height: f64,
) -> Vec<ElkPort> {
    let row_height = wire_height / (count as f64 + 1.0);
    let (source_side, target_side) = if direction == "LEFT" {
        ("WEST", "EAST")
    } else {
        ("EAST", "WEST")
    };
    (0..count)
        .flat_map(|index| {
            let y = (index as f64 + 1.5) * row_height;
            [
                ElkPort {
                    id: format!(
                        "{}.{}.src",
                        node.external_id,
                        names
                            .and_then(|values| values.get(index))
                            .map_or_else(|| index.to_string(), Clone::clone)
                    ),
                    x: 0.0,
                    y,
                    width: 0.0,
                    height: 0.0,
                    layout_options: port_layout_options(source_side),
                },
                ElkPort {
                    id: format!(
                        "{}.{}.dst",
                        node.external_id,
                        names
                            .and_then(|values| values.get(index))
                            .map_or_else(|| index.to_string(), Clone::clone)
                    ),
                    x: 0.0,
                    y,
                    width: 0.0,
                    height: 0.0,
                    layout_options: port_layout_options(target_side),
                },
            ]
        })
        .collect()
}

fn build_edge(
    graph: &Graph,
    id: EdgeId,
    source: NodeId,
    target: NodeId,
    table_columns: &BTreeMap<NodeId, Vec<String>>,
    direction: &str,
) -> ElkEdge {
    let columns = graph.edge_table_columns(id);
    ElkEdge {
        junction_points: Vec::new(),
        id: d2_rs_core::edge_abs_id(graph, id).unwrap_or_else(|| format!("edge-{}", id.0)),
        sources: vec![endpoint_id(
            graph,
            source,
            columns.source,
            true,
            id,
            table_columns,
            direction,
        )],
        targets: vec![endpoint_id(
            graph,
            target,
            columns.target,
            false,
            id,
            table_columns,
            direction,
        )],
        sections: Vec::new(),
        labels: Vec::new(),
        container: String::new(),
        layout_options: None,
    }
}

fn endpoint_id(
    graph: &Graph,
    node_id: NodeId,
    column: Option<usize>,
    source: bool,
    edge_id: EdgeId,
    table_columns: &BTreeMap<NodeId, Vec<String>>,
    direction: &str,
) -> String {
    let Some(node) = graph.node(node_id) else {
        return format!("node-{}", node_id.0);
    };
    if node.shape != ShapeKind::SqlTable {
        return node.external_id.clone();
    }
    let side = if source { "src" } else { "dst" };
    if let Some(column) = column {
        let name = table_columns
            .get(&node_id)
            .and_then(|values| values.get(column))
            .cloned()
            .unwrap_or_else(|| column.to_string());
        format!("{}.{}.{}", node.external_id, name, side)
    } else {
        // Root table ports are created below when a table edge does not name a
        // column. Their side follows ELK's source/target convention.
        let _ = direction;
        format!("{}.__root__.{}.{}", node.external_id, side, edge_id.0)
    }
}

fn add_dynamic_table_ports(
    graph: &Graph,
    nodes: &mut [ElkNode],
    edges: &[ElkEdge],
    direction: &str,
) {
    // d2elklayout groups ports by (table, side), not only by table. In
    // particular, source and target ports each receive an independent evenly
    // spaced run across the table width. Keep endpoint order within each group
    // as the Go adapter does while retaining the original interleaved port
    // order on the node.
    let mut groups = BTreeMap::<(String, String), Vec<String>>::new();
    for (position, edge) in edges.iter().enumerate() {
        let Some(original) = graph.edges.get(position) else {
            continue;
        };
        let columns = graph.edge_table_columns(EdgeId(position as u32));
        for (endpoint, id, column, source) in [
            (edge.sources.first(), original.source, columns.source, true),
            (edge.targets.first(), original.target, columns.target, false),
        ] {
            let Some(endpoint) = endpoint else {
                continue;
            };
            let Some(object) = graph.node(id) else {
                continue;
            };
            if object.shape != ShapeKind::SqlTable || column.is_some() {
                continue;
            }
            let node_id = object.external_id.as_str();
            let side = if matches!(direction, "UP") {
                if source { "NORTH" } else { "SOUTH" }
            } else if source {
                "SOUTH"
            } else {
                "NORTH"
            };
            let Some(node) = find_node_mut(nodes, node_id) else {
                continue;
            };
            node.ports.push(ElkPort {
                id: endpoint.clone(),
                x: 0.0,
                y: 0.0,
                width: 0.0,
                height: 0.0,
                layout_options: port_layout_options(side),
            });
            groups
                .entry((node_id.to_owned(), side.to_owned()))
                .or_default()
                .push(endpoint.clone());
        }
    }
    for ((node_id, _side), endpoints) in groups {
        let Some(node) = find_node_mut(nodes, &node_id) else {
            continue;
        };
        let spacing = node.width / (endpoints.len() + 1) as f64;
        for (index, endpoint) in endpoints.iter().enumerate() {
            if let Some(port) = node.ports.iter_mut().find(|port| port.id == *endpoint) {
                // Match the source expression's operation order exactly:
                // spacing is divided once, then multiplied by each ordinal.
                port.x = (index + 1) as f64 * spacing;
            }
        }
    }
}

fn find_node_mut<'a>(nodes: &'a mut [ElkNode], id: &str) -> Option<&'a mut ElkNode> {
    for node in nodes {
        if node.id == id {
            return Some(node);
        }
        if let Some(found) = find_node_mut(&mut node.children, id) {
            return Some(found);
        }
    }
    None
}

fn add_edge_labels(graph: &Graph, edges: &mut [ElkEdge]) {
    for (id, _edge) in graph.edges() {
        if let Some(label) = graph.edge_label(id)
            && let Some(elk_edge) = edges.get_mut(id.0 as usize)
        {
            elk_edge.labels.push(ElkLabel {
                id: None,
                text: label.text.clone(),
                x: 0.0,
                y: 0.0,
                width: label.size.width,
                height: label.size.height,
                layout_options: Some(inline_label_layout_options()),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{apply_elk_graph_result, graph_to_elk_graph};
    use crate::ElkOptions;
    use crate::model::{ElkEdge, ElkEdgeSection, ElkGraph, ElkNode, ElkPoint};
    use d2_rs_core::{
        Edge, EdgeLabel, EdgeTableColumns, Graph, Insets, LayoutOptions, Node, Point, ShapeKind,
        Size,
    };
    use serde_json::json;
    use std::collections::BTreeMap;

    #[test]
    fn queue_inner_box_has_one_left_arc_and_two_right_arcs() {
        for (width, left, right) in [(1.0, 0.5, 1.0), (48.0, 24.0, 48.0), (200.0, 24.0, 48.0)] {
            let insets = super::source_shape_inner_insets(ShapeKind::Queue, width, 100.0);
            assert_eq!(
                insets,
                Insets {
                    top: 0.0,
                    left,
                    bottom: 0.0,
                    right
                }
            );
        }
    }

    #[test]
    fn codec_matches_go_root_option_names_and_hierarchy() {
        let mut graph = Graph::default();
        let parent = graph.add_node(Node::new(
            "container",
            Size {
                width: 180.0,
                height: 120.0,
            },
        ));
        graph.node_mut(parent).unwrap().fixed_width = true;
        graph.node_mut(parent).unwrap().fixed_height = true;
        let mut child = Node::new(
            "container.child",
            Size {
                width: 40.0,
                height: 30.0,
            },
        );
        child.parent = Some(parent);
        let child = graph.add_node(child);
        let edge = graph.add_edge(Edge {
            source: child,
            target: child,
        });
        graph.set_edge_label(
            edge,
            Some(EdgeLabel {
                text: "loop".into(),
                size: Size {
                    width: 30.0,
                    height: 12.0,
                },
                position: d2_rs_core::LabelPosition::Unset,
                percentage: 0.5,
            }),
        );
        graph.set_node_label(child, "child");
        graph.node_mut(child).unwrap().label_size = Some(Size {
            width: 40.0,
            height: 16.0,
        });

        let elk = graph_to_elk_graph(&graph, &BTreeMap::new(), ElkOptions::default());
        assert_eq!(elk.children.len(), 1);
        assert_eq!(elk.children[0].children[0].id, "container.child");
        assert_eq!(elk.layout_options["elk.direction"], "DOWN");
        assert_eq!(
            elk.layout_options["elk.hierarchyHandling"],
            "INCLUDE_CHILDREN"
        );
        assert_eq!(
            elk.children[0].layout_options.as_ref().unwrap()["elk.direction"],
            ""
        );
        assert_eq!(
            elk.children[0].layout_options.as_ref().unwrap()["spacing.nodeNodeBetweenLayers"],
            70.0
        );
        assert_eq!(
            elk.children[0].layout_options.as_ref().unwrap()["d2-rs.fixedWidth"],
            true
        );
        assert_eq!(
            elk.children[0].layout_options.as_ref().unwrap()["d2-rs.fixedHeight"],
            true
        );
        assert_eq!(elk.edges[0].labels[0].text, "loop");
        assert_eq!(elk.children[0].children[0].labels[0].text, "child");
    }

    #[test]
    fn codec_applies_shape_aware_container_label_padding() {
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
        let package = graph.add_node(package);
        graph.set_node_label(package, "aa");
        let mut diamond = Node::new(
            "aa.bb",
            Size {
                width: 66.0,
                height: 92.0,
            },
        );
        diamond.parent = Some(package);
        diamond.shape = ShapeKind::Diamond;
        let diamond = graph.add_node(diamond);
        graph.set_node_label(diamond, "bb");
        graph.node_mut(diamond).unwrap().label_size = Some(Size {
            width: 18.0,
            height: 21.0,
        });

        let elk = graph_to_elk_graph(&graph, &BTreeMap::new(), ElkOptions::default());
        assert_eq!(
            elk.children[0].layout_options.as_ref().unwrap()["elk.padding"],
            "[top=92,left=50,bottom=50,right=50]"
        );
        assert_eq!(elk.children[0].children[0].id, "aa.bb");
    }

    #[test]
    fn codec_emits_fixed_and_dynamic_table_ports() {
        let mut graph = Graph::default();
        let mut table = Node::new(
            "users",
            Size {
                width: 100.0,
                height: 80.0,
            },
        );
        table.shape = ShapeKind::SqlTable;
        let table = graph.add_node(table);
        graph.set_table_column_count(table, Some(2));
        let other = graph.add_node(Node::new(
            "service",
            Size {
                width: 80.0,
                height: 40.0,
            },
        ));
        let fixed = graph.add_edge(Edge {
            source: table,
            target: other,
        });
        graph.set_edge_table_columns(
            fixed,
            EdgeTableColumns {
                source: Some(1),
                target: None,
            },
        );
        graph.add_edge(Edge {
            source: table,
            target: other,
        });
        let elk = graph_to_elk_graph(&graph, &BTreeMap::new(), ElkOptions::default());
        let table_node = &elk.children[0];
        assert!(table_node.ports.iter().any(|port| port.id == "users.1.src"));
        assert!(
            table_node
                .ports
                .iter()
                .any(|port| port.id == "users.__root__.src.1")
        );
        assert_eq!(
            table_node
                .ports
                .iter()
                .find(|port| port.id == "users.__root__.src.1")
                .unwrap()
                .x,
            50.0
        );
        assert_eq!(elk.edges[0].sources[0], "users.1.src");
        assert_eq!(elk.edges[1].sources[0], "users.__root__.src.1");
    }

    #[test]
    fn dynamic_table_ports_are_spaced_independently_by_side() {
        let mut graph = Graph::default();
        let mut table = Node::new(
            "users",
            Size {
                width: 100.0,
                height: 80.0,
            },
        );
        table.shape = ShapeKind::SqlTable;
        let table = graph.add_node(table);
        graph.set_table_column_count(table, Some(2));
        let service = graph.add_node(Node::new(
            "service",
            Size {
                width: 80.0,
                height: 40.0,
            },
        ));
        for (source, target) in [
            (table, service),
            (service, table),
            (table, service),
            (service, table),
        ] {
            graph.add_edge(Edge { source, target });
        }

        let elk = graph_to_elk_graph(&graph, &BTreeMap::new(), ElkOptions::default());
        let table_node = &elk.children[0];
        let x = |id: &str| {
            table_node
                .ports
                .iter()
                .find(|port| port.id == id)
                .unwrap()
                .x
        };
        let one_third = 100.0 / 3.0;
        assert_eq!(x("users.__root__.src.0"), one_third);
        assert_eq!(x("users.__root__.src.2"), 2.0 * one_third);
        assert_eq!(x("users.__root__.dst.1"), one_third);
        assert_eq!(x("users.__root__.dst.3"), 2.0 * one_third);
    }

    #[test]
    fn codec_omits_structured_shape_labels_like_has_label() {
        let mut graph = Graph::default();
        for (index, shape) in [
            ShapeKind::Text,
            ShapeKind::Class,
            ShapeKind::SqlTable,
            ShapeKind::Code,
        ]
        .into_iter()
        .enumerate()
        {
            let id = graph.add_node(Node::new(
                format!("node-{index}"),
                Size {
                    width: 80.0,
                    height: 40.0,
                },
            ));
            graph.node_mut(id).unwrap().shape = shape;
            graph.node_mut(id).unwrap().label_size = Some(Size {
                width: 24.0,
                height: 16.0,
            });
            graph.set_node_label(id, "structured");
        }

        let elk = graph_to_elk_graph(&graph, &BTreeMap::new(), ElkOptions::default());
        assert!(elk.children.iter().all(|node| node.labels.is_empty()));
    }

    #[test]
    fn codec_round_trips_spacing_margins_like_go_postprocess() {
        let mut graph = Graph::default();
        let mut parent_node = Node::new(
            "parent",
            Size {
                width: 100.0,
                height: 80.0,
            },
        );
        parent_node.layout_margins = Insets {
            top: 6.0,
            right: 4.0,
            bottom: 2.0,
            left: 10.0,
        };
        let parent = graph.add_node(parent_node);
        let mut child_node = Node::new(
            "parent.child",
            Size {
                width: 20.0,
                height: 12.0,
            },
        );
        child_node.parent = Some(parent);
        let child = graph.add_node(child_node);

        let elk = graph_to_elk_graph(&graph, &BTreeMap::new(), ElkOptions::default());
        assert_eq!(elk.children[0].width, 114.0);
        assert_eq!(elk.children[0].height, 88.0);

        let result = d2_rs_dagre::layout(&graph, &LayoutOptions::default()).unwrap();
        let returned = ElkGraph {
            children_present: false,
            edges_present: false,
            x: None,
            y: None,
            width: None,
            height: None,
            id: String::new(),
            layout_options: json!({"elk.direction": "DOWN"}),
            children: vec![ElkNode {
                edges: Vec::new(),
                id: "parent".into(),
                x: 20.0,
                y: 30.0,
                width: 114.0,
                height: 88.0,
                children: vec![ElkNode {
                    edges: Vec::new(),
                    id: "parent.child".into(),
                    x: 5.0,
                    y: 7.0,
                    width: 20.0,
                    height: 12.0,
                    children: Vec::new(),
                    ports: Vec::new(),
                    labels: Vec::new(),
                    layout_options: None,
                }],
                ports: Vec::new(),
                labels: Vec::new(),
                layout_options: None,
            }],
            edges: Vec::new(),
        };
        let published = apply_elk_graph_result(&graph, &returned, &result).unwrap();
        assert_eq!(published.boxes[&parent].origin, Point { x: 30.0, y: 36.0 });
        assert_eq!(
            published.boxes[&parent].size,
            Size {
                width: 100.0,
                height: 80.0
            }
        );
        // Verbatim Go margin-block oracle: both positive leading halves and
        // negative trailing halves move strict descendants.
        assert_eq!(published.boxes[&child].origin, Point { x: 28.0, y: 39.0 });
    }

    #[test]
    fn codec_applies_relative_nodes_and_sections_to_absolute_result() {
        let mut graph = Graph::default();
        let parent = graph.add_node(Node::new(
            "parent",
            Size {
                width: 100.0,
                height: 80.0,
            },
        ));
        let mut child_node = Node::new(
            "child",
            Size {
                width: 30.0,
                height: 20.0,
            },
        );
        child_node.parent = Some(parent);
        let child = graph.add_node(child_node);
        let other = graph.add_node(Node::new(
            "other",
            Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        graph.add_edge(Edge {
            source: child,
            target: other,
        });
        let base = d2_rs_dagre::layout(&graph, &LayoutOptions::default()).unwrap();
        let elk = ElkGraph {
            children_present: false,
            edges_present: false,
            x: None,
            y: None,
            width: None,
            height: None,
            id: String::new(),
            layout_options: json!({"elk.direction": "DOWN"}),
            children: vec![
                ElkNode {
                    edges: Vec::new(),
                    id: "parent".into(),
                    x: 10.0,
                    y: 20.0,
                    width: 110.2,
                    height: 90.8,
                    children: vec![ElkNode {
                        edges: Vec::new(),
                        id: "child".into(),
                        x: 3.0,
                        y: 4.0,
                        width: 30.2,
                        height: 20.8,
                        children: Vec::new(),
                        ports: Vec::new(),
                        labels: Vec::new(),
                        layout_options: None,
                    }],
                    ports: Vec::new(),
                    labels: Vec::new(),
                    layout_options: None,
                },
                ElkNode {
                    edges: Vec::new(),
                    id: "other".into(),
                    x: 100.0,
                    y: 10.0,
                    width: 40.2,
                    height: 30.2,
                    children: Vec::new(),
                    ports: Vec::new(),
                    labels: Vec::new(),
                    layout_options: None,
                },
            ],
            edges: vec![ElkEdge {
                junction_points: Vec::new(),
                id: "edge-0".into(),
                sources: vec!["child".into()],
                targets: vec!["other".into()],
                sections: vec![ElkEdgeSection {
                    id: None,
                    start: ElkPoint { x: 1.0, y: 2.0 },
                    end: ElkPoint { x: 7.0, y: 8.0 },
                    bends: vec![ElkPoint { x: 4.0, y: 5.0 }],
                    incoming_shape: None,
                    outgoing_shape: None,
                }],
                labels: Vec::new(),
                container: "parent".into(),
                layout_options: None,
            }],
        };
        let result = apply_elk_graph_result(&graph, &elk, &base).unwrap();
        assert_eq!(result.boxes[&child].origin.x, 13.0);
        assert_eq!(result.boxes[&child].origin.y, 24.0);
        assert_eq!(result.boxes[&child].size.width, 31.0);
        assert_eq!(
            result.routes[&d2_rs_core::EdgeId(0)],
            vec![
                d2_rs_core::Point { x: 11.0, y: 22.0 },
                d2_rs_core::Point { x: 14.0, y: 25.0 },
                d2_rs_core::Point { x: 17.0, y: 28.0 },
            ]
        );
    }
}
