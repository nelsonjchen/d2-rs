// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Pinned d2oracle/edit.go:937–1697, Delete and reference mutation helpers.
use super::*;

fn parts(path: &Shared<KeyPath>) -> Vec<Option<Shared<ast::StringBox>>> {
    path.read()
        .path
        .iter()
        .flat_map(SharedSlice::iter)
        .map(|p| p.read().clone())
        .collect()
}
fn names(path: &Option<Shared<KeyPath>>) -> Vec<String> {
    path.as_ref()
        .and_then(|p| p.read().string_ida())
        .unwrap_or_default()
}
fn remove(map: &Shared<Map>, key: &Shared<Key>) -> bool {
    let mut m = map.write();
    let mut nodes = map_nodes(&m);
    let Some(index) = nodes
        .iter()
        .position(|n| n.map_key.as_ref().is_some_and(|k| k.ptr_eq(key)))
    else {
        return false;
    };
    nodes.remove(index);
    m.nodes = Some(nodes.into());
    true
}
fn append(map: &Shared<Map>, key: Key) {
    // Delete appends null overrides even when other references already exist.
    let key = Shared::new(key);
    map.write().insert_after(None, Some(Child::Key(key)));
    let mut m = map.write();
    if !m.is_file_map() && m.range.one_line() {
        m.range.end.line += 1;
    }
}
pub(super) fn imported(refs: &[Reference], base: &Shared<Map>) -> bool {
    refs.iter()
        .any(|r| r.path.read().has_glob() || r.path.read().range.path != base.read().range.path)
}
pub(super) fn writable(r: &Reference, base: &Shared<Map>) -> bool {
    r.scope_ast.as_ref().is_some_and(|s| s.ptr_eq(base))
        && r.path.read().range.path == base.read().range.path
        && r.key.read().edges.as_ref().is_none_or(|e| e.is_empty())
}

