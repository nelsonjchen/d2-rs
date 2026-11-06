// SPDX-License-Identifier: MPL-2.0
//! Initial source-to-measured-graph lowering.
//!
//! This lowers the structural subset represented by the translated IR:
//! ordinary fields, nested containers, and the compiler's resolved edge set.
//! D2-specific configuration, classes, boards, and rich style/label semantics
//! are added incrementally while preserving unresolved-path errors at the
//! graph boundary.

mod source_objects;
mod source_root;
use super::Compiler;
use crate::format;
use d2_rs_core::{
    ArrowheadLabel, Direction, Edge as GraphEdge, EdgeArrowheadDeclarations, EdgeArrowheadLabels,
    EdgeArrowheads, EdgeArrows, EdgeLabel, EdgeStyle, EdgeTableColumns, ExternalAlignment,
    ExternalLabel, ExternalSide, Graph, GridSpacing, Insets, LabelPosition, Legend,
    LegendConnection, LegendShape, Node as GraphNode, NodeId as GraphNodeId, Point, ShapeKind,
    Size,
    ast::{self, Child, Node as AstNode, ScalarBox},
    ir::{Node as IrNode, NodeId as IrNodeId},
};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::OnceLock;

#[derive(Clone, Debug, PartialEq, Eq)]
/// A structural source-to-graph feature that still needs translation.
pub enum GraphCompileError {
    /// Original graph-stage cooperative cancellation/deadline causes.
    Cancelled,
    DeadlineExceeded,
    ContextFailure(String),
    /// Compilation-wide logical copy budget exhausted before materialization.
    VariableExpansionLimit(u64),
    /// Source fails D2's graph-stage validation, in diagnostic order.
    InvalidAttributes(Vec<ast::Error>),
    /// Native LaTeX intrinsic measurement failed before layout.
    LatexMeasurement(String),
    /// The IR contains an edge without both concrete endpoint paths.
    MissingEdgeEndpoint,
    /// An edge uses a wildcard or underscore path that needs D2 resolution.
    UnresolvedEdgePath(String),
    /// Two fields lower to the same external graph identity.
    DuplicateNode(String),
    /// Two edges lower to the same external graph identity.
    DuplicateEdge(String),
    /// A `near` property names a node that is not present in this graph.
    UnresolvedNear(String),
    /// An image node must provide the icon payload used by the target.
    ImageMissingIcon(String),
    /// Image nodes cannot contain graph children outside sequence diagrams.
    ImageHasChildren(String),
}

impl std::fmt::Display for GraphCompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => f.write_str("context canceled"),
            Self::DeadlineExceeded => f.write_str("context deadline exceeded"),
            Self::ContextFailure(error) => f.write_str(error),
            Self::VariableExpansionLimit(limit) => write!(
                f,
                "variable substitution expansion exceeds limit of {limit} work units"
            ),
            Self::LatexMeasurement(error) => f.write_str(error),
            Self::InvalidAttributes(errors) => {
                for (index, error) in errors.iter().enumerate() {
                    if index != 0 {
                        f.write_str("\n")?;
                    }
                    write!(f, "{error}")?;
                }
                Ok(())
            }
            Self::MissingEdgeEndpoint => f.write_str("edge is missing a source or destination"),
            Self::UnresolvedEdgePath(path) => {
                write!(
                    f,
                    "edge endpoint requires unresolved path semantics: {path:?}"
                )
            }
            Self::DuplicateNode(path) => write!(f, "duplicate graph node identity {path:?}"),
            Self::DuplicateEdge(path) => write!(f, "duplicate graph edge identity {path:?}"),
            Self::UnresolvedNear(path) => write!(f, "near target is not a graph node: {path:?}"),
            Self::ImageMissingIcon(path) => {
                write!(f, "{path}: image shape must include an \"icon\" field")
            }
            Self::ImageHasChildren(path) => write!(f, "{path}: image shapes cannot have children."),
        }
    }
}

impl std::error::Error for GraphCompileError {}

fn scalar_text(value: &ScalarBox) -> String {
    value.scalar_string()
}

fn path_text(parts: &[Option<Child>]) -> Option<String> {
    if parts.iter().any(Option::is_none) {
        return None;
    }
    let parts = parts
        .iter()
        .flatten()
        .map(|part| {
            ast::raw_string_box(
                ScalarBox::from_child(Some(part.clone())).scalar_bytes(),
                true,
            )
        })
        .collect::<Vec<_>>();
    Some(String::from_utf8_lossy(&format::format_key_path(&parts)).into_owned())
}

fn endpoint_path(path: &Option<Vec<Option<Child>>>) -> Result<String, GraphCompileError> {
    let path = path
        .as_deref()
        .ok_or(GraphCompileError::MissingEdgeEndpoint)?;
    // Concrete IR paths retain AST quoting/pattern provenance. A quoted
    // literal "*" is an object ID, not an unresolved glob. Source resolves
    // endpoints through those scalar components rather than formatted text.
    let unresolved = path.iter().flatten().any(|part| {
        if let Child::Unquoted(value) = part {
            value
                .read()
                .pattern
                .as_ref()
                .is_some_and(|pattern| !pattern.is_empty())
                || ScalarBox::from_child(Some(part.clone())).scalar_bytes() == b"_"
        } else {
            false
        }
    });
    let text = path_text(path).ok_or(GraphCompileError::MissingEdgeEndpoint)?;
    if unresolved {
        return Err(GraphCompileError::UnresolvedEdgePath(text));
    }
    Ok(text)
}

fn qualify_endpoint_at(parent: Option<&str>, endpoint: String) -> String {
    let Some(parent) = parent else {
        return endpoint;
    };
    if endpoint == parent || endpoint.starts_with(&format!("{parent}.")) {
        endpoint
    } else {
        format!("{parent}.{endpoint}")
    }
}

/// Resolve a relative edge endpoint using the nearest existing scope before
/// falling back to the syntactic parent prefix. Sequence diagrams are special
/// here: references written inside nested edge groups (`alice`, `bob.a`) are
/// resolved against the sequence root's actors and spans, just as d2graph's
/// reference resolver does, rather than materializing duplicate group-local
/// nodes.
fn resolve_endpoint_at(
    parent: Option<&str>,
    endpoint: String,
    nodes: &BTreeMap<String, GraphNodeId>,
    graph: &Graph,
    table_columns: &BTreeMap<String, Vec<String>>,
) -> String {
    if let Some(parent) = parent {
        let candidate = format!("{parent}.{endpoint}");
        if nodes.contains_key(&candidate) {
            return candidate;
        }
    }
    // Find the owning sequence scope before ordinary lexical lookup. D2's
    // sequence reference resolver binds actor names in nested edge groups to
    // the sequence root, even when a group-local field with the same spelling
    // was materialized while lowering.
    let mut sequence_root = None;
    let mut root_scope = parent;
    while let Some(prefix) = root_scope {
        if nodes
            .get(prefix)
            .is_some_and(|id| graph.is_sequence_diagram(*id))
        {
            sequence_root = Some(prefix);
            break;
        }
        root_scope = prefix.rsplit_once('.').map(|(prefix, _)| prefix);
    }
    if let Some(prefix) = sequence_root {
        let root_candidate = format!("{prefix}.{endpoint}");
        let first = endpoint.split('.').next().unwrap_or_default();
        if nodes.contains_key(&root_candidate) || nodes.contains_key(&format!("{prefix}.{first}")) {
            return root_candidate;
        }
    }
    let mut scope = parent;
    while let Some(prefix) = scope {
        let candidate = format!("{prefix}.{endpoint}");
        if nodes.contains_key(&candidate)
            || resolve_structured_endpoint(&candidate, nodes, table_columns, graph).is_some()
        {
            return candidate;
        }
        scope = prefix.rsplit_once('.').map(|(prefix, _)| prefix);
    }
    // A fully-qualified endpoint can still be written from inside a nested
    // map. Only fall back to the root namespace after trying every lexical
    // scope; otherwise a root node such as `start` shadows the nearer
    // `Check PIN.start` that D2 resolves for an edge inside that container.
    if nodes.contains_key(&endpoint)
        || resolve_structured_endpoint(&endpoint, nodes, table_columns, graph).is_some()
    {
        return endpoint;
    }
    // If the edge is below an explicit sequence root, a root actor with the
    // same first path component owns any missing descendant span. Returning
    // the root-qualified path lets the caller attach the newly implicit span
    // below that actor after insertion.
    if let Some(prefix) = sequence_root {
        let first = endpoint.split('.').next().unwrap_or_default();
        if nodes.contains_key(&format!("{prefix}.{first}")) {
            return format!("{prefix}.{endpoint}");
        }
    }
    qualify_endpoint_at(parent, endpoint)
}

fn attach_implicit_parent(
    graph: &mut Graph,
    nodes: &BTreeMap<String, GraphNodeId>,
    path: &str,
    node: GraphNodeId,
) {
    let mut scope = path.rsplit_once('.').map(|(prefix, _)| prefix);
    while let Some(prefix) = scope {
        if let Some(parent) = nodes.get(prefix).copied() {
            graph.node_mut(node).expect("implicit node exists").parent = Some(parent);
            break;
        }
        scope = prefix.rsplit_once('.').map(|(prefix, _)| prefix);
    }
}

fn field_name(compiler: &Compiler, field: IrNodeId) -> Option<String> {
    let IrNode::Field(field) = compiler.arena.node(field) else {
        return None;
    };
    Some(
        ast::ByteString::from(field.name.scalar_bytes())
            .to_text()
            .into_owned(),
    )
}

fn is_metadata_name(name: &str) -> bool {
    // d2ast.SimpleReservedKeywords does not reserve d2-config. Configuration
    // is extracted only from vars.d2-config; elsewhere it is an object name.
    matches!(
        name,
        "style"
            | "shape"
            | "label"
            | "label-position"
            | "width"
            | "height"
            | "top"
            | "left"
            | "near"
            | "icon"
            | "icon-position"
            | "tooltip"
            | "link"
            | "direction"
            | "constraint"
            | "grid-rows"
            | "grid-columns"
            | "grid-gap"
            | "vertical-gap"
            | "horizontal-gap"
            | "language"
            | "classes"
            | "class"
            | "vars"
    )
}

fn is_metadata_field(compiler: &Compiler, field: IrNodeId) -> bool {
    // compileField only treats reserved words as attributes when unquoted.
    // Quoted words remain graph objects, including nested tooltip/near names.
    let IrNode::Field(field_node) = compiler.arena.node(field) else {
        return true;
    };
    if field_node.name.unquoted_string.is_none() {
        return false;
    }
    let Some(name) = field_name(compiler, field) else {
        return true;
    };
    if is_metadata_name(&name) {
        return true;
    }
    // `class` is an object attribute, but it remains a valid top-level node
    // name (`class: {shape: class}`).  Only treat it as metadata once it is
    // inside a non-root object map.
    name == "class"
        && compiler
            .arena
            .parent_map(field)
            .is_some_and(|map| !compiler.arena.root(map))
}

fn property_scalar_in_map(compiler: &Compiler, map: IrNodeId, path: &[&str]) -> Option<ScalarBox> {
    let field = compiler.graph_field(map, path)?;
    let primary = compiler.arena.primary(field)?;
    let IrNode::Scalar(scalar) = compiler.arena.node(primary) else {
        return None;
    };
    Some(scalar.value.clone())
}

fn property_scalar(compiler: &Compiler, node: IrNodeId, path: &[&str]) -> Option<ScalarBox> {
    property_scalar_in_map(compiler, compiler.arena.map(node)?, path)
}

fn property_text(compiler: &Compiler, node: IrNodeId, path: &[&str]) -> Option<String> {
    property_scalar(compiler, node, path).map(|value| scalar_text(&value))
}

fn property_present(compiler: &Compiler, node: IrNodeId, path: &[&str]) -> bool {
    compiler
        .arena
        .map(node)
        .is_some_and(|map| compiler.graph_field(map, path).is_some())
        || class_maps(compiler, node)
            .iter()
            .any(|map| compiler.graph_field(*map, path).is_some())
}

fn arrowhead_declared_shape(compiler: &Compiler, edge: IrNodeId, endpoint: &str) -> Option<String> {
    if !property_present(compiler, edge, &[endpoint]) {
        return None;
    }
    // compileReserved lowercases shapes before ToArrowhead. Preserve that
    // compiled shape for engine-specific validation before resolving it.
    let shape = property_text_inherited(compiler, edge, &[endpoint, "shape"]).unwrap_or_default();
    Some(
        String::from_utf8(d2_rs_core::ir::go_lowercase(shape.as_bytes()))
            .expect("Go lowercase produces UTF-8"),
    )
}

fn arrowhead_shape(compiler: &Compiler, edge: IrNodeId, endpoint: &str) -> Option<String> {
    let shape = arrowhead_declared_shape(compiler, edge, endpoint)?;
    // Style scalars retain the spellings accepted by strconv.ParseBool.
    let filled = property_text_inherited(compiler, edge, &[endpoint, "style", "filled"])
        .map(|value| d2_rs_core::parse_d2_bool(&value).unwrap_or(false));
    Some(d2_rs_core::resolve_d2_arrowhead_shape(&shape, filled))
}

/// Returns the explicit board label used by paged document exporters.
///
/// compileBoardsField applies the enclosing field's primary label after
/// compileBoard has consumed the map's `label` property.
pub fn board_label(compiler: &Compiler, board: IrNodeId) -> Option<String> {
    compiler
        .arena
        .parent_field(board)
        .and_then(|field| compiler.arena.primary(field))
        .and_then(|primary| match compiler.arena.node(primary) {
            IrNode::Scalar(value) => Some(scalar_text(&value.value)),
            _ => None,
        })
        .or_else(|| property_text(compiler, board, &["label"]))
}

/// Whether a field is nested below a sequence-diagram scope. D2 keeps actor
/// labels at the normal 16 px face inside sequence diagrams even when those
/// actors themselves contain child maps; the larger container title sizes are
/// reserved for ordinary diagram containers.
fn has_sequence_ancestor(compiler: &Compiler, field: IrNodeId) -> bool {
    let mut current = compiler
        .arena
        .parent(field)
        .and_then(|map| compiler.arena.parent(map));
    while let Some(ancestor) = current {
        // compileLegend starts a separate graph; ambient host sequence shape
        // must not turn legend entries into actors or spans.
        if matches!(compiler.arena.node(ancestor), IrNode::Field(f) if f.name.unquoted_string.is_some() && f.name.scalar_string() == "d2-legend")
            && compiler.arena.parent(ancestor).and_then(|map| compiler.arena.parent_field(map))
                .is_some_and(|id| matches!(compiler.arena.node(id), IrNode::Field(f) if f.name.unquoted_string.is_some() && f.name.scalar_string() == "vars"))
        {
            return false;
        }
        if property_text_inherited(compiler, ancestor, &["shape"])
            .is_some_and(|value| value.eq_ignore_ascii_case("sequence_diagram"))
        {
            return true;
        }
        current = compiler
            .arena
            .parent(ancestor)
            .and_then(|map| compiler.arena.parent(map));
    }
    false
}

/// Apply D2 v0.9.0's object-link validation before exposing links to the
/// target. Remote links survive; local links survive only when they resolve
/// to a different, existing board. Other relative keys are cleared by
/// `validateBoardLinks` during upstream compilation.
fn node_link_after_board_validation(
    compiler: &Compiler,
    root: IrNodeId,
    field: IrNodeId,
) -> Option<String> {
    let value = property_text_inherited(compiler, field, &["link"])?;
    if super::import_urls::is_remote(value.as_bytes()) {
        return Some(value);
    }
    let Ok(key) = crate::parse_key(value.as_bytes()) else {
        // The Go compiler leaves a link alone when ParseKey rejects it.
        return Some(value);
    };
    let Some(parts) = key.string_ida() else {
        return Some(value);
    };
    if parts.first().map(String::as_str) != Some("root") {
        return None;
    }
    let components = key
        .path
        .as_ref()
        .expect("concrete board key")
        .iter()
        .skip(1)
        .map(|slot| {
            slot.read()
                .as_ref()
                .expect("board component")
                .read()
                .clone()
        })
        .collect::<Vec<_>>();
    let target = String::from_utf8(crate::format::format_key_path(&components))
        .expect("formatted board target is UTF-8");
    let mut document_root = root;
    while !compiler.arena.root(document_root) {
        let Some(parent) = compiler.arena.parent_map(document_root) else {
            break;
        };
        document_root = parent;
    }
    match super::boards::resolve_board_target(compiler, document_root, &target) {
        Ok(_) if parts != go_graph_ida(compiler, document_root, root) => Some(value),
        _ => None,
    }
}

/// d2graph.Graph.IDA (d2graph.go:94–151), which validateBoardLinks compares
/// against a link's StringIDA to drop self links. It joins `root`, only the
/// innermost board collection name, then every board name from the top: for
/// `layers.x.steps.next` it is `root.steps.x.next`. Nested self links
/// therefore survive, as they do upstream.
fn go_graph_ida(compiler: &Compiler, document_root: IrNodeId, board: IrNodeId) -> Vec<String> {
    let Some(target) = super::boards::board_targets(compiler, document_root)
        .into_iter()
        .find_map(|(target, node)| (node == board).then_some(target))
        .filter(|target| !target.is_empty())
    else {
        return vec!["root".to_owned()];
    };
    let components = crate::parse_key(target.as_bytes())
        .ok()
        .and_then(|key| key.string_ida())
        .unwrap_or_default();
    let mut ida = vec!["root".to_owned()];
    if components.len() >= 2 {
        ida.push(components[components.len() - 2].clone());
    }
    ida.extend(components.iter().skip(1).step_by(2).cloned());
    ida
}

/// Return the display text D2 v0.9.0 derives for an internal board link.
/// External links keep their source URL as the target's PrettyLink fallback;
/// internal links instead display board labels (or board names), omitting the
/// wrappers such as `layers` and `scenarios`.
fn node_pretty_link_after_board_validation(
    compiler: &Compiler,
    root: IrNodeId,
    field: IrNodeId,
) -> Option<ast::ByteString> {
    let link = node_link_after_board_validation(compiler, root, field)?;
    if super::import_urls::is_remote(link.as_bytes()) {
        return super::import_urls::shortened_url_label(link.as_bytes()).map(ast::ByteString::from);
    }
    let Ok(key) = crate::parse_key(link.as_bytes()) else {
        return None;
    };
    let Some(parts) = key.string_ida() else {
        return None;
    };
    if parts.first().map(String::as_str) != Some("root") {
        return None;
    }

    let mut document_root = root;
    while !compiler.arena.root(document_root) {
        let Some(parent) = compiler.arena.parent_map(document_root) else {
            break;
        };
        document_root = parent;
    }

    // d2exporter.toPrettyLink starts with the root board's name/label. The
    // root graph has no name unless a label was declared, so this is often an
    // empty first component; Go then falls back to the last component below.
    let mut labels = vec![board_label(compiler, document_root).unwrap_or_default()];
    let target_parts = &parts[1..];
    if !target_parts.is_empty() {
        for (board_path, board) in super::boards::board_targets(compiler, document_root) {
            let board_parts = crate::parse_key(board_path.as_bytes())
                .expect("compiled board key")
                .string_ida()
                .expect("concrete board key");
            if board_parts.len() <= target_parts.len()
                && target_parts[..board_parts.len()] == board_parts[..]
            {
                labels.push(
                    board_label(compiler, board)
                        .unwrap_or_else(|| board_parts.last().cloned().unwrap_or_default()),
                );
            }
        }
    }

    if labels.iter().any(String::is_empty) {
        labels.last().cloned().map(ast::ByteString::from)
    } else {
        Some(ast::ByteString::from(labels.join(" > ")))
    }
}

fn property_number(compiler: &Compiler, node: IrNodeId, path: &[&str]) -> Option<f64> {
    property_text(compiler, node, path)?.parse().ok()
}

/// Returns the source range of a direct property key. D2's grid adapter uses
/// the key ranges (rather than the resolved values) to retain whether rows or
/// columns was declared first.
fn property_key_range_in_map(
    compiler: &Compiler,
    map: IrNodeId,
    path: &[&str],
) -> Option<ast::Range> {
    let field = compiler.graph_field(map, path)?;
    let IrNode::Field(field) = compiler.arena.node(field) else {
        return None;
    };
    let key = field.name.unbox()?;
    Some(ast::Node::range(&key))
}

fn property_key_range(compiler: &Compiler, node: IrNodeId, path: &[&str]) -> Option<ast::Range> {
    property_key_range_in_map(compiler, compiler.arena.map(node)?, path)
}

fn property_key_range_inherited(
    compiler: &Compiler,
    node: IrNodeId,
    path: &[&str],
) -> Option<ast::Range> {
    property_key_range(compiler, node, path).or_else(|| {
        class_maps(compiler, node)
            .iter()
            .rev()
            .find_map(|map| property_key_range_in_map(compiler, *map, path))
    })
}

fn grid_row_directed(compiler: &Compiler, node: IrNodeId) -> Option<bool> {
    let rows = property_key_range_inherited(compiler, node, &["grid-rows"])?;
    let columns = property_key_range_inherited(compiler, node, &["grid-columns"])?;
    Some(rows.before(&columns))
}

fn root_grid_count(compiler: &Compiler, root: IrNodeId, path: &[&str]) -> Option<usize> {
    let value = property_number(compiler, root, path)?;
    (value.is_finite() && value >= 1.0)
        .then_some(value as usize)
        .filter(|value| *value > 0)
}

/// Return the class definitions which apply to a field, in the same order as
/// d2's compiler: inherited classes first, then the class itself.  A class
/// list is therefore resolved from right to left when looking up a property,
/// while an explicit field value always wins over every class value.
fn class_maps(compiler: &Compiler, node: IrNodeId) -> super::graph_fields::ClassMaps {
    let Some(map) = compiler.arena.map(node) else {
        return super::graph_fields::ClassMaps::default();
    };
    // Match compileBoard's independent class namespace. Parent navigation is
    // insufficient for substituted composites and quoted board-keyword nodes.
    let root = compiler
        .graph_class_root
        .unwrap_or_else(|| compiler.arena.root_map(map));
    class_maps_at_root(compiler, node, root)
}

pub(super) enum ClassApplication {
    Map(IrNodeId),
    Error(ast::Error),
}

pub(super) fn class_maps_at_root(
    compiler: &Compiler,
    node: IrNodeId,
    root: IrNodeId,
) -> super::graph_fields::ClassMaps {
    compiler.session_class_maps(node, root, || {
        class_applications_at_root(compiler, node, root, false, &mut |_| true)
            .into_iter()
            .filter_map(|event| match event {
                ClassApplication::Map(map) => Some(map),
                ClassApplication::Error(_) => None,
            })
            .collect()
    })
}

pub(super) fn class_names(
    compiler: &Compiler,
    map: IrNodeId,
) -> Option<(IrNodeId, Vec<Vec<u8>>, Vec<IrNodeId>)> {
    let field = compiler.graph_field(map, &["class"])?;
    let mut names = Vec::new();
    let mut invalid = Vec::new();
    if let Some(primary) = compiler.arena.primary(field) {
        names.push(crate::ir::format(&compiler.arena, primary));
    } else if let IrNode::Field(holder) = compiler.arena.node(field)
        && let Some(composite) = holder.composite
        && let IrNode::Array(array) = compiler.arena.node(composite)
    {
        for value in array.values.as_deref().unwrap_or_default() {
            if let IrNode::Scalar(scalar) = compiler.arena.node(*value) {
                names.push(scalar.value.scalar_bytes());
            } else {
                invalid.push(*value);
            }
        }
    }
    Some((field, names, invalid))
}

fn target_class_names(compiler: &Compiler, node: IrNodeId) -> Vec<String> {
    let Some(map) = compiler.arena.map(node) else {
        return Vec::new();
    };
    let Some(field) = compiler.graph_field(map, &["class"]) else {
        return Vec::new();
    };
    if let Some(primary) = compiler.arena.primary(field) {
        if let IrNode::Scalar(value) = compiler.arena.node(primary) {
            return vec![scalar_text(&value.value)];
        }
    }
    if let IrNode::Field(holder) = compiler.arena.node(field) {
        if let Some(composite) = holder.composite {
            if let IrNode::Array(array) = compiler.arena.node(composite) {
                return array
                    .values
                    .as_deref()
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|value| match compiler.arena.node(*value) {
                        IrNode::Scalar(value) => Some(scalar_text(&value.value)),
                        _ => None,
                    })
                    .collect();
            }
        }
    }
    Vec::new()
}

/// Source compileMap/compileEdgeMap apply each class occurrence; beginClass
/// detects only an active recursion path, not previous completed applications.
pub(super) fn class_applications_at_root(
    compiler: &Compiler,
    node: IrNodeId,
    root: IrNodeId,
    edge: bool,
    reserve_copy: &mut dyn FnMut(IrNodeId) -> bool,
) -> Vec<ClassApplication> {
    let Some(map) = compiler.arena.map(node) else {
        return Vec::new();
    };
    fn lookup(compiler: &Compiler, root: IrNodeId, name: &[u8]) -> Option<IrNodeId> {
        compiler
            .graph_class_index
            .borrow_mut()
            .get(&compiler.arena, root, name, || {
                compiler.graph_context_cause()
            })
            .ok()
            .flatten()
    }
    fn trim_space(mut bytes: &[u8]) -> &[u8] {
        // Go strings.TrimSpace uses Unicode White_Space. Keep invalid bytes
        // intact while examining each complete leading/trailing UTF8 rune.
        let space = |part: &[u8]| {
            std::str::from_utf8(part).ok().is_some_and(|s| {
                let mut chars = s.chars();
                chars.next().is_some_and(char::is_whitespace) && chars.next().is_none()
            })
        };
        while let Some(width) = (1..=4).find(|width| bytes.get(..*width).is_some_and(space)) {
            bytes = &bytes[width..];
        }
        while let Some(width) = (1..=4).find(|width| {
            bytes
                .len()
                .checked_sub(*width)
                .is_some_and(|start| space(&bytes[start..]))
        }) {
            bytes = &bytes[..bytes.len() - width];
        }
        bytes
    }
    fn ref_error(
        compiler: &Compiler,
        field: IrNodeId,
        message: String,
    ) -> Option<ClassApplication> {
        compiler
            .arena
            .last_ref(field)
            .and_then(|reference| reference.ast())
            .map(|node| ClassApplication::Error(super::edge_creation::error(&node, message)))
    }
    fn visit(
        compiler: &Compiler,
        map: IrNodeId,
        root: IrNodeId,
        edge: bool,
        active: &mut HashSet<IrNodeId>,
        output: &mut Vec<ClassApplication>,
        reserve_copy: &mut dyn FnMut(IrNodeId) -> bool,
    ) {
        let Some((field, names, invalid)) = class_names(compiler, map) else {
            return;
        };
        for value in invalid {
            if let Some(key) = compiler.arena.last_primary_key(value) {
                output.push(ClassApplication::Error(super::edge_creation::error(
                    &ast::Child::Key(key),
                    "invalid value in array",
                )));
            }
        }
        for name in names {
            if compiler.graph_copy_failed.get() {
                return;
            }
            if let Some(class) = lookup(compiler, root, &name) {
                if active.insert(class) {
                    // Go reserves before descending into the class. Charging
                    // a flattened event list later would expand an alias DAG
                    // without a bound before the budget ever sees it.
                    if !reserve_copy(class) {
                        return;
                    }
                    visit(compiler, class, root, edge, active, output, reserve_copy);
                    if compiler.graph_copy_failed.get() {
                        return;
                    }
                    output.push(ClassApplication::Map(class));
                    active.remove(&class);
                } else if let Some(event) = ref_error(
                    compiler,
                    field,
                    format!(
                        "class {} forms a reference cycle",
                        crate::parser::quote_input(&name)
                    ),
                ) {
                    output.push(event);
                }
            } else if !edge
                && name.contains(&b',')
                && name
                    .split(|byte| *byte == b',')
                    .all(|part| lookup(compiler, root, trim_space(part)).is_some())
                && let Some(event) = ref_error(
                    compiler,
                    field,
                    format!(
                        "class \"{}\" not found. Did you mean to use \";\" to separate array items?",
                        String::from_utf8_lossy(&name)
                    ),
                )
            {
                output.push(event);
            }
        }
    }
    let mut active = HashSet::new();
    let mut output = Vec::new();
    visit(
        compiler,
        map,
        root,
        edge,
        &mut active,
        &mut output,
        reserve_copy,
    );
    output
}

