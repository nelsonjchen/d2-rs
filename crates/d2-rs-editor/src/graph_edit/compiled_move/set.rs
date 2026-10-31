// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Pinned d2oracle/edit.go:80–129,327–935: Set and scope/reference selection.
use super::delete::{edge_list, edge_refs, imported, imported_edge, writable};
use super::*;

fn names(path: &Option<Shared<KeyPath>>) -> Vec<String> {
    path.as_ref()
        .and_then(|p| p.read().string_ida())
        .unwrap_or_default()
}
fn trim(path: &mut Option<Shared<KeyPath>>, count: usize) {
    let Some(p) = path else { return };
    let parts = p
        .read()
        .path
        .iter()
        .flat_map(SharedSlice::iter)
        .map(|s| s.read().clone())
        .skip(count)
        .collect::<Vec<_>>();
    if parts.is_empty() {
        *path = None;
    } else {
        p.write().path = Some(parts.into());
    }
}
fn new_map() -> Shared<Map> {
    let mut range = ast::Range::default();
    range.start.line = 1;
    range.end.line = 1;
    Shared::new(Map { range, nodes: None })
}
fn append(map: &Shared<Map>, key: Key) {
    let reserved = names(&key.key)
        .first()
        .is_some_and(|s| is_reserved_keyword(s));
    let mut m = map.write();
    let mut nodes = map_nodes(&m);
    nodes.push(MapNodeBox {
        map_key: Some(Shared::new(key)),
        ..Default::default()
    });
    let single_reserved = nodes.len() == 1 && reserved;
    m.nodes = Some(nodes.into());
    if !single_reserved && !m.is_file_map() && m.range.one_line() {
        m.range.end.line += 1;
    }
}
fn property(arena: &ir::Arena, object: ir::NodeId, path: &[String]) -> Option<ir::NodeId> {
    let map = match arena.node(object) {
        ir::Node::Map(_) => Some(object),
        _ => arena.map(object),
    }?;
    arena.get_field(
        map,
        &path
            .iter()
            .map(|s| ast::raw_string_box(s, true))
            .collect::<Vec<_>>(),
    )
}
fn label_key(arena: &ir::Arena, object: ir::NodeId) -> Option<Shared<Key>> {
    if matches!(arena.node(object), ir::Node::Edge(_)) {
        return arena.last_primary_key(object);
    }
    property(arena, object, &["label".into()])
        .filter(|f| arena.primary(*f).is_some())
        .and_then(|f| arena.last_primary_key(f))
        .or_else(|| {
            (!matches!(arena.node(object), ir::Node::Map(_)))
                .then(|| arena.last_primary_key(object))
                .flatten()
        })
}
fn object_map(arena: &ir::Arena, object: ir::NodeId) -> Option<Shared<Map>> {
    references(arena, object)
        .iter()
        .filter(|r| r.primary)
        .filter_map(|r| r.key.read().value.map.clone())
        .last()
}
fn in_class(map: &Shared<Map>, wanted: &Shared<Key>, inside: bool) -> bool {
    let nodes = map_nodes(&map.read());
    for node in nodes {
        let Some(k) = node.map_key else { continue };
        if k.ptr_eq(wanted) {
            return inside;
        }
        let parts = names(&k.read().key);
        if let Some(child) = k.read().value.map.clone() {
            if in_class(
                &child,
                wanted,
                inside || parts.first().is_some_and(|s| s == "classes"),
            ) {
                return true;
            }
        }
    }
    false
}
fn inline(ast: &Shared<Map>, base: &Shared<Map>, key: &Shared<Key>) -> bool {
    key.read().range.path == base.read().range.path
        && !key.read().has_glob()
        && !in_class(ast, key, false)
}

