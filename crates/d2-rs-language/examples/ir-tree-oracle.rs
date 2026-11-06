// SPDX-License-Identifier: MPL-2.0
use d2_rs_core::{ast, ir::*};
use serde::Deserialize;
use serde_json::json;
#[derive(Deserialize)]
pub struct Request {
    parts: Vec<String>,
    in_key: bool,
    #[serde(default)]
    mode: String,
    #[serde(default)]
    hex: String,
    #[serde(default)]
    path: String,
    #[serde(default)]
    options: String,
}
fn string(value: &str, quoted: bool) -> ast::StringBox {
    if quoted {
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
    }
}
fn field(arena: &mut Arena, map: NodeId, name: ast::StringBox) -> NodeId {
    let id = arena.insert(
        Some(map),
        Node::Field(Field {
            name,
            ..Default::default()
        }),
    );
    let Node::Map(m) = arena.node_mut(map) else {
        panic!()
    };
    m.fields.get_or_insert_default().push(id);
    id
}
fn map(arena: &mut Arena, parent: NodeId) -> NodeId {
    let id = arena.insert(Some(parent), Node::Map(Map::default()));
    match arena.node_mut(parent) {
        Node::Field(f) => f.composite = Some(id),
        Node::Edge(e) => e.map = Some(id),
        _ => {}
    }
    id
}
fn scalar(arena: &mut Arena, parent: NodeId, text: &str, input: Option<&[u8]>) -> NodeId {
    let value = ast::ScalarBox::from_child(input.map_or_else(
        || string(text, false).unbox(),
        |input| Some(d2_rs_language::parse_value(input).unwrap()),
    ));
    let id = arena.insert(Some(parent), Node::Scalar(Scalar { value }));
    match arena.node_mut(parent) {
        Node::Field(f) => f.primary = Some(id),
        Node::Edge(e) => e.primary = Some(id),
        _ => {}
    }
    id
}
pub fn evaluate(request: Request) -> serde_json::Value {
    let input = ((request.mode == "ir_ast"
        || request.mode == "ir_copy"
        || request.mode == "ir_import"
        || request.mode == "ir_glob")
        && !request.hex.is_empty())
    .then(|| {
        request
            .hex
            .as_bytes()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|h| u8::from_str_radix(std::str::from_utf8(h).unwrap(), 16).unwrap())
            .collect::<Vec<_>>()
    });
    let mut arena = Arena::default();
    let root = arena.new_root();
    let mut current = root;
    let mut leaf = None;
    for (i, part) in request.parts.iter().enumerate() {
        let bytes: Vec<_> = part
            .as_bytes()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|h| u8::from_str_radix(std::str::from_utf8(h).unwrap(), 16).unwrap())
            .collect();
        let name = String::from_utf8(bytes).unwrap();
        let name = string(&name, request.in_key);
        if let Some(n) = &name.unquoted_string {
            n.write().range.path = request.path.clone();
        }
        if let Some(n) = &name.single_quoted_string {
            n.write().range.path = request.path.clone();
        }
        let node = field(&mut arena, current, name);
        leaf = Some(node);
        if i + 1 < request.parts.len() {
            current = map(&mut arena, node);
        }
    }
    if let Some(leaf) = leaf {
        scalar(&mut arena, leaf, "leaf", input.as_deref());
        let array = arena.insert(Some(leaf), Node::Array(Array::default()));
        let Node::Field(f) = arena.node_mut(leaf) else {
            panic!()
        };
        f.composite = Some(array);
        let item = scalar(&mut arena, array, "item", input.as_deref());
        let nested = map(&mut arena, array);
        let inside = field(&mut arena, nested, string("inside", false));
        scalar(&mut arena, inside, "nested", input.as_deref());
        let Node::Array(a) = arena.node_mut(array) else {
            panic!()
        };
        a.values = Some(vec![item, nested]);
    }
    let edge = arena.insert(
        Some(root),
        Node::Edge(Edge {
            id: Some(ast::Shared::new(if request.options.is_empty() {
                EdgeId::default()
            } else {
                new_edge_ids(&d2_rs_language::parse_map_key(request.options.as_bytes()).unwrap())
                    .unwrap()
                    .remove(0)
            })),
            ..Default::default()
        }),
    );
    let Node::Map(m) = arena.node_mut(root) else {
        panic!()
    };
    m.edges = Some(vec![edge]);
    scalar(&mut arena, edge, "edge", input.as_deref());
    let edge_map = map(&mut arena, edge);
    let vars = field(&mut arena, edge_map, string("vars", false));
    let vars_map = map(&mut arena, vars);
    let end = field(&mut arena, vars_map, string("end", false));
    scalar(&mut arena, end, "value", input.as_deref());
    fn visit(arena: &Arena, id: NodeId, nodes: &mut Vec<NodeId>) {
        nodes.push(id);
        for child in arena.children(id) {
            visit(arena, child, nodes);
        }
    }
    if request.mode == "ir_glob" {
        let mut current = root;
        loop {
            let Node::Map(m) = arena.node(current) else {
                panic!()
            };
            let next = m
                .fields
                .as_ref()
                .and_then(|f| f.first())
                .and_then(|f| arena.map(*f));
            let peer = field(&mut arena, current, string("peer", false));
            scalar(&mut arena, peer, "sibling", input.as_deref());
            if let Some(next) = next {
                current = next
            } else {
                break;
            }
        }
    }
    let root = if request.mode == "ir_copy" || request.mode == "ir_import" {
        arena.copy(root, None)
    } else {
        root
    };
    let mut nodes = vec![arena.parent(root).unwrap()];
    visit(&arena, root, &mut nodes);
    if request.mode == "ir_glob" {
        return glob_oracle::evaluate(&arena, &nodes);
    }
    if request.mode == "ir_import" {
        return import_oracle::evaluate(&mut arena, &nodes, root, &request.path);
    }
    if request.mode == "ir_copy" {
        return copy_oracle::evaluate(&mut arena, &nodes, root);
    }
    if request.mode == "ir_ast" {
        use ast::{Node as _, Scalar as _};
        let snapshots:Vec<_>=nodes.iter().map(|id|{
            let node=arena.ast(*id).unwrap();let before=json!(node);
            fn collect(node:&ast::Child,out:&mut Vec<String>){let scalar=ast::ScalarBox::from_child(Some(node.clone()));if scalar.unbox().is_some(){out.push(scalar.scalar_bytes().iter().map(|b|format!("{b:02x}")).collect());}for child in node.children(){collect(&child,out);}}
            let mut scalar_bytes=Vec::new();collect(&node,&mut scalar_bytes);
            let bytes=d2_rs_language::format::format(&node);let after=json!(node);
            let scalar_alias=if let Node::Scalar(s)=arena.node(*id){Some(node.ptr_eq(&s.value.unbox().unwrap()))}else{None};
            let name_alias=if let Node::Field(f)=arena.node(*id){let ast::Child::Key(key)=&node else{unreachable!()};let name=key.read().key.as_ref().unwrap().read().children()[0].clone();Some(name.ptr_eq(&f.name.unbox().unwrap()))}else{None};
            let edge_id=if matches!(arena.node(*id),Node::Edge(_)){Some(d2_rs_language::ir::edge_id_string(&arena,*id).scalar_bytes().iter().map(|b|format!("{b:02x}")).collect::<String>())}else{None};
            json!({"scalar_bytes":scalar_bytes,"before":before,"after":after,"hex":bytes.iter().map(|b|format!("{b:02x}")).collect::<String>(),"scalar_alias":scalar_alias,"name_alias":name_alias,"edge_id":edge_id})
        }).collect();
        return json!(snapshots);
    }
    let index = |id: Option<NodeId>| id.and_then(|id| nodes.iter().position(|n| *n == id));
    let summaries:Vec<_>=nodes.iter().map(|id|{let map=arena.map(*id);json!({"kind":arena.node(*id).kind(),"parent":index(arena.parent(*id)),"primary":index(arena.primary(*id)),"map":index(map),"parent_map":index(arena.parent_map(*id)),"parent_field":index(arena.parent_field(*id)),"parent_edge":index(arena.parent_edge(*id)),"parent_board":index(arena.parent_board(*id)),"board":arena.board_kind(*id),"is_var":arena.is_var(*id),"root":arena.root(*id),"root_map":index(map.map(|m|arena.root_map(m))),"fields":map.map_or(0,|m|arena.field_count_recursive(m)),"edges":map.map_or(0,|m|arena.edge_count_recursive(m))})}).collect();
    let Node::Field(root_field) = arena.node(nodes[0]) else {
        panic!()
    };
    let root_reference = root_field.references.as_ref().unwrap()[0]
        .read()
        .context
        .as_ref()
        .unwrap()
        .read()
        .scope_map
        == Some(root);
    json!({"nodes":summaries,"root_reference":root_reference})
}
fn main() {
    let requests: Vec<Request> = serde_json::from_reader(std::io::stdin()).unwrap();
    println!(
        "{}",
        serde_json::to_string(&requests.into_iter().map(evaluate).collect::<Vec<_>>()).unwrap()
    );
}

#[path = "support/ir-copy.rs"]
mod copy_oracle;

#[path = "support/import-contexts.rs"]
mod import_oracle;

#[path = "support/globs.rs"]
mod glob_oracle;
