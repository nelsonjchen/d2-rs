// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! AST-backed graph editing primitives.
//!
//! These source operations parse the requested keys, mutate the AST, format
//! the complete document, and validate the result. Root-field edits and
//! single-edge reconnects are syntax-level operations. Object edits resolve
//! compiler references for renames, declaration transplants, child hoisting,
//! edge rewrites and board scope checks, with an explicit import provider.

use d2_rs_core::ast::{
    Child, EdgeIndex, Key, KeyPath, Map, MapNodeBox, Shared, SharedSlice, ValueBox,
    keywords::is_reserved_keyword,
};
use d2_rs_language::{format, parse, parse_map_key, parse_value};
use serde::Serialize;
use thiserror::Error;

mod compiled_move;

/// Result of creating a root-level field.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CreateRootResult {
    /// The formatted, validated document.
    pub source: String,
    /// The key selected after applying the deterministic uniqueness suffix.
    pub key: String,
}

/// Result of compiler-resolved Create, including nested keys, boards and edges.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CreateObjectResult {
    pub source: String,
    pub key: String,
    pub graph: serde_json::Value,
}

/// Result of a root-level set operation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SetRootResult {
    /// The formatted, validated document.
    pub source: String,
    /// The field that was updated or created.
    pub key: String,
}

/// Result of deleting a root field.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DeleteRootResult {
    /// The formatted, validated document.
    pub source: String,
    /// Whether a matching root field was present.
    pub deleted: bool,
}

/// Result of a compiler-resolved D2 Delete operation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DeleteObjectResult {
    /// Complete formatted source after deleting and updating references.
    pub source: String,
    /// Complete pre-measurement graph from the native post-edit compile.
    /// Access directly; legacy serialized source-edit responses stay unchanged.
    #[serde(skip_serializing)]
    pub graph: serde_json::Value,
}

/// Result of a compiler-resolved D2 Set operation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SetObjectResult {
    /// Complete formatted source after setting the literal value.
    pub source: String,
    /// Complete pre-measurement graph from the native post-edit compile.
    /// Access directly; legacy serialized source-edit responses stay unchanged.
    #[serde(skip_serializing)]
    pub graph: serde_json::Value,
}

/// Result of renaming a root field.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RenameRootResult {
    /// The formatted, validated document.
    pub source: String,
    /// The selected destination key after uniqueness suffixing.
    pub key: String,
}

/// Result of a compiler-resolved Rename operation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RenameObjectResult {
    /// Complete formatted source after reference updates.
    pub source: String,
    /// Generated name returned by D2's Rename wrapper.
    pub key: String,
    /// Complete pre-measurement graph from the native post-edit compile.
    /// Access directly; legacy serialized source-edit responses stay unchanged.
    #[serde(skip_serializing)]
    pub graph: serde_json::Value,
}

/// Result of reconnecting one source-level edge.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ReconnectResult {
    /// The formatted, validated document.
    pub source: String,
    /// Whether an edge matching the requested key was changed.
    pub changed: bool,
}

/// Compiler-resolved edge reconnection, including its complete returned graph.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ReconnectObjectResult {
    pub source: String,
    pub changed: bool,
    pub graph: serde_json::Value,
}

/// Result of moving an object or changing an indexed edge's arrows.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct MoveObjectResult {
    /// The formatted, validated document.
    pub source: String,
    /// The selected destination object path or indexed edge key after suffixing.
    pub key: String,
    /// Complete pre-measurement graph from the native post-edit compile.
    /// Access directly; legacy serialized source-edit responses stay unchanged.
    #[serde(skip_serializing)]
    pub graph: serde_json::Value,
}

/// Source context for compiler-backed object edits.
#[derive(Clone, Debug)]
pub struct MoveOptions {
    /// Path used for diagnostics and relative import resolution.
    pub path: String,
    /// Virtual files available to imports (the supplied source is the entry).
    pub files: std::collections::HashMap<String, String>,
    /// Board names, traversed through layers, scenarios and steps in that order.
    pub board_path: Vec<String>,
}

impl Default for MoveOptions {
    fn default() -> Self {
        Self {
            path: "-".into(),
            files: Default::default(),
            board_path: Vec::new(),
        }
    }
}

/// Errors returned by [`create_root_field`].
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum GraphEditError {
    /// The source document did not parse completely.
    #[error("source parse failed: {0}")]
    SourceParse(String),
    /// The requested key did not parse as one direct root key.
    #[error("root field key is invalid: {0}")]
    InvalidKey(String),
    /// Formatting produced a document that could not be parsed again.
    #[error("formatted source failed validation: {0}")]
    Validation(String),
    /// The requested graph edit needs compiler semantics this AST-only slice
    /// does not yet implement.
    #[error("move is not supported for this source shape: {0}")]
    UnsupportedMove(String),
    /// A compiler-resolved edit failed, retaining the source oracle's context.
    #[error("{0}")]
    Move(String),
    /// An unwrapped ReconnectEdge error (unlike the contextual edit wrappers).
    #[error("{0}")]
    Reconnect(String),
    #[error("operation would modify AST outside of given scope")]
    OutsideScope,
}

/// Reconnect using compiler references, including inherited board edges.
pub fn reconnect_edge_with_options(
    source: &str,
    edge_key: &str,
    new_src: Option<&str>,
    new_dst: Option<&str>,
    options: &MoveOptions,
) -> Result<ReconnectObjectResult, GraphEditError> {
    let parsed = parse(&options.path, source.as_bytes(), None);
    if let Some(error) = parsed.error {
        return Err(GraphEditError::SourceParse(error.to_string()));
    }
    compiled_move::reconnect(parsed.map, edge_key, new_src, new_dst, options)
}

fn plain_object_path(key: &Key) -> Option<Vec<String>> {
    if key.ampersand
        || key.not_ampersand
        || key.edges.is_some()
        || key.edge_index.is_some()
        || key.edge_key.is_some()
    {
        return None;
    }
    let path = key.key.as_ref()?.read().string_ida()?;
    (!path.is_empty() && path.iter().all(|part| !is_reserved_keyword(part))).then_some(path)
}

fn checked_object_path(input: &str) -> Result<Vec<String>, GraphEditError> {
    let parsed = parse_map_key(input.as_bytes())
        .map_err(|error| GraphEditError::InvalidKey(error.to_string()))?;
    if parsed
        .key
        .as_ref()
        .is_some_and(|path| path.read().has_glob())
    {
        return Err(GraphEditError::InvalidKey(
            "move keys cannot contain glob patterns".into(),
        ));
    }
    let path = (!parsed.ampersand
        && !parsed.not_ampersand
        && parsed.edges.is_none()
        && parsed.edge_index.is_none()
        && parsed.edge_key.is_none())
    .then(|| {
        parsed
            .key
            .as_ref()
            .and_then(|path| path.read().string_ida())
    })
    .flatten()
    .filter(|path| !path.is_empty())
    .ok_or_else(|| GraphEditError::InvalidKey("move keys must be plain object paths".into()))?;
    Ok(path)
}

fn format_path(path: &[String]) -> String {
    let key_path = KeyPath::from_strings(path.iter().cloned());
    String::from_utf8(format::format(&Child::KeyPath(Shared::new(key_path))))
        .expect("D2 formatter emits UTF-8")
        .trim()
        .to_owned()
}

