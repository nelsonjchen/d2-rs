// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Raw LoopOffsets controls using production map, routing and padding methods.
//!
//! Inputs contain geometry and arbitrary Go orientation keys; case names are
//! labels only. No source golden values or case-specific outputs live here.

#![cfg(feature = "diagnostic-traces")]

use super::*;
use crate::{Edge, Node};
use serde_json::{Value, json};

struct BackgroundContext;

impl limits::Context for BackgroundContext {
    fn done(&self) -> Option<bool> {
        None
    }

    fn error(&self) -> Option<limits::ContextError> {
        None
    }
}

#[derive(Deserialize)]
struct RawPoint {
    x_bits: String,
    y_bits: String,
}

impl RawPoint {
    fn point(&self) -> Result<Point, String> {
        Ok(Point {
            x: from_bits(&self.x_bits)?,
            y: from_bits(&self.y_bits)?,
        })
    }
}

#[derive(Deserialize)]
struct RawEntry {
    key: i64,
    value_bits: String,
}

#[derive(Deserialize)]
struct PreparedNode {
    id: i64,
    width_bits: String,
    height_bits: String,
    position: Option<RawPoint>,
    loop_offsets: Option<Vec<RawEntry>>,
    #[serde(default)]
    shape: Option<String>,
    #[serde(default)]
    is_3d: bool,
    #[serde(default)]
    is_multiple: bool,
}

#[derive(Deserialize)]
struct PreparedEdge {
    id: i64,
    from: i64,
    to: i64,
    min_width: f64,
    min_height: f64,
    #[serde(default)]
    source_arrowhead: String,
    #[serde(default)]
    target_arrowhead: String,
    #[serde(default)]
    label: Option<Value>,
}

#[derive(Deserialize)]
struct Request {
    family: String,
    case: String,
    node: PreparedNode,
    actions: Vec<String>,
    #[serde(default)]
    peer: Option<PreparedNode>,
    #[serde(default)]
    proposed_peer_positions: Vec<RawPoint>,
    #[serde(default)]
    edges: Vec<PreparedEdge>,
    #[serde(default)]
    padding_work_limit: Option<i64>,
    #[serde(default)]
    w_minus_one_limit: Option<i64>,
    #[serde(default)]
    replace_before_compute_loop_offsets: Option<Vec<RawEntry>>,
    #[serde(default)]
    base_bounds_observed: bool,
}

fn bits(value: f64) -> String {
    format!("0x{:016x}", value.to_bits())
}

fn from_bits(value: &str) -> Result<f64, String> {
    let Some(hex) = value.strip_prefix("0x") else {
        return Err(format!("IEEE-754 value must start with 0x: {value}"));
    };
    if hex.len() != 16 {
        return Err(format!(
            "IEEE-754 value must contain 16 hexadecimal digits: {value}"
        ));
    }
    u64::from_str_radix(hex, 16)
        .map(f64::from_bits)
        .map_err(|error| format!("invalid IEEE-754 bits {value}: {error}"))
}

fn map_from_entries(entries: Option<&[RawEntry]>) -> Result<Option<LoopOffsetsMap>, String> {
    entries
        .map(|entries| {
            let mut map = LoopOffsetsMap::new();
            // Repeated assignment has Go map semantics: the last value wins.
            for entry in entries {
                map.insert(entry.key, from_bits(&entry.value_bits)?);
            }
            Ok(map)
        })
        .transpose()
}

fn point_state(point: Option<Point>) -> Value {
    point.map_or(
        Value::Null,
        |point| json!({"x_bits": bits(point.x), "y_bits": bits(point.y)}),
    )
}

fn raw_state(node: &ArenaNode) -> Value {
    let map = node.loop_offsets.as_ref();
    let entries = map
        .into_iter()
        .flat_map(|map| map.iter())
        .map(|(&key, &value)| json!({"key": key, "value_bits": bits(value)}))
        .collect::<Vec<_>>();
    json!({
        "map": {"nil": map.is_none(), "len": entries.len(), "entries": entries},
        "position": point_state(node.position),
        "width_bits": bits(node.rect.size.width),
        "height_bits": bits(node.rect.size.height),
    })
}

fn observe(node: &ArenaNode, repeats: usize) -> Result<Value, String> {
    let state = raw_state(node);
    for _ in 0..repeats {
        if raw_state(node) != state {
            return Err("repeated raw observation changed node state".into());
        }
    }
    Ok(state)
}

