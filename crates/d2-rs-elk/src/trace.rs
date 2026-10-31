// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Opt-in, deterministic layout diagnostics.
//!
//! The module is feature-gated so normal and release builds do not construct
//! trace arguments or retain event buffers. Floating-point values are stored
//! as IEEE-754 bits; this avoids silently turning a one-ULP divergence into a
//! tolerance match.

use crate::layered::{BkAlignment, LayeredArena};
use crate::model::ElkGraph;
use serde::Serialize;
use std::cell::RefCell;
use std::collections::BTreeMap;

thread_local! {
    static ACTIVE: RefCell<Option<TraceRecorder>> = const { RefCell::new(None) };
}

/// One normalized trace event.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct TraceEvent {
    /// Monotonic event number within one layout invocation.
    pub sequence: u64,
    /// Stable pipeline stage name.
    pub phase: String,
    /// Stable operation name.
    pub operation: String,
    /// Entity/property path for the event.
    pub path: String,
    /// Exact floating-point representation, when the event carries a number.
    pub float_bits: Option<u64>,
    /// Stable textual value, when the event is not numeric.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// Stable random-stream name for RNG operation events.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<String>,
    /// Cumulative primitive draws consumed by the stream after this operation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub draw_count: Option<u64>,
}

/// Recorder used only by diagnostic builds.
#[derive(Default)]
pub struct TraceRecorder {
    next_sequence: u64,
    events: Vec<TraceEvent>,
}

impl TraceRecorder {
    /// Creates an empty recorder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a textual phase marker.
    pub fn marker(&mut self, phase: &str, operation: &str, path: &str, value: &str) {
        self.events.push(TraceEvent {
            sequence: self.next_sequence,
            phase: phase.to_owned(),
            operation: operation.to_owned(),
            path: path.to_owned(),
            float_bits: None,
            value: Some(value.to_owned()),
            stream: None,
            draw_count: None,
        });
        self.next_sequence += 1;
    }

    /// Records an exact floating-point value.
    pub fn float(&mut self, phase: &str, operation: &str, path: &str, value: f64) {
        self.events.push(TraceEvent {
            sequence: self.next_sequence,
            phase: phase.to_owned(),
            operation: operation.to_owned(),
            path: path.to_owned(),
            float_bits: Some(value.to_bits()),
            value: None,
            stream: None,
            draw_count: None,
        });
        self.next_sequence += 1;
    }

    /// Records all currently published wire geometry in stable order.
    pub fn graph(&mut self, phase: &str, operation: &str, graph: &ElkGraph) {
        self.float(phase, operation, "graph.width", graph.width.unwrap_or(0.0));
        self.float(
            phase,
            operation,
            "graph.height",
            graph.height.unwrap_or(0.0),
        );
        for (index, node) in graph.children.iter().enumerate() {
            self.node(
                phase,
                operation,
                &format!("children[{index}]({})", node.id),
                node,
            );
        }
        self.edges(phase, operation, "", &graph.edges);
    }

    fn edges(
        &mut self,
        phase: &str,
        operation: &str,
        prefix: &str,
        edges: &[crate::model::ElkEdge],
    ) {
        for (index, edge) in edges.iter().enumerate() {
            let edge_prefix = if prefix.is_empty() {
                format!("edges[{index}]({})", edge.id)
            } else {
                format!("{prefix}/edges[{index}]({})", edge.id)
            };
            for (section_index, section) in edge.sections.iter().enumerate() {
                let prefix = format!("{edge_prefix}/sections[{section_index}]");
                self.float(
                    phase,
                    operation,
                    &format!("{prefix}.start.x"),
                    section.start.x,
                );
                self.float(
                    phase,
                    operation,
                    &format!("{prefix}.start.y"),
                    section.start.y,
                );
                for (bend_index, bend) in section.bends.iter().enumerate() {
                    self.float(
                        phase,
                        operation,
                        &format!("{prefix}.bends[{bend_index}].x"),
                        bend.x,
                    );
                    self.float(
                        phase,
                        operation,
                        &format!("{prefix}.bends[{bend_index}].y"),
                        bend.y,
                    );
                }
                self.float(phase, operation, &format!("{prefix}.end.x"), section.end.x);
                self.float(phase, operation, &format!("{prefix}.end.y"), section.end.y);
            }
            for (index, point) in edge.junction_points.iter().enumerate() {
                self.float(
                    phase,
                    operation,
                    &format!("{edge_prefix}/junctions[{index}].x"),
                    point.x,
                );
                self.float(
                    phase,
                    operation,
                    &format!("{edge_prefix}/junctions[{index}].y"),
                    point.y,
                );
            }
            for (label_index, label) in edge.labels.iter().enumerate() {
                let prefix = format!("{edge_prefix}/labels[{label_index}]");
                self.marker(phase, operation, &format!("{prefix}.text"), &label.text);
                self.float(phase, operation, &format!("{prefix}.x"), label.x);
                self.float(phase, operation, &format!("{prefix}.y"), label.y);
                self.float(phase, operation, &format!("{prefix}.width"), label.width);
                self.float(phase, operation, &format!("{prefix}.height"), label.height);
            }
        }
    }

