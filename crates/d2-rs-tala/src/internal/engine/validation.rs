// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Validation of public graph invariants before layout or routing.
//!
//! Dimensions, typed-ID capacity, parent references, and edge endpoints are
//! checked before an internal arena is constructed. Near relations are
//! intentionally allowed to form cycles: upstream TALA treats them as an
//! undirected proximity relation and initializes cyclic components normally.

use super::*;

/// adapter_validate.go validates the caller's selection before translateGraph
/// observes cancellation or validates geometry. Keep this order even when a
/// later check would reject the graph or an earlier reference is foreign.
pub(super) fn validate_requested_edges(
    graph: &Graph,
    requested: &[EdgeId],
) -> Result<(), LayoutError> {
    // limits.MaxEngineEdges, shared by the original adapter's two count gates.
    const MAX_D2_EDGES: usize = 50_000;
    if requested.len() > MAX_D2_EDGES {
        return Err(LayoutError::D2RequestedEdgeLimit {
            limit: MAX_D2_EDGES,
        });
    }
    let mut seen = BTreeSet::new();
    for (index, edge) in requested.iter().enumerate() {
        if !seen.insert(*edge) {
            return Err(LayoutError::D2RequestedEdgeRepeated { index });
        }
    }
    let available = graph.edges.len();
    if available > MAX_D2_EDGES {
        return Err(LayoutError::D2EdgeLimit {
            limit: MAX_D2_EDGES,
        });
    }
    if requested.len() > available {
        return Err(LayoutError::D2RequestedEdgeCount {
            requested: requested.len(),
            available,
        });
    }
    for (index, edge) in requested.iter().enumerate() {
        if edge.0 as usize >= available {
            return Err(LayoutError::D2RequestedEdgeMissing { index });
        }
    }
    Ok(())
}

// Original adapter_validate.go validates authored scalar strings independently
// of the measured box. Rich-label fitting must not erase invalid declarations.
fn validate_authored_geometry(attrs: &d2_rs_core::D2AuthoredGeometry) -> Result<(), LayoutError> {
    const MAX_COORDINATE: isize = 1_000_000_000;
    if attrs.top.is_some() != attrs.left.is_some() {
        return Err(LayoutError::D2UnpairedPosition {
            object: attrs.object_id.clone(),
        });
    }
    for (attribute, bytes, min) in [
        ("top", &attrs.top, -MAX_COORDINATE),
        ("left", &attrs.left, -MAX_COORDINATE),
        ("width", &attrs.width, 1),
        ("height", &attrs.height, 1),
    ] {
        let Some(bytes) = bytes else { continue };
        // The pinned Go oracle has 64-bit int even when Rust targets WASM32.
        let value = std::str::from_utf8(bytes)
            .ok()
            .and_then(|s| s.parse::<i64>().ok());
        let Some(value) = value else {
            return Err(LayoutError::D2InvalidIntegerAttribute {
                object: attrs.object_id.clone(),
                attribute,
                value: bytes.clone(),
            });
        };
        if value < min as i64 || value > MAX_COORDINATE as i64 {
            return Err(LayoutError::D2IntegerAttributeRange {
                object: attrs.object_id.clone(),
                attribute,
                min,
                max: MAX_COORDINATE,
            });
        }
    }
    Ok(())
}

pub(super) fn validate(graph: &Graph) -> Result<(), LayoutError> {
    validate_inner(graph, false)
}

pub(super) fn validate_existing(graph: &Graph) -> Result<(), LayoutError> {
    validate_inner(graph, true)
}

fn validate_label_dimensions(
    owner: &str,
    name: &str,
    attrs: &d2_rs_core::D2ValidationAttributes,
) -> Result<(), LayoutError> {
    if attrs.label_dimensions.iter().any(|&v| v < 0) {
        return Err(LayoutError::D2AttributeValidation(format!(
            "{owner}: {name} dimensions must be nonnegative"
        )));
    }
    if attrs.label_dimensions.iter().any(|&v| v > 1_000_000_000) {
        return Err(LayoutError::D2AttributeValidation(format!(
            "{owner}: {name} dimensions exceed supported limit 1000000000"
        )));
    }
    Ok(())
}

fn validate_style(
    owner: &str,
    attrs: &d2_rs_core::D2ValidationAttributes,
) -> Result<(), LayoutError> {
    for (name, _) in d2_rs_core::D2_VALIDATED_STYLE_KEYS {
        let Some(bytes) = attrs.style.get(*name) else {
            continue;
        };
        let text = std::str::from_utf8(bytes).ok();
        let valid = match *name {
            "opacity" => text
                .and_then(d2_rs_core::parse_d2_float)
                .is_some_and(|v| v.is_finite() && (0.0..=1.0).contains(&v)),
            "stroke-width" | "font-size" => text
                .and_then(|s| s.parse::<i64>().ok())
                .is_some_and(|v| v >= i64::from(*name == "font-size") && v <= 1_000_000_000),
            _ => text.and_then(d2_rs_core::parse_d2_bool).is_some(),
        };
        if !valid {
            let kind = if matches!(*name, "opacity" | "stroke-width" | "font-size") {
                ""
            } else {
                " boolean"
            };
            return Err(LayoutError::D2AttributeValidation(format!(
                "{owner} has invalid {name}{kind} {}",
                d2_rs_core::quote_go_bytes(bytes)
            )));
        }
    }
    Ok(())
}

