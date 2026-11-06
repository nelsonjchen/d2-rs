// Copyright (c) 2008, 2013, 2015, 2020 Kiel University and others.
// Rust translation and modifications: Copyright 2026 Nelson Chen.
// This program and the accompanying materials are made available under the
// terms of the Eclipse Public License 2.0 which is available at
// https://www.eclipse.org/legal/epl-2.0.
// SPDX-License-Identifier: EPL-2.0

//! Cross-axis evaluation order of compound.go and the layout transferrer.
use super::{
    LEdge, LEdgeId, LGraphId, LNodeId, LayeredArena, LayeringError, lowest_common_graph,
    node_is_ancestor,
};

pub(crate) fn source_node_y(arena: &LayeredArena, node: LNodeId) -> f64 {
    arena
        .nested_transfer_y
        .get(&node)
        .map_or(arena.nodes[node as usize].position_y, |entry| entry.0)
}
fn padding_y(arena: &LayeredArena, graph: LGraphId) -> f64 {
    arena.graphs[graph as usize].padding.map_or(12.0, |p| p.top)
}
pub(crate) fn hierarchy_reference_graph(arena: &LayeredArena, edge: &LEdge) -> LGraphId {
    if node_is_ancestor(arena, edge.source, edge.target) {
        if let Some(graph) = arena.nodes[edge.source as usize].nested_graph {
            return graph;
        }
    }
    arena.nodes[edge.source as usize].graph
}
/// The source mutates a coordinate through both complete chains. Replacing
/// these operations with the difference of two origins changes exact f64 bits.
pub(crate) fn change_graph_y(
    arena: &LayeredArena,
    mut point: f64,
    from: LGraphId,
    to: LGraphId,
) -> f64 {
    if from == to {
        return point;
    }
    let mut current = from;
    loop {
        let graph = &arena.graphs[current as usize];
        point += graph.offset_y;
        let Some(parent) = graph.parent_node else {
            break;
        };
        point += padding_y(arena, current);
        point += source_node_y(arena, parent);
        current = arena.nodes[parent as usize].graph;
    }
    current = to;
    loop {
        let graph = &arena.graphs[current as usize];
        point -= graph.offset_y;
        let Some(parent) = graph.parent_node else {
            break;
        };
        point -= padding_y(arena, current);
        point -= source_node_y(arena, parent);
        current = arena.nodes[parent as usize].graph;
    }
    point
}
pub(crate) fn hierarchy_edge_offset_y(arena: &LayeredArena, edge: &LEdge) -> f64 {
    let root = arena
        .graphs
        .iter()
        .position(|g| g.parent_node.is_none())
        .unwrap_or(0) as LGraphId;
    // Codec.UpdateContainment assigns ancestor edges to that ancestor's
    // own contained graph, rather than its parent's graph.
    let origin = if node_is_ancestor(arena, edge.source, edge.target) {
        arena.nodes[edge.source as usize]
            .nested_graph
            .unwrap_or(root)
    } else if node_is_ancestor(arena, edge.target, edge.source) {
        arena.nodes[edge.target as usize]
            .nested_graph
            .unwrap_or(root)
    } else {
        lowest_common_graph(
            arena,
            arena.nodes[edge.source as usize].graph,
            arena.nodes[edge.target as usize].graph,
            root,
        )
    };
    // Transferrer.edgeLayoutOffsets takes the base offset from the layout
    // (reference) graph, but walks the hierarchical offset from the source
    // node's own graph. For an ancestor source those differ: the walk starts
    // outside the ancestor and never meets its child-graph origin, so it
    // accumulates every enclosing node and graph up to the root.
    let reference = hierarchy_reference_graph(arena, edge);
    let base = arena.graphs[reference as usize].offset_y + padding_y(arena, reference);
    let mut current = arena.nodes[edge.source as usize].graph;
    let mut hierarchy = 0.0;
    while current != origin {
        let Some(parent) = arena.graphs[current as usize].parent_node else {
            break;
        };
        hierarchy += source_node_y(arena, parent);
        current = arena.nodes[parent as usize].graph;
        hierarchy += arena.graphs[current as usize].offset_y;
        hierarchy += padding_y(arena, current);
    }
    base + hierarchy
}
/// Flow-axis counterpart of [`hierarchy_edge_offset_y`] for joined
/// cross-hierarchy routes, which arrive in the endpoints' common graph frame.
///
/// compound.go's postprocessor and Transferrer.applyEdge publish an edge
/// between an ancestor and its descendant relative to the ancestor node, so
/// the ancestor's own position is removed. For an ancestor source,
/// edgeLayoutOffsets then adds the enclosing nodes and the root padding it
/// collects while walking from the ancestor's graph towards an origin it
/// never reaches. Arena flow-axis positions of nested nodes already include
/// their graph padding, so only the root padding is added explicitly.
pub(crate) fn hierarchy_edge_offset_x(arena: &LayeredArena, edge: &LEdge) -> f64 {
    let ancestor = if node_is_ancestor(arena, edge.source, edge.target) {
        edge.source
    } else if node_is_ancestor(arena, edge.target, edge.source) {
        edge.target
    } else {
        return 0.0;
    };
    let mut offset = -arena.nodes[ancestor as usize].position_x;
    if ancestor == edge.source {
        let mut current = arena.nodes[ancestor as usize].graph;
        while let Some(parent) = arena.graphs[current as usize].parent_node {
            offset += arena.nodes[parent as usize].position_x;
            current = arena.nodes[parent as usize].graph;
            if arena.graphs[current as usize].parent_node.is_none() {
                offset += arena.graphs[current as usize]
                    .padding
                    .map_or(12.0, |padding| padding.left);
            }
        }
    }
    offset
}
pub(super) fn local_anchor_y(arena: &LayeredArena, edge: &LEdge, source: bool) -> f64 {
    let node = if source { edge.source } else { edge.target };
    let port = if source {
        edge.source_port
    } else {
        edge.target_port
    };
    (source_node_y(arena, node)
        + port.map_or(arena.nodes[node as usize].height / 2.0, |p| {
            arena.ports[p as usize].y
        }))
        + 0.0
}
pub(super) fn hierarchy_endpoint_y(arena: &LayeredArena, edge: &LEdge, source: bool) -> f64 {
    let reference = hierarchy_reference_graph(arena, edge);
    if source {
        if node_is_ancestor(arena, edge.source, edge.target) {
            let port = edge
                .source_port
                .map_or(arena.nodes[edge.source as usize].height / 2.0, |p| {
                    arena.ports[p as usize].y
                });
            return (port + 0.0)
                - (arena.graphs[reference as usize].offset_y + padding_y(arena, reference));
        }
        local_anchor_y(arena, edge, true)
    } else {
        local_anchor_y(arena, edge, false)
            + change_graph_y(
                arena,
                0.0,
                arena.nodes[edge.target as usize].graph,
                reference,
            )
    }
}
pub(super) fn segment_y(arena: &LayeredArena, id: LEdgeId) -> Result<Vec<f64>, LayeringError> {
    let edge = &arena.edges[id as usize];
    let mut ys = Vec::with_capacity(edge.bend_points.len() + 2);
    ys.push(local_anchor_y(arena, edge, true));
    if let Some((local, _)) = arena.nested_edge_transfer_y.get(&id) {
        if local.len() != edge.bend_points.len() {
            return Err(LayeringError::Import(
                "layered: incomplete source segment bend inventory".into(),
            ));
        }
        ys.extend(local);
    } else {
        ys.extend(edge.bend_points.iter().map(|p| p.y));
    }
    ys.push(local_anchor_y(arena, edge, false));
    Ok(ys)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_reference_graph_operations_preserve_fractional_offsets_and_label_bits() {
        // Frozen source arithmetic trace: compound.changeGraphCoordinates and
        // Transferrer.edgeLayoutOffsets, before the final codec publication.
        let wire: crate::model::ElkGraph = serde_json::from_value(serde_json::json!({
            "id":"root","layoutOptions":{"elk.hierarchyHandling":"INCLUDE_CHILDREN"},
            "children":[{"id":"g","width":279,"height":160,"children":[
                {"id":"b","width":30,"height":20}]},{"id":"outside","width":50,"height":40}],
            "edges":[{"id":"boundary","sources":["b"],"targets":["outside"]}]
        }))
        .unwrap();
        let (mut arena, root) = LayeredArena::from_elk_graph(&wire).unwrap();
        super::super::materialize_implicit_ports(&mut arena, root).unwrap();
        let edge = arena.edges[0].clone();
        let child = arena.nodes[edge.source as usize].graph;
        let parent = arena.graphs[child as usize].parent_node.unwrap();
        arena.graphs[root as usize].offset_y = 175.0;
        arena.graphs[child as usize].offset_y = 82.66666666666667;
        arena.nodes[parent as usize].position_y = -150.0;
        arena.nodes[edge.source as usize].position_y = -28.0;
        arena.nested_transfer_y.insert(edge.source, (-40.0, 12.0));
        let offset = hierarchy_edge_offset_y(&arena, &edge);
        assert_eq!(offset.to_bits(), 131.66666666666669_f64.to_bits());
        assert_eq!(
            change_graph_y(&arena, 0.0, root, child).to_bits(),
            55.333333333333314_f64.to_bits()
        );
        assert_eq!(
            (hierarchy_endpoint_y(&arena, &edge, true) + offset).to_bits(),
            101.66666666666669_f64.to_bits()
        );
        assert_eq!(
            (hierarchy_endpoint_y(&arena, &edge, false) + offset).to_bits(),
            207.0_f64.to_bits()
        );
        let local_label = change_graph_y(&arena, -175.0, root, child);
        assert_eq!(local_label.to_bits(), (-119.66666666666667_f64).to_bits());
        assert_eq!(
            (local_label + offset).to_bits(),
            12.000000000000014_f64.to_bits()
        );
    }
    #[test]
    fn source_identical_graph_conversion_does_no_arithmetic_on_negative_zero() {
        let wire: crate::model::ElkGraph =
            serde_json::from_value(serde_json::json!({"id":"root"})).unwrap();
        let (mut arena, root) = LayeredArena::from_elk_graph(&wire).unwrap();
        arena.graphs[root as usize].offset_y = 175.0;
        assert_eq!(
            change_graph_y(&arena, -0.0, root, root).to_bits(),
            (-0.0_f64).to_bits()
        );
    }
}