fn observer_repeats(request: &Value) -> Result<usize, String> {
    let selected = match request.get("observer_repeats") {
        None => None,
        Some(value) => match value.as_u64() {
            Some(0) => Some(0),
            Some(2) => Some(2),
            _ => return Err("select scalar observer_repeats 0 or 2 for one probe call".into()),
        },
    };
    let observer = match request.get("observer") {
        None => None,
        Some(Value::Bool(value)) => Some(if *value { 2 } else { 0 }),
        _ => return Err("observer must be a boolean".into()),
    };
    if let (Some(selected), Some(observer)) = (selected, observer)
        && selected != observer
    {
        return Err("observer and observer_repeats select different modes".into());
    }
    Ok(selected.or(observer).unwrap_or(0))
}

fn prepared_pipeline(nodes: &[&PreparedNode], edges: &[PreparedEdge]) -> Result<Pipeline, String> {
    let mut input = Graph::default();
    let mut ids = BTreeMap::new();
    for prepared in nodes {
        let size = Size {
            width: from_bits(&prepared.width_bits)?,
            height: from_bits(&prepared.height_bits)?,
        };
        let mut node = Node::new(prepared.id.to_string(), size);
        node.shape =
            serde_json::from_value(json!(prepared.shape.as_deref().unwrap_or("rectangle")))
                .map_err(|error| format!("unsupported native shape: {error}"))?;
        node.is_3d = prepared.is_3d;
        node.is_multiple = prepared.is_multiple;
        node.content_insets = Insets::uniform(0.0);
        let id = input.add_node(node);
        if ids.insert(prepared.id, id).is_some() {
            return Err(format!("duplicate prepared node ID {}", prepared.id));
        }
        input.tala_id_overrides[id.0 as usize] = Some(prepared.id);
        input.initial_positions[id.0 as usize] = prepared
            .position
            .as_ref()
            .map(RawPoint::point)
            .transpose()?;
        input.tala_node_loop_offsets[id.0 as usize] =
            map_from_entries(prepared.loop_offsets.as_deref())?;
    }
    for prepared in edges {
        if !prepared.source_arrowhead.is_empty()
            || !prepared.target_arrowhead.is_empty()
            || prepared.label.is_some()
        {
            return Err(
                "prepared edge labels and authored arrowheads are outside this probe's input scope"
                    .into(),
            );
        }
        let source = ids
            .get(&prepared.from)
            .copied()
            .ok_or_else(|| format!("unknown source node {}", prepared.from))?;
        let target = ids
            .get(&prepared.to)
            .copied()
            .ok_or_else(|| format!("unknown target node {}", prepared.to))?;
        let id = input.add_edge(Edge { source, target });
        input.tala_edge_ids[id.0 as usize] = Some(prepared.id);
        input.set_edge_min_dimensions(id, Some(prepared.min_width), Some(prepared.min_height));
    }
    let mut pipeline = Pipeline::new_untraced(&input, 0, false, false);
    let graph = &mut pipeline.graph;
    // These controls use source NewGraph/NewNode/Connect, bypassing D2's
    // translation adapter. Its implicit root key and decoration defaults are
    // not part of those prepared source constructors.
    graph.containers.clear();
    graph.source_container_child_order.clear();
    graph.connected_container_child_order.clear();
    graph.directions.clear();
    graph.scoring_directions.clear();
    graph.directions_by_tala.clear();
    graph.scoring_directions_by_tala.clear();
    for (index, node) in graph.nodes.iter_mut().enumerate() {
        node.label_text_present = false;
        node.content_insets = Insets::uniform(0.0);
        node.position = nodes[index]
            .position
            .as_ref()
            .map(RawPoint::point)
            .transpose()?;
    }
    for edge in &mut graph.edge_records {
        edge.source_arrow = false;
        edge.target_arrow = false;
        edge.source_arrowhead = None;
        edge.target_arrowhead = None;
    }
    graph.cell_size = 0.0;
    Ok(pipeline)
}

