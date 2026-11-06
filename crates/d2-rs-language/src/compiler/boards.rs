// SPDX-License-Identifier: MPL-2.0
//! Board inheritance and glob-frame scoping from pinned d2ir/compile.go.
use super::Compiler;
use d2_rs_core::{ast, ir::*};
use std::collections::HashSet;

/// Pinned compileBoardsField rejects a name already used by an earlier
/// layer, scenario, or step before compiling that board's attributes.
pub(super) fn validate_board_name(
    compiler: &Compiler,
    field: NodeId,
    names: &mut HashSet<String>,
) -> Option<ast::Error> {
    let Node::Field(field) = compiler.arena.node(field) else {
        return None;
    };
    let name = field.name.scalar_string();
    if names.insert(name.clone()) {
        return None;
    }
    let node = field.references.as_ref()?.first()?.read().ast()?;
    Some(super::edge_creation::error(
        &node,
        format!("board name {name} already used by another board"),
    ))
}

/// Error returned when a CLI or library board target cannot be resolved.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BoardTargetError {
    /// The target key could not be parsed using D2 key syntax.
    InvalidKey(String),
    /// A board target must start with `layers`, `scenarios`, or `steps`.
    InvalidRoot(String),
    /// A board wrapper or board name is absent from the compiled IR.
    NotFound(String),
    /// A path selected an object map rather than a board path.
    InvalidShape(String),
}

impl std::fmt::Display for BoardTargetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidKey(message) => write!(f, "invalid board target: {message}"),
            Self::InvalidRoot(target) => write!(
                f,
                "invalid board target {target:?}: path must start with layers, scenarios, or steps"
            ),
            Self::NotFound(target) => write!(f, "render target {target:?} not found"),
            Self::InvalidShape(target) => write!(
                f,
                "invalid board target {target:?}: expected alternating board wrapper and name"
            ),
        }
    }
}

impl std::error::Error for BoardTargetError {}

/// Resolve a D2 board path to its compiled IR map.
///
/// Follows pinned d2target.Diagram.GetBoard (d2target.go:108–169): wrapper
/// components are optional, and a final component matching the current board
/// selects that board. Only board children participate, never object maps.
pub fn resolve_board_target(
    compiler: &Compiler,
    root: NodeId,
    target: &str,
) -> Result<NodeId, BoardTargetError> {
    if target.is_empty() {
        return Ok(root);
    }
    let key = crate::parse_key(target.as_bytes())
        .map_err(|error| BoardTargetError::InvalidKey(error.to_string()))?;
    let parts = key
        .string_ida()
        .filter(|parts| !parts.is_empty())
        .ok_or_else(|| BoardTargetError::InvalidKey("empty key".into()))?;
    fn child(compiler: &Compiler, map: NodeId, wrapper: &str, name: &str) -> Option<NodeId> {
        let wrapper_field = compiler
            .arena
            .get_field(map, &[ast::raw_string_box(wrapper, true)])?;
        let Node::Field(field) = compiler.arena.node(wrapper_field) else {
            return None;
        };
        field.name.unquoted_string.as_ref()?;
        let wrapper_map = compiler.arena.map(wrapper_field)?;
        let board = compiler
            .arena
            .get_field(wrapper_map, &[ast::raw_string_box(name, true)])?;
        compiler.arena.map(board)
    }
    fn resolve(compiler: &Compiler, map: NodeId, parts: &[String]) -> Option<NodeId> {
        let Some(head) = parts.first() else {
            return Some(map);
        };
        if parts.len() == 1
            && !compiler.arena.root(map)
            && let Some(field) = compiler.arena.parent_field(map)
            && let Node::Field(field) = compiler.arena.node(field)
            && field.name.scalar_string() == *head
        {
            return Some(map);
        }
        if matches!(head.as_str(), "layers" | "scenarios" | "steps") {
            let name = parts.get(1)?;
            if let Some(board) = child(compiler, map, head, name) {
                return resolve(compiler, board, &parts[2..]);
            }
        }
        for wrapper in ["layers", "scenarios", "steps"] {
            if let Some(board) = child(compiler, map, wrapper, head) {
                return resolve(compiler, board, &parts[1..]);
            }
        }
        None
    }
    resolve(compiler, root, &parts).ok_or_else(|| BoardTargetError::NotFound(parts.join(".")))
}

/// Return every concrete layer/scenario/step board in source order.
///
/// The root board is intentionally omitted. Callers that package a complete
/// document can render it separately and then append these paths. The map IDs
/// are the already overlaid compiler maps, so selecting a board here preserves
/// the same inheritance and glob replay decisions as an explicit target.
pub fn board_targets(compiler: &Compiler, root: NodeId) -> Vec<(String, NodeId)> {
    collect_board_targets(compiler, root, false)
}