    fn node(&mut self, phase: &str, operation: &str, prefix: &str, node: &crate::model::ElkNode) {
        for (name, value) in [
            ("x", node.x),
            ("y", node.y),
            ("width", node.width),
            ("height", node.height),
        ] {
            self.float(phase, operation, &format!("{prefix}.{name}"), value);
        }
        for (index, port) in node.ports.iter().enumerate() {
            let port_prefix = format!("{prefix}/ports[{index}]({})", port.id);
            for (name, value) in [
                ("x", port.x),
                ("y", port.y),
                ("width", port.width),
                ("height", port.height),
            ] {
                self.float(phase, operation, &format!("{port_prefix}.{name}"), value);
            }
        }
        for (index, label) in node.labels.iter().enumerate() {
            let label_prefix = format!("{prefix}/labels[{index}]");
            self.marker(
                phase,
                operation,
                &format!("{label_prefix}.text"),
                &label.text,
            );
            self.float(phase, operation, &format!("{label_prefix}.x"), label.x);
            self.float(phase, operation, &format!("{label_prefix}.y"), label.y);
            self.float(
                phase,
                operation,
                &format!("{label_prefix}.width"),
                label.width,
            );
            self.float(
                phase,
                operation,
                &format!("{label_prefix}.height"),
                label.height,
            );
        }
        for (index, child) in node.children.iter().enumerate() {
            self.node(
                phase,
                operation,
                &format!("{prefix}/children[{index}]({})", child.id),
                child,
            );
        }
        self.edges(phase, operation, prefix, &node.edges);
    }

    /// Finishes recording and returns events in sequence order.
    pub fn finish(self) -> Vec<TraceEvent> {
        self.events
    }
}

impl TraceRecorder {
    /// Records one public RNG operation with its exact result and cumulative
    /// primitive draw count. The stream name is kept separate from `path` so
    /// consumers can compare operation paths without parsing labels.
    pub fn rng(
        &mut self,
        stream: &str,
        operation: &str,
        draw_count: u64,
        float_bits: Option<u64>,
        value: Option<&str>,
    ) {
        self.events.push(TraceEvent {
            sequence: self.next_sequence,
            phase: "rng".to_owned(),
            operation: operation.to_owned(),
            path: stream.to_owned(),
            float_bits,
            value: value.map(str::to_owned),
            stream: Some(stream.to_owned()),
            draw_count: Some(draw_count),
        });
        self.next_sequence += 1;
    }