fn apply(
    ast: &Shared<Map>,
    compiler: &compiler::Compiler,
    root: ir::NodeId,
    base: &Shared<Map>,
    mut mk: Key,
) -> Result<(), String> {
    let arena = &compiler.arena;
    let mut scope = base.clone();
    edges::trim_common(&mut mk);
    // HasEdge is called on the resolved object after scope trimming. Retain
    // its identity before mutating the parsed key's shared path components.
    let edge_origin = if mk.edges.as_ref().is_some_and(|e| !e.is_empty()) && mk.edge_index.is_some()
    {
        let snapshot = edges::Snapshot::new(compiler, root)?;
        snapshot
            .scope(&mk)
            .and_then(|s| snapshot.find(s, &mk))
            .map(|id| snapshot.origins[id.0 as usize])
    } else {
        None
    };
    let mut object = root;
    let mut to_skip = 1;
    let mut reserved = false;
    let mut is_imported = false;
    let mut reserved_key = String::new();
    let mut reserved_target = String::new();
    if mk.key.is_some() {
        let mut found = true;
        let path = names(&mk.key);
        let mut absolute = Vec::new();
        for part in &path {
            if is_reserved_keyword(part) {
                reserved = true;
                break;
            }
            absolute.push(part.clone());
            let Some(child) = field(arena, root, &absolute) else {
                found = false;
                break;
            };
            object = child;
            let refs = references(arena, object);
            is_imported = imported(&refs, base);
            let maybe_scope = if !base.ptr_eq(ast) || is_imported {
                refs.iter().filter(|r| writable(r, base)).find_map(|r| {
                    let key = r.key.read();
                    if key
                        .key
                        .as_ref()
                        .zip(mk.key.as_ref())
                        .is_some_and(|(a, b)| a.ptr_eq(b))
                    {
                        key.value.map.clone()
                    } else {
                        None
                    }
                })
            } else {
                object_map(arena, object)
            };
            if let Some(map) = maybe_scope {
                scope = map;
                trim(&mut mk.key, to_skip);
                to_skip = 1;
            } else {
                to_skip += 1;
            }
        }
        let remainder = names(&mk.key);
        if remainder.len() == 2 && ["layers", "scenarios", "steps"].contains(&remainder[0].as_str())
        {
            if remainder[1].contains(['.', '"']) {
                return Err("board names cannot contain dots or quotes".into());
            }
            let container = map_nodes(&scope.read())
                .iter()
                .filter_map(|n| n.map_key.as_ref())
                .find_map(|k| {
                    (names(&k.read().key) == remainder[..1])
                        .then(|| k.read().value.map.clone())
                        .flatten()
                });
            let container = container.unwrap_or_else(|| {
                let map = new_map();
                append(
                    &scope,
                    Key {
                        key: Some(Shared::new(KeyPath::from_strings([remainder[0].clone()]))),
                        value: ValueBox::from_child(Some(Child::Map(map.clone()))),
                        ..Default::default()
                    },
                );
                map
            });
            append(
                &container,
                Key {
                    key: Some(Shared::new(KeyPath::from_strings([remainder[1].clone()]))),
                    value: mk.value,
                    ..Default::default()
                },
            );
            return Ok(());
        }
        let refs = references(arena, object);
        let mut object_key = None;
        let label = label_key(arena, object);
        let label_writable = if !base.ptr_eq(ast) || is_imported {
            let refs = refs
                .iter()
                .filter(|r| writable(r, base))
                .collect::<Vec<_>>();
            object_key = refs.first().map(|r| r.key.clone());
            if object_key.is_none() {
                append(&scope, mk);
                return Ok(());
            }
            label
                .as_ref()
                .is_some_and(|l| refs.iter().any(|r| r.key.ptr_eq(l)))
        } else {
            label
                .as_ref()
                .is_some_and(|l| refs.iter().any(|r| r.key.ptr_eq(l)))
        };
        let m = object_key
            .as_ref()
            .and_then(|k| k.read().value.map.clone())
            .or_else(|| {
                object_key
                    .is_none()
                    .then(|| object_map(arena, object))
                    .flatten()
            });
        if let Some(label) = label.filter(|_| label_writable) {
            let p = names(&label.read().key);
            if m.is_none()
                && (!found || reserved || mk.edges.as_ref().is_some_and(|e| !e.is_empty()))
                && p.last().is_none_or(|s| s != "label")
            {
                let m2 = new_map();
                let objk = object_key.unwrap_or(label);
                let primary = objk.read().value.scalar_box();
                objk.write().primary = primary;
                objk.write().value = ValueBox::from_child(Some(Child::Map(m2.clone())));
                scope = m2;
                trim(&mut mk.key, to_skip - 1);
                to_skip = 1;
            }
        }
        if !found {
            append(&scope, mk);
            return Ok(());
        }
    }
    let mut attrs = object;
    let mut edge_object = None;
    if mk.edges.as_ref().is_some_and(|e| !e.is_empty()) {
        if mk.edge_index.is_none() {
            append(&scope, mk);
            return Ok(());
        }
        let origin = edge_origin.ok_or("edge not found")?;
        edge_object = Some(origin);
        let mut refs = edge_refs(arena, origin);
        is_imported = imported_edge(&refs, base);
        if !base.ptr_eq(ast) || is_imported {
            refs.retain(|r| r.scope_ast.as_ref().is_some_and(|s| s.ptr_eq(base)));
        }
        let mut only_chain = true;
        let mut earliest = None;
        for r in &refs {
            if earliest.is_none_or(|i: usize| r.key.read().range.before(&refs[i].key.read().range))
            {
                earliest = Some(refs.iter().position(|x| x.key.ptr_eq(&r.key)).unwrap());
            }
            if edge_list(&r.key).len() == 1
                && (r.key.read().edge_index.is_none() || r.key.read().value.map.is_some())
            {
                only_chain = false;
            }
            if r.key
                .read()
                .edge_index
                .as_ref()
                .is_none_or(|i| !i.read().glob)
            {
                let mut a = r.key.read().clone();
                let mut b = mk.clone();
                a.value = ValueBox::default();
                b.value = ValueBox::default();
                if a.d2_oracle_equals(&b) {
                    r.key.write().value = mk.value;
                    return Ok(());
                }
            }
        }
        if only_chain {
            if let Some(r) = earliest
                .map(|i| &refs[i])
                .filter(|r| scope.read().range.before(&r.key.read().range))
            {
                let e = r.edge.read();
                if !e.src.as_ref().is_some_and(|p| p.read().has_glob())
                    && !e.dst.as_ref().is_some_and(|p| p.read().has_glob())
                {
                    mk.edges = Some(vec![Some(r.edge.clone())].into());
                }
                r.scope.write().insert_after(
                    Some(&Child::Key(r.key.clone())),
                    Some(Child::Key(Shared::new(mk))),
                );
            } else {
                append(&scope, mk);
            }
            return Ok(());
        }
        attrs = origin;
        if let Some(edge_key) = mk.edge_key.clone() {
            let p = names(&Some(edge_key.clone()));
            if !p.first().is_some_and(|s| is_reserved_keyword(s)) {
                return Err("edge key must be reserved".into());
            }
            reserved = true;
            to_skip = 1;
            mk = Key {
                key: Some(Shared::new(edge_key.read().clone())),
                value: mk.value,
                ..Default::default()
            };
            let mut found_map = false;
            for r in &refs {
                if let Some(map) = r.key.read().value.map.clone() {
                    found_map = true;
                    scope = map;
                    let nodes = map_nodes(&scope.read());
                    for node in nodes {
                        let Some(key) = node.map_key else { continue };
                        let p = names(&key.read().key);
                        let child = key.read().value.map.clone();
                        if p.len() == 1
                            && p[0] == names(&mk.key)[to_skip - 1]
                            && let Some(child) = child
                        {
                            scope = child;
                            reserved_key = names(&mk.key)[0].clone();
                            if ["source-arrowhead", "target-arrowhead"]
                                .contains(&reserved_key.as_str())
                            {
                                if let Some(arrow) =
                                    property(arena, origin, &[reserved_key.clone()])
                                {
                                    attrs = arrow;
                                }
                            }
                            trim(&mut mk.key, 1);
                            reserved_target = names(&mk.key).first().cloned().unwrap_or_default();
                            break;
                        }
                    }
                    break;
                }
            }
            if !found_map && let Some(label) = label_key(arena, attrs) {
                let primary = label.read().value.scalar_box();
                label.write().primary = primary;
                let map = new_map();
                label.write().value = ValueBox::from_child(Some(Child::Map(map.clone())));
                scope = map;
            }
        }
    }
    if reserved {
        let p = names(&mk.key);
        if !p.is_empty() {
            let index = to_skip - 1;
            if reserved_key.is_empty() {
                reserved_key = p.get(index).cloned().unwrap_or_default();
            }
            let mut attribute_path = Vec::new();
            match reserved_key.as_str() {
                "shape" | "link" | "tooltip" | "width" | "height" | "top" | "left"
                | "grid-rows" | "grid-columns" | "grid-gap" | "vertical-gap" | "horizontal-gap" => {
                    attribute_path.push(reserved_key.clone());
                }
                "source-arrowhead" | "target-arrowhead" => {
                    if let Some(origin) = edge_object
                        && let Some(arrow) = property(arena, origin, &[reserved_key.clone()])
                    {
                        attrs = arrow;
                        if reserved_target.is_empty() {
                            if p.len().saturating_sub(index) < 2 {
                                return Err(
                                    "malformed style setting, expected >= 2 part path".into()
                                );
                            }
                            reserved_target = p[index + 1].clone();
                        }
                        match reserved_target.as_str() {
                            "shape" | "label" => attribute_path.push(reserved_target.clone()),
                            "style" => attribute_path.extend(["style".into(), "filled".into()]),
                            _ => {}
                        }
                    }
                }
                "style" => {
                    if reserved_target.is_empty() {
                        if p.len().saturating_sub(index) != 2 {
                            return Err("malformed style setting, expected 2 part path".into());
                        }
                        reserved_target = p[index + 1].clone();
                    }
                    attribute_path.extend(["style".into(), reserved_target.clone()]);
                }
                "label" => {
                    if p.len().saturating_sub(index) > 1 {
                        if p[index + 1] == "near" {
                            attribute_path.extend(["label".into(), "near".into()]);
                        }
                    } else {
                        attribute_path.push("label".into());
                    }
                }
                _ => {}
            }
            let key = if attribute_path == ["label"] {
                label_key(arena, attrs)
            } else if attribute_path.is_empty() {
                None
            } else {
                property(arena, attrs, &attribute_path).and_then(|f| arena.last_primary_key(f))
            };
            if let Some(key) = key.filter(|k| inline(ast, base, k)) {
                key.write().set_scalar(mk.value.scalar_box());
                return Ok(());
            }
        }
    } else if let Some(key) = label_key(arena, attrs) {
        key.write().set_scalar(mk.value.scalar_box());
        return Ok(());
    }
    append(&scope, mk);
    Ok(())
}