fn validate_label_position(
    owner: &str,
    name: &str,
    raw: &Option<Vec<u8>>,
    edge: bool,
) -> Result<(), LayoutError> {
    let Some(bytes) = raw else { return Ok(()) };
    use d2_rs_core::LabelPosition::*;
    let position = std::str::from_utf8(bytes)
        .ok()
        .map(d2_rs_core::LabelPosition::from_d2_name)
        .unwrap_or(Unset);
    let valid = bytes.as_slice() == position.d2_name().as_bytes()
        && if edge {
            matches!(
                position,
                OutsideTopLeft
                    | OutsideTopCenter
                    | OutsideTopRight
                    | InsideMiddleLeft
                    | InsideMiddleCenter
                    | InsideMiddleRight
                    | OutsideBottomLeft
                    | OutsideBottomCenter
                    | OutsideBottomRight
                    | UnlockedTop
                    | UnlockedMiddle
                    | UnlockedBottom
            )
        } else {
            !matches!(
                position,
                Unset | UnlockedTop | UnlockedMiddle | UnlockedBottom
            )
        };
    if !valid {
        return Err(LayoutError::D2AttributeValidation(format!(
            "{owner} has invalid {name} position {}",
            d2_rs_core::quote_go_bytes(bytes)
        )));
    }
    Ok(())
}

