// Copyright (c) 2010, 2015 Kiel University and others.
// Copyright (c) 2015, 2020 Kiel University and others.
// SPDX-License-Identifier: EPL-2.0
// elk-go v0.2.0/internal/random/provider.go.
use crate::layered::JavaRandom;
use crate::utility::{layout_default_children, minimum, option, padding};
use crate::{ElkEdge, ElkEdgeSection, ElkGraph, ElkNode, ElkOptions, ElkPoint, LayeringError};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicI64, Ordering};
static SEED_UNIQUIFIER: AtomicI64 = AtomicI64::new(8682522807148012);
fn default_seed() -> i64 {
    let mut current = SEED_UNIQUIFIER.load(Ordering::SeqCst);
    loop {
        let next = current.wrapping_mul(181783497276652981);
        match SEED_UNIQUIFIER.compare_exchange(current, next, Ordering::SeqCst, Ordering::SeqCst) {
            Ok(_) => {
                let nanos = match web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH) {
                    Ok(d) => d.as_nanos() as i64,
                    Err(e) => (e.duration().as_nanos() as i64).wrapping_neg(),
                };
                return next ^ nanos;
            }
            Err(actual) => current = actual,
        }
    }
}
fn number(options: &Value, name: &str, default: f64) -> f64 {
    option(options, name)
        .and_then(|v| {
            v.as_f64()
                .or_else(|| d2_rs_core::parse_d2_float(v.as_str()?.trim()))
        })
        .unwrap_or(default)
}
// Codec converts JSON option numbers through float64 and Go's shortest `g`
// format before ParseInt(32). Integral numbers at |n| >= 1e6 use exponents,
// which ParseInt rejects. Strings retain their decimal integer spelling.
fn seed_value(value: &Value) -> Option<i64> {
    match value {
        Value::String(text) => text.trim().parse::<i32>().ok().map(i64::from),
        Value::Number(_) => value
            .as_f64()
            .filter(|n| n.is_finite() && n.fract() == 0.0 && n.abs() < 1e6)
            .map(|n| n as i64),
        _ => None,
    }
}
// Pinned Go1.27 arm64 lowers these expressions to FMADD/FMSUB (recorded
// source-random-edge.asm). Go's wasm and baseline non-arm64 paths do not.
fn source_mul_add(a: f64, b: f64, c: f64) -> f64 {
    #[cfg(target_arch = "aarch64")]
    {
        a.mul_add(b, c)
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        a * b + c
    }
}
fn maximum(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else {
        a.max(b)
    }
}
#[derive(Clone)]
struct Shape {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    parent: Option<String>,
    port_parent_x: Option<f64>,
}
fn shapes(graph: &ElkGraph) -> BTreeMap<String, Shape> {
    fn collect(nodes: &[ElkNode], parent: &str, out: &mut BTreeMap<String, Shape>) {
        for node in nodes {
            out.insert(
                node.id.clone(),
                Shape {
                    x: node.x,
                    y: node.y,
                    width: node.width,
                    height: node.height,
                    parent: Some(parent.to_owned()),
                    port_parent_x: None,
                },
            );
            for port in &node.ports {
                out.insert(
                    port.id.clone(),
                    Shape {
                        x: port.x,
                        y: port.y,
                        width: port.width,
                        height: port.height,
                        parent: Some(parent.to_owned()),
                        port_parent_x: Some(node.x),
                    },
                );
            }
            collect(&node.children, &node.id, out);
        }
    }
    let mut result = BTreeMap::new();
    result.insert(
        graph.id.clone(),
        Shape {
            x: graph.x.unwrap_or(0.0),
            y: graph.y.unwrap_or(0.0),
            width: graph.width.unwrap_or(0.0),
            height: graph.height.unwrap_or(0.0),
            parent: None,
            port_parent_x: None,
        },
    );
    collect(&graph.children, &graph.id, &mut result);
    result
}
pub(crate) fn validate_endpoints(graph: &ElkGraph) -> Result<(), LayeringError> {
    fn check(
        edges: &[ElkEdge],
        nodes: &[ElkNode],
        known: &BTreeMap<String, Shape>,
    ) -> Result<(), LayeringError> {
        for edge in edges {
            for endpoint in edge.sources.iter().chain(&edge.targets) {
                if !known.contains_key(endpoint) {
                    return Err(LayeringError::Import(format!(
                        "edge {:?} references unknown node or port {endpoint:?}",
                        edge.id
                    )));
                }
            }
        }
        for node in nodes {
            check(&node.edges, &node.children, known)?;
        }
        Ok(())
    }
    check(&graph.edges, &graph.children, &shapes(graph))
}
fn randomize_edge(
    edge: &mut ElkEdge,
    rng: &mut JavaRandom,
    shape: &Shape,
    width: f64,
    height: f64,
) -> Result<(), LayeringError> {
    let (mut x, mut y) = (shape.x, shape.y);
    let (half_width, half_height) = (shape.width / 2.0, shape.height / 2.0);
    if let Some(parent_x) = shape.port_parent_x {
        x += parent_x;
        x += parent_x;
    }
    x += half_width;
    y += half_height;
    // Both endpoint lookups use Sources[0] in the pinned provider.
    let anchor_x = if x > x + half_width {
        x + half_width
    } else if x < x - half_width {
        x - half_width
    } else {
        x
    };
    let anchor_y = if y > y + half_height {
        y + half_height
    } else if y < y - half_height {
        y - half_height
    } else {
        y
    };
    let (mut start_x, start_y) = (anchor_x, anchor_y);
    if start_x > x - half_width
        && start_x < x + half_width
        && start_y > y - half_height
        && start_y < y + half_height
    {
        start_x = x + half_width;
    }
    let (end_x, mut end_y) = (anchor_x, anchor_y);
    if end_x > x - half_width
        && end_x < x + half_width
        && end_y > y - half_height
        && end_y < y + half_height
    {
        end_y = y + half_height;
    }
    if edge.sections.len() > 1 {
        return Err(LayeringError::Import(format!(
            "random edge {} has multiple sections",
            d2_rs_core::quote_go_bytes(edge.id.as_bytes())
        )));
    }
    if edge.sections.is_empty() {
        edge.sections.push(ElkEdgeSection {
            id: Some(format!("{}_s0", edge.id)),
            start: ElkPoint::default(),
            end: ElkPoint::default(),
            bends: vec![],
            incoming_shape: None,
            outgoing_shape: None,
        });
    }
    let section = &mut edge.sections[0];
    section.start = ElkPoint {
        x: start_x,
        y: start_y,
    };
    section.end = ElkPoint { x: end_x, y: end_y };
    let bends = rng.next_int_n(5) + 1;
    let (dx, dy) = (end_x - start_x, end_y - start_y);
    let distance = source_mul_add(dy, dy, dx * dx).sqrt();
    let max_random = distance * (0.2_f32 as f64);
    let (x_increment, y_increment) = (dx / (bends + 1) as f64, dy / (bends + 1) as f64);
    let (mut x, mut y) = (start_x, start_y);
    section.bends.clear();
    for _ in 0..bends {
        x += x_increment;
        y += y_increment;
        let mut px = source_mul_add(
            -0.5,
            max_random,
            source_mul_add(rng.next_float() as f64, max_random, x),
        );
        if px < 0.0 {
            px = 1.0;
        } else if px > width {
            px = width - 1.0;
        }
        let mut py = source_mul_add(
            -0.5,
            max_random,
            source_mul_add(rng.next_float() as f64, max_random, y),
        );
        if py < 0.0 {
            py = 1.0;
        } else if py > height {
            py = height - 1.0;
        }
        section.bends.push(ElkPoint { x: px, y: py });
    }
    Ok(())
}
/// Executes the source Random provider, including zero-seed entropy semantics.
pub(crate) fn layout(graph: &mut ElkGraph, options: ElkOptions) -> Result<(), LayeringError> {
    if graph.children.is_empty() {
        return Ok(());
    }
    layout_default_children(graph, options)?;
    let seed = option(&graph.layout_options, "randomSeed")
        .and_then(seed_value)
        .unwrap_or(0);
    let seed = if seed == 0 { default_seed() } else { seed };
    let mut rng = JavaRandom::with_stream(seed, "elk-random-layout");
    let aspect = number(&graph.layout_options, "aspectRatio", 1.6_f32 as f64) as f32 as f64;
    let spacing = number(&graph.layout_options, "spacing.nodeNode", 15.0) as f32 as f64;
    let pad = padding(&graph.layout_options);
    let mut outgoing = Vec::new();
    let (mut area, mut max_width, mut max_height) = (0.0_f64, 0.0_f64, 0.0_f64);
    for node in &graph.children {
        for shape_id in std::iter::once(&node.id).chain(node.ports.iter().map(|p| &p.id)) {
            outgoing.extend(
                graph
                    .edges
                    .iter()
                    .enumerate()
                    .filter_map(|(i, edge)| edge.sources.contains(shape_id).then_some(i)),
            );
        }
        max_width = maximum(max_width, node.width);
        max_height = maximum(max_height, node.height);
        area += node.width * node.height;
    }
    let mut spacing_area = 2.0 * spacing;
    spacing_area *= spacing;
    spacing_area *= (outgoing.len() + 1) as f64;
    spacing_area *= graph.children.len() as f64;
    let root = (area + spacing_area).sqrt();
    let draw_width = maximum(root * aspect, max_width);
    let draw_height = maximum(root / aspect, max_height);
    for node in &mut graph.children {
        let offset_x = rng.next_double() * (draw_width - node.width);
        node.x = pad[1] + offset_x;
        let offset_y = rng.next_double() * (draw_height - node.height);
        node.y = pad[1] + offset_y;
    }
    let width = draw_width + (pad[1] + pad[3]);
    let height = draw_height + (pad[0] + pad[2]);
    let geometries = shapes(graph);
    for index in outgoing {
        let edge = &mut graph.edges[index];
        let mut parent = None;
        let mut initialized = false;
        let mut hierarchical = false;
        for id in edge.sources.iter().chain(&edge.targets) {
            let shape = geometries.get(id).ok_or_else(|| {
                LayeringError::Import(format!("unknown Random edge endpoint {id:?}"))
            })?;
            if !initialized {
                parent = shape.parent.clone();
                initialized = true;
            } else if parent != shape.parent {
                hierarchical = true;
                break;
            }
        }
        if !hierarchical {
            if let Some(shape) = edge
                .sources
                .first()
                .and_then(|id| geometries.get(id))
                .cloned()
            {
                randomize_edge(edge, &mut rng, &shape, width, height)?;
            }
        }
    }
    let containers = crate::edge_container_ids(graph, &graph.edges);
    for (edge, container) in graph.edges.iter_mut().zip(containers) {
        edge.container = container;
    }
    let (min_width, min_height) = minimum(&graph.layout_options);
    graph.width = Some(maximum(min_width, width + (pad[1] + pad[3])));
    graph.height = Some(maximum(min_height, height + (pad[0] + pad[2])));
    graph.x = Some(graph.x.unwrap_or(0.0));
    graph.y = Some(graph.y.unwrap_or(0.0));
    Ok(())
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod tests {
    use super::*;
    #[test]
    #[cfg(feature = "oracle-fixtures")]
    fn original_go_seeded_random_boxes_ports_and_routes() {
        fn project(v: &Value) -> Value {
            match v {
                Value::Object(fields) => Value::Object(
                    fields
                        .iter()
                        .filter(|(key, _)| {
                            [
                                "id",
                                "text",
                                "x",
                                "y",
                                "width",
                                "height",
                                "children",
                                "ports",
                                "labels",
                                "edges",
                                "sources",
                                "targets",
                                "container",
                                "sections",
                                "startPoint",
                                "endPoint",
                                "bendPoints",
                            ]
                            .contains(&key.as_str())
                        })
                        .map(|(k, v)| (k.clone(), project(v)))
                        .collect(),
                ),
                Value::Array(values) => Value::Array(values.iter().map(project).collect()),
                value => value.clone(),
            }
        }
        fn compare(a: &Value, b: &Value, path: &str) {
            match (a, b) {
                (Value::Number(a), Value::Number(b)) => assert_eq!(
                    a.as_f64().unwrap().to_bits(),
                    b.as_f64().unwrap().to_bits(),
                    "{path}: {a} vs {b}"
                ),
                (Value::Object(a), Value::Object(b)) => {
                    assert_eq!(
                        a.keys().collect::<std::collections::BTreeSet<_>>(),
                        b.keys().collect::<std::collections::BTreeSet<_>>(),
                        "{path}"
                    );
                    for (key, b) in b {
                        compare(&a[key], b, &format!("{path}/{key}"));
                    }
                }
                (Value::Array(a), Value::Array(b)) => {
                    assert_eq!(a.len(), b.len(), "{path}");
                    for (i, (a, b)) in a.iter().zip(b).enumerate() {
                        compare(a, b, &format!("{path}/{i}"));
                    }
                }
                _ => assert_eq!(a, b, "{path}"),
            }
        }
        let rows: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "src/fixtures/original_random_seeded.json"
        ))
        .unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 32);
        for row in rows.as_array().unwrap() {
            let mut graph: ElkGraph = serde_json::from_value(row["input"].clone()).unwrap();
            crate::layout_elk_graph_with_random_pipeline(&mut graph, ElkOptions::default())
                .unwrap();
            compare(
                &project(&serde_json::to_value(graph).unwrap()),
                &row["expected"],
                row["name"].as_str().unwrap(),
            );
        }
    }
}