pub(in crate::graph_edit) fn set(
    map: Map,
    spelling: &str,
    tag: Option<&str>,
    value: Option<&str>,
    options: &MoveOptions,
) -> Result<SetObjectResult, GraphEditError> {
    let quote = |s: &str| d2_rs_core::quote_go_bytes(s.as_bytes());
    let help = value.map(quote).unwrap_or_else(|| "(*string)(nil)".into());
    let fail = |e: String| {
        GraphEditError::Move(if let Some(tag) = tag {
            format!(
                "failed to set {} to {} {}: {e}",
                quote(spelling),
                quote(tag),
                quote(&help)
            )
        } else {
            format!("failed to set {} to {}: {e}", quote(spelling), quote(&help))
        })
    };
    let ast = Shared::new(map);
    let (mut compiler, root) =
        compile_with_options(ast.clone(), options).map_err(GraphEditError::Move)?;
    validate_graphs(&compiler, root).map_err(GraphEditError::Move)?;
    let (selected, base) =
        board(&mut compiler.arena, root, &ast, &options.board_path).ok_or_else(|| {
            fail(format!(
                "board [{}] not found",
                options.board_path.join(" ")
            ))
        })?;
    let base = base.ok_or_else(|| {
        fail(format!(
            "board [{}] cannot be modified through this file",
            options.board_path.join(" ")
        ))
    })?;
    if tag.is_some_and(|t| t.chars().any(char::is_whitespace)) {
        return Err(fail("spaces are not allowed in blockstring tags".into()));
    }
    let mut key = parse_map_key(spelling.as_bytes()).map_err(|e| fail(e.to_string()))?;
    if key.edges.as_ref().is_some_and(|e| e.len() > 1) {
        return Err(fail("can only set one edge at a time".into()));
    }
    key.value = value
        .map(|v| {
            ValueBox::from_child(if let Some(tag) = tag {
                Some(Child::Block(Shared::new(ast::BlockString {
                    tag: tag.into(),
                    value: v.into(),
                    ..Default::default()
                })))
            } else {
                ast::raw_string_box(v, false).unbox()
            })
        })
        .unwrap_or_default();
    // For nested boards, _set compares BaseAST to the projected board AST.
    let projected = if options.board_path.is_empty() {
        ast.clone()
    } else {
        match compiler.arena.ast(selected).expect("board AST") {
            Child::Map(m) => m,
            _ => unreachable!(),
        }
    };
    apply(&projected, &compiler, selected, &base, key).map_err(&fail)?;
    let (source, graph) = recompile(ast.read().clone(), options).map_err(fail)?;
    Ok(SetObjectResult { source, graph })
}