fn property_scalar_inherited(
    compiler: &Compiler,
    node: IrNodeId,
    path: &[&str],
) -> Option<ScalarBox> {
    if let Some(scalar) = compiler.graph_scalar_inherited(node, path, || class_maps(compiler, node))
    {
        return scalar.map(|scalar| match compiler.arena.node(scalar) {
            IrNode::Scalar(value) => value.value.clone(),
            _ => unreachable!("resolved scalar"),
        });
    }
    property_scalar(compiler, node, path).or_else(|| {
        class_maps(compiler, node)
            .iter()
            .rev()
            .find_map(|map| property_scalar_in_map(compiler, *map, path))
    })
}

/// Preserve compileMap's near assignments, including malformed class values
/// which report an error even when a later direct value overrides them.
fn collect_near_assignments(
    compiler: &Compiler,
    map: IrNodeId,
    prefix: Option<&str>,
    owner: Option<GraphNodeId>,
    nodes: &BTreeMap<String, GraphNodeId>,
    include_classes: bool,
    output: &mut Vec<(Option<GraphNodeId>, ScalarBox)>,
) {
    enum Frame {
        Map(IrNodeId, Option<String>, Option<GraphNodeId>, bool),
        Fields(IrNodeId, usize, Option<String>, Option<GraphNodeId>),
    }
    let mut frames = vec![Frame::Map(
        map,
        prefix.map(str::to_owned),
        owner,
        include_classes,
    )];
    while let Some(frame) = frames.pop() {
        let (map, index, prefix, owner) = match frame {
            Frame::Map(map, prefix, owner, include_classes) => {
                frames.push(Frame::Fields(map, 0, prefix.clone(), owner));
                if include_classes {
                    for class in class_maps(compiler, map).iter().rev() {
                        frames.push(Frame::Map(*class, prefix.clone(), owner, false));
                    }
                }
                continue;
            }
            Frame::Fields(map, index, prefix, owner) => (map, index, prefix, owner),
        };
        let IrNode::Map(map_node) = compiler.arena.node(map) else {
            continue;
        };
        let Some(&field) = map_node.fields.as_deref().unwrap_or_default().get(index) else {
            continue;
        };
        frames.push(Frame::Fields(map, index + 1, prefix.clone(), owner));
        let IrNode::Field(field_node) = compiler.arena.node(field) else {
            continue;
        };
        let name = field_node.name.scalar_string();
        if name == "near" && field_node.name.unquoted_string.is_some() {
            if let Some(primary) = compiler.arena.primary(field)
                && let IrNode::Scalar(scalar) = compiler.arena.node(primary)
            {
                output.push((owner, scalar.value.clone()));
            }
            continue;
        }
        if is_metadata_field(compiler, field) || is_board_wrapper(compiler, field, &name) {
            continue;
        }
        if let Some(child_map) = compiler.arena.map(field)
            && let Some(component) = path_component(compiler, field)
        {
            let path = append_path(prefix.as_deref(), &component);
            let child_owner = nodes.get(&path).copied();
            frames.push(Frame::Map(child_map, Some(path), child_owner, true));
        }
    }
}

// compileReserved only compiles icon style alongside that map's scalar icon.
// A later map containing only icon.style does not update prior class icon style.
fn source_icon_border_radius(compiler: &Compiler, node: IrNodeId) -> Option<String> {
    let mut radius = None;
    for map in class_maps(compiler, node)
        .iter()
        .copied()
        .chain(compiler.arena.map(node))
    {
        if property_scalar_in_map(compiler, map, &["icon"]).is_some() {
            if let Some(value) =
                property_scalar_in_map(compiler, map, &["icon", "style", "border-radius"])
            {
                radius = Some(scalar_text(&value));
            }
        }
    }
    radius
}

fn property_text_inherited(compiler: &Compiler, node: IrNodeId, path: &[&str]) -> Option<String> {
    property_scalar_inherited(compiler, node, path).map(|value| scalar_text(&value))
}

pub(super) fn normalize_language(value: &str) -> String {
    match value {
        "md" => "markdown".into(),
        "tex" => "latex".into(),
        "js" => "javascript".into(),
        "go" => "golang".into(),
        "py" => "python".into(),
        "rb" => "ruby".into(),
        "ts" => "typescript".into(),
        value => value.into(),
    }
}

/// Returns the effective label language using D2's explicit attribute first,
/// then the block-string tag carried by the label or primary value.
pub(super) fn field_language(compiler: &Compiler, field: IrNodeId) -> Option<String> {
    if let Some(language) = property_text_inherited(compiler, field, &["language"]) {
        return Some(normalize_language(&language));
    }
    let block_language = |value: &ScalarBox| {
        value
            .block_string
            .as_ref()
            .map(|block| normalize_language(&block.read().tag))
    };
    let mut language =
        compiler
            .arena
            .primary(field)
            .and_then(|primary| match compiler.arena.node(primary) {
                IrNode::Scalar(scalar) => block_language(&scalar.value),
                _ => None,
            });
    let edge = matches!(compiler.arena.node(field), IrNode::Edge(_));
    let class_maps = class_maps(compiler, field);
    let maps = class_maps.iter().copied().chain(compiler.arena.map(field));
    for map in maps {
        let IrNode::Map(scope) = compiler.arena.node(map) else {
            continue;
        };
        let fields = scope.fields.as_deref().unwrap_or_default();
        // Object compileMap moves shape ahead of every other field. Edge
        // compileEdgeMap retains field order, including shape/code resets.
        let ordered = if edge {
            fields.to_vec()
        } else {
            fields.iter().filter(|field| matches!(compiler.arena.node(**field), IrNode::Field(holder) if holder.name.unquoted_string.is_some() && holder.name.scalar_string()=="shape"))
                .chain(fields.iter().filter(|field| !matches!(compiler.arena.node(**field), IrNode::Field(holder) if holder.name.unquoted_string.is_some() && holder.name.scalar_string()=="shape")))
                .copied().collect()
        };
        for field in ordered {
            let IrNode::Field(holder) = compiler.arena.node(field) else {
                continue;
            };
            if !holder.name.unquoted_string.is_some() {
                continue;
            }
            let Some(primary) = compiler.arena.primary(field) else {
                continue;
            };
            let IrNode::Scalar(scalar) = compiler.arena.node(primary) else {
                continue;
            };
            match holder.name.scalar_string().as_str() {
                "shape" if scalar_text(&scalar.value).eq_ignore_ascii_case("code") => {
                    language = Some("text".into())
                }
                "label" => {
                    if let Some(tag) = block_language(&scalar.value) {
                        language = Some(tag);
                    }
                }
                _ => {}
            }
        }
    }
    language
}

fn property_number_inherited(compiler: &Compiler, node: IrNodeId, path: &[&str]) -> Option<f64> {
    property_text_inherited(compiler, node, path)?.parse().ok()
}

fn property_is_true_inherited(compiler: &Compiler, node: IrNodeId, path: &[&str]) -> bool {
    property_text_inherited(compiler, node, path)
        .is_some_and(|value| d2_rs_core::parse_d2_bool(&value) == Some(true))
}

fn source_node_is_invisible(compiler: &Compiler, field: IrNodeId, has_icon: bool) -> bool {
    if property_text_inherited(compiler, field, &["style", "opacity"])
        .and_then(|value| d2_rs_core::parse_d2_float(&value))
        .is_some_and(|value| value == 0.0)
    {
        return true;
    }
    let no_fill = property_text_inherited(compiler, field, &["style", "fill"])
        .is_some_and(|value| value.eq_ignore_ascii_case("transparent"));
    let no_stroke_width = property_number_inherited(compiler, field, &["style", "stroke-width"])
        .is_some_and(|value| value == 0.0);
    let transparent_stroke = property_text_inherited(compiler, field, &["style", "stroke"])
        .is_some_and(|value| value.eq_ignore_ascii_case("transparent"));
    let no_label =
        property_text_inherited(compiler, field, &["label"]).is_none_or(|value| value.is_empty());
    no_fill && (no_stroke_width || transparent_stroke) && no_label && !has_icon
}

fn default_shape(language: Option<&str>, sequence_ancestor: bool) -> ShapeKind {
    if sequence_ancestor {
        ShapeKind::Rectangle
    } else {
        match language {
            Some("markdown" | "latex") => ShapeKind::Text,
            Some(_) => ShapeKind::Code,
            None => ShapeKind::Rectangle,
        }
    }
}

fn parse_shape(value: Option<&str>) -> ShapeKind {
    match value {
        // D2's public `square` selects the unit-aspect RealSquare shape;
        // `Square` in GraphJSON is the ordinary rectangle carrier.
        Some("square") => ShapeKind::RealSquare,
        Some("real_square") => ShapeKind::RealSquare,
        Some("parallelogram") => ShapeKind::Parallelogram,
        Some("document") => ShapeKind::Document,
        Some("cylinder") => ShapeKind::Cylinder,
        Some("queue") => ShapeKind::Queue,
        Some("page") => ShapeKind::Page,
        Some("package") => ShapeKind::Package,
        Some("step") => ShapeKind::Step,
        Some("callout") => ShapeKind::Callout,
        Some("stored_data") => ShapeKind::StoredData,
        Some("person") => ShapeKind::Person,
        // `c4-person` is the public shape name used by D2 v0.9.0. Keep the
        // underscore spelling as a compatibility alias for older callers,
        // but always lower both forms to the same graph shape.
        Some("c4-person" | "c4_person") => ShapeKind::C4Person,
        Some("diamond") => ShapeKind::Diamond,
        Some("oval") => ShapeKind::Oval,
        Some("circle") => ShapeKind::Circle,
        Some("hexagon") => ShapeKind::Hexagon,
        Some("cloud") => ShapeKind::Cloud,
        Some("sql_table") => ShapeKind::SqlTable,
        Some("class") => ShapeKind::Class,
        Some("text") => ShapeKind::Text,
        Some("code") => ShapeKind::Code,
        Some("image") => ShapeKind::Image,
        _ => ShapeKind::Rectangle,
    }
}

#[cfg(test)]
mod shape_tests {
    use super::parse_shape;
    use d2_rs_core::ShapeKind;

    #[test]
    fn accepts_public_c4_person_shape_name() {
        assert_eq!(parse_shape(Some("c4-person")), ShapeKind::C4Person);
        assert_eq!(parse_shape(Some("c4_person")), ShapeKind::C4Person);
    }
}

#[cfg(test)]
mod arrowhead_graph_tests {
    use super::lower_graph;
    use crate::{compiler::compile_ir, parse};
    use d2_rs_core::ast::Shared;

    #[test]
    fn native_edge_label_percentage_retains_upstream_zero_default() {
        let parsed = parse("label-percentage.d2", b"a -> b: automatic", None);
        assert!(parsed.error.is_none(), "{:?}", parsed.error);
        let (compiler, root) = compile_ir(Shared::new(parsed.map)).unwrap();
        let graph = lower_graph(&compiler, root).unwrap();
        assert!(!graph.edge_label_percentage_present(d2_rs_core::EdgeId(0)));
        assert_eq!(
            graph.edge_label(d2_rs_core::EdgeId(0)).unwrap().percentage,
            0.0
        );
        // Explicit label-percentage is not in the pinned keyword inventory.
        // The original three-edge input is retained in the pristine complete
        // diagnostic fixture, which asserts rejection of both explicit fields.
    }

    #[test]
    fn native_graph_allocates_adapter_ids_for_scopes_repeats_and_collisions() {
        let parsed = parse(
            "identity.d2",
            b"costarring
liquid
group: {\"a.b\" -> target; target <- \"a.b\"}
",
            None,
        );
        assert!(parsed.error.is_none(), "{:?}", parsed.error);
        let (compiler, root) = compile_ir(Shared::new(parsed.map)).unwrap();
        let graph = lower_graph(&compiler, root).unwrap();
        let collision_ids = graph
            .nodes()
            .filter(|(_, node)| node.external_id == "costarring" || node.external_id == "liquid")
            .map(|(id, _)| graph.tala_id_overrides[id.0 as usize].unwrap())
            .collect::<Vec<_>>();
        assert_eq!(collision_ids, vec![1i64 << 32, (1i64 << 32) + 1]);
        let identities = graph
            .edges()
            .map(|(id, _)| d2_rs_core::edge_abs_id(&graph, id).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            identities,
            vec![
                "group.(\"a.b\" -> target)[0]",
                "group.(target <- \"a.b\")[1]"
            ]
        );
        let allocated = d2_rs_core::identity::allocate(
            &identities
                .iter()
                .map(|name| name.as_bytes())
                .collect::<Vec<_>>(),
        )
        .unwrap();
        assert_eq!(
            graph.tala_edge_ids,
            allocated.into_iter().map(Some).collect::<Vec<_>>()
        );
        assert!(
            graph
                .tala_edge_ids
                .iter()
                .all(|id| id.is_some_and(|id| id != 0))
        );
    }

    #[test]
    fn shorthand_arrowhead_labels_and_colors_survive_graph_lowering() {
        let source = b"direction: right\nx <-> y: {\n  source-arrowhead: eth1 {\n    style.font-color: green\n  }\n  target-arrowhead: eth2 {\n    style.font-color: red\n  }\n}\n";
        let parsed = parse("arrowhead-label.d2", source, None);
        assert!(parsed.error.is_none(), "{:?}", parsed.error);
        let (compiler, root) = compile_ir(Shared::new(parsed.map)).unwrap();
        assert!(compiler.errors.is_empty(), "{:?}", compiler.errors);
        let graph = lower_graph(&compiler, root).unwrap();
        let (edge_id, _) = graph.edges().next().unwrap();
        let labels = graph.edge_arrowhead_labels(edge_id);
        assert_eq!(
            labels.source.as_ref().map(|label| label.text.as_str()),
            Some("eth1")
        );
        assert_eq!(
            labels
                .source
                .as_ref()
                .and_then(|label| label.color.as_deref()),
            Some("green")
        );
        assert_eq!(
            labels.target.as_ref().map(|label| label.text.as_str()),
            Some("eth2")
        );
        assert_eq!(
            labels
                .target
                .as_ref()
                .and_then(|label| label.color.as_deref()),
            Some("red")
        );
    }

    #[test]
    fn filled_arrowhead_styles_lower_to_tala_arrowhead_identities() {
        let source = b"direction: right\na <-> b: {\n  source-arrowhead.shape: diamond\n  source-arrowhead.style.filled: true\n  target-arrowhead.shape: diamond\n  target-arrowhead.style.filled: false\n}\n";
        let parsed = parse("arrowhead-filled.d2", source, None);
        assert!(parsed.error.is_none(), "{:?}", parsed.error);
        let (compiler, root) = compile_ir(Shared::new(parsed.map)).unwrap();
        assert!(compiler.errors.is_empty(), "{:?}", compiler.errors);
        let graph = lower_graph(&compiler, root).unwrap();
        let (edge_id, _) = graph.edges().next().unwrap();
        assert_eq!(
            graph.edge_arrowheads(edge_id),
            d2_rs_core::EdgeArrowheads {
                source: Some("filled-diamond".into()),
                target: Some("diamond".into()),
            }
        );
    }

    #[test]
    fn arrowhead_lowering_preserves_d2_shape_case_and_boolean_spellings() {
        for (shape, filled, expected) in [
            ("DIAMOND", "1", "filled-diamond"),
            ("DIAMOND", "t", "filled-diamond"),
            ("DIAMOND", "T", "filled-diamond"),
            ("DIAMOND", "TRUE", "filled-diamond"),
            ("DIAMOND", "true", "filled-diamond"),
            ("DIAMOND", "True", "filled-diamond"),
            ("TRIANGLE", "0", "unfilled-triangle"),
            ("TRIANGLE", "f", "unfilled-triangle"),
            ("TRIANGLE", "F", "unfilled-triangle"),
            ("TRIANGLE", "FALSE", "unfilled-triangle"),
            ("TRIANGLE", "false", "unfilled-triangle"),
            ("TRIANGLE", "False", "unfilled-triangle"),
            ("NONE", "TRUE", "none"),
            ("cİrcle", "true", "filled-circle"),
            ("cİrcle", "false", "circle"),
            ("dİamond", "true", "filled-diamond"),
            ("trİangle", "false", "unfilled-triangle"),
        ] {
            // Quotes preserve spelling rather than the AST boolean scalar's
            // canonical true/false serialization.
            let source = format!(
                "a -> b: {{source-arrowhead.shape: {shape}; source-arrowhead.style.filled: \"{filled}\"}}\n"
            );
            let parsed = parse("arrowhead-spellings.d2", source.as_bytes(), None);
            assert!(parsed.error.is_none(), "{:?}", parsed.error);
            let (compiler, root) = compile_ir(Shared::new(parsed.map)).unwrap();
            assert!(compiler.errors.is_empty(), "{:?}", compiler.errors);
            let graph = lower_graph(&compiler, root).unwrap();
            let (edge_id, _) = graph.edges().next().unwrap();
            assert_eq!(
                graph.edge_arrowheads(edge_id).source.as_deref(),
                Some(expected),
                "{shape}/{filled}"
            );
        }
    }

    #[test]
    fn compiled_arrowhead_declarations_retain_fallback_shapes_and_go_unicode_lowercase() {
        for (source, expected_source, expected_target, resolved_target) in [
            (
                "a -> b: {source-arrowhead.shape: rectangle; target-arrowhead.shape: ſquare}",
                Some("rectangle"),
                Some("ſquare"),
                "triangle",
            ),
            (
                "classes: {head: {target-arrowhead.shape: rectangle}}\na -> b: {class: head; target-arrowhead.shape: cİrcle}",
                None,
                Some("circle"),
                "circle",
            ),
            (
                "a -> b: {target-arrowhead: label {shape: \"\"}}",
                None,
                Some(""),
                "triangle",
            ),
        ] {
            let parsed = parse("arrowhead-declarations.d2", source.as_bytes(), None);
            assert!(parsed.error.is_none(), "{:?}", parsed.error);
            let (compiler, root) = compile_ir(Shared::new(parsed.map)).unwrap();
            assert!(compiler.errors.is_empty(), "{:?}", compiler.errors);
            let graph = lower_graph(&compiler, root).unwrap();
            let (edge_id, _) = graph.edges().next().unwrap();
            let declarations = graph.edge_arrowhead_declarations(edge_id);
            assert_eq!(declarations.source.as_deref(), expected_source);
            assert_eq!(declarations.target.as_deref(), expected_target);
            assert_eq!(
                graph.edge_arrowheads(edge_id).target.as_deref(),
                Some(resolved_target)
            );
        }
    }
}

#[cfg(test)]
mod inherited_link_tooltip_tests {
    use super::lower_graph_with_measurer_and_labels_theme;
    use crate::{compiler::compile_ir, parse};
    use d2_rs_core::{Size, ast::Shared};

    fn fixed_measurement(_: &str) -> Size {
        Size {
            width: 20.0,
            height: 20.0,
        }
    }

    #[test]
    fn hex_zero_opacity_preserves_scalar_and_removes_transparent_legend_entries() {
        let source = b"a: {style.opacity: 0x0p0}\nvars: {d2-legend: {hidden: {style.opacity: 0x0p0}; visible: {style.opacity: 0x1p-1}}}\n";
        let parsed = parse("opacity.d2", source, None);
        assert!(parsed.error.is_none(), "{:?}", parsed.error);
        let (compiler, root) = compile_ir(Shared::new(parsed.map)).unwrap();
        let (graph, _, styles) =
            lower_graph_with_measurer_and_labels_theme(&compiler, root, &fixed_measurement, false)
                .unwrap();
        let (node_id, _) = graph
            .nodes()
            .find(|(_, node)| node.external_id == "a")
            .unwrap();
        assert!(graph.node_is_invisible(node_id));
        assert_eq!(styles[&node_id].opacity.as_deref(), Some("0x0p0"));
        let legend = graph.legend.as_ref().unwrap();
        assert_eq!(legend.shapes.len(), 1);
        assert_eq!(legend.shapes[0].label, "visible");
    }

    #[test]
    fn applied_font_normalizes_without_normalizing_pattern_or_transform_values() {
        let source = b"classes: {named: {style.font: MONO}}\na: {class: named; style.fill-pattern: DOTS; style.text-transform: NONE}\na -> b: \"MiXeD Words\" {style.font: DEFAULT; style.text-transform: UPPERCASE}\n";
        let parsed = parse("style-font.d2", source, None);
        assert!(parsed.error.is_none(), "{:?}", parsed.error);
        let (compiler, root) = compile_ir(Shared::new(parsed.map)).unwrap();
        let (graph, _, styles) =
            lower_graph_with_measurer_and_labels_theme(&compiler, root, &fixed_measurement, false)
                .unwrap();
        let (node_id, _) = graph
            .nodes()
            .find(|(_, node)| node.external_id == "a")
            .unwrap();
        let style = &styles[&node_id];
        assert_eq!(style.font.as_deref(), Some("mono"));
        assert_eq!(style.fill_pattern.as_deref(), Some("DOTS"));
        assert_eq!(style.text_transform.as_deref(), Some("NONE"));
        let edge_style = graph.edge_style(d2_rs_core::EdgeId(0));
        assert_eq!(edge_style.font.as_deref(), Some("default"));
        assert_eq!(edge_style.text_transform.as_deref(), Some("UPPERCASE"));
        assert_eq!(
            graph.edge_label(d2_rs_core::EdgeId(0)).unwrap().text,
            "MiXeD Words"
        );
    }

    #[test]
    fn class_links_and_tooltips_reach_the_render_style_map() {
        let source = b"classes: {\n  linked.link: https://example.com\n  tooltipped.tooltip: example\n}\nitem: {class: [linked; tooltipped]}\n";
        let parsed = parse("class-links.d2", source, None);
        assert!(parsed.error.is_none(), "{:?}", parsed.error);
        let (compiler, root) = compile_ir(Shared::new(parsed.map)).unwrap();
        assert!(compiler.errors.is_empty(), "{:?}", compiler.errors);
        let (graph, _, styles) =
            lower_graph_with_measurer_and_labels_theme(&compiler, root, &fixed_measurement, false)
                .unwrap();
        let (node_id, _) = graph
            .nodes()
            .find(|(_, node)| node.external_id == "item")
            .expect("classed object is lowered");
        let style = &styles[&node_id];
        assert_eq!(style.link.as_deref(), Some("https://example.com"));
        assert_eq!(style.tooltip.as_deref(), Some("example"));
    }

    #[test]
    fn board_links_are_kept_only_when_they_target_a_different_board() {
        let source = b"cat: {link: layers.cat}\nlayers: {cat: {home: {link: _}; same: {link: _.layers.cat}}}\n";
        let parsed = parse("board-links.d2", source, None);
        assert!(parsed.error.is_none(), "{:?}", parsed.error);
        let (compiler, root) = compile_ir(Shared::new(parsed.map)).unwrap();
        assert!(compiler.errors.is_empty(), "{:?}", compiler.errors);

        let (root_graph, _, root_styles) =
            lower_graph_with_measurer_and_labels_theme(&compiler, root, &fixed_measurement, false)
                .unwrap();
        let (cat_id, _) = root_graph
            .nodes()
            .find(|(_, node)| node.external_id == "cat")
            .expect("root-board link source is lowered");
        assert_eq!(
            root_styles[&cat_id].link.as_deref(),
            Some("root.layers.cat")
        );
        assert_eq!(root_styles[&cat_id].pretty_link.as_deref(), Some("cat"));

        let cat_board = crate::compiler::resolve_board_target(&compiler, root, "layers.cat")
            .expect("linked layer exists");
        let (layer_graph, _, layer_styles) = lower_graph_with_measurer_and_labels_theme(
            &compiler,
            cat_board,
            &fixed_measurement,
            false,
        )
        .unwrap();
        let (home_id, _) = layer_graph
            .nodes()
            .find(|(_, node)| node.external_id == "home")
            .expect("root-linked layer object is lowered");
        assert_eq!(layer_styles[&home_id].link.as_deref(), Some("root"));
        assert_eq!(layer_styles[&home_id].pretty_link.as_deref(), Some(""));

        let (same_id, _) = layer_graph
            .nodes()
            .find(|(_, node)| node.external_id == "same")
            .expect("self-linked layer object is lowered");
        assert_eq!(layer_styles[&same_id].link, None);
    }