    /// Records the layer membership used by one graph-scoped ordering pass.
    pub fn graph_scope(
        &mut self,
        phase: &str,
        operation: &str,
        graph_id: u32,
        arena: &LayeredArena,
    ) {
        let Some(graph) = arena.graphs.get(graph_id as usize) else {
            return;
        };
        let parent = graph
            .parent_node
            .and_then(|node_id| arena.nodes.get(node_id as usize))
            .map_or_else(|| "root".to_owned(), |node| node.source_id.clone());
        let layers = graph
            .layers
            .iter()
            .filter_map(|layer_id| arena.layers.get(*layer_id as usize))
            .map(|layer| {
                let nodes = layer
                    .nodes
                    .iter()
                    .filter_map(|node_id| arena.nodes.get(*node_id as usize))
                    .map(|node| node.source_id.as_str())
                    .collect::<Vec<_>>()
                    .join(",");
                format!("[{nodes}]")
            })
            .collect::<Vec<_>>()
            .join(";");
        let port_orders = graph
            .layers
            .iter()
            .filter_map(|layer_id| arena.layers.get(*layer_id as usize))
            .flat_map(|layer| layer.nodes.iter())
            .filter_map(|node_id| arena.nodes.get(*node_id as usize))
            .map(|node| {
                let ports = node
                    .ports
                    .iter()
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join(",");
                format!("{}:[{ports}]", node.source_id)
            })
            .collect::<Vec<_>>()
            .join(";");
        let p3 = crate::layered::p3_sweep_decision(arena, graph_id);
        let root_id = arena
            .graphs
            .iter()
            .position(|candidate| candidate.parent_node.is_none())
            .map(|index| index as u32);
        let plan_fields = if root_id == Some(graph_id) {
            let plan = crate::layered::p3_hierarchy_plan(arena, graph_id);
            let hierarchy_index = plan
                .breadth_first_graphs
                .iter()
                .position(|candidate| *candidate == graph_id)
                .unwrap_or(usize::MAX);
            let sweep_index = plan
                .graphs_to_sweep_on
                .iter()
                .position(|candidate| *candidate == graph_id)
                .map_or_else(|| "recursive".to_owned(), |index| index.to_string());
            let bfs_graphs = plan
                .breadth_first_graphs
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(",");
            let sweep_queue = plan
                .graphs_to_sweep_on
                .iter()
                .filter_map(|swept| {
                    plan.breadth_first_graphs
                        .iter()
                        .position(|candidate| candidate == swept)
                })
                .map(|index| index.to_string())
                .collect::<Vec<_>>()
                .join(",");
            let eligibility = plan
                .breadth_first_graphs
                .iter()
                .enumerate()
                .map(|(index, graph_id)| {
                    format!("{index}:{}", plan.decisions[graph_id].use_bottom_up)
                })
                .collect::<Vec<_>>()
                .join(",");
            format!(
                "hierarchy_index={hierarchy_index};sweep_index={sweep_index};bfs_graphs={bfs_graphs};sweep_queue={sweep_queue};eligibility={eligibility};"
            )
        } else {
            String::new()
        };
        self.marker(
            phase,
            operation,
            &format!("graphs[{graph_id}]"),
            &format!(
                "parent={parent};layers={layers};port_orders={port_orders};{plan_fields}dont_sweep_into={};reason={};paths_to_random={};paths_to_hierarchical={};threshold_bits={:016x}",
                p3.use_bottom_up,
                p3.reason,
                p3.paths_to_random,
                p3.paths_to_hierarchical,
                p3.threshold.to_bits(),
            ),
        );
    }

    /// Records one proposed barycenter candidate before the stable sort.
    pub fn candidate(
        &mut self,
        layer_id: u32,
        node_id: u32,
        barycenter: f64,
        model_position: usize,
        arena: &LayeredArena,
    ) {
        let node_path = arena
            .nodes
            .get(node_id as usize)
            .map(|node| node.source_id.as_str())
            .unwrap_or("unknown-node");
        self.events.push(TraceEvent {
            sequence: self.next_sequence,
            phase: "ordering".to_owned(),
            operation: "candidate".to_owned(),
            path: format!("layer[{layer_id}].nodes[{node_path}]"),
            float_bits: Some(barycenter.to_bits()),
            value: Some(format!("model_position={model_position}")),
            stream: None,
            draw_count: None,
        });
        self.next_sequence += 1;
    }

