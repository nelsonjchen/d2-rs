// SPDX-License-Identifier: MPL-2.0
//! IR overlays and substitution expansion from pinned d2ir/merge.go.
use super::{Arena, Node, NodeId};
impl Arena {
    fn structure_counts(&self, map: Option<NodeId>) -> (usize, usize) {
        map.map_or((0, 0), |m| {
            (self.field_count_recursive(m), self.edge_count_recursive(m))
        })
    }
    fn mark_structure_count_delta(
        &mut self,
        parent: Option<NodeId>,
        before: (usize, usize),
        after: Option<NodeId>,
    ) {
        if before != self.structure_counts(after)
            && let Some(parent) = parent
        {
            self.mark_structure_changed(parent);
        }
    }
    pub fn overlay_map(&mut self, base: NodeId, overlay: NodeId, indexed: bool) {
        let Node::Map(m) = self.node(overlay) else {
            panic!("expected overlay map")
        };
        let fields = m.fields.clone().unwrap_or_default();
        for field in fields {
            let Node::Field(f) = self.node(field) else {
                panic!("expected overlay field")
            };
            let name = f.name.clone();
            let target = if indexed {
                self.get_field_indexed(base, &[name])
            } else {
                self.get_field(base, &[name])
            };
            if let Some(target) = target {
                self.overlay_field(target, field, indexed);
            } else {
                let copy = self.copy(field, Some(base));
                self.append_field(base, copy);
            }
        }
        self.overlay_edges(base, overlay, indexed);
    }
    /// Expand fields at the placeholder's position. The placeholder itself is
    /// retained; edges are appended or overlaid independently of field position.
    pub fn expand_substitution(
        &mut self,
        map: NodeId,
        resolved: NodeId,
        placeholder: NodeId,
        indexed: bool,
    ) {
        let Node::Map(m) = self.node(map) else {
            panic!("expected destination map")
        };
        let mut index = m.fields.iter().flatten().position(|f| *f == placeholder);
        let Node::Map(m) = self.node(resolved) else {
            panic!("expected resolved map")
        };
        let fields = m.fields.clone().unwrap_or_default();
        for field in fields {
            let Node::Field(f) = self.node(field) else {
                panic!("expected resolved field")
            };
            let name = f.name.clone();
            let target = if indexed {
                self.get_field_indexed(map, &[name])
            } else {
                self.get_field(map, &[name])
            };
            if let Some(target) = target {
                self.overlay_field(target, field, indexed);
            } else {
                let copy = self.copy(field, Some(map));
                let i = index.expect("substitution placeholder is not in destination map");
                self.insert_field(map, i, copy);
                index = Some(i + 1);
            }
        }
        self.overlay_edges(map, resolved, indexed);
    }
    fn overlay_edges(&mut self, base: NodeId, overlay: NodeId, indexed: bool) {
        let Node::Map(m) = self.node(overlay) else {
            panic!("expected overlay map")
        };
        let edges = m.edges.clone().unwrap_or_default();
        for edge in edges {
            let Node::Edge(e) = self.node(edge) else {
                panic!("expected overlay edge")
            };
            let id = e.id.clone().expect("nil overlay edge identifier");
            let id = id.read();
            let targets = if indexed {
                self.get_edges_indexed(base, &id)
            } else {
                self.get_edges(base, &id)
            };
            if let Some(target) = targets.and_then(|v| v.first().copied()) {
                self.overlay_edge(target, edge, indexed);
            } else {
                let copy = self.copy(edge, Some(base));
                self.append_edge(base, copy);
            }
        }
    }
    /// Upstream retains this field-level operation as a deprecated public API.
    pub fn overlay_field(&mut self, base: NodeId, overlay: NodeId, indexed: bool) {
        let Node::Field(f) = self.node(overlay) else {
            panic!("expected overlay field")
        };
        let primary = f.primary;
        if let Some(primary) = primary {
            let copy = self.copy(primary, Some(base));
            let Node::Field(f) = self.node_mut(base) else {
                panic!("expected base field")
            };
            f.primary = Some(copy);
        }
        let Node::Field(f) = self.node(overlay) else {
            unreachable!()
        };
        let composite = f.composite;
        if let Some(composite) = composite {
            match (self.map(base), self.map(overlay)) {
                (Some(base), Some(overlay)) => self.overlay_map(base, overlay, indexed),
                _ => {
                    let before = self.structure_counts(self.map(base));
                    let copy = self.copy(composite, Some(base));
                    let Node::Field(f) = self.node_mut(base) else {
                        panic!("expected base field")
                    };
                    f.composite = Some(copy);
                    self.mark_structure_count_delta(self.parent_map(base), before, self.map(base));
                }
            }
        }
        let Node::Field(f) = self.node(overlay) else {
            unreachable!()
        };
        let refs = f.references.clone().unwrap_or_default();
        if !refs.is_empty() {
            let Node::Field(f) = self.node_mut(base) else {
                panic!("expected base field")
            };
            f.references.get_or_insert_default().extend(refs);
        }
    }
    /// Upstream retains this edge-level operation as a deprecated public API.
    pub fn overlay_edge(&mut self, base: NodeId, overlay: NodeId, indexed: bool) {
        let Node::Edge(e) = self.node(overlay) else {
            panic!("expected overlay edge")
        };
        let primary = e.primary;
        if let Some(primary) = primary {
            let copy = self.copy(primary, Some(base));
            let Node::Edge(e) = self.node_mut(base) else {
                panic!("expected base edge")
            };
            e.primary = Some(copy);
        }
        let Node::Edge(e) = self.node(overlay) else {
            unreachable!()
        };
        let map = e.map;
        if let Some(overlay_map) = map {
            if let Some(base_map) = self.map(base) {
                self.overlay_map(base_map, overlay_map, indexed);
            } else {
                let before = self.structure_counts(self.map(base));
                let copy = self.copy(overlay_map, Some(base));
                let Node::Edge(e) = self.node_mut(base) else {
                    panic!("expected base edge")
                };
                e.map = Some(copy);
                self.mark_structure_count_delta(self.parent_map(base), before, self.map(base));
            }
        }
        let Node::Edge(e) = self.node(overlay) else {
            unreachable!()
        };
        let refs = e.references.clone().unwrap_or_default();
        if !refs.is_empty() {
            let Node::Edge(e) = self.node_mut(base) else {
                panic!("expected base edge")
            };
            e.references.get_or_insert_default().extend(refs);
        }
    }
}