    #[test]
    fn nested_board_self_links_survive_like_go_graph_ida() {
        // Reduced from upstream d2svg appendix `internal-links`. Go compares
        // a link with Graph.IDA, which is `root.steps.x.next` for this step,
        // so the step's own `root.layers.x.steps.next` link is kept.
        let source = b"layers: {x: {next.link: steps.next; steps: {next: {hi}}}}\n";
        let parsed = parse("nested-self-link.d2", source, None);
        assert!(parsed.error.is_none(), "{:?}", parsed.error);
        let (compiler, root) = compile_ir(Shared::new(parsed.map)).unwrap();
        assert!(compiler.errors.is_empty(), "{:?}", compiler.errors);
        let step = crate::compiler::resolve_board_target(&compiler, root, "layers.x.steps.next")
            .expect("step exists");
        let (graph, _, styles) =
            lower_graph_with_measurer_and_labels_theme(&compiler, step, &fixed_measurement, false)
                .unwrap();
        let (next_id, _) = graph
            .nodes()
            .find(|(_, node)| node.external_id == "next")
            .expect("inherited link source is lowered");
        assert_eq!(
            styles[&next_id].link.as_deref(),
            Some("root.layers.x.steps.next")
        );
    }
}

fn parse_direction(value: Option<&str>) -> Option<Direction> {
    match value {
        Some("right") => Some(Direction::Right),
        Some("left") => Some(Direction::Left),
        Some("up") => Some(Direction::Up),
        Some("down") => Some(Direction::Down),
        _ => None,
    }
}

#[derive(Clone)]
struct GraphField {
    field: IrNodeId,
    path: String,
    parent_path: Option<String>,
    source_order: Option<ast::Position>,
}

#[derive(Default)]
struct GraphChildren {
    // EnsureChild's sequence actor search uses declaration order and exact IDs.
    ordered: Vec<usize>,
    // d2graph.Object.Children uses strings.ToLower on the formatted key. This
    // table is queried only; graph order continues to come from the vectors.
    by_name: HashMap<Vec<u8>, usize>,
}

/// Translate d2graph.Object.EnsureChild's graph identity separately from the
/// lexical IR. Existing actors are resolved in compilation order; looking up
/// all actors in advance would incorrectly redirect earlier group children.
fn canonical_graph_fields(
    compiler: &Compiler,
    fields: Vec<GraphField>,
    root_sequence: bool,
) -> (
    Vec<GraphField>,
    BTreeMap<String, String>,
    Option<Compiler>,
    BTreeMap<String, usize>,
) {
    let mut canonical: Vec<GraphField> = Vec::new();
    let mut aliases: BTreeMap<String, String> = BTreeMap::new();
    let mut children: BTreeMap<Option<String>, GraphChildren> = BTreeMap::new();
    let mut sequences = HashSet::new();
    let mut property_view: Option<Compiler> = None;
    let mut copied = HashSet::new();
    let mut components: Vec<String> = Vec::new();
    let mut scope_stack = Vec::new();
    let mut scope_ends = BTreeMap::new();
    for mut field in fields {
        // compileMap connects edges after its complete child-map traversal,
        // before compiling the following sibling's fields. Retain that
        // boundary for EnsureChild's second actor lookup at Connect time.
        while scope_stack
            .last()
            .is_some_and(|scope| Some(scope) != field.parent_path.as_ref())
        {
            scope_ends.insert(scope_stack.pop().unwrap(), canonical.len());
        }
        let lexical_path = field.path.clone();
        let mut parent = field.parent_path.as_ref().map(|p| aliases[p].clone());
        let scalar = field_name(compiler, field.field).expect("graph field name");
        let component = path_component(compiler, field.field).expect("graph field component");
        // With no sequence in the compiled prefix, no ancestor can redirect
        // this child. Avoid scanning the whole canonical prefix at each depth.
        let sequence = if !root_sequence && sequences.is_empty() {
            None
        } else {
            // OuterSequenceDiagram starts at the object's parent, not itself.
            let mut ancestor = parent.as_ref().and_then(|p| {
                canonical
                    .iter()
                    .find(|f| &f.path == p)
                    .and_then(|f| f.parent_path.clone())
            });
            loop {
                match ancestor {
                    Some(ref path) if sequences.contains(path) => break Some(Some(path.clone())),
                    Some(ref path) => {
                        ancestor = canonical
                            .iter()
                            .find(|f| &f.path == path)
                            .and_then(|f| f.parent_path.clone())
                    }
                    None => break (root_sequence && parent.is_some()).then_some(None),
                }
            }
        };
        if let Some(sequence) = sequence {
            let actor_exists = children
                .get(&sequence)
                .into_iter()
                .flat_map(|children| &children.ordered)
                .any(|i| components[*i] == scalar);
            let same_name = parent
                .as_ref()
                .and_then(|p| canonical.iter().find(|f| &f.path == p))
                .and_then(|f| canonical.iter().position(|c| c.path == f.path))
                .is_some_and(|i| components[i] == scalar);
            if actor_exists && !same_name {
                parent = sequence;
            }
        }
        let folded = d2_rs_core::ir::go_lowercase(component.as_bytes());
        let existing = children
            .get(&parent)
            .and_then(|children| children.by_name.get(&folded))
            .copied();
        let index = if let Some(index) = existing {
            let view = property_view.get_or_insert_with(|| Compiler {
                arena: compiler.arena.fork_storage(),
                graph_class_root: compiler.graph_class_root,
                graph_class_index: compiler.graph_class_index.clone(),
                graph_context: compiler.graph_context.clone(),
                ..Default::default()
            });
            let base = canonical[index].field;
            // Copy the first field only once; original IDs stay available for
            // subsequent lexical declarations and source scope navigation.
            if copied.insert(index) {
                canonical[index].field = view.arena.copy(base, view.arena.parent(base));
            }
            view.arena
                .overlay_field(canonical[index].field, field.field, false);
            index
        } else {
            field.parent_path = parent.clone();
            field.path = append_path(parent.as_deref(), &component);
            let index = canonical.len();
            canonical.push(field.clone());
            components.push(component.clone());
            let children = children.entry(parent).or_default();
            children.ordered.push(index);
            children.by_name.insert(folded, index);
            index
        };
        // Shape declarations take effect before compiling the child map.
        if let Some(shape) = property_text_inherited(compiler, field.field, &["shape"]) {
            if shape == "sequence_diagram" {
                sequences.insert(canonical[index].path.clone());
            } else {
                sequences.remove(&canonical[index].path);
            }
        }
        if root_sequence || !sequences.is_empty() || !scope_stack.is_empty() {
            scope_stack.push(lexical_path.clone());
        }
        aliases.insert(lexical_path, canonical[index].path.clone());
    }
    for scope in scope_stack {
        scope_ends.insert(scope, canonical.len());
    }
    (canonical, aliases, property_view, scope_ends)
}

#[derive(Clone)]
struct CollectedGraphEdge {
    edge: IrNodeId,
    parent_path: Option<String>,
    validation_index: usize,
}

fn ordered_graph_edges(compiler: &Compiler, root: IrNodeId) -> Vec<CollectedGraphEdge> {
    let mut edges = Vec::new();
    collect_edges_at(compiler, root, None, &mut edges);
    // d2compiler.Compile sorts only the document graph after compileIR.
    // compileBoardsField calls compileBoard directly, so child boards keep
    // compileMap's child-map-before-parent-edge traversal order. In
    // particular, sequence message layout order and exported edge order are
    // separate contracts.
    if !compiler.arena.root(root) {
        return edges;
    }
    // Recursive collection visits child maps before their parent edge list.
    // Sequence layout, like D2's `getEdgeEarliestLineNum`, consumes messages
    // in source order instead of map traversal order.
    // Match Go's `sort.Slice` behavior, including its particular PDQsort
    // permutation for equal reference ranges. This matters for wildcard
    // expansions whose references intentionally share one source range.
    super::go_sort::sort_by(&mut edges, |left, right| {
        match (
            edge_reference_position(compiler, left.edge),
            edge_reference_position(compiler, right.edge),
        ) {
            (Some(left), Some(right)) => left.before(right),
            _ => false,
        }
    });
    edges
}

/// IR source identities in the same order as edges returned by graph lowering.
/// The second value is the original compileMap occurrence order, before the
/// AST sort; graph edge indices are assigned in that order. A substituted
/// composite may expose the same IR identity through multiple occurrences.
pub fn graph_edge_origins(compiler: &Compiler, root: IrNodeId) -> Vec<(IrNodeId, usize)> {
    ordered_graph_edges(compiler, root)
        .into_iter()
        .map(|edge| (edge.edge, edge.validation_index))
        .collect()
}

fn append_path(prefix: Option<&str>, name: &str) -> String {
    prefix.map_or_else(|| name.to_owned(), |prefix| format!("{prefix}.{name}"))
}

fn path_component(compiler: &Compiler, field: IrNodeId) -> Option<String> {
    let IrNode::Field(field) = compiler.arena.node(field) else {
        return None;
    };
    Some(
        String::from_utf8_lossy(&format::format_key_path(std::slice::from_ref(&field.name)))
            .into_owned(),
    )
}

fn is_board_wrapper(compiler: &Compiler, field: IrNodeId, name: &str) -> bool {
    let unquoted = matches!(compiler.arena.node(field), IrNode::Field(field) if field.name.unquoted_string.is_some());
    // NodeBoardKind is a navigation helper whose parent-name comparison is
    // intentionally quote-insensitive in Go. It cannot classify graph fields:
    // a child under quoted "layers" is still an ordinary object. The walk
    // already stops at actual unquoted board-wrapper fields themselves.
    unquoted && matches!(name, "layers" | "scenarios" | "steps")
}

/// Walk graph-visible fields with an explicit use-site prefix.
///
/// Composite substitutions intentionally share the resolved variable map in
/// the IR. Walking through `Arena::parent` therefore reports the definition's
/// `vars.*` path even when the same map is used below several concrete fields.
/// Carrying the prefix here preserves that sharing while presenting each
/// expansion at its actual graph path.
fn collect_fields_at(
    compiler: &Compiler,
    map: IrNodeId,
    prefix: Option<&str>,
    output: &mut Vec<GraphField>,
) -> Result<(), GraphCompileError> {
    compiler.graph_check()?;
    let IrNode::Map(map_node) = compiler.arena.node(map) else {
        return Ok(());
    };
    let mut frames = vec![(
        map_node.fields.clone().unwrap_or_default().into_iter(),
        prefix.map(str::to_owned),
    )];
    while let Some((fields, prefix)) = frames.last_mut() {
        let Some(field) = fields.next() else {
            frames.pop();
            continue;
        };
        compiler.graph_check()?;
        let Some(name) = field_name(compiler, field) else {
            continue;
        };
        if is_metadata_field(compiler, field) {
            continue;
        }
        let Some(path_name) = path_component(compiler, field) else {
            continue;
        };
        let path = append_path(prefix.as_deref(), &path_name);
        let board_wrapper = is_board_wrapper(compiler, field, &name);
        if !board_wrapper {
            output.push(GraphField {
                field,
                path: path.clone(),
                parent_path: prefix.clone(),
                source_order: object_source_position(compiler, field),
            });
        } else {
            // Boards are alternate graph roots.  The v0.9.0 compiler does not
            // flatten their contents into the ordinary root graph; selected
            // layer/scenario/step materialization happens at the board
            // orchestration stage.  Returning here keeps the base graph free
            // of every step's overlay (and, in particular, avoids emitting
            // `steps.*` objects in a normal layout pass).
            continue;
        }
        let content_shape =
            property_text_inherited(compiler, field, &["shape"]).map(|value| value.to_lowercase());
        if !matches!(content_shape.as_deref(), Some("class" | "sql_table"))
            && let Some(child_map) = compiler.arena.map(field)
        {
            compiler.graph_check()?;
            if let IrNode::Map(child) = compiler.arena.node(child_map) {
                frames.push((
                    child.fields.clone().unwrap_or_default().into_iter(),
                    Some(path),
                ));
            }
        }
    }
    Ok(())
}

/// D2 v0.9.0 calls `Graph.SortObjectsByAST` before constructing Dagro's
/// graph. Its stable order comes from the first object reference's selected
/// key-path component, not recursive object creation order. Preserve that
/// order separately from stable arena IDs so TALA's graph traversal remains
/// independent.
fn object_source_position(compiler: &Compiler, field: IrNodeId) -> Option<ast::Position> {
    let IrNode::Field(field) = compiler.arena.node(field) else {
        return None;
    };
    let reference = field.references.as_ref()?.first()?.read();
    let context = reference.context.as_ref()?.read();
    if context
        .scope_map
        .is_some_and(|scope| compiler.arena.is_var(scope))
    {
        return None;
    }
    drop(context);
    let key_path = reference.key_path.as_ref()?.read();
    let index = reference.key_path_index();
    let component = key_path.path.as_ref()?.get(index)?.read().as_ref()?.clone();
    component.read().unbox().map(|node| node.range().start)
}

// d2sequence.getObjEarliestLineNum/getEdgeEarliestLineNum inspect every
// reference's whole MapKey and ignore glob paths, unlike graph AST sorting.
fn sequence_object_line(compiler: &Compiler, field: IrNodeId) -> Option<i64> {
    let IrNode::Field(field) = compiler.arena.node(field) else {
        return None;
    };
    field
        .references
        .as_ref()?
        .iter()
        .filter_map(|reference| {
            let reference = reference.read();
            if reference.key_path.as_ref()?.read().has_glob() {
                return None;
            }
            let context = reference.context.as_ref()?.read();
            let key = context.key.as_ref()?.read();
            Some(key.range.start.line)
        })
        .min()
}

// Original d2graph.Object.hasEdgeRef tests MapKey.Edges, including every
// canonical declaration reference. Graph incidence is a different contract.
fn sequence_object_has_edge_reference(compiler: &Compiler, field: IrNodeId) -> bool {
    let IrNode::Field(field) = compiler.arena.node(field) else {
        return false;
    };
    field.references.as_ref().is_some_and(|references| {
        references.iter().any(|reference| {
            reference
                .read()
                .context
                .as_ref()
                .and_then(|context| context.read().key.clone())
                .is_some_and(|key| {
                    key.read()
                        .edges
                        .as_ref()
                        .is_some_and(|edges| !edges.is_empty())
                })
        })
    })
}

fn sequence_edge_line(compiler: &Compiler, edge: IrNodeId) -> Option<i64> {
    let IrNode::Edge(edge) = compiler.arena.node(edge) else {
        return None;
    };
    edge.references
        .as_ref()?
        .iter()
        .filter_map(|reference| {
            let reference = reference.read();
            let context = reference.context.as_ref()?.read();
            let edge = context.edge.as_ref()?.read();
            if edge.src.as_ref()?.read().has_glob() || edge.dst.as_ref()?.read().has_glob() {
                return None;
            }
            Some(context.key.as_ref()?.read().range.start.line)
        })
        .min()
}

fn content_field_count(compiler: &Compiler, field: IrNodeId) -> Option<usize> {
    let map = compiler.arena.map(field)?;
    let IrNode::Map(map_node) = compiler.arena.node(map) else {
        return None;
    };
    Some(
        map_node
            .fields
            .as_ref()
            .into_iter()
            .flatten()
            .filter(|child| {
                field_name(compiler, **child)
                    .is_some_and(|name| !is_metadata_field(compiler, **child) && name != "vars")
            })
            .count(),
    )
}

fn content_field_names(compiler: &Compiler, field: IrNodeId) -> Vec<String> {
    let Some(map) = compiler.arena.map(field) else {
        return Vec::new();
    };
    let IrNode::Map(map_node) = compiler.arena.node(map) else {
        return Vec::new();
    };
    map_node
        .fields
        .as_ref()
        .into_iter()
        .flatten()
        .filter_map(|child| {
            let name = field_name(compiler, *child)?;
            (!is_metadata_field(compiler, *child) && name != "vars").then_some(name)
        })
        .collect()
}

pub use d2_rs_core::SourceLegendGraph;

pub fn lower_legend_source_graph(
    compiler: &Compiler,
    root: IrNodeId,
) -> Result<Option<SourceLegendGraph>, GraphCompileError> {
    // Standalone legend callers still inherit the containing board's class
    // namespace; the reserved legend map is not a new compileBoard root.
    if compiler.graph_class_root.is_none() {
        let view = Compiler {
            arena: compiler.arena.fork_storage(),
            graph_class_root: Some(root),
            graph_context: compiler.graph_context.clone(),
            errors: compiler.errors.clone(),
            ..Default::default()
        };
        return lower_legend_source_graph(&view, root);
    }
    let Some(root_map) = compiler.arena.map(root) else {
        return Ok(None);
    };
    let Some(vars) = compiler
        .arena
        .get_field(root_map, &[ast::raw_string_box("vars", true)])
    else {
        return Ok(None);
    };
    let Some(vars_map) = compiler.arena.map(vars) else {
        return Ok(None);
    };
    let Some(field) = compiler
        .arena
        .get_field(vars_map, &[ast::raw_string_box("d2-legend", true)])
    else {
        return Ok(None);
    };
    let Some(legend_map) = compiler.arena.map(field) else {
        return Ok(None);
    };
    let label = compiler
        .arena
        .primary(field)
        .and_then(|id| match compiler.arena.node(id) {
            IrNode::Scalar(s) => Some(scalar_text(&s.value)),
            _ => None,
        })
        .unwrap_or_default();
    // The source never calls SetDimensions on this graph. The ordinary graph
    // compiler still supplies classes, styles, structure and edge identities.
    let (mut graph, labels, mut styles) = lower_graph_in_class_context(
        compiler,
        legend_map,
        &|_: &str| Size::default(),
        false,
        false,
        Some(root),
        GraphProjection::Source,
        None,
    )?;
    for node in &mut graph.nodes {
        node.size = Size {
            width: 100.0,
            height: 100.0,
        };
        node.label_size = Some(Size::default());
    }
    for style in styles.values_mut() {
        style.content_aspect_ratio_bits = None;
    }
    Ok(Some(SourceLegendGraph {
        label,
        graph,
        labels,
        styles,
    }))
}

/// Lower the reserved `vars.d2-legend` map into target metadata.
///
/// D2 compiles the legend through the ordinary graph compiler, then removes
/// it from the layout graph and retains the resulting objects and edges on
/// the target diagram. Keeping the same boundary here prevents legend entries
/// from affecting layout while preserving their source order for rendering.
fn lower_legend(compiler: &Compiler, root: IrNodeId) -> Result<Option<Legend>, GraphCompileError> {
    let Some(source) = lower_legend_source_graph(compiler, root)? else {
        return Ok(None);
    };
    // Preserve ordinary compileMap order, including implicit endpoints and
    // nested objects. Transparent shapes are omitted only at this boundary.
    let shapes = source
        .graph
        .nodes()
        .filter_map(|(id, node)| {
            let style = source.styles.get(&id).cloned().unwrap_or_default();
            if style
                .opacity
                .as_deref()
                .and_then(d2_rs_core::parse_d2_float)
                == Some(0.0)
            {
                return None;
            }
            Some(LegendShape {
                source_node: Some(id),
                font: style.font,
                label: source.labels.get(&id).cloned().unwrap_or_default(),
                shape: node.shape,
                fill: style.fill,
                stroke: style.stroke,
            })
        })
        .collect::<Vec<_>>();
    let connections = source
        .graph
        .edges()
        .map(|(id, _)| {
            let style = source.graph.edge_style(id);
            let arrows = source.graph.edge_arrows(id);
            let endpoint_labels = source.graph.edge_arrowhead_labels(id);
            LegendConnection {
                source_id: d2_rs_core::edge_abs_id(&source.graph, id).unwrap_or_default(),
                arrowheads: source.graph.edge_arrowheads(id),
                fill: style.fill,
                border_radius: style.border_radius,
                has_icon: source
                    .graph
                    .edge_icons
                    .get(id.0 as usize)
                    .is_some_and(Option::is_some),
                source_label: endpoint_labels.source.map(|l| l.text).unwrap_or_default(),
                target_label: endpoint_labels.target.map(|l| l.text).unwrap_or_default(),
                link: source.graph.edge_link(id).map(str::to_owned),
                tooltip: source.graph.edge_tooltip(id).map(str::to_owned),
                font: style.font,
                label: source
                    .graph
                    .edge_label(id)
                    .map(|l| l.text.clone())
                    .unwrap_or_default(),
                source_arrow: arrows.source,
                target_arrow: arrows.target,
                stroke: style.stroke,
                stroke_width: style.stroke_width,
                stroke_dash: style.stroke_dash,
                opacity: style.opacity,
                animated: style
                    .animated
                    .as_deref()
                    .and_then(d2_rs_core::parse_d2_bool)
                    == Some(true),
            }
        })
        .collect::<Vec<_>>();
    Ok(
        (!shapes.is_empty() || !connections.is_empty()).then_some(Legend {
            label: source.label.clone(),
            shapes,
            connections,
            source_graph: Some(Box::new(source)),
        }),
    )
}

/// Retain the semantic rows needed by the SQL-table target renderer. D2 keeps
/// column names, declared types, and constraint abbreviations in separate
/// target columns; flattening them to names loses information required by the
/// SVG and edge-label stages.
fn field_values(compiler: &Compiler, field: IrNodeId, path: &[&str]) -> Vec<String> {
    let Some(map) = compiler.arena.map(field) else {
        return Vec::new();
    };
    let Some(value_field) = compiler.graph_field(map, path) else {
        return Vec::new();
    };
    if let IrNode::Field(field_node) = compiler.arena.node(value_field)
        && let Some(composite) = field_node.composite
        && let IrNode::Array(array) = compiler.arena.node(composite)
    {
        return array
            .values
            .clone()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|value| match compiler.arena.node(value) {
                IrNode::Scalar(scalar) => {
                    Some(if path == ["constraint"] && scalar.value.null.is_some() {
                        "null".into()
                    } else {
                        scalar_text(&scalar.value)
                    })
                }
                _ => None,
            })
            .collect();
    }
    let Some(primary) = compiler.arena.primary(value_field) else {
        return Vec::new();
    };
    match compiler.arena.node(primary) {
        IrNode::Scalar(scalar) => vec![scalar_text(&scalar.value)],
        _ => Vec::new(),
    }
}

