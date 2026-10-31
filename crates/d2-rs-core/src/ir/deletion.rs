// SPDX-License-Identifier: MPL-2.0
//! Field deletion and board-base copies from pinned d2ir/d2ir.go.
use super::{Arena, Map, Node, NodeId};
use crate::ast::{ByteString, keywords::RESERVED_KEYWORD_HOLDERS};
impl Arena {
    /// Remove edges associated with the field's source contexts up to its board.
    pub fn delete_edges_for_field_references(&mut self, map: NodeId, field: NodeId) {
        let Node::Field(f) = self.node(field) else {
            panic!("expected IR field")
        };
        let refs = f.references.clone().unwrap_or_default();
        for fr in refs {
            let mut current = Some(map);
            while let Some(map) = current {
                let Node::Map(m) = self.node(map) else {
                    panic!("expected IR map")
                };
                // Go's range retains its original length and observes the live
                // backing array. removeEdge shifts its prefix but leaves a tail.
                let mut view = m.edges.clone().unwrap_or_default();
                for i in 0..view.len() {
                    let Node::Edge(edge) = self.node(view[i]) else {
                        panic!("expected IR edge")
                    };
                    let refs = edge.references.clone().unwrap_or_default();
                    let id = edge.id.clone();
                    for er in refs {
                        let matches = match (&er.read().context, &fr.read().context) {
                            (Some(a), Some(b)) => a.ptr_eq(b),
                            (None, None) => true,
                            _ => false,
                        };
                        if matches {
                            self.delete_edge(map, id.as_ref().map(|id| id.read()).as_deref());
                            let Node::Map(m) = self.node(map) else {
                                unreachable!()
                            };
                            let edges = m.edges.as_deref().unwrap_or_default();
                            view[..edges.len()].copy_from_slice(edges);
                            break;
                        }
                    }
                }
                if !self.board_kind(map).is_empty() {
                    break;
                }
                current = self.parent_map(map);
            }
        }
    }
    /// Delete the first matching field. Quote forms do not affect this lookup.
    pub fn delete_field(&mut self, map: NodeId, path: &[ByteString]) -> Option<NodeId> {
        let (name, rest) = path.split_first()?;
        let Node::Map(m) = self.node(map) else {
            panic!("expected IR map")
        };
        let fields = m.fields.clone().unwrap_or_default();
        for (i, id) in fields.into_iter().enumerate() {
            let Node::Field(f) = self.node(id) else {
                panic!("expected IR field")
            };
            f.name.unbox().expect("nil field name");
            if super::maps::fold_string(&f.name.scalar_bytes())
                != super::maps::fold_string(name.as_bytes())
            {
                continue;
            }
            if rest.is_empty() {
                self.delete_edges_for_field_references(map, id);
                self.remove_field(map, i);
                if let Some(parent) = self.parent_field(id) {
                    let Node::Field(f) = self.node(parent) else {
                        unreachable!()
                    };
                    let name = f.name.scalar_bytes();
                    if f.name.unquoted_string.is_some()
                        && RESERVED_KEYWORD_HOLDERS
                            .iter()
                            .any(|k| k.as_bytes() == name)
                    {
                        let holder = self.map(parent).expect("keyword holder has no map");
                        let Node::Map(m) = self.node(holder) else {
                            unreachable!()
                        };
                        if m.fields.as_ref().is_none_or(Vec::is_empty) {
                            let parent_map = self
                                .parent_map(parent)
                                .expect("keyword holder has no parent map");
                            let Node::Map(m) = self.node(parent_map) else {
                                unreachable!()
                            };
                            let index = m.fields.iter().flatten().position(|f| {
                                let Node::Field(f) = self.node(*f) else {
                                    unreachable!()
                                };
                                f.name.unquoted_string.is_some() && f.name.scalar_bytes() == name
                            });
                            if let Some(index) = index {
                                self.remove_field(parent_map, index);
                            }
                        }
                    }
                }
                return Some(id);
            }
            if let Some(map) = self.map(id) {
                return self.delete_field(map, rest);
            }
        }
        None
    }
    /// Copy without the first layers/scenarios/steps fields. The source is also
    /// reordered, and edges sharing those fields' contexts are removed first.
    pub fn copy_base(&mut self, map: Option<NodeId>, new_parent: Option<NodeId>) -> NodeId {
        let Some(map) = map else {
            if new_parent.is_none() {
                let id = self.new_root();
                let Node::Map(m) = self.node_mut(id) else {
                    unreachable!()
                };
                m.fields = Some(Vec::new());
                return id;
            }
            return self.insert(
                new_parent,
                Node::Map(Map {
                    fields: Some(Vec::new()),
                    ..Default::default()
                }),
            );
        };
        let Node::Map(m) = self.node(map) else {
            panic!("expected IR map")
        };
        let fields = m.fields.clone().unwrap_or_default();
        let mut base = Vec::new();
        let mut boards = [None; 3];
        for id in fields {
            let Node::Field(f) = self.node(id) else {
                panic!("expected IR field")
            };
            let index = f.name.unbox().and_then(|_| {
                let folded = super::maps::fold_string(&f.name.scalar_bytes());
                ["LAYERS", "SCENARIOS", "STEPS"]
                    .iter()
                    .position(|k| *k == folded)
            });
            if let Some(i) = index
                && boards[i].is_none()
            {
                boards[i] = Some(id);
            } else {
                base.push(id);
            }
        }
        for board in boards.into_iter().flatten() {
            self.delete_edges_for_field_references(map, board);
        }
        let Node::Map(m) = self.node_mut(map) else {
            unreachable!()
        };
        m.fields = Some(base.clone());
        let copy = self.copy(map, new_parent);
        base.extend(boards.into_iter().flatten());
        let Node::Map(m) = self.node_mut(map) else {
            unreachable!()
        };
        m.fields = Some(base);
        self.field_indexes.remove(&map);
        copy
    }
}
