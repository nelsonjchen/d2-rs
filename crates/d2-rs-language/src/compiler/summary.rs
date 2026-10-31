// SPDX-License-Identifier: MPL-2.0
//! Stable JSON projection of the translated IR.
//!
//! The IR arena deliberately keeps its storage private.  This projection is
//! the inspection boundary used by the native command and parity harnesses;
//! it preserves source order and the same absolute paths used by the Go
//! compiler oracle without exposing arena indexes as a public API.

use super::Compiler;
use crate::{format, ir};
use d2_rs_core::{ast, ir::EdgeReference, ir::FieldReference, ir::Node, ir::NodeId};
use serde_json::{Value, json};

fn node_path(compiler: &Compiler, node: NodeId) -> Option<String> {
    let parts = ir::ida(&compiler.arena, node)?;
    if parts.iter().any(Option::is_none) {
        return None;
    }
    let path = parts
        .into_iter()
        .flatten()
        .map(|part| {
            ast::raw_string_box(ast::ScalarBox::from_child(Some(part)).scalar_bytes(), true)
        })
        .collect::<Vec<_>>();
    Some(String::from_utf8_lossy(&format::format_key_path(&path)).into_owned())
}

fn reference_summary(compiler: &Compiler, reference: &ast::Shared<FieldReference>) -> Value {
    let reference = reference.read();
    let context = reference.context.as_ref().map(|context| context.read());
    json!({
        "key": context.as_ref().and_then(|context| context.key.as_ref()).map(|key| {
            String::from_utf8_lossy(&crate::format::format(&ast::Child::Key(key.clone()))).into_owned()
        }),
        "string": reference.string,
        "glob": reference.due_to_glob,
        "lazy": reference.due_to_lazy_glob,
        "scope": context.as_ref().and_then(|context| context.scope_map).and_then(|scope| node_path(compiler, scope)),
    })
}

fn edge_reference_summary(compiler: &Compiler, reference: &ast::Shared<EdgeReference>) -> Value {
    let reference = reference.read();
    let context = reference.context.as_ref().map(|context| context.read());
    json!({
        "key": context.as_ref().and_then(|context| context.key.as_ref()).map(|key| {
            String::from_utf8_lossy(&crate::format::format(&ast::Child::Key(key.clone()))).into_owned()
        }),
        "edge": context.as_ref().and_then(|context| context.edge.clone()),
        "glob": reference.due_to_glob,
        "lazy": reference.due_to_lazy_glob,
        "scope": context.as_ref().and_then(|context| context.scope_map).and_then(|scope| node_path(compiler, scope)),
    })
}

fn visit(compiler: &Compiler, node: NodeId, rows: &mut Vec<Value>) {
    if compiler.arena.is_nil_scalar(node) {
        rows.push(json!({"path": null, "kind": "scalar", "nil": true}));
        return;
    }

    let mut row = json!({"path": node_path(compiler, node)});
    let children = match compiler.arena.node(node) {
        Node::Map(map) => {
            row["kind"] = json!("map");
            row["version"] = json!(map.structure_version);
            row["globs"] = json!(map
                .globs
                .iter()
                .flat_map(ast::SharedSlice::iter)
                .map(|id| {
                    let glob = compiler.arena.glob(id.read().unwrap());
                    let context = glob.context.as_ref().unwrap().read();
                    let mut fields = glob
                        .applied_fields
                        .as_ref()
                        .map(|fields| fields.read().clone())
                        .into_iter()
                        .flatten()
                        .map(|field| ast::ByteString::from(field.clone()).to_text().into_owned())
                        .collect::<Vec<_>>();
                    fields.sort();
                    let mut edges = glob
                        .applied_edges
                        .as_ref()
                        .map(|edges| edges.read().clone())
                        .into_iter()
                        .flatten()
                        .map(|edge| ast::ByteString::from(edge).to_text().into_owned())
                        .collect::<Vec<_>>();
                    edges.sort();
                    json!({
                        "edges": edges,
                        "key": context.key.as_ref().map(|key| String::from_utf8_lossy(&crate::format::format(&ast::Child::Key(key.clone()))).into_owned()),
                        "scope": context.scope_map.and_then(|scope| node_path(compiler, scope)),
                        "fields": fields,
                    })
                })
                .collect::<Vec<_>>());
            map.fields
                .iter()
                .flatten()
                .chain(map.edges.iter().flatten())
                .copied()
                .collect::<Vec<_>>()
        }
        Node::Field(field) => {
            row["kind"] = json!("field");
            row["suspended"] = json!(field.suspended);
            row["refs"] = json!(
                field
                    .references
                    .iter()
                    .flatten()
                    .map(|reference| reference_summary(compiler, reference))
                    .collect::<Vec<_>>()
            );
            field
                .primary
                .into_iter()
                .chain(field.composite)
                .collect::<Vec<_>>()
        }
        Node::Array(array) => {
            row["kind"] = json!("array");
            array.values.clone().unwrap_or_default()
        }
        Node::Scalar(_) => {
            row["kind"] = json!("scalar");
            Vec::new()
        }
        Node::Edge(edge) => {
            row["kind"] = json!("edge");
            row["id"] = json!(edge.id);
            row["suspended"] = json!(edge.suspended);
            row["refs"] = json!(
                edge.references
                    .iter()
                    .flatten()
                    .map(|reference| edge_reference_summary(compiler, reference))
                    .collect::<Vec<_>>()
            );
            edge.primary.into_iter().chain(edge.map).collect::<Vec<_>>()
        }
    };
    rows.push(row);
    for child in children {
        visit(compiler, child, rows);
    }
}

/// Returns the source-ordered, oracle-compatible JSON projection of an IR
/// tree.  This is diagnostic data; it is intentionally separate from the
/// layout graph and does not claim that compilation or rendering is complete.
pub fn summarize_ir(compiler: &Compiler, root: NodeId) -> Value {
    let mut nodes = Vec::new();
    visit(compiler, root, &mut nodes);
    json!({
        "nodes": nodes,
        "errors": compiler.errors.clone(),
    })
}