fn object_scope(scope: &[String], key_path: &[String]) -> Vec<String> {
    let object_parts = key_path
        .iter()
        .take_while(|part| !is_reserved_keyword(part))
        .cloned()
        .collect::<Vec<_>>();
    resolve_scoped_path(scope, &object_parts)
        .unwrap_or_else(|_| scope.iter().cloned().chain(object_parts).collect())
}

/// Resolves the leading `_` parent markers using the map's object scope.
/// Pinned D2 permits one or more markers only at the beginning of a path; each
/// marker climbs one graph-object parent before the remaining components are
/// resolved.
fn resolve_scoped_path(scope: &[String], path: &[String]) -> Result<Vec<String>, GraphEditError> {
    let parents = path.iter().take_while(|part| *part == "_").count();
    if path[parents..].iter().any(|part| part == "_") {
        return Err(GraphEditError::UnsupportedMove(
            "parent `_` can only appear at the beginning of a path".into(),
        ));
    }
    if parents > scope.len() {
        return Err(GraphEditError::UnsupportedMove(
            "parent `_` cannot be used in the root scope".into(),
        ));
    }
    if parents == path.len() {
        return Ok(scope[..scope.len() - parents].to_vec());
    }
    let mut absolute = scope[..scope.len() - parents].to_vec();
    absolute.extend_from_slice(&path[parents..]);
    Ok(absolute)
}

fn map_nodes(map: &Map) -> Vec<MapNodeBox> {
    map.nodes
        .iter()
        .flat_map(SharedSlice::iter)
        .map(|node| node.read().clone())
        .collect()
}

fn validate_move_source(map: &Map, root: bool) -> Result<(), GraphEditError> {
    for node in map_nodes(map) {
        if node.import.is_some() {
            return Err(GraphEditError::UnsupportedMove(
                "documents with imports need compiler-resolved references".into(),
            ));
        }
        let Some(key_ref) = node.map_key else {
            continue;
        };
        let key = key_ref.read();
        if root
            && key
                .key
                .as_ref()
                .and_then(|path| path.read().string_ida())
                .is_some_and(|path| {
                    path.first().is_some_and(|part| {
                        matches!(part.as_str(), "layers" | "scenarios" | "steps")
                    })
                })
        {
            return Err(GraphEditError::UnsupportedMove(
                "documents with boards need board-scoped compiler resolution".into(),
            ));
        }
        if key.key.as_ref().is_some_and(|path| path.read().has_glob()) {
            return Err(GraphEditError::UnsupportedMove(
                "glob references need compiler resolution".into(),
            ));
        }
        if let Some(edges) = &key.edges {
            for edge in edges.iter().filter_map(|slot| slot.read().clone()) {
                let edge = edge.read();
                if edge
                    .src
                    .iter()
                    .chain(edge.dst.iter())
                    .any(|path| path.read().has_glob())
                {
                    return Err(GraphEditError::UnsupportedMove(
                        "glob edge references need compiler resolution".into(),
                    ));
                }
            }
        }
        if let Some(child) = key.value.map.clone() {
            validate_move_source(&child.read(), false)?;
        }
    }
    Ok(())
}

fn direct_key_path(key: &d2_rs_core::ast::Key) -> Result<String, GraphEditError> {
    if key.ampersand || key.not_ampersand || key.edges.is_some() || key.edge_index.is_some() {
        return Err(GraphEditError::InvalidKey(
            "filters and edge expressions are not root fields".into(),
        ));
    }
    if key.edge_key.is_some() {
        return Err(GraphEditError::InvalidKey(
            "edge target expressions are not root fields".into(),
        ));
    }
    let path = key
        .key
        .as_ref()
        .and_then(|path| path.read().string_ida())
        .ok_or_else(|| GraphEditError::InvalidKey("key path is empty".into()))?;
    if path.len() != 1 || path[0].is_empty() {
        return Err(GraphEditError::InvalidKey(
            "root creation requires exactly one non-empty key component".into(),
        ));
    }
    Ok(path[0].clone())
}

fn root_keys(map: &Map) -> impl Iterator<Item = String> + '_ {
    map.nodes
        .iter()
        .flat_map(|nodes| nodes.iter())
        .filter_map(|slot| slot.read().map_key.clone())
        .filter_map(|key| key.read().key.clone())
        .filter_map(|path| path.read().string_ida())
        .filter_map(|path| (path.len() == 1).then(|| path[0].clone()))
}

fn format_key(key: &d2_rs_core::ast::Key) -> String {
    String::from_utf8(format::format(&Child::Key(Shared::new(key.clone()))))
        .expect("D2 formatter emits UTF-8")
        .trim()
        .to_owned()
}

fn finish_map(map: Map) -> Result<String, GraphEditError> {
    let output = String::from_utf8(format::format(&Child::Map(Shared::new(map))))
        .expect("D2 formatter emits UTF-8");
    let validation = parse("-", output.as_bytes(), None);
    if let Some(error) = validation.error {
        return Err(GraphEditError::Validation(error.to_string()));
    }
    Ok(output)
}

fn parse_source(source: &str) -> Result<Map, GraphEditError> {
    let parsed = parse("-", source.as_bytes(), None);
    if let Some(error) = parsed.error {
        Err(GraphEditError::SourceParse(error.to_string()))
    } else {
        Ok(parsed.map)
    }
}

fn key_matches(key: &Key, wanted: &str) -> bool {
    key.key
        .as_ref()
        .and_then(|path| path.read().string_ida())
        .is_some_and(|path| path.len() == 1 && path[0] == wanted)
}

fn edge_index_equal(left: Option<&Shared<EdgeIndex>>, right: Option<&Shared<EdgeIndex>>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => {
            let left = left.read();
            let right = right.read();
            left.glob == right.glob
                && left.int.as_ref().map(|value| *value.read())
                    == right.int.as_ref().map(|value| *value.read())
        }
        (None, None) => true,
        _ => false,
    }
}

type EdgeSignature = (Vec<String>, String, Vec<String>, String);
type EdgeOrdinalCounts = std::collections::BTreeMap<EdgeSignature, i64>;

fn edge_signature(edge: &d2_rs_core::ast::Edge, owner_scope: &[String]) -> EdgeSignature {
    (
        absolute_edge_path(edge.src.as_ref(), owner_scope),
        edge.src_arrow.clone(),
        absolute_edge_path(edge.dst.as_ref(), owner_scope),
        edge.dst_arrow.clone(),
    )
}

fn source_edge_index(
    key: &Key,
    edge: &d2_rs_core::ast::Edge,
    owner_scope: &[String],
    ordinals: &mut EdgeOrdinalCounts,
) -> Option<i64> {
    if let Some(index) = &key.edge_index {
        let index = index.read();
        if index.glob {
            return None;
        }
        return index.int.as_ref().map(|value| *value.read());
    }
    if key.edge_key.is_some() {
        return None;
    }
    let index = ordinals
        .entry(edge_signature(edge, owner_scope))
        .or_default();
    let result = *index;
    *index += 1;
    Some(result)
}

fn source_edge_index_matches(
    source: Option<&Shared<EdgeIndex>>,
    requested: Option<&Shared<EdgeIndex>>,
    derived_source_index: Option<i64>,
) -> bool {
    if source.is_some() {
        return edge_index_equal(source, requested);
    }
    let Some(requested) = requested else {
        return false;
    };
    let requested = requested.read();
    !requested.glob
        && requested
            .int
            .as_ref()
            .is_some_and(|value| Some(*value.read()) == derived_source_index)
}