/// d2oracle.Create / generateUniqueKey (edit.go:31–77,2650–2731).
pub(in crate::graph_edit) fn create(
    map: Map,
    spelling: &str,
    options: &MoveOptions,
) -> Result<CreateObjectResult, GraphEditError> {
    let fail = |message: String| {
        GraphEditError::Move(format!(
            "failed to create {}: {message}",
            d2_rs_core::quote_go_bytes(spelling.as_bytes())
        ))
    };
    let ast = Shared::new(map);
    let (mut compiler, root) =
        compile_with_options(ast.clone(), options).map_err(GraphEditError::Move)?;
    validate_graphs(&compiler, root).map_err(GraphEditError::Move)?;
    let (selected, base) =
        board(&mut compiler.arena, root, &ast, &options.board_path).ok_or_else(|| {
            fail(format!(
                "board [{}] not found",
                options.board_path.join(" ")
            ))
        })?;
    let base = base.ok_or_else(|| {
        fail(format!(
            "board [{}] cannot be modified through this file",
            options.board_path.join(" ")
        ))
    })?;
    let mut key = parse_map_key(spelling.as_bytes()).map_err(|error| fail(error.to_string()))?;
    let edge_count = key.edges.as_ref().map_or(0, SharedSlice::len);
    if edge_count > 1 {
        return Err(fail("cannot generate unique key for edge chain".into()));
    }
    let new_key = if edge_count == 1 {
        let snapshot = edges::Snapshot::new(&compiler, selected).map_err(&fail)?;
        if key
            .edge_index
            .as_ref()
            .is_none_or(|i| i.read().int.is_none())
        {
            key.edge_index = Some(Shared::new(EdgeIndex {
                int: Some(Shared::new(0)),
                ..Default::default()
            }));
        }
        edges::trim_common(&mut key);
        if let Some(scope) = snapshot.scope(&key) {
            while snapshot.find(scope, &key).is_some() {
                let index = key.edge_index.as_ref().unwrap().read().int.clone().unwrap();
                *index.write() += 1;
            }
        }
        let generated = format_key(&key);
        // Create passes the original (usually unindexed) edge to _set.
        key = parse_map_key(spelling.as_bytes()).map_err(|error| fail(error.to_string()))?;
        generated
    } else {
        let path = names(&key.key);
        if path.is_empty() {
            return Err(fail("empty object key".into()));
        }
        let selected_path = unique_path(&compiler.arena, selected, &path, spelling);
        let generated = format_path(&selected_path);
        key = parse_map_key(generated.as_bytes()).map_err(|error| fail(error.to_string()))?;
        generated
    };
    let projected = if options.board_path.is_empty() {
        ast.clone()
    } else {
        match compiler.arena.ast(selected).expect("board AST") {
            Child::Map(map) => map,
            _ => unreachable!(),
        }
    };
    apply(&projected, &compiler, selected, &base, key).map_err(&fail)?;
    let (source, graph) = recompile(ast.read().clone(), options).map_err(fail)?;
    Ok(CreateObjectResult {
        source,
        graph,
        key: new_key,
    })
}