/// Go CLI rendering visits layers, scenarios and steps before their parent.
pub fn board_targets_postorder(compiler: &Compiler, root: NodeId) -> Vec<(String, NodeId)> {
    collect_board_targets(compiler, root, true)
}

fn collect_board_targets(
    compiler: &Compiler,
    root: NodeId,
    postorder: bool,
) -> Vec<(String, NodeId)> {
    fn visit(
        compiler: &Compiler,
        map: NodeId,
        prefix: Option<&str>,
        output: &mut Vec<(String, NodeId)>,
        postorder: bool,
    ) {
        let mut fields = match compiler.arena.node(map) {
            Node::Map(map) => map.fields.clone().unwrap_or_default(),
            _ => return,
        };
        // Original Graph/Diagram stores these as separate slices. Both public
        // preorder traversal and CLI postorder visit layers, scenarios, steps,
        // independently of the wrappers' textual declaration order.
        fields.sort_by_key(|id| match compiler.arena.node(*id) {
            Node::Field(field) => match field.name.scalar_string().as_str() {
                "layers" => 0,
                "scenarios" => 1,
                "steps" => 2,
                _ => 3,
            },
            _ => 3,
        });
        for wrapper in fields {
            let Node::Field(wrapper_field) = compiler.arena.node(wrapper) else {
                continue;
            };
            let wrapper_name = wrapper_field.name.scalar_string();
            if wrapper_field.name.unquoted_string.is_none()
                || !matches!(wrapper_name.as_str(), "layers" | "scenarios" | "steps")
            {
                continue;
            }
            let Some(wrapper_map) = compiler.arena.map(wrapper) else {
                continue;
            };
            let children = match compiler.arena.node(wrapper_map) {
                Node::Map(map) => map.fields.clone().unwrap_or_default(),
                _ => continue,
            };
            for board in children {
                let Node::Field(board_field) = compiler.arena.node(board) else {
                    continue;
                };
                let Some(board_map) = compiler.arena.map(board) else {
                    continue;
                };
                // Board names are key components, not dot-separated paths.
                let suffix = String::from_utf8(crate::format::format_key_path(&[
                    wrapper_field.name.clone(),
                    board_field.name.clone(),
                ]))
                .expect("formatted board path is UTF-8");
                let path = match prefix {
                    Some(prefix) => format!("{prefix}.{suffix}"),
                    None => suffix,
                };
                if !postorder {
                    output.push((path.clone(), board_map));
                }
                visit(compiler, board_map, Some(&path), output, postorder);
                if postorder {
                    output.push((path, board_map));
                }
            }
        }
    }

    let mut output = Vec::new();
    visit(compiler, root, None, &mut output, postorder);
    output
}

