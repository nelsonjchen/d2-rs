// Copyright (c) 2018, 2020 Kiel University and others.
// Copyright (c) 2020 Kiel University and others.
// Copyright (c) 2026 The elk-go Authors.
// SPDX-License-Identifier: EPL-2.0
// Native translation of the D2-reachable default Rectpacking processor chain.
mod approximate;
mod compact;
pub(crate) mod micro;
use crate::utility::{layout_default_children, minimum, option, padding};
use crate::{ElkGraph, ElkOptions, LayeringError};
mod profile;
use crate::property::{enum_value, flag, integer, node_flag, number, number_value};
use profile::Profile;
use serde_json::Value;

pub(crate) fn layout(graph: &mut ElkGraph, options: ElkOptions) -> Result<(), LayeringError> {
    if graph.children.is_empty() {
        return Ok(());
    }
    let consumed = layout_default_children(graph, options)?;
    let original_order: std::collections::BTreeMap<_, _> = graph
        .children
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id.clone(), i))
        .collect();
    let result = (|| {
        let omit_micro = flag(&graph.layout_options, "omitNodeMicroLayout", false);
        if !omit_micro {
            micro::apply(graph, &consumed);
        }
        let profile = Profile::new(&graph.layout_options);
        if flag(&graph.layout_options, "rectpacking.trybox", false) && graph.children.len() >= 3 {
            let spacing = number(&graph.layout_options, "spacing.nodeNode", 15.0);
            let stackable = graph.children.windows(3).any(|n| {
                n[0].height >= n[1].height + n[2].height + spacing
                    || n[2].height >= n[0].height + n[1].height + spacing
            });
            if !stackable {
                let original_options: Vec<_> = graph
                    .children
                    .iter()
                    .map(|n| n.layout_options.clone())
                    .collect();
                let count = graph.children.len();
                for (i, n) in graph.children.iter_mut().enumerate() {
                    let o = n
                        .layout_options
                        .get_or_insert_with(|| serde_json::json!({}));
                    o["org.eclipse.elk.priority"] = serde_json::json!(count - i);
                }
                let result = crate::box_layout::layout_prepared(graph);
                for (n, o) in graph.children.iter_mut().zip(original_options) {
                    n.layout_options = o;
                }
                return result;
            }
        }
        if flag(&graph.layout_options, "rectpacking.orderBySize", false) {
            graph.children.sort_by(|a, b| {
                b.height
                    .partial_cmp(&a.height)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        }
        if flag(&graph.layout_options, "interactive", false) {
            for n in &graph.children {
                if let Some(position) = n
                    .layout_options
                    .as_ref()
                    .and_then(|o| integer(o, "rectpacking.desiredPosition"))
                {
                    if position < 0 {
                        return Err(LayeringError::Import(format!(
                            "rectpacking desired position {position} is negative"
                        )));
                    }
                }
            }
            let mut fixed = Vec::new();
            let mut free = Vec::new();
            for n in std::mem::take(&mut graph.children) {
                let position = n
                    .layout_options
                    .as_ref()
                    .and_then(|o| integer(o, "rectpacking.desiredPosition"));
                if let Some(position) = position {
                    fixed.push((position, n));
                } else {
                    free.push(n);
                }
            }
            // The source's equality comparator reverses the fixed insertion order.
            fixed.reverse();
            fixed.sort_by_key(|x| x.0);
            for (position, n) in fixed {
                if position < 0 {
                    return Err(LayeringError::Import(format!(
                        "rectpacking desired position {position} is negative"
                    )));
                }
                free.insert((position as usize).min(free.len()), n);
            }
            graph.children = free;
        }
        let pad = padding(&graph.layout_options);
        let spacing = option(&graph.layout_options, "spacing.nodeNode")
            .and_then(|v| v.as_f64().or_else(|| v.as_str()?.parse().ok()))
            .unwrap_or(15.0);
        let aspect = option(&graph.layout_options, "aspectRatio")
            .and_then(|v| v.as_f64().or_else(|| v.as_str()?.parse().ok()))
            .unwrap_or(1.3);
        let min = minimum(&graph.layout_options);
        let strategy = enum_value(
            &graph.layout_options,
            "rectpacking.widthApproximation.strategy",
            &["GREEDY", "TARGET_WIDTH"],
            "GREEDY",
        );
        let width=if strategy=="TARGET_WIDTH" {
            number_value(&graph.layout_options,"rectpacking.widthApproximation.targetWidth").ok_or_else(||LayeringError::Import("rectpacking TARGET_WIDTH requires org.eclipse.elk.rectpacking.widthApproximation.targetWidth".into()))?
        } else {approximate::width(&mut graph.children,spacing,pad,aspect,&profile.goal,profile.shift)}.max(min.0);
        let (width, height) = compact::place(
            &mut graph.children,
            width,
            min,
            pad,
            spacing,
            aspect,
            &profile,
        )?;
        let (mut real_width, mut real_height) = (0.0_f64, 0.0_f64);
        for node in &graph.children {
            real_width = real_width.max(node.x + node.width);
            real_height = real_height.max(node.y + node.height);
        }
        let align = option(&graph.layout_options, "contentAlignment")
            .and_then(Value::as_str)
            .unwrap_or("V_TOP H_LEFT");
        let dx = if width > real_width {
            if align.contains("H_CENTER") {
                (width - real_width) / 2.0
            } else if align.contains("H_RIGHT") {
                width - real_width
            } else {
                0.0
            }
        } else {
            0.0
        };
        let dy = if height > real_height {
            if align.contains("V_CENTER") {
                (height - real_height) / 2.0
            } else if align.contains("V_BOTTOM") {
                height - real_height
            } else {
                0.0
            }
        } else {
            0.0
        };
        for node in &mut graph.children {
            node.x += dx;
            node.y += dy;
            node.x += pad[1];
            node.y += pad[0];
        }
        if !option(&graph.layout_options, "nodeSize.fixedGraphSize")
            .is_some_and(|v| v == true || v == "true")
        {
            graph.width = Some(width + (pad[1] + pad[3]));
            graph.height = Some(height + (pad[0] + pad[2]));
        }
        if !omit_micro {
            micro::apply(graph, &consumed);
        }
        graph.x = Some(graph.x.unwrap_or(0.0));
        graph.y = Some(graph.y.unwrap_or(0.0));
        Ok(())
    })();
    // The source codec exports the original wire inventory, independently of
    // the provider's internal child EList ordering.
    graph.children.sort_by_key(|n| original_order[&n.id]);
    result
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod tests {
    use super::*;
    #[test]
    #[cfg(feature = "oracle-fixtures")]
    fn original_go_default_rectpacking_geometry() {
        check_rows(d2_rs_testdata::oracle_fixture!(
            "src/fixtures/original_rectpacking_default.json"
        ));
    }
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_go_rectpacking_option_profiles() {
        check_rows(d2_rs_testdata::oracle_fixture!(
            "src/fixtures/original_rectpacking_options.json"
        ));
    }
    #[test]
    #[cfg(feature = "oracle-fixtures")]
    fn original_go_rectpacking_provider_errors_preserve_children() {
        let rows: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "src/fixtures/original_rectpacking_errors.json"
        ))
        .unwrap();
        for row in rows.as_array().unwrap() {
            let mut graph: ElkGraph = serde_json::from_value(row["input"].clone()).unwrap();
            let ids: Vec<_> = graph.children.iter().map(|n| n.id.clone()).collect();
            let error = layout(&mut graph, ElkOptions::default()).unwrap_err();
            let LayeringError::Import(message) = error else {
                panic!("unexpected error {error}")
            };
            assert_eq!(
                message,
                row["provider_error"].as_str().unwrap(),
                "{}",
                row["name"]
            );
            assert_eq!(
                graph
                    .children
                    .iter()
                    .map(|n| n.id.clone())
                    .collect::<Vec<_>>(),
                ids
            );
        }
    }
    fn check_rows(input: &str) {
        let rows: Value = serde_json::from_str(input).unwrap();
        let failed: Vec<_> = rows
            .as_array()
            .unwrap()
            .iter()
            .filter(|row| std::panic::catch_unwind(|| check_row(row)).is_err())
            .map(|r| r["name"].clone())
            .collect();
        assert!(failed.is_empty(), "failed controls: {failed:?}");
    }
    fn check_row(row: &Value) {
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
            assert_eq!(node.id, shape["id"].as_str().unwrap());
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
            let expected_labels = shape
                .get("labels")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            assert_eq!(node.labels.len(), expected_labels.len());
            for (label, expected) in node.labels.iter().zip(&expected_labels) {
                assert_eq!(label.text, expected["text"].as_str().unwrap());
                for (key, actual) in [
                    ("x", label.x),
                    ("y", label.y),
                    ("width", label.width),
                    ("height", label.height),
                ] {
                    assert_eq!(
                        actual.to_bits(),
                        expected[key].as_f64().unwrap().to_bits(),
                        "{} label {key}",
                        row["name"]
                    );
                }
            }
            let expected_ports = shape
                .get("ports")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            assert_eq!(node.ports.len(), expected_ports.len());
            for (port, expected) in node.ports.iter().zip(&expected_ports) {
                assert_eq!(port.id, expected["id"].as_str().unwrap());
                for (key, actual) in [
                    ("x", port.x),
                    ("y", port.y),
                    ("width", port.width),
                    ("height", port.height),
                ] {
                    assert_eq!(
                        actual.to_bits(),
                        expected[key].as_f64().unwrap().to_bits(),
                        "{} port {key}",
                        row["name"]
                    );
                }
            }
        }
    }
}
