// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! d2oracle/edit.go:3337–3478, UpdateImport and its path helpers.
use d2_rs_core::ast::{Child, Import, Map, Shared, raw_string_box};
use d2_rs_language::{format, parse};

/// A parse error with the original UpdateImport operation context.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("{0}")]
pub struct UpdateImportError(String);

/// Rename or remove matching import paths throughout a document's maps.
///
/// A trailing slash on `old_path` selects a directory prefix. Directory suffixes
/// are preserved only when `new_path` also ends in a slash. This syntax-only
/// operation does not open imports or compile the document.
pub fn update_import(
    source: &str,
    old_path: &str,
    new_path: Option<&str>,
) -> Result<String, UpdateImportError> {
    let parsed = parse("", source.as_bytes(), None);
    if let Some(error) = parsed.error {
        let quote = |value: &str| d2_rs_core::quote_go_bytes(value.as_bytes());
        let context = match new_path {
            None => format!("failed to remove import {}", quote(old_path)),
            Some(new) => format!(
                "failed to update import from {} to {}",
                quote(old_path),
                quote(new)
            ),
        };
        return Err(UpdateImportError(format!("{context}: {error}")));
    }
    let mut map = parsed.map;
    update_map(&mut map, old_path, new_path);
    Ok(
        String::from_utf8(format::format(&Child::Map(Shared::new(map))))
            .expect("D2 formatter emits UTF-8"),
    )
}

fn matches_path(path: &str, old: &str) -> bool {
    if old.ends_with('/') {
        path.starts_with(old)
    } else {
        path == old
    }
}

fn replace_path(import: &mut Import, path: &str, old: &str, new: &str) {
    let new = if old.ends_with('/') && new.ends_with('/') {
        format!("{new}{}", &path[old.len()..])
    } else {
        new.to_owned()
    };
    let split = new
        .char_indices()
        .find(|(_, c)| *c != '.' && *c != '/')
        .map_or(new.len(), |(i, _)| i);
    let (pre, part) = new.split_at(split);
    import.pre = pre.into();
    if !part.is_empty() {
        let value = Some(Shared::new(raw_string_box(part, true)));
        if let Some(slot) = import.path.as_ref().and_then(|path| path.get(0)) {
            *slot.write() = value;
        } else {
            import.path = Some(vec![value].into());
        }
    } else if import.path.as_ref().is_none_or(|path| path.is_empty()) {
        import.path = Some(vec![Some(Shared::new(raw_string_box("", true)))].into());
    }
    // With an all-dot/slash path upstream retains an existing first component.
    // Later components (selective import keys) are always preserved.
}

fn update_map(map: &mut Map, old: &str, new: Option<&str>) {
    let Some(nodes) = &map.nodes else { return };
    let mut retained = Vec::with_capacity(nodes.len());
    for slot in nodes.iter() {
        let mut node = slot.read().clone();
        if let Some(import) = node.import.clone() {
            let path = import.read().path_with_pre();
            if matches_path(&path, old) {
                match new {
                    Some(new) => replace_path(&mut import.write(), &path, old, new),
                    None if import.read().spread => continue,
                    None => node.import = None,
                }
                retained.push(node);
                continue;
            }
        }
        if let Some(key) = &node.map_key {
            let mut key = key.write();
            if let Some(import) = key.value.import.clone() {
                let path = import.read().path_with_pre();
                if matches_path(&path, old) {
                    match new {
                        Some(new) => replace_path(&mut import.write(), &path, old, new),
                        None if import.read().spread && key.value.map.is_none() => continue,
                        None => key.value.import = None,
                    }
                }
            }
            // ScalarBox has no Import member in the pinned AST, so the Go
            // Primary -> ValueBox -> Import branch cannot contain an import.
            if let Some(nested) = &key.value.map {
                update_map(&mut nested.write(), old, new);
            }
        }
        retained.push(node);
    }
    map.nodes = Some(retained.into());
}