fn sql_constraint_abbreviation(value: &str) -> String {
    match value {
        "primary_key" => "PK".into(),
        "foreign_key" => "FK".into(),
        "unique" => "UNQ".into(),
        _ => value.to_owned(),
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
struct SqlTableDimensions {
    size: Size,
    max_name_width: u32,
    columns: Vec<SourceSqlColumn>,
}

fn structured_inner_padding(compiler: &Compiler, field: IrNodeId) -> f64 {
    let label = property_scalar_inherited(compiler, field, &["label"]).or_else(|| {
        compiler
            .arena
            .primary(field)
            .and_then(|id| match compiler.arena.node(id) {
                IrNode::Scalar(scalar) => Some(scalar.value.clone()),
                _ => None,
            })
    });
    if label.is_some_and(|value| value.scalar_bytes().is_empty())
        || ["width", "height"].iter().any(|key| {
            property_number_inherited(compiler, field, &[key]).is_some_and(|value| value != 0.0)
        })
    {
        0.0
    } else {
        5.0
    }
}

fn sql_table_dimensions<M: GraphMeasurer>(
    compiler: &Compiler,
    field: IrNodeId,
    label_size: Size,
    measurer: &M,
    theme_mono: bool,
) -> SqlTableDimensions {
    let Some(map) = compiler.arena.map(field) else {
        return SqlTableDimensions::default();
    };
    let fields = match compiler.arena.node(map) {
        IrNode::Map(map) => map.fields.clone().unwrap_or_default(),
        _ => Vec::new(),
    };
    let font_size = property_number_inherited(compiler, field, &["style", "font-size"])
        .and_then(|value| u32::try_from(value as i64).ok())
        .unwrap_or(20);
    // SetDimensions measures columns in the graph font family, which a
    // mono theme replaces with Source Code Pro; object fonts do not apply.
    let measure_column = |text: &str| {
        if theme_mono {
            measurer.measure_mono_styled_label_at_size(text, font_size, false, false)
        } else {
            measurer.measure_styled_label_at_size(text, font_size, false, false)
        }
    };
    let mut max_name: f64 = 0.0;
    let mut max_type: f64 = 0.0;
    let mut max_constraint: f64 = 0.0;
    let mut count = 0usize;
    let mut columns = Vec::new();
    for column in fields {
        let Some(name) = field_name(compiler, column) else {
            continue;
        };
        if is_metadata_field(compiler, column) {
            continue;
        }
        count += 1;
        let name_size = measure_column(&name);
        max_name = max_name.max(name_size.width);
        let type_name = property_scalar_inherited(compiler, column, &["label"])
            .map(|value| scalar_text(&value))
            .or_else(|| {
                compiler.arena.primary(column).and_then(|primary| {
                    match compiler.arena.node(primary) {
                        IrNode::Scalar(scalar) => Some(scalar_text(&scalar.value)),
                        _ => None,
                    }
                })
            })
            .unwrap_or_else(|| name.clone());
        let type_name = if type_name == name {
            String::new()
        } else {
            type_name
        };
        let type_size = measure_column(&type_name);
        max_type = max_type.max(type_size.width);
        let raw_constraints = field_values(compiler, column, &["constraint"]);
        let constraints = raw_constraints
            .iter()
            .map(|value| sql_constraint_abbreviation(value))
            .collect::<Vec<_>>();
        if !constraints.is_empty() {
            let text = constraints.join(", ");
            max_constraint = max_constraint.max(measure_column(&text).width);
        }
        columns.push(SourceSqlColumn {
            name: SourceSqlText::measured(name, name_size),
            type_text: SourceSqlText::measured(type_name, type_size),
            constraint: (!raw_constraints.is_empty()).then_some(raw_constraints),
            // addSQLTableColumnIndices mutates a Go range-copy, so the
            // original exported column retains its empty reference.
            reference: String::new(),
        });
    }
    // GetDefaultSize suppresses inner label padding when either authored
    // dimension is nonzero, including for structured headers.
    let inner_padding = structured_inner_padding(compiler, field);
    let header_width = 10.0 + label_size.width + inner_padding + 10.0;
    let rows_width = 10.0
        + max_name
        + 20.0
        + max_type
        + 20.0
        + max_constraint
        + if max_constraint > 0.0 { 20.0 } else { 0.0 };
    // The Go table target gives each row five pixels of extra vertical
    // breathing room because the header uses the larger font face.
    let row_height = label_size.height + inner_padding;
    SqlTableDimensions {
        size: Size {
            width: header_width.max(rows_width).max(12.0),
            height: (row_height * (count as f64 + 1.0)).max(12.0),
        },
        // D2's target renderer positions every type after the widest measured
        // column name. Keep the graph-stage integer measurement with the
        // source style so rendering does not approximate it from character
        // count.
        max_name_width: max_name.ceil().max(0.0) as u32,
        columns,
    }
}

fn class_member_rows(compiler: &Compiler, field: IrNodeId) -> Vec<(String, String)> {
    compiler
        .arena
        .map(field)
        .and_then(|map| match compiler.arena.node(map) {
            IrNode::Map(map) => Some(map.fields.clone().unwrap_or_default()),
            _ => None,
        })
        .unwrap_or_default()
        .into_iter()
        .filter_map(|member| {
            let name = field_name(compiler, member)?;
            if is_metadata_field(compiler, member) {
                return None;
            }
            let value = property_scalar_inherited(compiler, member, &["label"])
                .map(|value| scalar_text(&value))
                .or_else(|| {
                    compiler.arena.primary(member).and_then(|primary| {
                        match compiler.arena.node(primary) {
                            IrNode::Scalar(scalar) => Some(scalar_text(&scalar.value)),
                            _ => None,
                        }
                    })
                })
                .unwrap_or_else(|| name.clone());
            let value = if value == name {
                if name.contains('(') {
                    "void".into()
                } else {
                    String::new()
                }
            } else {
                value
            };
            Some((name, value))
        })
        .collect()
}

fn class_member_underlines(compiler: &Compiler, field: IrNodeId, methods: bool) -> Vec<bool> {
    compiler
        .arena
        .map(field)
        .and_then(|map| match compiler.arena.node(map) {
            IrNode::Map(map) => Some(map.fields.clone().unwrap_or_default()),
            _ => None,
        })
        .unwrap_or_default()
        .into_iter()
        .filter_map(|member| {
            let name = field_name(compiler, member)?;
            if is_metadata_field(compiler, member) || name.contains('(') != methods {
                return None;
            }
            Some(
                property_text_inherited(compiler, member, &["style", "underline"]).as_deref()
                    == Some("true"),
            )
        })
        .collect()
}

fn class_field_rows(compiler: &Compiler, field: IrNodeId) -> Vec<(String, String)> {
    class_member_rows(compiler, field)
        .into_iter()
        .filter(|(name, _)| !name.contains('('))
        .collect()
}

fn class_method_rows(compiler: &Compiler, field: IrNodeId) -> Vec<(String, String)> {
    class_member_rows(compiler, field)
        .into_iter()
        .filter(|(name, _)| name.contains('('))
        .collect()
}

/// Computes the class target's two-column box from its field and method rows.
/// The source implementation uses Source Code Pro for member text, then adds
/// fixed prefix/type padding and a 20px vertical row allowance. Keeping this
/// calculation beside the SQL table sizing makes the graph lowering match the
/// target stage before TALA sees the nodes.
fn class_dimensions<M: GraphMeasurer>(
    compiler: &Compiler,
    field: IrNodeId,
    label_size: Size,
    measurer: &M,
) -> Size {
    // Source chooses row height from the first field, then the first
    // method if there are no fields, regardless of declaration interleaving.
    let members = class_field_rows(compiler, field)
        .into_iter()
        .chain(class_method_rows(compiler, field))
        .map(|(name, value)| {
            // Visibility is rendered in its own 20px prefix column. The
            // upstream width measurement covers only member name + type.
            let name = name.strip_prefix(['+', '-', '#']).unwrap_or(name.as_str());
            format!("{name}{value}")
        })
        .collect::<Vec<_>>();
    let font_size = property_number_inherited(compiler, field, &["style", "font-size"])
        .and_then(|value| u32::try_from(value as i64).ok())
        .unwrap_or(20);
    let inner_padding = structured_inner_padding(compiler, field);
    let mut max_width = (label_size.width + inner_padding).max(12.0);
    let mut row_height = 0.0;
    for member in &members {
        let measured = measurer.measure_mono_label_at_size(member, font_size);
        max_width = max_width.max(measured.width);
        if row_height == 0.0 {
            row_height = measured.height + 20.0;
        }
    }
    let width = 10.0 + 20.0 + max_width + 50.0 + 20.0;
    let height = if row_height == 0.0 {
        2.0 * (label_size.height + inner_padding).max(12.0) + 20.0
    } else {
        row_height * members.len() as f64
            + (2.0 * row_height).max(label_size.height + inner_padding + 10.0)
    };
    Size {
        width: width.max(12.0).ceil(),
        height: height.max(12.0).ceil(),
    }
}

// d2graph.EnsureChild searches the owning sequence's existing actors before
// each child lookup. Its same-name exception retains a.a beneath actor a.
fn sequence_child_parent(
    parent: GraphNodeId,
    scalar: &str,
    nodes: &BTreeMap<String, GraphNodeId>,
    graph: &Graph,
    visible_nodes: usize,
) -> Option<Option<GraphNodeId>> {
    let local_id = |id| {
        let node = graph.node(id)?;
        match node.parent.and_then(|p| graph.node(p)) {
            Some(parent) => node
                .external_id
                .strip_prefix(&parent.external_id)?
                .strip_prefix('.'),
            None => Some(node.external_id.as_str()),
        }
    };
    if local_id(parent) == Some(scalar) {
        return Some(Some(parent));
    }
    // OuterSequenceDiagram starts at the object's parent, not itself.
    let mut ancestor = graph.node(parent)?.parent;
    while let Some(id) = ancestor {
        if graph.is_sequence_diagram(id) {
            break;
        }
        ancestor = graph.node(id)?.parent;
    }
    if ancestor.is_some() || graph.root_sequence_diagram {
        let scope = ancestor
            .and_then(|id| graph.node(id))
            .map(|node| node.external_id.as_str());
        let candidate = append_path(scope, scalar);
        if nodes.get(&candidate).is_some_and(|id| {
            (id.0 as usize) < visible_nodes
                && graph.node(*id).is_some_and(|node| node.parent == ancestor)
                && local_id(*id) == Some(scalar)
        }) {
            return Some(ancestor);
        }
    }
    Some(Some(parent))
}

// Connect repeats ensureChildEdge after compiling a map's fields. Retain each
// missing intermediate child, even if a following component redirects again.
fn ensure_sequence_endpoint(
    endpoint: &str,
    nodes: &mut BTreeMap<String, GraphNodeId>,
    table_columns: &BTreeMap<String, Vec<String>>,
    graph: &mut Graph,
    visible_nodes: usize,
    labels: &mut BTreeMap<GraphNodeId, String>,
    measurer: &impl GraphMeasurer,
    theme_mono: bool,
) -> Option<(GraphNodeId, Option<usize>)> {
    if graph
        .root_object
        .as_ref()
        .is_some_and(|node| matches!(node.shape, ShapeKind::Class | ShapeKind::SqlTable))
    {
        return Some((GraphNodeId::ROOT, None));
    }
    let parsed = crate::parse::parse_key(endpoint.as_bytes()).ok()?;
    let components = format::key_path(&parsed)?;
    let mut prefix = String::new();
    let mut current = None;
    for (index, component) in components.iter().enumerate() {
        let scalar = parsed
            .path
            .as_ref()?
            .get(index)?
            .read()
            .as_ref()?
            .read()
            .scalar_string();
        if let Some(parent) = current {
            if matches!(
                graph.node(parent)?.shape,
                ShapeKind::Class | ShapeKind::SqlTable
            ) {
                let remainder = std::str::from_utf8(component).ok()?;
                return resolve_structured_endpoint(
                    &append_path(Some(&prefix), remainder),
                    nodes,
                    table_columns,
                    graph,
                );
            }
            current = sequence_child_parent(parent, &scalar, nodes, graph, visible_nodes)?;
            prefix = current
                .and_then(|id| graph.node(id))
                .map(|node| node.external_id.clone())
                .unwrap_or_default();
        }
        prefix = append_path(
            Some(prefix.as_str()).filter(|p| !p.is_empty()),
            std::str::from_utf8(component).ok()?,
        );
        let child = if let Some(id) = nodes.get(&prefix).copied() {
            id
        } else {
            let id = add_implicit_node(graph, nodes, &prefix, &scalar, measurer, theme_mono);
            graph.node_mut(id)?.parent = current;
            graph
                .hierarchy_children_order
                .entry(current)
                .or_default()
                .push(id);
            let level = current
                .and_then(|id| graph.node_label_levels.get(&id).copied())
                .unwrap_or(0)
                + 1;
            graph.node_label_levels.insert(id, level);
            graph.set_node_label(id, scalar.clone());
            // newObject has no References; a connected edge alone must not
            // enable the legacy measured-graph edge-reference fallback.
            graph.sequence_node_edge_refs.insert(id, false);
            labels.insert(id, scalar);
            id
        };
        current = Some(child);
    }
    current.map(|id| (id, None))
}

fn resolve_structured_endpoint(
    endpoint: &str,
    nodes: &BTreeMap<String, GraphNodeId>,
    table_columns: &BTreeMap<String, Vec<String>>,
    graph: &Graph,
) -> Option<(GraphNodeId, Option<usize>)> {
    // The graph root is also an Object in Go. A structured root truncates
    // every member path before descending; all such endpoints are the same
    // root (so SQL self-connections have no column indices).
    if graph
        .root_object
        .as_ref()
        .is_some_and(|node| matches!(node.shape, ShapeKind::Class | ShapeKind::SqlTable))
    {
        return Some((GraphNodeId::ROOT, None));
    }
    // ensureChildEdge stops as soon as traversal reaches a class/table;
    // member paths belong to that object rather than implicit child nodes.
    let parsed = crate::parse::parse_key(endpoint.as_bytes()).ok()?;
    let components = format::key_path(&parsed)?;
    let mut prefix = String::new();
    for (index, component) in components.iter().enumerate() {
        if !prefix.is_empty() {
            prefix.push('.');
        }
        prefix.push_str(std::str::from_utf8(component).ok()?);
        if index + 1 == components.len() {
            break;
        }
        let Some(node) = nodes.get(&prefix).copied() else {
            continue;
        };
        match graph.node(node)?.shape {
            ShapeKind::Class => return Some((node, None)),
            ShapeKind::SqlTable => {
                let member = parsed
                    .path
                    .as_ref()?
                    .get(index + 1)?
                    .read()
                    .as_ref()?
                    .read()
                    .scalar_string();
                let column = table_columns
                    .get(&prefix)?
                    .iter()
                    .position(|name| name == &member);
                return Some((node, column));
            }
            _ => {}
        }
    }
    nodes.get(&prefix).copied().map(|node| (node, None))
}

fn collect_edges_at(
    compiler: &Compiler,
    map: IrNodeId,
    prefix: Option<&str>,
    output: &mut Vec<CollectedGraphEdge>,
) {
    let IrNode::Map(map_node) = compiler.arena.node(map) else {
        return;
    };
    let mut frames = vec![(
        map,
        map_node.fields.clone().unwrap_or_default().into_iter(),
        prefix.map(str::to_owned),
    )];
    while let Some((map, fields, prefix)) = frames.last_mut() {
        let Some(field) = fields.next() else {
            let IrNode::Map(map_node) = compiler.arena.node(*map) else {
                unreachable!()
            };
            let validation_start = output.len();
            output.extend(
                map_node
                    .edges
                    .iter()
                    .flatten()
                    .enumerate()
                    .map(|(index, edge)| CollectedGraphEdge {
                        edge: *edge,
                        parent_path: prefix.clone(),
                        validation_index: validation_start + index,
                    }),
            );
            frames.pop();
            continue;
        };
        let Some(name) = field_name(compiler, field) else {
            continue;
        };
        if is_metadata_field(compiler, field) || name == "vars" {
            continue;
        }
        if is_board_wrapper(compiler, field, &name) {
            continue;
        }
        // Structured members do not become graph nodes, but their maps
        // still own edges connecting members of the same class/table.
        if let Some(child_map) = compiler.arena.map(field)
            && let Some(path_name) = path_component(compiler, field)
        {
            let path = append_path(prefix.as_deref(), &path_name);
            if let IrNode::Map(child) = compiler.arena.node(child_map) {
                frames.push((
                    child_map,
                    child.fields.clone().unwrap_or_default().into_iter(),
                    Some(path),
                ));
            }
        }
    }
}

fn explicit_external_label(position: LabelPosition, size: Size) -> Option<ExternalLabel> {
    let (side, alignment) = match position {
        LabelPosition::OutsideTopLeft => (ExternalSide::Top, ExternalAlignment::Start),
        LabelPosition::OutsideTopCenter => (ExternalSide::Top, ExternalAlignment::Center),
        LabelPosition::OutsideTopRight => (ExternalSide::Top, ExternalAlignment::End),
        LabelPosition::OutsideRightTop => (ExternalSide::Right, ExternalAlignment::Start),
        LabelPosition::OutsideRightMiddle => (ExternalSide::Right, ExternalAlignment::Center),
        LabelPosition::OutsideRightBottom => (ExternalSide::Right, ExternalAlignment::End),
        LabelPosition::OutsideBottomLeft => (ExternalSide::Bottom, ExternalAlignment::Start),
        LabelPosition::OutsideBottomCenter => (ExternalSide::Bottom, ExternalAlignment::Center),
        LabelPosition::OutsideBottomRight => (ExternalSide::Bottom, ExternalAlignment::End),
        LabelPosition::OutsideLeftTop => (ExternalSide::Left, ExternalAlignment::Start),
        LabelPosition::OutsideLeftMiddle => (ExternalSide::Left, ExternalAlignment::Center),
        LabelPosition::OutsideLeftBottom => (ExternalSide::Left, ExternalAlignment::End),
        _ => return None,
    };
    Some(ExternalLabel {
        size,
        side,
        alignment,
        automatic: false,
        reserve_space: true,
    })
}

fn automatic_external_label(
    shape: ShapeKind,
    is_container: bool,
    undersized: bool,
    size: Size,
) -> Option<ExternalLabel> {
    let (side, alignment) = if undersized && shape == ShapeKind::Circle {
        (ExternalSide::Top, ExternalAlignment::Center)
    } else if undersized {
        (ExternalSide::Bottom, ExternalAlignment::Center)
    } else if (is_container
        && matches!(
            shape,
            ShapeKind::Circle | ShapeKind::Oval | ShapeKind::Cylinder
        ))
        || shape == ShapeKind::Image
    {
        // Image labels are rendered below the 128px image box in D2. The
        // other automatic top-side cases are container decorations.
        if shape == ShapeKind::Image {
            (ExternalSide::Bottom, ExternalAlignment::Center)
        } else {
            (ExternalSide::Top, ExternalAlignment::Center)
        }
    } else if shape == ShapeKind::Person {
        (ExternalSide::Bottom, ExternalAlignment::Center)
    } else {
        return None;
    };
    Some(ExternalLabel {
        size,
        side,
        alignment,
        automatic: true,
        reserve_space: !undersized,
    })
}

fn source_content_insets(
    shape: ShapeKind,
    is_container: bool,
    has_icon: bool,
    icon_position: Option<LabelPosition>,
) -> Insets {
    let mut insets = if !is_container {
        Insets::uniform(60.0)
    } else {
        match shape {
            ShapeKind::Cylinder => Insets {
                top: 108.0,
                right: 60.0,
                bottom: 84.0,
                left: 60.0,
            },
            ShapeKind::Package => Insets {
                top: 77.0,
                right: 60.0,
                bottom: 60.0,
                left: 60.0,
            },
            _ => Insets::uniform(60.0),
        }
    };
    if has_icon && is_container {
        // D2's inferred container icon is outside the border, so it does not
        // enlarge the fitted content box. Only an explicitly inside icon
        // reserves the 64px icon plus 10px padding lane on its side.
        match icon_position {
            Some(
                LabelPosition::InsideTopLeft
                | LabelPosition::InsideTopCenter
                | LabelPosition::InsideTopRight,
            ) => insets.top = insets.top.max(74.0),
            Some(
                LabelPosition::InsideBottomLeft
                | LabelPosition::InsideBottomCenter
                | LabelPosition::InsideBottomRight,
            ) => insets.bottom = insets.bottom.max(74.0),
            Some(LabelPosition::InsideMiddleLeft) => insets.left = insets.left.max(74.0),
            Some(LabelPosition::InsideMiddleRight) => insets.right = insets.right.max(74.0),
            _ => {}
        }
    }
    insets
}

fn transform_text(label: String, transform: Option<&str>) -> String {
    match transform {
        Some("uppercase") => d2_rs_core::ir::go_uppercase_text(&label),
        Some("lowercase") => d2_rs_core::ir::go_lowercase_text(&label),
        Some("capitalize") => {
            let mut output = String::with_capacity(label.len());
            let mut word_start = true;
            for character in label.chars() {
                if character.is_alphanumeric() {
                    if word_start {
                        output.extend(character.to_uppercase());
                    } else {
                        output.extend(character.to_lowercase());
                    }
                    word_start = false;
                } else {
                    output.push(character);
                    word_start = true;
                }
            }
            output
        }
        _ => label,
    }
}

fn transform_with_theme(
    label: String,
    transform: Option<&str>,
    caps_lock: bool,
    code_shape: bool,
    language: Option<&str>,
) -> String {
    let label =
        if caps_lock && !code_shape && language != Some("latex") && transform != Some("none") {
            d2_rs_core::ir::go_uppercase_text(&label)
        } else {
            label
        };
    transform_text(label, transform)
}

// Go strings can retain malformed bytes. Only rune case conversions replace
// them; cases.Title preserves their storage. Keep presentation out of transport.
fn transform_label_bytes(
    bytes: Vec<u8>,
    transform: Option<&str>,
    caps_lock: bool,
    code_shape: bool,
    language: Option<&str>,
) -> Vec<u8> {
    if std::str::from_utf8(&bytes).is_ok()
        || matches!(transform, Some("uppercase" | "lowercase"))
        || (caps_lock && !code_shape && language != Some("latex") && transform != Some("none"))
    {
        return transform_with_theme(
            ast::ByteString::from(bytes).to_text().into_owned(),
            transform,
            caps_lock,
            code_shape,
            language,
        )
        .into_bytes();
    }
    if transform != Some("capitalize") {
        return bytes;
    }
    let mut output = Vec::with_capacity(bytes.len());
    let mut remaining = bytes.as_slice();
    let mut word_start = true;
    while !remaining.is_empty() {
        let (valid, invalid) = match std::str::from_utf8(remaining) {
            Ok(_) => (remaining.len(), false),
            Err(error) => (error.valid_up_to(), true),
        };
        for character in std::str::from_utf8(&remaining[..valid]).unwrap().chars() {
            let text = if character.is_alphanumeric() {
                let text = if word_start {
                    character.to_uppercase().collect::<String>()
                } else {
                    character.to_lowercase().collect::<String>()
                };
                word_start = false;
                text
            } else {
                word_start = true;
                character.to_string()
            };
            output.extend_from_slice(text.as_bytes());
        }
        remaining = &remaining[valid..];
        if invalid {
            output.push(remaining[0]);
            remaining = &remaining[1..];
            word_start = true;
        }
    }
    output
}

fn field_label_info(
    compiler: &Compiler,
    field: IrNodeId,
    path: &str,
    caps_lock: bool,
    sequence_ancestor: bool,
) -> (String, bool, Vec<u8>) {
    let shape =
        property_text_inherited(compiler, field, &["shape"]).map(|value| value.to_lowercase());
    let language = field_language(compiler, field);
    // setDefaultShapes gives sequence actors a rectangle before SetDimensions
    // applies the theme's caps-lock rule (d2graph/d2graph.go).
    let code_shape = shape.as_deref() == Some("code")
        || (shape.is_none()
            && !sequence_ancestor
            && language
                .as_deref()
                .is_some_and(|value| !matches!(value, "markdown" | "latex")));
    let transform = property_text_inherited(compiler, field, &["style", "text-transform"]);
    if let Some(label) = property_scalar_inherited(compiler, field, &["label"]) {
        let markdown = language.as_deref() == Some("markdown");
        let bytes = transform_label_bytes(
            label.scalar_bytes(),
            transform.as_deref(),
            caps_lock,
            code_shape,
            language.as_deref(),
        );
        return (
            ast::ByteString::from(bytes.as_slice())
                .to_text()
                .into_owned(),
            markdown,
            bytes,
        );
    }
    let IrNode::Field(field_node) = compiler.arena.node(field) else {
        let text = path.rsplit('.').next().unwrap_or(path).to_owned();
        return (text.clone(), false, text.into_bytes());
    };
    // An explicit empty scalar (`node: ""`) is a real empty label in D2.
    // Only fields with no scalar primary fall back to the key as their
    // implicit label.
    let label = field_node
        .primary
        .and_then(|primary| match compiler.arena.node(primary) {
            IrNode::Scalar(scalar) => Some((
                scalar.value.scalar_bytes(),
                scalar
                    .value
                    .block_string
                    .as_ref()
                    .is_some_and(|value| value.read().tag == "md"),
            )),
            _ => None,
        });
    // Key paths retain source quoting for stable IDs, but implicit labels use
    // the decoded scalar text (`I/O\nManager` becomes two rendered lines).
    let (label, _) = label.unwrap_or_else(|| (field_node.name.scalar_string().into_bytes(), false));
    let markdown = language.as_deref() == Some("markdown");
    let bytes = transform_label_bytes(
        label,
        transform.as_deref(),
        caps_lock,
        code_shape,
        language.as_deref(),
    );
    (
        ast::ByteString::from(bytes.as_slice())
            .to_text()
            .into_owned(),
        markdown,
        bytes,
    )
}

fn fallback_size(label: &str) -> Size {
    // Keep a positive deterministic rectangle if a packaged bundled face is
    // corrupt. Normal library calls use `BundledGraphMeasurer`; this path is
    // intentionally conservative so graph construction can still report
    // structural errors instead of panicking while loading assets.
    Size {
        width: (label.chars().count() as f64 * 8.0 + 32.0).max(40.0),
        height: 40.0,
    }
}

/// Apply the v0.9.0 shape-library sizing contract to measured label bounds.
///
/// D2 performs this before invoking a layout engine: the text ruler returns
/// raw label dimensions, the graph layer adds its 5 px inner label padding,
/// and the selected shape contributes its default padding and geometry.  The
/// layout engine must therefore receive these fitted dimensions rather than a
/// rectangle-shaped approximation for every node.
#[derive(Clone, Copy, Debug)]
struct ShapeFitOptions {
    shape: ShapeKind,
    raw_label: Size,
    font_size: u32,
    include_inner_label_padding: bool,
    desired_width: bool,
    desired_height: bool,
    has_icon: bool,
    has_link_tooltip: bool,
}

fn fit_source_shape(options: ShapeFitOptions) -> Size {
    let ShapeFitOptions {
        shape,
        raw_label,
        font_size,
        include_inner_label_padding,
        desired_width,
        desired_height,
        has_icon,
        has_link_tooltip,
    } = options;
    // GetDefaultSize mutates the label dimensions before shape fitting. Code
    // labels use 0.5em on each side; all ordinary labels use 5 px when no
    // explicit dimensions suppress that axis. Text and code shapes have no
    // shape padding of their own.
    let mut content_width = raw_label.width;
    let mut content_height = raw_label.height;
    if shape == ShapeKind::Code {
        content_width += f64::from(font_size);
        content_height += f64::from(font_size);
    } else if include_inner_label_padding && shape != ShapeKind::Text {
        content_width += 5.0;
        content_height += 5.0;
    }

    let mut padding_x = match shape {
        ShapeKind::Circle => 40.0 / std::f64::consts::SQRT_2,
        ShapeKind::Diamond => 10.0,
        ShapeKind::Hexagon => 20.0,
        ShapeKind::Person | ShapeKind::C4Person => 10.0,
        ShapeKind::Step => 10.0,
        ShapeKind::StoredData => 30.0,
        ShapeKind::Queue => 20.0,
        ShapeKind::Code => 0.0,
        _ => 40.0,
    };
    let mut padding_y = match shape {
        ShapeKind::Callout | ShapeKind::Cloud | ShapeKind::Cylinder => 20.0,
        ShapeKind::Circle => 40.0 / std::f64::consts::SQRT_2,
        ShapeKind::Diamond | ShapeKind::Hexagon => 20.0,
        ShapeKind::Person | ShapeKind::C4Person => 40.0,
        ShapeKind::Page => 20.348 + 40.0,
        ShapeKind::Package => 32.0,
        ShapeKind::Step => 75.0,
        ShapeKind::StoredData | ShapeKind::Queue => 40.0,
        ShapeKind::Document => 40.0 * 14.0 / 18.925,
        ShapeKind::Text | ShapeKind::Code | ShapeKind::Image => 0.0,
        _ => 40.0,
    };

    // SetDimensions disables the corresponding default padding for an
    // explicit width/height. Icons and link+tooltip decorations add space
    // only on unconstrained axes.
    if desired_width {
        padding_x = 0.0;
    }
    if desired_height {
        padding_y = 0.0;
    }
    if has_icon
        && !matches!(
            shape,
            ShapeKind::Class | ShapeKind::SqlTable | ShapeKind::Code | ShapeKind::Text
        )
    {
        let label_height = raw_label.height + 5.0;
        if !desired_width {
            padding_x += label_height;
        }
        if !desired_height {
            padding_y += label_height;
        }
    }
    if has_link_tooltip
        && !desired_width
        && !matches!(
            shape,
            ShapeKind::Class | ShapeKind::SqlTable | ShapeKind::Code | ShapeKind::Text
        )
    {
        // d2graph reserves a horizontal lane for the paired link/tooltip
        // badge. The decoration is rendered outside the object box, so it
        // does not affect the implicit height. v0.9.0 uses a fixed 64 px
        // lane here (see d2graph.GetDefaultSize/SizeToContent).
        padding_x += 64.0;
    }

    let (mut width, mut height) = match shape {
        // D2's graph layer uses MIN_SHAPE_SIZE (5px) for text shapes. The
        // larger 12px floor belongs to tabular/class envelopes; applying it
        // here widens short Markdown labels and changes their raster origin.
        ShapeKind::Text => (content_width.max(5.0), content_height.max(5.0)),
        ShapeKind::Image => (128.0, 128.0),
        ShapeKind::Circle => {
            let diameter = (std::f64::consts::SQRT_2
                * (content_width + padding_x).max(content_height + padding_y))
            .ceil();
            (diameter, diameter)
        }
        ShapeKind::RealSquare => {
            let side = (content_width + padding_x)
                .max(content_height + padding_y)
                .ceil();
            (side, side)
        }
        ShapeKind::Oval => {
            let theta = f64::from((content_height.atan2(content_width)) as f32);
            (
                (std::f64::consts::SQRT_2 * (content_width + padding_x * theta.cos())).ceil(),
                (std::f64::consts::SQRT_2 * (content_height + padding_y * theta.sin())).ceil(),
            )
        }
        ShapeKind::Diamond => (
            (2.0 * (content_width + padding_x)).ceil(),
            (2.0 * (content_height + padding_y)).ceil(),
        ),
        ShapeKind::Hexagon => (
            (1.5 * (content_width + padding_x)).ceil(),
            (1.5 * (content_height + padding_y)).ceil(),
        ),
        ShapeKind::Parallelogram => (
            (content_width + padding_x + 52.0).ceil(),
            (content_height + padding_y).ceil(),
        ),
        ShapeKind::Document => (
            (content_width + padding_x).ceil(),
            ((content_height + padding_y) * 18.925 / 14.0).ceil(),
        ),
        ShapeKind::Cylinder => (
            (content_width + padding_x).ceil(),
            (content_height + padding_y + 72.0).ceil(),
        ),
        ShapeKind::Queue => (
            (content_width + padding_x + 72.0).ceil(),
            (content_height + padding_y).ceil(),
        ),
        ShapeKind::Page => {
            let mut fitted_width = content_width + padding_x;
            let fitted_height = content_height + padding_y;
            if fitted_height < 3.0 * 20.348 {
                fitted_width += 20.8164;
            }
            (
                fitted_width.max(2.0 * 20.8164).ceil(),
                fitted_height.max(20.348).ceil(),
            )
        }
        ShapeKind::Package => {
            let inner_height = content_height + padding_y;
            let top_height = (inner_height * 0.2 / 0.8).min(55.0);
            (
                (content_width + padding_x).ceil(),
                (inner_height + top_height).ceil(),
            )
        }
        ShapeKind::Step => (
            (content_width + padding_x + 70.0).ceil(),
            (content_height + padding_y).ceil(),
        ),
        ShapeKind::Callout => {
            let base_height = content_height + padding_y;
            (
                (content_width + padding_x).ceil(),
                if base_height < 45.0 {
                    (2.0 * base_height).ceil()
                } else {
                    (base_height + 45.0).ceil()
                },
            )
        }
        ShapeKind::StoredData => (
            (content_width + padding_x + 30.0).ceil(),
            (content_height + padding_y).ceil(),
        ),
        // D2 deliberately bypasses shapePerson.GetDimensionsToFit in
        // Object.SizeToContent. The silhouette's shoulder geometry is drawn
        // around this rectangular content box and must not enlarge it.
        ShapeKind::Person => (content_width + padding_x, content_height + padding_y),
        ShapeKind::C4Person => {
            let total_width = (content_width + padding_x) / 0.9;
            let head_radius = total_width * 0.22;
            let body_top = head_radius + head_radius * 0.8;
            let mut fitted_width = total_width;
            let mut fitted_height = (content_height + padding_y + body_top + total_width * 0.06)
                .max(total_width * 0.95);
            if fitted_width > 1.5 * fitted_height {
                fitted_height = (fitted_width / 1.5).round();
            } else if fitted_height > 1.5 * fitted_width {
                fitted_width = (fitted_height / 1.5).round();
            }
            (fitted_width.ceil(), fitted_height.ceil())
        }
        // Class and SQL tables are expanded from their rows below. Their
        // source lowering keeps the dedicated table/class path intact.
        ShapeKind::Class | ShapeKind::SqlTable => (
            (content_width + padding_x).ceil(),
            (content_height + padding_y).ceil(),
        ),
        ShapeKind::Cloud => {
            const WIDE_WIDTH: f64 = 0.819;
            const WIDE_HEIGHT: f64 = 0.548;
            const TALL_WIDTH: f64 = 0.549;
            const TALL_HEIGHT: f64 = 0.820;
            const SQUARE_SIDE: f64 = 0.663;
            let padded_width = content_width + padding_x;
            let padded_height = content_height + padding_y;
            let aspect_ratio = padded_width / padded_height;
            if aspect_ratio > (1.0 + WIDE_WIDTH / WIDE_HEIGHT) / 2.0 {
                (
                    (padded_width / WIDE_WIDTH).ceil(),
                    (padded_height / WIDE_HEIGHT).ceil(),
                )
            } else if aspect_ratio < (1.0 + TALL_WIDTH / TALL_HEIGHT) / 2.0 {
                (
                    (padded_width / TALL_WIDTH).ceil(),
                    (padded_height / TALL_HEIGHT).ceil(),
                )
            } else {
                (
                    (padded_width / SQUARE_SIDE).ceil(),
                    (padded_height / SQUARE_SIDE).ceil(),
                )
            }
        }
        _ => (
            (content_width + padding_x).ceil(),
            (content_height + padding_y).ceil(),
        ),
    };
    width = width.max(5.0);
    height = height.max(5.0);
    Size { width, height }
}

/// Supplies label dimensions while lowering source IR into the shared graph.
///
/// Native callers can pass their own adapter (for example one that loads a
/// user-supplied font). The default [`lower_graph`] path uses the same bundled
/// D2 v0.9.0 faces as the CLI and WASM boundaries, so direct library users do
/// not silently receive character-count geometry.
pub trait GraphMeasurer {
    /// Returns the padded node size in layout pixels.
    fn measure(&self, label: &str) -> Size;

    /// Returns the raw text bounds before shape padding is applied.
    fn measure_label(&self, label: &str) -> Size {
        self.measure(label)
    }

    /// Whether this measurer supplies raw D2 ruler dimensions suitable for
    /// the shape-library fitting pass. Lightweight callers traditionally
    /// return already-padded node boxes and therefore retain the old path.
    fn supports_shape_fitting(&self) -> bool {
        false
    }

    /// Returns the rendered label size at a requested D2 font size.
    ///
    /// Simple callers can implement only [`Self::measure`]; source lowering
    /// falls back to that measurement when a measurer does not distinguish
    /// sizes.
    fn measure_at_size(&self, label: &str, _font_size: u32) -> Size {
        self.measure(label)
    }

    /// Returns raw text bounds at a requested D2 font size.
    fn measure_label_at_size(&self, label: &str, font_size: u32) -> Size {
        self.measure_at_size(label, font_size)
    }

    /// Returns a rendered node label size with the source style applied.
    ///
    /// The default keeps existing lightweight measurers source-compatible;
    /// native callers that have style-specific font faces can override it.
    fn measure_styled_at_size(
        &self,
        label: &str,
        font_size: u32,
        _bold: bool,
        _italic: bool,
    ) -> Size {
        self.measure_at_size(label, font_size)
    }

    /// Returns padded node dimensions using the diagram's default
    /// monospace family. Existing lightweight measurers remain compatible by
    /// falling back to their ordinary styled measurement.
    fn measure_mono_styled_at_size(
        &self,
        label: &str,
        font_size: u32,
        bold: bool,
        italic: bool,
    ) -> Size {
        self.measure_styled_at_size(label, font_size, bold, italic)
    }

    /// Returns raw text bounds with the source style applied.
    fn measure_styled_label_at_size(
        &self,
        label: &str,
        font_size: u32,
        _bold: bool,
        _italic: bool,
    ) -> Size {
        self.measure_label_at_size(label, font_size)
    }

    /// Returns raw monospace bounds for a source `code` label. D2 uses
    /// Source Code Pro for these labels, independently of the diagram's
    /// ordinary Source Sans Pro family.
    fn measure_code_label_at_size(&self, label: &str, font_size: u32) -> Size {
        self.measure_label_at_size(label, font_size)
    }

    /// Returns raw Source Code Pro bounds for a class field or method row.
    /// Class members use the mono face but retain ordinary line metrics (the
    /// code block's 1.3em line-height does not apply to these rows).
    fn measure_mono_label_at_size(&self, label: &str, font_size: u32) -> Size {
        self.measure_label_at_size(label, font_size)
    }

    /// Returns raw monospace bounds with an ordinary source weight.  This is
    /// used for `style.font: mono`, whose bold flag selects Source Code Pro's
    /// bold face rather than Source Sans Pro.
    fn measure_mono_styled_label_at_size(
        &self,
        label: &str,
        font_size: u32,
        _bold: bool,
        _italic: bool,
    ) -> Size {
        self.measure_mono_label_at_size(label, font_size)
    }

    /// Measures a Markdown label after its inline structure is rendered.
    fn measure_markdown(&self, label: &str) -> Size {
        self.measure(label)
    }

    /// Measures raw Markdown label bounds at a requested font size.
    fn measure_markdown_label_at_size(&self, label: &str, font_size: u32) -> Size {
        self.measure_label_at_size(label, font_size)
    }

    /// Returns a rendered Markdown node label size with the source style.
    fn measure_styled_markdown_at_size(
        &self,
        label: &str,
        font_size: u32,
        _bold: bool,
        _italic: bool,
    ) -> Size {
        self.measure_markdown_at_size(label, font_size)
    }

    /// Returns raw Markdown bounds with the source style.
    fn measure_styled_markdown_label_at_size(
        &self,
        label: &str,
        font_size: u32,
        _bold: bool,
        _italic: bool,
    ) -> Size {
        self.measure_markdown_label_at_size(label, font_size)
    }

    /// Measures Markdown using the diagram's default monospace family. An
    /// explicit `style.font` still selects the corresponding styled method.
    fn measure_mono_styled_markdown_at_size(
        &self,
        label: &str,
        font_size: u32,
        bold: bool,
        italic: bool,
    ) -> Size {
        self.measure_styled_markdown_at_size(label, font_size, bold, italic)
    }

    /// Returns raw Markdown bounds using the diagram's default monospace
    /// family. An explicit `style.font` still selects the corresponding
    /// styled method.
    fn measure_mono_styled_markdown_label_at_size(
        &self,
        label: &str,
        font_size: u32,
        bold: bool,
        italic: bool,
    ) -> Size {
        self.measure_styled_markdown_label_at_size(label, font_size, bold, italic)
    }

    /// Measures padded Markdown node dimensions at a requested font size.
    fn measure_markdown_at_size(&self, label: &str, font_size: u32) -> Size {
        self.measure_at_size(label, font_size)
    }

    /// Measures a LaTeX label after its math representation is prepared.
    ///
    /// Higher-level callers can provide successful custom metrics. The checked
    /// hooks preserve native math intrinsic-measurement errors before fallback
    /// geometry can be used.
    fn measure_latex(&self, label: &str) -> Size {
        self.measure(label)
    }

    /// Measures raw LaTeX bounds at the default font size.
    fn measure_latex_label(&self, label: &str) -> Size {
        self.measure_latex(label)
    }

    /// Preserve D2 GetLabelSize's fallible intrinsic math measurement.
    /// Custom successful metric hooks remain supported after validation.
    fn measure_latex_label_checked_at_size(
        &self,
        label: &str,
        font_size: u32,
    ) -> Result<Size, String> {
        d2_rs_math::measure(label).map_err(|error| match error {
            // Original renderer faults bypass GetLabelSize's ordinary error
            // wrapping and are recovered at the outer API boundary.
            d2_rs_math::MathError::SourceNilChild | d2_rs_math::MathError::InputLimit(_) => {
                error.to_string()
            }
            _ => format!("latex failed to parse: {error}"),
        })?;
        Ok(self.measure_latex_label_at_size(label, font_size))
    }
    fn measure_latex_checked_at_size(&self, label: &str, font_size: u32) -> Result<Size, String> {
        self.measure_latex_label_checked_at_size(label, font_size)?;
        Ok(self.measure_latex_at_size(label, font_size))
    }
    /// Measures padded LaTeX dimensions at a requested font size.
    fn measure_latex_at_size(&self, label: &str, font_size: u32) -> Size {
        self.measure_at_size(label, font_size)
    }

    /// Measures raw LaTeX bounds at a requested font size.
    fn measure_latex_label_at_size(&self, label: &str, font_size: u32) -> Size {
        self.measure_latex_at_size(label, font_size)
    }
}

pub use d2_rs_core::{SourceNodeStyle, SourceSqlColumn, SourceSqlText, SourceTargetAccents};

/// Source graph plus labels and target style metadata for native rendering.
pub type SourceGraph = (
    Graph,
    BTreeMap<GraphNodeId, String>,
    BTreeMap<GraphNodeId, SourceNodeStyle>,
);

impl<F> GraphMeasurer for F
where
    F: for<'a> Fn(&'a str) -> Size,
{
    fn measure(&self, label: &str) -> Size {
        self(label)
    }
}

struct FallbackMeasurer;

impl GraphMeasurer for FallbackMeasurer {
    fn measure(&self, label: &str) -> Size {
        fallback_size(label)
    }
}

/// The checked-in D2 faces used by the default graph lowering boundary.
///
/// Parsing is done once because font tables are immutable and the graph
/// lowering API is called for every board. Keeping the result optional makes
/// the API robust to a corrupted package while retaining the old deterministic
/// fallback as a last-resort behavior.
struct BundledGraphFonts {
    regular: d2_rs_text::measure::TrueTypeFont,
    bold: d2_rs_text::measure::TrueTypeFont,
    italic: d2_rs_text::measure::TrueTypeFont,
    semibold: d2_rs_text::measure::TrueTypeFont,
    mono: d2_rs_text::measure::TrueTypeFont,
    mono_bold: d2_rs_text::measure::TrueTypeFont,
    mono_italic: d2_rs_text::measure::TrueTypeFont,
    mono_semibold: d2_rs_text::measure::TrueTypeFont,
}

fn bundled_graph_fonts() -> Option<&'static BundledGraphFonts> {
    static FONTS: OnceLock<Option<BundledGraphFonts>> = OnceLock::new();
    FONTS
        .get_or_init(|| {
            Some(BundledGraphFonts {
                regular: d2_rs_text::measure::bundled_source_sans_pro_regular().ok()?,
                bold: d2_rs_text::measure::bundled_source_sans_pro_bold().ok()?,
                italic: d2_rs_text::measure::bundled_source_sans_pro_italic().ok()?,
                semibold: d2_rs_text::measure::bundled_source_sans_pro_semibold().ok()?,
                mono: d2_rs_text::measure::bundled_source_code_pro_regular().ok()?,
                mono_bold: d2_rs_text::measure::bundled_source_code_pro_bold().ok()?,
                mono_italic: d2_rs_text::measure::bundled_source_code_pro_italic().ok()?,
                mono_semibold: d2_rs_text::measure::bundled_source_code_pro_semibold().ok()?,
            })
        })
        .as_ref()
}

