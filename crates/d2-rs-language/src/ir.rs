// SPDX-License-Identifier: MPL-2.0
//! IR formatting helpers. IR data remains independent of the language formatter.
use d2_rs_core::{
    ast,
    ir::{Arena, Node, NodeId},
};
/// Equivalent to the IR node's String method, retaining arbitrary output bytes.
pub fn format(arena: &Arena, id: NodeId) -> Vec<u8> {
    arena
        .ast(id)
        .map_or_else(Vec::new, |node| crate::format::format(&node))
}
/// Canonical edge identity omits values, includes a concrete index, and does
/// not print the EdgeId's glob flag, matching upstream IDString.
pub fn edge_id_string(arena: &Arena, id: NodeId) -> ast::UnquotedString {
    let Node::Edge(edge) = arena.node(id) else {
        panic!("expected IR edge")
    };
    let ast::Child::Key(key) = arena.ast(id).expect("nil edge AST") else {
        unreachable!()
    };
    {
        let mut key = key.write();
        if let Some(index) = &edge.id.as_ref().expect("nil edge identifier").read().index {
            key.edge_index = Some(ast::Shared::new(ast::EdgeIndex {
                int: Some(index.clone()),
                ..Default::default()
            }));
        }
        key.primary = ast::ScalarBox::default();
        key.value = ast::ValueBox::default();
    }
    let bytes = crate::format::format(&ast::Child::Key(key));
    ast::UnquotedString::flat(bytes)
}

/// Absolute IR path, including the synthetic root field.
pub fn ida(arena: &Arena, node: NodeId) -> Option<Vec<Option<ast::Child>>> {
    path(arena, node, false)
}
/// IR path local to the nearest board, excluding its root field.
pub fn board_ida(arena: &Arena, node: NodeId) -> Option<Vec<Option<ast::Child>>> {
    path(arena, node, true)
}
fn path(arena: &Arena, mut node: NodeId, board: bool) -> Option<Vec<Option<ast::Child>>> {
    let mut parts = Vec::new();
    loop {
        match arena.node(node) {
            Node::Field(f) => {
                if board && (arena.root(node) || !arena.board_kind(node).is_empty()) {
                    break;
                }
                parts.push(f.name.unbox());
                if arena.root(node) {
                    break;
                }
            }
            Node::Edge(_) => parts.push(Some(ast::Child::Unquoted(ast::Shared::new(
                edge_id_string(arena, node),
            )))),
            _ => {}
        }
        let Some(parent) = arena.parent(node) else {
            break;
        };
        node = parent;
    }
    parts.reverse();
    (!parts.is_empty()).then_some(parts)
}
/// Relative IR paths use the full formatted edge, including values, matching
/// upstream RelIDA rather than the IDString used for absolute/board paths.
pub fn relative_ida(
    arena: &Arena,
    parent: Option<NodeId>,
    mut node: NodeId,
) -> Option<Vec<Option<ast::Child>>> {
    let mut parts = Vec::new();
    loop {
        match arena.node(node) {
            Node::Field(f) => {
                parts.push(f.name.unbox());
                if arena.root(node) {
                    break;
                }
            }
            Node::Edge(_) => parts.push(Some(ast::Child::Unquoted(ast::Shared::new(
                ast::UnquotedString::flat(format(arena, node)),
            )))),
            _ => {}
        }
        let Some(next) = arena.parent(node) else {
            break;
        };
        node = next;
        let stop = match arena.node(node) {
            // Composite is an interface in Go: its nil value equals a nil parent.
            Node::Field(f) => arena.root(node) || Some(node) == parent || f.composite == parent,
            Node::Edge(e) => Some(node) == parent || (parent.is_some() && e.map == parent),
            _ => false,
        };
        if stop {
            break;
        }
    }
    parts.reverse();
    (!parts.is_empty()).then_some(parts)
}
