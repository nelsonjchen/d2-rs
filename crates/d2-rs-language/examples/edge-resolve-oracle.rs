// SPDX-License-Identifier: MPL-2.0
use d2_rs_core::{ast, ir::*};
use serde::Deserialize;
use serde_json::{Value, json};
#[derive(Deserialize)]
pub struct Request {
    hex: String,
    parts: Vec<String>,
    in_key: bool,
}
fn bytes(hex: &str) -> Vec<u8> {
    hex.as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|h| u8::from_str_radix(std::str::from_utf8(h).unwrap(), 16).unwrap())
        .collect()
}
pub fn evaluate(request: Request) -> Value {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(request)))
        .unwrap_or_else(|_| json!({"panic":true}))
}
fn run(request: Request) -> Value {
    let mut arena = Arena::default();
    let root = arena.new_root();
    let mut maps = vec![root];
    let Node::Field(f) = arena.node(arena.parent(root).unwrap()) else {
        panic!()
    };
    let mut names = vec![f.name.unbox()];
    let mut m = root;
    for part in request.parts {
        let value = bytes(&part);
        let name = if request.in_key {
            ast::StringBox {
                single_quoted_string: Some(ast::Shared::new(ast::SingleQuotedString {
                    value: value.into(),
                    ..Default::default()
                })),
                ..Default::default()
            }
        } else {
            ast::StringBox {
                unquoted_string: Some(ast::Shared::new(ast::UnquotedString::flat(value))),
                ..Default::default()
            }
        };
        names.push(name.unbox());
        let field = arena.insert(
            Some(m),
            Node::Field(Field {
                name,
                ..Default::default()
            }),
        );
        let next = arena.insert(Some(field), Node::Map(Map::default()));
        let Node::Field(f) = arena.node_mut(field) else {
            panic!()
        };
        f.composite = Some(next);
        let Node::Map(map) = arena.node_mut(m) else {
            panic!()
        };
        map.fields = Some(vec![field]);
        m = next;
        maps.push(m);
    }
    let id = new_edge_ids(&d2_rs_language::parse_map_key(&bytes(&request.hex)).unwrap())
        .unwrap()
        .remove(0);
    let resolved = match id.resolve(&arena, m) {
        Ok(r) => r,
        Err(e) => return json!({"error":e.to_string()}),
    };
    let owners = |path: &Option<Vec<Option<ast::Child>>>| {
        path.iter()
            .flatten()
            .map(|n| {
                let mut owner = "unknown".to_string();
                for (prefix, values) in [
                    ("s", id.src_path.as_deref().unwrap_or_default()),
                    ("d", id.dst_path.as_deref().unwrap_or_default()),
                    ("n", names.as_slice()),
                ] {
                    for (i, v) in values.iter().enumerate() {
                        if n.as_ref().unwrap().ptr_eq(v.as_ref().unwrap()) {
                            owner = format!("{prefix}{i}");
                        }
                    }
                }
                owner
            })
            .collect::<Vec<_>>()
    };
    let raw: Vec<_> = resolved
        .id
        .src_path
        .iter()
        .flatten()
        .chain(resolved.id.dst_path.iter().flatten())
        .chain(resolved.common.iter().flatten())
        .map(|n| {
            ast::ScalarBox::from_child(n.clone())
                .scalar_bytes()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        })
        .collect();
    json!({"id":resolved.id,"original":id,"map":maps.iter().position(|m|*m==resolved.map),"common":resolved.common,"owners":[owners(&resolved.id.src_path),owners(&resolved.id.dst_path),owners(&resolved.common)],"index_alias":id.index.as_ref().map(|i|i.ptr_eq(resolved.id.index.as_ref().unwrap())),"raw":raw})
}
fn main() {
    std::panic::set_hook(Box::new(|_| {}));
    let requests: Vec<Request> = serde_json::from_reader(std::io::stdin()).unwrap();
    println!(
        "{}",
        json!(requests.into_iter().map(evaluate).collect::<Vec<_>>())
    );
}
