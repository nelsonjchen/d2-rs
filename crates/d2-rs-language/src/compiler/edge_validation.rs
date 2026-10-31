// SPDX-License-Identifier: MPL-2.0
//! Edge guards in d2compiler.validateNear/validateEdges from Go D2 v0.9.0.
use super::{Compiler, edge_creation::error};
use d2_rs_core::{
    EdgeId, Graph, NodeId, ast,
    ir::{Node, NodeId as IrNodeId},
};

fn descendant_of(graph: &Graph, mut node: NodeId, ancestor: NodeId) -> bool {
    // Every physical object belongs beneath its synthetic graph root.
    if ancestor == NodeId::ROOT {
        return graph.root_object.is_some() && graph.endpoint_node(node).is_some();
    }
    // Object.IsDescendantOf includes the receiver itself.
    loop {
        if node == ancestor {
            return true;
        }
        let Some(parent) = graph.endpoint_node(node).and_then(|node| node.parent) else {
            return false;
        };
        node = parent;
    }
}

fn is_grid(graph: &Graph, node: NodeId) -> bool {
    graph
        .endpoint_node(node)
        .is_some_and(|node| node.grid_rows.is_some() || node.grid_columns.is_some())
}

fn is_grid_cell(graph: &Graph, node: NodeId) -> bool {
    if node == NodeId::ROOT {
        return false;
    }
    graph.endpoint_node(node).is_some_and(|node| {
        node.parent.map_or_else(
            || graph.root_grid_rows.is_some() || graph.root_grid_columns.is_some(),
            |parent| is_grid(graph, parent),
        )
    })
}

pub(super) fn validate_edge_boundaries(
    compiler: &Compiler,
    graph: &Graph,
    // Go validates the compileMap insertion order before SortEdgesByAST.
    edges: &[(IrNodeId, EdgeId)],
) -> Vec<ast::Error> {
    let mut errors = Vec::new();
    // validateNear's edge loop precedes validateEdges and may report a
    // distinct error for the same range (e.g. a constant-near grid).
    for kinds in [
        &["constant near"][..],
        &["grid diagram", "grid cell", "sequence diagram"][..],
    ] {
        for &(origin, id) in edges {
            let edge = &graph.edges[id.0 as usize];
            let mut violation = None;
            'checks: for &kind in kinds {
                for (owner, other) in [(edge.source, edge.target), (edge.target, edge.source)] {
                    let special = match kind {
                        "constant near" => graph
                            .endpoint_node(owner)
                            .is_some_and(|node| node.canvas_position.is_some()),
                        "grid diagram" => is_grid(graph, owner),
                        "grid cell" => is_grid_cell(graph, owner),
                        "sequence diagram" => graph.is_sequence_diagram(owner),
                        _ => unreachable!(),
                    };
                    if special && descendant_of(graph, other, owner) {
                        violation = Some((kind, owner));
                        break 'checks;
                    }
                }
            }
            let Some((kind, owner)) = violation else {
                continue;
            };
            let Node::Edge(source_edge) = compiler.arena.node(origin) else {
                continue;
            };
            let source = source_edge
                .references
                .as_ref()
                .and_then(|references| references.first())
                .and_then(|reference| reference.read().context.clone())
                .and_then(|context| context.read().edge.clone())
                .expect("compiled source edge has its first AST reference");
            let name = crate::parser::quote_input(
                graph
                    .endpoint_node(owner)
                    .expect("resolved edge owner")
                    .external_id
                    .as_bytes(),
            );
            let diagnostic = error(
                &ast::Child::Edge(source),
                format!("edge from {kind} {name} cannot enter itself"),
            );
            // compiler.errorf deduplicates the complete range/message pair.
            if !errors.contains(&diagnostic) {
                errors.push(diagnostic);
            }
        }
    }
    errors
}

#[cfg(test)]
mod tests {
    use crate::{
        compiler::{
            compile_ir,
            graph::{GraphCompileError, lower_graph},
        },
        parse,
    };
    use d2_rs_core::ast::{Error, Shared};

