// SPDX-License-Identifier: MPL-2.0
//! Parsed near keys and validateNear object guards from Go D2 v0.9.0.
use super::edge_creation::error;
use d2_rs_core::{
    CanvasPosition, Graph, NodeId,
    ast::{self, Child, Node as AstNode, ScalarBox, Shared, keywords},
    ir::go_lowercase,
};
use std::collections::BTreeMap;

struct NearKey {
    source: Child,
    key: ast::KeyPath,
    parts: Vec<String>,
}

fn constant(parts: &[String]) -> Option<CanvasPosition> {
    Some(match parts.first()?.as_str() {
        "top-left" => CanvasPosition::TopLeft,
        "top-center" => CanvasPosition::TopCenter,
        "top-right" => CanvasPosition::TopRight,
        "center-left" => CanvasPosition::CenterLeft,
        "center-right" => CanvasPosition::CenterRight,
        "bottom-left" => CanvasPosition::BottomLeft,
        "bottom-center" => CanvasPosition::BottomCenter,
        "bottom-right" => CanvasPosition::BottomRight,
        _ => return None,
    })
}

// HasChild walks canonical components case-insensitively, with the source's
// terminal reserved-key behavior. Some(None) denotes the implicit graph root.
fn resolve(names: &BTreeMap<Vec<u8>, NodeId>, parts: &[String]) -> Option<Option<NodeId>> {
    let mut current = None;
    for (index, part) in parts.iter().enumerate() {
        if index + 1 == parts.len() && part != "style" && keywords::is_reserved_keyword(part) {
            return Some(current);
        }
        current = Some(*names.get(&go_lowercase(parts[..=index].join(".").as_bytes()))?);
    }
    Some(current)
}

fn descendant_of(graph: &Graph, mut node: NodeId, ancestor: NodeId) -> bool {
    loop {
        if node == ancestor {
            return true;
        }
        let Some(parent) = graph.node(node).and_then(|node| node.parent) else {
            return false;
        };
        node = parent;
    }
}

fn in_sequence(graph: &Graph, node: NodeId) -> bool {
    let mut parent = graph.node(node).and_then(|node| node.parent);
    while let Some(id) = parent {
        if graph.is_sequence_diagram(id) {
            return true;
        }
        parent = graph.node(id).and_then(|node| node.parent);
    }
    graph.root_sequence_diagram
}

fn in_grid(graph: &Graph, mut node: NodeId) -> bool {
    loop {
        let object = graph.node(node).expect("resolved near object");
        if object.grid_rows.is_some() || object.grid_columns.is_some() {
            return true;
        }
        let Some(parent) = object.parent else {
            return graph.root_grid_rows.is_some() || graph.root_grid_columns.is_some();
        };
        node = parent;
    }
}