fn padding(graph: &ArenaGraph, limit: i64) -> (f64, i64, String) {
    let context = BackgroundContext;
    let mut guard = match limits::WorkGuard::new(Some(&context), "LoopOffsetMapPadding", limit) {
        Ok(guard) => guard,
        Err(error) => return (0.0, 0, error.to_string()),
    };
    match graph.gap_transaction_sweep_padding_work(&mut guard) {
        Ok(value) => (value, guard.used(), String::new()),
        // The source padding routine returns its zero result on failure.
        Err(error) => (0.0, guard.used(), error.to_string()),
    }
}

fn require_actions(request: &Request, required: &[&str]) -> Result<(), String> {
    for action in required {
        if !request.actions.iter().any(|selected| selected == action) {
            return Err(format!("missing requested action {action}"));
        }
    }
    if let Some(action) = request
        .actions
        .iter()
        .find(|action| !required.contains(&action.as_str()))
    {
        return Err(format!(
            "unsupported action {action} for family {}",
            request.family
        ));
    }
    Ok(())
}

fn run_request(request: Request, repeats: usize) -> Result<Value, String> {
    let observer = repeats != 0;
    let mut pipeline = prepared_pipeline(&[&request.node], &request.edges)?;
    let node = NodeId(0);
    let before = observe(&pipeline.graph.nodes[0], repeats)?;
    let mut output = match request.family.as_str() {
        "authored-map" => {
            require_actions(&request, &["padding", "direction-spacing"])?;
            if !request.edges.is_empty() {
                return Err("authored direction work accounting requires an edgeless graph".into());
            }
            let limit = request
                .padding_work_limit
                .ok_or("padding_work_limit is required")?;
            let (value, used, error) = padding(&pipeline.graph, limit);
            let peer = request
                .peer
                .as_ref()
                .ok_or("direction-spacing requires a peer")?;
            let directions_graph = prepared_pipeline(&[&request.node, peer], &[])?.graph;
            let peer_id = NodeId(1);
            let target_position = directions_graph
                .active_node_position(node)
                .ok_or("direction-spacing requires a positioned target")?;
            let mut directions = Vec::new();
            for proposed in &request.proposed_peer_positions {
                let proposed = proposed.point()?;
                let orientation = directions_graph.sized_box_orientation(
                    (proposed, directions_graph.active_node_size(peer_id)),
                    (target_position, directions_graph.active_node_size(node)),
                );
                let delta = directions_graph.spacing_delta_with_loops(peer_id, node, proposed);
                if !delta.is_finite() {
                    return Err("native spacing returned a non-finite value outside integer source output scope".into());
                }
                directions.push(json!({
                    "point": point_state(Some(proposed)),
                    "orientation": loop_orientation_key(orientation),
                    "opposite": loop_orientation_key(orientation.opposite()),
                    "actual_target_lookup_bits": bits(loop_offset_toward(directions_graph.nodes[0].loop_offsets.as_ref(), orientation)),
                    "delta": delta as i64,
                    "delta_bits": bits(delta),
                    "work": 0,
                    "error": "",
                }));
            }
            let (_, limit_used, limit_error) = request
                .w_minus_one_limit
                .map_or((0.0, 0, String::new()), |limit| {
                    padding(&pipeline.graph, limit)
                });
            json!({
                "family": request.family, "case": request.case, "observer": observer,
                "before": before, "after": observe(&pipeline.graph.nodes[0], repeats)?,
                "padding_bits": bits(value), "work_used": used, "error": error,
                "directions": directions, "work_limit_used": limit_used, "work_limit_error": limit_error,
            })
        }
        "computed-map" => {
            require_actions(
                &request,
                &[
                    "source-node-bounds-without-offsets",
                    "compute-offsets",
                    "padding",
                ],
            )?;
            pipeline.graph.nodes[0].loop_offsets =
                map_from_entries(request.replace_before_compute_loop_offsets.as_deref())?;
            let replaced = observe(&pipeline.graph.nodes[0], repeats)?;
            let base_without_offsets = if request.base_bounds_observed {
                let current = &pipeline.graph.nodes[0];
                let origin = current
                    .position
                    .ok_or("base bounds requested for an unpositioned node")?;
                let (top_left, bottom_right) = loop_base_bounds_without_offsets(
                    current,
                    Rect {
                        origin,
                        size: current.rect.size,
                    },
                );
                json!({"top_left": point_state(Some(top_left)), "bottom_right": point_state(Some(bottom_right))})
            } else {
                Value::Null
            };
            // Production UpdateOffsets owns replacement and route cleanup;
            // pure compute is reached through it, rather than duplicating it.
            pipeline.graph.update_loop_offsets(node);
            let limit = request
                .padding_work_limit
                .ok_or("padding_work_limit is required")?;
            let (value, used, error) = padding(&pipeline.graph, limit);
            json!({
                "family": request.family, "case": request.case, "observer": observer,
                "entry": before, "before": replaced, "after": observe(&pipeline.graph.nodes[0], repeats)?,
                "base_without_offsets": base_without_offsets,
                "three_d": pipeline.graph.nodes[0].is_3d,
                "multiple": pipeline.graph.nodes[0].is_multiple,
                "loop": pipeline.graph.edge_records.iter().any(|edge| edge.from == node && edge.to == node),
                "routes_cleared": pipeline.graph.edge_records.iter().filter(|edge| edge.from == node && edge.to == node).all(|edge| edge.route.is_empty()),
                "padding_bits": bits(value), "work_used": used, "error": error,
            })
        }
        "clone-decode-boundaries" => {
            require_actions(
                &request,
                &[
                    "full-source-clone",
                    "graphjson-deserialize-without-loop-map-field",
                ],
            )?;
            let mut clone = pipeline.graph.clone();
            snapshot::prepare_compound_candidate_state(&mut clone);
            let original = &pipeline.graph.nodes[0];
            if !original.rect.size.width.is_finite()
                || !original.rect.size.height.is_finite()
                || original
                    .position
                    .is_some_and(|point| !point.x.is_finite() || !point.y.is_finite())
            {
                return Err("graphjson boundary cannot encode non-finite geometry".into());
            }
            let wire = json!({
                "nodes": [{"id": original.tala_id,
                    "width": original.rect.size.width, "height": original.rect.size.height,
                    "topLeft": original.position.map(|point| json!({"x": point.x, "y": point.y})),
                    "shapeType": request.node.shape.as_deref().unwrap_or("rectangle")}],
                "containers": {},
            });
            let wire = serde_json::to_vec(&wire)
                .map_err(|error| format!("graphjson wire encoding failed: {error}"))?;
            let decoded_input = graphjson::graphjson_to_graph(&wire)
                .map_err(|error| format!("graphjson production decode failed: {error}"))?;
            let decoded = ArenaGraph::from_input(&decoded_input);
            let decoded_node = decoded.nodes.first().ok_or("graphjson produced no node")?;
            json!({
                "family": request.family, "case": request.case, "observer": observer,
                "before": before, "original_after": observe(original, repeats)?,
                "clone": observe(&clone.nodes[0], repeats)?, "decoded": observe(decoded_node, repeats)?,
            })
        }
        family => return Err(format!("unsupported loop-offset probe family {family}")),
    };
    output["supported"] = json!(true);
    output["evidence_limits"] = json!({
        "map_observation": "sorted raw key/value bits and nil state; map traversal and alias identity are not measured",
        "geometry": "actual arena fields; base bounds use the shared production loop_base_bounds_without_offsets helper for source Node.BoundingBox(nil)",
        "direction_work": "edgeless production spacing method has no WorkGuard; zero corresponds only to absent incident-edge work",
        "clone_boundary": "actual prepare_compound_candidate_state full-clone boundary; not a complete general graph Clone qualification",
        "graphjson_boundary": "actual production graphjson_to_graph and ArenaGraph reconstruction without a loop-map wire field",
        "nil_position_base_bounds": "source harness did not observe these bounds; null is explicitly unqualified",
        "operation_trace_parity": false,
    });
    Ok(output)
}

/// Reads one generic prepared profile and returns source-schema raw controls.
///
/// Select `observer_repeats: 0` or `2` (or `observer: false`/`true`) before
/// calling. Floats use `0x` plus sixteen IEEE-754 hexadecimal digits. Invalid
/// requests and unsupported input scopes return an explicit `request_error`.
pub fn diagnostic_loop_offsets_probe(request: Value) -> Value {
    let result = observer_repeats(&request).and_then(|repeats| {
        serde_json::from_value::<Request>(request)
            .map_err(|error| format!("invalid loop-offset profile: {error}"))
            .and_then(|request| run_request(request, repeats))
    });
    match result {
        Ok(output) => output,
        Err(error) => json!({"supported": false, "request_error": error}),
    }
}