struct BundledGraphMeasurer {
    fonts: &'static BundledGraphFonts,
}

impl BundledGraphMeasurer {
    fn styled(&self, bold: bool, italic: bool) -> &d2_rs_text::measure::TrueTypeFont {
        if bold {
            &self.fonts.bold
        } else if italic {
            &self.fonts.italic
        } else {
            &self.fonts.regular
        }
    }

    fn mono_styled(&self, bold: bool, italic: bool) -> &d2_rs_text::measure::TrueTypeFont {
        if bold {
            &self.fonts.mono_bold
        } else if italic {
            &self.fonts.mono_italic
        } else {
            &self.fonts.mono
        }
    }

    fn raw_metrics(&self, label: &str, font_size: u32, bold: bool, italic: bool) -> Size {
        let metrics = self.styled(bold, italic).measure_with_fallback(
            label,
            f64::from(font_size),
            &self.fonts.mono,
        );
        Size {
            width: metrics.width.ceil(),
            height: metrics.height.ceil(),
        }
    }

    fn raw_mono_metrics(&self, label: &str, font_size: u32, bold: bool, italic: bool) -> Size {
        let font = self.mono_styled(bold, italic);
        let metrics = font.measure_with_fallback(label, f64::from(font_size), &self.fonts.mono);
        Size {
            width: metrics.width.ceil(),
            height: metrics.height.ceil(),
        }
    }

    fn padded(raw: Size) -> Size {
        Size {
            width: (raw.width + 45.0).max(40.0),
            height: (raw.height + 45.0).max(24.0),
        }
    }
}

impl GraphMeasurer for BundledGraphMeasurer {
    fn supports_shape_fitting(&self) -> bool {
        true
    }

    fn measure(&self, label: &str) -> Size {
        self.measure_styled_at_size(label, 16, false, false)
    }

    fn measure_label(&self, label: &str) -> Size {
        self.measure_styled_label_at_size(label, 16, false, false)
    }

    fn measure_at_size(&self, label: &str, font_size: u32) -> Size {
        self.measure_styled_at_size(label, font_size, false, false)
    }

    fn measure_label_at_size(&self, label: &str, font_size: u32) -> Size {
        self.measure_styled_label_at_size(label, font_size, false, false)
    }

    fn measure_styled_at_size(
        &self,
        label: &str,
        font_size: u32,
        bold: bool,
        italic: bool,
    ) -> Size {
        Self::padded(self.measure_styled_label_at_size(label, font_size, bold, italic))
    }

    fn measure_mono_styled_at_size(
        &self,
        label: &str,
        font_size: u32,
        bold: bool,
        italic: bool,
    ) -> Size {
        Self::padded(self.measure_mono_styled_label_at_size(label, font_size, bold, italic))
    }

    fn measure_styled_label_at_size(
        &self,
        label: &str,
        font_size: u32,
        bold: bool,
        italic: bool,
    ) -> Size {
        self.raw_metrics(label, font_size, bold, italic)
    }

    fn measure_code_label_at_size(&self, label: &str, font_size: u32) -> Size {
        let metrics =
            self.fonts
                .mono
                .measure_d2_code_label(label, f64::from(font_size), &self.fonts.mono);
        Size {
            width: metrics.width.ceil(),
            height: metrics.height,
        }
    }

    fn measure_mono_label_at_size(&self, label: &str, font_size: u32) -> Size {
        self.raw_mono_metrics(label, font_size, false, false)
    }

    fn measure_mono_styled_label_at_size(
        &self,
        label: &str,
        font_size: u32,
        bold: bool,
        italic: bool,
    ) -> Size {
        self.raw_mono_metrics(label, font_size, bold, italic)
    }

    fn measure_markdown_at_size(&self, label: &str, font_size: u32) -> Size {
        self.measure_styled_markdown_at_size(label, font_size, false, false)
    }

    fn measure_markdown_label_at_size(&self, label: &str, font_size: u32) -> Size {
        self.measure_styled_markdown_label_at_size(label, font_size, false, false)
    }

    fn measure_styled_markdown_at_size(
        &self,
        label: &str,
        font_size: u32,
        bold: bool,
        italic: bool,
    ) -> Size {
        Self::padded(self.measure_styled_markdown_label_at_size(label, font_size, bold, italic))
    }

    fn measure_styled_markdown_label_at_size(
        &self,
        label: &str,
        font_size: u32,
        bold: bool,
        italic: bool,
    ) -> Size {
        d2_rs_text::markdown_measure::MarkdownMeasurer {
            font: &self.fonts.regular,
            bold_font: &self.fonts.bold,
            italic_font: &self.fonts.italic,
            semibold_font: &self.fonts.semibold,
            mono_font: &self.fonts.mono,
            mono_bold_font: &self.fonts.mono_bold,
            mono_semibold_font: &self.fonts.mono_semibold,
            mono_italic_font: &self.fonts.mono_italic,
        }
        .markdown_dimensions(label, font_size, bold, italic, false)
    }

    fn measure_mono_styled_markdown_at_size(
        &self,
        label: &str,
        font_size: u32,
        bold: bool,
        italic: bool,
    ) -> Size {
        Self::padded(
            self.measure_mono_styled_markdown_label_at_size(label, font_size, bold, italic),
        )
    }
    fn measure_mono_styled_markdown_label_at_size(
        &self,
        label: &str,
        font_size: u32,
        bold: bool,
        italic: bool,
    ) -> Size {
        d2_rs_text::markdown_measure::MarkdownMeasurer {
            font: &self.fonts.regular,
            bold_font: &self.fonts.bold,
            italic_font: &self.fonts.italic,
            semibold_font: &self.fonts.semibold,
            mono_font: &self.fonts.mono,
            mono_bold_font: &self.fonts.mono_bold,
            mono_semibold_font: &self.fonts.mono_semibold,
            mono_italic_font: &self.fonts.mono_italic,
        }
        .markdown_dimensions(label, font_size, bold, italic, true)
    }

    fn measure_latex_at_size(&self, label: &str, font_size: u32) -> Size {
        Self::padded(self.measure_latex_label_at_size(label, font_size))
    }

    fn measure_latex_label_at_size(&self, label: &str, _font_size: u32) -> Size {
        d2_rs_math::measure(label).unwrap_or_default()
    }
}

fn d2_validation_attributes(
    compiler: &Compiler,
    field: IrNodeId,
    prefix: &[&str],
    size: Option<Size>,
) -> d2_rs_core::D2ValidationAttributes {
    let size = size.unwrap_or_default();
    d2_rs_core::D2ValidationAttributes {
        label_dimensions: [size.width as i64, size.height as i64],
        style: d2_rs_core::D2_VALIDATED_STYLE_KEYS
            .iter()
            .filter_map(|(name, _)| {
                let mut path = prefix.to_vec();
                path.extend(["style", *name]);
                property_scalar_inherited(compiler, field, &path)
                    .map(|value| ((*name).to_owned(), value.scalar_bytes()))
            })
            .collect(),
    }
}

fn apply_node_properties(
    compiler: &Compiler,
    field: IrNodeId,
    graph: &mut Graph,
    graph_id: GraphNodeId,
    retain_source_projection: bool,
) {
    if !retain_source_projection {
        // Planning's remaining graph consumers are endpoint resolution and
        // near/edge validation. They read shape and grid membership, while
        // dimensions, text/icon placement and layout styling are discarded.
        // Attribute validation has already read the IR in source order.
        let sequence_ancestor =
            graph.root_sequence_diagram || has_sequence_ancestor(compiler, field);
        let shape =
            property_text_inherited(compiler, field, &["shape"]).map(|value| value.to_lowercase());
        let node = graph.node_mut(graph_id).expect("new graph node");
        node.shape = shape.as_deref().map_or_else(
            || {
                default_shape(
                    field_language(compiler, field).as_deref(),
                    sequence_ancestor,
                )
            },
            |value| parse_shape(Some(value)),
        );
        node.grid_rows = property_number_inherited(compiler, field, &["grid-rows"])
            .and_then(|value| usize::try_from(value as i64).ok());
        node.grid_columns = property_number_inherited(compiler, field, &["grid-columns"])
            .and_then(|value| usize::try_from(value as i64).ok());
        return;
    }
    let grid_spacing = GridSpacing {
        grid_gap: property_number_inherited(compiler, field, &["grid-gap"]),
        vertical_gap: property_number_inherited(compiler, field, &["vertical-gap"]),
        horizontal_gap: property_number_inherited(compiler, field, &["horizontal-gap"]),
    };
    graph.set_grid_spacing(graph_id, grid_spacing);
    let row_directed = grid_row_directed(compiler, field);
    graph.set_grid_row_directed(graph_id, row_directed);
    let has_language = graph.node_has_language(graph_id);
    let sequence_ancestor = graph.root_sequence_diagram || has_sequence_ancestor(compiler, field);
    let node = graph.node_mut(graph_id).expect("new graph node");
    // These attributes carry source spelling into public graph/layout APIs.
    // Graph-stage validation reads the IR and structural graph directly.
    if retain_source_projection {
        let raw_attribute = |name| {
            property_scalar_inherited(compiler, field, &[name]).map(|value| value.scalar_bytes())
        };
        let authored = d2_rs_core::D2AuthoredGeometry {
            object_id: match compiler.arena.node(field) {
                IrNode::Field(field) => format::format_key_path(std::slice::from_ref(&field.name)),
                _ => Vec::new(),
            },
            width: raw_attribute("width"),
            height: raw_attribute("height"),
            top: raw_attribute("top"),
            left: raw_attribute("left"),
            input_box: None,
            attributes: Some(Box::new(d2_rs_core::D2ObjectValidation {
                missing_box: false,
                shape: property_text_inherited(compiler, field, &["shape"])
                    .map(|value| value.to_lowercase())
                    .map(|v| v.into_bytes()),
                attributes: d2_validation_attributes(compiler, field, &[], node.label_size),
                // Compiler-generated positions are already validated enums. The
                // wire adapter independently retains raw, possibly invalid names.
                label_position: (node.label_position != LabelPosition::Unset)
                    .then(|| node.label_position.d2_name().as_bytes().to_vec()),
                icon_position: node
                    .icon_position
                    .filter(|p| *p != LabelPosition::Unset)
                    .map(|p| p.d2_name().as_bytes().to_vec()),
                invalid_near_key_part: None,
            })),
        };
        // Retain the local D2 identity for adapter errors and traversal even when
        // no geometry scalar was authored.
        node.d2_authored_geometry = Some(Box::new(authored));
    }

    let shape_value =
        property_text_inherited(compiler, field, &["shape"]).map(|value| value.to_lowercase());
    // setDefaultShapes gives outer sequence diagrams precedence over language.
    node.shape = shape_value.as_deref().map_or_else(
        || {
            default_shape(
                field_language(compiler, field).as_deref(),
                sequence_ancestor,
            )
        },
        |value| parse_shape(Some(value)),
    );
    node.force_hierarchy = shape_value.as_deref() == Some("hierarchy");
    node.person = matches!(node.shape, ShapeKind::Person | ShapeKind::C4Person);
    node.is_3d = property_is_true_inherited(compiler, field, &["style", "3d"]);
    node.is_multiple = property_is_true_inherited(compiler, field, &["style", "multiple"]);
    node.has_icon = property_text_inherited(compiler, field, &["icon"]).is_some();
    node.content_alignment = match node.shape {
        ShapeKind::Circle | ShapeKind::Oval => d2_rs_core::ContentAlignment::Center,
        ShapeKind::Diamond => d2_rs_core::ContentAlignment::Diamond,
        ShapeKind::Cloud => d2_rs_core::ContentAlignment::Cloud,
        _ => d2_rs_core::ContentAlignment::Padding,
    };
    node.label_position = property_text_inherited(compiler, field, &["label-position"])
        .map(|value| LabelPosition::from_d2_name(&value))
        .or_else(|| {
            property_text_inherited(compiler, field, &["label", "near"])
                .and_then(|value| ast::keywords::label_position(&value))
        })
        .unwrap_or_default();
    node.icon_position = property_text_inherited(compiler, field, &["icon-position"])
        .map(|value| LabelPosition::from_d2_name(&value))
        .or_else(|| {
            property_text_inherited(compiler, field, &["icon", "near"])
                .and_then(|value| ast::keywords::label_position(&value))
        });

    if let Some(font_size) = property_number_inherited(compiler, field, &["style", "font-size"])
        .and_then(|value| u32::try_from(value as i64).ok())
    {
        node.font_size = Some(font_size);
    }
    if let Some(width) = property_number_inherited(compiler, field, &["width"]) {
        node.size.width =
            if matches!(node.shape, ShapeKind::SqlTable | ShapeKind::Class) || has_language {
                node.size.width.max(width.max(0.0))
            } else {
                width.max(0.0)
            };
        node.fixed_width = true;
    }
    if let Some(height) = property_number_inherited(compiler, field, &["height"]) {
        node.size.height =
            if matches!(node.shape, ShapeKind::SqlTable | ShapeKind::Class) || has_language {
                node.size.height.max(height.max(0.0))
            } else {
                height.max(0.0)
            };
        node.fixed_height = true;
    }
    // Object.SizeToContent applies aspect constraints after authored dimensions.
    // An explicit zero still leaves the source's desired dimension at zero.
    if matches!(node.shape, ShapeKind::Circle | ShapeKind::RealSquare) {
        let side = node.size.width.max(node.size.height);
        node.size = Size {
            width: side,
            height: side,
        };
    } else if property_number_inherited(compiler, field, &["width"]).unwrap_or(0.0) == 0.0
        || property_number_inherited(compiler, field, &["height"]).unwrap_or(0.0) == 0.0
    {
        let ratio = match node.shape {
            ShapeKind::Person => Some(1.5),
            ShapeKind::Oval => Some(3.0),
            _ => None,
        };
        if let Some(ratio) = ratio {
            if node.size.width > ratio * node.size.height {
                node.size.height = (node.size.width / ratio).round();
            } else if node.size.height > ratio * node.size.width {
                node.size.width = (node.size.height / ratio).round();
            }
        }
    }
    if node.fixed_width || node.fixed_height {
        node.declared_size = Some(node.size);
    }
    node.direction =
        parse_direction(property_text_inherited(compiler, field, &["direction"]).as_deref());
    node.grid_rows = property_number_inherited(compiler, field, &["grid-rows"])
        .and_then(|value| usize::try_from(value as i64).ok());
    node.grid_columns = property_number_inherited(compiler, field, &["grid-columns"])
        .and_then(|value| usize::try_from(value as i64).ok());
    node.canvas_position = property_text_inherited(compiler, field, &["near"]).and_then(|near| {
        Some(match near.as_str() {
            "top-left" => d2_rs_core::CanvasPosition::TopLeft,
            "top-center" => d2_rs_core::CanvasPosition::TopCenter,
            "top-right" => d2_rs_core::CanvasPosition::TopRight,
            "center-left" => d2_rs_core::CanvasPosition::CenterLeft,
            "center-right" => d2_rs_core::CanvasPosition::CenterRight,
            "bottom-left" => d2_rs_core::CanvasPosition::BottomLeft,
            "bottom-center" => d2_rs_core::CanvasPosition::BottomCenter,
            "bottom-right" => d2_rs_core::CanvasPosition::BottomRight,
            _ => return None,
        })
    });

    let top = property_number_inherited(compiler, field, &["top"]);
    let left = property_number_inherited(compiler, field, &["left"]);
    node.constrained_y = top;
    node.constrained_x = left;
    if let (Some(x), Some(y)) = (left, top) {
        node.locked_position = Some(Point { x, y });
    }
}

