// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! d2oracle.move edge branch, generateUniqueKey and d2graph.HasEdge.
use super::*;
use d2_rs_core::{EdgeId, Graph, NodeId};

fn key_parts(path: &Option<Shared<KeyPath>>) -> Vec<String> {
    path.as_ref()
        .and_then(|p| p.read().string_ida())
        .unwrap_or_default()
}
fn formatted_path(path: &Option<Shared<KeyPath>>) -> String {
    path.as_ref()
        .map(|p| {
            String::from_utf8(format::format(&Child::KeyPath(p.clone()))).expect("formatted key")
        })
        .unwrap_or_default()
}
fn ast_edges(key: &Key) -> Vec<Shared<ast::Edge>> {
    key.edges
        .iter()
        .flat_map(SharedSlice::iter)
        .filter_map(|e| e.read().clone())
        .collect()
}
pub(super) fn trim_common(key: &mut Key) {
    let edges = ast_edges(key);
    if edges.len() != 1 {
        return;
    }
    let edge = edges[0].read();
    let src = edge.src.as_ref().expect("parsed source");
    let dst = edge.dst.as_ref().expect("parsed destination");
    loop {
        let mut a = src
            .read()
            .path
            .as_ref()
            .unwrap()
            .iter()
            .map(|p| p.read().clone())
            .collect::<Vec<_>>();
        let mut b = dst
            .read()
            .path
            .as_ref()
            .unwrap()
            .iter()
            .map(|p| p.read().clone())
            .collect::<Vec<_>>();
        if a.len() <= 1 || b.len() <= 1 {
            break;
        }
        if !ir::equal_fold(
            a[0].as_ref().unwrap().read().scalar_bytes().as_ref(),
            b[0].as_ref().unwrap().read().scalar_bytes().as_ref(),
        ) {
            break;
        }
        let first = a.remove(0);
        b.remove(0);
        let scope = key
            .key
            .get_or_insert_with(|| Shared::new(KeyPath::default()));
        let mut prefix = scope
            .read()
            .path
            .iter()
            .flat_map(SharedSlice::iter)
            .map(|p| p.read().clone())
            .collect::<Vec<_>>();
        prefix.push(first);
        scope.write().path = Some(prefix.into());
        src.write().path = Some(a.into());
        dst.write().path = Some(b.into());
    }
}

pub(super) struct Snapshot {
    pub(super) graph: Graph,
    pub(super) origins: Vec<ir::NodeId>,
    indices: Vec<usize>,
}
impl Snapshot {
    pub(super) fn new(compiler: &compiler::Compiler, root: ir::NodeId) -> Result<Self, String> {
        let graph = compiler::lower_graph(compiler, root).map_err(|e| e.to_string())?;
        let origins = compiler::graph_edge_origins(compiler, root);
        if graph.edges().len() != origins.len() {
            return Err("compiled edge source inventory differs from graph".into());
        }
        let indices = graph
            .edges()
            .map(|(id, _)| graph.edge_index(id).unwrap())
            .collect();
        Ok(Self {
            graph,
            indices,
            origins: origins.into_iter().map(|x| x.0).collect(),
        })
    }
    fn local_id(&self, id: NodeId) -> &str {
        let node = self.graph.node(id).unwrap();
        node.parent
            .and_then(|p| self.graph.node(p))
            .and_then(|p| node.external_id.strip_prefix(&p.external_id))
            .and_then(|s| s.strip_prefix('.'))
            .unwrap_or(&node.external_id)
    }
    pub(super) fn scope(&self, key: &Key) -> Option<Option<NodeId>> {
        let mut parent = None;
        for part in key_parts(&key.key) {
            let name = format_path(&[part]);
            parent = Some(
                self.graph
                    .nodes()
                    .find(|(id, node)| {
                        node.parent == parent
                            && ir::go_lowercase(self.local_id(*id).as_bytes())
                                == ir::go_lowercase(name.as_bytes())
                    })?
                    .0,
            );
        }
        Some(parent)
    }
    fn endpoint(
        &self,
        mut scope: Option<NodeId>,
        path: &Option<Shared<KeyPath>>,
    ) -> Option<String> {
        let parts = key_parts(path)
            .into_iter()
            .map(|p| format_path(&[p]))
            .collect::<Vec<_>>();
        if parts.is_empty() {
            return None;
        }
        if let Some(id) = scope {
            if !self.graph.is_sequence_diagram(id) {
                let mut ancestor = self.graph.node(id)?.parent;
                let mut sequence = self.graph.root_sequence_diagram.then_some(None);
                while let Some(id) = ancestor {
                    let node = self.graph.node(id)?;
                    if self.graph.is_sequence_diagram(id) {
                        sequence = Some(Some(id));
                        break;
                    }
                    ancestor = node.parent;
                }
                if let Some(sequence) = sequence {
                    if self
                        .graph
                        .nodes()
                        .any(|(id, node)| node.parent == sequence && self.local_id(id) == parts[0])
                    {
                        scope = sequence;
                    }
                }
            }
        }
        let mut start = 0;
        for (index, part) in parts.iter().enumerate() {
            if part != "_" {
                continue;
            }
            if index != 0 && parts[index - 1] != "_" {
                return None;
            }
            let current = scope?;
            if index + 1 == parts.len() {
                return None;
            }
            scope = self.graph.node(current)?.parent;
            start += 1;
        }
        let suffix = parts[start..].join(".");
        Some(
            scope
                .map(|id| format!("{}.{}", self.graph.node(id).unwrap().external_id, suffix))
                .unwrap_or(suffix),
        )
    }
    pub(super) fn find(&self, scope: Option<NodeId>, key: &Key) -> Option<EdgeId> {
        let edges = ast_edges(key);
        if edges.len() != 1 {
            return None;
        }
        let index = key
            .edge_index
            .as_ref()?
            .read()
            .int
            .as_ref()
            .map(|n| *n.read())?;
        let edge = edges[0].read();
        let src = self.endpoint(scope, &edge.src)?;
        let dst = self.endpoint(scope, &edge.dst)?;
        let src_arrow = edge.src_arrow == "<";
        let dst_arrow = edge.dst_arrow == ">";
        self.graph
            .edges()
            .find(|(id, e)| {
                if self.indices[id.0 as usize] as i64 != index {
                    return false;
                }
                let source = &self.graph.node(e.source).unwrap().external_id;
                let target = &self.graph.node(e.target).unwrap().external_id;
                let arrows = self.graph.edge_arrows(*id);
                let same = |a: &str, b: &str| ir::equal_fold(a.as_bytes(), b.as_bytes());
                (same(&src, source)
                    && same(&dst, target)
                    && src_arrow == arrows.source
                    && dst_arrow == arrows.target)
                    || (same(&src, target)
                        && same(&dst, source)
                        && src_arrow == arrows.target
                        && dst_arrow == arrows.source)
            })
            .map(|(id, _)| id)
    }
}

