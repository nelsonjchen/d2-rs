// Copyright (c) 2010, 2019 Kiel University and others.
// Copyright (c) 2015, 2020 Kiel University and others.
// SPDX-License-Identifier: EPL-2.0
// Source helpers for the elk-go utility providers.

use crate::{ElkGraph, ElkNode, ElkOptions, LayeringError};
use serde_json::Value;
use std::collections::BTreeSet;

pub(crate) fn option<'a>(options: &'a Value, name: &str) -> Option<&'a Value> {
    let full = format!("org.eclipse.elk.{name}");
    options.as_object()?.iter().find_map(|(key, value)| {
        // BySuffix accepts only a unique registry-wide dot suffix. In
        // particular, `padding` also names nodeLabels.padding and is ignored.
        if key == &full {
            return Some(value);
        }
        if !full.strip_suffix(key).is_some_and(|p| p.ends_with('.')) {
            return None;
        }
        let count = include_str!("fixtures/original_option_ids.txt")
            .lines()
            .filter(|id| *id == key || id.strip_suffix(key).is_some_and(|p| p.ends_with('.')))
            .take(2)
            .count();
        (count == 1).then_some(value)
    })
}

pub(crate) fn point(options: &Value, name: &str) -> Option<(f64, f64)> {
    let text = option(options, name)?.as_str()?.trim_matches([
        '(', ')', '[', ']', '{', '}', '"', '\'', ' ', '\t', '\r', '\n',
    ]);
    let parts: Vec<_> = text
        .split([',', ';', '\r', '\n'])
        .filter(|s| !s.is_empty())
        .collect();
    if parts.len() != 2 {
        return None;
    }
    Some((
        d2_rs_core::parse_d2_float(parts[0].trim())?,
        d2_rs_core::parse_d2_float(parts[1].trim())?,
    ))
}

pub(crate) fn flag(options: &Value, name: &str) -> bool {
    option(options, name).is_some_and(|v| {
        v.as_bool() == Some(true)
            || v.as_str()
                .is_some_and(|text| text.eq_ignore_ascii_case("true"))
    })
}

pub(crate) fn padding(options: &Value) -> [f64; 4] {
    let Some(text) = option(options, "padding").and_then(Value::as_str) else {
        return [15.0; 4];
    };
    let mut values = [0.0; 4];
    for item in text.trim_matches(['[', ']']).split(',') {
        if let Some((key, value)) = item.split_once('=') {
            if let Some(index) = ["top", "left", "bottom", "right"]
                .iter()
                .position(|side| *side == key.trim())
            {
                values[index] = d2_rs_core::parse_d2_float(value.trim()).unwrap_or(0.0);
            }
        }
    }
    values
}

pub(crate) fn minimum(options: &Value) -> (f64, f64) {
    let constraints = option(options, "nodeSize.constraints")
        .and_then(Value::as_str)
        .unwrap_or("");
    if !constraints
        .split(|c: char| c.is_whitespace() || "[],".contains(c))
        .any(|c| c == "MINIMUM_SIZE")
    {
        return (0.0, 0.0);
    }
    let (mut x, mut y) = point(options, "nodeSize.minimum").unwrap_or_default();
    let size_options = option(options, "nodeSize.options")
        .and_then(Value::as_str)
        .unwrap_or("DEFAULT_MINIMUM_SIZE");
    if size_options.contains("DEFAULT_MINIMUM_SIZE") {
        if x <= 0.0 {
            x = 20.0;
        }
        if y <= 0.0 {
            y = 20.0;
        }
    }
    (x, y)
}

