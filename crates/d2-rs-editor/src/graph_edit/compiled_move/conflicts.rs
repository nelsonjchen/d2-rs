// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Source-reference ordered renameConflictsToParent prepass.
use super::*;
use std::collections::HashSet;

pub(super) fn rename(
    map: &mut Map,
    old: &[String],
    options: &MoveOptions,
) -> Result<bool, GraphEditError> {
    let (compiler, root) =
        compile_with_options(Shared::new(map.clone()), options).map_err(GraphEditError::Move)?;
    let Some(object) = field(&compiler.arena, root, old) else {
        return Ok(false);
    };
    if let Some(composite) = compiler.arena.map(object) {
        let shape = compiler
            .arena
            .get_field(composite, &[ast::raw_string_box("shape", true)])
            .and_then(|id| compiler.arena.primary(id))
            .and_then(|id| match compiler.arena.node(id) {
                ir::Node::Scalar(s) => Some(s.value.scalar_string()),
                _ => None,
            });
        if shape
            .as_deref()
            .is_some_and(|s| s == "sql_table" || s == "class")
        {
            return Ok(false);
        }
    }
    let ignore_object = compiler
        .arena
        .map(object)
        .and_then(|id| field(&compiler.arena, id, &[old.last().unwrap().clone()]))
        .is_none();
    let refs = references(&compiler.arena, object);
    let mut generated = Vec::new();
    let mut dedup = HashSet::new();
    let mut changed = false;
    for r in refs {
        let key = r.path.read().string_ida().unwrap_or_default();
        let mut absolute = Vec::<Vec<String>>::new();
        if r.index + 1 == key.len() {
            let Some(children) = r.key.read().value.map.clone() else {
                continue;
            };
            let mut paths = Vec::new();
            for node in map_nodes(&children.read()) {
                let Some(key) = node.map_key else { continue };
                let key = key.read();
                if let Some(path) = &key.key {
                    if path
                        .read()
                        .string_ida()
                        .and_then(|p| p.first().cloned())
                        .is_some_and(|s| is_reserved_keyword(&s))
                    {
                        continue;
                    }
                    paths.push(path.clone());
                }
                for edge in key
                    .edges
                    .iter()
                    .flat_map(SharedSlice::iter)
                    .filter_map(|slot| slot.read().clone())
                {
                    let edge = edge.read();
                    paths.extend(edge.src.iter().chain(edge.dst.iter()).cloned());
                }
            }
            for path in paths {
                if let Some(first) = path.read().string_ida().and_then(|p| p.first().cloned()) {
                    absolute.push(
                        r.scope_path
                            .iter()
                            .chain(&key)
                            .cloned()
                            .chain([first])
                            .collect(),
                    );
                }
            }
        } else if key.last().is_some_and(|s| !is_reserved_keyword(s)) {
            absolute.push(
                r.scope_path
                    .iter()
                    .chain(&key[..r.index + 2])
                    .cloned()
                    .collect(),
            );
        }
        let (current, current_root) = compile_with_options(Shared::new(map.clone()), options)
            .map_err(GraphEditError::Move)?;
        let ignored = if ignore_object {
            field(&current.arena, current_root, old)
        } else {
            None
        };
        let hoisted = |abs: &[String]| {
            r.scope_path
                .iter()
                .chain(&key[..r.index])
                .cloned()
                .chain(abs.last().cloned())
                .collect::<Vec<_>>()
        };
        let mut renames = Vec::<(Vec<String>, Vec<String>)>::new();
        for (index, abs) in absolute.iter().enumerate() {
            let spelling = format_path(abs);
            if !dedup.insert(spelling) {
                continue;
            }
            if abs.len() > 1 && abs[abs.len() - 1] == abs[abs.len() - 2] {
                continue;
            }
            let target = hoisted(abs);
            let mut included = generated.clone();
            included.extend(
                absolute
                    .iter()
                    .enumerate()
                    .filter(|(i, other)| *i != index && !dedup.contains(&format_path(other)))
                    .map(|(_, other)| format_path(&hoisted(other))),
            );
            let unique = unique_path_with_policy(
                &current.arena,
                current_root,
                &target,
                &format_path(&target),
                ignored,
                &included,
            );
            generated.push(format_path(&unique));
            let mut renamed = abs.clone();
            *renamed.last_mut().unwrap() = unique.last().unwrap().clone();
            if *abs != renamed {
                renames.push((abs.clone(), renamed.clone()));
            }
            dedup.insert(format_path(&renamed));
        }
        // Independent names commute. A destination that is itself being
        // renamed must be released first, as in the source conflict ordering.
        while !renames.is_empty() {
            let index = renames
                .iter()
                .position(|(_, to)| !renames.iter().any(|(from, _)| from == to))
                .ok_or_else(|| GraphEditError::Move("cyclic hoisted-name conflict".into()))?;
            let (from, to) = renames.remove(index);
            changed = true;
            let result = prepare(map, &format_path(&from), &format_path(&to), false, options)?;
            let parsed = parse(&options.path, result.source.as_bytes(), None);
            if let Some(error) = parsed.error {
                return Err(GraphEditError::SourceParse(error.to_string()));
            }
            *map = parsed.map;
        }
    }
    Ok(changed)
}
