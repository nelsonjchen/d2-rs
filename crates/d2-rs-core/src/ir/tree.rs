// SPDX-License-Identifier: MPL-2.0
//! IR ownership and parent navigation from pinned d2ir/d2ir.go.
use super::EdgeId;
use crate::ast::{self, Child, ScalarBox, Shared, StringBox};

/// An identity within one IR arena. Removed fields can retain their source
/// references: arena slots are never recycled while that IR is alive.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NodeId(pub usize);
#[derive(Clone, Debug, Default)]
pub struct RefContext {
    pub edge: Option<Shared<ast::Edge>>,
    pub key: Option<Shared<ast::Key>>,
    pub scope: Option<Shared<ast::Map>>,
    pub scope_map: Option<NodeId>,
    pub scope_ast: Option<Shared<ast::Map>>,
}
#[derive(Clone, Debug, Default)]
pub struct FieldReference {
    pub string: Option<Child>,
    pub key_path: Option<Shared<ast::KeyPath>>,
    pub context: Option<Shared<RefContext>>,
    pub due_to_glob: bool,
    pub due_to_lazy_glob: bool,
}
#[derive(Clone, Debug, Default)]
pub struct EdgeReference {
    pub context: Option<Shared<RefContext>>,
    pub due_to_glob: bool,
    pub due_to_lazy_glob: bool,
}
#[derive(Clone, Debug, Default)]
pub struct Scalar {
    pub value: ScalarBox,
}
#[derive(Clone, Debug, Default)]
pub struct Field {
    pub import_ast: Option<Child>,
    pub suspended: bool,
    pub name: StringBox,
    pub primary: Option<NodeId>,
    pub composite: Option<NodeId>,
    pub references: Option<Vec<Shared<FieldReference>>>,
}
#[derive(Clone, Debug, Default)]
pub struct Edge {
    pub import_ast: Option<Child>,
    pub suspended: bool,
    pub id: Option<Shared<EdgeId>>,
    pub primary: Option<NodeId>,
    pub map: Option<NodeId>,
    pub references: Option<Vec<Shared<EdgeReference>>>,
}
#[derive(Clone, Debug, Default)]
pub struct Array {
    pub values: Option<Vec<NodeId>>,
}
#[derive(Clone, Debug, Default)]
pub struct Map {
    pub import_ast: Option<Child>,
    pub fields: Option<Vec<NodeId>>,
    pub edges: Option<Vec<NodeId>>,
    pub structure_version: u64,
    pub globs: Option<ast::SharedSlice<Option<super::GlobId>>>,
}
#[derive(Clone, Debug)]
pub enum Node {
    Scalar(Scalar),
    Field(Field),
    Edge(Edge),
    Array(Array),
    Map(Map),
}
impl Node {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Scalar(_) => "scalar",
            Self::Field(_) => "field",
            Self::Edge(_) => "edge",
            Self::Array(_) => "array",
            Self::Map(_) => "map",
        }
    }
}
#[derive(Debug)]
struct Entry {
    parent: Option<NodeId>,
    node: Node,
}
#[derive(Debug, Default)]
pub struct Arena {
    nodes: Vec<Entry>,
    nil_scalar: Option<NodeId>,
    pub(super) globs: Vec<super::GlobContext>,
    pub(super) field_indexes: std::collections::HashMap<NodeId, super::maps::FieldIndex>,
    pub(super) edge_indexes: std::collections::HashMap<NodeId, super::maps::EdgeIndex>,
}
impl Arena {
    /// Fork IR storage while retaining node identities and read-only source
    /// references. Graph compilation can merge canonical object attributes
    /// in this private view without changing lexical IR or reference scopes.
    pub fn fork_storage(&self) -> Self {
        Self {
            nodes: self
                .nodes
                .iter()
                .map(|entry| Entry {
                    parent: entry.parent,
                    node: entry.node.clone(),
                })
                .collect(),
            nil_scalar: self.nil_scalar,
            globs: self.globs.clone(),
            field_indexes: Default::default(),
            edge_indexes: Default::default(),
        }
    }

