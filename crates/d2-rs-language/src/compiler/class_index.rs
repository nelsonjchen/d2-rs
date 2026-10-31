// SPDX-License-Identifier: MPL-2.0
//! d2compiler/compile.go getClassMap (570–651): one cancelable index per root.
use super::CompileContext;
use d2_rs_core::{LayoutError, ast, ir};
use std::collections::HashMap;

#[derive(Default)]
pub(super) struct ClassIndexState {
    roots: HashMap<ir::NodeId, HashMap<String, Option<ir::NodeId>>>,
}
impl ClassIndexState {
    pub(super) fn get(
        &mut self,
        arena: &ir::Arena,
        root: ir::NodeId,
        name: &[u8],
        mut check: impl FnMut() -> Option<LayoutError>,
    ) -> Result<Option<ir::NodeId>, LayoutError> {
        if let std::collections::hash_map::Entry::Vacant(entry) = self.roots.entry(root) {
            let mut index = HashMap::new();
            if let Some(classes) = arena
                .get_field(root, &[ast::raw_string_box("classes", true)])
                .and_then(|field| arena.map(field))
            {
                let ir::Node::Map(map) = arena.node(classes) else {
                    unreachable!()
                };
                for field in map.fields.iter().flatten() {
                    if let Some(error) = check() {
                        return Err(error);
                    }
                    let ir::Node::Field(value) = arena.node(*field) else {
                        continue;
                    };
                    if value.name.unbox().is_none() {
                        continue;
                    }
                    let bytes = value.name.scalar_bytes();
                    let lower = ir::go_lowercase(&bytes);
                    if value.name.unquoted_string.is_none()
                        && ast::keywords::is_reserved_keyword(&String::from_utf8_lossy(&lower))
                    {
                        continue;
                    }
                    // Preserve the first declaration, including a nil map.
                    index
                        .entry(ir::fold_string(&bytes))
                        .or_insert_with(|| arena.map(*field));
                }
            }
            if let Some(error) = check() {
                return Err(error);
            }
            // Publication is atomic: no partial index survives a failed check.
            entry.insert(index);
        }
        Ok(self.roots[&root]
            .get(&ir::fold_string(name))
            .copied()
            .flatten())
    }
}

/// Class lookups over an immutable IR arena. Reuse the session across queries;
/// its borrow prevents IR edits from making the cached root indexes stale.
/// Context checks occur while building each index, including empty indexes.
/// A failed construction retains its typed cause and publishes no partial index.
pub struct ClassLookup<'a> {
    arena: &'a ir::Arena,
    state: ClassIndexState,
    context: Option<CompileContext>,
    error: Option<LayoutError>,
}
impl<'a> ClassLookup<'a> {
    pub fn new(arena: &'a ir::Arena, context: Option<CompileContext>) -> Self {
        Self {
            arena,
            state: ClassIndexState::default(),
            context,
            error: None,
        }
    }
    pub fn get(&mut self, map: ir::NodeId, name: &[u8]) -> Result<Option<ir::NodeId>, LayoutError> {
        let root = self.arena.root_map(map);
        let error = &mut self.error;
        let context = &mut self.context;
        self.state.get(self.arena, root, name, || {
            if error.is_none() {
                *error = context.as_mut().and_then(|check| check());
            }
            error.clone()
        })
    }
    pub fn indexed_root_count(&self) -> usize {
        self.state.roots.len()
    }
    pub fn error(&self) -> Option<&LayoutError> {
        self.error.as_ref()
    }
}
