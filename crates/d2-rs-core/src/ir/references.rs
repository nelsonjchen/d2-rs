// SPDX-License-Identifier: MPL-2.0
//! Source reference selection and lazy-glob deduplication from d2ir/d2ir.go.
use super::*;
use crate::ast::{self, Child, Shared, SharedSlice};
fn ptr_eq<T>(a: &Option<Shared<T>>, b: &Option<Shared<T>>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a.ptr_eq(b),
        (None, None) => true,
        _ => false,
    }
}
fn child_eq(a: &Option<Child>, b: &Option<Child>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a.ptr_eq(b),
        (None, None) => true,
        _ => false,
    }
}
fn edge_count(key: &ast::Key) -> usize {
    key.edges.as_ref().map_or(0, SharedSlice::len)
}
impl RefContext {
    pub fn edge_index(&self) -> isize {
        let key = self.key.as_ref().expect("nil reference key").read();
        key.edges
            .iter()
            .flat_map(SharedSlice::iter)
            .position(|edge| ptr_eq(&edge.read(), &self.edge))
            .map_or(-1, |i| i as isize)
    }
    /// Semantic context equality intentionally ignores Edge and ScopeMap.
    pub fn equals(&self, other: &Self) -> bool {
        let a = self.key.as_ref().expect("nil reference key").read();
        let b = other.key.as_ref().expect("nil reference key").read();
        if !a.equals(&b)
            || !ptr_eq(&self.scope, &other.scope)
            || !ptr_eq(&self.scope_ast, &other.scope_ast)
        {
            return false;
        }
        let suspension = |key: &ast::Key| {
            key.primary
                .suspension
                .as_ref()
                .or(key.value.suspension.as_ref())
                .map(|s| s.read().value)
        };
        suspension(&a) == suspension(&b)
    }
}
/// Identity comparison used only for reference deduplication.
pub fn same_reference_context(
    a: &Option<Shared<RefContext>>,
    b: &Option<Shared<RefContext>>,
) -> bool {
    if ptr_eq(a, b) {
        return true;
    }
    let (Some(a), Some(b)) = (a, b) else {
        return false;
    };
    let a = a.read();
    let b = b.read();
    ptr_eq(&a.key, &b.key)
        && ptr_eq(&a.edge, &b.edge)
        && ptr_eq(&a.scope, &b.scope)
        && a.scope_map == b.scope_map
        && ptr_eq(&a.scope_ast, &b.scope_ast)
}
impl FieldReference {
    pub fn primary(&self) -> bool {
        let context = self
            .context
            .as_ref()
            .expect("nil field reference context")
            .read();
        let key = context.key.as_ref().expect("nil reference key").read();
        if ptr_eq(&self.key_path, &key.key) {
            edge_count(&key) == 0 && self.key_path_index() + 1 == self.path_len()
        } else if ptr_eq(&self.key_path, &key.edge_key) {
            edge_count(&key) == 1 && self.key_path_index() + 1 == self.path_len()
        } else {
            false
        }
    }
    fn path_len(&self) -> usize {
        self.key_path
            .as_ref()
            .expect("nil reference path")
            .read()
            .path
            .as_ref()
            .map_or(0, SharedSlice::len)
    }
    pub fn key_path_index(&self) -> usize {
        let path = self.key_path.as_ref().expect("nil reference path").read();
        path.path
            .iter()
            .flat_map(SharedSlice::iter)
            .position(|slot| {
                let slot = slot.read();
                child_eq(
                    &slot
                        .as_ref()
                        .expect("nil AST path component")
                        .read()
                        .unbox(),
                    &self.string,
                )
            })
            .expect("d2ir.KeyReference.KeyPathIndex: String not in KeyPath?")
    }
    pub fn in_edge(&self) -> bool {
        self.context
            .as_ref()
            .expect("nil field reference context")
            .read()
            .edge
            .is_some()
    }
    pub fn edge_dest(&self) -> bool {
        let context = self
            .context
            .as_ref()
            .expect("nil field reference context")
            .read();
        let edge = context.edge.as_ref().expect("nil context edge").read();
        ptr_eq(&self.key_path, &edge.dst)
    }
    pub fn ast(&self) -> Option<Child> {
        self.string.clone().or_else(|| {
            self.context
                .as_ref()
                .expect("nil field reference context")
                .read()
                .scope
                .clone()
                .map(Child::Map)
        })
    }
}
impl EdgeReference {
    pub fn ast(&self) -> Option<Child> {
        self.context
            .as_ref()
            .expect("nil edge reference context")
            .read()
            .edge
            .clone()
            .map(Child::Edge)
    }
    pub fn primary(&self) -> bool {
        let context = self
            .context
            .as_ref()
            .expect("nil edge reference context")
            .read();
        let key = context.key.as_ref().expect("nil reference key").read();
        edge_count(&key) == 1 && key.edge_key.is_none()
    }
}
impl Field {
    pub fn append_reference(&mut self, reference: Shared<FieldReference>) {
        let new = reference.read();
        if new.due_to_lazy_glob
            && self.references.iter().flatten().any(|old| {
                let old = old.read();
                same_reference_context(&old.context, &new.context)
                    && ptr_eq(&old.key_path, &new.key_path)
                    && child_eq(&old.string, &new.string)
                    && old.due_to_glob == new.due_to_glob
                    && old.due_to_lazy_glob == new.due_to_lazy_glob
            })
        {
            return;
        }
        drop(new);
        self.references.get_or_insert_default().push(reference);
    }
    pub fn last_ref(&self) -> Shared<FieldReference> {
        self.references
            .as_ref()
            .and_then(|r| r.last())
            .expect("field has no references")
            .clone()
    }
    pub fn last_primary_ref(&self) -> Option<Shared<FieldReference>> {
        self.references
            .iter()
            .flatten()
            .rev()
            .find(|r| r.read().primary())
            .cloned()
    }
    pub fn last_primary_key(&self) -> Option<Shared<ast::Key>> {
        self.last_primary_ref().and_then(|r| {
            r.read()
                .context
                .as_ref()
                .expect("nil reference context")
                .read()
                .key
                .clone()
        })
    }
}
impl Edge {
    pub fn append_reference(&mut self, reference: Shared<EdgeReference>) {
        let new = reference.read();
        if new.due_to_lazy_glob
            && self.references.iter().flatten().any(|old| {
                let old = old.read();
                same_reference_context(&old.context, &new.context)
                    && old.due_to_glob == new.due_to_glob
                    && old.due_to_lazy_glob == new.due_to_lazy_glob
            })
        {
            return;
        }
        drop(new);
        self.references.get_or_insert_default().push(reference);
    }
    pub fn last_ref(&self) -> Shared<EdgeReference> {
        self.references
            .as_ref()
            .and_then(|r| r.last())
            .expect("edge has no references")
            .clone()
    }
    pub fn last_primary_ref(&self) -> Option<Shared<EdgeReference>> {
        self.references
            .iter()
            .flatten()
            .rev()
            .find(|r| {
                let r = r.read();
                !r.due_to_lazy_glob
                    && r.context
                        .as_ref()
                        .expect("nil reference context")
                        .read()
                        .key
                        .as_ref()
                        .expect("nil reference key")
                        .read()
                        .edge_key
                        .is_none()
            })
            .cloned()
    }
    pub fn last_primary_key(&self) -> Option<Shared<ast::Key>> {
        self.last_primary_ref().and_then(|r| {
            r.read()
                .context
                .as_ref()
                .expect("nil reference context")
                .read()
                .key
                .clone()
        })
    }
}
#[derive(Clone, Debug)]
pub enum Reference {
    Field(Shared<FieldReference>),
    Edge(Shared<EdgeReference>),
}
impl Reference {
    pub fn primary(&self) -> bool {
        match self {
            Self::Field(r) => r.read().primary(),
            Self::Edge(r) => r.read().primary(),
        }
    }
    pub fn due_to_glob(&self) -> bool {
        match self {
            Self::Field(r) => r.read().due_to_glob,
            Self::Edge(r) => r.read().due_to_glob,
        }
    }
    pub fn due_to_lazy_glob(&self) -> bool {
        match self {
            Self::Field(r) => r.read().due_to_lazy_glob,
            Self::Edge(r) => r.read().due_to_lazy_glob,
        }
    }
    pub fn ast(&self) -> Option<Child> {
        match self {
            Self::Field(r) => r.read().ast(),
            Self::Edge(r) => r.read().ast(),
        }
    }
    pub fn context(&self) -> Option<Shared<RefContext>> {
        match self {
            Self::Field(r) => r.read().context.clone(),
            Self::Edge(r) => r.read().context.clone(),
        }
    }
}
impl Arena {
    // Upstream gives any ancestor Field precedence over an ancestor Edge,
    // even when that edge is closer to the queried node.
    fn reference_owner(&self, id: NodeId) -> Option<NodeId> {
        match self.node(id) {
            Node::Field(_) | Node::Edge(_) => Some(id),
            _ => self.parent_field(id).or_else(|| self.parent_edge(id)),
        }
    }
    pub fn last_ref(&self, id: NodeId) -> Option<Reference> {
        match self.node(self.reference_owner(id)?) {
            Node::Field(f) => Some(Reference::Field(f.last_ref())),
            Node::Edge(e) => Some(Reference::Edge(e.last_ref())),
            _ => unreachable!(),
        }
    }
    pub fn last_primary_ref(&self, id: NodeId) -> Option<Reference> {
        match self.node(self.reference_owner(id)?) {
            Node::Field(f) => f.last_primary_ref().map(Reference::Field),
            Node::Edge(e) => e.last_primary_ref().map(Reference::Edge),
            _ => unreachable!(),
        }
    }
    pub fn last_primary_key(&self, id: NodeId) -> Option<Shared<ast::Key>> {
        self.last_primary_ref(id).and_then(|r| {
            r.context()
                .expect("nil reference context")
                .read()
                .key
                .clone()
        })
    }
}
