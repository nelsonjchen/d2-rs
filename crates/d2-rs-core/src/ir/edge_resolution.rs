// SPDX-License-Identifier: MPL-2.0
//! Relative edge paths and context-free edge queries from pinned d2ir/d2ir.go.
use super::{Arena, EdgeId, Node, NodeId};
use crate::ast::{Child, ScalarBox, ValueBox};

#[derive(Debug)]
pub struct ResolvedEdge {
    pub id: EdgeId,
    pub map: NodeId,
    pub common: Option<Vec<Option<Child>>>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvalidUnderscore;
impl std::fmt::Display for InvalidUnderscore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("invalid underscore")
    }
}
impl std::error::Error for InvalidUnderscore {}
fn underscore(node: &Option<Child>) -> bool {
    matches!(node, Some(Child::Unquoted(_)))
        && ScalarBox::from_child(node.clone()).scalar_bytes() == b"_"
}
fn count_underscores(path: &Option<Vec<Option<Child>>>) -> usize {
    // Upstream returns zero when the whole path consists of underscores.
    path.iter()
        .flatten()
        .position(|n| !underscore(n))
        .unwrap_or(0)
}

impl EdgeId {
    /// Copy the identifier, lift endpoints through their leading underscores,
    /// then detach their shared container prefix. Endpoint AST nodes stay shared.
    pub fn resolve(
        &self,
        arena: &Arena,
        mut map: NodeId,
    ) -> Result<ResolvedEdge, InvalidUnderscore> {
        let mut id = self.clone();
        let mut common = Vec::new();
        let count = count_underscores(&id.src_path).max(count_underscores(&id.dst_path));
        for _ in 0..count {
            for path in [&mut id.src_path, &mut id.dst_path] {
                let values = path.as_mut().expect("nil edge endpoint path");
                if underscore(&values[0]) {
                    values.remove(0);
                } else {
                    let field = arena
                        .parent_field(map)
                        .expect("relative edge map has no field ancestor");
                    let Node::Field(f) = arena.node(field) else {
                        unreachable!()
                    };
                    values.insert(0, f.name.unbox());
                }
            }
            map = arena.parent_map(map).ok_or(InvalidUnderscore)?;
        }
        while id.src_path.as_ref().map_or(0, Vec::len) > 1
            && id.dst_path.as_ref().map_or(0, Vec::len) > 1
        {
            let src = id.src_path.as_mut().unwrap();
            let dst = id.dst_path.as_mut().unwrap();
            if !super::path_equal(&Some(vec![src[0].clone()]), &Some(vec![dst[0].clone()]))
                || ScalarBox::from_child(src[0].clone())
                    .scalar_bytes()
                    .contains(&b'*')
            {
                break;
            }
            common.push(src.remove(0));
            dst.remove(0);
        }
        Ok(ResolvedEdge {
            id,
            map,
            common: (!common.is_empty()).then_some(common),
        })
    }
}
impl Arena {
    /// Public context-free edge lookup. Glob expansion belongs to compilation.
    pub fn get_edges(&self, map: NodeId, id: &EdgeId) -> Option<Vec<NodeId>> {
        let resolved = id.resolve(self, map).ok()?;
        if let Some(common) = resolved.common {
            let path: Vec<_> = common
                .into_iter()
                .map(|n| ValueBox::from_child(n).string_box())
                .collect();
            let field = self.get_field(resolved.map, &path)?;
            return self.get_edges(self.map(field)?, &resolved.id);
        }
        let Node::Map(m) = self.node(resolved.map) else {
            panic!("expected IR map")
        };
        let found: Vec<_> = m
            .edges
            .iter()
            .flatten()
            .copied()
            .filter(|e| {
                let Node::Edge(e) = self.node(*e) else {
                    panic!("expected IR edge")
                };
                e.id.as_ref()
                    .expect("nil edge identifier")
                    .read()
                    .matches(&resolved.id)
            })
            .collect();
        (!found.is_empty()).then_some(found)
    }
    pub fn get_edges_indexed(&mut self, map: NodeId, id: &EdgeId) -> Option<Vec<NodeId>> {
        let resolved = id.resolve(self, map).ok()?;
        if let Some(common) = resolved.common {
            let path: Vec<_> = common
                .into_iter()
                .map(|n| ValueBox::from_child(n).string_box())
                .collect();
            let field = self.get_field_indexed(resolved.map, &path)?;
            return self.get_edges_indexed(self.map(field)?, &resolved.id);
        }
        let found = self.lookup_edges_local(resolved.map, &resolved.id);
        (!found.is_empty()).then_some(found)
    }
    /// Remove only the first matching edge, retaining its parent/source records.
    pub fn delete_edge(&mut self, map: NodeId, id: Option<&EdgeId>) -> Option<NodeId> {
        let resolved = id?.resolve(self, map).ok()?;
        if let Some(common) = resolved.common {
            let path: Vec<_> = common
                .into_iter()
                .map(|n| ValueBox::from_child(n).string_box())
                .collect();
            let field = self.get_field(resolved.map, &path)?;
            return self.delete_edge(self.map(field)?, Some(&resolved.id));
        }
        let Node::Map(m) = self.node(resolved.map) else {
            panic!("expected IR map")
        };
        let index = m.edges.iter().flatten().position(|e| {
            let Node::Edge(e) = self.node(*e) else {
                panic!("expected IR edge")
            };
            e.id.as_ref()
                .expect("nil edge identifier")
                .read()
                .matches(&resolved.id)
        })?;
        Some(self.remove_edge(resolved.map, index))
    }
}