    /// Records the exact fixed-layer port rank used by ELK's barycenter sum.
    pub fn port_rank(
        &mut self,
        free_node: u32,
        fixed_port: u32,
        rank: f32,
        node_relative: bool,
        forward: bool,
        arena: &LayeredArena,
    ) {
        let free_name = arena
            .nodes
            .get(free_node as usize)
            .map(|node| node.source_id.as_str())
            .unwrap_or("unknown-node");
        let fixed_port_id = fixed_port;
        let fixed_port = arena.ports.get(fixed_port_id as usize);
        let fixed_name = fixed_port
            .and_then(|port| arena.nodes.get(port.node as usize))
            .map(|node| node.source_id.as_str())
            .unwrap_or("unknown-node");
        let fixed_node_id = fixed_port.map_or(u32::MAX, |port| port.node);
        let fixed_port_source = fixed_port
            .and_then(|port| port.source_id.as_deref())
            .unwrap_or("synthetic");
        let fixed_port_order = fixed_port
            .and_then(|port| arena.nodes.get(port.node as usize))
            .map(|node| {
                node.ports
                    .iter()
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join(",")
            })
            .unwrap_or_default();
        self.events.push(TraceEvent {
            sequence: self.next_sequence,
            phase: "ordering".to_owned(),
            operation: "port_rank".to_owned(),
            path: format!("node[{free_name}].fixed_port[{fixed_name}]"),
            float_bits: Some(u64::from(rank.to_bits())),
            value: Some(format!(
                "node_relative={node_relative};forward={forward};free_node_id={free_node};fixed_node_id={fixed_node_id};fixed_port_id={fixed_port_id};fixed_port_source={fixed_port_source};fixed_port_order={fixed_port_order}"
            )),
            stream: None,
            draw_count: None,
        });
        self.next_sequence += 1;
    }

    /// Records a node's position after the candidate sort.
    pub fn ordered(&mut self, layer_id: u32, node_id: u32, position: usize, arena: &LayeredArena) {
        let node_path = arena
            .nodes
            .get(node_id as usize)
            .map(|node| node.source_id.as_str())
            .unwrap_or("unknown-node");
        self.marker(
            "ordering",
            "ordered",
            &format!("layer[{layer_id}][{position}]"),
            node_path,
        );
    }

    /// Records one BK directional candidate and its validity/score boundary.
    pub fn bk_candidate(
        &mut self,
        candidate_index: usize,
        direction: &str,
        layout_size: f64,
        valid: bool,
    ) {
        self.events.push(TraceEvent {
            sequence: self.next_sequence,
            phase: "placement".to_owned(),
            operation: "bk_candidate".to_owned(),
            path: format!("candidate[{candidate_index}]"),
            float_bits: Some(layout_size.to_bits()),
            value: Some(format!("direction={direction};valid={valid}")),
            stream: None,
            draw_count: None,
        });
        self.next_sequence += 1;
    }

    /// Records every node coordinate proposed by one directional BK pass.
    pub fn bk_candidate_positions(
        &mut self,
        candidate_index: usize,
        positions: &BTreeMap<u32, f64>,
        alignment: &BkAlignment,
        arena: &LayeredArena,
    ) {
        for (&node_id, &position) in positions {
            let Some(node) = arena.nodes.get(node_id as usize) else {
                continue;
            };
            let stable_id = arena.external_port_origins.get(&node_id).map_or_else(
                || node.source_id.clone(),
                |port_id| format!("lport:{port_id}"),
            );
            let path = format!("candidate[{candidate_index}].nodes[{stable_id}]");
            if let Some(layer_id) = node.layer
                && let Some(graph) = arena.graphs.get(node.graph as usize)
            {
                let layer_order = graph.layers.iter().position(|&id| id == layer_id);
                let node_order = arena
                    .layers
                    .get(layer_id as usize)
                    .and_then(|layer| layer.nodes.iter().position(|&id| id == node_id));
                self.marker(
                    "placement",
                    "bk_candidate_layer_order",
                    &path,
                    &format!("{layer_order:?}:{node_order:?}"),
                );
            }
            self.float(
                "placement",
                "bk_candidate_position",
                &format!("{path}.y"),
                position,
            );
            if let Some(&root_id) = alignment.roots.get(&node_id)
                && let Some(root) = arena.nodes.get(root_id as usize)
            {
                self.marker(
                    "placement",
                    "bk_candidate_alignment",
                    &format!("{path}.root"),
                    &root.source_id,
                );
            }
            if let Some(&align_id) = alignment.aligns.get(&node_id)
                && let Some(align) = arena.nodes.get(align_id as usize)
            {
                self.marker(
                    "placement",
                    "bk_candidate_alignment",
                    &format!("{path}.align"),
                    &align.source_id,
                );
            }
            if let Some(&inner_shift) = alignment.inner_shifts.get(&node_id) {
                self.float("placement", "bk_candidate_inner_shift", &path, inner_shift);
            }
            if let Some(&block_size) = alignment.block_sizes.get(&node_id) {
                self.float("placement", "bk_candidate_block_size", &path, block_size);
            }
        }
    }