    /// A Go Value interface can hold a typed nil *Scalar. Keep one identity
    /// without inventing an AST null value or eagerly rejecting its container.
    pub fn nil_scalar(&mut self) -> NodeId {
        if let Some(id) = self.nil_scalar {
            return id;
        }
        let id = self.insert(None, Node::Scalar(Scalar::default()));
        self.nil_scalar = Some(id);
        id
    }
    pub fn is_nil_scalar(&self, id: NodeId) -> bool {
        self.nil_scalar == Some(id)
    }
    pub fn insert(&mut self, parent: Option<NodeId>, node: Node) -> NodeId {
        let id = NodeId(self.nodes.len());
        self.nodes.push(Entry { parent, node });
        id
    }
    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id.0].node
    }
    pub fn node_mut(&mut self, id: NodeId) -> &mut Node {
        assert!(!self.is_nil_scalar(id), "nil scalar mutation");
        &mut self.nodes[id.0].node
    }
    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        assert!(!self.is_nil_scalar(id), "nil scalar parent");
        self.nodes[id.0].parent
    }
    /// Root maps have a synthetic parent field, whose first reference points
    /// back to the map. The field does not own the root as its composite.
    pub fn new_root(&mut self) -> NodeId {
        let map = self.insert(None, Node::Map(Map::default()));
        self.init_root(map);
        map
    }
    fn init_root(&mut self, map: NodeId) {
        let field = self.insert(
            None,
            Node::Field(Field {
                name: ast::raw_string_box("root", true),
                references: Some(vec![Shared::new(FieldReference {
                    context: Some(Shared::new(RefContext {
                        scope_map: Some(map),
                        ..Default::default()
                    })),
                    ..Default::default()
                })]),
                ..Default::default()
            }),
        );
        self.nodes[map.0].parent = Some(field);
    }
    /// Copy mutable IR descendants into this arena. Source references and field
    /// names remain shared; scalar ASTs and edge identifier storage are copied.
    pub fn copy(&mut self, source: NodeId, new_parent: Option<NodeId>) -> NodeId {
        assert!(!self.is_nil_scalar(source), "nil scalar copy");
        let mut node = self.node(source).clone();
        let id = self.insert(new_parent, Node::Scalar(Scalar::default()));
        match &mut node {
            Node::Scalar(s) => {
                s.value = ScalarBox::from_child(s.value.unbox().as_ref().map(super::clone_ast))
            }
            Node::Field(f) => {
                f.references = f.references.take().filter(|r| !r.is_empty());
                f.primary = f.primary.map(|n| self.copy(n, Some(id)));
                f.composite = f.composite.map(|n| self.copy(n, Some(id)));
            }
            Node::Edge(e) => {
                e.id = e.id.as_ref().map(|i| Shared::new(i.read().clone()));
                e.references = e.references.take().filter(|r| !r.is_empty());
                e.primary = e.primary.map(|n| self.copy(n, Some(id)));
                e.map = e.map.map(|n| self.copy(n, Some(id)));
            }
            Node::Array(a) => {
                a.values = a
                    .values
                    .take()
                    .filter(|v| !v.is_empty())
                    .map(|v| v.into_iter().map(|n| self.copy(n, Some(id))).collect());
            }
            Node::Map(m) => {
                m.structure_version = 0;
                m.fields = Some(
                    m.fields
                        .take()
                        .unwrap_or_default()
                        .into_iter()
                        .map(|n| self.copy(n, Some(id)))
                        .collect(),
                );
                m.edges = m
                    .edges
                    .take()
                    .filter(|v| !v.is_empty())
                    .map(|v| v.into_iter().map(|n| self.copy(n, Some(id))).collect());
            }
        }
        let root = new_parent.is_none() && matches!(node, Node::Map(_));
        *self.node_mut(id) = node;
        if root {
            self.init_root(id);
        }
        id
    }
    pub fn primary(&self, id: NodeId) -> Option<NodeId> {
        if self.is_nil_scalar(id) {
            return None;
        }
        match self.node(id) {
            Node::Scalar(_) => Some(id),
            Node::Field(f) => f.primary,
            Node::Edge(e) => e.primary,
            _ => None,
        }
    }
    pub fn map(&self, id: NodeId) -> Option<NodeId> {
        match self.node(id) {
            Node::Map(_) => Some(id),
            Node::Field(f) => f.composite.and_then(|c| self.map(c)),
            Node::Edge(e) => e.map,
            _ => None,
        }
    }
    pub fn root(&self, id: NodeId) -> bool {
        match self.node(id) {
            Node::Field(_) => self.parent(id).is_none(),
            Node::Map(_) => self
                .parent(id)
                .is_some_and(|p| matches!(self.node(p), Node::Field(_)) && self.root(p)),
            _ => false,
        }
    }
    fn parent_matching(&self, mut id: NodeId, check: impl Fn(&Node) -> bool) -> Option<NodeId> {
        while let Some(parent) = self.parent(id) {
            if check(self.node(parent)) {
                return Some(parent);
            }
            id = parent;
        }
        None
    }
    pub fn parent_map(&self, id: NodeId) -> Option<NodeId> {
        self.parent_matching(id, |n| matches!(n, Node::Map(_)))
    }
    pub fn parent_field(&self, id: NodeId) -> Option<NodeId> {
        self.parent_matching(id, |n| matches!(n, Node::Field(_)))
    }
    pub fn parent_edge(&self, id: NodeId) -> Option<NodeId> {
        self.parent_matching(id, |n| matches!(n, Node::Edge(_)))
    }
    pub fn board_kind(&self, id: NodeId) -> &'static str {
        let field = match self.node(id) {
            Node::Field(_) => {
                if self.parent(id).is_none() {
                    return "layer";
                }
                self.parent_field(id)
            }
            Node::Map(_) => {
                let Some(parent) = self.parent(id) else {
                    return "";
                };
                if !matches!(self.node(parent), Node::Field(_)) {
                    return "";
                }
                if self.root(parent) {
                    return "layer";
                }
                self.parent_field(parent)
            }
            _ => None,
        };
        let Some(field) = field else {
            return "";
        };
        let Node::Field(field) = self.node(field) else {
            unreachable!()
        };
        match field.name.scalar_bytes().as_slice() {
            b"layers" => "layer",
            b"scenarios" => "scenario",
            b"steps" => "step",
            _ => "",
        }
    }
    pub fn is_var(&self, mut id: NodeId) -> bool {
        loop {
            if !self.board_kind(id).is_empty() {
                return false;
            }
            if let Node::Field(field) = self.node(id)
                && field.name.unquoted_string.is_some()
                && field.name.scalar_bytes() == b"vars"
            {
                return true;
            }
            let Some(parent) = self.parent(id) else {
                return false;
            };
            id = parent;
        }
    }
    pub fn parent_board(&self, mut id: NodeId) -> Option<NodeId> {
        while let Some(parent) = self.parent(id) {
            if !self.board_kind(parent).is_empty() {
                return Some(parent);
            }
            id = parent;
        }
        None
    }
    pub fn root_map(&self, mut id: NodeId) -> NodeId {
        while !self.root(id) {
            id = self.parent_map(id).expect("IR map has no root ancestor");
        }
        id
    }
    pub fn field_count_recursive(&self, id: NodeId) -> usize {
        self.count_recursive(id, true)
    }
    pub fn edge_count_recursive(&self, id: NodeId) -> usize {
        self.count_recursive(id, false)
    }
    fn count_recursive(&self, id: NodeId, fields: bool) -> usize {
        let Node::Map(map) = self.node(id) else {
            panic!("expected IR map")
        };
        let mut count = if fields {
            map.fields.as_ref().map_or(0, Vec::len)
        } else {
            map.edges.as_ref().map_or(0, Vec::len)
        };
        for child in map
            .fields
            .iter()
            .flatten()
            .chain(map.edges.iter().flatten())
        {
            if let Some(map) = self.map(*child) {
                count += self.count_recursive(map, fields);
            }
        }
        count
    }
    /// Structural child order, used to inspect the tree independently of any
    /// map/sort traversal in compilation. Parents and source references are excluded.
    pub fn children(&self, id: NodeId) -> Vec<NodeId> {
        match self.node(id) {
            Node::Scalar(_) => Vec::new(),
            Node::Field(f) => f.primary.into_iter().chain(f.composite).collect(),
            Node::Edge(e) => e.primary.into_iter().chain(e.map).collect(),
            Node::Array(a) => a.values.clone().unwrap_or_default(),
            Node::Map(m) => m
                .fields
                .iter()
                .flatten()
                .chain(m.edges.iter().flatten())
                .copied()
                .collect(),
        }
    }
}
