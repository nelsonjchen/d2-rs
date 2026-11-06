// SPDX-License-Identifier: MPL-2.0
use d2_rs_core::{
    ast::{self, Shared},
    ir::*,
};
use serde_json::{Value, json};
#[path = "../ast-clone-oracle.rs"]
#[allow(dead_code)]
mod ast_oracle;
pub fn evaluate(arena: &mut Arena, nodes: &[NodeId], root: NodeId) -> Value {
    let marker = ast::raw_string_box("import", false).unbox();
    for id in nodes {
        match arena.node_mut(*id) {
            Node::Map(m) => {
                m.import_ast = marker.clone();
                m.structure_version = 9;
            }
            Node::Field(f) => {
                f.import_ast = marker.clone();
                f.references
                    .get_or_insert_default()
                    .push(Shared::new(FieldReference {
                        string: f.name.unbox(),
                        context: Some(Shared::new(RefContext {
                            scope_map: Some(root),
                            ..Default::default()
                        })),
                        ..Default::default()
                    }));
            }
            Node::Edge(e) => {
                e.import_ast = marker.clone();
                e.references = Some(vec![Shared::new(EdgeReference {
                    context: Some(Shared::new(RefContext {
                        scope_map: Some(root),
                        ..Default::default()
                    })),
                    ..Default::default()
                })]);
            }
            _ => {}
        }
    }
    let mut observations = Vec::new();
    for src in nodes {
        for parent in [None, Some(root)] {
            let dst = arena.copy(*src, parent);
            let mut pairs = Vec::new();
            pair(arena, *src, dst, parent, &mut pairs);
            observations.push(json!({"pairs":pairs,"ast":ast_oracle::observations(&arena.ast(*src).unwrap(),&arena.ast(dst).unwrap())}));
        }
    }
    json!(observations)
}
fn alias(a: &Option<ast::Child>, b: &Option<ast::Child>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a.ptr_eq(b),
        (None, None) => true,
        _ => false,
    }
}
fn pair(arena: &Arena, a: NodeId, b: NodeId, parent: Option<NodeId>, out: &mut Vec<Value>) {
    let mut parent_ok = arena.parent(b) == parent;
    if parent.is_none() && matches!(arena.node(b), Node::Map(_)) {
        parent_ok = arena.parent(b).is_some_and(|p| match arena.node(p) {
            Node::Field(f) => {
                arena.parent(p).is_none()
                    && f.references.as_ref().unwrap()[0]
                        .read()
                        .context
                        .as_ref()
                        .unwrap()
                        .read()
                        .scope_map
                        == Some(b)
            }
            _ => false,
        });
    }
    let mut row = json!({"fresh":a!=b,"parent_ok":parent_ok,"kind":arena.node(a).kind()});
    match (arena.node(a), arena.node(b)) {
        (Node::Scalar(a), Node::Scalar(b)) => {
            row["value_alias"] = json!(alias(&a.value.unbox(), &b.value.unbox()))
        }
        (Node::Field(a), Node::Field(b)) => {
            row["name_alias"] = json!(alias(&a.name.unbox(), &b.name.unbox()));
            row["import_alias"] = json!(alias(&a.import_ast, &b.import_ast));
            row["refs_nil"] = json!(b.references.is_none());
            if let Some(r) = a.references.as_ref().and_then(|r| r.first()) {
                row["ref_alias"] = json!(r.ptr_eq(&b.references.as_ref().unwrap()[0]));
            }
        }
        (Node::Edge(a), Node::Edge(b)) => {
            row["import_alias"] = json!(alias(&a.import_ast, &b.import_ast));
            row["refs_nil"] = json!(b.references.is_none());
            if let Some(r) = a.references.as_ref().and_then(|r| r.first()) {
                row["ref_alias"] = json!(r.ptr_eq(&b.references.as_ref().unwrap()[0]));
            }
            if let Some(id) = &a.id {
                let other = b.id.as_ref().unwrap();
                row["id_alias"] = json!(id.ptr_eq(other));
                let id = id.read();
                let other = other.read();
                if let Some(i) = &id.index {
                    row["index_alias"] = json!(i.ptr_eq(other.index.as_ref().unwrap()));
                }
                let aa = id
                    .src_path
                    .iter()
                    .flatten()
                    .chain(id.dst_path.iter().flatten());
                let bb = other
                    .src_path
                    .iter()
                    .flatten()
                    .chain(other.dst_path.iter().flatten());
                row["endpoints"] = json!(aa.zip(bb).map(|(a, b)| alias(a, b)).collect::<Vec<_>>());
            }
        }
        (Node::Array(_), Node::Array(b)) => row["values_nil"] = json!(b.values.is_none()),
        (Node::Map(a), Node::Map(b)) => {
            row["fields_nil"] = json!(b.fields.is_none());
            row["edges_nil"] = json!(b.edges.is_none());
            row["version"] = json!(b.structure_version);
            row["import_alias"] = json!(alias(&a.import_ast, &b.import_ast));
        }
        _ => panic!("changed node type"),
    }
    out.push(row);
    let aa = arena.children(a);
    let bb = arena.children(b);
    assert_eq!(aa.len(), bb.len());
    for (ac, bc) in aa.into_iter().zip(bb) {
        pair(arena, ac, bc, Some(b), out);
    }
}