    /// Records the selected edge and endpoint ports used by BK inner shifting.
    #[allow(clippy::too_many_arguments)]
    pub fn bk_inner_shift(
        &mut self,
        current: u32,
        next: u32,
        edge_id: u32,
        edge: &crate::layered::LEdge,
        horizontal: crate::layered::BkHorizontalDirection,
        delta: f64,
        arena: &LayeredArena,
    ) {
        let current_name = arena.nodes[current as usize].source_id.as_str();
        let next_name = arena.nodes[next as usize].source_id.as_str();
        let source_name = arena.nodes[edge.source as usize].source_id.as_str();
        let target_name = arena.nodes[edge.target as usize].source_id.as_str();
        let port_name = |port_id: Option<u32>| {
            port_id.map_or_else(
                || "none".to_owned(),
                |port_id| {
                    arena.ports[port_id as usize]
                        .source_id
                        .as_deref()
                        .map_or_else(|| format!("lport:{port_id}"), str::to_owned)
                },
            )
        };
        let source_port_owner = edge
            .source_port
            .map(|id| {
                arena.nodes[arena.ports[id as usize].node as usize]
                    .source_id
                    .as_str()
            })
            .unwrap_or("none");
        let target_port_owner = edge
            .target_port
            .map(|id| {
                arena.nodes[arena.ports[id as usize].node as usize]
                    .source_id
                    .as_str()
            })
            .unwrap_or("none");
        let center_bits = |port_id: Option<u32>| {
            port_id.map_or_else(
                || "none".to_owned(),
                |id| {
                    let port = &arena.ports[id as usize];
                    format!("{:016x}", (port.y + port.height / 2.0).to_bits())
                },
            )
        };
        self.marker(
            "placement",
            "bk_inner_shift_edge",
            &format!("bk-inner-shift[{current_name}=>{next_name}]"),
            &format!(
                "edge=edge:{edge_id};source={source_name};target={target_name};source_port={};source_port_owner={source_port_owner};source_center_y_bits={};target_port={};target_port_owner={target_port_owner};target_center_y_bits={};delta_bits={:016x};horizontal={horizontal:?}",
                port_name(edge.source_port),
                center_bits(edge.source_port),
                port_name(edge.target_port),
                center_bits(edge.target_port),
                delta.to_bits(),
            ),
        );
    }
}

/// Starts the recorder for the current diagnostic layout invocation.
pub fn begin() {
    ACTIVE.with(|active| *active.borrow_mut() = Some(TraceRecorder::new()));
}

/// Emits a phase marker to the active recorder.
pub fn emit_marker(phase: &str, operation: &str) {
    ACTIVE.with(|active| {
        if let Some(recorder) = active.borrow_mut().as_mut() {
            recorder.marker(phase, operation, "pipeline", operation);
        }
    });
}

/// Emits a stable textual diagnostic value to the active recorder.
pub fn emit_value(phase: &str, operation: &str, path: &str, value: &str) {
    ACTIVE.with(|active| {
        if let Some(recorder) = active.borrow_mut().as_mut() {
            recorder.marker(phase, operation, path, value);
        }
    });
}

/// Emits one source-order comparator invocation or result.
pub fn emit_order_comparison(
    scope: &str,
    index: usize,
    first: u32,
    second: u32,
    result: Option<std::cmp::Ordering>,
) {
    ACTIVE.with(|active| {
        if let Some(recorder) = active.borrow_mut().as_mut() {
            let base = format!("sort.{scope}.compare[{index}]");
            match result {
                Some(result) => recorder.marker(
                    "ordering",
                    "sort_by_input_model_order",
                    &format!("{base}.result"),
                    &format!("{result:?}"),
                ),
                None => recorder.marker(
                    "ordering",
                    "sort_by_input_model_order",
                    &format!("{base}.begin"),
                    &format!("first={first};second={second}"),
                ),
            }
        }
    });
}

