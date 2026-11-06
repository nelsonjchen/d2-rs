// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Compiler-reference translation of pinned d2oracle/edit.go::move.
//!
//! Same-scope moves rename compiled references in place. Cross-scope moves
//! whose references are all terminal edge endpoints need no landing map.
//! Declaration splitting, landing maps and hoisting are in cross_scope;
//! generated child-name conflicts follow the source-reference prepass.

use super::*;
use d2_rs_core::{ast, ir};
use d2_rs_language::compiler;

mod conflicts;
mod cross_scope;
mod delete;
mod edges;
mod reconnect;
mod set;

pub(super) use delete::delete;
pub(super) use reconnect::reconnect;
pub(super) use set::{create, set};

#[derive(Clone)]
struct Reference {
    path: Shared<KeyPath>,
    index: usize,
    key: Shared<Key>,
    edge: Option<Shared<ast::Edge>>,
    scope: Shared<Map>,
    scope_path: Vec<String>,
    primary: bool,
    scope_ast: Option<Shared<Map>>,
}

fn field(arena: &ir::Arena, mut map: ir::NodeId, path: &[String]) -> Option<ir::NodeId> {
    // Graph.HasChild uses lower-cased object IDs, not IR's Unicode simple
    // folding or its separate property namespace. A terminal reserved name
    // other than style resolves to the current object, even if it was quoted.
    for (index, part) in path.iter().enumerate() {
        if index + 1 == path.len() && part != "style" && is_reserved_keyword(part) {
            return arena.parent(map);
        }
        let ir::Node::Map(parent) = arena.node(map) else {
            return None;
        };
        let id = parent
            .fields
            .iter()
            .flatten()
            .find(|id| {
                let ir::Node::Field(candidate) = arena.node(**id) else {
                    return false;
                };
                let name = candidate.name.scalar_string();
                !(candidate.name.unquoted_string.is_some()
                    && is_reserved_keyword(&name.to_lowercase()))
                    && ir::go_lowercase(name.as_bytes()) == ir::go_lowercase(part.as_bytes())
            })
            .copied()?;
        if index + 1 == path.len() {
            return Some(id);
        }
        map = arena.map(id)?;
    }
    arena.parent(map)
}

fn references(arena: &ir::Arena, object: ir::NodeId) -> Vec<Reference> {
    let ir::Node::Field(object) = arena.node(object) else {
        return Vec::new();
    };
    object
        .references
        .iter()
        .flatten()
        .filter_map(|reference| {
            let reference = reference.read();
            let context = reference.context.as_ref()?.read();
            let scope_path = context
                .scope_map
                .and_then(|scope| d2_rs_language::ir::board_ida(arena, scope))
                .unwrap_or_default()
                .into_iter()
                .map(|part| ast::ScalarBox::from_child(part).scalar_string())
                .collect();
            Some(Reference {
                path: reference.key_path.clone()?,
                // Snapshot before replacing any StringBox: IR computes this by
                // original string identity; graph.Reference stores the index.
                index: reference.key_path_index(),
                primary: reference.primary(),
                scope_ast: context.scope_ast.clone(),
                key: context.key.clone()?,
                edge: context.edge.clone(),
                scope: context.scope.clone()?,
                scope_path,
            })
        })
        .collect()
}

fn unique_path(
    arena: &ir::Arena,
    root: ir::NodeId,
    requested: &[String],
    spelling: &str,
) -> Vec<String> {
    unique_path_with_policy(arena, root, requested, spelling, None, &[])
}