pub(super) fn apply_and_validate(
    graph: &mut Graph,
    // compileMap assignment order, including root attributes and classes.
    inputs: &[(Option<NodeId>, ScalarBox)],
) -> Vec<ast::Error> {
    let names = graph
        .nodes
        .iter()
        .enumerate()
        .map(|(index, node)| {
            (
                go_lowercase(node.external_id.as_bytes()),
                NodeId(index as u32),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut errors = Vec::new();
    let mut keys = BTreeMap::new();
    for (id, scalar) in inputs {
        let source = scalar.unbox().expect("effective near scalar");
        let bytes = scalar.scalar_bytes();
        match crate::parse_key(&bytes) {
            Ok(mut key) => {
                key.range = source.range();
                let parts = crate::format::key_path(&key)
                    .expect("parsed near key has components")
                    .into_iter()
                    .map(|part| String::from_utf8_lossy(&part).into_owned())
                    .collect();
                if let Some(id) = id {
                    keys.insert(*id, NearKey { source, key, parts });
                }
            }
            Err(cause) => {
                let diagnostic = error(
                    &source,
                    format!(
                        "bad near key {}: {cause}",
                        crate::parser::quote_input(&bytes)
                    ),
                );
                if !errors.contains(&diagnostic) {
                    errors.push(diagnostic);
                }
            }
        }
    }
    let target_near_constants = keys
        .iter()
        .filter_map(|(id, key)| constant(&key.parts).map(|_| *id))
        .collect::<std::collections::BTreeSet<_>>();
    for (id, key) in keys {
        let resolved = resolve(&names, &key.parts);
        let position = constant(&key.parts);
        let node = graph.node_mut(id).expect("near owner");
        node.d2_near_key = Some(key.parts.clone());
        // IsConstantNear checks an actual object match before the constants.
        node.near = resolved.flatten();
        node.canvas_position = resolved.is_none().then_some(position).flatten();
        let message = match resolved {
            Some(None) => Some("near keys cannot be set to an ancestor".to_owned()),
            Some(Some(target)) => {
                if descendant_of(graph, id, target) {
                    Some("near keys cannot be set to an ancestor".to_owned())
                } else if descendant_of(graph, target, id) {
                    Some("near keys cannot be set to an descendant".to_owned())
                } else if in_sequence(graph, target) {
                    Some("near keys cannot be set to an object within sequence diagrams".to_owned())
                } else if target_near_constants.contains(&target) {
                    Some("near keys cannot be set to an object with a constant near key".to_owned())
                } else if in_grid(graph, target) {
                    Some("near keys cannot be set to descendants of special objects, like grid cells".to_owned())
                } else {
                    None
                }
            }
            None if position.is_some() => graph
                .node(id)
                .and_then(|node| node.parent)
                .map(|_| "constant near keys can only be set on root level shapes".to_owned()),
            None => {
                let formatted = crate::format::format(&Child::KeyPath(Shared::new(key.key)));
                Some(format!(
                    "near key {} must be the absolute path to a shape or one of the following constants: {}",
                    crate::parser::quote_input(&formatted),
                    keywords::NEAR_CONSTANTS.join(", ")
                ))
            }
        };
        if let Some(message) = message {
            let diagnostic = error(&key.source, message);
            if !errors.contains(&diagnostic) {
                errors.push(diagnostic);
            }
        }
    }
    errors
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::{GraphCompileError, compile_ir, lower_graph};

    fn compile(source: &str) -> Result<Graph, GraphCompileError> {
        let parsed = crate::parse("near-validation.d2", source.as_bytes(), None);
        assert!(parsed.error.is_none(), "{:?}", parsed.error);
        let (compiler, root) = compile_ir(Shared::new(parsed.map)).unwrap();
        let graph = lower_graph(&compiler, root);
        assert_eq!(
            crate::compiler::board_is_renderable(&compiler, root),
            graph
                .as_ref()
                .map(|graph| !graph.nodes.is_empty())
                .map_err(Clone::clone),
            "planning must preserve near diagnostics: {source}"
        );
        graph
    }

    fn diagnostics(source: &str) -> Vec<ast::Error> {
        match compile(source) {
            Err(GraphCompileError::InvalidAttributes(errors)) => errors,
            other => panic!("expected graph diagnostics, got {other:?}"),
        }
    }

    #[test]
    fn near_guards_follow_source_priority_and_scalar_ranges() {
        for (source, message) in [
            ("a.near: a\n", "near keys cannot be set to an ancestor"),
            (
                "a: {b: {near: a}}\n",
                "near keys cannot be set to an ancestor",
            ),
            (
                "a: {near: a.b; b}\n",
                "near keys cannot be set to an descendant",
            ),
            (
                "a: {b: {near: top-left}}\n",
                "constant near keys can only be set on root level shapes",
            ),
            (
                "seq: {shape: sequence_diagram; a}\nb.near: seq.a\n",
                "near keys cannot be set to an object within sequence diagrams",
            ),
            (
                "shape: sequence_diagram\na\nb.near: a\n",
                "near keys cannot be set to an object within sequence diagrams",
            ),
            (
                "a.near: top-left\nb.near: a\n",
                "near keys cannot be set to an object with a constant near key",
            ),
            (
                "grid: {grid-columns: 1; a}\nb.near: grid\n",
                "near keys cannot be set to descendants of special objects, like grid cells",
            ),
            (
                "grid-columns: 1\na\nb.near: a\n",
                "near keys cannot be set to descendants of special objects, like grid cells",
            ),
            ("a.near: width\n", "near keys cannot be set to an ancestor"),
        ] {
            let errors = diagnostics(source);
            assert_eq!(errors.len(), 1, "{source}: {errors:?}");
            assert!(errors[0].message.ends_with(message), "{errors:?}");
        }
        let errors = diagnostics("a.near: a\n");
        assert_eq!(errors[0].range.start.line, 0);
        assert_eq!(errors[0].range.start.column, 8);
        let errors = diagnostics("a.near: missing\n");
        assert!(errors[0].message.ends_with(&format!(
            "near key \"missing\" must be the absolute path to a shape or one of the following constants: {}",
            keywords::NEAR_CONSTANTS.join(", ")
        )));
    }

    #[test]
    fn near_resolution_uses_canonical_absolute_keys_and_go_case_folding() {
        for source in [
            "Target\na.near: tArGeT\n",
            "İ\na.near: i\n",
            "\"图. 名\"\na.near: '\"图. 名\"'\n",
            "target\na.near: \"target: ignored\"\n",
            "target\na.near: target.width\n",
            "target\na.near: target.stroke\n",
        ] {
            let graph = compile(source).unwrap();
            assert_eq!(graph.nodes[1].near, Some(NodeId(0)), "{source}");
            assert_eq!(graph.nodes[1].canvas_position, None, "{source}");
        }
        let graph = compile("outer: {a; b: {near: outer.a}}\n").unwrap();
        assert_eq!(graph.nodes[2].near, Some(NodeId(1)));
        let errors = diagnostics("outer: {a; b: {near: a}}\n");
        assert!(
            errors[0]
                .message
                .contains("near key \"a\" must be the absolute path")
        );
    }

    #[test]
    fn constant_near_uses_parsed_first_component_and_object_shadowing() {
        for value in ["top-left", "top-left.missing", "'\"top-left\"'"] {
            let graph = compile(&format!("a.near: {value}\n")).unwrap();
            assert_eq!(
                graph.nodes[0].canvas_position,
                Some(CanvasPosition::TopLeft)
            );
            assert_eq!(graph.nodes[0].near, None);
        }
        let graph = compile("top-left\na.near: top-left\na -> a.child\n").unwrap();
        assert_eq!(graph.nodes[1].near, Some(NodeId(0)));
        assert_eq!(graph.nodes[1].canvas_position, None);
        // validateNear's target guard checks NearKey's first component even
        // when IsConstantNear would be false due to an existing object.
        let errors = diagnostics("top-left\na.near: top-left\nb.near: a\n");
        assert!(
            errors[0]
                .message
                .ends_with("near keys cannot be set to an object with a constant near key")
        );
    }

    #[test]
    fn malformed_keys_and_shared_diagnostics_preserve_source_errors() {
        for (value, detail) in [
            ("\"\"", "empty key: \"\""),
            ("'\"unfinished'", "failed to parse key"),
        ] {
            let errors = diagnostics(&format!("a.near: {value}\n"));
            assert_eq!(errors.len(), 1);
            assert!(errors[0].message.contains("bad near key"));
            assert!(errors[0].message.contains(detail));
        }
        for source in [
            "classes: {bad: {near: missing}}\na.class: bad\nb.class: bad\n",
            "vars: {shared: {near: missing}}\none: ${shared}\ntwo: ${shared}\n",
        ] {
            let errors = diagnostics(source);
            assert_eq!(errors.len(), 1, "{errors:?}");
            assert_eq!(errors[0].range.start.line, 0);
        }
        let errors = diagnostics(
            "a: {near: missing; child}\ngrid: {grid-columns: 1; cell}\ngrid -> grid.cell\n",
        );
        assert_eq!(errors.len(), 2);
        assert!(errors[0].message.contains("near key \"missing\""));
        assert!(
            errors[1]
                .message
                .ends_with("edge from grid diagram \"grid\" cannot enter itself")
        );
    }

    #[test]
    fn root_and_overridden_class_keys_keep_compile_map_errors() {
        let errors = diagnostics("near: \"\"\na\n");
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].range.start.column, 6);
        assert!(
            errors[0]
                .message
                .ends_with("bad near key \"\": empty key: \"\"")
        );
        let errors = diagnostics("classes: {c: {near: \"\"}}\na: {class: c; near: b}\nb\n");
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].range.start.line, 0);
        assert!(errors[0].message.contains("bad near key"));
        let errors = diagnostics("a.near: \"\"\nnear: \"\"\nb.near: missing\n");
        assert_eq!(
            errors
                .iter()
                .map(|error| error.range.start.line)
                .collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        // Failed direct parsing leaves the previously assigned class NearKey.
        let errors = diagnostics("classes: {c: {near: missing}}\na: {class: c; near: \"\"}\n");
        assert_eq!(errors.len(), 2);
        assert!(errors[0].message.contains("bad near key"));
        assert!(errors[1].message.contains("near key \"missing\" must"));
    }
}