/// Emits one exact RNG operation to the active recorder.
pub fn emit_rng(
    stream: &str,
    operation: &str,
    draw_count: u64,
    float_bits: Option<u64>,
    value: Option<&str>,
) {
    ACTIVE.with(|active| {
        if let Some(recorder) = active.borrow_mut().as_mut() {
            recorder.rng(stream, operation, draw_count, float_bits, value);
        }
    });
}

/// Emits a graph-scoped ordering boundary when diagnostic tracing is active.
pub fn emit_graph_scope(phase: &str, operation: &str, graph_id: u32, arena: &LayeredArena) {
    ACTIVE.with(|active| {
        if let Some(recorder) = active.borrow_mut().as_mut() {
            recorder.graph_scope(phase, operation, graph_id, arena);
        }
    });
}

/// Emits one proposed ordering candidate to the active recorder.
pub fn emit_candidate(
    layer_id: u32,
    node_id: u32,
    barycenter: f64,
    model_position: usize,
    arena: &LayeredArena,
) {
    ACTIVE.with(|active| {
        if let Some(recorder) = active.borrow_mut().as_mut() {
            recorder.candidate(layer_id, node_id, barycenter, model_position, arena);
        }
    });
}

/// Emits one exact source port-rank event.
pub fn emit_port_rank(
    free_node: u32,
    fixed_port: u32,
    rank: f32,
    node_relative: bool,
    forward: bool,
    arena: &LayeredArena,
) {
    ACTIVE.with(|active| {
        if let Some(recorder) = active.borrow_mut().as_mut() {
            recorder.port_rank(free_node, fixed_port, rank, node_relative, forward, arena);
        }
    });
}

/// Emits one post-sort ordering position to the active recorder.
pub fn emit_ordered(layer_id: u32, node_id: u32, position: usize, arena: &LayeredArena) {
    ACTIVE.with(|active| {
        if let Some(recorder) = active.borrow_mut().as_mut() {
            recorder.ordered(layer_id, node_id, position, arena);
        }
    });
}

/// Emits one BK directional candidate and its score boundary.
pub fn emit_bk_candidate(candidate_index: usize, direction: &str, layout_size: f64, valid: bool) {
    ACTIVE.with(|active| {
        if let Some(recorder) = active.borrow_mut().as_mut() {
            recorder.bk_candidate(candidate_index, direction, layout_size, valid);
        }
    });
}

/// Emits exact node coordinates proposed by one directional BK pass.
pub fn emit_bk_candidate_positions(
    candidate_index: usize,
    positions: &BTreeMap<u32, f64>,
    alignment: &BkAlignment,
    arena: &LayeredArena,
) {
    ACTIVE.with(|active| {
        if let Some(recorder) = active.borrow_mut().as_mut() {
            recorder.bk_candidate_positions(candidate_index, positions, alignment, arena);
        }
    });
}

/// Emits the selected endpoint ports used by one BK inner shift.
pub fn emit_bk_inner_shift(
    current: u32,
    next: u32,
    edge_id: u32,
    edge: &crate::layered::LEdge,
    horizontal: crate::layered::BkHorizontalDirection,
    delta: f64,
    arena: &LayeredArena,
) {
    ACTIVE.with(|active| {
        if let Some(recorder) = active.borrow_mut().as_mut() {
            recorder.bk_inner_shift(current, next, edge_id, edge, horizontal, delta, arena);
        }
    });
}

/// Emits an input/output graph snapshot to the active recorder.
pub fn emit_graph(phase: &str, operation: &str, graph: &ElkGraph) {
    ACTIVE.with(|active| {
        if let Some(recorder) = active.borrow_mut().as_mut() {
            recorder.graph(phase, operation, graph);
        }
    });
}