/// Compares edge-key modifiers that affect which source references are
/// writable. Endpoint identity and the edge index are resolved separately;
/// reserved edge-key paths point to metadata on the same graph edge.
fn edge_key_header_equal(left: &Key, right: &Key) -> bool {
    left.ampersand == right.ampersand && left.not_ampersand == right.not_ampersand
}

fn absolute_edge_path(path: Option<&Shared<KeyPath>>, owner_scope: &[String]) -> Vec<String> {
    owner_scope
        .iter()
        .cloned()
        .chain(
            path.and_then(|path| path.read().string_ida())
                .unwrap_or_default(),
        )
        .collect()
}

fn absolute_key_path(path: &KeyPath) -> Vec<String> {
    path.string_ida().unwrap_or_default()
}

fn edge_segment_equal(
    left: &d2_rs_core::ast::Edge,
    left_scope: &[String],
    right: &d2_rs_core::ast::Edge,
    right_scope: &[String],
) -> bool {
    absolute_edge_path(left.src.as_ref(), left_scope)
        == absolute_edge_path(right.src.as_ref(), right_scope)
        && left.src_arrow == right.src_arrow
        && absolute_edge_path(left.dst.as_ref(), left_scope)
            == absolute_edge_path(right.dst.as_ref(), right_scope)
        && left.dst_arrow == right.dst_arrow
}

/// Resolve one indexed reference to its segment in a source key. Endpoints
/// are compared after applying each key's source scope, while duplicate edge
/// indices are assigned in source order as D2 does during graph compilation.
fn selected_chain_segment(
    source: &Key,
    source_scope: &[String],
    requested: &Key,
    ordinals: &mut EdgeOrdinalCounts,
) -> Option<usize> {
    if !edge_key_header_equal(source, requested) {
        return None;
    }
    let source_owner_scope = source
        .key
        .as_ref()
        .and_then(|path| path.read().string_ida())
        .map(|path| object_scope(source_scope, &path))
        .unwrap_or_else(|| source_scope.to_vec());
    let requested_owner_scope = requested
        .key
        .as_ref()
        .and_then(|path| path.read().string_ida())
        .map(|path| object_scope(&[], &path))
        .unwrap_or_default();
    let requested_edges = requested.edges.as_ref()?;
    if requested_edges.len() != 1 {
        return None;
    }
    let source_edges = source.edges.as_ref()?;
    let requested_key = requested;
    let requested_edge = requested_edges.get(0)?.read().as_ref().cloned()?;
    source_edges.iter().enumerate().find_map(|(index, edge)| {
        let source_edge = edge.read().as_ref().cloned()?;
        let source_edge = source_edge.read();
        let source_index = source_edge_index(source, &source_edge, &source_owner_scope, ordinals);
        (source_edge_index_matches(
            source.edge_index.as_ref(),
            requested_key.edge_index.as_ref(),
            source_index,
        ) && edge_segment_equal(
            &source_edge,
            &source_owner_scope,
            &requested_edge.read(),
            &requested_owner_scope,
        ))
        .then_some(index)
    })
}

fn endpoint_path(value: &str) -> Result<KeyPath, GraphEditError> {
    let endpoint = parse_map_key(value.as_bytes())
        .map_err(|error| GraphEditError::InvalidKey(error.to_string()))?;
    if endpoint.ampersand
        || endpoint.not_ampersand
        || endpoint
            .edges
            .as_ref()
            .is_some_and(|edges| !edges.is_empty())
        || endpoint.edge_index.is_some()
        || endpoint.edge_key.is_some()
    {
        return Err(GraphEditError::InvalidKey(
            "edge endpoints must be a plain key path".into(),
        ));
    }
    endpoint
        .key
        .map(|path| path.read().clone())
        .filter(|path| path.string_ida().is_some_and(|parts| !parts.is_empty()))
        .ok_or_else(|| GraphEditError::InvalidKey("edge endpoint is empty".into()))
}

fn path_from_scope(target: &KeyPath, scope: &[String]) -> KeyPath {
    let target = target.string_ida().unwrap_or_default();
    let common = scope
        .iter()
        .zip(target.iter())
        .take_while(|(left, right)| left == right)
        .count();
    KeyPath::from_strings(
        std::iter::repeat_n("_".to_owned(), scope.len() - common)
            .chain(target.into_iter().skip(common)),
    )
}

#[derive(Default)]
struct GraphEditFacts {
    declared_objects: std::collections::BTreeSet<Vec<String>>,
    edge_reference_counts: std::collections::BTreeMap<Vec<String>, usize>,
}

fn collect_graph_edit_facts(map: &Map, scope: &[String], facts: &mut GraphEditFacts) {
    for node in map_nodes(map) {
        let Some(key_ref) = node.map_key else {
            continue;
        };
        let key = key_ref.read();
        let key_path = key.key.as_ref().and_then(|path| path.read().string_ida());
        let owner_scope = key_path
            .as_deref()
            .map(|path| object_scope(scope, path))
            .unwrap_or_else(|| scope.to_vec());
        if plain_object_path(&key).is_some() {
            facts.declared_objects.insert(owner_scope.clone());
        }
        if key.edge_key.is_none()
            && let Some(edges) = &key.edges
        {
            for edge in edges.iter().filter_map(|edge| edge.read().clone()) {
                let edge = edge.read();
                for endpoint in [edge.src.as_ref(), edge.dst.as_ref()].into_iter().flatten() {
                    let endpoint_parts = endpoint.read().string_ida().unwrap_or_default();
                    if endpoint_parts.iter().any(|part| part == "_") {
                        continue;
                    }
                    let absolute = absolute_edge_path(Some(endpoint), &owner_scope);
                    *facts.edge_reference_counts.entry(absolute).or_default() += 1;
                }
            }
        }
        let child = key.value.map.clone();
        drop(key);
        if let Some(child) = child {
            collect_graph_edit_facts(&child.read(), &owner_scope, facts);
        }
    }
}

fn should_preserve_endpoint(
    path: &KeyPath,
    owner_scope: &[String],
    facts: &GraphEditFacts,
) -> bool {
    let path = absolute_edge_path(Some(&Shared::new(path.clone())), owner_scope);
    if path.is_empty() || path.iter().any(|part| part == "_") {
        return false;
    }
    let has_declared_object = facts
        .declared_objects
        .iter()
        .any(|declared| declared.starts_with(&path));
    !has_declared_object
        && facts
            .edge_reference_counts
            .get(&path)
            .copied()
            .unwrap_or_default()
            <= 1
}

fn graph_object_exists(path: &KeyPath, facts: &GraphEditFacts) -> bool {
    let Some(path) = path.string_ida() else {
        return false;
    };
    !path.is_empty()
        && facts
            .declared_objects
            .iter()
            .chain(facts.edge_reference_counts.keys())
            .any(|known| known.starts_with(&path))
}

fn preserved_endpoint_key(owner: Option<&Shared<KeyPath>>, endpoint: &KeyPath) -> Key {
    let owner = owner
        .and_then(|path| path.read().string_ida())
        .unwrap_or_default();
    let endpoint = endpoint.string_ida().unwrap_or_default();
    Key {
        key: Some(Shared::new(KeyPath::from_strings(
            owner.into_iter().chain(endpoint),
        ))),
        ..Key::default()
    }
}

