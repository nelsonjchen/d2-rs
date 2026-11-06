// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Lossless-enough graph-model reconstruction for graphjson input.
//!
//! The upstream codec first interns every node and edge record (including
//! records reachable only through trees or aggregate vessels), then publishes
//! the active graph atomically.  The shared `Graph` does not yet have all of
//! the pointer-owned aggregate maps, but preserving the complete record
//! inventory here is still valuable: it lets direct-engine adapters exercise
//! signed identities, source order, routes, labels, and topology validation
//! without going through the binary-plugin representation.

mod records;

use super::wire::{
    SerializedEdge, SerializedGraph, SerializedLabel, SerializedNode, SerializedPoint,
    SerializedStyle, SerializedTree,
};
use d2_rs_core::{
    ArrowheadLabel, Direction, Edge, EdgeArrowheadLabels, EdgeArrowheads, EdgeArrows, EdgeLabel,
    EdgeStyle, EdgeTableColumns, Graph, Insets, LabelPosition, Node, Point,
    SerializedAggregateEdgeAbduction, SerializedClusterState, SerializedHierarchyState,
    SerializedHubState, SerializedSequenceState, SerializedTreeState, ShapeKind, Size,
};
use std::collections::BTreeMap;

fn point(value: &Option<SerializedPoint>) -> Option<Point> {
    value.as_ref().map(|value| Point {
        x: value.x,
        y: value.y,
    })
}

fn shape(value: &str) -> ShapeKind {
    match value {
        "Square" => ShapeKind::Square,
        "RealSquare" => ShapeKind::RealSquare,
        "Parallelogram" => ShapeKind::Parallelogram,
        "Document" => ShapeKind::Document,
        "Cylinder" => ShapeKind::Cylinder,
        "Queue" => ShapeKind::Queue,
        "Page" => ShapeKind::Page,
        "Package" => ShapeKind::Package,
        "Step" => ShapeKind::Step,
        "Callout" => ShapeKind::Callout,
        "StoredData" => ShapeKind::StoredData,
        "Person" => ShapeKind::Person,
        "C4Person" => ShapeKind::C4Person,
        "Diamond" => ShapeKind::Diamond,
        "Oval" => ShapeKind::Oval,
        "Circle" => ShapeKind::Circle,
        "Hexagon" => ShapeKind::Hexagon,
        "Cloud" => ShapeKind::Cloud,
        "Table" => ShapeKind::SqlTable,
        "Class" => ShapeKind::Class,
        "Text" => ShapeKind::Text,
        "Code" => ShapeKind::Code,
        "Image" => ShapeKind::Image,
        _ => ShapeKind::Rectangle,
    }
}

fn direction(value: &str) -> Option<Direction> {
    match value.to_ascii_lowercase().as_str() {
        "right" => Some(Direction::Right),
        "left" => Some(Direction::Left),
        "up" => Some(Direction::Up),
        "down" => Some(Direction::Down),
        _ => None,
    }
}

fn scalar(value: &Option<super::wire::SerializedScalar>) -> Option<String> {
    value.as_ref().map(|value| value.value.clone())
}

fn edge_style(style: &SerializedStyle) -> EdgeStyle {
    EdgeStyle {
        language: None, // The pinned layout codec does not serialize render languages.
        opacity: scalar(&style.opacity),
        stroke: scalar(&style.stroke),
        stroke_width: scalar(&style.stroke_width),
        stroke_dash: scalar(&style.stroke_dash),
        border_radius: scalar(&style.border_radius),
        animated: scalar(&style.animated),
        font_color: scalar(&style.font_color),
        font: scalar(&style.font),
        italic: scalar(&style.italic),
        underline: scalar(&style.underline),
        text_transform: scalar(&style.text_transform),
        fill: scalar(&style.fill),
        filled: scalar(&style.filled),
    }
}

// graphjson/codec.go::unmarshalLabel calls label.FromString, which accepts
// only canonical enum strings. Unknown or differently cased names are Unset.
fn serialized_label_position(value: &str) -> LabelPosition {
    let position = LabelPosition::from_d2_name(value);
    if position.d2_name() == value {
        position
    } else {
        LabelPosition::Unset
    }
}