#[derive(Clone)]
pub(super) struct EdgeRef {
    pub(super) key: Shared<Key>,
    pub(super) edge: Shared<ast::Edge>,
    pub(super) scope: Shared<Map>,
    pub(super) scope_path: Vec<String>,
    pub(super) scope_ast: Option<Shared<Map>>,
}
pub(super) fn edge_refs(arena: &ir::Arena, id: ir::NodeId) -> Vec<EdgeRef> {
    let ir::Node::Edge(edge) = arena.node(id) else {
        return Vec::new();
    };
    edge.references
        .iter()
        .flatten()
        .filter_map(|r| {
            let context = r.read().context.clone()?;
            let c = context.read();
            Some(EdgeRef {
                key: c.key.clone()?,
                edge: c.edge.clone()?,
                scope: c.scope.clone()?,
                scope_ast: c.scope_ast.clone(),
                scope_path: c
                    .scope_map
                    .and_then(|id| d2_rs_language::ir::board_ida(arena, id))
                    .unwrap_or_default()
                    .into_iter()
                    .map(|s| ast::ScalarBox::from_child(s).scalar_string())
                    .collect(),
            })
        })
        .collect()
}
pub(super) fn imported_edge(refs: &[EdgeRef], base: &Shared<Map>) -> bool {
    refs.iter().any(|r| {
        let e = r.edge.read();
        r.edge.read().range.path != base.read().range.path
            || (r.key.read().edge_index.is_none()
                && e.src
                    .iter()
                    .chain(e.dst.iter())
                    .any(|p| p.read().has_glob()))
    })
}
pub(super) fn edge_list(key: &Shared<Key>) -> Vec<Shared<ast::Edge>> {
    key.read()
        .edges
        .iter()
        .flat_map(SharedSlice::iter)
        .filter_map(|s| s.read().clone())
        .collect()
}
fn delete_edge(scope: &Shared<Map>, edge: &Shared<ast::Edge>) {
    // Locate by identity after earlier chain splits, rather than stale offsets.
    let nodes = map_nodes(&scope.read());
    for node in nodes {
        let Some(key) = node.map_key else { continue };
        let edges = edge_list(&key);
        let Some(index) = edges.iter().position(|e| e.ptr_eq(edge)) else {
            continue;
        };
        let after = edges[index + 1..].to_vec();
        key.write().edges = Some(
            edges[..index]
                .iter()
                .cloned()
                .map(Some)
                .collect::<Vec<_>>()
                .into(),
        );
        if !after.is_empty() {
            let mut new = key.read().clone();
            new.edges = Some(after.into_iter().map(Some).collect::<Vec<_>>().into());
            scope.write().insert_after(
                Some(&Child::Key(key.clone())),
                Some(Child::Key(Shared::new(new))),
            );
        }
        if index == 0 {
            remove(scope, &key);
        }
        return;
    }
}
pub(super) fn ensure_node(
    arena: &ir::Arena,
    root: ir::NodeId,
    excluded: &[Shared<ast::Edge>],
    scope_path: &[String],
    scope: &Shared<Map>,
    cursor: &Shared<Key>,
    path: &Shared<KeyPath>,
    before: bool,
) {
    let mut p = names(&cursor.read().key);
    p.extend(path.read().string_ida().unwrap_or_default());
    if p.is_empty() {
        return;
    }
    let Ok(absolute) = resolve_scoped_path(scope_path, &p) else {
        return;
    };
    let column = (1..absolute.len()).any(|length| field(arena,root,&absolute[..length])
        .and_then(|f| arena.map(f)).and_then(|m| arena.get_field(m,&[ast::raw_string_box("shape",true)]))
        .and_then(|f| arena.primary(f)).is_some_and(|s| matches!(arena.node(s),ir::Node::Scalar(s) if ["sql_table","class"].contains(&s.value.scalar_string().as_str()))));
    if let Some(object) = field(arena, root, &absolute).filter(|_| !column) {
        let persisting = references(arena, object).iter().any(|r| {
            r.edge
                .as_ref()
                .is_none_or(|e| !excluded.iter().any(|x| x.ptr_eq(e)))
        });
        if persisting {
            return;
        }
    }
    let key = Key {
        key: Some(Shared::new(KeyPath::from_strings(p))),
        ..Default::default()
    };
    if map_nodes(&scope.read()).iter().any(|n| {
        n.map_key
            .as_ref()
            .is_some_and(|k| k.read().d2_oracle_equals(&key))
    }) {
        return;
    }
    if before {
        scope.write().insert_before(
            Some(&Child::Key(cursor.clone())),
            Some(Child::Key(Shared::new(key))),
        );
    } else {
        scope.write().insert_after(
            Some(&Child::Key(cursor.clone())),
            Some(Child::Key(Shared::new(key))),
        );
    }
}
fn delete_map_field(map: &Shared<Map>, field: &str) -> bool {
    let mut deleted = false;
    // Source loop increments after a direct deletion; retain that traversal.
    let mut index = 0;
    loop {
        let nodes = map_nodes(&map.read());
        let Some(node) = nodes.get(index) else { break };
        if let Some(key) = &node.map_key {
            let p = names(&key.read().key);
            if p.first().is_some_and(|s| s == field) {
                remove(map, key);
            } else if p.first().is_some_and(|s| {
                [
                    "style",
                    "label",
                    "icon",
                    "source-arrowhead",
                    "target-arrowhead",
                ]
                .contains(&s.as_str())
            }) {
                if let Some(child) = key.read().value.map.clone() {
                    deleted |= delete_map_field(&child, field);
                    if map_nodes(&child.read()).is_empty() {
                        deleted |= remove(map, key);
                    }
                } else if p.len() == 2 && p[1] == field {
                    deleted |= remove(map, key);
                }
            }
        }
        index += 1;
    }
    deleted
}
fn delete_obj_field(
    ast: &Shared<Map>,
    arena: &ir::Arena,
    object: ir::NodeId,
    base: &Shared<Map>,
    field_name: &str,
    options: &MoveOptions,
) -> Result<bool, String> {
    let absolute = absolute_path(arena, object);
    let id = absolute.last().cloned().unwrap_or_default();
    let mut deleted = false;
    for r in references(arena, object) {
        if r.edge.is_some() || r.path.read().range.path != base.read().range.path {
            continue;
        }
        if let Some(map) = r.key.read().value.map.clone() {
            delete_map_field(&map, field_name);
        } else {
            let p = r.path.read().string_ida().unwrap_or_default();
            let matched = p.last().is_some_and(|s| s == field_name)
                && ((p.len() >= 2 && p[p.len() - 2] == id)
                    || (p.len() >= 3
                        && ["style", "label", "icon"].contains(&p[p.len() - 2].as_str())
                        && p[p.len() - 3] == id));
            if !matched {
                continue;
            }
            let saved = r.scope.read().nodes.clone();
            deleted |= remove(&r.scope, &r.key);
            let (text, _) = recompile(ast.read().clone(), options)?;
            let parsed = parse(&options.path, text.as_bytes(), None);
            let (next, next_root) = compile_with_options(Shared::new(parsed.map), options)?;
            if field(&next.arena, next_root, &absolute).is_none() {
                r.scope.write().nodes = saved;
                r.key.write().value = ValueBox::default();
                r.path.write().path = Some(parts(&r.path)[..=r.index].to_vec().into());
            }
        }
    }
    Ok(deleted)
}
fn reserved(
    ast: &Shared<Map>,
    compiler: &compiler::Compiler,
    board_root: ir::NodeId,
    base: &Shared<Map>,
    key: &mut Key,
    options: &MoveOptions,
) -> Result<bool, String> {
    let is_edge = key.edges.as_ref().is_some_and(|e| !e.is_empty());
    let target = if is_edge {
        key.edge_key.clone()
    } else {
        key.key.clone()
    };
    let p = names(&target);
    let Some(last) = p.last().filter(|s| is_reserved_keyword(s)) else {
        return Ok(false);
    };
    if is_edge {
        let snapshot = edges::Snapshot::new(compiler, board_root)?;
        let Some(edge) = snapshot
            .scope(key)
            .and_then(|scope| snapshot.find(scope, key))
        else {
            return Ok(true);
        };
        let refs = edge_refs(&compiler.arena, snapshot.origins[edge.0 as usize]);
        let mut deleted = false;
        for r in &refs {
            if edge_list(&r.key).len() > 1 || r.key.read().range.path != base.read().range.path {
                continue;
            }
            if let Some(map) = r.key.read().value.map.clone() {
                deleted |= delete_map_field(&map, last);
            } else if names(&r.key.read().edge_key).last() == Some(last) {
                deleted |= remove(&r.scope, &r.key);
            }
        }
        if !deleted && imported_edge(&refs, base) {
            key.value = ValueBox::from_child(Some(Child::Null(Shared::new(ast::Null::default()))));
            append(base, key.clone());
        }
    } else {
        let mut object = None;
        let mut nested = false;
        let mut is_imported = false;
        for (i, part) in p.iter().enumerate() {
            if is_reserved_keyword(part) {
                if part == "style" || ((part == "label" || part == "icon") && i + 1 < p.len()) {
                    nested = true;
                    continue;
                }
                if nested
                    || [
                        "near", "tooltip", "icon", "width", "height", "left", "top", "link",
                    ]
                    .contains(&part.as_str())
                {
                    let deleted = if let Some(object) = object {
                        delete_obj_field(ast, &compiler.arena, object, base, part, options)?
                    } else {
                        false
                    };
                    if !deleted && is_imported {
                        key.value = ValueBox::from_child(Some(Child::Null(Shared::new(
                            ast::Null::default(),
                        ))));
                        append(base, key.clone());
                    }
                    if nested {
                        continue;
                    }
                }
                break;
            }
            let obj = field(&compiler.arena, board_root, &p[..=i]).ok_or("object not found")?;
            is_imported = imported(&references(&compiler.arena, obj), base);
            object = Some(obj);
        }
    }
    Ok(true)
}
fn near_after_delete(map: &Shared<Map>, old: &[String], spelling: &str) {
    let nodes = map_nodes(&map.read());
    for node in nodes {
        let Some(key) = node.map_key else { continue };
        let p = names(&key.read().key);
        if p.last().is_some_and(|s| s == "near")
            && p.iter()
                .rev()
                .nth(1)
                .is_none_or(|s| s != "label" && s != "icon")
        {
            let value = key.read().value.scalar_box().scalar_string();
            if !ast::keywords::NEAR_CONSTANTS.contains(&value.as_str()) {
                if ir::equal_fold(value.as_bytes(), spelling.as_bytes()) {
                    remove(map, &key);
                } else if let Ok(parsed) = parse_map_key(value.as_bytes()) {
                    let mut target = names(&parsed.key);
                    if target.starts_with(old) && target.len() > old.len() {
                        target.remove(old.len() - 1);
                        key.write().value = ValueBox::from_child(
                            ast::raw_string_box(format_path(&target), false).unbox(),
                        );
                    }
                }
            }
        }
        if let Some(child) = key.read().value.map.clone() {
            near_after_delete(&child, old, spelling);
        }
    }
}