fn reconnect_map(
    map: &mut Map,
    wanted: &Key,
    source: Option<&KeyPath>,
    destination: Option<&KeyPath>,
    scope: &[String],
    facts: &GraphEditFacts,
    ordinals: &mut EdgeOrdinalCounts,
) -> bool {
    let mut changed_any = false;
    let slots = map
        .nodes
        .iter()
        .flat_map(|nodes| nodes.iter())
        .collect::<Vec<_>>();
    for slot in slots {
        let node = slot.write();
        let Some(key) = node.map_key.clone() else {
            continue;
        };
        drop(node);
        let key_snapshot = key.read().clone();
        let owner_scope = key_snapshot
            .key
            .as_ref()
            .and_then(|path| path.read().string_ida())
            .map(|path| object_scope(scope, &path))
            .unwrap_or_else(|| scope.to_vec());
        if let Some(segment_index) = selected_chain_segment(&key_snapshot, scope, wanted, ordinals)
        {
            let edges = key_snapshot
                .edges
                .as_ref()
                .expect("matched edge key has segments")
                .iter()
                .filter_map(|edge| edge.read().clone())
                .collect::<Vec<_>>();
            if edges.len() > 1 {
                let split_chain = !((segment_index == 0 && destination.is_none())
                    || (segment_index == edges.len() - 1 && source.is_none()));
                if split_chain {
                    let selected = Key {
                        edges: Some(vec![Some(edges[segment_index].clone())].into()),
                        ..key_snapshot.clone()
                    };
                    if segment_index + 1 < edges.len() {
                        let remainder = Key {
                            edges: Some(
                                edges[segment_index + 1..]
                                    .iter()
                                    .cloned()
                                    .map(Some)
                                    .collect::<Vec<_>>()
                                    .into(),
                            ),
                            ..key_snapshot.clone()
                        };
                        let cursor = Child::Key(key.clone());
                        map.insert_after(Some(&cursor), Some(Child::Key(Shared::new(selected))));
                        map.insert_after(Some(&cursor), Some(Child::Key(Shared::new(remainder))));
                    } else {
                        let cursor = Child::Key(key.clone());
                        map.insert_after(Some(&cursor), Some(Child::Key(Shared::new(selected))));
                    }
                    key.write().edges = Some(
                        edges[..segment_index]
                            .iter()
                            .cloned()
                            .map(Some)
                            .collect::<Vec<_>>()
                            .into(),
                    );
                    if segment_index == 0 {
                        let nodes = map
                            .nodes
                            .iter()
                            .flat_map(SharedSlice::iter)
                            .map(|entry| entry.read().clone())
                            .filter(|entry| {
                                !entry
                                    .map_key
                                    .as_ref()
                                    .is_some_and(|candidate| candidate.ptr_eq(&key))
                            })
                            .collect::<Vec<_>>();
                        map.nodes = Some(nodes.into());
                    }
                }
            }
            let edge = edges[segment_index].clone();
            let edge_snapshot = edge.read().clone();
            let source_changes = source.is_some_and(|source| {
                absolute_key_path(source)
                    != absolute_edge_path(edge_snapshot.src.as_ref(), &owner_scope)
            });
            let destination_changes = destination.is_some_and(|destination| {
                absolute_key_path(destination)
                    != absolute_edge_path(edge_snapshot.dst.as_ref(), &owner_scope)
            });
            let source_and_destination_match = edge_snapshot
                .src
                .as_ref()
                .zip(edge_snapshot.dst.as_ref())
                .is_some_and(|(source, destination)| {
                    absolute_edge_path(Some(source), &owner_scope)
                        == absolute_edge_path(Some(destination), &owner_scope)
                });
            if key_snapshot.edge_key.is_none()
                && source_changes != destination_changes
                && !source_and_destination_match
            {
                let cursor = Child::Key(key.clone());
                if source_changes
                    && let Some(old_source) = edge_snapshot.src.as_ref()
                    && should_preserve_endpoint(&old_source.read(), &owner_scope, facts)
                {
                    let preserved =
                        preserved_endpoint_key(key_snapshot.key.as_ref(), &old_source.read());
                    map.insert_before(Some(&cursor), Some(Child::Key(Shared::new(preserved))));
                }
                if destination_changes
                    && let Some(old_destination) = edge_snapshot.dst.as_ref()
                    && should_preserve_endpoint(&old_destination.read(), &owner_scope, facts)
                {
                    let preserved =
                        preserved_endpoint_key(key_snapshot.key.as_ref(), &old_destination.read());
                    map.insert_after(Some(&cursor), Some(Child::Key(Shared::new(preserved))));
                }
            }
            let mut changed = false;
            if let Some(source) = source {
                edge.write().src = Some(Shared::new(path_from_scope(source, &owner_scope)));
                changed = true;
            }
            if let Some(destination) = destination {
                edge.write().dst = Some(Shared::new(path_from_scope(destination, &owner_scope)));
                changed = true;
            }
            changed_any |= changed;
        }
        let child = key.read().value.map.clone();
        if let Some(child) = child {
            if reconnect_map(
                &mut child.write(),
                wanted,
                source,
                destination,
                &owner_scope,
                facts,
                ordinals,
            ) {
                changed_any = true;
            }
        }
    }
    changed_any
}

fn find_matching_edge(
    map: &Map,
    wanted: &Key,
    scope: &[String],
    ordinals: &mut EdgeOrdinalCounts,
) -> Option<(Shared<d2_rs_core::ast::Edge>, Vec<String>)> {
    for node in map_nodes(map) {
        let Some(key_ref) = node.map_key else {
            continue;
        };
        let key = key_ref.read();
        let key_snapshot = key.clone();
        let owner_scope = key_snapshot
            .key
            .as_ref()
            .and_then(|path| path.read().string_ida())
            .map(|path| object_scope(scope, &path))
            .unwrap_or_else(|| scope.to_vec());
        if let Some(segment_index) = selected_chain_segment(&key_snapshot, scope, wanted, ordinals)
            && let Some(edge) = key_snapshot
                .edges
                .as_ref()
                .and_then(|edges| edges.get(segment_index))
                .and_then(|edge| edge.read().clone())
        {
            return Some((edge, owner_scope));
        }
        let child = key_snapshot.value.map.clone();
        drop(key);
        if let Some(child) = child
            && let Some(found) = find_matching_edge(&child.read(), wanted, &owner_scope, ordinals)
        {
            return Some(found);
        }
    }
    None
}

fn root_key_slots(map: &Map) -> Vec<Shared<d2_rs_core::ast::MapNodeBox>> {
    map.nodes.iter().flat_map(|nodes| nodes.iter()).collect()
}

fn set_value(key: &mut Key, value: Option<&str>) -> Result<(), GraphEditError> {
    key.value = match value {
        Some(value) => {
            let parsed = parse_value(value.as_bytes())
                .map_err(|error| GraphEditError::InvalidKey(error.to_string()))?;
            if let Child::Map(map) = &parsed {
                // A helper parse starts at byte/column zero.  That is a valid
                // source position for a file map, but this node is a value
                // map and must retain braces when the parent is formatted.
                let mut map = map.write();
                map.range.start.column = 1;
                map.range.start.byte = 1;
            }
            ValueBox::from_child(Some(parsed))
        }
        None => ValueBox::default(),
    };
    Ok(())
}

