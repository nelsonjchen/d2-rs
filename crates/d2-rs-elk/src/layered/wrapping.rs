// Copyright (c) 2016 Kiel University and others.
// Copyright (c) 2026 The elk-go Authors.
// SPDX-License-Identifier: EPL-2.0
// Port of intermediate/wrapping/{stats,cuts,single_edge,cutting}.go,
// elk-go 8b285820520e (ELK 0.8.1).
use super::*;
pub(crate) mod multi_edge;
pub(crate) use multi_edge::{
    insert_breaking_points, process_breaking_points, remove_breaking_points,
};

pub(crate) fn is_multi_edge(arena: &LayeredArena, graph: LGraphId) -> bool {
    arena.graphs[graph as usize].wrapping.strategy == "MULTI_EDGE"
}

pub(crate) fn before_placement(
    arena: &mut LayeredArena,
    graph: LGraphId,
) -> Result<(), LayeringError> {
    process_breaking_points(arena, graph)?;
    wrap_single_edge(arena, graph)
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Options {
    strategy: String,
    cutting: String,
    cuts: Vec<i32>,
    freedom: i32,
    validify: Option<String>,
    forbidden: Option<Vec<i32>>,
    correction: f64,
    additional_spacing: f64,
    improve_cuts: bool,
    distance_penalty: f64,
    improve_wrapped: bool,
}

impl Options {
    pub fn parse(options: &Value) -> Self {
        let property =
            |key: &str| model_order_barycenter::property(options, &format!("wrapping.{key}"));
        let boolean = |key: &str| {
            property(key)
                .and_then(|v| v.as_bool().or_else(|| v.as_str()?.parse().ok()))
                .unwrap_or(true)
        };
        let string = |key: &str, default: &str| {
            property(key)
                .and_then(Value::as_str)
                .unwrap_or(default)
                .to_ascii_uppercase()
        };
        let indexes = |key: &str| {
            property(key).and_then(Value::as_array).map(|a| {
                a.iter()
                    .filter_map(Value::as_i64)
                    .map(|v| v as i32)
                    .collect()
            })
        };
        Self {
            improve_cuts: boolean("multiEdge.improveCuts"),
            distance_penalty: property("multiEdge.distancePenalty")
                .and_then(parse_option_double)
                .unwrap_or(2.),
            improve_wrapped: boolean("multiEdge.improveWrappedEdges"),
            strategy: string("strategy", "OFF"),
            cutting: string("cutting.strategy", "MSD"),
            cuts: indexes("cutting.cuts").unwrap_or_default(),
            freedom: property("cutting.msd.freedom")
                .and_then(|v| v.as_i64().or_else(|| v.as_str()?.parse().ok()))
                .unwrap_or(1) as i32,
            validify: property("validify.strategy")
                .and_then(Value::as_str)
                .map(str::to_ascii_uppercase),
            forbidden: indexes("validify.forbiddenIndices"),
            correction: property("correctionFactor")
                .and_then(parse_option_double)
                .unwrap_or(1.),
            additional_spacing: property("additionalEdgeSpacing")
                .and_then(parse_option_double)
                .unwrap_or(10.),
        }
    }
}

struct GraphStats {
    desired: f64,
    widths: Vec<f64>,
    heights: Vec<f64>,
    allowed: Vec<bool>,
}

impl GraphStats {
    fn new(arena: &LayeredArena, graph_id: LGraphId) -> Self {
        let graph = &arena.graphs[graph_id as usize];
        let mut desired = graph.aspect_ratio * graph.wrapping.correction;
        if matches!(graph.direction, LayerDirection::Up | LayerDirection::Down) {
            desired = 1. / desired;
        }
        let mut widths = Vec::new();
        let mut heights = Vec::new();
        let mut allowed = vec![false; graph.layers.len()];
        for (index, &layer) in graph.layers.iter().enumerate() {
            let mut width: f64 = 0.;
            let mut height = 0.;
            let mut first_target = None;
            let mut first_source = None;
            let mut cut_allowed = true;
            for &node_id in &arena.layers[layer as usize].nodes {
                let node = &arena.nodes[node_id as usize];
                width = width
                    .max(node.width + margin_left(node) + margin_right(node) + graph.layer_spacing);
                height +=
                    node.height + margin_top(node) + margin_bottom(node) + graph.node_node_spacing;
                for edge in arena.node_incoming_edges(node_id) {
                    let source = arena.edges[edge as usize].source;
                    if first_target.is_some_and(|v| v != node_id)
                        || first_source.is_some_and(|v| v != source)
                    {
                        cut_allowed = false;
                    }
                    first_target = Some(node_id);
                    first_source = Some(source);
                    if arena.nodes[source as usize].node_type == LayeredNodeType::NorthSouthPort {
                        if let Some(dummy) = arena
                            .north_south_port_dummies
                            .iter()
                            .find(|d| d.node == source)
                        {
                            let original =
                                &arena.nodes[arena.ports[dummy.origin_port as usize].node as usize];
                            height +=
                                original.height + margin_top(original) + margin_bottom(original);
                        }
                    }
                }
            }
            widths.push(width);
            heights.push(height);
            // Preserve ELK's explicit-forbidden-indices branch literally: no
            // entries are initialized true, even if the supplied list is empty.
            allowed[index] = index > 0 && graph.wrapping.forbidden.is_none() && cut_allowed;
        }
        Self {
            desired,
            widths,
            heights,
            allowed,
        }
    }
    fn max_width(&self) -> f64 {
        self.widths
            .iter()
            .copied()
            .fold(f64::NEG_INFINITY, f64::max)
    }
    fn max_height(&self) -> f64 {
        self.heights
            .iter()
            .copied()
            .fold(f64::NEG_INFINITY, f64::max)
    }
    fn chunks(&self) -> i32 {
        ((self.widths.iter().sum::<f64>() / (self.desired * self.max_height()))
            .sqrt()
            .round() as i32)
            .min(self.widths.len() as i32)
    }
    fn cuts(&self, options: &Options) -> Vec<i32> {
        if options.cutting == "MANUAL" {
            return options.cuts.clone();
        }
        if options.cutting == "ARD" {
            let rows = self.chunks();
            let step = self.widths.len() as f64 / rows as f64;
            return (1..rows)
                .map(|i| (i as f64 * step).round() as i32)
                .collect();
        }
        if self.widths.is_empty() {
            return Vec::new();
        }
        let mut width_at = vec![self.widths[0]; self.widths.len()];
        let mut total = self.widths[0];
        for i in 1..self.widths.len() {
            width_at[i] = width_at[i - 1] + self.widths[i];
            total += self.widths[i];
        }
        let count = self.chunks() - 1;
        let mut best_scale = f64::NEG_INFINITY;
        let mut best = Vec::new();
        for count in (count - options.freedom).max(0)
            ..=(count + options.freedom).min(self.widths.len() as i32 - 1)
        {
            let row_sum = total / (count + 1) as f64;
            let mut sum = 0.;
            let mut cuts = Vec::new();
            let mut width = f64::NEG_INFINITY;
            let mut last_width = 0.;
            let mut height = 0.;
            let mut row_height = self.heights[0];
            if count == 0 {
                width = total;
                height = self.max_height();
            } else {
                for i in 1..self.widths.len() {
                    if width_at[i - 1] - sum >= row_sum {
                        cuts.push(i as i32);
                        width = width.max(width_at[i - 1] - last_width);
                        height += row_height;
                        sum += width_at[i - 1] - sum;
                        last_width = width_at[i - 1];
                        row_height = self.heights[i];
                    }
                    row_height = row_height.max(self.heights[i]);
                }
                height += row_height;
            }
            let scale = (1. / width).min((1. / self.desired) / height);
            if scale > best_scale {
                best_scale = scale;
                best = cuts;
            }
        }
        best
    }
    fn validify(&self, desired: Vec<i32>, strategy: &str) -> Vec<i32> {
        if strategy == "LOOK_BACK" {
            let mut valid = vec![i32::MIN];
            valid.extend(
                (1..self.allowed.len())
                    .filter(|&i| self.allowed[i])
                    .map(|i| i as i32),
            );
            if valid.len() == 1 {
                return Vec::new();
            }
            valid.push(i32::MAX);
            return looking_back(&desired, &valid);
        }
        if strategy != "GREEDY" {
            return desired;
        }
        let mut valid = Vec::new();
        let mut offset = 0;
        for requested in desired {
            let mut cut = requested + offset;
            while cut < self.widths.len() as i32 && !self.allowed[cut as usize] {
                cut += 1;
                offset += 1;
            }
            if cut >= self.widths.len() as i32 {
                break;
            }
            valid.push(cut);
        }
        valid
    }
}

fn looking_back(desired: &[i32], valid: &[i32]) -> Vec<i32> {
    let mut cuts = Vec::new();
    let (mut vi, mut ci, mut offset) = (0, 0, 0i64);
    while vi < valid.len() - 1 && ci < desired.len() {
        let current = desired[ci] as i64 + offset;
        while (valid[vi + 1] as i64) < current {
            vi += 1;
        }
        let selection = usize::from(current - valid[vi] as i64 > valid[vi + 1] as i64 - current);
        let selected = valid[vi + selection];
        cuts.push(selected);
        offset += selected as i64 - current;
        ci += 1;
        while ci < desired.len() && desired[ci] as i64 + offset <= selected as i64 {
            ci += 1;
        }
        vi += 1 + selection;
    }
    cuts
}

fn move_node(arena: &mut LayeredArena, node: LNodeId, layer: LayerId, index: usize) {
    if let Some(old) = arena.nodes[node as usize].layer {
        arena.layers[old as usize].nodes.retain(|&n| n != node);
    }
    let nodes = &mut arena.layers[layer as usize].nodes;
    nodes.insert(index, node);
    arena.nodes[node as usize].layer = Some(layer);
}

fn append_node(arena: &mut LayeredArena, node: LNodeId, layer: LayerId) {
    if let Some(old) = arena.nodes[node as usize].layer {
        arena.layers[old as usize].nodes.retain(|&n| n != node);
    }
    arena.layers[layer as usize].nodes.push(node);
    arena.nodes[node as usize].layer = Some(layer);
}

fn insert_dummies(
    arena: &mut LayeredArena,
    graph_id: LGraphId,
    original: LEdgeId,
    offset: usize,
) -> Result<Vec<LEdgeId>, LayeringError> {
    let layers = arena.graphs[graph_id as usize].layers.clone();
    let source = arena.edges[original as usize].source;
    let target = arena.edges[original as usize].target;
    let source_index = layers
        .iter()
        .position(|&l| Some(l) == arena.nodes[source as usize].layer)
        .unwrap();
    let target_index = layers
        .iter()
        .position(|&l| Some(l) == arena.nodes[target as usize].layer)
        .unwrap();
    let individual = arena.graphs[graph_id as usize].edge_node_vertical_spacing
        + arena.graphs[graph_id as usize].wrapping.additional_spacing;
    let mut edge = original;
    let mut created = Vec::new();
    for (index, &layer) in layers
        .iter()
        .enumerate()
        .take(target_index + 1)
        .skip(source_index)
    {
        if let Some(thickness) = arena.edge_thickness.get_mut(&edge) {
            *thickness = thickness.max(0.);
        }
        let dummy = arena.add_long_edge_dummy(graph_id, layer, edge);
        if index == source_index {
            let insert_at = arena.layers[layer as usize].nodes.len() - 1 - offset;
            move_node(arena, dummy, layer, insert_at);
        }
        arena.wrapping_original_edges.insert(dummy, edge);
        arena.wrapping_edge_node_spacing.insert(dummy, individual);
        let old_source = arena.edges[edge as usize].source;
        let old_endpoints =
            if arena.nodes[old_source as usize].node_type == LayeredNodeType::LongEdge {
                arena.long_edge_endpoints.get(&old_source).copied()
            } else {
                arena.edges[edge as usize]
                    .source_port
                    .zip(arena.edges[edge as usize].target_port)
            };
        let next = arena.split_edge_at_dummy(edge, dummy, target)?;
        // Cutting.setDummyProperties only inherits through LongEdge nodes.
        arena.long_edge_endpoints.remove(&dummy);
        arena.long_edge_label_dummies.remove(&dummy);
        if let Some(ends) = old_endpoints {
            arena.long_edge_endpoints.insert(dummy, ends);
        }
        let y = (arena.nodes[dummy as usize].height / 2.).floor();
        for port in arena.nodes[dummy as usize].ports.clone() {
            arena.ports[port as usize].y = y;
        }
        created.push(next);
        edge = next;
    }
    Ok(created)
}

pub(crate) fn wrap_single_edge(
    arena: &mut LayeredArena,
    graph_id: LGraphId,
) -> Result<(), LayeringError> {
    let graph = &arena.graphs[graph_id as usize];
    if graph.wrapping.strategy != "SINGLE_EDGE" || graph.layers.is_empty() {
        return Ok(());
    }
    let stats = GraphStats::new(arena, graph_id);
    let sum_width = stats.max_width() * stats.widths.len() as f64;
    if stats.desired > sum_width / stats.max_width() {
        return Ok(());
    }
    let cuts = stats.validify(
        stats.cuts(&graph.wrapping),
        graph.wrapping.validify.as_deref().unwrap_or("GREEDY"),
    );
    if cuts.is_empty() {
        return Ok(());
    }
    let layers = graph.layers.clone();
    let (mut new_index, mut cut_index) = (0, 0);
    let mut next_cut = cuts[0];
    for index in 0..layers.len() {
        if index as i32 == next_cut {
            new_index = 0;
            cut_index += 1;
            next_cut = cuts
                .get(cut_index)
                .copied()
                .unwrap_or(layers.len() as i32 + 1);
        }
        if index != new_index {
            let layer = layers[new_index];
            for node in arena.layers[layers[index] as usize].nodes.clone() {
                move_node(arena, node, layer, arena.layers[layer as usize].nodes.len());
                if new_index == 0 {
                    for edge in arena.node_incoming_edges(node) {
                        arena.reverse_edge(edge, true);
                        insert_dummies(arena, graph_id, edge, 1)?;
                    }
                }
            }
        }
        new_index += 1;
    }
    arena.graphs[graph_id as usize]
        .layers
        .retain(|&l| !arena.layers[l as usize].nodes.is_empty());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    pub(super) fn path(count: usize, options: Value) -> (LayeredArena, LGraphId) {
        let graph = serde_json::from_value(json!({
            "id": "root", "layoutOptions": options,
            "children": (0..count).map(|i| json!({"id": i.to_string(), "width": 30., "height": 20.})).collect::<Vec<_>>(),
            "edges": (1..count).map(|i| json!({"id": format!("e{i}"), "sources": [(i-1).to_string()], "targets": [i.to_string()]})).collect::<Vec<_>>()
        })).unwrap();
        let (mut arena, root) = LayeredArena::from_elk_graph(&graph).unwrap();
        materialize_implicit_ports(&mut arena, root).unwrap();
        assign_longest_path_layers(&mut arena, root).unwrap();
        (arena, root)
    }

    // elk-go wrapping_test.go::TestSingleEdgeManualCutWrapsPath.
    #[test]
    fn single_edge_manual_cut_wraps_path_and_restores_edges() {
        let (mut arena, root) = path(
            6,
            json!({
                "elk.direction": "RIGHT",
                "elk.layered.wrapping.strategy": "SINGLE_EDGE",
                "elk.layered.wrapping.cutting.strategy": "MANUAL",
                "elk.layered.wrapping.cutting.cuts": [3],
                "elk.layered.wrapping.validify.strategy": "GREEDY"
            }),
        );
        wrap_single_edge(&mut arena, root).unwrap();
        assert_eq!(arena.graphs[root as usize].layers.len(), 3);
        for (i, &layer) in arena.graphs[root as usize].layers.iter().enumerate() {
            let normals = arena.layers[layer as usize]
                .nodes
                .iter()
                .copied()
                .filter(|&n| arena.nodes[n as usize].node_type == LayeredNodeType::Normal)
                .collect::<Vec<_>>();
            assert_eq!(normals, vec![i as u32, i as u32 + 3]);
        }
        assert_eq!(arena.wrapping_original_edges.len(), 3);
        for (&dummy, &spacing) in &arena.wrapping_edge_node_spacing {
            assert_eq!(spacing, 20.);
            assert_eq!(
                arena.nodes[dummy as usize].port_constraints,
                PortConstraints::FixedPosition
            );
            assert_eq!(
                arena.long_edge_endpoints[&dummy].0,
                arena.edges[2].source_port.unwrap()
            );
            assert_eq!(bk_vertical_spacing(&arena, dummy, 0, 20.), 20.);
        }
        join_long_edges(&mut arena, root).unwrap();
        restore_reversed_edges_for_layout(&mut arena, root);
        for i in 0..5 {
            assert!(arena.edges[i].active);
            assert_eq!(
                (arena.edges[i].source, arena.edges[i].target),
                (i as u32, i as u32 + 1)
            );
            assert!(!arena.edges[i].reversed);
        }
        assert_eq!(arena.edges.iter().filter(|e| e.active).count(), 5);
    }

    #[test]
    fn cut_validation_preserves_upstream_offsets_and_forbidden_branch() {
        let stats = GraphStats {
            desired: 1.6,
            widths: vec![50.; 10],
            heights: vec![40.; 10],
            allowed: vec![
                false, true, false, false, true, false, true, true, false, true,
            ],
        };
        assert_eq!(stats.validify(vec![2, 5, 7], "GREEDY"), vec![4, 7, 9]);
        assert_eq!(stats.validify(vec![3, 6], "LOOK_BACK"), vec![4, 7]);
        let (arena, root) = path(
            6,
            json!({"elk.layered.wrapping.validify.forbiddenIndices": []}),
        );
        assert_eq!(GraphStats::new(&arena, root).allowed, vec![false; 6]);
    }

    #[test]
    fn statistics_include_one_spacing_per_layer_and_node() {
        let (arena, root) = path(8, json!({"elk.direction": "RIGHT"}));
        let stats = GraphStats::new(&arena, root);
        assert_eq!(stats.widths, vec![50.; 8]);
        assert_eq!(stats.heights, vec![40.; 8]);
        let mut options = Options::parse(&Value::Null);
        options.cutting = "ARD".into();
        // cuts.go: round(sqrt(400 / (1.6 * 40))) = 3 rows.
        assert_eq!(stats.cuts(&options), vec![3, 5]);
        options.cutting = "MANUAL".into();
        assert!(stats.cuts(&options).is_empty());
    }
}