fn validate_inner(graph: &Graph, require_position: bool) -> Result<(), LayoutError> {
    if let Some(message) = &graph.d2_boundary_errors.topology {
        return Err(LayoutError::D2GraphValidation(message.clone()));
    }
    if graph.nodes.len() > u32::MAX as usize || graph.edges.len() > u32::MAX as usize {
        return Err(LayoutError::IdOverflow);
    }
    // D2 object diagnostics follow ChildrenArray pre-order, not Graph.Objects
    // allocation order. Keep the generic Rust graph boundary's existing order.
    let mut node_order: Vec<_> = graph.nodes().map(|(id, _)| id).collect();
    if require_position
        || graph
            .nodes()
            .any(|(_, node)| node.d2_authored_geometry.is_some())
    {
        let visits = d2_object_visits(graph);
        let visited: BTreeSet<_> = visits.iter().copied().collect();
        node_order.retain(|id| !visited.contains(id));
        node_order.splice(..0, visits);
    }
    let mut table_columns = 0;
    for (object_index, id) in node_order.into_iter().enumerate() {
        let node = graph.node(id).expect("allocated node");
        if require_position || node.d2_authored_geometry.is_some() {
            // validateD2Objects checks the Box before raw scalar declarations
            // for both entrypoints. Fresh layout discards stored positions
            // only after validation; they must not become placement hints.
            let object = || {
                node.d2_authored_geometry.as_ref().map_or_else(
                    || node.external_id.as_bytes().to_vec(),
                    |attrs| attrs.object_id.clone(),
                )
            };
            if node
                .d2_authored_geometry
                .as_ref()
                .and_then(|a| a.attributes.as_ref())
                .is_some_and(|a| a.missing_box)
            {
                return Err(LayoutError::D2AttributeValidation(format!(
                    "D2 object {object_index} ({}) has no box",
                    d2_rs_core::quote_go_bytes(&object())
                )));
            }
            let input_box = node
                .d2_authored_geometry
                .as_ref()
                .and_then(|attrs| attrs.input_box.as_ref());
            let size = input_box.map_or(node.size, |b| b.size);
            let position = input_box.map_or_else(
                || {
                    graph
                        .initial_positions
                        .get(id.0 as usize)
                        .copied()
                        .flatten()
                        // A compiler-created locked_position is an authored
                        // constraint, not an existing Box.TopLeft during layout.
                        .or_else(|| require_position.then_some(node.locked_position).flatten())
                },
                |b| b.top_left,
            );
            if require_position && position.is_none() {
                return Err(LayoutError::D2MissingPosition { object: object() });
            }
            for (dimension, value) in [("width", size.width), ("height", size.height)] {
                if !value.is_finite() || value <= 0.0 || value > 1_000_000_000.0 {
                    return Err(LayoutError::D2ObjectDimension {
                        object: object(),
                        dimension,
                        value,
                    });
                }
            }
            if let Some(position) = position {
                for (coordinate, value) in [
                    ("top-left x", position.x),
                    ("top-left y", position.y),
                    ("bottom-right x", position.x + size.width),
                    ("bottom-right y", position.y + size.height),
                ] {
                    if !value.is_finite() || value.abs() > 1_000_000_000.0 {
                        return Err(LayoutError::D2ObjectCoordinate {
                            object: object(),
                            coordinate,
                            value,
                        });
                    }
                }
            }
        }
        if let Some(attrs) = node
            .d2_authored_geometry
            .as_ref()
            .and_then(|a| a.attributes.as_ref())
        {
            let object_id = &node
                .d2_authored_geometry
                .as_ref()
                .expect("D2 attributes")
                .object_id;
            let owner = format!("D2 object {}", d2_rs_core::quote_go_bytes(object_id));
            validate_label_dimensions(&owner, "label", &attrs.attributes)?;
            validate_style(&owner, &attrs.attributes)?;
            validate_label_position(&owner, "label", &attrs.label_position, false)?;
            validate_label_position(&owner, "icon", &attrs.icon_position, false)?;
        }
        if node.size.width.is_finite()
            && node.size.height.is_finite()
            && node.size.width >= 0.0
            && node.size.height >= 0.0
            && let Some(attrs) = &node.d2_authored_geometry
        {
            validate_authored_geometry(attrs)?;
        }
        if let Some(attrs) = &node.d2_authored_geometry {
            let owner = || format!("D2 object {}", d2_rs_core::quote_go_bytes(&attrs.object_id));
            if let Some(size) = node.font_size
                && (size == 0 || size > 1_000_000_000)
            {
                return Err(LayoutError::D2AttributeValidation(format!(
                    "{} has unsupported effective font size {size}",
                    owner()
                )));
            }
            if let Some(columns) = graph.table_column_count(id) {
                let exact_table_shape = attrs
                    .attributes
                    .as_ref()
                    .and_then(|a| a.shape.as_deref())
                    .map_or(node.shape == d2_rs_core::ShapeKind::SqlTable, |s| {
                        s == b"sql_table"
                    });
                if !exact_table_shape {
                    return Err(LayoutError::D2AttributeValidation(format!(
                        "{} has SQL columns but is not an SQL table",
                        owner()
                    )));
                }
                const MAX_TABLE_COLUMNS: usize = 10_000;
                if columns > MAX_TABLE_COLUMNS - table_columns {
                    return Err(LayoutError::D2AttributeValidation(
                        "D2 SQL table column count exceeds limit 10000".into(),
                    ));
                }
                table_columns += columns;
            }
        }
        if !node.size.width.is_finite()
            || !node.size.height.is_finite()
            || node.size.width <= 0.0
            || node.size.height <= 0.0
        {
            return Err(LayoutError::InvalidSize {
                node: id,
                width: node.size.width,
                height: node.size.height,
            });
        }
        if let Some(parent) = node.parent
            && graph.node(parent).is_none()
        {
            return Err(LayoutError::MissingParent { node: id, parent });
        }
        let mut seen = BTreeSet::new();
        let mut cursor = node.parent;
        while let Some(parent) = cursor {
            if parent == id || !seen.insert(parent) {
                return Err(LayoutError::ParentCycle { node: id });
            }
            cursor = graph.node(parent).and_then(|candidate| candidate.parent);
        }
        if node.d2_near_key.is_none()
            && let Some(near) = node.near
            && graph.node(near).is_none()
        {
            return Err(LayoutError::MissingNear { node: id, near });
        }
        for near in graph
            .near_targets
            .get(id.0 as usize)
            .into_iter()
            .flatten()
            .copied()
        {
            if node.d2_near_key.is_none() && graph.node(near).is_none() {
                return Err(LayoutError::MissingNear { node: id, near });
            }
        }
    }
    // adapter_validate.go validateD2Edges rejects the whole edge inventory
    // (limits.MaxEngineEdges) before examining any edge. Without it an 80,000
    // edge board exhausts memory in layout instead of failing as Go does.
    const MAX_D2_EDGES: usize = 50_000;
    if graph.edges.len() > MAX_D2_EDGES {
        return Err(LayoutError::D2EdgeLimit {
            limit: MAX_D2_EDGES,
        });
    }
    // The adapter validates stored routes even for layout, where translation
    // subsequently discards them. The aggregate count is checked per edge,
    // before that edge's point count/coordinates (not as a global pre-pass).
    const MAX_ROUTE_POINTS: usize = 1_000_000;
    let mut route_points = 0;
    for (id, edge) in graph.edges() {
        if graph.node(edge.source).is_none() || graph.node(edge.target).is_none() {
            return Err(LayoutError::MissingEndpoint { edge: id });
        }
        let owner = format!("D2 edge {}", id.0);
        let attributes = graph.d2_edge_validation(id);
        if let Some(attrs) = attributes {
            validate_label_dimensions(&owner, "label", &attrs.attributes)?;
            validate_style(&owner, &attrs.attributes)?;
            validate_label_position(&owner, "label", &attrs.label_position, true)?;
            if let Some(value) = attrs.label_percentage
                && (!value.is_finite() || !(0.0..=1.0).contains(&value))
            {
                return Err(LayoutError::D2LabelPercentage { edge: id, value });
            }
        }
        // adapter_validate.go validates compiled D2 attributes before
        // ToArrowhead conversion and source-only edge reversal. It visits
        // source before destination regardless of syntax arrow presence.
        let declarations = graph.edge_arrowhead_declarations(id);
        for (endpoint, shape, attrs) in [
            (
                "source",
                declarations.source,
                attributes.and_then(|a| a.source_arrowhead.as_ref()),
            ),
            (
                "destination",
                declarations.target,
                attributes.and_then(|a| a.target_arrowhead.as_ref()),
            ),
        ] {
            if let Some(shape) = shape
                && !shape.is_empty()
                && !matches!(
                    shape.as_str(),
                    "none"
                        | "arrow"
                        | "triangle"
                        | "diamond"
                        | "circle"
                        | "box"
                        | "cross"
                        | "cf-one"
                        | "cf-many"
                        | "cf-one-required"
                        | "cf-many-required"
                )
            {
                return Err(LayoutError::UnsupportedD2Arrowhead {
                    edge: id,
                    endpoint,
                    shape,
                });
            }
            if let Some(attrs) = attrs {
                validate_style(&format!("{owner} {endpoint} arrowhead"), attrs)?;
                validate_label_dimensions(&owner, &format!("{endpoint} arrowhead label"), attrs)?;
            }
        }
        if let Some(attrs) = attributes {
            let (source, target) = if attrs.endpoints_reversed {
                (edge.target, edge.source)
            } else {
                (edge.source, edge.target)
            };
            for (endpoint, object, index) in [
                ("source", source, attrs.source_table_column),
                ("destination", target, attrs.target_table_column),
            ] {
                let Some(index) = index else { continue };
                let Some(columns) = graph.table_column_count(object) else {
                    return Err(LayoutError::D2AttributeValidation(format!(
                        "{owner} has a {endpoint} table column index for a non-table object"
                    )));
                };
                if index < 0 || index as u64 >= columns as u64 {
                    return Err(LayoutError::D2AttributeValidation(format!(
                        "{owner} {endpoint} table column index {index} is outside table with {columns} columns"
                    )));
                }
            }
        }
        let route = graph.d2_input_route(id);
        if route.len() > MAX_ROUTE_POINTS - route_points {
            return Err(LayoutError::D2RoutePointLimit {
                limit: MAX_ROUTE_POINTS,
            });
        }
        if route.len() == 1 {
            return Err(LayoutError::D2SingleRoutePoint { edge: id });
        }
        route_points += route.len();
        for (index, point) in route.iter().enumerate() {
            if attributes.is_some_and(|a| a.nil_route_points.contains(&index)) {
                return Err(LayoutError::D2AttributeValidation(format!(
                    "D2 edge {} has a nil route point at index {index}",
                    id.0
                )));
            }
            for (axis, value) in [("x", point.x), ("y", point.y)] {
                if !value.is_finite() || value.abs() > 1_000_000_000.0 {
                    return Err(LayoutError::D2RouteCoordinate {
                        edge: id,
                        index,
                        axis,
                        value,
                    });
                }
            }
            if index > 0 && *point == route[index - 1] {
                return Err(LayoutError::D2DegenerateRoute { edge: id, index });
            }
        }
    }
    if let Some(message) = &graph.d2_boundary_errors.identity {
        return Err(LayoutError::D2GraphValidation(message.clone()));
    }
    resolve_d2_near_keys(graph)?;
    // translateGraph computes minimum edge dimensions only after every input
    // attribute/route and near key has passed validation. Empty main labels
    // still contribute dimensions; arrow labels contribute with nonempty text.
    for (id, _) in graph.edges() {
        let Some(attrs) = graph.d2_edge_validation(id) else {
            continue;
        };
        let mut width = attrs.attributes.label_dimensions[0] + 10;
        let mut height = attrs.attributes.label_dimensions[1] + 10;
        let labels = graph.edge_arrowhead_labels(id);
        for label in [labels.source, labels.target].into_iter().flatten() {
            let extent = (label.size.width.max(label.size.height) + 5.0) as i64;
            width += extent;
            height += extent;
        }
        if width > 1_000_000_000 || height > 1_000_000_000 {
            let name = attrs
                .external_id
                .clone()
                .or_else(|| d2_rs_core::edge_abs_id(graph, id))
                .unwrap_or_else(|| id.0.to_string());
            return Err(LayoutError::D2AttributeValidation(format!(
                "D2 edge {} minimum dimensions exceed supported limit 1000000000",
                d2_rs_core::quote_go_bytes(name.as_bytes())
            )));
        }
    }
    Ok(())
}