/// Creates a direct field in the root map and validates the formatted result.
///
/// Existing root keys receive the same human-readable numeric suffix shape
/// used by D2 (`name 2`, `name 3`, ...).  Nested paths, edges, filters, and
/// import-aware graph edits are intentionally rejected until their compiler
/// and board semantics are translated.
pub fn create_root_field(
    source: &str,
    requested_key: &str,
) -> Result<CreateRootResult, GraphEditError> {
    let parsed_key = parse_map_key(requested_key.as_bytes())
        .map_err(|error| GraphEditError::InvalidKey(error.to_string()))?;
    let base = direct_key_path(&parsed_key)?;

    let mut map = parse_source(source)?;
    validate_move_source(&map, true)?;
    let existing = root_keys(&map).collect::<std::collections::BTreeSet<_>>();
    let mut selected = base.clone();
    let mut suffix = 2;
    while existing.contains(&selected) {
        selected = format!("{base} {suffix}");
        suffix += 1;
    }
    let selected_key = if selected == base {
        parsed_key
    } else {
        parse_map_key(selected.as_bytes())
            .map_err(|error| GraphEditError::InvalidKey(error.to_string()))?
    };
    let formatted_key = format_key(&selected_key);
    map.insert_after(None, Some(Child::Key(Shared::new(selected_key))));
    let output = finish_map(map)?;
    Ok(CreateRootResult {
        source: output,
        key: formatted_key,
    })
}

/// Create a unique object or edge using the compiled board and import context.
pub fn create_object_with_options(
    source: &str,
    key: &str,
    options: &MoveOptions,
) -> Result<CreateObjectResult, GraphEditError> {
    let parsed = parse(&options.path, source.as_bytes(), None);
    if let Some(error) = parsed.error {
        return Err(GraphEditError::SourceParse(error.to_string()));
    }
    compiled_move::create(parsed.map, key, options)
}

pub fn create_object(source: &str, key: &str) -> Result<CreateObjectResult, GraphEditError> {
    create_object_with_options(source, key, &MoveOptions::default())
}

/// Sets or creates one direct root field and validates the resulting source.
/// `value` is D2 syntax, rather than a quoted host-language string.
pub fn set_root_field(
    source: &str,
    requested_key: &str,
    value: Option<&str>,
) -> Result<SetRootResult, GraphEditError> {
    let parsed_key = parse_map_key(requested_key.as_bytes())
        .map_err(|error| GraphEditError::InvalidKey(error.to_string()))?;
    let base = direct_key_path(&parsed_key)?;
    let mut map = parse_source(source)?;
    let mut selected = base.clone();
    let mut found = false;
    for slot in root_key_slots(&map) {
        let Some(key) = slot.read().map_key.clone() else {
            continue;
        };
        if key_matches(&key.read(), &base) {
            found = true;
            set_value(&mut key.write(), value)?;
            break;
        }
    }
    if !found {
        let existing = root_keys(&map).collect::<std::collections::BTreeSet<_>>();
        let mut suffix = 2;
        while existing.contains(&selected) {
            selected = format!("{base} {suffix}");
            suffix += 1;
        }
        let mut new_key = if selected == base {
            parsed_key
        } else {
            parse_map_key(selected.as_bytes())
                .map_err(|error| GraphEditError::InvalidKey(error.to_string()))?
        };
        set_value(&mut new_key, value)?;
        map.insert_after(None, Some(Child::Key(Shared::new(new_key))));
    }
    let output = finish_map(map)?;
    Ok(SetRootResult {
        source: output,
        key: if found { base } else { selected },
    })
}

/// Deletes one direct root field. Missing fields are a deterministic no-op,
/// matching the graph editor's delete behavior for absent objects.
pub fn delete_root_field(
    source: &str,
    requested_key: &str,
) -> Result<DeleteRootResult, GraphEditError> {
    let parsed_key = parse_map_key(requested_key.as_bytes())
        .map_err(|error| GraphEditError::InvalidKey(error.to_string()))?;
    let wanted = direct_key_path(&parsed_key)?;
    let mut map = parse_source(source)?;
    let mut deleted = false;
    let nodes = root_key_slots(&map)
        .into_iter()
        .filter_map(|slot| {
            let remove = slot
                .read()
                .map_key
                .as_ref()
                .is_some_and(|key| key_matches(&key.read(), &wanted));
            if remove {
                deleted = true;
                None
            } else {
                Some(slot.read().clone())
            }
        })
        .collect::<Vec<_>>();
    map.nodes = Some(nodes.into());
    Ok(DeleteRootResult {
        source: finish_map(map)?,
        deleted,
    })
}

/// Deletes a compiled object, indexed edge or reserved field. Container
/// children are hoisted and references are updated as in D2's Delete API.
pub fn delete_object(source: &str, key: &str) -> Result<DeleteObjectResult, GraphEditError> {
    delete_object_with_options(source, key, &MoveOptions::default())
}

/// Delete with explicit import and board context.
pub fn delete_object_with_options(
    source: &str,
    key: &str,
    options: &MoveOptions,
) -> Result<DeleteObjectResult, GraphEditError> {
    let parsed = parse(&options.path, source.as_bytes(), None);
    if let Some(error) = parsed.error {
        return Err(GraphEditError::SourceParse(error.to_string()));
    }
    compiled_move::delete(parsed.map, key, options)
}

/// Sets a compiled object label or reserved property. Values are literal
/// strings, matching D2's Set API; an optional tag creates a block string.
pub fn set_object(
    source: &str,
    key: &str,
    tag: Option<&str>,
    value: Option<&str>,
) -> Result<SetObjectResult, GraphEditError> {
    set_object_with_options(source, key, tag, value, &MoveOptions::default())
}

/// Set with explicit import and board context.
pub fn set_object_with_options(
    source: &str,
    key: &str,
    tag: Option<&str>,
    value: Option<&str>,
    options: &MoveOptions,
) -> Result<SetObjectResult, GraphEditError> {
    let parsed = parse(&options.path, source.as_bytes(), None);
    if let Some(error) = parsed.error {
        return Err(GraphEditError::SourceParse(error.to_string()));
    }
    compiled_move::set(parsed.map, key, tag, value, options)
}

/// Renames one direct root field, applying a numeric suffix when the
/// destination already exists. Descendant and import-aware moves remain open.
pub fn rename_root_field(
    source: &str,
    old_key: &str,
    requested_new_key: &str,
) -> Result<RenameRootResult, GraphEditError> {
    let old = parse_map_key(old_key.as_bytes())
        .map_err(|error| GraphEditError::InvalidKey(error.to_string()))?;
    let new = parse_map_key(requested_new_key.as_bytes())
        .map_err(|error| GraphEditError::InvalidKey(error.to_string()))?;
    let old_path = direct_key_path(&old)?;
    let new_base = direct_key_path(&new)?;
    let map = parse_source(source)?;
    let exists = root_keys(&map).collect::<std::collections::BTreeSet<_>>();
    let mut selected = new_base.clone();
    let mut suffix = 2;
    while exists.contains(&selected) && selected != old_path {
        selected = format!("{new_base} {suffix}");
        suffix += 1;
    }
    let mut renamed = false;
    for slot in root_key_slots(&map) {
        let Some(key) = slot.read().map_key.clone() else {
            continue;
        };
        if !key_matches(&key.read(), &old_path) {
            continue;
        }
        let replacement = if selected == new_base {
            new.clone()
        } else {
            parse_map_key(selected.as_bytes())
                .map_err(|error| GraphEditError::InvalidKey(error.to_string()))?
        };
        key.write().key = replacement.key;
        renamed = true;
        break;
    }
    if !renamed {
        return Err(GraphEditError::InvalidKey(
            "source key does not exist".into(),
        ));
    }
    Ok(RenameRootResult {
        source: finish_map(map)?,
        key: selected,
    })
}

