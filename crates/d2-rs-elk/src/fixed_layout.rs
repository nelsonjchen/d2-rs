// Copyright (c) 2010, 2019 Kiel University and others.
// Copyright (c) 2015, 2020 Kiel University and others.
// SPDX-License-Identifier: EPL-2.0
// Translation of elk-go v0.2.0/internal/fixed/provider.go.

use crate::utility::{layout_default_children, minimum, option, padding, point};
use crate::{ElkEdgeSection, ElkGraph, ElkLabel, ElkOptions, ElkPoint, LayeringError};
use serde_json::Value;
use std::collections::BTreeSet;

fn move_label(label: &mut ElkLabel) {
    if let Some((x, y)) = label
        .layout_options
        .as_ref()
        .and_then(|o| point(o, "position"))
    {
        label.x = x;
        label.y = y;
    }
}

fn chain(options: &Value) -> Option<Vec<ElkPoint>> {
    let text = option(options, "bendPoints")?.as_str()?;
    let parts: Vec<_> = text
        .split(|c: char| c.is_whitespace() || ",;()[]{}".contains(c))
        .filter(|s| !s.is_empty())
        .collect();
    if parts.len() % 2 != 0 {
        return None;
    }
    parts
        .chunks(2)
        .map(|pair| {
            Some(ElkPoint {
                x: d2_rs_core::parse_d2_float(pair[0])?,
                y: d2_rs_core::parse_d2_float(pair[1])?,
            })
        })
        .collect()
}

