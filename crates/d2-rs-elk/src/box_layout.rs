// Copyright (c) 2009, 2020 Kiel University and others.
// Copyright (c) 2015, 2020 Kiel University and others.
// SPDX-License-Identifier: EPL-2.0
// Translation of elk-go v0.2.0/internal/box/provider.go.

use d2_rs_core::stable_sort::StableSort;

#[path = "box_grouping.rs"]
mod grouping;

use crate::{ElkGraph, ElkNode, ElkOptions, LayeringError};
use serde_json::Value;
use std::cmp::Ordering;

use crate::utility::option;

fn number(options: &Value, name: &str, default: f64) -> f64 {
    option(options, name)
        .and_then(|v| v.as_f64().or_else(|| v.as_str()?.trim().parse().ok()))
        .unwrap_or(default)
}

fn java_compare(a: f64, b: f64) -> Ordering {
    if a < b {
        return Ordering::Less;
    }
    if a > b {
        return Ordering::Greater;
    }
    let bits = |v: f64| {
        if v.is_nan() {
            0x7ff8_0000_0000_0000_i64
        } else {
            v.to_bits() as i64
        }
    };
    bits(a).cmp(&bits(b))
}

fn maximum(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else {
        a.max(b)
    }
}

/// Executes the source Box provider on the ELK wire model.
/// Nested containers without an explicit algorithm use the source default
/// layered provider.
pub fn layout(graph: &mut ElkGraph, options: ElkOptions) -> Result<(), LayeringError> {
    crate::layout_utility_wire(graph, options, execute)
}

fn execute(graph: &mut ElkGraph, options: ElkOptions) -> Result<(), LayeringError> {
    if graph.children.is_empty() {
        return Ok(());
    }
    let original: Vec<_> = graph
        .children
        .iter()
        .map(|n| {
            (
                n.layout_options.clone(),
                n.ports
                    .iter()
                    .map(|p| p.layout_options.clone())
                    .collect::<Vec<_>>(),
            )
        })
        .collect();
    let consumed = crate::utility::layout_default_children(graph, options)?;
    for child in &mut graph.children {
        if !consumed.contains(&child.id) {
            continue;
        }
        let opts = child.layout_options.as_ref().unwrap_or(&Value::Null);
        let algorithm = option(opts, "algorithm")
            .and_then(Value::as_str)
            .and_then(|s| crate::ElkAlgorithm::parse(s).ok())
            .unwrap_or(crate::ElkAlgorithm::Layered);
        let cleared = match algorithm {
            crate::ElkAlgorithm::Layered
            | crate::ElkAlgorithm::Box
            | crate::ElkAlgorithm::Rectpacking
            | crate::ElkAlgorithm::Random
            | crate::ElkAlgorithm::SporeOverlap
            | crate::ElkAlgorithm::SporeCompaction => true,
            crate::ElkAlgorithm::Fixed => !crate::utility::flag(opts, "nodeSize.fixedGraphSize"),
            _ => false,
        };
        if cleared {
            let opts = child
                .layout_options
                .get_or_insert_with(|| serde_json::json!({}));
            if let Some(map) = opts.as_object_mut() {
                map.retain(|key, _| !key.ends_with("nodeSize.constraints"));
                map.insert(
                    "org.eclipse.elk.nodeSize.constraints".into(),
                    Value::String("[]".into()),
                );
            }
        }
    }
    let result = layout_prepared(graph);
    // Core properties mutate internally; Codec clones the original JSON
    // options rather than exporting those cached/reset properties.
    for (child, (opts, ports)) in graph.children.iter_mut().zip(original) {
        child.layout_options = opts;
        for (port, opts) in child.ports.iter_mut().zip(ports) {
            port.layout_options = opts;
        }
    }
    result
}