/// Rename an object or change an indexed edge's arrows, following
/// d2oracle.Rename. The new object name is a literal component, so a dot in
/// it is quoted rather than interpreted as a destination scope.
pub fn rename_object(
    source: &str,
    key: &str,
    new_name: &str,
) -> Result<RenameObjectResult, GraphEditError> {
    rename_object_with_options(source, key, new_name, &MoveOptions::default())
}

/// Rename with the source path, imports and board context used at compile.
pub fn rename_object_with_options(
    source: &str,
    key: &str,
    new_name: &str,
    options: &MoveOptions,
) -> Result<RenameObjectResult, GraphEditError> {
    let parsed = parse(&options.path, source.as_bytes(), None);
    if let Some(error) = parsed.error {
        return Err(GraphEditError::Move(error.to_string()));
    }
    let mut map = parsed.map;
    compiled_move::rename(&mut map, key, new_name, options)
}

/// Moves an object path to the root or an existing container map. Supported
/// matching declarations travel with it; ordinary key and edge references are
/// resolved against their source scope, including leading `_` parent climbs,
/// edge endpoints, and `near` paths. Flattened destination objects are opened
/// into nested maps. Compiled references preserve identity through declaration
/// splits, child hoisting, edge rewrites and result recompilation. For virtual
/// imports or a selected board, use [`move_object_with_options`]. An indexed
/// edge key follows D2's edge Move branch: it changes arrows on compiled
/// references without changing their written endpoint paths.
pub fn move_object(
    source: &str,
    old_key: &str,
    requested_new_key: &str,
    include_descendants: bool,
) -> Result<MoveObjectResult, GraphEditError> {
    move_object_with_options(
        source,
        old_key,
        requested_new_key,
        include_descendants,
        &MoveOptions::default(),
    )
}

/// Moves an object using an explicit source path, virtual files and board.
pub fn move_object_with_options(
    source: &str,
    old_key: &str,
    requested_new_key: &str,
    include_descendants: bool,
    options: &MoveOptions,
) -> Result<MoveObjectResult, GraphEditError> {
    // The source wrapper compiles the entry document before Move checks its
    // keys or returns from a no-op. Keep parse errors at that same boundary.
    let parsed = parse(&options.path, source.as_bytes(), None);
    if let Some(error) = parsed.error {
        return Err(GraphEditError::Move(error.to_string()));
    }
    let mut map = parsed.map;
    compiled_move::prepare(
        &mut map,
        old_key,
        requested_new_key,
        include_descendants,
        options,
    )
}

/// Reconnects one written edge segment and validates the formatted source.
/// `new_src` and `new_dst` may be omitted independently. The lookup resolves
/// source paths through nested value maps, matches an optional edge index,
/// and updates source-level references to the same edge. Compiler-resolved
/// globs, imported references, and descendant propagation remain outside this
/// AST-only operation.
pub fn reconnect_edge(
    source: &str,
    edge_key: &str,
    new_src: Option<&str>,
    new_dst: Option<&str>,
) -> Result<ReconnectResult, GraphEditError> {
    let wanted = parse_map_key(edge_key.as_bytes())
        .map_err(|error| GraphEditError::InvalidKey(error.to_string()))?;
    if wanted.edges.as_ref().is_none_or(|edges| edges.len() != 1) {
        return Err(GraphEditError::InvalidKey(
            "reconnect requires exactly one edge".into(),
        ));
    }
    if wanted.edge_index.is_none() {
        return Err(GraphEditError::InvalidKey(
            "edge key must refer to an existing edge".into(),
        ));
    }
    let mut map = parse_source(source)?;
    let mut search_ordinals = EdgeOrdinalCounts::default();
    let Some((selected_edge, owner_scope)) =
        find_matching_edge(&map, &wanted, &[], &mut search_ordinals)
    else {
        return Err(GraphEditError::InvalidKey("edge not found".into()));
    };
    let selected_edge = selected_edge.read().clone();
    let source_path = new_src.map(endpoint_path).transpose()?.filter(|path| {
        absolute_key_path(path) != absolute_edge_path(selected_edge.src.as_ref(), &owner_scope)
    });
    let destination_path = new_dst.map(endpoint_path).transpose()?.filter(|path| {
        absolute_key_path(path) != absolute_edge_path(selected_edge.dst.as_ref(), &owner_scope)
    });
    if source_path.is_none() && destination_path.is_none() {
        return Ok(ReconnectResult {
            source: source.to_owned(),
            changed: false,
        });
    }
    let mut facts = GraphEditFacts::default();
    collect_graph_edit_facts(&map, &[], &mut facts);
    if let Some(source_path) = source_path.as_ref()
        && !graph_object_exists(source_path, &facts)
    {
        return Err(GraphEditError::InvalidKey("newSrc not found".into()));
    }
    if let Some(destination_path) = destination_path.as_ref()
        && !graph_object_exists(destination_path, &facts)
    {
        return Err(GraphEditError::InvalidKey("newDst not found".into()));
    }
    let mut edit_ordinals = EdgeOrdinalCounts::default();
    let changed = reconnect_map(
        &mut map,
        &wanted,
        source_path.as_ref(),
        destination_path.as_ref(),
        &[],
        &facts,
        &mut edit_ordinals,
    );
    if !changed {
        return Err(GraphEditError::InvalidKey("edge not found".into()));
    }
    Ok(ReconnectResult {
        source: finish_map(map)?,
        changed,
    })
}

#[cfg(test)]
mod tests {
    use super::{GraphEditError, create_root_field, move_object, reconnect_edge};

    #[test]
    fn creates_and_validates_a_root_field() {
        let result = create_root_field("alpha: one\n", "beta").unwrap();
        assert_eq!(result.key, "beta");
        assert_eq!(result.source, "alpha: one\nbeta\n");
    }

    #[test]
    fn chooses_a_numeric_suffix_for_duplicate_root_keys() {
        let result = create_root_field("alpha\nalpha 2\n", "alpha").unwrap();
        assert_eq!(result.key, "alpha 3");
        assert_eq!(result.source, "alpha\nalpha 2\nalpha 3\n");
    }

    #[test]
    fn rejects_nested_and_edge_requests_until_compiler_semantics_exist() {
        assert!(matches!(
            create_root_field("", "a.b"),
            Err(GraphEditError::InvalidKey(_))
        ));
        assert!(matches!(
            create_root_field("", "a -> b"),
            Err(GraphEditError::InvalidKey(_))
        ));
    }

    #[test]
    fn rejects_invalid_existing_source() {
        assert!(matches!(
            create_root_field("a: [", "b"),
            Err(GraphEditError::SourceParse(_))
        ));
    }

    #[test]
    fn sets_existing_and_creates_missing_root_values() {
        let updated = super::set_root_field("a: one\n", "a", Some("two")).unwrap();
        assert_eq!(updated.source, "a: two\n");
        let created = super::set_root_field(&updated.source, "b", Some("{x: 1}")).unwrap();
        assert_eq!(created.source, "a: two\nb: {x: 1}\n");
    }

