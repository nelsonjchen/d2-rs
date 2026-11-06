// SPDX-License-Identifier: MPL-2.0
use d2_rs_core::{
    ast::{self, Node as _, Shared},
    ir::*,
};
use d2_rs_language::{
    compiler::{compile_ir, imports::import_template_safe_at},
    parse,
};
use serde::Deserialize;
use serde_json::{Value, json};
#[derive(Deserialize)]
pub struct Request {
    input: String,
    import: String,
}
fn snapshot(arena: &Arena, node: NodeId, out: &mut Vec<Value>) {
    let mut scopes = Vec::new();
    let children = match arena.node(node) {
        Node::Map(m) => m
            .fields
            .iter()
            .flatten()
            .chain(m.edges.iter().flatten())
            .copied()
            .collect(),
        Node::Field(f) => {
            scopes.extend(f.references.iter().flatten().map(|r| {
                r.read()
                    .context
                    .as_ref()
                    .unwrap()
                    .read()
                    .scope_map
                    .is_none()
            }));
            f.primary.into_iter().chain(f.composite).collect()
        }
        Node::Edge(e) => {
            scopes.extend(e.references.iter().flatten().map(|r| {
                r.read()
                    .context
                    .as_ref()
                    .unwrap()
                    .read()
                    .scope_map
                    .is_none()
            }));
            e.primary.into_iter().chain(e.map).collect()
        }
        Node::Array(a) => a.values.clone().unwrap_or_default(),
        Node::Scalar(_) => Vec::new(),
    };
    out.push(json!({"kind":arena.node(node).kind(),"nil_scopes":scopes,"safe":import_template_safe_at(arena,Some(node))}));
    for child in children {
        snapshot(arena, child, out);
    }
}
pub fn evaluate(r: Request) -> Value {
    let parsed = parse("source.d2", r.input.as_bytes(), None);
    assert!(parsed.error.is_none());
    let (mut c, root) = compile_ir(Shared::new(parsed.map)).unwrap();
    assert!(c.errors.is_empty(), "{:?}", c.errors);
    let parsed = parse("import.d2", format!("x: {}", r.import).as_bytes(), None);
    assert!(parsed.error.is_none());
    let ast::Child::Key(key) = parsed.map.children()[0].clone() else {
        panic!()
    };
    let import = key.read().value.import.clone().unwrap();
    let mut before = Vec::new();
    snapshot(&c.arena, root, &mut before);
    let selected = c.select_import(root, import);
    let mut after = Vec::new();
    snapshot(&c.arena, root, &mut after);
    json!({"before":before,"after":after,"selected":selected.map(|s|c.arena.ast(s)),"errors":c.errors,"ok":selected.is_some()})
}
fn main() {
    let requests: Vec<Request> = serde_json::from_reader(std::io::stdin()).unwrap();
    println!(
        "{}",
        serde_json::to_string(&requests.into_iter().map(evaluate).collect::<Vec<_>>()).unwrap()
    );
}
