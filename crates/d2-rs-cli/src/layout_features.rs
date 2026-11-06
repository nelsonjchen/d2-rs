// SPDX-License-Identifier: MPL-2.0
//! internal/d2layoutfeatures/features.go at the manifest pin.
use d2_rs_core::{Graph, NodeId};
use std::collections::BTreeSet;

pub fn check(name: &str, graph: &Graph) -> Result<(), String> {
    let name = name.to_ascii_lowercase();
    let (near_object, container_dimensions, top_left, descendant_edges) = match name.as_str() {
        "dagre" => (false, false, false, false),
        "elk" => (false, true, false, true),
        "tala" => (true, true, true, true),
        _ => {
            return Err(format!(
                "unknown built-in layout engine {}",
                d2_rs_core::quote_go_bytes(name.as_bytes())
            ));
        }
    };
    let containers = graph
        .nodes()
        .filter_map(|(_, node)| node.parent)
        .collect::<BTreeSet<_>>();
    for (id, node) in graph.nodes() {
        if !top_left && (node.constrained_x.is_some() || node.constrained_y.is_some()) {
            return Err(format!(
                "Object {:?} has attribute \"top\" and/or \"left\" set, but layout engine \"{name}\" does not support locked positions. See https://d2lang.com/tour/layouts/#layout-specific-functionality for more.",
                node.external_id
            ));
        }
        if !container_dimensions
            && (node.fixed_width || node.fixed_height)
            && containers.contains(&id)
            && node.grid_rows.is_none()
            && node.grid_columns.is_none()
        {
            return Err(format!(
                "Object {:?} has attribute \"width\" and/or \"height\" set, but layout engine \"{name}\" does not support dimensions set on containers. See https://d2lang.com/tour/layouts/#layout-specific-functionality for more.",
                node.external_id
            ));
        }
        if !near_object && node.near.is_some() {
            return Err(format!(
                "Object {:?} has \"near\" set to another object, but layout engine \"{name}\" only supports constant values for \"near\". See https://d2lang.com/tour/layouts/#layout-specific-functionality for more.",
                node.external_id
            ));
        }
    }
    if descendant_edges {
        return Ok(());
    }
    // FeatureSupportCheck without DESCENDANT_EDGES. Object.OuterSequenceDiagram
    // starts at the parent, so a sequence diagram's own edges are checked.
    let outer_sequence = |id: NodeId| {
        let mut current = graph.node(id).and_then(|node| node.parent);
        while let Some(parent) = current {
            if graph.is_sequence_diagram(parent) {
                return true;
            }
            current = graph.node(parent).and_then(|node| node.parent);
        }
        graph.root_sequence_diagram
    };
    let is_descendant = |mut id: NodeId, ancestor: NodeId| {
        while let Some(parent) = graph.node(id).and_then(|node| node.parent) {
            if parent == ancestor {
                return true;
            }
            id = parent;
        }
        false
    };
    for (edge_id, edge) in graph.edges() {
        if outer_sequence(edge.source) || outer_sequence(edge.target) {
            continue;
        }
        if !containers.contains(&edge.source) && !containers.contains(&edge.target) {
            continue;
        }
        let abs_id = d2_rs_core::edge_abs_id(graph, edge_id).unwrap_or_default();
        if edge.source == edge.target {
            return Err(format!(
                "Connection \"{abs_id}\" is a self loop on a container, but layout engine \"{name}\" does not support this. See https://d2lang.com/tour/layouts/#layout-specific-functionality for more."
            ));
        }
        if is_descendant(edge.source, edge.target) || is_descendant(edge.target, edge.source) {
            return Err(format!(
                "Connection \"{abs_id}\" goes from a container to a descendant, but layout engine \"{name}\" does not support this. See https://d2lang.com/tour/layouts/#layout-specific-functionality for more."
            ));
        }
    }
    Ok(())
}