/// Runs Fixed without synthesizing positions or routes through another engine.
pub(crate) fn layout(graph: &mut ElkGraph, options: ElkOptions) -> Result<(), LayeringError> {
    if graph.children.is_empty() {
        return Ok(());
    }
    layout_default_children(graph, options)?;
    let mut max_x: f64 = 0.0;
    let mut max_y: f64 = 0.0;
    let mut direct = BTreeSet::new();
    for node in &mut graph.children {
        direct.insert(node.id.clone());
        if let Some((x, y)) = node
            .layout_options
            .as_ref()
            .and_then(|o| point(o, "position"))
        {
            node.x = x;
            node.y = y;
            let (width, height) = node
                .layout_options
                .as_ref()
                .filter(|o| point(o, "nodeSize.minimum").is_some_and(|(x, y)| x > 0.0 && y > 0.0))
                .map(minimum)
                .unwrap_or_default();
            if width > 0.0 && height > 0.0 {
                let old_width = node.width;
                let old_height = node.height;
                node.width = width;
                node.height = height;
                let fixed_ports = node
                    .layout_options
                    .as_ref()
                    .and_then(|o| option(o, "portConstraints"))
                    .is_some_and(|v| v == "FIXED_POS");
                let direction = option(&graph.layout_options, "direction")
                    .and_then(Value::as_str)
                    .unwrap_or("UNDEFINED");
                for port in &mut node.ports {
                    let mut side = option(&port.layout_options, "port.side")
                        .and_then(Value::as_str)
                        .unwrap_or("UNDEFINED");
                    if side == "UNDEFINED" && (old_width > 0.0 || old_height > 0.0) {
                        let px = (port.x + port.width / 2.0) / old_width;
                        let py = (port.y + port.height / 2.0) / old_height;
                        side = if matches!(direction, "LEFT" | "RIGHT") && port.x < 0.0 {
                            "WEST"
                        } else if matches!(direction, "LEFT" | "RIGHT")
                            && port.x + port.width > old_width
                        {
                            "EAST"
                        } else if matches!(direction, "UP" | "DOWN") && port.y < 0.0 {
                            "NORTH"
                        } else if matches!(direction, "UP" | "DOWN")
                            && port.y + port.height > old_height
                        {
                            "SOUTH"
                        } else if px + py <= 1.0 && px - py <= 0.0 {
                            "WEST"
                        } else if px + py >= 1.0 && px - py >= 0.0 {
                            "EAST"
                        } else if py < 0.5 {
                            "NORTH"
                        } else {
                            "SOUTH"
                        };
                    }
                    match side {
                        "NORTH" => {
                            if !fixed_ports {
                                port.x *= width / old_width;
                            }
                        }
                        "EAST" => {
                            port.x += width - old_width;
                            if !fixed_ports {
                                port.y *= height / old_height;
                            }
                        }
                        "SOUTH" => {
                            if !fixed_ports {
                                port.x *= width / old_width;
                            }
                            port.y += height - old_height;
                        }
                        "WEST" => {
                            if !fixed_ports {
                                port.y *= height / old_height;
                            }
                        }
                        _ => {}
                    }
                }
                // ElkUtil.resizeNode moves peripheral labels when a position
                // property also activates a MINIMUM_SIZE constraint.
                for label in &mut node.labels {
                    let mid_x = label.x + label.width / 2.0;
                    let mid_y = label.y + label.height / 2.0;
                    let px = mid_x / old_width;
                    let py = mid_y / old_height;
                    if px + py >= 1.0 {
                        if px - py > 0.0 && mid_y >= 0.0 {
                            label.x += width - old_width;
                            label.y += (height - old_height) * py;
                        } else if px - py < 0.0 && mid_x >= 0.0 {
                            label.x += (width - old_width) * px;
                            label.y += height - old_height;
                        }
                    }
                }
            }
        }
        max_x = max_x.max(node.x + node.width);
        max_y = max_y.max(node.y + node.height);
        for label in &mut node.labels {
            move_label(label);
            max_x = max_x.max(node.x + label.x + label.width);
            max_y = max_y.max(node.y + label.y + label.height);
        }
        for port in &mut node.ports {
            direct.insert(port.id.clone());
            if let Some((x, y)) = point(&port.layout_options, "position") {
                port.x = x;
                port.y = y;
            }
            max_x = max_x.max(node.x + port.x + port.width);
            max_y = max_y.max(node.y + port.y + port.height);
        }
    }
    for edge in &mut graph.edges {
        let Some((source, target)) = edge.sources.first().zip(edge.targets.first()) else {
            continue;
        };
        if !direct.contains(source) && !direct.contains(target) {
            continue;
        }
        let same_hierarchy = direct.contains(source) && direct.contains(target);
        if let Some(points) = edge
            .layout_options
            .as_ref()
            .and_then(chain)
            .filter(|p| p.len() >= 2)
        {
            if edge.sections.len() > 1 {
                return Err(LayeringError::Import(format!(
                    "fixed edge {} has multiple sections",
                    d2_rs_core::quote_go_bytes(edge.id.as_bytes())
                )));
            }
            if edge.sections.is_empty() {
                edge.sections.push(ElkEdgeSection {
                    id: None,
                    start: ElkPoint::default(),
                    end: ElkPoint::default(),
                    bends: Vec::new(),
                    incoming_shape: None,
                    outgoing_shape: None,
                });
            }
            let section = &mut edge.sections[0];
            section.start = points[0];
            section.end = points[points.len() - 1];
            section.bends = points[1..points.len() - 1].to_vec();
        }
        if same_hierarchy {
            for section in &edge.sections {
                for p in &section.bends {
                    max_x = max_x.max(p.x);
                    max_y = max_y.max(p.y);
                }
            }
        }
        for label in &mut edge.labels {
            move_label(label);
            if same_hierarchy {
                max_x = max_x.max(label.x + label.width);
                max_y = max_y.max(label.y + label.height);
            }
        }
    }
    let fixed_size = option(&graph.layout_options, "nodeSize.fixedGraphSize")
        .is_some_and(|v| v == true || v == "true");
    if !fixed_size {
        let [top, left, bottom, right] = padding(&graph.layout_options);
        let (minimum_width, minimum_height) = minimum(&graph.layout_options);
        graph.width = Some(minimum_width.max(max_x + left + right));
        graph.height = Some(minimum_height.max(max_y + top + bottom));
    }
    graph.x = Some(graph.x.unwrap_or(0.0));
    graph.y = Some(graph.y.unwrap_or(0.0));
    Ok(())
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod tests {
    use super::*;
    #[test]
    #[cfg(feature = "oracle-fixtures")]
    fn original_go_fixed_geometry_and_existing_sections() {
        let rows: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "src/fixtures/original_fixed.json"
        ))
        .unwrap();
        fn compare_numbers(actual: &Value, expected: &Value, path: &str) {
            match (actual, expected) {
                (Value::Number(a), Value::Number(b)) => assert_eq!(
                    a.as_f64().unwrap().to_bits(),
                    b.as_f64().unwrap().to_bits(),
                    "{path}"
                ),
                (Value::Array(a), Value::Array(b)) => {
                    assert_eq!(a.len(), b.len(), "{path}");
                    for (i, (a, b)) in a.iter().zip(b).enumerate() {
                        compare_numbers(a, b, &format!("{path}/{i}"));
                    }
                }
                (Value::Object(a), Value::Object(b)) => {
                    assert_eq!(
                        a.keys().collect::<BTreeSet<_>>(),
                        b.keys().collect::<BTreeSet<_>>(),
                        "{path}"
                    );
                    for (key, b) in b {
                        compare_numbers(&a[key], b, &format!("{path}/{key}"));
                    }
                }
                _ => assert_eq!(actual, expected, "{path}"),
            }
        }
        fn geometry(value: &Value) -> Value {
            match value {
                Value::Object(values) => Value::Object(
                    values
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
                                "sections",
                                "startPoint",
                                "endPoint",
                                "bendPoints",
                            ]
                            .contains(&key.as_str())
                        })
                        .map(|(key, v)| (key.clone(), geometry(v)))
                        .collect(),
                ),
                Value::Array(values) => Value::Array(values.iter().map(geometry).collect()),
                value => value.clone(),
            }
        }
        for row in rows.as_array().unwrap() {
            let mut graph: ElkGraph = serde_json::from_value(row["input"].clone()).unwrap();
            layout(&mut graph, ElkOptions::default()).unwrap();
            compare_numbers(
                &geometry(&serde_json::to_value(graph).unwrap()),
                &row["expected"],
                row["name"].as_str().unwrap(),
            );
        }
    }
}