fn d2_object_visits(graph: &Graph) -> Vec<NodeId> {
    let mut fallback_children: BTreeMap<Option<NodeId>, Vec<NodeId>> = BTreeMap::new();
    for (id, node) in graph.nodes() {
        fallback_children.entry(node.parent).or_default().push(id);
    }
    let children = |parent| {
        graph
            .hierarchy_children_order
            .get(&parent)
            .or_else(|| fallback_children.get(&parent))
            .cloned()
            .unwrap_or_default()
    };
    // translateGraph visits ChildrenArray depth-first, independently of the
    // sorted graph.Objects list. Its toNode map contains exactly those visits.
    let mut stack = children(None);
    stack.reverse();
    let mut seen = BTreeSet::new();
    let mut visits = Vec::new();
    while let Some(id) = stack.pop() {
        if !seen.insert(id) || graph.node(id).is_none() {
            continue;
        }
        visits.push(id);
        stack.extend(children(Some(id)).into_iter().rev());
    }

    visits
}

/// Resolve the original D2 keys at the current adapter boundary. Extraction
/// reparents objects, so the compiler's cached `Node.near` may name a different
/// object (or have been removed) even though the authored key is unchanged.
pub(super) fn resolve_d2_near_keys(graph: &Graph) -> Result<Vec<(NodeId, NodeId)>, LayoutError> {
    use d2_rs_core::{ast::keywords::NEAR_CONSTANTS, ir::go_lowercase};

    if graph.nodes().all(|(_, node)| node.d2_near_key.is_none()) {
        return Ok(Vec::new());
    }
    let mut named_children = BTreeMap::new();
    for (id, node) in graph.nodes() {
        // D2 Children is keyed by local canonical ID, not by original AbsID.
        // A quoted component containing a dot must remain one component.
        let local_id = node
            .d2_authored_geometry
            .as_ref()
            .map(|geometry| geometry.object_id.as_slice())
            .unwrap_or_else(|| {
                node.parent
                    .and_then(|parent| graph.node(parent))
                    .and_then(|parent| {
                        node.external_id
                            .strip_prefix(&format!("{}.", parent.external_id))
                    })
                    .unwrap_or(&node.external_id)
                    .as_bytes()
            });
        named_children.insert((node.parent, go_lowercase(local_id)), id);
    }
    let visits = d2_object_visits(graph);
    let seen: BTreeSet<_> = visits.iter().copied().collect();

    let mut resolved = Vec::new();
    for id in visits {
        let node = graph.node(id).expect("visited D2 object");
        let Some(key) = &node.d2_near_key else {
            continue;
        };
        if key.len() > 256 {
            return Err(LayoutError::D2NearKeyDepth {
                object: node.external_id.clone(),
                max: 256,
            });
        }
        if let Some(index) = node
            .d2_authored_geometry
            .as_ref()
            .and_then(|geometry| geometry.attributes.as_ref())
            .and_then(|attributes| attributes.invalid_near_key_part)
        {
            return Err(LayoutError::D2InvalidNearKeyPart {
                object: node.external_id.clone(),
                index,
            });
        }
        if key.is_empty() {
            return Err(LayoutError::D2EmptyNearKey {
                object: node.external_id.clone(),
            });
        }
        // Unlike compiler HasChild, adapter findD2Object does not treat a
        // final reserved keyword as an alias for the containing object.
        let mut parent = None;
        let mut found = true;
        for part in key {
            match named_children.get(&(parent, go_lowercase(part.as_bytes()))) {
                Some(target) => parent = Some(*target),
                None => {
                    found = false;
                    break;
                }
            }
        }
        if found {
            let target = parent.expect("nonempty resolved D2 near key");
            if seen.contains(&target) {
                resolved.push((id, target));
                continue;
            }
        } else if key.len() == 1 && NEAR_CONSTANTS.contains(&key[0].as_str()) {
            continue;
        }
        return Err(LayoutError::D2NearOutsideTree {
            object: node.external_id.clone(),
        });
    }
    Ok(resolved)
}

