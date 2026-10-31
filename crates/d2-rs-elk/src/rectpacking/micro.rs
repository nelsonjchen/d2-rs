// Copyright (c) 2014, 2015 Kiel University and others.
// Copyright (c) 2017, 2019 Kiel University and others.
// SPDX-License-Identifier: EPL-2.0
// elk-go internal/common/nodespacing/node_label_and_size_calculator.go.
use crate::{
    ElkGraph,
    utility::{minimum, option},
};
use serde_json::Value;
use std::collections::BTreeSet;
fn number(value: Option<&Value>, default: f64) -> f64 {
    value
        .and_then(|v| v.as_f64().or_else(|| v.as_str()?.parse().ok()))
        .unwrap_or(default)
}
fn flag(value: Option<&Value>) -> bool {
    value.is_some_and(|v| {
        v.as_bool() == Some(true) || v.as_str().is_some_and(|s| s.eq_ignore_ascii_case("true"))
    })
}
fn has(text: &str, key: &str) -> bool {
    text.split(|c: char| c.is_whitespace() || "[],".contains(c))
        .any(|token| token == key)
}
pub(crate) fn apply(graph: &mut ElkGraph, consumed: &BTreeSet<String>) {
    let parent_fixed = flag(option(&graph.layout_options, "nodeSize.fixedGraphSize"))
        || flag(option(&graph.layout_options, "topdownLayout"));
    let parent_spacing = number(option(&graph.layout_options, "spacing.labelNode"), 5.0);
    for node in &mut graph.children {
        let options = node.layout_options.as_ref().unwrap_or(&Value::Null);
        let constraints = if consumed.contains(&node.id) {
            ""
        } else {
            option(options, "nodeSize.constraints")
                .and_then(Value::as_str)
                .unwrap_or("")
        };
        let tokens: Vec<_> = constraints
            .split(|c: char| c.is_whitespace() || "[],".contains(c))
            .filter(|s| !s.is_empty())
            .collect();
        let placement = option(options, "nodeLabels.placement")
            .and_then(Value::as_str)
            .unwrap_or("");
        if !tokens.is_empty() && !(tokens.len() == 1 && tokens[0] == "PORT_LABELS") {
            let (mut width, mut height) = (0.0_f64, 0.0_f64);
            if has(constraints, "NODE_LABELS") {
                let spacing = number(option(options, "spacing.labelNode"), parent_spacing);
                for label in &node.labels {
                    let placement = label
                        .layout_options
                        .as_ref()
                        .and_then(|o| option(o, "nodeLabels.placement"))
                        .and_then(Value::as_str)
                        .unwrap_or(placement);
                    if !has(placement, "OUTSIDE") {
                        width = width.max(label.width + 2.0 * spacing);
                        height = height.max(label.height + 2.0 * spacing);
                    }
                }
            }
            if has(constraints, "MINIMUM_SIZE") {
                let (w, h) = minimum(options);
                width = width.max(0.0_f64.max(w));
                height = height.max(0.0_f64.max(h));
            }
            if parent_fixed {
                width = width.max(node.width);
                height = height.max(node.height);
            }
            node.width = width;
            node.height = height;
        }
        if option(options, "portConstraints").is_some_and(|v| v == "FIXED_POS") {
            for port in &mut node.ports {
                let offset = number(option(&port.layout_options, "port.borderOffset"), 0.0);
                match option(&port.layout_options, "port.side")
                    .and_then(Value::as_str)
                    .unwrap_or("UNDEFINED")
                {
                    "NORTH" => port.y = -port.height - offset,
                    "SOUTH" => port.y = node.height + offset,
                    "EAST" => port.x = node.width + offset,
                    "WEST" => port.x = -port.width - offset,
                    _ => {}
                }
            }
        }
        for label in &mut node.labels {
            let position = label
                .layout_options
                .as_ref()
                .and_then(|o| option(o, "nodeLabels.placement"))
                .and_then(Value::as_str)
                .unwrap_or(placement);
            if position
                .trim_matches(|c: char| c.is_whitespace() || "[],".contains(c))
                .is_empty()
            {
                continue;
            }
            label.x = if has(position, "H_LEFT") {
                0.0
            } else if has(position, "H_RIGHT") {
                node.width - label.width
            } else {
                (node.width - label.width) / 2.0
            };
            label.y = if has(position, "V_TOP") {
                0.0
            } else if has(position, "V_BOTTOM") {
                node.height - label.height
            } else {
                (node.height - label.height) / 2.0
            };
        }
    }
}