fn add_implicit_node(
    graph: &mut Graph,
    nodes: &mut BTreeMap<String, GraphNodeId>,
    path: &str,
    label: &str,
    measurer: &impl GraphMeasurer,
    theme_mono: bool,
) -> GraphNodeId {
    if let Some(id) = nodes.get(path).copied() {
        return id;
    }
    let size = if theme_mono {
        measurer.measure_mono_styled_at_size(label, 16, true, false)
    } else {
        measurer.measure_styled_at_size(label, 16, true, false)
    };
    let mut node = GraphNode::new(path, size);
    node.declared_size = Some(size);
    node.font_size = Some(16);
    node.label_size = Some(if theme_mono {
        measurer.measure_mono_styled_label_at_size(label, 16, true, false)
    } else {
        measurer.measure_styled_label_at_size(label, 16, true, false)
    });
    let id = graph.add_node(node);
    nodes.insert(path.to_owned(), id);
    id
}

/// Lowers a compiled IR root into the shared measured graph model.
pub fn lower_graph(compiler: &Compiler, root: IrNodeId) -> Result<Graph, GraphCompileError> {
    if let Some(fonts) = bundled_graph_fonts() {
        lower_graph_with_measurer(compiler, root, &BundledGraphMeasurer { fonts })
    } else {
        lower_graph_with_measurer(compiler, root, &FallbackMeasurer)
    }
}

/// Lowers a compiled IR root using caller-supplied label measurements.
pub fn lower_graph_with_measurer<M: GraphMeasurer>(
    compiler: &Compiler,
    root: IrNodeId,
    measurer: &M,
) -> Result<Graph, GraphCompileError> {
    lower_graph_with_measurer_and_labels(compiler, root, measurer).map(|(graph, _, _)| graph)
}

/// Lowers a compiled IR root and returns the source labels alongside the
/// shared measured graph.
///
/// Labels are kept outside [`Graph`] so the layout model remains compatible
/// with callers that construct its public node records directly. Renderers
/// that begin from source can use this map to preserve explicit labels while
/// the layout engine continues to consume only measured dimensions.
pub fn lower_graph_with_measurer_and_labels<M: GraphMeasurer>(
    compiler: &Compiler,
    root: IrNodeId,
    measurer: &M,
) -> Result<SourceGraph, GraphCompileError> {
    lower_graph_with_measurer_and_labels_theme(compiler, root, measurer, false)
}

/// Lowers a compiled graph while applying a target theme's caps-lock rule
/// before measurement. D2 applies this in the graph stage, so themed labels
/// must be transformed before their dimensions reach TALA. The ordinary
/// lowering entry point remains theme-neutral for callers that render later.
pub fn lower_graph_with_measurer_and_labels_theme<M: GraphMeasurer>(
    compiler: &Compiler,
    root: IrNodeId,
    measurer: &M,
    caps_lock: bool,
) -> Result<SourceGraph, GraphCompileError> {
    lower_graph_with_measurer_and_labels_theme_options(compiler, root, measurer, caps_lock, false)
}

/// Lowers a compiled graph while applying theme text rules before measurement.
/// D2 applies caps-lock and monospace theme rules in the graph stage, so both
/// transformed text and selected font metrics must reach layout.
pub fn lower_graph_with_measurer_and_labels_theme_options<M: GraphMeasurer>(
    compiler: &Compiler,
    root: IrNodeId,
    measurer: &M,
    caps_lock: bool,
    theme_mono: bool,
) -> Result<SourceGraph, GraphCompileError> {
    lower_graph_with_projection(
        compiler,
        root,
        measurer,
        caps_lock,
        theme_mono,
        GraphProjection::Source,
    )
}

/// Lower a measured graph for rendering without the auxiliary object/edge JSON
/// records used by source-graph inspection APIs. All layout/render attributes,
/// synthetic-root metadata and source validation remain present.
pub fn lower_render_graph_with_measurer<M: GraphMeasurer>(
    compiler: &Compiler,
    root: IrNodeId,
    measurer: &M,
    caps_lock: bool,
    theme_mono: bool,
) -> Result<SourceGraph, GraphCompileError> {
    lower_graph_with_projection(
        compiler,
        root,
        measurer,
        caps_lock,
        theme_mono,
        GraphProjection::Render,
    )
}

#[derive(Clone, Copy)]
enum GraphProjection {
    Plan,
    Render,
    Source,
}

#[cfg(test)]
mod render_projection_contract {
    use super::*;

    #[test]
    fn render_projection_preserves_measured_graphs_and_ordered_errors() {
        for source in [
            "a -> b: edge",
            "classes: { c: {style.fill: red; icon: icon.svg} }\na: {class: c}",
            "label: root\nicon: root.svg\ngrid-columns: 1\na: {icon: node.svg}",
            "shape: sql_table\nid: int\nid -> id",
            "table: {shape: sql_table; id: int}\ntable.id -> a",
            "type: {shape: class; field: int}\na -> type",
            "shape: sequence_diagram\na -> b: message\na.note: note",
            "a: |md **bold** |\nb: |latex x^2 |\na -> b",
            "a -> b: {source-arrowhead: label {shape: triangle; icon: arrow.svg}}",
            "layers: {detail: {label: detail; a -> b}}",
            "a: {icon: \"%\"}\nb: {shape: image}",
            "a: {shape: image}\nb: {icon: \"%\"}",
            "a -> b: {source-arrowhead.icon: \"%\"; target-arrowhead.icon: \"%\"}",
            "a: {width: -1; style.opacity: 2}\nb: {near: missing}",
        ] {
            let parsed = crate::parse("projection.d2", source.as_bytes(), None);
            assert!(parsed.error.is_none(), "{source}: {:?}", parsed.error);
            let (compiler, root) = super::super::compile_ir(ast::Shared::new(parsed.map)).unwrap();
            let boards = super::super::boards::board_targets(&compiler, root);
            for (_, board) in boards {
                for (caps_lock, theme_mono) in [(false, false), (true, false), (false, true)] {
                    let measured = |label: &str| Size {
                        width: label.len() as f64,
                        height: 17.0,
                    };
                    let full = lower_graph_with_measurer_and_labels_theme_options(
                        &compiler, board, &measured, caps_lock, theme_mono,
                    );
                    let rendered = lower_render_graph_with_measurer(
                        &compiler, board, &measured, caps_lock, theme_mono,
                    );
                    match (full, rendered) {
                        (Ok((mut full, labels, styles)), Ok(rendered)) => {
                            assert!(rendered.0.source_metadata.is_none(), "{source}");
                            if !full.nodes.is_empty() {
                                assert!(full.source_metadata.is_some(), "{source}");
                            }
                            // Include serde-skipped layout/render state, root metadata,
                            // labels and styles; remove only the inspection records.
                            full.source_metadata = None;
                            assert_eq!(
                                format!("{rendered:?}"),
                                format!("{:?}", (full, labels, styles)),
                                "{source}"
                            );
                        }
                        (Err(full), Err(rendered)) => {
                            assert_eq!(rendered.to_string(), full.to_string(), "{source}")
                        }
                        (full, rendered) => panic!("{source}: {full:?} / {rendered:?}"),
                    }
                }
            }
        }
    }
}

/// Validates a board and reports whether it has drawable objects.
///
/// CLI planning discards the measured graph. Keep its zero-size measurement
/// and validation order, but omit infallible text/style projections used only
/// by graph APIs and rendering. The incomplete planning graph never escapes
/// this API.
pub fn board_is_renderable(compiler: &Compiler, root: IrNodeId) -> Result<bool, GraphCompileError> {
    struct PlanMeasurer;
    impl GraphMeasurer for PlanMeasurer {
        fn measure(&self, _: &str) -> Size {
            Size::default()
        }
    }
    lower_graph_with_projection(
        compiler,
        root,
        &PlanMeasurer,
        false,
        false,
        GraphProjection::Plan,
    )
    .map(|(graph, _, _)| !graph.nodes.is_empty())
}

#[cfg(test)]
mod board_plan_contract {
    use super::*;
    use crate::{compiler::compile_ir, parse};

    #[test]
    fn structural_plan_preserves_renderability_and_complete_errors() {
        // Compare the planner with the original zero-measurement graph path,
        // including object errors that must precede later icon/edge errors.
        for source in [
            "",
            "a: |md **bold** |\nb: |latex x^2 |\na -> b",
            "a: |latex \\frac{ |\nb: {shape: image}",
            "a: |latex \\frac{ |\na.shape: class\nb: {shape: image}",
            "classes: {image: {shape: image}}\na.class: image\nb.icon: \"%\"",
            "a: {shape: image; child}\nb: {shape: image}",
            "a: {shape: image; icon: \"%\"}\nb: {shape: image}",
            "shape: sequence_diagram\na: {shape: image; icon: image.png; child}\na -> b",
            "table: {shape: sql_table; id: int}\ntable.id -> node",
            "type: {shape: class; field: int}\ntype.field -> node",
            "shape: sql_table\nid: int\nid -> id",
            "grid: {grid-columns: 1; cell}\ngrid -> grid.cell",
            "a: {shape: class; field: int; near: b}\nb",
            "a: {icon: \"%\"}\nb: {near: missing}",
        ] {
            let parsed = parse("plan.d2", source.as_bytes(), None);
            assert!(parsed.error.is_none(), "{source}: {:?}", parsed.error);
            let (compiler, root) = compile_ir(ast::Shared::new(parsed.map)).unwrap();
            let original = lower_graph_with_measurer(&compiler, root, &|_: &str| Size::default())
                .map(|graph| !graph.nodes.is_empty());
            assert_eq!(board_is_renderable(&compiler, root), original, "{source}");
        }
    }
}

fn lower_graph_with_projection<M: GraphMeasurer>(
    compiler: &Compiler,
    root: IrNodeId,
    measurer: &M,
    caps_lock: bool,
    theme_mono: bool,
    projection: GraphProjection,
) -> Result<SourceGraph, GraphCompileError> {
    if compiler.graph_class_root.is_none() {
        let mut view = Compiler {
            arena: compiler.arena.fork_storage(),
            variable_expansion: compiler.variable_expansion.clone(),
            graph_copies_accounted: std::cell::Cell::new(compiler.graph_copies_accounted.get()),
            graph_context: compiler.graph_context.clone(),
            errors: compiler.errors.clone(),
            ..Default::default()
        };
        let root = view.prepare_graph_board(root)?;
        view.graph_class_root = Some(root);
        return lower_graph_in_class_context(
            &view,
            root,
            measurer,
            caps_lock,
            theme_mono,
            None,
            projection,
            Some(&compiler.arrowhead_validation_memo),
        );
    }
    lower_graph_in_class_context(
        compiler, root, measurer, caps_lock, theme_mono, None, projection, None,
    )
}