#[cfg(test)]
mod near_key_tests {
    use crate::{Engine, Graph, LayoutError, LayoutOptions, Node, Size};

    #[test]
    fn original_near_key_is_checked_before_layout_and_routing() {
        let mut graph = Graph::default();
        let target = graph.add_node(Node::new(
            "target",
            Size {
                width: 40.0,
                height: 40.0,
            },
        ));
        let mut source = Node::new(
            "a",
            Size {
                width: 40.0,
                height: 40.0,
            },
        );
        source.near = Some(target);
        source.d2_near_key = Some(vec!["target".into(), "width".into()]);
        let source = graph.add_node(source);
        // RouteEdges validates existing positions before resolving near keys.
        graph.set_initial_position(target, Some(crate::Point { x: 0.0, y: 0.0 }));
        graph.set_initial_position(source, Some(crate::Point { x: 100.0, y: 0.0 }));
        let options = LayoutOptions::default();
        for error in [
            Engine.layout(&graph, &options).unwrap_err(),
            Engine.route_edges(&graph, &options).unwrap_err(),
        ] {
            assert!(matches!(error, LayoutError::D2NearOutsideTree { .. }));
            assert_eq!(
                error.to_string(),
                "D2 object \"a\" has a near key outside the object tree"
            );
        }
        let json = serde_json::to_vec(&graph).unwrap();
        let restored: Graph = serde_json::from_slice(&json).unwrap();
        assert!(super::validate(&restored).is_err());
        graph.node_mut(source).unwrap().d2_near_key = Some(vec!["TaRgEt".into()]);
        super::validate(&graph).unwrap();
        graph.node_mut(source).unwrap().d2_near_key = None;
        super::validate(&graph).unwrap();
    }