    #[test]
    fn deletes_root_fields_and_is_idempotent_for_missing_fields() {
        let deleted = super::delete_root_field("a\nb\n", "a").unwrap();
        assert!(deleted.deleted);
        assert_eq!(deleted.source, "b\n");
        let missing = super::delete_root_field(&deleted.source, "a").unwrap();
        assert!(!missing.deleted);
        assert_eq!(missing.source, "b\n");
    }

    #[test]
    fn renames_root_fields_with_deterministic_suffixes() {
        let renamed = super::rename_root_field("a\nb\n", "a", "b").unwrap();
        assert_eq!(renamed.key, "b 2");
        assert_eq!(renamed.source, "b 2\nb\n");
    }

    #[test]
    fn reconnects_nested_single_edge_and_preserves_value() {
        let result = reconnect_edge(
            "group: {\n  a -> b: hello\n  c\n  d\n}\n",
            "(group.a -> group.b)[0]",
            Some("group.c"),
            Some("group.d"),
        )
        .unwrap();
        assert!(result.changed);
        assert_eq!(result.source, "group: {\n  c -> d: hello\n  c\n  d\n}\n");
    }

    #[test]
    fn reconnect_requires_one_written_edge_and_is_idempotent_without_endpoints() {
        assert!(matches!(
            reconnect_edge("a -> b\n", "a -> b -> c", Some("d"), None),
            Err(GraphEditError::InvalidKey(_))
        ));
        let original = "a -> b\n";
        let result = reconnect_edge(original, "(a -> b)[0]", None, None).unwrap();
        assert!(!result.changed);
        assert_eq!(result.source, original);
        let same_endpoints = reconnect_edge(original, "(a -> b)[0]", Some("a"), Some("b")).unwrap();
        assert!(!same_endpoints.changed);
        assert_eq!(same_endpoints.source, original);
        assert!(matches!(
            reconnect_edge("a -> b\n", "(a -> b)[0]", None, Some("missing")),
            Err(GraphEditError::InvalidKey(message)) if message == "newDst not found"
        ));
        assert!(matches!(
            reconnect_edge("a -> b\n", "(missing -> edge)[0]", None, Some("also_missing")),
            Err(GraphEditError::InvalidKey(message)) if message == "edge not found"
        ));
        assert!(matches!(
            reconnect_edge("a -> b\n", "(b -> a)[0]", None, Some("a")),
            Err(GraphEditError::InvalidKey(message)) if message == "edge not found"
        ));
    }

    #[test]
    fn reconnect_matches_indexed_reserved_edge_keys() {
        let result = reconnect_edge(
            "a -> b\nc\n(a -> b)[0].style.opacity: 0.4\n",
            "(a -> b)[0].style.opacity",
            Some("c"),
            None,
        )
        .unwrap();
        assert_eq!(
            result.source,
            "a\nc -> b\nc\n(c -> b)[0].style.opacity: 0.4\n"
        );
    }

    #[test]
    fn reconnect_splits_selected_segments_of_edge_chains_like_v090() {
        // These expected sources are the corresponding cases from the pinned
        // d2oracle/edit_test.go::TestReconnectEdge table.
        let cases = [
            (
                "a\nb\nc\na -> b\n",
                "(a -> b)[0]",
                None,
                Some("c"),
                "a\nb\nc\na -> c\n",
            ),
            (
                "a\nb\nc\na -> b\n",
                "(a -> b)[0]",
                Some("c"),
                None,
                "a\nb\nc\nc -> b\n",
            ),
            (
                "a\nb\nc\na -> b\n",
                "(a -> b)[0]",
                Some("b"),
                Some("a"),
                "a\nb\nc\nb -> a\n",
            ),
            ("a -> b\n", "(a -> b)[0]", Some("b"), Some("a"), "b -> a\n"),
            (
                "a -> b -> c\nx\n",
                "(a -> b)[0]",
                None,
                Some("x"),
                "b -> c\na -> x\nx\n",
            ),
            (
                "a -> b -> c\nx\n",
                "(b -> c)[0]",
                Some("x"),
                None,
                "a -> b\nx -> c\nx\n",
            ),
            (
                "a -> b -> c -> d\nx\n",
                "(b -> c)[0]",
                Some("x"),
                Some("x"),
                "a -> b\nc -> d\nx -> x\nx\n",
            ),
            (
                "a -> b -> a -> c\n",
                "(a -> b)[0]",
                None,
                Some("c"),
                "b -> a -> c\na -> c\n",
            ),
            (
                "a -> c -> a -> c\nb\n",
                "(a -> c)[0]",
                None,
                Some("b"),
                "c -> a -> c\na -> b\nb\n",
            ),
            (
                "a -> b -> c -> d\nx\n",
                "(a -> b)[0]",
                Some("x"),
                None,
                "a\nx -> b -> c -> d\nx\n",
            ),
            (
                "a -> b -> c -> d\nx\n",
                "(c -> d)[0]",
                None,
                Some("x"),
                "a -> b -> c -> x\nd\nx\n",
            ),
            (
                "a.x -> a.y\na.z\n",
                "a.(x -> y)[0]",
                None,
                Some("a.z"),
                "a.x -> a.z\na.y\na.z\n",
            ),
            ("a -> a\nb\n", "(a -> a)[0]", None, Some("b"), "a -> b\nb\n"),
            (
                "a -> b\n(a -> b)[0].style.stroke: red\nc\n",
                "(a -> b)[0]",
                Some("a"),
                Some("c"),
                "a -> c\nb\n(a -> c)[0].style.stroke: red\nc\n",
            ),
            (
                "a -> b\nx\n(a -> b)[0].style.stroke: red\n",
                "(a -> b)[0]",
                None,
                Some("x"),
                "a -> x\nb\nx\n(a -> x)[0].style.stroke: red\n",
            ),
            (
                "a -> b: {\n  style.stroke: blue\n}\na -> b: {\n  style.stroke: red\n}\nx\n",
                "(a -> b)[1]",
                None,
                Some("x"),
                "a -> b: {\n  style.stroke: blue\n}\na -> x: {\n  style.stroke: red\n}\nx\n",
            ),
            (
                "a: {\n  x -> y\n}\nb\n",
                "(a.x -> a.y)[0]",
                None,
                Some("b"),
                "a: {\n  x -> _.b\n  y\n}\nb\n",
            ),
            (
                "a: {\n  x -> y\n  z: {\n    b\n  }\n}\n",
                "(a.x -> a.y)[0]",
                None,
                Some("a.z.b"),
                "a: {\n  x -> z.b\n  y\n\n  z: {\n    b\n  }\n}\n",
            ),
        ];
        for (source, edge_key, new_src, new_dst, expected) in cases {
            let actual = reconnect_edge(source, edge_key, new_src, new_dst).unwrap();
            assert_eq!(actual.source, expected, "edge key {edge_key}");
        }
    }

    #[test]
    fn moves_a_nested_object_and_rewrites_key_and_edge_references() {
        let result = move_object(
            "a: {\n  b: { child }\n}\nx: {}\nsource -> a.b.child\n",
            "a.b",
            "x.b",
            true,
        )
        .unwrap();
        assert_eq!(result.key, "x.b");
        assert!(result.source.contains("x:"), "{}", result.source);
        assert!(
            result.source.contains("source -> x.b.child"),
            "{}",
            result.source
        );
    }

    #[test]
    fn moves_out_to_root_and_updates_flat_references() {
        let result =
            move_object("a: {\n  b\n}\na.b.style.opacity: 0.4\n", "a.b", "b", true).unwrap();
        assert_eq!(result.key, "b");
        assert!(
            result.source.contains("b.style.opacity: 0.4"),
            "{}",
            result.source
        );
    }