fn label(value: &Option<SerializedLabel>) -> Option<EdgeLabel> {
    value.as_ref().map(|value| EdgeLabel {
        text: value.text.clone(),
        size: Size {
            width: value.width,
            height: value.height,
        },
        position: serialized_label_position(&value.position),
        percentage: 0.5,
    })
}

fn arrowhead_label(value: &Option<SerializedLabel>) -> Option<ArrowheadLabel> {
    value.as_ref().map(|value| ArrowheadLabel {
        text: value.text.clone(),
        size: Size {
            width: value.width,
            height: value.height,
        },
        color: None,
    })
}

fn checked_edge_abductions(
    values: Option<&[super::wire::SerializedEdgeAbduction]>,
    kind: &str,
    aggregate_id: i64,
    node_ids: &BTreeMap<i64, usize>,
    edge_ids: &BTreeMap<i64, usize>,
) -> Result<Vec<SerializedAggregateEdgeAbduction>, String> {
    let mut result = Vec::new();
    for value in values.unwrap_or_default() {
        if !edge_ids.contains_key(&value.edge) {
            return Err(format!(
                "couldn't find edge abduction edge {} for {} {}",
                value.edge, kind, aggregate_id
            ));
        }
        for (label, node_id) in [
            ("OriginallyFrom", value.originally_from_node),
            ("OriginallyTo", value.originally_to_node),
            ("CurrentFrom", value.current_from_node),
            ("CurrentTo", value.current_to_node),
        ] {
            if node_id != 0 && !node_ids.contains_key(&node_id) {
                return Err(format!(
                    "couldn't find edge abduction {label} {node_id} for {kind} {aggregate_id}"
                ));
            }
        }
        result.push(SerializedAggregateEdgeAbduction {
            edge_id: value.edge,
            originally_from_node: value.originally_from_node,
            originally_to_node: value.originally_to_node,
            current_from_node: value.current_from_node,
            current_to_node: value.current_to_node,
        });
    }
    Ok(result)
}

fn node(record: &SerializedNode) -> Node {
    let size = Size {
        width: record.width,
        height: record.height,
    };
    let mut result = Node::new(
        record
            .d2_id
            .clone()
            .unwrap_or_else(|| record.id.to_string()),
        size,
    );
    // Graphjson `width`/`height` are the measured box already supplied to
    // TALA. They are not source-level fixed dimensions; the wire format's
    // explicit fixed-size hints are `desiredWidth`/`desiredHeight` below.
    // Keeping `declared_size` empty matches Go's deserializeNode, which does
    // not manufacture a declaration from the current measured box.
    result.declared_size = None;
    result.shape = shape(&record.shape_type);
    result.label_size = record.label.as_ref().map(|label| Size {
        width: label.width,
        height: label.height,
    });
    result.label_position = record.label.as_ref().map_or(LabelPosition::Unset, |label| {
        serialized_label_position(&label.position)
    });
    result.font_size = record.font_size.and_then(|value| u32::try_from(value).ok());
    result.locked_position = point(&record.fixed_top_left);
    result.fixed_width = record.desired_width.is_some();
    result.fixed_height = record.desired_height.is_some();
    result.force_hierarchy = record.force_hierarchy;
    result.is_3d = record.is3_d;
    result.is_multiple = record.is_multiple;
    result.has_icon = record.icon.is_some();
    result.icon_position = record
        .icon
        .as_ref()
        .map(|icon| LabelPosition::from_d2_name(&icon.position));
    result.content_insets = Insets::uniform(60.0);
    result
}

/// Reconstruct a shared graph from a validated wire graph.
pub(super) fn to_core_graph(serialized: &SerializedGraph) -> Result<Graph, String> {
    Ok(to_core_graph_with_records(serialized)?.0)
}