    fn add_node(graph: &mut Graph, name: &str, parent: Option<crate::NodeId>) -> crate::NodeId {
        let mut node = Node::new(
            name,
            Size {
                width: 40.0,
                height: 40.0,
            },
        );
        node.parent = parent;
        graph.add_node(node)
    }

    #[test]
    fn authored_key_materializes_current_target_instead_of_stale_cache() {
        let mut graph = Graph::default();
        let target = add_node(&mut graph, "target", None);
        let stale = add_node(&mut graph, "stale", None);
        let owner = add_node(&mut graph, "a", None);
        graph.node_mut(owner).unwrap().d2_near_key = Some(vec!["TaRgEt".into()]);
        for cached in [None, Some(stale), Some(crate::NodeId(999))] {
            graph.node_mut(owner).unwrap().near = cached;
            super::validate(&graph).unwrap();
            let arena = super::super::ArenaGraph::from_input(&graph);
            assert_eq!(arena.nodes[owner.0 as usize].nears.keys(), vec![target]);
            assert_eq!(arena.nodes[target.0 as usize].nears.keys(), vec![owner]);
            assert!(arena.nodes[stale.0 as usize].nears.keys().is_empty());
        }
        graph.node_mut(owner).unwrap().d2_near_key = None;
        graph.node_mut(owner).unwrap().near = Some(stale);
        let arena = super::super::ArenaGraph::from_input(&graph);
        assert_eq!(arena.nodes[owner.0 as usize].nears.keys(), vec![stale]);
    }

    #[test]
    fn extraction_retargets_authored_near_against_current_local_tree() {
        // Source DSL: cell: {x}; grid: {grid-columns: 1;
        // cell: {x; a.near: cell.x}}. The compiler's original target is
        // outside grid.cell. ExtractSubgraph(grid.cell, true) removes that
        // target and reparents grid.cell below the empty root as local cell.
        let mut graph = Graph::default();
        let cell = add_node(&mut graph, "cell", None);
        let local_target = add_node(&mut graph, "cell.x", Some(cell));
        let owner = add_node(&mut graph, "cell.a", Some(cell));
        graph.node_mut(owner).unwrap().d2_near_key = Some(vec!["cell".into(), "x".into()]);
        assert_eq!(graph.node(owner).unwrap().near, None);
        super::validate(&graph).unwrap();
        let arena = super::super::ArenaGraph::from_input(&graph);
        assert_eq!(
            arena.nodes[owner.0 as usize].nears.keys(),
            vec![local_target]
        );
        assert_eq!(
            arena.nodes[local_target.0 as usize].nears.keys(),
            vec![owner]
        );
    }

    #[test]
    fn edge_attribute_errors_precede_adapter_near_errors() {
        let mut graph = Graph::default();
        let source = add_node(&mut graph, "a", None);
        let target = add_node(&mut graph, "b", None);
        graph.node_mut(source).unwrap().d2_near_key = Some(vec!["missing".into()]);
        let edge = graph.add_edge(crate::Edge { source, target });
        graph.set_edge_arrowhead_declarations(
            edge,
            crate::EdgeArrowheadDeclarations {
                source: Some("rectangle".into()),
                target: None,
            },
        );
        assert!(matches!(
            super::validate(&graph),
            Err(LayoutError::UnsupportedD2Arrowhead { .. })
        ));
    }

    #[test]
    fn direct_adapter_keys_keep_empty_depth_and_constant_contracts() {
        let mut graph = Graph::default();
        let id = graph.add_node(Node::new(
            "a",
            Size {
                width: 40.0,
                height: 40.0,
            },
        ));
        graph.node_mut(id).unwrap().d2_near_key = Some(Vec::new());
        assert!(matches!(
            super::validate(&graph),
            Err(LayoutError::D2EmptyNearKey { .. })
        ));
        graph.node_mut(id).unwrap().d2_near_key = Some(vec!["x".into(); 257]);
        assert!(matches!(
            super::validate(&graph),
            Err(LayoutError::D2NearKeyDepth { max: 256, .. })
        ));
        graph.node_mut(id).unwrap().d2_near_key = Some(vec!["top-left".into()]);
        super::validate(&graph).unwrap();
        graph.node_mut(id).unwrap().d2_near_key = Some(vec!["top-left".into(), "x".into()]);
        assert!(matches!(
            super::validate(&graph),
            Err(LayoutError::D2NearOutsideTree { .. })
        ));
    }
}

