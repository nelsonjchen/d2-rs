// SPDX-License-Identifier: MPL-2.0
//! Import template isolation from pinned d2ir/import_cache.go and compile.go.
use super::{Arena, Node, NodeId, RefContext};
use crate::ast::{Shared, SharedSlice};
use std::collections::{HashMap, HashSet};

/// Arena identity permits shared or cyclic glob roots without owning cycles.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GlobId(pub usize);
#[derive(Clone, Debug, Default)]
pub struct GlobContext {
    pub root: Option<GlobId>,
    pub context: Option<Shared<RefContext>>,
    pub applied_fields: Option<Shared<HashSet<Vec<u8>>>>,
    pub applied_edges: Option<Shared<HashSet<Vec<u8>>>>,
}
#[derive(Debug)]
pub struct ImportTemplate {
    pub ir: NodeId,
    selective: bool,
    selection_checked: bool,
}
impl ImportTemplate {
    pub fn new(ir: NodeId) -> Self {
        Self {
            ir,
            selective: false,
            selection_checked: false,
        }
    }
    pub fn can_select(&mut self, arena: &Arena) -> bool {
        if !self.selection_checked {
            self.selective = arena.can_select_import(self.ir);
            self.selection_checked = true;
        }
        self.selective
    }
}
impl Arena {
    /// Source globContext.copy shares applied sets and copies the root context.
    pub fn copy_glob_context(&mut self, source: GlobId) -> GlobId {
        let mut glob = self.glob(source).clone();
        let root = self.glob(glob.root.expect("nil glob root"));
        glob.context = Some(Shared::new(
            root.context
                .as_ref()
                .expect("nil root context")
                .read()
                .clone(),
        ));
        self.insert_glob(glob)
    }
    /// Scenario-local copies always allocate maps, including for nil source maps.
    pub fn copy_glob_applied(&mut self, target: GlobId, source: GlobId) {
        if target == source {
            // Go replaces each destination map before reading the source map.
            let target = self.glob_mut(target);
            target.applied_fields = Some(Shared::new(HashSet::new()));
            target.applied_edges = Some(Shared::new(HashSet::new()));
            return;
        }
        let source = self.glob(source);
        let fields = source
            .applied_fields
            .as_ref()
            .map(|s| s.read().clone())
            .unwrap_or_default();
        let edges = source
            .applied_edges
            .as_ref()
            .map(|s| s.read().clone())
            .unwrap_or_default();
        let target = self.glob_mut(target);
        target.applied_fields = Some(Shared::new(fields));
        target.applied_edges = Some(Shared::new(edges));
    }
    pub fn insert_glob(&mut self, glob: GlobContext) -> GlobId {
        let id = GlobId(self.globs.len());
        self.globs.push(glob);
        id
    }
    pub fn glob(&self, id: GlobId) -> &GlobContext {
        &self.globs[id.0]
    }
    pub fn glob_mut(&mut self, id: GlobId) -> &mut GlobContext {
        &mut self.globs[id.0]
    }
    pub fn can_select_import(&self, node: NodeId) -> bool {
        self.can_select_import_node(node, false)
    }
    fn can_select_import_node(&self, node: NodeId, in_array: bool) -> bool {
        match self.node(node) {
            Node::Map(m) => {
                !in_array
                    && m.globs.as_ref().is_none_or(SharedSlice::is_empty)
                    && m.fields
                        .iter()
                        .flatten()
                        .chain(m.edges.iter().flatten())
                        .all(|n| self.can_select_import(*n))
            }
            Node::Field(f) => {
                self.board_kind(node).is_empty()
                    && f.composite.is_none_or(|n| self.can_select_import(n))
            }
            Node::Edge(e) => e.map.is_none_or(|n| self.can_select_import(n)),
            Node::Array(a) => a
                .values
                .iter()
                .flatten()
                .all(|n| self.can_select_import_node(*n, true)),
            Node::Scalar(_) => true,
        }
    }
    pub fn clone_import_map(&mut self, source: NodeId) -> NodeId {
        assert!(
            matches!(self.node(source), Node::Map(_)),
            "expected import map"
        );
        self.clone_import(source)
    }
    pub fn clone_import_field(&mut self, source: NodeId) -> NodeId {
        assert!(
            matches!(self.node(source), Node::Field(_)),
            "expected import field"
        );
        self.clone_import(source)
    }
    fn clone_import(&mut self, source: NodeId) -> NodeId {
        let copy = self.copy(source, None);
        let mut pairs = Vec::new();
        self.collect_import_pairs(source, copy, &mut pairs);
        let maps: HashMap<_, _> = pairs
            .iter()
            .copied()
            .filter(|(n, _)| matches!(self.node(*n), Node::Map(_)))
            .collect();
        let mut contexts = HashMap::new();
        for (source, target) in &pairs {
            match self.node(*source) {
                Node::Field(f) => {
                    let refs = f
                        .references
                        .iter()
                        .flatten()
                        .map(|r| {
                            let mut r = r.read().clone();
                            r.context = clone_context(&r.context, &maps, &mut contexts);
                            Shared::new(r)
                        })
                        .collect();
                    let Node::Field(f) = self.node_mut(*target) else {
                        unreachable!()
                    };
                    f.references = Some(refs);
                }
                Node::Edge(e) => {
                    let refs = e
                        .references
                        .iter()
                        .flatten()
                        .map(|r| {
                            let mut r = r.read().clone();
                            r.context = clone_context(&r.context, &maps, &mut contexts);
                            Shared::new(r)
                        })
                        .collect();
                    let Node::Edge(e) = self.node_mut(*target) else {
                        unreachable!()
                    };
                    e.references = Some(refs);
                }
                _ => {}
            }
        }
        let mut globs = HashMap::new();
        // The Go implementation iterates a pointer-keyed map here. Source order
        // gives stable arena allocation; allocation identities are not serialized.
        for (source, target) in pairs {
            let Node::Map(m) = self.node(source) else {
                continue;
            };
            let source_globs: Vec<_> = m
                .globs
                .iter()
                .flat_map(SharedSlice::iter)
                .map(|g| *g.read())
                .collect();
            let copies = source_globs
                .into_iter()
                .map(|g| self.clone_glob(g, &maps, &mut contexts, &mut globs))
                .collect::<Vec<_>>();
            let Node::Map(m) = self.node_mut(target) else {
                unreachable!()
            };
            m.globs = Some(copies.into());
        }
        copy
    }
    fn collect_import_pairs(
        &self,
        source: NodeId,
        target: NodeId,
        pairs: &mut Vec<(NodeId, NodeId)>,
    ) {
        pairs.push((source, target));
        let children = |id| match self.node(id) {
            Node::Map(m) => m
                .fields
                .iter()
                .flatten()
                .chain(m.edges.iter().flatten())
                .copied()
                .collect::<Vec<_>>(),
            Node::Field(f) => f.composite.into_iter().collect(),
            Node::Edge(e) => e.map.into_iter().collect(),
            Node::Array(a) => a.values.clone().unwrap_or_default(),
            Node::Scalar(_) => Vec::new(),
        };
        for (source, target) in children(source).into_iter().zip(children(target)) {
            self.collect_import_pairs(source, target, pairs);
        }
    }
    fn clone_glob(
        &mut self,
        source: Option<GlobId>,
        maps: &HashMap<NodeId, NodeId>,
        contexts: &mut HashMap<usize, Shared<RefContext>>,
        globs: &mut HashMap<GlobId, GlobId>,
    ) -> Option<GlobId> {
        let source = source?;
        if let Some(copy) = globs.get(&source) {
            return Some(*copy);
        }
        let mut glob = self.glob(source).clone();
        // Import copies isolate each applied set; ordinary glob copies share it.
        glob.applied_fields = glob
            .applied_fields
            .as_ref()
            .map(|s| Shared::new(s.read().clone()));
        glob.applied_edges = glob
            .applied_edges
            .as_ref()
            .map(|s| Shared::new(s.read().clone()));
        let copy = self.insert_glob(GlobContext::default());
        globs.insert(source, copy);
        glob.context = clone_context(&glob.context, maps, contexts);
        glob.root = self.clone_glob(glob.root, maps, contexts, globs);
        *self.glob_mut(copy) = glob;
        Some(copy)
    }
}
fn clone_context(
    source: &Option<Shared<RefContext>>,
    maps: &HashMap<NodeId, NodeId>,
    copies: &mut HashMap<usize, Shared<RefContext>>,
) -> Option<Shared<RefContext>> {
    let source = source.as_ref()?;
    Some(
        copies
            .entry(source.identity())
            .or_insert_with(|| {
                let mut context = source.read().clone();
                if let Some(copy) = context.scope_map.and_then(|n| maps.get(&n)) {
                    context.scope_map = Some(*copy);
                }
                Shared::new(context)
            })
            .clone(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn template_selection_remains_cached_when_ir_changes() {
        let mut arena = Arena::default();
        let root = arena.new_root();
        let mut eligible = ImportTemplate::new(root);
        assert!(eligible.can_select(&arena));
        let Node::Map(m) = arena.node_mut(root) else {
            unreachable!()
        };
        m.globs = Some(vec![None].into());
        assert!(!arena.can_select_import(root));
        assert!(eligible.can_select(&arena));
        let mut ineligible = ImportTemplate::new(root);
        assert!(!ineligible.can_select(&arena));
        let Node::Map(m) = arena.node_mut(root) else {
            unreachable!()
        };
        m.globs = None;
        assert!(arena.can_select_import(root));
        assert!(!ineligible.can_select(&arena));
    }
}
