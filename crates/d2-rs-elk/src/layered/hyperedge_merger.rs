// Copyright (c) 2010, 2018 Kiel University and others.
// Rust translation and modifications: Copyright 2026 Nelson Chen.
// This program and the accompanying materials are made available under the
// terms of the Eclipse Public License 2.0 which is available at
// https://www.eclipse.org/legal/epl-2.0.
// SPDX-License-Identifier: EPL-2.0

//! Source HyperedgeDummyMerger: port DFS and adjacent long-edge coalescing.
use super::{
    LGraphId, LNodeId, LPortId, LayeredArena, LayeredNodeType, LayeringError, PortSide,
    gather_center_label_dummy_info, rewire_edge_endpoint,
};
use std::collections::BTreeSet;

type Ends = (Option<LPortId>, Option<LPortId>);
fn endpoints(arena: &LayeredArena, node: LNodeId) -> Ends {
    arena
        .long_edge_merge_ends
        .get(&node)
        .copied()
        .or_else(|| {
            arena
                .long_edge_endpoints
                .get(&node)
                .map(|(a, b)| (Some(*a), Some(*b)))
        })
        .unwrap_or_default()
}

pub(super) fn merge(arena: &mut LayeredArena, graph: LGraphId) -> Result<(), LayeringError> {
    let layers = arena.graphs[graph as usize].layers.clone();
    let ports = layers
        .iter()
        .flat_map(|layer| arena.layers[*layer as usize].nodes.iter())
        .flat_map(|node| arena.nodes[*node as usize].ports.iter().copied())
        .collect::<Vec<_>>();
    // Nonmember ports are not part of this graph's temporary -1 ID set.
    let mut ids = vec![Some(usize::MAX); arena.ports.len()];
    for port in &ports {
        ids[*port as usize] = None;
    }
    let mut next_id = 0;
    for port in &ports {
        if ids[*port as usize].is_some() {
            continue;
        }
        let mut stack = vec![*port];
        while let Some(port) = stack.pop() {
            if ids[port as usize].is_some() {
                continue;
            }
            ids[port as usize] = Some(next_id);
            let p = &arena.ports[port as usize];
            let mut neighbors = Vec::new();
            for edge in &p.incoming_edges {
                if arena.edges[*edge as usize].active
                    && let Some(source) = arena.edges[*edge as usize].source_port
                {
                    neighbors.push(source);
                }
            }
            for edge in &p.outgoing_edges {
                if arena.edges[*edge as usize].active
                    && let Some(target) = arena.edges[*edge as usize].target_port
                {
                    neighbors.push(target);
                }
            }
            if arena.nodes[p.node as usize].node_type == LayeredNodeType::LongEdge {
                neighbors.extend(
                    arena.nodes[p.node as usize]
                        .ports
                        .iter()
                        .copied()
                        .filter(|other| *other != port),
                );
            }
            stack.extend(
                neighbors
                    .into_iter()
                    .rev()
                    .filter(|other| ids[*other as usize].is_none()),
            );
        }
        next_id += 1;
    }
    // LabelDummySwitcher marks every long-edge node before its label, even
    // when that label already occupies the selected layer.
    let mut before_label = BTreeSet::new();
    for layer in &layers {
        for node in &arena.layers[*layer as usize].nodes {
            if arena.nodes[*node as usize].node_type == LayeredNodeType::Label {
                before_label
                    .extend(gather_center_label_dummy_info(arena, graph, *node)?.left_long_edges);
            }
        }
    }
    for layer in layers {
        let mut index = 1;
        while index < arena.layers[layer as usize].nodes.len() {
            let last = arena.layers[layer as usize].nodes[index - 1];
            let current = arena.layers[layer as usize].nodes[index];
            if arena.nodes[last as usize].node_type != LayeredNodeType::LongEdge
                || arena.nodes[current as usize].node_type != LayeredNodeType::LongEdge
            {
                index += 1;
                continue;
            }
            let a = endpoints(arena, current);
            let b = endpoints(arena, last);
            let same_source = a.0.is_some() && a.0 == b.0;
            let same_target = a.1.is_some() && a.1 == b.1;
            let current_labels = arena.long_edge_label_dummies.contains(&current);
            let last_labels = arena.long_edge_label_dummies.contains(&last);
            let allow = if !current_labels && !last_labels {
                let Some(p) = arena.nodes[current as usize].ports.first() else {
                    return Err(LayeringError::Import(
                        "layered: long-edge dummy has no ports".into(),
                    ));
                };
                let Some(q) = arena.nodes[last as usize].ports.first() else {
                    return Err(LayeringError::Import(
                        "layered: long-edge dummy has no ports".into(),
                    ));
                };
                ids[*p as usize] == ids[*q as usize]
            } else {
                label_merge_allowed(
                    same_source,
                    same_target,
                    current_labels,
                    last_labels,
                    before_label.contains(&current),
                    before_label.contains(&last),
                )
            };
            if !allow {
                index += 1;
                continue;
            }
            let input = arena.nodes[last as usize]
                .ports
                .iter()
                .copied()
                .find(|p| arena.ports[*p as usize].side == PortSide::West);
            let output = arena.nodes[last as usize]
                .ports
                .iter()
                .copied()
                .find(|p| arena.ports[*p as usize].side == PortSide::East);
            let (Some(input), Some(output)) = (input, output) else {
                return Err(LayeringError::Import(format!(
                    "layered: long-edge merge target {last} has no west/east ports"
                )));
            };
            for port in arena.nodes[current as usize].ports.clone() {
                while let Some(edge) = arena.ports[port as usize].incoming_edges.first().copied() {
                    rewire_edge_endpoint(arena, edge, false, last, input);
                }
                while let Some(edge) = arena.ports[port as usize].outgoing_edges.first().copied() {
                    rewire_edge_endpoint(arena, edge, true, last, output);
                }
            }
            let retained = (
                if same_source { b.0 } else { None },
                if same_target { b.1 } else { None },
            );
            arena.long_edge_merge_ends.insert(last, retained);
            if !same_source || !same_target {
                arena.long_edge_endpoints.remove(&last);
            }
            // Source removes the layer entry but retains the dummy Layer pointer.
            arena.layers[layer as usize].nodes.remove(index);
            #[cfg(feature = "diagnostic-traces")]
            crate::trace::emit_value(
                "hierarchy",
                "hyperedge_merge",
                &format!("graphs[{graph}]/layers[{layer}]/nodes[{current}]"),
                &serde_json::json!({
                    "retired":current,"surviving":last,"input_port":input,"output_port":output,
                    "same_source":same_source,"same_target":same_target,
                    "source":retained.0,"target":retained.1,
                    "incoming":arena.ports[input as usize].incoming_edges,
                    "outgoing":arena.ports[output as usize].outgoing_edges,
                    "layer_nodes":arena.layers[layer as usize].nodes
                })
                .to_string(),
            );
        }
    }
    Ok(())
}