pub(super) enum PreparedMove {
    Edge(MoveObjectResult),
    Objects {
        old: Key,
        new: Key,
        destination_is_edge: bool,
    },
}

pub(super) fn prepare(
    compiler: &compiler::Compiler,
    root: ir::NodeId,
    board_root: ir::NodeId,
    ast: Shared<Map>,
    old_spelling: &str,
    requested_spelling: &str,
    options: &MoveOptions,
) -> Result<PreparedMove, String> {
    let mut new = parse_map_key(requested_spelling.as_bytes()).map_err(|e| e.to_string())?;
    let new_edges = ast_edges(&new);
    if new_edges.len() > 1 {
        return Err("cannot generate unique key for edge chain".into());
    }
    if new_edges.len() == 1 {
        if new
            .edge_index
            .as_ref()
            .is_none_or(|index| index.read().int.is_none())
        {
            new.edge_index = Some(Shared::new(ast::EdgeIndex {
                int: Some(Shared::new(0)),
                ..Default::default()
            }));
        }
        trim_common(&mut new);
        let selected = Snapshot::new(compiler, board_root)?;
        if let Some(scope) = selected.scope(&new) {
            while selected.find(scope, &new).is_some() {
                let index = new.edge_index.as_ref().unwrap().read().int.clone().unwrap();
                *index.write() += 1;
            }
        }
    }
    let mut old = parse_map_key(old_spelling.as_bytes()).map_err(|e| e.to_string())?;
    let old_edges = ast_edges(&old);
    trim_common(&mut old);
    trim_common(&mut new);
    if old_edges.is_empty() || old.edge_key.is_some() {
        // The source's object branch consumes mk.Key even when the map key
        // also names an edge/property. A scoped edge key can therefore rename
        // its containing object; a root edge key has no such object path.
        return Ok(PreparedMove::Objects {
            old,
            new,
            destination_is_edge: !new_edges.is_empty(),
        });
    }
    if formatted_path(&old.key) != formatted_path(&new.key) {
        return Err("moving across scopes isn't supported for edges".into());
    }
    let snapshot = Snapshot::new(compiler, root)?;
    let scope = snapshot
        .scope(&old)
        .ok_or("edge referenced by from does not exist")?;
    if old.edge_index.is_none() && old_edges.len() == 1 {
        return Err("edge key must include an explicit index".into());
    }
    let edge = snapshot
        .find(scope, &old)
        .ok_or("edge referenced by to does not exist")?;
    if snapshot.find(scope, &new).is_some() {
        return Err("to edge already exists".into());
    }
    let Some(new_edge) = new_edges.first() else {
        return Err("moving between object and edge keys is not supported".into());
    };
    let arrows = new_edge.read();
    let ir::Node::Edge(original) = compiler.arena.node(snapshot.origins[edge.0 as usize]) else {
        unreachable!()
    };
    for reference in original.references.iter().flatten().rev() {
        let context = reference
            .read()
            .context
            .clone()
            .expect("compiled edge context");
        let context = context.read();
        let key = context.key.as_ref().expect("compiled source key").read();
        let index = context.edge_index();
        let edge = key
            .edges
            .as_ref()
            .and_then(|edges| edges.get(index as usize))
            .and_then(|slot| slot.read().clone())
            .expect("compiled edge source slot");
        edge.write().src_arrow = arrows.src_arrow.clone();
        edge.write().dst_arrow = arrows.dst_arrow.clone();
    }
    let key = String::from_utf8(format::format(&Child::Key(Shared::new(new.clone()))))
        .expect("formatted edge key");
    let (source, graph) = recompile(ast.read().clone(), options)?;
    Ok(PreparedMove::Edge(MoveObjectResult { source, graph, key }))
}