impl Compiler {
    pub(super) fn push_map_globs(&mut self, map: NodeId) {
        let mut globs = Vec::new();
        if let Some(previous) = self.glob_context_stack.last().cloned() {
            match self.arena.board_kind(map) {
                "layer" if !self.arena.root(map) => {
                    for source in previous {
                        if self.glob_is_triple(source) {
                            globs.push(self.copy_glob_to_map(source, map, false));
                        }
                    }
                }
                "scenario" => {
                    // Both source loops are intentional: scenarios append two
                    // distinct context copies, with separate non-triple sets.
                    for _ in 0..2 {
                        for source in &previous {
                            let isolate = !self.glob_is_triple(*source);
                            globs.push(self.copy_glob_to_map(*source, map, isolate));
                        }
                    }
                }
                "step" => {
                    for source in previous {
                        globs.push(self.copy_glob_to_map(source, map, false));
                    }
                }
                _ => globs = previous,
            }
        }
        self.glob_context_stack.push(globs);
    }
    fn glob_is_triple(&self, glob: GlobId) -> bool {
        self.arena
            .glob(glob)
            .context
            .as_ref()
            .unwrap()
            .read()
            .key
            .as_ref()
            .unwrap()
            .read()
            .has_triple_glob()
    }
    fn copy_glob_to_map(&mut self, source: GlobId, map: NodeId, isolate: bool) -> GlobId {
        let copy = self.arena.copy_glob_context(source);
        self.arena
            .glob(copy)
            .context
            .as_ref()
            .unwrap()
            .write()
            .scope_map = Some(map);
        if isolate {
            self.arena.copy_glob_applied(copy, source)
        }
        copy
    }
    pub(super) fn overlay_board(&mut self, base: Option<NodeId>, field: NodeId) {
        let Some(existing) = self.arena.map(field) else {
            let Node::Field(f) = self.arena.node(field) else {
                unreachable!()
            };
            let context = f.references.as_ref().unwrap()[0]
                .read()
                .context
                .clone()
                .unwrap();
            let range = context.read().key.as_ref().unwrap().read().range.clone();
            self.errors.push(ast::Error {
                message: format!("{range}: invalid {}", self.arena.board_kind(field)),
                range,
            });
            return;
        };
        let source = self.arena.last_ref(field).unwrap().ast().unwrap();
        // Go's walker checks ctx.Err before dereferencing a typed nil *Map.
        // A prior budget halt is not a context error and must not hide it.
        let Some(base) = base else {
            if let Some(error) = self.context.as_mut().and_then(|check| check()) {
                self.context_error = Some(error);
            }
            if self.context_error.is_none() {
                self.nil_board_base = true;
            }
            self.halted = true;
            return;
        };
        if !self.reserve_variable_copy(&source, base) {
            return;
        }
        let base = self.arena.copy_base(Some(base), Some(field));
        self.arena.delete_field(base, &["label".into()]);
        if !self.reserve_variable_copy(&source, existing) {
            return;
        }
        self.arena.overlay_map(base, existing, true);
        let Node::Field(f) = self.arena.node_mut(field) else {
            unreachable!()
        };
        f.composite = Some(base);
    }
    pub(super) fn inherit_board(&mut self, field: NodeId) {
        match self.arena.board_kind(field) {
            "scenario" => {
                let base = self
                    .arena
                    .parent_board(field)
                    .and_then(|b| self.arena.map(b));
                self.overlay_board(base, field)
            }
            "step" => {
                let map = self.arena.parent_map(field).unwrap();
                let Node::Map(m) = self.arena.node(map) else {
                    unreachable!()
                };
                if let Some(index) = m.fields.as_ref().unwrap().iter().position(|f| *f == field) {
                    let base = if index == 0 {
                        self.arena
                            .parent_board(field)
                            .and_then(|b| self.arena.map(b))
                    } else {
                        self.arena.map(m.fields.as_ref().unwrap()[index - 1])
                    };
                    self.overlay_board(base, field);
                }
            }
            _ => {}
        }
    }
    pub(super) fn overlay_classes(&mut self, map: NodeId) {
        let Some(classes) = self
            .arena
            .get_field_indexed(map, &[ast::raw_string_box("classes", true)])
        else {
            return;
        };
        if self.arena.map(classes).is_none() {
            return;
        }
        let Some(layers) = self
            .arena
            .get_field_indexed(map, &[ast::raw_string_box("layers", true)])
            .and_then(|f| self.arena.map(f))
        else {
            return;
        };
        let Node::Map(m) = self.arena.node(layers) else {
            unreachable!()
        };
        let fields = m.fields.clone().unwrap_or_default();
        for field in fields {
            if self.stopped() {
                return;
            }
            let Some(layer) = self.arena.map(field) else {
                continue;
            };
            if self.arena.primary(field).is_some() {
                continue;
            }
            if let Some(existing) = self
                .arena
                .get_field_indexed(layer, &[ast::raw_string_box("classes", true)])
            {
                if let Some(existing) = self.arena.map(existing) {
                    let source = self.arena.last_ref(field).unwrap().ast().unwrap();
                    if !self.reserve_variable_copy(&source, classes) {
                        return;
                    }
                    let base = self.arena.copy(classes, Some(layer));
                    if !self.reserve_variable_copy(&source, existing) {
                        return;
                    }
                    self.arena
                        .overlay_map(self.arena.map(base).unwrap(), existing, true);
                    self.arena.delete_field(layer, &["classes".into()]);
                    self.arena.append_field(layer, base);
                }
            } else {
                let source = self.arena.last_ref(field).unwrap().ast().unwrap();
                if !self.reserve_variable_copy(&source, classes) {
                    return;
                }
                let copy = self.arena.copy(classes, Some(layer));
                self.arena.append_field(layer, copy)
            }
            self.overlay_classes(layer);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nil_base_walk_checks_context_even_after_a_budget_halt() {
        for cancelled in [false, true] {
            let parsed = crate::parse("", b"a: { x }", None);
            let (mut compiler, root) =
                crate::compiler::compile_ir(ast::Shared::new(parsed.map)).unwrap();
            let field = compiler
                .arena
                .get_field(root, &[ast::raw_string_box("a", true)])
                .unwrap();
            compiler.halted = true;
            compiler.context = Some(Box::new(move || {
                cancelled.then_some(d2_rs_core::LayoutError::Cancelled)
            }));
            compiler.overlay_board(None, field);
            if cancelled {
                assert!(!compiler.nil_board_base);
                assert!(matches!(
                    compiler.context_error,
                    Some(d2_rs_core::LayoutError::Cancelled)
                ));
            } else {
                assert!(compiler.nil_board_base);
            }
        }
    }
}