pub(in crate::graph_edit) fn delete(
    mut map: Map,
    spelling: &str,
    options: &MoveOptions,
) -> Result<DeleteObjectResult, GraphEditError> {
    let fail = |e: String| {
        GraphEditError::Move(format!(
            "failed to delete {}: {e}",
            d2_rs_core::quote_go_bytes(spelling.as_bytes())
        ))
    };
    let mut ast = Shared::new(map.clone());
    let (mut compiler, mut root) =
        compile_with_options(ast.clone(), options).map_err(GraphEditError::Move)?;
    validate_graphs(&compiler, root).map_err(GraphEditError::Move)?;
    let mut key = parse_map_key(spelling.as_bytes()).map_err(|e| fail(e.to_string()))?;
    if key.edges.as_ref().is_some_and(|e| e.len() > 1) {
        return Err(fail("can only delete one edge at a time".into()));
    }
    edges::trim_common(&mut key);
    let (mut selected, mut base) = board(&mut compiler.arena, root, &ast, &options.board_path)
        .ok_or_else(|| {
            fail(format!(
                "board [{}] not found",
                options.board_path.join(" ")
            ))
        })?;
    let mut base_ast = base.take().ok_or_else(|| {
        fail(format!(
            "board [{}] cannot be modified through this file",
            options.board_path.join(" ")
        ))
    })?;
    if reserved(&ast, &compiler, selected, &base_ast, &mut key, options).map_err(&fail)? {
        let (source, graph) = recompile(ast.read().clone(), options).map_err(fail)?;
        return Ok(DeleteObjectResult { source, graph });
    }
    if key.edges.as_ref().is_some_and(|e| !e.is_empty()) {
        let snapshot = edges::Snapshot::new(&compiler, selected).map_err(&fail)?;
        let edge = snapshot
            .scope(&key)
            .and_then(|scope| snapshot.find(scope, &key));
        if let Some(edge) = edge {
            let refs = edge_refs(&compiler.arena, snapshot.origins[edge.0 as usize]);
            if imported_edge(&refs, &base_ast)
                || (!options.board_path.is_empty()
                    && refs
                        .iter()
                        .any(|r| r.scope_ast.as_ref().is_none_or(|s| !s.ptr_eq(&base_ast))))
            {
                key.value =
                    ValueBox::from_child(Some(Child::Null(Shared::new(ast::Null::default()))));
                append(&base_ast, key);
            } else {
                let excluded = refs.iter().map(|r| r.edge.clone()).collect::<Vec<_>>();
                if let Some(r) = refs.first() {
                    let edge = r.edge.read();
                    if let Some(src) = &edge.src {
                        ensure_node(
                            &compiler.arena,
                            selected,
                            &excluded,
                            &r.scope_path,
                            &r.scope,
                            &r.key,
                            src,
                            true,
                        );
                    }
                    if let Some(dst) = &edge.dst {
                        ensure_node(
                            &compiler.arena,
                            selected,
                            &excluded,
                            &r.scope_path,
                            &r.scope,
                            &r.key,
                            dst,
                            false,
                        );
                    }
                }
                for r in refs.iter().rev() {
                    if r.key
                        .read()
                        .edge_index
                        .as_ref()
                        .is_none_or(|i| !i.read().glob)
                    {
                        delete_edge(&r.scope, &r.edge);
                    }
                }
                // Decrement later indexed references to the same edge family.
                let original_index = key
                    .edge_index
                    .as_ref()
                    .and_then(|i| i.read().int.clone())
                    .map(|n| *n.read())
                    .unwrap_or(0);
                let mut candidate = key.clone();
                let mut next = original_index + 1;
                loop {
                    candidate.edge_index = Some(Shared::new(EdgeIndex {
                        int: Some(Shared::new(next)),
                        ..Default::default()
                    }));
                    let Some(other) = snapshot
                        .scope(&candidate)
                        .and_then(|s| snapshot.find(s, &candidate))
                    else {
                        break;
                    };
                    for r in edge_refs(&compiler.arena, snapshot.origins[other.0 as usize]) {
                        if let Some(index) = r
                            .key
                            .read()
                            .edge_index
                            .as_ref()
                            .and_then(|i| i.read().int.clone())
                        {
                            *index.write() -= 1;
                        }
                    }
                    next += 1;
                }
            }
        }
        let (source, graph) = recompile(ast.read().clone(), options).map_err(fail)?;
        return Ok(DeleteObjectResult { source, graph });
    }
    let wanted = names(&key.key);
    let Some(mut object) = field(&compiler.arena, selected, &wanted) else {
        let (source, graph) = recompile(ast.read().clone(), options).map_err(fail)?;
        return Ok(DeleteObjectResult { source, graph });
    };
    let refs = references(&compiler.arena, object);
    if imported(&refs, &base_ast) {
        key.value = ValueBox::from_child(Some(Child::Null(Shared::new(ast::Null::default()))));
        append(&base_ast, key);
    } else {
        if options.board_path.is_empty() {
            conflicts::rename(&mut map, &wanted, options)?;
            ast = Shared::new(map);
            (compiler, root) = compile_with_options(ast.clone(), options).map_err(&fail)?;
            selected = root;
            base_ast = ast.clone();
            object = field(&compiler.arena, selected, &wanted)
                .ok_or_else(|| fail("object not found".into()))?;
        }
        let refs = references(&compiler.arena, object);
        if !options.board_path.is_empty() && refs.iter().any(|r| !writable(r, &base_ast)) {
            key.value = ValueBox::from_child(Some(Child::Null(Shared::new(ast::Null::default()))));
            append(&base_ast, key);
        } else {
            let graph = cross_scope::Graph::new(&compiler.arena, selected, base_ast.clone());
            let special = compiler.arena.map(object).and_then(|m| compiler.arena.get_field(m,&[ast::raw_string_box("shape",true)]))
                .and_then(|f| compiler.arena.primary(f)).is_some_and(|s| matches!(compiler.arena.node(s),ir::Node::Scalar(s) if ["sql_table","class"].contains(&s.value.scalar_string().as_str())));
            let excluded = refs
                .iter()
                .filter_map(|r| r.edge.clone())
                .collect::<Vec<_>>();
            let mut visited = Vec::<Shared<KeyPath>>::new();
            let original_lengths = refs
                .iter()
                .map(|r| parts(&r.path).len())
                .collect::<Vec<_>>();
            for (r, original_length) in refs.iter().zip(original_lengths).rev() {
                let terminal_edge = r.edge.is_some() && (special || r.index + 1 == original_length);
                if !terminal_edge {
                    if visited.iter().any(|p| p.ptr_eq(&r.path)) {
                        continue;
                    }
                    visited.push(r.path.clone());
                }
                if r.key.read().edges.as_ref().is_none_or(|e| e.is_empty()) {
                    let mut p = parts(&r.path);
                    if r.index >= p.len() {
                        continue;
                    }
                    let suffix = r.index + 1 == p.len();
                    if suffix {
                        r.key.write().primary = Default::default();
                    }
                    p.remove(r.index);
                    r.path.write().path = Some(p.clone().into());
                    let without_special = p
                        .iter()
                        .filter(|p| {
                            p.as_ref().is_some_and(|p| {
                                let s = p.read().scalar_string();
                                s != "_" && !is_reserved_keyword(&s)
                            })
                        })
                        .count();
                    if special {
                        remove(&r.scope, &r.key);
                    } else if without_special == 0 {
                        graph.hoist_children(&wanted, r);
                        remove(&r.scope, &r.key);
                    } else if r.key.read().value.unbox().is_none()
                        && suffix
                        && wanted.len() > 1
                        && field(&compiler.arena, selected, &wanted[..wanted.len() - 1])
                            .is_some_and(|p| references(&compiler.arena, p).len() > 1)
                    {
                        remove(&r.scope, &r.key);
                    } else if suffix {
                        if let Some(child) = r.key.read().value.map.clone() {
                            let nodes = map_nodes(&child.read());
                            for node in nodes {
                                if let Some(key) = node.map_key
                                    && names(&key.read().key)
                                        .first()
                                        .is_some_and(|s| is_reserved_keyword(s))
                                {
                                    remove(&child, &key);
                                }
                            }
                        } else {
                            r.key.write().value = ValueBox::default();
                        }
                    }
                } else if let Some(edge) = &r.edge {
                    if terminal_edge {
                        let edge_value = edge.read();
                        let other = if edge_value.dst.as_ref().is_some_and(|p| p.ptr_eq(&r.path)) {
                            edge_value.src.as_ref()
                        } else {
                            edge_value.dst.as_ref()
                        };
                        if let Some(other) = other {
                            ensure_node(
                                &compiler.arena,
                                selected,
                                &excluded,
                                &r.scope_path,
                                &r.scope,
                                &r.key,
                                other,
                                edge_value.dst.as_ref().is_some_and(|p| p.ptr_eq(&r.path)),
                            );
                        }
                        drop(edge_value);
                        delete_edge(&r.scope, edge);
                    } else {
                        let mut p = parts(&r.path);
                        p.remove(r.index);
                        r.path.write().path = Some(p.into());
                    }
                } else {
                    let mut p = parts(&r.path);
                    p.remove(r.index);
                    r.path.write().path = (!p.is_empty()).then(|| p.into());
                    if r.path.read().path.is_none() {
                        r.key.write().key = None;
                    }
                }
            }
            near_after_delete(&base_ast, &wanted, spelling);
        }
    }
    let (source, graph) = recompile(ast.read().clone(), options).map_err(fail)?;
    Ok(DeleteObjectResult { source, graph })
}
