// SPDX-License-Identifier: MPL-2.0
use d2_rs_core::{
    ast::{self, Shared},
    ir::*,
};
use d2_rs_language::{
    compiler::{Compiler, FieldQuery},
    format, ir, parse_key, parse_map_key, parse_value,
};
use serde::Deserialize;
use serde_json::{Value, json};
#[derive(Default, Deserialize)]
#[serde(default)]
pub struct Operation {
    op: String,
    key: String,
    map: String,
    path: String,
    value: String,
    array: bool,
    create: bool,
    indexed: bool,
    start: usize,
    no_context: bool,
    context: Option<usize>,
    glob: bool,
    enter: bool,
    stack: Vec<usize>,
    lazy: bool,
    target: String,
}
fn path_parts(path: &ast::KeyPath) -> Vec<ast::StringBox> {
    path.path
        .iter()
        .flat_map(ast::SharedSlice::iter)
        .map(|s| s.read().as_ref().unwrap().read().clone())
        .collect()
}
fn lookup(arena: &Arena, root: NodeId, path: &str) -> NodeId {
    arena
        .get_field(root, &path_parts(&parse_key(path.as_bytes()).unwrap()))
        .expect("missing field")
}
fn name(arena: &Arena, node: NodeId) -> String {
    let parts: Vec<_> = ir::ida(arena, node)
        .into_iter()
        .flatten()
        .map(|p| ast::raw_string_box(ast::ScalarBox::from_child(p).scalar_bytes(), true))
        .collect();
    String::from_utf8(format::format_key_path(&parts)).unwrap()
}
fn names(arena: &Arena, fields: &[NodeId]) -> Option<Vec<String>> {
    (!fields.is_empty()).then(|| fields.iter().map(|f| name(arena, *f)).collect())
}
fn snapshot_nodes(
    arena: &Arena,
    node: NodeId,
    contexts: &[Shared<RefContext>],
    paths: &[Shared<ast::KeyPath>],
    rows: &mut Vec<Value>,
) {
    let mut row = json!({"path":name(arena,node)});
    let children = match arena.node(node) {
        Node::Map(m) => {
            row["kind"] = json!("map");
            row["version"] = json!(m.structure_version);
            m.fields
                .iter()
                .flatten()
                .chain(m.edges.iter().flatten())
                .copied()
                .collect::<Vec<_>>()
        }
        Node::Field(f) => {
            row["kind"] = json!("field");
            row["refs"]=json!(f.references.iter().flatten().map(|r|{let r=r.read();json!({"context":r.context.as_ref().and_then(|c|contexts.iter().position(|v|v.ptr_eq(c))),"path":r.key_path.as_ref().and_then(|p|paths.iter().position(|v|v.ptr_eq(p))),"string":r.string,"glob":r.due_to_glob,"lazy":r.due_to_lazy_glob})}).collect::<Vec<_>>());
            let mut owner = None;
            for (i, path) in paths.iter().enumerate() {
                for (j, s) in path
                    .read()
                    .path
                    .iter()
                    .flat_map(ast::SharedSlice::iter)
                    .enumerate()
                {
                    if let (Some(a), Some(b)) =
                        (s.read().as_ref().unwrap().read().unbox(), f.name.unbox())
                        && a.ptr_eq(&b)
                    {
                        owner = Some([i, j]);
                    }
                }
            }
            row["name_owner"] = json!(owner);
            f.primary.into_iter().chain(f.composite).collect()
        }
        Node::Array(a) => {
            row["kind"] = json!("array");
            a.values.clone().unwrap_or_default()
        }
        Node::Scalar(_) => {
            row["kind"] = json!("scalar");
            Vec::new()
        }
        Node::Edge(_) => panic!("unexpected edge"),
    };
    rows.push(row);
    for child in children {
        snapshot_nodes(arena, child, contexts, paths, rows)
    }
}
pub fn evaluate(ops: Vec<Operation>) -> Value {
    let mut c = Compiler::default();
    c.glob_context_stack.push(Vec::new());
    let root = c.arena.new_root();
    let mut contexts: Vec<Shared<RefContext>> = Vec::new();
    let mut paths: Vec<Shared<ast::KeyPath>> = Vec::new();
    let mut rows = Vec::new();
    for op in ops {
        let mut selected = Vec::new();
        let mut created = Vec::new();
        let mut error = None;
        if op.op == "set" {
            let field = lookup(&c.arena, root, &op.path);
            let node = if op.array {
                Node::Array(Array::default())
            } else {
                Node::Scalar(Scalar {
                    value: ast::ScalarBox::from_child(Some(
                        parse_value(op.value.as_bytes()).unwrap(),
                    )),
                })
            };
            let value = c.arena.insert(Some(field), node);
            let Node::Field(f) = c.arena.node_mut(field) else {
                unreachable!()
            };
            if op.array {
                f.composite = Some(value)
            } else {
                f.primary = Some(value)
            }
        } else {
            let map = if op.map.is_empty() {
                root
            } else {
                c.arena
                    .map(lookup(&c.arena, root, &op.map))
                    .expect("missing query map")
            };
            let (context, path) = if let Some(index) = op.context {
                (contexts[index].clone(), paths[index].clone())
            } else {
                let key = Shared::new(parse_map_key(op.key.as_bytes()).unwrap());
                let path = {
                    let key = key.read();
                    match op.path.as_str() {
                        "src" | "dst" => {
                            let edge = key
                                .edges
                                .as_ref()
                                .unwrap()
                                .get(0)
                                .unwrap()
                                .read()
                                .clone()
                                .unwrap();
                            let edge = edge.read();
                            if op.path == "src" {
                                edge.src.clone().unwrap()
                            } else {
                                edge.dst.clone().unwrap()
                            }
                        }
                        "edge" => key.edge_key.clone().unwrap(),
                        _ => key.key.clone().unwrap(),
                    }
                };
                let context = Shared::new(RefContext {
                    key: Some(key),
                    scope_map: Some(map),
                    ..Default::default()
                });
                contexts.push(context.clone());
                paths.push(path.clone());
                (context, path)
            };
            c.glob_ref_context_stack = op.stack.iter().map(|i| contexts[*i].clone()).collect();
            if op.enter {
                c.glob_ref_context_stack.push(context.clone())
            }
            let glob = op.glob.then(|| c.ensure_glob_context(context.clone()));
            c.lazy_glob_being_applied = op.lazy;
            c.lazy_glob_target =
                (!op.target.is_empty()).then(|| lookup(&c.arena, root, &op.target));
            let result = c.construct_fields(
                map,
                op.start,
                &FieldQuery {
                    path,
                    context: (!op.no_context).then_some(context),
                    create: op.create,
                    glob,
                    indexed: op.indexed,
                },
            );
            selected = result.fields;
            created = result.created;
            error = result.error;
        }
        let mut nodes = Vec::new();
        snapshot_nodes(&c.arena, root, &contexts, &paths, &mut nodes);
        let globs:Vec<_>=c.glob_contexts().iter().map(|id|{let g=c.arena.glob(*id);let mut fields:Vec<_>=g.applied_fields.as_ref().map(|s|s.read().clone()).into_iter().flatten().map(|s|ast::ByteString::from(s.clone()).to_text().into_owned()).collect();fields.sort();json!({"context":g.context.as_ref().and_then(|p|contexts.iter().position(|v|v.ptr_eq(p))),"fields":fields})}).collect();
        rows.push(json!({"selected":names(&c.arena,&selected),"created":names(&c.arena,&created),"error":error,"ast":c.arena.ast(root),"nodes":nodes,"globs":globs}));
    }
    json!(rows)
}
fn main() {
    let requests: Vec<Vec<Operation>> = serde_json::from_reader(std::io::stdin()).unwrap();
    println!(
        "{}",
        json!(requests.into_iter().map(evaluate).collect::<Vec<_>>())
    );
}
