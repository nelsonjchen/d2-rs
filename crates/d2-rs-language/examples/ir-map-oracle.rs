// SPDX-License-Identifier: MPL-2.0
use d2_rs_core::{ast, ir::*};
use serde::Deserialize;
use serde_json::{Value, json};
#[derive(Default, Deserialize)]
struct Name {
    #[serde(default)]
    hex: String,
    #[serde(default)]
    quoted: bool,
    #[serde(default)]
    nil: bool,
}
fn bytes(hex: &str) -> Vec<u8> {
    hex.as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|h| u8::from_str_radix(std::str::from_utf8(h).unwrap(), 16).unwrap())
        .collect()
}
impl Name {
    fn ast(&self) -> ast::StringBox {
        if self.nil {
            return ast::StringBox::default();
        }
        let value = bytes(&self.hex);
        if self.quoted {
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
}
#[derive(Deserialize)]
struct Operation {
    op: String,
    #[serde(default)]
    map: usize,
    #[serde(default)]
    index: usize,
    #[serde(default)]
    id: usize,
    #[serde(default)]
    name: Name,
    #[serde(default)]
    key: String,
    #[serde(default)]
    child: bool,
    #[serde(default)]
    path: Vec<String>,
}
#[derive(Deserialize)]
struct Input {
    operations: Vec<Operation>,
    queries: Vec<Vec<Name>>,
    edges: Vec<String>,
    #[serde(default)]
    detailed: bool,
}
#[derive(Deserialize)]
pub struct Request {
    hex: String,
}
fn eid(key: &str) -> EdgeId {
    new_edge_ids(&d2_rs_language::parse_map_key(key.as_bytes()).unwrap())
        .unwrap()
        .remove(0)
}
fn make_field(arena: &mut Arena, map: NodeId, name: ast::StringBox) -> NodeId {
    arena.insert(
        Some(map),
        Node::Field(Field {
            name,
            ..Default::default()
        }),
    )
}
fn child_map(arena: &mut Arena, field: NodeId) -> NodeId {
    let id = arena.insert(Some(field), Node::Map(Map::default()));
    let Node::Field(f) = arena.node_mut(field) else {
        panic!()
    };
    f.composite = Some(id);
    id
}
fn direct_fields(arena: &mut Arena, map: NodeId, fields: Vec<NodeId>) {
    let Node::Map(m) = arena.node_mut(map) else {
        panic!()
    };
    m.fields = Some(fields);
}
pub fn evaluate(request: Request) -> Value {
    let input: Input = serde_json::from_slice(&bytes(&request.hex)).unwrap();
    let mut arena = Arena::default();
    let root = arena.new_root();
    let scope = make_field(&mut arena, root, ast::raw_string_box("scope", true));
    let middle = child_map(&mut arena, scope);
    direct_fields(&mut arena, root, vec![scope]);
    let dive = make_field(&mut arena, middle, ast::raw_string_box("dive", true));
    let bottom = child_map(&mut arena, dive);
    direct_fields(&mut arena, middle, vec![dive]);
    let mut maps = vec![root, middle, bottom];
    let mut fields = vec![scope, dive];
    let mut edges = Vec::new();
    let mut contexts = std::collections::HashMap::<usize, ast::Shared<RefContext>>::new();
    let mut labels = RefLabels::default();
    let mut snapshots = vec![snapshot(
        &mut arena,
        &maps,
        &fields,
        &edges,
        &input,
        (None, None),
        &mut labels,
    )];
    for op in &input.operations {
        let map = maps[op.map];
        let mut removed = None;
        let mut created = None;
        match op.op.as_str() {
            "append" | "insert" | "direct_append" => {
                let f = make_field(&mut arena, map, op.name.ast());
                fields.push(f);
                if op.child {
                    let m = child_map(&mut arena, f);
                    let target = make_field(&mut arena, m, ast::raw_string_box("target", true));
                    direct_fields(&mut arena, m, vec![target]);
                    maps.push(m);
                    fields.push(target);
                }
                if op.op == "append" {
                    arena.append_field(map, f)
                } else if op.op == "insert" {
                    arena.insert_field(map, op.index, f)
                } else {
                    let Node::Map(m) = arena.node_mut(map) else {
                        panic!()
                    };
                    m.fields.get_or_insert_default().push(f);
                }
            }
            "remove" => {
                let id = arena.remove_field(map, op.index);
                removed = fields.iter().position(|f| *f == id);
            }
            "rename" => {
                let Node::Field(f) = arena.node_mut(fields[op.id]) else {
                    panic!()
                };
                f.name = op.name.ast();
            }
            "swap" => {
                let Node::Map(m) = arena.node_mut(map) else {
                    panic!()
                };
                let f = m.fields.as_mut().unwrap();
                let last = f.len() - 1;
                f.swap(0, last);
            }
            "append_edge" => {
                let e = arena.insert(
                    Some(map),
                    Node::Edge(Edge {
                        id: (!op.key.is_empty()).then(|| ast::Shared::new(eid(&op.key))),
                        ..Default::default()
                    }),
                );
                edges.push(e);
                arena.append_edge(map, e);
            }
            "remove_edge" => {
                let id = arena.remove_edge(map, op.index);
                removed = edges.iter().position(|e| *e == id);
            }
            "reedge" => {
                let Node::Edge(e) = arena.node_mut(edges[op.id]) else {
                    panic!()
                };
                e.id = Some(ast::Shared::new(eid(&op.key)));
            }
            "swap_edges" => {
                let Node::Map(m) = arena.node_mut(map) else {
                    panic!()
                };
                let e = m.edges.as_mut().unwrap();
                let last = e.len() - 1;
                e.swap(0, last);
            }
            "mark" => arena.mark_structure_changed(map),
            "ref_field" | "ref_edge" => {
                let context = contexts
                    .entry(op.index)
                    .or_insert_with(|| {
                        ast::Shared::new(RefContext {
                            scope_map: Some(root),
                            ..Default::default()
                        })
                    })
                    .clone();
                let context = (!op.name.nil).then_some(context);
                if op.op == "ref_field" {
                    let Node::Field(f) = arena.node_mut(fields[op.id]) else {
                        panic!()
                    };
                    f.references
                        .get_or_insert_default()
                        .push(ast::Shared::new(FieldReference {
                            string: f.name.unbox(),
                            context,
                            ..Default::default()
                        }));
                } else {
                    let Node::Edge(e) = arena.node_mut(edges[op.id]) else {
                        panic!()
                    };
                    e.references
                        .get_or_insert_default()
                        .push(ast::Shared::new(EdgeReference {
                            context,
                            ..Default::default()
                        }));
                }
            }
            "field_primary" | "edge_primary" => {
                let parent = if op.op == "field_primary" {
                    fields[op.id]
                } else {
                    edges[op.id]
                };
                let value = ast::ScalarBox::from_child(Some(
                    d2_rs_language::parse_value(op.key.as_bytes()).unwrap(),
                ));
                let scalar = arena.insert(Some(parent), Node::Scalar(Scalar { value }));
                match arena.node_mut(parent) {
                    Node::Field(f) => f.primary = Some(scalar),
                    Node::Edge(e) => e.primary = Some(scalar),
                    _ => panic!(),
                }
            }
            "field_array" => {
                let field = fields[op.id];
                let array = arena.insert(Some(field), Node::Array(Array::default()));
                let mut values = Vec::new();
                if !op.key.is_empty() {
                    values.push(arena.insert(
                        Some(array),
                        Node::Scalar(Scalar {
                            value: ast::ScalarBox::from_child(Some(
                                d2_rs_language::parse_value(op.key.as_bytes()).unwrap(),
                            )),
                        }),
                    ));
                }
                if op.child {
                    let m = arena.insert(Some(array), Node::Map(Map::default()));
                    let f = make_field(&mut arena, m, ast::raw_string_box("inside", true));
                    direct_fields(&mut arena, m, vec![f]);
                    values.push(m);
                }
                let Node::Array(a) = arena.node_mut(array) else {
                    panic!()
                };
                a.values = (!values.is_empty()).then_some(values);
                let Node::Field(f) = arena.node_mut(field) else {
                    panic!()
                };
                f.composite = Some(array);
                register_value(&arena, array, &mut maps, &mut fields, &mut edges);
            }
            "field_map" | "edge_map" => {
                let parent = if op.op == "field_map" {
                    fields[op.id]
                } else {
                    edges[op.id]
                };
                let m = arena.insert(
                    Some(parent),
                    Node::Map(Map {
                        fields: Some(Vec::new()),
                        ..Default::default()
                    }),
                );
                if op.child {
                    let f = make_field(&mut arena, m, ast::raw_string_box("inside", true));
                    direct_fields(&mut arena, m, vec![f]);
                }
                match arena.node_mut(parent) {
                    Node::Field(f) => f.composite = Some(m),
                    Node::Edge(e) => e.map = Some(m),
                    _ => panic!(),
                };
                register(&arena, m, &mut maps, &mut fields, &mut edges);
            }
            "merge_map" => {
                arena.overlay_map(map, maps[op.id], op.child);
                register(&arena, map, &mut maps, &mut fields, &mut edges);
            }
            "merge_field" => {
                arena.overlay_field(fields[op.id], fields[op.index], op.child);
                register(&arena, map, &mut maps, &mut fields, &mut edges);
            }
            "merge_edge" => {
                arena.overlay_edge(edges[op.id], edges[op.index], op.child);
                register(&arena, map, &mut maps, &mut fields, &mut edges);
            }
            "expand" => {
                arena.expand_substitution(map, maps[op.id], fields[op.index], op.child);
                register(&arena, map, &mut maps, &mut fields, &mut edges);
            }
            "replace_context" => {
                contexts.insert(
                    op.index,
                    ast::Shared::new(RefContext {
                        scope_map: Some(root),
                        ..Default::default()
                    }),
                );
            }
            "delete_edge" => {
                let id = (!op.key.is_empty()).then(|| eid(&op.key));
                removed = arena
                    .delete_edge(map, id.as_ref())
                    .and_then(|e| edges.iter().position(|id| *id == e));
            }
            "delete_field" => {
                let path: Vec<_> = op
                    .path
                    .iter()
                    .map(|h| ast::ByteString::from(bytes(h)))
                    .collect();
                removed = arena
                    .delete_field(map, &path)
                    .and_then(|f| fields.iter().position(|id| *id == f));
            }
            "base" | "base_nil" => {
                let copy =
                    arena.copy_base((op.op == "base").then_some(map), op.child.then_some(root));
                created = Some(maps.len());
                register(&arena, copy, &mut maps, &mut fields, &mut edges);
            }
            _ => panic!("unknown operation"),
        }
        snapshots.push(snapshot(
            &mut arena,
            &maps,
            &fields,
            &edges,
            &input,
            (removed, created),
            &mut labels,
        ));
    }
    json!(snapshots)
}
fn snapshot(
    arena: &mut Arena,
    maps: &[NodeId],
    fields: &[NodeId],
    edges: &[NodeId],
    input: &Input,
    result: (Option<usize>, Option<usize>),
    labels: &mut RefLabels,
) -> Value {
    let (removed, created) = result;
    let rows:Vec<_>=maps.iter().map(|map|{
        let Node::Map(m)=arena.node(*map)else{panic!()};
        let fs:Vec<_>=m.fields.iter().flatten().map(|f|fields.iter().position(|id|id==f)).collect();let es:Vec<_>=m.edges.iter().flatten().map(|e|edges.iter().position(|id|id==e)).collect();let version=m.structure_version;
        let detail=if input.detailed{
            let frs:Vec<_>=m.fields.iter().flatten().map(|id|{let Node::Field(f)=arena.node(*id)else{panic!()};f.references.as_ref().map(|refs|refs.iter().map(|r|{if let Some(i)=labels.fields.iter().position(|v|v.ptr_eq(r)){i}else{labels.fields.push(r.clone());labels.fields.len()-1}}).collect::<Vec<_>>())}).collect();
            let ers:Vec<_>=m.edges.iter().flatten().map(|id|{let Node::Edge(e)=arena.node(*id)else{panic!()};e.references.as_ref().map(|refs|refs.iter().map(|r|{if let Some(i)=labels.edges.iter().position(|v|v.ptr_eq(r)){i}else{labels.edges.push(r.clone());labels.edges.len()-1}}).collect::<Vec<_>>())}).collect();
            Some((json!(arena.ast(*map)),json!(frs),json!(ers)))
        }else{None};
        let mut linear=Vec::new();let mut indexed=Vec::new();
        for q in &input.queries{let path:Vec<_>=q.iter().map(Name::ast).collect();linear.push(arena.get_field(*map,&path).and_then(|f|fields.iter().position(|id|*id==f)));indexed.push(arena.get_field_indexed(*map,&path).and_then(|f|fields.iter().position(|id|*id==f)));}
        let queries:Vec<_>=input.edges.iter().map(|key|(if input.detailed{arena.get_edges_indexed(*map,&eid(key)).unwrap_or_default()}else{arena.lookup_edges_local(*map,&eid(key))}).iter().map(|e|edges.iter().position(|id|id==e)).collect::<Vec<_>>()).collect();
        let mut row=json!({"fields":fs,"edges":es,"version":version,"linear":linear,"indexed":indexed,"edge_queries":queries});
        if let Some((ast,frs,ers))=detail{row["ast"]=ast;row["field_refs"]=frs;row["edge_refs"]=ers;row["linear_edges"]=json!(input.edges.iter().map(|key|arena.get_edges(*map,&eid(key)).map(|es|es.iter().map(|e|edges.iter().position(|id|id==e)).collect::<Vec<_>>())).collect::<Vec<_>>());}
        row
    }).collect();
    let mut result = json!({"maps":rows,"removed":removed});
    if input.detailed {
        result["created"] = json!(created);
    }
    result
}
fn main() {
    let requests: Vec<Request> = serde_json::from_reader(std::io::stdin()).unwrap();
    println!(
        "{}",
        json!(requests.into_iter().map(evaluate).collect::<Vec<_>>())
    );
}

fn register(
    arena: &Arena,
    map: NodeId,
    maps: &mut Vec<NodeId>,
    fields: &mut Vec<NodeId>,
    edges: &mut Vec<NodeId>,
) {
    if !maps.contains(&map) {
        maps.push(map);
    }
    let Node::Map(m) = arena.node(map) else {
        panic!()
    };
    for f in m.fields.iter().flatten() {
        if !fields.contains(f) {
            fields.push(*f);
        }
        let Node::Field(f) = arena.node(*f) else {
            panic!()
        };
        if let Some(value) = f.composite {
            register_value(arena, value, maps, fields, edges);
        }
    }
    for e in m.edges.iter().flatten() {
        if !edges.contains(e) {
            edges.push(*e);
        }
        if let Some(map) = arena.map(*e) {
            register(arena, map, maps, fields, edges);
        }
    }
}

#[derive(Default)]
struct RefLabels {
    fields: Vec<ast::Shared<FieldReference>>,
    edges: Vec<ast::Shared<EdgeReference>>,
}
fn register_value(
    arena: &Arena,
    node: NodeId,
    maps: &mut Vec<NodeId>,
    fields: &mut Vec<NodeId>,
    edges: &mut Vec<NodeId>,
) {
    match arena.node(node) {
        Node::Map(_) => register(arena, node, maps, fields, edges),
        Node::Array(a) => {
            for v in a.values.iter().flatten() {
                register_value(arena, *v, maps, fields, edges)
            }
        }
        _ => {}
    }
}
