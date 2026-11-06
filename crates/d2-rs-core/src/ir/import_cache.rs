// SPDX-License-Identifier: MPL-2.0
//! AST cloning from master d2ir/import_cache.go:13–382. Import templates are
//! mutable during compilation, so working copies must own nested records.
use crate::{LayoutError, ast::*};

/// Clone AST storage with the import cache's rules, including non-nil empty
/// container/path/interpolation slices and independently allocated integers.
pub fn clone_ast(node: &Child) -> Child {
    clone_ast_with_context(node, &mut || None).expect("background AST clone")
}

/// Clone with cooperative cancellation. An error discards the entire working
/// copy, including partially cloned arrays. The caller retains its source tree.
/// Checks follow d2ir's astCloner, including boxes and slice iterations.
pub fn clone_ast_with_context(
    node: &Child,
    check: &mut dyn FnMut() -> Option<LayoutError>,
) -> Result<Child, LayoutError> {
    AstCloner { check }.node(node)
}

struct AstCloner<'a> {
    check: &'a mut dyn FnMut() -> Option<LayoutError>,
}
impl AstCloner<'_> {
    fn active(&mut self) -> Result<(), LayoutError> {
        (self.check)().map_or(Ok(()), Err)
    }
    fn node(&mut self, node: &Child) -> Result<Child, LayoutError> {
        macro_rules! leaf {
            ($n:expr, $variant:ident) => {
                Child::$variant(Shared::new($n.read().clone()))
            };
        }
        // Comments are copied directly by their containing map/array loop.
        if let Child::Comment(n) = node {
            return Ok(leaf!(n, Comment));
        }
        if let Child::BlockComment(n) = node {
            return Ok(leaf!(n, BlockComment));
        }
        // EdgeIndex is copied inside cloneKey without a separate check.
        if let Child::EdgeIndex(n) = node {
            let mut n = n.read().clone();
            n.int = n.int.as_ref().map(|i| Shared::new(*i.read()));
            return Ok(Child::EdgeIndex(Shared::new(n)));
        }
        self.active()?;
        Ok(match node {
            Child::Null(n) => leaf!(n, Null),
            Child::Suspension(n) => leaf!(n, Suspension),
            Child::Boolean(n) => leaf!(n, Boolean),
            Child::SingleQuoted(n) => leaf!(n, SingleQuoted),
            Child::Block(n) => leaf!(n, Block),
            Child::Number(n) => {
                let mut n = n.read().clone();
                n.value = n.value.as_ref().map(|v| Shared::new(v.read().clone()));
                self.active()?;
                Child::Number(Shared::new(n))
            }
            Child::Unquoted(n) => {
                let mut n = n.read().clone();
                n.pattern = n.pattern.as_ref().filter(|p| !p.is_empty()).map(|p| {
                    p.iter()
                        .map(|s| s.read().clone())
                        .collect::<Vec<_>>()
                        .into()
                });
                n.value = Some(self.interpolation(&n.value)?);
                Child::Unquoted(Shared::new(n))
            }
            Child::DoubleQuoted(n) => {
                let mut n = n.read().clone();
                n.value = Some(self.interpolation(&n.value)?);
                Child::DoubleQuoted(Shared::new(n))
            }
            Child::KeyPath(n) => {
                let n = n.read();
                Child::KeyPath(Shared::new(KeyPath {
                    range: n.range.clone(),
                    path: Some(self.path(&n.path)?),
                }))
            }
            Child::Substitution(n) => {
                let mut n = n.read().clone();
                n.path = Some(self.path(&n.path)?);
                Child::Substitution(Shared::new(n))
            }
            Child::Import(n) => {
                let mut n = n.read().clone();
                n.path = Some(self.path(&n.path)?);
                Child::Import(Shared::new(n))
            }
            Child::Edge(n) => {
                let mut n = n.read().clone();
                n.src = self.key_path(&n.src)?;
                n.dst = self.key_path(&n.dst)?;
                Child::Edge(Shared::new(n))
            }
            Child::Key(n) => {
                let mut n = n.read().clone();
                n.key = self.key_path(&n.key)?;
                n.edge_key = self.key_path(&n.edge_key)?;
                n.primary = if let Some(primary) = n.primary.unbox() {
                    self.active()?; // cloneScalarBox
                    ScalarBox::from_child(Some(self.node(&primary)?))
                } else {
                    ScalarBox::default()
                };
                self.active()?; // cloneValueBox checks even an empty box
                n.value = ValueBox::from_child(
                    n.value.unbox().as_ref().map(|v| self.node(v)).transpose()?,
                );
                n.edge_index = n
                    .edge_index
                    .as_ref()
                    .map(|i| {
                        let Child::EdgeIndex(i) = self.node(&Child::EdgeIndex(i.clone()))? else {
                            unreachable!()
                        };
                        Ok(i)
                    })
                    .transpose()?;
                let mut edges = Vec::new();
                for edge in n.edges.iter().flat_map(SharedSlice::iter) {
                    // node(Edge) performs the per-edge cloneKey loop check.
                    let Child::Edge(edge) = self.node(&Child::Edge(
                        edge.read().as_ref().expect("nil AST edge").clone(),
                    ))?
                    else {
                        unreachable!()
                    };
                    edges.push(Some(edge));
                }
                n.edges = Some(edges.into());
                Child::Key(Shared::new(n))
            }
            Child::Map(n) => {
                let n = n.read();
                let mut nodes = Vec::new();
                for slot in n.nodes.iter().flat_map(SharedSlice::iter) {
                    self.active()?;
                    if let Some(child) = slot.read().unbox() {
                        nodes.push(MapNodeBox::from_child(Some(self.node(&child)?)));
                    }
                }
                Child::Map(Shared::new(Map {
                    range: n.range.clone(),
                    nodes: Some(nodes.into()),
                }))
            }
            Child::Array(n) => {
                let n = n.read();
                let mut nodes = Vec::new();
                for slot in n.nodes.iter().flat_map(SharedSlice::iter) {
                    self.active()?;
                    if let Some(child) = slot.read().unbox() {
                        nodes.push(ArrayNodeBox::from_child(Some(self.node(&child)?)));
                    }
                }
                Child::Array(Shared::new(Array {
                    range: n.range.clone(),
                    nodes: Some(nodes.into()),
                }))
            }
            Child::Comment(_) | Child::BlockComment(_) | Child::EdgeIndex(_) => unreachable!(),
        })
    }
    fn key_path(
        &mut self,
        path: &Option<Shared<KeyPath>>,
    ) -> Result<Option<Shared<KeyPath>>, LayoutError> {
        path.as_ref()
            .map(|p| {
                let Child::KeyPath(p) = self.node(&Child::KeyPath(p.clone()))? else {
                    unreachable!()
                };
                Ok(p)
            })
            .transpose()
    }
    fn path(
        &mut self,
        path: &Option<SharedSlice<Option<Shared<StringBox>>>>,
    ) -> Result<SharedSlice<Option<Shared<StringBox>>>, LayoutError> {
        let mut slots = Vec::new();
        for slot in path.iter().flat_map(SharedSlice::iter) {
            self.active()?;
            let child = slot.read().as_ref().and_then(|s| s.read().unbox());
            slots.push(if let Some(child) = child {
                self.active()?; // cloneStringBox
                Some(Shared::new(
                    ValueBox::from_child(Some(self.node(&child)?)).string_box(),
                ))
            } else {
                None
            });
        }
        Ok(slots.into())
    }
    fn interpolation(
        &mut self,
        parts: &Option<SharedSlice<InterpolationBox>>,
    ) -> Result<SharedSlice<InterpolationBox>, LayoutError> {
        self.active()?;
        let mut values = Vec::new();
        for part in parts.iter().flat_map(SharedSlice::iter) {
            self.active()?;
            let part = part.read();
            values.push(InterpolationBox {
                string: part.string.as_ref().map(|s| Shared::new(s.read().clone())),
                raw_string: part
                    .raw_string
                    .as_ref()
                    .map(|s| Shared::new(s.read().clone())),
                substitution: part
                    .substitution
                    .as_ref()
                    .map(|s| {
                        let Child::Substitution(s) = self.node(&Child::Substitution(s.clone()))?
                        else {
                            unreachable!()
                        };
                        Ok(s)
                    })
                    .transpose()?,
            });
        }
        Ok(values.into())
    }
}