    fn diagnostics(source: &str) -> Vec<Error> {
        let parsed = parse("edge-validation.d2", source.as_bytes(), None);
        assert!(parsed.error.is_none(), "{:?}", parsed.error);
        let (compiler, root) = compile_ir(Shared::new(parsed.map)).unwrap();
        let graph = lower_graph(&compiler, root);
        assert_eq!(
            crate::compiler::board_is_renderable(&compiler, root),
            graph
                .as_ref()
                .map(|graph| !graph.nodes.is_empty())
                .map_err(Clone::clone),
            "planning must preserve edge diagnostics: {source}"
        );
        match graph {
            Ok(_) => Vec::new(),
            Err(GraphCompileError::InvalidAttributes(errors)) => errors,
            Err(error) => panic!("unexpected graph error: {error}"),
        }
    }

    #[test]
    fn special_ancestor_edges_reject_both_directions_and_self_loops() {
        for (declaration, owner, descendant, kind) in [
            (
                "grid-columns: 1\ncell: {child}",
                "cell",
                "cell.child",
                "grid cell",
            ),
            (
                "grid: {grid-columns: 1; child}",
                "grid",
                "grid.child",
                "grid diagram",
            ),
            (
                "seq: {shape: sequence_diagram; child}",
                "seq",
                "seq.child",
                "sequence diagram",
            ),
            (
                "owner: {near: top-left; child}",
                "owner",
                "owner.child",
                "constant near",
            ),
        ] {
            for (source, target) in [(owner, descendant), (descendant, owner), (owner, owner)] {
                let errors = diagnostics(&format!("{declaration}\n{source} -> {target}\n"));
                assert_eq!(errors.len(), 1, "{errors:?}");
                assert!(
                    errors[0]
                        .message
                        .ends_with(&format!("edge from {kind} \"{owner}\" cannot enter itself")),
                    "{errors:?}"
                );
            }
        }
    }

    #[test]
    fn diagnostics_keep_graph_insertion_order_phase_order_and_deduplication() {
        let errors =
            diagnostics("grid-columns: 2\na -> a\nbox: {\n grid-columns: 1\n b -> b\n}\na -> a\n");
        assert_eq!(
            errors
                .iter()
                .map(|e| e.range.start.line)
                .collect::<Vec<_>>(),
            vec![4, 1, 6]
        );
        let errors = diagnostics(
            "owner: {near: top-center; grid-columns: 1; child}\nowner -> owner.child\n",
        );
        assert_eq!(errors.len(), 2);
        assert!(
            errors[0]
                .message
                .ends_with("edge from constant near \"owner\" cannot enter itself")
        );
        assert!(
            errors[1]
                .message
                .ends_with("edge from grid diagram \"owner\" cannot enter itself")
        );
        assert_eq!(errors[0].range, errors[1].range);
        let errors = diagnostics("grid-columns: 1\ncell: {a; b}\ncell -> cell.*\n");
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].range.start.line, 2);
        assert_eq!(errors[0].range.start.column, 0);
    }

    #[test]
    fn shared_composite_maps_validate_every_graph_use_site() {
        let errors = diagnostics(
            "vars: {shared: {grid-columns: 1; cell: {child}; cell -> cell.child}}\none: ${shared}\ntwo: ${shared}\n",
        );
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert_eq!(errors[0].range, errors[1].range);
        assert_eq!(errors[0].range.start.line, 0);
        assert_eq!(errors[0].range.start.column, 48);
        for (error, owner) in errors.iter().zip(["one.cell", "two.cell"]) {
            assert!(
                error.message.ends_with(&format!(
                    "edge from grid cell \"{owner}\" cannot enter itself"
                )),
                "{errors:?}"
            );
        }
    }

    #[test]
    fn permitted_edges_and_actor_loops_remain_valid() {
        for source in [
            "cell: {child}\ncell -> cell.child\n",
            "cell -> cell\n",
            "grid: {grid-columns: 1; child}\ngrid -> outside\n",
            "grid-columns: 2\na -> b\n",
            "seq: {shape: sequence_diagram; a -> a}\nseq -> outside\n",
        ] {
            assert!(diagnostics(source).is_empty(), "{source}");
        }
    }
}