/// Emits a stable snapshot of the translated intermediate arena.
pub fn emit_arena(phase: &str, operation: &str, arena: &LayeredArena) {
    ACTIVE.with(|active| {
        if let Some(recorder) = active.borrow_mut().as_mut() {
            for (index, graph) in arena.graphs.iter().enumerate() {
                let prefix = format!("arena.graphs[{index}]");
                recorder.float(phase, operation, &format!("{prefix}.width"), graph.width);
                recorder.float(phase, operation, &format!("{prefix}.height"), graph.height);
                recorder.float(
                    phase,
                    operation,
                    &format!("{prefix}.offset_y"),
                    graph.offset_y,
                );
                recorder.marker(
                    phase,
                    operation,
                    &format!("{prefix}.parent_node"),
                    &graph
                        .parent_node
                        .map_or_else(|| "root".to_owned(), |node| node.to_string()),
                );
                if let Some(padding) = graph.padding {
                    for (side, value) in [
                        ("top", padding.top),
                        ("right", padding.right),
                        ("bottom", padding.bottom),
                        ("left", padding.left),
                    ] {
                        recorder.float(
                            phase,
                            operation,
                            &format!("{prefix}.padding.{side}"),
                            value,
                        );
                    }
                }
            }
            for (index, node) in arena.nodes.iter().enumerate() {
                let prefix = format!("arena.nodes[{index}]({})", node.source_id);
                recorder.float(phase, operation, &format!("{prefix}.x"), node.position_x);
                recorder.float(phase, operation, &format!("{prefix}.y"), node.position_y);
                recorder.float(phase, operation, &format!("{prefix}.width"), node.width);
                recorder.float(phase, operation, &format!("{prefix}.height"), node.height);
                for (name, value) in [
                    ("margin.top", node.margin_top),
                    ("margin.right", node.margin_right),
                    ("margin.bottom", node.margin_bottom),
                    ("margin.left", node.margin_left),
                ] {
                    recorder.float(phase, operation, &format!("{prefix}.{name}"), value);
                }
                recorder.marker(
                    phase,
                    operation,
                    &format!("{prefix}.graph"),
                    &node.graph.to_string(),
                );
                recorder.marker(
                    phase,
                    operation,
                    &format!("{prefix}.layer"),
                    &node
                        .layer
                        .map_or_else(|| "none".to_owned(), |layer| layer.to_string()),
                );
                for (port_order, port_id) in node.ports.iter().enumerate() {
                    recorder.marker(
                        phase,
                        operation,
                        &format!("{prefix}.ports[{port_order}]"),
                        &port_id.to_string(),
                    );
                }
            }
            for (axis, transfers) in [
                ("x", &arena.nested_transfer_x),
                ("y", &arena.nested_transfer_y),
            ] {
                for (node, (internal, offset)) in transfers {
                    let prefix = format!("arena.nested_transfer_{axis}[{node}]");
                    recorder.float(phase, operation, &format!("{prefix}.internal"), *internal);
                    recorder.float(phase, operation, &format!("{prefix}.offset"), *offset);
                }
            }
            for (node, component) in &arena.component_transfers {
                let prefix = format!("arena.component_transfers[{node}]");
                recorder.float(
                    phase,
                    operation,
                    &format!("{prefix}.axis_width"),
                    component.axis_width,
                );
                recorder.float(
                    phase,
                    operation,
                    &format!("{prefix}.offset.x"),
                    component.offset.x,
                );
                recorder.float(
                    phase,
                    operation,
                    &format!("{prefix}.offset.y"),
                    component.offset.y,
                );
            }
            for (layer_id, layer) in arena.layers.iter().enumerate() {
                for (position, node_id) in layer.nodes.iter().enumerate() {
                    recorder.marker(
                        phase,
                        operation,
                        &format!("arena.layers[{layer_id}].nodes[{position}]"),
                        &node_id.to_string(),
                    );
                }
            }
            for (index, port) in arena.ports.iter().enumerate() {
                let prefix = format!("arena.ports[{index}]");
                recorder.float(phase, operation, &format!("{prefix}.x"), port.x);
                recorder.float(phase, operation, &format!("{prefix}.y"), port.y);
                recorder.float(phase, operation, &format!("{prefix}.width"), port.width);
                recorder.float(phase, operation, &format!("{prefix}.height"), port.height);
                recorder.marker(
                    phase,
                    operation,
                    &format!("{prefix}.side"),
                    &format!("{:?}", port.side),
                );
                recorder.marker(
                    phase,
                    operation,
                    &format!("{prefix}.node"),
                    &port.node.to_string(),
                );
                recorder.marker(
                    phase,
                    operation,
                    &format!("{prefix}.source_id"),
                    port.source_id.as_deref().unwrap_or("synthetic"),
                );
                if let Some(model_order) = port.model_order {
                    recorder.marker(
                        phase,
                        operation,
                        &format!("{prefix}.model_order"),
                        &model_order.to_string(),
                    );
                }
            }
            for (index, edge) in arena.edges.iter().enumerate() {
                let prefix = format!("arena.edges[{index}]");
                recorder.marker(
                    phase,
                    operation,
                    &format!("{prefix}.source"),
                    &edge.source.to_string(),
                );
                recorder.marker(
                    phase,
                    operation,
                    &format!("{prefix}.target"),
                    &edge.target.to_string(),
                );
                recorder.marker(
                    phase,
                    operation,
                    &format!("{prefix}.source_port"),
                    &edge
                        .source_port
                        .map_or_else(|| "none".to_owned(), |port| port.to_string()),
                );
                recorder.marker(
                    phase,
                    operation,
                    &format!("{prefix}.target_port"),
                    &edge
                        .target_port
                        .map_or_else(|| "none".to_owned(), |port| port.to_string()),
                );
                recorder.marker(
                    phase,
                    operation,
                    &format!("{prefix}.active"),
                    &edge.active.to_string(),
                );
                recorder.marker(
                    phase,
                    operation,
                    &format!("{prefix}.reversed"),
                    &edge.reversed.to_string(),
                );
                for (bend_index, bend) in edge.bend_points.iter().enumerate() {
                    recorder.float(
                        phase,
                        operation,
                        &format!("{prefix}.bends[{bend_index}].x"),
                        bend.x,
                    );
                    recorder.float(
                        phase,
                        operation,
                        &format!("{prefix}.bends[{bend_index}].y"),
                        bend.y,
                    );
                }
                for (junction_index, junction) in edge.junction_points.iter().enumerate() {
                    recorder.float(
                        phase,
                        operation,
                        &format!("{prefix}.junctions[{junction_index}].x"),
                        junction.x,
                    );
                    recorder.float(
                        phase,
                        operation,
                        &format!("{prefix}.junctions[{junction_index}].y"),
                        junction.y,
                    );
                }
            }
        }
    });
}