#[allow(clippy::too_many_arguments)]
fn lower_graph_in_class_context<M: GraphMeasurer>(
    compiler: &Compiler,
    root: IrNodeId,
    measurer: &M,
    caps_lock: bool,
    theme_mono: bool,
    legend_board: Option<IrNodeId>,
    projection: GraphProjection,
    validation_memo: Option<&super::arrowhead_validation::ValidationMemo>,
) -> Result<SourceGraph, GraphCompileError> {
    let retain_source_projection = !matches!(projection, GraphProjection::Plan);
    let retain_source_metadata = matches!(projection, GraphProjection::Source);
    let mut source_keys = None;
    compiler.graph_check()?;
    let _field_session = compiler.graph_field_session();
    // d2compiler.Compile returns IR diagnostics before constructing any graph.
    // The IR arena can contain a useful partial root alongside those errors;
    // that root must not be mistaken for a successful graph compilation.
    if !compiler.errors.is_empty() {
        return Err(GraphCompileError::InvalidAttributes(
            compiler.errors.clone(),
        ));
    }
    // D2's IR accepts these attributes; graph compilation validates applied
    // styles and arrowheads before defaults or rendering hide invalid values.
    let (errors, validated_hierarchy) = if let Some(board_root) = legend_board {
        (
            super::arrowhead_validation::validate_legend_attributes(compiler, root, board_root),
            None,
        )
    } else if let Some(memo) = validation_memo {
        super::arrowhead_validation::validate_arrowhead_attributes_memoized(
            compiler,
            root,
            retain_source_projection,
            memo,
        )
    } else {
        super::arrowhead_validation::validate_arrowhead_attributes(
            compiler,
            root,
            retain_source_projection,
        )
    };
    compiler.graph_copy_check()?;
    compiler.graph_check()?;
    if !errors.is_empty() {
        return Err(GraphCompileError::InvalidAttributes(errors));
    }
    let mut graph = Graph::default();
    if let Some(config) = super::extract_config(compiler, root) {
        graph.d2_config_data = config.graph_data();
        graph.d2_config_pad = config.pad;
        graph.d2_config_center = config.center;
        graph.d2_config_layout_engine = config.layout_engine;
    }
    if retain_source_projection {
        let root_style = d2_rs_core::RootStyle {
            fill: property_text_inherited(compiler, root, &["style", "fill"]).or_else(|| {
                (property_text_inherited(compiler, root, &["shape"])
                    .map(|value| value.to_lowercase())
                    .as_deref()
                    == Some("text"))
                .then(|| "transparent".to_owned())
            }),
            fill_pattern: property_text_inherited(compiler, root, &["style", "fill-pattern"]),
            stroke: property_text_inherited(compiler, root, &["style", "stroke"]),
            stroke_width: property_text_inherited(compiler, root, &["style", "stroke-width"]),
            stroke_dash: property_text_inherited(compiler, root, &["style", "stroke-dash"]),
            border_radius: property_text_inherited(compiler, root, &["style", "border-radius"]),
            double_border: property_text_inherited(compiler, root, &["style", "double-border"]),
            opacity: property_text_inherited(compiler, root, &["style", "opacity"]),
            shadow: property_text_inherited(compiler, root, &["style", "shadow"]),
            three_dee: property_text_inherited(compiler, root, &["style", "3d"]),
            multiple: property_text_inherited(compiler, root, &["style", "multiple"]),
            font_color: property_text_inherited(compiler, root, &["style", "font-color"]),
            font: property_text_inherited(compiler, root, &["style", "font"]).map(|value| {
                String::from_utf8(d2_rs_core::ir::go_lowercase(value.as_bytes()))
                    .expect("source font is UTF-8")
            }),
            bold: property_text_inherited(compiler, root, &["style", "bold"]),
            italic: property_text_inherited(compiler, root, &["style", "italic"]),
            underline: property_text_inherited(compiler, root, &["style", "underline"]),
            icon_border_radius: source_icon_border_radius(compiler, root),
        };
        if root_style != d2_rs_core::RootStyle::default() {
            graph.root_style = Some(root_style);
        }
    }

    let mut labels = BTreeMap::new();
    let mut styles = BTreeMap::new();
    if let Some(direction) =
        parse_direction(property_text(compiler, root, &["direction"]).as_deref())
    {
        graph.direction = direction;
        graph.explicit_direction = Some(direction);
    }
    graph.root_grid_rows = root_grid_count(compiler, root, &["grid-rows"]);
    graph.root_grid_columns = root_grid_count(compiler, root, &["grid-columns"]);
    graph.root_grid_row_directed = match (
        property_key_range(compiler, root, &["grid-rows"]),
        property_key_range(compiler, root, &["grid-columns"]),
    ) {
        (Some(rows), Some(columns)) => Some(rows.before(&columns)),
        _ => None,
    };
    graph.root_grid_gap = property_number(compiler, root, &["grid-gap"]);
    graph.root_vertical_gap = property_number(compiler, root, &["vertical-gap"]);
    graph.root_horizontal_gap = property_number(compiler, root, &["horizontal-gap"]);
    graph.root_hierarchy =
        property_text(compiler, root, &["shape"]).as_deref() == Some("hierarchy");
    let root_sequence_diagram = property_text(compiler, root, &["shape"])
        .is_some_and(|shape| shape.eq_ignore_ascii_case("sequence_diagram"));
    graph.root_sequence_diagram = root_sequence_diagram;
    let mut nodes = BTreeMap::new();
    let mut table_columns = BTreeMap::new();
    let mut fields = Vec::new();
    collect_fields_at(compiler, root, None, &mut fields)?;
    let (mut fields, aliases, property_view, sequence_scope_ends) =
        canonical_graph_fields(compiler, fields, root_sequence_diagram);
    let original_compiler = compiler;
    let compiler = property_view.as_ref().unwrap_or(compiler);
    if retain_source_projection {
        graph.source_root = Some(source_root::compile(compiler, root)?);
    } else {
        source_root::icon(compiler, root, &[])?;
    }
    // compileMap compiles structured content before its edges, including on
    // the synthetic board root. Member fields are no longer Graph.Objects.
    let root_shape = property_text_inherited(compiler, root, &["shape"])
        .map(|value| parse_shape(Some(&value.to_lowercase())));
    if let Some(shape @ (ShapeKind::Class | ShapeKind::SqlTable)) = root_shape {
        let mut object = GraphNode::new("", Size::default());
        object.shape = shape;
        object.grid_rows = graph.root_grid_rows;
        object.grid_columns = graph.root_grid_columns;
        object.canvas_position =
            property_text_inherited(compiler, root, &["near"]).and_then(|near| {
                Some(match near.as_str() {
                    "top-left" => d2_rs_core::CanvasPosition::TopLeft,
                    "top-center" => d2_rs_core::CanvasPosition::TopCenter,
                    "top-right" => d2_rs_core::CanvasPosition::TopRight,
                    "center-left" => d2_rs_core::CanvasPosition::CenterLeft,
                    "center-right" => d2_rs_core::CanvasPosition::CenterRight,
                    "bottom-left" => d2_rs_core::CanvasPosition::BottomLeft,
                    "bottom-center" => d2_rs_core::CanvasPosition::BottomCenter,
                    "bottom-right" => d2_rs_core::CanvasPosition::BottomRight,
                    _ => return None,
                })
            });
        graph.root_object = Some(Box::new(object));
        // The source's member-child validation has already run in the IR;
        // these fields are rows, not independently measured objects.
        if retain_source_metadata {
            graph.source_metadata_mut().objects_allocated = !fields.is_empty();
        }
        fields.clear();
    }
    let actual_parents: HashSet<_> = fields
        .iter()
        .filter_map(|f| f.parent_path.clone())
        .collect();
    let mut graph_fields = Vec::with_capacity(fields.len());
    let mut object_reference_scopes = BTreeMap::<GraphNodeId, Vec<String>>::new();
    let mut note_reference_scopes = BTreeMap::<GraphNodeId, Vec<String>>::new();

    for GraphField {
        field,
        path,
        parent_path,
        source_order,
    } in fields.iter().cloned()
    {
        compiler.graph_check()?;
        if nodes.contains_key(&path) {
            return Err(GraphCompileError::DuplicateNode(path));
        }
        if !retain_source_projection {
            // The private planner needs structural objects, not Object.Text
            // or SetDimensions. IR validation above already checked attribute
            // values. Preserve image child errors, fallible intrinsic LaTeX checks
            // and icon URL errors before advancing to another object or edge.
            let graph_id = graph.add_node(GraphNode::new(path.clone(), Size::default()));
            apply_node_properties(compiler, field, &mut graph, graph_id, false);
            let shape = graph.node(graph_id).expect("new plan node").shape;
            if legend_board.is_none() && shape == ShapeKind::Image {
                if actual_parents.contains(&path)
                    && !root_sequence_diagram
                    && !has_sequence_ancestor(compiler, field)
                {
                    return Err(GraphCompileError::ImageHasChildren(path));
                }
            }
            if field_language(compiler, field).as_deref() == Some("latex") {
                let (label, _, _) = field_label_info(compiler, field, &path, caps_lock, false);
                // PlanMeasurer's intrinsic check is independent of font size;
                // only its discarded, zero-size result depends on that hook.
                measurer
                    .measure_latex_label_checked_at_size(&label, 16)
                    .map_err(GraphCompileError::LatexMeasurement)?;
            }
            if property_text_inherited(compiler, field, &["shape"])
                .is_some_and(|shape| shape.to_lowercase() == "sequence_diagram")
            {
                graph.set_sequence_diagram(graph_id, true);
            }
            if shape == ShapeKind::SqlTable {
                table_columns.insert(path.clone(), content_field_names(compiler, field));
            }
            source_root::icon(compiler, field, &[])?;
            nodes.insert(path.clone(), graph_id);
            graph_fields.push((
                GraphField {
                    field,
                    path,
                    parent_path,
                    source_order,
                },
                graph_id,
            ));
            continue;
        }
        let sequence_ancestor = root_sequence_diagram || has_sequence_ancestor(compiler, field);
        let (label, is_markdown, label_bytes) =
            field_label_info(compiler, field, &path, caps_lock, sequence_ancestor);
        let language = field_language(compiler, field);
        let is_latex = language.as_deref() == Some("latex");
        let explicit_font_size =
            property_number_inherited(compiler, field, &["style", "font-size"])
                .and_then(|value| u32::try_from(value as i64).ok());
        let shape =
            property_text_inherited(compiler, field, &["shape"]).map(|value| value.to_lowercase());
        let shape_kind = shape.as_deref().map_or_else(
            || default_shape(language.as_deref(), sequence_ancestor),
            |value| parse_shape(Some(value)),
        );
        let measure_markdown = language.is_some() && !is_latex && shape_kind != ShapeKind::Code;
        let mono_style = property_text_inherited(compiler, field, &["style", "font"])
            .map_or(theme_mono, |value| value.eq_ignore_ascii_case("mono"));
        let table_column_names = if shape.as_deref() == Some("sql_table") {
            content_field_names(compiler, field)
        } else {
            Vec::new()
        };
        // A grid declaration is a container in D2 even when its current
        // scope has no child fields. This affects the level-dependent title
        // font and therefore the measured graph boundary.
        let has_graph_children = actual_parents.contains(&path)
            && !matches!(shape_kind, ShapeKind::Class | ShapeKind::SqlTable);
        let is_container = has_graph_children
            || property_number_inherited(compiler, field, &["grid-rows"]).is_some()
            || property_number_inherited(compiler, field, &["grid-columns"]).is_some();
        if legend_board.is_none()
            && shape_kind == ShapeKind::Image
            && has_graph_children
            && !sequence_ancestor
        {
            return Err(GraphCompileError::ImageHasChildren(path.clone()));
        }
        let explicit_bold = property_text_inherited(compiler, field, &["style", "bold"])
            .is_some_and(|value| value == "true");
        let bold = if sequence_ancestor {
            // D2's Object.Text clears the ordinary non-container bold default
            // for every descendant of a sequence diagram. This includes
            // actors, nested groups, and lifeline notes.
            false
        } else if is_markdown || is_latex {
            // Markdown/LaTeX labels use their own inline styles; a plain
            // block starts in the regular face even when ordinary D2 labels
            // default to bold.
            explicit_bold
        } else {
            shape.as_deref() != Some("class")
                && (explicit_bold || (!has_graph_children && shape.as_deref() != Some("text")))
        };
        let italic = property_text_inherited(compiler, field, &["style", "italic"])
            .is_some_and(|value| value == "true");
        let is_blank_label = label.trim().is_empty()
            && !matches!(shape.as_deref(), Some("image" | "class" | "sql_table"));
        let whitespace_only_label =
            is_blank_label && !label.is_empty() && shape_kind != ShapeKind::Code;
        // Go's empty-scalar shortcut does not apply to nonempty code made
        // only of controls/spaces: its mono leading/trailing line boxes count.
        let empty_plain_label =
            is_blank_label && (shape_kind != ShapeKind::Code || label.is_empty());
        // The target applies HeaderFontAdd after resolving the base class or
        // table font, including when the user supplies style.font-size.
        let tabular_shape = matches!(shape.as_deref(), Some("class" | "sql_table"));
        let container_level = parent_path
            .as_deref()
            .map(|path| path.split('.').count() + 1)
            .unwrap_or(1);
        let effective_font_size = if tabular_shape {
            explicit_font_size.unwrap_or(20).saturating_add(4)
        } else {
            explicit_font_size
                .or_else(|| {
                    (is_container && !sequence_ancestor && shape_kind != ShapeKind::Text).then_some(
                        match container_level {
                            1 => 28,
                            2 => 24,
                            3 => 20,
                            _ => 16,
                        },
                    )
                })
                .unwrap_or(16)
        };
        let has_icon = property_text_inherited(compiler, field, &["icon"]).is_some();
        // validateKeys checks missing image icons only while visiting a local
        // shape key (d2compiler/compile.go). Attribute validation above mirrors
        // that walk; a shape inherited from a class has no such key to visit.
        // Class maps can provide link and tooltip decorations as well as
        // visual styles. SetDimensions reserves a lane for every retained
        // link paired with a tooltip, including validated board links
        // (d2graph.go:1644–1653). Dropped unresolved links reserve nothing.
        let has_link_tooltip = node_link_after_board_validation(compiler, root, field).is_some()
            && property_text_inherited(compiler, field, &["tooltip"]).is_some();
        let desired_width = property_number_inherited(compiler, field, &["width"])
            .is_some_and(|value| value != 0.0);
        let desired_height = property_number_inherited(compiler, field, &["height"])
            .is_some_and(|value| value != 0.0);
        let raw_label_size = if shape_kind == ShapeKind::SqlTable && label.is_empty() {
            if language.as_deref().is_some_and(|value| !value.is_empty()) {
                measurer.measure_code_label_at_size("Table", effective_font_size)
            } else if mono_style {
                measurer.measure_mono_styled_label_at_size(
                    "Table",
                    effective_font_size,
                    bold,
                    italic,
                )
            } else {
                measurer.measure_styled_label_at_size("Table", effective_font_size, bold, italic)
            }
        } else if shape_kind == ShapeKind::Class {
            // Class.Text retains Language and italic, but clears bold. The
            // Go ruler routes nonempty languages through its code branch.
            if language.as_deref().is_some_and(|value| !value.is_empty()) {
                measurer.measure_code_label_at_size(&label, effective_font_size)
            } else {
                measurer.measure_mono_styled_label_at_size(
                    &label,
                    effective_font_size,
                    false,
                    italic,
                )
            }
        } else if shape_kind == ShapeKind::Code && !is_latex {
            measurer.measure_code_label_at_size(&label, effective_font_size)
        } else if mono_style && measure_markdown {
            measurer.measure_mono_styled_markdown_label_at_size(
                &label,
                effective_font_size,
                bold,
                italic,
            )
        } else if mono_style && !is_latex {
            measurer.measure_mono_styled_label_at_size(&label, effective_font_size, bold, italic)
        } else if measure_markdown {
            measurer.measure_styled_markdown_label_at_size(
                &label,
                effective_font_size,
                bold,
                italic,
            )
        } else if is_latex {
            measurer
                .measure_latex_label_checked_at_size(&label, effective_font_size)
                .map_err(GraphCompileError::LatexMeasurement)?
        } else {
            measurer.measure_styled_label_at_size(&label, effective_font_size, bold, italic)
        };
        let size = if whitespace_only_label {
            Size {
                // D2 collapses whitespace-only labels to an unlabeled
                // 45-by-45 placeholder while retaining the source scalar.
                width: 45.0,
                height: 45.0,
            }
        } else if empty_plain_label {
            // A truly empty scalar (or an implicit empty field) uses D2's
            // default 100 px shape size; only whitespace-only labels take the
            // compact placeholder path above.
            let side = if matches!(shape_kind, ShapeKind::Circle | ShapeKind::RealSquare)
                && (desired_width || desired_height)
            {
                property_number_inherited(compiler, field, &["width"])
                    .unwrap_or(0.0)
                    .max(property_number_inherited(compiler, field, &["height"]).unwrap_or(0.0))
            } else {
                100.0
            };
            Size {
                width: side,
                height: side,
            }
        } else if !measurer.supports_shape_fitting() {
            if measure_markdown {
                if mono_style {
                    measurer.measure_mono_styled_markdown_at_size(
                        &label,
                        effective_font_size,
                        bold,
                        italic,
                    )
                } else {
                    measurer.measure_styled_markdown_at_size(
                        &label,
                        effective_font_size,
                        bold,
                        italic,
                    )
                }
            } else if is_latex {
                measurer
                    .measure_latex_checked_at_size(&label, effective_font_size)
                    .map_err(GraphCompileError::LatexMeasurement)?
            } else if mono_style && shape_kind != ShapeKind::Code {
                measurer.measure_mono_styled_at_size(&label, effective_font_size, bold, italic)
            } else {
                measurer.measure_styled_at_size(&label, effective_font_size, bold, italic)
            }
        } else {
            fit_source_shape(ShapeFitOptions {
                shape: shape_kind,
                raw_label: raw_label_size,
                font_size: effective_font_size,
                include_inner_label_padding: !desired_width
                    && !desired_height
                    && shape_kind != ShapeKind::Text,
                desired_width,
                desired_height,
                has_icon,
                has_link_tooltip,
            })
        };
        let label_size = if shape_kind == ShapeKind::SqlTable && label.is_empty() {
            raw_label_size
        } else if empty_plain_label || (shape_kind == ShapeKind::Image && label.is_empty()) {
            Size::default()
        } else if shape_kind == ShapeKind::Class {
            raw_label_size
        } else if mono_style && measure_markdown {
            measurer.measure_mono_styled_markdown_label_at_size(
                &label,
                effective_font_size,
                bold,
                italic,
            )
        } else if mono_style && !is_latex && shape_kind != ShapeKind::Code {
            measurer.measure_mono_styled_label_at_size(&label, effective_font_size, bold, italic)
        } else if shape_kind == ShapeKind::Code {
            raw_label_size
        } else if measure_markdown {
            measurer.measure_styled_markdown_label_at_size(
                &label,
                effective_font_size,
                bold,
                italic,
            )
        } else if is_latex {
            measurer
                .measure_latex_label_checked_at_size(&label, effective_font_size)
                .map_err(GraphCompileError::LatexMeasurement)?
        } else {
            measurer.measure_styled_label_at_size(&label, effective_font_size, bold, italic)
        };
        let mut node = GraphNode::new(path.clone(), size);
        let (table_max_name_width, table_target_columns) = if shape_kind == ShapeKind::SqlTable {
            let dimensions =
                sql_table_dimensions(compiler, field, label_size, measurer, theme_mono);
            node.size = dimensions.size;
            (dimensions.max_name_width, dimensions.columns)
        } else if shape_kind == ShapeKind::Class {
            node.size = class_dimensions(compiler, field, label_size, measurer);
            (0, Vec::new())
        } else {
            (0, Vec::new())
        };
        node.declared_size = Some(node.size);
        // Structured targets keep the source/base font in the graph when it
        // was explicitly declared; their header adjustment is a target
        // rendering concern. With no explicit declaration retain the
        // effective default so downstream layout/render code still has a
        // concrete value.
        node.font_size = Some(if tabular_shape {
            explicit_font_size.unwrap_or(effective_font_size)
        } else {
            effective_font_size
        });
        let graph_id = graph.add_node(node);
        if retain_source_projection {
            let classes = target_class_names(compiler, field);
            if !classes.is_empty() {
                graph.target_node_classes.insert(graph_id, classes);
            }
        }
        graph
            .node_font_size_declarations
            .insert(graph_id, explicit_font_size);
        graph.set_node_has_language(graph_id, field_language(compiler, field).is_some());
        // Preserve an explicitly empty source label in graph metadata. Layout
        // adapters must distinguish it from an unspecified label/default ID.
        graph.set_node_label_bytes(graph_id, label_bytes);
        if !label.is_empty() {
            labels.insert(graph_id, label);
        }
        if retain_source_projection {
            let pretty_link_bytes = node_pretty_link_after_board_validation(compiler, root, field);
            let pretty_link = pretty_link_bytes
                .as_ref()
                .map(|value| value.to_text().into_owned());
            let content_aspect_ratio_bits = if shape_kind == ShapeKind::Cloud && !empty_plain_label
            {
                // SizeToContent constructs its temporary cloud from GetDefaultSize,
                // not from the fitted/layout box. GetInnerBoxForContent scales that
                // original box with the cloud's selected content coefficients.
                let padding = if !desired_width && !desired_height {
                    5.0
                } else {
                    0.0
                };
                let mut width = raw_label_size.width + padding;
                let mut height = raw_label_size.height + padding;
                let aspect = width / height;
                if aspect > (1.0 + 0.819 / 0.548) / 2.0 {
                    width *= 0.819;
                    height *= 0.548;
                } else if aspect < (1.0 + 0.549 / 0.820) / 2.0 {
                    width *= 0.549;
                    height *= 0.820;
                } else {
                    width *= 0.663;
                    height *= 0.663;
                }
                Some((width / height).to_bits())
            } else {
                None
            };
            styles.insert(
                graph_id,
                SourceNodeStyle {
                    fill: property_text_inherited(compiler, field, &["style", "fill"]),
                    fill_pattern: property_text_inherited(
                        compiler,
                        field,
                        &["style", "fill-pattern"],
                    ),
                    stroke: property_text_inherited(compiler, field, &["style", "stroke"]),
                    stroke_width: property_text_inherited(
                        compiler,
                        field,
                        &["style", "stroke-width"],
                    ),
                    opacity: property_text_inherited(compiler, field, &["style", "opacity"]),
                    stroke_dash: property_text_inherited(
                        compiler,
                        field,
                        &["style", "stroke-dash"],
                    ),
                    animated: property_text_inherited(compiler, field, &["style", "animated"]),
                    border_radius: property_text_inherited(
                        compiler,
                        field,
                        &["style", "border-radius"],
                    ),
                    shadow: property_text_inherited(compiler, field, &["style", "shadow"]),
                    three_dee: property_text_inherited(compiler, field, &["style", "3d"]),
                    multiple: property_text_inherited(compiler, field, &["style", "multiple"]),
                    double_border: property_text_inherited(
                        compiler,
                        field,
                        &["style", "double-border"],
                    ),
                    font_color: property_text_inherited(compiler, field, &["style", "font-color"]),
                    // Object.Text measures with its ordinary bold default, while
                    // exporter.applyStyles subsequently honors explicit false.
                    bold: property_text_inherited(compiler, field, &["style", "bold"])
                        .and_then(|value| d2_rs_core::parse_d2_bool(&value))
                        .unwrap_or(bold),
                    target_bold: (is_markdown || is_latex).then(|| {
                        property_text_inherited(compiler, field, &["style", "bold"])
                            .and_then(|value| d2_rs_core::parse_d2_bool(&value))
                            .unwrap_or(
                                !sequence_ancestor
                                    && shape_kind != ShapeKind::Class
                                    && !has_graph_children
                                    && shape_kind != ShapeKind::Text,
                            )
                    }),
                    // Export applies ParseBool after Object.Text measured the raw
                    // literal spelling. Preserve the independent measurement face.
                    italic: property_text_inherited(compiler, field, &["style", "italic"])
                        .and_then(|value| d2_rs_core::parse_d2_bool(&value))
                        .unwrap_or(italic),
                    underline: property_text_inherited(compiler, field, &["style", "underline"])
                        .and_then(|value| d2_rs_core::parse_d2_bool(&value))
                        .unwrap_or(false),
                    text_transform: property_text_inherited(
                        compiler,
                        field,
                        &["style", "text-transform"],
                    ),
                    // Style.Apply normalizes font names; pattern/transform spellings
                    // retain their original values in the graph.
                    font: property_text_inherited(compiler, field, &["style", "font"]).map(
                        |value| {
                            String::from_utf8(d2_rs_core::ir::go_lowercase(value.as_bytes()))
                                .expect("style font is UTF-8")
                        },
                    ),
                    filled: property_text_inherited(compiler, field, &["style", "filled"]),
                    font_size: explicit_font_size,
                    target_font_size: tabular_shape.then_some(explicit_font_size.unwrap_or(20)),
                    target_accents: tabular_shape.then(SourceTargetAccents::structured),
                    language,
                    table_columns: table_column_names.clone(),
                    table_rows: table_target_columns
                        .iter()
                        .map(|column| {
                            (
                                column.name.label.clone(),
                                column.type_text.label.clone(),
                                column
                                    .constraint
                                    .as_deref()
                                    .unwrap_or_default()
                                    .iter()
                                    .map(|value| sql_constraint_abbreviation(value))
                                    .collect::<Vec<_>>()
                                    .join(", "),
                            )
                        })
                        .collect(),
                    table_target_columns,
                    table_max_name_width,
                    class_field_underlines: if shape_kind == ShapeKind::Class {
                        class_member_underlines(compiler, field, false)
                    } else {
                        Vec::new()
                    },
                    class_method_underlines: if shape_kind == ShapeKind::Class {
                        class_member_underlines(compiler, field, true)
                    } else {
                        Vec::new()
                    },
                    class_fields: if shape_kind == ShapeKind::Class {
                        class_field_rows(compiler, field)
                    } else {
                        Vec::new()
                    },
                    class_methods: if shape_kind == ShapeKind::Class {
                        class_method_rows(compiler, field)
                    } else {
                        Vec::new()
                    },
                    markdown: is_markdown,
                    link: node_link_after_board_validation(compiler, root, field),
                    pretty_link,
                    pretty_link_bytes,
                    content_aspect_ratio_bits,
                    tooltip: property_text_inherited(compiler, field, &["tooltip"]),
                    tooltip_position: property_text_inherited(
                        compiler,
                        field,
                        &["tooltip", "near"],
                    ),
                    // Icons participate in the same class inheritance chain as
                    // shape and style properties. A direct lookup left
                    // class-provided icons visible to sizing (`has_icon`) but
                    // dropped them at the target/render boundary.
                    icon: property_text_inherited(compiler, field, &["icon"]),
                    icon_border_radius: source_icon_border_radius(compiler, field),
                },
            );
        }
        nodes.insert(path.clone(), graph_id);
        apply_node_properties(
            compiler,
            field,
            &mut graph,
            graph_id,
            retain_source_projection,
        );
        if shape.as_deref() == Some("sequence_diagram") {
            graph.set_sequence_diagram(graph_id, true);
        }
        if shape.as_deref() == Some("sql_table") {
            table_columns.insert(path.clone(), content_field_names(compiler, field));
        }
        // Keep measured dimensions on text nodes as well.  D2's GraphJSON
        // exposes `labelDimensions` for text/LaTeX/Markdown objects even
        // though placement treats their box as the label itself.
        graph.node_mut(graph_id).expect("new graph node").label_size = Some(label_size);
        let is_container = !matches!(
            graph.node(graph_id).map(|node| node.shape),
            Some(ShapeKind::Class | ShapeKind::SqlTable)
        ) && is_container;
        let node = graph.node_mut(graph_id).expect("new graph node");
        node.content_insets =
            source_content_insets(node.shape, is_container, node.has_icon, node.icon_position);
        if let Some(attrs) = node
            .d2_authored_geometry
            .as_mut()
            .and_then(|a| a.attributes.as_mut())
        {
            attrs.attributes.label_dimensions = [label_size.width as i64, label_size.height as i64];
            attrs.label_position = (node.label_position != LabelPosition::Unset)
                .then(|| node.label_position.d2_name().as_bytes().to_vec());
            attrs.icon_position = node
                .icon_position
                .filter(|p| *p != LabelPosition::Unset)
                .map(|p| p.d2_name().as_bytes().to_vec());
        }
        // d2graph.SetDimensions only materializes authored positions when
        // HasLabel/Icon is true. Keep authored validation attributes above;
        // absent content must not carry those positions into layout.
        if !labels.contains_key(&graph_id)
            || matches!(
                node.shape,
                ShapeKind::Text | ShapeKind::Code | ShapeKind::Class | ShapeKind::SqlTable
            )
        {
            node.label_position = LabelPosition::Unset;
        }
        if !node.has_icon {
            node.icon_position = None;
        }
        let has_nonempty_label = node
            .label_size
            .is_some_and(|size| size.width > 0.0 && size.height > 0.0);
        node.port_spread = if is_container {
            0.0
        } else if node.shape == ShapeKind::Image {
            5.0
        } else if has_nonempty_label {
            12.0
        } else {
            0.0
        };
        node.external_label = node
            .label_size
            .filter(|size| size.width > 0.0 && size.height > 0.0)
            .and_then(|label_size| {
                explicit_external_label(node.label_position, label_size).or_else(|| {
                    // Source passes every authored label.near position to TALA
                    // as fixed. An automatic outside carrier must not erase
                    // that provenance merely because the shape is undersized
                    // or normally prefers an external label.
                    (node.label_position == LabelPosition::Unset)
                        .then(|| {
                            automatic_external_label(
                                node.shape,
                                is_container,
                                node.fixed_height && node.size.height < label_size.height + 20.0,
                                label_size,
                            )
                        })
                        .flatten()
                })
            });
        if node.shape == ShapeKind::SqlTable {
            graph.set_table_column_count(graph_id, content_field_count(compiler, field));
            graph
                .table_column_names
                .insert(graph_id, table_column_names.clone());
        }
        if retain_source_projection {
            let has_icon = graph.node(graph_id).is_some_and(|node| node.has_icon);
            graph.set_node_invisible(
                graph_id,
                source_node_is_invisible(compiler, field, has_icon),
            );
        }
        let scopes = object_reference_scopes.entry(graph_id).or_default();
        let note_scopes = note_reference_scopes.entry(graph_id).or_default();
        if let IrNode::Field(source_field) = compiler.arena.node(field) {
            for reference in source_field.references.iter().flatten() {
                let scope_map = reference
                    .read()
                    .context
                    .as_ref()
                    .and_then(|context| context.read().scope_map);
                let scope_path =
                    if let Some(scope_map) = scope_map.filter(|map| !compiler.arena.is_var(*map)) {
                        crate::ir::board_ida(&compiler.arena, scope_map)
                            .and_then(|parts| path_text(&parts))
                    } else {
                        parent_path.clone()
                    };
                if let Some(scope) = scope_path {
                    if !reference
                        .read()
                        .key_path
                        .as_ref()
                        .is_some_and(|key| key.read().has_glob())
                    {
                        note_scopes.push(scope.clone());
                    }
                    scopes.push(scope);
                }
            }
        }
        let has_edge_reference = sequence_object_has_edge_reference(compiler, field);
        graph
            .sequence_node_edge_refs
            .entry(graph_id)
            .and_modify(|previous| *previous |= has_edge_reference)
            .or_insert(has_edge_reference);
        if let Some(line) = sequence_object_line(compiler, field) {
            graph
                .sequence_node_lines
                .entry(graph_id)
                .and_modify(|previous| *previous = (*previous).min(line))
                .or_insert(line);
        }
        if retain_source_metadata {
            source_objects::insert(
                compiler,
                field,
                &mut graph,
                graph_id,
                labels.get(&graph_id).map(String::as_str).unwrap_or(""),
                styles.get(&graph_id).expect("compiled source style"),
                source_keys.get_or_insert_with(|| Box::new(source_objects::KeyPool::default())),
            )?;
        } else {
            source_root::icon(compiler, field, &[])?;
        }
        graph_fields.push((
            GraphField {
                field,
                path,
                parent_path,
                source_order,
            },
            graph_id,
        ));
    }

    let compiler = original_compiler;
    for (lexical, canonical) in aliases {
        if let Some(id) = nodes.get(&canonical).copied() {
            nodes.insert(lexical, id);
        }
    }
    graph.sequence_object_scopes = object_reference_scopes
        .into_iter()
        .map(|(id, scopes)| {
            (
                id,
                scopes
                    .into_iter()
                    .filter_map(|scope| nodes.get(&scope).copied())
                    .collect(),
            )
        })
        .collect();
    graph.sequence_note_scopes = note_reference_scopes
        .into_iter()
        .map(|(id, scopes)| {
            (
                id,
                scopes
                    .into_iter()
                    .filter_map(|scope| nodes.get(&scope).copied())
                    .collect(),
            )
        })
        .collect();
    let mut hierarchy_order: BTreeMap<Option<GraphNodeId>, Vec<GraphNodeId>> = BTreeMap::new();
    for (GraphField { parent_path, .. }, graph_id) in &graph_fields {
        compiler.graph_check()?;
        let parent = parent_path
            .as_deref()
            .and_then(|parent| nodes.get(parent).copied());
        graph.node_mut(*graph_id).expect("new graph node").parent = parent;
        hierarchy_order.entry(parent).or_default().push(*graph_id);
    }
    graph.set_hierarchy_children_order(hierarchy_order);
    let source_levels = graph
        .nodes()
        .map(|(id, node)| {
            let mut level = 1;
            let mut parent = node.parent;
            while let Some(candidate) = parent {
                level += 1;
                parent = graph.node(candidate).and_then(|node| node.parent);
            }
            (id, level)
        })
        .collect();
    graph.node_label_levels = source_levels;

    let mut near_inputs = Vec::new();
    collect_near_assignments(compiler, root, None, None, &nodes, true, &mut near_inputs);
    let mut graph_validation_errors =
        super::near_validation::apply_and_validate(&mut graph, &near_inputs);
    // Original compile.validateKeys checks presence, including `3d: false`,
    // against the root's authored shape before export drops that shape.
    if property_scalar(compiler, root, &["style", "3d"]).is_some()
        && !property_text_inherited(compiler, root, &["shape"])
            .map(|value| value.to_lowercase())
            .is_some_and(|shape| {
                ["square", "rectangle", "hexagon"]
                    .iter()
                    .any(|allowed| shape.eq_ignore_ascii_case(allowed))
            })
    {
        let names = ["style", "3d"].map(|part| ast::raw_string_box(part, true));
        let maps = compiler.arena.map(root).into_iter();
        if let Some(key) = maps
            .filter_map(|map| compiler.arena.get_field(map, &names))
            .find_map(|field| compiler.arena.last_primary_key(field))
        {
            graph_validation_errors.push(super::edge_creation::error(
                &Child::Key(key),
                "key \"3d\" can only be applied to squares, rectangles, and hexagons",
            ));
        }
    }
    if property_scalar(compiler, root, &["style", "double-border"]).is_some()
        && property_text_inherited(compiler, root, &["shape"])
            .map(|value| value.to_lowercase())
            .is_some_and(|shape| {
                !shape.is_empty()
                    && !["square", "rectangle", "circle", "oval"]
                        .iter()
                        .any(|allowed| shape.eq_ignore_ascii_case(allowed))
            })
    {
        let names = ["style", "double-border"].map(|part| ast::raw_string_box(part, true));
        if let Some(key) = compiler
            .arena
            .map(root)
            .and_then(|map| compiler.arena.get_field(map, &names))
            .and_then(|field| compiler.arena.last_primary_key(field))
        {
            graph_validation_errors.push(super::edge_creation::error(
                &Child::Key(key),
                "key \"double-border\" can only be applied to squares, rectangles, circles, ovals",
            ));
        }
    }

    let edges = ordered_graph_edges(compiler, root);
    let resolve_sequence =
        graph.root_sequence_diagram || graph.sequence_diagram_nodes.contains(&true);
    // Preserve every substituted occurrence in original compilation order.
    let mut validation_edges = vec![None; edges.len()];
    for CollectedGraphEdge {
        edge,
        parent_path,
        validation_index,
    } in edges
    {
        compiler.graph_check()?;
        let visible_sequence_nodes = resolve_sequence.then(|| {
            parent_path
                .as_ref()
                .and_then(|path| sequence_scope_ends.get(path))
                .copied()
                .unwrap_or(usize::MAX)
        });
        let IrNode::Edge(edge_node) = compiler.arena.node(edge) else {
            continue;
        };
        let edge_id = edge_node
            .id
            .as_ref()
            .expect("compiled edge has an identity")
            .read()
            .clone();
        let source_path = resolve_endpoint_at(
            parent_path.as_deref(),
            endpoint_path(&edge_id.src_path)?,
            &nodes,
            &graph,
            &table_columns,
        );
        let target_path = resolve_endpoint_at(
            parent_path.as_deref(),
            endpoint_path(&edge_id.dst_path)?,
            &nodes,
            &graph,
            &table_columns,
        );
        let (source, source_column) = if let Some(visible_nodes) = visible_sequence_nodes {
            ensure_sequence_endpoint(
                &source_path,
                &mut nodes,
                &table_columns,
                &mut graph,
                visible_nodes,
                &mut labels,
                measurer,
                theme_mono,
            )
        } else {
            resolve_structured_endpoint(&source_path, &nodes, &table_columns, &graph)
        }
        .unwrap_or_else(|| {
            let id = add_implicit_node(
                &mut graph,
                &mut nodes,
                &source_path,
                source_path.rsplit('.').next().unwrap_or(&source_path),
                measurer,
                theme_mono,
            );
            attach_implicit_parent(&mut graph, &nodes, &source_path, id);
            labels.insert(
                id,
                source_path
                    .rsplit('.')
                    .next()
                    .unwrap_or(&source_path)
                    .to_owned(),
            );
            (id, None)
        });
        let (target, target_column) = if let Some(visible_nodes) = visible_sequence_nodes {
            ensure_sequence_endpoint(
                &target_path,
                &mut nodes,
                &table_columns,
                &mut graph,
                visible_nodes,
                &mut labels,
                measurer,
                theme_mono,
            )
        } else {
            resolve_structured_endpoint(&target_path, &nodes, &table_columns, &graph)
        }
        .unwrap_or_else(|| {
            let id = add_implicit_node(
                &mut graph,
                &mut nodes,
                &target_path,
                target_path.rsplit('.').next().unwrap_or(&target_path),
                measurer,
                theme_mono,
            );
            attach_implicit_parent(&mut graph, &nodes, &target_path, id);
            labels.insert(
                id,
                target_path
                    .rsplit('.')
                    .next()
                    .unwrap_or(&target_path)
                    .to_owned(),
            );
            (id, None)
        });
        let graph_edge = graph.add_edge(GraphEdge { source, target });
        let classes = target_class_names(compiler, edge);
        if !classes.is_empty() {
            graph.target_edge_classes.insert(graph_edge, classes);
        }
        if let Some(line) = sequence_edge_line(compiler, edge) {
            graph.sequence_edge_lines.insert(graph_edge, line);
        }
        validation_edges[validation_index] = Some((edge, graph_edge));
        graph.set_edge_link(
            graph_edge,
            property_text_inherited(compiler, edge, &["link"]),
        );
        graph.set_edge_tooltip(
            graph_edge,
            property_text_inherited(compiler, edge, &["tooltip"]),
        );
        graph.set_edge_icon(
            graph_edge,
            property_text(compiler, edge, &["icon"]),
            property_text_inherited(compiler, edge, &["icon-position"])
                .or_else(|| property_text_inherited(compiler, edge, &["icon", "near"]))
                .map(|value| LabelPosition::from_d2_name(&value)),
            source_icon_border_radius(compiler, edge)
                .as_deref()
                .and_then(d2_rs_core::parse_d2_float)
                .unwrap_or(0.0),
        );
        // Original compileEdge keeps every reference's ScopeMap/BoardIDA,
        // which can differ from the canonical IR edge owner after `_` paths
        // resolve outward. Group containment follows those declaration scopes.
        let mut reference_scopes = std::collections::BTreeSet::new();
        if let Some(references) = edge_node.references.as_ref() {
            for reference in references {
                let scope_map = reference
                    .read()
                    .context
                    .as_ref()
                    .and_then(|context| context.read().scope_map);
                let scope_path =
                    if let Some(scope_map) = scope_map.filter(|map| !compiler.arena.is_var(*map)) {
                        crate::ir::board_ida(&compiler.arena, scope_map)
                            .and_then(|parts| path_text(&parts))
                    } else {
                        parent_path.clone()
                    };
                if let Some(scope) = scope_path
                    .as_deref()
                    .and_then(|path| nodes.get(path).copied())
                {
                    reference_scopes.insert(scope);
                }
            }
        } else if let Some(scope) = parent_path
            .as_deref()
            .and_then(|path| nodes.get(path).copied())
        {
            reference_scopes.insert(scope);
        }
        for scope in reference_scopes {
            graph
                .sequence_group_edges
                .entry(scope)
                .or_default()
                .push(graph_edge);
        }
        graph.set_edge_table_columns(
            graph_edge,
            EdgeTableColumns {
                source: source_column,
                target: target_column,
            },
        );
        graph.set_edge_arrows(
            graph_edge,
            EdgeArrows {
                source: edge_id.src_arrow,
                target: edge_id.dst_arrow,
            },
        );
        graph.set_edge_arrowheads(
            graph_edge,
            EdgeArrowheads {
                source: arrowhead_shape(compiler, edge, "source-arrowhead"),
                target: arrowhead_shape(compiler, edge, "target-arrowhead"),
            },
        );
        graph.set_edge_arrowhead_declarations(
            graph_edge,
            EdgeArrowheadDeclarations {
                source: arrowhead_declared_shape(compiler, edge, "source-arrowhead"),
                target: arrowhead_declared_shape(compiler, edge, "target-arrowhead"),
            },
        );
        let primary_label_scalar =
            edge_node
                .primary
                .and_then(|primary| match compiler.arena.node(primary) {
                    IrNode::Scalar(scalar) => Some(scalar.value.clone()),
                    _ => None,
                });
        let reserved_label_scalar = property_scalar_inherited(compiler, edge, &["label"]);
        let edge_language = field_language(compiler, edge);
        graph.set_edge_style(
            graph_edge,
            EdgeStyle {
                language: edge_language.clone(),
                opacity: property_text_inherited(compiler, edge, &["style", "opacity"]),
                stroke: property_text_inherited(compiler, edge, &["style", "stroke"]),
                stroke_width: property_text_inherited(compiler, edge, &["style", "stroke-width"]),
                stroke_dash: property_text_inherited(compiler, edge, &["style", "stroke-dash"]),
                border_radius: property_text_inherited(compiler, edge, &["style", "border-radius"]),
                animated: property_text_inherited(compiler, edge, &["style", "animated"]),
                font_color: property_text_inherited(compiler, edge, &["style", "font-color"]),
                // Style.Apply normalizes font names; pattern/transform spellings
                // retain their original values in the graph.
                font: property_text_inherited(compiler, edge, &["style", "font"]).map(|value| {
                    String::from_utf8(d2_rs_core::ir::go_lowercase(value.as_bytes()))
                        .expect("style font is UTF-8")
                }),
                italic: property_text_inherited(compiler, edge, &["style", "italic"]),
                underline: property_text_inherited(compiler, edge, &["style", "underline"]),
                text_transform: property_text_inherited(
                    compiler,
                    edge,
                    &["style", "text-transform"],
                ),
                fill: property_text_inherited(compiler, edge, &["style", "fill"]),
                filled: property_text_inherited(compiler, edge, &["style", "filled"]),
            },
        );
        let edge_font_size = property_number_inherited(compiler, edge, &["style", "font-size"])
            .and_then(|value| u32::try_from(value as i64).ok());
        graph.set_edge_font_size(graph_edge, edge_font_size.unwrap_or(16));
        // D2's Edge.Text uses italic text by default (including sequence
        // messages), and switches to the monospace family when
        // `style.font: mono` is present. Keep that rendered text contract in
        // the measured graph so layout sees the same label advances as the
        // v0.9.0 ruler; ordinary node labels retain their own bold defaults.
        let edge_bold = property_text_inherited(compiler, edge, &["style", "bold"])
            .and_then(|value| d2_rs_core::parse_d2_bool(&value))
            .unwrap_or(false);
        graph.set_edge_label_bold(graph_edge, edge_bold);
        let edge_mono = property_text_inherited(compiler, edge, &["style", "font"])
            .map_or(theme_mono, |value| value.eq_ignore_ascii_case("mono"));
        // d2lib.compileBoard skips SetDimensions together with layout when
        // Graph.Objects is empty, even if synthetic-root edges remain.
        let measure_labels = !graph.nodes.is_empty();
        let measure_edge_label = |text: &str| {
            if !measure_labels {
                return Size::default();
            }
            let font_size = edge_font_size.unwrap_or(16);
            if edge_mono {
                measurer.measure_mono_styled_label_at_size(text, font_size, edge_bold, true)
            } else {
                measurer.measure_styled_label_at_size(text, font_size, edge_bold, true)
            }
        };
        let arrowhead_label = |name: &str| {
            // Each class map is compiled in order, followed by the direct
            // edge map. Within one arrowhead, its body label follows primary.
            let names = [ast::raw_string_box(name, true)];
            let mut label = None;
            for map in class_maps(compiler, edge)
                .iter()
                .copied()
                .chain(compiler.arena.map(edge))
            {
                if let Some(field) = compiler.arena.get_field(map, &names) {
                    if let Some(primary) = compiler.arena.primary(field)
                        && let IrNode::Scalar(value) = compiler.arena.node(primary)
                    {
                        label = Some(scalar_text(&value.value));
                    }
                    if let Some(value) = property_text(compiler, field, &["label"]) {
                        label = Some(value);
                    }
                }
            }
            label.filter(|text| !text.is_empty())
        };
        let edge_transform = property_text_inherited(compiler, edge, &["style", "text-transform"]);
        let edge_transform = if measure_labels { edge_transform } else { None };
        let source_arrowhead_label = arrowhead_label("source-arrowhead")
            .map(|text| transform_text(text, edge_transform.as_deref()));
        let target_arrowhead_label = arrowhead_label("target-arrowhead")
            .map(|text| transform_text(text, edge_transform.as_deref()));
        let source_arrowhead_color =
            property_text_inherited(compiler, edge, &["source-arrowhead", "style", "font-color"]);
        let target_arrowhead_color =
            property_text_inherited(compiler, edge, &["target-arrowhead", "style", "font-color"]);
        graph.set_edge_arrowhead_labels(
            graph_edge,
            EdgeArrowheadLabels {
                source: source_arrowhead_label.map(|text| ArrowheadLabel {
                    size: measure_edge_label(&text),
                    text,
                    color: source_arrowhead_color.clone(),
                }),
                target: target_arrowhead_label.map(|text| ArrowheadLabel {
                    size: measure_edge_label(&text),
                    text,
                    color: target_arrowhead_color.clone(),
                }),
            },
        );
        // D2's compiler promotes a link to the edge label when the edge has
        // no explicit label.  This is observable in the target renderer: a
        // bare `a -> b: { link: https://… }` draws the URL as an underlined
        // connection label while retaining the link metadata.  An explicit
        // `label: ""` remains intentionally empty, so preserve `Some("")`
        // from the property lookup and only fall back when no label field was
        // supplied at all.
        let edge_label_text = reserved_label_scalar
            .as_ref()
            .or(primary_label_scalar.as_ref())
            .map(scalar_text)
            .or_else(|| property_text_inherited(compiler, edge, &["link"]));
        if let Some(text) = edge_label_text.map(|text| {
            transform_with_theme(
                text,
                edge_transform.as_deref(),
                caps_lock && measure_labels,
                false,
                edge_language.as_deref(),
            )
        }) && !text.is_empty()
        {
            let size = if !measure_labels {
                Size::default()
            } else if edge_language.as_deref() == Some("latex") {
                measurer
                    .measure_latex_label_checked_at_size(&text, edge_font_size.unwrap_or(16))
                    .map_err(GraphCompileError::LatexMeasurement)?
            } else if edge_language.as_deref() == Some("markdown") {
                if edge_mono {
                    measurer.measure_mono_styled_markdown_label_at_size(
                        &text,
                        edge_font_size.unwrap_or(16),
                        edge_bold,
                        true,
                    )
                } else {
                    measurer.measure_styled_markdown_label_at_size(
                        &text,
                        edge_font_size.unwrap_or(16),
                        edge_bold,
                        true,
                    )
                }
            } else if edge_language
                .as_deref()
                .is_some_and(|language| !language.is_empty())
            {
                measurer.measure_code_label_at_size(&text, edge_font_size.unwrap_or(16))
            } else {
                measure_edge_label(&text)
            };
            let position = property_text(compiler, edge, &["label-position"])
                .map(|value| LabelPosition::from_d2_name(&value))
                .unwrap_or_default();
            let authored_percentage = property_number(compiler, edge, &["label-percentage"]);
            let percentage = authored_percentage.unwrap_or(0.0).clamp(0.0, 1.0);
            graph.set_edge_label(
                graph_edge,
                Some(EdgeLabel {
                    text,
                    size,
                    position,
                    percentage,
                }),
            );
            graph.set_edge_label_percentage_present(graph_edge, authored_percentage.is_some());
        }
        if retain_source_projection {
            let declarations = graph.edge_arrowhead_declarations(graph_edge);
            let arrow_labels = graph.edge_arrowhead_labels(graph_edge);
            let arrow_attributes =
                |name: &str, declaration: &Option<String>, label: &Option<ArrowheadLabel>| {
                    let attrs = d2_validation_attributes(
                        compiler,
                        edge,
                        &[name],
                        label.as_ref().map(|v| v.size),
                    );
                    (declaration.is_some() || label.is_some() || !attrs.style.is_empty())
                        .then_some(attrs)
                };
            graph.set_d2_edge_validation(
                graph_edge,
                d2_rs_core::D2EdgeValidation {
                    nil_route_points: Default::default(),
                    external_id: None,
                    attributes: d2_validation_attributes(
                        compiler,
                        edge,
                        &[],
                        graph.edge_label(graph_edge).map(|v| v.size),
                    ),
                    label_position: graph
                        .edge_label(graph_edge)
                        .filter(|v| v.position != LabelPosition::Unset)
                        .map(|v| v.position.d2_name().as_bytes().to_vec()),
                    label_percentage: property_number(compiler, edge, &["label-percentage"]),
                    source_arrowhead: arrow_attributes(
                        "source-arrowhead",
                        &declarations.source,
                        &arrow_labels.source,
                    ),
                    target_arrowhead: arrow_attributes(
                        "target-arrowhead",
                        &declarations.target,
                        &arrow_labels.target,
                    ),
                    source_table_column: source_column.map(|v| v as i64),
                    target_table_column: target_column.map(|v| v as i64),
                    endpoints_reversed: false,
                },
            );
        }
        if retain_source_metadata {
            let source_edge = source_objects::edge(compiler, edge, &graph, graph_edge)?;
            graph
                .source_metadata_mut()
                .edges
                .insert(graph_edge, source_edge);
        } else {
            source_root::icon(compiler, edge, &[])?;
            for name in ["source-arrowhead", "target-arrowhead"] {
                if property_present(compiler, edge, &[name]) {
                    source_root::icon(compiler, edge, &[name])?;
                }
            }
        }
    }
    let validation_edges = validation_edges.into_iter().flatten().collect::<Vec<_>>();
    // Connect.initIndex precedes SortEdgesByAST. Count canonical graph
    // endpoints in the original compileMap order, including collapsed
    // table/class members and substituted occurrences sharing one IR edge.
    for (position, (_, edge)) in validation_edges.iter().enumerate() {
        compiler.graph_check()?;
        let index = validation_edges[..position]
            .iter()
            .filter(|(_, previous)| graph.edges_equivalent(*previous, *edge))
            .count();
        graph.set_edge_index(*edge, index);
    }
    let errors =
        super::edge_validation::validate_edge_boundaries(compiler, &graph, &validation_edges);
    for error in errors {
        if !graph_validation_errors.contains(&error) {
            graph_validation_errors.push(error);
        }
    }
    if legend_board.is_none() && !graph_validation_errors.is_empty() {
        return Err(GraphCompileError::InvalidAttributes(
            graph_validation_errors,
        ));
    }
    // Go Compile sorts the document's Graph.Objects by AST; child boards
    // retain compileMap order. ChildrenArray always retains
    // EnsureChild creation order, including nodes first created by earlier
    // nested edges. IR field order alone does not preserve that distinction.
    // Planning consumes only renderability after validation. Reconstructing
    // EnsureChild order is an export/layout projection, with no diagnostics.
    if retain_source_projection {
        // Legends use compileMap alone, so they retain their independent
        // reconstruction. Ordinary boards reuse the late-validation hierarchy.
        let source_child_order = validated_hierarchy
            .unwrap_or_else(|| super::late_validation::hierarchy_order(compiler, root))
            .into_iter()
            .map(|(parent, children)| {
                let parent = ast::ByteString::from(parent).to_text().into_owned();
                let children = children
                    .into_iter()
                    .enumerate()
                    .map(|(index, name)| {
                        (ast::ByteString::from(name).to_text().into_owned(), index)
                    })
                    .collect::<BTreeMap<_, _>>();
                (parent, children)
            })
            .collect::<BTreeMap<_, _>>();
        for (parent, children) in &mut graph.hierarchy_children_order {
            let parent = parent
                .map(|id| graph.nodes[id.0 as usize].external_id.as_str())
                .unwrap_or("");
            if let Some(order) = source_child_order.get(parent) {
                // Retain every native child. Unmapped identities remain visible
                // to graph-inventory comparisons instead of being filtered out.
                children.sort_by_key(|id| {
                    order
                        .get(&graph.nodes[id.0 as usize].external_id)
                        .copied()
                        .unwrap_or(usize::MAX)
                });
            }
        }
    }
    let mut source_node_order = graph_fields
        .iter()
        .map(|(field, graph_id)| (field.source_order, *graph_id))
        .collect::<Vec<_>>();
    if compiler.arena.root(root) {
        super::go_sort::sort_by_index(
            &mut source_node_order,
            |left_index, right_index, left, right| {
                match (left.0, right.0) {
                    (Some(left), Some(right)) => left.before(right),
                    // D2's comparator uses the live `sort.Slice` indices when either
                    // object lacks a source reference or comes from variable
                    // substitution. Go's PDQsort can move such values during the
                    // sort, so a stable Rust tie is not equivalent.
                    _ => left_index < right_index,
                }
            },
        );
    }
    let mut node_order = Vec::with_capacity(graph.nodes.len());
    let mut seen = HashSet::with_capacity(graph.nodes.len());
    for (_, id) in source_node_order {
        if seen.insert(id) {
            node_order.push(id);
        }
    }
    // Edge-only endpoint objects are appended while edges are resolved. Keep
    // them after explicit objects until their source-reference ordering is
    // represented in the graph model as well.
    for (id, _) in graph.nodes() {
        if seen.insert(id) {
            node_order.push(id);
        }
    }
    graph.set_node_order_override(node_order);
    // The native path must use the same collision-safe adapter identities as
    // wire-graph decoding. Arena indices are storage, not TALA EntityIDs.
    let node_names = graph
        .nodes()
        .map(|(_, node)| node.external_id.clone())
        .collect::<Vec<_>>();
    graph.tala_id_overrides = d2_rs_core::identity::allocate(
        &node_names
            .iter()
            .map(|name| name.as_bytes())
            .collect::<Vec<_>>(),
    )
    .map_err(|index| GraphCompileError::DuplicateNode(node_names[index].clone()))?
    .into_iter()
    .map(Some)
    .collect();
    let edge_names = graph
        .edges()
        .map(|(id, _)| {
            d2_rs_core::edge_abs_id(&graph, id).ok_or(GraphCompileError::MissingEdgeEndpoint)
        })
        .collect::<Result<Vec<_>, _>>()?;
    graph.tala_edge_ids = d2_rs_core::identity::allocate(
        &edge_names
            .iter()
            .map(|name| name.as_bytes())
            .collect::<Vec<_>>(),
    )
    .map_err(|index| GraphCompileError::DuplicateEdge(edge_names[index].clone()))?
    .into_iter()
    .map(Some)
    .collect();
    if legend_board.is_none() {
        graph.legend = lower_legend(compiler, root)?;
    }
    compiler.graph_check()?;
    Ok((graph, labels, styles))
}

