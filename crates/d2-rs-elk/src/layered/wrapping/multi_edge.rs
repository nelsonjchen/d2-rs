// Copyright (c) 2016 Kiel University and others.
// Copyright (c) 2026 The elk-go Authors.
// SPDX-License-Identifier: EPL-2.0
// Port of intermediate/wrapping/breaking_point_{inserter,processor,remover}.go
// at elk-go 8b285820520e.
use super::*;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct BPInfo {
    start: LNodeId,
    end: LNodeId,
    node_start_edge: LEdgeId,
    start_end_edge: LEdgeId,
    original_edge: LEdgeId,
    start_dummy: Option<LNodeId>,
    end_dummy: Option<LNodeId>,
    start_in_layer_edge: Option<LEdgeId>,
    end_in_layer_edge: Option<LEdgeId>,
    prev: Option<usize>,
    next: Option<usize>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct State {
    infos: Vec<BPInfo>,
    nodes: BTreeMap<LNodeId, usize>,
    processed: BTreeSet<LGraphId>,
}

fn add_layer(arena: &mut LayeredArena, graph: LGraphId) -> LayerId {
    let layer = arena.layers.len() as LayerId;
    arena.layers.push(Layer {
        graph,
        ..Layer::default()
    });
    layer
}

fn marker(
    arena: &mut LayeredArena,
    graph: LGraphId,
    layer: LayerId,
) -> (LNodeId, LPortId, LPortId) {
    let node = arena.add_long_edge_dummy(graph, layer, 0);
    let n = &mut arena.nodes[node as usize];
    n.node_type = LayeredNodeType::BreakingPoint;
    n.port_constraints = PortConstraints::FixedSide;
    n.height = 0.;
    let input = arena.add_dummy_port(node, PortSide::West);
    let output = arena.add_dummy_port(node, PortSide::East);
    (node, input, output)
}

pub(crate) fn insert_breaking_points(arena: &mut LayeredArena, graph_id: LGraphId) {
    let graph = &arena.graphs[graph_id as usize];
    if graph.wrapping.strategy != "MULTI_EDGE" || graph.layers.is_empty() {
        return;
    }
    let stats = GraphStats::new(arena, graph_id);
    let mut cuts = stats.cuts(&graph.wrapping);
    if graph.wrapping.improve_cuts {
        cuts = improve_cuts(arena, graph_id, &cuts);
    }
    // Unlike SINGLE_EDGE, the source validates only an explicitly set strategy.
    if let Some(strategy) = &graph.wrapping.validify {
        cuts = stats.validify(cuts, strategy);
    }
    if !cuts.is_empty() {
        apply_cuts(arena, graph_id, &cuts);
    }
}

fn apply_cuts(arena: &mut LayeredArena, graph: LGraphId, cuts: &[i32]) {
    let (mut position, mut original_index, mut cut_index) = (0, 0, 0);
    let mut cut = cuts[0];
    let mut open = Vec::new();
    let mut open_set = BTreeSet::new();
    while position < arena.graphs[graph as usize].layers.len() {
        let layer = arena.graphs[graph as usize].layers[position];
        position += 1;
        for node in arena.layers[layer as usize].nodes.clone() {
            for edge in arena.node_outgoing_edges(node) {
                if open_set.insert(edge) && !open.contains(&edge) {
                    open.push(edge);
                }
            }
            for edge in arena.node_incoming_edges(node) {
                open_set.remove(&edge);
            }
        }
        if original_index + 1 == cut {
            let first = add_layer(arena, graph);
            arena.graphs[graph as usize].layers.insert(position, first);
            position += 1;
            let second = add_layer(arena, graph);
            arena.graphs[graph as usize].layers.insert(position, second);
            position += 1;
            for original in open.iter().copied().filter(|e| open_set.contains(e)) {
                let (start, start_in, start_out) = marker(arena, graph, first);
                let (end, end_in, end_out) = marker(arena, graph, second);
                let old_source = arena.edges[original as usize].source_port.unwrap();
                let source_node = arena.ports[old_source as usize].node;
                let node_start = arena.add_edge(
                    graph,
                    EndpointRef {
                        node: source_node,
                        port: Some(old_source),
                    },
                    EndpointRef {
                        node: start,
                        port: Some(start_in),
                    },
                );
                let start_end = arena.add_edge(
                    graph,
                    EndpointRef {
                        node: start,
                        port: Some(start_out),
                    },
                    EndpointRef {
                        node: end,
                        port: Some(end_in),
                    },
                );
                if let Some(order) = arena.edge_model_order.get(&original).copied() {
                    arena.edge_model_order.insert(node_start, order);
                    arena.edge_model_order.insert(start_end, order);
                } else {
                    arena.edge_model_order.remove(&node_start);
                    arena.edge_model_order.remove(&start_end);
                }
                set_edge_source_port(arena, original, end_out);
                let prev = (arena.nodes[source_node as usize].node_type
                    == LayeredNodeType::BreakingPoint)
                    .then(|| arena.breaking_points.nodes[&source_node]);
                let id = arena.breaking_points.infos.len();
                arena.breaking_points.infos.push(BPInfo {
                    start,
                    end,
                    node_start_edge: node_start,
                    start_end_edge: start_end,
                    original_edge: original,
                    start_dummy: None,
                    end_dummy: None,
                    start_in_layer_edge: None,
                    end_in_layer_edge: None,
                    prev,
                    next: None,
                });
                arena.breaking_points.nodes.insert(start, id);
                arena.breaking_points.nodes.insert(end, id);
                if let Some(prev) = prev {
                    arena.breaking_points.infos[prev].next = Some(id);
                }
            }
            cut_index += 1;
            if cut_index < cuts.len() {
                cut = cuts[cut_index];
            } else {
                break;
            }
        }
        original_index += 1;
    }
}

#[derive(Clone)]
struct Cut {
    index: i32,
    new_index: i32,
    assigned: bool,
}

fn next_candidate(candidates: &[Cut], start: usize) -> Option<usize> {
    (start..candidates.len()).find(|&i| !candidates[i].assigned)
}

fn improve_cuts(arena: &LayeredArena, graph_id: LGraphId, cuts: &[i32]) -> Vec<i32> {
    let mut candidates = cuts
        .iter()
        .map(|&index| Cut {
            index,
            new_index: 0,
            assigned: false,
        })
        .collect::<Vec<_>>();
    let spans = edge_spans(arena, graph_id);
    let mut improved = Vec::new();
    for _ in 0..candidates.len() {
        let mut left: Option<usize> = None;
        let mut right = next_candidate(&candidates, 0);
        let mut best = None;
        let mut best_score = f64::INFINITY;
        for index in 1..arena.graphs[graph_id as usize].layers.len() {
            let index = index as i32;
            let right_distance = right
                .map(|r| (candidates[r].index - index).abs())
                .unwrap_or_else(|| (index - candidates[left.unwrap()].index).abs() + 1);
            let left_distance = left
                .map(|l| (index - candidates[l].index).abs())
                .unwrap_or(right_distance + 1);
            let (distance, hit) = if left_distance < right_distance {
                (left_distance, left)
            } else {
                (right_distance, right)
            };
            let score = spans[index as usize] as f64
                + (distance as f64).powf(arena.graphs[graph_id as usize].wrapping.distance_penalty);
            if score < best_score {
                best_score = score;
                best = hit;
                candidates[hit.unwrap()].new_index = index;
            }
            if right.is_some_and(|r| index == candidates[r].index) {
                left = right;
                right = next_candidate(&candidates, right.unwrap() + 1);
            }
        }
        if let Some(best) = best {
            improved.push(candidates[best].new_index);
            candidates[best].assigned = true;
            let offset = candidates[best].new_index - candidates[best].index;
            candidates[best].index += offset;
            for i in (0..best).rev() {
                if candidates[i].assigned {
                    break;
                }
                candidates[i].index += offset;
            }
            for candidate in &mut candidates[best + 1..] {
                if candidate.assigned {
                    break;
                }
                candidate.index += offset;
            }
        }
    }
    improved.sort();
    improved
}

fn edge_spans(arena: &LayeredArena, graph: LGraphId) -> Vec<usize> {
    let mut spans = vec![0; arena.graphs[graph as usize].layers.len() + 1];
    let mut open = BTreeSet::new();
    for (i, &layer) in arena.graphs[graph as usize].layers.iter().enumerate() {
        spans[i] = open.len();
        for &node in &arena.layers[layer as usize].nodes {
            open.extend(arena.node_outgoing_edges(node));
        }
        for &node in &arena.layers[layer as usize].nodes {
            for edge in arena.node_incoming_edges(node) {
                open.remove(&edge);
            }
        }
    }
    spans
}

pub(crate) fn process_breaking_points(
    arena: &mut LayeredArena,
    graph: LGraphId,
) -> Result<(), LayeringError> {
    if arena.graphs[graph as usize].wrapping.strategy != "MULTI_EDGE" {
        return Ok(());
    }
    if !arena.breaking_points.processed.insert(graph) {
        return Ok(());
    }
    let empty = add_layer(arena, graph);
    arena.graphs[graph as usize].layers.insert(0, empty);
    let (mut reverse, mut index, mut position) = (false, 1i32, 1);
    while position < arena.graphs[graph as usize].layers.len() {
        let layer = arena.graphs[graph as usize].layers[position];
        position += 1;
        let new_layer = arena.graphs[graph as usize].layers[index as usize];
        let nodes = arena.layers[layer as usize].nodes.clone();
        let offset = nodes.len();
        for &node in &nodes {
            append_node(arena, node, new_layer);
        }
        if reverse {
            for &node in nodes.iter().rev() {
                for edge in arena.node_incoming_edges(node) {
                    arena.reverse_edge(edge, true);
                    let dummy_edges = insert_dummies(arena, graph, edge, offset)?;
                    let start_edge = *dummy_edges.last().unwrap();
                    let id = arena.breaking_points.nodes[&node];
                    let info = &mut arena.breaking_points.infos[id];
                    info.start_dummy = Some(arena.edges[start_edge as usize].source);
                    info.start_in_layer_edge = Some(start_edge);
                    info.end_dummy = Some(arena.edges[edge as usize].target);
                    info.end_in_layer_edge = Some(edge);
                }
            }
            reverse = false;
        } else if nodes
            .first()
            .is_some_and(|&n| arena.nodes[n as usize].node_type == LayeredNodeType::BreakingPoint)
        {
            reverse = true;
            index = -1;
        }
        index += 1;
    }
    arena.graphs[graph as usize]
        .layers
        .retain(|&l| !arena.layers[l as usize].nodes.is_empty());
    if !arena.graphs[graph as usize].wrapping.improve_wrapped {
        return Ok(());
    }
    for &layer in &arena.graphs[graph as usize].layers {
        for (i, &node) in arena.layers[layer as usize].nodes.iter().enumerate() {
            arena.nodes[node as usize].traversal_index = i;
        }
    }
    improve_multi_cut_edges(arena, graph)?;
    improve_long_edges(arena, graph, true)?;
    improve_long_edges(arena, graph, false)?;
    Ok(())
}

fn info_for(arena: &LayeredArena, node: LNodeId, start: bool) -> Option<usize> {
    let id = *arena.breaking_points.nodes.get(&node)?;
    let info = &arena.breaking_points.infos[id];
    ((if start { info.start } else { info.end }) == node).then_some(id)
}

fn detach_node(arena: &mut LayeredArena, node: LNodeId) {
    if let Some(layer) = arena.nodes[node as usize].layer.take() {
        arena.layers[layer as usize].nodes.retain(|&n| n != node);
    }
    // Go enumerates edges through layered nodes. Its detached, still allocated
    // chains must also disappear from Rust's active-edge arena scans.
    for edge in arena
        .node_incoming_edges(node)
        .into_iter()
        .chain(arena.node_outgoing_edges(node))
    {
        retire_edge(arena, edge);
    }
}

fn retire_edge(arena: &mut LayeredArena, edge: LEdgeId) {
    if let Some(p) = arena.edges[edge as usize].source_port.take() {
        arena.ports[p as usize]
            .outgoing_edges
            .retain(|&e| e != edge);
    }
    if let Some(p) = arena.edges[edge as usize].target_port.take() {
        arena.ports[p as usize]
            .incoming_edges
            .retain(|&e| e != edge);
    }
    arena.edges[edge as usize].active = false;
}

fn improve_multi_cut_edges(arena: &mut LayeredArena, graph: LGraphId) -> Result<(), LayeringError> {
    for layer in arena.graphs[graph as usize].layers.clone() {
        for node in arena.layers[layer as usize].nodes.clone() {
            let Some(id) = info_for(arena, node, true) else {
                continue;
            };
            let mut current = arena.breaking_points.infos[id].clone();
            if current.prev.is_some() || current.next.is_none() {
                continue;
            }
            let mut next = current.next;
            while let Some(next_id) = next {
                let next_info = arena.breaking_points.infos[next_id].clone();
                drop_dummies(
                    arena,
                    next_info.start,
                    next_info.start_dummy.unwrap(),
                    false,
                    true,
                )?;
                for n in [
                    current.end,
                    next_info.start,
                    next_info.start_dummy.unwrap(),
                    next_info.end_dummy.unwrap(),
                ] {
                    update_indexes_after(arena, n);
                }
                let current_end_edge = current.end_in_layer_edge.unwrap();
                let target = arena.edges[current_end_edge as usize].target_port.unwrap();
                set_edge_target_port(arena, next_info.end_in_layer_edge.unwrap(), target);
                // Source only detaches the target here: the old edge is no
                // longer active but its source slot is not used afterwards.
                retire_edge(arena, current_end_edge);
                for n in [
                    current.end,
                    next_info.start,
                    next_info.start_dummy.unwrap(),
                    next_info.end_dummy.unwrap(),
                ] {
                    detach_node(arena, n);
                }
                let new_info = BPInfo {
                    start: current.start,
                    end: next_info.end,
                    node_start_edge: current.node_start_edge,
                    start_end_edge: next_info.start_end_edge,
                    original_edge: next_info.original_edge,
                    start_dummy: current.start_dummy,
                    start_in_layer_edge: current.start_in_layer_edge,
                    end_dummy: current.end_dummy,
                    end_in_layer_edge: next_info.end_in_layer_edge,
                    prev: current.prev,
                    next: next_info.next,
                };
                let new_id = arena.breaking_points.infos.len();
                arena.breaking_points.nodes.insert(current.start, new_id);
                arena.breaking_points.nodes.insert(next_info.end, new_id);
                arena.breaking_points.infos.push(new_info.clone());
                next = next_info.next;
                current = new_info;
            }
        }
    }
    Ok(())
}

fn improve_long_edges(
    arena: &mut LayeredArena,
    graph: LGraphId,
    forwards: bool,
) -> Result<(), LayeringError> {
    loop {
        let mut changed = false;
        let mut layers = arena.graphs[graph as usize].layers.clone();
        if forwards {
            layers.reverse();
        }
        for layer in layers {
            // Source's ignored Lists.reverse result leaves nodes in forward order.
            for node in arena.layers[layer as usize].nodes.clone() {
                let Some(id) = info_for(arena, node, !forwards) else {
                    continue;
                };
                let info = &arena.breaking_points.infos[id];
                let dummy = if forwards {
                    info.end_dummy
                } else {
                    info.start_dummy
                }
                .unwrap();
                // Intentional assignment: source keeps only the last match.
                changed = drop_dummies(arena, node, dummy, forwards, false)?;
            }
        }
        if !changed {
            return Ok(());
        }
    }
}

fn next_dummy(arena: &LayeredArena, start: LNodeId, forwards: bool) -> Option<LNodeId> {
    let edges = if forwards {
        arena.node_outgoing_edges(start)
    } else {
        arena.node_incoming_edges(start)
    };
    for edge in edges {
        let e = &arena.edges[edge as usize];
        let other = if e.source == start {
            e.target
        } else {
            e.source
        };
        if arena.nodes[other as usize].node_type == LayeredNodeType::LongEdge
            && arena.nodes[other as usize].layer != arena.nodes[start as usize].layer
        {
            return Some(other);
        }
    }
    None
}

fn in_layer_dummy(arena: &LayeredArena, node: LNodeId) -> bool {
    arena.nodes[node as usize].node_type == LayeredNodeType::LongEdge
        && arena
            .node_connected_edges_source_order(node)
            .iter()
            .any(|(_, e)| {
                let other = if e.source == node { e.target } else { e.source };
                other != node
                    && arena.nodes[node as usize].layer.is_some()
                    && arena.nodes[node as usize].layer == arena.nodes[other as usize].layer
            })
}

fn adjacent(arena: &LayeredArena, first: LNodeId, second: LNodeId, forwards: bool) -> bool {
    let (start, end) = if forwards {
        (second, first)
    } else {
        (first, second)
    };
    let layer = arena.nodes[first as usize].layer.unwrap();
    for i in
        arena.nodes[start as usize].traversal_index + 1..arena.nodes[end as usize].traversal_index
    {
        let n = arena.layers[layer as usize].nodes[i];
        if arena.nodes[n as usize].node_type != LayeredNodeType::BreakingPoint
            && !in_layer_dummy(arena, n)
        {
            return false;
        }
    }
    true
}

fn update_indexes_after(arena: &mut LayeredArena, node: LNodeId) {
    let layer = arena.nodes[node as usize].layer.unwrap();
    for &n in &arena.layers[layer as usize].nodes[arena.nodes[node as usize].traversal_index + 1..]
    {
        arena.nodes[n as usize].traversal_index -= 1;
    }
}

fn drop_dummies(
    arena: &mut LayeredArena,
    breaking_point: LNodeId,
    dummy: LNodeId,
    forwards: bool,
    force: bool,
) -> Result<bool, LayeringError> {
    let mut first = next_dummy(arena, breaking_point, forwards);
    let mut second = next_dummy(arena, dummy, forwards);
    let mut changed = false;
    while let (Some(f), Some(s)) = (first, second) {
        if !force && !adjacent(arena, f, s, forwards) {
            break;
        }
        let next_first = next_dummy(arena, f, forwards);
        let next_second = next_dummy(arena, s, forwards);
        update_indexes_after(arena, dummy);
        update_indexes_after(arena, breaking_point);
        let layer = arena.nodes[f as usize].layer.unwrap();
        let fi = arena.nodes[f as usize].traversal_index;
        let si = arena.nodes[s as usize].traversal_index;
        join_edge_segments_at_dummy(arena, f)?;
        join_edge_segments_at_dummy(arena, s)?;
        if forwards {
            move_node(arena, dummy, layer, si);
            arena.nodes[dummy as usize].traversal_index = si;
            move_node(arena, breaking_point, layer, fi + 1);
            arena.nodes[breaking_point as usize].traversal_index = fi;
        } else {
            move_node(arena, breaking_point, layer, fi);
            arena.nodes[breaking_point as usize].traversal_index = fi;
            move_node(arena, dummy, layer, si + 1);
            arena.nodes[dummy as usize].traversal_index = si;
        }
        detach_node(arena, f);
        detach_node(arena, s);
        first = next_first;
        second = next_second;
        changed = true;
    }
    Ok(changed)
}

pub(crate) fn remove_breaking_points(arena: &mut LayeredArena, graph: LGraphId) {
    for layer in arena.graphs[graph as usize].layers.clone() {
        for node in arena.layers[layer as usize].nodes.clone() {
            let Some(id) = info_for(arena, node, false) else {
                continue;
            };
            if arena.breaking_points.infos[id].next.is_none() {
                remove_breaking_point(arena, id);
            }
        }
    }
}

fn remove_breaking_point(arena: &mut LayeredArena, id: usize) {
    let info = arena.breaking_points.infos[id].clone();
    let mut bends = arena.edges[info.node_start_edge as usize]
        .bend_points
        .clone();
    bends.extend(
        arena.edges[info.start_end_edge as usize]
            .bend_points
            .iter()
            .rev()
            .copied(),
    );
    bends.extend(
        arena.edges[info.original_edge as usize]
            .bend_points
            .iter()
            .copied(),
    );
    arena.edges[info.original_edge as usize].bend_points = bends;
    let source = arena.edges[info.node_start_edge as usize]
        .source_port
        .unwrap();
    set_edge_source_port(arena, info.original_edge, source);
    let mut junctions = arena.edges[info.original_edge as usize]
        .junction_points
        .clone();
    junctions.extend(
        arena.edges[info.start_end_edge as usize]
            .junction_points
            .iter()
            .copied(),
    );
    junctions.extend(
        arena.edges[info.node_start_edge as usize]
            .junction_points
            .iter()
            .copied(),
    );
    arena.edges[info.original_edge as usize].junction_points = junctions;
    retire_edge(arena, info.start_end_edge);
    retire_edge(arena, info.node_start_edge);
    detach_node(arena, info.end);
    detach_node(arena, info.start);
    if let Some(prev) = info.prev {
        remove_breaking_point(arena, prev);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // wrapping_test.go::TestMultiEdgeInserterCreatesPairedBreakingPoints.
    #[test]
    fn inserter_creates_paired_markers_and_copies_model_order() {
        let (mut arena, root) = super::super::tests::path(
            8,
            json!({
                "elk.direction": "RIGHT",
                "elk.layered.wrapping.strategy": "MULTI_EDGE",
                "elk.layered.wrapping.cutting.strategy": "ARD",
                "elk.layered.wrapping.multiEdge.improveCuts": false
            }),
        );
        for i in 0..arena.edges.len() {
            arena.edge_model_order.insert(i as u32, i + 10);
        }
        insert_breaking_points(&mut arena, root);
        let mut starts = 0;
        let mut ends = 0;
        for &layer in &arena.graphs[root as usize].layers {
            for &node in &arena.layers[layer as usize].nodes {
                if let Some(id) = info_for(&arena, node, true) {
                    starts += 1;
                    let info = &arena.breaking_points.infos[id];
                    for edge in [info.node_start_edge, info.start_end_edge] {
                        assert_eq!(
                            arena.edge_model_order[&edge],
                            arena.edge_model_order[&info.original_edge]
                        );
                    }
                }
                if info_for(&arena, node, false).is_some() {
                    ends += 1;
                }
            }
        }
        assert!(starts > 0);
        assert_eq!(starts, ends);
    }

    // wrapping_test.go::TestMultiEdgeManualWithoutCutsDoesNotFallThroughToARD.
    #[test]
    fn manual_without_cuts_does_not_fall_through_to_ard() {
        let (mut arena, root) = super::super::tests::path(
            8,
            json!({
                "elk.layered.wrapping.strategy": "MULTI_EDGE",
                "elk.layered.wrapping.cutting.strategy": "MANUAL"
            }),
        );
        let before = arena.clone();
        insert_breaking_points(&mut arena, root);
        assert_eq!(arena, before);
    }

    #[test]
    fn multi_cut_long_edge_restores_original_identity_with_both_improvements() {
        for improve in [false, true] {
            let (mut arena, root) = super::super::tests::path(
                9,
                json!({
                    "elk.direction": "RIGHT",
                    "elk.layered.wrapping.strategy": "MULTI_EDGE",
                    "elk.layered.wrapping.cutting.strategy": "MANUAL",
                    "elk.layered.wrapping.cutting.cuts": [3, 6],
                    "elk.layered.wrapping.multiEdge.improveCuts": false,
                    "elk.layered.wrapping.multiEdge.improveWrappedEdges": improve
                }),
            );
            let source = arena.add_dummy_port(0, PortSide::East);
            let target = arena.add_dummy_port(8, PortSide::West);
            arena.add_edge(
                root,
                EndpointRef {
                    node: 0,
                    port: Some(source),
                },
                EndpointRef {
                    node: 8,
                    port: Some(target),
                },
            );
            let original = arena.edges.clone();
            insert_breaking_points(&mut arena, root);
            split_long_edges(&mut arena, root).unwrap();
            process_breaking_points(&mut arena, root).unwrap();
            assert!(arena.graphs[root as usize].layers.len() < 9);
            join_long_edges(&mut arena, root).unwrap();
            remove_breaking_points(&mut arena, root);
            restore_reversed_edges_for_layout(&mut arena, root);
            assert_eq!(
                arena.edges.iter().filter(|e| e.active).count(),
                original.len(),
                "improve={improve}; active={:?}",
                arena
                    .edges
                    .iter()
                    .enumerate()
                    .filter(|(_, e)| e.active)
                    .map(|(i, e)| (i, e.source, e.target))
                    .collect::<Vec<_>>()
            );
            for (i, expected) in original.iter().enumerate() {
                let edge = &arena.edges[i];
                assert!(edge.active);
                assert_eq!(
                    (edge.source, edge.target, edge.source_port, edge.target_port),
                    (
                        expected.source,
                        expected.target,
                        expected.source_port,
                        expected.target_port
                    ),
                    "{improve}/{i}"
                );
                assert!(!edge.reversed);
            }
            for &layer in &arena.graphs[root as usize].layers {
                assert!(
                    arena.layers[layer as usize]
                        .nodes
                        .iter()
                        .all(|&n| arena.nodes[n as usize].node_type == LayeredNodeType::Normal)
                );
            }
        }
    }
}
