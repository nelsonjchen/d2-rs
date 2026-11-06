// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Pinned d2oracle/edit.go:124–313: ReconnectEdge and pathFromScopeObj.
use super::delete::{edge_list, edge_refs, ensure_node};
use super::*;

fn endpoint(spelling: &str, scope: &[String]) -> Result<Shared<KeyPath>, GraphEditError> {
    let key = parse_map_key(spelling.as_bytes())
        .map_err(|error| GraphEditError::Reconnect(error.to_string()))?;
    let path = key
        .key
        .ok_or_else(|| GraphEditError::Reconnect("empty endpoint".into()))?;
    let names = path.read().string_ida().unwrap_or_default();
    let common = scope.iter().zip(&names).take_while(|(a, b)| a == b).count();
    let mut parts = (common..scope.len())
        .map(|_| Some(Shared::new(ast::raw_string_box("_", true))))
        .collect::<Vec<_>>();
    parts.extend(
        path.read()
            .path
            .iter()
            .flat_map(SharedSlice::iter)
            .skip(common)
            .map(|slot| slot.read().clone()),
    );
    path.write().path = Some(parts.into());
    Ok(path)
}

pub(in crate::graph_edit) fn reconnect(
    map: Map,
    spelling: &str,
    mut src: Option<&str>,
    mut dst: Option<&str>,
    options: &MoveOptions,
) -> Result<ReconnectObjectResult, GraphEditError> {
    let fail = GraphEditError::Reconnect;
    let ast = Shared::new(map);
    let (mut compiler, root) = compile_with_options(ast.clone(), options).map_err(fail)?;
    validate_graphs(&compiler, root).map_err(fail)?;
    let mut key = parse_map_key(spelling.as_bytes()).map_err(|error| fail(error.to_string()))?;
    if key.edges.as_ref().is_none_or(|edges| edges.is_empty()) {
        return Err(fail("edgeKey must be an edge".into()));
    }
    if key.edge_index.is_none() {
        return Err(fail("edgeKey must refer to an existing edge".into()));
    }
    edges::trim_common(&mut key);
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
    let snapshot = edges::Snapshot::new(&compiler, selected).map_err(fail)?;
    let edge_id = snapshot
        .scope(&key)
        .and_then(|scope| snapshot.find(scope, &key))
        .ok_or_else(|| fail("edge not found".into()))?;
    let edge = snapshot
        .graph
        .edges()
        .find(|(id, _)| *id == edge_id)
        .expect("resolved edge")
        .1;
    if src
        == Some(
            snapshot
                .graph
                .node(edge.source)
                .unwrap()
                .external_id
                .as_str(),
        )
    {
        src = None;
    }
    if dst
        == Some(
            snapshot
                .graph
                .node(edge.target)
                .unwrap()
                .external_id
                .as_str(),
        )
    {
        dst = None;
    }
    if src.is_none() && dst.is_none() {
        let (source, graph) = compiled_result(&compiler, root, ast).map_err(fail)?;
        return Ok(ReconnectObjectResult {
            source,
            graph,
            changed: false,
        });
    }
    for (spelling, message) in [(src, "newSrc not found"), (dst, "newDst not found")] {
        if let Some(spelling) = spelling {
            let key =
                parse_map_key(spelling.as_bytes()).map_err(|error| fail(error.to_string()))?;
            let names = key
                .key
                .as_ref()
                .and_then(|key| key.read().string_ida())
                .unwrap_or_default();
            if field(&compiler.arena, selected, &names).is_none() {
                return Err(fail(message.into()));
            }
        }
    }
    let mut refs = edge_refs(&compiler.arena, snapshot.origins[edge_id.0 as usize]);
    if !base.ptr_eq(&ast) {
        // GetWriteableEdgeRefs checks ScopeAST identity only (get.go:243–250).
        refs.retain(|r| r.scope_ast.as_ref().is_some_and(|s| s.ptr_eq(&base)));
        if refs.is_empty() || !refs[0].scope_ast.as_ref().is_some_and(|s| s.ptr_eq(&base)) {
            return Err(GraphEditError::OutsideScope);
        }
    }
    let first = refs
        .first()
        .ok_or_else(|| fail("edge has no source reference".into()))?;
    if edge.source != edge.target && (src.is_none() || dst.is_none()) {
        let excluded = refs.iter().map(|r| r.edge.clone()).collect::<Vec<_>>();
        for (changed, path, before) in [
            (src.is_some(), first.edge.read().src.clone(), true),
            (dst.is_some(), first.edge.read().dst.clone(), false),
        ] {
            if changed {
                ensure_node(
                    &compiler.arena,
                    selected,
                    &excluded,
                    &first.scope_path,
                    &first.scope,
                    &first.key,
                    &path.expect("edge endpoint"),
                    before,
                );
            }
        }
    }
    for reference in refs {
        let chain = edge_list(&reference.key);
        let index = chain
            .iter()
            .position(|e| e.ptr_eq(&reference.edge))
            .expect("edge reference");
        if chain.len() > 1
            && reference.key.read().edge_index.is_none()
            && !(index == 0 && dst.is_none())
            && !(index + 1 == chain.len() && src.is_none())
        {
            let mut isolated = reference.key.read().clone();
            isolated.edges = Some(vec![Some(reference.edge.clone())].into());
            reference.scope.write().insert_after(
                Some(&Child::Key(reference.key.clone())),
                Some(Child::Key(Shared::new(isolated))),
            );
            if index + 1 < chain.len() {
                let mut tail = reference.key.read().clone();
                tail.edges = Some(
                    chain[index + 1..]
                        .iter()
                        .cloned()
                        .map(Some)
                        .collect::<Vec<_>>()
                        .into(),
                );
                reference.scope.write().insert_after(
                    Some(&Child::Key(reference.key.clone())),
                    Some(Child::Key(Shared::new(tail))),
                );
            }
            reference.key.write().edges = Some(
                chain[..index]
                    .iter()
                    .cloned()
                    .map(Some)
                    .collect::<Vec<_>>()
                    .into(),
            );
        }
        if let Some(src) = src {
            reference.edge.write().src = Some(endpoint(src, &reference.scope_path)?);
        }
        if let Some(dst) = dst {
            reference.edge.write().dst = Some(endpoint(dst, &reference.scope_path)?);
        }
    }
    let (source, graph) = recompile(ast.read().clone(), options).map_err(fail)?;
    Ok(ReconnectObjectResult {
        source,
        graph,
        changed: true,
    })
}