#[cfg(test)]
mod boundary_tests {
    use super::*;
    #[test]
    fn source_json_integer_seed_encoding_boundary() {
        for (input, expected) in [
            (serde_json::json!(1.0), Some(1)),
            (serde_json::json!(999999), Some(999999)),
            (serde_json::json!(1000000), None),
            (serde_json::json!(-2147483648_i64), None),
            (serde_json::json!("1000000"), Some(1000000)),
            (serde_json::json!("-2147483648"), Some(-2147483648)),
            (serde_json::json!(0), Some(0)),
        ] {
            assert_eq!(seed_value(&input), expected, "{input}");
        }
    }
    #[test]
    fn unknown_owned_edge_is_rejected_without_losing_inventories() {
        let mut graph: ElkGraph = serde_json::from_value(serde_json::json!({
            "id":"root","layoutOptions":{"elk.randomSeed":"42"},
            "children":[{"id":"g","width":60,"height":50,
                "children":[{"id":"a","width":40,"height":30},{"id":"b","width":20,"height":10}],
                "edges":[{"id":"invalid","sources":["a"],"targets":["missing"]}]}]
        }))
        .unwrap();
        assert!(
            crate::layout_elk_graph_with_random_pipeline(&mut graph, ElkOptions::default())
                .is_err()
        );
        assert_eq!(
            graph.children[0]
                .children
                .iter()
                .map(|n| n.id.as_str())
                .collect::<Vec<_>>(),
            ["a", "b"]
        );
        assert_eq!(graph.children[0].edges.len(), 1);
        assert_eq!(graph.children[0].edges[0].id, "invalid");
        assert!(graph.edges.is_empty());
    }
}