/// Keep immutable codec metadata in the same dense allocation order as the
/// reconstructed graph. Geometry and topology are subsequently read from the
/// live arena; these records are never used as a replacement layout result.
pub(super) fn to_core_graph_with_records(
    serialized: &SerializedGraph,
) -> Result<(Graph, Vec<SerializedNode>, Vec<SerializedEdge>), String> {
    let decoded = records::Records::decode(serialized)?;
    let records = &decoded.nodes;
    let edges = &decoded.edges;
    let edge_ids = &decoded.edge_ids;

    let mut graph = Graph::default();
    graph.tala_decoded_edge_records = edge_ids
        .iter()
        .map(|(&id, &index)| (id, d2_rs_core::EdgeId(index as u32)))
        .collect();
    let mut dense = BTreeMap::new();
    for record in records {
        let id = graph.add_node(node(record));
        graph
            .tala_desired_dimensions
            .insert(id, [record.desired_width, record.desired_height]);
        graph
            .tala_decoded_label_presence
            .insert(id, record.label.is_some());
        dense.insert(record.id, id);
        graph.tala_id_overrides[id.0 as usize] = Some(record.id);
        graph.set_node_invisible(id, record.is_invisible);
        graph.set_initial_position(id, point(&record.top_left));
        if record.num_columns > 0 || shape(&record.shape_type) == ShapeKind::SqlTable {
            // v0.9.0 graphjson does not serialize table rows when the count is
            // zero. Preserve that explicit zero so routing uses the generic
            // eight-port table shape, while programmatic graphs with no
            // metadata continue using the recovered height fallback.
            graph.set_table_column_count(id, usize::try_from(record.num_columns).ok());
        }
    }

    let mut hierarchy_order = BTreeMap::new();
    for (parent_id, children) in serialized.containers.iter().flatten() {
        let parent = if *parent_id == 0 {
            None
        } else {
            Some(dense[parent_id])
        };
        // MarkDecodedContainer is independent of the number of children.
        // An explicit empty entry still makes this node a container.
        if let Some(parent) = parent {
            graph.set_container_flag(parent, true);
        }
        let mut order = Vec::new();
        for child_id in children.as_deref().unwrap_or_default() {
            let child = *dense
                .get(child_id)
                .ok_or_else(|| format!("container references unknown child {child_id}"))?;
            graph
                .node_mut(child)
                .ok_or_else(|| format!("container child {child_id} is out of range"))?
                .parent = parent;
            order.push(child);
        }
        hierarchy_order.insert(parent, order);
    }
    graph.set_hierarchy_children_order(hierarchy_order);
    // Keep the serialized hierarchy as durable adapter metadata. TALA
    // replaces each temporary Graph's Containers map while combining
    // subgraphs, but its shared Node pointers still retain their original
    // source parent identities for diagnostics and projected geometry.
    for record in records {
        graph.source_parent_tala_by_tala.insert(record.id, None);
        if let Some(position) = point(&record.top_left) {
            graph.source_positions_by_tala.insert(record.id, position);
        }
    }
    for (parent_id, children) in serialized.containers.as_ref().into_iter().flatten() {
        let parent_tala_id = (*parent_id != 0).then_some(*parent_id);
        for child_id in children.as_deref().unwrap_or_default() {
            graph
                .source_parent_tala_by_tala
                .insert(*child_id, parent_tala_id);
        }
    }
    graph.set_node_order_override(
        serialized
            .nodes
            .as_deref()
            .unwrap_or_default()
            .iter()
            .filter_map(|record| dense.get(&record.id).copied())
            .collect(),
    );
    graph.tala_decoded_members = Some((
        serialized
            .nodes
            .as_deref()
            .unwrap_or_default()
            .iter()
            .map(|record| dense[&record.id])
            .collect(),
        serialized
            .edges
            .as_deref()
            .unwrap_or_default()
            .iter()
            .map(|record| d2_rs_core::EdgeId(edge_ids[&record.id] as u32))
            .collect(),
    ));
    graph.root_hierarchy = serialized.is_root_hierarchy;
    if let Some(directions) = &serialized.directions {
        for (id, value) in directions {
            let Some(direction) = direction(value) else {
                continue;
            };
            if *id == 0 {
                graph.direction = direction;
                graph.explicit_direction = Some(direction);
            } else if let Some(node) = dense.get(id).copied().and_then(|id| graph.node_mut(id)) {
                node.direction = Some(direction);
            }
        }
    }

    graph.tala_decoded_nil_owners = decoded.nil_owners.iter().map(|id| dense[id]).collect();
    for &(source, target) in &decoded.nears {
        let source = dense[&source];
        let target = dense[&target];
        let node = graph.node_mut(source).expect("interned near source");
        if node.near.is_none() {
            node.near = Some(target);
        } else {
            graph.near_targets[source.0 as usize].push(target);
        }
    }

    for record in edges {
        let source = *dense.get(&record.from_node).ok_or_else(|| {
            format!(
                "edge {} references unknown source {}",
                record.id, record.from_node
            )
        })?;
        let target = *dense.get(&record.to_node).ok_or_else(|| {
            format!(
                "edge {} references unknown target {}",
                record.id, record.to_node
            )
        })?;
        let edge_id = graph.add_edge(Edge { source, target });
        graph.tala_edge_ids[edge_id.0 as usize] = Some(record.original_id.unwrap_or(record.id));
        graph.tala_edge_invisible[edge_id.0 as usize] = Some(record.is_invisible);
        graph.tala_edge_label_positions.insert(
            edge_id,
            [
                record
                    .label
                    .as_ref()
                    .map(|v| serialized_label_position(&v.position)),
                record
                    .source_arrowhead_label
                    .as_ref()
                    .map(|v| serialized_label_position(&v.position)),
                record
                    .target_arrowhead_label
                    .as_ref()
                    .map(|v| serialized_label_position(&v.position)),
            ],
        );
        graph.set_edge_min_dimensions(
            edge_id,
            Some(record.min_width as f64),
            Some(record.min_height as f64),
        );
        graph.set_edge_route(
            edge_id,
            record
                .points
                .as_deref()
                .unwrap_or_default()
                .iter()
                .filter_map(point)
                .collect(),
        );
        graph.tala_decoded_routes.insert(
            edge_id,
            record
                .points
                .as_deref()
                .unwrap_or_default()
                .iter()
                .map(point)
                .collect(),
        );
        graph.set_edge_arrows(
            edge_id,
            EdgeArrows {
                source: !record.source_arrowhead.is_empty() && record.source_arrowhead != "none",
                target: !record.target_arrowhead.is_empty() && record.target_arrowhead != "none",
            },
        );
        graph.set_edge_arrowheads(
            edge_id,
            EdgeArrowheads {
                // These are effective TALA identities, including empty and
                // none. They are not absent D2 arrowhead records.
                source: Some(record.source_arrowhead.clone()),
                target: Some(record.target_arrowhead.clone()),
            },
        );
        if let Some(mut value) = label(&record.label) {
            value.percentage = record.label_percentage;
            graph.set_edge_label(edge_id, Some(value));
        }
        graph.set_edge_arrowhead_labels(
            edge_id,
            EdgeArrowheadLabels {
                source: arrowhead_label(&record.source_arrowhead_label),
                target: arrowhead_label(&record.target_arrowhead_label),
            },
        );
        graph.set_edge_style(edge_id, edge_style(&record.style));
        graph.set_edge_table_columns(
            edge_id,
            EdgeTableColumns {
                source: record
                    .from_table_column_index
                    .and_then(|value| usize::try_from(value).ok()),
                target: record
                    .to_table_column_index
                    .and_then(|value| usize::try_from(value).ok()),
            },
        );
    }
    graph.source_edge_endpoints = edges
        .iter()
        .map(|record| (record.from_node, record.to_node))
        .collect();
    graph.source_edge_endpoints_by_id = edges
        .iter()
        .map(|record| {
            (
                record.original_id.unwrap_or(record.id),
                (record.from_node, record.to_node),
            )
        })
        .collect();
    // Preserve TALA's active aggregate maps for graphjson replay and direct
    // in-process layout.
    // Vessels have canonical node records, independently of Graph.Nodes.
    // Numerical group metadata refers to those same interned geometries.
    graph.serialized_clusters = serialized
        .clusters
        .as_ref()
        .into_iter()
        .flat_map(|clusters| clusters.iter())
        .map(|(&vessel_tala_id, cluster)| {
            let vessel = &records[decoded.node_ids[&vessel_tala_id]];
            let vessel_position = point(&vessel.top_left);
            let vessel_size = Size {
                width: vessel.width,
                height: vessel.height,
            };
            SerializedClusterState {
                vessel_tala_id,
                member_tala_ids: cluster.nodes.clone().unwrap_or_default(),
                arrangement: cluster.arrangement.clone(),
                desired_arrangement: cluster.desired_arrangement.clone(),
                edge_abductions: decoded.cluster_abductions[&vessel_tala_id].clone(),
                container_tala_id: cluster.container,
                fixed_size: cluster.fixed_size,
                padding: cluster.padding,
                vessel_position,
                vessel_size,
            }
        })
        .collect();
    graph.serialized_sequences = serialized
        .sequences
        .as_ref()
        .into_iter()
        .flat_map(|sequences| sequences.iter())
        .map(|(&vessel_tala_id, sequence)| {
            let vessel = &records[decoded.node_ids[&vessel_tala_id]];
            let vessel_position = point(&vessel.top_left);
            let vessel_size = Size {
                width: vessel.width,
                height: vessel.height,
            };
            SerializedSequenceState {
                vessel_tala_id,
                member_tala_ids: sequence.nodes.clone().unwrap_or_default(),
                edge_abductions: decoded.sequence_abductions[&vessel_tala_id].clone(),
                container_tala_id: sequence.container,
                vessel_position,
                vessel_size,
            }
        })
        .collect();
    graph.serialized_hubs = serialized
        .hubs
        .as_ref()
        .into_iter()
        .flat_map(|hubs| hubs.iter())
        .map(|(&hub_tala_id, spokes)| {
            if !dense.contains_key(&hub_tala_id) {
                return Err(format!("hub {hub_tala_id} is missing from nodes"));
            }
            let spoke_tala_ids = spokes.as_deref().unwrap_or_default().to_vec();
            for spoke in &spoke_tala_ids {
                if !dense.contains_key(spoke) {
                    return Err(format!("hub {hub_tala_id} references unknown node {spoke}"));
                }
            }
            Ok(SerializedHubState {
                hub_tala_id,
                spoke_tala_ids,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    graph.serialized_trees = decoded.trees;
    graph.serialized_hierarchies = serialized
        .hierarchies
        .as_ref()
        .into_iter()
        .flat_map(|hierarchies| hierarchies.iter())
        .map(|hierarchy| {
            let levels = hierarchy.levels.clone().unwrap_or_default();
            for node_id in levels.keys() {
                if !dense.contains_key(node_id) {
                    return Err(format!("hierarchy references unknown node {node_id}"));
                }
            }
            Ok(SerializedHierarchyState { levels })
        })
        .collect::<Result<Vec<_>, _>>()?;
    graph.serialized_directions = serialized
        .directions
        .as_ref()
        .into_iter()
        .flat_map(|directions| directions.iter())
        .filter_map(|(&node_id, value)| direction(value).map(|value| (node_id, value)))
        .collect();
    graph.serialized_cell_size = Some(serialized.cell_size);
    graph.serialized_routing_costs = Some((
        serialized.crossing_cost,
        serialized.turn_cost,
        serialized.non_center_port_cost,
    ));
    if crate::internal::trace_env_enabled("TALA_TRACE_SOURCE_EDGES") {
        eprintln!(
            "SOURCE_EDGES_RECONSTRUCT records={} map={} first={:?}",
            edges.len(),
            graph.source_edge_endpoints_by_id.len(),
            graph
                .source_edge_endpoints
                .iter()
                .take(20)
                .collect::<Vec<_>>()
        );
    }
    Ok((graph, decoded.nodes, decoded.edges))
}
