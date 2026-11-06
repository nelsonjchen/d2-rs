// SPDX-License-Identifier: MPL-2.0
use d2_rs_core::{
    ast::{self, Child, Shared, SharedSlice},
    ir::*,
};
use serde::Deserialize;
use serde_json::json;
#[derive(Deserialize)]
pub struct Request {
    hex: String,
    parts: Vec<String>,
}
fn decode(s: &str) -> Vec<u8> {
    s.as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|h| u8::from_str_radix(std::str::from_utf8(h).unwrap(), 16).unwrap())
        .collect()
}
fn ast_eq(a: Option<Child>, b: Option<Child>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a.ptr_eq(&b),
        (None, None) => true,
        _ => false,
    }
}
fn context_variant(
    base: &Shared<RefContext>,
    change: impl FnOnce(&mut RefContext),
) -> Shared<RefContext> {
    let mut copy = base.read().clone();
    change(&mut copy);
    Shared::new(copy)
}
pub fn evaluate(r: Request) -> serde_json::Value {
    let key = Shared::new(d2_rs_language::parse_map_key(&decode(&r.hex)).unwrap());
    let scope = Shared::new(ast::Map::default());
    let base = Shared::new(RefContext {
        key: Some(key.clone()),
        edge: key
            .read()
            .edges
            .as_ref()
            .and_then(|edges| edges.get(0))
            .and_then(|e| e.read().clone()),
        scope: Some(scope.clone()),
        scope_ast: Some(scope.clone()),
        scope_map: Some(NodeId(0)),
    });
    let contexts = [
        base.clone(),
        context_variant(&base, |_| {}),
        context_variant(&base, |c| c.edge = Some(Shared::new(ast::Edge::default()))),
        context_variant(&base, |c| c.scope_map = Some(NodeId(1))),
        context_variant(&base, |c| c.scope = Some(Shared::new(ast::Map::default()))),
        context_variant(&base, |c| {
            c.scope_ast = Some(Shared::new(ast::Map::default()))
        }),
        context_variant(&base, |c| c.key = Some(Shared::new(key.read().clone()))),
        context_variant(&base, |c| {
            c.key = Some(Shared::new(
                d2_rs_language::parse_map_key(&decode(&r.hex)).unwrap(),
            ))
        }),
    ];
    let equal: Vec<Vec<_>> = contexts
        .iter()
        .map(|a| {
            contexts
                .iter()
                .map(|b| a.read().equals(&b.read()))
                .collect()
        })
        .collect();
    let identity: Vec<Vec<_>> = contexts
        .iter()
        .map(|a| {
            contexts
                .iter()
                .map(|b| same_reference_context(&Some(a.clone()), &Some(b.clone())))
                .collect()
        })
        .collect();
    let indices: Vec<_> = contexts.iter().map(|c| c.read().edge_index()).collect();
    let peer_equal: Vec<_> = r
        .parts
        .iter()
        .map(|p| {
            let c = context_variant(&base, |c| {
                c.key = Some(Shared::new(
                    d2_rs_language::parse_map_key(&decode(p)).unwrap(),
                ))
            });
            base.read().equals(&c.read())
        })
        .collect();
    let mut refs = Vec::new();
    let mut add = |path: Option<Shared<ast::KeyPath>>, edge: Option<Shared<ast::Edge>>| {
        if let Some(path) = path {
            let context = context_variant(&base, |c| c.edge = edge);
            for slot in path.read().path.iter().flat_map(SharedSlice::iter) {
                refs.push(Shared::new(FieldReference {
                    string: slot.read().as_ref().unwrap().read().unbox(),
                    key_path: Some(path.clone()),
                    context: Some(context.clone()),
                    ..Default::default()
                }));
            }
        }
    };
    add(key.read().key.clone(), None);
    for edge in key.read().edges.iter().flat_map(SharedSlice::iter) {
        let edge = edge.read().clone().unwrap();
        let e = edge.read();
        add(e.src.clone(), Some(edge.clone()));
        add(e.dst.clone(), Some(edge.clone()));
    }
    add(key.read().edge_key.clone(), base.read().edge.clone());
    let field_rows:Vec<_>=refs.iter().map(|r|{let r=r.read();json!({"primary":r.primary(),"path_index":r.key_path_index(),"in_edge":r.in_edge(),"edge_dest":r.in_edge().then(||r.edge_dest()),"ast_identity":ast_eq(r.ast(),r.string.clone()),"edge_index":r.context.as_ref().unwrap().read().edge_index()})}).collect();
    let first = refs.last().unwrap();
    let normal = Shared::new(first.read().clone());
    let mut lazy = first.read().clone();
    lazy.due_to_glob = true;
    lazy.due_to_lazy_glob = true;
    let lazy = Shared::new(lazy);
    let mut copy = lazy.read().clone();
    copy.context = Some(context_variant(copy.context.as_ref().unwrap(), |_| {}));
    let copy = Shared::new(copy);
    let mut other_map = lazy.read().clone();
    other_map.context = Some(context_variant(other_map.context.as_ref().unwrap(), |c| {
        c.scope_map = Some(NodeId(2))
    }));
    let other_map = Shared::new(other_map);
    let mut other_path = lazy.read().clone();
    let copied_path = other_path.key_path.as_ref().unwrap().read().clone();
    other_path.key_path = Some(Shared::new(copied_path));
    let other_path = Shared::new(other_path);
    let mut last = first.read().clone();
    last.due_to_glob = true;
    let last = Shared::new(last);
    let mut field = Field::default();
    let mut field_counts = Vec::new();
    let mut field_primary_steps = Vec::new();
    for r in [
        normal.clone(),
        normal,
        lazy.clone(),
        lazy,
        copy,
        other_map,
        other_path,
        last,
    ] {
        field.append_reference(r);
        field_counts.push(field.references.as_ref().unwrap().len());
        field_primary_steps.push(field.last_primary_ref().and_then(|r| {
            field
                .references
                .as_ref()
                .unwrap()
                .iter()
                .position(|v| v.ptr_eq(&r))
        }));
    }
    let field_index = |r: Option<Shared<FieldReference>>| {
        r.and_then(|r| {
            field
                .references
                .as_ref()
                .unwrap()
                .iter()
                .position(|v| v.ptr_eq(&r))
        })
    };
    let enormal = Shared::new(EdgeReference {
        context: Some(base.clone()),
        ..Default::default()
    });
    let elazy = Shared::new(EdgeReference {
        context: Some(base.clone()),
        due_to_glob: true,
        due_to_lazy_glob: true,
    });
    let ecopy = Shared::new(EdgeReference {
        context: Some(context_variant(&base, |_| {})),
        ..elazy.read().clone()
    });
    let emap = Shared::new(EdgeReference {
        context: Some(context_variant(&base, |c| c.scope_map = Some(NodeId(3)))),
        ..elazy.read().clone()
    });
    let elast = Shared::new(EdgeReference {
        context: Some(base.clone()),
        due_to_glob: true,
        ..Default::default()
    });
    let mut edge = Edge::default();
    let mut edge_counts = Vec::new();
    let mut edge_primary_steps = Vec::new();
    for r in [
        enormal.clone(),
        enormal.clone(),
        elazy.clone(),
        elazy,
        ecopy,
        emap,
        elast,
    ] {
        edge.append_reference(r);
        edge_counts.push(edge.references.as_ref().unwrap().len());
        edge_primary_steps.push(edge.last_primary_ref().and_then(|r| {
            edge.references
                .as_ref()
                .unwrap()
                .iter()
                .position(|v| v.ptr_eq(&r))
        }));
    }
    let edge_index = |r: Option<Shared<EdgeReference>>| {
        r.and_then(|r| {
            edge.references
                .as_ref()
                .unwrap()
                .iter()
                .position(|v| v.ptr_eq(&r))
        })
    };
    let root_ref = FieldReference {
        context: Some(base.clone()),
        ..Default::default()
    };
    let mut arena = Arena::default();
    for _ in 0..4 {
        arena.insert(None, Node::Map(Map::default()));
    }
    let root = arena.new_root();
    let owner = arena.insert(
        Some(root),
        Node::Field(Field {
            name: ast::raw_string_box("owner", true),
            references: Some(vec![first.clone()]),
            ..Default::default()
        }),
    );
    let owner_map = arena.insert(Some(owner), Node::Map(Map::default()));
    let nested_edge = arena.insert(
        Some(owner_map),
        Node::Edge(Edge {
            references: Some(vec![enormal.clone()]),
            ..Default::default()
        }),
    );
    let scalar = arena.insert(
        Some(nested_edge),
        Node::Scalar(Scalar {
            value: ast::ScalarBox::from_child(ast::raw_string_box("value", false).unbox()),
        }),
    );
    let nested_map = arena.insert(Some(nested_edge), Node::Map(Map::default()));
    if let Node::Field(f) = arena.node_mut(owner) {
        f.composite = Some(owner_map);
    }
    if let Node::Map(m) = arena.node_mut(root) {
        m.fields = Some(vec![owner]);
    }
    if let Node::Map(m) = arena.node_mut(owner_map) {
        m.edges = Some(vec![nested_edge]);
    }
    if let Node::Edge(e) = arena.node_mut(nested_edge) {
        e.primary = Some(scalar);
        e.map = Some(nested_map);
    }
    let parent_selection:Vec<_>=[owner,owner_map,nested_edge,scalar,nested_map].into_iter().map(|id|{
        let reference=arena.last_ref(id);let primary=arena.last_primary_ref(id);
        let is_field=|r:&Option<Reference>|matches!(r,Some(Reference::Field(r)) if r.ptr_eq(first));
        let is_edge=|r:&Option<Reference>|matches!(r,Some(Reference::Edge(r)) if r.ptr_eq(&enormal));
        json!({"last_field":is_field(&reference),"last_edge":is_edge(&reference),"primary_field":is_field(&primary),"primary_edge":is_edge(&primary),"primary_key":arena.last_primary_key(id).is_some_and(|k|k.ptr_eq(&key)),"primary_claim":primary.as_ref().map(Reference::primary)})
    }).collect();
    json!({"parent_selection":parent_selection,"field_primary_steps":field_primary_steps,"edge_primary_steps":edge_primary_steps,"fields":field_rows,"contexts_equal":equal,"contexts_identity":identity,"context_edge_indices":indices,"peer_equal":peer_equal,"field_counts":field_counts,"field_last":field_index(Some(field.last_ref())),"field_last_primary":field_index(field.last_primary_ref()),"field_primary_key":field.last_primary_key().is_some_and(|k|k.ptr_eq(&key)),"edge_counts":edge_counts,"edge_last":edge_index(Some(edge.last_ref())),"edge_last_primary":edge_index(edge.last_primary_ref()),"edge_primary_key":edge.last_primary_key().is_some_and(|k|k.ptr_eq(&key)),"edge_reference_primary":enormal.read().primary(),"edge_ast_identity":ast_eq(enormal.read().ast(),base.read().edge.clone().map(Child::Edge)),"root_ast_identity":ast_eq(root_ref.ast(),Some(Child::Map(scope)))})
}
fn main() {
    let requests: Vec<Request> = serde_json::from_reader(std::io::stdin()).unwrap();
    println!(
        "{}",
        serde_json::to_string(&requests.into_iter().map(evaluate).collect::<Vec<_>>()).unwrap()
    );
}