pub(crate) fn layout_default_children(
    graph: &mut ElkGraph,
    _options: ElkOptions,
) -> Result<BTreeSet<String>, LayeringError> {
    fn ids(node: &ElkNode, result: &mut BTreeSet<String>) {
        result.insert(node.id.clone());
        result.extend(node.ports.iter().map(|p| p.id.clone()));
        for child in &node.children {
            ids(child, result);
        }
    }
    let mut consumed = BTreeSet::new();
    for child in &mut graph.children {
        if child
            .layout_options
            .as_ref()
            .is_some_and(|options| flag(options, "noLayout"))
        {
            continue;
        }
        if child.children.is_empty() {
            continue;
        }
        let mut members = BTreeSet::new();
        ids(child, &mut members);
        let selected: Vec<_> = graph
            .edges
            .iter()
            .enumerate()
            .filter_map(|(i, edge)| {
                (!edge.sources.is_empty()
                    && !edge.targets.is_empty()
                    && edge
                        .sources
                        .iter()
                        .chain(&edge.targets)
                        .all(|id| members.contains(id)))
                .then_some(i)
            })
            .collect();
        let local_count = child.edges.len();
        let mut edges = std::mem::take(&mut child.edges);
        edges.extend(selected.iter().map(|&i| graph.edges[i].clone()));
        let mut scope = ElkGraph {
            children_present: false,
            edges_present: false,
            id: child.id.clone(),
            x: Some(child.x),
            y: Some(child.y),
            width: Some(child.width),
            height: Some(child.height),
            layout_options: child
                .layout_options
                .clone()
                .unwrap_or_else(|| serde_json::json!({})),
            children: std::mem::take(&mut child.children),
            edges,
        };
        // Each unconfigured child resolves the registered Layered provider's
        // defaults, not D2's adapter defaults or the parent's provider profile.
        // Explicit D2 container options in the wire model hydrate these below.
        let child_options = ElkOptions {
            algorithm: crate::ElkAlgorithm::Layered,
            node_spacing: 20.0,
            edge_node_spacing: 10.0,
            edge_edge_spacing: 10.0,
            padding: 12.0,
            self_loop_spacing: 10.0,
        };
        // core.Engine resolves each receiver's own algorithm, falling back
        // to Layered only when that receiver has no explicit selection.
        let algorithm = option(&scope.layout_options, "algorithm")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(crate::ElkAlgorithm::parse)
            .transpose()
            .map_err(|error| LayeringError::Import(error.to_string()))?
            .unwrap_or(crate::ElkAlgorithm::Layered);
        let outcome = match algorithm {
            crate::ElkAlgorithm::Layered => {
                crate::layout_elk_graph_with_layered_pipeline(&mut scope, child_options)
            }
            crate::ElkAlgorithm::Fixed => crate::fixed_layout::layout(&mut scope, child_options),
            crate::ElkAlgorithm::Box => crate::box_layout::layout(&mut scope, child_options),
            crate::ElkAlgorithm::Rectpacking => {
                crate::rectpacking::layout(&mut scope, child_options)
            }
            crate::ElkAlgorithm::Random => {
                crate::layout_elk_graph_with_random_pipeline(&mut scope, child_options)
            }
            crate::ElkAlgorithm::Force => {
                crate::layout_elk_graph_with_force_pipeline(&mut scope, child_options)
            }
            crate::ElkAlgorithm::Stress => {
                crate::layout_elk_graph_with_stress_pipeline(&mut scope, child_options)
            }
            crate::ElkAlgorithm::SporeOverlap => {
                crate::layout_elk_graph_with_spore_overlap_pipeline(&mut scope, child_options)
            }
            crate::ElkAlgorithm::SporeCompaction => {
                crate::layout_elk_graph_with_spore_compaction_pipeline(&mut scope, child_options)
            }
            crate::ElkAlgorithm::MrTree => {
                crate::layout_elk_graph_with_mrtree_pipeline(&mut scope, child_options)
            }
            crate::ElkAlgorithm::Radial => {
                crate::layout_elk_graph_with_radial_pipeline(&mut scope, child_options)
            }
        };
        if outcome.is_ok() {
            consumed.insert(child.id.clone());
        }
        child.width = scope.width.unwrap_or(child.width);
        child.height = scope.height.unwrap_or(child.height);
        child.children = scope.children;
        let transferred = scope.edges.split_off(local_count);
        child.edges = scope.edges;
        for (&index, edge) in selected.iter().zip(transferred) {
            graph.edges[index] = edge;
        }
        outcome?;
    }
    Ok(consumed)
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod registry_tests {
    use super::*;
    #[test]
    #[cfg(feature = "oracle-fixtures")]
    fn original_go_registry_suffix_resolution() {
        let rows: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "src/fixtures/original_option_resolution.json"
        ))
        .unwrap();
        for (key, expected) in rows.as_object().unwrap() {
            let candidate = expected
                .as_str()
                .or_else(|| {
                    include_str!("fixtures/original_option_ids.txt")
                        .lines()
                        .find(|id| id.strip_suffix(key).is_some_and(|p| p.ends_with('.')))
                })
                .unwrap_or("org.eclipse.elk.padding");
            let name = candidate.strip_prefix("org.eclipse.elk.").unwrap();
            assert_eq!(
                option(&serde_json::json!({key:true}), name).is_some(),
                expected.is_string(),
                "{key}: {expected}"
            );
        }
    }
}