fn edge_reference_position(compiler: &Compiler, edge: IrNodeId) -> Option<ast::Position> {
    let IrNode::Edge(edge_node) = compiler.arena.node(edge) else {
        return None;
    };
    // d2graph.SortEdgesByAST compares the first reference's complete edge
    // range. Equal ranges intentionally retain the graph's insertion order
    // (Go's sort.Slice is unstable, but its comparator returns false for an
    // equal range). In particular, wildcard edge expansion must keep the
    // compiler's applied-field order; sorting by resolved endpoint ranges
    // would reorder `default.* -> layout` even though the Go graph does not.
    edge_node
        .references
        .as_ref()
        .and_then(|references| references.first())
        .and_then(|reference| reference.read().context.clone())
        .and_then(|context| context.read().edge.clone())
        .map(|edge| edge.read().range.start)
}

#[cfg(test)]
mod authored_geometry_handoff_contract {
    use super::*;
    use crate::{compiler::compile_ir, parse};
    use d2_rs_core::ast::Shared;

    #[test]
    fn only_document_objects_receive_the_final_ast_sort() {
        // Reduced from the upstream sequence-layer/sequence-spread-layer
        // cases: compileMap emits group edges before the board's own edges.
        // Compile sorts the document graph, but never its child boards.
        let messages = "shape: sequence_diagram\na: me\nb: you\na -> b: first\na.note\ngroup: { a <- b: second }";
        for (source, target, expected) in [
            (messages.to_owned(), "", ["first", "second"]),
            (
                format!("layers: {{ seq: {{ {messages} }} }}"),
                "layers.seq",
                ["second", "first"],
            ),
        ] {
            let parsed = parse("sequence-order.d2", source.as_bytes(), None);
            assert!(parsed.error.is_none(), "{:?}", parsed.error);
            let (compiler, root) = compile_ir(Shared::new(parsed.map)).unwrap();
            let board =
                super::super::boards::resolve_board_target(&compiler, root, target).unwrap();
            let graph = lower_graph(&compiler, board).unwrap();
            let labels = graph
                .edges()
                .map(|(id, _)| graph.edge_label(id).unwrap().text.as_str())
                .collect::<Vec<_>>();
            assert_eq!(labels, expected, "{target}");
            let objects = graph
                .node_order_override
                .iter()
                .map(|id| graph.nodes[id.0 as usize].external_id.as_str())
                .collect::<Vec<_>>();
            assert_eq!(
                objects,
                if target.is_empty() {
                    ["a", "b", "a.note", "group"]
                } else {
                    ["a", "a.note", "b", "group"]
                },
                "{target}"
            );
        }
    }

    #[test]
    fn rejected_board_links_are_absent_from_compiler_records() {
        // Reduced from TestCompile/link-board-not-found-1. The same source
        // validation clears links back to the object's own board.
        for source in ["x.link: layers.missing", "x.link: root", "x.link: relative"] {
            let parsed = parse("link-record.d2", source.as_bytes(), None);
            let (compiler, root) = compile_ir(Shared::new(parsed.map)).unwrap();
            let (graph, _, styles) =
                lower_graph_with_measurer_and_labels(&compiler, root, &|_: &str| Size::default())
                    .unwrap();
            let id = graph.nodes().next().unwrap().0;
            assert!(styles[&id].link.is_none());
            assert!(
                graph.source_metadata.as_ref().unwrap().objects[&id].value["attributes"]
                    .get("link")
                    .is_none()
            );
        }
        let parsed = parse("link-record.d2", b"x.link: https://example.com", None);
        let (compiler, root) = compile_ir(Shared::new(parsed.map)).unwrap();
        let graph = lower_graph(&compiler, root).unwrap();
        let id = graph.nodes().next().unwrap().0;
        assert_eq!(
            graph.source_metadata.as_ref().unwrap().objects[&id].value["attributes"]["link"]["value"],
            "https://example.com"
        );
    }

    #[test]
    fn raw_dimension_and_local_identity_survive_rich_label_measurement() {
        for source in [
            "m: |md hello |\nm.width: 0",
            "parent: {\n \"local.id\": |md hello |\n \"local.id\".width: 0\n}",
            "classes: { zero: { width: 0 } }\nm: |md hello |\nm.class: zero",
        ] {
            let parsed = parse("geometry.d2", source.as_bytes(), None);
            assert!(parsed.error.is_none(), "{:?}", parsed.error);
            let (compiler, root) = compile_ir(Shared::new(parsed.map)).unwrap();
            let (graph, _, _) =
                lower_graph_with_measurer_and_labels(&compiler, root, &FallbackMeasurer).unwrap();
            let expected_id = if source.contains("local.id") {
                "parent.\"local.id\""
            } else {
                "m"
            };
            let (_, node) = graph
                .nodes()
                .find(|(_, node)| node.external_id == expected_id)
                .unwrap();
            let attrs = node.d2_authored_geometry.as_ref().unwrap();
            assert_eq!(attrs.width.as_deref(), Some(b"0".as_slice()));
            assert_eq!(
                attrs.object_id,
                if source.contains("local.id") {
                    b"\"local.id\"".to_vec()
                } else {
                    b"m".to_vec()
                }
            );
            assert!(node.size.width > 0.0);
            if source.contains("local.id") {
                let (_, parent) = graph
                    .nodes()
                    .find(|(_, node)| node.external_id == "parent")
                    .unwrap();
                let identity = parent.d2_authored_geometry.as_ref().unwrap();
                assert_eq!(identity.object_id, b"parent");
                assert!(
                    identity.width.is_none()
                        && identity.height.is_none()
                        && identity.top.is_none()
                        && identity.left.is_none()
                );
            }
        }
    }
}

#[cfg(test)]
#[cfg(feature = "oracle-fixtures")]
mod tooltip_handoff_source_contract {
    use super::*;
    use crate::{compiler::compile_ir, parse};
    use d2_rs_core::ast::Shared;

    #[test]
    fn tooltip_metadata_and_positions_match_pristine_compiler() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/tooltip-handoff-source-v090.json"
        ))
        .unwrap();
        let mut differences = Vec::new();
        for row in rows.as_array().unwrap() {
            let parsed = parse(
                "tooltip.d2",
                row["source"].as_str().unwrap().as_bytes(),
                None,
            );
            assert!(
                parsed.error.is_none(),
                "{}: {:?}",
                row["name"],
                parsed.error
            );
            let (compiler, root) = compile_ir(Shared::new(parsed.map)).unwrap();
            let lowered = lower_graph_with_measurer_and_labels(&compiler, root, &FallbackMeasurer);
            if let Err(error) = lowered {
                if error.to_string() != row["error"].as_str().unwrap() {
                    differences.push(serde_json::json!({"name":row["name"],"error":error.to_string(),"expected":row["error"]}));
                }
                continue;
            }
            let (graph, _, styles) = lowered.unwrap();
            let actual: Vec<_> = graph.nodes().map(|(id,node)| {
                let style = serde_json::to_value(&styles[&id]).unwrap();
                serde_json::json!({"id":node.external_id,"tooltip":style["tooltip"],"position":style["tooltip_position"]})
            }).collect();
            if !row["error"].as_str().unwrap().is_empty()
                || serde_json::json!(actual) != row["objects"]
            {
                differences.push(serde_json::json!({"name":row["name"],"actual":actual,"expected":row["objects"],"error":row["error"]}));
            }
        }
        assert!(
            differences.is_empty(),
            "pristine tooltip handoff differences: {differences:#?}"
        );
    }
}

#[cfg(test)]
#[cfg(feature = "oracle-fixtures")]
mod sequence_canonical_identity_source_contract {
    use super::*;
    use crate::{compiler::compile_ir, parse};
    use d2_rs_core::ast::Shared;

    #[test]
    fn complete_objects_and_edges_match_pristine_graph_compilation() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/sequence-canonical-identity-source-v090.json"
        ))
        .unwrap();
        let mut differences = Vec::new();
        for row in rows.as_array().unwrap() {
            let parsed = parse(
                "identity.d2",
                row["source"].as_str().unwrap().as_bytes(),
                None,
            );
            assert!(parsed.error.is_none());
            let (compiler, root) = compile_ir(Shared::new(parsed.map)).unwrap();
            let before = format!("{:?}", compiler.arena);
            let (graph, _, styles) =
                lower_graph_with_measurer_and_labels(&compiler, root, &FallbackMeasurer).unwrap();
            assert_eq!(
                format!("{:?}", compiler.arena),
                before,
                "lexical IR mutated"
            );
            let objects: Vec<_> = graph.nodes().map(|(id,node)| serde_json::json!({
                "id":node.external_id,
                "parent":node.parent.map(|p| graph.node(p).unwrap().external_id.clone()).unwrap_or_default(),
                "label":graph.node_labels.get(&id).map(String::as_str).unwrap_or(""),
                "fill":styles[&id].fill.as_ref().map(|v| serde_json::json!({"value":v})),
                "stroke":styles[&id].stroke.as_ref().map(|v| serde_json::json!({"value":v})),
            })).collect();
            let edges: Vec<_> = graph
                .edges()
                .map(|(id, edge)| {
                    let scopes: Vec<_> = graph
                        .sequence_group_edges
                        .iter()
                        .filter(|(_, edges)| edges.contains(&id))
                        .map(|(scope, _)| graph.node(*scope).unwrap().external_id.clone())
                        .collect();
                    serde_json::json!({"source":graph.node(edge.source).unwrap().external_id,
                    "target":graph.node(edge.target).unwrap().external_id,
                    "scopes":if scopes.is_empty() {vec![String::new()]} else {scopes}})
                })
                .collect();
            fn sorted(mut rows: Vec<serde_json::Value>) -> Vec<serde_json::Value> {
                rows.sort_by_key(|v| {
                    (
                        v["id"].to_string(),
                        v["source"].to_string(),
                        v["target"].to_string(),
                        v["scopes"].to_string(),
                    )
                });
                rows
            }
            let expected = &row["expected"];
            if sorted(objects.clone()) != sorted(expected["objects"].as_array().unwrap().clone())
                || sorted(edges.clone()) != sorted(expected["edges"].as_array().unwrap().clone())
            {
                differences.push(serde_json::json!({"name":row["name"],"objects":objects,"edges":edges,"expected":expected}));
            }
        }
        assert!(
            differences.is_empty(),
            "identity differences: {differences:#?}"
        );
    }
}

#[cfg(test)]
#[cfg(feature = "oracle-fixtures")]
mod sequence_declaration_lines_source_contract {
    use super::*;
    use crate::{compiler::compile_ir, parse};
    use d2_rs_core::ast::Shared;

    #[test]
    fn complete_reference_line_inventories_match_original_go() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/sequence-declaration-lines-source-v090.json"
        ))
        .unwrap();
        for row in rows.as_array().unwrap() {
            let parsed = parse("lines.d2", row["source"].as_str().unwrap().as_bytes(), None);
            assert!(parsed.error.is_none());
            let (compiler, root) = compile_ir(Shared::new(parsed.map)).unwrap();
            let graph = lower_graph(&compiler, root).unwrap();
            let objects: BTreeMap<_, _> = graph
                .nodes()
                .map(|(id, node)| {
                    (
                        node.external_id.clone(),
                        graph
                            .sequence_node_lines
                            .get(&id)
                            .copied()
                            .unwrap_or(i32::MAX as i64),
                    )
                })
                .collect();
            let edges: Vec<_> = graph.edges().map(|(id, edge)| serde_json::json!({
                "source":graph.node(edge.source).unwrap().external_id,
                "target":graph.node(edge.target).unwrap().external_id,
                "line":graph.sequence_edge_lines.get(&id).copied().unwrap_or(i32::MAX as i64)
            })).collect();
            assert_eq!(
                serde_json::json!(objects),
                row["objects"],
                "{} objects",
                row["name"]
            );
            assert_eq!(
                serde_json::json!(edges),
                row["edges"],
                "{} edges",
                row["name"]
            );
        }
    }
}

#[cfg(test)]
#[cfg(feature = "oracle-fixtures")]
mod raw_label_bytes_source_contract {
    use super::*;
    use crate::{compiler::compile_ir, parse};
    use d2_rs_core::ast::Shared;

    #[test]
    fn complete_raw_labels_and_transform_bytes_match_original_go() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/raw-label-bytes-source-v090.json"
        ))
        .unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 64);
        for row in rows.as_array().unwrap() {
            let parsed = parse("source.d2", row["input"].as_str().unwrap().as_bytes(), None);
            if let Some(error) = parsed.error {
                assert_eq!(error.to_string(), row["compile_error"].as_str().unwrap());
                continue;
            }
            let (compiler, root) = match compile_ir(Shared::new(parsed.map)) {
                Ok(value) => value,
                Err(error) => {
                    assert_eq!(error.to_string(), row["compile_error"].as_str().unwrap());
                    continue;
                }
            };
            let graph = match lower_graph(&compiler, root) {
                Ok(value) => value,
                Err(error) => {
                    assert_eq!(error.to_string(), row["compile_error"].as_str().unwrap());
                    continue;
                }
            };
            let objects: Vec<_> = graph
                .nodes()
                .map(|(id, node)| {
                    let bytes = graph.node_label_bytes.get(&id).unwrap();
                    serde_json::json!({"id":node.external_id,
                    "after_hex":bytes.iter().map(|b|format!("{b:02x}")).collect::<String>(),
                    "valid_utf8":std::str::from_utf8(bytes).is_ok()})
                })
                .collect();
            let expected: Vec<_> = row["objects"].as_array().unwrap().iter().map(|item|
                serde_json::json!({"id":item["id"],"after_hex":item["after_hex"],"valid_utf8":item["valid_utf8"]})).collect();
            assert_eq!(objects, expected, "{row}");
        }
    }
}

#[cfg(test)]
mod latex_measurement_source_contract {
    use super::*;
    use crate::{compiler::compile_ir, parse};
    use d2_rs_core::ast::Shared;
    #[test]
    fn responsive_tagged_math_measurement_error_precedes_fallback_geometry() {
        let source = r"x: |latex
\begin{equation}x\tag{T}\end{equation}
|";
        let parsed = parse("tagged.d2", source.as_bytes(), None);
        assert!(parsed.error.is_none());
        let (compiler, root) = compile_ir(Shared::new(parsed.map)).unwrap();
        let error = lower_graph(&compiler, root).unwrap_err();
        assert_eq!(
            error.to_string(),
            "latex failed to parse: mathjax-go: SVG dimensions not found"
        );
    }
}