fn unique_path_with_policy(
    arena: &ir::Arena,
    root: ir::NodeId,
    requested: &[String],
    spelling: &str,
    ignored: Option<ir::NodeId>,
    included: &[String],
) -> Vec<String> {
    let mut stem = requested.to_vec();
    // generateUniqueKey removes a numeric suffix from the supplied spelling,
    // then starts again at the unsuffixed key. Quoted numbers stay quoted.
    if field(arena, root, requested).is_some_and(|id| Some(id) != ignored)
        && let Some((prefix, suffix)) = spelling.rsplit_once(' ')
        && suffix.parse::<i64>().is_ok()
        && let Ok(path) = checked_object_path(prefix)
    {
        stem = path;
    }
    let mut selected = stem.clone();
    let last = selected.len() - 1;
    for suffix in 2u64.. {
        if field(arena, root, &selected).is_none_or(|id| Some(id) == ignored)
            && !included.contains(&format_path(&selected))
        {
            return selected;
        }
        selected[last] = format!("{} {suffix}", stem[last]);
    }
    unreachable!("unique object names have an unbounded suffix space")
}

fn compile_with_options(
    map: Shared<Map>,
    options: &MoveOptions,
) -> Result<(compiler::Compiler, ir::NodeId), String> {
    let files = options.files.clone();
    let opener: compiler::ImportOpener = Box::new(move |path| {
        files
            .get(path)
            .cloned()
            .map(|text| {
                Box::new(std::io::Cursor::new(text)) as Box<dyn d2_rs_language::stream::ByteReader>
            })
            .ok_or_else(|| format!("open {path}: no such file or directory"))
    });
    let (compiler, root) = compiler::compile_ir_with_options(
        map,
        compiler::CompileOptions {
            import_opener: Some(opener),
            utf16_positions: false,
        },
    )
    .map_err(|e| e.to_string())?;
    if !compiler.errors.is_empty() {
        return Err(compiler
            .errors
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n"));
    }
    Ok((compiler, root))
}

/// d2oracle/edit.go:1699–1748: Rename constructs a same-parent destination
/// and calls the shared move implementation with includeDescendants=false.
pub(super) fn rename(
    map: &mut Map,
    spelling: &str,
    new_name: &str,
    options: &MoveOptions,
) -> Result<RenameObjectResult, GraphEditError> {
    let fail = |message: String| {
        GraphEditError::Move(format!(
            "failed to rename {} to {}: {message}",
            d2_rs_core::quote_go_bytes(spelling.as_bytes()),
            d2_rs_core::quote_go_bytes(new_name.as_bytes()),
        ))
    };
    let root_ast = Shared::new(map.clone());
    let (mut compiler, root) =
        compile_with_options(root_ast.clone(), options).map_err(GraphEditError::Move)?;
    validate_graphs(&compiler, root).map_err(GraphEditError::Move)?;
    let mut key = parse_map_key(spelling.as_bytes()).map_err(|e| fail(e.to_string()))?;
    let (board_root, _) = board(&mut compiler.arena, root, &root_ast, &options.board_path)
        .ok_or_else(|| {
            fail(format!(
                "board [{}] not found",
                options.board_path.join(" ")
            ))
        })?;
    let mut selected_name = new_name.to_owned();
    if key.edges.as_ref().is_some_and(|edges| !edges.is_empty()) && key.edge_key.is_none() {
        let mut replacement =
            parse_map_key(new_name.as_bytes()).map_err(|e| fail(e.to_string()))?;
        replacement.key = key.key;
        key = replacement;
    } else {
        if is_reserved_keyword(new_name) {
            return Err(fail(format!(
                "cannot rename to reserved keyword: {}",
                d2_rs_core::quote_go_bytes(new_name.as_bytes())
            )));
        }
        if let Some(path) = &key.key {
            let parts = path.read().string_ida().unwrap_or_default();
            let object = field(&compiler.arena, board_root, &parts)
                .ok_or_else(|| fail("key does not exist".into()))?;
            // The source first attempts generateUniqueKey on the supplied
            // name in the board root, ignoring the object being renamed.
            // Invalid key syntax is allowed here: RawString quotes it below.
            if let Ok(requested) = checked_object_path(new_name) {
                let selected = unique_path_with_policy(
                    &compiler.arena,
                    board_root,
                    &requested,
                    new_name,
                    Some(object),
                    &[],
                );
                selected_name = format_path(&selected);
            }
        }
        let path = key
            .key
            .as_ref()
            .ok_or_else(|| fail("key does not exist".into()))?;
        let mut parts = path
            .read()
            .path
            .as_ref()
            .map(|p| p.iter().map(|slot| slot.read().clone()).collect::<Vec<_>>())
            .unwrap_or_default();
        let last = parts
            .last_mut()
            .ok_or_else(|| fail("key does not exist".into()))?;
        *last = Some(Shared::new(ast::raw_string_box(&selected_name, true)));
        path.write().path = Some(parts.into());
    }
    let destination = format_key(&key);
    let moved = prepare(map, spelling, &destination, false, options).map_err(|error| {
        let message = match error {
            GraphEditError::Move(message) => {
                let prefix = format!(
                    "failed to move: {} to {}: ",
                    d2_rs_core::quote_go_bytes(spelling.as_bytes()),
                    d2_rs_core::quote_go_bytes(destination.as_bytes())
                );
                message.strip_prefix(&prefix).unwrap_or(&message).to_owned()
            }
            other => other.to_string(),
        };
        fail(message)
    })?;
    Ok(RenameObjectResult {
        source: moved.source,
        graph: moved.graph,
        key: selected_name,
    })
}

fn validate_graphs(compiler: &compiler::Compiler, root: ir::NodeId) -> Result<(), String> {
    // compileBoard validates every board, even when Move targets another board
    // or returns unchanged. Root-only lowering misses board-local near errors.
    compiler::lower_graph(compiler, root).map_err(|error| error.to_string())?;
    for (_, board) in compiler::board_targets(compiler, root) {
        compiler::lower_graph(compiler, board).map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn board(
    arena: &mut ir::Arena,
    mut root: ir::NodeId,
    ast: &Shared<Map>,
    names: &[String],
) -> Option<(ir::NodeId, Option<Shared<Map>>)> {
    let mut base = Some(ast.clone());
    for name in names {
        let mut selected = None;
        for section in ["layers", "scenarios", "steps"] {
            let Some(map) = arena
                .get_field(root, &[ast::raw_string_box(section, true)])
                .and_then(|id| arena.map(id))
            else {
                continue;
            };
            let ir::Node::Map(map) = arena.node(map) else {
                continue;
            };
            let Some(id) = map.fields.iter().flatten().find(|id|matches!(arena.node(**id),ir::Node::Field(f) if f.name.scalar_string()==*name)).copied() else { continue };
            let map = arena.map(id).unwrap_or_else(|| arena.new_root());
            base = base
                .and_then(|base| compiler::find_board_base_ast(&base, &[section, name.as_str()]));
            selected = Some(map);
            break;
        }
        root = selected?;
    }
    Some((root, base))
}

fn absolute_path(arena: &ir::Arena, object: ir::NodeId) -> Vec<String> {
    d2_rs_language::ir::board_ida(arena, object)
        .unwrap_or_default()
        .into_iter()
        .map(|part| ast::ScalarBox::from_child(part).scalar_string())
        .collect()
}

fn near_references(
    map: &mut Map,
    context: &[String],
    arena: &ir::Arena,
    root: ir::NodeId,
    old: &[String],
    new: &[String],
    split_descendants: bool,
) -> Result<(), GraphEditError> {
    for node in map_nodes(map) {
        let Some(key) = node.map_key else { continue };
        let mut key = key.write();
        let parts = key
            .key
            .as_ref()
            .and_then(|path| path.read().string_ida())
            .unwrap_or_default();
        let full = context.iter().chain(&parts).cloned().collect::<Vec<_>>();
        // updateNear has two source passes. Its flat-object-reference pass
        // also visits constants; the nested-map pass skips placements and
        // constants. Preserve that distinction, including its odd edge cases.
        let placement = full
            .iter()
            .rev()
            .nth(1)
            .is_some_and(|s| s == "label" || s == "icon");
        let flat = parts.len() > 1 && !is_reserved_keyword(&parts[0]);
        if (flat || !placement) && parts.last().is_some_and(|s| s == "near") {
            let value = key.value.scalar_box().scalar_string();
            if (flat || !ast::keywords::NEAR_CONSTANTS.contains(&value.as_str()))
                && let Ok(parsed) = parse_map_key(value.as_bytes())
                && let Some(path) = parsed.key.and_then(|p| p.read().string_ida())
                && let Some(target) = field(arena, root, &path)
                && let path = absolute_path(arena, target)
                // MoveIDDeltas is keyed by the canonical, formatted AbsID.
                // updateNear looks up the original scalar literally.
                && format_path(&path) == value
                && path.len() >= old.len()
                && path.starts_with(old)
            {
                let prefix = if split_descendants && path.len() > old.len() {
                    &old[..old.len() - 1]
                } else {
                    new
                };
                let new_path = prefix
                    .iter()
                    .chain(&path[old.len()..])
                    .cloned()
                    .collect::<Vec<_>>();
                key.value = ValueBox::from_child(
                    ast::raw_string_box(format_path(&new_path), false).unbox(),
                );
            }
        }
        if let Some(child) = &key.value.map {
            near_references(
                &mut child.write(),
                &full,
                arena,
                root,
                old,
                new,
                split_descendants,
            )?;
        }
    }
    Ok(())
}

fn ensure_parent(
    arena: &ir::Arena,
    root: ir::NodeId,
    refs: &[Reference],
    reference: &Reference,
) -> Result<(), GraphEditError> {
    let mut path = reference.path.read().string_ida().unwrap_or_default();
    if path.iter().filter(|part| part.as_str() != "_").count() <= 1 {
        return Ok(());
    }
    path.pop();
    // ensureNode prepends the owner of a scoped edge key before resolving it.
    let owner = reference
        .key
        .read()
        .key
        .as_ref()
        .and_then(|p| p.read().string_ida())
        .unwrap_or_default();
    let path = owner.into_iter().chain(path).collect::<Vec<_>>();
    let absolute = resolve_scoped_path(&reference.scope_path, &path)?;
    if let Some(parent) = field(arena, root, &absolute) {
        let ir::Node::Field(parent) = arena.node(parent) else {
            unreachable!()
        };
        let persists = parent.references.iter().flatten().any(|r| {
            let r = r.read();
            let Some(context) = &r.context else {
                return false;
            };
            let context = context.read();
            let Some(edge) = &context.edge else {
                return true;
            };
            !refs
                .iter()
                .filter_map(|r| r.edge.as_ref())
                .any(|excluded| excluded.ptr_eq(edge))
        });
        if persists {
            return Ok(());
        }
    }
    let detached = Key {
        key: Some(Shared::new(KeyPath::from_strings(path))),
        ..Default::default()
    };
    if !map_nodes(&reference.scope.read()).iter().any(|node| {
        node.map_key
            .as_ref()
            .is_some_and(|key| key.read().d2_oracle_equals(&detached))
    }) {
        reference.scope.write().insert_after(
            Some(&Child::Key(reference.key.clone())),
            Some(Child::Key(Shared::new(detached))),
        );
    }
    Ok(())
}

/// Return the selected unique path and recompiled source for an object move.
pub(super) fn prepare(
    map: &mut Map,
    old_spelling: &str,
    requested_spelling: &str,
    include_descendants: bool,
    options: &MoveOptions,
) -> Result<MoveObjectResult, GraphEditError> {
    let fail = |message: String| {
        GraphEditError::Move(format!(
            "failed to move: {} to {}: {message}",
            d2_rs_core::quote_go_bytes(old_spelling.as_bytes()),
            d2_rs_core::quote_go_bytes(requested_spelling.as_bytes()),
        ))
    };
    let mut root_ast = Shared::new(map.clone());
    let (mut compiler, mut root) =
        compile_with_options(root_ast.clone(), options).map_err(GraphEditError::Move)?;
    validate_graphs(&compiler, root).map_err(GraphEditError::Move)?;
    if old_spelling == requested_spelling {
        let (source, graph) =
            compiled_result(&compiler, root, root_ast).map_err(GraphEditError::Move)?;
        return Ok(MoveObjectResult {
            source,
            graph,
            key: checked_object_path(requested_spelling)
                .map(|path| format_path(&path))
                .unwrap_or_else(|_| requested_spelling.to_owned()),
        });
    }
    let (board_root, base_ast) = board(&mut compiler.arena, root, &root_ast, &options.board_path)
        .ok_or_else(|| {
        fail(format!(
            "board [{}] not found",
            options.board_path.join(" ")
        ))
    })?;
    let base_ast = base_ast.ok_or_else(|| {
        fail(format!(
            "board [{}] cannot be modified through this file",
            options.board_path.join(" ")
        ))
    })?;
    let (old_key, requested_key, destination_is_edge) = match edges::prepare(
        &compiler,
        root,
        board_root,
        root_ast.clone(),
        old_spelling,
        requested_spelling,
        options,
    )
    .map_err(&fail)?
    {
        edges::PreparedMove::Edge(result) => return Ok(result),
        edges::PreparedMove::Objects {
            old,
            new,
            destination_is_edge,
        } => (old, new, destination_is_edge),
    };
    // Object branches consume the map key's object prefix, including scoped
    // edge-property keys. Root edge/object cross-kind requests make the Go
    // source dereference a nil path; reject those explicitly instead.
    let parse_key = |key: &Key, spelling: &str| {
        if key.edges.is_some() {
            return key
                .key
                .as_ref()
                .and_then(|path| path.read().string_ida())
                .filter(|path| !path.is_empty())
                .ok_or_else(|| {
                    fail("moving between object and root edge keys is not supported".into())
                });
        }
        checked_object_path(spelling).map_err(|error| match error {
            GraphEditError::InvalidKey(message) => fail(message),
            other => other,
        })
    };
    let requested = &parse_key(&requested_key, requested_spelling)?;
    let old = &parse_key(&old_key, old_spelling)?;
    if requested.len() > old.len() && requested.starts_with(old) {
        return Err(GraphEditError::UnsupportedMove(
            "an object cannot be moved into its own descendant".into(),
        ));
    }
    root = board_root;
    let selected = if destination_is_edge {
        requested.clone()
    } else {
        unique_path(&compiler.arena, root, requested, requested_spelling)
    };
    let cross_scope = old[..old.len() - 1] != selected[..selected.len() - 1];
    if cross_scope && !include_descendants && !options.board_path.is_empty() {
        let Child::Map(projected) = compiler.arena.ast(root).expect("compiled board AST") else {
            unreachable!()
        };
        let mut projected = projected.read().clone();
        let local_options = MoveOptions {
            board_path: Vec::new(),
            ..options.clone()
        };
        // A child rename recompiles the projected board. Its new references
        // no longer belong to BaseAST, so GetWriteableRefs rejects the move.
        if conflicts::rename(&mut projected, old, &local_options)? {
            return Err(fail(
                "operation would modify AST outside of given scope".into(),
            ));
        }
    }
    if cross_scope && !include_descendants && options.board_path.is_empty() {
        let mut renamed = root_ast.read().clone();
        conflicts::rename(&mut renamed, old, options)?;
        root_ast = Shared::new(renamed);
        (compiler, root) =
            compile_with_options(root_ast.clone(), options).map_err(GraphEditError::Move)?;
    }
    let arena = &compiler.arena;
    let object = field(arena, root, old)
        .ok_or_else(|| fail("key referenced by from does not exist".into()))?;
    let refs = references(arena, object);
    if !options.board_path.is_empty()
        && refs.iter().any(|r| {
            !r.scope_ast
                .as_ref()
                .is_some_and(|scope| scope.ptr_eq(&base_ast))
                || r.path.read().range.path != base_ast.read().range.path
                || r.key
                    .read()
                    .edges
                    .as_ref()
                    .is_some_and(|edges| !edges.is_empty())
        })
    {
        return Err(fail(
            "operation would modify AST outside of given scope".into(),
        ));
    }
    if cross_scope
        && selected.len() > 1
        && field(arena, root, &selected[..selected.len() - 1]).is_none()
    {
        return Err(fail("key referenced by to parent does not exist".into()));
    }
    let canonical_old = absolute_path(arena, object);
    let mut canonical_new = if selected.len() > 1 {
        field(arena, root, &selected[..selected.len() - 1])
            .map(|parent| absolute_path(arena, parent))
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    canonical_new.push(selected.last().expect("destination name").clone());
    for reference in refs.iter().filter(|_| !cross_scope) {
        if old.last() != selected.last() {
            let slot = reference
                .path
                .read()
                .path
                .as_ref()
                .and_then(|p| p.get(reference.index))
                .expect("compiled reference component");
            *slot.write() = Some(Shared::new(ast::raw_string_box(
                selected.last().expect("destination name").as_str(),
                true,
            )));
        }
    }
    if cross_scope {
        // pathFromScopeObj retains the parsed destination StringBoxes. In
        // particular, legal unquoted parentheses must not be re-quoted by
        // MakeKeyPath/RawString when the leaf name itself did not change.
        let selected_key = if &selected == requested {
            requested_key
                .key
                .as_ref()
                .expect("object path")
                .read()
                .clone()
        } else {
            KeyPath::from_strings(selected.iter().cloned())
        };
        let parts = selected_key
            .path
            .iter()
            .flat_map(SharedSlice::iter)
            .map(|slot| slot.read().clone())
            .collect();
        let working_ast = if options.board_path.is_empty() {
            root_ast.clone()
        } else {
            match arena.ast(root).expect("compiled board AST") {
                Child::Map(map) => map,
                _ => unreachable!(),
            }
        };
        cross_scope::execute(
            arena,
            root,
            working_ast,
            object,
            old,
            &selected,
            parts,
            include_descendants,
        )?;
    }
    *map = root_ast.read().clone();
    let mut near_ast = if options.board_path.is_empty() {
        map.clone()
    } else {
        base_ast.read().clone()
    };
    near_references(
        &mut near_ast,
        &[],
        arena,
        root,
        &canonical_old,
        &canonical_new,
        cross_scope && !include_descendants,
    )?;
    let (source, graph) = recompile(map.clone(), options).map_err(fail)?;
    Ok(MoveObjectResult {
        key: format_path(&selected),
        source,
        graph,
    })
}

fn recompile(map: Map, options: &MoveOptions) -> Result<(String, serde_json::Value), String> {
    let output = finish_map(map).map_err(|error| error.to_string())?;
    let fail = |error| format!("failed to recompile:\n{output}\n{error}");
    let parsed = parse(&options.path, output.as_bytes(), None);
    if let Some(error) = parsed.error {
        return Err(fail(error.to_string()));
    }
    let ast = Shared::new(parsed.map);
    let (compiler, root) = compile_with_options(ast.clone(), options).map_err(&fail)?;
    compiled_result(&compiler, root, ast).map_err(fail)
}

// d2oracle's public result is formatted before graph JSON serialization. The
// formatter fills raw quoted-string text in the same AST used by references.
fn compiled_result(
    compiler: &compiler::Compiler,
    root: ir::NodeId,
    ast: Shared<Map>,
) -> Result<(String, serde_json::Value), String> {
    let source =
        String::from_utf8(format::format(&Child::Map(ast.clone()))).expect("formatter emits UTF-8");
    let graph =
        compiler::source_graph_json(compiler, root, ast).map_err(|error| error.to_string())?;
    Ok((source, graph))
}
