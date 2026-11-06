// SPDX-License-Identifier: MPL-2.0
//! IR-to-AST reconstruction from pinned d2ir/d2ir.go.
use super::{Arena, Node, NodeId};
use crate::ast::{self, Child, Node as _, Shared};
impl Arena {
    /// Reconstruct containers while retaining the source scalar/name objects.
    pub fn ast(&self, id: NodeId) -> Option<Child> {
        assert!(!self.is_nil_scalar(id), "nil scalar AST");
        Some(match self.node(id) {
            Node::Scalar(s) => return s.value.unbox(),
            Node::Field(f) => {
                let name = ast::ValueBox::from_child(f.name.unbox()).string_box();
                let mut key = ast::Key {
                    key: Some(Shared::new(ast::KeyPath {
                        path: Some(vec![Some(Shared::new(name))].into()),
                        ..Default::default()
                    })),
                    ..Default::default()
                };
                if let Some(primary) = f.primary {
                    key.primary = ast::ScalarBox::from_child(Some(
                        self.ast(primary).expect("nil primary AST"),
                    ));
                }
                if let Some(composite) = f.composite {
                    let value = self.ast(composite).expect("nil composite AST");
                    if let Child::Map(map) = &value {
                        let mut map = map.write();
                        let path = map.range.path.clone();
                        map.range = ast::Range::default();
                        map.range.path = path;
                        map.range.start.line = 1;
                        map.range.end.line = 2;
                    }
                    key.value = ast::ValueBox::from_child(Some(value));
                }
                Child::Key(Shared::new(key))
            }
            Node::Edge(e) => {
                let eid = e.id.as_ref().expect("nil IR edge identifier").read();
                let mut key = ast::Key {
                    edges: Some(
                        vec![Some(Shared::new(ast::Edge {
                            src: Some(canonical_path(&eid.src_path)),
                            src_arrow: if eid.src_arrow {
                                "<".into()
                            } else {
                                String::new()
                            },
                            dst: Some(canonical_path(&eid.dst_path)),
                            dst_arrow: if eid.dst_arrow {
                                ">".into()
                            } else {
                                String::new()
                            },
                            ..Default::default()
                        }))]
                        .into(),
                    ),
                    ..Default::default()
                };
                if let Some(primary) = e.primary {
                    key.primary = ast::ScalarBox::from_child(Some(
                        self.ast(primary).expect("nil edge primary AST"),
                    ));
                }
                if let Some(map) = e.map {
                    key.value =
                        ast::ValueBox::from_child(Some(self.ast(map).expect("nil edge map AST")));
                }
                Child::Key(Shared::new(key))
            }
            Node::Array(array) => {
                let nodes: Vec<_> = array
                    .values
                    .iter()
                    .flatten()
                    .map(|value| {
                        ast::ArrayNodeBox::from_child(Some(
                            self.ast(*value).expect("nil array value AST"),
                        ))
                    })
                    .collect();
                Child::Array(Shared::new(ast::Array {
                    nodes: (!nodes.is_empty()).then(|| nodes.into()),
                    ..Default::default()
                }))
            }
            Node::Map(map) => {
                let mut range = ast::Range::default();
                range.end.line = 1;
                if !self.board_kind(id).is_empty()
                    && let Some(parent) = self.parent(id)
                    && let Node::Field(field) = self.node(parent)
                {
                    range.path = field.name.unbox().expect("nil board name").range().path;
                }
                let nodes: Vec<_> = map
                    .fields
                    .iter()
                    .flatten()
                    .chain(map.edges.iter().flatten())
                    .map(|node| {
                        ast::MapNodeBox::from_child(Some(
                            self.ast(*node).expect("nil map child AST"),
                        ))
                    })
                    .collect();
                Child::Map(Shared::new(ast::Map {
                    range,
                    nodes: (!nodes.is_empty()).then(|| nodes.into()),
                }))
            }
        })
    }
}
fn canonical_path(path: &Option<Vec<Option<Child>>>) -> Shared<ast::KeyPath> {
    Shared::new(ast::KeyPath::from_strings(path.iter().flatten().map(
        |node| ast::ScalarBox::from_child(node.clone()).scalar_bytes(),
    )))
}