#[cfg(test)]
mod arrowhead_declaration_tests {
    use crate::{
        CancellationToken, Edge, EdgeArrowheadDeclarations, EdgeArrowheads, EdgeArrows, Engine,
        Graph, LayoutError, LayoutOptions, Node, Size,
    };

    fn graph() -> (Graph, crate::EdgeId) {
        let mut graph = Graph::default();
        let source = graph.add_node(Node::new(
            "a",
            Size {
                width: 40.0,
                height: 40.0,
            },
        ));
        let target = graph.add_node(Node::new(
            "b",
            Size {
                width: 40.0,
                height: 40.0,
            },
        ));
        let edge = graph.add_edge(Edge { source, target });
        // Both layout and standalone-routing assertions below use valid
        // positioned objects so they reach the intended edge-attribute gate.
        graph.set_initial_position(source, Some(crate::Point { x: 0.0, y: 0.0 }));
        graph.set_initial_position(target, Some(crate::Point { x: 100.0, y: 0.0 }));
        (graph, edge)
    }

    fn assert_unsupported(
        error: LayoutError,
        edge: crate::EdgeId,
        endpoint: &'static str,
        expected_shape: &str,
    ) {
        assert_eq!(
            error.to_string(),
            format!(
                "D2 edge {} has unsupported {endpoint} arrowhead {expected_shape:?}",
                edge.0
            )
        );
        assert!(
            matches!(error, LayoutError::UnsupportedD2Arrowhead { edge: found_edge, endpoint: found_endpoint, shape } if found_edge == edge && found_endpoint == endpoint && shape == expected_shape)
        );
    }

    #[test]
    fn all_layout_and_route_apis_reject_original_source_before_destination() {
        let (mut graph, edge) = graph();
        // Syntax has only a source arrow, so canonicalization would reverse
        // it. Validation must still report the original D2 source first.
        graph.set_edge_arrows(
            edge,
            EdgeArrows {
                source: true,
                target: false,
            },
        );
        graph.set_edge_arrowheads(
            edge,
            EdgeArrowheads {
                source: Some("triangle".into()),
                target: Some("triangle".into()),
            },
        );
        graph.set_edge_arrowhead_declarations(
            edge,
            EdgeArrowheadDeclarations {
                source: Some("rectangle".into()),
                target: Some("ſquare".into()),
            },
        );
        let options = LayoutOptions::default();
        let token = CancellationToken::new();
        for error in [
            Engine.layout(&graph, &options).unwrap_err(),
            Engine
                .layout_with_cancellation(&graph, &options, &token)
                .unwrap_err(),
            Engine.route_edges(&graph, &options).unwrap_err(),
            Engine
                .route_edges_with_cancellation(&graph, &options, &token)
                .unwrap_err(),
            Engine
                .route_edges_selected(&graph, &options, &[edge])
                .unwrap_err(),
            Engine
                .route_edges_selected_with_cancellation(&graph, &options, &[edge], &token)
                .unwrap_err(),
        ] {
            assert_unsupported(error, edge, "source", "rectangle");
        }
        graph.set_edge_arrowhead_declarations(
            edge,
            EdgeArrowheadDeclarations {
                source: None,
                target: Some("ſquare".into()),
            },
        );
        assert_unsupported(
            Engine.layout(&graph, &options).unwrap_err(),
            edge,
            "destination",
            "ſquare",
        );
    }

    #[test]
    fn selected_routing_validates_unrequested_edges_in_graph_order() {
        let (mut graph, first) = graph();
        let second = graph.add_edge(graph.edges[first.0 as usize].clone());
        graph.set_edge_arrowhead_declarations(
            first,
            EdgeArrowheadDeclarations {
                source: None,
                target: Some("rectangle".into()),
            },
        );
        graph.set_edge_arrowhead_declarations(
            second,
            EdgeArrowheadDeclarations {
                source: Some("ſquare".into()),
                target: None,
            },
        );
        let error = Engine
            .route_edges_selected(&graph, &LayoutOptions::default(), &[second])
            .unwrap_err();
        assert_unsupported(error, first, "destination", "rectangle");
    }