pub(crate) fn layout_prepared(graph: &mut ElkGraph) -> Result<(), LayeringError> {
    if option(&graph.layout_options, "box.packingMode")
        .and_then(Value::as_str)
        .unwrap_or("SIMPLE")
        != "SIMPLE"
    {
        return grouping::layout(graph);
    }
    let spacing = number(&graph.layout_options, "spacing.nodeNode", 15.0) as f32 as f64;
    let mut padding = [15.0; 4]; // top, left, bottom, right
    if let Some(text) = option(&graph.layout_options, "padding").and_then(Value::as_str) {
        padding = [0.0; 4];
        for item in text.trim_matches(['[', ']']).split(',') {
            if let Some((key, value)) = item.split_once('=') {
                if let Some(index) = ["top", "left", "bottom", "right"]
                    .iter()
                    .position(|side| *side == key.trim())
                {
                    padding[index] = value.trim().parse().unwrap_or(0.0);
                }
            }
        }
    }
    let [top, left, bottom, right] = padding;
    let aspect = number(&graph.layout_options, "aspectRatio", 1.3_f32 as f64);
    let aspect = if aspect <= 0.0 { 1.3 } else { aspect };
    let interactive =
        option(&graph.layout_options, "interactive").is_some_and(|v| v == true || v == "true");
    let mut order: Vec<usize> = (0..graph.children.len()).collect();
    order.stable_sort_by(|&a, &b| {
        let first = &graph.children[a];
        let second = &graph.children[b];
        let priority = |n: &ElkNode| {
            n.layout_options
                .as_ref()
                .map(|o| number(o, "priority", 0.0))
                .unwrap_or(0.0)
        };
        java_compare(priority(second), priority(first))
            .then_with(|| {
                if interactive {
                    java_compare(first.y, second.y).then_with(|| java_compare(first.x, second.x))
                } else {
                    Ordering::Equal
                }
            })
            .then_with(|| java_compare(first.width * first.height, second.width * second.height))
    });
    let mut max_width = 0.0;
    let mut area = 0.0;
    let direction = option(&graph.layout_options, "direction")
        .and_then(Value::as_str)
        .unwrap_or("UNDEFINED")
        .to_owned();
    for &i in &order {
        grouping::resize_for_constraints(&mut graph.children[i], &direction);
    }
    for &i in &order {
        let n = &graph.children[i];
        max_width = maximum(max_width, n.width);
        area += n.width * n.height;
    }
    let mean = area / order.len() as f64;
    let variance = order
        .iter()
        .map(|&i| {
            let n = &graph.children[i];
            let delta = n.width * n.height - mean;
            delta * delta
        })
        .sum::<f64>();
    area += order.len() as f64 * (variance / (order.len() - 1) as f64).sqrt();
    area += area.sqrt() * (bottom + top);
    area += area.sqrt() * right;
    max_width = maximum(max_width, (area * aspect).sqrt()) + left;
    let mut x = left;
    let mut y = top;
    let mut highest = 0.0;
    let mut broadest = left + right;
    let expand = crate::utility::flag(&graph.layout_options, "expandNodes");
    let mut row_indices = vec![0];
    let mut row_heights = Vec::new();
    for (index, &i) in order.iter().enumerate() {
        let n = &mut graph.children[i];
        if x + n.width > max_width {
            if expand {
                row_heights.push(highest);
                row_indices.push(index);
            }
            x = left;
            y += highest + spacing;
            highest = 0.0;
            broadest = maximum(broadest, left + right + n.width);
        }
        n.x = x;
        n.y = y;
        broadest = maximum(broadest, x + n.width + right);
        highest = maximum(highest, n.height);
        x += n.width + spacing;
    }
    let minimum = crate::utility::minimum(&graph.layout_options);
    broadest = maximum(broadest, minimum.0);
    let mut total_height = y + highest + bottom;
    if total_height < minimum.1 {
        highest += minimum.1 - total_height;
        total_height = minimum.1;
    }
    if expand {
        row_indices.push(order.len());
        row_heights.push(highest);
        let mut row = 0;
        let mut height = 0.0;
        x = left;
        for (index, &i) in order.iter().enumerate() {
            if index == row_indices[row] {
                x = left;
                height = row_heights[row];
                row += 1;
            }
            let node = &mut graph.children[i];
            let old = (node.width, node.height);
            node.height = height;
            if index + 1 == row_indices[row] {
                node.width = broadest - x - right;
                let align = node
                    .layout_options
                    .as_ref()
                    .and_then(|v| option(v, "contentAlignment"))
                    .and_then(Value::as_str)
                    .unwrap_or("[H_LEFT, V_TOP]");
                let shift = |delta: f64, center: &str, end: &str| {
                    if delta <= 0.0 {
                        0.0
                    } else if align.contains(center) {
                        delta / 2.0
                    } else if align.contains(end) {
                        delta
                    } else {
                        0.0
                    }
                };
                let dx = shift(node.width - old.0, "H_CENTER", "H_RIGHT");
                let dy = shift(node.height - old.1, "V_CENTER", "V_BOTTOM");
                grouping::translate_contents(graph, i, dx, dy);
            }
            x += graph.children[i].width + spacing;
        }
    }
    graph.width = Some(broadest);
    graph.height = Some(total_height);
    graph.x = Some(graph.x.unwrap_or(0.0));
    graph.y = Some(graph.y.unwrap_or(0.0));
    Ok(())
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod tests {
    use super::*;
    #[test]
    #[cfg(feature = "oracle-fixtures")]
    fn original_go_simple_box_geometry() {
        let rows: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "src/fixtures/original_box_simple.json"
        ))
        .unwrap();
        for row in rows.as_array().unwrap() {
            let mut graph: ElkGraph = serde_json::from_value(row["input"].clone()).unwrap();
            layout(&mut graph, ElkOptions::default()).unwrap();
            let expected = &row["expected"];
            for (key, actual) in [
                ("width", graph.width.unwrap()),
                ("height", graph.height.unwrap()),
            ] {
                assert_eq!(
                    actual.to_bits(),
                    expected[key].as_f64().unwrap().to_bits(),
                    "{} {key}",
                    row["name"]
                );
            }
            assert_eq!(
                graph.children.len(),
                expected["children"].as_array().unwrap().len()
            );
            for (node, shape) in graph
                .children
                .iter()
                .zip(expected["children"].as_array().unwrap())
            {
                for (key, actual) in [
                    ("x", node.x),
                    ("y", node.y),
                    ("width", node.width),
                    ("height", node.height),
                ] {
                    assert_eq!(
                        actual.to_bits(),
                        shape[key].as_f64().unwrap().to_bits(),
                        "{} {} {key}",
                        row["name"],
                        node.id
                    );
                }
            }
        }
    }
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod grouping_tests {
    use super::*;
    fn compare(actual: &Value, expected: &Value, path: &str) -> Result<(), String> {
        match (actual, expected) {
            (Value::Number(a), Value::Number(b)) => {
                let (a, b) = (a.as_f64().unwrap(), b.as_f64().unwrap());
                if a.to_bits() != b.to_bits() {
                    return Err(format!(
                        "{path}: {a:?} [{:016x}] != {b:?} [{:016x}]",
                        a.to_bits(),
                        b.to_bits()
                    ));
                }
            }
            (Value::Array(a), Value::Array(b)) => {
                if a.len() != b.len() {
                    return Err(format!("{path}: inventory {} != {}", a.len(), b.len()));
                }
                for (i, (a, b)) in a.iter().zip(b).enumerate() {
                    compare(a, b, &format!("{path}[{i}]"))?;
                }
            }
            (Value::Object(a), Value::Object(b)) => {
                const KEYS: &[&str] = &[
                    "x",
                    "y",
                    "width",
                    "height",
                    "id",
                    "text",
                    "container",
                    "sources",
                    "targets",
                    "children",
                    "edges",
                    "ports",
                    "labels",
                    "sections",
                    "startPoint",
                    "endPoint",
                    "bendPoints",
                    "junctionPoints",
                    "incomingShape",
                    "outgoingShape",
                ];
                for key in KEYS {
                    let a = a.get(*key);
                    let b = b.get(*key);
                    match (a, b) {
                        (Some(a), Some(b)) => compare(a, b, &format!("{path}.{key}"))?,
                        (None, None) => {}
                        _ => return Err(format!("{path}.{key}: field presence {a:?} != {b:?}")),
                    }
                }
            }
            _ => {
                if actual != expected {
                    return Err(format!("{path}: {actual} != {expected}"));
                }
            }
        }
        Ok(())
    }

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_go_box_grouping_expansion_and_constraints() {
        exercise(
            d2_rs_testdata::oracle_fixture!("src/fixtures/original_box_grouping.json"),
            180,
            "D2RS_BOX_RECORDS",
        );
    }
    #[test]
    #[cfg(feature = "oracle-fixtures")]
    fn original_go_box_grouping_nested_and_port_inference() {
        exercise(
            d2_rs_testdata::oracle_fixture!("src/fixtures/original_box_grouping_expanded.json"),
            68,
            "D2RS_BOX_EXPANDED_RECORDS",
        );
    }
    fn exercise(text: &str, count: usize, record_key: &str) {
        let rows: Value = serde_json::from_str(text).unwrap();
        assert_eq!(rows.as_array().unwrap().len(), count);
        let mut failures = vec![];
        let mut records = vec![];
        for row in rows.as_array().unwrap() {
            let mut graph: ElkGraph = serde_json::from_value(row["input"].clone()).unwrap();
            let outcome = layout(&mut graph, ElkOptions::default());
            let actual = serde_json::to_value(graph).unwrap();
            records.push(serde_json::json!({"name":row["name"],"actual":actual,"error":outcome.as_ref().err().map(ToString::to_string)}));
            if let Err(error) = outcome
                .map_err(|e| e.to_string())
                .and_then(|_| compare(&actual, &row["expected"], "root"))
            {
                failures.push(format!("{}: {error}", row["name"]));
            }
        }
        if let Ok(path) = std::env::var(record_key) {
            std::fs::write(path, serde_json::to_vec_pretty(&records).unwrap()).unwrap();
        }
        assert!(
            failures.is_empty(),
            "{} differences:\n{}",
            failures.len(),
            failures.join("\n")
        );
    }
}
