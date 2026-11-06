// SPDX-License-Identifier: MPL-2.0
//! Imported board/icon path relocation from pinned d2ir/compile.go.
use super::{Compiler, edge_creation::error, import_urls::is_remote};
use d2_rs_core::{
    ast::{self, Shared},
    ir::*,
};
fn scalar(value: &Option<ast::Child>) -> Vec<u8> {
    ast::ScalarBox::from_child(value.clone()).scalar_bytes()
}
fn join_path(dir: &[u8], value: &[u8]) -> Vec<u8> {
    if dir.is_empty() && value.is_empty() {
        return Vec::new();
    }
    let mut joined = dir.to_vec();
    if !joined.is_empty() {
        joined.push(b'/');
    }
    joined.extend(value);
    let rooted = joined.starts_with(b"/");
    let mut parts: Vec<&[u8]> = Vec::new();
    for part in joined.split(|c| *c == b'/') {
        match part {
            b"" | b"." => {}
            b".." if parts.last().is_some_and(|p| *p != b"..") => {
                parts.pop();
            }
            b".." if rooted => {}
            _ => parts.push(part),
        }
    }
    let mut result = Vec::new();
    if rooted {
        result.push(b'/');
    }
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            result.push(b'/');
        }
        result.extend(*part);
    }
    if result.is_empty() {
        result.push(b'.');
    }
    result
}
impl Compiler {
    fn set_import_link_value(&mut self, field: NodeId, value: Vec<u8>) {
        let primary = self
            .arena
            .primary(field)
            .expect("nil imported link primary");
        let Node::Scalar(primary) = self.arena.node_mut(primary) else {
            unreachable!()
        };
        primary.value = ast::ScalarBox::from_child(Some(ast::Child::Unquoted(Shared::new(
            ast::UnquotedString::flat(value),
        ))));
    }
    pub(super) fn extend_import_links(
        &mut self,
        map: NodeId,
        import_field: Option<NodeId>,
        dir: &[u8],
    ) {
        let board = self.arena.board_kind(map);
        // Keep this path mutable across sibling fields: upstream underscore
        // traversal shortens the shared local variable for subsequent links.
        let mut import_path = import_field
            .and_then(|f| crate::ir::ida(&self.arena, f))
            .unwrap_or_default();
        let Node::Map(m) = self.arena.node(map) else {
            unreachable!()
        };
        let fields = m.fields.clone().unwrap_or_default();
        for field in fields {
            let Node::Field(f) = self.arena.node(field) else {
                unreachable!()
            };
            if f.name.unbox().is_none() {
                continue;
            }
            let name = f.name.scalar_bytes();
            let unquoted = f.name.unquoted_string.is_some();
            if name == b"link" && unquoted {
                if !board.is_empty() {
                    self.errors.push(error(&self.arena.last_ref(field).unwrap().ast().unwrap(),"a board itself cannot be linked; only objects within a board can be linked"));
                    continue;
                }
                let value = self
                    .arena
                    .primary(field)
                    .map(|p| {
                        let Node::Scalar(s) = self.arena.node(p) else {
                            unreachable!()
                        };
                        s.value.scalar_bytes()
                    })
                    .expect("nil imported link primary");
                if is_remote(&value) {
                    continue;
                }
                let Ok(link) = crate::parse_key(&value) else {
                    continue;
                };
                let mut link_path = link.ida().unwrap_or_default();
                if link_path.is_empty() {
                    continue;
                }
                for id in link_path.clone().into_iter().skip(1) {
                    if scalar(&id) == b"_" && matches!(id, Some(ast::Child::Unquoted(_))) {
                        if link_path.len() < 2 || import_path.len() < 2 {
                            break;
                        }
                        link_path.remove(1);
                        import_path.truncate(import_path.len() - 2);
                    } else {
                        break;
                    }
                }
                let extended = import_path
                    .iter()
                    .chain(link_path[1..].iter())
                    .cloned()
                    .map(|s| ast::ValueBox::from_child(s).string_box())
                    .collect::<Vec<_>>();
                self.set_import_link_value(field, crate::format::format_key_path(&extended));
            }
            if name == b"icon"
                && unquoted
                && let Some(primary) = self.arena.primary(field)
            {
                let Node::Scalar(s) = self.arena.node(primary) else {
                    unreachable!()
                };
                let value = s.value.scalar_bytes();
                if value.is_empty() || is_remote(&value) {
                    continue;
                }
                self.set_import_link_value(field, join_path(dir, &value));
            }
            if let Some(map) = self.arena.map(field) {
                self.extend_import_links(map, import_field, dir);
            }
        }
    }
}