/// Finishes the active recorder.
pub fn finish() -> Vec<TraceEvent> {
    ACTIVE.with(|active| {
        active
            .borrow_mut()
            .take()
            .map_or_else(Vec::new, TraceRecorder::finish)
    })
}

#[cfg(test)]
mod tests {
    use super::TraceRecorder;

    #[test]
    fn float_events_preserve_ieee_bits_and_sequence() {
        let mut recorder = TraceRecorder::new();
        recorder.float("placement", "candidate", "node.x", -0.0);
        recorder.marker("placement", "accept", "node", "stable");
        let events = recorder.finish();
        assert_eq!(events[0].sequence, 0);
        assert_eq!(events[0].float_bits, Some((-0.0f64).to_bits()));
        assert_eq!(events[1].sequence, 1);
        assert_eq!(events[1].value.as_deref(), Some("stable"));
        assert_eq!(events[1].stream, None);
        assert_eq!(events[1].draw_count, None);
    }

    #[test]
    fn rng_events_preserve_stream_result_and_draw_count() {
        let mut recorder = TraceRecorder::new();
        recorder.rng("ordering", "nextInt", 3, None, Some("7"));
        let events = recorder.finish();
        assert_eq!(events[0].phase, "rng");
        assert_eq!(events[0].path, "ordering");
        assert_eq!(events[0].stream.as_deref(), Some("ordering"));
        assert_eq!(events[0].value.as_deref(), Some("7"));
        assert_eq!(events[0].draw_count, Some(3));
    }
}