    #[test]
    fn absent_and_empty_declarations_do_not_validate_resolved_shape_identities() {
        let (mut graph, edge) = graph();
        for resolved in [
            "filled-diamond",
            "filled-circle",
            "filled-box",
            "unfilled-triangle",
        ] {
            graph.set_edge_arrowheads(
                edge,
                EdgeArrowheads {
                    source: Some(resolved.into()),
                    target: Some(resolved.into()),
                },
            );
            for source in [None, Some(String::new()), Some("diamond".into())] {
                graph.set_edge_arrowhead_declarations(
                    edge,
                    EdgeArrowheadDeclarations {
                        source,
                        target: Some("circle".into()),
                    },
                );
                super::validate(&graph).unwrap();
            }
        }
        // Native Graph JSON must retain declarations for the same shared
        // validation boundary; old/resolved-only JSON remains accepted.
        graph.set_edge_arrowhead_declarations(
            edge,
            EdgeArrowheadDeclarations {
                source: Some("rectangle".into()),
                target: None,
            },
        );
        let json = serde_json::to_vec(&graph).unwrap();
        let decoded: Graph = serde_json::from_slice(&json).unwrap();
        assert_eq!(
            decoded.edge_arrowhead_declarations(edge),
            graph.edge_arrowhead_declarations(edge)
        );
        assert_eq!(decoded.edge_arrowheads(edge), graph.edge_arrowheads(edge));
        assert_unsupported(
            Engine
                .layout(&decoded, &LayoutOptions::default())
                .unwrap_err(),
            edge,
            "source",
            "rectangle",
        );
        let mut json: serde_json::Value = serde_json::from_slice(&json).unwrap();
        json.as_object_mut()
            .unwrap()
            .remove("edge_arrowhead_declarations");
        let mut legacy: Graph = serde_json::from_value(json).unwrap();
        super::validate(&legacy).unwrap();
        legacy.set_edge_arrowhead_declarations(
            edge,
            EdgeArrowheadDeclarations {
                source: Some(String::new()),
                target: None,
            },
        );
        assert_eq!(
            legacy.edge_arrowhead_declarations(edge).source.as_deref(),
            Some("")
        );
        assert_eq!(legacy.edge_arrowheads(edge), graph.edge_arrowheads(edge));
        super::validate(&legacy).unwrap();
    }
}

#[cfg(test)]
mod authored_geometry_source_contract {
    use super::*;
    #[test]
    fn authored_geometry_diagnostics_follow_children_array_preorder() {
        let mut graph = Graph::default();
        let mut invalid = |name: &str, parent| {
            let mut node = crate::Node::new(
                name,
                crate::Size {
                    width: 100.0,
                    height: 80.0,
                },
            );
            node.parent = parent;
            node.d2_authored_geometry = Some(Box::new(d2_rs_core::D2AuthoredGeometry {
                object_id: name.as_bytes().to_vec(),
                width: Some(b"0".to_vec()),
                ..Default::default()
            }));
            graph.add_node(node)
        };
        let sibling = invalid("sibling", None);
        let parent = invalid("parent", None);
        let child = invalid("child", Some(parent));
        graph.node_mut(parent).unwrap().d2_authored_geometry = None;
        graph.set_hierarchy_children_order(BTreeMap::from([
            (None, vec![parent, sibling]),
            (Some(parent), vec![child]),
        ]));
        assert_eq!(
            validate(&graph).unwrap_err().to_string(),
            "D2 object \"child\" width attribute must be between 1 and 1000000000"
        );
    }
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn complete_original_integer_attribute_and_pairing_diagnostics() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/authored-geometry-source-v090.json"
        ))
        .unwrap();
        let rows = rows.as_array().unwrap();
        assert_eq!(rows.len(), 487);
        let mut outputs = Vec::new();
        for row in rows {
            let r = &row["request"];
            let bytes = |name: &str| {
                r[name].as_array().map(|v| {
                    v.iter()
                        .map(|b| b.as_u64().unwrap() as u8)
                        .collect::<Vec<_>>()
                })
            };
            let attrs = d2_rs_core::D2AuthoredGeometry {
                object_id: bytes("ObjectID").unwrap(),
                width: bytes("Width"),
                height: bytes("Height"),
                top: bytes("Top"),
                left: bytes("Left"),
                input_box: None,
                attributes: None,
            };
            let error = validate_authored_geometry(&attrs)
                .err()
                .map(|e| e.to_string())
                .unwrap_or_default();
            assert_eq!(error, row["error"].as_str().unwrap(), "{}", r["Name"]);
            // Exercise the full layout validation boundary with the same
            // numeric fixed-position metadata supplied by the compiler.
            // Authored constraints must not masquerade as Box.TopLeft.
            let mut node = crate::Node::new(
                "a",
                crate::Size {
                    width: 100.0,
                    height: 80.0,
                },
            );
            let numeric = |value: &Option<Vec<u8>>| {
                value
                    .as_ref()
                    .and_then(|bytes| std::str::from_utf8(bytes).ok())
                    .and_then(d2_rs_core::parse_d2_float)
            };
            node.locked_position = numeric(&attrs.left)
                .zip(numeric(&attrs.top))
                .map(|(x, y)| crate::Point { x, y });
            node.d2_authored_geometry = Some(Box::new(attrs));
            let mut graph = Graph::default();
            graph.add_node(node);
            let full_error = validate(&graph)
                .err()
                .map(|e| e.to_string())
                .unwrap_or_default();
            assert_eq!(full_error, error, "full validation: {}", r["Name"]);
            outputs.push(serde_json::json!({"name":r["Name"],"error":error}));
        }
        if let Some(path) = std::env::var_os("D2_RS_TEST_AUTHORED_GEOMETRY_OUTPUT") {
            std::fs::write(path, serde_json::to_vec_pretty(&outputs).unwrap()).unwrap();
        }
    }
}
