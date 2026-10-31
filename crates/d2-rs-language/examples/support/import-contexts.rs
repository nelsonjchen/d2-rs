// SPDX-License-Identifier: MPL-2.0
use d2_rs_core::{ast, ir::*};
use serde_json::{Value, json};
use std::collections::HashSet;
#[derive(Default)]
struct Registry {
    contexts: Vec<ast::Shared<RefContext>>,
    refs: Vec<Reference>,
    globs: Vec<GlobId>,
}
impl Registry {
    fn context(&mut self, c: &Option<ast::Shared<RefContext>>) -> Option<usize> {
        c.as_ref().map(|c| {
            if let Some(i) = self.contexts.iter().position(|v| v.ptr_eq(c)) {
                i
            } else {
                self.contexts.push(c.clone());
                self.contexts.len() - 1
            }
        })
    }
    fn reference(&mut self, r: Reference) -> usize {
        if let Some(i) = self.refs.iter().position(|v| match (v, &r) {
            (Reference::Field(a), Reference::Field(b)) => a.ptr_eq(b),
            (Reference::Edge(a), Reference::Edge(b)) => a.ptr_eq(b),
            _ => false,
        }) {
            i
        } else {
            self.refs.push(r);
            self.refs.len() - 1
        }
    }
    fn glob(&mut self, g: Option<GlobId>) -> Option<usize> {
        g.map(|g| {
            if let Some(i) = self.globs.iter().position(|v| *v == g) {
                i
            } else {
                self.globs.push(g);
                self.globs.len() - 1
            }
        })
    }
}
struct Probe {
    source: Vec<NodeId>,
    copy: Vec<NodeId>,
    key: ast::Shared<ast::Key>,
    scope: ast::Shared<ast::Map>,
    scope_ast: ast::Shared<ast::Map>,
}
impl Probe {
    fn node_name(&self, id: Option<NodeId>) -> Value {
        match id {
            None => Value::Null,
            Some(id) => {
                if let Some(i) = self.source.iter().position(|v| *v == id) {
                    json!(format!("s{i}"))
                } else if let Some(i) = self.copy.iter().position(|v| *v == id) {
                    json!(format!("c{i}"))
                } else {
                    json!("external")
                }
            }
        }
    }
    fn snapshot(&self, arena: &Arena, r: &mut Registry) -> Value {
        let mut ns = Vec::new();
        for n in self.source.iter().chain(&self.copy) {
            let mut row = json!({"id":self.node_name(Some(*n))});
            match arena.node(*n) {
                Node::Map(m) => {
                    row["globs"] = json!(
                        m.globs
                            .as_ref()
                            .map(|v| v.iter().map(|g| r.glob(*g.read())).collect::<Vec<_>>())
                    )
                }
                Node::Field(f) => {
                    row["refs"] = json!(f.references.as_ref().map(|v| {
                        v.iter()
                            .map(|v| {
                                let id = r.reference(Reference::Field(v.clone()));
                                r.context(&v.read().context);
                                id
                            })
                            .collect::<Vec<_>>()
                    }))
                }
                Node::Edge(e) => {
                    row["refs"] = json!(e.references.as_ref().map(|v| {
                        v.iter()
                            .map(|v| {
                                let id = r.reference(Reference::Edge(v.clone()));
                                r.context(&v.read().context);
                                id
                            })
                            .collect::<Vec<_>>()
                    }))
                }
                _ => {}
            }
            ns.push(row);
        }
        let mut gs = Vec::new();
        let mut i = 0;
        while i < r.globs.len() {
            let g = arena.glob(r.globs[i]);
            let set = |s: &Option<ast::Shared<HashSet<Vec<u8>>>>| {
                s.as_ref().map(|s| {
                    let mut s: Vec<_> = s
                        .read()
                        .iter()
                        .map(|v| v.iter().map(|b| format!("{b:02x}")).collect::<String>())
                        .collect();
                    s.sort();
                    s
                })
            };
            gs.push(json!({"root":r.glob(g.root),"context":r.context(&g.context),"appliedFields":set(&g.applied_fields),"appliedEdges":set(&g.applied_edges)}));
            i += 1;
        }
        let path = self
            .key
            .read()
            .edges
            .as_ref()
            .unwrap()
            .get(0)
            .unwrap()
            .read()
            .as_ref()
            .unwrap()
            .read()
            .src
            .clone()
            .unwrap();
        let rr:Vec<_>=r.refs.clone().iter().map(|v|match v{
   Reference::Field(v)=>{let v=v.read();json!({"kind":"field","context":r.context(&v.context),"glob":v.due_to_glob,"lazy":v.due_to_lazy_glob,"path_alias":v.key_path.as_ref().is_some_and(|v|v.ptr_eq(&path))})},
   Reference::Edge(v)=>{let v=v.read();json!({"kind":"edge","context":r.context(&v.context),"glob":v.due_to_glob,"lazy":v.due_to_lazy_glob})}
  }).collect();
        let edge = self
            .key
            .read()
            .edges
            .as_ref()
            .unwrap()
            .get(0)
            .unwrap()
            .read()
            .clone()
            .unwrap();
        let cc:Vec<_>=r.contexts.iter().map(|c|{let c=c.read();json!({"map":self.node_name(c.scope_map),"key":c.key.as_ref().is_some_and(|k|k.ptr_eq(&self.key)),"edge":c.edge.as_ref().is_some_and(|e|e.ptr_eq(&edge)),"scope":c.scope.as_ref().is_some_and(|s|s.ptr_eq(&self.scope)),"scope_ast":c.scope_ast.as_ref().is_some_and(|s|s.ptr_eq(&self.scope_ast))})}).collect();
        json!({"nodes":ns,"refs":rr,"contexts":cc,"globs":gs})
    }
}
pub fn evaluate(arena: &mut Arena, nodes: &[NodeId], root: NodeId, config: &str) -> Value {
    let flags: Vec<_> = config.split(':').collect();
    let maps: Vec<_> = nodes
        .iter()
        .copied()
        .filter(|n| matches!(arena.node(*n), Node::Map(_)))
        .collect();
    let external = arena.new_root();
    let key = ast::Shared::new(d2_rs_language::parse_map_key(b"a -> b").unwrap());
    let edge = key
        .read()
        .edges
        .as_ref()
        .unwrap()
        .get(0)
        .unwrap()
        .read()
        .clone()
        .unwrap();
    let path = edge.read().src.clone().unwrap();
    let scope = ast::Shared::new(ast::Map::default());
    let scope_ast = ast::Shared::new(ast::Map::default());
    let contexts: Vec<_> = [
        Some(root),
        maps.last().copied(),
        Some(external),
        None,
        maps.last().copied(),
    ]
    .into_iter()
    .map(|scope_map| {
        ast::Shared::new(RefContext {
            key: Some(key.clone()),
            edge: Some(edge.clone()),
            scope: Some(scope.clone()),
            scope_ast: Some(scope_ast.clone()),
            scope_map,
        })
    })
    .collect();
    for (i, n) in nodes.iter().enumerate() {
        match arena.node_mut(*n) {
            Node::Field(f) => {
                f.references = None;
                if i % 3 == 1 {
                    f.references = Some(Vec::new());
                }
                if i % 3 == 2 || i == 0 {
                    let context = contexts.get(i % 6).cloned();
                    let r = ast::Shared::new(FieldReference {
                        string: f.name.unbox(),
                        key_path: Some(path.clone()),
                        context,
                        due_to_lazy_glob: true,
                        ..Default::default()
                    });
                    f.references = Some(vec![r.clone(), r]);
                }
            }
            Node::Edge(e) => {
                let r = ast::Shared::new(EdgeReference {
                    context: Some(contexts[i % 5].clone()),
                    due_to_lazy_glob: true,
                    ..Default::default()
                });
                e.references = Some(vec![r.clone(), r]);
            }
            _ => {}
        }
    }
    let globs: Vec<_> = (0..3)
        .map(|_| arena.insert_glob(GlobContext::default()))
        .collect();
    for (i, id) in globs.iter().enumerate() {
        let sets = |j| {
            if (i + j) % 3 == 2 {
                None
            } else {
                let mut s = HashSet::new();
                if (i + j) % 3 == 0 {
                    s.insert(b"a".to_vec());
                    s.insert(vec![255]);
                }
                Some(ast::Shared::new(s))
            }
        };
        *arena.glob_mut(*id) = GlobContext {
            root: Some(globs[[1, 0, 1][i]]),
            context: Some(contexts[[0, 1, 4][i]].clone()),
            applied_fields: sets(0),
            applied_edges: sets(1),
        };
    }
    for (i, id) in maps.iter().enumerate() {
        let Node::Map(m) = arena.node_mut(*id) else {
            panic!()
        };
        m.globs = match flags[0] {
            "none" => None,
            "empty" => Some(Vec::new().into()),
            "nil" => Some(vec![None].into()),
            "shared" => {
                Some(vec![Some(globs[i % 3]), Some(globs[2]), None, Some(globs[i % 3])].into())
            }
            _ => panic!("glob mode"),
        };
    }
    let source = if flags[2] == "field" {
        let Node::Map(m) = arena.node(root) else {
            panic!()
        };
        m.fields
            .as_ref()
            .and_then(|f| f.first())
            .copied()
            .unwrap_or(nodes[0])
    } else {
        root
    };
    let selected = arena.can_select_import(source);
    let copy = if flags[1] == "ordinary" {
        arena.copy(source, None)
    } else if matches!(arena.node(source), Node::Map(_)) {
        arena.clone_import_map(source)
    } else {
        arena.clone_import_field(source)
    };
    let mut copied = Vec::new();
    if matches!(arena.node(copy), Node::Map(_)) {
        copied.push(arena.parent(copy).unwrap());
    }
    fn visit(arena: &Arena, n: NodeId, nodes: &mut Vec<NodeId>) {
        nodes.push(n);
        for c in arena.children(n) {
            visit(arena, c, nodes);
        }
    }
    visit(arena, copy, &mut copied);
    let p = Probe {
        source: nodes.to_vec(),
        copy: copied.clone(),
        key,
        scope,
        scope_ast,
    };
    let mut registry = Registry::default();
    let before = p.snapshot(arena, &mut registry);
    fn mutate_glob(arena: &mut Arena, id: Option<GlobId>, seen: &mut HashSet<GlobId>) {
        let Some(id) = id else { return };
        if !seen.insert(id) {
            return;
        }
        let g = arena.glob_mut(id);
        g.applied_fields
            .get_or_insert_with(|| ast::Shared::new(HashSet::new()))
            .write()
            .insert(b"changed".to_vec());
        if let Some(c) = &g.context {
            c.write().scope_map = None;
        }
        let root = g.root;
        mutate_glob(arena, root, seen);
    }
    let mut seen = HashSet::new();
    for n in copied {
        match arena.node(n) {
            Node::Field(f) => {
                if let Some(r) = f.references.as_ref().and_then(|r| r.first()) {
                    let mut r = r.write();
                    r.due_to_glob = true;
                    if let Some(c) = &r.context {
                        c.write().scope_map = None;
                    }
                }
            }
            Node::Edge(e) => {
                if let Some(r) = e.references.as_ref().and_then(|r| r.first()) {
                    let mut r = r.write();
                    r.due_to_glob = true;
                    if let Some(c) = &r.context {
                        c.write().scope_map = None;
                    }
                }
            }
            Node::Map(m) => {
                let g = m.globs.clone();
                for slot in g.iter().flat_map(ast::SharedSlice::iter) {
                    mutate_glob(arena, *slot.read(), &mut seen);
                }
                if let Some(slot) = g.as_ref().and_then(|g| g.get(0)) {
                    *slot.write() = None;
                }
            }
            _ => {}
        }
    }
    json!({"selected":selected,"copy_selected":arena.can_select_import(copy),"before":before,"after":p.snapshot(arena,&mut registry)})
}