    #[test]
    fn moves_update_near_references() {
        let result = move_object("x: { near: y }\na: {}\ny\n", "y", "a.y", true).unwrap();
        assert!(result.source.contains("near: a.y"), "{}", result.source);
    }

    #[test]
    fn opens_an_existing_plain_destination_object_as_a_container() {
        let result = move_object("a\nc\n", "c", "a.c", true).unwrap();
        assert_eq!(result.key, "a.c");
        assert_eq!(result.source, "a: {\n  c\n}\n");
    }

    #[test]
    fn flat_moves_keep_implicit_source_parents_at_the_original_position() {
        let out = move_object("a.b: \"yo\"\n", "a.b", "b", true).unwrap();
        assert_eq!(out.source, "a\nb: \"yo\"\n");

        let between = move_object("a.b\nc\n", "a.b", "c.b", true).unwrap();
        assert_eq!(between.source, "a\nc: {\n  b\n}\n");
    }

    #[test]
    fn cross_scope_move_without_descendants_hoists_ordinary_children() {
        let result = move_object("x: {\n  y: {\n    z\n  }\n}\n", "x.y", "y", false).unwrap();
        assert_eq!(result.source, "x: {\n  z\n}\ny\n");

        let flat = move_object(
            "a.b: {\n  # this is reserved\n  shape: hexagon\n  # this is not\n  c\n}\n",
            "a.b",
            "b",
            false,
        )
        .unwrap();
        assert_eq!(
            flat.source,
            "a: {\n  # this is not\n  c\n}\nb: {\n  # this is reserved\n  shape: hexagon\n}\n"
        );

        let no_children = move_object("a: {\n  b\n}\n", "a.b", "b", false).unwrap();
        assert_eq!(no_children.source, "a\nb\n");

        let flat_value = move_object("a.b: \"yo\"\n", "a.b", "b", false).unwrap();
        assert_eq!(flat_value.source, "a\nb: \"yo\"\n");

        let flat_reserved =
            move_object("a.b: {\n  shape: hexagon\n}\n", "a.b", "b", false).unwrap();
        assert_eq!(flat_reserved.source, "a\nb: {\n  shape: hexagon\n}\n");
    }

    #[test]
    fn same_scope_move_without_descendants_keeps_the_subtree() {
        let result = move_object("a: {\n  b: {\n    c\n  }\n}\n", "a.b", "a.x", false).unwrap();
        assert_eq!(result.source, "a: {\n  x: {\n    c\n  }\n}\n");
    }

    #[test]
    fn split_move_inserts_hoisted_children_at_the_original_sibling_position() {
        let result = move_object(
            "x: {\n  before\n  y: {\n    child\n  }\n  after\n}\n",
            "x.y",
            "y",
            false,
        )
        .unwrap();
        assert_eq!(
            result.source,
            "x: {\n  before\n\n  child\n\n  after\n}\ny\n"
        );
    }

    #[test]
    fn split_move_rewrites_object_child_edge_and_near_references() {
        let result = move_object(
            "x: {\n  y: {\n    z\n  }\n}\nq: { near: x.y.z }\nsource -> x.y\nsource -> x.y.z\n",
            "x.y",
            "y",
            false,
        )
        .unwrap();
        assert!(result.source.contains("near: x.z"), "{}", result.source);
        assert!(result.source.contains("source -> y"), "{}", result.source);
        assert!(result.source.contains("source -> x.z"), "{}", result.source);
    }

    #[test]
    fn move_suffixes_conflicts_and_rejects_untranslated_shapes() {
        let result = move_object("a: { b }\nx: { b }\n", "a.b", "x.b", true).unwrap();
        assert_eq!(result.key, "x.b 2");
        assert!(result.source.contains("b 2"));
        let implicit = move_object("a.b\nz\n", "z", "a", true).unwrap();
        assert_eq!(implicit.key, "a 2");
        assert!(implicit.source.contains("a.b"));
        assert!(implicit.source.contains("a 2"));
        let split = move_object("a: { b }\n", "a.b", "b", false).unwrap();
        assert!(split.source.contains("b\n"));
        assert_eq!(
            move_object("a: { b }\n", "a.b", "missing.b", true)
                .unwrap_err()
                .to_string(),
            "failed to move: \"a.b\" to \"missing.b\": key referenced by to parent does not exist"
        );
    }

    #[test]
    fn source_move_cases_match_pinned_v090_edit_fixtures() {
        // These cases are selected from d2oracle/edit_test.go::TestMove.
        // Keeping the original source and expected formatter output here makes
        // the Rust edit path exercise upstream's source-level contract.
        let cases = [
            (
                "a: {\n  b: {\n    shape: cylinder\n  }\n}\n\na: {\n  b: {\n    shape: cylinder\n  }\n}\n",
                "a.b",
                "b",
                true,
                "a\n\na\nb: {\n  shape: cylinder\n}\n",
            ),
            (
                r#"a: {
  style: {
    opacity: 0.4
  }
}
a: {
  style: {
    fill: "red"
  }
}
d
"#,
                "a",
                "d.a",
                false,
                r#"d: {
  a: {
    style: {
      opacity: 0.4
    }
  }
  a: {
    style: {
      fill: "red"
    }
  }
}
"#,
            ),
            ("a: {\n  b\n}\nc\n", "a.b", "c.b", true, "a\nc: {\n  b\n}\n"),
            (
                "a.b\nc.d: meow\n",
                "a.b",
                "c.b",
                true,
                "a\nc: {\n  d: meow\n  b\n}\n",
            ),
            (
                "x.style.border-radius: 5\ny\n",
                "y",
                "x.y",
                true,
                "x: {\n  style.border-radius: 5\n  y\n}\n",
            ),
            (
                "a: {\n  b: {\n    c\n  }\n}\n",
                "a.b",
                "a.x",
                false,
                "a: {\n  x: {\n    c\n  }\n}\n",
            ),
            (
                "x\nx 2\nx 3: {\n  x 3\n  x 4\n}\nx 4\ny\n",
                "x 3",
                "y.x 3",
                false,
                "x\nx 2\n\nx 3\nx 5\n\nx 4\ny: {\n  x 3\n}\n",
            ),
            (
                "x: {\n  near: y\n}\na\ny\n",
                "y",
                "a.y",
                true,
                "x: {\n  near: a.y\n}\na: {\n  y\n}\n",
            ),
            (
                "a: {\n  b: {\n    c\n  }\n  b.c -> d\n}\na.b.c -> a.d\n",
                "a.b.c",
                "c",
                true,
                "a: {\n  b\n  _.c -> d\n}\nc -> a.d\nc\n",
            ),
            (
                "guitar: {\n  books: {\n    _._.pipe\n  }\n}\n",
                "pipe",
                "guitar.pipe",
                true,
                "guitar: {\n  books\n  pipe\n}\n",
            ),
            (
                "a: {\n  _.b -> c\n}\nb\n",
                "b",
                "c",
                true,
                "a: {\n  _.c -> c\n}\nc\n",
            ),
        ];
        for (source, old_key, new_key, descendants, expected) in cases {
            let actual = move_object(source, old_key, new_key, descendants)
                .unwrap_or_else(|error| panic!("{old_key} -> {new_key} in {source:?}: {error}"));
            assert_eq!(actual.source, expected, "{old_key} -> {new_key}");
        }
    }
}