fn label_merge_allowed(
    same_source: bool,
    same_target: bool,
    current_labels: bool,
    last_labels: bool,
    current_before: bool,
    last_before: bool,
) -> bool {
    let source_eligible = (!current_labels || current_before) && (!last_labels || last_before);
    let target_eligible = (!current_labels || !current_before) && (!last_labels || !last_before);
    same_source && source_eligible || same_target && target_eligible
}

#[cfg(test)]
mod tests {
    use super::super::{EndpointRef, Layer, join_long_edges};
    use super::*;
    fn parallel(separate: bool) -> (LayeredArena, LGraphId, Vec<LPortId>) {
        let children = (0..if separate { 6 } else { 5 })
            .map(|i| serde_json::json!({"id":format!("n{i}"),"width":40,"height":30}))
            .collect::<Vec<_>>();
        let graph: crate::model::ElkGraph =
            serde_json::from_value(serde_json::json!({"id":"root","children":children})).unwrap();
        let (mut arena, root) = LayeredArena::from_elk_graph(&graph).unwrap();
        arena.layers = vec![
            Layer {
                graph: root,
                nodes: vec![0],
                ..Default::default()
            },
            Layer {
                graph: root,
                nodes: if separate { vec![1, 5, 2] } else { vec![1, 2] },
                ..Default::default()
            },
            Layer {
                graph: root,
                nodes: vec![3, 4],
                ..Default::default()
            },
        ];
        arena.graphs[root as usize].layers = vec![0, 1, 2];
        arena.graphs[root as usize].layerless_nodes.clear();
        for (layer, nodes) in [
            (0, vec![0]),
            (1, if separate { vec![1, 5, 2] } else { vec![1, 2] }),
            (2, vec![3, 4]),
        ] {
            for node in nodes {
                arena.nodes[node].layer = Some(layer);
            }
        }
        for node in [1, 2] {
            arena.nodes[node].node_type = LayeredNodeType::LongEdge;
            arena.nodes[node].width = 0.0;
            arena.nodes[node].height = 0.0;
        }
        let mut ports = Vec::new();
        for (node, side) in [
            (0, PortSide::East),
            (1, PortSide::West),
            (1, PortSide::East),
            (2, PortSide::West),
            (2, PortSide::East),
            (3, PortSide::West),
            (4, PortSide::West),
        ] {
            ports.push(arena.add_dummy_port(node, side));
        }
        for (source, target) in [(0, 1), (0, 3), (2, 5), (4, 6)] {
            arena.add_edge(
                root,
                EndpointRef {
                    node: arena.ports[ports[source] as usize].node,
                    port: Some(ports[source]),
                },
                EndpointRef {
                    node: arena.ports[ports[target] as usize].node,
                    port: Some(ports[target]),
                },
            );
        }
        arena
            .long_edge_merge_ends
            .insert(1, (Some(ports[0]), Some(ports[5])));
        arena
            .long_edge_merge_ends
            .insert(2, (Some(ports[0]), Some(ports[6])));
        (arena, root, ports)
    }
    #[test]
    fn original_go_adjacent_hyperedge_proxies_merge_and_join_in_adjacency_order() {
        let (mut arena, root, p) = parallel(false);
        merge(&mut arena, root).unwrap();
        assert_eq!(arena.layers[1].nodes, vec![1]);
        assert_eq!(arena.ports[p[1] as usize].incoming_edges, vec![0, 1]);
        assert_eq!(arena.ports[p[2] as usize].outgoing_edges, vec![2, 3]);
        assert_eq!(arena.long_edge_merge_ends[&1], (Some(p[0]), None));
        for port in arena.nodes[2].ports.clone() {
            assert!(arena.ports[port as usize].incoming_edges.is_empty());
            assert!(arena.ports[port as usize].outgoing_edges.is_empty());
        }
        join_long_edges(&mut arena, root).unwrap();
        assert!(arena.layers[1].nodes.is_empty());
        assert_eq!(arena.ports[p[5] as usize].incoming_edges, vec![0]);
        assert_eq!(arena.ports[p[6] as usize].incoming_edges, vec![1]);
        assert_eq!((arena.edges[0].source, arena.edges[0].target), (0, 3));
        assert_eq!((arena.edges[1].source, arena.edges[1].target), (0, 4));
        assert!(!arena.edges[2].active && !arena.edges[3].active);
        for node in [1, 2] {
            for port in &arena.nodes[node].ports {
                assert!(arena.ports[*port as usize].incoming_edges.is_empty());
                assert!(arena.ports[*port as usize].outgoing_edges.is_empty());
            }
        }
    }
    #[test]
    fn original_go_hyperedge_merger_requires_layer_adjacency() {
        let (mut arena, root, _) = parallel(true);
        merge(&mut arena, root).unwrap();
        assert_eq!(arena.layers[1].nodes, vec![1, 5, 2]);
    }
    #[test]
    fn original_go_label_side_guards_require_the_corresponding_common_endpoint() {
        assert!(label_merge_allowed(true, false, true, true, true, true));
        assert!(!label_merge_allowed(true, false, true, true, false, false));
        assert!(label_merge_allowed(false, true, true, true, false, false));
        assert!(!label_merge_allowed(false, true, true, true, true, true));
        assert!(!label_merge_allowed(true, true, true, true, true, false));
        let (mut arena, root, _) = parallel(false);
        arena.long_edge_label_dummies.extend([1, 2]);
        merge(&mut arena, root).unwrap();
        assert_eq!(arena.layers[1].nodes, vec![1, 2]);
    }
    #[test]
    fn original_go_hyperedge_configuration_uses_import_time_adjacency() {
        let input = serde_json::json!({"id":"root","children":[
            {"id":"a","width":40,"height":30,"ports":[{"id":"a.p","x":0,"y":0,"width":0,"height":0,"layoutOptions":{}}]},
            {"id":"b","width":40,"height":30,"ports":[{"id":"b.p","x":0,"y":0,"width":0,"height":0,"layoutOptions":{}}]},
            {"id":"c","width":40,"height":30,"ports":[{"id":"c.p","x":0,"y":0,"width":0,"height":0,"layoutOptions":{}}]}
        ],"edges":[
            {"id":"ab","sources":["a.p"],"targets":["b.p"]},
            {"id":"bc","sources":["b.p"],"targets":["c.p"]}
        ]});
        let graph: crate::model::ElkGraph = serde_json::from_value(input.clone()).unwrap();
        let (mut arena, root) = LayeredArena::from_elk_graph(&graph).unwrap();
        assert!(!arena.graphs[root as usize].has_hyperedges);
        // P1 reversal can create a shared outgoing port. The source property
        // remains the import-time value rather than being recomputed at P4.
        let a = arena.edges[0].source_port.unwrap();
        let b = arena.edges[0].target_port.unwrap();
        let a_node = arena.ports[a as usize].node;
        let b_node = arena.ports[b as usize].node;
        rewire_edge_endpoint(&mut arena, 0, false, a_node, a);
        rewire_edge_endpoint(&mut arena, 0, true, b_node, b);
        assert_eq!(arena.ports[b as usize].outgoing_edges.len(), 2);
        assert!(!arena.graphs[root as usize].has_hyperedges);
        let mut shared = input;
        shared["edges"][0]["sources"] = serde_json::json!(["b.p"]);
        shared["edges"][0]["targets"] = serde_json::json!(["a.p"]);
        let graph: crate::model::ElkGraph = serde_json::from_value(shared).unwrap();
        let (arena, root) = LayeredArena::from_elk_graph(&graph).unwrap();
        assert!(arena.graphs[root as usize].has_hyperedges);
    }
}
